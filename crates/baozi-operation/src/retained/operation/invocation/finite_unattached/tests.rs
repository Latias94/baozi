use super::*;
use crate::control::{
    CancellationOrigin, CancellationSignal, ControlCause, OperationControl, ProgressDecision,
    ProgressEvent, ProgressObserver, ProgressStorageError,
};
use crate::ledger::finite::FiniteLedgerLimits;
use crate::ledger::{
    LedgerEvidenceError, WorkLane, test_evidence_allocation_guard,
    test_fail_next_evidence_allocation,
};
use crate::retained::{
    CanonicalLineageAuthority, PhysicalStorageAccount, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
};
use crate::{DimensionPolicy, LedgerEvidencePurpose, LedgerLimits, OperationCoordinator};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Owner;
impl OperationOwner for Owner {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Work,
    Panic,
}

impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        match self {
            Self::Work => 0,
            Self::Panic => panic!("finite owner phase ordering panicked"),
        }
    }

    fn stable_id(self) -> &'static str {
        "finite-unattached"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    First,
    Second,
    Third,
    Fourth,
}

impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

type Invocation = FiniteOperationInvocation<Owner, u8, Phase, Unit, u8>;

#[derive(Default)]
struct ControlTrace {
    polls: AtomicUsize,
    events: AtomicUsize,
    drops: AtomicUsize,
    cancelled: AtomicBool,
    cancel_on_event: AtomicBool,
    poison_on_drop: AtomicBool,
}

struct Signal {
    account: Arc<PhysicalStorageAccount>,
    trace: Arc<ControlTrace>,
}

impl CancellationSignal for Signal {
    fn is_cancelled(&self) -> bool {
        let _ = self.account.snapshot();
        self.trace.polls.fetch_add(1, Ordering::SeqCst);
        self.trace.cancelled.load(Ordering::SeqCst)
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        let _ = self.account.snapshot();
        self.trace.drops.fetch_add(1, Ordering::SeqCst);
        if self.trace.poison_on_drop.load(Ordering::SeqCst) {
            self.account
                .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
    }
}

struct Observer {
    account: Arc<PhysicalStorageAccount>,
    trace: Arc<ControlTrace>,
}

impl ProgressObserver<Phase, Unit> for Observer {
    fn observe(&mut self, _: ProgressEvent<Phase, Unit>) -> ProgressDecision {
        let _ = self.account.snapshot();
        self.trace.events.fetch_add(1, Ordering::SeqCst);
        if self.trace.cancel_on_event.load(Ordering::SeqCst) {
            self.trace.cancelled.store(true, Ordering::SeqCst);
        }
        ProgressDecision::Continue
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        let _ = self.account.snapshot();
        self.trace.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn account() -> (PhysicalStorageAccountFactory, Arc<PhysicalStorageAccount>) {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 128).unwrap())
        .unwrap();
    (factory, Arc::new(account))
}

fn invocation(account: &Arc<PhysicalStorageAccount>, trace: &Arc<ControlTrace>) -> Invocation {
    let control = OperationControl::default()
        .with_cancellation(Signal {
            account: Arc::clone(account),
            trace: Arc::clone(trace),
        })
        .with_progress_observer(Observer {
            account: Arc::clone(account),
            trace: Arc::clone(trace),
        });
    FiniteOperationInvocation::try_new(
        control,
        FiniteLedgerLimits::try_new(
            16_384,
            16_384,
            [6, 2, 5, 1, 4, 3].map(|dimension| {
                (
                    dimension,
                    DimensionPolicy::reservable_bytes(64, 64, 64, 16_384),
                )
            }),
        )
        .unwrap(),
        [3, 1, 2],
        CanonicalLineageAuthority::fresh(),
    )
    .unwrap()
}

fn complete_work(invocation: &mut Invocation) {
    let (mut internal, _) = invocation
        .coordinator
        .retained_ledger_mut()
        .view()
        .into_internal();
    let layout = internal.bind_ordinary([1, 6]).unwrap();
    let mut frame = {
        let mut loan = internal.reborrow().unwrap();
        let mut frame = layout
            .admit_observed(&mut loan.work, [9, 5], WorkLane::Internal)
            .unwrap();
        frame
            .complete_batch_with_outcome(&layout, &mut loan.work, [4, 2])
            .into_result()
            .unwrap();
        frame
    };
    internal.resolve(layout, &mut frame, false).unwrap();
}

fn assert_original_admission(account: &PhysicalStorageAccount, poison_events: u128) {
    let actions = crate::retained::account::lock_recovering_poison(&account.inner.value().linear)
        .admitted_account_actions;
    assert_eq!(actions, 1);
    assert_eq!(account.share_call_count(), 0);
    let physical = account.snapshot();
    assert_eq!(physical.live_retained_bytes(), 0);
    assert_eq!(physical.live_retained_allocations(), 0);
    assert_eq!(physical.total_admitted_retained_bytes(), 0);
    assert_eq!(physical.total_admitted_retained_allocations(), 0);
    assert_eq!(physical.poison_event_count(), poison_events);
    assert_eq!(physical.poisoned(), poison_events != 0);
    assert!(physical.poison_event_count_exact());
    assert!(physical.poison_event_domain_valid());
}

#[test]
fn inline_and_shared_close_return_fixed_actual_facts_without_heap_admission() {
    let _allocation_guard = test_evidence_allocation_guard();
    for shared in [false, true] {
        let (factory, account) = account();
        let trace = Arc::new(ControlTrace::default());
        let admission = account.try_admit_operation_start().unwrap();
        let mut invocation = invocation(&account, &trace);
        complete_work(&mut invocation);
        if shared {
            drop(
                invocation
                    .coordinator
                    .retained_ledger_mut()
                    .view()
                    .reserve(1, 2, 7)
                    .unwrap(),
            );
        }
        invocation
            .coordinator
            .retained_ledger_mut()
            .view()
            .try_complete_phase(1)
            .unwrap();
        for unit in [Unit::First, Unit::Second, Unit::Third, Unit::Fourth] {
            invocation
                .coordinator
                .view()
                .report_progress(Phase::Work, unit, 1, Some(2))
                .unwrap();
        }
        let known = invocation.coordinator.known_administrative_bytes().unwrap();
        assert_eq!(known > 0, shared);
        let mut allocation_probe = OperationCoordinator::<Owner, u8, Phase, Unit, u8>::try_new(
            OperationControl::default(),
            LedgerLimits::new(0, []).unwrap(),
            &[],
        )
        .unwrap();
        test_fail_next_evidence_allocation();

        let terminal = invocation
            .prepare_and_finish_unattached(
                admission,
                FinalProgress::new(Phase::Work, Unit::First, 2),
            )
            .unwrap_or_else(|_| panic!("the actual finite owner closes cleanly"));

        assert!(matches!(
            allocation_probe.report_progress(Phase::Work, Unit::First, 0, None),
            Err(ControlCause::ProgressStorage {
                cause: ProgressStorageError::Evidence {
                    cause: LedgerEvidenceError::AllocationFailed {
                        purpose: LedgerEvidencePurpose::Progress,
                        ..
                    },
                },
            })
        ));
        assert_eq!(terminal.cause, None);
        assert_eq!(terminal.snapshot.dimensions().len(), 6);
        assert_eq!(terminal.snapshot.phases().len(), 3);
        assert_eq!(terminal.snapshot.last_completed_phase(), Some(1));
        assert_eq!(terminal.snapshot.current_live_bytes(), 0);
        assert_eq!(terminal.snapshot.active_reservations(), 0);
        assert_eq!(
            terminal.snapshot.peak_live_bytes(),
            if shared { 7 } else { 0 }
        );
        assert_eq!(terminal.snapshot.current_administrative_bytes(), known);
        assert_eq!(
            terminal.snapshot.covered_administrative_bytes(),
            Some(known)
        );
        assert_eq!(trace.polls.load(Ordering::SeqCst), 6);
        assert_eq!(trace.events.load(Ordering::SeqCst), 5);
        assert_eq!(trace.drops.load(Ordering::SeqCst), 2);
        assert_original_admission(&account, 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
        let work = terminal.snapshot.dimension(1).unwrap();
        assert_eq!((work.attempted(), work.consumed()), (9, 4));
        let work = terminal.snapshot.dimension(6).unwrap();
        assert_eq!((work.attempted(), work.consumed()), (5, 2));
    }
}

#[test]
fn semantic_cancellation_returns_clean_facts_and_survives_the_final_account_gate() {
    let (factory, account) = account();
    let trace = Arc::new(ControlTrace::default());
    trace.cancel_on_event.store(true, Ordering::SeqCst);
    trace.poison_on_drop.store(true, Ordering::SeqCst);
    let admission = account.try_admit_operation_start().unwrap();
    let mut invocation = invocation(&account, &trace);
    complete_work(&mut invocation);

    let terminal = invocation
        .prepare_and_finish_unattached(admission, FinalProgress::new(Phase::Work, Unit::First, 1))
        .unwrap_or_else(|_| panic!("semantic failure still owns structurally clean fixed facts"));

    assert_eq!(
        terminal.cause,
        Some(PreparationFailureCode::from_control(
            &ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            }
        ))
    );
    assert_eq!(terminal.snapshot.current_live_bytes(), 0);
    assert_eq!(terminal.snapshot.active_reservations(), 0);
    assert_eq!(terminal.snapshot.current_administrative_bytes(), 0);
    assert_eq!(terminal.snapshot.covered_administrative_bytes(), Some(0));
    let work = terminal.snapshot.dimension(1).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (9, 4));
    assert_eq!(trace.polls.load(Ordering::SeqCst), 2);
    assert_eq!(trace.events.load(Ordering::SeqCst), 1);
    assert_eq!(trace.drops.load(Ordering::SeqCst), 2);
    assert_original_admission(&account, 1);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn zero_byte_active_reservation_returns_owned_shared_residue_after_input_host_exit() {
    let (mut failure, zero, identity, known) = {
        let (factory, account) = account();
        let trace = Arc::new(ControlTrace::default());
        let admission = account.try_admit_operation_start().unwrap();
        let mut invocation = invocation(&account, &trace);
        complete_work(&mut invocation);
        let identity = invocation
            .coordinator
            .retained_ledger()
            .as_read()
            .identity();
        let zero = invocation
            .coordinator
            .retained_ledger_mut()
            .view()
            .reserve(1, 2, 0)
            .unwrap();
        let known = invocation.coordinator.known_administrative_bytes().unwrap();
        assert!(known > 0);
        let failure = invocation
            .prepare_and_finish_unattached(
                admission,
                FinalProgress::new(Phase::Work, Unit::First, 1),
            )
            .unwrap_err();
        assert_original_admission(&account, 1);
        assert_eq!(trace.drops.load(Ordering::SeqCst), 2);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
        (failure, zero, identity, known)
    };

    assert_eq!(
        failure.cause(),
        &FiniteUnattachedFailureCause::Ledger(LedgerCause::ActiveReservationsRemain { count: 1 })
    );
    assert_eq!(
        failure.terminal_cause(),
        Some(PreparationFailureCode::ledger())
    );
    let common = failure.residue.as_mut().unwrap();
    assert!(common.lifecycle.terminal_outstanding);
    assert!(common.lifecycle.terminal_seal.is_none());
    assert_eq!(
        common.coordinator.retained_ledger().as_read().identity(),
        identity
    );
    assert_eq!(common.coordinator.known_administrative_bytes(), Some(known));
    let work = common
        .coordinator
        .retained_ledger()
        .as_read()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!((work.attempted(), work.consumed()), (9, 4));
    {
        let (_, storage) = common
            .coordinator
            .retained_ledger_mut()
            .view()
            .into_internal();
        storage.validate_live_owner(&zero).unwrap();
        assert_eq!(storage.planning_state().current_live_bytes(), 0);
        assert_eq!(storage.planning_state().active_reservations(), 1);
    }
    drop(zero);
    assert_eq!(
        common
            .coordinator
            .retained_ledger()
            .as_read()
            .terminal_residue_cause(),
        None
    );
    assert_eq!(common.coordinator.known_administrative_bytes(), Some(known));
    assert_eq!(
        failure.cause(),
        &FiniteUnattachedFailureCause::Ledger(LedgerCause::ActiveReservationsRemain { count: 1 })
    );
    drop(failure);
}

#[test]
fn completed_zero_pending_frame_keeps_the_actual_open_header_and_progress_after_host_exit() {
    let (mut failure, frame, identity) = {
        let (factory, account) = account();
        let trace = Arc::new(ControlTrace::default());
        let admission = account.try_admit_operation_start().unwrap();
        let mut invocation = invocation(&account, &trace);
        invocation
            .coordinator
            .view()
            .report_progress(Phase::Work, Unit::First, 1, Some(2))
            .unwrap();
        let identity = invocation
            .coordinator
            .retained_ledger()
            .as_read()
            .identity();
        let frame = {
            let (mut internal, _) = invocation
                .coordinator
                .retained_ledger_mut()
                .view()
                .into_internal();
            let layout = internal.bind_ordinary([1]).unwrap();
            let mut loan = internal.reborrow().unwrap();
            let mut frame = layout
                .admit_observed(&mut loan.work, [3], WorkLane::Internal)
                .unwrap();
            frame
                .complete_batch_with_outcome(&layout, &mut loan.work, [3])
                .into_result()
                .unwrap();
            frame
        };
        let failure = invocation
            .prepare_and_finish_unattached(
                admission,
                FinalProgress::new(Phase::Work, Unit::First, 2),
            )
            .unwrap_err();
        assert_original_admission(&account, 1);
        assert_eq!(trace.polls.load(Ordering::SeqCst), 1);
        assert_eq!(trace.events.load(Ordering::SeqCst), 1);
        assert_eq!(trace.drops.load(Ordering::SeqCst), 2);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
        (failure, frame, identity)
    };

    assert!(frame.active);
    assert_eq!(frame.remaining, [0]);
    assert_eq!(
        failure.cause(),
        &FiniteUnattachedFailureCause::Ledger(LedgerCause::OutstandingLinearWorkPermits {
            count: 1,
        })
    );
    assert_eq!(
        failure.terminal_cause(),
        Some(PreparationFailureCode::ledger())
    );
    let common = failure.residue.as_mut().unwrap();
    assert_eq!(
        common.coordinator.retained_ledger().as_read().identity(),
        identity
    );
    assert_eq!(common.coordinator.known_administrative_bytes(), Some(0));
    assert_eq!(
        common
            .coordinator
            .reported_progress(Phase::Work, Unit::First),
        Some((1, Some(2)))
    );
    assert_eq!(
        common
            .coordinator
            .retained_ledger()
            .as_read()
            .check_work_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    assert_eq!(
        common
            .coordinator
            .retained_ledger_mut()
            .view()
            .into_internal()
            .0
            .reborrow()
            .unwrap()
            .counters(1),
        Ok((3, 3, 0))
    );
    assert!(common.lifecycle.terminal_outstanding);
    drop(failure);
}

#[test]
fn two_actual_prepared_stamps_cannot_cross_owners_or_replay_preparation() {
    let (left_factory, left_account) = account();
    let (right_factory, right_account) = account();
    let left_trace = Arc::new(ControlTrace::default());
    let right_trace = Arc::new(ControlTrace::default());
    let mut left = FiniteUnattachedGuard {
        common: Some(invocation(&left_account, &left_trace)),
        admission: left_account.try_admit_operation_start().unwrap(),
    };
    let mut right = FiniteUnattachedGuard {
        common: Some(invocation(&right_account, &right_trace)),
        admission: right_account.try_admit_operation_start().unwrap(),
    };
    let left_identity = left
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger()
        .as_read()
        .identity();
    let right_identity = right
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger()
        .as_read()
        .identity();
    assert_ne!(left_identity, right_identity);
    let left_stamp = left
        .prepare(FinalProgress::new(Phase::Work, Unit::First, 1))
        .unwrap_or_else(|()| panic!("left owner is ready"));
    let right_stamp = right
        .prepare(FinalProgress::new(Phase::Work, Unit::First, 2))
        .unwrap_or_else(|()| panic!("right owner is ready"));
    assert!(
        left.prepare(FinalProgress::new(Phase::Panic, Unit::First, 99))
            .is_err()
    );
    assert_original_admission(&left_account, 0);
    assert_original_admission(&right_account, 0);

    let left_failure = left.finish(right_stamp).unwrap_err();
    let right_failure = right.finish(left_stamp).unwrap_err();

    for (failure, identity, completed) in [
        (&left_failure, left_identity, 1),
        (&right_failure, right_identity, 2),
    ] {
        assert_eq!(
            failure.cause(),
            &FiniteUnattachedFailureCause::OwnershipUnverified
        );
        let common = failure.residue().unwrap();
        assert_eq!(
            common.coordinator.retained_ledger().as_read().identity(),
            identity
        );
        assert_eq!(
            common
                .coordinator
                .reported_progress(Phase::Work, Unit::First),
            Some((completed, Some(completed)))
        );
        assert!(common.lifecycle.terminal_outstanding);
        assert!(common.lifecycle.terminal_seal.is_some());
    }
    for (account, trace) in [(&left_account, &left_trace), (&right_account, &right_trace)] {
        assert_original_admission(account, 1);
        assert_eq!(trace.polls.load(Ordering::SeqCst), 2);
        assert_eq!(trace.events.load(Ordering::SeqCst), 1);
        assert_eq!(trace.drops.load(Ordering::SeqCst), 2);
    }
    drop(left_account);
    drop(right_account);
    assert_eq!(left_factory.snapshot().live_accounts(), 0);
    assert_eq!(right_factory.snapshot().live_accounts(), 0);
    drop((left_failure, right_failure));
}

#[test]
fn owner_phase_order_panic_keeps_actual_rows_shared_storage_and_control_owners() {
    for consuming_entry in [false, true] {
        let (factory, account) = account();
        let trace = Arc::new(ControlTrace::default());
        let admission = account.try_admit_operation_start().unwrap();
        let mut invocation = invocation(&account, &trace);
        complete_work(&mut invocation);
        let scratch = invocation
            .coordinator
            .retained_ledger_mut()
            .view()
            .reserve(1, 2, 7)
            .unwrap();
        invocation
            .coordinator
            .view()
            .report_progress(Phase::Work, Unit::First, 1, Some(2))
            .unwrap();
        invocation.lifecycle.control_action_debt.set(7);
        let identity = invocation
            .coordinator
            .retained_ledger()
            .as_read()
            .identity();
        let known = invocation.coordinator.known_administrative_bytes().unwrap();
        assert!(known > 0);
        let final_progress = FinalProgress::new(Phase::Panic, Unit::First, 2);
        let payload = if consuming_entry {
            let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
                invocation.prepare_and_finish_unattached(admission, final_progress)
            })) else {
                panic!("the owner-defined phase must unwind through the consuming entry");
            };
            payload
        } else {
            let mut guard = FiniteUnattachedGuard {
                common: Some(invocation),
                admission,
            };
            let Err(payload) = catch_unwind(AssertUnwindSafe(|| guard.prepare(final_progress)))
            else {
                panic!("the actual kernel must call the owner-defined phase order");
            };
            assert_original_admission(&account, 0);
            let common = guard.common.as_mut().unwrap();
            assert!(common.lifecycle.terminal_outstanding);
            assert!(common.lifecycle.terminal_seal.is_none());
            assert_eq!(common.lifecycle.control_action_debt.get(), 7);
            assert_eq!(
                common.coordinator.retained_ledger().as_read().identity(),
                identity
            );
            assert_eq!(common.coordinator.known_administrative_bytes(), Some(known));
            assert_eq!(
                common
                    .coordinator
                    .reported_progress(Phase::Work, Unit::First),
                Some((1, Some(2)))
            );
            let (actual, cause) = common
                .coordinator
                .retained_ledger_mut()
                .take_terminal_snapshot(0);
            assert_eq!(
                cause,
                Some(LedgerCause::LiveReservationsRemain { bytes: 7 })
            );
            assert_eq!(actual.current_live_bytes(), 7);
            assert_eq!(actual.active_reservations(), 1);
            assert_eq!(actual.current_administrative_bytes(), known);
            assert_eq!(actual.covered_administrative_bytes(), Some(known));
            let work = actual.dimension(1).unwrap();
            assert_eq!((work.attempted(), work.consumed()), (9, 4));
            drop(guard);
            payload
        };
        assert_eq!(
            payload.downcast_ref::<&str>(),
            Some(&"finite owner phase ordering panicked")
        );
        drop(payload);
        assert_original_admission(&account, 1);
        assert_eq!(trace.polls.load(Ordering::SeqCst), 2);
        assert_eq!(trace.events.load(Ordering::SeqCst), 1);
        assert_eq!(trace.drops.load(Ordering::SeqCst), 0);
        drop(scratch);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
        assert!(factory.snapshot().live_account_control_bytes() > 0);
        assert_eq!(trace.drops.load(Ordering::SeqCst), 0);
    }
}
