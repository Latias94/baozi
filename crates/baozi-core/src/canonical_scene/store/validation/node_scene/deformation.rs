use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef, CanonicalUtf8Ref,
    StoreValidationSession, StoredScalar,
};

use crate::canonical_scene::store::production::{
    DeformationMorphPayloadKind, MorphDeltaPayloadAuthoring, Presence, ValidatedDeformationBatch,
    ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::deformation::{
    DEFORMATION_DIRECTORY_FIELD_COUNT, DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
    DEFORMATION_MESH_INSTANCE_FIELD_COUNT, DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
    DEFORMATION_MORPH_DELTA_FIELD_COUNT, DEFORMATION_MORPH_TARGET_FIELD_COUNT,
    DEFORMATION_PRIMITIVE_FIELD_COUNT, DEFORMATION_SCHEMA_REVISION, DEFORMATION_SKIN_FIELD_COUNT,
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
    deformation_directory_field, deformation_logical_morph_field, deformation_mesh_instance_field,
    deformation_morph_definition_field, deformation_morph_delta_field,
    deformation_morph_target_field, deformation_primitive_field, deformation_skin_field,
    encode_deformation_morph_payload, encode_deformation_morph_semantic,
    encode_deformation_skin_matrix_spaces, encode_deformation_skin_space,
};
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_LOGICAL_MESH_FIELD_COUNT, GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    GEOMETRY_PRIMITIVE_FIELD_COUNT, GeometryLogicalMeshTableSchema,
    GeometryMeshInstanceTableSchema, GeometryPrimitiveTableSchema,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, NODE_SCENE_NODE_FIELD_COUNT, NodeSceneNodeTableSchema, PRESENCE_ABSENT,
    PRESENCE_PRESENT,
};

use super::super::require_scalar;

const MATRIX_SCALAR_COUNT: u32 = 16;
const MORPH_VECTOR_WIDTH: u32 = 3;

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

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_deformation_graph<R, NR>(
    session: &mut StoreValidationSession<'_, '_>,
    deformation_ref: CanonicalRecordRef<DeformationDirectorySchema>,
    dependencies: DeformationValidationDependencies,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(deformation_ref)?;
    if directory.rows() != 1 || directory.fields() != DEFORMATION_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, deformation_directory_field::SCHEMA_REVISION),
        DEFORMATION_SCHEMA_REVISION,
    )?;

    validate_dependencies(session, dependencies, expected_nodes, expected)?;

    let skins = optional_record_ref::<DeformationSkinTableSchema>(
        session,
        directory,
        deformation_directory_field::SKIN_COUNT,
        deformation_directory_field::SKINS,
        expected.skin_count(),
        DEFORMATION_SKIN_FIELD_COUNT,
    )?;
    let joint_nodes = optional_sequence_ref::<u32, DeformationSkinJointNodeSchema>(
        session,
        directory,
        deformation_directory_field::JOINT_NODE_COUNT,
        deformation_directory_field::JOINT_NODES,
        expected.joint_node_count(),
    )?;
    let inverse_bind_matrices = optional_sequence_ref::<f32, DeformationInverseBindMatrixSchema>(
        session,
        directory,
        deformation_directory_field::INVERSE_BIND_SCALAR_COUNT,
        deformation_directory_field::INVERSE_BIND_MATRICES,
        expected.inverse_bind_scalar_count(),
    )?;
    let logical_morphs = optional_record_ref::<DeformationLogicalMorphTableSchema>(
        session,
        directory,
        deformation_directory_field::LOGICAL_MORPH_COUNT,
        deformation_directory_field::LOGICAL_MORPHS,
        expected.logical_morph_count(),
        DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
    )?;
    let primitives = optional_record_ref::<DeformationPrimitiveTableSchema>(
        session,
        directory,
        deformation_directory_field::PRIMITIVE_DEFORMATION_COUNT,
        deformation_directory_field::PRIMITIVE_DEFORMATIONS,
        expected.primitive_count(),
        DEFORMATION_PRIMITIVE_FIELD_COUNT,
    )?;
    let influence_row_offsets = optional_sequence_ref::<u32, DeformationInfluenceRowOffsetSchema>(
        session,
        directory,
        deformation_directory_field::INFLUENCE_ROW_OFFSET_COUNT,
        deformation_directory_field::INFLUENCE_ROW_OFFSETS,
        expected.influence_row_offset_count(),
    )?;
    let influence_joints = optional_sequence_ref::<u32, DeformationInfluenceJointSchema>(
        session,
        directory,
        deformation_directory_field::INFLUENCE_COUNT,
        deformation_directory_field::INFLUENCE_JOINTS,
        expected.influence_count(),
    )?;
    let influence_weights =
        optional_sequence_ref_same_count::<f32, DeformationInfluenceWeightSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_WEIGHTS,
            expected.influence_count(),
        )?;
    let influence_source_orders =
        optional_sequence_ref_same_count::<u32, DeformationInfluenceSourceOrderSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_SOURCE_ORDERS,
            expected.influence_count(),
        )?;
    let morph_targets = optional_record_ref::<DeformationMorphTargetTableSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_TARGET_COUNT,
        deformation_directory_field::MORPH_TARGETS,
        expected.morph_target_count(),
        DEFORMATION_MORPH_TARGET_FIELD_COUNT,
    )?;
    let morph_deltas = optional_record_ref::<DeformationMorphDeltaTableSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_DELTA_COUNT,
        deformation_directory_field::MORPH_DELTAS,
        expected.morph_delta_count(),
        DEFORMATION_MORPH_DELTA_FIELD_COUNT,
    )?;
    let morph_defaults = optional_sequence_ref::<f32, DeformationMorphDefaultWeightSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_DEFAULT_COUNT,
        deformation_directory_field::MORPH_DEFAULTS,
        expected.morph_default_weight_count(),
    )?;
    let morph_values = optional_sequence_ref::<f32, DeformationMorphValueSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_VALUE_COUNT,
        deformation_directory_field::MORPH_VALUES,
        expected.morph_value_count(),
    )?;
    let morph_sparse_indices = optional_sequence_ref::<u32, DeformationMorphSparseIndexSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_SPARSE_INDEX_COUNT,
        deformation_directory_field::MORPH_SPARSE_INDICES,
        expected.morph_sparse_index_count(),
    )?;
    let mesh_instances = optional_record_ref::<DeformationMeshInstanceTableSchema>(
        session,
        directory,
        deformation_directory_field::MESH_INSTANCE_COUNT,
        deformation_directory_field::MESH_INSTANCES,
        expected.mesh_instance_count(),
        DEFORMATION_MESH_INSTANCE_FIELD_COUNT,
    )?;
    let instance_morph_weights = optional_sequence_ref::<f32, DeformationInstanceMorphWeightSchema>(
        session,
        directory,
        deformation_directory_field::INSTANCE_MORPH_WEIGHT_COUNT,
        deformation_directory_field::INSTANCE_MORPH_WEIGHTS,
        expected.instance_morph_weight_count(),
    )?;
    let skin_correction_matrices =
        optional_sequence_ref::<f32, DeformationSkinCorrectionMatrixSchema>(
            session,
            directory,
            deformation_directory_field::SKIN_CORRECTION_SCALAR_COUNT,
            deformation_directory_field::SKIN_CORRECTION_MATRICES,
            expected.skin_correction_scalar_count(),
        )?;
    let morph_definitions = optional_record_ref::<DeformationMorphDefinitionTableSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_DEFINITION_COUNT,
        deformation_directory_field::MORPH_DEFINITIONS,
        expected.morph_definition_count(),
        DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
    )?;
    let morph_names = optional_utf8_ref::<DeformationMorphNameSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_NAME_BYTE_COUNT,
        deformation_directory_field::MORPH_NAMES,
        expected.morph_name_byte_count(),
    )?;
    let skin_names = optional_utf8_ref::<DeformationSkinNameSchema>(
        session,
        directory,
        deformation_directory_field::SKIN_NAME_BYTE_COUNT,
        deformation_directory_field::SKIN_NAMES,
        expected.skin_name_byte_count(),
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
    if payload_block_count != expected.payload_block_count() {
        return None;
    }

    validate_skins(
        session,
        skins,
        joint_nodes,
        inverse_bind_matrices,
        skin_correction_matrices,
        skin_names,
        dependencies.nodes.into_store(),
        expected_nodes,
        expected,
        observe_work,
    )?;
    validate_morph_definitions(
        session,
        morph_definitions,
        morph_names,
        expected,
        observe_work,
    )?;
    validate_logical_morphs(
        session,
        logical_morphs,
        morph_definitions,
        morph_defaults,
        expected,
        observe_work,
    )?;
    validate_morph_deltas(
        session,
        morph_deltas,
        morph_values,
        morph_sparse_indices,
        expected,
        observe_work,
    )?;
    validate_morph_targets(session, morph_targets, morph_deltas, expected, observe_work)?;
    validate_primitives(
        session,
        primitives,
        influence_row_offsets,
        influence_joints,
        influence_weights,
        influence_source_orders,
        morph_targets,
        expected,
        observe_work,
    )?;
    validate_mesh_instances(
        session,
        mesh_instances,
        skins,
        instance_morph_weights,
        expected,
        observe_work,
    )?;

    Some(DeformationValidatedRefs {
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
    })
}

fn validate_dependencies<R, NR>(
    session: &mut StoreValidationSession<'_, '_>,
    dependencies: DeformationValidationDependencies,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedDeformationBatch<'_, R>,
) -> Option<()> {
    let nodes = session.canonical_records(dependencies.nodes.into_store())?;
    if nodes.rows() != usize::try_from(expected_nodes.canonical_node_count()).ok()?
        || nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT
    {
        return None;
    }
    validate_optional_dependency(
        session,
        dependencies.logical_meshes,
        expected.logical_morph_count(),
        GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
    )?;
    validate_optional_dependency(
        session,
        dependencies.primitives,
        expected.primitive_count(),
        GEOMETRY_PRIMITIVE_FIELD_COUNT,
    )?;
    validate_optional_dependency(
        session,
        dependencies.mesh_instances,
        expected.mesh_instance_count(),
        GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    )?;
    Some(())
}

fn validate_optional_dependency<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalBlockRef<Tag>>,
    expected_count: u32,
    expected_fields: usize,
) -> Option<()>
where
    Tag: crate::canonical_scene::store::schema::CanonicalSchema<
            StoreKind = baozi_operation::RecordBlock,
        >,
{
    match (expected_count, reference) {
        (0, None) => Some(()),
        (0, Some(_)) | (_, None) => None,
        (_, Some(reference)) => {
            let table = session.canonical_records(reference.into_store())?;
            (table.rows() == usize::try_from(expected_count).ok()?
                && table.fields() == expected_fields)
                .then_some(())
        }
    }
}

fn optional_record_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
    expected_fields: usize,
) -> Option<Option<CanonicalRecordRef<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.record_ref::<Tag>(0, reference_field)?;
        let table = session.canonical_records(reference)?;
        if table.rows() != usize::try_from(expected_count).ok()?
            || table.fields() != expected_fields
        {
            return None;
        }
        Some(Some(reference))
    }
}

fn optional_sequence_ref<T, Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
) -> Option<Option<CanonicalSequenceRef<T, Tag>>>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    optional_sequence_ref_same_count(session, directory, reference_field, expected_count)
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
    expected_count: u32,
) -> Option<Option<CanonicalUtf8Ref<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.utf8_ref::<Tag>(0, reference_field)?;
        let payload = session.canonical_utf8(reference)?;
        (payload.len() == usize::try_from(expected_count).ok()?).then_some(Some(reference))
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_skins<R, NR>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<DeformationSkinTableSchema>>,
    joint_nodes: Option<CanonicalSequenceRef<u32, DeformationSkinJointNodeSchema>>,
    inverse_bind_matrices: Option<CanonicalSequenceRef<f32, DeformationInverseBindMatrixSchema>>,
    correction_matrices: Option<CanonicalSequenceRef<f32, DeformationSkinCorrectionMatrixSchema>>,
    names: Option<CanonicalUtf8Ref<DeformationSkinNameSchema>>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.skin_count() == 0).then_some(());
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
    let mapping = expected_nodes.canonical_index_by_local();
    let mut joint_cursor = 0_u32;
    let mut inverse_cursor = 0_u32;
    let mut correction_cursor = 0_u32;
    let mut name_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let skin = expected.skin(row)?;
        let authored = skin.authored();
        let skin_id = u32::try_from(row).ok()?;
        let joint_count = u32::try_from(authored.joints().len()).ok()?;
        let inverse_count = u32::try_from(authored.inverse_bind_matrices().len()).ok()?;
        let inverse_scalar_count = inverse_count.checked_mul(MATRIX_SCALAR_COUNT)?;
        if joint_count != inverse_count
            || skin.joint_offset() != joint_cursor
            || skin.joint_count() != joint_count
            || skin.inverse_bind_scalar_offset() != inverse_cursor
        {
            return None;
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
            observe_work(u64::from(joint_count))?;
            for (stored, authored) in stored.get(range)?.iter().zip(authored.joints()) {
                let local = usize::try_from(authored.index()).ok()?;
                if *stored != *mapping.get(local)? {
                    return None;
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
            let mut scalar_index = 0_usize;
            for matrix in authored.inverse_bind_matrices() {
                for column in matrix.cols {
                    for expected_scalar in column {
                        require_float32_bits(stored.get(scalar_index).copied(), expected_scalar)?;
                        scalar_index = scalar_index.checked_add(1)?;
                    }
                }
            }
            if scalar_index != stored.len() {
                return None;
            }
        }

        let root = match authored.skeleton_root() {
            Presence::Absent => {
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::SKELETON_ROOT_PRESENCE),
                    PRESENCE_ABSENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::SKELETON_ROOT),
                    0_u32,
                )?;
                None
            }
            Presence::Present(root) => {
                let local = usize::try_from(root.index()).ok()?;
                let canonical = *mapping.get(local)?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::SKELETON_ROOT_PRESENCE),
                    PRESENCE_PRESENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::SKELETON_ROOT),
                    canonical,
                )?;
                Some(canonical)
            }
        };
        require_optional_record_reference(
            table,
            row,
            deformation_skin_field::NODES,
            joint_count != 0 || root.is_some(),
            Some(nodes_ref),
        )?;

        let spaces = authored.matrix_spaces();
        let encoded_spaces = encode_deformation_skin_matrix_spaces(spaces);
        for (field, encoded) in [
            deformation_skin_field::INPUT_POSITION_SPACE,
            deformation_skin_field::INVERSE_BIND_FROM_SPACE,
            deformation_skin_field::INVERSE_BIND_TO_SPACE,
            deformation_skin_field::CURRENT_JOINT_FROM_SPACE,
            deformation_skin_field::CURRENT_JOINT_TO_SPACE,
            deformation_skin_field::OUTPUT_POSITION_SPACE,
        ]
        .into_iter()
        .zip(encoded_spaces)
        {
            require_scalar(table.scalar::<u8>(row, field), encoded)?;
        }

        match spaces.correction() {
            None => {
                if skin.correction_matrix_scalar_offset() != 0
                    || skin.correction_matrix_scalar_count() != 0
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_PRESENCE),
                    PRESENCE_ABSENT,
                )?;
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
            Some(correction) => {
                if skin.correction_matrix_scalar_offset() != correction_cursor
                    || skin.correction_matrix_scalar_count() != MATRIX_SCALAR_COUNT
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_PRESENCE),
                    PRESENCE_PRESENT,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_FROM_SPACE),
                    encode_deformation_skin_space(correction.from()),
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_TO_SPACE),
                    encode_deformation_skin_space(correction.to()),
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
                let mut scalar_index = 0_usize;
                for column in correction.matrix().cols {
                    for expected_scalar in column {
                        require_float32_bits(stored.get(scalar_index).copied(), expected_scalar)?;
                        scalar_index = scalar_index.checked_add(1)?;
                    }
                }
                if scalar_index != stored.len() {
                    return None;
                }
                correction_cursor = correction_cursor.checked_add(MATRIX_SCALAR_COUNT)?;
            }
        }

        let name_count = skin.name_byte_count();
        match authored.name() {
            Presence::Absent => {
                if skin.name_byte_offset() != 0 || name_count != 0 {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::NAME_PRESENCE),
                    PRESENCE_ABSENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::NAME_OFFSET),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::NAME_COUNT),
                    0_u32,
                )?;
                require_scalar(table.scalar::<u8>(row, deformation_skin_field::NAMES), 0_u8)?;
            }
            Presence::Present(name) => {
                if skin.name_byte_offset() != name_cursor
                    || usize::try_from(name_count).ok()? != name.as_str().len()
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::NAME_PRESENCE),
                    PRESENCE_PRESENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::NAME_OFFSET),
                    name_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::NAME_COUNT),
                    name_count,
                )?;
                require_optional_utf8_reference(
                    table,
                    row,
                    deformation_skin_field::NAMES,
                    name_count != 0,
                    names,
                )?;
                if name_count != 0 {
                    let stored = name_values?.as_bytes();
                    let range = checked_range(name_cursor, name_count, stored.len())?;
                    observe_work(u64::from(name_count))?;
                    if stored.get(range)? != name.as_str().as_bytes() {
                        return None;
                    }
                }
                name_cursor = name_cursor.checked_add(name_count)?;
            }
        }

        joint_cursor = joint_cursor.checked_add(joint_count)?;
        inverse_cursor = inverse_cursor.checked_add(inverse_scalar_count)?;
    }
    (joint_cursor == expected.joint_node_count()
        && inverse_cursor == expected.inverse_bind_scalar_count()
        && correction_cursor == expected.skin_correction_scalar_count()
        && name_cursor == expected.skin_name_byte_count())
    .then_some(())
}

fn validate_morph_definitions<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<DeformationMorphDefinitionTableSchema>>,
    names: Option<CanonicalUtf8Ref<DeformationMorphNameSchema>>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.morph_definition_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    let name_values = match names {
        Some(reference) => Some(session.canonical_utf8(reference)?),
        None => None,
    };
    let mut name_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let definition = expected.morph_definition(row)?;
        let authored = definition.authored();
        require_scalar(
            table.scalar::<u32>(row, deformation_morph_definition_field::MORPH_DEFINITION),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_morph_definition_field::LOGICAL_MESH),
            definition.logical_mesh(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_morph_definition_field::TARGET_ORDINAL),
            definition.target_ordinal(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_morph_definition_field::STABLE_IDENTITY),
            authored.identity().value(),
        )?;
        match authored.name() {
            Presence::Absent => {
                if definition.name_byte_offset() != 0 || definition.name_byte_count() != 0 {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_morph_definition_field::NAME_PRESENCE),
                    PRESENCE_ABSENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_definition_field::NAME_OFFSET),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_definition_field::NAME_COUNT),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_morph_definition_field::NAMES),
                    0_u8,
                )?;
            }
            Presence::Present(name) => {
                let count = u32::try_from(name.as_str().len()).ok()?;
                if definition.name_byte_offset() != name_cursor
                    || definition.name_byte_count() != count
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_morph_definition_field::NAME_PRESENCE),
                    PRESENCE_PRESENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_definition_field::NAME_OFFSET),
                    name_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_definition_field::NAME_COUNT),
                    count,
                )?;
                require_optional_utf8_reference(
                    table,
                    row,
                    deformation_morph_definition_field::NAMES,
                    count != 0,
                    names,
                )?;
                if count != 0 {
                    let stored = name_values?.as_bytes();
                    let range = checked_range(name_cursor, count, stored.len())?;
                    observe_work(u64::from(count))?;
                    if stored.get(range)? != name.as_str().as_bytes() {
                        return None;
                    }
                }
                name_cursor = name_cursor.checked_add(count)?;
            }
        }
    }
    (name_cursor == expected.morph_name_byte_count()).then_some(())
}

fn validate_logical_morphs<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<DeformationLogicalMorphTableSchema>>,
    definitions: Option<CanonicalRecordRef<DeformationMorphDefinitionTableSchema>>,
    defaults: Option<CanonicalSequenceRef<f32, DeformationMorphDefaultWeightSchema>>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.logical_morph_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    let default_values = match defaults {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut definition_cursor = 0_u32;
    let mut default_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let logical = expected.logical_morph(row)?;
        let definition_range = logical.definitions();
        let authored_defaults = logical.authored().default_weights();
        let default_count = u32::try_from(authored_defaults.len()).ok()?;
        if definition_range.offset() != definition_cursor
            || definition_range.count() != logical.target_count()
            || logical.default_weight_offset() != default_cursor
            || default_count != logical.target_count()
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
            definition_range.count(),
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_logical_morph_field::DEFINITIONS,
            definition_range.count() != 0,
            definitions,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_OFFSET),
            default_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_COUNT),
            default_count,
        )?;
        require_optional_sequence_reference(
            table,
            row,
            deformation_logical_morph_field::DEFAULT_WEIGHTS,
            default_count != 0,
            defaults,
        )?;
        if default_count != 0 {
            let stored = default_values?.as_slice();
            let range = checked_range(default_cursor, default_count, stored.len())?;
            observe_work(u64::from(default_count))?;
            for (stored, expected) in stored.get(range)?.iter().zip(authored_defaults) {
                require_float32_bits(Some(*stored), *expected)?;
            }
        }
        definition_cursor = definition_cursor.checked_add(definition_range.count())?;
        default_cursor = default_cursor.checked_add(default_count)?;
    }
    (definition_cursor == expected.morph_definition_count()
        && default_cursor == expected.morph_default_weight_count())
    .then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_morph_deltas<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<DeformationMorphDeltaTableSchema>>,
    values: Option<CanonicalSequenceRef<f32, DeformationMorphValueSchema>>,
    sparse_indices: Option<CanonicalSequenceRef<u32, DeformationMorphSparseIndexSchema>>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.morph_delta_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    let stored_values = match values {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_indices = match sparse_indices {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let primitive_count = usize::try_from(expected.primitive_count()).ok()?;
    let mut target_cursor = 0_u32;
    let mut delta_cursor = 0_u32;
    let mut value_cursor = 0_u32;
    let mut sparse_cursor = 0_u32;
    for primitive_index in 0..primitive_count {
        observe_work(1)?;
        let primitive = expected.primitive(primitive_index)?;
        for target_index in 0..primitive.morph_target_count() {
            observe_work(1)?;
            let target = primitive.morph_target(target_index)?;
            if target.primitive_index() != u32::try_from(primitive_index).ok()?
                || target.target_ordinal() != u32::try_from(target_index).ok()?
            {
                return None;
            }
            for delta_index in 0..target.delta_count() {
                observe_work(1)?;
                let row = usize::try_from(delta_cursor).ok()?;
                let delta = target.delta(delta_index)?;
                if delta.semantic() != delta.authored().semantic()
                    || delta.value_offset() != value_cursor
                    || delta.sparse_index_offset() != sparse_cursor
                {
                    return None;
                }
                let (sparse, logical_count, explicit_count, authored_values, authored_indices) =
                    match delta.authored_payload() {
                        MorphDeltaPayloadAuthoring::Dense { values } => {
                            let scalar_count = u32::try_from(values.len()).ok()?;
                            if scalar_count % MORPH_VECTOR_WIDTH != 0 {
                                return None;
                            }
                            let logical_count = scalar_count / MORPH_VECTOR_WIDTH;
                            (false, logical_count, logical_count, values, &[][..])
                        }
                        MorphDeltaPayloadAuthoring::Sparse {
                            logical_count,
                            indices,
                            values,
                        } => (
                            true,
                            logical_count,
                            u32::try_from(indices.len()).ok()?,
                            values,
                            indices,
                        ),
                    };
                let payload_kind = if sparse {
                    DeformationMorphPayloadKind::Sparse
                } else {
                    DeformationMorphPayloadKind::Dense
                };
                let value_count = u32::try_from(authored_values.len()).ok()?;
                let sparse_count = u32::try_from(authored_indices.len()).ok()?;
                let expected_value_count = explicit_count.checked_mul(MORPH_VECTOR_WIDTH)?;
                if delta.payload_kind() != payload_kind
                    || delta.logical_count() != logical_count
                    || delta.explicit_count() != explicit_count
                    || value_count != expected_value_count
                    || delta.value_count() != expected_value_count
                    || delta.sparse_index_count() != sparse_count
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
                require_scalar(
                    table.scalar::<u8>(row, deformation_morph_delta_field::SEMANTIC),
                    encode_deformation_morph_semantic(delta.semantic()),
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_morph_delta_field::PAYLOAD_KIND),
                    encode_deformation_morph_payload(sparse),
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::LOGICAL_COUNT),
                    logical_count,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::EXPLICIT_COUNT),
                    explicit_count,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::VALUE_OFFSET),
                    value_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::VALUE_COUNT),
                    value_count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_morph_delta_field::VALUES,
                    value_count != 0,
                    values,
                )?;
                if value_count != 0 {
                    let stored = stored_values?.as_slice();
                    let range = checked_range(value_cursor, value_count, stored.len())?;
                    observe_work(u64::from(value_count))?;
                    for (stored, expected) in stored.get(range)?.iter().zip(authored_values) {
                        require_float32_bits(Some(*stored), *expected)?;
                    }
                }
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::SPARSE_INDEX_OFFSET),
                    sparse_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::SPARSE_INDEX_COUNT),
                    sparse_count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_morph_delta_field::SPARSE_INDICES,
                    sparse_count != 0,
                    sparse_indices,
                )?;
                if sparse_count != 0 {
                    let stored = stored_indices?.as_slice();
                    let range = checked_range(sparse_cursor, sparse_count, stored.len())?;
                    observe_work(u64::from(sparse_count))?;
                    if stored.get(range)? != authored_indices {
                        return None;
                    }
                }
                value_cursor = value_cursor.checked_add(value_count)?;
                sparse_cursor = sparse_cursor.checked_add(sparse_count)?;
                delta_cursor = delta_cursor.checked_add(1)?;
            }
            target_cursor = target_cursor.checked_add(1)?;
        }
    }
    (target_cursor == expected.morph_target_count()
        && delta_cursor == expected.morph_delta_count()
        && value_cursor == expected.morph_value_count()
        && sparse_cursor == expected.morph_sparse_index_count())
    .then_some(())
}

fn validate_morph_targets<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<DeformationMorphTargetTableSchema>>,
    deltas: Option<CanonicalRecordRef<DeformationMorphDeltaTableSchema>>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.morph_target_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    let primitive_count = usize::try_from(expected.primitive_count()).ok()?;
    let mut target_cursor = 0_u32;
    let mut delta_cursor = 0_u32;
    for primitive_index in 0..primitive_count {
        observe_work(1)?;
        let primitive = expected.primitive(primitive_index)?;
        for target_index in 0..primitive.morph_target_count() {
            observe_work(1)?;
            let row = usize::try_from(target_cursor).ok()?;
            let target = primitive.morph_target(target_index)?;
            let primitive_id = u32::try_from(primitive_index).ok()?;
            let target_ordinal = u32::try_from(target_index).ok()?;
            let delta_count = u32::try_from(target.delta_count()).ok()?;
            if target.primitive_index() != primitive_id || target.target_ordinal() != target_ordinal
            {
                return None;
            }
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
                delta_count,
            )?;
            require_optional_record_reference(
                table,
                row,
                deformation_morph_target_field::DELTAS,
                delta_count != 0,
                deltas,
            )?;
            delta_cursor = delta_cursor.checked_add(delta_count)?;
            target_cursor = target_cursor.checked_add(1)?;
        }
    }
    (target_cursor == expected.morph_target_count() && delta_cursor == expected.morph_delta_count())
        .then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_primitives<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<DeformationPrimitiveTableSchema>>,
    row_offsets: Option<CanonicalSequenceRef<u32, DeformationInfluenceRowOffsetSchema>>,
    joints: Option<CanonicalSequenceRef<u32, DeformationInfluenceJointSchema>>,
    weights: Option<CanonicalSequenceRef<f32, DeformationInfluenceWeightSchema>>,
    source_orders: Option<CanonicalSequenceRef<u32, DeformationInfluenceSourceOrderSchema>>,
    morph_targets: Option<CanonicalRecordRef<DeformationMorphTargetTableSchema>>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.primitive_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
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
    for row in 0..table.rows() {
        observe_work(1)?;
        let primitive = expected.primitive(row)?;
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::PRIMITIVE),
            u32::try_from(row).ok()?,
        )?;
        match primitive.authored().influences() {
            Presence::Absent => {
                if primitive.influence_row_offset_offset() != 0
                    || primitive.influence_row_offset_count() != 0
                    || primitive.influence_offset() != 0
                    || primitive.influence_count() != 0
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_primitive_field::INFLUENCE_PRESENCE),
                    PRESENCE_ABSENT,
                )?;
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
            Presence::Present(influences) => {
                let row_count = u32::try_from(influences.vertex_offsets().len()).ok()?;
                let influence_count = u32::try_from(influences.influences().len()).ok()?;
                if primitive.influence_row_offset_offset() != row_offset_cursor
                    || primitive.influence_row_offset_count() != row_count
                    || primitive.influence_offset() != influence_cursor
                    || primitive.influence_count() != influence_count
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_primitive_field::INFLUENCE_PRESENCE),
                    PRESENCE_PRESENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_OFFSET),
                    row_offset_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_COUNT),
                    row_count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::ROW_OFFSETS,
                    row_count != 0,
                    row_offsets,
                )?;
                let stored = stored_row_offsets?.as_slice();
                let range = checked_range(row_offset_cursor, row_count, stored.len())?;
                observe_work(u64::from(row_count))?;
                if stored.get(range)? != influences.vertex_offsets() {
                    return None;
                }
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_OFFSET),
                    influence_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_COUNT),
                    influence_count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    influence_count != 0,
                    joints,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    influence_count != 0,
                    weights,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                    influence_count != 0,
                    source_orders,
                )?;
                if influence_count != 0 {
                    let joint_values = stored_joints?.as_slice();
                    let weight_values = stored_weights?.as_slice();
                    let order_values = stored_source_orders?.as_slice();
                    let joint_range =
                        checked_range(influence_cursor, influence_count, joint_values.len())?;
                    let weight_range =
                        checked_range(influence_cursor, influence_count, weight_values.len())?;
                    let order_range =
                        checked_range(influence_cursor, influence_count, order_values.len())?;
                    observe_work(u64::from(influence_count).checked_mul(3)?)?;
                    for (relative, authored) in influences.influences().iter().enumerate() {
                        let joint = *joint_values.get(joint_range.start.checked_add(relative)?)?;
                        let weight =
                            *weight_values.get(weight_range.start.checked_add(relative)?)?;
                        let source_order =
                            *order_values.get(order_range.start.checked_add(relative)?)?;
                        if joint != authored.joint() || source_order != authored.source_order() {
                            return None;
                        }
                        require_float32_bits(Some(weight), authored.weight())?;
                    }
                }
                row_offset_cursor = row_offset_cursor.checked_add(row_count)?;
                influence_cursor = influence_cursor.checked_add(influence_count)?;
            }
        }
        let target_count = u32::try_from(primitive.morph_target_count()).ok()?;
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_OFFSET),
            target_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_COUNT),
            target_count,
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_primitive_field::MORPH_TARGETS,
            target_count != 0,
            morph_targets,
        )?;
        target_cursor = target_cursor.checked_add(target_count)?;
    }
    (row_offset_cursor == expected.influence_row_offset_count()
        && influence_cursor == expected.influence_count()
        && target_cursor == expected.morph_target_count())
    .then_some(())
}

fn validate_mesh_instances<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    skins: Option<CanonicalRecordRef<DeformationSkinTableSchema>>,
    morph_weights: Option<CanonicalSequenceRef<f32, DeformationInstanceMorphWeightSchema>>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.mesh_instance_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    let stored_weights = match morph_weights {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut weight_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let instance = expected.mesh_instance(row)?;
        let authored = instance.authored();
        require_scalar(
            table.scalar::<u32>(row, deformation_mesh_instance_field::MESH_INSTANCE),
            u32::try_from(row).ok()?,
        )?;
        match authored.skin() {
            Presence::Absent => {
                if instance.skin().is_some() {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_mesh_instance_field::SKIN_PRESENCE),
                    PRESENCE_ABSENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_mesh_instance_field::SKIN),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_mesh_instance_field::SKINS),
                    0_u8,
                )?;
            }
            Presence::Present(skin) => {
                if instance.skin() != Some(skin) {
                    return None;
                }
                require_scalar(
                    table.scalar::<u8>(row, deformation_mesh_instance_field::SKIN_PRESENCE),
                    PRESENCE_PRESENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_mesh_instance_field::SKIN),
                    skin.index(),
                )?;
                require_optional_record_reference(
                    table,
                    row,
                    deformation_mesh_instance_field::SKINS,
                    true,
                    skins,
                )?;
            }
        }
        match authored.morph_weights() {
            Presence::Absent => {
                if instance.morph_weight_offset() != 0 || instance.morph_weight_count() != 0 {
                    return None;
                }
                require_scalar(
                    table
                        .scalar::<u8>(row, deformation_mesh_instance_field::MORPH_WEIGHTS_PRESENCE),
                    PRESENCE_ABSENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_mesh_instance_field::MORPH_WEIGHT_OFFSET),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_mesh_instance_field::MORPH_WEIGHT_COUNT),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_mesh_instance_field::MORPH_WEIGHTS),
                    0_u8,
                )?;
            }
            Presence::Present(weights) => {
                let count = u32::try_from(weights.len()).ok()?;
                if instance.morph_weight_offset() != weight_cursor
                    || instance.morph_weight_count() != count
                {
                    return None;
                }
                require_scalar(
                    table
                        .scalar::<u8>(row, deformation_mesh_instance_field::MORPH_WEIGHTS_PRESENCE),
                    PRESENCE_PRESENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_mesh_instance_field::MORPH_WEIGHT_OFFSET),
                    weight_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_mesh_instance_field::MORPH_WEIGHT_COUNT),
                    count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_mesh_instance_field::MORPH_WEIGHTS,
                    count != 0,
                    morph_weights,
                )?;
                if count != 0 {
                    let stored = stored_weights?.as_slice();
                    let range = checked_range(weight_cursor, count, stored.len())?;
                    observe_work(u64::from(count))?;
                    for (stored, expected) in stored.get(range)?.iter().zip(weights) {
                        require_float32_bits(Some(*stored), *expected)?;
                    }
                }
                weight_cursor = weight_cursor.checked_add(count)?;
            }
        }
    }
    (weight_cursor == expected.instance_morph_weight_count()).then_some(())
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

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
}

fn require_float32_bits(actual: Option<f32>, expected: f32) -> Option<()> {
    (actual?.to_bits() == expected.to_bits()).then_some(())
}
