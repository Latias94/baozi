use baozi_operation::{
    CancellationOrigin, CancellationToken, ControlCause, LedgerEvidenceError,
    LedgerEvidencePurpose, LedgerLimits, OperationControl, OperationCoordinator, OperationOwner,
    ProgressDecision, ProgressDeliveryLimits, ProgressEvent, ProgressObserver, ProgressPhase,
    ProgressStorageError, ProgressUnit, ProgressViolation,
};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct PanickingCancellation;
struct PanickingDropCancellation;
struct DropOnlyCancellation;
struct PanickingDropObserver;
struct DropOnlyObserver;

struct PanickingDropPayload;

impl Drop for PanickingDropPayload {
    fn drop(&mut self) {
        panic!("panic payload drop must never run");
    }
}

impl Drop for PanickingDropCancellation {
    fn drop(&mut self) {
        panic!("panicking cancellation drop must be contained");
    }
}

impl Drop for PanickingDropObserver {
    fn drop(&mut self) {
        panic!("panicking observer drop must be contained");
    }
}

impl Drop for DropOnlyCancellation {
    fn drop(&mut self) {
        panic!("cancellation drop must be contained");
    }
}

impl Drop for DropOnlyObserver {
    fn drop(&mut self) {
        panic!("observer drop must be contained");
    }
}

#[derive(Debug)]
struct TestOwner;

impl OperationOwner for TestOwner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LedgerPhase {}

impl baozi_operation::CancellationSignal for PanickingCancellation {
    fn is_cancelled(&self) -> bool {
        panic!("cancellation panic")
    }
}

impl baozi_operation::CancellationSignal for PanickingDropCancellation {
    fn is_cancelled(&self) -> bool {
        std::panic::panic_any(PanickingDropPayload)
    }
}

impl baozi_operation::CancellationSignal for DropOnlyCancellation {
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Acquire,
    Parse,
}

impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        match self {
            Self::Acquire => 0,
            Self::Parse => 1,
        }
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Acquire => "acquire",
            Self::Parse => "parse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    Bytes,
    Records,
}

impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::Records => "records",
        }
    }
}

#[derive(Clone)]
struct RecordingObserver {
    events: Arc<Mutex<Vec<ProgressEvent<Phase, Unit>>>>,
    decision: ProgressDecision,
}

impl ProgressObserver<Phase, Unit> for RecordingObserver {
    fn observe(&mut self, event: ProgressEvent<Phase, Unit>) -> ProgressDecision {
        self.events.lock().expect("recording lock").push(event);
        self.decision
    }
}

#[derive(Default)]
struct ParallelObserverProbe {
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    calls: AtomicUsize,
    nested_runs: AtomicUsize,
    callback_threads: Mutex<HashSet<thread::ThreadId>>,
}

impl ParallelObserverProbe {
    fn enter(&self) -> ObserverCallGuard<'_> {
        let in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);
        self.callback_threads
            .lock()
            .expect("callback thread set is not poisoned")
            .insert(thread::current().id());
        ObserverCallGuard {
            in_flight: &self.in_flight,
        }
    }
}

struct ObserverCallGuard<'probe> {
    in_flight: &'probe AtomicUsize,
}

impl Drop for ObserverCallGuard<'_> {
    fn drop(&mut self) {
        let previous = self.in_flight.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous > 0, "observer in-flight count must not underflow");
    }
}

struct ParallelObserver {
    probe: Arc<ParallelObserverProbe>,
    panic_on_call: Option<usize>,
}

impl ProgressObserver<Phase, Unit> for ParallelObserver {
    fn observe(&mut self, _event: ProgressEvent<Phase, Unit>) -> ProgressDecision {
        let _call_guard = self.probe.enter();
        let call = self.probe.calls.fetch_add(1, Ordering::SeqCst) + 1;

        let mut nested = coordinator(OperationControl::default());
        nested
            .report_progress(Phase::Acquire, Unit::Bytes, 1, Some(1))
            .expect("nested operation remains independent during callback delivery");
        self.probe.nested_runs.fetch_add(1, Ordering::SeqCst);
        thread::yield_now();

        assert_ne!(self.panic_on_call, Some(call), "parallel observer panic");
        ProgressDecision::Continue
    }
}

impl ProgressObserver<Phase, Unit> for PanickingDropObserver {
    fn observe(&mut self, _event: ProgressEvent<Phase, Unit>) -> ProgressDecision {
        std::panic::panic_any(PanickingDropPayload)
    }
}

impl ProgressObserver<Phase, Unit> for DropOnlyObserver {
    fn observe(&mut self, _event: ProgressEvent<Phase, Unit>) -> ProgressDecision {
        ProgressDecision::Continue
    }
}

type Coordinator = OperationCoordinator<TestOwner, LedgerPhase, Phase, Unit, Dimension>;

fn coordinator(control: OperationControl<Phase, Unit>) -> Coordinator {
    Coordinator::new(
        control,
        LedgerLimits::new(0, []).expect("empty control-only policy"),
    )
}

fn run_parallel_progress_workers(
    coordinator: &Arc<Mutex<Coordinator>>,
    worker_count: usize,
) -> Vec<Result<(), ControlCause>> {
    let total = u64::try_from(worker_count).expect("worker count fits progress total");
    let start = Arc::new(Barrier::new(worker_count));
    let workers = (0..worker_count)
        .map(|_| {
            let coordinator = Arc::clone(coordinator);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                let mut coordinator = coordinator
                    .lock()
                    .expect("coordinator lock is not poisoned");
                let completed = coordinator
                    .reported_progress(Phase::Parse, Unit::Records)
                    .map_or(1, |(completed, _)| completed + 1);
                coordinator.report_progress(Phase::Parse, Unit::Records, completed, Some(total))
            })
        })
        .collect::<Vec<_>>();

    workers
        .into_iter()
        .map(|worker| worker.join().expect("parallel worker must not panic"))
        .collect()
}

#[test]
fn absent_control_is_a_no_op() {
    let mut coordinator = coordinator(OperationControl::default());

    coordinator.poll().expect("no-op control remains active");
    coordinator
        .report_progress(Phase::Acquire, Unit::Bytes, 1, None)
        .expect("progress without an observer is accepted");
}

#[test]
fn semantic_progress_state_count_is_fallibly_bounded_and_latched() {
    let limits = ProgressDeliveryLimits::new(8, 1024, 1).with_max_states(1);
    let mut coordinator = coordinator(OperationControl::default().with_progress_limits(limits));
    coordinator
        .report_progress(Phase::Acquire, Unit::Bytes, 1, None)
        .expect("first semantic state fits");
    let expected = ControlCause::ProgressStorage {
        cause: ProgressStorageError::StateLimitExceeded { value: 2, limit: 1 },
    };
    assert_eq!(
        coordinator.report_progress(Phase::Acquire, Unit::Records, 1, None),
        Err(expected.clone())
    );
    assert_eq!(coordinator.poll(), Err(expected));
}

#[test]
fn semantic_progress_capacity_obeys_the_evidence_limit_and_latches() {
    let limits = LedgerLimits::new(0, [])
        .expect("empty control-only policy")
        .try_with_evidence_byte_limit(0)
        .expect("empty policy retains no evidence");
    let mut coordinator = Coordinator::new(OperationControl::default(), limits);

    let error = coordinator
        .report_progress(Phase::Acquire, Unit::Bytes, 1, None)
        .expect_err("progress capacity must obey the evidence byte limit");
    assert!(matches!(
        error,
        ControlCause::ProgressStorage {
            cause: ProgressStorageError::Evidence {
                cause: LedgerEvidenceError::LimitExceeded {
                    purpose: LedgerEvidencePurpose::Progress,
                    limit: 0,
                    ..
                }
            }
        }
    ));
    assert_eq!(coordinator.poll(), Err(error));

    let snapshot = coordinator
        .finish()
        .expect("denied progress growth retains no evidence capacity");
    assert_eq!(snapshot.current_evidence_bytes(), 0);
}

#[test]
fn semantic_progress_capacity_contributes_to_evidence_peak_then_releases() {
    let limits = LedgerLimits::new(0, []).expect("empty control-only policy");
    let frozen_evidence_bytes = limits.frozen_evidence_bytes();
    let mut coordinator = Coordinator::new(OperationControl::default(), limits);

    coordinator
        .report_progress(Phase::Acquire, Unit::Bytes, 1, None)
        .expect("first semantic state");
    coordinator
        .report_progress(Phase::Acquire, Unit::Records, 1, None)
        .expect("second semantic state");

    let interim = coordinator
        .try_ledger_snapshot()
        .expect("bounded interim evidence");
    assert!(interim.current_evidence_bytes() > frozen_evidence_bytes);
    assert!(interim.peak_evidence_bytes() >= interim.current_evidence_bytes());

    let final_snapshot = coordinator.finish().expect("clean final evidence");
    assert_eq!(final_snapshot.current_evidence_bytes(), 0);
    assert!(
        final_snapshot.peak_evidence_bytes() >= interim.current_evidence_bytes(),
        "the final high-water mark must retain progress evidence capacity"
    );
}

#[test]
fn cancellation_is_monotonic_after_the_first_observation() {
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let mut coordinator = coordinator(control);

    coordinator.poll().expect("operation starts active");
    cancellation.cancel();

    let expected = ControlCause::Cancelled {
        origin: CancellationOrigin::Signal,
    };
    assert_eq!(coordinator.poll(), Err(expected.clone()));
    assert_eq!(coordinator.poll(), Err(expected));
}

#[test]
fn progress_is_ordered_monotonic_and_serialized_by_the_coordinator() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let observer = RecordingObserver {
        events: Arc::clone(&events),
        decision: ProgressDecision::Continue,
    };
    let control = OperationControl::default().with_progress_observer(observer);
    let mut coordinator = coordinator(control);

    coordinator
        .report_progress(Phase::Acquire, Unit::Bytes, 2, Some(4))
        .expect("first event");
    coordinator
        .report_progress(Phase::Acquire, Unit::Bytes, 4, Some(4))
        .expect("completed phase");
    coordinator
        .report_progress(Phase::Parse, Unit::Records, 1, None)
        .expect("later phase");

    let events = events.lock().expect("recording lock");
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].sequence(), 0);
    assert_eq!(events[1].sequence(), 1);
    assert_eq!(events[2].sequence(), 2);
    assert_eq!(events[2].phase(), Phase::Parse);
    assert_eq!(events[2].unit(), Unit::Records);
}

#[test]
fn parallel_workers_share_one_coordinator_without_concurrent_observer_calls() {
    const WORKER_COUNT: usize = 8;

    let probe = Arc::new(ParallelObserverProbe::default());
    let observer = ParallelObserver {
        probe: Arc::clone(&probe),
        panic_on_call: None,
    };
    let coordinator = Arc::new(Mutex::new(coordinator(
        OperationControl::default().with_progress_observer(observer),
    )));
    for result in run_parallel_progress_workers(&coordinator, WORKER_COUNT) {
        result.expect("parallel progress remains valid");
    }

    assert_eq!(probe.calls.load(Ordering::SeqCst), WORKER_COUNT);
    assert_eq!(probe.nested_runs.load(Ordering::SeqCst), WORKER_COUNT);
    assert_eq!(probe.in_flight.load(Ordering::SeqCst), 0);
    assert_eq!(probe.max_in_flight.load(Ordering::SeqCst), 1);
    assert_eq!(
        probe
            .callback_threads
            .lock()
            .expect("callback thread set is not poisoned")
            .len(),
        WORKER_COUNT,
        "every worker must deliver one callback through the shared coordinator"
    );
    assert_eq!(
        coordinator
            .lock()
            .expect("coordinator lock is not poisoned")
            .reported_progress(Phase::Parse, Unit::Records),
        Some((8, Some(8)))
    );
}

#[test]
fn parallel_observer_panic_is_typed_latched_and_preserves_nested_operation_access() {
    const WORKER_COUNT: usize = 8;
    const PANIC_ON_CALL: usize = 3;

    let probe = Arc::new(ParallelObserverProbe::default());
    let observer = ParallelObserver {
        probe: Arc::clone(&probe),
        panic_on_call: Some(PANIC_ON_CALL),
    };
    let coordinator = Arc::new(Mutex::new(coordinator(
        OperationControl::default().with_progress_observer(observer),
    )));
    let results = run_parallel_progress_workers(&coordinator, WORKER_COUNT);
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        PANIC_ON_CALL - 1
    );
    assert!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|cause| *cause == ControlCause::ObserverPanicked)
    );

    assert_eq!(probe.calls.load(Ordering::SeqCst), PANIC_ON_CALL);
    assert_eq!(probe.nested_runs.load(Ordering::SeqCst), PANIC_ON_CALL);
    assert_eq!(probe.in_flight.load(Ordering::SeqCst), 0);
    assert_eq!(probe.max_in_flight.load(Ordering::SeqCst), 1);
    assert_eq!(
        coordinator
            .lock()
            .expect("coordinator lock is not poisoned")
            .poll(),
        Err(ControlCause::ObserverPanicked)
    );
}

#[test]
fn invalid_progress_is_a_latched_control_cause() {
    let mut coordinator = coordinator(OperationControl::default());
    coordinator
        .report_progress(Phase::Parse, Unit::Records, 2, Some(4))
        .expect("initial progress");

    let error = coordinator
        .report_progress(Phase::Acquire, Unit::Bytes, 1, None)
        .expect_err("phase regression must fail");
    assert!(matches!(error, ControlCause::InvalidProgress { .. }));
    assert_eq!(coordinator.poll(), Err(error));
}

#[test]
fn progress_delivery_is_bounded_without_changing_reported_work() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let observer = RecordingObserver {
        events: Arc::clone(&events),
        decision: ProgressDecision::Continue,
    };
    let limits = ProgressDeliveryLimits::new(2, usize::MAX, 1);
    let control = OperationControl::default()
        .with_progress_observer(observer)
        .with_progress_limits(limits);
    let mut coordinator = coordinator(control);

    for completed in 1..=8 {
        coordinator
            .report_progress(Phase::Parse, Unit::Records, completed, Some(8))
            .expect("delivery caps do not abort semantic work");
    }

    assert_eq!(events.lock().expect("recording lock").len(), 2);
    assert_eq!(
        coordinator.reported_progress(Phase::Parse, Unit::Records),
        Some((8, Some(8)))
    );
}

#[test]
fn progress_byte_cap_and_min_delta_coalesce_delivery_only() {
    let byte_capped_events = Arc::new(Mutex::new(Vec::new()));
    let byte_capped = RecordingObserver {
        events: Arc::clone(&byte_capped_events),
        decision: ProgressDecision::Continue,
    };
    let mut byte_capped_coordinator = coordinator(
        OperationControl::default()
            .with_progress_observer(byte_capped)
            .with_progress_limits(ProgressDeliveryLimits::new(usize::MAX, 0, 1)),
    );
    byte_capped_coordinator
        .report_progress(Phase::Parse, Unit::Records, 7, None)
        .expect("byte delivery cap does not reject semantic work");
    assert!(
        byte_capped_events
            .lock()
            .expect("recording lock")
            .is_empty()
    );
    assert_eq!(
        byte_capped_coordinator.reported_progress(Phase::Parse, Unit::Records),
        Some((7, None))
    );

    let coalesced_events = Arc::new(Mutex::new(Vec::new()));
    let coalesced = RecordingObserver {
        events: Arc::clone(&coalesced_events),
        decision: ProgressDecision::Continue,
    };
    let mut coalesced_coordinator = coordinator(
        OperationControl::default()
            .with_progress_observer(coalesced)
            .with_progress_limits(ProgressDeliveryLimits::new(usize::MAX, usize::MAX, 4)),
    );
    for completed in [1, 2, 4, 5, 8] {
        coalesced_coordinator
            .report_progress(Phase::Parse, Unit::Records, completed, Some(10))
            .expect("coalesced progress remains valid");
    }
    assert_eq!(coalesced_events.lock().expect("recording lock").len(), 2);
    assert_eq!(
        coalesced_coordinator.reported_progress(Phase::Parse, Unit::Records),
        Some((8, Some(10)))
    );
}

#[test]
fn repeated_identical_final_progress_is_delivered_once() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let observer = RecordingObserver {
        events: Arc::clone(&events),
        decision: ProgressDecision::Continue,
    };
    let mut coordinator = coordinator(OperationControl::default().with_progress_observer(observer));

    coordinator
        .report_progress(Phase::Parse, Unit::Records, 10, Some(10))
        .expect("first final progress");
    coordinator
        .report_progress(Phase::Parse, Unit::Records, 10, Some(10))
        .expect("identical final progress is a semantic no-op");

    assert_eq!(events.lock().expect("recording lock").len(), 1);
}

#[test]
fn every_progress_regression_is_typed_and_latched() {
    let cases = [
        (
            (Phase::Parse, Unit::Records, 2, Some(4)),
            (Phase::Acquire, Unit::Bytes, 1, None),
            ProgressViolation::PhaseRegressed,
        ),
        (
            (Phase::Parse, Unit::Records, 2, Some(4)),
            (Phase::Parse, Unit::Records, 1, Some(4)),
            ProgressViolation::CompletedRegressed,
        ),
        (
            (Phase::Parse, Unit::Records, 2, Some(4)),
            (Phase::Parse, Unit::Records, 3, Some(3)),
            ProgressViolation::TotalRegressed,
        ),
        (
            (Phase::Parse, Unit::Records, 2, Some(4)),
            (Phase::Parse, Unit::Records, 3, None),
            ProgressViolation::TotalRegressed,
        ),
        (
            (Phase::Parse, Unit::Records, 2, Some(4)),
            (Phase::Parse, Unit::Records, 5, Some(4)),
            ProgressViolation::CompletedExceededTotal,
        ),
    ];

    for (initial, invalid, expected) in cases {
        let mut coordinator = coordinator(OperationControl::default());
        coordinator
            .report_progress(initial.0, initial.1, initial.2, initial.3)
            .expect("initial progress");
        let error = coordinator
            .report_progress(invalid.0, invalid.1, invalid.2, invalid.3)
            .expect_err("regression must fail");
        assert_eq!(
            error,
            ControlCause::InvalidProgress {
                violation: expected,
            }
        );
        assert_eq!(coordinator.poll(), Err(error));
    }
}

#[test]
fn progress_observer_can_run_a_nested_operation_without_internal_locks() {
    let nested_runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let nested_counter = Arc::clone(&nested_runs);
    let observer = move |_event: ProgressEvent<Phase, Unit>| {
        let mut nested = coordinator(OperationControl::default());
        nested.poll().expect("nested operation is independent");
        nested_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ProgressDecision::Continue
    };
    let mut outer = coordinator(OperationControl::default().with_progress_observer(observer));

    outer
        .report_progress(Phase::Acquire, Unit::Bytes, 1, Some(1))
        .expect("observer runs without coordinator or ledger locks");

    assert_eq!(nested_runs.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn observer_cancellation_and_panic_become_typed_latched_causes() {
    let observer = RecordingObserver {
        events: Arc::new(Mutex::new(Vec::new())),
        decision: ProgressDecision::Cancel,
    };
    let mut cancelled = coordinator(OperationControl::default().with_progress_observer(observer));
    assert_eq!(
        cancelled.report_progress(Phase::Acquire, Unit::Bytes, 1, None),
        Err(ControlCause::Cancelled {
            origin: CancellationOrigin::Observer,
        })
    );

    let mut panicked = coordinator(OperationControl::default().with_progress_observer(
        |_event: ProgressEvent<Phase, Unit>| -> ProgressDecision { panic!("observer panic") },
    ));
    let error = panicked
        .report_progress(Phase::Acquire, Unit::Bytes, 1, None)
        .expect_err("observer panic must be contained when unwind is available");
    assert_eq!(error, ControlCause::ObserverPanicked);
    assert_eq!(panicked.poll(), Err(ControlCause::ObserverPanicked));
}

#[test]
fn an_expired_deadline_is_a_typed_latched_cause() {
    let control = OperationControl::default().with_deadline(
        Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("past instant"),
    );
    let mut coordinator = coordinator(control);

    assert_eq!(coordinator.poll(), Err(ControlCause::DeadlineExceeded));
    assert_eq!(coordinator.poll(), Err(ControlCause::DeadlineExceeded));
}

#[test]
fn cancellation_signal_panic_becomes_a_typed_latched_cause() {
    let control = OperationControl::default().with_cancellation(PanickingCancellation);
    let mut coordinator = coordinator(control);

    assert_eq!(
        coordinator.poll(),
        Err(ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(
        coordinator.poll(),
        Err(ControlCause::CancellationSignalPanicked)
    );
}

#[test]
fn opaque_panicking_drop_payloads_cannot_escape_control_boundaries() {
    let mut cancellation =
        coordinator(OperationControl::default().with_cancellation(PanickingDropCancellation));
    assert_eq!(
        cancellation.poll(),
        Err(ControlCause::CancellationSignalPanicked)
    );

    let mut observer =
        coordinator(OperationControl::default().with_progress_observer(PanickingDropObserver));
    assert_eq!(
        observer.report_progress(Phase::Acquire, Unit::Bytes, 1, None),
        Err(ControlCause::ObserverPanicked)
    );
}

#[test]
fn extension_destructor_panics_are_contained_when_control_is_dropped() {
    let control = OperationControl::<Phase, Unit>::default()
        .with_cancellation(DropOnlyCancellation)
        .with_progress_observer(DropOnlyObserver);
    drop(control);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {}
