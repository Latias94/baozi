use baozi_operation::__canonical_activation::{
    CanonicalReservation, CanonicalValidationCheckpoint, CanonicalValidationPreparationError,
    CanonicalValidationSession,
};
use baozi_operation::{
    CapacityPlan, OperationOwner, ProgressPhase, ProgressUnit, RetainedStoreReservation,
    StorePlanError, StoreRuntimeError, StoreWorkError,
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
    GeometryAttributeValidationScratch, GeometryCanonicalCounts,
    GeometryLogicalMeshValidationProjection, GeometryMeshInstanceValidationProjection,
    GeometryPrimitiveValidationProjection, GeometryPrimitiveValidationScratch,
    GeometryValidationScratch, GeometryValidationScratchRequirements, NodeSceneValidationScratch,
    ProfileMeteredCounts, ProfileMeteredScratch,
};

use super::super::SceneConstructionOwner;
use super::{
    NODE_SCENE_STORAGE_REJECTION_CODE, NodeSceneWorkMeter, SceneEvidenceCounts, SceneStoreAssembly,
    SceneStoreControlCheckpoint, SceneStoreKeys,
};

/// Exact validation-scratch capacity and initialization work derived from one sealed Scene plan.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneValidationScratchPlan {
    node_transform_kinds: CapacityPlan,
    camera_placement_counts: CapacityPlan,
    light_placement_counts: CapacityPlan,
    appearance_image_media_counts: CapacityPlan,
    appearance_bindings: CapacityPlan,
    appearance_materials: CapacityPlan,
    geometry_attributes: CapacityPlan,
    geometry_primitives: CapacityPlan,
    geometry_logical_meshes: CapacityPlan,
    geometry_primitive_projections: CapacityPlan,
    geometry_mesh_instances: CapacityPlan,
    deformation_skins: CapacityPlan,
    deformation_morph_definitions: CapacityPlan,
    deformation_logical_morphs: CapacityPlan,
    deformation_morph_deltas: CapacityPlan,
    deformation_morph_targets: CapacityPlan,
    deformation_primitives: CapacityPlan,
    deformation_mesh_instances: CapacityPlan,
    deformation_influence_row_offsets: CapacityPlan,
    deformation_influence_joints: CapacityPlan,
    deformation_influence_source_orders: CapacityPlan,
    deformation_palette_maxima: CapacityPlan,
    deformation_uniqueness_marks: CapacityPlan,
    animation_clip_descriptors: CapacityPlan,
    animation_channel_descriptors: CapacityPlan,
    animation_target_marks: CapacityPlan,
    profile_resources: CapacityPlan,
    profile_sources: CapacityPlan,
    profile_provenance: CapacityPlan,
    profile_operations: CapacityPlan,
    profile_evidence: CapacityPlan,
    profile_parents: CapacityPlan,
    profile_wide: CapacityPlan,
    profile_lineage_text: CapacityPlan,
    profile_lineage_digests: CapacityPlan,
    profile_provenance_depths: CapacityPlan,
    profile_owner_rows: CapacityPlan,
    profile_attachments: CapacityPlan,
    profile_metadata_rows: CapacityPlan,
    profile_metadata_text: CapacityPlan,
    profile_metadata_bytes: CapacityPlan,
    profile_metadata_bools: CapacityPlan,
    profile_metadata_i64s: CapacityPlan,
    profile_metadata_u64s: CapacityPlan,
    profile_metadata_f64s: CapacityPlan,
    profile_metadata_f32s: CapacityPlan,
    appearance_requirements: AppearanceValidationScratchRequirements,
    geometry_requirements: GeometryValidationScratchRequirements,
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
        let node_count =
            usize::try_from(counts.node_count).map_err(|_| StorePlanError::SizeOverflow)?;
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
        let geometry_requirements =
            GeometryValidationScratchRequirements::from_counts(GeometryCanonicalCounts::new(
                counts.logical_mesh_count,
                counts.primitive_count,
                counts.attribute_count,
                counts.mesh_instance_count,
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
        let node_transform_kinds = scratch_capacity_plan::<u8>(node_count)?;
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
        let geometry_attributes = scratch_capacity_plan::<
            GeometryAttributeValidationScratch<'static>,
        >(geometry_requirements.attributes())?;
        let geometry_primitives = scratch_capacity_plan::<
            GeometryPrimitiveValidationScratch<'static>,
        >(geometry_requirements.primitives())?;
        let geometry_logical_meshes = scratch_capacity_plan::<
            GeometryLogicalMeshValidationProjection,
        >(geometry_requirements.logical_meshes())?;
        let geometry_primitive_projections = scratch_capacity_plan::<
            GeometryPrimitiveValidationProjection,
        >(geometry_requirements.primitives())?;
        let geometry_mesh_instances = scratch_capacity_plan::<
            GeometryMeshInstanceValidationProjection,
        >(geometry_requirements.mesh_instances())?;
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
        let deformation_influence_row_offsets =
            scratch_capacity_plan::<u32>(deformation_requirements.influence_row_offsets)?;
        let deformation_influence_joints =
            scratch_capacity_plan::<u32>(deformation_requirements.influence_joints)?;
        let deformation_influence_source_orders =
            scratch_capacity_plan::<u32>(deformation_requirements.influence_source_orders)?;
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
        let profile_metadata_text =
            scratch_capacity_plan::<u8>(profile_count(profile_counts.metadata_text_bytes)?)?;
        let profile_metadata_bytes =
            scratch_capacity_plan::<u8>(profile_count(profile_counts.metadata_payload_bytes)?)?;
        let profile_metadata_bools =
            scratch_capacity_plan::<u8>(profile_count(profile_counts.metadata_bool_count)?)?;
        let profile_metadata_i64s =
            scratch_capacity_plan::<i64>(profile_count(profile_counts.metadata_i64_count)?)?;
        let profile_metadata_u64s =
            scratch_capacity_plan::<u64>(profile_count(profile_counts.metadata_u64_count)?)?;
        let profile_metadata_f64s =
            scratch_capacity_plan::<f64>(profile_count(profile_counts.metadata_f64_count)?)?;
        let profile_metadata_f32s =
            scratch_capacity_plan::<f32>(profile_count(profile_counts.metadata_f32_count)?)?;
        let envelope_bytes = [
            node_transform_kinds,
            camera_placement_counts,
            light_placement_counts,
            appearance_image_media_counts,
            appearance_bindings,
            appearance_materials,
            geometry_attributes,
            geometry_primitives,
            geometry_logical_meshes,
            geometry_primitive_projections,
            geometry_mesh_instances,
            deformation_skins,
            deformation_morph_definitions,
            deformation_logical_morphs,
            deformation_morph_deltas,
            deformation_morph_targets,
            deformation_primitives,
            deformation_mesh_instances,
            deformation_influence_row_offsets,
            deformation_influence_joints,
            deformation_influence_source_orders,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_clip_descriptors,
            animation_channel_descriptors,
            animation_target_marks,
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
            profile_metadata_text,
            profile_metadata_bytes,
            profile_metadata_bools,
            profile_metadata_i64s,
            profile_metadata_u64s,
            profile_metadata_f64s,
            profile_metadata_f32s,
        ]
        .into_iter()
        .try_fold(0_u64, |total, plan| {
            let bytes = plan
                .envelope_delta_bytes()
                .map_err(|_| StorePlanError::SizeOverflow)?;
            total.checked_add(bytes).ok_or(StorePlanError::SizeOverflow)
        })?;
        let base_initialization_work = [
            node_count,
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
                geometry_requirements
                    .initialization_work_units()
                    .and_then(|additional| work.checked_add(additional))
            })
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
                geometry_requirements
                    .semantic_projection_work_units()
                    .and_then(|additional| work.checked_add(additional))
            })
            .and_then(|work| {
                profile_counts
                    .semantic_work()
                    .and_then(|additional| work.checked_add(additional))
            })
            .ok_or(StorePlanError::SizeOverflow)?;

        Ok(Self {
            node_transform_kinds,
            camera_placement_counts,
            light_placement_counts,
            appearance_image_media_counts,
            appearance_bindings,
            appearance_materials,
            geometry_attributes,
            geometry_primitives,
            geometry_logical_meshes,
            geometry_primitive_projections,
            geometry_mesh_instances,
            deformation_skins,
            deformation_morph_definitions,
            deformation_logical_morphs,
            deformation_morph_deltas,
            deformation_morph_targets,
            deformation_primitives,
            deformation_mesh_instances,
            deformation_influence_row_offsets,
            deformation_influence_joints,
            deformation_influence_source_orders,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_clip_descriptors,
            animation_channel_descriptors,
            animation_target_marks,
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
            profile_metadata_text,
            profile_metadata_bytes,
            profile_metadata_bools,
            profile_metadata_i64s,
            profile_metadata_u64s,
            profile_metadata_f64s,
            profile_metadata_f32s,
            appearance_requirements,
            geometry_requirements,
            deformation_requirements,
            animation_counts,
            animation_requirements,
            profile_counts,
            envelope_bytes,
            initialization_work,
            semantic_projection_work,
        })
    }

    pub(super) const fn initialization_work_units(self) -> u64 {
        self.initialization_work
    }

    pub(super) const fn semantic_projection_work_units(self) -> u64 {
        self.semantic_projection_work
    }

    pub(in crate::canonical_scene::store) const fn appearance_requirements(
        self,
    ) -> AppearanceValidationScratchRequirements {
        self.appearance_requirements
    }

    pub(in crate::canonical_scene::store) const fn geometry_requirements(
        self,
    ) -> GeometryValidationScratchRequirements {
        self.geometry_requirements
    }

    pub(in crate::canonical_scene::store) const fn deformation_requirements(
        self,
    ) -> DeformationValidationScratchRequirements {
        self.deformation_requirements
    }

    pub(in crate::canonical_scene::store) const fn animation_counts(
        self,
    ) -> AnimationMeteredCounts {
        self.animation_counts
    }

    pub(in crate::canonical_scene::store) const fn profile_counts(self) -> ProfileMeteredCounts {
        self.profile_counts
    }
}

/// Operation-accounted storage retained only across central canonical validation.
pub(in crate::canonical_scene::store) struct NodeSceneValidationScratchOwner {
    // Storage must drop before the reservation that accounts for it.
    camera_placement_counts: Vec<u32>,
    light_placement_counts: Vec<u32>,
    deformation_palette_maxima: Vec<Option<u32>>,
    deformation_uniqueness_marks: Vec<u64>,
    animation_target_marks: Vec<u32>,
    profile_provenance_depths: Vec<u32>,
    _reservation: RetainedStoreReservation<SceneConstructionOwner>,
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
    node_transform_kinds: Vec<u8>,
    camera_placement_counts: Vec<u32>,
    light_placement_counts: Vec<u32>,
    appearance_image_media_counts: Vec<u32>,
    appearance_bindings: Vec<AppearanceBindingValidationScratch>,
    appearance_materials: Vec<AppearanceMaterialValidationScratch>,
    geometry_attributes: Vec<GeometryAttributeValidationScratch<'session>>,
    geometry_primitives: Vec<GeometryPrimitiveValidationScratch<'session>>,
    geometry_logical_meshes: Vec<GeometryLogicalMeshValidationProjection>,
    geometry_primitive_projections: Vec<GeometryPrimitiveValidationProjection>,
    geometry_mesh_instances: Vec<GeometryMeshInstanceValidationProjection>,
    deformation_skins: Vec<DeformationSkinValidationProjection>,
    deformation_morph_definitions: Vec<DeformationMorphDefinitionValidationProjection>,
    deformation_logical_morphs: Vec<DeformationLogicalMorphValidationProjection>,
    deformation_morph_deltas: Vec<DeformationMorphDeltaValidationProjection>,
    deformation_morph_targets: Vec<DeformationMorphTargetValidationProjection>,
    deformation_primitives: Vec<DeformationPrimitiveValidationProjection>,
    deformation_mesh_instances: Vec<DeformationMeshInstanceValidationProjection>,
    deformation_influence_row_offsets: Vec<u32>,
    deformation_influence_joints: Vec<u32>,
    deformation_influence_source_orders: Vec<u32>,
    deformation_palette_maxima: Vec<Option<u32>>,
    deformation_uniqueness_marks: Vec<u64>,
    animation_clip_descriptors: Vec<AnimationClipValidationDescriptor>,
    animation_channel_descriptors: Vec<AnimationChannelValidationDescriptor>,
    animation_target_marks: Vec<u32>,
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
    profile_metadata_text: Vec<u8>,
    profile_metadata_bytes: Vec<u8>,
    profile_metadata_bools: Vec<u8>,
    profile_metadata_i64s: Vec<i64>,
    profile_metadata_u64s: Vec<u64>,
    profile_metadata_f64s: Vec<f64>,
    profile_metadata_f32s: Vec<f32>,
    _reservation: CanonicalReservation<'scope, O>,
}

pub(in crate::canonical_scene::store) struct NodeSceneMeteredValidationScratch<'scratch, 'session> {
    pub(in crate::canonical_scene::store) node_transform_kinds: &'scratch mut [u8],
    pub(in crate::canonical_scene::store) camera_placement_counts: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) light_placement_counts: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) appearance: AppearanceValidationScratch<'scratch>,
    pub(in crate::canonical_scene::store) geometry: GeometryValidationScratch<'scratch, 'session>,
    pub(in crate::canonical_scene::store) deformation: DeformationValidationScratch<'scratch>,
    pub(in crate::canonical_scene::store) animation: AnimationValidationScratch<'scratch>,
    pub(in crate::canonical_scene::store) profile: ProfileMeteredScratch<'scratch>,
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
                let bytes = qualified_bytes(plan.$field, values.len(), values.capacity())
                    .map_err(validation_scratch_runtime_error)?;
                actual = actual.checked_add(bytes).ok_or_else(|| {
                    validation_scratch_runtime_error(StoreRuntimeError::Plan(
                        StorePlanError::SizeOverflow,
                    ))
                })?;
                values
            }};
        }

        let node_transform_kinds = allocate!(node_transform_kinds);
        let camera_placement_counts = allocate!(camera_placement_counts);
        let light_placement_counts = allocate!(light_placement_counts);
        let appearance_image_media_counts = allocate!(appearance_image_media_counts);
        let appearance_bindings = allocate!(appearance_bindings);
        let appearance_materials = allocate!(appearance_materials);
        let geometry_attributes = allocate!(geometry_attributes);
        let geometry_primitives = allocate!(geometry_primitives);
        let geometry_logical_meshes = allocate!(geometry_logical_meshes);
        let geometry_primitive_projections = allocate!(geometry_primitive_projections);
        let geometry_mesh_instances = allocate!(geometry_mesh_instances);
        let deformation_skins = allocate!(deformation_skins);
        let deformation_morph_definitions = allocate!(deformation_morph_definitions);
        let deformation_logical_morphs = allocate!(deformation_logical_morphs);
        let deformation_morph_deltas = allocate!(deformation_morph_deltas);
        let deformation_morph_targets = allocate!(deformation_morph_targets);
        let deformation_primitives = allocate!(deformation_primitives);
        let deformation_mesh_instances = allocate!(deformation_mesh_instances);
        let deformation_influence_row_offsets = allocate!(deformation_influence_row_offsets);
        let deformation_influence_joints = allocate!(deformation_influence_joints);
        let deformation_influence_source_orders = allocate!(deformation_influence_source_orders);
        let deformation_palette_maxima = allocate!(deformation_palette_maxima);
        let deformation_uniqueness_marks = allocate!(deformation_uniqueness_marks);
        let animation_clip_descriptors = allocate!(animation_clip_descriptors);
        let animation_channel_descriptors = allocate!(animation_channel_descriptors);
        let animation_target_marks = allocate!(animation_target_marks);
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
        let profile_metadata_text = allocate!(profile_metadata_text);
        let profile_metadata_bytes = allocate!(profile_metadata_bytes);
        let profile_metadata_bools = allocate!(profile_metadata_bools);
        let profile_metadata_i64s = allocate!(profile_metadata_i64s);
        let profile_metadata_u64s = allocate!(profile_metadata_u64s);
        let profile_metadata_f64s = allocate!(profile_metadata_f64s);
        let profile_metadata_f32s = allocate!(profile_metadata_f32s);
        checkpoint.resize(&mut reservation, actual)?;

        Ok(Self {
            node_transform_kinds,
            camera_placement_counts,
            light_placement_counts,
            appearance_image_media_counts,
            appearance_bindings,
            appearance_materials,
            geometry_attributes,
            geometry_primitives,
            geometry_logical_meshes,
            geometry_primitive_projections,
            geometry_mesh_instances,
            deformation_skins,
            deformation_morph_definitions,
            deformation_logical_morphs,
            deformation_morph_deltas,
            deformation_morph_targets,
            deformation_primitives,
            deformation_mesh_instances,
            deformation_influence_row_offsets,
            deformation_influence_joints,
            deformation_influence_source_orders,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_clip_descriptors,
            animation_channel_descriptors,
            animation_target_marks,
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
            profile_metadata_text,
            profile_metadata_bytes,
            profile_metadata_bools,
            profile_metadata_i64s,
            profile_metadata_u64s,
            profile_metadata_f64s,
            profile_metadata_f32s,
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

        initialize!(node_transform_kinds, 0);
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
            geometry_attributes,
            GeometryAttributeValidationScratch::EMPTY
        );
        initialize!(
            geometry_primitives,
            GeometryPrimitiveValidationScratch::EMPTY
        );
        initialize!(
            geometry_logical_meshes,
            GeometryLogicalMeshValidationProjection::EMPTY
        );
        initialize!(
            geometry_primitive_projections,
            GeometryPrimitiveValidationProjection::EMPTY
        );
        initialize!(
            geometry_mesh_instances,
            GeometryMeshInstanceValidationProjection::EMPTY
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
        initialize!(deformation_influence_row_offsets, 0);
        initialize!(deformation_influence_joints, 0);
        initialize!(deformation_influence_source_orders, 0);
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
        initialize!(profile_metadata_text, 0);
        initialize!(profile_metadata_bytes, 0);
        initialize!(profile_metadata_bools, 0);
        initialize!(profile_metadata_i64s, 0);
        initialize!(profile_metadata_u64s, 0);
        initialize!(profile_metadata_f64s, 0.0);
        initialize!(profile_metadata_f32s, 0.0);
        Ok(true)
    }

    pub(in crate::canonical_scene::store) fn metered_view(
        &mut self,
    ) -> NodeSceneMeteredValidationScratch<'_, 'session> {
        NodeSceneMeteredValidationScratch {
            node_transform_kinds: &mut self.node_transform_kinds,
            camera_placement_counts: &mut self.camera_placement_counts,
            light_placement_counts: &mut self.light_placement_counts,
            appearance: AppearanceValidationScratch::new(
                &mut self.appearance_image_media_counts,
                &mut self.appearance_bindings,
                &mut self.appearance_materials,
            ),
            geometry: GeometryValidationScratch::new(
                &mut self.geometry_attributes,
                &mut self.geometry_primitives,
                &mut self.geometry_logical_meshes,
                &mut self.geometry_primitive_projections,
                &mut self.geometry_mesh_instances,
            ),
            deformation: DeformationValidationScratch::new(
                &mut self.deformation_skins,
                &mut self.deformation_morph_definitions,
                &mut self.deformation_logical_morphs,
                &mut self.deformation_morph_deltas,
                &mut self.deformation_morph_targets,
                &mut self.deformation_primitives,
                &mut self.deformation_mesh_instances,
                &mut self.deformation_influence_row_offsets,
                &mut self.deformation_influence_joints,
                &mut self.deformation_influence_source_orders,
                &mut self.deformation_palette_maxima,
                &mut self.deformation_uniqueness_marks,
            ),
            animation: AnimationValidationScratch::new(
                &mut self.animation_clip_descriptors,
                &mut self.animation_channel_descriptors,
                &mut self.animation_target_marks,
            ),
            profile: ProfileMeteredScratch {
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
                metadata_text: &mut self.profile_metadata_text,
                metadata_bytes: &mut self.profile_metadata_bytes,
                metadata_bools: &mut self.profile_metadata_bools,
                metadata_i64s: &mut self.profile_metadata_i64s,
                metadata_u64s: &mut self.profile_metadata_u64s,
                metadata_f64s: &mut self.profile_metadata_f64s,
                metadata_f32s: &mut self.profile_metadata_f32s,
            },
        }
    }

    pub(in crate::canonical_scene::store) fn validation_view(
        &mut self,
    ) -> NodeSceneValidationScratch<'_> {
        NodeSceneValidationScratch::new(
            &mut self.camera_placement_counts,
            &mut self.light_placement_counts,
            &mut self.deformation_palette_maxima,
            &mut self.deformation_uniqueness_marks,
            &mut self.animation_target_marks,
            &mut self.profile_provenance_depths,
        )
    }

    pub(in crate::canonical_scene::store) fn appearance_view(
        &mut self,
    ) -> AppearanceValidationScratch<'_> {
        AppearanceValidationScratch::new(
            &mut self.appearance_image_media_counts,
            &mut self.appearance_bindings,
            &mut self.appearance_materials,
        )
    }

    pub(in crate::canonical_scene::store) fn geometry_view(
        &mut self,
    ) -> GeometryValidationScratch<'_, 'session> {
        GeometryValidationScratch::new(
            &mut self.geometry_attributes,
            &mut self.geometry_primitives,
            &mut self.geometry_logical_meshes,
            &mut self.geometry_primitive_projections,
            &mut self.geometry_mesh_instances,
        )
    }

    pub(in crate::canonical_scene::store) fn deformation_view(
        &mut self,
    ) -> DeformationValidationScratch<'_> {
        DeformationValidationScratch::new(
            &mut self.deformation_skins,
            &mut self.deformation_morph_definitions,
            &mut self.deformation_logical_morphs,
            &mut self.deformation_morph_deltas,
            &mut self.deformation_morph_targets,
            &mut self.deformation_primitives,
            &mut self.deformation_mesh_instances,
            &mut self.deformation_influence_row_offsets,
            &mut self.deformation_influence_joints,
            &mut self.deformation_influence_source_orders,
            &mut self.deformation_palette_maxima,
            &mut self.deformation_uniqueness_marks,
        )
    }

    pub(in crate::canonical_scene::store) const fn appearance_requirements(
        plan: NodeSceneValidationScratchPlan,
    ) -> AppearanceValidationScratchRequirements {
        plan.appearance_requirements
    }

    pub(in crate::canonical_scene::store) const fn geometry_requirements(
        plan: NodeSceneValidationScratchPlan,
    ) -> GeometryValidationScratchRequirements {
        plan.geometry_requirements
    }

    pub(in crate::canonical_scene::store) const fn deformation_requirements(
        plan: NodeSceneValidationScratchPlan,
    ) -> DeformationValidationScratchRequirements {
        plan.deformation_requirements
    }

    pub(in crate::canonical_scene::store) const fn animation_requirements(
        plan: NodeSceneValidationScratchPlan,
    ) -> AnimationValidationScratchRequirements {
        plan.animation_requirements
    }

    pub(in crate::canonical_scene::store) const fn animation_counts(
        plan: NodeSceneValidationScratchPlan,
    ) -> AnimationMeteredCounts {
        plan.animation_counts
    }

    pub(in crate::canonical_scene::store) fn animation_view(
        &mut self,
    ) -> AnimationValidationScratch<'_> {
        AnimationValidationScratch::new(
            &mut self.animation_clip_descriptors,
            &mut self.animation_channel_descriptors,
            &mut self.animation_target_marks,
        )
    }

    pub(in crate::canonical_scene::store) const fn profile_counts(
        plan: NodeSceneValidationScratchPlan,
    ) -> ProfileMeteredCounts {
        plan.profile_counts
    }

    pub(in crate::canonical_scene::store) fn profile_view(&mut self) -> ProfileMeteredScratch<'_> {
        ProfileMeteredScratch {
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
            metadata_text: &mut self.profile_metadata_text,
            metadata_bytes: &mut self.profile_metadata_bytes,
            metadata_bools: &mut self.profile_metadata_bools,
            metadata_i64s: &mut self.profile_metadata_i64s,
            metadata_u64s: &mut self.profile_metadata_u64s,
            metadata_f64s: &mut self.profile_metadata_f64s,
            metadata_f32s: &mut self.profile_metadata_f32s,
        }
    }

    pub(in crate::canonical_scene::store) fn animation_target_marks(&mut self) -> &mut [u32] {
        &mut self.animation_target_marks
    }

    pub(in crate::canonical_scene::store) fn node_transform_kinds(&mut self) -> &mut [u8] {
        &mut self.node_transform_kinds
    }

    pub(in crate::canonical_scene::store) fn profile_provenance_depths(&mut self) -> &mut [u32] {
        &mut self.profile_provenance_depths
    }
}

impl NodeSceneValidationScratchOwner {
    pub(in crate::canonical_scene::store) fn try_new<L>(
        assembly: &mut SceneStoreAssembly<L>,
        keys: SceneStoreKeys,
        counts: SceneEvidenceCounts,
        deformation_uniqueness_capacity: usize,
    ) -> Result<Self, StoreRuntimeError> {
        let camera_count = NodeSceneValidationScratch::required_camera_placement_counts(counts)
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let light_count = NodeSceneValidationScratch::required_light_placement_counts(counts)
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let palette_count = NodeSceneValidationScratch::required_deformation_palette_marks(counts)
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let animation_count = NodeSceneValidationScratch::required_animation_target_marks(counts)
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let profile_provenance_count = usize::try_from(counts.profile_provenance_count)
            .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let camera_plan = scratch_plan::<u32>(camera_count)?;
        let light_plan = scratch_plan::<u32>(light_count)?;
        let palette_plan = scratch_plan::<Option<u32>>(palette_count)?;
        let uniqueness_plan = scratch_plan::<u64>(deformation_uniqueness_capacity)?;
        let animation_plan = scratch_plan::<u32>(animation_count)?;
        let profile_provenance_plan = scratch_plan::<u32>(profile_provenance_count)?;
        let envelope = camera_plan
            .envelope_delta_bytes()
            .ok()
            .and_then(|first| {
                light_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|second| first.checked_add(second))
            })
            .and_then(|subtotal| {
                palette_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|third| subtotal.checked_add(third))
            })
            .and_then(|subtotal| {
                uniqueness_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|fourth| subtotal.checked_add(fourth))
            })
            .and_then(|subtotal| {
                animation_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|fifth| subtotal.checked_add(fifth))
            })
            .and_then(|subtotal| {
                profile_provenance_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|sixth| subtotal.checked_add(sixth))
            })
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;

        let mut checkpoint = assembly.try_control_checkpoint()?;
        let mut reservation = checkpoint.reserve(keys.phase, keys.dimension, envelope)?;
        checkpoint.validate_live_owner(&reservation)?;

        let mut camera_placement_counts = allocate_scratch(camera_plan)?;
        let mut light_placement_counts = allocate_scratch(light_plan)?;
        let mut deformation_palette_maxima = allocate_scratch(palette_plan)?;
        let mut deformation_uniqueness_marks = allocate_scratch(uniqueness_plan)?;
        let mut animation_target_marks = allocate_scratch(animation_plan)?;
        let mut profile_provenance_depths = allocate_scratch(profile_provenance_plan)?;
        let actual = qualified_bytes(
            camera_plan,
            camera_placement_counts.len(),
            camera_placement_counts.capacity(),
        )?
        .checked_add(qualified_bytes(
            light_plan,
            light_placement_counts.len(),
            light_placement_counts.capacity(),
        )?)
        .and_then(|subtotal| {
            qualified_bytes(
                palette_plan,
                deformation_palette_maxima.len(),
                deformation_palette_maxima.capacity(),
            )
            .ok()
            .and_then(|third| subtotal.checked_add(third))
        })
        .and_then(|subtotal| {
            qualified_bytes(
                uniqueness_plan,
                deformation_uniqueness_marks.len(),
                deformation_uniqueness_marks.capacity(),
            )
            .ok()
            .and_then(|fourth| subtotal.checked_add(fourth))
        })
        .and_then(|subtotal| {
            qualified_bytes(
                animation_plan,
                animation_target_marks.len(),
                animation_target_marks.capacity(),
            )
            .ok()
            .and_then(|fifth| subtotal.checked_add(fifth))
        })
        .and_then(|subtotal| {
            qualified_bytes(
                profile_provenance_plan,
                profile_provenance_depths.len(),
                profile_provenance_depths.capacity(),
            )
            .ok()
            .and_then(|sixth| subtotal.checked_add(sixth))
        })
        .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        checkpoint.resize(&mut reservation, actual)?;

        let mut work = NodeSceneWorkMeter::new();
        fill_scratch(
            &mut checkpoint,
            keys,
            &mut work,
            &mut camera_placement_counts,
            camera_count,
            0,
        )?;
        fill_scratch(
            &mut checkpoint,
            keys,
            &mut work,
            &mut light_placement_counts,
            light_count,
            0,
        )?;
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
        fill_scratch(
            &mut checkpoint,
            keys,
            &mut work,
            &mut profile_provenance_depths,
            profile_provenance_count,
            0,
        )?;
        checkpoint.poll()?;

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

    pub(in crate::canonical_scene::store) fn validation_view(
        &mut self,
    ) -> NodeSceneValidationScratch<'_> {
        NodeSceneValidationScratch::new(
            &mut self.camera_placement_counts,
            &mut self.light_placement_counts,
            &mut self.deformation_palette_maxima,
            &mut self.deformation_uniqueness_marks,
            &mut self.animation_target_marks,
            &mut self.profile_provenance_depths,
        )
    }
}

fn scratch_plan<T>(count: usize) -> Result<CapacityPlan, StoreRuntimeError> {
    CapacityPlan::for_type::<T>(0, 0, count)
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))
}

fn scratch_capacity_plan<T>(count: usize) -> Result<CapacityPlan, StorePlanError> {
    CapacityPlan::for_type::<T>(0, 0, count).map_err(|_| StorePlanError::SizeOverflow)
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

fn initialize_scratch_values<O, PP, U, T>(
    session: &mut CanonicalValidationSession<'_, '_, '_, '_, O, PP, U>,
    values: &mut Vec<T>,
    plan: CapacityPlan,
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
