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

/// Internal carrier for the extension-tier Scene authoring vocabulary owned by `baozi-import`.
///
/// This module is public only because Rust has no workspace-private visibility. It is not a
/// canonical-model compatibility surface; importer authors should use `baozi_import::authoring`.
#[doc(hidden)]
pub mod __canonical_authoring {
    pub use crate::canonical_scene::store::api::authoring::*;
}

#[cfg(not(feature = "_u4-import"))]
/// The temporary U4 import bridge is absent from ordinary `baozi-core` builds.
///
/// ```compile_fail,E0432
/// use baozi_core::__canonical_import::SceneDraft;
/// ```
mod canonical_import_visibility_contract {}

/// Temporary cross-crate integration surface for the atomic U4 import cutover.
///
/// Authoring values do not belong to this transaction bridge.
///
/// ```compile_fail,E0432
/// use baozi_core::__canonical_import::SceneAuthoringBatch;
/// ```
#[cfg(feature = "_u4-import")]
#[doc(hidden)]
pub mod __canonical_import {
    pub use crate::canonical_scene::store::api::import::*;
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
