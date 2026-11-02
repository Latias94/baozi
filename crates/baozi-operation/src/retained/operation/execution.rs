//! Disjoint invocation capabilities for repeated library execution.

use super::*;
use crate::ledger::{BoundWorkGroup, Ledger, ParserWorkError, ParserWorkScope, WorkLane};

const MAX_ACTIONS_BETWEEN_POLLS: u16 = 256;

pub(crate) enum ExecutionError<D> {
    Control(ControlCause),
    Ledger(LedgerCause<D>),
    Stopped,
}

/// Numeric authority and failure selection borrow the invocation, never this view's stack slot.
pub(crate) struct ExecutionWork<'op, O, P, D> {
    ledger: &'op Ledger<O, P, D>,
    failure: &'op Cell<Option<PreparationFailureCode>>,
}

pub(crate) struct ExecutionStorage<'op, O, P, D> {
    ledger: &'op Ledger<O, P, D>,
    account: &'op PhysicalStorageAccount,
    share_work: RetainedHandleSharePlan<D>,
    failure: &'op Cell<Option<PreparationFailureCode>>,
}

pub(crate) struct ExecutionControl<'op, O, P, PP, U, D> {
    progress: OperationProgressCheckpoint<'op, O, P, PP, U, D>,
    failure: &'op Cell<Option<PreparationFailureCode>>,
    actions_since_poll: &'op Cell<u16>,
}

pub(crate) struct ExecutionGroup<'op, D: Copy, const N: usize> {
    bound: BoundWorkGroup<'op, D, N>,
    failure: &'op Cell<Option<PreparationFailureCode>>,
}

pub(crate) struct ExecutionPermit<'op, D: Copy, const N: usize> {
    work: Option<LinearWorkBatchPermit<'op, D, N>>,
    failure: &'op Cell<Option<PreparationFailureCode>>,
}

pub(crate) struct ExecutionParserScope<'op, D: Copy, const N: usize> {
    work: ParserWorkScope<'op, D, N>,
    failure: &'op Cell<Option<PreparationFailureCode>>,
}

fn preserve_ledger<T, D>(
    failure: &Cell<Option<PreparationFailureCode>>,
    result: Result<T, LedgerCause<D>>,
) -> Result<T, ExecutionError<D>> {
    result.map_err(|cause| {
        record_terminal_cause_with_precedence(failure, PreparationFailureCode::ledger());
        ExecutionError::Ledger(cause)
    })
}

fn ensure_running<D>(
    failure: &Cell<Option<PreparationFailureCode>>,
) -> Result<(), ExecutionError<D>> {
    if failure.get().is_some() {
        Err(ExecutionError::Stopped)
    } else {
        Ok(())
    }
}

impl<'op, O, P, D> ExecutionWork<'op, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn bind<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<ExecutionGroup<'op, D, N>, ExecutionError<D>> {
        ensure_running(self.failure)?;
        let ledger: &'op Ledger<O, P, D> = self.ledger;
        let bound = preserve_ledger(self.failure, ledger.bind_work(dimensions))?;
        Ok(ExecutionGroup {
            bound,
            failure: self.failure,
        })
    }

    pub(crate) fn finish_region(&self) -> Result<(), ExecutionError<D>> {
        preserve_ledger(self.failure, self.ledger.check_work_quiescent())
    }

    pub(crate) fn dimension_evidence(
        &self,
        dimension: D,
    ) -> Result<DimensionSnapshot, ExecutionError<D>> {
        preserve_ledger(self.failure, self.ledger.bind_work([dimension]))?;
        preserve_ledger(self.failure, self.ledger.dimension_evidence(dimension))
    }

    pub(crate) fn storage_observation(
        &self,
        dimension: D,
    ) -> Result<crate::ledger::LedgerStorageObservation<'op, P, D>, ExecutionError<D>> {
        preserve_ledger(self.failure, self.ledger.bind_work([dimension]))?;
        preserve_ledger(self.failure, self.ledger.storage_observation(dimension))
    }

    pub(crate) fn tighten_policy(
        &self,
        dimension: D,
        policy: DimensionPolicy,
    ) -> Result<(), ExecutionError<D>> {
        ensure_running(self.failure)?;
        preserve_ledger(self.failure, self.ledger.bind_work([dimension]))?;
        preserve_ledger(
            self.failure,
            self.ledger.tighten_dimension_policy(dimension, policy),
        )
    }

    pub(crate) fn reject_binding(&self) {
        record_terminal_cause_with_precedence(self.failure, PreparationFailureCode::ledger());
    }
}

impl<'op, D: Copy, const N: usize> ExecutionGroup<'op, D, N> {
    pub(crate) fn begin_parser(
        &self,
        amounts: [u64; N],
        item_limits: [Option<u64>; N],
        parent: bool,
    ) -> Result<ExecutionParserScope<'op, D, N>, ParserWorkError<D>> {
        if self.failure.get().is_some() {
            return Err(ParserWorkError::Ledger(
                LedgerCause::PublicationAlreadyPending,
            ));
        }
        let result = if parent {
            self.bound.begin_parser_parent(amounts, item_limits)
        } else {
            self.bound.begin_parser_child(amounts, item_limits)
        };
        result
            .map(|work| ExecutionParserScope {
                work,
                failure: self.failure,
            })
            .inspect_err(|_| {
                record_terminal_cause_with_precedence(
                    self.failure,
                    PreparationFailureCode::ledger(),
                );
            })
    }

    pub(crate) fn admit_observed(
        &self,
        amounts: [u64; N],
    ) -> Result<ExecutionPermit<'op, D, N>, ExecutionError<D>> {
        ensure_running(self.failure)?;
        let work = preserve_ledger(
            self.failure,
            self.bound.admit_observed(amounts, WorkLane::Producer),
        )?;
        Ok(ExecutionPermit {
            work: Some(work),
            failure: self.failure,
        })
    }

    pub(crate) fn charge_attempted(&self, amounts: [u64; N]) -> Result<(), ExecutionError<D>> {
        ensure_running(self.failure)?;
        preserve_ledger(self.failure, self.bound.charge_attempted_in_order(amounts))
    }

    pub(crate) fn complete_direct(&self, amounts: [u64; N]) -> Result<(), ExecutionError<D>> {
        preserve_ledger(self.failure, self.bound.consume_direct_atomic(amounts))
    }

    pub(crate) fn complete_observed(&self, amounts: [u64; N]) -> Result<(), ExecutionError<D>> {
        preserve_ledger(self.failure, self.bound.consume_observed_in_order(amounts))
    }

    pub(crate) fn charge_observed(&self, amounts: [u64; N]) -> Result<(), ExecutionError<D>> {
        preserve_ledger(self.failure, self.bound.charge_observed_ordered(amounts))
    }
}

impl<D: Copy, const N: usize> ExecutionParserScope<'_, D, N> {
    pub(crate) fn remaining(&self) -> [u64; N] {
        self.work.remaining()
    }

    pub(crate) fn complete(&mut self, amounts: [u64; N]) -> Result<(), ExecutionError<D>> {
        preserve_ledger(self.failure, self.work.complete(amounts))
    }

    pub(crate) fn complete_all(&mut self) -> Result<(), ExecutionError<D>> {
        preserve_ledger(self.failure, self.work.complete_all())
    }

    pub(crate) fn finish_exact(self) -> Result<(), ExecutionError<D>> {
        preserve_ledger(self.failure, self.work.finish_exact())
    }

    pub(crate) fn finish_partial(self) -> Result<(), ExecutionError<D>> {
        preserve_ledger(self.failure, self.work.finish_partial())
    }
}

impl<D: Copy> ExecutionParserScope<'_, D, 1> {
    pub(crate) fn replenish(&mut self, amount: u64) -> Result<(), ExecutionError<D>> {
        ensure_running(self.failure)?;
        preserve_ledger(self.failure, self.work.replenish(amount))
    }
}

impl<D: Copy, const N: usize> ExecutionPermit<'_, D, N> {
    pub(crate) fn remaining(&self) -> [u64; N] {
        self.work
            .as_ref()
            .expect("an unresolved execution envelope owns its work")
            .remaining()
    }

    pub(crate) fn complete(&mut self, amounts: [u64; N]) -> Result<(), ExecutionError<D>> {
        let result = self
            .work
            .as_mut()
            .expect("an unresolved execution envelope owns its work")
            .complete_batch_with_outcome(amounts)
            .into_result();
        preserve_ledger(self.failure, result)
    }

    pub(crate) fn finish(mut self) -> Result<(), ExecutionError<D>> {
        if self.remaining().iter().any(|amount| *amount != 0) {
            record_terminal_cause_with_precedence(
                self.failure,
                PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE),
            );
            self.finish_partial()?;
            return Err(ExecutionError::Stopped);
        }
        let work = self.work.take().expect("execution work resolves once");
        preserve_ledger(self.failure, work.finish())
    }

    pub(crate) fn finish_partial(mut self) -> Result<(), ExecutionError<D>> {
        let work = self.work.take().expect("execution work resolves once");
        preserve_ledger(self.failure, work.finish_partial())
    }
}

impl<D: Copy, const N: usize> Drop for ExecutionPermit<'_, D, N> {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            record_terminal_cause_with_precedence(
                self.failure,
                PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE),
            );
            let _ = preserve_ledger(self.failure, work.finish_partial());
        }
    }
}

impl<O, P, PP, U, D> ExecutionControl<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    /// Entry, allocation, IO, external callback and region completion use this full checkpoint.
    pub(crate) fn checkpoint(&mut self) -> Result<(), ExecutionError<D>> {
        if self.failure.get().is_some() {
            return Err(self
                .progress
                .recorded_control_cause()
                .map_or(ExecutionError::Stopped, ExecutionError::Control));
        }
        ensure_running(self.failure)?;
        self.progress.poll().map_err(|cause| {
            record_terminal_cause_with_precedence(
                self.failure,
                PreparationFailureCode::from_control(&cause),
            );
            ExecutionError::Control(cause)
        })?;
        self.actions_since_poll.set(0);
        Ok(())
    }

    pub(crate) fn before_action(&mut self) -> Result<(), ExecutionError<D>> {
        if self.failure.get().is_some() {
            return Err(self
                .progress
                .recorded_control_cause()
                .map_or(ExecutionError::Stopped, ExecutionError::Control));
        }
        ensure_running(self.failure)?;
        if self.actions_since_poll.get() >= MAX_ACTIONS_BETWEEN_POLLS {
            self.checkpoint()?;
        }
        Ok(())
    }

    /// Work completion precedes this cadence-only update; no uncommitted work lives here.
    pub(crate) fn completed_action(&mut self) {
        self.actions_since_poll
            .set(self.actions_since_poll.get().saturating_add(1));
    }

    pub(crate) fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ExecutionError<D>> {
        self.checkpoint()?;
        self.progress
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| {
                record_terminal_cause_with_precedence(
                    self.failure,
                    PreparationFailureCode::from_control(&cause),
                );
                ExecutionError::Control(cause)
            })
    }
}

impl<O, P, D> ExecutionStorage<'_, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    pub(crate) fn shares_invocation<PP, U>(
        &self,
        control: &ExecutionControl<'_, O, P, PP, U, D>,
    ) -> bool {
        std::ptr::eq(self.failure, control.failure)
    }

    pub(crate) fn reject_binding(&self) {
        record_terminal_cause_with_precedence(self.failure, PreparationFailureCode::ledger());
    }

    #[allow(
        clippy::type_complexity,
        reason = "retained backing keeps its ledger reservation and physical lease together"
    )]
    pub(crate) fn reserve_retained(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
        allocations: u64,
    ) -> Result<
        (LiveReservation<O, P, D>, super::super::RetainedStorageLease),
        RetainedWriterError<D>,
    > {
        self.ledger
            .ensure_work_open()
            .map_err(RetainedWriterError::Ledger)?;
        let result = (|| {
            let preflight = self
                .account
                .try_begin_account_preflight()
                .map_err(RetainedWriterError::AccountPoisoned)?;
            let reservation = self
                .ledger
                .reserve(phase, dimension, bytes)
                .map_err(RetainedWriterError::Ledger)?;
            let action = preflight
                .admit_action()
                .map_err(super::super::map_account_action_admission_error)?;
            let share = super::super::try_admit_retained_share_work_before_physical(
                self.ledger,
                self.account,
                self.share_work,
            )?;
            let mut admission = self
                .account
                .try_admit_retained_after_action(
                    action,
                    super::super::AdmissionCharge::new(bytes, allocations),
                    share,
                )
                .map_err(|cause| match cause {
                    super::super::RetainedShareAdmissionError::Physical(cause) => {
                        RetainedWriterError::Physical(cause)
                    }
                    super::super::RetainedShareAdmissionError::Ledger(cause) => {
                        RetainedWriterError::Ledger(cause)
                    }
                })?;
            Ok((
                reservation,
                super::super::RetainedStorageLease::from_admission(&mut admission),
            ))
        })();
        if let Err(cause) = &result {
            let code = match cause {
                RetainedWriterError::Ledger(_) => PreparationFailureCode::ledger(),
                _ => PreparationFailureCode::product_graph(0x5354_4f52),
            };
            record_terminal_cause_with_precedence(self.failure, code);
        }
        result
    }

    pub(crate) fn reserve(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, ExecutionError<D>> {
        ensure_running(self.failure)?;
        preserve_ledger(self.failure, self.ledger.reserve(phase, dimension, bytes))
    }

    pub(crate) fn resize(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        bytes: u64,
    ) -> Result<(), ExecutionError<D>> {
        preserve_ledger(
            self.failure,
            self.ledger.resize_reservation(reservation, bytes),
        )
    }

    pub(crate) fn validate_live_owner(
        &self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), ExecutionError<D>> {
        preserve_ledger(
            self.failure,
            self.ledger.validate_live_reservation(reservation),
        )
    }

    pub(crate) fn transition(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<(), ExecutionError<D>> {
        preserve_ledger(
            self.failure,
            self.ledger
                .transition_reservation(reservation, phase, dimension, bytes),
        )
    }
}

impl<O, P, PP, U, D, F> RetainedOperation<O, P, PP, U, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    #[allow(
        clippy::type_complexity,
        reason = "concrete disjoint invocation capabilities share one lifetime"
    )]
    pub(crate) fn execution(
        &mut self,
    ) -> (
        ExecutionWork<'_, O, P, D>,
        ExecutionStorage<'_, O, P, D>,
        ExecutionControl<'_, O, P, PP, U, D>,
    ) {
        let Self {
            coordinator,
            baseline,
            terminal_account_share_work,
            sticky_terminal_cause,
            control_action_debt,
            ..
        } = self;
        let (progress, ledger) = coordinator.split_progress_checkpoint_and_retained_ledger();
        (
            ExecutionWork {
                ledger,
                failure: sticky_terminal_cause,
            },
            ExecutionStorage {
                ledger,
                account: baseline.account(),
                share_work: *terminal_account_share_work,
                failure: sticky_terminal_cause,
            },
            ExecutionControl {
                progress,
                failure: sticky_terminal_cause,
                actions_since_poll: control_action_debt,
            },
        )
    }
}
