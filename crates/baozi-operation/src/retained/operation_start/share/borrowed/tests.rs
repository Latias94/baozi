use super::*;
use crate::ledger::{CounterClass, DimensionPolicy, LedgerCause, LedgerLimits};
use crate::retained::{
    AdmissionCharge, AdmittedRetainedShareWork, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits, RetainedStorageLease,
    SharedRetainedStorage,
};

struct Owner;
impl OperationOwner for Owner {}

fn ledger(attempted: u64, consumed: u64) -> Ledger<Owner, u8, u8> {
    Ledger::try_new(
        LedgerLimits::new(
            16_384,
            [(1, DimensionPolicy::counter(attempted, consumed, 0))],
        )
        .unwrap(),
        &[1],
    )
    .unwrap()
}

fn source() -> (PhysicalStorageAccount, SharedRetainedStorage<u64>) {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(16_384, 8).unwrap())
        .unwrap();
    let source = {
        let mut admission = account
            .try_admit_retained(
                AdmissionCharge::new(SharedRetainedStorage::<u64>::header_bytes().unwrap(), 1),
                AdmittedRetainedShareWork { private: () },
            )
            .unwrap();
        RetainedStorageLease::from_admission(&mut admission)
            .try_bind_shared(37)
            .unwrap()
    };
    (account, source)
}

#[test]
fn borrowed_share_uses_one_backing_and_resolves_both_duplicate_row_prefixes() {
    let mut ledger = ledger(2, 2);
    let (account, source) = source();
    let baseline = account.snapshot();
    let share =
        try_admit_borrowed_retained_share_work(&mut ledger, RetainedHandleSharePlan::new(1, 1))
            .unwrap()
            .try_share_allocation(source.allocation())
            .unwrap_or_else(|_| panic!("admitted clone and handle complete"));
    let shared = SharedRetainedStorage::from_allocation(share);
    assert!(std::ptr::eq(source.as_ref(), shared.as_ref()));
    let work = ledger.dimension_evidence(1).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (2, 2));
    ledger.check_work_quiescent().unwrap();
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        baseline.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        baseline.live_retained_allocations()
    );
    drop(source);
    assert_eq!(*shared.as_ref(), 37);
    drop(shared);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn borrowed_share_denial_keeps_attempts_and_never_clones_or_reserves_a_suffix() {
    for (attempted, consumed, class) in [
        (1, 1, CounterClass::Attempted),
        (2, 1, CounterClass::Consumed),
    ] {
        let mut ledger = ledger(attempted, consumed);
        let (account, source) = source();
        let baseline = account.snapshot();
        let result =
            try_admit_borrowed_retained_share_work(&mut ledger, RetainedHandleSharePlan::new(1, 1));
        assert!(matches!(
            result,
            Err(RetainedWriterError::Ledger(LedgerCause::LimitExceeded {
                class: actual,
                dimension: 1,
                value: 2,
                limit: 1,
            })) if actual == class
        ));
        drop(result);
        let work = ledger.dimension_evidence(1).unwrap();
        assert_eq!((work.attempted(), work.consumed()), (2, 0));
        ledger.check_work_quiescent().unwrap();
        assert_eq!(
            account.snapshot().live_retained_bytes(),
            baseline.live_retained_bytes()
        );
        drop(source);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert!(!account.snapshot().poisoned());
    }
}

#[test]
fn borrowed_share_abandonment_preserves_completed_work_and_latches_integrity() {
    for completed in [false, true] {
        let mut ledger = ledger(2, 2);
        let mut work =
            try_admit_borrowed_retained_share_work(&mut ledger, RetainedHandleSharePlan::new(1, 1))
                .unwrap();
        if completed {
            work.complete_reference_clone().unwrap();
        }
        drop(work);
        assert!(matches!(
            ledger.check_work_quiescent(),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        ));
        let failure = ledger
            .finish()
            .expect_err("abandoned work cannot finalize cleanly");
        assert!(matches!(
            failure.cause(),
            LedgerCause::FinalSnapshotShapeMismatch
        ));
        let work = failure.snapshot().dimension(1).unwrap();
        assert_eq!(
            (work.attempted(), work.consumed()),
            (2, u64::from(completed))
        );
    }
}

#[test]
fn borrowed_share_explicit_partial_resolution_releases_only_the_unfinished_suffix() {
    for completed in [false, true] {
        let mut ledger = ledger(2, 2);
        let mut work =
            try_admit_borrowed_retained_share_work(&mut ledger, RetainedHandleSharePlan::new(1, 1))
                .unwrap();
        if completed {
            work.complete_reference_clone().unwrap();
        }
        work.finish_partial().unwrap();
        let work = ledger.dimension_evidence(1).unwrap();
        assert_eq!(
            (work.attempted(), work.consumed()),
            (2, u64::from(completed))
        );
        ledger.check_work_quiescent().unwrap();
    }
}

#[test]
fn borrowed_share_clone_completion_failure_returns_its_owner_and_releases_only_the_suffix() {
    struct FailAfterClone<'op>(BorrowedRetainedShareWork<'op, u8>);

    impl AdmittedShareWork<u8> for FailAfterClone<'_> {
        fn complete_reference_clone(&mut self) -> Result<(), LedgerCause<u8>> {
            self.0.complete_reference_clone()?;
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        }

        fn complete_typed_handle(self) -> Result<(), LedgerCause<u8>> {
            panic!("the failed clone checkpoint cannot construct a typed handle")
        }

        fn finish_partial(self) -> Result<(), LedgerCause<u8>> {
            self.0.finish_partial()
        }
    }

    let mut ledger = ledger(2, 2);
    let (account, source) = source();
    let work =
        try_admit_borrowed_retained_share_work(&mut ledger, RetainedHandleSharePlan::new(1, 1))
            .unwrap();
    let Err(failure) =
        share_allocation_after_admitted_work(FailAfterClone(work), source.allocation())
    else {
        panic!("injected failure after the real reference clone")
    };
    let (shared, cause) = failure.into_parts();
    assert!(matches!(cause, LedgerCause::FinalSnapshotShapeMismatch));
    let work = ledger.dimension_evidence(1).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (2, 1));
    ledger.check_work_quiescent().unwrap();
    drop(source);
    assert!(account.snapshot().live_retained_bytes() > 0);
    drop(shared);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn startup_shares_preserve_non_unit_duplicate_charges_and_the_admitted_physical_prefix() {
    for (attempted_limit, consumed_limit, completed_shares, attempted, consumed, denied) in [
        (20, 20, 4, 20, 20, None),
        (14, 14, 2, 15, 10, Some(CounterClass::Attempted)),
        (20, 14, 2, 15, 10, Some(CounterClass::Consumed)),
    ] {
        let mut ledger = ledger(attempted_limit, consumed_limit);
        let (account, source) = source();
        let baseline = account.snapshot();
        let shares_before = account.share_call_count();
        let result = {
            let (mut work, _) = ledger.split_internal();
            super::super::try_admit_start_share_work(
                &mut work,
                &account,
                StartupAccountShareEnvelope::four_identical([(1, 2), (1, 3)]),
            )
        };
        if let Some(class) = denied {
            assert!(matches!(
                result,
                Err(RetainedWriterError::Ledger(LedgerCause::LimitExceeded {
                    class: actual,
                    dimension: 1,
                    value: 15,
                    limit: 14,
                })) if actual == class
            ));
        } else {
            assert!(result.is_ok());
        }
        drop(result);
        let work = ledger.dimension_evidence(1).unwrap();
        assert_eq!((work.attempted(), work.consumed()), (attempted, consumed));
        ledger.check_work_quiescent().unwrap();
        assert_eq!(account.share_call_count(), shares_before + completed_shares);
        assert_eq!(
            account.snapshot().live_retained_bytes(),
            baseline.live_retained_bytes()
        );
        assert_eq!(
            account.snapshot().live_retained_allocations(),
            baseline.live_retained_allocations()
        );
        assert_eq!(*source.as_ref(), 37);
        drop(source);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert!(!account.snapshot().poisoned());
    }
}
