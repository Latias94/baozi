#![forbid(unsafe_code)]

//! Canonical Baozi scene, math, diagnostic, and domain types.

mod animation;
mod appearance;
mod attribute;
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

/// Internal Scene construction and edit integration used by the workspace adapters.
///
/// Cargo feature selection does not grant publication authority. These names are always
/// available; private fields, single-use values, central validation, and paired handoff enforce
/// the protocol. Format plugins use `baozi_import::authoring` instead of this internal namespace.
///
/// A materialized draft cannot expose or forge its canonical descriptor:
///
/// ```compile_fail,E0616
/// use baozi_core::__scene_integration::integration::SceneDraft;
/// fn extract(draft: SceneDraft) { let _ = draft.0; }
/// ```
///
/// Construction keeps its branded execution request private:
///
/// ```compile_fail,E0616
/// use baozi_core::__scene_integration::integration::SceneConstruction;
/// fn extract(construction: SceneConstruction<'_>) { let _ = construction.0; }
/// ```
///
/// Semantic validation proofs remain unforgeable despite being nameable:
///
/// ```compile_fail,E0616
/// use baozi_core::__scene_integration::integration::SceneValidationProof;
/// fn extract(proof: SceneValidationProof) { let _ = proof.0; }
/// ```
///
/// The handoff token carries actual table ownership and cannot publish twice:
///
/// ```compile_fail,E0382
/// use baozi_core::__scene_integration::integration::SceneHandoffToken;
/// fn duplicate(token: SceneHandoffToken) -> (SceneHandoffToken, SceneHandoffToken) {
///     (token, token)
/// }
/// ```
#[doc(hidden)]
pub mod __scene_integration {
    /// Authority-free values used to describe one complete canonical Scene input.
    ///
    /// Custom format importers should use the re-export at `baozi_import::authoring` so they do not
    /// depend on core transaction internals.
    pub mod authoring {
        pub use crate::canonical_scene::store::api::authoring::*;
    }

    /// Canonical Scene construction and edit transactions consumed by workspace adapters.
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
    TextureChannel, TextureChannelSwizzle, TextureRole, TextureStrength, TextureTransform,
};
pub use attribute::{
    AttributeDomain, AttributeSemantic, AttributeSemanticKind, CustomAttributeSemantic,
};
pub use canonical_scene::store::api::read::{
    AnimationChannelRef, AnimationClipRef, AnimationTarget,
};
pub use canonical_scene::store::api::read::{
    AttributeData, AttributeElement, AttributeRef, AttributeSemanticRef, CameraRef, ImageRef,
    LightRef, LogicalMeshRef, MaterialBindingRef, MaterialRef, MeshInstanceRef, NodeChildren,
    NodeRef, PrimitiveRef, SamplerRef, SceneSpaceRef, TangentBasisRef, TangentEncodingRef,
    TextureRef,
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
