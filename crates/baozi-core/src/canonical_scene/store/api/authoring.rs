//! Authority-free canonical Scene authoring vocabulary.

pub use super::super::production::input::animation::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationTargetAuthoring,
};
pub use super::super::production::input::appearance::{
    AppearanceAuthoringBatch, ImageAuthoring, ImageLocalId, MaterialAuthoring,
    MaterialBindingAuthoring, MaterialLocalId, SamplerAuthoring, SamplerLocalId, TextureAuthoring,
    TextureLocalId,
};
pub use super::super::production::input::deformation::{
    DeformationAuthoringBatch, InfluenceAuthoring, InfluenceRowsAuthoring, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MorphDeltaAuthoring, MorphDeltaPayloadAuthoring,
    MorphTargetDefinitionAuthoring, MorphTargetIdentity, PrimitiveDeformationAuthoring,
    PrimitiveMorphTargetAuthoring, SkinAuthoring, SkinLocalId,
};
pub use super::super::production::input::geometry::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, LogicalMeshLocalId,
};
pub use super::super::production::input::lineage::{
    LineageAuthoringBatch, ObservedResourceAuthoring, OwnerProvenanceAuthoring, ProfileOwner,
    ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
    ResourceIdentityLocalId, SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId,
};
pub use super::super::production::input::metadata::{
    MetadataAuthoringBatch, MetadataAuthoringEntry,
};
pub use super::super::production::input::spatial::{
    CameraAuthoring, CameraLocalId, LightAuthoring, LightAuthoringError, LightLocalId,
    SpatialAuthoringBatch,
};
pub use super::super::production::input::{
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent, Presence,
    ProfileAuthoringBatch, ResourceSnapshotAuthoring, SceneAuthoringBatch,
};
pub use crate::FormatId;
pub use crate::animation::{AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain};
pub use crate::appearance::{
    AlphaMode, AppearanceBuildError, ColorInterpretation, LegacyMaterialFactors, MaterialFactors,
    PbrMetallicRoughnessFactors, PbrSpecularGlossinessFactors, ShadingModel,
    SupplementalPbrFactors, TextureBlendOperation, TextureChannel, TextureChannelSwizzle,
    TextureCoordinateSource, TextureRole, TextureSourceKey, TextureStrength, TextureTransform,
};
pub use crate::attribute::{AttributeDomain, AttributeSemantic, CustomAttributeSemantic};
pub use crate::deform::{MorphDeltaSemantic, SkinMatrixSpaces};
pub use crate::entity::{
    AuthoredTransform, CameraProjection, EntityName, LightKind, MAX_ENTITY_NAME_BYTES,
};
pub use crate::extension::{ExtensionMediaType, ExtensionSchemaId, ExtensionValueRef};
pub use crate::image::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};
pub use crate::math::{Color, Mat4, Quat, Vec2, Vec3};
pub use crate::namespaced::NamespacedName;
pub use crate::provenance::{
    ContentDigest, DIGEST_LEN, DigestAlgorithm, EvidenceDigest, MAX_RESOURCE_IDENTITY_LEN,
    OperationEvidenceSummary, OperationKind, OptionsDigest, ResourceIdentity,
    ResourceIdentityError, SourceDisplayName, SourceLocatorValue, SourceObjectKind,
    SourceObjectLocator,
};
pub use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpace, TangentBasisConvention,
    TangentHandednessEncoding, UvOrigin,
};
