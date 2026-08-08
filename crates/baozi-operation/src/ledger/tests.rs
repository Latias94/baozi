use super::*;
use crate::{
    ControlCause, OperationControl, OperationCoordinator, ProgressPhase, ProgressStorageError,
    ProgressUnit,
};
use std::sync::atomic::AtomicUsize;

fn production_source(source: &'static str) -> &'static str {
    let test_attribute = ["#[cfg(", "test)]"].concat();
    source
        .rsplit_once(&test_attribute)
        .map_or(source, |(production, _)| production)
}

fn ledger_root_source() -> &'static str {
    production_source(include_str!("../ledger.rs"))
}

fn ledger_snapshot_source() -> &'static str {
    production_source(include_str!("snapshot.rs"))
}

fn ledger_reservation_source() -> &'static str {
    production_source(include_str!("reservation.rs"))
}

fn ledger_commitment_source() -> &'static str {
    production_source(include_str!("commitment.rs"))
}

fn ledger_production_source() -> String {
    [
        ledger_root_source(),
        ledger_commitment_source(),
        ledger_reservation_source(),
        ledger_snapshot_source(),
        production_source(include_str!("startup.rs")),
    ]
    .join("\n")
}

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
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Progress {
    Work,
    Other,
}

impl ProgressPhase for Progress {
    fn order(self) -> u32 {
        match self {
            Self::Work => 0,
            Self::Other => 1,
        }
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    Work,
}

impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "work"
    }
}

fn limits() -> LedgerLimits<Dimension> {
    LedgerLimits::new(
        16,
        [
            (
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(16, 16, 16, 16),
            ),
            (Dimension::Records, DimensionPolicy::counter(16, 16, 16)),
        ],
    )
    .expect("test policy")
}

const COMPLETE_PHASES: &[Phase] = &[Phase::Build, Phase::Finalize, Phase::Unknown];

fn test_ledger(limits: LedgerLimits<Dimension>) -> Ledger<Owner, Phase, Dimension> {
    Ledger::try_new(limits, COMPLETE_PHASES).expect("complete test phase catalog")
}

#[test]
fn runtime_ledger_has_no_dynamic_phase_growth_rail() {
    let root = ledger_root_source();
    let production = ledger_production_source();
    assert!(!production.contains("PhaseCatalogState"));
    assert!(!production.contains("fn ensure_phase"));
    assert!(!production.contains("try_new_with_phase_catalog"));
    let lookup = root
        .split_once("fn phase_position")
        .expect("frozen phase lookup exists")
        .1
        .split_once("fn try_insert_phase_catalog_entry")
        .expect("frozen phase lookup is bounded")
        .0;
    assert!(lookup.contains(".iter()"));
    assert!(lookup.contains(".find("));
    assert!(lookup.contains("LedgerCause::UnknownPhase"));
    assert!(!lookup.contains("binary_search"));
    assert!(!lookup.contains("try_grow_evidence_vec"));
    assert!(!lookup.contains("push("));
    assert!(!lookup.contains("insert("));
}

#[test]
fn frozen_phase_catalog_rejects_duplicates_instead_of_rewriting_lifecycle_order() {
    let rejected = Ledger::<Owner, Phase, Dimension>::try_new(
        limits(),
        &[Phase::Build, Phase::Finalize, Phase::Build],
    );

    assert!(matches!(rejected, Err(LedgerCause::DuplicatePhase)));
}

#[test]
fn frozen_phase_catalog_prebuilds_complete_final_evidence_shape() {
    let ledger =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("frozen phase catalog");

    let state = lock(&ledger.state);
    assert_eq!(state.phases.len(), 2);
    assert_eq!(state.phase_states.len(), 2);
    assert_eq!(state.phase_snapshot_staging.len(), 2);
    assert_eq!(state.phases[0].phase, Phase::Build);
    assert_eq!(state.phases[1].phase, Phase::Finalize);
    assert_eq!(state.phase_snapshot_staging[0].0, Phase::Build);
    assert_eq!(state.phase_snapshot_staging[1].0, Phase::Finalize);
    drop(state);

    assert_eq!(
        ledger.phase_evidence(Phase::Build),
        Some(PhaseSnapshot::default())
    );
    assert_eq!(
        ledger.phase_evidence(Phase::Finalize),
        Some(PhaseSnapshot::default())
    );
}

#[test]
fn frozen_phase_catalog_rejects_unknown_phase_without_allocation_or_counter_mutation() {
    let ledger =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("frozen phase catalog");
    let before = {
        let state = lock(&ledger.state);
        (
            state.phases.len(),
            state.phases.capacity(),
            state.phase_states.len(),
            state.phase_states.capacity(),
            state.phase_snapshot_staging.len(),
            state.phase_snapshot_staging.capacity(),
            state.current_evidence_bytes,
            state.peak_evidence_bytes,
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            dimension_snapshot(state.dimensions[0].state),
        )
    };

    let rejected = ledger.reserve(Phase::Unknown, Dimension::Bytes, 1);

    assert!(matches!(rejected, Err(LedgerCause::UnknownPhase)));
    let after = {
        let state = lock(&ledger.state);
        (
            state.phases.len(),
            state.phases.capacity(),
            state.phase_states.len(),
            state.phase_states.capacity(),
            state.phase_snapshot_staging.len(),
            state.phase_snapshot_staging.capacity(),
            state.current_evidence_bytes,
            state.peak_evidence_bytes,
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            dimension_snapshot(state.dimensions[0].state),
        )
    };
    assert_eq!(after, before);
}

#[test]
fn aggregate_reservation_preflight_enforces_operation_headroom_without_mutation() {
    let limits = LedgerLimits::new(
        6,
        [(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(16, 16, 16, 16),
        )],
    )
    .expect("aggregate preflight policy");
    let ledger = Ledger::<Owner, Phase, Dimension>::try_new(limits, &[Phase::Build])
        .expect("aggregate preflight ledger");
    let existing = ledger
        .reserve(Phase::Build, Dimension::Bytes, 2)
        .expect("existing live reservation");
    let before = {
        let state = lock(&ledger.state);
        (
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            state.dimensions[0].state,
            state.phase_states[0].current_live_bytes,
            state.phase_states[0].peak_live_bytes,
        )
    };

    assert!(matches!(
        ledger.preflight_reservation_batch(
            Phase::Build,
            Dimension::Bytes,
            5,
            NonZeroU64::new(2).expect("nonzero batch"),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value: 7,
            limit: 6,
        })
    ));
    let after_rejection = {
        let state = lock(&ledger.state);
        (
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            state.dimensions[0].state,
            state.phase_states[0].current_live_bytes,
            state.phase_states[0].peak_live_bytes,
        )
    };
    assert_eq!(after_rejection, before);

    ledger
        .preflight_reservation_batch(
            Phase::Build,
            Dimension::Bytes,
            4,
            NonZeroU64::new(2).expect("nonzero retry batch"),
        )
        .expect("smaller aggregate remains retryable");
    let after_retry = {
        let state = lock(&ledger.state);
        (
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            state.dimensions[0].state,
            state.phase_states[0].current_live_bytes,
            state.phase_states[0].peak_live_bytes,
        )
    };
    assert_eq!(after_retry, before);

    let first = ledger
        .reserve(Phase::Build, Dimension::Bytes, 2)
        .expect("first preflighted item");
    let second = ledger
        .reserve(Phase::Build, Dimension::Bytes, 2)
        .expect("second preflighted item");
    assert_eq!(ledger.test_probe().current_live_bytes(), 6);
    drop((first, second, existing));
    assert_eq!(ledger.test_probe().current_live_bytes(), 0);
}

#[test]
fn frozen_phase_completion_is_monotonic_by_catalog_position_and_preserves_rejections() {
    let ledger =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Finalize, Phase::Build])
            .expect("frozen phase catalog");

    let initial = ledger.try_snapshot().expect("initial snapshot fits");
    assert_eq!(initial.last_completed_phase(), None);
    assert_eq!(
        initial.phases().map(|(phase, _)| phase).collect::<Vec<_>>(),
        vec![Phase::Finalize, Phase::Build],
        "the caller-declared catalog order is the operation lifecycle"
    );

    ledger
        .try_complete_phase(Phase::Finalize)
        .expect("first catalog phase completes");
    ledger
        .try_complete_phase(Phase::Finalize)
        .expect("repeating the completed phase is idempotent");
    assert_eq!(
        ledger
            .try_snapshot()
            .expect("idempotent snapshot fits")
            .last_completed_phase(),
        Some(Phase::Finalize)
    );

    ledger
        .try_complete_phase(Phase::Build)
        .expect("later catalog position completes even when its key sorts earlier");
    assert_eq!(
        ledger.try_complete_phase(Phase::Finalize),
        Err(PhaseCompletionError::Regression {
            last_completed_phase: Phase::Build,
            phase: Phase::Finalize,
        })
    );
    assert_eq!(
        ledger.try_complete_phase(Phase::Unknown),
        Err(PhaseCompletionError::UnknownPhase {
            phase: Phase::Unknown,
        })
    );
    assert_eq!(
        ledger
            .try_snapshot()
            .expect("rejection-preserving snapshot fits")
            .last_completed_phase(),
        Some(Phase::Build)
    );
}

#[test]
fn phase_completion_obeys_pending_rollback_and_published_boundaries() {
    let ledger =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("frozen phase catalog");
    let mut first_batch = PublicationReservationBatch::default();
    let pending = ledger
        .prepare_publication(&mut first_batch, &[])
        .expect("empty publication covers zero live bytes");

    assert_eq!(
        ledger.try_complete_phase(Phase::Build),
        Err(PhaseCompletionError::PublicationPending)
    );
    assert_eq!(
        ledger
            .try_snapshot()
            .expect("pending snapshot fits")
            .last_completed_phase(),
        None
    );

    pending.rollback();
    ledger
        .try_complete_phase(Phase::Build)
        .expect("rollback reopens semantic completion");

    let mut replacement_batch = PublicationReservationBatch::default();
    let mut published = ledger
        .prepare_publication(&mut replacement_batch, &[])
        .expect("replacement publication");
    ledger
        .publish(&mut published)
        .expect("publication becomes durable");
    assert_eq!(
        ledger.try_complete_phase(Phase::Finalize),
        Err(PhaseCompletionError::PublicationCompleted)
    );
    assert_eq!(
        ledger
            .try_snapshot()
            .expect("published snapshot fits")
            .last_completed_phase(),
        Some(Phase::Build)
    );
}

fn phase_shape_probe(
    ledger: &Ledger<Owner, Phase, Dimension>,
) -> (Vec<usize>, Option<(Phase, usize)>, u64, u64) {
    let state = lock(&ledger.state);
    (
        state
            .phases
            .iter()
            .map(|entry| entry.state_position)
            .collect(),
        state
            .last_completed_phase
            .map(|completed| (completed.phase, completed.state_position)),
        state.current_evidence_bytes,
        state.peak_evidence_bytes,
    )
}

#[test]
fn phase_catalog_shape_faults_reject_completion_and_snapshot_without_mutation() {
    let duplicate =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("duplicate fault ledger");
    {
        let mut state = lock(&duplicate.state);
        state.phases[1].state_position = state.phases[0].state_position;
    }
    let duplicate_before = phase_shape_probe(&duplicate);
    assert_eq!(
        duplicate.try_complete_phase(Phase::Build),
        Err(PhaseCompletionError::CatalogShapeMismatch)
    );
    assert_eq!(
        duplicate.try_snapshot(),
        Err(LedgerEvidenceError::PhaseCatalogShapeMismatch)
    );
    assert_eq!(phase_shape_probe(&duplicate), duplicate_before);

    let out_of_range =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("out-of-range fault ledger");
    out_of_range
        .try_complete_phase(Phase::Build)
        .expect("build completes before fault injection");
    {
        let mut state = lock(&out_of_range.state);
        state.phases[1].state_position = state.phase_states.len();
    }
    let out_of_range_before = phase_shape_probe(&out_of_range);
    assert_eq!(
        out_of_range.try_complete_phase(Phase::Build),
        Err(PhaseCompletionError::CatalogShapeMismatch),
        "idempotent completion still validates the whole catalog"
    );
    assert_eq!(
        out_of_range.try_complete_phase(Phase::Finalize),
        Err(PhaseCompletionError::CatalogShapeMismatch),
        "forward completion still validates the whole catalog"
    );
    assert_eq!(
        out_of_range.try_snapshot(),
        Err(LedgerEvidenceError::PhaseCatalogShapeMismatch)
    );
    assert_eq!(phase_shape_probe(&out_of_range), out_of_range_before);

    let missing_last =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("missing-last fault ledger");
    missing_last
        .try_complete_phase(Phase::Finalize)
        .expect("finalize completes before fault injection");
    {
        let mut state = lock(&missing_last.state);
        state
            .last_completed_phase
            .as_mut()
            .expect("completed phase")
            .state_position = state.phase_states.len();
    }
    let missing_last_before = phase_shape_probe(&missing_last);
    assert_eq!(
        missing_last.try_complete_phase(Phase::Build),
        Err(PhaseCompletionError::CatalogShapeMismatch)
    );
    assert_eq!(
        missing_last.try_snapshot(),
        Err(LedgerEvidenceError::PhaseCatalogShapeMismatch)
    );
    assert_eq!(phase_shape_probe(&missing_last), missing_last_before);
}

#[test]
fn final_shape_failure_preserves_the_stored_completed_phase_key() {
    let ledger =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("frozen phase catalog");
    ledger
        .try_complete_phase(Phase::Finalize)
        .expect("finalize completes before fault injection");
    {
        let mut state = lock(&ledger.state);
        state
            .last_completed_phase
            .as_mut()
            .expect("completed phase")
            .state_position = state.phase_states.len();
    }

    let error = ledger
        .finish()
        .expect_err("invalid completed pair rejects finalization");

    assert_eq!(error.cause(), &LedgerCause::FinalSnapshotShapeMismatch);
    assert_eq!(
        error.snapshot().last_completed_phase(),
        Some(Phase::Finalize)
    );
}

#[test]
fn frozen_dimension_count_matches_the_canonical_dimension_vocabulary() {
    let limits = limits();

    assert_eq!(limits.dimension_count(), 2);
    assert!(limits.dimension(Dimension::Bytes).is_some());
    assert!(limits.dimension(Dimension::Records).is_some());
}

#[test]
fn consumed_batch_is_atomic_and_aggregates_duplicate_dimensions() {
    let ledger = test_ledger(limits());
    ledger
        .charge_attempted(Dimension::Bytes, 2)
        .expect("byte work must be admitted");
    ledger
        .charge_attempted(Dimension::Records, 1)
        .expect("record work must be admitted");

    assert!(matches!(
        ledger.charge_consumed_batch(&[
            (Dimension::Bytes, 1),
            (Dimension::Bytes, 1),
            (Dimension::Records, 2),
        ]),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: Dimension::Records,
            consumed: 2,
            attempted: 1,
        })
    ));
    assert_eq!(
        ledger
            .dimension_evidence(Dimension::Bytes)
            .expect("byte evidence")
            .consumed(),
        0,
        "a later batch failure must not retain an earlier charge"
    );
    assert_eq!(
        ledger
            .dimension_evidence(Dimension::Records)
            .expect("record evidence")
            .consumed(),
        0
    );

    assert!(matches!(
        ledger.charge_consumed_batch(&[(Dimension::Records, 1), (Dimension::Bytes, 1),]),
        Err(LedgerCause::ConsumedBatchOutOfOrder {
            previous: Dimension::Records,
            dimension: Dimension::Bytes,
        })
    ));

    ledger
        .charge_consumed_batch(&[
            (Dimension::Bytes, 1),
            (Dimension::Bytes, 1),
            (Dimension::Records, 1),
        ])
        .expect("the admitted batch must commit");
    assert_eq!(
        ledger
            .dimension_evidence(Dimension::Bytes)
            .expect("byte evidence")
            .consumed(),
        2
    );
    assert_eq!(
        ledger
            .dimension_evidence(Dimension::Records)
            .expect("record evidence")
            .consumed(),
        1
    );
}

#[cfg(feature = "_u4-activation")]
#[test]
fn linear_work_admission_records_attempted_before_consumed_completion() {
    let ledger = test_ledger(limits());

    let mut permit = ledger
        .try_admit_linear_work(Dimension::Records, 3)
        .expect("three linear work units fit both counters");
    let admitted = ledger
        .dimension_evidence(Dimension::Records)
        .expect("admitted work evidence");
    assert_eq!((admitted.attempted(), admitted.consumed()), (3, 0));

    permit
        .complete(0, 2)
        .expect("the completed prefix fits the admitted envelope");
    let prefix = ledger
        .dimension_evidence(Dimension::Records)
        .expect("completed-prefix evidence");
    assert_eq!((prefix.attempted(), prefix.consumed()), (3, 2));

    permit
        .complete(0, 1)
        .expect("the exact completed suffix fits the admitted envelope");
    let complete = ledger
        .dimension_evidence(Dimension::Records)
        .expect("completed linear work evidence");
    assert_eq!((complete.attempted(), complete.consumed()), (3, 3));
}

#[cfg(feature = "_u4-activation")]
#[test]
fn linear_work_admission_preserves_attempted_when_consumed_capacity_is_short() {
    let constrained = LedgerLimits::new(
        16,
        [(Dimension::Records, DimensionPolicy::counter(4, 2, 0))],
    )
    .expect("linear work limits");
    let ledger = test_ledger(constrained);

    assert!(matches!(
        ledger.try_admit_linear_work(Dimension::Records, 3),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 3,
            limit: 2,
        })
    ));
    let rejected = ledger
        .dimension_evidence(Dimension::Records)
        .expect("rejected admission evidence");
    assert_eq!(
        (rejected.attempted(), rejected.consumed()),
        (3, 0),
        "consumed preflight rejection must retain attempted-only evidence"
    );
}

#[test]
fn linear_work_batch_aggregates_one_dimension_before_recording_attempted() {
    let constrained = LedgerLimits::new(
        16,
        [(Dimension::Records, DimensionPolicy::counter(4, 1, 0))],
    )
    .expect("linear batch limits");
    let ledger = test_ledger(constrained);

    assert!(matches!(
        ledger.try_admit_linear_work_batch(&[(Dimension::Records, 1), (Dimension::Records, 1),]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 2,
            limit: 1,
        })
    ));
    let rejected = ledger
        .dimension_evidence(Dimension::Records)
        .expect("rejected batch evidence");
    assert_eq!(
        (rejected.attempted(), rejected.consumed()),
        (2, 0),
        "a rejected batch retains its exact attempted envelope without fabricating completion"
    );
}

#[test]
fn linear_work_batch_aggregate_overflow_is_side_effect_free() {
    let ledger = test_ledger(limits());

    assert!(matches!(
        ledger.try_admit_linear_work_batch(&[(Dimension::Bytes, u64::MAX), (Dimension::Bytes, 1),]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
        })
    ));
    let unchanged = ledger
        .dimension_evidence(Dimension::Bytes)
        .expect("overflow dimension evidence");
    assert_eq!((unchanged.attempted(), unchanged.consumed()), (0, 0));
}

#[test]
fn linear_work_pending_capacity_prevents_oversell_and_drop_releases_the_suffix() {
    let constrained = LedgerLimits::new(
        16,
        [(Dimension::Records, DimensionPolicy::counter(8, 2, 0))],
    )
    .expect("pending-work limits");
    let ledger = test_ledger(constrained);
    let first = ledger
        .try_admit_linear_work(Dimension::Records, 2)
        .expect("the first permit reserves the complete consumed envelope");

    assert!(matches!(
        ledger.try_admit_linear_work(Dimension::Records, 1),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 3,
            limit: 2,
        })
    ));
    assert!(matches!(
        ledger.charge_consumed(Dimension::Records, 1),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 3,
            limit: 2,
        })
    ));
    assert!(matches!(
        ledger.charge_consumed_batch(&[(Dimension::Records, 1)]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 3,
            limit: 2,
        })
    ));
    assert!(matches!(
        ledger.preflight_repeated_observed_batch(&[(Dimension::Records, 1)], NonZeroU64::MIN,),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 3,
            limit: 2,
        })
    ));
    let reserved = ledger
        .dimension_evidence(Dimension::Records)
        .expect("pending-work evidence");
    assert_eq!(
        (reserved.attempted(), reserved.consumed()),
        (3, 0),
        "neither a second admission nor an ordinary charge may consume reserved capacity"
    );
    {
        let state = lock(&ledger.state);
        let entry = dimension_entry(&state.dimensions, Dimension::Records)
            .expect("record dimension remains frozen");
        assert_eq!(entry.state.pending_consumed, 2);
        assert_eq!(state.active_linear_work_permits, 1);
    }

    drop(first);
    {
        let state = lock(&ledger.state);
        let entry = dimension_entry(&state.dimensions, Dimension::Records)
            .expect("record dimension remains frozen");
        assert_eq!(entry.state.pending_consumed, 0);
        assert_eq!(state.active_linear_work_permits, 0);
    }
    ledger
        .charge_consumed(Dimension::Records, 1)
        .expect("dropping the unfinished permit releases its pending suffix");
}

#[test]
fn forgotten_linear_work_permit_blocks_prepare_and_final_snapshot() {
    let ledger = test_ledger(limits());
    let permit = ledger
        .try_admit_linear_work(Dimension::Records, 1)
        .expect("one work unit is admitted");
    std::mem::forget(permit);

    let mut reservations = PublicationReservationBatch::default();
    assert!(matches!(
        ledger.prepare_publication(&mut reservations, &[]),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    ));
    let error = ledger
        .finish()
        .expect_err("forgotten pending consumed capacity must fail closed");
    assert_eq!(
        error.cause(),
        &LedgerCause::OutstandingLinearWorkPermits { count: 1 }
    );
    let evidence = error
        .snapshot()
        .dimension(Dimension::Records)
        .expect("final failure retains record evidence");
    assert_eq!((evidence.attempted(), evidence.consumed()), (1, 0));
}

#[test]
fn linear_work_release_fault_rejects_final_snapshot_even_without_an_active_permit() {
    let ledger = test_ledger(limits());
    let permit = ledger
        .try_admit_linear_work(Dimension::Records, 1)
        .expect("one work unit is admitted");
    {
        let mut state = lock(&ledger.state);
        let entry = dimension_entry_mut(&mut state.dimensions, Dimension::Records)
            .expect("record dimension remains frozen");
        entry.state.pending_consumed = 0;
    }
    drop(permit);

    let error = ledger
        .finish()
        .expect_err("a corrupted permit release must fault final evidence");
    assert_eq!(error.cause(), &LedgerCause::FinalSnapshotShapeMismatch);
}

#[test]
fn observed_batch_preserves_the_ordered_non_refundable_prefix_on_failure() {
    let ledger = test_ledger(limits());
    ledger
        .charge_attempted(Dimension::Records, 16)
        .expect("record attempted budget must reach its exact limit");

    assert!(matches!(
        ledger.charge_observed_batch(&[(Dimension::Bytes, 1), (Dimension::Records, 1)]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 17,
            limit: 16,
        })
    ));
    let bytes = ledger
        .dimension_evidence(Dimension::Bytes)
        .expect("byte evidence");
    assert_eq!(bytes.attempted(), 1);
    assert_eq!(bytes.consumed(), 1);
    let records = ledger
        .dimension_evidence(Dimension::Records)
        .expect("record evidence");
    assert_eq!(records.attempted(), 17);
    assert_eq!(records.consumed(), 0);
}

#[test]
fn repeated_observed_preflight_aggregates_duplicate_dimensions_without_mutation() {
    let ledger = test_ledger(limits());
    let exact = NonZeroU64::new(8).expect("exact repetition count");

    ledger
        .preflight_repeated_observed_batch(&[(Dimension::Bytes, 1), (Dimension::Bytes, 1)], exact)
        .expect("two charges repeated eight times fit the exact limit");
    let unchanged = ledger
        .dimension_evidence(Dimension::Bytes)
        .expect("byte evidence");
    assert_eq!(unchanged.attempted(), 0);
    assert_eq!(unchanged.consumed(), 0);

    ledger
        .preflight_repeated_observed_batch(
            &[(Dimension::Bytes, 0), (Dimension::Bytes, 0)],
            NonZeroU64::new(u64::MAX).expect("maximum repetition count"),
        )
        .expect("zero work remains zero for every nonzero repetition count");
    let zero_unchanged = ledger
        .dimension_evidence(Dimension::Bytes)
        .expect("byte evidence after zero-work preflight");
    assert_eq!(zero_unchanged.attempted(), 0);
    assert_eq!(zero_unchanged.consumed(), 0);

    let excess = NonZeroU64::new(9).expect("excess repetition count");
    assert!(matches!(
        ledger.preflight_repeated_observed_batch(
            &[(Dimension::Bytes, 1), (Dimension::Bytes, 1)],
            excess,
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
            value: 18,
            limit: 16,
        })
    ));
    let still_unchanged = ledger
        .dimension_evidence(Dimension::Bytes)
        .expect("byte evidence after rejection");
    assert_eq!(still_unchanged.attempted(), 0);
    assert_eq!(still_unchanged.consumed(), 0);
}

#[test]
fn repeated_observed_preflight_binds_distinct_dimensions_and_checks_arithmetic() {
    let ledger = test_ledger(limits());
    ledger
        .preflight_repeated_observed_batch(
            &[(Dimension::Bytes, 1), (Dimension::Records, 2)],
            NonZeroU64::new(8).expect("repetition count"),
        )
        .expect("distinct dimensions fit their exact limits");

    assert!(matches!(
        ledger.preflight_repeated_observed_batch(
            &[(Dimension::Unknown, 1), (Dimension::Bytes, 1)],
            NonZeroU64::new(1).expect("repetition count"),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert!(matches!(
        ledger.preflight_repeated_observed_batch(
            &[(Dimension::Bytes, u64::MAX), (Dimension::Records, 1)],
            NonZeroU64::new(2).expect("repetition count"),
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
        })
    ));
    assert!(matches!(
        ledger.preflight_repeated_observed_batch(
            &[(Dimension::Bytes, u64::MAX), (Dimension::Bytes, 1)],
            NonZeroU64::new(1).expect("repetition count"),
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
        })
    ));
    for dimension in [Dimension::Bytes, Dimension::Records] {
        let evidence = ledger
            .dimension_evidence(dimension)
            .expect("preflight dimension evidence");
        assert_eq!(evidence.attempted(), 0);
        assert_eq!(evidence.consumed(), 0);
    }
}

#[test]
fn dimension_policy_tightening_is_monotonic_and_preserves_existing_evidence() {
    let ledger = test_ledger(limits());
    ledger
        .tighten_dimension_policy(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(8, 8, 8, 8),
        )
        .expect("the initial policy must tighten");
    ledger
        .tighten_dimension_policy(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(12, 12, 12, 12),
        )
        .expect("a numerical relaxation request must preserve the tighter policy");
    ledger
        .charge_attempted(Dimension::Bytes, 8)
        .expect("the tightened attempted limit remains usable");
    assert!(matches!(
        ledger.charge_attempted(Dimension::Bytes, 1),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
            value: 9,
            limit: 8,
        })
    ));

    let reservation = ledger
        .reserve(Phase::Build, Dimension::Bytes, 4)
        .expect("live state below the tightened bound must remain admitted");
    let before = ledger
        .dimension_evidence(Dimension::Bytes)
        .expect("dimension evidence must remain available");
    assert!(matches!(
        ledger.tighten_dimension_policy(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(7, 7, 7, 3),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
            value: 9,
            limit: 7,
        })
    ));
    assert_eq!(
        ledger
            .dimension_evidence(Dimension::Bytes)
            .expect("failed admission must preserve exact evidence"),
        before,
        "policy tightening must not fabricate a second ledger or rewrite prior evidence"
    );
    assert!(matches!(
        ledger.charge_attempted(Dimension::Bytes, 0),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Bytes,
            value: 9,
            limit: 7,
        })
    ));
    assert!(matches!(
        ledger.tighten_dimension_policy(Dimension::Bytes, DimensionPolicy::counter(7, 7, 7)),
        Err(LedgerCause::DimensionPolicyShapeMismatch {
            dimension: Dimension::Bytes,
        })
    ));
    drop(reservation);
}

#[test]
fn snapshot_failure_cleanup_is_structurally_storage_before_accounting() {
    let source = ledger_snapshot_source();
    let cleanup = source
        .split_once("impl<P, D> Drop for SnapshotEvidenceAssembly")
        .expect("snapshot evidence cleanup owner exists")
        .1
        .split_once("fn try_snapshot")
        .expect("snapshot cleanup precedes snapshot construction")
        .0;
    let phase_storage = cleanup
        .find("drop(std::mem::take(&mut self.phases))")
        .expect("phase snapshot storage is destroyed explicitly");
    let dimension_storage = cleanup
        .find("drop(std::mem::take(&mut self.dimensions))")
        .expect("dimension snapshot storage is destroyed explicitly");
    let accounting_release = cleanup
        .find("release_evidence(self.state, outgoing_bytes)")
        .expect("snapshot evidence accounting is released explicitly");
    assert!(phase_storage < accounting_release);
    assert!(dimension_storage < accounting_release);

    let snapshot = source
        .split_once("fn try_snapshot<P, D>")
        .expect("snapshot constructor exists")
        .1
        .split_once("fn validate_final_snapshot_shape")
        .expect("snapshot constructor has a bounded body")
        .0;
    assert!(snapshot.contains("SnapshotEvidenceAssembly::new(state)"));
    assert!(!snapshot.contains("capacity_bytes("));
}

#[test]
fn phase_snapshot_admission_failure_restores_exact_evidence_accounting() {
    let ledger = test_ledger(limits());
    let (evidence_before, peak_before, original_limit) = {
        let mut state = lock(&ledger.state);
        let dimension_plan =
            CapacityPlan::for_type::<(Dimension, DimensionSnapshot)>(0, 0, state.dimensions.len())
                .expect("dimension snapshot capacity plan");
        let dimension_envelope = dimension_plan
            .envelope_delta_bytes()
            .expect("dimension snapshot envelope");
        let evidence_before = state.current_evidence_bytes;
        let peak_before = state.peak_evidence_bytes;
        let original_limit = state.evidence_byte_limit;
        state.evidence_byte_limit = evidence_before
            .checked_add(dimension_envelope)
            .expect("dimension snapshot fits the test limit");
        (evidence_before, peak_before, original_limit)
    };

    let error = ledger
        .try_snapshot()
        .expect_err("phase snapshot storage must exceed the tightened limit");
    assert!(matches!(
        error,
        LedgerEvidenceError::LimitExceeded {
            purpose: LedgerEvidencePurpose::Snapshot,
            ..
        }
    ));

    {
        let mut state = lock(&ledger.state);
        assert_eq!(
            state.current_evidence_bytes, evidence_before,
            "failed snapshot storage must release its complete local accounting"
        );
        assert!(
            state.peak_evidence_bytes >= peak_before,
            "failed transient storage remains visible in peak evidence"
        );
        state.evidence_byte_limit = original_limit;
    }
    assert_eq!(
        ledger
            .try_snapshot()
            .expect("a later snapshot remains usable")
            .current_evidence_bytes(),
        evidence_before
    );
}

#[test]
fn injected_allocator_failure_releases_current_evidence_but_preserves_peak() {
    let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
        .lock()
        .expect("allocation test lock");
    let mut values = Vec::<u64>::new();
    let mut current = 0;
    let mut peak = 0;
    test_set_evidence_allocation_failure(true);
    let error = try_grow_evidence_vec(
        &mut values,
        1,
        &mut current,
        &mut peak,
        1024,
        LedgerEvidencePurpose::Snapshot,
    )
    .expect_err("injected allocation failure");
    assert!(matches!(
        error,
        LedgerEvidenceError::AllocationFailed {
            purpose: LedgerEvidencePurpose::Snapshot,
            requested_bytes: 8,
        }
    ));
    assert_eq!(values.capacity(), 0);
    assert_eq!(current, 0);
    assert_eq!(peak, 8);
}

#[test]
fn injected_over_envelope_failure_preserves_non_empty_local_vec_and_accounting() {
    let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
        .lock()
        .expect("allocation test lock");
    let mut values = Vec::with_capacity(2);
    values.extend([11_u64, 22]);
    let original_values = values.clone();
    let original_len = values.len();
    let original_capacity = values.capacity();
    let original_bytes = capacity_bytes(&values).expect("test capacity bytes");
    let mut current = original_bytes;
    let mut peak = original_bytes;

    test_fail_next_evidence_over_envelope();
    let error = try_grow_evidence_vec(
        &mut values,
        original_capacity.saturating_add(1),
        &mut current,
        &mut peak,
        u64::MAX,
        LedgerEvidencePurpose::Snapshot,
    )
    .expect_err("injected post-allocation envelope violation");

    assert!(matches!(
        error,
        LedgerEvidenceError::Capacity {
            purpose: LedgerEvidencePurpose::Snapshot,
            cause: CapacityPlanError::CapacityEnvelopeViolated { .. },
        }
    ));
    assert_eq!(values, original_values);
    assert_eq!(values.len(), original_len);
    assert_eq!(values.capacity(), original_capacity);
    assert_eq!(current, original_bytes);
    assert!(peak > original_bytes);
}

#[test]
fn replacement_growth_accounts_the_complete_transient_candidate() {
    let mut values = Vec::with_capacity(2);
    values.extend([11_u64, 22]);
    let original_values = values.clone();
    let original_bytes = capacity_bytes(&values).expect("test capacity bytes");
    let desired_capacity = values.capacity().saturating_add(1);
    let replacement_plan =
        CapacityPlan::for_type::<u64>(0, 0, desired_capacity).expect("replacement capacity plan");
    let replacement_envelope_bytes = replacement_plan
        .envelope_delta_bytes()
        .expect("replacement envelope bytes");
    let mut current = original_bytes;
    let mut peak = original_bytes;

    try_grow_evidence_vec(
        &mut values,
        desired_capacity,
        &mut current,
        &mut peak,
        u64::MAX,
        LedgerEvidencePurpose::Snapshot,
    )
    .expect("replacement growth");

    assert_eq!(values, original_values);
    assert!(values.capacity() >= desired_capacity);
    assert_eq!(
        current,
        capacity_bytes(&values).expect("replacement capacity bytes")
    );
    assert_eq!(
        peak,
        original_bytes + replacement_envelope_bytes,
        "the old backing and complete candidate envelope overlap at allocation time"
    );
}

#[test]
fn tracked_coverage_mismatch_rejects_before_allocator_or_accounting_mutation() {
    let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
        .lock()
        .expect("allocation test lock");
    let mut values = Vec::with_capacity(2);
    values.extend([11_u64, 22]);
    let original_values = values.clone();
    let original_capacity = values.capacity();
    let owned_capacity_bytes = capacity_bytes(&values).expect("test capacity bytes");
    let mut retained_bytes = 0;
    let mut current = owned_capacity_bytes;
    let mut peak = owned_capacity_bytes;

    test_set_evidence_allocation_failure(true);
    let error = try_grow_tracked_evidence_vec(
        &mut values,
        original_capacity.saturating_add(1),
        &mut retained_bytes,
        &mut current,
        &mut peak,
        u64::MAX,
        LedgerEvidencePurpose::Progress,
    )
    .expect_err("mismatched retained owner must reject");

    assert_eq!(
        error,
        LedgerEvidenceError::TrackedStorageCoverageMismatch {
            purpose: LedgerEvidencePurpose::Progress,
            retained_bytes: 0,
            capacity_bytes: owned_capacity_bytes,
        }
    );
    assert_eq!(values, original_values);
    assert_eq!(values.capacity(), original_capacity);
    assert_eq!(retained_bytes, 0);
    assert_eq!(current, owned_capacity_bytes);
    assert_eq!(peak, owned_capacity_bytes);
    assert!(
        test_evidence_allocation_failure_is_armed(),
        "coverage rejection must precede the allocator failpoint"
    );
    test_set_evidence_allocation_failure(false);
}

#[test]
fn zero_sized_tracked_storage_needs_no_candidate_or_evidence_bytes() {
    let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
        .lock()
        .expect("allocation test lock");
    let mut values = vec![(), ()];
    let original_capacity = values.capacity();
    let mut retained_bytes = 0;
    let mut current = 0;
    let mut peak = 0;

    test_set_evidence_allocation_failure(true);
    try_grow_tracked_evidence_vec(
        &mut values,
        usize::MAX,
        &mut retained_bytes,
        &mut current,
        &mut peak,
        0,
        LedgerEvidencePurpose::Progress,
    )
    .expect("zero-sized storage already has logical capacity");

    assert_eq!(values, [(), ()]);
    assert_eq!(values.capacity(), original_capacity);
    assert_eq!(retained_bytes, 0);
    assert_eq!(current, 0);
    assert_eq!(peak, 0);
    assert!(
        test_evidence_allocation_failure_is_armed(),
        "zero-sized growth must not create a replacement backing"
    );
    test_set_evidence_allocation_failure(false);
}

#[test]
fn injected_progress_allocation_failure_is_atomic_and_latched() {
    let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
        .lock()
        .expect("allocation test lock");
    let limits = limits();
    let mut coordinator = OperationCoordinator::<Owner, Phase, Progress, Unit, Dimension>::try_new(
        OperationControl::default(),
        limits,
        COMPLETE_PHASES,
    )
    .expect("complete test phase catalog");
    let frozen_evidence_bytes = coordinator
        .try_ledger_snapshot()
        .expect("complete frozen evidence baseline")
        .current_evidence_bytes();

    test_set_evidence_allocation_failure(true);
    let error = coordinator
        .report_progress(Progress::Work, Unit::Work, 1, None)
        .expect_err("injected progress allocation failure");
    assert!(matches!(
        error,
        ControlCause::ProgressStorage {
            cause: ProgressStorageError::Evidence {
                cause: LedgerEvidenceError::AllocationFailed {
                    purpose: LedgerEvidencePurpose::Progress,
                    ..
                }
            }
        }
    ));
    assert_eq!(coordinator.poll(), Err(error));

    let interim = coordinator
        .try_ledger_snapshot()
        .expect("allocation failure leaves capturable evidence");
    assert_eq!(
        interim.current_evidence_bytes(),
        frozen_evidence_bytes,
        "failed growth must roll back current evidence bytes"
    );
    assert!(
        interim.peak_evidence_bytes() > frozen_evidence_bytes,
        "the rejected allocator envelope remains visible in the high-water mark"
    );

    let final_snapshot = coordinator
        .finish()
        .expect("failed growth finalizes cleanly");
    assert_eq!(final_snapshot.current_evidence_bytes(), 0);
    assert!(final_snapshot.peak_evidence_bytes() > frozen_evidence_bytes);
}

#[test]
fn injected_progress_over_envelope_failure_preserves_storage_and_releases_on_finish() {
    let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
        .lock()
        .expect("allocation test lock");
    let limits = limits();
    let mut coordinator = OperationCoordinator::<Owner, Phase, Progress, Unit, Dimension>::try_new(
        OperationControl::default(),
        limits,
        COMPLETE_PHASES,
    )
    .expect("complete test phase catalog");
    coordinator
        .report_progress(Progress::Work, Unit::Work, 1, None)
        .expect("initial progress state");
    let frozen_evidence_bytes = coordinator
        .try_ledger_snapshot()
        .expect("progress evidence baseline")
        .current_evidence_bytes();

    test_fail_next_evidence_over_envelope();
    let error = coordinator
        .report_progress(Progress::Other, Unit::Work, 1, None)
        .expect_err("injected post-allocation envelope violation");
    assert!(matches!(
        &error,
        ControlCause::ProgressStorage {
            cause: ProgressStorageError::Evidence {
                cause: LedgerEvidenceError::Capacity {
                    purpose: LedgerEvidencePurpose::Progress,
                    cause: CapacityPlanError::CapacityEnvelopeViolated { .. },
                }
            }
        }
    ));
    assert_eq!(coordinator.poll(), Err(error));
    assert_eq!(
        coordinator.reported_progress(Progress::Work, Unit::Work),
        Some((1, None)),
        "failed replacement must leave the previous progress entry intact"
    );
    let interim = coordinator
        .try_ledger_snapshot()
        .expect("terminal coordinator retains capturable evidence");
    assert_eq!(
        interim.current_evidence_bytes(),
        frozen_evidence_bytes,
        "failed growth must restore current evidence exactly"
    );
    assert!(
        interim.peak_evidence_bytes() > frozen_evidence_bytes,
        "the transient candidate envelope remains visible in the high-water mark"
    );

    let final_snapshot = coordinator
        .finish()
        .expect("terminal progress storage must release on finish");
    assert_eq!(final_snapshot.current_evidence_bytes(), 0);
}

#[test]
fn finalization_never_enters_the_evidence_allocator() {
    let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
        .lock()
        .expect("allocation test lock");
    let ledger = test_ledger(limits());
    let reservation = ledger
        .reserve(Phase::Build, Dimension::Bytes, 1)
        .expect("live reservation");
    drop(reservation);
    test_set_evidence_allocation_failure(true);
    let evidence = ledger
        .dimension_evidence(Dimension::Bytes)
        .expect("scalar evidence is allocation-free");
    assert_eq!(evidence.current_live_bytes(), 0);
    assert!(test_evidence_allocation_failure_is_armed());
    let snapshot = ledger.finish().expect("clean finalization");
    assert_eq!(snapshot.current_evidence_bytes(), 0);
    assert!(test_evidence_allocation_failure_is_armed());
    test_set_evidence_allocation_failure(false);

    let ledger = test_ledger(limits());
    let reservation = ledger
        .reserve(Phase::Build, Dimension::Bytes, 1)
        .expect("retained reservation");
    test_set_evidence_allocation_failure(true);
    let error = ledger
        .finish()
        .expect_err("live state is an invariant failure");
    assert_eq!(error.snapshot().current_live_bytes(), 1);
    assert!(error.snapshot().current_evidence_bytes() > 0);
    assert!(test_evidence_allocation_failure_is_armed());
    test_set_evidence_allocation_failure(false);
    drop(reservation);
}

#[test]
fn completed_phase_survives_clean_and_non_clean_final_snapshots() {
    let clean =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("clean frozen catalog");
    clean
        .try_complete_phase(Phase::Build)
        .expect("build completes");
    assert_eq!(
        clean
            .finish()
            .expect("clean ledger finishes")
            .last_completed_phase(),
        Some(Phase::Build)
    );

    let non_clean =
        Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
            .expect("non-clean frozen catalog");
    non_clean
        .try_complete_phase(Phase::Build)
        .expect("build completes");
    let reservation = non_clean
        .reserve(Phase::Finalize, Dimension::Bytes, 1)
        .expect("live finalize reservation");
    let error = non_clean
        .finish()
        .expect_err("live reservation makes finalization non-clean");
    assert_eq!(error.snapshot().last_completed_phase(), Some(Phase::Build));
    drop(reservation);
}

fn typed_limits() -> LedgerLimits<Dimension> {
    LedgerLimits::new(
        u64::MAX,
        [(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )],
    )
    .expect("typed test policy")
}

fn probe_values<P, D>(probe: &LedgerTestProbe<P, D>) -> (u64, u64) {
    (probe.active_reservations(), probe.current_live_bytes())
}

#[test]
fn typed_foreign_attribution_push_returns_the_token_without_mutating_either_ledger() {
    let target = test_ledger(typed_limits());
    let foreign = test_ledger(typed_limits());
    let target_probe = target.test_probe();
    let foreign_probe = foreign.test_probe();
    let mut target_batch = target
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("target batch");
    let foreign_token = foreign
        .reserve_operation_attribution::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 5)
        .expect("foreign attribution");
    let target_before = probe_values(&target_probe);
    let foreign_before = probe_values(&foreign_probe);
    assert_eq!(target_before.0, 1);
    assert_eq!(foreign_before.0, 1);
    assert!(target_before.1 > 0);
    assert!(foreign_before.1 >= 5);

    let recovered = match target_batch.try_push_attribution(foreign_token) {
        Ok(()) => panic!("foreign attribution must not enter the target batch"),
        Err(token) => token,
    };

    assert_eq!(target_batch.attribution_len(), 0);
    assert_eq!(probe_values(&target_probe), target_before);
    assert_eq!(probe_values(&foreign_probe), foreign_before);
    drop(recovered);
    drop(target_batch);
    assert_eq!(probe_values(&target_probe), (0, 0));
    assert_eq!(probe_values(&foreign_probe), (0, 0));
}

#[test]
fn typed_commitment_resolves_its_dimension_before_entering_the_batch() {
    let ledger = test_ledger(typed_limits());
    let probe = ledger.test_probe();
    let mut batch = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 1)
        .expect("commitment batch");
    let before = probe_values(&probe);

    let recovered = match ledger
        .try_push_commitment(&mut batch, OutputCommitment::new(Dimension::Records, 5))
    {
        Err(CommitmentPushError::Ledger {
            commitment,
            cause:
                LedgerCause::UnknownDimension {
                    dimension: Dimension::Records,
                },
        }) => commitment,
        Ok(()) => panic!("an unknown commitment dimension must not enter the batch"),
        Err(_) => panic!("unknown commitment dimension returned the wrong rejection"),
    };

    assert_eq!(recovered.dimension(), Dimension::Records);
    assert_eq!(recovered.amount(), 5);
    assert_eq!(recovered.dimension_position, usize::MAX);
    assert!(batch.commitments.is_empty());
    assert_eq!(probe_values(&probe), before);
    drop(batch);
    assert_eq!(probe_values(&probe), (0, 0));
}

#[test]
fn empty_typed_batch_still_rejects_a_foreign_ledger_owner() {
    let target = test_ledger(typed_limits());
    let foreign = test_ledger(typed_limits());
    let target_probe = target.test_probe();
    let foreign_probe = foreign.test_probe();
    let mut target_batch = target
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 1)
        .expect("target commitment batch");
    let target_before = probe_values(&target_probe);
    let foreign_before = probe_values(&foreign_probe);

    let recovered = match foreign.try_push_commitment(
        &mut target_batch,
        OutputCommitment::new(Dimension::Bytes, 3),
    ) {
        Err(CommitmentPushError::Ledger {
            commitment,
            cause: LedgerCause::OwnerMismatch,
        }) => commitment,
        Ok(()) => panic!("an empty foreign batch must not bypass owner validation"),
        Err(_) => panic!("foreign empty batch returned the wrong rejection"),
    };

    assert_eq!(recovered.dimension(), Dimension::Bytes);
    assert_eq!(recovered.amount(), 3);
    assert!(target_batch.commitments.is_empty());
    assert_eq!(probe_values(&target_probe), target_before);
    assert_eq!(probe_values(&foreign_probe), foreign_before);
    drop(target_batch);
    assert_eq!(probe_values(&target_probe), (0, 0));
    assert_eq!(probe_values(&foreign_probe), (0, 0));
}

#[test]
fn typed_foreign_fallback_batch_rejects_the_baseline_without_mutation() {
    let target = test_ledger(typed_limits());
    let foreign = test_ledger(typed_limits());
    let target_probe = target.test_probe();
    let foreign_probe = foreign.test_probe();
    let canonical = target
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("target canonical assembly");
    let evidence_assembly = target
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("target evidence assembly");
    let fallback = foreign
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("foreign fallback batch");
    let evidence = target
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("target evidence batch");
    let target_before = probe_values(&target_probe);
    let foreign_before = probe_values(&foreign_probe);
    assert_eq!(target_before.0, 1);
    assert_eq!(foreign_before.0, 1);

    let rejection =
        match target.try_fallback_baseline(canonical, evidence_assembly, fallback, evidence) {
            Ok(_) => panic!("foreign fallback batch must reject the baseline"),
            Err(rejection) => rejection,
        };

    assert_eq!(probe_values(&target_probe), target_before);
    assert_eq!(probe_values(&foreign_probe), foreign_before);
    let (baseline, cause) = rejection.into_parts();
    assert!(matches!(cause, LedgerCause::OwnerMismatch));
    let (canonical, evidence_assembly, fallback, evidence) = baseline.into_parts();
    drop(canonical);
    drop(evidence_assembly);
    drop(fallback);
    drop(evidence);
    assert_eq!(probe_values(&target_probe), (0, 0));
    assert_eq!(probe_values(&foreign_probe), (0, 0));
}

#[test]
fn typed_terminal_qualification_rejects_a_foreign_discard_batch() {
    let target = test_ledger(typed_limits());
    let foreign = test_ledger(typed_limits());
    let target_probe = target.test_probe();
    let foreign_probe = foreign.test_probe();
    let canonical = target
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("target canonical assembly");
    let evidence_assembly = target
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("target evidence assembly");
    let fallback = target
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("target fallback batch");
    let evidence = target
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("target evidence batch");
    let baseline = target
        .try_fallback_baseline(canonical, evidence_assembly, fallback, evidence)
        .unwrap_or_else(|_| panic!("target baseline must be admitted"));
    let mut foreign_source = foreign
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("foreign source batch");
    let foreign_token = foreign
        .reserve_operation_attribution::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 5)
        .expect("foreign source attribution");
    assert!(foreign_source.try_push_attribution(foreign_token).is_ok());
    let target_before = probe_values(&target_probe);
    let foreign_before = probe_values(&foreign_probe);
    assert_eq!(target_before.0, 1);
    assert_eq!(foreign_before.0, 2);

    let mut baseline = baseline;
    let mut foreign_source = Some(foreign_source);
    let rejection = target
        .try_bind_primary(&mut baseline, foreign_source.as_mut(), None, true)
        .expect_err("foreign primary source must be rejected");

    assert!(matches!(rejection, LedgerCause::OwnerMismatch));
    assert_eq!(probe_values(&target_probe), target_before);
    assert_eq!(probe_values(&foreign_probe), foreign_before);
    let proof = baseline.into_fallback_proof(target, foreign_source, None);
    let proof = match proof.try_into_qualified_discards() {
        Ok(_) => panic!("a foreign discard batch must not qualify for terminal mutation"),
        Err(proof) => proof,
    };
    drop(proof);
    assert_eq!(probe_values(&target_probe), (0, 0));
    assert_eq!(probe_values(&foreign_probe), (0, 0));
}

#[test]
fn typed_primary_rejection_preserves_batches_for_evidence_only_retry() {
    macro_rules! batch_state {
        ($batch:expr) => {{
            let batch = $batch;
            (
                batch.attribution_len(),
                batch.commitment_len(),
                batch.attributed_bytes(),
                batch.active_count(),
                batch.live_bytes(),
                batch.attributions.capacity(),
                batch.commitments.capacity(),
                batch.capacity_cover.is_some(),
            )
        }};
    }

    let limits = LedgerLimits::new(
        u64::MAX,
        [(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )],
    )
    .expect("canonical-rejecting policy");
    let ledger = test_ledger(limits);
    let probe = ledger.test_probe();
    let canonical_assembly = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 1)
        .expect("canonical assembly");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 2, 1)
        .expect("evidence assembly");
    let fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("empty fallback");
    let mut evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("baseline evidence");
    let baseline_evidence = ledger
        .reserve_operation_attribution::<OutcomeEvidenceDestination>(
            Phase::Build,
            Dimension::Bytes,
            2,
        )
        .expect("baseline evidence attribution");
    assert!(evidence.try_push_attribution(baseline_evidence).is_ok());
    let mut baseline = ledger
        .try_fallback_baseline(canonical_assembly, evidence_assembly, fallback, evidence)
        .unwrap_or_else(|_| panic!("primary baseline must bind"));

    let mut canonical_source = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 1)
        .expect("canonical source");
    let canonical_attribution = ledger
        .reserve_operation_attribution::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 3)
        .expect("canonical source attribution");
    assert!(
        canonical_source
            .try_push_attribution(canonical_attribution)
            .is_ok()
    );
    assert!(
        ledger
            .try_push_commitment(
                &mut canonical_source,
                OutputCommitment::new(Dimension::Bytes, 1),
            )
            .is_ok()
    );
    let mut evidence_source = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 1)
        .expect("outcome evidence source");
    let outcome_evidence_attribution = ledger
        .reserve_operation_attribution::<OutcomeEvidenceDestination>(
            Phase::Build,
            Dimension::Bytes,
            4,
        )
        .expect("outcome evidence source attribution");
    assert!(
        evidence_source
            .try_push_attribution(outcome_evidence_attribution)
            .is_ok()
    );
    assert!(
        ledger
            .try_push_commitment(
                &mut evidence_source,
                OutputCommitment::new(Dimension::Bytes, 5),
            )
            .is_ok()
    );

    let snapshot_before = ledger.try_snapshot().expect("pre-rejection snapshot");
    let probe_before = probe_values(&probe);
    let publication_before = lock(&ledger.state).publication;
    let baseline_before = (
        batch_state!(&baseline.canonical_assembly),
        batch_state!(&baseline.evidence_assembly),
        batch_state!(&baseline.fallback),
        batch_state!(&baseline.evidence),
    );
    let canonical_source_before = batch_state!(&canonical_source);
    let evidence_source_before = batch_state!(&evidence_source);

    let rejection = ledger
        .try_bind_primary(
            &mut baseline,
            Some(&mut canonical_source),
            Some(&mut evidence_source),
            true,
        )
        .expect_err("canonical commitment must reject the first primary bind");

    assert!(matches!(
        rejection,
        LedgerCause::LimitExceeded {
            class: CounterClass::Committed,
            dimension: Dimension::Bytes,
            value: 1,
            limit: 0,
        }
    ));
    assert_eq!(
        ledger.try_snapshot().expect("post-rejection snapshot"),
        snapshot_before
    );
    assert_eq!(probe_values(&probe), probe_before);
    assert_eq!(lock(&ledger.state).publication, publication_before);
    assert_eq!(
        (
            batch_state!(&baseline.canonical_assembly),
            batch_state!(&baseline.evidence_assembly),
            batch_state!(&baseline.fallback),
            batch_state!(&baseline.evidence),
        ),
        baseline_before
    );
    assert_eq!(batch_state!(&canonical_source), canonical_source_before);
    assert_eq!(batch_state!(&evidence_source), evidence_source_before);

    ledger
        .try_bind_primary(
            &mut baseline,
            Some(&mut canonical_source),
            Some(&mut evidence_source),
            false,
        )
        .unwrap_or_else(|_| panic!("evidence-only retry must bind the intact owners"));
    let proof = ledger.finish_bound_primary(
        baseline,
        Some(canonical_source),
        Some(evidence_source),
        false,
    );
    let discard = match proof.try_into_qualified_discards() {
        Ok(QualifiedTypedDiscardFreezeProof::Canonical(discard)) => discard,
        Ok(_) => panic!("evidence-only retry must leave only canonical storage for discard"),
        Err(_) => panic!("bound evidence-only retry must qualify for terminal freeze"),
    };
    let snapshot = discard.release_discard_after_physical_cleanup().freeze();
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
    let dimension = snapshot
        .dimensions()
        .find_map(|(dimension, snapshot)| (dimension == Dimension::Bytes).then_some(snapshot))
        .expect("bytes dimension");
    assert_eq!(dimension.committed_canonical_output(), 0);
    assert_eq!(dimension.published_outcome_evidence(), 5);
    assert_eq!(probe_values(&probe), (0, 0));
}

static PANIC_ON_BOUND_ORD: AtomicBool = AtomicBool::new(false);
static REVERSE_BOUND_ORD: AtomicBool = AtomicBool::new(false);
static BOUND_ORD_COMPARISONS_REMAINING: AtomicUsize = AtomicUsize::new(usize::MAX);
static BOUND_ORD_COMPARISON_COUNT: AtomicUsize = AtomicUsize::new(0);
static BOUND_ORD_TEST_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy)]
struct StatefulOrd(u8);

impl PartialEq for StatefulOrd {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for StatefulOrd {}

impl PartialOrd for StatefulOrd {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StatefulOrd {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        BOUND_ORD_COMPARISON_COUNT.fetch_add(1, Ordering::SeqCst);
        assert!(
            !PANIC_ON_BOUND_ORD.load(Ordering::SeqCst),
            "typed freeze called owner-defined Ord after positions were bound"
        );
        let remaining = BOUND_ORD_COMPARISONS_REMAINING.load(Ordering::SeqCst);
        if remaining != usize::MAX {
            assert_ne!(
                remaining, 0,
                "operation called owner-defined Ord after its lookup preflight"
            );
            BOUND_ORD_COMPARISONS_REMAINING.fetch_sub(1, Ordering::SeqCst);
        }
        let ordering = self.0.cmp(&other.0);
        if REVERSE_BOUND_ORD.load(Ordering::SeqCst) {
            ordering.reverse()
        } else {
            ordering
        }
    }
}

fn stateful_ord_limits() -> LedgerLimits<StatefulOrd> {
    LedgerLimits::new(
        u64::MAX,
        [(
            StatefulOrd(7),
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )],
    )
    .expect("stateful Ord test policy")
}

fn stateful_multi_dimension_limits() -> LedgerLimits<StatefulOrd> {
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    LedgerLimits::new(
        u64::MAX,
        [(StatefulOrd(7), policy), (StatefulOrd(9), policy)],
    )
    .expect("stateful multi-dimension test policy")
}

fn stateful_ledger(
    limits: LedgerLimits<StatefulOrd>,
    phases: &[StatefulOrd],
) -> Ledger<Owner, StatefulOrd, StatefulOrd> {
    Ledger::try_new(limits, phases).expect("complete stateful phase catalog")
}

fn freeze_while_ord_panics(
    proof: TypedFreezeProof<Owner, StatefulOrd, StatefulOrd>,
) -> LedgerSnapshot<StatefulOrd, StatefulOrd> {
    PANIC_ON_BOUND_ORD.store(true, Ordering::SeqCst);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match proof.try_into_qualified_discards() {
            Ok(QualifiedTypedDiscardFreezeProof::None(proof)) => proof.freeze(),
            Ok(proof) => {
                proof.retain_fail_closed();
                panic!("test proof unexpectedly requires physical discard acknowledgement")
            }
            Err(_) => panic!("bound typed proof must qualify"),
        }
    }));
    PANIC_ON_BOUND_ORD.store(false, Ordering::SeqCst);
    result.expect("bound typed freeze must not call owner-defined Ord")
}

#[test]
fn consumed_batch_never_compares_owner_keys_after_its_lookup_preflight() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let first = StatefulOrd(7);
    let second = StatefulOrd(9);
    let ledger = stateful_ledger(stateful_multi_dimension_limits(), &[StatefulOrd(2)]);
    ledger
        .charge_attempted(first, 1)
        .expect("first attempted work");
    ledger
        .charge_attempted(second, 1)
        .expect("second attempted work");

    let lookup_comparisons = {
        let state = lock(&ledger.state);
        BOUND_ORD_COMPARISON_COUNT.store(0, Ordering::SeqCst);
        for dimension in [first, second] {
            state
                .dimensions
                .binary_search_by_key(&dimension, |entry| entry.dimension)
                .expect("predeclared dimension");
        }
        BOUND_ORD_COMPARISON_COUNT.load(Ordering::SeqCst)
    };
    BOUND_ORD_COMPARISONS_REMAINING.store(lookup_comparisons, Ordering::SeqCst);
    let charged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ledger.charge_consumed_batch(&[(first, 1), (second, 1)])
    }));
    let remaining = BOUND_ORD_COMPARISONS_REMAINING.swap(usize::MAX, Ordering::SeqCst);

    assert_eq!(remaining, 0, "every admitted lookup must consume its quota");
    charged
        .expect("batch compared owner keys after resolving numeric positions")
        .expect("predeclared completed work must fit");
    assert_eq!(lock(&ledger.state).dimensions[0].state.consumed, 1);
    assert_eq!(lock(&ledger.state).dimensions[1].state.consumed, 1);
}

#[test]
fn reservation_transition_never_compares_owner_keys_after_its_lookup_preflight() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let phase = StatefulOrd(2);
    let dimension = StatefulOrd(7);
    let ledger = stateful_ledger(stateful_ord_limits(), &[phase]);
    let probe = ledger.test_probe();
    let first = ledger
        .reserve(phase, dimension, 2)
        .expect("first transition source");
    let second = ledger
        .reserve(phase, dimension, 3)
        .expect("second transition source");
    let mut reservations = [first, second];

    BOUND_ORD_COMPARISONS_REMAINING.store(2, Ordering::SeqCst);
    let transition = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ledger.transition_reservations(&mut reservations, phase, dimension, 4)
    }));
    let remaining = BOUND_ORD_COMPARISONS_REMAINING.swap(usize::MAX, Ordering::SeqCst);

    assert_eq!(
        remaining, 0,
        "dimension and phase lookup must each compare once"
    );
    transition
        .expect("transition compared owner keys after releasing source bytes")
        .expect("same-dimension transition must succeed");
    assert_eq!(reservations[0].bytes(), 2);
    assert_eq!(reservations[1].bytes(), 2);
    assert_eq!(probe_values(&probe), (2, 4));
    drop(reservations);
    assert_eq!(probe_values(&probe), (0, 0));
}

#[test]
fn reservation_construction_binds_keys_once_and_resize_uses_only_bound_positions() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let phase = StatefulOrd(2);
    let dimension = StatefulOrd(7);
    let ledger = stateful_ledger(stateful_ord_limits(), &[phase]);

    BOUND_ORD_COMPARISONS_REMAINING.store(2, Ordering::SeqCst);
    let reservation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ledger.reserve(phase, dimension, 3)
    }));
    let remaining = BOUND_ORD_COMPARISONS_REMAINING.swap(usize::MAX, Ordering::SeqCst);
    assert_eq!(
        remaining, 0,
        "reservation admission resolves dimension and phase exactly once"
    );
    let mut reservation = reservation
        .expect("reservation admission repeated an owner-key lookup")
        .expect("predeclared reservation must fit");

    PANIC_ON_BOUND_ORD.store(true, Ordering::SeqCst);
    let resize = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ledger.resize_reservation(&mut reservation, 5)
    }));
    PANIC_ON_BOUND_ORD.store(false, Ordering::SeqCst);
    resize
        .expect("resize compared keys already bound into the reservation")
        .expect("bound reservation resize must fit");
    assert_eq!(reservation.bytes(), 5);
    assert_eq!(ledger.test_probe().current_live_bytes(), 5);
    drop(reservation);
    assert_eq!(ledger.test_probe().current_live_bytes(), 0);
}

#[test]
fn reservation_transfer_uses_bound_positions_after_its_lookup_preflight() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let source_phase = StatefulOrd(1);
    let target_phase = StatefulOrd(2);
    let dimension = StatefulOrd(7);
    let ledger = stateful_ledger(stateful_ord_limits(), &[source_phase, target_phase]);
    let mut reservation = ledger
        .reserve(source_phase, dimension, 5)
        .expect("transfer source");

    BOUND_ORD_COMPARISONS_REMAINING.store(3, Ordering::SeqCst);
    let transfer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ledger.transfer(&mut reservation, target_phase)
    }));
    let remaining = BOUND_ORD_COMPARISONS_REMAINING.swap(usize::MAX, Ordering::SeqCst);

    assert_eq!(
        remaining, 0,
        "source equality and target phase lookup must finish before mutation"
    );
    transfer
        .expect("transfer compared owner keys after its target lookup")
        .expect("predeclared target phase must accept the transfer");
    assert_eq!(
        ledger
            .phase_evidence(source_phase)
            .expect("source phase evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        ledger
            .phase_evidence(target_phase)
            .expect("target phase evidence")
            .current_live_bytes(),
        5
    );

    REVERSE_BOUND_ORD.store(true, Ordering::SeqCst);
    let release = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drop(reservation);
    }));
    REVERSE_BOUND_ORD.store(false, Ordering::SeqCst);
    release.expect("bound reservation release must not compare owner keys");
    assert_eq!(ledger.test_probe().current_live_bytes(), 0);
}

#[test]
fn commitment_aggregate_panic_releases_current_evidence_before_unwind() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let phase = StatefulOrd(2);
    let dimension = StatefulOrd(7);
    let ledger = stateful_ledger(stateful_ord_limits(), &[phase]);
    let before = lock(&ledger.state).current_evidence_bytes;
    let mut reservations = PublicationReservationBatch::default();

    PANIC_ON_BOUND_ORD.store(true, Ordering::SeqCst);
    let aggregation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ledger.prepare_publication(&mut reservations, &[Commitment::new(dimension, 3)])
    }));
    PANIC_ON_BOUND_ORD.store(false, Ordering::SeqCst);

    assert!(
        aggregation.is_err(),
        "owner comparator must reach the test panic"
    );
    let state = lock(&ledger.state);
    assert_eq!(state.current_evidence_bytes, before);
    assert_eq!(state.publication, PublicationState::Open);
    assert_eq!(state.dimensions[0].state.pending_publication, 0);
}

#[test]
fn pending_publication_publish_and_drop_use_positions_after_ord_reverses() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let phase = StatefulOrd(2);
    let dimension = StatefulOrd(9);
    let ledger = stateful_ledger(stateful_multi_dimension_limits(), &[phase]);
    let mut reservations = PublicationReservationBatch::default();
    let mut pending = ledger
        .prepare_publication(&mut reservations, &[Commitment::new(dimension, 3)])
        .expect("pending publication");

    REVERSE_BOUND_ORD.store(true, Ordering::SeqCst);
    let published = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ledger.publish(&mut pending)
    }));
    REVERSE_BOUND_ORD.store(false, Ordering::SeqCst);
    published
        .expect("publish compared a dimension after admission")
        .expect("bound publication must succeed");
    assert_eq!(lock(&ledger.state).dimensions[1].state.committed, 3);

    let rollback_ledger = stateful_ledger(stateful_multi_dimension_limits(), &[phase]);
    let baseline_evidence = lock(&rollback_ledger.state).current_evidence_bytes;
    let mut rollback_reservations = PublicationReservationBatch::default();
    let rollback = rollback_ledger
        .prepare_publication(&mut rollback_reservations, &[Commitment::new(dimension, 4)])
        .expect("rollback publication");
    REVERSE_BOUND_ORD.store(true, Ordering::SeqCst);
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(rollback)));
    REVERSE_BOUND_ORD.store(false, Ordering::SeqCst);
    dropped.expect("rollback compared a dimension after admission");
    let state = lock(&rollback_ledger.state);
    assert_eq!(state.dimensions[1].state.pending_publication, 0);
    assert_eq!(state.current_evidence_bytes, baseline_evidence);
    assert_eq!(state.publication, PublicationState::Open);
}

#[test]
fn fallback_baseline_counts_zero_byte_attribution_capabilities() {
    let ledger = test_ledger(typed_limits());
    let extra = ledger
        .reserve_operation_attribution::<OutcomeEvidenceDestination>(
            Phase::Build,
            Dimension::Bytes,
            0,
        )
        .expect("zero-byte attribution");
    let canonical = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("empty canonical assembly");
    let evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("empty evidence assembly");
    let fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("empty fallback batch");
    let snapshot = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("empty snapshot batch");

    let rejection = match ledger.try_fallback_baseline(canonical, evidence, fallback, snapshot) {
        Ok(_) => panic!("unowned zero-byte capability must reject the baseline"),
        Err(rejection) => rejection,
    };
    assert!(matches!(
        rejection.into_parts().1,
        LedgerCause::ActiveReservationsRemain { count: 1 }
    ));
    drop(extra);
}

#[test]
fn included_zero_byte_attribution_freezes_to_zero_active_capabilities() {
    let ledger = Ledger::<Owner, Phase, Dimension>::try_new(typed_limits(), &[Phase::Build])
        .expect("typed frozen catalog");
    let racing_ledger = Ledger::<Owner, Phase, Dimension> {
        state: Arc::clone(&ledger.state),
        operation_live_byte_limit: ledger.operation_live_byte_limit,
        operation_owner: PhantomData,
    };
    ledger
        .try_complete_phase(Phase::Build)
        .expect("build completes before typed freeze");
    let canonical = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("empty canonical assembly");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("evidence assembly");
    let mut fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("fallback batch");
    let zero = ledger
        .reserve_operation_attribution::<OutcomeEvidenceDestination>(
            Phase::Build,
            Dimension::Bytes,
            0,
        )
        .expect("zero-byte attribution");
    assert!(fallback.try_push_attribution(zero).is_ok());
    let evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("snapshot batch");
    let baseline =
        match ledger.try_fallback_baseline(canonical, evidence_assembly, fallback, evidence) {
            Ok(baseline) => baseline,
            Err(_) => panic!("exact zero-byte baseline must be admitted"),
        };
    let qualified = match baseline
        .into_fallback_proof(ledger, None, None)
        .try_into_qualified_discards()
    {
        Ok(QualifiedTypedDiscardFreezeProof::None(proof)) => proof,
        Ok(proof) => {
            proof.retain_fail_closed();
            panic!("zero-discard proof changed terminal variant")
        }
        Err(_) => panic!("exact typed proof must qualify"),
    };
    assert_eq!(
        lock(&racing_ledger.state).publication,
        PublicationState::TerminalQualified
    );
    let racing_mutation = std::thread::spawn(move || {
        racing_ledger.tighten_dimension_policy(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
    })
    .join()
    .expect("racing policy mutation must not panic");
    assert_eq!(
        racing_mutation,
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    let state = Arc::clone(
        &qualified
            .proof
            .as_ref()
            .expect("qualified proof")
            .proof
            .as_ref()
            .expect("typed proof")
            .ledger
            .state,
    );
    let snapshot = qualified.freeze();
    assert_eq!(lock(&state).publication, PublicationState::Published);
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
    assert_eq!(snapshot.last_completed_phase(), Some(Phase::Build));
}

#[test]
fn terminal_qualification_rejects_an_out_of_bounds_zero_byte_attribution() {
    let ledger = Ledger::<Owner, Phase, Dimension>::try_new(typed_limits(), &[Phase::Build])
        .expect("typed frozen catalog");
    let probe = ledger.test_probe();
    let canonical = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("empty canonical assembly");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("evidence assembly");
    let mut fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("fallback batch");
    let zero = ledger
        .reserve_operation_attribution::<OutcomeEvidenceDestination>(
            Phase::Build,
            Dimension::Bytes,
            0,
        )
        .expect("zero-byte attribution");
    assert!(fallback.try_push_attribution(zero).is_ok());
    let evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("snapshot batch");
    let baseline = ledger
        .try_fallback_baseline(canonical, evidence_assembly, fallback, evidence)
        .unwrap_or_else(|_| panic!("exact zero-byte baseline must bind"));
    let mut proof = baseline.into_fallback_proof(ledger, None, None);
    assert_eq!(proof.evidence.attributions[0].core.bytes, 0);
    proof.evidence.attributions[0].core.dimension_position = usize::MAX;
    proof.evidence.attributions[0].core.phase_position = usize::MAX;

    let proof = match proof.try_into_qualified_discards() {
        Ok(_) => panic!("out-of-bounds zero-byte attribution must not qualify"),
        Err(proof) => proof,
    };
    drop(proof);
    assert_eq!(probe_values(&probe), (0, 0));
}

#[test]
fn terminal_qualification_rechecks_bound_commitments_after_policy_tightening() {
    let ledger = test_ledger(typed_limits());
    let probe = ledger.test_probe();
    let canonical_assembly = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 1)
        .expect("canonical assembly");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("evidence assembly");
    let fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("fallback batch");
    let evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("evidence batch");
    let baseline = ledger
        .try_fallback_baseline(canonical_assembly, evidence_assembly, fallback, evidence)
        .unwrap_or_else(|_| panic!("exact primary baseline must bind"));
    let mut source = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 1)
        .expect("canonical source");
    let attribution = ledger
        .reserve_operation_attribution::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0)
        .expect("zero-byte canonical attribution");
    assert!(source.try_push_attribution(attribution).is_ok());
    assert!(
        ledger
            .try_push_commitment(&mut source, OutputCommitment::new(Dimension::Bytes, 5),)
            .is_ok()
    );
    let mut baseline = baseline;
    let mut source = Some(source);
    ledger
        .try_bind_primary(&mut baseline, source.as_mut(), None, true)
        .unwrap_or_else(|_| panic!("commitment must bind under the original policy"));
    ledger
        .tighten_dimension_policy(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 4, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
        .expect("policy may tighten before terminal qualification");
    let proof = ledger.finish_bound_primary(baseline, source, None, true);

    let proof = match proof.try_into_qualified_discards() {
        Ok(_) => panic!("commitment above the final policy must not qualify"),
        Err(proof) => proof,
    };
    drop(proof);
    assert_eq!(probe_values(&probe), (0, 0));
}

#[test]
fn typed_final_preflight_rejects_an_invalid_completed_phase_pair_without_mutation() {
    let ledger = Ledger::<Owner, Phase, Dimension>::try_new(typed_limits(), &[Phase::Build])
        .expect("typed frozen catalog");
    ledger
        .try_complete_phase(Phase::Build)
        .expect("build completes before fault injection");
    let canonical = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("empty canonical assembly");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("evidence assembly");
    let fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 1, 0)
        .expect("fallback batch");
    let evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Build, Dimension::Bytes, 0, 0)
        .expect("evidence batch");
    let probe = ledger.test_probe();
    let before = probe_values(&probe);
    {
        let mut state = lock(&ledger.state);
        state
            .last_completed_phase
            .as_mut()
            .expect("completed phase")
            .state_position = state.phase_states.len();
    }

    let rejection =
        match ledger.try_fallback_baseline(canonical, evidence_assembly, fallback, evidence) {
            Ok(_) => panic!("invalid completed pair cannot produce a typed freeze proof"),
            Err(rejection) => rejection,
        };

    assert_eq!(probe_values(&probe), before);
    let (baseline, cause) = rejection.into_parts();
    assert_eq!(cause, LedgerCause::FinalSnapshotShapeMismatch);
    let (canonical, evidence_assembly, fallback, evidence) = baseline.into_parts();
    drop(canonical);
    drop(evidence_assembly);
    drop(fallback);
    drop(evidence);
    assert_eq!(probe_values(&probe), (0, 0));
}

#[test]
fn fallback_freeze_uses_stable_positions_for_attribution_and_capacity_cover() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let terminal_phase = StatefulOrd(2);
    let dimension = StatefulOrd(7);
    let ledger = stateful_ledger(stateful_ord_limits(), &[terminal_phase, StatefulOrd(1)]);
    let canonical = ledger
        .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 0, 0)
        .expect("empty canonical assembly");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 1, 0)
        .expect("covered evidence assembly");
    let mut fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 1, 0)
        .expect("covered fallback batch");
    let attribution = ledger
        .reserve_operation_attribution::<OutcomeEvidenceDestination>(terminal_phase, dimension, 3)
        .expect("fallback attribution");
    assert!(fallback.try_push_attribution(attribution).is_ok());
    let evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
        .expect("empty evidence batch");
    let baseline = ledger
        .try_fallback_baseline(canonical, evidence_assembly, fallback, evidence)
        .unwrap_or_else(|_| panic!("exact fallback baseline must bind"));

    let earlier = ledger
        .reserve(StatefulOrd(1), dimension, 1)
        .expect("predeclared earlier phase");
    drop(earlier);
    let proof = baseline.into_fallback_proof(ledger, None, None);
    let snapshot = freeze_while_ord_panics(proof);

    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
}

#[test]
fn primary_freeze_uses_bound_source_attribution_cover_and_commitment_positions() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let terminal_phase = StatefulOrd(2);
    let dimension = StatefulOrd(7);
    let ledger = stateful_ledger(stateful_ord_limits(), &[terminal_phase, StatefulOrd(1)]);
    let canonical_assembly = ledger
        .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
        .expect("covered canonical assembly");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
        .expect("empty evidence assembly");
    let fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
        .expect("empty fallback batch");
    let evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
        .expect("empty evidence batch");
    let baseline = ledger
        .try_fallback_baseline(canonical_assembly, evidence_assembly, fallback, evidence)
        .unwrap_or_else(|_| panic!("exact primary baseline must bind"));

    let earlier = ledger
        .reserve(StatefulOrd(1), dimension, 1)
        .expect("predeclared earlier phase");
    drop(earlier);
    let mut source = ledger
        .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
        .expect("covered canonical source");
    let attribution = ledger
        .reserve_operation_attribution::<CanonicalOutput>(terminal_phase, dimension, 5)
        .expect("canonical attribution");
    assert!(source.try_push_attribution(attribution).is_ok());
    assert!(
        ledger
            .try_push_commitment(&mut source, OutputCommitment::new(dimension, 3))
            .is_ok()
    );
    let mut baseline = baseline;
    let mut source = Some(source);
    ledger
        .try_bind_primary(&mut baseline, source.as_mut(), None, true)
        .unwrap_or_else(|_| panic!("exact primary publication must bind"));
    let proof = ledger.finish_bound_primary(baseline, source, None, true);
    let snapshot = freeze_while_ord_panics(proof);

    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
    assert_eq!(
        snapshot
            .dimensions()
            .next()
            .expect("one dimension")
            .1
            .committed_canonical_output(),
        3
    );
}

#[test]
fn terminal_freeze_keeps_commitments_on_bound_positions_after_ord_reverses() {
    let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
    let earlier_phase = StatefulOrd(1);
    let terminal_phase = StatefulOrd(2);
    let dimension = StatefulOrd(9);
    let ledger = stateful_ledger(
        stateful_multi_dimension_limits(),
        &[earlier_phase, terminal_phase],
    );
    let earlier = ledger
        .reserve(earlier_phase, StatefulOrd(7), 1)
        .expect("predeclared earlier phase");
    drop(earlier);
    let canonical_assembly = ledger
        .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
        .expect("covered canonical assembly");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
        .expect("empty evidence assembly");
    let fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
        .expect("empty fallback batch");
    let evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
        .expect("empty evidence batch");
    let baseline = ledger
        .try_fallback_baseline(canonical_assembly, evidence_assembly, fallback, evidence)
        .unwrap_or_else(|_| panic!("exact primary baseline must bind"));
    let mut source = ledger
        .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
        .expect("covered canonical source");
    let attribution = ledger
        .reserve_operation_attribution::<CanonicalOutput>(terminal_phase, dimension, 5)
        .expect("canonical attribution");
    assert!(source.try_push_attribution(attribution).is_ok());
    assert!(
        ledger
            .try_push_commitment(&mut source, OutputCommitment::new(dimension, 3))
            .is_ok()
    );
    let mut baseline = baseline;
    let mut source = Some(source);
    ledger
        .try_bind_primary(&mut baseline, source.as_mut(), None, true)
        .unwrap_or_else(|_| panic!("exact primary publication must bind"));
    let proof = ledger.finish_bound_primary(baseline, source, None, true);

    REVERSE_BOUND_ORD.store(true, Ordering::SeqCst);
    let frozen = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match proof.try_into_qualified_discards() {
            Ok(QualifiedTypedDiscardFreezeProof::None(proof)) => proof.freeze(),
            Ok(proof) => {
                proof.retain_fail_closed();
                panic!("primary publication unexpectedly requires a discard")
            }
            Err(_) => panic!("bound typed proof must qualify after Ord reverses"),
        }
    }));
    REVERSE_BOUND_ORD.store(false, Ordering::SeqCst);
    let snapshot = frozen.expect("terminal freeze must use bound numeric positions");

    let mut dimensions = snapshot.dimensions();
    let (first_key, first) = dimensions.next().expect("first dimension");
    let (second_key, second) = dimensions.next().expect("second dimension");
    assert_eq!(first_key, StatefulOrd(7));
    assert_eq!(first.committed_canonical_output(), 0);
    assert_eq!(second_key, StatefulOrd(9));
    assert_eq!(second.committed_canonical_output(), 3);
    assert!(dimensions.next().is_none());
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
}

#[test]
fn typed_terminal_binding_uses_prestored_numeric_positions_without_a_mutation_tail() {
    let root = ledger_root_source();
    let commitment = ledger_commitment_source();
    let reservation = ledger_reservation_source();
    let production = ledger_production_source();
    let primary = root
        .split("pub(crate) fn try_bind_primary(")
        .nth(1)
        .expect("typed primary binding exists")
        .split("pub(crate) fn try_bind_fallback_discards(")
        .next()
        .expect("typed primary binding boundary exists");
    assert_eq!(
        primary
            .matches("preflight_typed_publication_batch(")
            .count(),
        5
    );
    assert_eq!(primary.matches("lock(&self.state)").count(), 1);
    assert!(primary.contains("validate_primary_publication_locked("));
    assert!(!primary.contains("bind_preflighted"));

    let fallback = root
        .split("pub(crate) fn try_bind_fallback_baseline(")
        .nth(1)
        .expect("fallback binding exists")
        .split("fn validate_fallback_baseline_locked(")
        .next()
        .expect("fallback binding boundary exists");
    assert_eq!(
        fallback
            .matches("preflight_typed_publication_batch(")
            .count(),
        4
    );
    assert_eq!(fallback.matches("lock(&self.state)").count(), 1);
    assert!(fallback.contains("validate_fallback_baseline_locked("));
    assert!(!fallback.contains("bind_preflighted"));

    let preflight = commitment
        .split("fn preflight_typed_publication_batch")
        .nth(1)
        .expect("numeric preflight exists")
        .split("fn preflight_typed_discard_batch")
        .next()
        .expect("numeric preflight boundary exists");
    assert!(preflight.contains("bound_positions_are_in_bounds"));
    for forbidden in [
        "binary_search",
        "publication_positions",
        "bind_publication_positions",
        "&mut AttributionReservationBatch",
    ] {
        assert!(
            !preflight.contains(forbidden),
            "terminal preflight regained a semantic lookup or mutation through `{forbidden}`"
        );
    }

    let push = root
        .split("pub(crate) fn try_push_commitment")
        .nth(1)
        .expect("ledger commitment insertion exists")
        .split("/// Transfers live ownership")
        .next()
        .expect("ledger commitment insertion boundary exists");
    let search = push
        .find("binary_search_by_key")
        .expect("commitment dimension is resolved at insertion");
    let store = push
        .find("commitment.dimension_position = dimension_position")
        .expect("resolved numeric position is stored in the commitment");
    let insertion = push
        .find("push_positioned_commitment(commitment)")
        .expect("positioned commitment is inserted without allocation");
    assert!(search < store && store < insertion);
    for forbidden in [
        "?",
        "binary_search",
        "dimension_entry(",
        "dimension_entry_mut(",
        "try_reserve",
        ".reserve(",
        ".insert(",
        "expect(",
        "unwrap",
        "panic!",
    ] {
        assert!(
            !push[store..].contains(forbidden),
            "commitment insertion regained a lookup, allocation, or fault gate after binding its position through `{forbidden}`"
        );
    }

    let batch = reservation
        .split("pub(crate) struct AttributionReservationBatch")
        .nth(1)
        .expect("typed publication batch exists")
        .split("impl<O, P, D, Destination> AttributionReservationBatch")
        .next()
        .expect("typed publication batch boundary exists");
    assert!(batch.contains("owner: Arc<Mutex<LedgerState<P, D>>>"));
    assert!(
        !reservation.contains("TypedFreezeProof"),
        "reservation batches must not depend on the terminal proof"
    );
    for removed in [
        "fn bind_preflighted_typed_publication_batch",
        "fn bind_preflighted_typed_discard_batch",
        "fn publication_positions",
        "fn bind_publication_positions",
    ] {
        assert!(
            !production.contains(removed),
            "removed two-pass binding seam returned through `{removed}`"
        );
    }
}

#[test]
fn consumed_batch_binds_every_dimension_before_its_numeric_mutation_tail() {
    let production = ledger_root_source();
    let batch = production
        .split("pub fn charge_consumed_batch<const N: usize>(")
        .nth(1)
        .expect("fixed-array consumed batch exists")
        .split("/// Reserves byte capacity")
        .next()
        .expect("consumed batch boundary exists");
    assert!(batch.contains("charges: &[(D, u64); N]"));
    assert!(batch.contains("let mut positions = [0_usize; N]"));
    assert!(batch.contains("let mut projected = [0_u64; N]"));

    let first_mutation = batch
        .find("state.dimensions[position].state.consumed = projected[index]")
        .expect("consumed batch has one numeric mutation tail");
    let preflight = &batch[..first_mutation];
    assert!(preflight.contains("binary_search_by_key"));
    assert!(preflight.contains("position < positions[index - 1]"));
    let mutation_tail = &batch[first_mutation..];
    for forbidden in [
        "?",
        "charges[",
        "binary_search",
        "dimension_entry(",
        "charge_consumed_open(",
        "checked_",
        "try_reserve",
        ".reserve(",
        ".push(",
        ".insert(",
        "expect(",
        "unwrap",
        "panic!",
    ] {
        assert!(
            !mutation_tail.contains(forbidden),
            "consumed batch regained a semantic, fallible, or allocating operation after its first mutation through `{forbidden}`"
        );
    }
}

#[test]
fn reservation_transition_finishes_all_fallible_lookup_before_releasing_source_bytes() {
    let production = ledger_root_source();
    let transition = production
        .split("pub fn transition_reservations(")
        .nth(1)
        .expect("reservation transition exists")
        .split("/// Splits bytes")
        .next()
        .expect("reservation transition boundary exists");
    let dimension_lookup = transition
        .find("binary_search_by_key")
        .expect("target dimension is resolved before mutation");
    let phase_lookup = transition
        .find("phase_position(&state, phase)")
        .expect("target phase is resolved before mutation");
    let first_release = transition
        .find("release_positioned_bytes_locked")
        .expect("source bytes are released by numeric position");
    assert!(dimension_lookup < first_release);
    assert!(phase_lookup < first_release);
    assert!(
        transition[..first_release]
            .contains("reservation.core.dimension_position >= state.dimensions.len()")
    );
    assert!(
        transition[..first_release]
            .contains("reservation.core.phase_position >= state.phase_states.len()")
    );

    let mutation_tail = &transition[first_release..];
    for forbidden in [
        "?",
        "binary_search",
        "phase_position(",
        "dimension_entry(",
        "phase_state(",
        "release_phase(",
        "release_bytes_locked(",
        "try_reserve",
        ".reserve(",
        ".push(",
        ".insert(",
        "expect(",
        "unwrap",
        "panic!",
    ] {
        assert!(
            !mutation_tail.contains(forbidden),
            "transition regained a fallible or semantic operation after its first release through `{forbidden}`"
        );
    }
}

#[test]
fn reservation_admission_and_resize_use_one_preflight_then_numeric_slots() {
    let production = ledger_root_source();
    let reserve = production
        .split("fn reserve_core(")
        .nth(1)
        .expect("reservation admission exists")
        .split("fn project_reservation(")
        .next()
        .expect("reservation admission boundary exists");
    let first_mutation = reserve
        .find("dimension_state.current_live_bytes = dimension_live")
        .expect("dimension accounting mutation exists");
    assert!(
        reserve[..first_mutation].contains("self.project_reservation("),
        "reservation admission must complete the shared projection before its first write"
    );
    let projection = production
        .split("fn project_reservation(")
        .nth(1)
        .expect("shared reservation projection exists")
        .split("/// Checks aggregate reservation headroom")
        .next()
        .expect("shared reservation projection boundary exists");
    assert!(projection.contains("binary_search_by_key"));
    assert!(projection.contains("phase_position(state, phase)"));
    assert!(
        !projection.contains("current_live_bytes ="),
        "shared reservation projection must remain read-only"
    );
    for forbidden in [
        "?",
        "binary_search",
        "dimension_entry(",
        "dimension_entry_mut(",
        "phase_position(",
        "phase_state(",
        "try_reserve",
        ".reserve(",
        ".push(",
        ".insert(",
        "expect(",
        "unwrap",
        "panic!",
    ] {
        assert!(
            !reserve[first_mutation..].contains(forbidden),
            "reservation admission regained a fallible or semantic operation after its first write through `{forbidden}`"
        );
    }

    let resize = production
        .split("fn resize_core(")
        .nth(1)
        .expect("reservation resize exists")
        .split("/// Atomically replaces one reservation")
        .next()
        .expect("reservation resize boundary exists");
    assert!(resize.contains("let dimension_position = reservation.dimension_position"));
    assert!(resize.contains("let phase_position = reservation.phase_position"));
    assert!(resize.contains("dimension_position >= state.dimensions.len()"));
    assert!(resize.contains("phase_position >= state.phase_states.len()"));
    let first_resize_mutation = resize
        .find("dimension_state.current_live_bytes = dimension_live")
        .expect("resize dimension mutation exists");
    for forbidden in [
        "binary_search",
        "dimension_entry(",
        "dimension_entry_mut(",
        "phase_position(",
        "phase_state(",
    ] {
        assert!(
            !resize.contains(forbidden),
            "reservation resize ignored its bound numeric slot through `{forbidden}`"
        );
    }
    for forbidden in [
        "?",
        "try_reserve",
        ".reserve(",
        ".push(",
        ".insert(",
        "expect(",
        "unwrap",
        "panic!",
    ] {
        assert!(
            !resize[first_resize_mutation..].contains(forbidden),
            "reservation resize regained an allocation or fault gate after its first write through `{forbidden}`"
        );
    }
}

#[test]
fn reservation_transfer_finishes_all_fallible_lookup_before_mutating_phase_bytes() {
    let production = ledger_root_source();
    let transfer = production
        .split("pub fn transfer(")
        .nth(1)
        .expect("reservation transfer exists")
        .split("/// Resizes one live reservation")
        .next()
        .expect("reservation transfer boundary exists");
    let target_lookup = transfer
        .find("phase_position(&state, phase)")
        .expect("target phase is resolved before mutation");
    let first_mutation = transfer
        .find("state.phase_states[source_position].current_live_bytes = source_live")
        .expect("source bytes are released by numeric position");
    assert!(target_lookup < first_mutation);
    assert!(transfer[..first_mutation].contains("source_position >= state.phase_states.len()"));

    let mutation_tail = &transfer[first_mutation..];
    for forbidden in [
        "?",
        "binary_search",
        "phase_position(",
        "phase_state(",
        "release_phase(",
        "try_reserve",
        ".reserve(",
        ".push(",
        ".insert(",
        "expect(",
        "unwrap",
        "panic!",
    ] {
        assert!(
            !mutation_tail.contains(forbidden),
            "transfer regained a fallible or semantic operation after its first write through `{forbidden}`"
        );
    }
}

#[test]
fn legacy_publication_binds_commitment_positions_before_counter_mutation() {
    let production = ledger_root_source();
    let aggregate = ledger_commitment_source()
        .split("struct AggregatedCommitment")
        .nth(1)
        .expect("commitment unwind guard exists")
        .split("fn validate_commitments")
        .next()
        .expect("commitment aggregate boundary exists");
    assert!(aggregate.contains("dimension_position: usize"));
    assert!(aggregate.contains("drop(std::mem::take(&mut self.entries))"));
    assert!(aggregate.contains("release_evidence(self.state, self.evidence_bytes)"));

    let prepare = production
        .split("pub fn prepare_publication(")
        .nth(1)
        .expect("legacy publication prepare exists")
        .split("/// Captures immutable owner-neutral evidence")
        .next()
        .expect("legacy prepare boundary exists");
    let prepare_write = prepare
        .find("entry.pending_publication += commitment.amount")
        .expect("pending commitment write exists");
    assert!(prepare[..prepare_write].contains("validate_commitments"));
    for forbidden in [
        "?",
        "binary_search",
        "dimension_entry(",
        "dimension_entry_mut(",
        "try_reserve",
        ".reserve(",
        ".push(",
        ".insert(",
        "expect(",
        "unwrap",
        "panic!",
    ] {
        assert!(
            !prepare[prepare_write..].contains(forbidden),
            "publication prepare regained a lookup, allocation, or fault gate after its first counter write through `{forbidden}`"
        );
    }

    let publish = production
        .split("pub(crate) fn publish(")
        .nth(1)
        .expect("legacy publication publish exists")
        .split("fn ensure_open")
        .next()
        .expect("legacy publish boundary exists");
    let publish_write = publish
        .find("entry.pending_publication -= commitment.amount")
        .expect("published commitment write exists");
    assert!(publish[..publish_write].contains("commitment.dimension_position"));
    assert!(!publish.contains("dimension_entry("));
    assert!(!publish.contains("dimension_entry_mut("));

    let rollback = production
        .split("impl<O, P, D> Drop for PendingPublication")
        .nth(1)
        .expect("pending publication rollback exists")
        .split("fn lock")
        .next()
        .expect("pending publication rollback boundary exists");
    assert!(rollback.contains("get_mut(commitment.dimension_position)"));
    assert!(!rollback.contains("dimension_entry("));
    assert!(!rollback.contains("dimension_entry_mut("));
}

#[test]
fn typed_freeze_proof_owns_exact_batches_and_has_no_post_mutation_fault_gate() {
    let root = ledger_root_source();
    let commitment = ledger_commitment_source();
    let reservation_source = ledger_reservation_source();
    let snapshot_source = ledger_snapshot_source();
    let production = ledger_production_source();
    let proof = root
        .split("pub(crate) struct TypedFreezeProof")
        .nth(1)
        .expect("typed proof exists")
        .split("impl<O, P, D> TypedFreezeProof")
        .next()
        .expect("typed proof fields exist");
    assert!(proof.contains("canonical: AttributionReservationBatch"));
    assert!(proof.contains("evidence: AttributionReservationBatch"));
    assert!(proof.contains("ledger: Ledger"));
    assert!(!proof.contains("PhantomData"));
    assert!(
        !reservation_source.contains("TypedFreezeProof"),
        "reservation implementation must not own or construct terminal proofs"
    );
    assert!(!production.contains("struct TypedFreezeQualification"));
    assert!(production.contains("struct QualifiedTypedFreezeProof"));
    assert!(!production.contains("pub(crate) struct QualifiedTypedFreezeProof"));
    assert!(production.contains("    fn qualify_terminal(self)"));
    assert!(!production.contains("pub(crate) fn qualify_terminal"));
    assert!(!production.contains("pub(crate) fn try_into_qualified("));
    assert_eq!(
        production.matches("pub(crate) fn freeze_in_place").count(),
        1,
        "only the freeze-ready typed wrapper may expose in-place publication"
    );
    assert_eq!(
        production.matches("pub(crate) fn freeze(mut self)").count(),
        1,
        "only the freeze-ready typed wrapper may expose consuming publication"
    );
    let generic_qualified = root
        .split("impl<O, P, D> QualifiedTypedFreezeProof")
        .nth(1)
        .expect("generic qualified proof implementation exists")
        .split("impl<O, P, D> QualifiedEvidenceDiscardFreezeProof")
        .next()
        .expect("generic qualified proof implementation is bounded");
    assert!(generic_qualified.contains("    fn freeze_in_place(&mut self)"));
    assert!(generic_qualified.contains("    fn freeze(mut self)"));
    assert!(!generic_qualified.contains("pub(crate) fn freeze"));

    let qualification = root
        .split("fn validate_terminal(")
        .nth(1)
        .expect("typed terminal qualification exists")
        .split("pub(crate) fn retain_fail_closed")
        .next()
        .expect("typed terminal qualification boundary exists");
    assert!(qualification.contains("typed_terminal_snapshot_positions_are_valid"));
    assert_eq!(
        qualification
            .matches("bound_positions_are_in_bounds")
            .count(),
        5,
        "published and deferred-storage batches must prove attribution, cover, and commitment positions"
    );
    assert_eq!(
        qualification
            .matches("discard_positions_are_in_bounds")
            .count(),
        2,
        "canonical and evidence discards must prove only physically released attribution and cover positions"
    );
    assert_eq!(
        qualification
            .matches("positioned_commitment_amount")
            .count(),
        2,
        "canonical and evidence commitments must be requalified under the final lock"
    );
    for required in [
        "checked_add(canonical_increment)",
        "entry.policy.committed_limit",
        "committed_canonical_output > committed",
        "checked_add(outcome_evidence_increment)",
        "entry.policy.published_outcome_evidence_limit",
        "state.publication = PublicationState::TerminalQualified",
    ] {
        assert!(
            qualification.contains(required),
            "terminal qualification lost final commitment/state gate `{required}`"
        );
    }
    for forbidden in [
        "validate_final_snapshot_shape",
        "binary_search",
        "PartialEq",
    ] {
        assert!(
            !qualification.contains(forbidden),
            "self-consuming qualification calls owner-defined key behavior through `{forbidden}`"
        );
    }
    let bound_positions = reservation_source
        .split("fn bound_positions_are_in_bounds")
        .nth(1)
        .expect("bound-position qualification exists")
        .split("fn release_bound_attributions")
        .next()
        .expect("bound-position qualification boundary exists");
    for required in [
        "token.core.dimension_position < dimension_count",
        "token.core.phase_position < phase_count",
        "cover.core.dimension_position < dimension_count",
        "cover.core.phase_position < phase_count",
        "commitment.dimension_position < dimension_count",
    ] {
        assert!(
            bound_positions.contains(required),
            "terminal qualification lost bound-position gate `{required}`"
        );
    }
    let discard_positions = reservation_source
        .split("fn discard_positions_are_in_bounds")
        .nth(1)
        .expect("discard-position qualification exists")
        .split("fn release_bound_attributions")
        .next()
        .expect("discard-position qualification boundary exists");
    for required in [
        "token.core.dimension_position < dimension_count",
        "token.core.phase_position < phase_count",
        "cover.core.dimension_position < dimension_count",
        "cover.core.phase_position < phase_count",
    ] {
        assert!(
            discard_positions.contains(required),
            "discard qualification lost physical-accounting position gate `{required}`"
        );
    }
    assert!(
        !discard_positions.contains("commitments"),
        "discard qualification must not apply a payload commitment that is being destroyed"
    );

    let freeze = root
        .split("fn freeze_in_place(&mut self)")
        .nth(1)
        .expect("in-place typed freeze exists")
        .split("    fn freeze(mut self)")
        .next()
        .expect("in-place typed freeze body exists");
    let mutation_tail = freeze
        .split("let snapshot = {")
        .nth(1)
        .expect("typed mutation tail exists");
    for forbidden in [
        "Result",
        "?",
        "assert",
        "checked_",
        "binary_search",
        "saturating_",
        "unwrap",
        "expect",
        "unreachable",
    ] {
        assert!(
            !mutation_tail.contains(forbidden),
            "freeze contains post-mutation fault gate `{forbidden}`"
        );
    }
    assert_eq!(freeze.matches("lock(").count(), 1);
    let release = freeze
        .find("release_bound_attributions")
        .expect("freeze releases bound attributions");
    let discard_canonical = freeze
        .find("drop(proof.discard_canonical.take())")
        .expect("freeze drops discarded canonical batch");
    let discard_evidence = freeze
        .find("drop(proof.discard_evidence.take())")
        .expect("freeze drops discarded evidence batch");
    let commitments = freeze
        .find("apply_typed_commitments")
        .expect("freeze applies bound commitments");
    let destroy_storage = freeze
        .find("destroy_storage_then_release_bound_cover")
        .expect("freeze destroys batch storage before releasing its cover");
    let published = freeze
        .find("PublicationState::Published")
        .expect("freeze publishes atomically");
    let final_snapshot = freeze
        .find("take_typed_final_snapshot")
        .expect("freeze moves preallocated final evidence");
    assert!(discard_canonical < release);
    assert!(discard_evidence < release);
    assert!(release < commitments);
    assert!(commitments < destroy_storage);
    assert!(destroy_storage < published);
    assert!(published < final_snapshot);
    let after_snapshot = &freeze[final_snapshot + "take_typed_final_snapshot".len()..];
    assert!(!after_snapshot.contains("lock("));
    assert!(!after_snapshot.contains("release_bound_attributions"));
    assert!(!after_snapshot.contains("destroy_storage_then_release_bound_cover"));
    assert!(!after_snapshot.contains("apply_typed_commitments"));

    let bound_release = reservation_source
        .split("fn release_bound_locked")
        .nth(1)
        .expect("bound release helper exists")
        .split("impl<O, P, D> Drop for ReservationCore")
        .next()
        .expect("bound release helper body exists");
    let apply = commitment
        .split("fn apply_typed_commitments")
        .nth(1)
        .expect("bound commitment helper exists")
        .split_once("\n    }\n}\n")
        .expect("bound commitment helper has a closed function body")
        .0;
    let batch_release = reservation_source
        .split("fn release_bound_attributions")
        .nth(1)
        .expect("bound batch release helper exists")
        .split("fn destroy_storage_then_release_bound_cover")
        .next()
        .expect("bound batch release helper body exists");
    assert!(!batch_release.contains("capacity_cover.core.release_bound_locked"));
    let storage_release = reservation_source
        .split("fn destroy_storage_then_release_bound_cover")
        .nth(1)
        .expect("storage-before-cover helper exists");
    let attribution_storage = storage_release
        .find("take(&mut self.attributions)")
        .expect("attribution vector storage is destroyed");
    let commitment_storage = storage_release
        .find("take(&mut self.commitments)")
        .expect("commitment vector storage is destroyed");
    let capacity_release = storage_release
        .find("capacity_cover.core.release_bound_locked")
        .expect("capacity cover is released under the ledger lock");
    assert!(attribution_storage < capacity_release);
    assert!(commitment_storage < capacity_release);
    let snapshot = snapshot_source
        .split("fn take_typed_final_snapshot")
        .nth(1)
        .expect("typed snapshot helper exists")
        .split("const fn dimension_snapshot")
        .next()
        .expect("typed snapshot helper body exists");
    for (name, helper) in [
        ("bound release", bound_release),
        ("bound batch release", batch_release),
        ("commitment apply", apply),
        ("typed snapshot", snapshot),
    ] {
        for forbidden in [
            "Result",
            "?",
            "assert",
            "checked_",
            "binary_search",
            "saturating_",
            "unwrap",
            "expect",
            "unreachable",
            "lock(",
        ] {
            assert!(
                !helper.contains(forbidden),
                "{name} contains post-binding fault gate `{forbidden}`"
            );
        }
    }
    assert!(!freeze.contains("canonical:"));
    assert!(!freeze.contains("evidence:"));
}
