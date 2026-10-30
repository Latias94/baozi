use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::ValidatedSpatialBatch;
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, capacity, count_work,
    finish, push,
};

use super::*;

/// One inline space action, one action per row, and one string action per inline name byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct SpatialStoragePlan {
    cameras: CapacityPlan,
    lights: CapacityPlan,
    name_bytes: usize,
}

impl SpatialStoragePlan {
    pub(in crate::canonical_scene::store) fn new<R>(
        batch: &ValidatedSpatialBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        Self::for_shape(
            batch.camera_count(),
            batch.light_count(),
            u64::from(batch.tangent_basis_schema_name_bytes())
                .checked_add(u64::from(batch.tangent_encoding_schema_name_bytes()))
                .ok_or(StorePlanError::SizeOverflow)?,
        )
    }

    pub(in crate::canonical_scene::store) fn for_shape(
        cameras: u32,
        lights: u32,
        name_bytes: u64,
    ) -> Result<Self, StorePlanError> {
        Ok(Self {
            cameras: capacity::<CameraProjection>(u64::from(cameras))?,
            lights: capacity::<StoredLight>(u64::from(lights))?,
            name_bytes: usize::try_from(name_bytes).map_err(|_| StorePlanError::SizeOverflow)?,
        })
    }

    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 2] {
        [self.cameras, self.lights]
    }

    pub(in crate::canonical_scene::store) fn camera_count(self) -> u32 {
        self.cameras.desired_capacity() as u32
    }

    pub(in crate::canonical_scene::store) fn light_count(self) -> u32 {
        self.lights.desired_capacity() as u32
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(self) -> usize {
        self.name_bytes
    }

    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        count_work(self.capacities())?
            .checked_add(u64::try_from(self.name_bytes).map_err(|_| StorePlanError::SizeOverflow)?)
            .and_then(|work| work.checked_add(1))
            .ok_or(StorePlanError::SizeOverflow)
    }

    pub(in crate::canonical_scene::store) fn validation_work(self) -> Result<u64, StorePlanError> {
        self.work()
    }

    /// Initialized storage header plus typed rows; names are already inline in SceneSpace.
    /// Allocator spare capacity and retained-owner infrastructure are admitted separately.
    pub(in crate::canonical_scene::store) fn canonical_bytes(self) -> Result<u64, StorePlanError> {
        [
            (
                self.cameras.desired_capacity(),
                std::mem::size_of::<CameraProjection>(),
            ),
            (
                self.lights.desired_capacity(),
                std::mem::size_of::<StoredLight>(),
            ),
        ]
        .into_iter()
        .try_fold(
            std::mem::size_of::<SpatialStorage>() as u64,
            |sum, (count, size)| {
                let bytes = u64::try_from(count)
                    .ok()
                    .and_then(|count| count.checked_mul(size as u64))
                    .ok_or(StorePlanError::SizeOverflow)?;
                sum.checked_add(bytes).ok_or(StorePlanError::SizeOverflow)
            },
        )
    }

    /// The lane admits both Vec capacities and the inline owner before entering this builder.
    pub(in crate::canonical_scene::store) fn build<R, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedSpatialBatch<'_, R>,
        mut control: C,
    ) -> BuildResult<SpatialStorage, C::Error> {
        let source = batch.scene_space();
        if schema_name_bytes(source) != self.name_bytes {
            return Err(TypedStorageBuildError::Invariant);
        }
        let mut cameras = allocate(self.cameras)?;
        let mut lights = allocate(self.lights)?;
        let space = control
            .run_unit(|| source.clone())
            .map_err(TypedStorageBuildError::Control)?;
        for byte in schema_name_iter(source) {
            control
                .run_string_unit(|| byte)
                .map_err(TypedStorageBuildError::Control)?;
        }
        for camera in batch.cameras() {
            push(
                &mut cameras,
                self.cameras,
                camera.projection(),
                &mut control,
            )?;
        }
        for light in batch.lights() {
            let emission = light.emission();
            push(
                &mut lights,
                self.lights,
                StoredLight {
                    kind: light.kind(),
                    tint: emission.tint(),
                    intensity: emission.intensity(),
                },
                &mut control,
            )?;
        }
        finish(&cameras, self.cameras)?;
        finish(&lights, self.lights)?;
        Ok(SpatialStorage {
            space,
            cameras,
            lights,
        })
    }
}
