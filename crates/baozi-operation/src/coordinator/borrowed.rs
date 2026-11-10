//! Short loans of the actual coordinator control, ledger, and progress fields.

use super::{
    CoordinatorControlState, FinalProgress, OperationPollCheckpoint, OperationProgressFields,
    ProgressRows, PublicationCause,
};
use crate::OperationOwner;
use crate::control::{ControlCause, ProgressPhase, ProgressUnit};
use crate::ledger::{LedgerReadView, LedgerView};
use std::fmt;

pub(crate) struct CoordinatorView<'op, O, LP, PP, U, D> {
    control_state: &'op mut CoordinatorControlState<PP, U, D>,
    ledger: LedgerView<'op, O, LP, D>,
    progress: ProgressRows<'op, PP, U>,
}

type CoordinatorParts<'op, O, LP, PP, U, D> = (
    OperationPollCheckpoint<'op, PP, U>,
    OperationProgressFields<'op, PP, U>,
    LedgerView<'op, O, LP, D>,
);

impl<'op, O, LP, PP, U, D> CoordinatorView<'op, O, LP, PP, U, D>
where
    O: OperationOwner,
    LP: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) fn new(
        control_state: &'op mut CoordinatorControlState<PP, U, D>,
        ledger: LedgerView<'op, O, LP, D>,
        progress: ProgressRows<'op, PP, U>,
    ) -> Self {
        Self {
            control_state,
            ledger,
            progress,
        }
    }

    #[cfg(test)]
    pub(crate) fn reborrow(&mut self) -> CoordinatorView<'_, O, LP, PP, U, D> {
        CoordinatorView::new(
            self.control_state,
            self.ledger.reborrow(),
            self.progress.reborrow(),
        )
    }

    pub(crate) fn ledger_read(&self) -> LedgerReadView<'_, O, LP, D> {
        self.ledger.as_read()
    }

    pub(crate) fn ledger_mut(&mut self) -> &mut LedgerView<'op, O, LP, D> {
        &mut self.ledger
    }

    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.control_state.poll()
    }

    pub(crate) fn recorded_control_cause(&self) -> Option<ControlCause> {
        self.control_state.recorded_control_cause()
    }

    pub(crate) fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ControlCause> {
        self.control_state.report_progress(
            &mut self.ledger,
            self.progress.reborrow(),
            phase,
            unit,
            completed,
            total,
        )
    }

    pub(crate) fn try_prepare_typed_control(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<(), PublicationCause<D>> {
        self.control_state.try_prepare_typed_control(
            &mut self.ledger,
            self.progress.reborrow(),
            final_progress,
        )
    }

    pub(crate) fn close_for_terminal_extraction(&mut self) -> Result<(), PublicationCause<D>> {
        self.control_state
            .close_for_terminal_extraction(&mut self.ledger)
    }

    pub(crate) fn into_exclusive_progress_and_ledger(
        self,
    ) -> CoordinatorParts<'op, O, LP, PP, U, D> {
        let (poll, progress) = self.control_state.split_progress(self.progress);
        (poll, progress, self.ledger)
    }

    pub(crate) fn into_poll_and_ledger(
        self,
    ) -> (
        OperationPollCheckpoint<'op, PP, U>,
        LedgerView<'op, O, LP, D>,
    ) {
        (self.control_state.poll_checkpoint(false), self.ledger)
    }

    /// A previously accepted control failure cannot cancel its selected bounded cleanup.
    /// When control was live at this split, later cancellation and deadline failures remain visible.
    pub(crate) fn into_terminal_poll_and_ledger(
        self,
    ) -> (
        OperationPollCheckpoint<'op, PP, U>,
        LedgerView<'op, O, LP, D>,
    ) {
        (self.control_state.terminal_poll_checkpoint(), self.ledger)
    }
}
