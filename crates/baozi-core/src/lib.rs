#![forbid(unsafe_code)]

//! Canonical Baozi scene, math, material, diagnostic, and domain error types.

mod attribute;
pub mod diagnostic;
pub mod error;
mod extension;
mod format_id;
mod identity;
pub mod material;
pub mod math;
mod namespaced;
mod ordered_map;
mod provenance;
pub mod scene;
mod scene_storage;
mod shared_storage;
mod validation;
mod validation_control;

pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, SourceLocation};
pub use error::{
    MetadataKeyViolation, MetadataValueViolation, SceneAnimationValueKind, SceneBuildError,
    SceneBuildResource, SceneCameraParameter, SceneLightParameter, SceneMeshChannel,
    SceneMetadataOwner, SceneReferenceKind, SceneStringOwner, SceneStringViolation,
    SceneValidationCode, SceneValidationData, SceneValidationError, SceneValidationIssue,
    SceneValidationLimit, SceneValidationPath,
};
pub use format_id::{FormatId, FormatIdError, MAX_FORMAT_ID_LEN};
pub use material::{
    AlphaMode, ColorSpace, Material, MaterialProperty, MaterialPropertyMap, ShadingModel, Texture,
    TextureFilterMode, TextureRole, TextureSampler, TextureSlot, TextureSource, TextureTransform,
    TextureWrapMode,
};
pub use math::{Aabb, Color, Mat4, Quat, Vec2, Vec3, Vec4};
pub use ordered_map::{OrderedMap, OrderedMapCapacityPlan, OrderedMapError, OrderedMapIter};
pub use scene::{
    Animation, AnimationChannel, AnimationId, AnimationInterpolation, AnimationProperty,
    AnimationTarget, AnimationValues, Camera, CameraId, CameraProjection, Light, LightId,
    LightKind, MaterialId, Mesh, MeshBinding, MeshId, MetadataMap, MetadataValue, MorphTarget,
    Node, NodeId, PrimitiveTopology, SceneBuilder, SceneCapacityPlan, SceneCollection, SceneDraft,
    SceneEdit, SceneGenerationId, SceneSpace, Skin, SkinId, TextureId, ValidatedScene,
    VertexAttribute, VertexAttributeData, VertexAttributeSemantic, validate_scene,
    validate_scene_with,
};
pub use scene_storage::SceneStorageFootprint;
pub use validation_control::{
    SCENE_VALIDATION_POLL_INTERVAL, SceneCanonicalCounts, SceneFootprint, SceneReservationKind,
    SceneValidationControl, SceneValidationFailure, SceneValidationLimits, SceneValidationStats,
    SceneValidationSuccess, SceneValidationWork,
};
