#![allow(
    clippy::result_large_err,
    reason = "node authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

pub(super) mod animation;
pub(super) mod appearance;
pub(super) mod deformation;
pub(super) mod geometry;
pub(super) mod profile;
pub(super) mod spatial;

use std::error::Error;
use std::fmt;

use baozi_operation::{CapacityPlan, CapacityPlanError};
use thiserror::Error;

use crate::entity::{AuthoredTransformKind, EntityNameError, validate_entity_name_str};
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork};

use super::super::schema::{NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_TRS_SCALAR_COUNT};
use super::super::typed_storage::build::NodeStoragePlan;
use super::control::CanonicalAuthoringControl;
use super::input::{NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, Presence};

const MATRIX_SCALARS_PER_NODE: u64 = NODE_SCENE_MATRIX_SCALAR_COUNT as u64;
const TRS_SCALARS_PER_NODE: u64 = NODE_SCENE_TRS_SCALAR_COUNT as u64;
#[derive(Clone, Copy)]
pub(super) struct NodeShapeScratchEnvelope {
    peak_bytes: u64,
    retained_bytes: u64,
}

impl NodeShapeScratchEnvelope {
    pub(super) const fn peak_bytes(self) -> u64 {
        self.peak_bytes
    }

    pub(super) const fn retained_bytes(self) -> u64 {
        self.retained_bytes
    }
}

pub(super) fn node_shape_scratch_envelope(
    input: NodeAuthoringBatch<'_>,
) -> Result<NodeShapeScratchEnvelope, NodeAuthoringSemanticError> {
    let count = input.nodes().len();
    let canonical_order = scratch_envelope(
        CapacityPlan::for_type::<usize>(0, 0, count).map_err(|cause| {
            NodeAuthoringSemanticError::ScratchCapacity {
                resource: "canonical node order",
                cause,
            }
        })?,
        "canonical node order",
    )?;
    let canonical_index = scratch_envelope(
        CapacityPlan::for_type::<u32>(0, 0, count).map_err(|cause| {
            NodeAuthoringSemanticError::ScratchCapacity {
                resource: "canonical index by local node",
                cause,
            }
        })?,
        "canonical index by local node",
    )?;
    let visit_state = scratch_envelope(
        CapacityPlan::for_type::<VisitState>(0, 0, count).map_err(|cause| {
            NodeAuthoringSemanticError::ScratchCapacity {
                resource: "node graph visit state",
                cause,
            }
        })?,
        "node graph visit state",
    )?;
    let child_ranges = scratch_envelope(
        CapacityPlan::for_type::<ChildRange>(0, 0, count).map_err(|cause| {
            NodeAuthoringSemanticError::ScratchCapacity {
                resource: "child ranges by local node",
                cause,
            }
        })?,
        "child ranges by local node",
    )?;
    let ordered_children = scratch_envelope(
        CapacityPlan::for_type::<usize>(0, 0, count).map_err(|cause| {
            NodeAuthoringSemanticError::ScratchCapacity {
                resource: "ordered child table",
                cause,
            }
        })?,
        "ordered child table",
    )?;
    let peak_bytes = checked_sum_bytes(
        [
            canonical_order,
            canonical_index,
            visit_state,
            child_ranges,
            ordered_children,
        ],
        "node shape scratch envelope",
    )?;
    let retained_bytes = checked_sum_bytes(
        [canonical_order, canonical_index, child_ranges],
        "retained node shape scratch envelope",
    )?;
    Ok(NodeShapeScratchEnvelope {
        peak_bytes,
        retained_bytes,
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeAuthoringLimit {
    Nodes,
    Edges,
    Depth,
    SemanticCells,
    References,
    CanonicalBytes,
    Work,
    ScratchBytes,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeAuthoringSemanticError {
    #[error("the authored root list is absent")]
    RootsAbsent,
    #[error(
        "{authored_nodes} authored nodes plus the synthetic root exceed \
         the u32 canonical node domain"
    )]
    NodeDomainTooLarge { authored_nodes: usize },
    #[error("{limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: NodeAuthoringLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("node authoring accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error(
        "authored transform scalar count {scalar_count} exceeds the u32 canonical sequence domain"
    )]
    TransformDomainTooLarge { scalar_count: u64 },
    #[error(
        "root reference {root} at roots[{root_index}] is outside authored node count {node_count}"
    )]
    RootOutOfRange {
        root_index: usize,
        root: u32,
        node_count: usize,
    },
    #[error("root reference {root} repeats at roots[{duplicate_index}]")]
    DuplicateRoot { root: u32, duplicate_index: usize },
    #[error("declared root node {root} has parent {parent}")]
    RootHasParent { root: usize, parent: u32 },
    #[error("parentless node {node} is absent from the explicit root list")]
    MissingRoot { node: usize },
    #[error("parent reference {parent} on node {node} is outside authored node count {node_count}")]
    ParentOutOfRange {
        node: usize,
        parent: u32,
        node_count: usize,
    },
    #[error("node {node} references itself as parent")]
    SelfParent { node: usize },
    #[error("node {node} has an invalid display name: {cause}")]
    InvalidName {
        node: usize,
        #[source]
        cause: EntityNameError,
    },
    #[error("node parent graph contains a cycle through node {node}")]
    Cycle { node: usize },
    #[error(
        "child ordinal {ordinal} on node {node} exceeds parent {parent} child count {child_count}"
    )]
    ChildOrdinalOutOfRange {
        node: u32,
        parent: u32,
        ordinal: u32,
        child_count: u32,
    },
    #[error(
        "child ordinal {ordinal} on parent {parent} is assigned to both node {first} and node {duplicate}"
    )]
    DuplicateChildOrdinal {
        parent: u32,
        ordinal: u32,
        first: u32,
        duplicate: u32,
    },
    #[error("scratch capacity planning failed for {resource}: {cause}")]
    ScratchCapacity {
        resource: &'static str,
        #[source]
        cause: CapacityPlanError,
    },
    #[error("scratch allocation failed for {resource}")]
    ScratchAllocation { resource: &'static str },
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeAuthoringFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Semantic(#[from] NodeAuthoringSemanticError),
    #[error("node authoring validation control failed")]
    Control(#[source] E),
}

/// Operation-accounted node semantics paired with the exact authored view that
/// produced them.
///
/// The private constructor keeps source values and compiled facts from sibling
/// validation runs structurally inseparable. Consumers that need both must use
/// this paired value rather than accepting two independently supplied inputs.
#[must_use = "dropping the validated batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedNodeBatch<'input, R> {
    compiled: NodeValidationStorage<R>,
    input: NodeAuthoringBatch<'input>,
}

/// Canonical index zero is the implicit synthetic root. `canonical_order`
/// contains authored local indices only, and `canonical_index_by_local` maps
/// each local index to a canonical index in `1..canonical_node_count`.
#[must_use = "dropping the compiled plan releases its scratch reservation"]
struct NodeValidationStorage<R> {
    canonical_node_count: u64,
    source_root_count: u32,
    reference_count: u64,
    matrix_scalar_count: u64,
    trs_scalar_count: u64,
    node_name_byte_count: u64,
    node_name_reference_count: u64,
    canonical_bytes: u64,
    validation_work: u64,
    scratch_bytes: u64,
    // Compiled storage must be destroyed before the reservation covering it.
    canonical_order: Vec<usize>,
    canonical_index_by_local: Vec<u32>,
    child_ranges_by_local: Vec<ChildRange>,
    #[cfg(test)]
    storage_drop_probe: Option<StorageDropProbe>,
    reservation: R,
}

impl<'input, R> ValidatedNodeBatch<'input, R> {
    const fn new(input: NodeAuthoringBatch<'input>, compiled: NodeValidationStorage<R>) -> Self {
        Self { compiled, input }
    }

    pub(in crate::canonical_scene::store) const fn input(&self) -> NodeAuthoringBatch<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn canonical_node_count(&self) -> u64 {
        self.compiled.canonical_node_count
    }

    pub(in crate::canonical_scene::store) fn authored_node_count(&self) -> usize {
        self.compiled.canonical_index_by_local.len()
    }

    pub(in crate::canonical_scene::store) const fn source_root_count(&self) -> u32 {
        self.compiled.source_root_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.compiled.reference_count
    }

    pub(in crate::canonical_scene::store) const fn matrix_scalar_count(&self) -> u64 {
        self.compiled.matrix_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn trs_scalar_count(&self) -> u64 {
        self.compiled.trs_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn node_name_byte_count(&self) -> u64 {
        self.compiled.node_name_byte_count
    }

    pub(in crate::canonical_scene::store) const fn node_name_reference_count(&self) -> u64 {
        self.compiled.node_name_reference_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.compiled.canonical_bytes
    }

    pub(in crate::canonical_scene::store) const fn semantic_cell_count(&self) -> u64 {
        self.compiled.matrix_scalar_count + self.compiled.trs_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.compiled.validation_work
    }

    pub(in crate::canonical_scene::store) const fn scratch_bytes(&self) -> u64 {
        self.compiled.scratch_bytes
    }

    pub(in crate::canonical_scene::store) fn canonical_order(&self) -> &[usize] {
        &self.compiled.canonical_order
    }

    pub(in crate::canonical_scene::store) fn canonical_index_by_local(&self) -> &[u32] {
        &self.compiled.canonical_index_by_local
    }

    pub(in crate::canonical_scene::store) fn child_count_by_local(
        &self,
        local: usize,
    ) -> Option<u32> {
        self.compiled
            .child_ranges_by_local
            .get(local)
            .map(|range| range.count)
    }

    #[cfg(test)]
    pub(super) fn install_storage_drop_probe(&mut self, stage: std::rc::Rc<std::cell::Cell<u8>>) {
        self.compiled.storage_drop_probe = Some(StorageDropProbe(stage));
    }

    #[cfg(test)]
    pub(super) fn into_compiled_facts_for_test(self) -> (usize, u32, Vec<usize>, Vec<u32>, R) {
        let NodeValidationStorage {
            canonical_index_by_local,
            canonical_order,
            child_ranges_by_local,
            source_root_count,
            reservation,
            ..
        } = self.compiled;
        let authored_node_count = canonical_index_by_local.len();
        drop(child_ranges_by_local);
        (
            authored_node_count,
            source_root_count,
            canonical_order,
            canonical_index_by_local,
            reservation,
        )
    }
}

impl<R> fmt::Debug for NodeValidationStorage<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeValidationStorage")
            .field("canonical_node_count", &self.canonical_node_count)
            .field("source_root_count", &self.source_root_count)
            .field("reference_count", &self.reference_count)
            .field("matrix_scalar_count", &self.matrix_scalar_count)
            .field("trs_scalar_count", &self.trs_scalar_count)
            .field("node_name_byte_count", &self.node_name_byte_count)
            .field("node_name_reference_count", &self.node_name_reference_count)
            .field("canonical_order", &self.canonical_order)
            .field("canonical_index_by_local", &self.canonical_index_by_local)
            .field("child_ranges_by_local", &self.child_ranges_by_local)
            .field("canonical_bytes", &self.canonical_bytes)
            .field("validation_work", &self.validation_work)
            .field("scratch_bytes", &self.scratch_bytes)
            .field("reservation", &"<held>")
            .finish()
    }
}

impl<R> fmt::Debug for ValidatedNodeBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedNodeBatch")
            .field("input", &self.input)
            .field("compiled", &self.compiled)
            .finish()
    }
}

#[cfg(test)]
struct StorageDropProbe(std::rc::Rc<std::cell::Cell<u8>>);

#[cfg(test)]
impl Drop for StorageDropProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.0.replace(1),
            0,
            "compiled node storage must begin the coupled drop sequence"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Unseen,
    Visiting,
    Depth(u32),
}

enum ParentTraversalStep {
    Resolved(u32),
    Continue(usize),
}

#[derive(Debug, Clone, Copy, Default)]
struct ChildRange {
    start: u32,
    count: u32,
}

struct ValidationStorage<R> {
    canonical_order: Vec<usize>,
    canonical_index_by_local: Vec<u32>,
    visit_state_by_local: Vec<VisitState>,
    child_ranges_by_local: Vec<ChildRange>,
    ordered_children: Vec<usize>,
    // Every allocation above must drop before its accounting token.
    reservation: R,
}

type ValidationResult<T, C> =
    Result<T, NodeAuthoringFailure<<C as CanonicalAuthoringControl>::Error>>;

struct ValidationContext<'control, C: CanonicalAuthoringControl> {
    control: &'control mut C,
    limits: SceneValidationLimits,
    work: u64,
    work_since_poll: u64,
    edges: u64,
    references: u64,
    semantic_cells: u64,
    matrix_scalars: u64,
    trs_scalars: u64,
}

impl<'control, C: CanonicalAuthoringControl> ValidationContext<'control, C> {
    const fn new(limits: SceneValidationLimits, control: &'control mut C) -> Self {
        Self {
            control,
            limits,
            work: 0,
            work_since_poll: 0,
            edges: 0,
            references: 0,
            semantic_cells: 0,
            matrix_scalars: 0,
            trs_scalars: 0,
        }
    }

    fn poll_now(&mut self) -> ValidationResult<(), C> {
        self.control.poll().map_err(NodeAuthoringFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn finish(&mut self) -> ValidationResult<(), C> {
        self.poll_now()
    }

    fn ensure_work_capacity(&self, amount: u64) -> ValidationResult<u64, C> {
        let final_work = self.work.checked_add(amount).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work",
            },
        )?;
        enforce_limit::<C::Error>(NodeAuthoringLimit::Work, final_work, self.limits.max_work())?;
        Ok(final_work)
    }

    fn run_unit<R>(
        &mut self,
        kind: SceneValidationWork,
        action: impl FnMut() -> R,
    ) -> ValidationResult<R, C> {
        let final_work = self.ensure_work_capacity(1)?;
        let outcome = self
            .control
            .run_unit(kind, action)
            .map_err(NodeAuthoringFailure::Control)?;
        self.work = final_work;
        self.work_since_poll = self.work_since_poll.checked_add(1).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work poll interval",
            },
        )?;
        if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
            self.poll_now()?;
        }
        Ok(outcome)
    }

    fn run_semantic_unit<R>(
        &mut self,
        kind: SceneValidationWork,
        action: impl FnMut() -> Result<R, NodeAuthoringSemanticError>,
    ) -> ValidationResult<R, C> {
        self.run_unit(kind, action)?
            .map_err(NodeAuthoringFailure::Semantic)
    }

    fn add_relation(&mut self) -> ValidationResult<(), C> {
        let edges = self.edges;
        let maximum_edges = self.limits.max_edges();
        self.edges = self.run_semantic_unit(SceneValidationWork::GraphEdge, || {
            let next = edges
                .checked_add(1)
                .ok_or(NodeAuthoringSemanticError::AccountingOverflow { dimension: "edges" })?;
            check_limit(NodeAuthoringLimit::Edges, next, maximum_edges)?;
            Ok(next)
        })?;

        let references = self.references;
        let maximum_references = self.limits.max_references();
        self.references = self.run_semantic_unit(SceneValidationWork::Reference, || {
            let next = references.checked_add(1).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "references",
                },
            )?;
            check_limit(NodeAuthoringLimit::References, next, maximum_references)?;
            Ok(next)
        })?;
        Ok(())
    }

    fn add_transform(&mut self, kind: AuthoredTransformKind) -> ValidationResult<(), C> {
        let scalar_count = match kind {
            AuthoredTransformKind::Matrix => MATRIX_SCALARS_PER_NODE,
            AuthoredTransformKind::Trs => TRS_SCALARS_PER_NODE,
        };
        self.ensure_work_capacity(scalar_count)?;

        let mut matrix_scalars = self.matrix_scalars;
        let mut trs_scalars = self.trs_scalars;
        let mut semantic_cells = self.semantic_cells;
        let maximum_semantic_cells = self.limits.max_payload_cells();
        for _ in 0..scalar_count {
            (matrix_scalars, trs_scalars, semantic_cells) =
                self.run_semantic_unit(SceneValidationWork::PayloadCell, || {
                    let next_matrix_scalars = if kind == AuthoredTransformKind::Matrix {
                        matrix_scalars.checked_add(1).ok_or(
                            NodeAuthoringSemanticError::AccountingOverflow {
                                dimension: "matrix scalars",
                            },
                        )?
                    } else {
                        matrix_scalars
                    };
                    let next_trs_scalars = if kind == AuthoredTransformKind::Trs {
                        trs_scalars.checked_add(1).ok_or(
                            NodeAuthoringSemanticError::AccountingOverflow {
                                dimension: "TRS scalars",
                            },
                        )?
                    } else {
                        trs_scalars
                    };
                    let next_semantic_cells = semantic_cells.checked_add(1).ok_or(
                        NodeAuthoringSemanticError::AccountingOverflow {
                            dimension: "semantic cells",
                        },
                    )?;
                    if next_semantic_cells > u64::from(u32::MAX) {
                        return Err(NodeAuthoringSemanticError::TransformDomainTooLarge {
                            scalar_count: next_semantic_cells,
                        });
                    }
                    check_limit(
                        NodeAuthoringLimit::SemanticCells,
                        next_semantic_cells,
                        maximum_semantic_cells,
                    )?;
                    Ok((next_matrix_scalars, next_trs_scalars, next_semantic_cells))
                })?;
        }
        self.matrix_scalars = matrix_scalars;
        self.trs_scalars = trs_scalars;
        self.semantic_cells = semantic_cells;
        Ok(())
    }

    fn scratch_plan<T>(
        &self,
        count: usize,
        resource: &'static str,
    ) -> Result<CapacityPlan, NodeAuthoringSemanticError> {
        CapacityPlan::for_type::<T>(0, 0, count)
            .map_err(|cause| NodeAuthoringSemanticError::ScratchCapacity { resource, cause })
    }

    fn reserve_scratch(&mut self, bytes: u64) -> ValidationResult<C::Reservation, C> {
        enforce_limit::<C::Error>(
            NodeAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        let reservation = self
            .control
            .reserve_scratch(bytes)
            .map_err(NodeAuthoringFailure::Control)?;
        self.control
            .validate_scratch(&reservation)
            .map_err(NodeAuthoringFailure::Control)?;
        Ok(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> ValidationResult<(), C> {
        enforce_limit::<C::Error>(
            NodeAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.control
            .resize_scratch(reservation, bytes)
            .map_err(NodeAuthoringFailure::Control)
    }

    fn allocate_scratch<T>(
        &self,
        plan: CapacityPlan,
        resource: &'static str,
    ) -> Result<(Vec<T>, u64), NodeAuthoringSemanticError> {
        plan.verify_preflight(0, 0)
            .map_err(|cause| NodeAuthoringSemanticError::ScratchCapacity { resource, cause })?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| NodeAuthoringSemanticError::ScratchAllocation { resource })?;
        let actual_bytes = plan
            .finish(values.len(), values.capacity())
            .map_err(|cause| NodeAuthoringSemanticError::ScratchCapacity { resource, cause })?;
        Ok((values, actual_bytes))
    }

    fn fill_scratch<T: Copy>(
        &mut self,
        values: &mut Vec<T>,
        count: usize,
        value: T,
    ) -> ValidationResult<(), C> {
        let remaining = count.checked_sub(values.len()).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "scratch fill length",
            },
        )?;
        self.ensure_work_capacity(usize_to_u64(remaining, "validation work")?)?;
        while values.len() < count {
            self.run_unit(SceneValidationWork::Traversal, || values.push(value))?;
        }
        Ok(())
    }
}

pub(in crate::canonical_scene::store) fn validate_node_batch_with_control<'input, C>(
    input: NodeAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedNodeBatch<'input, C::Reservation>, NodeAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    validate_node_batch_inner(input, limits, control)
}

fn validate_node_batch_inner<'input, C>(
    input: NodeAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedNodeBatch<'input, C::Reservation>, NodeAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    let mut context = ValidationContext::new(limits, control);
    context.poll_now()?;

    let nodes = input.nodes();
    let maximum_authored_nodes = (u32::MAX as usize).saturating_sub(1);
    if nodes.len() > maximum_authored_nodes {
        return Err(NodeAuthoringSemanticError::NodeDomainTooLarge {
            authored_nodes: nodes.len(),
        }
        .into());
    }
    let authored_node_count = usize_to_u64(nodes.len(), "authored nodes")?;
    let canonical_node_count = authored_node_count.checked_add(1).ok_or(
        NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical nodes",
        },
    )?;
    enforce_limit::<C::Error>(
        NodeAuthoringLimit::Nodes,
        canonical_node_count,
        limits.max_nodes(),
    )?;
    let maximum_depth = context.limits.max_depth();
    let roots = context.run_semantic_unit(SceneValidationWork::GraphNode, || {
        let roots = match input.roots() {
            Presence::Absent => return Err(NodeAuthoringSemanticError::RootsAbsent),
            Presence::Present(roots) => roots,
        };
        check_limit(NodeAuthoringLimit::Depth, 1, maximum_depth)?;
        Ok(roots)
    })?;
    let source_root_count = usize_to_u32(roots.len(), "source roots")?;

    let order_plan = context.scratch_plan::<usize>(nodes.len(), "canonical node order")?;
    let index_plan = context.scratch_plan::<u32>(nodes.len(), "canonical index by local node")?;
    let visit_plan = context.scratch_plan::<VisitState>(nodes.len(), "node graph visit state")?;
    let child_range_plan =
        context.scratch_plan::<ChildRange>(nodes.len(), "child ranges by local node")?;
    let ordered_child_plan = context.scratch_plan::<usize>(nodes.len(), "ordered child table")?;
    let scratch_envelope = checked_sum_bytes(
        [
            scratch_envelope(order_plan, "canonical node order")?,
            scratch_envelope(index_plan, "canonical index by local node")?,
            scratch_envelope(visit_plan, "node graph visit state")?,
            scratch_envelope(child_range_plan, "child ranges by local node")?,
            scratch_envelope(ordered_child_plan, "ordered child table")?,
        ],
        "scratch envelope",
    )?;

    let mut reservation = context.reserve_scratch(scratch_envelope)?;
    let (canonical_order, order_bytes) =
        context.allocate_scratch(order_plan, "canonical node order")?;
    let (canonical_index_by_local, index_bytes) =
        context.allocate_scratch(index_plan, "canonical index by local node")?;
    let (visit_state_by_local, visit_bytes) =
        context.allocate_scratch(visit_plan, "node graph visit state")?;
    let (child_ranges_by_local, child_range_bytes) =
        context.allocate_scratch(child_range_plan, "child ranges by local node")?;
    let (ordered_children, ordered_child_bytes) =
        context.allocate_scratch(ordered_child_plan, "ordered child table")?;
    let scratch_actual = checked_sum_bytes(
        [
            order_bytes,
            index_bytes,
            visit_bytes,
            child_range_bytes,
            ordered_child_bytes,
        ],
        "scratch allocation",
    )?;
    context.resize_scratch(&mut reservation, scratch_actual)?;

    let mut storage = ValidationStorage {
        canonical_order,
        canonical_index_by_local,
        visit_state_by_local,
        child_ranges_by_local,
        ordered_children,
        reservation,
    };
    context.fill_scratch(
        &mut storage.visit_state_by_local,
        nodes.len(),
        VisitState::Unseen,
    )?;

    for (root_index, &root) in roots.iter().enumerate() {
        let maximum_depth = context.limits.max_depth();
        context.run_semantic_unit(SceneValidationWork::GraphEdge, || {
            let local = local_index(root)?;
            if local >= nodes.len() {
                return Err(NodeAuthoringSemanticError::RootOutOfRange {
                    root_index,
                    root: root.index(),
                    node_count: nodes.len(),
                });
            }
            let state = storage.visit_state_by_local.get(local).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "root visit state",
                },
            )?;
            if !matches!(state, VisitState::Unseen) {
                return Err(NodeAuthoringSemanticError::DuplicateRoot {
                    root: root.index(),
                    duplicate_index: root_index,
                });
            }
            if let Presence::Present(parent) = nodes[local].parent() {
                return Err(NodeAuthoringSemanticError::RootHasParent {
                    root: local,
                    parent: parent.node().index(),
                });
            }
            check_limit(NodeAuthoringLimit::Depth, 2, maximum_depth)?;
            *storage.visit_state_by_local.get_mut(local).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "root visit state",
                },
            )? = VisitState::Depth(2);
            Ok(())
        })?;
        context.add_relation()?;
    }

    let mut node_name_byte_count = 0_u64;
    let mut node_name_reference_count = 0_u64;
    for (node_index, node) in nodes.iter().copied().enumerate() {
        let name_byte_count = context.run_semantic_unit(SceneValidationWork::Entity, || {
            let Presence::Present(name) = node.name() else {
                return Ok(0_u64);
            };
            validate_entity_name_str(name).map_err(|cause| {
                NodeAuthoringSemanticError::InvalidName {
                    node: node_index,
                    cause,
                }
            })?;
            u64::try_from(name.len()).map_err(|_| NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "node name bytes",
            })
        })?;
        if name_byte_count != 0 {
            node_name_reference_count = node_name_reference_count.checked_add(1).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "node name references",
                },
            )?;
        }
        node_name_byte_count = node_name_byte_count.checked_add(name_byte_count).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "node name bytes",
            },
        )?;
        let has_parent_relation =
            context.run_semantic_unit(SceneValidationWork::GraphNode, || {
                let declared_root = matches!(
                    storage.visit_state_by_local.get(node_index),
                    Some(VisitState::Depth(2))
                );
                match (declared_root, node.parent()) {
                    (true, Presence::Absent) => Ok(false),
                    (true, Presence::Present(parent)) => {
                        Err(NodeAuthoringSemanticError::RootHasParent {
                            root: node_index,
                            parent: parent.node().index(),
                        })
                    }
                    (false, Presence::Absent) => {
                        Err(NodeAuthoringSemanticError::MissingRoot { node: node_index })
                    }
                    (false, Presence::Present(parent)) => {
                        let parent_index = local_index(parent.node())?;
                        if parent_index >= nodes.len() {
                            return Err(NodeAuthoringSemanticError::ParentOutOfRange {
                                node: node_index,
                                parent: parent.node().index(),
                                node_count: nodes.len(),
                            });
                        }
                        if parent_index == node_index {
                            return Err(NodeAuthoringSemanticError::SelfParent {
                                node: node_index,
                            });
                        }
                        Ok(true)
                    }
                }
            })?;
        if has_parent_relation {
            context.add_relation()?;
        }
        context.add_transform(node.authored_transform().kind())?;
    }

    for start in 0..nodes.len() {
        let already_resolved = context.run_unit(SceneValidationWork::Traversal, || {
            matches!(
                storage.visit_state_by_local.get(start),
                Some(VisitState::Depth(_))
            )
        })?;
        if already_resolved {
            continue;
        }

        storage.canonical_order.clear();
        let mut cursor = start;
        let mut resolved_depth = loop {
            let step = context.run_semantic_unit(SceneValidationWork::Traversal, || {
                let state = *storage.visit_state_by_local.get(cursor).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "node traversal state",
                    },
                )?;
                match state {
                    VisitState::Depth(depth) => Ok(ParentTraversalStep::Resolved(depth)),
                    VisitState::Visiting => Err(NodeAuthoringSemanticError::Cycle { node: cursor }),
                    VisitState::Unseen => {
                        *storage.visit_state_by_local.get_mut(cursor).ok_or(
                            NodeAuthoringSemanticError::AccountingOverflow {
                                dimension: "node traversal state",
                            },
                        )? = VisitState::Visiting;
                        storage.canonical_order.push(cursor);
                        let Presence::Present(parent) = nodes[cursor].parent() else {
                            return Err(NodeAuthoringSemanticError::MissingRoot { node: cursor });
                        };
                        Ok(ParentTraversalStep::Continue(local_index(parent.node())?))
                    }
                }
            })?;
            match step {
                ParentTraversalStep::Resolved(depth) => break depth,
                ParentTraversalStep::Continue(parent) => cursor = parent,
            }
        };

        while let Some(node) = storage.canonical_order.pop() {
            let maximum_depth = context.limits.max_depth();
            resolved_depth = context.run_semantic_unit(SceneValidationWork::Traversal, || {
                let next_depth = resolved_depth.checked_add(1).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "node depth",
                    },
                )?;
                if u64::from(next_depth) > maximum_depth {
                    return Err(NodeAuthoringSemanticError::LimitExceeded {
                        limit: NodeAuthoringLimit::Depth,
                        actual: u64::from(next_depth),
                        maximum: maximum_depth,
                    });
                }
                *storage.visit_state_by_local.get_mut(node).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "node traversal state",
                    },
                )? = VisitState::Depth(next_depth);
                Ok(next_depth)
            })?;
        }
    }

    compile_canonical_breadth_first_order(nodes, roots, &mut storage, &mut context)?;

    context
        .matrix_scalars
        .checked_add(context.trs_scalars)
        .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical transform scalars",
        })?;
    let canonical_bytes = NodeStoragePlan::canonical_bytes_for_shape(
        canonical_node_count,
        context.references,
        node_name_byte_count,
    )
    .map_err(|_| NodeAuthoringSemanticError::AccountingOverflow {
        dimension: "initialized typed node footprint",
    })?;
    enforce_limit::<C::Error>(
        NodeAuthoringLimit::CanonicalBytes,
        canonical_bytes,
        limits.max_canonical_bytes(),
    )?;

    let retained_bytes = checked_sum_bytes(
        [order_bytes, index_bytes, child_range_bytes],
        "compiled node plan",
    )?;
    let ValidationStorage {
        canonical_order,
        canonical_index_by_local,
        visit_state_by_local,
        child_ranges_by_local,
        ordered_children,
        reservation,
    } = storage;
    drop(visit_state_by_local);
    drop(ordered_children);

    let mut compiled = NodeValidationStorage {
        canonical_node_count,
        source_root_count,
        reference_count: context.references,
        matrix_scalar_count: context.matrix_scalars,
        trs_scalar_count: context.trs_scalars,
        node_name_byte_count,
        node_name_reference_count,
        canonical_bytes,
        validation_work: context.work,
        scratch_bytes: retained_bytes,
        canonical_order,
        canonical_index_by_local,
        child_ranges_by_local,
        #[cfg(test)]
        storage_drop_probe: None,
        reservation,
    };
    context.resize_scratch(&mut compiled.reservation, retained_bytes)?;
    context.finish()?;
    Ok(ValidatedNodeBatch::new(input, compiled))
}

fn compile_canonical_breadth_first_order<C, R>(
    nodes: &[NodeAuthoringNode],
    roots: &[NodeLocalId],
    storage: &mut ValidationStorage<R>,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    storage.canonical_order.clear();
    storage.canonical_index_by_local.clear();
    storage.child_ranges_by_local.clear();
    storage.ordered_children.clear();
    context.fill_scratch(&mut storage.canonical_order, nodes.len(), usize::MAX)?;
    context.fill_scratch(&mut storage.canonical_index_by_local, nodes.len(), 0)?;
    context.fill_scratch(
        &mut storage.child_ranges_by_local,
        nodes.len(),
        ChildRange::default(),
    )?;

    for node in nodes.iter().copied() {
        context.run_semantic_unit(SceneValidationWork::Traversal, || {
            let Presence::Present(parent) = node.parent() else {
                return Ok(());
            };
            let parent_local = local_index(parent.node())?;
            let range = storage.child_ranges_by_local.get_mut(parent_local).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "child-count parent",
                },
            )?;
            range.count = range.count.checked_add(1).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "child count",
                },
            )?;
            Ok(())
        })?;
    }

    let mut next_child_slot = 0_u32;
    for range in &mut storage.child_ranges_by_local {
        next_child_slot = context.run_semantic_unit(SceneValidationWork::Traversal, || {
            range.start = next_child_slot;
            next_child_slot.checked_add(range.count).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "ordered child slots",
                },
            )
        })?;
    }

    let non_root_count = nodes.len().checked_sub(roots.len()).ok_or(
        NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "non-root node count",
        },
    )?;
    if next_child_slot != usize_to_u32(non_root_count, "ordered child slots")? {
        return Err(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "ordered child count",
        }
        .into());
    }
    context.fill_scratch(&mut storage.ordered_children, non_root_count, usize::MAX)?;

    for (local, node) in nodes.iter().copied().enumerate() {
        context.run_semantic_unit(SceneValidationWork::Traversal, || {
            let Presence::Present(parent) = node.parent() else {
                return Ok(());
            };
            let parent_local = local_index(parent.node())?;
            let child_range = storage
                .child_ranges_by_local
                .get(parent_local)
                .copied()
                .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "ordered child range",
                })?;
            let ordinal = parent.child_ordinal();
            if ordinal >= child_range.count {
                return Err(NodeAuthoringSemanticError::ChildOrdinalOutOfRange {
                    node: usize_to_u32(local, "child local node")?,
                    parent: parent.node().index(),
                    ordinal,
                    child_count: child_range.count,
                });
            }
            let child_slot = child_range.start.checked_add(ordinal).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "ordered child slot",
                },
            )?;
            let child_slot = usize::try_from(child_slot).map_err(|_| {
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "ordered child slot",
                }
            })?;
            let assigned = storage.ordered_children.get_mut(child_slot).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "ordered child slot",
                },
            )?;
            if *assigned != usize::MAX {
                return Err(NodeAuthoringSemanticError::DuplicateChildOrdinal {
                    parent: parent.node().index(),
                    ordinal,
                    first: usize_to_u32(*assigned, "first child local node")?,
                    duplicate: usize_to_u32(local, "duplicate child local node")?,
                });
            }
            *assigned = local;
            Ok(())
        })?;
    }

    for (root_position, &root) in roots.iter().enumerate() {
        context.run_semantic_unit(SceneValidationWork::Traversal, || {
            let local = local_index(root)?;
            *storage.canonical_order.get_mut(root_position).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "canonical root position",
                },
            )? = local;
            *storage.canonical_index_by_local.get_mut(local).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "canonical root index",
                },
            )? = usize_to_u32(root_position, "canonical root index")?
                .checked_add(1)
                .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "canonical root index",
                })?;
            Ok(())
        })?;
    }

    let mut read_cursor = 0_usize;
    let mut write_cursor = roots.len();
    while read_cursor < write_cursor {
        let (child_range, end) =
            context.run_semantic_unit(SceneValidationWork::Traversal, || {
                let parent_local = *storage.canonical_order.get(read_cursor).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical breadth-first parent",
                    },
                )?;
                let child_range = storage
                    .child_ranges_by_local
                    .get(parent_local)
                    .copied()
                    .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical breadth-first child range",
                    })?;
                let end = child_range.start.checked_add(child_range.count).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical breadth-first child range",
                    },
                )?;
                read_cursor = read_cursor.checked_add(1).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical breadth-first traversal",
                    },
                )?;
                Ok((child_range, end))
            })?;
        for child_slot in child_range.start..end {
            context.run_semantic_unit(SceneValidationWork::Traversal, || {
                let child_slot = usize::try_from(child_slot).map_err(|_| {
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical breadth-first child slot",
                    }
                })?;
                let child = *storage.ordered_children.get(child_slot).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical breadth-first child slot",
                    },
                )?;
                if child == usize::MAX {
                    return Err(NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical breadth-first child gap",
                    });
                }
                let canonical_index = usize_to_u32(write_cursor, "canonical node index")?
                    .checked_add(1)
                    .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical node index",
                    })?;
                let mapped = storage.canonical_index_by_local.get_mut(child).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical local index",
                    },
                )?;
                if *mapped != 0 {
                    return Err(NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "duplicate canonical child",
                    });
                }
                *storage.canonical_order.get_mut(write_cursor).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical node order",
                    },
                )? = child;
                *mapped = canonical_index;
                write_cursor = write_cursor.checked_add(1).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "canonical node order",
                    },
                )?;
                Ok(())
            })?;
        }
    }

    if write_cursor != nodes.len() || read_cursor != nodes.len() {
        return Err(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical breadth-first reachability",
        }
        .into());
    }
    Ok(())
}

fn scratch_envelope(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, NodeAuthoringSemanticError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| NodeAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn checked_sum_bytes<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, NodeAuthoringSemanticError> {
    values.into_iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or(NodeAuthoringSemanticError::AccountingOverflow { dimension })
    })
}

fn local_index(local: NodeLocalId) -> Result<usize, NodeAuthoringSemanticError> {
    usize::try_from(local.index()).map_err(|_| NodeAuthoringSemanticError::AccountingOverflow {
        dimension: "local node index",
    })
}

fn usize_to_u32(value: usize, dimension: &'static str) -> Result<u32, NodeAuthoringSemanticError> {
    u32::try_from(value).map_err(|_| NodeAuthoringSemanticError::AccountingOverflow { dimension })
}

fn usize_to_u64(value: usize, dimension: &'static str) -> Result<u64, NodeAuthoringSemanticError> {
    u64::try_from(value).map_err(|_| NodeAuthoringSemanticError::AccountingOverflow { dimension })
}

fn enforce_limit<E>(
    limit: NodeAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), NodeAuthoringFailure<E>>
where
    E: Error + Send + Sync + 'static,
{
    check_limit(limit, actual, maximum).map_err(NodeAuthoringFailure::Semantic)
}

fn check_limit(
    limit: NodeAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), NodeAuthoringSemanticError> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(NodeAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    }
}
