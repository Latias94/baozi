//! Private node and geometry tables. Rows contain logical indices and pool ranges only.
//!
//! The containing Scene owns these tables through operation's retained-storage owner. In
//! particular, no row contains a publication-local concrete-store slot: an unchanged geometry
//! owner can therefore be shared by a new generation without rewriting its rows or payloads.

use std::marker::PhantomData;
use std::ops::Range;

use baozi_operation::__retained_store_integration::authority::SharedRetainedStorage as RetainedStorage;

use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::entity::AuthoredTransform;
use crate::geometry::TopologyKind;
use crate::math::{Mat4, Quat, Vec3};

pub(super) mod build;
pub(super) mod edit;
#[cfg(test)]
mod tests;
pub(super) mod validation;

/// The same actual table owners move from materialization through proof into the Scene root.
pub(super) struct NodeGeometryStorage {
    pub(super) nodes: RetainedStorage<NodeStorage>,
    pub(super) geometry: RetainedStorage<GeometryStorage>,
}

/// A half-open range in exactly one private typed table or scalar pool.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TableRange<T> {
    start: usize,
    count: usize,
    marker: PhantomData<fn() -> T>,
}

impl<T> Copy for TableRange<T> {}

impl<T> Clone for TableRange<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> TableRange<T> {
    pub(super) const fn new(start: usize, count: usize) -> Self {
        Self {
            start,
            count,
            marker: PhantomData,
        }
    }

    pub(super) const fn start(self) -> usize {
        self.start
    }
    pub(super) const fn len(self) -> usize {
        self.count
    }
    pub(super) const fn is_empty(self) -> bool {
        self.count == 0
    }

    pub(super) fn checked(self, length: usize) -> Option<Range<usize>> {
        let end = self.start.checked_add(self.count)?;
        (end <= length).then_some(self.start..end)
    }

    pub(super) fn get(self, table: &[T]) -> Option<&[T]> {
        table.get(self.checked(table.len())?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct StoredNodeIndex(pub(super) u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct StoredLogicalMeshIndex(pub(super) u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TextRange(pub(super) TableRange<u8>);

impl TextRange {
    pub(super) fn get(self, text: &[u8]) -> Option<&str> {
        std::str::from_utf8(self.0.get(text)?).ok()
    }
}

/// Raw typed authored components are rechecked by central validation before publication.
#[derive(Debug, Clone, Copy)]
pub(super) enum StoredTransform {
    Synthetic,
    Matrix(Mat4),
    Trs {
        translation: Vec3,
        rotation: Quat,
        scale: Vec3,
    },
}

impl StoredTransform {
    pub(super) fn from_authored(authored: AuthoredTransform) -> Option<Self> {
        if let Some(matrix) = authored.authored_matrix() {
            Some(Self::Matrix(matrix))
        } else {
            let trs = authored.authored_trs()?;
            Some(Self::Trs {
                translation: trs.translation(),
                rotation: trs.rotation(),
                scale: trs.scale(),
            })
        }
    }

    pub(super) fn authored(self) -> Option<AuthoredTransform> {
        match self {
            Self::Synthetic => None,
            Self::Matrix(matrix) => AuthoredTransform::try_matrix(matrix).ok(),
            Self::Trs {
                translation,
                rotation,
                scale,
            } => AuthoredTransform::try_trs(translation, rotation, scale).ok(),
        }
    }

    pub(super) const fn scalar_count(self) -> usize {
        match self {
            Self::Synthetic => 0,
            Self::Matrix(_) => 16,
            Self::Trs { .. } => 10,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StoredNode {
    pub(super) children: TableRange<StoredNodeIndex>,
    pub(super) transform: StoredTransform,
    pub(super) name: Option<TextRange>,
    pub(super) camera: Option<u32>,
    pub(super) light: Option<u32>,
}

/// Canonical row zero is the synthetic root; public node indices address rows one onward.
pub(super) struct NodeStorage {
    pub(super) transform_scalar_count: usize,
    pub(super) nodes: Vec<StoredNode>,
    pub(super) children: Vec<StoredNodeIndex>,
    pub(super) names: Vec<u8>,
}

impl NodeStorage {
    pub(super) fn source_node(&self, index: usize) -> Option<&StoredNode> {
        self.nodes.get(index.checked_add(1)?)
    }

    pub(super) fn roots(&self) -> Option<&[StoredNodeIndex]> {
        self.nodes.first()?.children.get(&self.children)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoredAttributeSemantic {
    Position,
    Normal,
    Tangent,
    Texcoord(u32),
    Color(u32),
    SmoothingGroup,
    Custom(TextRange),
}

impl StoredAttributeSemantic {
    pub(super) fn from_authored(value: &AttributeSemantic, custom: TextRange) -> Self {
        match value {
            AttributeSemantic::Position => Self::Position,
            AttributeSemantic::Normal => Self::Normal,
            AttributeSemantic::Tangent => Self::Tangent,
            AttributeSemantic::Texcoord(set) => Self::Texcoord(*set),
            AttributeSemantic::Color(set) => Self::Color(*set),
            AttributeSemantic::SmoothingGroup => Self::SmoothingGroup,
            AttributeSemantic::Custom(_) => Self::Custom(custom),
        }
    }
}

/// Scalar kind and dense/sparse shape cannot disagree with a separately stored tag.
#[derive(Debug, Clone, Copy)]
pub(super) enum StoredAttributePayload {
    DenseF32(TableRange<f32>),
    DenseF64(TableRange<f64>),
    DenseU32(TableRange<u32>),
    DenseI32(TableRange<i32>),
    SparseF32 {
        indices: TableRange<u32>,
        values: TableRange<f32>,
        default: TableRange<f32>,
    },
    SparseU32 {
        indices: TableRange<u32>,
        values: TableRange<u32>,
        default: TableRange<u32>,
    },
    SparseI32 {
        indices: TableRange<u32>,
        values: TableRange<i32>,
        default: TableRange<i32>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StoredAttribute {
    pub(super) semantic: StoredAttributeSemantic,
    pub(super) domain: AttributeDomain,
    pub(super) lanes: u8,
    pub(super) element_count: u32,
    pub(super) payload: StoredAttributePayload,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StoredPrimitive {
    pub(super) topology: TopologyKind,
    pub(super) vertex_count: u32,
    pub(super) corner_count: u32,
    pub(super) face_count: u32,
    pub(super) polygon_boundaries: Option<TableRange<u32>>,
    pub(super) indices: Option<TableRange<u32>>,
    pub(super) material: Option<u32>,
    pub(super) attributes: TableRange<StoredAttribute>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StoredLogicalMesh {
    pub(super) primitives: TableRange<StoredPrimitive>,
    pub(super) name: Option<TextRange>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StoredMeshInstance {
    pub(super) owner_node: StoredNodeIndex,
    pub(super) logical_mesh: StoredLogicalMeshIndex,
}

/// One immutable geometry owner; every scalar lane remains contiguous and directly borrowable.
pub(super) struct GeometryStorage {
    pub(super) logical_meshes: Vec<StoredLogicalMesh>,
    pub(super) primitives: Vec<StoredPrimitive>,
    pub(super) attributes: Vec<StoredAttribute>,
    pub(super) mesh_instances: Vec<StoredMeshInstance>,
    pub(super) names: Vec<u8>,
    pub(super) f32_values: Vec<f32>,
    pub(super) f64_values: Vec<f64>,
    pub(super) u32_values: Vec<u32>,
    pub(super) i32_values: Vec<i32>,
}

impl GeometryStorage {
    pub(in crate::canonical_scene::store) fn validation_work(&self) -> Option<u64> {
        [
            self.logical_meshes.len(),
            self.primitives.len(),
            self.primitives.len(),
            self.attributes.len(),
            self.mesh_instances.len(),
            self.f32_values.len(),
            self.f64_values.len(),
            self.u32_values.len(),
            self.i32_values.len(),
        ]
        .into_iter()
        .try_fold(0_u64, |sum, count| sum.checked_add(count as u64))
    }
}
