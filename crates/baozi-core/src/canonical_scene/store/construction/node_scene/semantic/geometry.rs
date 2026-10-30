use baozi_operation::StoredScalar;

use crate::attribute::AttributeDomain;
use crate::canonical_scene::store::typed_storage::{
    GeometryStorage, StoredAttributePayload, StoredAttributeSemantic, TableRange,
};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::geometry::TopologyKind;

use super::{NodeSceneSemanticResult, invariant};

/// Borrowed actual geometry tables; no record decoding or immutable projection allocation.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryTables<'scene> {
    storage: &'scene GeometryStorage,
}

impl<'scene> NodeSceneGeometryTables<'scene> {
    pub(in crate::canonical_scene::store) fn try_new(
        storage: &'scene GeometryStorage,
        proof: &NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        if [
            storage.logical_meshes.len(),
            storage.primitives.len(),
            storage.attributes.len(),
            storage.mesh_instances.len(),
        ] != [
            proof.logical_mesh_count(),
            proof.primitive_count(),
            proof.attribute_count(),
            proof.mesh_instance_count(),
        ] {
            return Err(invariant(
                "typed geometry tables disagree with central validation proof",
            ));
        }
        Ok(Self { storage })
    }

    pub(super) const fn into_cursor(self) -> NodeSceneGeometryCursor<'scene> {
        NodeSceneGeometryCursor {
            tables: self,
            phase: GeometryCursorPhase::LogicalMeshes,
            next_logical_mesh: 0,
            next_primitive: 0,
            next_attribute: 0,
            next_mesh_instance: 0,
        }
    }

    pub(in crate::canonical_scene::store) fn logical_mesh_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneGeometryLogicalMesh<'scene>> {
        let mesh = self
            .storage
            .logical_meshes
            .get(row)
            .ok_or_else(|| invariant("logical-mesh row is absent"))?;
        let name = mesh
            .name
            .map(|range| {
                range
                    .get(&self.storage.names)
                    .ok_or_else(|| invariant("logical-mesh name range is unavailable"))
            })
            .transpose()?;
        Ok(NodeSceneGeometryLogicalMesh {
            source_index: row,
            name,
            primitive_offset: mesh.primitives.start(),
            primitive_count: mesh.primitives.len(),
        })
    }

    pub(in crate::canonical_scene::store) fn primitive_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneGeometryPrimitive<'scene>> {
        let primitive = self
            .storage
            .primitives
            .get(row)
            .ok_or_else(|| invariant("primitive row is absent"))?;
        let polygon_boundaries = primitive
            .polygon_boundaries
            .map(|range| leaf(range, &self.storage.u32_values))
            .transpose()?;
        let indices = primitive
            .indices
            .map(|range| leaf(range, &self.storage.u32_values))
            .transpose()?;
        Ok(NodeSceneGeometryPrimitive {
            source_index: row,
            topology: primitive.topology,
            vertex_count: primitive.vertex_count,
            corner_count: primitive.corner_count,
            face_count: primitive.face_count,
            polygon_boundaries_present: polygon_boundaries.is_some(),
            polygon_boundary_count: polygon_boundaries
                .as_ref()
                .map_or(0, |values| values.declared_count()),
            polygon_boundaries,
            indices_present: indices.is_some(),
            index_count: indices.as_ref().map_or(0, |values| values.declared_count()),
            indices,
            material: primitive.material.map(|index| index as usize),
            attribute_offset: primitive.attributes.start(),
            attribute_count: primitive.attributes.len(),
        })
    }

    pub(in crate::canonical_scene::store) fn attribute_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneGeometryAttribute<'scene>> {
        let attribute = self
            .storage
            .attributes
            .get(row)
            .ok_or_else(|| invariant("attribute row is absent"))?;
        let semantic = match attribute.semantic {
            StoredAttributeSemantic::Position => NodeSceneGeometryAttributeSemantic::Position,
            StoredAttributeSemantic::Normal => NodeSceneGeometryAttributeSemantic::Normal,
            StoredAttributeSemantic::Tangent => NodeSceneGeometryAttributeSemantic::Tangent,
            StoredAttributeSemantic::Texcoord(set) => {
                NodeSceneGeometryAttributeSemantic::Texcoord(set)
            }
            StoredAttributeSemantic::Color(set) => NodeSceneGeometryAttributeSemantic::Color(set),
            StoredAttributeSemantic::SmoothingGroup => {
                NodeSceneGeometryAttributeSemantic::SmoothingGroup
            }
            StoredAttributeSemantic::Custom(range) => NodeSceneGeometryAttributeSemantic::Custom(
                range
                    .get(&self.storage.names)
                    .ok_or_else(|| invariant("attribute semantic name is unavailable"))?,
            ),
        };
        use NodeSceneGeometryAttributePayload as P;
        use NodeSceneGeometryScalarKind as S;
        let (payload, scalar_kind, sparse, explicit_count, scalar_count) = match attribute.payload {
            StoredAttributePayload::DenseF32(range) => (
                P::DenseF32 {
                    values: leaf(range, &self.storage.f32_values)?,
                },
                S::F32,
                false,
                attribute.element_count as usize,
                range.len(),
            ),
            StoredAttributePayload::DenseF64(range) => (
                P::DenseF64 {
                    values: leaf(range, &self.storage.f64_values)?,
                },
                S::F64,
                false,
                attribute.element_count as usize,
                range.len(),
            ),
            StoredAttributePayload::DenseU32(range) => (
                P::DenseU32 {
                    values: leaf(range, &self.storage.u32_values)?,
                },
                S::U32,
                false,
                attribute.element_count as usize,
                range.len(),
            ),
            StoredAttributePayload::DenseI32(range) => (
                P::DenseI32 {
                    values: leaf(range, &self.storage.i32_values)?,
                },
                S::I32,
                false,
                attribute.element_count as usize,
                range.len(),
            ),
            StoredAttributePayload::SparseF32 {
                indices,
                values,
                default,
            } => (
                P::SparseF32 {
                    indices: leaf(indices, &self.storage.u32_values)?,
                    values: leaf(values, &self.storage.f32_values)?,
                    default: leaf(default, &self.storage.f32_values)?,
                },
                S::F32,
                true,
                indices.len(),
                values
                    .len()
                    .checked_add(default.len())
                    .ok_or_else(|| invariant("attribute scalar count overflows"))?,
            ),
            StoredAttributePayload::SparseU32 {
                indices,
                values,
                default,
            } => (
                P::SparseU32 {
                    indices: leaf(indices, &self.storage.u32_values)?,
                    values: leaf(values, &self.storage.u32_values)?,
                    default: leaf(default, &self.storage.u32_values)?,
                },
                S::U32,
                true,
                indices.len(),
                values
                    .len()
                    .checked_add(default.len())
                    .ok_or_else(|| invariant("attribute scalar count overflows"))?,
            ),
            StoredAttributePayload::SparseI32 {
                indices,
                values,
                default,
            } => (
                P::SparseI32 {
                    indices: leaf(indices, &self.storage.u32_values)?,
                    values: leaf(values, &self.storage.i32_values)?,
                    default: leaf(default, &self.storage.i32_values)?,
                },
                S::I32,
                true,
                indices.len(),
                values
                    .len()
                    .checked_add(default.len())
                    .ok_or_else(|| invariant("attribute scalar count overflows"))?,
            ),
        };
        Ok(NodeSceneGeometryAttribute {
            source_index: row,
            semantic,
            domain: attribute.domain,
            scalar_kind,
            lanes: attribute.lanes,
            normalization: NodeSceneGeometryNormalization::None,
            payload_kind: if sparse {
                NodeSceneGeometryPayloadKind::Sparse
            } else {
                NodeSceneGeometryPayloadKind::Dense
            },
            element_count: attribute.element_count,
            explicit_element_count: u32::try_from(explicit_count)
                .map_err(|_| invariant("attribute explicit count overflows"))?,
            canonical_scalar_count: u64::try_from(scalar_count)
                .map_err(|_| invariant("attribute scalar count overflows"))?,
            payload,
        })
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneGeometryMeshInstance> {
        let instance = self
            .storage
            .mesh_instances
            .get(row)
            .ok_or_else(|| invariant("mesh-instance row is absent"))?;
        Ok(NodeSceneGeometryMeshInstance {
            source_index: row,
            owner_node: (instance.owner_node.0 as usize)
                .checked_sub(1)
                .ok_or_else(|| invariant("synthetic root cannot own a mesh instance"))?,
            logical_mesh: instance.logical_mesh.0 as usize,
        })
    }
}

fn leaf<'a, T: StoredScalar>(
    range: TableRange<T>,
    values: &'a [T],
) -> NodeSceneSemanticResult<NodeSceneGeometryLeafCursor<'a, T>> {
    range
        .get(values)
        .map(NodeSceneGeometryLeafCursor::new)
        .ok_or_else(|| invariant("typed geometry payload range is unavailable"))
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GeometryCursorPhase {
    LogicalMeshes,
    Primitives,
    Attributes,
    MeshInstances,
}

pub(in crate::canonical_scene::store) struct NodeSceneGeometryCursor<'scene> {
    tables: NodeSceneGeometryTables<'scene>,
    phase: GeometryCursorPhase,
    next_logical_mesh: usize,
    next_primitive: usize,
    next_attribute: usize,
    next_mesh_instance: usize,
}

impl<'scene> NodeSceneGeometryCursor<'scene> {
    pub(in crate::canonical_scene::store) fn logical_mesh_count(&self) -> usize {
        self.tables.storage.logical_meshes.len()
    }
    pub(in crate::canonical_scene::store) fn primitive_count(&self) -> usize {
        self.tables.storage.primitives.len()
    }
    pub(in crate::canonical_scene::store) fn attribute_count(&self) -> usize {
        self.tables.storage.attributes.len()
    }
    pub(in crate::canonical_scene::store) fn mesh_instance_count(&self) -> usize {
        self.tables.storage.mesh_instances.len()
    }
    pub(in crate::canonical_scene::store) fn next_logical_mesh(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryLogicalMesh<'scene>>> {
        self.enter_phase(GeometryCursorPhase::LogicalMeshes)?;
        if self.next_logical_mesh == self.logical_mesh_count() {
            return Ok(None);
        }
        let row = self.tables.logical_mesh_at(self.next_logical_mesh)?;
        self.next_logical_mesh += 1;
        Ok(Some(row))
    }
    pub(in crate::canonical_scene::store) fn next_primitive(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryPrimitive<'scene>>> {
        self.enter_phase(GeometryCursorPhase::Primitives)?;
        if self.next_primitive == self.primitive_count() {
            return Ok(None);
        }
        let row = self.tables.primitive_at(self.next_primitive)?;
        self.next_primitive += 1;
        Ok(Some(row))
    }
    pub(in crate::canonical_scene::store) fn next_attribute(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryAttribute<'scene>>> {
        self.enter_phase(GeometryCursorPhase::Attributes)?;
        if self.next_attribute == self.attribute_count() {
            return Ok(None);
        }
        let row = self.tables.attribute_at(self.next_attribute)?;
        self.next_attribute += 1;
        Ok(Some(row))
    }
    pub(in crate::canonical_scene::store) fn next_mesh_instance(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryMeshInstance>> {
        self.enter_phase(GeometryCursorPhase::MeshInstances)?;
        if self.next_mesh_instance == self.mesh_instance_count() {
            return Ok(None);
        }
        let row = self.tables.mesh_instance_at(self.next_mesh_instance)?;
        self.next_mesh_instance += 1;
        Ok(Some(row))
    }
    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if [
            self.next_logical_mesh,
            self.next_primitive,
            self.next_attribute,
            self.next_mesh_instance,
        ] != [
            self.logical_mesh_count(),
            self.primitive_count(),
            self.attribute_count(),
            self.mesh_instance_count(),
        ] {
            return Err(invariant(
                "geometry cursor ended before its proof-qualified counts",
            ));
        }
        Ok(())
    }
    fn enter_phase(&mut self, target: GeometryCursorPhase) -> NodeSceneSemanticResult<()> {
        if target < self.phase
            || (target >= GeometryCursorPhase::Primitives
                && self.next_logical_mesh != self.logical_mesh_count())
            || (target >= GeometryCursorPhase::Attributes
                && self.next_primitive != self.primitive_count())
            || (target >= GeometryCursorPhase::MeshInstances
                && self.next_attribute != self.attribute_count())
        {
            return Err(invariant(
                "geometry cursor must exhaust its tables in order",
            ));
        }
        self.phase = target;
        Ok(())
    }
}

/// One logical mesh and its exact primitive range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryLogicalMesh<'scene> {
    source_index: usize,
    name: Option<&'scene str>,
    primitive_offset: usize,
    primitive_count: usize,
}

impl<'scene> NodeSceneGeometryLogicalMesh<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn primitive_offset(self) -> usize {
        self.primitive_offset
    }

    pub(in crate::canonical_scene::store) const fn name(self) -> Option<&'scene str> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn primitive_count(self) -> usize {
        self.primitive_count
    }
}

/// One primitive row with lazily resolved, presence-preserving payload leaves.
#[must_use = "the primitive payload cursors must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryPrimitive<'scene> {
    source_index: usize,
    topology: TopologyKind,
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
    polygon_boundaries_present: bool,
    polygon_boundary_count: usize,
    polygon_boundaries: Option<NodeSceneGeometryU32Cursor<'scene>>,
    indices_present: bool,
    index_count: usize,
    indices: Option<NodeSceneGeometryU32Cursor<'scene>>,
    material: Option<usize>,
    attribute_offset: usize,
    attribute_count: usize,
}

impl<'scene> NodeSceneGeometryPrimitive<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn topology(&self) -> TopologyKind {
        self.topology
    }

    pub(in crate::canonical_scene::store) const fn vertex_count(&self) -> u32 {
        self.vertex_count
    }

    pub(in crate::canonical_scene::store) const fn corner_count(&self) -> u32 {
        self.corner_count
    }

    pub(in crate::canonical_scene::store) const fn face_count(&self) -> u32 {
        self.face_count
    }

    pub(in crate::canonical_scene::store) const fn polygon_boundaries_present(&self) -> bool {
        self.polygon_boundaries_present
    }

    pub(in crate::canonical_scene::store) const fn polygon_boundary_count(&self) -> usize {
        self.polygon_boundary_count
    }

    pub(in crate::canonical_scene::store) fn take_polygon_boundaries(
        &mut self,
    ) -> Option<NodeSceneGeometryU32Cursor<'scene>> {
        self.polygon_boundaries.take()
    }

    pub(in crate::canonical_scene::store) const fn indices_present(&self) -> bool {
        self.indices_present
    }

    pub(in crate::canonical_scene::store) const fn index_count(&self) -> usize {
        self.index_count
    }

    pub(in crate::canonical_scene::store) fn take_indices(
        &mut self,
    ) -> Option<NodeSceneGeometryU32Cursor<'scene>> {
        self.indices.take()
    }

    pub(in crate::canonical_scene::store) const fn material(&self) -> Option<usize> {
        self.material
    }

    pub(in crate::canonical_scene::store) const fn attribute_offset(&self) -> usize {
        self.attribute_offset
    }

    pub(in crate::canonical_scene::store) const fn attribute_count(&self) -> usize {
        self.attribute_count
    }
}

/// Allocation-free built-in semantic decoded from the canonical wire code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryAttributeSemantic<'scene> {
    Position,
    Normal,
    Tangent,
    Texcoord(u32),
    Color(u32),
    SmoothingGroup,
    Custom(&'scene str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryScalarKind {
    F32,
    F64,
    U32,
    I32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryNormalization {
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryPayloadKind {
    Dense,
    Sparse,
}

/// One attribute descriptor coupled to its exact typed payload cursors.
#[must_use = "the attribute payload cursors must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryAttribute<'scene> {
    source_index: usize,
    semantic: NodeSceneGeometryAttributeSemantic<'scene>,
    domain: AttributeDomain,
    scalar_kind: NodeSceneGeometryScalarKind,
    lanes: u8,
    normalization: NodeSceneGeometryNormalization,
    payload_kind: NodeSceneGeometryPayloadKind,
    element_count: u32,
    explicit_element_count: u32,
    canonical_scalar_count: u64,
    payload: NodeSceneGeometryAttributePayload<'scene>,
}

impl<'scene> NodeSceneGeometryAttribute<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn semantic(
        &self,
    ) -> NodeSceneGeometryAttributeSemantic<'scene> {
        self.semantic
    }

    pub(in crate::canonical_scene::store) const fn domain(&self) -> AttributeDomain {
        self.domain
    }

    pub(in crate::canonical_scene::store) const fn scalar_kind(
        &self,
    ) -> NodeSceneGeometryScalarKind {
        self.scalar_kind
    }

    pub(in crate::canonical_scene::store) const fn lanes(&self) -> u8 {
        self.lanes
    }

    pub(in crate::canonical_scene::store) const fn normalization(
        &self,
    ) -> NodeSceneGeometryNormalization {
        self.normalization
    }

    pub(in crate::canonical_scene::store) const fn payload_kind(
        &self,
    ) -> NodeSceneGeometryPayloadKind {
        self.payload_kind
    }

    pub(in crate::canonical_scene::store) const fn element_count(&self) -> u32 {
        self.element_count
    }

    pub(in crate::canonical_scene::store) const fn explicit_element_count(&self) -> u32 {
        self.explicit_element_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_scalar_count(&self) -> u64 {
        self.canonical_scalar_count
    }

    pub(in crate::canonical_scene::store) fn into_payload(
        self,
    ) -> NodeSceneGeometryAttributePayload<'scene> {
        self.payload
    }
}

/// Typed attribute payload; empty cursors remain present for authored empty data.
#[must_use = "every payload leaf cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryAttributePayload<'scene> {
    DenseF32 {
        values: NodeSceneGeometryF32Cursor<'scene>,
    },
    DenseF64 {
        values: NodeSceneGeometryLeafCursor<'scene, f64>,
    },
    DenseU32 {
        values: NodeSceneGeometryU32Cursor<'scene>,
    },
    DenseI32 {
        values: NodeSceneGeometryI32Cursor<'scene>,
    },
    SparseF32 {
        values: NodeSceneGeometryF32Cursor<'scene>,
        indices: NodeSceneGeometryU32Cursor<'scene>,
        default: NodeSceneGeometryF32Cursor<'scene>,
    },
    SparseU32 {
        values: NodeSceneGeometryU32Cursor<'scene>,
        indices: NodeSceneGeometryU32Cursor<'scene>,
        default: NodeSceneGeometryU32Cursor<'scene>,
    },
    SparseI32 {
        values: NodeSceneGeometryI32Cursor<'scene>,
        indices: NodeSceneGeometryU32Cursor<'scene>,
        default: NodeSceneGeometryI32Cursor<'scene>,
    },
}

/// One source-ordered mesh instance with canonical source-space references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryMeshInstance {
    source_index: usize,
    owner_node: usize,
    logical_mesh: usize,
}

impl NodeSceneGeometryMeshInstance {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn owner_node(self) -> usize {
        self.owner_node
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh(self) -> usize {
        self.logical_mesh
    }
}

/// Exact, allocation-free traversal of one typed scalar-sequence leaf.
#[must_use = "the payload leaf cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryLeafCursor<'scene, T: StoredScalar> {
    values: &'scene [T],
    next: usize,
}

pub(in crate::canonical_scene::store) type NodeSceneGeometryF32Cursor<'scene> =
    NodeSceneGeometryLeafCursor<'scene, f32>;
pub(in crate::canonical_scene::store) type NodeSceneGeometryU32Cursor<'scene> =
    NodeSceneGeometryLeafCursor<'scene, u32>;
pub(in crate::canonical_scene::store) type NodeSceneGeometryI32Cursor<'scene> =
    NodeSceneGeometryLeafCursor<'scene, i32>;

impl<'scene, T: StoredScalar> NodeSceneGeometryLeafCursor<'scene, T> {
    const fn new(values: &'scene [T]) -> Self {
        Self { values, next: 0 }
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.values.len()
    }

    pub(in crate::canonical_scene::store) const fn into_slice(self) -> &'scene [T] {
        self.values
    }

    pub(in crate::canonical_scene::store) fn next(&mut self) -> NodeSceneSemanticResult<Option<T>> {
        if self.next == self.values.len() {
            return Ok(None);
        }
        let value =
            self.values.get(self.next).copied().ok_or_else(|| {
                invariant("payload leaf cursor advanced beyond its declared count")
            })?;
        self.next = advance_row(self.next, "payload leaf cursor index overflows")?;
        Ok(Some(value))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next != self.values.len() {
            return Err(invariant(
                "payload leaf cursor ended before its declared count",
            ));
        }
        Ok(())
    }
}

fn advance_row(row: usize, reason: &'static str) -> NodeSceneSemanticResult<usize> {
    row.checked_add(1).ok_or_else(|| invariant(reason))
}
