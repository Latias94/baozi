//! Short role loans over the actual dynamic or finite fallback batch owners.

use super::*;

pub(crate) struct FallbackBatchReadView<'loan, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) canonical_assembly: &'loan AttributionReservationBatch<O, P, D, CanonicalOutput>,
    pub(crate) evidence_assembly:
        &'loan AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    pub(crate) fallback: &'loan AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    pub(crate) numerical_evidence:
        Option<&'loan AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
}

pub(crate) struct FallbackBatchView<'loan, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) canonical_assembly: &'loan mut AttributionReservationBatch<O, P, D, CanonicalOutput>,
    pub(crate) evidence_assembly:
        &'loan mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    pub(crate) fallback:
        &'loan mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    pub(crate) numerical_evidence:
        Option<&'loan mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
}

type CanonicalAssemblyReplacement<O, P, D> = Result<
    AttributionReservationBatch<O, P, D, CanonicalOutput>,
    AttributionReservationBatch<O, P, D, CanonicalOutput>,
>;

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> FallbackBatchView<'_, O, P, D> {
    pub(crate) fn reborrow(&mut self) -> FallbackBatchView<'_, O, P, D> {
        FallbackBatchView {
            canonical_assembly: &mut *self.canonical_assembly,
            evidence_assembly: &mut *self.evidence_assembly,
            fallback: &mut *self.fallback,
            numerical_evidence: self.numerical_evidence.as_deref_mut(),
        }
    }

    pub(crate) fn as_read(&self) -> FallbackBatchReadView<'_, O, P, D> {
        FallbackBatchReadView {
            canonical_assembly: self.canonical_assembly,
            evidence_assembly: self.evidence_assembly,
            fallback: self.fallback,
            numerical_evidence: self.numerical_evidence.as_deref(),
        }
    }

    pub(crate) fn replace_empty_canonical_assembly(
        &mut self,
        replacement: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    ) -> CanonicalAssemblyReplacement<O, P, D> {
        if !self.canonical_assembly.is_empty_storage_shell()
            || !replacement.is_empty_storage_shell()
        {
            return Err(replacement);
        }
        Ok(std::mem::replace(self.canonical_assembly, replacement))
    }

    pub(super) fn try_prepare_fallback_sources(self) -> Result<(), ()> {
        if !self
            .evidence_assembly
            .can_absorb_sources([self.numerical_evidence.as_deref(), Some(self.fallback)])
        {
            return Err(());
        }
        if let Some(evidence) = self.numerical_evidence {
            self.evidence_assembly
                .absorb_proven_retaining_source_storage(evidence);
        }
        self.evidence_assembly
            .absorb_proven_retaining_source_storage(self.fallback);
        Ok(())
    }
}
