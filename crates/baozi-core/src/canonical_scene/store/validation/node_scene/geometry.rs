use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef, ScalarSequence,
    StoreValidationSession,
};

use crate::canonical_scene::store::production::{
    GeometryAttributePayloadAuthoring, GeometryAttributePayloadKind, GeometryAttributeScalarKind,
    GeometryTopologyAuthoring, Presence, ValidatedGeometryBatch, ValidatedGeometryPrimitive,
    ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::appearance::AppearanceMaterialTableSchema;
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_ATTRIBUTE_FIELD_COUNT, GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
    GEOMETRY_MESH_INSTANCE_FIELD_COUNT, GEOMETRY_NORMALIZATION_NONE, GEOMETRY_PAYLOAD_DENSE,
    GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_PRIMITIVE_FIELD_COUNT, GEOMETRY_SCALAR_F32,
    GEOMETRY_SCALAR_U32, GeometryAttributeTableSchema, GeometryF32ValuesSchema,
    GeometryLogicalMeshTableSchema, GeometryMeshInstanceTableSchema,
    GeometryPolygonBoundariesSchema, GeometryPrimitiveIndicesSchema, GeometryPrimitiveTableSchema,
    GeometrySparseF32DefaultsSchema, GeometrySparseIndicesSchema, GeometrySparseU32DefaultsSchema,
    GeometryU32ValuesSchema, encode_geometry_domain, encode_geometry_semantic,
    encode_geometry_topology, geometry_attribute_field, geometry_logical_mesh_field,
    geometry_mesh_instance_field, geometry_primitive_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NODE_SCENE_NODE_FIELD_COUNT, NodeSceneNodeTableSchema,
    PRESENCE_ABSENT, PRESENCE_PRESENT, SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT,
    SCENE_GEOMETRY_SCHEMA_REVISION, SceneGeometryDirectorySchema, scene_geometry_directory_field,
};

use super::super::require_scalar;

#[derive(Clone, Copy)]
pub(super) struct GeometryValidatedRefs {
    pub(super) logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    pub(super) primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    pub(super) attributes: Option<CanonicalBlockRef<GeometryAttributeTableSchema>>,
    pub(super) mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
}

pub(super) fn validate_geometry_graph<NR, GR>(
    session: &mut StoreValidationSession<'_, '_>,
    geometry_ref: CanonicalRecordRef<SceneGeometryDirectorySchema>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    appearance_materials: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedGeometryBatch<'_, GR>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<GeometryValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(geometry_ref)?;
    if directory.rows() != 1 || directory.fields() != SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, scene_geometry_directory_field::SCHEMA_REVISION),
        SCENE_GEOMETRY_SCHEMA_REVISION,
    )?;

    let logical_meshes = optional_table_ref::<GeometryLogicalMeshTableSchema>(
        session,
        directory,
        scene_geometry_directory_field::LOGICAL_MESH_COUNT,
        scene_geometry_directory_field::LOGICAL_MESHES,
        expected.logical_mesh_count(),
        GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
    )?;
    let primitives = optional_table_ref::<GeometryPrimitiveTableSchema>(
        session,
        directory,
        scene_geometry_directory_field::PRIMITIVE_COUNT,
        scene_geometry_directory_field::PRIMITIVES,
        expected.primitive_count(),
        GEOMETRY_PRIMITIVE_FIELD_COUNT,
    )?;
    let attributes = optional_table_ref::<GeometryAttributeTableSchema>(
        session,
        directory,
        scene_geometry_directory_field::ATTRIBUTE_COUNT,
        scene_geometry_directory_field::ATTRIBUTES,
        expected.attribute_count(),
        GEOMETRY_ATTRIBUTE_FIELD_COUNT,
    )?;
    let mesh_instances = optional_table_ref::<GeometryMeshInstanceTableSchema>(
        session,
        directory,
        scene_geometry_directory_field::MESH_INSTANCE_COUNT,
        scene_geometry_directory_field::MESH_INSTANCES,
        expected.mesh_instance_count(),
        GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    )?;

    validate_attribute_table(session, attributes, expected, observe_work)?;
    validate_primitive_table(
        session,
        primitives,
        appearance_materials,
        attributes,
        expected,
        observe_work,
    )?;
    validate_logical_mesh_table(session, logical_meshes, primitives, expected, observe_work)?;
    validate_mesh_instance_table(
        session,
        mesh_instances,
        nodes_ref,
        logical_meshes,
        expected_nodes,
        expected,
        observe_work,
    )?;

    Some(GeometryValidatedRefs {
        logical_meshes: logical_meshes.map(CanonicalBlockRef::from_store),
        primitives: primitives.map(CanonicalBlockRef::from_store),
        attributes: attributes.map(CanonicalBlockRef::from_store),
        mesh_instances: mesh_instances.map(CanonicalBlockRef::from_store),
    })
}

fn optional_table_ref<Tag>(
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

fn validate_logical_mesh_table<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryLogicalMeshTableSchema>>,
    primitive_reference: Option<CanonicalRecordRef<GeometryPrimitiveTableSchema>>,
    expected: &ValidatedGeometryBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.logical_mesh_count() == 0).then_some(());
    };
    let primitive_reference = primitive_reference?;
    let table = session.canonical_records(reference)?;
    let logical_meshes = expected.input().logical_meshes();
    for (row, logical_mesh) in logical_meshes.iter().copied().enumerate() {
        observe_work(1)?;
        let range = logical_mesh.primitives();
        require_scalar(
            table.scalar::<u32>(row, geometry_logical_mesh_field::LOGICAL_MESH),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_logical_mesh_field::PRIMITIVE_OFFSET),
            range.offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_logical_mesh_field::PRIMITIVE_COUNT),
            range.count(),
        )?;
        require_scalar(
            table.record_ref::<GeometryPrimitiveTableSchema>(
                row,
                geometry_logical_mesh_field::PRIMITIVES,
            ),
            primitive_reference,
        )?;
    }
    Some(())
}

fn validate_primitive_table<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryPrimitiveTableSchema>>,
    material_reference: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    attribute_reference: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    expected: &ValidatedGeometryBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.primitive_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    for row in 0..table.rows() {
        observe_work(1)?;
        let primitive = expected.primitive(row)?;
        require_scalar(
            table.scalar::<u32>(row, geometry_primitive_field::PRIMITIVE),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u8>(row, geometry_primitive_field::TOPOLOGY),
            encode_geometry_topology(primitive.topology()),
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_primitive_field::VERTEX_COUNT),
            primitive.vertex_count(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_primitive_field::CORNER_COUNT),
            primitive.corner_count(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_primitive_field::FACE_COUNT),
            primitive.face_count(),
        )?;
        validate_polygon_payload(session, table, row, &primitive, observe_work)?;
        validate_index_payload(session, table, row, &primitive, observe_work)?;
        match primitive.material() {
            None => {
                require_scalar(
                    table.scalar::<u8>(row, geometry_primitive_field::MATERIAL_PRESENCE),
                    PRESENCE_ABSENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, geometry_primitive_field::MATERIAL),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, geometry_primitive_field::MATERIALS),
                    0_u8,
                )?;
            }
            Some(material) => {
                require_scalar(
                    table.scalar::<u8>(row, geometry_primitive_field::MATERIAL_PRESENCE),
                    PRESENCE_PRESENT,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, geometry_primitive_field::MATERIAL),
                    material.index(),
                )?;
                require_scalar(
                    table.record_ref::<AppearanceMaterialTableSchema>(
                        row,
                        geometry_primitive_field::MATERIALS,
                    ),
                    material_reference?,
                )?;
            }
        }
        let attribute_range = primitive.attribute_range();
        require_scalar(
            table.scalar::<u32>(row, geometry_primitive_field::ATTRIBUTE_OFFSET),
            attribute_range.offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_primitive_field::ATTRIBUTE_COUNT),
            attribute_range.count(),
        )?;
        if attribute_range.count() == 0 {
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::ATTRIBUTES),
                0_u8,
            )?;
        } else {
            require_scalar(
                table.record_ref::<GeometryAttributeTableSchema>(
                    row,
                    geometry_primitive_field::ATTRIBUTES,
                ),
                attribute_reference?,
            )?;
        }
    }
    Some(())
}

fn validate_polygon_payload(
    session: &mut StoreValidationSession<'_, '_>,
    table: CanonicalRecordView<'_>,
    row: usize,
    primitive: &ValidatedGeometryPrimitive<'_, '_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    match primitive.authored_topology() {
        GeometryTopologyAuthoring::Polygons(boundaries) => {
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::POLYGON_BOUNDARIES_PRESENCE),
                PRESENCE_PRESENT,
            )?;
            require_scalar(
                table.scalar::<u32>(row, geometry_primitive_field::POLYGON_BOUNDARIES_COUNT),
                u32::try_from(boundaries.len()).ok()?,
            )?;
            if boundaries.is_empty() {
                require_scalar(
                    table.scalar::<u8>(row, geometry_primitive_field::POLYGON_BOUNDARIES),
                    0_u8,
                )
            } else {
                let reference = table.sequence_ref::<u32, GeometryPolygonBoundariesSchema>(
                    row,
                    geometry_primitive_field::POLYGON_BOUNDARIES,
                )?;
                require_u32_leaf(session, reference, boundaries, observe_work)
            }
        }
        _ => {
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::POLYGON_BOUNDARIES_PRESENCE),
                PRESENCE_ABSENT,
            )?;
            require_scalar(
                table.scalar::<u32>(row, geometry_primitive_field::POLYGON_BOUNDARIES_COUNT),
                0_u32,
            )?;
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::POLYGON_BOUNDARIES),
                0_u8,
            )
        }
    }
}

fn validate_index_payload(
    session: &mut StoreValidationSession<'_, '_>,
    table: CanonicalRecordView<'_>,
    row: usize,
    primitive: &ValidatedGeometryPrimitive<'_, '_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    match primitive.authored_indices() {
        Presence::Absent => {
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::INDEX_PRESENCE),
                PRESENCE_ABSENT,
            )?;
            require_scalar(
                table.scalar::<u32>(row, geometry_primitive_field::INDEX_COUNT),
                0_u32,
            )?;
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::INDICES),
                0_u8,
            )
        }
        Presence::Present(indices) => {
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::INDEX_PRESENCE),
                PRESENCE_PRESENT,
            )?;
            require_scalar(
                table.scalar::<u32>(row, geometry_primitive_field::INDEX_COUNT),
                u32::try_from(indices.len()).ok()?,
            )?;
            if indices.is_empty() {
                require_scalar(
                    table.scalar::<u8>(row, geometry_primitive_field::INDICES),
                    0_u8,
                )
            } else {
                let reference = table.sequence_ref::<u32, GeometryPrimitiveIndicesSchema>(
                    row,
                    geometry_primitive_field::INDICES,
                )?;
                require_u32_leaf(session, reference, indices, observe_work)
            }
        }
    }
}

fn validate_attribute_table<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    expected: &ValidatedGeometryBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.attribute_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    for row in 0..table.rows() {
        observe_work(1)?;
        let attribute = expected.attribute(row)?;
        let (semantic_kind, semantic_set) = encode_geometry_semantic(attribute.semantic())?;
        let domain = encode_geometry_domain(attribute.domain())?;
        let scalar_kind = match attribute.scalar_kind() {
            GeometryAttributeScalarKind::F32 => GEOMETRY_SCALAR_F32,
            GeometryAttributeScalarKind::U32 => GEOMETRY_SCALAR_U32,
        };
        let payload_kind = match attribute.payload_kind() {
            GeometryAttributePayloadKind::Dense => GEOMETRY_PAYLOAD_DENSE,
            GeometryAttributePayloadKind::Sparse => GEOMETRY_PAYLOAD_SPARSE,
        };

        require_scalar(
            table.scalar::<u32>(row, geometry_attribute_field::ATTRIBUTE),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u8>(row, geometry_attribute_field::SEMANTIC_KIND),
            semantic_kind,
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_SET),
            semantic_set,
        )?;
        require_scalar(
            table.scalar::<u8>(row, geometry_attribute_field::DOMAIN),
            domain,
        )?;
        require_scalar(
            table.scalar::<u8>(row, geometry_attribute_field::SCALAR_KIND),
            scalar_kind,
        )?;
        require_scalar(
            table.scalar::<u8>(row, geometry_attribute_field::LANE_COUNT),
            attribute.lanes(),
        )?;
        require_scalar(
            table.scalar::<u8>(row, geometry_attribute_field::NORMALIZATION),
            GEOMETRY_NORMALIZATION_NONE,
        )?;
        require_scalar(
            table.scalar::<u8>(row, geometry_attribute_field::PAYLOAD_KIND),
            payload_kind,
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_attribute_field::ELEMENT_COUNT),
            attribute.element_count(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_attribute_field::EXPLICIT_COUNT),
            attribute.explicit_element_count(),
        )?;
        require_scalar(
            table.scalar::<u64>(row, geometry_attribute_field::CANONICAL_SCALAR_COUNT),
            attribute.canonical_scalar_count(),
        )?;
        validate_attribute_payload(
            session,
            table,
            row,
            attribute.authored_payload(),
            attribute.canonical_scalar_count(),
            observe_work,
        )?;
    }
    Some(())
}

fn validate_attribute_payload(
    session: &mut StoreValidationSession<'_, '_>,
    table: CanonicalRecordView<'_>,
    row: usize,
    payload: GeometryAttributePayloadAuthoring<'_>,
    canonical_scalar_count: u64,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    match payload {
        GeometryAttributePayloadAuthoring::DenseF32 {
            lanes,
            source_values,
            source_indices,
        } => {
            match source_indices {
                Presence::Absent if source_values.is_empty() => {
                    require_absent_leaf(table, row, geometry_attribute_field::VALUES)?
                }
                Presence::Absent => {
                    let reference = table.sequence_ref::<f32, GeometryF32ValuesSchema>(
                        row,
                        geometry_attribute_field::VALUES,
                    )?;
                    require_f32_leaf_bits(session, reference, source_values, observe_work)?;
                }
                Presence::Present([]) => {
                    require_absent_leaf(table, row, geometry_attribute_field::VALUES)?
                }
                Presence::Present(indices) => {
                    let reference = table.sequence_ref::<f32, GeometryF32ValuesSchema>(
                        row,
                        geometry_attribute_field::VALUES,
                    )?;
                    require_projected_f32_leaf(
                        session,
                        reference,
                        source_values,
                        indices,
                        lanes,
                        canonical_scalar_count,
                        observe_work,
                    )?;
                }
            }
            require_absent_leaf(table, row, geometry_attribute_field::SPARSE_INDICES)?;
            require_absent_leaf(table, row, geometry_attribute_field::SPARSE_DEFAULT)
        }
        GeometryAttributePayloadAuthoring::DenseU32 { values, .. } => {
            if values.is_empty() {
                require_absent_leaf(table, row, geometry_attribute_field::VALUES)?;
            } else {
                let reference = table.sequence_ref::<u32, GeometryU32ValuesSchema>(
                    row,
                    geometry_attribute_field::VALUES,
                )?;
                require_u32_leaf(session, reference, values, observe_work)?;
            }
            require_absent_leaf(table, row, geometry_attribute_field::SPARSE_INDICES)?;
            require_absent_leaf(table, row, geometry_attribute_field::SPARSE_DEFAULT)
        }
        GeometryAttributePayloadAuthoring::SparseF32 {
            indices,
            values,
            default,
            ..
        } => {
            if values.is_empty() {
                require_absent_leaf(table, row, geometry_attribute_field::VALUES)?;
            } else {
                let reference = table.sequence_ref::<f32, GeometryF32ValuesSchema>(
                    row,
                    geometry_attribute_field::VALUES,
                )?;
                require_f32_leaf_bits(session, reference, values, observe_work)?;
            }
            validate_sparse_indices(session, table, row, indices, observe_work)?;
            if default.is_empty() {
                require_absent_leaf(table, row, geometry_attribute_field::SPARSE_DEFAULT)
            } else {
                let reference = table.sequence_ref::<f32, GeometrySparseF32DefaultsSchema>(
                    row,
                    geometry_attribute_field::SPARSE_DEFAULT,
                )?;
                require_f32_leaf_bits(session, reference, default, observe_work)
            }
        }
        GeometryAttributePayloadAuthoring::SparseU32 {
            indices,
            values,
            default,
            ..
        } => {
            if values.is_empty() {
                require_absent_leaf(table, row, geometry_attribute_field::VALUES)?;
            } else {
                let reference = table.sequence_ref::<u32, GeometryU32ValuesSchema>(
                    row,
                    geometry_attribute_field::VALUES,
                )?;
                require_u32_leaf(session, reference, values, observe_work)?;
            }
            validate_sparse_indices(session, table, row, indices, observe_work)?;
            if default.is_empty() {
                require_absent_leaf(table, row, geometry_attribute_field::SPARSE_DEFAULT)
            } else {
                let reference = table.sequence_ref::<u32, GeometrySparseU32DefaultsSchema>(
                    row,
                    geometry_attribute_field::SPARSE_DEFAULT,
                )?;
                require_u32_leaf(session, reference, default, observe_work)
            }
        }
    }
}

fn validate_sparse_indices(
    session: &mut StoreValidationSession<'_, '_>,
    table: CanonicalRecordView<'_>,
    row: usize,
    indices: &[u32],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    if indices.is_empty() {
        require_absent_leaf(table, row, geometry_attribute_field::SPARSE_INDICES)
    } else {
        let reference = table.sequence_ref::<u32, GeometrySparseIndicesSchema>(
            row,
            geometry_attribute_field::SPARSE_INDICES,
        )?;
        require_u32_leaf(session, reference, indices, observe_work)
    }
}

fn validate_mesh_instance_table<NR, GR>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryMeshInstanceTableSchema>>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    logical_mesh_reference: Option<CanonicalRecordRef<GeometryLogicalMeshTableSchema>>,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedGeometryBatch<'_, GR>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.mesh_instance_count() == 0).then_some(());
    };
    let logical_mesh_reference = logical_mesh_reference?;
    let nodes = session.canonical_records(nodes_ref)?;
    if nodes.rows() != usize::try_from(expected_nodes.canonical_node_count()).ok()?
        || nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT
    {
        return None;
    }
    let table = session.canonical_records(reference)?;
    let mapping = expected_nodes.canonical_index_by_local();
    for (row, instance) in expected
        .input()
        .mesh_instances()
        .iter()
        .copied()
        .enumerate()
    {
        observe_work(1)?;
        let owner_local = usize::try_from(instance.owner().index()).ok()?;
        let owner_node = *mapping.get(owner_local)?;
        require_scalar(
            table.scalar::<u32>(row, geometry_mesh_instance_field::MESH_INSTANCE),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_mesh_instance_field::OWNER_NODE),
            owner_node,
        )?;
        require_scalar(
            table.record_ref::<NodeSceneNodeTableSchema>(row, geometry_mesh_instance_field::NODES),
            nodes_ref,
        )?;
        require_scalar(
            table.scalar::<u32>(row, geometry_mesh_instance_field::LOGICAL_MESH),
            instance.logical_mesh().index(),
        )?;
        require_scalar(
            table.record_ref::<GeometryLogicalMeshTableSchema>(
                row,
                geometry_mesh_instance_field::LOGICAL_MESHES,
            ),
            logical_mesh_reference,
        )?;
    }
    Some(())
}

fn require_absent_leaf(table: CanonicalRecordView<'_>, row: usize, field: usize) -> Option<()> {
    require_scalar(table.scalar::<u8>(row, field), 0_u8)
}

fn require_u32_leaf<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: CanonicalSequenceRef<u32, Tag>,
    expected: &[u32],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<u32>>,
{
    let stored = session.canonical_sequence(reference)?;
    let stored = stored.as_slice();
    if stored.len() != expected.len() {
        return None;
    }
    for (&actual, &expected) in stored.iter().zip(expected) {
        observe_work(1)?;
        require_scalar(Some(actual), expected)?;
    }
    Some(())
}

fn require_f32_leaf_bits<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: CanonicalSequenceRef<f32, Tag>,
    expected: &[f32],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    let stored = session.canonical_sequence(reference)?;
    let stored = stored.as_slice();
    if stored.len() != expected.len() {
        return None;
    }
    for (&actual, &expected) in stored.iter().zip(expected) {
        observe_work(1)?;
        require_scalar(Some(actual.to_bits()), expected.to_bits())?;
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn require_projected_f32_leaf(
    session: &mut StoreValidationSession<'_, '_>,
    reference: CanonicalSequenceRef<f32, GeometryF32ValuesSchema>,
    source_values: &[f32],
    source_indices: &[u32],
    lanes: u8,
    expected_scalar_count: u64,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let lane_count = usize::from(lanes);
    if lane_count == 0 {
        return None;
    }
    let expected_len = usize::try_from(expected_scalar_count).ok()?;
    let projected_len = source_indices.len().checked_mul(lane_count)?;
    if projected_len != expected_len {
        return None;
    }
    let stored = session.canonical_sequence(reference)?;
    let stored = stored.as_slice();
    if stored.len() != expected_len {
        return None;
    }

    let mut cursor = 0_usize;
    for &source_index in source_indices {
        let source_start = usize::try_from(source_index)
            .ok()?
            .checked_mul(lane_count)?;
        let source_end = source_start.checked_add(lane_count)?;
        let expected_values = source_values.get(source_start..source_end)?;
        for &expected in expected_values {
            observe_work(1)?;
            let actual = *stored.get(cursor)?;
            require_scalar(Some(actual.to_bits()), expected.to_bits())?;
            cursor = cursor.checked_add(1)?;
        }
    }
    (cursor == stored.len()).then_some(())
}
