use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
    CanonicalMaterialization, CanonicalValidationProgress, RetainedStoreSessionOutcomeOwner,
};
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::__retained_store_integration::producer::{
    CanonicalActivationDescription, CanonicalActivationPlanSealer, CanonicalActivationRequest,
    CanonicalSealedActivationPlan, CanonicalSuccessRequirements, CanonicalTableShape,
};
use baozi_operation::{
    FinalProgress, OperationControl, OperationOwner, ProgressPhase, ProgressUnit, RecordBlock,
    RetainedStoreBatchLimits, StorePlanError, StoreRuntimeError,
};
use thiserror::Error;

use crate::canonical_scene::store::schema::{NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema};
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::super::super::{
    SceneConstructionError, SceneConstructionLimits, SceneConstructionPhase,
    SceneConstructionStartError, SceneConstructionUnit, SceneFailureEvidenceWork,
    SceneStoreOperation, final_progress, try_plan_scene_operation,
};
use super::super::materialization::{
    NodeSceneActivationOutput, NodeSceneActivationPostflight, NodeSceneFailureEvidencePlan,
    NodeSceneLaneOutput, NodeSceneLaneRejection, node_scene_activation_roles,
};
use super::super::{NodeSceneConstructionRejection, NodeSceneDraft, PendingNodeSceneFailure};
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

struct NodeSceneEditExecutionPlan<'scope, 'scene, O>
where
    O: OperationOwner,
{
    pending: BoundPendingNodeSceneEdit<'scene>,
    inherited: NodeSceneInheritedLeafPlan<'scope, O>,
}

struct NodeSceneEditActivationDescription<'scope, 'scene, O>
where
    O: OperationOwner,
{
    activation_request: CanonicalActivationRequest,
    requirements: CanonicalSuccessRequirements,
    execution: NodeSceneEditExecutionPlan<'scope, 'scene, O>,
}

impl<'scope, 'scene, O> CanonicalActivationDescription
    for NodeSceneEditActivationDescription<'scope, 'scene, O>
where
    O: OperationOwner,
{
    type Execution = NodeSceneEditExecutionPlan<'scope, 'scene, O>;

    fn seal_activation<'activation>(
        self,
        sealer: CanonicalActivationPlanSealer<'activation>,
    ) -> CanonicalSealedActivationPlan<'activation, Self::Execution> {
        sealer.seal(self.activation_request, self.requirements, self.execution)
    }
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
        let (brand, activation_request) =
            super::super::super::ConstructionBrand::reserve_with_activation_request()?;
        let failure_requirements = failure_evidence
            .requirements_for(brand)
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
        let raw = SceneStoreOperation::try_start_derived(
            start.control,
            start.ledger_plan,
            &pending.input.canonical,
            start.start_plan,
        )
        .map_err(SceneConstructionStartError::from)?;
        let owner = raw
            .into_session_outcome_owner(
                node_scene_activation_roles(start.keys).session_policy(),
                outcome_evidence_writer_limits,
                failure_requirements,
            )
            .map_err(SceneConstructionError::from)?;

        prepare_scoped_edit(
            owner,
            pending,
            failure_evidence,
            brand,
            activation_request,
            CanonicalValidationProgress::new(
                SceneConstructionPhase::Validation,
                SceneConstructionUnit::WorkUnits,
            ),
            final_progress(),
            reserve_generation,
        )
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

#[allow(
    clippy::too_many_arguments,
    reason = "the scoped stage borrows the pending edit and moves each independent publication capability once"
)]
fn prepare_scoped_edit<O, PP, U>(
    owner: RetainedStoreSessionOutcomeOwner<O, PP, U, CanonicalInheritanceSource>,
    pending: BoundPendingNodeSceneEdit<'_>,
    failure_evidence: NodeSceneFailureEvidencePlan,
    brand: super::super::super::ConstructionBrand,
    activation_request: CanonicalActivationRequest,
    validation_progress: CanonicalValidationProgress<PP, U>,
    final_progress: FinalProgress<PP, U>,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let evidence_plan = failure_evidence.records;
    let evidence_counts = pending.counts.evidence;
    let postflight =
        NodeSceneActivationPostflight::from_parts(brand, failure_evidence, evidence_counts);
    let input_generation = pending.input.generation();
    let invalidations = pending.invalidations;
    let mut success_requirements = Some(edit_success_requirements(&pending, brand)?);
    let mut pending = Some(pending);
    let mut activation_request = Some(activation_request);

    let outcome = owner.plan_and_activate_scoped(move |mut planning| {
        let pending = match pending.take() {
            Some(pending) => pending,
            None => {
                let rejection =
                    NodeSceneLaneRejection::from(NodeSceneConstructionRejection::Materialization(
                        StoreRuntimeError::ConstructionInvariantUnverified,
                    ));
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
                let rejection = NodeSceneLaneRejection::from(
                    NodeSceneConstructionRejection::Materialization(cause),
                );
                let stable_code = rejection.stable_code();
                return planning.reject(rejection, stable_code);
            }
        };
        let description = NodeSceneEditActivationDescription {
            activation_request: activation_request
                .take()
                .expect("the edit activation request is consumed exactly once"),
            requirements: success_requirements
                .take()
                .expect("the edit success requirements are consumed exactly once"),
            execution: NodeSceneEditExecutionPlan { pending, inherited },
        };
        let permit = match planning.seal_activation(description, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit,
            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
        };
        permit.activate(move |lane, execution| materialize_edit(lane, execution, brand))
    })?;

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
            return prepare_pending_edit_failure::<O, PP, U>(
                PendingNodeSceneFailure::PreparedFallback(fallback),
            );
        }
    };

    let candidate_evidence = postflight.candidate_evidence(activated.output());
    let central_counts = candidate_evidence.counts();
    let outcome = activated.validate_centrally(
        validation_progress,
        |lane, _output| match lane.try_retain_records(
            candidate_evidence.record_plan(),
            NodeSceneOutcomeEvidenceSchema,
            |control| control.write_row(|row| candidate_evidence.write_row(row)),
        ) {
            Ok(evidence) => lane.complete(evidence),
            Err(_) => lane.abort(),
        },
        |planning, output| postflight.validate_candidate(planning, output),
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
            prepare_pending_edit_failure::<O, PP, U>(PendingNodeSceneFailure::PreparedFallback(
                fallback,
            ))
        }
    }
}

fn edit_success_requirements(
    pending: &BoundPendingNodeSceneEdit<'_>,
    brand: super::super::super::ConstructionBrand,
) -> Result<CanonicalSuccessRequirements, StorePlanError> {
    let total = pending.counts.canonical_blocks.as_usize();
    CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(total, pending.counts.new_canonical_blocks.as_usize())?,
        4,
        total,
        pending.counts.central_validation_plan.work(),
    )
    .map(|requirements| requirements.with_request_identity(brand.activation_request_identity()))
}

fn materialize_edit<'lane, 'scope, O, PP, U>(
    mut lane: baozi_operation::__retained_store_integration::activation::CanonicalLane<
        'lane,
        O,
        PP,
        U,
        CanonicalInheritanceSource,
    >,
    execution: &NodeSceneEditExecutionPlan<'scope, '_, O>,
    brand: super::super::super::ConstructionBrand,
) -> CanonicalMaterialization<
    'lane,
    NodeSceneLaneOutput,
    NodeSceneLaneRejection,
    RecordBlock,
    SceneDirectorySchema,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let graph = match try_rewrite_node_scene(&mut lane, &execution.pending, &execution.inherited) {
        Ok(graph) => graph,
        Err(cause) => {
            let rejection = NodeSceneConstructionRejection::Materialization(cause);
            let stable_code = rejection.stable_code();
            return lane.reject(NodeSceneLaneRejection::from(rejection), stable_code);
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
    lane.complete(
        graph.directory.into_store(),
        NodeSceneLaneOutput::from_inner(output),
    )
}
