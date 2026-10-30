//! Actual spatial values, independent of publication-local record slots.

use crate::entity::{CameraProjection, LightKind};
use crate::math::Vec3;
use crate::scene_space::SceneSpace;

pub(in crate::canonical_scene::store) mod build;
#[cfg(test)]
mod tests;
mod validation;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct StoredLight {
    pub(in crate::canonical_scene::store) kind: LightKind,
    pub(in crate::canonical_scene::store) tint: Vec3,
    pub(in crate::canonical_scene::store) intensity: f32,
}

/// SceneSpace and its bounded extension identities are entirely inline.
pub(in crate::canonical_scene::store) struct SpatialStorage {
    pub(in crate::canonical_scene::store) space: SceneSpace,
    pub(in crate::canonical_scene::store) cameras: Vec<CameraProjection>,
    pub(in crate::canonical_scene::store) lights: Vec<StoredLight>,
}

pub(in crate::canonical_scene::store) fn schema_name_bytes(space: &SceneSpace) -> usize {
    space
        .tangent_basis
        .schema_id()
        .map_or(0, |schema| schema.name().as_str().len())
        + space
            .tangent_handedness_encoding
            .schema_id()
            .map_or(0, |schema| schema.name().as_str().len())
}

fn schema_name_iter(space: &SceneSpace) -> impl Iterator<Item = u8> + '_ {
    space
        .tangent_basis
        .schema_id()
        .into_iter()
        .chain(space.tangent_handedness_encoding.schema_id())
        .flat_map(|schema| schema.name().as_str().bytes())
}
