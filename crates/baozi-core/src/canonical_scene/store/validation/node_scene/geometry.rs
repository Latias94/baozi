use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, ScalarSequence, StoreValidationSession,
};

use crate::canonical_scene::store::production::{
    GeometryAttributePayloadAuthoring, GeometryAttributePayloadKind, GeometryAttributeScalarKind,
    GeometryTopologyAuthoring, Presence, ValidatedGeometryBatch, ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::appearance::AppearanceMaterialTableSchema;
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_ATTRIBUTE_FIELD_COUNT, GEOMETRY_DOMAIN_CORNER, GEOMETRY_DOMAIN_FACE,
    GEOMETRY_DOMAIN_VERTEX, GEOMETRY_LOGICAL_MESH_FIELD_COUNT, GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    GEOMETRY_NORMALIZATION_NONE, GEOMETRY_PAYLOAD_DENSE, GEOMETRY_PAYLOAD_SPARSE,
    GEOMETRY_PRIMITIVE_FIELD_COUNT, GEOMETRY_SCALAR_F32, GEOMETRY_SCALAR_U32,
    GEOMETRY_SEMANTIC_COLOR, GEOMETRY_SEMANTIC_NORMAL, GEOMETRY_SEMANTIC_POSITION,
    GEOMETRY_SEMANTIC_SMOOTHING_GROUP, GEOMETRY_SEMANTIC_TANGENT, GEOMETRY_SEMANTIC_TEXCOORD,
    GEOMETRY_TOPOLOGY_LINE_LIST, GEOMETRY_TOPOLOGY_LINE_LOOP, GEOMETRY_TOPOLOGY_LINE_STRIP,
    GEOMETRY_TOPOLOGY_POINTS, GEOMETRY_TOPOLOGY_POLYGONS, GEOMETRY_TOPOLOGY_TRIANGLE_FAN,
    GEOMETRY_TOPOLOGY_TRIANGLE_LIST, GEOMETRY_TOPOLOGY_TRIANGLE_STRIP,
    GeometryAttributeTableSchema, GeometryF32ValuesSchema, GeometryLogicalMeshTableSchema,
    GeometryMeshInstanceTableSchema, GeometryPolygonBoundariesSchema,
    GeometryPrimitiveIndicesSchema, GeometryPrimitiveTableSchema, GeometrySparseF32DefaultsSchema,
    GeometrySparseIndicesSchema, GeometrySparseU32DefaultsSchema, GeometryU32ValuesSchema,
    encode_geometry_domain, encode_geometry_semantic, encode_geometry_topology,
    geometry_attribute_field, geometry_logical_mesh_field, geometry_mesh_instance_field,
    geometry_primitive_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NodeSceneNodeTableSchema, PRESENCE_ABSENT,
    PRESENCE_PRESENT, SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT, SCENE_GEOMETRY_SCHEMA_REVISION,
    SceneGeometryDirectorySchema, scene_geometry_directory_field,
};

use super::super::require_scalar;

#[derive(Clone, Copy)]
pub(super) struct GeometryValidatedRefs {
    pub(super) logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    pub(super) primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    pub(super) attributes: Option<CanonicalBlockRef<GeometryAttributeTableSchema>>,
    pub(super) mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
}

#[derive(Clone, Copy)]
pub(super) struct GeometryValidatedShape {
    pub(super) refs: GeometryValidatedRefs,
    pub(super) payload_block_count: u32,
}

#[derive(Clone, Copy)]
pub(super) struct GeometryCanonicalCounts {
    logical_meshes: u32,
    primitives: u32,
    attributes: u32,
    mesh_instances: u32,
}

impl GeometryCanonicalCounts {
    pub(super) const fn new(
        logical_meshes: u32,
        primitives: u32,
        attributes: u32,
        mesh_instances: u32,
    ) -> Self {
        Self {
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct GeometryCanonicalDependencies {
    nodes: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    canonical_node_count: u32,
    materials: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    material_count: u32,
}

impl GeometryCanonicalDependencies {
    pub(super) const fn new(
        nodes: CanonicalRecordRef<NodeSceneNodeTableSchema>,
        canonical_node_count: u32,
        materials: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
        material_count: u32,
    ) -> Self {
        Self {
            nodes,
            canonical_node_count,
            materials,
            material_count,
        }
    }
}

#[derive(Clone, Copy)]
enum CanonicalPresence<T> {
    Absent,
    Present(T),
}

#[derive(Clone, Copy)]
enum CanonicalGeometryAttributePayload<'a> {
    DenseF32 {
        values: &'a [f32],
    },
    DenseU32 {
        values: &'a [u32],
    },
    SparseF32 {
        indices: &'a [u32],
        values: &'a [f32],
        default: &'a [f32],
    },
    SparseU32 {
        indices: &'a [u32],
        values: &'a [u32],
        default: &'a [u32],
    },
}

#[derive(Clone, Copy)]
enum GeometryAttributePayloadExpectation<'a> {
    StructuralOnly,
    Exact(GeometryAttributePayloadAuthoring<'a>),
}

#[derive(Clone, Copy)]
enum F32LeafExpectation<'a> {
    Exact(&'a [f32]),
    Projected {
        source_values: &'a [f32],
        source_indices: &'a [u32],
        lanes: u8,
    },
}

impl CanonicalGeometryAttributePayload<'_> {
    fn block_count(self) -> u32 {
        match self {
            Self::DenseF32 { values } => u32::from(!values.is_empty()),
            Self::DenseU32 { values } => u32::from(!values.is_empty()),
            Self::SparseF32 {
                indices,
                values,
                default,
            } => {
                u32::from(!indices.is_empty())
                    + u32::from(!values.is_empty())
                    + u32::from(!default.is_empty())
            }
            Self::SparseU32 {
                indices,
                values,
                default,
            } => {
                u32::from(!indices.is_empty())
                    + u32::from(!values.is_empty())
                    + u32::from(!default.is_empty())
            }
        }
    }
}

#[derive(Clone, Copy)]
struct CanonicalGeometryPrimitivePayload<'a> {
    polygon_boundaries: CanonicalPresence<&'a [u32]>,
    indices: CanonicalPresence<&'a [u32]>,
}

#[derive(Clone, Copy)]
enum GeometryPrimitivePayloadExpectation<'a> {
    StructuralOnly,
    Exact {
        topology: GeometryTopologyAuthoring<'a>,
        indices: Presence<&'a [u32]>,
    },
}

impl CanonicalGeometryPrimitivePayload<'_> {
    fn block_count(self) -> u32 {
        u32::from(matches!(
            self.polygon_boundaries,
            CanonicalPresence::Present(values) if !values.is_empty()
        )) + u32::from(matches!(
            self.indices,
            CanonicalPresence::Present(values) if !values.is_empty()
        ))
    }
}

trait GeometryEquivalence {
    fn validate_directory(&mut self, directory: CanonicalRecordView<'_>) -> Option<()>;

    fn validate_attribute(&mut self, row: usize, table: CanonicalRecordView<'_>) -> Option<()>;

    fn attribute_payload(&self, row: usize) -> Option<GeometryAttributePayloadExpectation<'_>>;

    fn validate_primitive(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        materials: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
        attributes: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    ) -> Option<()>;

    fn primitive_payload(&self, row: usize) -> Option<GeometryPrimitivePayloadExpectation<'_>>;

    fn requires_attribute_range_scan(&self) -> bool;

    fn validate_logical_mesh(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        primitives: CanonicalRecordRef<GeometryPrimitiveTableSchema>,
    ) -> Option<()>;

    fn validate_mesh_instance(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        nodes: CanonicalRecordRef<NodeSceneNodeTableSchema>,
        logical_meshes: CanonicalRecordRef<GeometryLogicalMeshTableSchema>,
    ) -> Option<()>;
}

struct CanonicalGeometryEquivalence;

impl GeometryEquivalence for CanonicalGeometryEquivalence {
    fn validate_directory(&mut self, _directory: CanonicalRecordView<'_>) -> Option<()> {
        Some(())
    }

    fn validate_attribute(&mut self, _row: usize, _table: CanonicalRecordView<'_>) -> Option<()> {
        Some(())
    }

    fn attribute_payload(&self, _row: usize) -> Option<GeometryAttributePayloadExpectation<'_>> {
        Some(GeometryAttributePayloadExpectation::StructuralOnly)
    }

    fn validate_primitive(
        &mut self,
        _row: usize,
        _table: CanonicalRecordView<'_>,
        _materials: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
        _attributes: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    ) -> Option<()> {
        Some(())
    }

    fn primitive_payload(&self, _row: usize) -> Option<GeometryPrimitivePayloadExpectation<'_>> {
        Some(GeometryPrimitivePayloadExpectation::StructuralOnly)
    }

    fn requires_attribute_range_scan(&self) -> bool {
        true
    }

    fn validate_logical_mesh(
        &mut self,
        _row: usize,
        _table: CanonicalRecordView<'_>,
        _primitives: CanonicalRecordRef<GeometryPrimitiveTableSchema>,
    ) -> Option<()> {
        Some(())
    }

    fn validate_mesh_instance(
        &mut self,
        _row: usize,
        _table: CanonicalRecordView<'_>,
        _nodes: CanonicalRecordRef<NodeSceneNodeTableSchema>,
        _logical_meshes: CanonicalRecordRef<GeometryLogicalMeshTableSchema>,
    ) -> Option<()> {
        Some(())
    }
}

struct FreshGeometryEquivalence<'batch, 'node, 'input, NR, GR> {
    expected_nodes: &'batch ValidatedNodeBatch<'node, NR>,
    expected: &'batch ValidatedGeometryBatch<'input, GR>,
}

impl<NR, GR> GeometryEquivalence for FreshGeometryEquivalence<'_, '_, '_, NR, GR> {
    fn validate_directory(&mut self, directory: CanonicalRecordView<'_>) -> Option<()> {
        for (field, expected) in [
            (
                scene_geometry_directory_field::LOGICAL_MESH_COUNT,
                self.expected.logical_mesh_count(),
            ),
            (
                scene_geometry_directory_field::PRIMITIVE_COUNT,
                self.expected.primitive_count(),
            ),
            (
                scene_geometry_directory_field::ATTRIBUTE_COUNT,
                self.expected.attribute_count(),
            ),
            (
                scene_geometry_directory_field::MESH_INSTANCE_COUNT,
                self.expected.mesh_instance_count(),
            ),
        ] {
            require_scalar(directory.scalar::<u32>(0, field), expected)?;
        }
        Some(())
    }

    fn validate_attribute(&mut self, row: usize, table: CanonicalRecordView<'_>) -> Option<()> {
        validate_attribute_fresh(row, table, self.expected)
    }

    fn attribute_payload(&self, row: usize) -> Option<GeometryAttributePayloadExpectation<'_>> {
        Some(GeometryAttributePayloadExpectation::Exact(
            self.expected.attribute(row)?.authored_payload(),
        ))
    }

    fn validate_primitive(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        materials: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
        attributes: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    ) -> Option<()> {
        validate_primitive_fresh(row, table, materials, attributes, self.expected)
    }

    fn primitive_payload(&self, row: usize) -> Option<GeometryPrimitivePayloadExpectation<'_>> {
        let primitive = self.expected.primitive(row)?;
        Some(GeometryPrimitivePayloadExpectation::Exact {
            topology: primitive.authored_topology(),
            indices: primitive.authored_indices(),
        })
    }

    fn requires_attribute_range_scan(&self) -> bool {
        false
    }

    fn validate_logical_mesh(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        primitives: CanonicalRecordRef<GeometryPrimitiveTableSchema>,
    ) -> Option<()> {
        validate_logical_mesh_fresh(row, table, primitives, self.expected)
    }

    fn validate_mesh_instance(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        nodes: CanonicalRecordRef<NodeSceneNodeTableSchema>,
        logical_meshes: CanonicalRecordRef<GeometryLogicalMeshTableSchema>,
    ) -> Option<()> {
        validate_mesh_instance_fresh(
            row,
            table,
            nodes,
            logical_meshes,
            self.expected_nodes,
            self.expected,
        )
    }
}

pub(super) fn validate_geometry_canonical_structure(
    session: &mut StoreValidationSession<'_, '_>,
    geometry_ref: CanonicalRecordRef<SceneGeometryDirectorySchema>,
    dependencies: GeometryCanonicalDependencies,
    counts: GeometryCanonicalCounts,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<GeometryValidatedShape> {
    validate_geometry_graph_inner(
        session,
        geometry_ref,
        dependencies,
        counts,
        &mut CanonicalGeometryEquivalence,
        observe_work,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "fresh geometry validation keeps canonical dependencies and both source proof batches explicit"
)]
pub(super) fn validate_geometry_graph<NR, GR>(
    session: &mut StoreValidationSession<'_, '_>,
    geometry_ref: CanonicalRecordRef<SceneGeometryDirectorySchema>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    appearance_materials: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    appearance_material_count: u32,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedGeometryBatch<'_, GR>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<GeometryValidatedRefs> {
    validate_geometry_graph_inner(
        session,
        geometry_ref,
        GeometryCanonicalDependencies::new(
            nodes_ref,
            u32::try_from(expected_nodes.canonical_node_count()).ok()?,
            appearance_materials,
            appearance_material_count,
        ),
        GeometryCanonicalCounts::new(
            expected.logical_mesh_count(),
            expected.primitive_count(),
            expected.attribute_count(),
            expected.mesh_instance_count(),
        ),
        &mut FreshGeometryEquivalence {
            expected_nodes,
            expected,
        },
        observe_work,
    )
    .map(|shape| shape.refs)
}

fn validate_geometry_graph_inner(
    session: &mut StoreValidationSession<'_, '_>,
    geometry_ref: CanonicalRecordRef<SceneGeometryDirectorySchema>,
    dependencies: GeometryCanonicalDependencies,
    counts: GeometryCanonicalCounts,
    equivalence: &mut impl GeometryEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<GeometryValidatedShape> {
    observe_work(1)?;
    let directory = session.canonical_records(geometry_ref)?;
    if directory.rows() != 1 || directory.fields() != SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, scene_geometry_directory_field::SCHEMA_REVISION),
        SCENE_GEOMETRY_SCHEMA_REVISION,
    )?;
    equivalence.validate_directory(directory)?;

    let logical_meshes = optional_table_ref::<GeometryLogicalMeshTableSchema>(
        session,
        directory,
        scene_geometry_directory_field::LOGICAL_MESH_COUNT,
        scene_geometry_directory_field::LOGICAL_MESHES,
        counts.logical_meshes,
        GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
    )?;
    let primitives = optional_table_ref::<GeometryPrimitiveTableSchema>(
        session,
        directory,
        scene_geometry_directory_field::PRIMITIVE_COUNT,
        scene_geometry_directory_field::PRIMITIVES,
        counts.primitives,
        GEOMETRY_PRIMITIVE_FIELD_COUNT,
    )?;
    let attributes = optional_table_ref::<GeometryAttributeTableSchema>(
        session,
        directory,
        scene_geometry_directory_field::ATTRIBUTE_COUNT,
        scene_geometry_directory_field::ATTRIBUTES,
        counts.attributes,
        GEOMETRY_ATTRIBUTE_FIELD_COUNT,
    )?;
    let mesh_instances = optional_table_ref::<GeometryMeshInstanceTableSchema>(
        session,
        directory,
        scene_geometry_directory_field::MESH_INSTANCE_COUNT,
        scene_geometry_directory_field::MESH_INSTANCES,
        counts.mesh_instances,
        GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    )?;

    let (attribute_table, attribute_payload_block_count) =
        validate_attribute_table(session, attributes, equivalence, observe_work)?;
    let primitive_payload_block_count = validate_primitive_table(
        session,
        primitives,
        dependencies.materials,
        dependencies.material_count,
        attributes,
        attribute_table,
        equivalence,
        observe_work,
    )?;
    validate_logical_mesh_table(
        session,
        logical_meshes,
        primitives,
        counts.primitives,
        equivalence,
        observe_work,
    )?;
    validate_mesh_instance_table(
        session,
        mesh_instances,
        dependencies.nodes,
        dependencies.canonical_node_count,
        logical_meshes,
        counts.logical_meshes,
        equivalence,
        observe_work,
    )?;

    Some(GeometryValidatedShape {
        refs: GeometryValidatedRefs {
            logical_meshes: logical_meshes.map(CanonicalBlockRef::from_store),
            primitives: primitives.map(CanonicalBlockRef::from_store),
            attributes: attributes.map(CanonicalBlockRef::from_store),
            mesh_instances: mesh_instances.map(CanonicalBlockRef::from_store),
        },
        payload_block_count: attribute_payload_block_count
            .checked_add(primitive_payload_block_count)?,
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

fn validate_logical_mesh_table(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryLogicalMeshTableSchema>>,
    primitive_reference: Option<CanonicalRecordRef<GeometryPrimitiveTableSchema>>,
    primitive_count: u32,
    equivalence: &mut impl GeometryEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (primitive_count == 0).then_some(());
    };
    let primitive_reference = primitive_reference?;
    let table = session.canonical_records(reference)?;
    let mut primitive_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, geometry_logical_mesh_field::LOGICAL_MESH),
            u32::try_from(row).ok()?,
        )?;
        let offset = table.scalar::<u32>(row, geometry_logical_mesh_field::PRIMITIVE_OFFSET)?;
        let count = table.scalar::<u32>(row, geometry_logical_mesh_field::PRIMITIVE_COUNT)?;
        require_scalar(Some(offset), primitive_cursor)?;
        if count == 0 {
            return None;
        }
        primitive_cursor = offset.checked_add(count)?;
        if primitive_cursor > primitive_count {
            return None;
        }
        require_scalar(
            table.record_ref::<GeometryPrimitiveTableSchema>(
                row,
                geometry_logical_mesh_field::PRIMITIVES,
            ),
            primitive_reference,
        )?;
        equivalence.validate_logical_mesh(row, table, primitive_reference)?;
    }
    (primitive_cursor == primitive_count).then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_primitive_table(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryPrimitiveTableSchema>>,
    material_reference: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    material_count: u32,
    attribute_reference: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    attribute_table: Option<CanonicalRecordView<'_>>,
    equivalence: &mut impl GeometryEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<u32> {
    let Some(reference) = reference else {
        return attribute_reference.is_none().then_some(0);
    };
    let table = session.canonical_records(reference)?;
    let attribute_rows = attribute_table.map_or(0, CanonicalRecordView::rows);
    let mut attribute_cursor = 0_usize;
    let mut payload_block_count = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, geometry_primitive_field::PRIMITIVE),
            u32::try_from(row).ok()?,
        )?;
        let topology = table.scalar::<u8>(row, geometry_primitive_field::TOPOLOGY)?;
        let vertex_count = table.scalar::<u32>(row, geometry_primitive_field::VERTEX_COUNT)?;
        let corner_count = table.scalar::<u32>(row, geometry_primitive_field::CORNER_COUNT)?;
        let face_count = table.scalar::<u32>(row, geometry_primitive_field::FACE_COUNT)?;
        let payload_expectation = equivalence.primitive_payload(row)?;
        let polygon_boundaries = validate_polygon_payload_structure(
            session,
            table,
            row,
            topology,
            corner_count,
            face_count,
            payload_expectation,
            observe_work,
        )?;
        let indices = validate_index_payload_structure(
            session,
            table,
            row,
            vertex_count,
            corner_count,
            payload_expectation,
            observe_work,
        )?;
        let payload = CanonicalGeometryPrimitivePayload {
            polygon_boundaries,
            indices,
        };
        payload_block_count = payload_block_count.checked_add(payload.block_count())?;
        validate_topology_shape(topology, corner_count, face_count, polygon_boundaries)?;

        match table.scalar::<u8>(row, geometry_primitive_field::MATERIAL_PRESENCE)? {
            PRESENCE_ABSENT => {
                require_scalar(
                    table.scalar::<u32>(row, geometry_primitive_field::MATERIAL),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, geometry_primitive_field::MATERIALS),
                    0_u8,
                )?;
            }
            PRESENCE_PRESENT => {
                let material = table.scalar::<u32>(row, geometry_primitive_field::MATERIAL)?;
                if material >= material_count {
                    return None;
                }
                require_scalar(
                    table.record_ref::<AppearanceMaterialTableSchema>(
                        row,
                        geometry_primitive_field::MATERIALS,
                    ),
                    material_reference?,
                )?;
            }
            _ => return None,
        }
        let attribute_offset =
            table.scalar::<u32>(row, geometry_primitive_field::ATTRIBUTE_OFFSET)?;
        let attribute_count =
            table.scalar::<u32>(row, geometry_primitive_field::ATTRIBUTE_COUNT)?;
        require_scalar(
            Some(attribute_offset),
            u32::try_from(attribute_cursor).ok()?,
        )?;
        let attribute_range = checked_range(attribute_offset, attribute_count, attribute_rows)?;
        attribute_cursor = attribute_range.end;
        if attribute_count == 0 {
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::ATTRIBUTES),
                0_u8,
            )?;
            if vertex_count != 0 {
                return None;
            }
        } else {
            let attributes = attribute_reference?;
            require_scalar(
                table.record_ref::<GeometryAttributeTableSchema>(
                    row,
                    geometry_primitive_field::ATTRIBUTES,
                ),
                attributes,
            )?;
            if equivalence.requires_attribute_range_scan() {
                validate_primitive_attribute_range(
                    attribute_table?,
                    attribute_range,
                    vertex_count,
                    corner_count,
                    face_count,
                    observe_work,
                )?;
            }
        }
        equivalence.validate_primitive(row, table, material_reference, attribute_reference)?;
    }
    (attribute_cursor == attribute_rows).then_some(payload_block_count)
}

#[allow(
    clippy::too_many_arguments,
    reason = "polygon payload validation keeps its canonical row coordinates and bounded work authority explicit"
)]
fn validate_polygon_payload_structure<'session, 'storage>(
    session: &mut StoreValidationSession<'session, 'storage>,
    table: CanonicalRecordView<'storage>,
    row: usize,
    topology: u8,
    corner_count: u32,
    face_count: u32,
    expectation: GeometryPrimitivePayloadExpectation<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<CanonicalPresence<&'storage [u32]>> {
    let presence =
        table.scalar::<u8>(row, geometry_primitive_field::POLYGON_BOUNDARIES_PRESENCE)?;
    let count = table.scalar::<u32>(row, geometry_primitive_field::POLYGON_BOUNDARIES_COUNT)?;
    let expected = match expectation {
        GeometryPrimitivePayloadExpectation::StructuralOnly => None,
        GeometryPrimitivePayloadExpectation::Exact { topology, .. } => Some(match topology {
            GeometryTopologyAuthoring::Polygons(boundaries) => {
                CanonicalPresence::Present(boundaries)
            }
            _ => CanonicalPresence::Absent,
        }),
    };
    match (topology, presence) {
        (GEOMETRY_TOPOLOGY_POLYGONS, PRESENCE_PRESENT) => {
            require_scalar(Some(count), face_count)?;
            let expected = match expected {
                Some(CanonicalPresence::Present(expected)) => {
                    require_scalar(Some(count), u32::try_from(expected.len()).ok()?)?;
                    Some(expected)
                }
                Some(CanonicalPresence::Absent) => return None,
                None => None,
            };
            if count == 0 {
                require_scalar(
                    table.scalar::<u8>(row, geometry_primitive_field::POLYGON_BOUNDARIES),
                    0_u8,
                )?;
                Some(CanonicalPresence::Present(&[]))
            } else {
                let reference = table.sequence_ref::<u32, GeometryPolygonBoundariesSchema>(
                    row,
                    geometry_primitive_field::POLYGON_BOUNDARIES,
                )?;
                let boundaries = session.canonical_sequence(reference)?.as_slice();
                if boundaries.len() != usize::try_from(count).ok()? {
                    return None;
                }
                let mut total = 0_u32;
                for (index, &boundary) in boundaries.iter().enumerate() {
                    observe_work(1)?;
                    if boundary < 3 {
                        return None;
                    }
                    if expected.is_some_and(|expected| expected.get(index) != Some(&boundary)) {
                        return None;
                    }
                    total = total.checked_add(boundary)?;
                }
                (total == corner_count).then_some(CanonicalPresence::Present(boundaries))
            }
        }
        (GEOMETRY_TOPOLOGY_POLYGONS, _) => None,
        (_, PRESENCE_ABSENT) => {
            if matches!(expected, Some(CanonicalPresence::Present(_))) {
                return None;
            }
            require_scalar(Some(count), 0_u32)?;
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::POLYGON_BOUNDARIES),
                0_u8,
            )?;
            Some(CanonicalPresence::Absent)
        }
        _ => None,
    }
}

fn validate_index_payload_structure<'session, 'storage>(
    session: &mut StoreValidationSession<'session, 'storage>,
    table: CanonicalRecordView<'storage>,
    row: usize,
    vertex_count: u32,
    corner_count: u32,
    expectation: GeometryPrimitivePayloadExpectation<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<CanonicalPresence<&'storage [u32]>> {
    let presence = table.scalar::<u8>(row, geometry_primitive_field::INDEX_PRESENCE)?;
    let count = table.scalar::<u32>(row, geometry_primitive_field::INDEX_COUNT)?;
    let expected = match expectation {
        GeometryPrimitivePayloadExpectation::StructuralOnly => None,
        GeometryPrimitivePayloadExpectation::Exact { indices, .. } => Some(match indices {
            Presence::Absent => CanonicalPresence::Absent,
            Presence::Present(indices) => CanonicalPresence::Present(indices),
        }),
    };
    match presence {
        PRESENCE_ABSENT => {
            if matches!(expected, Some(CanonicalPresence::Present(_))) {
                return None;
            }
            require_scalar(Some(count), 0_u32)?;
            require_scalar(Some(corner_count), vertex_count)?;
            require_scalar(
                table.scalar::<u8>(row, geometry_primitive_field::INDICES),
                0_u8,
            )?;
            Some(CanonicalPresence::Absent)
        }
        PRESENCE_PRESENT => {
            require_scalar(Some(count), corner_count)?;
            let expected = match expected {
                Some(CanonicalPresence::Present(expected)) => {
                    require_scalar(Some(count), u32::try_from(expected.len()).ok()?)?;
                    Some(expected)
                }
                Some(CanonicalPresence::Absent) => return None,
                None => None,
            };
            if count == 0 {
                require_scalar(
                    table.scalar::<u8>(row, geometry_primitive_field::INDICES),
                    0_u8,
                )?;
                Some(CanonicalPresence::Present(&[]))
            } else {
                let reference = table.sequence_ref::<u32, GeometryPrimitiveIndicesSchema>(
                    row,
                    geometry_primitive_field::INDICES,
                )?;
                let indices = session.canonical_sequence(reference)?.as_slice();
                if indices.len() != usize::try_from(count).ok()? {
                    return None;
                }
                for (position, &index) in indices.iter().enumerate() {
                    observe_work(1)?;
                    if index >= vertex_count {
                        return None;
                    }
                    if expected.is_some_and(|expected| expected.get(position) != Some(&index)) {
                        return None;
                    }
                }
                Some(CanonicalPresence::Present(indices))
            }
        }
        _ => None,
    }
}

fn validate_topology_shape(
    topology: u8,
    corner_count: u32,
    face_count: u32,
    polygon_boundaries: CanonicalPresence<&[u32]>,
) -> Option<()> {
    let expected_face_count = match topology {
        GEOMETRY_TOPOLOGY_POINTS => corner_count,
        GEOMETRY_TOPOLOGY_LINE_LIST => {
            if !corner_count.is_multiple_of(2) {
                return None;
            }
            corner_count / 2
        }
        GEOMETRY_TOPOLOGY_LINE_STRIP => corner_count.saturating_sub(1),
        GEOMETRY_TOPOLOGY_LINE_LOOP => {
            if corner_count < 2 {
                0
            } else {
                corner_count
            }
        }
        GEOMETRY_TOPOLOGY_TRIANGLE_LIST => {
            if !corner_count.is_multiple_of(3) {
                return None;
            }
            corner_count / 3
        }
        GEOMETRY_TOPOLOGY_TRIANGLE_STRIP | GEOMETRY_TOPOLOGY_TRIANGLE_FAN => {
            corner_count.saturating_sub(2)
        }
        GEOMETRY_TOPOLOGY_POLYGONS => match polygon_boundaries {
            CanonicalPresence::Present(boundaries) => u32::try_from(boundaries.len()).ok()?,
            CanonicalPresence::Absent => return None,
        },
        _ => return None,
    };
    require_scalar(Some(face_count), expected_face_count)
}

fn validate_primitive_attribute_range(
    table: CanonicalRecordView<'_>,
    range: std::ops::Range<usize>,
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let mut previous_key = None;
    let mut has_position = false;
    for row in range {
        observe_work(1)?;
        let semantic = table.scalar::<u8>(row, geometry_attribute_field::SEMANTIC_KIND)?;
        let semantic_set = table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_SET)?;
        let domain = table.scalar::<u8>(row, geometry_attribute_field::DOMAIN)?;
        let key = (semantic, semantic_set, domain);
        if previous_key.is_some_and(|previous| previous >= key) {
            return None;
        }
        previous_key = Some(key);

        let expected_element_count = match domain {
            GEOMETRY_DOMAIN_VERTEX => vertex_count,
            GEOMETRY_DOMAIN_CORNER => corner_count,
            GEOMETRY_DOMAIN_FACE => face_count,
            _ => return None,
        };
        require_scalar(
            table.scalar::<u32>(row, geometry_attribute_field::ELEMENT_COUNT),
            expected_element_count,
        )?;
        has_position |= semantic == GEOMETRY_SEMANTIC_POSITION;
    }
    (vertex_count == 0 || has_position).then_some(())
}

fn validate_attribute_table<'session, 'storage>(
    session: &mut StoreValidationSession<'session, 'storage>,
    reference: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    equivalence: &mut impl GeometryEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<(Option<CanonicalRecordView<'storage>>, u32)> {
    let Some(reference) = reference else {
        return Some((None, 0));
    };
    let table = session.canonical_records(reference)?;
    let mut payload_block_count = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, geometry_attribute_field::ATTRIBUTE),
            u32::try_from(row).ok()?,
        )?;
        let semantic = table.scalar::<u8>(row, geometry_attribute_field::SEMANTIC_KIND)?;
        let semantic_set = table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_SET)?;
        let domain = table.scalar::<u8>(row, geometry_attribute_field::DOMAIN)?;
        let scalar_kind = table.scalar::<u8>(row, geometry_attribute_field::SCALAR_KIND)?;
        let lanes = table.scalar::<u8>(row, geometry_attribute_field::LANE_COUNT)?;
        require_scalar(
            table.scalar::<u8>(row, geometry_attribute_field::NORMALIZATION),
            GEOMETRY_NORMALIZATION_NONE,
        )?;
        validate_attribute_schema(semantic, semantic_set, domain, scalar_kind, lanes)?;
        let payload_kind = table.scalar::<u8>(row, geometry_attribute_field::PAYLOAD_KIND)?;
        let element_count = table.scalar::<u32>(row, geometry_attribute_field::ELEMENT_COUNT)?;
        let explicit_count = table.scalar::<u32>(row, geometry_attribute_field::EXPLICIT_COUNT)?;
        let scalar_count =
            table.scalar::<u64>(row, geometry_attribute_field::CANONICAL_SCALAR_COUNT)?;
        equivalence.validate_attribute(row, table)?;
        let payload_expectation = equivalence.attribute_payload(row)?;
        let payload = validate_attribute_payload_structure(
            session,
            table,
            row,
            scalar_kind,
            payload_kind,
            lanes,
            element_count,
            explicit_count,
            scalar_count,
            payload_expectation,
            observe_work,
        )?;
        payload_block_count = payload_block_count.checked_add(payload.block_count())?;
    }
    Some((Some(table), payload_block_count))
}

#[allow(clippy::too_many_arguments)]
fn validate_attribute_payload_structure<'session, 'storage>(
    session: &mut StoreValidationSession<'session, 'storage>,
    table: CanonicalRecordView<'storage>,
    row: usize,
    scalar_kind: u8,
    payload_kind: u8,
    lanes: u8,
    element_count: u32,
    explicit_count: u32,
    canonical_scalar_count: u64,
    expectation: GeometryAttributePayloadExpectation<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<CanonicalGeometryAttributePayload<'storage>> {
    let lane_count = usize::from(lanes);
    if lane_count == 0 {
        return None;
    }
    let element_count_usize = usize::try_from(element_count).ok()?;
    let explicit_count_usize = usize::try_from(explicit_count).ok()?;
    let dense_scalar_count = element_count_usize.checked_mul(lane_count)?;
    let explicit_scalar_count = explicit_count_usize.checked_mul(lane_count)?;
    match (payload_kind, scalar_kind) {
        (GEOMETRY_PAYLOAD_DENSE, GEOMETRY_SCALAR_F32) => {
            require_scalar(Some(explicit_count), element_count)?;
            require_scalar(
                Some(canonical_scalar_count),
                u64::try_from(dense_scalar_count).ok()?,
            )?;
            let expected = match expectation {
                GeometryAttributePayloadExpectation::StructuralOnly => None,
                GeometryAttributePayloadExpectation::Exact(
                    GeometryAttributePayloadAuthoring::DenseF32 {
                        lanes,
                        source_values,
                        source_indices,
                    },
                ) => Some(match source_indices {
                    Presence::Absent => F32LeafExpectation::Exact(source_values),
                    Presence::Present(source_indices) => F32LeafExpectation::Projected {
                        source_values,
                        source_indices,
                        lanes,
                    },
                }),
                GeometryAttributePayloadExpectation::Exact(_) => return None,
            };
            let values = read_f32_leaf::<GeometryF32ValuesSchema>(
                session,
                table,
                row,
                geometry_attribute_field::VALUES,
                dense_scalar_count,
                expected,
                observe_work,
            )?;
            require_absent_leaf(table, row, geometry_attribute_field::SPARSE_INDICES)?;
            require_absent_leaf(table, row, geometry_attribute_field::SPARSE_DEFAULT)?;
            Some(CanonicalGeometryAttributePayload::DenseF32 { values })
        }
        (GEOMETRY_PAYLOAD_DENSE, GEOMETRY_SCALAR_U32) => {
            require_scalar(Some(explicit_count), element_count)?;
            require_scalar(
                Some(canonical_scalar_count),
                u64::try_from(dense_scalar_count).ok()?,
            )?;
            let expected = match expectation {
                GeometryAttributePayloadExpectation::StructuralOnly => None,
                GeometryAttributePayloadExpectation::Exact(
                    GeometryAttributePayloadAuthoring::DenseU32 { values, .. },
                ) => Some(values),
                GeometryAttributePayloadExpectation::Exact(_) => return None,
            };
            let values = read_u32_leaf::<GeometryU32ValuesSchema>(
                session,
                table,
                row,
                geometry_attribute_field::VALUES,
                dense_scalar_count,
                expected,
                None,
                observe_work,
            )?;
            require_absent_leaf(table, row, geometry_attribute_field::SPARSE_INDICES)?;
            require_absent_leaf(table, row, geometry_attribute_field::SPARSE_DEFAULT)?;
            Some(CanonicalGeometryAttributePayload::DenseU32 { values })
        }
        (GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_SCALAR_F32) => {
            if explicit_count > element_count {
                return None;
            }
            require_scalar(
                Some(canonical_scalar_count),
                u64::try_from(explicit_scalar_count.checked_add(lane_count)?).ok()?,
            )?;
            let (expected_indices, expected_values, expected_default) = match expectation {
                GeometryAttributePayloadExpectation::StructuralOnly => (None, None, None),
                GeometryAttributePayloadExpectation::Exact(
                    GeometryAttributePayloadAuthoring::SparseF32 {
                        indices,
                        values,
                        default,
                        ..
                    },
                ) => (
                    Some(indices),
                    Some(F32LeafExpectation::Exact(values)),
                    Some(F32LeafExpectation::Exact(default)),
                ),
                GeometryAttributePayloadExpectation::Exact(_) => return None,
            };
            let values = read_f32_leaf::<GeometryF32ValuesSchema>(
                session,
                table,
                row,
                geometry_attribute_field::VALUES,
                explicit_scalar_count,
                expected_values,
                observe_work,
            )?;
            let indices = read_sparse_indices(
                session,
                table,
                row,
                explicit_count_usize,
                element_count,
                expected_indices,
                observe_work,
            )?;
            let default = read_f32_leaf::<GeometrySparseF32DefaultsSchema>(
                session,
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                lane_count,
                expected_default,
                observe_work,
            )?;
            Some(CanonicalGeometryAttributePayload::SparseF32 {
                indices,
                values,
                default,
            })
        }
        (GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_SCALAR_U32) => {
            if explicit_count > element_count {
                return None;
            }
            require_scalar(
                Some(canonical_scalar_count),
                u64::try_from(explicit_scalar_count.checked_add(lane_count)?).ok()?,
            )?;
            let (expected_indices, expected_values, expected_default) = match expectation {
                GeometryAttributePayloadExpectation::StructuralOnly => (None, None, None),
                GeometryAttributePayloadExpectation::Exact(
                    GeometryAttributePayloadAuthoring::SparseU32 {
                        indices,
                        values,
                        default,
                        ..
                    },
                ) => (Some(indices), Some(values), Some(default)),
                GeometryAttributePayloadExpectation::Exact(_) => return None,
            };
            let values = read_u32_leaf::<GeometryU32ValuesSchema>(
                session,
                table,
                row,
                geometry_attribute_field::VALUES,
                explicit_scalar_count,
                expected_values,
                None,
                observe_work,
            )?;
            let indices = read_sparse_indices(
                session,
                table,
                row,
                explicit_count_usize,
                element_count,
                expected_indices,
                observe_work,
            )?;
            let default = read_u32_leaf::<GeometrySparseU32DefaultsSchema>(
                session,
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                lane_count,
                expected_default,
                None,
                observe_work,
            )?;
            Some(CanonicalGeometryAttributePayload::SparseU32 {
                indices,
                values,
                default,
            })
        }
        _ => None,
    }
}

fn validate_attribute_schema(
    semantic: u8,
    semantic_set: u32,
    domain: u8,
    scalar_kind: u8,
    lanes: u8,
) -> Option<()> {
    let valid = match semantic {
        GEOMETRY_SEMANTIC_POSITION => {
            semantic_set == 0
                && domain == GEOMETRY_DOMAIN_VERTEX
                && scalar_kind == GEOMETRY_SCALAR_F32
                && lanes == 3
        }
        GEOMETRY_SEMANTIC_NORMAL => {
            semantic_set == 0
                && matches!(domain, GEOMETRY_DOMAIN_VERTEX | GEOMETRY_DOMAIN_CORNER)
                && scalar_kind == GEOMETRY_SCALAR_F32
                && lanes == 3
        }
        GEOMETRY_SEMANTIC_TANGENT => {
            semantic_set == 0
                && matches!(domain, GEOMETRY_DOMAIN_VERTEX | GEOMETRY_DOMAIN_CORNER)
                && scalar_kind == GEOMETRY_SCALAR_F32
                && lanes == 4
        }
        GEOMETRY_SEMANTIC_TEXCOORD => {
            matches!(domain, GEOMETRY_DOMAIN_VERTEX | GEOMETRY_DOMAIN_CORNER)
                && scalar_kind == GEOMETRY_SCALAR_F32
                && (1..=3).contains(&lanes)
        }
        GEOMETRY_SEMANTIC_COLOR => {
            matches!(
                domain,
                GEOMETRY_DOMAIN_VERTEX | GEOMETRY_DOMAIN_CORNER | GEOMETRY_DOMAIN_FACE
            ) && scalar_kind == GEOMETRY_SCALAR_F32
                && matches!(lanes, 3 | 4)
        }
        GEOMETRY_SEMANTIC_SMOOTHING_GROUP => {
            semantic_set == 0
                && domain == GEOMETRY_DOMAIN_FACE
                && scalar_kind == GEOMETRY_SCALAR_U32
                && lanes == 1
        }
        _ => false,
    };
    valid.then_some(())
}

impl F32LeafExpectation<'_> {
    fn len(self) -> Option<usize> {
        match self {
            Self::Exact(values) => Some(values.len()),
            Self::Projected {
                source_indices,
                lanes,
                ..
            } => source_indices.len().checked_mul(usize::from(lanes)),
        }
    }

    fn value(self, index: usize) -> Option<f32> {
        match self {
            Self::Exact(values) => values.get(index).copied(),
            Self::Projected {
                source_values,
                source_indices,
                lanes,
            } => {
                let lanes = usize::from(lanes);
                if lanes == 0 {
                    return None;
                }
                let source_element = usize::try_from(*source_indices.get(index / lanes)?).ok()?;
                let source_index = source_element
                    .checked_mul(lanes)?
                    .checked_add(index % lanes)?;
                source_values.get(source_index).copied()
            }
        }
    }
}

fn read_f32_leaf<'session, 'storage, Tag>(
    session: &mut StoreValidationSession<'session, 'storage>,
    table: CanonicalRecordView<'storage>,
    row: usize,
    field: usize,
    expected_len: usize,
    expectation: Option<F32LeafExpectation<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage [f32]>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    if expectation.is_some_and(|expectation| expectation.len() != Some(expected_len)) {
        return None;
    }
    if expected_len == 0 {
        require_absent_leaf(table, row, field)?;
        return Some(&[]);
    }
    let reference = table.sequence_ref::<f32, Tag>(row, field)?;
    let values = session.canonical_sequence(reference)?.as_slice();
    if values.len() != expected_len {
        return None;
    }
    for (index, &value) in values.iter().enumerate() {
        observe_work(1)?;
        if !value.is_finite() {
            return None;
        }
        if let Some(expectation) = expectation {
            require_scalar(Some(value.to_bits()), expectation.value(index)?.to_bits())?;
        }
    }
    Some(values)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the leaf reader keeps schema coordinates, semantic constraints, and work accounting explicit"
)]
fn read_u32_leaf<'session, 'storage, Tag>(
    session: &mut StoreValidationSession<'session, 'storage>,
    table: CanonicalRecordView<'storage>,
    row: usize,
    field: usize,
    expected_len: usize,
    expectation: Option<&[u32]>,
    strictly_increasing_below: Option<u32>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage [u32]>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<u32>>,
{
    if expectation.is_some_and(|expectation| expectation.len() != expected_len) {
        return None;
    }
    if expected_len == 0 {
        require_absent_leaf(table, row, field)?;
        return Some(&[]);
    }
    let reference = table.sequence_ref::<u32, Tag>(row, field)?;
    let values = session.canonical_sequence(reference)?.as_slice();
    if values.len() != expected_len {
        return None;
    }
    let mut previous = None;
    for (index, &value) in values.iter().enumerate() {
        observe_work(1)?;
        if expectation.is_some_and(|expectation| expectation.get(index) != Some(&value)) {
            return None;
        }
        if let Some(limit) = strictly_increasing_below {
            if value >= limit || previous.is_some_and(|previous| value <= previous) {
                return None;
            }
            previous = Some(value);
        }
    }
    Some(values)
}

fn read_sparse_indices<'session, 'storage>(
    session: &mut StoreValidationSession<'session, 'storage>,
    table: CanonicalRecordView<'storage>,
    row: usize,
    expected_len: usize,
    element_count: u32,
    expectation: Option<&[u32]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage [u32]> {
    read_u32_leaf::<GeometrySparseIndicesSchema>(
        session,
        table,
        row,
        geometry_attribute_field::SPARSE_INDICES,
        expected_len,
        expectation,
        Some(element_count),
        observe_work,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_mesh_instance_table(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryMeshInstanceTableSchema>>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    canonical_node_count: u32,
    logical_mesh_reference: Option<CanonicalRecordRef<GeometryLogicalMeshTableSchema>>,
    logical_mesh_count: u32,
    equivalence: &mut impl GeometryEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return Some(());
    };
    let logical_mesh_reference = logical_mesh_reference?;
    let table = session.canonical_records(reference)?;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, geometry_mesh_instance_field::MESH_INSTANCE),
            u32::try_from(row).ok()?,
        )?;
        let owner_node = table.scalar::<u32>(row, geometry_mesh_instance_field::OWNER_NODE)?;
        if owner_node == 0 || owner_node >= canonical_node_count {
            return None;
        }
        require_scalar(
            table.record_ref::<NodeSceneNodeTableSchema>(row, geometry_mesh_instance_field::NODES),
            nodes_ref,
        )?;
        let logical_mesh = table.scalar::<u32>(row, geometry_mesh_instance_field::LOGICAL_MESH)?;
        if logical_mesh >= logical_mesh_count {
            return None;
        }
        require_scalar(
            table.record_ref::<GeometryLogicalMeshTableSchema>(
                row,
                geometry_mesh_instance_field::LOGICAL_MESHES,
            ),
            logical_mesh_reference,
        )?;
        equivalence.validate_mesh_instance(row, table, nodes_ref, logical_mesh_reference)?;
    }
    Some(())
}

fn require_absent_leaf(table: CanonicalRecordView<'_>, row: usize, field: usize) -> Option<()> {
    require_scalar(table.scalar::<u8>(row, field), 0_u8)
}

fn validate_attribute_fresh<R>(
    row: usize,
    table: CanonicalRecordView<'_>,
    expected: &ValidatedGeometryBatch<'_, R>,
) -> Option<()> {
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
    )
}

fn validate_primitive_fresh<R>(
    row: usize,
    table: CanonicalRecordView<'_>,
    materials: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    attributes: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    expected: &ValidatedGeometryBatch<'_, R>,
) -> Option<()> {
    let primitive = expected.primitive(row)?;
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
    match primitive.material() {
        None => require_scalar(
            table.scalar::<u8>(row, geometry_primitive_field::MATERIAL_PRESENCE),
            PRESENCE_ABSENT,
        )?,
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
                materials?,
            )?;
        }
    }
    let range = primitive.attribute_range();
    require_scalar(
        table.scalar::<u32>(row, geometry_primitive_field::ATTRIBUTE_OFFSET),
        range.offset(),
    )?;
    require_scalar(
        table.scalar::<u32>(row, geometry_primitive_field::ATTRIBUTE_COUNT),
        range.count(),
    )?;
    if range.count() != 0 {
        require_scalar(
            table.record_ref::<GeometryAttributeTableSchema>(
                row,
                geometry_primitive_field::ATTRIBUTES,
            ),
            attributes?,
        )?;
    }
    Some(())
}

fn validate_logical_mesh_fresh<R>(
    row: usize,
    table: CanonicalRecordView<'_>,
    primitives: CanonicalRecordRef<GeometryPrimitiveTableSchema>,
    expected: &ValidatedGeometryBatch<'_, R>,
) -> Option<()> {
    let logical_mesh = expected.input().logical_meshes().get(row)?.primitives();
    require_scalar(
        table.scalar::<u32>(row, geometry_logical_mesh_field::PRIMITIVE_OFFSET),
        logical_mesh.offset(),
    )?;
    require_scalar(
        table.scalar::<u32>(row, geometry_logical_mesh_field::PRIMITIVE_COUNT),
        logical_mesh.count(),
    )?;
    require_scalar(
        table.record_ref::<GeometryPrimitiveTableSchema>(
            row,
            geometry_logical_mesh_field::PRIMITIVES,
        ),
        primitives,
    )
}

fn validate_mesh_instance_fresh<NR, GR>(
    row: usize,
    table: CanonicalRecordView<'_>,
    nodes: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    logical_meshes: CanonicalRecordRef<GeometryLogicalMeshTableSchema>,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedGeometryBatch<'_, GR>,
) -> Option<()> {
    let instance = expected.input().mesh_instances().get(row)?;
    let owner_local = usize::try_from(instance.owner().index()).ok()?;
    let owner_node = *expected_nodes.canonical_index_by_local().get(owner_local)?;
    require_scalar(
        table.scalar::<u32>(row, geometry_mesh_instance_field::OWNER_NODE),
        owner_node,
    )?;
    require_scalar(
        table.record_ref::<NodeSceneNodeTableSchema>(row, geometry_mesh_instance_field::NODES),
        nodes,
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
        logical_meshes,
    )
}

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
}
