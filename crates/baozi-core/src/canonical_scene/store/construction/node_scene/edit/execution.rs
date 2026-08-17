use baozi_operation::__canonical_activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
    CanonicalMaterialization, CanonicalSuccessRequirements, CanonicalTableShape,
    CanonicalValidationDecision, CanonicalValidationProgress,
};
use baozi_operation::{
    CanonicalInheritanceSource, OperationControl, RecordBlock, RetainedStoreBatchLimits,
    StorePlanError, StoreRuntimeError,
};
use thiserror::Error;

use crate::canonical_scene::store::schema::{NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema};
use crate::canonical_scene::store::validation::validate_node_scene_canonical_graph_metered;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::super::super::{
    SceneConstructionError, SceneConstructionLimits, SceneConstructionPhase,
    SceneConstructionStartError, SceneConstructionUnit, SceneFailureEvidenceWork,
    SceneStoreOperation, final_progress, try_plan_scene_operation,
};
use super::super::materialization::{
    NodeSceneActivationOutput, NodeSceneFailureEvidencePlan, node_scene_activation_roles,
    write_node_scene_evidence_row,
};
use super::super::scratch::ScopedNodeSceneValidationScratchOwner;
use super::super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NodeSceneConstructionRejection, NodeSceneDraft,
    PendingNodeSceneFailure,
};
use super::graph::try_rewrite_node_scene;
use super::leaf_plan::NodeSceneInheritedLeafPlan;
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditPlanError, PendingNodeSceneEdit};
use super::publication::{
    PreparedNodeSceneEdit, prepare_edit_candidate, prepare_pending_edit_failure,
};

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeSceneEditPrepareError {
    #[error("node Scene edit could not bind its exact source")]
    Plan(#[from] NodeSceneEditPlanError),
    #[error("node Scene edit operation could not start")]
    Start(#[from] SceneConstructionStartError),
    #[error("node Scene edit construction failed closed")]
    Construction(#[from] SceneConstructionError),
}

struct NodeSceneEditExecutionPlan<'scope, 'scene> {
    pending: BoundPendingNodeSceneEdit<'scene>,
    inherited: NodeSceneInheritedLeafPlan<'scope, super::super::super::SceneConstructionOwner>,
}

impl PendingNodeSceneEdit {
    pub(in crate::canonical_scene::store) fn prepare(
        self,
        input: &super::super::ValidatedNodeScene,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ) -> Result<PreparedNodeSceneEdit, NodeSceneEditPrepareError> {
        self.prepare_with_control_and_generation_source(
            input,
            limits,
            control,
            SceneGenerationId::reserve,
        )
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn prepare_with_generation_source(
        self,
        input: &super::super::ValidatedNodeScene,
        limits: SceneConstructionLimits,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneEdit, NodeSceneEditPrepareError> {
        self.prepare_with_control_and_generation_source(
            input,
            limits,
            OperationControl::default(),
            reserve_generation,
        )
    }

    fn prepare_with_control_and_generation_source(
        self,
        input: &super::super::ValidatedNodeScene,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneEdit, NodeSceneEditPrepareError> {
        let pending = self.bind(input)?;
        let failure_evidence =
            NodeSceneFailureEvidencePlan::try_new().map_err(SceneConstructionStartError::from)?;
        let failure_requirements = failure_evidence
            .requirements()
            .map_err(SceneConstructionStartError::from)?;
        let canonical_writer_limits =
            RetainedStoreBatchLimits::new(pending.counts.canonical_attribution_slots.as_usize(), 4)
                .map_err(SceneConstructionStartError::from)?;
        let outcome_evidence_writer_limits = RetainedStoreBatchLimits::new(
            pending.counts.outcome_evidence_attribution_slots.as_usize(),
            4,
        )
        .map_err(SceneConstructionStartError::from)?;
        let terminal_attribution_slots = canonical_writer_limits
            .attribution_slots()
            .max(outcome_evidence_writer_limits.attribution_slots());
        let start = try_plan_scene_operation(
            limits,
            control,
            terminal_attribution_slots,
            SceneFailureEvidenceWork::exact_single_record(
                crate::canonical_scene::store::schema::NODE_SCENE_EVIDENCE_FIELD_COUNT,
            ),
        )?;
        let brand = super::super::super::ConstructionBrand::reserve()?;
        let raw = SceneStoreOperation::try_start_derived(
            start.control,
            start.ledger_plan,
            &pending.input.canonical,
            start.start_plan,
        )
        .map_err(SceneConstructionStartError::from)?;
        let policy = node_scene_activation_roles(start.keys)
            .bind(canonical_writer_limits, outcome_evidence_writer_limits);
        let owner = raw
            .into_scoped_session_outcome_owner(policy, failure_requirements)
            .map_err(SceneConstructionError::from)?;

        prepare_scoped_edit(owner, pending, failure_evidence, brand, reserve_generation)
            .map_err(Into::into)
    }

    #[cfg(test)]
    pub(super) fn prepare_with_control_and_generation_source_for_test(
        self,
        input: &super::super::ValidatedNodeScene,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneEdit, NodeSceneEditPrepareError> {
        self.prepare_with_control_and_generation_source(input, limits, control, reserve_generation)
    }
}

fn prepare_scoped_edit(
    owner: baozi_operation::__canonical_activation::ScopedRetainedStoreSessionOutcomeOwner<
        super::super::super::SceneConstructionOwner,
        SceneConstructionPhase,
        SceneConstructionUnit,
        CanonicalInheritanceSource,
    >,
    pending: BoundPendingNodeSceneEdit<'_>,
    failure_evidence: NodeSceneFailureEvidencePlan,
    brand: super::super::super::ConstructionBrand,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    let evidence_plan = failure_evidence.records;
    let evidence_counts = pending.counts.evidence;
    let input_generation = pending.input.generation();
    let invalidations = pending.invalidations;
    let success_requirements = edit_success_requirements(&pending)?;
    let mut pending = Some(pending);

    let outcome = owner.plan_and_activate_scoped(move |mut planning| {
        let pending = match pending.take() {
            Some(pending) => pending,
            None => {
                let rejection = NodeSceneConstructionRejection::Materialization(
                    StoreRuntimeError::ConstructionInvariantUnverified,
                );
                let stable_code = rejection.stable_code();
                return planning.reject(rejection, stable_code);
            }
        };
        let planned = match planning.begin_semantic_work() {
            Ok(mut semantic) => {
                match NodeSceneInheritedLeafPlan::try_collect(&pending, &mut semantic) {
                    Ok(inherited) => match semantic.finish_exact() {
                        Ok(completion) => Ok((inherited, completion)),
                        Err(cause) => {
                            drop(inherited);
                            Err(cause)
                        }
                    },
                    Err(cause) => {
                        let _ = semantic.finish_rejected();
                        Err(cause)
                    }
                }
            }
            Err(cause) => Err(cause),
        };
        let (inherited, completion) = match planned {
            Ok(planned) => planned,
            Err(cause) => {
                let rejection = NodeSceneConstructionRejection::Materialization(cause);
                let stable_code = rejection.stable_code();
                return planning.reject(rejection, stable_code);
            }
        };
        let execution = NodeSceneEditExecutionPlan { pending, inherited };
        let permit = match planning.seal_activation(success_requirements, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit,
            CanonicalActivationSeal::Unsealed(failure) => {
                drop(execution);
                return failure.finish_after_plan_drop();
            }
        };
        permit.activate(execution, move |lane, execution| {
            materialize_edit(lane, execution, brand)
        })
    })?;

    let validation_progress = CanonicalValidationProgress::new(
        SceneConstructionPhase::Validation,
        SceneConstructionUnit::WorkUnits,
    );
    let final_progress = final_progress();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(rejected) => {
            return prepare_pending_edit_failure(PendingNodeSceneFailure::ActivationRejected {
                owner: rejected,
                evidence_plan,
                evidence_counts,
                final_progress,
            });
        }
        CanonicalActivationOutcome::NeverActivated(never_activated) => {
            return prepare_pending_edit_failure(PendingNodeSceneFailure::NeverActivated {
                owner: never_activated,
                evidence_plan,
                evidence_counts,
                final_progress,
            });
        }
        CanonicalActivationOutcome::Fallback(fallback) => {
            return prepare_pending_edit_failure(PendingNodeSceneFailure::PreparedFallback(
                fallback,
            ));
        }
    };

    let central_counts = activated.output().evidence_counts;
    let outcome = activated.validate_centrally(
        validation_progress,
        |lane, output| match lane.try_retain_records(
            evidence_plan,
            NodeSceneOutcomeEvidenceSchema,
            |control| {
                control.write_row(|row| write_node_scene_evidence_row(row, output.evidence_counts))
            },
        ) {
            Ok(evidence) => lane.complete(evidence),
            Err(_) => lane.abort(),
        },
        |planning, output| {
            let validation_plan = output.central_validation;
            let scratch_plan = validation_plan.scratch_plan();
            planning.validate_with(
                |checkpoint| {
                    ScopedNodeSceneValidationScratchOwner::try_allocate(checkpoint, scratch_plan)
                },
                |session, directory, evidence, scratch| {
                    let initialized = scratch.initialize(session, scratch_plan)?;
                    let proof = if initialized && output.draft.brand == brand {
                        let mut scratch = scratch.metered_view();
                        validate_node_scene_canonical_graph_metered(
                            session,
                            directory,
                            evidence,
                            output.evidence_counts,
                            output.refs,
                            validation_plan,
                            &mut scratch,
                        )?
                    } else {
                        None
                    };
                    Ok(match proof {
                        Some(proof) => CanonicalValidationDecision::accept(proof),
                        None => CanonicalValidationDecision::reject(
                            NodeSceneConstructionRejection::<StoreRuntimeError>::Materialization(
                                StoreRuntimeError::ConstructionInvariantUnverified,
                            ),
                            NODE_SCENE_BUILD_INVARIANT_CODE,
                        ),
                    })
                },
            )
        },
    )?;
    match outcome {
        CanonicalCentralValidationOutcome::Candidate(candidate) => prepare_edit_candidate(
            candidate,
            input_generation,
            invalidations,
            final_progress,
            reserve_generation,
        ),
        CanonicalCentralValidationOutcome::Rejected(rejected) => {
            prepare_pending_edit_failure(PendingNodeSceneFailure::CentralRejected {
                owner: rejected,
                evidence_counts: central_counts,
                final_progress,
            })
        }
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => {
            prepare_pending_edit_failure(PendingNodeSceneFailure::CentralInterrupted {
                owner: interrupted,
                evidence_counts: central_counts,
                final_progress,
            })
        }
        CanonicalCentralValidationOutcome::Fallback(fallback) => {
            prepare_pending_edit_failure(PendingNodeSceneFailure::PreparedFallback(fallback))
        }
    }
}

fn edit_success_requirements(
    pending: &BoundPendingNodeSceneEdit<'_>,
) -> Result<CanonicalSuccessRequirements, StorePlanError> {
    let total = pending.counts.canonical_blocks.as_usize();
    CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(total, pending.counts.new_canonical_blocks.as_usize())?,
        total,
        pending.counts.central_validation_plan.work(),
    )
}

fn materialize_edit<'lane, 'scope>(
    mut lane: baozi_operation::__canonical_activation::CanonicalLane<
        'lane,
        super::super::super::SceneConstructionOwner,
        SceneConstructionPhase,
        SceneConstructionUnit,
        CanonicalInheritanceSource,
    >,
    execution: &NodeSceneEditExecutionPlan<'scope, '_>,
    brand: super::super::super::ConstructionBrand,
) -> CanonicalMaterialization<
    'lane,
    NodeSceneActivationOutput,
    NodeSceneConstructionRejection<StoreRuntimeError>,
    RecordBlock,
    SceneDirectorySchema,
> {
    let graph = match try_rewrite_node_scene(&mut lane, &execution.pending, &execution.inherited) {
        Ok(graph) => graph,
        Err(cause) => {
            let rejection = NodeSceneConstructionRejection::Materialization(cause);
            let stable_code = rejection.stable_code();
            return lane.reject(rejection, stable_code);
        }
    };
    let output = NodeSceneActivationOutput {
        draft: NodeSceneDraft {
            brand,
            directory: graph.directory,
        },
        refs: graph.refs,
        evidence_counts: execution.pending.counts.evidence,
        central_validation: execution.pending.counts.central_validation_plan,
    };
    lane.complete(graph.directory.into_store(), output)
}
