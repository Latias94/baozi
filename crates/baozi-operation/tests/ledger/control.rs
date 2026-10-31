use super::*;

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
    let mut coordinator = coordinator_with(
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
    let mut coordinator = coordinator_with(
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
    let mut coordinator = coordinator_with(control, limits(20));
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
        let mut coordinator = coordinator_with(control, publication_limits(20));

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
    let mut coordinator = coordinator_with(
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
    let mut coordinator = coordinator_with(
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
    let mut coordinator = coordinator_with(control, publication_limits(20));
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
    let mut duplicate = coordinator_with(
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
