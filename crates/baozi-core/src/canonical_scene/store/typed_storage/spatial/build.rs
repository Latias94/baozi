use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::ValidatedSpatialBatch;
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, capacity, count_work,
    finish, push,
};

use super::*;

/// One inline space action and one action per camera/light row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct SpatialStoragePlan {
    cameras: CapacityPlan,
    lights: CapacityPlan,
}

impl SpatialStoragePlan {
    pub(in crate::canonical_scene::store) fn for_storage(
        source: &SpatialStorage,
    ) -> Result<Self, StorePlanError> {
        Self::for_shape(
            u32::try_from(source.cameras.len()).map_err(|_| StorePlanError::SizeOverflow)?,
            u32::try_from(source.lights.len()).map_err(|_| StorePlanError::SizeOverflow)?,
        )
    }

    pub(in crate::canonical_scene::store) fn new<R>(
        batch: &ValidatedSpatialBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        Self::for_shape(batch.camera_count(), batch.light_count())
    }

    pub(in crate::canonical_scene::store) fn for_shape(
        cameras: u32,
        lights: u32,
    ) -> Result<Self, StorePlanError> {
        Ok(Self {
            cameras: capacity::<CameraProjection>(u64::from(cameras))?,
            lights: capacity::<LightDefinition>(u64::from(lights))?,
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

    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        count_work(self.capacities())?
            .checked_add(1)
            .ok_or(StorePlanError::SizeOverflow)
    }

    pub(in crate::canonical_scene::store) fn validation_work(self) -> Result<u64, StorePlanError> {
        self.work()
    }

    /// Initialized storage header plus typed rows.
    /// Allocator spare capacity and retained-owner infrastructure are admitted separately.
    pub(in crate::canonical_scene::store) fn canonical_bytes(self) -> Result<u64, StorePlanError> {
        [
            (
                self.cameras.desired_capacity(),
                std::mem::size_of::<CameraProjection>(),
            ),
            (
                self.lights.desired_capacity(),
                std::mem::size_of::<LightDefinition>(),
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
        let mut cameras = allocate(self.cameras)?;
        let mut lights = allocate(self.lights)?;
        let space = control
            .run_unit(|| source.clone())
            .map_err(TypedStorageBuildError::Control)?;
        for camera in batch.cameras() {
            push(&mut cameras, self.cameras, *camera, &mut control)?;
        }
        for &light in batch.lights() {
            push(&mut lights, self.lights, light, &mut control)?;
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
