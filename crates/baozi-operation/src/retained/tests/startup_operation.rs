use super::*;

#[test]
fn publication_evidence_precharges_the_capacity_envelope() {
    type EvidenceOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, u8, u32>;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let dimensions = (0_u8..17).map(|dimension| {
        (
            dimension,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
    });
    let limits = LedgerLimits::new(u64::MAX, dimensions).unwrap();
    let backing = TerminalEvidenceBackingPlan::<TestPhase, u8>::try_new(17, 1).unwrap();
    let footprint = backing.footprint();
    let plan = RetainedOperationStartPlan::new(
        RetainedWriterPlan::new(TestPhase::Build, 0, 0, RetainedBatchLimits::new(2, 2)),
        RetainedHandleSharePlan::new(0, 0),
        empty_plan(),
        OutputCommitment::new(0, 1),
        OutputCommitment::new(0, 1),
    );
    let operation =
        EvidenceOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("evidence operation must start"));
    assert_eq!(account.share_call_count(), 4);
    let allocation = operation.baseline.test_evidence_allocation();
    let outer_bytes = FinalOwnerAllocation::<
        RetainedFinalOwnerInner<
            RetainedPayload<TerminalLedgerEvidence<TestPhase, u8>>,
            LeafRetainedPolicy,
        >,
    >::checked_layout_bytes()
    .unwrap();

    assert_eq!(backing.dimension_capacity().envelope_capacity(), 32);
    assert_eq!(allocation.charged_bytes(), outer_bytes + footprint.bytes());
    assert_eq!(
        allocation.charged_allocations(),
        1 + footprint.allocations()
    );
    drop(operation);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn startup_share_qualification_rejection_preserves_the_borrowed_host_account() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let policy = DimensionPolicy::reservable_bytes(7, 7, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let builder_called = AtomicBool::new(false);
    let result =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            builder_called.store(true, Ordering::SeqCst);
            Ok(7_u32)
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
        ))
    ));
    assert!(!builder_called.load(Ordering::SeqCst));
    assert_eq!(account.share_call_count(), 0);
    let retained = account
        .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(9_u32))
        .expect("borrowed host account remains usable after start rejection");
    drop(retained);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn block_handle_clone_is_charged_before_builder_or_account_share() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let policy = DimensionPolicy::reservable_bytes(15, 15, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("exact pre-block handle budget must start"));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("exact pre-block handle budget must open tables"));
    let shares_before = account.share_call_count();
    let builder_called = AtomicBool::new(false);

    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            builder_called.store(true, Ordering::SeqCst);
            Ok(emplacer.place(11, CanonicalRootTag))
        }),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
        ))
    ));
    assert!(!builder_called.load(Ordering::SeqCst));
    assert_eq!(account.share_call_count(), shares_before);
    drop(handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("handle rejection must preserve minimal fallback")),
    ));
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn startup_fallback_builder_error_returns_original_cause_and_restores_baseline() {
    reset_startup_test_observations();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let before = account.snapshot();
    let builder_called = AtomicBool::new(false);
    let (limits, plan) = startup_limits_and_plan();

    let result = TestOperation::try_start(
        OperationControl::default(),
        limits,
        &account,
        plan,
        || -> Result<u32, RetainedBuildFailureCode> {
            builder_called.store(true, Ordering::SeqCst);
            Err(BUILD_FAILED)
        },
    );

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    assert!(builder_called.load(Ordering::SeqCst));
    assert_eq!(take_start_evidence_build_calls(), 0);
    assert_eq!(take_startup_failure_ledger_snapshot(), Some((0, 0)));
    let after = account.snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert_eq!(after.poison_event_count(), before.poison_event_count());
    assert_eq!(account.share_call_count(), 4);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn startup_evidence_builder_error_drops_fallback_once_and_restores_baseline() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    fail_next_start_evidence_build(BUILD_FAILED);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let before = account.snapshot();
    let (limits, plan) = startup_limits_and_plan();
    type DropOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, DropCounter>;

    let result =
        DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(take_startup_failure_ledger_snapshot(), Some((0, 0)));
    let after = account.snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert_eq!(after.poison_event_count(), before.poison_event_count());
    assert_eq!(account.share_call_count(), 4);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn existing_poison_does_not_suppress_a_later_startup_ownership_fault() {
    reset_startup_test_observations();
    fail_next_start_evidence_build(BUILD_FAILED);
    lose_next_start_fallback_account_anchor();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (limits, plan) = startup_limits_and_plan();

    let result =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            let _ = account.record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
            Ok::<u32, RetainedBuildFailureCode>(7)
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::OwnershipUnverified
        ))
    ));
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 2);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_evidence_structural_failure_is_contained_as_an_internal_invariant() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    fail_next_start_evidence_structural();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let before = account.snapshot();
    let evidence_backing = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(1, 1)
        .unwrap_or_else(|_| panic!("evidence structural backing must qualify"));
    let footprint = evidence_backing.footprint();
    let evidence_plan = RetainedAllocationPlan::new(footprint.bytes(), footprint.allocations())
        .unwrap_or_else(|_| panic!("evidence structural plan must qualify"));
    let evidence_charge = account
        .checked_retained_charge::<TerminalLedgerEvidence<TestPhase, TestDimension>>(evidence_plan)
        .unwrap_or_else(|_| panic!("evidence structural charge must qualify"));
    let (limits, plan) = startup_limits_and_plan();
    type DropOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, DropCounter>;

    let result =
        DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::ConstructionInvariantUnverified)
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    let snapshot = account.snapshot();
    assert_eq!(
        snapshot.live_retained_bytes() - before.live_retained_bytes(),
        evidence_charge.bytes
    );
    assert_eq!(
        snapshot.live_retained_allocations() - before.live_retained_allocations(),
        evidence_charge.allocations
    );
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::TerminalEvidenceConstructionUnverified)
    );
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("structural evidence failure must capture ledger state"));
    assert_eq!(active, 2);
    assert!(live > evidence_charge.bytes);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_evidence_builder_panic_releases_the_clean_fallback_sibling() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    panic_next_start_evidence_build();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let before = account.snapshot();
    let evidence_backing = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(1, 1)
        .unwrap_or_else(|_| panic!("evidence panic backing must qualify"));
    let footprint = evidence_backing.footprint();
    let evidence_plan = RetainedAllocationPlan::new(footprint.bytes(), footprint.allocations())
        .unwrap_or_else(|_| panic!("evidence panic plan must qualify"));
    let evidence_charge = account
        .checked_retained_charge::<TerminalLedgerEvidence<TestPhase, TestDimension>>(evidence_plan)
        .unwrap_or_else(|_| panic!("evidence panic charge must qualify"));
    let (limits, plan) = startup_limits_and_plan();
    type DropOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, DropCounter>;

    let result =
        DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    let snapshot = account.snapshot();
    assert_eq!(
        snapshot.live_retained_bytes() - before.live_retained_bytes(),
        evidence_charge.bytes
    );
    assert_eq!(
        snapshot.live_retained_allocations() - before.live_retained_allocations(),
        evidence_charge.allocations
    );
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("evidence panic must capture startup ledger state"));
    assert_eq!(active, 2);
    assert!(live > evidence_charge.bytes);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_evidence_builder_panic_keeps_storage_release_as_r_and_records_builder_f() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    panic_next_start_evidence_build();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (limits, plan) = startup_limits_and_plan();
    type PanicDropOperation = RetainedOperation<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        PanickingDropProbe,
    >;

    let result =
        PanicDropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok::<PanickingDropProbe, RetainedBuildFailureCode>(PanickingDropProbe(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::StorageDropUnverified
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 2);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("combined evidence panic must capture startup ledger state"));
    assert_eq!(active, 4);
    assert!(live > 0);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_evidence_builder_panic_prefers_sibling_counter_f_without_builder_f() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    panic_next_start_evidence_build();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (limits, plan) = startup_limits_and_plan();
    type DropOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, DropCounter>;

    let result =
        DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            let mut state = lock_recovering_poison(&account.inner.value().linear);
            state.counters.live_bytes = 0;
            state.counters.live_allocations = 0;
            drop(state);
            Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::ChargeReleaseUnverified
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_fallback_panic_contains_fallback_and_cleans_evidence() {
    reset_startup_test_observations();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let fallback_charge = account
        .checked_retained_charge::<u32>(empty_plan())
        .unwrap_or_else(|_| panic!("fallback charge must qualify"));
    let (limits, plan) = startup_limits_and_plan();

    let result = TestOperation::try_start(
        OperationControl::default(),
        limits,
        &account,
        plan,
        || -> Result<u32, RetainedBuildFailureCode> { panic!("inject fallback startup panic") },
    );

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 0);
    let snapshot = account.snapshot();
    assert!(snapshot.poisoned());
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );
    assert_eq!(snapshot.live_retained_bytes(), fallback_charge.bytes);
    assert_eq!(
        snapshot.live_retained_allocations(),
        fallback_charge.allocations
    );
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("fallback panic must capture startup ledger state"));
    assert_eq!(active, 2);
    assert!(live > fallback_charge.bytes);
    assert_eq!(account.share_call_count(), 4);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_fallback_panic_with_sibling_underflow_records_only_counter_fault() {
    reset_startup_test_observations();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (limits, plan) = startup_limits_and_plan();

    let result = TestOperation::try_start(
        OperationControl::default(),
        limits,
        &account,
        plan,
        || -> Result<u32, RetainedBuildFailureCode> {
            let mut state = lock_recovering_poison(&account.inner.value().linear);
            state.counters.live_bytes = 0;
            state.counters.live_allocations = 0;
            drop(state);
            panic!("inject fallback panic after counter corruption")
        },
    );

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::ChargeReleaseUnverified
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 0);
    let snapshot = account.snapshot();
    assert!(snapshot.poisoned());
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("underflow panic must capture startup ledger state"));
    assert_eq!(active, 4);
    assert!(live > 0);
    assert_eq!(account.share_call_count(), 4);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn single_value_bootstrap_and_primary_prepare_exclude_duplicate_tail_work() {
    let production = retained_production_source();
    assert!(!production.contains("set: fn("));
    assert!(!production.contains("AggregateSetterPanicked"));
    assert!(!production.contains("fn try_create_single_retained_part"));
    assert_eq!(production.matches(".try_prepare_typed_control(").count(), 1);
    assert_eq!(production.matches("ledger.try_bind_primary(").count(), 1);
    assert_eq!(
        production.matches("ledger.finish_bound_primary(").count(),
        1
    );
}
