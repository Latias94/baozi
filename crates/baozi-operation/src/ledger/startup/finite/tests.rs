use super::*;
use crate::DimensionPolicy;
use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};
use crate::ledger::{
    CompletedPhase, PublicationState, StorageDimensionState, StoragePolicy, WorkDimensionState,
    WorkLane, WorkMode, WorkPolicy, test_attribution_batch_allocation_entries,
    test_evidence_allocation_failure_is_armed, test_evidence_allocation_guard,
    test_fail_next_evidence_allocation, test_set_evidence_allocation_failure,
};

#[derive(Debug)]
struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Build,
    Finalize,
    Publish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    Bytes,
    Records,
    Scratch,
    Canonical,
    Evidence,
    Progress,
    Unknown,
}

fn finite_ledger(operation_live_byte_limit: u64) -> FiniteLedger<Owner, Phase, Dimension> {
    let policies = [
        Dimension::Progress,
        Dimension::Bytes,
        Dimension::Evidence,
        Dimension::Records,
        Dimension::Canonical,
        Dimension::Scratch,
    ]
    .map(|dimension| {
        let policy = if dimension == Dimension::Records {
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
        } else {
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        };
        (
            dimension,
            policy.with_published_outcome_evidence_limit(u64::MAX),
        )
    });
    FiniteLedger::try_new(
        FiniteLedgerLimits::try_new(operation_live_byte_limit, u64::MAX, policies)
            .expect("six finite policies"),
        [Phase::Publish, Phase::Build, Phase::Finalize],
    )
    .expect("three finite phases")
}

fn graph_request(
    canonical_nonempty: bool,
    evidence_nonempty: bool,
) -> (
    FiniteStartupLedgerPlanParts<Owner, Phase, Dimension>,
    [u64; 3],
) {
    let canonical_slots = usize::from(canonical_nonempty);
    let evidence_slots = usize::from(evidence_nonempty);
    let canonical_assembly = AttributionBatchPlan::try_new(
        Phase::Build,
        Dimension::Bytes,
        2 * canonical_slots,
        canonical_slots,
    )
    .expect("canonical assembly plan");
    let evidence_assembly = AttributionBatchPlan::try_new(
        Phase::Finalize,
        Dimension::Scratch,
        2 * evidence_slots,
        2 * evidence_slots,
    )
    .expect("evidence assembly plan");
    let fallback = AttributionBatchPlan::try_new(Phase::Publish, Dimension::Evidence, 1, 1)
        .expect("one physical fallback batch");
    let covers = [
        canonical_assembly.planned_bytes,
        evidence_assembly.planned_bytes,
        fallback.planned_bytes,
    ];
    (
        FiniteStartupLedgerPlanParts {
            canonical_assembly,
            evidence_assembly,
            fallback,
            fallback_attribution: StartupRetainedAttribution::new(
                Phase::Finalize,
                Dimension::Progress,
                5,
            ),
            account_shares: StartupAccountShareEnvelope::three_identical([
                (Dimension::Records, 1),
                (Dimension::Records, 2),
            ]),
            fallback_commitment: OutputCommitment::new(Dimension::Records, 4),
        },
        covers,
    )
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    dimensions: [(Dimension, StoragePolicy, StorageDimensionState); 6],
    phases: [(u64, u64, u64); 3],
    header: ([u64; 8], PublicationState, Option<CompletedPhase<Phase>>),
    work: [(Dimension, WorkPolicy, WorkDimensionState); 6],
    work_header: (WorkMode, u64, bool, bool, bool, bool, Option<usize>, bool),
}

fn observe(ledger: &FiniteLedger<Owner, Phase, Dimension>) -> Observation {
    let storage = ledger.storage.read();
    let state = storage.view();
    let work = &ledger.work_header;
    Observation {
        dimensions: std::array::from_fn(|position| {
            let entry = &state.dimensions[position];
            (entry.dimension, entry.policy, entry.state)
        }),
        phases: std::array::from_fn(|position| {
            let phase = &state.phase_states[position];
            (
                phase.current_live_bytes,
                phase.peak_live_bytes,
                phase.retained_output_bytes,
            )
        }),
        header: (
            [
                state.current_live_bytes,
                state.peak_live_bytes,
                state.active_reservations,
                state.retained_output_bytes,
                state.retained_output_reservations,
                state.evidence_byte_limit,
                state.current_administrative_bytes,
                state.peak_administrative_bytes,
            ],
            state.publication,
            state.last_completed_phase,
        ),
        work: ledger
            .work
            .each_ref()
            .map(|row| (row.dimension, row.policy, row.state)),
        work_header: (
            work.mode,
            work.open_envelopes,
            work.producer_open,
            work.parser_parent_open,
            work.parser_child_open,
            work.protected_open,
            work.protected_dimension,
            work.integrity_fault,
        ),
    }
}

fn backing_bytes<Destination: PublicationDestination>(
    batch: &StartupAttributionBatchBackings<Owner, Phase, Dimension, Destination>,
) -> u64 {
    let attributions = batch.attributions.capacity()
        * mem::size_of::<OperationAttributionReservation<Owner, Phase, Dimension, Destination>>();
    let commitments =
        batch.commitments.capacity() * mem::size_of::<OutputCommitment<Dimension, Destination>>();
    u64::try_from(attributions + commitments).expect("bounded backing bytes")
}

fn assert_preflight_rejection(
    ledger: &mut FiniteLedger<Owner, Phase, Dimension>,
    parts: FiniteStartupLedgerPlanParts<Owner, Phase, Dimension>,
    expected: LedgerCause<Dimension>,
) {
    let before = observe(ledger);
    let identity = ledger.identity;
    let work_rows = ledger.work.as_ptr();
    let shared = ledger.storage.shared().map(Arc::as_ptr);
    let probes = (
        test_startup_graph_allocation_entries(),
        test_attribution_batch_allocation_entries(),
        test_startup_graph_storage_releases(),
        test_startup_graph_ledger_release_locks(),
        test_startup_graph_storage_releases_at_ledger_lock(),
    );
    test_fail_next_evidence_allocation();
    let result = ledger
        .view()
        .try_bind_finite_startup_attribution_graph(parts);
    let promotion_not_entered = test_evidence_allocation_failure_is_armed();
    test_set_evidence_allocation_failure(false);
    assert!(matches!(
        result,
        Err(AttributionBatchCreateError::Ledger(actual)) if actual == expected
    ));
    assert!(promotion_not_entered);
    assert_eq!(observe(ledger), before);
    assert_eq!(ledger.identity, identity);
    assert_eq!(ledger.work.as_ptr(), work_rows);
    assert_eq!(ledger.storage.shared().map(Arc::as_ptr), shared);
    assert_eq!(
        (
            test_startup_graph_allocation_entries(),
            test_attribution_batch_allocation_entries(),
            test_startup_graph_storage_releases(),
            test_startup_graph_ledger_release_locks(),
            test_startup_graph_storage_releases_at_ledger_lock(),
        ),
        probes
    );
}

#[test]
fn all_assembly_shapes_own_exact_real_credits_and_only_one_fallback_commitment() {
    let mut foreign = finite_ledger(u64::MAX);
    let _foreign_credit = foreign
        .view()
        .reserve(Phase::Build, Dimension::Canonical, 1)
        .expect("a different actual shared owner");
    for (canonical_nonempty, evidence_nonempty, active, allocations) in [
        (true, true, 4, 6),
        (true, false, 3, 4),
        (false, true, 3, 4),
        (false, false, 2, 2),
    ] {
        test_reset_startup_graph_probes(None);
        let mut ledger = finite_ledger(u64::MAX);
        ledger
            .view()
            .charge_attempted(Dimension::Records, 2)
            .expect("existing attempts");
        ledger
            .view()
            .charge_consumed(Dimension::Records, 1)
            .expect("existing consumption");
        let records = ledger.catalog.binary_search(&Dimension::Records).unwrap();
        ledger.storage.write().view_mut().dimensions[records]
            .state
            .published_outcome_evidence = 1;
        ledger
            .view()
            .tighten_dimension_policy(
                Dimension::Records,
                DimensionPolicy::counter(11, 10, u64::MAX).with_published_outcome_evidence_limit(5),
            )
            .expect("exact headroom for three shares and one commitment");
        let before = observe(&ledger);
        let identity = ledger.identity;
        let (parts, covers) = graph_request(canonical_nonempty, evidence_nonempty);
        let total_live = covers.into_iter().sum::<u64>() + 5;
        let (graph, shares) = ledger
            .view()
            .try_bind_finite_startup_attribution_graph(parts)
            .expect("only nine work units and four evidence units must fit");
        let shared = Arc::clone(ledger.storage.shared().expect("actual promoted storage"));
        let administrative = ledger.known_administrative_bytes(0).unwrap();
        assert!(administrative > 0);
        assert_eq!(test_startup_graph_allocation_entries(), allocations);
        assert_eq!(observe(&ledger).work, before.work);
        assert_eq!(observe(&ledger).work_header, before.work_header);
        assert!(graph.belongs_to_view(&ledger.as_read()));
        assert!(!graph.belongs_to_view(&foreign.as_read()));

        let ownership = graph.ownership.as_ref().expect("original graph ownership");
        assert_eq!(ownership.identity, identity);
        assert_eq!(ownership.cores.iter().flatten().count() as u64, active);
        for ((actual, planned), core) in [
            backing_bytes(&ownership.canonical_assembly),
            backing_bytes(&ownership.evidence_assembly),
            backing_bytes(&ownership.fallback),
        ]
        .into_iter()
        .zip(covers)
        .zip(&ownership.cores[..3])
        {
            assert_eq!(actual, planned);
            assert_eq!(core.is_some(), planned != 0);
            if let Some(core) = core {
                assert_eq!(core.bytes, actual);
            }
        }
        for (owner, core) in ownership.runtime_owners.iter().zip(&ownership.cores[..3]) {
            assert_eq!(owner.is_some(), core.is_some());
            if let Some(owner) = owner {
                assert!(Arc::ptr_eq(owner, &shared));
            }
        }
        for core in ownership.cores.iter().flatten() {
            assert!(core.active);
            assert!(core.is_owned_by(&shared));
            assert!(!core.retained_output);
        }
        let fallback_attribution = ownership.cores[3]
            .as_ref()
            .expect("one real fallback attribution in every shape");
        assert_eq!(fallback_attribution.bytes, 5);
        assert_eq!(fallback_attribution.phase, Phase::Finalize);
        assert_eq!(fallback_attribution.dimension, Dimension::Progress);
        assert_eq!(
            ownership
                .cores
                .iter()
                .flatten()
                .map(|core| core.bytes)
                .sum::<u64>(),
            total_live
        );
        assert!(ownership.canonical_assembly.attributions.is_empty());
        assert!(ownership.canonical_assembly.commitments.is_empty());
        assert!(ownership.evidence_assembly.attributions.is_empty());
        assert!(ownership.evidence_assembly.commitments.is_empty());
        assert!(ownership.fallback.attributions.is_empty());
        assert_eq!(ownership.fallback.commitments.len(), 1);
        let commitment = ownership.fallback.commitments[0];
        assert_eq!(commitment.dimension(), Dimension::Records);
        assert_eq!(commitment.amount(), 4);
        assert_eq!(commitment.dimension_position, records);
        let steps = shares.into_steps();
        assert_eq!(steps.len(), 3);
        for step in steps {
            assert_eq!(
                step.into_charges(),
                [(Dimension::Records, 1), (Dimension::Records, 2)]
            );
        }
        {
            let state = lock(&shared);
            assert_eq!(state.active_reservations, active);
            assert_eq!(state.current_live_bytes, total_live);
            assert_eq!(state.peak_live_bytes, total_live);
            assert_eq!(state.current_administrative_bytes, administrative);
            for (entry, expected) in state
                .dimensions
                .iter()
                .zip([covers[0], 0, covers[1], 0, covers[2], 5])
            {
                assert_eq!(entry.state.current_live_bytes, expected);
                assert_eq!(entry.state.peak_live_bytes, expected);
                assert_eq!(
                    entry.state.published_outcome_evidence,
                    u64::from(entry.dimension == Dimension::Records)
                );
                assert_eq!(entry.state.committed, 0);
                assert_eq!(entry.state.committed_canonical_output, 0);
            }
            for (phase, expected) in
                state
                    .phase_states
                    .iter()
                    .zip([covers[2], covers[0], covers[1] + 5])
            {
                assert_eq!(phase.current_live_bytes, expected);
                assert_eq!(phase.peak_live_bytes, expected);
            }
        }

        drop(graph);

        assert_eq!(test_startup_graph_storage_releases(), 6);
        assert_eq!(test_startup_graph_ledger_release_locks(), 1);
        assert_eq!(
            test_startup_graph_storage_releases_at_ledger_lock(),
            Some(6)
        );
        assert_eq!(observe(&ledger).work, before.work);
        assert_eq!(observe(&ledger).work_header, before.work_header);
        assert_eq!(ledger.known_administrative_bytes(0), Some(administrative));
        let state = lock(&shared);
        assert_eq!(state.current_live_bytes, 0);
        assert_eq!(state.active_reservations, 0);
        assert_eq!(state.peak_live_bytes, total_live);
        assert!(
            state
                .dimensions
                .iter()
                .all(|entry| entry.state.current_live_bytes == 0)
        );
        assert!(
            state
                .phase_states
                .iter()
                .all(|phase| phase.current_live_bytes == 0)
        );
        assert_eq!(
            state.dimensions[records].state.published_outcome_evidence,
            1
        );
        assert_eq!(Arc::strong_count(&shared), 2);
    }
}

#[test]
fn tightened_preflight_limits_reject_without_promotion_or_any_state_change() {
    let _guard = test_evidence_allocation_guard();
    let (_, covers) = graph_request(true, true);
    let total_live = covers.into_iter().sum::<u64>() + 5;
    for (operation_limit, dimension, policy, expected) in [
        (
            u64::MAX,
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, covers[0] - 1),
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
                value: covers[0],
                limit: covers[0] - 1,
            },
        ),
        (
            total_live - 1,
            Dimension::Records,
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::Progress,
                value: total_live,
                limit: total_live - 1,
            },
        ),
        (
            u64::MAX,
            Dimension::Records,
            DimensionPolicy::counter(10, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Records,
                value: 11,
                limit: 10,
            },
        ),
        (
            u64::MAX,
            Dimension::Records,
            DimensionPolicy::counter(11, 9, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
            LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: Dimension::Records,
                value: 10,
                limit: 9,
            },
        ),
        (
            u64::MAX,
            Dimension::Records,
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(4),
            LedgerCause::LimitExceeded {
                class: CounterClass::PublishedOutcomeEvidence,
                dimension: Dimension::Records,
                value: 5,
                limit: 4,
            },
        ),
    ] {
        test_reset_startup_graph_probes(None);
        let mut ledger = finite_ledger(operation_limit);
        ledger
            .view()
            .charge_attempted(Dimension::Records, 2)
            .unwrap();
        ledger
            .view()
            .charge_consumed(Dimension::Records, 1)
            .unwrap();
        let records = ledger.catalog.binary_search(&Dimension::Records).unwrap();
        ledger.storage.write().view_mut().dimensions[records]
            .state
            .published_outcome_evidence = 1;
        ledger
            .view()
            .tighten_dimension_policy(dimension, policy)
            .unwrap();
        let (parts, _) = graph_request(true, true);
        assert_preflight_rejection(&mut ledger, parts, expected);
        assert!(ledger.storage.shared().is_none());
        assert_eq!(test_startup_graph_allocation_entries(), 0);
    }
}

#[test]
fn live_and_work_preflight_failures_precede_deferred_binding_errors() {
    let _guard = test_evidence_allocation_guard();
    test_reset_startup_graph_probes(None);
    let mut ledger = finite_ledger(u64::MAX);
    let (mut parts, covers) = graph_request(true, true);
    ledger
        .view()
        .tighten_dimension_policy(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, covers[0] - 1),
        )
        .unwrap();
    parts.account_shares = StartupAccountShareEnvelope::three_identical([
        (Dimension::Unknown, 1),
        (Dimension::Records, 2),
    ]);
    parts.fallback_commitment = OutputCommitment::new(Dimension::Unknown, 4);
    assert_preflight_rejection(
        &mut ledger,
        parts,
        LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value: covers[0],
            limit: covers[0] - 1,
        },
    );

    let mut ledger = finite_ledger(u64::MAX);
    ledger
        .view()
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(8, u64::MAX, u64::MAX),
        )
        .unwrap();
    let (mut parts, _) = graph_request(true, true);
    parts.fallback_commitment = OutputCommitment::new(Dimension::Unknown, 4);
    assert_preflight_rejection(
        &mut ledger,
        parts,
        LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 9,
            limit: 8,
        },
    );
    assert_eq!(test_startup_graph_allocation_entries(), 0);
}

#[test]
fn current_pending_work_and_later_shared_tightening_use_three_share_headroom() {
    let _guard = test_evidence_allocation_guard();
    test_reset_startup_graph_probes(None);
    let mut ledger = finite_ledger(u64::MAX);
    let (layout, mut pending) = {
        let (mut work, _) = ledger.view().into_internal();
        let layout = work.bind_ordinary([Dimension::Records]).unwrap();
        let pending = layout
            .admit_observed(&mut work.reborrow().unwrap().work, [4], WorkLane::Internal)
            .expect("real pending consumption");
        (layout, pending)
    };
    ledger
        .view()
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(13, 12, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .unwrap();
    let (parts, _) = graph_request(true, true);
    assert_preflight_rejection(
        &mut ledger,
        parts,
        LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 13,
            limit: 12,
        },
    );
    assert!(ledger.storage.shared().is_none());
    ledger
        .view()
        .into_internal()
        .0
        .resolve(layout, &mut pending, false)
        .unwrap();
    let before = observe(&ledger);
    let (parts, _) = graph_request(true, true);
    let (graph, _) = ledger
        .view()
        .try_bind_finite_startup_attribution_graph(parts)
        .expect("four existing attempts plus nine proposed units fit");
    assert_eq!(observe(&ledger).work, before.work);
    assert_eq!(observe(&ledger).work_header, before.work_header);
    drop(graph);

    ledger
        .view()
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(12, 12, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .unwrap();
    let (parts, _) = graph_request(true, true);
    assert_preflight_rejection(
        &mut ledger,
        parts,
        LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 13,
            limit: 12,
        },
    );
    assert_eq!(test_startup_graph_allocation_entries(), 6);
    assert_eq!(test_startup_graph_ledger_release_locks(), 1);
}

#[test]
fn selected_reservation_count_sets_the_active_overflow_boundary() {
    let _guard = test_evidence_allocation_guard();
    for (canonical_nonempty, evidence_nonempty, reservations, dimension) in [
        (false, false, 2, Dimension::Evidence),
        (true, false, 3, Dimension::Bytes),
        (false, true, 3, Dimension::Scratch),
    ] {
        for already_shared in [false, true] {
            for overflow in [false, true] {
                test_reset_startup_graph_probes(None);
                let mut ledger = finite_ledger(u64::MAX);
                let existing = already_shared.then(|| {
                    ledger
                        .view()
                        .reserve(Phase::Build, Dimension::Canonical, 7)
                        .expect("preexisting real credit")
                });
                let legitimate_active = u64::from(already_shared);
                ledger.storage.write().view_mut().active_reservations =
                    u64::MAX - reservations + u64::from(overflow);
                let (parts, _) = graph_request(canonical_nonempty, evidence_nonempty);
                if overflow {
                    assert_preflight_rejection(
                        &mut ledger,
                        parts,
                        LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Live,
                            dimension,
                        },
                    );
                    ledger.storage.write().view_mut().active_reservations = legitimate_active;
                } else {
                    let (graph, _) = ledger
                        .view()
                        .try_bind_finite_startup_attribution_graph(parts)
                        .expect("exact active reservation headroom");
                    assert_eq!(ledger.storage.read().view().active_reservations, u64::MAX);
                    ledger.storage.write().view_mut().active_reservations =
                        legitimate_active + reservations;
                    drop(graph);
                }
                assert_eq!(
                    ledger.storage.read().view().active_reservations,
                    legitimate_active
                );
                drop(existing);
                assert_eq!(ledger.storage.read().view().active_reservations, 0);
            }
        }
    }
}

#[test]
fn selected_reservations_in_one_dimension_share_the_live_limit() {
    let _guard = test_evidence_allocation_guard();
    for one_over in [false, true] {
        test_reset_startup_graph_probes(None);
        let mut ledger = finite_ledger(u64::MAX);
        let (mut parts, covers) = graph_request(true, true);
        let limit = covers[0] + 5;
        parts.fallback_attribution.dimension = Dimension::Bytes;
        parts.fallback_attribution.bytes += u64::from(one_over);
        ledger
            .view()
            .tighten_dimension_policy(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, limit),
            )
            .unwrap();
        if one_over {
            assert_preflight_rejection(
                &mut ledger,
                parts,
                LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: Dimension::Bytes,
                    value: limit + 1,
                    limit,
                },
            );
        } else {
            let (graph, _) = ledger
                .view()
                .try_bind_finite_startup_attribution_graph(parts)
                .expect("grouped reservations exactly fill the dimension");
            let bytes = ledger.catalog.binary_search(&Dimension::Bytes).unwrap();
            let actual = ledger.storage.read().view().dimensions[bytes]
                .state
                .current_live_bytes;
            assert_eq!(actual, limit);
            drop(graph);
            assert_eq!(ledger.storage.read().view().current_live_bytes, 0);
        }
    }
}

#[test]
fn every_shape_backing_failure_drops_six_slots_before_one_exact_refund() {
    for ((canonical_nonempty, evidence_nonempty, allocations), already_shared) in [
        (true, true, 6),
        (true, false, 4),
        (false, true, 4),
        (false, false, 2),
    ]
    .into_iter()
    .flat_map(|shape| [false, true].map(|already_shared| (shape, already_shared)))
    {
        for failure_index in 0..allocations {
            test_reset_startup_graph_probes(Some(failure_index));
            let mut ledger = finite_ledger(u64::MAX);
            let existing = already_shared.then(|| {
                ledger
                    .view()
                    .reserve(Phase::Build, Dimension::Canonical, 7)
                    .expect("preexisting real credit")
            });
            let shared_before = ledger.storage.shared().map(Arc::clone);
            let administrative_before = ledger.known_administrative_bytes(0);
            let (layout, mut pending) = {
                let (mut work, _) = ledger.view().into_internal();
                let layout = work.bind_ordinary([Dimension::Records]).unwrap();
                let pending = layout
                    .admit_observed(&mut work.reborrow().unwrap().work, [4], WorkLane::Internal)
                    .expect("existing pending work");
                (layout, pending)
            };
            let before = observe(&ledger);
            let identity = ledger.identity;
            let work_rows = ledger.work.as_ptr();
            let (parts, covers) = graph_request(canonical_nonempty, evidence_nonempty);
            assert!(matches!(
                ledger
                    .view()
                    .try_bind_finite_startup_attribution_graph(parts),
                Err(AttributionBatchCreateError::AllocationFailed)
            ));
            assert_eq!(test_startup_graph_allocation_entries(), failure_index + 1);
            assert_eq!(test_startup_graph_storage_releases(), 6);
            assert_eq!(test_startup_graph_ledger_release_locks(), 1);
            assert_eq!(
                test_startup_graph_storage_releases_at_ledger_lock(),
                Some(6)
            );
            assert_eq!(ledger.identity, identity);
            assert_eq!(ledger.work.as_ptr(), work_rows);
            assert_eq!(observe(&ledger).work, before.work);
            assert_eq!(observe(&ledger).work_header, before.work_header);
            let shared = Arc::clone(ledger.storage.shared().expect("actual shared state"));
            let administrative = ledger.known_administrative_bytes(0).unwrap();
            assert!(administrative > 0);
            if let Some(shared_before) = &shared_before {
                assert!(Arc::ptr_eq(shared_before, &shared));
                assert_eq!(Some(administrative), administrative_before);
            }
            drop(shared_before);
            let baseline_bytes = if already_shared { 7 } else { 0 };
            {
                let state = lock(&shared);
                assert_eq!(state.current_live_bytes, baseline_bytes);
                assert_eq!(state.active_reservations, u64::from(already_shared));
                assert_eq!(
                    state.peak_live_bytes,
                    covers.into_iter().sum::<u64>() + 5 + baseline_bytes
                );
                assert_eq!(state.current_administrative_bytes, administrative);
                assert_eq!(state.retained_output_bytes, 0);
                assert_eq!(state.retained_output_reservations, 0);
                assert_eq!(state.publication, PublicationState::Open);
                for entry in &state.dimensions {
                    assert_eq!(
                        entry.state.current_live_bytes,
                        if entry.dimension == Dimension::Canonical {
                            baseline_bytes
                        } else {
                            0
                        }
                    );
                    assert_eq!(entry.state.published_outcome_evidence, 0);
                }
                for (position, phase) in state.phase_states.iter().enumerate() {
                    assert_eq!(
                        phase.current_live_bytes,
                        if position == 1 { baseline_bytes } else { 0 }
                    );
                }
                assert_eq!(
                    Arc::strong_count(&shared),
                    if already_shared { 3 } else { 2 }
                );
            }
            ledger
                .view()
                .into_internal()
                .0
                .resolve(layout, &mut pending, false)
                .unwrap();
            drop(existing);
            assert_eq!(lock(&shared).current_live_bytes, 0);
            assert_eq!(lock(&shared).active_reservations, 0);
            assert_eq!(Arc::strong_count(&shared), 2);
        }
    }
}
