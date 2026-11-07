use super::*;
use crate::retained::RetainedFailureWorkControl;

pub(in crate::concrete_store) struct StoreFailureEvidenceLoan<'op, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    publication: StorePublicationLoan<'op, O, PP, U, ()>,
}

impl<'op, O, PP, U> StoreFailureEvidenceLoan<'op, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn new(
        raw: RawStoreAssemblyBorrow<'op, O, PP, U>,
        domain: &'op StoreKeyDomain,
        state: &'op mut StorePublicationState,
    ) -> Self {
        Self {
            publication: StorePublicationLoan {
                raw,
                domain,
                state,
                lineage: PhantomData,
            },
        }
    }

    pub(in crate::concrete_store) fn publication_impl(
        &mut self,
    ) -> &mut StorePublicationLoan<'op, O, PP, U, ()> {
        &mut self.publication
    }

    pub(in crate::concrete_store) fn try_retain_protected<Kind, Tag>(
        &mut self,
        allocation: RetainedAllocationPlan,
        build: impl for<'placement, 'control> FnOnce(
            BlockEmplacer<'placement, StoredBlock>,
            RetainedFailureWorkControl<'control, O, StorePhaseKey, PP, U, StoreDimensionKey>,
        ) -> Result<
            BlockPlacement<'placement, StoredBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<OutcomeEvidenceStoreRef<Kind, Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.publication.ensure_evidence_writable()?;
        let result = self
            .publication
            .raw
            .loan_mut()
            .try_retain_outcome_evidence_controlled(allocation, build);
        self.publication
            .map_action(result)
            .map(|raw| OutcomeEvidenceStoreRef {
                raw,
                marker: PhantomData,
            })
    }
}
