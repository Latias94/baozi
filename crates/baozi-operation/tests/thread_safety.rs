use baozi_operation::{
    CancellationOrigin, CancellationToken, Commitment, ControlCause, CounterClass, DimensionPolicy,
    DimensionSnapshot, FinalProgress, LedgerCause, LedgerEvidenceError, LedgerEvidencePurpose,
    LedgerFinishError, LedgerLimits, LedgerLimitsError, LedgerSnapshot, LiveReservation,
    OperationControl, OperationCoordinator, OperationOwner, PendingPublication, PhaseSnapshot,
    ProgressDecision, ProgressDeliveryLimits, ProgressEvent, ProgressPhase, ProgressStorageError,
    ProgressUnit, ProgressViolation, PublicationCause,
};

struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LedgerPhase {
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ObserverPhase {
    Work,
}

impl ProgressPhase for ObserverPhase {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "work"
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
    Bytes,
}

type Coordinator = OperationCoordinator<Owner, LedgerPhase, ObserverPhase, Unit, Dimension>;
type Reservation = LiveReservation<Owner, LedgerPhase, Dimension>;
type Publication = PendingPublication<Owner, LedgerPhase, Dimension>;

fn assert_send<T: Send>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn operation_types_expose_the_intended_thread_safety_contract() {
    let _vocabulary = (
        LedgerPhase::Build,
        ObserverPhase::Work,
        Unit::Work,
        Dimension::Bytes,
    );

    // Mutable observers are serialized through exclusive coordinator access, so these types move
    // between threads but are not required to support shared access themselves.
    assert_send::<OperationControl<ObserverPhase, Unit>>();
    assert_send::<Coordinator>();

    assert_send_sync::<CancellationToken>();
    assert_send_sync::<Reservation>();
    assert_send_sync::<Publication>();
    assert_send_sync::<LedgerLimits<Dimension>>();
    assert_send_sync::<LedgerSnapshot<LedgerPhase, Dimension>>();
    assert_send_sync::<DimensionSnapshot>();
    assert_send_sync::<PhaseSnapshot>();
    assert_send_sync::<LedgerCause<Dimension>>();
    assert_send_sync::<LedgerEvidenceError>();
    assert_send_sync::<LedgerEvidencePurpose>();
    assert_send_sync::<LedgerLimitsError<Dimension>>();
    assert_send_sync::<LedgerFinishError<LedgerPhase, Dimension>>();
    assert_send_sync::<PublicationCause<Dimension>>();
    assert_send_sync::<ControlCause>();
    assert_send_sync::<CancellationOrigin>();
    assert_send_sync::<CounterClass>();
    assert_send_sync::<ProgressViolation>();
    assert_send_sync::<ProgressStorageError>();
    assert_send_sync::<ProgressDecision>();
    assert_send_sync::<ProgressDeliveryLimits>();
    assert_send_sync::<ProgressEvent<ObserverPhase, Unit>>();
    assert_send_sync::<FinalProgress<ObserverPhase, Unit>>();
    assert_send_sync::<DimensionPolicy>();
    assert_send_sync::<Commitment<Dimension>>();
}
