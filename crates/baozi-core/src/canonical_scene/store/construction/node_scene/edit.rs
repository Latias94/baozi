use baozi_operation::{CanonicalRecordView, RecordBlock, StoreRuntimeError};
use thiserror::Error;

use crate::canonical_scene::store::api::scene::ValidatedScene;
use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};
use crate::entity::AuthoredTransform;
use crate::identity::{NodeId, SceneGenerationId};
use crate::provenance::OptionsDigest;

use super::SceneStorage;

mod execution;
mod graph;
mod leaf_plan;
mod plan;
mod publication;
mod report;
mod source;

use plan::PendingNodeSceneEdit;
use source::NodeTransformEditStartError;

/// A typed, generation-local node transform request for the hidden Scene integration boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneNodeTransformEditRequest {
    node_index: u32,
    replacement: AuthoredTransform,
    options_digest: OptionsDigest,
}

impl SceneNodeTransformEditRequest {
    /// Creates a request addressed by the source node's zero-based index.
    pub const fn new(
        node_index: u32,
        replacement: AuthoredTransform,
        options_digest: OptionsDigest,
    ) -> Self {
        Self {
            node_index,
            replacement,
            options_digest,
        }
    }

    /// Returns the source node index addressed by this request.
    pub const fn node_index(self) -> u32 {
        self.node_index
    }

    /// Returns the replacement authored transform.
    pub const fn replacement(self) -> AuthoredTransform {
        self.replacement
    }

    /// Returns the caller-supplied operation options digest.
    pub const fn options_digest(self) -> OptionsDigest {
        self.options_digest
    }
}

/// The allocation-free result of compiling one typed node transform request.
#[must_use = "a compiled Scene edit must be executed or deliberately dropped"]
#[allow(
    clippy::large_enum_variant,
    reason = "the changed plan stays inline so planning remains allocation-free"
)]
pub enum SceneEditCompilation {
    /// The request already matches the authored bits in the input Scene.
    Unchanged,
    /// The request produced an opaque edit plan sealed to the input publication.
    Changed(SceneNodeTransformEditPlan),
}

impl std::fmt::Debug for SceneEditCompilation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unchanged => formatter.write_str("SceneEditCompilation::Unchanged"),
            Self::Changed(plan) => formatter
                .debug_tuple("SceneEditCompilation::Changed")
                .field(plan)
                .finish(),
        }
    }
}

/// An opaque node-transform edit plan that can be executed only by the core edit rail.
#[must_use = "a compiled Scene edit plan must be executed or deliberately dropped"]
pub struct SceneNodeTransformEditPlan(PendingNodeSceneEdit);

impl std::fmt::Debug for SceneNodeTransformEditPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SceneNodeTransformEditPlan")
            .field("input_generation", &self.input_generation())
            .finish_non_exhaustive()
    }
}

impl SceneNodeTransformEditPlan {
    /// Returns the exact generation against which the plan was compiled.
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.0.input_generation()
    }
}

/// A stable, postprocess-safe projection of edit-planning failure.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SceneEditCompileError {
    /// The request addressed an unavailable or foreign node.
    #[error("the Scene edit target is not valid for the input generation")]
    InvalidTarget,
    /// The sealed Scene could not be read through its semantic view.
    #[error("the validated Scene semantic view could not be read")]
    SemanticRead,
    /// The retained source did not satisfy the edit plan's invariants.
    #[error("the validated Scene cannot support the requested edit")]
    InvalidSource,
    /// A checked edit shape calculation overflowed.
    #[error("the Scene edit shape exceeded its checked bounds")]
    SizeOverflow,
}

impl From<NodeTransformEditStartError> for SceneEditCompileError {
    fn from(error: NodeTransformEditStartError) -> Self {
        match error {
            NodeTransformEditStartError::Lookup(_) => Self::InvalidTarget,
            NodeTransformEditStartError::SemanticRead { .. } => Self::SemanticRead,
            NodeTransformEditStartError::InvalidSource { .. } => Self::InvalidSource,
            NodeTransformEditStartError::SizeOverflow { .. } => Self::SizeOverflow,
        }
    }
}

/// Compiles one typed node transform request without starting retained work or reserving a
/// destination generation.
pub fn compile_node_transform_edit(
    input: &ValidatedScene,
    request: SceneNodeTransformEditRequest,
) -> Result<SceneEditCompilation, SceneEditCompileError> {
    let node = NodeId::new(input.generation(), request.node_index());
    match source::try_begin_node_transform_edit(
        input.as_node_scene(),
        node,
        request.replacement(),
        request.options_digest(),
    )
    .map_err(SceneEditCompileError::from)?
    {
        source::NodeTransformEditStart::Unchanged(_) => Ok(SceneEditCompilation::Unchanged),
        source::NodeTransformEditStart::Changed(plan) => Ok(SceneEditCompilation::Changed(
            SceneNodeTransformEditPlan(plan),
        )),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(in crate::canonical_scene::store) use publication::NodeSceneEditTerminal;
#[cfg(test)]
pub(in crate::canonical_scene::store) use source::{
    NodeTransformEditStart, try_begin_node_transform_edit,
};

fn required_record_view<'scene, Schema>(
    input: &'scene SceneStorage,
    source: CanonicalBlockRef<Schema>,
) -> Result<CanonicalRecordView<'scene>, StoreRuntimeError>
where
    Schema: CanonicalSchema<StoreKind = RecordBlock>,
{
    input
        .canonical
        .records(source.into_store())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)
}
