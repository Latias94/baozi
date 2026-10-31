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
    let measurements = [64_usize, 256, 1_024].map(|leaf_count| {
        let limits = ResourceLimits {
            // Keep the old graph-wide planner observable through the largest case before
            // checking the smaller phase budgets below.
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
        let before_unique = graph_work(host);

        for index in 0..leaf_count {
            let identity = dependency_identity(&authority, &format!("r{index:04}.bin"));
            add_available(&mut graph, host, primary, &role, &identity, b"x");
        }
        let unique_work = graph_work(host) - before_unique;
        let count = u64::try_from(leaf_count).unwrap();
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

        let last = dependency_identity(&authority, &format!("r{:04}.bin", leaf_count - 1));
        // This actual lookup is outside both phases. The unchanged cache must use the same
        // probe chain for every subsequent reference, regardless of RandomState's seed.
        let before_lookup = graph_work(host);
        let last_node = graph.find_node(host, &last).unwrap().unwrap();
        let lookup_work = graph_work(host) - before_lookup;
        let before_reuse = graph_work(host);
        for _ in 0..leaf_count {
            assert_eq!(
                add_available(&mut graph, host, primary, &role, &last, b"x"),
                last_node,
            );
        }
        let reuse_work = graph_work(host) - before_reuse;
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
            "resource_graph_growth leaves={leaf_count} unique_work={unique_work} \
             reuse_work={reuse_work} last_lookup_work={lookup_work}",
        );
        (count, unique_work, reuse_work, lookup_work)
    });

    for (count, unique_work, reuse_work, lookup_work) in measurements {
        // Each nine-byte reference hashes nine bytes, admits one relation, compares the
        // primary once, and either initializes one cache binding or checks unchanged depth.
        let fixed_work = 12 * count;
        let unique_probes = unique_work.checked_sub(fixed_work).unwrap();
        let last_probes = lookup_work.checked_sub(1).unwrap();
        assert!(unique_probes >= count - 1); // Only the first lookup has an empty cache.
        assert!(last_probes >= 1);
        // These are finite regression budgets for a randomized, at-most-half-full table,
        // not worst-case linearity guarantees. Unique queries average across keys; the hot
        // key gets a larger allowance because repeating it does not average collisions.
        assert!(
            unique_probes <= 16 * count,
            "unique probes at N={count}: {unique_probes}"
        );
        assert!(
            last_probes <= 64,
            "hot-key probes at N={count}: {last_probes}"
        );
        assert_eq!(
            reuse_work,
            fixed_work + count * last_probes,
            "reuse must perform only its fixed actions and the measured probes at N={count}",
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
                resolving.try_clone().unwrap(),
                reserve_graph_storage(host, resolving.path().as_str().len() as u64).unwrap(),
            )
            .unwrap()
    );
    assert_eq!(graph.find_node(host, &resolving).unwrap(), None);
    graph
        .complete_failed_acquisition(
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
    drop(snapshot);
    drop(graph);
    assert_store_released(owner, account);
}

#[test]
fn indexed_lookup_observes_cancellation_before_the_next_probe() {
    let limits = ResourceLimits::default();
    let cancellation = baozi_operation::CancellationToken::new();
    let control = ImportOperationControl::default().with_cancellation(cancellation.clone());
    let (owner, account) = store_owner_and_account(&limits, control);
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let primary_path = AssetPath::new("main.bin").unwrap();
    let role = DependencyRoleId::new("test.resource").unwrap();
    let identity = dependency_identity(&authority, "leaf.bin");
    let mut graph = ResourceGraph::new(limits);
    graph
        .bind_primary(host, &authority, &primary_path, b"primary")
        .unwrap();
    let primary = graph.source_for_primary().unwrap();
    add_available(&mut graph, host, primary, &role, &identity, b"x");
    let before = graph_work(host);
    cancellation.cancel();
    assert!(matches!(
        graph.cache.controlled_record_index(host, &identity),
        Err(ScopeAbortCause::Control(
            baozi_operation::ControlCause::Cancelled { .. }
        ))
    ));
    assert_eq!(graph_work(host), before);
    drop(graph);
    assert_store_released(owner, account);
}
