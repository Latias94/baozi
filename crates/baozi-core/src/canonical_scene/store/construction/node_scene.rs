use baozi_operation::{
    OutcomeEvidenceRecordRef, PublishedCanonicalStore, PublishedOutcomeEvidenceStore,
    PublishedRichFailureStore, RecordBlock, RetainedStoreReservation, RetainedStoreWorkControl,
    StoreFailureAssembly, StoreFailureValidation, StoreLedgerEvidence, StoreRuntimeError,
    StoreWorkError, ValidatedStoreSuccess,
};
use thiserror::Error;

use crate::SCENE_VALIDATION_POLL_INTERVAL;
use crate::identity::{ReservedSceneGeneration, SceneGenerationId};

use super::super::production::{
    AnimationAuthoringFailure, AppearanceAuthoringFailure, DeformationAuthoringFailure,
    GeometryAuthoringFailure, NodeAuthoringFailure, OperationAuthoringControlError,
    SceneAuthoringAggregateError, SpatialAuthoringFailure, ValidatedAnimationBatch,
    ValidatedAppearanceBatch, ValidatedDeformationBatch, ValidatedGeometryBatch,
    ValidatedNodeBatch, ValidatedSpatialBatch,
};
use super::super::schema::{
    CanonicalBlockRef, NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema,
};
use super::super::validation::NodeSceneValidationProof;
use super::{
    ConstructionBrand, PreparedTracerSceneFallback, SceneConstructionOperation,
    SceneConstructionOwner, SceneConstructionPhase, SceneConstructionUnit, SceneStoreAssembly,
    SceneStoreControlCheckpoint, SceneStoreKeys, SceneStoreOperation, TracerSceneFallback,
};

mod animation;
mod appearance;
mod deformation;
pub(crate) mod edit;
mod geometry;
mod materialization;
mod publication;
mod scratch;
pub(in crate::canonical_scene::store) mod semantic;
pub(in crate::canonical_scene::store) mod snapshot;
mod view;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(in crate::canonical_scene::store) use tests::{
    GLTF_SOURCE_VECTOR_SCHEMA_ID, GltfCapabilityCanonicalShapeObservation,
    GltfCapabilitySourceShapeObservation, OBJ_SOURCE_VECTOR_SCHEMA_ID,
    ObjCapabilityCanonicalShapeObservation, ObjCapabilitySourceShapeObservation,
    build_evidence_publication_limit_terminal_for_probe, build_gltf_capability_scene_for_probe,
    build_obj_capability_scene_for_probe, build_protected_terminal_activation_for_probe,
    capture_gltf_capability_snapshots_for_probe, capture_obj_capability_snapshots_for_probe,
    collect_gltf_capability_vectors_for_probe, collect_obj_capability_vectors_for_probe,
    inspect_gltf_capability_semantics_for_probe,
    inspect_gltf_capability_semantics_with_first_transform_for_probe,
    inspect_obj_capability_semantics_for_probe,
};

const NODE_SCENE_BUILD_INVARIANT_CODE: u32 = 0x5343_1001;
const NODE_SCENE_CONTROL_INTERRUPTION_CODE: u32 = 0x5343_1002;
const NODE_SCENE_WORK_INTERRUPTION_CODE: u32 = 0x5343_1003;
const NODE_SCENE_COMMITMENT_REJECTION_CODE: u32 = 0x5343_1004;
const NODE_SCENE_STORAGE_REJECTION_CODE: u32 = 0x5343_1005;
const NODE_SCENE_NODE_AUTHORING_REJECTION_CODE: u32 = 0x5343_1101;
const NODE_SCENE_APPEARANCE_AUTHORING_REJECTION_CODE: u32 = 0x5343_1102;
const NODE_SCENE_GEOMETRY_AUTHORING_REJECTION_CODE: u32 = 0x5343_1103;
const NODE_SCENE_DEFORMATION_AUTHORING_REJECTION_CODE: u32 = 0x5343_1104;
const NODE_SCENE_ANIMATION_AUTHORING_REJECTION_CODE: u32 = 0x5343_1105;
const NODE_SCENE_AGGREGATE_AUTHORING_REJECTION_CODE: u32 = 0x5343_1106;
const NODE_SCENE_SPATIAL_AUTHORING_REJECTION_CODE: u32 = 0x5343_1107;

#[derive(Clone, Copy)]
struct NodeSceneBuildFaults {
    alter_first_authored_transform: bool,
    alter_first_geometry_f32: bool,
    outcome_evidence_commitment: u64,
}

impl NodeSceneBuildFaults {
    const fn none() -> Self {
        Self {
            alter_first_authored_transform: false,
            alter_first_geometry_f32: false,
            outcome_evidence_commitment: 1,
        }
    }

    #[cfg(test)]
    const fn altered_authored_transform() -> Self {
        Self {
            alter_first_authored_transform: true,
            ..Self::none()
        }
    }

    #[cfg(test)]
    const fn excess_outcome_evidence_commitment() -> Self {
        Self {
            outcome_evidence_commitment: 2,
            ..Self::none()
        }
    }

    #[cfg(test)]
    const fn altered_geometry_f32() -> Self {
        Self {
            alter_first_geometry_f32: true,
            ..Self::none()
        }
    }

    fn transform_scalar(&mut self, value: f32) -> f32 {
        if !self.alter_first_authored_transform {
            return value;
        }
        self.alter_first_authored_transform = false;
        f32::NAN
    }

    fn geometry_f32(&mut self, value: f32) -> f32 {
        if !self.alter_first_geometry_f32 {
            return value;
        }
        self.alter_first_geometry_f32 = false;
        f32::NAN
    }
}

struct NodeSceneWorkMeter {
    started: bool,
    work_since_poll: u64,
}

impl NodeSceneWorkMeter {
    const fn new() -> Self {
        Self {
            started: false,
            work_since_poll: 0,
        }
    }

    fn observe(
        &mut self,
        control: &mut RetainedStoreWorkControl<
            '_,
            SceneConstructionOwner,
            SceneConstructionPhase,
            SceneConstructionUnit,
        >,
        amount: u64,
    ) -> Result<(), StoreWorkError> {
        if amount == 0 {
            return Ok(());
        }
        if !self.started {
            self.poll_now(control)?;
        }
        let mut remaining = amount;
        while remaining != 0 {
            let until_poll = SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll;
            let chunk = remaining.min(until_poll);
            control.charge_observed(chunk)?;
            self.work_since_poll += chunk;
            remaining -= chunk;
            if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
                self.poll_now(control)?;
            }
        }
        Ok(())
    }

    fn finish(
        &mut self,
        control: &mut RetainedStoreWorkControl<
            '_,
            SceneConstructionOwner,
            SceneConstructionPhase,
            SceneConstructionUnit,
        >,
    ) -> Result<(), StoreWorkError> {
        self.poll_now(control)
    }

    fn observe_checkpoint(
        &mut self,
        control: &mut SceneStoreControlCheckpoint<'_>,
        keys: SceneStoreKeys,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        if amount == 0 {
            return Ok(());
        }
        if !self.started {
            self.poll_checkpoint_now(control)?;
        }
        let mut remaining = amount;
        while remaining != 0 {
            let until_poll = SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll;
            let chunk = remaining.min(until_poll);
            control.charge_observed(keys.dimension, chunk)?;
            self.work_since_poll += chunk;
            remaining -= chunk;
            if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
                self.poll_checkpoint_now(control)?;
            }
        }
        Ok(())
    }

    fn poll_now(
        &mut self,
        control: &mut RetainedStoreWorkControl<
            '_,
            SceneConstructionOwner,
            SceneConstructionPhase,
            SceneConstructionUnit,
        >,
    ) -> Result<(), StoreWorkError> {
        control.poll()?;
        self.started = true;
        self.work_since_poll = 0;
        Ok(())
    }

    fn poll_checkpoint_now(
        &mut self,
        control: &mut SceneStoreControlCheckpoint<'_>,
    ) -> Result<(), StoreRuntimeError> {
        control.poll()?;
        self.started = true;
        self.work_since_poll = 0;
        Ok(())
    }
}

type ValidatedNodeStoreSuccess = ValidatedStoreSuccess<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type PreparedNodeStoreSuccess = baozi_operation::PreparedStoreSuccess<
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type NodeSceneStoreFailureAssembly = StoreFailureAssembly<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type NodeSceneStoreFailureValidation = StoreFailureValidation<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type PreparedNodeStoreFailure =
    baozi_operation::PreparedStoreFailure<RecordBlock, NodeSceneOutcomeEvidenceSchema>;

struct FreshNodeSceneOwner {
    raw: SceneStoreOperation,
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
}

/// A validated canonical Scene input coupled to the exact fresh operation that paid for it.
#[must_use = "the validated node Scene construction must be prepared or deliberately dropped"]
pub(in crate::canonical_scene::store) struct ValidatedNodeSceneConstruction<'input> {
    animations: ValidatedAnimationBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    deformation:
        ValidatedDeformationBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    geometry: ValidatedGeometryBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    appearance: ValidatedAppearanceBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    nodes: ValidatedNodeBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    spatial: ValidatedSpatialBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    // The operation owner must outlive every compiled-plan reservation above.
    owner: FreshNodeSceneOwner,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeSceneConstructionRejection {
    #[error("spatial authoring validation rejected the Scene batch")]
    SpatialAuthoring(#[source] SpatialAuthoringFailure<OperationAuthoringControlError>),
    #[error("node-domain authoring validation rejected the Scene batch")]
    NodeAuthoring(#[source] NodeAuthoringFailure<OperationAuthoringControlError>),
    #[error("appearance-domain authoring validation rejected the Scene batch")]
    AppearanceAuthoring(#[source] AppearanceAuthoringFailure<OperationAuthoringControlError>),
    #[error("geometry-domain authoring validation rejected the Scene batch")]
    GeometryAuthoring(#[source] GeometryAuthoringFailure<OperationAuthoringControlError>),
    #[error("deformation-domain authoring validation rejected the Scene batch")]
    DeformationAuthoring(#[source] DeformationAuthoringFailure<OperationAuthoringControlError>),
    #[error("animation-domain authoring validation rejected the Scene batch")]
    AnimationAuthoring(#[source] AnimationAuthoringFailure<OperationAuthoringControlError>),
    #[error("aggregate authoring validation rejected the Scene batch")]
    Aggregate(#[source] SceneAuthoringAggregateError),
}

impl NodeSceneConstructionRejection {
    const fn stable_code(&self) -> u32 {
        match self {
            Self::SpatialAuthoring(_) => NODE_SCENE_SPATIAL_AUTHORING_REJECTION_CODE,
            Self::NodeAuthoring(_) => NODE_SCENE_NODE_AUTHORING_REJECTION_CODE,
            Self::AppearanceAuthoring(_) => NODE_SCENE_APPEARANCE_AUTHORING_REJECTION_CODE,
            Self::GeometryAuthoring(_) => NODE_SCENE_GEOMETRY_AUTHORING_REJECTION_CODE,
            Self::DeformationAuthoring(_) => NODE_SCENE_DEFORMATION_AUTHORING_REJECTION_CODE,
            Self::AnimationAuthoring(_) => NODE_SCENE_ANIMATION_AUTHORING_REJECTION_CODE,
            Self::Aggregate(_) => NODE_SCENE_AGGREGATE_AUTHORING_REJECTION_CODE,
        }
    }
}

/// A consumed validation attempt which still owns its exact operation and rejection.
#[must_use = "a rejected node Scene construction must preserve its rich failure or be dropped"]
pub(in crate::canonical_scene::store) struct RejectedNodeSceneConstruction {
    operation: SceneConstructionOperation,
    failure: NodeSceneConstructionRejection,
}

pub(in crate::canonical_scene::store) struct NodeSceneDraft {
    brand: ConstructionBrand,
    directory: CanonicalBlockRef<SceneDirectorySchema>,
}

/// Retained materialization owner awaiting its exact parser-facing descriptor.
///
/// The expected directory remains private to the owner so central validation can reject a
/// descriptor from another construction before registering commitments or allocating scratch.
#[must_use = "a pending node Scene draft owner must be centrally prepared or deliberately dropped"]
pub(in crate::canonical_scene::store) struct PendingNodeSceneDraft {
    assembly: SceneStoreAssembly,
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
    expected_directory: CanonicalBlockRef<SceneDirectorySchema>,
    evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    evidence_counts: SceneEvidenceCounts,
    work: NodeSceneWorkMeter,
    deformation_uniqueness_capacity: usize,
}

/// The exact outcome of source-backed materialization before central graph validation.
#[must_use = "a node Scene draft build owns either central validation work or a failure"]
#[allow(
    clippy::large_enum_variant,
    reason = "the failure owner stays inline so materialization preserves exact containment"
)]
pub(in crate::canonical_scene::store) enum NodeSceneDraftBuild {
    Draft {
        draft: NodeSceneDraft,
        owner: PendingNodeSceneDraft,
    },
    Failure(PreparedNodeSceneFailure),
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct SceneEvidenceCounts {
    pub(in crate::canonical_scene::store) canonical_block_count: u32,
    pub(in crate::canonical_scene::store) node_count: u32,
    pub(in crate::canonical_scene::store) camera_count: u32,
    pub(in crate::canonical_scene::store) light_count: u32,
    pub(in crate::canonical_scene::store) image_count: u32,
    pub(in crate::canonical_scene::store) sampler_count: u32,
    pub(in crate::canonical_scene::store) texture_count: u32,
    pub(in crate::canonical_scene::store) material_count: u32,
    pub(in crate::canonical_scene::store) material_binding_count: u32,
    pub(in crate::canonical_scene::store) logical_mesh_count: u32,
    pub(in crate::canonical_scene::store) primitive_count: u32,
    pub(in crate::canonical_scene::store) attribute_count: u32,
    pub(in crate::canonical_scene::store) mesh_instance_count: u32,
    pub(in crate::canonical_scene::store) skin_count: u32,
    pub(in crate::canonical_scene::store) logical_morph_count: u32,
    pub(in crate::canonical_scene::store) morph_definition_count: u32,
    pub(in crate::canonical_scene::store) primitive_deformation_count: u32,
    pub(in crate::canonical_scene::store) morph_target_count: u32,
    pub(in crate::canonical_scene::store) morph_delta_count: u32,
    pub(in crate::canonical_scene::store) animation_clip_count: u32,
    pub(in crate::canonical_scene::store) animation_channel_count: u32,
    pub(in crate::canonical_scene::store) payload_block_count: u32,
}

impl SceneEvidenceCounts {
    const fn authoring_rejection() -> Self {
        Self {
            canonical_block_count: 0,
            node_count: 0,
            camera_count: 0,
            light_count: 0,
            image_count: 0,
            sampler_count: 0,
            texture_count: 0,
            material_count: 0,
            material_binding_count: 0,
            logical_mesh_count: 0,
            primitive_count: 0,
            attribute_count: 0,
            mesh_instance_count: 0,
            skin_count: 0,
            logical_morph_count: 0,
            morph_definition_count: 0,
            primitive_deformation_count: 0,
            morph_target_count: 0,
            morph_delta_count: 0,
            animation_clip_count: 0,
            animation_channel_count: 0,
            payload_block_count: 0,
        }
    }
}

struct PendingNodeSceneCommitments;

struct ValidatedNodeSceneCandidate {
    store: ValidatedNodeStoreSuccess,
    evidence_counts: SceneEvidenceCounts,
    proof: NodeSceneValidationProof,
    pending_commitments: PendingNodeSceneCommitments,
}

pub(in crate::canonical_scene::store) struct PreparedNodeSceneSuccess {
    generation: ReservedSceneGeneration,
    store: PreparedNodeStoreSuccess,
    proof: NodeSceneValidationProof,
}

pub(in crate::canonical_scene::store) struct PreparedNodeSceneRichFailure {
    store: PreparedNodeStoreFailure,
}

#[must_use = "every prepared node Scene failure owns an exact frozen publication"]
#[allow(
    clippy::large_enum_variant,
    reason = "prepared failure owners remain inline so terminal handoff is allocation-free"
)]
pub(in crate::canonical_scene::store) enum PreparedNodeSceneFailure {
    RichFailure(PreparedNodeSceneRichFailure),
    MinimalFallback(PreparedTracerSceneFallback),
}

#[must_use = "every prepared node Scene owns an exact frozen publication"]
#[allow(
    clippy::large_enum_variant,
    reason = "prepared publication owners remain inline so terminal handoff is allocation-free"
)]
pub(in crate::canonical_scene::store) enum PreparedNodeSceneConstruction {
    Success(PreparedNodeSceneSuccess),
    RichFailure(PreparedNodeSceneRichFailure),
    MinimalFallback(PreparedTracerSceneFallback),
}

impl From<PreparedNodeSceneFailure> for PreparedNodeSceneConstruction {
    fn from(failure: PreparedNodeSceneFailure) -> Self {
        match failure {
            PreparedNodeSceneFailure::RichFailure(failure) => Self::RichFailure(failure),
            PreparedNodeSceneFailure::MinimalFallback(fallback) => Self::MinimalFallback(fallback),
        }
    }
}

/// Private inspector over a centrally validated node-domain Scene.
pub(in crate::canonical_scene::store) struct ValidatedNodeScene {
    generation: SceneGenerationId,
    canonical: PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>,
    proof: NodeSceneValidationProof,
    #[cfg(test)]
    semantic_read_fault: Option<semantic::NodeSceneSemanticReadFault>,
}

pub(in crate::canonical_scene::store) struct NodeSceneSuccessReport {
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

pub(in crate::canonical_scene::store) struct NodeSceneSuccessParts {
    pub(in crate::canonical_scene::store) scene: ValidatedNodeScene,
    pub(in crate::canonical_scene::store) report: NodeSceneSuccessReport,
}

pub(in crate::canonical_scene::store) struct NodeSceneRichFailure {
    outcome_evidence: PublishedRichFailureStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

#[must_use = "the terminal owns the exact node Scene construction outcome"]
#[allow(
    clippy::large_enum_variant,
    reason = "published Scene and evidence owners remain inline so handoff is a pure ownership move"
)]
pub(in crate::canonical_scene::store) enum NodeSceneConstructionTerminal {
    Success(NodeSceneSuccessParts),
    RichFailure(NodeSceneRichFailure),
    MinimalFallback(TracerSceneFallback),
}
