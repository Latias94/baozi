//! Concrete finite publication of the original validated Scene-edit owners.

use baozi_operation::__retained_store_integration::activation::{
    FiniteCanonicalFreezePermit, FiniteCanonicalJointSuccess,
};
use baozi_operation::__retained_store_integration::authority::{
    FiniteStoreExitError, PreparedFiniteStoreFallback, PreparedFiniteStoreSuccess,
};
use baozi_operation::FiniteStoreFacts;

use super::*;

/// The original finite Ready permit for one centrally validated Scene edit.
pub type FiniteSceneEditFreezePermit<'inv, O, PP, U> = FiniteCanonicalFreezePermit<
    'inv,
    O,
    PP,
    U,
    SceneEditOutput,
    SceneEditValidationProof,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    SceneEditEvidenceTag,
>;

/// The committed finite store and Core's matching generation and semantic proof.
#[must_use = "prepared finite Scene-edit success owns the exact committed store"]
pub struct PreparedFiniteSceneEditSuccess {
    token: SceneEditHandoffToken,
    store: PreparedFiniteStoreSuccess<
        RecordBlock,
        SceneDirectorySchema,
        RecordBlock,
        SceneEditEvidenceTag,
    >,
}

impl PreparedFiniteSceneEditSuccess {
    /// Transfers the original physical owners and their sole fixed facts owner.
    pub fn handoff(self) -> FiniteSceneEditPublishedSuccess {
        let published = self.store.handoff();
        let (scene, evidence) = self
            .token
            .handoff_physical(published.canonical, published.outcome_evidence);
        FiniteSceneEditPublishedSuccess {
            scene,
            evidence,
            facts: published.facts,
        }
    }
}

/// The exact prepared finite outcome after generation admission.
#[must_use = "finite Scene-edit publication owns its committed storage and facts"]
#[allow(
    clippy::large_enum_variant,
    reason = "publication transfers the exact finite owners without allocating after commit"
)]
pub enum PreparedFiniteSceneEditPublication {
    Success(PreparedFiniteSceneEditSuccess),
    MinimalFallback { store: PreparedFiniteStoreFallback },
}

/// Published Scene and rooted effects with exactly one qualified finite facts owner.
#[must_use = "published finite Scene-edit success must be transferred to its caller"]
pub struct FiniteSceneEditPublishedSuccess {
    scene: SceneStorage,
    evidence: SceneEditEvidence,
    facts: FiniteStoreFacts,
}

impl FiniteSceneEditPublishedSuccess {
    pub fn into_parts(self) -> (crate::ValidatedScene, SceneEditEvidence, FiniteStoreFacts) {
        (
            crate::ValidatedScene::from_node_scene(self.scene),
            self.evidence,
            self.facts,
        )
    }
}

/// Admits the original recovery slot and proven roots, then reserves one generation.
///
/// The matching joint prepares exact frozen success. Generation exhaustion prepares the
/// original frozen minimal fallback without another progress or control observation.
#[allow(
    clippy::result_large_err,
    reason = "every rejection transfers the original finite error and its actual residue"
)]
pub fn consume_finite_scene_edit_freeze_permit<'inv, O, PP, U>(
    permit: FiniteSceneEditFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<'inv, StoreCanonicalRejection<SceneEditGenerationRejection>>,
) -> Result<PreparedFiniteSceneEditPublication, FiniteStoreExitError<O, PP, U>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    consume_finite_scene_edit_freeze_permit_with_generation_source(
        permit,
        rejection_slot,
        SceneGenerationId::reserve,
    )
}

#[allow(
    clippy::result_large_err,
    reason = "generation admission preserves the original marker-generic finite error"
)]
fn consume_finite_scene_edit_freeze_permit_with_generation_source<'inv, O, PP, U>(
    permit: FiniteSceneEditFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<'inv, StoreCanonicalRejection<SceneEditGenerationRejection>>,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedFiniteSceneEditPublication, FiniteStoreExitError<O, PP, U>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut original_error = None;
    match prepare_finite_scene_edit_publication(
        permit,
        rejection_slot,
        reserve_generation,
        &mut original_error,
    ) {
        Ok(prepared) => {
            assert!(
                original_error.is_none(),
                "successful preparation cannot retain an error"
            );
            Ok(prepared)
        }
        Err(()) => Err(original_error
            .take()
            .expect("the entry owns its original finite error")),
    }
}

// Keep the returned error in the entry once Operation has finished its own cleanup.
fn finite_publication_step<O, PP, U, T>(
    complete: impl FnOnce() -> Result<T, FiniteStoreExitError<O, PP, U>>,
    original_error: &mut Option<FiniteStoreExitError<O, PP, U>>,
) -> Result<T, ()>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match complete() {
        Ok(value) => Ok(value),
        Err(error) => {
            assert!(
                original_error.is_none(),
                "one original finite error per publication"
            );
            *original_error = Some(error);
            Err(())
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "each Operation call returns its actual inline owner before the entry retains it"
)]
fn prepare_finite_scene_edit_publication<'inv, O, PP, U>(
    permit: FiniteSceneEditFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<'inv, StoreCanonicalRejection<SceneEditGenerationRejection>>,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    original_error: &mut Option<FiniteStoreExitError<O, PP, U>>,
) -> Result<PreparedFiniteSceneEditPublication, ()>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (permit, (reserve_generation, (), ())) = finite_publication_step(
        || permit.admit_generation_rejection(&rejection_slot, (reserve_generation, (), ())),
        original_error,
    )?;
    let canonical_root = permit.proof().scene.directory().into_store();
    let evidence_root = permit.proof().evidence_root;
    let (permit, reserve_generation) = finite_publication_step(
        || permit.admit_generation_pairing(canonical_root, evidence_root, reserve_generation),
        original_error,
    )?;
    let generation =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(reserve_generation)) {
            Ok(Ok(generation)) => generation,
            Ok(Err(SceneGenerationError::Exhausted)) => {
                let rejected = finite_publication_step(
                    || {
                        permit.reject_generation(
                            rejection_slot,
                            SceneEditGenerationRejection,
                            SCENE_EDIT_GENERATION_REJECTION_CODE,
                        )
                    },
                    original_error,
                )?;
                let (store, _) = finite_publication_step(
                    || rejected.prepare_minimal_fallback(),
                    original_error,
                )?
                .into_parts();
                return Ok(PreparedFiniteSceneEditPublication::MinimalFallback { store });
            }
            Err(payload) => {
                let _ = permit.contain_generation_fault();
                std::panic::resume_unwind(payload);
            }
        };

    permit.bind_joint_commit(|joint, seal| {
        let FiniteCanonicalJointSuccess {
            output,
            proof,
            store,
        } = finite_publication_step(|| joint.prepare(seal), original_error)?;
        Ok(PreparedFiniteSceneEditPublication::Success(
            PreparedFiniteSceneEditSuccess {
                token: SceneEditHandoffToken {
                    generation,
                    proof,
                    tables: output.scene.into_tables(),
                },
                store,
            },
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    use baozi_operation::__retained_store_integration::activation::{
        CanonicalActivationSeal, CanonicalActivationSessionPolicy, CanonicalMaterializationRole,
        CanonicalValidationDecision, CanonicalValidationProgress, CanonicalValidationRole,
        FiniteCanonicalActivationOutcome, FiniteCanonicalCentralValidationOutcome,
        FiniteCanonicalFreezePermitIssue, FiniteCanonicalInvocationCompletion,
        FiniteCanonicalInvocationHost, FiniteCanonicalInvocationOutcome, RetainedStoreWriterRole,
    };
    use baozi_operation::__retained_store_integration::authority::{
        FiniteRetainedStoreStartPlan, FiniteStoreExitCause, FiniteStoreFinalizeCause,
        RetainedStoreSharePlan, RetainedStoreWriterPlan,
    };
    use baozi_operation::{
        AccountPoisonReason, CancellationOrigin, CancellationSignal, CancellationToken,
        ControlCause, OperationControl, OutcomeEvidenceStoreRef, ProgressDecision, ProgressEvent,
        ProgressObserver, RetainedStoreFailureTerminalWorkPlan, StoreDimensionKey,
        StoreFallbackEnvelope, StorePhaseKey, StorePreparationFailureClass, StoreTerminalCause,
    };

    use crate::SceneValidationLimits;
    use crate::canonical_scene::store::construction::node_scene::edit::tests::try_build_single_trs_scene;
    use crate::canonical_scene::store::construction::node_scene::edit::transaction::{
        SceneEditBootstrap, SceneEditRejection, SceneGeometryPlanStorage, SceneGeometryReplacement,
        resolve_scene_space_conversion, run_finite_scene_edit_body,
    };
    use crate::canonical_scene::store::construction::node_scene::scratch::ScopedNodeSceneValidationScratchOwner;
    use crate::canonical_scene::store::construction::node_scene::tests::{
        test_account, test_factory,
    };
    use crate::canonical_scene::store::construction::{
        SceneConstructionOwner, SceneConstructionPhase, SceneConstructionUnit, final_progress,
    };

    use super::super::tests::finite_scene_edit_plans;
    use super::*;

    type Host = FiniteCanonicalInvocationHost<
        SceneConstructionOwner,
        SceneConstructionPhase,
        SceneConstructionUnit,
        SceneEditOutput,
        SceneEditValidationProof,
        SceneEditRejection,
        SceneEditRejection,
        SceneEditGenerationRejection,
        RecordBlock,
        SceneDirectorySchema,
        RecordBlock,
        SceneEditEvidenceTag,
    >;

    type Completion<'host> = FiniteCanonicalInvocationCompletion<
        'host,
        SceneConstructionOwner,
        SceneConstructionPhase,
        SceneConstructionUnit,
        SceneEditOutput,
        SceneEditValidationProof,
        SceneEditRejection,
        SceneEditRejection,
        SceneEditGenerationRejection,
        RecordBlock,
        SceneDirectorySchema,
        RecordBlock,
        SceneEditEvidenceTag,
    >;

    #[derive(Clone, Copy)]
    enum ProofFault {
        None,
        Canonical,
        Evidence(OutcomeEvidenceStoreRef<RecordBlock, SceneEditEvidenceTag>),
    }

    #[derive(Default)]
    struct WorkCalls {
        planning: Cell<usize>,
        evidence: Cell<usize>,
        validation: Cell<usize>,
    }

    impl WorkCalls {
        fn counts(&self) -> [usize; 3] {
            [
                self.planning.get(),
                self.evidence.get(),
                self.validation.get(),
            ]
        }
    }

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
        fn control(
            self: &Arc<Self>,
        ) -> OperationControl<SceneConstructionPhase, SceneConstructionUnit> {
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

    impl CancellationSignal for Signal {
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

    impl ProgressObserver<SceneConstructionPhase, SceneConstructionUnit> for Observer {
        fn observe(
            &mut self,
            _: ProgressEvent<SceneConstructionPhase, SceneConstructionUnit>,
        ) -> ProgressDecision {
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

    fn candidate_completion<'host>(
        host: &'host mut Host,
        scene: &crate::ValidatedScene,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        edits: Option<&[SceneGeometryReplacement<'_>]>,
        fault: ProofFault,
        calls: &WorkCalls,
        reject_evidence: bool,
    ) -> (
        Completion<'host>,
        [StoreDimensionKey; 6],
        [StorePhaseKey; 3],
    ) {
        let (writer, failure, admission) = SceneEditBootstrap::try_new().unwrap().into_parts();
        let (ledger, _, dimensions, phases) = finite_scene_edit_plans(reject_evidence);
        let requirement =
            RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(4, 4096, 4096)
                .unwrap();
        let plan = FiniteRetainedStoreStartPlan::new(
            RetainedStoreWriterPlan::new(phases[2], dimensions[0], dimensions[1], writer),
            RetainedStoreSharePlan::new(dimensions[2], dimensions[3]),
            StoreFallbackEnvelope::new(23),
            dimensions[4],
            1,
            RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
                dimensions[5],
                4,
                4096,
                4096,
                (5 << 20) + requirement.attempted_work_limit(),
                (5 << 20) + requirement.consumed_work_limit(),
            )
            .unwrap(),
        );
        let mut inputs = Some((writer, failure, admission));
        let finish =
            run_finite_scene_edit_body(host, scene, control, ledger, plan, |parts, disposition| {
                let (writer, failure, admission) = inputs.take().unwrap();
                let mut operation = parts.startup.attach().unwrap();
                let mut mappings = edits.map(|edits| {
                    operation
                        .with_action_execution(dimensions[2], |actions| {
                            SceneGeometryPlanStorage::try_new(
                                actions,
                                phases[0],
                                dimensions[3],
                                edits.len(),
                            )
                        })
                        .unwrap()
                });
                let owner = operation.into_session_outcome_owner(
                    CanonicalActivationSessionPolicy::new(
                        RetainedStoreWriterRole::new(phases[1], dimensions[0], dimensions[1]),
                        RetainedStoreWriterRole::new(phases[1], dimensions[0], dimensions[1]),
                        CanonicalMaterializationRole::new(dimensions[2], dimensions[3]),
                        CanonicalValidationRole::new(phases[2], dimensions[2], dimensions[3]),
                        dimensions[2],
                        dimensions[4],
                    )
                    .with_semantic_bytes_dimension(dimensions[2]),
                    writer,
                    failure,
                )?;
                let mut transaction = if let Some(edits) = edits {
                    admission.into_geometry_transaction(
                        scene,
                        edits,
                        mappings.as_mut().unwrap().scratch(),
                        crate::provenance::OptionsDigest::sha256([91; 32]),
                        SceneValidationLimits::default(),
                    )
                } else {
                    let resolved = resolve_scene_space_conversion(
                        scene,
                        TargetSpace::STANDARD,
                        Some(TargetSpace::STANDARD),
                    )
                    .unwrap();
                    assert!(!resolved.is_unchanged());
                    admission.into_space_transaction(resolved, SceneValidationLimits::default())
                };
                let activated = owner.plan_and_activate_scoped(
                    parts.output,
                    parts.activation,
                    |mut planning| {
                        calls.planning.set(calls.planning.get() + 1);
                        let (sealed, completion) = planning
                            .with_semantic_work(|semantic| transaction.preflight(semantic))
                            .unwrap();
                        match planning.seal_activation(sealed, completion) {
                            CanonicalActivationSeal::Sealed(permit) => {
                                permit.activate(|lane, plan| plan.materialize(lane))
                            }
                            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
                        }
                    },
                )?;
                let postflight = transaction.finish();
                let FiniteCanonicalActivationOutcome::Activated(activated) = activated else {
                    panic!("the finite Core transaction must materialize its actual Scene");
                };
                let central = activated.validate_centrally(
                    parts.proof,
                    parts.validation,
                    CanonicalValidationProgress::new(
                        SceneConstructionPhase::Validation,
                        SceneConstructionUnit::WorkUnits,
                    ),
                    |lane, output| {
                        calls.evidence.set(calls.evidence.get() + 1);
                        postflight.build_evidence(lane, output)
                    },
                    |planning, output| {
                        calls.validation.set(calls.validation.get() + 1);
                        if matches!(fault, ProofFault::None) {
                            return postflight.validate_candidate(planning, output);
                        }
                        let scratch_plan = output.scene.inner().central_validation.scratch_plan();
                        planning.validate_with(
                            |checkpoint| {
                                ScopedNodeSceneValidationScratchOwner::try_allocate(
                                    checkpoint,
                                    scratch_plan,
                                )
                            },
                            |session, directory, evidence, scratch| {
                                let mut proof = postflight
                                    .prove_candidate(session, directory, evidence, scratch, output)?
                                    .expect("the actual metered candidate must validate");
                                match fault {
                                    ProofFault::Canonical => {
                                        proof.scene = scene.as_node_scene().proof
                                    }
                                    ProofFault::Evidence(root) => proof.evidence_root = root,
                                    ProofFault::None => unreachable!(),
                                }
                                Ok(CanonicalValidationDecision::accept(proof))
                            },
                        )
                    },
                )?;
                drop(mappings);
                let FiniteCanonicalCentralValidationOutcome::Candidate(candidate) = central else {
                    panic!("the finite Core transaction must produce its actual Candidate");
                };
                assert!(candidate.canonical_member_count() > 0);
                assert_eq!(
                    candidate.outcome_evidence_member_count(),
                    if edits.is_some() { 4 } else { 2 },
                );
                candidate.request_finish(disposition)?;
                Ok(())
            })
            .unwrap();
        let completion = finish.resume().unwrap();
        (completion, dimensions, phases)
    }

    struct GenerationCapture<'account> {
        account: &'account baozi_operation::PhysicalStorageAccount,
        drops: &'account Cell<usize>,
        retained_before: u64,
        panic_on_drop: bool,
    }

    impl Drop for GenerationCapture<'_> {
        fn drop(&mut self) {
            assert!(self.account.snapshot().live_retained_bytes() > self.retained_before);
            self.drops.set(self.drops.get() + 1);
            if self.panic_on_drop {
                std::panic::panic_any("finite generation capture fault");
            }
        }
    }

    #[test]
    fn finite_space_handoff_keeps_exact_roots_and_facts_after_source_and_host_end() {
        let factory = test_factory();
        let account = test_account(&factory, 1024);
        let generations = AtomicU64::new(0);
        let calls = WorkCalls::default();
        let probe = Arc::new(ControlProbe::default());
        let generation_calls = Cell::new(0);
        let (prepared, input_generation, canonical_root, evidence_root, dimensions, frozen) = {
            let source = try_build_single_trs_scene(&account, || {
                SceneGenerationId::reserve_from(&generations)
            })
            .unwrap();
            let scene = crate::ValidatedScene::from_node_scene(source.scene);
            let input_generation = scene.generation();
            let mut host = Host::new();
            let (completion, dimensions, _) = candidate_completion(
                &mut host,
                &scene,
                probe.control(),
                None,
                ProofFault::None,
                &calls,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("the finite finish must restore the actual Core Candidate");
            };
            assert_ne!(
                candidate.canonical_member_count(),
                candidate.outcome_evidence_member_count()
            );
            let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
                .try_issue_freeze_permit(completion.generation.interruption, final_progress())
                .unwrap()
            else {
                panic!("the actual finite Core Candidate must reach Ready");
            };
            let canonical_root = permit.proof().scene.directory().into_store();
            let evidence_root = permit.proof().evidence_root;
            let frozen = probe.counts();
            assert_eq!(&frozen[2..], [1, 1]);
            probe.cancellation.cancel();
            let prepared = consume_finite_scene_edit_freeze_permit_with_generation_source(
                permit,
                completion.generation.rejection,
                || {
                    generation_calls.set(generation_calls.get() + 1);
                    assert_eq!(probe.counts(), frozen);
                    SceneGenerationId::reserve_from(&generations)
                },
            )
            .unwrap();
            assert_eq!(generation_calls.get(), 1);
            assert_eq!(generations.load(Ordering::Relaxed), 2);
            assert_eq!(probe.counts(), frozen);
            assert_eq!(calls.counts(), [1, 1, 1]);
            assert_eq!(scene.generation(), input_generation);
            (
                prepared,
                input_generation,
                canonical_root,
                evidence_root,
                dimensions,
                frozen,
            )
        };
        let PreparedFiniteSceneEditPublication::Success(prepared) = prepared else {
            panic!("Ready must prepare exact finite success despite later cancellation");
        };
        let before_handoff = account.snapshot();
        let (scene, evidence, facts) = prepared.handoff().into_parts();
        assert_eq!(account.snapshot(), before_handoff);
        assert_ne!(scene.generation(), input_generation);
        assert_eq!(scene.node_count(), 1);
        assert_eq!(scene.as_node_scene().canonical.root_ref(), canonical_root);
        assert_eq!(facts.header().terminal_cause(), StoreTerminalCause::Success);
        assert_eq!(facts.header().dimension_count(), 6);
        assert_eq!(facts.header().phase_count(), 3);
        assert_eq!(
            facts
                .dimension(dimensions[2])
                .unwrap()
                .committed_canonical_output(),
            u64::try_from(scene.canonical_block_count()).unwrap(),
        );
        assert_eq!(
            facts
                .dimension(dimensions[4])
                .unwrap()
                .published_outcome_evidence(),
            2
        );
        assert!(facts.covered_administrative_bytes().unwrap() > 0);
        assert_eq!(facts.original_freeze_control(), None);
        let SceneEditEvidence::SpaceConversion(evidence) = evidence else {
            panic!("finite handoff must preserve the concrete space-conversion evidence");
        };
        assert_eq!(evidence._store.root_ref(), evidence_root);
        assert_eq!(evidence.input_generation(), input_generation);
        assert_eq!(evidence.output_generation(), scene.generation());
        assert_eq!(evidence.source(), TargetSpace::STANDARD);
        assert_eq!(evidence.target(), TargetSpace::STANDARD);
        assert_eq!(evidence.triangle_corner_order(), [0, 1, 2]);
        assert!(evidence.assumed_source_field(SceneSpaceField::Handedness));
        drop(scene);
        assert_eq!(evidence._store.root_ref(), evidence_root);
        assert!(account.snapshot().live_retained_allocations() > 0);
        drop(evidence);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(facts.header().terminal_cause(), StoreTerminalCause::Success);
        assert_eq!(probe.counts(), frozen);
        assert_eq!(calls.counts(), [1, 1, 1]);
    }

    #[test]
    fn finite_generation_exhaustion_preserves_prepared_fallback_and_independent_account() {
        let factory = test_factory();
        let account = test_account(&factory, 1024);
        let generations = AtomicU64::new(0);
        let calls = WorkCalls::default();
        let probe = Arc::new(ControlProbe::default());
        let generation_calls = Cell::new(0);
        let generation_drops = Cell::new(0);
        let (store, dimensions, frozen) = {
            let source = try_build_single_trs_scene(&account, || {
                SceneGenerationId::reserve_from(&generations)
            })
            .unwrap();
            let scene = crate::ValidatedScene::from_node_scene(source.scene);
            let retained_before = account.snapshot().live_retained_bytes();
            let mut host = Host::new();
            let (completion, dimensions, _) = candidate_completion(
                &mut host,
                &scene,
                probe.control(),
                None,
                ProofFault::None,
                &calls,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("generation exhaustion must consume the actual restored Candidate");
            };
            let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
                .try_issue_freeze_permit(completion.generation.interruption, final_progress())
                .unwrap()
            else {
                panic!("generation exhaustion must follow Ready");
            };
            let frozen = probe.counts();
            generations.store(u64::MAX, Ordering::Relaxed);
            let capture = GenerationCapture {
                account: &account,
                drops: &generation_drops,
                retained_before,
                panic_on_drop: false,
            };
            let PreparedFiniteSceneEditPublication::MinimalFallback { store } =
                consume_finite_scene_edit_freeze_permit_with_generation_source(
                    permit,
                    completion.generation.rejection,
                    || {
                        let _capture = capture;
                        generation_calls.set(generation_calls.get() + 1);
                        SceneGenerationId::reserve_from(&generations)
                    },
                )
                .unwrap()
            else {
                panic!("exhaustion must return the actual prepared finite minimal fallback");
            };
            assert_eq!(generation_calls.get(), 1);
            assert_eq!(generation_drops.get(), 1);
            assert_eq!(generations.load(Ordering::Relaxed), u64::MAX);
            assert_eq!(probe.counts(), frozen);
            (store, dimensions, frozen)
        };
        let before_handoff = account.snapshot();
        let published = store.handoff();
        assert_eq!(account.snapshot(), before_handoff);
        let StoreTerminalCause::Failure(cause) = published.facts.header().terminal_cause() else {
            panic!("generation exhaustion must retain its original requested cause");
        };
        assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
        assert_eq!(cause.code(), SCENE_EDIT_GENERATION_REJECTION_CODE);
        assert_eq!(
            published
                .facts
                .dimension(dimensions[2])
                .unwrap()
                .committed_canonical_output(),
            0
        );
        assert_eq!(
            published
                .facts
                .dimension(dimensions[4])
                .unwrap()
                .published_outcome_evidence(),
            1
        );
        assert_eq!(published.facts.original_freeze_control(), None);
        assert!(!published.terminal_account.snapshot().poisoned());
        let facts = published.facts;
        drop(published.fallback);
        drop(published.terminal_account);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert!(matches!(
            facts.header().terminal_cause(),
            StoreTerminalCause::Failure(_)
        ));
        assert_eq!(probe.counts(), frozen);
        assert_eq!(calls.counts(), [1, 1, 1]);
    }

    #[test]
    fn finite_slot_and_both_root_admissions_preserve_owned_errors_before_generation() {
        use baozi_operation::{PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits};

        for mismatch in 0..3 {
            let factory = PhysicalStorageAccountFactory::try_new(
                PhysicalStorageAccountFactoryLimits::new(u64::MAX, 2).unwrap(),
            )
            .unwrap();
            let account = test_account(&factory, 1024);
            let foreign_account = test_account(&factory, 1024);
            let generations = AtomicU64::new(0);
            let generation_calls = Cell::new(0);
            let generation_drops = Cell::new(0);
            let calls = WorkCalls::default();
            let foreign_calls = WorkCalls::default();
            let probe = Arc::new(ControlProbe::default());
            let (error, dimensions, phases, foreign_dimension, frozen, first_poison) = {
                let source = try_build_single_trs_scene(&account, || {
                    SceneGenerationId::reserve_from(&generations)
                })
                .unwrap();
                let scene = crate::ValidatedScene::from_node_scene(source.scene);
                let foreign_source = try_build_single_trs_scene(&foreign_account, || {
                    SceneGenerationId::reserve_from(&generations)
                })
                .unwrap();
                let foreign_scene = crate::ValidatedScene::from_node_scene(foreign_source.scene);
                let mut foreign_host = Host::new();
                let (foreign, foreign_dimensions, _) = candidate_completion(
                    &mut foreign_host,
                    &foreign_scene,
                    OperationControl::default(),
                    None,
                    ProofFault::None,
                    &foreign_calls,
                    false,
                );
                let FiniteCanonicalInvocationOutcome::Candidate(foreign_candidate) =
                    foreign.outcome
                else {
                    panic!(
                        "the foreign slot and root must come from another real finite Candidate"
                    );
                };
                let FiniteCanonicalFreezePermitIssue::Ready(foreign_permit) = foreign_candidate
                    .try_issue_freeze_permit(foreign.generation.interruption, final_progress())
                    .unwrap()
                else {
                    panic!("the foreign Candidate must retain its actual roots");
                };
                let fault = match mismatch {
                    0 => ProofFault::None,
                    1 => ProofFault::Canonical,
                    2 => ProofFault::Evidence(foreign_permit.proof().evidence_root),
                    _ => unreachable!(),
                };
                let retained_before = account.snapshot().live_retained_bytes();
                let mut host = Host::new();
                let (completion, dimensions, phases) = candidate_completion(
                    &mut host,
                    &scene,
                    probe.control(),
                    None,
                    fault,
                    &calls,
                    false,
                );
                let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome
                else {
                    panic!("the original Core Candidate must be restored before admission");
                };
                let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
                    .try_issue_freeze_permit(completion.generation.interruption, final_progress())
                    .unwrap()
                else {
                    panic!("root admission must run after the actual Ready decision");
                };
                let frozen = probe.counts();
                let rejection = if mismatch == 0 {
                    foreign.generation.rejection
                } else {
                    completion.generation.rejection
                };
                let capture = GenerationCapture {
                    account: &account,
                    drops: &generation_drops,
                    retained_before,
                    panic_on_drop: false,
                };
                let error = consume_finite_scene_edit_freeze_permit_with_generation_source(
                    permit,
                    rejection,
                    || {
                        let _capture = capture;
                        generation_calls.set(generation_calls.get() + 1);
                        SceneGenerationId::reserve_from(&generations)
                    },
                )
                .err()
                .expect("a foreign slot or root must return the original owned finite error");
                assert_eq!(generation_calls.get(), 0);
                assert_eq!(generation_drops.get(), 1);
                assert_eq!(generations.load(Ordering::Relaxed), 2);
                assert_eq!(probe.counts(), frozen);
                let first_poison = account.snapshot().first_poison_evidence().unwrap();
                assert_eq!(
                    first_poison.reason(),
                    AccountPoisonReason::OwnershipGraphUnverified
                );
                drop(foreign_permit);
                (
                    error,
                    dimensions,
                    phases,
                    foreign_dimensions[0],
                    frozen,
                    first_poison,
                )
            };
            let FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Publication(
                StorePrepareError::FinalizationContainment(cause),
            )) = error.cause()
            else {
                panic!("the Core consumer must preserve the original finite containment cause");
            };
            assert_eq!(
                cause.reason(),
                AccountPoisonReason::OwnershipGraphUnverified
            );
            assert!(error.has_residue());
            assert_eq!(error.original_freeze_control(), None);
            let domain = error.domain().unwrap();
            for dimension in dimensions {
                assert_eq!(
                    domain.try_admit_dimension(dimension.semantic_id()),
                    Some(dimension)
                );
            }
            for phase in phases {
                assert_eq!(domain.try_admit_phase(phase.semantic_id()), Some(phase));
            }
            assert_ne!(
                domain.try_admit_dimension(foreign_dimension.semantic_id()),
                Some(foreign_dimension)
            );
            assert_eq!(
                account.snapshot().first_poison_evidence(),
                Some(first_poison)
            );
            assert!(account.snapshot().live_retained_bytes() > 0);
            let retained = account.snapshot();
            drop(error);
            assert_eq!(account.snapshot(), retained);
            assert_eq!(probe.counts(), frozen);
            assert_eq!(calls.counts(), [1, 1, 1]);
            assert_eq!(foreign_account.snapshot().live_retained_bytes(), 0);
        }
    }

    #[test]
    fn finite_generation_and_capture_unwind_contain_original_permit_and_burn_once() {
        for panic_on_drop in [false, true] {
            let factory = test_factory();
            let account = test_account(&factory, 1024);
            let generations = AtomicU64::new(0);
            let calls = WorkCalls::default();
            let probe = Arc::new(ControlProbe::default());
            let generation_calls = Cell::new(0);
            let generation_drops = Cell::new(0);
            let frozen = {
                let source = try_build_single_trs_scene(&account, || {
                    SceneGenerationId::reserve_from(&generations)
                })
                .unwrap();
                let scene = crate::ValidatedScene::from_node_scene(source.scene);
                let retained_before = account.snapshot().live_retained_bytes();
                let mut host = Host::new();
                let (completion, _, _) = candidate_completion(
                    &mut host,
                    &scene,
                    probe.control(),
                    None,
                    ProofFault::None,
                    &calls,
                    false,
                );
                let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome
                else {
                    panic!("the generation fault must follow the real finite Candidate");
                };
                let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
                    .try_issue_freeze_permit(completion.generation.interruption, final_progress())
                    .unwrap()
                else {
                    panic!("the generation fault must follow Ready");
                };
                let frozen = probe.counts();
                let capture = GenerationCapture {
                    account: &account,
                    drops: &generation_drops,
                    retained_before,
                    panic_on_drop,
                };
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    consume_finite_scene_edit_freeze_permit_with_generation_source(
                        permit,
                        completion.generation.rejection,
                        || {
                            let _capture = capture;
                            generation_calls.set(generation_calls.get() + 1);
                            let generation = SceneGenerationId::reserve_from(&generations).unwrap();
                            if !panic_on_drop {
                                std::panic::panic_any("finite generation source fault");
                            }
                            Ok(generation)
                        },
                    )
                }))
                .err()
                .expect("the original generation panic must resume after finite containment");
                assert_eq!(
                    panic.downcast_ref::<&str>(),
                    Some(&if panic_on_drop {
                        "finite generation capture fault"
                    } else {
                        "finite generation source fault"
                    }),
                );
                assert_eq!(generation_calls.get(), 1);
                assert_eq!(generation_drops.get(), 1);
                assert_eq!(generations.load(Ordering::Relaxed), 2);
                assert_eq!(probe.counts(), frozen);
                frozen
            };
            let retained = account.snapshot();
            assert!(retained.poisoned());
            assert_eq!(
                retained.first_poison_evidence().unwrap().reason(),
                AccountPoisonReason::OwnershipGraphUnverified
            );
            assert!(retained.live_retained_bytes() > 0);
            assert_eq!(probe.counts(), frozen);
            assert_eq!(calls.counts(), [1, 1, 1]);
        }
    }

    #[test]
    fn finite_non_ready_core_candidates_keep_minimal_policy_and_detailed_control() {
        for interrupted in [false, true] {
            let factory = test_factory();
            let account = test_account(&factory, 1024);
            let generations = AtomicU64::new(0);
            let calls = WorkCalls::default();
            let probe = Arc::new(ControlProbe::default());
            let expected = ControlCause::Cancelled {
                origin: CancellationOrigin::Observer,
            };
            let store = {
                let source = try_build_single_trs_scene(&account, || {
                    SceneGenerationId::reserve_from(&generations)
                })
                .unwrap();
                let scene = crate::ValidatedScene::from_node_scene(source.scene);
                let mut host = Host::new();
                let (completion, _, _) = candidate_completion(
                    &mut host,
                    &scene,
                    probe.control(),
                    None,
                    ProofFault::None,
                    &calls,
                    !interrupted,
                );
                let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome
                else {
                    panic!("the non-Ready decision must come from the actual Core Candidate");
                };
                let before_issue = probe.counts();
                probe
                    .cancel_on_progress
                    .store(interrupted, Ordering::SeqCst);
                let issue = candidate
                    .try_issue_freeze_permit(completion.generation.interruption, final_progress())
                    .unwrap();
                let store = match issue {
                    FiniteCanonicalFreezePermitIssue::Interrupted(failure) if interrupted => {
                        assert_eq!(failure.cause().control_cause(), Some(&expected));
                        let frozen = probe.counts();
                        assert_eq!(frozen[1], before_issue[1] + 1);
                        let (store, _) = failure.prepare_minimal_fallback().unwrap().into_parts();
                        assert_eq!(probe.counts(), frozen);
                        store
                    }
                    FiniteCanonicalFreezePermitIssue::DestinationRejected(failure)
                        if !interrupted =>
                    {
                        assert_eq!(probe.counts(), before_issue);
                        assert_eq!(failure.cause().control_cause(), None);
                        failure.prepare_minimal_fallback().unwrap().into_parts().0
                    }
                    _ => panic!(
                        "the actual non-Ready rail must preserve its original failure typestate"
                    ),
                };
                assert_eq!(generations.load(Ordering::Relaxed), 1);
                assert_eq!(calls.counts(), [1, 1, 1]);
                store
            };
            let published = store.handoff();
            assert_eq!(
                published.facts.original_freeze_control(),
                interrupted.then_some(&expected)
            );
            let StoreTerminalCause::Failure(cause) = published.facts.header().terminal_cause()
            else {
                panic!("the non-Ready decision must keep its original terminal cause");
            };
            assert_eq!(
                cause.class(),
                if interrupted {
                    StorePreparationFailureClass::Control
                } else {
                    StorePreparationFailureClass::Ledger
                }
            );
            drop(published.fallback);
            drop(published.terminal_account);
            assert!(!account.snapshot().poisoned());
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
        }
    }

    #[test]
    fn finite_selected_geometry_handoff_keeps_remaps_after_both_scenes_end() {
        use crate::attribute::{AttributeDomain, AttributeSemantic};
        use crate::canonical_scene::store::construction::SceneCanonicalInvocationHost;
        use crate::canonical_scene::store::construction::node_scene::NodeSceneTestTransaction;
        use crate::canonical_scene::store::construction::node_scene::edit::transaction::SceneTriangleOrigins;
        use crate::canonical_scene::store::construction::node_scene::tests::{
            profile_fixture, single_trs_batch, successful_parts, test_limits,
        };
        use crate::canonical_scene::store::production::*;

        let factory = test_factory();
        let account = test_account(&factory, 1024);
        let calls = WorkCalls::default();
        let probe = Arc::new(ControlProbe::default());
        let triangles = [[0, 1, 2], [1, 3, 2]];
        let (prepared, input_target, dimensions, frozen) = {
            let positions = [
                0.0_f64, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0,
            ];
            let attributes = [GeometryAttributeAuthoring::f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
            )];
            let primitives = [GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleStrip,
                Presence::Absent,
                crate::MaterialAssignment::Unspecified,
                AuthoringRange::new(0, 1),
            )];
            let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
            let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
            let deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
            let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
            let deformation =
                DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &deformations, &[]);
            let spatial = SpatialAuthoringBatch::new(&crate::SceneSpace::STANDARD_TARGET, &[], &[]);
            let appearance = AppearanceAuthoringBatch::empty();
            let animation = AnimationAuthoringBatch::empty();
            let profile = profile_fixture(
                spatial,
                single_trs_batch(),
                appearance,
                geometry,
                deformation,
                animation,
            );
            let input = SceneAuthoringBatch::new(
                spatial,
                single_trs_batch(),
                appearance,
                geometry,
                deformation,
                animation,
                profile.batch(),
            );
            let mut source_host = SceneCanonicalInvocationHost::new();
            let source = successful_parts(
                NodeSceneTestTransaction::with_account(
                    &mut source_host,
                    &account,
                    test_limits(),
                    input,
                    SceneValidationLimits::default(),
                )
                .unwrap()
                .finish_draft()
                .unwrap()
                .publish_with_generation_source_for_test(SceneGenerationId::reserve)
                .unwrap(),
            );
            let scene = crate::ValidatedScene::from_node_scene(source.scene);
            let input_target = scene
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .id();
            let mut edit = SceneGeometryReplacement::new(input_target);
            edit.origins = Some(SceneTriangleOrigins {
                triangles: &triangles,
                face_offsets: &[0, 1, 2],
            });
            let edits = [edit];
            let mut host = Host::new();
            let (completion, dimensions, _) = candidate_completion(
                &mut host,
                &scene,
                probe.control(),
                Some(&edits),
                ProofFault::None,
                &calls,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("the finite selected transaction must restore its actual Core Candidate");
            };
            let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
                .try_issue_freeze_permit(completion.generation.interruption, final_progress())
                .unwrap()
            else {
                panic!("the actual finite selection must reach Ready");
            };
            let frozen = probe.counts();
            let prepared =
                consume_finite_scene_edit_freeze_permit(permit, completion.generation.rejection)
                    .unwrap();
            assert_eq!(probe.counts(), frozen);
            (prepared, input_target, dimensions, frozen)
        };
        let PreparedFiniteSceneEditPublication::Success(prepared) = prepared else {
            panic!("the finite selected transaction must prepare exact success");
        };
        let before_handoff = account.snapshot();
        let (scene, evidence, facts) = prepared.handoff().into_parts();
        assert_eq!(account.snapshot(), before_handoff);
        let SceneEditEvidence::Selection(evidence) = evidence else {
            panic!("the shared physical handoff must preserve selected-target evidence");
        };
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence.input_generation(), input_target.generation());
        assert_eq!(evidence.output_generation(), scene.generation());
        assert_eq!(evidence.geometry_reconstruction_count(), 1);
        assert_eq!(evidence.reconstructed_primitive_count(), 1);
        let target = evidence.target(0).unwrap();
        assert_eq!(target.input_target(), input_target);
        assert!(target.changed());
        assert!(target.lowered());
        assert_eq!(target.output_target().generation(), scene.generation());
        assert_eq!(target.corner_origins().unwrap(), triangles.as_flattened());
        assert_eq!(target.face_offsets().unwrap(), [0, 1, 2]);
        assert_eq!(
            scene
                .primitive(target.output_target())
                .unwrap()
                .indices()
                .unwrap(),
            triangles.as_flattened()
        );
        assert_eq!(facts.header().terminal_cause(), StoreTerminalCause::Success);
        assert_eq!(
            facts
                .dimension(dimensions[2])
                .unwrap()
                .committed_canonical_output(),
            u64::try_from(scene.canonical_block_count()).unwrap(),
        );
        assert_eq!(
            facts
                .dimension(dimensions[4])
                .unwrap()
                .published_outcome_evidence(),
            4
        );
        drop(scene);
        assert_eq!(
            evidence.target(0).unwrap().corner_origins().unwrap(),
            triangles.as_flattened()
        );
        assert_eq!(
            evidence.target(0).unwrap().face_offsets().unwrap(),
            [0, 1, 2]
        );
        drop(evidence);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert!(!account.snapshot().poisoned());
        assert_eq!(facts.header().terminal_cause(), StoreTerminalCause::Success);
        assert_eq!(probe.counts(), frozen);
        assert_eq!(calls.counts(), [1, 1, 1]);
    }

    #[test]
    fn finite_clean_start_rejection_keeps_original_domain_after_source_and_host_end() {
        let (error, dimensions, phases, foreign_dimension) = {
            let factory = test_factory();
            let account = test_account(&factory, 1024);
            let generations = AtomicU64::new(0);
            let source = try_build_single_trs_scene(&account, || {
                SceneGenerationId::reserve_from(&generations)
            })
            .unwrap();
            let scene = crate::ValidatedScene::from_node_scene(source.scene);
            let (ledger, _, dimensions, phases) = finite_scene_edit_plans(false);
            let (_, foreign_plan, foreign_dimensions, _) = finite_scene_edit_plans(false);
            let mut host = Host::new();
            let error = run_finite_scene_edit_body(
                &mut host,
                &scene,
                OperationControl::default(),
                ledger,
                foreign_plan,
                |_, _| panic!("a foreign startup plan must reject before the body"),
            )
            .err()
            .expect("the clean early rejection must retain its original owned finite error");
            assert!(!account.snapshot().poisoned());
            (error, dimensions, phases, foreign_dimensions[0])
        };
        assert!(matches!(error.cause(), FiniteStoreExitCause::Start(_)));
        assert!(error.has_residue());
        let domain = error.domain().unwrap();
        for dimension in dimensions {
            assert_eq!(
                domain.try_admit_dimension(dimension.semantic_id()),
                Some(dimension)
            );
        }
        for phase in phases {
            assert_eq!(domain.try_admit_phase(phase.semantic_id()), Some(phase));
        }
        assert_ne!(
            domain.try_admit_dimension(foreign_dimension.semantic_id()),
            Some(foreign_dimension)
        );
    }
}
