use super::*;
use std::any::TypeId;
use std::num::NonZeroU64;

type ValidatedScalarSuccess<'inv> = ValidatedRetainedSuccess<
    'inv,
    TestOwner,
    TestPhase,
    TestProgress,
    TestUnit,
    TestDimension,
    u32,
    u64,
    u64,
>;

fn scalar_decision_assembly_on_account<'inv>(
    root: &'inv mut TestInvocationRoot<u64, u64>,
    account: &PhysicalStorageAccount,
    failure_work: bool,
) -> TestPublication<'inv, u64, u64> {
    let operation = if failure_work {
        test_operation_on_account_with_failure_work(
            root,
            account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
            1,
            7_u32,
        )
    } else {
        test_operation_on_account_with_control(
            root,
            account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        )
    };
    operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("the original scalar decision assembly must open"))
}

#[test]
fn borrowed_publication_open_binds_original_fields_without_replaying_admission() {
    for forgotten in [false, true] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::<u64, u64>::new();
        let mut operation = test_operation_on_account_with_control(
            &mut root,
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let opened = {
            let ordinary = operation
                .loan_mut()
                .reborrow()
                .open_publication_borrow(publication_plan(1, 1))
                .unwrap_or_else(|_| panic!("the short original-field open must succeed"));
            let opened = ordinary
                .opened_stamp()
                .expect("the original open remains installed");
            if forgotten {
                std::mem::forget(ordinary);
            } else {
                drop(ordinary);
            }
            opened
        };
        let physical = account.snapshot();
        let shares = account.share_call_count();
        let assembly = match operation.bind_opened_assembly(opened) {
            Ok(assembly) => assembly,
            Err(_) => panic!("the original owner must bind its installed tables"),
        };
        assert_eq!(account.snapshot(), physical);
        assert_eq!(account.share_call_count(), shares);
        assert_eq!(assembly.loan().canonical_len(), 0);
        assert_eq!(assembly.loan().outcome_evidence_len(), 0);
        drop(assembly);
        drop(root);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(account.snapshot().poison_event_count(), 0);
    }
}

#[test]
fn borrowed_publication_open_stamp_returns_original_foreign_owner_and_remains_bindable() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let mut source_root = TestInvocationRoot::<u64, u64>::new();
    let mut target_root = TestInvocationRoot::<u64, u64>::new();
    let mut source = test_operation_on_account_with_control(
        &mut source_root,
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let target = test_operation_on_account_with_control(
        &mut target_root,
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let opened = {
        let ordinary = source
            .loan_mut()
            .reborrow()
            .open_publication_borrow(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("the original short publication must open"));
        ordinary
            .opened_stamp()
            .expect("the original opened stamp must exist")
    };
    let physical = account.snapshot();
    let shares = account.share_call_count();
    let (target, opened) = match target.bind_opened_assembly(opened) {
        Err(originals) => originals,
        Ok(_) => panic!("a foreign unopened owner cannot bind an installed publication"),
    };
    assert_eq!(account.snapshot(), physical);
    assert_eq!(account.share_call_count(), shares);
    let source = match source.bind_opened_assembly(opened) {
        Ok(assembly) => assembly,
        Err(_) => panic!("the unchanged original stamp must bind its original fields"),
    };
    assert_eq!(account.snapshot(), physical);
    assert_eq!(account.share_call_count(), shares);
    let target = target
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("the rejected target must remain a writable unopened owner"));
    drop(source);
    drop(target);
    drop(source_root);
    drop(target_root);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot().poison_event_count(), 0);
}

#[test]
fn ordinary_success_decision_cannot_restore_success_after_a_later_rejection() {
    for failure_work in [false, true] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::new();
        let mut assembly = scalar_decision_assembly_on_account(&mut root, &account, failure_work);
        let (canonical, evidence) = retain_scalar_success(&mut assembly);
        let (success, rejected) = {
            let mut ordinary = assembly.loan_mut().borrow_ordinary();
            let success = ordinary.validate_success_decision(
                canonical,
                evidence,
                |session, canonical, evidence| {
                    session.canonical(canonical) == Some(&11)
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            );
            let rejected = ordinary.validate_success_decision(canonical, evidence, |_, _, _| false);
            (success, rejected)
        };
        let physical = account.snapshot();
        let shares = account.share_call_count();
        let (assembly, _original_success) = match assembly.bind_success_decision(success) {
            Err(originals) => originals,
            Ok(_) => panic!("an earlier success decision must not bypass actual rejection"),
        };
        assert_eq!(account.snapshot(), physical);
        assert_eq!(account.share_call_count(), shares);
        assert!(assembly.loan().fields.success_outcome_rejected);
        assert!(
            !assembly
                .loan()
                .operation
                .as_ref()
                .unwrap()
                .common
                .terminal_is_outstanding()
        );
        let failure = match assembly.bind_success_decision(rejected) {
            Ok(RetainedSuccessValidation::Rejected(failure)) => failure,
            _ => panic!("the original later rejection must remain bindable"),
        };
        drop(failure);
        drop(root);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(account.snapshot().poison_event_count(), 0);
    }
}

#[test]
fn ordinary_closure_decision_rejects_foreign_assembly_without_changing_either_owner() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let mut source_root = TestInvocationRoot::new();
    let mut target_root = TestInvocationRoot::new();
    let mut source = scalar_decision_assembly_on_account(&mut source_root, &account, false);
    let mut target = scalar_decision_assembly_on_account(&mut target_root, &account, false);
    let (canonical, evidence) = retain_scalar_success(&mut source);
    let (target_canonical, target_evidence) = retain_scalar_success(&mut target);
    let decision = source
        .loan_mut()
        .borrow_ordinary()
        .validate_success_decision(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        });
    let physical = account.snapshot();
    let shares = account.share_call_count();
    let (target, decision) = match target.bind_success_decision(decision) {
        Err(originals) => originals,
        Ok(_) => panic!("a foreign set must return the original assembly and decision"),
    };
    assert_eq!(account.snapshot(), physical);
    assert_eq!(account.share_call_count(), shares);
    assert!(matches!(
        source.bind_success_decision(decision),
        Ok(RetainedSuccessValidation::Valid(_))
    ));
    assert!(matches!(
        target.validate_success(
            target_canonical,
            target_evidence,
            |session, canonical, evidence| {
                session.canonical(canonical) == Some(&11)
                    && session.outcome_evidence(evidence) == Some(&7)
            }
        ),
        RetainedSuccessValidation::Valid(_)
    ));
    drop(source_root);
    drop(target_root);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot().poison_event_count(), 0);
}

#[test]
fn ordinary_closure_decisions_preserve_both_kinds_on_wrong_binder_rejection() {
    for evidence_only in [false, true] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::new();
        let mut assembly = scalar_decision_assembly_on_account(&mut root, &account, evidence_only);
        let (canonical, evidence) = retain_scalar_success(&mut assembly);
        let decision = {
            let mut ordinary = assembly.loan_mut().borrow_ordinary();
            if evidence_only {
                ordinary.validate_evidence_decision(evidence, |session, evidence| {
                    session.outcome_evidence(evidence) == Some(&7)
                })
            } else {
                ordinary.validate_success_decision(
                    canonical,
                    evidence,
                    |session, canonical, evidence| {
                        session.canonical(canonical) == Some(&11)
                            && session.outcome_evidence(evidence) == Some(&7)
                    },
                )
            }
        };
        let physical = account.snapshot();
        let shares = account.share_call_count();
        if evidence_only {
            let (assembly, decision) = match assembly.bind_success_decision(decision) {
                Err(originals) => originals,
                Ok(_) => panic!("an evidence decision cannot bind success"),
            };
            assert_eq!(account.snapshot(), physical);
            assert_eq!(account.share_call_count(), shares);
            assert!(matches!(
                assembly.bind_evidence_decision(decision),
                Ok(RetainedFailureValidation::Valid(_))
            ));
        } else {
            let (assembly, decision) = match assembly.bind_evidence_decision(decision) {
                Err(originals) => originals,
                Ok(_) => panic!("a success decision cannot bind evidence"),
            };
            assert_eq!(account.snapshot(), physical);
            assert_eq!(account.share_call_count(), shares);
            assert!(matches!(
                assembly.bind_success_decision(decision),
                Ok(RetainedSuccessValidation::Valid(_))
            ));
        }
        drop(root);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(account.snapshot().poison_event_count(), 0);
    }
}

#[test]
fn dynamic_ordinary_borrow_drop_and_forget_leave_payload_cleanup_with_original_owner() {
    static DROPS: AtomicU64 = AtomicU64::new(0);
    for (forgotten, forgotten_assembly) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        DROPS.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::<DropCounter, DropCounter>::new();
        let operation = test_operation_on_account_with_control(
            &mut root,
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let mut assembly = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("the dynamic original assembly must open"));
        let _canonical = assembly
            .loan_mut()
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(DropCounter(&DROPS), CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("the original member must retain"));
        let physical = account.snapshot();
        let shares = account.share_call_count();
        let ordinary = assembly.loan_mut().borrow_ordinary();
        if forgotten {
            std::mem::forget(ordinary);
        } else {
            drop(ordinary);
        }
        assert_eq!(DROPS.load(Ordering::SeqCst), 0);
        assert_eq!(account.snapshot(), physical);
        assert_eq!(account.share_call_count(), shares);
        assert!(
            !assembly
                .loan()
                .operation
                .as_ref()
                .unwrap()
                .common
                .terminal_is_outstanding()
        );
        if forgotten_assembly {
            std::mem::forget(assembly);
            assert_eq!(DROPS.load(Ordering::SeqCst), 0);
            assert_eq!(account.snapshot(), physical);
        } else {
            drop(assembly);
            assert_eq!(DROPS.load(Ordering::SeqCst), 1);
        }
        drop(root);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(account.snapshot().poison_event_count(), 0);
    }
}

fn validated_scalar_success_on_account<'inv>(
    root: &'inv mut TestInvocationRoot<u64, u64>,
    account: &PhysicalStorageAccount,
) -> ValidatedScalarSuccess<'inv> {
    let mut operation = test_operation_on_account_with_control(
        root,
        account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    operation
        .loan_mut()
        .try_complete_phase(TestPhase::Build)
        .unwrap_or_else(|_| panic!("joint-commit build phase must complete"));
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("joint-commit publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);
    assembly
        .loan_mut()
        .try_commit_canonical(TestDimension::Bytes, 5)
        .unwrap_or_else(|_| panic!("canonical commitment must fit"));
    assembly
        .loan_mut()
        .try_commit_outcome_evidence(TestDimension::Bytes, 6)
        .unwrap_or_else(|_| panic!("outcome-evidence commitment must fit"));
    match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
        session.canonical(canonical) == Some(&11) && session.outcome_evidence(evidence) == Some(&7)
    }) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("joint-commit publication must validate"),
    }
}

fn validated_freeze_success_with_limits<'inv>(
    root: &'inv mut TestInvocationRoot<u64, u64>,
    account: &PhysicalStorageAccount,
    committed_limit: u64,
    evidence_limit: u64,
    evidence_members: usize,
) -> ValidatedScalarSuccess<'inv> {
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
    let mut operation = RetainedOperation::try_start(
        root,
        OperationControl::default(),
        limits,
        account,
        plan,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("freeze qualification operation must start"));
    operation
        .loan_mut()
        .try_complete_phase(TestPhase::Build)
        .unwrap_or_else(|_| panic!("freeze qualification build phase must complete"));
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, evidence_members))
        .unwrap_or_else(|_| panic!("freeze qualification publication must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("freeze canonical root must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("freeze evidence root must retain"));
    let detail = (evidence_members == 2).then(|| {
        assembly
            .loan_mut()
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

#[test]
fn foreign_or_rejected_success_proof_cannot_stage_destinations_or_run_final_control() {
    for foreign in [false, true] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut source_root = TestInvocationRoot::new();
        let mut target_root = TestInvocationRoot::new();
        let ValidatedRetainedSuccess {
            assembly: _source,
            proof: source_proof,
        } = validated_freeze_success_with_limits(&mut source_root, &account, 64, 64, 1);
        let final_calls = StdArc::new(AtomicU64::new(0));
        let observed_calls = StdArc::clone(&final_calls);
        let control = OperationControl::default().with_progress_observer(
            move |_event: crate::ProgressEvent<TestProgress, TestUnit>| {
                observed_calls.fetch_add(1, Ordering::SeqCst);
                crate::ProgressDecision::Continue
            },
        );
        let mut operation = test_operation_on_account_with_control(
            &mut target_root,
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            control,
        );
        operation
            .loan_mut()
            .try_complete_phase(TestPhase::Build)
            .unwrap_or_else(|_| panic!("the original target build phase must complete"));
        let mut assembly = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("the original target publication must open"));
        let (canonical, evidence) = retain_scalar_success(&mut assembly);
        let ValidatedRetainedSuccess {
            mut assembly,
            proof,
        } = match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => {
                panic!("the original target proof must be valid before the adversarial transition")
            }
        };
        if !foreign {
            let _later_rejection = assembly
                .loan_mut()
                .borrow_ordinary()
                .validate_success_decision(canonical, evidence, |_, _, _| false);
        }
        let ledger = assembly.loan().test_ledger_probe();
        let before = ledger
            .dimension_evidence(TestDimension::Bytes)
            .expect("the actual target accounting remains observable");
        assert_eq!(assembly.loan().canonical_commitment_len(), 0);
        assert_eq!(assembly.loan().outcome_evidence_commitment_len(), 0);
        assert!(
            assembly
                .loan_mut()
                .try_issue_freeze_decision(
                    if foreign { &source_proof } else { &proof },
                    TestDimension::Bytes,
                    TestDimension::Bytes,
                    final_progress(),
                )
                .is_err()
        );
        assert_eq!(final_calls.load(Ordering::SeqCst), 0);
        let after = ledger
            .dimension_evidence(TestDimension::Bytes)
            .expect("containment retains the original target accounting");
        assert_eq!(after.committed(), before.committed());
        assert_eq!(after.pending_publication(), before.pending_publication());
        assert_eq!(
            after.published_outcome_evidence(),
            before.published_outcome_evidence()
        );
        assert!(!ledger.is_terminal_qualified());
        assert!(account.snapshot().poisoned());
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

fn expect_retry_operation<'inv>(
    error: RetainedPublicationOpenError<TestOperation<'inv, u64, u64>, TestDimension>,
    context: &str,
) -> TestOperation<'inv, u64, u64> {
    match error.into_recovery() {
        Ok(RetainedPublicationOpenRecovery::Retry(operation)) => operation,
        Ok(RetainedPublicationOpenRecovery::FallbackOnly(_)) => {
            panic!("{context}: failure unexpectedly consumed terminal authority")
        }
        Err(cause) => panic!("{context}: open failed closed: {cause:?}"),
    }
}

fn expect_fallback_only_operation<'inv>(
    error: RetainedPublicationOpenError<TestOperation<'inv, u64, u64>, TestDimension>,
    context: &str,
) -> TestOperation<'inv, u64, u64> {
    match error.into_recovery() {
        Ok(RetainedPublicationOpenRecovery::FallbackOnly(operation)) => operation,
        Ok(RetainedPublicationOpenRecovery::Retry(_)) => {
            panic!("{context}: consumed terminal authority was returned as retryable")
        }
        Err(cause) => panic!("{context}: verified rollback failed closed: {cause:?}"),
    }
}

fn reject_invalid_publication_plan<'inv>(
    operation: TestOperation<'inv, u64, u64>,
    plan: RetainedPublicationPlan<TestPhase, TestDimension>,
    context: &str,
) -> TestOperation<'inv, u64, u64> {
    let error = match operation.try_publication_assembly(plan) {
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

fn failure_terminal_operation<'inv>(
    root: &'inv mut TestInvocationRoot<u64, u64>,
    account: &PhysicalStorageAccount,
    ordinary_work_limit: u64,
    max_evidence_members: usize,
) -> TestOperation<'inv, u64, u64> {
    let failure_actions = max_evidence_members
        .checked_add(2)
        .and_then(|actions| u64::try_from(actions).ok())
        .and_then(|actions| actions.checked_mul(2))
        .unwrap_or_else(|| panic!("failure-terminal test bound must fit"));
    failure_terminal_operation_with_limit(
        root,
        account,
        ordinary_work_limit,
        max_evidence_members,
        failure_actions,
    )
}

fn failure_terminal_operation_with_limit<'inv>(
    root: &'inv mut TestInvocationRoot<u64, u64>,
    account: &PhysicalStorageAccount,
    ordinary_work_limit: u64,
    max_evidence_members: usize,
    failure_work_limit: u64,
) -> TestOperation<'inv, u64, u64> {
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
    RetainedOperation::try_start(
        root,
        OperationControl::default(),
        limits,
        account,
        plan,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("failure-terminal test operation must start"))
}

fn ordinary_linear_work_operation<'inv>(
    root: &'inv mut TestInvocationRoot<u64, u64>,
    account: &PhysicalStorageAccount,
    attempted_limit: u64,
    consumed_limit: u64,
) -> TestOperation<'inv, u64, u64> {
    ordinary_linear_work_operation_with_control(
        root,
        account,
        attempted_limit,
        consumed_limit,
        OperationControl::default(),
    )
}

fn ordinary_linear_work_operation_with_control<'inv>(
    root: &'inv mut TestInvocationRoot<u64, u64>,
    account: &PhysicalStorageAccount,
    attempted_limit: u64,
    consumed_limit: u64,
    control: OperationControl<TestProgress, TestUnit>,
) -> TestOperation<'inv, u64, u64> {
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
    RetainedOperation::try_start(root, control, limits, account, plan, || Ok(7_u32))
        .unwrap_or_else(|_| panic!("ordinary linear work operation must start"))
}

fn finish_ordinary_linear_work_test(
    assembly: TestPublication<'_, u64, u64>,
) -> PreparationFailureCode {
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 8, 8);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("ordinary work publication must open"));

    let _evidence = assembly
        .loan_mut()
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
                    Err(RetainedWorkFinishError::Ledger(cause)) => {
                        panic!("exact ordinary work permit must resolve: {cause:?}")
                    }
                }
                Ok(emplacer.place(7, EvidenceRootTag))
            },
        )
        .unwrap_or_else(|_| panic!("exact ordinary work evidence must retain"));

    let work = assembly
        .loan()
        .test_dimension_evidence(TestDimension::Work)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 8, 8);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("incomplete-work publication must open"));

    assert!(matches!(
        assembly.loan_mut().try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
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
        .loan()
        .test_dimension_evidence(TestDimension::Work)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 8, 8);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("partial-work publication must open"));

    assert!(matches!(
        assembly.loan_mut().try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
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
        .loan()
        .test_dimension_evidence(TestDimension::Work)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 8, 8);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("abandoned borrowed-work publication must open"));

    assert!(matches!(
        assembly.loan_mut().try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
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
        .loan()
        .test_dimension_evidence(TestDimension::Work)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 8, 8);
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let rejected = match validation {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        RetainedSuccessValidation::Valid(_) => {
            panic!("an unresolved validation permission must not produce a success proof")
        }
        RetainedSuccessValidation::FallbackOnly(_) => {
            panic!("the validation host must release an unresolved frame")
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
fn forgotten_validation_permission_rejects_success_and_the_host_releases_its_frame() {
    assert_unresolved_validation_permit_cannot_validate_success(true);
}

#[test]
fn partially_resolved_validation_work_cannot_validate_success() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 8, 8);
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation_with_control(
        &mut invocation_root,
        &account,
        8,
        8,
        OperationControl::default().with_cancellation(cancellation.clone()),
    );
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 0, 0);
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation =
        ordinary_linear_work_operation_with_control(&mut invocation_root, &account, 8, 8, control);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("cancellable borrowed-work publication must open"));

    assert!(matches!(
        assembly.loan_mut().try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
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
        .loan()
        .test_dimension_evidence(TestDimension::Work)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 8, 8);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("overrun publication must open"));

    assert!(matches!(
        assembly.loan_mut().try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
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
        .loan()
        .test_dimension_evidence(TestDimension::Work)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = ordinary_linear_work_operation(&mut invocation_root, &account, 8, 2);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("short-capacity publication must open"));
    let mut builder_ran = false;
    let mut action_ran = false;

    assert!(matches!(
        assembly.loan_mut().try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
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
        .loan()
        .test_dimension_evidence(TestDimension::Work)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation =
        ordinary_linear_work_operation_with_control(&mut invocation_root, &account, 8, 8, control);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|error| panic!("cancellation publication must open: {error:?}"));
    cancellation.cancel();
    let mut action_ran = false;

    assert!(matches!(
        assembly.loan_mut().try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
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
        .loan()
        .test_dimension_evidence(TestDimension::Work)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = failure_terminal_operation(&mut invocation_root, &account, 64, 1);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("protected-dimension publication must open"));

    assert!(matches!(
        assembly.loan_mut().try_retain_success_outcome_evidence_controlled::<EvidenceRootTag>(
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
        .loan()
        .test_dimension_evidence(TestDimension::FailureTerminalWork)
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

    let mut invocation_root: TestInvocationRoot = InvocationRoot::new();
    assert!(
        RetainedOperation::try_start(
            &mut invocation_root,
            OperationControl::default(),
            limits,
            &account,
            plan,
            || Ok(7_u32),
        )
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = failure_terminal_operation(&mut invocation_root, &account, 64, 1);
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
    let phase_error = match operation.try_publication_assembly(invalid_phase) {
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
    let dimension_error = match operation.try_publication_assembly(invalid_dimension) {
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

    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(
            RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
        ))
        .unwrap_or_else(|_| panic!("corrected plan must retain terminal capability"));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let mut operation = RetainedOperation::try_start(
        &mut invocation_root,
        OperationControl::default(),
        limits,
        &account,
        plan,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("failure-terminal operation must start"));

    assert!(
        operation
            .loan_mut()
            .common
            .test_ledger_mut()
            .charge_observed_batch(&[(TestDimension::Bytes, 1)])
            .is_err(),
        "ordinary work must retain its non-refundable failing attempt"
    );
    let ordinary = operation
        .loan()
        .common
        .as_read()
        .dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|_| panic!("ordinary dimension evidence"));
    assert_eq!(ordinary.attempted(), 9);
    assert_eq!(ordinary.consumed(), 8);

    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(
            RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
        ))
        .unwrap_or_else(|_| panic!("reserved rich-failure assembly must open"));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = failure_terminal_operation(&mut invocation_root, &account, 64, 1);
    let invalid_plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 2, 2));

    let error = match operation.try_publication_assembly(invalid_plan) {
        Ok(_) => panic!("oversized failure evidence plan must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let operation = expect_retry_operation(error, "failure-terminal plan preflight");

    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(
            RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
        ))
        .unwrap_or_else(|_| panic!("corrected rich-failure plan must retain capability"));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = failure_terminal_operation(&mut invocation_root, &account, 64, 1);
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));

    fail_next_final_owner_allocation();
    let error = match operation.try_publication_assembly(plan) {
        Ok(_) => panic!("injected post-preflight allocation must fail"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = expect_fallback_only_operation(error, "post-preflight allocation failure");
    let terminal = operation
        .loan()
        .common
        .as_read()
        .dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|_| panic!("failure-terminal evidence after attempted open"));
    assert_eq!((terminal.attempted(), terminal.consumed()), (4, 4));

    let published = handoff_for_test(
        operation
            .prepare_minimal_fallback(PreparationFailureCode::domain(704))
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation =
        failure_terminal_operation(&mut invocation_root, &account, ORDINARY_WORK_LIMIT, 2);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 2))
        .unwrap_or_else(|_| panic!("success-capable publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
    let ordinary = assembly
        .loan()
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ordinary dimension evidence before exhaustion"));
    let remaining = ORDINARY_WORK_LIMIT
        .checked_sub(ordinary.consumed())
        .unwrap_or_else(|| panic!("success candidate exceeded ordinary work limit"));
    assembly
        .loan_mut()
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
        .borrow_ordinary()
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = failure_terminal_operation(&mut invocation_root, &account, 64, 2);
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = failure_terminal_operation(&mut invocation_root, &account, 64, 2);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 2))
        .unwrap_or_else(|_| panic!("success-capable publication must open"));
    let (_canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let mut failure = match assembly.validate_rich_failure(evidence_root, |_session, _root| false) {
        RetainedFailureValidation::Rejected(failure) => failure,
        _ => panic!("direct rich validation must retain a failure assembly"),
    };
    let ordinary_before = failure
        .assembly
        .loan()
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ordinary evidence before protected append"));
    let detail = failure
        .borrow_ordinary()
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("direct rich validation must activate protected append work"));
    let ordinary_after = failure
        .assembly
        .loan()
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ordinary evidence after protected append"));
    let protected = failure
        .assembly
        .loan()
        .test_dimension_evidence(TestDimension::FailureTerminalWork)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation =
        failure_terminal_operation_with_limit(&mut invocation_root, &account, 64, 2, 64);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(
            RetainedBlockTablePlan::new(writer_plan(3, 2), 2, 2),
        ))
        .unwrap_or_else(|_| panic!("protected failure assembly must open"));
    let evidence_root = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("protected evidence root must retain"));

    assert!(matches!(
        assembly.loan_mut().try_retain_outcome_evidence::<EvidenceDetailTag>(empty_plan(), |_emplacer| {
            Err(BUILD_FAILED)
        }),
        Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::Build(cause)
        ))) if cause == BUILD_FAILED
    ));
    let after_first_failure = assembly
        .loan()
        .test_dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("failure-terminal evidence after protected failure"));

    assert!(matches!(
        assembly
            .loan_mut()
            .try_retain_outcome_evidence::<EvidenceDetailTag>(empty_plan(), |emplacer| {
                Ok(emplacer.place(9, EvidenceDetailTag))
            }),
        Err(RetainedPublicationError::FailureTerminalWorkSealed)
    ));
    let after_rejected_retry = assembly
        .loan()
        .test_dimension_evidence(TestDimension::FailureTerminalWork)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation =
        failure_terminal_operation(&mut invocation_root, &account, ORDINARY_WORK_LIMIT, 3);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(2, 3))
        .unwrap_or_else(|_| panic!("success-capable publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);
    let ordinary = assembly
        .loan()
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("ordinary evidence before terminal rejection"));
    let failing_amount = ORDINARY_WORK_LIMIT
        .checked_sub(ordinary.consumed())
        .and_then(|remaining| remaining.checked_add(1))
        .unwrap_or_else(|| panic!("ordinary failing amount must fit"));
    assert!(matches!(
        assembly
            .loan_mut()
            .try_control_checkpoint()
            .unwrap_or_else(|_| panic!("ordinary control checkpoint"))
            .charge_observed(TestDimension::Bytes, failing_amount),
        Err(LedgerCause::LimitExceeded { .. })
    ));

    let detail = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("recorded rejection must activate protected evidence work"));
    let protected_after_activation = assembly
        .loan()
        .test_dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected evidence after activation"));
    assert_eq!(
        (
            protected_after_activation.attempted(),
            protected_after_activation.consumed()
        ),
        (2, 2)
    );

    assert!(matches!(
        assembly
            .loan_mut()
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(12, CanonicalRootTag))
            }),
        Err(RetainedPublicationError::SuccessOutcomeRejected)
    ));
    let protected_after_canonical_retry = assembly
        .loan()
        .test_dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected evidence after canonical retry"));
    assert_eq!(protected_after_canonical_retry, protected_after_activation);

    assert!(matches!(
        assembly.loan_mut().try_retain_outcome_evidence::<EvidenceDetailTag>(empty_plan(), |_emplacer| {
            Err(BUILD_FAILED)
        }),
        Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::Build(cause)
        ))) if cause == BUILD_FAILED
    ));
    let protected_after_failure = assembly
        .loan()
        .test_dimension_evidence(TestDimension::FailureTerminalWork)
        .unwrap_or_else(|| panic!("protected evidence after sealed failure"));
    assert!(matches!(
        assembly
            .loan_mut()
            .try_commit_outcome_evidence(TestDimension::Bytes, 1),
        Err(RetainedPublicationError::FailureTerminalWorkSealed)
    ));
    let protected_after_commit = assembly
        .loan()
        .test_dimension_evidence(TestDimension::FailureTerminalWork)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = match case {
        FailureGuardCase::RichEvidence
        | FailureGuardCase::RichBoth
        | FailureGuardCase::MinimalEvidence => test_operation_on_account_with_failure_work(
            &mut invocation_root,
            &account,
            evidence_limit,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
            1,
            7_u32,
        ),
        FailureGuardCase::MinimalNone | FailureGuardCase::MinimalBoth => {
            test_operation_on_account_with_control(
                &mut invocation_root,
                &account,
                evidence_limit,
                RetainedBatchLimits::new(2, 2),
                OperationControl::default(),
            )
        }
    };
    let ledger = operation.loan().common.test_ledger().test_shared_probe();

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
            let mut assembly: TestPublication<'_, u64, u64> = operation
                .try_publication_assembly(plan)
                .unwrap_or_else(|_| panic!("rich-evidence fixture must open"));
            let evidence = assembly
                .loan_mut()
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
            let mut assembly: TestPublication<'_, u64, u64> = operation
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
        FailureGuardCase::MinimalNone => {
            run_prepare!(operation.prepare_minimal_fallback(PreparationFailureCode::domain(203)))
        }
        FailureGuardCase::MinimalEvidence => {
            let plan = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(2, 2),
                1,
                1,
            ));
            let mut assembly: TestPublication<'_, u64, u64> = operation
                .try_publication_assembly(plan)
                .unwrap_or_else(|_| panic!("minimal-evidence fixture must open"));
            let evidence = assembly
                .loan_mut()
                .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                    Ok(emplacer.place(7, EvidenceRootTag))
                })
                .unwrap_or_else(|_| panic!("minimal-evidence root must retain"));
            assembly
                .loan_mut()
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
            let mut assembly: TestPublication<'_, u64, u64> = operation
                .try_publication_assembly(publication_plan(1, 1))
                .unwrap_or_else(|_| panic!("minimal-both fixture must open"));
            let (canonical, evidence) = retain_scalar_success(&mut assembly);
            assembly
                .loan_mut()
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
    let mut invocation_root: TestInvocationRoot = InvocationRoot::new();
    let mut operation = test_operation_with_writer_limits(
        &mut invocation_root,
        &factory,
        64,
        RetainedBatchLimits::new(3, 2),
    );
    operation
        .loan_mut()
        .try_complete_phase(TestPhase::Build)
        .unwrap_or_else(|_| panic!("semantic phase completion must fit the frozen catalog"));
    let mut assembly: TestPublication<'_, TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 2))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let leaf = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(11), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("canonical leaf must retain"));
    let canonical_root = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical root must retain"));
    let detail = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(7), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence detail must retain"));
    let evidence_root = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Root { detail }, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));

    assert_eq!(assembly.loan().canonical_len(), 2);
    assert_eq!(assembly.loan().outcome_evidence_len(), 2);
    assembly
        .loan_mut()
        .try_commit_canonical(TestDimension::Bytes, 5)
        .unwrap_or_else(|_| panic!("canonical commitment must fit"));
    assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let validated = validated_scalar_success_on_account(&mut invocation_root, &account);

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

#[test]
fn frozen_success_classification_defers_qualification_and_freeze_until_finish() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let (deferred, ledger) = {
        let validated =
            validated_freeze_success_with_limits(&mut invocation_root, &account, 64, 64, 1);
        let ledger = validated
            .assembly
            .loan()
            .operation
            .as_ref()
            .expect("validated success must retain its operation")
            .common
            .test_ledger()
            .test_shared_probe();
        let permit = match validated
            .try_issue_freeze_permit(TestDimension::Bytes, TestDimension::Bytes, final_progress())
            .unwrap_or_else(|_| panic!("the exact destinations must issue a permit"))
        {
            RetainedSuccessFreezePermitIssue::Ready(permit) => permit,
            _ => panic!("the healthy candidate must retain a ready permit"),
        };
        let physical_before = account.snapshot();
        let RetainedSuccessFreezeContribution { validated } = permit.into_contribution();
        let ValidatedRetainedSuccess { assembly, proof } = validated;
        let classification = assembly
            .into_loan()
            .try_classify_terminal_preparation(RetainedTerminalRequest::FrozenSuccess { proof })
            .unwrap_or_else(|_| panic!("the frozen contribution must classify before preparation"));

        assert!(classification.is_success());
        assert_eq!(classification.control_cause(), None);
        assert!(!classification.state.test_evidence_is_frozen());
        assert!(ledger.is_available());
        assert!(!ledger.is_terminal_qualified());
        let dimension = ledger
            .dimension_evidence(TestDimension::Bytes)
            .expect("the original ledger must remain observable");
        assert_eq!(dimension.committed(), 0);
        assert_eq!(dimension.committed_canonical_output(), 0);
        assert_eq!(dimension.published_outcome_evidence(), 0);
        assert_eq!(
            account.snapshot().live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            account.snapshot().live_retained_allocations(),
            physical_before.live_retained_allocations()
        );

        let classification = classification
            .check_payload_cleanup()
            .unwrap_or_else(|_| panic!("a healthy preflight must retain the same classification"));
        assert!(!classification.state.test_evidence_is_frozen());
        assert!(!ledger.is_terminal_qualified());
        let (deferred, control_cause) = classification
            .require_success()
            .unwrap_or_else(|_| panic!("the healthy classification must retain success"))
            .finish_deferred()
            .unwrap_or_else(|_| {
                panic!("the healthy classification must prepare its actual fields")
            });
        assert_eq!(control_cause, None);
        assert!(ledger.is_available());
        (deferred, ledger)
    };
    let stamp = invocation_root.install_terminal_preparation(deferred);
    let formation = invocation_root
        .try_finish_publication_terminal(stamp)
        .unwrap_or_else(|_| panic!("the original root must accept its own stamp"))
        .unwrap_or_else(|_| panic!("the healthy actual fields must form an intent"));
    let intent = match formation {
        RetainedSuccessJointCommitFormation::Intent(intent) => intent,
        _ => panic!("the frozen success cannot become a prepared failure"),
    };
    assert!(!ledger.is_available());
    let published = intent
        .qualify_and_commit_frozen_exact()
        .unwrap_or_else(|_| panic!("the exact original account must commit"))
        .handoff();
    assert_eq!(*published.canonical.root(), 11);
    assert_eq!(*published.outcome_evidence.root(), 7);
    let dimension = published
        .ledger_evidence
        .dimension(TestDimension::Bytes)
        .expect("the frozen ledger must preserve its dimension");
    assert_eq!(dimension.committed_canonical_output(), 1);
    assert_eq!(dimension.published_outcome_evidence(), 2);
    drop(published);
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    match validated_freeze_success_with_limits(
        &mut invocation_root,
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

    drop(invocation_root);

    let mut invocation_root_2: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    {
        let issue = validated_freeze_success_with_limits(
            &mut invocation_root_2,
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
                assert_eq!(
                    failure.assembly.loan.fields.mode,
                    RetainedOutcomeMode::FailureOnly
                );
                assert!(failure.assembly.loan.canonical_ownership_is_absent());
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
    }
    drop(invocation_root_2);
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let validated = validated_freeze_success_with_limits(&mut invocation_root, &account, 1, 2, 1);

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
    assert_eq!(
        permit.validated.assembly.loan().canonical_commitment_len(),
        1
    );
    assert_eq!(
        permit
            .validated
            .assembly
            .loan()
            .outcome_evidence_commitment_len(),
        1
    );

    drop(permit);
    drop(invocation_root);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_joint_commit_intent_matches_the_composed_prepare_result_exactly() {
    let old_factory = account_factory(1);
    let old_account = account(&old_factory, u64::MAX, 128);
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let old = validated_scalar_success_on_account(&mut invocation_root, &old_account)
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("legacy preparation must succeed"));
    let old_signature = scalar_success_signature(old);

    let intent_factory = account_factory(1);
    let intent_account = account(&intent_factory, u64::MAX, 128);
    let mut invocation_root_2: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let intent = match validated_scalar_success_on_account(&mut invocation_root_2, &intent_account)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        control,
    );
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
fn control_failure_classification_defers_rich_and_minimal_freeze_until_cleanup() {
    for minimal in [false, true] {
        for (cleanup_poison, require_success) in [(false, false), (true, false), (false, true)] {
            let factory = account_factory(1);
            let account = account(&factory, u64::MAX, 128);
            let control = OperationControl::default().with_progress_observer(
                |_event: crate::ProgressEvent<TestProgress, TestUnit>| {
                    crate::ProgressDecision::Cancel
                },
            );
            let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
            let (deferred, control_cause, ledger, expected_control, expected_cause) = {
                let operation = test_operation_on_account_with_control(
                    &mut invocation_root,
                    &account,
                    if minimal { 4 } else { 64 },
                    RetainedBatchLimits::new(2, 2),
                    control,
                );
                let ledger = operation.loan().common.test_ledger().test_shared_probe();
                let mut assembly: TestPublication<'_, u64, u64> = operation
                    .try_publication_assembly(publication_plan(1, 1))
                    .unwrap_or_else(|_| panic!("control failure publication must open"));
                let (canonical, evidence) = retain_scalar_success(&mut assembly);
                if minimal {
                    assembly
                        .loan_mut()
                        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
                        .unwrap_or_else(|_| panic!("the existing evidence-limit trigger must fit"));
                }
                let validated = match assembly.validate_success(
                    canonical,
                    evidence,
                    |session, canonical, evidence| {
                        session.canonical(canonical) == Some(&11)
                            && session.outcome_evidence(evidence) == Some(&7)
                    },
                ) {
                    RetainedSuccessValidation::Valid(validated) => validated,
                    _ => panic!("the control failure fixture must validate its actual roots"),
                };
                let physical_before = account.snapshot();
                let ValidatedRetainedSuccess { assembly, proof } = validated;
                let classification = assembly
                    .into_loan()
                    .try_classify_terminal_preparation(RetainedTerminalRequest::Success {
                        proof,
                        final_progress: final_progress(),
                    })
                    .unwrap_or_else(|_| {
                        panic!("control rejection must retain a pending failure owner")
                    });
                let expected_control = classification
                    .control_cause()
                    .cloned()
                    .expect("the actual final control rejection must survive classification");
                assert!(matches!(expected_control, ControlCause::Cancelled { .. }));
                let expected_cause = PreparationFailureCode::from_control(&expected_control);
                assert!(!classification.is_success());
                assert_eq!(
                    matches!(
                        &classification.state,
                        TerminalPreFreezeTransition::Fallback { .. }
                    ),
                    minimal
                );
                assert!(!classification.state.test_evidence_is_frozen());
                assert!(ledger.is_available());
                assert!(!ledger.is_terminal_qualified());
                let dimension = ledger
                    .dimension_evidence(TestDimension::Bytes)
                    .expect("the original failure ledger must still be present");
                assert_eq!(dimension.committed_canonical_output(), 0);
                assert_eq!(dimension.published_outcome_evidence(), 0);
                assert_eq!(
                    account.snapshot().live_retained_bytes(),
                    physical_before.live_retained_bytes()
                );
                assert_eq!(
                    account.snapshot().live_retained_allocations(),
                    physical_before.live_retained_allocations()
                );

                if require_success {
                    let cause = match classification.require_success() {
                        Err(RetainedPublicationPrepareError::FinalizationContainment(cause)) => {
                            cause
                        }
                        Ok(_) => {
                            panic!("the success-only finish cannot publish a classified failure")
                        }
                    };
                    assert_eq!(
                        cause.reason(),
                        AccountPoisonReason::OwnershipGraphUnverified
                    );
                    assert!(ledger.is_available());
                    assert!(!ledger.is_terminal_qualified());
                    assert_eq!(
                        account.snapshot().live_retained_bytes(),
                        physical_before.live_retained_bytes()
                    );
                    assert_eq!(
                        account.snapshot().live_retained_allocations(),
                        physical_before.live_retained_allocations()
                    );
                    let dimension = ledger.dimension_evidence(TestDimension::Bytes).unwrap();
                    assert_eq!(dimension.committed_canonical_output(), 0);
                    assert_eq!(dimension.published_outcome_evidence(), 0);
                    drop(account);
                    assert_eq!(factory.snapshot().live_accounts(), 1);
                    continue;
                }

                if cleanup_poison {
                    let first_poison = account
                        .test_record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
                    let poison_events = account.snapshot().poison_event_count();
                    let cause = match classification.check_payload_cleanup() {
                        Err(cause) => cause,
                        Ok(_) => panic!("payload release poison must prevent failure preparation"),
                    };
                    assert_eq!(
                        cause.reason(),
                        AccountPoisonReason::RetainedStorageReleaseUnverified
                    );
                    assert_eq!(
                        cause.account_poisoned().snapshot().first_poison_evidence(),
                        first_poison.snapshot().first_poison_evidence()
                    );
                    // Each of the two one-member tables records its table and member edge;
                    // the fallback and ledger-evidence allocations add one edge each.
                    // The existing first fault is preserved without another root fault.
                    assert_eq!(account.snapshot().poison_event_count(), poison_events + 6);
                    assert!(ledger.is_available());
                    assert!(!ledger.is_terminal_qualified());
                    assert_eq!(
                        account.snapshot().live_retained_bytes(),
                        physical_before.live_retained_bytes()
                    );
                    assert_eq!(
                        account.snapshot().live_retained_allocations(),
                        physical_before.live_retained_allocations()
                    );
                    drop(account);
                    assert_eq!(factory.snapshot().live_accounts(), 1);
                    continue;
                }

                let classification = classification.check_payload_cleanup().unwrap_or_else(|_| {
                    panic!("clean payload disposal must retain the failure owner")
                });
                assert!(!classification.state.test_evidence_is_frozen());
                assert!(!ledger.is_terminal_qualified());
                let (deferred, control_cause) =
                    classification.finish_deferred().unwrap_or_else(|_| {
                        panic!("the selected failure rail must prepare after cleanup")
                    });
                (
                    deferred,
                    control_cause,
                    ledger,
                    expected_control,
                    expected_cause,
                )
            };
            let stamp = invocation_root.install_terminal_preparation(deferred);
            let formation = invocation_root
                .try_finish_publication_terminal(stamp)
                .unwrap_or_else(|_| panic!("the original failure root must accept its stamp"))
                .unwrap_or_else(|_| {
                    panic!("the selected failure rail must consume its actual owners")
                });
            assert_eq!(control_cause, Some(expected_control));
            let prepared = match formation {
                RetainedSuccessJointCommitFormation::PreparedFailure(prepared) => prepared,
                RetainedSuccessJointCommitFormation::Intent(_) => {
                    panic!("a classified control failure cannot become success")
                }
            };
            match (minimal, handoff_for_test(prepared)) {
                (
                    false,
                    PublishedRetainedPublicationParts::RichFailure {
                        outcome_evidence,
                        ledger_evidence,
                    },
                ) => {
                    assert_eq!(*outcome_evidence.outcome_evidence().root(), 7);
                    assert_eq!(published_failure_cause(&ledger_evidence), expected_cause);
                }
                (
                    true,
                    PublishedRetainedPublicationParts::MinimalFallback {
                        fallback,
                        ledger_evidence,
                    },
                ) => {
                    assert_eq!(*fallback.value(), 7);
                    assert_eq!(published_failure_cause(&ledger_evidence), expected_cause);
                }
                _ => panic!("finishing must preserve the actual rich or minimal decision"),
            }
            assert!(!ledger.is_available());
            drop(account);
            assert_eq!(factory.snapshot().live_accounts(), 0);
        }
    }
}

#[test]
fn requested_minimal_classification_preserves_cause_without_freezing() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let requested_cause = PreparationFailureCode::domain(91);
    let (deferred, control_cause) = {
        let validated = validated_scalar_success_on_account(&mut invocation_root, &account);
        let classification = validated
            .assembly
            .into_loan()
            .try_classify_terminal_preparation(RetainedTerminalRequest::MinimalFallback {
                requested_cause,
            })
            .unwrap_or_else(|_| {
                panic!("an explicit minimal request must retain its pending owner")
            });

        assert!(!classification.is_success());
        assert_eq!(classification.control_cause(), None);
        assert!(!classification.state.test_evidence_is_frozen());
        classification
            .check_payload_cleanup()
            .unwrap_or_else(|_| panic!("the original fallback account must remain healthy"))
            .finish_deferred()
            .unwrap_or_else(|_| panic!("the explicit minimal request must prepare"))
    };
    let stamp = invocation_root.install_terminal_preparation(deferred);
    let formation = invocation_root
        .try_finish_publication_terminal(stamp)
        .unwrap_or_else(|_| panic!("the original minimal root must accept its stamp"))
        .unwrap_or_else(|_| panic!("the explicit minimal request must consume its actual owners"));
    assert_eq!(control_cause, None);
    let prepared = match formation {
        RetainedSuccessJointCommitFormation::PreparedFailure(prepared) => prepared,
        RetainedSuccessJointCommitFormation::Intent(_) => {
            panic!("an explicit minimal request must not form a success intent")
        }
    };
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback {
            fallback,
            ledger_evidence,
        } => {
            assert_eq!(*fallback.value(), 7);
            assert_eq!(published_failure_cause(&ledger_evidence), requested_cause);
        }
        _ => panic!("the explicit minimal request must keep its selected terminal owner"),
    }
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn containing_pre_freeze_classification_retains_unqualified_original_graph() {
    for poisoned in [false, true] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
        let validated = validated_scalar_success_on_account(&mut invocation_root, &account);
        let ledger = validated
            .assembly
            .loan()
            .operation
            .as_ref()
            .expect("the actual candidate must retain its ledger")
            .common
            .test_ledger()
            .test_shared_probe();
        let ValidatedRetainedSuccess { assembly, proof } = validated;
        let classification = assembly
            .into_loan()
            .try_classify_terminal_preparation(RetainedTerminalRequest::Success {
                proof,
                final_progress: final_progress(),
            })
            .unwrap_or_else(|_| panic!("the healthy candidate must classify before containment"));
        assert!(classification.is_success());
        assert!(!classification.state.test_evidence_is_frozen());
        let physical_before = account.snapshot();
        let expected_reason = if poisoned {
            account.test_record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
            AccountPoisonReason::RetainedStorageReleaseUnverified
        } else {
            AccountPoisonReason::OwnershipGraphUnverified
        };
        let cause = classification.contain_activation_fault();

        assert_eq!(cause.reason(), expected_reason);
        assert!(ledger.is_available());
        assert!(!ledger.is_terminal_qualified());
        // One root fault plus the six actual table/member/baseline edges above.
        assert_eq!(account.snapshot().poison_event_count(), 7);
        assert_eq!(
            account.snapshot().live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            account.snapshot().live_retained_allocations(),
            physical_before.live_retained_allocations()
        );
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }
}

#[test]
fn dropping_success_joint_commit_intent_fails_closed_before_eligibility() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let intent = match validated_scalar_success_on_account(&mut invocation_root, &account)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let intent = match validated_scalar_success_on_account(&mut invocation_root, &account)
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
        let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
        let operation = test_operation_on_account_with_control(
            &mut invocation_root,
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let ledger = operation.loan().common.test_ledger().test_shared_probe();
        let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
    let physical_before = account.snapshot();
    let ledger_before = (ledger.active_reservations(), ledger.current_live_bytes());

    panic_next_terminal_evidence_qualification();
    let cause = match operation.prepare_minimal_fallback(PreparationFailureCode::domain(93)) {
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
        .test_member_share_call_count(source_canonical_slot.identity())
        .expect("canonical source slot must resolve");
    let evidence_shares_before = source_evidence
        .aggregate()
        .inner
        .test_member_share_call_count(source_evidence_slot.identity())
        .expect("evidence source slot must resolve");

    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_derived_operation_on_account_with_control(
        &mut invocation_root,
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
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("inherited publication must open"));
    assembly
        .loan_mut()
        .try_bind_canonical_source(source_canonical.aggregate(), TestDimension::Bytes)
        .unwrap_or_else(|_| panic!("canonical source map must bind"));
    assert_eq!(
        assembly
            .loan()
            .test_take_canonical_source_mapping_probe_count(),
        0,
        "binding initializes the dense map without lookup probes"
    );
    let physical_before_shares = account.snapshot();

    let canonical_root = assembly
        .loan_mut()
        .try_share_canonical(source_canonical.aggregate(), source_canonical_slot)
        .unwrap_or_else(|_| panic!("first canonical share must succeed"));
    assert_eq!(
        assembly
            .loan()
            .test_take_canonical_source_mapping_probe_count(),
        1,
        "a first source lookup performs one dense-map probe"
    );
    let canonical_second = assembly
        .loan_mut()
        .try_share_canonical(source_canonical.aggregate(), source_canonical_slot)
        .unwrap_or_else(|_| panic!("second canonical share must succeed"));
    assert_eq!(
        assembly
            .loan()
            .test_take_canonical_source_mapping_probe_count(),
        1,
        "a duplicate source lookup remains one dense-map probe"
    );
    let evidence_root = assembly
        .loan_mut()
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("first evidence share must succeed"));
    let evidence_second = assembly
        .loan_mut()
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("second evidence share must succeed"));
    let evidence_third = assembly
        .loan_mut()
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("third evidence share must succeed"));

    assert_eq!(account.snapshot(), physical_before_shares);
    assert_eq!(canonical_second, canonical_root);
    assert_eq!(assembly.loan().canonical_len(), 1);
    assert_eq!(assembly.loan().outcome_evidence_len(), 3);
    assert!(
        assembly.loan().test_storage_graph_is_exact(),
        "inherited-only incremental proofs must match the full structural audit"
    );
    assert_eq!(
        assembly
            .loan()
            .fields
            .canonical_batch
            .as_ref()
            .expect("canonical attribution batch")
            .attribution_len(),
        1
    );
    assert_eq!(
        assembly
            .loan()
            .fields
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
            .test_member_share_call_count(source_canonical_slot.identity())
            .expect("canonical source slot must remain observable"),
        canonical_shares_before + 1
    );
    assert_eq!(
        source_evidence
            .aggregate()
            .inner
            .test_member_share_call_count(source_evidence_slot.identity())
            .expect("evidence source slot must remain observable"),
        evidence_shares_before + 3
    );
    assert!(source_canonical.aggregate().get(canonical_root).is_none());
    assert!(source_evidence.aggregate().get(evidence_root).is_none());

    let canonical_new = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(13, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("one new canonical block must fit"));
    assert_eq!(assembly.loan().canonical_len(), 2);
    assert_eq!(
        assembly
            .loan()
            .fields
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
        assembly.loan().test_storage_graph_is_exact(),
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let source_operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        64,
        RetainedBatchLimits::new(SOURCE_BLOCKS + 1, 2),
        OperationControl::default(),
    );
    let mut source: TestPublication<'_, u64, u64> = source_operation
        .try_publication_assembly(publication_plan(SOURCE_BLOCKS, 1))
        .unwrap_or_else(|_| panic!("dense source publication must open"));
    let mut source_slots = Vec::with_capacity(SOURCE_BLOCKS);
    for value in 0..SOURCE_BLOCKS {
        let value = u64::try_from(value).expect("test source ordinal fits u64");
        let slot = source
            .loan_mut()
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(value, CanonicalLeafTag))
            })
            .unwrap_or_else(|_| panic!("dense source member must retain"));
        source_slots.push(slot);
    }
    let evidence = source
        .loan_mut()
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

    let mut invocation_root_2: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let undersized_operation = test_operation_on_account_with_control(
        &mut invocation_root_2,
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
    let mut undersized: TestPublication<'_, u64, u64> = undersized_operation
        .try_publication_assembly(undersized_plan)
        .unwrap_or_else(|_| panic!("undersized dense-map target must open"));
    assert!(matches!(
        undersized
            .loan_mut()
            .try_bind_canonical_source(source_canonical.aggregate(), TestDimension::Bytes,),
        Err(CanonicalSourceBindError::Publication(
            RetainedPublicationError::InvalidSourceSlot
        ))
    ));
    assert_eq!(
        undersized
            .loan()
            .test_take_canonical_source_mapping_probe_count(),
        0
    );
    assert_eq!(undersized.loan().canonical_len(), 0);
    drop(undersized);
    drop(invocation_root_2);

    let mut invocation_root_3: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let target_operation = test_derived_operation_on_account_with_control(
        &mut invocation_root_3,
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
    let mut target: TestPublication<'_, u64, u64> = target_operation
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
        .loan_mut()
        .try_bind_canonical_source(source_canonical.aggregate(), TestDimension::Bytes)
        .unwrap_or_else(|_| panic!("dense source map must bind"));
    assert_eq!(
        target
            .loan()
            .test_take_canonical_source_mapping_probe_count(),
        0
    );

    let source_slot = source_slots[SOURCE_BLOCKS - 2];
    let destination = target
        .loan_mut()
        .try_share_canonical(source_canonical.aggregate(), source_slot)
        .unwrap_or_else(|_| panic!("far source slot must share"));
    assert_eq!(
        target
            .loan()
            .test_take_canonical_source_mapping_probe_count(),
        1
    );
    for _ in 0..REPEATED_LOOKUPS {
        assert_eq!(
            target
                .loan_mut()
                .try_share_canonical(source_canonical.aggregate(), source_slot)
                .unwrap_or_else(|_| panic!("duplicate dense-map lookup must share")),
            destination
        );
    }
    assert_eq!(
        target
            .loan()
            .test_take_canonical_source_mapping_probe_count(),
        REPEATED_LOOKUPS,
        "each lookup probes exactly one dense-map entry regardless of source size"
    );
    assert_eq!(target.loan().canonical_len(), 1);

    drop(target);
    drop(invocation_root_3);
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_derived_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        &source_canonical,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan_with_canonical_source_mapping(1, 1, 1))
        .unwrap_or_else(|_| panic!("poison-first binding target must open"));
    let first_poison = account.test_record_poison(AccountPoisonReason::OwnershipGraphUnverified);

    assert!(matches!(
        assembly.loan_mut().try_bind_canonical_source(wrong_canonical.aggregate(), TestDimension::Bytes),
        Err(CanonicalSourceBindError::Publication(
            RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(cause))
        )) if cause == first_poison
    ));
    assert_eq!(
        assembly
            .loan()
            .test_take_canonical_source_mapping_probe_count(),
        0
    );
    assert_eq!(assembly.loan().canonical_len(), 0);

    drop(assembly);
    drop(invocation_root);
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

    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let result = RetainedOperation::try_start_derived_with_phase_catalog(
        &mut invocation_root,
        OperationControl::default(),
        (limits, &[TestPhase::Build]),
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
        .test_member_share_call_count(source_slot.identity())
        .expect("source slot must resolve");
    let _ = take_test_member_lookups();

    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_derived_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        &bound_canonical,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("exact-source target must open"));
    let result = assembly
        .loan_mut()
        .try_share_canonical(source_canonical.aggregate(), source_slot);

    assert!(matches!(
        result,
        Err(RetainedPublicationError::InvalidSourceSlot)
    ));
    assert_eq!(take_test_member_lookups(), 0);
    assert_eq!(assembly.loan().canonical_len(), 0);
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
            .test_member_share_call_count(source_slot.identity())
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
        .test_member_share_call_count(source_slot.identity())
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = RetainedOperation::try_start_derived_with_phase_catalog(
        &mut invocation_root,
        OperationControl::default(),
        (limits, &[TestPhase::Build]),
        &account,
        source_canonical.aggregate(),
        start,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("exact share-work operation must start"));
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan_with_canonical_source_mapping(1, 1, 1))
        .unwrap_or_else(|_| panic!("exact share-work tables must open"));
    assembly
        .loan_mut()
        .try_bind_canonical_source(source_canonical.aggregate(), TestDimension::Bytes)
        .unwrap_or_else(|_| panic!("canonical source map must bind before share work"));
    let physical_before_share = account.snapshot();
    let work_before_share = assembly
        .loan()
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap_or_else(|| panic!("share work dimension must remain observable"));
    assert_eq!(work_before_share.consumed(), 15);

    let result = assembly
        .loan_mut()
        .try_share_canonical(source_canonical.aggregate(), source_slot);

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
        .loan()
        .test_dimension_evidence(TestDimension::Bytes)
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
    assert_eq!(assembly.loan().canonical_len(), 0);
    assert_eq!(account.snapshot(), physical_before_share);
    assert!(
        physical_before_share.live_retained_bytes()
            > physical_before_operation.live_retained_bytes()
    );
    assert_eq!(
        source_canonical
            .aggregate()
            .inner
            .test_member_share_call_count(source_slot.identity())
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
        let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
        let operation = test_operation_with_failure_work(
            &mut invocation_root,
            &factory,
            64,
            RetainedBatchLimits::new(2, 2),
            1,
        );
        let dimension_before = operation
            .loan()
            .common
            .as_read()
            .dimension_evidence(TestDimension::Bytes)
            .unwrap_or_else(|_| panic!("{case}: dimension evidence before open"));
        let physical_before = operation.loan().account().snapshot();
        let account_shares_before = operation.loan().account().share_call_count();
        let set_ids_before = test_retained_set_id_requests();
        let batch_allocations_before = crate::ledger::test_attribution_batch_allocation_entries();

        let error = match operation.try_publication_assembly(invalid_plan) {
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
            operation.loan().account().share_call_count(),
            account_shares_before,
            "{case}"
        );
        assert_eq!(
            operation.loan().account().snapshot(),
            physical_before,
            "{case}"
        );
        assert_eq!(
            operation
                .loan()
                .common
                .as_read()
                .dimension_evidence(TestDimension::Bytes)
                .unwrap_or_else(|_| panic!("{case}: dimension evidence after rejection")),
            dimension_before,
            "{case}"
        );

        if index == 2 {
            let assembly: TestPublication<'_, u64, u64> = operation
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
                    .prepare_minimal_fallback(PreparationFailureCode::domain(70))
                    .unwrap_or_else(|_| panic!("direct fallback must prepare")),
            ));
        }
        assert_eq!(factory.snapshot().live_accounts(), 0, "{case}");
    }
}

#[test]
fn recoverable_open_failures_prepare_the_baseline_without_tables_or_roots() {
    let factory = account_factory(1);
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    let invalid_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 2),
        RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
    );
    let error = match operation.try_publication_assembly(invalid_plan) {
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
            .prepare_minimal_fallback(PreparationFailureCode::domain(71))
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

    let mut invocation_root_2: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root_2, &factory, 64);
    fail_next_final_owner_allocation();
    let error = match operation.try_publication_assembly(publication_plan(1, 1)) {
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
            .prepare_minimal_fallback(PreparationFailureCode::domain(72))
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    fail_final_owner_allocation_after(1);

    let error = match operation.try_publication_assembly(publication_plan(1, 1)) {
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
            .prepare_minimal_fallback(PreparationFailureCode::domain(73))
            .unwrap_or_else(|_| panic!("recovered operation must prepare fallback")),
    );
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn second_table_allocation_failure_reports_canonical_storage_cleanup_failure() {
    let factory = account_factory(1);
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    fail_final_owner_allocation_after(1);
    test_panic_final_owner_drop_after(0);

    let error = match operation.try_publication_assembly(publication_plan(1, 1)) {
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_with_failure_work(
        &mut invocation_root,
        &factory,
        64,
        RetainedBatchLimits::new(3, 2),
        2,
    );
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
        .borrow_ordinary()
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("bounded failure detail must retain"));
    failure
        .borrow_ordinary()
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_with_failure_work(
        &mut invocation_root,
        &factory,
        64,
        RetainedBatchLimits::new(2, 2),
        1,
    );
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
fn typed_slot_identity_round_trips_and_rejects_mismatched_tags() {
    let set_id = try_next_retained_set_id().expect("test retained set id");
    let canonical_identity = RetainedSlotIdentity {
        set_id,
        index: 3,
        tag: TypeId::of::<CanonicalRootTag>(),
    };
    let canonical =
        CanonicalRetainedSlot::<u64, CanonicalRootTag>::from_identity(canonical_identity);
    assert_eq!(canonical.identity(), canonical_identity);
    assert!(
        catch_unwind(|| {
            CanonicalRetainedSlot::<u64, CanonicalLeafTag>::from_identity(canonical_identity)
        })
        .is_err()
    );

    let evidence_identity = RetainedSlotIdentity {
        set_id,
        index: 5,
        tag: TypeId::of::<EvidenceRootTag>(),
    };
    let evidence =
        OutcomeEvidenceRetainedSlot::<u64, EvidenceRootTag>::from_identity(evidence_identity);
    assert_eq!(evidence.identity(), evidence_identity);
    assert!(
        catch_unwind(|| {
            OutcomeEvidenceRetainedSlot::<u64, EvidenceDetailTag>::from_identity(evidence_identity)
        })
        .is_err()
    );
}

#[test]
fn published_lookup_checks_actual_member_tags_for_retagged_typed_slots() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (canonical, canonical_slot, evidence, evidence_slot) = publish_scalar_source(&account);
    let retagged_canonical: CanonicalRetainedSlot<u64, CanonicalLeafTag> = CanonicalRetainedSlot {
        set_id: canonical_slot.set_id,
        index: canonical_slot.index,
        marker: PhantomData,
    };
    let canonical_member = canonical
        .aggregate()
        .lookup_member(retagged_canonical)
        .expect("retagged slot still addresses the original canonical member");
    assert!(!canonical_member.schema_tag_matches());
    assert!(canonical.aggregate().get(retagged_canonical).is_none());
    assert_eq!(canonical.aggregate().get(canonical_slot), Some(&11));

    let retagged_evidence: OutcomeEvidenceRetainedSlot<u64, EvidenceDetailTag> =
        OutcomeEvidenceRetainedSlot {
            set_id: evidence_slot.set_id,
            index: evidence_slot.index,
            marker: PhantomData,
        };
    let evidence_member = evidence
        .aggregate()
        .lookup_member(retagged_evidence)
        .expect("retagged slot still addresses the original evidence member");
    assert!(!evidence_member.schema_tag_matches());
    assert!(evidence.aggregate().get(retagged_evidence).is_none());
    assert_eq!(evidence.aggregate().get(evidence_slot), Some(&7));

    drop(canonical);
    drop(evidence);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn consuming_closure_validation_rejects_orphans_and_retagged_slots() {
    let factory = account_factory(1);
    let mut invocation_root: TestInvocationRoot = InvocationRoot::new();
    let operation = test_operation_with_writer_limits(
        &mut invocation_root,
        &factory,
        64,
        RetainedBatchLimits::new(3, 2),
    );
    let mut assembly: TestPublication<'_, TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let orphan = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("orphan candidate must retain"));
    let root = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf: orphan }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("root must retain"));
    let evidence = assembly
        .loan_mut()
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

    let mut invocation_root_2: TestInvocationRoot = InvocationRoot::new();
    let operation = test_operation_with_writer_limits(
        &mut invocation_root_2,
        &factory,
        64,
        RetainedBatchLimits::new(3, 2),
    );
    let mut assembly: TestPublication<'_, TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 1))
        .unwrap_or_else(|_| panic!("publication assembly must reopen"));
    let orphan = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("orphan candidate must retain"));
    let root = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf: orphan }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("root must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(9), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence must retain"));
    let retagged: CanonicalRetainedSlot<TestCanonicalBlock, CanonicalRootTag> =
        CanonicalRetainedSlot {
            set_id: orphan.set_id,
            index: orphan.index,
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
    let mut invocation_root: TestInvocationRoot = InvocationRoot::new();
    let operation = test_operation_with_writer_limits(
        &mut invocation_root,
        &factory,
        64,
        RetainedBatchLimits::new(2, 2),
    );
    let mut assembly: TestPublication<'_, TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let _ = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("first block must fit"));
    BUILDER_RAN.store(false, Ordering::SeqCst);
    assert!(matches!(
        assembly
            .loan_mut()
            .try_retain_canonical(empty_plan(), |emplacer| {
                BUILDER_RAN.store(true, Ordering::SeqCst);
                Ok(emplacer.place(TestCanonicalBlock::Leaf(2), CanonicalLeafTag))
            }),
        Err(RetainedPublicationError::CanonicalSlotLimit)
    ));
    assert!(!BUILDER_RAN.load(Ordering::SeqCst));
    let _ = assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<LiveDropObservation, u64> = InvocationRoot::new();
    let operation = test_operation_with_failure_work(
        &mut invocation_root,
        &factory,
        64,
        RetainedBatchLimits::new(2, 2),
        1,
    );
    let mut assembly: TestPublication<'_, LiveDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let ledger = assembly
        .loan_mut()
        .operation
        .as_mut()
        .expect("assembly retains operation")
        .common
        .test_ledger_mut();
    let observation = LiveDropObservation {
        ledger: ledger.test_probe(),
        account: assembly
            .loan()
            .fields
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
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(observation, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("partial canonical block must retain"));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_with_failure_work(
        &mut invocation_root,
        &factory,
        64,
        RetainedBatchLimits::new(2, 2),
        1,
    );
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("failure-only assembly must open"));
    assert!(matches!(
        assembly
            .loan_mut()
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(11, CanonicalRootTag))
            }),
        Err(RetainedPublicationError::WrongOutcomeMode)
    ));
    assert!(matches!(
        assembly
            .loan_mut()
            .try_commit_canonical(TestDimension::Bytes, 1),
        Err(RetainedPublicationError::WrongOutcomeMode)
    ));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        control,
    );
    drop(account);
    let mut assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        4,
        RetainedBatchLimits::new(2, 2),
        control,
    );
    drop(account);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("control rejection publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);
    assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation_with_failure_work(
        &mut invocation_root,
        &factory,
        4,
        RetainedBatchLimits::new(2, 2),
        1,
    );
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("failure-only assembly must open"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("failure evidence must retain"));
    assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<DropCounter, DropCounter> = InvocationRoot::new();
    let operation = RetainedOperation::try_start(
        &mut invocation_root,
        OperationControl::default(),
        limits,
        &account,
        start,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("canonical rejection operation must start"));
    drop(account);
    let mut assembly: TestPublication<'_, DropCounter, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("canonical rejection assembly must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
    assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<DropCounter, DropCounter> = InvocationRoot::new();
    let operation = RetainedOperation::try_start(
        &mut invocation_root,
        OperationControl::default(),
        limits,
        &account,
        start,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("retry-panic operation must start"));
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
    let mut assembly: TestPublication<'_, DropCounter, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("retry-panic publication must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("retry-panic canonical block must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("retry-panic evidence block must retain"));
    assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<DropCounter, DropCounter> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 4);
    let mut assembly: TestPublication<'_, DropCounter, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("double rejection assembly must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
    assembly
        .loan_mut()
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
fn terminal_prepare_never_calls_dimension_ordering() {
    static BLOCK_DROPS: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_LIVE: AtomicU64 = AtomicU64::new(0);

    let _ord_lock = TERMINAL_DIMENSION_ORD_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _reset = TerminalDimensionOrdReset;
    FORBID_TERMINAL_DIMENSION_ORD.store(false, Ordering::SeqCst);
    BLOCK_DROPS.store(0, Ordering::SeqCst);
    OBSERVED_ACTIVE.store(0, Ordering::SeqCst);
    OBSERVED_LIVE.store(0, Ordering::SeqCst);

    type OrderingOperation<'inv> = RetainedOperation<
        'inv,
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TerminalOrderingDimension,
        u32,
        PrimaryBindDropObservation,
        u64,
    >;
    type OrderingPublication<'inv> = RetainedPublicationAssembly<
        'inv,
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TerminalOrderingDimension,
        u32,
        PrimaryBindDropObservation,
        u64,
    >;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let dimension = TerminalOrderingDimension(7);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(dimension, policy)])
        .unwrap_or_else(|_| panic!("terminal-order policy must be valid"));
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
    let mut invocation_root = InvocationRoot::new();
    let operation = OrderingOperation::try_start(
        &mut invocation_root,
        OperationControl::default(),
        limits,
        &account,
        start,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("terminal-order operation must start"));
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
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
    let mut assembly: OrderingPublication<'_> = operation
        .try_publication_assembly(publication_plan)
        .unwrap_or_else(|_| panic!("terminal-order publication must open"));
    let canonical = assembly
        .loan_mut()
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
        .unwrap_or_else(|_| panic!("terminal-order canonical block must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9_u64, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("terminal-order evidence block must retain"));

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&9)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("terminal-order candidate must validate"),
        };
    let before_physical = account.snapshot();
    FORBID_TERMINAL_DIMENSION_ORD.store(true, Ordering::SeqCst);
    let result = validated.try_prepare(final_progress());
    FORBID_TERMINAL_DIMENSION_ORD.store(false, Ordering::SeqCst);

    let prepared = result.unwrap_or_else(|_| {
        panic!("terminal preparation must succeed without invoking dimension ordering")
    });
    assert_eq!(BLOCK_DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(OBSERVED_ACTIVE.load(Ordering::SeqCst), 0);
    assert_eq!(OBSERVED_LIVE.load(Ordering::SeqCst), 0);
    let after_physical = account.snapshot();
    assert!(!after_physical.poisoned());
    assert!(after_physical.live_retained_bytes() > 0);
    assert!(after_physical.live_retained_allocations() > 0);
    assert_eq!(
        after_physical.total_admitted_retained_bytes(),
        before_physical.total_admitted_retained_bytes()
    );
    assert_eq!(
        after_physical.total_admitted_retained_allocations(),
        before_physical.total_admitted_retained_allocations()
    );
    let published = handoff_for_test(prepared);
    assert!(matches!(
        &published,
        PublishedRetainedPublicationParts::Success { .. }
    ));
    assert_eq!(BLOCK_DROPS.load(Ordering::SeqCst), 0);
    drop(published);
    assert_eq!(BLOCK_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
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
fn evidence_only_binding_panic_stops_before_later_storage_cleanup() {
    let _ord_lock = TERMINAL_DIMENSION_ORD_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _reset = TerminalDimensionOrdReset;
    FORBID_TERMINAL_DIMENSION_ORD.store(false, Ordering::SeqCst);

    type PanicOperation<'inv> = RetainedOperation<
        'inv,
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TerminalOrderingDimension,
        u32,
        u64,
        u64,
    >;
    type PanicPublication<'inv> = RetainedPublicationAssembly<
        'inv,
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TerminalOrderingDimension,
        u32,
        u64,
        u64,
    >;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let dimension = TerminalOrderingDimension(7);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, u64::MAX)
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
    let mut invocation_root = InvocationRoot::new();
    let operation = PanicOperation::try_start(
        &mut invocation_root,
        OperationControl::default(),
        limits,
        &account,
        start,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("combined-fault operation must start"));
    let publication_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer, 1, 1),
        RetainedBlockTablePlan::new(writer, 1, 1),
    );
    let mut assembly: PanicPublication<'_> = operation
        .try_publication_assembly(publication_plan)
        .unwrap_or_else(|_| panic!("combined-fault publication must open"));
    let ledger = assembly
        .loan()
        .operation
        .as_ref()
        .expect("combined-fault operation")
        .common
        .test_ledger()
        .test_shared_probe();
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11_u64, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("combined-fault canonical block must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9_u64, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("combined-fault evidence block must retain"));
    assembly
        .loan_mut()
        .try_commit_canonical(dimension, 1)
        .unwrap_or_else(|_| panic!("canonical rejection commitment must fit the batch"));
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
    panic_next_evidence_only_primary_bind();
    let result = validated.try_prepare(final_progress());

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
    let mut invocation_root: TestInvocationRoot<DropCounter, PanickingDropProbe> =
        InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 4);
    let mut assembly: TestPublication<'_, DropCounter, PanickingDropProbe> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("success rejection assembly must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking evidence must retain"));
    assembly
        .loan_mut()
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let probe = assembly
        .loan()
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .common
        .test_ledger()
        .test_shared_probe();
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
    let mut invocation_root: TestInvocationRoot<PanickingDropProbe, DropCounter> =
        InvocationRoot::new();
    let operation = test_operation_on_account_with_control(
        &mut invocation_root,
        &account,
        4,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<'_, PanickingDropProbe, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("success rejection assembly must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking canonical must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
    assembly
        .loan_mut()
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let probe = assembly
        .loan()
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .common
        .test_ledger()
        .test_shared_probe();
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
    let mut invocation_root: TestInvocationRoot<PanickingDropProbe, DropCounter> =
        InvocationRoot::new();
    let operation = test_operation_with_failure_work(
        &mut invocation_root,
        &factory,
        64,
        RetainedBatchLimits::new(2, 2),
        1,
    );
    let account = operation
        .loan()
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("rich-both account share must be admitted"));
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
    let mut assembly: TestPublication<'_, PanickingDropProbe, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    DROP_COUNT.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let _canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&DROP_COUNT), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking block must retain"));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root = InvocationRoot::new();
    let operation: RetainedOperation<
        '_,
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        DeferredLiveDropObservation,
        u64,
        u64,
    > = RetainedOperation::try_start(
        &mut invocation_root,
        OperationControl::default(),
        limits,
        &account,
        plan,
        || Ok(fallback),
    )
    .unwrap_or_else(|_| panic!("baseline observation operation must start"));
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
    EXPECTED_ACTIVE.store(ledger.active_reservations(), Ordering::SeqCst);
    EXPECTED_OPERATION_LIVE.store(ledger.current_live_bytes(), Ordering::SeqCst);
    let physical = operation.loan().account().snapshot();
    EXPECTED_PHYSICAL_BYTES.store(physical.live_retained_bytes(), Ordering::SeqCst);
    EXPECTED_PHYSICAL_ALLOCATIONS.store(physical.live_retained_allocations(), Ordering::SeqCst);
    *lock_recovering_poison(&deferred_ledger) = Some(ledger);

    drop(operation);
    drop(invocation_root);
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
    let mut invocation_root: TestInvocationRoot<PanickingDropProbe, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    let mut assembly: TestPublication<'_, PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("prepared drop assembly must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&PREPARED_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("prepared drop block must retain"));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root_2: TestInvocationRoot<PanickingDropProbe, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root_2, &factory, 64);
    let mut assembly: TestPublication<'_, PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("published drop assembly must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&PUBLISHED_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("published drop block must retain"));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root_3: TestInvocationRoot<
        TestCanonicalBlock,
        TestEvidenceBlock,
        PanickingDropProbe,
    > = InvocationRoot::new();
    let operation = test_operation_with_fallback(
        &mut invocation_root_3,
        &factory,
        PanickingDropProbe(&BASELINE_DROPS),
    );
    drop(operation);
    assert!(catch_unwind(AssertUnwindSafe(|| drop(invocation_root_3))).is_ok());
    assert_eq!(BASELINE_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn healthy_prepared_and_published_owners_release_during_unrelated_host_unwind() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static CANONICAL_BYTES: AtomicU64 = AtomicU64::new(0);
    static CANONICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_BYTES: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    for publish in [false, true] {
        for counter in [
            &CANONICAL_DROPS,
            &CANONICAL_BYTES,
            &CANONICAL_ALLOCATIONS,
            &EVIDENCE_DROPS,
            &EVIDENCE_BYTES,
            &EVIDENCE_ALLOCATIONS,
        ] {
            counter.store(0, Ordering::SeqCst);
        }
        let factory = account_factory(1);
        let mut invocation_root: TestInvocationRoot<
            PhysicalDropObservation,
            PhysicalDropObservation,
        > = InvocationRoot::new();
        let operation = test_operation(&mut invocation_root, &factory, 64);
        let account = operation
            .loan()
            .account()
            .share_after_test_admission()
            .unwrap();
        let canonical_charge = account
            .checked_retained_charge::<PhysicalDropObservation>(empty_plan())
            .unwrap();
        let canonical_table_charge = account
            .test_checked_retained_table_charge::<PhysicalDropObservation>(1)
            .unwrap();
        let mut assembly: TestPublication<'_, PhysicalDropObservation, PhysicalDropObservation> =
            operation
                .try_publication_assembly(publication_plan(1, 1))
                .unwrap_or_else(|_| panic!("host-unwind publication must open"));
        let canonical = assembly
            .loan_mut()
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(
                    PhysicalDropObservation {
                        account: account.share_after_test_admission().unwrap(),
                        drop_count: &CANONICAL_DROPS,
                        observed_bytes: &CANONICAL_BYTES,
                        observed_allocations: &CANONICAL_ALLOCATIONS,
                    },
                    CanonicalRootTag,
                ))
            })
            .unwrap_or_else(|_| panic!("host-unwind canonical root must retain"));
        let evidence = assembly
            .loan_mut()
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(
                    PhysicalDropObservation {
                        account: account.share_after_test_admission().unwrap(),
                        drop_count: &EVIDENCE_DROPS,
                        observed_bytes: &EVIDENCE_BYTES,
                        observed_allocations: &EVIDENCE_ALLOCATIONS,
                    },
                    EvidenceRootTag,
                ))
            })
            .unwrap_or_else(|_| panic!("host-unwind evidence root must retain"));
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence).is_some()
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("host-unwind publication must validate"),
            };
        let prepared = validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("host-unwind publication must prepare"));
        let charged = account.snapshot();
        assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 0);
        assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 0);

        let result = if publish {
            let prepared = match prepared.into_outcome() {
                PreparedRetainedPublicationOutcome::Success(prepared) => prepared,
                _ => panic!("host-unwind publication must prepare success"),
            };
            let published = prepared.handoff();
            catch_unwind(AssertUnwindSafe(move || {
                let _published = published;
                panic!("unrelated host panic");
            }))
        } else {
            catch_unwind(AssertUnwindSafe(move || {
                let _prepared = prepared;
                panic!("unrelated host panic");
            }))
        };
        assert_eq!(
            result.unwrap_err().downcast_ref::<&str>(),
            Some(&"unrelated host panic")
        );
        assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(
            CANONICAL_BYTES.load(Ordering::SeqCst),
            charged.live_retained_bytes()
        );
        assert_eq!(
            CANONICAL_ALLOCATIONS.load(Ordering::SeqCst),
            charged.live_retained_allocations()
        );
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
    let mut invocation_root: TestInvocationRoot<PhysicalDropObservation, u64> =
        InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    let observed_account = operation
        .loan()
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    let mut assembly: TestPublication<'_, PhysicalDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("prepared observation assembly must open"));
    let canonical = assembly
        .loan_mut()
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
        .loan_mut()
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
    let mut invocation_root_2: TestInvocationRoot<PhysicalDropObservation, u64> =
        InvocationRoot::new();
    let operation = test_operation(&mut invocation_root_2, &factory, 64);
    let observed_account = operation
        .loan()
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    let mut assembly: TestPublication<'_, PhysicalDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("handoff observation assembly must open"));
    let canonical = assembly
        .loan_mut()
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
        .loan_mut()
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
        let mut invocation_root: TestInvocationRoot<u64, u64, PanickingRichFallback> =
            InvocationRoot::new();
        let operation = {
            let account = account(&factory, u64::MAX, 128);
            test_operation_on_account_with_failure_work(
                &mut invocation_root,
                &account,
                64,
                RetainedBatchLimits::new(2, 2),
                OperationControl::default(),
                1,
                PanickingRichFallback,
            )
        };
        let account = operation
            .loan()
            .account()
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("rich fallback account share must be admitted"));
        let ledger = operation.loan().common.test_ledger().test_shared_probe();
        let validated = if includes_canonical_discard {
            let mut assembly = operation
                .try_publication_assembly(publication_plan(1, 1))
                .unwrap_or_else(|_| panic!("rich-both fallback fixture must open"));
            let canonical = assembly
                .loan_mut()
                .try_retain_canonical(empty_plan(), |emplacer| {
                    Ok(emplacer.place(11_u64, CanonicalRootTag))
                })
                .unwrap_or_else(|_| panic!("rich-both canonical root must retain"));
            let evidence = assembly
                .loan_mut()
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
                .loan_mut()
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

    type PanicFallbackOperation<'inv> = RetainedOperation<
        'inv,
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        PanickingFallback,
        u64,
        u64,
    >;
    type PanicFallbackPublication<'inv> = RetainedPublicationAssembly<
        'inv,
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
    let mut invocation_root = InvocationRoot::new();
    let operation = PanicFallbackOperation::try_start(
        &mut invocation_root,
        OperationControl::default(),
        limits,
        &account,
        plan,
        || Ok(PanickingFallback),
    )
    .unwrap_or_else(|_| panic!("panic fallback operation must start"));
    let mut assembly: PanicFallbackPublication<'_> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("panic fallback assembly must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(41, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical block must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence block must retain"));
    let probe = assembly
        .loan()
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .common
        .test_ledger()
        .test_shared_probe();
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
    let mut invocation_root: TestInvocationRoot<u64, PanickingEvidence> = InvocationRoot::new();
    let operation = test_operation_with_failure_work(
        &mut invocation_root,
        &factory,
        4,
        RetainedBatchLimits::new(2, 2),
        1,
    );
    let account = operation
        .loan()
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("minimal-evidence account share must be admitted"));
    let ledger = operation.loan().common.test_ledger().test_shared_probe();
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<'_, u64, PanickingEvidence> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("rejected primary assembly must open"));
    DROP_COUNT.store(0, Ordering::SeqCst);
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingEvidence, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking evidence must retain"));
    assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    let invalid_plan = publication_plan(0, 1);
    let invalid = match operation.try_publication_assembly(invalid_plan) {
        Ok(_) => panic!("zero-slot table must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        invalid.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let operation = expect_retry_operation(invalid, "invalid plan");

    fail_next_final_owner_allocation();
    let allocation = match operation.try_publication_assembly(publication_plan(1, 1)) {
        Ok(_) => panic!("injected table allocation must fail"),
        Err(error) => error,
    };
    assert!(matches!(
        allocation.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = expect_retry_operation(allocation, "verified allocation failure");
    let assembly: TestPublication<'_, u64, u64> = operation
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("retry assembly must open"));
    let probe = assembly
        .loan()
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .common
        .test_ledger()
        .test_shared_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();
    let before_physical = assembly
        .loan()
        .fields
        .account
        .as_ref()
        .expect("assembly retains account")
        .snapshot();

    fail_next_final_owner_allocation();
    assert!(matches!(
        assembly
            .loan_mut()
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(11, CanonicalRootTag))
            }),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
        ))
    ));
    let after_physical = assembly
        .loan()
        .fields
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
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical retry must retain"));
    let evidence = assembly
        .loan_mut()
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
    let mut invocation_root: TestInvocationRoot<u64, u64> = InvocationRoot::new();
    let operation = test_operation(&mut invocation_root, &factory, 64);
    let mut assembly: TestPublication<'_, u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical root must retain"));
    let evidence = assembly
        .loan_mut()
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
