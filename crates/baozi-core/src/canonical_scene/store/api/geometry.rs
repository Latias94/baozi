use std::fmt;

use super::super::construction::node_scene::semantic::{
    NodeSceneGeometryInspectionCursor, NodeSceneGeometryPrimitiveFacts, NodeSceneSemanticError,
};

pub use super::scene::ValidatedScene;
pub use crate::geometry::{PrimitiveCardinality, TopologyKind};
pub use crate::identity::{LogicalMeshId, PrimitiveId, SceneGenerationId};

impl ValidatedScene {
    /// Starts an allocation-free scan of proof-qualified primitive facts.
    pub fn geometry_cursor(&self) -> Result<SceneGeometryCursor<'_>, SceneGeometryReadError> {
        self.as_node_scene()
            .semantic_geometry_inspection()
            .map(SceneGeometryCursor::new)
            .map_err(SceneGeometryReadError)
    }
}

/// Allocation-free primitive facts exposed without retained storage vocabulary.
#[must_use = "the geometry cursor must be consumed or deliberately discarded"]
pub struct SceneGeometryCursor<'scene> {
    inner: NodeSceneGeometryInspectionCursor<'scene>,
}

impl<'scene> SceneGeometryCursor<'scene> {
    const fn new(inner: NodeSceneGeometryInspectionCursor<'scene>) -> Self {
        Self { inner }
    }

    /// Returns the exact primitive count sealed by central validation.
    pub const fn primitive_count(&self) -> usize {
        self.inner.primitive_count()
    }

    /// Reads the next source-ordered primitive and its post-process planning facts.
    pub fn next_primitive(
        &mut self,
    ) -> Result<Option<ScenePrimitiveFacts>, SceneGeometryReadError> {
        self.inner
            .next_primitive()
            .map(|primitive| primitive.map(ScenePrimitiveFacts::from_inner))
            .map_err(SceneGeometryReadError)
    }

    /// Proves that every primitive and attribute range was traversed exactly once.
    pub fn finish(self) -> Result<(), SceneGeometryReadError> {
        self.inner.finish().map_err(SceneGeometryReadError)
    }
}

/// Post-process planning facts for one proof-qualified primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenePrimitiveFacts {
    id: PrimitiveId,
    topology: TopologyKind,
    cardinality: PrimitiveCardinality,
    has_normals: bool,
}

impl ScenePrimitiveFacts {
    const fn from_inner(inner: NodeSceneGeometryPrimitiveFacts) -> Self {
        Self {
            id: inner.id(),
            topology: inner.topology(),
            cardinality: inner.cardinality(),
            has_normals: inner.has_normals(),
        }
    }

    /// Returns the generation- and logical-mesh-qualified primitive identity.
    pub const fn id(self) -> PrimitiveId {
        self.id
    }

    /// Returns the payload-free topology category.
    pub const fn topology(self) -> TopologyKind {
        self.topology
    }

    /// Returns the centrally validated vertex, corner, and face counts.
    pub const fn cardinality(self) -> PrimitiveCardinality {
        self.cardinality
    }

    /// Returns whether the primitive owns a validated Normal attribute.
    pub const fn has_normals(self) -> bool {
        self.has_normals
    }
}

/// Canonical geometry could not be projected from its sealed validation proof.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SceneGeometryReadError(NodeSceneSemanticError);

impl fmt::Debug for SceneGeometryReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SceneGeometryReadError")
    }
}

impl fmt::Display for SceneGeometryReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("validated canonical geometry could not be read")
    }
}

impl std::error::Error for SceneGeometryReadError {}
