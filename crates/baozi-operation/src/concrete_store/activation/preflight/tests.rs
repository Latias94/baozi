use super::*;
use crate::concrete_store::{RetainedStoreSharePlan, RetainedStoreStartPlan};
use crate::{
    DimensionPolicy, LedgerCause, LedgerLimits, OperationControl, PhysicalStorageAccount,
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, StoreDimensionSemanticId, StoreFallbackEnvelope, StoreLedgerPlan,
    StorePhaseSemanticId, StoreSemanticCatalogId,
};
use std::num::NonZeroU32;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Owner;
impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Progress;
impl ProgressPhase for Progress {
    fn order(self) -> u32 {
        0
    }
    fn stable_id(self) -> &'static str {
        "build"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Unit;
impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

type Operation = RetainedStoreOperation<Owner, Progress, Unit>;

#[derive(Clone, Copy)]
struct Keys {
    phase: StorePhaseKey,
    work: StoreDimensionKey,
    storage: StoreDimensionKey,
    terminal: StoreDimensionKey,
}

impl Keys {
    fn writer(self, members: usize) -> RetainedStoreWriterPlan {
        RetainedStoreWriterPlan::new(
            self.phase,
            self.storage,
            self.storage,
            RetainedStoreBatchLimits::new(members + 1, 4).unwrap(),
        )
    }

    fn session(
        self,
        operation: Operation,
    ) -> RetainedStoreSessionOutcomeOwner<Owner, Progress, Unit> {
        operation
            .into_session_outcome_owner(
                CanonicalActivationSessionPolicy::new(
                    RetainedStoreWriterRole::new(self.phase, self.storage, self.storage),
                    RetainedStoreWriterRole::new(self.phase, self.storage, self.storage),
                    CanonicalMaterializationRole::new(self.work, self.storage),
                    CanonicalValidationRole::new(self.phase, self.work, self.storage),
                    self.storage,
                    self.storage,
                ),
                RetainedStoreBatchLimits::new(5, 4).unwrap(),
                CanonicalFailureRequirements::new(CanonicalTableShape::new(4, 4).unwrap()),
            )
            .unwrap()
    }
}

fn operation(
    total_attempted: u64,
    total_consumed: u64,
    control: OperationControl<Progress, Unit>,
) -> (Operation, Keys, PhysicalStorageAccount) {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(1 << 20, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(1 << 20, 128).unwrap())
        .unwrap();
    let catalog = StoreSemanticCatalogId::new(*b"terminal-qualify", NonZeroU32::MIN);
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let dimension = |local| {
        domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    };
    let keys = Keys {
        phase: domain
            .try_admit_phase(StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
            .unwrap(),
        work: dimension(1),
        storage: dimension(2),
        terminal: dimension(3),
    };
    let terminal = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        keys.terminal,
        2,
        5,
        5,
        total_attempted,
        total_consumed,
    )
    .unwrap();
    let limits = LedgerLimits::new(
        1 << 20,
        [
            (
                keys.work,
                DimensionPolicy::counter(
                    terminal.ordinary_attempted_work_limit(),
                    terminal.ordinary_consumed_work_limit(),
                    0,
                ),
            ),
            (
                keys.storage,
                DimensionPolicy::reservable_bytes(0, 0, 1 << 20, 1 << 20)
                    .with_published_outcome_evidence_limit(1 << 20),
            ),
            (keys.terminal, terminal.requirement().dimension_policy()),
        ],
    )
    .unwrap();
    let ledger = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        keys.writer(4),
        RetainedStoreSharePlan::new(keys.work, keys.work),
        StoreFallbackEnvelope::new(7),
        keys.storage,
        1,
        keys.storage,
        1,
        terminal,
    );
    let operation = Operation::try_start(control, ledger, &account, start).unwrap();
    (operation, keys, account)
}

fn counters(operation: &mut Operation, keys: Keys) -> (u64, u64) {
    let state = operation
        .producer_host()
        .dimension_state(keys.work)
        .unwrap();
    (state.attempted(), state.consumed())
}

#[derive(Clone, Copy)]
struct Tag;

#[test]
fn exact_transfer_preserves_work_and_qualifies_four_real_evidence_members() {
    let (mut operation, keys, account) = operation(512, 500, OperationControl::default());
    {
        let mut host = operation.producer_host();
        let mut work = host.try_begin_observed_work([(keys.work, 7)]).unwrap();
        work.complete([4]).unwrap();
        work.finish_partial();
    }
    let before = counters(&mut operation, keys);
    // Two record members (2 + 3 fields) and two scalar members (6 + 20 fields).
    operation
        .qualify_failure_validation_work(keys.work, 4, 31)
        .unwrap();
    assert_eq!(counters(&mut operation, keys), before);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((4, 5, 31))
    );
    let RetainedStoreSessionOutcomeOwner {
        dormant,
        mut failure_work,
        ..
    } = keys.session(operation);
    assert_eq!(failure_work.remaining_members, 4);
    assert_eq!(failure_work.remaining_validation_work, 31);
    let operation = dormant.into_operation().unwrap();
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(keys.writer(1), 1, 1).unwrap(),
        RetainedStoreBlockTablePlan::new(keys.writer(4), 4, 4).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    let mut fault = None;
    {
        let mut lane = CanonicalOutcomeEvidenceLane {
            assembly: &mut assembly,
            first_fault: &mut fault,
            failure_work: &mut failure_work,
            work_dimension: keys.work,
        };
        let counts = lane
            .try_retain_records(RecordPlan::new(1, 2).unwrap(), Tag, |writer| {
                writer.write_row(|row| {
                    row.write_scalar(2_u32)?;
                    row.write_scalar(6_u32)
                })
            })
            .unwrap();
        let corners = lane
            .try_retain_sequence(SequencePlan::<u32>::new(6).unwrap(), Tag, |writer| {
                writer.extend_from_slice(&[0, 1, 2, 0, 2, 3])
            })
            .unwrap();
        let faces = lane
            .try_retain_sequence(SequencePlan::<u32>::new(20).unwrap(), Tag, |writer| {
                writer.extend_from_slice(&[0; 20])
            })
            .unwrap();
        let _root = lane
            .try_retain_records(RecordPlan::new(1, 3).unwrap(), Tag, |writer| {
                writer.write_row(|row| {
                    row.write_record_ref(counts)?;
                    row.write_sequence_ref(corners)?;
                    row.write_sequence_ref(faces)
                })
            })
            .unwrap();
    }
    assert!(fault.is_none());
    assert_eq!(failure_work.remaining_members, 0);
    assert_eq!(failure_work.remaining_validation_work, 0);
    assert_eq!(failure_work.planned_validation_work().unwrap().get(), 31);
    assert!(failure_work.with_existing_member_validation(1).is_err());
    let published = assembly.prepare_minimal_fallback_with(7).unwrap().handoff();
    let ordinary = published.ledger_evidence.dimension(keys.work).unwrap();
    let terminal = published.ledger_evidence.dimension(keys.terminal).unwrap();
    assert!(ordinary.attempted() >= before.0 + 31);
    assert!(ordinary.consumed() >= before.1 + 31);
    assert_eq!((terminal.attempted(), terminal.consumed()), (0, 0));
    assert!(ordinary.attempted() + terminal.attempted() <= 512);
    assert!(ordinary.consumed() + terminal.consumed() <= 500);
    drop(published);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn insufficient_unused_work_preserves_startup_guarantee_and_fallback() {
    let (mut operation, keys, account) = operation(64, 64, OperationControl::default());
    operation
        .producer_host()
        .charge_observed_batch([(keys.work, 30)])
        .unwrap();
    let before = counters(&mut operation, keys);
    assert!(matches!(
        operation.qualify_failure_validation_work(keys.work, 4, 31),
        Err(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded { .. }))
    ));
    assert_eq!(counters(&mut operation, keys), before);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
    let published = operation.prepare_minimal_fallback(7).unwrap().handoff();
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(published.ledger_evidence.header().active_reservations(), 0);
    let ordinary = published.ledger_evidence.dimension(keys.work).unwrap();
    assert_eq!((ordinary.attempted(), ordinary.consumed()), before);
    drop(published);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn qualification_is_one_time_and_cannot_reduce_startup_guarantees() {
    let (mut operation, keys, account) = operation(128, 128, OperationControl::default());
    operation
        .qualify_failure_validation_work(keys.work, 1, 1)
        .unwrap();
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
    let before = counters(&mut operation, keys);
    assert!(matches!(
        operation.qualify_failure_validation_work(keys.work, 4, 31),
        Err(StoreRuntimeError::InvalidPublicationPlan)
    ));
    assert_eq!(counters(&mut operation, keys), before);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
    drop(operation.prepare_minimal_fallback(7).unwrap());
    assert!(!account.snapshot().poisoned());
}

#[test]
fn unrepresentable_qualification_preserves_counters_and_fallback() {
    for (members, fields) in [(usize::MAX, 31), (4, u64::MAX)] {
        let (mut operation, keys, account) = operation(128, 128, OperationControl::default());
        let before = counters(&mut operation, keys);
        assert!(matches!(
            operation.qualify_failure_validation_work(keys.work, members, fields),
            Err(StoreRuntimeError::InvalidPublicationPlan)
        ));
        assert_eq!(counters(&mut operation, keys), before);
        assert_eq!(
            operation.raw.failure_terminal_work_limits(),
            Some((2, 5, 5))
        );
        drop(operation.prepare_minimal_fallback(7).unwrap());
        assert!(!account.snapshot().poisoned());
    }
}

struct Stop {
    cancelled: Arc<AtomicBool>,
    polls: Arc<AtomicUsize>,
}
impl crate::CancellationSignal for Stop {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::Relaxed);
        self.cancelled.load(Ordering::Relaxed)
    }
}

#[test]
fn cancellation_before_qualification_retains_startup_fallback() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = OperationControl::default().with_cancellation(Stop {
        cancelled: cancelled.clone(),
        polls: Arc::new(AtomicUsize::new(0)),
    });
    let (mut operation, keys, account) = operation(128, 128, control);
    let before = counters(&mut operation, keys);
    cancelled.store(true, Ordering::Relaxed);
    assert!(matches!(
        operation.qualify_failure_validation_work(keys.work, 4, 31),
        Err(StoreRuntimeError::Control(_))
    ));
    assert_eq!(counters(&mut operation, keys), before);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
    let published = operation.prepare_minimal_fallback(7).unwrap().handoff();
    assert!(
        matches!(published.ledger_evidence.header().terminal_cause(),
        crate::StoreTerminalCause::Failure(cause)
            if cause.class() == crate::StorePreparationFailureClass::Control)
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn poison_is_checked_before_qualification_poll_or_policy_changes() {
    let polls = Arc::new(AtomicUsize::new(0));
    let control = OperationControl::default().with_cancellation(Stop {
        cancelled: Arc::new(AtomicBool::new(false)),
        polls: polls.clone(),
    });
    let (mut operation, keys, account) = operation(128, 128, control);
    let _ = account.test_record_poison(crate::AccountPoisonReason::OwnershipGraphUnverified);
    let before = account.snapshot();
    let before_polls = polls.load(Ordering::Relaxed);
    assert!(matches!(
        operation.qualify_failure_validation_work(keys.work, 4, 31),
        Err(StoreRuntimeError::AccountPoisoned(_))
    ));
    assert_eq!(account.snapshot(), before);
    assert_eq!(polls.load(Ordering::Relaxed), before_polls);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
}

type SemanticOutcome =
    CanonicalActivationOutcome<Owner, Progress, Unit, (), (), ScalarSequence<u8>, Tag>;

fn release_semantic_rejection(outcome: SemanticOutcome, account: &PhysicalStorageAccount) {
    let CanonicalActivationOutcome::NeverActivated(rejected) = outcome else {
        panic!("semantic rejection must not activate canonical storage");
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    drop(prepared.handoff());
    let snapshot = account.snapshot();
    assert!(!snapshot.poisoned());
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
}

#[test]
fn prior_ledger_failure_skips_the_transaction_and_preserves_healthy_failure_evidence() {
    let (mut operation, keys, account) = operation(4096, 4096, OperationControl::default());
    assert!(
        operation
            .producer_host()
            .charge_attempted(keys.work, u64::MAX)
            .is_err()
    );
    let transaction_ran = std::cell::Cell::new(false);
    let outcome: SemanticOutcome = keys
        .session(operation)
        .plan_and_activate_scoped(|planning| {
            transaction_ran.set(true);
            planning.reject((), 88)
        })
        .unwrap();
    assert!(!transaction_ran.get());
    let CanonicalActivationOutcome::NeverActivated(rejected) = outcome else {
        panic!("prior ledger rejection must retain its dormant failure owner");
    };
    assert!(matches!(
        rejected.cause(),
        StoreNeverActivatedCause::Runtime(StoreRuntimeError::SuccessOutcomeRejected)
    ));
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert!(matches!(
        published.ledger_evidence.header().terminal_cause(),
        crate::concrete_store::StoreTerminalCause::Failure(cause)
            if cause.class() == crate::concrete_store::StorePreparationFailureClass::Ledger
    ));
    drop(published);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn prior_failure_still_contains_a_panicking_transaction_capture_drop() {
    let (mut operation, keys, account) = operation(4096, 4096, OperationControl::default());
    assert!(
        operation
            .producer_host()
            .charge_attempted(keys.work, u64::MAX)
            .is_err()
    );
    let transaction_ran = std::cell::Cell::new(false);
    let dropped = std::cell::Cell::new(false);
    let capture = SemanticPanicDrop(&dropped);
    let ran = &transaction_ran;
    let outcome: Result<SemanticOutcome, _> =
        keys.session(operation)
            .plan_and_activate_scoped(move |planning| {
                let _capture = &capture;
                ran.set(true);
                planning.reject((), 89)
            });
    assert!(!transaction_ran.get());
    assert!(dropped.get());
    assert!(outcome.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn prior_outstanding_zero_or_completed_permit_cannot_use_the_healthy_failure_exit() {
    for admitted in [0_u64, 1] {
        let (mut operation, keys, account) = operation(4096, 4096, OperationControl::default());
        {
            let (work, _, _) = operation.execution().unwrap();
            let group = work.bind_group([keys.work]).unwrap();
            let mut permit = group.admit_observed([admitted]).unwrap();
            permit.complete([admitted]).unwrap();
            std::mem::forget(permit);
        }
        let transaction_ran = std::cell::Cell::new(false);
        let outcome: Result<SemanticOutcome, _> =
            keys.session(operation)
                .plan_and_activate_scoped(|planning| {
                    transaction_ran.set(true);
                    planning.reject((), 90)
                });
        assert!(!transaction_ran.get());
        assert!(outcome.is_err());
        assert!(account.snapshot().poisoned());
    }
}

fn semantic_counters(
    semantic: &CanonicalSemanticWorkSession<'_, '_, Owner, Progress, Unit>,
    keys: Keys,
) -> (u64, u64) {
    let state = semantic.work.dimension_state(keys.work).unwrap();
    (state.attempted(), state.consumed())
}

#[test]
fn semantic_execution_polls_at_entry_256_actions_and_finish() {
    for actions in [255_u64, 256, 257] {
        let polls = Arc::new(AtomicUsize::new(0));
        let control = OperationControl::default().with_cancellation(Stop {
            cancelled: Arc::new(AtomicBool::new(false)),
            polls: polls.clone(),
        });
        let (operation, keys, account) = operation(4096, 4096, control);
        let outcome: SemanticOutcome = keys
            .session(operation)
            .plan_and_activate_scoped(|mut planning| {
                let before_entry = polls.load(Ordering::Relaxed);
                let mut semantic = planning.begin_semantic_work().unwrap();
                assert_eq!(polls.load(Ordering::Relaxed), before_entry + 1);
                let baseline = semantic_counters(&semantic, keys);
                for completed in 1..=actions {
                    semantic.run_unit(|| ()).unwrap();
                    assert_eq!(
                        semantic_counters(&semantic, keys),
                        (baseline.0 + completed, baseline.1 + completed)
                    );
                    let block_polls = usize::try_from((completed - 1) / 256).unwrap();
                    assert_eq!(
                        polls.load(Ordering::Relaxed),
                        before_entry + 1 + block_polls
                    );
                }
                let before_finish = polls.load(Ordering::Relaxed);
                semantic.finish_rejected().unwrap();
                assert_eq!(polls.load(Ordering::Relaxed), before_finish + 1);
                planning.reject((), 81)
            })
            .unwrap();
        release_semantic_rejection(outcome, &account);
    }
}

#[test]
fn semantic_returned_error_completes_before_any_checkpoint() {
    let (operation, keys, account) = operation(4096, 4096, OperationControl::default());
    let outcome: SemanticOutcome = keys
        .session(operation)
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let baseline = semantic_counters(&semantic, keys);
            assert_eq!(
                semantic
                    .run_unit(|| Err::<(), _>("invalid domain value"))
                    .unwrap(),
                Err("invalid domain value")
            );
            assert_eq!(
                semantic_counters(&semantic, keys),
                (baseline.0 + 1, baseline.1 + 1)
            );
            semantic.finish_rejected().unwrap();
            planning.reject((), 82)
        })
        .unwrap();
    release_semantic_rejection(outcome, &account);
}

#[test]
fn semantic_consumed_limit_denies_the_callback_before_execution() {
    let (operation, keys, account) = operation(4096, 4096, OperationControl::default());
    let executed = std::cell::Cell::new(false);
    let outcome: SemanticOutcome = keys
        .session(operation)
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let baseline = semantic_counters(&semantic, keys);
            semantic
                .work
                .tighten_dimension_policy(
                    keys.work,
                    DimensionPolicy::counter(baseline.0 + 8, baseline.1, 0),
                )
                .unwrap();
            assert!(matches!(
                semantic.run_unit(|| executed.set(true)),
                Err(StoreRuntimeError::Ledger(_))
            ));
            assert!(!executed.get());
            assert_eq!(
                semantic_counters(&semantic, keys),
                (baseline.0 + 1, baseline.1)
            );
            assert!(matches!(
                semantic.finish_rejected(),
                Err(StoreRuntimeError::Ledger(_))
            ));
            planning.reject((), 83)
        })
        .unwrap();
    release_semantic_rejection(outcome, &account);
}

#[test]
fn semantic_scratch_boundaries_poll_before_the_action_cadence_is_due() {
    for boundary in ["reserve", "resize", "validate"] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let control = OperationControl::default().with_cancellation(Stop {
            cancelled: cancelled.clone(),
            polls: Arc::new(AtomicUsize::new(0)),
        });
        let (operation, keys, account) = operation(4096, 4096, control);
        let outcome: SemanticOutcome = keys
            .session(operation)
            .plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                let mut scratch = semantic.reserve(4).unwrap();
                semantic.run_unit(|| ()).unwrap();
                let before = scratch.raw.covered_bytes_for_test();
                let physical_before = account.snapshot();
                cancelled.store(true, Ordering::Relaxed);
                let result = match boundary {
                    "reserve" => semantic.reserve(8).map(drop),
                    "resize" => semantic.resize(&mut scratch, 8),
                    "validate" => semantic.validate_live_owner(&scratch),
                    _ => unreachable!(),
                };
                assert!(matches!(result, Err(StoreRuntimeError::Control(_))));
                assert_eq!(scratch.raw.covered_bytes_for_test(), before);
                assert_eq!(account.snapshot(), physical_before);
                assert!(matches!(
                    semantic.finish_rejected(),
                    Err(StoreRuntimeError::Control(_))
                ));
                drop(scratch);
                planning.reject((), 84)
            })
            .unwrap();
        release_semantic_rejection(outcome, &account);
    }
}

struct SemanticPanicDrop<'a>(&'a std::cell::Cell<bool>);

impl Drop for SemanticPanicDrop<'_> {
    fn drop(&mut self) {
        self.0.set(true);
        panic!("semantic capture drop panic");
    }
}

#[test]
fn semantic_capture_drop_failure_retains_the_returned_action_prefix() {
    let (operation, keys, account) = operation(4096, 4096, OperationControl::default());
    let dropped = std::cell::Cell::new(false);
    let result: Result<SemanticOutcome, _> =
        keys.session(operation)
            .plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                let baseline = semantic_counters(&semantic, keys);
                let capture = SemanticPanicDrop(&dropped);
                assert!(matches!(
                    semantic.run_unit(move || {
                        let _capture = &capture;
                    }),
                    Err(StoreRuntimeError::ConstructionInvariantUnverified)
                ));
                assert!(dropped.get());
                assert_eq!(
                    semantic_counters(&semantic, keys),
                    (baseline.0 + 1, baseline.1 + 1)
                );
                assert!(semantic.first_fault.as_ref().unwrap().contract_seen);
                assert!(matches!(
                    semantic.finish_rejected(),
                    Err(StoreRuntimeError::ConstructionInvariantUnverified)
                ));
                planning.reject((), 85)
            });
    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn semantic_denied_admission_catches_capture_drop_without_replacing_ledger_cause() {
    let (operation, keys, account) = operation(4096, 4096, OperationControl::default());
    let dropped = std::cell::Cell::new(false);
    let executed = std::cell::Cell::new(false);
    let result: Result<SemanticOutcome, _> =
        keys.session(operation)
            .plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                let baseline = semantic_counters(&semantic, keys);
                semantic
                    .work
                    .tighten_dimension_policy(
                        keys.work,
                        DimensionPolicy::counter(baseline.0 + 8, baseline.1, 0),
                    )
                    .unwrap();
                let capture = SemanticPanicDrop(&dropped);
                let executed_ref = &executed;
                assert!(matches!(
                    semantic.run_unit(move || {
                        let _capture = &capture;
                        executed_ref.set(true);
                    }),
                    Err(StoreRuntimeError::Ledger(_))
                ));
                assert!(dropped.get());
                assert!(!executed.get());
                assert_eq!(
                    semantic_counters(&semantic, keys),
                    (baseline.0 + 1, baseline.1)
                );
                let fault = semantic.first_fault.as_ref().unwrap();
                assert!(matches!(&fault.cause, StoreRuntimeError::Ledger(_)));
                assert!(fault.contract_seen);
                assert!(matches!(
                    semantic.finish_rejected(),
                    Err(StoreRuntimeError::Ledger(_))
                ));
                planning.reject((), 86)
            });
    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn semantic_body_and_capture_drop_panics_keep_only_the_completed_prefix() {
    for completed_prefix in [0_u64, 1] {
        let (operation, keys, account) = operation(4096, 4096, OperationControl::default());
        let dropped = std::cell::Cell::new(false);
        let result: Result<SemanticOutcome, _> =
            keys.session(operation)
                .plan_and_activate_scoped(|mut planning| {
                    let mut semantic = planning.begin_semantic_work().unwrap();
                    let baseline = semantic_counters(&semantic, keys);
                    if completed_prefix == 1 {
                        semantic.run_unit(|| ()).unwrap();
                    }
                    let capture = SemanticPanicDrop(&dropped);
                    let panicked = catch_unwind(AssertUnwindSafe(|| {
                        let _ = semantic.run_unit(move || -> () {
                            let _capture = &capture;
                            panic!("semantic body panic");
                        });
                    }));
                    assert!(panicked.is_err());
                    assert!(dropped.get());
                    assert_eq!(
                        semantic_counters(&semantic, keys),
                        (
                            baseline.0 + completed_prefix + 1,
                            baseline.1 + completed_prefix,
                        )
                    );
                    assert!(matches!(
                        semantic.finish_rejected(),
                        Err(StoreRuntimeError::ConstructionInvariantUnverified)
                    ));
                    planning.reject((), 87)
                });
        assert!(result.is_err());
        assert!(account.snapshot().poisoned());
    }
}
