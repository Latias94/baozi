//! Operation-bound node authoring for canonical store construction.

mod control;
pub(in crate::canonical_scene::store) mod input;
mod plan;
mod scene;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) use crate::animation::{
    AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain,
};
pub(super) use control::{
    CanonicalAuthoringControl, OperationAuthoringControlError, ScopedAuthoringControl,
};
#[cfg(test)]
pub(super) use input::NodeAuthoringBatch;
#[cfg(test)]
pub(super) use input::ProfileAuthoringBatch;
#[cfg(test)]
pub(super) use input::ResourceSnapshotAuthoring;
#[cfg(test)]
pub(super) use input::ResourceSnapshotAuthoringTable;
pub(super) use input::animation::AnimationTargetAuthoring;
#[cfg(test)]
pub(super) use input::animation::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
};
#[cfg(test)]
pub(super) use input::appearance::AppearanceAuthoringBatch;
#[cfg(test)]
pub(super) use input::appearance::{
    ImageAuthoring, ImageLocalId, MaterialAuthoring, MaterialBindingAuthoring, MaterialLocalId,
    SamplerAuthoring, SamplerLocalId, TextureAuthoring, TextureLocalId,
};
pub(super) use input::deformation::MorphDeltaPayloadAuthoring;
#[cfg(test)]
pub(super) use input::deformation::{
    DeformationAuthoringBatch, InfluenceAuthoring, InfluenceRowsAuthoring, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MorphDeltaAuthoring, MorphTargetDefinitionAuthoring,
    MorphTargetIdentity, PrimitiveDeformationAuthoring, PrimitiveMorphTargetAuthoring,
    SkinAuthoring, SkinLocalId,
};
#[cfg(test)]
pub(super) use input::geometry::GeometryAuthoringBatch;
#[cfg(test)]
pub(super) use input::geometry::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, LogicalMeshLocalId,
};
pub(super) use input::geometry::{GeometryAttributePayloadAuthoring, GeometryTopologyAuthoring};
#[cfg(test)]
pub(super) use input::lineage::{
    LineageAuthoringBatch, ObservedResourceAuthoring, OwnerProvenanceAuthoring, ProfileOwner,
    ProvenanceOperationAuthoring, ResourceIdentityLocalId, SourceIdentityAuthoring,
    SourceIdentityLocalId,
};
pub(super) use input::lineage::{
    ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentityStability, SessionOpaqueReason,
};
#[cfg(test)]
pub(super) use input::lineage::{
    StableResourceIdentityFixture, classify_stable_resource_identity_fixture,
};
#[cfg(test)]
pub(super) use input::metadata::{MetadataAuthoringBatch, MetadataAuthoringEntry};
#[cfg(test)]
pub(super) use input::spatial::{CameraLocalId, LightLocalId, SpatialAuthoringBatch};
#[cfg(test)]
pub(super) use input::{NodeAuthoringNode, NodeLocalId, NodeParent};
pub(super) use input::{Presence, SceneAuthoringBatch};
pub(super) use plan::animation::{
    AnimationAuthoringFailure, AnimationTargetCatalog, ValidatedAnimationBatch,
    validate_animation_batch_with_control,
};
pub(super) use plan::animation::{
    AnimationAuthoringLimit, AnimationAuthoringSemanticError, AnimationRateKind,
    AnimationSamplePart,
};
pub(super) use plan::appearance::{
    AppearanceAuthoringFailure, ValidatedAppearanceBatch, validate_appearance_batch_with_control,
};
pub(super) use plan::appearance::{AppearanceAuthoringLimit, AppearanceAuthoringSemanticError};
pub(super) use plan::deformation::{
    DeformationAuthoringFailure, ValidatedDeformationBatch, validate_deformation_batch_with_control,
};
pub(super) use plan::deformation::{DeformationAuthoringLimit, DeformationAuthoringSemanticError};
pub(super) use plan::geometry::{
    GeometryAuthoringFailure, ValidatedGeometryBatch, validate_geometry_batch_with_control,
};
pub(super) use plan::geometry::{GeometryAuthoringLimit, GeometryAuthoringSemanticError};
pub(super) use plan::profile::{
    CanonicalProfileOwner, ProfileAuthoringFailure, ProfileOwnerCatalog, ProfileOwnerCatalogError,
    ProfileOwnerCounts, ProfileOwnerLayout, ValidatedProfileBatch,
    validate_profile_batch_with_control,
};
pub(super) use plan::profile::{ProfileAuthoringLimit, ProfileAuthoringSemanticError};
pub(super) use plan::spatial::{
    SpatialAuthoringFailure, ValidatedSpatialBatch, validate_spatial_batch_with_control,
};
pub(super) use plan::spatial::{SpatialAuthoringLimit, SpatialAuthoringSemanticError};
pub(super) use plan::{NodeAuthoringFailure, ValidatedNodeBatch, validate_node_batch_with_control};
pub(super) use plan::{NodeAuthoringLimit, NodeAuthoringSemanticError};
pub(super) use scene::{
    SceneAuthoringAggregateError, SceneAuthoringAggregateLimit, SceneInputEnvelopeError,
    animation_limits_after_deformation, appearance_limits_after_nodes,
    deformation_limits_after_geometry, geometry_limits_after_appearance, node_limits_after_spatial,
    profile_limits_after_animation, scene_input_terminal_attribution_slots,
    validate_scene_aggregate,
};
