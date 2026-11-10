//! Concrete dynamic terminal ownership after every qualification loan has ended.

use super::terminal_loan::TerminalEvidenceQualification;
use super::*;

pub(super) enum TerminalDisposition {
    Success {
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
        set_id: RetainedSetId,
    },
    RichFailure {
        evidence_root: RetainedSlotIdentity,
        set_id: RetainedSetId,
        cause: PreparationFailureCode,
    },
    MinimalFallback {
        cause: PreparationFailureCode,
    },
    EvidenceOnlySuccess,
}

/// Internal metadata handoff. The original root installs it before issuing the ticket.
pub(crate) struct DeferredAttachedTerminal {
    pub(super) stamp: AttachedTerminalStamp,
    pub(super) qualification: TerminalEvidenceQualification,
    pub(super) disposition: TerminalDisposition,
}

/// Owns the original physical graph and detached numerical cover as one cleanup boundary.
pub(super) struct DynamicTerminalCapsule<O, P, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) physical: Option<TerminalPhysicalState<P, D, F, CB, EB>>,
    pub(super) proof: Option<crate::ledger::DetachedTypedFreezeProof<O, P, D>>,
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord, F, CB, EB>
    DynamicTerminalCapsule<O, P, D, F, CB, EB>
{
    pub(super) fn new(
        physical: TerminalPhysicalState<P, D, F, CB, EB>,
        proof: crate::ledger::DetachedTypedFreezeProof<O, P, D>,
    ) -> Self {
        Self {
            physical: Some(physical),
            proof: Some(proof),
        }
    }

    pub(super) fn disposition(&self) -> &TerminalDisposition {
        self.physical
            .as_ref()
            .expect("the original terminal graph is installed")
            .disposition
            .as_ref()
            .expect("the original terminal decision is installed")
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord, F, CB, EB> Drop
    for DynamicTerminalCapsule<O, P, D, F, CB, EB>
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
    }
}
