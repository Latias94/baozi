use baozi_operation::{
    CanonicalRecordRef, OutcomeEvidenceRecordRef, StoreEvidenceValidationSession,
    StoreValidationSession,
};

mod animation;
mod appearance;
mod deformation;
mod geometry;

use super::super::construction::node_scene::SceneEvidenceCounts;
use super::super::node_scene::{authored_transform_scalar_count, decode_authored_transform};
use super::super::production::{
    Presence, ValidatedAnimationBatch, ValidatedAppearanceBatch, ValidatedDeformationBatch,
    ValidatedGeometryBatch, ValidatedNodeBatch,
};
use super::super::schema::animation::{
    AnimationChannelTableSchema, AnimationClipTableSchema, AnimationDirectorySchema,
    AnimationInTangentsSchema, AnimationOutTangentsSchema, AnimationTimesSchema,
    AnimationValuesSchema,
};
use super::super::schema::appearance::{
    AppearanceBindingTableSchema, AppearanceDirectorySchema, AppearanceEncodedImagePayloadSchema,
    AppearanceImageTableSchema, AppearanceMaterialTableSchema, AppearanceMediaTypeSchema,
    AppearanceSamplerTableSchema, AppearanceTextureTableSchema,
};
use super::super::schema::deformation::{
    DeformationDirectorySchema, DeformationInfluenceJointSchema,
    DeformationInfluenceRowOffsetSchema, DeformationInfluenceSourceOrderSchema,
    DeformationInfluenceWeightSchema, DeformationInstanceMorphWeightSchema,
    DeformationInverseBindMatrixSchema, DeformationLogicalMorphTableSchema,
    DeformationMeshInstanceTableSchema, DeformationMorphDefaultWeightSchema,
    DeformationMorphDefinitionTableSchema, DeformationMorphDeltaTableSchema,
    DeformationMorphNameSchema, DeformationMorphSparseIndexSchema,
    DeformationMorphTargetTableSchema, DeformationMorphValueSchema,
    DeformationPrimitiveTableSchema, DeformationSkinCorrectionMatrixSchema,
    DeformationSkinJointNodeSchema, DeformationSkinNameSchema, DeformationSkinTableSchema,
};
use super::super::schema::geometry::{
    GeometryAttributeTableSchema, GeometryLogicalMeshTableSchema, GeometryMeshInstanceTableSchema,
    GeometryPrimitiveTableSchema,
};
use super::super::schema::{
    CanonicalBlockRef, NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE, NODE_SCENE_NODE_FIELD_COUNT,
    NODE_SCENE_TRANSFORM_SYNTHETIC, NodeSceneAuthoredTransformSchema,
    NodeSceneChildReferenceTableSchema, NodeSceneNodeTableSchema, NodeSceneOutcomeEvidenceSchema,
    PRESENCE_ABSENT, PRESENCE_PRESENT, SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION,
    SceneDirectorySchema, SceneGeometryDirectorySchema, node_scene_child_reference_field,
    node_scene_evidence_field, node_scene_node_field, scene_directory_field,
};
use super::{require_optional_collection, require_scalar};
use animation::{AnimationValidatedRefs, validate_animation_graph};
use appearance::{AppearanceValidatedRefs, validate_appearance_graph};
use deformation::{
    DeformationValidatedRefs, DeformationValidationDependencies, validate_deformation_graph,
};
use geometry::{GeometryValidatedRefs, validate_geometry_graph};

/// Immutable semantic authority minted only by central node-scene validation.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneValidationProof {
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    references: Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>>,
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
    appearance: AppearanceValidatedRefs,
    geometry: GeometryValidatedRefs,
    deformation: DeformationValidatedRefs,
    animation: AnimationValidatedRefs,
    source_node_count: usize,
    source_root_count: usize,
    source_edge_count: usize,
    counts: SceneEvidenceCounts,
}

impl NodeSceneValidationProof {
    pub(in crate::canonical_scene::store) const fn nodes(
        self,
    ) -> CanonicalBlockRef<NodeSceneNodeTableSchema> {
        self.nodes
    }

    pub(in crate::canonical_scene::store) const fn references(
        self,
    ) -> Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>> {
        self.references
    }

    pub(in crate::canonical_scene::store) const fn transforms(
        self,
    ) -> Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>> {
        self.transforms
    }

    pub(in crate::canonical_scene::store) const fn appearance_images(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceImageTableSchema>> {
        self.appearance.images
    }

    pub(in crate::canonical_scene::store) const fn appearance_samplers(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceSamplerTableSchema>> {
        self.appearance.samplers
    }

    pub(in crate::canonical_scene::store) const fn appearance_textures(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceTextureTableSchema>> {
        self.appearance.textures
    }

    pub(in crate::canonical_scene::store) const fn materials(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceMaterialTableSchema>> {
        self.appearance.materials
    }

    pub(in crate::canonical_scene::store) const fn appearance_bindings(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceBindingTableSchema>> {
        self.appearance.bindings
    }

    pub(in crate::canonical_scene::store) const fn appearance_image_payloads(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceEncodedImagePayloadSchema>> {
        self.appearance.image_payloads
    }

    pub(in crate::canonical_scene::store) const fn appearance_media_types(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceMediaTypeSchema>> {
        self.appearance.media_types
    }

    pub(in crate::canonical_scene::store) const fn logical_meshes(
        self,
    ) -> Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>> {
        self.geometry.logical_meshes
    }

    pub(in crate::canonical_scene::store) const fn primitives(
        self,
    ) -> Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>> {
        self.geometry.primitives
    }

    pub(in crate::canonical_scene::store) const fn attributes(
        self,
    ) -> Option<CanonicalBlockRef<GeometryAttributeTableSchema>> {
        self.geometry.attributes
    }

    pub(in crate::canonical_scene::store) const fn mesh_instances(
        self,
    ) -> Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>> {
        self.geometry.mesh_instances
    }

    pub(in crate::canonical_scene::store) const fn deformation_skins(
        self,
    ) -> Option<CanonicalBlockRef<DeformationSkinTableSchema>> {
        self.deformation.skins
    }

    pub(in crate::canonical_scene::store) const fn deformation_logical_morphs(
        self,
    ) -> Option<CanonicalBlockRef<DeformationLogicalMorphTableSchema>> {
        self.deformation.logical_morphs
    }

    pub(in crate::canonical_scene::store) const fn deformation_morph_definitions(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMorphDefinitionTableSchema>> {
        self.deformation.morph_definitions
    }

    pub(in crate::canonical_scene::store) const fn deformation_primitives(
        self,
    ) -> Option<CanonicalBlockRef<DeformationPrimitiveTableSchema>> {
        self.deformation.primitives
    }

    pub(in crate::canonical_scene::store) const fn deformation_morph_targets(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMorphTargetTableSchema>> {
        self.deformation.morph_targets
    }

    pub(in crate::canonical_scene::store) const fn deformation_morph_deltas(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMorphDeltaTableSchema>> {
        self.deformation.morph_deltas
    }

    pub(in crate::canonical_scene::store) const fn deformation_mesh_instances(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>> {
        self.deformation.mesh_instances
    }

    pub(in crate::canonical_scene::store) const fn deformation_joint_nodes(
        self,
    ) -> Option<CanonicalBlockRef<DeformationSkinJointNodeSchema>> {
        self.deformation.joint_nodes
    }

    pub(in crate::canonical_scene::store) const fn deformation_inverse_bind_matrices(
        self,
    ) -> Option<CanonicalBlockRef<DeformationInverseBindMatrixSchema>> {
        self.deformation.inverse_bind_matrices
    }

    pub(in crate::canonical_scene::store) const fn deformation_skin_correction_matrices(
        self,
    ) -> Option<CanonicalBlockRef<DeformationSkinCorrectionMatrixSchema>> {
        self.deformation.skin_correction_matrices
    }

    pub(in crate::canonical_scene::store) const fn deformation_skin_names(
        self,
    ) -> Option<CanonicalBlockRef<DeformationSkinNameSchema>> {
        self.deformation.skin_names
    }

    pub(in crate::canonical_scene::store) const fn deformation_influence_row_offsets(
        self,
    ) -> Option<CanonicalBlockRef<DeformationInfluenceRowOffsetSchema>> {
        self.deformation.influence_row_offsets
    }

    pub(in crate::canonical_scene::store) const fn deformation_influence_joints(
        self,
    ) -> Option<CanonicalBlockRef<DeformationInfluenceJointSchema>> {
        self.deformation.influence_joints
    }

    pub(in crate::canonical_scene::store) const fn deformation_influence_weights(
        self,
    ) -> Option<CanonicalBlockRef<DeformationInfluenceWeightSchema>> {
        self.deformation.influence_weights
    }

    pub(in crate::canonical_scene::store) const fn deformation_influence_source_orders(
        self,
    ) -> Option<CanonicalBlockRef<DeformationInfluenceSourceOrderSchema>> {
        self.deformation.influence_source_orders
    }

    pub(in crate::canonical_scene::store) const fn deformation_morph_defaults(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMorphDefaultWeightSchema>> {
        self.deformation.morph_defaults
    }

    pub(in crate::canonical_scene::store) const fn deformation_morph_names(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMorphNameSchema>> {
        self.deformation.morph_names
    }

    pub(in crate::canonical_scene::store) const fn deformation_morph_values(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMorphValueSchema>> {
        self.deformation.morph_values
    }

    pub(in crate::canonical_scene::store) const fn deformation_morph_sparse_indices(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMorphSparseIndexSchema>> {
        self.deformation.morph_sparse_indices
    }

    pub(in crate::canonical_scene::store) const fn deformation_instance_morph_weights(
        self,
    ) -> Option<CanonicalBlockRef<DeformationInstanceMorphWeightSchema>> {
        self.deformation.instance_morph_weights
    }

    pub(in crate::canonical_scene::store) const fn animation_clips(
        self,
    ) -> Option<CanonicalBlockRef<AnimationClipTableSchema>> {
        self.animation.clips
    }

    pub(in crate::canonical_scene::store) const fn animation_channels(
        self,
    ) -> Option<CanonicalBlockRef<AnimationChannelTableSchema>> {
        self.animation.channels
    }

    pub(in crate::canonical_scene::store) const fn animation_times(
        self,
    ) -> Option<CanonicalBlockRef<AnimationTimesSchema>> {
        self.animation.times
    }

    pub(in crate::canonical_scene::store) const fn animation_values(
        self,
    ) -> Option<CanonicalBlockRef<AnimationValuesSchema>> {
        self.animation.values
    }

    pub(in crate::canonical_scene::store) const fn animation_in_tangents(
        self,
    ) -> Option<CanonicalBlockRef<AnimationInTangentsSchema>> {
        self.animation.in_tangents
    }

    pub(in crate::canonical_scene::store) const fn animation_out_tangents(
        self,
    ) -> Option<CanonicalBlockRef<AnimationOutTangentsSchema>> {
        self.animation.out_tangents
    }

    pub(in crate::canonical_scene::store) const fn source_node_count(self) -> usize {
        self.source_node_count
    }

    pub(in crate::canonical_scene::store) const fn source_root_count(self) -> usize {
        self.source_root_count
    }

    pub(in crate::canonical_scene::store) const fn source_edge_count(self) -> usize {
        self.source_edge_count
    }

    pub(in crate::canonical_scene::store) fn material_count(self) -> usize {
        count_as_usize(self.counts.material_count)
    }

    pub(in crate::canonical_scene::store) fn image_count(self) -> usize {
        count_as_usize(self.counts.image_count)
    }

    pub(in crate::canonical_scene::store) fn sampler_count(self) -> usize {
        count_as_usize(self.counts.sampler_count)
    }

    pub(in crate::canonical_scene::store) fn texture_count(self) -> usize {
        count_as_usize(self.counts.texture_count)
    }

    pub(in crate::canonical_scene::store) fn material_binding_count(self) -> usize {
        count_as_usize(self.counts.material_binding_count)
    }

    pub(in crate::canonical_scene::store) fn logical_mesh_count(self) -> usize {
        count_as_usize(self.counts.logical_mesh_count)
    }

    pub(in crate::canonical_scene::store) fn primitive_count(self) -> usize {
        count_as_usize(self.counts.primitive_count)
    }

    pub(in crate::canonical_scene::store) fn attribute_count(self) -> usize {
        count_as_usize(self.counts.attribute_count)
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_count(self) -> usize {
        count_as_usize(self.counts.mesh_instance_count)
    }

    pub(in crate::canonical_scene::store) fn skin_count(self) -> usize {
        count_as_usize(self.counts.skin_count)
    }

    pub(in crate::canonical_scene::store) fn logical_morph_count(self) -> usize {
        count_as_usize(self.counts.logical_morph_count)
    }

    pub(in crate::canonical_scene::store) fn morph_definition_count(self) -> usize {
        count_as_usize(self.counts.morph_definition_count)
    }

    pub(in crate::canonical_scene::store) fn primitive_deformation_count(self) -> usize {
        count_as_usize(self.counts.primitive_deformation_count)
    }

    pub(in crate::canonical_scene::store) fn morph_target_count(self) -> usize {
        count_as_usize(self.counts.morph_target_count)
    }

    pub(in crate::canonical_scene::store) fn morph_delta_count(self) -> usize {
        count_as_usize(self.counts.morph_delta_count)
    }

    pub(in crate::canonical_scene::store) fn animation_clip_count(self) -> usize {
        count_as_usize(self.counts.animation_clip_count)
    }

    pub(in crate::canonical_scene::store) fn animation_channel_count(self) -> usize {
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

#[allow(clippy::too_many_arguments)]
pub(in crate::canonical_scene::store) fn validate_node_scene_graph<NR, AR, GR, DR, ANR, E>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected_appearance: &ValidatedAppearanceBatch<'_, AR>,
    expected_geometry: &ValidatedGeometryBatch<'_, GR>,
    expected_deformation: &ValidatedDeformationBatch<'_, DR>,
    expected_animation: &ValidatedAnimationBatch<'_, ANR>,
    expected_counts: SceneEvidenceCounts,
    mut observe_work: impl FnMut(u64) -> Result<(), E>,
) -> Result<Option<NodeSceneValidationProof>, E> {
    let mut work_failure = None;
    let proof = validate_node_scene_graph_inner(
        session,
        directory_ref,
        evidence_ref,
        expected_nodes,
        expected_appearance,
        expected_geometry,
        expected_deformation,
        expected_animation,
        expected_counts,
        &mut |amount| match observe_work(amount) {
            Ok(()) => Some(()),
            Err(cause) => {
                work_failure = Some(cause);
                None
            }
        },
    );
    match work_failure {
        Some(cause) => Err(cause),
        None => Ok(proof),
    }
}

pub(in crate::canonical_scene::store) fn validate_node_scene_evidence_graph(
    session: &mut StoreEvidenceValidationSession<'_, '_>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    counts: SceneEvidenceCounts,
) -> bool {
    let Some(evidence) = session.records(evidence_ref) else {
        return false;
    };
    evidence.rows() == 1
        && evidence.fields() == NODE_SCENE_EVIDENCE_FIELD_COUNT
        && evidence.scalar::<u32>(0, node_scene_evidence_field::SCHEMA_REVISION)
            == Some(SCENE_SCHEMA_REVISION)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::OUTCOME)
            == Some(NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::CANONICAL_BLOCK_COUNT)
            == Some(counts.canonical_block_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::NODE_COUNT)
            == Some(counts.node_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::IMAGE_COUNT)
            == Some(counts.image_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::SAMPLER_COUNT)
            == Some(counts.sampler_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::TEXTURE_COUNT)
            == Some(counts.texture_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::MATERIAL_COUNT)
            == Some(counts.material_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::MATERIAL_BINDING_COUNT)
            == Some(counts.material_binding_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::LOGICAL_MESH_COUNT)
            == Some(counts.logical_mesh_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PRIMITIVE_COUNT)
            == Some(counts.primitive_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::ATTRIBUTE_COUNT)
            == Some(counts.attribute_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::MESH_INSTANCE_COUNT)
            == Some(counts.mesh_instance_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::SKIN_COUNT)
            == Some(counts.skin_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::LOGICAL_MORPH_COUNT)
            == Some(counts.logical_morph_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::MORPH_DEFINITION_COUNT)
            == Some(counts.morph_definition_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PRIMITIVE_DEFORMATION_COUNT)
            == Some(counts.primitive_deformation_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::MORPH_TARGET_COUNT)
            == Some(counts.morph_target_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::MORPH_DELTA_COUNT)
            == Some(counts.morph_delta_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::ANIMATION_CLIP_COUNT)
            == Some(counts.animation_clip_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::ANIMATION_CHANNEL_COUNT)
            == Some(counts.animation_channel_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PAYLOAD_BLOCK_COUNT)
            == Some(counts.payload_block_count)
}

#[allow(clippy::too_many_arguments)]
fn validate_node_scene_graph_inner<NR, AR, GR, DR, ANR>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    expected: &ValidatedNodeBatch<'_, NR>,
    expected_appearance: &ValidatedAppearanceBatch<'_, AR>,
    expected_geometry: &ValidatedGeometryBatch<'_, GR>,
    expected_deformation: &ValidatedDeformationBatch<'_, DR>,
    expected_animation: &ValidatedAnimationBatch<'_, ANR>,
    expected_counts: SceneEvidenceCounts,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<NodeSceneValidationProof> {
    observe_work(1)?;
    let directory = session.canonical_records(directory_ref)?;
    if directory.rows() != 1 || directory.fields() != SCENE_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, scene_directory_field::SCHEMA_REVISION),
        SCENE_SCHEMA_REVISION,
    )?;
    require_scalar(
        directory.scalar::<u32>(0, scene_directory_field::ROOT_NODE),
        0,
    )?;

    let node_count_u32 = directory.scalar::<u32>(0, scene_directory_field::NODE_COUNT)?;
    require_scalar(
        Some(node_count_u32),
        u32::try_from(expected.canonical_node_count()).ok()?,
    )?;
    let node_count = usize::try_from(node_count_u32).ok()?;
    let source_node_count = node_count.checked_sub(1)?;
    let input = expected.input();
    let input_nodes = input.nodes();
    if source_node_count != input_nodes.len()
        || source_node_count != expected.canonical_order().len()
        || source_node_count != expected.canonical_index_by_local().len()
    {
        return None;
    }
    let source_node_count_u32 = u32::try_from(source_node_count).ok()?;
    require_scalar(
        Some(source_node_count_u32),
        u32::try_from(expected.reference_count()).ok()?,
    )?;
    let expected_transform_scalar_count = expected
        .matrix_scalar_count()
        .checked_add(expected.trs_scalar_count())?;
    let expected_transform_scalar_count_u32 =
        u32::try_from(expected_transform_scalar_count).ok()?;
    let has_source_nodes = source_node_count != 0;
    let expected_presence = if has_source_nodes {
        PRESENCE_PRESENT
    } else {
        PRESENCE_ABSENT
    };
    require_optional_collection(
        directory.scalar::<u8>(0, scene_directory_field::REFERENCE_PRESENCE),
        directory.scalar::<u32>(0, scene_directory_field::REFERENCE_COUNT),
        expected_presence,
        source_node_count_u32,
    )?;
    require_scalar(
        directory.scalar::<u8>(0, scene_directory_field::TRANSFORM_PRESENCE),
        expected_presence,
    )?;

    let nodes_ref =
        directory.record_ref::<NodeSceneNodeTableSchema>(0, scene_directory_field::NODES)?;
    let nodes = session.canonical_records(nodes_ref)?;
    if nodes.rows() != node_count || nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT {
        return None;
    }

    let (references_ref, references, transforms_ref, transforms) = if has_source_nodes {
        let references_ref = directory.record_ref::<NodeSceneChildReferenceTableSchema>(
            0,
            scene_directory_field::REFERENCES,
        )?;
        let references = session.canonical_records(references_ref)?;
        if references.rows() != source_node_count
            || references.fields() != NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT
        {
            return None;
        }
        let transforms_ref = directory.sequence_ref::<f32, NodeSceneAuthoredTransformSchema>(
            0,
            scene_directory_field::TRANSFORMS,
        )?;
        let transforms = session.canonical_sequence(transforms_ref)?.as_slice();
        require_scalar(
            directory.scalar::<u32>(0, scene_directory_field::TRANSFORM_SCALAR_COUNT),
            expected_transform_scalar_count_u32,
        )?;
        if transforms.len() != usize::try_from(expected_transform_scalar_count).ok()? {
            return None;
        }
        (
            Some(references_ref),
            Some(references),
            Some(transforms_ref),
            Some(transforms),
        )
    } else {
        require_scalar(
            directory.scalar::<u8>(0, scene_directory_field::REFERENCES),
            0,
        )?;
        require_scalar(
            directory.scalar::<u32>(0, scene_directory_field::TRANSFORM_SCALAR_COUNT),
            0,
        )?;
        require_scalar(Some(expected_transform_scalar_count_u32), 0)?;
        require_scalar(
            directory.scalar::<u8>(0, scene_directory_field::TRANSFORMS),
            0,
        )?;
        (None, None, None, None)
    };

    let source_root_count = match input.roots() {
        Presence::Absent => return None,
        Presence::Present(roots) => roots.len(),
    };
    let expected_root_count = u32::try_from(source_root_count).ok()?;

    observe_work(1)?;
    require_scalar(nodes.scalar::<u32>(0, node_scene_node_field::NODE), 0)?;
    require_optional_collection(
        nodes.scalar::<u8>(0, node_scene_node_field::CHILDREN_PRESENCE),
        nodes.scalar::<u32>(0, node_scene_node_field::CHILD_REFERENCE_COUNT),
        PRESENCE_PRESENT,
        expected_root_count,
    )?;
    require_scalar(
        nodes.scalar::<u32>(0, node_scene_node_field::CHILD_REFERENCE_OFFSET),
        0,
    )?;
    require_scalar(
        nodes.scalar::<u8>(0, node_scene_node_field::TRANSFORM_KIND),
        NODE_SCENE_TRANSFORM_SYNTHETIC,
    )?;
    require_scalar(
        nodes.scalar::<u32>(0, node_scene_node_field::TRANSFORM_OFFSET),
        0,
    )?;
    require_scalar(
        nodes.scalar::<u32>(0, node_scene_node_field::TRANSFORM_LENGTH),
        0,
    )?;
    require_scalar(nodes.scalar::<u8>(0, node_scene_node_field::TRANSFORMS), 0)?;

    if has_source_nodes {
        let references = references.as_ref()?;
        for source_row in 0..source_node_count {
            observe_work(1)?;
            let reference = u32::try_from(source_row).ok()?;
            require_scalar(
                references.scalar::<u32>(source_row, node_scene_child_reference_field::REFERENCE),
                reference,
            )?;
            require_scalar(
                references.scalar::<u32>(source_row, node_scene_child_reference_field::CHILD),
                reference.checked_add(1)?,
            )?;
        }
    }

    let mut transform_offset = 0_usize;
    let mut child_reference_offset = expected_root_count;
    for row in 1..node_count {
        observe_work(1)?;
        let source_row = row - 1;
        let local = *expected.canonical_order().get(source_row)?;
        let expected_node = *input_nodes.get(local)?;
        let expected_child_count = expected.child_count_by_local(local)?;
        let row_u32 = u32::try_from(row).ok()?;
        require_scalar(
            expected.canonical_index_by_local().get(local).copied(),
            row_u32,
        )?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::NODE),
            row_u32,
        )?;
        require_optional_collection(
            nodes.scalar::<u8>(row, node_scene_node_field::CHILDREN_PRESENCE),
            nodes.scalar::<u32>(row, node_scene_node_field::CHILD_REFERENCE_COUNT),
            PRESENCE_PRESENT,
            expected_child_count,
        )?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::CHILD_REFERENCE_OFFSET),
            child_reference_offset,
        )?;
        if expected_child_count != 0 && child_reference_offset.checked_add(1)? <= row_u32 {
            return None;
        }
        child_reference_offset = child_reference_offset.checked_add(expected_child_count)?;

        let kind = nodes.scalar::<u8>(row, node_scene_node_field::TRANSFORM_KIND)?;
        let transform_len = authored_transform_scalar_count(kind)?;
        observe_work(u64::try_from(transform_len).ok()?)?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::TRANSFORM_OFFSET),
            u32::try_from(transform_offset).ok()?,
        )?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::TRANSFORM_LENGTH),
            u32::try_from(transform_len).ok()?,
        )?;
        let row_transforms = nodes.sequence_ref::<f32, NodeSceneAuthoredTransformSchema>(
            row,
            node_scene_node_field::TRANSFORMS,
        )?;
        if Some(row_transforms) != transforms_ref {
            return None;
        }
        let next_transform_offset = transform_offset.checked_add(transform_len)?;
        let values = transforms
            .as_ref()?
            .get(transform_offset..next_transform_offset)?;
        let stored_transform = decode_authored_transform(kind, values)?;
        if !stored_transform.has_same_authored_bits(expected_node.authored_transform()) {
            return None;
        }
        transform_offset = next_transform_offset;
    }

    require_scalar(Some(child_reference_offset), source_node_count_u32)?;
    require_scalar(
        directory.scalar::<u32>(0, scene_directory_field::SOURCE_ROOT_COUNT),
        expected_root_count,
    )?;
    if has_source_nodes && transform_offset != transforms?.len() {
        return None;
    }

    let appearance_ref =
        directory.record_ref::<AppearanceDirectorySchema>(0, scene_directory_field::APPEARANCE)?;
    let appearance_refs =
        validate_appearance_graph(session, appearance_ref, expected_appearance, observe_work)?;

    let geometry_ref =
        directory.record_ref::<SceneGeometryDirectorySchema>(0, scene_directory_field::GEOMETRY)?;
    let geometry_refs = validate_geometry_graph(
        session,
        geometry_ref,
        nodes_ref,
        appearance_refs.materials.map(CanonicalBlockRef::into_store),
        expected,
        expected_geometry,
        observe_work,
    )?;

    let node_ref = CanonicalBlockRef::from_store(nodes_ref);
    let deformation_ref = directory
        .record_ref::<DeformationDirectorySchema>(0, scene_directory_field::DEFORMATION)?;
    let deformation_refs = validate_deformation_graph(
        session,
        deformation_ref,
        DeformationValidationDependencies::new(
            node_ref,
            geometry_refs.logical_meshes,
            geometry_refs.primitives,
            geometry_refs.mesh_instances,
        ),
        expected,
        expected_deformation,
        observe_work,
    )?;

    let animation_ref =
        directory.record_ref::<AnimationDirectorySchema>(0, scene_directory_field::ANIMATION)?;
    let animation_refs = validate_animation_graph(
        session,
        animation_ref,
        nodes_ref,
        deformation_refs
            .mesh_instances
            .map(CanonicalBlockRef::into_store),
        expected_animation,
        observe_work,
    )?;

    if !evidence_counts_match_batches(
        expected_counts,
        expected,
        expected_appearance,
        expected_geometry,
        expected_deformation,
        expected_animation,
    ) {
        return None;
    }
    observe_work(1)?;
    let evidence = session.outcome_evidence_records(evidence_ref)?;
    if evidence.rows() != 1 || evidence.fields() != NODE_SCENE_EVIDENCE_FIELD_COUNT {
        return None;
    }
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::SCHEMA_REVISION),
        SCENE_SCHEMA_REVISION,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::OUTCOME),
        NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::CANONICAL_BLOCK_COUNT),
        expected_counts.canonical_block_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::NODE_COUNT),
        expected_counts.node_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::IMAGE_COUNT),
        expected_counts.image_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::SAMPLER_COUNT),
        expected_counts.sampler_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::TEXTURE_COUNT),
        expected_counts.texture_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::MATERIAL_COUNT),
        expected_counts.material_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::MATERIAL_BINDING_COUNT),
        expected_counts.material_binding_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::LOGICAL_MESH_COUNT),
        expected_counts.logical_mesh_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::PRIMITIVE_COUNT),
        expected_counts.primitive_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::ATTRIBUTE_COUNT),
        expected_counts.attribute_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::MESH_INSTANCE_COUNT),
        expected_counts.mesh_instance_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::SKIN_COUNT),
        expected_counts.skin_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::LOGICAL_MORPH_COUNT),
        expected_counts.logical_morph_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::MORPH_DEFINITION_COUNT),
        expected_counts.morph_definition_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::PRIMITIVE_DEFORMATION_COUNT),
        expected_counts.primitive_deformation_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::MORPH_TARGET_COUNT),
        expected_counts.morph_target_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::MORPH_DELTA_COUNT),
        expected_counts.morph_delta_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::ANIMATION_CLIP_COUNT),
        expected_counts.animation_clip_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::ANIMATION_CHANNEL_COUNT),
        expected_counts.animation_channel_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::PAYLOAD_BLOCK_COUNT),
        expected_counts.payload_block_count,
    )?;
    let source_edge_count = source_node_count.checked_sub(source_root_count)?;
    Some(NodeSceneValidationProof {
        nodes: node_ref,
        references: references_ref.map(CanonicalBlockRef::from_store),
        transforms: transforms_ref.map(CanonicalBlockRef::from_store),
        appearance: appearance_refs,
        geometry: geometry_refs,
        deformation: deformation_refs,
        animation: animation_refs,
        source_node_count,
        source_root_count,
        source_edge_count,
        counts: expected_counts,
    })
}

#[allow(clippy::too_many_arguments)]
fn evidence_counts_match_batches<NR, AR, GR, DR, ANR>(
    counts: SceneEvidenceCounts,
    nodes: &ValidatedNodeBatch<'_, NR>,
    appearance: &ValidatedAppearanceBatch<'_, AR>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    deformation: &ValidatedDeformationBatch<'_, DR>,
    animation: &ValidatedAnimationBatch<'_, ANR>,
) -> bool {
    let payload_block_count = appearance
        .payload_block_count()
        .checked_add(geometry.payload_block_count())
        .and_then(|count| count.checked_add(deformation.payload_block_count()))
        .and_then(|count| count.checked_add(animation.payload_block_count()));
    u32::try_from(nodes.canonical_node_count()) == Ok(counts.node_count)
        && appearance.image_count() == counts.image_count
        && appearance.sampler_count() == counts.sampler_count
        && appearance.texture_count() == counts.texture_count
        && appearance.material_count() == counts.material_count
        && appearance.binding_count() == counts.material_binding_count
        && geometry.logical_mesh_count() == counts.logical_mesh_count
        && geometry.primitive_count() == counts.primitive_count
        && geometry.attribute_count() == counts.attribute_count
        && geometry.mesh_instance_count() == counts.mesh_instance_count
        && deformation.skin_count() == counts.skin_count
        && deformation.logical_morph_count() == counts.logical_morph_count
        && deformation.morph_definition_count() == counts.morph_definition_count
        && deformation.primitive_count() == counts.primitive_deformation_count
        && deformation.morph_target_count() == counts.morph_target_count
        && deformation.morph_delta_count() == counts.morph_delta_count
        && animation.clip_count() == counts.animation_clip_count
        && animation.channel_count() == counts.animation_channel_count
        && payload_block_count == Some(counts.payload_block_count)
}

fn count_as_usize(count: u32) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}
