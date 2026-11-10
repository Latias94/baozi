//! A short open borrows original slots; the concrete transition binds them after it ends.

use super::super::{
    RetainedStoreBlockTablePlan, RetainedStoreOperation, RetainedStoreProducerHost,
    RetainedStorePublicationPlan, StoreFallbackEnvelope, map_writer_error,
};
use super::*;
use crate::retained::{
    FinalizationContainmentError, RetainedAssemblyOpened, RetainedOperationLoan,
    RetainedPlanningBaseline, RetainedPublicationAssemblyBorrow, RetainedPublicationBorrowFailure,
};

type RawOperationLoan<'op, O, PP, U> = RetainedOperationLoan<
    'op,
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;

pub(in crate::concrete_store) struct StoreOperationLoan<'op, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) raw: RawOperationLoan<'op, O, PP, U>,
    pub(in crate::concrete_store) domain: &'op StoreKeyDomain,
    lineage: PhantomData<&'op L>,
}

pub(in crate::concrete_store) enum StorePublicationOpenDecision<'op, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Opened(StorePublicationLoan<'op, O, PP, U, L>),
    Failed {
        operation: StoreOperationLoan<'op, O, PP, U, L>,
        state: &'op mut StorePublicationState,
        failure: RetainedPublicationBorrowFailure<StoreDimensionKey>,
    },
}

pub(in crate::concrete_store) enum StorePublicationOpenDisposition {
    Opened(RetainedAssemblyOpened),
    Failed(RetainedPublicationBorrowFailure<StoreDimensionKey>),
}

impl<'op, O, PP, U, L> StoreOperationLoan<'op, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn from_raw(
        raw: RawOperationLoan<'op, O, PP, U>,
        domain: &'op StoreKeyDomain,
    ) -> Self {
        Self {
            raw,
            domain,
            lineage: PhantomData,
        }
    }

    pub(in crate::concrete_store) fn contain_fault(self) -> FinalizationContainmentError {
        self.raw.contain_activation_fault()
    }

    pub(in crate::concrete_store) fn planning_baseline(&self) -> RetainedPlanningBaseline {
        self.raw.planning_baseline()
    }

    pub(in crate::concrete_store) fn planning_matches_baseline(
        &self,
        baseline: RetainedPlanningBaseline,
    ) -> bool {
        self.raw.planning_matches_baseline(baseline)
    }

    pub(in crate::concrete_store) fn invocation_identity(&self) -> *const () {
        self.raw.invocation_identity()
    }

    pub(in crate::concrete_store) fn activation_terminal_cause(
        &self,
    ) -> Option<PreparationFailureCode> {
        self.raw.sticky_terminal_cause()
    }

    pub(in crate::concrete_store) fn latch_activation_terminal_cause(
        &mut self,
        cause: PreparationFailureCode,
    ) {
        self.raw.record_terminal_cause(cause);
    }

    pub(in crate::concrete_store) fn poll_terminal_control(&mut self) {
        self.raw.poll_terminal_control();
    }

    pub(in crate::concrete_store) fn control_checkpoint(
        &mut self,
    ) -> RetainedStoreControlCheckpoint<'_, O, PP, U> {
        let terminal_only = self.raw.is_terminal_only();
        RetainedStoreControlCheckpoint {
            raw: self.raw.control_checkpoint(),
            domain: self.domain,
            terminal_only,
        }
    }

    pub(in crate::concrete_store) fn producer_host(
        &mut self,
    ) -> RetainedStoreProducerHost<'_, O, PP, U> {
        let terminal_only = self.raw.is_terminal_only();
        RetainedStoreProducerHost {
            raw: self.raw.producer_host(),
            domain: self.domain,
            terminal_only,
        }
    }

    pub(in crate::concrete_store) fn rebind_activation_canonical_assembly(
        &mut self,
        plan: RetainedStoreBlockTablePlan,
    ) -> Result<(), StoreRuntimeError> {
        let writer = plan.writer();
        let (phase, dimensions) = writer.keys_for_publication_validation();
        self.validate_writer_keys(phase, dimensions)?;
        self.raw
            .try_rebind_canonical_assembly(writer.into_raw())
            .map_err(|cause| map_writer_error(&cause))
    }

    pub(in crate::concrete_store) fn validate_publication_plan_keys(
        &self,
        plan: &RetainedStorePublicationPlan,
    ) -> Result<(), StoreRuntimeError> {
        for table in plan.tables_for_validation().into_iter().flatten() {
            let (phase, dimensions) = table.keys_for_publication_validation();
            self.validate_writer_keys(phase, dimensions)?;
        }
        Ok(())
    }

    pub(in crate::concrete_store) fn validate_writer_keys(
        &self,
        phase: StorePhaseKey,
        dimensions: [StoreDimensionKey; 2],
    ) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        for dimension in dimensions {
            if !self.domain.owns_dimension(dimension) {
                return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
            }
        }
        if !self.raw.contains_phase(phase)
            || dimensions
                .into_iter()
                .any(|dimension| !self.raw.contains_dimension(dimension))
        {
            return Err(StoreRuntimeError::InvalidPublicationPlan);
        }
        Ok(())
    }

    pub(in crate::concrete_store) fn validate_activation_phase(
        &self,
        phase: StorePhaseKey,
    ) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        if !self.raw.contains_phase(phase) {
            return Err(StoreRuntimeError::InvalidPublicationPlan);
        }
        Ok(())
    }

    pub(in crate::concrete_store) fn validate_activation_dimensions<const N: usize>(
        &self,
        dimensions: [StoreDimensionKey; N],
    ) -> Result<(), StoreRuntimeError> {
        for dimension in dimensions {
            if !self.domain.owns_dimension(dimension) {
                return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
            }
            if !self.raw.contains_dimension(dimension) {
                return Err(StoreRuntimeError::InvalidPublicationPlan);
            }
        }
        Ok(())
    }

    pub(in crate::concrete_store) fn open_publication_loan(
        self,
        plan: RetainedStorePublicationPlan,
        state: &'op mut StorePublicationState,
    ) -> StorePublicationOpenDecision<'op, O, PP, U, L> {
        match self.raw.try_publication_assembly(plan.into_raw()) {
            Ok(raw) => StorePublicationOpenDecision::Opened(StorePublicationLoan {
                raw: RetainedPublicationAssemblyBorrow::from_loan(raw),
                domain: self.domain,
                state,
                lineage: self.lineage,
            }),
            Err(error) => {
                let (raw, failure) = error.into_borrowed_failure();
                StorePublicationOpenDecision::Failed {
                    operation: Self {
                        raw,
                        domain: self.domain,
                        lineage: self.lineage,
                    },
                    state,
                    failure,
                }
            }
        }
    }
}

impl<O, PP, U, L> StorePublicationLoan<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn contain_fault(&mut self) -> FinalizationContainmentError {
        self.raw.loan_mut().contain_activation_fault()
    }

    pub(in crate::concrete_store) fn prepare_rejection(
        &mut self,
        cause: PreparationFailureCode,
    ) -> Result<(), FinalizationContainmentError> {
        self.raw
            .loan_mut()
            .try_discard_canonical_for_rejection(cause)
    }
}

impl<O, PP, U, L> StorePublicationOpenDecision<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn into_disposition(
        self,
    ) -> Result<StorePublicationOpenDisposition, StoreRuntimeError> {
        match self {
            Self::Opened(ordinary) => ordinary
                .raw
                .opened_stamp()
                .map(StorePublicationOpenDisposition::Opened)
                .ok_or(StoreRuntimeError::ConstructionInvariantUnverified),
            Self::Failed {
                operation: _,
                state: _,
                failure,
            } => Ok(StorePublicationOpenDisposition::Failed(failure)),
        }
    }
}

impl<O, PP, U, L> RetainedStoreOperation<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn ordinary_operation(
        &mut self,
    ) -> StoreOperationLoan<'_, O, PP, U, L> {
        StoreOperationLoan::from_raw(self.raw.loan_mut().reborrow(), &self.domain)
    }
}
