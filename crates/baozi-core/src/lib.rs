#![forbid(unsafe_code)]

//! Canonical Baozi scene, math, material, diagnostic, and domain error types.

#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod animation;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod appearance;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod attribute;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod canonical_edit;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod canonical_scene;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod canonical_validation;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod deform;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod derived;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod derived_input;
pub mod diagnostic;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod entity;
pub mod error;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod extension;
mod format_id;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod geometry;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod identity;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod image;
pub mod material;
pub mod math;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod namespaced;
mod ordered_map;
mod process_nonce;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod provenance;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod remap;
pub mod scene;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
mod scene_space;
mod scene_storage;
#[allow(dead_code, reason = "U4-private-canonical-rail: remove at U4 cutover")]
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
