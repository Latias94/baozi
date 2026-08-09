//! Operation-bound node authoring for canonical store construction.

mod control;
mod input;
mod plan;
mod scene;

#[cfg(test)]
mod tests;

pub(super) use control::{OperationAuthoringControl, OperationAuthoringControlError};
#[cfg(test)]
pub(super) use input::ProfileAuthoringBatch;
#[cfg(test)]
pub(super) use input::animation::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
};
pub(super) use input::animation::{
    AnimationExtrapolationAuthoring, AnimationInterpolationAuthoring, AnimationTargetAuthoring,
    AnimationTimeDomainAuthoring,
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
    ResourceIdentityLocalId, SourceIdentityAuthoring, SourceIdentityLocalId,
};
pub(super) use input::lineage::{
    ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
    ResourceIdentityStability, SessionOpaqueReason,
};
#[cfg(test)]
pub(super) use input::lineage::{
    StableResourceIdentityFixture, classify_stable_resource_identity_fixture,
};
#[cfg(test)]
pub(super) use input::metadata::{MetadataAuthoringBatch, MetadataAuthoringEntry};
#[cfg(test)]
pub(super) use input::spatial::{
    CameraAuthoring, CameraLocalId, LightAuthoring, LightLocalId, SpatialAuthoringBatch,
};
#[cfg(test)]
pub(super) use input::{NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent};
pub(super) use input::{Presence, SceneAuthoringBatch};
#[cfg(test)]
pub(super) use plan::animation::AnimationAuthoringSemanticError;
pub(super) use plan::animation::{
    AnimationAuthoringFailure, AnimationTargetCatalog, ValidatedAnimationBatch,
    validate_animation_batch,
};
#[cfg(test)]
pub(super) use plan::appearance::AppearanceAuthoringSemanticError;
pub(super) use plan::appearance::{
    AppearanceAuthoringFailure, ValidatedAppearanceBatch, validate_appearance_batch,
};
#[cfg(test)]
pub(super) use plan::deformation::DeformationAuthoringSemanticError;
pub(super) use plan::deformation::{
    DeformationAuthoringFailure, DeformationMorphPayloadKind, ValidatedDeformationBatch,
    validate_deformation_batch,
};
#[cfg(test)]
pub(super) use plan::geometry::GeometryAuthoringSemanticError;
pub(super) use plan::geometry::{
    GeometryAttributePayloadKind, GeometryAttributeScalarKind, GeometryAuthoringFailure,
    ValidatedGeometryBatch, validate_geometry_batch,
};
pub(super) use plan::profile::{
    CanonicalProfileOwner, ProfileAuthoringFailure, ProfileOwnerCatalog, ProfileOwnerCatalogError,
    ValidatedProfileBatch, validate_profile_batch,
};
#[cfg(test)]
pub(super) use plan::profile::{ProfileAuthoringLimit, ProfileAuthoringSemanticError};
#[cfg(test)]
pub(super) use plan::spatial::SpatialAuthoringSemanticError;
pub(super) use plan::spatial::{
    SpatialAuthoringFailure, ValidatedSpatialBatch, validate_spatial_batch,
};
pub(super) use plan::{NodeAuthoringFailure, ValidatedNodeBatch, validate_node_batch};
#[cfg(test)]
pub(super) use plan::{NodeAuthoringLimit, NodeAuthoringSemanticError};
pub(super) use scene::{
    SceneAuthoringAggregateError, SceneInputEnvelope, SceneInputEnvelopeError,
    animation_limits_after_deformation, appearance_limits_after_nodes,
    deformation_limits_after_geometry, geometry_limits_after_appearance, node_limits_after_spatial,
    profile_limits_after_animation, validate_scene_aggregate,
};
