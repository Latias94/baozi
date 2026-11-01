//! Actual spatial values, independent of publication-local record slots.

use crate::entity::{CameraProjection, LightDefinition};
use crate::scene_space::SceneSpace;

pub(in crate::canonical_scene::store) mod build;
#[cfg(test)]
mod tests;
mod validation;

/// Spatial declarations are inline; tangent declarations belong to geometry attributes.
pub(in crate::canonical_scene::store) struct SpatialStorage {
    pub(in crate::canonical_scene::store) space: SceneSpace,
    pub(in crate::canonical_scene::store) cameras: Vec<CameraProjection>,
    pub(in crate::canonical_scene::store) lights: Vec<LightDefinition>,
}
