use super::*;
use crate::retained::ownership::{RetainedPart, StartupFallbackOwner};

type ObservedRetainedPart = RetainedPart<
    LiveDropObservation,
    OutcomeEvidenceDestination,
    TestOwner,
    TestPhase,
    TestDimension,
>;
type ObservedStartupOwner =
    StartupFallbackOwner<TestOwner, TestPhase, TestDimension, LiveDropObservation>;

fn attribute_observed_allocation(
    ledger: &mut Ledger<TestOwner, TestPhase, TestDimension>,
    account: &PhysicalStorageAccount,
    allocation: RetainedAllocation<LiveDropObservation>,
) -> ObservedRetainedPart {
    let mut batch = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(
            TestPhase::Build,
            TestDimension::Bytes,
            1,
            0,
        )
        .unwrap();
    let attribution = ledger
        .reserve_operation_attribution::<OutcomeEvidenceDestination>(
            TestPhase::Build,
            TestDimension::Bytes,
            allocation.charged_bytes(),
        )
        .unwrap();
    assert!(batch.try_push_attribution(attribution).is_ok());
    assert_eq!(batch.attribution_len(), 1);
    assert_eq!(batch.attributed_bytes(), Some(allocation.charged_bytes()));
    RetainedPart::from_complete(
        allocation,
        batch,
        account.share_after_test_admission().unwrap(),
    )
}

fn startup_owner_with_observed_fallback(
    ledger: &mut Ledger<TestOwner, TestPhase, TestDimension>,
    account: &PhysicalStorageAccount,
    fallback: ObservedRetainedPart,
) -> ObservedStartupOwner {
    let backing = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(1, 1).unwrap();
    let footprint = backing.footprint();
    let evidence_plan =
        RetainedAllocationPlan::new(footprint.bytes(), footprint.allocations()).unwrap();
    let evidence = account
        .try_retain_with(evidence_plan, || {
            Ok(backing
                .try_build(&[TestDimension::Bytes], &[TestPhase::Build])
                .unwrap())
        })
        .unwrap();
    let mut evidence_batch = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(
            TestPhase::Build,
            TestDimension::Bytes,
            1,
            0,
        )
        .unwrap();
    let attribution = ledger
        .reserve_operation_attribution::<OutcomeEvidenceDestination>(
            TestPhase::Build,
            TestDimension::Bytes,
            evidence.charged_bytes(),
        )
        .unwrap();
    assert!(evidence_batch.try_push_attribution(attribution).is_ok());
    let evidence = RetainedPart::from_complete(
        evidence,
        evidence_batch,
        account.share_after_test_admission().unwrap(),
    );
    let canonical_assembly = ledger
        .try_attribution_batch::<CanonicalOutput>(TestPhase::Build, TestDimension::Bytes, 1, 1)
        .unwrap();
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(
            TestPhase::Build,
            TestDimension::Bytes,
            1,
            1,
        )
        .unwrap();
    StartupFallbackOwner::from_complete(fallback, evidence, canonical_assembly, evidence_assembly)
}

#[test]
fn healthy_detached_part_releases_during_unrelated_host_unwind() {
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_LIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let (limits, _) = startup_limits_and_plan();
    let mut ledger =
        Ledger::<TestOwner, TestPhase, TestDimension>::try_new(limits, &[TestPhase::Build])
            .unwrap();
    let probe = ledger.test_probe();
    let allocation = account
        .try_retain_with(empty_plan(), || {
            Ok(LiveDropObservation {
                ledger: ledger.test_probe(),
                account: account.share_after_test_admission().unwrap(),
                observed_active: &OBSERVED_ACTIVE,
                observed_operation_live: &OBSERVED_LIVE,
                observed_physical_bytes: &OBSERVED_BYTES,
                observed_physical_allocations: &OBSERVED_ALLOCATIONS,
            })
        })
        .unwrap();
    let part = attribute_observed_allocation(&mut ledger, &account, allocation);
    let owner = startup_owner_with_observed_fallback(&mut ledger, &account, part);
    let mut baseline = owner
        .try_into_baseline()
        .unwrap_or_else(|_| panic!("complete startup owner must detach"));
    let (detached, evidence, batches) = baseline.take_parts();
    let (canonical_assembly, evidence_assembly, fallback_batch, evidence_batch) =
        batches.into_parts();
    // Keep the detached payload's real attribution alive through its destructor.
    drop(evidence);
    drop(evidence_batch);
    drop(canonical_assembly);
    drop(evidence_assembly);
    assert!(detached.has_exact_owner());
    let charged = account.snapshot();
    let active = probe.active_reservations();
    let live = probe.current_live_bytes();
    assert_eq!(
        charged.live_retained_bytes(),
        detached.allocation().charged_bytes()
    );
    assert_eq!(charged.live_retained_allocations(), 1);
    assert!(active > 0);
    assert!(live >= charged.live_retained_bytes());

    let payload = catch_unwind(AssertUnwindSafe(move || {
        let _detached = detached;
        panic!("unrelated host panic");
    }))
    .unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"unrelated host panic")
    );
    drop(payload);

    assert_eq!(OBSERVED_ACTIVE.load(Ordering::SeqCst), active);
    assert_eq!(OBSERVED_LIVE.load(Ordering::SeqCst), live);
    assert_eq!(
        OBSERVED_BYTES.load(Ordering::SeqCst),
        charged.live_retained_bytes()
    );
    assert_eq!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst), 1);
    assert_eq!(probe.active_reservations(), active);
    assert_eq!(probe.current_live_bytes(), live);
    drop(fallback_batch);
    assert!(probe.is_available());
    assert_eq!(probe.active_reservations(), 0);
    assert_eq!(probe.current_live_bytes(), 0);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
    assert_eq!(released.poison_event_count(), 0);
    assert_eq!(released.first_poison_evidence(), None);
    drop(ledger);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn healthy_attributed_retained_part_releases_during_unrelated_host_unwind() {
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_LIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let (limits, _) = startup_limits_and_plan();
    let mut ledger =
        Ledger::<TestOwner, TestPhase, TestDimension>::try_new(limits, &[TestPhase::Build])
            .unwrap();
    let probe = ledger.test_probe();
    let allocation = account
        .try_retain_with(empty_plan(), || {
            Ok(LiveDropObservation {
                ledger: ledger.test_probe(),
                account: account.share_after_test_admission().unwrap(),
                observed_active: &OBSERVED_ACTIVE,
                observed_operation_live: &OBSERVED_LIVE,
                observed_physical_bytes: &OBSERVED_BYTES,
                observed_physical_allocations: &OBSERVED_ALLOCATIONS,
            })
        })
        .unwrap();
    let part = attribute_observed_allocation(&mut ledger, &account, allocation);
    let charged = account.snapshot();
    let active = probe.active_reservations();
    let live = probe.current_live_bytes();
    assert_eq!(charged.live_retained_allocations(), 1);
    assert!(active > 0);
    assert!(live >= charged.live_retained_bytes());

    let payload = catch_unwind(AssertUnwindSafe(move || {
        let _part = part;
        panic!("unrelated host panic");
    }))
    .unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"unrelated host panic")
    );
    drop(payload);

    assert_eq!(OBSERVED_ACTIVE.load(Ordering::SeqCst), active);
    assert_eq!(OBSERVED_LIVE.load(Ordering::SeqCst), live);
    assert_eq!(
        OBSERVED_BYTES.load(Ordering::SeqCst),
        charged.live_retained_bytes()
    );
    assert_eq!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst), 1);
    assert!(probe.is_available());
    assert_eq!(probe.active_reservations(), 0);
    assert_eq!(probe.current_live_bytes(), 0);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
    assert_eq!(released.poison_event_count(), 0);
    assert_eq!(released.first_poison_evidence(), None);
    drop(ledger);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn healthy_startup_fallback_owner_releases_during_unrelated_host_unwind() {
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_LIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let (limits, _) = startup_limits_and_plan();
    let mut ledger =
        Ledger::<TestOwner, TestPhase, TestDimension>::try_new(limits, &[TestPhase::Build])
            .unwrap();
    let probe = ledger.test_probe();
    let allocation = account
        .try_retain_with(empty_plan(), || {
            Ok(LiveDropObservation {
                ledger: ledger.test_probe(),
                account: account.share_after_test_admission().unwrap(),
                observed_active: &OBSERVED_ACTIVE,
                observed_operation_live: &OBSERVED_LIVE,
                observed_physical_bytes: &OBSERVED_BYTES,
                observed_physical_allocations: &OBSERVED_ALLOCATIONS,
            })
        })
        .unwrap();
    let fallback = attribute_observed_allocation(&mut ledger, &account, allocation);
    let owner = startup_owner_with_observed_fallback(&mut ledger, &account, fallback);
    let charged = account.snapshot();
    let active = probe.active_reservations();
    let live = probe.current_live_bytes();
    assert!(charged.live_retained_allocations() > 1);
    assert!(active > 0);
    assert!(live >= charged.live_retained_bytes());

    let payload = catch_unwind(AssertUnwindSafe(move || {
        let _owner = owner;
        panic!("unrelated host panic");
    }))
    .unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"unrelated host panic")
    );
    drop(payload);

    assert_eq!(OBSERVED_ACTIVE.load(Ordering::SeqCst), active);
    assert_eq!(OBSERVED_LIVE.load(Ordering::SeqCst), live);
    assert_eq!(
        OBSERVED_BYTES.load(Ordering::SeqCst),
        charged.live_retained_bytes()
    );
    assert_eq!(
        OBSERVED_ALLOCATIONS.load(Ordering::SeqCst),
        charged.live_retained_allocations()
    );
    assert!(probe.is_available());
    assert_eq!(probe.active_reservations(), 0);
    assert_eq!(probe.current_live_bytes(), 0);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
    assert_eq!(released.poison_event_count(), 0);
    assert_eq!(released.first_poison_evidence(), None);
    drop(ledger);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn healthy_unprepared_publication_assembly_releases_during_unrelated_host_unwind() {
    static CANONICAL_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static CANONICAL_LIVE: AtomicU64 = AtomicU64::new(0);
    static CANONICAL_BYTES: AtomicU64 = AtomicU64::new(0);
    static CANONICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_LIVE: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_BYTES: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let mut invocation_observation = None;
    let payload = catch_unwind(AssertUnwindSafe(|| {
        let mut invocation_root: TestInvocationRoot<LiveDropObservation, LiveDropObservation> =
            InvocationRoot::new();
        let operation = test_operation(&mut invocation_root, &factory, 64);
        let account = operation
            .loan()
            .account()
            .share_after_test_admission()
            .unwrap();
        let canonical_charge = account
            .checked_retained_charge::<LiveDropObservation>(empty_plan())
            .unwrap();
        let canonical_table_charge = account
            .test_checked_retained_table_charge::<LiveDropObservation>(1)
            .unwrap();
        let mut assembly: TestPublication<'_, LiveDropObservation, LiveDropObservation> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("host-unwind publication must open"));
        let probe = assembly.loan().test_ledger_probe();
        let canonical = LiveDropObservation {
            ledger: assembly.loan().test_ledger_probe(),
            account: account.share_after_test_admission().unwrap(),
            observed_active: &CANONICAL_ACTIVE,
            observed_operation_live: &CANONICAL_LIVE,
            observed_physical_bytes: &CANONICAL_BYTES,
            observed_physical_allocations: &CANONICAL_ALLOCATIONS,
        };
        let _canonical = assembly
            .loan_mut()
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(canonical, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("unprepared canonical root must retain"));
        let evidence = LiveDropObservation {
            ledger: assembly.loan().test_ledger_probe(),
            account: account.share_after_test_admission().unwrap(),
            observed_active: &EVIDENCE_ACTIVE,
            observed_operation_live: &EVIDENCE_LIVE,
            observed_physical_bytes: &EVIDENCE_BYTES,
            observed_physical_allocations: &EVIDENCE_ALLOCATIONS,
        };
        let _evidence = assembly
            .loan_mut()
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(evidence, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("unprepared evidence root must retain"));
        assert_eq!(assembly.loan().canonical_len(), 1);
        assert_eq!(assembly.loan().outcome_evidence_len(), 1);
        let charged = account.snapshot();
        let active = probe.active_reservations();
        let live = probe.current_live_bytes();
        assert!(active > 0);
        assert!(live >= charged.live_retained_bytes());

        invocation_observation = Some((
            account,
            canonical_charge,
            canonical_table_charge,
            probe,
            charged,
            active,
            live,
        ));
        // The caller owns the actual root while its unprepared permission unwinds.
        let _assembly = assembly;
        panic!("unrelated host panic");
    }))
    .unwrap_err();
    let (account, canonical_charge, canonical_table_charge, probe, charged, active, live) =
        invocation_observation.expect("unprepared fixture must reach the host unwind");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"unrelated host panic")
    );
    drop(payload);

    assert_eq!(CANONICAL_ACTIVE.load(Ordering::SeqCst), active);
    assert_eq!(CANONICAL_LIVE.load(Ordering::SeqCst), live);
    assert_eq!(
        CANONICAL_BYTES.load(Ordering::SeqCst),
        charged.live_retained_bytes()
    );
    assert_eq!(
        CANONICAL_ALLOCATIONS.load(Ordering::SeqCst),
        charged.live_retained_allocations()
    );
    assert_eq!(EVIDENCE_ACTIVE.load(Ordering::SeqCst), active);
    assert_eq!(EVIDENCE_LIVE.load(Ordering::SeqCst), live);
    assert_eq!(
        EVIDENCE_BYTES.load(Ordering::SeqCst),
        charged.live_retained_bytes() - canonical_charge.bytes - canonical_table_charge.bytes
    );
    assert_eq!(
        EVIDENCE_ALLOCATIONS.load(Ordering::SeqCst),
        charged.live_retained_allocations()
            - canonical_charge.allocations
            - canonical_table_charge.allocations
    );
    // With no external ledger owner, availability would expose a leaked reservation.
    assert!(!probe.is_available());
    assert_eq!(probe.active_reservations(), 0);
    assert_eq!(probe.current_live_bytes(), 0);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
    assert_eq!(released.poison_event_count(), 0);
    assert_eq!(released.first_poison_evidence(), None);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}
