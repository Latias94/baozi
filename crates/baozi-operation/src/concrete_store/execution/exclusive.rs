//! Exclusive numeric execution and host-owned action/parser frames over one invocation.

use super::*;
use crate::ledger::{
    ParserChannel, ParserFrame, ParserReadRow, WorkFrame, WorkLane, WorkLayout, WorkView,
};
use crate::retained::{
    AdmittedShareWork, ExclusiveOperationParts, PreparationFailureCode, StoreExecutionState,
    checked_action_debt, record_terminal_cause_with_precedence,
};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

mod actions;
pub use actions::StoreActionExecution;
mod copy_units;
mod services;
pub use services::{StoreChildAdmission, StoreInvocationServices};

const ABANDONED: u32 = StoreExecutionState::ABANDONED;
const INVALID_COST: u32 = 0x4143_4f53;

/// Disposition of the real registry invocation, independent of a movable Dormant root.
/// This contains no numeric admission or mutable work capability.
pub struct StoreImporterBoundary {
    disposition: Cell<ImporterDisposition>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImporterDisposition {
    Idle,
    Active,
    Returned,
    Panicked,
    Poisoned,
}

impl Default for StoreImporterBoundary {
    fn default() -> Self {
        Self {
            disposition: Cell::new(ImporterDisposition::Idle),
        }
    }
}

impl StoreImporterBoundary {
    /// Called by the registry host only after acquiring the actual registration permit.
    pub fn begin(&self) -> bool {
        if self.disposition.get() == ImporterDisposition::Active {
            return false;
        }
        self.disposition.set(ImporterDisposition::Active);
        true
    }
    /// A successful catch can still be poisoned by another actual invocation.
    pub fn finish_returned(&self, poisoned: bool) {
        self.disposition.set(if poisoned {
            ImporterDisposition::Poisoned
        } else {
            ImporterDisposition::Returned
        });
    }
    pub fn finish_panicked(&self) {
        self.disposition.set(ImporterDisposition::Panicked);
    }
    pub fn finish_poisoned(&self) {
        self.disposition.set(ImporterDisposition::Poisoned);
    }
    fn active(&self) -> bool {
        self.disposition.get() == ImporterDisposition::Active
    }
}

/// A physical extension call returned, or unwound while the host still owned its admission.
pub enum StoreExtensionResult<T> {
    Returned(T),
    Panicked,
}

/// The trusted text scanner either exhausts its admitted chunk or finds a semantic boundary.
///
/// Boundary return releases only the unused suffix of the actual parser frame. It does not
/// mark a parser error or count an exhausted channel. Ordinary parser callbacks remain exact.
/// A parent can admit the next sequential scan after a short boundary:
///
/// ```
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::{
///     StoreInvocationError, StoreInvocationLayout, StoreInvocationParser, StoreScanReturn,
/// };
/// fn scan<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     parent: &mut StoreInvocationParser<'_, '_, 'op, O, P, U, 1, false>,
///     layout: StoreInvocationLayout<'op, 1>,
/// ) -> Result<bool, StoreInvocationError> {
///     let input = b"\nx";
///     parent.scan_child(layout, [2], [None], |scan| {
///         let newline = scan.run_prefix([1], 1, || {
///             Ok::<_, StoreInvocationError>(input[0] == b'\n')
///         })?;
///         Ok(StoreScanReturn::Boundary(newline))
///     })
/// }
/// ```
///
/// A current child cannot admit a nested scan:
///
/// ```compile_fail,E0599
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::{
///     StoreInvocationError, StoreInvocationLayout, StoreInvocationParser, StoreScanReturn,
/// };
/// fn scan<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     child: &mut StoreInvocationParser<'_, '_, 'op, O, P, U, 1, true>,
///     layout: StoreInvocationLayout<'op, 1>,
/// ) -> Result<bool, StoreInvocationError> {
///     let input = b"\nx";
///     child.scan_child(layout, [2], [None], |scan| {
///         let newline = scan.run_prefix([1], 1, || {
///             Ok::<_, StoreInvocationError>(input[0] == b'\n')
///         })?;
///         Ok(StoreScanReturn::Boundary(newline))
///     })
/// }
/// ```
pub enum StoreScanReturn<T> {
    Exhausted(T),
    Boundary(T),
}

fn mark_unwind(
    state: &Cell<StoreExecutionState>,
    failure: &Cell<Option<PreparationFailureCode>>,
    boundary: Option<&StoreImporterBoundary>,
) {
    StoreExecutionState::stop(state, StoreExecutionState::UnwindPending);
    if !boundary.is_some_and(StoreImporterBoundary::active) {
        record(failure, PreparationFailureCode::product_graph(ABANDONED));
    }
}

/// Trusted error construction can reserve/reconcile storage, but cannot admit parser/body work.
pub struct StoreFailureStorage<'borrow, 'op, O, PP, U> {
    context: &'borrow mut StoreInvocation<'op, O, PP, U>,
}
impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> StoreFailureStorage<'_, '_, O, PP, U> {
    pub fn reserve(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreInvocationError> {
        self.context
            .reserve_failure_storage(phase, dimension, bytes, allocations)
    }
    pub fn resize(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        bytes: u64,
    ) -> Result<(), StoreInvocationError> {
        self.context.resize_failure_storage(reservation, bytes)
    }
}

/// The replacement runner retains the existing parser item's literal limit diagnostic.
pub type StoreInvocationError = StoreParserWorkError;

impl From<StoreRuntimeError> for StoreParserWorkError {
    fn from(cause: StoreRuntimeError) -> Self {
        Self::Runtime(cause)
    }
}

/// Immutable layout branded by the exclusive invocation. It is neither a permit nor an owner.
#[derive(Clone, Copy)]
pub struct StoreInvocationLayout<'op, const N: usize> {
    raw: WorkLayout<N>,
    brand: PhantomData<fn(&'op ()) -> &'op ()>,
    local: PhantomData<Rc<()>>,
}

/// Host-owned context; clients receive only a mutable borrow from `with_exclusive_execution`.
///
/// A leaf cannot reenter the context while its action is admitted (paired with the valid example):
///
/// ```
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::{
///     StoreInvocation, StoreInvocationError, StoreInvocationLayout,
/// };
/// fn valid<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     context: &mut StoreInvocation<'op, O, P, U>, layout: StoreInvocationLayout<'op, 1>,
/// ) -> Result<(), StoreInvocationError> {
///     let mut output = 0;
///     context.run_action(layout, [1], 3, || output = 7)?;
///     context.checkpoint()?;
///     assert_eq!(output, 7);
///     Ok(())
/// }
/// ```
///
/// ```compile_fail,E0499
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::{
///     StoreInvocation, StoreInvocationError, StoreInvocationLayout,
/// };
/// fn reenter<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     context: &mut StoreInvocation<'op, O, P, U>, layout: StoreInvocationLayout<'op, 1>,
/// ) -> Result<(), StoreInvocationError> {
///     let action_context = &mut *context;
///     context.run_action(layout, [1], 3, || action_context.checkpoint())?;
///     Ok(())
/// }
/// ```
///
/// Layouts cannot escape the generative region, while an owned scalar result can:
///
/// ```
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
/// fn owned<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: &mut RetainedStoreOperation<'_, O, P, U>, key: StoreDimensionKey,
/// ) -> Result<u64, StoreInvocationError> {
///     operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
///         let layout = context.bind_group([key])?;
///         context.run_action(layout, [1], 2, || 9)
///     })
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
/// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: &mut RetainedStoreOperation<'_, O, P, U>, key: StoreDimensionKey,
/// ) {
///     let _escaped = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| context.bind_group([key]));
/// }
/// ```
///
/// Two invocations cannot exchange invariant layouts, even when both operations are live:
///
/// ```compile_fail
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
/// fn mix<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     first: &mut RetainedStoreOperation<'_, O, P, U>, second: &mut RetainedStoreOperation<'_, O, P, U>,
///     key: StoreDimensionKey,
/// ) {
///     first.with_exclusive_execution::<_, StoreInvocationError>(|outer| {
///         let layout = outer.bind_group([key])?;
///         second.with_exclusive_execution::<_, StoreInvocationError>(|inner| inner.run_action(layout, [1], 1, || ()))
///     }).unwrap();
/// }
/// ```
///
/// The host's mutable context cannot be returned or swapped with another host's context.
/// The owned-result and separate-invocation positive examples use these same valid APIs:
///
/// ```compile_fail
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
/// fn context_escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: &mut RetainedStoreOperation<'_, O, P, U>,
/// ) {
///     let _escaped = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| Ok(context));
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
/// fn context_swap<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     first: &mut RetainedStoreOperation<'_, O, P, U>, second: &mut RetainedStoreOperation<'_, O, P, U>,
/// ) {
///     first.with_exclusive_execution::<_, StoreInvocationError>(|outer| {
///         second.with_exclusive_execution::<_, StoreInvocationError>(|inner| {
///             std::mem::swap(outer, inner);
///             Ok(())
///         })
///     }).unwrap();
/// }
/// ```
///
/// ```
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
/// fn separate<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     first: &mut RetainedStoreOperation<'_, O, P, U>, second: &mut RetainedStoreOperation<'_, O, P, U>,
///     first_key: StoreDimensionKey, second_key: StoreDimensionKey,
/// ) -> Result<(u64, u64), StoreInvocationError> {
///     let left = first.with_exclusive_execution::<_, StoreInvocationError>(|context| {
///         let layout = context.bind_group([first_key])?;
///         context.run_action(layout, [1], 2, || 7)
///     })?;
///     let right = second.with_exclusive_execution::<_, StoreInvocationError>(|context| {
///         let layout = context.bind_group([second_key])?;
///         context.run_action(layout, [1], 3, || 9)
///     })?;
///     Ok((left, right))
/// }
/// ```
///
/// Scoped threads exercise the local authority requirement without relying on `'static`:
///
/// ```
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
/// fn send_owned<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: &mut RetainedStoreOperation<'_, O, P, U>, key: StoreDimensionKey,
/// ) -> Result<u64, StoreInvocationError> {
///     let owned_fact = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
///         let layout = context.bind_group([key])?;
///         context.run_action(layout, [1], 2, || 9_u64)
///     })?;
///     Ok(std::thread::scope(|threads| threads.spawn(move || owned_fact).join().unwrap()))
/// }
/// ```
///
/// ```compile_fail,E0277
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::StoreInvocation;
/// fn send<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     context: &mut StoreInvocation<'_, O, P, U>,
/// ) {
///     std::thread::scope(|threads| { threads.spawn(move || context.checkpoint()); });
/// }
/// ```
pub struct StoreInvocation<'op, O, PP, U> {
    parts: ExclusiveOperationParts<'op, O, StorePhaseKey, PP, U, StoreDimensionKey>,
    domain: &'op StoreKeyDomain,
    share_layout: WorkLayout<2>,
    boundary: Option<&'op StoreImporterBoundary>,
    brand: PhantomData<fn(&'op ()) -> &'op ()>,
    local: PhantomData<Rc<()>>,
}

fn record(slot: &Cell<Option<PreparationFailureCode>>, cause: PreparationFailureCode) {
    record_terminal_cause_with_precedence(slot, cause);
}

fn ledger<T>(
    slot: &Cell<Option<PreparationFailureCode>>,
    result: Result<T, LedgerCause<StoreDimensionKey>>,
) -> Result<T, StoreInvocationError> {
    runtime_ledger(slot, result).map_err(Into::into)
}

fn runtime_ledger<T>(
    slot: &Cell<Option<PreparationFailureCode>>,
    result: Result<T, LedgerCause<StoreDimensionKey>>,
) -> Result<T, StoreRuntimeError> {
    result.map_err(|cause| {
        record(slot, PreparationFailureCode::ledger());
        StoreRuntimeError::Ledger(cause)
    })
}

/// Only the host owns this guard. A forgotten client borrow cannot forget the frame.
pub(in crate::concrete_store) struct ActionHost<'a, 'op, const N: usize> {
    work: &'a mut WorkView<'op, StoreDimensionKey>,
    layout: WorkLayout<N>,
    frame: WorkFrame<N>,
    admitted: [u64; N],
    failure: &'a Cell<Option<PreparationFailureCode>>,
    execution_state: &'a Cell<StoreExecutionState>,
    boundary: Option<&'a StoreImporterBoundary>,
}

impl<const N: usize> ActionHost<'_, '_, N> {
    pub(in crate::concrete_store) fn complete(
        &mut self,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<StoreDimensionKey>> {
        let result = self
            .frame
            .complete_batch_with_outcome(&self.layout, self.work, amounts)
            .into_result();
        if result.is_err() {
            // The kernel already distinguished applied physical prefixes from unapplied failure.
            record(self.failure, PreparationFailureCode::ledger());
            let _ = self.frame.resolve(&self.layout, self.work, false);
        }
        result
    }

    pub(in crate::concrete_store) fn finish(
        mut self,
    ) -> Result<(), LedgerCause<StoreDimensionKey>> {
        let result = self.frame.resolve(&self.layout, self.work, false);
        if result.is_err() {
            record(self.failure, PreparationFailureCode::ledger());
        }
        result
    }

    pub(in crate::concrete_store) fn finish_partial(
        self,
    ) -> Result<(), LedgerCause<StoreDimensionKey>> {
        if self.frame.active {
            self.finish()
        } else {
            Ok(())
        }
    }
}

impl<const N: usize> Drop for ActionHost<'_, '_, N> {
    fn drop(&mut self) {
        if self.frame.active {
            mark_unwind(self.execution_state, self.failure, self.boundary);
            if self.frame.resolve(&self.layout, self.work, false).is_err() {
                record(self.failure, PreparationFailureCode::ledger());
            }
        }
    }
}

impl AdmittedShareWork<StoreDimensionKey> for ActionHost<'_, '_, 2> {
    fn complete_reference_clone(&mut self) -> Result<(), LedgerCause<StoreDimensionKey>> {
        self.complete([self.admitted[0], 0])
    }
    fn complete_typed_handle(mut self) -> Result<(), LedgerCause<StoreDimensionKey>> {
        self.complete([0, self.admitted[1]])?;
        self.finish()
    }
    fn finish_partial(self) -> Result<(), LedgerCause<StoreDimensionKey>> {
        ActionHost::finish_partial(self)
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> StoreInvocation<'op, O, PP, U> {
    pub(in crate::concrete_store) fn before_metered_action(
        &mut self,
        cost: u16,
    ) -> Result<(), StoreRuntimeError> {
        self.parts
            .ordinary
            .before_action(cost)
            .map_err(map_execution_error)
    }

    pub(in crate::concrete_store) fn completed_metered_action(
        &mut self,
        cost: u16,
    ) -> Result<(), StoreRuntimeError> {
        self.parts
            .ordinary
            .completed_action(cost)
            .map_err(map_execution_error)
    }

    pub(in crate::concrete_store) fn check_work_quiescent(&self) -> Result<(), StoreRuntimeError> {
        runtime_ledger(
            self.parts.ordinary.failure,
            self.parts.work.check_quiescent(),
        )
    }

    pub(in crate::concrete_store) fn validate_live_owner(
        &self,
        reservation: &RetainedStoreReservation<O>,
    ) -> Result<(), StoreRuntimeError> {
        runtime_ledger(
            self.parts.ordinary.failure,
            self.parts
                .ordinary
                .storage
                .validate_live_owner(&reservation.raw),
        )
    }

    fn running(&self) -> Result<(), StoreInvocationError> {
        crate::retained::check_execution_state(
            self.parts.ordinary.failure,
            self.parts.ordinary.execution_state,
            || self.parts.ordinary.poll.recorded_control_cause(),
        )
        .map_err(|cause| map_execution_error(cause).into())
    }

    fn reject(&self, cause: StoreRuntimeError) -> StoreInvocationError {
        record(
            self.parts.ordinary.failure,
            PreparationFailureCode::ledger(),
        );
        cause.into()
    }

    /// Binds membership, duplicate representatives and catalog completion order exactly once.
    pub fn bind_group<const N: usize>(
        &mut self,
        dimensions: [StoreDimensionKey; N],
    ) -> Result<StoreInvocationLayout<'op, N>, StoreInvocationError> {
        self.bind_work(dimensions).map_err(Into::into)
    }

    pub(in crate::concrete_store) fn bind_work<const N: usize>(
        &mut self,
        dimensions: [StoreDimensionKey; N],
    ) -> Result<StoreInvocationLayout<'op, N>, StoreRuntimeError> {
        self.parts.ordinary.running().map_err(map_execution_error)?;
        for dimension in dimensions {
            if !self.domain.owns_dimension(dimension) {
                record(
                    self.parts.ordinary.failure,
                    PreparationFailureCode::ledger(),
                );
                return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
            }
        }
        let raw = runtime_ledger(
            self.parts.ordinary.failure,
            self.parts.work.bind(dimensions),
        )?;
        Ok(StoreInvocationLayout {
            raw,
            brand: PhantomData,
            local: PhantomData,
        })
    }

    /// Real cancellation/deadline checkpoint; resets only this invocation's qualified debt.
    pub fn checkpoint(&mut self) -> Result<(), StoreInvocationError> {
        self.running()?;
        self.prescribed_exit_checkpoint()
    }

    /// Reports through the invocation's real progress owner after its ordinary checkpoint.
    pub fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), StoreInvocationError> {
        self.checkpoint()?;
        self.parts
            .progress
            .report_progress(&mut self.parts.ordinary.poll, phase, unit, completed, total)
            .map_err(|cause| {
                record(
                    self.parts.ordinary.failure,
                    PreparationFailureCode::from_control(&cause),
                );
                StoreRuntimeError::Control(cause).into()
            })
    }

    /// Trusted host-only exit poll. It never reopens body execution or clears a cause.
    pub fn prescribed_exit_checkpoint(&mut self) -> Result<(), StoreInvocationError> {
        crate::retained::poll_execution(
            &mut self.parts.ordinary.poll,
            self.parts.ordinary.failure,
            self.parts.ordinary.debt,
        )
        .map_err(|cause| map_execution_error(cause).into())
    }

    /// Borrows ordinary storage and poll fields, excluding work and progress.
    pub fn ordinary_storage(&mut self) -> StoreOrdinaryStorage<'_, 'op, O, PP, U> {
        StoreOrdinaryStorage {
            domain: self.domain,
            raw: &mut self.parts.ordinary,
        }
    }

    fn before(&mut self, cost: u16) -> Result<(), StoreInvocationError> {
        self.running()?;
        let next = checked_action_debt(self.parts.ordinary.debt.get(), cost).map_err(|()| {
            record(
                self.parts.ordinary.failure,
                PreparationFailureCode::product_graph(INVALID_COST),
            );
            StoreInvocationError::from(StoreRuntimeError::FallbackOnly)
        })?;
        if next.is_none() {
            self.checkpoint()?;
        }
        Ok(())
    }

    fn completed(&mut self, captured_cost: u16) -> Result<(), StoreInvocationError> {
        match checked_action_debt(self.parts.ordinary.debt.get(), captured_cost) {
            Ok(Some(next)) => {
                self.parts.ordinary.debt.set(next);
                Ok(())
            }
            _ => {
                record(
                    self.parts.ordinary.failure,
                    PreparationFailureCode::product_graph(INVALID_COST),
                );
                Err(StoreRuntimeError::FallbackOnly.into())
            }
        }
    }

    /// Completes a normally returned bounded invocation, including a returned semantic `Err`.
    /// The leaf receives no context. Physical-prefix completion precedes any result inspection.
    pub fn run_action<T, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        control_cost: u16,
        action: impl FnOnce() -> T,
    ) -> Result<T, StoreInvocationError> {
        self.run_action_outcome(layout, amounts, Some(control_cost), action, |_| true)
    }

    /// A detector or validating host consumes its chunk only when the physical callback succeeds.
    /// Panics still unwind through the actual action guard to the caller's existing catch boundary.
    pub fn run_fallible_action<T, P, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        control_cost: u16,
        action: impl FnOnce() -> Result<T, P>,
    ) -> Result<Result<T, P>, StoreInvocationError> {
        self.run_action_outcome(layout, amounts, Some(control_cost), action, Result::is_ok)
    }

    /// A root host has already performed its prescribed poll before observed admission.
    /// This path does not add parser/action cadence debt to that existing checkpoint policy.
    pub fn run_observed_action<T, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        action: impl FnOnce() -> T,
    ) -> Result<T, StoreInvocationError> {
        self.run_action_outcome(layout, amounts, None, action, |_| true)
    }

    pub fn run_observed_fallible_action<T, P, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        action: impl FnOnce() -> Result<T, P>,
    ) -> Result<Result<T, P>, StoreInvocationError> {
        self.run_action_outcome(layout, amounts, None, action, Result::is_ok)
    }

    fn run_action_outcome<T, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        control_cost: Option<u16>,
        action: impl FnOnce() -> T,
        consumes: impl FnOnce(&T) -> bool,
    ) -> Result<T, StoreInvocationError> {
        let captured_cost = control_cost;
        if let Some(cost) = captured_cost {
            self.before(cost)?;
        } else {
            self.running()?;
        }
        let mut host = self.admit_action(layout, amounts)?;
        let value = action();
        let completed = consumes(&value);
        if completed {
            let completion = host.complete(amounts);
            // Complete() resolves immediately on a ledger failure, before value/capture destruction.
            if let Err(cause) = completion {
                return Err(StoreRuntimeError::Ledger(cause).into());
            }
        }
        let finish = host.finish();
        ledger(self.parts.ordinary.failure, finish)?;
        if completed && let Some(cost) = captured_cost {
            self.completed(cost)?;
        }
        Ok(value)
    }

    pub(in crate::concrete_store) fn admit_action<const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
    ) -> Result<ActionHost<'_, 'op, N>, StoreRuntimeError> {
        let frame = runtime_ledger(
            self.parts.ordinary.failure,
            layout
                .raw
                .admit_observed(&mut self.parts.work.work, amounts, WorkLane::Producer),
        )?;
        Ok(ActionHost {
            work: &mut self.parts.work.work,
            layout: layout.raw,
            frame,
            admitted: amounts,
            failure: self.parts.ordinary.failure,
            execution_state: self.parts.ordinary.execution_state,
            boundary: self.boundary,
        })
    }

    /// Installs a real joint work/storage policy between closed actions, with the old error order.
    pub fn tighten_dimension_policy(
        &mut self,
        dimension: StoreDimensionKey,
        policy: DimensionPolicy,
    ) -> Result<(), StoreInvocationError> {
        self.running()?;
        if !self.domain.owns_dimension(dimension) {
            return Err(self.reject(StoreRuntimeError::ForeignDimensionDomain { dimension }));
        }
        ledger(
            self.parts.ordinary.failure,
            self.parts
                .work
                .tighten_policy(&self.parts.ordinary.storage, dimension, policy),
        )
    }

    /// Cold exact work observation, including pending competition. This does not allocate.
    pub fn work_counters(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<(u64, u64, u64), StoreInvocationError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(self.reject(StoreRuntimeError::ForeignDimensionDomain { dimension }));
        }
        ledger(
            self.parts.ordinary.failure,
            self.parts.work.counters(dimension),
        )
    }

    /// Actual Vec allocation, logical capacity reservation and physical Internal clone/handle work.
    /// The returned owner remains live scratch; this method never classifies it as output.
    pub fn allocate_bytes(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        count: usize,
    ) -> Result<crate::retained::RetainedStorage<Vec<u8>>, StoreInvocationError> {
        self.running()?;
        if !self.domain.owns_phase(phase) {
            return Err(self.reject(StoreRuntimeError::ForeignPhaseDomain { phase }));
        }
        if !self.domain.owns_dimension(dimension) {
            return Err(self.reject(StoreRuntimeError::ForeignDimensionDomain { dimension }));
        }
        let plan = CapacityPlan::for_type::<u8>(0, 0, count).map_err(|_| {
            self.reject(StoreRuntimeError::Build(
                StoreBuildError::capacity_envelope_violated(),
            ))
        })?;
        let bytes = plan
            .envelope_delta_bytes()
            .map_err(|_| self.reject(StoreRuntimeError::AccountingOverflow))?;
        self.checkpoint()?;
        let mut reservation =
            self.reserve_backing(phase, dimension, bytes, u64::from(bytes != 0))?;
        let failure = self.parts.ordinary.failure;
        let (values, capacity) = match try_allocate_vec::<u8>(plan) {
            Ok(values) => values,
            Err(cause) => {
                record(failure, PreparationFailureCode::domain(0x414c_4c4f));
                reservation.release_storage(());
                return Err(StoreRuntimeError::Build(cause).into());
            }
        };
        let actual_bytes = capacity as u64;
        let resize = self
            .parts
            .ordinary
            .storage
            .resize(&mut reservation.raw, actual_bytes)
            .map_err(StoreRuntimeError::Ledger)
            .and_then(|()| {
                reservation
                    .physical
                    .as_mut()
                    .expect("live allocation admission")
                    .shrink_to(actual_bytes)
                    .map_err(|cause| map_writer_error(&RetainedWriterError::Physical(cause)))
            });
        if let Err(cause) = resize {
            record(failure, PreparationFailureCode::ledger());
            reservation.release_storage(values);
            return Err(cause.into());
        }
        // Allocation is an explicit control frontier on both sides. The actual payload is
        // still uniquely coupled to these admissions, so rejection has verified cleanup.
        if let Err(cause) = self.checkpoint() {
            reservation.release_storage(values);
            return Err(cause);
        }
        reservation.bind_storage(values).map_err(|cause| {
            record(failure, PreparationFailureCode::product_graph(0x5354_4f52));
            cause.into()
        })
    }

    fn reserve_backing(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        let charges = self.parts.share_work.charges().map(|(_, amount)| amount);
        let work = &mut self.parts.work.work;
        let failure = self.parts.ordinary.failure;
        let execution_state = self.parts.ordinary.execution_state;
        let boundary = self.boundary;
        let share_layout = self.share_layout;
        let result = crate::retained::reserve_exclusive_backing(
            &self.parts.ordinary.storage,
            self.parts.account,
            phase,
            dimension,
            bytes,
            allocations,
            || {
                let frame = share_layout
                    .admit_observed(work, charges, WorkLane::Internal)
                    .map_err(RetainedWriterError::Ledger)?;
                Ok(ActionHost {
                    work,
                    layout: share_layout,
                    frame,
                    admitted: charges,
                    failure,
                    execution_state,
                    boundary,
                })
            },
        );
        let (raw, physical) = result.map_err(|cause| {
            let code = match &cause {
                RetainedWriterError::Ledger(_) => PreparationFailureCode::ledger(),
                _ => PreparationFailureCode::product_graph(0x5354_4f52),
            };
            record(failure, code);
            map_writer_error(&cause)
        })?;
        Ok(RetainedStoreReservation {
            raw,
            physical: Some(physical),
        })
    }

    /// Only the trusted error materializer uses this while its failed parser frame is guarded.
    /// This is the existing retained reserve sequence, not allocate_bytes' extra allocation polls.
    pub fn reserve_failure_storage(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreInvocationError> {
        if self.parts.ordinary.failure.get().is_some()
            || !matches!(
                self.parts.ordinary.execution_state.get(),
                StoreExecutionState::Running | StoreExecutionState::KnownError
            )
        {
            return Err(StoreRuntimeError::FallbackOnly.into());
        }
        if !self.domain.owns_phase(phase) {
            return Err(self.reject(StoreRuntimeError::ForeignPhaseDomain { phase }));
        }
        if !self.domain.owns_dimension(dimension) {
            return Err(self.reject(StoreRuntimeError::ForeignDimensionDomain { dimension }));
        }
        self.prescribed_exit_checkpoint()?;
        self.retain_backing(phase, dimension, bytes, allocations)
    }

    fn retain_backing(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreInvocationError> {
        let mut reservation = self.reserve_backing(phase, dimension, bytes, allocations)?;
        if let Err(cause) = ledger(
            self.parts.ordinary.failure,
            reservation.raw.retain_for_output(),
        ) {
            reservation.release_storage(());
            return Err(cause);
        }
        Ok(reservation)
    }

    /// Reconciles the actual retained envelope using the same logical and physical authorities.
    pub fn resize_failure_storage(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        bytes: u64,
    ) -> Result<(), StoreInvocationError> {
        resize_store_reservation(
            &self.parts.ordinary.storage,
            self.parts.ordinary.failure,
            reservation,
            bytes,
        )
        .map_err(Into::into)
    }

    pub fn failure_storage(&mut self) -> StoreFailureStorage<'_, 'op, O, PP, U> {
        StoreFailureStorage { context: self }
    }

    /// A normal closed callback proves that any retained numeric unwind was caught locally.
    pub fn settle_returned_callback(&self) {
        StoreExecutionState::settle_caught_unwind(
            self.parts.ordinary.execution_state,
            self.parts.ordinary.failure,
        );
    }

    pub fn stop_known_error(&self) {
        StoreExecutionState::stop(
            self.parts.ordinary.execution_state,
            StoreExecutionState::KnownError,
        );
    }

    /// Catch inside the host guard: returned Err/EOF complete, extension panic does not.
    pub fn run_extension<T, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        control_cost: u16,
        action: impl FnOnce() -> T,
    ) -> Result<StoreExtensionResult<T>, StoreInvocationError> {
        self.before(control_cost)?;
        let frame = ledger(
            self.parts.ordinary.failure,
            layout
                .raw
                .admit_observed(&mut self.parts.work.work, amounts, WorkLane::Producer),
        )?;
        let mut host = ActionHost {
            work: &mut self.parts.work.work,
            layout: layout.raw,
            frame,
            admitted: amounts,
            failure: self.parts.ordinary.failure,
            execution_state: self.parts.ordinary.execution_state,
            boundary: self.boundary,
        };
        match catch_unwind(AssertUnwindSafe(action)) {
            Ok(value) => {
                ledger(self.parts.ordinary.failure, host.complete(amounts))?;
                ledger(self.parts.ordinary.failure, host.finish())?;
                self.completed(control_cost)?;
                Ok(StoreExtensionResult::Returned(value))
            }
            Err(payload) => {
                // Deliberately resolve before panic-payload disposal and concrete projection.
                ledger(self.parts.ordinary.failure, host.finish())?;
                self.stop_known_error();
                crate::control::discard_untrusted_panic_payload(payload);
                Ok(StoreExtensionResult::Panicked)
            }
        }
    }

    /// A host-owned attempted-only parent; children successively reborrow the same context.
    pub fn parser_parent<T, E: From<StoreInvocationError>, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        limits: [Option<u64>; N],
        body: impl FnOnce(&mut StoreInvocationParser<'_, '_, 'op, O, PP, U, N, false>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.parser::<T, E, N, false>(layout, amounts, limits, |parser| {
            body(parser).map(StoreScanReturn::Exhausted)
        })
    }

    /// A direct attempted-only child without a semantic parent, on the same numeric kernel.
    pub fn parser_child<T, E: From<StoreInvocationError>, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        limits: [Option<u64>; N],
        body: impl FnOnce(&mut StoreInvocationParser<'_, '_, 'op, O, PP, U, N, true>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.parser::<T, E, N, true>(layout, amounts, limits, |parser| {
            body(parser).map(StoreScanReturn::Exhausted)
        })
    }

    /// Runs a trusted text scan whose boundary can precede the end of its admitted chunk.
    ///
    /// The scan uses the same host, channel, prefix kernel and error handling as an ordinary
    /// child. Only a normally returned `Boundary` permits an unused suffix, without stopping
    /// subsequent children or adding successful-channel control debt. A returned `Exhausted`
    /// remains exact, and an actual error or unwind cannot become a healthy boundary.
    pub fn parser_scan_child<T, E: From<StoreInvocationError>, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        limits: [Option<u64>; N],
        body: impl FnOnce(
            &mut StoreInvocationParser<'_, '_, 'op, O, PP, U, N, true>,
        ) -> Result<StoreScanReturn<T>, E>,
    ) -> Result<T, E> {
        self.parser::<T, E, N, true>(layout, amounts, limits, body)
    }

    fn parser<T, E: From<StoreInvocationError>, const N: usize, const CHILD: bool>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        limits: [Option<u64>; N],
        body: impl FnOnce(
            &mut StoreInvocationParser<'_, '_, 'op, O, PP, U, N, CHILD>,
        ) -> Result<StoreScanReturn<T>, E>,
    ) -> Result<T, E> {
        self.running()?;
        if self.boundary.is_some() {
            self.before(1)?;
        }
        let channel = if CHILD {
            ParserChannel::Child
        } else {
            ParserChannel::Parent
        };
        let frame = layout
            .raw
            .begin_parser(&mut self.parts.work.work, amounts, limits, channel)
            .map_err(|cause| {
                record(
                    self.parts.ordinary.failure,
                    PreparationFailureCode::ledger(),
                );
                match cause {
                    crate::ledger::ParserWorkError::Ledger(cause) => {
                        StoreInvocationError::from(StoreRuntimeError::Ledger(cause))
                    }
                    crate::ledger::ParserWorkError::ItemLimit {
                        index,
                        value,
                        limit,
                    } => StoreInvocationError::ItemLimit {
                        index,
                        value,
                        limit,
                    },
                }
            })?;
        let mut host = ParserHost {
            context: self,
            layout: layout.raw,
            frame,
        };
        let result = body(&mut StoreInvocationParser { host: &mut host });
        StoreExecutionState::settle_caught_unwind(
            host.context.parts.ordinary.execution_state,
            host.context.parts.ordinary.failure,
        );
        if result.is_err() {
            host.context.stop_known_error();
        }
        let (result, exact) = match result {
            Ok(StoreScanReturn::Exhausted(value)) => (Ok(value), true),
            Ok(StoreScanReturn::Boundary(value)) => (Ok(value), false),
            Err(cause) => (Err(cause), false),
        };
        let close = host.close(exact);
        match result {
            Err(cause) => {
                let _ = close;
                Err(cause)
            }
            Ok(value) => {
                close?;
                host.context.running()?;
                if exact && host.context.boundary.is_some() {
                    host.context.completed(1)?;
                }
                Ok(value)
            }
        }
    }
}

struct ParserHost<'ctx, 'op, O, PP, U, const N: usize> {
    context: &'ctx mut StoreInvocation<'op, O, PP, U>,
    layout: WorkLayout<N>,
    frame: ParserFrame<N>,
}

impl<O, PP, U, const N: usize> ParserHost<'_, '_, O, PP, U, N> {
    fn close(&mut self, exact: bool) -> Result<(), StoreInvocationError> {
        if !self.frame.active {
            return Ok(());
        }
        let incomplete = exact && self.frame.remaining.iter().any(|amount| *amount != 0);
        if incomplete {
            StoreExecutionState::stop(
                self.context.parts.ordinary.execution_state,
                StoreExecutionState::IncompleteSuccess,
            );
            if self.context.boundary.is_none() {
                record(
                    self.context.parts.ordinary.failure,
                    PreparationFailureCode::product_graph(ABANDONED),
                );
            }
        }
        ledger(
            self.context.parts.ordinary.failure,
            self.frame.resolve(&mut self.context.parts.work.work),
        )?;
        if incomplete {
            return Err(StoreRuntimeError::ObservedWorkIncomplete.into());
        }
        Ok(())
    }
}

impl<O, PP, U, const N: usize> Drop for ParserHost<'_, '_, O, PP, U, N> {
    fn drop(&mut self) {
        if self.frame.active {
            mark_unwind(
                self.context.parts.ordinary.execution_state,
                self.context.parts.ordinary.failure,
                self.context.boundary,
            );
            let _ = self.close(false);
        }
    }
}

/// The host resolves every prefix immediately on unwind. Direct calls latch abandonment;
/// an active registry boundary retains UnwindPending until normal reuse/return or its real catch.
struct ParserActionHost<'a, 'op, const N: usize> {
    work: &'a mut WorkView<'op, StoreDimensionKey>,
    frame: &'a mut ParserFrame<N>,
    layout: WorkLayout<N>,
    failure: &'a Cell<Option<PreparationFailureCode>>,
    execution_state: &'a Cell<StoreExecutionState>,
    boundary: Option<&'a StoreImporterBoundary>,
    armed: bool,
}

impl<const N: usize> ParserActionHost<'_, '_, N> {
    fn complete(&mut self, amounts: [u64; N]) -> Result<(), LedgerCause<StoreDimensionKey>> {
        let result = self.frame.complete(&self.layout, self.work, amounts);
        if result.is_err() {
            record(self.failure, PreparationFailureCode::ledger());
            let _ = self.frame.resolve(self.work);
        }
        self.armed = false;
        result
    }
}

impl<const N: usize> Drop for ParserActionHost<'_, '_, N> {
    fn drop(&mut self) {
        if self.armed {
            mark_unwind(self.execution_state, self.failure, self.boundary);
            if self.frame.active && self.frame.resolve(self.work).is_err() {
                record(self.failure, PreparationFailureCode::ledger());
            }
        }
    }
}

/// A short loan of one actual parser row and remainder, with no storage or callback runner.
/// The enclosing host retains the frame and its armed unwind guard.
pub struct StoreParserReadRow<'read> {
    row: ParserReadRow<'read, StoreDimensionKey>,
    failure: &'read Cell<Option<PreparationFailureCode>>,
    debt: Option<&'read Cell<u16>>,
    stopped: bool,
}

impl StoreParserReadRow<'_> {
    /// Returns false before a direct-mode checkpoint is due. End this loan and checkpoint
    /// through the parser host before reading another unit; loan entry never resets debt.
    #[inline]
    pub fn ready_for_read(&mut self) -> Result<bool, StoreInvocationError> {
        if self.stopped {
            return Err(StoreRuntimeError::FallbackOnly.into());
        }
        match self.debt.map(|debt| checked_action_debt(debt.get(), 1)) {
            None => Ok(true),
            Some(Ok(next)) => Ok(next.is_some()),
            Some(Err(())) => Err(self.reject_cost()),
        }
    }

    /// Commits one literal returned read before any later policy, write or control action.
    /// Numeric failure leaves row/remainder unchanged and stops every later call in this loan.
    #[inline]
    pub fn complete_read(&mut self, amount: u64) -> Result<(), StoreInvocationError> {
        if self.stopped {
            return Err(StoreRuntimeError::FallbackOnly.into());
        }
        let next_debt = match self.debt.map(|debt| checked_action_debt(debt.get(), 1)) {
            None => None,
            Some(Ok(Some(next))) => Some(next),
            Some(Ok(None) | Err(())) => return Err(self.reject_cost()),
        };
        if let Err(cause) = self.row.complete(amount) {
            self.stopped = true;
            record(self.failure, PreparationFailureCode::ledger());
            return Err(StoreRuntimeError::Ledger(cause).into());
        }
        if let Some((debt, next)) = self.debt.zip(next_debt) {
            debt.set(next);
        }
        Ok(())
    }

    fn reject_cost(&mut self) -> StoreInvocationError {
        self.stopped = true;
        record(
            self.failure,
            PreparationFailureCode::product_graph(INVALID_COST),
        );
        StoreRuntimeError::FallbackOnly.into()
    }
}

/// A borrowed parser view. The host keeps the actual frame even if this borrow is forgotten.
/// Only a parent has `child`; a child can perform prefixes and storage, then returns its borrow.
///
/// A valid sequential child precedes the parent's semantic completion:
///
/// ```
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::{StoreInvocation, StoreInvocationError, StoreInvocationLayout};
/// fn nested<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     context: &mut StoreInvocation<'op, O, P, U>, layout: StoreInvocationLayout<'op, 1>,
/// ) -> Result<(), StoreInvocationError> {
///     context.parser_parent::<_, StoreInvocationError, 1>(layout, [1], [None], |parent| {
///         parent.child::<_, StoreInvocationError, 1>(layout, [1], [None], |child| child.run_prefix([1], 2, || Ok(())))?;
///         parent.run_prefix([1], 3, || Ok(()))
///     })
/// }
/// ```
///
/// The live child callback cannot reenter its borrowed parent:
///
/// ```compile_fail,E0499
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::{StoreInvocation, StoreInvocationError, StoreInvocationLayout};
/// fn overlap<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     context: &mut StoreInvocation<'op, O, P, U>, layout: StoreInvocationLayout<'op, 1>,
/// ) -> Result<(), StoreInvocationError> {
///     context.parser_parent::<_, StoreInvocationError, 1>(layout, [1], [None], |parent| {
///         let parent_alias = &mut *parent;
///         parent.child::<_, StoreInvocationError, 1>(layout, [1], [None], |_child| parent_alias.run_prefix([1], 1, || Ok(())))
///     })
/// }
/// ```
///
/// A child borrow cannot escape its host frame:
///
/// ```compile_fail
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::{StoreInvocation, StoreInvocationError, StoreInvocationLayout};
/// fn child_escape<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     context: &mut StoreInvocation<'op, O, P, U>, layout: StoreInvocationLayout<'op, 1>,
/// ) -> Result<(), StoreInvocationError> {
///     context.parser_parent::<_, StoreInvocationError, 1>(layout, [0], [None], |parent| {
///         let _escaped = parent.child::<_, StoreInvocationError, 1>(layout, [0], [None], |child| Ok::<_, StoreInvocationError>(child))?;
///         Ok(())
///     })
/// }
/// ```
///
/// `child` intentionally exists only on the present parent role (`CHILD = false`), so a
/// present child role cannot open another child. The sequential-child positive above uses
/// the same method on the permitted role; this is a role gate, not a stale-symbol example.
///
/// ```compile_fail,E0599
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// use baozi_operation::__retained_store_integration::activation::{StoreInvocationParser, StoreInvocationError, StoreInvocationLayout};
/// fn nested_child<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     child: &mut StoreInvocationParser<'_, '_, 'op, O, P, U, 1, true>, layout: StoreInvocationLayout<'op, 1>,
/// ) {
///     let _ = child.child::<(), StoreInvocationError, 1>(layout, [0], [None], |_| Ok(()));
/// }
/// ```
pub struct StoreInvocationParser<'frame, 'ctx, 'op, O, PP, U, const N: usize, const CHILD: bool> {
    host: &'frame mut ParserHost<'ctx, 'op, O, PP, U, N>,
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, const N: usize, const CHILD: bool>
    StoreInvocationParser<'_, '_, 'op, O, PP, U, N, CHILD>
{
    /// Copies the unresolved suffix of this actual host-owned parser frame.
    pub fn remaining(&self) -> [u64; N] {
        self.host.frame.remaining
    }

    /// Reborrows the same progress owner while this parser's numeric frame remains active.
    pub fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), StoreInvocationError> {
        self.host
            .context
            .report_progress(phase, unit, completed, total)
    }

    /// Runs a fallible parser piece and records its prefix only after success. A normal Err
    /// closes the channel with prior verified pieces intact and returns the original error.
    pub fn run_prefix<T, E: From<StoreInvocationError>>(
        &mut self,
        amounts: [u64; N],
        control_cost: u16,
        action: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        self.run_prefix_with_failure(amounts, control_cost, action, |_, cause| cause)
    }

    /// Trusted compound failure service; the decoder itself receives no context.
    pub fn run_prefix_with_failure<T, P, E: From<StoreInvocationError>>(
        &mut self,
        amounts: [u64; N],
        control_cost: u16,
        action: impl FnOnce() -> Result<T, P>,
        materialize: impl FnOnce(&mut Self, P) -> E,
    ) -> Result<T, E> {
        let mut slot = Some(|_storage: StoreOrdinaryStorage<'_, 'op, O, PP, U>| action());
        self.run_prefix_with_storage(amounts, control_cost, &mut slot, E::from, materialize)
    }

    /// One guarded prefix runner; runtime projection precedes disposal of uncalled captures.
    pub fn run_prefix_with_storage<T, P, E, F>(
        &mut self,
        amounts: [u64; N],
        control_cost: u16,
        action: &mut Option<F>,
        project: impl FnOnce(StoreInvocationError) -> E,
        materialize: impl FnOnce(&mut Self, P) -> E,
    ) -> Result<T, E>
    where
        F: for<'s> FnOnce(StoreOrdinaryStorage<'s, 'op, O, PP, U>) -> Result<T, P>,
    {
        self.run_guarded_prefix(
            Some(amounts),
            control_cost,
            action,
            project,
            materialize,
            |_| amounts,
        )
    }

    /// Records the literal completed amounts returned by one physical scanner invocation.
    /// The existing parser admission bounds the result; no preliminary scan is required.
    pub fn run_returned_prefix_with_storage<T, P, E, F>(
        &mut self,
        control_cost: u16,
        action: &mut Option<F>,
        project: impl FnOnce(StoreInvocationError) -> E,
        materialize: impl FnOnce(&mut Self, P) -> E,
    ) -> Result<T, E>
    where
        F: for<'s> FnOnce(StoreOrdinaryStorage<'s, 'op, O, PP, U>) -> Result<(T, [u64; N]), P>,
    {
        self.run_guarded_prefix(
            None,
            control_cost,
            action,
            project,
            materialize,
            |(_, amounts)| *amounts,
        )
        .map(|(value, _)| value)
    }

    fn run_guarded_prefix<T, P, E, F>(
        &mut self,
        expected: Option<[u64; N]>,
        control_cost: u16,
        action: &mut Option<F>,
        project: impl FnOnce(StoreInvocationError) -> E,
        materialize: impl FnOnce(&mut Self, P) -> E,
        completed: impl FnOnce(&T) -> [u64; N],
    ) -> Result<T, E>
    where
        F: for<'s> FnOnce(StoreOrdinaryStorage<'s, 'op, O, PP, U>) -> Result<T, P>,
    {
        let captured_cost = control_cost;
        let entry = if self.host.context.boundary.is_none() {
            let checkpoint = if !CHILD {
                self.host.context.checkpoint()
            } else {
                Ok(())
            };
            match checkpoint {
                Ok(()) => self.host.context.before(captured_cost),
                Err(cause) => Err(cause),
            }
        } else {
            self.host.context.running()
        };
        if let Err(cause) = entry {
            return Err(project(cause));
        }
        if expected.is_some_and(|amounts| {
            amounts
                .iter()
                .zip(self.host.frame.remaining)
                .any(|(amount, remaining)| *amount > remaining)
        }) {
            let cause = self
                .host
                .context
                .reject(StoreRuntimeError::ObservedWorkExceedsAdmitted);
            return Err(project(cause));
        }
        let failure = self.host.context.parts.ordinary.failure;
        let mut prefix = ParserActionHost {
            work: &mut self.host.context.parts.work.work,
            frame: &mut self.host.frame,
            layout: self.host.layout,
            failure,
            execution_state: self.host.context.parts.ordinary.execution_state,
            boundary: self.host.context.boundary,
            armed: true,
        };
        let storage = StoreOrdinaryStorage {
            domain: self.host.context.domain,
            raw: &mut self.host.context.parts.ordinary,
        };
        let action = action
            .take()
            .expect("a host-owned prefix callback runs once");
        let value = match action(storage) {
            Ok(value) => value,
            Err(cause) => {
                prefix.armed = false;
                drop(prefix);
                return Err(self.materialize_error(cause, materialize));
            }
        };
        let result = prefix.complete(completed(&value));
        drop(prefix);
        if let Err(cause) = result {
            return Err(project(StoreRuntimeError::Ledger(cause).into()));
        }
        if self.host.context.boundary.is_none()
            && let Err(cause) = self.host.context.completed(captured_cost)
        {
            return Err(project(cause));
        }
        Ok(value)
    }

    fn materialize_error<P, E>(
        &mut self,
        cause: P,
        materialize: impl FnOnce(&mut Self, P) -> E,
    ) -> E {
        self.host.context.stop_known_error();
        let owned = materialize(self, cause);
        if self.host.frame.active {
            let _ = ledger(
                self.host.context.parts.ordinary.failure,
                self.host
                    .frame
                    .resolve(&mut self.host.context.parts.work.work),
            );
        }
        owned
    }

    /// A live loan excludes reentry of this parser role.
    pub fn ordinary_storage(&mut self) -> StoreOrdinaryStorage<'_, 'op, O, PP, U> {
        self.host.context.ordinary_storage()
    }

    pub fn failure_storage(&mut self) -> StoreFailureStorage<'_, 'op, O, PP, U> {
        StoreFailureStorage {
            context: self.host.context,
        }
    }

    pub fn work_counters(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<(u64, u64, u64), StoreInvocationError> {
        self.host.context.work_counters(dimension)
    }

    pub fn prescribed_exit_checkpoint(&mut self) -> Result<(), StoreInvocationError> {
        self.host.context.prescribed_exit_checkpoint()
    }
    pub fn reserve_failure_storage(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreInvocationError> {
        self.host
            .context
            .reserve_failure_storage(phase, dimension, bytes, allocations)
    }
    pub fn resize_failure_storage(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        bytes: u64,
    ) -> Result<(), StoreInvocationError> {
        self.host.context.resize_failure_storage(reservation, bytes)
    }

    /// Explicit parser error/IO/control frontier, using the invocation's sole control owner.
    pub fn checkpoint(&mut self) -> Result<(), StoreInvocationError> {
        self.host.context.checkpoint()
    }

    /// Binds another group in the same invariant catalog.
    pub fn bind_group<const M: usize>(
        &mut self,
        dimensions: [StoreDimensionKey; M],
    ) -> Result<StoreInvocationLayout<'op, M>, StoreInvocationError> {
        self.host.context.bind_group(dimensions)
    }

    /// Actual disjoint storage/Internal work while this attempted-only channel remains open.
    pub fn allocate_bytes(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        count: usize,
    ) -> Result<crate::retained::RetainedStorage<Vec<u8>>, StoreInvocationError> {
        self.host.context.allocate_bytes(phase, dimension, count)
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit>
    StoreInvocationParser<'_, '_, 'op, O, PP, U, 1, true>
{
    /// Lends the actual singleton row to a closed first-party read loop. Each read completes
    /// immediately; a healthy return keeps the admitted frame open for the next region.
    /// The loop must return before a due checkpoint; the host owns control and settlement.
    ///
    /// An owned result can leave the region; the borrowed row cannot:
    ///
    /// ```
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
    /// use baozi_operation::__retained_store_integration::activation::{
    ///     StoreInvocationError, StoreInvocationParser,
    /// };
    /// fn ready<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     child: &mut StoreInvocationParser<'_, '_, '_, O, P, U, 1, true>,
    /// ) -> Result<bool, StoreInvocationError> {
    ///     child.with_read_row(&mut |row| row.ready_for_read(), |cause| cause, |_, cause| cause)
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
    /// use baozi_operation::__retained_store_integration::activation::{
    ///     StoreInvocationError, StoreInvocationParser,
    /// };
    /// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     child: &mut StoreInvocationParser<'_, '_, '_, O, P, U, 1, true>,
    /// ) {
    ///     let _escaped = child.with_read_row(
    ///         &mut |row| Ok::<_, StoreInvocationError>(row),
    ///         |cause| cause, |_, cause| cause,
    ///     );
    /// }
    /// ```
    pub fn with_read_row<T, P, E>(
        &mut self,
        body: &mut impl FnMut(&mut StoreParserReadRow<'_>) -> Result<T, P>,
        project: impl FnOnce(StoreInvocationError) -> E,
        materialize: impl FnOnce(&mut Self, P) -> E,
    ) -> Result<T, E> {
        if let Err(cause) = self.host.context.running() {
            return Err(project(cause));
        }
        let failure = self.host.context.parts.ordinary.failure;
        let debt = self
            .host
            .context
            .boundary
            .is_none()
            .then_some(self.host.context.parts.ordinary.debt);
        let mut scope = ParserActionHost {
            work: &mut self.host.context.parts.work.work,
            frame: &mut self.host.frame,
            layout: self.host.layout,
            failure,
            execution_state: self.host.context.parts.ordinary.execution_state,
            boundary: self.host.context.boundary,
            armed: true,
        };
        let (result, stopped) = {
            let row = match scope.frame.borrow_read_row(&scope.layout, scope.work) {
                Ok(row) => row,
                Err(cause) => {
                    record(failure, PreparationFailureCode::ledger());
                    let _ = scope.frame.resolve(scope.work);
                    scope.armed = false;
                    return Err(project(StoreRuntimeError::Ledger(cause).into()));
                }
            };
            let mut reads = StoreParserReadRow {
                row,
                failure,
                debt,
                stopped: false,
            };
            let result = body(&mut reads);
            (result, reads.stopped)
        };
        scope.armed = false;
        drop(scope);
        if stopped && result.is_ok() {
            let _ = ledger(
                failure,
                self.host
                    .frame
                    .resolve(&mut self.host.context.parts.work.work),
            );
            let cause = project(StoreRuntimeError::FallbackOnly.into());
            drop(result);
            return Err(cause);
        }
        match result {
            Ok(value) => {
                self.host.context.running().map_err(project)?;
                Ok(value)
            }
            Err(cause) => Err(self.materialize_error(cause, materialize)),
        }
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, const CHILD: bool>
    StoreInvocationParser<'_, '_, 'op, O, PP, U, 1, CHILD>
{
    /// Admits another chunk only after consuming the prior chunk, without a per-item limit.
    pub fn replenish(&mut self, amount: u64) -> Result<(), StoreInvocationError> {
        self.host.context.running()?;
        ledger(
            self.host.context.parts.ordinary.failure,
            self.host.frame.replenish(
                &self.host.layout,
                &mut self.host.context.parts.work.work,
                amount,
            ),
        )
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, const N: usize>
    StoreInvocationParser<'_, '_, 'op, O, PP, U, N, false>
{
    pub fn complete_parent(&mut self) -> Result<(), StoreInvocationError> {
        self.host.context.running()?;
        let remaining = self.host.frame.remaining;
        ledger(
            self.host.context.parts.ordinary.failure,
            self.host.frame.complete(
                &self.host.layout,
                &mut self.host.context.parts.work.work,
                remaining,
            ),
        )
    }

    /// Sequential child reborrow; safe Rust prevents using the parent while this callback runs.
    pub fn child<T, E: From<StoreInvocationError>, const M: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, M>,
        amounts: [u64; M],
        limits: [Option<u64>; M],
        body: impl FnOnce(&mut StoreInvocationParser<'_, '_, 'op, O, PP, U, M, true>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.host
            .context
            .parser_child(layout, amounts, limits, body)
    }

    /// Reborrows the parent's invocation for one trusted sequential text scan.
    pub fn scan_child<T, E: From<StoreInvocationError>, const M: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, M>,
        amounts: [u64; M],
        limits: [Option<u64>; M],
        body: impl FnOnce(
            &mut StoreInvocationParser<'_, '_, 'op, O, PP, U, M, true>,
        ) -> Result<StoreScanReturn<T>, E>,
    ) -> Result<T, E> {
        self.host
            .context
            .parser_scan_child(layout, amounts, limits, body)
    }
}

struct RegionHost<'op, O, PP, U> {
    context: StoreInvocation<'op, O, PP, U>,
    guard: InvocationRegionGuard<'op>,
}

struct InvocationRegionGuard<'op> {
    state: &'op Cell<StoreExecutionState>,
    failure: &'op Cell<Option<PreparationFailureCode>>,
    boundary: Option<&'op StoreImporterBoundary>,
    resolved: bool,
}

enum RegionPolicy {
    RootLoan,
    Parser,
    Exclusive,
}

impl<'op> InvocationRegionGuard<'op> {
    fn new<O, PP, U>(context: &StoreInvocation<'op, O, PP, U>) -> Self {
        Self {
            state: context.parts.ordinary.execution_state,
            failure: context.parts.ordinary.failure,
            boundary: context.boundary,
            resolved: false,
        }
    }
}

impl Drop for InvocationRegionGuard<'_> {
    fn drop(&mut self) {
        if !self.resolved && std::thread::panicking() {
            mark_unwind(self.state, self.failure, self.boundary);
        }
    }
}

impl<'inv, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, L>
    RetainedStoreOperation<'inv, O, PP, U, L>
{
    /// Generative closed entry. No owning numeric permit or mutable invocation escapes.
    /// Known errors close without an additional poll; success has an explicit final checkpoint.
    pub fn with_exclusive_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.execute_region(None, RegionPolicy::Exclusive, body)
    }

    /// Ordinary root loans preserve the former loan's one entry checkpoint.
    pub(crate) fn with_root_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        boundary: &StoreImporterBoundary,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.execute_region(Some(boundary), RegionPolicy::RootLoan, body)
    }

    pub(crate) fn with_parser_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        boundary: &StoreImporterBoundary,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.execute_region(Some(boundary), RegionPolicy::Parser, body)
    }

    fn execute_region<R, E: From<StoreInvocationError>>(
        &mut self,
        boundary: Option<&StoreImporterBoundary>,
        policy: RegionPolicy,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        let context = self
            .invocation_context(boundary)
            .map_err(StoreInvocationError::from)?;
        execute_invocation_region(context, policy, body)
    }

    pub(in crate::concrete_store) fn invocation_context<'op>(
        &'op mut self,
        boundary: Option<&'op StoreImporterBoundary>,
    ) -> Result<StoreInvocation<'op, O, PP, U>, StoreRuntimeError> {
        let parts = self.raw.exclusive_parts().map_err(map_execution_error)?;
        bind_invocation_context(parts, &self.domain, boundary)
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, L>
    BoundStoreOperation<'_, '_, O, PP, U, L>
{
    /// Runs client prework through the same region host used after retained attachment.
    pub fn with_exclusive_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        let context = self
            .invocation_context()
            .map_err(StoreInvocationError::from)?;
        execute_invocation_region(context, RegionPolicy::Exclusive, body)
    }

    pub(in crate::concrete_store) fn invocation_context(
        &mut self,
    ) -> Result<StoreInvocation<'_, O, PP, U>, StoreRuntimeError> {
        let parts = self.raw.exclusive_parts().map_err(map_execution_error)?;
        bind_invocation_context(parts, &self.domain, None)
    }
}

fn bind_invocation_context<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit>(
    parts: ExclusiveOperationParts<'op, O, StorePhaseKey, PP, U, StoreDimensionKey>,
    domain: &'op StoreKeyDomain,
    boundary: Option<&'op StoreImporterBoundary>,
) -> Result<StoreInvocation<'op, O, PP, U>, StoreRuntimeError> {
    let share_layout = runtime_ledger(
        parts.ordinary.failure,
        parts
            .work
            .bind(parts.share_work.charges().map(|(dimension, _)| dimension)),
    )?;
    Ok(StoreInvocation {
        parts,
        domain,
        share_layout,
        boundary,
        brand: PhantomData,
        local: PhantomData,
    })
}

fn execute_invocation_region<
    'op,
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    R,
    E: From<StoreInvocationError>,
>(
    context: StoreInvocation<'op, O, PP, U>,
    policy: RegionPolicy,
    body: impl FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
) -> Result<R, E> {
    let mut host = RegionHost {
        guard: InvocationRegionGuard::new(&context),
        context,
    };
    host.context.checkpoint()?;
    let result = body(&mut host.context);
    StoreExecutionState::settle_caught_unwind(
        host.context.parts.ordinary.execution_state,
        host.context.parts.ordinary.failure,
    );
    if result.is_err() {
        host.context.stop_known_error();
    }
    let mut close = ledger(
        host.context.parts.ordinary.failure,
        host.context.parts.work.check_quiescent(),
    );
    if result.is_ok() && close.is_ok() && !matches!(policy, RegionPolicy::RootLoan) {
        if matches!(policy, RegionPolicy::Exclusive)
            && host.context.parts.ordinary.execution_state.get()
                == StoreExecutionState::IncompleteSuccess
        {
            record(
                host.context.parts.ordinary.failure,
                PreparationFailureCode::product_graph(ABANDONED),
            );
        }
        close = host.context.running();
        if close.is_ok() && matches!(policy, RegionPolicy::Exclusive) {
            close = host.context.checkpoint();
        }
    }
    match result {
        Err(cause) => {
            host.guard.resolved = true;
            let _ = close;
            Err(cause)
        }
        Ok(value) => match close {
            Ok(()) => {
                host.guard.resolved = true;
                Ok(value)
            }
            Err(cause) => {
                drop(value);
                host.guard.resolved = true;
                Err(cause.into())
            }
        },
    }
}

#[cfg(test)]
mod tests;
