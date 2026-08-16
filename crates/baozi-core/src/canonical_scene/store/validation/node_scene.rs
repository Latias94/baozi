use baozi_operation::__canonical_activation::{
    CanonicalValidationMember, CanonicalValidationSession, OutcomeEvidenceValidationMember,
};
use baozi_operation::{
    CanonicalRecordRef, CanonicalSequenceRef, OperationOwner, OutcomeEvidenceRecordRef,
    ProgressPhase, ProgressUnit, RecordBlock, ScalarSequence, StoreEvidenceValidationSession,
    StoreValidationSession, StoreWorkError,
};

use crate::entity::AuthoredTransform;

mod animation;
mod appearance;
mod deformation;
mod geometry;
mod profile;
mod spatial;

use super::super::construction::node_scene::SceneEvidenceCounts;
use super::super::construction::node_scene::{
    NodeSceneMaterializedRefs, NodeSceneMeteredValidationScratch, NodeSceneValidationScratchPlan,
};
use super::super::node_scene::{authored_transform_scalar_count, decode_authored_transform};
use super::super::production::{
    Presence, ValidatedAnimationBatch, ValidatedAppearanceBatch, ValidatedDeformationBatch,
    ValidatedGeometryBatch, ValidatedNodeBatch, ValidatedProfileBatch, ValidatedSpatialBatch,
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
use super::super::schema::profile::{
    ProfileDirectorySchema, ProfileMetadataDirectorySchema, ProfileMetadataEntryTableSchema,
    ProfileOwnerTableSchema, ProfileSegmentTableSchema,
};
use super::super::schema::spatial::{
    SpatialCameraTableSchema, SpatialDirectorySchema, SpatialLightTableSchema,
    SpatialTangentBasisSchemaNameSchema, SpatialTangentEncodingSchemaNameSchema,
};
use super::super::schema::{
    CanonicalBlockRef, CanonicalSchema, NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    NODE_SCENE_NODE_FIELD_COUNT, NODE_SCENE_TRANSFORM_SYNTHETIC, NodeSceneAuthoredTransformSchema,
    NodeSceneChildReferenceTableSchema, NodeSceneNodeTableSchema, NodeSceneOutcomeEvidenceSchema,
    PRESENCE_ABSENT, PRESENCE_PRESENT, SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION,
    SceneDirectorySchema, SceneGeometryDirectorySchema, node_scene_child_reference_field,
    node_scene_evidence_field, node_scene_node_field, scene_directory_field,
};
use super::{require_optional_collection, require_scalar};
use animation::{
    AnimationCanonicalDependencies, AnimationMeteredDependencies, AnimationValidatedRefs,
    AnimationValidatedShape, required_animation_target_marks as required_animation_marks,
    validate_animation_graph, validate_animation_graph_metered, validate_animation_graph_structure,
};
pub(in crate::canonical_scene::store) use animation::{
    AnimationChannelValidationDescriptor, AnimationClipValidationDescriptor,
    AnimationMeteredCounts, AnimationValidationScratch, AnimationValidationScratchRequirements,
};
pub(in crate::canonical_scene::store) use appearance::{
    AppearanceBindingValidationScratch, AppearanceCanonicalCounts,
    AppearanceMaterialValidationScratch, AppearanceValidationScratch,
    AppearanceValidationScratchRequirements,
};
use appearance::{
    AppearanceValidatedRefs, validate_appearance_canonical_structure,
    validate_appearance_canonical_structure_metered, validate_appearance_graph,
};
use deformation::{
    DeformationGeometryValidationProjection, DeformationValidatedRefs, DeformationValidatedShape,
    DeformationValidationDependencies, DeformationValidationDependenciesMetered,
    validate_deformation_graph, validate_deformation_graph_canonical,
    validate_deformation_graph_canonical_metered,
};
pub(in crate::canonical_scene::store) use deformation::{
    DeformationLogicalMorphValidationProjection, DeformationMeshInstanceValidationProjection,
    DeformationMorphDefinitionValidationProjection, DeformationMorphDeltaValidationProjection,
    DeformationMorphTargetValidationProjection, DeformationPrimitiveValidationProjection,
    DeformationSkinValidationProjection, DeformationValidationScratch,
    DeformationValidationScratchRequirements,
};
pub(in crate::canonical_scene::store) use geometry::{
    GeometryAttributeValidationScratch, GeometryCanonicalCounts,
    GeometryLogicalMeshValidationProjection, GeometryMeshInstanceValidationProjection,
    GeometryPrimitiveValidationProjection, GeometryPrimitiveValidationScratch,
    GeometryValidationScratch, GeometryValidationScratchRequirements,
};
use geometry::{
    GeometryCanonicalDependencies, GeometryCanonicalValidationDependencies, GeometryValidatedRefs,
    GeometryValidatedShape, validate_geometry_canonical_structure,
    validate_geometry_canonical_structure_metered, validate_geometry_graph,
};
pub(in crate::canonical_scene::store) use profile::{ProfileMeteredCounts, ProfileMeteredScratch};
use profile::{
    ProfileValidatedRefs, ProfileValidationScratch, validate_profile_canonical_structure,
    validate_profile_graph, validate_profile_graph_metered,
};
use spatial::{
    SpatialValidatedMembers, SpatialValidatedRefs, validate_spatial_canonical_structure,
    validate_spatial_canonical_structure_metered, validate_spatial_graph,
};

trait NodeEquivalence {
    fn validate_counts(&mut self, node_count: usize, source_node_count: usize) -> Option<()>;

    fn validate_reference_count(&mut self, reference_count: u32) -> Option<()>;

    fn transform_scalar_count_hint(&mut self) -> Option<Option<u32>>;

    fn source_root_count_hint(&mut self) -> Option<Option<u32>>;

    fn validate_node_identity(&mut self, row: usize) -> Option<()>;

    fn validate_node_children(&mut self, row: usize, child_count: u32) -> Option<()>;

    fn validate_node_transform(&mut self, row: usize, transform: AuthoredTransform) -> Option<()>;

    fn validate_node_spatial_attachments(
        &mut self,
        row: usize,
        camera: Option<u32>,
        light: Option<u32>,
    ) -> Option<()>;
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

    fn validate_node_spatial_attachments(
        &mut self,
        _row: usize,
        _camera: Option<u32>,
        _light: Option<u32>,
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
            || source_node_count != self.expected.authored_node_count()
            || source_node_count != self.expected.canonical_order().len()
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
        Some(Some(self.expected.source_root_count()))
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

    fn validate_node_spatial_attachments(
        &mut self,
        row: usize,
        camera: Option<u32>,
        light: Option<u32>,
    ) -> Option<()> {
        let source_row = row.checked_sub(1)?;
        let local = *self.expected.canonical_order().get(source_row)?;
        let expected = *self.expected.input().nodes().get(local)?;
        let expected_camera = match expected.camera() {
            Presence::Absent => None,
            Presence::Present(camera) => Some(camera.index()),
        };
        let expected_light = match expected.light() {
            Presence::Absent => None,
            Presence::Present(light) => Some(light.index()),
        };
        (camera == expected_camera && light == expected_light).then_some(())
    }
}

#[derive(Clone, Copy)]
struct NodeCanonicalStructure {
    directory: CanonicalRecordRef<SceneDirectorySchema>,
    nodes: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    references: Option<CanonicalRecordRef<NodeSceneChildReferenceTableSchema>>,
    transforms: Option<CanonicalSequenceRef<f32, NodeSceneAuthoredTransformSchema>>,
    spatial: SpatialValidatedRefs,
    appearance_directory: CanonicalRecordRef<AppearanceDirectorySchema>,
    geometry_directory: CanonicalRecordRef<SceneGeometryDirectorySchema>,
    deformation_directory: CanonicalRecordRef<DeformationDirectorySchema>,
    animation_directory: CanonicalRecordRef<AnimationDirectorySchema>,
    profile_directory: CanonicalRecordRef<ProfileDirectorySchema>,
    source_node_count: usize,
    source_root_count: usize,
    source_edge_count: usize,
}

#[derive(Clone, Copy)]
struct NodeMeteredStructure<'session> {
    directory: CanonicalValidationMember<'session, RecordBlock, SceneDirectorySchema>,
    nodes: CanonicalValidationMember<'session, RecordBlock, NodeSceneNodeTableSchema>,
    references: Option<
        CanonicalValidationMember<'session, RecordBlock, NodeSceneChildReferenceTableSchema>,
    >,
    transforms: Option<
        CanonicalValidationMember<'session, ScalarSequence<f32>, NodeSceneAuthoredTransformSchema>,
    >,
    spatial: SpatialValidatedMembers<'session>,
    appearance_directory:
        CanonicalValidationMember<'session, RecordBlock, AppearanceDirectorySchema>,
    geometry_directory:
        CanonicalValidationMember<'session, RecordBlock, SceneGeometryDirectorySchema>,
    deformation_directory:
        CanonicalValidationMember<'session, RecordBlock, DeformationDirectorySchema>,
    animation_directory: CanonicalValidationMember<'session, RecordBlock, AnimationDirectorySchema>,
    profile_directory: CanonicalValidationMember<'session, RecordBlock, ProfileDirectorySchema>,
    source_node_count: usize,
    source_root_count: usize,
    source_edge_count: usize,
}

pub(in crate::canonical_scene::store) struct NodeSceneValidationScratch<'scratch> {
    camera_placement_counts: &'scratch mut [u32],
    light_placement_counts: &'scratch mut [u32],
    deformation_palette_maxima: &'scratch mut [Option<u32>],
    deformation_uniqueness_marks: &'scratch mut [u64],
    animation_target_marks: &'scratch mut [u32],
    profile_provenance_depths: &'scratch mut [u32],
}

#[derive(Clone, Copy)]
struct NodeSceneValidationScratchRequirements {
    camera_placement_counts: usize,
    light_placement_counts: usize,
    deformation_palette_marks: usize,
    deformation_uniqueness_marks: usize,
    animation_target_marks: usize,
    profile_provenance_depths: usize,
}

impl<'scratch> NodeSceneValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) const fn new(
        camera_placement_counts: &'scratch mut [u32],
        light_placement_counts: &'scratch mut [u32],
        deformation_palette_maxima: &'scratch mut [Option<u32>],
        deformation_uniqueness_marks: &'scratch mut [u64],
        animation_target_marks: &'scratch mut [u32],
        profile_provenance_depths: &'scratch mut [u32],
    ) -> Self {
        Self {
            camera_placement_counts,
            light_placement_counts,
            deformation_palette_maxima,
            deformation_uniqueness_marks,
            animation_target_marks,
            profile_provenance_depths,
        }
    }

    pub(in crate::canonical_scene::store) fn required_deformation_palette_marks(
        counts: SceneEvidenceCounts,
    ) -> Option<usize> {
        usize::try_from(counts.logical_mesh_count).ok()
    }

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
            camera_placement_counts: Self::required_camera_placement_counts(counts)?,
            light_placement_counts: Self::required_light_placement_counts(counts)?,
            deformation_palette_marks: Self::required_deformation_palette_marks(counts)?,
            deformation_uniqueness_marks,
            animation_target_marks: Self::required_animation_target_marks(counts)?,
            profile_provenance_depths: usize::try_from(counts.profile_provenance_count).ok()?,
        })
    }

    fn validate_known_capacity(
        &self,
        requirements: NodeSceneValidationScratchRequirements,
    ) -> Option<()> {
        (self.deformation_palette_maxima.len() >= requirements.deformation_palette_marks
            && self.camera_placement_counts.len() >= requirements.camera_placement_counts
            && self.light_placement_counts.len() >= requirements.light_placement_counts
            && self.deformation_uniqueness_marks.len() >= requirements.deformation_uniqueness_marks
            && self.animation_target_marks.len() >= requirements.animation_target_marks
            && self.profile_provenance_depths.len() >= requirements.profile_provenance_depths)
            .then_some(())
    }

    fn initialize(
        &mut self,
        requirements: NodeSceneValidationScratchRequirements,
        observe_work: &mut impl FnMut(u64) -> Option<()>,
    ) -> Option<()> {
        for count in self
            .camera_placement_counts
            .get_mut(..requirements.camera_placement_counts)?
        {
            observe_work(1)?;
            *count = 0;
        }
        for count in self
            .light_placement_counts
            .get_mut(..requirements.light_placement_counts)?
        {
            observe_work(1)?;
            *count = 0;
        }
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
        for depth in self
            .profile_provenance_depths
            .get_mut(..requirements.profile_provenance_depths)?
        {
            observe_work(1)?;
            *depth = 0;
        }
        Some(())
    }
}

/// Immutable semantic authority minted only by central node-scene validation.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneValidationProof {
    directory: CanonicalBlockRef<SceneDirectorySchema>,
    spatial: SpatialValidatedRefs,
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
    profile: ProfileValidatedRefs,
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

    pub(in crate::canonical_scene::store) const fn spatial_directory(
        self,
    ) -> CanonicalBlockRef<SpatialDirectorySchema> {
        self.spatial.directory
    }

    pub(in crate::canonical_scene::store) const fn cameras(
        self,
    ) -> Option<CanonicalBlockRef<SpatialCameraTableSchema>> {
        self.spatial.cameras
    }

    pub(in crate::canonical_scene::store) const fn lights(
        self,
    ) -> Option<CanonicalBlockRef<SpatialLightTableSchema>> {
        self.spatial.lights
    }

    pub(in crate::canonical_scene::store) const fn tangent_basis_schema_name(
        self,
    ) -> Option<CanonicalBlockRef<SpatialTangentBasisSchemaNameSchema>> {
        self.spatial.tangent_basis_schema_name
    }

    pub(in crate::canonical_scene::store) const fn tangent_encoding_schema_name(
        self,
    ) -> Option<CanonicalBlockRef<SpatialTangentEncodingSchemaNameSchema>> {
        self.spatial.tangent_encoding_schema_name
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

    pub(in crate::canonical_scene::store) const fn profile_directory(
        self,
    ) -> CanonicalBlockRef<ProfileDirectorySchema> {
        self.profile.directory
    }

    pub(in crate::canonical_scene::store) const fn profile_segments(
        self,
    ) -> CanonicalBlockRef<ProfileSegmentTableSchema> {
        self.profile.segments
    }

    pub(in crate::canonical_scene::store) fn profile_segment_count(self) -> usize {
        count_as_usize(self.profile.segment_count)
    }

    pub(in crate::canonical_scene::store) const fn profile_resource_count(self) -> u32 {
        self.profile.resource_count
    }

    pub(in crate::canonical_scene::store) const fn profile_source_count(self) -> u32 {
        self.profile.source_count
    }

    pub(in crate::canonical_scene::store) const fn profile_provenance_count(self) -> u32 {
        self.profile.provenance_count
    }

    pub(in crate::canonical_scene::store) const fn profile_operation_count(self) -> u32 {
        self.profile.operation_count
    }

    pub(in crate::canonical_scene::store) const fn profile_evidence_count(self) -> u32 {
        self.profile.evidence_count
    }

    pub(in crate::canonical_scene::store) const fn profile_parent_count(self) -> u32 {
        self.profile.parent_count
    }

    pub(in crate::canonical_scene::store) const fn profile_wide_count(self) -> u32 {
        self.profile.wide_count
    }

    pub(in crate::canonical_scene::store) const fn profile_lineage_text_bytes(self) -> u32 {
        self.profile.lineage_text_bytes
    }

    pub(in crate::canonical_scene::store) const fn profile_lineage_digest_bytes(self) -> u32 {
        self.profile.lineage_digest_bytes
    }

    pub(in crate::canonical_scene::store) const fn profile_owners(
        self,
    ) -> CanonicalBlockRef<ProfileOwnerTableSchema> {
        self.profile.owners
    }

    pub(in crate::canonical_scene::store) const fn profile_metadata_directory(
        self,
    ) -> CanonicalBlockRef<ProfileMetadataDirectorySchema> {
        self.profile.metadata_directory
    }

    pub(in crate::canonical_scene::store) const fn profile_metadata_entries(
        self,
    ) -> Option<CanonicalBlockRef<ProfileMetadataEntryTableSchema>> {
        self.profile.metadata_entries
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

    pub(in crate::canonical_scene::store) fn camera_count(self) -> usize {
        count_as_usize(self.counts.camera_count)
    }

    pub(in crate::canonical_scene::store) fn light_count(self) -> usize {
        count_as_usize(self.counts.light_count)
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
fn validate_node_canonical_structure<SR>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    expected_node_count: Option<u32>,
    expected_spatial: Option<&ValidatedSpatialBatch<'_, SR>>,
    expected_camera_count: u32,
    expected_light_count: u32,
    mut camera_placement_counts: Option<&mut [u32]>,
    mut light_placement_counts: Option<&mut [u32]>,
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
    let spatial_directory =
        directory.record_ref::<SpatialDirectorySchema>(0, scene_directory_field::SPATIAL)?;
    let spatial = match expected_spatial {
        Some(expected) => {
            validate_spatial_graph(session, spatial_directory, expected, observe_work)?
        }
        None => validate_spatial_canonical_structure(
            session,
            spatial_directory,
            expected_camera_count,
            expected_light_count,
            observe_work,
        )?,
    };

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
    require_scalar(
        nodes.scalar::<u8>(0, node_scene_node_field::CAMERA_PRESENCE),
        PRESENCE_ABSENT,
    )?;
    require_scalar(nodes.scalar::<u8>(0, node_scene_node_field::CAMERA), 0)?;
    require_scalar(nodes.scalar::<u8>(0, node_scene_node_field::CAMERAS), 0)?;
    require_scalar(
        nodes.scalar::<u8>(0, node_scene_node_field::LIGHT_PRESENCE),
        PRESENCE_ABSENT,
    )?;
    require_scalar(nodes.scalar::<u8>(0, node_scene_node_field::LIGHT), 0)?;
    require_scalar(nodes.scalar::<u8>(0, node_scene_node_field::LIGHTS), 0)?;

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

        let camera = validate_node_spatial_attachment::<SpatialCameraTableSchema>(
            nodes,
            row,
            node_scene_node_field::CAMERA_PRESENCE,
            node_scene_node_field::CAMERA,
            node_scene_node_field::CAMERAS,
            spatial.cameras,
            spatial.camera_count,
            camera_placement_counts.as_deref_mut(),
        )?;
        let light = validate_node_spatial_attachment::<SpatialLightTableSchema>(
            nodes,
            row,
            node_scene_node_field::LIGHT_PRESENCE,
            node_scene_node_field::LIGHT,
            node_scene_node_field::LIGHTS,
            spatial.lights,
            spatial.light_count,
            light_placement_counts.as_deref_mut(),
        )?;
        equivalence.validate_node_spatial_attachments(row, camera, light)?;
    }

    require_scalar(Some(child_reference_offset), source_node_count_u32)?;
    require_scalar(
        directory.scalar::<u32>(0, scene_directory_field::SOURCE_ROOT_COUNT),
        source_root_count_u32,
    )?;
    if has_source_nodes && transform_offset != transforms?.len() {
        return None;
    }
    if let Some(counts) = camera_placement_counts {
        for &placements in counts.get(..usize::try_from(spatial.camera_count).ok()?)? {
            observe_work(1)?;
            if placements == 0 {
                return None;
            }
        }
    }
    if let Some(counts) = light_placement_counts {
        for &placements in counts.get(..usize::try_from(spatial.light_count).ok()?)? {
            observe_work(1)?;
            if placements == 0 {
                return None;
            }
        }
    }

    let appearance_directory =
        directory.record_ref::<AppearanceDirectorySchema>(0, scene_directory_field::APPEARANCE)?;
    let geometry_directory =
        directory.record_ref::<SceneGeometryDirectorySchema>(0, scene_directory_field::GEOMETRY)?;
    let deformation_directory = directory
        .record_ref::<DeformationDirectorySchema>(0, scene_directory_field::DEFORMATION)?;
    let animation_directory =
        directory.record_ref::<AnimationDirectorySchema>(0, scene_directory_field::ANIMATION)?;
    let profile_directory =
        directory.record_ref::<ProfileDirectorySchema>(0, scene_directory_field::PROFILE)?;
    Some(NodeCanonicalStructure {
        directory: directory_ref,
        nodes: nodes_ref,
        references: references_ref,
        transforms: transforms_ref,
        spatial,
        appearance_directory,
        geometry_directory,
        deformation_directory,
        animation_directory,
        profile_directory,
        source_node_count,
        source_root_count,
        source_edge_count: source_node_count.checked_sub(source_root_count)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_node_spatial_attachment<Tag>(
    nodes: baozi_operation::CanonicalRecordView<'_>,
    row: usize,
    presence_field: usize,
    local_field: usize,
    table_field: usize,
    expected_table: Option<CanonicalBlockRef<Tag>>,
    expected_count: u32,
    placement_counts: Option<&mut [u32]>,
) -> Option<Option<u32>>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::RecordBlock>,
{
    match nodes.scalar::<u8>(row, presence_field)? {
        PRESENCE_ABSENT => {
            require_scalar(nodes.scalar::<u8>(row, local_field), 0_u8)?;
            require_scalar(nodes.scalar::<u8>(row, table_field), 0_u8)?;
            Some(None)
        }
        PRESENCE_PRESENT => {
            let local = nodes.scalar::<u32>(row, local_field)?;
            if local >= expected_count {
                return None;
            }
            require_scalar(
                nodes.record_ref::<Tag>(row, table_field),
                expected_table?.into_store(),
            )?;
            if let Some(counts) = placement_counts {
                let count = counts.get_mut(usize::try_from(local).ok()?)?;
                *count = count.checked_add(1)?;
            }
            Some(Some(local))
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::canonical_scene::store) fn validate_node_scene_graph<
    SR,
    NR,
    AR,
    GR,
    DR,
    ANR,
    PR,
    E,
>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    expected_spatial: &ValidatedSpatialBatch<'_, SR>,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected_appearance: &ValidatedAppearanceBatch<'_, AR>,
    expected_geometry: &ValidatedGeometryBatch<'_, GR>,
    expected_deformation: &ValidatedDeformationBatch<'_, DR>,
    expected_animation: &ValidatedAnimationBatch<'_, ANR>,
    expected_profile: &ValidatedProfileBatch<'_, PR>,
    expected_counts: SceneEvidenceCounts,
    profile_scratch: &mut ProfileValidationScratch<'_>,
    mut observe_work: impl FnMut(u64) -> Result<(), E>,
) -> Result<Option<NodeSceneValidationProof>, E> {
    let mut work_failure = None;
    let proof = validate_node_scene_graph_inner(
        session,
        directory_ref,
        evidence_ref,
        expected_spatial,
        expected_nodes,
        expected_appearance,
        expected_geometry,
        expected_deformation,
        expected_animation,
        expected_profile,
        expected_counts,
        profile_scratch,
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
fn validate_node_canonical_structure_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: CanonicalValidationMember<'session, RecordBlock, SceneDirectorySchema>,
    expected_counts: SceneEvidenceCounts,
    scratch: &mut NodeSceneMeteredValidationScratch<'_, 'session>,
) -> Result<Option<NodeMeteredStructure<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut root_node = None;
    let mut node_count = None;
    let mut nodes = None;
    let mut reference_presence = None;
    let mut reference_count = None;
    let mut references = None;
    let mut transform_presence = None;
    let mut transform_scalar_count = None;
    let mut transforms = None;
    let mut source_root_count = None;
    let mut spatial = None;
    let mut appearance = None;
    let mut geometry = None;
    let mut deformation = None;
    let mut animation = None;
    let mut profile = None;
    let directory_valid = session.visit_canonical_records(
        directory,
        1,
        SCENE_DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            let Some(schema_revision) =
                row.scalar(scene_directory_field::SCHEMA_REVISION, |value: u32| value)?
            else {
                return Ok(false);
            };
            root_node = row.scalar(scene_directory_field::ROOT_NODE, |value: u32| value)?;
            node_count = row.scalar(scene_directory_field::NODE_COUNT, |value: u32| value)?;
            nodes = row.record_ref::<NodeSceneNodeTableSchema, _>(
                scene_directory_field::NODES,
                |value| value,
            )?;
            reference_presence =
                row.scalar(scene_directory_field::REFERENCE_PRESENCE, |value: u8| value)?;
            reference_count =
                row.scalar(scene_directory_field::REFERENCE_COUNT, |value: u32| value)?;
            references = match reference_presence {
                Some(PRESENCE_PRESENT) => row.record_ref::<NodeSceneChildReferenceTableSchema, _>(
                    scene_directory_field::REFERENCES,
                    |value| value,
                )?,
                Some(PRESENCE_ABSENT) => {
                    let Some(absent) =
                        row.scalar(scene_directory_field::REFERENCES, |value: u8| value)?
                    else {
                        return Ok(false);
                    };
                    if absent != 0 {
                        return Ok(false);
                    }
                    None
                }
                _ => return Ok(false),
            };
            transform_presence =
                row.scalar(scene_directory_field::TRANSFORM_PRESENCE, |value: u8| value)?;
            transform_scalar_count = row.scalar(
                scene_directory_field::TRANSFORM_SCALAR_COUNT,
                |value: u32| value,
            )?;
            transforms = match transform_presence {
                Some(PRESENCE_PRESENT) => row
                    .sequence_ref::<f32, NodeSceneAuthoredTransformSchema, _>(
                        scene_directory_field::TRANSFORMS,
                        |value| value,
                    )?,
                Some(PRESENCE_ABSENT) => {
                    let Some(absent) =
                        row.scalar(scene_directory_field::TRANSFORMS, |value: u8| value)?
                    else {
                        return Ok(false);
                    };
                    if absent != 0 {
                        return Ok(false);
                    }
                    None
                }
                _ => return Ok(false),
            };
            source_root_count =
                row.scalar(scene_directory_field::SOURCE_ROOT_COUNT, |value: u32| value)?;
            spatial = row.record_ref::<SpatialDirectorySchema, _>(
                scene_directory_field::SPATIAL,
                |value| value,
            )?;
            appearance = row.record_ref::<AppearanceDirectorySchema, _>(
                scene_directory_field::APPEARANCE,
                |value| value,
            )?;
            geometry = row.record_ref::<SceneGeometryDirectorySchema, _>(
                scene_directory_field::GEOMETRY,
                |value| value,
            )?;
            deformation = row.record_ref::<DeformationDirectorySchema, _>(
                scene_directory_field::DEFORMATION,
                |value| value,
            )?;
            animation = row.record_ref::<AnimationDirectorySchema, _>(
                scene_directory_field::ANIMATION,
                |value| value,
            )?;
            profile = row.record_ref::<ProfileDirectorySchema, _>(
                scene_directory_field::PROFILE,
                |value| value,
            )?;
            Ok(schema_revision == SCENE_SCHEMA_REVISION)
        },
    )?;
    let (
        Some(root_node),
        Some(node_count),
        Some(nodes),
        Some(reference_presence),
        Some(reference_count),
        Some(transform_presence),
        Some(transform_scalar_count),
        Some(source_root_count),
        Some(spatial),
        Some(appearance_directory),
        Some(geometry_directory),
        Some(deformation_directory),
        Some(animation_directory),
        Some(profile_directory),
    ) = (
        root_node,
        node_count,
        nodes,
        reference_presence,
        reference_count,
        transform_presence,
        transform_scalar_count,
        source_root_count,
        spatial,
        appearance,
        geometry,
        deformation,
        animation,
        profile,
    )
    else {
        return Ok(None);
    };
    let source_node_count = node_count.checked_sub(1);
    let Some(source_node_count) = source_node_count else {
        return Ok(None);
    };
    let has_source_nodes = source_node_count != 0;
    let expected_presence = if has_source_nodes {
        PRESENCE_PRESENT
    } else {
        PRESENCE_ABSENT
    };
    if !directory_valid
        || root_node != 0
        || node_count != expected_counts.node_count
        || reference_presence != expected_presence
        || transform_presence != expected_presence
        || reference_count != source_node_count
        || references.is_some() != has_source_nodes
        || transforms.is_some() != has_source_nodes
        || source_root_count > source_node_count
    {
        return Ok(None);
    }
    let source_node_count_usize = match usize::try_from(source_node_count) {
        Ok(count) => count,
        Err(_) => return Ok(None),
    };
    let source_root_count_usize = match usize::try_from(source_root_count) {
        Ok(count) => count,
        Err(_) => return Ok(None),
    };
    if scratch.node_transform_kinds.len()
        != match usize::try_from(node_count) {
            Ok(count) => count,
            Err(_) => return Ok(None),
        }
    {
        return Ok(None);
    }

    let Some(spatial) = validate_spatial_canonical_structure_metered(
        session,
        spatial,
        expected_counts.camera_count,
        expected_counts.light_count,
    )?
    else {
        return Ok(None);
    };

    if let Some(references) = references {
        let references_valid =
            session.visit_canonical_records(
                references,
                source_node_count_usize,
                NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT,
                |row_index, row| {
                    let Some(reference) = row
                        .scalar(node_scene_child_reference_field::REFERENCE, |value: u32| {
                            value
                        })?
                    else {
                        return Ok(false);
                    };
                    let Some(child) =
                        row.scalar(node_scene_child_reference_field::CHILD, |value: u32| value)?
                    else {
                        return Ok(false);
                    };
                    let Ok(reference_expected) = u32::try_from(row_index) else {
                        return Ok(false);
                    };
                    Ok(reference == reference_expected
                        && child == reference_expected.saturating_add(1))
                },
            )?;
        if !references_valid {
            return Ok(None);
        }
    }

    let mut child_reference_offset = source_root_count;
    let mut transform_offset = 0_u32;
    let nodes_valid = session.visit_canonical_records(
        nodes,
        usize::try_from(node_count).unwrap_or(usize::MAX),
        NODE_SCENE_NODE_FIELD_COUNT,
        |row_index, row| {
            let Some(node) = row.scalar(node_scene_node_field::NODE, |value: u32| value)? else {
                return Ok(false);
            };
            let Some(children_presence) =
                row.scalar(node_scene_node_field::CHILDREN_PRESENCE, |value: u8| value)?
            else {
                return Ok(false);
            };
            let Some(child_offset) = row.scalar(
                node_scene_node_field::CHILD_REFERENCE_OFFSET,
                |value: u32| value,
            )?
            else {
                return Ok(false);
            };
            let Some(child_count) = row.scalar(
                node_scene_node_field::CHILD_REFERENCE_COUNT,
                |value: u32| value,
            )?
            else {
                return Ok(false);
            };
            let Some(transform_kind) =
                row.scalar(node_scene_node_field::TRANSFORM_KIND, |value: u8| value)?
            else {
                return Ok(false);
            };
            let Some(stored_transform_offset) =
                row.scalar(node_scene_node_field::TRANSFORM_OFFSET, |value: u32| value)?
            else {
                return Ok(false);
            };
            let Some(stored_transform_length) =
                row.scalar(node_scene_node_field::TRANSFORM_LENGTH, |value: u32| value)?
            else {
                return Ok(false);
            };
            let transforms_match = if row_index == 0 {
                row.scalar(node_scene_node_field::TRANSFORMS, |value: u8| value)? == Some(0)
            } else {
                row.sequence_ref::<f32, NodeSceneAuthoredTransformSchema, _>(
                    node_scene_node_field::TRANSFORMS,
                    |member| Some(member) == transforms,
                )? == Some(true)
            };
            let Some(camera_presence) =
                row.scalar(node_scene_node_field::CAMERA_PRESENCE, |value: u8| value)?
            else {
                return Ok(false);
            };
            let (camera, camera_cell_valid, cameras_match) = match camera_presence {
                PRESENCE_ABSENT => (
                    0,
                    row.scalar(node_scene_node_field::CAMERA, |value: u8| value)? == Some(0),
                    row.scalar(node_scene_node_field::CAMERAS, |value: u8| value)? == Some(0),
                ),
                PRESENCE_PRESENT => {
                    let camera = row.scalar(node_scene_node_field::CAMERA, |value: u32| value)?;
                    let cameras_match = row.record_ref::<SpatialCameraTableSchema, _>(
                        node_scene_node_field::CAMERAS,
                        |member| Some(member) == spatial.cameras,
                    )? == Some(true);
                    (camera.unwrap_or(0), camera.is_some(), cameras_match)
                }
                _ => (
                    0,
                    row.scalar(node_scene_node_field::CAMERA, |_: u8| false)? == Some(true),
                    row.scalar(node_scene_node_field::CAMERAS, |_: u8| false)? == Some(true),
                ),
            };
            let Some(light_presence) =
                row.scalar(node_scene_node_field::LIGHT_PRESENCE, |value: u8| value)?
            else {
                return Ok(false);
            };
            let (light, light_cell_valid, lights_match) = match light_presence {
                PRESENCE_ABSENT => (
                    0,
                    row.scalar(node_scene_node_field::LIGHT, |value: u8| value)? == Some(0),
                    row.scalar(node_scene_node_field::LIGHTS, |value: u8| value)? == Some(0),
                ),
                PRESENCE_PRESENT => {
                    let light = row.scalar(node_scene_node_field::LIGHT, |value: u32| value)?;
                    let lights_match = row.record_ref::<SpatialLightTableSchema, _>(
                        node_scene_node_field::LIGHTS,
                        |member| Some(member) == spatial.lights,
                    )? == Some(true);
                    (light.unwrap_or(0), light.is_some(), lights_match)
                }
                _ => (
                    0,
                    row.scalar(node_scene_node_field::LIGHT, |_: u8| false)? == Some(true),
                    row.scalar(node_scene_node_field::LIGHTS, |_: u8| false)? == Some(true),
                ),
            };
            let Ok(row_u32) = u32::try_from(row_index) else {
                return Ok(false);
            };
            let Some(kind_slot) = scratch.node_transform_kinds.get_mut(row_index) else {
                return Ok(false);
            };
            *kind_slot = transform_kind;
            if row_index == 0 {
                return Ok(node == 0
                    && children_presence == PRESENCE_PRESENT
                    && child_offset == 0
                    && child_count == source_root_count
                    && transform_kind == NODE_SCENE_TRANSFORM_SYNTHETIC
                    && stored_transform_offset == 0
                    && stored_transform_length == 0
                    && transforms_match
                    && camera_presence == PRESENCE_ABSENT
                    && camera_cell_valid
                    && cameras_match
                    && light_presence == PRESENCE_ABSENT
                    && light_cell_valid
                    && lights_match);
            }
            let Some(width) = authored_transform_scalar_count(transform_kind) else {
                return Ok(false);
            };
            let Ok(width) = u32::try_from(width) else {
                return Ok(false);
            };
            let topology_valid = node == row_u32
                && children_presence == PRESENCE_PRESENT
                && child_offset == child_reference_offset
                && (child_count == 0 || child_reference_offset.saturating_add(1) > row_u32)
                && stored_transform_offset == transform_offset
                && stored_transform_length == width
                && transforms_match;
            child_reference_offset = match child_reference_offset.checked_add(child_count) {
                Some(next) => next,
                None => return Ok(false),
            };
            transform_offset = match transform_offset.checked_add(width) {
                Some(next) => next,
                None => return Ok(false),
            };
            let camera_valid = match camera_presence {
                PRESENCE_ABSENT => camera_cell_valid && cameras_match,
                PRESENCE_PRESENT => {
                    let Ok(index) = usize::try_from(camera) else {
                        return Ok(false);
                    };
                    let Some(count) = scratch.camera_placement_counts.get_mut(index) else {
                        return Ok(false);
                    };
                    *count = match count.checked_add(1) {
                        Some(count) => count,
                        None => return Ok(false),
                    };
                    camera_cell_valid && camera < expected_counts.camera_count && cameras_match
                }
                _ => false,
            };
            let light_valid = match light_presence {
                PRESENCE_ABSENT => light_cell_valid && lights_match,
                PRESENCE_PRESENT => {
                    let Ok(index) = usize::try_from(light) else {
                        return Ok(false);
                    };
                    let Some(count) = scratch.light_placement_counts.get_mut(index) else {
                        return Ok(false);
                    };
                    *count = match count.checked_add(1) {
                        Some(count) => count,
                        None => return Ok(false),
                    };
                    light_cell_valid && light < expected_counts.light_count && lights_match
                }
                _ => false,
            };
            Ok(topology_valid && camera_valid && light_valid)
        },
    )?;
    if !nodes_valid
        || child_reference_offset != source_node_count
        || transform_offset != transform_scalar_count
    {
        return Ok(None);
    }

    if let Some(transforms) = transforms {
        let transform_len = match usize::try_from(transform_scalar_count) {
            Ok(count) => count,
            Err(_) => return Ok(None),
        };
        let mut node_index = 1_usize;
        let mut local_index = 0_usize;
        let mut values = [0.0_f32; 16];
        let transforms_valid =
            session.visit_canonical_sequence(transforms, transform_len, |_index, value| {
                let Some(&kind) = scratch.node_transform_kinds.get(node_index) else {
                    return false;
                };
                let Some(width) = authored_transform_scalar_count(kind) else {
                    return false;
                };
                let Some(slot) = values.get_mut(local_index) else {
                    return false;
                };
                *slot = value;
                local_index += 1;
                if local_index == width {
                    if decode_authored_transform(kind, &values[..width]).is_none() {
                        return false;
                    }
                    node_index += 1;
                    local_index = 0;
                }
                true
            })?;
        if !transforms_valid
            || node_index != usize::try_from(node_count).unwrap_or(usize::MAX)
            || local_index != 0
        {
            return Ok(None);
        }
    } else if transform_scalar_count != 0 {
        return Ok(None);
    }

    for &placements in scratch.camera_placement_counts.iter() {
        let Some(valid) = session.run_validation_unit(|| placements != 0)? else {
            return Ok(None);
        };
        if !valid {
            return Ok(None);
        }
    }
    for &placements in scratch.light_placement_counts.iter() {
        let Some(valid) = session.run_validation_unit(|| placements != 0)? else {
            return Ok(None);
        };
        if !valid {
            return Ok(None);
        }
    }

    Ok(Some(NodeMeteredStructure {
        directory,
        nodes,
        references,
        transforms,
        spatial,
        appearance_directory,
        geometry_directory,
        deformation_directory,
        animation_directory,
        profile_directory,
        source_node_count: source_node_count_usize,
        source_root_count: source_root_count_usize,
        source_edge_count: source_node_count_usize.saturating_sub(source_root_count_usize),
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
    validation_plan: NodeSceneValidationScratchPlan,
    scratch: &mut NodeSceneMeteredValidationScratch<'_, 'session>,
) -> Result<Option<NodeSceneValidationProof>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(node) =
        validate_node_canonical_structure_metered(session, directory, expected_counts, scratch)?
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

    let geometry_requirements = validation_plan.geometry_requirements();
    let geometry_counts = GeometryCanonicalCounts::new(
        expected_counts.logical_mesh_count,
        expected_counts.primitive_count,
        expected_counts.attribute_count,
        expected_counts.mesh_instance_count,
    );
    let Some(geometry) = validate_geometry_canonical_structure_metered(
        session,
        node.geometry_directory,
        GeometryCanonicalValidationDependencies::new(
            node.nodes,
            expected_counts.node_count,
            appearance.materials,
            expected_counts.material_count,
        ),
        geometry_counts,
        &mut scratch.geometry,
    )?
    else {
        return Ok(None);
    };
    let Some(geometry_logical_meshes) = scratch
        .geometry
        .logical_mesh_projections(geometry_requirements)
    else {
        return Ok(None);
    };
    let Some(geometry_primitives) = scratch
        .geometry
        .primitive_projections(geometry_requirements)
    else {
        return Ok(None);
    };
    let Some(geometry_mesh_instances) = scratch
        .geometry
        .mesh_instance_projections(geometry_requirements)
    else {
        return Ok(None);
    };

    let deformation_requirements = validation_plan.deformation_requirements();
    let Some(deformation) = validate_deformation_graph_canonical_metered(
        session,
        node.deformation_directory,
        DeformationValidationDependenciesMetered {
            nodes: node.nodes,
            node_count: expected_counts.node_count,
            logical_meshes: geometry.members.logical_meshes,
            primitives: geometry.members.primitives,
            mesh_instances: geometry.members.mesh_instances,
            geometry: DeformationGeometryValidationProjection {
                logical_meshes: geometry_logical_meshes,
                primitives: geometry_primitives,
                mesh_instances: geometry_mesh_instances,
            },
        },
        &mut scratch.deformation,
    )?
    else {
        return Ok(None);
    };
    let Some(logical_morph_projections) = scratch
        .deformation
        .logical_morph_projections(deformation_requirements)
    else {
        return Ok(None);
    };

    let animation_counts = validation_plan.animation_counts();
    let Some(animation) = validate_animation_graph_metered(
        session,
        node.animation_directory,
        AnimationMeteredDependencies::new(
            node.nodes,
            expected_counts.node_count,
            scratch.node_transform_kinds,
            deformation.members.mesh_instances,
            deformation.mesh_instance_count,
            geometry.members.mesh_instances,
            expected_counts.mesh_instance_count,
            deformation.members.logical_morphs,
            deformation.logical_morph_count,
            geometry_mesh_instances,
            logical_morph_projections,
        ),
        animation_counts,
        &mut scratch.animation,
    )?
    else {
        return Ok(None);
    };

    let profile_counts = validation_plan.profile_counts();
    let Some(profile) = validate_profile_graph_metered(
        session,
        node.profile_directory,
        profile_counts,
        &mut scratch.profile,
    )?
    else {
        return Ok(None);
    };
    if !validate_node_scene_success_evidence_metered(session, evidence, expected_counts)? {
        return Ok(None);
    }

    let source_node_count = expected_counts.node_count.checked_sub(1);
    let Some(source_node_count) = source_node_count else {
        return Ok(None);
    };
    for (actual, expected) in [
        (node.spatial.camera_count, expected_counts.camera_count),
        (node.spatial.light_count, expected_counts.light_count),
        (deformation.skin_count, expected_counts.skin_count),
        (
            deformation.logical_morph_count,
            expected_counts.logical_morph_count,
        ),
        (
            deformation.morph_definition_count,
            expected_counts.morph_definition_count,
        ),
        (
            deformation.primitive_count,
            expected_counts.primitive_deformation_count,
        ),
        (
            deformation.morph_target_count,
            expected_counts.morph_target_count,
        ),
        (
            deformation.morph_delta_count,
            expected_counts.morph_delta_count,
        ),
        (
            deformation.mesh_instance_count,
            expected_counts.mesh_instance_count,
        ),
        (animation.clip_count, expected_counts.animation_clip_count),
        (
            animation.channel_count,
            expected_counts.animation_channel_count,
        ),
        (profile.segment_count, expected_counts.profile_segment_count),
        (
            profile.resource_count,
            expected_counts.profile_resource_count,
        ),
        (profile.source_count, expected_counts.profile_source_count),
        (
            profile.provenance_count,
            expected_counts.profile_provenance_count,
        ),
        (
            profile.operation_count,
            expected_counts.profile_operation_count,
        ),
        (profile.parent_count, expected_counts.profile_parent_count),
        (profile.owner_count, expected_counts.profile_owner_count),
        (
            profile.attachment_count,
            expected_counts.profile_attachment_count,
        ),
        (
            profile.metadata_entry_count,
            expected_counts.profile_metadata_entry_count,
        ),
    ] {
        if actual != expected {
            return Ok(None);
        }
    }
    for (actual, expected) in [
        (profile.owner_classes.nodes, source_node_count),
        (profile.owner_classes.cameras, expected_counts.camera_count),
        (profile.owner_classes.lights, expected_counts.light_count),
        (profile.owner_classes.images, expected_counts.image_count),
        (
            profile.owner_classes.samplers,
            expected_counts.sampler_count,
        ),
        (
            profile.owner_classes.textures,
            expected_counts.texture_count,
        ),
        (
            profile.owner_classes.texture_bindings,
            expected_counts.material_binding_count,
        ),
        (
            profile.owner_classes.materials,
            expected_counts.material_count,
        ),
        (
            profile.owner_classes.logical_meshes,
            expected_counts.logical_mesh_count,
        ),
        (
            profile.owner_classes.primitives,
            expected_counts.primitive_count,
        ),
        (
            profile.owner_classes.attributes,
            expected_counts.attribute_count,
        ),
        (
            profile.owner_classes.mesh_instances,
            expected_counts.mesh_instance_count,
        ),
        (profile.owner_classes.skins, expected_counts.skin_count),
        (
            profile.owner_classes.morph_definitions,
            expected_counts.morph_definition_count,
        ),
        (
            profile.owner_classes.morph_deltas,
            expected_counts.morph_delta_count,
        ),
        (
            profile.owner_classes.animation_clips,
            expected_counts.animation_clip_count,
        ),
        (
            profile.owner_classes.animation_channels,
            expected_counts.animation_channel_count,
        ),
    ] {
        if actual != expected {
            return Ok(None);
        }
    }

    let spatial_payload_count = [
        node.spatial.tangent_basis_schema_name.is_some(),
        node.spatial.tangent_encoding_schema_name.is_some(),
    ]
    .into_iter()
    .map(u32::from)
    .sum::<u32>();
    let appearance_payload_count = [
        appearance.image_payloads.is_some(),
        appearance.media_types.is_some(),
        appearance.source_keys.is_some(),
        appearance.binding_semantic_scalars.is_some(),
        appearance.material_factor_scalars.is_some(),
    ]
    .into_iter()
    .map(u32::from)
    .sum::<u32>();
    let domain_payload_count = spatial_payload_count
        .checked_add(appearance_payload_count)
        .and_then(|count| count.checked_add(geometry.payload_block_count))
        .and_then(|count| count.checked_add(deformation.payload_block_count))
        .and_then(|count| count.checked_add(animation.payload_block_count));
    let Some(domain_payload_count) = domain_payload_count else {
        return Ok(None);
    };
    if domain_payload_count.checked_add(profile.payload_leaf_count)
        != Some(expected_counts.payload_block_count)
        || domain_payload_count.checked_add(profile.shareable_leaf_count)
            != Some(expected_counts.shareable_leaf_block_count)
    {
        return Ok(None);
    }
    let canonical_block_count = 2_u32
        .checked_add(u32::from(node.references.is_some()))
        .and_then(|count| count.checked_add(u32::from(node.transforms.is_some())))
        .and_then(|count| {
            count.checked_add(
                1 + [
                    node.spatial.cameras.is_some(),
                    node.spatial.lights.is_some(),
                    node.spatial.tangent_basis_schema_name.is_some(),
                    node.spatial.tangent_encoding_schema_name.is_some(),
                ]
                .into_iter()
                .map(u32::from)
                .sum::<u32>(),
            )
        })
        .and_then(|count| {
            count.checked_add(
                1 + [
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
                ]
                .into_iter()
                .map(u32::from)
                .sum::<u32>(),
            )
        })
        .and_then(|count| {
            count.checked_add(
                1 + [
                    geometry.members.logical_meshes.is_some(),
                    geometry.members.primitives.is_some(),
                    geometry.members.attributes.is_some(),
                    geometry.members.mesh_instances.is_some(),
                ]
                .into_iter()
                .map(u32::from)
                .sum::<u32>()
                    + geometry.payload_block_count,
            )
        })
        .and_then(|count| {
            count.checked_add(
                1 + [
                    deformation.members.skins.is_some(),
                    deformation.members.logical_morphs.is_some(),
                    deformation.members.morph_definitions.is_some(),
                    deformation.members.primitives.is_some(),
                    deformation.members.morph_targets.is_some(),
                    deformation.members.morph_deltas.is_some(),
                    deformation.members.mesh_instances.is_some(),
                ]
                .into_iter()
                .map(u32::from)
                .sum::<u32>()
                    + deformation.payload_block_count,
            )
        })
        .and_then(|count| {
            count.checked_add(
                1 + [
                    animation.members.clips.is_some(),
                    animation.members.channels.is_some(),
                ]
                .into_iter()
                .map(u32::from)
                .sum::<u32>()
                    + animation.payload_block_count,
            )
        })
        .and_then(|count| count.checked_add(profile.canonical_block_count));
    if canonical_block_count != Some(expected_counts.canonical_block_count) {
        return Ok(None);
    }

    let identities_match = node
        .directory
        .identifies(expected_refs.directory.into_store())
        && node.nodes.identifies(expected_refs.nodes.into_store())
        && optional_member_identifies(node.references, expected_refs.references)
        && optional_member_identifies(node.transforms, expected_refs.transforms)
        && node
            .spatial
            .directory
            .identifies(expected_refs.spatial.directory.into_store())
        && optional_member_identifies(node.spatial.cameras, expected_refs.spatial.cameras)
        && optional_member_identifies(node.spatial.lights, expected_refs.spatial.lights)
        && optional_member_identifies(
            node.spatial.tangent_basis_schema_name,
            expected_refs.spatial.tangent_basis_schema_name,
        )
        && optional_member_identifies(
            node.spatial.tangent_encoding_schema_name,
            expected_refs.spatial.tangent_encoding_schema_name,
        )
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
            geometry.members.logical_meshes,
            expected_refs.geometry.tables.logical_meshes,
        )
        && optional_member_identifies(
            geometry.members.primitives,
            expected_refs.geometry.tables.primitives,
        )
        && optional_member_identifies(
            geometry.members.attributes,
            expected_refs.geometry.tables.attributes,
        )
        && optional_member_identifies(
            geometry.members.mesh_instances,
            expected_refs.geometry.tables.mesh_instances,
        )
        && node
            .geometry_directory
            .identifies(expected_refs.geometry.directory.into_store())
        && node
            .deformation_directory
            .identifies(expected_refs.deformation.directory.into_store())
        && optional_member_identifies(
            deformation.members.skins,
            expected_refs.deformation.tables.skins,
        )
        && optional_member_identifies(
            deformation.members.logical_morphs,
            expected_refs.deformation.tables.logical_morphs,
        )
        && optional_member_identifies(
            deformation.members.morph_definitions,
            expected_refs.deformation.tables.morph_definitions,
        )
        && optional_member_identifies(
            deformation.members.primitives,
            expected_refs.deformation.tables.primitives,
        )
        && optional_member_identifies(
            deformation.members.morph_targets,
            expected_refs.deformation.tables.morph_targets,
        )
        && optional_member_identifies(
            deformation.members.morph_deltas,
            expected_refs.deformation.tables.morph_deltas,
        )
        && optional_member_identifies(
            deformation.members.mesh_instances,
            expected_refs.deformation.tables.mesh_instances,
        )
        && optional_member_identifies(
            deformation.members.joint_nodes,
            expected_refs.deformation.payloads.joint_nodes,
        )
        && optional_member_identifies(
            deformation.members.inverse_bind_matrices,
            expected_refs.deformation.payloads.inverse_bind_matrices,
        )
        && optional_member_identifies(
            deformation.members.skin_correction_matrices,
            expected_refs.deformation.payloads.skin_correction_matrices,
        )
        && optional_member_identifies(
            deformation.members.skin_names,
            expected_refs.deformation.payloads.skin_names,
        )
        && optional_member_identifies(
            deformation.members.influence_row_offsets,
            expected_refs.deformation.payloads.influence_row_offsets,
        )
        && optional_member_identifies(
            deformation.members.influence_joints,
            expected_refs.deformation.payloads.influence_joints,
        )
        && optional_member_identifies(
            deformation.members.influence_weights,
            expected_refs.deformation.payloads.influence_weights,
        )
        && optional_member_identifies(
            deformation.members.influence_source_orders,
            expected_refs.deformation.payloads.influence_source_orders,
        )
        && optional_member_identifies(
            deformation.members.morph_defaults,
            expected_refs.deformation.payloads.morph_defaults,
        )
        && optional_member_identifies(
            deformation.members.morph_names,
            expected_refs.deformation.payloads.morph_names,
        )
        && optional_member_identifies(
            deformation.members.morph_values,
            expected_refs.deformation.payloads.morph_values,
        )
        && optional_member_identifies(
            deformation.members.morph_sparse_indices,
            expected_refs.deformation.payloads.morph_sparse_indices,
        )
        && optional_member_identifies(
            deformation.members.instance_morph_weights,
            expected_refs.deformation.payloads.instance_morph_weights,
        )
        && node
            .animation_directory
            .identifies(expected_refs.animation.directory.into_store())
        && optional_member_identifies(animation.members.clips, expected_refs.animation.clips)
        && optional_member_identifies(animation.members.channels, expected_refs.animation.channels)
        && optional_member_identifies(
            animation.members.times,
            expected_refs.animation.payloads.times,
        )
        && optional_member_identifies(
            animation.members.values,
            expected_refs.animation.payloads.values,
        )
        && optional_member_identifies(
            animation.members.in_tangents,
            expected_refs.animation.payloads.in_tangents,
        )
        && optional_member_identifies(
            animation.members.out_tangents,
            expected_refs.animation.payloads.out_tangents,
        )
        && profile
            .directory
            .identifies(expected_refs.profile.directory.into_store())
        && profile
            .segments
            .identifies(expected_refs.profile.segments.into_store())
        && profile
            .owners
            .identifies(expected_refs.profile.owners.into_store())
        && optional_member_identifies(profile.resources, expected_refs.profile.lineage.resources)
        && optional_member_identifies(profile.sources, expected_refs.profile.lineage.sources)
        && optional_member_identifies(profile.provenance, expected_refs.profile.lineage.provenance)
        && optional_member_identifies(profile.operations, expected_refs.profile.lineage.operations)
        && optional_member_identifies(profile.evidence, expected_refs.profile.lineage.evidence)
        && optional_member_identifies(profile.parents, expected_refs.profile.lineage.parents)
        && optional_member_identifies(profile.wide, expected_refs.profile.lineage.wide)
        && optional_member_identifies(profile.lineage_text, expected_refs.profile.lineage.text)
        && optional_member_identifies(
            profile.lineage_digests,
            expected_refs.profile.lineage.digests,
        )
        && expected_refs
            .profile
            .lineage
            .attachments
            .is_some_and(|slot| profile.attachments.identifies(slot.into_store()))
        && profile
            .metadata_directory
            .identifies(expected_refs.profile.metadata.directory.into_store())
        && optional_member_identifies(
            profile.metadata_entries,
            expected_refs.profile.metadata.leaves.entries,
        )
        && optional_member_identifies(
            profile.metadata_text,
            expected_refs.profile.metadata.leaves.text,
        )
        && optional_member_identifies(
            profile.metadata_bytes,
            expected_refs.profile.metadata.leaves.bytes,
        )
        && optional_member_identifies(
            profile.metadata_bools,
            expected_refs.profile.metadata.leaves.bools,
        )
        && optional_member_identifies(
            profile.metadata_i64s,
            expected_refs.profile.metadata.leaves.i64s,
        )
        && optional_member_identifies(
            profile.metadata_u64s,
            expected_refs.profile.metadata.leaves.u64s,
        )
        && optional_member_identifies(
            profile.metadata_f64s,
            expected_refs.profile.metadata.leaves.f64s,
        )
        && optional_member_identifies(
            profile.metadata_f32s,
            expected_refs.profile.metadata.leaves.f32s,
        );
    if !identities_match {
        return Ok(None);
    }
    let Some(profile_attachments) = expected_refs.profile.lineage.attachments else {
        return Ok(None);
    };

    Ok(Some(NodeSceneValidationProof {
        directory: expected_refs.directory,
        spatial: SpatialValidatedRefs {
            directory: expected_refs.spatial.directory,
            cameras: expected_refs.spatial.cameras,
            lights: expected_refs.spatial.lights,
            tangent_basis_schema_name: expected_refs.spatial.tangent_basis_schema_name,
            tangent_encoding_schema_name: expected_refs.spatial.tangent_encoding_schema_name,
            camera_count: node.spatial.camera_count,
            light_count: node.spatial.light_count,
        },
        appearance_directory: expected_refs.appearance.directory,
        geometry_directory: expected_refs.geometry.directory,
        deformation_directory: expected_refs.deformation.directory,
        animation_directory: expected_refs.animation.directory,
        nodes: expected_refs.nodes,
        references: expected_refs.references,
        transforms: expected_refs.transforms,
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
        },
        geometry: GeometryValidatedRefs {
            logical_meshes: expected_refs.geometry.tables.logical_meshes,
            primitives: expected_refs.geometry.tables.primitives,
            attributes: expected_refs.geometry.tables.attributes,
            mesh_instances: expected_refs.geometry.tables.mesh_instances,
        },
        deformation: DeformationValidatedRefs {
            skins: expected_refs.deformation.tables.skins,
            logical_morphs: expected_refs.deformation.tables.logical_morphs,
            morph_definitions: expected_refs.deformation.tables.morph_definitions,
            primitives: expected_refs.deformation.tables.primitives,
            morph_targets: expected_refs.deformation.tables.morph_targets,
            morph_deltas: expected_refs.deformation.tables.morph_deltas,
            mesh_instances: expected_refs.deformation.tables.mesh_instances,
            joint_nodes: expected_refs.deformation.payloads.joint_nodes,
            inverse_bind_matrices: expected_refs.deformation.payloads.inverse_bind_matrices,
            skin_correction_matrices: expected_refs.deformation.payloads.skin_correction_matrices,
            skin_names: expected_refs.deformation.payloads.skin_names,
            influence_row_offsets: expected_refs.deformation.payloads.influence_row_offsets,
            influence_joints: expected_refs.deformation.payloads.influence_joints,
            influence_weights: expected_refs.deformation.payloads.influence_weights,
            influence_source_orders: expected_refs.deformation.payloads.influence_source_orders,
            morph_defaults: expected_refs.deformation.payloads.morph_defaults,
            morph_names: expected_refs.deformation.payloads.morph_names,
            morph_values: expected_refs.deformation.payloads.morph_values,
            morph_sparse_indices: expected_refs.deformation.payloads.morph_sparse_indices,
            instance_morph_weights: expected_refs.deformation.payloads.instance_morph_weights,
        },
        animation: AnimationValidatedRefs {
            clips: expected_refs.animation.clips,
            channels: expected_refs.animation.channels,
            times: expected_refs.animation.payloads.times,
            values: expected_refs.animation.payloads.values,
            in_tangents: expected_refs.animation.payloads.in_tangents,
            out_tangents: expected_refs.animation.payloads.out_tangents,
        },
        profile: ProfileValidatedRefs {
            directory: expected_refs.profile.directory,
            segments: expected_refs.profile.segments,
            owners: expected_refs.profile.owners,
            attachments: profile_attachments,
            metadata_directory: expected_refs.profile.metadata.directory,
            metadata_entries: expected_refs.profile.metadata.leaves.entries,
            metadata_text: expected_refs.profile.metadata.leaves.text,
            metadata_bytes: expected_refs.profile.metadata.leaves.bytes,
            metadata_bools: expected_refs.profile.metadata.leaves.bools,
            metadata_i64s: expected_refs.profile.metadata.leaves.i64s,
            metadata_u64s: expected_refs.profile.metadata.leaves.u64s,
            metadata_f64s: expected_refs.profile.metadata.leaves.f64s,
            metadata_f32s: expected_refs.profile.metadata.leaves.f32s,
            segment_count: profile.segment_count,
            resource_count: profile.resource_count,
            source_count: profile.source_count,
            provenance_count: profile.provenance_count,
            operation_count: profile.operation_count,
            evidence_count: profile.evidence_count,
            parent_count: profile.parent_count,
            wide_count: profile.wide_count,
            lineage_text_bytes: profile.lineage_text_bytes,
            lineage_digest_bytes: profile.lineage_digest_bytes,
            attachment_count: profile.attachment_count,
            owner_count: profile.owner_count,
            metadata_entry_count: profile.metadata_entry_count,
            metadata_text_bytes: profile.metadata_text_bytes,
            metadata_payload_bytes: profile.metadata_payload_bytes,
            metadata_bool_count: profile.metadata_bool_count,
            metadata_i64_count: profile.metadata_i64_count,
            metadata_u64_count: profile.metadata_u64_count,
            metadata_f64_count: profile.metadata_f64_count,
            metadata_f32_count: profile.metadata_f32_count,
            canonical_block_count: profile.canonical_block_count,
            payload_leaf_count: profile.payload_leaf_count,
            shareable_leaf_count: profile.shareable_leaf_count,
            maximum_provenance_depth: profile.maximum_provenance_depth,
            owner_classes: profile.owner_classes,
        },
        source_node_count: node.source_node_count,
        source_root_count: node.source_root_count,
        source_edge_count: node.source_edge_count,
        deformation_uniqueness_mark_count: deformation.maximum_uniqueness_count,
        counts: expected_counts,
    }))
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
    let camera_placement_counts = scratch
        .camera_placement_counts
        .get_mut(..scratch_requirements.camera_placement_counts)?;
    let light_placement_counts = scratch
        .light_placement_counts
        .get_mut(..scratch_requirements.light_placement_counts)?;
    let node_structure = validate_node_canonical_structure::<()>(
        session,
        directory_ref,
        Some(expected_counts.node_count),
        None,
        expected_counts.camera_count,
        expected_counts.light_count,
        Some(camera_placement_counts),
        Some(light_placement_counts),
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
    let profile = validate_profile_canonical_structure(
        session,
        node_structure.profile_directory,
        &mut ProfileValidationScratch::new(
            scratch
                .profile_provenance_depths
                .get_mut(..scratch_requirements.profile_provenance_depths)?,
        ),
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
        profile,
    )?;
    validate_node_scene_success_evidence(session, evidence_ref, expected_counts, observe_work)?;
    Some(assemble_node_scene_validation_proof(
        node_structure,
        appearance,
        geometry.refs,
        deformation,
        animation.refs,
        profile,
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
        && evidence.scalar::<u32>(0, node_scene_evidence_field::CAMERA_COUNT)
            == Some(counts.camera_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::LIGHT_COUNT)
            == Some(counts.light_count)
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
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_SEGMENT_COUNT)
            == Some(counts.profile_segment_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_RESOURCE_COUNT)
            == Some(counts.profile_resource_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_SOURCE_COUNT)
            == Some(counts.profile_source_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_PROVENANCE_COUNT)
            == Some(counts.profile_provenance_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_OPERATION_COUNT)
            == Some(counts.profile_operation_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_PARENT_COUNT)
            == Some(counts.profile_parent_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_OWNER_COUNT)
            == Some(counts.profile_owner_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_ATTACHMENT_COUNT)
            == Some(counts.profile_attachment_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PROFILE_METADATA_ENTRY_COUNT)
            == Some(counts.profile_metadata_entry_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::SHAREABLE_LEAF_BLOCK_COUNT)
            == Some(counts.shareable_leaf_block_count)
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
        (node_scene_evidence_field::CAMERA_COUNT, counts.camera_count),
        (node_scene_evidence_field::LIGHT_COUNT, counts.light_count),
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
        (
            node_scene_evidence_field::PROFILE_SEGMENT_COUNT,
            counts.profile_segment_count,
        ),
        (
            node_scene_evidence_field::PROFILE_RESOURCE_COUNT,
            counts.profile_resource_count,
        ),
        (
            node_scene_evidence_field::PROFILE_SOURCE_COUNT,
            counts.profile_source_count,
        ),
        (
            node_scene_evidence_field::PROFILE_PROVENANCE_COUNT,
            counts.profile_provenance_count,
        ),
        (
            node_scene_evidence_field::PROFILE_OPERATION_COUNT,
            counts.profile_operation_count,
        ),
        (
            node_scene_evidence_field::PROFILE_PARENT_COUNT,
            counts.profile_parent_count,
        ),
        (
            node_scene_evidence_field::PROFILE_OWNER_COUNT,
            counts.profile_owner_count,
        ),
        (
            node_scene_evidence_field::PROFILE_ATTACHMENT_COUNT,
            counts.profile_attachment_count,
        ),
        (
            node_scene_evidence_field::PROFILE_METADATA_ENTRY_COUNT,
            counts.profile_metadata_entry_count,
        ),
        (
            node_scene_evidence_field::SHAREABLE_LEAF_BLOCK_COUNT,
            counts.shareable_leaf_block_count,
        ),
    ] {
        require_scalar(evidence.scalar::<u32>(0, field), expected)?;
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_canonical_evidence_counts(
    counts: SceneEvidenceCounts,
    canonical_unique_block_count: u32,
    node: NodeCanonicalStructure,
    appearance: AppearanceValidatedRefs,
    geometry: GeometryValidatedShape,
    deformation: DeformationValidatedShape,
    animation: AnimationValidatedShape,
    profile: ProfileValidatedRefs,
) -> Option<()> {
    for (actual, expected) in [
        (node.spatial.camera_count, counts.camera_count),
        (node.spatial.light_count, counts.light_count),
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
        (profile.segment_count, counts.profile_segment_count),
        (profile.resource_count, counts.profile_resource_count),
        (profile.source_count, counts.profile_source_count),
        (profile.provenance_count, counts.profile_provenance_count),
        (profile.operation_count, counts.profile_operation_count),
        (profile.parent_count, counts.profile_parent_count),
        (profile.owner_count, counts.profile_owner_count),
        (profile.attachment_count, counts.profile_attachment_count),
        (
            profile.metadata_entry_count,
            counts.profile_metadata_entry_count,
        ),
    ] {
        require_scalar(Some(actual), expected)?;
    }
    let source_node_count = counts.node_count.checked_sub(1)?;
    for (actual, expected) in [
        (profile.owner_classes.nodes, source_node_count),
        (profile.owner_classes.cameras, counts.camera_count),
        (profile.owner_classes.lights, counts.light_count),
        (profile.owner_classes.images, counts.image_count),
        (profile.owner_classes.samplers, counts.sampler_count),
        (profile.owner_classes.textures, counts.texture_count),
        (
            profile.owner_classes.texture_bindings,
            counts.material_binding_count,
        ),
        (profile.owner_classes.materials, counts.material_count),
        (
            profile.owner_classes.logical_meshes,
            counts.logical_mesh_count,
        ),
        (profile.owner_classes.primitives, counts.primitive_count),
        (profile.owner_classes.attributes, counts.attribute_count),
        (
            profile.owner_classes.mesh_instances,
            counts.mesh_instance_count,
        ),
        (profile.owner_classes.skins, counts.skin_count),
        (
            profile.owner_classes.morph_definitions,
            counts.morph_definition_count,
        ),
        (profile.owner_classes.morph_deltas, counts.morph_delta_count),
        (
            profile.owner_classes.animation_clips,
            counts.animation_clip_count,
        ),
        (
            profile.owner_classes.animation_channels,
            counts.animation_channel_count,
        ),
    ] {
        require_scalar(Some(actual), expected)?;
    }

    let spatial_payload_block_count = present_count(&[
        node.spatial.tangent_basis_schema_name.is_some(),
        node.spatial.tangent_encoding_schema_name.is_some(),
    ])?;
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
    let domain_payload_block_count = spatial_payload_block_count
        .checked_add(appearance_payload_block_count)?
        .checked_add(geometry.payload_block_count)?
        .checked_add(deformation_payload_block_count)?
        .checked_add(animation_payload_block_count)?;
    let payload_block_count = domain_payload_block_count.checked_add(profile.payload_leaf_count)?;
    require_scalar(Some(payload_block_count), counts.payload_block_count)?;
    let shareable_leaf_block_count =
        domain_payload_block_count.checked_add(profile.shareable_leaf_count)?;
    require_scalar(
        Some(shareable_leaf_block_count),
        counts.shareable_leaf_block_count,
    )?;

    let node_block_count = 2_u32
        .checked_add(u32::from(node.references.is_some()))?
        .checked_add(u32::from(node.transforms.is_some()))?;
    let spatial_block_count = 1_u32.checked_add(present_count(&[
        node.spatial.cameras.is_some(),
        node.spatial.lights.is_some(),
        node.spatial.tangent_basis_schema_name.is_some(),
        node.spatial.tangent_encoding_schema_name.is_some(),
    ])?)?;
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
        .checked_add(spatial_block_count)?
        .checked_add(appearance_block_count)?
        .checked_add(geometry_block_count)?
        .checked_add(deformation_block_count)?
        .checked_add(animation_block_count)?
        .checked_add(profile.canonical_block_count)?;
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
    profile: ProfileValidatedRefs,
    counts: SceneEvidenceCounts,
) -> NodeSceneValidationProof {
    NodeSceneValidationProof {
        directory: CanonicalBlockRef::from_store(node.directory),
        spatial: node.spatial,
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
        profile,
        source_node_count: node.source_node_count,
        source_root_count: node.source_root_count,
        source_edge_count: node.source_edge_count,
        deformation_uniqueness_mark_count: deformation.maximum_uniqueness_count,
        counts,
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_node_scene_graph_inner<SR, NR, AR, GR, DR, ANR, PR>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    expected_spatial: &ValidatedSpatialBatch<'_, SR>,
    expected: &ValidatedNodeBatch<'_, NR>,
    expected_appearance: &ValidatedAppearanceBatch<'_, AR>,
    expected_geometry: &ValidatedGeometryBatch<'_, GR>,
    expected_deformation: &ValidatedDeformationBatch<'_, DR>,
    expected_animation: &ValidatedAnimationBatch<'_, ANR>,
    expected_profile: &ValidatedProfileBatch<'_, PR>,
    expected_counts: SceneEvidenceCounts,
    profile_scratch: &mut ProfileValidationScratch<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<NodeSceneValidationProof> {
    let node_structure = validate_node_canonical_structure(
        session,
        directory_ref,
        None,
        Some(expected_spatial),
        expected_spatial.camera_count(),
        expected_spatial.light_count(),
        None,
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
    let profile = validate_profile_graph(
        session,
        node_structure.profile_directory,
        expected_profile,
        profile_scratch,
        observe_work,
    )?;

    if !evidence_counts_match_batches(
        expected_counts,
        expected_spatial,
        expected,
        expected_appearance,
        expected_geometry,
        expected_deformation,
        expected_animation,
        expected_profile,
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
        profile,
        expected_counts,
    ))
}

#[allow(clippy::too_many_arguments)]
fn evidence_counts_match_batches<SR, NR, AR, GR, DR, ANR, PR>(
    counts: SceneEvidenceCounts,
    spatial: &ValidatedSpatialBatch<'_, SR>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    appearance: &ValidatedAppearanceBatch<'_, AR>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    deformation: &ValidatedDeformationBatch<'_, DR>,
    animation: &ValidatedAnimationBatch<'_, ANR>,
    profile: &ValidatedProfileBatch<'_, PR>,
) -> bool {
    let payload_block_count = spatial
        .payload_block_count()
        .checked_add(appearance.payload_block_count())
        .and_then(|count| count.checked_add(geometry.payload_block_count()))
        .and_then(|count| count.checked_add(deformation.payload_block_count()))
        .and_then(|count| count.checked_add(animation.payload_block_count()))
        .and_then(|count| count.checked_add(profile.retention_shape().payload_block_count()));
    let profile_shape = profile.retention_shape();
    let domain_payload_block_count = spatial
        .payload_block_count()
        .checked_add(appearance.payload_block_count())
        .and_then(|count| count.checked_add(geometry.payload_block_count()))
        .and_then(|count| count.checked_add(deformation.payload_block_count()))
        .and_then(|count| count.checked_add(animation.payload_block_count()));
    let shareable_leaf_block_count = domain_payload_block_count
        .and_then(|count| count.checked_add(profile_shape.shareable_leaf_block_count()));
    u32::try_from(nodes.canonical_node_count()) == Ok(counts.node_count)
        && spatial.camera_count() == counts.camera_count
        && spatial.light_count() == counts.light_count
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
        && profile.segment_count() == counts.profile_segment_count
        && profile.resource_count() == counts.profile_resource_count
        && profile.source_count() == counts.profile_source_count
        && profile.provenance_node_count() == counts.profile_provenance_count
        && profile.operation_count() == counts.profile_operation_count
        && u64::from(counts.profile_parent_count) == profile.parent_edge_count()
        && profile.owner_count() == u64::from(counts.profile_owner_count)
        && profile.owner_attachment_count() == u64::from(counts.profile_attachment_count)
        && profile.metadata_entry_count() == counts.profile_metadata_entry_count
        && payload_block_count == Some(counts.payload_block_count)
        && shareable_leaf_block_count == Some(counts.shareable_leaf_block_count)
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
        let mut cameras = [];
        let mut lights = [];
        let mut palette = [Some(7)];
        let mut uniqueness = [11_u64, 12];
        let mut animation = [21_u32];
        let mut profile_depths = [];
        let scratch = NodeSceneValidationScratch::new(
            &mut cameras,
            &mut lights,
            &mut palette,
            &mut uniqueness,
            &mut animation,
            &mut profile_depths,
        );
        let counts = SceneEvidenceCounts {
            canonical_block_count: 0,
            node_count: 0,
            camera_count: 0,
            light_count: 0,
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
            profile_segment_count: 0,
            profile_resource_count: 0,
            profile_source_count: 0,
            profile_provenance_count: 0,
            profile_operation_count: 0,
            profile_parent_count: 0,
            profile_owner_count: 0,
            profile_attachment_count: 0,
            profile_metadata_entry_count: 0,
            shareable_leaf_block_count: 0,
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
        let mut cameras = [31_u32];
        let mut lights = [41_u32];
        let mut palette = [Some(7), Some(u32::MAX)];
        let mut uniqueness = [11_u64, 12, 13];
        let mut animation = [21_u32, 22];
        let mut profile_depths = [31_u32, 32];

        let mut observed = 0_u64;
        let requirements = NodeSceneValidationScratchRequirements {
            camera_placement_counts: 1,
            light_placement_counts: 1,
            deformation_palette_marks: 2,
            deformation_uniqueness_marks: 3,
            animation_target_marks: 2,
            profile_provenance_depths: 2,
        };
        {
            let mut scratch = NodeSceneValidationScratch::new(
                &mut cameras,
                &mut lights,
                &mut palette,
                &mut uniqueness,
                &mut animation,
                &mut profile_depths,
            );
            assert_eq!(
                scratch.initialize(requirements, &mut |amount| {
                    observed = observed.checked_add(amount)?;
                    (observed < 3).then_some(())
                }),
                None
            );
        }
        assert_eq!(cameras, [0]);
        assert_eq!(lights, [0]);
        assert_eq!(palette, [Some(7), Some(u32::MAX)]);
        assert_eq!(uniqueness, [11, 12, 13]);
        assert_eq!(animation, [21, 22]);
        assert_eq!(profile_depths, [31, 32]);

        observed = 0;
        {
            let mut scratch = NodeSceneValidationScratch::new(
                &mut cameras,
                &mut lights,
                &mut palette,
                &mut uniqueness,
                &mut animation,
                &mut profile_depths,
            );
            assert_eq!(
                scratch.initialize(requirements, &mut |amount| {
                    observed = observed.checked_add(amount)?;
                    Some(())
                }),
                Some(())
            );
        }
        assert_eq!(observed, 11);
        assert_eq!(cameras, [0]);
        assert_eq!(lights, [0]);
        assert_eq!(palette, [None, None]);
        assert_eq!(uniqueness, [0, 0, 0]);
        assert_eq!(animation, [0, 0]);
        assert_eq!(profile_depths, [0, 0]);
    }

    #[test]
    fn scratch_initialization_leaves_unqualified_capacity_untouched() {
        let mut cameras = [31_u32, 32];
        let mut lights = [41_u32, 42];
        let mut palette = [Some(7), Some(8)];
        let mut uniqueness = [11_u64, 12, 13];
        let mut animation = [21_u32, 22];
        let mut profile_depths = [51_u32, 52];
        let requirements = NodeSceneValidationScratchRequirements {
            camera_placement_counts: 1,
            light_placement_counts: 1,
            deformation_palette_marks: 1,
            deformation_uniqueness_marks: 2,
            animation_target_marks: 1,
            profile_provenance_depths: 1,
        };
        let mut observed = 0_u64;

        let mut scratch = NodeSceneValidationScratch::new(
            &mut cameras,
            &mut lights,
            &mut palette,
            &mut uniqueness,
            &mut animation,
            &mut profile_depths,
        );
        assert_eq!(
            scratch.initialize(requirements, &mut |amount| {
                observed = observed.checked_add(amount)?;
                Some(())
            }),
            Some(())
        );

        assert_eq!(observed, 7);
        assert_eq!(cameras, [0, 32]);
        assert_eq!(lights, [0, 42]);
        assert_eq!(palette, [None, Some(8)]);
        assert_eq!(uniqueness, [0, 0, 13]);
        assert_eq!(animation, [0, 22]);
        assert_eq!(profile_depths, [0, 52]);
    }
}
