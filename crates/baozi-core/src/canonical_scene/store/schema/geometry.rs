//! Private canonical Scene geometry schema.

use super::CanonicalSchema;
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::geometry::TopologyKind;
use baozi_operation::{RecordBlock, ScalarSequence, Utf8Block};

pub(in super::super) const GEOMETRY_LOGICAL_MESH_FIELD_COUNT: usize = 8;
pub(in super::super) const GEOMETRY_MESH_INSTANCE_FIELD_COUNT: usize = 5;
pub(in super::super) const GEOMETRY_PRIMITIVE_FIELD_COUNT: usize = 17;
pub(in super::super) const GEOMETRY_ATTRIBUTE_FIELD_COUNT: usize = 17;

pub(in super::super) const GEOMETRY_TOPOLOGY_POINTS: u8 = 0;
pub(in super::super) const GEOMETRY_TOPOLOGY_LINE_LIST: u8 = 1;
pub(in super::super) const GEOMETRY_TOPOLOGY_LINE_STRIP: u8 = 2;
pub(in super::super) const GEOMETRY_TOPOLOGY_LINE_LOOP: u8 = 3;
pub(in super::super) const GEOMETRY_TOPOLOGY_TRIANGLE_LIST: u8 = 4;
pub(in super::super) const GEOMETRY_TOPOLOGY_TRIANGLE_STRIP: u8 = 5;
pub(in super::super) const GEOMETRY_TOPOLOGY_TRIANGLE_FAN: u8 = 6;
pub(in super::super) const GEOMETRY_TOPOLOGY_POLYGONS: u8 = 7;

pub(in super::super) const GEOMETRY_SEMANTIC_POSITION: u8 = 0;
pub(in super::super) const GEOMETRY_SEMANTIC_NORMAL: u8 = 1;
pub(in super::super) const GEOMETRY_SEMANTIC_TANGENT: u8 = 2;
pub(in super::super) const GEOMETRY_SEMANTIC_TEXCOORD: u8 = 3;
pub(in super::super) const GEOMETRY_SEMANTIC_COLOR: u8 = 4;
pub(in super::super) const GEOMETRY_SEMANTIC_SMOOTHING_GROUP: u8 = 5;
pub(in super::super) const GEOMETRY_SEMANTIC_CUSTOM: u8 = 6;

pub(in super::super) const GEOMETRY_DOMAIN_VERTEX: u8 = 0;
pub(in super::super) const GEOMETRY_DOMAIN_CORNER: u8 = 1;
pub(in super::super) const GEOMETRY_DOMAIN_FACE: u8 = 2;

pub(in super::super) const GEOMETRY_SCALAR_F32: u8 = 0;
pub(in super::super) const GEOMETRY_SCALAR_U32: u8 = 1;
pub(in super::super) const GEOMETRY_SCALAR_I32: u8 = 2;
pub(in super::super) const GEOMETRY_SCALAR_F64: u8 = 3;
pub(in super::super) const GEOMETRY_NORMALIZATION_NONE: u8 = 0;
pub(in super::super) const GEOMETRY_PAYLOAD_DENSE: u8 = 0;
pub(in super::super) const GEOMETRY_PAYLOAD_SPARSE: u8 = 1;

pub(in super::super) const fn encode_geometry_topology(topology: TopologyKind) -> u8 {
    match topology {
        TopologyKind::Points => GEOMETRY_TOPOLOGY_POINTS,
        TopologyKind::LineList => GEOMETRY_TOPOLOGY_LINE_LIST,
        TopologyKind::LineStrip => GEOMETRY_TOPOLOGY_LINE_STRIP,
        TopologyKind::LineLoop => GEOMETRY_TOPOLOGY_LINE_LOOP,
        TopologyKind::TriangleList => GEOMETRY_TOPOLOGY_TRIANGLE_LIST,
        TopologyKind::TriangleStrip => GEOMETRY_TOPOLOGY_TRIANGLE_STRIP,
        TopologyKind::TriangleFan => GEOMETRY_TOPOLOGY_TRIANGLE_FAN,
        TopologyKind::Polygons => GEOMETRY_TOPOLOGY_POLYGONS,
    }
}

pub(in super::super) fn encode_geometry_semantic(
    semantic: &AttributeSemantic,
) -> Option<(u8, u32)> {
    match semantic {
        AttributeSemantic::Position => Some((GEOMETRY_SEMANTIC_POSITION, 0)),
        AttributeSemantic::Normal => Some((GEOMETRY_SEMANTIC_NORMAL, 0)),
        AttributeSemantic::Tangent => Some((GEOMETRY_SEMANTIC_TANGENT, 0)),
        AttributeSemantic::Texcoord(set) => Some((GEOMETRY_SEMANTIC_TEXCOORD, *set)),
        AttributeSemantic::Color(set) => Some((GEOMETRY_SEMANTIC_COLOR, *set)),
        AttributeSemantic::SmoothingGroup => Some((GEOMETRY_SEMANTIC_SMOOTHING_GROUP, 0)),
        AttributeSemantic::Custom(_) => Some((GEOMETRY_SEMANTIC_CUSTOM, 0)),
    }
}

pub(in super::super) const fn encode_geometry_domain(domain: AttributeDomain) -> Option<u8> {
    match domain {
        AttributeDomain::Vertex => Some(GEOMETRY_DOMAIN_VERTEX),
        AttributeDomain::Corner => Some(GEOMETRY_DOMAIN_CORNER),
        AttributeDomain::Face => Some(GEOMETRY_DOMAIN_FACE),
        AttributeDomain::Instance => None,
    }
}

pub(in super::super) mod geometry_logical_mesh_field {
    pub(crate) const LOGICAL_MESH: usize = 0;
    pub(crate) const PRIMITIVE_OFFSET: usize = 1;
    pub(crate) const PRIMITIVE_COUNT: usize = 2;
    pub(crate) const PRIMITIVES: usize = 3;
    pub(crate) const NAME_PRESENCE: usize = 4;
    pub(crate) const NAME_OFFSET: usize = 5;
    pub(crate) const NAME_COUNT: usize = 6;
    pub(crate) const NAMES: usize = 7;
}

pub(in super::super) mod geometry_mesh_instance_field {
    pub(crate) const MESH_INSTANCE: usize = 0;
    pub(crate) const OWNER_NODE: usize = 1;
    pub(crate) const NODES: usize = 2;
    pub(crate) const LOGICAL_MESH: usize = 3;
    pub(crate) const LOGICAL_MESHES: usize = 4;
}

pub(in super::super) mod geometry_primitive_field {
    pub(crate) const PRIMITIVE: usize = 0;
    pub(crate) const TOPOLOGY: usize = 1;
    pub(crate) const VERTEX_COUNT: usize = 2;
    pub(crate) const CORNER_COUNT: usize = 3;
    pub(crate) const FACE_COUNT: usize = 4;
    pub(crate) const POLYGON_BOUNDARIES_PRESENCE: usize = 5;
    pub(crate) const POLYGON_BOUNDARIES_COUNT: usize = 6;
    pub(crate) const POLYGON_BOUNDARIES: usize = 7;
    pub(crate) const INDEX_PRESENCE: usize = 8;
    pub(crate) const INDEX_COUNT: usize = 9;
    pub(crate) const INDICES: usize = 10;
    pub(crate) const MATERIAL_PRESENCE: usize = 11;
    pub(crate) const MATERIAL: usize = 12;
    pub(crate) const MATERIALS: usize = 13;
    pub(crate) const ATTRIBUTE_OFFSET: usize = 14;
    pub(crate) const ATTRIBUTE_COUNT: usize = 15;
    pub(crate) const ATTRIBUTES: usize = 16;
}

pub(in super::super) mod geometry_attribute_field {
    pub(crate) const ATTRIBUTE: usize = 0;
    pub(crate) const SEMANTIC_KIND: usize = 1;
    pub(crate) const SEMANTIC_SET: usize = 2;
    pub(crate) const DOMAIN: usize = 3;
    pub(crate) const SCALAR_KIND: usize = 4;
    pub(crate) const LANE_COUNT: usize = 5;
    pub(crate) const NORMALIZATION: usize = 6;
    pub(crate) const PAYLOAD_KIND: usize = 7;
    pub(crate) const ELEMENT_COUNT: usize = 8;
    pub(crate) const EXPLICIT_COUNT: usize = 9;
    pub(crate) const CANONICAL_SCALAR_COUNT: usize = 10;
    pub(crate) const VALUES: usize = 11;
    pub(crate) const SPARSE_INDICES: usize = 12;
    pub(crate) const SPARSE_DEFAULT: usize = 13;
    pub(crate) const SEMANTIC_NAME_OFFSET: usize = 14;
    pub(crate) const SEMANTIC_NAME_COUNT: usize = 15;
    pub(crate) const SEMANTIC_NAMES: usize = 16;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryLogicalMeshTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryLogicalMeshNameSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryMeshInstanceTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryPrimitiveTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryAttributeTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryPolygonBoundariesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryPrimitiveIndicesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryF32ValuesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryF64ValuesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryU32ValuesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryI32ValuesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometrySparseIndicesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometrySparseF32DefaultsSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometrySparseU32DefaultsSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometrySparseI32DefaultsSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct GeometryAttributeSemanticNameSchema;

impl CanonicalSchema for GeometryLogicalMeshTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for GeometryLogicalMeshNameSchema {
    type StoreKind = Utf8Block;
}

impl CanonicalSchema for GeometryMeshInstanceTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for GeometryPrimitiveTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for GeometryAttributeTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for GeometryPolygonBoundariesSchema {
    type StoreKind = ScalarSequence<u32>;
}

impl CanonicalSchema for GeometryPrimitiveIndicesSchema {
    type StoreKind = ScalarSequence<u32>;
}

impl CanonicalSchema for GeometryF32ValuesSchema {
    type StoreKind = ScalarSequence<f32>;
}

impl CanonicalSchema for GeometryF64ValuesSchema {
    type StoreKind = ScalarSequence<f64>;
}

impl CanonicalSchema for GeometryU32ValuesSchema {
    type StoreKind = ScalarSequence<u32>;
}

impl CanonicalSchema for GeometryI32ValuesSchema {
    type StoreKind = ScalarSequence<i32>;
}

impl CanonicalSchema for GeometrySparseIndicesSchema {
    type StoreKind = ScalarSequence<u32>;
}

impl CanonicalSchema for GeometrySparseF32DefaultsSchema {
    type StoreKind = ScalarSequence<f32>;
}

impl CanonicalSchema for GeometrySparseU32DefaultsSchema {
    type StoreKind = ScalarSequence<u32>;
}

impl CanonicalSchema for GeometrySparseI32DefaultsSchema {
    type StoreKind = ScalarSequence<i32>;
}

impl CanonicalSchema for GeometryAttributeSemanticNameSchema {
    type StoreKind = Utf8Block;
}
