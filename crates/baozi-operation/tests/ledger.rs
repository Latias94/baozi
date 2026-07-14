use baozi_operation::{
    CancellationOrigin, CancellationSignal, CancellationToken, CapacityPlan, Commitment,
    ControlCause, CounterClass, DimensionPolicy, FinalProgress, LedgerCause, LedgerEvidenceError,
    LedgerEvidencePurpose, LedgerLimits, LedgerLimitsError, LedgerSnapshot, LiveReservation,
    OperationControl, OperationCoordinator, OperationOwner, OperationPoll, ProgressDecision,
    ProgressDeliveryLimits, ProgressEvent, ProgressObserver, ProgressPhase, ProgressUnit,
    PublicationCause, PublicationReservationBatch, PublicationReservationBatchErrorKind,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug)]
struct TestOwner;

impl OperationOwner for TestOwner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Snapshot,
    Decode,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ObserverPhase {
    Work,
    Publish,
}

impl ProgressPhase for ObserverPhase {
    fn order(self) -> u32 {
        match self {
            Self::Work => 0,
            Self::Publish => 1,
        }
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Publish => "publish",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    AssetBytes,
    OutputBytes,
    Records,
    Unknown,
}

type Coordinator = OperationCoordinator<TestOwner, Phase, ObserverPhase, Unit, Dimension>;
type Reservation = LiveReservation<TestOwner, Phase, Dimension>;
type PublicationBatch = PublicationReservationBatch<TestOwner, Phase, Dimension>;

fn publication_capacity_bytes() -> u64 {
    CapacityPlan::for_type::<Reservation>(0, 0, 1)
        .and_then(CapacityPlan::envelope_delta_bytes)
        .expect("one publication reservation must have a representable capacity envelope")
}

fn publication_limits(payload_live_bytes: u64) -> LedgerLimits<Dimension> {
    limits(
        payload_live_bytes
            .checked_add(publication_capacity_bytes())
            .expect("publication test live limit must be representable"),
    )
}

fn publication_coordinator(payload_live_bytes: u64) -> Coordinator {
    Coordinator::new(
        OperationControl::default(),
        publication_limits(payload_live_bytes),
    )
}

fn publication_batch(
    coordinator: &mut Coordinator,
    reservations: [Reservation; 1],
) -> PublicationBatch {
    let plan = CapacityPlan::for_type::<Reservation>(0, 0, reservations.len())
        .expect("publication batch capacity plan");
    let envelope = plan
        .envelope_delta_bytes()
        .expect("publication batch capacity envelope");
    let mut capacity_reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, envelope)
        .expect("publication batch capacity reservation");
    let mut values = Vec::new();
    plan.verify_preflight(values.len(), values.capacity())
        .expect("fresh publication batch capacity plan");
    values
        .try_reserve_exact(plan.allocator_additional())
        .expect("publication batch capacity allocation");
    let actual = plan
        .finish(values.len(), values.capacity())
        .expect("qualified publication batch capacity");
    coordinator
        .resize_reservation(&mut capacity_reservation, actual)
        .expect("publication batch actual capacity reservation");
    values.extend(reservations);
    PublicationReservationBatch::try_new(values, Some(capacity_reservation))
        .expect("publication batch allocation is covered")
}

trait SnapshotTestExt {
    fn ledger_snapshot(&self) -> LedgerSnapshot<Phase, Dimension>;
}

impl SnapshotTestExt for Coordinator {
    fn ledger_snapshot(&self) -> LedgerSnapshot<Phase, Dimension> {
        self.try_ledger_snapshot()
            .expect("test evidence snapshot must fit the configured bound")
    }
}

struct ReleasingCancellation {
    reservation: Arc<Mutex<Option<Reservation>>>,
}

impl CancellationSignal for ReleasingCancellation {
    fn is_cancelled(&self) -> bool {
        let reservation = self
            .reservation
            .lock()
            .expect("cancellation reservation lock")
            .take()
            .expect("cancellation reservation");
        drop(reservation);
        false
    }
}

fn limits(operation_live_bytes: u64) -> LedgerLimits<Dimension> {
    LedgerLimits::new(
        operation_live_bytes,
        [
            (
                Dimension::AssetBytes,
                DimensionPolicy::reservable_bytes(
                    10,
                    8,
                    0,
                    publication_capacity_bytes()
                        .checked_add(20)
                        .expect("test asset live limit must be representable"),
                ),
            ),
            (
                Dimension::OutputBytes,
                DimensionPolicy::reservable_bytes(64, 64, 16, 16),
            ),
            (Dimension::Records, DimensionPolicy::counter(32, 32, 8)),
        ],
    )
    .expect("valid frozen policy")
}

fn coordinator(operation_live_bytes: u64) -> Coordinator {
    Coordinator::new(OperationControl::default(), limits(operation_live_bytes))
}

fn run_with_timeout<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = work();
        let _ = sender.send(result);
    });
    match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => {
            worker.join().expect("operation worker");
            result
        }
        Err(error) => {
            drop(worker);
            panic!("operation callback blocked on an internal lock: {error}");
        }
    }
}

#[test]
fn ledger_limits_reject_invalid_or_duplicate_dimension_policy() {
    assert_eq!(
        LedgerLimits::new(
            4,
            [(Dimension::Records, DimensionPolicy::counter(3, 4, 0),)],
        ),
        Err(LedgerLimitsError::ConsumedLimitExceedsAttempted {
            dimension: Dimension::Records,
            consumed_limit: 4,
            attempted_limit: 3,
        })
    );

    assert_eq!(
        LedgerLimits::new(
            4,
            [
                (Dimension::Records, DimensionPolicy::counter(4, 4, 0)),
                (Dimension::Records, DimensionPolicy::counter(5, 5, 0)),
            ],
        ),
        Err(LedgerLimitsError::DuplicateDimension {
            dimension: Dimension::Records,
        })
    );
}

#[test]
fn attempted_and_consumed_are_non_refundable_and_preserve_their_invariant() {
    let mut coordinator = coordinator(32);
    let mut checkpoint = coordinator.checkpoint();

    checkpoint
        .charge_attempted(Dimension::AssetBytes, 8)
        .expect("first attempt fits");
    let attempted_error = checkpoint
        .charge_attempted(Dimension::AssetBytes, 4)
        .expect_err("crossing the frozen limit fails");
    assert_eq!(
        attempted_error,
        LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::AssetBytes,
            value: 12,
            limit: 10,
        }
    );

    checkpoint
        .charge_consumed(Dimension::AssetBytes, 5)
        .expect("first consumption fits");
    let consumed_error = checkpoint
        .charge_consumed(Dimension::AssetBytes, 4)
        .expect_err("crossing the frozen consumed limit fails");
    assert_eq!(
        consumed_error,
        LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::AssetBytes,
            value: 9,
            limit: 8,
        }
    );

    let invariant_error = checkpoint
        .charge_consumed(Dimension::AssetBytes, 4)
        .expect_err("consumption cannot outrun attempted work");
    assert_eq!(
        invariant_error,
        LedgerCause::ConsumedExceedsAttempted {
            dimension: Dimension::AssetBytes,
            consumed: 13,
            attempted: 12,
        }
    );
    let snapshot = coordinator.ledger_snapshot();
    let asset = snapshot
        .dimension(Dimension::AssetBytes)
        .expect("asset evidence");
    assert_eq!(asset.attempted(), 12);
    assert_eq!(asset.consumed(), 9);
    assert!(asset.consumed() <= asset.attempted());
}

#[test]
fn checked_counter_overflow_keeps_the_last_valid_evidence() {
    let limits = LedgerLimits::new(
        0,
        [(
            Dimension::Records,
            DimensionPolicy::counter(u64::MAX, u64::MAX, 0),
        )],
    )
    .expect("maximum counters are valid");
    let mut coordinator = Coordinator::new(OperationControl::default(), limits);
    let mut checkpoint = coordinator.checkpoint();

    checkpoint
        .charge_attempted(Dimension::Records, u64::MAX)
        .expect("maximum attempted value fits");
    assert_eq!(
        checkpoint.charge_attempted(Dimension::Records, 1),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
        })
    );
    checkpoint
        .charge_consumed(Dimension::Records, u64::MAX)
        .expect("maximum consumed value fits attempted");
    assert_eq!(
        checkpoint.charge_consumed(Dimension::Records, 1),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
        })
    );
    let evidence = coordinator
        .ledger_snapshot()
        .dimension(Dimension::Records)
        .copied()
        .expect("record evidence");
    assert_eq!(evidence.attempted(), u64::MAX);
    assert_eq!(evidence.consumed(), u64::MAX);
}

#[test]
fn unknown_and_counter_only_dimensions_reject_live_reservations() {
    let mut coordinator = coordinator(32);
    let mut checkpoint = coordinator.checkpoint();

    assert_eq!(
        checkpoint.charge_attempted(Dimension::Unknown, 1),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    );
    assert!(matches!(
        checkpoint.reserve(Phase::Snapshot, Dimension::Records, 1),
        Err(LedgerCause::DimensionNotReservable {
            dimension: Dimension::Records,
        })
    ));
}

#[test]
fn reservations_track_overlap_phase_peaks_transfer_and_raii_release() {
    let mut coordinator = coordinator(20);
    let snapshot_bytes = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("snapshot reservation");
    let mut decode_bytes = coordinator
        .checkpoint()
        .reserve(Phase::Decode, Dimension::AssetBytes, 7)
        .expect("decode reservation");

    let overlap = coordinator.ledger_snapshot();
    assert_eq!(overlap.current_live_bytes(), 15);
    assert_eq!(overlap.peak_live_bytes(), 15);
    assert_eq!(
        overlap
            .phase(Phase::Snapshot)
            .expect("snapshot phase")
            .current_live_bytes(),
        8
    );

    coordinator
        .transfer_reservation(&mut decode_bytes, Phase::Output)
        .expect("same-ledger phase transfer");
    let transferred = coordinator.ledger_snapshot();
    assert_eq!(
        transferred
            .phase(Phase::Decode)
            .expect("decode phase")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transferred
            .phase(Phase::Output)
            .expect("output phase")
            .current_live_bytes(),
        7
    );

    drop(snapshot_bytes);
    drop(decode_bytes);
    let released = coordinator.ledger_snapshot();
    assert_eq!(released.current_live_bytes(), 0);
    assert_eq!(released.peak_live_bytes(), 15);
    assert_eq!(
        released
            .phase(Phase::Output)
            .expect("output phase")
            .peak_live_bytes(),
        7
    );
}

#[test]
fn dimension_and_operation_live_limits_are_both_frozen_gates() {
    let mut dimension_limited = coordinator(64);
    let first = dimension_limited
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 12)
        .expect("first dimension reservation");
    assert!(matches!(
        dimension_limited
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 5),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 17,
            limit: 16,
        })
    ));
    drop(first);

    let mut operation_limited = coordinator(10);
    let first = operation_limited
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("first operation reservation");
    assert!(matches!(
        operation_limited
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 3),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 11,
            limit: 10,
        })
    ));
    drop(first);
}

#[test]
fn reservation_resize_tracks_peak_and_preserves_the_token_on_failure() {
    let mut coordinator = coordinator(12);
    let mut reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("initial reservation");

    coordinator
        .resize_reservation(&mut reservation, 10)
        .expect("reservation may grow before allocator entry");
    assert_eq!(reservation.bytes(), 10);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 10);
    assert_eq!(coordinator.ledger_snapshot().peak_live_bytes(), 10);

    coordinator
        .resize_reservation(&mut reservation, 6)
        .expect("unused envelope bytes may be released");
    let lowered = coordinator.ledger_snapshot();
    assert_eq!(reservation.bytes(), 6);
    assert_eq!(lowered.current_live_bytes(), 6);
    assert_eq!(lowered.peak_live_bytes(), 10);

    let before_failure = (
        lowered.current_live_bytes(),
        lowered.peak_live_bytes(),
        *lowered
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence"),
    );
    assert_eq!(
        coordinator.resize_reservation(&mut reservation, 13),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::AssetBytes,
            value: 13,
            limit: 12,
        })
    );
    assert_eq!(reservation.bytes(), 6);
    let after_failure = coordinator.ledger_snapshot();
    assert_eq!(
        (
            after_failure.current_live_bytes(),
            after_failure.peak_live_bytes(),
            *after_failure
                .dimension(Dimension::AssetBytes)
                .expect("asset evidence"),
        ),
        before_failure
    );

    drop(reservation);
    let released = coordinator.ledger_snapshot();
    assert_eq!(released.current_live_bytes(), 0);
    assert_eq!(released.peak_live_bytes(), 10);
}

#[test]
fn reservation_split_conserves_zero_exact_and_excess_bytes() {
    let mut coordinator = coordinator(20);
    let mut reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 10)
        .expect("initial reservation");

    let zero = coordinator
        .split_reservation(&mut reservation, 0)
        .expect("zero split is a no-op token");
    assert_eq!(zero.bytes(), 0);
    assert_eq!(reservation.bytes(), 10);
    drop(zero);

    assert!(matches!(
        coordinator.split_reservation(&mut reservation, 11),
        Err(LedgerCause::SplitExceedsReservation {
            requested: 11,
            available: 10,
        })
    ));
    assert_eq!(reservation.bytes(), 10);

    let exact = coordinator
        .split_reservation(&mut reservation, 10)
        .expect("exact split transfers all bytes");
    assert_eq!(reservation.bytes(), 0);
    assert_eq!(exact.bytes(), 10);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 10);
    drop(exact);
    drop(reservation);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn reservation_transition_atomically_changes_size_dimension_and_phase() {
    let mut coordinator = coordinator(32);
    let mut reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("source reservation");

    coordinator
        .transition_reservation(&mut reservation, Phase::Snapshot, Dimension::AssetBytes, 12)
        .expect("same-class growth");
    let grown = coordinator.ledger_snapshot();
    assert_eq!(grown.current_live_bytes(), 12);
    assert_eq!(grown.peak_live_bytes(), 12);
    assert_eq!(
        grown
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        12
    );

    coordinator
        .checkpoint()
        .transition_reservation(&mut reservation, Phase::Snapshot, Dimension::AssetBytes, 5)
        .expect("same-class shrink");
    let shrunk = coordinator.ledger_snapshot();
    assert_eq!(shrunk.current_live_bytes(), 5);
    assert_eq!(shrunk.peak_live_bytes(), 12);
    assert_eq!(
        shrunk
            .phase(Phase::Snapshot)
            .expect("snapshot phase")
            .current_live_bytes(),
        5
    );

    coordinator
        .transition_reservation(&mut reservation, Phase::Output, Dimension::OutputBytes, 10)
        .expect("cross-class transition");
    let transitioned = coordinator.ledger_snapshot();
    assert_eq!(transitioned.current_live_bytes(), 10);
    assert_eq!(transitioned.peak_live_bytes(), 12);
    assert_eq!(
        transitioned
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .peak_live_bytes(),
        12
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .current_live_bytes(),
        10
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .peak_live_bytes(),
        10
    );
    assert_eq!(
        transitioned
            .phase(Phase::Snapshot)
            .expect("snapshot phase")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .phase(Phase::Snapshot)
            .expect("snapshot phase")
            .peak_live_bytes(),
        12
    );
    assert_eq!(
        transitioned
            .phase(Phase::Output)
            .expect("output phase")
            .current_live_bytes(),
        10
    );
    assert_eq!(
        transitioned
            .phase(Phase::Output)
            .expect("output phase")
            .peak_live_bytes(),
        10
    );

    drop(reservation);
    let released = coordinator.ledger_snapshot();
    assert_eq!(released.current_live_bytes(), 0);
    assert_eq!(released.peak_live_bytes(), 12);
}

#[test]
fn failed_reservation_transition_preserves_source_without_target_pollution() {
    let mut dimension_limited = coordinator(32);
    let mut source = dimension_limited
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("dimension-limit source");
    assert_eq!(
        dimension_limited
            .transition_reservation(&mut source, Phase::Output, Dimension::OutputBytes, 17)
            .expect_err("target dimension limit must fail"),
        LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 17,
            limit: 16,
        }
    );
    let dimension_failure = dimension_limited.ledger_snapshot();
    assert_eq!(source.bytes(), 8);
    assert_eq!(dimension_failure.current_live_bytes(), 8);
    assert_eq!(dimension_failure.peak_live_bytes(), 8);
    assert_eq!(
        dimension_failure
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .peak_live_bytes(),
        0
    );
    assert!(dimension_failure.phase(Phase::Output).is_none());
    dimension_limited
        .transition_reservation(&mut source, Phase::Output, Dimension::OutputBytes, 16)
        .expect("preserved source can be retried");
    assert_eq!(source.bytes(), 16);
    drop(source);
    assert_eq!(dimension_limited.ledger_snapshot().current_live_bytes(), 0);

    let mut operation_limited = coordinator(10);
    let mut source = operation_limited
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("operation-limit source");
    assert_eq!(
        operation_limited
            .transition_reservation(&mut source, Phase::Output, Dimension::OutputBytes, 11)
            .expect_err("operation limit must fail"),
        LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 11,
            limit: 10,
        }
    );
    let operation_failure = operation_limited.ledger_snapshot();
    assert_eq!(source.bytes(), 8);
    assert_eq!(operation_failure.current_live_bytes(), 8);
    assert_eq!(operation_failure.peak_live_bytes(), 8);
    assert_eq!(
        operation_failure
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .peak_live_bytes(),
        0
    );
    assert!(operation_failure.phase(Phase::Output).is_none());
    drop(source);
    assert_eq!(operation_limited.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn failed_batch_transition_preserves_every_source_token_atomically() {
    let mut coordinator = coordinator(32);
    let first = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("first source reservation");
    let second = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("second source reservation");
    let mut sources = [first, second];

    assert_eq!(
        coordinator
            .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 17,)
            .expect_err("target dimension limit must reject the whole batch"),
        LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 17,
            limit: 16,
        }
    );
    assert_eq!(sources[0].bytes(), 4);
    assert_eq!(sources[1].bytes(), 4);
    let rejected = coordinator.ledger_snapshot();
    assert_eq!(rejected.current_live_bytes(), 8);
    assert_eq!(
        rejected
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        8
    );
    assert_eq!(
        rejected
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .peak_live_bytes(),
        0
    );
    assert!(rejected.phase(Phase::Output).is_none());

    coordinator
        .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 8)
        .expect("preserved batch can be retried");
    assert_eq!(sources[0].bytes(), 4);
    assert_eq!(sources[1].bytes(), 4);
    let transitioned = coordinator.ledger_snapshot();
    assert_eq!(transitioned.current_live_bytes(), 8);
    assert_eq!(
        transitioned
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .current_live_bytes(),
        8
    );

    drop(sources);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn mixed_batch_transition_conserves_operation_dimension_and_phase_totals() {
    let mut coordinator = coordinator(32);
    let first = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("asset source reservation");
    let second = coordinator
        .checkpoint()
        .reserve(Phase::Decode, Dimension::OutputBytes, 3)
        .expect("output source reservation");
    let mut sources = [first, second];

    coordinator
        .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 9)
        .expect("mixed batch transition should fit");

    assert_eq!(sources[0].bytes(), 4);
    assert_eq!(sources[1].bytes(), 5);
    let transitioned = coordinator.ledger_snapshot();
    assert_eq!(transitioned.current_live_bytes(), 9);
    assert_eq!(transitioned.peak_live_bytes(), 9);
    assert_eq!(
        transitioned
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .current_live_bytes(),
        9
    );
    assert_eq!(
        transitioned
            .phase(Phase::Snapshot)
            .expect("snapshot evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .phase(Phase::Decode)
            .expect("decode evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .phase(Phase::Output)
            .expect("output phase evidence")
            .current_live_bytes(),
        9
    );

    drop(sources);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn failed_batch_target_phase_evidence_admission_preserves_all_source_state() {
    let mut probe = coordinator(20);
    let probe_reservation = probe
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 1)
        .expect("one phase should be measurable");
    let one_phase_evidence_limit = probe.ledger_snapshot().current_evidence_bytes();
    drop(probe_reservation);
    probe
        .finish()
        .expect("the evidence probe should finalize cleanly");

    let tight = limits(20)
        .try_with_evidence_byte_limit(one_phase_evidence_limit)
        .expect("one source phase should fit exactly");
    let mut coordinator = Coordinator::new(OperationControl::default(), tight);
    let first = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
        .expect("first source reservation");
    let second = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
        .expect("second source reservation");
    let mut sources = [first, second];
    let before_dimension = coordinator
        .dimension_evidence(Dimension::AssetBytes)
        .expect("source dimension evidence");
    let before_phase = coordinator
        .phase_evidence(Phase::Snapshot)
        .expect("source phase evidence");

    let failure = coordinator
        .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 4)
        .expect_err("the second phase must exceed the exact evidence bound");

    assert!(matches!(
        failure,
        LedgerCause::Evidence {
            cause: LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::Phases,
                ..
            }
        }
    ));
    assert_eq!(sources[0].bytes(), 2);
    assert_eq!(sources[1].bytes(), 2);
    assert_eq!(
        coordinator
            .dimension_evidence(Dimension::AssetBytes)
            .expect("preserved source dimension evidence"),
        before_dimension
    );
    assert_eq!(
        coordinator.phase_evidence(Phase::Snapshot),
        Some(before_phase)
    );
    assert!(coordinator.phase_evidence(Phase::Output).is_none());

    drop(sources);
    coordinator
        .finish()
        .expect("preserved source reservations should finalize cleanly");
}

#[test]
fn batch_transition_rejects_owner_mismatch_and_pending_mutation_atomically() {
    let mut owner = coordinator(20);
    let mut other = coordinator(20);
    let first = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
        .expect("first owner reservation");
    let second = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
        .expect("second owner reservation");
    let mut sources = [first, second];
    assert_eq!(
        other
            .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 4)
            .expect_err("another ledger cannot transition the batch"),
        LedgerCause::OwnerMismatch
    );
    assert_eq!(sources[0].bytes(), 2);
    assert_eq!(sources[1].bytes(), 2);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 4);
    assert_eq!(other.ledger_snapshot().current_live_bytes(), 0);
    drop(sources);

    let first = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("first zero-byte pending fixture");
    let second = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("second zero-byte pending fixture");
    let mut zero_sources = [first, second];
    let mut batch = PublicationBatch::default();
    let pending = owner
        .prepare_publication(&mut batch, &[])
        .expect("zero live bytes are fully covered");
    assert_eq!(
        owner
            .transition_reservations(&mut zero_sources, Phase::Output, Dimension::OutputBytes, 3,)
            .expect_err("pending publication seals batch transitions"),
        LedgerCause::PublicationAlreadyPending
    );
    assert_eq!(zero_sources[0].bytes(), 0);
    assert_eq!(zero_sources[1].bytes(), 0);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 0);
    assert!(owner.phase_evidence(Phase::Output).is_none());
    drop(zero_sources);
    pending.rollback();
}

#[test]
fn reservation_transition_rejects_owner_mismatch_and_pending_mutation() {
    let mut owner = coordinator(20);
    let mut other = coordinator(20);
    let mut source = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 3)
        .expect("owner source");
    assert_eq!(
        other
            .transition_reservation(&mut source, Phase::Output, Dimension::OutputBytes, 3)
            .expect_err("another ledger cannot transition this token"),
        LedgerCause::OwnerMismatch
    );
    assert_eq!(source.bytes(), 3);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 3);
    assert_eq!(other.ledger_snapshot().current_live_bytes(), 0);
    drop(source);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 0);

    let mut zero = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("zero-byte pending fixture");
    let mut batch = PublicationBatch::default();
    let pending = owner
        .prepare_publication(&mut batch, &[])
        .expect("zero live bytes are fully covered");
    assert_eq!(
        owner
            .checkpoint()
            .transition_reservation(&mut zero, Phase::Output, Dimension::OutputBytes, 3)
            .expect_err("pending publication seals transitions"),
        LedgerCause::PublicationAlreadyPending
    );
    let pending_snapshot = owner.ledger_snapshot();
    assert_eq!(pending_snapshot.current_live_bytes(), 0);
    assert_eq!(pending_snapshot.peak_live_bytes(), 3);
    assert_eq!(
        pending_snapshot
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .peak_live_bytes(),
        0
    );
    assert!(pending_snapshot.phase(Phase::Output).is_none());
    assert_eq!(zero.bytes(), 0);
    drop(zero);
    pending.rollback();
}

#[test]
fn same_owner_type_cannot_transfer_across_ledger_instances() {
    let mut owner = publication_coordinator(20);
    let mut other = coordinator(20);
    let mut reservation = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 3)
        .expect("owner reservation");

    assert_eq!(
        other.transfer_reservation(&mut reservation, Phase::Decode),
        Err(LedgerCause::OwnerMismatch)
    );
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 3);
    assert_eq!(other.ledger_snapshot().current_live_bytes(), 0);

    let mut batch = publication_batch(&mut owner, [reservation]);
    assert!(matches!(
        other.prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 1)],),
        Err(LedgerCause::OwnerMismatch)
    ));
    assert_eq!(
        owner.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 3
    );
    assert_eq!(other.ledger_snapshot().current_live_bytes(), 0);
    drop(batch);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn publication_must_take_every_live_byte_or_commit_nothing() {
    let mut coordinator = publication_coordinator(20);
    let retained = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
        .expect("retained snapshot reservation");
    let submitted = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 3)
        .expect("submitted output reservation");

    let mut batch = publication_batch(&mut coordinator, [submitted]);
    assert!(matches!(
        coordinator.prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)],),
        Err(LedgerCause::PublicationLiveMismatch {
            transaction_bytes,
            ledger_bytes,
        }) if transaction_bytes == publication_capacity_bytes() + 3
            && ledger_bytes == publication_capacity_bytes() + 5
    ));
    let evidence = coordinator.ledger_snapshot();
    assert_eq!(
        evidence.current_live_bytes(),
        publication_capacity_bytes() + 5
    );
    assert_eq!(
        evidence
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    drop(batch);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 2);
    drop(retained);
}

#[test]
fn pending_and_published_boundaries_seal_all_accounting_mutation() {
    let mut coordinator = coordinator(20);
    let mut zero = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("zero-byte linear token");
    let mut first_batch = PublicationBatch::default();
    let pending = coordinator
        .prepare_publication(&mut first_batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("zero live bytes are fully covered");

    assert_eq!(
        coordinator
            .checkpoint()
            .charge_attempted(Dimension::Records, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert!(matches!(
        coordinator
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    assert_eq!(
        coordinator.transfer_reservation(&mut zero, Phase::Output),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert!(matches!(
        coordinator.split_reservation(&mut zero, 0),
        Err(LedgerCause::PublicationAlreadyPending)
    ));

    pending.rollback();
    coordinator
        .transfer_reservation(&mut zero, Phase::Output)
        .expect("rollback reopens accounting mutation");
    drop(zero);

    let mut replacement_batch = PublicationBatch::default();
    let mut pending = coordinator
        .prepare_publication(
            &mut replacement_batch,
            &[Commitment::new(Dimension::Records, 1)],
        )
        .expect("replacement transaction");
    coordinator
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("durable publication");
    assert_eq!(
        coordinator
            .checkpoint()
            .charge_attempted(Dimension::Records, 1),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
}

#[test]
fn pending_drop_rolls_back_live_and_commitments_then_allows_reprepare() {
    let mut coordinator = publication_coordinator(20);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 6)
        .expect("output reservation");
    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let pending = coordinator
        .prepare_publication(
            &mut batch,
            &[
                Commitment::new(Dimension::OutputBytes, 5),
                Commitment::new(Dimension::Records, 2),
            ],
        )
        .expect("pending output transaction");

    let before_drop = coordinator.ledger_snapshot();
    assert_eq!(
        before_drop.current_live_bytes(),
        publication_capacity_bytes() + 6
    );
    assert_eq!(
        before_drop
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .pending_publication(),
        5
    );
    assert_eq!(
        before_drop
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .committed(),
        0
    );

    drop(pending);
    let rolled_back = coordinator.ledger_snapshot();
    assert_eq!(rolled_back.current_live_bytes(), 0);
    assert_eq!(
        rolled_back
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .pending_publication(),
        0
    );
    assert_eq!(
        rolled_back
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );

    let replacement = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("replacement reservation");
    let mut replacement_batch = publication_batch(&mut coordinator, [replacement]);
    let mut replacement = coordinator
        .prepare_publication(
            &mut replacement_batch,
            &[Commitment::new(Dimension::Records, 1)],
        )
        .expect("rollback reopens publication admission");
    coordinator
        .publish(
            &mut replacement,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("replacement publishes");
    assert_eq!(
        coordinator
            .ledger_snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        1
    );
}

#[test]
fn publication_batch_releases_accounted_vector_capacity_on_rollback_and_publish() {
    let mut coordinator = publication_coordinator(20);
    let payload = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("payload reservation");
    let mut batch = publication_batch(&mut coordinator, [payload]);
    let pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("the batch owns every live byte");

    assert_eq!(
        coordinator.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 4
    );
    pending.rollback();
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);

    let payload = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 5)
        .expect("replacement payload reservation");
    let mut batch = publication_batch(&mut coordinator, [payload]);
    let mut pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("replacement batch owns every live byte");
    coordinator
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("publication releases both reservation classes");

    let snapshot = coordinator
        .finish()
        .expect("published capacity leaves no live token");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.peak_live_bytes(), publication_capacity_bytes() + 5);
    assert_eq!(
        snapshot
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        1
    );
}

#[test]
fn publication_batch_rejects_missing_or_insufficient_capacity_coverage() {
    let mut missing = publication_coordinator(8);
    let payload = missing
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("missing-capacity payload reservation");
    let error = PublicationReservationBatch::try_new(vec![payload], None)
        .expect_err("an allocated batch requires a capacity token");
    assert!(matches!(
        error.kind(),
        PublicationReservationBatchErrorKind::MissingCapacityReservation { required_bytes }
            if required_bytes == publication_capacity_bytes()
    ));
    drop(error);
    assert_eq!(missing.ledger_snapshot().current_live_bytes(), 0);

    let mut insufficient = publication_coordinator(8);
    let capacity = insufficient
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 1)
        .expect("insufficient capacity token");
    let payload = insufficient
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("insufficient-capacity payload reservation");
    let error = PublicationReservationBatch::try_new(vec![payload], Some(capacity))
        .expect_err("an undersized capacity token cannot cover the batch allocation");
    assert!(matches!(
        error.kind(),
        PublicationReservationBatchErrorKind::InsufficientCapacityReservation {
            required_bytes,
            reserved_bytes: 1,
        } if required_bytes == publication_capacity_bytes()
    ));
    drop(error);
    assert_eq!(insufficient.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn rejected_publication_batch_releases_its_complete_owner_during_unwind() {
    let mut coordinator = publication_coordinator(8);
    let capacity = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 1)
        .expect("rejected batch capacity token");
    let payload = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("rejected batch payload token");

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _error = PublicationReservationBatch::try_new(vec![payload], Some(capacity))
            .expect_err("the capacity token deliberately under-covers the vector");
        panic!("exercise rejected batch unwind ownership");
    }));

    assert!(unwind.is_err());
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn publication_aggregates_duplicate_commitments_atomically() {
    let mut coordinator = publication_coordinator(20);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 9)
        .expect("output reservation");
    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let mut pending = coordinator
        .prepare_publication(
            &mut batch,
            &[
                Commitment::new(Dimension::OutputBytes, 3),
                Commitment::new(Dimension::OutputBytes, 4),
                Commitment::new(Dimension::Records, 2),
            ],
        )
        .expect("duplicate dimensions aggregate");

    coordinator
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 9),
        )
        .expect("atomic publication");

    let snapshot = coordinator
        .finish()
        .expect("publishing every live token makes finalization infallible");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.peak_live_bytes(), publication_capacity_bytes() + 9);
    assert_eq!(
        snapshot
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .committed(),
        7
    );
    assert_eq!(
        snapshot
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        2
    );
}

#[test]
fn commitment_failures_preserve_live_batch_for_retry() {
    let overflow_limits = LedgerLimits::new(
        8 + publication_capacity_bytes(),
        [
            (
                Dimension::AssetBytes,
                DimensionPolicy::reservable_bytes(0, 0, 0, publication_capacity_bytes()),
            ),
            (
                Dimension::OutputBytes,
                DimensionPolicy::reservable_bytes(8, 8, u64::MAX, 8),
            ),
        ],
    )
    .expect("overflow fixture policy");
    let mut overflow = Coordinator::new(OperationControl::default(), overflow_limits);
    let reservation = overflow
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("overflow fixture reservation");
    let mut batch = publication_batch(&mut overflow, [reservation]);
    assert!(matches!(
        overflow.prepare_publication(
            &mut batch,
            &[
                Commitment::new(Dimension::OutputBytes, u64::MAX),
                Commitment::new(Dimension::OutputBytes, 1),
            ],
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::PendingPublication,
            dimension: Dimension::OutputBytes,
        })
    ));
    assert_eq!(
        overflow.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 1
    );
    assert!(!batch.is_empty());
    let pending = overflow
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 1)])
        .expect("overflow failure leaves the batch retryable");
    assert!(batch.is_empty());
    pending.rollback();
    assert_eq!(overflow.ledger_snapshot().current_live_bytes(), 0);

    let mut unknown = publication_coordinator(20);
    let reservation = unknown
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("unknown fixture reservation");
    let mut batch = publication_batch(&mut unknown, [reservation]);
    assert!(matches!(
        unknown.prepare_publication(&mut batch, &[Commitment::new(Dimension::Unknown, 1)],),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert_eq!(
        unknown.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 1
    );
    assert!(!batch.is_empty());
    let pending = unknown
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("unknown-dimension failure leaves the batch retryable");
    pending.rollback();
    assert_eq!(unknown.ledger_snapshot().current_live_bytes(), 0);

    let mut limited = publication_coordinator(20);
    let reservation = limited
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("limit fixture reservation");
    let mut batch = publication_batch(&mut limited, [reservation]);
    assert!(matches!(
        limited.prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 17)],),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Committed,
            dimension: Dimension::OutputBytes,
            value: 17,
            limit: 16,
        })
    ));
    let evidence = limited.ledger_snapshot();
    assert_eq!(
        evidence.current_live_bytes(),
        publication_capacity_bytes() + 1
    );
    assert_eq!(
        evidence
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .committed(),
        0
    );
    assert!(!batch.is_empty());
    let pending = limited
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 1)])
        .expect("commitment-limit failure leaves the batch retryable");
    pending.rollback();
    assert_eq!(limited.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn publication_evidence_failure_preserves_live_batch_for_retry() {
    let mut probe = publication_coordinator(20);
    let probe_reservation = probe
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("publication evidence probe reservation");
    let probe_batch = publication_batch(&mut probe, [probe_reservation]);
    let prepublication_evidence_limit = probe.ledger_snapshot().current_evidence_bytes();
    drop(probe_batch);
    probe
        .finish()
        .expect("the evidence probe should finalize cleanly");

    let tight_limits = publication_limits(20)
        .try_with_evidence_byte_limit(prepublication_evidence_limit)
        .expect("dimension and one phase evidence fit exactly");
    let mut coordinator = Coordinator::new(OperationControl::default(), tight_limits);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("the prepublication envelope should fit");
    let mut batch = publication_batch(&mut coordinator, [reservation]);

    assert!(matches!(
        coordinator.prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 1)],),
        Err(LedgerCause::Evidence {
            cause: LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::Publication,
                ..
            }
        })
    ));
    assert!(!batch.is_empty());
    let output = coordinator
        .dimension_evidence(Dimension::OutputBytes)
        .expect("output evidence");
    assert_eq!(output.current_live_bytes(), 1);
    assert_eq!(output.pending_publication(), 0);

    let pending = coordinator
        .prepare_publication(&mut batch, &[])
        .expect("the preserved batch remains retryable without commitment evidence");
    assert!(batch.is_empty());
    pending.rollback();
    assert_eq!(
        coordinator
            .dimension_evidence(Dimension::OutputBytes)
            .expect("rolled-back output evidence")
            .current_live_bytes(),
        0
    );
}

#[test]
fn only_one_pending_or_published_batch_is_admitted() {
    let mut coordinator = coordinator(20);
    let mut first_batch = PublicationBatch::default();
    let pending = coordinator
        .prepare_publication(&mut first_batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("first pending batch");
    let mut blocked_batch = PublicationBatch::default();
    assert!(matches!(
        coordinator.prepare_publication(&mut blocked_batch, &[]),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    pending.rollback();

    let mut pending = coordinator
        .prepare_publication(
            &mut blocked_batch,
            &[Commitment::new(Dimension::Records, 1)],
        )
        .expect("rollback permits a replacement batch");
    coordinator
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("first durable batch");
    let mut completed_batch = PublicationBatch::default();
    assert!(matches!(
        coordinator.prepare_publication(
            &mut completed_batch,
            &[Commitment::new(Dimension::Unknown, u64::MAX)],
        ),
        Err(LedgerCause::PublicationAlreadyCompleted)
    ));
}

#[test]
fn wrong_coordinator_rejects_pending_before_progress_and_preserves_it_for_owner() {
    use std::sync::{Arc, Mutex};

    let observed = Arc::new(Mutex::new(0usize));
    let observed_by_other = Arc::clone(&observed);
    let mut owner = publication_coordinator(20);
    let mut other = Coordinator::new(
        OperationControl::default().with_progress_observer(move |_| {
            *observed_by_other.lock().expect("observer lock") += 1;
            baozi_operation::ProgressDecision::Continue
        }),
        limits(20),
    );
    let reservation = owner
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 3)
        .expect("owner reservation");
    let mut batch = publication_batch(&mut owner, [reservation]);
    let mut pending = owner
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("owner pending transaction");

    assert_eq!(
        other.publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        ),
        Err(PublicationCause::Ledger(LedgerCause::OwnerMismatch))
    );
    assert_eq!(*observed.lock().expect("observer lock"), 0);
    assert_eq!(
        owner.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 3
    );
    assert_eq!(
        owner
            .ledger_snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    owner
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("the owning coordinator can publish the preserved transaction");
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn progress_observers_can_release_same_ledger_reservations_without_blocking() {
    let reservation_slot = Arc::new(Mutex::new(None::<Reservation>));
    let reservation_from_observer = Arc::clone(&reservation_slot);
    let observer = move |_| {
        let reservation = reservation_from_observer
            .lock()
            .expect("observer reservation lock")
            .take()
            .expect("observer reservation");
        drop(reservation);
        ProgressDecision::Continue
    };
    let mut coordinator = Coordinator::new(
        OperationControl::default().with_progress_observer(observer),
        limits(20),
    );
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("observer reservation");
    *reservation_slot.lock().expect("reservation slot") = Some(reservation);

    let current_live_bytes = run_with_timeout(move || {
        coordinator
            .report_progress(ObserverPhase::Work, Unit::Work, 1, Some(1))
            .expect("observer progress");
        coordinator.ledger_snapshot().current_live_bytes()
    });
    assert_eq!(current_live_bytes, 0);

    let final_slot = Arc::new(Mutex::new(None::<Reservation>));
    let reservation_from_final_observer = Arc::clone(&final_slot);
    let final_observer = move |_| {
        let reservation = reservation_from_final_observer
            .lock()
            .expect("final observer reservation lock")
            .take()
            .expect("final observer reservation");
        drop(reservation);
        ProgressDecision::Continue
    };
    let mut coordinator = Coordinator::new(
        OperationControl::default().with_progress_observer(final_observer),
        limits(20),
    );
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("zero-byte final observer reservation");
    *final_slot.lock().expect("final reservation slot") = Some(reservation);
    let mut batch = PublicationBatch::default();
    let mut pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("final observer pending publication");

    let committed = run_with_timeout(move || {
        coordinator
            .publish(
                &mut pending,
                FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
            )
            .expect("final observer publication");
        coordinator
            .ledger_snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed()
    });
    assert_eq!(committed, 1);
}

#[test]
fn cancellation_callback_can_release_same_ledger_reservation_without_blocking() {
    let reservation_slot = Arc::new(Mutex::new(None::<Reservation>));
    let control = OperationControl::default().with_cancellation(ReleasingCancellation {
        reservation: Arc::clone(&reservation_slot),
    });
    let mut coordinator = Coordinator::new(control, limits(20));
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("cancellation reservation");
    *reservation_slot.lock().expect("reservation slot") = Some(reservation);

    let current_live_bytes = run_with_timeout(move || {
        coordinator.poll().expect("cancellation poll");
        coordinator.ledger_snapshot().current_live_bytes()
    });
    assert_eq!(current_live_bytes, 0);
}

#[test]
fn final_observer_cancel_and_panic_roll_back_before_durable_commit() {
    assert_final_control_failure(
        |_| ProgressDecision::Cancel,
        ControlCause::Cancelled {
            origin: CancellationOrigin::Observer,
        },
    );
    assert_final_control_failure(
        |_| panic!("final observer panic"),
        ControlCause::ObserverPanicked,
    );
}

#[test]
fn final_observer_decision_bypasses_exhausted_transient_delivery_limits() {
    let cases = [
        (
            "zero event quota with an identical suppressed final report",
            ProgressDeliveryLimits::new(0, usize::MAX, 1),
            Some(ObserverPhase::Publish),
            vec![ObserverPhase::Publish],
        ),
        (
            "exhausted event quota",
            ProgressDeliveryLimits::new(1, usize::MAX, 1),
            Some(ObserverPhase::Work),
            vec![ObserverPhase::Work, ObserverPhase::Publish],
        ),
        (
            "zero encoded-byte quota",
            ProgressDeliveryLimits::new(usize::MAX, 0, 1),
            None,
            vec![ObserverPhase::Publish],
        ),
    ];

    for (case, delivery_limits, prior_phase, expected_deliveries) in cases {
        let deliveries = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let deliveries_from_observer = std::sync::Arc::clone(&deliveries);
        let observer = move |event: ProgressEvent<ObserverPhase, Unit>| {
            deliveries_from_observer
                .lock()
                .expect("delivery lock")
                .push(event.phase());
            if event.phase() == ObserverPhase::Publish {
                ProgressDecision::Cancel
            } else {
                ProgressDecision::Continue
            }
        };
        let control = OperationControl::default()
            .with_progress_observer(observer)
            .with_progress_limits(delivery_limits);
        let mut coordinator = Coordinator::new(control, publication_limits(20));

        if let Some(prior_phase) = prior_phase {
            let (completed, total) = match prior_phase {
                ObserverPhase::Work => (1, 1),
                ObserverPhase::Publish => (3, 3),
            };
            coordinator
                .report_progress(prior_phase, Unit::Work, completed, Some(total))
                .unwrap_or_else(|error| panic!("{case}: prior progress failed: {error}"));
        }

        let reservation = coordinator
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 3)
            .unwrap_or_else(|error| panic!("{case}: output reservation failed: {error}"));
        let mut batch = publication_batch(&mut coordinator, [reservation]);
        let mut pending = coordinator
            .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
            .unwrap_or_else(|error| panic!("{case}: pending publication failed: {error}"));

        assert_eq!(
            coordinator.publish(
                &mut pending,
                FinalProgress::new(ObserverPhase::Publish, Unit::Work, 3),
            ),
            Err(PublicationCause::Control(ControlCause::Cancelled {
                origin: CancellationOrigin::Observer,
            })),
            "{case}"
        );
        assert_eq!(
            deliveries.lock().expect("delivery lock").as_slice(),
            expected_deliveries,
            "{case}"
        );
        let snapshot = coordinator.ledger_snapshot();
        assert_eq!(
            snapshot.current_live_bytes(),
            publication_capacity_bytes() + 3,
            "{case}"
        );
        assert_eq!(
            snapshot
                .dimension(Dimension::Records)
                .expect("record evidence")
                .pending_publication(),
            1,
            "{case}"
        );
        assert_eq!(
            snapshot
                .dimension(Dimension::Records)
                .expect("record evidence")
                .committed(),
            0,
            "{case}"
        );
        drop(pending);
        let rolled_back = coordinator.ledger_snapshot();
        assert_eq!(rolled_back.current_live_bytes(), 0, "{case}");
        assert_eq!(
            rolled_back
                .dimension(Dimension::Records)
                .expect("record evidence")
                .pending_publication(),
            0,
            "{case}"
        );
    }
}

#[test]
fn final_observer_receives_publication_event_after_identical_transient_delivery() {
    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let observer_deliveries = Arc::clone(&deliveries);
    let observer = move |event: ProgressEvent<ObserverPhase, Unit>| {
        let mut deliveries = observer_deliveries.lock().expect("delivery lock");
        deliveries.push((event.phase(), event.completed(), event.total()));
        if deliveries.len() == 2 {
            ProgressDecision::Cancel
        } else {
            ProgressDecision::Continue
        }
    };
    let mut coordinator = Coordinator::new(
        OperationControl::default().with_progress_observer(observer),
        publication_limits(20),
    );

    coordinator
        .report_progress(ObserverPhase::Publish, Unit::Work, 3, Some(3))
        .expect("transient completion report");
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 3)
        .expect("output reservation");
    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let mut pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("pending publication");

    assert_eq!(
        coordinator.publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 3),
        ),
        Err(PublicationCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Observer,
        }))
    );
    assert_eq!(
        deliveries.lock().expect("delivery lock").as_slice(),
        [
            (ObserverPhase::Publish, 3, Some(3)),
            (ObserverPhase::Publish, 3, Some(3)),
        ]
    );
    let evidence = coordinator.ledger_snapshot();
    assert_eq!(
        evidence.current_live_bytes(),
        publication_capacity_bytes() + 3
    );
    assert_eq!(
        evidence
            .dimension(Dimension::Records)
            .expect("record evidence")
            .pending_publication(),
        1
    );
    assert_eq!(
        evidence
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    drop(pending);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

fn assert_final_control_failure(
    observer: impl ProgressObserver<ObserverPhase, Unit>,
    expected: ControlCause,
) {
    let mut coordinator = Coordinator::new(
        OperationControl::default().with_progress_observer(observer),
        publication_limits(20),
    );
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 5)
        .expect("output reservation");
    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let mut pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 3)])
        .expect("pending transaction");

    assert_eq!(
        coordinator.publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 5),
        ),
        Err(PublicationCause::Control(expected.clone()))
    );
    let evidence = coordinator.ledger_snapshot();
    assert_eq!(
        evidence.current_live_bytes(),
        publication_capacity_bytes() + 5
    );
    let records = evidence
        .dimension(Dimension::Records)
        .expect("record evidence");
    assert_eq!(records.pending_publication(), 3);
    assert_eq!(records.committed(), 0);
    drop(pending);
    let rolled_back = coordinator.ledger_snapshot();
    assert_eq!(rolled_back.current_live_bytes(), 0);
    assert_eq!(
        rolled_back
            .dimension(Dimension::Records)
            .expect("record evidence")
            .pending_publication(),
        0
    );
    assert_eq!(coordinator.poll(), Err(expected));
}

#[test]
fn final_callback_can_run_nested_work_and_last_poll_observes_its_cancellation() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let cancellation = CancellationToken::new();
    let cancellation_from_observer = cancellation.clone();
    let nested_runs = Arc::new(AtomicUsize::new(0));
    let nested_runs_from_observer = Arc::clone(&nested_runs);
    let observer = move |_| {
        let mut nested = coordinator(0);
        nested.poll().expect("nested control remains independent");
        nested_runs_from_observer.fetch_add(1, Ordering::SeqCst);
        cancellation_from_observer.cancel();
        ProgressDecision::Continue
    };
    let control = OperationControl::default()
        .with_cancellation(cancellation)
        .with_progress_observer(observer);
    let mut coordinator = Coordinator::new(control, publication_limits(20));
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("output reservation");
    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let mut pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("pending transaction");

    assert_eq!(
        coordinator.publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 4),
        ),
        Err(PublicationCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        }))
    );
    assert_eq!(nested_runs.load(Ordering::SeqCst), 1);
    let evidence = coordinator.ledger_snapshot();
    assert_eq!(
        evidence.current_live_bytes(),
        publication_capacity_bytes() + 4
    );
    assert_eq!(
        evidence
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    drop(pending);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn identical_transient_or_invalid_final_progress_cannot_bypass_the_publication_gate() {
    use std::sync::{Arc, Mutex};

    let deliveries = Arc::new(Mutex::new(0usize));
    let deliveries_from_observer = Arc::clone(&deliveries);
    let observer = move |_| {
        *deliveries_from_observer.lock().expect("delivery lock") += 1;
        ProgressDecision::Continue
    };
    let mut duplicate = Coordinator::new(
        OperationControl::default().with_progress_observer(observer),
        limits(20),
    );
    duplicate
        .report_progress(ObserverPhase::Publish, Unit::Work, 5, Some(5))
        .expect("transient progress delivery");
    let mut batch = PublicationBatch::default();
    let mut pending = duplicate
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("pending transaction");
    duplicate
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 5),
        )
        .expect("identical publication progress is delivered before publication");
    assert_eq!(*deliveries.lock().expect("delivery lock"), 2);

    let mut invalid = publication_coordinator(20);
    invalid
        .report_progress(ObserverPhase::Publish, Unit::Work, 5, Some(5))
        .expect("higher prior progress");
    let reservation = invalid
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 3)
        .expect("output reservation");
    let mut batch = publication_batch(&mut invalid, [reservation]);
    let mut pending = invalid
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("pending transaction");
    assert!(matches!(
        invalid.publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 4),
        ),
        Err(PublicationCause::Control(
            ControlCause::InvalidProgress { .. }
        ))
    ));
    let evidence = invalid.ledger_snapshot();
    assert_eq!(
        evidence.current_live_bytes(),
        publication_capacity_bytes() + 3
    );
    assert_eq!(
        evidence
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    drop(pending);
    assert_eq!(invalid.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn finish_error_preserves_real_live_and_pending_evidence() {
    let mut live_coordinator = coordinator(20);
    let live = live_coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 7)
        .expect("live fixture reservation");
    let live_error = live_coordinator
        .finish()
        .expect_err("live bytes prevent clean finalization");
    assert_eq!(
        live_error.cause(),
        &LedgerCause::LiveReservationsRemain { bytes: 7 }
    );
    assert_eq!(live_error.snapshot().current_live_bytes(), 7);
    assert_eq!(live_error.snapshot().peak_live_bytes(), 7);
    drop(live);

    let mut combined_coordinator = publication_coordinator(20);
    let combined_live = combined_coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("combined fixture reservation");
    let mut combined_batch = publication_batch(&mut combined_coordinator, [combined_live]);
    let combined_pending = combined_coordinator
        .prepare_publication(
            &mut combined_batch,
            &[Commitment::new(Dimension::Records, 2)],
        )
        .expect("combined pending transaction");
    let combined_error = combined_coordinator
        .finish()
        .expect_err("pending live output prevents finalization");
    assert_eq!(
        combined_error.snapshot().current_live_bytes(),
        publication_capacity_bytes() + 4
    );
    assert_eq!(
        combined_error
            .snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .pending_publication(),
        2
    );
    assert_eq!(
        combined_error
            .snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    drop(combined_pending);

    let mut pending_coordinator = coordinator(20);
    let mut pending_batch = PublicationBatch::default();
    let pending = pending_coordinator
        .prepare_publication(&mut pending_batch, &[])
        .expect("even an empty transaction has linear pending state");
    let pending_error = pending_coordinator
        .finish()
        .expect_err("empty pending transaction cannot disappear from evidence");
    let (cause, snapshot) = pending_error.into_parts();
    assert_eq!(cause, LedgerCause::PendingPublicationsRemain);
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert!(
        snapshot
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .pending_publication()
            == 0
    );
    drop(pending);
}

#[test]
fn checkpoint_exposes_control_and_accounting_without_state_authority() {
    let mut coordinator = coordinator(20);
    {
        let mut checkpoint = coordinator.checkpoint();
        poll_core_work(&mut checkpoint).expect("object-safe polling remains active");
        checkpoint
            .report_progress(ObserverPhase::Work, Unit::Work, 1, None)
            .expect("owner-defined progress");
        checkpoint
            .charge_attempted(Dimension::AssetBytes, 2)
            .expect("checked attempted work");
        let mut reservation = checkpoint
            .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
            .expect("checked storage");
        let split = checkpoint
            .split(&mut reservation, 1)
            .expect("checkpoint may partition live ownership");
        drop(split);
        drop(reservation);
    }

    let snapshot = coordinator.ledger_snapshot();
    assert_eq!(
        snapshot
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .attempted(),
        2
    );
    assert_eq!(snapshot.current_live_bytes(), 0);
}

#[test]
fn frozen_and_phase_evidence_obey_the_independent_hard_limit() {
    assert!(matches!(
        LedgerLimits::with_evidence_byte_limit(
            20,
            0,
            [(
                Dimension::AssetBytes,
                DimensionPolicy::reservable_bytes(20, 20, 0, 20),
            )],
        ),
        Err(LedgerLimitsError::Evidence {
            cause: LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::Dimensions,
                ..
            },
        })
    ));

    let frozen = limits(20);
    let frozen_bytes = frozen.frozen_evidence_bytes();
    let tight = frozen
        .try_with_evidence_byte_limit(frozen_bytes)
        .expect("the frozen allocation exactly fits its own bound");
    let mut coordinator = Coordinator::new(OperationControl::default(), tight);
    assert!(matches!(
        coordinator
            .checkpoint()
            .reserve(Phase::Snapshot, Dimension::AssetBytes, 1),
        Err(LedgerCause::Evidence {
            cause: LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::Phases,
                ..
            },
        })
    ));
    let snapshot = coordinator
        .finish()
        .expect("failed metadata admission leaves no live reservation");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.current_evidence_bytes(), 0);
    assert_eq!(snapshot.peak_evidence_bytes(), frozen_bytes);
}

#[test]
fn zero_byte_reservation_capabilities_must_be_released_before_clean_finalization() {
    let mut coordinator = coordinator(20);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("zero-byte reservation remains a linear capability");
    let error = coordinator
        .finish()
        .expect_err("an active capability retains ledger state even without payload bytes");
    assert_eq!(
        error.cause(),
        &LedgerCause::ActiveReservationsRemain { count: 1 }
    );
    assert_eq!(error.snapshot().current_live_bytes(), 0);
    assert_eq!(error.snapshot().active_reservations(), 1);
    assert!(error.snapshot().current_evidence_bytes() > 0);
    drop(reservation);
}

#[test]
fn evidence_peak_covers_snapshot_and_publication_storage_then_finishes_at_zero() {
    let mut coordinator = publication_coordinator(20);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("output reservation");
    let before_publication = coordinator
        .try_ledger_snapshot()
        .expect("bounded interim evidence");
    assert!(before_publication.current_evidence_bytes() > 0);
    assert!(before_publication.peak_evidence_bytes() > before_publication.current_evidence_bytes());

    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let before_prepare = coordinator
        .try_ledger_snapshot()
        .expect("accounted publication batch evidence");
    let pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 4)])
        .expect("bounded sorted commitment evidence");
    let while_pending = coordinator
        .try_ledger_snapshot()
        .expect("pending evidence snapshot");
    assert!(while_pending.current_evidence_bytes() > before_prepare.current_evidence_bytes());
    drop(pending);
    let after_rollback = coordinator
        .try_ledger_snapshot()
        .expect("rollback evidence snapshot");
    assert_eq!(
        after_rollback.current_evidence_bytes(),
        before_prepare.current_evidence_bytes()
    );
    let snapshot = coordinator.finish().expect("clean final evidence");
    assert_eq!(snapshot.current_evidence_bytes(), 0);
    assert!(
        snapshot.peak_evidence_bytes() >= while_pending.peak_evidence_bytes(),
        "publication and snapshot envelopes remain visible in the high-water mark"
    );
}

#[test]
fn snapshot_iteration_is_stably_sorted_after_out_of_order_policy_and_phase_use() {
    let limits = LedgerLimits::new(
        20,
        [
            (Dimension::Records, DimensionPolicy::counter(8, 8, 8)),
            (
                Dimension::OutputBytes,
                DimensionPolicy::reservable_bytes(20, 20, 20, 20),
            ),
            (
                Dimension::AssetBytes,
                DimensionPolicy::reservable_bytes(20, 20, 20, 20),
            ),
        ],
    )
    .expect("out-of-order policy is valid");
    let mut coordinator = Coordinator::new(OperationControl::default(), limits);
    let output = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("output phase");
    let snapshot = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 1)
        .expect("snapshot phase");
    drop(output);
    drop(snapshot);
    let evidence = coordinator.finish().expect("sorted final evidence");
    assert_eq!(
        evidence
            .dimensions()
            .map(|(dimension, _)| dimension)
            .collect::<Vec<_>>(),
        vec![
            Dimension::AssetBytes,
            Dimension::OutputBytes,
            Dimension::Records,
        ]
    );
    assert_eq!(
        evidence
            .phases()
            .map(|(phase, _)| phase)
            .collect::<Vec<_>>(),
        vec![Phase::Snapshot, Phase::Output]
    );
}

fn poll_core_work(poll: &mut dyn OperationPoll) -> Result<(), baozi_operation::ControlCause> {
    poll.poll()
}
