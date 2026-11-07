use super::*;
use crate::concrete_store::activation::{
    CanonicalActivationSessionPolicy, CanonicalFailureRequirements, CanonicalMaterializationRole,
    CanonicalTableShape, CanonicalValidationRole, FiniteCanonicalActivationOutcome,
    RetainedStoreWriterRole,
};
use crate::concrete_store::finite::tests::plans;
use crate::concrete_store::tests::{TestOwner, TestProgress, TestUnit, test_account, test_factory};
use crate::{RetainedStoreBatchLimits, ScalarSequence, StorePhaseKey};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

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

type Outcome<'host> = FiniteCanonicalActivationOutcome<
    'host,
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    &'static str,
    ScalarSequence<u8>,
    (),
>;

#[test]
fn finite_scoped_semantic_rejection_uses_original_slots_and_original_terminal_facts() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (facts, dimensions) = {
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut admission = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, phases) = plans();
        let mut operation = bind_finite_store(
            &mut root,
            &mut admission,
            &mut control,
            ledger,
            None,
            start,
            FreshStoreLineage { _private: () },
        )
        .unwrap()
        .attach()
        .unwrap();
        let identity = operation.invocation_identity();
        let mut output = None;
        let mut never = None;
        let mut rejected = None;
        let order = Cell::new(PayloadOrder::ProofBeforeOutput);
        let owner = operation
            .into_session_outcome_owner(
                policy(dimensions, phases),
                RetainedStoreBatchLimits::new(2, 2).unwrap(),
                failure_requirements(),
            )
            .unwrap();
        let outcome: Outcome<'_> = owner
            .plan_and_activate_scoped(
                CanonicalOutputSlot {
                    value: &mut output,
                    order: &order,
                    identity,
                },
                CanonicalActivationSlots {
                    never: CanonicalCauseSlot {
                        value: &mut never,
                        order: &order,
                        identity,
                    },
                    rejection: CanonicalCauseSlot {
                        value: &mut rejected,
                        order: &order,
                        identity,
                    },
                },
                |mut planning| {
                    let ((), _completion) = planning
                        .with_semantic_work(|semantic| {
                            semantic.run_unit(|| ()).unwrap();
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                    planning.reject("finite semantic rejection", 77)
                },
            )
            .unwrap();
        assert!(matches!(
            outcome,
            FiniteCanonicalActivationOutcome::NeverActivated(_)
        ));
        drop(outcome);
        assert!(output.is_none() && never.is_none() && rejected.is_none());
        let prepared = root.fields().prepare_minimal(77).unwrap();
        (prepared.handoff().facts, dimensions)
    };
    let crate::StoreTerminalCause::Failure(cause) = facts.header().terminal_cause() else {
        panic!("the scoped finite rejection must preserve its terminal cause");
    };
    assert_eq!(cause.code(), 77);
    assert!(facts.dimension(dimensions[2]).unwrap().consumed() > 0);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_scoped_foreign_output_rejects_before_callback_and_owns_original_root() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (error, dimension) = {
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let foreign = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut admission = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, phases) = plans();
        let mut operation = bind_finite_store(
            &mut root,
            &mut admission,
            &mut control,
            ledger,
            None,
            start,
            FreshStoreLineage { _private: () },
        )
        .unwrap()
        .attach()
        .unwrap();
        let identity = operation.invocation_identity();
        let mut output = None;
        let mut never = None;
        let mut rejected = None;
        let order = Cell::new(PayloadOrder::ProofBeforeOutput);
        let owner = operation
            .into_session_outcome_owner(
                policy(dimensions, phases),
                RetainedStoreBatchLimits::new(2, 2).unwrap(),
                failure_requirements(),
            )
            .unwrap();
        let callback_entered = Cell::new(false);
        let result: Result<Outcome<'_>, _> = owner.plan_and_activate_scoped(
            CanonicalOutputSlot {
                value: &mut output,
                order: &order,
                identity: foreign.invocation_identity(),
            },
            CanonicalActivationSlots {
                never: CanonicalCauseSlot {
                    value: &mut never,
                    order: &order,
                    identity,
                },
                rejection: CanonicalCauseSlot {
                    value: &mut rejected,
                    order: &order,
                    identity,
                },
            },
            |_| {
                callback_entered.set(true);
                panic!("foreign output permission must reject before transaction execution");
            },
        );
        let Some(error) = result.err() else {
            panic!("foreign output permission cannot produce a finite continuation");
        };
        assert!(
            !callback_entered.get(),
            "foreign permission must reject before user callback"
        );
        assert!(output.is_none() && never.is_none() && rejected.is_none());
        (error, dimensions[0])
    };
    assert!(error.has_residue());
    assert!(error.domain().unwrap().owns_dimension(dimension));
    assert!(account.snapshot().poisoned());
}

#[test]
fn finite_scoped_capture_destructor_panic_keeps_actual_error_after_root_exit() {
    struct Capture(Arc<AtomicUsize>);
    impl Drop for Capture {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("finite transaction capture disposal must finish before owner transfer");
        }
    }
    let factory = test_factory();
    let account = test_account(&factory);
    let drops = Arc::new(AtomicUsize::new(0));
    let (error, dimension) = {
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut admission = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, phases) = plans();
        let mut operation = bind_finite_store(
            &mut root,
            &mut admission,
            &mut control,
            ledger,
            None,
            start,
            FreshStoreLineage { _private: () },
        )
        .unwrap()
        .attach()
        .unwrap();
        let identity = operation.invocation_identity();
        let mut output = None;
        let mut never = None;
        let mut rejected = None;
        let order = Cell::new(PayloadOrder::ProofBeforeOutput);
        let owner = operation
            .into_session_outcome_owner(
                policy(dimensions, phases),
                RetainedStoreBatchLimits::new(2, 2).unwrap(),
                failure_requirements(),
            )
            .unwrap();
        let capture = Capture(Arc::clone(&drops));
        let result: Result<Outcome<'_>, _> = owner.plan_and_activate_scoped(
            CanonicalOutputSlot {
                value: &mut output,
                order: &order,
                identity,
            },
            CanonicalActivationSlots {
                never: CanonicalCauseSlot {
                    value: &mut never,
                    order: &order,
                    identity,
                },
                rejection: CanonicalCauseSlot {
                    value: &mut rejected,
                    order: &order,
                    identity,
                },
            },
            move |planning| {
                let _capture = &capture;
                planning.reject("finite rejected before capture disposal", 79)
            },
        );
        let Some(error) = result.err() else {
            panic!("capture destructor failure must prevent a finite continuation");
        };
        assert!(output.is_none() && never.is_none() && rejected.is_none());
        (error, dimensions[0])
    };
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert!(error.has_residue());
    assert!(error.domain().unwrap().owns_dimension(dimension));
    assert!(account.snapshot().poisoned());
}

#[test]
fn finite_scoped_materialization_uses_shared_runner_for_activated_and_rejected_owners() {
    use crate::concrete_store::activation::{
        CanonicalActivationSeal, CanonicalSuccessRequirements,
    };
    use crate::concrete_store::tests::TestActivationDescription;
    use std::num::NonZeroU64;

    for reject in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let facts = {
            let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
            let mut admission = FiniteEntryAdmission::new(&account);
            let mut control = Some(OperationControl::default());
            let (ledger, start, dimensions, phases) = plans();
            let mut operation = bind_finite_store(
                &mut root,
                &mut admission,
                &mut control,
                ledger,
                None,
                start,
                FreshStoreLineage { _private: () },
            )
            .unwrap()
            .attach()
            .unwrap();
            let identity = operation.invocation_identity();
            let mut output = None;
            let mut never = None;
            let mut rejected = None;
            let order = Cell::new(PayloadOrder::ProofBeforeOutput);
            let owner = operation
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
            let outcome: Outcome<'_> = owner
                .plan_and_activate_scoped(
                    CanonicalOutputSlot {
                        value: &mut output,
                        order: &order,
                        identity,
                    },
                    CanonicalActivationSlots {
                        never: CanonicalCauseSlot {
                            value: &mut never,
                            order: &order,
                            identity,
                        },
                        rejection: CanonicalCauseSlot {
                            value: &mut rejected,
                            order: &order,
                            identity,
                        },
                    },
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
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                        permit.activate(|mut lane, _| {
                            let canonical = lane
                                .try_retain_sequence(
                                    crate::SequencePlan::<u8>::new(1).unwrap(),
                                    (),
                                    |writer| writer.push(17),
                                )
                                .unwrap();
                            if reject {
                                lane.reject("finite materialization rejection", 83)
                            } else {
                                lane.complete(canonical, ())
                            }
                        })
                    },
                )
                .unwrap();
            match &outcome {
                FiniteCanonicalActivationOutcome::Activated(_) if !reject => {}
                FiniteCanonicalActivationOutcome::Rejected(_) if reject => {}
                _ => panic!("the original pending branch must bind to the same finite owner"),
            }
            drop(outcome);
            assert!(output.is_none() && never.is_none() && rejected.is_none());
            root.fields()
                .prepare_publication_fallback(
                    crate::retained::PreparationFailureCode::domain(83),
                    None,
                )
                .unwrap()
                .handoff()
                .facts
        };
        assert!(matches!(
            facts.header().terminal_cause(),
            crate::StoreTerminalCause::Failure(_)
        ));
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert!(!account.snapshot().poisoned());
    }
}

#[test]
fn finite_session_drop_retires_original_lineage_before_releasing_its_fields() {
    struct Lineage {
        drops: Arc<AtomicUsize>,
        panic: bool,
    }
    impl Drop for Lineage {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
            assert!(!self.panic, "finite session lineage destructor failure");
        }
    }

    for (armed, panic) in [(false, false), (true, false), (false, true)] {
        let factory = test_factory();
        let account = test_account(&factory);
        let drops = Arc::new(AtomicUsize::new(0));
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut admission = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, phases) = plans();
        let operation = bind_finite_store(
            &mut root,
            &mut admission,
            &mut control,
            ledger,
            None,
            start,
            Lineage {
                drops: Arc::clone(&drops),
                panic,
            },
        )
        .unwrap()
        .attach()
        .unwrap();
        let owner = operation
            .into_session_outcome_owner(
                policy(dimensions, phases),
                RetainedStoreBatchLimits::new(2, 2).unwrap(),
                failure_requirements(),
            )
            .unwrap();
        let owner = if armed {
            owner.into_activation_transition()
        } else {
            owner
        };
        drop(owner);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(account.snapshot().poisoned(), armed || panic);
        if armed || panic {
            let error = root.into_error(FiniteStoreExitCause::Finalize(
                FiniteStoreFinalizeCause::ForeignTerminalAuthority,
            ));
            assert!(error.has_residue());
            assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
            drop(error);
        } else {
            let output = root.fields().prepare_minimal(89).unwrap();
            drop(output);
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
            assert_eq!(account.snapshot().live_retained_allocations(), 0);
        }
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn finite_activated_and_never_activated_drop_observe_lineage_panic_with_original_fields() {
    use crate::concrete_store::activation::{
        CanonicalActivationSeal, CanonicalSuccessRequirements,
    };
    use crate::concrete_store::tests::TestActivationDescription;
    use std::num::NonZeroU64;

    struct Lineage(Arc<AtomicUsize>);
    impl Drop for Lineage {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("original finite outcome lineage destructor failure");
        }
    }

    for activate in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let drops = Arc::new(AtomicUsize::new(0));
        let (error, dimension, first_poison) = {
            let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
            let mut admission = FiniteEntryAdmission::new(&account);
            let mut control = Some(OperationControl::default());
            let (ledger, start, dimensions, phases) = plans();
            let mut operation = bind_finite_store(
                &mut root,
                &mut admission,
                &mut control,
                ledger,
                None,
                start,
                Lineage(Arc::clone(&drops)),
            )
            .unwrap()
            .attach()
            .unwrap();
            let identity = operation.invocation_identity();
            let mut output = None;
            let mut never = None;
            let mut rejected = None;
            let order = Cell::new(PayloadOrder::ProofBeforeOutput);
            let owner = operation
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
            let outcome: FiniteCanonicalActivationOutcome<
                '_,
                TestOwner,
                TestProgress,
                TestUnit,
                (),
                &'static str,
                ScalarSequence<u8>,
                (),
                Lineage,
            > = owner
                .plan_and_activate_scoped(
                    CanonicalOutputSlot {
                        value: &mut output,
                        order: &order,
                        identity,
                    },
                    CanonicalActivationSlots {
                        never: CanonicalCauseSlot {
                            value: &mut never,
                            order: &order,
                            identity,
                        },
                        rejection: CanonicalCauseSlot {
                            value: &mut rejected,
                            order: &order,
                            identity,
                        },
                    },
                    |mut planning| {
                        if !activate {
                            return planning.reject("finite never activated", 91);
                        }
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
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                        permit.activate(|mut lane, _| {
                            let canonical = lane
                                .try_retain_sequence(
                                    crate::SequencePlan::<u8>::new(1).unwrap(),
                                    (),
                                    |writer| writer.push(17),
                                )
                                .unwrap();
                            lane.complete(canonical, ())
                        })
                    },
                )
                .unwrap();
            assert_eq!(drops.load(Ordering::Relaxed), 0);
            assert!(!account.snapshot().poisoned());
            match &outcome {
                FiniteCanonicalActivationOutcome::Activated(_) if activate => {}
                FiniteCanonicalActivationOutcome::NeverActivated(_) if !activate => {}
                _ => panic!("the actual original lineage must remain in its selected outcome"),
            }
            drop(outcome);
            assert_eq!(drops.load(Ordering::Relaxed), 1);
            assert!(output.is_none() && never.is_none() && rejected.is_none());
            assert!(account.snapshot().poisoned());
            let first_poison = account.snapshot().first_poison_evidence();
            let error = root.into_error(FiniteStoreExitCause::Finalize(
                FiniteStoreFinalizeCause::ForeignTerminalAuthority,
            ));
            (error, dimensions[0], first_poison)
        };
        assert!(error.has_residue());
        assert!(error.domain().unwrap().owns_dimension(dimension));
        assert_eq!(account.snapshot().first_poison_evidence(), first_poison);
        drop(error);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(account.snapshot().first_poison_evidence(), first_poison);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CentralFault {
    None,
    Body,
    Capture,
    Restoration,
    Lineage,
    ReturnedCapture,
    ReturnedPayload,
}

fn finite_central_candidate_round_trip(fault: CentralFault) {
    use crate::concrete_store::activation::{
        CanonicalActivationSeal, CanonicalSuccessRequirements, CanonicalValidationDecision,
        CanonicalValidationProgress, FiniteCanonicalInvocationOutcome,
    };
    use crate::concrete_store::finite::tests::plans_with_failure_work;
    use crate::concrete_store::tests::TestActivationDescription;
    use std::{cell::RefCell, num::NonZeroU64, rc::Rc};

    struct Tracked(
        &'static str,
        Rc<RefCell<Vec<&'static str>>>,
        bool,
        Option<Arc<AtomicUsize>>,
    );
    impl Drop for Tracked {
        fn drop(&mut self) {
            if let Some(error_control_drops) = self.3.as_ref() {
                assert_eq!(
                    error_control_drops.load(Ordering::SeqCst),
                    0,
                    "the original returned error must outlive every capture and payload"
                );
            }
            self.1.borrow_mut().push(self.0);
            if self.2 {
                panic!("{}", self.0);
            }
        }
    }
    type CentralHost = FiniteCanonicalInvocationHost<
        TestOwner,
        TestProgress,
        TestUnit,
        Tracked,
        Tracked,
        &'static str,
        &'static str,
        (),
        ScalarSequence<u8>,
        (),
        ScalarSequence<u8>,
        (),
    >;
    let factory = test_factory();
    let account = test_account(&factory);
    let events = Rc::new(RefCell::new(Vec::new()));
    let returned_fault = matches!(
        fault,
        CentralFault::ReturnedCapture | CentralFault::ReturnedPayload
    );
    let error_control_drops = Arc::new(AtomicUsize::new(0));
    struct ErrorSignal(Arc<AtomicUsize>);
    impl crate::CancellationSignal for ErrorSignal {
        fn is_cancelled(&self) -> bool {
            false
        }
    }
    impl Drop for ErrorSignal {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let mut transported_error = if returned_fault {
        let original_factory = test_factory();
        let original_account = test_account(&original_factory);
        let (ledger, start, dimensions, _) = plans();
        let mut original_host =
            FiniteCanonicalInvocationHost::<TestOwner, TestProgress, TestUnit, (), ()>::new();
        let error = original_host
            .run_bound_body(
                OperationControl::default()
                    .with_cancellation(ErrorSignal(Arc::clone(&error_control_drops))),
                ledger,
                &original_account,
                start,
                |mut parts, _| {
                    parts
                        .startup
                        .with_action_execution(dimensions[2], |actions| {
                            actions.begin_action(1)?;
                            actions.complete_action()?;
                            Ok::<_, StoreRuntimeError>(())
                        })
                        .unwrap();
                    Ok(())
                },
            )
            .err()
            .expect("actual original work without a request owns its Body error");
        assert!(error.has_residue());
        assert!(error.domain().unwrap().owns_dimension(dimensions[2]));
        assert_eq!(error_control_drops.load(Ordering::SeqCst), 0);
        Some(error)
    } else {
        None
    };
    let probe = || returned_fault.then(|| Arc::clone(&error_control_drops));
    let throwing_payloads = fault != CentralFault::None;
    let mut output_value = Some(Tracked(
        "output",
        Rc::clone(&events),
        throwing_payloads,
        probe(),
    ));
    let mut proof_value = Some(Tracked(
        "proof",
        Rc::clone(&events),
        throwing_payloads,
        probe(),
    ));
    let capture = Tracked(
        "capture",
        Rc::clone(&events),
        matches!(fault, CentralFault::Capture | CentralFault::ReturnedCapture),
        probe(),
    );
    struct Lineage {
        panic: bool,
        events: Rc<RefCell<Vec<&'static str>>>,
    }
    impl Drop for Lineage {
        fn drop(&mut self) {
            if self.panic {
                self.events.borrow_mut().push("lineage");
                panic!("lineage");
            }
        }
    }
    let lineage = Lineage {
        panic: fault == CentralFault::Lineage,
        events: Rc::clone(&events),
    };
    let mut original_error = None;
    let (ledger, start, dimensions, phases) = plans_with_failure_work(1, 1, 1);
    let mut host = CentralHost::new();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let finish = host.run_bound_body_with(
            OperationControl::default(),
            ledger,
            (&account, None, lineage),
            start,
            move |parts, disposition| {
                let _capture = &capture;
                let operation = parts.startup.attach().unwrap();
                let owner = operation
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
                let outcome = owner
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
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                        permit.activate(|mut lane, _| {
                            let root = lane
                                .try_retain_sequence(
                                    crate::SequencePlan::<u8>::new(1).unwrap(),
                                    (),
                                    |writer| writer.push(17),
                                )
                                .unwrap();
                            lane.complete(root, output_value.take().unwrap())
                        })
                    })
                    .unwrap();
                let FiniteCanonicalActivationOutcome::Activated(activated) = outcome else {
                    panic!("the original finite candidate must activate");
                };
                let outcome = activated.validate_centrally(
                    parts.proof,
                    parts.validation,
                    CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
                    |lane, _| {
                        let root = lane
                            .try_retain_sequence(
                                crate::SequencePlan::<u8>::new(1).unwrap(),
                                (),
                                |writer| writer.push(23),
                            )
                            .unwrap();
                        lane.complete(root)
                    },
                    |planning, _| {
                        planning.validate_with(
                            |_| Ok(()),
                            |session, canonical, evidence, &mut ()| {
                                assert!(session.visit_canonical_sequence(
                                    canonical,
                                    1,
                                    |_, value| value == 17
                                )?);
                                assert!(session.visit_outcome_evidence_sequence(
                                    evidence,
                                    1,
                                    |_, value| value == 23
                                )?);
                                Ok(CanonicalValidationDecision::<_, &'static str>::accept(
                                    proof_value.take().unwrap(),
                                ))
                            },
                        )
                    },
                );
                let outcome = outcome?;
                outcome.request_finish(disposition)?;
                if let Some(error) = transported_error.take() {
                    return Err(error);
                }
                if fault == CentralFault::Body {
                    panic!("body");
                }
                Ok(())
            },
        );
        if fault == CentralFault::Lineage {
            original_error = Some(
                finish
                    .err()
                    .expect("the actual owned central error must cross the finite body boundary"),
            );
            return;
        }
        let finish = finish.unwrap();
        assert_eq!(*events.borrow(), ["capture"]);
        if fault == CentralFault::Restoration {
            let Some(FiniteDisposition::Candidate(pending)) = finish.host.pending.as_mut() else {
                panic!("the actual validated candidate remains staged");
            };
            pending.store.domain.identity = std::ptr::null();
        }
        {
            let completion = finish.resume().unwrap();
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("the exact validated finite stage must restore as a candidate");
            };
            assert_eq!(candidate.output().0, "output");
            assert_eq!(candidate.proof().0, "proof");
            drop(candidate);
            assert_eq!(*events.borrow(), ["capture", "proof", "output"]);
        }
    }));
    if fault == CentralFault::None {
        assert!(result.is_ok());
        assert!(!account.snapshot().poisoned());
        drop(host);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        return;
    }
    if fault == CentralFault::Lineage {
        assert!(
            result.is_ok(),
            "the original lineage panic becomes an owned finite error"
        );
        let error = original_error
            .as_ref()
            .expect("the central guard retains its original error");
        assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
        assert_eq!(*events.borrow(), ["lineage", "proof", "output", "capture"]);
    } else {
        let panic =
            result.expect_err("the original body or cleanup panic must escape after all catches");
        let message = panic
            .downcast_ref::<&'static str>()
            .copied()
            .or_else(|| panic.downcast_ref::<String>().map(String::as_str));
        let expected = match fault {
            CentralFault::Body => "body",
            CentralFault::Capture | CentralFault::ReturnedCapture => "capture",
            CentralFault::Restoration | CentralFault::ReturnedPayload => "proof",
            _ => unreachable!(),
        };
        assert_eq!(message, Some(expected));
        assert_eq!(*events.borrow(), ["capture", "proof", "output"]);
    }
    if returned_fault {
        assert_eq!(error_control_drops.load(Ordering::SeqCst), 1);
    }
    assert!(host.output.is_none() && host.proof.is_none() && host.never.is_none());
    assert!(account.snapshot().poisoned());
    let poison = account.snapshot().first_poison_evidence();
    drop(host);
    assert_eq!(account.snapshot().first_poison_evidence(), poison);
    if let Some(error) = original_error {
        assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
        drop(error);
    }
}

#[test]
fn finite_central_candidate_resumes_only_after_capture_cleanup_with_original_payloads() {
    finite_central_candidate_round_trip(CentralFault::None);
}

#[test]
fn finite_staged_candidate_body_and_capture_panics_dispose_both_throwing_payloads() {
    for fault in [CentralFault::Body, CentralFault::Capture] {
        finite_central_candidate_round_trip(fault);
    }
}

#[test]
fn finite_candidate_restoration_fault_clears_throwing_payloads_against_owned_error_root() {
    finite_central_candidate_round_trip(CentralFault::Restoration);
}

#[test]
fn finite_central_lineage_drop_fault_invalidates_candidate_and_returns_original_owned_error() {
    finite_central_candidate_round_trip(CentralFault::Lineage);
}

#[test]
fn finite_returned_error_outlives_throwing_staged_payloads_and_body_capture() {
    for fault in [CentralFault::ReturnedCapture, CentralFault::ReturnedPayload] {
        finite_central_candidate_round_trip(fault);
    }
}

#[test]
fn finite_never_activated_restoration_rejects_foreign_stamp_with_original_owned_error() {
    type NeverHost = FiniteCanonicalInvocationHost<
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        &'static str,
        (),
        (),
        ScalarSequence<u8>,
        (),
    >;
    let factory = test_factory();
    let account = test_account(&factory);
    let (ledger_a, start_a, dimensions_a, phases_a) = plans();
    let (ledger_b, start_b, dimensions_b, phases_b) = plans();
    let mut first = NeverHost::new();
    let mut second = NeverHost::new();
    let finish_a = first
        .run_bound_body(
            OperationControl::default(),
            ledger_a,
            &account,
            start_a,
            |parts, disposition| {
                let owner = parts
                    .startup
                    .attach()
                    .unwrap()
                    .into_session_outcome_owner(
                        policy(dimensions_a, phases_a),
                        RetainedStoreBatchLimits::new(2, 2).unwrap(),
                        failure_requirements(),
                    )
                    .unwrap();
                let outcome: Outcome<'_> = owner
                    .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                        let ((), _completion) = planning
                            .with_semantic_work(|semantic| {
                                semantic.run_unit(|| ()).unwrap();
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                        planning.reject("first original cause", 91)
                    })
                    .unwrap();
                let FiniteCanonicalActivationOutcome::NeverActivated(never) = outcome else {
                    panic!("never activated");
                };
                never.request_finish(disposition)
            },
        )
        .unwrap();
    let finish_b = second
        .run_bound_body(
            OperationControl::default(),
            ledger_b,
            &account,
            start_b,
            |parts, disposition| {
                let owner = parts
                    .startup
                    .attach()
                    .unwrap()
                    .into_session_outcome_owner(
                        policy(dimensions_b, phases_b),
                        RetainedStoreBatchLimits::new(2, 2).unwrap(),
                        failure_requirements(),
                    )
                    .unwrap();
                let outcome: Outcome<'_> = owner
                    .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                        let ((), _completion) = planning
                            .with_semantic_work(|semantic| {
                                semantic.run_unit(|| ()).unwrap();
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                        planning.reject("second original cause", 92)
                    })
                    .unwrap();
                let FiniteCanonicalActivationOutcome::NeverActivated(never) = outcome else {
                    panic!("never activated");
                };
                never.request_finish(disposition)
            },
        )
        .unwrap();
    // Only ownerless stamps are swapped; each original Host keeps its root and payloads.
    let pending_a = finish_a.host.pending.take().unwrap();
    let pending_b = finish_b.host.pending.take().unwrap();
    finish_a.host.pending = Some(pending_b);
    finish_b.host.pending = Some(pending_a);
    let error_a = finish_a
        .resume()
        .err()
        .expect("a foreign stage must return the original owned error");
    let error_b = finish_b
        .resume()
        .err()
        .expect("a foreign stage must return the original owned error");
    assert!(error_a.domain().unwrap().owns_dimension(dimensions_a[0]));
    assert!(!error_a.domain().unwrap().owns_dimension(dimensions_b[0]));
    assert!(error_b.domain().unwrap().owns_dimension(dimensions_b[0]));
    assert!(first.never.is_none() && first.output.is_none() && first.proof.is_none());
    assert!(second.never.is_none() && second.output.is_none() && second.proof.is_none());
    assert!(matches!(error_a.cause(), FiniteStoreExitCause::Finalize(_)));
    assert!(matches!(error_b.cause(), FiniteStoreExitCause::Finalize(_)));
    drop((first, second));
    assert!(error_a.domain().unwrap().owns_dimension(dimensions_a[0]));
    drop((error_a, error_b));
}

#[test]
fn finite_host_restores_each_original_semantic_failure_role_after_body_cleanup() {
    use crate::concrete_store::activation::{
        CanonicalActivationSeal, CanonicalSuccessRequirements, CanonicalValidationDecision,
        CanonicalValidationPreparationError, CanonicalValidationProgress,
        FiniteCanonicalInvocationOutcome,
    };
    use crate::concrete_store::finite::tests::plans_with_failure_work;
    use crate::concrete_store::tests::TestActivationDescription;
    use std::{num::NonZeroU64, sync::atomic::AtomicBool};

    struct Signal(Arc<AtomicBool>);
    impl crate::CancellationSignal for Signal {
        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }
    type FailureHost = FiniteCanonicalInvocationHost<
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        &'static str,
        &'static str,
        (),
        ScalarSequence<u8>,
        (),
        ScalarSequence<u8>,
        (),
    >;
    for branch in 0..6 {
        let factory = test_factory();
        let account = test_account(&factory);
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Signal(Arc::clone(&cancelled));
        let (ledger, start, dimensions, phases) = plans_with_failure_work(1, 1, 1);
        let mut host = FailureHost::new();
        let finish = host
            .run_bound_body(
                OperationControl::default().with_cancellation(signal),
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
                    let outcome: Outcome<'_> = owner
                        .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                            let ((), completion) = planning
                                .with_semantic_work(|semantic| {
                                    semantic.run_unit(|| ()).unwrap();
                                    Ok::<_, ()>(())
                                })
                                .unwrap();
                            if branch == 0 {
                                return planning.reject("original never cause", 81);
                            }
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
                                if branch == 1 {
                                    lane.reject("original activation cause", 82)
                                } else {
                                    lane.complete(root, ())
                                }
                            })
                        })
                        .unwrap();
                    let activated = match outcome {
                        FiniteCanonicalActivationOutcome::NeverActivated(never) if branch == 0 => {
                            return never.request_finish(disposition);
                        }
                        FiniteCanonicalActivationOutcome::Rejected(rejected) if branch == 1 => {
                            return rejected.request_finish(disposition);
                        }
                        FiniteCanonicalActivationOutcome::Activated(activated) => activated,
                        _ => panic!("the actual activation branch must remain unchanged"),
                    };
                    let outcome = activated
                        .validate_centrally(
                            parts.proof,
                            parts.validation,
                            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
                            |lane, _| {
                                if branch == 5 {
                                    crate::retained::fail_next_final_owner_allocation();
                                    let error = lane
                                        .try_retain_sequence(
                                            crate::SequencePlan::<u8>::new(1).unwrap(),
                                            (),
                                            |writer| writer.push(23),
                                        )
                                        .unwrap_err();
                                    assert!(matches!(error, StoreRuntimeError::AllocationFailed));
                                    return lane.abort::<ScalarSequence<u8>, ()>();
                                }
                                let root = lane
                                    .try_retain_sequence(
                                        crate::SequencePlan::<u8>::new(1).unwrap(),
                                        (),
                                        |writer| writer.push(23),
                                    )
                                    .unwrap();
                                lane.complete(root)
                            },
                            |planning, _| {
                                planning.validate_with(
                                    |_| {
                                        if branch == 4 {
                                            Err(CanonicalValidationPreparationError::Runtime {
                                                cause: StoreRuntimeError::AllocationFailed,
                                                stable_code: 89,
                                            })
                                        } else {
                                            Ok(())
                                        }
                                    },
                                    |session, canonical, evidence, &mut ()| {
                                        if branch == 3 {
                                            cancelled.store(true, Ordering::SeqCst);
                                        }
                                        assert!(session.visit_canonical_sequence(
                                            canonical,
                                            1,
                                            |_, value| value == 17
                                        )?);
                                        assert!(session.visit_outcome_evidence_sequence(
                                            evidence,
                                            1,
                                            |_, value| value == 23
                                        )?);
                                        Ok(CanonicalValidationDecision::<(), _>::reject(
                                            "original central cause",
                                            83,
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
        {
            let completion = finish.resume().unwrap();
            let actual_role = match &completion.outcome {
                FiniteCanonicalInvocationOutcome::Operation(_) => "operation",
                FiniteCanonicalInvocationOutcome::Candidate(_) => "candidate",
                FiniteCanonicalInvocationOutcome::NeverActivated(_) => "never-activated",
                FiniteCanonicalInvocationOutcome::Rejected(_) => "activation-rejected",
                FiniteCanonicalInvocationOutcome::CentralRejected(_) => "central-rejected",
                FiniteCanonicalInvocationOutcome::Interrupted(_) => "interrupted",
                FiniteCanonicalInvocationOutcome::Fallback(_) => "fallback",
            };
            match completion.outcome {
                FiniteCanonicalInvocationOutcome::NeverActivated(never) if branch == 0 => {
                    assert!(matches!(
                        never.cause(),
                        StoreNeverActivatedCause::Requested("original never cause")
                    ))
                }
                FiniteCanonicalInvocationOutcome::Rejected(rejected) if branch == 1 => {
                    assert!(matches!(
                        rejected.cause(),
                        StoreCanonicalRejection::Requested("original activation cause")
                    ))
                }
                FiniteCanonicalInvocationOutcome::CentralRejected(rejected) if branch == 2 => {
                    assert!(matches!(
                        rejected.cause(),
                        StoreCanonicalRejection::Requested("original central cause")
                    ))
                }
                FiniteCanonicalInvocationOutcome::Interrupted(interrupted) if branch == 3 => {
                    assert!(interrupted.cause().control_cause().is_some())
                }
                FiniteCanonicalInvocationOutcome::CentralRejected(rejected) if branch == 4 => {
                    assert!(matches!(
                        rejected.cause(),
                        StoreCanonicalRejection::Runtime(StoreRuntimeError::AllocationFailed)
                    ));
                }
                FiniteCanonicalInvocationOutcome::Fallback(fallback) if branch == 5 => {
                    assert!(matches!(
                        fallback.cause(),
                        StoreRuntimeError::AllocationFailed
                    ));
                    let (prepared, cause) =
                        fallback.prepare_minimal_fallback().unwrap().into_parts();
                    assert!(matches!(cause, StoreRuntimeError::AllocationFailed));
                    let parts = prepared.handoff();
                    assert!(matches!(
                        parts.facts.header().terminal_cause(),
                        crate::StoreTerminalCause::Failure(cause)
                            if cause.class() == crate::StorePreparationFailureClass::ProductGraph
                    ));
                    drop(parts);
                }
                _ => panic!("finite branch {branch} restored as {actual_role} after cleanup"),
            }
        }
        drop(host);
        assert!(!account.snapshot().poisoned());
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}

#[test]
fn finite_never_request_lineage_panic_disposes_throwing_cause_without_replacing_first_panic() {
    use std::{cell::RefCell, rc::Rc};
    struct Cause(Rc<RefCell<Vec<&'static str>>>);
    impl Drop for Cause {
        fn drop(&mut self) {
            self.0.borrow_mut().push("cause");
            panic!("secondary cause destructor");
        }
    }
    struct Lineage(Rc<RefCell<Vec<&'static str>>>);
    impl Drop for Lineage {
        fn drop(&mut self) {
            self.0.borrow_mut().push("lineage");
            panic!("original request lineage");
        }
    }
    type NeverHost = FiniteCanonicalInvocationHost<
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        Cause,
        (),
        (),
        ScalarSequence<u8>,
        (),
        (),
        (),
    >;
    let factory = test_factory();
    let account = test_account(&factory);
    let events = Rc::new(RefCell::new(Vec::new()));
    let lineage = Lineage(Rc::clone(&events));
    let mut cause = Some(Cause(Rc::clone(&events)));
    let (ledger, start, dimensions, phases) = plans();
    let mut host = NeverHost::new();
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let result = host.run_bound_body_with(
            OperationControl::default(),
            ledger,
            (&account, None, lineage),
            start,
            move |parts, disposition| {
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
                let outcome: FiniteCanonicalActivationOutcome<
                    '_,
                    TestOwner,
                    TestProgress,
                    TestUnit,
                    (),
                    Cause,
                    ScalarSequence<u8>,
                    (),
                    Lineage,
                > = owner
                    .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                        let ((), _completion) = planning
                            .with_semantic_work(|semantic| {
                                semantic.run_unit(|| ()).unwrap();
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                        planning.reject(cause.take().unwrap(), 93)
                    })
                    .unwrap();
                let FiniteCanonicalActivationOutcome::NeverActivated(never) = outcome else {
                    panic!("the real unopened finite operation must remain never-activated");
                };
                never.request_finish(disposition)
            },
        );
        assert!(
            result.is_err(),
            "lineage disposal cannot grant a healthy finish"
        );
    }))
    .expect_err("the original request lineage panic must resume after all cleanup");
    assert_eq!(
        panic.downcast_ref::<&'static str>(),
        Some(&"original request lineage")
    );
    assert_eq!(*events.borrow(), ["lineage", "cause"]);
    assert!(host.never.is_none() && host.output.is_none() && host.proof.is_none());
    assert!(host.store.is_none());
    assert!(account.snapshot().poisoned());
    let first = account.snapshot().first_poison_evidence();
    drop(host);
    assert_eq!(account.snapshot().first_poison_evidence(), first);
}

#[test]
fn finite_returned_error_is_guarded_before_unconsumed_entry_lineage_panics() {
    struct ErrorSignal(Arc<AtomicUsize>);
    impl crate::CancellationSignal for ErrorSignal {
        fn is_cancelled(&self) -> bool {
            false
        }
    }
    impl Drop for ErrorSignal {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct EntryLineage(Arc<AtomicUsize>);
    impl Drop for EntryLineage {
        fn drop(&mut self) {
            assert_eq!(
                self.0.load(Ordering::SeqCst),
                0,
                "the owned error must be installed before the local entry destructor"
            );
            panic!("original entry lineage panic");
        }
    }
    struct Capture(Arc<AtomicUsize>, Arc<AtomicUsize>);
    impl Drop for Capture {
        fn drop(&mut self) {
            assert_eq!(
                self.0.load(Ordering::SeqCst),
                0,
                "the owned error must survive the entry panic and capture teardown"
            );
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
    let error_drops = Arc::new(AtomicUsize::new(0));
    let capture_drops = Arc::new(AtomicUsize::new(0));
    let factory = test_factory();
    let account = test_account(&factory);
    let (ledger, start, dimensions, _) = plans();
    let mut original_host =
        FiniteCanonicalInvocationHost::<TestOwner, TestProgress, TestUnit, (), ()>::new();
    let original = original_host
        .run_bound_body(
            OperationControl::default().with_cancellation(ErrorSignal(Arc::clone(&error_drops))),
            ledger,
            &account,
            start,
            |mut parts, _| {
                parts
                    .startup
                    .with_action_execution(dimensions[2], |actions| {
                        actions.begin_action(1)?;
                        actions.complete_action()?;
                        Ok::<_, StoreRuntimeError>(())
                    })
                    .unwrap();
                Ok(())
            },
        )
        .err()
        .expect("the first actual root remains owned by its original Body error");
    assert!(original.has_residue());
    let mut original = Some(original);
    let capture = Capture(Arc::clone(&error_drops), Arc::clone(&capture_drops));
    let (ledger, start, _, _) = plans();
    let mut host =
        FiniteCanonicalInvocationHost::<TestOwner, TestProgress, TestUnit, (), ()>::new();
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _finish = host.run_bound_body_with(
            OperationControl::default(),
            ledger,
            (&account, None, EntryLineage(Arc::clone(&error_drops))),
            start,
            move |_parts, _disposition| {
                let _capture = &capture;
                Err(original
                    .take()
                    .expect("the actual original error is returned once"))
            },
        );
    }))
    .expect_err("the original local-entry destructor panic must resume after full cleanup");
    assert_eq!(
        panic.downcast_ref::<&'static str>(),
        Some(&"original entry lineage panic")
    );
    assert_eq!(capture_drops.load(Ordering::SeqCst), 1);
    assert_eq!(error_drops.load(Ordering::SeqCst), 1);
    assert!(host.store.is_none() && host.pending.is_none());
    drop((host, original_host));
    assert_eq!(error_drops.load(Ordering::SeqCst), 1);
    assert_eq!(capture_drops.load(Ordering::SeqCst), 1);
}
