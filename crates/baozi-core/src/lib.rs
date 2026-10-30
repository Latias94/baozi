#![forbid(unsafe_code)]

//! Canonical Baozi scene, math, diagnostic, and domain types.

mod animation;
mod appearance;
mod attribute;
#[allow(
    dead_code,
    reason = "sealed canonical storage is reached through feature-scoped crate adapters"
)]
mod canonical_scene;
mod deform;
pub mod diagnostic;
mod entity;
mod extension;
mod format_id;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod geometry;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod identity;
mod image;
pub mod local_axes;
mod math;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod namespaced;
mod ordered_map;
mod provenance;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod scene_space;
mod space_conversion;
mod validation_control;

#[cfg(not(feature = "_scene-integration"))]
/// The Scene authoring and transaction boundary is absent from ordinary `baozi-core` builds.
///
/// ```compile_fail,E0432
/// use baozi_core::scene::integration::SceneDraft;
/// ```
///
/// ```compile_fail,E0432
/// use baozi_core::scene::authoring::SceneAuthoringBatch;
/// ```
mod scene_integration_visibility_contract {}

/// Canonical Scene vocabulary consumed by the importer extension tier.
#[cfg(feature = "_scene-integration")]
#[doc(hidden)]
pub mod scene {
    /// Authority-free values used to describe one complete canonical Scene input.
    ///
    /// Custom format importers should use the re-export at `baozi_import::authoring` so they do not
    /// depend on core transaction internals.
    pub mod authoring {
        pub use crate::canonical_scene::store::api::authoring::*;
    }

    /// Canonical Scene transaction boundary consumed only by `baozi-import`.
    ///
    /// Rust has no workspace-private visibility, so this module is nameable but intentionally
    /// hidden from generated documentation and ordinary importer APIs.
    #[doc(hidden)]
    pub mod integration {
        pub use crate::canonical_scene::store::api::integration::*;
    }
}

pub use animation::{AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain};
pub use appearance::{
    AlphaCutoff, AlphaMode, ColorInterpretation, LegacyMaterialFactors, MaterialFactorError,
    MaterialFactorField, MaterialFactors, PbrMetallicRoughnessFactors,
    PbrSpecularGlossinessFactors, ShadingModel, SupplementalPbrFactors, TextureBlendOperation,
    TextureChannel, TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength,
    TextureTransform,
};
pub use attribute::{
    AttributeDomain, AttributeSemantic, AttributeSemanticKind, CustomAttributeSemantic,
};
pub use canonical_scene::store::api::read::{
    AnimationChannelRef, AnimationClipRef, AnimationTarget,
};
pub use canonical_scene::store::api::read::{
    AttributeData, AttributeRef, AttributeSemanticRef, CameraRef, ImageRef, LightRef,
    LogicalMeshRef, MaterialBindingRef, MaterialRef, MeshInstanceRef, NodeChildren, NodeRef,
    PrimitiveRef, SamplerRef, SceneSpaceRef, TangentBasisRef, TangentEncodingRef, TextureRef,
};
pub use canonical_scene::store::api::read::{
    InfluenceRef, InfluenceRowRef, InfluenceRows, MorphDeltaData, MorphDeltaRef,
    MorphTargetDefinitionRef, MorphTargetRef, SkinRef,
};
pub use canonical_scene::store::api::scene::{
    SceneSnapshot, SceneSnapshotError, SceneSnapshotLimits, ValidatedScene,
};
pub use canonical_scene::store::api::validation::{
    AppearanceBuildResource, AppearanceLimitDimension, EntityNameError, MaterialFactorGroup,
    SceneAnimationRateKind, SceneAnimationSamplePart, SceneAnimationTarget, SceneAppearanceRule,
    SceneAuthoredProfileOwner, SceneInputCollection, SceneLightEmissionRule, SceneProfileOwner,
    SceneValidationDomain, SceneValidationError, SceneValidationLimit, TextureStrengthRequirement,
};
pub use deform::{
    MorphDeltaSemantic, SkinMatrixCorrection, SkinMatrixCorrectionError, SkinMatrixSpace,
    SkinMatrixSpaces,
};
pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, SourceLocation};
pub use entity::{
    AuthoredTransform, AuthoredTransformError, AuthoredTransformKind, CameraProjection,
    CameraProjectionError, LightDefinition, LightEmissionError, LightError, LightKind,
    LightKindCategory, OrthographicProjection, PerspectiveProjection, TrsTransform,
};
pub use extension::{ExtensionSchemaId, ExtensionSchemaIdError};
pub use format_id::{FormatId, FormatIdError, MAX_FORMAT_ID_LEN};
pub use geometry::TopologyKind;
pub use identity::AnimationClipId;
pub use identity::SceneGenerationId;
pub use identity::{
    CameraId, ImageId, LightId, LogicalMeshId, MaterialId, MeshInstanceId, NodeId, PrimitiveId,
    SamplerId, SceneEntityKind, SceneLookupError, TextureId,
};
pub use identity::{MorphTargetId, SkinId};
pub use image::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};
pub use local_axes::{LocalAxes, LocalAxesError, LocalAxis};
pub use math::{Aabb, Color, Mat4, Quat, Vec2, Vec3, Vec4};
pub use namespaced::{NamespacedName, NamespacedNameError};
pub use ordered_map::{
    OrderedMap, OrderedMapCapacityPlan, OrderedMapError, OrderedMapIter, PreparedOrderedMapCapacity,
};
pub use scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpace, SceneSpaceField, SceneSpaceValidationError,
    TangentBasisConvention, TangentHandednessEncoding, UvOrigin,
};
pub use space_conversion::{
    ConventionValue, NumericConversionError, SourceSpaceError, TargetSpace,
};
pub use validation_control::{
    SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork,
};
