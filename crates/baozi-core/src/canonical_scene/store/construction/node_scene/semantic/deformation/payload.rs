//! Small borrowed slice cursors. Construction never scans a payload or allocates a copy.

use super::*;

#[must_use = "the skin joint cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationJointCursor<'scene> {
    canonical_nodes: &'scene [StoredNodeIndex],
    source_node_count: usize,
    next: usize,
}

impl<'scene> NodeSceneDeformationJointCursor<'scene> {
    pub(super) const fn new(
        canonical_nodes: &'scene [StoredNodeIndex],
        source_node_count: usize,
    ) -> Self {
        Self {
            canonical_nodes,
            source_node_count,
            next: 0,
        }
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.canonical_nodes.len()
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<usize>> {
        let Some(&index) = self.canonical_nodes.get(self.next) else {
            return Ok(None);
        };
        let source = source_node(index, self.source_node_count)?;
        self.next += 1;
        Ok(Some(source))
    }

    /// These remain canonical typed node indices; row zero is the synthetic root.
    pub(in crate::canonical_scene::store) const fn into_slice(self) -> &'scene [StoredNodeIndex] {
        self.canonical_nodes
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.next, self.canonical_nodes.len())
    }
}

#[must_use = "the matrix cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMatrixCursor<'scene> {
    matrices: &'scene [Mat4],
    next: usize,
}

impl<'scene> NodeSceneDeformationMatrixCursor<'scene> {
    pub(super) const fn new(matrices: &'scene [Mat4]) -> Self {
        Self { matrices, next: 0 }
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.matrices.len()
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<Mat4>> {
        let Some(matrix) = self.matrices.get(self.next).copied() else {
            return Ok(None);
        };
        let matrix = affine_matrix(matrix)?;
        self.next += 1;
        Ok(Some(matrix))
    }

    /// Only the yielded Mat4 is copied; all remaining matrices stay borrowed.
    pub(in crate::canonical_scene::store) fn into_matrices(
        self,
    ) -> impl ExactSizeIterator<Item = NodeSceneSemanticResult<Mat4>>
    + DoubleEndedIterator
    + Clone
    + 'scene {
        self.matrices.iter().copied().map(affine_matrix)
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.next, self.matrices.len())
    }
}

fn affine_matrix(matrix: Mat4) -> NodeSceneSemanticResult<Mat4> {
    if !matrix.is_finite() || !matrix.is_affine() {
        return Err(invariant(
            "validated deformation matrix is not finite affine data",
        ));
    }
    Ok(matrix)
}

#[must_use = "the F32 cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationF32Cursor<'scene> {
    values: &'scene [f32],
    next: usize,
}

impl<'scene> NodeSceneDeformationF32Cursor<'scene> {
    pub(super) const fn new(values: &'scene [f32]) -> Self {
        Self { values, next: 0 }
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.values.len()
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<f32>> {
        let Some(&value) = self.values.get(self.next) else {
            return Ok(None);
        };
        if !value.is_finite() {
            return Err(invariant("validated deformation weight is not finite"));
        }
        self.next += 1;
        Ok(Some(value))
    }

    pub(in crate::canonical_scene::store) const fn into_slice(self) -> &'scene [f32] {
        self.values
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.next, self.values.len())
    }
}

#[must_use = "the morph-vector cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationVec3Cursor<'scene> {
    scalars: &'scene [f32],
    next_vector: usize,
}

impl<'scene> NodeSceneDeformationVec3Cursor<'scene> {
    pub(super) fn try_new(scalars: &'scene [f32]) -> NodeSceneSemanticResult<Self> {
        if !scalars.len().is_multiple_of(MORPH_VECTOR_WIDTH) {
            return Err(invariant(
                "validated morph payload is not an exact F32x3 sequence",
            ));
        }
        Ok(Self {
            scalars,
            next_vector: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.scalars.len() / MORPH_VECTOR_WIDTH
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<Vec3>> {
        if self.next_vector == self.declared_count() {
            return Ok(None);
        }
        let start = self.next_vector * MORPH_VECTOR_WIDTH;
        let values = self
            .scalars
            .get(start..start + MORPH_VECTOR_WIDTH)
            .ok_or_else(|| invariant("morph-vector cursor advanced beyond its range"))?;
        let value = Vec3::new(values[0], values[1], values[2]);
        if !value.is_finite() {
            return Err(invariant("validated morph-vector payload is not finite"));
        }
        self.next_vector += 1;
        Ok(Some(value))
    }

    pub(in crate::canonical_scene::store) const fn into_slice(self) -> &'scene [f32] {
        self.scalars
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.next_vector, self.declared_count())
    }
}

#[must_use = "the sparse morph cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationSparseDeltaCursor<'scene> {
    logical_count: usize,
    indices: &'scene [u32],
    values: NodeSceneDeformationVec3Cursor<'scene>,
    next: usize,
}

impl<'scene> NodeSceneDeformationSparseDeltaCursor<'scene> {
    pub(super) fn try_new(
        logical_count: usize,
        indices: &'scene [u32],
        scalars: &'scene [f32],
    ) -> NodeSceneSemanticResult<Self> {
        let values = NodeSceneDeformationVec3Cursor::try_new(scalars)?;
        if indices.len() != values.declared_count() {
            return Err(invariant(
                "validated sparse morph indices and values have different lengths",
            ));
        }
        Ok(Self {
            logical_count,
            indices,
            values,
            next: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.indices.len()
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationSparseDelta>> {
        let Some(&index) = self.indices.get(self.next) else {
            return Ok(None);
        };
        if index as usize >= self.logical_count
            || (self.next > 0 && self.indices[self.next - 1] >= index)
        {
            return Err(invariant(
                "validated sparse morph indices are unordered or out of range",
            ));
        }
        let value = self
            .values
            .next()?
            .ok_or_else(|| invariant("sparse morph value is unavailable"))?;
        self.next += 1;
        Ok(Some(NodeSceneDeformationSparseDelta {
            index: index as usize,
            value,
        }))
    }

    pub(in crate::canonical_scene::store) const fn into_slices(
        self,
    ) -> (&'scene [u32], &'scene [f32]) {
        (self.indices, self.values.into_slice())
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.next, self.indices.len())?;
        self.values.finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationSparseDelta {
    index: usize,
    value: Vec3,
}

impl NodeSceneDeformationSparseDelta {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn value(self) -> Vec3 {
        self.value
    }
}

#[must_use = "the influence-row cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationInfluenceRowsCursor<'scene> {
    offsets: &'scene [u32],
    joints: &'scene [u32],
    weights: &'scene [f32],
    source_orders: &'scene [u32],
    next_vertex: usize,
}

impl<'scene> NodeSceneDeformationInfluenceRowsCursor<'scene> {
    pub(super) fn try_new(
        offsets: &'scene [u32],
        joints: &'scene [u32],
        weights: &'scene [f32],
        source_orders: &'scene [u32],
    ) -> NodeSceneSemanticResult<Self> {
        if offsets.first().copied() != Some(0)
            || joints.len() != weights.len()
            || joints.len() != source_orders.len()
            || offsets.last().and_then(|&end| usize::try_from(end).ok()) != Some(joints.len())
        {
            return Err(invariant(
                "validated influence rows have inconsistent payload boundaries",
            ));
        }
        Ok(Self {
            offsets,
            joints,
            weights,
            source_orders,
            next_vertex: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn vertex_count(&self) -> usize {
        self.offsets.len() - 1
    }

    pub(in crate::canonical_scene::store) fn next_row(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationInfluenceRow<'scene>>> {
        if self.next_vertex == self.vertex_count() {
            return Ok(None);
        }
        let start = self.offsets[self.next_vertex] as usize;
        let end = self.offsets[self.next_vertex + 1] as usize;
        let row = NodeSceneDeformationInfluenceRow {
            vertex: self.next_vertex,
            influences: NodeSceneDeformationInfluenceCursor {
                joints: self
                    .joints
                    .get(start..end)
                    .ok_or_else(|| invariant("validated influence-joint row is unavailable"))?,
                weights: self
                    .weights
                    .get(start..end)
                    .ok_or_else(|| invariant("validated influence-weight row is unavailable"))?,
                source_orders: self.source_orders.get(start..end).ok_or_else(|| {
                    invariant("validated influence source-order row is unavailable")
                })?,
                next: 0,
            },
        };
        self.next_vertex += 1;
        Ok(Some(row))
    }

    pub(in crate::canonical_scene::store) const fn into_slices(
        self,
    ) -> (&'scene [u32], &'scene [u32], &'scene [f32], &'scene [u32]) {
        (self.offsets, self.joints, self.weights, self.source_orders)
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.next_vertex, self.vertex_count())
    }
}

#[must_use = "the influence row must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationInfluenceRow<'scene> {
    vertex: usize,
    influences: NodeSceneDeformationInfluenceCursor<'scene>,
}

impl<'scene> NodeSceneDeformationInfluenceRow<'scene> {
    pub(in crate::canonical_scene::store) const fn vertex(&self) -> usize {
        self.vertex
    }

    pub(in crate::canonical_scene::store) fn into_influences(
        self,
    ) -> NodeSceneDeformationInfluenceCursor<'scene> {
        self.influences
    }
}

#[must_use = "the influence cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationInfluenceCursor<'scene> {
    joints: &'scene [u32],
    weights: &'scene [f32],
    source_orders: &'scene [u32],
    next: usize,
}

impl NodeSceneDeformationInfluenceCursor<'_> {
    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.joints.len()
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationInfluence>> {
        let Some(&joint) = self.joints.get(self.next) else {
            return Ok(None);
        };
        let weight = self.weights[self.next];
        let source_order = self.source_orders[self.next];
        if !weight.is_finite() || weight < 0.0 || source_order as usize >= self.joints.len() {
            return Err(invariant(
                "validated influence weight or source order is invalid",
            ));
        }
        self.next += 1;
        Ok(Some(NodeSceneDeformationInfluence {
            palette_joint: joint,
            weight,
            source_order,
        }))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.next, self.joints.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationInfluence {
    palette_joint: u32,
    weight: f32,
    source_order: u32,
}

impl NodeSceneDeformationInfluence {
    pub(in crate::canonical_scene::store) const fn palette_joint(self) -> u32 {
        self.palette_joint
    }

    pub(in crate::canonical_scene::store) const fn weight(self) -> f32 {
        self.weight
    }

    pub(in crate::canonical_scene::store) const fn source_order(self) -> u32 {
        self.source_order
    }
}

fn exhausted(next: usize, count: usize) -> NodeSceneSemanticResult<()> {
    (next == count)
        .then_some(())
        .ok_or_else(|| invariant("deformation payload cursor ended before its declared count"))
}
