//! Incremental longest-depth planning over the target's reachable region.
//!
//! Committed depths strictly increase along every available edge. They therefore prove that an
//! edge whose target is already deeper than its source cannot create a cycle or increase any
//! depth. Harder edges walk only the target's reachable nodes. Epoch-tagged visit fields live in
//! the already admitted graph rows; each walk allocates only its own stack, order, and updates.

use super::*;

#[derive(Debug, Default)]
pub(super) struct DepthVisit {
    epoch: u64,
    state: u8,
    proposed: u64,
}

struct DepthFrame<'graph> {
    index: usize,
    outgoing: OutgoingRelations<'graph>,
}

impl ResourceGraph {
    pub(super) fn plan_existing_target(
        &mut self,
        exec: &mut ImportStoreExecution<'_>,
        target: usize,
        source: usize,
        target_depth: u64,
    ) -> Result<AccountedVec<DepthUpdate>, ResourceGraphError> {
        exec.poll()
            .map_err(|cause| ResourceGraphError::Scope(cause.into_scope_abort_cause()))?;
        let group = exec
            .bind_group([ImportDimension::ResourceGraphWork])
            .map_err(|cause| ResourceGraphError::Scope(cause.into_scope_abort_cause()))?;
        charge_bound_work(&group, 1)?;
        if target == source {
            return Err(ResourceGraphError::Cycle);
        }
        let target_node = self
            .nodes
            .get(target)
            .ok_or(ResourceGraphError::DanglingNode)?;
        if target_depth <= target_node.depth {
            return Ok(AccountedVec::new());
        }
        if target_node.outgoing_count == 0 {
            let mut updates = AccountedVec::new();
            push_scratch(
                exec,
                &group,
                &mut updates,
                DepthUpdate {
                    index: target,
                    depth: target_depth,
                },
                "resource-graph-depth-updates",
            )?;
            return Ok(updates);
        }

        self.depth_epoch = self
            .depth_epoch
            .checked_add(1)
            .ok_or(ResourceGraphError::Overflow)?;
        let epoch = self.depth_epoch;
        let nodes = &mut self.nodes;
        let relations = &self.relations;
        let mut stack = AccountedVec::new();
        let mut postorder = AccountedVec::new();
        let first = enter(nodes, relations, target, epoch)?;
        push_scratch(
            exec,
            &group,
            &mut stack,
            first,
            "resource-graph-depth-stack",
        )?;
        while let Some(frame) = stack.last_mut() {
            charge_bound_graph_work(exec, &group, 1)?;
            match frame.outgoing.next() {
                Some(Err(error)) => return Err(error),
                Some(Ok(relation)) => {
                    let GraphRelationOutcome::Available { target: child } = relation.outcome else {
                        continue;
                    };
                    if child == source {
                        return Err(ResourceGraphError::Cycle);
                    }
                    let visit = &nodes
                        .get(child)
                        .ok_or(ResourceGraphError::DanglingNode)?
                        .depth_visit;
                    if visit.epoch == epoch {
                        if visit.state == 1 {
                            return Err(ResourceGraphError::Cycle);
                        }
                        continue;
                    }
                    let child = enter(nodes, relations, child, epoch)?;
                    push_scratch(
                        exec,
                        &group,
                        &mut stack,
                        child,
                        "resource-graph-depth-stack",
                    )?;
                }
                None => {
                    let index = frame.index;
                    nodes[index].depth_visit.state = 2;
                    push_scratch(
                        exec,
                        &group,
                        &mut postorder,
                        index,
                        "resource-graph-depth-postorder",
                    )?;
                    stack.pop();
                }
            }
        }

        nodes[target].depth_visit.proposed = target_depth;
        let maximum_depth = u64::try_from(self.limits.max_resource_graph_depth).unwrap_or(u64::MAX);
        let mut updates = AccountedVec::new();
        for &index in postorder.iter().rev() {
            charge_bound_graph_work(exec, &group, 1)?;
            let node = &nodes[index];
            let proposed = node.depth_visit.proposed;
            if proposed > maximum_depth {
                return Err(ResourceGraphError::Limit {
                    resource: "max_resource_graph_depth",
                    limit: maximum_depth,
                });
            }
            if proposed > node.depth {
                push_scratch(
                    exec,
                    &group,
                    &mut updates,
                    DepthUpdate {
                        index,
                        depth: proposed,
                    },
                    "resource-graph-depth-updates",
                )?;
            }
            let child_depth = proposed
                .checked_add(1)
                .ok_or(ResourceGraphError::Overflow)?;
            let outgoing = outgoing(node, relations);
            for relation in outgoing {
                charge_bound_graph_work(exec, &group, 1)?;
                if let GraphRelationOutcome::Available { target: child } = relation?.outcome {
                    let visit = &mut nodes
                        .get_mut(child)
                        .ok_or(ResourceGraphError::DanglingNode)?
                        .depth_visit;
                    if visit.epoch != epoch || visit.state != 2 {
                        return Err(ResourceGraphError::DanglingNode);
                    }
                    visit.proposed = visit.proposed.max(child_depth);
                }
            }
        }
        Ok(updates)
    }
}

fn enter<'graph>(
    nodes: &mut [GraphNode],
    relations: &'graph [GraphRelation],
    index: usize,
    epoch: u64,
) -> Result<DepthFrame<'graph>, ResourceGraphError> {
    let node = nodes
        .get_mut(index)
        .ok_or(ResourceGraphError::DanglingNode)?;
    node.depth_visit = DepthVisit {
        epoch,
        state: 1,
        proposed: node.depth,
    };
    Ok(DepthFrame {
        index,
        outgoing: outgoing(node, relations),
    })
}

fn outgoing<'graph>(
    node: &GraphNode,
    relations: &'graph [GraphRelation],
) -> OutgoingRelations<'graph> {
    OutgoingRelations {
        relations,
        next: node.first_outgoing,
        remaining: node.outgoing_count,
        terminal_checked: false,
        direction: OutgoingDirection::Forward,
    }
}

fn push_scratch<T>(
    exec: &mut ImportStoreExecution<'_>,
    group: &ImportBoundWorkGroup<'_, 1>,
    values: &mut AccountedVec<T>,
    value: T,
    resource: &'static str,
) -> Result<(), ResourceGraphError> {
    charge_bound_graph_work(exec, group, 1)?;
    values
        .try_reserve(1, resource, |bytes| reserve_graph_storage(exec, bytes))
        .map_err(ResourceGraphError::Scope)?;
    values
        .try_push_reserved(value)
        .map_err(|_| ResourceGraphError::Allocation { resource })
}
