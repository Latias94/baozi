use super::{
    CoordinatorControlState, CoordinatorView, FiniteProgress4, OperationControl, ProgressRows,
};
use crate::control::{ControlCause, ProgressPhase, ProgressUnit};
use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};
use crate::{LedgerCause, OperationOwner};
use std::fmt;

pub(crate) struct FiniteOperationCoordinator<O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    control_state: CoordinatorControlState<PP, U, D>,
    ledger: FiniteLedger<O, LP, D>,
    progress: FiniteProgress4<PP, U>,
}

impl<O, LP, PP, U, D> FiniteOperationCoordinator<O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    #[cfg(test)]
    pub(crate) fn try_new(
        control: OperationControl<PP, U>,
        ledger_limits: FiniteLedgerLimits<D>,
        phases: [LP; 3],
    ) -> Result<Self, LedgerCause<D>> {
        Self::try_new_with_control(&mut Some(control), ledger_limits, phases)
    }

    pub(crate) fn try_new_with_control(
        control: &mut Option<OperationControl<PP, U>>,
        ledger_limits: FiniteLedgerLimits<D>,
        phases: [LP; 3],
    ) -> Result<Self, LedgerCause<D>> {
        let ledger = FiniteLedger::try_new(ledger_limits, phases)?;
        Ok(Self {
            control_state: CoordinatorControlState::new(
                control
                    .take()
                    .expect("the original finite entry retains control until installation"),
            ),
            ledger,
            progress: FiniteProgress4::new(),
        })
    }

    pub(crate) fn view(&mut self) -> CoordinatorView<'_, O, LP, PP, U, D> {
        CoordinatorView::new(
            &mut self.control_state,
            self.ledger.view(),
            ProgressRows::Finite(&mut self.progress),
        )
    }

    pub(crate) fn recorded_control_cause(&self) -> Option<ControlCause> {
        self.control_state.recorded_control_cause()
    }

    #[cfg(test)]
    pub(crate) fn reported_progress(&self, phase: PP, unit: U) -> Option<(u64, Option<u64>)> {
        self.progress.rows[..self.progress.len]
            .binary_search_by_key(&Some((phase, unit)), |row| {
                row.as_ref().map(|(key, _)| *key)
            })
            .ok()
            .and_then(|position| self.progress.rows[position])
            .map(|(_, state)| (state.completed, state.total))
    }

    pub(crate) fn retained_ledger(&self) -> &FiniteLedger<O, LP, D> {
        &self.ledger
    }

    pub(crate) fn retained_ledger_mut(&mut self) -> &mut FiniteLedger<O, LP, D> {
        &mut self.ledger
    }

    pub(crate) fn known_administrative_bytes(&self) -> Option<u64> {
        self.ledger.known_administrative_bytes(0)
    }
}

#[cfg(test)]
mod tests;
