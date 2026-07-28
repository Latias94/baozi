use baozi_operation::{CanonicalRecordView, SequenceView, StoredScalar};

use crate::canonical_scene::store::node_scene::decode_authored_transform;
use crate::canonical_scene::store::schema::{
    node_scene_child_reference_field, node_scene_node_field,
};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::entity::AuthoredTransform;

use super::ValidatedNodeScene;

pub(in crate::canonical_scene::store) type NodeSceneSemanticResult<T> =
    Result<T, NodeSceneSemanticError>;

#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) enum NodeSceneSemanticReadFault {
    ChildReferenceRow(usize),
    AuthoredTransformNode(usize),
}

/// A physical typed-view failure below an already validated semantic proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticError {
    reason: &'static str,
}

impl NodeSceneSemanticError {
    const fn new(reason: &'static str) -> Self {
        Self { reason }
    }

    pub(in crate::canonical_scene::store) const fn reason(&self) -> &'static str {
        self.reason
    }
}

/// One-time typed projections governed by the central validation proof.
///
/// This layer closes the constant-time physical table shape against the proof,
/// but deliberately does not revalidate directory semantics, topology,
/// identity, or strict-forward order. Traversal then fails closed whenever a
/// proof-bound scalar or transform slice cannot be read mechanically.
#[must_use = "the semantic view must be traversed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticView<'scene> {
    nodes: CanonicalRecordView<'scene>,
    references: Option<CanonicalRecordView<'scene>>,
    transforms: Option<SequenceView<'scene, f32>>,
    proof: NodeSceneValidationProof,
    #[cfg(test)]
    read_fault: Option<NodeSceneSemanticReadFault>,
}

impl ValidatedNodeScene {
    /// Resolves each typed retained view once under the attached validation proof.
    pub(in crate::canonical_scene::store) fn semantic_view(
        &self,
    ) -> NodeSceneSemanticResult<NodeSceneSemanticView<'_>> {
        NodeSceneSemanticView::try_new(self)
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_semantic_layout_fault_for_test(
        &mut self,
        source_node_count: usize,
        source_root_count: usize,
    ) -> bool {
        self.proof
            .install_layout_fault_for_test(source_node_count, source_root_count)
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_foreign_semantic_proof_for_test(
        &mut self,
        donor: &Self,
    ) {
        self.proof = donor.proof;
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_child_reference_read_fault_for_test(
        &mut self,
        row: usize,
    ) {
        self.semantic_read_fault = Some(NodeSceneSemanticReadFault::ChildReferenceRow(row));
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_authored_transform_read_fault_for_test(
        &mut self,
        source_node_index: usize,
    ) {
        self.semantic_read_fault = Some(NodeSceneSemanticReadFault::AuthoredTransformNode(
            source_node_index,
        ));
    }
}

impl<'scene> NodeSceneSemanticView<'scene> {
    fn try_new(scene: &'scene ValidatedNodeScene) -> NodeSceneSemanticResult<Self> {
        let proof = scene.proof;
        let nodes = scene
            .canonical
            .records(proof.nodes().into_store())
            .ok_or_else(|| invariant("validated node table is unavailable or mistyped"))?;
        let references = match proof.references() {
            Some(reference) => Some(scene.canonical.records(reference.into_store()).ok_or_else(
                || invariant("validated child-reference table is unavailable or mistyped"),
            )?),
            None => None,
        };
        let transforms = match proof.transforms() {
            Some(transform) => Some(
                scene
                    .canonical
                    .sequence(transform.into_store())
                    .ok_or_else(|| {
                        invariant(
                            "validated authored-transform sequence is unavailable or mistyped",
                        )
                    })?,
            ),
            None => None,
        };

        let expected_node_rows = proof
            .source_node_count()
            .checked_add(1)
            .ok_or_else(|| invariant("proof-qualified node table row count overflows"))?;
        if nodes.rows() != expected_node_rows {
            return Err(invariant(
                "validated node table row count disagrees with validation proof",
            ));
        }
        let expected_reference_rows = proof
            .source_root_count()
            .checked_add(proof.source_edge_count())
            .ok_or_else(|| invariant("proof-qualified reference table row count overflows"))?;
        if expected_reference_rows != proof.source_node_count() {
            return Err(invariant(
                "validation proof contains an inconsistent root-edge partition",
            ));
        }
        match (proof.source_node_count(), references) {
            (0, None) => {}
            (0, Some(_)) => {
                return Err(invariant(
                    "validated empty node scene retains a child-reference table",
                ));
            }
            (_, Some(reference)) if reference.rows() == expected_reference_rows => {}
            (_, Some(_)) => {
                return Err(invariant(
                    "validated child-reference table row count disagrees with validation proof",
                ));
            }
            (_, None) => {
                return Err(invariant(
                    "validated populated node scene is missing its child-reference table",
                ));
            }
        }
        match (proof.source_node_count(), transforms) {
            (0, None) => {}
            (0, Some(_)) => {
                return Err(invariant(
                    "validated empty node scene retains an authored-transform sequence",
                ));
            }
            (_, Some(_)) => {}
            (_, None) => {
                return Err(invariant(
                    "validated populated node scene is missing its authored-transform sequence",
                ));
            }
        }
        Ok(Self {
            nodes,
            references,
            transforms,
            proof,
            #[cfg(test)]
            read_fault: scene.semantic_read_fault,
        })
    }

    pub(in crate::canonical_scene::store) const fn node_count(&self) -> usize {
        self.proof.source_node_count()
    }

    pub(in crate::canonical_scene::store) const fn root_count(&self) -> usize {
        self.proof.source_root_count()
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> usize {
        self.proof.source_edge_count()
    }

    pub(in crate::canonical_scene::store) fn into_cursor(
        self,
    ) -> NodeSceneSemanticResult<NodeSceneSemanticCursor<'scene>> {
        let roots = NodeSceneChildCursor::try_new(
            self.references,
            0,
            self.proof.source_root_count(),
            self.proof.source_root_count(),
            #[cfg(test)]
            self.read_fault,
        )?;
        Ok(NodeSceneSemanticCursor {
            nodes: self.nodes,
            references: self.references,
            transforms: self.transforms,
            proof: self.proof,
            roots: Some(roots),
            next_node_index: 0,
            #[cfg(test)]
            read_fault: self.read_fault,
        })
    }
}

/// Linear semantic traversal bounded by the immutable validation proof.
#[must_use = "the semantic cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticCursor<'scene> {
    nodes: CanonicalRecordView<'scene>,
    references: Option<CanonicalRecordView<'scene>>,
    transforms: Option<SequenceView<'scene, f32>>,
    proof: NodeSceneValidationProof,
    roots: Option<NodeSceneChildCursor<'scene>>,
    next_node_index: usize,
    #[cfg(test)]
    read_fault: Option<NodeSceneSemanticReadFault>,
}

impl<'scene> NodeSceneSemanticCursor<'scene> {
    pub(in crate::canonical_scene::store) fn take_roots(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneChildCursor<'scene>> {
        self.roots
            .take()
            .ok_or_else(|| invariant("synthetic-root cursor was already taken"))
    }

    /// Returns `None` only at the proof-qualified source-node count.
    pub(in crate::canonical_scene::store) fn next_node(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSemanticNode<'scene>>> {
        if self.roots.is_some() {
            return Err(invariant(
                "synthetic-root cursor must be taken before source nodes",
            ));
        }
        if self.next_node_index == self.proof.source_node_count() {
            return Ok(None);
        }
        if self.next_node_index > self.proof.source_node_count() {
            return Err(invariant(
                "node cursor advanced beyond its proof-qualified count",
            ));
        }

        let source_index = self.next_node_index;
        let row = source_index
            .checked_add(1)
            .ok_or_else(|| invariant("canonical node row index overflows"))?;
        let child_offset = read_u32_as_usize(
            self.nodes,
            row,
            node_scene_node_field::CHILD_REFERENCE_OFFSET,
            "validated source-node child offset is unreadable",
        )?;
        let child_count = read_u32_as_usize(
            self.nodes,
            row,
            node_scene_node_field::CHILD_REFERENCE_COUNT,
            "validated source-node child count is unreadable",
        )?;
        let child_end = child_offset
            .checked_add(child_count)
            .ok_or_else(|| invariant("validated source-node child range overflows"))?;

        let transform_kind = read_scalar::<u8>(
            self.nodes,
            row,
            node_scene_node_field::TRANSFORM_KIND,
            "validated source-node transform kind is unreadable",
        )?;
        let transform_offset = read_u32_as_usize(
            self.nodes,
            row,
            node_scene_node_field::TRANSFORM_OFFSET,
            "validated source-node transform offset is unreadable",
        )?;
        let transform_length = read_u32_as_usize(
            self.nodes,
            row,
            node_scene_node_field::TRANSFORM_LENGTH,
            "validated source-node transform length is unreadable",
        )?;
        let transform_end = transform_offset
            .checked_add(transform_length)
            .ok_or_else(|| invariant("validated source-node transform range overflows"))?;
        #[cfg(test)]
        if matches!(
            self.read_fault,
            Some(NodeSceneSemanticReadFault::AuthoredTransformNode(index)) if index == source_index
        ) {
            return Err(invariant("injected authored-transform read failure"));
        }
        let transforms = self
            .transforms
            .ok_or_else(|| invariant("validated authored-transform view is absent"))?;
        let values = transforms
            .as_slice()
            .get(transform_offset..transform_end)
            .ok_or_else(|| invariant("validated authored-transform range is unreadable"))?;
        let authored_transform = decode_authored_transform(transform_kind, values)
            .ok_or_else(|| invariant("validated authored-transform payload is undecodable"))?;

        let children = NodeSceneChildCursor::try_new(
            self.references,
            child_offset,
            child_end,
            child_count,
            #[cfg(test)]
            self.read_fault,
        )?;
        self.next_node_index = source_index
            .checked_add(1)
            .ok_or_else(|| invariant("node cursor index overflows"))?;
        Ok(Some(NodeSceneSemanticNode {
            source_index,
            authored_transform,
            children,
        }))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.roots.is_some() {
            return Err(invariant("synthetic-root cursor was not taken"));
        }
        if self.next_node_index != self.proof.source_node_count() {
            return Err(invariant(
                "node cursor ended before its proof-qualified count",
            ));
        }
        Ok(())
    }
}

/// One decoded source node coupled to its mechanically readable child range.
#[must_use = "the decoded node owns its exact child cursor"]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticNode<'scene> {
    source_index: usize,
    authored_transform: AuthoredTransform,
    children: NodeSceneChildCursor<'scene>,
}

impl<'scene> NodeSceneSemanticNode<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn authored_transform(&self) -> AuthoredTransform {
        self.authored_transform
    }

    pub(in crate::canonical_scene::store) const fn child_count(&self) -> usize {
        self.children.declared_count()
    }

    pub(in crate::canonical_scene::store) fn into_children(self) -> NodeSceneChildCursor<'scene> {
        self.children
    }
}

/// Strict mechanical traversal of one proof-qualified reference range.
#[must_use = "the child cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneChildCursor<'scene> {
    references: Option<CanonicalRecordView<'scene>>,
    next_row: usize,
    end_row: usize,
    declared_count: usize,
    #[cfg(test)]
    read_fault: Option<NodeSceneSemanticReadFault>,
}

impl<'scene> NodeSceneChildCursor<'scene> {
    fn try_new(
        references: Option<CanonicalRecordView<'scene>>,
        start: usize,
        end: usize,
        declared_count: usize,
        #[cfg(test)] read_fault: Option<NodeSceneSemanticReadFault>,
    ) -> NodeSceneSemanticResult<Self> {
        if end.checked_sub(start) != Some(declared_count) {
            return Err(invariant("validated child-reference range is inconsistent"));
        }
        match references {
            Some(view) if end <= view.rows() => {}
            Some(_) => {
                return Err(invariant(
                    "validated child-reference range exceeds its typed view",
                ));
            }
            None if start == 0 && end == 0 => {}
            None => return Err(invariant("validated child-reference view is absent")),
        }
        Ok(Self {
            references,
            next_row: start,
            end_row: end,
            declared_count,
            #[cfg(test)]
            read_fault,
        })
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.declared_count
    }

    /// Returns `None` only at the exact end of the mechanically declared range.
    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<usize>> {
        if self.next_row == self.end_row {
            return Ok(None);
        }
        if self.next_row > self.end_row {
            return Err(invariant("child cursor advanced beyond its declared range"));
        }
        let row = self.next_row;
        #[cfg(test)]
        if matches!(
            self.read_fault,
            Some(NodeSceneSemanticReadFault::ChildReferenceRow(fault_row)) if fault_row == row
        ) {
            return Err(invariant("injected child-reference read failure"));
        }
        let references = self
            .references
            .ok_or_else(|| invariant("validated child-reference view is absent"))?;
        let child = read_u32_as_usize(
            references,
            row,
            node_scene_child_reference_field::CHILD,
            "validated child reference is unreadable",
        )?
        .checked_sub(1)
        .ok_or_else(|| invariant("validated child reference cannot address source space"))?;
        self.next_row = row
            .checked_add(1)
            .ok_or_else(|| invariant("child cursor row index overflows"))?;
        Ok(Some(child))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next_row != self.end_row {
            return Err(invariant("child cursor ended before its declared range"));
        }
        Ok(())
    }
}

fn read_u32_as_usize(
    records: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<usize> {
    let value = read_scalar::<u32>(records, row, field, reason)?;
    usize::try_from(value).map_err(|_| invariant(reason))
}

fn read_scalar<T: StoredScalar>(
    records: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<T> {
    records
        .scalar::<T>(row, field)
        .ok_or_else(|| invariant(reason))
}

const fn invariant(reason: &'static str) -> NodeSceneSemanticError {
    NodeSceneSemanticError::new(reason)
}
