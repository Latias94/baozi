use std::num::NonZeroU64;

use baozi_operation::__retained_store_integration::activation::{
    CanonicalReservation, CanonicalValidationCheckpoint, CanonicalValidationPreparationError,
    CanonicalValidationSession,
};
use baozi_operation::{
    CapacityPlan, OperationOwner, ProgressPhase, ProgressUnit, StorePlanError, StoreRuntimeError,
    StoreWorkError,
};

use crate::canonical_scene::store::profile_provenance::PROFILE_PROVENANCE_ADDRESS_SCALARS;
use crate::canonical_scene::store::schema::profile::{
    PROFILE_EVIDENCE_SCALAR_COUNT, PROFILE_OPERATION_SCALAR_COUNT, PROFILE_PROVENANCE_SCALAR_COUNT,
    PROFILE_RESOURCE_SCALAR_COUNT, PROFILE_SOURCE_SCALAR_COUNT,
};
use crate::canonical_scene::store::validation::{
    AnimationChannelValidationDescriptor, AnimationClipValidationDescriptor,
    AnimationMeteredCounts, AnimationValidationScratch, AnimationValidationScratchRequirements,
    AppearanceBindingValidationScratch, AppearanceCanonicalCounts,
    AppearanceMaterialValidationScratch, AppearanceValidationScratch,
    AppearanceValidationScratchRequirements, DeformationLogicalMorphValidationProjection,
    DeformationMeshInstanceValidationProjection, DeformationMorphDefinitionValidationProjection,
    DeformationMorphDeltaValidationProjection, DeformationMorphTargetValidationProjection,
    DeformationPrimitiveValidationProjection, DeformationSkinValidationProjection,
    DeformationValidationScratch, DeformationValidationScratchRequirements,
    NodeSceneValidationScratch, ProfileMeteredCounts, ProfileMeteredScratch,
    ProfileSegmentValidationScratch,
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
}

impl NodeSceneCentralValidationPlan {
    pub(super) fn try_new(
        scratch: NodeSceneValidationScratchPlan,
        work: u64,
    ) -> Result<Self, StorePlanError> {
        Ok(Self {
            scratch,
            work: NonZeroU64::new(work).ok_or(StorePlanError::InvalidBlockTable)?,
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

    pub(super) fn try_rebind_node_transform(
        self,
        counts: SceneEvidenceCounts,
        profile_counts: ProfileMeteredCounts,
    ) -> Result<Self, StorePlanError> {
        let scratch = self
            .scratch
            .try_rebind_profile_edit(counts, profile_counts)?;
        let source_profile_work = self
            .scratch
            .profile_counts()
            .validation_work()
            .ok_or(StorePlanError::SizeOverflow)?;
        let output_profile_work = scratch
            .profile_counts()
            .validation_work()
            .ok_or(StorePlanError::SizeOverflow)?;
        // Transform variants live inline in typed node rows; central validation performs
        // one checked authored-transform action per node regardless of Matrix/TRS width.
        let output_work = self
            .work
            .get()
            .checked_sub(source_profile_work)
            .and_then(|work| work.checked_add(output_profile_work))
            .ok_or(StorePlanError::SizeOverflow)?;
        Self::try_new(scratch, output_work)
    }
}

/// Exact validation-scratch capacity and initialization work derived from one sealed Scene plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneValidationScratchPlan {
    camera_placement_counts: ScratchCapacityPlan,
    light_placement_counts: ScratchCapacityPlan,
    appearance_image_media_counts: ScratchCapacityPlan,
    appearance_bindings: ScratchCapacityPlan,
    appearance_materials: ScratchCapacityPlan,
    deformation_skins: ScratchCapacityPlan,
    deformation_morph_definitions: ScratchCapacityPlan,
    deformation_logical_morphs: ScratchCapacityPlan,
    deformation_morph_deltas: ScratchCapacityPlan,
    deformation_morph_targets: ScratchCapacityPlan,
    deformation_primitives: ScratchCapacityPlan,
    deformation_mesh_instances: ScratchCapacityPlan,
    deformation_palette_maxima: ScratchCapacityPlan,
    deformation_uniqueness_marks: ScratchCapacityPlan,
    animation_clip_descriptors: ScratchCapacityPlan,
    animation_channel_descriptors: ScratchCapacityPlan,
    animation_target_marks: ScratchCapacityPlan,
    profile_segments: ScratchCapacityPlan,
    profile_resources: ScratchCapacityPlan,
    profile_sources: ScratchCapacityPlan,
    profile_provenance: ScratchCapacityPlan,
    profile_operations: ScratchCapacityPlan,
    profile_evidence: ScratchCapacityPlan,
    profile_parents: ScratchCapacityPlan,
    profile_wide: ScratchCapacityPlan,
    profile_lineage_text: ScratchCapacityPlan,
    profile_lineage_digests: ScratchCapacityPlan,
    profile_provenance_depths: ScratchCapacityPlan,
    profile_owner_rows: ScratchCapacityPlan,
    profile_attachments: ScratchCapacityPlan,
    profile_metadata_rows: ScratchCapacityPlan,
    appearance_requirements: AppearanceValidationScratchRequirements,
    deformation_requirements: DeformationValidationScratchRequirements,
    animation_counts: AnimationMeteredCounts,
    animation_requirements: AnimationValidationScratchRequirements,
    profile_counts: ProfileMeteredCounts,
    envelope_bytes: u64,
    initialization_work: u64,
    semantic_projection_work: u64,
}

impl NodeSceneValidationScratchPlan {
    pub(super) fn try_new(
        counts: SceneEvidenceCounts,
        deformation_uniqueness_capacity: usize,
        deformation_influence_row_offset_count: u32,
        deformation_influence_count: u32,
        animation_counts: AnimationMeteredCounts,
        profile_counts: ProfileMeteredCounts,
    ) -> Result<Self, StorePlanError> {
        let camera_count = NodeSceneValidationScratch::required_camera_placement_counts(counts)
            .ok_or(StorePlanError::SizeOverflow)?;
        let light_count = NodeSceneValidationScratch::required_light_placement_counts(counts)
            .ok_or(StorePlanError::SizeOverflow)?;
        let palette_count = NodeSceneValidationScratch::required_deformation_palette_marks(counts)
            .ok_or(StorePlanError::SizeOverflow)?;
        let appearance_requirements =
            AppearanceValidationScratchRequirements::from_counts(AppearanceCanonicalCounts::new(
                counts.image_count,
                counts.sampler_count,
                counts.texture_count,
                counts.material_count,
                counts.material_binding_count,
            ))
            .ok_or(StorePlanError::SizeOverflow)?;
        let deformation_requirements = DeformationValidationScratchRequirements::try_new(
            counts.skin_count,
            counts.morph_definition_count,
            counts.logical_morph_count,
            counts.morph_delta_count,
            counts.morph_target_count,
            counts.primitive_deformation_count,
            counts.mesh_instance_count,
            deformation_influence_row_offset_count,
            deformation_influence_count,
        )
        .ok_or(StorePlanError::SizeOverflow)?;
        let animation_requirements = AnimationValidationScratchRequirements::try_new(
            animation_counts,
            counts.node_count,
            counts.mesh_instance_count,
        )
        .ok_or(StorePlanError::SizeOverflow)?;
        let camera_placement_counts = scratch_capacity_plan::<u32>(camera_count)?;
        let light_placement_counts = scratch_capacity_plan::<u32>(light_count)?;
        let appearance_image_media_counts =
            scratch_capacity_plan::<u32>(appearance_requirements.image_media_counts())?;
        let appearance_bindings = scratch_capacity_plan::<AppearanceBindingValidationScratch>(
            appearance_requirements.bindings(),
        )?;
        let appearance_materials = scratch_capacity_plan::<AppearanceMaterialValidationScratch>(
            appearance_requirements.materials(),
        )?;
        let deformation_skins = scratch_capacity_plan::<DeformationSkinValidationProjection>(
            deformation_requirements.skins,
        )?;
        let deformation_morph_definitions = scratch_capacity_plan::<
            DeformationMorphDefinitionValidationProjection,
        >(deformation_requirements.morph_definitions)?;
        let deformation_logical_morphs = scratch_capacity_plan::<
            DeformationLogicalMorphValidationProjection,
        >(deformation_requirements.logical_morphs)?;
        let deformation_morph_deltas = scratch_capacity_plan::<
            DeformationMorphDeltaValidationProjection,
        >(deformation_requirements.morph_deltas)?;
        let deformation_morph_targets = scratch_capacity_plan::<
            DeformationMorphTargetValidationProjection,
        >(deformation_requirements.morph_targets)?;
        let deformation_primitives = scratch_capacity_plan::<
            DeformationPrimitiveValidationProjection,
        >(deformation_requirements.primitives)?;
        let deformation_mesh_instances = scratch_capacity_plan::<
            DeformationMeshInstanceValidationProjection,
        >(deformation_requirements.mesh_instances)?;
        let deformation_palette_maxima = scratch_capacity_plan::<Option<u32>>(palette_count)?;
        let deformation_uniqueness_marks =
            scratch_capacity_plan::<u64>(deformation_uniqueness_capacity)?;
        let animation_clip_descriptors = scratch_capacity_plan::<AnimationClipValidationDescriptor>(
            animation_requirements.clip_descriptors,
        )?;
        let animation_channel_descriptors = scratch_capacity_plan::<
            AnimationChannelValidationDescriptor,
        >(animation_requirements.channel_descriptors)?;
        let animation_target_marks =
            scratch_capacity_plan::<u32>(animation_requirements.target_marks)?;
        let profile_segments = scratch_capacity_plan::<ProfileSegmentValidationScratch<'static>>(
            profile_count(profile_counts.segment_count)?,
        )?;
        let profile_resources = scratch_capacity_plan::<u32>(profile_physical_count(
            profile_counts.resource_count,
            PROFILE_RESOURCE_SCALAR_COUNT,
        )?)?;
        let profile_sources = scratch_capacity_plan::<u32>(profile_physical_count(
            profile_counts.source_count,
            PROFILE_SOURCE_SCALAR_COUNT,
        )?)?;
        let profile_provenance = scratch_capacity_plan::<u32>(profile_physical_count(
            profile_counts.provenance_count,
            PROFILE_PROVENANCE_SCALAR_COUNT,
        )?)?;
        let profile_operations = scratch_capacity_plan::<u32>(profile_physical_count(
            profile_counts.operation_count,
            PROFILE_OPERATION_SCALAR_COUNT,
        )?)?;
        let profile_evidence = scratch_capacity_plan::<u32>(profile_physical_count(
            profile_counts.evidence_count,
            PROFILE_EVIDENCE_SCALAR_COUNT,
        )?)?;
        let profile_parents = scratch_capacity_plan::<u64>(profile_physical_count(
            profile_counts.parent_count,
            PROFILE_PROVENANCE_ADDRESS_SCALARS,
        )?)?;
        let profile_wide = scratch_capacity_plan::<u64>(profile_count(profile_counts.wide_count)?)?;
        let profile_lineage_text =
            scratch_capacity_plan::<u8>(profile_count(profile_counts.lineage_text_bytes)?)?;
        let profile_lineage_digests =
            scratch_capacity_plan::<u8>(profile_count(profile_counts.lineage_digest_bytes)?)?;
        let profile_provenance_depths =
            scratch_capacity_plan::<u32>(profile_count(profile_counts.provenance_count)?)?;
        let profile_owner_rows =
            scratch_capacity_plan::<u32>(profile_physical_count(profile_counts.owner_count, 4)?)?;
        let profile_attachments = scratch_capacity_plan::<u64>(profile_physical_count(
            profile_counts.attachment_count,
            PROFILE_PROVENANCE_ADDRESS_SCALARS,
        )?)?;
        let profile_metadata_rows = scratch_capacity_plan::<u32>(profile_physical_count(
            profile_counts.metadata_entry_count,
            13,
        )?)?;
        let envelope_bytes = [
            camera_placement_counts,
            light_placement_counts,
            appearance_image_media_counts,
            appearance_bindings,
            appearance_materials,
            deformation_skins,
            deformation_morph_definitions,
            deformation_logical_morphs,
            deformation_morph_deltas,
            deformation_morph_targets,
            deformation_primitives,
            deformation_mesh_instances,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_clip_descriptors,
            animation_channel_descriptors,
            animation_target_marks,
            profile_segments,
            profile_resources,
            profile_sources,
            profile_provenance,
            profile_operations,
            profile_evidence,
            profile_parents,
            profile_wide,
            profile_lineage_text,
            profile_lineage_digests,
            profile_provenance_depths,
            profile_owner_rows,
            profile_attachments,
            profile_metadata_rows,
        ]
        .into_iter()
        .try_fold(0_u64, |total, plan| {
            total
                .checked_add(plan.envelope_bytes)
                .ok_or(StorePlanError::SizeOverflow)
        })?;
        let base_initialization_work = [
            camera_count,
            light_count,
            palette_count,
            deformation_uniqueness_capacity,
        ]
        .into_iter()
        .try_fold(0_u64, |total, count| {
            let count = u64::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
            total.checked_add(count).ok_or(StorePlanError::SizeOverflow)
        })?;
        let initialization_work = base_initialization_work
            .checked_add(
                appearance_requirements
                    .initialization_work_units()
                    .ok_or(StorePlanError::SizeOverflow)?,
            )
            .and_then(|work| {
                deformation_requirements
                    .initialization_work_units()
                    .and_then(|additional| work.checked_add(additional))
            })
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
        let semantic_projection_work = appearance_requirements
            .semantic_projection_work_units()
            .and_then(|work| {
                profile_counts
                    .semantic_work()
                    .and_then(|additional| work.checked_add(additional))
            })
            .ok_or(StorePlanError::SizeOverflow)?;

        Ok(Self {
            camera_placement_counts,
            light_placement_counts,
            appearance_image_media_counts,
            appearance_bindings,
            appearance_materials,
            deformation_skins,
            deformation_morph_definitions,
            deformation_logical_morphs,
            deformation_morph_deltas,
            deformation_morph_targets,
            deformation_primitives,
            deformation_mesh_instances,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_clip_descriptors,
            animation_channel_descriptors,
            animation_target_marks,
            profile_segments,
            profile_resources,
            profile_sources,
            profile_provenance,
            profile_operations,
            profile_evidence,
            profile_parents,
            profile_wide,
            profile_lineage_text,
            profile_lineage_digests,
            profile_provenance_depths,
            profile_owner_rows,
            profile_attachments,
            profile_metadata_rows,
            appearance_requirements,
            deformation_requirements,
            animation_counts,
            animation_requirements,
            profile_counts,
            envelope_bytes,
            initialization_work,
            semantic_projection_work,
        })
    }

    pub(super) fn try_rebind_profile_edit(
        self,
        counts: SceneEvidenceCounts,
        profile_counts: ProfileMeteredCounts,
    ) -> Result<Self, StorePlanError> {
        let influence_row_offset_count =
            u32::try_from(self.deformation_requirements.influence_row_offset_count)
                .map_err(|_| StorePlanError::SizeOverflow)?;
        let influence_count = u32::try_from(self.deformation_requirements.influence_count)
            .map_err(|_| StorePlanError::SizeOverflow)?;
        Self::try_new(
            counts,
            self.deformation_uniqueness_marks.desired_capacity(),
            influence_row_offset_count,
            influence_count,
            self.animation_counts,
            profile_counts,
        )
    }

    pub(super) const fn initialization_work_units(&self) -> u64 {
        self.initialization_work
    }

    pub(super) const fn semantic_projection_work_units(&self) -> u64 {
        self.semantic_projection_work
    }

    pub(in crate::canonical_scene::store) const fn appearance_requirements(
        &self,
    ) -> AppearanceValidationScratchRequirements {
        self.appearance_requirements
    }

    pub(in crate::canonical_scene::store) const fn deformation_requirements(
        &self,
    ) -> DeformationValidationScratchRequirements {
        self.deformation_requirements
    }

    pub(in crate::canonical_scene::store) const fn animation_counts(
        &self,
    ) -> AnimationMeteredCounts {
        self.animation_counts
    }

    pub(in crate::canonical_scene::store) const fn profile_counts(&self) -> ProfileMeteredCounts {
        self.profile_counts
    }
}

/// Scoped validation scratch whose storage is reserved before the exact validation permit starts.
pub(in crate::canonical_scene::store) struct ScopedNodeSceneValidationScratchOwner<
    'session,
    'scope,
    O,
> where
    O: OperationOwner,
{
    // Storage must drop before the reservation that accounts for it.
    camera_placement_counts: Vec<u32>,
    light_placement_counts: Vec<u32>,
    appearance_image_media_counts: Vec<u32>,
    appearance_bindings: Vec<AppearanceBindingValidationScratch>,
    appearance_materials: Vec<AppearanceMaterialValidationScratch>,
    deformation_skins: Vec<DeformationSkinValidationProjection>,
    deformation_morph_definitions: Vec<DeformationMorphDefinitionValidationProjection>,
    deformation_logical_morphs: Vec<DeformationLogicalMorphValidationProjection>,
    deformation_morph_deltas: Vec<DeformationMorphDeltaValidationProjection>,
    deformation_morph_targets: Vec<DeformationMorphTargetValidationProjection>,
    deformation_primitives: Vec<DeformationPrimitiveValidationProjection>,
    deformation_mesh_instances: Vec<DeformationMeshInstanceValidationProjection>,
    deformation_palette_maxima: Vec<Option<u32>>,
    deformation_uniqueness_marks: Vec<u64>,
    animation_clip_descriptors: Vec<AnimationClipValidationDescriptor>,
    animation_channel_descriptors: Vec<AnimationChannelValidationDescriptor>,
    animation_target_marks: Vec<u32>,
    profile_segments: Vec<ProfileSegmentValidationScratch<'session>>,
    profile_resources: Vec<u32>,
    profile_sources: Vec<u32>,
    profile_provenance: Vec<u32>,
    profile_operations: Vec<u32>,
    profile_evidence: Vec<u32>,
    profile_parents: Vec<u64>,
    profile_wide: Vec<u64>,
    profile_lineage_text: Vec<u8>,
    profile_lineage_digests: Vec<u8>,
    profile_provenance_depths: Vec<u32>,
    profile_owner_rows: Vec<u32>,
    profile_attachments: Vec<u64>,
    profile_metadata_rows: Vec<u32>,
    _reservation: CanonicalReservation<'scope, O>,
}

pub(in crate::canonical_scene::store) struct NodeSceneMeteredValidationScratch<'scratch, 'session> {
    pub(in crate::canonical_scene::store) camera_placement_counts: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) light_placement_counts: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) appearance: AppearanceValidationScratch<'scratch>,
    pub(in crate::canonical_scene::store) deformation: DeformationValidationScratch<'scratch>,
    pub(in crate::canonical_scene::store) animation: AnimationValidationScratch<'scratch>,
    pub(in crate::canonical_scene::store) profile: ProfileMeteredScratch<'scratch, 'session>,
}

impl<'session, 'scope, O> ScopedNodeSceneValidationScratchOwner<'session, 'scope, O>
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
        let appearance_image_media_counts = allocate!(appearance_image_media_counts);
        let appearance_bindings = allocate!(appearance_bindings);
        let appearance_materials = allocate!(appearance_materials);
        let deformation_skins = allocate!(deformation_skins);
        let deformation_morph_definitions = allocate!(deformation_morph_definitions);
        let deformation_logical_morphs = allocate!(deformation_logical_morphs);
        let deformation_morph_deltas = allocate!(deformation_morph_deltas);
        let deformation_morph_targets = allocate!(deformation_morph_targets);
        let deformation_primitives = allocate!(deformation_primitives);
        let deformation_mesh_instances = allocate!(deformation_mesh_instances);
        let deformation_palette_maxima = allocate!(deformation_palette_maxima);
        let deformation_uniqueness_marks = allocate!(deformation_uniqueness_marks);
        let animation_clip_descriptors = allocate!(animation_clip_descriptors);
        let animation_channel_descriptors = allocate!(animation_channel_descriptors);
        let animation_target_marks = allocate!(animation_target_marks);
        let profile_segments = allocate!(profile_segments);
        let profile_resources = allocate!(profile_resources);
        let profile_sources = allocate!(profile_sources);
        let profile_provenance = allocate!(profile_provenance);
        let profile_operations = allocate!(profile_operations);
        let profile_evidence = allocate!(profile_evidence);
        let profile_parents = allocate!(profile_parents);
        let profile_wide = allocate!(profile_wide);
        let profile_lineage_text = allocate!(profile_lineage_text);
        let profile_lineage_digests = allocate!(profile_lineage_digests);
        let profile_provenance_depths = allocate!(profile_provenance_depths);
        let profile_owner_rows = allocate!(profile_owner_rows);
        let profile_attachments = allocate!(profile_attachments);
        let profile_metadata_rows = allocate!(profile_metadata_rows);
        checkpoint.resize(&mut reservation, actual)?;

        Ok(Self {
            camera_placement_counts,
            light_placement_counts,
            appearance_image_media_counts,
            appearance_bindings,
            appearance_materials,
            deformation_skins,
            deformation_morph_definitions,
            deformation_logical_morphs,
            deformation_morph_deltas,
            deformation_morph_targets,
            deformation_primitives,
            deformation_mesh_instances,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_clip_descriptors,
            animation_channel_descriptors,
            animation_target_marks,
            profile_segments,
            profile_resources,
            profile_sources,
            profile_provenance,
            profile_operations,
            profile_evidence,
            profile_parents,
            profile_wide,
            profile_lineage_text,
            profile_lineage_digests,
            profile_provenance_depths,
            profile_owner_rows,
            profile_attachments,
            profile_metadata_rows,
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
        initialize!(appearance_image_media_counts, 0);
        initialize!(
            appearance_bindings,
            AppearanceBindingValidationScratch::EMPTY
        );
        initialize!(
            appearance_materials,
            AppearanceMaterialValidationScratch::EMPTY
        );
        initialize!(
            deformation_skins,
            DeformationSkinValidationProjection::EMPTY
        );
        initialize!(
            deformation_morph_definitions,
            DeformationMorphDefinitionValidationProjection::EMPTY
        );
        initialize!(
            deformation_logical_morphs,
            DeformationLogicalMorphValidationProjection::EMPTY
        );
        initialize!(
            deformation_morph_deltas,
            DeformationMorphDeltaValidationProjection::EMPTY
        );
        initialize!(
            deformation_morph_targets,
            DeformationMorphTargetValidationProjection::EMPTY
        );
        initialize!(
            deformation_primitives,
            DeformationPrimitiveValidationProjection::EMPTY
        );
        initialize!(
            deformation_mesh_instances,
            DeformationMeshInstanceValidationProjection::EMPTY
        );
        initialize!(deformation_palette_maxima, None);
        initialize!(deformation_uniqueness_marks, 0);
        initialize!(
            animation_clip_descriptors,
            AnimationClipValidationDescriptor::EMPTY
        );
        initialize!(
            animation_channel_descriptors,
            AnimationChannelValidationDescriptor::EMPTY
        );
        initialize!(animation_target_marks, 0);
        initialize!(profile_segments, ProfileSegmentValidationScratch::EMPTY);
        initialize!(profile_resources, 0);
        initialize!(profile_sources, 0);
        initialize!(profile_provenance, 0);
        initialize!(profile_operations, 0);
        initialize!(profile_evidence, 0);
        initialize!(profile_parents, 0);
        initialize!(profile_wide, 0);
        initialize!(profile_lineage_text, 0);
        initialize!(profile_lineage_digests, 0);
        initialize!(profile_provenance_depths, 0);
        initialize!(profile_owner_rows, 0);
        initialize!(profile_attachments, 0);
        initialize!(profile_metadata_rows, 0);
        Ok(true)
    }

    pub(in crate::canonical_scene::store) fn metered_view(
        &mut self,
    ) -> NodeSceneMeteredValidationScratch<'_, 'session> {
        NodeSceneMeteredValidationScratch {
            camera_placement_counts: &mut self.camera_placement_counts,
            light_placement_counts: &mut self.light_placement_counts,
            appearance: AppearanceValidationScratch::new(
                &mut self.appearance_image_media_counts,
                &mut self.appearance_bindings,
                &mut self.appearance_materials,
            ),
            deformation: DeformationValidationScratch::new(
                &mut self.deformation_skins,
                &mut self.deformation_morph_definitions,
                &mut self.deformation_logical_morphs,
                &mut self.deformation_morph_deltas,
                &mut self.deformation_morph_targets,
                &mut self.deformation_primitives,
                &mut self.deformation_mesh_instances,
                &mut self.deformation_palette_maxima,
                &mut self.deformation_uniqueness_marks,
            ),
            animation: AnimationValidationScratch::new(
                &mut self.animation_clip_descriptors,
                &mut self.animation_channel_descriptors,
                &mut self.animation_target_marks,
            ),
            profile: ProfileMeteredScratch {
                segments: &mut self.profile_segments,
                resources: &mut self.profile_resources,
                sources: &mut self.profile_sources,
                provenance: &mut self.profile_provenance,
                operations: &mut self.profile_operations,
                evidence: &mut self.profile_evidence,
                parents: &mut self.profile_parents,
                wide: &mut self.profile_wide,
                lineage_text: &mut self.profile_lineage_text,
                lineage_digests: &mut self.profile_lineage_digests,
                provenance_depths: &mut self.profile_provenance_depths,
                owner_rows: &mut self.profile_owner_rows,
                attachments: &mut self.profile_attachments,
                metadata_rows: &mut self.profile_metadata_rows,
            },
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

fn profile_count(count: u32) -> Result<usize, StorePlanError> {
    usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)
}

fn profile_physical_count(count: u32, width: usize) -> Result<usize, StorePlanError> {
    profile_count(count)?
        .checked_mul(width)
        .ok_or(StorePlanError::SizeOverflow)
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
