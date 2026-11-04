use super::*;

#[test]
fn provisional_manifest_keeps_paths_and_tables_live_after_the_graph_is_destroyed() {
    let limits = ResourceLimits::default();
    with_store_body(
        limits.clone(),
        ImportOperationControl::default(),
        |exec, _account| {
            let baseline = exec
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
                .bind_primary(exec, &authority, &source, b"immutable-primary")
                .unwrap();
            graph.seal(exec).unwrap();
            let manifest = graph.take_manifest().unwrap();
            let together = exec
                .dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes();
            drop(graph);
            let retained = exec
                .dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes();
            assert!(retained - baseline >= source.as_str().len() as u64);
            assert!(retained < together);
            assert_eq!(manifest.node_classes()[0].identity().value(), &source);
            drop(manifest);
            assert_eq!(
                exec.dimension_state(ImportDimension::ScratchBytes)
                    .unwrap()
                    .live_bytes(),
                baseline
            );
        },
    );
}

#[test]
fn long_manifest_identity_is_admitted_before_its_path_is_cloned() {
    let limits = ResourceLimits::default();
    with_store_body(
        limits.clone(),
        ImportOperationControl::default(),
        |exec, _account| {
            let baseline = exec
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
            exec.tighten_dimension_policy(
                ImportDimension::ScratchBytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, baseline),
            )
            .unwrap();
            let path_envelope = CapacityPlan::for_type::<u8>(0, 0, source.value().as_str().len())
                .unwrap()
                .envelope_delta_bytes()
                .unwrap();
            assert!(matches!(
                clone_manifest_identity(exec, &source, "test-manifest-path"),
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
                exec.dimension_state(ImportDimension::ScratchBytes)
                    .unwrap()
                    .live_bytes(),
                baseline
            );
            assert!(source.value().as_str().len() > 300);
        },
    );
}

#[test]
fn graph_path_capacity_stays_charged_until_its_owner_drops() {
    let limits = ResourceLimits::default();
    with_store_body(
        limits.clone(),
        ImportOperationControl::default(),
        |exec, _account| {
            let baseline = exec
                .dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes();
            let authority = AuthorityDomain::new();
            let path = AssetPath::new("models/primary.asset").unwrap();
            let mut graph = ResourceGraph::new(limits);
            graph
                .bind_primary(exec, &authority, &path, b"primary")
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
                exec.dimension_state(ImportDimension::ScratchBytes)
                    .unwrap()
                    .live_bytes()
                    >= baseline + path_bytes
            );
            drop(graph);
            assert_eq!(
                exec.dimension_state(ImportDimension::ScratchBytes)
                    .unwrap()
                    .live_bytes(),
                baseline
            );
        },
    );
}

#[test]
fn graph_path_budget_rejects_before_a_copy_can_escape() {
    let limits = ResourceLimits::default();
    with_store_body(
        limits.clone(),
        ImportOperationControl::default(),
        |exec, _account| {
            let baseline = exec
                .dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes();
            exec.tighten_dimension_policy(
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
                clone_graph_path(exec, &path, "test-graph-path"),
                Err(ResourceGraphError::Scope(ScopeAbortCause::Ledger(
                    baozi_operation::LedgerCause::LimitExceeded {
                        class: baozi_operation::CounterClass::Live,
                        dimension: ImportDimension::ScratchBytes,
                        value,
                        limit,
                    }
                ))) if value == baseline + path_envelope && limit == baseline
            ));
            let state = exec.dimension_state(ImportDimension::ScratchBytes).unwrap();
            assert_eq!(state.live_bytes(), baseline);
        },
    );
}

#[test]
fn pending_depth_updates_retain_only_their_own_scratch_capacity() {
    let limits = ResourceLimits::default();
    with_store_body(
        limits.clone(),
        ImportOperationControl::default(),
        |exec, _account| {
            let startup = exec
                .dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes();
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
            let baseline = exec
                .dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes();
            graph.prepare_target(exec, &mut pending, &target).unwrap();
            let Some(TargetAdmission::Existing { depth_updates, .. }) = &pending.target else {
                panic!("an existing shared target must retain its planned depth updates")
            };
            assert!(!depth_updates.is_empty());
            let live = exec
                .dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes();
            assert_eq!(live, baseline + depth_updates.capacity_bytes());
            drop(pending);
            assert_eq!(
                exec.dimension_state(ImportDimension::ScratchBytes)
                    .unwrap()
                    .live_bytes(),
                baseline
            );
            drop(graph);
            assert_eq!(
                exec.dimension_state(ImportDimension::ScratchBytes)
                    .unwrap()
                    .live_bytes(),
                startup
            );
        },
    );
}
