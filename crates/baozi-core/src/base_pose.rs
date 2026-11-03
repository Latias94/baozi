//! Derived queries over authored node base poses, without Scene mutation or time sampling.

use std::{fmt, mem::size_of};

use baozi_operation::CancellationToken;
use thiserror::Error;

use crate::{
    Aabb, AttributeData, AttributeSemanticRef, AuthoredTransform, MeshInstanceId, MeshInstanceRef,
    NodeId, NumericConversionError, PrimitiveId, SceneEntityKind, SceneGenerationId,
    SceneLookupError, ValidatedScene,
};

const IDENTITY: [[f64; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Explicit bounds for a derived spatial query, independent of Scene storage accounting.
///
/// Work counts initialized transform slots, evaluated nodes, visited instances, effective morph
/// weights, primitives, inspected attribute descriptors, and transformed logical Position elements.
/// Each unit includes its fixed-size F64 arithmetic. Bytes bound the actual capacity of the owned
/// transform table at `size_of::<[[f64; 4]; 4]>()` bytes per slot, excluding its inline owner.
/// Traversal needs no separate queue; bounds queries allocate no heap storage or dense copies of
/// sparse attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpatialQueryLimits {
    max_work: u64,
    max_bytes: u64,
}

impl SpatialQueryLimits {
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

/// Which explicit query limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialQueryLimit {
    Work,
    Bytes,
}

/// Why a read-only spatial query could not produce its complete result.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SpatialQueryError {
    #[error(transparent)]
    Lookup(#[from] SceneLookupError),
    #[error("spatial query {limit:?} reached {actual}, above limit {maximum}")]
    LimitExceeded {
        limit: SpatialQueryLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("spatial query size or work cannot be represented")]
    SizeOverflow,
    #[error("spatial query allocation failed")]
    AllocationFailed,
    #[error("spatial query cancelled")]
    Cancelled,
    #[error("base-pose world transform for {node:?} cannot be represented: {source}")]
    TransformNumeric {
        node: NodeId,
        #[source]
        source: NumericConversionError,
    },
    #[error(
        "world Position {vertex} of {primitive:?} in {instance:?} cannot be represented: {source}"
    )]
    PositionNumeric {
        instance: MeshInstanceId,
        primitive: PrimitiveId,
        vertex: u32,
        #[source]
        source: NumericConversionError,
    },
    #[error("static bounds do not evaluate the skin bound to {instance:?}")]
    SkinUnsupported { instance: MeshInstanceId },
    #[error("static bounds do not evaluate nonzero morph weight {target} of {instance:?}")]
    NonzeroMorphWeight {
        instance: MeshInstanceId,
        target: u32,
    },
}

/// Owned, generation-bound local-to-world matrices for the authored node base pose.
///
/// Coordinates and units remain those of the originating Scene, including unknown declarations.
/// No clip is sampled, no skin is evaluated, and no geometry is changed. Construction visits the
/// existing node forest once; lookup is O(1). This table may outlive the Scene. After any Scene edit
/// publishes a new generation, derive a new table for queries on that output.
///
/// Authored matrix/TRS components remain F32 and widen exactly on read. Local-matrix derivation,
/// parent composition and point transformation all compute and retain F64, with no F32 rounding.
/// Nonfinite arithmetic and multiplication of nonzero operands rounding to zero are rejected;
/// finite F64 subnormals and exact cancellation to zero are accepted. F64 arithmetic is not exact
/// real arithmetic and does not preserve every residual smaller than an F64 result's precision.
pub struct BasePoseWorldTransforms {
    generation: SceneGenerationId,
    transforms: Vec<[[f64; 4]; 4]>,
}

impl fmt::Debug for BasePoseWorldTransforms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BasePoseWorldTransforms")
            .field("generation", &self.generation)
            .field("node_count", &self.node_count())
            .finish()
    }
}

impl BasePoseWorldTransforms {
    pub const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    pub fn node_count(&self) -> usize {
        self.transforms.len()
    }

    /// Actual owned transform capacity in bytes, excluding this table's inline fields.
    pub fn allocated_bytes(&self) -> usize {
        self.transforms.capacity() * size_of::<[[f64; 4]; 4]>()
    }

    /// Returns a node's local-to-world matrix in O(1), checking generation and range.
    ///
    /// The array is column-major with column-vector semantics: `p_world = matrix * p_local`.
    pub fn node_transform(&self, node: NodeId) -> Result<[[f64; 4]; 4], SceneLookupError> {
        self.check_generation(SceneEntityKind::Node, node.generation())?;
        self.transforms
            .get(node.index() as usize)
            .copied()
            .ok_or(SceneLookupError::OutOfRange {
                kind: SceneEntityKind::Node,
                length: self.node_count(),
            })
    }

    /// Bounds all base Position vertices of one placement in the Scene's world coordinates.
    ///
    /// Includes unreferenced vertices and implicit sparse defaults, independently of topology or
    /// index-stream emptiness. Transforms actual positions, so shear does not inflate the result
    /// by introducing nonexistent local-box corners. Empty geometry returns `None`; a point has
    /// a zero-size `Some(Aabb)`. F32 Position widens exactly; F64 Position retains its width.
    /// The AABB encloses computed F64 world vertices, without an additional real-arithmetic
    /// interval, outward rounding or rasterization guarantee.
    ///
    /// Any skin or nonzero effective morph weight is rejected, including on empty geometry.
    /// Instance morph overrides take precedence over mesh defaults. Clips may be present; only
    /// authored base transforms are used. There is no animation-time parameter.
    ///
    /// Traversal is linear in this instance's weights, primitives, attribute descriptors and
    /// logical Position count; it neither scans other instances nor allocates heap storage.
    pub fn instance_bounds(
        &self,
        instance: MeshInstanceRef<'_>,
        limits: SpatialQueryLimits,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Option<Aabb>, SpatialQueryError> {
        let mut query = Query::new(limits, cancellation)?;
        let bounds = self.bound_instance(instance, &mut query)?;
        query.poll()?;
        Ok(bounds)
    }

    /// Explicitly unions the bounds of every mesh instance in this exact Scene generation.
    ///
    /// Uninstanced meshes, cameras and lights do not contribute. One unsupported instance rejects
    /// the complete query. Shared geometry is visited once per placement, under its world matrix.
    /// The single work limit covers the complete scan; no partial bounds escape on failure.
    pub fn scene_bounds(
        &self,
        scene: &ValidatedScene,
        limits: SpatialQueryLimits,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Option<Aabb>, SpatialQueryError> {
        let mut query = Query::new(limits, cancellation)?;
        self.check_generation(SceneEntityKind::Node, scene.generation())?;
        let mut bounds = None;
        for instance in scene.mesh_instances() {
            if let Some(instance_bounds) = self.bound_instance(instance, &mut query)? {
                include_point(&mut bounds, instance_bounds.min);
                include_point(&mut bounds, instance_bounds.max);
            }
        }
        query.poll()?;
        Ok(bounds)
    }

    fn check_generation(
        &self,
        kind: SceneEntityKind,
        actual: SceneGenerationId,
    ) -> Result<(), SceneLookupError> {
        if actual != self.generation {
            return Err(SceneLookupError::GenerationMismatch {
                kind,
                expected: self.generation,
                actual,
            });
        }
        Ok(())
    }

    fn bound_instance(
        &self,
        instance: MeshInstanceRef<'_>,
        query: &mut Query<'_>,
    ) -> Result<Option<Aabb>, SpatialQueryError> {
        query.step()?;
        let id = instance.id();
        self.check_generation(SceneEntityKind::MeshInstance, id.generation())?;
        if instance.skin().is_some() {
            return Err(SpatialQueryError::SkinUnsupported { instance: id });
        }
        let mesh = instance.mesh();
        let weights = instance
            .morph_weights()
            .unwrap_or_else(|| mesh.default_morph_weights());
        for (target, &weight) in weights.iter().enumerate() {
            query.step()?;
            if weight != 0.0 {
                return Err(SpatialQueryError::NonzeroMorphWeight {
                    instance: id,
                    target: target as u32,
                });
            }
        }
        let world = self.node_transform(instance.node().id())?;
        let mut bounds = None;
        for primitive in mesh.primitives() {
            query.step()?;
            let mut positions = None;
            for attribute in primitive.attributes() {
                query.step()?;
                if attribute.semantic() == AttributeSemanticRef::Position {
                    positions = Some(attribute);
                    break;
                }
            }
            let Some(positions) = positions else {
                // Central validation permits missing Position only for zero vertices.
                continue;
            };
            let mut include = |vertex: usize, position: [f64; 3]| {
                query.step()?;
                let point = transform_point(world, position).map_err(|source| {
                    SpatialQueryError::PositionNumeric {
                        instance: id,
                        primitive: primitive.id(),
                        vertex: vertex as u32,
                        source,
                    }
                })?;
                include_point(&mut bounds, point);
                Ok::<_, SpatialQueryError>(())
            };
            match positions.data() {
                AttributeData::DenseF32 { values, .. } => {
                    for (vertex, position) in values.chunks_exact(3).enumerate() {
                        include(
                            vertex,
                            [position[0], position[1], position[2]].map(f64::from),
                        )?;
                    }
                }
                AttributeData::DenseF64 { values, .. } => {
                    for (vertex, position) in values.chunks_exact(3).enumerate() {
                        include(vertex, [position[0], position[1], position[2]])?;
                    }
                }
                AttributeData::SparseF32 {
                    logical_count,
                    indices,
                    values,
                    default,
                    ..
                } => {
                    let mut explicit = 0;
                    for vertex in 0..logical_count {
                        let position = if indices.get(explicit) == Some(&vertex) {
                            let start = explicit * 3;
                            explicit += 1;
                            &values[start..start + 3]
                        } else {
                            default
                        };
                        include(
                            vertex as usize,
                            [position[0], position[1], position[2]].map(f64::from),
                        )?;
                    }
                }
                AttributeData::SparseF64 {
                    logical_count,
                    indices,
                    values,
                    default,
                    ..
                } => {
                    let mut explicit = 0;
                    for vertex in 0..logical_count {
                        let position = if indices.get(explicit) == Some(&vertex) {
                            let start = explicit * 3;
                            explicit += 1;
                            &values[start..start + 3]
                        } else {
                            default
                        };
                        include(vertex as usize, [position[0], position[1], position[2]])?;
                    }
                }
                _ => unreachable!("validated Position has F32x3 or F64x3 storage"),
            }
        }
        Ok(bounds)
    }
}

impl ValidatedScene {
    /// Derives all authored base-pose world matrices in O(node count) time and space.
    ///
    /// Traverses the retained forest, composing `parent_world * local`, with no dependence on
    /// source node order. The internal synthetic root contributes identity. The query preserves
    /// the Scene and its generation. Cancellation is checked before allocation, once per work
    /// unit, and before returning; the complete table is discarded on any failure.
    ///
    /// Construction charges two work units per node and the actual F64 transform capacity in
    /// bytes. The requested capacity is checked before allocation and the allocator's returned
    /// capacity before initialization. No traversal stack or attribute copies are allocated.
    pub fn base_pose_world_transforms(
        &self,
        limits: SpatialQueryLimits,
        cancellation: Option<&CancellationToken>,
    ) -> Result<BasePoseWorldTransforms, SpatialQueryError> {
        let mut query = Query::new(limits, cancellation)?;
        let count = self.node_count();
        let work = u64::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(2))
            .ok_or(SpatialQueryError::SizeOverflow)?;
        check_limit(SpatialQueryLimit::Work, work, limits.max_work)?;
        query.bytes(table_bytes(count)?)?;
        let mut transforms = Vec::new();
        transforms
            .try_reserve_exact(count)
            .map_err(|_| SpatialQueryError::AllocationFailed)?;
        query.bytes(table_bytes(transforms.capacity())?)?;
        for _ in 0..count {
            query.step()?;
            transforms.push(IDENTITY);
        }
        let nodes = self.bind_node_query();
        let roots = nodes.roots();
        #[cfg(test)]
        query_boundary_for_test(QueryBoundary::RootReferences);
        for root in roots {
            query.step()?;
            transforms[root.id().index() as usize] = checked_local(nodes.authored_transform(root))
                .map_err(|source| SpatialQueryError::TransformNumeric {
                    node: root.id(),
                    source,
                })?;
        }
        // Central node validation guarantees canonical parent-before-child order. This is an
        // internal retained-table invariant, independent of authoring IDs and public order policy.
        for node in self.nodes() {
            query.poll()?;
            let parent = transforms[node.id().index() as usize];
            let children = nodes.children(node);
            #[cfg(test)]
            query_boundary_for_test(QueryBoundary::ChildReferences);
            for child in children {
                query.step()?;
                let world = checked_local(nodes.authored_transform(child))
                    .and_then(|local| compose(parent, local))
                    .map_err(|source| SpatialQueryError::TransformNumeric {
                        node: child.id(),
                        source,
                    })?;
                transforms[child.id().index() as usize] = world;
            }
        }
        #[cfg(test)]
        query_boundary_for_test(QueryBoundary::Return);
        query.poll()?;
        Ok(BasePoseWorldTransforms {
            generation: self.generation(),
            transforms,
        })
    }
}

fn table_bytes(transforms: usize) -> Result<u64, SpatialQueryError> {
    let bytes = transforms
        .checked_mul(size_of::<[[f64; 4]; 4]>())
        .ok_or(SpatialQueryError::SizeOverflow)?;
    u64::try_from(bytes).map_err(|_| SpatialQueryError::SizeOverflow)
}

struct Query<'a> {
    limits: SpatialQueryLimits,
    cancellation: Option<&'a CancellationToken>,
    work: u64,
}

impl<'a> Query<'a> {
    fn new(
        limits: SpatialQueryLimits,
        cancellation: Option<&'a CancellationToken>,
    ) -> Result<Self, SpatialQueryError> {
        let query = Self {
            limits,
            cancellation,
            work: 0,
        };
        query.poll()?;
        Ok(query)
    }

    fn poll(&self) -> Result<(), SpatialQueryError> {
        if self
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(SpatialQueryError::Cancelled);
        }
        Ok(())
    }

    fn step(&mut self) -> Result<(), SpatialQueryError> {
        self.poll()?;
        self.work = self
            .work
            .checked_add(1)
            .ok_or(SpatialQueryError::SizeOverflow)?;
        check_limit(SpatialQueryLimit::Work, self.work, self.limits.max_work)
    }

    fn bytes(&self, bytes: u64) -> Result<(), SpatialQueryError> {
        self.poll()?;
        check_limit(SpatialQueryLimit::Bytes, bytes, self.limits.max_bytes)
    }
}

fn check_limit(
    limit: SpatialQueryLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), SpatialQueryError> {
    if actual > maximum {
        return Err(SpatialQueryError::LimitExceeded {
            limit,
            actual,
            maximum,
        });
    }
    Ok(())
}

fn checked_finite(value: f64) -> Result<f64, NumericConversionError> {
    if !value.is_finite() {
        return Err(NumericConversionError::NonFiniteOutput);
    }
    Ok(value)
}

fn checked_product(left: f64, right: f64) -> Result<f64, NumericConversionError> {
    let product = checked_finite(left * right)?;
    if left != 0.0 && right != 0.0 && product == 0.0 {
        return Err(NumericConversionError::UnderflowToZero);
    }
    Ok(product)
}

fn checked_local(transform: AuthoredTransform) -> Result<[[f64; 4]; 4], NumericConversionError> {
    if let Some(matrix) = transform.authored_matrix() {
        return Ok(matrix.cols.map(|column| column.map(f64::from)));
    }
    let trs = transform
        .authored_trs()
        .expect("authored transform is matrix or TRS");
    let columns = crate::math::trs_columns_f64(trs.translation(), trs.rotation(), trs.scale());
    // Every factor is authored F32, so this fixed TRS expression cannot underflow F64.
    for &component in columns.iter().flatten() {
        checked_finite(component)?;
    }
    Ok(columns)
}

fn compose(
    parent: [[f64; 4]; 4],
    local: [[f64; 4]; 4],
) -> Result<[[f64; 4]; 4], NumericConversionError> {
    let mut world = IDENTITY;
    for (column, output) in world.iter_mut().enumerate() {
        for (row, cell) in output.iter_mut().take(3).enumerate() {
            let mut sum = 0.0;
            for inner in 0..4 {
                let product = checked_product(parent[inner][row], local[column][inner])?;
                sum = checked_finite(sum + product)?;
            }
            *cell = sum;
        }
    }
    Ok(world)
}

fn transform_point(
    matrix: [[f64; 4]; 4],
    position: [f64; 3],
) -> Result<[f64; 3], NumericConversionError> {
    let mut output = [0.0; 3];
    for (row, cell) in output.iter_mut().enumerate() {
        let mut sum = 0.0;
        for (column, &value) in position.iter().enumerate() {
            sum = checked_finite(sum + checked_product(matrix[column][row], value)?)?;
        }
        *cell = checked_finite(sum + matrix[3][row])?;
    }
    Ok(output)
}

fn include_point(bounds: &mut Option<Aabb>, point: [f64; 3]) {
    if let Some(bounds) = bounds {
        for (axis, value) in point.into_iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(value);
            bounds.max[axis] = bounds.max[axis].max(value);
        }
    } else {
        *bounds = Some(Aabb {
            min: point,
            max: point,
        });
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryBoundary {
    RootReferences,
    ChildReferences,
    Return,
}

#[cfg(test)]
std::thread_local! {
    static CANCEL_AT_QUERY_BOUNDARY: std::cell::Cell<Option<QueryBoundary>> =
        const { std::cell::Cell::new(None) };
    static QUERY_BOUNDARY_CANCELLATION: std::cell::RefCell<Option<CancellationToken>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
#[must_use]
pub(crate) struct QueryCancellationHook;

#[cfg(test)]
pub(crate) fn cancel_at_query_boundary_for_test(
    boundary: QueryBoundary,
    cancellation: CancellationToken,
) -> QueryCancellationHook {
    CANCEL_AT_QUERY_BOUNDARY.with(|pending| {
        assert!(
            pending.get().is_none(),
            "query cancellation hook is already installed"
        );
    });
    QUERY_BOUNDARY_CANCELLATION.with(|pending| *pending.borrow_mut() = Some(cancellation));
    CANCEL_AT_QUERY_BOUNDARY.with(|pending| pending.set(Some(boundary)));
    QueryCancellationHook
}

#[cfg(test)]
fn query_boundary_for_test(boundary: QueryBoundary) {
    let reached = CANCEL_AT_QUERY_BOUNDARY.with(|pending| {
        let reached = pending.get() == Some(boundary);
        if reached {
            pending.set(None);
        }
        reached
    });
    if reached {
        let cancellation = QUERY_BOUNDARY_CANCELLATION.with(|pending| pending.borrow_mut().take());
        cancellation
            .expect("installed query cancellation token")
            .cancel();
    }
}

#[cfg(test)]
impl Drop for QueryCancellationHook {
    fn drop(&mut self) {
        CANCEL_AT_QUERY_BOUNDARY.with(|pending| pending.set(None));
        QUERY_BOUNDARY_CANCELLATION.with(|pending| *pending.borrow_mut() = None);
    }
}

#[cfg(test)]
mod tests;
