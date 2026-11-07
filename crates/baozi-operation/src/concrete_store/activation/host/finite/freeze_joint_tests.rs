use super::*;

mod failure_tests;
mod guard_tests;
use crate::concrete_store::activation::{
    CanonicalActivationSeal, CanonicalFreezeInterruptionClass, CanonicalMaterializationRole,
    CanonicalSuccessRequirements, CanonicalTableShape, CanonicalValidationDecision,
    CanonicalValidationProgress, CanonicalValidationRole, FailureEvidenceMember,
    FailureEvidenceValidationSession, FiniteCanonicalActivationOutcome,
    FiniteCanonicalFreezePermitIssue, FiniteCanonicalJointSuccess, RootedFailureValidation,
};
use crate::concrete_store::finite::tests::plans_with_failure_work;
use crate::concrete_store::tests::{
    TestActivationDescription, TestOwner, TestProgress, TestUnit, test_account, test_factory,
};
use crate::concrete_store::{
    CanonicalStoreRef, OutcomeEvidenceStoreRef, PreparedFiniteStoreFailureOutcome, RecordBlock,
    RetainedStoreWriterRole,
};
use crate::{
    CancellationOrigin, CancellationToken, ControlCause, DimensionPolicy, ProgressDecision,
    ScalarSequence, StorePreparationFailureClass, StoreTerminalCause,
};
use std::cell::{Cell, RefCell};
use std::num::NonZeroU64;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Payload {
    value: Box<u8>,
    label: &'static str,
    drops: Rc<RefCell<Vec<&'static str>>>,
}

impl Payload {
    fn new(label: &'static str, value: u8, drops: &Rc<RefCell<Vec<&'static str>>>) -> Self {
        Self {
            value: Box::new(value),
            label,
            drops: Rc::clone(drops),
        }
    }
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.drops.borrow_mut().push(self.label);
    }
}

struct Proof {
    payload: Payload,
    canonical: CanonicalStoreRef<ScalarSequence<u8>, ()>,
    evidence: OutcomeEvidenceStoreRef<RecordBlock, ()>,
}

type Host = FiniteCanonicalInvocationHost<
    TestOwner,
    TestProgress,
    TestUnit,
    Payload,
    Proof,
    &'static str,
    &'static str,
    &'static str,
    ScalarSequence<u8>,
    (),
    RecordBlock,
    (),
>;

type Completion<'host> = FiniteCanonicalInvocationCompletion<
    'host,
    TestOwner,
    TestProgress,
    TestUnit,
    Payload,
    Proof,
    &'static str,
    &'static str,
    &'static str,
    ScalarSequence<u8>,
    (),
    RecordBlock,
    (),
>;

#[derive(Default)]
struct ControlProbe {
    cancellation: CancellationToken,
    cancel_on_progress: AtomicBool,
    polls: AtomicUsize,
    progress: AtomicUsize,
    signal_drops: AtomicUsize,
    observer_drops: AtomicUsize,
}

impl ControlProbe {
    fn control(self: &Arc<Self>) -> OperationControl<TestProgress, TestUnit> {
        OperationControl::default()
            .with_cancellation(Signal(Arc::clone(self)))
            .with_progress_observer(Observer(Arc::clone(self)))
    }

    fn counts(&self) -> [usize; 4] {
        [
            self.polls.load(Ordering::SeqCst),
            self.progress.load(Ordering::SeqCst),
            self.signal_drops.load(Ordering::SeqCst),
            self.observer_drops.load(Ordering::SeqCst),
        ]
    }
}

struct Signal(Arc<ControlProbe>);

impl crate::CancellationSignal for Signal {
    fn is_cancelled(&self) -> bool {
        self.0.polls.fetch_add(1, Ordering::SeqCst);
        self.0.cancellation.is_cancelled()
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        self.0.signal_drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct Observer(Arc<ControlProbe>);

impl crate::ProgressObserver<TestProgress, TestUnit> for Observer {
    fn observe(&mut self, _: crate::ProgressEvent<TestProgress, TestUnit>) -> ProgressDecision {
        self.0.progress.fetch_add(1, Ordering::SeqCst);
        if self.0.cancel_on_progress.load(Ordering::SeqCst) {
            ProgressDecision::Cancel
        } else {
            ProgressDecision::Continue
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.0.observer_drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct WorkCalls {
    semantic: Cell<usize>,
    evidence: Cell<usize>,
    validation: Cell<usize>,
    scratch_releases: Cell<usize>,
}

impl WorkCalls {
    fn counts(&self) -> [usize; 4] {
        [
            self.semantic.get(),
            self.evidence.get(),
            self.validation.get(),
            self.scratch_releases.get(),
        ]
    }
}

fn progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 3)
}

fn candidate_completion<'host>(
    host: &'host mut Host,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    (output, proof): (Payload, Payload),
    calls: &WorkCalls,
    scratch_bytes: u64,
    reject_destination: bool,
) -> (Completion<'host>, [StoreDimensionKey; 6]) {
    candidate_completion_for_destination(
        host,
        account,
        control,
        (output, proof),
        calls,
        scratch_bytes,
        if reject_destination {
            DestinationAdmission::RejectCanonical
        } else {
            DestinationAdmission::Exact
        },
    )
}

#[derive(Clone, Copy)]
enum DestinationAdmission {
    Exact,
    RejectCanonical,
    RejectEvidence,
}

fn candidate_completion_for_destination<'host>(
    host: &'host mut Host,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    (output, proof): (Payload, Payload),
    calls: &WorkCalls,
    scratch_bytes: u64,
    destination: DestinationAdmission,
) -> (Completion<'host>, [StoreDimensionKey; 6]) {
    let (ledger, start, dimensions, phases) = plans_with_failure_work(2, 2, 2);
    let (ledger, start) = match destination {
        DestinationAdmission::Exact => (ledger, start),
        DestinationAdmission::RejectCanonical | DestinationAdmission::RejectEvidence => {
            let (domain, limits, _) = ledger.into_parts();
            let policies = dimensions.map(|dimension| {
                let policy = match destination {
                    DestinationAdmission::RejectCanonical if dimension == dimensions[2] => {
                        DimensionPolicy::reservable_bytes(256, 256, 0, 1 << 20)
                            .with_published_outcome_evidence_limit(256)
                    }
                    DestinationAdmission::RejectEvidence if dimension == dimensions[4] => {
                        DimensionPolicy::reservable_bytes(256, 256, 256, 1 << 20)
                            .with_published_outcome_evidence_limit(1)
                    }
                    _ => limits.dimension(dimension).unwrap(),
                };
                (dimension, policy)
            });
            let start = if matches!(destination, DestinationAdmission::RejectEvidence) {
                let requirement =
                    crate::RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(
                        2, 2, 2,
                    )
                    .unwrap();
                FiniteRetainedStoreStartPlan::new(
                    crate::concrete_store::RetainedStoreWriterPlan::new(
                        phases[2],
                        dimensions[0],
                        dimensions[1],
                        RetainedStoreBatchLimits::new(4, 2).unwrap(),
                    ),
                    crate::concrete_store::RetainedStoreSharePlan::new(
                        dimensions[2],
                        dimensions[3],
                    ),
                    crate::concrete_store::StoreFallbackEnvelope::new(23),
                    dimensions[4],
                    1,
                    crate::RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
                        dimensions[5],
                        2,
                        2,
                        2,
                        1280 + requirement.attempted_work_limit(),
                        1280 + requirement.consumed_work_limit(),
                    )
                    .unwrap(),
                )
            } else {
                start
            };
            (
                FiniteStoreLedgerPlan::try_new(domain, 8 << 20, 8 << 20, policies, phases).unwrap(),
                start,
            )
        }
    };
    let mut policy = Some(CanonicalActivationSessionPolicy::new(
        RetainedStoreWriterRole::new(phases[1], dimensions[0], dimensions[1]),
        RetainedStoreWriterRole::new(phases[1], dimensions[0], dimensions[1]),
        CanonicalMaterializationRole::new(dimensions[2], dimensions[3]),
        CanonicalValidationRole::new(phases[2], dimensions[2], dimensions[3]),
        dimensions[2],
        dimensions[4],
    ));
    let mut output = Some(output);
    let mut proof = Some(proof);
    let canonical_root = Cell::new(None);
    let finish = host
        .run_bound_body(control, ledger, account, start, |parts, disposition| {
            let owner = parts
                .startup
                .attach()
                .unwrap()
                .into_session_outcome_owner(
                    policy
                        .take()
                        .expect("the original session policy transfers once"),
                    RetainedStoreBatchLimits::new(3, 1).unwrap(),
                    CanonicalFailureRequirements::new(CanonicalTableShape::new(2, 2).unwrap()),
                )
                .unwrap();
            let mut description = Some(TestActivationDescription::new(
                CanonicalSuccessRequirements::new(
                    CanonicalTableShape::new(1, 1).unwrap(),
                    1,
                    1,
                    NonZeroU64::new(3).unwrap(),
                )
                .unwrap(),
                (),
            ));
            let outcome = owner
                .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                    let ((), completion) = planning
                        .with_semantic_work(|semantic| {
                            semantic
                                .run_unit(|| calls.semantic.set(calls.semantic.get() + 1))
                                .unwrap();
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                    let permit =
                        match planning.seal_activation(description.take().unwrap(), completion) {
                            CanonicalActivationSeal::Sealed(permit) => permit,
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                    permit.activate(|mut lane, _| {
                        let root = lane
                            .try_retain_sequence(
                                crate::SequencePlan::new(1).unwrap(),
                                (),
                                |writer| writer.push(17_u8),
                            )
                            .unwrap();
                        canonical_root.set(Some(root));
                        lane.complete(root, output.take().unwrap())
                    })
                })
                .unwrap();
            let FiniteCanonicalActivationOutcome::Activated(activated) = outcome else {
                panic!("freeze must start with an actual finite canonical root");
            };
            activated
                .validate_centrally(
                    parts.proof,
                    parts.validation,
                    CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
                    |lane, _| {
                        calls.evidence.set(calls.evidence.get() + 1);
                        let leaf = lane
                            .try_retain_sequence(
                                crate::SequencePlan::new(1).unwrap(),
                                (),
                                |writer| writer.push(29_u8),
                            )
                            .unwrap();
                        let root = lane
                            .try_retain_records(
                                crate::RecordPlan::new(1, 1).unwrap(),
                                (),
                                |writer| writer.write_row(|row| row.write_sequence_ref(leaf)),
                            )
                            .unwrap();
                        lane.complete(root)
                    },
                    |planning, _| {
                        planning.validate_with(
                            |checkpoint| {
                                if scratch_bytes == 0 {
                                    Ok(None)
                                } else {
                                    let scratch = checkpoint.reserve(scratch_bytes)?;
                                    checkpoint.validate_live_owner(&scratch)?;
                                    Ok(Some(scratch))
                                }
                            },
                            |session, canonical, evidence, scratch| {
                                calls.validation.set(calls.validation.get() + 1);
                                assert!(canonical.identifies(canonical_root.get().unwrap()));
                                assert!(session.visit_canonical_sequence(
                                    canonical,
                                    1,
                                    |_, value| value == 17,
                                )?);
                                let mut leaf = None;
                                assert!(session.visit_outcome_evidence_records(
                                    evidence,
                                    1,
                                    1,
                                    |_, row| {
                                        leaf = row.sequence_ref::<u8, (), _>(0, |member| member)?;
                                        Ok(leaf.is_some())
                                    },
                                )?);
                                assert!(session.visit_outcome_evidence_sequence(
                                    leaf.unwrap(),
                                    1,
                                    |_, value| value == 29,
                                )?);
                                if let Some(scratch) = scratch.take() {
                                    drop(scratch);
                                    calls.scratch_releases.set(calls.scratch_releases.get() + 1);
                                }
                                Ok(CanonicalValidationDecision::<_, &'static str>::accept(
                                    Proof {
                                        payload: proof.take().unwrap(),
                                        canonical: canonical_root.get().unwrap(),
                                        evidence: evidence.reference(),
                                    },
                                ))
                            },
                        )
                    },
                )
                .unwrap()
                .request_finish(disposition)
        })
        .unwrap();
    assert_eq!(calls.counts(), [1, 1, 1, usize::from(scratch_bytes != 0)]);
    let completion = finish.resume().unwrap();
    (completion, dimensions)
}

fn validate_evidence<'session, 'storage, 'control>(
    session: &mut FailureEvidenceValidationSession<
        'session,
        'storage,
        'control,
        TestOwner,
        TestProgress,
        TestUnit,
    >,
    root: FailureEvidenceMember<'session, RecordBlock, ()>,
) -> bool {
    let mut leaf = None;
    let accepted = session
        .visit_records(root, |_, row| {
            leaf = Some(row.sequence_ref::<u8, (), _>(0, |member| member).unwrap());
            true
        })
        .unwrap();
    accepted
        && session
            .visit_sequence(leaf.unwrap(), |_, value| value == 29)
            .unwrap()
}

#[test]
fn finite_cancellation_after_ready_commits_original_unequal_roots_without_reentering_control() {
    for scratch_bytes in [0, 64] {
        let factory = test_factory();
        let account = test_account(&factory);
        let probe = Arc::new(ControlProbe::default());
        let calls = WorkCalls::default();
        let drops = Rc::new(RefCell::new(Vec::new()));
        let output = Payload::new("output", 41, &drops);
        let proof = Payload::new("proof", 43, &drops);
        let output_identity = std::ptr::from_ref(output.value.as_ref());
        let proof_identity = std::ptr::from_ref(proof.value.as_ref());
        let (success, dimensions, frozen_counts) = {
            let mut host = Host::new();
            let (completion, dimensions) = candidate_completion(
                &mut host,
                &account,
                probe.control(),
                (output, proof),
                &calls,
                scratch_bytes,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("the original finite candidate must resume after Host cleanup");
            };
            assert_eq!(candidate.canonical_member_count(), 1);
            assert_eq!(candidate.outcome_evidence_member_count(), 2);
            let canonical = candidate.proof().canonical;
            let evidence = candidate.proof().evidence;
            let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
                .try_issue_freeze_permit(completion.generation.interruption, progress())
                .unwrap()
            else {
                panic!("healthy exact destinations must issue the finite permit");
            };
            let (permit, ()) = permit
                .admit_generation_pairing(canonical, evidence, ())
                .unwrap();
            let mut permit = permit;
            assert!(permit.destination_counters_are_zero());
            assert!(std::ptr::eq(
                permit.output().value.as_ref(),
                output_identity
            ));
            assert!(std::ptr::eq(
                permit.proof().payload.value.as_ref(),
                proof_identity
            ));
            assert!(drops.borrow().is_empty());
            let frozen_counts = probe.counts();
            assert!(frozen_counts[0] > 0 && frozen_counts[1] > 0);
            assert_eq!(&frozen_counts[2..], [1, 1]);
            let admitted = account.snapshot();
            probe.cancellation.cancel();
            let success = permit.bind_joint_commit(|joint, seal| joint.prepare(seal).unwrap());
            assert_eq!(probe.counts(), frozen_counts);
            assert_eq!(calls.counts(), [1, 1, 1, usize::from(scratch_bytes != 0)]);
            assert!(drops.borrow().is_empty());
            assert_eq!(
                account.snapshot().total_admitted_retained_allocations(),
                admitted.total_admitted_retained_allocations(),
            );
            (success, dimensions, frozen_counts)
        };
        let FiniteCanonicalJointSuccess {
            output,
            proof,
            store,
        } = success;
        assert!(std::ptr::eq(output.value.as_ref(), output_identity));
        assert!(std::ptr::eq(proof.payload.value.as_ref(), proof_identity));
        let parts = store.handoff();
        assert_eq!(parts.canonical.root().as_slice(), [17]);
        let leaf = parts
            .outcome_evidence
            .root()
            .sequence_ref::<u8, ()>(0, 0)
            .unwrap();
        assert_eq!(
            parts.outcome_evidence.sequence(leaf).unwrap().as_slice(),
            [29]
        );
        assert_eq!(
            parts.facts.header().terminal_cause(),
            StoreTerminalCause::Success
        );
        assert_eq!(parts.facts.original_freeze_control(), None);
        assert_eq!(parts.facts.header().active_reservations(), 0);
        // The actual startup attribution has already promoted the original finite ledger.
        assert!(parts.facts.covered_administrative_bytes().unwrap() > 0);
        let canonical = parts.facts.dimension(dimensions[2]).unwrap();
        let evidence = parts.facts.dimension(dimensions[4]).unwrap();
        assert_eq!(canonical.pending_publication(), 0);
        assert_eq!(canonical.committed_canonical_output(), 1);
        assert_eq!(evidence.pending_publication(), 0);
        assert_eq!(evidence.published_outcome_evidence(), 2);
        let (_, _, foreign, _) = plans_with_failure_work(2, 2, 2);
        assert!(parts.facts.dimension(foreign[2]).is_err());
        drop(parts);
        drop(output);
        drop(proof);
        assert_eq!(*drops.borrow(), ["output", "proof"]);
        assert_eq!(probe.counts(), frozen_counts);
        assert!(!account.snapshot().poisoned());
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
    }
}

#[test]
fn finite_final_observer_cancellation_reuses_frozen_evidence_and_detailed_control() {
    let factory = test_factory();
    let account = test_account(&factory);
    let probe = Arc::new(ControlProbe::default());
    let calls = WorkCalls::default();
    let drops = Rc::new(RefCell::new(Vec::new()));
    let expected = ControlCause::Cancelled {
        origin: CancellationOrigin::Observer,
    };
    let (prepared, dimensions, frozen_counts) = {
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
            0,
            false,
        );
        let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
            panic!("final cancellation must follow real central validation");
        };
        probe.cancel_on_progress.store(true, Ordering::SeqCst);
        let FiniteCanonicalFreezePermitIssue::Interrupted(rooted) = candidate
            .try_issue_freeze_permit(completion.generation.interruption, progress())
            .unwrap()
        else {
            panic!("the final observer must interrupt issuance rather than issue Ready");
        };
        assert_eq!(
            rooted.cause().class(),
            CanonicalFreezeInterruptionClass::Control
        );
        assert_eq!(rooted.cause().control_cause(), Some(&expected));
        assert_eq!(*drops.borrow(), ["proof", "output"]);
        let frozen_counts = probe.counts();
        assert_eq!(&frozen_counts[2..], [1, 1]);
        let admitted = account.snapshot();
        let mut validations = 0;
        let RootedFailureValidation::Valid(validated) = rooted
            .validate_rich_failure(completion.failure_fallback, |session, root| {
                validations += 1;
                validate_evidence(session, root)
            })
            .unwrap()
        else {
            panic!("the already staged two-member evidence must validate as Frozen");
        };
        let prepared = validated.try_prepare_frozen().unwrap();
        assert_eq!(validations, 1);
        assert_eq!(probe.counts(), frozen_counts);
        assert_eq!(calls.counts(), [1, 1, 1, 0]);
        assert_eq!(
            account.snapshot().total_admitted_retained_allocations(),
            admitted.total_admitted_retained_allocations(),
        );
        (prepared, dimensions, frozen_counts)
    };
    let (prepared, cause, control) = prepared.into_parts_with_control();
    assert_eq!(cause.control_cause(), Some(&expected));
    assert_eq!(control, None);
    let PreparedFiniteStoreFailureOutcome::RichFailure(prepared) = prepared else {
        panic!("Frozen preparation must retain the actual evidence owner");
    };
    let parts = prepared.handoff();
    assert_eq!(parts.facts.original_freeze_control(), Some(&expected));
    assert!(
        matches!(parts.facts.header().terminal_cause(), StoreTerminalCause::Failure(cause)
        if cause.class() == StorePreparationFailureClass::Control && cause.code() == 1)
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[2])
            .unwrap()
            .committed_canonical_output(),
        0
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        2
    );
    drop(parts);
    assert_eq!(probe.counts(), frozen_counts);
    assert_eq!(*drops.borrow(), ["proof", "output"]);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_destination_denial_keeps_ordinary_evidence_and_runs_final_control_once() {
    let factory = test_factory();
    let account = test_account(&factory);
    let probe = Arc::new(ControlProbe::default());
    let calls = WorkCalls::default();
    let drops = Rc::new(RefCell::new(Vec::new()));
    let (prepared, dimensions) = {
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
            0,
            true,
        );
        let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
            panic!("destination denial must follow a real validated finite candidate");
        };
        let before_issue = probe.counts();
        let FiniteCanonicalFreezePermitIssue::DestinationRejected(rooted) = candidate
            .try_issue_freeze_permit(completion.generation.interruption, progress())
            .unwrap()
        else {
            panic!("a zero canonical output limit must preserve the Ordinary failure rail");
        };
        assert_eq!(probe.counts(), before_issue);
        assert_eq!(
            rooted.cause().class(),
            CanonicalFreezeInterruptionClass::Ledger
        );
        assert_eq!(rooted.cause().control_cause(), None);
        assert_eq!(*drops.borrow(), ["proof", "output"]);
        let admitted = account.snapshot();
        let mut validations = 0;
        let RootedFailureValidation::Valid(validated) = rooted
            .validate_rich_failure(completion.failure_fallback, |session, root| {
                validations += 1;
                validate_evidence(session, root)
            })
            .unwrap()
        else {
            panic!("Ordinary validation must stage the original evidence exactly once");
        };
        let before_prepare = probe.counts();
        assert_eq!(&before_prepare[2..], [0, 0]);
        let prepared = validated.try_prepare(progress()).unwrap();
        let after_prepare = probe.counts();
        assert_eq!(after_prepare[1], before_prepare[1] + 1);
        assert_eq!(&after_prepare[2..], [1, 1]);
        assert_eq!(validations, 1);
        assert_eq!(calls.counts(), [1, 1, 1, 0]);
        assert_eq!(
            account.snapshot().total_admitted_retained_allocations(),
            admitted.total_admitted_retained_allocations(),
        );
        (prepared, dimensions)
    };
    let (prepared, cause, control) = prepared.into_parts_with_control();
    assert_eq!(cause.class(), CanonicalFreezeInterruptionClass::Ledger);
    assert_eq!(control, None);
    let PreparedFiniteStoreFailureOutcome::RichFailure(prepared) = prepared else {
        panic!("destination denial must prepare the original rich evidence");
    };
    let parts = prepared.handoff();
    assert_eq!(parts.facts.original_freeze_control(), None);
    assert!(
        matches!(parts.facts.header().terminal_cause(), StoreTerminalCause::Failure(cause)
        if cause.class() == StorePreparationFailureClass::Ledger)
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[2])
            .unwrap()
            .committed_canonical_output(),
        0
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        2
    );
    drop(parts);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_second_destination_denial_leaves_both_original_batches_empty_before_ordinary_failure() {
    let factory = test_factory();
    let account = test_account(&factory);
    let probe = Arc::new(ControlProbe::default());
    let calls = WorkCalls::default();
    let drops = Rc::new(RefCell::new(Vec::new()));
    let (fallback, dimensions, final_counts) = {
        let mut host = Host::new();
        let (completion, dimensions) = candidate_completion_for_destination(
            &mut host,
            &account,
            probe.control(),
            (
                Payload::new("output", 41, &drops),
                Payload::new("proof", 43, &drops),
            ),
            &calls,
            0,
            DestinationAdmission::RejectEvidence,
        );
        let FiniteCanonicalInvocationOutcome::Candidate(mut candidate) = completion.outcome else {
            panic!("second-side denial requires an actual centrally validated candidate");
        };
        assert_eq!(candidate.canonical_member_count(), 1);
        assert_eq!(candidate.outcome_evidence_member_count(), 2);
        {
            let assembly = candidate
                .state
                .owner
                .storage_mut()
                .storage_mut()
                .ordinary_assembly();
            assert_eq!(assembly.loan().canonical_commitment_len(), 0);
            assert_eq!(assembly.loan().outcome_evidence_commitment_len(), 0);
        }
        let before_issue = probe.counts();
        let admitted = account.snapshot();
        let FiniteCanonicalFreezePermitIssue::DestinationRejected(mut rooted) = candidate
            .try_issue_freeze_permit(completion.generation.interruption, progress())
            .unwrap()
        else {
            panic!("the original second destination admits only one of two evidence members");
        };
        assert_eq!(probe.counts(), before_issue);
        assert_eq!(&before_issue[2..], [0, 0]);
        assert_eq!(
            rooted.cause().class(),
            CanonicalFreezeInterruptionClass::Ledger
        );
        assert_eq!(rooted.cause().control_cause(), None);
        assert_eq!(*drops.borrow(), ["proof", "output"]);
        {
            let mut evidence = rooted.state.owner.storage_mut().evidence_loan().unwrap();
            let assembly = evidence.publication_impl();
            assert_eq!(assembly.raw.loan().canonical_commitment_len(), 0);
            assert_eq!(assembly.raw.loan().outcome_evidence_commitment_len(), 0);
            let canonical = assembly
                .raw
                .loan()
                .terminal_observation(dimensions[2])
                .unwrap();
            let evidence = assembly
                .raw
                .loan()
                .terminal_observation(dimensions[4])
                .unwrap();
            assert!(
                crate::concrete_store::activation::freeze_driver::destinations_are_zero(
                    canonical, evidence
                )
            );
        }
        let mut validations = 0;
        let RootedFailureValidation::Valid(validated) = rooted
            .validate_rich_failure(completion.failure_fallback, |session, root| {
                validations += 1;
                validate_evidence(session, root)
            })
            .unwrap()
        else {
            panic!(
                "Ordinary validation stages its original evidence row before terminal eligibility"
            );
        };
        assert_eq!(validations, 1);
        assert_eq!(calls.counts(), [1, 1, 1, 0]);
        assert_eq!(
            account.snapshot().total_admitted_retained_allocations(),
            admitted.total_admitted_retained_allocations(),
        );
        let before_prepare = probe.counts();
        assert_eq!(&before_prepare[2..], [0, 0]);
        let prepared = validated.try_prepare(progress()).unwrap();
        let final_counts = probe.counts();
        assert_eq!(final_counts[1], before_prepare[1] + 1);
        assert_eq!(&final_counts[2..], [1, 1]);
        (prepared, dimensions, final_counts)
    };
    let (prepared, cause) = fallback.into_parts();
    assert_eq!(cause.class(), CanonicalFreezeInterruptionClass::Ledger);
    let PreparedFiniteStoreFailureOutcome::MinimalFallback(prepared) = prepared else {
        panic!("terminal evidence eligibility must preserve the one-unit admitted fallback");
    };
    let parts = prepared.handoff();
    assert_eq!(parts.facts.original_freeze_control(), None);
    assert!(
        matches!(parts.facts.header().terminal_cause(), StoreTerminalCause::Failure(cause)
        if cause.class() == StorePreparationFailureClass::Ledger)
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[2])
            .unwrap()
            .committed_canonical_output(),
        0
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        1
    );
    drop(parts);
    assert_eq!(probe.counts(), final_counts);
    assert_eq!(*drops.borrow(), ["proof", "output"]);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_generation_rejection_preserves_frozen_evidence_without_replaying_control() {
    let factory = test_factory();
    let account = test_account(&factory);
    let probe = Arc::new(ControlProbe::default());
    let calls = WorkCalls::default();
    let drops = Rc::new(RefCell::new(Vec::new()));
    let (prepared, dimensions, frozen_counts) = {
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
            0,
            false,
        );
        let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
            panic!("generation rejection must consume the original restored candidate");
        };
        let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
            .try_issue_freeze_permit(completion.generation.interruption, progress())
            .unwrap()
        else {
            panic!("generation admission must follow Ready");
        };
        let frozen_counts = probe.counts();
        let generated = Cell::new(0);
        let (permit, (generation, (), ())) = permit
            .admit_generation_rejection(
                &completion.generation.rejection,
                (|| generated.set(generated.get() + 1), (), ()),
            )
            .unwrap();
        assert_eq!(generated.get(), 0);
        generation();
        let rooted = permit
            .reject_generation(completion.generation.rejection, "generation exhausted", 113)
            .unwrap();
        assert_eq!(generated.get(), 1);
        assert_eq!(*drops.borrow(), ["proof", "output"]);
        let admitted = account.snapshot();
        let RootedFailureValidation::Valid(validated) = rooted
            .validate_rich_failure(completion.failure_fallback, validate_evidence)
            .unwrap()
        else {
            panic!("generation exhaustion must retain the already staged evidence row");
        };
        let prepared = validated.try_prepare_frozen().unwrap();
        assert_eq!(probe.counts(), frozen_counts);
        assert_eq!(calls.counts(), [1, 1, 1, 0]);
        assert_eq!(
            account.snapshot().total_admitted_retained_allocations(),
            admitted.total_admitted_retained_allocations(),
        );
        (prepared, dimensions, frozen_counts)
    };
    let (prepared, cause, control) = prepared.into_parts_with_control();
    assert!(matches!(
        cause,
        StoreCanonicalRejection::Requested("generation exhausted")
    ));
    assert_eq!(control, None);
    let PreparedFiniteStoreFailureOutcome::RichFailure(prepared) = prepared else {
        panic!("generation rejection must preserve the rich failure owner");
    };
    let parts = prepared.handoff();
    assert_eq!(parts.facts.original_freeze_control(), None);
    assert!(
        matches!(parts.facts.header().terminal_cause(), StoreTerminalCause::Failure(cause)
        if cause.class() == StorePreparationFailureClass::Requested && cause.code() == 113)
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[2])
            .unwrap()
            .committed_canonical_output(),
        0
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        2
    );
    drop(parts);
    assert_eq!(probe.counts(), frozen_counts);
    assert_eq!(*drops.borrow(), ["proof", "output"]);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn healthy_finite_permit_and_joint_abandonment_release_original_payloads_and_storage() {
    for bind_joint in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let probe = Arc::new(ControlProbe::default());
        let calls = WorkCalls::default();
        let drops = Rc::new(RefCell::new(Vec::new()));
        let frozen_counts = {
            let mut host = Host::new();
            let (completion, _) = candidate_completion(
                &mut host,
                &account,
                probe.control(),
                (
                    Payload::new("output", 41, &drops),
                    Payload::new("proof", 43, &drops),
                ),
                &calls,
                0,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("healthy abandonment requires the actual restored candidate");
            };
            let FiniteCanonicalFreezePermitIssue::Ready(mut permit) = candidate
                .try_issue_freeze_permit(completion.generation.interruption, progress())
                .unwrap()
            else {
                panic!("healthy abandonment must begin after Ready");
            };
            assert!(permit.destination_counters_are_zero());
            let frozen_counts = probe.counts();
            if bind_joint {
                permit.bind_joint_commit(|joint, _seal| drop(joint));
            } else {
                drop(permit);
            }
            assert_eq!(*drops.borrow(), ["output", "proof"]);
            assert_eq!(probe.counts(), frozen_counts);
            frozen_counts
        };
        assert_eq!(probe.counts(), frozen_counts);
        assert_eq!(calls.counts(), [1, 1, 1, 0]);
        assert_eq!(*drops.borrow(), ["output", "proof"]);
        assert!(!account.snapshot().poisoned());
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
    }
}
