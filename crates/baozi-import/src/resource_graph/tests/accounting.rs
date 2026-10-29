use super::*;

#[test]
fn graph_path_capacity_stays_charged_until_its_owner_drops() {
    let limits = ResourceLimits::default();
    let owner = store_owner(&limits);
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let path = AssetPath::new("models/primary.asset").unwrap();
    let mut graph = ResourceGraph::new(limits);
    graph
        .bind_primary(host, &authority, &path, b"primary")
        .unwrap();
    let node = &graph.nodes[0];
    assert_eq!(node.exact_identity.path(), &path);
    assert_eq!(node.public_identity.value(), &path);
    let path_bytes = node
        ._path_reservations
        .iter()
        .map(|reservation| reservation.as_ref().unwrap().bytes())
        .sum::<u64>();
    assert_eq!(
        path_bytes,
        (node.exact_identity.path().capacity_bytes()
            + node.public_identity.value().capacity_bytes()) as u64
    );
    assert!(
        host.dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes()
            >= path_bytes
    );
    drop(graph);
    assert_eq!(
        host.dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes(),
        0
    );
}

#[test]
fn graph_path_budget_rejects_before_a_copy_can_escape() {
    let limits = ResourceLimits::default();
    let owner = store_owner(&limits);
    let host = owner.host();
    host.tighten_dimension_policy(
        ImportDimension::ScratchBytes,
        DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, 0),
    )
    .unwrap();
    let path = AssetPath::new("models/primary.asset").unwrap();
    assert!(clone_graph_path(host, &path, "test-graph-path").is_err());
    let state = host.dimension_state(ImportDimension::ScratchBytes).unwrap();
    assert_eq!(state.live_bytes(), 0);
}

#[test]
fn pending_depth_updates_retain_only_their_own_scratch_capacity() {
    let limits = ResourceLimits::default();
    let owner = store_owner(&limits);
    let host = owner.host();
    let (mut graph, role, target, source) = shared_depth_prefix(host, 3);
    let mut pending = graph
        .begin_relation(
            host,
            source,
            &role,
            DependencyRequirement::Required,
            target.path().as_str(),
        )
        .unwrap();
    let baseline = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
    graph.prepare_target(host, &mut pending, &target).unwrap();
    let Some(TargetAdmission::Existing { depth_updates, .. }) = &pending.target else {
        panic!("an existing shared target must retain its planned depth updates")
    };
    assert!(!depth_updates.is_empty());
    let live = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
    assert!(live > baseline);
    drop(pending);
    assert_eq!(
        host.dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes(),
        baseline
    );
    drop(graph);
    assert_eq!(
        host.dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes(),
        0
    );
}
