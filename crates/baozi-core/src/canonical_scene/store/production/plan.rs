#![allow(
    clippy::result_large_err,
    reason = "node authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

use std::error::Error;
use std::fmt;

use baozi_operation::{
    CapacityPlan, CapacityPlanError, OperationOwner, ProgressPhase, ProgressUnit,
    RetainedStoreReservation,
};
use thiserror::Error;

use crate::entity::AuthoredTransformKind;
use crate::{
    SCENE_VALIDATION_POLL_INTERVAL, SceneReservationKind, SceneValidationControl,
    SceneValidationLimits, SceneValidationWork,
};

use super::super::schema::{NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_TRS_SCALAR_COUNT};
use super::control::{OperationAuthoringControl, OperationAuthoringControlError};
use super::input::{NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, Presence};

const MATRIX_SCALARS_PER_NODE: u64 = NODE_SCENE_MATRIX_SCALAR_COUNT as u64;
const TRS_SCALARS_PER_NODE: u64 = NODE_SCENE_TRS_SCALAR_COUNT as u64;
const LOGICAL_U8_BYTES: u64 = 1;
const LOGICAL_U32_BYTES: u64 = 4;
const LOGICAL_F32_BYTES: u64 = 4;
const LOGICAL_REFERENCE_BYTES: u64 = 4;

// This is the schema-owned logical representation, not a retained allocation envelope. It counts
// stable scalar widths and one logical identity width per reachable reference while excluding table
// capacity, slot ordinals, alignment, and allocator metadata.
const NODE_SCENE_DIRECTORY_BASE_BYTES: u64 =
    (6 * LOGICAL_U32_BYTES) + (2 * LOGICAL_U8_BYTES) + LOGICAL_REFERENCE_BYTES;
const NODE_SCENE_DIRECTORY_POPULATED_REFERENCE_BYTES: u64 = 2 * LOGICAL_REFERENCE_BYTES;
const NODE_SCENE_NODE_BASE_BYTES: u64 = (5 * LOGICAL_U32_BYTES) + (2 * LOGICAL_U8_BYTES);
const NODE_SCENE_NODE_TRANSFORM_REFERENCE_BYTES: u64 = LOGICAL_REFERENCE_BYTES;
const NODE_SCENE_REFERENCE_BYTES: u64 = 2 * LOGICAL_U32_BYTES;

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

/// Operation-accounted node semantics ready for canonical materialization.
///
/// Canonical index zero is the implicit synthetic root. `canonical_order`
/// contains authored local indices only, and `canonical_index_by_local` maps
/// each local index to a canonical index in `1..canonical_node_count`.
#[must_use = "dropping the validated batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedNodeBatch<'a, R> {
    input: NodeAuthoringBatch<'a>,
    canonical_node_count: u64,
    reference_count: u64,
    matrix_scalar_count: u64,
    trs_scalar_count: u64,
    canonical_bytes: u64,
    // Compiled storage must be destroyed before the reservation covering it.
    canonical_order: Vec<usize>,
    canonical_index_by_local: Vec<u32>,
    child_ranges_by_local: Vec<ChildRange>,
    #[cfg(test)]
    storage_drop_probe: Option<StorageDropProbe>,
    reservation: R,
}

impl<R> ValidatedNodeBatch<'_, R> {
    pub(in crate::canonical_scene::store) const fn input(&self) -> NodeAuthoringBatch<'_> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn canonical_node_count(&self) -> u64 {
        self.canonical_node_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.reference_count
    }

    pub(in crate::canonical_scene::store) const fn matrix_scalar_count(&self) -> u64 {
        self.matrix_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn trs_scalar_count(&self) -> u64 {
        self.trs_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.canonical_bytes
    }

    pub(in crate::canonical_scene::store) fn canonical_order(&self) -> &[usize] {
        &self.canonical_order
    }

    pub(in crate::canonical_scene::store) fn canonical_index_by_local(&self) -> &[u32] {
        &self.canonical_index_by_local
    }

    pub(in crate::canonical_scene::store) fn child_count_by_local(
        &self,
        local: usize,
    ) -> Option<u32> {
        self.child_ranges_by_local
            .get(local)
            .map(|range| range.count)
    }

    #[cfg(test)]
    pub(super) fn install_storage_drop_probe(&mut self, stage: std::rc::Rc<std::cell::Cell<u8>>) {
        self.storage_drop_probe = Some(StorageDropProbe(stage));
    }
}

impl<R> fmt::Debug for ValidatedNodeBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedNodeBatch")
            .field("input", &self.input)
            .field("canonical_node_count", &self.canonical_node_count)
            .field("reference_count", &self.reference_count)
            .field("matrix_scalar_count", &self.matrix_scalar_count)
            .field("trs_scalar_count", &self.trs_scalar_count)
            .field("canonical_order", &self.canonical_order)
            .field("canonical_index_by_local", &self.canonical_index_by_local)
            .field("child_ranges_by_local", &self.child_ranges_by_local)
            .field("reservation", &"<held>")
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

type ValidationResult<T, C> = Result<T, NodeAuthoringFailure<<C as SceneValidationControl>::Error>>;

struct ValidationContext<'control, C: SceneValidationControl> {
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

impl<'control, C: SceneValidationControl> ValidationContext<'control, C> {
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

    fn work(&mut self, kind: SceneValidationWork, amount: u64) -> ValidationResult<(), C> {
        let final_work = self.work.checked_add(amount).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work",
            },
        )?;
        enforce_limit::<C::Error>(NodeAuthoringLimit::Work, final_work, self.limits.max_work())?;

        let mut remaining = amount;
        while remaining > 0 {
            let until_poll = SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll;
            let chunk = remaining.min(until_poll);
            self.control
                .charge_work(kind, chunk)
                .map_err(NodeAuthoringFailure::Control)?;
            self.work = self.work.checked_add(chunk).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "validation work",
                },
            )?;
            self.work_since_poll += chunk;
            remaining -= chunk;
            if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
                self.poll_now()?;
            }
        }
        Ok(())
    }

    fn add_relation(&mut self) -> ValidationResult<(), C> {
        self.edges = self
            .edges
            .checked_add(1)
            .ok_or(NodeAuthoringSemanticError::AccountingOverflow { dimension: "edges" })?;
        enforce_limit::<C::Error>(
            NodeAuthoringLimit::Edges,
            self.edges,
            self.limits.max_edges(),
        )?;

        self.references = self.references.checked_add(1).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "references",
            },
        )?;
        enforce_limit::<C::Error>(
            NodeAuthoringLimit::References,
            self.references,
            self.limits.max_references(),
        )?;
        self.work(SceneValidationWork::GraphEdge, 1)?;
        self.work(SceneValidationWork::Reference, 1)
    }

    fn add_transform(&mut self, kind: AuthoredTransformKind) -> ValidationResult<(), C> {
        let scalar_count = match kind {
            AuthoredTransformKind::Matrix => {
                self.matrix_scalars = self
                    .matrix_scalars
                    .checked_add(MATRIX_SCALARS_PER_NODE)
                    .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "matrix scalars",
                    })?;
                MATRIX_SCALARS_PER_NODE
            }
            AuthoredTransformKind::Trs => {
                self.trs_scalars = self.trs_scalars.checked_add(TRS_SCALARS_PER_NODE).ok_or(
                    NodeAuthoringSemanticError::AccountingOverflow {
                        dimension: "TRS scalars",
                    },
                )?;
                TRS_SCALARS_PER_NODE
            }
        };
        self.semantic_cells = self.semantic_cells.checked_add(scalar_count).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        if self.semantic_cells > u64::from(u32::MAX) {
            return Err(NodeAuthoringSemanticError::TransformDomainTooLarge {
                scalar_count: self.semantic_cells,
            }
            .into());
        }
        enforce_limit::<C::Error>(
            NodeAuthoringLimit::SemanticCells,
            self.semantic_cells,
            self.limits.max_payload_cells(),
        )?;
        self.work(SceneValidationWork::PayloadCell, scalar_count)
    }

    fn record_depth(&self, depth: u64) -> ValidationResult<(), C> {
        enforce_limit::<C::Error>(NodeAuthoringLimit::Depth, depth, self.limits.max_depth())
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
            .reserve(SceneReservationKind::ValidationScratch, bytes)
            .map_err(NodeAuthoringFailure::Control)?;
        self.control
            .validate_reservation(&reservation, SceneReservationKind::ValidationScratch)
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
            .resize_reservation(reservation, SceneReservationKind::ValidationScratch, bytes)
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
        while values.len() < count {
            let remaining = count - values.len();
            let until_poll = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll)
                .map_err(|_| NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "validation work",
                })?;
            let amount = remaining.min(until_poll);
            self.work(
                SceneValidationWork::Traversal,
                usize_to_u64(amount, "validation work")?,
            )?;
            values.extend(std::iter::repeat_n(value, amount));
        }
        Ok(())
    }
}

pub(in crate::canonical_scene::store) fn validate_node_batch<'a, O, PP, U>(
    input: NodeAuthoringBatch<'a>,
    limits: SceneValidationLimits,
    control: &mut OperationAuthoringControl<'_, O, PP, U>,
) -> Result<
    ValidatedNodeBatch<'a, RetainedStoreReservation<O>>,
    NodeAuthoringFailure<OperationAuthoringControlError>,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    validate_node_batch_inner(input, limits, control)
}

#[cfg(test)]
pub(super) fn validate_node_batch_with_control<'a, C>(
    input: NodeAuthoringBatch<'a>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedNodeBatch<'a, C::Reservation>, NodeAuthoringFailure<C::Error>>
where
    C: SceneValidationControl,
{
    validate_node_batch_inner(input, limits, control)
}

fn validate_node_batch_inner<'a, C>(
    input: NodeAuthoringBatch<'a>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedNodeBatch<'a, C::Reservation>, NodeAuthoringFailure<C::Error>>
where
    C: SceneValidationControl,
{
    let mut context = ValidationContext::new(limits, control);
    context.poll_now()?;

    let roots = match input.roots() {
        Presence::Absent => return Err(NodeAuthoringSemanticError::RootsAbsent.into()),
        Presence::Present(roots) => roots,
    };
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
    context.record_depth(1)?;

    context.work(SceneValidationWork::GraphNode, 1)?;

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
        context.work(SceneValidationWork::GraphEdge, 1)?;
        let local = local_index(root)?;
        if local >= nodes.len() {
            return Err(NodeAuthoringSemanticError::RootOutOfRange {
                root_index,
                root: root.index(),
                node_count: nodes.len(),
            }
            .into());
        }
        let state = storage.visit_state_by_local.get_mut(local).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "root visit state",
            },
        )?;
        if !matches!(state, VisitState::Unseen) {
            return Err(NodeAuthoringSemanticError::DuplicateRoot {
                root: root.index(),
                duplicate_index: root_index,
            }
            .into());
        }
        if let Presence::Present(parent) = nodes[local].parent() {
            return Err(NodeAuthoringSemanticError::RootHasParent {
                root: local,
                parent: parent.node().index(),
            }
            .into());
        }
        context.record_depth(2)?;
        *state = VisitState::Depth(2);
        context.add_relation()?;
    }

    for (node_index, node) in nodes.iter().copied().enumerate() {
        context.work(SceneValidationWork::GraphNode, 1)?;
        let declared_root = matches!(
            storage.visit_state_by_local.get(node_index),
            Some(VisitState::Depth(2))
        );
        match (declared_root, node.parent()) {
            (true, Presence::Absent) => {}
            (true, Presence::Present(parent)) => {
                return Err(NodeAuthoringSemanticError::RootHasParent {
                    root: node_index,
                    parent: parent.node().index(),
                }
                .into());
            }
            (false, Presence::Absent) => {
                return Err(NodeAuthoringSemanticError::MissingRoot { node: node_index }.into());
            }
            (false, Presence::Present(parent)) => {
                let parent_index = local_index(parent.node())?;
                if parent_index >= nodes.len() {
                    return Err(NodeAuthoringSemanticError::ParentOutOfRange {
                        node: node_index,
                        parent: parent.node().index(),
                        node_count: nodes.len(),
                    }
                    .into());
                }
                if parent_index == node_index {
                    return Err(NodeAuthoringSemanticError::SelfParent { node: node_index }.into());
                }
                context.add_relation()?;
            }
        }
        context.add_transform(node.authored_transform().kind())?;
    }

    for start in 0..nodes.len() {
        context.work(SceneValidationWork::Traversal, 1)?;
        if matches!(
            storage.visit_state_by_local.get(start),
            Some(VisitState::Depth(_))
        ) {
            continue;
        }

        storage.canonical_order.clear();
        let mut cursor = start;
        let mut resolved_depth = loop {
            context.work(SceneValidationWork::Traversal, 1)?;
            let state = *storage.visit_state_by_local.get(cursor).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "node traversal state",
                },
            )?;
            match state {
                VisitState::Depth(depth) => break depth,
                VisitState::Visiting => {
                    return Err(NodeAuthoringSemanticError::Cycle { node: cursor }.into());
                }
                VisitState::Unseen => {
                    *storage.visit_state_by_local.get_mut(cursor).ok_or(
                        NodeAuthoringSemanticError::AccountingOverflow {
                            dimension: "node traversal state",
                        },
                    )? = VisitState::Visiting;
                    storage.canonical_order.push(cursor);
                    let Presence::Present(parent) = nodes[cursor].parent() else {
                        return Err(NodeAuthoringSemanticError::MissingRoot { node: cursor }.into());
                    };
                    cursor = local_index(parent.node())?;
                }
            }
        };

        while let Some(node) = storage.canonical_order.pop() {
            context.work(SceneValidationWork::Traversal, 1)?;
            resolved_depth = resolved_depth.checked_add(1).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "node depth",
                },
            )?;
            context.record_depth(u64::from(resolved_depth))?;
            *storage.visit_state_by_local.get_mut(node).ok_or(
                NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "node traversal state",
                },
            )? = VisitState::Depth(resolved_depth);
        }
    }

    compile_canonical_breadth_first_order(nodes, roots, &mut storage, &mut context)?;

    let transform_scalar_count = context
        .matrix_scalars
        .checked_add(context.trs_scalars)
        .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical transform scalars",
        })?;
    let canonical_bytes = node_scene_logical_bytes(
        canonical_node_count,
        context.references,
        transform_scalar_count,
    )?;
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

    let mut validated = ValidatedNodeBatch {
        input,
        canonical_node_count,
        reference_count: context.references,
        matrix_scalar_count: context.matrix_scalars,
        trs_scalar_count: context.trs_scalars,
        canonical_bytes,
        canonical_order,
        canonical_index_by_local,
        child_ranges_by_local,
        #[cfg(test)]
        storage_drop_probe: None,
        reservation,
    };
    context.resize_scratch(&mut validated.reservation, retained_bytes)?;
    context.finish()?;
    Ok(validated)
}

fn compile_canonical_breadth_first_order<C, R>(
    nodes: &[NodeAuthoringNode],
    roots: &[NodeLocalId],
    storage: &mut ValidationStorage<R>,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C>
where
    C: SceneValidationControl,
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
        context.work(SceneValidationWork::Traversal, 1)?;
        let Presence::Present(parent) = node.parent() else {
            continue;
        };
        let parent_local = local_index(parent.node())?;
        let range = storage.child_ranges_by_local.get_mut(parent_local).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "child-count parent",
            },
        )?;
        range.count =
            range
                .count
                .checked_add(1)
                .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "child count",
                })?;
    }

    let mut next_child_slot = 0_u32;
    for range in &mut storage.child_ranges_by_local {
        context.work(SceneValidationWork::Traversal, 1)?;
        range.start = next_child_slot;
        next_child_slot = next_child_slot.checked_add(range.count).ok_or(
            NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "ordered child slots",
            },
        )?;
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
        context.work(SceneValidationWork::Traversal, 1)?;
        let Presence::Present(parent) = node.parent() else {
            continue;
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
            }
            .into());
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
            }
            .into());
        }
        *assigned = local;
    }

    for (root_position, &root) in roots.iter().enumerate() {
        context.work(SceneValidationWork::Traversal, 1)?;
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
    }

    let mut read_cursor = 0_usize;
    let mut write_cursor = roots.len();
    while read_cursor < write_cursor {
        context.work(SceneValidationWork::Traversal, 1)?;
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
        for child_slot in child_range.start..end {
            context.work(SceneValidationWork::Traversal, 1)?;
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
                }
                .into());
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
                }
                .into());
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
        }
        read_cursor =
            read_cursor
                .checked_add(1)
                .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
                    dimension: "canonical breadth-first traversal",
                })?;
    }

    if write_cursor != nodes.len() || read_cursor != nodes.len() {
        return Err(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical breadth-first reachability",
        }
        .into());
    }
    Ok(())
}

fn node_scene_logical_bytes(
    canonical_node_count: u64,
    reference_count: u64,
    transform_scalar_count: u64,
) -> Result<u64, NodeAuthoringSemanticError> {
    let source_node_count = canonical_node_count.checked_sub(1).ok_or(
        NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical source nodes",
        },
    )?;
    if reference_count != source_node_count {
        return Err(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical parent references",
        });
    }

    let directory_bytes = if source_node_count == 0 {
        NODE_SCENE_DIRECTORY_BASE_BYTES
    } else {
        NODE_SCENE_DIRECTORY_BASE_BYTES
            .checked_add(NODE_SCENE_DIRECTORY_POPULATED_REFERENCE_BYTES)
            .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical directory",
            })?
    };
    let source_node_bytes = NODE_SCENE_NODE_BASE_BYTES
        .checked_add(NODE_SCENE_NODE_TRANSFORM_REFERENCE_BYTES)
        .and_then(|bytes| bytes.checked_mul(source_node_count))
        .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical node records",
        })?;
    let node_bytes = NODE_SCENE_NODE_BASE_BYTES
        .checked_add(source_node_bytes)
        .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical node records",
        })?;
    let reference_bytes = reference_count
        .checked_mul(NODE_SCENE_REFERENCE_BYTES)
        .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical reference records",
        })?;
    let transform_bytes = transform_scalar_count
        .checked_mul(LOGICAL_F32_BYTES)
        .ok_or(NodeAuthoringSemanticError::AccountingOverflow {
            dimension: "canonical transform values",
        })?;

    checked_sum_bytes(
        [
            directory_bytes,
            node_bytes,
            reference_bytes,
            transform_bytes,
        ],
        "canonical logical footprint",
    )
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
    if actual <= maximum {
        Ok(())
    } else {
        Err(NodeAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        }
        .into())
    }
}
