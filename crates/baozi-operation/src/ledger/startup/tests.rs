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
fn startup_qualification_has_one_production_module() {
    let ledger_root = include_str!("../../ledger.rs");
    let startup = include_str!("../startup.rs");
    let crate_root = include_str!("../../lib.rs");

    assert!(ledger_root.contains("mod startup;"));
    assert!(ledger_root.contains("pub(crate) use startup::{"));
    assert!(!ledger_root.contains("pub(crate) struct StartupLedgerPlan<"));
    assert!(startup.contains("pub(crate) struct StartupLedgerPlan<"));
    assert!(startup.contains("pub(crate) fn try_qualify_startup<"));
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
    for forbidden in ["pub fn ", "pub(", "fn accept", "fn into_parts"] {
        assert!(
            !graph.contains(forbidden),
            "provisional graph exposes forbidden seam `{forbidden}`"
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

fn qualify_graph_from_batches(
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
    let qualified = graph_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .try_qualify_startup(
            batches,
            retained,
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        )
        .expect("startup graph qualification");
    let (limits, parts) = qualified.into_parts();
    (limits, parts, covers)
}

fn qualify_graph(
    phases: [Phase; 4],
    dimensions: [Dimension; 4],
    retained: [(Phase, Dimension, u64); 2],
) -> (
    LedgerLimits<Dimension>,
    StartupLedgerPlanParts<Owner, Phase, Dimension>,
    [u64; 4],
) {
    qualify_graph_from_batches(
        graph_batch_plans(phases, dimensions),
        retained.map(|(phase, dimension, bytes)| {
            StartupRetainedAttribution::new(phase, dimension, bytes)
        }),
    )
}

fn default_qualified_graph() -> (
    LedgerLimits<Dimension>,
    StartupLedgerPlanParts<Owner, Phase, Dimension>,
    [u64; 4],
) {
    qualify_graph(
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
    let probe = ledger.test_probe();
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
        probe.current_live_bytes(),
        probe.active_reservations(),
        bytes.current_live_bytes(),
        records.current_live_bytes(),
        scratch.current_live_bytes(),
        records.attempted(),
        records.consumed(),
        bytes.attempted(),
    )
}

#[test]
fn startup_qualification_preserves_the_exact_typed_envelope() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let batch_bytes = startup_batch_bytes(&batches);
    let retained_attributions = startup_attributions(Dimension::Bytes, Dimension::Bytes);
    let total_live = batch_bytes.checked_add(8).expect("test live envelope");
    let limits = startup_limits(total_live, total_live, 12, 12, 10);

    let qualified = limits
        .try_qualify_startup(
            batches,
            retained_attributions,
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        )
        .expect("the exact startup envelope must qualify");
    let (limits, parts) = qualified.into_parts();

    assert_eq!(startup_batch_bytes(&parts.batches), batch_bytes);
    assert_eq!(parts.retained_attributions, retained_attributions);
    let (phase, dimension, bytes) = parts.retained_attributions[0].into_parts();
    assert_eq!(
        (phase, dimension, bytes),
        (Phase::Build, Dimension::Bytes, 3)
    );
    let (_canonical, _evidence_assembly, _fallback, _evidence) = parts.batches.into_parts();
    for share in parts.account_shares.into_steps() {
        assert_eq!(share.into_charges(), startup_share_charges());
    }
    assert_eq!(parts.commitments[0].dimension(), Dimension::Records);
    assert_eq!(parts.commitments[0].amount(), 4);
    assert_eq!(parts.commitments[1].dimension(), Dimension::Records);
    assert_eq!(parts.commitments[1].amount(), 6);
    assert_eq!(limits.dimension_count(), 2);
}

#[test]
fn startup_live_and_observed_work_are_independent_on_the_same_dimension() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let limits = startup_limits(total_live, total_live, 12, 12, 10);
    let account_shares =
        StartupAccountShareEnvelope::four_identical([(Dimension::Bytes, 1), (Dimension::Bytes, 2)]);

    assert!(
        limits
            .try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                account_shares,
                startup_commitments(Dimension::Bytes, Dimension::Bytes),
            )
            .is_ok(),
        "live reservations and observed work use independent ledger counter classes"
    );
}

#[test]
fn startup_qualification_rejects_unknown_dimensions_without_allocation() {
    let allocations_before = test_attribution_batch_allocation_entries();

    assert!(matches!(
        startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
            startup_batch_plans(Dimension::Unknown),
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert!(matches!(
        startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
            startup_batch_plans(Dimension::Bytes),
            startup_attributions(Dimension::Unknown, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert!(matches!(
        startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
            startup_batch_plans(Dimension::Bytes),
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            StartupAccountShareEnvelope::four_identical([
                (Dimension::Unknown, 1),
                (Dimension::Records, 2),
            ]),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert!(matches!(
        startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
            startup_batch_plans(Dimension::Bytes),
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Unknown, Dimension::Records),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));

    assert_eq!(
        test_attribution_batch_allocation_entries(),
        allocations_before,
        "qualification rejection must not allocate a batch"
    );
}

#[test]
fn startup_qualification_checks_live_dimension_and_operation_limits() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let dimension_limited = startup_limits(
        u64::MAX,
        total_live.checked_sub(1).expect("nonzero envelope"),
        12,
        12,
        10,
    );
    assert!(matches!(
        dimension_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value,
            limit,
        }) if value == total_live && limit == total_live - 1
    ));

    let batches = startup_batch_plans(Dimension::Bytes);
    let operation_limited = startup_limits(
        total_live.checked_sub(1).expect("nonzero envelope"),
        total_live,
        12,
        12,
        10,
    );
    assert!(matches!(
        operation_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value,
            limit,
        }) if value == total_live && limit == total_live - 1
    ));

    let non_reservable = startup_limits(u64::MAX, u64::MAX, 12, 12, 10);
    assert!(matches!(
        non_reservable.try_qualify_startup(
            startup_batch_plans(Dimension::Records),
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::DimensionNotReservable {
            dimension: Dimension::Records,
        })
    ));

    let live_overflow = startup_limits(u64::MAX, u64::MAX, 12, 12, 10);
    assert!(matches!(
        live_overflow.try_qualify_startup(
            startup_batch_plans(Dimension::Bytes),
            [
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, u64::MAX,),
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, 0),
            ],
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
        })
    ));
}

#[test]
fn startup_qualification_aggregates_four_account_shares() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let attempted_limited = startup_limits(total_live, total_live, 11, 11, 10);
    assert!(matches!(
        attempted_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 12,
            limit: 11,
        })
    ));

    let batches = startup_batch_plans(Dimension::Bytes);
    let consumed_limited = startup_limits(total_live, total_live, 12, 11, 10);
    assert!(matches!(
        consumed_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 12,
            limit: 11,
        })
    ));

    let batches = startup_batch_plans(Dimension::Bytes);
    let share_overflow = startup_limits(total_live, total_live, u64::MAX, u64::MAX, 10);
    assert!(matches!(
        share_overflow.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            StartupAccountShareEnvelope::four_identical([
                (Dimension::Records, u64::MAX),
                (Dimension::Records, 0),
            ]),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
        })
    ));
}

#[test]
fn startup_qualification_checked_sums_outcome_evidence_commitments() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let evidence_limited = startup_limits(total_live, total_live, 12, 12, 9);
    assert!(matches!(
        evidence_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension: Dimension::Records,
            value: 10,
            limit: 9,
        })
    ));

    let batches = startup_batch_plans(Dimension::Bytes);
    let overflow_limits = startup_limits(total_live, total_live, 12, 12, u64::MAX);
    assert!(matches!(
        overflow_limits.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            [
                OutputCommitment::new(Dimension::Records, u64::MAX),
                OutputCommitment::new(Dimension::Records, 1),
            ],
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension: Dimension::Records,
        })
    ));
}

#[test]
fn startup_graph_owns_exact_envelopes_and_drops_storage_before_one_ledger_rollback() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, covers) = default_qualified_graph();
    let ledger = frozen_graph_ledger(limits);
    let total_live = covers
        .into_iter()
        .try_fold(8_u64, u64::checked_add)
        .expect("startup graph live bytes");

    let graph = ledger
        .try_admit_startup_attribution_graph(parts)
        .expect("startup graph admission");

    assert_eq!(test_startup_graph_allocation_entries(), 8);
    assert_eq!(ledger.test_probe().active_reservations(), 6);
    assert_eq!(ledger.test_probe().current_live_bytes(), total_live);
    assert_eq!(
        [
            graph.cores.canonical_assembly_cover.bytes,
            graph.cores.evidence_assembly_cover.bytes,
            graph.cores.fallback_cover.bytes,
            graph.cores.evidence_cover.bytes,
        ],
        covers
    );
    assert_eq!(
        [
            graph.cores.fallback_attribution.bytes,
            graph.cores.evidence_attribution.bytes,
        ],
        [3, 5]
    );
    for (actual, cover) in [
        batch_backing_bytes(&graph.canonical_assembly),
        batch_backing_bytes(&graph.evidence_assembly),
        batch_backing_bytes(&graph.fallback),
        batch_backing_bytes(&graph.evidence),
    ]
    .into_iter()
    .zip(covers)
    {
        assert!(actual <= cover);
    }
    assert_eq!(graph.canonical_assembly.commitments.len(), 0);
    assert_eq!(graph.evidence_assembly.commitments.len(), 0);
    assert_eq!(graph.fallback.commitments.len(), 1);
    assert_eq!(graph.evidence.commitments.len(), 1);
    assert_eq!(graph._account_shares.steps.len(), 4);

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
    assert_eq!(records.attempted(), 12);
    assert_eq!(records.consumed(), 12);
    assert_eq!(records.published_outcome_evidence(), 0);
}

#[test]
fn startup_graph_accounts_cross_dimension_and_phase_reservations_exactly() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, covers) = qualify_graph(
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
    let ledger = frozen_graph_ledger(limits);
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
        let (limits, parts, _) = qualify_graph_from_batches(
            batches,
            [
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, 3),
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, 5),
            ],
        );
        let ledger = frozen_graph_ledger(limits);
        let before = ledger_graph_observation(&ledger);

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
        .split("fn preflight_startup_ledger(")
        .next()
        .expect("startup admission boundary exists");
    assert!(admission.contains("preflight_startup_ledger"));
    assert!(admission.contains("commit_startup_ledger"));
    assert!(!admission.contains("try_reserve"));
    assert!(!admission.contains("ensure_phase"));
}

#[test]
fn startup_graph_rejects_unknown_phase_and_invalid_dimensions_without_a_prefix() {
    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_qualified_graph();
    let ledger = Ledger::<Owner, Phase, Dimension>::try_new(limits, &[])
        .expect("empty frozen catalog is valid for an unknown-phase fixture");
    let before = ledger_graph_observation(&ledger);
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::UnknownPhase
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = qualify_graph(
        [Phase::Unknown, Phase::Build, Phase::Build, Phase::Build],
        [Dimension::Bytes; 4],
        [
            (Phase::Build, Dimension::Bytes, 3),
            (Phase::Build, Dimension::Bytes, 5),
        ],
    );
    let ledger = frozen_graph_ledger(limits);
    let before = ledger_graph_observation(&ledger);
    assert!(matches!(
        ledger.try_admit_startup_attribution_graph(parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::UnknownPhase
        ))
    ));
    assert_eq!(ledger_graph_observation(&ledger), before);
    assert_eq!(test_startup_graph_allocation_entries(), 0);

    test_reset_startup_graph_probes(None);
    let (limits, mut parts, _) = default_qualified_graph();
    parts.batches.canonical_assembly.capacity_dimension = Dimension::Unknown;
    let ledger = frozen_graph_ledger(limits);
    let before = ledger_graph_observation(&ledger);
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
    let (limits, mut parts, _) = default_qualified_graph();
    parts.batches.canonical_assembly.capacity_dimension = Dimension::Records;
    let ledger = frozen_graph_ledger(limits);
    let before = ledger_graph_observation(&ledger);
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
    let (_, parts, covers) = default_qualified_graph();
    let total_live = covers
        .into_iter()
        .try_fold(8_u64, u64::checked_add)
        .expect("startup graph live bytes");
    let ledger = frozen_graph_ledger(graph_limits(
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
    let (_, parts, covers) = default_qualified_graph();
    let total_live = covers
        .into_iter()
        .try_fold(8_u64, u64::checked_add)
        .expect("startup graph live bytes");
    let ledger = frozen_graph_ledger(graph_limits(
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
    let (_, parts, _) = default_qualified_graph();
    let ledger = frozen_graph_ledger(graph_limits(u64::MAX, u64::MAX, u64::MAX, 11, 11));
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
    let (_, parts, _) = default_qualified_graph();
    let ledger = frozen_graph_ledger(graph_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, 11));
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
    let (_, parts, _) = default_qualified_graph();
    let ledger = frozen_graph_ledger(graph_limits_with_published(
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
    let (limits, parts, _) = default_qualified_graph();
    let ledger = frozen_graph_ledger(limits);
    ledger
        .charge_attempted(Dimension::Records, u64::MAX)
        .expect("maximum attempted baseline");
    ledger
        .charge_consumed(Dimension::Records, u64::MAX)
        .expect("maximum consumed baseline");
    let before = ledger_graph_observation(&ledger);
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
    let (limits, parts, _) = default_qualified_graph();
    let ledger = frozen_graph_ledger(limits);
    let live = ledger
        .reserve(Phase::Build, Dimension::Bytes, u64::MAX)
        .expect("maximum live baseline");
    let before = ledger_graph_observation(&ledger);
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
    let (limits, parts, _) = default_qualified_graph();
    let ledger = frozen_graph_ledger(limits);
    {
        let mut state = lock(&ledger.state);
        state.active_reservations = u64::MAX;
    }
    let before = ledger_graph_observation(&ledger);
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
    lock(&ledger.state).active_reservations = 0;

    test_reset_startup_graph_probes(None);
    let (limits, parts, _) = default_qualified_graph();
    let ledger = frozen_graph_ledger(limits);
    let build_position = phase_entry(&lock(&ledger.state).phases, Phase::Build)
        .expect("build phase")
        .state_position;
    {
        let mut state = lock(&ledger.state);
        state.phase_states[build_position].current_live_bytes = u64::MAX;
        state.phase_states[build_position].peak_live_bytes = u64::MAX;
    }
    let before_phase = ledger
        .phase_evidence(Phase::Build)
        .expect("build phase evidence");
    let before = ledger_graph_observation(&ledger);
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
        let mut state = lock(&ledger.state);
        state.phase_states[build_position].current_live_bytes = 0;
    }
}

#[test]
fn every_startup_graph_allocator_failure_drops_eight_backings_then_rolls_back_six_cores_once() {
    for failure_index in 0..8 {
        test_reset_startup_graph_probes(Some(failure_index));
        let (limits, parts, _) = default_qualified_graph();
        let ledger = frozen_graph_ledger(limits);

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
        assert_eq!(records.attempted(), 12);
        assert_eq!(records.consumed(), 12);
    }
}

#[test]
fn startup_graph_allocator_failpoints_are_thread_local() {
    let workers = (0..8)
        .map(|failure_index| {
            std::thread::spawn(move || {
                test_reset_startup_graph_probes(Some(failure_index));
                let (limits, parts, _) = default_qualified_graph();
                let ledger = frozen_graph_ledger(limits);

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
