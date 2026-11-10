use super::*;
use crate::concrete_store::activation::FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE;
use crate::{AccountPoisonReason, StorePrepareError, StoreRuntimeError};

#[test]
fn finite_postdetach_rich_and_minimal_errors_own_original_freeze_metadata_and_live_cover() {
    for rich_failure in [true, false] {
        let factory = test_factory();
        let account = test_account(&factory);
        let probe = Arc::new(ControlProbe::default());
        let calls = WorkCalls::default();
        let drops = Rc::new(RefCell::new(Vec::new()));
        let expected = ControlCause::Cancelled {
            origin: CancellationOrigin::Observer,
        };
        let (mut error, dimensions, frozen_counts, failed_account, cover) = {
            let mut host = Host::new();
            let (completion, dimensions) = candidate_completion(
                &mut host,
                &account,
                probe.control(),
                (
                    Payload::new("output", 41, &drops),
                    Payload::new("proof", 43, &drops),
                ),
                &calls,
                64,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("postdetach failure must consume the actual centrally validated candidate");
            };
            let before_issue = probe.counts();
            probe.cancel_on_progress.store(true, Ordering::SeqCst);
            let FiniteCanonicalFreezePermitIssue::Interrupted(rooted) = candidate
                .try_issue_freeze_permit(completion.generation.interruption, progress())
                .unwrap()
            else {
                panic!("the final observer must produce the original Frozen interruption");
            };
            assert_eq!(
                rooted.cause().class(),
                CanonicalFreezeInterruptionClass::Control,
            );
            assert_eq!(rooted.cause().control_cause(), Some(&expected));
            assert_eq!(*drops.borrow(), ["proof", "output"]);
            let frozen_counts = probe.counts();
            assert_eq!(frozen_counts[1], before_issue[1] + 1);
            assert_eq!(&frozen_counts[2..], [1, 1]);
            let admitted = account.snapshot();
            assert!(!admitted.poisoned());
            let mut error = if rich_failure {
                let mut validations = 0;
                let RootedFailureValidation::Valid(validated) = rooted
                    .validate_rich_failure(completion.failure_fallback, |session, root| {
                        validations += 1;
                        validate_evidence(session, root)
                    })
                    .unwrap()
                else {
                    panic!("rich preparation must validate the original staged evidence");
                };
                assert_eq!(validations, 1);
                assert_eq!(validated.cause().control_cause(), Some(&expected));
                assert_eq!(probe.counts(), frozen_counts);
                crate::retained::reject_next_account_bound_commit();
                validated
                    .try_prepare_frozen()
                    .err()
                    .expect("actual account eligibility must reject the detached rich commit")
            } else {
                crate::retained::reject_next_account_bound_commit();
                rooted
                    .prepare_minimal_fallback()
                    .err()
                    .expect("actual account eligibility must reject the detached minimal commit")
            };
            assert!(error.has_residue());
            let domain = error
                .domain()
                .expect("the owned error retains its original domain");
            assert!(
                dimensions
                    .iter()
                    .all(|dimension| domain.owns_dimension(*dimension))
            );
            assert_eq!(error.original_freeze_control(), Some(&expected));
            let cover = error
                .test_uncommitted_cover_observation()
                .expect("the normal Commit error must own its actual detached numerical proof");
            assert!(cover.0 > 0);
            assert!(cover.1 > 0);
            assert!(cover.2 > 0);
            assert_ne!(cover.3, 0);
            let failed_account = account.snapshot();
            assert!(failed_account.poisoned());
            assert!(failed_account.live_retained_bytes() > 0);
            assert!(failed_account.live_retained_allocations() > 0);
            assert_eq!(
                failed_account.total_admitted_retained_bytes(),
                admitted.total_admitted_retained_bytes(),
            );
            assert_eq!(
                failed_account.total_admitted_retained_allocations(),
                admitted.total_admitted_retained_allocations(),
            );
            assert_eq!(probe.counts(), frozen_counts);
            assert_eq!(calls.counts(), [1, 1, 1, 1]);
            (error, dimensions, frozen_counts, failed_account, cover)
        };
        assert!(error.has_residue());
        assert_eq!(error.original_freeze_control(), Some(&expected));
        assert_eq!(error.test_uncommitted_cover_observation(), Some(cover));
        let domain = error
            .domain()
            .expect("Host destruction must preserve the original domain");
        assert!(
            dimensions
                .iter()
                .all(|dimension| domain.owns_dimension(*dimension))
        );
        let domain_identity = std::ptr::from_ref(domain);
        let primary_cause = format!("{:?}", error.cause());
        let FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Publication(
            StorePrepareError::FinalizationContainment(cause),
        )) = error.cause()
        else {
            panic!("the primary error must remain the real physical publication rejection");
        };
        assert_eq!(
            cause.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        let first_poison = failed_account
            .first_poison_evidence()
            .expect("the rejected publication must retain its first account poison evidence");
        assert_eq!(
            first_poison.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        assert_eq!(first_poison.event_ordinal(), 1);
        assert_eq!(
            cause.account_poisoned().snapshot().first_poison_evidence(),
            Some(first_poison),
        );
        assert_eq!(account.snapshot(), failed_account);
        error.attach_original_freeze_control(Some(ControlCause::DeadlineExceeded));
        error.attach_original_freeze_control(None);
        assert_eq!(error.original_freeze_control(), Some(&expected));
        assert_eq!(format!("{:?}", error.cause()), primary_cause);
        assert!(error.has_residue());
        assert!(std::ptr::eq(error.domain().unwrap(), domain_identity));
        assert_eq!(error.test_uncommitted_cover_observation(), Some(cover));
        assert_eq!(account.snapshot(), failed_account);
        assert_eq!(probe.counts(), frozen_counts);
        let mut moved_error = error;
        assert_eq!(moved_error.original_freeze_control(), Some(&expected));
        assert_eq!(format!("{:?}", moved_error.cause()), primary_cause);
        assert_eq!(
            moved_error.test_uncommitted_cover_observation(),
            Some(cover)
        );
        assert!(
            dimensions
                .iter()
                .all(|dimension| { moved_error.domain().unwrap().owns_dimension(*dimension) })
        );
        drop(moved_error);
        assert_eq!(account.snapshot(), failed_account);
        assert_eq!(
            account.snapshot().first_poison_evidence(),
            Some(first_poison)
        );
        assert_eq!(probe.counts(), frozen_counts);
        assert_eq!(calls.counts(), [1, 1, 1, 1]);
        assert_eq!(*drops.borrow(), ["proof", "output"]);
        assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
    }
}

#[test]
fn finite_frozen_validation_runtime_replacement_preserves_independent_freeze_metadata() {
    let factory = test_factory();
    let account = test_account(&factory);
    let probe = Arc::new(ControlProbe::default());
    let calls = WorkCalls::default();
    let drops = Rc::new(RefCell::new(Vec::new()));
    let expected = ControlCause::Cancelled {
        origin: CancellationOrigin::Observer,
    };
    let (fallback, dimensions, frozen_counts) = {
        let mut host = Host::new();
        let (completion, dimensions) = candidate_completion(
            &mut host,
            &account,
            probe.control(),
            (
                Payload::new("output", 41, &drops),
                Payload::new("proof", 43, &drops),
            ),
            &calls,
            64,
            false,
        );
        let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
            panic!("runtime replacement must start with the actual finite candidate");
        };
        let before_issue = probe.counts();
        probe.cancel_on_progress.store(true, Ordering::SeqCst);
        let FiniteCanonicalFreezePermitIssue::Interrupted(rooted) = candidate
            .try_issue_freeze_permit(completion.generation.interruption, progress())
            .unwrap()
        else {
            panic!("the original final observer must interrupt the finite freeze");
        };
        assert_eq!(rooted.cause().control_cause(), Some(&expected));
        let frozen_counts = probe.counts();
        assert_eq!(frozen_counts[1], before_issue[1] + 1);
        assert_eq!(&frozen_counts[2..], [1, 1]);
        let admitted = account.snapshot();
        let mut validations = 0;
        let RootedFailureValidation::Fallback(fallback) = rooted
            .validate_rich_failure(completion.failure_fallback, |_, _| {
                validations += 1;
                true
            })
            .unwrap()
        else {
            panic!("missing validation work must replace the source with its real runtime cause");
        };
        assert_eq!(validations, 1);
        assert!(matches!(
            fallback.cause(),
            StoreRuntimeError::Build(cause)
                if cause.code() == FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE
        ));
        assert_eq!(probe.counts(), frozen_counts);
        assert_eq!(calls.counts(), [1, 1, 1, 1]);
        assert_eq!(
            account.snapshot().total_admitted_retained_bytes(),
            admitted.total_admitted_retained_bytes(),
        );
        assert_eq!(
            account.snapshot().total_admitted_retained_allocations(),
            admitted.total_admitted_retained_allocations(),
        );
        (fallback, dimensions, frozen_counts)
    };
    let (prepared, cause) = fallback.into_parts();
    assert!(matches!(
        cause,
        StoreRuntimeError::Build(cause)
            if cause.code() == FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE
    ));
    let parts = prepared.handoff();
    assert_eq!(parts.facts.original_freeze_control(), Some(&expected));
    assert!(parts.terminal_account.is_same_account(&account));
    assert!(
        matches!(parts.facts.header().terminal_cause(), StoreTerminalCause::Failure(cause)
        if cause.class() == StorePreparationFailureClass::Control && cause.code() == 1)
    );
    for dimension in dimensions {
        assert!(parts.facts.dimension(dimension).is_ok());
    }
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[2])
            .unwrap()
            .committed_canonical_output(),
        0,
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        3,
    );
    assert_eq!(probe.counts(), frozen_counts);
    drop(parts);
    assert_eq!(probe.counts(), frozen_counts);
    assert_eq!(calls.counts(), [1, 1, 1, 1]);
    assert_eq!(*drops.borrow(), ["proof", "output"]);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
}

#[test]
fn finite_fixture_cause_rebind_preserves_frozen_postdetach_error_through_cleanup_fault() {
    use crate::PhysicalStorageAccountSnapshot;
    use crate::concrete_store::activation::payload_owner::{
        PayloadLoan, PayloadOrder, PayloadOwner,
    };
    use crate::concrete_store::activation::{
        CanonicalFinal, CanonicalFreezeInterruption, FrozenFailureState, ValidatedFailureEvidence,
    };

    struct FaultingCause<'probe> {
        original: CanonicalFreezeInterruption,
        account: &'probe PhysicalStorageAccount,
        observations: &'probe RefCell<Vec<(PhysicalStorageAccountSnapshot, Option<ControlCause>)>>,
    }

    impl Drop for FaultingCause<'_> {
        fn drop(&mut self) {
            self.observations.borrow_mut().push((
                self.account.snapshot(),
                self.original.control_cause().cloned(),
            ));
            panic!("fixture-only cause cleanup fault after the actual finite commit rejection");
        }
    }

    let factory = test_factory();
    let account = test_account(&factory);
    let probe = Arc::new(ControlProbe::default());
    let calls = WorkCalls::default();
    let drops = Rc::new(RefCell::new(Vec::new()));
    let observations = RefCell::new(Vec::new());
    let expected = ControlCause::Cancelled {
        origin: CancellationOrigin::Observer,
    };
    let (mut error, dimensions, frozen_counts, failed_account, cover) = {
        let mut host = Host::new();
        let result = {
            let (completion, dimensions) = candidate_completion(
                &mut host,
                &account,
                probe.control(),
                (
                    Payload::new("output", 41, &drops),
                    Payload::new("proof", 43, &drops),
                ),
                &calls,
                64,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!(
                    "the cleanup witness must start with the actual centrally validated candidate"
                );
            };
            let before_issue = probe.counts();
            probe.cancel_on_progress.store(true, Ordering::SeqCst);
            let FiniteCanonicalFreezePermitIssue::Interrupted(rooted) = candidate
                .try_issue_freeze_permit(completion.generation.interruption, progress())
                .unwrap()
            else {
                panic!("the original final observer must produce the Frozen interruption");
            };
            assert_eq!(
                rooted.cause().class(),
                CanonicalFreezeInterruptionClass::Control
            );
            assert_eq!(rooted.cause().control_cause(), Some(&expected));
            assert_eq!(*drops.borrow(), ["proof", "output"]);
            let frozen_counts = probe.counts();
            assert_eq!(frozen_counts[1], before_issue[1] + 1);
            assert_eq!(&frozen_counts[2..], [1, 1]);
            let mut validations = 0;
            let RootedFailureValidation::Valid(validated) = rooted
                .validate_rich_failure(completion.failure_fallback, |session, root| {
                    validations += 1;
                    validate_evidence(session, root)
                })
                .unwrap()
            else {
                panic!("the cleanup witness must retain the actual validated failure storage");
            };
            assert_eq!(validations, 1);
            assert_eq!(validated.cause().control_cause(), Some(&expected));
            let original_code = validated.cause().stable_code();
            let (original_cause, original_storage) = validated.state.owner.into_parts();
            let original_identity = original_cause.identity();
            let mut faulting_cause = Some(FaultingCause {
                original: original_cause.into_owned(),
                account: &account,
                observations: &observations,
            });
            let order = Cell::new(PayloadOrder::ProofBeforeOutput);
            let rebound = CanonicalFinal::new(ValidatedFailureEvidence {
                owner: PayloadOwner::new(
                    PayloadLoan::new(&mut faulting_cause, &order, original_identity),
                    original_storage,
                ),
                _lifecycle: PhantomData::<FrozenFailureState>,
            });
            assert_eq!(
                rebound.state.owner.value().identity(),
                original_identity,
                "fixture-only cause rebind keeps the original identity and actual validated storage",
            );
            assert_eq!(rebound.cause().original.stable_code(), original_code);
            assert_eq!(rebound.cause().original.control_cause(), Some(&expected));
            assert!(observations.borrow().is_empty());
            assert_eq!(probe.counts(), frozen_counts);
            let admitted = account.snapshot();
            assert!(!admitted.poisoned());
            crate::retained::reject_next_account_bound_commit();
            let mut error = rebound
                .try_prepare_frozen()
                .err()
                .expect("the actual detached commit error must survive the cause cleanup panic");
            assert!(faulting_cause.is_none());
            assert_eq!(observations.borrow().len(), 1);
            let observed_account = {
                let observed = observations.borrow();
                assert_eq!(observed[0].1.as_ref(), Some(&expected));
                observed[0].0
            };
            assert!(observed_account.poisoned());
            let first_poison = observed_account.first_poison_evidence().unwrap();
            assert_eq!(
                first_poison.reason(),
                AccountPoisonReason::OwnershipGraphUnverified
            );
            assert_eq!(first_poison.event_ordinal(), 1);
            let FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Publication(
                StorePrepareError::FinalizationContainment(cause),
            )) = error.cause()
            else {
                panic!("cause cleanup must preserve the installed physical publication rejection");
            };
            assert_eq!(
                cause.reason(),
                AccountPoisonReason::OwnershipGraphUnverified
            );
            assert_eq!(
                cause.account_poisoned().snapshot().first_poison_evidence(),
                Some(first_poison),
            );
            assert!(error.has_residue());
            assert_eq!(error.original_freeze_control(), Some(&expected));
            assert!(
                dimensions
                    .iter()
                    .all(|dimension| error.domain().unwrap().owns_dimension(*dimension))
            );
            let cover = error.test_uncommitted_cover_observation().expect(
                "the original Commit error must retain its actual detached numerical proof",
            );
            assert!(cover.0 > 0 && cover.1 > 0 && cover.2 > 0);
            assert_ne!(cover.3, 0);
            let failed_account = account.snapshot();
            assert!(failed_account.live_retained_bytes() > 0);
            assert!(failed_account.live_retained_allocations() > 0);
            assert_eq!(failed_account.first_poison_evidence(), Some(first_poison));
            assert_eq!(
                failed_account.live_retained_bytes(),
                observed_account.live_retained_bytes(),
            );
            assert_eq!(
                failed_account.live_retained_allocations(),
                observed_account.live_retained_allocations(),
            );
            assert_eq!(
                failed_account.total_admitted_retained_bytes(),
                admitted.total_admitted_retained_bytes(),
            );
            assert_eq!(
                failed_account.total_admitted_retained_allocations(),
                admitted.total_admitted_retained_allocations(),
            );
            assert_eq!(probe.counts(), frozen_counts);
            assert_eq!(calls.counts(), [1, 1, 1, 1]);
            (error, dimensions, frozen_counts, failed_account, cover)
        };
        assert!(host.freeze_interrupted.is_none());
        result
    };
    assert!(error.has_residue());
    assert_eq!(error.original_freeze_control(), Some(&expected));
    assert_eq!(error.test_uncommitted_cover_observation(), Some(cover));
    assert_eq!(account.snapshot(), failed_account);
    let domain_identity = std::ptr::from_ref(error.domain().unwrap());
    let primary_cause = format!("{:?}", error.cause());
    error.attach_original_freeze_control(Some(ControlCause::DeadlineExceeded));
    error.attach_original_freeze_control(None);
    assert_eq!(error.original_freeze_control(), Some(&expected));
    assert_eq!(format!("{:?}", error.cause()), primary_cause);
    assert!(std::ptr::eq(error.domain().unwrap(), domain_identity));
    let mut moved_error = error;
    assert!(moved_error.has_residue());
    assert_eq!(moved_error.original_freeze_control(), Some(&expected));
    assert_eq!(format!("{:?}", moved_error.cause()), primary_cause);
    assert_eq!(
        moved_error.test_uncommitted_cover_observation(),
        Some(cover)
    );
    assert!(
        dimensions
            .iter()
            .all(|dimension| moved_error.domain().unwrap().owns_dimension(*dimension))
    );
    drop(moved_error);
    assert_eq!(account.snapshot(), failed_account);
    assert_eq!(observations.borrow().len(), 1);
    assert_eq!(probe.counts(), frozen_counts);
    assert_eq!(calls.counts(), [1, 1, 1, 1]);
    assert_eq!(*drops.borrow(), ["proof", "output"]);
    assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
}
