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

const COMPLETE_PHASES: &[Phase] = &[Phase::Snapshot, Phase::Decode, Phase::Output];

fn coordinator_with(
    control: OperationControl<ObserverPhase, Unit>,
    limits: LedgerLimits<Dimension>,
) -> Coordinator {
    Coordinator::try_new(control, limits, COMPLETE_PHASES).expect("complete test phase catalog")
}

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
    coordinator_with(
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
    coordinator_with(OperationControl::default(), limits(operation_live_bytes))
}

fn coordinator_with_tight_evidence_limit(
    mut build_limits: impl FnMut() -> LedgerLimits<Dimension>,
) -> Coordinator {
    let mut evidence_limit = 0;
    for _ in 0..16 {
        let limits = build_limits();
        evidence_limit = evidence_limit.max(limits.frozen_evidence_bytes());
        let limits = limits
            .try_with_evidence_byte_limit(evidence_limit)
            .expect("the frozen dimension evidence must fit its observed bound");
        match Coordinator::try_new(OperationControl::default(), limits, COMPLETE_PHASES) {
            Ok(coordinator) => return coordinator,
            Err(LedgerCause::Evidence {
                cause:
                    LedgerEvidenceError::LimitExceeded {
                        purpose: LedgerEvidencePurpose::Phases,
                        value,
                        ..
                    },
            }) => evidence_limit = value,
            Err(cause) => panic!("unexpected tight evidence qualification failure: {cause}"),
        }
    }
    panic!("the tiny test catalog did not converge to its admitted evidence envelope")
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

#[path = "ledger/accounting.rs"]
mod accounting;
#[path = "ledger/control.rs"]
mod control;
#[path = "ledger/evidence.rs"]
mod evidence;
#[path = "ledger/publication.rs"]
mod publication;
#[path = "ledger/reservation.rs"]
mod reservation;
