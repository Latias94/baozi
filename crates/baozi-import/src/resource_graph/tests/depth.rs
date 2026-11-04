//! Concrete incremental depth and rejection tests over the actual session graph.

use super::*;

fn work(exec: &ImportRootExecution<'_, '_>) -> u64 {
    exec.dimension_state(ImportDimension::ResourceGraphWork)
        .unwrap()
        .consumed()
}

fn committed(graph: &ResourceGraph) -> (Vec<u64>, usize, u64, u64) {
    (
        graph.nodes.iter().map(|node| node.depth).collect(),
        graph.relations.len(),
        graph.relation_count,
        graph.maximum_depth,
    )
}

#[test]
fn already_deeper_target_needs_no_scratch_or_descendant_traversal() {
    let limits = ResourceLimits::default();
    with_store_body(
        limits.clone(),
        ImportOperationControl::default(),
        |exec, account| {
            let (mut graph, _, target, _) = shared_depth_prefix(exec, 3);
            let target = graph.find_node(exec, &target).unwrap().unwrap();
            assert!(graph.nodes[target].outgoing_count > 0);
            let before = committed(&graph);
            let start = work(exec);
            let before_allocation = account.snapshot();
            let updates = graph.plan_existing_target(exec, target, 0, 1).unwrap();
            assert!(updates.is_empty());
            assert_eq!(work(exec) - start, 1);
            assert_eq!(account.snapshot(), before_allocation);
            assert_eq!(committed(&graph), before);
            assert!(matches!(
                graph.plan_existing_target(exec, target, target, 2),
                Err(ResourceGraphError::Cycle)
            ));
            drop(graph);
        },
    );
}

#[test]
fn converging_paths_merge_longest_depth_and_rejected_cycles_change_no_committed_state() {
    for reverse in [false, true] {
        let limits = ResourceLimits::default();
        with_store_body(
            limits.clone(),
            ImportOperationControl::default(),
            |exec, _account| {
                let authority = AuthorityDomain::new();
                let role = DependencyRoleId::new("test.resource").unwrap();
                let mut graph = ResourceGraph::new(limits);
                graph
                    .bind_primary(
                        exec,
                        &authority,
                        &AssetPath::new("main.bin").unwrap(),
                        b"primary",
                    )
                    .unwrap();
                let target = dependency_identity(&authority, "target.bin");
                let a = dependency_identity(&authority, "a.bin");
                let b = dependency_identity(&authority, "b.bin");
                let d = dependency_identity(&authority, "d.bin");
                let c = dependency_identity(&authority, "c.bin");
                let source = dependency_identity(&authority, "source.bin");
                let t = add_available(&mut graph, exec, 0, &role, &target, b"t");
                let (left, right) = if reverse {
                    let right = add_available(&mut graph, exec, t, &role, &b, b"b");
                    let left = add_available(&mut graph, exec, t, &role, &a, b"a");
                    (left, right)
                } else {
                    let left = add_available(&mut graph, exec, t, &role, &a, b"a");
                    let right = add_available(&mut graph, exec, t, &role, &b, b"b");
                    (left, right)
                };
                let tail = add_available(&mut graph, exec, left, &role, &c, b"c");
                let middle = add_available(&mut graph, exec, right, &role, &d, b"d");
                assert_eq!(
                    add_available(&mut graph, exec, middle, &role, &c, b"c"),
                    tail
                );
                let s = add_available(&mut graph, exec, 0, &role, &source, b"s");
                let before = committed(&graph);
                let plan = graph.plan_existing_target(exec, t, s, 2).unwrap();
                assert_eq!(
                    committed(&graph),
                    before,
                    "planning may only modify epoch-tagged scratch"
                );
                assert_eq!(plan.len(), 5);
                let expected = [(t, 2), (left, 3), (right, 3), (middle, 4), (tail, 5)];
                for (index, depth) in expected {
                    assert_eq!(
                        plan.iter().filter(|update| update.index == index).count(),
                        1
                    );
                    assert_eq!(
                        plan.iter()
                            .find(|update| update.index == index)
                            .unwrap()
                            .depth,
                        depth
                    );
                }
                drop(plan);
                assert_eq!(add_available(&mut graph, exec, s, &role, &target, b"t"), t);
                for (index, depth) in expected {
                    assert_eq!(graph.nodes[index].depth, depth);
                }
                let before = committed(&graph);
                assert!(matches!(
                    graph.plan_existing_target(exec, t, tail, 6),
                    Err(ResourceGraphError::Cycle)
                ));
                assert_eq!(committed(&graph), before);
                // A new epoch after rejection must ignore the incomplete visitation marks.
                let side = dependency_identity(&authority, "side.bin");
                let side = add_available(&mut graph, exec, 0, &role, &side, b"side");
                let deeper = dependency_identity(&authority, "deeper.bin");
                let deeper = add_available(&mut graph, exec, side, &role, &deeper, b"deeper");
                let retried = graph.plan_existing_target(exec, t, deeper, 3).unwrap();
                assert_eq!(retried.len(), 5);
                assert_eq!(
                    retried
                        .iter()
                        .find(|update| update.index == tail)
                        .unwrap()
                        .depth,
                    6
                );
                drop(retried);
                assert_eq!(
                    add_available(&mut graph, exec, deeper, &role, &target, b"t"),
                    t
                );
                graph.seal(exec).unwrap();
                assert_eq!(graph.take_manifest().unwrap().counts().maximum_depth(), 6);
                drop(graph);
            },
        );
    }
}

#[test]
fn interrupted_local_walk_releases_scratch_without_changing_committed_state() {
    for allowed in [1_u64, 3, 6] {
        let limits = ResourceLimits::default();
        with_store_body(
            limits.clone(),
            ImportOperationControl::default(),
            |exec, _account| {
                let (mut graph, _, identity, source) = shared_depth_prefix(exec, 3);
                let target = graph.find_node(exec, &identity).unwrap().unwrap();
                let before = committed(&graph);
                let scratch = exec
                    .dimension_state(ImportDimension::ScratchBytes)
                    .unwrap()
                    .live_bytes();
                let start = work(exec);
                exec.tighten_dimension_policy(
                    ImportDimension::ResourceGraphWork,
                    DimensionPolicy::counter(start + allowed, start + allowed, 0),
                )
                .unwrap();
                assert!(matches!(
                    graph.plan_existing_target(exec, target, source, 2),
                    Err(ResourceGraphError::Scope(_))
                ));
                assert_eq!(committed(&graph), before);
                assert_eq!(
                    exec.dimension_state(ImportDimension::ScratchBytes)
                        .unwrap()
                        .live_bytes(),
                    scratch
                );
                drop(graph);
            },
        );
    }
}

#[test]
fn late_depth_work_denial_commits_neither_relation_nor_depth_updates() {
    let limits = ResourceLimits::default();
    with_store_body(
        limits.clone(),
        ImportOperationControl::default(),
        |exec, _account| {
            let (mut graph, role, target, source) = shared_depth_prefix(exec, 3);
            let mut pending = graph
                .begin_relation(
                    exec,
                    source,
                    &role,
                    DependencyRequirement::Required,
                    target.path().as_str(),
                )
                .unwrap();
            graph.prepare_target(exec, &mut pending, &target).unwrap();
            let before = committed(&graph);
            let start = work(exec);
            exec.tighten_dimension_policy(
                ImportDimension::ResourceGraphWork,
                DimensionPolicy::counter(start + 1, start + 1, 0),
            )
            .unwrap();
            let (cache, snapshot) = cache_snapshot(&mut graph, exec, &target, b"a");
            assert!(matches!(
                graph.complete_available(
                    exec,
                    pending,
                    cache,
                    snapshot.bytes(),
                    AvailableResourceIdentity::new(&target, target.path(), None)
                ),
                Err(ResourceGraphError::Scope(_))
            ));
            assert_eq!(committed(&graph), before);
            drop(snapshot);
            drop(graph);
        },
    );
}

#[test]
fn rejected_relation_completion_commits_no_new_node_cache_binding_or_links() {
    let limits = ResourceLimits::default();
    with_store_body(
        limits.clone(),
        ImportOperationControl::default(),
        |exec, _account| {
            let authority = AuthorityDomain::new();
            let role = DependencyRoleId::new("test.resource").unwrap();
            let mut graph = ResourceGraph::new(limits);
            graph
                .bind_primary(
                    exec,
                    &authority,
                    &AssetPath::new("main.bin").unwrap(),
                    b"primary",
                )
                .unwrap();
            let identity = dependency_identity(&authority, "new.bin");
            let mut pending = graph
                .begin_relation(
                    exec,
                    0,
                    &role,
                    DependencyRequirement::Required,
                    identity.path().as_str(),
                )
                .unwrap();
            graph.prepare_target(exec, &mut pending, &identity).unwrap();
            let (cache, snapshot) = cache_snapshot(&mut graph, exec, &identity, b"new");
            let before = committed(&graph);
            let relations = exec
                .dimension_state(ImportDimension::ResourceGraphRelations)
                .unwrap();
            exec.tighten_dimension_policy(
                ImportDimension::ResourceGraphRelations,
                DimensionPolicy::counter(relations.attempted(), relations.consumed(), 0),
            )
            .unwrap();
            assert!(matches!(
                graph.complete_available(
                    exec,
                    pending,
                    cache,
                    snapshot.bytes(),
                    AvailableResourceIdentity::new(&identity, identity.path(), None)
                ),
                Err(ResourceGraphError::Scope(_))
            ));
            assert_eq!(committed(&graph), before);
            assert!(graph.cache_to_node.get(cache).is_none_or(Option::is_none));
            assert_eq!(graph.nodes[0].first_outgoing, None);
            assert_eq!(graph.nodes[0].last_outgoing, None);
            assert_eq!(graph.nodes[0].outgoing_count, 0);
            drop(snapshot);
            drop(graph);
        },
    );
}

#[test]
fn prepared_target_rejects_interleaved_commits_that_change_depth_or_create_a_back_edge() {
    for back_edge in [false, true] {
        let limits = ResourceLimits::default();
        with_store_body(
            limits.clone(),
            ImportOperationControl::default(),
            |exec, _account| {
                let authority = AuthorityDomain::new();
                let role = DependencyRoleId::new("test.resource").unwrap();
                let mut graph = ResourceGraph::new(limits);
                graph
                    .bind_primary(
                        exec,
                        &authority,
                        &AssetPath::new("main.bin").unwrap(),
                        b"primary",
                    )
                    .unwrap();
                let a = dependency_identity(&authority, "a.bin");
                let b = dependency_identity(&authority, "b.bin");
                let t = dependency_identity(&authority, "target.bin");
                let source = add_available(&mut graph, exec, 0, &role, &a, b"a");
                let other = add_available(&mut graph, exec, 0, &role, &b, b"b");
                add_available(&mut graph, exec, 0, &role, &t, b"target");
                let (identity, bytes) = if back_edge {
                    (&b, b"b".as_slice())
                } else {
                    (&t, b"target".as_slice())
                };
                let mut pending = graph
                    .begin_relation(
                        exec,
                        source,
                        &role,
                        DependencyRequirement::Required,
                        identity.path().as_str(),
                    )
                    .unwrap();
                graph.prepare_target(exec, &mut pending, identity).unwrap();
                let (cache, snapshot) = cache_snapshot(&mut graph, exec, identity, bytes);
                add_available(&mut graph, exec, other, &role, &a, b"a");
                let before = committed(&graph);
                assert!(matches!(
                    graph.complete_available(
                        exec,
                        pending,
                        cache,
                        snapshot.bytes(),
                        AvailableResourceIdentity::new(identity, identity.path(), None)
                    ),
                    Err(ResourceGraphError::StaleRelation)
                ));
                assert_eq!(committed(&graph), before);
                drop(snapshot);
                drop(graph);
            },
        );
    }
}

#[test]
fn cancellation_after_cached_target_preparation_cannot_commit_the_edge_or_depths() {
    for changes_depth in [false, true] {
        let limits = ResourceLimits::default();
        let cancellation = baozi_operation::CancellationToken::new();
        with_store_body(
            limits.clone(),
            ImportOperationControl::default().with_cancellation(cancellation.clone()),
            |exec, _account| {
                let (mut graph, role, target, source) = shared_depth_prefix(exec, 3);
                let source = if changes_depth { source } else { 0 };
                let mut pending = graph
                    .begin_relation(
                        exec,
                        source,
                        &role,
                        DependencyRequirement::Required,
                        target.path().as_str(),
                    )
                    .unwrap();
                graph.prepare_target(exec, &mut pending, &target).unwrap();
                let Some(TargetAdmission::Existing { depth_updates, .. }) = &pending.target else {
                    panic!("cached graph target must already exist");
                };
                assert_eq!(depth_updates.is_empty(), !changes_depth);
                let (cache, snapshot) = cache_snapshot(&mut graph, exec, &target, b"a");
                // Ensure completion can reuse an already calculated digest rather than polling in hashing.
                snapshot.content_digest(exec).unwrap();
                let before = committed(&graph);
                cancellation.cancel();
                assert!(matches!(
                    graph.complete_available(
                        exec,
                        pending,
                        cache,
                        snapshot.bytes(),
                        AvailableResourceIdentity::new(&target, target.path(), None)
                    ),
                    Err(ResourceGraphError::Scope(ScopeAbortCause::Control(_)))
                ));
                assert_eq!(committed(&graph), before);
                drop(snapshot);
                drop(graph);
            },
        );
    }
}

#[test]
fn cancellation_during_local_depth_walk_releases_its_scratch() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Stop {
        polls: Arc<AtomicUsize>,
        at: Arc<AtomicUsize>,
    }
    impl baozi_operation::CancellationSignal for Stop {
        fn is_cancelled(&self) -> bool {
            self.polls.fetch_add(1, Ordering::Relaxed) >= self.at.load(Ordering::Relaxed)
        }
    }
    let limits = ResourceLimits::default();
    let polls = Arc::new(AtomicUsize::new(0));
    let at = Arc::new(AtomicUsize::new(usize::MAX));
    with_store_body(
        limits.clone(),
        ImportOperationControl::default().with_cancellation(Stop {
            polls: polls.clone(),
            at: at.clone(),
        }),
        |exec, _account| {
            let (mut graph, _, target, source) = shared_depth_prefix(exec, 3);
            let target = graph.find_node(exec, &target).unwrap().unwrap();
            let before = committed(&graph);
            let scratch = exec
                .dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes();
            at.store(polls.load(Ordering::Relaxed) + 4, Ordering::Relaxed);
            assert!(matches!(
                graph.plan_existing_target(exec, target, source, 2),
                Err(ResourceGraphError::Scope(ScopeAbortCause::Control(_)))
            ));
            assert!(graph.depth_epoch > 0);
            assert_eq!(committed(&graph), before);
            assert_eq!(
                exec.dimension_state(ImportDimension::ScratchBytes)
                    .unwrap()
                    .live_bytes(),
                scratch
            );
            drop(graph);
        },
    );
}
