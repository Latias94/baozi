use super::*;

#[test]
fn provisional_manifest_keeps_paths_and_tables_live_after_the_graph_is_destroyed() {
    let limits = ResourceLimits::default();
    let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
    let host = owner.host();
    let baseline = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
    let source = AssetPath::new(format!(
        "models/{}primary.asset",
        "retained-path/".repeat(24)
    ))
    .unwrap();
    let authority = AuthorityDomain::new();
    let mut graph = ResourceGraph::new(limits);
    graph
        .bind_primary(host, &authority, &source, b"immutable-primary")
        .unwrap();
    graph.seal(host).unwrap();
    let manifest = graph.take_manifest().unwrap();
    let together = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
    drop(graph);
    let retained = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
    assert!(retained - baseline >= source.as_str().len() as u64);
    assert!(retained < together);
    assert_eq!(manifest.node_classes()[0].identity().value(), &source);
    drop(manifest);
    assert_eq!(
        host.dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes(),
        baseline
    );
    assert_store_released(owner, account);
}

#[test]
fn long_manifest_identity_is_admitted_before_its_path_is_cloned() {
    let limits = ResourceLimits::default();
    let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
    let host = owner.host();
    let baseline = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
    let source = ManifestResourceIdentity::new(
        AssetPath::new(format!(
            "models/{}primary.asset",
            "retained-path/".repeat(24)
        ))
        .unwrap(),
        ManifestResourceIdentityStability::SessionOpaque(
            ManifestSessionOpaqueReason::UnregisteredResolverIdentity,
        ),
    );
    host.tighten_dimension_policy(
        ImportDimension::ScratchBytes,
        DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, baseline),
    )
    .unwrap();
    let path_envelope = CapacityPlan::for_type::<u8>(0, 0, source.value().as_str().len())
        .unwrap()
        .envelope_delta_bytes()
        .unwrap();
    assert!(matches!(
        clone_manifest_identity(host, &source, "test-manifest-path"),
        Err(ResourceGraphError::Scope(ScopeAbortCause::Ledger(
            baozi_operation::LedgerCause::LimitExceeded {
                class: baozi_operation::CounterClass::Live,
                dimension: ImportDimension::ScratchBytes,
                value,
                limit,
            }
        ))) if value == baseline + path_envelope && limit == baseline
    ));
    assert_eq!(
        host.dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes(),
        baseline
    );
    assert!(source.value().as_str().len() > 300);
    assert_store_released(owner, account);
}

#[test]
fn graph_path_capacity_stays_charged_until_its_owner_drops() {
    let limits = ResourceLimits::default();
    let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
    let host = owner.host();
    let baseline = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
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
            >= baseline + path_bytes
    );
    drop(graph);
    assert_eq!(
        host.dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes(),
        baseline
    );
    assert_store_released(owner, account);
}

#[test]
fn graph_path_budget_rejects_before_a_copy_can_escape() {
    let limits = ResourceLimits::default();
    let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
    let host = owner.host();
    let baseline = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
    host.tighten_dimension_policy(
        ImportDimension::ScratchBytes,
        DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, baseline),
    )
    .unwrap();
    let path = AssetPath::new("models/primary.asset").unwrap();
    let path_envelope = CapacityPlan::for_type::<u8>(0, 0, path.as_str().len())
        .unwrap()
        .envelope_delta_bytes()
        .unwrap();
    assert!(matches!(
        clone_graph_path(host, &path, "test-graph-path"),
        Err(ResourceGraphError::Scope(ScopeAbortCause::Ledger(
            baozi_operation::LedgerCause::LimitExceeded {
                class: baozi_operation::CounterClass::Live,
                dimension: ImportDimension::ScratchBytes,
                value,
                limit,
            }
        ))) if value == baseline + path_envelope && limit == baseline
    ));
    let state = host.dimension_state(ImportDimension::ScratchBytes).unwrap();
    assert_eq!(state.live_bytes(), baseline);
    assert_store_released(owner, account);
}

#[test]
fn pending_depth_updates_retain_only_their_own_scratch_capacity() {
    let limits = ResourceLimits::default();
    let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
    let host = owner.host();
    let startup = host
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
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
    assert_eq!(live, baseline + depth_updates.capacity_bytes());
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
        startup
    );
    assert_store_released(owner, account);
}
