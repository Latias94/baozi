use baozi_operation::__canonical_activation::{
    CanonicalFreezePermitIssue, CanonicalJointSeal, FailureEvidenceMember, FailureEvidenceOutcome,
    FailureEvidenceValidationSession, RootedFailureValidation, StoreActivationContainment,
    StoreCanonicalRejection, StoreNeverActivatedCause,
};
use baozi_operation::{
    ControlCause, CounterClass, FinalProgress, LedgerCause, OperationOwner,
    PreparedStoreFailureOutcome, PreparedStoreFallback, PreparedStoreSuccessOutcome, ProgressPhase,
    ProgressUnit, RecordBlock, StoreBuildFailureClass, StoreRuntimeError,
};

use crate::canonical_scene::store::schema::{
    NODE_SCENE_EVIDENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    NodeSceneOutcomeEvidenceSchema, SCENE_SCHEMA_REVISION, node_scene_evidence_field,
};
#[cfg(test)]
use crate::identity::SceneGenerationId;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError};

use super::super::{
    GENERATION_EXHAUSTED_CODE, PreparedTracerSceneFallback, SceneConstructionError,
    TracerSceneFallback, burn_generation_reservation,
};
use super::materialization::{NodeSceneActivationOutput, write_node_scene_evidence_row};
use super::{
    DeferredNodeSceneDraftBuild, NODE_SCENE_BUILD_INVARIANT_CODE,
    NODE_SCENE_CONTROL_INTERRUPTION_CODE, NODE_SCENE_STORAGE_REJECTION_CODE,
    NODE_SCENE_WORK_INTERRUPTION_CODE, NodeSceneCanonicalJointCommit,
    NodeSceneConstructionRejection, NodeSceneConstructionTerminal, NodeSceneDraft,
    NodeSceneDraftBuild, NodeSceneRichFailure, NodeSceneSuccessParts, NodeSceneSuccessReport,
    PendingNodeSceneDraft, PendingNodeSceneFailure, PreparedNodeSceneConstruction,
    PreparedNodeSceneFailure, PreparedNodeSceneRichFailure, PreparedNodeSceneSuccess,
    SceneEvidenceCounts, ValidatedNodeScene,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeSceneGenerationRejection {
    MissingDraft,
    DraftMismatch,
}

macro_rules! prepare_frozen_node_failure {
    ($rooted:expr, $counts:expr) => {{
        let validation = $rooted.validate_rich_failure(|session, root| {
            validate_node_scene_failure_evidence(session, root, $counts)
        })?;
        match validation {
            RootedFailureValidation::Valid(validated) => {
                let (prepared, _cause) = validated.try_prepare_frozen()?.into_parts();
                Ok(PreparedNodeSceneConstruction::from(
                    map_node_prepared_failure(prepared),
                ))
            }
            RootedFailureValidation::Rejected(rejected) => {
                let (store, _cause) = rejected.prepare_minimal_fallback()?.into_parts();
                Ok(PreparedNodeSceneConstruction::MinimalFallback(
                    PreparedTracerSceneFallback { store },
                ))
            }
            RootedFailureValidation::Fallback(fallback) => {
                let (store, _cause) = fallback.into_parts();
                Ok(PreparedNodeSceneConstruction::MinimalFallback(
                    PreparedTracerSceneFallback { store },
                ))
            }
        }
    }};
}

/// Core's complete success contribution while the operation joint owner remains scoped.
///
/// Rust drops fields in declaration order. The generation, proof, descriptor, and seal therefore
/// disappear before the operation-owned joint contribution observes any unwind.
#[must_use = "the scoped node Scene joint commit must be prepared or deliberately dropped"]
struct ScopedNodeSceneJointCommit<'joint, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    generation: ReservedSceneGeneration,
    proof: super::NodeSceneValidationProof,
    output: NodeSceneActivationOutput,
    seal: CanonicalJointSeal<'joint>,
    joint: NodeSceneCanonicalJointCommit<'joint, O, PP, U>,
}

impl<O, PP, U> ScopedNodeSceneJointCommit<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn prepare(self) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        let Self {
            generation,
            proof,
            output: _,
            seal,
            joint,
        } = self;
        match joint.prepare(seal)? {
            PreparedStoreSuccessOutcome::Success(store) => Ok(
                PreparedNodeSceneConstruction::Success(PreparedNodeSceneSuccess {
                    generation,
                    proof,
                    store,
                }),
            ),
            PreparedStoreSuccessOutcome::RichFailure(store) => {
                burn_generation_reservation(generation);
                Ok(PreparedNodeSceneConstruction::RichFailure(
                    PreparedNodeSceneRichFailure { store },
                ))
            }
            PreparedStoreSuccessOutcome::MinimalFallback(store) => {
                burn_generation_reservation(generation);
                Ok(PreparedNodeSceneConstruction::MinimalFallback(
                    PreparedTracerSceneFallback { store },
                ))
            }
        }
    }
}

impl<O, PP, U> PendingNodeSceneDraft<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) fn prepare_with_generation_source(
        self,
        draft: NodeSceneDraft,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        prepare_pending_node_scene_draft(self, Some(draft), Some(reserve_generation))
            .map(|(prepared, _)| prepared)
    }

    pub(in crate::canonical_scene::store) fn prepare_import_with_generation_source(
        self,
        draft: NodeSceneDraft,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<(PreparedNodeSceneConstruction, Option<ControlCause>), SceneConstructionError> {
        prepare_pending_node_scene_draft(self, Some(draft), Some(reserve_generation))
    }

    pub(in crate::canonical_scene::store) fn prepare_missing_draft(
        self,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        prepare_pending_node_scene_draft(
            self,
            None,
            None::<fn() -> Result<ReservedSceneGeneration, SceneGenerationError>>,
        )
        .map(|(prepared, _)| prepared)
    }

    pub(in crate::canonical_scene::store) fn prepare_import_missing_draft(
        self,
    ) -> Result<(PreparedNodeSceneConstruction, Option<ControlCause>), SceneConstructionError> {
        prepare_pending_node_scene_draft(
            self,
            None,
            None::<fn() -> Result<ReservedSceneGeneration, SceneGenerationError>>,
        )
    }
}

#[cfg(test)]
impl NodeSceneDraftBuild {
    pub(super) fn prepare_for_test(
        self,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        self.prepare_with_generation_source_for_test(SceneGenerationId::reserve)
    }

    pub(super) fn prepare_with_generation_source_for_test(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        match self {
            Self::Draft { draft, owner } => {
                owner.prepare_with_generation_source(draft, reserve_generation)
            }
            Self::Failure { prepared, .. } => Ok(prepared.into()),
        }
    }
}

fn prepare_pending_node_scene_draft<O, PP, U, ReserveGeneration>(
    owner: PendingNodeSceneDraft<O, PP, U>,
    parser_draft: Option<NodeSceneDraft>,
    reserve_generation: Option<ReserveGeneration>,
) -> Result<(PreparedNodeSceneConstruction, Option<ControlCause>), SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    ReserveGeneration: FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
{
    let PendingNodeSceneDraft {
        candidate,
        final_progress,
    } = owner;
    let output = *candidate.output();
    let evidence_counts = output.evidence_counts;
    match candidate.try_issue_freeze_permit(final_progress)? {
        CanonicalFreezePermitIssue::Ready(permit) => {
            let pairing_rejection = match parser_draft {
                None => Some(NodeSceneGenerationRejection::MissingDraft),
                Some(draft) if !node_scene_draft_matches(draft, output.draft) => {
                    Some(NodeSceneGenerationRejection::DraftMismatch)
                }
                Some(_) => None,
            };
            if let Some(cause) = pairing_rejection {
                let rejected = permit.reject_generation(cause, NODE_SCENE_BUILD_INVARIANT_CODE)?;
                return prepare_frozen_node_failure!(rejected, evidence_counts)
                    .map(|prepared| (prepared, None));
            }

            let reserve_generation = reserve_generation
                .expect("a matching parser draft retains the generation admission callback");
            let generation = match reserve_generation() {
                Ok(generation) => generation,
                Err(SceneGenerationError::Exhausted) => {
                    let rejected = permit.reject_generation(
                        SceneGenerationError::Exhausted,
                        GENERATION_EXHAUSTED_CODE,
                    )?;
                    return prepare_frozen_node_failure!(rejected, evidence_counts)
                        .map(|prepared| (prepared, None));
                }
            };
            permit
                .bind_joint_commit(|output, proof, joint, seal| {
                    ScopedNodeSceneJointCommit {
                        generation,
                        proof,
                        output,
                        seal,
                        joint,
                    }
                    .prepare()
                })
                .map(|prepared| (prepared, None))
        }
        CanonicalFreezePermitIssue::Interrupted(interrupted) => {
            let control_cause = interrupted.cause().control_cause().cloned();
            prepare_frozen_node_failure!(interrupted, evidence_counts)
                .map(|prepared| (prepared, control_cause))
        }
        CanonicalFreezePermitIssue::DestinationRejected(rejected) => {
            let validation = rejected.validate_rich_failure(|session, root| {
                validate_node_scene_failure_evidence(session, root, evidence_counts)
            })?;
            prepare_node_ordinary_construction_failure(validation, final_progress)
                .map(|prepared| (prepared, None))
        }
    }
}

fn node_scene_draft_matches(parser: NodeSceneDraft, activated: NodeSceneDraft) -> bool {
    parser.brand == activated.brand && parser.directory == activated.directory
}

pub(super) trait NodeActivationFailureCause {
    fn is_authoring_rejection(&self) -> bool;

    fn control_cause(&self) -> Option<&ControlCause>;

    fn into_authoring_rejection(self) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>>;
}

impl NodeActivationFailureCause
    for StoreNeverActivatedCause<NodeSceneConstructionRejection<StoreRuntimeError>>
{
    fn is_authoring_rejection(&self) -> bool {
        matches!(
            self,
            StoreNeverActivatedCause::Requested(rejection)
                if !matches!(rejection, NodeSceneConstructionRejection::Materialization(_))
        )
    }

    fn control_cause(&self) -> Option<&ControlCause> {
        match self {
            StoreNeverActivatedCause::Runtime(StoreRuntimeError::Control(cause)) => Some(cause),
            StoreNeverActivatedCause::Requested(_) | StoreNeverActivatedCause::Runtime(_) => None,
        }
    }

    fn into_authoring_rejection(self) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>> {
        match self {
            StoreNeverActivatedCause::Requested(rejection) => {
                retain_node_authoring_rejection(rejection)
            }
            StoreNeverActivatedCause::Runtime(_) => None,
        }
    }
}

impl NodeActivationFailureCause
    for StoreCanonicalRejection<NodeSceneConstructionRejection<StoreRuntimeError>>
{
    fn is_authoring_rejection(&self) -> bool {
        matches!(
            self,
            StoreCanonicalRejection::Requested(rejection)
                if !matches!(rejection, NodeSceneConstructionRejection::Materialization(_))
        )
    }

    fn control_cause(&self) -> Option<&ControlCause> {
        match self {
            StoreCanonicalRejection::Runtime(StoreRuntimeError::Control(cause)) => Some(cause),
            StoreCanonicalRejection::Requested(_) | StoreCanonicalRejection::Runtime(_) => None,
        }
    }

    fn into_authoring_rejection(self) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>> {
        match self {
            StoreCanonicalRejection::Requested(rejection) => {
                retain_node_authoring_rejection(rejection)
            }
            StoreCanonicalRejection::Runtime(_) => None,
        }
    }
}

impl<O, PP, U, L> PendingNodeSceneFailure<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) fn prepare(
        self,
    ) -> Result<
        (
            PreparedNodeSceneConstruction,
            Option<NodeSceneConstructionRejection<StoreRuntimeError>>,
            Option<ControlCause>,
        ),
        SceneConstructionError,
    > {
        let mut control_cause = None;
        let build = match self {
            Self::ActivationRejected {
                owner,
                evidence_plan,
                evidence_counts,
                final_progress,
            } => {
                control_cause = owner.cause().control_cause().cloned();
                prepare_node_activation_failure(
                    owner.build_failure_evidence(|lane| {
                        match lane.try_retain_records(
                            evidence_plan,
                            NodeSceneOutcomeEvidenceSchema,
                            |writer| {
                                writer.write_row(|row| {
                                    write_node_scene_evidence_row(row, evidence_counts)
                                })
                            },
                        ) {
                            Ok(evidence) => lane.complete(evidence),
                            Err(_) => lane.abort(),
                        }
                    }),
                    evidence_counts,
                    final_progress,
                )?
            }
            Self::NeverActivated {
                owner,
                evidence_plan,
                evidence_counts,
                final_progress,
            } => {
                control_cause = owner.cause().control_cause().cloned();
                prepare_node_activation_failure(
                    owner.build_failure_evidence(|lane| {
                        match lane.try_retain_records(
                            evidence_plan,
                            NodeSceneOutcomeEvidenceSchema,
                            |writer| {
                                writer.write_row(|row| {
                                    write_node_scene_evidence_row(row, evidence_counts)
                                })
                            },
                        ) {
                            Ok(evidence) => lane.complete(evidence),
                            Err(_) => lane.abort(),
                        }
                    }),
                    evidence_counts,
                    final_progress,
                )?
            }
            Self::CentralRejected {
                owner,
                evidence_counts,
                final_progress,
            } => prepare_node_central_failure(
                owner.validate_rich_failure(|session, root| {
                    validate_node_scene_failure_evidence(session, root, evidence_counts)
                }),
                final_progress,
            )?,
            Self::CentralInterrupted {
                owner,
                evidence_counts,
                final_progress,
            } => {
                control_cause = owner.cause().control_cause().cloned();
                prepare_node_central_failure(
                    owner.validate_rich_failure(|session, root| {
                        validate_node_scene_failure_evidence(session, root, evidence_counts)
                    }),
                    final_progress,
                )?
            }
            Self::PreparedFallback(fallback) => {
                let (store, _cause) = fallback.into_parts();
                return Ok((
                    PreparedNodeSceneConstruction::MinimalFallback(PreparedTracerSceneFallback {
                        store,
                    }),
                    None,
                    None,
                ));
            }
        };
        let NodeSceneDraftBuild::Failure {
            authoring_rejection,
            prepared,
        } = build
        else {
            unreachable!("a pending failure cannot prepare a successful Scene draft")
        };
        Ok((prepared.into(), authoring_rejection, control_cause))
    }
}

impl<O, PP, U, L> DeferredNodeSceneDraftBuild<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) fn prepare_immediately(
        self,
    ) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError> {
        match self {
            Self::Draft { draft, owner } => Ok(NodeSceneDraftBuild::Draft { draft, owner }),
            Self::Failure { owner, .. } => {
                let (prepared, authoring_rejection, _control_cause) = owner.prepare()?;
                let prepared = match prepared {
                    PreparedNodeSceneConstruction::RichFailure(failure) => {
                        PreparedNodeSceneFailure::RichFailure(failure)
                    }
                    PreparedNodeSceneConstruction::MinimalFallback(fallback) => {
                        PreparedNodeSceneFailure::MinimalFallback(fallback)
                    }
                    PreparedNodeSceneConstruction::Success(_) => {
                        unreachable!("a pending failure cannot prepare canonical success")
                    }
                };
                Ok(NodeSceneDraftBuild::Failure {
                    authoring_rejection,
                    prepared,
                })
            }
        }
    }
}

fn retain_node_authoring_rejection(
    rejection: NodeSceneConstructionRejection<StoreRuntimeError>,
) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>> {
    match rejection {
        NodeSceneConstructionRejection::Materialization(_) => None,
        rejection => Some(rejection),
    }
}

/// Prepares a rejected or never-activated scoped outcome after its one-shot evidence lane closes.
pub(super) fn prepare_node_activation_failure<O, PP, U, Cause>(
    outcome: Result<
        FailureEvidenceOutcome<O, PP, U, Cause, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
        StoreActivationContainment,
    >,
    evidence_counts: SceneEvidenceCounts,
    final_progress: FinalProgress<PP, U>,
) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Cause: NodeActivationFailureCause,
{
    let (prepared, cause) = match outcome? {
        FailureEvidenceOutcome::Rooted(rooted) => {
            let validation = rooted.validate_rich_failure(|session, root| {
                validate_node_scene_failure_evidence(session, root, evidence_counts)
            })?;
            prepare_node_ordinary_failure_validation(validation, final_progress)?
        }
        FailureEvidenceOutcome::MinimalFallback(fallback) => {
            let (store, cause) = fallback.into_parts();
            (
                PreparedNodeSceneFailure::MinimalFallback(PreparedTracerSceneFallback { store }),
                Some(cause),
            )
        }
    };
    Ok(NodeSceneDraftBuild::Failure {
        authoring_rejection: cause.and_then(NodeActivationFailureCause::into_authoring_rejection),
        prepared,
    })
}

/// Prepares an ordinary central-validation failure. The caller retains the concrete rejected or
/// interrupted owner long enough to invoke its rich-evidence validator, then passes the typed
/// validation result here without recovering any writable store state.
pub(super) fn prepare_node_central_failure<O, PP, U, Cause>(
    validation: Result<
        RootedFailureValidation<O, PP, U, Cause, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
        StoreActivationContainment,
    >,
    final_progress: FinalProgress<PP, U>,
) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (prepared, _cause) = prepare_node_ordinary_failure_validation(validation?, final_progress)?;
    Ok(NodeSceneDraftBuild::Failure {
        authoring_rejection: None,
        prepared,
    })
}

pub(super) fn prepare_node_central_fallback<O, PP, U>(
    store: PreparedStoreFallback,
) -> NodeSceneDraftBuild<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    NodeSceneDraftBuild::Failure {
        authoring_rejection: None,
        prepared: PreparedNodeSceneFailure::MinimalFallback(PreparedTracerSceneFallback { store }),
    }
}

fn prepare_node_ordinary_construction_failure<O, PP, U, Cause>(
    validation: RootedFailureValidation<
        O,
        PP,
        U,
        Cause,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
    final_progress: FinalProgress<PP, U>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (prepared, _cause) = prepare_node_ordinary_failure_validation(validation, final_progress)?;
    Ok(prepared.into())
}

fn prepare_node_ordinary_failure_validation<O, PP, U, Cause>(
    validation: RootedFailureValidation<
        O,
        PP,
        U,
        Cause,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
    final_progress: FinalProgress<PP, U>,
) -> Result<(PreparedNodeSceneFailure, Option<Cause>), SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match validation {
        RootedFailureValidation::Valid(validated) => {
            let (prepared, cause) = validated.try_prepare(final_progress)?.into_parts();
            Ok((map_node_prepared_failure(prepared), Some(cause)))
        }
        RootedFailureValidation::Rejected(rejected) => {
            let (store, cause) = rejected.prepare_minimal_fallback()?.into_parts();
            Ok((
                PreparedNodeSceneFailure::MinimalFallback(PreparedTracerSceneFallback { store }),
                Some(cause),
            ))
        }
        RootedFailureValidation::Fallback(fallback) => {
            let (store, _cause) = fallback.into_parts();
            Ok((
                PreparedNodeSceneFailure::MinimalFallback(PreparedTracerSceneFallback { store }),
                None,
            ))
        }
    }
}

fn map_node_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
) -> PreparedNodeSceneFailure {
    match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedNodeSceneFailure::RichFailure(PreparedNodeSceneRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedNodeSceneFailure::MinimalFallback(PreparedTracerSceneFallback { store })
        }
    }
}

/// Validates all 34 cells of the exact single-record NodeScene failure-evidence contract.
pub(super) fn validate_node_scene_failure_evidence<'session, 'storage, 'control, O, PP, U>(
    session: &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
    root: FailureEvidenceMember<'session, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    counts: SceneEvidenceCounts,
) -> bool
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let expected = node_scene_evidence_values(counts);
    session
        .visit_records(root, |row_index, row| {
            let mut valid = row_index == 0 && row.fields() == NODE_SCENE_EVIDENCE_FIELD_COUNT;
            for (field, expected) in expected.into_iter().enumerate() {
                let matches = row
                    .scalar::<u32, _>(field, |value| value == expected)
                    .unwrap_or(false);
                valid &= matches;
            }
            valid
        })
        .unwrap_or(false)
}

fn node_scene_evidence_values(
    counts: SceneEvidenceCounts,
) -> [u32; NODE_SCENE_EVIDENCE_FIELD_COUNT] {
    let mut values = [0; NODE_SCENE_EVIDENCE_FIELD_COUNT];
    values[node_scene_evidence_field::SCHEMA_REVISION] = SCENE_SCHEMA_REVISION;
    values[node_scene_evidence_field::OUTCOME] = NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE;
    values[node_scene_evidence_field::CANONICAL_BLOCK_COUNT] = counts.canonical_block_count;
    values[node_scene_evidence_field::NODE_COUNT] = counts.node_count;
    values[node_scene_evidence_field::CAMERA_COUNT] = counts.camera_count;
    values[node_scene_evidence_field::LIGHT_COUNT] = counts.light_count;
    values[node_scene_evidence_field::IMAGE_COUNT] = counts.image_count;
    values[node_scene_evidence_field::SAMPLER_COUNT] = counts.sampler_count;
    values[node_scene_evidence_field::TEXTURE_COUNT] = counts.texture_count;
    values[node_scene_evidence_field::MATERIAL_COUNT] = counts.material_count;
    values[node_scene_evidence_field::MATERIAL_BINDING_COUNT] = counts.material_binding_count;
    values[node_scene_evidence_field::LOGICAL_MESH_COUNT] = counts.logical_mesh_count;
    values[node_scene_evidence_field::PRIMITIVE_COUNT] = counts.primitive_count;
    values[node_scene_evidence_field::ATTRIBUTE_COUNT] = counts.attribute_count;
    values[node_scene_evidence_field::MESH_INSTANCE_COUNT] = counts.mesh_instance_count;
    values[node_scene_evidence_field::SKIN_COUNT] = counts.skin_count;
    values[node_scene_evidence_field::LOGICAL_MORPH_COUNT] = counts.logical_morph_count;
    values[node_scene_evidence_field::MORPH_DEFINITION_COUNT] = counts.morph_definition_count;
    values[node_scene_evidence_field::PRIMITIVE_DEFORMATION_COUNT] =
        counts.primitive_deformation_count;
    values[node_scene_evidence_field::MORPH_TARGET_COUNT] = counts.morph_target_count;
    values[node_scene_evidence_field::MORPH_DELTA_COUNT] = counts.morph_delta_count;
    values[node_scene_evidence_field::ANIMATION_CLIP_COUNT] = counts.animation_clip_count;
    values[node_scene_evidence_field::ANIMATION_CHANNEL_COUNT] = counts.animation_channel_count;
    values[node_scene_evidence_field::PAYLOAD_BLOCK_COUNT] = counts.payload_block_count;
    values[node_scene_evidence_field::PROFILE_SEGMENT_COUNT] = counts.profile_segment_count;
    values[node_scene_evidence_field::PROFILE_RESOURCE_COUNT] = counts.profile_resource_count;
    values[node_scene_evidence_field::PROFILE_SOURCE_COUNT] = counts.profile_source_count;
    values[node_scene_evidence_field::PROFILE_PROVENANCE_COUNT] = counts.profile_provenance_count;
    values[node_scene_evidence_field::PROFILE_OPERATION_COUNT] = counts.profile_operation_count;
    values[node_scene_evidence_field::PROFILE_PARENT_COUNT] = counts.profile_parent_count;
    values[node_scene_evidence_field::PROFILE_OWNER_COUNT] = counts.profile_owner_count;
    values[node_scene_evidence_field::PROFILE_ATTACHMENT_COUNT] = counts.profile_attachment_count;
    values[node_scene_evidence_field::PROFILE_METADATA_ENTRY_COUNT] =
        counts.profile_metadata_entry_count;
    values[node_scene_evidence_field::SHAREABLE_LEAF_BLOCK_COUNT] =
        counts.shareable_leaf_block_count;
    values
}

pub(super) fn post_evidence_failure_code(cause: &StoreRuntimeError) -> Option<u32> {
    match cause {
        StoreRuntimeError::Control(_) => Some(NODE_SCENE_CONTROL_INTERRUPTION_CODE),
        StoreRuntimeError::Build(cause) => post_evidence_build_failure_code(cause.class()),
        StoreRuntimeError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted | CounterClass::Consumed,
                ..
            }
            | LedgerCause::ArithmeticOverflow {
                class: CounterClass::Attempted | CounterClass::Consumed,
                ..
            }
            | LedgerCause::ConsumedExceedsAttempted { .. },
        ) => Some(NODE_SCENE_WORK_INTERRUPTION_CODE),
        StoreRuntimeError::Ledger(
            LedgerCause::LimitExceeded {
                class:
                    CounterClass::Live
                    | CounterClass::PendingPublication
                    | CounterClass::Committed
                    | CounterClass::PublishedOutcomeEvidence,
                ..
            }
            | LedgerCause::ArithmeticOverflow {
                class:
                    CounterClass::Live
                    | CounterClass::PendingPublication
                    | CounterClass::Committed
                    | CounterClass::PublishedOutcomeEvidence,
                ..
            },
        )
        | StoreRuntimeError::BatchAllocationFailed
        | StoreRuntimeError::PhysicalByteLimit { .. }
        | StoreRuntimeError::PhysicalAllocationLimit { .. }
        | StoreRuntimeError::AllocationFailed
        | StoreRuntimeError::MembershipExhausted => Some(NODE_SCENE_STORAGE_REJECTION_CODE),
        _ => None,
    }
}

pub(super) const fn post_evidence_build_failure_code(class: StoreBuildFailureClass) -> Option<u32> {
    match class {
        StoreBuildFailureClass::AllocationFailed => Some(NODE_SCENE_STORAGE_REJECTION_CODE),
        _ => None,
    }
}

pub(super) fn node_scene_failure_code(cause: &StoreRuntimeError) -> u32 {
    post_evidence_failure_code(cause).unwrap_or(NODE_SCENE_BUILD_INVARIANT_CODE)
}

impl PreparedNodeSceneConstruction {
    pub(in crate::canonical_scene::store) fn handoff(self) -> NodeSceneConstructionTerminal {
        match self {
            Self::Success(prepared) => {
                let PreparedNodeSceneSuccess {
                    generation,
                    proof,
                    store,
                } = prepared;
                let published = store.handoff();
                let generation = generation.into_id();
                NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts {
                    scene: ValidatedNodeScene {
                        generation,
                        canonical: published.canonical,
                        proof,
                        #[cfg(test)]
                        semantic_read_fault: None,
                    },
                    report: NodeSceneSuccessReport {
                        outcome_evidence: published.outcome_evidence,
                        ledger_evidence: published.ledger_evidence,
                    },
                })
            }
            Self::RichFailure(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::RichFailure(NodeSceneRichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                })
            }
            Self::MinimalFallback(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::MinimalFallback(TracerSceneFallback {
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}
