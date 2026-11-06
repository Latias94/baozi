use super::super::{
    AttributionBatchPlan, CounterClass, LedgerCause, LedgerLimits, OutcomeEvidenceDestination,
    OutputCommitment, test_attribution_batch_allocation_entries,
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
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    Bytes,
    Records,
    Scratch,
    Unknown,
}

#[test]
fn startup_current_state_admission_has_one_production_module() {
    let ledger_root = include_str!("../../ledger.rs");
    let reservation = include_str!("../reservation.rs");
    let startup = include_str!("../startup.rs");
    let crate_root = include_str!("../../lib.rs");

    assert!(ledger_root.contains("mod reservation;"));
    assert!(ledger_root.contains("pub(crate) use reservation::{"));
    assert!(ledger_root.contains("mod startup;"));
    assert!(ledger_root.contains("pub(crate) use startup::{"));
    assert!(!ledger_root.contains("pub(crate) struct StartupLedgerPlan<"));
    assert!(reservation.contains("pub(crate) struct AttributionReservationBatch<"));
    assert!(!reservation.contains("TypedFreezeProof"));
    assert!(!startup.contains("pub(crate) struct StartupLedgerPlan<"));
    assert!(!startup.contains("fn try_qualify_startup<"));
    assert!(startup.contains("pub(crate) struct StartupLedgerPlanParts<"));
    assert!(startup.contains("pub(crate) fn preflight_startup_attribution_graph("));
    assert!(startup.contains("pub(crate) struct StartupAttributionGraph<"));
    assert!(!crate_root.contains("StartupAttributionGraph"));
    assert!(!startup.contains("ensure_phase("));

    let graph = startup
        .split("pub(crate) struct StartupAttributionGraph<")
        .nth(1)
        .expect("startup graph exists")
        .split("impl<O, P, D> Ledger<O, P, D>")
        .next()
        .expect("startup graph boundary exists");
    assert!(graph.contains("cores: StartupLedgerCores"));
    assert!(!graph.contains("AttributionReservationBatch"));
    assert_eq!(
        graph
            .matches("pub(crate) fn into_runtime_baseline(")
            .count(),
        1
    );
    assert!(graph.contains("_authority: ReadyStartupGraphSplitAuthority"));
    let graph_without_ready_split = graph.replacen(
        "pub(crate) fn into_runtime_baseline(",
        "fn into_runtime_baseline(",
        1,
    );
    assert_eq!(
        graph_without_ready_split.matches("pub(crate) fn ").count(),
        1,
        "the graph exposes only its actual-ledger identity check"
    );
    assert!(graph_without_ready_split.contains("pub(crate) fn belongs_to("));
    for forbidden in [
        "pub fn ",
        "pub(super)",
        "pub(in ",
        "fn accept",
        "fn into_parts",
    ] {
        assert!(
            !graph_without_ready_split.contains(forbidden),
            "startup graph exposes forbidden seam `{forbidden}`"
        );
    }
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
            .try_admit_startup_ledger(
                parts.reservation_specs(),
                &parts.account_shares,
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
            .try_admit_startup_ledger(
                parts.reservation_specs(),
                &parts.account_shares,
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
            .try_admit_startup_ledger(
                parts.reservation_specs(),
                &parts.account_shares,
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
            .try_admit_startup_ledger(
                parts.reservation_specs(),
                &parts.account_shares,
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
    assert_eq!(account_shares.steps.len(), 4);

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
fn startup_graph_commit_tail_is_infallible_and_has_no_dynamic_phase_path() {
    let startup = include_str!("../startup.rs");
    let commit = startup
        .split("fn commit_startup_ledger")
        .nth(1)
        .expect("startup commit helper exists")
        .split("fn empty_startup_batch")
        .next()
        .expect("startup commit boundary exists");
    for forbidden in [
        "Result<",
        "?",
        "checked_",
        "binary_search",
        "ensure_phase",
        "try_reserve",
    ] {
        assert!(
            !commit.contains(forbidden),
            "startup commit tail contains fallible operation `{forbidden}`"
        );
    }
    let admission = startup
        .split("fn try_admit_startup_ledger(")
        .nth(1)
        .expect("startup ledger admission exists")
        .split("fn bind_startup_ledger(")
        .next()
        .expect("startup admission boundary exists");
    assert_eq!(admission.matches("preflight_startup_ledger(").count(), 1);
    assert_eq!(admission.matches("bind_startup_ledger(").count(), 1);
    assert!(admission.contains("commit_startup_ledger"));
    assert!(!admission.contains("try_reserve"));
    assert!(!admission.contains("ensure_phase"));
    let preflight = startup
        .split("pub(crate) fn preflight_startup_attribution_graph(")
        .nth(1)
        .expect("read-only startup graph preflight exists")
        .split("pub(crate) fn try_bind_startup_attribution_graph(")
        .next()
        .expect("read-only preflight boundary exists");
    assert_eq!(preflight.matches("bind_startup_ledger(").count(), 1);
    assert_eq!(preflight.matches("preflight_startup_ledger(").count(), 1);
    assert!(!preflight.contains("commit_startup_ledger"));
    assert!(!admission.contains("preflight_startup_attribution_graph"));
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
