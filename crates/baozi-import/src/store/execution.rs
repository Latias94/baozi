//! Import-semantic views over the existing invocation's disjoint execution capabilities.

#![allow(
    clippy::result_large_err,
    reason = "import failures preserve exact inline ledger and control evidence"
)]

use baozi_operation::__retained_store_integration::activation::{
    StoreFailureStorage, StoreInvocation, StoreInvocationServices, StoreOrdinaryStorage,
    StoreParserWorkError,
};

use super::*;

pub(crate) struct ImportStoreServices<'loan, 'op> {
    raw: StoreInvocationServices<
        'loan,
        'op,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
    >,
    vocabulary: &'loan ImportStoreVocabulary,
}

impl<'loan, 'op> ImportStoreServices<'loan, 'op> {
    pub(crate) fn new(
        raw: StoreInvocationServices<
            'loan,
            'op,
            ImportOperationOwner,
            ImportProgressPhase,
            ImportProgressUnit,
        >,
        vocabulary: &'loan ImportStoreVocabulary,
    ) -> Self {
        Self { raw, vocabulary }
    }

    fn runtime_error(&self, cause: StoreParserWorkError) -> ImportStoreHostError {
        match cause {
            StoreParserWorkError::Runtime(cause) => {
                project_store_runtime_error(self.vocabulary, cause)
            }
            StoreParserWorkError::ItemLimit { .. } => ImportStoreHostError::Contract {
                code: "parse-work-item-limit-lane-invalid",
            },
        }
    }

    pub(crate) fn ordinary(&mut self) -> ImportOrdinaryStorage<'_, 'op> {
        ImportOrdinaryStorage::new(self.raw.ordinary_storage(), self.vocabulary)
    }

    pub(crate) fn poll(&mut self) -> Result<(), ImportStoreHostError> {
        self.raw
            .checkpoint()
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn report_progress(
        &mut self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn reserve(
        &mut self,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ImportStoreHostError> {
        self.ordinary().reserve(phase, dimension, bytes)
    }

    pub(crate) fn reserve_retained(
        &mut self,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
        allocations: u64,
    ) -> Result<ImportStoreReservation, ImportStoreHostError> {
        let raw = self
            .raw
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
        self.raw
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

    pub(crate) fn charge_attempted(
        &mut self,
        dimension: ImportDimension,
        amount: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .charge_attempted([self.vocabulary.dimension(dimension)], [amount])
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn charge_consumed(
        &mut self,
        dimension: ImportDimension,
        amount: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete_observed([self.vocabulary.dimension(dimension)], [amount])
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn charge_observed_batch<const N: usize>(
        &mut self,
        charges: &[(ImportDimension, u64); N],
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .charge_observed(
                charges.map(|(dimension, _)| self.vocabulary.dimension(dimension)),
                charges.map(|(_, amount)| amount),
            )
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn charge_consumed_batch<const N: usize>(
        &mut self,
        charges: &[(ImportDimension, u64); N],
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete_direct(
                charges.map(|(dimension, _)| self.vocabulary.dimension(dimension)),
                charges.map(|(_, amount)| amount),
            )
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn failure_storage(
        &mut self,
    ) -> (
        StoreFailureStorage<'_, 'op, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>,
        &ImportStoreVocabulary,
    ) {
        (self.raw.failure_storage(), self.vocabulary)
    }
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

impl ImportStoreSessionOwner<'_> {
    pub(crate) fn with_root_execution<R, E: From<ImportStoreHostError>>(
        &mut self,
        action: impl for<'op> FnOnce(&mut super::ImportRootExecution<'_, 'op>) -> Result<R, E>,
    ) -> Result<R, E> {
        let data = &mut self.storage[0];
        let ImportStoreSessionData {
            state,
            vocabulary,
            importer_boundary,
            ..
        } = data;
        let ImportStoreSessionState::Dormant(owner) = state else {
            return Err(inactive_store_state_error(state).into());
        };
        owner
            .with_root_execution(importer_boundary, |numeric| {
                action(&mut super::ImportRootExecution::new(numeric, vocabulary))
                    .map_err(ImportExecutionFailure::Owned)
            })
            .map_err(|cause| match cause {
                ImportExecutionFailure::Owned(cause) => cause,
                ImportExecutionFailure::Runtime(StoreParserWorkError::Runtime(cause)) => {
                    project_store_runtime_error(vocabulary, cause).into()
                }
                ImportExecutionFailure::Runtime(StoreParserWorkError::ItemLimit { .. }) => {
                    ImportStoreHostError::Contract {
                        code: "parse-work-item-limit-lane-invalid",
                    }
                    .into()
                }
            })
    }

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
}
