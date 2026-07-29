use crate::attribute::{AttributeDomain, AttributeSemantic};

use super::appearance::MaterialLocalId;
use super::{NodeLocalId, Presence};

/// One checked-at-validation contiguous range in a flat authoring table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::canonical_scene::store) struct AuthoringRange {
    offset: u32,
    count: u32,
}

impl AuthoringRange {
    pub(in crate::canonical_scene::store) const fn new(offset: u32, count: u32) -> Self {
        Self { offset, count }
    }

    pub(in crate::canonical_scene::store) const fn offset(self) -> u32 {
        self.offset
    }

    pub(in crate::canonical_scene::store) const fn count(self) -> u32 {
        self.count
    }
}

/// Identity of one logical mesh within an authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) struct LogicalMeshLocalId(u32);

impl LogicalMeshLocalId {
    pub(in crate::canonical_scene::store) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

/// Borrowed topology authored for one primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum GeometryTopologyAuthoring<'a> {
    Points,
    LineList,
    LineStrip,
    LineLoop,
    TriangleList,
    TriangleStrip,
    TriangleFan,
    Polygons(&'a [u32]),
}

/// Borrowed canonical attribute payload, before retained leaf construction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) enum GeometryAttributePayloadAuthoring<'a> {
    DenseF32 {
        lanes: u8,
        source_values: &'a [f32],
        source_indices: Presence<&'a [u32]>,
    },
    DenseU32 {
        lanes: u8,
        values: &'a [u32],
    },
    SparseF32 {
        lanes: u8,
        logical_count: u32,
        indices: &'a [u32],
        values: &'a [f32],
        default: &'a [f32],
    },
    SparseU32 {
        lanes: u8,
        logical_count: u32,
        indices: &'a [u32],
        values: &'a [u32],
        default: &'a [u32],
    },
}

/// One primitive-owned built-in attribute descriptor.
#[derive(Debug, PartialEq)]
pub(in crate::canonical_scene::store) struct GeometryAttributeAuthoring<'a> {
    semantic: AttributeSemantic,
    domain: AttributeDomain,
    payload: GeometryAttributePayloadAuthoring<'a>,
}

impl<'a> GeometryAttributeAuthoring<'a> {
    pub(in crate::canonical_scene::store) const fn f32(
        semantic: AttributeSemantic,
        domain: AttributeDomain,
        lanes: u8,
        source_values: &'a [f32],
        source_indices: Presence<&'a [u32]>,
    ) -> Self {
        Self {
            semantic,
            domain,
            payload: GeometryAttributePayloadAuthoring::DenseF32 {
                lanes,
                source_values,
                source_indices,
            },
        }
    }

    pub(in crate::canonical_scene::store) const fn u32(
        semantic: AttributeSemantic,
        domain: AttributeDomain,
        lanes: u8,
        values: &'a [u32],
    ) -> Self {
        Self {
            semantic,
            domain,
            payload: GeometryAttributePayloadAuthoring::DenseU32 { lanes, values },
        }
    }

    pub(in crate::canonical_scene::store) const fn sparse_f32(
        semantic: AttributeSemantic,
        domain: AttributeDomain,
        lanes: u8,
        logical_count: u32,
        indices: &'a [u32],
        values: &'a [f32],
        default: &'a [f32],
    ) -> Self {
        Self {
            semantic,
            domain,
            payload: GeometryAttributePayloadAuthoring::SparseF32 {
                lanes,
                logical_count,
                indices,
                values,
                default,
            },
        }
    }

    pub(in crate::canonical_scene::store) const fn sparse_u32(
        semantic: AttributeSemantic,
        domain: AttributeDomain,
        lanes: u8,
        logical_count: u32,
        indices: &'a [u32],
        values: &'a [u32],
        default: &'a [u32],
    ) -> Self {
        Self {
            semantic,
            domain,
            payload: GeometryAttributePayloadAuthoring::SparseU32 {
                lanes,
                logical_count,
                indices,
                values,
                default,
            },
        }
    }

    pub(in crate::canonical_scene::store) const fn semantic(&self) -> &AttributeSemantic {
        &self.semantic
    }

    pub(in crate::canonical_scene::store) const fn domain(&self) -> AttributeDomain {
        self.domain
    }

    pub(in crate::canonical_scene::store) const fn payload(
        &self,
    ) -> GeometryAttributePayloadAuthoring<'a> {
        self.payload
    }
}

/// One primitive descriptor using flat attribute ownership ranges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct GeometryPrimitiveAuthoring<'a> {
    topology: GeometryTopologyAuthoring<'a>,
    indices: Presence<&'a [u32]>,
    material: Presence<MaterialLocalId>,
    attributes: AuthoringRange,
}

impl<'a> GeometryPrimitiveAuthoring<'a> {
    pub(in crate::canonical_scene::store) const fn new(
        topology: GeometryTopologyAuthoring<'a>,
        indices: Presence<&'a [u32]>,
        material: Presence<MaterialLocalId>,
        attributes: AuthoringRange,
    ) -> Self {
        Self {
            topology,
            indices,
            material,
            attributes,
        }
    }

    pub(in crate::canonical_scene::store) const fn topology(self) -> GeometryTopologyAuthoring<'a> {
        self.topology
    }

    pub(in crate::canonical_scene::store) const fn indices(self) -> Presence<&'a [u32]> {
        self.indices
    }

    pub(in crate::canonical_scene::store) const fn material(self) -> Presence<MaterialLocalId> {
        self.material
    }

    pub(in crate::canonical_scene::store) const fn attributes(self) -> AuthoringRange {
        self.attributes
    }
}

/// One logical mesh descriptor using a flat primitive ownership range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct GeometryLogicalMeshAuthoring {
    primitives: AuthoringRange,
}

impl GeometryLogicalMeshAuthoring {
    pub(in crate::canonical_scene::store) const fn new(primitives: AuthoringRange) -> Self {
        Self { primitives }
    }

    pub(in crate::canonical_scene::store) const fn primitives(self) -> AuthoringRange {
        self.primitives
    }
}

/// One node-owned instance of a logical mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct GeometryMeshInstanceAuthoring {
    owner: NodeLocalId,
    logical_mesh: LogicalMeshLocalId,
}

impl GeometryMeshInstanceAuthoring {
    pub(in crate::canonical_scene::store) const fn new(
        owner: NodeLocalId,
        logical_mesh: LogicalMeshLocalId,
    ) -> Self {
        Self {
            owner,
            logical_mesh,
        }
    }

    pub(in crate::canonical_scene::store) const fn owner(self) -> NodeLocalId {
        self.owner
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh(self) -> LogicalMeshLocalId {
        self.logical_mesh
    }
}

/// Borrowed, flat geometry-domain input supplied by a format adapter.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryAuthoringBatch<'a> {
    logical_meshes: &'a [GeometryLogicalMeshAuthoring],
    primitives: &'a [GeometryPrimitiveAuthoring<'a>],
    attributes: &'a [GeometryAttributeAuthoring<'a>],
    mesh_instances: &'a [GeometryMeshInstanceAuthoring],
}

impl<'a> GeometryAuthoringBatch<'a> {
    pub(in crate::canonical_scene::store) const fn empty() -> Self {
        Self::new(&[], &[], &[], &[])
    }

    pub(in crate::canonical_scene::store) const fn new(
        logical_meshes: &'a [GeometryLogicalMeshAuthoring],
        primitives: &'a [GeometryPrimitiveAuthoring<'a>],
        attributes: &'a [GeometryAttributeAuthoring<'a>],
        mesh_instances: &'a [GeometryMeshInstanceAuthoring],
    ) -> Self {
        Self {
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
        }
    }

    pub(in crate::canonical_scene::store) const fn logical_meshes(
        self,
    ) -> &'a [GeometryLogicalMeshAuthoring] {
        self.logical_meshes
    }

    pub(in crate::canonical_scene::store) const fn primitives(
        self,
    ) -> &'a [GeometryPrimitiveAuthoring<'a>] {
        self.primitives
    }

    pub(in crate::canonical_scene::store) const fn attributes(
        self,
    ) -> &'a [GeometryAttributeAuthoring<'a>] {
        self.attributes
    }

    pub(in crate::canonical_scene::store) const fn mesh_instances(
        self,
    ) -> &'a [GeometryMeshInstanceAuthoring] {
        self.mesh_instances
    }
}
