//! Short loans over the actual dynamic or finite fallback publication owners.

use super::*;
use crate::ledger::FallbackBatchReadView;

pub(in crate::retained) enum FallbackPublicationBaselineLoan<'inv, O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Dynamic(&'inv mut FallbackPublicationBaseline<O, P, D, F>),
    Finite(&'inv mut FiniteFallbackPublicationBaseline<O, P, D, F>),
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord, F>
    FallbackPublicationBaselineLoan<'_, O, P, D, F>
{
    pub(in crate::retained) const fn is_finite(&self) -> bool {
        matches!(self, Self::Finite(_))
    }

    pub(in crate::retained) fn account(&self) -> &PhysicalStorageAccount {
        match self {
            Self::Dynamic(baseline) => baseline.account(),
            Self::Finite(baseline) => baseline.account(),
        }
    }

    pub(in crate::retained) fn first_account(&self) -> Option<&PhysicalStorageAccount> {
        match self {
            Self::Dynamic(baseline) => baseline.first_account(),
            Self::Finite(baseline) => baseline.first_account(),
        }
    }

    pub(in crate::retained) fn has_batches(&self) -> bool {
        match self {
            Self::Dynamic(baseline) => baseline.with_batches(|_| ()).is_some(),
            Self::Finite(baseline) => baseline.with_batches(|_| ()).is_some(),
        }
    }

    pub(in crate::retained) fn can_take_physical(&self) -> bool {
        match self {
            Self::Dynamic(baseline) => baseline.physical_parts_are_installed(),
            Self::Finite(baseline) => baseline.physical_parts_are_installed(),
        }
    }

    pub(in crate::retained) fn batch_read(&self) -> Option<FallbackBatchReadView<'_, O, P, D>> {
        match self {
            Self::Dynamic(baseline) => baseline.batch_read(),
            Self::Finite(baseline) => baseline.batch_read(),
        }
    }

    pub(in crate::retained) fn record_forgotten_physical_edges(&self) {
        match self {
            Self::Dynamic(baseline) => baseline.record_forgotten_physical_edges(),
            Self::Finite(baseline) => baseline.record_forgotten_physical_edges(),
        }
    }

    pub(in crate::retained) fn fail_closed_after_recorded_poison(&mut self) {
        match self {
            Self::Dynamic(baseline) => baseline.fail_closed_after_recorded_poison(),
            Self::Finite(baseline) => baseline.fail_closed_after_recorded_poison(),
        }
    }

    pub(in crate::retained) fn try_replace_canonical_assembly(
        &mut self,
        replacement: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    ) -> Result<(), RetainedWriterError<D>> {
        match self {
            Self::Dynamic(baseline) => baseline.try_replace_canonical_assembly(replacement),
            Self::Finite(baseline) => baseline.try_replace_canonical_assembly(replacement),
        }
    }

    #[cfg(test)]
    pub(in crate::retained) fn test_evidence_allocation(
        &self,
    ) -> &RetainedAllocation<TerminalLedgerEvidence<P, D>> {
        match self {
            Self::Dynamic(baseline) => baseline.test_evidence_allocation(),
            Self::Finite(_) => {
                panic!("the dynamic evidence observation requires its original dynamic owner")
            }
        }
    }

    #[cfg(test)]
    pub(in crate::retained) fn test_batch_identity(&self) -> Option<*const ()> {
        match self {
            Self::Dynamic(baseline) => {
                baseline.with_batches(|batches| std::ptr::from_ref(batches).cast::<()>())
            }
            Self::Finite(baseline) => {
                baseline.with_batches(|batches| std::ptr::from_ref(batches).cast::<()>())
            }
        }
    }
}
