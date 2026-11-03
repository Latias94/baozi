use std::fmt;
use std::iter::FusedIterator;

use super::super::super::construction::node_scene::semantic::{
    NodeSceneChildCursor, NodeSceneNodeView, NodeSceneSemanticNode,
};
use super::{ValidatedScene, checked_index, expect_validated};
use crate::entity::AuthoredTransform;
use crate::identity::{NodeId, SceneEntityKind, SceneLookupError};

/// One source node borrowed from an immutable Scene. The synthetic root is hidden.
#[derive(Clone, Copy)]
pub struct NodeRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> NodeRef<'s> {
    pub fn metadata_owner(self) -> super::EntityMetadataOwner {
        super::EntityMetadataOwner::new(
            self.scene,
            crate::canonical_scene::store::production::CanonicalProfileOwner::Node(
                self.index as u32 + 1,
            ),
        )
    }
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    fn read(self) -> NodeSceneSemanticNode<'s> {
        expect_validated(
            expect_validated(self.scene.as_node_scene().semantic_view()).node_at(self.index),
        )
    }

    pub fn id(self) -> NodeId {
        NodeId::new(self.scene.generation(), self.index as u32)
    }

    pub fn name(self) -> Option<&'s str> {
        self.read().name()
    }

    /// Preserves the authored matrix or TRS representation.
    pub fn authored_transform(self) -> AuthoredTransform {
        self.read().authored_transform()
    }

    pub fn children(self) -> NodeChildren<'s> {
        NodeChildren::new(self.scene, self.read().into_children())
    }

    pub fn camera(self) -> Option<super::CameraRef<'s>> {
        self.read()
            .camera()
            .map(|index| super::CameraRef::at(self.scene, index))
    }

    pub fn light(self) -> Option<super::LightRef<'s>> {
        self.read()
            .light()
            .map(|index| super::LightRef::at(self.scene, index))
    }
}

impl fmt::Debug for NodeRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

/// An independent iterator over an ordered root or child range.
pub struct NodeChildren<'s> {
    scene: &'s ValidatedScene,
    cursor: NodeSceneChildCursor<'s>,
    remaining: usize,
}

impl<'s> NodeChildren<'s> {
    fn new(scene: &'s ValidatedScene, cursor: NodeSceneChildCursor<'s>) -> Self {
        Self {
            scene,
            remaining: cursor.declared_count(),
            cursor,
        }
    }
}

impl<'s> Iterator for NodeChildren<'s> {
    type Item = NodeRef<'s>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = expect_validated(self.cursor.next())?;
        self.remaining -= 1;
        Some(NodeRef::at(self.scene, index))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for NodeChildren<'_> {}
impl FusedIterator for NodeChildren<'_> {}

/// Reuses one complete semantic binding for an immutable node query.
pub(crate) struct NodeQuery<'s> {
    scene: &'s ValidatedScene,
    view: NodeSceneNodeView<'s>,
}

impl<'s> NodeQuery<'s> {
    fn read(&self, node: NodeRef<'s>) -> NodeSceneSemanticNode<'s> {
        assert!(
            std::ptr::eq(self.scene, node.scene),
            "node query received a node from another Scene"
        );
        expect_validated(self.view.node_at(node.index))
    }

    pub(crate) fn roots(&self) -> NodeChildren<'s> {
        NodeChildren::new(self.scene, expect_validated(self.view.roots()))
    }

    pub(crate) fn authored_transform(&self, node: NodeRef<'s>) -> AuthoredTransform {
        self.read(node).authored_transform()
    }

    pub(crate) fn children(&self, node: NodeRef<'s>) -> NodeChildren<'s> {
        NodeChildren::new(self.scene, self.read(node).into_children())
    }
}

impl ValidatedScene {
    pub(crate) fn bind_node_query(&self) -> NodeQuery<'_> {
        NodeQuery {
            scene: self,
            view: expect_validated(self.as_node_scene().semantic_view()).node_view(),
        }
    }

    /// Returns the source node count, excluding the internal synthetic root.
    pub fn node_count(&self) -> usize {
        self.as_node_scene().read_proof().source_node_count()
    }

    pub fn root_count(&self) -> usize {
        self.as_node_scene().read_proof().source_root_count()
    }

    pub fn nodes(
        &self,
    ) -> impl ExactSizeIterator<Item = NodeRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.node_count()).map(|index| NodeRef::at(self, index))
    }

    pub fn roots(&self) -> NodeChildren<'_> {
        NodeChildren::new(
            self,
            expect_validated(expect_validated(self.as_node_scene().semantic_view()).roots()),
        )
    }

    pub fn node(&self, id: NodeId) -> Result<NodeRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::Node,
            id.generation(),
            id.index(),
            self.node_count(),
        )?;
        Ok(NodeRef::at(self, index))
    }
}
