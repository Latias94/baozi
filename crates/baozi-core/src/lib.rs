#![forbid(unsafe_code)]

//! Canonical Baozi scene, math, diagnostic, and domain types.

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
pub mod math;
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

pub use canonical_scene::store::api::scene::{
    SceneSnapshot, SceneSnapshotError, SceneSnapshotLimits, ValidatedScene,
};
pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, SourceLocation};
pub use format_id::{FormatId, FormatIdError, MAX_FORMAT_ID_LEN};
pub use identity::SceneGenerationId;
pub use math::{Aabb, Color, Mat4, Quat, Vec2, Vec3, Vec4};
pub use ordered_map::{
    OrderedMap, OrderedMapCapacityPlan, OrderedMapError, OrderedMapIter, PreparedOrderedMapCapacity,
};
pub use validation_control::{
    SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork,
};
