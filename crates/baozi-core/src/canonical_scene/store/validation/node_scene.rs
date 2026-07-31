use baozi_operation::{
    CanonicalRecordRef, CanonicalSequenceRef, OutcomeEvidenceRecordRef,
    StoreEvidenceValidationSession, StoreValidationSession,
};

use crate::entity::AuthoredTransform;

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
    AppearanceBindingSemanticScalarSchema, AppearanceBindingTableSchema, AppearanceDirectorySchema,
    AppearanceEncodedImagePayloadSchema, AppearanceImageTableSchema,
    AppearanceMaterialFactorScalarSchema, AppearanceMaterialTableSchema, AppearanceMediaTypeSchema,
    AppearanceSamplerTableSchema, AppearanceSourceKeySchema, AppearanceTextureTableSchema,
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
use animation::{
    AnimationCanonicalDependencies, AnimationValidatedRefs, AnimationValidatedShape,
    required_animation_target_marks as required_animation_marks, validate_animation_graph,
    validate_animation_graph_structure,
};
use appearance::{
    AppearanceCanonicalCounts, AppearanceValidatedRefs, validate_appearance_canonical_structure,
    validate_appearance_graph,
};
use deformation::{
    DeformationValidatedRefs, DeformationValidatedShape, DeformationValidationDependencies,
    validate_deformation_graph, validate_deformation_graph_canonical,
};
use geometry::{
    GeometryCanonicalCounts, GeometryCanonicalDependencies, GeometryValidatedRefs,
    GeometryValidatedShape, validate_geometry_canonical_structure, validate_geometry_graph,
};

trait NodeEquivalence {
    fn validate_counts(&mut self, node_count: usize, source_node_count: usize) -> Option<()>;

    fn validate_reference_count(&mut self, reference_count: u32) -> Option<()>;

    fn transform_scalar_count_hint(&mut self) -> Option<Option<u32>>;

    fn source_root_count_hint(&mut self) -> Option<Option<u32>>;

    fn validate_node_identity(&mut self, row: usize) -> Option<()>;

    fn validate_node_children(&mut self, row: usize, child_count: u32) -> Option<()>;

    fn validate_node_transform(&mut self, row: usize, transform: AuthoredTransform) -> Option<()>;
}

struct CanonicalNodeEquivalence;

impl NodeEquivalence for CanonicalNodeEquivalence {
    fn validate_counts(&mut self, _node_count: usize, _source_node_count: usize) -> Option<()> {
        Some(())
    }

    fn validate_reference_count(&mut self, _reference_count: u32) -> Option<()> {
        Some(())
    }

    fn transform_scalar_count_hint(&mut self) -> Option<Option<u32>> {
        Some(None)
    }

    fn source_root_count_hint(&mut self) -> Option<Option<u32>> {
        Some(None)
    }

    fn validate_node_identity(&mut self, _row: usize) -> Option<()> {
        Some(())
    }

    fn validate_node_children(&mut self, _row: usize, _child_count: u32) -> Option<()> {
        Some(())
    }

    fn validate_node_transform(
        &mut self,
        _row: usize,
        _transform: AuthoredTransform,
    ) -> Option<()> {
        Some(())
    }
}

struct FreshNodeEquivalence<'batch, 'input, R> {
    expected: &'batch ValidatedNodeBatch<'input, R>,
}

impl<R> NodeEquivalence for FreshNodeEquivalence<'_, '_, R> {
    fn validate_counts(&mut self, node_count: usize, source_node_count: usize) -> Option<()> {
        if node_count != usize::try_from(self.expected.canonical_node_count()).ok()?
            || source_node_count != self.expected.input().nodes().len()
            || source_node_count != self.expected.canonical_order().len()
            || source_node_count != self.expected.canonical_index_by_local().len()
        {
            return None;
        }
        Some(())
    }

    fn validate_reference_count(&mut self, reference_count: u32) -> Option<()> {
        require_scalar(
            Some(reference_count),
            u32::try_from(self.expected.reference_count()).ok()?,
        )
    }

    fn transform_scalar_count_hint(&mut self) -> Option<Option<u32>> {
        let expected = self
            .expected
            .matrix_scalar_count()
            .checked_add(self.expected.trs_scalar_count())?;
        Some(Some(u32::try_from(expected).ok()?))
    }

    fn source_root_count_hint(&mut self) -> Option<Option<u32>> {
        let Presence::Present(roots) = self.expected.input().roots() else {
            return None;
        };
        Some(Some(u32::try_from(roots.len()).ok()?))
    }

    fn validate_node_identity(&mut self, row: usize) -> Option<()> {
        let source_row = row.checked_sub(1)?;
        let local = *self.expected.canonical_order().get(source_row)?;
        require_scalar(
            self.expected.canonical_index_by_local().get(local).copied(),
            u32::try_from(row).ok()?,
        )
    }

    fn validate_node_children(&mut self, row: usize, child_count: u32) -> Option<()> {
        let source_row = row.checked_sub(1)?;
        let local = *self.expected.canonical_order().get(source_row)?;
        require_scalar(
            Some(child_count),
            self.expected.child_count_by_local(local)?,
        )
    }

    fn validate_node_transform(&mut self, row: usize, transform: AuthoredTransform) -> Option<()> {
        let source_row = row.checked_sub(1)?;
        let local = *self.expected.canonical_order().get(source_row)?;
        let expected_node = *self.expected.input().nodes().get(local)?;
        transform
            .has_same_authored_bits(expected_node.authored_transform())
            .then_some(())
    }
}

#[derive(Clone, Copy)]
struct NodeCanonicalStructure {
    directory: CanonicalRecordRef<SceneDirectorySchema>,
    nodes: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    references: Option<CanonicalRecordRef<NodeSceneChildReferenceTableSchema>>,
    transforms: Option<CanonicalSequenceRef<f32, NodeSceneAuthoredTransformSchema>>,
    appearance_directory: CanonicalRecordRef<AppearanceDirectorySchema>,
    geometry_directory: CanonicalRecordRef<SceneGeometryDirectorySchema>,
    deformation_directory: CanonicalRecordRef<DeformationDirectorySchema>,
    animation_directory: CanonicalRecordRef<AnimationDirectorySchema>,
    source_node_count: usize,
    source_root_count: usize,
    source_edge_count: usize,
}

pub(in crate::canonical_scene::store) struct NodeSceneValidationScratch<'scratch> {
    deformation_palette_maxima: &'scratch mut [Option<u32>],
    deformation_uniqueness_marks: &'scratch mut [u64],
    animation_target_marks: &'scratch mut [u32],
}

#[derive(Clone, Copy)]
struct NodeSceneValidationScratchRequirements {
    deformation_palette_marks: usize,
    deformation_uniqueness_marks: usize,
    animation_target_marks: usize,
}

impl<'scratch> NodeSceneValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) const fn new(
        deformation_palette_maxima: &'scratch mut [Option<u32>],
        deformation_uniqueness_marks: &'scratch mut [u64],
        animation_target_marks: &'scratch mut [u32],
    ) -> Self {
        Self {
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_target_marks,
        }
    }

    pub(in crate::canonical_scene::store) fn required_deformation_palette_marks(
        counts: SceneEvidenceCounts,
    ) -> Option<usize> {
        usize::try_from(counts.logical_mesh_count).ok()
    }

    pub(in crate::canonical_scene::store) fn required_animation_target_marks(
        counts: SceneEvidenceCounts,
    ) -> Option<usize> {
        required_animation_marks(
            usize::try_from(counts.node_count).ok()?,
            usize::try_from(counts.mesh_instance_count).ok()?,
        )
    }

    fn requirements(
        counts: SceneEvidenceCounts,
        deformation_uniqueness_marks: usize,
    ) -> Option<NodeSceneValidationScratchRequirements> {
        Some(NodeSceneValidationScratchRequirements {
            deformation_palette_marks: Self::required_deformation_palette_marks(counts)?,
            deformation_uniqueness_marks,
            animation_target_marks: Self::required_animation_target_marks(counts)?,
        })
    }

    fn validate_known_capacity(
        &self,
        requirements: NodeSceneValidationScratchRequirements,
    ) -> Option<()> {
        (self.deformation_palette_maxima.len() >= requirements.deformation_palette_marks
            && self.deformation_uniqueness_marks.len() >= requirements.deformation_uniqueness_marks
            && self.animation_target_marks.len() >= requirements.animation_target_marks)
            .then_some(())
    }

    fn initialize(
        &mut self,
        requirements: NodeSceneValidationScratchRequirements,
        observe_work: &mut impl FnMut(u64) -> Option<()>,
    ) -> Option<()> {
        for maximum in self
            .deformation_palette_maxima
            .get_mut(..requirements.deformation_palette_marks)?
        {
            observe_work(1)?;
            *maximum = None;
        }
        for mark in self
            .deformation_uniqueness_marks
            .get_mut(..requirements.deformation_uniqueness_marks)?
        {
            observe_work(1)?;
            *mark = 0;
        }
        for mark in self
            .animation_target_marks
            .get_mut(..requirements.animation_target_marks)?
        {
            observe_work(1)?;
            *mark = 0;
        }
        Some(())
    }
}

/// Immutable semantic authority minted only by central node-scene validation.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneValidationProof {
    directory: CanonicalBlockRef<SceneDirectorySchema>,
    appearance_directory: CanonicalBlockRef<AppearanceDirectorySchema>,
    geometry_directory: CanonicalBlockRef<SceneGeometryDirectorySchema>,
    deformation_directory: CanonicalBlockRef<DeformationDirectorySchema>,
    animation_directory: CanonicalBlockRef<AnimationDirectorySchema>,
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
    deformation_uniqueness_mark_count: usize,
    counts: SceneEvidenceCounts,
}

impl NodeSceneValidationProof {
    pub(in crate::canonical_scene::store) const fn directory(
        self,
    ) -> CanonicalBlockRef<SceneDirectorySchema> {
        self.directory
    }

    pub(in crate::canonical_scene::store) const fn appearance_directory(
        self,
    ) -> CanonicalBlockRef<AppearanceDirectorySchema> {
        self.appearance_directory
    }

    pub(in crate::canonical_scene::store) const fn geometry_directory(
        self,
    ) -> CanonicalBlockRef<SceneGeometryDirectorySchema> {
        self.geometry_directory
    }

    pub(in crate::canonical_scene::store) const fn deformation_directory(
        self,
    ) -> CanonicalBlockRef<DeformationDirectorySchema> {
        self.deformation_directory
    }

    pub(in crate::canonical_scene::store) const fn animation_directory(
        self,
    ) -> CanonicalBlockRef<AnimationDirectorySchema> {
        self.animation_directory
    }

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

    pub(in crate::canonical_scene::store) const fn appearance_source_keys(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceSourceKeySchema>> {
        self.appearance.source_keys
    }

    pub(in crate::canonical_scene::store) const fn appearance_binding_semantic_scalars(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceBindingSemanticScalarSchema>> {
        self.appearance.binding_semantic_scalars
    }

    pub(in crate::canonical_scene::store) const fn appearance_material_factor_scalars(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceMaterialFactorScalarSchema>> {
        self.appearance.material_factor_scalars
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

    pub(in crate::canonical_scene::store) const fn counts(self) -> SceneEvidenceCounts {
        self.counts
    }

    pub(in crate::canonical_scene::store) const fn deformation_uniqueness_mark_count(
        self,
    ) -> usize {
        self.deformation_uniqueness_mark_count
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

fn validate_node_canonical_structure(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    expected_node_count: Option<u32>,
    equivalence: &mut impl NodeEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<NodeCanonicalStructure> {
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
    if let Some(expected_node_count) = expected_node_count {
        require_scalar(Some(node_count_u32), expected_node_count)?;
    }
    let node_count = usize::try_from(node_count_u32).ok()?;
    let source_node_count = node_count.checked_sub(1)?;
    equivalence.validate_counts(node_count, source_node_count)?;
    let source_node_count_u32 = u32::try_from(source_node_count).ok()?;
    equivalence.validate_reference_count(source_node_count_u32)?;
    let has_source_nodes = source_node_count != 0;
    let collection_presence = if has_source_nodes {
        PRESENCE_PRESENT
    } else {
        PRESENCE_ABSENT
    };
    require_optional_collection(
        directory.scalar::<u8>(0, scene_directory_field::REFERENCE_PRESENCE),
        directory.scalar::<u32>(0, scene_directory_field::REFERENCE_COUNT),
        collection_presence,
        source_node_count_u32,
    )?;
    require_scalar(
        directory.scalar::<u8>(0, scene_directory_field::TRANSFORM_PRESENCE),
        collection_presence,
    )?;

    let nodes_ref =
        directory.record_ref::<NodeSceneNodeTableSchema>(0, scene_directory_field::NODES)?;
    let nodes = session.canonical_records(nodes_ref)?;
    if nodes.rows() != node_count || nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT {
        return None;
    }

    let transform_scalar_count_hint = equivalence.transform_scalar_count_hint()?;
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
        let transform_scalar_count =
            directory.scalar::<u32>(0, scene_directory_field::TRANSFORM_SCALAR_COUNT)?;
        if let Some(expected_transform_scalar_count) = transform_scalar_count_hint {
            require_scalar(
                Some(transform_scalar_count),
                expected_transform_scalar_count,
            )?;
        }
        if transforms.len() != usize::try_from(transform_scalar_count).ok()? {
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
        if let Some(expected_transform_scalar_count) = transform_scalar_count_hint {
            require_scalar(Some(expected_transform_scalar_count), 0)?;
        }
        require_scalar(
            directory.scalar::<u8>(0, scene_directory_field::TRANSFORMS),
            0,
        )?;
        (None, None, None, None)
    };

    let source_root_count_u32 = match equivalence.source_root_count_hint()? {
        Some(root_count) => root_count,
        None => directory.scalar::<u32>(0, scene_directory_field::SOURCE_ROOT_COUNT)?,
    };
    if source_root_count_u32 > source_node_count_u32 {
        return None;
    }
    let source_root_count = usize::try_from(source_root_count_u32).ok()?;

    observe_work(1)?;
    require_scalar(nodes.scalar::<u32>(0, node_scene_node_field::NODE), 0)?;
    require_optional_collection(
        nodes.scalar::<u8>(0, node_scene_node_field::CHILDREN_PRESENCE),
        nodes.scalar::<u32>(0, node_scene_node_field::CHILD_REFERENCE_COUNT),
        PRESENCE_PRESENT,
        source_root_count_u32,
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

    if let Some(references) = references {
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
    let mut child_reference_offset = source_root_count_u32;
    for row in 1..node_count {
        observe_work(1)?;
        let row_u32 = u32::try_from(row).ok()?;
        equivalence.validate_node_identity(row)?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::NODE),
            row_u32,
        )?;
        let child_count = nodes.scalar::<u32>(row, node_scene_node_field::CHILD_REFERENCE_COUNT)?;
        require_optional_collection(
            nodes.scalar::<u8>(row, node_scene_node_field::CHILDREN_PRESENCE),
            Some(child_count),
            PRESENCE_PRESENT,
            child_count,
        )?;
        equivalence.validate_node_children(row, child_count)?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::CHILD_REFERENCE_OFFSET),
            child_reference_offset,
        )?;
        if child_count != 0 && child_reference_offset.checked_add(1)? <= row_u32 {
            return None;
        }
        child_reference_offset = child_reference_offset.checked_add(child_count)?;

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
        require_scalar(
            nodes.sequence_ref::<f32, NodeSceneAuthoredTransformSchema>(
                row,
                node_scene_node_field::TRANSFORMS,
            ),
            transforms_ref?,
        )?;
        let next_transform_offset = transform_offset.checked_add(transform_len)?;
        let values = transforms?.get(transform_offset..next_transform_offset)?;
        let transform = decode_authored_transform(kind, values)?;
        equivalence.validate_node_transform(row, transform)?;
        transform_offset = next_transform_offset;
    }

    require_scalar(Some(child_reference_offset), source_node_count_u32)?;
    require_scalar(
        directory.scalar::<u32>(0, scene_directory_field::SOURCE_ROOT_COUNT),
        source_root_count_u32,
    )?;
    if has_source_nodes && transform_offset != transforms?.len() {
        return None;
    }

    let appearance_directory =
        directory.record_ref::<AppearanceDirectorySchema>(0, scene_directory_field::APPEARANCE)?;
    let geometry_directory =
        directory.record_ref::<SceneGeometryDirectorySchema>(0, scene_directory_field::GEOMETRY)?;
    let deformation_directory = directory
        .record_ref::<DeformationDirectorySchema>(0, scene_directory_field::DEFORMATION)?;
    let animation_directory =
        directory.record_ref::<AnimationDirectorySchema>(0, scene_directory_field::ANIMATION)?;
    Some(NodeCanonicalStructure {
        directory: directory_ref,
        nodes: nodes_ref,
        references: references_ref,
        transforms: transforms_ref,
        appearance_directory,
        geometry_directory,
        deformation_directory,
        animation_directory,
        source_node_count,
        source_root_count,
        source_edge_count: source_node_count.checked_sub(source_root_count)?,
    })
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

#[allow(
    clippy::too_many_arguments,
    reason = "the canonical boundary keeps root references, immutable proof counts, scratch, and work authority explicit"
)]
pub(in crate::canonical_scene::store) fn validate_node_scene_canonical_graph<E>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    expected_counts: SceneEvidenceCounts,
    canonical_unique_block_count: u32,
    deformation_uniqueness_capacity: usize,
    scratch: &mut NodeSceneValidationScratch<'_>,
    mut observe_work: impl FnMut(u64) -> Result<(), E>,
) -> Result<Option<NodeSceneValidationProof>, E> {
    let mut work_failure = None;
    let proof = validate_node_scene_canonical_graph_inner(
        session,
        directory_ref,
        evidence_ref,
        expected_counts,
        canonical_unique_block_count,
        deformation_uniqueness_capacity,
        scratch,
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

#[allow(
    clippy::too_many_arguments,
    reason = "the inner boundary mirrors the checked public validator without hiding proof or scratch ownership"
)]
fn validate_node_scene_canonical_graph_inner(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    expected_counts: SceneEvidenceCounts,
    canonical_unique_block_count: u32,
    deformation_uniqueness_capacity: usize,
    scratch: &mut NodeSceneValidationScratch<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<NodeSceneValidationProof> {
    let scratch_requirements =
        NodeSceneValidationScratch::requirements(expected_counts, deformation_uniqueness_capacity)?;
    scratch.validate_known_capacity(scratch_requirements)?;
    scratch.initialize(scratch_requirements, observe_work)?;
    let node_structure = validate_node_canonical_structure(
        session,
        directory_ref,
        Some(expected_counts.node_count),
        &mut CanonicalNodeEquivalence,
        observe_work,
    )?;
    let node_ref = CanonicalBlockRef::from_store(node_structure.nodes);
    let appearance = validate_appearance_canonical_structure(
        session,
        node_structure.appearance_directory,
        AppearanceCanonicalCounts::new(
            expected_counts.image_count,
            expected_counts.sampler_count,
            expected_counts.texture_count,
            expected_counts.material_count,
            expected_counts.material_binding_count,
        ),
        observe_work,
    )?;
    let geometry = validate_geometry_canonical_structure(
        session,
        node_structure.geometry_directory,
        GeometryCanonicalDependencies::new(
            node_structure.nodes,
            expected_counts.node_count,
            appearance.materials.map(CanonicalBlockRef::into_store),
            expected_counts.material_count,
        ),
        GeometryCanonicalCounts::new(
            expected_counts.logical_mesh_count,
            expected_counts.primitive_count,
            expected_counts.attribute_count,
            expected_counts.mesh_instance_count,
        ),
        observe_work,
    )?;
    let deformation = validate_deformation_graph_canonical(
        session,
        node_structure.deformation_directory,
        DeformationValidationDependencies::new(
            node_ref,
            geometry.refs.logical_meshes,
            geometry.refs.primitives,
            geometry.refs.mesh_instances,
        ),
        scratch
            .deformation_palette_maxima
            .get_mut(..scratch_requirements.deformation_palette_marks)?,
        scratch
            .deformation_uniqueness_marks
            .get_mut(..scratch_requirements.deformation_uniqueness_marks)?,
        observe_work,
    )?;
    let animation = validate_animation_graph_structure(
        session,
        node_structure.animation_directory,
        node_structure.nodes,
        AnimationCanonicalDependencies::new(
            deformation.refs.mesh_instances,
            geometry.refs.mesh_instances,
            deformation.refs.logical_morphs,
        ),
        scratch
            .animation_target_marks
            .get_mut(..scratch_requirements.animation_target_marks)?,
        observe_work,
    )?;
    validate_canonical_evidence_counts(
        expected_counts,
        canonical_unique_block_count,
        node_structure,
        appearance,
        geometry,
        deformation,
        animation,
    )?;
    validate_node_scene_success_evidence(session, evidence_ref, expected_counts, observe_work)?;
    Some(assemble_node_scene_validation_proof(
        node_structure,
        appearance,
        geometry.refs,
        deformation,
        animation.refs,
        expected_counts,
    ))
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

fn validate_node_scene_success_evidence(
    session: &mut StoreValidationSession<'_, '_>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    counts: SceneEvidenceCounts,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
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
    for (field, expected) in [
        (
            node_scene_evidence_field::CANONICAL_BLOCK_COUNT,
            counts.canonical_block_count,
        ),
        (node_scene_evidence_field::NODE_COUNT, counts.node_count),
        (node_scene_evidence_field::IMAGE_COUNT, counts.image_count),
        (
            node_scene_evidence_field::SAMPLER_COUNT,
            counts.sampler_count,
        ),
        (
            node_scene_evidence_field::TEXTURE_COUNT,
            counts.texture_count,
        ),
        (
            node_scene_evidence_field::MATERIAL_COUNT,
            counts.material_count,
        ),
        (
            node_scene_evidence_field::MATERIAL_BINDING_COUNT,
            counts.material_binding_count,
        ),
        (
            node_scene_evidence_field::LOGICAL_MESH_COUNT,
            counts.logical_mesh_count,
        ),
        (
            node_scene_evidence_field::PRIMITIVE_COUNT,
            counts.primitive_count,
        ),
        (
            node_scene_evidence_field::ATTRIBUTE_COUNT,
            counts.attribute_count,
        ),
        (
            node_scene_evidence_field::MESH_INSTANCE_COUNT,
            counts.mesh_instance_count,
        ),
        (node_scene_evidence_field::SKIN_COUNT, counts.skin_count),
        (
            node_scene_evidence_field::LOGICAL_MORPH_COUNT,
            counts.logical_morph_count,
        ),
        (
            node_scene_evidence_field::MORPH_DEFINITION_COUNT,
            counts.morph_definition_count,
        ),
        (
            node_scene_evidence_field::PRIMITIVE_DEFORMATION_COUNT,
            counts.primitive_deformation_count,
        ),
        (
            node_scene_evidence_field::MORPH_TARGET_COUNT,
            counts.morph_target_count,
        ),
        (
            node_scene_evidence_field::MORPH_DELTA_COUNT,
            counts.morph_delta_count,
        ),
        (
            node_scene_evidence_field::ANIMATION_CLIP_COUNT,
            counts.animation_clip_count,
        ),
        (
            node_scene_evidence_field::ANIMATION_CHANNEL_COUNT,
            counts.animation_channel_count,
        ),
        (
            node_scene_evidence_field::PAYLOAD_BLOCK_COUNT,
            counts.payload_block_count,
        ),
    ] {
        require_scalar(evidence.scalar::<u32>(0, field), expected)?;
    }
    Some(())
}

fn validate_canonical_evidence_counts(
    counts: SceneEvidenceCounts,
    canonical_unique_block_count: u32,
    node: NodeCanonicalStructure,
    appearance: AppearanceValidatedRefs,
    geometry: GeometryValidatedShape,
    deformation: DeformationValidatedShape,
    animation: AnimationValidatedShape,
) -> Option<()> {
    for (actual, expected) in [
        (deformation.skin_count, counts.skin_count),
        (deformation.logical_morph_count, counts.logical_morph_count),
        (
            deformation.morph_definition_count,
            counts.morph_definition_count,
        ),
        (
            deformation.primitive_count,
            counts.primitive_deformation_count,
        ),
        (deformation.morph_target_count, counts.morph_target_count),
        (deformation.morph_delta_count, counts.morph_delta_count),
        (deformation.mesh_instance_count, counts.mesh_instance_count),
        (animation.clip_count, counts.animation_clip_count),
        (animation.channel_count, counts.animation_channel_count),
    ] {
        require_scalar(Some(actual), expected)?;
    }

    let appearance_payload_block_count = present_count(&[
        appearance.image_payloads.is_some(),
        appearance.media_types.is_some(),
        appearance.source_keys.is_some(),
        appearance.binding_semantic_scalars.is_some(),
        appearance.material_factor_scalars.is_some(),
    ])?;
    let deformation_payload_block_count = present_count(&[
        deformation.refs.joint_nodes.is_some(),
        deformation.refs.inverse_bind_matrices.is_some(),
        deformation.refs.skin_correction_matrices.is_some(),
        deformation.refs.skin_names.is_some(),
        deformation.refs.influence_row_offsets.is_some(),
        deformation.refs.influence_joints.is_some(),
        deformation.refs.influence_weights.is_some(),
        deformation.refs.influence_source_orders.is_some(),
        deformation.refs.morph_defaults.is_some(),
        deformation.refs.morph_names.is_some(),
        deformation.refs.morph_values.is_some(),
        deformation.refs.morph_sparse_indices.is_some(),
        deformation.refs.instance_morph_weights.is_some(),
    ])?;
    require_scalar(
        Some(deformation.payload_block_count),
        deformation_payload_block_count,
    )?;
    let animation_payload_block_count = present_count(&[
        animation.refs.times.is_some(),
        animation.refs.values.is_some(),
        animation.refs.in_tangents.is_some(),
        animation.refs.out_tangents.is_some(),
    ])?;
    require_scalar(
        Some(animation.payload_block_count),
        animation_payload_block_count,
    )?;
    let payload_block_count = appearance_payload_block_count
        .checked_add(geometry.payload_block_count)?
        .checked_add(deformation_payload_block_count)?
        .checked_add(animation_payload_block_count)?;
    require_scalar(Some(payload_block_count), counts.payload_block_count)?;

    let node_block_count = 2_u32
        .checked_add(u32::from(node.references.is_some()))?
        .checked_add(u32::from(node.transforms.is_some()))?;
    let appearance_block_count = 1_u32.checked_add(present_count(&[
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
    ])?)?;
    let geometry_block_count = 1_u32
        .checked_add(present_count(&[
            geometry.refs.logical_meshes.is_some(),
            geometry.refs.primitives.is_some(),
            geometry.refs.attributes.is_some(),
            geometry.refs.mesh_instances.is_some(),
        ])?)?
        .checked_add(geometry.payload_block_count)?;
    let deformation_block_count = 1_u32.checked_add(present_count(&[
        deformation.refs.skins.is_some(),
        deformation.refs.logical_morphs.is_some(),
        deformation.refs.morph_definitions.is_some(),
        deformation.refs.primitives.is_some(),
        deformation.refs.morph_targets.is_some(),
        deformation.refs.morph_deltas.is_some(),
        deformation.refs.mesh_instances.is_some(),
        deformation.refs.joint_nodes.is_some(),
        deformation.refs.inverse_bind_matrices.is_some(),
        deformation.refs.skin_correction_matrices.is_some(),
        deformation.refs.skin_names.is_some(),
        deformation.refs.influence_row_offsets.is_some(),
        deformation.refs.influence_joints.is_some(),
        deformation.refs.influence_weights.is_some(),
        deformation.refs.influence_source_orders.is_some(),
        deformation.refs.morph_defaults.is_some(),
        deformation.refs.morph_names.is_some(),
        deformation.refs.morph_values.is_some(),
        deformation.refs.morph_sparse_indices.is_some(),
        deformation.refs.instance_morph_weights.is_some(),
    ])?)?;
    let animation_block_count = 1_u32.checked_add(present_count(&[
        animation.refs.clips.is_some(),
        animation.refs.channels.is_some(),
        animation.refs.times.is_some(),
        animation.refs.values.is_some(),
        animation.refs.in_tangents.is_some(),
        animation.refs.out_tangents.is_some(),
    ])?)?;
    let canonical_logical_block_count = node_block_count
        .checked_add(appearance_block_count)?
        .checked_add(geometry_block_count)?
        .checked_add(deformation_block_count)?
        .checked_add(animation_block_count)?;
    require_scalar(
        Some(canonical_unique_block_count),
        counts.canonical_block_count,
    )?;
    // NodeScene canonical normal form assigns one destination slot to every logical block.
    require_scalar(
        Some(canonical_logical_block_count),
        canonical_unique_block_count,
    )
}

fn present_count(present: &[bool]) -> Option<u32> {
    present.iter().try_fold(0_u32, |count, &present| {
        count.checked_add(u32::from(present))
    })
}

fn assemble_node_scene_validation_proof(
    node: NodeCanonicalStructure,
    appearance: AppearanceValidatedRefs,
    geometry: GeometryValidatedRefs,
    deformation: DeformationValidatedShape,
    animation: AnimationValidatedRefs,
    counts: SceneEvidenceCounts,
) -> NodeSceneValidationProof {
    NodeSceneValidationProof {
        directory: CanonicalBlockRef::from_store(node.directory),
        appearance_directory: CanonicalBlockRef::from_store(node.appearance_directory),
        geometry_directory: CanonicalBlockRef::from_store(node.geometry_directory),
        deformation_directory: CanonicalBlockRef::from_store(node.deformation_directory),
        animation_directory: CanonicalBlockRef::from_store(node.animation_directory),
        nodes: CanonicalBlockRef::from_store(node.nodes),
        references: node.references.map(CanonicalBlockRef::from_store),
        transforms: node.transforms.map(CanonicalBlockRef::from_store),
        appearance,
        geometry,
        deformation: deformation.refs,
        animation,
        source_node_count: node.source_node_count,
        source_root_count: node.source_root_count,
        source_edge_count: node.source_edge_count,
        deformation_uniqueness_mark_count: deformation.maximum_uniqueness_count,
        counts,
    }
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
    let node_structure = validate_node_canonical_structure(
        session,
        directory_ref,
        None,
        &mut FreshNodeEquivalence { expected },
        observe_work,
    )?;
    let nodes_ref = node_structure.nodes;
    let appearance_ref = node_structure.appearance_directory;
    let appearance_refs =
        validate_appearance_graph(session, appearance_ref, expected_appearance, observe_work)?;

    let geometry_ref = node_structure.geometry_directory;
    let geometry_refs = validate_geometry_graph(
        session,
        geometry_ref,
        nodes_ref,
        appearance_refs.materials.map(CanonicalBlockRef::into_store),
        expected_appearance.material_count(),
        expected,
        expected_geometry,
        observe_work,
    )?;

    let node_ref = CanonicalBlockRef::from_store(nodes_ref);
    let deformation_ref = node_structure.deformation_directory;
    let deformation = validate_deformation_graph(
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

    let animation_ref = node_structure.animation_directory;
    let animation_refs = validate_animation_graph(
        session,
        animation_ref,
        nodes_ref,
        deformation
            .refs
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
    validate_node_scene_success_evidence(session, evidence_ref, expected_counts, observe_work)?;
    Some(assemble_node_scene_validation_proof(
        node_structure,
        appearance_refs,
        geometry_refs,
        deformation,
        animation_refs,
        expected_counts,
    ))
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

#[cfg(test)]
mod tests {
    use crate::canonical_scene::store::construction::node_scene::SceneEvidenceCounts;

    use super::{NodeSceneValidationScratch, NodeSceneValidationScratchRequirements};

    #[test]
    fn scratch_capacity_rejects_missing_uniqueness_marks_before_mutation() {
        let mut palette = [Some(7)];
        let mut uniqueness = [11_u64, 12];
        let mut animation = [21_u32];
        let scratch =
            NodeSceneValidationScratch::new(&mut palette, &mut uniqueness, &mut animation);
        let counts = SceneEvidenceCounts {
            canonical_block_count: 0,
            node_count: 0,
            image_count: 0,
            sampler_count: 0,
            texture_count: 0,
            material_count: 0,
            material_binding_count: 0,
            logical_mesh_count: 0,
            primitive_count: 0,
            attribute_count: 0,
            mesh_instance_count: 0,
            skin_count: 0,
            logical_morph_count: 0,
            morph_definition_count: 0,
            primitive_deformation_count: 0,
            morph_target_count: 0,
            morph_delta_count: 0,
            animation_clip_count: 0,
            animation_channel_count: 0,
            payload_block_count: 0,
        };

        let requirements = NodeSceneValidationScratch::requirements(counts, 3)
            .expect("test scratch requirements must fit the platform");
        assert_eq!(scratch.validate_known_capacity(requirements), None);
        assert_eq!(palette, [Some(7)]);
        assert_eq!(uniqueness, [11, 12]);
        assert_eq!(animation, [21]);
    }

    #[test]
    fn scratch_initialization_restarts_after_cancellation() {
        let mut palette = [Some(7), Some(u32::MAX)];
        let mut uniqueness = [11_u64, 12, 13];
        let mut animation = [21_u32, 22];

        let mut observed = 0_u64;
        let requirements = NodeSceneValidationScratchRequirements {
            deformation_palette_marks: 2,
            deformation_uniqueness_marks: 3,
            animation_target_marks: 2,
        };
        {
            let mut scratch =
                NodeSceneValidationScratch::new(&mut palette, &mut uniqueness, &mut animation);
            assert_eq!(
                scratch.initialize(requirements, &mut |amount| {
                    observed = observed.checked_add(amount)?;
                    (observed < 3).then_some(())
                }),
                None
            );
        }
        assert_eq!(palette, [None, None]);
        assert_eq!(uniqueness, [11, 12, 13]);
        assert_eq!(animation, [21, 22]);

        observed = 0;
        {
            let mut scratch =
                NodeSceneValidationScratch::new(&mut palette, &mut uniqueness, &mut animation);
            assert_eq!(
                scratch.initialize(requirements, &mut |amount| {
                    observed = observed.checked_add(amount)?;
                    Some(())
                }),
                Some(())
            );
        }
        assert_eq!(observed, 7);
        assert_eq!(palette, [None, None]);
        assert_eq!(uniqueness, [0, 0, 0]);
        assert_eq!(animation, [0, 0]);
    }

    #[test]
    fn scratch_initialization_leaves_unqualified_capacity_untouched() {
        let mut palette = [Some(7), Some(8)];
        let mut uniqueness = [11_u64, 12, 13];
        let mut animation = [21_u32, 22];
        let requirements = NodeSceneValidationScratchRequirements {
            deformation_palette_marks: 1,
            deformation_uniqueness_marks: 2,
            animation_target_marks: 1,
        };
        let mut observed = 0_u64;

        let mut scratch =
            NodeSceneValidationScratch::new(&mut palette, &mut uniqueness, &mut animation);
        assert_eq!(
            scratch.initialize(requirements, &mut |amount| {
                observed = observed.checked_add(amount)?;
                Some(())
            }),
            Some(())
        );

        assert_eq!(observed, 4);
        assert_eq!(palette, [None, Some(8)]);
        assert_eq!(uniqueness, [0, 0, 13]);
        assert_eq!(animation, [0, 22]);
    }
}
