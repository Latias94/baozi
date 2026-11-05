//! Disjoint invocation capabilities for repeated library execution.

use super::{
    ControlCause, LedgerCause, LiveReservation, OperationOwner, OperationPollCheckpoint,
    PreparationFailureCode, StoreExecutionState, checked_action_debt,
    record_terminal_cause_with_precedence,
};
use crate::ledger::ExclusiveStorage;
use std::{cell::Cell, fmt};

pub(crate) enum ExecutionError<D> {
    Control(ControlCause),
    Ledger(LedgerCause<D>),
    Stopped,
}

pub(crate) struct ExecutionOrdinaryStorage<'b, O, P, PP, U, D> {
    pub(crate) storage: ExclusiveStorage<'b, O, P, D>,
    pub(crate) poll: OperationPollCheckpoint<'b, PP, U>,
    pub(crate) failure: &'b Cell<Option<PreparationFailureCode>>,
    pub(crate) debt: &'b Cell<u16>,
    pub(crate) execution_state: &'b Cell<StoreExecutionState>,
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
