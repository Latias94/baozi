use crate::entity::{CameraProjection, LightDefinition};
use crate::scene_space::SceneSpace;

/// Dense camera identity within one source-shaped authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CameraLocalId(u32);

impl CameraLocalId {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

impl From<u32> for CameraLocalId {
    fn from(index: u32) -> Self {
        Self::new(index)
    }
}

/// Dense light identity within one source-shaped authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LightLocalId(u32);

impl LightLocalId {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

impl From<u32> for LightLocalId {
    fn from(index: u32) -> Self {
        Self::new(index)
    }
}

/// Borrowed Scene-space, camera, and light input supplied by a format producer.
///
/// Camera and light identities are their dense positions in the corresponding
/// slices. Placement remains exclusively node-owned.
#[derive(Debug, Clone, Copy)]
pub struct SpatialAuthoringBatch<'a> {
    scene_space: &'a SceneSpace,
    cameras: &'a [CameraProjection],
    lights: &'a [LightDefinition],
}

impl<'a> SpatialAuthoringBatch<'a> {
    pub const fn new(
        scene_space: &'a SceneSpace,
        cameras: &'a [CameraProjection],
        lights: &'a [LightDefinition],
    ) -> Self {
        Self {
            scene_space,
            cameras,
            lights,
        }
    }

    pub(in crate::canonical_scene::store) const fn scene_space(self) -> &'a SceneSpace {
        self.scene_space
    }

    pub(in crate::canonical_scene::store) const fn cameras(self) -> &'a [CameraProjection] {
        self.cameras
    }

    pub(in crate::canonical_scene::store) const fn lights(self) -> &'a [LightDefinition] {
        self.lights
    }
}
