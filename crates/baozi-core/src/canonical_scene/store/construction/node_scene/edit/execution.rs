use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalActivationSlots,
    CanonicalDispositionSlot, CanonicalInvocationOutcome, CanonicalMaterialization,
    CanonicalOutputSlot, CanonicalProofSlot, CanonicalSemanticWorkError,
    CanonicalValidationProgress, CanonicalValidationSlots, RetainedStoreSessionOutcomeOwner,
};
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::__retained_store_integration::producer::{
    CanonicalActivationDescription, CanonicalActivationPlanSealer, CanonicalActivationRequest,
    CanonicalSealedActivationPlan, CanonicalSuccessRequirements, CanonicalTableShape,
};
use baozi_operation::{
    OperationControl, OperationOwner, ProgressPhase, ProgressUnit, RecordBlock,
    RetainedStoreBatchLimits, StorePlanError, StoreRuntimeError,
};
use thiserror::Error;

use crate::canonical_scene::store::schema::{NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema};
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::super::super::{
    SceneCanonicalInvocationHost, SceneConstructionError, SceneConstructionLimits,
    SceneConstructionPhase, SceneConstructionStartError, SceneConstructionUnit,
    SceneFailureEvidenceWork, SceneStoreOperation, final_progress, try_plan_scene_operation,
};
use super::super::materialization::{
    NodeSceneActivationOutput, NodeSceneActivationPostflight, NodeSceneEvidenceRecipe,
    NodeSceneFailureEvidencePlan, NodeSceneLaneOutput, NodeSceneLaneRejection,
    NodeSceneLaneValidationProof, node_scene_activation_roles,
};
use super::super::{NodeSceneConstructionRejection, PendingNodeSceneFailure};
use super::graph::try_rewrite_node_scene;
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditPlanError, PendingNodeSceneEdit};
use super::publication::{
    NodeSceneEditGenerationRejection, PreparedNodeSceneEdit, prepare_edit_candidate,
    prepare_pending_edit_failure,
};

type NodeSceneEditInvocationHost = SceneCanonicalInvocationHost<
    NodeSceneLaneOutput,
    NodeSceneLaneValidationProof,
    NodeSceneLaneRejection,
    NodeSceneLaneRejection,
    NodeSceneEditGenerationRejection,
>;

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeSceneEditPrepareError {
    #[error("node Scene edit could not bind its exact source")]
    Plan(#[from] NodeSceneEditPlanError),
    #[error("node Scene edit operation could not start")]
    Start(#[from] SceneConstructionStartError),
    #[error("node Scene edit construction failed closed")]
    Construction(#[from] SceneConstructionError),
}

struct NodeSceneEditActivationDescription<'scene> {
    activation_request: CanonicalActivationRequest,
    requirements: CanonicalSuccessRequirements,
    execution: BoundPendingNodeSceneEdit<'scene>,
}

impl<'scene> CanonicalActivationDescription for NodeSceneEditActivationDescription<'scene> {
    type Execution = BoundPendingNodeSceneEdit<'scene>;

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
        host: &mut NodeSceneEditInvocationHost,
        input: &super::super::SceneStorage,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ) -> Result<PreparedNodeSceneEdit, NodeSceneEditPrepareError> {
        self.prepare_with_control_and_generation_source(
            host,
            input,
            limits,
            control,
            SceneGenerationId::reserve,
        )
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn prepare_with_generation_source(
        self,
        host: &mut NodeSceneEditInvocationHost,
        input: &super::super::SceneStorage,
        limits: SceneConstructionLimits,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneEdit, NodeSceneEditPrepareError> {
        self.prepare_with_control_and_generation_source(
            host,
            input,
            limits,
            OperationControl::default(),
            reserve_generation,
        )
    }

    fn prepare_with_control_and_generation_source(
        self,
        host: &mut NodeSceneEditInvocationHost,
        input: &super::super::SceneStorage,
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
        let input_generation = pending.input.generation();
        let invalidations = pending.invalidations;
        let mut inputs = Some((start, pending, activation_request, failure_requirements));
        let mut evidence = None;
        let mut body_error = None;
        let finish = host.run_body(|parts, disposition| {
            let (start, pending, activation_request, failure_requirements) =
                inputs.take().expect("one edit body");
            let result = (|| {
                let operation = SceneStoreOperation::try_start_derived(
                    parts.startup,
                    start.control,
                    start.ledger_plan,
                    &pending.input.canonical,
                    start.start_plan,
                )
                .map_err(SceneConstructionStartError::from)?;
                let owner = operation.into_session_outcome_owner(
                    node_scene_activation_roles(start.keys).session_policy(),
                    outcome_evidence_writer_limits,
                    failure_requirements,
                )?;
                request_scoped_edit(
                    owner,
                    parts.output,
                    parts.proof,
                    parts.activation,
                    parts.validation,
                    disposition,
                    pending,
                    failure_evidence,
                    brand,
                    activation_request,
                    CanonicalValidationProgress::new(
                        SceneConstructionPhase::Validation,
                        SceneConstructionUnit::WorkUnits,
                    ),
                )
            })();
            match result {
                Ok(recipe) => evidence = Some(recipe),
                Err(cause) => body_error = Some(cause),
            }
        });
        let finish = match finish {
            Ok(finish) => finish,
            Err(cause) => {
                return Err(body_error.unwrap_or_else(|| cause.into()).into());
            }
        };
        let completion = finish.resume();
        let evidence = evidence.expect("an edit disposition retains its evidence recipe");
        let pending = match completion.outcome {
            CanonicalInvocationOutcome::Candidate(candidate) => {
                return prepare_edit_candidate(
                    candidate,
                    completion.generation,
                    completion.failure_fallback,
                    input_generation,
                    invalidations,
                    final_progress(),
                    reserve_generation,
                )
                .map_err(Into::into);
            }
            CanonicalInvocationOutcome::Rejected(owner) => {
                PendingNodeSceneFailure::ActivationRejected {
                    owner,
                    evidence_plan: evidence.record_plan(),
                    evidence_counts: evidence.counts(),
                    final_progress: final_progress(),
                }
            }
            CanonicalInvocationOutcome::NeverActivated(owner) => {
                PendingNodeSceneFailure::NeverActivated {
                    owner,
                    evidence_plan: evidence.record_plan(),
                    evidence_counts: evidence.counts(),
                    final_progress: final_progress(),
                }
            }
            CanonicalInvocationOutcome::CentralRejected(owner) => {
                PendingNodeSceneFailure::CentralRejected {
                    owner,
                    evidence_counts: evidence.counts(),
                    final_progress: final_progress(),
                }
            }
            CanonicalInvocationOutcome::Interrupted(owner) => {
                PendingNodeSceneFailure::CentralInterrupted {
                    owner,
                    evidence_counts: evidence.counts(),
                    final_progress: final_progress(),
                }
            }
            CanonicalInvocationOutcome::Fallback(owner) => {
                PendingNodeSceneFailure::DeferredFallback(owner)
            }
            CanonicalInvocationOutcome::Operation(_) => {
                unreachable!("an edit records a canonical disposition")
            }
        };
        prepare_pending_edit_failure(pending, completion.failure_fallback).map_err(Into::into)
    }

    #[cfg(test)]
    pub(super) fn prepare_with_control_and_generation_source_for_test(
        self,
        host: &mut NodeSceneEditInvocationHost,
        input: &super::super::SceneStorage,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneEdit, NodeSceneEditPrepareError> {
        self.prepare_with_control_and_generation_source(
            host,
            input,
            limits,
            control,
            reserve_generation,
        )
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the scoped stage borrows the pending edit and moves each independent publication capability once"
)]
fn request_scoped_edit<'inv, O, PP, U>(
    owner: RetainedStoreSessionOutcomeOwner<'inv, O, PP, U, CanonicalInheritanceSource>,
    output_slot: CanonicalOutputSlot<'inv, NodeSceneLaneOutput>,
    proof_slot: CanonicalProofSlot<'inv, NodeSceneLaneValidationProof>,
    activation_slots: CanonicalActivationSlots<'inv, NodeSceneLaneRejection>,
    validation_slots: CanonicalValidationSlots<'inv, NodeSceneLaneRejection>,
    disposition: CanonicalDispositionSlot<
        'inv,
        RecordBlock,
        SceneDirectorySchema,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
    pending: BoundPendingNodeSceneEdit<'_>,
    failure_evidence: NodeSceneFailureEvidencePlan,
    brand: super::super::super::ConstructionBrand,
    activation_request: CanonicalActivationRequest,
    validation_progress: CanonicalValidationProgress<PP, U>,
) -> Result<NodeSceneEvidenceRecipe, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let evidence_counts = pending.counts.evidence;
    let postflight =
        NodeSceneActivationPostflight::from_parts(brand, failure_evidence, evidence_counts);
    let mut success_requirements = Some(edit_success_requirements(&pending, brand)?);
    let mut pending = Some(pending);
    let mut activation_request = Some(activation_request);

    let outcome =
        owner.plan_and_activate_scoped(output_slot, activation_slots, move |mut planning| {
            let pending = match pending.take() {
                Some(pending) => pending,
                None => {
                    let rejection = NodeSceneLaneRejection::from(
                        NodeSceneConstructionRejection::Materialization(
                            StoreRuntimeError::ConstructionInvariantUnverified,
                        ),
                    );
                    let stable_code = rejection.stable_code();
                    return planning.reject(rejection, stable_code);
                }
            };
            let completion = planning.with_semantic_work(|semantic| {
                semantic
                    .run_unit(|| {
                        let super::plan::NodeSceneEditProgram::NodeTransform(rewrite) =
                            &pending.program
                        else {
                            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
                        };
                        let source = pending
                            .input
                            .semantic_authored_transform_at(
                                rewrite.target().source_index.as_usize(),
                            )
                            .map_err(|_| StoreRuntimeError::InvalidSourceSlot)?;
                        if source
                            .authored_transform()
                            .has_same_authored_bits(rewrite.replacement())
                            || *pending.operation.kind() != pending.program.operation_kind()
                        {
                            return Err(StoreRuntimeError::InvalidSourceSlot);
                        }
                        Ok(())
                    })
                    .and_then(|result| result)
            });
            let completion = match completion {
                Ok(((), completion)) => completion,
                Err(
                    CanonicalSemanticWorkError::Runtime(cause)
                    | CanonicalSemanticWorkError::Rejected(cause),
                ) => {
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
                execution: pending,
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
            rejected.request_finish(disposition)?;
            return Ok(postflight.failure_evidence());
        }
        CanonicalActivationOutcome::NeverActivated(never_activated) => {
            never_activated.request_finish(disposition)?;
            return Ok(postflight.failure_evidence());
        }
    };

    let candidate_evidence = postflight.candidate_evidence(activated.output());
    let outcome = activated.validate_centrally(
        proof_slot,
        validation_slots,
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
    outcome.request_finish(disposition)?;
    Ok(candidate_evidence)
}

fn edit_success_requirements(
    pending: &BoundPendingNodeSceneEdit<'_>,
    brand: super::super::super::ConstructionBrand,
) -> Result<CanonicalSuccessRequirements, StorePlanError> {
    let total = pending.counts.canonical_blocks.as_usize();
    CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(total, total)?,
        4,
        total,
        pending.counts.central_validation_plan.work(),
    )
    .map(|requirements| {
        requirements
            .with_request_identity(brand.activation_request_identity())
            .with_semantic_bytes(pending.input.read_nodes().names.len() as u64)
    })
}

fn materialize_edit<'lane, O, PP, U>(
    mut lane: baozi_operation::__retained_store_integration::activation::CanonicalLane<
        '_,
        'lane,
        O,
        PP,
        U,
        CanonicalInheritanceSource,
    >,
    pending: &BoundPendingNodeSceneEdit<'_>,
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
    let graph = match try_rewrite_node_scene(&mut lane, pending) {
        Ok(graph) => graph,
        Err(cause) => {
            let rejection = NodeSceneConstructionRejection::Materialization(cause);
            let stable_code = rejection.stable_code();
            return lane.reject(NodeSceneLaneRejection::from(rejection), stable_code);
        }
    };
    let output = NodeSceneActivationOutput {
        tables: graph.tables,
        brand,
        refs: graph.refs,
        evidence_counts: pending.counts.evidence,
        central_validation: pending.counts.central_validation_plan,
    };
    lane.complete(
        graph.directory.into_store(),
        NodeSceneLaneOutput::from_inner(output),
    )
}
