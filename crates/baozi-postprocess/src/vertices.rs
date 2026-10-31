//! Borrowed draw-order application tuples over the single immutable Scene.

use std::ops::Range;

use baozi_core::{
    AttributeDomain, AttributeElement, AttributeRef, AttributeSemanticRef, InfluenceRowRef,
    MorphDeltaSemantic, MorphTargetId, PrimitiveRef, SceneLookupError, TopologyKind, Vec3,
};
use thiserror::Error;

use crate::{PrimitiveCorner, PrimitiveFace, PrimitiveVertex};

/// A source topology that has no triangle draw-order tuple interpretation.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum VertexLayoutError {
    #[error("application triangle vertices require TriangleList, found {topology:?}")]
    UnsupportedTopology { topology: TopologyKind },
}

/// Borrows one complete application vertex per draw corner, preserving triangle order and winding.
///
/// This reader allocates nothing, changes no Scene, and performs no sorting or vertex welding.
/// Every field uses the same output index: Vertex fields follow the source vertex, Corner fields
/// follow this corner, and Face fields follow its triangle. Thus UV seams and face boundaries do
/// not require the application to invent a shared-vertex mapping. Equal tuples may be repeated;
/// unreferenced source vertices stay in the Scene and do not appear in the draw layout.
///
/// Applications own buffer allocation, selected fields, index packing, and any precision reduction.
/// The implicit output index stream is `0..corner_count`; collecting it is optional. Skin palettes
/// and effective morph weights remain mesh-instance choices and are not baked into this view.
pub fn triangle_vertices(
    primitive: PrimitiveRef<'_>,
) -> Result<TriangleVertices<'_>, VertexLayoutError> {
    let topology = primitive.topology();
    if topology != TopologyKind::TriangleList {
        return Err(VertexLayoutError::UnsupportedTopology { topology });
    }
    Ok(TriangleVertices {
        primitive,
        indices: primitive.indices(),
        count: primitive.corner_count(),
    })
}

/// A zero-allocation layout borrowing one validated triangle primitive.
///
/// Constructing the layout and selecting a vertex take constant work. Reading every attribute
/// visits that primitive's actual descriptor sequence once per tuple; dense element reads are
/// constant time and sparse reads use binary search. No full-attribute lookup is repeated while
/// iterating the complete field sequence.
#[derive(Debug, Clone, Copy)]
pub struct TriangleVertices<'scene> {
    primitive: PrimitiveRef<'scene>,
    indices: Option<&'scene [u32]>,
    count: u32,
}

impl<'scene> TriangleVertices<'scene> {
    pub const fn primitive(self) -> PrimitiveRef<'scene> {
        self.primitive
    }

    /// The application vertex count equals the source draw-corner count.
    pub const fn len(self) -> usize {
        self.count as usize
    }

    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    /// Returns the implicit application index stream without allocating a buffer.
    pub fn indices(self) -> Range<u32> {
        0..self.count
    }

    /// Reads one application tuple; all returned slices borrow the Scene, not this layout.
    pub fn vertex(self, index: usize) -> Option<TriangleVertex<'scene>> {
        let corner = u32::try_from(index).ok()?;
        if corner >= self.count {
            return None;
        }
        let vertex = self.indices.map_or(corner, |indices| indices[index]);
        Some(TriangleVertex {
            primitive: self.primitive,
            corner,
            vertex,
        })
    }

    pub fn vertices(
        self,
    ) -> impl ExactSizeIterator<Item = TriangleVertex<'scene>> + DoubleEndedIterator + Clone + 'scene
    {
        (0..self.len()).map(move |index| self.vertex(index).expect("validated draw corner exists"))
    }
}

/// One complete tuple in draw order, with its exact source primitive and domains.
#[derive(Debug, Clone, Copy)]
pub struct TriangleVertex<'scene> {
    primitive: PrimitiveRef<'scene>,
    corner: u32,
    vertex: u32,
}

impl<'scene> TriangleVertex<'scene> {
    pub const fn primitive(self) -> PrimitiveRef<'scene> {
        self.primitive
    }

    /// This tuple's application index, equal to its source corner ordinal.
    pub const fn index(self) -> u32 {
        self.corner
    }

    pub fn source_vertex(self) -> PrimitiveVertex {
        PrimitiveVertex {
            primitive: self.primitive.id(),
            index: self.vertex,
        }
    }

    pub fn source_corner(self) -> PrimitiveCorner {
        PrimitiveCorner {
            primitive: self.primitive.id(),
            index: self.corner,
        }
    }

    pub fn source_face(self) -> PrimitiveFace {
        PrimitiveFace {
            primitive: self.primitive.id(),
            index: self.corner / 3,
        }
    }

    /// Reads all fields in retained order, keeping their complete semantic/source-domain keys.
    pub fn attributes(
        self,
    ) -> impl ExactSizeIterator<Item = VertexAttribute<'scene>> + DoubleEndedIterator + Clone + 'scene
    {
        self.primitive
            .attributes()
            .map(move |attribute| self.read_attribute(attribute))
    }

    /// Looks up one exact key in O(attribute count), preserving simultaneous source domains.
    pub fn attribute(
        self,
        semantic: AttributeSemanticRef<'_>,
        domain: AttributeDomain,
    ) -> Option<VertexAttribute<'scene>> {
        self.primitive
            .attribute(semantic, domain)
            .map(|attribute| self.read_attribute(attribute))
    }

    /// Retains absence, empty rows, every influence, and authored ordering without truncation.
    /// Palette-local joint indices are resolved through the mesh instance's skin.
    pub fn influences(self) -> Option<InfluenceRowRef<'scene>> {
        self.primitive.influences().map(|rows| {
            rows.get(self.vertex as usize)
                .expect("validated source vertex has an influence row")
        })
    }

    /// Reads this primitive's relative delta at the same source vertex as its base attributes.
    /// A missing stream is `None`; an implicit sparse zero in a present stream is `Some(ZERO)`.
    /// Generation and logical target ownership are checked before looking up the stream.
    pub fn morph_delta(
        self,
        target: MorphTargetId,
        semantic: MorphDeltaSemantic,
    ) -> Result<Option<Vec3>, SceneLookupError> {
        Ok(self
            .primitive
            .morph_target(target)?
            .delta(semantic)
            .map(|delta| {
                delta
                    .data()
                    .value(self.vertex as usize)
                    .expect("validated source vertex is in the morph domain")
            }))
    }

    fn read_attribute(self, attribute: AttributeRef<'scene>) -> VertexAttribute<'scene> {
        let index = match attribute.domain() {
            AttributeDomain::Vertex => self.vertex,
            AttributeDomain::Corner => self.corner,
            AttributeDomain::Face => self.corner / 3,
            AttributeDomain::Instance => {
                unreachable!("validated primitive attributes exclude the Instance domain")
            }
        };
        VertexAttribute {
            attribute,
            element: attribute
                .element(index as usize)
                .expect("validated attribute covers its source domain"),
        }
    }
}

/// A tuple field with its original descriptor and a typed element borrowing actual Scene storage.
/// The descriptor retains the source domain and dense/sparse representation.
#[derive(Debug, Clone, Copy)]
pub struct VertexAttribute<'scene> {
    attribute: AttributeRef<'scene>,
    element: AttributeElement<'scene>,
}

impl<'scene> VertexAttribute<'scene> {
    pub const fn attribute(self) -> AttributeRef<'scene> {
        self.attribute
    }

    pub const fn element(self) -> AttributeElement<'scene> {
        self.element
    }
}
