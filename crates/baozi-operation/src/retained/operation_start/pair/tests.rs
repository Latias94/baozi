use super::{RetainedWriterError, StartupRetainedPartGuard};
use crate::ledger::{
    AttributionBatchPlan, Ledger, LedgerTestProbe, OutputCommitment, StartupAccountShareEnvelope,
    StartupAttributionBatchPlans, StartupLedgerPlanParts, StartupRetainedAttribution,
};
use crate::retained::account::AdmissionCharge;
use crate::retained::allocation::{
    RetainedAllocationError, RetainedAllocationPlan, RetainedBuildFailureCode,
};
use crate::retained::evidence::{TerminalEvidenceBackingPlan, TerminalLedgerEvidence};
use crate::retained::startup::{StartupCrossPlanePlan, StartupPhysicalManifest};
use crate::retained::{
    PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
};
use crate::{DimensionPolicy, LedgerLimits, OperationOwner};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug)]
struct Owner;
impl OperationOwner for Owner {}

type ReadyGuard<'account> = StartupRetainedPartGuard<'account, Owner, u8, u8, u64>;
type Observation = ((u64, u64), u64, u64);

fn account() -> PhysicalStorageAccount {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX).unwrap())
        .unwrap()
}

fn ready_guard(
    account: &PhysicalStorageAccount,
) -> (Ledger<Owner, u8, u8>, ReadyGuard<'_>, [AdmissionCharge; 2]) {
    let fallback_plan = RetainedAllocationPlan::new(0, 0).unwrap();
    let footprint = TerminalEvidenceBackingPlan::<u8, u8>::try_new(2, 1)
        .unwrap()
        .footprint();
    let evidence_plan =
        RetainedAllocationPlan::new(footprint.bytes(), footprint.allocations()).unwrap();
    let fallback = account
        .checked_retained_charge::<u64>(fallback_plan)
        .unwrap();
    let evidence = account
        .checked_retained_charge::<TerminalLedgerEvidence<u8, u8>>(evidence_plan)
        .unwrap();
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (
                1,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
            ),
            (
                2,
                DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            ),
        ],
    )
    .unwrap();
    let admission = account.try_admit_operation_start().unwrap();
    let mut ledger = Ledger::<Owner, u8, u8>::try_new(limits, &[1]).unwrap();
    let parts = StartupLedgerPlanParts {
        batches: StartupAttributionBatchPlans::new(
            AttributionBatchPlan::try_new(1, 1, 1, 1).unwrap(),
            AttributionBatchPlan::try_new(1, 1, 1, 1).unwrap(),
            AttributionBatchPlan::try_new(1, 1, 1, 1).unwrap(),
            AttributionBatchPlan::try_new(1, 1, 1, 1).unwrap(),
        ),
        retained_attributions: [
            StartupRetainedAttribution::new(1, 1, fallback.bytes),
            StartupRetainedAttribution::new(1, 1, evidence.bytes),
        ],
        account_shares: StartupAccountShareEnvelope::four_identical([(2, 1), (2, 2)]),
        commitments: [OutputCommitment::new(2, 1), OutputCommitment::new(2, 1)],
    };
    let manifest = StartupPhysicalManifest::try_new(fallback, evidence).unwrap();
    let plan = StartupCrossPlanePlan::try_new(parts, manifest).unwrap();
    let ready = admission
        .try_prepare_startup_cross_plane(&mut ledger, plan)
        .unwrap_or_else(|_| panic!("real cross-plane startup must reach Ready"));
    let (baseline, admission_guard) = ready.split();
    let guard =
        StartupRetainedPartGuard::new(admission_guard, baseline, fallback_plan, evidence_plan);
    (ledger, guard, [fallback, evidence])
}

fn observe(account: &PhysicalStorageAccount, ledger: &LedgerTestProbe<u8, u8>) -> Observation {
    let snapshot = account.snapshot();
    (
        (
            snapshot.live_retained_bytes(),
            snapshot.live_retained_allocations(),
        ),
        ledger.current_live_bytes(),
        ledger.active_reservations(),
    )
}

struct EvidenceCapture<'probe> {
    account: &'probe PhysicalStorageAccount,
    ledger: &'probe LedgerTestProbe<u8, u8>,
    observed: &'probe Cell<Option<Observation>>,
    invoked: &'probe Cell<bool>,
    panics: bool,
}

impl Drop for EvidenceCapture<'_> {
    fn drop(&mut self) {
        self.observed.set(Some(observe(self.account, self.ledger)));
        if self.panics {
            panic!("uninvoked evidence capture drop panicked");
        }
    }
}

fn run_ready_capture_case(fallback_panics: bool, capture_panics: bool) {
    let account = account();
    let (mut ledger, guard, [fallback, evidence]) = ready_guard(&account);
    let probe = ledger.test_probe();
    let before = observe(&account, &probe);
    assert_eq!(
        before.0,
        (
            fallback.bytes + evidence.bytes,
            fallback.allocations + evidence.allocations
        )
    );
    assert_eq!(before.2, 6);
    assert_eq!(account.share_call_count(), 4);
    let observed = Cell::new(None);
    let fallback_invoked = Cell::new(false);
    let evidence_invoked = Cell::new(false);
    let capture = EvidenceCapture {
        account: &account,
        ledger: &probe,
        observed: &observed,
        invoked: &evidence_invoked,
        panics: capture_panics,
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        guard.try_build(
            || {
                fallback_invoked.set(true);
                if fallback_panics {
                    panic!("fallback builder panicked");
                }
                Err(RetainedBuildFailureCode::new(17))
            },
            move || {
                capture.invoked.set(true);
                drop(capture);
                panic!("evidence builder must not run after fallback failure");
            },
        )
    }));
    assert!(fallback_invoked.get());
    assert!(!evidence_invoked.get());
    assert_eq!(observed.get(), Some(before));
    let error = match outcome {
        Ok(Err(error)) => error,
        _ => panic!("Ready builder failure must be contained as an error"),
    };
    let (physical, live_bytes, reservations) = observe(&account, &probe);
    assert_eq!(
        account.snapshot().poisoned(),
        fallback_panics || capture_panics
    );
    assert_eq!(account.share_call_count(), 4);
    let work = ledger.dimension_evidence(2).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (12, 12));
    if capture_panics {
        assert!(matches!(
            error,
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ));
        assert_eq!(
            account.snapshot().first_poison_evidence().unwrap().reason(),
            crate::retained::AccountPoisonReason::RetainedBuilderPanicked
        );
        assert_eq!(physical, before.0);
        assert!(live_bytes > fallback.bytes + evidence.bytes);
        assert_eq!(reservations, 4);
    } else if fallback_panics {
        assert!(matches!(
            error,
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ));
        assert_eq!(physical, (fallback.bytes, fallback.allocations));
        assert!(live_bytes > fallback.bytes);
        assert_eq!(reservations, 2);
    } else {
        assert!(
            matches!(error, RetainedWriterError::Physical(RetainedAllocationError::Build(code))
            if code == RetainedBuildFailureCode::new(17))
        );
        assert_eq!((physical, live_bytes, reservations), ((0, 0), 0, 0));
    }
}

#[test]
fn ready_fallback_failure_disposes_evidence_capture_before_cleanup() {
    for fallback_panics in [false, true] {
        run_ready_capture_case(fallback_panics, false);
    }
}

#[test]
fn ready_evidence_capture_drop_panic_retains_unknown_side_cover() {
    for fallback_panics in [false, true] {
        run_ready_capture_case(fallback_panics, true);
    }
}

#[test]
fn early_rejection_attempts_both_uninvoked_captures_under_original_cover() {
    for (first_panics, second_panics) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let account = account();
        let (mut ledger, guard, _) = ready_guard(&account);
        let probe = ledger.test_probe();
        let before = observe(&account, &probe);
        let first_observed = Cell::new(None);
        let second_observed = Cell::new(None);
        let first_invoked = Cell::new(false);
        let second_invoked = Cell::new(false);
        let first = EvidenceCapture {
            account: &account,
            ledger: &probe,
            observed: &first_observed,
            invoked: &first_invoked,
            panics: first_panics,
        };
        let second = EvidenceCapture {
            account: &account,
            ledger: &probe,
            observed: &second_observed,
            invoked: &second_invoked,
            panics: second_panics,
        };
        crate::retained::fail_next_final_owner_allocation();
        let result = guard.try_build(
            move || {
                first.invoked.set(true);
                drop(first);
                panic!("first builder must remain uninvoked")
            },
            move || {
                second.invoked.set(true);
                drop(second);
                panic!("second builder must remain uninvoked")
            },
        );
        assert!(result.is_err());
        assert!(!first_invoked.get() && !second_invoked.get());
        assert_eq!(first_observed.get(), Some(before));
        assert_eq!(second_observed.get(), Some(before));
        let (physical, live_bytes, reservations) = observe(&account, &probe);
        if first_panics || second_panics {
            assert_eq!(physical, before.0);
            assert_eq!(reservations, 4);
            assert!(live_bytes > 0);
            assert_eq!(
                account.snapshot().first_poison_evidence().unwrap().reason(),
                crate::retained::AccountPoisonReason::RetainedBuilderPanicked
            );
        } else {
            assert_eq!((physical, live_bytes, reservations), ((0, 0), 0, 0));
        }
        assert_eq!(account.share_call_count(), 4);
    }
}
