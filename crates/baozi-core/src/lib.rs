#![forbid(unsafe_code)]

//! Canonical Baozi scene, math, diagnostic, and domain types.

#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod animation;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod appearance;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod attribute;
#[allow(
    dead_code,
    reason = "sealed canonical storage is reached through feature-scoped crate adapters"
)]
mod canonical_scene;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod deform;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod derived;
pub mod diagnostic;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod entity;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
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
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod image;
pub mod math;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod namespaced;
mod ordered_map;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod provenance;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod remap;
#[allow(
    dead_code,
    reason = "canonical domain implementation is exposed through sealed views"
)]
mod scene_space;
#[allow(
    dead_code,
    reason = "canonical domain values still own retained nested payloads"
)]
mod shared_storage;
mod validation_control;

#[cfg(not(feature = "_u4-import"))]
/// The temporary U4 import bridge is absent from ordinary `baozi-core` builds.
///
/// ```compile_fail,E0432
/// use baozi_core::__canonical_import::SceneDraft;
/// ```
mod canonical_import_visibility_contract {}

/// Temporary cross-crate integration surface for the atomic U4 import cutover.
#[cfg(feature = "_u4-import")]
#[doc(hidden)]
pub mod __canonical_import {
    pub use crate::canonical_scene::store::api::import::*;
}

#[cfg(not(feature = "_u7-geometry"))]
/// The temporary U7 geometry seam is absent from ordinary `baozi-core` builds.
///
/// ```compile_fail,E0432
/// use baozi_core::__canonical_geometry::SceneGeometryCursor;
/// ```
mod canonical_geometry_visibility_contract {}

/// Temporary cross-crate semantic-read surface for the U7 post-process cutover.
#[cfg(feature = "_u7-geometry")]
#[doc(hidden)]
pub mod __canonical_geometry {
    pub use crate::canonical_scene::store::api::geometry::*;
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
