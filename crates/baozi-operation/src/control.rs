use crate::LedgerEvidenceError;
use std::fmt;
use std::panic::{AssertUnwindSafe, RefUnwindSafe, UnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Supplies a monotonic cooperative-cancellation signal.
pub trait CancellationSignal: Send + Sync + RefUnwindSafe + 'static {
    /// Returns whether cancellation has been requested.
    fn is_cancelled(&self) -> bool;
}

/// A cloneable cancellation signal that can be triggered by a host.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates an active token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. This transition cannot be reversed.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl CancellationSignal for CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

#[derive(Debug)]
struct NeverCancelled;

impl CancellationSignal for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Identifies the mechanism that requested cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancellationOrigin {
    /// The configured cancellation signal was triggered.
    Signal,
    /// The progress observer requested cancellation.
    Observer,
}

/// Describes an invalid progress transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgressViolation {
    /// A later report used an earlier phase.
    PhaseRegressed,
    /// Completed work decreased for one phase and unit.
    CompletedRegressed,
    /// The declared total decreased for one phase and unit.
    TotalRegressed,
    /// Completed work exceeded the declared total.
    CompletedExceededTotal,
}

/// A bounded semantic-progress state could not be retained safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProgressStorageError {
    /// The owner reported more distinct phase/unit pairs than the hard state bound.
    StateLimitExceeded { value: usize, limit: usize },
    /// Ledger-owned evidence admission or allocation rejected progress state growth.
    Evidence { cause: LedgerEvidenceError },
}

impl fmt::Display for ProgressStorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StateLimitExceeded { value, limit } => write!(
                formatter,
                "semantic progress state count reached {value}, above limit {limit}"
            ),
            Self::Evidence { cause } => write!(formatter, "progress evidence failed: {cause}"),
        }
    }
}

impl std::error::Error for ProgressStorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Evidence { cause } => Some(cause),
            Self::StateLimitExceeded { .. } => None,
        }
    }
}

/// An owner-neutral cause returned by operation control.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ControlCause {
    /// Cooperative cancellation was observed.
    Cancelled { origin: CancellationOrigin },
    /// The configured deadline elapsed.
    DeadlineExceeded,
    /// A progress observer panicked while unwinding was available.
    ObserverPanicked,
    /// A custom cancellation signal panicked while unwinding was available.
    CancellationSignalPanicked,
    /// An owner reported progress that violated the monotonic contract.
    InvalidProgress { violation: ProgressViolation },
    /// Bounded semantic progress state could not be retained.
    ProgressStorage { cause: ProgressStorageError },
}

impl fmt::Display for ControlCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled { origin } => write!(formatter, "operation cancelled by {origin:?}"),
            Self::DeadlineExceeded => formatter.write_str("operation deadline exceeded"),
            Self::ObserverPanicked => formatter.write_str("progress observer panicked"),
            Self::CancellationSignalPanicked => formatter.write_str("cancellation signal panicked"),
            Self::InvalidProgress { violation } => {
                write!(formatter, "invalid progress transition: {violation:?}")
            }
            Self::ProgressStorage { cause } => {
                write!(formatter, "progress storage failed: {cause}")
            }
        }
    }
}

impl std::error::Error for ControlCause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ProgressStorage { cause } => Some(cause),
            _ => None,
        }
    }
}

/// A minimal object-safe cooperative polling capability.
///
/// Core builders and validators can accept this interface without depending on an operation's
/// ledger dimensions, accounting phases, or progress vocabulary.
pub trait OperationPoll {
    /// Polls cancellation and deadline state.
    fn poll(&mut self) -> Result<(), ControlCause>;
}

/// Supplies stable owner-defined progress phase vocabulary.
pub trait ProgressPhase: Copy + Eq + Ord + fmt::Debug + Send + Sync + UnwindSafe + 'static {
    /// Returns the phase's order within its owning operation.
    fn order(self) -> u32;

    /// Returns the stable owner-defined identifier used for bounded delivery accounting.
    fn stable_id(self) -> &'static str;
}

/// Supplies stable owner-defined progress unit vocabulary.
pub trait ProgressUnit: Copy + Eq + Ord + fmt::Debug + Send + Sync + UnwindSafe + 'static {
    /// Returns the stable owner-defined identifier used for bounded delivery accounting.
    fn stable_id(self) -> &'static str;
}

/// A transient, owner-qualified progress event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressEvent<P, U> {
    phase: P,
    unit: U,
    completed: u64,
    total: Option<u64>,
    sequence: u64,
}

impl<P: Copy, U: Copy> ProgressEvent<P, U> {
    pub(crate) const fn new(
        phase: P,
        unit: U,
        completed: u64,
        total: Option<u64>,
        sequence: u64,
    ) -> Self {
        Self {
            phase,
            unit,
            completed,
            total,
            sequence,
        }
    }

    /// Returns the owner-defined operation phase.
    pub const fn phase(&self) -> P {
        self.phase
    }

    /// Returns the owner-defined work unit.
    pub const fn unit(&self) -> U {
        self.unit
    }

    /// Returns monotonically increasing completed work.
    pub const fn completed(&self) -> u64 {
        self.completed
    }

    /// Returns a nondecreasing total when one is known.
    pub const fn total(&self) -> Option<u64> {
        self.total
    }

    /// Returns the operation-local delivered-event sequence.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Controls whether an operation continues after progress delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgressDecision {
    /// Continue the operation.
    Continue,
    /// Request cooperative cancellation.
    Cancel,
}

/// Receives progress serially through an operation coordinator.
pub trait ProgressObserver<P, U>: Send + UnwindSafe + 'static {
    /// Observes one bounded progress event.
    fn observe(&mut self, event: ProgressEvent<P, U>) -> ProgressDecision;
}

impl<P, U, F> ProgressObserver<P, U> for F
where
    F: FnMut(ProgressEvent<P, U>) -> ProgressDecision + Send + UnwindSafe + 'static,
{
    fn observe(&mut self, event: ProgressEvent<P, U>) -> ProgressDecision {
        self(event)
    }
}

/// Hard bounds and coalescing interval for transient progress delivery.
///
/// The single final publication event is delivered outside these transient limits so an observer's
/// final cancellation decision cannot be bypassed by earlier progress traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressDeliveryLimits {
    max_events: usize,
    max_encoded_bytes: usize,
    min_work_delta: u64,
    max_states: usize,
}

const MAX_PROGRESS_STATE_COUNT: usize = 4_096;

impl ProgressDeliveryLimits {
    /// Creates delivery limits.
    pub const fn new(max_events: usize, max_encoded_bytes: usize, min_work_delta: u64) -> Self {
        Self {
            max_events,
            max_encoded_bytes,
            min_work_delta,
            max_states: MAX_PROGRESS_STATE_COUNT,
        }
    }

    /// Tightens the number of retained semantic phase/unit states.
    pub const fn with_max_states(mut self, max_states: usize) -> Self {
        self.max_states = if max_states == 0 {
            1
        } else if max_states > MAX_PROGRESS_STATE_COUNT {
            MAX_PROGRESS_STATE_COUNT
        } else {
            max_states
        };
        self
    }

    pub(crate) const fn max_events(self) -> usize {
        self.max_events
    }

    pub(crate) const fn max_encoded_bytes(self) -> usize {
        self.max_encoded_bytes
    }

    pub(crate) const fn min_work_delta(self) -> u64 {
        self.min_work_delta
    }

    pub(crate) const fn max_states(self) -> usize {
        self.max_states
    }
}

impl Default for ProgressDeliveryLimits {
    fn default() -> Self {
        Self::new(4_096, 256 * 1024, 1)
    }
}

/// Live operation capabilities, separate from frozen semantic configuration.
pub struct OperationControl<P, U> {
    pub(crate) cancellation: Arc<dyn CancellationSignal>,
    pub(crate) observer: Option<Box<dyn ProgressObserver<P, U>>>,
    pub(crate) deadline: Option<Instant>,
    pub(crate) progress_limits: ProgressDeliveryLimits,
}

impl<P, U> OperationControl<P, U> {
    /// Replaces the no-op cancellation signal.
    pub fn with_cancellation(mut self, cancellation: impl CancellationSignal) -> Self {
        let previous = std::mem::replace(&mut self.cancellation, Arc::new(cancellation));
        discard_extension(previous);
        self
    }

    /// Installs the operation-local progress observer.
    pub fn with_progress_observer(
        mut self,
        observer: impl ProgressObserver<P, U> + 'static,
    ) -> Self {
        if let Some(previous) = self.observer.replace(Box::new(observer)) {
            discard_extension(previous);
        }
        self
    }

    /// Installs a deadline backstop.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Replaces the default progress delivery bounds.
    pub fn with_progress_limits(mut self, limits: ProgressDeliveryLimits) -> Self {
        self.progress_limits = limits;
        self
    }

    pub(crate) fn discard_cancellation(&mut self) {
        let cancellation = std::mem::replace(&mut self.cancellation, Arc::new(NeverCancelled));
        discard_extension(cancellation);
    }
}

impl<P, U> Default for OperationControl<P, U> {
    fn default() -> Self {
        Self {
            cancellation: Arc::new(NeverCancelled),
            observer: None,
            deadline: None,
            progress_limits: ProgressDeliveryLimits::default(),
        }
    }
}

impl<P, U> Drop for OperationControl<P, U> {
    fn drop(&mut self) {
        let cancellation = std::mem::replace(&mut self.cancellation, Arc::new(NeverCancelled));
        discard_extension(cancellation);
        if let Some(observer) = self.observer.take() {
            discard_extension(observer);
        }
    }
}

pub(crate) fn discard_untrusted_panic_payload(payload: Box<dyn std::any::Any + Send>) {
    if payload.is::<String>() || payload.is::<&'static str>() {
        drop(payload);
    } else {
        // An opaque extension payload may panic again from Drop; leaking it contains that boundary.
        std::mem::forget(payload);
    }
}

pub(crate) fn discard_extension<T>(extension: T) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(extension))) {
        discard_untrusted_panic_payload(payload);
    }
}
