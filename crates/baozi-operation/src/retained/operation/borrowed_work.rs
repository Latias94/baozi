//! Retained helpers keep numeric frames outside callbacks and lend only short permission views.

use super::*;
use crate::ledger::{ExclusiveStorage, InternalWork, WorkFrame, WorkLane, WorkLayout};

pub(crate) struct RetainedWorkRoot<'op, O, P, D> {
    pub(crate) work: InternalWork<'op, D>,
    pub(crate) storage: ExclusiveStorage<'op, O, P, D>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Ordinary,
    Protected,
}

impl Role {
    fn abandoned(self) -> u32 {
        match self {
            Self::Ordinary => ORDINARY_WORK_PERMIT_ABANDONED_CODE,
            Self::Protected => FAILURE_WORK_PERMIT_ABANDONED_CODE,
        }
    }

    fn overrun(self) -> u32 {
        match self {
            Self::Ordinary => ORDINARY_WORK_PERMIT_OVERRUN_CODE,
            Self::Protected => FAILURE_WORK_PERMIT_OVERRUN_CODE,
        }
    }

    fn lane(self) -> WorkLane {
        match self {
            Self::Ordinary => WorkLane::Internal,
            Self::Protected => WorkLane::Protected,
        }
    }
}

/// The original split's binding result and poll policy; no work has been admitted.
pub(crate) struct RetainedWorkBinding<'op, PP, U, D> {
    pub(super) poll: OperationPollCheckpoint<'op, PP, U>,
    pub(super) dimension: D,
    pub(super) reservation_dimension: D,
    pub(super) binding: Option<Result<WorkLayout<1>, LedgerCause<D>>>,
    pub(super) outstanding: &'op mut bool,
    pub(super) partially_resolved: &'op mut bool,
    pub(super) failure: &'op Cell<Option<PreparationFailureCode>>,
    role: Role,
}

pub(crate) struct RetainedFailureWorkBinding<'op, PP, U, D>(
    pub(super) RetainedWorkBinding<'op, PP, U, D>,
);

pub(crate) enum RetainedWorkStartError<D> {
    Control(ControlCause),
    Ledger(LedgerCause<D>),
    Outstanding,
}

pub(crate) enum RetainedWorkConsumeError<D> {
    Ledger(LedgerCause<D>),
    ExceedsAttempted,
}

pub(crate) enum RetainedWorkFinishError<D> {
    Incomplete,
    Ledger(LedgerCause<D>),
}

pub(crate) enum RetainedFailureWorkStartError<D> {
    Control(ControlCause),
    Ledger(LedgerCause<D>),
    Outstanding,
}

pub(crate) enum RetainedFailureWorkConsumeError<D> {
    Ledger(LedgerCause<D>),
    ExceedsAttempted,
}

pub(crate) enum RetainedFailureWorkFinishError<D> {
    Incomplete,
    Ledger(LedgerCause<D>),
}

/// The actual frame owner stays in the existing physical builder/validator adapter.
struct RetainedControlFrame<'op, O, P, PP, U, D: Copy> {
    work: InternalWork<'op, D>,
    storage: ExclusiveStorage<'op, O, P, D>,
    poll: OperationPollCheckpoint<'op, PP, U>,
    dimension: D,
    reservation_dimension: D,
    binding: Option<Result<WorkLayout<1>, LedgerCause<D>>>,
    layout: Option<WorkLayout<1>>,
    frame: Option<WorkFrame<1>>,
    outstanding: &'op mut bool,
    partially_resolved: &'op mut bool,
    failure: &'op Cell<Option<PreparationFailureCode>>,
    faulted: bool,
    partial: bool,
    role: Role,
}

/// A callback permission borrows the real frame slot; forgetting it cannot forget that frame.
pub(crate) struct RetainedWorkControl<'a, O, P, PP, U, D: Copy> {
    work: InternalWork<'a, D>,
    storage: ExclusiveStorage<'a, O, P, D>,
    poll: OperationPollCheckpoint<'a, PP, U>,
    dimension: D,
    reservation_dimension: D,
    binding: Option<Result<WorkLayout<1>, LedgerCause<D>>>,
    layout: &'a mut Option<WorkLayout<1>>,
    frame: &'a mut Option<WorkFrame<1>>,
    outstanding: &'a mut bool,
    partially_resolved: &'a mut bool,
    failure: &'a Cell<Option<PreparationFailureCode>>,
    faulted: &'a mut bool,
    partial: &'a mut bool,
    role: Role,
}

#[must_use = "resolve the borrowed work permission explicitly"]
pub(crate) struct RetainedWorkPermission<'a, O, P, PP, U, D: Copy> {
    control: Option<RetainedWorkControl<'a, O, P, PP, U, D>>,
}

#[must_use = "resolve the borrowed work permission explicitly"]
pub(crate) struct BorrowedRetainedWorkPermission<'borrow, 'operation, O, P, PP, U, D: Copy> {
    control: &'borrow mut RetainedWorkControl<'operation, O, P, PP, U, D>,
    resolved: bool,
}

pub(crate) struct RetainedFailureWorkControl<'a, O, P, PP, U, D: Copy>(
    RetainedWorkControl<'a, O, P, PP, U, D>,
);

#[must_use = "resolve the protected permission explicitly"]
pub(crate) struct RetainedFailureWorkPermission<'a, O, P, PP, U, D: Copy>(
    RetainedWorkPermission<'a, O, P, PP, U, D>,
);

impl<'op, PP, U, D: Copy + Ord + fmt::Debug> RetainedWorkBinding<'op, PP, U, D> {
    pub(super) fn ordinary(
        poll: OperationPollCheckpoint<'op, PP, U>,
        dimension: D,
        reservation_dimension: D,
        binding: Result<WorkLayout<1>, LedgerCause<D>>,
        outstanding: &'op mut bool,
        partially_resolved: &'op mut bool,
        failure: &'op Cell<Option<PreparationFailureCode>>,
    ) -> Self {
        Self {
            poll,
            dimension,
            reservation_dimension,
            binding: Some(binding),
            outstanding,
            partially_resolved,
            failure,
            role: Role::Ordinary,
        }
    }

    pub(super) fn protected(
        poll: OperationPollCheckpoint<'op, PP, U>,
        dimension: D,
        outstanding: &'op mut bool,
        partially_resolved: &'op mut bool,
        failure: &'op Cell<Option<PreparationFailureCode>>,
    ) -> RetainedFailureWorkBinding<'op, PP, U, D> {
        RetainedFailureWorkBinding(Self {
            poll,
            dimension,
            reservation_dimension: dimension,
            binding: None,
            outstanding,
            partially_resolved,
            failure,
            role: Role::Protected,
        })
    }

    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.poll.poll().inspect_err(|cause| {
            record_terminal_cause_with_precedence(
                self.failure,
                PreparationFailureCode::from_control(cause),
            );
        })
    }

    pub(crate) fn charge_observed<O: OperationOwner, P: Copy + Ord>(
        &mut self,
        root: &mut RetainedWorkRoot<'_, O, P, D>,
        amount: u64,
    ) -> Result<(), LedgerCause<D>> {
        let result = (|| {
            let layout = self
                .binding
                .as_ref()
                .expect("ordinary observed work has a prepared binding")
                .clone()?;
            let mut work = root.work.reborrow()?;
            work.charge_observed(layout, [amount])
        })();
        result.inspect_err(|_| {
            record_terminal_cause_with_precedence(self.failure, PreparationFailureCode::ledger())
        })
    }

    fn run<O: OperationOwner, P: Copy + Ord, R>(
        &mut self,
        root: &mut RetainedWorkRoot<'_, O, P, D>,
        action: impl for<'a> FnOnce(RetainedWorkControl<'a, O, P, PP, U, D>) -> R,
        accepts: impl FnOnce(&R) -> bool,
    ) -> (R, Result<(), RetainedWorkFinishError<D>>) {
        let mut host = RetainedControlFrame {
            work: root.work.loan(),
            storage: root.storage.reborrow(),
            poll: self.poll.reborrow(),
            dimension: self.dimension,
            reservation_dimension: self.reservation_dimension,
            binding: self.binding.clone(),
            layout: None,
            frame: None,
            outstanding: self.outstanding,
            partially_resolved: self.partially_resolved,
            failure: self.failure,
            faulted: false,
            partial: false,
            role: self.role,
        };
        let result = action(host.control());
        let completion = host.finish(accepts(&result));
        (result, completion)
    }

    pub(crate) fn build<O: OperationOwner, P: Copy + Ord, R>(
        mut self,
        root: &mut RetainedWorkRoot<'_, O, P, D>,
        action: impl for<'a> FnOnce(
            RetainedWorkControl<'a, O, P, PP, U, D>,
        ) -> Result<R, RetainedBuildFailureCode>,
    ) -> Result<R, RetainedBuildFailureCode> {
        let (result, completion) = self.run(root, action, Result::is_ok);
        match (result, completion) {
            (Ok(value), Err(_)) => {
                drop(value);
                Err(RetainedBuildFailureCode::new(self.role.abandoned()))
            }
            (result, _) => result,
        }
    }

    pub(crate) fn validate<O: OperationOwner, P: Copy + Ord>(
        mut self,
        root: &mut RetainedWorkRoot<'_, O, P, D>,
        action: impl for<'a> FnOnce(RetainedWorkControl<'a, O, P, PP, U, D>) -> bool,
    ) -> bool {
        let (accepted, completion) = self.run(root, action, |accepted| *accepted);
        accepted && completion.is_ok()
    }
}

impl<PP, U, D: Copy + Ord + fmt::Debug> RetainedFailureWorkBinding<'_, PP, U, D> {
    pub(crate) fn build<O: OperationOwner, P: Copy + Ord, R>(
        self,
        root: &mut RetainedWorkRoot<'_, O, P, D>,
        action: impl for<'a> FnOnce(
            RetainedFailureWorkControl<'a, O, P, PP, U, D>,
        ) -> Result<R, RetainedBuildFailureCode>,
    ) -> Result<R, RetainedBuildFailureCode> {
        self.0
            .build(root, |control| action(RetainedFailureWorkControl(control)))
    }

    pub(crate) fn validate<O: OperationOwner, P: Copy + Ord>(
        self,
        root: &mut RetainedWorkRoot<'_, O, P, D>,
        action: impl for<'a> FnOnce(RetainedFailureWorkControl<'a, O, P, PP, U, D>) -> bool,
    ) -> bool {
        self.0
            .validate(root, |control| action(RetainedFailureWorkControl(control)))
    }
}

impl<O: OperationOwner, P: Copy + Ord, PP, U, D: Copy + Ord + fmt::Debug>
    RetainedControlFrame<'_, O, P, PP, U, D>
{
    fn control(&mut self) -> RetainedWorkControl<'_, O, P, PP, U, D> {
        RetainedWorkControl {
            work: self.work.loan(),
            storage: self.storage.reborrow(),
            poll: self.poll.reborrow(),
            dimension: self.dimension,
            reservation_dimension: self.reservation_dimension,
            binding: self.binding.clone(),
            layout: &mut self.layout,
            frame: &mut self.frame,
            outstanding: self.outstanding,
            partially_resolved: self.partially_resolved,
            failure: self.failure,
            faulted: &mut self.faulted,
            partial: &mut self.partial,
            role: self.role,
        }
    }

    fn finish(&mut self, accepted: bool) -> Result<(), RetainedWorkFinishError<D>> {
        if self.frame.is_some() {
            let code = self.role.abandoned();
            let mut control = self.control();
            control.record_product_graph_failure(code);
            control
                .resolve(false)
                .map_err(RetainedWorkFinishError::Ledger)?;
            return Err(RetainedWorkFinishError::Incomplete);
        }
        if accepted && (self.faulted || self.partial) {
            return Err(RetainedWorkFinishError::Incomplete);
        }
        Ok(())
    }
}

impl<O, P, PP, U, D: Copy> Drop for RetainedControlFrame<'_, O, P, PP, U, D> {
    fn drop(&mut self) {
        if let (Some(frame), Some(layout)) = (self.frame.as_mut(), self.layout) {
            record_terminal_cause_with_precedence(
                self.failure,
                PreparationFailureCode::product_graph(self.role.abandoned()),
            );
            match self.work.resolve(layout, frame, false) {
                Ok(()) => *self.outstanding = false,
                Err(_) => record_terminal_cause_with_precedence(
                    self.failure,
                    PreparationFailureCode::ledger(),
                ),
            }
        }
    }
}

impl<'a, O: OperationOwner, P: Copy + Ord, PP, U, D: Copy + Ord + fmt::Debug>
    RetainedWorkControl<'a, O, P, PP, U, D>
{
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.poll.poll().inspect_err(|cause| {
            *self.faulted = true;
            record_terminal_cause_with_precedence(
                self.failure,
                PreparationFailureCode::from_control(cause),
            );
        })
    }

    pub(crate) fn record_product_graph_failure(&mut self, code: u32) {
        *self.faulted = true;
        record_terminal_cause_with_precedence(
            self.failure,
            PreparationFailureCode::product_graph(code),
        );
    }

    fn record_ledger<T>(&mut self, result: Result<T, LedgerCause<D>>) -> Result<T, LedgerCause<D>> {
        result.inspect_err(|_| {
            *self.faulted = true;
            record_terminal_cause_with_precedence(self.failure, PreparationFailureCode::ledger());
        })
    }

    fn prepare_layout(&self) -> Result<WorkLayout<1>, LedgerCause<D>> {
        match &self.binding {
            Some(binding) => binding.clone(),
            None => self.work.bind_protected([self.dimension]),
        }
    }

    pub(crate) fn charge_observed(&mut self, amount: u64) -> Result<(), LedgerCause<D>> {
        let result = (|| {
            let layout = self.prepare_layout()?;
            self.work.reborrow()?.charge_observed(layout, [amount])
        })();
        self.record_ledger(result)
    }

    fn admit(&mut self, amount: NonZeroU64) -> Result<(), RetainedWorkStartError<D>> {
        if *self.outstanding || self.frame.is_some() {
            self.record_product_graph_failure(self.role.abandoned());
            return Err(RetainedWorkStartError::Outstanding);
        }
        self.poll().map_err(RetainedWorkStartError::Control)?;
        let result = (|| {
            let layout = self.prepare_layout()?;
            let mut work = self.work.reborrow()?;
            work.validate_layout(layout)?;
            let frame = layout.admit_observed(&mut work.work, [amount.get()], self.role.lane())?;
            *self.layout = Some(layout);
            *self.frame = Some(frame);
            Ok(())
        })();
        self.record_ledger(result)
            .map_err(RetainedWorkStartError::Ledger)?;
        *self.outstanding = true;
        Ok(())
    }

    pub(crate) fn try_begin(
        mut self,
        amount: NonZeroU64,
    ) -> Result<RetainedWorkPermission<'a, O, P, PP, U, D>, RetainedWorkStartError<D>> {
        self.admit(amount)?;
        Ok(RetainedWorkPermission {
            control: Some(self),
        })
    }

    pub(crate) fn try_begin_borrowed<'borrow>(
        &'borrow mut self,
        amount: NonZeroU64,
    ) -> Result<
        BorrowedRetainedWorkPermission<'borrow, 'a, O, P, PP, U, D>,
        RetainedWorkStartError<D>,
    > {
        self.admit(amount)?;
        Ok(BorrowedRetainedWorkPermission {
            control: self,
            resolved: false,
        })
    }

    fn remaining(&self) -> u64 {
        self.frame.as_ref().map_or(0, |frame| frame.remaining[0])
    }

    fn complete_units(&mut self, amount: NonZeroU64) -> Result<(), RetainedWorkConsumeError<D>> {
        if amount.get() > self.remaining() {
            self.record_product_graph_failure(self.role.overrun());
            return Err(RetainedWorkConsumeError::ExceedsAttempted);
        }
        let result = (|| {
            let frame = self
                .frame
                .as_mut()
                .ok_or(LedgerCause::InactiveLinearWorkPermit)?;
            if !frame.active || frame.faulted {
                return Err(LedgerCause::InactiveLinearWorkPermit);
            }
            let layout = self.layout.expect("an admitted frame retains its layout");
            let mut work = self.work.reborrow()?;
            if let Err(cause) = work.validate_layout(layout) {
                frame.faulted = true;
                return Err(cause);
            }
            frame
                .complete_batch_with_outcome(&layout, &mut work.work, [amount.get()])
                .into_result()
        })();
        self.record_ledger(result)
            .map_err(RetainedWorkConsumeError::Ledger)
    }

    fn resolve(&mut self, partial: bool) -> Result<(), LedgerCause<D>> {
        let result = match (self.frame.as_mut(), *self.layout) {
            (Some(frame), Some(layout)) => self.work.resolve(layout, frame, false),
            _ => Err(LedgerCause::InactiveLinearWorkPermit),
        };
        self.record_ledger(result)?;
        *self.frame = None;
        *self.layout = None;
        *self.outstanding = false;
        if partial {
            *self.partial = true;
            if self.role == Role::Ordinary {
                *self.partially_resolved = true;
            }
        }
        Ok(())
    }

    fn finish_exact(&mut self) -> Result<(), RetainedWorkFinishError<D>> {
        if self.remaining() != 0 {
            self.record_product_graph_failure(self.role.abandoned());
            return Err(RetainedWorkFinishError::Incomplete);
        }
        self.resolve(false).map_err(RetainedWorkFinishError::Ledger)
    }

    pub(crate) fn reserve(
        &mut self,
        phase: P,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        let result = self
            .storage
            .reserve(phase, self.reservation_dimension, bytes);
        self.record_ledger(result)
    }

    pub(crate) fn resize(
        &mut self,
        reservation: &mut LiveReservation<O, P, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        let result = self.storage.resize(reservation, bytes);
        self.record_ledger(result)
    }

    pub(crate) fn validate_live_owner(
        &mut self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        let result = self.storage.validate_live_owner(reservation);
        self.record_ledger(result)
    }
}

impl<'a, O: OperationOwner, P: Copy + Ord, PP, U, D: Copy + Ord + fmt::Debug>
    RetainedWorkPermission<'a, O, P, PP, U, D>
{
    fn control(&mut self) -> &mut RetainedWorkControl<'a, O, P, PP, U, D> {
        self.control
            .as_mut()
            .expect("unresolved permission retains its control loan")
    }
    pub(crate) fn record_product_graph_failure(&mut self, code: u32) {
        self.control().record_product_graph_failure(code);
    }
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.control().poll()
    }
    pub(crate) fn remaining_units(&self) -> u64 {
        self.control
            .as_ref()
            .map_or(0, RetainedWorkControl::remaining)
    }
    pub(crate) fn complete_units(
        &mut self,
        amount: NonZeroU64,
    ) -> Result<(), RetainedWorkConsumeError<D>> {
        self.control().complete_units(amount)
    }
    pub(crate) fn reserve(
        &mut self,
        phase: P,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        self.control().reserve(phase, bytes)
    }
    pub(crate) fn resize(
        &mut self,
        reservation: &mut LiveReservation<O, P, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.control().resize(reservation, bytes)
    }
    pub(crate) fn validate_live_owner(
        &mut self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.control().validate_live_owner(reservation)
    }
    pub(crate) fn complete(
        mut self,
    ) -> Result<RetainedWorkControl<'a, O, P, PP, U, D>, RetainedWorkFinishError<D>> {
        self.control().finish_exact()?;
        Ok(self.control.take().expect("permission resolves once"))
    }
    pub(crate) fn finish_partial(mut self) {
        if let Some(mut control) = self.control.take() {
            let _ = control.resolve(true);
        }
    }
}

impl<O, P, PP, U, D: Copy> Drop for RetainedWorkPermission<'_, O, P, PP, U, D> {
    fn drop(&mut self) {
        if let Some(control) = self.control.as_mut() {
            record_terminal_cause_with_precedence(
                control.failure,
                PreparationFailureCode::product_graph(control.role.abandoned()),
            );
            *control.faulted = true;
            if let (Some(frame), Some(layout)) = (control.frame.as_mut(), *control.layout) {
                match control.work.resolve(layout, frame, false) {
                    Ok(()) => {
                        *control.outstanding = false;
                        *control.frame = None;
                        *control.layout = None;
                    }
                    Err(_) => record_terminal_cause_with_precedence(
                        control.failure,
                        PreparationFailureCode::ledger(),
                    ),
                }
            }
        }
    }
}

impl<O: OperationOwner, P: Copy + Ord, PP, U, D: Copy + Ord + fmt::Debug>
    BorrowedRetainedWorkPermission<'_, '_, O, P, PP, U, D>
{
    pub(crate) fn record_product_graph_failure(&mut self, code: u32) {
        self.control.record_product_graph_failure(code);
    }
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.control.poll()
    }
    pub(crate) fn remaining_units(&self) -> u64 {
        self.control.remaining()
    }
    pub(crate) fn complete_units(
        &mut self,
        amount: NonZeroU64,
    ) -> Result<(), RetainedWorkConsumeError<D>> {
        self.control.complete_units(amount)
    }
    pub(crate) fn complete(mut self) -> Result<(), RetainedWorkFinishError<D>> {
        self.control.finish_exact()?;
        self.resolved = true;
        Ok(())
    }
    pub(crate) fn finish_partial(mut self) {
        let _ = self.control.resolve(true);
        self.resolved = true;
    }
}

impl<O, P, PP, U, D: Copy> Drop for BorrowedRetainedWorkPermission<'_, '_, O, P, PP, U, D> {
    fn drop(&mut self) {
        if self.resolved {
            return;
        }
        record_terminal_cause_with_precedence(
            self.control.failure,
            PreparationFailureCode::product_graph(self.control.role.abandoned()),
        );
        *self.control.faulted = true;
        if let (Some(frame), Some(layout)) = (self.control.frame.as_mut(), *self.control.layout) {
            match self.control.work.resolve(layout, frame, false) {
                Ok(()) => {
                    *self.control.outstanding = false;
                    *self.control.frame = None;
                    *self.control.layout = None;
                }
                Err(_) => record_terminal_cause_with_precedence(
                    self.control.failure,
                    PreparationFailureCode::ledger(),
                ),
            }
        }
    }
}

impl<'a, O: OperationOwner, P: Copy + Ord, PP, U, D: Copy + Ord + fmt::Debug>
    RetainedFailureWorkControl<'a, O, P, PP, U, D>
{
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.0.poll()
    }
    pub(crate) fn record_product_graph_failure(&mut self, code: u32) {
        self.0.record_product_graph_failure(code);
    }
    pub(crate) fn try_begin(
        self,
        amount: NonZeroU64,
    ) -> Result<RetainedFailureWorkPermission<'a, O, P, PP, U, D>, RetainedFailureWorkStartError<D>>
    {
        self.0
            .try_begin(amount)
            .map(RetainedFailureWorkPermission)
            .map_err(|cause| match cause {
                RetainedWorkStartError::Control(cause) => {
                    RetainedFailureWorkStartError::Control(cause)
                }
                RetainedWorkStartError::Ledger(cause) => {
                    RetainedFailureWorkStartError::Ledger(cause)
                }
                RetainedWorkStartError::Outstanding => RetainedFailureWorkStartError::Outstanding,
            })
    }
}

impl<'a, O: OperationOwner, P: Copy + Ord, PP, U, D: Copy + Ord + fmt::Debug>
    RetainedFailureWorkPermission<'a, O, P, PP, U, D>
{
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.0.poll()
    }
    pub(crate) fn record_product_graph_failure(&mut self, code: u32) {
        self.0.record_product_graph_failure(code);
    }
    pub(crate) fn remaining_units(&self) -> u64 {
        self.0.remaining_units()
    }
    pub(crate) fn complete_units(
        &mut self,
        amount: NonZeroU64,
    ) -> Result<(), RetainedFailureWorkConsumeError<D>> {
        self.0.complete_units(amount).map_err(|cause| match cause {
            RetainedWorkConsumeError::Ledger(cause) => {
                RetainedFailureWorkConsumeError::Ledger(cause)
            }
            RetainedWorkConsumeError::ExceedsAttempted => {
                RetainedFailureWorkConsumeError::ExceedsAttempted
            }
        })
    }
    pub(crate) fn complete(
        self,
    ) -> Result<RetainedFailureWorkControl<'a, O, P, PP, U, D>, RetainedFailureWorkFinishError<D>>
    {
        self.0
            .complete()
            .map(RetainedFailureWorkControl)
            .map_err(|cause| match cause {
                RetainedWorkFinishError::Incomplete => RetainedFailureWorkFinishError::Incomplete,
                RetainedWorkFinishError::Ledger(cause) => {
                    RetainedFailureWorkFinishError::Ledger(cause)
                }
            })
    }
    pub(crate) fn finish_validation(
        mut self,
        accepted: bool,
    ) -> Result<RetainedFailureWorkControl<'a, O, P, PP, U, D>, RetainedFailureWorkFinishError<D>>
    {
        if accepted {
            return self.complete();
        }
        let mut control = self
            .0
            .control
            .take()
            .expect("validation permission resolves once");
        control
            .resolve(true)
            .map_err(RetainedFailureWorkFinishError::Ledger)?;
        Ok(RetainedFailureWorkControl(control))
    }
    pub(crate) fn finish_partial(self) {
        self.0.finish_partial();
    }
}
