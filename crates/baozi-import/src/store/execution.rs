//! Import-semantic views over the existing invocation's disjoint execution capabilities.

#![allow(
    clippy::result_large_err,
    reason = "import failures preserve exact inline ledger and control evidence"
)]

use baozi_operation::__retained_store_integration::activation::{
    StoreBoundWorkGroup, StoreExecutionPermit, StoreExecutionServices, StoreExecutionWork,
    StoreFailureStorage, StoreInvocation, StoreOrdinaryStorage, StoreParserWorkError,
    StoreParserWorkScope,
};

use super::*;

pub(crate) struct ImportStoreExecution<'region> {
    work: StoreExecutionWork<'region, ImportOperationOwner>,
    services: StoreExecutionServices<
        'region,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
    >,
    vocabulary: &'region ImportStoreVocabulary,
}

pub(crate) struct ImportOrdinaryStorage<'b, 'op: 'b> {
    raw: StoreOrdinaryStorage<
        'b,
        'op,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
    >,
    vocabulary: &'b ImportStoreVocabulary,
}
impl<'b, 'op: 'b> ImportOrdinaryStorage<'b, 'op> {
    pub(crate) fn new(
        raw: StoreOrdinaryStorage<
            'b,
            'op,
            ImportOperationOwner,
            ImportProgressPhase,
            ImportProgressUnit,
        >,
        vocabulary: &'b ImportStoreVocabulary,
    ) -> Self {
        Self { raw, vocabulary }
    }
    pub(crate) fn poll(&mut self) -> Result<(), ImportStoreHostError> {
        self.raw
            .checkpoint()
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
    pub(crate) fn reserve(
        &mut self,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ImportStoreHostError> {
        let raw = self
            .raw
            .reserve(
                self.vocabulary.phase(phase),
                self.vocabulary.dimension(dimension),
                bytes,
            )
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))?;
        Ok(ImportStoreReservation {
            raw: Some(raw),
            bytes,
        })
    }
    pub(crate) fn resize(
        &self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .resize(
                reservation
                    .raw
                    .as_mut()
                    .expect("a live import reservation owns its backing"),
                bytes,
            )
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))?;
        reservation.bytes = bytes;
        Ok(())
    }
}

pub(crate) struct ImportBoundWorkGroup<'region, const N: usize> {
    raw: StoreBoundWorkGroup<'region, N>,
    vocabulary: &'region ImportStoreVocabulary,
}

pub(crate) struct ImportParserWorkScope<'region, const N: usize> {
    raw: StoreParserWorkScope<'region, N>,
    vocabulary: &'region ImportStoreVocabulary,
}

pub(crate) enum ImportParserAdmissionError {
    Store(ImportStoreHostError),
    ItemLimit {
        index: usize,
        value: u64,
        limit: u64,
    },
}

pub(crate) struct ImportObservedExecution<'region, const N: usize> {
    raw: StoreExecutionPermit<'region, N>,
    vocabulary: &'region ImportStoreVocabulary,
}

pub(crate) enum ImportExecutionFailure<E> {
    Runtime(StoreParserWorkError),
    Owned(E),
}
impl<E> From<StoreParserWorkError> for ImportExecutionFailure<E> {
    fn from(cause: StoreParserWorkError) -> Self {
        Self::Runtime(cause)
    }
}

impl ImportStoreReservation {
    pub(crate) fn reserve_failure_storage(
        storage: &mut StoreFailureStorage<
            '_,
            '_,
            ImportOperationOwner,
            ImportProgressPhase,
            ImportProgressUnit,
        >,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<Self, StoreParserWorkError> {
        let raw = storage.reserve(phase, dimension, bytes, allocations)?;
        Ok(Self {
            raw: Some(raw),
            bytes,
        })
    }

    pub(crate) fn resize_failure_storage(
        &mut self,
        storage: &StoreFailureStorage<
            '_,
            '_,
            ImportOperationOwner,
            ImportProgressPhase,
            ImportProgressUnit,
        >,
        bytes: u64,
    ) -> Result<(), StoreParserWorkError> {
        storage.resize(
            self.raw
                .as_mut()
                .expect("a live import reservation owns its backing"),
            bytes,
        )?;
        self.bytes = bytes;
        Ok(())
    }
}

impl ImportStoreSessionOwner {
    pub(crate) fn parser_runtime_failure(
        &self,
        cause: StoreParserWorkError,
        terminal: &std::cell::RefCell<Option<ScopeAbortCause>>,
    ) -> crate::ParseFailure {
        let cause = match cause {
            StoreParserWorkError::Runtime(cause) => {
                project_store_runtime_error(&self.storage[0].vocabulary, cause)
                    .into_scope_abort_cause()
            }
            StoreParserWorkError::ItemLimit { .. } => ScopeAbortCause::DescriptorContract {
                code: "parse-work-item-limit-lane-invalid",
            },
        };
        crate::scope::latch_abort(terminal, cause)
    }

    pub(crate) fn with_parser_execution<R, E>(
        &mut self,
        action: impl for<'op> FnOnce(
            &mut StoreInvocation<'op, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>,
            &ImportStoreVocabulary,
        ) -> Result<R, ImportExecutionFailure<E>>,
    ) -> Result<R, ImportExecutionFailure<E>> {
        let data = &mut self.storage[0];
        let ImportStoreSessionData {
            state,
            vocabulary,
            importer_boundary,
            ..
        } = data;
        let ImportStoreSessionState::Dormant(owner) = state else {
            return Err(ImportExecutionFailure::Runtime(
                StoreParserWorkError::Runtime(baozi_operation::StoreRuntimeError::FallbackOnly),
            ));
        };
        owner.with_parser_execution(importer_boundary, |numeric| action(numeric, vocabulary))
    }

    pub(crate) fn execution(&mut self) -> Result<ImportStoreExecution<'_>, ImportStoreHostError> {
        let data = &mut self.storage[0];
        let vocabulary = &data.vocabulary;
        let state = &mut data.state;
        let ImportStoreSessionState::Dormant(owner) = state else {
            return Err(inactive_store_state_error(state));
        };
        owner
            .check_execution_running()
            .map_err(|cause| project_store_runtime_error(vocabulary, cause))?;
        let (work, services) = owner
            .execution()
            .map_err(|cause| project_store_runtime_error(vocabulary, cause))?;
        Ok(ImportStoreExecution {
            work,
            services,
            vocabulary,
        })
    }
}

impl<'region> ImportStoreExecution<'region> {
    pub(crate) fn ordinary(&mut self) -> ImportOrdinaryStorage<'_, 'region> {
        ImportOrdinaryStorage::new(self.services.ordinary(), self.vocabulary)
    }

    #[cfg(test)]
    pub(crate) fn storage_observation(
        &self,
        dimension: ImportDimension,
    ) -> baozi_operation::__retained_store_integration::authority::StoreStorageObservation<'region>
    {
        self.work
            .storage_observation(self.vocabulary.dimension(dimension))
            .expect("the test observes an admitted storage dimension")
    }

    #[cfg(test)]
    pub(crate) fn validate_live_owner(
        &self,
        reservation: &ImportStoreReservation,
    ) -> Result<(), ImportStoreHostError> {
        self.services
            .validate_live_owner(reservation.raw.as_ref().expect("live reservation"))
            .map_err(|cause| self.runtime_error(cause))
    }

    #[cfg(test)]
    pub(crate) fn transition(
        &self,
        reservation: &mut ImportStoreReservation,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.services
            .transition(
                reservation.raw.as_mut().expect("live reservation"),
                self.vocabulary.phase(phase),
                self.vocabulary.dimension(dimension),
                bytes,
            )
            .map_err(|cause| self.runtime_error(cause))?;
        reservation.bytes = bytes;
        Ok(())
    }

    fn runtime_error(&self, cause: StoreRuntimeError) -> ImportStoreHostError {
        project_store_runtime_error(self.vocabulary, cause)
    }

    pub(crate) fn bind_group<const N: usize>(
        &self,
        dimensions: [ImportDimension; N],
    ) -> Result<ImportBoundWorkGroup<'region, N>, ImportStoreHostError> {
        self.work
            .bind_group(dimensions.map(|dimension| self.vocabulary.dimension(dimension)))
            .map(|raw| ImportBoundWorkGroup {
                raw,
                vocabulary: self.vocabulary,
            })
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn finish_region(&self) -> Result<(), ImportStoreHostError> {
        self.work
            .finish_region()
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn poll(&mut self) -> Result<(), ImportStoreHostError> {
        self.services
            .checkpoint()
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn before_action(&mut self, max_cost: u16) -> Result<(), ImportStoreHostError> {
        self.services
            .before_action(max_cost)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn completed_action(&mut self, max_cost: u16) -> Result<(), ImportStoreHostError> {
        self.services
            .completed_action(max_cost)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn report_progress(
        &mut self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ImportStoreHostError> {
        self.services
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn dimension_state(
        &self,
        dimension: ImportDimension,
    ) -> Result<RetainedStoreDimensionState, ImportStoreHostError> {
        self.work
            .dimension_state(self.vocabulary.dimension(dimension))
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn tighten_dimension_policy(
        &self,
        dimension: ImportDimension,
        policy: DimensionPolicy,
    ) -> Result<(), ImportStoreHostError> {
        self.work
            .tighten_dimension_policy(self.vocabulary.dimension(dimension), policy)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn reserve(
        &mut self,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ImportStoreHostError> {
        let vocabulary = self.vocabulary;
        let raw = self
            .services
            .ordinary()
            .reserve(
                vocabulary.phase(phase),
                vocabulary.dimension(dimension),
                bytes,
            )
            .map_err(|cause| project_store_runtime_error(vocabulary, cause))?;
        Ok(ImportStoreReservation {
            raw: Some(raw),
            bytes,
        })
    }

    pub(crate) fn reserve_retained(
        &mut self,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
        allocations: u64,
    ) -> Result<ImportStoreReservation, ImportStoreHostError> {
        let raw = self
            .services
            .reserve_retained(
                self.vocabulary.phase(phase),
                self.vocabulary.dimension(dimension),
                bytes,
                allocations,
            )
            .map_err(|cause| self.runtime_error(cause))?;
        Ok(ImportStoreReservation {
            raw: Some(raw),
            bytes,
        })
    }

    pub(crate) fn resize(
        &self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.services
            .resize(
                reservation
                    .raw
                    .as_mut()
                    .expect("a live import reservation owns its backing"),
                bytes,
            )
            .map_err(|cause| self.runtime_error(cause))?;
        reservation.bytes = bytes;
        Ok(())
    }

    // Cold adapters bind their complete operation once; repeated parser loops retain the group.
    pub(crate) fn charge_attempted(
        &self,
        dimension: ImportDimension,
        amount: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.bind_group([dimension])?.charge_attempted([amount])
    }

    pub(crate) fn charge_consumed(
        &self,
        dimension: ImportDimension,
        amount: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.bind_group([dimension])?.complete_observed([amount])
    }

    pub(crate) fn charge_observed_batch<const N: usize>(
        &self,
        charges: &[(ImportDimension, u64); N],
    ) -> Result<(), ImportStoreHostError> {
        self.bind_group(charges.map(|(dimension, _)| dimension))?
            .charge_observed(charges.map(|(_, amount)| amount))
    }

    pub(crate) fn charge_consumed_batch<const N: usize>(
        &self,
        charges: &[(ImportDimension, u64); N],
    ) -> Result<(), ImportStoreHostError> {
        self.bind_group(charges.map(|(dimension, _)| dimension))?
            .complete_direct(charges.map(|(_, amount)| amount))
    }

    pub(crate) fn with_observed_work<const N: usize, T, E>(
        &mut self,
        charges: [(ImportDimension, u64); N],
        action: impl FnOnce(&mut Self, &mut ImportObservedExecution<'region, N>) -> Result<T, E>,
    ) -> Result<T, ImportObservedWorkFailure<E>> {
        self.poll().map_err(ImportObservedWorkFailure::Store)?;
        let group = self
            .bind_group(charges.map(|(dimension, _)| dimension))
            .map_err(ImportObservedWorkFailure::Store)?;
        let raw = group
            .raw
            .admit_observed(charges.map(|(_, amount)| amount))
            .map_err(|cause| ImportObservedWorkFailure::Store(self.runtime_error(cause)))?;
        let mut work = ImportObservedExecution {
            raw,
            vocabulary: self.vocabulary,
        };
        match action(self, &mut work) {
            Ok(value) => {
                work.finish().map_err(ImportObservedWorkFailure::Store)?;
                Ok(value)
            }
            Err(cause) => {
                let _ = work.finish_partial();
                Err(ImportObservedWorkFailure::Action(cause))
            }
        }
    }
}

impl<'region, const N: usize> ImportBoundWorkGroup<'region, N> {
    pub(crate) fn admit_observed(
        &self,
        amounts: [u64; N],
    ) -> Result<ImportObservedExecution<'region, N>, ImportStoreHostError> {
        self.raw
            .admit_observed(amounts)
            .map(|raw| ImportObservedExecution {
                raw,
                vocabulary: self.vocabulary,
            })
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }

    pub(crate) fn begin_parser_parent(
        &self,
        amounts: [u64; N],
        limits: [Option<u64>; N],
    ) -> Result<ImportParserWorkScope<'region, N>, ImportParserAdmissionError> {
        self.map_scope(self.raw.begin_parser_parent(amounts, limits))
    }

    pub(crate) fn begin_parser_child(
        &self,
        amounts: [u64; N],
        limits: [Option<u64>; N],
    ) -> Result<ImportParserWorkScope<'region, N>, ImportParserAdmissionError> {
        self.map_scope(self.raw.begin_parser_child(amounts, limits))
    }

    fn map_scope(
        &self,
        result: Result<StoreParserWorkScope<'region, N>, StoreParserWorkError>,
    ) -> Result<ImportParserWorkScope<'region, N>, ImportParserAdmissionError> {
        result
            .map(|raw| ImportParserWorkScope {
                raw,
                vocabulary: self.vocabulary,
            })
            .map_err(|cause| match cause {
                StoreParserWorkError::Runtime(cause) => ImportParserAdmissionError::Store(
                    project_store_runtime_error(self.vocabulary, cause),
                ),
                StoreParserWorkError::ItemLimit {
                    index,
                    value,
                    limit,
                } => ImportParserAdmissionError::ItemLimit {
                    index,
                    value,
                    limit,
                },
            })
    }

    pub(crate) fn charge_attempted(&self, amounts: [u64; N]) -> Result<(), ImportStoreHostError> {
        self.raw
            .charge_attempted(amounts)
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }

    pub(crate) fn complete_direct(&self, amounts: [u64; N]) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete_direct(amounts)
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }

    pub(crate) fn complete_observed(&self, amounts: [u64; N]) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete_observed(amounts)
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }

    pub(crate) fn charge_observed(&self, amounts: [u64; N]) -> Result<(), ImportStoreHostError> {
        self.raw
            .charge_observed(amounts)
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
}

impl<const N: usize> ImportParserWorkScope<'_, N> {
    pub(crate) fn remaining(&self) -> [u64; N] {
        self.raw.remaining()
    }
    pub(crate) fn complete(&mut self, amounts: [u64; N]) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete(amounts)
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
    pub(crate) fn complete_all(&mut self) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete_all()
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
    pub(crate) fn finish_exact(self) -> Result<(), ImportStoreHostError> {
        self.raw
            .finish_exact()
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
    pub(crate) fn finish_partial(self) -> Result<(), ImportStoreHostError> {
        self.raw
            .finish_partial()
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
}

impl ImportParserWorkScope<'_, 1> {
    pub(crate) fn replenish(&mut self, amount: u64) -> Result<(), ImportStoreHostError> {
        self.raw
            .replenish(amount)
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
}

impl<const N: usize> ImportObservedExecution<'_, N> {
    #[cfg(test)]
    pub(crate) fn remaining(&self) -> [u64; N] {
        self.raw.remaining()
    }
    pub(crate) fn complete(&mut self, amounts: [u64; N]) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete(amounts)
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
    pub(crate) fn finish(self) -> Result<(), ImportStoreHostError> {
        self.raw
            .finish()
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
    pub(crate) fn finish_partial(self) -> Result<(), ImportStoreHostError> {
        self.raw
            .finish_partial()
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }
}
