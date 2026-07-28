use crate::entity::AuthoredTransform;

pub(super) mod geometry;

/// Distinguishes an absent source field from a present value, including an empty slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::canonical_scene::store) enum Presence<T> {
    Absent,
    Present(T),
}

impl<T> Presence<T> {
    pub(in crate::canonical_scene::store) const fn as_ref(&self) -> Presence<&T> {
        match self {
            Self::Absent => Presence::Absent,
            Self::Present(value) => Presence::Present(value),
        }
    }
}

/// Identity of an authored node within one producer batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) struct NodeLocalId(u32);

impl NodeLocalId {
    pub(in crate::canonical_scene::store) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

impl From<u32> for NodeLocalId {
    fn from(index: u32) -> Self {
        Self::new(index)
    }
}

/// Ordered parent relation for one authored non-root node.
///
/// `child_ordinal` is the zero-based position in the parent's source child list.
/// Coupling it to the parent identity keeps ordered-child semantics local instead
/// of requiring a producer to choose Baozi's canonical traversal order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::canonical_scene::store) struct NodeParent {
    node: NodeLocalId,
    child_ordinal: u32,
}

impl NodeParent {
    pub(in crate::canonical_scene::store) const fn new(
        node: NodeLocalId,
        child_ordinal: u32,
    ) -> Self {
        Self {
            node,
            child_ordinal,
        }
    }

    pub(in crate::canonical_scene::store) const fn node(self) -> NodeLocalId {
        self.node
    }

    pub(in crate::canonical_scene::store) const fn child_ordinal(self) -> u32 {
        self.child_ordinal
    }
}

/// One authored node before its local parent is compiled to canonical order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeAuthoringNode {
    parent: Presence<NodeParent>,
    authored_transform: AuthoredTransform,
}

impl NodeAuthoringNode {
    pub(in crate::canonical_scene::store) const fn new(
        parent: Presence<NodeParent>,
        authored_transform: AuthoredTransform,
    ) -> Self {
        Self {
            parent,
            authored_transform,
        }
    }

    pub(in crate::canonical_scene::store) const fn parent(self) -> Presence<NodeParent> {
        self.parent
    }

    pub(in crate::canonical_scene::store) const fn authored_transform(self) -> AuthoredTransform {
        self.authored_transform
    }
}

/// Borrowed node-domain input supplied by a format producer.
///
/// `roots` is explicit because an absent source field and a present empty root list
/// carry different source semantics. Root slice order defines the synthetic root's
/// ordered children; each non-root node carries its source child ordinal. Local node
/// identifiers index `nodes`.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeAuthoringBatch<'a> {
    roots: Presence<&'a [NodeLocalId]>,
    nodes: &'a [NodeAuthoringNode],
}

/// One borrowed producer input for every domain of a canonical Scene.
///
/// The store accepts this aggregate as a unit so admission, validation, and
/// publication cannot observe a node graph without its geometry domain.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct SceneAuthoringBatch<'a> {
    nodes: NodeAuthoringBatch<'a>,
    geometry: geometry::GeometryAuthoringBatch<'a>,
}

impl<'a> SceneAuthoringBatch<'a> {
    pub(in crate::canonical_scene::store) const fn new(
        nodes: NodeAuthoringBatch<'a>,
        geometry: geometry::GeometryAuthoringBatch<'a>,
    ) -> Self {
        Self { nodes, geometry }
    }

    pub(in crate::canonical_scene::store) const fn nodes(self) -> NodeAuthoringBatch<'a> {
        self.nodes
    }

    pub(in crate::canonical_scene::store) const fn geometry(
        self,
    ) -> geometry::GeometryAuthoringBatch<'a> {
        self.geometry
    }
}

impl<'a> NodeAuthoringBatch<'a> {
    pub(in crate::canonical_scene::store) const fn new(
        roots: Presence<&'a [NodeLocalId]>,
        nodes: &'a [NodeAuthoringNode],
    ) -> Self {
        Self { roots, nodes }
    }

    pub(in crate::canonical_scene::store) const fn roots(self) -> Presence<&'a [NodeLocalId]> {
        self.roots
    }

    pub(in crate::canonical_scene::store) const fn nodes(self) -> &'a [NodeAuthoringNode] {
        self.nodes
    }
}
