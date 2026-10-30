use super::*;
use crate::resource_hash::{HASH_CHUNK_BYTES, chunked_sha256};
use baozi_operation::{CancellationSignal, ControlCause, LedgerCause};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CancelOnPoll {
    calls: Arc<AtomicUsize>,
    cancel_at: Arc<AtomicUsize>,
}

impl CancellationSignal for CancelOnPoll {
    fn is_cancelled(&self) -> bool {
        let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        call >= self.cancel_at.load(Ordering::Acquire)
    }
}

#[test]
fn chunked_hashes_preserve_sha256_and_charge_exact_bytes() {
    for dimension in [
        ImportDimension::WorkUnits,
        ImportDimension::InputManifestWork,
    ] {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        for length in [
            0,
            1,
            63,
            64,
            65,
            HASH_CHUNK_BYTES - 1,
            HASH_CHUNK_BYTES,
            HASH_CHUNK_BYTES + 1,
            2 * HASH_CHUNK_BYTES + 17,
        ] {
            let bytes: Vec<_> = (0..length).map(|index| (index % 251) as u8).collect();
            let before = host.dimension_state(dimension).unwrap();
            let digest = chunked_sha256(host, &bytes, dimension).unwrap();
            assert_eq!(
                digest,
                ManifestDigest::of(&bytes).as_bytes(),
                "length {length}"
            );
            let work = host.dimension_state(dimension).unwrap();
            assert_eq!(work.attempted() - before.attempted(), length as u64);
            assert_eq!(work.consumed() - before.consumed(), length as u64);
        }
    }
}

#[test]
fn hash_work_rejection_keeps_the_completed_chunk_prefix() {
    for dimension in [
        ImportDimension::WorkUnits,
        ImportDimension::InputManifestWork,
    ] {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let before = host.dimension_state(dimension).unwrap();
        host.tighten_dimension_policy(
            dimension,
            DimensionPolicy::counter(
                before.attempted() + HASH_CHUNK_BYTES as u64,
                before.consumed() + HASH_CHUNK_BYTES as u64,
                0,
            ),
        )
        .unwrap();
        let bytes = vec![7; 2 * HASH_CHUNK_BYTES + 17];

        assert!(matches!(
            chunked_sha256(host, &bytes, dimension),
            Err(ScopeAbortCause::Ledger(
                LedgerCause::LimitExceeded { dimension: rejected, .. }
            )) if rejected == dimension
        ));
        let work = host.dimension_state(dimension).unwrap();
        assert_eq!(
            work.attempted() - before.attempted(),
            2 * HASH_CHUNK_BYTES as u64
        );
        assert_eq!(work.consumed() - before.consumed(), HASH_CHUNK_BYTES as u64);
    }
}

#[test]
fn hash_cancellation_stops_before_the_next_chunk_admission() {
    for dimension in [
        ImportDimension::WorkUnits,
        ImportDimension::InputManifestWork,
    ] {
        for completed_chunks in [0, 1, 2] {
            let limits = ResourceLimits::default();
            let calls = Arc::new(AtomicUsize::new(0));
            let cancel_at = Arc::new(AtomicUsize::new(usize::MAX));
            let control = ImportOperationControl::default().with_cancellation(CancelOnPoll {
                calls: Arc::clone(&calls),
                cancel_at: Arc::clone(&cancel_at),
            });
            let owner = store_owner_with_control(&limits, control);
            let host = owner.host();
            let work_before = host.dimension_state(dimension).unwrap();
            let before = calls.load(Ordering::Acquire);
            cancel_at.store(before + completed_chunks + 1, Ordering::Release);
            let bytes = vec![11; 3 * HASH_CHUNK_BYTES + 17];

            assert!(matches!(
                chunked_sha256(host, &bytes, dimension),
                Err(ScopeAbortCause::Control(ControlCause::Cancelled { .. }))
            ));
            let work = host.dimension_state(dimension).unwrap();
            let completed = (completed_chunks * HASH_CHUNK_BYTES) as u64;
            assert_eq!(work.attempted() - work_before.attempted(), completed);
            assert_eq!(work.consumed() - work_before.consumed(), completed);
            assert_eq!(calls.load(Ordering::Acquire) - before, completed_chunks + 1);
        }
    }
}

#[test]
fn empty_resource_and_manifest_hashes_observe_cancellation() {
    for dimension in [
        ImportDimension::WorkUnits,
        ImportDimension::InputManifestWork,
    ] {
        let limits = ResourceLimits::default();
        let cancellation = baozi_operation::CancellationToken::new();
        let control = ImportOperationControl::default().with_cancellation(cancellation.clone());
        let owner = store_owner_with_control(&limits, control);
        let host = owner.host();
        let before = host.dimension_state(dimension).unwrap();
        cancellation.cancel();

        assert!(matches!(
            chunked_sha256(host, &[], dimension),
            Err(ScopeAbortCause::Control(ControlCause::Cancelled { .. }))
        ));
        let work = host.dimension_state(dimension).unwrap();
        assert_eq!(work.attempted() - before.attempted(), 0);
        assert_eq!(work.consumed() - before.consumed(), 0);
    }
}

#[test]
fn primary_snapshot_hash_is_metered_once_before_manifest_sealing() {
    let limits = ResourceLimits::default();
    let owner = store_owner(&limits);
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let path = AssetPath::new("primary.bin").unwrap();
    let bytes = vec![23; 2 * HASH_CHUNK_BYTES + 17];
    let mut graph = ResourceGraph::new(limits);

    let before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    graph.bind_primary(host, &authority, &path, &bytes).unwrap();
    assert_eq!(graph.nodes[0].content_digest, ManifestDigest::of(&bytes));
    let before_sealing = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    assert_eq!(
        before_sealing.attempted() - before.attempted(),
        bytes.len() as u64
    );
    assert_eq!(
        before_sealing.consumed() - before.consumed(),
        bytes.len() as u64
    );
    // Manifest ownership may share handles, but cannot admit even the first hash chunk again.
    let sealing_budget = HASH_CHUNK_BYTES as u64 - 1;
    host.tighten_dimension_policy(
        ImportDimension::WorkUnits,
        DimensionPolicy::counter(
            before_sealing.attempted() + sealing_budget,
            before_sealing.consumed() + sealing_budget,
            0,
        ),
    )
    .unwrap();
    graph.seal(host).unwrap();
    let manifest = graph.take_manifest().unwrap();
    assert_eq!(
        manifest.node_classes()[0].content_digest(),
        ManifestDigest::of(&bytes)
    );
    let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    let attempted = work.attempted() - before_sealing.attempted();
    let consumed = work.consumed() - before_sealing.consumed();
    assert!(attempted <= sealing_budget);
    assert_eq!(consumed, attempted);
}

#[test]
fn cancelled_primary_hash_does_not_publish_a_partial_node() {
    let limits = ResourceLimits::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let cancel_at = Arc::new(AtomicUsize::new(usize::MAX));
    let control = ImportOperationControl::default().with_cancellation(CancelOnPoll {
        calls: Arc::clone(&calls),
        cancel_at: Arc::clone(&cancel_at),
    });
    let owner = store_owner_with_control(&limits, control);
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let path = AssetPath::new("primary.bin").unwrap();
    let bytes = vec![29; 2 * HASH_CHUNK_BYTES + 17];
    let mut graph = ResourceGraph::new(limits);
    graph
        .nodes
        .try_reserve(1, "resource-graph-nodes", |bytes| {
            reserve_graph_storage(host, bytes)
        })
        .unwrap();
    let before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    let polls_before = calls.load(Ordering::Acquire);
    cancel_at.store(polls_before + 2, Ordering::Release);

    assert!(matches!(
        graph.bind_primary(host, &authority, &path, &bytes),
        Err(ResourceGraphError::Scope(ScopeAbortCause::Control(
            ControlCause::Cancelled { .. }
        )))
    ));
    assert!(graph.nodes.is_empty());
    assert!(graph.primary.is_none());
    let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    assert_eq!(
        work.attempted() - before.attempted(),
        HASH_CHUNK_BYTES as u64
    );
    assert_eq!(work.consumed() - before.consumed(), HASH_CHUNK_BYTES as u64);
    assert_eq!(calls.load(Ordering::Acquire) - polls_before, 2);
    let nodes = host
        .dimension_state(ImportDimension::ResourceGraphNodes)
        .unwrap();
    assert_eq!(nodes.attempted(), 1);
    assert_eq!(nodes.consumed(), 0);
}

#[test]
fn repeated_available_references_reuse_the_snapshot_digest() {
    let limits = ResourceLimits::default();
    let owner = store_owner(&limits);
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let path = AssetPath::new("primary.bin").unwrap();
    let identity = dependency_identity(&authority, "shared.bin");
    let role = DependencyRoleId::new("test.resource").unwrap();
    let bytes = vec![31; 2 * HASH_CHUNK_BYTES + 17];
    let mut graph = ResourceGraph::new(limits);
    graph
        .bind_primary(host, &authority, &path, b"primary")
        .unwrap();
    let primary = graph.source_for_primary().unwrap();
    let (cache_index, snapshot) = cache_snapshot(&mut graph, host, &identity, &bytes);
    let before_hash = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    let node = add_available(&mut graph, host, primary, &role, &identity, &bytes);
    let before_reuse = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    assert_eq!(
        before_reuse.attempted() - before_hash.attempted(),
        bytes.len() as u64
    );
    assert_eq!(
        before_reuse.consumed() - before_hash.consumed(),
        bytes.len() as u64
    );
    // Admit incidental handle work, but reject even the first chunk of a repeated payload hash.
    let reuse_budget = HASH_CHUNK_BYTES as u64 - 1;
    host.tighten_dimension_policy(
        ImportDimension::WorkUnits,
        DimensionPolicy::counter(
            before_reuse.attempted() + reuse_budget,
            before_reuse.consumed() + reuse_budget,
            0,
        ),
    )
    .unwrap();

    for _ in 1..128 {
        let Some(ResourceAcquisitionState::Available {
            cache_index: reused_index,
            bytes: reused,
            ..
        }) = graph.acquisition_state(&identity)
        else {
            panic!("an available acquisition must remain cached");
        };
        assert_eq!(reused_index, cache_index);
        assert!(Arc::ptr_eq(&snapshot, &reused));
        assert_eq!(
            add_available(&mut graph, host, primary, &role, &identity, reused.bytes()),
            node
        );
    }

    graph.seal(host).unwrap();
    let manifest = graph.take_manifest().unwrap();
    assert_eq!(manifest.counts().node_class_count(), 2);
    assert_eq!(manifest.counts().available_relation_occurrence_count(), 128);
    assert_eq!(manifest.counts().quotient_relation_record_count(), 1);
    let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    let attempted = work.attempted() - before_reuse.attempted();
    let consumed = work.consumed() - before_reuse.consumed();
    assert!(attempted <= reuse_budget);
    assert_eq!(consumed, attempted);
}

#[test]
fn digest_follows_the_shared_snapshot_across_distinct_cache_entries() {
    let limits = ResourceLimits::default();
    let owner = store_owner(&limits);
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let path = AssetPath::new("primary.bin").unwrap();
    let identity = dependency_identity(&authority, "first.bin");
    let alias = dependency_identity(&authority, "alias.bin");
    let role = DependencyRoleId::new("test.resource").unwrap();
    let bytes = vec![37; HASH_CHUNK_BYTES + 17];
    let mut graph = ResourceGraph::new(limits);
    graph.bind_primary(host, &authority, &path, b"").unwrap();
    let primary = graph.source_for_primary().unwrap();
    let (_, snapshot) = cache_snapshot(&mut graph, host, &identity, &bytes);
    let before_hash = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    let first = add_available(&mut graph, host, primary, &role, &identity, &bytes);
    let after_hash = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    assert_eq!(
        after_hash.attempted() - before_hash.attempted(),
        bytes.len() as u64
    );
    assert_eq!(
        after_hash.consumed() - before_hash.consumed(),
        bytes.len() as u64
    );
    graph.reserve_acquisition_capacity(host).unwrap();
    assert!(
        graph
            .begin_acquisition(
                alias.try_clone().unwrap(),
                reserve_graph_storage(host, alias.path().as_str().len() as u64).unwrap(),
            )
            .unwrap()
    );
    graph
        .complete_acquisition(&alias, Arc::clone(&snapshot), None)
        .unwrap();
    let before_reuse = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    let reuse_budget = HASH_CHUNK_BYTES as u64 - 1;
    host.tighten_dimension_policy(
        ImportDimension::WorkUnits,
        DimensionPolicy::counter(
            before_reuse.attempted() + reuse_budget,
            before_reuse.consumed() + reuse_budget,
            0,
        ),
    )
    .unwrap();

    let second = add_available(&mut graph, host, primary, &role, &alias, &bytes);

    assert_ne!(first, second);
    assert_eq!(
        graph.nodes[first].content_digest,
        graph.nodes[second].content_digest
    );
    let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    let attempted = work.attempted() - before_reuse.attempted();
    let consumed = work.consumed() - before_reuse.consumed();
    assert!(attempted <= reuse_budget);
    assert_eq!(consumed, attempted);
}

#[test]
fn cancelled_dependency_hash_does_not_seal_a_partial_digest() {
    let limits = ResourceLimits::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let cancel_at = Arc::new(AtomicUsize::new(usize::MAX));
    let control = ImportOperationControl::default().with_cancellation(CancelOnPoll {
        calls: Arc::clone(&calls),
        cancel_at: Arc::clone(&cancel_at),
    });
    let owner = store_owner_with_control(&limits, control);
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let path = AssetPath::new("primary.bin").unwrap();
    let identity = dependency_identity(&authority, "dependency.bin");
    let role = DependencyRoleId::new("test.resource").unwrap();
    let bytes = vec![41; 2 * HASH_CHUNK_BYTES + 17];
    let mut graph = ResourceGraph::new(limits);
    graph.bind_primary(host, &authority, &path, b"").unwrap();
    let primary = graph.source_for_primary().unwrap();
    let (cache_index, snapshot) = cache_snapshot(&mut graph, host, &identity, &bytes);
    let mut pending = graph
        .begin_relation(
            host,
            primary,
            &role,
            DependencyRequirement::Required,
            "dependency.bin",
        )
        .unwrap();
    graph.prepare_target(host, &mut pending, &identity).unwrap();
    let before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    let polls_before = calls.load(Ordering::Acquire);
    cancel_at.store(polls_before + 2, Ordering::Release);

    assert!(matches!(
        graph.complete_available(
            host,
            pending,
            cache_index,
            snapshot.bytes(),
            AvailableResourceIdentity::new(&identity, identity.path(), None),
        ),
        Err(ResourceGraphError::Scope(ScopeAbortCause::Control(
            ControlCause::Cancelled { .. }
        )))
    ));
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.relation_count, 0);
    assert!(graph.source_for_cache(cache_index).is_err());
    assert_eq!(calls.load(Ordering::Acquire) - polls_before, 2);
    // A sealed partial digest would bypass the operation's sticky cancellation on this access.
    assert!(matches!(
        snapshot.content_digest(host),
        Err(ScopeAbortCause::Control(ControlCause::Cancelled { .. }))
    ));
    let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    assert_eq!(
        work.attempted() - before.attempted(),
        HASH_CHUNK_BYTES as u64
    );
    assert_eq!(work.consumed() - before.consumed(), HASH_CHUNK_BYTES as u64);
}

#[test]
fn available_reference_rejects_cache_index_and_byte_owner_mismatches() {
    for mismatch in ["index", "owner", "slice"] {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let authority = AuthorityDomain::new();
        let path = AssetPath::new("primary.bin").unwrap();
        let identity = dependency_identity(&authority, "dependency.bin");
        let other_identity = dependency_identity(&authority, "other.bin");
        let role = DependencyRoleId::new("test.resource").unwrap();
        let mut graph = ResourceGraph::new(limits);
        graph.bind_primary(host, &authority, &path, b"").unwrap();
        let primary = graph.source_for_primary().unwrap();
        let (cache_index, snapshot) = cache_snapshot(&mut graph, host, &identity, b"same");
        let (other_index, other_snapshot) =
            cache_snapshot(&mut graph, host, &other_identity, b"same");
        let mut pending = graph
            .begin_relation(
                host,
                primary,
                &role,
                DependencyRequirement::Required,
                "dependency.bin",
            )
            .unwrap();
        graph.prepare_target(host, &mut pending, &identity).unwrap();
        let (index, bytes) = match mismatch {
            "index" => (other_index, snapshot.bytes()),
            "owner" => (cache_index, other_snapshot.bytes()),
            "slice" => (cache_index, &snapshot.bytes()[..3]),
            _ => unreachable!(),
        };

        let before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
        assert!(
            matches!(
                graph.complete_available(
                    host,
                    pending,
                    index,
                    bytes,
                    AvailableResourceIdentity::new(&identity, identity.path(), None),
                ),
                Err(ResourceGraphError::DigestCollision)
            ),
            "{mismatch}"
        );
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.relation_count, 0);
        let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
        assert_eq!(work.attempted() - before.attempted(), 0);
        assert_eq!(work.consumed() - before.consumed(), 0);
    }
}

#[test]
fn cached_snapshot_keeps_existing_node_digest_and_cycle_checks() {
    let limits = ResourceLimits::default();
    let owner = store_owner(&limits);
    let host = owner.host();
    let authority = AuthorityDomain::new();
    let path = AssetPath::new("primary.bin").unwrap();
    let primary_identity = DependencyIdentity::new(authority.clone(), path.try_clone().unwrap());
    let identity = dependency_identity(&authority, "dependency.bin");
    let role = DependencyRoleId::new("test.resource").unwrap();
    let mut graph = ResourceGraph::new(limits);
    graph.bind_primary(host, &authority, &path, b"").unwrap();
    let primary = graph.source_for_primary().unwrap();
    let dependency = add_available(&mut graph, host, primary, &role, &identity, b"dependency");
    let mut cycle = graph
        .begin_relation(
            host,
            dependency,
            &role,
            DependencyRequirement::Required,
            path.as_str(),
        )
        .unwrap();
    assert!(matches!(
        graph.prepare_target(host, &mut cycle, &primary_identity),
        Err(ResourceGraphError::Cycle)
    ));

    let mut pending = graph
        .begin_relation(
            host,
            primary,
            &role,
            DependencyRequirement::Required,
            "dependency.bin",
        )
        .unwrap();
    graph.prepare_target(host, &mut pending, &identity).unwrap();
    let (cache_index, snapshot) = cache_snapshot(&mut graph, host, &identity, b"dependency");
    graph.nodes[dependency].content_digest = ManifestDigest::of(b"conflicting-digest");
    let before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    assert!(matches!(
        graph.complete_available(
            host,
            pending,
            cache_index,
            snapshot.bytes(),
            AvailableResourceIdentity::new(&identity, identity.path(), None),
        ),
        Err(ResourceGraphError::DigestCollision)
    ));
    let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
    assert_eq!(work.attempted() - before.attempted(), 0);
    assert_eq!(work.consumed() - before.consumed(), 0);
}
