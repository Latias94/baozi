use baozi_operation::{
    CancellationOrigin, CancellationSignal, Commitment, ControlCause, DimensionPolicy,
    FinalProgress, LedgerCause, LedgerLimits, LiveReservation, OperationControl,
    OperationCoordinator, OperationFinishError, OperationOwner, PendingPublication,
    ProgressDecision, ProgressEvent, ProgressObserver, ProgressPhase, ProgressUnit,
    PublicationCause, PublicationReservationBatch,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Final,
}

impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "final"
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
    Work,
}

type Coordinator = OperationCoordinator<Owner, Phase, Phase, Unit, Dimension>;
type Reservation = LiveReservation<Owner, Phase, Dimension>;
type Pending = PendingPublication<Owner, Phase, Dimension>;

fn coordinator(control: OperationControl<Phase, Unit>) -> Coordinator {
    let limits = LedgerLimits::new(
        1024,
        [
            (
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(1024, 1024, 1024, 1024),
            ),
            (Dimension::Work, DimensionPolicy::counter(64, 64, 8)),
        ],
    )
    .expect("valid disposal test policy");
    Coordinator::try_new(control, limits, &[Phase::Final]).expect("complete phase catalog")
}

fn pending(coordinator: &mut Coordinator) -> Pending {
    coordinator
        .prepare_publication(
            &mut PublicationReservationBatch::default(),
            &[Commitment::new(Dimension::Work, 1)],
        )
        .expect("empty live graph can stage one output commitment")
}

fn final_progress() -> FinalProgress<Phase, Unit> {
    FinalProgress::new(Phase::Final, Unit::Work, 1)
}

struct PanickingDropObserver {
    events: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl ProgressObserver<Phase, Unit> for PanickingDropObserver {
    fn observe(&mut self, _: ProgressEvent<Phase, Unit>) -> ProgressDecision {
        self.events.fetch_add(1, Ordering::SeqCst);
        ProgressDecision::Continue
    }
}

impl Drop for PanickingDropObserver {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        panic!("observer destructor failure must be reconciled before publication");
    }
}

struct PanickingDropCancellation {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl CancellationSignal for PanickingDropCancellation {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::SeqCst);
        false
    }
}

impl Drop for PanickingDropCancellation {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        panic!("cancellation destructor failure must be reconciled before publication");
    }
}

struct ReleasingObserver {
    reservation: Arc<Mutex<Option<Reservation>>>,
    drops: Arc<AtomicUsize>,
    panic_on_drop: bool,
}

impl ProgressObserver<Phase, Unit> for ReleasingObserver {
    fn observe(&mut self, _: ProgressEvent<Phase, Unit>) -> ProgressDecision {
        ProgressDecision::Continue
    }
}

impl Drop for ReleasingObserver {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        let reservation = self
            .reservation
            .lock()
            .expect("callback reservation slot")
            .take();
        drop(reservation);
        assert!(
            !self.panic_on_drop,
            "observer failed after releasing its storage"
        );
    }
}

#[test]
fn observer_disposal_failure_rejects_publication_without_replaying_the_final_event() {
    let events = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let control = OperationControl::default().with_progress_observer(PanickingDropObserver {
        events: Arc::clone(&events),
        drops: Arc::clone(&drops),
    });
    let mut coordinator = coordinator(control);
    let mut pending = pending(&mut coordinator);
    let expected = PublicationCause::Control(ControlCause::ObserverPanicked);

    assert_eq!(
        coordinator.publish(&mut pending, final_progress()),
        Err(expected.clone())
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(events.load(Ordering::SeqCst), 1);
    let evidence = coordinator
        .dimension_evidence(Dimension::Work)
        .expect("work evidence");
    assert_eq!(evidence.pending_publication(), 1);
    assert_eq!(evidence.committed(), 0);

    assert_eq!(
        coordinator.publish(&mut pending, final_progress()),
        Err(expected.clone())
    );
    assert_eq!(events.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    pending.rollback();

    let expected_administrative_bytes = coordinator
        .try_ledger_snapshot()
        .expect("post-rollback evidence")
        .current_administrative_bytes();
    let failure = coordinator
        .finish()
        .expect_err("disposed observer failure remains terminal");
    assert_eq!(failure.cause(), &expected);
    assert_eq!(failure.snapshot().current_live_bytes(), 0);
    assert_eq!(
        failure.snapshot().current_administrative_bytes(),
        expected_administrative_bytes
    );
    assert_eq!(
        failure
            .snapshot()
            .dimension(Dimension::Work)
            .unwrap()
            .committed(),
        0
    );
    assert_eq!(events.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_disposal_failure_rejects_publication_and_retry_performs_no_more_callbacks() {
    let events = Arc::new(AtomicUsize::new(0));
    let event_calls = Arc::clone(&events);
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let control = OperationControl::default()
        .with_cancellation(PanickingDropCancellation {
            polls: Arc::clone(&polls),
            drops: Arc::clone(&drops),
        })
        .with_progress_observer(move |_: ProgressEvent<Phase, Unit>| {
            event_calls.fetch_add(1, Ordering::SeqCst);
            ProgressDecision::Continue
        });
    let mut coordinator = coordinator(control);
    let mut pending = pending(&mut coordinator);
    let expected = PublicationCause::Control(ControlCause::CancellationSignalPanicked);

    assert_eq!(
        coordinator.publish(&mut pending, final_progress()),
        Err(expected.clone())
    );
    let polls_after_failure = polls.load(Ordering::SeqCst);
    assert!(polls_after_failure > 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(events.load(Ordering::SeqCst), 1);
    assert_eq!(
        coordinator.publish(&mut pending, final_progress()),
        Err(expected.clone())
    );
    assert_eq!(polls.load(Ordering::SeqCst), polls_after_failure);
    assert_eq!(events.load(Ordering::SeqCst), 1);
    assert_eq!(
        coordinator
            .dimension_evidence(Dimension::Work)
            .unwrap()
            .committed(),
        0
    );
    pending.rollback();
    let expected_administrative_bytes = coordinator
        .try_ledger_snapshot()
        .expect("post-rollback evidence")
        .current_administrative_bytes();
    let failure = coordinator
        .finish()
        .expect_err("cancellation disposal failure remains terminal");
    assert_eq!(failure.cause(), &expected);
    assert_eq!(
        failure.snapshot().current_administrative_bytes(),
        expected_administrative_bytes
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn finish_reports_typed_disposal_failure_after_attempting_both_extension_drops() {
    let observer_drops = Arc::new(AtomicUsize::new(0));
    let cancellation_drops = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let control = OperationControl::default()
        .with_cancellation(PanickingDropCancellation {
            polls: Arc::clone(&polls),
            drops: Arc::clone(&cancellation_drops),
        })
        .with_progress_observer(PanickingDropObserver {
            events: Arc::clone(&events),
            drops: Arc::clone(&observer_drops),
        });

    let mut coordinator = coordinator(control);
    let expected_administrative_bytes = coordinator
        .try_ledger_snapshot()
        .expect("evidence before extension disposal")
        .current_administrative_bytes();
    let failure: OperationFinishError<Phase, Dimension> = coordinator
        .finish()
        .expect_err("finish must return the contained extension disposal failure");
    assert_eq!(
        failure.cause(),
        &PublicationCause::Control(ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(observer_drops.load(Ordering::SeqCst), 1);
    assert_eq!(cancellation_drops.load(Ordering::SeqCst), 1);
    assert_eq!(events.load(Ordering::SeqCst), 0);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    let (cause, snapshot) = failure.into_parts();
    assert_eq!(
        cause,
        PublicationCause::Control(ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
    assert_eq!(
        snapshot.current_administrative_bytes(),
        expected_administrative_bytes
    );
}

#[test]
fn callback_owned_storage_is_released_before_success_or_failure_final_evidence() {
    for panic_on_drop in [false, true] {
        let reservation = Arc::new(Mutex::new(None));
        let drops = Arc::new(AtomicUsize::new(0));
        let control = OperationControl::default().with_progress_observer(ReleasingObserver {
            reservation: Arc::clone(&reservation),
            drops: Arc::clone(&drops),
            panic_on_drop,
        });
        let mut coordinator = coordinator(control);
        *reservation.lock().unwrap() = Some(
            coordinator
                .checkpoint()
                .reserve(Phase::Final, Dimension::Bytes, 64)
                .expect("callback-owned retained storage"),
        );
        let interim = coordinator.try_ledger_snapshot().unwrap();
        assert_eq!(interim.current_live_bytes(), 64);

        let snapshot = match coordinator.finish() {
            Ok(snapshot) => {
                assert!(!panic_on_drop);
                snapshot
            }
            Err(failure) => {
                assert!(panic_on_drop);
                let (cause, snapshot) = failure.into_parts();
                assert_eq!(
                    cause,
                    PublicationCause::Control(ControlCause::ObserverPanicked)
                );
                snapshot
            }
        };
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(reservation.lock().unwrap().is_none());
        assert_eq!(snapshot.current_live_bytes(), 0);
        assert_eq!(snapshot.peak_live_bytes(), 64);
        assert_eq!(snapshot.active_reservations(), 0);
        assert_eq!(
            snapshot.current_administrative_bytes(),
            interim.current_administrative_bytes()
        );
        assert_eq!(
            snapshot
                .dimension(Dimension::Bytes)
                .unwrap()
                .current_live_bytes(),
            0
        );
        assert_eq!(
            snapshot
                .dimension(Dimension::Bytes)
                .unwrap()
                .peak_live_bytes(),
            64
        );
        assert_eq!(
            snapshot.phase(Phase::Final).unwrap().current_live_bytes(),
            0
        );
    }
}

#[test]
fn finish_preserves_ledger_failure_and_its_non_normalized_snapshot() {
    let mut coordinator = coordinator(OperationControl::default());
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Final, Dimension::Bytes, 64)
        .expect("independently retained live storage");
    let failure: OperationFinishError<Phase, Dimension> = coordinator
        .finish()
        .expect_err("live storage cannot become a clean final snapshot");
    assert_eq!(
        failure.cause(),
        &PublicationCause::Ledger(LedgerCause::LiveReservationsRemain { bytes: 64 })
    );
    assert_eq!(failure.snapshot().current_live_bytes(), 64);
    assert_eq!(failure.snapshot().active_reservations(), 1);
    drop(reservation);
    assert_eq!(failure.snapshot().current_live_bytes(), 64);
}

#[test]
fn failed_publish_rollback_cannot_resume_ordinary_accounting_or_allocation() {
    let events = Arc::new(AtomicUsize::new(0));
    let event_calls = Arc::clone(&events);
    let control =
        OperationControl::default().with_progress_observer(move |_: ProgressEvent<Phase, Unit>| {
            event_calls.fetch_add(1, Ordering::SeqCst);
            ProgressDecision::Cancel
        });
    let mut coordinator = coordinator(control);
    coordinator
        .checkpoint()
        .charge_observed_batch(&[(Dimension::Work, 2)])
        .expect("ordinary completed prefix before sealing");
    let mut pending = pending(&mut coordinator);
    let cause = PublicationCause::Control(ControlCause::Cancelled {
        origin: CancellationOrigin::Observer,
    });
    assert_eq!(
        coordinator.publish(&mut pending, final_progress()),
        Err(cause.clone())
    );
    pending.rollback();

    assert_eq!(
        coordinator
            .checkpoint()
            .charge_attempted(Dimension::Work, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(
        coordinator.checkpoint().charge_consumed(Dimension::Work, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(
        coordinator
            .checkpoint()
            .charge_observed_batch(&[(Dimension::Work, 1)]),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert!(matches!(
        coordinator
            .checkpoint()
            .reserve(Phase::Final, Dimension::Bytes, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    assert!(matches!(
        coordinator.prepare_publication(
            &mut PublicationReservationBatch::default(),
            &[Commitment::new(Dimension::Work, 1)],
        ),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    let work = coordinator.dimension_evidence(Dimension::Work).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (2, 2));
    assert_eq!(work.pending_publication(), 0);
    assert_eq!(work.committed(), 0);
    assert_eq!(events.load(Ordering::SeqCst), 1);

    let expected_administrative_bytes = coordinator
        .try_ledger_snapshot()
        .expect("evidence after rejected accounting mutations")
        .current_administrative_bytes();
    let failure = coordinator
        .finish()
        .expect_err("final control rejection remains authoritative");
    assert_eq!(failure.cause(), &cause);
    assert_eq!(
        failure
            .snapshot()
            .dimension(Dimension::Work)
            .unwrap()
            .consumed(),
        2
    );
    assert_eq!(
        failure.snapshot().current_administrative_bytes(),
        expected_administrative_bytes
    );
    assert_eq!(events.load(Ordering::SeqCst), 1);
}
