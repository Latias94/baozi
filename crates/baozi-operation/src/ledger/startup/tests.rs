use super::super::finite::{FiniteLedger, FiniteLedgerLimits};
use super::super::{
    AttributionBatchPlan, CounterClass, LedgerCause, LedgerLimits, OutcomeEvidenceDestination,
    OutputCommitment, PublicationState, WorkDimensionState, WorkLane, WorkMode, WorkPolicy,
    test_attribution_batch_allocation_entries, test_evidence_allocation_failure_is_armed,
    test_evidence_allocation_guard, test_fail_next_evidence_allocation,
    test_set_evidence_allocation_failure,
};
use super::*;
use crate::{DimensionPolicy, OperationOwner};

#[derive(Debug)]
struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Build,
    Finalize,
    Publish,
    Unknown,
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

#[test]
fn startup_dynamic_adapters_share_the_borrowed_builder_and_owner_check() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    ledger
        .preflight_startup_attribution_graph(&parts)
        .expect("dynamic immutable preflight");
    ledger
        .view()
        .preflight_startup_attribution_graph(&parts)
        .expect("borrowed dynamic preflight");
    assert_eq!(test_startup_graph_allocation_entries(), 0);
    assert!(ledger.state.shared().is_none());

    let (graph, _) = ledger
        .view()
        .try_admit_startup_attribution_graph(parts)
        .expect("dynamic graph through the borrowed builder")
        .into_parts();
    assert!(graph.belongs_to(&ledger));
    assert!(graph.belongs_to_view(&ledger.view().as_read()));

    let (limits, parts, _) = default_graph_request();
    let mut foreign = frozen_graph_ledger(limits);
    assert!(!graph.belongs_to(&foreign));
    assert!(!graph.belongs_to_view(&foreign.view().as_read()));
    let foreign_graph = foreign
        .try_bind_startup_attribution_graph(parts)
        .expect("foreign graph with the same catalog");
    assert!(!graph.belongs_to(&foreign));
    assert!(!graph.belongs_to_view(&foreign.view().as_read()));

    drop(graph);
    assert_eq!(ledger.test_probe().current_live_bytes(), 0);
    assert_eq!(ledger.test_probe().active_reservations(), 0);
    drop(foreign_graph);
    assert_eq!(foreign.test_probe().current_live_bytes(), 0);
    assert_eq!(foreign.test_probe().active_reservations(), 0);
}

fn startup_limits(
    operation_live: u64,
    dimension_live: u64,
    attempted: u64,
    consumed: u64,
    published_evidence: u64,
) -> LedgerLimits<Dimension> {
    LedgerLimits::new(
        operation_live,
        [
            (
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(attempted, consumed, u64::MAX, dimension_live)
                    .with_published_outcome_evidence_limit(published_evidence),
            ),
            (
                Dimension::Records,
                DimensionPolicy::counter(attempted, consumed, u64::MAX)
                    .with_published_outcome_evidence_limit(published_evidence),
            ),
        ],
    )
    .expect("startup test policy")
}

fn startup_batch_plans(
    capacity_dimension: Dimension,
) -> StartupAttributionBatchPlans<Owner, Phase, Dimension> {
    StartupAttributionBatchPlans::new(
        AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 2, 1)
            .expect("canonical assembly plan"),
        AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 2, 2)
            .expect("evidence assembly plan"),
        AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 1, 1)
            .expect("fallback plan"),
        AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 1, 1)
            .expect("evidence plan"),
    )
}

fn startup_batch_bytes(batches: &StartupAttributionBatchPlans<Owner, Phase, Dimension>) -> u64 {
    [
        batches.canonical_assembly.planned_bytes,
        batches.evidence_assembly.planned_bytes,
        batches.fallback.planned_bytes,
        batches.evidence.planned_bytes,
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .expect("test batch envelope")
}

fn startup_attributions(
    first_dimension: Dimension,
    second_dimension: Dimension,
) -> [StartupRetainedAttribution<Phase, Dimension>; 2] {
    [
        StartupRetainedAttribution::new(Phase::Build, first_dimension, 3),
        StartupRetainedAttribution::new(Phase::Build, second_dimension, 5),
    ]
}

fn startup_share_charges() -> [(Dimension, u64); 2] {
    [(Dimension::Records, 1), (Dimension::Records, 2)]
}

fn startup_account_shares() -> StartupAccountShareEnvelope<Dimension> {
    StartupAccountShareEnvelope::four_identical(startup_share_charges())
}

fn startup_commitments(
    first_dimension: Dimension,
    second_dimension: Dimension,
) -> [OutputCommitment<Dimension, OutcomeEvidenceDestination>; 2] {
    [
        OutputCommitment::new(first_dimension, 4),
        OutputCommitment::new(second_dimension, 6),
    ]
}

fn graph_limits(
    operation_live: u64,
    bytes_live: u64,
    scratch_live: u64,
    attempted: u64,
    consumed: u64,
) -> LedgerLimits<Dimension> {
    graph_limits_with_published(
        operation_live,
        bytes_live,
        scratch_live,
        attempted,
        consumed,
        u64::MAX,
    )
}

fn graph_limits_with_published(
    operation_live: u64,
    bytes_live: u64,
    scratch_live: u64,
    attempted: u64,
    consumed: u64,
    published: u64,
) -> LedgerLimits<Dimension> {
    LedgerLimits::new(
        operation_live,
        [
            (
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(attempted, consumed, u64::MAX, bytes_live)
                    .with_published_outcome_evidence_limit(published),
            ),
            (
                Dimension::Records,
                DimensionPolicy::counter(attempted, consumed, u64::MAX)
                    .with_published_outcome_evidence_limit(published),
            ),
            (
                Dimension::Scratch,
                DimensionPolicy::reservable_bytes(attempted, consumed, u64::MAX, scratch_live)
                    .with_published_outcome_evidence_limit(published),
            ),
        ],
    )
    .expect("startup graph policy")
}

fn graph_batch_plans(
    phases: [Phase; 4],
    dimensions: [Dimension; 4],
) -> StartupAttributionBatchPlans<Owner, Phase, Dimension> {
    StartupAttributionBatchPlans::new(
        AttributionBatchPlan::try_new(phases[0], dimensions[0], 2, 1)
            .expect("canonical graph plan"),
        AttributionBatchPlan::try_new(phases[1], dimensions[1], 2, 2)
            .expect("evidence assembly graph plan"),
        AttributionBatchPlan::try_new(phases[2], dimensions[2], 1, 1).expect("fallback graph plan"),
        AttributionBatchPlan::try_new(phases[3], dimensions[3], 1, 1).expect("evidence graph plan"),
    )
}

fn graph_batch_plans_with_terminal_limits(
    fallback_attributions: usize,
    fallback_commitments: usize,
    evidence_attributions: usize,
    evidence_commitments: usize,
) -> StartupAttributionBatchPlans<Owner, Phase, Dimension> {
    StartupAttributionBatchPlans::new(
        AttributionBatchPlan::try_new(Phase::Build, Dimension::Bytes, 2, 1)
            .expect("canonical graph plan"),
        AttributionBatchPlan::try_new(Phase::Build, Dimension::Bytes, 2, 2)
            .expect("evidence assembly graph plan"),
        AttributionBatchPlan::try_new(
            Phase::Build,
            Dimension::Bytes,
            fallback_attributions,
            fallback_commitments,
        )
        .expect("fallback graph plan"),
        AttributionBatchPlan::try_new(
            Phase::Build,
            Dimension::Bytes,
            evidence_attributions,
            evidence_commitments,
        )
        .expect("evidence graph plan"),
    )
}

fn graph_request_from_batches(
    batches: StartupAttributionBatchPlans<Owner, Phase, Dimension>,
    retained: [StartupRetainedAttribution<Phase, Dimension>; 2],
) -> (
    LedgerLimits<Dimension>,
    StartupLedgerPlanParts<Owner, Phase, Dimension>,
    [u64; 4],
) {
    let covers = [
        batches.canonical_assembly.planned_bytes,
        batches.evidence_assembly.planned_bytes,
        batches.fallback.planned_bytes,
        batches.evidence.planned_bytes,
    ];
    let limits = graph_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX);
    let parts = StartupLedgerPlanParts {
        batches,
        retained_attributions: retained,
        account_shares: startup_account_shares(),
        commitments: startup_commitments(Dimension::Records, Dimension::Records),
    };
    (limits, parts, covers)
}

fn graph_request(
    phases: [Phase; 4],
    dimensions: [Dimension; 4],
    retained: [(Phase, Dimension, u64); 2],
) -> (
    LedgerLimits<Dimension>,
    StartupLedgerPlanParts<Owner, Phase, Dimension>,
    [u64; 4],
) {
    graph_request_from_batches(
        graph_batch_plans(phases, dimensions),
        retained.map(|(phase, dimension, bytes)| {
            StartupRetainedAttribution::new(phase, dimension, bytes)
        }),
    )
}

fn default_graph_request() -> (
    LedgerLimits<Dimension>,
    StartupLedgerPlanParts<Owner, Phase, Dimension>,
    [u64; 4],
) {
    graph_request(
        [Phase::Build; 4],
        [Dimension::Bytes; 4],
        [
            (Phase::Build, Dimension::Bytes, 3),
            (Phase::Build, Dimension::Bytes, 5),
        ],
    )
}

fn frozen_graph_ledger(limits: LedgerLimits<Dimension>) -> Ledger<Owner, Phase, Dimension> {
    Ledger::try_new(limits, &[Phase::Build, Phase::Finalize]).expect("frozen startup graph ledger")
}

fn finite_startup_ledger(
    operation_live_byte_limit: u64,
    evidence_byte_limit: u64,
) -> FiniteLedger<Owner, Phase, Dimension> {
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
        FiniteLedgerLimits::try_new(operation_live_byte_limit, evidence_byte_limit, policies)
            .expect("six finite startup policies"),
        [Phase::Publish, Phase::Build, Phase::Finalize],
    )
    .expect("three finite startup phases")
}

fn finite_graph_request() -> (StartupLedgerPlanParts<Owner, Phase, Dimension>, [u64; 4]) {
    let batches = graph_batch_plans(
        [Phase::Build, Phase::Finalize, Phase::Publish, Phase::Build],
        [
            Dimension::Bytes,
            Dimension::Scratch,
            Dimension::Canonical,
            Dimension::Evidence,
        ],
    );
    let covers = [
        batches.canonical_assembly.planned_bytes,
        batches.evidence_assembly.planned_bytes,
        batches.fallback.planned_bytes,
        batches.evidence.planned_bytes,
    ];
    (
        StartupLedgerPlanParts {
            batches,
            retained_attributions: [
                StartupRetainedAttribution::new(Phase::Finalize, Dimension::Bytes, 3),
                StartupRetainedAttribution::new(Phase::Publish, Dimension::Progress, 5),
            ],
            account_shares: startup_account_shares(),
            commitments: startup_commitments(Dimension::Records, Dimension::Records),
        },
        covers,
    )
}

#[derive(Debug, PartialEq, Eq)]
struct FiniteStartupWorkObservation {
    rows: [(Dimension, WorkPolicy, WorkDimensionState); 6],
    mode: WorkMode,
    open_envelopes: u64,
    producer_open: bool,
    parser_parent_open: bool,
    parser_child_open: bool,
    protected_open: bool,
    protected_dimension: Option<usize>,
    integrity_fault: bool,
}

fn finite_work_observation(
    ledger: &FiniteLedger<Owner, Phase, Dimension>,
) -> FiniteStartupWorkObservation {
    FiniteStartupWorkObservation {
        rows: ledger
            .work
            .each_ref()
            .map(|row| (row.dimension, row.policy, row.state)),
        mode: ledger.work_header.mode,
        open_envelopes: ledger.work_header.open_envelopes,
        producer_open: ledger.work_header.producer_open,
        parser_parent_open: ledger.work_header.parser_parent_open,
        parser_child_open: ledger.work_header.parser_child_open,
        protected_open: ledger.work_header.protected_open,
        protected_dimension: ledger.work_header.protected_dimension,
        integrity_fault: ledger.work_header.integrity_fault,
    }
}

fn preflight_startup(
    ledger: &Ledger<Owner, Phase, Dimension>,
    parts: &StartupLedgerPlanParts<Owner, Phase, Dimension>,
) -> Result<(), LedgerCause<Dimension>> {
    let dimensions = [Dimension::Bytes, Dimension::Records, Dimension::Scratch];
    let phases = [Phase::Build, Phase::Finalize];
    let dimensions_before = dimensions.map(|dimension| ledger.dimension_evidence(dimension));
    let phases_before = phases.map(|phase| ledger.phase_evidence(phase));
    let storage_before = {
        let state = ledger.state.read();
        (
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            state.current_administrative_bytes,
            state.peak_administrative_bytes,
            state.publication,
        )
    };
    let work_before = {
        let state = &ledger.work;
        state
            .rows
            .iter()
            .map(|row| (row.policy, row.state))
            .collect::<Vec<_>>()
    };
    let owners_before = ledger.state.shared().map_or(1, Arc::strong_count);
    let allocations_before = test_startup_graph_allocation_entries();
    let batch_allocations_before = test_attribution_batch_allocation_entries();
    let releases_before = test_startup_graph_storage_releases();
    let release_locks_before = test_startup_graph_ledger_release_locks();

    let result = ledger.preflight_startup_attribution_graph(parts);

    assert_eq!(
        dimensions.map(|dimension| ledger.dimension_evidence(dimension)),
        dimensions_before
    );
    assert_eq!(
        phases.map(|phase| ledger.phase_evidence(phase)),
        phases_before
    );
    {
        let state = ledger.state.read();
        assert_eq!(
            (
                state.current_live_bytes,
                state.peak_live_bytes,
                state.active_reservations,
                state.current_administrative_bytes,
                state.peak_administrative_bytes,
                state.publication,
            ),
            storage_before
        );
    }
    {
        let state = &ledger.work;
        assert_eq!(
            state
                .rows
                .iter()
                .map(|row| (row.policy, row.state))
                .collect::<Vec<_>>(),
            work_before
        );
    }
    assert_eq!(
        ledger.state.shared().map_or(1, Arc::strong_count),
        owners_before
    );
    assert_eq!(test_startup_graph_allocation_entries(), allocations_before);
    assert_eq!(
        test_attribution_batch_allocation_entries(),
        batch_allocations_before
    );
    assert_eq!(test_startup_graph_storage_releases(), releases_before);
    assert_eq!(
        test_startup_graph_ledger_release_locks(),
        release_locks_before
    );

    match result {
        Ok(()) => Ok(()),
        Err(AttributionBatchCreateError::Ledger(cause)) => Err(cause),
        Err(error) => panic!("valid graph capacity must reach ledger preflight: {error:?}"),
    }
}

fn vec_capacity_bytes<T>(values: &Vec<T>) -> u64 {
    let bytes = values
        .capacity()
        .checked_mul(std::mem::size_of::<T>())
        .expect("test backing bytes");
    u64::try_from(bytes).expect("test backing bytes fit u64")
}

fn batch_backing_bytes<O, P, D, Destination>(
    batch: &StartupAttributionBatchBackings<O, P, D, Destination>,
) -> u64
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    vec_capacity_bytes(&batch.attributions)
        .checked_add(vec_capacity_bytes(&batch.commitments))
        .expect("test aggregate backing bytes")
}

fn ledger_graph_observation(
    ledger: &Ledger<Owner, Phase, Dimension>,
) -> (u64, u64, u64, u64, u64, u64, u64, u64) {
    let (current_live_bytes, active_reservations) = {
        let state = ledger.state.read();
        (state.current_live_bytes, state.active_reservations)
    };
    let bytes = ledger
        .dimension_evidence(Dimension::Bytes)
        .expect("byte evidence");
    let records = ledger
        .dimension_evidence(Dimension::Records)
        .expect("record evidence");
    let scratch = ledger
        .dimension_evidence(Dimension::Scratch)
        .expect("scratch evidence");
    (
        current_live_bytes,
        active_reservations,
        bytes.current_live_bytes(),
        records.current_live_bytes(),
        scratch.current_live_bytes(),
        records.attempted(),
        records.consumed(),
        bytes.attempted(),
    )
}

#[test]
fn startup_preflight_preserves_the_exact_typed_envelope() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let batch_bytes = startup_batch_bytes(&batches);
    let retained_attributions = startup_attributions(Dimension::Bytes, Dimension::Bytes);
    let total_live = batch_bytes.checked_add(8).expect("test live envelope");
    let ledger = frozen_graph_ledger(startup_limits(total_live, total_live, 12, 12, 10));
    let parts = StartupLedgerPlanParts {
        batches,
        retained_attributions,
        account_shares: startup_account_shares(),
        commitments: startup_commitments(Dimension::Records, Dimension::Records),
    };

    preflight_startup(&ledger, &parts).expect("the exact startup envelope must fit current state");

    assert_eq!(startup_batch_bytes(&parts.batches), batch_bytes);
    assert_eq!(parts.retained_attributions, retained_attributions);
    let StartupRetainedAttribution {
        phase,
        dimension,
        bytes,
    } = parts.retained_attributions[0];
    assert_eq!(
        (phase, dimension, bytes),
        (Phase::Build, Dimension::Bytes, 3)
    );
    let StartupAttributionBatchPlans {
        canonical_assembly: _canonical,
        evidence_assembly: _evidence_assembly,
        fallback: _fallback,
        evidence: _evidence,
    } = parts.batches;
    for share in parts.account_shares.into_steps() {
        assert_eq!(share.into_charges(), startup_share_charges());
    }
    assert_eq!(parts.commitments[0].dimension(), Dimension::Records);
    assert_eq!(parts.commitments[0].amount(), 4);
    assert_eq!(parts.commitments[1].dimension(), Dimension::Records);
    assert_eq!(parts.commitments[1].amount(), 6);
    assert_eq!(ledger.work_catalog.len(), 2);
}

#[test]
fn startup_live_and_observed_work_are_independent_on_the_same_dimension() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let ledger = frozen_graph_ledger(startup_limits(total_live, total_live, 12, 12, 10));
    let account_shares =
        StartupAccountShareEnvelope::four_identical([(Dimension::Bytes, 1), (Dimension::Bytes, 2)]);

    let parts = StartupLedgerPlanParts {
        batches,
        retained_attributions: startup_attributions(Dimension::Bytes, Dimension::Bytes),
        account_shares,
        commitments: startup_commitments(Dimension::Bytes, Dimension::Bytes),
    };

    assert!(
        preflight_startup(&ledger, &parts).is_ok(),
        "live reservations and observed work use independent ledger counter classes"
    );
}

#[test]
fn startup_preflight_rejects_unknown_dimensions_without_allocation() {
    let ledger = frozen_graph_ledger(startup_limits(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ));
    let allocations_before = test_attribution_batch_allocation_entries();

    for parts in [
        StartupLedgerPlanParts {
            batches: startup_batch_plans(Dimension::Unknown),
            retained_attributions: startup_attributions(Dimension::Bytes, Dimension::Bytes),
            account_shares: startup_account_shares(),
            commitments: startup_commitments(Dimension::Records, Dimension::Records),
        },
        StartupLedgerPlanParts {
            batches: startup_batch_plans(Dimension::Bytes),
            retained_attributions: startup_attributions(Dimension::Unknown, Dimension::Bytes),
            account_shares: startup_account_shares(),
            commitments: startup_commitments(Dimension::Records, Dimension::Records),
        },
        StartupLedgerPlanParts {
            batches: startup_batch_plans(Dimension::Bytes),
            retained_attributions: startup_attributions(Dimension::Bytes, Dimension::Bytes),
            account_shares: StartupAccountShareEnvelope::four_identical([
                (Dimension::Unknown, 1),
                (Dimension::Records, 2),
            ]),
            commitments: startup_commitments(Dimension::Records, Dimension::Records),
        },
        StartupLedgerPlanParts {
            batches: startup_batch_plans(Dimension::Bytes),
            retained_attributions: startup_attributions(Dimension::Bytes, Dimension::Bytes),
            account_shares: startup_account_shares(),
            commitments: startup_commitments(Dimension::Unknown, Dimension::Records),
        },
    ] {
        assert_eq!(
            preflight_startup(&ledger, &parts),
            Err(LedgerCause::UnknownDimension {
                dimension: Dimension::Unknown,
            })
        );
    }

    assert_eq!(
        test_attribution_batch_allocation_entries(),
        allocations_before,
        "current-state preflight rejection must not allocate a batch"
    );
}

#[test]
fn startup_preflight_checks_live_dimension_and_operation_limits() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let dimension_limited = frozen_graph_ledger(startup_limits(
        u64::MAX,
        total_live.checked_sub(1).expect("nonzero envelope"),
        12,
        12,
        10,
    ));
    let parts = StartupLedgerPlanParts {
        batches,
        retained_attributions: startup_attributions(Dimension::Bytes, Dimension::Bytes),
        account_shares: startup_account_shares(),
        commitments: startup_commitments(Dimension::Records, Dimension::Records),
    };
    assert!(matches!(
        preflight_startup(&dimension_limited, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value,
            limit,
        }) if value == total_live && limit == total_live - 1
    ));

    let operation_limited = frozen_graph_ledger(startup_limits(
        total_live.checked_sub(1).expect("nonzero envelope"),
        total_live,
        12,
        12,
        10,
    ));
    assert!(matches!(
        preflight_startup(&operation_limited, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value,
            limit,
        }) if value == total_live && limit == total_live - 1
    ));

    let non_reservable = frozen_graph_ledger(startup_limits(u64::MAX, u64::MAX, 12, 12, 10));
    let parts = StartupLedgerPlanParts {
        batches: startup_batch_plans(Dimension::Records),
        retained_attributions: startup_attributions(Dimension::Bytes, Dimension::Bytes),
        account_shares: startup_account_shares(),
        commitments: startup_commitments(Dimension::Records, Dimension::Records),
    };
    assert!(matches!(
        preflight_startup(&non_reservable, &parts),
        Err(LedgerCause::DimensionNotReservable {
            dimension: Dimension::Records,
        })
    ));

    let live_overflow = frozen_graph_ledger(startup_limits(u64::MAX, u64::MAX, 12, 12, 10));
    let parts = StartupLedgerPlanParts {
        batches: startup_batch_plans(Dimension::Bytes),
        retained_attributions: [
            StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, u64::MAX),
            StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, 0),
        ],
        account_shares: startup_account_shares(),
        commitments: startup_commitments(Dimension::Records, Dimension::Records),
    };
    assert!(matches!(
        preflight_startup(&live_overflow, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
        })
    ));
}

#[test]
fn startup_preflight_aggregates_four_account_shares() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let attempted_limited = frozen_graph_ledger(startup_limits(total_live, total_live, 11, 11, 10));
    let mut parts = StartupLedgerPlanParts {
        batches,
        retained_attributions: startup_attributions(Dimension::Bytes, Dimension::Bytes),
        account_shares: startup_account_shares(),
        commitments: startup_commitments(Dimension::Records, Dimension::Records),
    };
    assert!(matches!(
        preflight_startup(&attempted_limited, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 12,
            limit: 11,
        })
    ));

    let consumed_limited = frozen_graph_ledger(startup_limits(total_live, total_live, 12, 11, 10));
    assert!(matches!(
        preflight_startup(&consumed_limited, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 12,
            limit: 11,
        })
    ));

    let share_overflow = frozen_graph_ledger(startup_limits(
        total_live,
        total_live,
        u64::MAX,
        u64::MAX,
        10,
    ));
    parts.account_shares = StartupAccountShareEnvelope::four_identical([
        (Dimension::Records, u64::MAX),
        (Dimension::Records, 0),
    ]);
    assert!(matches!(
        preflight_startup(&share_overflow, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
        })
    ));
}

#[test]
fn startup_preflight_checked_sums_outcome_evidence_commitments() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let evidence_limited = frozen_graph_ledger(startup_limits(total_live, total_live, 12, 12, 9));
    let mut parts = StartupLedgerPlanParts {
        batches,
        retained_attributions: startup_attributions(Dimension::Bytes, Dimension::Bytes),
        account_shares: startup_account_shares(),
        commitments: startup_commitments(Dimension::Records, Dimension::Records),
    };
    assert!(matches!(
        preflight_startup(&evidence_limited, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension: Dimension::Records,
            value: 10,
            limit: 9,
        })
    ));

    let overflow_ledger =
        frozen_graph_ledger(startup_limits(total_live, total_live, 12, 12, u64::MAX));
    parts.commitments = [
        OutputCommitment::new(Dimension::Records, u64::MAX),
        OutputCommitment::new(Dimension::Records, 1),
    ];
    assert!(matches!(
        preflight_startup(&overflow_ledger, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension: Dimension::Records,
        })
    ));
}

#[test]
fn startup_preflight_includes_existing_live_and_observed_work() {
    let (_, parts, covers) = default_graph_request();
    let total_live = covers
        .into_iter()
        .try_fold(8_u64, u64::checked_add)
        .expect("startup live envelope");
    for (operation_limit, dimension_limit, fits) in [
        (total_live + 11, total_live + 11, true),
        (total_live + 10, u64::MAX, false),
        (u64::MAX, total_live + 10, false),
    ] {
        let mut ledger = frozen_graph_ledger(graph_limits(
            operation_limit,
            dimension_limit,
            u64::MAX,
            19,
            15,
        ));
        ledger
            .charge_attempted(Dimension::Records, 7)
            .expect("existing attempted work");
        ledger
            .charge_consumed(Dimension::Records, 3)
            .expect("existing consumed work");
        let live = ledger
            .reserve(Phase::Build, Dimension::Bytes, 11)
            .expect("existing live bytes");

        if fits {
            assert_eq!(preflight_startup(&ledger, &parts), Ok(()));
        } else {
            assert_eq!(
                preflight_startup(&ledger, &parts),
                Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: Dimension::Bytes,
                    value: total_live + 11,
                    limit: total_live + 10,
                })
            );
        }
        assert_eq!(
            ledger_graph_observation(&ledger),
            (11, 1, 11, 0, 0, 7, 3, 0)
        );
        drop(live);
    }
}

#[test]
fn startup_preflight_uses_current_share_policy_and_pending_consumption() {
    let (limits, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    let (layout, mut pending) = {
        let (mut work, _) = ledger.split_internal();
        let layout = work
            .bind_ordinary([Dimension::Records])
            .expect("share work binding");
        let pending = layout
            .admit_observed(
                &mut work.reborrow().expect("current internal work").work,
                [4],
                super::super::WorkLane::Internal,
            )
            .expect("existing pending work");
        (layout, pending)
    };
    ledger
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(16, 16, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .expect("exact share headroom");
    assert_eq!(preflight_startup(&ledger, &parts), Ok(()));

    ledger
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(16, 15, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .expect("one-short current share headroom");
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 16,
            limit: 15,
        })
    );
    assert_eq!(ledger_graph_observation(&ledger), (0, 0, 0, 0, 0, 4, 0, 0));
    ledger
        .split_internal()
        .0
        .resolve(layout, &mut pending, false)
        .expect("release existing pending work");
    assert_eq!(preflight_startup(&ledger, &parts), Ok(()));
}

#[test]
fn startup_graph_projection_preserves_pending_work_and_current_share_policy() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    let (layout, mut pending) = {
        let (mut work, _storage) = ledger.split_internal();
        let layout = work
            .bind_ordinary([Dimension::Records])
            .expect("share work binding");
        let mut numeric = work.reborrow().expect("current internal work");
        let pending = layout
            .admit_observed(&mut numeric.work, [4], super::super::WorkLane::Internal)
            .expect("existing pending work");
        (layout, pending)
    };
    ledger
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(16, 16, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .expect("exact share headroom");
    assert_eq!(preflight_startup(&ledger, &parts), Ok(()));
    let graph = ledger
        .try_bind_startup_attribution_graph(parts)
        .expect("startup graph permits an existing internal frame");
    {
        let state = &ledger.work;
        let records = state
            .rows
            .iter()
            .find(|row| row.dimension == Dimension::Records)
            .expect("record work state");
        assert_eq!(state.header.open_envelopes, 1);
        assert_eq!(records.state.attempted, 4);
        assert_eq!(records.state.consumed, 0);
        assert_eq!(records.state.pending_consumed, 4);
    }
    assert_eq!(ledger.test_probe().active_reservations(), 6);
    drop(graph);

    ledger
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(16, 15, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .expect("one-short current share headroom");
    let (_, parts, _) = default_graph_request();
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 16,
            limit: 15,
        })
    );
    assert!(matches!(
        ledger.try_bind_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: Dimension::Records,
                value: 16,
                limit: 15,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), (0, 0, 0, 0, 0, 4, 0, 0));
    assert_eq!(test_startup_graph_allocation_entries(), 8);
    ledger
        .split_internal()
        .0
        .resolve(layout, &mut pending, false)
        .expect("release only the existing frame's pending work");

    let (_, parts, _) = default_graph_request();
    let graph = ledger
        .try_bind_startup_attribution_graph(parts)
        .expect("released pending work restores current share headroom");
    drop(graph);
    assert_eq!(ledger_graph_observation(&ledger), (0, 0, 0, 0, 0, 4, 0, 0));
}

#[test]
fn startup_preflight_includes_current_outcome_evidence_and_policy() {
    let (limits, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    {
        let mut state = ledger.state.write();
        let records = state
            .dimensions
            .iter_mut()
            .find(|entry| entry.dimension == Dimension::Records)
            .expect("record dimension");
        records.state.published_outcome_evidence = 1;
    }
    ledger
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(11),
        )
        .expect("exact evidence headroom");
    assert_eq!(preflight_startup(&ledger, &parts), Ok(()));

    ledger
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(10),
        )
        .expect("one-short evidence headroom");
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension: Dimension::Records,
            value: 11,
            limit: 10,
        })
    );

    {
        let mut state = ledger.state.write();
        let records = state
            .dimensions
            .iter_mut()
            .find(|entry| entry.dimension == Dimension::Records)
            .expect("record dimension");
        records.state.published_outcome_evidence = u64::MAX;
    }
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension: Dimension::Records,
        })
    );
}

#[test]
fn startup_preflight_preserves_live_work_and_commitment_error_order() {
    let (_, mut parts, covers) = default_graph_request();
    let total_live = covers
        .into_iter()
        .try_fold(8_u64, u64::checked_add)
        .expect("startup live envelope");
    let mut ledger = frozen_graph_ledger(graph_limits(u64::MAX, total_live - 1, u64::MAX, 11, 11));
    parts.account_shares = StartupAccountShareEnvelope::four_identical([
        (Dimension::Unknown, 1),
        (Dimension::Records, 2),
    ]);
    parts.commitments[0] = OutputCommitment::new(Dimension::Unknown, 4);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value: total_live,
            limit: total_live - 1,
        })
    );
    assert_eq!(
        ledger
            .view()
            .try_admit_startup_ledger(
                parts.reservation_specs(),
                parts.account_shares.work_specs(),
                parts.commitments,
            )
            .err(),
        Some(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value: total_live,
            limit: total_live - 1,
        })
    );

    let mut ledger = frozen_graph_ledger(graph_limits(u64::MAX, u64::MAX, u64::MAX, 11, 11));
    parts.account_shares = startup_account_shares();
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 12,
            limit: 11,
        })
    );
    assert_eq!(
        ledger
            .view()
            .try_admit_startup_ledger(
                parts.reservation_specs(),
                parts.account_shares.work_specs(),
                parts.commitments,
            )
            .err(),
        Some(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 12,
            limit: 11,
        })
    );

    let mut ledger = frozen_graph_ledger(graph_limits(u64::MAX, u64::MAX, u64::MAX, 3, 3));
    parts.account_shares = StartupAccountShareEnvelope::four_identical([
        (Dimension::Records, 1),
        (Dimension::Bytes, u64::MAX),
    ]);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 4,
            limit: 3,
        })
    );
    assert_eq!(
        ledger
            .view()
            .try_admit_startup_ledger(
                parts.reservation_specs(),
                parts.account_shares.work_specs(),
                parts.commitments,
            )
            .err(),
        Some(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 4,
            limit: 3,
        })
    );
    parts.account_shares = StartupAccountShareEnvelope::four_identical([
        (Dimension::Bytes, u64::MAX),
        (Dimension::Records, 1),
    ]);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
        })
    );
    assert_eq!(
        ledger
            .view()
            .try_admit_startup_ledger(
                parts.reservation_specs(),
                parts.account_shares.work_specs(),
                parts.commitments,
            )
            .err(),
        Some(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
        })
    );
}

#[test]
fn startup_preflight_checks_current_publication_before_deferred_bindings() {
    use super::super::PublicationState;

    for (publication, cause) in [
        (
            PublicationState::Closing,
            LedgerCause::PublicationAlreadyPending,
        ),
        (
            PublicationState::Pending,
            LedgerCause::PublicationAlreadyPending,
        ),
        (
            PublicationState::TerminalQualified,
            LedgerCause::PublicationAlreadyCompleted,
        ),
        (
            PublicationState::Published,
            LedgerCause::PublicationAlreadyCompleted,
        ),
    ] {
        let (limits, mut parts, _) = default_graph_request();
        let mut ledger = frozen_graph_ledger(limits);
        ledger.state.write().publication = publication;
        parts.batches.canonical_assembly.phase = Phase::Unknown;
        assert_eq!(preflight_startup(&ledger, &parts), Err(cause.clone()));
        assert!(matches!(
            ledger.try_bind_startup_attribution_graph(parts),
            Err(AttributionBatchCreateError::Ledger(actual)) if actual == cause
        ));
    }
}

#[test]
fn startup_graph_defers_work_faults_until_after_storage_and_live_checks() {
    use super::super::PublicationState;

    for shape_fault in [false, true] {
        for (publication, phase, limited_live) in [
            (PublicationState::Pending, Phase::Unknown, false),
            (PublicationState::Open, Phase::Unknown, false),
            (PublicationState::Open, Phase::Build, true),
            (PublicationState::Open, Phase::Build, false),
        ] {
            test_reset_startup_graph_probes(None);
            let (_, mut parts, covers) = default_graph_request();
            let total_live = covers
                .into_iter()
                .try_fold(8_u64, u64::checked_add)
                .expect("startup graph live bytes");
            let live_limit = if limited_live {
                total_live - 1
            } else {
                u64::MAX
            };
            let mut ledger = frozen_graph_ledger(graph_limits(
                u64::MAX,
                live_limit,
                u64::MAX,
                u64::MAX,
                u64::MAX,
            ));
            parts.batches.canonical_assembly.phase = phase;
            parts.commitments[0] = OutputCommitment::new(Dimension::Unknown, 4);
            ledger.state.write().publication = publication;
            if shape_fault {
                ledger.work.rows.pop().expect("work row for shape fault");
            } else {
                ledger.work.header.integrity_fault = true;
            }
            let cause = if publication == PublicationState::Pending {
                LedgerCause::PublicationAlreadyPending
            } else if phase == Phase::Unknown {
                LedgerCause::UnknownPhase
            } else if limited_live {
                LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: Dimension::Bytes,
                    value: total_live,
                    limit: live_limit,
                }
            } else {
                LedgerCause::FinalSnapshotShapeMismatch
            };
            assert!(matches!(
                ledger.preflight_startup_attribution_graph(&parts),
                Err(AttributionBatchCreateError::Ledger(actual)) if actual == cause
            ));
            assert!(matches!(
                ledger.try_bind_startup_attribution_graph(parts),
                Err(AttributionBatchCreateError::Ledger(actual)) if actual == cause
            ));
            let (_, mut invalid_capacity, _) = default_graph_request();
            invalid_capacity.batches.fallback.attribution_limit = 0;
            assert!(matches!(
                ledger.try_bind_startup_attribution_graph(invalid_capacity),
                Err(AttributionBatchCreateError::CapacityOverflow)
            ));
            assert_eq!(test_startup_graph_allocation_entries(), 0);
            assert_eq!(test_startup_graph_storage_releases(), 0);
            assert_eq!(test_startup_graph_ledger_release_locks(), 0);
            let state = ledger.state.read();
            assert_eq!(state.current_live_bytes, 0);
            assert_eq!(state.active_reservations, 0);
        }
    }
}

#[test]
fn startup_graph_owns_exact_envelopes_and_drops_storage_before_one_ledger_rollback() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, covers) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    let total_live = covers
        .into_iter()
        .try_fold(8_u64, u64::checked_add)
        .expect("startup graph live bytes");

    assert_eq!(preflight_startup(&ledger, &parts), Ok(()));
    let (graph, account_shares) = ledger
        .try_admit_startup_attribution_graph(parts)
        .expect("startup graph admission")
        .into_parts();

    assert_eq!(test_startup_graph_allocation_entries(), 8);
    assert_eq!(ledger.test_probe().active_reservations(), 6);
    assert_eq!(ledger.test_probe().current_live_bytes(), total_live);
    let ownership = graph
        .ownership
        .as_ref()
        .expect("startup graph ownership before drop");
    assert_eq!(
        [
            ownership.cores.canonical_assembly_cover.bytes,
            ownership.cores.evidence_assembly_cover.bytes,
            ownership.cores.fallback_cover.bytes,
            ownership.cores.evidence_cover.bytes,
        ],
        covers
    );
    assert_eq!(
        [
            ownership.cores.fallback_attribution.bytes,
            ownership.cores.evidence_attribution.bytes,
        ],
        [3, 5]
    );
    for (actual, cover) in [
        batch_backing_bytes(&ownership.canonical_assembly),
        batch_backing_bytes(&ownership.evidence_assembly),
        batch_backing_bytes(&ownership.fallback),
        batch_backing_bytes(&ownership.evidence),
    ]
    .into_iter()
    .zip(covers)
    {
        assert!(actual <= cover);
    }
    assert_eq!(ownership.canonical_assembly.commitments.len(), 0);
    assert_eq!(ownership.evidence_assembly.commitments.len(), 0);
    assert_eq!(ownership.fallback.commitments.len(), 1);
    assert_eq!(ownership.evidence.commitments.len(), 1);
    assert_eq!(account_shares.into_steps().len(), 4);

    drop(graph);

    assert_eq!(test_startup_graph_storage_releases(), 8);
    assert_eq!(test_startup_graph_ledger_release_locks(), 1);
    assert_eq!(
        test_startup_graph_storage_releases_at_ledger_lock(),
        Some(8)
    );
    assert_eq!(ledger.test_probe().active_reservations(), 0);
    assert_eq!(ledger.test_probe().current_live_bytes(), 0);
    let records = ledger
        .dimension_evidence(Dimension::Records)
        .expect("record evidence");
    assert_eq!(records.attempted(), 0);
    assert_eq!(records.consumed(), 0);
    assert_eq!(records.published_outcome_evidence(), 0);
}

#[test]
fn startup_graph_accounts_cross_dimension_and_phase_reservations_exactly() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, covers) = graph_request(
        [Phase::Build, Phase::Finalize, Phase::Build, Phase::Finalize],
        [
            Dimension::Bytes,
            Dimension::Scratch,
            Dimension::Scratch,
            Dimension::Bytes,
        ],
        [
            (Phase::Build, Dimension::Bytes, 3),
            (Phase::Finalize, Dimension::Scratch, 5),
        ],
    );
    let mut ledger = frozen_graph_ledger(limits);
    assert_eq!(preflight_startup(&ledger, &parts), Ok(()));
    let graph = ledger
        .try_admit_startup_attribution_graph(parts)
        .expect("cross-phase startup graph");

    let expected_bytes = covers[0]
        .checked_add(covers[3])
        .and_then(|bytes| bytes.checked_add(3))
        .expect("byte dimension total");
    let expected_scratch = covers[1]
        .checked_add(covers[2])
        .and_then(|bytes| bytes.checked_add(5))
        .expect("scratch dimension total");
    let expected_build = covers[0]
        .checked_add(covers[2])
        .and_then(|bytes| bytes.checked_add(3))
        .expect("build phase total");
    let expected_finalize = covers[1]
        .checked_add(covers[3])
        .and_then(|bytes| bytes.checked_add(5))
        .expect("finalize phase total");
    assert_eq!(
        ledger
            .dimension_evidence(Dimension::Bytes)
            .expect("byte evidence")
            .current_live_bytes(),
        expected_bytes
    );
    assert_eq!(
        ledger
            .dimension_evidence(Dimension::Scratch)
            .expect("scratch evidence")
            .current_live_bytes(),
        expected_scratch
    );
    assert_eq!(
        ledger
            .phase_evidence(Phase::Build)
            .expect("build evidence")
            .current_live_bytes(),
        expected_build
    );
    assert_eq!(
        ledger
            .phase_evidence(Phase::Finalize)
            .expect("finalize evidence")
            .current_live_bytes(),
        expected_finalize
    );

    drop(graph);
    assert_eq!(ledger.test_probe().current_live_bytes(), 0);
    assert_eq!(ledger.test_probe().active_reservations(), 0);
    assert_eq!(
        ledger
            .phase_evidence(Phase::Build)
            .expect("build evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        ledger
            .phase_evidence(Phase::Finalize)
            .expect("finalize evidence")
            .current_live_bytes(),
        0
    );
}

#[test]
fn startup_graph_rejects_missing_terminal_slots_before_ledger_or_allocator_side_effects() {
    for batches in [
        graph_batch_plans_with_terminal_limits(0, 1, 1, 1),
        graph_batch_plans_with_terminal_limits(1, 1, 0, 1),
        graph_batch_plans_with_terminal_limits(1, 0, 1, 1),
        graph_batch_plans_with_terminal_limits(1, 1, 1, 0),
    ] {
        test_reset_startup_graph_probes(None);
        let (limits, parts, _) = graph_request_from_batches(
            batches,
            [
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, 3),
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, 5),
            ],
        );
        let mut ledger = frozen_graph_ledger(limits);
        let before = ledger_graph_observation(&ledger);

        assert!(matches!(
            ledger.preflight_startup_attribution_graph(&parts),
            Err(AttributionBatchCreateError::CapacityOverflow)
        ));
        assert!(matches!(
            ledger.try_admit_startup_attribution_graph(parts),
            Err(AttributionBatchCreateError::CapacityOverflow)
        ));
        assert_eq!(ledger_graph_observation(&ledger), before);
        assert_eq!(test_startup_graph_allocation_entries(), 0);
        assert_eq!(test_startup_graph_storage_releases(), 0);
        assert_eq!(test_startup_graph_ledger_release_locks(), 0);
    }
}

#[test]
fn startup_graph_rejects_unknown_phase_and_invalid_dimensions_without_a_prefix() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_graph_request();
    let mut ledger = Ledger::<Owner, Phase, Dimension>::try_new(limits, &[])
        .expect("empty frozen catalog is valid for an unknown-phase fixture");
    let before = ledger_graph_observation(&ledger);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::UnknownPhase)
    );
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::UnknownPhase
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = graph_request(
        [Phase::Unknown, Phase::Build, Phase::Build, Phase::Build],
        [Dimension::Bytes; 4],
        [
            (Phase::Build, Dimension::Bytes, 3),
            (Phase::Build, Dimension::Bytes, 5),
        ],
    );
    let mut ledger = frozen_graph_ledger(limits);
    let before = ledger_graph_observation(&ledger);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::UnknownPhase)
    );
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::UnknownPhase
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (limits, mut parts, _) = default_graph_request();
    parts.batches.canonical_assembly.capacity_dimension = Dimension::Unknown;
    let mut ledger = frozen_graph_ledger(limits);
    let before = ledger_graph_observation(&ledger);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    );
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::UnknownDimension {
                dimension: Dimension::Unknown,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (limits, mut parts, _) = default_graph_request();
    parts.batches.canonical_assembly.capacity_dimension = Dimension::Records;
    let mut ledger = frozen_graph_ledger(limits);
    let before = ledger_graph_observation(&ledger);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::DimensionNotReservable {
            dimension: Dimension::Records,
        })
    );
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::DimensionNotReservable {
                dimension: Dimension::Records,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);
}

#[test]
fn startup_graph_runtime_live_work_and_commitment_limits_are_atomic() {
    test_reset_startup_graph_probes(None);
    let (_, parts, covers) = default_graph_request();
    let total_live = covers
        .into_iter()
        .try_fold(8_u64, u64::checked_add)
        .expect("startup graph live bytes");
    let mut ledger = frozen_graph_ledger(graph_limits(
        u64::MAX,
        total_live.checked_sub(1).expect("nonzero graph"),
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ));
    let before = ledger_graph_observation(&ledger);
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
                value,
                limit,
            }
        )) if value == total_live && limit == total_live - 1
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (_, parts, covers) = default_graph_request();
    let total_live = covers
        .into_iter()
        .try_fold(8_u64, u64::checked_add)
        .expect("startup graph live bytes");
    let mut ledger = frozen_graph_ledger(graph_limits(
        total_live.checked_sub(1).expect("nonzero graph"),
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ));
    let before = ledger_graph_observation(&ledger);
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
                value,
                limit,
            }
        )) if value == total_live && limit == total_live - 1
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (_, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(graph_limits(u64::MAX, u64::MAX, u64::MAX, 11, 11));
    let before = ledger_graph_observation(&ledger);
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Records,
                value: 12,
                limit: 11,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (_, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(graph_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, 11));
    let before = ledger_graph_observation(&ledger);
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: Dimension::Records,
                value: 12,
                limit: 11,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (_, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(graph_limits_with_published(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        9,
    ));
    let before = ledger_graph_observation(&ledger);
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::PublishedOutcomeEvidence,
                dimension: Dimension::Records,
                value: 10,
                limit: 9,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);
}

#[test]
fn startup_graph_overflow_checks_leave_all_prior_ledger_state_unchanged() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    ledger
        .charge_attempted(Dimension::Records, u64::MAX)
        .expect("maximum attempted baseline");
    ledger
        .charge_consumed(Dimension::Records, u64::MAX)
        .expect("maximum consumed baseline");
    let before = ledger_graph_observation(&ledger);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
        })
    );
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::ArithmeticOverflow {
                class: CounterClass::Attempted,
                dimension: Dimension::Records,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    let live = ledger
        .reserve(Phase::Build, Dimension::Bytes, u64::MAX)
        .expect("maximum live baseline");
    let before = ledger_graph_observation(&ledger);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
        })
    );
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);
    drop(live);

    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    {
        let mut state = ledger.state.write();
        state.active_reservations = u64::MAX;
    }
    let before = ledger_graph_observation(&ledger);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
        })
    );
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);
    ledger.state.write().active_reservations = 0;

    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_graph_request();
    let mut ledger = frozen_graph_ledger(limits);
    let build_position = ledger
        .bound_phase_position(Phase::Build)
        .expect("build phase");
    {
        let mut state = ledger.state.write();
        state.phase_states[build_position].current_live_bytes = u64::MAX;
        state.phase_states[build_position].peak_live_bytes = u64::MAX;
    }
    let before_phase = ledger
        .phase_evidence(Phase::Build)
        .expect("build phase evidence");
    let before = ledger_graph_observation(&ledger);
    assert_eq!(
        preflight_startup(&ledger, &parts),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
        })
    );
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
            }
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(
        ledger
            .phase_evidence(Phase::Build)
            .expect("build phase evidence"),
        before_phase
    );
    assert_eq!(test_startup_graph_allocation_entries(), 0);
    {
        let mut state = ledger.state.write();
        state.phase_states[build_position].current_live_bytes = 0;
    }
}

#[test]
fn every_startup_graph_allocator_failure_drops_eight_backings_then_rolls_back_six_cores_once() {
    for failure_index in 0..8 {
        test_reset_startup_graph_probes(Some(failure_index));
        let (limits, parts, _) = default_graph_request();
        let mut ledger = frozen_graph_ledger(limits);

        assert!(matches!(
            ledger.try_admit_startup_attribution_graph(parts),
            Err(AttributionBatchCreateError::AllocationFailed)
        ));
        assert_eq!(test_startup_graph_allocation_entries(), failure_index + 1);
        assert_eq!(test_startup_graph_storage_releases(), 8);
        assert_eq!(test_startup_graph_ledger_release_locks(), 1);
        assert_eq!(
            test_startup_graph_storage_releases_at_ledger_lock(),
            Some(8)
        );
        assert_eq!(ledger.test_probe().current_live_bytes(), 0);
        assert_eq!(ledger.test_probe().active_reservations(), 0);
        let records = ledger
            .dimension_evidence(Dimension::Records)
            .expect("record evidence");
        assert_eq!(records.attempted(), 0);
        assert_eq!(records.consumed(), 0);
    }
}

#[test]
fn startup_graph_allocator_failpoints_are_thread_local() {
    let workers = (0..8)
        .map(|failure_index| {
            std::thread::spawn(move || {
                test_reset_startup_graph_probes(Some(failure_index));
                let (limits, parts, _) = default_graph_request();
                let mut ledger = frozen_graph_ledger(limits);

                assert!(matches!(
                    ledger.try_admit_startup_attribution_graph(parts),
                    Err(AttributionBatchCreateError::AllocationFailed)
                ));
                assert_eq!(test_startup_graph_allocation_entries(), failure_index + 1);
                assert_eq!(test_startup_graph_storage_releases(), 8);
                assert_eq!(test_startup_graph_ledger_release_locks(), 1);
                assert_eq!(
                    test_startup_graph_storage_releases_at_ledger_lock(),
                    Some(8)
                );
                assert_eq!(ledger.test_probe().current_live_bytes(), 0);
                assert_eq!(ledger.test_probe().active_reservations(), 0);
            })
        })
        .collect::<Vec<_>>();

    for worker in workers {
        worker.join().expect("startup graph failpoint worker");
    }
}

#[test]
fn finite_startup_graph_promotes_actual_storage_and_drops_exact_credits_after_backings() {
    test_reset_startup_graph_probes(None);
    let mut ledger = finite_startup_ledger(u64::MAX, u64::MAX);
    let (parts, covers) = finite_graph_request();
    let total_live = covers.into_iter().sum::<u64>() + 8;
    let identity = ledger.identity;
    let work_rows = ledger.work.as_ptr();
    let (layout, mut pending) = {
        let (mut work, _) = ledger.view().into_internal();
        let layout = work
            .bind_ordinary([Dimension::Records])
            .expect("finite share layout");
        let pending = layout
            .admit_observed(
                &mut work.reborrow().expect("finite work view").work,
                [4],
                WorkLane::Internal,
            )
            .expect("actual pending finite work");
        (layout, pending)
    };
    let before_work = finite_work_observation(&ledger);

    ledger
        .view()
        .preflight_startup_attribution_graph(&parts)
        .expect("finite read-only preflight");
    assert!(ledger.storage.shared().is_none());
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    assert_eq!(test_startup_graph_allocation_entries(), 0);
    assert_eq!(finite_work_observation(&ledger), before_work);

    let (graph, shares) = ledger
        .view()
        .try_admit_startup_attribution_graph(parts)
        .expect("actual finite startup promotion")
        .into_parts();
    let shared = Arc::clone(ledger.storage.shared().expect("promoted finite storage"));
    let administrative_bytes = ledger
        .known_administrative_bytes(0)
        .expect("owned storage bytes");
    assert!(administrative_bytes > 0);
    assert_eq!(ledger.identity, identity);
    assert_eq!(ledger.work.as_ptr(), work_rows);
    assert_eq!(finite_work_observation(&ledger), before_work);
    assert!(graph.belongs_to_view(&ledger.view().as_read()));
    assert_eq!(test_startup_graph_allocation_entries(), 8);

    let ownership = graph
        .ownership
        .as_ref()
        .expect("finite graph backings and credits");
    assert!(
        ownership
            .runtime_owners
            .iter()
            .all(|owner| Arc::ptr_eq(owner, &shared))
    );
    for core in [
        &ownership.cores.canonical_assembly_cover,
        &ownership.cores.evidence_assembly_cover,
        &ownership.cores.fallback_cover,
        &ownership.cores.evidence_cover,
        &ownership.cores.fallback_attribution,
        &ownership.cores.evidence_attribution,
    ] {
        assert!(Arc::ptr_eq(
            core.owner.as_ref().expect("issued finite credit"),
            &shared
        ));
    }
    for (actual, cover) in [
        batch_backing_bytes(&ownership.canonical_assembly),
        batch_backing_bytes(&ownership.evidence_assembly),
        batch_backing_bytes(&ownership.fallback),
        batch_backing_bytes(&ownership.evidence),
    ]
    .into_iter()
    .zip(covers)
    {
        assert!(actual <= cover);
    }
    assert!(ownership.canonical_assembly.commitments.is_empty());
    assert!(ownership.evidence_assembly.commitments.is_empty());
    assert_eq!(
        ownership.fallback.commitments[0].dimension,
        Dimension::Records
    );
    assert_eq!(ownership.fallback.commitments[0].amount, 4);
    assert_eq!(
        ownership.evidence.commitments[0].dimension,
        Dimension::Records
    );
    assert_eq!(ownership.evidence.commitments[0].amount, 6);
    for step in shares.into_steps() {
        assert_eq!(step.into_charges(), startup_share_charges());
    }
    {
        let state = lock(&shared);
        assert_eq!(state.active_reservations, 6);
        assert_eq!(state.current_live_bytes, total_live);
        assert_eq!(state.peak_live_bytes, total_live);
        assert_eq!(state.current_administrative_bytes, administrative_bytes);
        let expected_dimensions = [covers[0] + 3, 0, covers[1], covers[2], covers[3], 5];
        for (entry, expected) in state.dimensions.iter().zip(expected_dimensions) {
            assert_eq!(entry.state.current_live_bytes, expected);
            assert_eq!(entry.state.peak_live_bytes, expected);
            assert_eq!(entry.state.published_outcome_evidence, 0);
        }
        for (phase, expected) in
            state
                .phase_states
                .iter()
                .zip([covers[2] + 5, covers[0] + covers[3], covers[1] + 3])
        {
            assert_eq!(phase.current_live_bytes, expected);
            assert_eq!(phase.peak_live_bytes, expected);
        }
    }

    drop(graph);

    assert_eq!(test_startup_graph_storage_releases(), 8);
    assert_eq!(test_startup_graph_ledger_release_locks(), 1);
    assert_eq!(
        test_startup_graph_storage_releases_at_ledger_lock(),
        Some(8)
    );
    assert_eq!(finite_work_observation(&ledger), before_work);
    assert_eq!(
        ledger.known_administrative_bytes(0),
        Some(administrative_bytes)
    );
    assert!(Arc::ptr_eq(
        &shared,
        ledger.storage.shared().expect("same finite storage")
    ));
    {
        let state = lock(&shared);
        assert_eq!(state.active_reservations, 0);
        assert_eq!(state.current_live_bytes, 0);
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
        assert_eq!(Arc::strong_count(&shared), 2);
    }
    ledger
        .view()
        .into_internal()
        .0
        .resolve(layout, &mut pending, false)
        .expect("resolve original finite frame");
}

#[test]
fn finite_startup_graph_reuses_shared_identity_after_owner_move_and_current_live_rejection() {
    test_reset_startup_graph_probes(None);
    let mut ledger = finite_startup_ledger(u64::MAX, u64::MAX);
    let (parts, covers) = finite_graph_request();
    let (first, _) = ledger
        .view()
        .try_admit_startup_attribution_graph(parts)
        .expect("first finite graph")
        .into_parts();
    let shared = Arc::clone(ledger.storage.shared().expect("first finite identity"));
    let administrative_bytes = ledger.known_administrative_bytes(0);
    let total_live = covers.into_iter().sum::<u64>() + 8;
    let mut moved = ledger;
    let mut foreign = finite_startup_ledger(u64::MAX, u64::MAX);
    assert!(first.belongs_to_view(&moved.view().as_read()));
    assert!(!first.belongs_to_view(&foreign.view().as_read()));
    let (foreign_parts, _) = finite_graph_request();
    let foreign_graph = foreign
        .view()
        .try_bind_startup_attribution_graph(foreign_parts)
        .expect("foreign finite graph with identical keys");
    assert!(!first.belongs_to_view(&foreign.view().as_read()));
    drop(foreign_graph);
    test_reset_startup_graph_probes(None);

    let bytes = covers[0] + 3;
    moved
        .view()
        .tighten_dimension_policy(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, 2 * bytes - 1)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .expect("tighten current finite storage policy");
    let (parts, _) = finite_graph_request();
    assert!(matches!(
        moved.view().try_bind_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value,
            limit,
        })) if value == 2 * bytes && limit == 2 * bytes - 1
    ));
    assert_eq!(test_startup_graph_allocation_entries(), 0);
    assert_eq!(lock(&shared).active_reservations, 6);
    assert_eq!(lock(&shared).current_live_bytes, total_live);
    assert_eq!(moved.known_administrative_bytes(0), administrative_bytes);
    assert!(first.belongs_to_view(&moved.view().as_read()));

    drop(first);
    let (parts, _) = finite_graph_request();
    let second = moved
        .view()
        .try_bind_startup_attribution_graph(parts)
        .expect("released credits fit the current tightened policy");
    assert!(Arc::ptr_eq(
        &shared,
        moved.storage.shared().expect("reused finite identity")
    ));
    assert_eq!(moved.known_administrative_bytes(0), administrative_bytes);
    assert_eq!(test_startup_graph_allocation_entries(), 8);
    drop(second);
    assert_eq!(lock(&shared).active_reservations, 0);
    assert_eq!(lock(&shared).current_live_bytes, 0);
    assert_eq!(Arc::strong_count(&shared), 2);
}

#[test]
fn finite_startup_graph_outlives_the_fixed_root_and_releases_its_actual_shared_state() {
    test_reset_startup_graph_probes(None);
    let (graph, shared, total_live) = {
        let mut ledger = finite_startup_ledger(u64::MAX, u64::MAX);
        let (parts, covers) = finite_graph_request();
        let (graph, _) = ledger
            .view()
            .try_admit_startup_attribution_graph(parts)
            .expect("finite graph before its root exits")
            .into_parts();
        let shared = Arc::clone(ledger.storage.shared().expect("escaping actual storage"));
        (graph, shared, covers.into_iter().sum::<u64>() + 8)
    };
    assert_eq!(lock(&shared).current_live_bytes, total_live);
    assert_eq!(lock(&shared).active_reservations, 6);
    drop(graph);
    assert_eq!(test_startup_graph_storage_releases(), 8);
    assert_eq!(test_startup_graph_ledger_release_locks(), 1);
    assert_eq!(
        test_startup_graph_storage_releases_at_ledger_lock(),
        Some(8)
    );
    assert_eq!(lock(&shared).current_live_bytes, 0);
    assert_eq!(lock(&shared).active_reservations, 0);
    assert_eq!(lock(&shared).peak_live_bytes, total_live);
    assert_eq!(Arc::strong_count(&shared), 1);
}

#[test]
fn finite_startup_numeric_rejections_precede_promotion_and_deferred_binding_errors() {
    let _guard = test_evidence_allocation_guard();
    let (_, covers) = finite_graph_request();
    let bytes = covers[0] + 3;
    let total_live = covers.into_iter().sum::<u64>() + 8;
    for (operation_limit, tightened, expected) in [
        (
            u64::MAX,
            Some((
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, bytes - 1)
                    .with_published_outcome_evidence_limit(u64::MAX),
            )),
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
                value: bytes,
                limit: bytes - 1,
            },
        ),
        (
            total_live - 1,
            None,
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::Progress,
                value: total_live,
                limit: total_live - 1,
            },
        ),
        (
            u64::MAX,
            Some((
                Dimension::Records,
                DimensionPolicy::counter(11, 11, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            )),
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Records,
                value: 12,
                limit: 11,
            },
        ),
        (
            u64::MAX,
            Some((
                Dimension::Records,
                DimensionPolicy::counter(12, 11, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            )),
            LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: Dimension::Records,
                value: 12,
                limit: 11,
            },
        ),
        (
            u64::MAX,
            Some((
                Dimension::Records,
                DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(9),
            )),
            LedgerCause::LimitExceeded {
                class: CounterClass::PublishedOutcomeEvidence,
                dimension: Dimension::Records,
                value: 10,
                limit: 9,
            },
        ),
    ] {
        test_reset_startup_graph_probes(None);
        let mut ledger = finite_startup_ledger(operation_limit, 0);
        let (mut parts, _) = finite_graph_request();
        if let Some((dimension, policy)) = tightened {
            ledger
                .view()
                .tighten_dimension_policy(dimension, policy)
                .expect("current finite limit");
        }
        match &expected {
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                ..
            } => {
                parts.account_shares = StartupAccountShareEnvelope::four_identical([
                    (Dimension::Unknown, 1),
                    (Dimension::Records, 2),
                ]);
                parts.commitments[0] = OutputCommitment::new(Dimension::Unknown, 4);
            }
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted | CounterClass::Consumed,
                ..
            } => parts.commitments[0] = OutputCommitment::new(Dimension::Unknown, 4),
            _ => {}
        }
        let before_work = finite_work_observation(&ledger);
        test_fail_next_evidence_allocation();
        assert!(matches!(
            ledger.view().preflight_startup_attribution_graph(&parts),
            Err(AttributionBatchCreateError::Ledger(actual)) if actual == expected
        ));
        assert!(matches!(
            ledger.view().try_bind_startup_attribution_graph(parts),
            Err(AttributionBatchCreateError::Ledger(actual)) if actual == expected
        ));
        assert!(test_evidence_allocation_failure_is_armed());
        test_set_evidence_allocation_failure(false);
        assert!(ledger.storage.shared().is_none());
        assert_eq!(finite_work_observation(&ledger), before_work);
        assert_eq!(test_startup_graph_allocation_entries(), 0);
        assert_eq!(test_startup_graph_storage_releases(), 0);
        assert_eq!(test_startup_graph_ledger_release_locks(), 0);
        let state = ledger.storage.read();
        let view = state.view();
        assert_eq!(view.current_live_bytes, 0);
        assert_eq!(view.peak_live_bytes, 0);
        assert_eq!(view.active_reservations, 0);
        assert_eq!(view.current_administrative_bytes, 0);
        assert_eq!(view.peak_administrative_bytes, 0);
        assert!(view.dimensions.iter().all(|entry| {
            entry.state.current_live_bytes == 0
                && entry.state.peak_live_bytes == 0
                && entry.state.published_outcome_evidence == 0
        }));
        assert!(
            view.phase_states
                .iter()
                .all(|phase| { phase.current_live_bytes == 0 && phase.peak_live_bytes == 0 })
        );
    }
}

#[test]
fn finite_startup_uses_current_pending_work_and_tightened_policies_on_both_storage_paths() {
    test_reset_startup_graph_probes(None);
    let mut ledger = finite_startup_ledger(u64::MAX, u64::MAX);
    let (layout, mut pending) = {
        let (mut work, _) = ledger.view().into_internal();
        let layout = work
            .bind_ordinary([Dimension::Records])
            .expect("finite pending share layout");
        let pending = layout
            .admit_observed(
                &mut work.reborrow().expect("finite pending work").work,
                [4],
                WorkLane::Internal,
            )
            .expect("finite pending consumption");
        (layout, pending)
    };
    ledger
        .view()
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(16, 15, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .expect("tightened finite work limits");
    let before_work = finite_work_observation(&ledger);
    let (parts, _) = finite_graph_request();
    assert!(matches!(
        ledger.view().preflight_startup_attribution_graph(&parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: Dimension::Records,
                value: 16,
                limit: 15,
            }
        ))
    ));
    assert!(matches!(
        ledger.view().try_bind_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: Dimension::Records,
                value: 16,
                limit: 15,
            }
        ))
    ));
    assert!(ledger.storage.shared().is_none());
    assert_eq!(finite_work_observation(&ledger), before_work);
    assert_eq!(test_startup_graph_allocation_entries(), 0);
    ledger
        .view()
        .into_internal()
        .0
        .resolve(layout, &mut pending, false)
        .expect("release pending finite work");

    let after_resolve = finite_work_observation(&ledger);
    let (parts, _) = finite_graph_request();
    let graph = ledger
        .view()
        .try_bind_startup_attribution_graph(parts)
        .expect("resolved pending work leaves exact startup headroom");
    let shared = Arc::clone(
        ledger
            .storage
            .shared()
            .expect("finite promotion after resolution"),
    );
    drop(graph);
    assert_eq!(finite_work_observation(&ledger), after_resolve);

    ledger
        .view()
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(15, 15, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .expect("tighten the shared finite work policy");
    let before_rejection = finite_work_observation(&ledger);
    let (parts, _) = finite_graph_request();
    assert!(matches!(
        ledger.view().try_bind_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Records,
                value: 16,
                limit: 15,
            }
        ))
    ));
    assert_eq!(finite_work_observation(&ledger), before_rejection);
    assert!(Arc::ptr_eq(
        &shared,
        ledger.storage.shared().expect("unchanged shared identity")
    ));
    assert_eq!(test_startup_graph_allocation_entries(), 8);
    assert_eq!(lock(&shared).active_reservations, 0);
    assert_eq!(lock(&shared).current_live_bytes, 0);
}

#[test]
fn finite_startup_checks_current_published_evidence_before_promoting() {
    test_reset_startup_graph_probes(None);
    let mut ledger = finite_startup_ledger(u64::MAX, 0);
    let records = ledger
        .catalog
        .binary_search(&Dimension::Records)
        .expect("finite records slot");
    ledger.storage.write().view_mut().dimensions[records]
        .state
        .published_outcome_evidence = 1;
    ledger
        .view()
        .tighten_dimension_policy(
            Dimension::Records,
            DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(10),
        )
        .expect("current published finite policy");
    let before_work = finite_work_observation(&ledger);
    let (parts, _) = finite_graph_request();
    assert!(matches!(
        ledger.view().preflight_startup_attribution_graph(&parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::PublishedOutcomeEvidence,
                dimension: Dimension::Records,
                value: 11,
                limit: 10,
            }
        ))
    ));
    assert!(matches!(
        ledger.view().try_bind_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::PublishedOutcomeEvidence,
                dimension: Dimension::Records,
                value: 11,
                limit: 10,
            }
        ))
    ));
    assert!(ledger.storage.shared().is_none());
    assert_eq!(finite_work_observation(&ledger), before_work);
    assert_eq!(test_startup_graph_allocation_entries(), 0);
    let state = ledger.storage.read();
    let view = state.view();
    assert_eq!(view.dimensions[records].state.published_outcome_evidence, 1);
    assert_eq!(view.active_reservations, 0);
    assert_eq!(view.current_live_bytes, 0);
    assert_eq!(view.current_administrative_bytes, 0);
    assert_eq!(view.peak_administrative_bytes, 0);
}

#[test]
fn finite_startup_promotion_denial_and_candidate_failure_preserve_the_actual_inline_source() {
    let _guard = test_evidence_allocation_guard();
    for allocation_failure in [false, true] {
        test_reset_startup_graph_probes(None);
        let evidence_limit = if allocation_failure { u64::MAX } else { 0 };
        let mut ledger = finite_startup_ledger(u64::MAX, evidence_limit);
        ledger
            .view()
            .charge_attempted(Dimension::Records, 2)
            .expect("existing finite attempts");
        ledger
            .view()
            .charge_consumed(Dimension::Records, 1)
            .expect("existing finite consumption");
        let before_work = finite_work_observation(&ledger);
        let work_rows = ledger.work.as_ptr();
        let identity = ledger.identity;
        let (dimensions, phases, phase_states, before_dimensions) = {
            let state = ledger.storage.read();
            let view = state.view();
            let rows: [_; 6] = std::array::from_fn(|position| {
                let entry = &view.dimensions[position];
                (entry.dimension, entry.policy, entry.state)
            });
            (
                view.dimensions.as_ptr(),
                state.phases().as_ptr(),
                view.phase_states.as_ptr(),
                rows,
            )
        };
        let (parts, _) = finite_graph_request();
        ledger
            .view()
            .preflight_startup_attribution_graph(&parts)
            .expect("finite numeric preflight before promotion");
        assert_eq!(ledger.known_administrative_bytes(0), Some(0));
        test_fail_next_evidence_allocation();
        let result = ledger.view().try_bind_startup_attribution_graph(parts);
        if allocation_failure {
            assert!(matches!(
                result,
                Err(AttributionBatchCreateError::Ledger(LedgerCause::Evidence {
                    cause: super::super::LedgerEvidenceError::AllocationFailed {
                        purpose: super::super::LedgerEvidencePurpose::SharedStorage,
                        requested_bytes,
                    },
                })) if requested_bytes == u64::try_from(std::mem::size_of::<
                    Mutex<Option<LedgerState<Phase, Dimension>>>,
                >()).expect("known shared payload bytes")
            ));
            assert!(!test_evidence_allocation_failure_is_armed());
        } else {
            assert!(matches!(
                result,
                Err(AttributionBatchCreateError::Ledger(LedgerCause::Evidence {
                    cause: super::super::LedgerEvidenceError::LimitExceeded {
                        purpose: super::super::LedgerEvidencePurpose::SharedStorage,
                        value,
                        limit: 0,
                    },
                })) if value > 0
            ));
            assert!(test_evidence_allocation_failure_is_armed());
        }
        test_set_evidence_allocation_failure(false);
        assert!(ledger.storage.shared().is_none());
        assert_eq!(ledger.identity, identity);
        assert_eq!(ledger.work.as_ptr(), work_rows);
        assert_eq!(finite_work_observation(&ledger), before_work);
        assert_eq!(test_startup_graph_allocation_entries(), 0);
        assert_eq!(test_startup_graph_storage_releases(), 0);
        assert_eq!(test_startup_graph_ledger_release_locks(), 0);
        {
            let state = ledger.storage.read();
            let view = state.view();
            let rows: [_; 6] = std::array::from_fn(|position| {
                let entry = &view.dimensions[position];
                (entry.dimension, entry.policy, entry.state)
            });
            assert_eq!(view.dimensions.as_ptr(), dimensions);
            assert_eq!(state.phases().as_ptr(), phases);
            assert_eq!(view.phase_states.as_ptr(), phase_states);
            assert_eq!(rows, before_dimensions);
            assert_eq!(view.current_live_bytes, 0);
            assert_eq!(view.peak_live_bytes, 0);
            assert_eq!(view.active_reservations, 0);
            assert_eq!(view.current_administrative_bytes, 0);
            assert_eq!(view.peak_administrative_bytes > 0, allocation_failure);
            assert_eq!(view.publication, PublicationState::Open);
            assert!(
                view.phase_states
                    .iter()
                    .all(|phase| { phase.current_live_bytes == 0 && phase.peak_live_bytes == 0 })
            );
        }
        if allocation_failure {
            let (parts, _) = finite_graph_request();
            let graph = ledger
                .view()
                .try_bind_startup_attribution_graph(parts)
                .expect("original inline source remains promotable after candidate failure");
            drop(graph);
            assert_eq!(ledger.identity, identity);
            assert_eq!(finite_work_observation(&ledger), before_work);
            assert_eq!(test_startup_graph_allocation_entries(), 8);
            assert_eq!(ledger.storage.read().view().current_live_bytes, 0);
            assert_eq!(ledger.storage.read().view().active_reservations, 0);
        }
    }
}

#[test]
fn every_finite_startup_backing_failure_releases_only_its_six_credits_after_eight_backings() {
    for already_shared in [false, true] {
        for failure_index in 0..8 {
            test_reset_startup_graph_probes(Some(failure_index));
            let mut ledger = finite_startup_ledger(u64::MAX, u64::MAX);
            let existing = already_shared.then(|| {
                ledger
                    .view()
                    .reserve(Phase::Finalize, Dimension::Progress, 7)
                    .expect("preexisting finite credit")
            });
            let shared_before = ledger.storage.shared().map(Arc::clone);
            let administrative_before = ledger.known_administrative_bytes(0);
            let (layout, mut pending) = {
                let (mut work, _) = ledger.view().into_internal();
                let layout = work
                    .bind_ordinary([Dimension::Records])
                    .expect("finite failure work layout");
                let pending = layout
                    .admit_observed(
                        &mut work.reborrow().expect("finite failure work").work,
                        [4],
                        WorkLane::Internal,
                    )
                    .expect("preexisting finite pending work");
                (layout, pending)
            };
            let before_work = finite_work_observation(&ledger);
            let work_rows = ledger.work.as_ptr();
            let (parts, covers) = finite_graph_request();
            assert!(matches!(
                ledger.view().try_bind_startup_attribution_graph(parts),
                Err(AttributionBatchCreateError::AllocationFailed)
            ));
            assert_eq!(test_startup_graph_allocation_entries(), failure_index + 1);
            assert_eq!(test_startup_graph_storage_releases(), 8);
            assert_eq!(test_startup_graph_ledger_release_locks(), 1);
            assert_eq!(
                test_startup_graph_storage_releases_at_ledger_lock(),
                Some(8)
            );
            assert_eq!(ledger.work.as_ptr(), work_rows);
            assert_eq!(finite_work_observation(&ledger), before_work);
            let shared = Arc::clone(
                ledger
                    .storage
                    .shared()
                    .expect("actual finite shared source"),
            );
            if let Some(before) = &shared_before {
                assert!(Arc::ptr_eq(before, &shared));
                assert_eq!(ledger.known_administrative_bytes(0), administrative_before);
            }
            drop(shared_before);
            let baseline_bytes = if already_shared { 7 } else { 0 };
            let baseline_reservations = u64::from(already_shared);
            let total_live = covers.into_iter().sum::<u64>() + 8 + baseline_bytes;
            {
                let state = lock(&shared);
                assert_eq!(state.current_live_bytes, baseline_bytes);
                assert_eq!(state.active_reservations, baseline_reservations);
                assert_eq!(state.peak_live_bytes, total_live);
                assert!(state.current_administrative_bytes > 0);
                for entry in &state.dimensions {
                    let expected = if entry.dimension == Dimension::Progress {
                        baseline_bytes
                    } else {
                        0
                    };
                    assert_eq!(entry.state.current_live_bytes, expected);
                    assert_eq!(entry.state.published_outcome_evidence, 0);
                }
                for (position, phase) in state.phase_states.iter().enumerate() {
                    assert_eq!(
                        phase.current_live_bytes,
                        if position == 2 { baseline_bytes } else { 0 }
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
                .expect("resolve only the original frame");
            drop(existing);
            assert_eq!(lock(&shared).current_live_bytes, 0);
            assert_eq!(lock(&shared).active_reservations, 0);
            assert_eq!(Arc::strong_count(&shared), 2);
        }
    }
}
