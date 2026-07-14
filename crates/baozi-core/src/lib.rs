#![forbid(unsafe_code)]

//! Canonical Baozi scene, math, material, diagnostic, and domain error types.

mod animation;
mod appearance;
mod attribute;
mod canonical_scene;
mod canonical_validation;
mod deform;
mod derived;
mod derived_input;
pub mod diagnostic;
mod entity;
pub mod error;
mod extension;
mod format_id;
mod geometry;
mod identity;
mod image;
pub mod material;
pub mod math;
mod namespaced;
mod ordered_map;
mod provenance;
mod remap;
pub mod scene;
mod scene_space;
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
pub use ordered_map::{
    OrderedMap, OrderedMapCapacityPlan, OrderedMapError, OrderedMapIter, PreparedOrderedMapCapacity,
};
pub use scene::{
    Animation, AnimationChannel, AnimationId, AnimationInterpolation, AnimationProperty,
    AnimationTarget, AnimationValues, Camera, CameraId, CameraProjection, Light, LightId,
    LightKind, MaterialId, Mesh, MeshBinding, MeshId, MetadataMap, MetadataValue, MorphTarget,
    Node, NodeId, PreparedNodeInsert, PreparedNodeMeshBindingInsert, PreparedSceneInsert,
    PreparedSceneMetadataInsert, PrimitiveTopology, SceneBuilder, SceneCapacityPlan,
    SceneCollection, SceneDraft, SceneEdit, SceneGenerationId, SceneMetadataCapacityPlan,
    SceneNodeCapacityPlan, SceneSpace, Skin, SkinId, TextureId, ValidatedScene, VertexAttribute,
    VertexAttributeData, VertexAttributeSemantic, validate_scene, validate_scene_with,
};
pub use scene_storage::{SceneBuildValue, SceneStorageFootprint, SceneValueFootprint};
pub use validation_control::{
    SCENE_VALIDATION_POLL_INTERVAL, SceneCanonicalCounts, SceneFootprint, SceneReservationKind,
    SceneValidationControl, SceneValidationFailure, SceneValidationLimits, SceneValidationStats,
    SceneValidationSuccess, SceneValidationWork,
};
