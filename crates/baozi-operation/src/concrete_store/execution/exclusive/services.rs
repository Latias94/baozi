//! Short reborrows of the actual invocation; neither view owns a numeric frame.
use super::*;

/// Restricted service authority over the same invocation's progress, evidence and storage.
/// Retained storage uses real Internal work; this is not an ordinary-storage-only view.
pub struct StoreInvocationServices<'loan, 'op, O, PP, U> {
    context: &'loan mut StoreInvocation<'op, O, PP, U>,
}

/// Sequential child admission lent by a root or parent, without parent admission/completion.
pub struct StoreChildAdmission<'loan, 'op, O, PP, U> {
    context: &'loan mut StoreInvocation<'op, O, PP, U>,
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> StoreInvocation<'op, O, PP, U> {
    pub(in crate::concrete_store) fn observation_live_bytes(
        &self,
        observation: crate::ledger::LedgerStorageObservation<'op, StorePhaseKey, StoreDimensionKey>,
    ) -> u64 {
        self.parts
            .ordinary
            .storage
            .observation_live_bytes(observation)
    }

    /// Borrows shared release evidence without lending any numeric work authority.
    pub fn storage_observation(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<StoreStorageObservation<'op>, StoreInvocationError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(self.reject(StoreRuntimeError::ForeignDimensionDomain { dimension }));
        }
        ledger(
            self.parts.ordinary.failure,
            self.parts.ordinary.storage.storage_observation(dimension),
        )
        .map(|raw| StoreStorageObservation { raw })
    }

    pub fn services(&mut self) -> StoreInvocationServices<'_, 'op, O, PP, U> {
        StoreInvocationServices { context: self }
    }

    pub fn child_admission(&mut self) -> StoreChildAdmission<'_, 'op, O, PP, U> {
        StoreChildAdmission { context: self }
    }

    /// Reuses an invocation-bound layout without searching or borrowing a second numeric owner.
    pub fn charge_attempted_bound<const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
    ) -> Result<(), StoreInvocationError> {
        self.running()?;
        ledger(
            self.parts.ordinary.failure,
            self.parts.work.charge_attempted(layout.raw, amounts),
        )
    }

    /// Physical completion remains recordable after an earlier cause, as for the original group.
    pub fn complete_observed_bound<const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
    ) -> Result<(), StoreInvocationError> {
        ledger(
            self.parts.ordinary.failure,
            self.parts.work.complete_observed(layout.raw, amounts),
        )
    }

    pub fn complete_direct_bound<const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
    ) -> Result<(), StoreInvocationError> {
        ledger(
            self.parts.ordinary.failure,
            self.parts.work.complete_direct(layout.raw, amounts),
        )
    }

    pub fn charge_observed_bound<const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
    ) -> Result<(), StoreInvocationError> {
        ledger(
            self.parts.ordinary.failure,
            self.parts.work.charge_observed(layout.raw, amounts),
        )
    }

    pub fn dimension_state(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<RetainedStoreDimensionState, StoreInvocationError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(self.reject(StoreRuntimeError::ForeignDimensionDomain { dimension }));
        }
        let snapshot = ledger(
            self.parts.ordinary.failure,
            self.parts
                .work
                .dimension_evidence(&self.parts.ordinary.storage, dimension),
        )?;
        Ok(RetainedStoreDimensionState {
            attempted: snapshot.attempted(),
            consumed: snapshot.consumed(),
            live_bytes: snapshot.current_live_bytes(),
        })
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, const N: usize, const CHILD: bool>
    StoreInvocationParser<'_, '_, 'op, O, PP, U, N, CHILD>
{
    pub fn services(&mut self) -> StoreInvocationServices<'_, 'op, O, PP, U> {
        self.host.context.services()
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, const N: usize>
    StoreInvocationParser<'_, '_, 'op, O, PP, U, N, false>
{
    /// Child admission is available only while borrowing a parent or root.
    ///
    /// ```compile_fail,E0599
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
    /// use baozi_operation::__retained_store_integration::activation::StoreInvocationParser;
    /// fn nested<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     child: &mut StoreInvocationParser<'_, '_, '_, O, P, U, 1, true>,
    /// ) {
    ///     let _ = child.child_admission();
    /// }
    /// ```
    pub fn child_admission(&mut self) -> StoreChildAdmission<'_, 'op, O, PP, U> {
        self.host.context.child_admission()
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit>
    StoreChildAdmission<'_, 'op, O, PP, U>
{
    pub fn services(&mut self) -> StoreInvocationServices<'_, 'op, O, PP, U> {
        self.context.services()
    }

    pub fn bind_group<const N: usize>(
        &mut self,
        dimensions: [StoreDimensionKey; N],
    ) -> Result<StoreInvocationLayout<'op, N>, StoreInvocationError> {
        self.context.bind_group(dimensions)
    }

    pub fn prescribed_exit_checkpoint(&mut self) -> Result<(), StoreInvocationError> {
        self.context.prescribed_exit_checkpoint()
    }

    pub fn child<T, E: From<StoreInvocationError>, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        limits: [Option<u64>; N],
        body: impl FnOnce(&mut StoreInvocationParser<'_, '_, 'op, O, PP, U, N, true>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.context.parser_child(layout, amounts, limits, body)
    }

    /// Executes one exact child piece under its original frame host. The caller retains
    /// uncalled captures until admission/runtime errors have been projected.
    ///
    /// An owning result can leave the piece:
    ///
    /// ```
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
    /// use baozi_operation::__retained_store_integration::activation::{
    ///     StoreChildAdmission, StoreInvocationError, StoreInvocationLayout,
    ///     StoreOrdinaryStorage,
    /// };
    /// fn piece<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     children: &mut StoreChildAdmission<'_, 'op, O, P, U>,
    ///     layout: StoreInvocationLayout<'op, 1>,
    /// ) -> Result<u32, StoreInvocationError> {
    ///     let mut action = Some(|_storage: StoreOrdinaryStorage<'_, 'op, O, P, U>| {
    ///         Ok::<_, StoreInvocationError>((7, [1]))
    ///     });
    ///     children.run_piece_with_storage(
    ///         layout, [1], [None], &mut action, |cause| cause, |_, cause| cause,
    ///     )
    /// }
    /// ```
    ///
    /// The piece cannot return its storage loan:
    ///
    /// ```compile_fail
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
    /// use baozi_operation::__retained_store_integration::activation::{
    ///     StoreChildAdmission, StoreInvocationError, StoreInvocationLayout,
    ///     StoreOrdinaryStorage,
    /// };
    /// fn escape<'op, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     children: &mut StoreChildAdmission<'_, 'op, O, P, U>,
    ///     layout: StoreInvocationLayout<'op, 1>,
    /// ) {
    ///     let mut action = Some(|storage: StoreOrdinaryStorage<'_, 'op, O, P, U>| {
    ///         Ok::<_, StoreInvocationError>((storage, [1]))
    ///     });
    ///     let _ = children.run_piece_with_storage(
    ///         layout, [1], [None], &mut action, |cause| cause, |_, cause| cause,
    ///     );
    /// }
    /// ```
    pub fn run_piece_with_storage<T, P, E, F, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        limits: [Option<u64>; N],
        action: &mut Option<F>,
        project: impl FnOnce(StoreInvocationError) -> E,
        materialize: impl FnOnce(&mut StoreInvocationParser<'_, '_, 'op, O, PP, U, N, true>, P) -> E,
    ) -> Result<T, E>
    where
        E: From<StoreInvocationError>,
        F: for<'s> FnOnce(StoreOrdinaryStorage<'s, 'op, O, PP, U>) -> Result<(T, [u64; N]), P>,
    {
        self.context.parser_piece_with_storage(
            layout,
            amounts,
            limits,
            action,
            project,
            materialize,
        )
    }

    pub fn scan_child<T, E: From<StoreInvocationError>, const N: usize>(
        &mut self,
        layout: StoreInvocationLayout<'op, N>,
        amounts: [u64; N],
        limits: [Option<u64>; N],
        body: impl FnOnce(
            &mut StoreInvocationParser<'_, '_, 'op, O, PP, U, N, true>,
        ) -> Result<StoreScanReturn<T>, E>,
    ) -> Result<T, E> {
        self.context
            .parser_scan_child(layout, amounts, limits, body)
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit>
    StoreInvocationServices<'_, 'op, O, PP, U>
{
    pub fn checkpoint(&mut self) -> Result<(), StoreInvocationError> {
        self.context.checkpoint()
    }

    pub fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), StoreInvocationError> {
        self.context.report_progress(phase, unit, completed, total)
    }

    pub fn ordinary_storage(&mut self) -> StoreOrdinaryStorage<'_, 'op, O, PP, U> {
        self.context.ordinary_storage()
    }

    pub fn failure_storage(&mut self) -> StoreFailureStorage<'_, 'op, O, PP, U> {
        self.context.failure_storage()
    }

    /// Normal retained evidence requires a running invocation, including under a live parser.
    pub fn reserve_retained(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreInvocationError> {
        self.context.running()?;
        if !self.context.domain.owns_phase(phase) {
            return Err(self
                .context
                .reject(StoreRuntimeError::ForeignPhaseDomain { phase }));
        }
        if !self.context.domain.owns_dimension(dimension) {
            return Err(self
                .context
                .reject(StoreRuntimeError::ForeignDimensionDomain { dimension }));
        }
        self.context.checkpoint()?;
        self.context
            .retain_backing(phase, dimension, bytes, allocations)
    }

    pub fn resize(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        bytes: u64,
    ) -> Result<(), StoreInvocationError> {
        self.context.running()?;
        resize_store_reservation(
            &self.context.parts.ordinary.storage,
            self.context.parts.ordinary.failure,
            reservation,
            bytes,
        )
        .map_err(Into::into)
    }

    pub fn validate_live_owner(
        &self,
        reservation: &RetainedStoreReservation<O>,
    ) -> Result<(), StoreInvocationError> {
        self.context
            .validate_live_owner(reservation)
            .map_err(Into::into)
    }

    /// Reclassifies the same live reservation through the existing atomic storage algorithm.
    pub fn transition(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
    ) -> Result<(), StoreInvocationError> {
        if !self.context.domain.owns_phase(phase) {
            return Err(self
                .context
                .reject(StoreRuntimeError::ForeignPhaseDomain { phase }));
        }
        if !self.context.domain.owns_dimension(dimension) {
            return Err(self
                .context
                .reject(StoreRuntimeError::ForeignDimensionDomain { dimension }));
        }
        if reservation
            .physical
            .as_ref()
            .is_some_and(|lease| bytes > lease.admitted_bytes())
        {
            return Err(self
                .context
                .reject(StoreRuntimeError::ConstructionInvariantUnverified));
        }
        ledger(
            self.context.parts.ordinary.failure,
            self.context.parts.ordinary.storage.transition(
                &mut reservation.raw,
                phase,
                dimension,
                bytes,
            ),
        )?;
        if let Some(physical) = &mut reservation.physical {
            physical.shrink_to(bytes).map_err(|cause| {
                StoreInvocationError::from(map_writer_error(&RetainedWriterError::Physical(cause)))
            })?;
        }
        Ok(())
    }

    pub fn charge_attempted<const N: usize>(
        &mut self,
        dimensions: [StoreDimensionKey; N],
        amounts: [u64; N],
    ) -> Result<(), StoreInvocationError> {
        let layout = self.context.bind_work(dimensions)?;
        self.context.charge_attempted_bound(layout, amounts)
    }

    pub fn complete_observed<const N: usize>(
        &mut self,
        dimensions: [StoreDimensionKey; N],
        amounts: [u64; N],
    ) -> Result<(), StoreInvocationError> {
        let layout = self.context.bind_work(dimensions)?;
        self.context.complete_observed_bound(layout, amounts)
    }

    pub fn complete_direct<const N: usize>(
        &mut self,
        dimensions: [StoreDimensionKey; N],
        amounts: [u64; N],
    ) -> Result<(), StoreInvocationError> {
        let layout = self.context.bind_work(dimensions)?;
        self.context.complete_direct_bound(layout, amounts)
    }

    pub fn charge_observed<const N: usize>(
        &mut self,
        dimensions: [StoreDimensionKey; N],
        amounts: [u64; N],
    ) -> Result<(), StoreInvocationError> {
        let layout = self.context.bind_work(dimensions)?;
        self.context.charge_observed_bound(layout, amounts)
    }
}
