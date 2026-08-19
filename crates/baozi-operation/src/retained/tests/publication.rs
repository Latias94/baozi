use super::*;
use std::num::NonZeroU64;

type ValidatedScalarSuccess = ValidatedRetainedSuccess<
    TestOwner,
    TestPhase,
    TestProgress,
    TestUnit,
    TestDimension,
    u32,
    u64,
    u64,
>;

fn validated_scalar_success_on_account(account: &PhysicalStorageAccount) -> ValidatedScalarSuccess {
    let operation = test_operation_on_account_with_control(
        account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    operation
        .try_complete_phase(TestPhase::Build)
        .unwrap_or_else(|_| panic!("joint-commit build phase must complete"));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("joint-commit publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);
    assembly
        .try_commit_canonical(TestDimension::Bytes, 5)
        .unwrap_or_else(|_| panic!("canonical commitment must fit"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 6)
        .unwrap_or_else(|_| panic!("outcome-evidence commitment must fit"));
    match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
        session.canonical(canonical) == Some(&11) && session.outcome_evidence(evidence) == Some(&7)
    }) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("joint-commit publication must validate"),
    }
}

fn validated_freeze_success_with_limits(
    account: &PhysicalStorageAccount,
    committed_limit: u64,
    evidence_limit: u64,
    evidence_members: usize,
) -> ValidatedScalarSuccess {
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, committed_limit, u64::MAX)
        .with_published_outcome_evidence_limit(evidence_limit);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)])
        .unwrap_or_else(|_| panic!("freeze qualification limits must qualify"));
    let plan = RetainedOperationStartPlan::new(
        writer_plan(evidence_members + 1, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation =
        TestOperation::try_start(OperationControl::default(), limits, account, plan, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("freeze qualification operation must start"));
    operation
        .try_complete_phase(TestPhase::Build)
        .unwrap_or_else(|_| panic!("freeze qualification build phase must complete"));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, evidence_members))
        .unwrap_or_else(|_| panic!("freeze qualification publication must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("freeze canonical root must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("freeze evidence root must retain"));
    let detail = (evidence_members == 2).then(|| {
        assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(9, EvidenceDetailTag))
            })
            .unwrap_or_else(|_| panic!("freeze evidence detail must retain"))
    });
    match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
        session.canonical(canonical) == Some(&11)
            && session.outcome_evidence(evidence) == Some(&7)
            && detail.is_none_or(|detail| session.outcome_evidence(detail) == Some(&9))
    }) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("freeze qualification publication must validate"),
    }
}

fn scalar_success_signature(
    prepared: PreparedRetainedPublication<u64, u64, u32, TestPhase, TestDimension>,
) -> (
    u64,
    u64,
    FallbackHeaderSnapshot<TestPhase>,
    DimensionSnapshot,
) {
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::Success {
            canonical,
            outcome_evidence,
            ledger_evidence,
        } => (
            *canonical.root(),
            *outcome_evidence.root(),
            ledger_evidence
                .header()
                .expect("success terminal ledger header"),
            ledger_evidence
                .dimension(TestDimension::Bytes)
                .expect("success dimension evidence"),
        ),
        _ => panic!("success preparation must not publish a failure owner"),
    }
}

fn expect_retry_operation(
    error: RetainedPublicationOpenError<TestOperation, TestDimension>,
    context: &str,
) -> TestOperation {
    match error.into_recovery() {
        Ok(RetainedPublicationOpenRecovery::Retry(operation)) => operation,
        Ok(RetainedPublicationOpenRecovery::FallbackOnly(_)) => {
            panic!("{context}: failure unexpectedly consumed terminal authority")
        }
        Err(cause) => panic!("{context}: open failed closed: {cause:?}"),
    }
}

fn expect_fallback_only_operation(
    error: RetainedPublicationOpenError<TestOperation, TestDimension>,
    context: &str,
) -> TestOperation {
    match error.into_recovery() {
        Ok(RetainedPublicationOpenRecovery::FallbackOnly(operation)) => operation,
        Ok(RetainedPublicationOpenRecovery::Retry(_)) => {
            panic!("{context}: consumed terminal authority was returned as retryable")
        }
        Err(cause) => panic!("{context}: verified rollback failed closed: {cause:?}"),
    }
}

fn reject_invalid_publication_plan(
    operation: TestOperation,
    plan: RetainedPublicationPlan<TestPhase, TestDimension>,
    context: &str,
) -> TestOperation {
    let error = match operation.try_publication_assembly::<u64, u64>(plan) {
        Ok(_) => panic!("{context}: invalid publication plan unexpectedly opened"),
        Err(error) => error,
    };
    assert!(
        matches!(error.cause(), RetainedWriterError::InvalidPublicationPlan),
        "{context}: unexpected failure: {}",
        error.cause()
    );
    expect_retry_operation(error, context)
}

fn failure_terminal_operation(
    account: &PhysicalStorageAccount,
    ordinary_work_limit: u64,
    max_evidence_members: usize,
) -> TestOperation {
    let failure_actions = max_evidence_members
        .checked_add(2)
        .and_then(|actions| u64::try_from(actions).ok())
        .and_then(|actions| actions.checked_mul(2))
        .unwrap_or_else(|| panic!("failure-terminal test bound must fit"));
    failure_terminal_operation_with_limit(
        account,
        ordinary_work_limit,
        max_evidence_members,
        failure_actions,
    )
}

fn failure_terminal_operation_with_limit(
    account: &PhysicalStorageAccount,
    ordinary_work_limit: u64,
    max_evidence_members: usize,
    failure_work_limit: u64,
) -> TestOperation {
    let ordinary_policy = DimensionPolicy::reservable_bytes(
        ordinary_work_limit,
        ordinary_work_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(64);
    let failure_policy = DimensionPolicy::counter(failure_work_limit, failure_work_limit, 0);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (TestDimension::Bytes, ordinary_policy),
            (TestDimension::FailureTerminalWork, failure_policy),
        ],
    )
    .unwrap_or_else(|_| panic!("failure-terminal test limits must qualify"));
    let terminal_attribution_slots = max_evidence_members
        .checked_add(1)
        .unwrap_or_else(|| panic!("failure-terminal writer bound must fit"));
    let plan = RetainedOperationStartPlan::new(
        writer_plan(terminal_attribution_slots, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    )
    .with_failure_terminal_work(RetainedFailureTerminalWorkPlan::new(
        TestDimension::FailureTerminalWork,
        max_evidence_members,
    ));
    TestOperation::try_start(OperationControl::default(), limits, account, plan, || {
        Ok(7_u32)
    })
    .unwrap_or_else(|_| panic!("failure-terminal test operation must start"))
}

fn ordinary_linear_work_operation(
    account: &PhysicalStorageAccount,
    attempted_limit: u64,
    consumed_limit: u64,
) -> TestOperation {
    ordinary_linear_work_operation_with_control(
        account,
        attempted_limit,
        consumed_limit,
        OperationControl::default(),
    )
}

fn ordinary_linear_work_operation_with_control(
    account: &PhysicalStorageAccount,
    attempted_limit: u64,
    consumed_limit: u64,
    control: OperationControl<TestProgress, TestUnit>,
) -> TestOperation {
    let storage_policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let work_policy = DimensionPolicy::counter(attempted_limit, consumed_limit, 0);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (TestDimension::Bytes, storage_policy),
            (TestDimension::Work, work_policy),
        ],
    )
    .unwrap_or_else(|_| panic!("ordinary linear work limits must qualify"));
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    TestOperation::try_start(control, limits, account, plan, || Ok(7_u32))
        .unwrap_or_else(|_| panic!("ordinary linear work operation must start"))
}

fn finish_ordinary_linear_work_test(assembly: TestPublication<u64, u64>) -> PreparationFailureCode {
    let published = handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("ordinary linear work fallback must prepare")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        fallback,
        ledger_evidence,
    } = published
    else {
        panic!("ordinary linear work test must publish minimal fallback");
    };
    let cause = published_failure_cause(&ledger_evidence);
    drop(fallback);
    drop(ledger_evidence);
    cause
}

#[test]
fn ordinary_work_permit_records_exact_attempted_and_completed_work() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 8, 8);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("ordinary work publication must open"));

    let _evidence = assembly
        .try_retain_success_outcome_evidence_controlled(
            TestDimension::Work,
            empty_plan(),
            |emplacer, control| {
                let mut permit =
                    match control.try_begin(NonZeroU64::new(3).expect("nonzero work envelope")) {
                        Ok(permit) => permit,
                        Err(_) => panic!("exact ordinary work envelope must be admitted"),
                    };
                assert_eq!(permit.remaining_units(), 3);
                permit
                    .poll()
                    .unwrap_or_else(|_| panic!("work must remain live"));
                assert!(
                    permit
                        .complete_units(NonZeroU64::new(2).expect("nonzero completed prefix"))
                        .is_ok()
                );
                assert_eq!(permit.remaining_units(), 1);
                assert!(
                    permit
                        .complete_units(NonZeroU64::new(1).expect("nonzero completed suffix"))
                        .is_ok()
                );
                match permit.complete() {
                    Ok(_control) => {}
                    Err(RetainedWorkFinishError::Incomplete) => {
                        panic!("exact ordinary work permit must complete")
                    }
                }
                Ok(emplacer.place(7, EvidenceRootTag))
            },
        )
        .unwrap_or_else(|_| panic!("exact ordinary work evidence must retain"));

    let work = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Work)
        .unwrap_or_else(|| panic!("ordinary work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (3, 3));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::Requested
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn ordinary_work_permit_preserves_completed_prefix_when_dropped_incomplete() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 8, 8);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("incomplete-work publication must open"));

    assert!(matches!(
        assembly.try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
            TestDimension::Work,
            empty_plan(),
            |_emplacer, control| {
                let mut permit = match control
                    .try_begin(NonZeroU64::new(3).expect("nonzero work envelope"))
                {
                    Ok(permit) => permit,
                    Err(_) => panic!("incomplete ordinary work envelope must be admitted"),
                };
                assert!(
                    permit
                        .complete_units(NonZeroU64::new(2).expect("nonzero completed prefix"))
                        .is_ok()
                );
                drop(permit);
                Err(BUILD_FAILED)
            },
        ),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    let work = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Work)
        .unwrap_or_else(|| panic!("incomplete ordinary work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (3, 2));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::ProductGraph
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn borrowed_work_permit_resolves_verified_partial_work_without_abandonment() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 8, 8);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("partial-work publication must open"));

    assert!(matches!(
        assembly.try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
            TestDimension::Work,
            empty_plan(),
            |_emplacer, mut control| {
                let mut permit = control
                    .try_begin_borrowed(
                        NonZeroU64::new(3).expect("nonzero borrowed work envelope"),
                    )
                    .unwrap_or_else(|_| panic!("borrowed work envelope must be admitted"));
                permit
                    .complete_units(
                        NonZeroU64::new(2).expect("nonzero borrowed completed prefix"),
                    )
                    .unwrap_or_else(|_| panic!("borrowed completed prefix must fit"));
                permit.finish_partial();
                Err(BUILD_FAILED)
            },
        ),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    let work = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Work)
        .unwrap_or_else(|| panic!("borrowed work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (3, 2));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::Requested
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn dropped_borrowed_work_permit_preserves_prefix_and_latches_abandonment() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 8, 8);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("abandoned borrowed-work publication must open"));

    assert!(matches!(
        assembly.try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
            TestDimension::Work,
            empty_plan(),
            |_emplacer, mut control| {
                let mut permit = control
                    .try_begin_borrowed(
                        NonZeroU64::new(3).expect("nonzero abandoned work envelope"),
                    )
                    .unwrap_or_else(|_| panic!("abandoned work envelope must be admitted"));
                permit
                    .complete_units(
                        NonZeroU64::new(1).expect("nonzero abandoned completed prefix"),
                    )
                    .unwrap_or_else(|_| panic!("abandoned completed prefix must fit"));
                drop(permit);
                Err(BUILD_FAILED)
            },
        ),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    let work = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Work)
        .unwrap_or_else(|| panic!("abandoned borrowed-work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (3, 1));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::ProductGraph
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

fn assert_unresolved_validation_permit_cannot_validate_success(forget: bool) {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 8, 8);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("abandoned-validation publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let validation = assembly.validate_success_controlled(
        TestDimension::Work,
        TestDimension::Work,
        canonical_root,
        evidence_root,
        |session, canonical, evidence, mut control| {
            let mut permit = control
                .try_begin_borrowed(NonZeroU64::MIN)
                .unwrap_or_else(|_| panic!("validation work must be admitted"));
            permit
                .complete_units(NonZeroU64::MIN)
                .unwrap_or_else(|_| panic!("validation work must complete"));
            if forget {
                std::mem::forget(permit);
            } else {
                drop(permit);
            }
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    );
    if forget {
        let fallback = match validation {
            RetainedSuccessValidation::FallbackOnly(fallback) => fallback,
            RetainedSuccessValidation::Valid(_) | RetainedSuccessValidation::Rejected(_) => {
                panic!("a forgotten validation permit must remain unresolvable")
            }
        };
        let containment = match fallback.prepare_minimal_fallback() {
            Ok(_) => panic!("a forgotten validation permit must not prepare any exact owner"),
            Err(RetainedPublicationPrepareError::FinalizationContainment(containment)) => {
                containment
            }
        };
        assert_eq!(
            containment.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        let snapshot = account.snapshot();
        assert!(snapshot.poisoned());
        assert!(snapshot.live_retained_allocations() > 0);
        drop(account);
        assert!(factory.snapshot().live_accounts() > 0);
        return;
    }
    let rejected = match validation {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        RetainedSuccessValidation::Valid(_) => {
            panic!("a dropped validation permit must not produce a success proof")
        }
        RetainedSuccessValidation::FallbackOnly(_) => {
            panic!("a dropped validation permit must release its linear capability")
        }
    };
    let published = handoff_for_test(
        rejected
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("dropped validation rejection must prepare fallback")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        ledger_evidence, ..
    } = &published
    else {
        panic!("abandoned validation must publish minimal fallback")
    };
    let cause = published_failure_cause(ledger_evidence);
    assert_eq!(cause.class(), PreparationFailureClass::ProductGraph);
    assert_eq!(cause.code(), 0x4f57_5041);
    let work = ledger_evidence
        .dimension(TestDimension::Work)
        .unwrap_or_else(|| panic!("dropped validation work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (1, 1));
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn dropped_validation_permit_cannot_validate_success() {
    assert_unresolved_validation_permit_cannot_validate_success(false);
}

#[test]
fn forgotten_validation_permit_forces_finalization_containment() {
    assert_unresolved_validation_permit_cannot_validate_success(true);
}

#[test]
fn partially_resolved_validation_work_cannot_validate_success() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 8, 8);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("partial-validation publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let rejected = match assembly.validate_success_controlled(
        TestDimension::Work,
        TestDimension::Work,
        canonical_root,
        evidence_root,
        |session, canonical, evidence, mut control| {
            let mut permit = control
                .try_begin_borrowed(
                    NonZeroU64::new(3).expect("nonzero partial validation envelope"),
                )
                .unwrap_or_else(|_| panic!("partial validation work must be admitted"));
            permit
                .complete_units(NonZeroU64::new(2).expect("nonzero completed prefix"))
                .unwrap_or_else(|_| panic!("partial validation prefix must fit"));
            permit.finish_partial();
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        RetainedSuccessValidation::Valid(_) | RetainedSuccessValidation::FallbackOnly(_) => {
            panic!("partially resolved validation work must reject success")
        }
    };
    let published = handoff_for_test(
        rejected
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("partial validation rejection must prepare fallback")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        ledger_evidence, ..
    } = &published
    else {
        panic!("partial validation rejection must publish minimal fallback")
    };
    assert_eq!(
        published_failure_cause(ledger_evidence).class(),
        PreparationFailureClass::ProductGraph
    );
    let work = ledger_evidence
        .dimension(TestDimension::Work)
        .unwrap_or_else(|| panic!("partial validation work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (3, 2));
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn ignored_validation_control_failure_cannot_validate_success() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let cancellation = crate::CancellationToken::new();
    let operation = ordinary_linear_work_operation_with_control(
        &account,
        8,
        8,
        OperationControl::default().with_cancellation(cancellation.clone()),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("control-failure publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let rejected = match assembly.validate_success_controlled(
        TestDimension::Work,
        TestDimension::Work,
        canonical_root,
        evidence_root,
        |session, canonical, evidence, mut control| {
            cancellation.cancel();
            assert!(control.poll().is_err());
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        RetainedSuccessValidation::Valid(_) | RetainedSuccessValidation::FallbackOnly(_) => {
            panic!("ignored validation control failure must reject success")
        }
    };
    let published = handoff_for_test(
        rejected
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("control rejection must prepare fallback")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        ledger_evidence, ..
    } = &published
    else {
        panic!("control rejection must publish minimal fallback")
    };
    assert_eq!(
        published_failure_cause(ledger_evidence).class(),
        PreparationFailureClass::Control
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn ignored_validation_ledger_failure_cannot_validate_success() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 0, 0);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("ledger-failure publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let rejected = match assembly.validate_success_controlled(
        TestDimension::Work,
        TestDimension::Work,
        canonical_root,
        evidence_root,
        |session, canonical, evidence, mut control| {
            assert!(control.charge_observed(1).is_err());
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        RetainedSuccessValidation::Valid(_) | RetainedSuccessValidation::FallbackOnly(_) => {
            panic!("ignored validation ledger failure must reject success")
        }
    };
    let published = handoff_for_test(
        rejected
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("ledger rejection must prepare fallback")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        ledger_evidence, ..
    } = &published
    else {
        panic!("ledger rejection must publish minimal fallback")
    };
    assert_eq!(
        published_failure_cause(ledger_evidence).class(),
        PreparationFailureClass::Ledger
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn borrowed_work_permit_preserves_prefix_before_mid_action_cancellation() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let cancellation = crate::CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let operation = ordinary_linear_work_operation_with_control(&account, 8, 8, control);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("cancellable borrowed-work publication must open"));

    assert!(matches!(
        assembly.try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
            TestDimension::Work,
            empty_plan(),
            |_emplacer, mut control| {
                let mut permit = control
                    .try_begin_borrowed(
                        NonZeroU64::new(3).expect("nonzero cancellable work envelope"),
                    )
                    .unwrap_or_else(|_| panic!("cancellable work envelope must be admitted"));
                permit
                    .complete_units(
                        NonZeroU64::new(1).expect("nonzero pre-cancellation prefix"),
                    )
                    .unwrap_or_else(|_| panic!("pre-cancellation prefix must fit"));
                cancellation.cancel();
                assert!(permit.poll().is_err());
                permit.finish_partial();
                Err(BUILD_FAILED)
            },
        ),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    let work = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Work)
        .unwrap_or_else(|| panic!("cancellable borrowed-work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (3, 1));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::Control
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn ordinary_work_permit_rejects_consumed_overrun_without_fabricating_completion() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 8, 8);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("overrun publication must open"));

    assert!(matches!(
        assembly.try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
            TestDimension::Work,
            empty_plan(),
            |_emplacer, control| {
                let mut permit = match control
                    .try_begin(NonZeroU64::new(3).expect("nonzero work envelope"))
                {
                    Ok(permit) => permit,
                    Err(_) => panic!("overrun ordinary work envelope must be admitted"),
                };
                assert!(matches!(
                    permit.complete_units(
                        NonZeroU64::new(4).expect("nonzero overrun completion")
                    ),
                    Err(RetainedWorkConsumeError::ExceedsAttempted)
                ));
                Err(BUILD_FAILED)
            },
        ),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    let work = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Work)
        .unwrap_or_else(|| panic!("overrun ordinary work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (3, 0));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::ProductGraph
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn ordinary_work_admission_rejects_short_consumed_capacity_before_action() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = ordinary_linear_work_operation(&account, 8, 2);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("short-capacity publication must open"));
    let mut builder_ran = false;
    let mut action_ran = false;

    assert!(matches!(
        assembly.try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
            TestDimension::Work,
            empty_plan(),
            |_emplacer, control| {
                builder_ran = true;
                match control.try_begin(NonZeroU64::new(3).expect("nonzero work envelope")) {
                    Err(RetainedWorkStartError::Ledger(LedgerCause::LimitExceeded {
                        class: CounterClass::Consumed,
                        dimension: TestDimension::Work,
                        value: 3,
                        limit: 2,
                    })) => return Err(BUILD_FAILED),
                    Ok(_) => action_ran = true,
                    Err(_) => panic!("short consumed capacity returned the wrong cause"),
                }
                Err(BUILD_FAILED)
            },
        ),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    assert!(builder_ran, "the retained builder callback must execute");
    assert!(
        !action_ran,
        "the protected action must not start after rejection"
    );
    let work = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Work)
        .unwrap_or_else(|| panic!("short-capacity ordinary work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (3, 0));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::Ledger
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn ordinary_work_admission_observes_preexisting_cancellation_before_attempting_work() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let cancellation = crate::CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let operation = ordinary_linear_work_operation_with_control(&account, 8, 8, control);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|error| panic!("cancellation publication must open: {error:?}"));
    cancellation.cancel();
    let mut action_ran = false;

    assert!(matches!(
        assembly.try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
            TestDimension::Work,
            empty_plan(),
            |_emplacer, control| {
                match control.try_begin(NonZeroU64::new(3).expect("nonzero work envelope")) {
                    Err(RetainedWorkStartError::Control(_)) => return Err(BUILD_FAILED),
                    Ok(_) => action_ran = true,
                    Err(_) => panic!("preexisting cancellation returned the wrong cause"),
                }
                Err(BUILD_FAILED)
            },
        ),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    assert!(
        !action_ran,
        "cancelled work must not enter the protected action"
    );
    let work = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Work)
        .unwrap_or_else(|| panic!("cancelled ordinary work evidence"));
    assert_eq!((work.attempted(), work.consumed()), (0, 0));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::Control
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn ordinary_work_permit_cannot_address_the_protected_failure_dimension() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation(&account, 64, 1);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("protected-dimension publication must open"));

    assert!(matches!(
        assembly.try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
            TestDimension::FailureTerminalWork,
            empty_plan(),
            |_emplacer, control| {
                assert!(matches!(
                    control.try_begin(NonZeroU64::new(1).expect("nonzero work envelope")),
                    Err(RetainedWorkStartError::Ledger(LedgerCause::UnknownDimension {
                        dimension: TestDimension::FailureTerminalWork,
                    }))
                ));
                Err(BUILD_FAILED)
            },
        ),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    let protected = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected work evidence"));
    assert_eq!((protected.attempted(), protected.consumed()), (0, 0));
    assert_eq!(
        finish_ordinary_linear_work_test(assembly).class(),
        PreparationFailureClass::Ledger
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn rejected_failure_terminal_preflight_rolls_back_started_storage() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let ordinary_policy = DimensionPolicy::reservable_bytes(8, 8, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let failure_policy = DimensionPolicy::counter(5, 5, 0);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (TestDimension::Bytes, ordinary_policy),
            (TestDimension::FailureTerminalWork, failure_policy),
        ],
    )
    .unwrap_or_else(|_| panic!("failure-terminal rejection limits must qualify"));
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    )
    .with_failure_terminal_work(RetainedFailureTerminalWorkPlan::new(
        TestDimension::FailureTerminalWork,
        1,
    ));

    assert!(
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || Ok(
            7_u32
        ),)
        .is_err()
    );
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn publication_key_membership_preflight_preserves_terminal_capability() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation(&account, 64, 1);
    let invalid_phase = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
        RetainedWriterPlan::new(
            TestPhase::Unregistered,
            TestDimension::Bytes,
            TestDimension::Bytes,
            RetainedBatchLimits::new(2, 2),
        ),
        1,
        1,
    ));
    let phase_error = match operation.try_publication_assembly::<u64, u64>(invalid_phase) {
        Ok(_) => panic!("an unregistered writer phase must fail pure preflight"),
        Err(error) => error,
    };
    assert!(matches!(
        phase_error.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let operation = expect_retry_operation(phase_error, "phase preflight");

    let invalid_dimension = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
        RetainedWriterPlan::new(
            TestPhase::Build,
            TestDimension::Unregistered,
            TestDimension::Bytes,
            RetainedBatchLimits::new(2, 2),
        ),
        1,
        1,
    ));
    let dimension_error = match operation.try_publication_assembly::<u64, u64>(invalid_dimension) {
        Ok(_) => panic!("an unregistered writer dimension must fail pure preflight"),
        Err(error) => error,
    };
    assert!(matches!(
        dimension_error.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let mut operation = expect_retry_operation(dimension_error, "dimension preflight");

    let invalid_writers = [
        (
            "evidence attribution dimension",
            false,
            RetainedWriterPlan::new(
                TestPhase::Build,
                TestDimension::Bytes,
                TestDimension::Unregistered,
                RetainedBatchLimits::new(2, 2),
            ),
        ),
        (
            "canonical phase",
            true,
            RetainedWriterPlan::new(
                TestPhase::Unregistered,
                TestDimension::Bytes,
                TestDimension::Bytes,
                RetainedBatchLimits::new(2, 2),
            ),
        ),
        (
            "canonical capacity dimension",
            true,
            RetainedWriterPlan::new(
                TestPhase::Build,
                TestDimension::Unregistered,
                TestDimension::Bytes,
                RetainedBatchLimits::new(2, 2),
            ),
        ),
        (
            "canonical attribution dimension",
            true,
            RetainedWriterPlan::new(
                TestPhase::Build,
                TestDimension::Bytes,
                TestDimension::Unregistered,
                RetainedBatchLimits::new(2, 2),
            ),
        ),
        (
            "protected capacity dimension",
            false,
            RetainedWriterPlan::new(
                TestPhase::Build,
                TestDimension::FailureTerminalWork,
                TestDimension::Bytes,
                RetainedBatchLimits::new(2, 2),
            ),
        ),
        (
            "protected attribution dimension",
            false,
            RetainedWriterPlan::new(
                TestPhase::Build,
                TestDimension::Bytes,
                TestDimension::FailureTerminalWork,
                RetainedBatchLimits::new(2, 2),
            ),
        ),
    ];
    for (context, canonical_destination, writer) in invalid_writers {
        let table = RetainedBlockTablePlan::new(writer, 1, 1);
        let plan = if canonical_destination {
            RetainedPublicationPlan::success(
                table,
                RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
            )
        } else {
            RetainedPublicationPlan::rich_failure(table)
        };
        operation = reject_invalid_publication_plan(operation, plan, context);
    }

    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(
            RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
        ))
        .unwrap_or_else(|_| panic!("corrected plan must retain terminal capability"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("corrected evidence must retain"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("corrected rich-failure graph must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(703), final_progress())
            .unwrap_or_else(|_| panic!("corrected rich failure must prepare")),
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn startup_qualified_failure_terminal_work_survives_exhausted_ordinary_work() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let ordinary_policy = DimensionPolicy::reservable_bytes(8, 8, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let failure_policy = DimensionPolicy::counter(6, 6, 0);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (TestDimension::Bytes, ordinary_policy),
            (TestDimension::FailureTerminalWork, failure_policy),
        ],
    )
    .unwrap_or_else(|_| panic!("failure-terminal test limits must qualify"));
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    )
    .with_failure_terminal_work(RetainedFailureTerminalWorkPlan::new(
        TestDimension::FailureTerminalWork,
        1,
    ));
    let operation =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("failure-terminal operation must start"));

    assert!(
        operation
            .coordinator
            .retained_ledger()
            .charge_observed_batch(&[(TestDimension::Bytes, 1)])
            .is_err(),
        "ordinary work must retain its non-refundable failing attempt"
    );
    let ordinary = operation
        .coordinator
        .dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|_| panic!("ordinary dimension evidence"));
    assert_eq!(ordinary.attempted(), 9);
    assert_eq!(ordinary.consumed(), 8);

    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(
            RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
        ))
        .unwrap_or_else(|_| panic!("reserved rich-failure assembly must open"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("reserved rich-failure evidence must retain"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("reserved rich-failure evidence must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(701), final_progress())
            .unwrap_or_else(|_| panic!("reserved rich failure must prepare")),
    );
    let PublishedRetainedPublicationParts::RichFailure {
        outcome_evidence,
        ledger_evidence,
    } = published
    else {
        panic!("reserved terminal work must publish rich failure");
    };
    let ordinary = ledger_evidence
        .dimension(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("published ordinary dimension"));
    let failure = ledger_evidence
        .dimension(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("published failure-terminal dimension"));
    assert_eq!((ordinary.attempted(), ordinary.consumed()), (9, 8));
    assert_eq!((failure.attempted(), failure.consumed()), (6, 6));
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn recoverable_failure_terminal_preflight_preserves_capability_for_retry() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation(&account, 64, 1);
    let invalid_plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 2, 2));

    let error = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
        Ok(_) => panic!("oversized failure evidence plan must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let operation = expect_retry_operation(error, "failure-terminal plan preflight");

    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(
            RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
        ))
        .unwrap_or_else(|_| panic!("corrected rich-failure plan must retain capability"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("corrected rich-failure evidence must retain"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("corrected rich-failure graph must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(702), final_progress())
            .unwrap_or_else(|_| panic!("corrected rich failure must prepare")),
    );
    let PublishedRetainedPublicationParts::RichFailure {
        outcome_evidence,
        ledger_evidence,
    } = published
    else {
        panic!("corrected rich-failure plan must publish rich evidence");
    };
    let terminal = ledger_evidence
        .dimension(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("published failure-terminal dimension"));
    assert_eq!((terminal.attempted(), terminal.consumed()), (6, 6));
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn post_preflight_open_failure_burns_failure_terminal_capability() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation(&account, 64, 1);
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));

    fail_next_final_owner_allocation();
    let error = match operation.try_publication_assembly::<u64, u64>(plan) {
        Ok(_) => panic!("injected post-preflight allocation must fail"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = expect_fallback_only_operation(error, "post-preflight allocation failure");
    let terminal = operation
        .coordinator
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|_| panic!("failure-terminal evidence after attempted open"));
    assert_eq!((terminal.attempted(), terminal.consumed()), (4, 4));

    let published = handoff_for_test(
        operation
            .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(704))
            .unwrap_or_else(|_| panic!("burned operation must retain minimal fallback")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        fallback,
        ledger_evidence,
    } = published
    else {
        panic!("burned terminal authority must end in minimal fallback");
    };
    drop(fallback);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn validated_success_can_downgrade_after_ordinary_work_is_exhausted() {
    const ORDINARY_WORK_LIMIT: u64 = 64;
    const DOWNGRADE_CAUSE: u32 = 703;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation(&account, ORDINARY_WORK_LIMIT, 2);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 2))
        .unwrap_or_else(|_| panic!("success-capable publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
    let ordinary = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ordinary dimension evidence before exhaustion"));
    let remaining = ORDINARY_WORK_LIMIT
        .checked_sub(ordinary.consumed())
        .unwrap_or_else(|| panic!("success candidate exceeded ordinary work limit"));
    assembly
        .try_control_checkpoint()
        .unwrap_or_else(|_| panic!("success candidate control checkpoint"))
        .charge_observed(TestDimension::Bytes, remaining)
        .unwrap_or_else(|_| panic!("ordinary work must exhaust exactly"));

    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("complete success candidate must validate"),
    };
    let mut failure =
        validated.downgrade_to_rich_failure(PreparationFailureCode::domain(DOWNGRADE_CAUSE));
    let detail = failure
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("protected terminal work must retain downgrade evidence"));
    let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
        session.outcome_evidence(root) == Some(&7) && session.outcome_evidence(detail) == Some(&9)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("downgraded evidence graph must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(
                PreparationFailureCode::domain(DOWNGRADE_CAUSE),
                final_progress(),
            )
            .unwrap_or_else(|_| panic!("downgraded rich failure must prepare")),
    );
    let PublishedRetainedPublicationParts::RichFailure {
        outcome_evidence,
        ledger_evidence,
    } = published
    else {
        panic!("admission downgrade must publish rich failure");
    };
    assert_eq!(
        published_failure_cause(&ledger_evidence),
        PreparationFailureCode::domain(DOWNGRADE_CAUSE)
    );
    let ordinary = ledger_evidence
        .dimension(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("published ordinary dimension"));
    let terminal = ledger_evidence
        .dimension(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("published failure-terminal dimension"));
    assert_eq!(
        (ordinary.attempted(), ordinary.consumed()),
        (ORDINARY_WORK_LIMIT, ORDINARY_WORK_LIMIT)
    );
    assert_eq!((terminal.attempted(), terminal.consumed()), (2, 2));
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn successful_publication_leaves_failure_terminal_work_unused() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation(&account, 64, 2);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("success publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("success graph must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("success publication must prepare")),
    );
    let PublishedRetainedPublicationParts::Success {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = published
    else {
        panic!("valid success must publish canonical output");
    };
    let terminal = ledger_evidence
        .dimension(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("published failure-terminal dimension"));
    assert_eq!((terminal.attempted(), terminal.consumed()), (0, 0));
    drop(canonical);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn direct_rich_validation_of_a_success_plan_activates_protected_work() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation(&account, 64, 2);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 2))
        .unwrap_or_else(|_| panic!("success-capable publication must open"));
    let (_canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let mut failure = match assembly.validate_rich_failure(evidence_root, |_session, _root| false) {
        RetainedFailureValidation::Rejected(failure) => failure,
        _ => panic!("direct rich validation must retain a failure assembly"),
    };
    let ordinary_before = failure
        .assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ordinary evidence before protected append"));
    let detail = failure
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("direct rich validation must activate protected append work"));
    let ordinary_after = failure
        .assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ordinary evidence after protected append"));
    let protected = failure
        .assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected evidence after direct rich validation"));
    assert_eq!(ordinary_after.attempted(), ordinary_before.attempted());
    assert_eq!(ordinary_after.consumed(), ordinary_before.consumed());
    assert!(ordinary_after.current_live_bytes() > ordinary_before.current_live_bytes());
    assert_eq!((protected.attempted(), protected.consumed()), (2, 2));

    let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
        session.outcome_evidence(root) == Some(&7) && session.outcome_evidence(detail) == Some(&9)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("direct rich-failure graph must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(705), final_progress())
            .unwrap_or_else(|_| panic!("direct rich failure must prepare")),
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn failed_protected_evidence_action_seals_the_terminal_rail() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation_with_limit(&account, 64, 2, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(
            RetainedBlockTablePlan::new(writer_plan(3, 2), 2, 2),
        ))
        .unwrap_or_else(|_| panic!("protected failure assembly must open"));
    let evidence_root = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("protected evidence root must retain"));

    assert!(matches!(
        assembly.try_retain_outcome_evidence::<EvidenceDetailTag>(empty_plan(), |_emplacer| {
            Err(BUILD_FAILED)
        }),
        Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::Build(cause)
        ))) if cause == BUILD_FAILED
    ));
    let after_first_failure = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("failure-terminal evidence after protected failure"));

    assert!(matches!(
        assembly.try_retain_outcome_evidence::<EvidenceDetailTag>(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        }),
        Err(RetainedPublicationError::FailureTerminalWorkSealed)
    ));
    let after_rejected_retry = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("failure-terminal evidence after sealed retry"));
    assert_eq!(after_rejected_retry, after_first_failure);

    let fallback_only = match assembly.validate_rich_failure(evidence_root, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::FallbackOnly(fallback_only) => fallback_only,
        _ => panic!("failed protected work must seal the graph to minimal fallback"),
    };
    let published = handoff_for_test(
        fallback_only
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("sealed protected rail must prepare minimal fallback")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        fallback,
        ledger_evidence,
    } = published
    else {
        panic!("sealed protected rail must publish minimal fallback");
    };
    let cause = published_failure_cause(&ledger_evidence);
    assert_eq!(cause.class(), PreparationFailureClass::ProductGraph);
    assert_eq!(cause.code(), FAILURE_TERMINAL_WORK_SEALED_CODE);
    drop(fallback);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn active_and_failed_terminal_states_seal_canonical_and_success_paths() {
    const ORDINARY_WORK_LIMIT: u64 = 64;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = failure_terminal_operation(&account, ORDINARY_WORK_LIMIT, 3);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(2, 3))
        .unwrap_or_else(|_| panic!("success-capable publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
    let ordinary = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ordinary evidence before terminal rejection"));
    let failing_amount = ORDINARY_WORK_LIMIT
        .checked_sub(ordinary.consumed())
        .and_then(|remaining| remaining.checked_add(1))
        .unwrap_or_else(|| panic!("ordinary failing amount must fit"));
    assert!(matches!(
        assembly
            .try_control_checkpoint()
            .unwrap_or_else(|_| panic!("ordinary control checkpoint"))
            .charge_observed(TestDimension::Bytes, failing_amount),
        Err(LedgerCause::LimitExceeded { .. })
    ));

    let detail = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("recorded rejection must activate protected evidence work"));
    let protected_after_activation = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected evidence after activation"));
    assert_eq!(
        (
            protected_after_activation.attempted(),
            protected_after_activation.consumed()
        ),
        (2, 2)
    );

    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(12, CanonicalRootTag))
        }),
        Err(RetainedPublicationError::SuccessOutcomeRejected)
    ));
    let protected_after_canonical_retry = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected evidence after canonical retry"));
    assert_eq!(protected_after_canonical_retry, protected_after_activation);

    assert!(matches!(
        assembly.try_retain_outcome_evidence::<EvidenceDetailTag>(empty_plan(), |_emplacer| {
            Err(BUILD_FAILED)
        }),
        Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::Build(cause)
        ))) if cause == BUILD_FAILED
    ));
    let protected_after_failure = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected evidence after sealed failure"));
    assert!(matches!(
        assembly.try_commit_outcome_evidence(TestDimension::Bytes, 1),
        Err(RetainedPublicationError::FailureTerminalWorkSealed)
    ));
    let protected_after_commit = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected evidence after sealed commitment"));
    assert_eq!(protected_after_commit, protected_after_failure);

    let fallback_only = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
                && session.outcome_evidence(detail) == Some(&9)
        },
    ) {
        RetainedSuccessValidation::FallbackOnly(fallback_only) => fallback_only,
        _ => panic!("failed protected work must prohibit success validation"),
    };
    let published = handoff_for_test(
        fallback_only
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("failed protected work must prepare minimal fallback")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        fallback,
        ledger_evidence,
    } = published
    else {
        panic!("failed protected work must publish minimal fallback");
    };
    assert_eq!(
        published_failure_cause(&ledger_evidence).class(),
        PreparationFailureClass::Ledger
    );
    drop(fallback);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[derive(Debug, Clone, Copy)]
enum FailureGuardCase {
    RichEvidence,
    RichBoth,
    MinimalNone,
    MinimalEvidence,
    MinimalBoth,
}

impl FailureGuardCase {
    const fn rail(self) -> FailureTerminalRail {
        match self {
            Self::RichEvidence | Self::RichBoth => FailureTerminalRail::Rich,
            Self::MinimalNone | Self::MinimalEvidence | Self::MinimalBoth => {
                FailureTerminalRail::Minimal
            }
        }
    }

    const fn has_discard(self) -> bool {
        !matches!(self, Self::MinimalNone)
    }
}

#[derive(Debug, Clone, Copy)]
enum FailureFault {
    Guard(FailureGuardDropStage),
    EvidenceWrite,
}

fn arm_failure_fault(case: FailureGuardCase, fault: FailureFault) {
    match fault {
        FailureFault::Guard(stage) => drop_next_failure_guard(case.rail(), stage),
        FailureFault::EvidenceWrite => panic_next_terminal_evidence_write(),
    }
}

fn run_failure_guard_case(case: FailureGuardCase, fault: FailureFault) {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let evidence_limit = match case {
        FailureGuardCase::MinimalEvidence | FailureGuardCase::MinimalBoth => 4,
        FailureGuardCase::RichEvidence
        | FailureGuardCase::RichBoth
        | FailureGuardCase::MinimalNone => 64,
    };
    let operation = test_operation_on_account_with_control(
        &account,
        evidence_limit,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.coordinator.retained_ledger().test_probe();

    macro_rules! run_prepare {
        ($prepare:expr) => {{
            let physical = account.snapshot();
            let physical_before = (
                physical.live_retained_bytes(),
                physical.live_retained_allocations(),
            );
            let ledger_before = (ledger.active_reservations(), ledger.current_live_bytes());
            arm_failure_fault(case, fault);
            let unwind = catch_unwind(AssertUnwindSafe(|| {
                let _ = $prepare;
            }));
            (physical_before, ledger_before, unwind)
        }};
    }

    let (physical_before, ledger_before, unwind) = match case {
        FailureGuardCase::RichEvidence => {
            let plan = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(2, 2),
                1,
                1,
            ));
            let mut assembly: TestPublication<u64, u64> = operation
                .try_publication_assembly(plan)
                .unwrap_or_else(|_| panic!("rich-evidence fixture must open"));
            let evidence = assembly
                .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                    Ok(emplacer.place(7, EvidenceRootTag))
                })
                .unwrap_or_else(|_| panic!("rich-evidence root must retain"));
            let validated = match assembly.validate_rich_failure(evidence, |session, root| {
                session.outcome_evidence(root) == Some(&7)
            }) {
                RetainedFailureValidation::Valid(validated) => validated,
                _ => panic!("rich-evidence fixture must validate"),
            };
            run_prepare!(
                validated.try_prepare(PreparationFailureCode::domain(201), final_progress())
            )
        }
        FailureGuardCase::RichBoth => {
            let mut assembly: TestPublication<u64, u64> = operation
                .try_publication_assembly(publication_plan(1, 1))
                .unwrap_or_else(|_| panic!("rich-both fixture must open"));
            let (canonical, evidence) = retain_scalar_success(&mut assembly);
            let failure = match assembly.validate_success(
                canonical,
                evidence,
                |_session, _canonical, _evidence| false,
            ) {
                RetainedSuccessValidation::Rejected(failure) => failure,
                _ => panic!("rich-both fixture must enter failure validation"),
            };
            let validated = match failure.validate_rich_failure(evidence, |session, root| {
                session.outcome_evidence(root) == Some(&7)
            }) {
                RetainedFailureValidation::Valid(validated) => validated,
                _ => panic!("rich-both fixture must validate"),
            };
            run_prepare!(
                validated.try_prepare(PreparationFailureCode::domain(202), final_progress())
            )
        }
        FailureGuardCase::MinimalNone => run_prepare!(
            operation.prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(203))
        ),
        FailureGuardCase::MinimalEvidence => {
            let plan = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(2, 2),
                1,
                1,
            ));
            let mut assembly: TestPublication<u64, u64> = operation
                .try_publication_assembly(plan)
                .unwrap_or_else(|_| panic!("minimal-evidence fixture must open"));
            let evidence = assembly
                .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                    Ok(emplacer.place(7, EvidenceRootTag))
                })
                .unwrap_or_else(|_| panic!("minimal-evidence root must retain"));
            assembly
                .try_commit_outcome_evidence(TestDimension::Bytes, 4)
                .unwrap_or_else(|_| panic!("minimal-evidence commitment must fit"));
            let validated = match assembly.validate_rich_failure(evidence, |session, root| {
                session.outcome_evidence(root) == Some(&7)
            }) {
                RetainedFailureValidation::Valid(validated) => validated,
                _ => panic!("minimal-evidence fixture must validate"),
            };
            run_prepare!(
                validated.try_prepare(PreparationFailureCode::domain(204), final_progress())
            )
        }
        FailureGuardCase::MinimalBoth => {
            let mut assembly: TestPublication<u64, u64> = operation
                .try_publication_assembly(publication_plan(1, 1))
                .unwrap_or_else(|_| panic!("minimal-both fixture must open"));
            let (canonical, evidence) = retain_scalar_success(&mut assembly);
            assembly
                .try_commit_outcome_evidence(TestDimension::Bytes, 4)
                .unwrap_or_else(|_| panic!("minimal-both commitment must fit"));
            let validated = match assembly.validate_success(
                canonical,
                evidence,
                |session, canonical, evidence| {
                    session.canonical(canonical) == Some(&11)
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            ) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("minimal-both fixture must validate"),
            };
            run_prepare!(validated.try_prepare(final_progress()))
        }
    };

    assert!(unwind.is_err(), "{case:?} {fault:?} must unwind");
    let physical_after = account.snapshot();
    let physical_after_counts = (
        physical_after.live_retained_bytes(),
        physical_after.live_retained_allocations(),
    );
    let ledger_after = (ledger.active_reservations(), ledger.current_live_bytes());
    assert_eq!(
        physical_after
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified),
        "{case:?} {fault:?} must contain the exact physical continuation"
    );

    match fault {
        FailureFault::Guard(FailureGuardDropStage::Qualified) => {
            assert!(ledger.is_terminal_qualified());
            assert_eq!(physical_after_counts, physical_before);
            assert_eq!(ledger_after, ledger_before);
        }
        FailureFault::Guard(FailureGuardDropStage::CanonicalAcknowledged) => {
            assert!(ledger.is_terminal_qualified());
            assert!(physical_after_counts.0 < physical_before.0);
            assert!(physical_after_counts.1 < physical_before.1);
            assert!(ledger_after.0 < ledger_before.0);
            assert!(ledger_after.1 < ledger_before.1);
            assert!(ledger_after.0 > 0);
            assert!(ledger_after.1 > 0);
        }
        FailureFault::Guard(FailureGuardDropStage::DiscardCleaned) => {
            assert!(ledger.is_terminal_qualified());
            assert!(physical_after_counts.0 < physical_before.0);
            assert!(physical_after_counts.1 < physical_before.1);
            assert!(ledger_after.0 <= ledger_before.0);
            assert!(ledger_after.1 <= ledger_before.1);
            assert!(ledger_after.0 > 0);
            assert!(ledger_after.1 > 0);
        }
        FailureFault::Guard(FailureGuardDropStage::FreezeReady) => {
            assert!(ledger.is_terminal_qualified());
            assert!(physical_after_counts.0 <= physical_before.0);
            assert!(physical_after_counts.1 <= physical_before.1);
            assert!(ledger_after.0 <= ledger_before.0);
            assert!(ledger_after.1 <= ledger_before.1);
            if case.has_discard() {
                assert!(physical_after_counts.0 < physical_before.0);
                assert!(physical_after_counts.1 < physical_before.1);
                assert!(ledger_after.0 < ledger_before.0);
                assert!(ledger_after.1 < ledger_before.1);
            }
        }
        FailureFault::Guard(
            FailureGuardDropStage::EligibilityReady | FailureGuardDropStage::LedgerDetached,
        )
        | FailureFault::EvidenceWrite => {
            assert!(!ledger.is_available());
            assert_eq!(ledger_after, (0, 0));
            assert!(physical_after_counts.0 > 0);
            assert!(physical_after_counts.1 > 0);
        }
    }
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn rich_and_minimal_owning_guards_contain_direct_drop_at_every_stable_state() {
    for case in [
        FailureGuardCase::RichEvidence,
        FailureGuardCase::RichBoth,
        FailureGuardCase::MinimalNone,
        FailureGuardCase::MinimalEvidence,
        FailureGuardCase::MinimalBoth,
    ] {
        for stage in [
            FailureGuardDropStage::Qualified,
            FailureGuardDropStage::EligibilityReady,
            FailureGuardDropStage::FreezeReady,
        ] {
            run_failure_guard_case(case, FailureFault::Guard(stage));
        }
    }
}

#[test]
fn both_discard_continuation_owns_the_graph_after_canonical_acknowledgement() {
    for case in [FailureGuardCase::RichBoth, FailureGuardCase::MinimalBoth] {
        run_failure_guard_case(
            case,
            FailureFault::Guard(FailureGuardDropStage::CanonicalAcknowledged),
        );
    }
}

#[test]
fn evidence_discard_continuation_owns_the_graph_after_physical_cleanup() {
    for case in [
        FailureGuardCase::RichEvidence,
        FailureGuardCase::RichBoth,
        FailureGuardCase::MinimalEvidence,
        FailureGuardCase::MinimalBoth,
    ] {
        run_failure_guard_case(
            case,
            FailureFault::Guard(FailureGuardDropStage::DiscardCleaned),
        );
    }
}

#[test]
fn failure_rails_contain_both_sides_of_the_ledger_sealed_evidence_write_window() {
    for case in [
        FailureGuardCase::RichEvidence,
        FailureGuardCase::MinimalNone,
    ] {
        run_failure_guard_case(
            case,
            FailureFault::Guard(FailureGuardDropStage::LedgerDetached),
        );
        run_failure_guard_case(case, FailureFault::EvidenceWrite);
    }
}

#[test]
fn block_table_success_freezes_dual_batches_and_splits_owners() {
    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
    operation
        .try_complete_phase(TestPhase::Build)
        .unwrap_or_else(|_| panic!("semantic phase completion must fit the frozen catalog"));
    let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 2))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let leaf = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(11), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("canonical leaf must retain"));
    let canonical_root = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical root must retain"));
    let detail = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(7), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence detail must retain"));
    let evidence_root = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Root { detail }, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));

    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(assembly.outcome_evidence_len(), 2);
    assembly
        .try_commit_canonical(TestDimension::Bytes, 5)
        .unwrap_or_else(|_| panic!("canonical commitment must fit"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 6)
        .unwrap_or_else(|_| panic!("evidence commitment must fit"));

    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            let Some(TestCanonicalBlock::Root { leaf }) = session.canonical(canonical_root) else {
                return false;
            };
            if !matches!(session.canonical(*leaf), Some(TestCanonicalBlock::Leaf(11))) {
                return false;
            }
            // A DAG may reach the same slot more than once; only unique members are counted.
            if !matches!(session.canonical(*leaf), Some(TestCanonicalBlock::Leaf(11))) {
                return false;
            }
            let Some(TestEvidenceBlock::Root { detail }) = session.outcome_evidence(evidence_root)
            else {
                return false;
            };
            matches!(
                session.outcome_evidence(*detail),
                Some(TestEvidenceBlock::Detail(7))
            )
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("dual publication graph must validate"),
    };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("dual publication must prepare"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::Success {
            canonical,
            outcome_evidence,
            ledger_evidence,
        } => {
            let header = ledger_evidence
                .header()
                .expect("success terminal ledger header");
            assert_eq!(header.terminal_cause(), TerminalEvidenceCause::Success);
            assert_eq!(header.last_completed_phase(), Some(TestPhase::Build));
            assert_eq!(header.dimension_count(), 1);
            assert_eq!(header.phase_count(), 1);
            assert!(matches!(canonical.root(), TestCanonicalBlock::Root { .. }));
            assert!(matches!(
                outcome_evidence.root(),
                TestEvidenceBlock::Root { .. }
            ));
            let snapshot = ledger_evidence
                .dimension(TestDimension::Bytes)
                .expect("published dimension evidence");
            assert_eq!(snapshot.committed_canonical_output(), 5);
            assert_eq!(snapshot.published_outcome_evidence(), 7);
        }
        _ => panic!("success must not publish a failure owner"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_joint_commit_intent_keeps_publication_uncommitted_until_qualification() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let validated = validated_scalar_success_on_account(&account);

    let intent = match validated
        .try_form_joint_commit_intent(final_progress())
        .unwrap_or_else(|_| panic!("valid success must form a joint-commit intent"))
    {
        RetainedSuccessJointCommitFormation::Intent(intent) => intent,
        RetainedSuccessJointCommitFormation::PreparedFailure(_) => {
            panic!("healthy success must not downgrade during intent formation")
        }
    };
    let dimension = intent
        .test_dimension_evidence(TestDimension::Bytes)
        .expect("intent must retain the detached dimension state");

    assert_eq!(dimension.pending_publication(), 0);
    assert_eq!(dimension.committed(), 0);
    assert_eq!(dimension.committed_canonical_output(), 0);
    assert_eq!(dimension.published_outcome_evidence(), 0);
    assert!(dimension.current_live_bytes() > 0);
    assert!(intent.test_current_live_bytes() > 0);
    assert!(intent.test_active_reservations() > 0);
    assert!(intent.test_cover_matches_state());

    let prepared = intent
        .qualify_and_commit()
        .unwrap_or_else(|_| panic!("healthy intent must jointly commit"));
    let (canonical, evidence, header, published) = scalar_success_signature(prepared);
    assert_eq!(canonical, 11);
    assert_eq!(evidence, 7);
    assert_eq!(header.terminal_cause(), TerminalEvidenceCause::Success);
    assert_eq!(header.last_completed_phase(), Some(TestPhase::Build));
    assert_eq!(header.dimension_count(), 1);
    assert_eq!(header.phase_count(), 1);
    assert_eq!(published.pending_publication(), 0);
    assert_eq!(published.committed(), 5);
    assert_eq!(published.committed_canonical_output(), 5);
    assert_eq!(published.published_outcome_evidence(), 7);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

fn assert_freeze_destination_rejected(
    account: &PhysicalStorageAccount,
    committed_limit: u64,
    evidence_limit: u64,
    evidence_members: usize,
    expected_class: CounterClass,
    expected_value: u64,
    expected_limit: u64,
) {
    match validated_freeze_success_with_limits(
        account,
        committed_limit,
        evidence_limit,
        evidence_members,
    )
    .test_probe_freeze_permit_qualification(TestDimension::Bytes, TestDimension::Bytes)
    {
        FreezePermitQualificationProbe::Ready => {
            panic!("destination-ineligible exact pair must not qualify")
        }
        FreezePermitQualificationProbe::Rejected {
            cause:
                CommitmentPairPushError::Ledger(LedgerCause::LimitExceeded {
                    class,
                    dimension,
                    value,
                    limit,
                }),
            canonical_commitments,
            outcome_evidence_commitments,
            dimension: snapshot,
        } => {
            assert_eq!(class, expected_class);
            assert_eq!(dimension, TestDimension::Bytes);
            assert_eq!(value, expected_value);
            assert_eq!(limit, expected_limit);
            assert_eq!(canonical_commitments, 0);
            assert_eq!(outcome_evidence_commitments, 0);
            assert_eq!(snapshot.pending_publication(), 0);
            assert_eq!(snapshot.committed(), 0);
            assert_eq!(snapshot.committed_canonical_output(), 0);
            assert_eq!(snapshot.published_outcome_evidence(), 0);
        }
        FreezePermitQualificationProbe::Rejected { .. } => {
            panic!("destination rejection must preserve its exact typed limit cause")
        }
    }

    let issue = validated_freeze_success_with_limits(
        account,
        committed_limit,
        evidence_limit,
        evidence_members,
    )
    .try_issue_freeze_permit(TestDimension::Bytes, TestDimension::Bytes, final_progress())
    .unwrap_or_else(|_| panic!("destination rejection must retain the ordinary failure owner"));
    assert!(!account.snapshot().poisoned());
    match issue {
        RetainedSuccessFreezePermitIssue::DestinationRejected { failure, cause } => {
            assert_eq!(cause.class(), PreparationFailureClass::Ledger);
            assert!(failure.canonical_ownership_is_absent());
            assert_eq!(failure.outcome_evidence_commitment_len(), 0);
            drop(failure);
        }
        RetainedSuccessFreezePermitIssue::Ready(_) => {
            panic!("destination-ineligible exact pair must not issue a ready permit")
        }
        RetainedSuccessFreezePermitIssue::RichFailure { .. } => {
            panic!("zero-commit destination rejection must not masquerade as frozen failure")
        }
    }
    assert!(!account.snapshot().poisoned());
}

#[test]
fn freeze_pair_prequalification_rejects_zero_committed_limit_without_mutation() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    assert_freeze_destination_rejected(&account, 0, 64, 1, CounterClass::Committed, 1, 0);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn freeze_pair_prequalification_rejects_insufficient_evidence_limit_without_mutation() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    assert_freeze_destination_rejected(
        &account,
        u64::MAX,
        2,
        2,
        CounterClass::PublishedOutcomeEvidence,
        3,
        2,
    );

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn freeze_permit_issues_ready_for_the_exact_qualified_pair() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let validated = validated_freeze_success_with_limits(&account, 1, 2, 1);

    let permit = match validated
        .try_issue_freeze_permit(TestDimension::Bytes, TestDimension::Bytes, final_progress())
        .unwrap_or_else(|_| panic!("qualified exact pair must retain the freeze owner"))
    {
        RetainedSuccessFreezePermitIssue::Ready(permit) => permit,
        RetainedSuccessFreezePermitIssue::RichFailure { .. } => {
            panic!("qualified exact pair must issue a ready freeze permit")
        }
        RetainedSuccessFreezePermitIssue::DestinationRejected { .. } => {
            panic!("qualified exact pair must not reject its destinations")
        }
    };
    let snapshot = permit
        .terminal_observation(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ready freeze permit must expose the ledger observation"));
    assert_eq!(snapshot.pending_publication(), 0);
    assert_eq!(snapshot.committed(), 0);
    assert_eq!(snapshot.committed_canonical_output(), 0);
    assert_eq!(snapshot.published_outcome_evidence(), 0);
    assert_eq!(permit.validated.assembly.canonical_commitment_len(), 1);
    assert_eq!(
        permit.validated.assembly.outcome_evidence_commitment_len(),
        1
    );

    drop(permit);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_joint_commit_intent_matches_the_composed_prepare_result_exactly() {
    let old_factory = account_factory(1);
    let old_account = account(&old_factory, u64::MAX, 128);
    let old = validated_scalar_success_on_account(&old_account)
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("legacy preparation must succeed"));
    let old_signature = scalar_success_signature(old);

    let intent_factory = account_factory(1);
    let intent_account = account(&intent_factory, u64::MAX, 128);
    let intent = match validated_scalar_success_on_account(&intent_account)
        .try_form_joint_commit_intent(final_progress())
        .unwrap_or_else(|_| panic!("success must form a joint-commit intent"))
    {
        RetainedSuccessJointCommitFormation::Intent(intent) => intent,
        RetainedSuccessJointCommitFormation::PreparedFailure(_) => {
            panic!("healthy success must not downgrade during intent formation")
        }
    };
    let intent_signature = scalar_success_signature(
        intent
            .qualify_and_commit()
            .unwrap_or_else(|_| panic!("joint-commit intent must qualify")),
    );

    assert_eq!(intent_signature, old_signature);
    drop(old_account);
    drop(intent_account);
    assert_eq!(old_factory.snapshot().live_accounts(), 0);
    assert_eq!(intent_factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_intent_formation_preserves_a_control_downgrade_as_prepared_failure() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let control = OperationControl::default().with_progress_observer(
        |_event: crate::ProgressEvent<TestProgress, TestUnit>| crate::ProgressDecision::Cancel,
    );
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        control,
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("control-downgrade publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("control-downgrade publication must validate"),
        };

    let prepared = match validated
        .try_form_joint_commit_intent(final_progress())
        .unwrap_or_else(|_| panic!("control downgrade must retain a terminal owner"))
    {
        RetainedSuccessJointCommitFormation::PreparedFailure(prepared) => prepared,
        RetainedSuccessJointCommitFormation::Intent(_) => {
            panic!("cancelled final progress must not form a success intent")
        }
    };
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence,
            ledger_evidence,
        } => {
            assert_eq!(
                published_failure_cause(&ledger_evidence).class(),
                PreparationFailureClass::Control
            );
            assert_eq!(*outcome_evidence.outcome_evidence().root(), 7);
        }
        _ => panic!("control downgrade must preserve the exact rich-failure owner"),
    }
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn dropping_success_joint_commit_intent_fails_closed_before_eligibility() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let intent = match validated_scalar_success_on_account(&account)
        .try_form_joint_commit_intent(final_progress())
        .unwrap_or_else(|_| panic!("success must form a joint-commit intent"))
    {
        RetainedSuccessJointCommitFormation::Intent(intent) => intent,
        RetainedSuccessJointCommitFormation::PreparedFailure(_) => {
            panic!("healthy success must not downgrade during intent formation")
        }
    };
    let before = account.snapshot();
    assert!(!before.poisoned());
    assert!(before.live_retained_bytes() > 0);

    drop(intent);

    let after = account.snapshot();
    assert!(after.poisoned());
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert_eq!(
        after
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn poisoned_account_rejects_success_joint_commit_intent_without_prepared_owner() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let intent = match validated_scalar_success_on_account(&account)
        .try_form_joint_commit_intent(final_progress())
        .unwrap_or_else(|_| panic!("success must form a joint-commit intent"))
    {
        RetainedSuccessJointCommitFormation::Intent(intent) => intent,
        RetainedSuccessJointCommitFormation::PreparedFailure(_) => {
            panic!("healthy success must not downgrade during intent formation")
        }
    };
    let first_poison = account.test_record_poison(AccountPoisonReason::OwnershipGraphUnverified);

    let cause = match intent.qualify_and_commit() {
        Err(cause) => cause,
        Ok(_) => panic!("poisoned eligibility must not produce prepared success"),
    };

    assert_eq!(
        cause.reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_eq!(
        cause.account_poisoned().snapshot().first_poison_evidence(),
        first_poison.snapshot().first_poison_evidence()
    );
    assert!(account.snapshot().live_retained_bytes() > 0);
    assert!(account.snapshot().live_retained_allocations() > 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn terminal_evidence_qualification_error_contains_the_complete_success_graph() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("freeze-error publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("freeze-error graph must validate"),
    };
    let physical_before = account.snapshot();
    let ledger_before = (ledger.active_reservations(), ledger.current_live_bytes());

    fail_next_terminal_evidence_qualification();
    let cause = match validated.try_prepare(final_progress()) {
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause)) => cause,
        Ok(_) => panic!("an unfrozen terminal evidence graph must never publish"),
    };
    assert_eq!(
        cause.reason(),
        AccountPoisonReason::TerminalEvidenceFreezeUnverified
    );
    let contained = account.snapshot();
    assert_eq!(
        contained.live_retained_bytes(),
        physical_before.live_retained_bytes()
    );
    assert_eq!(
        contained.live_retained_allocations(),
        physical_before.live_retained_allocations()
    );
    assert_eq!(
        contained
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
    );
    assert!(ledger.is_terminal_qualified());
    assert_eq!(ledger.active_reservations(), ledger_before.0);
    assert_eq!(ledger.current_live_bytes(), ledger_before.1);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn success_guard_direct_drop_contains_physical_before_retaining_ledger_proof() {
    for stage in [
        SuccessGuardDropStage::Qualified,
        SuccessGuardDropStage::Verified,
        SuccessGuardDropStage::PublicationReady,
        SuccessGuardDropStage::EligibilityReady,
    ] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let ledger = operation.coordinator.retained_ledger().test_probe();
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("direct-drop publication must open"));
        let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
        let validated = match assembly.validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                session.canonical(canonical) == Some(&11)
                    && session.outcome_evidence(evidence) == Some(&7)
            },
        ) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("direct-drop graph must validate"),
        };
        let physical_before = account.snapshot();
        let ledger_before = (ledger.active_reservations(), ledger.current_live_bytes());

        drop_next_success_guard(stage);
        let unwind = catch_unwind(AssertUnwindSafe(|| {
            let _ = validated.try_prepare(final_progress());
        }));
        assert!(
            unwind.is_err(),
            "{stage:?} guard must be dropped by the probe"
        );

        let physical_after = account.snapshot();
        let ledger_after = (ledger.active_reservations(), ledger.current_live_bytes());
        assert_eq!(
            physical_after
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::OwnershipGraphUnverified),
            "{stage:?} must poison the physical graph before retaining proof"
        );
        match stage {
            SuccessGuardDropStage::Qualified => {
                assert!(ledger.is_terminal_qualified());
                assert_eq!(
                    physical_after.live_retained_bytes(),
                    physical_before.live_retained_bytes()
                );
                assert_eq!(
                    physical_after.live_retained_allocations(),
                    physical_before.live_retained_allocations()
                );
                assert_eq!(ledger_after, ledger_before);
            }
            SuccessGuardDropStage::Verified => {
                assert!(ledger.is_terminal_qualified());
                assert!(
                    physical_after.live_retained_bytes() < physical_before.live_retained_bytes()
                );
                assert!(
                    physical_after.live_retained_allocations()
                        < physical_before.live_retained_allocations()
                );
                assert_eq!(ledger_after, ledger_before);
            }
            SuccessGuardDropStage::PublicationReady => {
                assert!(ledger.is_terminal_qualified());
                assert!(
                    physical_after.live_retained_bytes() < physical_before.live_retained_bytes()
                );
                assert!(
                    physical_after.live_retained_allocations()
                        < physical_before.live_retained_allocations()
                );
                assert!(ledger_after.0 < ledger_before.0);
                assert!(ledger_after.1 < ledger_before.1);
            }
            SuccessGuardDropStage::EligibilityReady => {
                assert!(!ledger.is_available());
                assert!(
                    physical_after.live_retained_bytes() < physical_before.live_retained_bytes()
                );
                assert!(
                    physical_after.live_retained_allocations()
                        < physical_before.live_retained_allocations()
                );
                assert_eq!(ledger_after, (0, 0));
            }
            SuccessGuardDropStage::VerifiedInProgress | SuccessGuardDropStage::LedgerDetached => {
                unreachable!("mid-transition stages are covered by unwind probes")
            }
        }
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }
}

#[test]
fn success_acknowledgement_panic_does_not_retry_or_release_unacknowledged_ledger_state() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("acknowledgement-panic publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("acknowledgement-panic graph must validate"),
    };
    let physical_before = account.snapshot();
    let ledger_before = (ledger.active_reservations(), ledger.current_live_bytes());

    drop_next_success_guard(SuccessGuardDropStage::VerifiedInProgress);
    let unwind = catch_unwind(AssertUnwindSafe(|| {
        let _ = validated.try_prepare(final_progress());
    }));
    assert!(unwind.is_err());

    let physical_after = account.snapshot();
    assert!(ledger.is_terminal_qualified());
    assert_eq!(
        (ledger.active_reservations(), ledger.current_live_bytes()),
        ledger_before,
        "an in-progress acknowledgement panic must retain, not retry, the exact proof"
    );
    assert!(physical_after.live_retained_bytes() < physical_before.live_retained_bytes());
    assert!(
        physical_after.live_retained_allocations() < physical_before.live_retained_allocations()
    );
    assert_eq!(
        physical_after
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn success_ledger_detach_panic_before_evidence_freeze_uses_scopeguard_containment() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("ledger-detached publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("ledger-detached graph must validate"),
    };

    drop_next_success_guard(SuccessGuardDropStage::LedgerDetached);
    let unwind = catch_unwind(AssertUnwindSafe(|| {
        let _ = validated.try_prepare(final_progress());
    }));
    assert!(unwind.is_err());

    let physical_after = account.snapshot();
    assert!(!ledger.is_available());
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(
        physical_after
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified),
        "scopeguard must contain the physical graph when evidence freeze never starts"
    );
    assert!(physical_after.live_retained_bytes() > 0);
    assert!(physical_after.live_retained_allocations() > 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn terminal_evidence_qualification_panic_contains_the_complete_fallback_graph() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let physical_before = account.snapshot();
    let ledger_before = (ledger.active_reservations(), ledger.current_live_bytes());

    panic_next_terminal_evidence_qualification();
    let cause =
        match operation.prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(93)) {
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause)) => cause,
            Ok(_) => panic!("a panicked terminal evidence freeze must never publish fallback"),
        };
    assert_eq!(
        cause.reason(),
        AccountPoisonReason::TerminalEvidenceFreezeUnverified
    );
    let contained = account.snapshot();
    assert_eq!(
        contained.live_retained_bytes(),
        physical_before.live_retained_bytes()
    );
    assert_eq!(
        contained.live_retained_allocations(),
        physical_before.live_retained_allocations()
    );
    assert_eq!(
        contained
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
    );
    assert!(ledger.is_terminal_qualified());
    assert_eq!(ledger.active_reservations(), ledger_before.0);
    assert_eq!(ledger.current_live_bytes(), ledger_before.1);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn inherited_blocks_deduplicate_source_slots_without_new_attribution_or_physical_charge() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source_canonical, source_canonical_slot, source_evidence, source_evidence_slot) =
        publish_scalar_source(&account);
    let canonical_shares_before = source_canonical
        .aggregate()
        .inner
        .test_member_share_call_count(source_canonical_slot.identity)
        .expect("canonical source slot must resolve");
    let evidence_shares_before = source_evidence
        .aggregate()
        .inner
        .test_member_share_call_count(source_evidence_slot.identity)
        .expect("evidence source slot must resolve");

    let operation = test_derived_operation_on_account_with_control(
        &account,
        &source_canonical,
        64,
        RetainedBatchLimits::new(3, 2),
        OperationControl::default(),
    );
    let plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer_plan(2, 2), 3, 1)
            .with_canonical_source_mapping_capacity(source_canonical.aggregate().len()),
        RetainedBlockTablePlan::new(writer_plan(1, 2), 3, 0),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("inherited publication must open"));
    assembly
        .try_bind_canonical_source(source_canonical.aggregate(), TestDimension::Bytes)
        .unwrap_or_else(|_| panic!("canonical source map must bind"));
    assert_eq!(
        assembly.test_take_canonical_source_mapping_probe_count(),
        0,
        "binding initializes the dense map without lookup probes"
    );
    let physical_before_shares = account.snapshot();

    let canonical_root = assembly
        .try_share_canonical(source_canonical.aggregate(), source_canonical_slot)
        .unwrap_or_else(|_| panic!("first canonical share must succeed"));
    assert_eq!(
        assembly.test_take_canonical_source_mapping_probe_count(),
        1,
        "a first source lookup performs one dense-map probe"
    );
    let canonical_second = assembly
        .try_share_canonical(source_canonical.aggregate(), source_canonical_slot)
        .unwrap_or_else(|_| panic!("second canonical share must succeed"));
    assert_eq!(
        assembly.test_take_canonical_source_mapping_probe_count(),
        1,
        "a duplicate source lookup remains one dense-map probe"
    );
    let evidence_root = assembly
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("first evidence share must succeed"));
    let evidence_second = assembly
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("second evidence share must succeed"));
    let evidence_third = assembly
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("third evidence share must succeed"));

    assert_eq!(account.snapshot(), physical_before_shares);
    assert_eq!(canonical_second, canonical_root);
    assert_eq!(assembly.canonical_len(), 1);
    assert_eq!(assembly.outcome_evidence_len(), 3);
    assert!(
        assembly.test_storage_graph_is_exact(),
        "inherited-only incremental proofs must match the full structural audit"
    );
    assert_eq!(
        assembly
            .canonical_batch
            .as_ref()
            .expect("canonical attribution batch")
            .attribution_len(),
        1
    );
    assert_eq!(
        assembly
            .evidence_batch
            .as_ref()
            .expect("evidence attribution batch")
            .attribution_len(),
        1
    );
    assert_eq!(
        source_canonical
            .aggregate()
            .inner
            .test_member_share_call_count(source_canonical_slot.identity)
            .expect("canonical source slot must remain observable"),
        canonical_shares_before + 1
    );
    assert_eq!(
        source_evidence
            .aggregate()
            .inner
            .test_member_share_call_count(source_evidence_slot.identity)
            .expect("evidence source slot must remain observable"),
        evidence_shares_before + 3
    );
    assert!(source_canonical.aggregate().get(canonical_root).is_none());
    assert!(source_evidence.aggregate().get(evidence_root).is_none());

    let canonical_new = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(13, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("one new canonical block must fit"));
    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(
        assembly
            .canonical_batch
            .as_ref()
            .expect("canonical attribution batch")
            .attribution_len(),
        2
    );
    assert!(
        account.snapshot().live_retained_bytes() > physical_before_shares.live_retained_bytes()
    );
    assert!(
        assembly.test_storage_graph_is_exact(),
        "mixed inherited/new incremental proofs must match the full structural audit"
    );

    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            session.canonical(canonical_root) == Some(&11)
                && session.canonical(canonical_second) == Some(&11)
                && session.canonical(canonical_new) == Some(&13)
                && session.outcome_evidence(evidence_root) == Some(&7)
                && session.outcome_evidence(evidence_second) == Some(&7)
                && session.outcome_evidence(evidence_third) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("inherited publication must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("inherited publication must prepare")),
    );
    match &published {
        PublishedRetainedPublicationParts::Success {
            canonical,
            outcome_evidence,
            ..
        } => {
            assert_eq!(canonical.aggregate().get(canonical_root), Some(&11));
            assert!(canonical.aggregate().get(source_canonical_slot).is_none());
            assert_eq!(outcome_evidence.aggregate().get(evidence_root), Some(&7));
            assert!(
                outcome_evidence
                    .aggregate()
                    .get(source_evidence_slot)
                    .is_none()
            );
        }
        _ => panic!("inherited publication must succeed"),
    }
    drop(published);
    assert!(
        !account.snapshot().poisoned(),
        "normal inherited publication release must not poison the physical account"
    );
    drop(source_canonical);
    drop(source_evidence);
    drop(account);
    assert_eq!(
        factory.snapshot().live_accounts(),
        0,
        "normal inherited publication ownership must release every account reference"
    );
}

#[test]
fn dense_canonical_source_lookup_is_constant_and_physically_accounted() {
    const SOURCE_BLOCKS: usize = 65;
    const REPEATED_LOOKUPS: u64 = 64;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 512);
    let source_operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(SOURCE_BLOCKS + 1, 2),
        OperationControl::default(),
    );
    let mut source: TestPublication<u64, u64> = source_operation
        .try_publication_assembly(publication_plan(SOURCE_BLOCKS, 1))
        .unwrap_or_else(|_| panic!("dense source publication must open"));
    let mut source_slots = Vec::with_capacity(SOURCE_BLOCKS);
    for value in 0..SOURCE_BLOCKS {
        let value = u64::try_from(value).expect("test source ordinal fits u64");
        let slot = source
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(value, CanonicalLeafTag))
            })
            .unwrap_or_else(|_| panic!("dense source member must retain"));
        source_slots.push(slot);
    }
    let evidence = source
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("dense source evidence must retain"));
    let source_root = source_slots[SOURCE_BLOCKS - 1];
    let source = match source.validate_success(source_root, evidence, |session, _, evidence| {
        source_slots
            .iter()
            .enumerate()
            .all(|(index, slot)| session.canonical(*slot).copied() == u64::try_from(index).ok())
            && session.outcome_evidence(evidence) == Some(&7)
    }) {
        RetainedSuccessValidation::Valid(validated) => handoff_for_test(
            validated
                .try_prepare(final_progress())
                .unwrap_or_else(|_| panic!("dense source publication must prepare")),
        ),
        _ => panic!("dense source publication must validate"),
    };
    let (source_canonical, source_evidence) = match source {
        PublishedRetainedPublicationParts::Success {
            canonical,
            outcome_evidence,
            ledger_evidence,
        } => {
            drop(ledger_evidence);
            (canonical, outcome_evidence)
        }
        _ => panic!("dense source publication must succeed"),
    };

    let undersized_operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(1, 2),
        OperationControl::default(),
    );
    let undersized_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer_plan(1, 2), 1, 0)
            .with_canonical_source_mapping_capacity(SOURCE_BLOCKS - 1),
        RetainedBlockTablePlan::new(writer_plan(1, 2), 1, 0),
    );
    let mut undersized: TestPublication<u64, u64> = undersized_operation
        .try_publication_assembly(undersized_plan)
        .unwrap_or_else(|_| panic!("undersized dense-map target must open"));
    assert!(matches!(
        undersized.try_bind_canonical_source(source_canonical.aggregate(), TestDimension::Bytes,),
        Err(CanonicalSourceBindError::Publication(
            RetainedPublicationError::InvalidSourceSlot
        ))
    ));
    assert_eq!(
        undersized.test_take_canonical_source_mapping_probe_count(),
        0
    );
    assert_eq!(undersized.canonical_len(), 0);
    drop(undersized);

    let target_operation = test_derived_operation_on_account_with_control(
        &account,
        &source_canonical,
        64,
        RetainedBatchLimits::new(1, 2),
        OperationControl::default(),
    );
    let target_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer_plan(1, 2), 1, 0)
            .with_canonical_source_mapping_capacity(SOURCE_BLOCKS),
        RetainedBlockTablePlan::new(writer_plan(1, 2), 1, 0),
    );
    let before_target = account.snapshot();
    let mut target: TestPublication<u64, u64> = target_operation
        .try_publication_assembly(target_plan)
        .unwrap_or_else(|_| panic!("dense-map target publication must open"));
    let after_target = account.snapshot();
    assert_eq!(
        after_target
            .live_retained_allocations()
            .checked_sub(before_target.live_retained_allocations())
            .expect("dense-map target allocation count grows"),
        5,
        "the canonical map is one physically accounted nested table allocation"
    );
    target
        .try_bind_canonical_source(source_canonical.aggregate(), TestDimension::Bytes)
        .unwrap_or_else(|_| panic!("dense source map must bind"));
    assert_eq!(target.test_take_canonical_source_mapping_probe_count(), 0);

    let source_slot = source_slots[SOURCE_BLOCKS - 2];
    let destination = target
        .try_share_canonical(source_canonical.aggregate(), source_slot)
        .unwrap_or_else(|_| panic!("far source slot must share"));
    assert_eq!(target.test_take_canonical_source_mapping_probe_count(), 1);
    for _ in 0..REPEATED_LOOKUPS {
        assert_eq!(
            target
                .try_share_canonical(source_canonical.aggregate(), source_slot)
                .unwrap_or_else(|_| panic!("duplicate dense-map lookup must share")),
            destination
        );
    }
    assert_eq!(
        target.test_take_canonical_source_mapping_probe_count(),
        REPEATED_LOOKUPS,
        "each lookup probes exactly one dense-map entry regardless of source size"
    );
    assert_eq!(target.canonical_len(), 1);

    drop(target);
    assert!(account.snapshot().live_retained_bytes() < before_target.live_retained_bytes());
    assert!(
        account.snapshot().live_retained_allocations() < before_target.live_retained_allocations()
    );
    drop(source_canonical);
    drop(source_evidence);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_source_binding_checks_account_health_before_mapping_work() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source_canonical, _, source_evidence, _) = publish_scalar_source(&account);
    let (wrong_canonical, _, wrong_evidence, _) = publish_scalar_source(&account);
    let operation = test_derived_operation_on_account_with_control(
        &account,
        &source_canonical,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan_with_canonical_source_mapping(1, 1, 1))
        .unwrap_or_else(|_| panic!("poison-first binding target must open"));
    let first_poison = account.test_record_poison(AccountPoisonReason::OwnershipGraphUnverified);

    assert!(matches!(
        assembly.try_bind_canonical_source(wrong_canonical.aggregate(), TestDimension::Bytes),
        Err(CanonicalSourceBindError::Publication(
            RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(cause))
        )) if cause == first_poison
    ));
    assert_eq!(assembly.test_take_canonical_source_mapping_probe_count(), 0);
    assert_eq!(assembly.canonical_len(), 0);

    drop(assembly);
    drop(source_canonical);
    drop(source_evidence);
    drop(wrong_canonical);
    drop(wrong_evidence);
    drop(account);
    assert_eq!(
        factory.snapshot().live_accounts(),
        1,
        "an explicitly poisoned account remains quarantined after all handles are dropped"
    );
}

#[test]
fn poisoned_derived_start_precedes_foreign_source_validation() {
    let factory = account_factory(2);
    let target_account = account(&factory, u64::MAX, 128);
    let foreign_account = account(&factory, u64::MAX, 128);
    let (foreign_canonical, _, foreign_evidence, _) = publish_scalar_source(&foreign_account);
    let first_poison =
        target_account.test_record_poison(AccountPoisonReason::OwnershipGraphUnverified);
    let target_before = target_account.snapshot();
    let foreign_before = foreign_account.snapshot();
    let fallback_called = StdArc::new(AtomicBool::new(false));
    let fallback_observer = StdArc::clone(&fallback_called);
    let (limits, plan) = startup_limits_and_plan();

    let result = RetainedOperation::<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        u32,
    >::try_start_derived_with_phase_catalog(
        OperationControl::default(),
        limits,
        &[TestPhase::Build],
        &target_account,
        foreign_canonical.aggregate(),
        plan,
        move || {
            fallback_observer.store(true, Ordering::Relaxed);
            Ok(7)
        },
    );

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::AccountPoisoned(cause)
        )) if cause == first_poison
    ));
    assert!(!fallback_called.load(Ordering::Relaxed));
    assert_eq!(target_account.snapshot(), target_before);
    assert_eq!(foreign_account.snapshot(), foreign_before);

    drop(foreign_canonical);
    drop(foreign_evidence);
    drop(target_account);
    drop(foreign_account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_source_length_corruption_poisons_the_exact_source_account() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source_canonical, _, source_evidence, _) = publish_scalar_source(&account);
    let aggregate = source_canonical.aggregate();
    let corrupt_length = aggregate
        .len()
        .checked_add(1)
        .expect("test source length must have a larger representable value");
    let authority = CanonicalLineageAuthority::Derived {
        source_set: aggregate.set_id(),
        source_len: corrupt_length,
    };

    let error = authority
        .require_source::<TestDimension, _>(&account, aggregate)
        .expect_err("same-set length corruption must fail closed");
    assert!(matches!(
        error,
        RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(_))
    ));
    assert_eq!(
        account
            .snapshot()
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );

    drop(source_canonical);
    drop(source_evidence);
    drop(account);
    assert_eq!(
        factory.snapshot().live_accounts(),
        0,
        "without a live publication owner, the poisoned account can release its factory lease"
    );
}

#[test]
fn non_authoritative_same_account_aggregate_is_rejected_before_member_access_or_reference_clone() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source_canonical, source_slot, source_evidence, _) = publish_scalar_source(&account);
    let (bound_canonical, _, bound_evidence, _) = publish_scalar_source(&account);
    let table_shares_before = source_canonical
        .aggregate()
        .inner
        .test_table_share_call_count();
    let block_shares_before = source_canonical
        .aggregate()
        .inner
        .test_member_share_call_count(source_slot.identity)
        .expect("source slot must resolve");
    let _ = take_test_member_lookups();

    let operation = test_derived_operation_on_account_with_control(
        &account,
        &bound_canonical,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("exact-source target must open"));
    let result = assembly.try_share_canonical(source_canonical.aggregate(), source_slot);

    assert!(matches!(
        result,
        Err(RetainedPublicationError::InvalidSourceSlot)
    ));
    assert_eq!(take_test_member_lookups(), 0);
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(
        source_canonical
            .aggregate()
            .inner
            .test_table_share_call_count(),
        table_shares_before
    );
    assert_eq!(
        source_canonical
            .aggregate()
            .inner
            .test_member_share_call_count(source_slot.identity)
            .expect("source slot must remain observable"),
        block_shares_before
    );
    drop(handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("source rejection must preserve fallback")),
    ));
    drop(source_canonical);
    drop(source_evidence);
    drop(bound_canonical);
    drop(bound_evidence);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn share_work_rejection_does_not_clone_or_mutate_the_target_table() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source_canonical, source_slot, source_evidence, _) = publish_scalar_source(&account);
    let block_shares_before = source_canonical
        .aggregate()
        .inner
        .test_member_share_call_count(source_slot.identity)
        .expect("source slot must resolve");
    let physical_before_operation = account.snapshot();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, 16, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let start = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation = TestOperation::try_start_derived_with_phase_catalog(
        OperationControl::default(),
        limits,
        &[TestPhase::Build],
        &account,
        source_canonical.aggregate(),
        start,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("exact share-work operation must start"));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan_with_canonical_source_mapping(1, 1, 1))
        .unwrap_or_else(|_| panic!("exact share-work tables must open"));
    assembly
        .try_bind_canonical_source(source_canonical.aggregate(), TestDimension::Bytes)
        .unwrap_or_else(|_| panic!("canonical source map must bind before share work"));
    let physical_before_share = account.snapshot();
    let work_before_share = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("share work dimension must remain observable"));
    assert_eq!(work_before_share.consumed(), 15);

    let result = assembly.try_share_canonical(source_canonical.aggregate(), source_slot);

    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: TestDimension::Bytes,
                value: 17,
                limit: 16,
            })
        ))
    ));
    let work_after_share = assembly
        .test_ledger_probe()
        .dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("rejected share work evidence must remain observable"));
    assert_eq!(
        work_after_share.attempted(),
        work_before_share.attempted() + 2
    );
    assert_eq!(
        work_after_share.consumed(),
        work_before_share.consumed(),
        "batch admission rejection must not report the first share unit as completed"
    );
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(account.snapshot(), physical_before_share);
    assert!(
        physical_before_share.live_retained_bytes()
            > physical_before_operation.live_retained_bytes()
    );
    assert_eq!(
        source_canonical
            .aggregate()
            .inner
            .test_member_share_call_count(source_slot.identity)
            .expect("source slot must remain observable"),
        block_shares_before
    );
    drop(handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("share-work rejection must preserve fallback")),
    ));
    drop(source_canonical);
    drop(source_evidence);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn invalid_publication_capacity_is_rejected_before_every_open_side_effect() {
    let factory = account_factory(1);
    let invalid_plans = [
        (
            "attribution capacity overflow",
            RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(usize::MAX, 1),
                1,
                0,
            )),
            false,
        ),
        (
            "commitment capacity overflow",
            RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(1, usize::MAX),
                1,
                0,
            )),
            false,
        ),
        (
            "member capacity overflow",
            RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(1, 1),
                usize::MAX,
                0,
            )),
            true,
        ),
        (
            "new block limit exceeds total blocks",
            RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(2, 1),
                1,
                2,
            )),
            false,
        ),
    ];

    for (index, (case, invalid_plan, expects_batch_capacity)) in
        invalid_plans.into_iter().enumerate()
    {
        let operation = test_operation(&factory, 64);
        let dimension_before = operation
            .coordinator
            .dimension_evidence(TestDimension::Bytes)
            .unwrap_or_else(|_| panic!("{case}: dimension evidence before open"));
        let physical_before = operation.account().snapshot();
        let account_shares_before = operation.account().share_call_count();
        let set_ids_before = test_retained_set_id_requests();
        let batch_allocations_before = crate::ledger::test_attribution_batch_allocation_entries();

        let error = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
            Ok(_) => panic!("{case}: invalid plan unexpectedly opened"),
            Err(error) => error,
        };
        assert!(
            if expects_batch_capacity {
                matches!(error.cause(), RetainedWriterError::BatchCapacity)
            } else {
                matches!(error.cause(), RetainedWriterError::InvalidPublicationPlan)
            },
            "{case}: unexpected preflight cause: {}",
            error.cause()
        );
        let operation = expect_retry_operation(error, case);

        assert_eq!(test_retained_set_id_requests(), set_ids_before, "{case}");
        assert_eq!(
            crate::ledger::test_attribution_batch_allocation_entries(),
            batch_allocations_before,
            "{case}"
        );
        assert_eq!(
            operation.account().share_call_count(),
            account_shares_before,
            "{case}"
        );
        assert_eq!(operation.account().snapshot(), physical_before, "{case}");
        assert_eq!(
            operation
                .coordinator
                .dimension_evidence(TestDimension::Bytes)
                .unwrap_or_else(|_| panic!("{case}: dimension evidence after rejection")),
            dimension_before,
            "{case}"
        );

        if index == 2 {
            let assembly: TestPublication<u64, u64> = operation
                .try_publication_assembly(RetainedPublicationPlan::rich_failure(
                    RetainedBlockTablePlan::new(writer_plan(1, 1), 1, 0),
                ))
                .unwrap_or_else(|_| panic!("{case}: valid retry must open"));
            drop(handoff_for_test(
                assembly
                    .prepare_minimal_fallback()
                    .unwrap_or_else(|_| panic!("{case}: retry fallback must prepare")),
            ));
        } else {
            drop(handoff_for_test(
                operation
                    .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(70))
                    .unwrap_or_else(|_| panic!("direct fallback must prepare")),
            ));
        }
        assert_eq!(factory.snapshot().live_accounts(), 0, "{case}");
    }
}

#[test]
fn recoverable_open_failures_prepare_the_baseline_without_tables_or_roots() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let invalid_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 2),
        RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
    );
    let error = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
        Ok(_) => panic!("invalid total/new relation must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let operation = expect_retry_operation(error, "invalid total/new relation");
    let published = handoff_for_test(
        operation
            .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(71))
            .unwrap_or_else(|_| panic!("direct fallback must prepare")),
    );
    match published {
        PublishedRetainedPublicationParts::MinimalFallback {
            fallback,
            ledger_evidence,
        } => {
            assert_eq!(
                published_failure_cause(&ledger_evidence),
                PreparationFailureCode::domain(71)
            );
            assert_eq!(*fallback.value(), 7);
            assert_eq!(
                ledger_evidence
                    .dimension(TestDimension::Bytes)
                    .expect("direct fallback ledger evidence")
                    .committed_canonical_output(),
                0
            );
        }
        _ => panic!("invalid plan must publish minimal fallback"),
    }

    let operation = test_operation(&factory, 64);
    fail_next_final_owner_allocation();
    let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
        Ok(_) => panic!("injected table allocation failure must reject open"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = expect_retry_operation(error, "verified allocation failure");
    let published = handoff_for_test(
        operation
            .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(72))
            .unwrap_or_else(|_| panic!("direct fallback must prepare")),
    );
    assert!(matches!(
        &published,
        PublishedRetainedPublicationParts::MinimalFallback { ledger_evidence, .. }
            if published_failure_cause(ledger_evidence) == PreparationFailureCode::domain(72)
    ));
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn second_table_allocation_failure_releases_canonical_and_returns_operation() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    fail_final_owner_allocation_after(1);

    let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
        Ok(_) => panic!("evidence table allocation failure must reject open"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = expect_retry_operation(error, "clean canonical rollback");
    let published = handoff_for_test(
        operation
            .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(73))
            .unwrap_or_else(|_| panic!("recovered operation must prepare fallback")),
    );
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn second_table_allocation_failure_reports_canonical_storage_cleanup_failure() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    fail_final_owner_allocation_after(1);
    test_panic_final_owner_drop_after(0);

    let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
        Ok(_) => panic!("combined evidence allocation and canonical drop failure must reject"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::StorageDropUnverified
    ));
    assert!(matches!(
        error.into_recovery(),
        Err(RetainedWriterError::StorageDropUnverified)
    ));
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn semantic_rejection_can_append_evidence_and_prepare_a_rich_failure() {
    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 2))
        .unwrap_or_else(|_| panic!("semantic rejection publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let mut failure = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical, evidence| {
            let _ = session.canonical(canonical);
            let _ = session.outcome_evidence(evidence);
            false
        },
    ) {
        RetainedSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must preserve evidence-building ownership"),
    };
    let detail = failure
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("bounded failure detail must retain"));
    failure
        .try_commit_outcome_evidence(TestDimension::Bytes, 3)
        .unwrap_or_else(|_| panic!("failure evidence commitment must fit"));
    let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
        session.outcome_evidence(root) == Some(&7) && session.outcome_evidence(detail) == Some(&9)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("augmented failure evidence must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(91), final_progress())
            .unwrap_or_else(|_| panic!("rich failure must prepare")),
    );
    match published {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence,
            ledger_evidence,
        } => {
            assert_eq!(
                published_failure_cause(&ledger_evidence),
                PreparationFailureCode::domain(91)
            );
            assert_eq!(outcome_evidence.outcome_evidence().root(), &7);
            assert_eq!(
                outcome_evidence.outcome_evidence().aggregate().get(detail),
                Some(&9)
            );
            assert_eq!(
                ledger_evidence
                    .dimension(TestDimension::Bytes)
                    .expect("rich failure ledger evidence")
                    .committed_canonical_output(),
                0
            );
        }
        _ => panic!("semantic rejection must publish rich failure only"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn validation_panic_transitions_to_fallback_only() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("validation-panic publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let fallback_only = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |_session, _canonical, _evidence| panic!("validator panic"),
    ) {
        RetainedSuccessValidation::FallbackOnly(fallback) => fallback,
        _ => panic!("validator panic must expose only fallback"),
    };
    let published = handoff_for_test(
        fallback_only
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("validation panic fallback must prepare")),
    );
    match &published {
        PublishedRetainedPublicationParts::MinimalFallback {
            ledger_evidence, ..
        } => {
            let cause = published_failure_cause(ledger_evidence);
            assert_eq!(cause.class(), PreparationFailureClass::ProductGraph);
            assert_eq!(cause.code(), 5);
        }
        _ => panic!("validation panic must publish minimal fallback"),
    }
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn repeated_semantic_rejection_preserves_the_product_graph_fallback_cause() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("repeated-rejection publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);

    let failure =
        match assembly
            .validate_success(canonical, evidence, |_session, _canonical, _evidence| false)
        {
            RetainedSuccessValidation::Rejected(failure) => failure,
            _ => panic!("first semantic rejection must expose failure building"),
        };
    let failure = match failure.validate_rich_failure(evidence, |_session, _evidence| false) {
        RetainedFailureValidation::Rejected(failure) => failure,
        _ => panic!("second semantic rejection must preserve failure building"),
    };
    let published = handoff_for_test(
        failure
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("repeated rejection fallback must prepare")),
    );
    match &published {
        PublishedRetainedPublicationParts::MinimalFallback {
            ledger_evidence, ..
        } => {
            let cause = published_failure_cause(ledger_evidence);
            assert_eq!(cause.class(), PreparationFailureClass::ProductGraph);
            assert_eq!(cause.code(), 4);
        }
        _ => panic!("repeated semantic rejection must publish only fallback"),
    }
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn consuming_closure_validation_rejects_orphans_and_retagged_slots() {
    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
    let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let orphan = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("orphan candidate must retain"));
    let root = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf: orphan }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("root must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(9), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence must retain"));

    let rejected = match assembly.validate_success(root, evidence, |session, root, evidence| {
        session.canonical(root).is_some() && session.outcome_evidence(evidence).is_some()
    }) {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        _ => panic!("orphan graph must be rejected"),
    };
    let prepared = rejected
        .prepare_minimal_fallback()
        .unwrap_or_else(|_| panic!("orphan rejection must prepare minimal fallback"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback {
            ledger_evidence, ..
        } => {
            let cause = published_failure_cause(&ledger_evidence);
            assert_eq!(cause.class(), PreparationFailureClass::ProductGraph);
            assert_eq!(cause.code(), 4);
        }
        _ => panic!("orphan rejection must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);

    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
    let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 1))
        .unwrap_or_else(|_| panic!("publication assembly must reopen"));
    let orphan = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("orphan candidate must retain"));
    let root = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf: orphan }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("root must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(9), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence must retain"));
    let retagged: CanonicalRetainedSlot<TestCanonicalBlock, CanonicalRootTag> =
        CanonicalRetainedSlot {
            identity: orphan.identity,
            marker: PhantomData,
        };
    let rejected = match assembly.validate_success(root, evidence, |session, root, evidence| {
        session.canonical(root).is_some()
            && session.canonical(retagged).is_some()
            && session.outcome_evidence(evidence).is_some()
    }) {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        _ => panic!("retagged graph must be rejected"),
    };
    let prepared = rejected
        .prepare_minimal_fallback()
        .unwrap_or_else(|_| panic!("retag rejection must prepare minimal fallback"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback {
            ledger_evidence, ..
        } => {
            let cause = published_failure_cause(&ledger_evidence);
            assert_eq!(cause.class(), PreparationFailureClass::ProductGraph);
            assert_eq!(cause.code(), 4);
        }
        _ => panic!("retag rejection must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn block_limits_reject_before_running_the_builder_and_minimal_fallback_stays_available() {
    static BUILDER_RAN: AtomicBool = AtomicBool::new(false);

    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
    let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let _ = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("first block must fit"));
    BUILDER_RAN.store(false, Ordering::SeqCst);
    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            BUILDER_RAN.store(true, Ordering::SeqCst);
            Ok(emplacer.place(TestCanonicalBlock::Leaf(2), CanonicalLeafTag))
        }),
        Err(RetainedPublicationError::CanonicalSlotLimit)
    ));
    assert!(!BUILDER_RAN.load(Ordering::SeqCst));
    let _ = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(3), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence block must fit"));
    let prepared = assembly
        .prepare_minimal_fallback()
        .unwrap_or_else(|_| panic!("minimal fallback must remain available"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(*fallback.value(), 7);
        }
        _ => panic!("direct fallback must publish the minimal owner"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn partial_canonical_downgrades_to_rich_failure_after_storage_drops_first() {
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
    let mut assembly: TestPublication<LiveDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let ledger = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger();
    let observation = LiveDropObservation {
        ledger: ledger.test_probe(),
        account: assembly
            .account
            .as_ref()
            .expect("assembly retains account")
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted")),
        observed_active: &OBSERVED_ACTIVE,
        observed_operation_live: &OBSERVED_OPERATION_LIVE,
        observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
        observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
    };
    let _partial = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(observation, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("partial canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(5, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));
    OBSERVED_ACTIVE.store(0, Ordering::SeqCst);
    OBSERVED_OPERATION_LIVE.store(0, Ordering::SeqCst);
    OBSERVED_PHYSICAL_BYTES.store(0, Ordering::SeqCst);
    OBSERVED_PHYSICAL_ALLOCATIONS.store(0, Ordering::SeqCst);
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&5)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("partial canonical failure graph must validate"),
    };
    let prepared = validated
        .try_prepare(PreparationFailureCode::domain(9), final_progress())
        .unwrap_or_else(|_| panic!("partial canonical must downgrade to rich failure"));

    assert!(OBSERVED_ACTIVE.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_OPERATION_LIVE.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_PHYSICAL_BYTES.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst) > 0);
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence,
            ledger_evidence,
        } => {
            assert_eq!(*outcome_evidence.outcome_evidence().root(), 5);
            assert_eq!(
                published_failure_cause(&ledger_evidence),
                PreparationFailureCode::domain(9)
            );
        }
        _ => panic!("partial canonical must not publish canonical output"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn failure_only_plan_rejects_canonical_construction_and_publishes_evidence() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("failure-only assembly must open"));
    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        }),
        Err(RetainedPublicationError::WrongOutcomeMode)
    ));
    assert!(matches!(
        assembly.try_commit_canonical(TestDimension::Bytes, 1),
        Err(RetainedPublicationError::WrongOutcomeMode)
    ));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("failure evidence must retain"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("failure-only graph must validate"),
    };
    match handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(3), final_progress())
            .unwrap_or_else(|_| panic!("failure-only publication must prepare")),
    ) {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence, ..
        } => assert_eq!(*outcome_evidence.outcome_evidence().root(), 7),
        _ => panic!("failure-only plan cannot publish canonical output"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_control_rejection_atomically_downgrades_to_rich_failure() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let control = OperationControl::default().with_progress_observer(
        |_event: crate::ProgressEvent<TestProgress, TestUnit>| crate::ProgressDecision::Cancel,
    );
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        control,
    );
    drop(account);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("control rejection candidate must validate"),
        };
    match handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("control rejection must produce a rich failure")),
    ) {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence,
            ledger_evidence,
        } => {
            let cause = published_failure_cause(&ledger_evidence);
            assert_eq!(cause.class(), PreparationFailureClass::Control);
            assert_eq!(*outcome_evidence.outcome_evidence().root(), 7);
        }
        _ => panic!("control rejection must not publish canonical output"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn primary_ledger_rejection_replaces_only_a_provisional_requested_cause() {
    assert_eq!(
        cause_after_primary_ledger_rejection(PreparationFailureCode::domain(73)),
        PreparationFailureCode::ledger()
    );
    for established in [
        PreparationFailureCode::control(2),
        PreparationFailureCode::product_graph(3),
        PreparationFailureCode::ledger(),
    ] {
        assert_eq!(
            cause_after_primary_ledger_rejection(established),
            established
        );
    }
}

#[test]
fn primary_binding_failure_preserves_the_earlier_control_cause() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let control = OperationControl::default().with_progress_observer(
        |_event: crate::ProgressEvent<TestProgress, TestUnit>| crate::ProgressDecision::Cancel,
    );
    let operation = test_operation_on_account_with_control(
        &account,
        4,
        RetainedBatchLimits::new(2, 2),
        control,
    );
    drop(account);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("control rejection publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("evidence commitment must fit the preallocated batch"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("control rejection candidate must validate"),
        };

    match handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("binding rejection must retain minimal fallback")),
    ) {
        PublishedRetainedPublicationParts::MinimalFallback {
            fallback,
            ledger_evidence,
        } => {
            assert_eq!(*fallback.value(), 7);
            assert_eq!(
                published_failure_cause(&ledger_evidence).class(),
                PreparationFailureClass::Control
            );
        }
        _ => panic!("failed evidence binding must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn evidence_primary_rejection_uses_minimal_fallback_without_retry() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("failure-only assembly must open"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("failure evidence must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("evidence rejection candidate must validate"),
    };
    match handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(5), final_progress())
            .unwrap_or_else(|_| panic!("ledger rejection must retain minimal fallback")),
    ) {
        PublishedRetainedPublicationParts::MinimalFallback {
            fallback,
            ledger_evidence,
        } => {
            assert_eq!(*fallback.value(), 7);
            assert_eq!(
                published_failure_cause(&ledger_evidence).class(),
                PreparationFailureClass::Ledger
            );
        }
        _ => panic!("primary ledger rejection must not retry rich failure"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_canonical_primary_rejection_downgrades_to_retained_rich_failure() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    CANONICAL_DROPS.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let start = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation =
        TestOperation::try_start(OperationControl::default(), limits, &account, start, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("canonical rejection operation must start"));
    drop(account);
    let mut assembly: TestPublication<DropCounter, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("canonical rejection assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
    assembly
        .try_commit_canonical(TestDimension::Bytes, 1)
        .unwrap_or_else(|_| panic!("canonical commitment must fit the preallocated batch"));

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("canonical rejection candidate must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("canonical rejection must prepare a rich failure"));
    assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 0);
    let published = handoff_for_test(prepared);
    match &published {
        PublishedRetainedPublicationParts::RichFailure {
            ledger_evidence, ..
        } => {
            assert_eq!(
                published_failure_cause(ledger_evidence).class(),
                PreparationFailureClass::Ledger
            );
        }
        _ => panic!("canonical ledger rejection must retain rich failure evidence"),
    }
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 0);
    drop(published);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn evidence_only_retry_panic_retains_storage_and_attribution_fail_closed() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    CANONICAL_DROPS.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let start = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation =
        TestOperation::try_start(OperationControl::default(), limits, &account, start, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("retry-panic operation must start"));
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let mut assembly: TestPublication<DropCounter, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("retry-panic publication must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("retry-panic canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("retry-panic evidence block must retain"));
    assembly
        .try_commit_canonical(TestDimension::Bytes, 1)
        .unwrap_or_else(|_| panic!("retry-panic canonical commitment must fit"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("retry-panic candidate must validate"),
        };
    let before = account.snapshot();
    let ledger_before = (ledger.active_reservations(), ledger.current_live_bytes());

    panic_next_evidence_only_primary_bind();
    let result = validated.try_prepare(final_progress());

    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::PrimaryPublicationBindingPanicked
    ));
    assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(
        (ledger.active_reservations(), ledger.current_live_bytes()),
        ledger_before
    );
    let after = account.snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert_eq!(
        after.total_admitted_retained_bytes(),
        before.total_admitted_retained_bytes()
    );
    assert_eq!(
        after.total_admitted_retained_allocations(),
        before.total_admitted_retained_allocations()
    );
    assert_eq!(
        after
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::PrimaryPublicationBindingPanicked)
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn success_double_primary_rejection_drops_both_tables_before_minimal_fallback() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    CANONICAL_DROPS.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let mut assembly: TestPublication<DropCounter, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("double rejection assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("success rejection candidate must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("double rejection must prepare minimal fallback"));
    assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback {
            ledger_evidence, ..
        } => {
            assert_eq!(
                published_failure_cause(&ledger_evidence).class(),
                PreparationFailureClass::Ledger
            );
        }
        _ => panic!("double primary rejection must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn primary_ordering_panic_retains_storage_and_attribution_without_cleanup() {
    static BLOCK_DROPS: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_LIVE: AtomicU64 = AtomicU64::new(0);

    let _ord_lock = PRIMARY_DIMENSION_ORD_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _reset = PrimaryDimensionOrdReset;
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);
    BLOCK_DROPS.store(0, Ordering::SeqCst);
    OBSERVED_ACTIVE.store(0, Ordering::SeqCst);
    OBSERVED_LIVE.store(0, Ordering::SeqCst);

    type PanicOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, PrimaryPanicDimension, u32>;
    type PanicPublication = RetainedPublicationAssembly<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        PrimaryPanicDimension,
        u32,
        PrimaryBindDropObservation,
        u64,
    >;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let dimension = PrimaryPanicDimension(7);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(dimension, policy)])
        .unwrap_or_else(|_| panic!("panic-order policy must be valid"));
    let writer = RetainedWriterPlan::new(
        TestPhase::Build,
        dimension,
        dimension,
        RetainedBatchLimits::new(2, 2),
    );
    let start = RetainedOperationStartPlan::new(
        writer,
        RetainedHandleSharePlan::new(dimension, dimension),
        empty_plan(),
        OutputCommitment::new(dimension, 1),
        OutputCommitment::new(dimension, 1),
    );
    let operation =
        PanicOperation::try_start(OperationControl::default(), limits, &account, start, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("panic-order operation must start"));
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let deferred_ledger = StdArc::new(StdMutex::new(Some(ledger)));
    let table_writer = RetainedWriterPlan::new(
        TestPhase::Build,
        dimension,
        dimension,
        RetainedBatchLimits::new(2, 2),
    );
    let publication_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(table_writer, 1, 1),
        RetainedBlockTablePlan::new(table_writer, 1, 1),
    );
    let mut assembly: PanicPublication = operation
        .try_publication_assembly(publication_plan)
        .unwrap_or_else(|_| panic!("panic-order publication must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(
                PrimaryBindDropObservation {
                    ledger: StdArc::clone(&deferred_ledger),
                    drop_count: &BLOCK_DROPS,
                    observed_active: &OBSERVED_ACTIVE,
                    observed_live: &OBSERVED_LIVE,
                },
                CanonicalRootTag,
            ))
        })
        .unwrap_or_else(|_| panic!("panic-order canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9_u64, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("panic-order evidence block must retain"));
    let (expected_active, expected_live) = {
        let ledger = lock_recovering_poison(&deferred_ledger);
        let ledger = ledger.as_ref().expect("panic-order ledger probe");
        (ledger.active_reservations(), ledger.current_live_bytes())
    };

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&9)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("panic-order candidate must validate"),
        };
    let before_physical = account.snapshot();
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(true, Ordering::SeqCst);
    let result = validated.try_prepare(final_progress());
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::PrimaryPublicationBindingPanicked
    ));
    assert_eq!(BLOCK_DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(OBSERVED_ACTIVE.load(Ordering::SeqCst), 0);
    assert_eq!(OBSERVED_LIVE.load(Ordering::SeqCst), 0);
    {
        let ledger = lock_recovering_poison(&deferred_ledger);
        let ledger = ledger.as_ref().expect("retained panic-order ledger probe");
        assert_eq!(ledger.active_reservations(), expected_active);
        assert_eq!(ledger.current_live_bytes(), expected_live);
    }
    let after_physical = account.snapshot();
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert_eq!(
        after_physical.total_admitted_retained_bytes(),
        before_physical.total_admitted_retained_bytes()
    );
    assert_eq!(
        after_physical.total_admitted_retained_allocations(),
        before_physical.total_admitted_retained_allocations()
    );
    assert_eq!(
        after_physical
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::PrimaryPublicationBindingPanicked)
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn cleanup_lattice_is_order_independent_and_preserves_error_precedence() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 16);
    let storage =
        RetainedStorageDropOutcome::StorageReleaseUnverified(record_account_poison_event(
            &account,
            AccountPoisonReason::RetainedStorageReleaseUnverified,
        ));
    let ownership = RetainedStorageDropOutcome::ForeignOwnershipFault(record_account_poison_event(
        &account,
        AccountPoisonReason::OwnershipGraphUnverified,
    ));
    let charge = RetainedStorageDropOutcome::ChargeReleaseUnverified(record_account_poison_event(
        &account,
        AccountPoisonReason::CounterInvariantViolated,
    ));

    for (left, right, expected) in [
        (storage, ownership, RetainedStorageDropSeverity::Storage),
        (storage, charge, RetainedStorageDropSeverity::Storage),
        (ownership, charge, RetainedStorageDropSeverity::Ownership),
        (
            charge,
            RetainedStorageDropOutcome::Clean,
            RetainedStorageDropSeverity::Charge,
        ),
    ] {
        assert_eq!(left.join(right).severity(), expected);
        assert_eq!(right.join(left).severity(), expected);
    }
    assert_eq!(
        RetainedStorageDropOutcome::MissingFaultAnchor
            .join(ownership)
            .witness()
            .map(|witness| witness.reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
    assert_eq!(
        ownership
            .join(RetainedStorageDropOutcome::MissingFaultAnchor)
            .witness()
            .map(|witness| witness.reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );

    let cause: RetainedWriterError<TestDimension> = join_pre_attribution_cleanup_cause(
        RetainedStorageDropOutcome::Clean,
        RetainedWriterError::InvalidPublicationPlan,
    );
    assert!(matches!(cause, RetainedWriterError::InvalidPublicationPlan));
    let cause = join_pre_attribution_cleanup_cause(
        charge,
        RetainedWriterError::<TestDimension>::OwnershipUnverified,
    );
    assert!(matches!(cause, RetainedWriterError::OwnershipUnverified));
    let cause = join_pre_attribution_cleanup_cause(
        ownership,
        RetainedWriterError::<TestDimension>::StorageDropUnverified,
    );
    assert!(matches!(cause, RetainedWriterError::StorageDropUnverified));
    let cause = join_pre_attribution_cleanup_cause(
        storage,
        RetainedWriterError::<TestDimension>::OwnershipUnverified,
    );
    assert!(matches!(cause, RetainedWriterError::StorageDropUnverified));

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn primary_binding_panic_stops_before_later_storage_cleanup() {
    let _ord_lock = PRIMARY_DIMENSION_ORD_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _reset = PrimaryDimensionOrdReset;
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

    type PanicOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, PrimaryPanicDimension, u32>;
    type PanicPublication = RetainedPublicationAssembly<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        PrimaryPanicDimension,
        u32,
        u64,
        u64,
    >;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let dimension = PrimaryPanicDimension(7);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(dimension, policy)])
        .unwrap_or_else(|_| panic!("combined-fault policy must be valid"));
    let writer = RetainedWriterPlan::new(
        TestPhase::Build,
        dimension,
        dimension,
        RetainedBatchLimits::new(2, 2),
    );
    let start = RetainedOperationStartPlan::new(
        writer,
        RetainedHandleSharePlan::new(dimension, dimension),
        empty_plan(),
        OutputCommitment::new(dimension, 1),
        OutputCommitment::new(dimension, 1),
    );
    let operation =
        PanicOperation::try_start(OperationControl::default(), limits, &account, start, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("combined-fault operation must start"));
    let publication_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer, 1, 1),
        RetainedBlockTablePlan::new(writer, 1, 1),
    );
    let mut assembly: PanicPublication = operation
        .try_publication_assembly(publication_plan)
        .unwrap_or_else(|_| panic!("combined-fault publication must open"));
    let ledger = assembly
        .operation
        .as_ref()
        .expect("combined-fault operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11_u64, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("combined-fault canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9_u64, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("combined-fault evidence block must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&9)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("combined-fault candidate must validate"),
        };
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let before_physical = account.snapshot();

    test_panic_final_owner_drop_after(0);
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(true, Ordering::SeqCst);
    let result = validated.try_prepare(final_progress());
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::PrimaryPublicationBindingPanicked
    ));
    let snapshot = account.snapshot();
    assert!(snapshot.poison_event_count() > 0);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::PrimaryPublicationBindingPanicked)
    );
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    assert_eq!(
        snapshot.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        snapshot.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert!(
        take_injected_final_owner_drop_panic(),
        "primary binding uncertainty must stop before any later storage cleanup"
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn second_table_drop_panic_keeps_success_rejection_accounting_fail_closed() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    CANONICAL_DROPS.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let mut assembly: TestPublication<DropCounter, PanickingDropProbe> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("success rejection assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking evidence must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("panicking-drop candidate must validate"),
        };
    let result = validated.try_prepare(final_progress());
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(before_active - probe.active_reservations(), 3);
    assert!(probe.active_reservations() > 0);
    assert!(probe.current_live_bytes() > 0);
    assert!(probe.current_live_bytes() < before_live);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn first_table_drop_panic_stops_before_evidence_cleanup_or_attribution_ack() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    CANONICAL_DROPS.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = test_operation_on_account_with_control(
        &account,
        4,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<PanickingDropProbe, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("success rejection assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking canonical must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();
    let before_physical = account.snapshot();
    let canonical_table_charge = account
        .test_checked_retained_table_charge::<PanickingDropProbe>(1)
        .unwrap_or_else(|_| panic!("canonical table charge must remain derivable"));

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("panicking-drop candidate must validate"),
        };
    let result = validated.try_prepare(final_progress());
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(probe.active_reservations(), before_active);
    assert_eq!(probe.current_live_bytes(), before_live);
    let after_physical = account.snapshot();
    // The table shell is verified released, while the panicking member final owner and the
    // untouched evidence table remain physically accounted and every attribution remains live.
    assert_eq!(
        after_physical
            .live_retained_bytes()
            .checked_add(canonical_table_charge.bytes),
        Some(before_physical.live_retained_bytes())
    );
    assert_eq!(
        after_physical
            .live_retained_allocations()
            .checked_add(canonical_table_charge.allocations),
        Some(before_physical.live_retained_allocations())
    );
    assert_eq!(
        after_physical
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn table_payload_drop_panic_fails_closed_before_attribution_release() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let account = operation
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("rich-both account share must be admitted"));
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let mut assembly: TestPublication<PanickingDropProbe, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    DROP_COUNT.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let _canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&DROP_COUNT), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence must retain"));
    let before_ledger = (ledger.active_reservations(), ledger.current_live_bytes());
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root).is_some()
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("table-drop failure candidate must validate"),
    };
    assert!(matches!(
        validated.try_prepare(PreparationFailureCode::domain(2), final_progress()),
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(
        (ledger.active_reservations(), ledger.current_live_bytes()),
        before_ledger,
        "failed canonical cleanup must not ACK either discard batch"
    );
    assert_eq!(
        account
            .snapshot()
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn baseline_drop_payload_before_releasing_live_ownership() {
    static EXPECTED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static EXPECTED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
    static EXPECTED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
    static EXPECTED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let deferred_ledger = StdArc::new(StdMutex::new(None));
    let fallback = DeferredLiveDropObservation {
        ledger: StdArc::clone(&deferred_ledger),
        account: account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted")),
        observed_active: &OBSERVED_ACTIVE,
        observed_operation_live: &OBSERVED_OPERATION_LIVE,
        observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
        observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
    };
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation: RetainedOperation<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        DeferredLiveDropObservation,
    > = RetainedOperation::try_start(OperationControl::default(), limits, &account, plan, || {
        Ok(fallback)
    })
    .unwrap_or_else(|_| panic!("baseline observation operation must start"));
    let ledger = operation.coordinator.retained_ledger().test_probe();
    EXPECTED_ACTIVE.store(ledger.active_reservations(), Ordering::SeqCst);
    EXPECTED_OPERATION_LIVE.store(ledger.current_live_bytes(), Ordering::SeqCst);
    let physical = operation.account().snapshot();
    EXPECTED_PHYSICAL_BYTES.store(physical.live_retained_bytes(), Ordering::SeqCst);
    EXPECTED_PHYSICAL_ALLOCATIONS.store(physical.live_retained_allocations(), Ordering::SeqCst);
    *lock_recovering_poison(&deferred_ledger) = Some(ledger);

    drop(operation);
    assert_eq!(
        OBSERVED_ACTIVE.load(Ordering::SeqCst),
        EXPECTED_ACTIVE.load(Ordering::SeqCst)
    );
    assert_eq!(
        OBSERVED_OPERATION_LIVE.load(Ordering::SeqCst),
        EXPECTED_OPERATION_LIVE.load(Ordering::SeqCst)
    );
    assert_eq!(
        OBSERVED_PHYSICAL_BYTES.load(Ordering::SeqCst),
        EXPECTED_PHYSICAL_BYTES.load(Ordering::SeqCst)
    );
    assert_eq!(
        OBSERVED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst),
        EXPECTED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst)
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn prepared_published_and_baseline_owners_contain_payload_drop_panics() {
    static PREPARED_DROPS: AtomicU64 = AtomicU64::new(0);
    static PUBLISHED_DROPS: AtomicU64 = AtomicU64::new(0);
    static BASELINE_DROPS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    PREPARED_DROPS.store(0, Ordering::SeqCst);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("prepared drop assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&PREPARED_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("prepared drop block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("prepared drop evidence must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("prepared-drop graph must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("prepared drop publication must prepare"));
    assert!(catch_unwind(AssertUnwindSafe(|| drop(prepared))).is_ok());
    assert_eq!(PREPARED_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);

    let factory = account_factory(1);
    PUBLISHED_DROPS.store(0, Ordering::SeqCst);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("published drop assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&PUBLISHED_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("published drop block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("published drop evidence must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("published-drop graph must validate"),
        };
    let published = handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("published drop publication must prepare")),
    );
    assert!(catch_unwind(AssertUnwindSafe(|| drop(published))).is_ok());
    assert_eq!(PUBLISHED_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);

    let factory = account_factory(1);
    BASELINE_DROPS.store(0, Ordering::SeqCst);
    let operation = test_operation_with_fallback(&factory, PanickingDropProbe(&BASELINE_DROPS));
    assert!(catch_unwind(AssertUnwindSafe(|| drop(operation))).is_ok());
    assert_eq!(BASELINE_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn prepared_and_published_drop_keep_physical_charge_and_handoff_is_a_pure_move() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    OBSERVED_BYTES.store(0, Ordering::SeqCst);
    OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let observed_account = operation
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    let mut assembly: TestPublication<PhysicalDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("prepared observation assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(
                PhysicalDropObservation {
                    account: observed_account,
                    drop_count: &DROP_COUNT,
                    observed_bytes: &OBSERVED_BYTES,
                    observed_allocations: &OBSERVED_ALLOCATIONS,
                },
                CanonicalRootTag,
            ))
        })
        .unwrap_or_else(|_| panic!("prepared observation block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("prepared observation evidence must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("prepared observation graph must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("prepared observation publication must prepare"));
    drop(prepared);
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
    assert_eq!(factory.snapshot().live_accounts(), 0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    OBSERVED_BYTES.store(0, Ordering::SeqCst);
    OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let observed_account = operation
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    let mut assembly: TestPublication<PhysicalDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("handoff observation assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(
                PhysicalDropObservation {
                    account: observed_account,
                    drop_count: &DROP_COUNT,
                    observed_bytes: &OBSERVED_BYTES,
                    observed_allocations: &OBSERVED_ALLOCATIONS,
                },
                CanonicalRootTag,
            ))
        })
        .unwrap_or_else(|_| panic!("handoff observation block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("handoff observation evidence must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("handoff observation graph must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("handoff observation publication must prepare"));
    let prepared = match prepared.into_outcome() {
        PreparedRetainedPublicationOutcome::Success(prepared) => prepared,
        _ => panic!("handoff observation must prepare success"),
    };
    let (canonical_identity, canonical_shares, evidence_identity, evidence_shares) = (
        prepared.canonical.inner.test_table_allocation_identity(),
        prepared.canonical.inner.test_table_share_call_count(),
        prepared
            .outcome_evidence
            .inner
            .test_table_allocation_identity(),
        prepared
            .outcome_evidence
            .inner
            .test_table_share_call_count(),
    );
    let ledger_allocation = prepared.evidence.allocation();
    let ledger_identity = ledger_allocation.allocation_identity();
    let ledger_shares = ledger_allocation.share_call_count();

    let published = prepared.handoff();
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(OBSERVED_BYTES.load(Ordering::SeqCst), 0);
    assert_eq!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(
        published
            .canonical
            .aggregate
            .inner
            .test_table_allocation_identity(),
        canonical_identity
    );
    assert_eq!(
        published
            .canonical
            .aggregate
            .inner
            .test_table_share_call_count(),
        canonical_shares
    );
    assert_eq!(
        published
            .outcome_evidence
            .aggregate
            .inner
            .test_table_allocation_identity(),
        evidence_identity
    );
    assert_eq!(
        published
            .outcome_evidence
            .aggregate
            .inner
            .test_table_share_call_count(),
        evidence_shares
    );
    let ledger_allocation = published.ledger_evidence.evidence.allocation();
    assert_eq!(ledger_allocation.allocation_identity(), ledger_identity);
    assert_eq!(ledger_allocation.share_call_count(), ledger_shares);

    drop(published);
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn rich_failure_fallback_drop_panic_preserves_every_unacknowledged_attribution() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingRichFallback;

    impl Drop for PanickingRichFallback {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    for includes_canonical_discard in [false, true] {
        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let operation = test_operation_with_fallback(&factory, PanickingRichFallback);
        let account = operation
            .account()
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("rich fallback account share must be admitted"));
        let ledger = operation.coordinator.retained_ledger().test_probe();
        let validated = if includes_canonical_discard {
            let mut assembly = operation
                .try_publication_assembly(publication_plan(1, 1))
                .unwrap_or_else(|_| panic!("rich-both fallback fixture must open"));
            let canonical = assembly
                .try_retain_canonical(empty_plan(), |emplacer| {
                    Ok(emplacer.place(11_u64, CanonicalRootTag))
                })
                .unwrap_or_else(|_| panic!("rich-both canonical root must retain"));
            let evidence = assembly
                .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                    Ok(emplacer.place(7_u64, EvidenceRootTag))
                })
                .unwrap_or_else(|_| panic!("rich-both evidence root must retain"));
            let failure = match assembly.validate_success(
                canonical,
                evidence,
                |_session, _canonical, _evidence| false,
            ) {
                RetainedSuccessValidation::Rejected(failure) => failure,
                _ => panic!("rich-both fixture must enter failure validation"),
            };
            match failure.validate_rich_failure(evidence, |session, root| {
                session.outcome_evidence(root) == Some(&7)
            }) {
                RetainedFailureValidation::Valid(validated) => validated,
                _ => panic!("rich-both fallback fixture must validate"),
            }
        } else {
            let plan = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(2, 2),
                1,
                1,
            ));
            let mut assembly = operation
                .try_publication_assembly(plan)
                .unwrap_or_else(|_| panic!("rich-evidence fallback fixture must open"));
            let evidence = assembly
                .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                    Ok(emplacer.place(7_u64, EvidenceRootTag))
                })
                .unwrap_or_else(|_| panic!("rich-evidence root must retain"));
            match assembly.validate_rich_failure(evidence, |session, root| {
                session.outcome_evidence(root) == Some(&7)
            }) {
                RetainedFailureValidation::Valid(validated) => validated,
                _ => panic!("rich-evidence fallback fixture must validate"),
            }
        };
        let before_ledger = (ledger.active_reservations(), ledger.current_live_bytes());
        let before_physical = account.snapshot();

        let result = catch_unwind(AssertUnwindSafe(|| {
            validated.try_prepare(PreparationFailureCode::domain(205), final_progress())
        }))
        .expect("rich fallback destructor panic must be contained");

        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        let after_ledger = (ledger.active_reservations(), ledger.current_live_bytes());
        if includes_canonical_discard {
            assert!(after_ledger.0 < before_ledger.0);
            assert!(after_ledger.1 < before_ledger.1);
        } else {
            assert_eq!(after_ledger, before_ledger);
        }
        assert!(after_ledger.0 > 0);
        assert!(after_ledger.1 > 0);
        let after_physical = account.snapshot();
        assert!(after_physical.live_retained_bytes() <= before_physical.live_retained_bytes());
        assert!(
            after_physical.live_retained_allocations()
                <= before_physical.live_retained_allocations()
        );
        assert!(after_physical.live_retained_bytes() > 0);
        assert!(after_physical.live_retained_allocations() > 0);
        assert_eq!(
            after_physical
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
        );
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }
}

#[test]
fn success_preparation_contains_fallback_drop_panic() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingFallback;

    impl Drop for PanickingFallback {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    type PanicFallbackOperation = RetainedOperation<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        PanickingFallback,
    >;
    type PanicFallbackPublication = RetainedPublicationAssembly<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        PanickingFallback,
        u64,
        u64,
    >;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    DROP_COUNT.store(0, Ordering::SeqCst);
    let operation = PanicFallbackOperation::try_start(
        OperationControl::default(),
        limits,
        &account,
        plan,
        || Ok(PanickingFallback),
    )
    .unwrap_or_else(|_| panic!("panic fallback operation must start"));
    let mut assembly: PanicFallbackPublication = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("panic fallback assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(41, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence block must retain"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&41)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("fallback-panic candidate must validate"),
        };
    let result = catch_unwind(AssertUnwindSafe(|| validated.try_prepare(final_progress())))
        .expect("fallback destructor panic must be contained");
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert!(before_active > 0);
    assert!(before_live > 0);
    assert!(probe.active_reservations() > 0);
    assert!(probe.active_reservations() <= before_active);
    assert!(probe.current_live_bytes() > 0);
    assert!(probe.current_live_bytes() <= before_live);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn rejected_primary_payload_drop_panic_fails_closed_before_token_release() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingEvidence;

    impl Drop for PanickingEvidence {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let account = operation
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("minimal-evidence account share must be admitted"));
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<u64, PanickingEvidence> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("rejected primary assembly must open"));
    DROP_COUNT.store(0, Ordering::SeqCst);
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingEvidence, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking evidence must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root).is_some()
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("rejected-primary candidate must validate"),
    };
    let before_ledger = (ledger.active_reservations(), ledger.current_live_bytes());
    let result = catch_unwind(AssertUnwindSafe(|| {
        validated.try_prepare(PreparationFailureCode::domain(5), final_progress())
    }))
    .expect("primary rejection payload panic must be contained");
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(
        (ledger.active_reservations(), ledger.current_live_bytes()),
        before_ledger,
        "failed evidence cleanup must not ACK its discard batch"
    );
    assert_eq!(
        account
            .snapshot()
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn invalid_and_allocation_failed_open_preserve_operation_for_retry() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let invalid_plan = publication_plan(0, 1);
    let invalid = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
        Ok(_) => panic!("zero-slot table must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        invalid.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let operation = expect_retry_operation(invalid, "invalid plan");

    fail_next_final_owner_allocation();
    let allocation = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
        Ok(_) => panic!("injected table allocation must fail"),
        Err(error) => error,
    };
    assert!(matches!(
        allocation.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = expect_retry_operation(allocation, "verified allocation failure");
    let assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("retry must open a valid assembly"));
    drop(handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("retry fallback must prepare")),
    ));
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn recoverable_block_allocation_failure_preserves_assembly_for_retry() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("retry assembly must open"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();
    let before_physical = assembly
        .account
        .as_ref()
        .expect("assembly retains account")
        .snapshot();

    fail_next_final_owner_allocation();
    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        }),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
        ))
    ));
    let after_physical = assembly
        .account
        .as_ref()
        .expect("assembly retains account after recoverable failure")
        .snapshot();
    assert_eq!(probe.active_reservations(), before_active);
    assert_eq!(probe.current_live_bytes(), before_live);
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );

    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical retry must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("retried assembly graph must validate"),
        };
    let published = handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("retried assembly must prepare")),
    );
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn closure_validation_panic_publishes_minimal_fallback() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical root must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));

    let fallback_only =
        match assembly.validate_success(canonical, evidence, |_session, _canonical, _evidence| {
            std::panic::panic_any(vec![0_u8; 8])
        }) {
            RetainedSuccessValidation::FallbackOnly(fallback) => fallback,
            _ => panic!("validation panic must expose fallback-only state"),
        };
    let prepared = fallback_only
        .prepare_minimal_fallback()
        .unwrap_or_else(|_| panic!("validation panic must prepare minimal fallback"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback {
            ledger_evidence, ..
        } => {
            let cause = published_failure_cause(&ledger_evidence);
            assert_eq!(cause.class(), PreparationFailureClass::ProductGraph);
            assert_eq!(cause.code(), 5);
        }
        _ => panic!("validation panic must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}
