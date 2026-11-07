//! Finite permissions lend the existing ordinary algorithms without exposing dynamic finalizers.

use super::*;

pub(super) type StoreTerminalFailure<O, PP, U> =
    crate::retained::operation::finite_root::FiniteRootTerminalFailure<
        O,
        StorePhaseKey,
        PP,
        U,
        StoreDimensionKey,
    >;

type PublicationOpenResult<'inv, O, PP, U, L> = Result<
    FiniteRetainedStorePublicationAssembly<'inv, O, PP, U, L>,
    FiniteStoreExitError<O, PP, U>,
>;

/// Finite final authority over the original owning field slots.
pub struct FiniteRetainedStoreOperation<'inv, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fields: FiniteStoreFinalFields<'inv, O, PP, U>,
    pub(super) lineage: L,
}

impl<'inv, O, PP, U, L> FiniteRetainedStoreOperation<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn invocation_identity(&mut self) -> *const () {
        self.fields
            .raw
            .loan()
            .expect("finite operation retains its actual fields")
            .invocation_identity()
    }

    pub fn with_exclusive_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut loan = self
            .fields
            .raw
            .loan()
            .ok_or(StoreInvocationError::from(StoreRuntimeError::FallbackOnly))?;
        let parts = loan
            .exclusive_parts()
            .map_err(execution::map_execution_error)
            .map_err(StoreInvocationError::from)?;
        execution::execute_store_parts(
            parts,
            self.fields.domain.as_ref().expect("original finite domain"),
            body,
        )
    }

    pub fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut loan = self
            .fields
            .raw
            .loan()
            .ok_or(StoreRuntimeError::FallbackOnly)?;
        let parts = loan
            .exclusive_parts()
            .map_err(execution::map_execution_error)?;
        execution::execute_store_action_parts(
            parts,
            self.fields.domain.as_ref().expect("original finite domain"),
            dimension,
            body,
        )
    }

    pub fn reserve_retained_storage(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        let domain = self.fields.domain.as_ref().expect("original finite domain");
        if !domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        if !domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        self.fields
            .raw
            .loan()
            .ok_or(StoreRuntimeError::FallbackOnly)?
            .reserve_retained_storage(phase, dimension, bytes, allocations)
            .and_then(|(mut raw, physical)| {
                if let Err(cause) = raw.retain_for_output() {
                    let _ = crate::retained::release_owned_storage(physical, ());
                    return Err(RetainedWriterError::Ledger(cause));
                }
                Ok(RetainedStoreReservation {
                    raw,
                    physical: Some(physical),
                })
            })
            .map_err(|cause| map_writer_error(&cause))
    }

    pub fn try_complete_phase(&mut self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        let domain = self.fields.domain.as_ref().expect("original finite domain");
        if !domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        let mut loan = self
            .fields
            .raw
            .loan()
            .ok_or(StoreRuntimeError::FallbackOnly)?;
        if !loan.contains_phase(phase) {
            return Err(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase));
        }
        loan.try_complete_phase(phase)
            .map_err(map_phase_completion_error)
    }

    pub fn physical_storage_snapshot(&mut self) -> crate::PhysicalStorageAccountSnapshot {
        self.fields
            .raw
            .loan()
            .expect("finite operation retains its actual fields")
            .account()
            .snapshot()
    }

    /// Opens publication tables while retaining the original finite final fields on every error.
    pub fn try_publication_assembly(
        self,
        plan: RetainedStorePublicationPlan,
    ) -> PublicationOpenResult<'inv, O, PP, U, L> {
        use ordinary::{StoreOperationLoan, StorePublicationOpenDisposition};

        let Self { fields, lineage } = self;
        let mut transition = publication::FinitePublicationTransition::new(
            fields,
            StorePublicationState::default(),
            lineage,
        );
        let disposition = (|| {
            let fields = transition.fields.as_mut().expect("original finite fields");
            let raw = fields.raw.loan().ok_or(StoreRuntimeError::FallbackOnly)?;
            let operation = StoreOperationLoan::<O, PP, U, L>::from_raw(
                raw,
                fields.domain.as_ref().expect("original finite domain"),
            );
            if operation.raw.is_terminal_only() {
                return Err(StoreRuntimeError::FallbackOnly);
            }
            operation.validate_publication_plan_keys(&plan)?;
            operation
                .open_publication_loan(plan, &mut transition.state)
                .into_disposition()
        })();
        match disposition {
            Ok(StorePublicationOpenDisposition::Opened(opened)) => {
                let fields = transition.fields.as_mut().expect("original finite fields");
                if fields.raw.bind_opened_stamp(opened).is_err() {
                    return Err(transition
                        .into_open_error(StoreRuntimeError::ConstructionInvariantUnverified));
                }
                Ok(transition.into_assembly())
            }
            Ok(StorePublicationOpenDisposition::Failed(failure)) => {
                let fields = transition.fields.as_mut().expect("original finite fields");
                let failure = match fields.raw.bind_publication_failure(failure) {
                    Ok(failure) | Err(failure) => failure,
                };
                let cause = map_writer_error(failure.cause());
                Err(transition.into_open_error(cause))
            }
            Err(cause) => Err(transition.into_open_error(cause)),
        }
    }

    pub(in crate::concrete_store) fn defer_body(
        self,
        requested: Option<PreparationFailureCode>,
    ) -> Result<(), StoreRuntimeError> {
        let Self {
            mut fields,
            lineage,
        } = self;
        let mut loan = fields.raw.loan().ok_or(StoreRuntimeError::FallbackOnly)?;
        if let Some(cause) = requested {
            loan.record_terminal_cause(cause);
        }
        let result = loan
            .defer_until_body_cleanup()
            .map_err(|_| StoreRuntimeError::OwnershipUnverified);
        drop(lineage);
        result
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> FiniteStoreFinalFields<'_, O, PP, U> {
    pub(in crate::concrete_store) fn prepare_evidence_only(
        mut self,
        progress: FinalProgress<PP, U>,
    ) -> Result<PreparedFiniteStoreEvidenceOnlyOutcome, FiniteStoreExitError<O, PP, U>> {
        match self.raw.finish_evidence_only(progress) {
            Ok(raw) => Ok(terminal_output::finite::classify_evidence_only(
                raw,
                self.take_domain(),
            )),
            Err(failure) => Err(self.terminal_error(failure)),
        }
    }

    pub(in crate::concrete_store) fn prepare_minimal(
        mut self,
        code: u32,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        match self
            .raw
            .finish_minimal(PreparationFailureCode::domain(code))
        {
            Ok(raw) => Ok(terminal_output::finite::classify_fallback(
                raw,
                self.take_domain(),
            )),
            Err(failure) => Err(self.terminal_error(failure)),
        }
    }

    pub(in crate::concrete_store) fn take_domain(&mut self) -> StoreKeyDomain {
        self.domain
            .take()
            .expect("qualified finite terminal takes its original domain once")
    }

    pub(in crate::concrete_store) fn terminal_error(
        self,
        failure: StoreTerminalFailure<O, PP, U>,
    ) -> FiniteStoreExitError<O, PP, U> {
        let cause = FiniteStoreExitCause::Finalize(failure.cause().map_or(
            FiniteStoreFinalizeCause::ForeignTerminalAuthority,
            |cause| {
                FiniteStoreFinalizeCause::Publication(StorePrepareError::FinalizationContainment(
                    *cause,
                ))
            },
        ));
        let mut error = self.into_error(cause);
        error.terminal = Some(failure);
        error
    }
}
