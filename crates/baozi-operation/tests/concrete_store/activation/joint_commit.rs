use super::*;
use baozi_operation::__retained_store_integration::activation::{
    CanonicalFreezeInterruptionClass, CanonicalFreezePermitIssue, CanonicalInvocationOutcome,
};
use baozi_operation::StorePreparationFailureClass;

const GENERATION_REJECTION_CODE: u32 = 0x4745_4e45;
const LEDGER_LIMIT_CODE: u32 = 1;
const CANCELLATION_CODE: u32 = 1;

fn candidate_final_progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 2)
}

type TestCanonicalCandidate<'inv> =
    baozi_operation::__retained_store_integration::activation::CanonicalCandidateStoreSession<
        'inv,
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

type BodyInvocationHost = CanonicalInvocationHost<
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    (),
    (),
    ValidationRejection,
    GenerationRejection,
    ScalarSequence<u8>,
    SequenceTag,
    ScalarSequence<u8>,
    EvidenceLeafTag,
>;

struct BodyCapture<'a> {
    account: &'a PhysicalStorageAccount,
    drops: &'a Cell<u32>,
    retained_at_drop: &'a Cell<bool>,
    panic_on_drop: bool,
}

impl Drop for BodyCapture<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
        let snapshot = self.account.snapshot();
        self.retained_at_drop
            .set(!snapshot.poisoned() && snapshot.live_retained_allocations() > 0);
        assert!(!self.panic_on_drop, "retained body capture panic");
    }
}

#[test]
fn retained_candidate_body_disposes_captures_before_final_control_and_commit() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let factory = factory(1);
    let account = account(&factory);
    let mut host = BodyInvocationHost::new();
    let drops = Cell::new(0);
    let retained_at_drop = Cell::new(false);
    let observer_calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&observer_calls);
    let mut control = Some(OperationControl::default().with_progress_observer(
        move |_: baozi_operation::ProgressEvent<TestProgress, TestUnit>| {
            observed.fetch_add(1, Ordering::SeqCst);
            baozi_operation::ProgressDecision::Continue
        },
    ));
    let capture = BodyCapture {
        account: &account,
        drops: &drops,
        retained_at_drop: &retained_at_drop,
        panic_on_drop: false,
    };
    let mut keys = None;
    let mut before_request = 0;
    let body_account = &account;
    let body_drops = &drops;
    let body_observer = &observer_calls;
    let saved_keys = &mut keys;
    let saved_before_request = &mut before_request;
    {
        let finish = host
            .run_body(move |parts, disposition| {
                let _capture = &capture;
                let (candidate, actual_keys) = candidate_with_operation(
                    operation_with_control(parts.startup, body_account, control.take().unwrap()),
                    parts.output,
                    parts.activation,
                    parts.proof,
                    parts.validation,
                );
                *saved_keys = Some(actual_keys);
                *saved_before_request = body_observer.load(Ordering::SeqCst);
                candidate.request_finish(disposition).unwrap();
                assert_eq!(
                    body_drops.get(),
                    0,
                    "the driver must retain captures through body return"
                );
            })
            .unwrap();
        assert_eq!(drops.get(), 1);
        assert!(retained_at_drop.get());
        assert_eq!(observer_calls.load(Ordering::SeqCst), before_request);
        let baozi_operation::__retained_store_integration::activation::CanonicalInvocationCompletion {
        outcome,
        generation,
        ..
    } = finish.resume();
        let CanonicalInvocationOutcome::Candidate(candidate) = outcome else {
            panic!("the recorded candidate must resume as the same success stage")
        };
        let permit = ready_permit(candidate, generation.interruption);
        assert!(observer_calls.load(Ordering::SeqCst) > before_request);
        let prepared = permit.bind_joint_commit(|joint, seal| joint.prepare(seal).unwrap());
        let CanonicalJointSuccess {
            output: (),
            proof: (),
            store: prepared,
        } = prepared;
        let published = prepared.handoff();
        assert_eq!(
            published
                .ledger_evidence
                .dimension(keys.unwrap().dimension)
                .unwrap()
                .committed_canonical_output(),
            1
        );
        drop(published);
    }
    drop(host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn retained_candidate_body_contains_capture_panic_after_request() {
    let factory = factory(1);
    let account = account(&factory);
    let mut host = BodyInvocationHost::new();
    let drops = Cell::new(0);
    let retained_at_drop = Cell::new(false);
    let capture = BodyCapture {
        account: &account,
        drops: &drops,
        retained_at_drop: &retained_at_drop,
        panic_on_drop: true,
    };
    let body_account = &account;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = host.run_body(move |parts, disposition| {
            let _capture = &capture;
            let (candidate, _) = candidate_with_operation(
                operation(parts.startup, body_account),
                parts.output,
                parts.activation,
                parts.proof,
                parts.validation,
            );
            candidate.request_finish(disposition).unwrap();
        });
    }));
    let payload = result.unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"retained body capture panic")
    );
    assert_eq!(drops.get(), 1);
    assert!(retained_at_drop.get());
    drop(host);
    assert!(account.snapshot().poisoned());
    assert!(account.snapshot().live_retained_allocations() > 0);
}

#[test]
fn retained_candidate_body_keeps_body_panic_when_capture_cleanup_also_panics() {
    let factory = factory(1);
    let account = account(&factory);
    let mut host = BodyInvocationHost::new();
    let drops = Cell::new(0);
    let retained_at_drop = Cell::new(false);
    let capture = BodyCapture {
        account: &account,
        drops: &drops,
        retained_at_drop: &retained_at_drop,
        panic_on_drop: true,
    };
    let body_account = &account;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = host.run_body(move |parts, disposition| {
            let _capture = &capture;
            let (candidate, _) = candidate_with_operation(
                operation(parts.startup, body_account),
                parts.output,
                parts.activation,
                parts.proof,
                parts.validation,
            );
            candidate.request_finish(disposition).unwrap();
            panic!("retained body invocation panic");
        });
    }));
    assert_eq!(
        result.unwrap_err().downcast_ref::<&str>(),
        Some(&"retained body invocation panic")
    );
    assert_eq!(drops.get(), 1);
    assert!(retained_at_drop.get());
    drop(host);
    assert!(account.snapshot().poisoned());
    assert!(account.snapshot().live_retained_allocations() > 0);
}

#[test]
fn retained_candidate_body_rejects_second_admission_after_forgotten_finish() {
    let factory = factory(1);
    let account = account(&factory);
    let mut host = BodyInvocationHost::new();
    let finish = host
        .run_body(|parts, disposition| {
            let (candidate, _) = candidate_with_operation(
                operation(parts.startup, &account),
                parts.output,
                parts.activation,
                parts.proof,
                parts.validation,
            );
            candidate.request_finish(disposition).unwrap();
        })
        .unwrap();
    #[allow(
        clippy::forget_non_drop,
        reason = "the regression deliberately abandons final-only authority before attempted reentry"
    )]
    std::mem::forget(finish);
    let called = Cell::new(false);
    let called_in_body = &called;
    let drops = Cell::new(0);
    let retained_at_drop = Cell::new(false);
    let capture = BodyCapture {
        account: &account,
        drops: &drops,
        retained_at_drop: &retained_at_drop,
        panic_on_drop: false,
    };
    let error = host
        .run_body(move |_, _| {
            let _capture = &capture;
            called_in_body.set(true);
        })
        .err()
        .unwrap();
    assert!(!called.get());
    assert_eq!(drops.get(), 1);
    assert!(retained_at_drop.get());
    assert!(error.containment().is_some());
    drop(host);
    assert!(account.snapshot().poisoned());
    assert!(account.snapshot().live_retained_allocations() > 0);
}

#[test]
fn retained_candidate_body_rejects_missing_request_and_contains_forgotten_candidate() {
    for forgotten in [false, true] {
        let factory = factory(1);
        let account = account(&factory);
        let mut host = BodyInvocationHost::new();
        let error = host
            .run_body(|parts, _disposition| {
                let (candidate, _) = candidate_with_operation(
                    operation(parts.startup, &account),
                    parts.output,
                    parts.activation,
                    parts.proof,
                    parts.validation,
                );
                if forgotten {
                    std::mem::forget(candidate);
                } else {
                    drop(candidate);
                }
            })
            .err()
            .unwrap();
        assert!(error.containment().is_some());
        drop(host);
        assert!(account.snapshot().poisoned());
        assert!(account.snapshot().live_retained_allocations() > 0);
    }
}

#[derive(Clone, Copy)]
struct TestCanonicalRoots {
    canonical: baozi_operation::CanonicalSequenceRef<u8, SequenceTag>,
    evidence: baozi_operation::OutcomeEvidenceSequenceRef<u8, EvidenceLeafTag>,
}

fn candidate<'inv>(
    host: &'inv mut TestInvocationHost<()>,
    account: &PhysicalStorageAccount,
) -> (
    CanonicalFinal<TestCanonicalCandidate<'inv>>,
    TestKeys,
    CanonicalGenerationSlots<'inv, GenerationRejection>,
    CanonicalCauseSlot<'inv, StoreRuntimeError>,
) {
    candidate_with_control(host, account, OperationControl::default())
}

fn candidate_with_control<'inv>(
    host: &'inv mut TestInvocationHost<()>,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
) -> (
    CanonicalFinal<TestCanonicalCandidate<'inv>>,
    TestKeys,
    CanonicalGenerationSlots<'inv, GenerationRejection>,
    CanonicalCauseSlot<'inv, StoreRuntimeError>,
) {
    let (candidate, keys, _, generation, fallback) =
        candidate_and_roots_with_control(host, account, control);
    (candidate, keys, generation, fallback)
}

fn candidate_and_roots_with_control<'inv>(
    host: &'inv mut TestInvocationHost<()>,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
) -> (
    CanonicalFinal<TestCanonicalCandidate<'inv>>,
    TestKeys,
    TestCanonicalRoots,
    CanonicalGenerationSlots<'inv, GenerationRejection>,
    CanonicalCauseSlot<'inv, StoreRuntimeError>,
) {
    let mut control = Some(control);
    let mut saved_keys = None;
    let mut saved_roots = None;
    let finish = host
        .run_body(|parts, disposition| {
            let (candidate, keys, roots) = candidate_with_roots(
                operation_with_control(parts.startup, account, control.take().unwrap()),
                parts.output,
                parts.activation,
                parts.proof,
                parts.validation,
            );
            saved_keys = Some(keys);
            saved_roots = Some(roots);
            candidate.request_finish(disposition).unwrap();
        })
        .unwrap();
    let completion = finish.resume();
    let CanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
        panic!("the recorded candidate must resume as the same success stage")
    };
    (
        candidate,
        saved_keys.unwrap(),
        saved_roots.unwrap(),
        completion.generation,
        completion.failure_fallback,
    )
}

fn candidate_with_operation<'inv>(
    (operation, keys): (TestBodyOperation<'inv>, TestKeys),
    output_slot: CanonicalOutputSlot<'inv, ()>,
    activation_slots: CanonicalActivationSlots<'inv, ()>,
    proof_slot: CanonicalProofSlot<'inv, ()>,
    validation_slots: CanonicalValidationSlots<'inv, ValidationRejection>,
) -> (TestCanonicalCandidate<'inv>, TestKeys) {
    let (candidate, keys, _) = candidate_with_roots(
        (operation, keys),
        output_slot,
        activation_slots,
        proof_slot,
        validation_slots,
    );
    (candidate, keys)
}

fn candidate_with_roots<'inv>(
    (operation, keys): (TestBodyOperation<'inv>, TestKeys),
    output_slot: CanonicalOutputSlot<'inv, ()>,
    activation_slots: CanonicalActivationSlots<'inv, ()>,
    proof_slot: CanonicalProofSlot<'inv, ()>,
    validation_slots: CanonicalValidationSlots<'inv, ValidationRejection>,
) -> (TestCanonicalCandidate<'inv>, TestKeys, TestCanonicalRoots) {
    let canonical_root = Cell::new(None);
    let evidence_root = Cell::new(None);
    let outcome = plan_scoped_activation!(
        operation,
        output_slot,
        activation_slots,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap()),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
                .unwrap();
            canonical_root.set(Some(root));
            lane.complete::<_, (), _, _>(root, ())
        },
    )
    .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_) | CanonicalActivationOutcome::NeverActivated(_) => {
            panic!("the exact candidate must activate")
        }
    };
    let outcome = activated
        .validate_centrally(
            proof_slot,
            validation_slots,
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
                        evidence_root.set(Some(evidence.reference()));
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
    (
        candidate,
        keys,
        TestCanonicalRoots {
            canonical: canonical_root.get().unwrap(),
            evidence: evidence_root.get().unwrap(),
        },
    )
}

fn ready_permit<'inv>(
    candidate: CanonicalFinal<TestCanonicalCandidate<'inv>>,
    interruption_slot: CanonicalCauseSlot<'inv, CanonicalFreezeInterruption>,
) -> baozi_operation::__retained_store_integration::activation::CanonicalFreezePermit<
    'inv,
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
    match candidate
        .try_issue_freeze_permit(interruption_slot, candidate_final_progress())
        .unwrap()
    {
        CanonicalFreezePermitIssue::Ready(permit) => permit,
        CanonicalFreezePermitIssue::Interrupted(_) => panic!("final progress must remain open"),
        CanonicalFreezePermitIssue::DestinationRejected(_) => {
            panic!("exact destinations must remain admissible")
        }
    }
}

struct GenerationPairingCapture<'a> {
    account: &'a PhysicalStorageAccount,
    dropped: &'a Cell<bool>,
    before_containment: &'a Cell<bool>,
    panic_on_drop: bool,
}

impl Drop for GenerationPairingCapture<'_> {
    fn drop(&mut self) {
        let snapshot = self.account.snapshot();
        self.before_containment
            .set(!snapshot.poisoned() && snapshot.live_retained_allocations() > 0);
        self.dropped.set(true);
        assert!(!self.panic_on_drop, "denied pairing source capture panic");
    }
}

fn assert_generation_pairing_denied(panic_on_drop: bool) {
    for foreign_roots in 1..=3 {
        let factory = factory(2);
        let own_account = account(&factory);
        let foreign_account = account(&factory);
        let mut own_host: TestInvocationHost<()> = TestInvocationHost::new();
        let mut foreign_host: TestInvocationHost<()> = TestInvocationHost::new();
        let source_called = Cell::new(false);
        let capture_dropped = Cell::new(false);
        let before_containment = Cell::new(false);
        {
            let (candidate, _, own_roots, generation, _) = candidate_and_roots_with_control(
                &mut own_host,
                &own_account,
                OperationControl::default(),
            );
            let permit = ready_permit(candidate, generation.interruption);
            let (candidate, _, other_roots, generation, _) = candidate_and_roots_with_control(
                &mut foreign_host,
                &foreign_account,
                OperationControl::default(),
            );
            let other_permit = ready_permit(candidate, generation.interruption);
            let capture = GenerationPairingCapture {
                account: &own_account,
                dropped: &capture_dropped,
                before_containment: &before_containment,
                panic_on_drop,
            };
            let called = &source_called;
            let source = move || {
                let _capture = capture;
                called.set(true);
            };
            let result = permit.admit_generation_pairing(
                if foreign_roots & 1 != 0 {
                    other_roots.canonical
                } else {
                    own_roots.canonical
                },
                if foreign_roots & 2 != 0 {
                    other_roots.evidence
                } else {
                    own_roots.evidence
                },
                source,
            );
            assert!(result.is_err());
            assert!(!source_called.get());
            assert!(capture_dropped.get());
            assert!(before_containment.get());
            drop(other_permit);
        }
        drop(own_host);
        drop(foreign_host);
        assert!(own_account.snapshot().poisoned());
        assert!(own_account.snapshot().live_retained_allocations() > 0);
        assert!(!foreign_account.snapshot().poisoned());
        assert_eq!(foreign_account.snapshot().live_retained_allocations(), 0);
    }
}

#[test]
fn generation_pairing_denies_either_foreign_root_before_source() {
    assert_generation_pairing_denied(false);
}

#[test]
fn generation_pairing_contains_denied_source_capture_panic() {
    assert_generation_pairing_denied(true);
}

#[test]
fn generation_pairing_preserves_matching_roots_and_success_publication() {
    let factory = factory(1);
    let account = account(&factory);
    let mut host: TestInvocationHost<()> = TestInvocationHost::new();
    let source_called = Cell::new(false);
    let (candidate, keys, roots, generation, _) =
        candidate_and_roots_with_control(&mut host, &account, OperationControl::default());
    let permit = ready_permit(candidate, generation.interruption);
    let (permit, source) = permit
        .admit_generation_pairing(roots.canonical, roots.evidence, || source_called.set(true))
        .unwrap();
    assert!(!source_called.get());
    assert!(permit.destination_counters_are_zero());
    source();
    assert!(source_called.get());
    let prepared = permit.bind_joint_commit(|joint, seal| joint.prepare(seal).unwrap());
    let CanonicalJointSuccess {
        output: (),
        proof: (),
        store: prepared,
    } = prepared;
    let published = prepared.handoff();
    assert!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .unwrap()
            .committed_canonical_output()
            > 0
    );
    drop(published);
    drop(host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn permit_stages_exact_destinations_without_publishing_counters() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::new();
    let (candidate, _keys, generation_slots, _failure_fallback) =
        candidate(&mut invocation_host, &account);
    let permit = ready_permit(candidate, generation_slots.interruption);

    assert!(permit.destination_counters_are_zero());
    drop(permit);
    drop(invocation_host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn scoped_joint_commit_prepares_and_commits_validated_counts() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::new();
    let (candidate, keys, generation_slots, _failure_fallback) =
        candidate(&mut invocation_host, &account);
    let permit = ready_permit(candidate, generation_slots.interruption);

    let prepared = permit.bind_joint_commit(|joint, seal| joint.prepare(seal).unwrap());
    let CanonicalJointSuccess {
        output: (),
        proof: (),
        store: prepared,
    } = prepared;
    let published = prepared.handoff();
    let dimension = published.ledger_evidence.dimension(keys.dimension).unwrap();
    assert_eq!(dimension.pending_publication(), 0);
    assert_eq!(dimension.committed(), 1);
    assert_eq!(dimension.committed_canonical_output(), 1);
    assert_eq!(dimension.published_outcome_evidence(), 2);
    assert_eq!(published.canonical.len(), 1);
    assert_eq!(published.outcome_evidence.len(), 1);
    drop(published);
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn dropping_scoped_joint_commit_releases_the_physical_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::new();
    let (candidate, _keys, generation_slots, _failure_fallback) =
        candidate(&mut invocation_host, &account);
    let permit = ready_permit(candidate, generation_slots.interruption);

    permit.bind_joint_commit(|joint, seal| {
        drop(seal);
        drop(joint);
    });
    drop(invocation_host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn generation_rejection_discards_canonical_and_preserves_rich_evidence() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::new();
    let (candidate, keys, generation_slots, failure_fallback) =
        candidate(&mut invocation_host, &account);
    let permit = ready_permit(candidate, generation_slots.interruption);
    let rejected = permit
        .reject_generation(
            generation_slots.rejection,
            GenerationRejection::Semantic,
            GENERATION_REJECTION_CODE,
        )
        .unwrap();
    let validated = match rejected
        .validate_rich_failure(failure_fallback, |session, evidence| {
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
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn destination_rejection_preserves_ordinary_failure_preparation() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<()>::new();
    let mut actual_keys = None;
    let finish = invocation_host
        .run_body(|parts, disposition| {
            let CanonicalBodyParts {
                startup: invocation_root,
                output: output_slot,
                proof: proof_slot,
                activation: activation_slots,
                validation: validation_slots,
            } = parts;
            let (candidate, keys) = candidate_with_operation(
                operation_with_commit_limit(invocation_root, &account, 0),
                output_slot,
                activation_slots,
                proof_slot,
                validation_slots,
            );
            actual_keys = Some(keys);
            candidate.request_finish(disposition).unwrap();
        })
        .unwrap();
    let (candidate, generation_slots, failure_fallback) = {
        let completion = finish.resume();
        let CanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
            panic!("the committed-output limit must be checked on the actual resumed candidate")
        };
        (
            candidate,
            completion.generation,
            completion.failure_fallback,
        )
    };
    let keys = actual_keys.unwrap();
    let rejected = match candidate
        .try_issue_freeze_permit(generation_slots.interruption, candidate_final_progress())
        .unwrap()
    {
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
        .validate_rich_failure(failure_fallback, |session, evidence| {
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
        .try_prepare(candidate_final_progress())
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
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn cancellation_after_ready_keeps_frozen_success_without_reentering_control() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Signal {
        token: CancellationToken,
        polls: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    impl baozi_operation::CancellationSignal for Signal {
        fn is_cancelled(&self) -> bool {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.token.is_cancelled()
        }
    }

    impl Drop for Signal {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct Observer {
        calls: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    impl baozi_operation::ProgressObserver<TestProgress, TestUnit> for Observer {
        fn observe(
            &mut self,
            _: baozi_operation::ProgressEvent<TestProgress, TestUnit>,
        ) -> baozi_operation::ProgressDecision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            baozi_operation::ProgressDecision::Continue
        }
    }

    impl Drop for Observer {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let polls = Arc::new(AtomicUsize::new(0));
    let signal_drops = Arc::new(AtomicUsize::new(0));
    let observer_calls = Arc::new(AtomicUsize::new(0));
    let observer_drops = Arc::new(AtomicUsize::new(0));
    let mut control = Some(
        OperationControl::default()
            .with_cancellation(Signal {
                token: cancellation.clone(),
                polls: Arc::clone(&polls),
                drops: Arc::clone(&signal_drops),
            })
            .with_progress_observer(Observer {
                calls: Arc::clone(&observer_calls),
                drops: Arc::clone(&observer_drops),
            }),
    );
    let mut host = BodyInvocationHost::new();
    let mut keys = None;
    {
        let finish = host
            .run_body(|parts, disposition| {
                let (candidate, actual_keys) = candidate_with_operation(
                    operation_with_control(parts.startup, &account, control.take().unwrap()),
                    parts.output,
                    parts.activation,
                    parts.proof,
                    parts.validation,
                );
                keys = Some(actual_keys);
                candidate.request_finish(disposition).unwrap();
            })
            .unwrap();
        let completion = finish.resume();
        let CanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
            panic!("the recorded candidate must resume its exact success state")
        };
        let permit = ready_permit(candidate, completion.generation.interruption);
        let frozen_polls = polls.load(Ordering::SeqCst);
        let frozen_observer_calls = observer_calls.load(Ordering::SeqCst);
        assert!(frozen_polls > 0);
        assert!(frozen_observer_calls > 0);
        assert_eq!(signal_drops.load(Ordering::SeqCst), 1);
        assert_eq!(observer_drops.load(Ordering::SeqCst), 1);
        assert!(!cancellation.is_cancelled());

        cancellation.cancel();
        let CanonicalJointSuccess {
            output: (),
            proof: (),
            store,
        } = permit.bind_joint_commit(|joint, seal| joint.prepare(seal).unwrap());
        let published = store.handoff();
        assert!(cancellation.is_cancelled());
        assert_eq!(
            published.ledger_evidence.header().terminal_cause(),
            StoreTerminalCause::Success
        );
        let dimension = published
            .ledger_evidence
            .dimension(keys.unwrap().dimension)
            .unwrap();
        assert_eq!(dimension.pending_publication(), 0);
        assert_eq!(dimension.committed_canonical_output(), 1);
        assert_eq!(dimension.published_outcome_evidence(), 2);
        assert_eq!(polls.load(Ordering::SeqCst), frozen_polls);
        assert_eq!(observer_calls.load(Ordering::SeqCst), frozen_observer_calls);
        assert_eq!(signal_drops.load(Ordering::SeqCst), 1);
        assert_eq!(observer_drops.load(Ordering::SeqCst), 1);
        drop(published);
    }
    drop(host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn final_progress_interruption_never_issues_a_success_permit() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    {
        let mut invocation_host = TestInvocationHost::new();
        let (candidate, _keys, generation_slots, _failure_fallback) =
            candidate_with_control(&mut invocation_host, &account, control);
        cancellation.cancel();
        let interrupted = match candidate
            .try_issue_freeze_permit(generation_slots.interruption, candidate_final_progress())
            .unwrap()
        {
            CanonicalFreezePermitIssue::Ready(_) => {
                panic!("cancelled progress cannot issue a permit")
            }
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
    }
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn healthy_candidate_and_freeze_permit_release_during_unrelated_host_unwind() {
    for freeze in [false, true] {
        let factory = factory(1);
        let account = account(&factory);
        let output_dropped = Cell::new(false);
        let proof_dropped = Cell::new(false);
        {
            let mut invocation_host = TestInvocationHost::<_, _>::new();
            let finish = invocation_host
                .run_body(|parts, disposition| {
                    let CanonicalBodyParts {
                        startup: invocation_root,
                        output: output_slot,
                        proof: proof_slot,
                        activation: activation_slots,
                        validation: validation_slots,
                        ..
                    } = parts;
                    let (operation, keys) = operation(invocation_root, &account);
                    let outcome = plan_scoped_activation!(
                        operation,
                        output_slot,
                        activation_slots,
                        scoped_activation_shape_with_validation_work(
                            keys,
                            1,
                            1,
                            NonZeroU64::new(2).unwrap()
                        ),
                        |mut lane| {
                            let root = lane
                                .try_retain_sequence(
                                    SequencePlan::<u8>::new(1).unwrap(),
                                    SequenceTag,
                                    |writer| writer.push(7),
                                )
                                .unwrap();
                            lane.complete::<_, (), _, _>(root, DropProbe(&output_dropped))
                        },
                    )
                    .unwrap();
                    let activated = match outcome {
                        CanonicalActivationOutcome::Activated(activated) => activated,
                        CanonicalActivationOutcome::Rejected(_)
                        | CanonicalActivationOutcome::NeverActivated(_) => {
                            panic!("materialization must activate")
                        }
                    };
                    let outcome = activated
                        .validate_centrally(
                            proof_slot,
                            validation_slots,
                            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
                                        Ok(
                                    CanonicalValidationDecision::<_, ValidationRejection>::accept(
                                        DropProbe(&proof_dropped),
                                    ),
                                )
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
                            panic!("valid central validation must produce a candidate")
                        }
                    };
                    candidate.request_finish(disposition).unwrap();
                })
                .unwrap();
            let (candidate, generation_slots) = {
                let completion = finish.resume();
                let candidate = match completion.outcome {
                    CanonicalInvocationOutcome::Candidate(candidate) => candidate,
                    _ => panic!("the actual borrowed payload candidate must resume"),
                };
                (candidate, completion.generation)
            };
            assert!(!output_dropped.get());
            assert!(!proof_dropped.get());
            assert!(!account.snapshot().poisoned());
            assert!(account.snapshot().live_retained_allocations() > 0);

            let payload = if freeze {
                let permit = match candidate
                    .try_issue_freeze_permit(
                        generation_slots.interruption,
                        candidate_final_progress(),
                    )
                    .unwrap()
                {
                    CanonicalFreezePermitIssue::Ready(permit) => permit,
                    CanonicalFreezePermitIssue::Interrupted(_) => {
                        panic!("final progress must remain open")
                    }
                    CanonicalFreezePermitIssue::DestinationRejected(_) => {
                        panic!("exact destinations must remain admissible")
                    }
                };
                assert!(permit.destination_counters_are_zero());
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    let _permit = permit;
                    panic!("unrelated host panic");
                }))
                .unwrap_err()
            } else {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    let _candidate = candidate;
                    panic!("unrelated host panic");
                }))
                .unwrap_err()
            };
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"unrelated host panic")
            );
            drop(payload);
        }

        assert!(output_dropped.get());
        assert!(proof_dropped.get());
        let after = account.snapshot();
        assert!(!after.poisoned());
        assert_eq!(after.poison_event_count(), 0);
        assert_eq!(after.first_poison_evidence(), None);
        assert_eq!(after.live_retained_bytes(), 0);
        assert_eq!(after.live_retained_allocations(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
        assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
    }
}

#[test]
fn candidate_and_freeze_permit_double_payload_failure_preserves_the_outer_host_panic() {
    for freeze in [false, true] {
        let factory = factory(1);
        let account = account(&factory);
        let output_dropped = Cell::new(false);
        let proof_dropped = Cell::new(false);
        let mut invocation_host = TestInvocationHost::<_, _>::new();
        let finish = invocation_host
            .run_body(|parts, disposition| {
                let CanonicalBodyParts {
                    startup: invocation_root,
                    output: output_slot,
                    proof: proof_slot,
                    activation: activation_slots,
                    validation: validation_slots,
                    ..
                } = parts;
                let (operation, keys) = operation(invocation_root, &account);
                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
                    scoped_activation_shape_with_validation_work(
                        keys,
                        1,
                        1,
                        NonZeroU64::new(2).unwrap()
                    ),
                    |mut lane| {
                        let root = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(1).unwrap(),
                                SequenceTag,
                                |writer| writer.push(7),
                            )
                            .unwrap();
                        lane.complete::<_, (), _, _>(
                            root,
                            (DropProbe(&output_dropped), PanicOnDrop),
                        )
                    },
                )
                .unwrap();
                let activated = match outcome {
                    CanonicalActivationOutcome::Activated(activated) => activated,
                    CanonicalActivationOutcome::Rejected(_)
                    | CanonicalActivationOutcome::NeverActivated(_) => {
                        panic!("materialization must activate")
                    }
                };
                let outcome = activated
                    .validate_centrally(
                        proof_slot,
                        validation_slots,
                        CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
                                    Ok(
                                CanonicalValidationDecision::<_, ValidationRejection>::accept((
                                    DropProbe(&proof_dropped),
                                    PanicOnDrop,
                                )),
                            )
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
                        panic!("valid central validation must produce a candidate")
                    }
                };
                candidate.request_finish(disposition).unwrap();
            })
            .unwrap();
        let (candidate, generation_slots) = {
            let completion = finish.resume();
            let candidate = match completion.outcome {
                CanonicalInvocationOutcome::Candidate(candidate) => candidate,
                _ => panic!("the actual double-panic candidate must resume"),
            };
            (candidate, completion.generation)
        };
        assert!(!output_dropped.get());
        assert!(!proof_dropped.get());

        let (payload, before) = if freeze {
            let permit = match candidate
                .try_issue_freeze_permit(generation_slots.interruption, candidate_final_progress())
                .unwrap()
            {
                CanonicalFreezePermitIssue::Ready(permit) => permit,
                CanonicalFreezePermitIssue::Interrupted(_) => {
                    panic!("final progress must remain open")
                }
                CanonicalFreezePermitIssue::DestinationRejected(_) => {
                    panic!("exact destinations must remain admissible")
                }
            };
            assert!(permit.destination_counters_are_zero());
            let before = account.snapshot();
            let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _permit = permit;
                panic!("unrelated host panic");
            }))
            .unwrap_err();
            (payload, before)
        } else {
            let before = account.snapshot();
            let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _candidate = candidate;
                panic!("unrelated host panic");
            }))
            .unwrap_err();
            (payload, before)
        };
        assert_eq!(
            payload.downcast_ref::<&str>(),
            Some(&"unrelated host panic")
        );
        drop(payload);

        assert!(proof_dropped.get());
        assert!(output_dropped.get());
        assert!(!before.poisoned());
        assert!(before.live_retained_allocations() > 0);
        let after = account.snapshot();
        assert!(after.poisoned());
        assert_eq!(
            after.first_poison_evidence().unwrap().reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
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
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }
}

#[test]
fn non_copy_output_remains_contained_before_the_success_binder() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<_, _>::new();
    let finish = invocation_host
        .run_body(|parts, disposition| {
            let CanonicalBodyParts {
                startup: invocation_root,
                output: output_slot,
                proof: proof_slot,
                activation: activation_slots,
                validation: validation_slots,
                ..
            } = parts;
            let (operation, keys) = operation(invocation_root, &account);
            let outcome = plan_scoped_activation!(
                operation,
                output_slot,
                activation_slots,
                scoped_activation_shape_with_validation_work(
                    keys,
                    1,
                    1,
                    NonZeroU64::new(2).unwrap()
                ),
                |mut lane| {
                    let root = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            SequenceTag,
                            |writer| writer.push(7),
                        )
                        .unwrap();
                    lane.complete::<_, (), _, _>(root, PanicOnDrop)
                },
            )
            .unwrap();
            let activated = match outcome {
                CanonicalActivationOutcome::Activated(activated) => activated,
                CanonicalActivationOutcome::Rejected(_)
                | CanonicalActivationOutcome::NeverActivated(_) => {
                    panic!("the exact candidate must activate")
                }
            };
            let outcome =
                activated
                    .validate_centrally(
                        proof_slot,
                        validation_slots,
                        CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
                | CanonicalCentralValidationOutcome::Interrupted(_) => {
                    panic!("validation must succeed")
                }
            };
            candidate.request_finish(disposition).unwrap();
        })
        .unwrap();
    let (candidate, generation_slots) = {
        let completion = finish.resume();
        let candidate = match completion.outcome {
            CanonicalInvocationOutcome::Candidate(candidate) => candidate,
            _ => panic!("the actual non-Copy output candidate must resume"),
        };
        (candidate, completion.generation)
    };
    let permit = match candidate
        .try_issue_freeze_permit(generation_slots.interruption, candidate_final_progress())
        .unwrap()
    {
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
    let mut invocation_host = TestInvocationHost::<_, _>::new();
    let finish = invocation_host
        .run_body(|parts, disposition| {
            let CanonicalBodyParts {
                startup: invocation_root,
                output: output_slot,
                proof: proof_slot,
                activation: activation_slots,
                validation: validation_slots,
                ..
            } = parts;
            let (operation, keys) = operation(invocation_root, &account);
            let outcome = plan_scoped_activation!(
                operation,
                output_slot,
                activation_slots,
                scoped_activation_shape_with_validation_work(
                    keys,
                    1,
                    1,
                    NonZeroU64::new(2).unwrap()
                ),
                |mut lane| {
                    let root = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            SequenceTag,
                            |writer| writer.push(7),
                        )
                        .unwrap();
                    lane.complete::<_, (), _, _>(root, ())
                },
            )
            .unwrap();
            let activated = match outcome {
                CanonicalActivationOutcome::Activated(activated) => activated,
                CanonicalActivationOutcome::Rejected(_)
                | CanonicalActivationOutcome::NeverActivated(_) => {
                    panic!("the exact candidate must activate")
                }
            };
            let outcome = activated
                .validate_centrally(
                    proof_slot,
                    validation_slots,
                    CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
                | CanonicalCentralValidationOutcome::Interrupted(_) => {
                    panic!("validation must succeed")
                }
            };
            candidate.request_finish(disposition).unwrap();
        })
        .unwrap();
    let (candidate, generation_slots) = {
        let completion = finish.resume();
        let candidate = match completion.outcome {
            CanonicalInvocationOutcome::Candidate(candidate) => candidate,
            _ => panic!("the actual non-Copy proof candidate must resume"),
        };
        (candidate, completion.generation)
    };
    let permit = match candidate
        .try_issue_freeze_permit(generation_slots.interruption, candidate_final_progress())
        .unwrap()
    {
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
