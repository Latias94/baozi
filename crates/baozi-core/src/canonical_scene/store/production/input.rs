use crate::entity::AuthoredTransform;

pub(in crate::canonical_scene::store) mod animation;
pub(in crate::canonical_scene::store) mod appearance;
pub(in crate::canonical_scene::store) mod deformation;
pub(in crate::canonical_scene::store) mod geometry;
pub(in crate::canonical_scene::store) mod lineage;
pub(in crate::canonical_scene::store) mod metadata;
pub(in crate::canonical_scene::store) mod spatial;

/// Distinguishes an absent source field from a present value, including an empty slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Presence<T> {
    Absent,
    Present(T),
}

impl<T> Presence<T> {
    pub const fn as_ref(&self) -> Presence<&T> {
        match self {
            Self::Absent => Presence::Absent,
            Self::Present(value) => Presence::Present(value),
        }
    }
}

/// Identity of an authored node within one producer batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeLocalId(u32);

impl NodeLocalId {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
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
pub struct NodeParent {
    node: NodeLocalId,
    child_ordinal: u32,
}

impl NodeParent {
    pub const fn new(node: NodeLocalId, child_ordinal: u32) -> Self {
        Self {
            node,
            child_ordinal,
        }
    }

    pub const fn node(self) -> NodeLocalId {
        self.node
    }

    pub const fn child_ordinal(self) -> u32 {
        self.child_ordinal
    }
}

/// One authored node before its local parent is compiled to canonical order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeAuthoringNode<'a> {
    name: Presence<&'a str>,
    parent: Presence<NodeParent>,
    authored_transform: AuthoredTransform,
    camera: Presence<spatial::CameraLocalId>,
    light: Presence<spatial::LightLocalId>,
}

impl<'a> NodeAuthoringNode<'a> {
    pub const fn new(
        parent: Presence<NodeParent>,
        authored_transform: AuthoredTransform,
        camera: Presence<spatial::CameraLocalId>,
        light: Presence<spatial::LightLocalId>,
    ) -> Self {
        Self {
            name: Presence::Absent,
            parent,
            authored_transform,
            camera,
            light,
        }
    }

    pub const fn with_name(mut self, name: &'a str) -> Self {
        self.name = Presence::Present(name);
        self
    }

    pub const fn name(self) -> Presence<&'a str> {
        self.name
    }

    pub const fn parent(self) -> Presence<NodeParent> {
        self.parent
    }

    pub const fn authored_transform(self) -> AuthoredTransform {
        self.authored_transform
    }

    pub const fn camera(self) -> Presence<spatial::CameraLocalId> {
        self.camera
    }

    pub const fn light(self) -> Presence<spatial::LightLocalId> {
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
pub struct NodeAuthoringBatch<'a> {
    roots: Presence<&'a [NodeLocalId]>,
    nodes: &'a [NodeAuthoringNode<'a>],
}

/// Source identity, lineage, descriptive metadata, and transient resource snapshots for one
/// canonical Scene batch.
///
/// Metadata may be empty, but the lineage batch must still cover the Scene, synthetic root, and
/// every canonical owner admitted by the other authoring domains. Resource snapshots are
/// authoring-only input: canonical publication retains owned payload bytes and profile lineage,
/// not this transient table.
#[derive(Debug, Clone, Copy)]
pub struct ProfileAuthoringBatch<'a> {
    lineage: lineage::LineageAuthoringBatch<'a>,
    metadata: metadata::MetadataAuthoringBatch<'a>,
    resource_snapshots: &'a [ResourceSnapshotAuthoring<'a>],
}

impl<'a> ProfileAuthoringBatch<'a> {
    pub const fn new(
        lineage: lineage::LineageAuthoringBatch<'a>,
        metadata: metadata::MetadataAuthoringBatch<'a>,
    ) -> Self {
        Self {
            lineage,
            metadata,
            resource_snapshots: &[],
        }
    }

    /// Injects transient bytes captured by the owning import session.
    ///
    /// Rows are positionally aligned with the profile resource table. They are consumed only to
    /// resolve same-session image references during materialization and never enter the retained
    /// profile or expose resource locators as canonical appearance semantics.
    pub const fn with_resource_snapshots(
        mut self,
        resource_snapshots: &'a [ResourceSnapshotAuthoring<'a>],
    ) -> Self {
        self.resource_snapshots = resource_snapshots;
        self
    }

    pub(in crate::canonical_scene::store) const fn lineage(
        self,
    ) -> lineage::LineageAuthoringBatch<'a> {
        self.lineage
    }

    pub(in crate::canonical_scene::store) const fn metadata(
        self,
    ) -> metadata::MetadataAuthoringBatch<'a> {
        self.metadata
    }

    pub(in crate::canonical_scene::store) const fn resource_snapshots(
        self,
    ) -> ResourceSnapshotAuthoringTable<'a> {
        ResourceSnapshotAuthoringTable::new(self.lineage.resources().len(), self.resource_snapshots)
    }
}

/// One transient session-owned byte snapshot aligned with a profile resource identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceSnapshotAuthoring<'a> {
    bytes: Presence<&'a [u8]>,
}

impl<'a> ResourceSnapshotAuthoring<'a> {
    pub const fn absent() -> Self {
        Self {
            bytes: Presence::Absent,
        }
    }

    pub const fn snapshotted(bytes: &'a [u8]) -> Self {
        Self {
            bytes: Presence::Present(bytes),
        }
    }

    pub(in crate::canonical_scene::store) const fn bytes(self) -> Presence<&'a [u8]> {
        self.bytes
    }
}

/// Narrow appearance-facing view of the session resource snapshot table.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct ResourceSnapshotAuthoringTable<'a> {
    resource_count: usize,
    snapshots: &'a [ResourceSnapshotAuthoring<'a>],
}

impl<'a> ResourceSnapshotAuthoringTable<'a> {
    pub(in crate::canonical_scene::store) const fn new(
        resource_count: usize,
        snapshots: &'a [ResourceSnapshotAuthoring<'a>],
    ) -> Self {
        Self {
            resource_count,
            snapshots,
        }
    }

    pub(in crate::canonical_scene::store) const fn empty() -> Self {
        Self::new(0, &[])
    }

    pub(in crate::canonical_scene::store) const fn resource_count(self) -> usize {
        self.resource_count
    }

    pub(in crate::canonical_scene::store) const fn snapshots(
        self,
    ) -> &'a [ResourceSnapshotAuthoring<'a>] {
        self.snapshots
    }
}

/// One borrowed producer input for every domain of a canonical Scene.
///
/// The store accepts this aggregate as a unit so admission, validation, and
/// publication cannot observe a node graph without its spatial foundation or
/// the appearance, geometry, deformation, and animation domains that qualify
/// the same Scene, or without the profile that accounts for every canonical
/// owner's source identity, lineage, and metadata.
#[derive(Debug, Clone, Copy)]
pub struct SceneAuthoringBatch<'a> {
    spatial: spatial::SpatialAuthoringBatch<'a>,
    nodes: NodeAuthoringBatch<'a>,
    appearance: appearance::AppearanceAuthoringBatch<'a>,
    geometry: geometry::GeometryAuthoringBatch<'a>,
    deformation: deformation::DeformationAuthoringBatch<'a>,
    animations: animation::AnimationAuthoringBatch<'a>,
    profile: ProfileAuthoringBatch<'a>,
}

impl<'a> SceneAuthoringBatch<'a> {
    pub const fn new(
        spatial: spatial::SpatialAuthoringBatch<'a>,
        nodes: NodeAuthoringBatch<'a>,
        appearance: appearance::AppearanceAuthoringBatch<'a>,
        geometry: geometry::GeometryAuthoringBatch<'a>,
        deformation: deformation::DeformationAuthoringBatch<'a>,
        animations: animation::AnimationAuthoringBatch<'a>,
        profile: ProfileAuthoringBatch<'a>,
    ) -> Self {
        Self {
            spatial,
            nodes,
            appearance,
            geometry,
            deformation,
            animations,
            profile,
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

    pub(in crate::canonical_scene::store) const fn profile(self) -> ProfileAuthoringBatch<'a> {
        self.profile
    }
}

impl<'a> NodeAuthoringBatch<'a> {
    pub const fn new(
        roots: Presence<&'a [NodeLocalId]>,
        nodes: &'a [NodeAuthoringNode<'a>],
    ) -> Self {
        Self { roots, nodes }
    }

    pub(in crate::canonical_scene::store) const fn roots(self) -> Presence<&'a [NodeLocalId]> {
        self.roots
    }

    pub(in crate::canonical_scene::store) const fn nodes(self) -> &'a [NodeAuthoringNode<'a>] {
        self.nodes
    }
}
