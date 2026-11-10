//! Shared freeze and payload policy borrows actual fields; concrete callers own every exit.

use super::*;
use crate::ledger::DimensionSnapshot;
use crate::retained::{RetainedSuccessClosureProof, RetainedSuccessFreezeDecision};

pub(super) fn issue_freeze<O, PP, U>(
    assembly: &mut super::super::RawStoreAssemblyBorrow<'_, O, PP, U>,
    proof: &RetainedSuccessClosureProof<StoredBlock, StoredBlock>,
    destinations: &ZeroCommitDestinationFacts,
    progress: FinalProgress<PP, U>,
) -> Result<RetainedSuccessFreezeDecision, FinalizationContainmentError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly.loan_mut().try_issue_freeze_decision(
        proof,
        destinations._canonical_dimension,
        destinations._outcome_evidence_dimension,
        progress,
    )
}

pub(super) fn dispose_pair<T, Proof>(
    output: &mut Option<T>,
    proof: &mut Option<Proof>,
    order: payload_owner::PayloadOrder,
) -> Result<(), ()> {
    let (first, second) = match order {
        payload_owner::PayloadOrder::ProofBeforeOutput => {
            let proof = drop_scoped_value(proof.take());
            let output = drop_scoped_value(output.take());
            (proof, output)
        }
        payload_owner::PayloadOrder::OutputBeforeProof => {
            let output = drop_scoped_value(output.take());
            let proof = drop_scoped_value(proof.take());
            (output, proof)
        }
    };
    first.and(second)
}

pub(super) fn dispose_generation_inputs<Generation, Reject, Prepare>(
    (generation, reject, prepare): (Generation, Reject, Prepare),
) {
    let _ = drop_scoped_value(generation);
    let _ = drop_scoped_value(reject);
    let _ = drop_scoped_value(prepare);
}

pub(super) fn destinations_are_zero(
    canonical: DimensionSnapshot,
    evidence: DimensionSnapshot,
) -> bool {
    [canonical, evidence].into_iter().all(|snapshot| {
        snapshot.pending_publication() == 0
            && snapshot.committed() == 0
            && snapshot.committed_canonical_output() == 0
            && snapshot.published_outcome_evidence() == 0
    })
}
