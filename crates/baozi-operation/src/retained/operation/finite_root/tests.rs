use super::*;
use crate::ledger::{DimensionPolicy, OutputCommitment};
use crate::retained::{
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Owner;
impl OperationOwner for Owner {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Phase;

impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        0
    }
    fn stable_id(self) -> &'static str {
        "finite-root"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Unit;

impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

struct Payload(Arc<AtomicUsize>);

impl Drop for Payload {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

type Root = FiniteInvocationRoot<Owner, u8, Phase, Unit, u8, Payload, (), ()>;

fn account() -> PhysicalStorageAccount {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 1).unwrap())
        .unwrap()
}

fn limits() -> FiniteLedgerLimits<u8> {
    let policies = [1_u8, 2, 3, 4, 5, 6].map(|dimension| {
        let policy = if dimension == 6 {
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX)
        } else {
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        };
        (dimension, policy)
    });
    FiniteLedgerLimits::try_new(u64::MAX, u64::MAX, policies).unwrap()
}

fn plan() -> FiniteRetainedOperationStartPlan<u8, u8> {
    FiniteRetainedOperationStartPlan::new(
        RetainedWriterPlan::new(3, 1, 4, RetainedBatchLimits::new(2, 1)),
        RetainedHandleSharePlan::new(6, 6),
        RetainedAllocationPlan::new(0, 0).unwrap(),
        OutputCommitment::new(6, 4),
    )
}

fn admitted_actions(account: &PhysicalStorageAccount) -> u64 {
    crate::retained::account::lock_recovering_poison(&account.inner.value().linear)
        .admitted_account_actions
}

#[test]
fn finite_opened_stamp_rejects_foreign_fields_and_binds_original_tables_without_readmission() {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 128).unwrap())
        .unwrap();
    let mut source_entry = FiniteEntryAdmission::new(&account);
    let mut target_entry = FiniteEntryAdmission::new(&account);
    let source_drops = Arc::new(AtomicUsize::new(0));
    let target_drops = Arc::new(AtomicUsize::new(0));
    let mut source = Root::new();
    let mut target = Root::new();
    let table = crate::retained::RetainedBlockTablePlan::new(
        RetainedWriterPlan::new(2, 1, 4, RetainedBatchLimits::new(2, 1)),
        1,
        1,
    );
    let opened_source = {
        let operation = source
            .try_start(
                OperationControl::default(),
                limits(),
                [1, 2, 3],
                &mut source_entry,
                plan(),
                || Ok(Payload(Arc::clone(&source_drops))),
            )
            .unwrap();
        let ordinary = operation
            .open_publication_borrow(crate::retained::RetainedPublicationPlan::success(
                table, table,
            ))
            .unwrap_or_else(|_| panic!("the actual source finite publication must open"));
        ordinary
            .opened_stamp()
            .expect("the actual source opened fields retain their stamp")
    };
    let opened_target = {
        let operation = target
            .try_start(
                OperationControl::default(),
                limits(),
                [1, 2, 3],
                &mut target_entry,
                plan(),
                || Ok(Payload(Arc::clone(&target_drops))),
            )
            .unwrap();
        let ordinary = operation
            .open_publication_borrow(crate::retained::RetainedPublicationPlan::success(
                table, table,
            ))
            .unwrap_or_else(|_| panic!("the actual target finite publication must open"));
        ordinary
            .opened_stamp()
            .expect("the actual target opened fields retain their stamp")
    };
    let physical = account.snapshot();
    let shares = account.share_call_count();
    let actions = admitted_actions(&account);
    let opened_source = match target.fields().bind_opened_stamp(opened_source) {
        Err(original) => original,
        Ok(()) => panic!("a foreign finite owner must not consume the source stamp"),
    };
    assert_eq!(account.snapshot(), physical);
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    assert!(source.fields().bind_opened_stamp(opened_source).is_ok());
    assert!(target.fields().bind_opened_stamp(opened_target).is_ok());
    assert_eq!(account.snapshot(), physical);
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    drop(source);
    drop(target);
    assert_eq!(source_drops.load(Ordering::Relaxed), 1);
    assert_eq!(target_drops.load(Ordering::Relaxed), 1);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn finite_failed_open_preserves_original_identity_cause_and_writable_recovery() {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 128).unwrap())
        .unwrap();
    let mut source_entry = FiniteEntryAdmission::new(&account);
    let mut target_entry = FiniteEntryAdmission::new(&account);
    let drops = Arc::new(AtomicUsize::new(0));
    let mut source = Root::new();
    let mut target = Root::new();
    let failed = {
        let operation = source
            .try_start(
                OperationControl::default(),
                limits(),
                [1, 2, 3],
                &mut source_entry,
                plan(),
                || Ok(Payload(Arc::clone(&drops))),
            )
            .unwrap();
        let unknown_phase = crate::retained::RetainedBlockTablePlan::new(
            RetainedWriterPlan::new(99, 1, 4, RetainedBatchLimits::new(2, 1)),
            1,
            1,
        );
        match operation.open_publication_borrow(crate::retained::RetainedPublicationPlan::success(
            unknown_phase,
            unknown_phase,
        )) {
            Err(failed) => failed,
            Ok(_) => panic!("an unregistered original phase must reject before allocation"),
        }
    };
    let operation = target
        .try_start(
            OperationControl::default(),
            limits(),
            [1, 2, 3],
            &mut target_entry,
            plan(),
            || Ok(Payload(Arc::clone(&drops))),
        )
        .unwrap();
    let _target_identity = operation.invocation_identity();
    let physical = account.snapshot();
    let shares = account.share_call_count();
    let actions = admitted_actions(&account);
    let failed = match target.fields().bind_publication_failure(failed) {
        Err(original) => original,
        Ok(_) => panic!("a foreign finite owner must return the original open failure"),
    };
    assert_eq!(account.snapshot(), physical);
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    let failed = match source.fields().bind_publication_failure(failed) {
        Ok(original) => original,
        Err(_) => panic!("the original finite failure must bind to its own unchanged owner"),
    };
    assert!(matches!(
        failed.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    assert!(!failed.is_fallback_only());
    assert!(!failed.is_fail_closed());
    assert_eq!(account.snapshot(), physical);
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    let table = crate::retained::RetainedBlockTablePlan::new(
        RetainedWriterPlan::new(2, 1, 4, RetainedBatchLimits::new(2, 1)),
        1,
        1,
    );
    let opened = {
        let operation = source
            .loan()
            .expect("clean rejected open preserves the same writable owner");
        let ordinary = operation
            .open_publication_borrow(crate::retained::RetainedPublicationPlan::success(
                table, table,
            ))
            .unwrap_or_else(|_| panic!("the original owner must still open a valid plan"));
        ordinary
            .opened_stamp()
            .expect("the actual retry has its own opened stamp")
    };
    assert!(source.fields().bind_opened_stamp(opened).is_ok());
    drop(source);
    drop(target);
    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn short_ordinary_assembly_borrows_leave_real_members_and_cleanup_with_original_owner() {
    type PopulatedRoot =
        FiniteInvocationRoot<Owner, u8, Phase, Unit, u8, Payload, Payload, Payload>;
    for (forgotten, forgotten_assembly) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let factory = PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
        )
        .unwrap();
        let account = factory
            .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 128).unwrap())
            .unwrap();
        let mut entry = FiniteEntryAdmission::new(&account);
        let fallback_drops = Arc::new(AtomicUsize::new(0));
        let member_drops = Arc::new(AtomicUsize::new(0));
        let mut root = PopulatedRoot::new();
        let operation = root
            .try_start(
                OperationControl::default(),
                limits(),
                [1, 2, 3],
                &mut entry,
                plan(),
                || Ok(Payload(Arc::clone(&fallback_drops))),
            )
            .unwrap();
        let writer = RetainedWriterPlan::new(2, 1, 4, RetainedBatchLimits::new(2, 1));
        let table = super::super::super::RetainedBlockTablePlan::new(writer, 1, 1);
        let mut assembly = operation
            .try_publication_assembly(super::super::super::RetainedPublicationPlan::success(
                table, table,
            ))
            .unwrap_or_else(|_| {
                panic!("the actual finite tables must open before an ordinary reborrow")
            });
        let _: super::super::super::CanonicalRetainedSlot<Payload, ()> = assembly
            .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                Ok(emplacer.place(Payload(Arc::clone(&member_drops)), ()))
            })
            .unwrap_or_else(|_| panic!("the original finite canonical member must retain"));
        let physical = account.snapshot();
        let shares = account.share_call_count();
        let actions = admitted_actions(&account);
        let ordinary = assembly.borrow_ordinary();
        assert_eq!(ordinary.loan().canonical_len(), 1);
        assert!(
            !ordinary
                .loan()
                .operation
                .as_ref()
                .unwrap()
                .common
                .terminal_is_outstanding()
        );
        if forgotten {
            std::mem::forget(ordinary);
        } else {
            drop(ordinary);
        }
        assert_eq!(account.snapshot(), physical);
        assert_eq!(
            (account.share_call_count(), admitted_actions(&account)),
            (shares, actions)
        );
        assert_eq!(member_drops.load(Ordering::Relaxed), 0);
        assert_eq!(fallback_drops.load(Ordering::Relaxed), 0);
        assert_eq!(assembly.canonical_len(), 1);
        assert!(
            !assembly
                .operation
                .as_ref()
                .unwrap()
                .common
                .terminal_is_outstanding()
        );
        if forgotten_assembly {
            std::mem::forget(assembly);
            assert_eq!(member_drops.load(Ordering::Relaxed), 0);
            assert_eq!(account.snapshot(), physical);
        } else {
            drop(assembly);
            assert_eq!(member_drops.load(Ordering::Relaxed), 1);
            assert!(account.snapshot().live_retained_bytes() < physical.live_retained_bytes());
        }
        assert_eq!(fallback_drops.load(Ordering::Relaxed), 0);
        drop(root);
        assert_eq!(member_drops.load(Ordering::Relaxed), 1);
        assert_eq!(fallback_drops.load(Ordering::Relaxed), 1);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert!(!account.snapshot().poisoned());
    }
}

#[test]
fn failed_finite_ledger_constructor_leaves_original_control_in_its_outer_slot() {
    struct Signal(Arc<AtomicUsize>);
    impl crate::CancellationSignal for Signal {
        fn is_cancelled(&self) -> bool {
            false
        }
    }
    impl Drop for Signal {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let account = account();
    let mut entry = FiniteEntryAdmission::new(&account);
    let drops = Arc::new(AtomicUsize::new(0));
    let mut control =
        Some(OperationControl::default().with_cancellation(Signal(Arc::clone(&drops))));
    let mut root = Root::new();
    let error = root
        .bind_with_control(&mut control, limits(), [1, 1, 3], &mut entry, plan())
        .err()
        .expect("duplicate phases reject before installing the actual finite coordinator");
    assert!(matches!(
        error,
        RetainedOperationStartError::Writer(RetainedWriterError::Ledger(
            LedgerCause::DuplicatePhase
        ))
    ));
    assert!(control.is_some() && entry.pending.is_some() && root.common.is_none());
    assert!(root.started);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    assert_eq!(admitted_actions(&account), 1);
    assert_eq!(account.share_call_count(), 0);
    drop(root);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    drop(control);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn early_start_and_attachment_rejection_dispose_uninvoked_builder_under_original_account() {
    struct PanickingCapture<'a>(&'a AtomicUsize);
    impl Drop for PanickingCapture<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("injected early finite builder capture disposal panic");
        }
    }

    for rejected_at_bind in [false, true] {
        let account = account();
        let mut entry = FiniteEntryAdmission::new(&account);
        let mut root = Root::new();
        let drops = AtomicUsize::new(0);
        let calls = AtomicUsize::new(0);
        let capture = PanickingCapture(&drops);
        let mut plan = plan();
        plan.terminal_writer.limits = if rejected_at_bind {
            RetainedBatchLimits::new(0, 1)
        } else {
            RetainedBatchLimits::new(usize::MAX, 1)
        };
        let failure = root
            .try_start(
                OperationControl::default(),
                limits(),
                [1, 2, 3],
                &mut entry,
                plan,
                || {
                    let _capture = capture;
                    calls.fetch_add(1, Ordering::Relaxed);
                    unreachable!(
                        "terminal limits reject before invoking the finite fallback builder"
                    )
                },
            )
            .err()
            .expect("invalid terminal limits must return the original start failure");
        assert!(matches!(
            failure.cause,
            RetainedOperationStartError::InvalidTerminalLimits
        ));
        assert!(!failure.has_residue());
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(entry.pending.is_some(), !rejected_at_bind);
        assert_eq!(root.common.is_some(), !rejected_at_bind);
        assert_eq!(account.share_call_count(), 0);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(account.snapshot().poison_event_count(), 1);
        assert_eq!(
            account.snapshot().first_poison_evidence().unwrap().reason(),
            AccountPoisonReason::RetainedBuilderPanicked
        );
        drop(root);
        assert_eq!(account.snapshot().poison_event_count(), 1);
    }
}

#[test]
fn foreign_deferred_preparation_returns_original_authority_without_mutating_target_root() {
    let first_account = account();
    let second_account = account();
    let mut first_entry = FiniteEntryAdmission::new(&first_account);
    let mut second_entry = FiniteEntryAdmission::new(&second_account);
    let first_drops = Arc::new(AtomicUsize::new(0));
    let second_drops = Arc::new(AtomicUsize::new(0));
    let mut first = Root::new();
    let mut second = Root::new();
    let _ = first
        .try_start(
            OperationControl::default(),
            limits(),
            [1, 2, 3],
            &mut first_entry,
            plan(),
            || Ok(Payload(Arc::clone(&first_drops))),
        )
        .unwrap();
    let _ = second
        .try_start(
            OperationControl::default(),
            limits(),
            [1, 2, 3],
            &mut second_entry,
            plan(),
            || Ok(Payload(Arc::clone(&second_drops))),
        )
        .unwrap();
    let identity = second.invocation_identity();
    let physical = second_account.snapshot();
    let shares = second_account.share_call_count();
    let actions = admitted_actions(&second_account);
    let before = second
        .terminal_catalog()
        .unwrap()
        .dimension_evidence(6)
        .unwrap();
    let prepared = first
        .loan()
        .unwrap()
        .prepare_minimal_fallback_deferred(PreparationFailureCode::domain(71))
        .unwrap();
    let failure = second
        .fields()
        .finish_deferred(prepared)
        .err()
        .expect("foreign Ready authority must be returned before effects");
    let FiniteRootTerminalFailure::Rejected(FiniteRootTerminalRejection::RejectedPreparation(
        prepared,
    )) = failure
    else {
        panic!("a foreign preparation must retain its original ownerless authority");
    };
    assert_eq!(second.invocation_identity(), identity);
    assert_eq!(
        second.terminal_catalog().unwrap().dimension_evidence(6),
        Ok(before)
    );
    assert_eq!(second_account.snapshot(), physical);
    assert_eq!(
        (
            second_account.share_call_count(),
            admitted_actions(&second_account)
        ),
        (shares, actions)
    );
    assert!(!second.contained && second.terminal.is_none());
    assert!(
        !second
            .common
            .as_ref()
            .unwrap()
            .lifecycle
            .terminal_outstanding
    );
    let output = first
        .fields()
        .finish_deferred(prepared)
        .unwrap_or_else(|_| {
            panic!("the returned Ready authority must still finish its original root")
        });
    drop(first);
    drop(output);
    drop(second);
    assert_eq!(first_drops.load(Ordering::Relaxed), 1);
    assert_eq!(second_drops.load(Ordering::Relaxed), 1);
    assert_eq!(second_account.snapshot().live_retained_bytes(), 0);
    assert!(!first_account.snapshot().poisoned() && !second_account.snapshot().poisoned());
}

#[test]
fn forgotten_pending_keeps_the_original_admission_in_the_outer_entry_and_start_is_once() {
    let account = account();
    let mut entry = FiniteEntryAdmission::new(&account);
    let mut root = Root::new();
    let pending = root
        .bind(
            OperationControl::default(),
            limits(),
            [1, 2, 3],
            &mut entry,
            plan(),
        )
        .unwrap_or_else(|cause| panic!("finite production bind failed: {cause}"));
    let _forgotten = std::mem::ManuallyDrop::new(pending);
    assert!(entry.pending.is_some());
    assert_eq!(admitted_actions(&account), 1);
    let repeated = root.bind(
        OperationControl::default(),
        limits(),
        [1, 2, 3],
        &mut entry,
        plan(),
    );
    assert!(matches!(
        repeated,
        Err(RetainedOperationStartError::InvocationAlreadyStarted)
    ));
    drop(repeated);
    drop(root);
    assert!(entry.pending.is_some());
    assert_eq!(account.share_call_count(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn real_root_attachment_has_one_fallback_and_three_original_shares_and_releases_when_forgotten() {
    for poisoned in [false, true] {
        let account = account();
        let mut entry = FiniteEntryAdmission::new(&account);
        let drops = Arc::new(AtomicUsize::new(0));
        let mut root = Root::new();
        let operation = root
            .try_start(
                OperationControl::default(),
                limits(),
                [1, 2, 3],
                &mut entry,
                plan(),
                || Ok(Payload(Arc::clone(&drops))),
            )
            .unwrap_or_else(|cause| panic!("finite production attach failed: {cause:?}"));
        let _forgotten = std::mem::ManuallyDrop::new(operation);
        assert!(entry.pending.is_none());
        assert_eq!(account.share_call_count(), 3);
        assert_eq!(account.snapshot().live_retained_allocations(), 1);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        let poison = poisoned
            .then(|| account.test_record_poison(AccountPoisonReason::RetainedBuilderPanicked));
        let first_poison = account.snapshot().first_poison_evidence();
        drop(root);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(account.snapshot().first_poison_evidence(), first_poison);
        assert_eq!(account.snapshot().poisoned(), poison.is_some());
    }
}

#[test]
fn unattached_finish_consumes_the_original_outer_admission_without_startup_shares() {
    let account = account();
    let mut entry = FiniteEntryAdmission::new(&account);
    let mut root = Root::new();
    let pending = root
        .bind(
            OperationControl::default(),
            limits(),
            [1, 2, 3],
            &mut entry,
            plan(),
        )
        .unwrap_or_else(|cause| panic!("finite production bind failed: {cause}"));
    let binding = pending
        .defer_unattached(None)
        .unwrap_or_else(|_| panic!("unattached deferral failed"));
    assert!(entry.pending.is_some());
    let terminal = root
        .finish_unattached(binding, &mut entry, FinalProgress::new(Phase, Unit, 1))
        .unwrap_or_else(|_| panic!("finite production unattached close failed"));
    assert!(entry.pending.is_none());
    assert!(root.common.is_none());
    let remainder = root.into_extracted_remainder();
    assert!(remainder.test_has_started());
    assert!(!remainder.must_retain_domain());
    drop(remainder);
    assert_eq!(terminal.snapshot.dimensions().len(), 6);
    assert_eq!(terminal.snapshot.phases().len(), 3);
    assert_eq!(terminal.snapshot.current_live_bytes(), 0);
    assert_eq!(terminal.snapshot.active_reservations(), 0);
    assert_eq!(terminal.cause, None);
    assert_eq!(account.share_call_count(), 0);
    assert_eq!(admitted_actions(&account), 1);
}

#[test]
fn builder_panic_returns_original_startup_owners_after_the_root_and_entry_end() {
    let account = account();
    let error = {
        let mut entry = FiniteEntryAdmission::new(&account);
        let mut root = Root::new();
        let error = match root.try_start(
            OperationControl::default(),
            limits(),
            [1, 2, 3],
            &mut entry,
            plan(),
            || panic!("production finite fallback builder panic"),
        ) {
            Ok(_) => panic!("the panicking builder cannot attach"),
            Err(error) => error,
        };
        assert!(entry.pending.is_none());
        let FiniteAttachmentResidue::Retained { _owner: retained } = &error.residue else {
            panic!("the builder failure must retain its actual construction owners");
        };
        assert!(
            retained
                .account()
                .is_some_and(|owner| owner.is_same_account(&account))
        );
        let state = root.fields().into_owned();
        assert!(state.common.is_some());
        drop(root);
        (error, state)
    };
    assert_eq!(account.share_call_count(), 3);
    assert_eq!(account.snapshot().live_retained_allocations(), 1);
    let FiniteAttachmentResidue::Retained { _owner: retained } = &error.0.residue else {
        panic!("the original construction owners must outlive the root and entry");
    };
    assert!(retained.account().is_some());
    drop(error);
    assert_eq!(account.snapshot().live_retained_allocations(), 1);
    assert_eq!(
        account
            .snapshot()
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );
}

#[test]
fn extracted_remainder_rejects_live_common_before_transferring_physical_owners() {
    let account = account();
    let mut entry = FiniteEntryAdmission::new(&account);
    let drops = Arc::new(AtomicUsize::new(0));
    let mut root = Root::new();
    let operation = root
        .try_start(
            OperationControl::default(),
            limits(),
            [1, 2, 3],
            &mut entry,
            plan(),
            || Ok(Payload(Arc::clone(&drops))),
        )
        .unwrap_or_else(|cause| panic!("finite attach failed: {cause:?}"));
    let _forgotten = std::mem::ManuallyDrop::new(operation);
    assert!(root.common.is_some());
    assert_eq!(account.snapshot().live_retained_allocations(), 1);
    assert!(catch_unwind(AssertUnwindSafe(|| root.into_extracted_remainder())).is_err());
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}
