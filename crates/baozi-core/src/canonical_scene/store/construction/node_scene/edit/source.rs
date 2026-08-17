use thiserror::Error;

use crate::entity::AuthoredTransform;
use crate::identity::{NodeId, NodeIndex, SceneEntityKind, SceneLookupError, SceneStorageIndex};
use crate::provenance::{OperationIdentity, OperationKind, OptionsDigest};

use super::super::{ValidatedNodeScene, semantic::NodeSceneSemanticError};
use super::plan::{NodeSceneEditPlanError, PendingNodeSceneEdit};

/// The allocation-free result of classifying one transform-only edit request.
#[must_use = "a node transform edit start must be consumed or deliberately dropped"]
#[allow(
    clippy::large_enum_variant,
    reason = "boxing Changed would allocate before edit-operation admission; the start result is consumed immediately"
)]
pub(in crate::canonical_scene::store) enum NodeTransformEditStart<'scene> {
    /// The requested authored bits already exist; this is the exact input Scene.
    Unchanged(&'scene ValidatedNodeScene),
    /// The request changes authored bits and is ready for operation admission.
    Changed(PendingNodeSceneEdit<'scene>),
}

/// A rejected transform-edit request before any operation or generation reservation starts.
#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeTransformEditStartError {
    #[error(transparent)]
    Lookup(#[from] SceneLookupError),
    #[error("validated node Scene semantic read failed while planning a transform edit: {reason}")]
    SemanticRead { reason: &'static str },
    #[error("validated node Scene cannot support a transform edit: {reason}")]
    InvalidSource { reason: &'static str },
    #[error("node transform edit size overflowed while computing {quantity}")]
    SizeOverflow { quantity: &'static str },
}

/// Classifies an authored node-transform replacement without starting retained work.
///
/// Generation qualification intentionally precedes every semantic lookup. An unchanged
/// result borrows `input` directly and performs no allocation, operation start, or generation
/// reservation. A changed result contains only logical coordinates and checked fixed counts.
pub(in crate::canonical_scene::store) fn try_begin_node_transform_edit<'scene>(
    input: &'scene ValidatedNodeScene,
    node: NodeId,
    replacement: AuthoredTransform,
    options_digest: OptionsDigest,
) -> Result<NodeTransformEditStart<'scene>, NodeTransformEditStartError> {
    let target = NodeIndex::from_handle(input.generation(), node)?;
    let proof = input.proof;
    let source_node_count = proof.source_node_count();
    let target = try_target_coordinates(target, source_node_count)?;
    let target_transform = input
        .semantic_authored_transform_at(target.source_index)
        .map_err(semantic_read_error)?;
    let old_transform = target_transform.authored_transform();
    if old_transform.has_same_authored_bits(replacement) {
        return Ok(NodeTransformEditStart::Unchanged(input));
    }

    PendingNodeSceneEdit::try_new_node_transform(
        input,
        target.source_index,
        target.canonical_row,
        target_transform.transform_offset(),
        old_transform,
        replacement,
        OperationIdentity::without_evidence(OperationKind::Transform, options_digest),
        source_node_count,
        proof.source_root_count(),
        proof.source_edge_count(),
        target_transform.sequence_scalar_count(),
    )
    .map(NodeTransformEditStart::Changed)
    .map_err(NodeTransformEditStartError::from)
}

#[derive(Debug, Clone, Copy)]
struct TargetCoordinates {
    source_index: usize,
    canonical_row: usize,
}

fn try_target_coordinates(
    target: NodeIndex,
    source_node_count: usize,
) -> Result<TargetCoordinates, NodeTransformEditStartError> {
    let source_index =
        target
            .try_to_usize()
            .map_err(|_| NodeTransformEditStartError::SizeOverflow {
                quantity: "target source node index",
            })?;
    if source_index >= source_node_count {
        return Err(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Node,
            length: source_node_count,
        }
        .into());
    }
    let canonical_row =
        source_index
            .checked_add(1)
            .ok_or(NodeTransformEditStartError::SizeOverflow {
                quantity: "target canonical node row",
            })?;
    let _ =
        u32::try_from(canonical_row).map_err(|_| NodeTransformEditStartError::SizeOverflow {
            quantity: "target canonical node row",
        })?;
    Ok(TargetCoordinates {
        source_index,
        canonical_row,
    })
}

fn semantic_read_error(error: NodeSceneSemanticError) -> NodeTransformEditStartError {
    NodeTransformEditStartError::SemanticRead {
        reason: error.reason(),
    }
}

impl From<NodeSceneEditPlanError> for NodeTransformEditStartError {
    fn from(error: NodeSceneEditPlanError) -> Self {
        match error {
            NodeSceneEditPlanError::InvalidSource { reason } => Self::InvalidSource { reason },
            NodeSceneEditPlanError::SizeOverflow { quantity } => Self::SizeOverflow { quantity },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::identity::{NodeIndex, SceneEntityKind, SceneLookupError};

    use super::{NodeTransformEditStartError, try_target_coordinates};

    #[test]
    fn source_index_zero_maps_to_the_first_row_after_the_synthetic_root() {
        let target = try_target_coordinates(NodeIndex::from_u32(0), 1)
            .expect("the first source node must remain addressable");

        assert_eq!(target.source_index, 0);
        assert_eq!(target.canonical_row, 1);
    }

    #[test]
    fn source_index_equal_to_the_proof_count_is_out_of_range() {
        let error = try_target_coordinates(NodeIndex::from_u32(3), 3)
            .expect_err("the upper source-node bound must be exclusive");

        assert!(matches!(
            error,
            NodeTransformEditStartError::Lookup(SceneLookupError::OutOfRange {
                kind: SceneEntityKind::Node,
                length: 3,
            })
        ));
    }
}
