use super::*;
use crate::concrete_store::activation::FiniteCanonicalOperationPublication;
use crate::concrete_store::tests::{TestOwner, TestProgress, TestUnit, test_account, test_factory};
use std::num::{NonZeroU32, NonZeroU64};

type Host = FiniteCanonicalInvocationHost<TestOwner, TestProgress, TestUnit, (), ()>;

#[test]
fn rejected_finite_start_disposes_body_captures_before_original_uninstalled_control() {
    use std::sync::{Arc, Mutex};

    struct Signal(Arc<Mutex<Vec<&'static str>>>);
    impl crate::CancellationSignal for Signal {
        fn is_cancelled(&self) -> bool {
            false
        }
    }
    impl Drop for Signal {
        fn drop(&mut self) {
            self.0.lock().unwrap().push("control");
        }
    }
    struct Capture(Arc<Mutex<Vec<&'static str>>>);
    impl Drop for Capture {
        fn drop(&mut self) {
            assert!(
                self.0.lock().unwrap().is_empty(),
                "body captures must end before original control"
            );
            self.0.lock().unwrap().push("capture");
        }
    }

    for poisoned_account in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let (ledger, start, dimensions, _) = plans();
        let start = if poisoned_account {
            let _ =
                account.test_record_poison(crate::AccountPoisonReason::OwnershipGraphUnverified);
            start
        } else {
            plans().1
        };
        let observations = Arc::new(Mutex::new(Vec::new()));
        let capture = Capture(Arc::clone(&observations));
        let control =
            OperationControl::default().with_cancellation(Signal(Arc::clone(&observations)));
        let mut host = Host::new();
        let error = host
            .run_bound_body(control, ledger, &account, start, move |_, _| {
                let _capture = &capture;
                panic!("the rejected finite entry must never invoke its body");
            })
            .err()
            .expect("the rejected finite entry must return its owned Start error");
        assert!(matches!(error.cause(), FiniteStoreExitCause::Start(_)));
        assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
        assert_eq!(*observations.lock().unwrap(), ["capture", "control"]);
        assert!(host.store.is_none());
    }
}

pub(in crate::concrete_store) fn plans() -> (
    FiniteStoreLedgerPlan,
    FiniteRetainedStoreStartPlan,
    [StoreDimensionKey; 6],
    [StorePhaseKey; 3],
) {
    plans_with_failure_members(1)
}

fn plans_with_failure_members(
    members: usize,
) -> (
    FiniteStoreLedgerPlan,
    FiniteRetainedStoreStartPlan,
    [StoreDimensionKey; 6],
    [StorePhaseKey; 3],
) {
    plans_with_failure_work(members, 0, 0)
}

pub(in crate::concrete_store) fn plans_with_failure_work(
    members: usize,
    construction: u64,
    validation: u64,
) -> (
    FiniteStoreLedgerPlan,
    FiniteRetainedStoreStartPlan,
    [StoreDimensionKey; 6],
    [StorePhaseKey; 3],
) {
    let catalog = StoreSemanticCatalogId::new(*b"baozi-unit-store", NonZeroU32::MIN);
    let domain = StoreKeyDomain::try_new(catalog).expect("original finite domain");
    let dimensions = [1, 2, 3, 4, 5, 6].map(|local| {
        domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    });
    let phases = [1, 2, 3].map(|local| {
        domain
            .try_admit_phase(StorePhaseSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    });
    let requirement = RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(
        members,
        construction,
        validation,
    )
    .unwrap();
    let failure = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        dimensions[5],
        members,
        construction,
        validation,
        1280 + requirement.attempted_work_limit(),
        1280 + requirement.consumed_work_limit(),
    )
    .unwrap();
    let mut policies = dimensions.map(|dimension| {
        (
            dimension,
            DimensionPolicy::reservable_bytes(256, 256, 256, 1 << 20)
                .with_published_outcome_evidence_limit(256),
        )
    });
    policies[5].1 = requirement.dimension_policy();
    let ledger =
        FiniteStoreLedgerPlan::try_new(domain, 8 << 20, 8 << 20, policies, phases).unwrap();
    let start = FiniteRetainedStoreStartPlan::new(
        RetainedStoreWriterPlan::new(
            phases[2],
            dimensions[0],
            dimensions[1],
            RetainedStoreBatchLimits::new(4, 2).unwrap(),
        ),
        RetainedStoreSharePlan::new(dimensions[2], dimensions[3]),
        StoreFallbackEnvelope::new(23),
        dimensions[4],
        3,
        failure,
    );
    (ledger, start, dimensions, phases)
}

fn progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
}

#[test]
fn finite_store_root_start_body_terminal_ownership() {
    let (facts, dimensions, phases) = {
        let factory = test_factory();
        let account = test_account(&factory);
        let (ledger, start, dimensions, phases) = plans();
        let mut host = Host::new();
        let finish = host
            .run_bound_body(
                OperationControl::default(),
                ledger,
                &account,
                start,
                |mut parts, disposition| {
                    parts
                        .startup
                        .with_action_execution(dimensions[2], |actions| {
                            actions.begin_action(1)?;
                            actions.complete_action()?;
                            Ok::<_, StoreRuntimeError>(())
                        })
                        .unwrap();
                    parts.startup.request_unchanged(disposition).unwrap();
                    Ok(())
                },
            )
            .unwrap();
        let facts = finish.finish_unattached(progress()).unwrap();
        assert!(!account.snapshot().poisoned());
        (facts, dimensions, phases)
    };
    let work = facts.dimension(dimensions[2]).unwrap();
    assert_eq!(work.attempted(), 1);
    assert_eq!(work.consumed(), 1);
    assert_eq!(facts.header().dimension_count(), 6);
    assert_eq!(facts.header().phase_count(), 3);
    assert_eq!(
        facts.phase(phases[0]).unwrap().phase(),
        phases[0].semantic_id()
    );
    let (_, _, foreign, _) = plans();
    assert!(matches!(
        facts.dimension(foreign[2]),
        Err(StoreRuntimeError::ForeignDimensionDomain { .. })
    ));
}

#[test]
fn finite_body_error_moves_original_domain_and_nonzero_work_after_host_end() {
    let (error, dimensions) = {
        let factory = test_factory();
        let account = test_account(&factory);
        let (ledger, start, dimensions, _) = plans();
        let mut host = Host::new();
        let error = match host.run_bound_body(
            OperationControl::default(),
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
                let _forgotten = std::mem::ManuallyDrop::new(parts.startup);
                Ok(())
            },
        ) {
            Err(error) => error,
            Ok(_) => panic!("a forgotten permission is not a terminal disposition"),
        };
        assert!(host.store.is_none());
        (error, dimensions)
    };
    assert!(matches!(error.cause(), FiniteStoreExitCause::Body(_)));
    assert!(error.domain().unwrap().owns_dimension(dimensions[2]));
    let actual = error
        .residue
        .as_ref()
        .unwrap()
        .raw
        .terminal_catalog()
        .unwrap();
    assert_eq!(
        actual
            .dimension_evidence(dimensions[2])
            .unwrap()
            .attempted(),
        1
    );
    assert_eq!(
        actual.dimension_evidence(dimensions[2]).unwrap().consumed(),
        1
    );
}

#[test]
fn finite_returned_body_error_keeps_healthy_original_owner_and_cleans_foreign_host() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Signal(Arc<AtomicUsize>);
    impl crate::CancellationSignal for Signal {
        fn is_cancelled(&self) -> bool {
            false
        }
    }
    impl Drop for Signal {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    for attached_host in [false, true] {
        let original_factory = test_factory();
        let original_account = test_account(&original_factory);
        let original_account_before = original_account.snapshot();
        let original_control_drops = Arc::new(AtomicUsize::new(0));
        let (original_error, dimensions, phases) = {
            let (ledger, start, dimensions, phases) = plans();
            let mut host = Host::new();
            let error = host
                .run_bound_body(
                    OperationControl::default()
                        .with_cancellation(Signal(Arc::clone(&original_control_drops))),
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
                .expect("unattached work without a request returns the actual owned Body error");
            assert!(host.body_entered);
            assert!(host.store.is_none());
            (error, dimensions, phases)
        };
        let FiniteStoreExitCause::Body(cause) = original_error.cause() else {
            panic!("the original real Host must return a Body error");
        };
        assert!(cause.containment().is_none());
        assert!(original_error.has_residue());
        let original_cause = format!("{:?}", original_error.cause());
        let (original_identity, original_work, original_phases) = {
            let root = original_error.residue.as_ref().unwrap();
            assert!(root.raw.has_started());
            assert!(!root.raw.must_retain_domain());
            assert!(root.startup.is_none());
            let actual = root.raw.terminal_catalog().unwrap();
            let work = dimensions.map(|dimension| actual.dimension_evidence(dimension).unwrap());
            assert_eq!((work[2].attempted(), work[2].consumed()), (1, 1));
            (
                actual.identity(),
                work,
                phases.map(|phase| actual.phase_evidence(phase).unwrap()),
            )
        };
        assert_eq!(original_account.snapshot(), original_account_before);
        assert_eq!(original_control_drops.load(Ordering::Relaxed), 0);

        let host_factory = test_factory();
        let host_account = test_account(&host_factory);
        let host_control_drops = Arc::new(AtomicUsize::new(0));
        let (ledger, start, host_dimensions, host_phases) = plans();
        let mut returned_error = Some(original_error);
        let error = {
            let mut host = Host::new();
            let error = host
                .run_bound_body(
                    OperationControl::default()
                        .with_cancellation(Signal(Arc::clone(&host_control_drops))),
                    ledger,
                    &host_account,
                    start,
                    move |mut parts, disposition| {
                        if attached_host {
                            let mut operation = parts.startup.attach().unwrap();
                            operation
                                .with_action_execution(host_dimensions[4], |actions| {
                                    actions.begin_action(1)?;
                                    actions.complete_action()?;
                                    Ok::<_, StoreRuntimeError>(())
                                })
                                .unwrap();
                            let live = operation.physical_storage_snapshot();
                            assert!(live.live_retained_bytes() > 0);
                            assert!(live.live_retained_allocations() > 0);
                            assert!(!live.poisoned());
                            operation.request_unchanged(disposition).unwrap();
                        } else {
                            parts
                                .startup
                                .with_action_execution(host_dimensions[4], |actions| {
                                    actions.begin_action(1)?;
                                    actions.complete_action()?;
                                    Ok::<_, StoreRuntimeError>(())
                                })
                                .unwrap();
                        }
                        Err(returned_error
                            .take()
                            .expect("the original error returns once"))
                    },
                )
                .err()
                .expect("the original error wins over either a missing or installed disposition");
            assert!(host.body_entered);
            assert!(host.store.is_none());
            assert_eq!(host_control_drops.load(Ordering::Relaxed), 1);
            error
        };

        assert_eq!(format!("{:?}", error.cause()), original_cause);
        assert!(error.has_residue());
        assert!(error.unattached.is_none());
        assert!(error.terminal.is_none());
        assert!(error.rejected_domain.is_none());
        let domain = error.domain().unwrap();
        for dimension in dimensions {
            assert!(domain.owns_dimension(dimension));
        }
        for phase in phases {
            assert!(domain.owns_phase(phase));
        }
        for dimension in host_dimensions {
            assert!(!domain.owns_dimension(dimension));
        }
        for phase in host_phases {
            assert!(!domain.owns_phase(phase));
        }
        let root = error.residue.as_ref().unwrap();
        assert!(root.raw.has_started());
        assert!(!root.raw.must_retain_domain());
        assert!(root.startup.is_none());
        let actual = root.raw.terminal_catalog().unwrap();
        assert_eq!(actual.identity(), original_identity);
        assert_eq!(
            dimensions.map(|dimension| actual.dimension_evidence(dimension).unwrap()),
            original_work
        );
        assert_eq!(
            phases.map(|phase| actual.phase_evidence(phase).unwrap()),
            original_phases
        );
        assert_eq!(original_account.snapshot(), original_account_before);
        let host_account_after = host_account.snapshot();
        assert_eq!(host_account_after.live_retained_bytes(), 0);
        assert_eq!(host_account_after.live_retained_allocations(), 0);
        assert!(!host_account_after.poisoned());
        assert_eq!(host_account_after.poison_event_count(), 0);
        assert!(host_account_after.first_poison_evidence().is_none());
        assert_eq!(original_control_drops.load(Ordering::Relaxed), 0);
        assert_eq!(host_control_drops.load(Ordering::Relaxed), 1);
        drop(error);
        assert_eq!(original_control_drops.load(Ordering::Relaxed), 1);
        assert_eq!(host_control_drops.load(Ordering::Relaxed), 1);
        assert_eq!(original_account.snapshot(), original_account_before);
        assert_eq!(host_account.snapshot(), host_account_after);
    }
}

#[test]
fn finite_start_error_retains_original_domain_without_fabricating_facts() {
    let (error, dimensions) = {
        let factory = test_factory();
        let account = test_account(&factory);
        let (ledger, _, dimensions, _) = plans();
        let (_, foreign_start, _, _) = plans();
        let mut host = Host::new();
        let error = match host.run_bound_body(
            OperationControl::default(),
            ledger,
            &account,
            foreign_start,
            |_, _| {
                panic!("foreign start keys reject before the callback");
            },
        ) {
            Err(error) => error,
            Ok(_) => panic!("foreign start keys cannot bind"),
        };
        (error, dimensions)
    };
    assert!(matches!(error.cause(), FiniteStoreExitCause::Start(_)));
    assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
    assert!(
        error
            .residue
            .as_ref()
            .unwrap()
            .raw
            .terminal_catalog()
            .is_none()
    );
}

#[test]
fn finite_attached_tableless_rails_use_the_production_host() {
    for minimal in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let (ledger, start, dimensions, _) = plans();
        let mut host = Host::new();
        let finish = host
            .run_bound_body(
                OperationControl::default(),
                ledger,
                &account,
                start,
                |parts, disposition| {
                    let mut operation = parts.startup.attach().unwrap();
                    operation
                        .with_action_execution(dimensions[2], |actions| {
                            actions.begin_action(1)?;
                            actions.complete_action()?;
                            Ok::<_, StoreRuntimeError>(())
                        })
                        .unwrap();
                    if minimal {
                        operation.request_minimal_fallback(disposition, 71).unwrap();
                    } else {
                        operation.request_unchanged(disposition).unwrap();
                    }
                    Ok(())
                },
            )
            .unwrap();
        let outcome = finish.finish_attached(progress()).unwrap();
        let facts = match outcome {
            FiniteCanonicalOperationPublication::EvidenceOnly(
                PreparedFiniteStoreEvidenceOnlyOutcome::Success(prepared),
            ) if !minimal => prepared.handoff().facts,
            FiniteCanonicalOperationPublication::MinimalFallback(prepared) if minimal => {
                let parts = prepared.handoff();
                assert_eq!(
                    parts
                        .facts
                        .dimension(dimensions[4])
                        .unwrap()
                        .published_outcome_evidence(),
                    3
                );
                parts.facts
            }
            _ => panic!("the original finite tableless rail must be preserved"),
        };
        assert!(facts.dimension(dimensions[2]).unwrap().attempted() > 0);
        assert!(!account.snapshot().poisoned());
    }
}

fn publication_plan(
    dimensions: [StoreDimensionKey; 6],
    phases: [StorePhaseKey; 3],
) -> RetainedStorePublicationPlan {
    let table = || {
        RetainedStoreBlockTablePlan::new(
            RetainedStoreWriterPlan::new(
                phases[1],
                dimensions[0],
                dimensions[1],
                RetainedStoreBatchLimits::new(2, 2).unwrap(),
            ),
            1,
            1,
        )
        .unwrap()
    };
    RetainedStorePublicationPlan::success(table(), table())
}

fn publication_graph<L>(
    assembly: &mut FiniteRetainedStorePublicationAssembly<'_, TestOwner, TestProgress, TestUnit, L>,
) -> (
    CanonicalSequenceRef<u8, ()>,
    OutcomeEvidenceSequenceRef<u8, ()>,
) {
    let canonical = assembly
        .ordinary()
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), (), |writer| {
            writer.push(17)
        })
        .unwrap();
    let evidence = assembly
        .ordinary()
        .try_retain_outcome_evidence_sequence(SequencePlan::<u8>::new(1).unwrap(), (), |writer| {
            writer.push(29)
        })
        .unwrap();
    (canonical, evidence)
}

#[test]
fn finite_primary_and_final_cancellation_keep_original_outputs_and_facts_after_root_exit() {
    for cancel in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let cancellation = crate::CancellationToken::new();
        let (output, dimensions) = {
            let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
            let mut entry = FiniteEntryAdmission::new(&account);
            let mut control =
                Some(OperationControl::default().with_cancellation(cancellation.clone()));
            let (ledger, start, dimensions, phases) = plans();
            let mut operation = bind_finite_store(
                &mut root,
                &mut entry,
                &mut control,
                ledger,
                None,
                start,
                FreshStoreLineage { _private: () },
            )
            .unwrap()
            .attach()
            .unwrap();
            operation
                .with_action_execution(dimensions[4], |actions| {
                    actions.begin_action(1)?;
                    actions.complete_action()?;
                    Ok::<_, StoreRuntimeError>(())
                })
                .unwrap();
            let mut assembly = operation
                .try_publication_assembly(publication_plan(dimensions, phases))
                .unwrap();
            let (canonical, evidence) = publication_graph(&mut assembly);
            assembly
                .ordinary()
                .try_commit_canonical(dimensions[2], 9)
                .unwrap();
            assembly
                .ordinary()
                .try_commit_outcome_evidence(dimensions[2], 6)
                .unwrap();
            let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
                session.canonical_sequence(c).is_some()
                    && session.outcome_evidence_sequence(e).is_some()
            }) {
                FiniteStoreSuccessValidation::Valid(validated) => validated,
                _ => panic!("the exact finite primary graph must validate"),
            };
            if cancel {
                cancellation.cancel();
            }
            (validated.try_prepare(progress()).unwrap(), dimensions)
        };
        assert!(!account.snapshot().poisoned());
        let facts = match output {
            PreparedFiniteStoreSuccessOutcome::Success(prepared) if !cancel => {
                let before = account.snapshot();
                let parts = prepared.handoff();
                assert_eq!(account.snapshot(), before);
                assert_eq!(parts.canonical.root().as_slice(), [17]);
                assert_eq!(parts.outcome_evidence.root().as_slice(), [29]);
                assert_eq!(
                    parts
                        .facts
                        .dimension(dimensions[2])
                        .unwrap()
                        .committed_canonical_output(),
                    9
                );
                parts.facts
            }
            PreparedFiniteStoreSuccessOutcome::RichFailure(prepared) if cancel => {
                let parts = prepared.handoff();
                assert_eq!(
                    parts.outcome_evidence.outcome_evidence().root().as_slice(),
                    [29]
                );
                let StoreTerminalCause::Failure(cause) = parts.facts.header().terminal_cause()
                else {
                    panic!("final cancellation must preserve a control failure");
                };
                assert_eq!(cause.class(), StorePreparationFailureClass::Control);
                assert_eq!(cause.code(), 1);
                assert_eq!(
                    parts
                        .facts
                        .dimension(dimensions[2])
                        .unwrap()
                        .committed_canonical_output(),
                    0
                );
                parts.facts
            }
            _ => panic!("the finite primary request must preserve its terminal disposition"),
        };
        assert_eq!(facts.header().dimension_count(), 6);
        assert_eq!(facts.header().phase_count(), 3);
        assert_eq!(facts.dimension(dimensions[4]).unwrap().consumed(), 1);
        assert_eq!(
            facts
                .dimension(dimensions[2])
                .unwrap()
                .published_outcome_evidence(),
            6
        );
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }
}

#[test]
fn finite_rejected_primary_retains_rich_evidence_and_requested_cause_after_root_exit() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (output, dimensions) = {
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut entry = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, phases) = plans();
        let operation = bind_finite_store(
            &mut root,
            &mut entry,
            &mut control,
            ledger,
            None,
            start,
            FreshStoreLineage { _private: () },
        )
        .unwrap()
        .attach()
        .unwrap();
        let mut assembly = operation
            .try_publication_assembly(publication_plan(dimensions, phases))
            .unwrap();
        let (canonical, evidence) = publication_graph(&mut assembly);
        assembly
            .ordinary()
            .try_commit_canonical(dimensions[2], 9)
            .unwrap();
        assembly
            .ordinary()
            .try_commit_outcome_evidence(dimensions[2], 6)
            .unwrap();
        let failure = match assembly.validate_success(canonical, evidence, |_, _, _| false) {
            FiniteStoreSuccessValidation::Rejected(failure) => failure,
            _ => panic!("semantic rejection must retain finite evidence authority"),
        };
        assert_eq!(failure.evidence_root(), evidence);
        let validated = match failure
            .validate_rich_failure(evidence, |session, root| session.sequence(root).is_some())
        {
            FiniteStoreFailureValidation::Valid(validated) => validated,
            _ => panic!("the exact rejected primary evidence remains valid"),
        };
        (validated.try_prepare(71, progress()).unwrap(), dimensions)
    };
    let parts = match output {
        PreparedFiniteStoreFailureOutcome::RichFailure(prepared) => prepared.handoff(),
        _ => panic!("verified evidence must publish through the rich finite rail"),
    };
    assert_eq!(
        parts.outcome_evidence.outcome_evidence().root().as_slice(),
        [29]
    );
    let StoreTerminalCause::Failure(cause) = parts.facts.header().terminal_cause() else {
        panic!("rich finite failure must retain its requested cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), 71);
    let actual = parts.facts.dimension(dimensions[2]).unwrap();
    assert_eq!(actual.committed_canonical_output(), 0);
    assert_eq!(actual.published_outcome_evidence(), 6);
    drop(parts);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_controlled_invalid_dimension_returns_original_writable_assembly() {
    let factory = test_factory();
    let account = test_account(&factory);
    let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
    let mut entry = FiniteEntryAdmission::new(&account);
    let mut control = Some(OperationControl::default());
    let (ledger, start, dimensions, phases) = plans();
    let foreign = plans().2[0];
    let operation = bind_finite_store(
        &mut root,
        &mut entry,
        &mut control,
        ledger,
        None,
        start,
        FreshStoreLineage { _private: () },
    )
    .unwrap()
    .attach()
    .unwrap();
    let mut assembly = operation
        .try_publication_assembly(publication_plan(dimensions, phases))
        .unwrap();
    let (canonical, evidence) = publication_graph(&mut assembly);
    let before = account.snapshot();
    let mut assembly = match assembly.validate_success_controlled(
        foreign,
        dimensions[3],
        canonical,
        evidence,
        |_, _, _, _| panic!("foreign dimensions must reject before user validation"),
    ) {
        FiniteStoreControlledSuccessValidation::InvalidWorkDimension {
            assembly,
            cause: StoreRuntimeError::ForeignDimensionDomain { dimension },
        } if dimension == foreign => assembly,
        _ => panic!("invalid dimensions must return the original finite envelope"),
    };
    assert_eq!(account.snapshot(), before);
    assert_eq!(assembly.ordinary().canonical_len(), 1);
    assert_eq!(assembly.ordinary().outcome_evidence_len(), 1);
    assembly
        .ordinary()
        .try_commit_canonical(dimensions[2], 9)
        .unwrap();
    let valid = match assembly.validate_success_controlled(
        dimensions[2],
        dimensions[3],
        canonical,
        evidence,
        |session, c, e, _| {
            Ok(session.canonical_sequence(c).is_some()
                && session.outcome_evidence_sequence(e).is_some())
        },
    ) {
        FiniteStoreControlledSuccessValidation::Valid(valid) => valid,
        _ => panic!("the returned original assembly must remain writable and valid"),
    };
    let output = valid.try_prepare(progress()).unwrap();
    let PreparedFiniteStoreSuccessOutcome::Success(prepared) = output else {
        panic!("foreign key rejection must not latch a finite publication failure");
    };
    assert_eq!(
        prepared
            .handoff()
            .facts
            .dimension(dimensions[2])
            .unwrap()
            .committed_canonical_output(),
        9
    );
    assert!(!account.snapshot().poisoned());
}

#[test]
fn finite_publication_open_error_owns_original_domain_and_nonzero_work_after_root_exit() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (error, dimensions) = {
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut entry = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, _) = plans();
        let mut operation = bind_finite_store(
            &mut root,
            &mut entry,
            &mut control,
            ledger,
            None,
            start,
            FreshStoreLineage { _private: () },
        )
        .unwrap()
        .attach()
        .unwrap();
        operation
            .with_action_execution(dimensions[4], |actions| {
                actions.begin_action(1)?;
                actions.complete_action()?;
                Ok::<_, StoreRuntimeError>(())
            })
            .unwrap();
        let (_, _, foreign_dimensions, foreign_phases) = plans();
        let error = match operation
            .try_publication_assembly(publication_plan(foreign_dimensions, foreign_phases))
        {
            Err(error) => error,
            Ok(_) => panic!("foreign table plans must fail before finite publication"),
        };
        (error, dimensions)
    };
    assert!(matches!(
        error.cause(),
        FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::PublicationOpen(
            StoreRuntimeError::ForeignPhaseDomain { .. }
        ))
    ));
    assert!(error.has_residue());
    assert!(error.domain().unwrap().owns_dimension(dimensions[2]));
    let actual = error
        .residue
        .as_ref()
        .unwrap()
        .raw
        .terminal_catalog()
        .unwrap();
    let work = actual.dimension_evidence(dimensions[4]).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (1, 1));
    drop(error);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn finite_failure_evidence_reborrow_preserves_containment_latch_and_original_error_owner() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (error, dimensions) = {
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut entry = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, phases) = plans_with_failure_members(2);
        let operation = bind_finite_store(
            &mut root,
            &mut entry,
            &mut control,
            ledger,
            None,
            start,
            FreshStoreLineage { _private: () },
        )
        .unwrap()
        .attach()
        .unwrap();
        let table = |members| {
            RetainedStoreBlockTablePlan::new(
                RetainedStoreWriterPlan::new(
                    phases[1],
                    dimensions[0],
                    dimensions[1],
                    RetainedStoreBatchLimits::new(members + 1, 2).unwrap(),
                ),
                members,
                members,
            )
            .unwrap()
        };
        let mut assembly = operation
            .try_publication_assembly(RetainedStorePublicationPlan::success(table(1), table(2)))
            .unwrap();
        let (canonical, evidence) = publication_graph(&mut assembly);
        let mut failure = match assembly.validate_success(canonical, evidence, |_, _, _| false) {
            FiniteStoreSuccessValidation::Rejected(failure) => failure,
            _ => panic!("semantic rejection must preserve the original finite evidence fields"),
        };
        let result = failure
            .evidence_loan()
            .unwrap()
            .publication_impl()
            .try_retain_outcome_evidence_records(RecordPlan::new(1, 1).unwrap(), (), |_| {
                panic!("an evidence builder panic must latch containment in actual fields")
            });
        assert!(result.is_err());
        assert!(matches!(
            failure.evidence_loan(),
            Err(StoreRuntimeError::FallbackOnly)
        ));
        let first_poison = account.snapshot().first_poison_evidence();
        let fallback = match failure.validate_rich_failure(evidence, |_, _| {
            panic!("a later evidence reborrow must not reopen contained validation");
        }) {
            FiniteStoreFailureValidation::FallbackOnly(fallback) => fallback,
            _ => panic!("the actual failure-state latch must survive the short permission"),
        };
        let error = fallback
            .prepare_minimal_fallback()
            .err()
            .expect("uncertain finite ownership must return its original owned residue");
        assert_eq!(account.snapshot().first_poison_evidence(), first_poison);
        (error, dimensions)
    };
    assert!(error.has_residue());
    assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
    assert!(account.snapshot().poisoned());
}

#[test]
fn finite_lineage_destructor_panic_retains_actual_root_and_domain_after_validation() {
    struct PanickingLineage;
    impl Drop for PanickingLineage {
        fn drop(&mut self) {
            panic!("finite lineage retirement must precede terminal ownership transfer");
        }
    }

    let factory = test_factory();
    let account = test_account(&factory);
    let (error, dimensions) = {
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut entry = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, phases) = plans();
        let operation = bind_finite_store(
            &mut root,
            &mut entry,
            &mut control,
            ledger,
            None,
            start,
            PanickingLineage,
        )
        .unwrap()
        .attach()
        .unwrap();
        let mut assembly = operation
            .try_publication_assembly(publication_plan(dimensions, phases))
            .unwrap();
        let (canonical, evidence) = publication_graph(&mut assembly);
        let fallback = match assembly.validate_success(canonical, evidence, |session, c, e| {
            session.canonical_sequence(c).is_some()
                && session.outcome_evidence_sequence(e).is_some()
        }) {
            FiniteStoreSuccessValidation::FallbackOnly(fallback) => fallback,
            _ => panic!("lineage destructor failure must prevent a success continuation"),
        };
        let error = fallback
            .prepare_minimal_fallback()
            .err()
            .expect("finite containment must transfer original root/domain before leaving scope");
        (error, dimensions)
    };
    assert!(matches!(
        error.cause(),
        FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Publication(
            StorePrepareError::FinalizationContainment(_)
        ))
    ));
    assert!(error.has_residue());
    assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
    assert!(account.snapshot().poisoned());
}

#[test]
fn finite_failure_fields_use_the_shared_protected_record_lane_and_keep_original_facts() {
    use crate::concrete_store::activation::{
        FailureEvidenceWorkBudget, FailureEvidenceWorkEnvelope, run_failure_evidence_build,
    };

    let factory = test_factory();
    let account = test_account(&factory);
    let (output, dimensions) = {
        let mut root = FiniteStoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
        let mut entry = FiniteEntryAdmission::new(&account);
        let mut control = Some(OperationControl::default());
        let (ledger, start, dimensions, phases) = plans_with_failure_work(2, 1, 1);
        let mut operation = bind_finite_store(
            &mut root,
            &mut entry,
            &mut control,
            ledger,
            None,
            start,
            FreshStoreLineage { _private: () },
        )
        .unwrap()
        .attach()
        .unwrap();
        let (max_evidence_members, construction_work_limit, validation_work_limit) = operation
            .fields
            .raw
            .loan()
            .unwrap()
            .failure_terminal_work_limits()
            .unwrap();
        let mut failure_work = FailureEvidenceWorkBudget::new(FailureEvidenceWorkEnvelope {
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
        });
        let table = |members| {
            RetainedStoreBlockTablePlan::new(
                RetainedStoreWriterPlan::new(
                    phases[1],
                    dimensions[0],
                    dimensions[1],
                    RetainedStoreBatchLimits::new(members + 1, 2).unwrap(),
                ),
                members,
                members,
            )
            .unwrap()
        };
        let mut assembly = operation
            .try_publication_assembly(RetainedStorePublicationPlan::success(table(1), table(2)))
            .unwrap();
        let (canonical, evidence) = publication_graph(&mut assembly);
        let mut failure = match assembly.validate_success(canonical, evidence, |_, _, _| false) {
            FiniteStoreSuccessValidation::Rejected(failure) => failure,
            _ => panic!("semantic rejection must retain the original finite failure owner"),
        };
        let failure_root = run_failure_evidence_build(
            failure.evidence_loan().unwrap(),
            &mut failure_work,
            |lane| {
                let root = lane
                    .try_retain_records(RecordPlan::new(1, 1).unwrap(), (), |writer| {
                        writer.write_row(|row| row.write_sequence_ref(evidence))
                    })
                    .unwrap();
                lane.complete(root)
            },
        )
        .unwrap();
        failure
            .evidence_loan()
            .unwrap()
            .publication_impl()
            .try_commit_outcome_evidence(dimensions[2], 7)
            .unwrap();
        let validated = match failure.validate_rich_failure(failure_root, |session, root| {
            session.records(root).is_some() && session.sequence(evidence).is_some()
        }) {
            FiniteStoreFailureValidation::Valid(validated) => validated,
            _ => panic!("the shared protected writer must retain exact finite evidence ownership"),
        };
        (validated.try_prepare(73, progress()).unwrap(), dimensions)
    };
    let parts = match output {
        PreparedFiniteStoreFailureOutcome::RichFailure(prepared) => prepared.handoff(),
        _ => panic!("the original finite evidence must commit through the rich rail"),
    };
    let StoreTerminalCause::Failure(cause) = parts.facts.header().terminal_cause() else {
        panic!("finite protected evidence must preserve its requested cause");
    };
    assert_eq!(cause.code(), 73);
    assert_eq!(
        parts
            .facts
            .dimension(dimensions[2])
            .unwrap()
            .published_outcome_evidence(),
        7
    );
    let protected = parts.facts.dimension(dimensions[5]).unwrap();
    assert!(protected.consumed() > 0);
    assert_eq!(protected.attempted(), protected.consumed());
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn finite_body_capture_cancellation_precedes_unchanged_terminal_authority() {
    struct CancelOnDrop(crate::CancellationToken);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            self.0.cancel();
        }
    }

    let factory = test_factory();
    let account = test_account(&factory);
    let (facts, dimension) = {
        let cancellation = crate::CancellationToken::new();
        let capture = CancelOnDrop(cancellation.clone());
        let (ledger, start, dimensions, _) = plans();
        let mut host = Host::new();
        let finish = host
            .run_bound_body(
                OperationControl::default().with_cancellation(cancellation.clone()),
                ledger,
                &account,
                start,
                move |parts, disposition| {
                    let _capture = &capture;
                    let mut operation = parts.startup.attach().unwrap();
                    operation
                        .with_action_execution(dimensions[4], |actions| {
                            actions.begin_action(1)?;
                            actions.complete_action()?;
                            Ok::<_, StoreRuntimeError>(())
                        })
                        .unwrap();
                    operation.request_unchanged(disposition).unwrap();
                    Ok(())
                },
            )
            .unwrap();
        assert!(cancellation.is_cancelled());
        let publication = finish.finish_attached(progress()).unwrap();
        let FiniteCanonicalOperationPublication::EvidenceOnly(
            PreparedFiniteStoreEvidenceOnlyOutcome::MinimalFallback(prepared),
        ) = publication
        else {
            panic!("capture cancellation must precede the outer unchanged publication gate");
        };
        (prepared.handoff().facts, dimensions[4])
    };
    let StoreTerminalCause::Failure(cause) = facts.header().terminal_cause() else {
        panic!("original finite facts must retain the capture's control cancellation");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Control);
    assert_eq!(cause.code(), 1);
    assert_eq!(facts.dimension(dimension).unwrap().consumed(), 1);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
