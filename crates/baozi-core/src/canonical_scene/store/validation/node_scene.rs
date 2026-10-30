use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationMember, CanonicalValidationSession, OutcomeEvidenceValidationMember,
};
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, StoreWorkError};

mod appearance;
mod profile;

use super::super::construction::node_scene::SceneEvidenceCounts;
use super::super::construction::node_scene::{
    NodeSceneCentralValidationPlan, NodeSceneMaterializedRefs, NodeSceneMeteredValidationScratch,
    NodeSceneValidationScratchPlan,
};
use super::super::schema::appearance::{
    AppearanceBindingSemanticScalarSchema, AppearanceBindingTableSchema, AppearanceDirectorySchema,
    AppearanceEncodedImagePayloadSchema, AppearanceImageTableSchema,
    AppearanceMaterialFactorScalarSchema, AppearanceMaterialNameSchema,
    AppearanceMaterialTableSchema, AppearanceMediaTypeSchema, AppearanceSamplerTableSchema,
    AppearanceSourceKeySchema, AppearanceTextureTableSchema,
};
use super::super::schema::{
    CanonicalBlockRef, CanonicalSchema, NODE_SCENE_EVIDENCE_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE, NodeSceneOutcomeEvidenceSchema,
    SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION, SceneDirectorySchema,
    scene_directory_field,
};
use super::super::typed_storage::SceneTables;
pub(in crate::canonical_scene::store) use appearance::{
    AppearanceBindingValidationScratch, AppearanceCanonicalCounts,
    AppearanceMaterialValidationScratch, AppearanceValidationScratch,
    AppearanceValidationScratchRequirements,
};
use appearance::{AppearanceValidatedRefs, validate_appearance_canonical_structure_metered};
use profile::validate_profile_graph_metered;
pub(in crate::canonical_scene::store) use profile::{ProfileMeteredCounts, ProfileMeteredScratch};
pub(in crate::canonical_scene::store) struct NodeSceneValidationScratch;
impl NodeSceneValidationScratch {
    pub(in crate::canonical_scene::store) fn required_camera_placement_counts(
        counts: SceneEvidenceCounts,
    ) -> Option<usize> {
        usize::try_from(counts.camera_count).ok()
    }
    pub(in crate::canonical_scene::store) fn required_light_placement_counts(
        counts: SceneEvidenceCounts,
    ) -> Option<usize> {
        usize::try_from(counts.light_count).ok()
    }
}

/// Immutable semantic authority minted only by central node-scene validation.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneValidationProof {
    directory: CanonicalBlockRef<SceneDirectorySchema>,
    appearance_directory: CanonicalBlockRef<AppearanceDirectorySchema>,
    appearance: AppearanceValidatedRefs,
    profile: ProfileMeteredCounts,
    maximum_provenance_depth: u32,
    source_node_count: usize,
    source_root_count: usize,
    source_edge_count: usize,
    central_validation_plan: NodeSceneCentralValidationPlan,
    counts: SceneEvidenceCounts,
}

impl NodeSceneValidationProof {
    pub(in crate::canonical_scene::store) const fn directory(
        &self,
    ) -> CanonicalBlockRef<SceneDirectorySchema> {
        self.directory
    }

    pub(in crate::canonical_scene::store) const fn spatial_name_bytes(&self) -> usize {
        self.central_validation_plan.spatial_name_bytes()
    }

    pub(in crate::canonical_scene::store) const fn appearance_directory(
        &self,
    ) -> CanonicalBlockRef<AppearanceDirectorySchema> {
        self.appearance_directory
    }

    pub(in crate::canonical_scene::store) const fn appearance_images(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceImageTableSchema>> {
        self.appearance.images
    }

    pub(in crate::canonical_scene::store) const fn appearance_samplers(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceSamplerTableSchema>> {
        self.appearance.samplers
    }

    pub(in crate::canonical_scene::store) const fn appearance_textures(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceTextureTableSchema>> {
        self.appearance.textures
    }

    pub(in crate::canonical_scene::store) const fn materials(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceMaterialTableSchema>> {
        self.appearance.materials
    }

    pub(in crate::canonical_scene::store) const fn appearance_bindings(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceBindingTableSchema>> {
        self.appearance.bindings
    }

    pub(in crate::canonical_scene::store) const fn appearance_image_payloads(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceEncodedImagePayloadSchema>> {
        self.appearance.image_payloads
    }

    pub(in crate::canonical_scene::store) const fn appearance_media_types(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceMediaTypeSchema>> {
        self.appearance.media_types
    }

    pub(in crate::canonical_scene::store) const fn appearance_source_keys(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceSourceKeySchema>> {
        self.appearance.source_keys
    }

    pub(in crate::canonical_scene::store) const fn appearance_binding_semantic_scalars(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceBindingSemanticScalarSchema>> {
        self.appearance.binding_semantic_scalars
    }

    pub(in crate::canonical_scene::store) const fn appearance_material_factor_scalars(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceMaterialFactorScalarSchema>> {
        self.appearance.material_factor_scalars
    }

    pub(in crate::canonical_scene::store) const fn appearance_material_names(
        &self,
    ) -> Option<CanonicalBlockRef<AppearanceMaterialNameSchema>> {
        self.appearance.material_names
    }

    pub(in crate::canonical_scene::store) const fn profile_counts(&self) -> ProfileMeteredCounts {
        self.profile
    }

    pub(in crate::canonical_scene::store) const fn profile_maximum_provenance_depth(&self) -> u32 {
        self.maximum_provenance_depth
    }

    pub(in crate::canonical_scene::store) fn profile_segment_count(&self) -> usize {
        count_as_usize(self.profile.segment_count)
    }

    pub(in crate::canonical_scene::store) const fn profile_resource_count(&self) -> u32 {
        self.profile.resource_count
    }

    pub(in crate::canonical_scene::store) const fn profile_source_count(&self) -> u32 {
        self.profile.source_count
    }

    pub(in crate::canonical_scene::store) const fn profile_provenance_count(&self) -> u32 {
        self.profile.provenance_count
    }

    pub(in crate::canonical_scene::store) const fn profile_operation_count(&self) -> u32 {
        self.profile.operation_count
    }

    pub(in crate::canonical_scene::store) const fn profile_evidence_count(&self) -> u32 {
        self.profile.evidence_count
    }

    pub(in crate::canonical_scene::store) const fn profile_parent_count(&self) -> u32 {
        self.profile.parent_count
    }

    pub(in crate::canonical_scene::store) const fn profile_wide_count(&self) -> u32 {
        self.profile.wide_count
    }

    pub(in crate::canonical_scene::store) const fn profile_lineage_text_bytes(&self) -> u32 {
        self.profile.lineage_text_bytes
    }

    pub(in crate::canonical_scene::store) const fn profile_lineage_digest_bytes(&self) -> u32 {
        self.profile.lineage_digest_bytes
    }

    pub(in crate::canonical_scene::store) const fn source_node_count(&self) -> usize {
        self.source_node_count
    }

    pub(in crate::canonical_scene::store) const fn source_root_count(&self) -> usize {
        self.source_root_count
    }

    pub(in crate::canonical_scene::store) const fn source_edge_count(&self) -> usize {
        self.source_edge_count
    }

    pub(in crate::canonical_scene::store) const fn counts(&self) -> SceneEvidenceCounts {
        self.counts
    }

    pub(in crate::canonical_scene::store) const fn validation_scratch_plan(
        &self,
    ) -> NodeSceneValidationScratchPlan {
        self.central_validation_plan.scratch_plan()
    }

    pub(in crate::canonical_scene::store) const fn central_validation_plan(
        &self,
    ) -> NodeSceneCentralValidationPlan {
        self.central_validation_plan
    }

    pub(in crate::canonical_scene::store) fn material_count(&self) -> usize {
        count_as_usize(self.counts.material_count)
    }

    pub(in crate::canonical_scene::store) fn camera_count(&self) -> usize {
        count_as_usize(self.counts.camera_count)
    }

    pub(in crate::canonical_scene::store) fn light_count(&self) -> usize {
        count_as_usize(self.counts.light_count)
    }

    pub(in crate::canonical_scene::store) fn image_count(&self) -> usize {
        count_as_usize(self.counts.image_count)
    }

    pub(in crate::canonical_scene::store) fn sampler_count(&self) -> usize {
        count_as_usize(self.counts.sampler_count)
    }

    pub(in crate::canonical_scene::store) fn texture_count(&self) -> usize {
        count_as_usize(self.counts.texture_count)
    }

    pub(in crate::canonical_scene::store) fn material_binding_count(&self) -> usize {
        count_as_usize(self.counts.material_binding_count)
    }

    pub(in crate::canonical_scene::store) fn logical_mesh_count(&self) -> usize {
        count_as_usize(self.counts.logical_mesh_count)
    }

    pub(in crate::canonical_scene::store) fn primitive_count(&self) -> usize {
        count_as_usize(self.counts.primitive_count)
    }

    pub(in crate::canonical_scene::store) fn attribute_count(&self) -> usize {
        count_as_usize(self.counts.attribute_count)
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_count(&self) -> usize {
        count_as_usize(self.counts.mesh_instance_count)
    }

    pub(in crate::canonical_scene::store) fn skin_count(&self) -> usize {
        count_as_usize(self.counts.skin_count)
    }

    pub(in crate::canonical_scene::store) fn logical_morph_count(&self) -> usize {
        count_as_usize(self.counts.logical_morph_count)
    }

    pub(in crate::canonical_scene::store) fn morph_definition_count(&self) -> usize {
        count_as_usize(self.counts.morph_definition_count)
    }

    pub(in crate::canonical_scene::store) fn primitive_deformation_count(&self) -> usize {
        count_as_usize(self.counts.primitive_deformation_count)
    }

    pub(in crate::canonical_scene::store) fn morph_target_count(&self) -> usize {
        count_as_usize(self.counts.morph_target_count)
    }

    pub(in crate::canonical_scene::store) fn morph_delta_count(&self) -> usize {
        count_as_usize(self.counts.morph_delta_count)
    }

    pub(in crate::canonical_scene::store) fn animation_clip_count(&self) -> usize {
        count_as_usize(self.counts.animation_clip_count)
    }

    pub(in crate::canonical_scene::store) fn animation_channel_count(&self) -> usize {
        count_as_usize(self.counts.animation_channel_count)
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_layout_fault_for_test(
        &mut self,
        source_node_count: usize,
        source_root_count: usize,
    ) -> bool {
        let Some(source_edge_count) = source_node_count.checked_sub(source_root_count) else {
            return false;
        };
        self.source_node_count = source_node_count;
        self.source_root_count = source_root_count;
        self.source_edge_count = source_edge_count;
        true
    }
}

struct NodeMeteredStructure<'session> {
    directory: CanonicalValidationMember<'session, RecordBlock, SceneDirectorySchema>,
    appearance_directory:
        CanonicalValidationMember<'session, RecordBlock, AppearanceDirectorySchema>,
    source_node_count: usize,
    source_root_count: usize,
    source_edge_count: usize,
}

fn validate_node_canonical_structure_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: CanonicalValidationMember<'session, RecordBlock, SceneDirectorySchema>,
    counts: SceneEvidenceCounts,
    tables: &SceneTables,
    scratch: &mut NodeSceneMeteredValidationScratch<'_>,
) -> Result<Option<NodeMeteredStructure<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if scratch.camera_placement_counts.len() != counts.camera_count as usize
        || scratch.light_placement_counts.len() != counts.light_count as usize
    {
        return Ok(None);
    }
    let mut appearance = None;
    let mut root_count = None;
    let complete = session.visit_canonical_records(
        directory,
        1,
        SCENE_DIRECTORY_FIELD_COUNT,
        |_index, row| {
            let revision =
                row.scalar(scene_directory_field::SCHEMA_REVISION, |value: u32| value)?;
            let node_count = row.scalar(scene_directory_field::NODE_COUNT, |value: u32| value)?;
            root_count =
                row.scalar(scene_directory_field::SOURCE_ROOT_COUNT, |value: u32| value)?;
            appearance = row.record_ref::<AppearanceDirectorySchema, _>(
                scene_directory_field::APPEARANCE,
                |value| value,
            )?;
            let mut valid =
                revision == Some(SCENE_SCHEMA_REVISION) && node_count == Some(counts.node_count);
            for (field, count) in [
                (
                    scene_directory_field::LOGICAL_MESH_COUNT,
                    counts.logical_mesh_count,
                ),
                (
                    scene_directory_field::PRIMITIVE_COUNT,
                    counts.primitive_count,
                ),
                (
                    scene_directory_field::ATTRIBUTE_COUNT,
                    counts.attribute_count,
                ),
                (
                    scene_directory_field::MESH_INSTANCE_COUNT,
                    counts.mesh_instance_count,
                ),
            ] {
                valid &= row.scalar(field, |value: u32| value)? == Some(count);
            }
            Ok(valid)
        },
    )?;
    let (Some(root_count), Some(appearance_directory)) = (root_count, appearance) else {
        return Ok(None);
    };
    if !complete {
        return Ok(None);
    }
    let nodes = tables.nodes.as_ref();
    if !nodes.validate(
        session,
        counts.node_count as usize,
        root_count as usize,
        nodes.names.len(),
        scratch.camera_placement_counts,
        scratch.light_placement_counts,
    )? {
        return Ok(None);
    }
    let Some(source_node_count) = (counts.node_count as usize).checked_sub(1) else {
        return Ok(None);
    };
    let Some(source_edge_count) = source_node_count.checked_sub(root_count as usize) else {
        return Ok(None);
    };
    Ok(Some(NodeMeteredStructure {
        directory,
        appearance_directory,
        source_node_count,
        source_root_count: root_count as usize,
        source_edge_count,
    }))
}

fn validate_node_scene_success_evidence_metered<
    'session,
    'storage,
    'control,
    'operation,
    O,
    PP,
    U,
>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    evidence: OutcomeEvidenceValidationMember<
        'session,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
    counts: SceneEvidenceCounts,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let expected = [
        SCENE_SCHEMA_REVISION,
        NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
        counts.canonical_block_count,
        counts.node_count,
        counts.camera_count,
        counts.light_count,
        counts.image_count,
        counts.sampler_count,
        counts.texture_count,
        counts.material_count,
        counts.material_binding_count,
        counts.logical_mesh_count,
        counts.primitive_count,
        counts.attribute_count,
        counts.mesh_instance_count,
        counts.skin_count,
        counts.logical_morph_count,
        counts.morph_definition_count,
        counts.primitive_deformation_count,
        counts.morph_target_count,
        counts.morph_delta_count,
        counts.animation_clip_count,
        counts.animation_channel_count,
        counts.payload_block_count,
        counts.profile_segment_count,
        counts.profile_resource_count,
        counts.profile_source_count,
        counts.profile_provenance_count,
        counts.profile_operation_count,
        counts.profile_parent_count,
        counts.profile_owner_count,
        counts.profile_attachment_count,
        counts.profile_metadata_entry_count,
        counts.shareable_leaf_block_count,
    ];
    session.visit_outcome_evidence_records(
        evidence,
        1,
        NODE_SCENE_EVIDENCE_FIELD_COUNT,
        |_row_index, row| {
            for (field, expected) in expected.into_iter().enumerate() {
                let Some(actual) = row.scalar(field, |value: u32| value)? else {
                    return Ok(false);
                };
                if actual != expected {
                    return Ok(false);
                }
            }
            Ok(true)
        },
    )
}

fn optional_member_identifies<Kind, Tag>(
    member: Option<CanonicalValidationMember<'_, Kind, Tag>>,
    slot: Option<CanonicalBlockRef<Tag>>,
) -> bool
where
    Tag: CanonicalSchema<StoreKind = Kind>,
{
    match (member, slot) {
        (Some(member), Some(slot)) => member.identifies(slot.into_store()),
        (None, None) => true,
        _ => false,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the fresh canonical proof binds the complete typed graph, exact sealed counts, and independently retained slots"
)]
pub(in crate::canonical_scene::store) fn validate_node_scene_canonical_graph_metered<
    'session,
    'storage,
    'control,
    'operation,
    O,
    PP,
    U,
>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: CanonicalValidationMember<'session, RecordBlock, SceneDirectorySchema>,
    evidence: OutcomeEvidenceValidationMember<
        'session,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
    expected_counts: SceneEvidenceCounts,
    expected_refs: NodeSceneMaterializedRefs,
    tables: &SceneTables,
    validation_plan: NodeSceneCentralValidationPlan,
    scratch: &mut NodeSceneMeteredValidationScratch<'_>,
) -> Result<Option<NodeSceneValidationProof>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if !tables.spatial.as_ref().validate(
        session,
        expected_counts.camera_count as usize,
        expected_counts.light_count as usize,
        validation_plan.spatial_name_bytes(),
    )? {
        return Ok(None);
    }
    let Some(node) = validate_node_canonical_structure_metered(
        session,
        directory,
        expected_counts,
        tables,
        scratch,
    )?
    else {
        return Ok(None);
    };
    let appearance_counts = AppearanceCanonicalCounts::new(
        expected_counts.image_count,
        expected_counts.sampler_count,
        expected_counts.texture_count,
        expected_counts.material_count,
        expected_counts.material_binding_count,
    );
    let Some(appearance) = validate_appearance_canonical_structure_metered(
        session,
        node.appearance_directory,
        appearance_counts,
        &mut scratch.appearance,
    )?
    else {
        return Ok(None);
    };

    let scratch_plan = validation_plan.scratch_plan();
    let geometry = tables.geometry.as_ref();
    if !geometry.validate(
        session,
        [
            expected_counts.logical_mesh_count as usize,
            expected_counts.primitive_count as usize,
            expected_counts.attribute_count as usize,
            expected_counts.mesh_instance_count as usize,
        ],
        expected_counts.node_count as usize,
        expected_counts.material_count as usize,
    )? {
        return Ok(None);
    }
    let deformation_counts = scratch_plan.deformation_counts();
    let Some(deformation) = tables.deformation.as_ref().validate(
        session,
        deformation_counts,
        tables.nodes.as_ref(),
        geometry,
        &mut scratch.deformation,
    )?
    else {
        return Ok(None);
    };
    if deformation.maximum_uniqueness_count
        > scratch_plan.deformation_requirements().uniqueness_marks
    {
        return Ok(None);
    }

    let animation_counts = scratch_plan.animation_counts();
    if !tables.animation.as_ref().validate(
        session,
        animation_counts,
        tables.nodes.as_ref(),
        geometry,
        tables.deformation.as_ref(),
        &mut scratch.animation,
    )? {
        return Ok(None);
    }

    let profile_counts = scratch_plan.profile_counts();
    let Some(profile_owners) = expected_counts.profile_owner_layout() else {
        return Ok(None);
    };
    let Some(profile) = validate_profile_graph_metered(
        session,
        &tables.profile,
        profile_counts,
        profile_owners,
        scratch_plan.profile_maximum_depth(),
        &mut scratch.profile,
    )?
    else {
        return Ok(None);
    };
    if !validate_node_scene_success_evidence_metered(session, evidence, expected_counts)? {
        return Ok(None);
    }

    for (actual, expected) in [
        (deformation_counts.skin_count, expected_counts.skin_count),
        (
            deformation_counts.logical_morph_count,
            expected_counts.logical_morph_count,
        ),
        (
            deformation_counts.morph_definition_count,
            expected_counts.morph_definition_count,
        ),
        (
            deformation_counts.primitive_count,
            expected_counts.primitive_deformation_count,
        ),
        (
            deformation_counts.morph_target_count,
            expected_counts.morph_target_count,
        ),
        (
            deformation_counts.morph_delta_count,
            expected_counts.morph_delta_count,
        ),
        (
            deformation_counts.mesh_instance_count,
            expected_counts.mesh_instance_count,
        ),
        (
            animation_counts.clip_count,
            expected_counts.animation_clip_count,
        ),
        (
            animation_counts.channel_count,
            expected_counts.animation_channel_count,
        ),
        (
            profile_counts.segment_count,
            expected_counts.profile_segment_count,
        ),
        (
            profile_counts.resource_count,
            expected_counts.profile_resource_count,
        ),
        (
            profile_counts.source_count,
            expected_counts.profile_source_count,
        ),
        (
            profile_counts.provenance_count,
            expected_counts.profile_provenance_count,
        ),
        (
            profile_counts.operation_count,
            expected_counts.profile_operation_count,
        ),
        (
            profile_counts.parent_count,
            expected_counts.profile_parent_count,
        ),
        (
            profile_counts.owner_count,
            expected_counts.profile_owner_count,
        ),
        (
            profile_counts.attachment_count,
            expected_counts.profile_attachment_count,
        ),
        (
            profile_counts.metadata_entry_count,
            expected_counts.profile_metadata_entry_count,
        ),
    ] {
        if actual != expected {
            return Ok(None);
        }
    }
    let appearance_payload_count = [
        appearance.image_payloads.is_some(),
        appearance.media_types.is_some(),
        appearance.source_keys.is_some(),
        appearance.binding_semantic_scalars.is_some(),
        appearance.material_factor_scalars.is_some(),
        appearance.material_names.is_some(),
    ]
    .into_iter()
    .map(u32::from)
    .sum::<u32>();
    if appearance_payload_count != expected_counts.payload_block_count
        || appearance_payload_count != expected_counts.shareable_leaf_block_count
    {
        return Ok(None);
    }
    // Only the Scene root and appearance directory retain concrete record-store roots.
    let canonical_block_count = 2_u32.checked_add(
        [
            appearance.images.is_some(),
            appearance.samplers.is_some(),
            appearance.textures.is_some(),
            appearance.materials.is_some(),
            appearance.bindings.is_some(),
            appearance.image_payloads.is_some(),
            appearance.media_types.is_some(),
            appearance.source_keys.is_some(),
            appearance.binding_semantic_scalars.is_some(),
            appearance.material_factor_scalars.is_some(),
            appearance.material_names.is_some(),
        ]
        .into_iter()
        .map(u32::from)
        .sum::<u32>(),
    );
    if canonical_block_count != Some(expected_counts.canonical_block_count) {
        return Ok(None);
    }

    let identities_match = node
        .directory
        .identifies(expected_refs.directory.into_store())
        && optional_member_identifies(appearance.images, expected_refs.appearance.tables.images)
        && optional_member_identifies(
            appearance.samplers,
            expected_refs.appearance.tables.samplers,
        )
        && optional_member_identifies(
            appearance.textures,
            expected_refs.appearance.tables.textures,
        )
        && optional_member_identifies(
            appearance.materials,
            expected_refs.appearance.tables.materials,
        )
        && optional_member_identifies(
            appearance.bindings,
            expected_refs.appearance.tables.bindings,
        )
        && optional_member_identifies(
            appearance.image_payloads,
            expected_refs.appearance.payloads.image_payloads,
        )
        && optional_member_identifies(
            appearance.media_types,
            expected_refs.appearance.payloads.media_types,
        )
        && optional_member_identifies(
            appearance.source_keys,
            expected_refs.appearance.payloads.source_keys,
        )
        && optional_member_identifies(
            appearance.binding_semantic_scalars,
            expected_refs.appearance.payloads.binding_semantic_scalars,
        )
        && optional_member_identifies(
            appearance.material_factor_scalars,
            expected_refs.appearance.payloads.material_factor_scalars,
        )
        && optional_member_identifies(
            appearance.material_names,
            expected_refs.appearance.payloads.material_names,
        );
    if !identities_match {
        return Ok(None);
    }
    Ok(Some(NodeSceneValidationProof {
        directory: expected_refs.directory,
        appearance_directory: expected_refs.appearance.directory,
        appearance: AppearanceValidatedRefs {
            images: expected_refs.appearance.tables.images,
            samplers: expected_refs.appearance.tables.samplers,
            textures: expected_refs.appearance.tables.textures,
            materials: expected_refs.appearance.tables.materials,
            bindings: expected_refs.appearance.tables.bindings,
            image_payloads: expected_refs.appearance.payloads.image_payloads,
            media_types: expected_refs.appearance.payloads.media_types,
            source_keys: expected_refs.appearance.payloads.source_keys,
            binding_semantic_scalars: expected_refs.appearance.payloads.binding_semantic_scalars,
            material_factor_scalars: expected_refs.appearance.payloads.material_factor_scalars,
            material_names: expected_refs.appearance.payloads.material_names,
        },
        profile: profile_counts,
        maximum_provenance_depth: profile.maximum_provenance_depth,
        source_node_count: node.source_node_count,
        source_root_count: node.source_root_count,
        source_edge_count: node.source_edge_count,
        central_validation_plan: validation_plan,
        counts: expected_counts,
    }))
}

fn count_as_usize(count: u32) -> usize {
    count as usize
}
