//! Disjoint invocation capabilities for repeated library execution.

use super::*;
use crate::ledger::{BoundWorkGroup, ExclusiveStorage, Ledger, WorkLane};

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
    pub(super) ledger: &'op Ledger<O, P, D>,
    pub(super) account: &'op PhysicalStorageAccount,
    pub(super) share_work: RetainedHandleSharePlan<D>,
    pub(super) failure: &'op Cell<Option<PreparationFailureCode>>,
}

pub(crate) struct ExecutionServices<'op, O, P, PP, U, D> {
    pub(crate) ordinary: ExecutionOrdinaryStorage<'op, O, P, PP, U, D>,
    pub(crate) retained: ExecutionStorage<'op, O, P, D>,
    progress: crate::coordinator::OperationProgressState<'op, O, P, PP, U, D>,
}

pub(crate) struct ExecutionOrdinaryStorage<'b, O, P, PP, U, D> {
    pub(crate) storage: ExclusiveStorage<'b, O, P, D>,
    pub(crate) poll: OperationPollCheckpoint<'b, PP, U>,
    pub(crate) failure: &'b Cell<Option<PreparationFailureCode>>,
    pub(crate) debt: &'b Cell<u16>,
    pub(crate) execution_state: &'b Cell<StoreExecutionState>,
}

pub(crate) struct ExecutionGroup<'op, D: Copy, const N: usize> {
    bound: BoundWorkGroup<'op, D, N>,
    failure: &'op Cell<Option<PreparationFailureCode>>,
}

pub(crate) struct ExecutionPermit<'op, D: Copy, const N: usize> {
    work: Option<LinearWorkBatchPermit<'op, D, N>>,
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

pub(crate) fn check_execution_state<D>(
    failure: &Cell<Option<PreparationFailureCode>>,
    state: &Cell<StoreExecutionState>,
    recorded: impl FnOnce() -> Option<ControlCause>,
) -> Result<(), ExecutionError<D>> {
    StoreExecutionState::settle_caught_unwind(state, failure);
    if failure.get().is_some() || state.get() != StoreExecutionState::Running {
        return Err(recorded().map_or(ExecutionError::Stopped, ExecutionError::Control));
    }
    Ok(())
}

pub(crate) fn poll_execution<PP, U, D>(
    poll: &mut OperationPollCheckpoint<'_, PP, U>,
    failure: &Cell<Option<PreparationFailureCode>>,
    debt: &Cell<u16>,
) -> Result<(), ExecutionError<D>> {
    if failure.get().is_some() {
        return Err(poll
            .recorded_control_cause()
            .map_or(ExecutionError::Stopped, ExecutionError::Control));
    }
    poll.poll().map_err(|cause| {
        record_terminal_cause_with_precedence(
            failure,
            PreparationFailureCode::from_control(&cause),
        );
        ExecutionError::Control(cause)
    })?;
    debt.set(0);
    Ok(())
}

impl<O: OperationOwner, P: Copy + Ord, PP, U, D: Copy + Ord + fmt::Debug>
    ExecutionOrdinaryStorage<'_, O, P, PP, U, D>
{
    pub(crate) fn checkpoint(&mut self) -> Result<(), ExecutionError<D>> {
        self.running()?;
        poll_execution(&mut self.poll, self.failure, self.debt)
    }
    pub(crate) fn running(&self) -> Result<(), ExecutionError<D>> {
        check_execution_state(self.failure, self.execution_state, || {
            self.poll.recorded_control_cause()
        })
    }
    pub(crate) fn before_action(&mut self, max_cost: u16) -> Result<(), ExecutionError<D>> {
        if self.failure.get().is_some() {
            return Err(self
                .poll
                .recorded_control_cause()
                .map_or(ExecutionError::Stopped, ExecutionError::Control));
        }
        ensure_running(self.failure)?;
        match checked_action_debt(self.debt.get(), max_cost) {
            Err(()) => return self.reject_action_cost(),
            Ok(None) => self.checkpoint()?,
            Ok(Some(_)) => {}
        }
        Ok(())
    }
    pub(crate) fn completed_action(&mut self, max_cost: u16) -> Result<(), ExecutionError<D>> {
        match checked_action_debt(self.debt.get(), max_cost) {
            Ok(Some(next)) => {
                self.debt.set(next);
                Ok(())
            }
            Err(()) | Ok(None) => self.reject_action_cost(),
        }
    }
    fn reject_action_cost(&self) -> Result<(), ExecutionError<D>> {
        record_terminal_cause_with_precedence(
            self.failure,
            PreparationFailureCode::product_graph(0x4143_4f53),
        );
        Err(ExecutionError::Stopped)
    }
    pub(crate) fn reserve(
        &mut self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, ExecutionError<D>> {
        self.checkpoint()?;
        preserve_ledger(self.failure, self.storage.reserve(phase, dimension, bytes))
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

impl<D: Copy> ExecutionPermit<'_, D, 1> {
    pub(crate) fn replenish_one(&mut self) -> Result<(), ExecutionError<D>> {
        ensure_running(self.failure)?;
        let result = self
            .work
            .as_mut()
            .expect("an unresolved execution envelope owns its work")
            .replenish_one();
        preserve_ledger(self.failure, result)
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

impl<O, P, PP, U, D> ExecutionServices<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    /// Entry, allocation, IO, external callback and region completion use this full checkpoint.
    pub(crate) fn checkpoint(&mut self) -> Result<(), ExecutionError<D>> {
        self.ordinary.checkpoint()
    }

    pub(crate) fn before_action(&mut self, max_cost: u16) -> Result<(), ExecutionError<D>> {
        self.ordinary.before_action(max_cost)
    }

    /// Advances conservative control cost after actual work completion, never ledger consumption.
    pub(crate) fn completed_action(&mut self, max_cost: u16) -> Result<(), ExecutionError<D>> {
        self.ordinary.completed_action(max_cost)
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
            .report_progress(&mut self.ordinary.poll, phase, unit, completed, total)
            .map_err(|cause| {
                record_terminal_cause_with_precedence(
                    self.ordinary.failure,
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
    pub(crate) fn storage_view(&self) -> ExclusiveStorage<'_, O, P, D> {
        self.ledger.storage_view()
    }
    pub(crate) fn failure(&self) -> &Cell<Option<PreparationFailureCode>> {
        self.failure
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
        let storage = self.ledger.storage_view();
        let result = super::super::reserve_exclusive_backing(
            &storage,
            self.account,
            phase,
            dimension,
            bytes,
            allocations,
            || super::super::try_admit_retained_share_work(self.ledger, self.share_work),
        );
        if let Err(cause) = &result {
            let code = match cause {
                RetainedWriterError::Ledger(_) => PreparationFailureCode::ledger(),
                _ => PreparationFailureCode::product_graph(0x5354_4f52),
            };
            record_terminal_cause_with_precedence(self.failure, code);
        }
        result
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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedOperation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
        ExecutionServices<'_, O, P, PP, U, D>,
    ) {
        let Self {
            retained, common, ..
        } = self;
        let super::RetainedOperationFields {
            baseline,
            terminal_account_share_work,
            ..
        } = &mut **retained;
        let OperationInvocation {
            coordinator,
            sticky_terminal_cause,
            control_action_debt,
            execution_state,
            ..
        } = &mut **common;
        let (progress, ledger) = coordinator.split_progress_checkpoint_and_retained_ledger();
        let (poll, progress) = progress.into_parts();
        (
            ExecutionWork {
                ledger,
                failure: sticky_terminal_cause,
            },
            ExecutionServices {
                ordinary: ExecutionOrdinaryStorage {
                    storage: ledger.storage_view(),
                    poll,
                    failure: sticky_terminal_cause,
                    debt: control_action_debt,
                    execution_state,
                },
                retained: ExecutionStorage {
                    ledger,
                    account: baseline.account(),
                    share_work: *terminal_account_share_work,
                    failure: sticky_terminal_cause,
                },
                progress,
            },
        )
    }
}
