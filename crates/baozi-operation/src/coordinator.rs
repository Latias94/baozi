use crate::OperationOwner;
use crate::control::{
    CancellationOrigin, ControlCause, OperationControl, OperationPoll, ProgressDecision,
    ProgressEvent, ProgressPhase, ProgressStorageError, ProgressUnit, ProgressViolation,
    discard_extension, discard_untrusted_panic_payload,
};
use crate::ledger::{
    Commitment, Ledger, LedgerCause, LedgerEvidenceError, LedgerLimits, LedgerSnapshot,
    LiveReservation, PendingPublication, ProgressEvidenceStorage, PublicationReservationBatch,
};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

/// The one final progress event delivered immediately before durable publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalProgress<P, U> {
    phase: P,
    unit: U,
    total: u64,
}

impl<P: Copy, U: Copy> FinalProgress<P, U> {
    /// Defines the owner-specific final phase, work unit, and completed total.
    pub const fn new(phase: P, unit: U, total: u64) -> Self {
        Self { phase, unit, total }
    }

    /// Returns the owner-defined final progress phase.
    pub const fn phase(self) -> P {
        self.phase
    }

    /// Returns the owner-defined final progress unit.
    pub const fn unit(self) -> U {
        self.unit
    }

    /// Returns the completed total delivered as both `completed` and `total`.
    pub const fn total(self) -> u64 {
        self.total
    }
}

/// A failure at the final control-to-publication boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PublicationCause<D> {
    /// Final progress delivery or the last cancellation/deadline poll failed.
    Control(ControlCause),
    /// The pending transaction did not belong to this ledger or could not publish atomically.
    Ledger(LedgerCause<D>),
}

impl<D: fmt::Debug> fmt::Display for PublicationCause<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Control(cause) => write!(formatter, "publication control failed: {cause}"),
            Self::Ledger(cause) => write!(formatter, "publication ledger failed: {cause}"),
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for PublicationCause<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Control(cause) => Some(cause),
            Self::Ledger(cause) => Some(cause),
        }
    }
}

impl<D> From<ControlCause> for PublicationCause<D> {
    fn from(cause: ControlCause) -> Self {
        Self::Control(cause)
    }
}

impl<D> From<LedgerCause<D>> for PublicationCause<D> {
    fn from(cause: LedgerCause<D>) -> Self {
        Self::Ledger(cause)
    }
}

/// A terminal operation failure with the exact final accounting evidence.
#[derive(Debug, PartialEq, Eq)]
pub struct OperationFinishError<P, D> {
    cause: PublicationCause<D>,
    snapshot: LedgerSnapshot<P, D>,
}

impl<P, D> OperationFinishError<P, D> {
    /// Returns the control or accounting failure that prevented clean finalization.
    pub const fn cause(&self) -> &PublicationCause<D> {
        &self.cause
    }

    /// Returns the exact evidence after extension owners have been disposed.
    pub const fn snapshot(&self) -> &LedgerSnapshot<P, D> {
        &self.snapshot
    }

    /// Separates the failure from its final accounting evidence.
    pub fn into_parts(self) -> (PublicationCause<D>, LedgerSnapshot<P, D>) {
        (self.cause, self.snapshot)
    }
}

impl<P, D: fmt::Debug> fmt::Display for OperationFinishError<P, D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "operation finalization failed: {}", self.cause)
    }
}

impl<P: fmt::Debug + 'static, D: fmt::Debug + 'static> std::error::Error
    for OperationFinishError<P, D>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

#[derive(Debug, Clone, Copy)]
struct ProgressState {
    completed: u64,
    total: Option<u64>,
    last_delivered: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgressDelivery {
    Transient,
    Final,
}

/// Poll-only control authority whose borrow is disjoint from the operation ledger.
///
/// Retained construction uses this private checkpoint while the same coordinator's ledger is
/// borrowed independently for accounting. It deliberately excludes progress, reservation, and
/// publication authority.
pub(crate) struct OperationPollCheckpoint<'a, PP, U> {
    control: &'a mut OperationControl<PP, U>,
    terminal: &'a mut Option<ControlCause>,
    allow_preexisting_terminal: bool,
}

impl<PP, U> OperationPollCheckpoint<'_, PP, U> {
    pub(crate) fn reborrow(&mut self) -> OperationPollCheckpoint<'_, PP, U> {
        OperationPollCheckpoint {
            control: self.control,
            terminal: self.terminal,
            allow_preexisting_terminal: self.allow_preexisting_terminal,
        }
    }

    pub(crate) fn recorded_control_cause(&self) -> Option<ControlCause> {
        self.terminal.clone()
    }

    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        if self.allow_preexisting_terminal {
            Ok(())
        } else {
            poll_operation_control(self.control, self.terminal)
        }
    }
}

/// Progress authority lent independently of the invocation-local work owner.
///
/// A producer can retain a borrowed work envelope while polling or reporting progress.
/// No callback runs while either accounting state is borrowed.
pub(crate) struct OperationProgressCheckpoint<'a, O, LP, PP, U, D> {
    poll: OperationPollCheckpoint<'a, PP, U>,
    state: OperationProgressState<'a, O, LP, PP, U, D>,
}

/// Progress metadata can move independently of the invocation's unique control borrow.
pub(crate) struct OperationProgressState<'a, O, LP, PP, U, D> {
    storage: ProgressEvidenceStorage<'a, O, LP, D>,
    progress: &'a mut Vec<((PP, U), ProgressState)>,
    progress_evidence_bytes: &'a mut u64,
    last_phase_order: &'a mut Option<u32>,
    delivered_events: &'a mut usize,
    delivered_bytes: &'a mut usize,
    next_sequence: &'a mut u64,
}

/// Progress metadata before its storage-only view is bound to the invocation's real storage.
pub(crate) struct OperationProgressFields<'a, PP, U> {
    progress: &'a mut Vec<((PP, U), ProgressState)>,
    progress_evidence_bytes: &'a mut u64,
    last_phase_order: &'a mut Option<u32>,
    delivered_events: &'a mut usize,
    delivered_bytes: &'a mut usize,
    next_sequence: &'a mut u64,
}

impl<'a, PP, U> OperationProgressFields<'a, PP, U> {
    pub(crate) fn bind<O, LP, D>(
        self,
        storage: ProgressEvidenceStorage<'a, O, LP, D>,
    ) -> OperationProgressState<'a, O, LP, PP, U, D> {
        let Self {
            progress,
            progress_evidence_bytes,
            last_phase_order,
            delivered_events,
            delivered_bytes,
            next_sequence,
        } = self;
        OperationProgressState {
            storage,
            progress,
            progress_evidence_bytes,
            last_phase_order,
            delivered_events,
            delivered_bytes,
            next_sequence,
        }
    }
}

type SplitExclusiveProgress<'a, O, LP, PP, U, D> = (
    OperationPollCheckpoint<'a, PP, U>,
    OperationProgressFields<'a, PP, U>,
    &'a mut Ledger<O, LP, D>,
);

impl<'a, O, LP, PP, U, D> OperationProgressCheckpoint<'a, O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn report_progress_with_delivery(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
        delivery: ProgressDelivery,
    ) -> Result<(), ControlCause> {
        self.state.report_progress_with_delivery(
            &mut self.poll,
            phase,
            unit,
            completed,
            total,
            delivery,
        )
    }
}

impl<O, LP, PP, U, D> OperationProgressState<'_, O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn report_progress(
        &mut self,
        poll: &mut OperationPollCheckpoint<'_, PP, U>,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ControlCause> {
        self.report_progress_with_delivery(
            poll,
            phase,
            unit,
            completed,
            total,
            ProgressDelivery::Transient,
        )
    }

    fn report_progress_with_delivery(
        &mut self,
        poll: &mut OperationPollCheckpoint<'_, PP, U>,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
        delivery: ProgressDelivery,
    ) -> Result<(), ControlCause> {
        poll.poll()?;
        if total.is_some_and(|total| completed > total) {
            return self.fail(
                poll,
                ControlCause::InvalidProgress {
                    violation: ProgressViolation::CompletedExceededTotal,
                },
            );
        }
        if self
            .last_phase_order
            .is_some_and(|last| phase.order() < last)
        {
            return self.fail(
                poll,
                ControlCause::InvalidProgress {
                    violation: ProgressViolation::PhaseRegressed,
                },
            );
        }

        let key = (phase, unit);
        let progress_position = self
            .progress
            .binary_search_by_key(&key, |(stored_key, _)| *stored_key);
        let previous = progress_position
            .ok()
            .map(|position| self.progress[position].1);
        let unchanged = previous
            .is_some_and(|previous| previous.completed == completed && previous.total == total);
        if let Some(previous) = previous {
            if completed < previous.completed {
                return self.fail(
                    poll,
                    ControlCause::InvalidProgress {
                        violation: ProgressViolation::CompletedRegressed,
                    },
                );
            }
            if matches!((previous.total, total), (Some(old), Some(new)) if new < old)
                || matches!((previous.total, total), (Some(_), None))
            {
                return self.fail(
                    poll,
                    ControlCause::InvalidProgress {
                        violation: ProgressViolation::TotalRegressed,
                    },
                );
            }
        }

        let previous_delivery = previous.and_then(|state| state.last_delivered);
        let should_deliver = match delivery {
            ProgressDelivery::Transient => {
                !unchanged
                    && previous_delivery.is_none_or(|previous| {
                        completed.saturating_sub(previous)
                            >= poll.control.progress_limits.min_work_delta()
                            || total == Some(completed)
                    })
            }
            ProgressDelivery::Final => true,
        };
        let progress_state = ProgressState {
            completed,
            total,
            last_delivered: previous_delivery,
        };
        let progress_position = match progress_position {
            Ok(position) => {
                self.progress[position].1 = progress_state;
                position
            }
            Err(position) => {
                if let Err(cause) = self.try_insert_progress(
                    position,
                    key,
                    progress_state,
                    poll.control.progress_limits.max_states(),
                ) {
                    return self.fail(poll, ControlCause::ProgressStorage { cause });
                }
                position
            }
        };
        *self.last_phase_order = Some(phase.order());

        if !should_deliver || poll.control.observer.is_none() {
            return Ok(());
        }
        let encoded_bytes = match delivery {
            ProgressDelivery::Transient => {
                if *self.delivered_events >= poll.control.progress_limits.max_events() {
                    return Ok(());
                }
                let encoded_bytes = estimated_event_bytes(phase, unit);
                if self.delivered_bytes.saturating_add(encoded_bytes)
                    > poll.control.progress_limits.max_encoded_bytes()
                {
                    return Ok(());
                }
                Some(encoded_bytes)
            }
            ProgressDelivery::Final => None,
        };

        let event = ProgressEvent::new(phase, unit, completed, total, *self.next_sequence);
        let Some(mut observer) = poll.control.observer.take() else {
            return Ok(());
        };
        let decision = catch_unwind(AssertUnwindSafe(|| observer.observe(event)));
        let decision = match decision {
            Ok(decision) => {
                poll.control.observer = Some(observer);
                decision
            }
            Err(payload) => {
                discard_extension(observer);
                discard_untrusted_panic_payload(payload);
                return self.fail(poll, ControlCause::ObserverPanicked);
            }
        };

        if let Some(encoded_bytes) = encoded_bytes {
            *self.delivered_events = self.delivered_events.saturating_add(1);
            *self.delivered_bytes = self.delivered_bytes.saturating_add(encoded_bytes);
        }
        *self.next_sequence = self.next_sequence.saturating_add(1);
        self.progress[progress_position].1.last_delivered = Some(completed);
        if decision == ProgressDecision::Cancel {
            return self.fail(
                poll,
                ControlCause::Cancelled {
                    origin: CancellationOrigin::Observer,
                },
            );
        }
        Ok(())
    }

    fn fail(
        &mut self,
        poll: &mut OperationPollCheckpoint<'_, PP, U>,
        cause: ControlCause,
    ) -> Result<(), ControlCause> {
        *poll.terminal = Some(cause.clone());
        Err(cause)
    }

    fn try_insert_progress(
        &mut self,
        position: usize,
        key: (PP, U),
        state: ProgressState,
        limit: usize,
    ) -> Result<(), ProgressStorageError> {
        let desired_capacity = self.progress.len().saturating_add(1);
        if desired_capacity > limit {
            return Err(ProgressStorageError::StateLimitExceeded {
                value: desired_capacity,
                limit,
            });
        }
        self.storage
            .try_grow(
                self.progress,
                desired_capacity,
                self.progress_evidence_bytes,
            )
            .map_err(|cause| ProgressStorageError::Evidence { cause })?;
        self.progress.insert(position, (key, state));
        Ok(())
    }
}

/// Serializes operation control and owns one generic ledger.
///
/// Ledger phases (`LP`) and progress phases (`PP`) are deliberately distinct vocabularies. Import
/// and post-process lifecycle evidence is owned above this module and is not represented here.
///
/// ```compile_fail
/// use baozi_operation::{
///     DimensionPolicy, LedgerLimits, OperationControl, OperationCoordinator, OperationOwner,
///     ProgressPhase, ProgressUnit,
/// };
///
/// struct Owner;
/// impl OperationOwner for Owner {}
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum LedgerPhase { Build }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum ObserverPhase { Parse }
/// impl ProgressPhase for ObserverPhase {
///     fn order(self) -> u32 { 0 }
///     fn stable_id(self) -> &'static str { "parse" }
/// }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Unit { Work }
/// impl ProgressUnit for Unit {
///     fn stable_id(self) -> &'static str { "work" }
/// }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Dimension { Bytes }
///
/// let mut coordinator = OperationCoordinator::<
///     Owner, LedgerPhase, ObserverPhase, Unit, Dimension,
/// >::try_new(
///     OperationControl::default(),
///     LedgerLimits::new(
///         8,
///         [(Dimension::Bytes, DimensionPolicy::reservable_bytes(8, 8, 8, 8))],
///     ).unwrap(),
///     &[LedgerPhase::Build],
/// ).unwrap();
/// coordinator.report_progress(LedgerPhase::Build, Unit::Work, 1, None).unwrap();
/// ```
pub struct OperationCoordinator<O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    control: OperationControl<PP, U>,
    ledger: Ledger<O, LP, D>,
    terminal: Option<ControlCause>,
    progress: Vec<((PP, U), ProgressState)>,
    progress_evidence_bytes: u64,
    last_phase_order: Option<u32>,
    delivered_events: usize,
    delivered_bytes: usize,
    next_sequence: u64,
    terminal_control_gate: Option<Result<(), PublicationCause<D>>>,
}

impl<O, LP, PP, U, D> OperationCoordinator<O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    /// Creates one coordinator with frozen initial policy and a complete phase vocabulary.
    pub fn try_new(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        phases: &[LP],
    ) -> Result<Self, LedgerCause<D>> {
        Ok(Self {
            control,
            ledger: Ledger::try_new(ledger_limits, phases)?,
            terminal: None,
            progress: Vec::new(),
            progress_evidence_bytes: 0,
            last_phase_order: None,
            delivered_events: 0,
            delivered_bytes: 0,
            next_sequence: 0,
            terminal_control_gate: None,
        })
    }

    /// Reads the original cold control cause without polling or changing control state.
    pub(crate) fn recorded_control_cause(&self) -> Option<ControlCause> {
        self.terminal.clone()
    }

    /// Polls cancellation and deadline state.
    pub fn poll(&mut self) -> Result<(), ControlCause> {
        poll_operation_control(&mut self.control, &mut self.terminal)
    }

    /// Validates, coalesces, and serially delivers owner-defined progress.
    pub fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ControlCause> {
        self.report_progress_with_delivery(
            phase,
            unit,
            completed,
            total,
            ProgressDelivery::Transient,
        )
    }

    fn report_final_progress(
        &mut self,
        phase: PP,
        unit: U,
        total: u64,
    ) -> Result<(), ControlCause> {
        self.report_progress_with_delivery(phase, unit, total, Some(total), ProgressDelivery::Final)
    }

    fn report_progress_with_delivery(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
        delivery: ProgressDelivery,
    ) -> Result<(), ControlCause> {
        self.split_progress_checkpoint_and_retained_ledger()
            .0
            .report_progress_with_delivery(phase, unit, completed, total, delivery)
    }

    /// Returns the latest semantic progress report, including reports not delivered to an observer.
    pub fn reported_progress(&self, phase: PP, unit: U) -> Option<(u64, Option<u64>)> {
        self.progress
            .binary_search_by_key(&(phase, unit), |(key, _)| *key)
            .ok()
            .map(|position| {
                let state = self.progress[position].1;
                (state.completed, state.total)
            })
    }

    /// Borrows a capability-limited checkpoint for parser, builder, or validator work.
    pub fn checkpoint(&mut self) -> OperationCheckpoint<'_, O, LP, PP, U, D> {
        OperationCheckpoint { coordinator: self }
    }

    /// Fallibly captures immutable owner-neutral ledger evidence.
    ///
    /// Snapshot storage is admitted against the independent evidence limit before allocator entry.
    /// Its construction overlap contributes to the evidence peak; ownership transfers to the
    /// returned snapshot, so its own vectors are excluded from current ledger-owned evidence.
    pub fn try_ledger_snapshot(&self) -> Result<LedgerSnapshot<LP, D>, LedgerEvidenceError> {
        self.ledger.try_snapshot()
    }

    /// Copies one dimension's scalar evidence without allocating a full snapshot.
    pub fn dimension_evidence(
        &self,
        dimension: D,
    ) -> Result<crate::DimensionSnapshot, LedgerCause<D>> {
        self.ledger.dimension_evidence(dimension)
    }

    /// Monotonically tightens one dimension on this coordinator's only ledger.
    ///
    /// Numerical fields are combined with the existing policy using field-wise minima. The
    /// operation remains bound to the same state and owner; prior evidence is never rewritten.
    pub fn tighten_dimension_policy(
        &mut self,
        dimension: D,
        upper_bound: crate::DimensionPolicy,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger.tighten_dimension_policy(dimension, upper_bound)
    }

    /// Copies one ledger phase's scalar evidence without allocating a full snapshot.
    pub fn phase_evidence(&self, phase: LP) -> Option<crate::PhaseSnapshot> {
        self.ledger.phase_evidence(phase)
    }

    /// Resizes one reservation while preserving it unchanged on admission failure.
    pub fn resize_reservation(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger.ensure_work_open()?;
        self.ledger.resize_reservation(reservation, bytes)
    }

    /// Atomically replaces a reservation's phase, dimension, and byte capacity.
    ///
    /// A failed transition preserves the source token and all accounting unchanged.
    pub fn transition_reservation(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        phase: LP,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger.ensure_work_open()?;
        self.ledger
            .transition_reservation(reservation, phase, dimension, bytes)
    }

    /// Atomically reclassifies a reservation batch, preserving every source on failure.
    pub fn transition_reservations(
        &mut self,
        reservations: &mut [LiveReservation<O, LP, D>],
        phase: LP,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger.ensure_work_open()?;
        self.ledger
            .transition_reservations(reservations, phase, dimension, bytes)
    }

    /// Begins an atomic publication transaction without changing durable committed evidence.
    ///
    /// Success empties `reservations`. Failure leaves it untouched so a storage owner can keep the
    /// reservation alive until the covered storage has been discarded or transferred.
    pub fn prepare_publication(
        &mut self,
        reservations: &mut PublicationReservationBatch<O, LP, D>,
        commitments: &[Commitment<D>],
    ) -> Result<PendingPublication<O, LP, D>, LedgerCause<D>> {
        self.ledger.ensure_work_open()?;
        self.ledger.prepare_publication(reservations, commitments)
    }

    /// Delivers final progress, performs the last control poll, and atomically publishes output.
    ///
    /// On error `publication` remains active. The caller must retain it until the covered output
    /// storage has been dropped, then explicitly roll it back or let it drop. A final observer may
    /// already have seen the event, so callers must not retry blindly.
    pub fn publish(
        &mut self,
        publication: &mut PendingPublication<O, LP, D>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<(), PublicationCause<D>> {
        self.ledger.check_publication(publication)?;
        self.try_prepare_typed_control(final_progress)?;
        self.ledger.close_work()?;
        self.ledger.publish(publication)?;
        Ok(())
    }

    /// Splits a reservation without changing live or peak accounting.
    pub fn split_reservation(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        bytes: u64,
    ) -> Result<LiveReservation<O, LP, D>, LedgerCause<D>> {
        self.ledger.ensure_work_open()?;
        self.ledger.split(reservation, bytes)
    }

    /// Verifies that a live reservation belongs to this operation without mutating accounting.
    pub fn validate_live_reservation(
        &self,
        reservation: &LiveReservation<O, LP, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger.validate_live_reservation(reservation)
    }

    pub(crate) fn retained_ledger(&self) -> &Ledger<O, LP, D> {
        &self.ledger
    }

    pub(crate) fn retained_ledger_mut(&mut self) -> &mut Ledger<O, LP, D> {
        &mut self.ledger
    }

    #[allow(
        clippy::type_complexity,
        reason = "the tuple exposes the concrete disjoint ledger and progress borrows"
    )]
    pub(crate) fn split_progress_checkpoint_and_retained_ledger(
        &mut self,
    ) -> (
        OperationProgressCheckpoint<'_, O, LP, PP, U, D>,
        &Ledger<O, LP, D>,
    ) {
        let (poll, progress, ledger) = self.split_exclusive_progress_and_ledger();
        let ledger = &*ledger;
        (
            OperationProgressCheckpoint {
                poll,
                state: progress.bind(ledger.storage_view().progress_evidence()),
            },
            ledger,
        )
    }

    pub(crate) fn split_poll_checkpoint_and_retained_ledger(
        &mut self,
    ) -> (OperationPollCheckpoint<'_, PP, U>, &mut Ledger<O, LP, D>) {
        let Self {
            control,
            ledger,
            terminal,
            ..
        } = self;
        (
            OperationPollCheckpoint {
                control,
                terminal,
                allow_preexisting_terminal: false,
            },
            ledger,
        )
    }

    pub(crate) fn split_exclusive_progress_and_ledger(
        &mut self,
    ) -> SplitExclusiveProgress<'_, O, LP, PP, U, D> {
        let Self {
            control,
            ledger,
            terminal,
            progress,
            progress_evidence_bytes,
            last_phase_order,
            delivered_events,
            delivered_bytes,
            next_sequence,
            ..
        } = self;
        (
            OperationPollCheckpoint {
                control,
                terminal,
                allow_preexisting_terminal: false,
            },
            OperationProgressFields {
                progress,
                progress_evidence_bytes,
                last_phase_order,
                delivered_events,
                delivered_bytes,
                next_sequence,
            },
            ledger,
        )
    }

    /// Splits the bounded terminal-work poll from the retained ledger.
    ///
    /// A control failure already accepted before this split cannot cancel the finite evidence
    /// cleanup it selected. If control was still live at the split, later cancellation and
    /// deadline failures remain observable at every protected-work poll.
    pub(crate) fn split_terminal_poll_checkpoint_and_retained_ledger(
        &mut self,
    ) -> (OperationPollCheckpoint<'_, PP, U>, &mut Ledger<O, LP, D>) {
        let Self {
            control,
            terminal,
            ledger,
            ..
        } = self;
        let allow_preexisting_terminal = terminal.is_some();
        (
            OperationPollCheckpoint {
                control,
                terminal,
                allow_preexisting_terminal,
            },
            ledger,
        )
    }

    pub(crate) fn try_prepare_typed_control(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<(), PublicationCause<D>> {
        if let Some(result) = &self.terminal_control_gate {
            return result.clone();
        }
        let delivery = match self.ledger.seal_ordinary_work() {
            Ok(()) => self
                .report_final_progress(
                    final_progress.phase,
                    final_progress.unit,
                    final_progress.total,
                )
                .and_then(|()| self.poll())
                .map_err(PublicationCause::Control),
            Err(cause) => Err(PublicationCause::Ledger(cause)),
        };
        // A rejected gate leaves ordinary work sealed and the finite failure grant dormant.
        // Dispose both extension owners even if sealing or final delivery failed.
        let disposal = self
            .dispose_control_owners()
            .map_err(PublicationCause::Control);
        let result = delivery.and(disposal);
        self.terminal_control_gate = Some(result.clone());
        result
    }

    fn dispose_control_owners(&mut self) -> Result<(), ControlCause> {
        if let Err(cause) = self.control.dispose_extensions() {
            if self.terminal.is_none() {
                self.terminal = Some(cause.clone());
            }
            return Err(cause);
        }
        Ok(())
    }

    pub(crate) fn close_for_terminal_extraction(&mut self) -> Result<(), PublicationCause<D>> {
        let seal = self.ledger.seal_ordinary_work();
        let disposal = self.dispose_control_owners();
        let close = if seal.is_ok() {
            self.ledger.close_work()
        } else {
            Ok(())
        };
        seal?;
        disposal?;
        close?;
        Ok(())
    }

    pub(crate) fn into_ledger_for_typed_publication(self) -> Ledger<O, LP, D> {
        let Self {
            control,
            ledger,
            progress,
            progress_evidence_bytes,
            ..
        } = self;
        drop(progress);
        ledger.release_progress_evidence(progress_evidence_bytes);
        drop(control);
        ledger
    }

    /// Finalizes the generic ledger after operation-owned live storage reaches zero.
    ///
    /// An invariant failure retains the exact non-normalized snapshot instead of fabricating clean
    /// evidence or replacing an owning domain's original failure cause.
    #[allow(
        clippy::result_large_err,
        reason = "boxing final evidence would allocate on the invariant-failure path"
    )]
    pub fn finish(mut self) -> Result<LedgerSnapshot<LP, D>, OperationFinishError<LP, D>> {
        let disposal = self.dispose_control_owners();
        let control_cause = self.terminal.clone().or_else(|| disposal.err());
        self.progress = Vec::new();
        self.ledger
            .release_progress_evidence(self.progress_evidence_bytes);
        self.progress_evidence_bytes = 0;
        let snapshot = self.ledger.finish().map_err(|failure| {
            let (cause, snapshot) = failure.into_parts();
            OperationFinishError {
                cause: PublicationCause::Ledger(cause),
                snapshot,
            }
        })?;
        if let Some(cause) = control_cause {
            return Err(OperationFinishError {
                cause: PublicationCause::Control(cause),
                snapshot,
            });
        }
        Ok(snapshot)
    }
}

fn poll_operation_control<PP, U>(
    control: &mut OperationControl<PP, U>,
    terminal: &mut Option<ControlCause>,
) -> Result<(), ControlCause> {
    if let Some(cause) = terminal.clone() {
        return Err(cause);
    }
    if let Some(cause) = control.disposal_failure().cloned() {
        return fail_operation_control(terminal, cause);
    }
    let cancellation = control.cancellation.as_ref().map(Arc::clone);
    let cancelled = catch_unwind(move || cancellation.is_some_and(|signal| signal.is_cancelled()));
    let cancelled = match cancelled {
        Ok(cancelled) => cancelled,
        Err(payload) => {
            control.discard_cancellation();
            discard_untrusted_panic_payload(payload);
            return fail_operation_control(terminal, ControlCause::CancellationSignalPanicked);
        }
    };
    if cancelled {
        return fail_operation_control(
            terminal,
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            },
        );
    }
    if control
        .deadline
        .is_some_and(|deadline| Instant::now() >= deadline)
    {
        return fail_operation_control(terminal, ControlCause::DeadlineExceeded);
    }
    Ok(())
}

fn fail_operation_control(
    terminal: &mut Option<ControlCause>,
    cause: ControlCause,
) -> Result<(), ControlCause> {
    *terminal = Some(cause.clone());
    Err(cause)
}

/// A borrowed low-authority view over one operation coordinator.
pub struct OperationCheckpoint<'a, O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    coordinator: &'a mut OperationCoordinator<O, LP, PP, U, D>,
}

impl<O, LP, PP, U, D> OperationCheckpoint<'_, O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    /// Polls cancellation and deadline state.
    pub fn poll(&mut self) -> Result<(), ControlCause> {
        self.coordinator.poll()
    }

    /// Reports bounded owner-defined progress.
    pub fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ControlCause> {
        self.coordinator
            .report_progress(phase, unit, completed, total)
    }

    /// Charges non-refundable attempted work.
    pub fn charge_attempted(&mut self, dimension: D, amount: u64) -> Result<(), LedgerCause<D>> {
        self.coordinator.ledger.charge_attempted(dimension, amount)
    }

    /// Charges non-refundable consumed work.
    pub fn charge_consumed(&mut self, dimension: D, amount: u64) -> Result<(), LedgerCause<D>> {
        self.coordinator.ledger.charge_consumed(dimension, amount)
    }

    /// Charges ordered attempted-and-consumed pairs while preserving non-refundable prefixes.
    pub fn charge_observed_batch(&mut self, charges: &[(D, u64)]) -> Result<(), LedgerCause<D>> {
        self.coordinator.ledger.charge_observed_batch(charges)
    }

    /// Atomically charges completed work across multiple dimensions.
    pub fn charge_consumed_batch<const N: usize>(
        &mut self,
        charges: &[(D, u64); N],
    ) -> Result<(), LedgerCause<D>> {
        self.coordinator.ledger.charge_consumed_batch(charges)
    }

    /// Reserves live capacity before allocation or backend entry.
    pub fn reserve(
        &mut self,
        phase: LP,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, LP, D>, LedgerCause<D>> {
        self.coordinator.ledger.ensure_work_open()?;
        self.coordinator.ledger.reserve(phase, dimension, bytes)
    }

    /// Resizes one reservation while preserving it unchanged on admission failure.
    pub fn resize_reservation(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.coordinator
            .ledger
            .resize_reservation(reservation, bytes)
    }

    /// Atomically replaces a reservation's phase, dimension, and byte capacity.
    ///
    /// A failed transition preserves the source token and all accounting unchanged.
    pub fn transition_reservation(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        phase: LP,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.coordinator
            .ledger
            .transition_reservation(reservation, phase, dimension, bytes)
    }

    /// Atomically reclassifies a reservation batch, preserving every source on failure.
    pub fn transition_reservations(
        &mut self,
        reservations: &mut [LiveReservation<O, LP, D>],
        phase: LP,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.coordinator
            .ledger
            .transition_reservations(reservations, phase, dimension, bytes)
    }

    /// Splits a reservation without changing live or peak accounting.
    pub fn split(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        bytes: u64,
    ) -> Result<LiveReservation<O, LP, D>, LedgerCause<D>> {
        self.coordinator.ledger.split(reservation, bytes)
    }
}

impl<O, LP, PP, U, D> OperationPoll for OperationCheckpoint<'_, O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn poll(&mut self) -> Result<(), ControlCause> {
        self.coordinator.poll()
    }
}

fn estimated_event_bytes<P: ProgressPhase, U: ProgressUnit>(phase: P, unit: U) -> usize {
    phase
        .stable_id()
        .len()
        .saturating_add(unit.stable_id().len())
        .saturating_add(32)
}

#[cfg(test)]
mod tests;
