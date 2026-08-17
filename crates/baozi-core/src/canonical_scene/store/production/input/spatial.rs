use crate::entity::{CameraProjection, LightEmission, LightEmissionError, LightKind};
use crate::math::Color;
use crate::scene_space::SceneSpace;
use thiserror::Error;

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

/// One scene-owned camera before retained materialization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraAuthoring {
    projection: CameraProjection,
}

impl CameraAuthoring {
    pub const fn new(projection: CameraProjection) -> Self {
        Self { projection }
    }

    pub(in crate::canonical_scene::store) const fn projection(self) -> CameraProjection {
        self.projection
    }
}

/// One scene-owned light before retained materialization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightAuthoring {
    kind: LightKind,
    emission: LightEmission,
}

/// Why source light color and intensity could not be normalized for canonical authoring.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error(transparent)]
pub struct LightAuthoringError(#[from] LightEmissionError);

impl LightAuthoring {
    /// Converts source linear-sRGB color and scalar intensity into canonical emission.
    pub fn try_from_scaled_linear_srgb(
        kind: LightKind,
        color: Color,
        intensity: f32,
    ) -> Result<Self, LightAuthoringError> {
        let emission = LightEmission::try_from_scaled_linear_srgb(color, intensity)?;
        Ok(Self::new(kind, emission))
    }

    pub(in crate::canonical_scene::store) const fn new(
        kind: LightKind,
        emission: LightEmission,
    ) -> Self {
        Self { kind, emission }
    }

    pub(in crate::canonical_scene::store) const fn kind(self) -> LightKind {
        self.kind
    }

    pub(in crate::canonical_scene::store) const fn emission(self) -> LightEmission {
        self.emission
    }
}

/// Borrowed Scene-space, camera, and light input supplied by a format producer.
///
/// Camera and light identities are their dense positions in the corresponding
/// slices. Placement remains exclusively node-owned.
#[derive(Debug, Clone, Copy)]
pub struct SpatialAuthoringBatch<'a> {
    scene_space: &'a SceneSpace,
    cameras: &'a [CameraAuthoring],
    lights: &'a [LightAuthoring],
}

impl<'a> SpatialAuthoringBatch<'a> {
    pub const fn new(
        scene_space: &'a SceneSpace,
        cameras: &'a [CameraAuthoring],
        lights: &'a [LightAuthoring],
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

    pub(in crate::canonical_scene::store) const fn cameras(self) -> &'a [CameraAuthoring] {
        self.cameras
    }

    pub(in crate::canonical_scene::store) const fn lights(self) -> &'a [LightAuthoring] {
        self.lights
    }
}
