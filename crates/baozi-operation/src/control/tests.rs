use super::*;
use std::panic::panic_any;
use std::sync::atomic::AtomicUsize;

struct DropProbe {
    drops: Arc<AtomicUsize>,
    panics: bool,
}

impl DropProbe {
    fn new(drops: &Arc<AtomicUsize>, panics: bool) -> Self {
        Self {
            drops: Arc::clone(drops),
            panics,
        }
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        assert!(!self.panics, "extension disposal panic");
    }
}

impl CancellationSignal for DropProbe {
    fn is_cancelled(&self) -> bool {
        false
    }
}

impl ProgressObserver<(), ()> for DropProbe {
    fn observe(&mut self, _event: ProgressEvent<(), ()>) -> ProgressDecision {
        ProgressDecision::Continue
    }
}

#[test]
fn clean_disposal_drops_both_extensions_once() {
    let cancellation_drops = Arc::new(AtomicUsize::new(0));
    let observer_drops = Arc::new(AtomicUsize::new(0));
    let mut control = OperationControl::<(), ()>::default()
        .with_cancellation(DropProbe::new(&cancellation_drops, false))
        .with_progress_observer(DropProbe::new(&observer_drops, false));

    assert_eq!(control.disposal_failure(), None);
    assert_eq!(control.dispose_extensions(), Ok(()));
    assert!(control.cancellation.is_none());
    assert!(control.observer.is_none());
    assert_eq!(control.dispose_extensions(), Ok(()));
    assert_eq!(control.disposal_failure(), None);
    drop(control);

    assert_eq!(cancellation_drops.load(Ordering::Relaxed), 1);
    assert_eq!(observer_drops.load(Ordering::Relaxed), 1);
}

#[test]
fn empty_disposal_is_idempotent() {
    let mut control = OperationControl::<(), ()>::default();
    assert_eq!(control.dispose_extensions(), Ok(()));
    assert_eq!(control.dispose_extensions(), Ok(()));
    assert_eq!(control.disposal_failure(), None);
}

#[test]
fn both_panics_are_contained_and_first_failure_is_sticky() {
    let cancellation_drops = Arc::new(AtomicUsize::new(0));
    let observer_drops = Arc::new(AtomicUsize::new(0));
    let mut control = OperationControl::<(), ()>::default()
        .with_cancellation(DropProbe::new(&cancellation_drops, true))
        .with_progress_observer(DropProbe::new(&observer_drops, true));

    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(cancellation_drops.load(Ordering::Relaxed), 1);
    assert_eq!(observer_drops.load(Ordering::Relaxed), 1);
    assert!(control.cancellation.is_none());
    assert!(control.observer.is_none());
    assert_eq!(
        control.disposal_failure(),
        Some(&ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::CancellationSignalPanicked)
    );
    drop(control);
    assert_eq!(cancellation_drops.load(Ordering::Relaxed), 1);
    assert_eq!(observer_drops.load(Ordering::Relaxed), 1);
}

#[test]
fn observer_disposal_panic_is_reported_after_clean_cancellation() {
    let cancellation_drops = Arc::new(AtomicUsize::new(0));
    let observer_drops = Arc::new(AtomicUsize::new(0));
    let mut control = OperationControl::<(), ()>::default()
        .with_cancellation(DropProbe::new(&cancellation_drops, false))
        .with_progress_observer(DropProbe::new(&observer_drops, true));

    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::ObserverPanicked)
    );
    assert_eq!(cancellation_drops.load(Ordering::Relaxed), 1);
    assert_eq!(observer_drops.load(Ordering::Relaxed), 1);
}

#[test]
fn cancellation_replacement_retains_failure_and_disposes_new_signal() {
    let previous_drops = Arc::new(AtomicUsize::new(0));
    let replacement_drops = Arc::new(AtomicUsize::new(0));
    let mut control = OperationControl::<(), ()>::default()
        .with_cancellation(DropProbe::new(&previous_drops, true))
        .with_cancellation(DropProbe::new(&replacement_drops, false));

    assert_eq!(previous_drops.load(Ordering::Relaxed), 1);
    assert_eq!(replacement_drops.load(Ordering::Relaxed), 0);
    assert_eq!(
        control.disposal_failure(),
        Some(&ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(replacement_drops.load(Ordering::Relaxed), 1);
}

#[test]
fn observer_replacement_failure_precedes_later_cancellation_failure() {
    let previous_drops = Arc::new(AtomicUsize::new(0));
    let replacement_drops = Arc::new(AtomicUsize::new(0));
    let cancellation_drops = Arc::new(AtomicUsize::new(0));
    let mut control = OperationControl::<(), ()>::default()
        .with_progress_observer(DropProbe::new(&previous_drops, true))
        .with_progress_observer(DropProbe::new(&replacement_drops, true))
        .with_cancellation(DropProbe::new(&cancellation_drops, true));

    assert_eq!(previous_drops.load(Ordering::Relaxed), 1);
    assert_eq!(replacement_drops.load(Ordering::Relaxed), 0);
    assert_eq!(
        control.disposal_failure(),
        Some(&ControlCause::ObserverPanicked)
    );
    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::ObserverPanicked)
    );
    assert_eq!(replacement_drops.load(Ordering::Relaxed), 1);
    assert_eq!(cancellation_drops.load(Ordering::Relaxed), 1);
    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::ObserverPanicked)
    );
}

#[test]
fn discarded_cancellation_failure_survives_replacement() {
    let previous_drops = Arc::new(AtomicUsize::new(0));
    let replacement_drops = Arc::new(AtomicUsize::new(0));
    let mut control = OperationControl::<(), ()>::default()
        .with_cancellation(DropProbe::new(&previous_drops, true));

    control.discard_cancellation();
    control.discard_cancellation();
    assert_eq!(previous_drops.load(Ordering::Relaxed), 1);
    assert_eq!(
        control.disposal_failure(),
        Some(&ControlCause::CancellationSignalPanicked)
    );
    control = control.with_cancellation(DropProbe::new(&replacement_drops, false));
    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(replacement_drops.load(Ordering::Relaxed), 1);
}

#[test]
fn implicit_drop_contains_both_panics() {
    let cancellation_drops = Arc::new(AtomicUsize::new(0));
    let observer_drops = Arc::new(AtomicUsize::new(0));
    let control = OperationControl::<(), ()>::default()
        .with_cancellation(DropProbe::new(&cancellation_drops, true))
        .with_progress_observer(DropProbe::new(&observer_drops, true));

    assert!(catch_unwind(AssertUnwindSafe(|| drop(control))).is_ok());
    assert_eq!(cancellation_drops.load(Ordering::Relaxed), 1);
    assert_eq!(observer_drops.load(Ordering::Relaxed), 1);
}

struct OpaquePanicPayload {
    drops: Arc<AtomicUsize>,
}

impl Drop for OpaquePanicPayload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        panic!("opaque payload must not be dropped");
    }
}

struct OpaquePanicSignal {
    drops: Arc<AtomicUsize>,
    payload_drops: Arc<AtomicUsize>,
}

impl CancellationSignal for OpaquePanicSignal {
    fn is_cancelled(&self) -> bool {
        false
    }
}

impl Drop for OpaquePanicSignal {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        panic_any(OpaquePanicPayload {
            drops: Arc::clone(&self.payload_drops),
        });
    }
}

#[test]
fn opaque_panic_payload_is_not_dropped_and_observer_is_still_disposed() {
    let signal_drops = Arc::new(AtomicUsize::new(0));
    let payload_drops = Arc::new(AtomicUsize::new(0));
    let observer_drops = Arc::new(AtomicUsize::new(0));
    let mut control = OperationControl::<(), ()>::default()
        .with_cancellation(OpaquePanicSignal {
            drops: Arc::clone(&signal_drops),
            payload_drops: Arc::clone(&payload_drops),
        })
        .with_progress_observer(DropProbe::new(&observer_drops, true));

    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::CancellationSignalPanicked)
    );
    assert_eq!(
        control.dispose_extensions(),
        Err(ControlCause::CancellationSignalPanicked)
    );
    drop(control);
    assert_eq!(signal_drops.load(Ordering::Relaxed), 1);
    assert_eq!(observer_drops.load(Ordering::Relaxed), 1);
    assert_eq!(payload_drops.load(Ordering::Relaxed), 0);
}

#[test]
fn best_effort_discard_contains_opaque_payload_without_recursive_panic() {
    let signal_drops = Arc::new(AtomicUsize::new(0));
    let payload_drops = Arc::new(AtomicUsize::new(0));
    let signal = OpaquePanicSignal {
        drops: Arc::clone(&signal_drops),
        payload_drops: Arc::clone(&payload_drops),
    };

    assert!(catch_unwind(AssertUnwindSafe(|| discard_extension(signal))).is_ok());
    assert_eq!(signal_drops.load(Ordering::Relaxed), 1);
    assert_eq!(payload_drops.load(Ordering::Relaxed), 0);
}
