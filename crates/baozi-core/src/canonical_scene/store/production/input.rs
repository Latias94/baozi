use crate::entity::AuthoredTransform;

pub(super) mod animation;
pub(super) mod appearance;
pub(super) mod deformation;
pub(super) mod geometry;
pub(super) mod spatial;

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
    camera: Presence<spatial::CameraLocalId>,
    light: Presence<spatial::LightLocalId>,
}

impl NodeAuthoringNode {
    pub(in crate::canonical_scene::store) const fn new(
        parent: Presence<NodeParent>,
        authored_transform: AuthoredTransform,
        camera: Presence<spatial::CameraLocalId>,
        light: Presence<spatial::LightLocalId>,
    ) -> Self {
        Self {
            parent,
            authored_transform,
            camera,
            light,
        }
    }

    pub(in crate::canonical_scene::store) const fn parent(self) -> Presence<NodeParent> {
        self.parent
    }

    pub(in crate::canonical_scene::store) const fn authored_transform(self) -> AuthoredTransform {
        self.authored_transform
    }

    pub(in crate::canonical_scene::store) const fn camera(
        self,
    ) -> Presence<spatial::CameraLocalId> {
        self.camera
    }

    pub(in crate::canonical_scene::store) const fn light(self) -> Presence<spatial::LightLocalId> {
        self.light
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
/// publication cannot observe a node graph without its spatial foundation or
/// the appearance, geometry, deformation, and animation domains that qualify
/// the same Scene.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct SceneAuthoringBatch<'a> {
    spatial: spatial::SpatialAuthoringBatch<'a>,
    nodes: NodeAuthoringBatch<'a>,
    appearance: appearance::AppearanceAuthoringBatch<'a>,
    geometry: geometry::GeometryAuthoringBatch<'a>,
    deformation: deformation::DeformationAuthoringBatch<'a>,
    animations: animation::AnimationAuthoringBatch<'a>,
}

impl<'a> SceneAuthoringBatch<'a> {
    pub(in crate::canonical_scene::store) const fn new(
        spatial: spatial::SpatialAuthoringBatch<'a>,
        nodes: NodeAuthoringBatch<'a>,
        appearance: appearance::AppearanceAuthoringBatch<'a>,
        geometry: geometry::GeometryAuthoringBatch<'a>,
        deformation: deformation::DeformationAuthoringBatch<'a>,
        animations: animation::AnimationAuthoringBatch<'a>,
    ) -> Self {
        Self {
            spatial,
            nodes,
            appearance,
            geometry,
            deformation,
            animations,
        }
    }

    pub(in crate::canonical_scene::store) const fn spatial(
        self,
    ) -> spatial::SpatialAuthoringBatch<'a> {
        self.spatial
    }

    pub(in crate::canonical_scene::store) const fn nodes(self) -> NodeAuthoringBatch<'a> {
        self.nodes
    }

    pub(in crate::canonical_scene::store) const fn appearance(
        self,
    ) -> appearance::AppearanceAuthoringBatch<'a> {
        self.appearance
    }

    pub(in crate::canonical_scene::store) const fn geometry(
        self,
    ) -> geometry::GeometryAuthoringBatch<'a> {
        self.geometry
    }

    pub(in crate::canonical_scene::store) const fn deformation(
        self,
    ) -> deformation::DeformationAuthoringBatch<'a> {
        self.deformation
    }

    pub(in crate::canonical_scene::store) const fn animations(
        self,
    ) -> animation::AnimationAuthoringBatch<'a> {
        self.animations
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
