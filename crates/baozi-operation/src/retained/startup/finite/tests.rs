use super::super::{StartupTransitionPanicPoint, set_startup_transition_panic};
use super::*;
use crate::DimensionPolicy;
use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};
use crate::ledger::{
    AttributionBatchPlan, DimensionSnapshot, OutputCommitment, StartupAccountShareEnvelope,
    StartupRetainedAttribution, test_startup_graph_allocation_entries,
    test_startup_graph_ledger_release_locks, test_startup_graph_storage_releases,
};
use crate::retained::allocation::{
    RetainedAllocationError, RetainedBuildFailureCode, test_retained_final_owner_allocator_entries,
};
use crate::retained::operation_start::test_startup_payload_allocation_attempts;
use crate::retained::{
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Owner;
impl OperationOwner for Owner {}
type TestLedger = FiniteLedger<Owner, u8, u8>;

fn account() -> Arc<PhysicalStorageAccount> {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let limits = PhysicalStorageAccountLimits::new(u64::MAX, 1).unwrap();
    Arc::new(factory.try_create_account(limits).unwrap())
}

fn ledger() -> TestLedger {
    let policies = [1_u8, 2, 3, 4, 5, 6].map(|dimension| {
        let policy = if dimension == 6 {
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX)
        } else {
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        };
        (dimension, policy)
    });
    let limits = FiniteLedgerLimits::try_new(u64::MAX, u64::MAX, policies).unwrap();
    FiniteLedger::try_new(limits, [3, 1, 2]).unwrap()
}

fn allocation_plan() -> RetainedAllocationPlan {
    RetainedAllocationPlan::new(0, 0).unwrap()
}

fn charge(account: &PhysicalStorageAccount) -> AdmissionCharge {
    account
        .checked_retained_charge::<SendSyncDropProbe>(allocation_plan())
        .unwrap()
}

fn parts(bytes: u64) -> FiniteStartupLedgerPlanParts<Owner, u8, u8> {
    FiniteStartupLedgerPlanParts {
        canonical_assembly: AttributionBatchPlan::try_new(1, 1, 2, 1).unwrap(),
        evidence_assembly: AttributionBatchPlan::try_new(2, 2, 2, 2).unwrap(),
        fallback: AttributionBatchPlan::try_new(3, 3, 1, 1).unwrap(),
        fallback_attribution: StartupRetainedAttribution::new(3, 4, bytes),
        account_shares: StartupAccountShareEnvelope::three_identical([(6, 1), (6, 2)]),
        fallback_commitment: OutputCommitment::new(6, 4),
    }
}

fn prepare<'account>(
    account: &'account PhysicalStorageAccount,
    ledger: &mut TestLedger,
) -> ReadyFiniteStartupCrossPlane<'account, Owner, u8, u8> {
    let charge = charge(account);
    let plan = FiniteStartupCrossPlanePlan::try_new(parts(charge.bytes), charge).unwrap();
    account
        .try_admit_operation_start()
        .unwrap()
        .try_prepare_finite_startup_cross_plane(&mut ledger.view(), plan)
        .unwrap_or_else(|_| panic!("actual finite startup must be ready"))
}

fn live(ledger: &TestLedger) -> [u64; 6] {
    [1_u8, 2, 3, 4, 5, 6].map(|dimension| {
        ledger
            .as_read()
            .dimension_evidence(dimension)
            .unwrap()
            .current_live_bytes()
    })
}

fn dimensions(ledger: &TestLedger) -> [DimensionSnapshot; 6] {
    [1_u8, 2, 3, 4, 5, 6].map(|dimension| ledger.as_read().dimension_evidence(dimension).unwrap())
}

struct SendSyncDropProbe {
    drops: Arc<AtomicUsize>,
    observing: Option<(Arc<PhysicalStorageAccount>, Arc<Mutex<TestLedger>>)>,
    panic_on_drop: bool,
}

fn probe(drops: &Arc<AtomicUsize>) -> SendSyncDropProbe {
    SendSyncDropProbe {
        drops: Arc::clone(drops),
        observing: None,
        panic_on_drop: false,
    }
}

impl Drop for SendSyncDropProbe {
    fn drop(&mut self) {
        if let Some((account, ledger)) = &self.observing {
            assert_eq!(
                account.snapshot().live_retained_bytes(),
                charge(account).bytes
            );
            assert_eq!(account.snapshot().live_retained_allocations(), 1);
            let ledger = ledger.lock().unwrap();
            let covered = live(&ledger);
            assert!(covered[..4].iter().all(|bytes| *bytes > 0));
            assert_eq!(covered[4..], [0, 0]);
            for dimension in [1_u8, 2, 3, 4] {
                let evidence = ledger.as_read().dimension_evidence(dimension).unwrap();
                assert_eq!(evidence.current_live_bytes(), evidence.peak_live_bytes());
            }
        }
        self.drops.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.panic_on_drop,
            "injected finite fallback destructor panic"
        );
    }
}

#[test]
fn baseline_drop_destroys_one_real_payload_before_refunding_its_batch_covers() {
    let account = account();
    let ledger = Arc::new(Mutex::new(ledger()));
    let drops = Arc::new(AtomicUsize::new(0));
    let allocations = test_retained_final_owner_allocator_entries();
    let attempts = test_startup_payload_allocation_attempts();
    let ready = prepare(&account, &mut ledger.lock().unwrap());
    let mut calls = 0;
    let baseline = ready
        .into_retained_guard::<SendSyncDropProbe>(allocation_plan())
        .try_build(|| {
            calls += 1;
            let mut payload = probe(&drops);
            payload.observing = Some((Arc::clone(&account), Arc::clone(&ledger)));
            Ok(payload)
        })
        .unwrap();
    assert_eq!(calls, 1);
    assert!(baseline.account().is_same_account(&account));
    assert_eq!(account.share_call_count(), 3);
    assert_eq!(
        test_retained_final_owner_allocator_entries(),
        allocations + 1
    );
    assert_eq!(
        test_startup_payload_allocation_attempts(),
        (attempts.0 + 1, attempts.1)
    );
    assert_eq!(account.snapshot().total_admitted_retained_allocations(), 1);
    let work = ledger
        .lock()
        .unwrap()
        .as_read()
        .dimension_evidence(6)
        .unwrap();
    assert_eq!((work.attempted(), work.consumed()), (9, 9));
    assert_eq!(work.published_outcome_evidence(), 0);
    drop(baseline);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(live(&ledger.lock().unwrap()), [0; 6]);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.share_call_count(), 3);
}

#[test]
fn taken_terminal_account_survives_fallback_discard_while_real_batches_stay_covered() {
    let account = account();
    let mut ledger = ledger();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut baseline = prepare(&account, &mut ledger)
        .into_retained_guard::<SendSyncDropProbe>(allocation_plan())
        .try_build(|| Ok(probe(&drops)))
        .unwrap();
    let covered = live(&ledger);
    let (mut fallback, terminal_account, batches) = baseline.take_parts();
    assert!(terminal_account.is_same_account(fallback.account()));
    drop(baseline);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(live(&ledger), covered);
    assert_eq!(terminal_account.snapshot().live_retained_allocations(), 1);
    drop(account);
    drop(fallback.try_discard_allocation().unwrap());
    drop(fallback);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(live(&ledger), covered);
    assert_eq!(terminal_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(terminal_account.snapshot().live_retained_allocations(), 0);
    let (canonical, evidence, fallback_batch) = batches.into_parts();
    assert_eq!(canonical.attribution_len(), 0);
    assert_eq!(evidence.attribution_len(), 0);
    assert_eq!(fallback_batch.attribution_len(), 1);
    assert_eq!(
        fallback_batch.attributed_bytes(),
        Some(charge(&terminal_account).bytes)
    );
    assert_eq!(fallback_batch.commitment_len(), 1);
    drop((canonical, evidence, fallback_batch));
    assert_eq!(live(&ledger), [0; 6]);
    assert_eq!(terminal_account.share_call_count(), 3);
    assert!(!terminal_account.snapshot().poisoned());
}

#[test]
fn physical_transfer_lends_original_batches_until_storage_cleanup_finishes() {
    let account = account();
    let ledger = Arc::new(Mutex::new(ledger()));
    let drops = Arc::new(AtomicUsize::new(0));
    let ready = prepare(&account, &mut ledger.lock().unwrap());
    let mut baseline = ready
        .into_retained_guard::<SendSyncDropProbe>(allocation_plan())
        .try_build(|| {
            let mut payload = probe(&drops);
            payload.observing = Some((Arc::clone(&account), Arc::clone(&ledger)));
            Ok(payload)
        })
        .unwrap();
    let covered = live(&ledger.lock().unwrap());
    let original_batches = baseline.with_batches(std::ptr::from_ref).unwrap();
    let original_fallback_batch = {
        let view = baseline.terminal_batches().unwrap();
        std::ptr::from_ref(&*view.fallback)
    };

    let (mut fallback, terminal_account, batches) = baseline.take_physical_and_lend_batches();

    assert!(terminal_account.is_same_account(&account));
    assert!(terminal_account.is_same_account(fallback.account()));
    assert!(Arc::ptr_eq(&fallback.allocation().as_ref().drops, &drops));
    let batch_slot = std::ptr::from_ref(&*batches);
    {
        let mut view = batches.as_mut().unwrap().view();
        ledger
            .lock()
            .unwrap()
            .as_read()
            .try_attach_fallback_rows(view.reborrow())
            .unwrap();
        let shorter = view.reborrow();
        assert_eq!(
            std::ptr::from_ref(&*shorter.fallback),
            original_fallback_batch
        );
        assert!(shorter.numerical_evidence.is_none());
    }
    assert_eq!(std::ptr::from_ref(&*batches), batch_slot);
    assert_eq!(
        std::ptr::from_ref(batches.as_ref().unwrap()),
        original_batches
    );
    assert_eq!(live(&ledger.lock().unwrap()), covered);

    drop(account);
    drop(fallback.try_discard_allocation().unwrap());
    drop(fallback);

    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(terminal_account.snapshot().live_retained_allocations(), 0);
    assert_eq!(live(&ledger.lock().unwrap()), covered);
    assert_eq!(
        std::ptr::from_ref(batches.as_ref().unwrap()),
        original_batches
    );

    drop(batches.take());
    drop(baseline);

    assert_eq!(live(&ledger.lock().unwrap()), [0; 6]);
    assert_eq!(
        ledger.lock().unwrap().as_read().terminal_residue_cause(),
        None
    );
    assert!(!terminal_account.snapshot().poisoned());
}

#[test]
fn transition_panics_return_owned_containment_without_resharing_or_refunding() {
    for (point, committed) in [
        (StartupTransitionPanicPoint::BeforeCommit, false),
        (StartupTransitionPanicPoint::AfterCommit, true),
    ] {
        let account = account();
        let mut ledger = ledger();
        let charge = charge(&account);
        let releases = test_startup_graph_storage_releases();
        let release_locks = test_startup_graph_ledger_release_locks();
        let admission = account.try_admit_operation_start().unwrap();
        let plan = FiniteStartupCrossPlanePlan::try_new(parts(charge.bytes), charge).unwrap();
        set_startup_transition_panic(point);
        let result = admission.try_prepare_finite_startup_cross_plane(&mut ledger.view(), plan);
        let containment = match result {
            Err(FiniteStartupCrossPlaneAdmissionFailure {
                cause: FiniteStartupAdmissionCause::Uncertain,
                containment: Some(containment),
            }) => containment,
            _ => panic!("transition panic must return the owned finite containment"),
        };
        let ownership = containment
            .ownership
            .as_ref()
            .expect("containment owns the actual graph");
        assert!(ownership.graph.belongs_to_view(&ledger.as_read()));
        for owner in [
            &ownership.owners.fallback_account,
            &ownership.owners.fallback_lease_owner,
            &ownership.owners.terminal_account,
        ] {
            assert!(owner.is_same_account(&account));
        }
        let covered = live(&ledger);
        assert!(covered[..4].iter().all(|bytes| *bytes > 0));
        let physical = account.snapshot();
        assert_eq!(
            physical.live_retained_bytes(),
            if committed { charge.bytes } else { 0 }
        );
        assert_eq!(physical.live_retained_allocations(), u64::from(committed));
        assert_eq!(physical.poison_event_count(), 1);
        assert_eq!(containment.poisoned().unwrap().snapshot(), physical);
        drop(containment);
        assert_eq!(live(&ledger), covered);
        assert_eq!(account.snapshot(), physical);
        assert_eq!(account.share_call_count(), 3);
        assert_eq!(test_startup_graph_storage_releases(), releases);
        assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    }
}

#[test]
fn abandoned_ready_drop_retains_the_whole_actual_graph_and_physical_charge() {
    let account = account();
    let mut ledger = ledger();
    let ready = prepare(&account, &mut ledger);
    let covered = live(&ledger);
    let physical = account.snapshot();
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();
    let allocations = test_retained_final_owner_allocator_entries();
    drop(ready);
    assert_eq!(live(&ledger), covered);
    assert!(covered[..4].iter().all(|bytes| *bytes > 0));
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        physical.live_retained_bytes()
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 1);
    let first = account.snapshot().first_poison_evidence().unwrap();
    assert_eq!(
        first.reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_eq!(account.snapshot().poison_event_count(), 1);
    assert_eq!(account.share_call_count(), 3);
    assert_eq!(test_retained_final_owner_allocator_entries(), allocations);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
}

#[test]
fn builder_rejection_and_panic_dispose_captures_and_preserve_current_first_poison() {
    for (panics, prior_poison) in [(false, false), (true, false), (false, true), (true, true)] {
        let account = account();
        let mut ledger = ledger();
        let ready = prepare(&account, &mut ledger);
        let covered = live(&ledger);
        let drops = Arc::new(AtomicUsize::new(0));
        let capture = probe(&drops);
        let allocations = test_retained_final_owner_allocator_entries();
        let mut calls = 0;
        let guard = ready.into_retained_guard::<SendSyncDropProbe>(allocation_plan());
        let result = guard.try_build(|| {
            let _capture = capture;
            calls += 1;
            if prior_poison {
                let reason = AccountPoisonReason::TerminalEvidenceFreezeUnverified;
                let _ = account.test_record_poison(reason);
            }
            assert!(!panics, "injected finite fallback builder panic");
            Err(RetainedBuildFailureCode::new(17))
        });
        let error = result.err().expect("injected builder failure");
        match &error.cause {
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked) => {
                assert!(panics);
            }
            RetainedWriterError::OwnershipUnverified => assert!(!panics && prior_poison),
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause)) => {
                assert!(!panics && !prior_poison);
                assert_eq!(*cause, RetainedBuildFailureCode::new(17));
            }
            other => panic!("unexpected finite builder failure: {other:?}"),
        }
        let retained = panics || prior_poison;
        assert_eq!(error.residue.is_some(), retained);
        if let Some(residue) = error.residue.as_ref() {
            assert!(residue.account().unwrap().is_same_account(&account));
        }
        drop(error);
        let expected_live = if retained {
            [0, 0, covered[2], covered[3], 0, 0]
        } else {
            [0; 6]
        };
        assert_eq!(live(&ledger), expected_live);
        let physical = account.snapshot();
        let retained_bytes = if retained { charge(&account).bytes } else { 0 };
        assert_eq!(physical.live_retained_bytes(), retained_bytes);
        assert_eq!(physical.live_retained_allocations(), u64::from(retained));
        assert_eq!(physical.poisoned(), retained);
        assert!(physical.poison_event_domain_valid());
        let first = if prior_poison {
            Some(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
        } else if panics {
            Some(AccountPoisonReason::RetainedBuilderPanicked)
        } else {
            None
        };
        assert_eq!(
            physical
                .first_poison_evidence()
                .map(|evidence| evidence.reason()),
            first
        );
        assert_eq!(
            physical.poison_event_count(),
            u128::from(panics) + u128::from(prior_poison)
        );
        assert_eq!(calls, 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(account.share_call_count(), 3);
        assert_eq!(
            test_retained_final_owner_allocator_entries(),
            allocations + 1
        );
    }
}

#[test]
fn byte_mismatch_and_current_physical_quota_reject_without_physical_mutation() {
    let account = account();
    let mut ledger = ledger();
    let charge = charge(&account);
    let before = account.snapshot();
    let graph_allocations = test_startup_graph_allocation_entries();
    assert!(matches!(
        FiniteStartupCrossPlanePlan::try_new(parts(charge.bytes + 1), charge),
        Err(StartupCrossPlanePlanError::FallbackByteMismatch { qualified, physical })
            if qualified == charge.bytes + 1 && physical == charge.bytes
    ));
    assert_eq!(account.snapshot(), before);
    assert_eq!(account.share_call_count(), 0);
    assert_eq!(test_startup_graph_allocation_entries(), graph_allocations);
    assert_eq!(live(&ledger), [0; 6]);
    let occupied = account
        .try_retain_with(allocation_plan(), || Ok(7_u8))
        .unwrap();
    let before = account.snapshot();
    let shares = account.share_call_count();
    let allocations = test_retained_final_owner_allocator_entries();
    let releases = test_startup_graph_storage_releases();
    let admission = account.try_admit_operation_start().unwrap();
    let plan = FiniteStartupCrossPlanePlan::try_new(parts(charge.bytes), charge).unwrap();
    let result = admission.try_prepare_finite_startup_cross_plane(&mut ledger.view(), plan);
    assert!(matches!(
        result,
        Err(FiniteStartupCrossPlaneAdmissionFailure {
            cause: FiniteStartupAdmissionCause::PhysicalNoMutation(
                StartupPhysicalNoMutation::AllocationLimitExceeded {
                    requested: 2,
                    limit: 1
                }
            ),
            containment: None,
        })
    ));
    assert_eq!(account.snapshot(), before);
    assert_eq!(live(&ledger), [0; 6]);
    assert_eq!(account.share_call_count(), shares + 3);
    assert_eq!(test_retained_final_owner_allocator_entries(), allocations);
    assert_eq!(test_startup_graph_storage_releases(), releases + 6);
    drop(occupied);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn uninvoked_builder_capture_is_disposed_while_original_startup_charge_is_covered() {
    struct Capture {
        account: Arc<PhysicalStorageAccount>,
        observations: Arc<Mutex<Vec<(u64, u64)>>>,
    }
    impl Drop for Capture {
        fn drop(&mut self) {
            let physical = self.account.snapshot();
            self.observations.lock().unwrap().push((
                physical.live_retained_bytes(),
                physical.live_retained_allocations(),
            ));
        }
    }

    for mismatch in [false, true] {
        let account = account();
        let mut ledger = ledger();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let captured = Capture {
            account: Arc::clone(&account),
            observations: Arc::clone(&observations),
        };
        let admitted = charge(&account).bytes;
        let ready = prepare(&account, &mut ledger);
        let requested = if mismatch {
            RetainedAllocationPlan::new(1, 1).unwrap()
        } else {
            crate::retained::fail_next_final_owner_allocation();
            allocation_plan()
        };
        let result = ready
            .into_retained_guard::<SendSyncDropProbe>(requested)
            .try_build(move || {
                drop(captured);
                panic!("a rejected constructor must not invoke its builder")
            });
        assert!(result.is_err());
        assert_eq!(*observations.lock().unwrap(), [(admitted, 1)]);
        assert_eq!(live(&ledger), [0; 6]);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}

#[test]
fn uninvoked_builder_capture_panic_keeps_the_original_physical_charge_and_attribution() {
    struct PanickingCapture {
        account: Arc<PhysicalStorageAccount>,
        observations: Arc<Mutex<Vec<(u64, u64)>>>,
    }
    impl Drop for PanickingCapture {
        fn drop(&mut self) {
            let physical = self.account.snapshot();
            self.observations.lock().unwrap().push((
                physical.live_retained_bytes(),
                physical.live_retained_allocations(),
            ));
            panic!("injected uninvoked capture destructor panic");
        }
    }
    let account = account();
    let mut ledger = ledger();
    let observations = Arc::new(Mutex::new(Vec::new()));
    let captured = PanickingCapture {
        account: Arc::clone(&account),
        observations: Arc::clone(&observations),
    };
    let admitted = charge(&account).bytes;
    let ready = prepare(&account, &mut ledger);
    let covered = live(&ledger);
    crate::retained::fail_next_final_owner_allocation();
    let result = ready
        .into_retained_guard::<SendSyncDropProbe>(allocation_plan())
        .try_build(move || {
            drop(captured);
            panic!("the builder must remain uninvoked")
        });
    assert!(matches!(
        result.as_ref().err().map(|error| &error.cause),
        Some(RetainedWriterError::Physical(
            RetainedAllocationError::BuilderPanicked
        ))
    ));
    assert!(result.as_ref().err().unwrap().residue.is_some());
    drop(result);
    assert_eq!(*observations.lock().unwrap(), [(admitted, 1)]);
    assert_eq!(live(&ledger), [0, 0, covered[2], covered[3], 0, 0]);
    let physical = account.snapshot();
    assert_eq!(physical.live_retained_bytes(), admitted);
    assert_eq!(physical.live_retained_allocations(), 1);
    assert_eq!(
        physical.first_poison_evidence().unwrap().reason(),
        AccountPoisonReason::RetainedBuilderPanicked
    );
    assert_eq!(account.share_call_count(), 3);
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn physical_attachment_keeps_the_real_fallback_and_unrelated_live_and_zero_byte_tokens() {
    let account = account();
    let mut ledger = ledger();
    let drops = Arc::new(AtomicUsize::new(0));
    let baseline = prepare(&account, &mut ledger)
        .into_retained_guard::<SendSyncDropProbe>(allocation_plan())
        .try_build(|| Ok(probe(&drops)))
        .unwrap();
    let unrelated = ledger.view().reserve(1, 5, 7).unwrap();
    let zero = ledger.view().reserve(2, 5, 0).unwrap();
    let before = dimensions(&ledger);
    let phases = [1_u8, 2, 3].map(|phase| ledger.as_read().phase_evidence(phase).unwrap());
    let physical = account.snapshot();
    let allocations = test_retained_final_owner_allocator_entries();
    let attempts = test_startup_payload_allocation_attempts();
    let graph_allocations = test_startup_graph_allocation_entries();
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();

    let attached = baseline.try_attach(ledger.as_read()).unwrap();

    assert!(attached.account().is_same_account(&account));
    assert!(physical.live_retained_bytes() > 0);
    assert_eq!(physical.live_retained_allocations(), 1);
    assert_eq!(account.snapshot(), physical);
    assert_eq!(dimensions(&ledger), before);
    assert_eq!(
        [1_u8, 2, 3].map(|phase| ledger.as_read().phase_evidence(phase).unwrap()),
        phases
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(account.share_call_count(), 3);
    assert_eq!(test_retained_final_owner_allocator_entries(), allocations);
    assert_eq!(test_startup_payload_allocation_attempts(), attempts);
    assert_eq!(test_startup_graph_allocation_entries(), graph_allocations);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);

    drop(attached);

    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(live(&ledger), [0, 0, 0, 0, 7, 0]);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
    // This hook counts pending-startup rollback, not ordinary attached batch destruction.
    assert_eq!(test_startup_graph_storage_releases(), releases);
    drop(unrelated);
    assert_eq!(live(&ledger), [0; 6]);
    assert!(matches!(
        ledger.as_read().terminal_residue_cause(),
        Some(crate::ledger::LedgerCause::ActiveReservationsRemain { count: 1 })
    ));
    drop(zero);
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
}

#[test]
fn foreign_physical_attachment_disposes_the_payload_before_its_actual_ledger_covers() {
    let account = account();
    let owner = Arc::new(Mutex::new(ledger()));
    let foreign = ledger();
    let drops = Arc::new(AtomicUsize::new(0));
    let ready = prepare(&account, &mut owner.lock().unwrap());
    let baseline = ready
        .into_retained_guard::<SendSyncDropProbe>(allocation_plan())
        .try_build(|| {
            let mut payload = probe(&drops);
            payload.observing = Some((Arc::clone(&account), Arc::clone(&owner)));
            Ok(payload)
        })
        .unwrap();
    let owner_before = dimensions(&owner.lock().unwrap());
    let foreign_before = dimensions(&foreign);
    let releases = test_startup_graph_storage_releases();

    let result = baseline.try_attach(foreign.as_read());

    let Err(RetainedWriterError::AccountPoisoned(poisoned)) = result else {
        panic!("foreign physical attachment must reject after verified cleanup");
    };
    assert_eq!(
        poisoned
            .snapshot()
            .first_poison_evidence()
            .unwrap()
            .reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(live(&owner.lock().unwrap()), [0; 6]);
    assert_eq!(dimensions(&owner.lock().unwrap())[5], owner_before[5]);
    assert_eq!(
        owner.lock().unwrap().as_read().terminal_residue_cause(),
        None
    );
    assert_eq!(dimensions(&foreign), foreign_before);
    assert_eq!(foreign.as_read().terminal_residue_cause(), None);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    let physical = account.snapshot();
    assert_eq!(physical.live_retained_bytes(), 0);
    assert_eq!(physical.live_retained_allocations(), 0);
    assert_eq!(physical.total_admitted_retained_allocations(), 1);
    assert_eq!(physical.poison_event_count(), 1);
    assert!(physical.poison_event_count_exact());
    assert!(physical.poison_event_domain_valid());
    assert_eq!(
        physical.first_poison_evidence(),
        poisoned.snapshot().first_poison_evidence()
    );
    assert_eq!(account.share_call_count(), 3);
}

#[test]
fn tightened_evidence_policy_refuses_physical_attachment_and_releases_actual_covers() {
    let account = account();
    let mut ledger = ledger();
    let drops = Arc::new(AtomicUsize::new(0));
    let baseline = prepare(&account, &mut ledger)
        .into_retained_guard::<SendSyncDropProbe>(allocation_plan())
        .try_build(|| Ok(probe(&drops)))
        .unwrap();
    ledger
        .view()
        .tighten_dimension_policy(
            6,
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(3),
        )
        .unwrap();
    let before = dimensions(&ledger);
    let releases = test_startup_graph_storage_releases();

    let result = baseline.try_attach(ledger.as_read());

    let Err(RetainedWriterError::AccountPoisoned(poisoned)) = result else {
        panic!("the current evidence limit must reject physical attachment after verified cleanup");
    };
    assert_eq!(
        poisoned
            .snapshot()
            .first_poison_evidence()
            .unwrap()
            .reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(live(&ledger), [0; 6]);
    assert_eq!(dimensions(&ledger)[5], before[5]);
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    let physical = account.snapshot();
    assert_eq!(physical.live_retained_bytes(), 0);
    assert_eq!(physical.live_retained_allocations(), 0);
    assert_eq!(physical.total_admitted_retained_allocations(), 1);
    assert_eq!(
        physical.first_poison_evidence(),
        poisoned.snapshot().first_poison_evidence()
    );
    assert_eq!(physical.poison_event_count(), 1);
    assert!(physical.poison_event_count_exact());
    assert!(physical.poison_event_domain_valid());
    assert_eq!(account.share_call_count(), 3);
}

#[test]
fn refused_physical_attachment_contains_uncertain_cleanup_and_preserves_prior_poison() {
    for opaque in [true, false] {
        let account = account();
        let owner = Arc::new(Mutex::new(ledger()));
        let foreign = ledger();
        let drops = Arc::new(AtomicUsize::new(0));
        let ready = prepare(&account, &mut owner.lock().unwrap());
        let baseline = ready
            .into_retained_guard::<SendSyncDropProbe>(allocation_plan())
            .try_build(|| {
                let mut payload = probe(&drops);
                payload.observing = Some((Arc::clone(&account), Arc::clone(&owner)));
                payload.panic_on_drop = !opaque;
                Ok(payload)
            })
            .unwrap();
        let owner_before = dimensions(&owner.lock().unwrap());
        let foreign_before = dimensions(&foreign);
        // The independent injected fault consumes its own real account-action admission.
        account.try_admit_account_action().unwrap().run(|account| {
            let _ =
                account.test_record_poison(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
        });
        let before = account.snapshot();
        let releases = test_startup_graph_storage_releases();
        let release_locks = test_startup_graph_ledger_release_locks();
        if opaque {
            crate::retained::test_panic_final_owner_drop_after(0);
        }

        let result = baseline.try_attach(foreign.as_read());

        assert!(matches!(
            result,
            Err(RetainedWriterError::StorageDropUnverified)
        ));
        assert_eq!(drops.load(Ordering::SeqCst), usize::from(!opaque));
        assert_eq!(dimensions(&owner.lock().unwrap()), owner_before);
        assert_eq!(dimensions(&foreign), foreign_before);
        assert_eq!(foreign.as_read().terminal_residue_cause(), None);
        assert_eq!(test_startup_graph_storage_releases(), releases);
        assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
        let physical = account.snapshot();
        assert_eq!(physical.live_retained_bytes(), before.live_retained_bytes());
        assert_eq!(physical.live_retained_allocations(), 1);
        assert_eq!(physical.total_admitted_retained_allocations(), 1);
        assert_eq!(
            physical.poison_event_count(),
            before.poison_event_count() + 2
        );
        assert!(physical.poison_event_count_exact());
        assert!(physical.poison_event_domain_valid());
        assert_eq!(
            physical.first_poison_evidence(),
            before.first_poison_evidence()
        );
        assert_eq!(account.share_call_count(), 3);
    }
}
