//! Bounded, owned reverse relations over a centrally validated Scene forest.

use std::{fmt, mem::size_of, num::NonZeroU32};

use baozi_operation::{CancellationToken, CapacityPlan, CapacityPlanError};
use thiserror::Error;

use crate::{
    MeshInstanceId, NodeId, SceneEntityKind, SceneGenerationId, SceneLookupError, ValidatedScene,
};

/// Explicit construction bounds, independent of Scene storage and import accounting.
///
/// For N public nodes, E parent-child edges and I instances, construction charges exactly
/// `5*N + E + 3*I + 1` work units: each initialized storage slot, each visited node and edge,
/// each instance in both inventory passes, and each node in the CSR prefix pass.
///
/// Bytes bound the sum of the admitted [`CapacityPlan`] envelopes for N optional nonzero U32
/// parent entries, N+1 `usize` offsets, I U32 instance ordinals and N `usize` scratch cursors.
/// Each envelope uses the actual element size; allocator-returned capacities are checked before
/// initialization. The inline owners and the existing Scene are excluded. Scratch is released
/// before success, and [`SceneRelations::allocated_bytes`] reports actual retained capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationsQueryLimits {
    max_work: u64,
    max_bytes: u64,
}

impl RelationsQueryLimits {
    pub const fn new(max_work: u64, max_bytes: u64) -> Self {
        Self {
            max_work,
            max_bytes,
        }
    }

    pub const fn max_work(self) -> u64 {
        self.max_work
    }

    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }
}

/// Which explicit relations construction limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationsQueryLimit {
    Work,
    Bytes,
}

/// Why a relations query could not produce its complete owned index.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RelationsQueryError {
    #[error("relations query {limit:?} reached {actual}, above limit {maximum}")]
    LimitExceeded {
        limit: RelationsQueryLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("relations query size or work cannot be represented")]
    SizeOverflow,
    #[error("relations query capacity contract failed: {0}")]
    Capacity(#[from] CapacityPlanError),
    #[error("relations query allocation failed")]
    AllocationFailed,
    #[error("relations query cancelled")]
    Cancelled,
}

/// Owned, generation-bound parent and node-to-instance relations.
///
/// Derivation is opt-in and takes O(N + E + I) time with bounded output and scratch storage.
/// Parent lookup and instance-row creation take O(1); advancing the exact-size iterator takes
/// O(1) per item. Lookups allocate nothing and check generation before range.
///
/// Roots have no parent. Instance rows preserve Scene inventory order, including distinct
/// placements sharing a mesh. Nodes without placements have an empty row. The internal
/// synthetic root is excluded from the index and every returned handle.
///
/// This index owns no Scene, resource payload, provenance or import accounting and may outlive
/// the source Scene. Its handles address only that generation. Derive a new index after any
/// Scene edit, even when the hierarchy and placements are unchanged.
pub struct SceneRelations {
    generation: SceneGenerationId,
    parents: Vec<Option<NonZeroU32>>,
    instance_offsets: Vec<usize>,
    instance_ordinals: Vec<u32>,
}

impl fmt::Debug for SceneRelations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneRelations")
            .field("generation", &self.generation)
            .field("node_count", &self.node_count())
            .field("mesh_instance_count", &self.mesh_instance_count())
            .finish()
    }
}

impl SceneRelations {
    pub const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    /// Number of public nodes, excluding the internal synthetic root.
    pub fn node_count(&self) -> usize {
        self.parents.len()
    }

    pub fn mesh_instance_count(&self) -> usize {
        self.instance_ordinals.len()
    }

    /// Actual retained vector capacity in bytes, excluding inline fields and released scratch.
    pub fn allocated_bytes(&self) -> usize {
        self.parents.capacity() * size_of::<Option<NonZeroU32>>()
            + self.instance_offsets.capacity() * size_of::<usize>()
            + self.instance_ordinals.capacity() * size_of::<u32>()
    }

    /// Returns the parent in O(1), or `None` for a forest root.
    pub fn parent(&self, node: NodeId) -> Result<Option<NodeId>, SceneLookupError> {
        let index = self.node_index(node)?;
        Ok(self.parents[index].map(|parent| NodeId::new(self.generation, parent.get() - 1)))
    }

    /// Returns an allocation-free exact-size row in original Scene instance inventory order.
    ///
    /// The iterator borrows only this index; the source Scene may already have been dropped.
    pub fn instances(
        &self,
        node: NodeId,
    ) -> Result<impl ExactSizeIterator<Item = MeshInstanceId> + '_, SceneLookupError> {
        let index = self.node_index(node)?;
        let start = self.instance_offsets[index];
        let end = self.instance_offsets[index + 1];
        Ok(self.instance_ordinals[start..end]
            .iter()
            .map(|&ordinal| MeshInstanceId::new(self.generation, ordinal)))
    }

    fn node_index(&self, node: NodeId) -> Result<usize, SceneLookupError> {
        if node.generation() != self.generation {
            return Err(SceneLookupError::GenerationMismatch {
                kind: SceneEntityKind::Node,
                expected: self.generation,
                actual: node.generation(),
            });
        }
        let index = node.index() as usize;
        if index >= self.node_count() {
            return Err(SceneLookupError::OutOfRange {
                kind: SceneEntityKind::Node,
                length: self.node_count(),
            });
        }
        Ok(index)
    }
}

impl ValidatedScene {
    /// Derives complete parent and node-to-instance relations without changing the Scene.
    ///
    /// All output and scratch capacity envelopes and the complete work count are admitted
    /// before allocation. Every allocation is fallible and checked against its capacity plan.
    /// Cancellation is polled before allocation, at each work unit and before returning.
    /// Any failure discards the complete index and scratch; no partial result escapes.
    ///
    /// Central validation owns the single-parent, acyclic forest invariant. Construction uses
    /// the existing child and instance-owner readers without revalidating the graph. An empty
    /// Scene is valid and needs one work unit and one offset slot for the CSR sentinel.
    pub fn relations(
        &self,
        limits: RelationsQueryLimits,
        cancellation: Option<&CancellationToken>,
    ) -> Result<SceneRelations, RelationsQueryError> {
        let mut query = Query::new(limits, cancellation)?;
        let count = self.node_count();
        let instance_count = self.mesh_instance_count();
        let edges = count
            .checked_sub(self.root_count())
            .ok_or(RelationsQueryError::SizeOverflow)?;
        check_limit(
            RelationsQueryLimit::Work,
            construction_work(count, edges, instance_count)?,
            limits.max_work,
        )?;
        let plan = RelationsPlan::new(count, instance_count)?;
        query.bytes(plan.peak_bytes()?)?;

        let mut parents = allocate::<Option<NonZeroU32>>(plan.parents, &query)?;
        let mut instance_offsets = allocate::<usize>(plan.offsets, &query)?;
        let mut instance_ordinals = allocate::<u32>(plan.ordinals, &query)?;
        let mut cursors = allocate::<usize>(plan.cursors, &query)?;
        query.bytes(storage_bytes([
            (parents.capacity(), size_of::<Option<NonZeroU32>>()),
            (instance_offsets.capacity(), size_of::<usize>()),
            (instance_ordinals.capacity(), size_of::<u32>()),
            (cursors.capacity(), size_of::<usize>()),
        ])?)?;

        for _ in 0..count {
            query.step()?;
            parents.push(None);
        }
        for _ in 0..=count {
            query.step()?;
            instance_offsets.push(0_usize);
        }
        for _ in 0..instance_count {
            query.step()?;
            instance_ordinals.push(0);
        }
        for _ in 0..count {
            query.step()?;
            cursors.push(0_usize);
        }

        // Only public child links participate, so roots retain None and no synthetic ID escapes.
        for node in self.nodes() {
            query.step()?;
            let parent = NonZeroU32::new(
                node.id()
                    .index()
                    .checked_add(1)
                    .ok_or(RelationsQueryError::SizeOverflow)?,
            );
            for child in node.children() {
                query.step()?;
                parents[child.id().index() as usize] = parent;
            }
        }
        for instance in self.mesh_instances() {
            query.step()?;
            let count = &mut cursors[instance.node().id().index() as usize];
            *count = count
                .checked_add(1)
                .ok_or(RelationsQueryError::SizeOverflow)?;
        }
        for (index, cursor) in cursors.iter_mut().enumerate() {
            query.step()?;
            instance_offsets[index + 1] = instance_offsets[index]
                .checked_add(*cursor)
                .ok_or(RelationsQueryError::SizeOverflow)?;
            *cursor = instance_offsets[index];
        }
        // Scattering a second inventory pass keeps each CSR row stable without sorting.
        for instance in self.mesh_instances() {
            query.step()?;
            let cursor = &mut cursors[instance.node().id().index() as usize];
            instance_ordinals[*cursor] = instance.id().index();
            *cursor = cursor
                .checked_add(1)
                .ok_or(RelationsQueryError::SizeOverflow)?;
        }
        drop(cursors);
        query.poll()?;
        Ok(SceneRelations {
            generation: self.generation(),
            parents,
            instance_offsets,
            instance_ordinals,
        })
    }
}

struct RelationsPlan {
    parents: CapacityPlan,
    offsets: CapacityPlan,
    ordinals: CapacityPlan,
    cursors: CapacityPlan,
}

impl RelationsPlan {
    fn new(nodes: usize, instances: usize) -> Result<Self, RelationsQueryError> {
        let offsets = nodes
            .checked_add(1)
            .ok_or(RelationsQueryError::SizeOverflow)?;
        Ok(Self {
            parents: CapacityPlan::for_type::<Option<NonZeroU32>>(0, 0, nodes)?,
            offsets: CapacityPlan::for_type::<usize>(0, 0, offsets)?,
            ordinals: CapacityPlan::for_type::<u32>(0, 0, instances)?,
            cursors: CapacityPlan::for_type::<usize>(0, 0, nodes)?,
        })
    }

    fn peak_bytes(&self) -> Result<u64, RelationsQueryError> {
        let bytes = [self.parents, self.offsets, self.ordinals, self.cursors]
            .into_iter()
            .try_fold(0_u64, |total, plan| {
                total
                    .checked_add(plan.envelope_delta_bytes()?)
                    .ok_or(RelationsQueryError::SizeOverflow)
            })?;
        usize::try_from(bytes).map_err(|_| RelationsQueryError::SizeOverflow)?;
        Ok(bytes)
    }
}

fn allocate<T>(plan: CapacityPlan, query: &Query<'_>) -> Result<Vec<T>, RelationsQueryError> {
    query.poll()?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| RelationsQueryError::AllocationFailed)?;
    plan.finish(values.len(), values.capacity())?;
    query.poll()?;
    Ok(values)
}

fn construction_work(
    nodes: usize,
    edges: usize,
    instances: usize,
) -> Result<u64, RelationsQueryError> {
    let nodes = u64::try_from(nodes).map_err(|_| RelationsQueryError::SizeOverflow)?;
    let edges = u64::try_from(edges).map_err(|_| RelationsQueryError::SizeOverflow)?;
    let instances = u64::try_from(instances).map_err(|_| RelationsQueryError::SizeOverflow)?;
    nodes
        .checked_mul(5)
        .and_then(|work| work.checked_add(edges))
        .and_then(|work| work.checked_add(instances.checked_mul(3)?))
        .and_then(|work| work.checked_add(1))
        .ok_or(RelationsQueryError::SizeOverflow)
}

fn storage_bytes<const N: usize>(
    capacities: [(usize, usize); N],
) -> Result<u64, RelationsQueryError> {
    let bytes = capacities
        .into_iter()
        .try_fold(0_usize, |total, (capacity, element_size)| {
            total.checked_add(capacity.checked_mul(element_size)?)
        })
        .ok_or(RelationsQueryError::SizeOverflow)?;
    u64::try_from(bytes).map_err(|_| RelationsQueryError::SizeOverflow)
}

struct Query<'a> {
    limits: RelationsQueryLimits,
    cancellation: Option<&'a CancellationToken>,
    work: u64,
}

impl<'a> Query<'a> {
    fn new(
        limits: RelationsQueryLimits,
        cancellation: Option<&'a CancellationToken>,
    ) -> Result<Self, RelationsQueryError> {
        let query = Self {
            limits,
            cancellation,
            work: 0,
        };
        query.poll()?;
        Ok(query)
    }

    fn poll(&self) -> Result<(), RelationsQueryError> {
        if self
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(RelationsQueryError::Cancelled);
        }
        Ok(())
    }

    fn step(&mut self) -> Result<(), RelationsQueryError> {
        self.poll()?;
        self.work = self
            .work
            .checked_add(1)
            .ok_or(RelationsQueryError::SizeOverflow)?;
        check_limit(RelationsQueryLimit::Work, self.work, self.limits.max_work)
    }

    fn bytes(&self, bytes: u64) -> Result<(), RelationsQueryError> {
        self.poll()?;
        check_limit(RelationsQueryLimit::Bytes, bytes, self.limits.max_bytes)
    }
}

fn check_limit(
    limit: RelationsQueryLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), RelationsQueryError> {
    if actual > maximum {
        return Err(RelationsQueryError::LimitExceeded {
            limit,
            actual,
            maximum,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
