use super::*;
use baozi_operation::__canonical_activation::{
    CanonicalFreezeInterruptionClass, CanonicalFreezePermitIssue,
};
use baozi_operation::StorePreparationFailureClass;

const GENERATION_REJECTION_CODE: u32 = 0x4745_4e45;
const LEDGER_LIMIT_CODE: u32 = 1;
const CANCELLATION_CODE: u32 = 1;

type TestCanonicalCandidate =
    baozi_operation::__canonical_activation::CanonicalCandidateStoreSession<
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        ScalarSequence<u8>,
        SequenceTag,
        ScalarSequence<u8>,
        EvidenceLeafTag,
    >;

fn candidate(account: &PhysicalStorageAccount) -> (TestCanonicalCandidate, TestKeys) {
    candidate_with_operation(operation(account))
}

fn candidate_with_control(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
) -> (TestCanonicalCandidate, TestKeys) {
    candidate_with_operation(operation_with_control(account, control))
}

fn candidate_with_operation(
    (operation, keys): (TestOperation, TestKeys),
) -> (TestCanonicalCandidate, TestKeys) {
    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap()),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
                .unwrap();
            lane.complete::<_, (), _, _>(root, ())
        },
    )
    .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("the exact candidate must activate"),
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        let mut canonical_value = None;
                        let mut evidence_value = None;
                        assert!(session.visit_canonical_sequence(
                            canonical,
                            1,
                            |_index, value| {
                                canonical_value = Some(value);
                                true
                            }
                        )?);
                        assert!(session.visit_outcome_evidence_sequence(
                            evidence,
                            1,
                            |_index, value| {
                                evidence_value = Some(value);
                                true
                            }
                        )?);
                        assert_eq!(canonical_value, Some(7));
                        assert_eq!(evidence_value, Some(9));
                        Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
                    },
                )
            },
        )
        .unwrap();
    let candidate = match outcome {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("the exact central closure must produce a candidate")
        }
    };
    (candidate, keys)
}

fn ready_permit(
    candidate: TestCanonicalCandidate,
) -> baozi_operation::__canonical_activation::CanonicalFreezePermit<
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    (),
    ScalarSequence<u8>,
    SequenceTag,
    ScalarSequence<u8>,
    EvidenceLeafTag,
> {
    match candidate.try_issue_freeze_permit(final_progress()).unwrap() {
        CanonicalFreezePermitIssue::Ready(permit) => permit,
        CanonicalFreezePermitIssue::Interrupted(_) => panic!("final progress must remain open"),
        CanonicalFreezePermitIssue::DestinationRejected(_) => {
            panic!("exact destinations must remain admissible")
        }
    }
}

#[test]
fn permit_stages_exact_destinations_without_publishing_counters() {
    let factory = factory(1);
    let account = account(&factory);
    let (candidate, _keys) = candidate(&account);
    let permit = ready_permit(candidate);

    assert!(permit.destination_counters_are_zero());
    drop(permit);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn scoped_joint_commit_prepares_and_commits_validated_counts() {
    let factory = factory(1);
    let account = account(&factory);
    let (candidate, keys) = candidate(&account);
    let permit = ready_permit(candidate);

    let (output, proof, prepared) = permit.bind_joint_commit(|output, proof, joint, seal| {
        (output, proof, joint.prepare(seal).unwrap())
    });
    assert_eq!(output, ());
    assert_eq!(proof, ());
    let prepared = match prepared {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        PreparedStoreSuccessOutcome::RichFailure(_)
        | PreparedStoreSuccessOutcome::MinimalFallback(_) => {
            panic!("the frozen success must remain successful")
        }
    };
    let published = prepared.handoff();
    let dimension = published.ledger_evidence.dimension(keys.dimension).unwrap();
    assert_eq!(dimension.pending_publication(), 0);
    assert_eq!(dimension.committed(), 1);
    assert_eq!(dimension.committed_canonical_output(), 1);
    assert_eq!(dimension.published_outcome_evidence(), 2);
    assert_eq!(published.canonical.len(), 1);
    assert_eq!(published.outcome_evidence.len(), 1);
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn dropping_scoped_joint_commit_releases_the_physical_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let (candidate, _keys) = candidate(&account);
    let permit = ready_permit(candidate);

    permit.bind_joint_commit(|_output, _proof, joint, seal| {
        drop(seal);
        drop(joint);
    });
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn generation_rejection_discards_canonical_and_preserves_rich_evidence() {
    let factory = factory(1);
    let account = account(&factory);
    let (candidate, keys) = candidate(&account);
    let permit = ready_permit(candidate);
    let rejected = permit
        .reject_generation(ValidationRejection::Semantic, GENERATION_REJECTION_CODE)
        .unwrap();
    let validated = match rejected
        .validate_rich_failure(|session, evidence| {
            session
                .visit_sequence(evidence, |_index, value| value == 9)
                .unwrap_or(false)
        })
        .unwrap()
    {
        RootedFailureValidation::Valid(validated) => validated,
        RootedFailureValidation::Rejected(_) | RootedFailureValidation::Fallback(_) => {
            panic!("preserved evidence must validate")
        }
    };
    let (prepared, _cause) = validated.try_prepare_frozen().unwrap().into_parts();
    let prepared = match prepared {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("generation rejection must retain rich evidence")
        }
    };
    let published = prepared.handoff();
    let terminal = store_failure_cause(&published.ledger_evidence);
    assert_eq!(terminal.class(), StorePreparationFailureClass::Requested);
    assert_eq!(terminal.code(), GENERATION_REJECTION_CODE);
    let dimension = published.ledger_evidence.dimension(keys.dimension).unwrap();
    assert_eq!(dimension.committed_canonical_output(), 0);
    assert_eq!(dimension.published_outcome_evidence(), 2);
    assert_eq!(published.outcome_evidence.outcome_evidence().len(), 1);
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn destination_rejection_preserves_ordinary_failure_preparation() {
    let factory = factory(1);
    let account = account(&factory);
    let (candidate, keys) = candidate_with_operation(operation_with_commit_limit(&account, 0));
    let rejected = match candidate.try_issue_freeze_permit(final_progress()).unwrap() {
        CanonicalFreezePermitIssue::Ready(_) => {
            panic!("the committed-output limit must reject the destination")
        }
        CanonicalFreezePermitIssue::Interrupted(_) => {
            panic!("destination admission is not a frozen interruption")
        }
        CanonicalFreezePermitIssue::DestinationRejected(rejected) => rejected,
    };
    assert_eq!(
        rejected.cause().class(),
        CanonicalFreezeInterruptionClass::Ledger
    );
    assert_eq!(rejected.cause().stable_code(), LEDGER_LIMIT_CODE);
    let validated = match rejected
        .validate_rich_failure(|session, evidence| {
            session
                .visit_sequence(evidence, |_index, value| value == 9)
                .unwrap_or(false)
        })
        .unwrap()
    {
        RootedFailureValidation::Valid(validated) => validated,
        RootedFailureValidation::Rejected(_) | RootedFailureValidation::Fallback(_) => {
            panic!("destination rejection must preserve rich evidence")
        }
    };
    let (prepared, _cause) = validated
        .try_prepare(final_progress())
        .unwrap()
        .into_parts();
    let prepared = match prepared {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("destination rejection must remain a rich failure")
        }
    };
    let published = prepared.handoff();
    let terminal = store_failure_cause(&published.ledger_evidence);
    assert_eq!(terminal.class(), StorePreparationFailureClass::Ledger);
    assert_eq!(terminal.code(), LEDGER_LIMIT_CODE);
    let dimension = published.ledger_evidence.dimension(keys.dimension).unwrap();
    assert_eq!(dimension.committed_canonical_output(), 0);
    assert_eq!(dimension.published_outcome_evidence(), 2);
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn final_progress_interruption_never_issues_a_success_permit() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (candidate, _keys) = candidate_with_control(&account, control);
    cancellation.cancel();
    let interrupted = match candidate.try_issue_freeze_permit(final_progress()).unwrap() {
        CanonicalFreezePermitIssue::Ready(_) => panic!("cancelled progress cannot issue a permit"),
        CanonicalFreezePermitIssue::Interrupted(interrupted) => interrupted,
        CanonicalFreezePermitIssue::DestinationRejected(_) => {
            panic!("cancellation is not a destination rejection")
        }
    };
    assert_eq!(
        interrupted.cause().class(),
        CanonicalFreezeInterruptionClass::Control
    );
    assert_eq!(interrupted.cause().stable_code(), CANCELLATION_CODE);
    drop(interrupted);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn non_copy_output_remains_contained_before_the_success_binder() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap()),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
                .unwrap();
            lane.complete::<_, (), _, _>(root, PanicOnDrop)
        },
    )
    .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("the exact candidate must activate"),
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        assert!(session.visit_canonical_sequence(
                            canonical,
                            1,
                            |_index, value| value == 7
                        )?);
                        assert!(session.visit_outcome_evidence_sequence(
                            evidence,
                            1,
                            |_index, value| value == 9
                        )?);
                        Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
                    },
                )
            },
        )
        .unwrap();
    let candidate = match outcome {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => panic!("validation must succeed"),
    };
    let permit = match candidate.try_issue_freeze_permit(final_progress()).unwrap() {
        CanonicalFreezePermitIssue::Ready(permit) => permit,
        CanonicalFreezePermitIssue::Interrupted(_) => panic!("final progress must remain open"),
        CanonicalFreezePermitIssue::DestinationRejected(_) => {
            panic!("exact destinations must remain admissible")
        }
    };

    assert!(permit.destination_counters_are_zero());
    let before = account.snapshot();
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(permit)));
    assert!(dropped.is_err());
    let after = account.snapshot();
    assert!(after.poisoned());
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
}

#[test]
fn non_copy_proof_remains_contained_before_the_success_binder() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap()),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
                .unwrap();
            lane.complete::<_, (), _, _>(root, ())
        },
    )
    .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("the exact candidate must activate"),
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        assert!(session.visit_canonical_sequence(
                            canonical,
                            1,
                            |_index, value| value == 7
                        )?);
                        assert!(session.visit_outcome_evidence_sequence(
                            evidence,
                            1,
                            |_index, value| value == 9
                        )?);
                        Ok(CanonicalValidationDecision::<
                            PanicOnDrop,
                            ValidationRejection,
                        >::accept(PanicOnDrop))
                    },
                )
            },
        )
        .unwrap();
    let candidate = match outcome {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => panic!("validation must succeed"),
    };
    let permit = match candidate.try_issue_freeze_permit(final_progress()).unwrap() {
        CanonicalFreezePermitIssue::Ready(permit) => permit,
        CanonicalFreezePermitIssue::Interrupted(_) => panic!("final progress must remain open"),
        CanonicalFreezePermitIssue::DestinationRejected(_) => {
            panic!("exact destinations must remain admissible")
        }
    };

    assert!(permit.destination_counters_are_zero());
    let before = account.snapshot();
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(permit)));
    assert!(dropped.is_err());
    let after = account.snapshot();
    assert!(after.poisoned());
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
}

#[test]
fn scoped_binder_api_is_higher_ranked_invariant_and_copy_gated() {
    let activation = include_str!("../../../src/concrete_store/activation.rs");
    let canonical = include_str!("../../../src/concrete_store/activation/canonical.rs");
    let operation = include_str!("../../../src/concrete_store.rs");

    assert_eq!(
        activation
            .matches("PhantomData<fn(&'joint mut ()) -> &'joint mut ()>")
            .count(),
        2
    );
    assert!(canonical.contains("bind: impl for<'joint> FnOnce("));
    assert!(canonical.contains("T: Copy,"));
    assert!(canonical.contains("Proof: Copy,"));
    assert!(canonical.contains("seal: CanonicalJointSeal<'joint>"));
    let prepare = operation
        .split_once("pub(super) fn prepare(")
        .expect("the operation contribution exposes one crate-private prepare")
        .1
        .split_once("fn try_form_joint_commit_intent(")
        .expect("prepare consumes the private formation before the intent helper")
        .0;
    assert!(prepare.contains("StoreSuccessJointCommitFormation::PreparedFailure("));
    assert!(prepare.contains("PreparedStoreSuccessOutcome::RichFailure(prepared)"));
    assert!(prepare.contains("PreparedStoreSuccessOutcome::MinimalFallback(prepared)"));
    assert!(!prepare.contains("unreachable"));
    assert!(!prepare.contains("panic!"));
}

#[test]
fn obsolete_operation_success_chain_is_absent() {
    let activation = include_str!("../../../src/concrete_store/activation.rs");
    let canonical = include_str!("../../../src/concrete_store/activation/canonical.rs");
    let exports = include_str!("../../../src/lib.rs");
    let combined = [activation, canonical, exports].concat();

    for obsolete in [
        "CanonicalGenerationReady",
        "CanonicalJointCommitIntent",
        "CanonicalJointCommitFormation",
        "CanonicalPreparedGeneration",
        "CanonicalPreparedJointCommitFailure",
        "CanonicalPublishedGeneration",
        "into_generation_ready",
    ] {
        assert!(
            !combined.contains(obsolete),
            "obsolete API remains: {obsolete}"
        );
    }
    for forbidden in ["pub fn attach", "pub fn into_parts", "GenerationToken"] {
        assert!(
            !canonical.contains(forbidden),
            "scoped success API exposes forbidden escape hatch: {forbidden}"
        );
    }
}
