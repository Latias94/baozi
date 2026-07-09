//! Core Baozi scene, math, material, diagnostic, and error types.

pub mod diagnostic;
pub mod error;
pub mod material;
pub mod math;
pub mod scene;
pub mod validation;

pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, SourceLocation};
pub use error::{BaoziError, Result};
pub use material::{
    AlphaMode, ColorSpace, Material, ShadingModel, Texture, TextureRole, TextureSlot, TextureSource,
};
pub use math::{Aabb, Color, Mat4, Quat, Vec2, Vec3, Vec4};
pub use scene::{
    Animation, AnimationId, Camera, CameraId, Light, LightId, MaterialId, Mesh, MeshId,
    MetadataMap, MetadataValue, Node, NodeId, PrimitiveTopology, Scene, SceneBuilder, SceneSpace,
    TextureId,
};
pub use validation::validate_scene;
