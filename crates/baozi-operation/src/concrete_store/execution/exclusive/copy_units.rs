//! Reusable scalar actions whose callback and returned fact have no owned destructor.

use super::*;

struct CopyUnitHost<'a, 'op> {
    work: &'a mut WorkView<'op, StoreDimensionKey>,
    frame: &'a mut WorkFrame<1>,
    layout: WorkLayout<1>,
    failure: &'a Cell<Option<PreparationFailureCode>>,
    execution_state: &'a Cell<StoreExecutionState>,
    boundary: Option<&'a StoreImporterBoundary>,
    armed: bool,
}

impl Drop for CopyUnitHost<'_, '_> {
    fn drop(&mut self) {
        if self.armed {
            mark_unwind(self.execution_state, self.failure, self.boundary);
            if self.frame.active && self.frame.resolve(&self.layout, self.work, false).is_err() {
                record(self.failure, PreparationFailureCode::ledger());
            }
        }
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> StoreInvocation<'op, O, PP, U> {
    pub(in crate::concrete_store) fn close_copy_units(
        &mut self,
        layout: StoreInvocationLayout<'op, 1>,
        pending: &mut Option<WorkFrame<1>>,
    ) -> Result<(), StoreRuntimeError> {
        match pending.take() {
            Some(mut frame) if frame.active => runtime_ledger(
                self.parts.ordinary.failure,
                frame.resolve(&layout.raw, &mut self.parts.work.work, false),
            ),
            _ => Ok(()),
        }
    }

    pub(in crate::concrete_store) fn run_copy_unit<R: Copy>(
        &mut self,
        layout: StoreInvocationLayout<'op, 1>,
        pending: &mut Option<WorkFrame<1>>,
        action: impl FnOnce() -> R + Copy,
    ) -> Result<R, StoreRuntimeError> {
        // A real poll can transfer control. Release the completed envelope before that frontier.
        if matches!(
            checked_action_debt(self.parts.ordinary.debt.get(), 1),
            Ok(None)
        ) {
            self.close_copy_units(layout, pending)?;
        }
        if let Err(cause) = self.before_metered_action(1) {
            let _ = self.close_copy_units(layout, pending);
            return Err(cause);
        }
        let admission = match pending {
            Some(frame) => runtime_ledger(
                self.parts.ordinary.failure,
                frame.replenish_one(&layout.raw, &mut self.parts.work.work),
            ),
            None => runtime_ledger(
                self.parts.ordinary.failure,
                layout
                    .raw
                    .admit_observed(&mut self.parts.work.work, [1], WorkLane::Producer),
            )
            .map(|frame| *pending = Some(frame)),
        };
        if let Err(cause) = admission {
            let _ = self.close_copy_units(layout, pending);
            return Err(cause);
        }
        let mut host = CopyUnitHost {
            work: &mut self.parts.work.work,
            frame: pending
                .as_mut()
                .expect("the scalar unit has an admitted frame"),
            layout: layout.raw,
            failure: self.parts.ordinary.failure,
            execution_state: self.parts.ordinary.execution_state,
            boundary: self.boundary,
            armed: true,
        };
        // No inner catch or callback disposal: the lexical host owns panic resolution, and both
        // captures and returned facts are Copy. The planning host retains the outer catch.
        let value = action();
        let completion = runtime_ledger(
            host.failure,
            host.frame
                .complete_batch_with_outcome(&host.layout, host.work, [1])
                .into_result(),
        );
        host.armed = false;
        drop(host);
        if let Err(cause) = completion {
            let _ = self.close_copy_units(layout, pending);
            return Err(cause);
        }
        if let Err(cause) = self.completed_metered_action(1) {
            let _ = self.close_copy_units(layout, pending);
            return Err(cause);
        }
        Ok(value)
    }
}
