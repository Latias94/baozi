#![forbid(unsafe_code)]

//! Core Baozi scene, math, material, diagnostic, and error types.

pub mod diagnostic;
pub mod error;
pub mod material;
pub mod math;
pub mod scene;
pub mod validation;

pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, SourceLocation};
pub use error::{BaoziError, BaoziErrorKind, Result};
pub use material::{
    AlphaMode, ColorSpace, Material, MaterialProperty, MaterialPropertyMap, ShadingModel, Texture,
    TextureFilterMode, TextureRole, TextureSampler, TextureSlot, TextureSource, TextureTransform,
    TextureWrapMode,
};
pub use math::{Aabb, Color, Mat4, Quat, Vec2, Vec3, Vec4};
pub use scene::{
    Animation, AnimationChannel, AnimationId, AnimationInterpolation, AnimationProperty,
    AnimationTarget, AnimationValues, Camera, CameraId, CameraProjection, Light, LightId,
    LightKind, MaterialId, Mesh, MeshId, MetadataMap, MetadataValue, MorphTarget, Node, NodeId,
    PrimitiveTopology, Scene, SceneBuilder, SceneSpace, Skin, SkinId, TextureId, VertexAttribute,
    VertexAttributeData, VertexAttributeSemantic,
};
pub use validation::validate_scene;
