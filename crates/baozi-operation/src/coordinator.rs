use crate::OperationOwner;
use crate::control::{
    CancellationOrigin, ControlCause, OperationControl, OperationPoll, ProgressDecision,
    ProgressEvent, ProgressPhase, ProgressStorageError, ProgressUnit, ProgressViolation,
    discard_extension, discard_untrusted_panic_payload,
};
use crate::ledger::{
    Commitment, Ledger, LedgerCause, LedgerEvidenceError, LedgerFinishError, LedgerLimits,
    LedgerSnapshot, LiveReservation, PendingPublication, PublicationReservationBatch,
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
/// >::new(
///     OperationControl::default(),
///     LedgerLimits::new(
///         8,
///         [(Dimension::Bytes, DimensionPolicy::reservable_bytes(8, 8, 8, 8))],
///     ).unwrap(),
/// );
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
}

impl<O, LP, PP, U, D> OperationCoordinator<O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    /// Creates one coordinator with frozen initial central ledger policy.
    pub fn new(control: OperationControl<PP, U>, ledger_limits: LedgerLimits<D>) -> Self {
        Self {
            control,
            ledger: Ledger::new(ledger_limits),
            terminal: None,
            progress: Vec::new(),
            progress_evidence_bytes: 0,
            last_phase_order: None,
            delivered_events: 0,
            delivered_bytes: 0,
            next_sequence: 0,
        }
    }

    /// Polls cancellation and deadline state.
    pub fn poll(&mut self) -> Result<(), ControlCause> {
        if let Some(cause) = self.terminal.clone() {
            return Err(cause);
        }
        let cancellation = self.control.cancellation.as_ref().map(Arc::clone);
        let cancelled = catch_unwind(move || {
            cancellation.is_some_and(|cancellation| cancellation.is_cancelled())
        });
        let cancelled = match cancelled {
            Ok(cancelled) => cancelled,
            Err(payload) => {
                self.control.discard_cancellation();
                discard_untrusted_panic_payload(payload);
                return self.fail(ControlCause::CancellationSignalPanicked);
            }
        };
        if cancelled {
            return self.fail(ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            });
        }
        if self
            .control
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return self.fail(ControlCause::DeadlineExceeded);
        }
        Ok(())
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
        self.poll()?;
        if total.is_some_and(|total| completed > total) {
            return self.fail(ControlCause::InvalidProgress {
                violation: ProgressViolation::CompletedExceededTotal,
            });
        }
        if self
            .last_phase_order
            .is_some_and(|last| phase.order() < last)
        {
            return self.fail(ControlCause::InvalidProgress {
                violation: ProgressViolation::PhaseRegressed,
            });
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
                return self.fail(ControlCause::InvalidProgress {
                    violation: ProgressViolation::CompletedRegressed,
                });
            }
            if matches!((previous.total, total), (Some(old), Some(new)) if new < old)
                || matches!((previous.total, total), (Some(_), None))
            {
                return self.fail(ControlCause::InvalidProgress {
                    violation: ProgressViolation::TotalRegressed,
                });
            }
        }

        let previous_delivery = previous.and_then(|state| state.last_delivered);
        let should_deliver = match delivery {
            ProgressDelivery::Transient => {
                !unchanged
                    && previous_delivery.is_none_or(|previous| {
                        completed.saturating_sub(previous)
                            >= self.control.progress_limits.min_work_delta()
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
                if let Err(cause) = self.try_insert_progress(position, key, progress_state) {
                    return self.fail(ControlCause::ProgressStorage { cause });
                }
                position
            }
        };
        self.last_phase_order = Some(phase.order());

        if !should_deliver || self.control.observer.is_none() {
            return Ok(());
        }
        let encoded_bytes = match delivery {
            ProgressDelivery::Transient => {
                if self.delivered_events >= self.control.progress_limits.max_events() {
                    return Ok(());
                }
                let encoded_bytes = estimated_event_bytes(phase, unit);
                if self.delivered_bytes.saturating_add(encoded_bytes)
                    > self.control.progress_limits.max_encoded_bytes()
                {
                    return Ok(());
                }
                Some(encoded_bytes)
            }
            ProgressDelivery::Final => None,
        };

        let event = ProgressEvent::new(phase, unit, completed, total, self.next_sequence);
        let Some(mut observer) = self.control.observer.take() else {
            return Ok(());
        };
        let decision = catch_unwind(AssertUnwindSafe(|| observer.observe(event)));
        let decision = match decision {
            Ok(decision) => {
                self.control.observer = Some(observer);
                decision
            }
            Err(payload) => {
                discard_extension(observer);
                discard_untrusted_panic_payload(payload);
                return self.fail(ControlCause::ObserverPanicked);
            }
        };

        if let Some(encoded_bytes) = encoded_bytes {
            self.delivered_events = self.delivered_events.saturating_add(1);
            self.delivered_bytes = self.delivered_bytes.saturating_add(encoded_bytes);
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.progress[progress_position].1.last_delivered = Some(completed);
        if decision == ProgressDecision::Cancel {
            return self.fail(ControlCause::Cancelled {
                origin: CancellationOrigin::Observer,
            });
        }
        Ok(())
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

    /// Transfers a reservation to another owner-defined phase.
    pub fn transfer_reservation(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        phase: LP,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger.transfer(reservation, phase)
    }

    /// Resizes one reservation while preserving it unchanged on admission failure.
    pub fn resize_reservation(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
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
        self.report_final_progress(
            final_progress.phase,
            final_progress.unit,
            final_progress.total,
        )?;
        self.poll()?;
        self.ledger.publish(publication)?;
        Ok(())
    }

    /// Splits a reservation without changing live or peak accounting.
    pub fn split_reservation(
        &mut self,
        reservation: &mut LiveReservation<O, LP, D>,
        bytes: u64,
    ) -> Result<LiveReservation<O, LP, D>, LedgerCause<D>> {
        self.ledger.split(reservation, bytes)
    }

    /// Verifies that a live reservation belongs to this operation without mutating accounting.
    pub fn validate_live_reservation(
        &self,
        reservation: &LiveReservation<O, LP, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger.validate_live_reservation(reservation)
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn retained_ledger(&self) -> &Ledger<O, LP, D> {
        &self.ledger
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn try_prepare_typed_control(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<(), ControlCause> {
        self.report_final_progress(
            final_progress.phase,
            final_progress.unit,
            final_progress.total,
        )?;
        self.poll()
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn into_ledger_for_typed_publication(self) -> Ledger<O, LP, D> {
        let Self {
            mut control,
            ledger,
            progress,
            progress_evidence_bytes,
            ..
        } = self;
        drop(progress);
        ledger.release_progress_evidence(progress_evidence_bytes);
        control.discard_cancellation();
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
    pub fn finish(mut self) -> Result<LedgerSnapshot<LP, D>, LedgerFinishError<LP, D>> {
        self.progress = Vec::new();
        self.ledger
            .release_progress_evidence(self.progress_evidence_bytes);
        self.progress_evidence_bytes = 0;
        self.ledger.finish()
    }

    fn fail(&mut self, cause: ControlCause) -> Result<(), ControlCause> {
        self.terminal = Some(cause.clone());
        Err(cause)
    }

    fn try_insert_progress(
        &mut self,
        position: usize,
        key: (PP, U),
        state: ProgressState,
    ) -> Result<(), ProgressStorageError> {
        let desired_capacity = self.progress.len().saturating_add(1);
        let limit = self.control.progress_limits.max_states();
        if desired_capacity > limit {
            return Err(ProgressStorageError::StateLimitExceeded {
                value: desired_capacity,
                limit,
            });
        }
        self.ledger
            .try_grow_progress_evidence(
                &mut self.progress,
                desired_capacity,
                &mut self.progress_evidence_bytes,
            )
            .map_err(|cause| ProgressStorageError::Evidence { cause })?;
        self.progress.insert(position, (key, state));
        Ok(())
    }
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
    pub fn charge_consumed_batch(&mut self, charges: &[(D, u64)]) -> Result<(), LedgerCause<D>> {
        self.coordinator.ledger.charge_consumed_batch(charges)
    }

    /// Reserves live capacity before allocation or backend entry.
    pub fn reserve(
        &mut self,
        phase: LP,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, LP, D>, LedgerCause<D>> {
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
