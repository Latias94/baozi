use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef, CanonicalUtf8Ref, SequenceView,
    StoreValidationSession, StoredScalar,
};

use crate::canonical_scene::store::production::{
    DeformationMorphPayloadKind, MorphDeltaPayloadAuthoring, Presence, ValidatedDeformationBatch,
    ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::deformation::{
    DEFORMATION_DIRECTORY_FIELD_COUNT, DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
    DEFORMATION_MESH_INSTANCE_FIELD_COUNT, DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
    DEFORMATION_MORPH_DELTA_FIELD_COUNT, DEFORMATION_MORPH_DENSE, DEFORMATION_MORPH_NORMAL,
    DEFORMATION_MORPH_POSITION, DEFORMATION_MORPH_SPARSE, DEFORMATION_MORPH_TANGENT,
    DEFORMATION_MORPH_TARGET_FIELD_COUNT, DEFORMATION_PRIMITIVE_FIELD_COUNT,
    DEFORMATION_SCHEMA_REVISION, DEFORMATION_SKIN_FIELD_COUNT, DEFORMATION_SPACE_JOINT_LOCAL,
    DEFORMATION_SPACE_JOINT_LOCAL_BIND, DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
    DEFORMATION_SPACE_MESH_INSTANCE_LOCAL, DEFORMATION_SPACE_SCENE, DeformationDirectorySchema,
    DeformationInfluenceJointSchema, DeformationInfluenceRowOffsetSchema,
    DeformationInfluenceSourceOrderSchema, DeformationInfluenceWeightSchema,
    DeformationInstanceMorphWeightSchema, DeformationInverseBindMatrixSchema,
    DeformationLogicalMorphTableSchema, DeformationMeshInstanceTableSchema,
    DeformationMorphDefaultWeightSchema, DeformationMorphDefinitionTableSchema,
    DeformationMorphDeltaTableSchema, DeformationMorphNameSchema,
    DeformationMorphSparseIndexSchema, DeformationMorphTargetTableSchema,
    DeformationMorphValueSchema, DeformationPrimitiveTableSchema,
    DeformationSkinCorrectionMatrixSchema, DeformationSkinJointNodeSchema,
    DeformationSkinNameSchema, DeformationSkinTableSchema, deformation_directory_field,
    deformation_logical_morph_field, deformation_mesh_instance_field,
    deformation_morph_definition_field, deformation_morph_delta_field,
    deformation_morph_target_field, deformation_primitive_field, deformation_skin_field,
    encode_deformation_morph_payload, encode_deformation_morph_semantic,
    encode_deformation_skin_matrix_spaces, encode_deformation_skin_space,
};
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_LOGICAL_MESH_FIELD_COUNT, GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    GEOMETRY_PRIMITIVE_FIELD_COUNT, GeometryLogicalMeshTableSchema,
    GeometryMeshInstanceTableSchema, GeometryPrimitiveTableSchema, geometry_logical_mesh_field,
    geometry_mesh_instance_field, geometry_primitive_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, NODE_SCENE_NODE_FIELD_COUNT, NodeSceneNodeTableSchema, PRESENCE_ABSENT,
    PRESENCE_PRESENT,
};
use crate::entity::MAX_ENTITY_NAME_BYTES;

use super::super::require_scalar;

const MATRIX_SCALAR_COUNT: u32 = 16;
const MORPH_VECTOR_WIDTH: u32 = 3;

struct FreshDeformationEquivalence<'fresh, 'nodes, 'deformation, R, NR> {
    nodes: &'fresh ValidatedNodeBatch<'nodes, NR>,
    deformation: &'fresh ValidatedDeformationBatch<'deformation, R>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StoredPresence {
    Absent,
    Present,
}

struct DeformationUniquenessScratch<'scratch> {
    marks: &'scratch mut [u64],
    next_marker: u64,
}

impl<'scratch> DeformationUniquenessScratch<'scratch> {
    /// Reuses one centrally zeroed buffer by assigning each dense group a distinct marker.
    const fn new(marks: &'scratch mut [u64]) -> Self {
        Self {
            marks,
            next_marker: 1,
        }
    }

    fn begin_group(&mut self, width: usize) -> Option<DeformationUniquenessGroup<'_>> {
        let marker = self.next_marker;
        self.next_marker = self.next_marker.checked_add(1)?;
        Some(DeformationUniquenessGroup {
            marks: self.marks.get_mut(..width)?,
            marker,
        })
    }
}

struct DeformationUniquenessGroup<'scratch> {
    marks: &'scratch mut [u64],
    marker: u64,
}

impl DeformationUniquenessGroup<'_> {
    fn mark(&mut self, ordinal: u32) -> Option<()> {
        let mark = self.marks.get_mut(usize::try_from(ordinal).ok()?)?;
        if *mark == self.marker {
            return None;
        }
        *mark = self.marker;
        Some(())
    }
}

#[derive(Clone, Copy)]
struct DeformationDependencyViews<'storage> {
    nodes: CanonicalRecordView<'storage>,
    logical_meshes: Option<CanonicalRecordView<'storage>>,
    primitives: Option<CanonicalRecordView<'storage>>,
    mesh_instances: Option<CanonicalRecordView<'storage>>,
}

#[derive(Clone, Copy)]
struct DeformationPrimitiveViews<'storage> {
    primitives: Option<CanonicalRecordView<'storage>>,
    influence_joints: Option<SequenceView<'storage, u32>>,
    maximum_uniqueness_count: usize,
}

#[derive(Clone, Copy)]
pub(super) struct DeformationValidationDependencies {
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
}

impl DeformationValidationDependencies {
    pub(super) const fn new(
        nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
        logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
        primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
        mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
    ) -> Self {
        Self {
            nodes,
            logical_meshes,
            primitives,
            mesh_instances,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct DeformationValidatedRefs {
    pub(super) skins: Option<CanonicalBlockRef<DeformationSkinTableSchema>>,
    pub(super) logical_morphs: Option<CanonicalBlockRef<DeformationLogicalMorphTableSchema>>,
    pub(super) morph_definitions: Option<CanonicalBlockRef<DeformationMorphDefinitionTableSchema>>,
    pub(super) primitives: Option<CanonicalBlockRef<DeformationPrimitiveTableSchema>>,
    pub(super) morph_targets: Option<CanonicalBlockRef<DeformationMorphTargetTableSchema>>,
    pub(super) morph_deltas: Option<CanonicalBlockRef<DeformationMorphDeltaTableSchema>>,
    pub(super) mesh_instances: Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>>,
    pub(super) joint_nodes: Option<CanonicalBlockRef<DeformationSkinJointNodeSchema>>,
    pub(super) inverse_bind_matrices: Option<CanonicalBlockRef<DeformationInverseBindMatrixSchema>>,
    pub(super) skin_correction_matrices:
        Option<CanonicalBlockRef<DeformationSkinCorrectionMatrixSchema>>,
    pub(super) skin_names: Option<CanonicalBlockRef<DeformationSkinNameSchema>>,
    pub(super) influence_row_offsets:
        Option<CanonicalBlockRef<DeformationInfluenceRowOffsetSchema>>,
    pub(super) influence_joints: Option<CanonicalBlockRef<DeformationInfluenceJointSchema>>,
    pub(super) influence_weights: Option<CanonicalBlockRef<DeformationInfluenceWeightSchema>>,
    pub(super) influence_source_orders:
        Option<CanonicalBlockRef<DeformationInfluenceSourceOrderSchema>>,
    pub(super) morph_defaults: Option<CanonicalBlockRef<DeformationMorphDefaultWeightSchema>>,
    pub(super) morph_names: Option<CanonicalBlockRef<DeformationMorphNameSchema>>,
    pub(super) morph_values: Option<CanonicalBlockRef<DeformationMorphValueSchema>>,
    pub(super) morph_sparse_indices: Option<CanonicalBlockRef<DeformationMorphSparseIndexSchema>>,
    pub(super) instance_morph_weights:
        Option<CanonicalBlockRef<DeformationInstanceMorphWeightSchema>>,
}

#[derive(Clone, Copy)]
pub(super) struct DeformationValidatedShape {
    pub(super) refs: DeformationValidatedRefs,
    pub(super) payload_block_count: u32,
    pub(super) skin_count: u32,
    pub(super) logical_morph_count: u32,
    pub(super) morph_definition_count: u32,
    pub(super) primitive_count: u32,
    pub(super) morph_target_count: u32,
    pub(super) morph_delta_count: u32,
    pub(super) mesh_instance_count: u32,
    pub(super) maximum_uniqueness_count: usize,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_deformation_graph<R, NR>(
    session: &mut StoreValidationSession<'_, '_>,
    deformation_ref: CanonicalRecordRef<DeformationDirectorySchema>,
    dependencies: DeformationValidationDependencies,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationValidatedShape> {
    let fresh = FreshDeformationEquivalence {
        nodes: expected_nodes,
        deformation: expected,
    };
    validate_deformation_graph_impl(
        session,
        deformation_ref,
        dependencies,
        Some(&fresh),
        None,
        None,
        observe_work,
    )
}

pub(super) fn validate_deformation_graph_canonical(
    session: &mut StoreValidationSession<'_, '_>,
    deformation_ref: CanonicalRecordRef<DeformationDirectorySchema>,
    dependencies: DeformationValidationDependencies,
    palette_scratch: &mut [Option<u32>],
    uniqueness_scratch: &mut [u64],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationValidatedShape> {
    validate_deformation_graph_impl::<(), ()>(
        session,
        deformation_ref,
        dependencies,
        None,
        Some(palette_scratch),
        Some(uniqueness_scratch),
        observe_work,
    )
}

#[allow(clippy::too_many_lines)]
fn validate_deformation_graph_impl<R, NR>(
    session: &mut StoreValidationSession<'_, '_>,
    deformation_ref: CanonicalRecordRef<DeformationDirectorySchema>,
    dependencies: DeformationValidationDependencies,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    palette_scratch: Option<&mut [Option<u32>]>,
    uniqueness_scratch: Option<&mut [u64]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationValidatedShape> {
    let mut uniqueness_scratch = uniqueness_scratch.map(DeformationUniquenessScratch::new);
    observe_work(1)?;
    let directory = session.canonical_records(deformation_ref)?;
    if directory.rows() != 1 || directory.fields() != DEFORMATION_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, deformation_directory_field::SCHEMA_REVISION),
        DEFORMATION_SCHEMA_REVISION,
    )?;

    // These three counts qualify the cross-domain dependency shapes before any
    // deformation-owned reference is followed, preserving the fresh validation order.
    let dependency_logical_morph_count =
        directory.scalar::<u32>(0, deformation_directory_field::LOGICAL_MORPH_COUNT)?;
    let dependency_primitive_count =
        directory.scalar::<u32>(0, deformation_directory_field::PRIMITIVE_DEFORMATION_COUNT)?;
    let dependency_mesh_instance_count =
        directory.scalar::<u32>(0, deformation_directory_field::MESH_INSTANCE_COUNT)?;
    let dependency_views = validate_dependencies(
        session,
        dependencies,
        dependency_logical_morph_count,
        dependency_primitive_count,
        dependency_mesh_instance_count,
        fresh,
    )?;

    let (skin_count, skins) = optional_record_ref::<DeformationSkinTableSchema>(
        session,
        directory,
        deformation_directory_field::SKIN_COUNT,
        deformation_directory_field::SKINS,
        DEFORMATION_SKIN_FIELD_COUNT,
    )?;
    require_fresh_count(
        skin_count,
        fresh.map(|fresh| fresh.deformation.skin_count()),
    )?;
    let (joint_node_count, joint_nodes) =
        optional_sequence_ref::<u32, DeformationSkinJointNodeSchema>(
            session,
            directory,
            deformation_directory_field::JOINT_NODE_COUNT,
            deformation_directory_field::JOINT_NODES,
        )?;
    require_fresh_count(
        joint_node_count,
        fresh.map(|fresh| fresh.deformation.joint_node_count()),
    )?;
    let (inverse_bind_scalar_count, inverse_bind_matrices) =
        optional_sequence_ref::<f32, DeformationInverseBindMatrixSchema>(
            session,
            directory,
            deformation_directory_field::INVERSE_BIND_SCALAR_COUNT,
            deformation_directory_field::INVERSE_BIND_MATRICES,
        )?;
    require_fresh_count(
        inverse_bind_scalar_count,
        fresh.map(|fresh| fresh.deformation.inverse_bind_scalar_count()),
    )?;
    let (logical_morph_count, logical_morphs) =
        optional_record_ref::<DeformationLogicalMorphTableSchema>(
            session,
            directory,
            deformation_directory_field::LOGICAL_MORPH_COUNT,
            deformation_directory_field::LOGICAL_MORPHS,
            DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
        )?;
    require_scalar(Some(logical_morph_count), dependency_logical_morph_count)?;
    require_fresh_count(
        logical_morph_count,
        fresh.map(|fresh| fresh.deformation.logical_morph_count()),
    )?;
    let (primitive_count, primitives) = optional_record_ref::<DeformationPrimitiveTableSchema>(
        session,
        directory,
        deformation_directory_field::PRIMITIVE_DEFORMATION_COUNT,
        deformation_directory_field::PRIMITIVE_DEFORMATIONS,
        DEFORMATION_PRIMITIVE_FIELD_COUNT,
    )?;
    require_scalar(Some(primitive_count), dependency_primitive_count)?;
    require_fresh_count(
        primitive_count,
        fresh.map(|fresh| fresh.deformation.primitive_count()),
    )?;
    let (influence_row_offset_count, influence_row_offsets) =
        optional_sequence_ref::<u32, DeformationInfluenceRowOffsetSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_ROW_OFFSET_COUNT,
            deformation_directory_field::INFLUENCE_ROW_OFFSETS,
        )?;
    require_fresh_count(
        influence_row_offset_count,
        fresh.map(|fresh| fresh.deformation.influence_row_offset_count()),
    )?;
    let (influence_count, influence_joints) =
        optional_sequence_ref::<u32, DeformationInfluenceJointSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_COUNT,
            deformation_directory_field::INFLUENCE_JOINTS,
        )?;
    require_fresh_count(
        influence_count,
        fresh.map(|fresh| fresh.deformation.influence_count()),
    )?;
    let influence_weights =
        optional_sequence_ref_same_count::<f32, DeformationInfluenceWeightSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_WEIGHTS,
            influence_count,
        )?;
    let influence_source_orders =
        optional_sequence_ref_same_count::<u32, DeformationInfluenceSourceOrderSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_SOURCE_ORDERS,
            influence_count,
        )?;
    let (morph_target_count, morph_targets) =
        optional_record_ref::<DeformationMorphTargetTableSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_TARGET_COUNT,
            deformation_directory_field::MORPH_TARGETS,
            DEFORMATION_MORPH_TARGET_FIELD_COUNT,
        )?;
    require_fresh_count(
        morph_target_count,
        fresh.map(|fresh| fresh.deformation.morph_target_count()),
    )?;
    let (morph_delta_count, morph_deltas) = optional_record_ref::<DeformationMorphDeltaTableSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_DELTA_COUNT,
        deformation_directory_field::MORPH_DELTAS,
        DEFORMATION_MORPH_DELTA_FIELD_COUNT,
    )?;
    require_fresh_count(
        morph_delta_count,
        fresh.map(|fresh| fresh.deformation.morph_delta_count()),
    )?;
    let (morph_default_count, morph_defaults) =
        optional_sequence_ref::<f32, DeformationMorphDefaultWeightSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_DEFAULT_COUNT,
            deformation_directory_field::MORPH_DEFAULTS,
        )?;
    require_fresh_count(
        morph_default_count,
        fresh.map(|fresh| fresh.deformation.morph_default_weight_count()),
    )?;
    let (morph_value_count, morph_values) =
        optional_sequence_ref::<f32, DeformationMorphValueSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_VALUE_COUNT,
            deformation_directory_field::MORPH_VALUES,
        )?;
    require_fresh_count(
        morph_value_count,
        fresh.map(|fresh| fresh.deformation.morph_value_count()),
    )?;
    let (morph_sparse_index_count, morph_sparse_indices) =
        optional_sequence_ref::<u32, DeformationMorphSparseIndexSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_SPARSE_INDEX_COUNT,
            deformation_directory_field::MORPH_SPARSE_INDICES,
        )?;
    require_fresh_count(
        morph_sparse_index_count,
        fresh.map(|fresh| fresh.deformation.morph_sparse_index_count()),
    )?;
    let (mesh_instance_count, mesh_instances) =
        optional_record_ref::<DeformationMeshInstanceTableSchema>(
            session,
            directory,
            deformation_directory_field::MESH_INSTANCE_COUNT,
            deformation_directory_field::MESH_INSTANCES,
            DEFORMATION_MESH_INSTANCE_FIELD_COUNT,
        )?;
    require_scalar(Some(mesh_instance_count), dependency_mesh_instance_count)?;
    require_fresh_count(
        mesh_instance_count,
        fresh.map(|fresh| fresh.deformation.mesh_instance_count()),
    )?;
    let (instance_morph_weight_count, instance_morph_weights) =
        optional_sequence_ref::<f32, DeformationInstanceMorphWeightSchema>(
            session,
            directory,
            deformation_directory_field::INSTANCE_MORPH_WEIGHT_COUNT,
            deformation_directory_field::INSTANCE_MORPH_WEIGHTS,
        )?;
    require_fresh_count(
        instance_morph_weight_count,
        fresh.map(|fresh| fresh.deformation.instance_morph_weight_count()),
    )?;
    let skin_correction_matrices =
        optional_sequence_ref::<f32, DeformationSkinCorrectionMatrixSchema>(
            session,
            directory,
            deformation_directory_field::SKIN_CORRECTION_SCALAR_COUNT,
            deformation_directory_field::SKIN_CORRECTION_MATRICES,
        )?;
    let (skin_correction_scalar_count, skin_correction_matrices) = skin_correction_matrices;
    require_fresh_count(
        skin_correction_scalar_count,
        fresh.map(|fresh| fresh.deformation.skin_correction_scalar_count()),
    )?;
    let (morph_definition_count, morph_definitions) =
        optional_record_ref::<DeformationMorphDefinitionTableSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_DEFINITION_COUNT,
            deformation_directory_field::MORPH_DEFINITIONS,
            DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
        )?;
    require_fresh_count(
        morph_definition_count,
        fresh.map(|fresh| fresh.deformation.morph_definition_count()),
    )?;
    let (morph_name_byte_count, morph_names) = optional_utf8_ref::<DeformationMorphNameSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_NAME_BYTE_COUNT,
        deformation_directory_field::MORPH_NAMES,
    )?;
    require_fresh_count(
        morph_name_byte_count,
        fresh.map(|fresh| fresh.deformation.morph_name_byte_count()),
    )?;
    let (skin_name_byte_count, skin_names) = optional_utf8_ref::<DeformationSkinNameSchema>(
        session,
        directory,
        deformation_directory_field::SKIN_NAME_BYTE_COUNT,
        deformation_directory_field::SKIN_NAMES,
    )?;
    require_fresh_count(
        skin_name_byte_count,
        fresh.map(|fresh| fresh.deformation.skin_name_byte_count()),
    )?;

    let payload_block_count = [
        joint_nodes.is_some(),
        inverse_bind_matrices.is_some(),
        skin_correction_matrices.is_some(),
        skin_names.is_some(),
        influence_row_offsets.is_some(),
        influence_joints.is_some(),
        influence_weights.is_some(),
        influence_source_orders.is_some(),
        morph_defaults.is_some(),
        morph_names.is_some(),
        morph_values.is_some(),
        morph_sparse_indices.is_some(),
        instance_morph_weights.is_some(),
    ]
    .into_iter()
    .try_fold(0_u32, |count, present| {
        count.checked_add(u32::from(present))
    })?;
    require_fresh_count(
        payload_block_count,
        fresh.map(|fresh| fresh.deformation.payload_block_count()),
    )?;

    let skin_view = validate_skins(
        session,
        skins,
        joint_nodes,
        inverse_bind_matrices,
        skin_correction_matrices,
        skin_names,
        dependencies.nodes.into_store(),
        dependency_views.nodes,
        skin_count,
        joint_node_count,
        inverse_bind_scalar_count,
        skin_correction_scalar_count,
        skin_name_byte_count,
        fresh,
        observe_work,
    )?;
    let morph_definition_view = validate_morph_definitions(
        session,
        morph_definitions,
        morph_names,
        morph_definition_count,
        morph_name_byte_count,
        logical_morph_count,
        fresh,
        observe_work,
    )?;
    let (logical_morph_view, logical_morph_maximum_uniqueness_count) = validate_logical_morphs(
        session,
        logical_morphs,
        morph_definitions,
        morph_definition_view,
        morph_defaults,
        logical_morph_count,
        morph_definition_count,
        morph_default_count,
        fresh,
        uniqueness_scratch.as_mut(),
        observe_work,
    )?;
    let morph_target_view = validate_morph_deltas(
        session,
        morph_deltas,
        morph_values,
        morph_sparse_indices,
        morph_targets,
        dependency_views.primitives,
        morph_delta_count,
        morph_value_count,
        morph_sparse_index_count,
        fresh,
        observe_work,
    )?;
    let primitive_view = validate_morph_targets(
        session,
        morph_targets,
        morph_target_view,
        morph_deltas,
        primitives,
        morph_target_count,
        morph_delta_count,
        fresh,
        observe_work,
    )?;
    let primitive_views = validate_primitives(
        session,
        primitives,
        primitive_view,
        influence_row_offsets,
        influence_joints,
        influence_weights,
        influence_source_orders,
        morph_targets,
        dependency_views.primitives,
        primitive_count,
        influence_row_offset_count,
        influence_count,
        morph_target_count,
        fresh,
        uniqueness_scratch.as_mut(),
        observe_work,
    )?;
    let mesh_instance_view = validate_mesh_instances(
        session,
        mesh_instances,
        skins,
        instance_morph_weights,
        logical_morph_view,
        dependency_views.mesh_instances,
        skin_count,
        mesh_instance_count,
        instance_morph_weight_count,
        fresh,
        observe_work,
    )?;
    if fresh.is_none() {
        validate_canonical_cross_domain_semantics(
            dependency_views.logical_meshes,
            dependency_views.mesh_instances,
            logical_morph_view,
            primitive_views.primitives,
            mesh_instance_view,
            skin_view,
            primitive_views.influence_joints,
            palette_scratch?,
            observe_work,
        )?;
    } else if palette_scratch.is_some() {
        return None;
    }

    Some(DeformationValidatedShape {
        refs: DeformationValidatedRefs {
            skins: skins.map(CanonicalBlockRef::from_store),
            logical_morphs: logical_morphs.map(CanonicalBlockRef::from_store),
            morph_definitions: morph_definitions.map(CanonicalBlockRef::from_store),
            primitives: primitives.map(CanonicalBlockRef::from_store),
            morph_targets: morph_targets.map(CanonicalBlockRef::from_store),
            morph_deltas: morph_deltas.map(CanonicalBlockRef::from_store),
            mesh_instances: mesh_instances.map(CanonicalBlockRef::from_store),
            joint_nodes: joint_nodes.map(CanonicalBlockRef::from_store),
            inverse_bind_matrices: inverse_bind_matrices.map(CanonicalBlockRef::from_store),
            skin_correction_matrices: skin_correction_matrices.map(CanonicalBlockRef::from_store),
            skin_names: skin_names.map(CanonicalBlockRef::from_store),
            influence_row_offsets: influence_row_offsets.map(CanonicalBlockRef::from_store),
            influence_joints: influence_joints.map(CanonicalBlockRef::from_store),
            influence_weights: influence_weights.map(CanonicalBlockRef::from_store),
            influence_source_orders: influence_source_orders.map(CanonicalBlockRef::from_store),
            morph_defaults: morph_defaults.map(CanonicalBlockRef::from_store),
            morph_names: morph_names.map(CanonicalBlockRef::from_store),
            morph_values: morph_values.map(CanonicalBlockRef::from_store),
            morph_sparse_indices: morph_sparse_indices.map(CanonicalBlockRef::from_store),
            instance_morph_weights: instance_morph_weights.map(CanonicalBlockRef::from_store),
        },
        payload_block_count,
        skin_count,
        logical_morph_count,
        morph_definition_count,
        primitive_count,
        morph_target_count,
        morph_delta_count,
        mesh_instance_count,
        maximum_uniqueness_count: logical_morph_maximum_uniqueness_count
            .max(primitive_views.maximum_uniqueness_count),
    })
}

fn validate_dependencies<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    dependencies: DeformationValidationDependencies,
    logical_morph_count: u32,
    primitive_count: u32,
    mesh_instance_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
) -> Option<DeformationDependencyViews<'storage>> {
    let nodes = session.canonical_records(dependencies.nodes.into_store())?;
    if nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT {
        return None;
    }
    if let Some(fresh) = fresh
        && nodes.rows() != usize::try_from(fresh.nodes.canonical_node_count()).ok()?
    {
        return None;
    }
    let logical_meshes = validate_optional_dependency(
        session,
        dependencies.logical_meshes,
        logical_morph_count,
        fresh.map(|fresh| fresh.deformation.logical_morph_count()),
        GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
    )?;
    let primitives = validate_optional_dependency(
        session,
        dependencies.primitives,
        primitive_count,
        fresh.map(|fresh| fresh.deformation.primitive_count()),
        GEOMETRY_PRIMITIVE_FIELD_COUNT,
    )?;
    let mesh_instances = validate_optional_dependency(
        session,
        dependencies.mesh_instances,
        mesh_instance_count,
        fresh.map(|fresh| fresh.deformation.mesh_instance_count()),
        GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    )?;
    Some(DeformationDependencyViews {
        nodes,
        logical_meshes,
        primitives,
        mesh_instances,
    })
}

fn validate_optional_dependency<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalBlockRef<Tag>>,
    canonical_count: u32,
    fresh_count: Option<u32>,
    expected_fields: usize,
) -> Option<Option<CanonicalRecordView<'storage>>>
where
    Tag: crate::canonical_scene::store::schema::CanonicalSchema<
            StoreKind = baozi_operation::RecordBlock,
        >,
{
    let view = match (canonical_count, reference) {
        (0, None) => Some(None),
        (0, Some(_)) | (_, None) => None,
        (_, Some(reference)) => {
            let table = session.canonical_records(reference.into_store())?;
            if table.rows() != usize::try_from(canonical_count).ok()?
                || table.fields() != expected_fields
            {
                return None;
            }
            Some(Some(table))
        }
    }?;
    require_fresh_count(canonical_count, fresh_count)?;
    Some(view)
}

fn optional_record_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_fields: usize,
) -> Option<(u32, Option<CanonicalRecordRef<Tag>>)>
where
    Tag: Copy + Send + Sync + 'static,
{
    let count = directory.scalar::<u32>(0, count_field)?;
    if count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some((count, None))
    } else {
        let reference = directory.record_ref::<Tag>(0, reference_field)?;
        let table = session.canonical_records(reference)?;
        if table.rows() != usize::try_from(count).ok()? || table.fields() != expected_fields {
            return None;
        }
        Some((count, Some(reference)))
    }
}

fn optional_sequence_ref<T, Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
) -> Option<(u32, Option<CanonicalSequenceRef<T, Tag>>)>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    let count = directory.scalar::<u32>(0, count_field)?;
    optional_sequence_ref_same_count(session, directory, reference_field, count)
        .map(|reference| (count, reference))
}

fn optional_sequence_ref_same_count<T, Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    reference_field: usize,
    expected_count: u32,
) -> Option<Option<CanonicalSequenceRef<T, Tag>>>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.sequence_ref::<T, Tag>(0, reference_field)?;
        let sequence = session.canonical_sequence(reference)?;
        (sequence.len() == usize::try_from(expected_count).ok()?).then_some(Some(reference))
    }
}

fn optional_utf8_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
) -> Option<(u32, Option<CanonicalUtf8Ref<Tag>>)>
where
    Tag: Copy + Send + Sync + 'static,
{
    let count = directory.scalar::<u32>(0, count_field)?;
    if count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some((count, None))
    } else {
        let reference = directory.utf8_ref::<Tag>(0, reference_field)?;
        let payload = session.canonical_utf8(reference)?;
        (payload.len() == usize::try_from(count).ok()?).then_some((count, Some(reference)))
    }
}

fn require_fresh_count(actual: u32, expected: Option<u32>) -> Option<()> {
    expected
        .is_none_or(|expected| actual == expected)
        .then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_skins<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationSkinTableSchema>>,
    joint_nodes: Option<CanonicalSequenceRef<u32, DeformationSkinJointNodeSchema>>,
    inverse_bind_matrices: Option<CanonicalSequenceRef<f32, DeformationInverseBindMatrixSchema>>,
    correction_matrices: Option<CanonicalSequenceRef<f32, DeformationSkinCorrectionMatrixSchema>>,
    names: Option<CanonicalUtf8Ref<DeformationSkinNameSchema>>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    nodes: CanonicalRecordView<'storage>,
    skin_count: u32,
    joint_node_count: u32,
    inverse_bind_scalar_count: u32,
    correction_scalar_count: u32,
    name_byte_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (skin_count == 0).then_some(None);
    };
    let table = session.canonical_records(reference)?;
    let joint_values = match joint_nodes {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let inverse_values = match inverse_bind_matrices {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let correction_values = match correction_matrices {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let name_values = match names {
        Some(reference) => Some(session.canonical_utf8(reference)?),
        None => None,
    };
    let mapping = fresh.map(|fresh| fresh.nodes.canonical_index_by_local());
    let mut joint_cursor = 0_u32;
    let mut inverse_cursor = 0_u32;
    let mut correction_cursor = 0_u32;
    let mut name_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_skin = match fresh {
            Some(fresh) => Some(fresh.deformation.skin(row)?),
            None => None,
        };
        let skin_id = u32::try_from(row).ok()?;
        let stored_joint_offset = table.scalar::<u32>(row, deformation_skin_field::JOINT_OFFSET)?;
        let joint_count = table.scalar::<u32>(row, deformation_skin_field::JOINT_COUNT)?;
        let stored_inverse_offset =
            table.scalar::<u32>(row, deformation_skin_field::INVERSE_BIND_OFFSET)?;
        let inverse_count = table.scalar::<u32>(row, deformation_skin_field::INVERSE_BIND_COUNT)?;
        let inverse_scalar_count = inverse_count.checked_mul(MATRIX_SCALAR_COUNT)?;
        if joint_count != inverse_count
            || stored_joint_offset != joint_cursor
            || stored_inverse_offset != inverse_cursor
        {
            return None;
        }
        if let Some(skin) = expected_skin.as_ref() {
            let authored = skin.authored();
            if skin.joint_offset() != joint_cursor
                || skin.joint_count() != joint_count
                || skin.inverse_bind_scalar_offset() != inverse_cursor
                || usize::try_from(joint_count).ok()? != authored.joints().len()
                || usize::try_from(inverse_count).ok()? != authored.inverse_bind_matrices().len()
            {
                return None;
            }
        }
        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::SKIN),
            skin_id,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::JOINT_OFFSET),
            joint_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::JOINT_COUNT),
            joint_count,
        )?;
        require_optional_sequence_reference(
            table,
            row,
            deformation_skin_field::JOINTS,
            joint_count != 0,
            joint_nodes,
        )?;
        if joint_count != 0 {
            let stored = joint_values?.as_slice();
            let range = checked_range(joint_cursor, joint_count, stored.len())?;
            let stored = stored.get(range)?;
            observe_work(u64::from(joint_count))?;
            if stored
                .iter()
                .any(|joint| !is_authored_node_index(*joint, nodes.rows()))
            {
                return None;
            }
            if let Some(skin) = expected_skin.as_ref() {
                for (stored, authored) in stored.iter().zip(skin.authored().joints()) {
                    let local = usize::try_from(authored.index()).ok()?;
                    if *stored != *mapping?.get(local)? {
                        return None;
                    }
                }
            }
        }

        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::INVERSE_BIND_OFFSET),
            inverse_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::INVERSE_BIND_COUNT),
            inverse_count,
        )?;
        require_optional_sequence_reference(
            table,
            row,
            deformation_skin_field::INVERSE_BINDS,
            inverse_count != 0,
            inverse_bind_matrices,
        )?;
        if inverse_scalar_count != 0 {
            let stored = inverse_values?.as_slice();
            let range = checked_range(inverse_cursor, inverse_scalar_count, stored.len())?;
            let stored = stored.get(range)?;
            observe_work(u64::from(inverse_scalar_count))?;
            validate_affine_matrices(stored)?;
            if let Some(skin) = expected_skin.as_ref() {
                let mut scalar_index = 0_usize;
                for matrix in skin.authored().inverse_bind_matrices() {
                    for column in matrix.cols {
                        for expected_scalar in column {
                            require_float32_bits(
                                stored.get(scalar_index).copied(),
                                expected_scalar,
                            )?;
                            scalar_index = scalar_index.checked_add(1)?;
                        }
                    }
                }
                if scalar_index != stored.len() {
                    return None;
                }
            }
        }

        let root = match require_presence(
            table.scalar::<u8>(row, deformation_skin_field::SKELETON_ROOT_PRESENCE),
        )? {
            StoredPresence::Absent => {
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::SKELETON_ROOT),
                    0_u32,
                )?;
                None
            }
            StoredPresence::Present => {
                let root = table.scalar::<u32>(row, deformation_skin_field::SKELETON_ROOT)?;
                if !is_authored_node_index(root, nodes.rows()) {
                    return None;
                }
                Some(root)
            }
        };
        if let Some(skin) = expected_skin.as_ref() {
            match (skin.authored().skeleton_root(), root) {
                (Presence::Absent, None) => {}
                (Presence::Present(expected), Some(root)) => {
                    let local = usize::try_from(expected.index()).ok()?;
                    require_scalar(Some(root), *mapping?.get(local)?)?;
                }
                _ => return None,
            }
        }
        require_optional_record_reference(
            table,
            row,
            deformation_skin_field::NODES,
            joint_count != 0 || root.is_some(),
            Some(nodes_ref),
        )?;

        let space_fields = [
            deformation_skin_field::INPUT_POSITION_SPACE,
            deformation_skin_field::INVERSE_BIND_FROM_SPACE,
            deformation_skin_field::INVERSE_BIND_TO_SPACE,
            deformation_skin_field::CURRENT_JOINT_FROM_SPACE,
            deformation_skin_field::CURRENT_JOINT_TO_SPACE,
            deformation_skin_field::OUTPUT_POSITION_SPACE,
        ];
        let canonical_spaces = [
            DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
            DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
            DEFORMATION_SPACE_JOINT_LOCAL_BIND,
            DEFORMATION_SPACE_JOINT_LOCAL,
            DEFORMATION_SPACE_SCENE,
            DEFORMATION_SPACE_MESH_INSTANCE_LOCAL,
        ];
        for (field, encoded) in space_fields.into_iter().zip(canonical_spaces) {
            require_scalar(table.scalar::<u8>(row, field), encoded)?;
        }
        if let Some(skin) = expected_skin.as_ref() {
            for (field, encoded) in
                space_fields
                    .into_iter()
                    .zip(encode_deformation_skin_matrix_spaces(
                        skin.authored().matrix_spaces(),
                    ))
            {
                require_scalar(table.scalar::<u8>(row, field), encoded)?;
            }
        }

        let correction_present =
            require_presence(table.scalar::<u8>(row, deformation_skin_field::CORRECTION_PRESENCE))?;
        match correction_present {
            StoredPresence::Absent => {
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_FROM_SPACE),
                    0_u8,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_TO_SPACE),
                    0_u8,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::CORRECTION_MATRIX_OFFSET),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::CORRECTION_MATRIX_COUNT),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_MATRICES),
                    0_u8,
                )?;
            }
            StoredPresence::Present => {
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_FROM_SPACE),
                    DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_TO_SPACE),
                    DEFORMATION_SPACE_MESH_INSTANCE_LOCAL,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::CORRECTION_MATRIX_OFFSET),
                    correction_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::CORRECTION_MATRIX_COUNT),
                    MATRIX_SCALAR_COUNT,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_skin_field::CORRECTION_MATRICES,
                    true,
                    correction_matrices,
                )?;
                let stored = correction_values?.as_slice();
                let range = checked_range(correction_cursor, MATRIX_SCALAR_COUNT, stored.len())?;
                let stored = stored.get(range)?;
                observe_work(u64::from(MATRIX_SCALAR_COUNT))?;
                validate_affine_matrices(stored)?;
                if let Some(skin) = expected_skin.as_ref() {
                    let correction = skin.authored().matrix_spaces().correction()?;
                    if skin.correction_matrix_scalar_offset() != correction_cursor
                        || skin.correction_matrix_scalar_count() != MATRIX_SCALAR_COUNT
                    {
                        return None;
                    }
                    require_scalar(
                        table.scalar::<u8>(row, deformation_skin_field::CORRECTION_FROM_SPACE),
                        encode_deformation_skin_space(correction.from()),
                    )?;
                    require_scalar(
                        table.scalar::<u8>(row, deformation_skin_field::CORRECTION_TO_SPACE),
                        encode_deformation_skin_space(correction.to()),
                    )?;
                    let mut scalar_index = 0_usize;
                    for column in correction.matrix().cols {
                        for expected_scalar in column {
                            require_float32_bits(
                                stored.get(scalar_index).copied(),
                                expected_scalar,
                            )?;
                            scalar_index = scalar_index.checked_add(1)?;
                        }
                    }
                    if scalar_index != stored.len() {
                        return None;
                    }
                }
                correction_cursor = correction_cursor.checked_add(MATRIX_SCALAR_COUNT)?;
            }
        }
        if let Some(skin) = expected_skin.as_ref() {
            match (
                skin.authored().matrix_spaces().correction(),
                correction_present,
            ) {
                (None, StoredPresence::Absent) => {
                    if skin.correction_matrix_scalar_offset() != 0
                        || skin.correction_matrix_scalar_count() != 0
                    {
                        return None;
                    }
                }
                (Some(_), StoredPresence::Present) => {}
                _ => return None,
            }
        }

        let name_present =
            require_presence(table.scalar::<u8>(row, deformation_skin_field::NAME_PRESENCE))?;
        let stored_name_offset = table.scalar::<u32>(row, deformation_skin_field::NAME_OFFSET)?;
        let stored_name_count = table.scalar::<u32>(row, deformation_skin_field::NAME_COUNT)?;
        match name_present {
            StoredPresence::Absent => {
                require_scalar(
                    Some(stored_name_offset.checked_add(stored_name_count)?),
                    0_u32,
                )?;
                require_scalar(table.scalar::<u8>(row, deformation_skin_field::NAMES), 0_u8)?;
            }
            StoredPresence::Present => {
                require_scalar(Some(stored_name_offset), name_cursor)?;
                require_optional_utf8_reference(
                    table,
                    row,
                    deformation_skin_field::NAMES,
                    stored_name_count != 0,
                    names,
                )?;
                let stored =
                    checked_utf8_range(name_values.unwrap_or(""), name_cursor, stored_name_count)?;
                if stored_name_count != 0 {
                    observe_work(u64::from(stored_name_count))?;
                }
                validate_entity_name_syntax(stored)?;
                if let Some(skin) = expected_skin.as_ref() {
                    let Presence::Present(name) = skin.authored().name() else {
                        return None;
                    };
                    if stored != name.as_str() {
                        return None;
                    }
                }
                name_cursor = name_cursor.checked_add(stored_name_count)?;
            }
        }
        if let Some(skin) = expected_skin.as_ref()
            && (skin.name_byte_offset() != stored_name_offset
                || skin.name_byte_count() != stored_name_count
                || matches!(skin.authored().name(), Presence::Present(_))
                    != (name_present == StoredPresence::Present))
        {
            return None;
        }

        joint_cursor = joint_cursor.checked_add(joint_count)?;
        inverse_cursor = inverse_cursor.checked_add(inverse_scalar_count)?;
    }
    (joint_cursor == joint_node_count
        && inverse_cursor == inverse_bind_scalar_count
        && correction_cursor == correction_scalar_count
        && name_cursor == name_byte_count)
        .then_some(Some(table))
}

#[allow(clippy::too_many_arguments)]
fn validate_morph_definitions<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationMorphDefinitionTableSchema>>,
    names: Option<CanonicalUtf8Ref<DeformationMorphNameSchema>>,
    definition_count: u32,
    name_byte_count: u32,
    logical_morph_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (definition_count == 0).then_some(None);
    };
    let table = session.canonical_records(reference)?;
    let name_values = match names {
        Some(reference) => Some(session.canonical_utf8(reference)?),
        None => None,
    };
    let mut name_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_definition = match fresh {
            Some(fresh) => Some(fresh.deformation.morph_definition(row)?),
            None => None,
        };
        require_scalar(
            table.scalar::<u32>(row, deformation_morph_definition_field::MORPH_DEFINITION),
            u32::try_from(row).ok()?,
        )?;
        let logical_mesh =
            table.scalar::<u32>(row, deformation_morph_definition_field::LOGICAL_MESH)?;
        let target_ordinal =
            table.scalar::<u32>(row, deformation_morph_definition_field::TARGET_ORDINAL)?;
        let stable_identity =
            table.scalar::<u32>(row, deformation_morph_definition_field::STABLE_IDENTITY)?;
        if logical_mesh >= logical_morph_count {
            return None;
        }
        if let Some(definition) = expected_definition.as_ref()
            && (logical_mesh != definition.logical_mesh()
                || target_ordinal != definition.target_ordinal()
                || stable_identity != definition.authored().identity().value())
        {
            return None;
        }
        let name_present = require_presence(
            table.scalar::<u8>(row, deformation_morph_definition_field::NAME_PRESENCE),
        )?;
        let name_offset =
            table.scalar::<u32>(row, deformation_morph_definition_field::NAME_OFFSET)?;
        let name_count =
            table.scalar::<u32>(row, deformation_morph_definition_field::NAME_COUNT)?;
        match name_present {
            StoredPresence::Absent => {
                require_scalar(Some(name_offset.checked_add(name_count)?), 0_u32)?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_morph_definition_field::NAMES),
                    0_u8,
                )?;
            }
            StoredPresence::Present => {
                require_scalar(Some(name_offset), name_cursor)?;
                require_optional_utf8_reference(
                    table,
                    row,
                    deformation_morph_definition_field::NAMES,
                    name_count != 0,
                    names,
                )?;
                let stored =
                    checked_utf8_range(name_values.unwrap_or(""), name_cursor, name_count)?;
                if name_count != 0 {
                    observe_work(u64::from(name_count))?;
                }
                validate_entity_name_syntax(stored)?;
                if let Some(definition) = expected_definition.as_ref() {
                    let Presence::Present(name) = definition.authored().name() else {
                        return None;
                    };
                    if stored != name.as_str() {
                        return None;
                    }
                }
                name_cursor = name_cursor.checked_add(name_count)?;
            }
        }
        if let Some(definition) = expected_definition.as_ref()
            && (definition.name_byte_offset() != name_offset
                || definition.name_byte_count() != name_count
                || matches!(definition.authored().name(), Presence::Present(_))
                    != (name_present == StoredPresence::Present))
        {
            return None;
        }
    }
    (name_cursor == name_byte_count).then_some(Some(table))
}

#[allow(clippy::too_many_arguments)]
fn validate_logical_morphs<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationLogicalMorphTableSchema>>,
    definitions: Option<CanonicalRecordRef<DeformationMorphDefinitionTableSchema>>,
    definition_values: Option<CanonicalRecordView<'storage>>,
    defaults: Option<CanonicalSequenceRef<f32, DeformationMorphDefaultWeightSchema>>,
    logical_morph_count: u32,
    definition_count: u32,
    default_weight_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    mut uniqueness_scratch: Option<&mut DeformationUniquenessScratch<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<(Option<CanonicalRecordView<'storage>>, usize)> {
    let Some(reference) = reference else {
        return (logical_morph_count == 0).then_some((None, 0));
    };
    let table = session.canonical_records(reference)?;
    let default_values = match defaults {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut definition_cursor = 0_u32;
    let mut default_cursor = 0_u32;
    let mut maximum_uniqueness_count = 0_usize;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_logical = match fresh {
            Some(fresh) => Some(fresh.deformation.logical_morph(row)?),
            None => None,
        };
        let stored_definition_offset =
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFINITION_OFFSET)?;
        let stored_definition_count =
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFINITION_COUNT)?;
        let stored_default_offset =
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_OFFSET)?;
        let stored_default_count =
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_COUNT)?;
        if stored_definition_offset != definition_cursor
            || stored_default_offset != default_cursor
            || stored_definition_count != stored_default_count
        {
            return None;
        }
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::LOGICAL_MESH),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFINITION_OFFSET),
            definition_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFINITION_COUNT),
            stored_definition_count,
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_logical_morph_field::DEFINITIONS,
            stored_definition_count != 0,
            definitions,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_OFFSET),
            default_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_COUNT),
            stored_default_count,
        )?;
        require_optional_sequence_reference(
            table,
            row,
            deformation_logical_morph_field::DEFAULT_WEIGHTS,
            stored_default_count != 0,
            defaults,
        )?;
        if stored_definition_count != 0 {
            let definitions = definition_values?;
            let definition_width = usize::try_from(stored_definition_count).ok()?;
            maximum_uniqueness_count = maximum_uniqueness_count.max(definition_width);
            let mut identity_marks = match fresh {
                Some(_) => None,
                None => Some(
                    uniqueness_scratch
                        .as_deref_mut()?
                        .begin_group(definition_width)?,
                ),
            };
            let range = checked_range(
                definition_cursor,
                stored_definition_count,
                definitions.rows(),
            )?;
            for (target, definition_row) in range.enumerate() {
                observe_work(1)?;
                require_scalar(
                    definitions.scalar::<u32>(
                        definition_row,
                        deformation_morph_definition_field::LOGICAL_MESH,
                    ),
                    u32::try_from(row).ok()?,
                )?;
                require_scalar(
                    definitions.scalar::<u32>(
                        definition_row,
                        deformation_morph_definition_field::TARGET_ORDINAL,
                    ),
                    u32::try_from(target).ok()?,
                )?;
                let identity = definitions.scalar::<u32>(
                    definition_row,
                    deformation_morph_definition_field::STABLE_IDENTITY,
                )?;
                if identity >= stored_definition_count {
                    return None;
                }
                if let Some(marks) = identity_marks.as_mut() {
                    marks.mark(identity)?;
                }
            }
        }
        if stored_default_count != 0 {
            let stored = default_values?.as_slice();
            let range = checked_range(default_cursor, stored_default_count, stored.len())?;
            let stored = stored.get(range)?;
            observe_work(u64::from(stored_default_count))?;
            if stored.iter().any(|weight| !weight.is_finite()) {
                return None;
            }
            if let Some(logical) = expected_logical.as_ref() {
                for (stored, expected) in stored.iter().zip(logical.authored().default_weights()) {
                    require_float32_bits(Some(*stored), *expected)?;
                }
            }
        }
        if let Some(logical) = expected_logical.as_ref()
            && (logical.definitions().offset() != stored_definition_offset
                || logical.definitions().count() != stored_definition_count
                || logical.target_count() != stored_definition_count
                || logical.default_weight_offset() != stored_default_offset
                || usize::try_from(stored_default_count).ok()?
                    != logical.authored().default_weights().len())
        {
            return None;
        }
        definition_cursor = definition_cursor.checked_add(stored_definition_count)?;
        default_cursor = default_cursor.checked_add(stored_default_count)?;
    }
    (definition_cursor == definition_count && default_cursor == default_weight_count)
        .then_some((Some(table), maximum_uniqueness_count))
}

#[allow(clippy::too_many_arguments)]
fn validate_morph_deltas<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationMorphDeltaTableSchema>>,
    values: Option<CanonicalSequenceRef<f32, DeformationMorphValueSchema>>,
    sparse_indices: Option<CanonicalSequenceRef<u32, DeformationMorphSparseIndexSchema>>,
    targets: Option<CanonicalRecordRef<DeformationMorphTargetTableSchema>>,
    geometry_primitives: Option<CanonicalRecordView<'storage>>,
    delta_count: u32,
    value_count: u32,
    sparse_index_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (delta_count == 0).then_some(None);
    };
    let table = session.canonical_records(reference)?;
    let target_table = session.canonical_records(targets?)?;
    let primitive_table = geometry_primitives?;
    let stored_values = match values {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_indices = match sparse_indices {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut target_cursor = 0_u32;
    let mut delta_cursor = 0_u32;
    let mut value_cursor = 0_u32;
    let mut sparse_cursor = 0_u32;
    for primitive_index in 0..primitive_table.rows() {
        observe_work(1)?;
        let expected_primitive = match fresh {
            Some(fresh) => Some(fresh.deformation.primitive(primitive_index)?),
            None => None,
        };
        let primitive_id = u32::try_from(primitive_index).ok()?;
        let vertex_count = primitive_table
            .scalar::<u32>(primitive_index, geometry_primitive_field::VERTEX_COUNT)?;
        let mut target_ordinal = 0_u32;
        while usize::try_from(target_cursor).ok()? < target_table.rows()
            && target_table.scalar::<u32>(
                usize::try_from(target_cursor).ok()?,
                deformation_morph_target_field::PRIMITIVE,
            )? == primitive_id
        {
            observe_work(1)?;
            let target_row = usize::try_from(target_cursor).ok()?;
            require_scalar(
                target_table
                    .scalar::<u32>(target_row, deformation_morph_target_field::MORPH_TARGET),
                target_cursor,
            )?;
            require_scalar(
                target_table
                    .scalar::<u32>(target_row, deformation_morph_target_field::TARGET_ORDINAL),
                target_ordinal,
            )?;
            let stored_delta_offset = target_table
                .scalar::<u32>(target_row, deformation_morph_target_field::DELTA_OFFSET)?;
            let stored_delta_count = target_table
                .scalar::<u32>(target_row, deformation_morph_target_field::DELTA_COUNT)?;
            require_scalar(Some(stored_delta_offset), delta_cursor)?;
            require_optional_record_reference(
                target_table,
                target_row,
                deformation_morph_target_field::DELTAS,
                stored_delta_count != 0,
                Some(reference),
            )?;
            let expected_target = match expected_primitive.as_ref() {
                Some(primitive) => {
                    let target = primitive.morph_target(usize::try_from(target_ordinal).ok()?)?;
                    if target.primitive_index() != primitive_id
                        || target.target_ordinal() != target_ordinal
                        || u32::try_from(target.delta_count()).ok()? != stored_delta_count
                    {
                        return None;
                    }
                    Some(target)
                }
                None => None,
            };
            let mut previous_semantic = None;
            for delta_ordinal in 0..stored_delta_count {
                observe_work(1)?;
                let row = usize::try_from(delta_cursor).ok()?;
                let semantic = table.scalar::<u8>(row, deformation_morph_delta_field::SEMANTIC)?;
                if !matches!(
                    semantic,
                    DEFORMATION_MORPH_POSITION
                        | DEFORMATION_MORPH_NORMAL
                        | DEFORMATION_MORPH_TANGENT
                ) || previous_semantic.is_some_and(|previous| semantic <= previous)
                {
                    return None;
                }
                previous_semantic = Some(semantic);
                let payload_kind =
                    table.scalar::<u8>(row, deformation_morph_delta_field::PAYLOAD_KIND)?;
                if !matches!(
                    payload_kind,
                    DEFORMATION_MORPH_DENSE | DEFORMATION_MORPH_SPARSE
                ) {
                    return None;
                }
                let logical_count =
                    table.scalar::<u32>(row, deformation_morph_delta_field::LOGICAL_COUNT)?;
                let explicit_count =
                    table.scalar::<u32>(row, deformation_morph_delta_field::EXPLICIT_COUNT)?;
                let stored_value_offset =
                    table.scalar::<u32>(row, deformation_morph_delta_field::VALUE_OFFSET)?;
                let stored_value_count =
                    table.scalar::<u32>(row, deformation_morph_delta_field::VALUE_COUNT)?;
                let stored_sparse_offset =
                    table.scalar::<u32>(row, deformation_morph_delta_field::SPARSE_INDEX_OFFSET)?;
                let stored_sparse_count =
                    table.scalar::<u32>(row, deformation_morph_delta_field::SPARSE_INDEX_COUNT)?;
                let expected_value_count = explicit_count.checked_mul(MORPH_VECTOR_WIDTH)?;
                if logical_count != vertex_count
                    || stored_value_offset != value_cursor
                    || stored_value_count != expected_value_count
                    || stored_sparse_offset != sparse_cursor
                    || match payload_kind {
                        DEFORMATION_MORPH_DENSE => {
                            explicit_count != logical_count || stored_sparse_count != 0
                        }
                        DEFORMATION_MORPH_SPARSE => stored_sparse_count != explicit_count,
                        _ => true,
                    }
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::MORPH_DELTA),
                    delta_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::MORPH_TARGET),
                    target_cursor,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_morph_delta_field::VALUES,
                    stored_value_count != 0,
                    values,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_morph_delta_field::SPARSE_INDICES,
                    stored_sparse_count != 0,
                    sparse_indices,
                )?;
                let expected_delta = match expected_target.as_ref() {
                    Some(target) => Some(target.delta(usize::try_from(delta_ordinal).ok()?)?),
                    None => None,
                };
                let (expected_payload_kind, authored_values, authored_indices) =
                    match expected_delta
                        .as_ref()
                        .map(|delta| delta.authored_payload())
                    {
                        None => (None, &[][..], &[][..]),
                        Some(MorphDeltaPayloadAuthoring::Dense { values }) => {
                            (Some(DeformationMorphPayloadKind::Dense), values, &[][..])
                        }
                        Some(MorphDeltaPayloadAuthoring::Sparse {
                            indices, values, ..
                        }) => (Some(DeformationMorphPayloadKind::Sparse), values, indices),
                    };
                if let Some(delta) = expected_delta.as_ref()
                    && (delta.semantic() != delta.authored().semantic()
                        || encode_deformation_morph_semantic(delta.semantic()) != semantic
                        || delta.payload_kind() != expected_payload_kind?
                        || encode_deformation_morph_payload(
                            delta.payload_kind() == DeformationMorphPayloadKind::Sparse,
                        ) != payload_kind
                        || delta.logical_count() != logical_count
                        || delta.explicit_count() != explicit_count
                        || delta.value_offset() != value_cursor
                        || delta.value_count() != stored_value_count
                        || delta.sparse_index_offset() != sparse_cursor
                        || delta.sparse_index_count() != stored_sparse_count
                        || u32::try_from(authored_values.len()).ok()? != stored_value_count
                        || u32::try_from(authored_indices.len()).ok()? != stored_sparse_count)
                {
                    return None;
                }
                if stored_value_count != 0 {
                    let stored = stored_values?.as_slice();
                    let range = checked_range(value_cursor, stored_value_count, stored.len())?;
                    let stored = stored.get(range)?;
                    observe_work(u64::from(stored_value_count))?;
                    if stored.iter().any(|value| !value.is_finite()) {
                        return None;
                    }
                    if expected_delta.is_some() {
                        for (stored, expected) in stored.iter().zip(authored_values) {
                            require_float32_bits(Some(*stored), *expected)?;
                        }
                    }
                }
                if stored_sparse_count != 0 {
                    let stored = stored_indices?.as_slice();
                    let range = checked_range(sparse_cursor, stored_sparse_count, stored.len())?;
                    let stored = stored.get(range)?;
                    observe_work(u64::from(stored_sparse_count))?;
                    if stored.iter().any(|index| *index >= logical_count)
                        || stored.windows(2).any(|pair| pair[0] >= pair[1])
                    {
                        return None;
                    }
                    if expected_delta.is_some() && stored != authored_indices {
                        return None;
                    }
                }
                value_cursor = value_cursor.checked_add(stored_value_count)?;
                sparse_cursor = sparse_cursor.checked_add(stored_sparse_count)?;
                delta_cursor = delta_cursor.checked_add(1)?;
            }
            target_cursor = target_cursor.checked_add(1)?;
            target_ordinal = target_ordinal.checked_add(1)?;
        }
        if let Some(primitive) = expected_primitive.as_ref()
            && primitive.morph_target_count() != usize::try_from(target_ordinal).ok()?
        {
            return None;
        }
    }
    (target_cursor == u32::try_from(target_table.rows()).ok()?
        && delta_cursor == delta_count
        && value_cursor == value_count
        && sparse_cursor == sparse_index_count)
        .then_some(Some(target_table))
}

#[allow(clippy::too_many_arguments)]
fn validate_morph_targets<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationMorphTargetTableSchema>>,
    target_view: Option<CanonicalRecordView<'storage>>,
    deltas: Option<CanonicalRecordRef<DeformationMorphDeltaTableSchema>>,
    primitives: Option<CanonicalRecordRef<DeformationPrimitiveTableSchema>>,
    target_count: u32,
    delta_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (target_count == 0).then_some(None);
    };
    let table = match target_view {
        Some(table) => table,
        None => session.canonical_records(reference)?,
    };
    let primitive_table = session.canonical_records(primitives?)?;
    let mut target_cursor = 0_u32;
    let mut delta_cursor = 0_u32;
    for primitive_index in 0..primitive_table.rows() {
        observe_work(1)?;
        let expected_primitive = match fresh {
            Some(fresh) => Some(fresh.deformation.primitive(primitive_index)?),
            None => None,
        };
        let stored_target_offset = primitive_table.scalar::<u32>(
            primitive_index,
            deformation_primitive_field::MORPH_TARGET_OFFSET,
        )?;
        let stored_target_count = primitive_table.scalar::<u32>(
            primitive_index,
            deformation_primitive_field::MORPH_TARGET_COUNT,
        )?;
        require_scalar(Some(stored_target_offset), target_cursor)?;
        require_optional_record_reference(
            primitive_table,
            primitive_index,
            deformation_primitive_field::MORPH_TARGETS,
            stored_target_count != 0,
            Some(reference),
        )?;
        if let Some(primitive) = expected_primitive.as_ref()
            && (primitive.morph_target_count() != usize::try_from(stored_target_count).ok()?)
        {
            return None;
        }
        for target_ordinal in 0..stored_target_count {
            observe_work(1)?;
            let row = usize::try_from(target_cursor).ok()?;
            let primitive_id = u32::try_from(primitive_index).ok()?;
            let stored_delta_offset =
                table.scalar::<u32>(row, deformation_morph_target_field::DELTA_OFFSET)?;
            let stored_delta_count =
                table.scalar::<u32>(row, deformation_morph_target_field::DELTA_COUNT)?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::MORPH_TARGET),
                target_cursor,
            )?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::PRIMITIVE),
                primitive_id,
            )?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::TARGET_ORDINAL),
                target_ordinal,
            )?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::DELTA_OFFSET),
                delta_cursor,
            )?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::DELTA_COUNT),
                stored_delta_count,
            )?;
            require_optional_record_reference(
                table,
                row,
                deformation_morph_target_field::DELTAS,
                stored_delta_count != 0,
                deltas,
            )?;
            if let Some(primitive) = expected_primitive.as_ref() {
                let target = primitive.morph_target(usize::try_from(target_ordinal).ok()?)?;
                if target.primitive_index() != primitive_id
                    || target.target_ordinal() != target_ordinal
                    || u32::try_from(target.delta_count()).ok()? != stored_delta_count
                    || stored_delta_offset != delta_cursor
                {
                    return None;
                }
            }
            delta_cursor = delta_cursor.checked_add(stored_delta_count)?;
            target_cursor = target_cursor.checked_add(1)?;
        }
    }
    (target_cursor == target_count && delta_cursor == delta_count).then_some(Some(primitive_table))
}

#[allow(clippy::too_many_arguments)]
fn validate_primitives<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationPrimitiveTableSchema>>,
    primitive_view: Option<CanonicalRecordView<'storage>>,
    row_offsets: Option<CanonicalSequenceRef<u32, DeformationInfluenceRowOffsetSchema>>,
    joints: Option<CanonicalSequenceRef<u32, DeformationInfluenceJointSchema>>,
    weights: Option<CanonicalSequenceRef<f32, DeformationInfluenceWeightSchema>>,
    source_orders: Option<CanonicalSequenceRef<u32, DeformationInfluenceSourceOrderSchema>>,
    morph_targets: Option<CanonicalRecordRef<DeformationMorphTargetTableSchema>>,
    geometry_primitives: Option<CanonicalRecordView<'storage>>,
    primitive_count: u32,
    row_offset_count: u32,
    influence_count: u32,
    morph_target_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    mut uniqueness_scratch: Option<&mut DeformationUniquenessScratch<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationPrimitiveViews<'storage>> {
    let Some(reference) = reference else {
        return (primitive_count == 0).then_some(DeformationPrimitiveViews {
            primitives: None,
            influence_joints: None,
            maximum_uniqueness_count: 0,
        });
    };
    let table = match primitive_view {
        Some(table) => table,
        None => session.canonical_records(reference)?,
    };
    let geometry = geometry_primitives?;
    let stored_row_offsets = match row_offsets {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_joints = match joints {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_weights = match weights {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_source_orders = match source_orders {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut row_offset_cursor = 0_u32;
    let mut influence_cursor = 0_u32;
    let mut target_cursor = 0_u32;
    let mut maximum_uniqueness_count = 0_usize;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_primitive = match fresh {
            Some(fresh) => Some(fresh.deformation.primitive(row)?),
            None => None,
        };
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::PRIMITIVE),
            u32::try_from(row).ok()?,
        )?;
        let vertex_count = geometry.scalar::<u32>(row, geometry_primitive_field::VERTEX_COUNT)?;
        let influences_present = require_presence(
            table.scalar::<u8>(row, deformation_primitive_field::INFLUENCE_PRESENCE),
        )?;
        let stored_row_offset_offset =
            table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_OFFSET)?;
        let stored_row_offset_count =
            table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_COUNT)?;
        let stored_influence_offset =
            table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_OFFSET)?;
        let stored_influence_count =
            table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_COUNT)?;
        match influences_present {
            StoredPresence::Absent => {
                for field in [
                    deformation_primitive_field::ROW_OFFSET_OFFSET,
                    deformation_primitive_field::ROW_OFFSET_COUNT,
                    deformation_primitive_field::INFLUENCE_OFFSET,
                    deformation_primitive_field::INFLUENCE_COUNT,
                ] {
                    require_scalar(table.scalar::<u32>(row, field), 0_u32)?;
                }
                for field in [
                    deformation_primitive_field::ROW_OFFSETS,
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                ] {
                    require_scalar(table.scalar::<u8>(row, field), 0_u8)?;
                }
            }
            StoredPresence::Present => {
                if stored_row_offset_count != vertex_count.checked_add(1)?
                    || stored_row_offset_offset != row_offset_cursor
                    || stored_influence_offset != influence_cursor
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_OFFSET),
                    row_offset_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_COUNT),
                    stored_row_offset_count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::ROW_OFFSETS,
                    true,
                    row_offsets,
                )?;
                let stored = stored_row_offsets?.as_slice();
                let range =
                    checked_range(row_offset_cursor, stored_row_offset_count, stored.len())?;
                let stored_offsets = stored.get(range)?;
                observe_work(u64::from(stored_row_offset_count))?;
                if stored_offsets.first().copied() != Some(0)
                    || stored_offsets.last().copied() != Some(stored_influence_count)
                    || stored_offsets
                        .iter()
                        .any(|offset| *offset > stored_influence_count)
                    || stored_offsets.windows(2).any(|pair| pair[0] > pair[1])
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_OFFSET),
                    influence_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_COUNT),
                    stored_influence_count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    stored_influence_count != 0,
                    joints,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    stored_influence_count != 0,
                    weights,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                    stored_influence_count != 0,
                    source_orders,
                )?;
                if stored_influence_count != 0 {
                    let joint_values = stored_joints?.as_slice();
                    let weight_values = stored_weights?.as_slice();
                    let order_values = stored_source_orders?.as_slice();
                    let joint_range = checked_range(
                        influence_cursor,
                        stored_influence_count,
                        joint_values.len(),
                    )?;
                    let weight_range = checked_range(
                        influence_cursor,
                        stored_influence_count,
                        weight_values.len(),
                    )?;
                    let order_range = checked_range(
                        influence_cursor,
                        stored_influence_count,
                        order_values.len(),
                    )?;
                    let joint_values = joint_values.get(joint_range)?;
                    let weight_values = weight_values.get(weight_range)?;
                    let order_values = order_values.get(order_range)?;
                    observe_work(u64::from(stored_influence_count).checked_mul(3)?)?;
                    if weight_values
                        .iter()
                        .any(|weight| !weight.is_finite() || *weight < 0.0)
                    {
                        return None;
                    }
                    for offsets in stored_offsets.windows(2) {
                        let start = usize::try_from(offsets[0]).ok()?;
                        let end = usize::try_from(offsets[1]).ok()?;
                        let orders = order_values.get(start..end)?;
                        let width = orders.len();
                        maximum_uniqueness_count = maximum_uniqueness_count.max(width);
                        let mut source_order_marks = match fresh {
                            Some(_) => None,
                            None => Some(uniqueness_scratch.as_deref_mut()?.begin_group(width)?),
                        };
                        for order in orders.iter().copied() {
                            observe_work(1)?;
                            match source_order_marks.as_mut() {
                                Some(marks) => marks.mark(order)?,
                                None => {
                                    let order = usize::try_from(order).ok()?;
                                    (order < width).then_some(())?;
                                }
                            }
                        }
                    }
                    if let Some(primitive) = expected_primitive.as_ref() {
                        let Presence::Present(influences) = primitive.authored().influences()
                        else {
                            return None;
                        };
                        if stored_offsets != influences.vertex_offsets()
                            || joint_values.len() != influences.influences().len()
                        {
                            return None;
                        }
                        for (((joint, weight), source_order), authored) in joint_values
                            .iter()
                            .zip(weight_values)
                            .zip(order_values)
                            .zip(influences.influences())
                        {
                            if *joint != authored.joint()
                                || *source_order != authored.source_order()
                            {
                                return None;
                            }
                            require_float32_bits(Some(*weight), authored.weight())?;
                        }
                    }
                }
                row_offset_cursor = row_offset_cursor.checked_add(stored_row_offset_count)?;
                influence_cursor = influence_cursor.checked_add(stored_influence_count)?;
            }
        }
        if let Some(primitive) = expected_primitive.as_ref()
            && (primitive.influence_row_offset_offset() != stored_row_offset_offset
                || primitive.influence_row_offset_count() != stored_row_offset_count
                || primitive.influence_offset() != stored_influence_offset
                || primitive.influence_count() != stored_influence_count
                || matches!(primitive.authored().influences(), Presence::Present(_))
                    != (influences_present == StoredPresence::Present))
        {
            return None;
        }
        let stored_target_offset =
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_OFFSET)?;
        let stored_target_count =
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_COUNT)?;
        require_scalar(Some(stored_target_offset), target_cursor)?;
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_OFFSET),
            target_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_COUNT),
            stored_target_count,
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_primitive_field::MORPH_TARGETS,
            stored_target_count != 0,
            morph_targets,
        )?;
        if let Some(primitive) = expected_primitive.as_ref()
            && primitive.morph_target_count() != usize::try_from(stored_target_count).ok()?
        {
            return None;
        }
        target_cursor = target_cursor.checked_add(stored_target_count)?;
    }
    (row_offset_cursor == row_offset_count
        && influence_cursor == influence_count
        && target_cursor == morph_target_count)
        .then_some(DeformationPrimitiveViews {
            primitives: Some(table),
            influence_joints: stored_joints,
            maximum_uniqueness_count,
        })
}

#[allow(clippy::too_many_arguments)]
fn validate_mesh_instances<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    skins: Option<CanonicalRecordRef<DeformationSkinTableSchema>>,
    morph_weights: Option<CanonicalSequenceRef<f32, DeformationInstanceMorphWeightSchema>>,
    logical_morphs: Option<CanonicalRecordView<'storage>>,
    geometry_mesh_instances: Option<CanonicalRecordView<'storage>>,
    skin_count: u32,
    mesh_instance_count: u32,
    morph_weight_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (mesh_instance_count == 0).then_some(None);
    };
    let table = session.canonical_records(reference)?;
    let logical_morphs = logical_morphs?;
    let geometry_mesh_instances = geometry_mesh_instances?;
    let stored_weights = match morph_weights {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut weight_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_instance = match fresh {
            Some(fresh) => Some(fresh.deformation.mesh_instance(row)?),
            None => None,
        };
        require_scalar(
            table.scalar::<u32>(row, deformation_mesh_instance_field::MESH_INSTANCE),
            u32::try_from(row).ok()?,
        )?;
        let logical_mesh = geometry_mesh_instances
            .scalar::<u32>(row, geometry_mesh_instance_field::LOGICAL_MESH)?;
        let logical_row = usize::try_from(logical_mesh).ok()?;
        if logical_row >= logical_morphs.rows() {
            return None;
        }
        let target_count = logical_morphs.scalar::<u32>(
            logical_row,
            deformation_logical_morph_field::DEFINITION_COUNT,
        )?;
        let skin_present = require_presence(
            table.scalar::<u8>(row, deformation_mesh_instance_field::SKIN_PRESENCE),
        )?;
        let stored_skin = table.scalar::<u32>(row, deformation_mesh_instance_field::SKIN)?;
        match skin_present {
            StoredPresence::Absent => {
                require_scalar(Some(stored_skin), 0_u32)?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_mesh_instance_field::SKINS),
                    0_u8,
                )?;
            }
            StoredPresence::Present => {
                if stored_skin >= skin_count {
                    return None;
                }
                require_optional_record_reference(
                    table,
                    row,
                    deformation_mesh_instance_field::SKINS,
                    true,
                    skins,
                )?;
            }
        }
        if let Some(instance) = expected_instance.as_ref() {
            match (instance.authored().skin(), skin_present) {
                (Presence::Absent, StoredPresence::Absent) if instance.skin().is_none() => {}
                (Presence::Present(skin), StoredPresence::Present)
                    if instance.skin() == Some(skin) && stored_skin == skin.index() => {}
                _ => return None,
            }
        }
        let weights_present = require_presence(
            table.scalar::<u8>(row, deformation_mesh_instance_field::MORPH_WEIGHTS_PRESENCE),
        )?;
        let stored_weight_offset =
            table.scalar::<u32>(row, deformation_mesh_instance_field::MORPH_WEIGHT_OFFSET)?;
        let stored_weight_count =
            table.scalar::<u32>(row, deformation_mesh_instance_field::MORPH_WEIGHT_COUNT)?;
        match weights_present {
            StoredPresence::Absent => {
                require_scalar(
                    Some(stored_weight_offset.checked_add(stored_weight_count)?),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_mesh_instance_field::MORPH_WEIGHTS),
                    0_u8,
                )?;
            }
            StoredPresence::Present => {
                if stored_weight_offset != weight_cursor || stored_weight_count != target_count {
                    return None;
                }
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_mesh_instance_field::MORPH_WEIGHTS,
                    stored_weight_count != 0,
                    morph_weights,
                )?;
                if stored_weight_count != 0 {
                    let stored = stored_weights?.as_slice();
                    let range = checked_range(weight_cursor, stored_weight_count, stored.len())?;
                    let stored = stored.get(range)?;
                    observe_work(u64::from(stored_weight_count))?;
                    if stored.iter().any(|weight| !weight.is_finite()) {
                        return None;
                    }
                    if let Some(instance) = expected_instance.as_ref() {
                        let Presence::Present(weights) = instance.authored().morph_weights() else {
                            return None;
                        };
                        for (stored, expected) in stored.iter().zip(weights) {
                            require_float32_bits(Some(*stored), *expected)?;
                        }
                    }
                }
                weight_cursor = weight_cursor.checked_add(stored_weight_count)?;
            }
        }
        if let Some(instance) = expected_instance.as_ref()
            && (instance.morph_weight_offset() != stored_weight_offset
                || instance.morph_weight_count() != stored_weight_count
                || matches!(instance.authored().morph_weights(), Presence::Present(_))
                    != (weights_present == StoredPresence::Present))
        {
            return None;
        }
    }
    (weight_cursor == morph_weight_count).then_some(Some(table))
}

#[allow(clippy::too_many_arguments)]
fn validate_canonical_cross_domain_semantics(
    logical_meshes: Option<CanonicalRecordView<'_>>,
    geometry_mesh_instances: Option<CanonicalRecordView<'_>>,
    logical_morphs: Option<CanonicalRecordView<'_>>,
    primitives: Option<CanonicalRecordView<'_>>,
    mesh_instances: Option<CanonicalRecordView<'_>>,
    skins: Option<CanonicalRecordView<'_>>,
    influence_joints: Option<SequenceView<'_, u32>>,
    palette_scratch: &mut [Option<u32>],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(logical_meshes) = logical_meshes else {
        return (logical_morphs.is_none()
            && primitives.is_none()
            && geometry_mesh_instances.is_none()
            && mesh_instances.is_none()
            && influence_joints.is_none())
        .then_some(());
    };
    let logical_morphs = logical_morphs?;
    if logical_meshes.rows() != logical_morphs.rows()
        || palette_scratch.len() < logical_meshes.rows()
    {
        return None;
    }
    let instance_views = match (geometry_mesh_instances, mesh_instances) {
        (None, None) => None,
        (Some(geometry), Some(deformation)) if geometry.rows() == deformation.rows() => {
            Some((geometry, deformation))
        }
        _ => return None,
    };
    let primitive_rows = primitives.map_or(0, |table| table.rows());
    let joint_values = influence_joints.map(|joints| joints.as_slice());
    let mut primitive_cursor = 0_u32;
    for logical_mesh_row in 0..logical_meshes.rows() {
        observe_work(1)?;
        let palette_mark = palette_scratch.get_mut(logical_mesh_row)?;
        *palette_mark = None;
        require_scalar(
            logical_meshes
                .scalar::<u32>(logical_mesh_row, geometry_logical_mesh_field::LOGICAL_MESH),
            u32::try_from(logical_mesh_row).ok()?,
        )?;
        let primitive_offset = logical_meshes.scalar::<u32>(
            logical_mesh_row,
            geometry_logical_mesh_field::PRIMITIVE_OFFSET,
        )?;
        let primitive_count = logical_meshes.scalar::<u32>(
            logical_mesh_row,
            geometry_logical_mesh_field::PRIMITIVE_COUNT,
        )?;
        require_scalar(Some(primitive_offset), primitive_cursor)?;
        let target_width = logical_morphs.scalar::<u32>(
            logical_mesh_row,
            deformation_logical_morph_field::DEFINITION_COUNT,
        )?;
        let primitive_range = checked_range(primitive_offset, primitive_count, primitive_rows)?;
        for primitive_row in primitive_range {
            observe_work(1)?;
            let primitives = primitives?;
            require_scalar(
                primitives.scalar::<u32>(
                    primitive_row,
                    deformation_primitive_field::MORPH_TARGET_COUNT,
                ),
                target_width,
            )?;
            let influence_offset = primitives
                .scalar::<u32>(primitive_row, deformation_primitive_field::INFLUENCE_OFFSET)?;
            let influence_count = primitives
                .scalar::<u32>(primitive_row, deformation_primitive_field::INFLUENCE_COUNT)?;
            if influence_count == 0 {
                continue;
            }
            let joint_values = joint_values?;
            let influence_range =
                checked_range(influence_offset, influence_count, joint_values.len())?;
            observe_work(u64::from(influence_count))?;
            for joint in joint_values.get(influence_range)?.iter().copied() {
                record_palette_joint(palette_mark, joint);
            }
        }
        primitive_cursor = primitive_offset.checked_add(primitive_count)?;
    }
    if usize::try_from(primitive_cursor).ok()? != primitive_rows {
        return None;
    }
    let Some((geometry_instances, deformation_instances)) = instance_views else {
        return Some(());
    };
    for instance_row in 0..deformation_instances.rows() {
        observe_work(1)?;
        let logical_mesh = usize::try_from(
            geometry_instances
                .scalar::<u32>(instance_row, geometry_mesh_instance_field::LOGICAL_MESH)?,
        )
        .ok()?;
        let skin_present = require_presence(
            deformation_instances
                .scalar::<u8>(instance_row, deformation_mesh_instance_field::SKIN_PRESENCE),
        )?;
        if skin_present == StoredPresence::Absent {
            continue;
        }
        let palette_maximum = *palette_scratch.get(logical_mesh)?;
        let skin = usize::try_from(
            deformation_instances
                .scalar::<u32>(instance_row, deformation_mesh_instance_field::SKIN)?,
        )
        .ok()?;
        let joint_count = skins?.scalar::<u32>(skin, deformation_skin_field::JOINT_COUNT)?;
        if !palette_fits_skin(palette_maximum, skin_present, joint_count) {
            return None;
        }
    }
    Some(())
}

fn require_optional_record_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    reference: Option<CanonicalRecordRef<Tag>>,
) -> Option<()>
where
    Tag: Copy + Send + Sync + 'static,
{
    if present {
        require_scalar(table.record_ref::<Tag>(row, field), reference?)
    } else {
        require_scalar(table.scalar::<u8>(row, field), 0_u8)
    }
}

fn require_optional_sequence_reference<T, Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    reference: Option<CanonicalSequenceRef<T, Tag>>,
) -> Option<()>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    if present {
        require_scalar(table.sequence_ref::<T, Tag>(row, field), reference?)
    } else {
        require_scalar(table.scalar::<u8>(row, field), 0_u8)
    }
}

fn require_optional_utf8_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    reference: Option<CanonicalUtf8Ref<Tag>>,
) -> Option<()>
where
    Tag: Copy + Send + Sync + 'static,
{
    if present {
        require_scalar(table.utf8_ref::<Tag>(row, field), reference?)
    } else {
        require_scalar(table.scalar::<u8>(row, field), 0_u8)
    }
}

fn require_presence(actual: Option<u8>) -> Option<StoredPresence> {
    match actual? {
        PRESENCE_ABSENT => Some(StoredPresence::Absent),
        PRESENCE_PRESENT => Some(StoredPresence::Present),
        _ => None,
    }
}

fn is_authored_node_index(index: u32, node_rows: usize) -> bool {
    index != 0 && usize::try_from(index).is_ok_and(|index| index < node_rows)
}

fn checked_utf8_range(value: &str, offset: u32, count: u32) -> Option<&str> {
    value.get(checked_range(offset, count, value.len())?)
}

fn validate_entity_name_syntax(value: &str) -> Option<()> {
    (value.len() <= MAX_ENTITY_NAME_BYTES).then_some(())?;
    (!value.chars().any(|character| character.is_control())).then_some(())
}

fn record_palette_joint(maximum: &mut Option<u32>, joint: u32) {
    *maximum = Some((*maximum).map_or(joint, |maximum| maximum.max(joint)));
}

fn palette_fits_skin(maximum: Option<u32>, skin_present: StoredPresence, joint_count: u32) -> bool {
    skin_present == StoredPresence::Absent || maximum.is_none_or(|maximum| maximum < joint_count)
}

fn validate_affine_matrices(values: &[f32]) -> Option<()> {
    let mut matrices = values.chunks_exact(usize::try_from(MATRIX_SCALAR_COUNT).ok()?);
    for matrix in &mut matrices {
        if matrix.iter().any(|value| !value.is_finite())
            || matrix.get(3).copied()? != 0.0
            || matrix.get(7).copied()? != 0.0
            || matrix.get(11).copied()? != 0.0
            || matrix.get(15).copied()? != 1.0
        {
            return None;
        }
    }
    matrices.remainder().is_empty().then_some(())
}

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
}

fn require_float32_bits(actual: Option<f32>, expected: f32) -> Option<()> {
    (actual?.to_bits() == expected.to_bits()).then_some(())
}

#[cfg(test)]
mod tests {
    use super::{
        DeformationUniquenessScratch, StoredPresence, checked_utf8_range, is_authored_node_index,
        palette_fits_skin, record_palette_joint, validate_entity_name_syntax,
    };
    use crate::entity::{EntityName, MAX_ENTITY_NAME_BYTES};

    #[test]
    fn authored_node_indices_exclude_the_synthetic_root() {
        assert!(!is_authored_node_index(0, 3));
        assert!(is_authored_node_index(1, 3));
        assert!(is_authored_node_index(2, 3));
        assert!(!is_authored_node_index(3, 3));
        assert!(!is_authored_node_index(u32::MAX, 3));
    }

    #[test]
    fn utf8_ranges_require_code_point_boundaries() {
        let value = "a\u{e9}z";

        assert_eq!(checked_utf8_range(value, 1, 2), Some("\u{e9}"));
        assert_eq!(checked_utf8_range(value, 2, 0), None);
        assert_eq!(checked_utf8_range(value, 1, 1), None);
    }

    #[test]
    fn entity_name_syntax_is_checked_without_ownership() {
        for value in ["", "valid \u{540d}\u{79f0}", "bad\nname"] {
            assert_eq!(
                validate_entity_name_syntax(value).is_some(),
                EntityName::try_new(value).is_ok()
            );
        }

        let oversized = [b'x'; MAX_ENTITY_NAME_BYTES + 1];
        let oversized = std::str::from_utf8(&oversized).expect("ASCII fixture");
        assert_eq!(validate_entity_name_syntax(oversized), None);
        assert!(EntityName::try_new(oversized).is_err());
    }

    #[test]
    fn palette_summary_covers_the_full_u32_domain() {
        let mut maximum = None;
        record_palette_joint(&mut maximum, 0);
        assert_eq!(maximum, Some(0));
        record_palette_joint(&mut maximum, u32::MAX);
        assert_eq!(maximum, Some(u32::MAX));

        assert!(palette_fits_skin(maximum, StoredPresence::Absent, 0));
        assert!(!palette_fits_skin(
            maximum,
            StoredPresence::Present,
            u32::MAX
        ));
        assert!(palette_fits_skin(None, StoredPresence::Present, 0));
        assert!(palette_fits_skin(Some(0), StoredPresence::Present, 1));
        assert!(!palette_fits_skin(Some(0), StoredPresence::Present, 0));
    }

    #[test]
    fn uniqueness_groups_reuse_marks_without_prefix_rescans() {
        let mut storage = [0_u64; 3];
        let mut scratch = DeformationUniquenessScratch::new(&mut storage);

        {
            let mut first = scratch.begin_group(3).expect("first group fits");
            assert_eq!(first.mark(2), Some(()));
            assert_eq!(first.mark(0), Some(()));
            assert_eq!(first.mark(1), Some(()));
            assert_eq!(first.mark(1), None);
        }

        let mut second = scratch.begin_group(2).expect("second group fits");
        assert_eq!(second.mark(1), Some(()));
        assert_eq!(second.mark(0), Some(()));
        assert_eq!(second.mark(2), None);
    }
}
