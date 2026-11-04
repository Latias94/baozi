//! Geometry action events over the invocation's unique numeric loan.

use super::*;
use crate::retained::ORDINARY_WORK_PERMIT_ABANDONED_CODE;

/// A borrowed action host for kernels that report separate begin and completion events.
///
/// The host owns the unfinished frame. Clients receive only a mutable borrow, so forgetting
/// that borrow cannot discard its pending work or cleanup. Normally completed leaves reuse
/// one producer envelope through the existing singleton replenishment kernel.
///
/// Owned results can leave the execution region:
/// ```
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey, StoreRuntimeError};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// fn owned<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: &mut RetainedStoreOperation<'_, O, P, U>, key: StoreDimensionKey,
/// ) -> Result<u64, StoreRuntimeError> {
///     operation.with_action_execution(key, |execution| {
///         execution.begin_action(1)?;
///         let value = 7;
///         execution.complete_action()?;
///         Ok(value)
///     })
/// }
/// ```
///
/// The host's mutable loan cannot escape the region:
/// ```compile_fail
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey, StoreRuntimeError};
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreOperation;
/// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: &mut RetainedStoreOperation<'_, O, P, U>, key: StoreDimensionKey,
/// ) {
///     let _ = operation.with_action_execution::<_, StoreRuntimeError>(key, |execution| Ok(execution));
/// }
/// ```
pub struct StoreActionExecution<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> {
    context: StoreInvocation<'op, O, PP, U>,
    layout: WorkLayout<1>,
    pending: Option<WorkFrame<1>>,
    action_pending: Option<u16>,
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> StoreActionExecution<'_, O, PP, U> {
    /// Polls the invocation's existing cancellation and deadline owner.
    pub fn checkpoint(&mut self) -> Result<(), StoreRuntimeError> {
        self.context
            .parts
            .ordinary
            .checkpoint()
            .map_err(map_execution_error)
    }

    /// Admits exactly one real action, capturing its control cost before physical execution.
    pub fn begin_action(&mut self, max_cost: u16) -> Result<(), StoreRuntimeError> {
        if self.action_pending.is_some() {
            return Err(StoreRuntimeError::ObservedWorkOutstanding);
        }
        self.context
            .parts
            .ordinary
            .before_action(max_cost)
            .map_err(map_execution_error)?;
        let failure = self.context.parts.ordinary.failure;
        match &mut self.pending {
            Some(frame) => runtime_ledger(
                failure,
                frame.replenish_one(&self.layout, &mut self.context.parts.work.work),
            )?,
            None => {
                self.pending = Some(runtime_ledger(
                    failure,
                    self.layout.admit_observed(
                        &mut self.context.parts.work.work,
                        [1],
                        WorkLane::Producer,
                    ),
                )?);
            }
        }
        self.action_pending = Some(max_cost);
        Ok(())
    }

    /// Records the literal completed prefix before advancing the captured control debt.
    pub fn complete_action(&mut self) -> Result<(), StoreRuntimeError> {
        let cost = self
            .action_pending
            .ok_or(StoreRuntimeError::ObservedWorkIncomplete)?;
        let frame = self
            .pending
            .as_mut()
            .ok_or(StoreRuntimeError::ObservedWorkIncomplete)?;
        if frame.remaining != [1] {
            return Err(StoreRuntimeError::ObservedWorkIncomplete);
        }
        let completed = runtime_ledger(
            self.context.parts.ordinary.failure,
            frame
                .complete_batch_with_outcome(&self.layout, &mut self.context.parts.work.work, [1])
                .into_result(),
        );
        self.action_pending = None;
        if let Err(cause) = completed {
            let _ = self.resolve_pending();
            return Err(cause);
        }
        let debt = self
            .context
            .parts
            .ordinary
            .completed_action(cost)
            .map_err(map_execution_error);
        if let Err(cause) = debt {
            let _ = self.resolve_pending();
            return Err(cause);
        }
        Ok(())
    }

    /// Tightens the actual joint work/storage policy, retaining already completed evidence.
    pub fn tighten_dimension_policy(
        &mut self,
        dimension: StoreDimensionKey,
        policy: DimensionPolicy,
    ) -> Result<(), StoreRuntimeError> {
        self.validate_dimension(dimension)?;
        self.context
            .parts
            .ordinary
            .running()
            .map_err(map_execution_error)?;
        runtime_ledger(
            self.context.parts.ordinary.failure,
            self.context.parts.work.tighten_policy(
                &self.context.parts.ordinary.storage,
                dimension,
                policy,
            ),
        )
    }

    /// Copies current work facts without an alias to the numeric state.
    pub fn work_counters(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<(u64, u64, u64), StoreRuntimeError> {
        self.validate_dimension(dimension)?;
        runtime_ledger(
            self.context.parts.ordinary.failure,
            self.context.parts.work.counters(dimension),
        )
    }

    /// Reserves actual retained scratch and its real Internal share work on this same owner.
    pub fn reserve_retained(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        if !self.context.domain.owns_phase(phase) {
            record(
                self.context.parts.ordinary.failure,
                PreparationFailureCode::ledger(),
            );
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        self.validate_dimension(dimension)?;
        self.checkpoint()?;
        let mut reservation = self
            .context
            .reserve_backing(phase, dimension, bytes, allocations)?;
        if let Err(cause) = runtime_ledger(
            self.context.parts.ordinary.failure,
            reservation.raw.retain_for_output(),
        ) {
            reservation.release_storage(());
            return Err(cause);
        }
        Ok(reservation)
    }

    /// Reconciles real capacity without another poll or numeric-state borrow.
    pub fn resize(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        super::super::resize_store_reservation(
            &self.context.parts.ordinary.storage,
            self.context.parts.ordinary.failure,
            reservation,
            bytes,
        )
    }

    fn validate_dimension(&self, dimension: StoreDimensionKey) -> Result<(), StoreRuntimeError> {
        if self.context.domain.owns_dimension(dimension) {
            Ok(())
        } else {
            record(
                self.context.parts.ordinary.failure,
                PreparationFailureCode::ledger(),
            );
            Err(StoreRuntimeError::ForeignDimensionDomain { dimension })
        }
    }

    fn resolve_pending(&mut self) -> Result<(), StoreRuntimeError> {
        match self.pending.take() {
            Some(mut frame) => runtime_ledger(
                self.context.parts.ordinary.failure,
                frame.resolve(&self.layout, &mut self.context.parts.work.work, false),
            ),
            None => Ok(()),
        }
    }

    fn finish(&mut self, successful: bool) -> Result<(), StoreRuntimeError> {
        if self.action_pending.is_some() {
            return Err(StoreRuntimeError::ObservedWorkIncomplete);
        }
        self.resolve_pending()?;
        runtime_ledger(
            self.context.parts.ordinary.failure,
            self.context.parts.work.check_quiescent(),
        )?;
        if successful {
            self.context
                .parts
                .ordinary
                .running()
                .map_err(map_execution_error)?;
            self.checkpoint()?;
        }
        Ok(())
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> Drop
    for StoreActionExecution<'_, O, PP, U>
{
    fn drop(&mut self) {
        if self.pending.is_some() {
            record(
                self.context.parts.ordinary.failure,
                PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE),
            );
            let _ = self.resolve_pending();
        }
    }
}

impl<'inv, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, L>
    RetainedStoreOperation<'inv, O, PP, U, L>
{
    /// Lends a host for literal geometry action events, using the existing work kernel.
    /// Entry and successful exit poll once; known errors and unwind never add an exit poll.
    pub fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        work_dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        execute_action_region(self.invocation_context(None)?, work_dimension, body)
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, L>
    BoundStoreOperation<'_, '_, O, PP, U, L>
{
    /// Lends the same action host before attachment, retaining the original work and control.
    ///
    /// ```
    /// use baozi_operation::__retained_store_integration::authority::BoundStoreOperation;
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey, StoreRuntimeError};
    /// fn prework<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     entry: &mut BoundStoreOperation<'_, '_, O, P, U>, key: StoreDimensionKey,
    /// ) -> Result<u64, StoreRuntimeError> {
    ///     entry.with_action_execution(key, |actions| {
    ///         actions.begin_action(1)?;
    ///         let value = 7;
    ///         actions.complete_action()?;
    ///         Ok(value)
    ///     })
    /// }
    /// ```
    ///
    /// The action host cannot escape its callback:
    ///
    /// ```compile_fail
    /// use baozi_operation::__retained_store_integration::authority::BoundStoreOperation;
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey, StoreRuntimeError};
    /// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     entry: &mut BoundStoreOperation<'_, '_, O, P, U>, key: StoreDimensionKey,
    /// ) {
    ///     let _ = entry.with_action_execution::<_, StoreRuntimeError>(key, |actions| Ok(actions));
    /// }
    /// ```
    pub fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        work_dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        execute_action_region(self.invocation_context()?, work_dimension, body)
    }
}

fn execute_action_region<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, R, E>(
    mut context: StoreInvocation<'op, O, PP, U>,
    work_dimension: StoreDimensionKey,
    body: impl FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
) -> Result<R, E>
where
    E: From<StoreRuntimeError>,
{
    let mut guard = InvocationRegionGuard::new(&context);
    // Bind after the real entry checkpoint, matching the existing action contract.
    context
        .parts
        .ordinary
        .checkpoint()
        .map_err(map_execution_error)?;
    if !context.domain.owns_dimension(work_dimension) {
        record(
            context.parts.ordinary.failure,
            PreparationFailureCode::ledger(),
        );
        return Err(StoreRuntimeError::ForeignDimensionDomain {
            dimension: work_dimension,
        }
        .into());
    }
    let layout = runtime_ledger(
        context.parts.ordinary.failure,
        context.parts.work.bind([work_dimension]),
    )?;
    let mut execution = StoreActionExecution {
        context,
        layout,
        pending: None,
        action_pending: None,
    };
    let result = body(&mut execution);
    execution.context.settle_returned_callback();
    if result.is_err() {
        execution.context.stop_known_error();
    }
    let closed = execution.finish(result.is_ok());
    match result {
        Ok(value) => match closed {
            Ok(()) => {
                guard.resolved = true;
                Ok(value)
            }
            Err(cause) => {
                drop(value);
                guard.resolved = true;
                Err(cause.into())
            }
        },
        Err(cause) => {
            guard.resolved = true;
            Err(cause)
        }
    }
}

#[cfg(test)]
mod tests;
