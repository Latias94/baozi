//! Borrowed draw-order application tuples over the single immutable Scene.

use std::ops::Range;

use baozi_core::{
    AttributeDomain, AttributeElement, AttributeRef, AttributeSemanticRef, InfluenceRowRef,
    MorphDeltaSemantic, MorphTargetId, PrimitiveRef, SceneLookupError, TopologyKind, Vec3,
};
use thiserror::Error;

use crate::{PrimitiveCorner, PrimitiveFace, PrimitiveVertex};

/// Why a primitive cannot be exposed as the requested application draw tuples.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum VertexLayoutError {
    #[error("the requested application vertex layout does not support {topology:?} topology")]
    UnsupportedTopology { topology: TopologyKind },
    #[error("the application draw-corner count exceeds u32")]
    SizeOverflow,
}

/// Borrows one complete application vertex per list draw corner.
///
/// This reader allocates nothing, changes no Scene, and performs no sorting or vertex welding.
/// Points, LineList and TriangleList retain their source Corner order. LineStrip and LineLoop
/// expand to LineList; TriangleStrip and TriangleFan expand to TriangleList. Polygons require
/// explicit triangulation before this reader can consume them.
///
/// For source Face `f`, the exact source Corner order is `[f, f + 1]` for LineStrip and
/// `[f, (f + 1) % n]` for LineLoop with `n` source Corners. TriangleStrip uses
/// `[f, f + 1, f + 2]` for even `f` and `[f, f + 2, f + 1]` for odd `f`.
/// TriangleFan uses `[f + 1, f + 2, 0]`. Each output Face follows source Face `f`.
/// Repeated indices and degenerate elements are retained, including their strip parity and the
/// closing LineLoop edge. Strips, fans and loops shorter than one complete element yield no tuples.
///
/// Vertex fields follow the source Vertex, Corner fields follow the source Corner, and Face fields
/// follow the source Face. Thus UV seams and face boundaries do not require the application to
/// invent a shared-vertex mapping. Equal tuples may be repeated; unreferenced source vertices stay
/// in the Scene and do not appear in the draw layout.
///
/// Applications own buffer allocation, selected fields, index packing, and any precision reduction.
/// The implicit output index stream is `0..output_corner_count`; collecting it is optional. Its
/// count must fit in u32. Applications also own interruption of their extraction loop; this view
/// starts no operation or resource ledger. Skin palettes and effective morph weights remain
/// mesh-instance choices and are not baked into this view.
pub fn list_vertices(primitive: PrimitiveRef<'_>) -> Result<ListVertices<'_>, VertexLayoutError> {
    let assembly = ListAssembly::from_topology(primitive.topology())?;
    let source_corners = primitive.corner_count();
    let count = assembly.output_corner_count(source_corners)?;
    Ok(ListVertices {
        primitive,
        indices: primitive.indices(),
        assembly,
        source_corners,
        count,
    })
}

/// Borrows TriangleList draw tuples through the same sampler as [`list_vertices`].
///
/// Only TriangleList is accepted, including an empty list. Use [`list_vertices`] to explicitly
/// expand strips, fans or loops or consume points and lines. Both readers preserve source fields
/// and borrow the original Scene without allocating.
pub fn triangle_vertices(
    primitive: PrimitiveRef<'_>,
) -> Result<ListVertices<'_>, VertexLayoutError> {
    let topology = primitive.topology();
    if topology != TopologyKind::TriangleList {
        return Err(VertexLayoutError::UnsupportedTopology { topology });
    }
    list_vertices(primitive)
}

#[derive(Debug, Clone, Copy)]
enum ListAssembly {
    Points,
    LineList,
    LineStrip,
    LineLoop,
    TriangleList,
    TriangleStrip,
    TriangleFan,
}

impl ListAssembly {
    fn from_topology(topology: TopologyKind) -> Result<Self, VertexLayoutError> {
        match topology {
            TopologyKind::Points => Ok(Self::Points),
            TopologyKind::LineList => Ok(Self::LineList),
            TopologyKind::LineStrip => Ok(Self::LineStrip),
            TopologyKind::LineLoop => Ok(Self::LineLoop),
            TopologyKind::TriangleList => Ok(Self::TriangleList),
            TopologyKind::TriangleStrip => Ok(Self::TriangleStrip),
            TopologyKind::TriangleFan => Ok(Self::TriangleFan),
            TopologyKind::Polygons => Err(VertexLayoutError::UnsupportedTopology { topology }),
        }
    }

    const fn topology(self) -> TopologyKind {
        match self {
            Self::Points => TopologyKind::Points,
            Self::LineList | Self::LineStrip | Self::LineLoop => TopologyKind::LineList,
            Self::TriangleList | Self::TriangleStrip | Self::TriangleFan => {
                TopologyKind::TriangleList
            }
        }
    }

    fn output_corner_count(self, corners: u32) -> Result<u32, VertexLayoutError> {
        let count = match self {
            Self::Points | Self::LineList | Self::TriangleList => return Ok(corners),
            Self::LineStrip => corners.saturating_sub(1).checked_mul(2),
            Self::LineLoop => {
                if corners < 2 {
                    Some(0)
                } else {
                    corners.checked_mul(2)
                }
            }
            Self::TriangleStrip | Self::TriangleFan => corners.saturating_sub(2).checked_mul(3),
        };
        count.ok_or(VertexLayoutError::SizeOverflow)
    }

    /// Called only for an index below the checked output Corner count.
    fn source_corner_and_face(self, index: u32, corners: u32) -> (u32, u32) {
        match self {
            Self::Points => (index, index),
            Self::LineList => (index, index / 2),
            Self::TriangleList => (index, index / 3),
            Self::LineStrip => {
                let face = index / 2;
                (face + index % 2, face)
            }
            Self::LineLoop => {
                let face = index / 2;
                ((face + index % 2) % corners, face)
            }
            Self::TriangleStrip => {
                let face = index / 3;
                let triangle = if face.is_multiple_of(2) {
                    [face, face + 1, face + 2]
                } else {
                    [face, face + 2, face + 1]
                };
                (triangle[(index % 3) as usize], face)
            }
            Self::TriangleFan => {
                let face = index / 3;
                ([face + 1, face + 2, 0][(index % 3) as usize], face)
            }
        }
    }
}

/// A zero-allocation list layout borrowing one validated primitive.
///
/// Constructing the layout and selecting a vertex take constant work. Reading every attribute
/// visits that primitive's actual descriptor sequence once per tuple; dense element reads are
/// constant time and sparse reads use binary search. No full-attribute lookup is repeated while
/// iterating the complete field sequence. Bind selected fields with [`Self::attribute`] or
/// [`Self::attributes`] to resolve descriptors once for a complete draw stream.
#[derive(Debug, Clone, Copy)]
pub struct ListVertices<'scene> {
    primitive: PrimitiveRef<'scene>,
    indices: Option<&'scene [u32]>,
    assembly: ListAssembly,
    source_corners: u32,
    count: u32,
}

impl<'scene> ListVertices<'scene> {
    /// The original primitive, retaining its source topology and fields.
    pub const fn primitive(self) -> PrimitiveRef<'scene> {
        self.primitive
    }

    /// The draw topology of this view: Points, LineList or TriangleList.
    pub const fn topology(self) -> TopologyKind {
        self.assembly.topology()
    }

    /// The application tuple count, including every expanded draw Corner.
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
    pub fn vertex(self, index: usize) -> Option<ListVertex<'scene>> {
        let index = u32::try_from(index).ok()?;
        if index >= self.count {
            return None;
        }
        let (corner, face) = self
            .assembly
            .source_corner_and_face(index, self.source_corners);
        let vertex = match self.indices {
            Some(indices) => *indices.get(corner as usize)?,
            None => corner,
        };
        Some(ListVertex {
            primitive: self.primitive,
            index,
            corner,
            vertex,
            face,
        })
    }

    pub fn vertices(
        self,
    ) -> impl ExactSizeIterator<Item = ListVertex<'scene>> + DoubleEndedIterator + Clone + 'scene
    {
        (0..self.len())
            .map(move |index| self.vertex(index).expect("checked list draw corner exists"))
    }

    /// Resolves one exact semantic/domain key once, without reading its scalar elements.
    ///
    /// Missing fields are `None`; a present field on an empty draw layout is still `Some`.
    /// Vertex, Corner and Face streams retain their distinct authored domains. Primitive
    /// attributes never have the Instance domain.
    #[inline]
    pub fn attribute(
        self,
        semantic: AttributeSemanticRef<'_>,
        domain: AttributeDomain,
    ) -> Option<ListAttribute<'scene>> {
        self.primitive
            .attribute(semantic, domain)
            .map(|attribute| ListAttribute::new(self, attribute))
    }

    /// Binds fields lazily in their retained descriptor order, without visiting scalar data.
    ///
    /// Each yielded stream owns its small borrowed view and may outlive this layout and iterator.
    pub fn attributes(
        self,
    ) -> impl ExactSizeIterator<Item = ListAttribute<'scene>> + DoubleEndedIterator + Clone + 'scene
    {
        self.primitive
            .attributes()
            .map(move |attribute| ListAttribute::new(self, attribute))
    }
}

/// One attribute resolved once against its owning list layout.
///
/// Sampling follows the same source coordinates as [`ListVertices::vertex`]. Vertex fields use
/// the source index stream; Corner fields retain seams and repeated strip/fan/loop occurrences;
/// Face fields follow the source Face. Dense reads are constant time and sparse reads use binary
/// search, preserving the exact explicit or default slice. No cache, projection, normalization,
/// scalar conversion or allocation is performed.
///
/// The descriptor and iterator borrow the Scene, not a temporary layout or stream wrapper:
///
/// ```
/// use baozi_core::{AttributeDomain, AttributeElement, AttributeSemanticRef, PrimitiveRef};
/// use baozi_postprocess::list_vertices;
///
/// fn positions<'scene>(primitive: PrimitiveRef<'scene>)
///     -> impl ExactSizeIterator<Item = AttributeElement<'scene>> + 'scene
/// {
///     list_vertices(primitive).unwrap()
///         .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex).unwrap()
///         .elements()
/// }
/// ```
///
/// A stream cannot escape its owning Scene:
///
/// ```compile_fail,E0515
/// use baozi_core::{AttributeDomain, AttributeSemanticRef, ValidatedScene};
/// use baozi_postprocess::{ListAttribute, list_vertices};
///
/// fn escape(scene: ValidatedScene) -> ListAttribute<'static> {
///     let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
///     list_vertices(primitive).unwrap()
///         .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex).unwrap()
/// }
/// ```
///
/// Borrowed elements cannot escape it either:
///
/// ```compile_fail,E0515
/// use baozi_core::{AttributeDomain, AttributeElement, AttributeSemanticRef, ValidatedScene};
/// use baozi_postprocess::list_vertices;
///
/// fn escape(scene: ValidatedScene) -> AttributeElement<'static> {
///     let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
///     list_vertices(primitive).unwrap()
///         .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex).unwrap()
///         .element(0).unwrap()
/// }
/// ```
///
/// An owning iterator also remains tied to the Scene:
///
/// ```compile_fail,E0515
/// use baozi_core::{AttributeDomain, AttributeElement, AttributeSemanticRef, ValidatedScene};
/// use baozi_postprocess::list_vertices;
///
/// fn escape(scene: ValidatedScene)
///     -> impl Iterator<Item = AttributeElement<'static>>
/// {
///     let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
///     list_vertices(primitive).unwrap()
///         .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex).unwrap()
///         .elements()
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ListAttribute<'scene> {
    layout: ListVertices<'scene>,
    attribute: AttributeRef<'scene>,
}

impl<'scene> ListAttribute<'scene> {
    fn new(layout: ListVertices<'scene>, attribute: AttributeRef<'scene>) -> Self {
        Self { layout, attribute }
    }

    /// The original descriptor, including scalar representation, declarations and source owner.
    pub const fn attribute(&self) -> AttributeRef<'scene> {
        self.attribute
    }

    /// The original layout, retaining the primitive and its generation-qualified source endpoints.
    pub const fn layout(&self) -> ListVertices<'scene> {
        self.layout
    }

    /// Number of draw occurrences, which may differ from the descriptor's logical element count.
    pub const fn len(&self) -> usize {
        self.layout.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.layout.is_empty()
    }

    /// Borrows one typed element at a draw ordinal without copying the full descriptor.
    ///
    /// Out-of-range ordinals, including `usize::MAX`, return `None` even for default-only sparse
    /// fields. The returned slice remains valid after this view is dropped, while the Scene lives.
    #[inline]
    pub fn element(&self, index: usize) -> Option<AttributeElement<'scene>> {
        let vertex = self.layout.vertex(index)?;
        let source = vertex.source_index(self.attribute.domain())?;
        self.attribute.element(source as usize)
    }

    /// Iterates typed elements in draw order, owning one copy of the bound view.
    ///
    /// Clones have independent traversal state; backward and random access do not change sparse
    /// lookup state. Dropping an iterator early has no deferred work or storage to reconcile.
    pub fn elements(
        self,
    ) -> impl ExactSizeIterator<Item = AttributeElement<'scene>> + DoubleEndedIterator + Clone + 'scene
    {
        (0..self.len()).map(move |index| {
            self.element(index)
                .expect("validated list attribute element exists")
        })
    }
}

/// One complete tuple in draw order, with its exact source primitive and domains.
///
/// Its application index may differ from its source Corner. Every source endpoint identifies the
/// original primitive and Scene generation; this borrowed tuple creates no retained remap.
#[derive(Debug, Clone, Copy)]
pub struct ListVertex<'scene> {
    primitive: PrimitiveRef<'scene>,
    index: u32,
    corner: u32,
    vertex: u32,
    face: u32,
}

impl<'scene> ListVertex<'scene> {
    pub const fn primitive(self) -> PrimitiveRef<'scene> {
        self.primitive
    }

    /// This tuple's application index in the expanded list draw order.
    pub const fn index(self) -> u32 {
        self.index
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
            index: self.face,
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

    fn source_index(&self, domain: AttributeDomain) -> Option<u32> {
        match domain {
            AttributeDomain::Vertex => Some(self.vertex),
            AttributeDomain::Corner => Some(self.corner),
            AttributeDomain::Face => Some(self.face),
            AttributeDomain::Instance => None,
        }
    }

    fn read_attribute(self, attribute: AttributeRef<'scene>) -> VertexAttribute<'scene> {
        let source = self
            .source_index(attribute.domain())
            .expect("validated primitive attributes exclude the Instance domain");
        VertexAttribute {
            attribute,
            element: attribute
                .element(source as usize)
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

#[cfg(test)]
mod tests {
    use super::{ListAssembly, VertexLayoutError};

    #[test]
    fn short_assemblies_have_no_output_corners() {
        for (assembly, minimum) in [
            (ListAssembly::LineStrip, 2),
            (ListAssembly::LineLoop, 2),
            (ListAssembly::TriangleStrip, 3),
            (ListAssembly::TriangleFan, 3),
        ] {
            for corners in 0..minimum {
                assert_eq!(assembly.output_corner_count(corners), Ok(0));
            }
        }
    }

    #[test]
    fn expanded_corner_counts_reject_the_first_u32_overflow() {
        let lines = u32::MAX / 2;
        let triangles = u32::MAX / 3;
        for (assembly, last_source_count, last_output_count) in [
            (ListAssembly::LineStrip, lines + 1, lines * 2),
            (ListAssembly::LineLoop, lines, lines * 2),
            (ListAssembly::TriangleStrip, triangles + 2, triangles * 3),
            (ListAssembly::TriangleFan, triangles + 2, triangles * 3),
        ] {
            assert_eq!(
                assembly.output_corner_count(last_source_count),
                Ok(last_output_count),
            );
            assert_eq!(
                assembly.output_corner_count(last_source_count + 1),
                Err(VertexLayoutError::SizeOverflow),
            );
        }
    }

    #[test]
    fn native_list_corner_counts_do_not_expand() {
        for (assembly, corners) in [
            (ListAssembly::Points, u32::MAX),
            (ListAssembly::LineList, u32::MAX - 1),
            (ListAssembly::TriangleList, u32::MAX),
        ] {
            assert_eq!(assembly.output_corner_count(corners), Ok(corners));
        }
    }
}
