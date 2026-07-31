use baozi_operation::{CapacityPlan, RetainedStoreReservation, StorePlanError, StoreRuntimeError};

use crate::canonical_scene::store::validation::NodeSceneValidationScratch;

use super::super::SceneConstructionOwner;
use super::{
    NodeSceneWorkMeter, SceneEvidenceCounts, SceneStoreAssembly, SceneStoreControlCheckpoint,
    SceneStoreKeys,
};

/// Operation-accounted storage retained only across central canonical validation.
pub(in crate::canonical_scene::store) struct NodeSceneValidationScratchOwner {
    // Storage must drop before the reservation that accounts for it.
    deformation_palette_maxima: Vec<Option<u32>>,
    deformation_uniqueness_marks: Vec<u64>,
    animation_target_marks: Vec<u32>,
    _reservation: RetainedStoreReservation<SceneConstructionOwner>,
}

impl NodeSceneValidationScratchOwner {
    pub(in crate::canonical_scene::store) fn try_new<L>(
        assembly: &mut SceneStoreAssembly<L>,
        keys: SceneStoreKeys,
        counts: SceneEvidenceCounts,
        deformation_uniqueness_capacity: usize,
    ) -> Result<Self, StoreRuntimeError> {
        let palette_count = NodeSceneValidationScratch::required_deformation_palette_marks(counts)
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let animation_count = NodeSceneValidationScratch::required_animation_target_marks(counts)
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let palette_plan = scratch_plan::<Option<u32>>(palette_count)?;
        let uniqueness_plan = scratch_plan::<u64>(deformation_uniqueness_capacity)?;
        let animation_plan = scratch_plan::<u32>(animation_count)?;
        let envelope = palette_plan
            .envelope_delta_bytes()
            .ok()
            .and_then(|first| {
                uniqueness_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|second| first.checked_add(second))
            })
            .and_then(|subtotal| {
                animation_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|third| subtotal.checked_add(third))
            })
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;

        let mut checkpoint = assembly.try_control_checkpoint()?;
        let mut reservation = checkpoint.reserve(keys.phase, keys.dimension, envelope)?;
        checkpoint.validate_live_owner(&reservation)?;

        let mut deformation_palette_maxima = allocate_scratch(palette_plan)?;
        let mut deformation_uniqueness_marks = allocate_scratch(uniqueness_plan)?;
        let mut animation_target_marks = allocate_scratch(animation_plan)?;
        let actual = qualified_bytes(
            palette_plan,
            deformation_palette_maxima.len(),
            deformation_palette_maxima.capacity(),
        )?
        .checked_add(qualified_bytes(
            uniqueness_plan,
            deformation_uniqueness_marks.len(),
            deformation_uniqueness_marks.capacity(),
        )?)
        .and_then(|subtotal| {
            qualified_bytes(
                animation_plan,
                animation_target_marks.len(),
                animation_target_marks.capacity(),
            )
            .ok()
            .and_then(|third| subtotal.checked_add(third))
        })
        .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        checkpoint.resize(&mut reservation, actual)?;

        let mut work = NodeSceneWorkMeter::new();
        fill_scratch(
            &mut checkpoint,
            keys,
            &mut work,
            &mut deformation_palette_maxima,
            palette_count,
            None,
        )?;
        fill_scratch(
            &mut checkpoint,
            keys,
            &mut work,
            &mut deformation_uniqueness_marks,
            deformation_uniqueness_capacity,
            0,
        )?;
        fill_scratch(
            &mut checkpoint,
            keys,
            &mut work,
            &mut animation_target_marks,
            animation_count,
            0,
        )?;
        checkpoint.poll().map_err(StoreRuntimeError::Control)?;

        Ok(Self {
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_target_marks,
            _reservation: reservation,
        })
    }

    pub(in crate::canonical_scene::store) fn validation_view(
        &mut self,
    ) -> NodeSceneValidationScratch<'_> {
        NodeSceneValidationScratch::new(
            &mut self.deformation_palette_maxima,
            &mut self.deformation_uniqueness_marks,
            &mut self.animation_target_marks,
        )
    }
}

fn scratch_plan<T>(count: usize) -> Result<CapacityPlan, StoreRuntimeError> {
    CapacityPlan::for_type::<T>(0, 0, count)
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))
}

fn allocate_scratch<T>(plan: CapacityPlan) -> Result<Vec<T>, StoreRuntimeError> {
    plan.verify_preflight(0, 0)
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| StoreRuntimeError::BatchAllocationFailed)?;
    Ok(values)
}

fn qualified_bytes(
    plan: CapacityPlan,
    len: usize,
    capacity: usize,
) -> Result<u64, StoreRuntimeError> {
    plan.finish(len, capacity)
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))
}

fn fill_scratch<T: Copy>(
    checkpoint: &mut SceneStoreControlCheckpoint<'_>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    values: &mut Vec<T>,
    count: usize,
    value: T,
) -> Result<(), StoreRuntimeError> {
    for _ in 0..count {
        work.observe_checkpoint(checkpoint, keys, 1)?;
        values.push(value);
    }
    Ok(())
}
