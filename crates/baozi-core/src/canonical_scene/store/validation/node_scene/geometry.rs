use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationMember, CanonicalValidationSession,
};
use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalUtf8Ref, OperationOwner, ProgressPhase,
    ProgressUnit, RecordBlock, ScalarSequence, StoreValidationSession, StoreWorkError, Utf8Block,
};

use crate::attribute::CustomAttributeSemantic;
use crate::canonical_scene::store::production::{
    GeometryAttributePayloadAuthoring, GeometryAttributePayloadKind, GeometryAttributeScalarKind,
    GeometryTopologyAuthoring, Presence, ValidatedGeometryBatch, ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::appearance::AppearanceMaterialTableSchema;
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_ATTRIBUTE_FIELD_COUNT, GEOMETRY_DOMAIN_CORNER, GEOMETRY_DOMAIN_FACE,
    GEOMETRY_DOMAIN_VERTEX, GEOMETRY_LOGICAL_MESH_FIELD_COUNT, GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    GEOMETRY_NORMALIZATION_NONE, GEOMETRY_PAYLOAD_DENSE, GEOMETRY_PAYLOAD_SPARSE,
    GEOMETRY_PRIMITIVE_FIELD_COUNT, GEOMETRY_SCALAR_F32, GEOMETRY_SCALAR_I32, GEOMETRY_SCALAR_U32,
    GEOMETRY_SEMANTIC_COLOR, GEOMETRY_SEMANTIC_CUSTOM, GEOMETRY_SEMANTIC_NORMAL,
    GEOMETRY_SEMANTIC_POSITION, GEOMETRY_SEMANTIC_SMOOTHING_GROUP, GEOMETRY_SEMANTIC_TANGENT,
    GEOMETRY_SEMANTIC_TEXCOORD, GEOMETRY_TOPOLOGY_LINE_LIST, GEOMETRY_TOPOLOGY_LINE_LOOP,
    GEOMETRY_TOPOLOGY_LINE_STRIP, GEOMETRY_TOPOLOGY_POINTS, GEOMETRY_TOPOLOGY_POLYGONS,
    GEOMETRY_TOPOLOGY_TRIANGLE_FAN, GEOMETRY_TOPOLOGY_TRIANGLE_LIST,
    GEOMETRY_TOPOLOGY_TRIANGLE_STRIP, GeometryAttributeSemanticNameSchema,
    GeometryAttributeTableSchema, GeometryF32ValuesSchema, GeometryI32ValuesSchema,
    GeometryLogicalMeshNameSchema, GeometryLogicalMeshTableSchema, GeometryMeshInstanceTableSchema,
    GeometryPolygonBoundariesSchema, GeometryPrimitiveIndicesSchema, GeometryPrimitiveTableSchema,
    GeometrySparseF32DefaultsSchema, GeometrySparseI32DefaultsSchema, GeometrySparseIndicesSchema,
    GeometrySparseU32DefaultsSchema, GeometryU32ValuesSchema, encode_geometry_domain,
    encode_geometry_semantic, encode_geometry_topology, geometry_attribute_field,
    geometry_logical_mesh_field, geometry_mesh_instance_field, geometry_primitive_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NodeSceneNodeTableSchema, PRESENCE_ABSENT,
    PRESENCE_PRESENT, SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT, SCENE_GEOMETRY_SCHEMA_REVISION,
    SceneGeometryDirectorySchema, scene_geometry_directory_field,
};
use crate::entity::{MAX_ENTITY_NAME_BYTES, validate_entity_name_str};
use crate::namespaced::{MAX_NAMESPACED_NAME_LEN, NamespacedName};

use super::super::require_scalar;

#[derive(Clone, Copy)]
pub(super) struct GeometryValidatedRefs {
    pub(super) logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    pub(super) logical_mesh_names: Option<CanonicalBlockRef<GeometryLogicalMeshNameSchema>>,
    pub(super) attribute_semantic_names:
        Option<CanonicalBlockRef<GeometryAttributeSemanticNameSchema>>,
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
pub(super) struct GeometryValidatedMembers<'session> {
    pub(super) logical_meshes:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryLogicalMeshTableSchema>>,
    pub(super) primitives:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryPrimitiveTableSchema>>,
    pub(super) attributes:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryAttributeTableSchema>>,
    pub(super) mesh_instances:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryMeshInstanceTableSchema>>,
    pub(super) logical_mesh_names:
        Option<CanonicalValidationMember<'session, Utf8Block, GeometryLogicalMeshNameSchema>>,
    pub(super) attribute_semantic_names:
        Option<CanonicalValidationMember<'session, Utf8Block, GeometryAttributeSemanticNameSchema>>,
}

#[derive(Clone, Copy)]
pub(super) struct GeometryValidatedMemberShape<'session> {
    pub(super) members: GeometryValidatedMembers<'session>,
    pub(super) payload_block_count: u32,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryCanonicalCounts {
    logical_meshes: u32,
    primitives: u32,
    attributes: u32,
    mesh_instances: u32,
}

impl GeometryCanonicalCounts {
    pub(in crate::canonical_scene::store) const fn new(
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
pub(super) struct GeometryCanonicalValidationDependencies<'session> {
    nodes: CanonicalValidationMember<'session, RecordBlock, NodeSceneNodeTableSchema>,
    canonical_node_count: u32,
    materials:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceMaterialTableSchema>>,
    material_count: u32,
}

impl<'session> GeometryCanonicalValidationDependencies<'session> {
    pub(super) const fn new(
        nodes: CanonicalValidationMember<'session, RecordBlock, NodeSceneNodeTableSchema>,
        canonical_node_count: u32,
        materials: Option<
            CanonicalValidationMember<'session, RecordBlock, AppearanceMaterialTableSchema>,
        >,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct GeometryValidationScratchRequirements {
    attributes: usize,
    primitives: usize,
    logical_meshes: usize,
    mesh_instances: usize,
}

impl GeometryValidationScratchRequirements {
    pub(in crate::canonical_scene::store) fn from_counts(
        counts: GeometryCanonicalCounts,
    ) -> Option<Self> {
        Some(Self {
            attributes: usize::try_from(counts.attributes).ok()?,
            primitives: usize::try_from(counts.primitives).ok()?,
            logical_meshes: usize::try_from(counts.logical_meshes).ok()?,
            mesh_instances: usize::try_from(counts.mesh_instances).ok()?,
        })
    }

    pub(in crate::canonical_scene::store) const fn attributes(self) -> usize {
        self.attributes
    }

    pub(in crate::canonical_scene::store) const fn primitives(self) -> usize {
        self.primitives
    }

    pub(in crate::canonical_scene::store) const fn logical_meshes(self) -> usize {
        self.logical_meshes
    }

    pub(in crate::canonical_scene::store) const fn mesh_instances(self) -> usize {
        self.mesh_instances
    }

    pub(in crate::canonical_scene::store) fn initialization_work_units(self) -> Option<u64> {
        [
            self.attributes,
            self.primitives,
            self.logical_meshes,
            self.primitives,
            self.mesh_instances,
        ]
        .into_iter()
        .try_fold(0_u64, |total, count| {
            total.checked_add(u64::try_from(count).ok()?)
        })
    }

    pub(in crate::canonical_scene::store) fn semantic_projection_work_units(self) -> Option<u64> {
        self.attributes
            .checked_mul(2)?
            .checked_add(self.primitives.checked_mul(2)?)
            .and_then(|count| u64::try_from(count).ok())
    }
}

#[derive(Clone, Copy)]
enum GeometryAttributeValuesMember<'session> {
    Absent,
    F32(CanonicalValidationMember<'session, ScalarSequence<f32>, GeometryF32ValuesSchema>),
    U32(CanonicalValidationMember<'session, ScalarSequence<u32>, GeometryU32ValuesSchema>),
    I32(CanonicalValidationMember<'session, ScalarSequence<i32>, GeometryI32ValuesSchema>),
}

#[derive(Clone, Copy)]
enum GeometryAttributeDefaultMember<'session> {
    Absent,
    F32(CanonicalValidationMember<'session, ScalarSequence<f32>, GeometrySparseF32DefaultsSchema>),
    U32(CanonicalValidationMember<'session, ScalarSequence<u32>, GeometrySparseU32DefaultsSchema>),
    I32(CanonicalValidationMember<'session, ScalarSequence<i32>, GeometrySparseI32DefaultsSchema>),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct GeometryAttributeSemanticName {
    bytes: [u8; MAX_NAMESPACED_NAME_LEN],
    len: u8,
}

impl GeometryAttributeSemanticName {
    fn try_new(value: &str) -> Option<Self> {
        NamespacedName::new(value).ok()?;
        let len = u8::try_from(value.len()).ok()?;
        let mut bytes = [0_u8; MAX_NAMESPACED_NAME_LEN];
        bytes
            .get_mut(..value.len())?
            .copy_from_slice(value.as_bytes());
        Some(Self { bytes, len })
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryAttributeValidationScratch<'session> {
    semantic: u8,
    semantic_set: u32,
    semantic_name_offset: u32,
    semantic_name_count: u32,
    semantic_name: Option<GeometryAttributeSemanticName>,
    domain: u8,
    scalar_kind: u8,
    lanes: u8,
    payload_kind: u8,
    element_count: u32,
    explicit_count: u32,
    canonical_scalar_count: u64,
    values: GeometryAttributeValuesMember<'session>,
    sparse_indices: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, GeometrySparseIndicesSchema>,
    >,
    sparse_default: GeometryAttributeDefaultMember<'session>,
    sparse_maximum: Option<u32>,
}

impl<'session> GeometryAttributeValidationScratch<'session> {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        semantic: 0,
        semantic_set: 0,
        semantic_name_offset: 0,
        semantic_name_count: 0,
        semantic_name: None,
        domain: 0,
        scalar_kind: 0,
        lanes: 0,
        payload_kind: 0,
        element_count: 0,
        explicit_count: 0,
        canonical_scalar_count: 0,
        values: GeometryAttributeValuesMember::Absent,
        sparse_indices: None,
        sparse_default: GeometryAttributeDefaultMember::Absent,
        sparse_maximum: None,
    };
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryPrimitiveValidationScratch<'session> {
    topology: u8,
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
    polygon_present: bool,
    polygon_count: u32,
    polygon_boundaries: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, GeometryPolygonBoundariesSchema>,
    >,
    polygon_total: Option<u32>,
    indices_present: bool,
    index_count: u32,
    indices: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, GeometryPrimitiveIndicesSchema>,
    >,
    index_maximum: Option<u32>,
    material: Option<u32>,
    materials:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceMaterialTableSchema>>,
    attribute_offset: u32,
    attribute_count: u32,
    attributes:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryAttributeTableSchema>>,
}

impl<'session> GeometryPrimitiveValidationScratch<'session> {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        topology: 0,
        vertex_count: 0,
        corner_count: 0,
        face_count: 0,
        polygon_present: false,
        polygon_count: 0,
        polygon_boundaries: None,
        polygon_total: None,
        indices_present: false,
        index_count: 0,
        indices: None,
        index_maximum: None,
        material: None,
        materials: None,
        attribute_offset: 0,
        attribute_count: 0,
        attributes: None,
    };
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryLogicalMeshValidationProjection {
    pub(super) primitive_offset: u32,
    pub(super) primitive_count: u32,
    name_presence: u8,
    name_count: u32,
}

impl GeometryLogicalMeshValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        primitive_offset: 0,
        primitive_count: 0,
        name_presence: PRESENCE_ABSENT,
        name_count: 0,
    };
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryPrimitiveValidationProjection {
    pub(super) vertex_count: u32,
}

impl GeometryPrimitiveValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self { vertex_count: 0 };
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryMeshInstanceValidationProjection {
    pub(super) logical_mesh: u32,
}

impl GeometryMeshInstanceValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self { logical_mesh: 0 };
}

pub(in crate::canonical_scene::store) struct GeometryValidationScratch<'scratch, 'session> {
    attributes: &'scratch mut [GeometryAttributeValidationScratch<'session>],
    primitives: &'scratch mut [GeometryPrimitiveValidationScratch<'session>],
    logical_meshes: &'scratch mut [GeometryLogicalMeshValidationProjection],
    primitive_projections: &'scratch mut [GeometryPrimitiveValidationProjection],
    mesh_instances: &'scratch mut [GeometryMeshInstanceValidationProjection],
}

type GeometryValidationScratchParts<'scratch, 'session> = (
    &'scratch mut [GeometryAttributeValidationScratch<'session>],
    &'scratch mut [GeometryPrimitiveValidationScratch<'session>],
    &'scratch mut [GeometryLogicalMeshValidationProjection],
    &'scratch mut [GeometryPrimitiveValidationProjection],
    &'scratch mut [GeometryMeshInstanceValidationProjection],
);

impl<'scratch, 'session> GeometryValidationScratch<'scratch, 'session> {
    pub(in crate::canonical_scene::store) const fn new(
        attributes: &'scratch mut [GeometryAttributeValidationScratch<'session>],
        primitives: &'scratch mut [GeometryPrimitiveValidationScratch<'session>],
        logical_meshes: &'scratch mut [GeometryLogicalMeshValidationProjection],
        primitive_projections: &'scratch mut [GeometryPrimitiveValidationProjection],
        mesh_instances: &'scratch mut [GeometryMeshInstanceValidationProjection],
    ) -> Self {
        Self {
            attributes,
            primitives,
            logical_meshes,
            primitive_projections,
            mesh_instances,
        }
    }

    fn parts(
        &mut self,
        requirements: GeometryValidationScratchRequirements,
    ) -> Option<GeometryValidationScratchParts<'_, 'session>> {
        Some((
            self.attributes.get_mut(..requirements.attributes)?,
            self.primitives.get_mut(..requirements.primitives)?,
            self.logical_meshes.get_mut(..requirements.logical_meshes)?,
            self.primitive_projections
                .get_mut(..requirements.primitives)?,
            self.mesh_instances.get_mut(..requirements.mesh_instances)?,
        ))
    }

    pub(in crate::canonical_scene::store) fn logical_mesh_projections(
        &self,
        requirements: GeometryValidationScratchRequirements,
    ) -> Option<&[GeometryLogicalMeshValidationProjection]> {
        self.logical_meshes.get(..requirements.logical_meshes)
    }

    pub(in crate::canonical_scene::store) fn primitive_projections(
        &self,
        requirements: GeometryValidationScratchRequirements,
    ) -> Option<&[GeometryPrimitiveValidationProjection]> {
        self.primitive_projections.get(..requirements.primitives)
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_projections(
        &self,
        requirements: GeometryValidationScratchRequirements,
    ) -> Option<&[GeometryMeshInstanceValidationProjection]> {
        self.mesh_instances.get(..requirements.mesh_instances)
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
    DenseI32 {
        values: &'a [i32],
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
    SparseI32 {
        indices: &'a [u32],
        values: &'a [i32],
        default: &'a [i32],
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
            Self::DenseI32 { values } => u32::from(!values.is_empty()),
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
            Self::SparseI32 {
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

    fn validate_attribute(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        semantic_name: Option<&str>,
    ) -> Option<()>;

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
        name: Option<&str>,
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

    fn validate_attribute(
        &mut self,
        _row: usize,
        _table: CanonicalRecordView<'_>,
        _semantic_name: Option<&str>,
    ) -> Option<()> {
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
        _name: Option<&str>,
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
                scene_geometry_directory_field::LOGICAL_MESH_NAME_BYTE_COUNT,
                self.expected.logical_mesh_name_byte_count(),
            ),
            (
                scene_geometry_directory_field::ATTRIBUTE_SEMANTIC_NAME_BYTE_COUNT,
                self.expected.attribute_semantic_name_byte_count(),
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

    fn validate_attribute(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        semantic_name: Option<&str>,
    ) -> Option<()> {
        validate_attribute_fresh(row, table, semantic_name, self.expected)
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
        name: Option<&str>,
    ) -> Option<()> {
        validate_logical_mesh_fresh(row, table, primitives, name, self.expected)
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
    reason = "typed geometry validation keeps the operation lifetimes and sealed dependencies explicit"
)]
pub(super) fn validate_geometry_canonical_structure_metered<
    'session,
    'storage,
    'control,
    'operation,
    'scratch,
    O,
    PP,
    U,
>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    geometry: CanonicalValidationMember<'session, RecordBlock, SceneGeometryDirectorySchema>,
    dependencies: GeometryCanonicalValidationDependencies<'session>,
    counts: GeometryCanonicalCounts,
    scratch: &mut GeometryValidationScratch<'scratch, 'session>,
) -> Result<Option<GeometryValidatedMemberShape<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(requirements) = GeometryValidationScratchRequirements::from_counts(counts) else {
        return Ok(None);
    };
    let Some((
        attributes_scratch,
        primitives_scratch,
        logical_mesh_projections,
        primitive_projections,
        mesh_instance_projections,
    )) = scratch.parts(requirements)
    else {
        return Ok(None);
    };

    let mut schema_revision = None;
    let mut logical_mesh_count = None;
    let mut logical_meshes = None;
    let mut logical_meshes_absent = None;
    let mut primitive_count = None;
    let mut primitives = None;
    let mut primitives_absent = None;
    let mut attribute_count = None;
    let mut attributes = None;
    let mut attributes_absent = None;
    let mut mesh_instance_count = None;
    let mut mesh_instances = None;
    let mut mesh_instances_absent = None;
    let mut logical_mesh_name_byte_count = None;
    let mut logical_mesh_names = None;
    let mut logical_mesh_names_absent = None;
    let mut attribute_semantic_name_byte_count = None;
    let mut attribute_semantic_names = None;
    let mut attribute_semantic_names_absent = None;
    let directory_complete = session.visit_canonical_records(
        geometry,
        1,
        SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            schema_revision = row.scalar(
                scene_geometry_directory_field::SCHEMA_REVISION,
                |value: u32| value,
            )?;
            logical_mesh_count = row.scalar(
                scene_geometry_directory_field::LOGICAL_MESH_COUNT,
                |value: u32| value,
            )?;
            if logical_mesh_count.is_some_and(|count| count != 0) {
                logical_meshes = row.record_ref::<GeometryLogicalMeshTableSchema, _>(
                    scene_geometry_directory_field::LOGICAL_MESHES,
                    |value| value,
                )?;
            } else {
                logical_meshes_absent = row.scalar(
                    scene_geometry_directory_field::LOGICAL_MESHES,
                    |value: u8| value,
                )?;
            }
            primitive_count = row.scalar(
                scene_geometry_directory_field::PRIMITIVE_COUNT,
                |value: u32| value,
            )?;
            if primitive_count.is_some_and(|count| count != 0) {
                primitives = row.record_ref::<GeometryPrimitiveTableSchema, _>(
                    scene_geometry_directory_field::PRIMITIVES,
                    |value| value,
                )?;
            } else {
                primitives_absent = row
                    .scalar(scene_geometry_directory_field::PRIMITIVES, |value: u8| {
                        value
                    })?;
            }
            attribute_count = row.scalar(
                scene_geometry_directory_field::ATTRIBUTE_COUNT,
                |value: u32| value,
            )?;
            if attribute_count.is_some_and(|count| count != 0) {
                attributes = row.record_ref::<GeometryAttributeTableSchema, _>(
                    scene_geometry_directory_field::ATTRIBUTES,
                    |value| value,
                )?;
            } else {
                attributes_absent = row
                    .scalar(scene_geometry_directory_field::ATTRIBUTES, |value: u8| {
                        value
                    })?;
            }
            mesh_instance_count = row.scalar(
                scene_geometry_directory_field::MESH_INSTANCE_COUNT,
                |value: u32| value,
            )?;
            if mesh_instance_count.is_some_and(|count| count != 0) {
                mesh_instances = row.record_ref::<GeometryMeshInstanceTableSchema, _>(
                    scene_geometry_directory_field::MESH_INSTANCES,
                    |value| value,
                )?;
            } else {
                mesh_instances_absent = row.scalar(
                    scene_geometry_directory_field::MESH_INSTANCES,
                    |value: u8| value,
                )?;
            }
            logical_mesh_name_byte_count = row.scalar(
                scene_geometry_directory_field::LOGICAL_MESH_NAME_BYTE_COUNT,
                |value: u32| value,
            )?;
            if logical_mesh_name_byte_count.is_some_and(|count| count != 0) {
                logical_mesh_names = row.utf8_ref::<GeometryLogicalMeshNameSchema, _>(
                    scene_geometry_directory_field::LOGICAL_MESH_NAMES,
                    |value| value,
                )?;
            } else {
                logical_mesh_names_absent = row.scalar(
                    scene_geometry_directory_field::LOGICAL_MESH_NAMES,
                    |value: u8| value,
                )?;
            }
            attribute_semantic_name_byte_count = row.scalar(
                scene_geometry_directory_field::ATTRIBUTE_SEMANTIC_NAME_BYTE_COUNT,
                |value: u32| value,
            )?;
            if attribute_semantic_name_byte_count.is_some_and(|count| count != 0) {
                attribute_semantic_names = row.utf8_ref::<GeometryAttributeSemanticNameSchema, _>(
                    scene_geometry_directory_field::ATTRIBUTE_SEMANTIC_NAMES,
                    |value| value,
                )?;
            } else {
                attribute_semantic_names_absent = row.scalar(
                    scene_geometry_directory_field::ATTRIBUTE_SEMANTIC_NAMES,
                    |value: u8| value,
                )?;
            }
            Ok(true)
        },
    )?;
    if !directory_complete
        || require_scalar(schema_revision, SCENE_GEOMETRY_SCHEMA_REVISION).is_none()
        || !metered_directory_collection_is_valid(
            logical_mesh_count,
            counts.logical_meshes,
            logical_meshes,
            logical_meshes_absent,
        )
        || !metered_directory_collection_is_valid(
            primitive_count,
            counts.primitives,
            primitives,
            primitives_absent,
        )
        || !metered_directory_collection_is_valid(
            attribute_count,
            counts.attributes,
            attributes,
            attributes_absent,
        )
        || !metered_directory_collection_is_valid(
            mesh_instance_count,
            counts.mesh_instances,
            mesh_instances,
            mesh_instances_absent,
        )
        || !metered_directory_payload_is_valid(
            logical_mesh_name_byte_count,
            logical_mesh_names,
            logical_mesh_names_absent,
        )
        || !metered_directory_payload_is_valid(
            attribute_semantic_name_byte_count,
            attribute_semantic_names,
            attribute_semantic_names_absent,
        )
    {
        return Ok(None);
    }
    let Some(logical_mesh_name_byte_count) = logical_mesh_name_byte_count else {
        return Ok(None);
    };
    let Some(attribute_semantic_name_byte_count) = attribute_semantic_name_byte_count else {
        return Ok(None);
    };

    let Some(attribute_payload_block_count) = validate_attribute_table_metered(
        session,
        attributes,
        attribute_semantic_names,
        attributes_scratch,
    )?
    else {
        return Ok(None);
    };
    if !visit_geometry_attribute_semantic_names(
        session,
        attribute_semantic_names,
        attribute_semantic_name_byte_count,
        attributes_scratch,
    )? {
        return Ok(None);
    }
    let Some(primitive_payload_block_count) = validate_primitive_table_metered(
        session,
        primitives,
        dependencies.materials,
        dependencies.material_count,
        attributes,
        attributes_scratch,
        primitives_scratch,
        primitive_projections,
    )?
    else {
        return Ok(None);
    };
    if !validate_logical_mesh_table_metered(
        session,
        logical_meshes,
        primitives,
        counts.primitives,
        logical_mesh_names,
        logical_mesh_name_byte_count,
        logical_mesh_projections,
    )? || !visit_geometry_logical_mesh_names(
        session,
        logical_mesh_names,
        logical_mesh_name_byte_count,
        logical_mesh_projections,
    )? || !validate_mesh_instance_table_metered(
        session,
        mesh_instances,
        dependencies.nodes,
        dependencies.canonical_node_count,
        logical_meshes,
        counts.logical_meshes,
        mesh_instance_projections,
    )? {
        return Ok(None);
    }
    let Some(payload_block_count) = attribute_payload_block_count
        .checked_add(primitive_payload_block_count)
        .and_then(|count| count.checked_add(u32::from(logical_mesh_names.is_some())))
        .and_then(|count| count.checked_add(u32::from(attribute_semantic_names.is_some())))
    else {
        return Ok(None);
    };
    Ok(Some(GeometryValidatedMemberShape {
        members: GeometryValidatedMembers {
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
            logical_mesh_names,
            attribute_semantic_names,
        },
        payload_block_count,
    }))
}

fn metered_directory_collection_is_valid<T>(
    actual_count: Option<u32>,
    expected_count: u32,
    member: Option<T>,
    absent: Option<u8>,
) -> bool {
    actual_count == Some(expected_count)
        && if expected_count == 0 {
            member.is_none() && absent == Some(0)
        } else {
            member.is_some()
        }
}

fn metered_directory_payload_is_valid<T>(
    count: Option<u32>,
    member: Option<T>,
    absent: Option<u8>,
) -> bool {
    match count {
        Some(0) => member.is_none() && absent == Some(0),
        Some(_) => member.is_some() && absent.is_none(),
        None => false,
    }
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
    let logical_mesh_names = optional_utf8_ref::<GeometryLogicalMeshNameSchema>(
        session,
        directory,
        scene_geometry_directory_field::LOGICAL_MESH_NAME_BYTE_COUNT,
        scene_geometry_directory_field::LOGICAL_MESH_NAMES,
        directory.scalar::<u32>(
            0,
            scene_geometry_directory_field::LOGICAL_MESH_NAME_BYTE_COUNT,
        )?,
    )?;
    let attribute_semantic_names = optional_utf8_ref::<GeometryAttributeSemanticNameSchema>(
        session,
        directory,
        scene_geometry_directory_field::ATTRIBUTE_SEMANTIC_NAME_BYTE_COUNT,
        scene_geometry_directory_field::ATTRIBUTE_SEMANTIC_NAMES,
        directory.scalar::<u32>(
            0,
            scene_geometry_directory_field::ATTRIBUTE_SEMANTIC_NAME_BYTE_COUNT,
        )?,
    )?;

    let (attribute_table, attribute_payload_block_count) = validate_attribute_table(
        session,
        attributes,
        attribute_semantic_names,
        equivalence,
        observe_work,
    )?;
    let primitive_payload_block_count = validate_primitive_table(
        session,
        primitives,
        dependencies.materials,
        dependencies.material_count,
        attributes,
        attribute_table,
        attribute_semantic_names,
        equivalence,
        observe_work,
    )?;
    validate_logical_mesh_table(
        session,
        logical_meshes,
        primitives,
        counts.primitives,
        logical_mesh_names,
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
            logical_mesh_names: logical_mesh_names.map(CanonicalBlockRef::from_store),
            attribute_semantic_names: attribute_semantic_names.map(CanonicalBlockRef::from_store),
            primitives: primitives.map(CanonicalBlockRef::from_store),
            attributes: attributes.map(CanonicalBlockRef::from_store),
            mesh_instances: mesh_instances.map(CanonicalBlockRef::from_store),
        },
        payload_block_count: attribute_payload_block_count
            .checked_add(primitive_payload_block_count)?
            .checked_add(u32::from(logical_mesh_names.is_some()))?
            .checked_add(u32::from(attribute_semantic_names.is_some()))?,
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

#[derive(Clone, Copy)]
enum GeometryAttributeLeafShape {
    DenseF32 {
        values: usize,
    },
    DenseU32 {
        values: usize,
    },
    DenseI32 {
        values: usize,
    },
    SparseF32 {
        values: usize,
        indices: usize,
        default: usize,
    },
    SparseU32 {
        values: usize,
        indices: usize,
        default: usize,
    },
    SparseI32 {
        values: usize,
        indices: usize,
        default: usize,
    },
}

fn geometry_attribute_leaf_shape(
    scalar_kind: u8,
    payload_kind: u8,
    lanes: u8,
    element_count: u32,
    explicit_count: u32,
    canonical_scalar_count: u64,
) -> Option<GeometryAttributeLeafShape> {
    let lane_count = usize::from(lanes);
    if lane_count == 0 {
        return None;
    }
    let element_count = usize::try_from(element_count).ok()?;
    let explicit_count_usize = usize::try_from(explicit_count).ok()?;
    let dense_scalar_count = element_count.checked_mul(lane_count)?;
    let explicit_scalar_count = explicit_count_usize.checked_mul(lane_count)?;
    match (payload_kind, scalar_kind) {
        (GEOMETRY_PAYLOAD_DENSE, GEOMETRY_SCALAR_F32) => (explicit_count_usize == element_count
            && canonical_scalar_count == u64::try_from(dense_scalar_count).ok()?)
        .then_some(GeometryAttributeLeafShape::DenseF32 {
            values: dense_scalar_count,
        }),
        (GEOMETRY_PAYLOAD_DENSE, GEOMETRY_SCALAR_U32) => (explicit_count_usize == element_count
            && canonical_scalar_count == u64::try_from(dense_scalar_count).ok()?)
        .then_some(GeometryAttributeLeafShape::DenseU32 {
            values: dense_scalar_count,
        }),
        (GEOMETRY_PAYLOAD_DENSE, GEOMETRY_SCALAR_I32) => (explicit_count_usize == element_count
            && canonical_scalar_count == u64::try_from(dense_scalar_count).ok()?)
        .then_some(GeometryAttributeLeafShape::DenseI32 {
            values: dense_scalar_count,
        }),
        (GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_SCALAR_F32) => (explicit_count_usize <= element_count
            && canonical_scalar_count
                == u64::try_from(explicit_scalar_count.checked_add(lane_count)?).ok()?)
        .then_some(GeometryAttributeLeafShape::SparseF32 {
            values: explicit_scalar_count,
            indices: explicit_count_usize,
            default: lane_count,
        }),
        (GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_SCALAR_U32) => (explicit_count_usize <= element_count
            && canonical_scalar_count
                == u64::try_from(explicit_scalar_count.checked_add(lane_count)?).ok()?)
        .then_some(GeometryAttributeLeafShape::SparseU32 {
            values: explicit_scalar_count,
            indices: explicit_count_usize,
            default: lane_count,
        }),
        (GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_SCALAR_I32) => (explicit_count_usize <= element_count
            && canonical_scalar_count
                == u64::try_from(explicit_scalar_count.checked_add(lane_count)?).ok()?)
        .then_some(GeometryAttributeLeafShape::SparseI32 {
            values: explicit_scalar_count,
            indices: explicit_count_usize,
            default: lane_count,
        }),
        _ => None,
    }
}

fn validate_attribute_table_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, RecordBlock, GeometryAttributeTableSchema>>,
    semantic_names: Option<
        CanonicalValidationMember<'session, Utf8Block, GeometryAttributeSemanticNameSchema>,
    >,
    scratch: &mut [GeometryAttributeValidationScratch<'session>],
) -> Result<Option<u32>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if scratch.is_empty() {
        return Ok(member.is_none().then_some(0));
    }
    let Some(member) = member else {
        return Ok(None);
    };
    let table_complete = session.visit_canonical_records(
        member,
        scratch.len(),
        GEOMETRY_ATTRIBUTE_FIELD_COUNT,
        |row_index, row| {
            let attribute = row.scalar(geometry_attribute_field::ATTRIBUTE, |value: u32| value)?;
            let semantic =
                row.scalar(geometry_attribute_field::SEMANTIC_KIND, |value: u8| value)?;
            let semantic_set =
                row.scalar(geometry_attribute_field::SEMANTIC_SET, |value: u32| value)?;
            let domain = row.scalar(geometry_attribute_field::DOMAIN, |value: u8| value)?;
            let scalar_kind =
                row.scalar(geometry_attribute_field::SCALAR_KIND, |value: u8| value)?;
            let lanes = row.scalar(geometry_attribute_field::LANE_COUNT, |value: u8| value)?;
            let normalization =
                row.scalar(geometry_attribute_field::NORMALIZATION, |value: u8| value)?;
            let payload_kind =
                row.scalar(geometry_attribute_field::PAYLOAD_KIND, |value: u8| value)?;
            let element_count =
                row.scalar(geometry_attribute_field::ELEMENT_COUNT, |value: u32| value)?;
            let explicit_count =
                row.scalar(geometry_attribute_field::EXPLICIT_COUNT, |value: u32| value)?;
            let canonical_scalar_count = row.scalar(
                geometry_attribute_field::CANONICAL_SCALAR_COUNT,
                |value: u64| value,
            )?;
            let semantic_name_offset = row.scalar(
                geometry_attribute_field::SEMANTIC_NAME_OFFSET,
                |value: u32| value,
            )?;
            let semantic_name_count = row.scalar(
                geometry_attribute_field::SEMANTIC_NAME_COUNT,
                |value: u32| value,
            )?;
            let semantic_name_cell_valid = if semantic_name_count.is_some_and(|count| count != 0) {
                row.utf8_ref::<GeometryAttributeSemanticNameSchema, _>(
                    geometry_attribute_field::SEMANTIC_NAMES,
                    |value| value,
                )? == semantic_names
                    && semantic_names.is_some()
            } else {
                row.scalar(geometry_attribute_field::SEMANTIC_NAMES, |value: u8| value)? == Some(0)
            };
            let leaf_shape = match (
                scalar_kind,
                payload_kind,
                lanes,
                element_count,
                explicit_count,
                canonical_scalar_count,
            ) {
                (
                    Some(scalar_kind),
                    Some(payload_kind),
                    Some(lanes),
                    Some(element_count),
                    Some(explicit_count),
                    Some(canonical_scalar_count),
                ) => geometry_attribute_leaf_shape(
                    scalar_kind,
                    payload_kind,
                    lanes,
                    element_count,
                    explicit_count,
                    canonical_scalar_count,
                ),
                _ => None,
            };

            let (values, values_cell_valid) = match leaf_shape {
                Some(GeometryAttributeLeafShape::DenseF32 { values })
                | Some(GeometryAttributeLeafShape::SparseF32 { values, .. })
                    if values != 0 =>
                {
                    let value = row.sequence_ref::<f32, GeometryF32ValuesSchema, _>(
                        geometry_attribute_field::VALUES,
                        |value| value,
                    )?;
                    let valid = value.is_some();
                    (
                        value.map_or(
                            GeometryAttributeValuesMember::Absent,
                            GeometryAttributeValuesMember::F32,
                        ),
                        valid,
                    )
                }
                Some(GeometryAttributeLeafShape::DenseU32 { values })
                | Some(GeometryAttributeLeafShape::SparseU32 { values, .. })
                    if values != 0 =>
                {
                    let value = row.sequence_ref::<u32, GeometryU32ValuesSchema, _>(
                        geometry_attribute_field::VALUES,
                        |value| value,
                    )?;
                    let valid = value.is_some();
                    (
                        value.map_or(
                            GeometryAttributeValuesMember::Absent,
                            GeometryAttributeValuesMember::U32,
                        ),
                        valid,
                    )
                }
                Some(GeometryAttributeLeafShape::DenseI32 { values })
                | Some(GeometryAttributeLeafShape::SparseI32 { values, .. })
                    if values != 0 =>
                {
                    let value = row.sequence_ref::<i32, GeometryI32ValuesSchema, _>(
                        geometry_attribute_field::VALUES,
                        |value| value,
                    )?;
                    let valid = value.is_some();
                    (
                        value.map_or(
                            GeometryAttributeValuesMember::Absent,
                            GeometryAttributeValuesMember::I32,
                        ),
                        valid,
                    )
                }
                _ => {
                    let absent = row.scalar(geometry_attribute_field::VALUES, |value: u8| value)?;
                    (GeometryAttributeValuesMember::Absent, absent == Some(0))
                }
            };

            let (sparse_indices, sparse_indices_cell_valid) = match leaf_shape {
                Some(GeometryAttributeLeafShape::SparseF32 { indices, .. })
                | Some(GeometryAttributeLeafShape::SparseU32 { indices, .. })
                | Some(GeometryAttributeLeafShape::SparseI32 { indices, .. })
                    if indices != 0 =>
                {
                    let value = row.sequence_ref::<u32, GeometrySparseIndicesSchema, _>(
                        geometry_attribute_field::SPARSE_INDICES,
                        |value| value,
                    )?;
                    let valid = value.is_some();
                    (value, valid)
                }
                _ => {
                    let absent =
                        row.scalar(geometry_attribute_field::SPARSE_INDICES, |value: u8| value)?;
                    (None, absent == Some(0))
                }
            };

            let (sparse_default, sparse_default_cell_valid) = match leaf_shape {
                Some(GeometryAttributeLeafShape::SparseF32 { default, .. }) if default != 0 => {
                    let value = row.sequence_ref::<f32, GeometrySparseF32DefaultsSchema, _>(
                        geometry_attribute_field::SPARSE_DEFAULT,
                        |value| value,
                    )?;
                    let valid = value.is_some();
                    (
                        value.map_or(
                            GeometryAttributeDefaultMember::Absent,
                            GeometryAttributeDefaultMember::F32,
                        ),
                        valid,
                    )
                }
                Some(GeometryAttributeLeafShape::SparseU32 { default, .. }) if default != 0 => {
                    let value = row.sequence_ref::<u32, GeometrySparseU32DefaultsSchema, _>(
                        geometry_attribute_field::SPARSE_DEFAULT,
                        |value| value,
                    )?;
                    let valid = value.is_some();
                    (
                        value.map_or(
                            GeometryAttributeDefaultMember::Absent,
                            GeometryAttributeDefaultMember::U32,
                        ),
                        valid,
                    )
                }
                Some(GeometryAttributeLeafShape::SparseI32 { default, .. }) if default != 0 => {
                    let value = row.sequence_ref::<i32, GeometrySparseI32DefaultsSchema, _>(
                        geometry_attribute_field::SPARSE_DEFAULT,
                        |value| value,
                    )?;
                    let valid = value.is_some();
                    (
                        value.map_or(
                            GeometryAttributeDefaultMember::Absent,
                            GeometryAttributeDefaultMember::I32,
                        ),
                        valid,
                    )
                }
                _ => {
                    let absent =
                        row.scalar(geometry_attribute_field::SPARSE_DEFAULT, |value: u8| value)?;
                    (GeometryAttributeDefaultMember::Absent, absent == Some(0))
                }
            };

            let Some(expected_attribute) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let (
                Some(semantic),
                Some(semantic_set),
                Some(domain),
                Some(scalar_kind),
                Some(lanes),
                Some(payload_kind),
                Some(element_count),
                Some(explicit_count),
                Some(canonical_scalar_count),
                Some(semantic_name_offset),
                Some(semantic_name_count),
            ) = (
                semantic,
                semantic_set,
                domain,
                scalar_kind,
                lanes,
                payload_kind,
                element_count,
                explicit_count,
                canonical_scalar_count,
                semantic_name_offset,
                semantic_name_count,
            )
            else {
                return Ok(false);
            };
            let valid = require_scalar(attribute, expected_attribute).is_some()
                && require_scalar(normalization, GEOMETRY_NORMALIZATION_NONE).is_some()
                && validate_attribute_schema(semantic, semantic_set, domain, scalar_kind, lanes)
                    .is_some()
                && leaf_shape.is_some()
                && values_cell_valid
                && sparse_indices_cell_valid
                && sparse_default_cell_valid
                && semantic_name_cell_valid
                && ((semantic == GEOMETRY_SEMANTIC_CUSTOM && semantic_name_count != 0)
                    || (semantic != GEOMETRY_SEMANTIC_CUSTOM && semantic_name_count == 0));
            if valid {
                let Some(destination) = scratch.get_mut(row_index) else {
                    return Ok(false);
                };
                *destination = GeometryAttributeValidationScratch {
                    semantic,
                    semantic_set,
                    semantic_name_offset,
                    semantic_name_count,
                    semantic_name: None,
                    domain,
                    scalar_kind,
                    lanes,
                    payload_kind,
                    element_count,
                    explicit_count,
                    canonical_scalar_count,
                    values,
                    sparse_indices,
                    sparse_default,
                    sparse_maximum: None,
                };
            }
            Ok(valid)
        },
    )?;
    if !table_complete {
        return Ok(None);
    }

    let mut payload_block_count = 0_u32;
    for row_index in 0..scratch.len() {
        let Some(block_count) = validate_attribute_payload_metered(session, scratch, row_index)?
        else {
            return Ok(None);
        };
        let Some(next) = payload_block_count.checked_add(block_count) else {
            return Ok(None);
        };
        payload_block_count = next;
    }
    Ok(Some(payload_block_count))
}

fn validate_attribute_payload_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    scratch: &mut [GeometryAttributeValidationScratch<'session>],
    row_index: usize,
) -> Result<Option<u32>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(current) = scratch.get(row_index).copied() else {
        return Ok(None);
    };
    let Some(leaf_shape) = geometry_attribute_leaf_shape(
        current.scalar_kind,
        current.payload_kind,
        current.lanes,
        current.element_count,
        current.explicit_count,
        current.canonical_scalar_count,
    ) else {
        return Ok(None);
    };
    let Some(cached_sparse_maximum) = session.run_validation_unit(|| {
        current.sparse_indices.and_then(|member| {
            scratch[..row_index]
                .iter()
                .find(|candidate| candidate.sparse_indices == Some(member))
                .map(|candidate| candidate.sparse_maximum)
        })
    })?
    else {
        return Ok(None);
    };

    let block_count = match leaf_shape {
        GeometryAttributeLeafShape::DenseF32 { values } => {
            if !validate_f32_values_metered(session, current.values, values)?
                || !matches!(
                    current.sparse_default,
                    GeometryAttributeDefaultMember::Absent
                )
                || current.sparse_indices.is_some()
            {
                return Ok(None);
            }
            u32::from(values != 0)
        }
        GeometryAttributeLeafShape::DenseU32 { values } => {
            if !validate_u32_values_metered(session, current.values, values)?
                || !matches!(
                    current.sparse_default,
                    GeometryAttributeDefaultMember::Absent
                )
                || current.sparse_indices.is_some()
            {
                return Ok(None);
            }
            u32::from(values != 0)
        }
        GeometryAttributeLeafShape::DenseI32 { values } => {
            if !validate_i32_values_metered(session, current.values, values)?
                || !matches!(
                    current.sparse_default,
                    GeometryAttributeDefaultMember::Absent
                )
                || current.sparse_indices.is_some()
            {
                return Ok(None);
            }
            u32::from(values != 0)
        }
        GeometryAttributeLeafShape::SparseF32 {
            values,
            indices,
            default,
        } => {
            if !validate_f32_values_metered(session, current.values, values)?
                || !validate_f32_default_metered(session, current.sparse_default, default)?
            {
                return Ok(None);
            }
            let Some(maximum) = validate_sparse_indices_metered(
                session,
                current.sparse_indices,
                indices,
                current.element_count,
                cached_sparse_maximum,
            )?
            else {
                return Ok(None);
            };
            if let Some(destination) = scratch.get_mut(row_index) {
                destination.sparse_maximum = maximum;
            } else {
                return Ok(None);
            }
            u32::from(values != 0) + u32::from(indices != 0) + u32::from(default != 0)
        }
        GeometryAttributeLeafShape::SparseU32 {
            values,
            indices,
            default,
        } => {
            if !validate_u32_values_metered(session, current.values, values)?
                || !validate_u32_default_metered(session, current.sparse_default, default)?
            {
                return Ok(None);
            }
            let Some(maximum) = validate_sparse_indices_metered(
                session,
                current.sparse_indices,
                indices,
                current.element_count,
                cached_sparse_maximum,
            )?
            else {
                return Ok(None);
            };
            if let Some(destination) = scratch.get_mut(row_index) {
                destination.sparse_maximum = maximum;
            } else {
                return Ok(None);
            }
            u32::from(values != 0) + u32::from(indices != 0) + u32::from(default != 0)
        }
        GeometryAttributeLeafShape::SparseI32 {
            values,
            indices,
            default,
        } => {
            if !validate_i32_values_metered(session, current.values, values)?
                || !validate_i32_default_metered(session, current.sparse_default, default)?
            {
                return Ok(None);
            }
            let Some(maximum) = validate_sparse_indices_metered(
                session,
                current.sparse_indices,
                indices,
                current.element_count,
                cached_sparse_maximum,
            )?
            else {
                return Ok(None);
            };
            if let Some(destination) = scratch.get_mut(row_index) {
                destination.sparse_maximum = maximum;
            } else {
                return Ok(None);
            }
            u32::from(values != 0) + u32::from(indices != 0) + u32::from(default != 0)
        }
    };
    Ok(Some(block_count))
}

fn validate_f32_values_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: GeometryAttributeValuesMember<'session>,
    expected_len: usize,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match (member, expected_len) {
        (GeometryAttributeValuesMember::Absent, 0) => Ok(true),
        (GeometryAttributeValuesMember::F32(member), expected_len) if expected_len != 0 => session
            .visit_canonical_sequence(member, expected_len, |_index, value| value.is_finite()),
        _ => Ok(false),
    }
}

fn validate_u32_values_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: GeometryAttributeValuesMember<'session>,
    expected_len: usize,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match (member, expected_len) {
        (GeometryAttributeValuesMember::Absent, 0) => Ok(true),
        (GeometryAttributeValuesMember::U32(member), expected_len) if expected_len != 0 => {
            session.visit_canonical_sequence(member, expected_len, |_index, _value| true)
        }
        _ => Ok(false),
    }
}

fn validate_i32_values_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: GeometryAttributeValuesMember<'session>,
    expected_len: usize,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match (member, expected_len) {
        (GeometryAttributeValuesMember::Absent, 0) => Ok(true),
        (GeometryAttributeValuesMember::I32(member), expected_len) if expected_len != 0 => {
            session.visit_canonical_sequence(member, expected_len, |_index, _value| true)
        }
        _ => Ok(false),
    }
}

fn validate_f32_default_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: GeometryAttributeDefaultMember<'session>,
    expected_len: usize,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match (member, expected_len) {
        (GeometryAttributeDefaultMember::Absent, 0) => Ok(true),
        (GeometryAttributeDefaultMember::F32(member), expected_len) if expected_len != 0 => session
            .visit_canonical_sequence(member, expected_len, |_index, value| value.is_finite()),
        _ => Ok(false),
    }
}

fn validate_u32_default_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: GeometryAttributeDefaultMember<'session>,
    expected_len: usize,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match (member, expected_len) {
        (GeometryAttributeDefaultMember::Absent, 0) => Ok(true),
        (GeometryAttributeDefaultMember::U32(member), expected_len) if expected_len != 0 => {
            session.visit_canonical_sequence(member, expected_len, |_index, _value| true)
        }
        _ => Ok(false),
    }
}

fn validate_i32_default_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: GeometryAttributeDefaultMember<'session>,
    expected_len: usize,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match (member, expected_len) {
        (GeometryAttributeDefaultMember::Absent, 0) => Ok(true),
        (GeometryAttributeDefaultMember::I32(member), expected_len) if expected_len != 0 => {
            session.visit_canonical_sequence(member, expected_len, |_index, _value| true)
        }
        _ => Ok(false),
    }
}

fn validate_sparse_indices_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, GeometrySparseIndicesSchema>,
    >,
    expected_len: usize,
    element_count: u32,
    cached_maximum: Option<Option<u32>>,
) -> Result<Option<Option<u32>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if expected_len == 0 {
        return Ok((member.is_none() && cached_maximum.is_none()).then_some(None));
    }
    let Some(member) = member else {
        return Ok(None);
    };
    if let Some(cached_maximum) = cached_maximum {
        let Some(cached_maximum) = cached_maximum else {
            return Ok(None);
        };
        let complete =
            session.visit_canonical_sequence(member, expected_len, |_index, _value| true)?;
        return Ok((complete && cached_maximum < element_count).then_some(Some(cached_maximum)));
    }
    let mut previous = None;
    let mut maximum = None;
    let complete = session.visit_canonical_sequence(member, expected_len, |_index, value| {
        let accepted = value < element_count && previous.is_none_or(|previous| value > previous);
        previous = Some(value);
        maximum = Some(value);
        accepted
    })?;
    Ok((complete && maximum.is_some()).then_some(maximum))
}

fn visit_geometry_attribute_semantic_names<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    names: Option<
        CanonicalValidationMember<'session, Utf8Block, GeometryAttributeSemanticNameSchema>,
    >,
    name_byte_count: u32,
    attributes: &mut [GeometryAttributeValidationScratch<'session>],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(names) = names else {
        return Ok(name_byte_count == 0
            && attributes.iter().all(|attribute| {
                attribute.semantic != GEOMETRY_SEMANTIC_CUSTOM
                    && attribute.semantic_name_offset == 0
                    && attribute.semantic_name_count == 0
                    && attribute.semantic_name.is_none()
            }));
    };
    let Some(name_byte_count) = usize::try_from(name_byte_count).ok() else {
        return Ok(false);
    };
    let mut attribute_index = 0_usize;
    let mut name_cursor = 0_u32;
    let mut segment_len = 0_usize;
    let mut segment = [0_u8; MAX_NAMESPACED_NAME_LEN];
    let complete = session.visit_canonical_utf8_bytes(names, name_byte_count, |_index, byte| {
        while let Some(attribute) = attributes.get(attribute_index) {
            if attribute.semantic_name_count != 0 {
                break;
            }
            if attribute.semantic == GEOMETRY_SEMANTIC_CUSTOM
                || attribute.semantic_name_offset != name_cursor
                || attribute.semantic_name.is_some()
            {
                return false;
            }
            attribute_index += 1;
        }
        let Some(attribute) = attributes.get(attribute_index) else {
            return false;
        };
        let Some(segment_count) = usize::try_from(attribute.semantic_name_count).ok() else {
            return false;
        };
        if attribute.semantic != GEOMETRY_SEMANTIC_CUSTOM
            || attribute.semantic_name_offset != name_cursor
            || segment_count == 0
            || segment_count > segment.len()
            || segment_len >= segment_count
        {
            return false;
        }
        segment[segment_len] = byte;
        segment_len += 1;
        if segment_len != segment_count {
            return true;
        }
        let Some(name) = std::str::from_utf8(&segment[..segment_len])
            .ok()
            .and_then(GeometryAttributeSemanticName::try_new)
        else {
            return false;
        };
        let Some(destination) = attributes.get_mut(attribute_index) else {
            return false;
        };
        destination.semantic_name = Some(name);
        let Some(next_name_cursor) = name_cursor.checked_add(destination.semantic_name_count)
        else {
            return false;
        };
        name_cursor = next_name_cursor;
        attribute_index += 1;
        segment_len = 0;
        true
    })?;
    while let Some(attribute) = attributes.get(attribute_index) {
        if attribute.semantic == GEOMETRY_SEMANTIC_CUSTOM
            || attribute.semantic_name_offset != name_cursor
            || attribute.semantic_name_count != 0
            || attribute.semantic_name.is_some()
        {
            return Ok(false);
        }
        attribute_index += 1;
    }
    Ok(complete
        && segment_len == 0
        && attribute_index == attributes.len()
        && usize::try_from(name_cursor).ok() == Some(name_byte_count))
}

#[allow(
    clippy::too_many_arguments,
    reason = "typed primitive validation keeps every dependency, projection, and bounded scratch slice explicit"
)]
fn validate_primitive_table_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, RecordBlock, GeometryPrimitiveTableSchema>>,
    materials: Option<
        CanonicalValidationMember<'session, RecordBlock, AppearanceMaterialTableSchema>,
    >,
    material_count: u32,
    attributes: Option<
        CanonicalValidationMember<'session, RecordBlock, GeometryAttributeTableSchema>,
    >,
    attribute_scratch: &[GeometryAttributeValidationScratch<'session>],
    primitive_scratch: &mut [GeometryPrimitiveValidationScratch<'session>],
    primitive_projections: &mut [GeometryPrimitiveValidationProjection],
) -> Result<Option<u32>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if primitive_scratch.is_empty() {
        return Ok((member.is_none() && attributes.is_none()).then_some(0));
    }
    let Some(member) = member else {
        return Ok(None);
    };
    if primitive_scratch.len() != primitive_projections.len() {
        return Ok(None);
    }
    let table_complete = session.visit_canonical_records(
        member,
        primitive_scratch.len(),
        GEOMETRY_PRIMITIVE_FIELD_COUNT,
        |row_index, row| {
            let primitive = row.scalar(geometry_primitive_field::PRIMITIVE, |value: u32| value)?;
            let topology = row.scalar(geometry_primitive_field::TOPOLOGY, |value: u8| value)?;
            let vertex_count =
                row.scalar(geometry_primitive_field::VERTEX_COUNT, |value: u32| value)?;
            let corner_count =
                row.scalar(geometry_primitive_field::CORNER_COUNT, |value: u32| value)?;
            let face_count =
                row.scalar(geometry_primitive_field::FACE_COUNT, |value: u32| value)?;
            let polygon_presence = row.scalar(
                geometry_primitive_field::POLYGON_BOUNDARIES_PRESENCE,
                |value: u8| value,
            )?;
            let polygon_count = row.scalar(
                geometry_primitive_field::POLYGON_BOUNDARIES_COUNT,
                |value: u32| value,
            )?;
            let (polygon_boundaries, polygon_cell_valid) =
                if polygon_presence == Some(PRESENCE_PRESENT) && polygon_count != Some(0) {
                    let value = row.sequence_ref::<u32, GeometryPolygonBoundariesSchema, _>(
                        geometry_primitive_field::POLYGON_BOUNDARIES,
                        |value| value,
                    )?;
                    let valid = value.is_some();
                    (value, valid)
                } else {
                    let absent = row
                        .scalar(geometry_primitive_field::POLYGON_BOUNDARIES, |value: u8| {
                            value
                        })?;
                    (None, absent == Some(0))
                };
            let index_presence =
                row.scalar(geometry_primitive_field::INDEX_PRESENCE, |value: u8| value)?;
            let index_count =
                row.scalar(geometry_primitive_field::INDEX_COUNT, |value: u32| value)?;
            let (indices, indices_cell_valid) = if index_presence == Some(PRESENCE_PRESENT)
                && index_count != Some(0)
            {
                let value = row.sequence_ref::<u32, GeometryPrimitiveIndicesSchema, _>(
                    geometry_primitive_field::INDICES,
                    |value| value,
                )?;
                let valid = value.is_some();
                (value, valid)
            } else {
                let absent = row.scalar(geometry_primitive_field::INDICES, |value: u8| value)?;
                (None, absent == Some(0))
            };
            let material_presence = row
                .scalar(geometry_primitive_field::MATERIAL_PRESENCE, |value: u8| {
                    value
                })?;
            let material = row.scalar(geometry_primitive_field::MATERIAL, |value: u32| value)?;
            let (material_member, materials_cell_valid) = if material_presence
                == Some(PRESENCE_PRESENT)
            {
                let value = row.record_ref::<AppearanceMaterialTableSchema, _>(
                    geometry_primitive_field::MATERIALS,
                    |value| value,
                )?;
                let valid = value.is_some();
                (value, valid)
            } else {
                let absent = row.scalar(geometry_primitive_field::MATERIALS, |value: u8| value)?;
                (None, absent == Some(0))
            };
            let attribute_offset = row
                .scalar(geometry_primitive_field::ATTRIBUTE_OFFSET, |value: u32| {
                    value
                })?;
            let attribute_count = row
                .scalar(geometry_primitive_field::ATTRIBUTE_COUNT, |value: u32| {
                    value
                })?;
            let (attribute_member, attributes_cell_valid) = if attribute_count != Some(0) {
                let value = row.record_ref::<GeometryAttributeTableSchema, _>(
                    geometry_primitive_field::ATTRIBUTES,
                    |value| value,
                )?;
                let valid = value.is_some();
                (value, valid)
            } else {
                let absent = row.scalar(geometry_primitive_field::ATTRIBUTES, |value: u8| value)?;
                (None, absent == Some(0))
            };

            let Some(expected_primitive) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let (
                Some(topology),
                Some(vertex_count),
                Some(corner_count),
                Some(face_count),
                Some(polygon_presence),
                Some(polygon_count),
                Some(index_presence),
                Some(index_count),
                Some(material_presence),
                Some(material),
                Some(attribute_offset),
                Some(attribute_count),
            ) = (
                topology,
                vertex_count,
                corner_count,
                face_count,
                polygon_presence,
                polygon_count,
                index_presence,
                index_count,
                material_presence,
                material,
                attribute_offset,
                attribute_count,
            )
            else {
                return Ok(false);
            };
            let material = match material_presence {
                PRESENCE_ABSENT if material == 0 && materials_cell_valid => None,
                PRESENCE_PRESENT if materials_cell_valid => Some(material),
                _ => return Ok(false),
            };
            let valid = require_scalar(primitive, expected_primitive).is_some()
                && matches!(polygon_presence, PRESENCE_ABSENT | PRESENCE_PRESENT)
                && matches!(index_presence, PRESENCE_ABSENT | PRESENCE_PRESENT)
                && polygon_cell_valid
                && indices_cell_valid
                && attributes_cell_valid;
            if valid {
                let Some(destination) = primitive_scratch.get_mut(row_index) else {
                    return Ok(false);
                };
                *destination = GeometryPrimitiveValidationScratch {
                    topology,
                    vertex_count,
                    corner_count,
                    face_count,
                    polygon_present: polygon_presence == PRESENCE_PRESENT,
                    polygon_count,
                    polygon_boundaries,
                    polygon_total: None,
                    indices_present: index_presence == PRESENCE_PRESENT,
                    index_count,
                    indices,
                    index_maximum: None,
                    material,
                    materials: material_member,
                    attribute_offset,
                    attribute_count,
                    attributes: attribute_member,
                };
            }
            Ok(valid)
        },
    )?;
    if !table_complete {
        return Ok(None);
    }

    let mut attribute_cursor = 0_usize;
    let mut payload_block_count = 0_u32;
    for row_index in 0..primitive_scratch.len() {
        let current = primitive_scratch[row_index];
        let Some(cached_polygon_total) = session.run_validation_unit(|| {
            current.polygon_boundaries.and_then(|member| {
                primitive_scratch[..row_index]
                    .iter()
                    .find(|candidate| candidate.polygon_boundaries == Some(member))
                    .map(|candidate| candidate.polygon_total)
            })
        })?
        else {
            return Ok(None);
        };
        let Some(cached_index_maximum) = session.run_validation_unit(|| {
            current.indices.and_then(|member| {
                primitive_scratch[..row_index]
                    .iter()
                    .find(|candidate| candidate.indices == Some(member))
                    .map(|candidate| candidate.index_maximum)
            })
        })?
        else {
            return Ok(None);
        };
        let Some(polygon_total) =
            validate_polygon_boundaries_metered(session, current, cached_polygon_total)?
        else {
            return Ok(None);
        };
        let Some(index_maximum) =
            validate_primitive_indices_metered(session, current, cached_index_maximum)?
        else {
            return Ok(None);
        };
        if !validate_topology_shape_metered(current, polygon_total)
            || !validate_primitive_material_metered(current, materials, material_count)
        {
            return Ok(None);
        }
        let Some(attribute_range) = checked_range(
            current.attribute_offset,
            current.attribute_count,
            attribute_scratch.len(),
        ) else {
            return Ok(None);
        };
        if attribute_range.start != attribute_cursor
            || (current.attribute_count == 0
                && (current.attributes.is_some() || current.vertex_count != 0))
            || (current.attribute_count != 0 && current.attributes != attributes)
            || !validate_primitive_attribute_range_metered(
                session,
                attribute_scratch,
                attribute_range.clone(),
                current.vertex_count,
                current.corner_count,
                current.face_count,
            )?
        {
            return Ok(None);
        }
        attribute_cursor = attribute_range.end;
        primitive_scratch[row_index].polygon_total = polygon_total;
        primitive_scratch[row_index].index_maximum = index_maximum;
        primitive_projections[row_index] = GeometryPrimitiveValidationProjection {
            vertex_count: current.vertex_count,
        };
        let Some(next) = payload_block_count
            .checked_add(u32::from(current.polygon_boundaries.is_some()))
            .and_then(|count| count.checked_add(u32::from(current.indices.is_some())))
        else {
            return Ok(None);
        };
        payload_block_count = next;
    }
    Ok((attribute_cursor == attribute_scratch.len()).then_some(payload_block_count))
}

fn validate_polygon_boundaries_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    primitive: GeometryPrimitiveValidationScratch<'session>,
    cached_total: Option<Option<u32>>,
) -> Result<Option<Option<u32>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if !primitive.polygon_present {
        return Ok(
            (primitive.polygon_count == 0 && primitive.polygon_boundaries.is_none())
                .then_some(None),
        );
    }
    if primitive.topology != GEOMETRY_TOPOLOGY_POLYGONS
        || primitive.polygon_count != primitive.face_count
    {
        return Ok(None);
    }
    if primitive.polygon_count == 0 {
        return Ok((primitive.polygon_boundaries.is_none()).then_some(Some(0)));
    }
    let Some(member) = primitive.polygon_boundaries else {
        return Ok(None);
    };
    let Some(expected_len) = usize::try_from(primitive.polygon_count).ok() else {
        return Ok(None);
    };
    if let Some(cached_total) = cached_total {
        let Some(cached_total) = cached_total else {
            return Ok(None);
        };
        let complete =
            session.visit_canonical_sequence(member, expected_len, |_index, _value| true)?;
        return Ok(complete.then_some(Some(cached_total)));
    }
    let mut total = Some(0_u32);
    let complete = session.visit_canonical_sequence(member, expected_len, |_index, boundary| {
        if boundary < 3 {
            return false;
        }
        total = total.and_then(|total| total.checked_add(boundary));
        total.is_some()
    })?;
    Ok((complete && total.is_some()).then_some(total))
}

fn validate_primitive_indices_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    primitive: GeometryPrimitiveValidationScratch<'session>,
    cached_maximum: Option<Option<u32>>,
) -> Result<Option<Option<u32>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if !primitive.indices_present {
        return Ok((primitive.index_count == 0
            && primitive.indices.is_none()
            && primitive.corner_count == primitive.vertex_count)
            .then_some(None));
    }
    if primitive.index_count != primitive.corner_count {
        return Ok(None);
    }
    if primitive.index_count == 0 {
        return Ok((primitive.indices.is_none()).then_some(Some(0)));
    }
    let Some(member) = primitive.indices else {
        return Ok(None);
    };
    let Some(expected_len) = usize::try_from(primitive.index_count).ok() else {
        return Ok(None);
    };
    if let Some(cached_maximum) = cached_maximum {
        let Some(cached_maximum) = cached_maximum else {
            return Ok(None);
        };
        let complete =
            session.visit_canonical_sequence(member, expected_len, |_index, _value| true)?;
        return Ok(
            (complete && cached_maximum < primitive.vertex_count).then_some(Some(cached_maximum))
        );
    }
    let mut maximum = None;
    let complete = session.visit_canonical_sequence(member, expected_len, |_position, index| {
        maximum = Some(maximum.map_or(index, |maximum: u32| maximum.max(index)));
        index < primitive.vertex_count
    })?;
    Ok((complete && maximum.is_some()).then_some(maximum))
}

fn validate_topology_shape_metered(
    primitive: GeometryPrimitiveValidationScratch<'_>,
    polygon_total: Option<u32>,
) -> bool {
    let expected_face_count = match primitive.topology {
        GEOMETRY_TOPOLOGY_POINTS => primitive.corner_count,
        GEOMETRY_TOPOLOGY_LINE_LIST if primitive.corner_count.is_multiple_of(2) => {
            primitive.corner_count / 2
        }
        GEOMETRY_TOPOLOGY_LINE_STRIP => primitive.corner_count.saturating_sub(1),
        GEOMETRY_TOPOLOGY_LINE_LOOP => {
            if primitive.corner_count < 2 {
                0
            } else {
                primitive.corner_count
            }
        }
        GEOMETRY_TOPOLOGY_TRIANGLE_LIST if primitive.corner_count.is_multiple_of(3) => {
            primitive.corner_count / 3
        }
        GEOMETRY_TOPOLOGY_TRIANGLE_STRIP | GEOMETRY_TOPOLOGY_TRIANGLE_FAN => {
            primitive.corner_count.saturating_sub(2)
        }
        GEOMETRY_TOPOLOGY_POLYGONS if polygon_total == Some(primitive.corner_count) => {
            primitive.polygon_count
        }
        _ => return false,
    };
    primitive.face_count == expected_face_count
        && (primitive.topology == GEOMETRY_TOPOLOGY_POLYGONS) == primitive.polygon_present
}

fn validate_primitive_material_metered(
    primitive: GeometryPrimitiveValidationScratch<'_>,
    materials: Option<CanonicalValidationMember<'_, RecordBlock, AppearanceMaterialTableSchema>>,
    material_count: u32,
) -> bool {
    match primitive.material {
        None => primitive.materials.is_none(),
        Some(material) => material < material_count && primitive.materials == materials,
    }
}

fn validate_primitive_attribute_range_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    attributes: &[GeometryAttributeValidationScratch<'session>],
    range: std::ops::Range<usize>,
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut previous_attribute = None;
    let mut has_position = false;
    for row_index in range {
        let Some(valid) = session.run_validation_unit(|| {
            let Some(attribute) = attributes.get(row_index) else {
                return false;
            };
            if let Some(previous) = previous_attribute {
                let Some(ordering) = compare_geometry_attribute_keys(previous, attribute) else {
                    return false;
                };
                if ordering != std::cmp::Ordering::Less {
                    return false;
                }
            }
            previous_attribute = Some(attribute);
            let expected_element_count = match attribute.domain {
                GEOMETRY_DOMAIN_VERTEX => vertex_count,
                GEOMETRY_DOMAIN_CORNER => corner_count,
                GEOMETRY_DOMAIN_FACE => face_count,
                _ => return false,
            };
            has_position |= attribute.semantic == GEOMETRY_SEMANTIC_POSITION;
            attribute.element_count == expected_element_count
        })?
        else {
            return Ok(false);
        };
        if !valid {
            return Ok(false);
        }
    }
    Ok(vertex_count == 0 || has_position)
}

fn compare_geometry_attribute_keys(
    left: &GeometryAttributeValidationScratch<'_>,
    right: &GeometryAttributeValidationScratch<'_>,
) -> Option<std::cmp::Ordering> {
    let semantic = left.semantic.cmp(&right.semantic);
    if semantic != std::cmp::Ordering::Equal {
        return Some(semantic);
    }
    let semantic_identity = if left.semantic == GEOMETRY_SEMANTIC_CUSTOM {
        left.semantic_name
            .as_ref()?
            .cmp(right.semantic_name.as_ref()?)
    } else {
        left.semantic_set.cmp(&right.semantic_set)
    };
    if semantic_identity != std::cmp::Ordering::Equal {
        return Some(semantic_identity);
    }
    Some(left.domain.cmp(&right.domain))
}

fn validate_logical_mesh_table_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<
        CanonicalValidationMember<'session, RecordBlock, GeometryLogicalMeshTableSchema>,
    >,
    primitives: Option<
        CanonicalValidationMember<'session, RecordBlock, GeometryPrimitiveTableSchema>,
    >,
    primitive_count: u32,
    logical_mesh_names: Option<
        CanonicalValidationMember<'session, Utf8Block, GeometryLogicalMeshNameSchema>,
    >,
    logical_mesh_name_byte_count: u32,
    projections: &mut [GeometryLogicalMeshValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if projections.is_empty() {
        return Ok(member.is_none()
            && primitive_count == 0
            && logical_mesh_names.is_none()
            && logical_mesh_name_byte_count == 0);
    }
    let (Some(member), Some(primitives)) = (member, primitives) else {
        return Ok(false);
    };
    let mut primitive_cursor = 0_u32;
    let mut name_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        member,
        projections.len(),
        GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
        |row_index, row| {
            let logical_mesh = row
                .scalar(geometry_logical_mesh_field::LOGICAL_MESH, |value: u32| {
                    value
                })?;
            let primitive_offset = row.scalar(
                geometry_logical_mesh_field::PRIMITIVE_OFFSET,
                |value: u32| value,
            )?;
            let row_primitive_count = row.scalar(
                geometry_logical_mesh_field::PRIMITIVE_COUNT,
                |value: u32| value,
            )?;
            let primitive_member = row.record_ref::<GeometryPrimitiveTableSchema, _>(
                geometry_logical_mesh_field::PRIMITIVES,
                |value| value,
            )?;
            let name_presence = row
                .scalar(geometry_logical_mesh_field::NAME_PRESENCE, |value: u8| {
                    value
                })?;
            let name_offset =
                row.scalar(geometry_logical_mesh_field::NAME_OFFSET, |value: u32| value)?;
            let name_count =
                row.scalar(geometry_logical_mesh_field::NAME_COUNT, |value: u32| value)?;
            let mut names_member = None;
            let mut names_absent = None;
            if name_count == Some(0) {
                names_absent = row.scalar(geometry_logical_mesh_field::NAMES, |value: u8| value)?;
            } else {
                names_member = row.utf8_ref::<GeometryLogicalMeshNameSchema, _>(
                    geometry_logical_mesh_field::NAMES,
                    |value| value,
                )?;
            }
            let Some(expected_logical_mesh) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let (Some(primitive_offset), Some(row_primitive_count)) =
                (primitive_offset, row_primitive_count)
            else {
                return Ok(false);
            };
            let Some(next_cursor) = primitive_offset.checked_add(row_primitive_count) else {
                return Ok(false);
            };
            let (Some(name_offset), Some(name_count)) = (name_offset, name_count) else {
                return Ok(false);
            };
            let Some(next_name_cursor) = name_offset.checked_add(name_count) else {
                return Ok(false);
            };
            let name_valid = match (name_presence, name_count) {
                (Some(PRESENCE_ABSENT), 0) | (Some(PRESENCE_PRESENT), 0) => {
                    names_member.is_none() && names_absent == Some(0)
                }
                (Some(PRESENCE_PRESENT), _) => {
                    names_member == logical_mesh_names
                        && names_member.is_some()
                        && names_absent.is_none()
                }
                _ => false,
            };
            let valid = require_scalar(logical_mesh, expected_logical_mesh).is_some()
                && primitive_offset == primitive_cursor
                && row_primitive_count != 0
                && next_cursor <= primitive_count
                && primitive_member == Some(primitives)
                && name_offset == name_cursor
                && next_name_cursor <= logical_mesh_name_byte_count
                && name_valid;
            if valid {
                let Some(destination) = projections.get_mut(row_index) else {
                    return Ok(false);
                };
                *destination = GeometryLogicalMeshValidationProjection {
                    primitive_offset,
                    primitive_count: row_primitive_count,
                    name_presence: name_presence.unwrap_or(PRESENCE_ABSENT),
                    name_count,
                };
                primitive_cursor = next_cursor;
                name_cursor = next_name_cursor;
            }
            Ok(valid)
        },
    )?;
    Ok(complete
        && primitive_cursor == primitive_count
        && name_cursor == logical_mesh_name_byte_count)
}

fn visit_geometry_logical_mesh_names<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    names: Option<CanonicalValidationMember<'session, Utf8Block, GeometryLogicalMeshNameSchema>>,
    name_byte_count: u32,
    logical_meshes: &[GeometryLogicalMeshValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(names) = names else {
        return Ok(name_byte_count == 0
            && logical_meshes.iter().all(|logical_mesh| {
                logical_mesh.name_count == 0
                    && matches!(
                        logical_mesh.name_presence,
                        PRESENCE_ABSENT | PRESENCE_PRESENT
                    )
                    && (logical_mesh.name_presence != PRESENCE_PRESENT
                        || validate_entity_name_str("").is_ok())
            }));
    };
    let mut logical_mesh_index = 0_usize;
    let mut segment_len = 0_usize;
    let mut segment = [0_u8; MAX_ENTITY_NAME_BYTES];
    let Some(name_byte_count) = usize::try_from(name_byte_count).ok() else {
        return Ok(false);
    };
    let complete = session.visit_canonical_utf8_bytes(names, name_byte_count, |_index, byte| {
        while let Some(logical_mesh) = logical_meshes.get(logical_mesh_index) {
            if logical_mesh.name_count != 0 {
                break;
            }
            if !matches!(
                logical_mesh.name_presence,
                PRESENCE_ABSENT | PRESENCE_PRESENT
            ) || (logical_mesh.name_presence == PRESENCE_PRESENT
                && validate_entity_name_str("").is_err())
            {
                return false;
            }
            logical_mesh_index += 1;
        }
        let Some(logical_mesh) = logical_meshes.get(logical_mesh_index) else {
            return false;
        };
        let Some(segment_count) = usize::try_from(logical_mesh.name_count).ok() else {
            return false;
        };
        if logical_mesh.name_presence != PRESENCE_PRESENT
            || segment_count > segment.len()
            || segment_len >= segment_count
        {
            return false;
        }
        segment[segment_len] = byte;
        segment_len += 1;
        if segment_len != segment_count {
            return true;
        }
        let valid = std::str::from_utf8(&segment[..segment_len])
            .ok()
            .is_some_and(|name| validate_entity_name_str(name).is_ok());
        logical_mesh_index += 1;
        segment_len = 0;
        valid
    })?;
    while let Some(logical_mesh) = logical_meshes.get(logical_mesh_index) {
        if logical_mesh.name_count != 0
            || !matches!(
                logical_mesh.name_presence,
                PRESENCE_ABSENT | PRESENCE_PRESENT
            )
            || (logical_mesh.name_presence == PRESENCE_PRESENT
                && validate_entity_name_str("").is_err())
        {
            return Ok(false);
        }
        logical_mesh_index += 1;
    }
    Ok(complete && segment_len == 0 && logical_mesh_index == logical_meshes.len())
}

#[allow(
    clippy::too_many_arguments,
    reason = "typed mesh-instance validation keeps node and logical-mesh dependencies explicit"
)]
fn validate_mesh_instance_table_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<
        CanonicalValidationMember<'session, RecordBlock, GeometryMeshInstanceTableSchema>,
    >,
    nodes: CanonicalValidationMember<'session, RecordBlock, NodeSceneNodeTableSchema>,
    canonical_node_count: u32,
    logical_meshes: Option<
        CanonicalValidationMember<'session, RecordBlock, GeometryLogicalMeshTableSchema>,
    >,
    logical_mesh_count: u32,
    projections: &mut [GeometryMeshInstanceValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if projections.is_empty() {
        return Ok(member.is_none());
    }
    let (Some(member), Some(logical_meshes)) = (member, logical_meshes) else {
        return Ok(false);
    };
    session.visit_canonical_records(
        member,
        projections.len(),
        GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
        |row_index, row| {
            let mesh_instance = row
                .scalar(geometry_mesh_instance_field::MESH_INSTANCE, |value: u32| {
                    value
                })?;
            let owner_node =
                row.scalar(geometry_mesh_instance_field::OWNER_NODE, |value: u32| value)?;
            let nodes_member = row.record_ref::<NodeSceneNodeTableSchema, _>(
                geometry_mesh_instance_field::NODES,
                |value| value,
            )?;
            let logical_mesh = row
                .scalar(geometry_mesh_instance_field::LOGICAL_MESH, |value: u32| {
                    value
                })?;
            let logical_meshes_member = row.record_ref::<GeometryLogicalMeshTableSchema, _>(
                geometry_mesh_instance_field::LOGICAL_MESHES,
                |value| value,
            )?;
            let Some(expected_mesh_instance) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let (Some(owner_node), Some(logical_mesh)) = (owner_node, logical_mesh) else {
                return Ok(false);
            };
            let valid = require_scalar(mesh_instance, expected_mesh_instance).is_some()
                && owner_node != 0
                && owner_node < canonical_node_count
                && nodes_member == Some(nodes)
                && logical_mesh < logical_mesh_count
                && logical_meshes_member == Some(logical_meshes);
            if valid {
                let Some(destination) = projections.get_mut(row_index) else {
                    return Ok(false);
                };
                *destination = GeometryMeshInstanceValidationProjection { logical_mesh };
            }
            Ok(valid)
        },
    )
}

fn validate_logical_mesh_table(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryLogicalMeshTableSchema>>,
    primitive_reference: Option<CanonicalRecordRef<GeometryPrimitiveTableSchema>>,
    primitive_count: u32,
    logical_mesh_names: Option<CanonicalUtf8Ref<GeometryLogicalMeshNameSchema>>,
    equivalence: &mut impl GeometryEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (primitive_count == 0 && logical_mesh_names.is_none()).then_some(());
    };
    let primitive_reference = primitive_reference?;
    let table = session.canonical_records(reference)?;
    let logical_mesh_names = match logical_mesh_names {
        Some(reference) => Some((reference, session.canonical_utf8(reference)?)),
        None => None,
    };
    let mut primitive_cursor = 0_u32;
    let mut name_cursor = 0_usize;
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
        let name_presence = table.scalar::<u8>(row, geometry_logical_mesh_field::NAME_PRESENCE)?;
        let name_offset = table.scalar::<u32>(row, geometry_logical_mesh_field::NAME_OFFSET)?;
        let name_count = table.scalar::<u32>(row, geometry_logical_mesh_field::NAME_COUNT)?;
        require_scalar(Some(name_offset), u32::try_from(name_cursor).ok()?)?;
        let (name, next_name_cursor) = match (name_presence, name_count) {
            (PRESENCE_ABSENT, 0) => {
                require_scalar(
                    table.scalar::<u8>(row, geometry_logical_mesh_field::NAMES),
                    0_u8,
                )?;
                (None, name_cursor)
            }
            (PRESENCE_PRESENT, 0) => {
                require_scalar(
                    table.scalar::<u8>(row, geometry_logical_mesh_field::NAMES),
                    0_u8,
                )?;
                validate_entity_name_str("").ok()?;
                (Some(""), name_cursor)
            }
            (PRESENCE_PRESENT, _) => {
                let (names_reference, payload) = logical_mesh_names?;
                require_scalar(
                    table.utf8_ref::<GeometryLogicalMeshNameSchema>(
                        row,
                        geometry_logical_mesh_field::NAMES,
                    ),
                    names_reference,
                )?;
                let range = checked_range(name_offset, name_count, payload.len())?;
                observe_work(u64::try_from(range.len()).ok()?)?;
                let name = payload.get(range.clone())?;
                validate_entity_name_str(name).ok()?;
                (Some(name), range.end)
            }
            _ => return None,
        };
        name_cursor = next_name_cursor;
        equivalence.validate_logical_mesh(row, table, primitive_reference, name)?;
    }
    (primitive_cursor == primitive_count
        && name_cursor == logical_mesh_names.map_or(0, |(_, payload)| payload.len()))
    .then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_primitive_table(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<GeometryPrimitiveTableSchema>>,
    material_reference: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    material_count: u32,
    attribute_reference: Option<CanonicalRecordRef<GeometryAttributeTableSchema>>,
    attribute_table: Option<CanonicalRecordView<'_>>,
    attribute_semantic_names: Option<CanonicalUtf8Ref<GeometryAttributeSemanticNameSchema>>,
    equivalence: &mut impl GeometryEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<u32> {
    let Some(reference) = reference else {
        return attribute_reference.is_none().then_some(0);
    };
    let table = session.canonical_records(reference)?;
    let attribute_semantic_names = match attribute_semantic_names {
        Some(reference) => Some(session.canonical_utf8(reference)?),
        None => None,
    };
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
                    attribute_semantic_names,
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
    attribute_semantic_names: Option<&str>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let mut previous_key = None;
    let mut has_position = false;
    for row in range {
        observe_work(1)?;
        let semantic = table.scalar::<u8>(row, geometry_attribute_field::SEMANTIC_KIND)?;
        let semantic_set = table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_SET)?;
        let domain = table.scalar::<u8>(row, geometry_attribute_field::DOMAIN)?;
        let semantic_name = if semantic == GEOMETRY_SEMANTIC_CUSTOM {
            let names = attribute_semantic_names?;
            let offset =
                table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_NAME_OFFSET)?;
            let count = table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_NAME_COUNT)?;
            let range = checked_range(offset, count, names.len())?;
            names.get(range)
        } else {
            None
        };
        let key = (semantic, semantic_set, semantic_name, domain);
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
    semantic_names_reference: Option<CanonicalUtf8Ref<GeometryAttributeSemanticNameSchema>>,
    equivalence: &mut impl GeometryEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<(Option<CanonicalRecordView<'storage>>, u32)> {
    let Some(reference) = reference else {
        return semantic_names_reference.is_none().then_some((None, 0));
    };
    let table = session.canonical_records(reference)?;
    let semantic_names = match semantic_names_reference {
        Some(reference) => Some((reference, session.canonical_utf8(reference)?)),
        None => None,
    };
    let mut semantic_name_cursor = 0_usize;
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
        let semantic_name_offset =
            table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_NAME_OFFSET)?;
        let semantic_name_count =
            table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_NAME_COUNT)?;
        require_scalar(
            Some(semantic_name_offset),
            u32::try_from(semantic_name_cursor).ok()?,
        )?;
        let semantic_name = if semantic == GEOMETRY_SEMANTIC_CUSTOM {
            if semantic_name_count == 0 {
                return None;
            }
            let (semantic_names_reference, semantic_names) = semantic_names?;
            require_scalar(
                table.utf8_ref::<GeometryAttributeSemanticNameSchema>(
                    row,
                    geometry_attribute_field::SEMANTIC_NAMES,
                ),
                semantic_names_reference,
            )?;
            let range = checked_range(
                semantic_name_offset,
                semantic_name_count,
                semantic_names.len(),
            )?;
            observe_work(u64::try_from(range.len()).ok()?)?;
            let name = semantic_names.get(range.clone())?;
            CustomAttributeSemantic::new(name).ok()?;
            semantic_name_cursor = range.end;
            Some(name)
        } else {
            require_scalar(Some(semantic_name_count), 0_u32)?;
            require_scalar(
                table.scalar::<u8>(row, geometry_attribute_field::SEMANTIC_NAMES),
                0_u8,
            )?;
            None
        };
        equivalence.validate_attribute(row, table, semantic_name)?;
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
    (semantic_name_cursor == semantic_names.map_or(0, |(_, names)| names.len()))
        .then_some((Some(table), payload_block_count))
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
        (GEOMETRY_PAYLOAD_DENSE, GEOMETRY_SCALAR_I32) => {
            require_scalar(Some(explicit_count), element_count)?;
            require_scalar(
                Some(canonical_scalar_count),
                u64::try_from(dense_scalar_count).ok()?,
            )?;
            let expected = match expectation {
                GeometryAttributePayloadExpectation::StructuralOnly => None,
                GeometryAttributePayloadExpectation::Exact(
                    GeometryAttributePayloadAuthoring::DenseI32 { values, .. },
                ) => Some(values),
                GeometryAttributePayloadExpectation::Exact(_) => return None,
            };
            let values = read_i32_leaf::<GeometryI32ValuesSchema>(
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
            Some(CanonicalGeometryAttributePayload::DenseI32 { values })
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
        (GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_SCALAR_I32) => {
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
                    GeometryAttributePayloadAuthoring::SparseI32 {
                        indices,
                        values,
                        default,
                        ..
                    },
                ) => (Some(indices), Some(values), Some(default)),
                GeometryAttributePayloadExpectation::Exact(_) => return None,
            };
            let values = read_i32_leaf::<GeometryI32ValuesSchema>(
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
            let default = read_i32_leaf::<GeometrySparseI32DefaultsSchema>(
                session,
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                lane_count,
                expected_default,
                observe_work,
            )?;
            Some(CanonicalGeometryAttributePayload::SparseI32 {
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
        GEOMETRY_SEMANTIC_CUSTOM => {
            semantic_set == 0
                && matches!(
                    domain,
                    GEOMETRY_DOMAIN_VERTEX | GEOMETRY_DOMAIN_CORNER | GEOMETRY_DOMAIN_FACE
                )
                && matches!(
                    scalar_kind,
                    GEOMETRY_SCALAR_F32 | GEOMETRY_SCALAR_U32 | GEOMETRY_SCALAR_I32
                )
                && (1..=16).contains(&lanes)
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

#[allow(
    clippy::too_many_arguments,
    reason = "the leaf reader keeps schema coordinates, exact typed expectations, and work accounting explicit"
)]
fn read_i32_leaf<'session, 'storage, Tag>(
    session: &mut StoreValidationSession<'session, 'storage>,
    table: CanonicalRecordView<'storage>,
    row: usize,
    field: usize,
    expected_len: usize,
    expectation: Option<&[i32]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage [i32]>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<i32>>,
{
    if expectation.is_some_and(|expectation| expectation.len() != expected_len) {
        return None;
    }
    if expected_len == 0 {
        require_absent_leaf(table, row, field)?;
        return Some(&[]);
    }
    let reference = table.sequence_ref::<i32, Tag>(row, field)?;
    let values = session.canonical_sequence(reference)?.as_slice();
    if values.len() != expected_len {
        return None;
    }
    for (index, &value) in values.iter().enumerate() {
        observe_work(1)?;
        if expectation.is_some_and(|expectation| expectation.get(index) != Some(&value)) {
            return None;
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
    semantic_name: Option<&str>,
    expected: &ValidatedGeometryBatch<'_, R>,
) -> Option<()> {
    let attribute = expected.attribute(row)?;
    let (semantic_kind, semantic_set) = encode_geometry_semantic(attribute.semantic())?;
    let domain = encode_geometry_domain(attribute.domain())?;
    let scalar_kind = match attribute.scalar_kind() {
        GeometryAttributeScalarKind::F32 => GEOMETRY_SCALAR_F32,
        GeometryAttributeScalarKind::U32 => GEOMETRY_SCALAR_U32,
        GeometryAttributeScalarKind::I32 => GEOMETRY_SCALAR_I32,
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
    )?;
    require_scalar(
        table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_NAME_OFFSET),
        attribute.semantic_name_offset(),
    )?;
    require_scalar(
        table.scalar::<u32>(row, geometry_attribute_field::SEMANTIC_NAME_COUNT),
        attribute.semantic_name_count(),
    )?;
    match attribute.semantic() {
        crate::attribute::AttributeSemantic::Custom(custom) => {
            (semantic_name == Some(custom.as_str())).then_some(())
        }
        _ => semantic_name.is_none().then_some(()),
    }
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
    name: Option<&str>,
    expected: &ValidatedGeometryBatch<'_, R>,
) -> Option<()> {
    let logical_mesh = *expected.input().logical_meshes().get(row)?;
    let primitives_range = logical_mesh.primitives();
    require_scalar(
        table.scalar::<u32>(row, geometry_logical_mesh_field::PRIMITIVE_OFFSET),
        primitives_range.offset(),
    )?;
    require_scalar(
        table.scalar::<u32>(row, geometry_logical_mesh_field::PRIMITIVE_COUNT),
        primitives_range.count(),
    )?;
    require_scalar(
        table.record_ref::<GeometryPrimitiveTableSchema>(
            row,
            geometry_logical_mesh_field::PRIMITIVES,
        ),
        primitives,
    )?;
    match logical_mesh.name() {
        Presence::Absent => name.is_none().then_some(()),
        Presence::Present(expected_name) => (name == Some(expected_name)).then_some(()),
    }
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
