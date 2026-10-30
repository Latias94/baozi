//! Direct semantic views over the proof-qualified typed deformation owner.
//!
//! Random reads borrow one row and its checked ranges. Exhaustive traversal additionally
//! checks phase ordering and exact range coverage; payload cursors inspect one item at a time.

use crate::canonical_scene::store::typed_storage::deformation::{
    DeformationStorage, StoredMorphDeltaPayload, StoredSkin,
};
use crate::canonical_scene::store::typed_storage::{StoredNodeIndex, TableRange, TextRange};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::deform::{MorphDeltaSemantic, SkinMatrixSpaces};
use crate::math::{Mat4, Vec3};

use super::{NodeSceneSemanticResult, invariant};

mod cursor;
mod payload;
mod read;
#[cfg(test)]
mod tests;

pub(in crate::canonical_scene::store) use cursor::NodeSceneDeformationCursor;
pub(in crate::canonical_scene::store) use payload::{
    NodeSceneDeformationF32Cursor, NodeSceneDeformationInfluenceRowsCursor,
    NodeSceneDeformationJointCursor, NodeSceneDeformationMatrixCursor,
    NodeSceneDeformationSparseDeltaCursor, NodeSceneDeformationVec3Cursor,
};

const MORPH_VECTOR_WIDTH: usize = 3;

#[cfg(test)]
std::thread_local! {
    static ROWS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn take_row_read_count_for_test() -> usize {
    ROWS_READ.with(|count| count.replace(0))
}

fn record_row_read() {
    #[cfg(test)]
    ROWS_READ.with(|count| count.set(count.get() + 1));
}

/// The actual deformation owner and the public source-node domain established by its proof.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationTables<'scene> {
    storage: &'scene DeformationStorage,
    source_node_count: usize,
}

impl<'scene> NodeSceneDeformationTables<'scene> {
    pub(in crate::canonical_scene::store) fn try_new(
        storage: &'scene DeformationStorage,
        proof: &NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        if [
            storage.skins.len(),
            storage.logical_morphs.len(),
            storage.morph_definitions.len(),
            storage.primitives.len(),
            storage.morph_targets.len(),
            storage.morph_deltas.len(),
            storage.mesh_instances.len(),
        ] != [
            proof.skin_count(),
            proof.logical_morph_count(),
            proof.morph_definition_count(),
            proof.primitive_deformation_count(),
            proof.morph_target_count(),
            proof.morph_delta_count(),
            proof.mesh_instance_count(),
        ] || storage.joints.len() != storage.inverse_bind_matrices.len()
            || storage.influence_joints.len() != storage.influence_weights.len()
            || storage.influence_joints.len() != storage.influence_source_orders.len()
        {
            return Err(invariant(
                "typed deformation tables disagree with central validation proof",
            ));
        }
        Ok(Self {
            storage,
            source_node_count: proof.source_node_count(),
        })
    }

    pub(super) fn into_cursor(self) -> NodeSceneDeformationCursor<'scene> {
        NodeSceneDeformationCursor::new(self)
    }
}

fn borrow_range<T>(range: TableRange<T>, values: &[T]) -> NodeSceneSemanticResult<&[T]> {
    range
        .get(values)
        .ok_or_else(|| invariant("validated deformation payload range is unavailable"))
}

fn borrow_name(name: Option<TextRange>, names: &[u8]) -> NodeSceneSemanticResult<Option<&str>> {
    name.map(|range| {
        range
            .get(names)
            .ok_or_else(|| invariant("validated deformation name range is unavailable"))
    })
    .transpose()
}

fn source_node(index: StoredNodeIndex, source_node_count: usize) -> NodeSceneSemanticResult<usize> {
    let source = usize::try_from(index.0)
        .ok()
        .and_then(|canonical| canonical.checked_sub(1))
        .filter(|source| *source < source_node_count)
        .ok_or_else(|| invariant("validated deformation node is outside source-node space"))?;
    Ok(source)
}

/// One borrowed skin, preserving its authored optional name, root, and matrix-space contract.
#[must_use = "the skin payload cursors must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationSkin<'scene> {
    source_index: usize,
    stored: &'scene StoredSkin,
    name: Option<&'scene str>,
    skeleton_root: Option<usize>,
    joints: Option<NodeSceneDeformationJointCursor<'scene>>,
    inverse_bind_matrices: Option<NodeSceneDeformationMatrixCursor<'scene>>,
}

impl<'scene> NodeSceneDeformationSkin<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn name(&self) -> Option<&'scene str> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn skeleton_root(&self) -> Option<usize> {
        self.skeleton_root
    }

    pub(in crate::canonical_scene::store) const fn matrix_spaces(&self) -> SkinMatrixSpaces {
        self.stored.matrix_spaces
    }

    pub(in crate::canonical_scene::store) fn take_joints(
        &mut self,
    ) -> Option<NodeSceneDeformationJointCursor<'scene>> {
        self.joints.take()
    }

    pub(in crate::canonical_scene::store) fn take_inverse_bind_matrices(
        &mut self,
    ) -> Option<NodeSceneDeformationMatrixCursor<'scene>> {
        self.inverse_bind_matrices.take()
    }
}

/// Logical target ownership and a borrowed default weight vector, including empty defaults.
#[must_use = "the logical-morph default cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationLogicalMorph<'scene> {
    source_index: usize,
    definition_offset: usize,
    definition_count: usize,
    default_weights: Option<NodeSceneDeformationF32Cursor<'scene>>,
}

impl<'scene> NodeSceneDeformationLogicalMorph<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn definition_offset(&self) -> usize {
        self.definition_offset
    }

    pub(in crate::canonical_scene::store) const fn definition_count(&self) -> usize {
        self.definition_count
    }

    pub(in crate::canonical_scene::store) fn take_default_weights(
        &mut self,
    ) -> Option<NodeSceneDeformationF32Cursor<'scene>> {
        self.default_weights.take()
    }
}

/// Stable producer identity is retained independently of the target's current ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMorphDefinition<'scene> {
    source_index: usize,
    logical_mesh: usize,
    target_ordinal: usize,
    stable_identity: u32,
    name: Option<&'scene str>,
}

impl<'scene> NodeSceneDeformationMorphDefinition<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh(self) -> usize {
        self.logical_mesh
    }

    pub(in crate::canonical_scene::store) const fn target_ordinal(self) -> usize {
        self.target_ordinal
    }

    pub(in crate::canonical_scene::store) const fn stable_identity(self) -> u32 {
        self.stable_identity
    }

    pub(in crate::canonical_scene::store) const fn name(self) -> Option<&'scene str> {
        self.name
    }
}

#[must_use = "the primitive influence cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationPrimitive<'scene> {
    source_index: usize,
    influences: Option<NodeSceneDeformationInfluenceRowsCursor<'scene>>,
    morph_target_offset: usize,
    morph_target_count: usize,
}

impl<'scene> NodeSceneDeformationPrimitive<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn influences_present(&self) -> bool {
        self.influences.is_some()
    }

    pub(in crate::canonical_scene::store) fn take_influences(
        &mut self,
    ) -> Option<NodeSceneDeformationInfluenceRowsCursor<'scene>> {
        self.influences.take()
    }

    pub(in crate::canonical_scene::store) const fn morph_target_offset(&self) -> usize {
        self.morph_target_offset
    }

    pub(in crate::canonical_scene::store) const fn morph_target_count(&self) -> usize {
        self.morph_target_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMorphTarget {
    source_index: usize,
    primitive: usize,
    target_ordinal: usize,
    delta_offset: usize,
    delta_count: usize,
}

impl NodeSceneDeformationMorphTarget {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn primitive(self) -> usize {
        self.primitive
    }

    pub(in crate::canonical_scene::store) const fn target_ordinal(self) -> usize {
        self.target_ordinal
    }

    pub(in crate::canonical_scene::store) const fn delta_offset(self) -> usize {
        self.delta_offset
    }

    pub(in crate::canonical_scene::store) const fn delta_count(self) -> usize {
        self.delta_count
    }
}

#[must_use = "the morph-delta payload cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMorphDelta<'scene> {
    source_index: usize,
    morph_target: usize,
    semantic: MorphDeltaSemantic,
    payload: NodeSceneDeformationMorphPayload<'scene>,
}

impl<'scene> NodeSceneDeformationMorphDelta<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn morph_target(&self) -> usize {
        self.morph_target
    }

    pub(in crate::canonical_scene::store) const fn semantic(&self) -> MorphDeltaSemantic {
        self.semantic
    }

    pub(in crate::canonical_scene::store) fn into_payload(
        self,
    ) -> NodeSceneDeformationMorphPayload<'scene> {
        self.payload
    }
}

/// Representation and explicit sparse tuples remain exactly as authored.
#[must_use = "the morph payload cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) enum NodeSceneDeformationMorphPayload<'scene> {
    Dense {
        logical_count: usize,
        values: NodeSceneDeformationVec3Cursor<'scene>,
    },
    Sparse {
        logical_count: usize,
        values: NodeSceneDeformationSparseDeltaCursor<'scene>,
    },
}

impl NodeSceneDeformationMorphPayload<'_> {
    pub(in crate::canonical_scene::store) const fn logical_count(&self) -> usize {
        match self {
            Self::Dense { logical_count, .. } | Self::Sparse { logical_count, .. } => {
                *logical_count
            }
        }
    }

    pub(in crate::canonical_scene::store) const fn explicit_count(&self) -> usize {
        match self {
            Self::Dense { values, .. } => values.declared_count(),
            Self::Sparse { values, .. } => values.declared_count(),
        }
    }
}

#[must_use = "the instance morph-weight cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMeshInstance<'scene> {
    source_index: usize,
    skin: Option<usize>,
    morph_weights: Option<NodeSceneDeformationF32Cursor<'scene>>,
}

impl<'scene> NodeSceneDeformationMeshInstance<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn skin(&self) -> Option<usize> {
        self.skin
    }

    pub(in crate::canonical_scene::store) const fn morph_weights_present(&self) -> bool {
        self.morph_weights.is_some()
    }

    pub(in crate::canonical_scene::store) fn take_morph_weights(
        &mut self,
    ) -> Option<NodeSceneDeformationF32Cursor<'scene>> {
        self.morph_weights.take()
    }
}
