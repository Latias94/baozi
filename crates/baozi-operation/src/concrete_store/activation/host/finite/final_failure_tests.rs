use super::*;
use crate::concrete_store::activation::payload_owner::PayloadOwner;
use crate::concrete_store::activation::{
    CanonicalActivationSeal, CanonicalFinal, CanonicalMaterializationRole,
    CanonicalSuccessRequirements, CanonicalTableShape, CanonicalValidationDecision,
    CanonicalValidationProgress, CanonicalValidationRole, FailureEvidenceOutcome,
    FiniteCanonicalActivationOutcome, RootedFailureEvidence, RootedFailureValidation,
};
use crate::concrete_store::finite::tests::{plans, plans_with_failure_work};
use crate::concrete_store::tests::{
    TestActivationDescription, TestOwner, TestProgress, TestUnit, test_account, test_factory,
};
use crate::concrete_store::{PreparedFiniteStoreFailureOutcome, RetainedStoreWriterRole};
use crate::{ScalarSequence, StorePreparationFailureClass, StoreTerminalCause};
use std::cell::Cell;
use std::num::NonZeroU64;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

type FailureHost<E> = FiniteCanonicalInvocationHost<
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    (),
    E,
    &'static str,
    (),
    ScalarSequence<u8>,
    (),
    ScalarSequence<u8>,
    (),
>;
type FailureCompletion<'host, E> = FiniteCanonicalInvocationCompletion<
    'host,
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    (),
    E,
    &'static str,
    (),
    ScalarSequence<u8>,
    (),
    ScalarSequence<u8>,
    (),
>;
type ActivationOutcome<'host, E> = FiniteCanonicalActivationOutcome<
    'host,
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    E,
    ScalarSequence<u8>,
    (),
>;

fn policy(
    dimensions: [StoreDimensionKey; 6],
    phases: [StorePhaseKey; 3],
) -> CanonicalActivationSessionPolicy {
    CanonicalActivationSessionPolicy::new(
        RetainedStoreWriterRole::new(phases[1], dimensions[0], dimensions[1]),
        RetainedStoreWriterRole::new(phases[1], dimensions[0], dimensions[1]),
        CanonicalMaterializationRole::new(dimensions[2], dimensions[3]),
        CanonicalValidationRole::new(phases[2], dimensions[2], dimensions[3]),
        dimensions[2],
        dimensions[4],
    )
}

fn failure_requirements() -> CanonicalFailureRequirements {
    CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap())
}

fn progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
}

fn never_completion<'host, E>(
    host: &'host mut FailureHost<E>,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    cause: E,
    code: u32,
) -> (
    FailureCompletion<'host, E>,
    [StoreDimensionKey; 6],
    [StorePhaseKey; 3],
) {
    let (ledger, start, dimensions, phases) = plans_with_failure_work(1, 1, 1);
    let mut cause = Some(cause);
    let finish = host
        .run_bound_body(control, ledger, account, start, |parts, disposition| {
            let owner = parts
                .startup
                .attach()
                .unwrap()
                .into_session_outcome_owner(
                    policy(dimensions, phases),
                    RetainedStoreBatchLimits::new(2, 2).unwrap(),
                    failure_requirements(),
                )
                .unwrap();
            let outcome: ActivationOutcome<'_, E> = owner
                .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                    let ((), _completion) = planning
                        .with_semantic_work(|semantic| {
                            semantic.run_unit(|| ()).unwrap();
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                    planning.reject(cause.take().unwrap(), code)
                })
                .unwrap();
            let FiniteCanonicalActivationOutcome::NeverActivated(never) = outcome else {
                panic!("the real unopened operation must retain its never-activated owner");
            };
            never.request_finish(disposition)
        })
        .unwrap();
    (finish.resume().unwrap(), dimensions, phases)
}

struct TrackedCause {
    drops: Rc<Cell<usize>>,
    panic_on_drop: bool,
}

#[test]
fn finite_final_recoverable_open_keeps_original_cause_or_owned_error_through_build_drop() {
    struct BuildCapture {
        drops: Arc<AtomicUsize>,
        panic_on_drop: bool,
    }
    impl Drop for BuildCapture {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic_on_drop, "finite unopened builder destructor");
        }
    }

    for capture_panics in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let drops = Arc::new(AtomicUsize::new(0));
        let original = Box::new(71_u8);
        let identity = std::ptr::from_ref(original.as_ref());
        let (result, dimensions) = {
            let mut host = FailureHost::new();
            let (completion, dimensions, _) = never_completion(
                &mut host,
                &account,
                OperationControl::default(),
                original,
                109,
            );
            let FiniteCanonicalInvocationOutcome::NeverActivated(never) = completion.outcome else {
                panic!("the actual unopened failure must resume after body cleanup");
            };
            let capture = BuildCapture {
                drops: Arc::clone(&drops),
                panic_on_drop: capture_panics,
            };
            crate::retained::fail_next_final_owner_allocation();
            let result = never.build_failure_evidence::<(), ()>(move |_| {
                std::hint::black_box(&capture);
                panic!("failed publication open must skip the original builder");
            });
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            let result = match result {
                Ok(FailureEvidenceOutcome::MinimalFallback(fallback)) if !capture_panics => {
                    let (prepared, cause) = fallback.into_parts();
                    assert!(matches!(cause, StoreNeverActivatedCause::Requested(cause)
                        if std::ptr::eq(cause.as_ref(), identity)));
                    Ok(prepared)
                }
                Err(error) if capture_panics => Err(error),
                _ => panic!(
                    "recoverable open and capture disposal must preserve their distinct outcomes"
                ),
            };
            (result, dimensions)
        };
        match result {
            Ok(prepared) => {
                let parts = prepared.handoff();
                assert!(
                    matches!(parts.facts.header().terminal_cause(), StoreTerminalCause::Failure(cause)
                    if cause.class() == StorePreparationFailureClass::ProductGraph)
                );
                assert!(parts.facts.dimension(dimensions[2]).is_ok());
                drop(parts);
                assert!(!account.snapshot().poisoned());
                assert_eq!(account.snapshot().live_retained_allocations(), 0);
                assert_eq!(account.snapshot().live_retained_bytes(), 0);
            }
            Err(error) => {
                assert!(error.has_residue());
                assert!(error.domain().unwrap().owns_dimension(dimensions[2]));
                assert!(account.snapshot().poisoned());
                let first = account.snapshot().first_poison_evidence();
                assert!(first.is_some());
                drop(error);
                assert_eq!(account.snapshot().first_poison_evidence(), first);
            }
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
    }
}

impl Drop for TrackedCause {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
        if self.panic_on_drop {
            panic!("finite prepared cause destructor");
        }
    }
}

#[test]
fn finite_final_never_build_validate_prepare_preserves_cause_and_original_facts() {
    let factory = test_factory();
    let account = test_account(&factory);
    let cause = Box::new(67_u8);
    let cause_identity = std::ptr::from_ref(cause.as_ref());
    let (prepared, dimensions, phases) = {
        let mut host = FailureHost::new();
        let (completion, dimensions, phases) =
            never_completion(&mut host, &account, OperationControl::default(), cause, 101);
        let FiniteCanonicalInvocationCompletion {
            outcome,
            failure_fallback,
            ..
        } = completion;
        let FiniteCanonicalInvocationOutcome::NeverActivated(never) = outcome else {
            panic!("Host cleanup must restore the original never-activated stage");
        };
        assert!(matches!(
            never.cause(),
            StoreNeverActivatedCause::Requested(value)
                if std::ptr::eq(value.as_ref(), cause_identity)
        ));
        let built = never
            .build_failure_evidence::<ScalarSequence<u8>, ()>(|lane| {
                let root = lane
                    .try_retain_sequence(crate::SequencePlan::new(1).unwrap(), (), |writer| {
                        writer.push(29_u8)
                    })
                    .unwrap();
                lane.complete(root)
            })
            .unwrap();
        let FailureEvidenceOutcome::Rooted(rooted) = built else {
            panic!("the admitted finite evidence must produce a rooted owner");
        };
        let RootedFailureEvidence {
            owner,
            failure_work,
            outcome_evidence_destination_dimension,
            _lifecycle,
        } = rooted.state;
        let (cause, mut storage) = owner.into_parts();
        {
            let mut evidence = storage.evidence_loan().unwrap();
            assert_eq!(
                evidence
                    .publication_impl()
                    .raw
                    .loan()
                    .outcome_evidence_commitment_len(),
                0,
            );
        }
        let rooted = CanonicalFinal::new(RootedFailureEvidence {
            owner: PayloadOwner::new(cause, storage),
            failure_work,
            outcome_evidence_destination_dimension,
            _lifecycle,
        });
        let visits = Cell::new(0);
        let validated = match rooted
            .validate_rich_failure(failure_fallback, |session, root| {
                session
                    .visit_sequence(root, |index, value| {
                        assert_eq!((index, value), (0, 29));
                        visits.set(visits.get() + 1);
                        true
                    })
                    .unwrap()
            })
            .unwrap()
        {
            RootedFailureValidation::Valid(validated) => validated,
            _ => panic!("the exact retained evidence must pass rich-failure validation"),
        };
        assert_eq!(visits.get(), 1);
        (
            validated.try_prepare(progress()).unwrap(),
            dimensions,
            phases,
        )
    };
    let (prepared, cause, control) = prepared.into_parts_with_control();
    assert_eq!(control, None);
    let StoreNeverActivatedCause::Requested(cause) = cause else {
        panic!("preparation must preserve the requested typed cause");
    };
    assert!(std::ptr::eq(cause.as_ref(), cause_identity));
    let PreparedFiniteStoreFailureOutcome::RichFailure(prepared) = prepared else {
        panic!("valid finite evidence must commit through the rich-failure rail");
    };
    let parts = prepared.handoff();
    assert_eq!(
        parts.outcome_evidence.outcome_evidence().root().as_slice(),
        [29]
    );
    let StoreTerminalCause::Failure(cause) = parts.facts.header().terminal_cause() else {
        panic!("the fixed terminal facts must retain the original failure code");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), 101);
    assert_eq!(parts.facts.header().dimension_count(), 6);
    assert_eq!(parts.facts.header().phase_count(), 3);
    let work = parts.facts.dimension(dimensions[2]).unwrap();
    assert!(work.consumed() > 0);
    assert_eq!(work.committed_canonical_output(), 0);
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        1
    );
    for phase in phases {
        assert_eq!(
            parts.facts.phase(phase).unwrap().phase(),
            phase.semantic_id()
        );
    }
    let (_, _, foreign, _) = plans();
    assert!(matches!(
        parts.facts.dimension(foreign[2]),
        Err(StoreRuntimeError::ForeignDimensionDomain { .. })
    ));
    drop(parts);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
}

#[test]
fn finite_final_rejection_builds_rich_evidence_after_body_capture_cleanup() {
    struct Capture(Rc<Cell<bool>>);
    impl Drop for Capture {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    let factory = test_factory();
    let account = test_account(&factory);
    let capture_dropped = Rc::new(Cell::new(false));
    let capture = Capture(Rc::clone(&capture_dropped));
    let cause = Box::new(71_u8);
    let cause_identity = std::ptr::from_ref(cause.as_ref());
    let mut cause = Some(cause);
    let (prepared, dimensions) = {
        let (ledger, start, dimensions, phases) = plans_with_failure_work(1, 1, 1);
        let mut host = FailureHost::new();
        let finish = host
            .run_bound_body(
                OperationControl::default(),
                ledger,
                &account,
                start,
                move |parts, disposition| {
                    let _capture = &capture;
                    let owner = parts
                        .startup
                        .attach()
                        .unwrap()
                        .into_session_outcome_owner(
                            policy(dimensions, phases),
                            RetainedStoreBatchLimits::new(2, 2).unwrap(),
                            failure_requirements(),
                        )
                        .unwrap();
                    let mut description = Some(TestActivationDescription::new(
                        CanonicalSuccessRequirements::new(
                            CanonicalTableShape::new(1, 1).unwrap(),
                            1,
                            1,
                            NonZeroU64::MIN,
                        )
                        .unwrap(),
                        (),
                    ));
                    let outcome: ActivationOutcome<'_, Box<u8>> = owner
                        .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                            let ((), completion) = planning
                                .with_semantic_work(|semantic| {
                                    semantic.run_unit(|| ()).unwrap();
                                    Ok::<_, ()>(())
                                })
                                .unwrap();
                            let permit = match planning
                                .seal_activation(description.take().unwrap(), completion)
                            {
                                CanonicalActivationSeal::Sealed(permit) => permit,
                                CanonicalActivationSeal::Unsealed(failure) => {
                                    return failure.finish();
                                }
                            };
                            permit.activate(|mut lane, _| {
                                let _canonical = lane
                                    .try_retain_sequence(
                                        crate::SequencePlan::<u8>::new(1).unwrap(),
                                        (),
                                        |writer| writer.push(17),
                                    )
                                    .unwrap();
                                lane.reject(cause.take().unwrap(), 102)
                            })
                        })
                        .unwrap();
                    let FiniteCanonicalActivationOutcome::Rejected(rejected) = outcome else {
                        panic!("rejection must follow actual canonical materialization");
                    };
                    assert!(matches!(
                        rejected.cause(),
                        StoreCanonicalRejection::Requested(value)
                            if std::ptr::eq(value.as_ref(), cause_identity)
                    ));
                    rejected.request_finish(disposition)
                },
            )
            .unwrap();
        assert!(capture_dropped.get());
        let FiniteCanonicalInvocationCompletion {
            outcome,
            failure_fallback,
            ..
        } = finish.resume().unwrap();
        let FiniteCanonicalInvocationOutcome::Rejected(rejected) = outcome else {
            panic!("Host must restore the actual activation rejection");
        };
        let built = rejected
            .build_failure_evidence::<ScalarSequence<u8>, ()>(|lane| {
                assert!(capture_dropped.get());
                let root = lane
                    .try_retain_sequence(crate::SequencePlan::new(1).unwrap(), (), |writer| {
                        writer.push(29_u8)
                    })
                    .unwrap();
                lane.complete(root)
            })
            .unwrap();
        let FailureEvidenceOutcome::Rooted(rooted) = built else {
            panic!("a real materialization rejection must retain rich-evidence capacity");
        };
        let validated = match rooted
            .validate_rich_failure(failure_fallback, |session, root| {
                session
                    .visit_sequence(root, |_, value| value == 29)
                    .unwrap()
            })
            .unwrap()
        {
            RootedFailureValidation::Valid(validated) => validated,
            _ => panic!("the rejection's newly authored evidence must validate"),
        };
        (validated.try_prepare(progress()).unwrap(), dimensions)
    };
    let (prepared, cause, control) = prepared.into_parts_with_control();
    assert_eq!(control, None);
    let StoreCanonicalRejection::Requested(cause) = cause else {
        panic!("the activation rejection must remain the original typed cause");
    };
    assert!(std::ptr::eq(cause.as_ref(), cause_identity));
    let PreparedFiniteStoreFailureOutcome::RichFailure(prepared) = prepared else {
        panic!("the validated rejection must publish rich failure");
    };
    let parts = prepared.handoff();
    assert_eq!(
        parts.outcome_evidence.outcome_evidence().root().as_slice(),
        [29]
    );
    let StoreTerminalCause::Failure(cause) = parts.facts.header().terminal_cause() else {
        panic!("the activation rejection must survive finite terminal capture");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), 102);
    let work = parts.facts.dimension(dimensions[2]).unwrap();
    assert_eq!(work.committed_canonical_output(), 0);
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        1
    );
    assert!(parts.facts.dimension(dimensions[5]).unwrap().consumed() > 0);
    drop(parts);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_final_semantic_evidence_rejection_keeps_original_code_in_minimal_fallback() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (fallback, dimensions) = {
        let mut host = FailureHost::new();
        let (completion, dimensions, _) = never_completion(
            &mut host,
            &account,
            OperationControl::default(),
            "original semantic cause",
            103,
        );
        let FiniteCanonicalInvocationCompletion {
            outcome,
            failure_fallback,
            ..
        } = completion;
        let FiniteCanonicalInvocationOutcome::NeverActivated(never) = outcome else {
            panic!("the actual pre-activation failure must resume");
        };
        let built = never
            .build_failure_evidence::<ScalarSequence<u8>, ()>(|lane| {
                let root = lane
                    .try_retain_sequence(crate::SequencePlan::new(1).unwrap(), (), |writer| {
                        writer.push(29_u8)
                    })
                    .unwrap();
                lane.complete(root)
            })
            .unwrap();
        let FailureEvidenceOutcome::Rooted(rooted) = built else {
            panic!("semantic rejection must inspect an actual retained evidence member");
        };
        let inspected = Cell::new(false);
        let rejected = match rooted
            .validate_rich_failure(failure_fallback, |session, root| {
                session
                    .visit_sequence(root, |index, value| {
                        assert_eq!((index, value), (0, 29));
                        inspected.set(true);
                        false
                    })
                    .unwrap()
            })
            .unwrap()
        {
            RootedFailureValidation::Rejected(rejected) => rejected,
            _ => panic!("semantic false must retain the typed rejected-evidence owner"),
        };
        assert!(inspected.get());
        (rejected.prepare_minimal_fallback().unwrap(), dimensions)
    };
    let (prepared, cause) = fallback.into_parts();
    assert!(matches!(
        cause,
        StoreNeverActivatedCause::Requested("original semantic cause")
    ));
    let parts = prepared.handoff();
    let StoreTerminalCause::Failure(cause) = parts.facts.header().terminal_cause() else {
        panic!("minimal fallback must retain the already verified original failure");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), 103);
    let work = parts.facts.dimension(dimensions[2]).unwrap();
    assert!(work.consumed() > 0);
    assert_eq!(work.committed_canonical_output(), 0);
    // The original start plan admits three fallback evidence units on this destination.
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        3,
    );
    drop(parts);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_final_central_failure_revalidates_existing_root_without_new_admission() {
    struct Signal(Arc<AtomicBool>);
    impl crate::CancellationSignal for Signal {
        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    for interrupt in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let cancelled = Arc::new(AtomicBool::new(false));
        let evidence_builds = Cell::new(0);
        let (prepared, dimensions) = {
            let (ledger, start, dimensions, phases) = plans_with_failure_work(1, 1, 1);
            let mut host = FailureHost::<&'static str>::new();
            let finish = host
                .run_bound_body(
                    OperationControl::default().with_cancellation(Signal(Arc::clone(&cancelled))),
                    ledger,
                    &account,
                    start,
                    |parts, disposition| {
                        let owner = parts
                            .startup
                            .attach()
                            .unwrap()
                            .into_session_outcome_owner(
                                policy(dimensions, phases),
                                RetainedStoreBatchLimits::new(2, 2).unwrap(),
                                failure_requirements(),
                            )
                            .unwrap();
                        let mut description = Some(TestActivationDescription::new(
                            CanonicalSuccessRequirements::new(
                                CanonicalTableShape::new(1, 1).unwrap(),
                                1,
                                1,
                                NonZeroU64::new(2).unwrap(),
                            )
                            .unwrap(),
                            (),
                        ));
                        let outcome: ActivationOutcome<'_, &'static str> = owner
                            .plan_and_activate_scoped(
                                parts.output,
                                parts.activation,
                                |mut planning| {
                                    let ((), completion) = planning
                                        .with_semantic_work(|semantic| {
                                            semantic.run_unit(|| ()).unwrap();
                                            Ok::<_, ()>(())
                                        })
                                        .unwrap();
                                    let permit = match planning
                                        .seal_activation(description.take().unwrap(), completion)
                                    {
                                        CanonicalActivationSeal::Sealed(permit) => permit,
                                        CanonicalActivationSeal::Unsealed(failure) => {
                                            return failure.finish();
                                        }
                                    };
                                    permit.activate(|mut lane, _| {
                                        let root = lane
                                            .try_retain_sequence(
                                                crate::SequencePlan::<u8>::new(1).unwrap(),
                                                (),
                                                |writer| writer.push(17),
                                            )
                                            .unwrap();
                                        lane.complete(root, ())
                                    })
                                },
                            )
                            .unwrap();
                        let FiniteCanonicalActivationOutcome::Activated(activated) = outcome else {
                            panic!("central failure must start with an actual canonical root");
                        };
                        let outcome = activated
                            .validate_centrally(
                                parts.proof,
                                parts.validation,
                                CanonicalValidationProgress::new(
                                    TestProgress::Build,
                                    TestUnit::Items,
                                ),
                                |lane, _| {
                                    evidence_builds.set(evidence_builds.get() + 1);
                                    let root = lane
                                        .try_retain_sequence(
                                            crate::SequencePlan::<u8>::new(1).unwrap(),
                                            (),
                                            |writer| writer.push(29),
                                        )
                                        .unwrap();
                                    lane.complete(root)
                                },
                                |planning, _| {
                                    planning.validate_with(
                                        |_| Ok(()),
                                        |session, canonical, evidence, &mut ()| {
                                            cancelled.store(interrupt, Ordering::SeqCst);
                                            assert!(session.visit_canonical_sequence(
                                                canonical,
                                                1,
                                                |_, value| value == 17
                                            )?);
                                            assert!(session.visit_outcome_evidence_sequence(
                                                evidence,
                                                1,
                                                |_, value| value == 29
                                            )?);
                                            Ok(CanonicalValidationDecision::<(), _>::reject(
                                                "original central rejection",
                                                104,
                                            ))
                                        },
                                    )
                                },
                            )
                            .unwrap();
                        outcome.request_finish(disposition)
                    },
                )
                .unwrap();
            let FiniteCanonicalInvocationCompletion {
                outcome,
                failure_fallback,
                ..
            } = finish.resume().unwrap();
            assert_eq!(evidence_builds.get(), 1);
            cancelled.store(false, Ordering::SeqCst);
            let admitted = account.snapshot();
            let prepared = match outcome {
                FiniteCanonicalInvocationOutcome::CentralRejected(failure) if !interrupt => {
                    assert!(matches!(
                        failure.cause(),
                        StoreCanonicalRejection::Requested("original central rejection")
                    ));
                    let validated = match failure
                        .validate_rich_failure(failure_fallback, |session, root| {
                            session
                                .visit_sequence(root, |_, value| value == 29)
                                .unwrap()
                        })
                        .unwrap()
                    {
                        RootedFailureValidation::Valid(validated) => validated,
                        _ => panic!("the original central evidence must remain valid"),
                    };
                    let (prepared, cause, control) = validated
                        .try_prepare(FinalProgress::new(TestProgress::Build, TestUnit::Items, 2))
                        .unwrap()
                        .into_parts_with_control();
                    assert_eq!(control, None);
                    assert!(matches!(
                        cause,
                        StoreCanonicalRejection::Requested("original central rejection")
                    ));
                    prepared
                }
                FiniteCanonicalInvocationOutcome::Interrupted(failure) if interrupt => {
                    let original = failure.cause().clone();
                    assert!(matches!(
                        original.control_cause(),
                        Some(crate::ControlCause::Cancelled { .. })
                    ));
                    let validated = match failure
                        .validate_rich_failure(failure_fallback, |session, root| {
                            session
                                .visit_sequence(root, |_, value| value == 29)
                                .unwrap()
                        })
                        .unwrap()
                    {
                        RootedFailureValidation::Valid(validated) => validated,
                        _ => panic!("interruption must preserve its existing evidence root"),
                    };
                    let (prepared, cause, control) = validated
                        .try_prepare(FinalProgress::new(TestProgress::Build, TestUnit::Items, 2))
                        .unwrap()
                        .into_parts_with_control();
                    assert!(matches!(
                        control,
                        Some(crate::ControlCause::Cancelled { .. })
                    ));
                    assert_eq!(cause, original);
                    prepared
                }
                _ => panic!("Host must restore the original central failure role"),
            };
            let after = account.snapshot();
            assert_eq!(
                after.total_admitted_retained_allocations(),
                admitted.total_admitted_retained_allocations(),
            );
            assert_eq!(
                after.total_admitted_retained_bytes(),
                admitted.total_admitted_retained_bytes(),
            );
            assert_eq!(evidence_builds.get(), 1);
            (prepared, dimensions)
        };
        let PreparedFiniteStoreFailureOutcome::RichFailure(prepared) = prepared else {
            panic!("the already-rooted central failure must retain the rich rail");
        };
        let parts = prepared.handoff();
        assert_eq!(
            parts.outcome_evidence.outcome_evidence().root().as_slice(),
            [29]
        );
        assert!(parts.outcome_evidence.account().is_same_account(&account));
        let StoreTerminalCause::Failure(cause) = parts.facts.header().terminal_cause() else {
            panic!("the central failure must retain its original fixed terminal cause");
        };
        assert_eq!(
            cause.class(),
            if interrupt {
                StorePreparationFailureClass::Control
            } else {
                StorePreparationFailureClass::Requested
            },
        );
        assert_eq!(cause.code(), if interrupt { 1 } else { 104 });
        let work = parts.facts.dimension(dimensions[2]).unwrap();
        assert_eq!(work.committed_canonical_output(), 0);
        assert_eq!(
            parts
                .facts
                .dimension(dimensions[4])
                .unwrap()
                .published_outcome_evidence(),
            1
        );
        drop(parts);
        assert!(!account.snapshot().poisoned());
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
    }
}

#[test]
fn finite_final_failure_preparation_reports_late_control_without_replacing_original_cause() {
    let factory = test_factory();
    let account = test_account(&factory);
    let cancellation = crate::CancellationToken::new();
    let (prepared, dimensions) = {
        let mut host = FailureHost::new();
        let (completion, dimensions, _) = never_completion(
            &mut host,
            &account,
            OperationControl::default().with_cancellation(cancellation.clone()),
            "original failure before final cancellation",
            105,
        );
        let FiniteCanonicalInvocationCompletion {
            outcome,
            failure_fallback,
            ..
        } = completion;
        let FiniteCanonicalInvocationOutcome::NeverActivated(never) = outcome else {
            panic!("late cancellation must start after the original Host failure");
        };
        let built = never
            .build_failure_evidence::<ScalarSequence<u8>, ()>(|lane| {
                let root = lane
                    .try_retain_sequence(crate::SequencePlan::new(1).unwrap(), (), |writer| {
                        writer.push(29_u8)
                    })
                    .unwrap();
                lane.complete(root)
            })
            .unwrap();
        let FailureEvidenceOutcome::Rooted(rooted) = built else {
            panic!("evidence construction must complete before final cancellation");
        };
        let validated = match rooted
            .validate_rich_failure(failure_fallback, |session, root| {
                session
                    .visit_sequence(root, |_, value| value == 29)
                    .unwrap()
            })
            .unwrap()
        {
            RootedFailureValidation::Valid(validated) => validated,
            _ => panic!("cancellation must occur only after the actual evidence proof exists"),
        };
        cancellation.cancel();
        (validated.try_prepare(progress()).unwrap(), dimensions)
    };
    let (prepared, cause, control) = prepared.into_parts_with_control();
    assert!(matches!(
        cause,
        StoreNeverActivatedCause::Requested("original failure before final cancellation")
    ));
    assert!(matches!(
        control,
        Some(crate::ControlCause::Cancelled { .. })
    ));
    let PreparedFiniteStoreFailureOutcome::RichFailure(prepared) = prepared else {
        panic!("final control must preserve the already verified rich-failure evidence");
    };
    let parts = prepared.handoff();
    let StoreTerminalCause::Failure(cause) = parts.facts.header().terminal_cause() else {
        panic!("final cancellation must remain visible in the original fixed terminal facts");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Control);
    assert_eq!(cause.code(), 1);
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        1,
    );
    drop(parts);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_final_validation_cancellation_transfers_the_original_cause_to_runtime_fallback() {
    let factory = test_factory();
    let account = test_account(&factory);
    let drops = Rc::new(Cell::new(0));
    let cancellation = crate::CancellationToken::new();
    let (fallback, dimensions) = {
        let mut host = FailureHost::new();
        let (completion, dimensions, _) = never_completion(
            &mut host,
            &account,
            OperationControl::default().with_cancellation(cancellation.clone()),
            TrackedCause {
                drops: Rc::clone(&drops),
                panic_on_drop: false,
            },
            110,
        );
        let FiniteCanonicalInvocationCompletion {
            outcome,
            failure_fallback,
            ..
        } = completion;
        let FiniteCanonicalInvocationOutcome::NeverActivated(never) = outcome else {
            panic!("the original unopened failure must resume");
        };
        let FailureEvidenceOutcome::Rooted(rooted) = never
            .build_failure_evidence::<ScalarSequence<u8>, ()>(|lane| {
                let root = lane
                    .try_retain_sequence(crate::SequencePlan::new(1).unwrap(), (), |writer| {
                        writer.push(29_u8)
                    })
                    .unwrap();
                lane.complete(root)
            })
            .unwrap()
        else {
            panic!("validation cancellation must start with real rooted evidence");
        };
        assert_eq!(drops.get(), 0);
        let admitted = account.snapshot();
        cancellation.cancel();
        let entered = Cell::new(false);
        let validation = rooted
            .validate_rich_failure(failure_fallback, |_, _| {
                entered.set(true);
                panic!("cancellation must reject before entering the validation callback");
            })
            .unwrap();
        assert!(!entered.get());
        let after = account.snapshot();
        assert_eq!(
            after.total_admitted_retained_allocations(),
            admitted.total_admitted_retained_allocations(),
        );
        assert_eq!(
            after.total_admitted_retained_bytes(),
            admitted.total_admitted_retained_bytes(),
        );
        assert_eq!(drops.get(), 1);
        let RootedFailureValidation::Fallback(fallback) = validation else {
            panic!("controlled validation failure must retain the runtime fallback permission");
        };
        assert!(matches!(
            fallback.cause(),
            StoreRuntimeError::Control(crate::ControlCause::Cancelled { .. })
        ));
        (fallback, dimensions)
    };
    let (prepared, cause) = fallback.into_parts();
    assert!(matches!(
        cause,
        StoreRuntimeError::Control(crate::ControlCause::Cancelled { .. })
    ));
    let parts = prepared.handoff();
    assert!(parts.terminal_account.is_same_account(&account));
    assert!(
        matches!(parts.facts.header().terminal_cause(), StoreTerminalCause::Failure(cause)
        if cause.class() == StorePreparationFailureClass::Control && cause.code() == 1)
    );
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[4])
            .unwrap()
            .published_outcome_evidence(),
        3
    );
    drop(parts);
    assert_eq!(drops.get(), 1);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
}

#[test]
fn finite_final_validation_capture_drop_failure_returns_original_owned_error() {
    struct Capture(Arc<AtomicUsize>);
    impl Drop for Capture {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("finite final validation capture destructor");
        }
    }

    let factory = test_factory();
    let account = test_account(&factory);
    let capture_drops = Arc::new(AtomicUsize::new(0));
    let entered = Cell::new(false);
    let (error, dimensions) = {
        let mut host = FailureHost::new();
        let (completion, dimensions, _) = never_completion(
            &mut host,
            &account,
            OperationControl::default(),
            "original final validation failure",
            106,
        );
        let FiniteCanonicalInvocationCompletion {
            outcome,
            failure_fallback,
            ..
        } = completion;
        let FiniteCanonicalInvocationOutcome::NeverActivated(never) = outcome else {
            panic!("the fault must consume an actual final never-activated owner");
        };
        let built = never
            .build_failure_evidence::<ScalarSequence<u8>, ()>(|lane| {
                let root = lane
                    .try_retain_sequence(crate::SequencePlan::new(1).unwrap(), (), |writer| {
                        writer.push(29_u8)
                    })
                    .unwrap();
                lane.complete(root)
            })
            .unwrap();
        let FailureEvidenceOutcome::Rooted(rooted) = built else {
            panic!("the callback disposal fault must follow actual evidence construction");
        };
        let capture = Capture(Arc::clone(&capture_drops));
        let callback_entered = &entered;
        let error = rooted
            .validate_rich_failure(failure_fallback, move |session, root| {
                let _capture = &capture;
                callback_entered.set(true);
                session
                    .visit_sequence(root, |_, value| value == 29)
                    .unwrap()
            })
            .err()
            .expect("a throwing final callback capture cannot grant a validated continuation");
        assert!(entered.get());
        assert_eq!(capture_drops.load(Ordering::SeqCst), 1);
        (error, dimensions)
    };
    assert!(matches!(
        error.cause(),
        FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Publication(
            crate::concrete_store::StorePrepareError::FinalizationContainment(_)
        ))
    ));
    assert!(error.has_residue());
    assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
    let (_, _, foreign, _) = plans();
    assert!(!error.domain().unwrap().owns_dimension(foreign[0]));
    assert!(account.snapshot().poisoned());
    let first = account.snapshot().first_poison_evidence();
    assert!(first.is_some());
    drop(error);
    assert_eq!(account.snapshot().first_poison_evidence(), first);
    assert_eq!(capture_drops.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
}

#[test]
fn finite_prepared_cause_destructor_fault_quarantines_original_rich_and_minimal_owners() {
    for rich in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let drops = Rc::new(Cell::new(0));
        let mut host = FailureHost::new();
        let first = {
            let (completion, _, _) = never_completion(
                &mut host,
                &account,
                OperationControl::default(),
                TrackedCause {
                    drops: Rc::clone(&drops),
                    panic_on_drop: true,
                },
                107,
            );
            let FiniteCanonicalInvocationCompletion {
                outcome,
                failure_fallback,
                ..
            } = completion;
            let FiniteCanonicalInvocationOutcome::NeverActivated(never) = outcome else {
                panic!("cause quarantine must use the real final failure owner");
            };
            let retained = Cell::new((0, 0));
            let panic = catch_unwind(AssertUnwindSafe(|| {
                if rich {
                    let built = never
                        .build_failure_evidence::<ScalarSequence<u8>, ()>(|lane| {
                            let root = lane
                                .try_retain_sequence(
                                    crate::SequencePlan::new(1).unwrap(),
                                    (),
                                    |writer| writer.push(29_u8),
                                )
                                .unwrap();
                            lane.complete(root)
                        })
                        .unwrap();
                    let FailureEvidenceOutcome::Rooted(rooted) = built else {
                        panic!("the rich owner must retain actual evidence before the cause fault");
                    };
                    let validated = match rooted
                        .validate_rich_failure(failure_fallback, |session, root| {
                            session
                                .visit_sequence(root, |_, value| value == 29)
                                .unwrap()
                        })
                        .unwrap()
                    {
                        RootedFailureValidation::Valid(validated) => validated,
                        _ => panic!("cause disposal must follow successful evidence validation"),
                    };
                    let prepared = validated.try_prepare(progress()).unwrap();
                    assert!(matches!(
                        prepared.cause(),
                        StoreNeverActivatedCause::Requested(cause)
                            if Rc::ptr_eq(&cause.drops, &drops)
                    ));
                    let live = account.snapshot();
                    retained.set((live.live_retained_bytes(), live.live_retained_allocations()));
                    drop(prepared);
                } else {
                    let prepared = never.prepare_minimal_fallback().unwrap();
                    assert!(matches!(
                        prepared.cause(),
                        StoreNeverActivatedCause::Requested(cause)
                            if Rc::ptr_eq(&cause.drops, &drops)
                    ));
                    let live = account.snapshot();
                    retained.set((live.live_retained_bytes(), live.live_retained_allocations()));
                    drop(prepared);
                }
            }))
            .expect_err("the first prepared-cause destructor panic must resume after quarantine");
            assert_eq!(
                panic.downcast_ref::<&'static str>(),
                Some(&"finite prepared cause destructor")
            );
            assert_eq!(drops.get(), 1);
            let after = account.snapshot();
            assert!(after.poisoned());
            assert!(retained.get().0 > 0 && retained.get().1 > 0);
            assert_eq!(
                (
                    after.live_retained_bytes(),
                    after.live_retained_allocations()
                ),
                retained.get(),
                "unverified prepared physical owners must remain quarantined without refunds",
            );
            let first = after.first_poison_evidence().unwrap();
            assert_eq!(
                first.reason(),
                crate::AccountPoisonReason::OwnershipGraphUnverified
            );
            first
        };
        drop(host);
        assert_eq!(account.snapshot().first_poison_evidence(), Some(first));
        assert_eq!(drops.get(), 1);
    }
}

#[test]
fn healthy_finite_prepared_failure_drops_release_storage_during_unrelated_unwind() {
    for rich in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let drops = Rc::new(Cell::new(0));
        let mut host = FailureHost::new();
        {
            let (completion, _, _) = never_completion(
                &mut host,
                &account,
                OperationControl::default(),
                TrackedCause {
                    drops: Rc::clone(&drops),
                    panic_on_drop: false,
                },
                108,
            );
            let FiniteCanonicalInvocationCompletion {
                outcome,
                failure_fallback,
                ..
            } = completion;
            let FiniteCanonicalInvocationOutcome::NeverActivated(never) = outcome else {
                panic!("unwind cleanup must use an actual final failure owner");
            };
            let panic = catch_unwind(AssertUnwindSafe(|| {
                if rich {
                    let built = never
                        .build_failure_evidence::<ScalarSequence<u8>, ()>(|lane| {
                            let root = lane
                                .try_retain_sequence(
                                    crate::SequencePlan::new(1).unwrap(),
                                    (),
                                    |writer| writer.push(29_u8),
                                )
                                .unwrap();
                            lane.complete(root)
                        })
                        .unwrap();
                    let FailureEvidenceOutcome::Rooted(rooted) = built else {
                        panic!("the healthy rich owner must contain actual retained evidence");
                    };
                    let validated = match rooted
                        .validate_rich_failure(failure_fallback, |session, root| {
                            session
                                .visit_sequence(root, |_, value| value == 29)
                                .unwrap()
                        })
                        .unwrap()
                    {
                        RootedFailureValidation::Valid(validated) => validated,
                        _ => panic!("the unrelated unwind must begin after successful validation"),
                    };
                    let _prepared = validated.try_prepare(progress()).unwrap();
                    assert!(account.snapshot().live_retained_allocations() > 0);
                    panic!("unrelated caller panic");
                } else {
                    let _prepared = never.prepare_minimal_fallback().unwrap();
                    assert!(account.snapshot().live_retained_allocations() > 0);
                    panic!("unrelated caller panic");
                }
            }))
            .expect_err("the caller's unrelated panic must remain the original unwind");
            assert_eq!(
                panic.downcast_ref::<&'static str>(),
                Some(&"unrelated caller panic")
            );
            assert_eq!(drops.get(), 1);
        }
        drop(host);
        assert!(!account.snapshot().poisoned());
        assert_eq!(account.snapshot().poison_event_count(), 0);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }
}
