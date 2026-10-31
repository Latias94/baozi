use super::*;
use baozi_io::{AssetOpenError, AssetOpenErrorKind, AssetOpenStage};

fn graph_work(host: ImportStoreHost<'_>) -> u64 {
    let state = host
        .dimension_state(ImportDimension::ResourceGraphWork)
        .unwrap();
    assert_eq!(state.attempted(), state.consumed());
    state.consumed()
}

#[test]
fn unique_resources_and_repeated_leaf_references_use_bounded_graph_work() {
    for leaf_count in [64_usize, 256, 1_024] {
        let limits = ResourceLimits {
            // Let regressions reach the finite per-phase assertions below instead of ending at
            // a shared session limit before their work can be compared.
            max_resource_graph_work: 16_000_000,
            ..ResourceLimits::default()
        };
        let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
        let host = owner.host();
        let authority = AuthorityDomain::new();
        let primary_path = AssetPath::new("main.bin").unwrap();
        let role = DependencyRoleId::new("test.resource").unwrap();
        let mut graph = ResourceGraph::new(limits);
        graph
            .bind_primary(host, &authority, &primary_path, b"primary")
            .unwrap();
        let primary = graph.source_for_primary().unwrap();
        let count = u64::try_from(leaf_count).unwrap();
        let identities: Vec<_> = (0..leaf_count)
            .map(|index| dependency_identity(&authority, &format!("r{index:04}.bin")))
            .collect();

        // Warm acquisition storage independently of logical graph insertion. This phase includes
        // index initialization, record moves/rebuilds, miss/insertion/transition probes, and no
        // graph nodes or relations. All paths have nine bytes and all snapshots have one byte.
        let before_warm = graph_work(host);
        for (index, identity) in identities.iter().enumerate() {
            let before = graph_work(host);
            let (cache_index, snapshot) = cache_snapshot(&mut graph, host, identity, b"x");
            assert_eq!(cache_index, index);
            assert_eq!(snapshot.bytes(), b"x");
            if index == 0 {
                // The empty cache initializes eight slots and performs one insertion probe and
                // one resolving-to-available probe. The initial empty lookup examines no slot.
                assert_eq!(graph_work(host) - before, 8 + 1 + 1);
            }
        }
        let warm_work = graph_work(host) - before_warm;
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.relation_count, 0);
        assert!(warm_work >= 8 + 2 * count);
        // Independent finite budgets for a half-full randomized index. Geometric growth needs
        // at most 4N initialized slots, N moved records, and N scanned rebuild records. Three
        // ordinary probe operations per identity and at most N rebuilt keys each get 16 probes
        // on average. This is a regression envelope, not a worst-case hashing guarantee.
        let maintenance_budget = (4 + 1 + 1) * count;
        let probe_budget = (3 + 1) * 16 * count;
        assert!(
            warm_work <= maintenance_budget + probe_budget,
            "cache warm-up at N={count}: {warm_work}"
        );

        // Sample only individual acquisition lookups against the now immutable cache. Bound the
        // actual probe costs independently before using them to predict graph operations. A
        // graph-wide prefix walk cannot hide by making this measurement grow with N.
        let mut probes = Vec::with_capacity(leaf_count);
        for (index, identity) in identities.iter().enumerate() {
            let before = graph_work(host);
            let Some(ResourceAcquisitionState::Available { cache_index, .. }) =
                graph.acquisition_state(host, identity).unwrap()
            else {
                panic!("warm-up must retain an available acquisition");
            };
            assert_eq!(cache_index, index);
            let actual = graph_work(host) - before;
            assert!((1..=64).contains(&actual), "lookup probes: {actual}");
            probes.push(actual);
        }
        let total_probes: u64 = probes.iter().sum();
        assert!(
            total_probes <= 16 * count,
            "acquisition lookup probes at N={count}: {total_probes}"
        );

        let before_find = graph_work(host);
        for identity in &identities {
            assert_eq!(graph.find_node(host, identity).unwrap(), None);
        }
        assert_eq!(
            graph_work(host) - before_find,
            count + total_probes,
            "unbound graph lookup performs one primary comparison plus acquisition probes"
        );

        let before_unique = graph_work(host);
        for identity in &identities {
            add_available(&mut graph, host, primary, &role, identity, b"x");
        }
        let unique_work = graph_work(host) - before_unique;
        // A nine-byte reference, one relation action, one primary comparison, and one cache
        // binding make 12 fixed actions. Target lookup and acquisition reuse each probe once.
        assert_eq!(unique_work, 12 * count + 2 * total_probes);
        let nodes = host
            .dimension_state(ImportDimension::ResourceGraphNodes)
            .unwrap();
        let relations = host
            .dimension_state(ImportDimension::ResourceGraphRelations)
            .unwrap();
        assert_eq!(nodes.attempted(), count + 1);
        assert_eq!(nodes.consumed(), count + 1);
        assert_eq!(relations.attempted(), count);
        assert_eq!(relations.consumed(), count);

        let last = identities.last().unwrap();
        let last_probes = *probes.last().unwrap();
        let before_lookup = graph_work(host);
        let last_node = graph.find_node(host, last).unwrap().unwrap();
        let lookup_work = graph_work(host) - before_lookup;
        assert_eq!(lookup_work, 1 + last_probes);
        let before_reuse = graph_work(host);
        for _ in 0..leaf_count {
            assert_eq!(
                add_available(&mut graph, host, primary, &role, last, b"x"),
                last_node,
            );
        }
        let reuse_work = graph_work(host) - before_reuse;
        // The existing-target depth action replaces new binding initialization. Acquisition
        // reuse is a separate probe pass even when its shared snapshot digest is already cached.
        assert_eq!(
            reuse_work,
            12 * count + 2 * count * last_probes,
            "reuse must perform only fixed actions and both bounded probe passes at N={count}"
        );
        let nodes = host
            .dimension_state(ImportDimension::ResourceGraphNodes)
            .unwrap();
        let relations = host
            .dimension_state(ImportDimension::ResourceGraphRelations)
            .unwrap();
        assert_eq!(nodes.attempted(), count + 1);
        assert_eq!(nodes.consumed(), count + 1);
        assert_eq!(relations.attempted(), 2 * count);
        assert_eq!(relations.consumed(), 2 * count);
        assert_eq!(graph.nodes.len(), leaf_count + 1);
        assert_eq!(graph.relations.len(), 2 * leaf_count);
        assert_eq!(graph.maximum_depth(), 1);

        // Manifest sorting is charged to InputManifestWork, outside these phase measurements.
        graph.seal(host).unwrap();
        let manifest = graph.take_manifest().unwrap();
        let counts = manifest.counts();
        assert_eq!(counts.node_class_count(), count + 1);
        assert_eq!(counts.quotient_relation_record_count(), count);
        assert_eq!(counts.relation_occurrence_count(), 2 * count);
        assert_eq!(counts.available_relation_record_count(), count);
        assert_eq!(counts.available_relation_occurrence_count(), 2 * count);
        assert_eq!(counts.unavailable_relation_record_count(), 0);
        assert_eq!(counts.unavailable_relation_occurrence_count(), 0);
        assert_eq!(counts.maximum_depth(), 1);
        let primary_class = manifest
            .node_classes()
            .iter()
            .find(|class| class.digest() == manifest.primary())
            .unwrap();
        assert_eq!(primary_class.identity().value(), &primary_path);
        assert_eq!(primary_class.outgoing_relations().len(), leaf_count);
        let mut repeated = 0;
        for relation in primary_class.outgoing_relations() {
            let ManifestReference::Normalized(reference) = relation.reference() else {
                panic!("every admitted reference must retain its normalized identity")
            };
            if reference.value() == last.path() {
                assert_eq!(relation.multiplicity(), count + 1);
                repeated += 1;
            } else {
                assert_eq!(relation.multiplicity(), 1);
            }
            assert!(matches!(
                relation.outcome(),
                ManifestRelationOutcome::Available { .. }
            ));
        }
        assert_eq!(repeated, 1);
        drop(graph);
        drop(manifest);
        assert_store_released(owner, account);
        eprintln!(
            "resource_graph_growth leaves={leaf_count} cache_warm_work={warm_work} \
             acquisition_probes={total_probes} unique_work={unique_work} \
             reuse_work={reuse_work} last_lookup_work={lookup_work}",
        );
    }
}

#[test]
fn indexed_node_lookup_requires_an_available_bound_exact_identity() {
    let limits = ResourceLimits::default();
    let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let other_authority = AuthorityDomain::new();
    let primary_path = AssetPath::new("main.bin").unwrap();
    let role = DependencyRoleId::new("test.resource").unwrap();
    let mut graph = ResourceGraph::new(limits);
    graph
        .bind_primary(host, &authority, &primary_path, b"primary")
        .unwrap();
    let primary = graph.source_for_primary().unwrap();
    let primary_identity = dependency_identity(&authority, primary_path.as_str());
    assert_eq!(
        graph.find_node(host, &primary_identity).unwrap(),
        Some(primary)
    );
    let other_primary = dependency_identity(&other_authority, primary_path.as_str());
    assert_eq!(graph.find_node(host, &other_primary).unwrap(), None);
    let other_node = add_available(&mut graph, host, primary, &role, &other_primary, b"x");
    assert_ne!(other_node, primary);
    assert_eq!(
        graph.find_node(host, &other_primary).unwrap(),
        Some(other_node)
    );

    let identity = dependency_identity(&authority, "leaf.bin");
    let (_, snapshot) = cache_snapshot(&mut graph, host, &identity, b"x");
    assert_eq!(graph.find_node(host, &identity).unwrap(), None);
    let node = add_available(&mut graph, host, primary, &role, &identity, b"x");
    assert_eq!(graph.find_node(host, &identity).unwrap(), Some(node));
    let foreign_identity = dependency_identity(&other_authority, identity.path().as_str());
    assert_eq!(graph.find_node(host, &foreign_identity).unwrap(), None);

    let resolving = dependency_identity(&authority, "resolving.bin");
    graph.reserve_acquisition_capacity(host).unwrap();
    assert!(
        graph
            .begin_acquisition(
                host,
                resolving.try_clone().unwrap(),
                reserve_graph_storage(host, resolving.path().as_str().len() as u64).unwrap(),
            )
            .unwrap()
    );
    assert_eq!(graph.find_node(host, &resolving).unwrap(), None);
    assert!(matches!(
        graph.acquisition_state(host, &resolving).unwrap(),
        Some(ResourceAcquisitionState::Resolving)
    ));
    graph
        .complete_failed_acquisition(
            host,
            &resolving,
            ScopeAbortCause::AssetOpen(AssetOpenError::new(
                AssetOpenStage::Open,
                AssetOpenErrorKind::NotFound,
            )),
            None,
            PendingNegativeObservation::None,
        )
        .unwrap();
    assert_eq!(graph.find_node(host, &resolving).unwrap(), None);
    assert!(matches!(
        graph.acquisition_state(host, &resolving).unwrap(),
        Some(ResourceAcquisitionState::Failed { .. })
    ));
    drop(snapshot);
    drop(graph);
    assert_store_released(owner, account);
}

#[test]
fn abandoned_cache_slot_reuse_binds_only_the_new_available_identity() {
    let limits = ResourceLimits::default();
    let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let primary_path = AssetPath::new("main.bin").unwrap();
    let role = DependencyRoleId::new("test.resource").unwrap();
    let mut graph = ResourceGraph::new(limits);
    graph
        .bind_primary(host, &authority, &primary_path, b"primary")
        .unwrap();
    let primary = graph.source_for_primary().unwrap();
    let anchor = dependency_identity(&authority, "anchor.bin");
    let (anchor_cache, snapshot) = cache_snapshot(&mut graph, host, &anchor, b"x");
    let anchor_node = add_available(&mut graph, host, primary, &role, &anchor, b"x");
    let abandoned = dependency_identity(&authority, "abandoned.bin");
    graph.reserve_acquisition_capacity(host).unwrap();
    assert!(
        graph
            .begin_acquisition(
                host,
                abandoned.try_clone().unwrap(),
                reserve_graph_storage(host, abandoned.path().as_str().len() as u64).unwrap(),
            )
            .unwrap()
    );
    let abandoned_slot = graph
        .cache
        .controlled_record_index(host, &abandoned)
        .unwrap()
        .unwrap();
    assert_ne!(abandoned_slot, anchor_cache);
    assert_eq!(graph.find_node(host, &abandoned).unwrap(), None);
    graph.abandon_acquisition(host, &abandoned).unwrap();
    assert!(graph.acquisition_state(host, &abandoned).unwrap().is_none());
    assert_eq!(graph.find_node(host, &abandoned).unwrap(), None);
    assert!(graph.source_for_cache(abandoned_slot).is_err());

    let replacement = dependency_identity(&authority, "replacement.bin");
    graph.reserve_acquisition_capacity(host).unwrap();
    assert!(
        graph
            .begin_acquisition(
                host,
                replacement.try_clone().unwrap(),
                reserve_graph_storage(host, replacement.path().as_str().len() as u64).unwrap(),
            )
            .unwrap()
    );
    let replacement_slot = graph
        .complete_acquisition(host, &replacement, Arc::clone(&snapshot), None)
        .unwrap();
    assert_eq!(replacement_slot, abandoned_slot);
    assert_eq!(graph.origin_identity(replacement_slot), Some(&replacement));
    assert_eq!(graph.find_node(host, &replacement).unwrap(), None);
    assert!(graph.source_for_cache(replacement_slot).is_err());
    let replacement_node = add_available(&mut graph, host, primary, &role, &replacement, b"x");
    assert_ne!(replacement_node, anchor_node);
    assert_eq!(
        graph.source_for_cache(replacement_slot).unwrap(),
        replacement_node
    );
    assert_eq!(
        graph.find_node(host, &replacement).unwrap(),
        Some(replacement_node)
    );
    assert_eq!(graph.find_node(host, &abandoned).unwrap(), None);
    assert!(graph.acquisition_state(host, &abandoned).unwrap().is_none());
    assert_eq!(graph.source_for_cache(anchor_cache).unwrap(), anchor_node);
    assert_eq!(graph.find_node(host, &anchor).unwrap(), Some(anchor_node));
    graph.seal(host).unwrap();
    let manifest = graph.take_manifest().unwrap();
    assert_eq!(manifest.counts().node_class_count(), 3);
    assert_eq!(manifest.counts().available_relation_occurrence_count(), 2);
    assert!(
        manifest
            .node_classes()
            .iter()
            .all(|class| class.identity().value() != abandoned.path())
    );
    drop(manifest);
    drop(snapshot);
    drop(graph);
    assert_store_released(owner, account);
}

#[test]
fn acquisition_operations_observe_cancellation_before_the_next_probe() {
    for operation in ["lookup", "begin", "available", "abandon", "failed"] {
        let limits = ResourceLimits::default();
        let cancellation = baozi_operation::CancellationToken::new();
        let control = ImportOperationControl::default().with_cancellation(cancellation.clone());
        let (owner, account) = store_owner_and_account(&limits, control);
        let host = owner.host();
        let authority = AuthorityDomain::new();
        let mut graph = ResourceGraph::new(limits);
        graph
            .bind_primary(
                host,
                &authority,
                &AssetPath::new("main.bin").unwrap(),
                b"primary",
            )
            .unwrap();
        let anchor = dependency_identity(&authority, "anchor.bin");
        let (_, snapshot) = cache_snapshot(&mut graph, host, &anchor, b"x");
        let identity = dependency_identity(&authority, "resolving.bin");
        graph.reserve_acquisition_capacity(host).unwrap();
        assert!(
            graph
                .begin_acquisition(
                    host,
                    identity.try_clone().unwrap(),
                    reserve_graph_storage(host, identity.path().as_str().len() as u64).unwrap(),
                )
                .unwrap()
        );
        let candidate = dependency_identity(&authority, "candidate.bin");
        graph.reserve_acquisition_capacity(host).unwrap();
        let mut candidate_reservation =
            Some(reserve_graph_storage(host, candidate.path().as_str().len() as u64).unwrap());
        let before = graph_work(host);
        cancellation.cancel();
        let result = match operation {
            "lookup" => graph
                .acquisition_state(host, &identity)
                .map(|_| ())
                .map_err(ResourceGraphError::into_scope_cause),
            "begin" => graph
                .begin_acquisition(host, candidate, candidate_reservation.take().unwrap())
                .map(|_| ())
                .map_err(ResourceGraphError::into_scope_cause),
            "available" => graph
                .complete_acquisition(host, &identity, Arc::clone(&snapshot), None)
                .map(|_| ())
                .map_err(ResourceGraphError::into_scope_cause),
            "abandon" => graph
                .abandon_acquisition(host, &identity)
                .map_err(ResourceGraphError::into_scope_cause),
            "failed" => graph
                .complete_failed_acquisition(
                    host,
                    &identity,
                    ScopeAbortCause::AssetOpen(AssetOpenError::new(
                        AssetOpenStage::Open,
                        AssetOpenErrorKind::NotFound,
                    )),
                    None,
                    PendingNegativeObservation::None,
                )
                .map(|_| ()),
            _ => unreachable!(),
        };
        assert!(
            matches!(
                result,
                Err(ScopeAbortCause::Control(
                    baozi_operation::ControlCause::Cancelled { .. }
                ))
            ),
            "{operation} must preserve cancellation instead of reporting a missing cache transition"
        );
        assert_eq!(graph_work(host), before);
        drop(candidate_reservation);
        drop(snapshot);
        drop(graph);
        assert_store_released(owner, account);
    }
}

#[test]
fn acquisition_lookup_preserves_graph_work_denial_as_a_scope_ledger_failure() {
    let limits = ResourceLimits::default();
    let (owner, account) = store_owner_and_account(&limits, ImportOperationControl::default());
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let identity = dependency_identity(&authority, "available.bin");
    let mut graph = ResourceGraph::new(limits);
    let (_, snapshot) = cache_snapshot(&mut graph, host, &identity, b"x");
    let before = graph_work(host);
    host.tighten_dimension_policy(
        ImportDimension::ResourceGraphWork,
        DimensionPolicy::counter(before, before, 0),
    )
    .unwrap();
    assert!(matches!(
        graph.acquisition_state(host, &identity),
        Err(ResourceGraphError::Scope(ScopeAbortCause::Ledger(
            baozi_operation::LedgerCause::LimitExceeded {
                class: baozi_operation::CounterClass::Attempted,
                dimension: ImportDimension::ResourceGraphWork,
                value,
                limit,
            }
        ))) if value == before + 1 && limit == before
    ));
    let work = host
        .dimension_state(ImportDimension::ResourceGraphWork)
        .unwrap();
    assert_eq!(work.attempted(), before + 1);
    assert_eq!(work.consumed(), before);
    assert_eq!(graph.nodes.len(), 0);
    assert_eq!(graph.relation_count, 0);
    drop(snapshot);
    drop(graph);
    assert_store_released(owner, account);
}
