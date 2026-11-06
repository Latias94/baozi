//! Root-only IO/cache/graph work over a short loan of the actual numeric invocation.
use super::*;
use baozi_operation::__retained_store_integration::activation::{
    StoreInvocation, StoreInvocationLayout, StoreParserWorkError,
};

type Numeric<'op> =
    StoreInvocation<'op, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;

pub(crate) struct ImportRootExecution<'loan, 'op> {
    pub(crate) raw: &'loan mut Numeric<'op>,
    vocabulary: &'loan ImportStoreVocabulary,
}

/// Cached membership and completion order, with no work/control authority of its own.
#[derive(Clone, Copy)]
pub(crate) struct ImportRootWorkLayout<'op, const N: usize> {
    raw: StoreInvocationLayout<'op, N>,
}

impl<'loan, 'op> ImportRootExecution<'loan, 'op> {
    pub(crate) fn new(
        raw: &'loan mut Numeric<'op>,
        vocabulary: &'loan ImportStoreVocabulary,
    ) -> Self {
        Self { raw, vocabulary }
    }

    pub(crate) fn reborrow(&mut self) -> ImportRootExecution<'_, 'op> {
        ImportRootExecution::new(self.raw, self.vocabulary)
    }

    fn project(&self, cause: StoreParserWorkError) -> ImportStoreHostError {
        match cause {
            StoreParserWorkError::Runtime(cause) => {
                project_store_runtime_error(self.vocabulary, cause)
            }
            StoreParserWorkError::ItemLimit { .. } => ImportStoreHostError::Contract {
                code: "parse-work-item-limit-lane-invalid",
            },
        }
    }

    pub(crate) fn poll(&mut self) -> Result<(), ImportStoreHostError> {
        self.raw.checkpoint().map_err(|cause| self.project(cause))
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
            .map_err(|cause| self.project(cause))
    }

    pub(crate) fn bind_group<const N: usize>(
        &mut self,
        dimensions: [ImportDimension; N],
    ) -> Result<ImportRootWorkLayout<'op, N>, ImportStoreHostError> {
        self.raw
            .bind_group(dimensions.map(|dimension| self.vocabulary.dimension(dimension)))
            .map(|raw| ImportRootWorkLayout { raw })
            .map_err(|cause| self.project(cause))
    }

    pub(crate) fn charge_attempted_group<const N: usize>(
        &mut self,
        layout: &ImportRootWorkLayout<'op, N>,
        amounts: [u64; N],
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .charge_attempted_bound(layout.raw, amounts)
            .map_err(|cause| self.project(cause))
    }

    pub(crate) fn complete_observed_group<const N: usize>(
        &mut self,
        layout: &ImportRootWorkLayout<'op, N>,
        amounts: [u64; N],
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete_observed_bound(layout.raw, amounts)
            .map_err(|cause| self.project(cause))
    }

    pub(crate) fn complete_direct_group<const N: usize>(
        &mut self,
        layout: &ImportRootWorkLayout<'op, N>,
        amounts: [u64; N],
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete_direct_bound(layout.raw, amounts)
            .map_err(|cause| self.project(cause))
    }

    pub(crate) fn charge_attempted(
        &mut self,
        dimension: ImportDimension,
        amount: u64,
    ) -> Result<(), ImportStoreHostError> {
        let layout = self.bind_group([dimension])?;
        self.charge_attempted_group(&layout, [amount])
    }

    pub(crate) fn charge_consumed(
        &mut self,
        dimension: ImportDimension,
        amount: u64,
    ) -> Result<(), ImportStoreHostError> {
        let layout = self.bind_group([dimension])?;
        self.complete_observed_group(&layout, [amount])
    }

    pub(crate) fn charge_consumed_batch<const N: usize>(
        &mut self,
        charges: &[(ImportDimension, u64); N],
    ) -> Result<(), ImportStoreHostError> {
        let layout = self.bind_group(charges.map(|(dimension, _)| dimension))?;
        self.complete_direct_group(&layout, charges.map(|(_, amount)| amount))
    }

    /// A returned outer error leaves the attempted chunk unconsumed; the original error escapes.
    pub(crate) fn run_fallible_action<T, E, const N: usize>(
        &mut self,
        layout: &ImportRootWorkLayout<'op, N>,
        amounts: [u64; N],
        action: impl FnOnce() -> Result<T, E>,
    ) -> Result<Result<T, E>, ImportStoreHostError> {
        self.poll()?;
        self.raw
            .run_observed_fallible_action(layout.raw, amounts, action)
            .map_err(|cause| self.project(cause))
    }

    pub(crate) fn dimension_state(
        &self,
        dimension: ImportDimension,
    ) -> Result<RetainedStoreDimensionState, ImportStoreHostError> {
        self.raw
            .dimension_state(self.vocabulary.dimension(dimension))
            .map_err(|cause| self.project(cause))
    }

    #[cfg(test)]
    pub(crate) fn observation_live_bytes(
        &self,
        observation: &baozi_operation::__retained_store_integration::authority::StoreStorageObservation<'op>,
    ) -> u64 {
        observation.live_bytes(self.raw)
    }

    #[cfg(test)]
    pub(crate) fn storage_observation(
        &self,
        dimension: ImportDimension,
    ) -> baozi_operation::__retained_store_integration::authority::StoreStorageObservation<'op>
    {
        self.raw
            .storage_observation(self.vocabulary.dimension(dimension))
            .expect("the test observes an admitted storage dimension")
    }

    #[cfg(test)]
    pub(crate) fn validate_live_owner(
        &mut self,
        reservation: &ImportStoreReservation,
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .services()
            .validate_live_owner(reservation.raw.as_ref().expect("live reservation"))
            .map_err(|cause| self.project(cause))
    }

    #[cfg(test)]
    pub(crate) fn transition(
        &mut self,
        reservation: &mut ImportStoreReservation,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .services()
            .transition(
                reservation.raw.as_mut().expect("live reservation"),
                self.vocabulary.phase(phase),
                self.vocabulary.dimension(dimension),
                bytes,
            )
            .map_err(|cause| self.project(cause))?;
        reservation.bytes = bytes;
        Ok(())
    }

    pub(crate) fn tighten_dimension_policy(
        &mut self,
        dimension: ImportDimension,
        policy: DimensionPolicy,
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .tighten_dimension_policy(self.vocabulary.dimension(dimension), policy)
            .map_err(|cause| self.project(cause))
    }

    pub(crate) fn reserve(
        &mut self,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ImportStoreHostError> {
        self.raw
            .ordinary_storage()
            .reserve(
                self.vocabulary.phase(phase),
                self.vocabulary.dimension(dimension),
                bytes,
            )
            .map(|raw| ImportStoreReservation {
                raw: Some(raw),
                bytes,
            })
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }

    pub(crate) fn reserve_retained(
        &mut self,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
        allocations: u64,
    ) -> Result<ImportStoreReservation, ImportStoreHostError> {
        self.raw
            .services()
            .reserve_retained(
                self.vocabulary.phase(phase),
                self.vocabulary.dimension(dimension),
                bytes,
                allocations,
            )
            .map(|raw| ImportStoreReservation {
                raw: Some(raw),
                bytes,
            })
            .map_err(|cause| self.project(cause))
    }

    pub(crate) fn resize(
        &mut self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.raw
            .services()
            .resize(
                reservation
                    .raw
                    .as_mut()
                    .expect("a live import reservation owns its backing"),
                bytes,
            )
            .map_err(|cause| self.project(cause))?;
        reservation.bytes = bytes;
        Ok(())
    }
}
