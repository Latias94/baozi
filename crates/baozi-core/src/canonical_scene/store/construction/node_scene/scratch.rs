use std::num::NonZeroU64;

use baozi_operation::__retained_store_integration::activation::{
    CanonicalReservation, CanonicalValidationCheckpoint, CanonicalValidationPreparationError,
    CanonicalValidationSession,
};
use baozi_operation::{
    CapacityPlan, OperationOwner, ProgressPhase, ProgressUnit, StorePlanError, StoreRuntimeError,
    StoreWorkError,
};

use crate::canonical_scene::store::validation::{
    AnimationMeteredCounts, AnimationValidationScratch, AnimationValidationScratchRequirements,
    AppearanceMeteredCounts, DeformationMeteredCounts, DeformationValidationScratch,
    DeformationValidationScratchRequirements, NodeSceneValidationScratch, ProfileMeteredCounts,
    ProfileMeteredScratch,
};

use super::{NODE_SCENE_STORAGE_REJECTION_CODE, SceneEvidenceCounts};

/// Complete central-validation authority derived from one sealed Scene construction plan.
///
/// Scratch shape and exact work stay coupled so a derived edit can rebind the validated source
/// without rescanning canonical storage or accepting a caller-supplied work envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneCentralValidationPlan {
    scratch: NodeSceneValidationScratchPlan,
    work: NonZeroU64,
    spatial_name_bytes: usize,
}

impl NodeSceneCentralValidationPlan {
    pub(super) fn try_new(
        scratch: NodeSceneValidationScratchPlan,
        work: u64,
        spatial_name_bytes: usize,
    ) -> Result<Self, StorePlanError> {
        Ok(Self {
            scratch,
            work: NonZeroU64::new(work).ok_or(StorePlanError::InvalidBlockTable)?,
            spatial_name_bytes,
        })
    }

    pub(in crate::canonical_scene::store) const fn scratch_plan(
        &self,
    ) -> NodeSceneValidationScratchPlan {
        self.scratch
    }

    pub(in crate::canonical_scene::store) const fn work(&self) -> NonZeroU64 {
        self.work
    }

    pub(in crate::canonical_scene::store) const fn spatial_name_bytes(&self) -> usize {
        self.spatial_name_bytes
    }

    pub(super) fn try_rebind_geometry(
        self,
        source_work: u64,
        output_work: u64,
    ) -> Result<Self, StorePlanError> {
        let work = self
            .work
            .get()
            .checked_sub(source_work)
            .and_then(|work| work.checked_add(output_work))
            .ok_or(StorePlanError::SizeOverflow)?;
        Self::try_new(self.scratch, work, self.spatial_name_bytes)
    }

    pub(super) fn try_rebind_profile_edit(
        self,
        counts: SceneEvidenceCounts,
        profile_counts: ProfileMeteredCounts,
        maximum_depth: u32,
    ) -> Result<Self, StorePlanError> {
        let scratch =
            self.scratch
                .try_rebind_profile_edit(counts, profile_counts, maximum_depth)?;
        let profile_work = |counts: ProfileMeteredCounts| {
            counts
                .validation_work()?
                .checked_add(counts.scratch_initialization_work()?)
        };
        let source_profile_work =
            profile_work(self.scratch.profile_counts()).ok_or(StorePlanError::SizeOverflow)?;
        let output_profile_work =
            profile_work(scratch.profile_counts()).ok_or(StorePlanError::SizeOverflow)?;
        // Transform variants live inline in typed node rows; central validation performs
        // one checked authored-transform action per node regardless of Matrix/TRS width.
        let output_work = self
            .work
            .get()
            .checked_sub(source_profile_work)
            .and_then(|work| work.checked_add(output_profile_work))
            .ok_or(StorePlanError::SizeOverflow)?;
        Self::try_new(scratch, output_work, self.spatial_name_bytes)
    }
}

/// Exact validation-scratch capacity and initialization work derived from one sealed Scene plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneValidationScratchPlan {
    camera_placement_counts: ScratchCapacityPlan,
    light_placement_counts: ScratchCapacityPlan,
    deformation_palette_maxima: ScratchCapacityPlan,
    deformation_uniqueness_marks: ScratchCapacityPlan,
    animation_target_marks: ScratchCapacityPlan,
    profile_provenance_depths: ScratchCapacityPlan,
    appearance_counts: AppearanceMeteredCounts,
    deformation_counts: DeformationMeteredCounts,
    deformation_requirements: DeformationValidationScratchRequirements,
    animation_counts: AnimationMeteredCounts,
    profile_counts: ProfileMeteredCounts,
    profile_maximum_depth: u32,
    envelope_bytes: u64,
    initialization_work: u64,
    semantic_validation_work: u64,
}

impl NodeSceneValidationScratchPlan {
    pub(super) fn try_new(
        counts: SceneEvidenceCounts,
        appearance_counts: AppearanceMeteredCounts,
        deformation_counts: DeformationMeteredCounts,
        deformation_uniqueness_capacity: usize,
        animation_counts: AnimationMeteredCounts,
        profile_counts: ProfileMeteredCounts,
        profile_maximum_depth: u32,
    ) -> Result<Self, StorePlanError> {
        let camera_count = NodeSceneValidationScratch::required_camera_placement_counts(counts)
            .ok_or(StorePlanError::SizeOverflow)?;
        let light_count = NodeSceneValidationScratch::required_light_placement_counts(counts)
            .ok_or(StorePlanError::SizeOverflow)?;
        let deformation_requirements = DeformationValidationScratchRequirements::try_new(
            deformation_counts,
            deformation_uniqueness_capacity,
        )
        .ok_or(StorePlanError::SizeOverflow)?;
        if [
            (appearance_counts.image_count, counts.image_count),
            (appearance_counts.sampler_count, counts.sampler_count),
            (appearance_counts.texture_count, counts.texture_count),
            (
                appearance_counts.binding_count,
                counts.material_binding_count,
            ),
            (appearance_counts.material_count, counts.material_count),
            (deformation_counts.skin_count, counts.skin_count),
            (
                deformation_counts.logical_morph_count,
                counts.logical_morph_count,
            ),
            (
                deformation_counts.logical_morph_count,
                counts.logical_mesh_count,
            ),
            (
                deformation_counts.morph_definition_count,
                counts.morph_definition_count,
            ),
            (
                deformation_counts.primitive_count,
                counts.primitive_deformation_count,
            ),
            (deformation_counts.primitive_count, counts.primitive_count),
            (
                deformation_counts.morph_target_count,
                counts.morph_target_count,
            ),
            (
                deformation_counts.morph_delta_count,
                counts.morph_delta_count,
            ),
            (
                deformation_counts.mesh_instance_count,
                counts.mesh_instance_count,
            ),
            (animation_counts.clip_count, counts.animation_clip_count),
            (
                animation_counts.channel_count,
                counts.animation_channel_count,
            ),
        ]
        .into_iter()
        .any(|(actual, expected)| actual != expected)
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let animation_requirements = AnimationValidationScratchRequirements::try_new(
            animation_counts,
            usize::try_from(counts.node_count).map_err(|_| StorePlanError::SizeOverflow)?,
            usize::try_from(counts.mesh_instance_count)
                .map_err(|_| StorePlanError::SizeOverflow)?,
        )
        .ok_or(StorePlanError::SizeOverflow)?;
        let camera_placement_counts = scratch_capacity_plan::<u32>(camera_count)?;
        let light_placement_counts = scratch_capacity_plan::<u32>(light_count)?;
        let deformation_palette_maxima =
            scratch_capacity_plan::<Option<u32>>(deformation_requirements.palette_maxima)?;
        let deformation_uniqueness_marks =
            scratch_capacity_plan::<u64>(deformation_requirements.uniqueness_marks)?;
        let animation_target_marks =
            scratch_capacity_plan::<u32>(animation_requirements.target_marks)?;
        let profile_provenance_depths = scratch_capacity_plan::<u32>(
            ProfileMeteredScratch::required_provenance_depths(profile_counts.provenance_count)
                .ok_or(StorePlanError::SizeOverflow)?,
        )?;
        let envelope_bytes = [
            camera_placement_counts,
            light_placement_counts,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_target_marks,
            profile_provenance_depths,
        ]
        .into_iter()
        .try_fold(0_u64, |total, plan| {
            total
                .checked_add(plan.envelope_bytes)
                .ok_or(StorePlanError::SizeOverflow)
        })?;
        let base_initialization_work =
            [camera_count, light_count]
                .into_iter()
                .try_fold(0_u64, |total, count| {
                    let count = u64::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
                    total.checked_add(count).ok_or(StorePlanError::SizeOverflow)
                })?;
        let initialization_work = base_initialization_work
            .checked_add(
                deformation_requirements
                    .initialization_work_units()
                    .ok_or(StorePlanError::SizeOverflow)?,
            )
            .and_then(|work| {
                animation_requirements
                    .initialization_work_units()
                    .and_then(|additional| work.checked_add(additional))
            })
            .and_then(|work| {
                profile_counts
                    .scratch_initialization_work()
                    .and_then(|additional| work.checked_add(additional))
            })
            .ok_or(StorePlanError::SizeOverflow)?;
        let semantic_validation_work = appearance_counts
            .validation_work()
            .and_then(|work| {
                deformation_counts
                    .validation_work()
                    .and_then(|deformation| work.checked_add(deformation))
            })
            .and_then(|work| {
                animation_counts
                    .validation_work()
                    .and_then(|animation| work.checked_add(animation))
            })
            .ok_or(StorePlanError::SizeOverflow)?;

        Ok(Self {
            camera_placement_counts,
            light_placement_counts,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_target_marks,
            profile_provenance_depths,
            appearance_counts,
            deformation_counts,
            deformation_requirements,
            animation_counts,
            profile_counts,
            profile_maximum_depth,
            envelope_bytes,
            initialization_work,
            semantic_validation_work,
        })
    }

    pub(super) fn try_rebind_profile_edit(
        self,
        counts: SceneEvidenceCounts,
        profile_counts: ProfileMeteredCounts,
        maximum_depth: u32,
    ) -> Result<Self, StorePlanError> {
        Self::try_new(
            counts,
            self.appearance_counts,
            self.deformation_counts,
            self.deformation_uniqueness_marks.desired_capacity(),
            self.animation_counts,
            profile_counts,
            maximum_depth,
        )
    }

    pub(super) const fn initialization_work_units(&self) -> u64 {
        self.initialization_work
    }

    pub(super) const fn envelope_bytes(&self) -> u64 {
        self.envelope_bytes
    }

    /// Appearance, deformation, and animation validate their actual typed rows and pools once.
    /// Their algorithm scratch initialization is counted separately above.
    pub(super) const fn semantic_validation_work_units(&self) -> u64 {
        self.semantic_validation_work
    }

    pub(in crate::canonical_scene::store) const fn appearance_counts(
        &self,
    ) -> AppearanceMeteredCounts {
        self.appearance_counts
    }

    pub(in crate::canonical_scene::store) const fn deformation_requirements(
        &self,
    ) -> DeformationValidationScratchRequirements {
        self.deformation_requirements
    }

    pub(in crate::canonical_scene::store) const fn deformation_counts(
        &self,
    ) -> DeformationMeteredCounts {
        self.deformation_counts
    }

    pub(in crate::canonical_scene::store) const fn animation_counts(
        &self,
    ) -> AnimationMeteredCounts {
        self.animation_counts
    }

    pub(in crate::canonical_scene::store) const fn profile_counts(&self) -> ProfileMeteredCounts {
        self.profile_counts
    }

    pub(in crate::canonical_scene::store) const fn profile_maximum_depth(&self) -> u32 {
        self.profile_maximum_depth
    }
}

/// Scoped validation scratch whose storage is reserved before the exact validation permit starts.
pub(in crate::canonical_scene::store) struct ScopedNodeSceneValidationScratchOwner<'scope, O>
where
    O: OperationOwner,
{
    // Storage must drop before the reservation that accounts for it.
    camera_placement_counts: Vec<u32>,
    light_placement_counts: Vec<u32>,
    deformation_palette_maxima: Vec<Option<u32>>,
    deformation_uniqueness_marks: Vec<u64>,
    animation_target_marks: Vec<u32>,
    profile_provenance_depths: Vec<u32>,
    _reservation: CanonicalReservation<'scope, O>,
}

pub(in crate::canonical_scene::store) struct NodeSceneMeteredValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) camera_placement_counts: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) light_placement_counts: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) deformation: DeformationValidationScratch<'scratch>,
    pub(in crate::canonical_scene::store) animation: AnimationValidationScratch<'scratch>,
    pub(in crate::canonical_scene::store) profile: ProfileMeteredScratch<'scratch>,
}

impl<'scope, O> ScopedNodeSceneValidationScratchOwner<'scope, O>
where
    O: OperationOwner,
{
    pub(in crate::canonical_scene::store) fn try_allocate<PP, U>(
        checkpoint: &mut CanonicalValidationCheckpoint<'_, 'scope, '_, O, PP, U>,
        plan: NodeSceneValidationScratchPlan,
    ) -> Result<Self, CanonicalValidationPreparationError>
    where
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let mut reservation = checkpoint.reserve(plan.envelope_bytes)?;
        checkpoint.validate_live_owner(&reservation)?;

        let mut actual = 0_u64;
        macro_rules! allocate {
            ($field:ident) => {{
                let values =
                    allocate_scratch(plan.$field).map_err(validation_scratch_runtime_error)?;
                let bytes = qualified_bytes(plan.$field, &values)
                    .map_err(validation_scratch_runtime_error)?;
                actual = actual.checked_add(bytes).ok_or_else(|| {
                    validation_scratch_runtime_error(StoreRuntimeError::Plan(
                        StorePlanError::SizeOverflow,
                    ))
                })?;
                values
            }};
        }
        let camera_placement_counts = allocate!(camera_placement_counts);
        let light_placement_counts = allocate!(light_placement_counts);
        let deformation_palette_maxima = allocate!(deformation_palette_maxima);
        let deformation_uniqueness_marks = allocate!(deformation_uniqueness_marks);
        let animation_target_marks = allocate!(animation_target_marks);
        let profile_provenance_depths = allocate!(profile_provenance_depths);
        checkpoint.resize(&mut reservation, actual)?;

        Ok(Self {
            camera_placement_counts,
            light_placement_counts,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_target_marks,
            profile_provenance_depths,
            _reservation: reservation,
        })
    }

    pub(in crate::canonical_scene::store) fn initialize<PP, U>(
        &mut self,
        session: &mut CanonicalValidationSession<'_, '_, '_, '_, O, PP, U>,
        plan: NodeSceneValidationScratchPlan,
    ) -> Result<bool, StoreWorkError>
    where
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        macro_rules! initialize {
            ($field:ident, $value:expr) => {
                if !initialize_scratch_values(session, &mut self.$field, plan.$field, $value)? {
                    return Ok(false);
                }
            };
        }
        initialize!(camera_placement_counts, 0);
        initialize!(light_placement_counts, 0);
        initialize!(deformation_palette_maxima, None);
        initialize!(deformation_uniqueness_marks, 0);
        initialize!(animation_target_marks, 0);
        initialize!(profile_provenance_depths, 0);
        Ok(true)
    }

    pub(in crate::canonical_scene::store) fn metered_view(
        &mut self,
    ) -> NodeSceneMeteredValidationScratch<'_> {
        NodeSceneMeteredValidationScratch {
            camera_placement_counts: &mut self.camera_placement_counts,
            light_placement_counts: &mut self.light_placement_counts,
            deformation: DeformationValidationScratch::new(
                &mut self.deformation_palette_maxima,
                &mut self.deformation_uniqueness_marks,
            ),
            animation: AnimationValidationScratch::new(&mut self.animation_target_marks),
            profile: ProfileMeteredScratch::new(&mut self.profile_provenance_depths),
        }
    }
}

/// Scratch allocations always start empty. Keep only their count and checked byte envelope,
/// rather than copying a general growth plan through every candidate and validation proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScratchCapacityPlan {
    count: usize,
    envelope_bytes: u64,
}

impl ScratchCapacityPlan {
    const fn desired_capacity(self) -> usize {
        self.count
    }

    fn capacity_plan<T>(self) -> Result<CapacityPlan, StoreRuntimeError> {
        let plan = CapacityPlan::for_type::<T>(0, 0, self.count)
            .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        if plan.envelope_delta_bytes().ok() != Some(self.envelope_bytes) {
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        Ok(plan)
    }
}

fn scratch_capacity_plan<T>(count: usize) -> Result<ScratchCapacityPlan, StorePlanError> {
    let plan =
        CapacityPlan::for_type::<T>(0, 0, count).map_err(|_| StorePlanError::SizeOverflow)?;
    let envelope_bytes = plan
        .envelope_delta_bytes()
        .map_err(|_| StorePlanError::SizeOverflow)?;
    Ok(ScratchCapacityPlan {
        count,
        envelope_bytes,
    })
}

fn validation_scratch_runtime_error(
    cause: StoreRuntimeError,
) -> CanonicalValidationPreparationError {
    CanonicalValidationPreparationError::runtime(cause, NODE_SCENE_STORAGE_REJECTION_CODE)
}

fn allocate_scratch<T>(plan: ScratchCapacityPlan) -> Result<Vec<T>, StoreRuntimeError> {
    let plan = plan.capacity_plan::<T>()?;
    plan.verify_preflight(0, 0)
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| StoreRuntimeError::BatchAllocationFailed)?;
    Ok(values)
}

fn qualified_bytes<T>(
    plan: ScratchCapacityPlan,
    values: &Vec<T>,
) -> Result<u64, StoreRuntimeError> {
    plan.capacity_plan::<T>()?
        .finish(values.len(), values.capacity())
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))
}

fn initialize_scratch_values<O, PP, U, T>(
    session: &mut CanonicalValidationSession<'_, '_, '_, '_, O, PP, U>,
    values: &mut Vec<T>,
    plan: ScratchCapacityPlan,
    value: T,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    T: Copy,
{
    if !values.is_empty() || values.capacity() < plan.desired_capacity() {
        return Ok(false);
    }
    for _ in 0..plan.desired_capacity() {
        let completed = session.run_validation_unit(|| values.push(value))?;
        if completed.is_none() {
            return Ok(false);
        }
    }
    Ok(values.len() == plan.desired_capacity())
}
