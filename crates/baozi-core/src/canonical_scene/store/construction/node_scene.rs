use baozi_operation::__retained_store_integration::activation::{
    CanonicalCandidateStoreSession, CentrallyRejectedCanonicalStoreSession,
    InterruptedCanonicalStoreSession, NeverActivatedStoreSession, PendingMinimalFallback,
    RejectedCanonicalStoreSession, VerifiedMinimalFallback,
};
#[cfg(test)]
use baozi_operation::__retained_store_integration::authority::PreparedStoreSuccess;
use baozi_operation::__retained_store_integration::authority::{
    FreshStoreLineage, PreparedStoreFailure, PreparedStoreFallback,
};
use baozi_operation::__retained_store_integration::producer::{
    CanonicalActivationRequest, CanonicalActivationRequestIdentity,
    mint_canonical_activation_request,
};
#[cfg(test)]
use baozi_operation::PublishedOutcomeEvidenceStore;
use baozi_operation::{
    BytesPlan, FinalProgress, OperationOwner, ProgressPhase, ProgressUnit, PublishedCanonicalStore,
    PublishedRichFailureStore, PublishedStoreFallbackOutput, RecordBlock, RecordPlan, SequencePlan,
    StoreLedgerEvidence, StorePlanError, StoreRuntimeError, StoredScalar, Utf8Plan,
};
use thiserror::Error;

use crate::identity::SceneGenerationId;

use super::super::production::{
    AnimationAuthoringFailure, AppearanceAuthoringFailure, DeformationAuthoringFailure,
    GeometryAuthoringFailure, NodeAuthoringFailure, OperationAuthoringControlError,
    ProfileAuthoringFailure, ProfileOwnerCatalogError, SceneAuthoringAggregateError,
    SpatialAuthoringFailure,
};
use super::super::schema::{
    CanonicalBlockRef, NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema,
};
use super::super::validation::NodeSceneValidationProof;
use super::{
    ConstructionBrand, SceneConstructionOwner, SceneConstructionPhase, SceneConstructionStartError,
    SceneConstructionUnit,
};

pub(super) type ConstructionRequestIdentity = CanonicalActivationRequestIdentity;

impl ConstructionBrand {
    fn reserve_with_activation_request()
    -> Result<(Self, CanonicalActivationRequest), SceneConstructionStartError> {
        mint_canonical_activation_request()
            .map(|(identity, request)| (Self(identity), request))
            .ok_or(SceneConstructionStartError::BrandExhausted)
    }

    fn activation_request_identity(self) -> CanonicalActivationRequestIdentity {
        self.0
    }
}

mod animation;
mod appearance;
pub(crate) mod edit;
mod geometry;
mod materialization;
pub(in crate::canonical_scene::store) use materialization::NodeSceneActivationRequest;
pub(in crate::canonical_scene::store) use materialization::NodeSceneMaterializedRefs;
#[cfg(feature = "_scene-integration")]
pub(in crate::canonical_scene::store) use materialization::maximum_node_scene_structural_work;
#[cfg(test)]
pub(in crate::canonical_scene::store) use materialization::validate_node_scene_batches_for_test;
#[cfg(feature = "_scene-integration")]
pub use materialization::{
    NodeSceneActivationPostflight, NodeSceneActivationTransaction, NodeSceneEvidenceRecipe,
    NodeSceneLaneOutput, NodeSceneLanePlan, NodeSceneLaneRejection, NodeSceneLaneValidationProof,
    NodeSceneSessionActivationBootstrap,
};
mod profile;
mod publication;
#[cfg(feature = "_scene-integration")]
pub use publication::{
    NodeSceneFrozenGenerationRejection, NodeSceneHandoffPairingError, NodeSceneHandoffToken,
    NodeSceneLaneFreezePermit, NodeSceneLaneGenerationRejection, NodeSceneLaneJointCommit,
    NodeScenePublishedSuccess, consume_node_scene_freeze_permit,
};
mod scratch;
pub(in crate::canonical_scene::store) use scratch::{
    NodeSceneCentralValidationPlan, NodeSceneMeteredValidationScratch,
    NodeSceneValidationScratchPlan,
};
pub(in crate::canonical_scene::store) mod semantic;
pub(in crate::canonical_scene::store) mod snapshot;
#[cfg(test)]
mod test_support;
#[cfg(test)]
pub(in crate::canonical_scene::store) use test_support::NodeSceneTestTransaction;
mod view;

#[cfg(test)]
mod tests;

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
const NODE_SCENE_PROFILE_AUTHORING_REJECTION_CODE: u32 = 0x5343_1108;
const NODE_SCENE_PROFILE_OWNER_CATALOG_REJECTION_CODE: u32 = 0x5343_1109;
const NODE_SCENE_PLANNING_REJECTION_CODE: u32 = 0x5343_110a;

fn checked_validation_work_sum<const N: usize>(values: [u64; N]) -> Result<u64, StorePlanError> {
    values
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .ok_or(StorePlanError::SizeOverflow)
}

fn record_validation_work(plan: RecordPlan) -> Result<u64, StorePlanError> {
    let cells = plan
        .rows()
        .checked_mul(plan.fields())
        .ok_or(StorePlanError::SizeOverflow)?;
    u64::try_from(cells).map_err(|_| StorePlanError::SizeOverflow)
}

fn optional_record_validation_work(plan: Option<RecordPlan>) -> Result<u64, StorePlanError> {
    plan.map_or(Ok(0), record_validation_work)
}

fn sequence_validation_work<T: StoredScalar>(plan: SequencePlan<T>) -> Result<u64, StorePlanError> {
    u64::try_from(plan.element_count()).map_err(|_| StorePlanError::SizeOverflow)
}

fn optional_sequence_validation_work<T: StoredScalar>(
    plan: Option<SequencePlan<T>>,
) -> Result<u64, StorePlanError> {
    plan.map_or(Ok(0), sequence_validation_work)
}

fn optional_bytes_validation_work(plan: Option<BytesPlan>) -> Result<u64, StorePlanError> {
    plan.map_or(Ok(0), |plan| {
        u64::try_from(plan.bytes()).map_err(|_| StorePlanError::SizeOverflow)
    })
}

fn optional_utf8_validation_work(plan: Option<Utf8Plan>) -> Result<u64, StorePlanError> {
    plan.map_or(Ok(0), |plan| {
        u64::try_from(plan.bytes()).map_err(|_| StorePlanError::SizeOverflow)
    })
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct NodeSceneBuildFaults {
    alter_first_authored_transform: bool,
    alter_first_geometry_f32: bool,
    spatial: Option<test_support::SpatialBuildFault>,
    deformation: Option<test_support::DeformationBuildFault>,
    profile: profile::ProfileBuildFault,
    planning: Option<materialization::NodeScenePlanningFault>,
    panic_central_validator: bool,
}

#[cfg(test)]
impl NodeSceneBuildFaults {
    const fn none() -> Self {
        Self {
            alter_first_authored_transform: false,
            alter_first_geometry_f32: false,
            spatial: None,
            deformation: None,
            profile: profile::ProfileBuildFault::none(),
            planning: None,
            panic_central_validator: false,
        }
    }

    const fn altered_authored_transform() -> Self {
        Self {
            alter_first_authored_transform: true,
            ..Self::none()
        }
    }

    const fn altered_geometry_f32() -> Self {
        Self {
            alter_first_geometry_f32: true,
            ..Self::none()
        }
    }

    const fn altered_profile(fault: profile::ProfileBuildFault) -> Self {
        Self {
            profile: fault,
            ..Self::none()
        }
    }

    const fn altered_spatial(fault: test_support::SpatialBuildFault) -> Self {
        Self {
            spatial: Some(fault),
            ..Self::none()
        }
    }

    const fn altered_deformation(fault: test_support::DeformationBuildFault) -> Self {
        Self {
            deformation: Some(fault),
            ..Self::none()
        }
    }

    const fn minimal_fallback() -> Self {
        Self {
            panic_central_validator: true,
            ..Self::none()
        }
    }
}

#[cfg(test)]
type PreparedNodeStoreSuccess = PreparedStoreSuccess<
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type PreparedNodeStoreFailure = PreparedStoreFailure<RecordBlock, NodeSceneOutcomeEvidenceSchema>;
type ActivatedNodeSceneCandidate<
    O = SceneConstructionOwner,
    PP = SceneConstructionPhase,
    U = SceneConstructionUnit,
> = CanonicalCandidateStoreSession<
    O,
    PP,
    U,
    materialization::NodeSceneLaneOutput,
    materialization::NodeSceneLaneValidationProof,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type RejectedNodeSceneActivation<O, PP, U> =
    RejectedCanonicalStoreSession<O, PP, U, materialization::NodeSceneLaneRejection>;
type NeverActivatedNodeScene<O, PP, U, L = FreshStoreLineage> =
    NeverActivatedStoreSession<O, PP, U, materialization::NodeSceneLaneRejection, L>;
type CentrallyRejectedNodeScene<O, PP, U> = CentrallyRejectedCanonicalStoreSession<
    O,
    PP,
    U,
    materialization::NodeSceneLaneRejection,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type InterruptedNodeScene<O, PP, U> =
    InterruptedCanonicalStoreSession<O, PP, U, RecordBlock, NodeSceneOutcomeEvidenceSchema>;
#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeSceneConstructionRejection<
    E = OperationAuthoringControlError,
> where
    E: std::error::Error + Send + Sync + 'static,
{
    #[error("spatial authoring validation rejected the Scene batch")]
    SpatialAuthoring(#[source] SpatialAuthoringFailure<E>),
    #[error("node-domain authoring validation rejected the Scene batch")]
    NodeAuthoring(#[source] NodeAuthoringFailure<E>),
    #[error("appearance-domain authoring validation rejected the Scene batch")]
    AppearanceAuthoring(#[source] AppearanceAuthoringFailure<E>),
    #[error("geometry-domain authoring validation rejected the Scene batch")]
    GeometryAuthoring(#[source] GeometryAuthoringFailure<E>),
    #[error("deformation-domain authoring validation rejected the Scene batch")]
    DeformationAuthoring(#[source] DeformationAuthoringFailure<E>),
    #[error("animation-domain authoring validation rejected the Scene batch")]
    AnimationAuthoring(#[source] AnimationAuthoringFailure<E>),
    #[error("profile owner catalog construction rejected the Scene batch")]
    ProfileOwnerCatalog(#[source] ProfileOwnerCatalogError),
    #[error("profile authoring validation rejected the Scene batch")]
    ProfileAuthoring(#[source] ProfileAuthoringFailure<E>),
    #[error("aggregate authoring validation rejected the Scene batch")]
    Aggregate(#[source] SceneAuthoringAggregateError),
    #[error("canonical Scene physical planning rejected the validated batch")]
    Planning(#[source] StorePlanError),
    #[error("canonical Scene materialization rejected the sealed construction plan")]
    Materialization(#[source] StoreRuntimeError),
}

impl<E> NodeSceneConstructionRejection<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    const fn stable_code(&self) -> u32 {
        match self {
            Self::SpatialAuthoring(_) => NODE_SCENE_SPATIAL_AUTHORING_REJECTION_CODE,
            Self::NodeAuthoring(_) => NODE_SCENE_NODE_AUTHORING_REJECTION_CODE,
            Self::AppearanceAuthoring(_) => NODE_SCENE_APPEARANCE_AUTHORING_REJECTION_CODE,
            Self::GeometryAuthoring(_) => NODE_SCENE_GEOMETRY_AUTHORING_REJECTION_CODE,
            Self::DeformationAuthoring(_) => NODE_SCENE_DEFORMATION_AUTHORING_REJECTION_CODE,
            Self::AnimationAuthoring(_) => NODE_SCENE_ANIMATION_AUTHORING_REJECTION_CODE,
            Self::ProfileOwnerCatalog(_) => NODE_SCENE_PROFILE_OWNER_CATALOG_REJECTION_CODE,
            Self::ProfileAuthoring(_) => NODE_SCENE_PROFILE_AUTHORING_REJECTION_CODE,
            Self::Aggregate(_) => NODE_SCENE_AGGREGATE_AUTHORING_REJECTION_CODE,
            Self::Planning(_) => NODE_SCENE_PLANNING_REJECTION_CODE,
            Self::Materialization(_) => NODE_SCENE_STORAGE_REJECTION_CODE,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneDraft {
    brand: ConstructionBrand,
    directory: CanonicalBlockRef<SceneDirectorySchema>,
}

/// Retained materialization owner awaiting its exact parser-facing descriptor.
///
/// The expected directory remains private to the owner so central validation can reject a
/// descriptor from another construction before registering commitments or allocating scratch.
#[must_use = "a pending node Scene draft owner must be centrally prepared or deliberately dropped"]
#[cfg(test)]
pub(in crate::canonical_scene::store) struct PendingNodeSceneDraft<
    O = SceneConstructionOwner,
    PP = SceneConstructionPhase,
    U = SceneConstructionUnit,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    candidate: ActivatedNodeSceneCandidate<O, PP, U>,
    final_progress: FinalProgress<PP, U>,
}

/// The exact outcome of source-backed materialization before central graph validation.
#[must_use = "a node Scene draft build owns either central validation work or a failure"]
#[allow(
    clippy::large_enum_variant,
    reason = "the failure owner stays inline so materialization preserves exact containment"
)]
#[cfg(test)]
pub(in crate::canonical_scene::store) enum NodeSceneDraftBuild<
    O = SceneConstructionOwner,
    PP = SceneConstructionPhase,
    U = SceneConstructionUnit,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Draft {
        draft: NodeSceneDraft,
        owner: PendingNodeSceneDraft<O, PP, U>,
    },
    Failure {
        authoring_rejection: Option<NodeSceneConstructionRejection<StoreRuntimeError>>,
        // Keep the prepared physical publication last so a future rejection payload destructor
        // cannot unwind after its containment owner has already been released.
        prepared: PreparedNodeSceneFailure,
    },
}

/// Failure continuation retained by the import session until every parser-owned reservation has
/// been released. No terminal ledger snapshot is frozen while this owner is pending.
#[must_use = "a pending Scene failure must be prepared after parser storage is released"]
#[allow(
    clippy::large_enum_variant,
    reason = "all linear failure states remain inline so deferred preparation cannot allocate"
)]
pub(in crate::canonical_scene::store) enum PendingNodeSceneFailure<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    ActivationRejected {
        owner: RejectedNodeSceneActivation<O, PP, U>,
        evidence_plan: RecordPlan,
        evidence_counts: SceneEvidenceCounts,
        final_progress: FinalProgress<PP, U>,
    },
    NeverActivated {
        owner: NeverActivatedNodeScene<O, PP, U, L>,
        evidence_plan: RecordPlan,
        evidence_counts: SceneEvidenceCounts,
        final_progress: FinalProgress<PP, U>,
    },
    CentralRejected {
        owner: CentrallyRejectedNodeScene<O, PP, U>,
        evidence_counts: SceneEvidenceCounts,
        final_progress: FinalProgress<PP, U>,
    },
    CentralInterrupted {
        owner: InterruptedNodeScene<O, PP, U>,
        evidence_counts: SceneEvidenceCounts,
        final_progress: FinalProgress<PP, U>,
    },
    PreparedFallback(VerifiedMinimalFallback<StoreRuntimeError>),
    DeferredFallback(PendingMinimalFallback<O, PP, U>),
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
    pub(in crate::canonical_scene::store) profile_segment_count: u32,
    pub(in crate::canonical_scene::store) profile_resource_count: u32,
    pub(in crate::canonical_scene::store) profile_source_count: u32,
    pub(in crate::canonical_scene::store) profile_provenance_count: u32,
    pub(in crate::canonical_scene::store) profile_operation_count: u32,
    pub(in crate::canonical_scene::store) profile_parent_count: u32,
    pub(in crate::canonical_scene::store) profile_owner_count: u32,
    pub(in crate::canonical_scene::store) profile_attachment_count: u32,
    pub(in crate::canonical_scene::store) profile_metadata_entry_count: u32,
    pub(in crate::canonical_scene::store) shareable_leaf_block_count: u32,
}

impl SceneEvidenceCounts {
    pub(in crate::canonical_scene::store) fn profile_owner_layout(
        self,
    ) -> Option<crate::canonical_scene::store::production::ProfileOwnerLayout> {
        use crate::canonical_scene::store::production::{ProfileOwnerCounts, ProfileOwnerLayout};
        ProfileOwnerLayout::try_new(ProfileOwnerCounts {
            nodes: self.node_count.checked_sub(1)?,
            cameras: self.camera_count,
            lights: self.light_count,
            images: self.image_count,
            samplers: self.sampler_count,
            textures: self.texture_count,
            texture_bindings: self.material_binding_count,
            materials: self.material_count,
            logical_meshes: self.logical_mesh_count,
            primitives: self.primitive_count,
            attributes: self.attribute_count,
            mesh_instances: self.mesh_instance_count,
            skins: self.skin_count,
            morph_definitions: self.morph_definition_count,
            morph_deltas: self.morph_delta_count,
            animation_clips: self.animation_clip_count,
            animation_channels: self.animation_channel_count,
        })
        .ok()
    }

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
            profile_segment_count: 0,
            profile_resource_count: 0,
            profile_source_count: 0,
            profile_provenance_count: 0,
            profile_operation_count: 0,
            profile_parent_count: 0,
            profile_owner_count: 0,
            profile_attachment_count: 0,
            profile_metadata_entry_count: 0,
            shareable_leaf_block_count: 0,
        }
    }
}

pub(in crate::canonical_scene::store) struct PreparedNodeSceneRichFailure {
    store: PreparedNodeStoreFailure,
}

pub(in crate::canonical_scene::store) struct PreparedNodeSceneFallback {
    store: PreparedStoreFallback,
}

pub(in crate::canonical_scene::store) struct NodeSceneFallback {
    fallback: PublishedStoreFallbackOutput,
    ledger_evidence: StoreLedgerEvidence,
}

impl NodeSceneFallback {
    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }
}

#[must_use = "every prepared node Scene failure owns an exact frozen publication"]
#[allow(
    clippy::large_enum_variant,
    reason = "prepared failure owners remain inline so terminal handoff is allocation-free"
)]
pub(in crate::canonical_scene::store) enum PreparedNodeSceneFailure {
    RichFailure(PreparedNodeSceneRichFailure),
    MinimalFallback(PreparedNodeSceneFallback),
}

#[must_use = "every prepared node Scene owns an exact frozen publication"]
#[allow(
    clippy::large_enum_variant,
    reason = "prepared publication owners remain inline so terminal handoff is allocation-free"
)]
#[cfg(test)]
pub(in crate::canonical_scene::store) enum PreparedNodeSceneConstruction {
    Success {
        token: publication::NodeSceneHandoffToken,
        // The physical publication owner must drop last so an earlier field destructor cannot
        // escape fail-closed containment before the terminal store has been prepared.
        store: PreparedNodeStoreSuccess,
    },
    Failure(PreparedNodeSceneFailure),
}

#[cfg(test)]
impl From<PreparedNodeSceneFailure> for PreparedNodeSceneConstruction {
    fn from(failure: PreparedNodeSceneFailure) -> Self {
        Self::Failure(failure)
    }
}

/// Private inspector over a centrally validated node-domain Scene.
pub(in crate::canonical_scene::store) struct SceneStorage {
    generation: SceneGenerationId,
    tables: super::super::typed_storage::SceneTables,
    canonical: PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>,
    proof: NodeSceneValidationProof,
    #[cfg(test)]
    semantic_read_fault: Option<semantic::NodeSceneSemanticReadFault>,
}

impl SceneStorage {
    pub(in crate::canonical_scene::store) fn read_nodes(
        &self,
    ) -> &super::super::typed_storage::NodeStorage {
        self.tables.nodes.as_ref()
    }

    pub(in crate::canonical_scene::store) fn read_spatial(
        &self,
    ) -> &super::super::typed_storage::spatial::SpatialStorage {
        self.tables.spatial.as_ref()
    }

    pub(in crate::canonical_scene::store) fn read_geometry(
        &self,
    ) -> &super::super::typed_storage::GeometryStorage {
        self.tables.geometry.as_ref()
    }

    pub(in crate::canonical_scene::store) fn read_deformation(
        &self,
    ) -> &super::super::typed_storage::deformation::DeformationStorage {
        self.tables.deformation.as_ref()
    }
    pub(in crate::canonical_scene::store) const fn read_proof(&self) -> &NodeSceneValidationProof {
        &self.proof
    }

    pub(in crate::canonical_scene::store) const fn read_canonical(
        &self,
    ) -> &PublishedCanonicalStore<RecordBlock, SceneDirectorySchema> {
        &self.canonical
    }
}

#[cfg(test)]
pub(in crate::canonical_scene::store) struct NodeSceneSuccessReport {
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

#[cfg(test)]
pub(in crate::canonical_scene::store) struct NodeSceneSuccessParts {
    pub(in crate::canonical_scene::store) scene: SceneStorage,
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
#[cfg(test)]
pub(in crate::canonical_scene::store) enum NodeSceneConstructionTerminal {
    Success(NodeSceneSuccessParts),
    RichFailure(NodeSceneRichFailure),
    MinimalFallback(NodeSceneFallback),
}
