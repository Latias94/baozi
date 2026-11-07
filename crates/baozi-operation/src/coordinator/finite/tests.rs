use super::*;
use crate::control::{
    CancellationOrigin, CancellationSignal, ProgressDecision, ProgressDeliveryLimits,
    ProgressEvent, ProgressObserver,
};
use crate::ledger::{
    LedgerEvidenceError, LiveReservation, WorkLane, test_evidence_allocation_guard,
    test_fail_next_evidence_allocation,
};
use crate::{
    DimensionPolicy, FinalProgress, LedgerLimits, OperationCoordinator, ProgressStorageError,
    PublicationCause,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Prepare,
    Finish,
}

impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        match self {
            Self::Prepare => 0,
            Self::Finish => 1,
        }
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Finish => "finish",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    First,
    Second,
    Third,
    Fourth,
    Fifth,
}

impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Second => "second",
            Self::Third => "third",
            Self::Fourth => "fourth",
            Self::Fifth => "fifth",
        }
    }
}

type Coordinator = FiniteOperationCoordinator<Owner, u8, Phase, Unit, u8>;
type Trace = Arc<Mutex<Vec<&'static str>>>;

fn limits(evidence_byte_limit: u64) -> FiniteLedgerLimits<u8> {
    FiniteLedgerLimits::try_new(
        64,
        evidence_byte_limit,
        [6, 2, 5, 1, 4, 3]
            .map(|dimension| (dimension, DimensionPolicy::reservable_bytes(32, 32, 0, 64))),
    )
    .unwrap()
}

fn coordinator(control: OperationControl<Phase, Unit>) -> Coordinator {
    Coordinator::try_new(control, limits(0), [3, 1, 2]).unwrap()
}

#[derive(Clone, Copy)]
enum ObserverAction {
    Continue,
    CancelSignal,
    CancelObserver,
    Panic,
}

struct TracedSignal {
    trace: Trace,
    cancelled: Arc<AtomicBool>,
    panic_on_drop: bool,
}

impl CancellationSignal for TracedSignal {
    fn is_cancelled(&self) -> bool {
        self.trace.lock().unwrap().push("poll");
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Drop for TracedSignal {
    fn drop(&mut self) {
        self.trace.lock().unwrap().push("drop signal");
        if self.panic_on_drop {
            panic!("cancellation disposal failure");
        }
    }
}

struct TracedObserver {
    trace: Trace,
    cancelled: Arc<AtomicBool>,
    action: ObserverAction,
    panic_on_drop: bool,
}

impl ProgressObserver<Phase, Unit> for TracedObserver {
    fn observe(&mut self, _: ProgressEvent<Phase, Unit>) -> ProgressDecision {
        self.trace.lock().unwrap().push("observe");
        match self.action {
            ObserverAction::Continue => ProgressDecision::Continue,
            ObserverAction::CancelSignal => {
                self.cancelled.store(true, Ordering::SeqCst);
                ProgressDecision::Continue
            }
            ObserverAction::CancelObserver => ProgressDecision::Cancel,
            ObserverAction::Panic => panic!("observer invocation failure"),
        }
    }
}

impl Drop for TracedObserver {
    fn drop(&mut self) {
        self.trace.lock().unwrap().push("drop observer");
        if self.panic_on_drop {
            panic!("observer disposal failure");
        }
    }
}

fn traced_coordinator(
    action: ObserverAction,
    panic_signal_drop: bool,
    panic_observer_drop: bool,
) -> (Coordinator, Trace, Arc<AtomicBool>) {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = OperationControl::default()
        .with_cancellation(TracedSignal {
            trace: Arc::clone(&trace),
            cancelled: Arc::clone(&cancelled),
            panic_on_drop: panic_signal_drop,
        })
        .with_progress_observer(TracedObserver {
            trace: Arc::clone(&trace),
            cancelled: Arc::clone(&cancelled),
            action,
            panic_on_drop: panic_observer_drop,
        });
    (coordinator(control), trace, cancelled)
}

#[test]
fn fixed_root_constructs_reports_and_closes_without_administrative_heap_admission() {
    let _guard = test_evidence_allocation_guard();
    let mut dynamic = OperationCoordinator::<Owner, u8, Phase, Unit, u8>::try_new(
        OperationControl::default(),
        LedgerLimits::new(0, []).unwrap(),
        &[],
    )
    .unwrap();
    test_fail_next_evidence_allocation();
    let mut finite = coordinator(OperationControl::default());
    assert_eq!(finite.known_administrative_bytes(), Some(0));
    assert_eq!(finite.progress.len, 0);
    assert!(finite.progress.rows.iter().all(Option::is_none));
    for unit in [Unit::Third, Unit::First, Unit::Fourth, Unit::Second] {
        finite
            .view()
            .report_progress(Phase::Prepare, unit, 1, Some(2))
            .unwrap();
    }
    finite
        .view()
        .try_prepare_typed_control(FinalProgress::new(Phase::Prepare, Unit::First, 2))
        .unwrap();
    finite.view().close_for_terminal_extraction().unwrap();
    finite.view().close_for_terminal_extraction().unwrap();
    assert_eq!(finite.progress.len, 4);
    assert!(finite.progress.rows.iter().all(Option::is_some));
    assert_eq!(finite.known_administrative_bytes(), Some(0));
    assert_eq!(
        finite.retained_ledger().known_administrative_bytes(0),
        Some(0)
    );
    assert_eq!(
        finite.reported_progress(Phase::Prepare, Unit::First),
        Some((2, Some(2)))
    );
    assert_eq!(finite.control_state.delivery.next_sequence, 0);
    assert_eq!(
        finite.retained_ledger_mut().view().ensure_work_open(),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    assert_eq!(
        finite.retained_ledger_mut().view().check_work_quiescent(),
        Ok(())
    );
    assert!(matches!(
        dynamic.report_progress(Phase::Prepare, Unit::First, 1, None),
        Err(ControlCause::ProgressStorage {
            cause: ProgressStorageError::Evidence {
                cause: LedgerEvidenceError::AllocationFailed {
                    purpose: crate::LedgerEvidencePurpose::Progress,
                    requested_bytes,
                },
            },
        }) if requested_bytes > 0
    ));
}

#[test]
fn final_gate_keeps_both_polls_and_latches_callback_cancellation_without_replay() {
    for cancel in [false, true] {
        let action = if cancel {
            ObserverAction::CancelSignal
        } else {
            ObserverAction::Continue
        };
        let (mut finite, trace, _) = traced_coordinator(action, false, false);
        let control_result = if cancel {
            Err(ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            })
        } else {
            Ok(())
        };
        let expected = control_result.clone().map_err(PublicationCause::Control);
        assert_eq!(
            finite.view().try_prepare_typed_control(FinalProgress::new(
                Phase::Finish,
                Unit::First,
                1
            )),
            expected
        );
        assert_eq!(
            trace.lock().unwrap().as_slice(),
            ["poll", "observe", "poll", "drop signal", "drop observer"]
        );
        assert_eq!(
            finite.view().try_prepare_typed_control(FinalProgress::new(
                Phase::Prepare,
                Unit::Second,
                99
            )),
            expected
        );
        assert_eq!(finite.view().poll(), control_result.clone());
        assert_eq!(finite.recorded_control_cause(), control_result.err());
        assert_eq!(
            finite.reported_progress(Phase::Finish, Unit::First),
            Some((1, Some(1)))
        );
        assert_eq!(finite.reported_progress(Phase::Prepare, Unit::Second), None);
        assert_eq!(finite.control_state.delivery.next_sequence, 1);
        assert_eq!(
            finite.retained_ledger_mut().view().ensure_work_open(),
            Err(LedgerCause::PublicationAlreadyPending)
        );
        finite.view().close_for_terminal_extraction().unwrap();
        drop(finite);
        assert_eq!(
            trace.lock().unwrap().as_slice(),
            ["poll", "observe", "poll", "drop signal", "drop observer"]
        );
    }
}

#[test]
fn callback_failure_keeps_the_first_cause_while_both_extension_drops_panic() {
    for (action, cause, expected_trace) in [
        (
            ObserverAction::CancelObserver,
            ControlCause::Cancelled {
                origin: CancellationOrigin::Observer,
            },
            ["poll", "observe", "drop signal", "drop observer"],
        ),
        (
            ObserverAction::Panic,
            ControlCause::ObserverPanicked,
            ["poll", "observe", "drop observer", "drop signal"],
        ),
    ] {
        let (mut finite, trace, _) = traced_coordinator(action, true, true);
        let expected = Err(PublicationCause::Control(cause.clone()));
        assert_eq!(
            finite.view().try_prepare_typed_control(FinalProgress::new(
                Phase::Finish,
                Unit::First,
                1
            )),
            expected
        );
        assert_eq!(finite.recorded_control_cause(), Some(cause.clone()));
        assert_eq!(finite.view().poll(), Err(cause.clone()));
        assert_eq!(
            finite.view().try_prepare_typed_control(FinalProgress::new(
                Phase::Finish,
                Unit::Second,
                2
            )),
            expected
        );
        assert_eq!(
            finite.view().close_for_terminal_extraction(),
            Err(PublicationCause::Control(
                ControlCause::CancellationSignalPanicked
            ))
        );
        assert_eq!(finite.recorded_control_cause(), Some(cause));
        assert_eq!(
            finite.retained_ledger_mut().view().ensure_work_open(),
            Err(LedgerCause::PublicationAlreadyCompleted)
        );
        drop(finite);
        assert_eq!(trace.lock().unwrap().as_slice(), expected_trace);
    }
}

#[test]
fn terminal_close_attempts_both_drops_and_closes_work_even_when_disposal_fails() {
    let (mut finite, trace, _) = traced_coordinator(ObserverAction::Continue, true, true);
    let expected = Err(PublicationCause::Control(
        ControlCause::CancellationSignalPanicked,
    ));
    assert_eq!(finite.view().close_for_terminal_extraction(), expected);
    assert_eq!(finite.view().close_for_terminal_extraction(), expected);
    assert_eq!(
        finite.retained_ledger_mut().view().ensure_work_open(),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    assert_eq!(
        finite.recorded_control_cause(),
        Some(ControlCause::CancellationSignalPanicked)
    );
    drop(finite);
    assert_eq!(
        trace.lock().unwrap().as_slice(),
        ["drop signal", "drop observer"]
    );
}

#[test]
fn final_gate_rejects_an_actual_zero_remaining_open_frame_and_latches_its_rejection() {
    let (mut finite, trace, _) = traced_coordinator(ObserverAction::Continue, false, false);
    let (layout, mut frame) = {
        let (mut work, _) = finite.retained_ledger_mut().view().into_internal();
        let layout = work.bind_ordinary([1]).unwrap();
        let mut frame = layout
            .admit_observed(&mut work.reborrow().unwrap().work, [1], WorkLane::Internal)
            .unwrap();
        frame
            .complete_batch_with_outcome(&layout, &mut work.reborrow().unwrap().work, [1])
            .into_result()
            .unwrap();
        assert_eq!(work.reborrow().unwrap().counters(1).unwrap(), (1, 1, 0));
        (layout, frame)
    };
    assert_eq!(frame.remaining, [0]);
    let expected = Err(PublicationCause::Ledger(
        LedgerCause::OutstandingLinearWorkPermits { count: 1 },
    ));
    assert_eq!(
        finite
            .view()
            .try_prepare_typed_control(FinalProgress::new(Phase::Finish, Unit::First, 1)),
        expected
    );
    assert_eq!(finite.recorded_control_cause(), None);
    assert_eq!(finite.reported_progress(Phase::Finish, Unit::First), None);
    assert_eq!(finite.progress.len, 0);
    assert_eq!(
        finite.retained_ledger_mut().view().ensure_work_open(),
        Ok(())
    );
    assert_eq!(finite.view().close_for_terminal_extraction(), expected);
    finite
        .retained_ledger_mut()
        .view()
        .into_internal()
        .0
        .resolve(layout, &mut frame, false)
        .unwrap();
    assert_eq!(
        finite
            .view()
            .try_prepare_typed_control(FinalProgress::new(Phase::Finish, Unit::First, 2)),
        expected
    );
    finite.view().close_for_terminal_extraction().unwrap();
    assert_eq!(finite.known_administrative_bytes(), Some(0));
    drop(finite);
    assert_eq!(
        trace.lock().unwrap().as_slice(),
        ["drop signal", "drop observer"]
    );
}

#[test]
fn borrowed_progress_and_poll_authorities_share_the_finite_roots_actual_work() {
    let mut finite = coordinator(OperationControl::default());
    {
        let (mut poll, mut progress, ledger) = finite.view().into_exclusive_progress_and_ledger();
        let (mut work, mut storage) = ledger.into_internal();
        let layout = work.bind_ordinary([1]).unwrap();
        let mut frame = layout
            .admit_observed(&mut work.reborrow().unwrap().work, [2], WorkLane::Internal)
            .unwrap();
        frame
            .complete_batch_with_outcome(&layout, &mut work.reborrow().unwrap().work, [1])
            .into_result()
            .unwrap();
        progress
            .report_progress(
                &mut poll,
                &mut storage,
                Phase::Prepare,
                Unit::First,
                1,
                Some(2),
            )
            .unwrap();
        assert_eq!(work.reborrow().unwrap().counters(1).unwrap(), (2, 1, 1));
        assert_eq!(frame.remaining, [1]);
        frame
            .complete_batch_with_outcome(&layout, &mut work.reborrow().unwrap().work, [1])
            .into_result()
            .unwrap();
        work.resolve(layout, &mut frame, false).unwrap();
    }
    assert_eq!(
        finite.reported_progress(Phase::Prepare, Unit::First),
        Some((1, Some(2)))
    );
    let (mut poll, ledger) = finite.view().into_poll_and_ledger();
    poll.poll().unwrap();
    assert_eq!(
        ledger
            .into_internal()
            .0
            .reborrow()
            .unwrap()
            .counters(1)
            .unwrap(),
        (2, 2, 0)
    );
}

#[test]
fn terminal_poll_only_exempts_a_cause_that_precedes_the_borrow() {
    let (mut finite, trace, cancelled) = traced_coordinator(ObserverAction::Continue, false, false);
    cancelled.store(true, Ordering::SeqCst);
    let cause = ControlCause::Cancelled {
        origin: CancellationOrigin::Signal,
    };
    assert_eq!(finite.view().poll(), Err(cause.clone()));
    {
        let (mut poll, ledger) = finite.view().into_terminal_poll_and_ledger();
        assert_eq!(poll.recorded_control_cause(), Some(cause.clone()));
        poll.poll().unwrap();
        ledger.check_work_quiescent().unwrap();
    }
    assert_eq!(trace.lock().unwrap().as_slice(), ["poll"]);
    assert_eq!(finite.view().poll(), Err(cause.clone()));
    let (mut live, live_trace, late_cancel) =
        traced_coordinator(ObserverAction::Continue, false, false);
    {
        let (mut poll, ledger) = live.view().into_terminal_poll_and_ledger();
        late_cancel.store(true, Ordering::SeqCst);
        assert_eq!(poll.poll(), Err(cause));
        ledger.check_work_quiescent().unwrap();
    }
    assert_eq!(live_trace.lock().unwrap().as_slice(), ["poll"]);
}

#[test]
fn coalescing_uses_work_delta_with_delivery_budget_remaining_and_still_delivers_final() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let delivered = Arc::clone(&events);
    let delivery_limits = ProgressDeliveryLimits::new(16, 4096, 3);
    let mut finite = coordinator(
        OperationControl::default()
            .with_progress_limits(delivery_limits)
            .with_progress_observer(move |event| {
                delivered.lock().unwrap().push(event);
                ProgressDecision::Continue
            }),
    );
    for (completed, delivered_count) in [(1, 1), (2, 1), (4, 2), (6, 2), (9, 3)] {
        finite
            .view()
            .report_progress(Phase::Prepare, Unit::First, completed, Some(10))
            .unwrap();
        assert_eq!(events.lock().unwrap().len(), delivered_count);
        assert_eq!(
            finite.reported_progress(Phase::Prepare, Unit::First),
            Some((completed, Some(10)))
        );
        assert!(finite.control_state.delivery.delivered_events < delivery_limits.max_events());
        assert!(
            finite.control_state.delivery.delivered_bytes < delivery_limits.max_encoded_bytes()
        );
    }
    assert_eq!(finite.progress.rows[0].unwrap().1.last_delivered, Some(9));
    finite
        .view()
        .report_progress(Phase::Prepare, Unit::First, 10, Some(10))
        .unwrap();
    assert_eq!(events.lock().unwrap().len(), 4);
    finite
        .view()
        .report_progress(Phase::Prepare, Unit::First, 10, Some(10))
        .unwrap();
    assert_eq!(events.lock().unwrap().len(), 4);
    finite
        .view()
        .try_prepare_typed_control(FinalProgress::new(Phase::Prepare, Unit::First, 10))
        .unwrap();
    let observed = events.lock().unwrap();
    assert_eq!(observed.len(), 5);
    for (event, (completed, sequence)) in
        observed
            .iter()
            .zip([(1, 0), (4, 1), (9, 2), (10, 3), (10, 4)])
    {
        assert_eq!(event.phase(), Phase::Prepare);
        assert_eq!(event.unit(), Unit::First);
        assert_eq!(event.completed(), completed);
        assert_eq!(event.total(), Some(10));
        assert_eq!(event.sequence(), sequence);
    }
    assert_eq!(finite.control_state.delivery.delivered_events, 4);
    assert_eq!(finite.control_state.delivery.next_sequence, 5);
    assert_eq!(finite.known_administrative_bytes(), Some(0));
}

#[test]
fn fifth_progress_key_is_rejected_by_the_actual_finite_owner_without_replaying_final() {
    let mut finite = coordinator(OperationControl::default());
    for unit in [Unit::Third, Unit::First, Unit::Fourth, Unit::Second] {
        finite
            .view()
            .report_progress(Phase::Prepare, unit, 1, Some(2))
            .unwrap();
    }
    let cause = ControlCause::ProgressStorage {
        cause: ProgressStorageError::StateLimitExceeded { value: 5, limit: 4 },
    };
    assert_eq!(
        finite
            .view()
            .report_progress(Phase::Prepare, Unit::Fifth, 1, Some(2)),
        Err(cause.clone())
    );
    assert_eq!(finite.progress.len, 4);
    assert_eq!(finite.reported_progress(Phase::Prepare, Unit::Fifth), None);
    assert_eq!(finite.recorded_control_cause(), Some(cause.clone()));
    assert_eq!(
        finite
            .view()
            .try_prepare_typed_control(FinalProgress::new(Phase::Prepare, Unit::First, 2)),
        Err(PublicationCause::Control(cause.clone()))
    );
    assert_eq!(
        finite
            .view()
            .try_prepare_typed_control(FinalProgress::new(Phase::Finish, Unit::Fifth, 3)),
        Err(PublicationCause::Control(cause))
    );
    assert_eq!(
        finite.reported_progress(Phase::Prepare, Unit::First),
        Some((1, Some(2)))
    );
    finite.view().close_for_terminal_extraction().unwrap();
    assert_eq!(finite.known_administrative_bytes(), Some(0));
}

#[test]
fn shared_storage_releases_actual_zero_and_nonzero_leases_during_extension_disposal() {
    type ReservationSlot = Arc<Mutex<Option<LiveReservation<Owner, u8, u8>>>>;

    struct ReservationOwner {
        slot: ReservationSlot,
        drops: Arc<AtomicUsize>,
        cancel_and_panic: bool,
    }

    impl CancellationSignal for ReservationOwner {
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    impl ProgressObserver<Phase, Unit> for ReservationOwner {
        fn observe(&mut self, _: ProgressEvent<Phase, Unit>) -> ProgressDecision {
            if self.cancel_and_panic {
                ProgressDecision::Cancel
            } else {
                ProgressDecision::Continue
            }
        }
    }

    impl Drop for ReservationOwner {
        fn drop(&mut self) {
            let reservation = self.slot.lock().unwrap().take();
            drop(reservation);
            self.drops.fetch_add(1, Ordering::SeqCst);
            if self.cancel_and_panic {
                panic!("release the actual reservation before extension destruction fails");
            }
        }
    }

    for (signal_bytes, observer_bytes, cancel_and_panic) in
        [(0, 0, false), (7, 9, false), (7, 9, true)]
    {
        let signal_slot = Arc::new(Mutex::new(None));
        let observer_slot = Arc::new(Mutex::new(None));
        let signal_drops = Arc::new(AtomicUsize::new(0));
        let observer_drops = Arc::new(AtomicUsize::new(0));
        let mut finite = Coordinator::try_new(
            OperationControl::default()
                .with_cancellation(ReservationOwner {
                    slot: Arc::clone(&signal_slot),
                    drops: Arc::clone(&signal_drops),
                    cancel_and_panic,
                })
                .with_progress_observer(ReservationOwner {
                    slot: Arc::clone(&observer_slot),
                    drops: Arc::clone(&observer_drops),
                    cancel_and_panic,
                }),
            limits(16_384),
            [3, 1, 2],
        )
        .unwrap();
        assert_eq!(finite.known_administrative_bytes(), Some(0));
        *signal_slot.lock().unwrap() = Some(
            finite
                .retained_ledger_mut()
                .view()
                .reserve(1, 1, signal_bytes)
                .unwrap(),
        );
        *observer_slot.lock().unwrap() = Some(
            finite
                .retained_ledger_mut()
                .view()
                .reserve(3, 6, observer_bytes)
                .unwrap(),
        );
        let administration = finite.known_administrative_bytes().unwrap();
        assert!(administration > 0);
        {
            let (_, storage) = finite.retained_ledger_mut().view().into_internal();
            let state = storage.planning_state();
            assert_eq!(state.current_live_bytes(), signal_bytes + observer_bytes);
            assert_eq!(state.active_reservations(), 2);
        }
        let gate = finite.view().try_prepare_typed_control(FinalProgress::new(
            Phase::Finish,
            Unit::First,
            1,
        ));
        let expected = if cancel_and_panic {
            Err(PublicationCause::Control(ControlCause::Cancelled {
                origin: CancellationOrigin::Observer,
            }))
        } else {
            Ok(())
        };
        assert_eq!(gate, expected);
        if cancel_and_panic {
            assert_eq!(
                finite.recorded_control_cause(),
                Some(ControlCause::Cancelled {
                    origin: CancellationOrigin::Observer
                })
            );
        }
        assert_eq!(signal_drops.load(Ordering::SeqCst), 1);
        assert_eq!(observer_drops.load(Ordering::SeqCst), 1);
        assert!(signal_slot.lock().unwrap().is_none());
        assert!(observer_slot.lock().unwrap().is_none());
        {
            let (_, storage) = finite.retained_ledger_mut().view().into_internal();
            let state = storage.planning_state();
            assert_eq!(state.current_live_bytes(), 0);
            assert_eq!(state.active_reservations(), 0);
        }
        assert_eq!(finite.known_administrative_bytes(), Some(administration));
        let close = finite.view().close_for_terminal_extraction();
        assert_eq!(
            close,
            if cancel_and_panic {
                Err(PublicationCause::Control(
                    ControlCause::CancellationSignalPanicked,
                ))
            } else {
                Ok(())
            }
        );
        assert_eq!(
            finite.view().try_prepare_typed_control(FinalProgress::new(
                Phase::Prepare,
                Unit::Second,
                2
            )),
            expected
        );
        assert_eq!(
            finite.retained_ledger_mut().view().ensure_work_open(),
            Err(LedgerCause::PublicationAlreadyCompleted)
        );
        assert_eq!(finite.known_administrative_bytes(), Some(administration));
        drop(finite);
        assert_eq!(signal_drops.load(Ordering::SeqCst), 1);
        assert_eq!(observer_drops.load(Ordering::SeqCst), 1);
    }
}
