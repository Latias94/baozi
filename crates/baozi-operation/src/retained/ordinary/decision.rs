//! Private closure decisions end their short borrow before binding the original concrete owner.

use super::*;

enum ClosureDecision<CB, EB> {
    Success(RetainedClosureValidation<RetainedSuccessClosureProof<CB, EB>>),
    Evidence(RetainedClosureValidation<RetainedEvidenceClosureProof<EB>>),
}

/// Ownerless and lifetime-free; its proof fields and constructor remain inside retained storage.
pub(crate) struct RetainedAssemblyDecision<CB, EB> {
    set_id: RetainedSetId,
    closure: ClosureDecision<CB, EB>,
}

impl<CB, EB> RetainedAssemblyDecision<CB, EB> {
    fn belongs_to<O, P, PP, U, D, F>(
        &self,
        assembly: &AssemblyLoan<'_, O, P, PP, U, D, F, CB, EB>,
    ) -> bool
    where
        O: OperationOwner,
        P: Copy + Ord,
        PP: ProgressPhase,
        U: ProgressUnit,
        D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
        F: Send + Sync + 'static,
        CB: Send + Sync + 'static,
        EB: Send + Sync + 'static,
    {
        if self.set_id != assembly.fields.set_id {
            return false;
        }
        match &self.closure {
            ClosureDecision::Success(RetainedClosureValidation::Valid(proof)) => {
                assembly.success_proof_is_current(proof)
            }
            ClosureDecision::Evidence(RetainedClosureValidation::Valid(proof)) => {
                assembly.evidence_proof_is_current(proof)
            }
            _ => true,
        }
    }
}

type SuccessProofBindingResult<CB, EB> = Result<
    RetainedClosureValidation<RetainedSuccessClosureProof<CB, EB>>,
    RetainedAssemblyDecision<CB, EB>,
>;
type EvidenceProofBindingResult<CB, EB> = Result<
    RetainedClosureValidation<RetainedEvidenceClosureProof<EB>>,
    RetainedAssemblyDecision<CB, EB>,
>;

impl<O, P, PP, U, D, F, CB, EB> AssemblyLoan<'_, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    pub(in crate::retained) fn success_proof_is_current(
        &self,
        proof: &RetainedSuccessClosureProof<CB, EB>,
    ) -> bool {
        self.fields.mode == RetainedOutcomeMode::SuccessCapable
            && self.fields.containment == AssemblyContainmentProvenance::Healthy
            && !self.fields.success_outcome_rejected
            && self.fields.failure_terminal_work_state != FailureTerminalWorkState::Failed
            && proof.set_id == self.fields.set_id
            && self.fields.canonical_table.as_ref().is_some_and(|table| {
                table.as_ref().member_count() == proof.canonical_count
                    && table
                        .as_ref()
                        .member(proof.canonical_root, proof.set_id)
                        .is_some()
            })
            && self.fields.evidence_table.as_ref().is_some_and(|table| {
                table.as_ref().member_count() == proof.evidence_count
                    && table
                        .as_ref()
                        .member(proof.evidence_root, proof.set_id)
                        .is_some()
            })
    }

    fn evidence_proof_is_current(&self, proof: &RetainedEvidenceClosureProof<EB>) -> bool {
        self.fields.containment == AssemblyContainmentProvenance::Healthy
            && self.fields.failure_terminal_work_state != FailureTerminalWorkState::Failed
            && proof.set_id == self.fields.set_id
            && self.fields.evidence_table.as_ref().is_some_and(|table| {
                table.as_ref().member_count() == proof.evidence_count
                    && table
                        .as_ref()
                        .member(proof.evidence_root, proof.set_id)
                        .is_some()
            })
    }

    pub(crate) fn bind_success_proof(
        &self,
        decision: RetainedAssemblyDecision<CB, EB>,
    ) -> SuccessProofBindingResult<CB, EB> {
        if !matches!(&decision.closure, ClosureDecision::Success(_)) || !decision.belongs_to(self) {
            return Err(decision);
        }
        let ClosureDecision::Success(validation) = decision.closure else {
            unreachable!("the original decision kind was checked without intervening effects");
        };
        Ok(validation)
    }

    pub(crate) fn bind_evidence_proof(
        &self,
        decision: RetainedAssemblyDecision<CB, EB>,
    ) -> EvidenceProofBindingResult<CB, EB> {
        if !matches!(&decision.closure, ClosureDecision::Evidence(_)) || !decision.belongs_to(self)
        {
            return Err(decision);
        }
        let ClosureDecision::Evidence(validation) = decision.closure else {
            unreachable!("the original decision kind was checked without intervening effects");
        };
        Ok(validation)
    }
}

impl<O, P, PP, U, D, F, CB, EB> RetainedPublicationAssemblyBorrow<'_, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    pub(crate) fn validate_success_decision<CT, ET>(
        &mut self,
        canonical: CanonicalRetainedSlot<CB, CT>,
        evidence: OutcomeEvidenceRetainedSlot<EB, ET>,
        validate: impl for<'a> FnOnce(
            &mut RetainedClosureSession<'a, CB, EB>,
            CanonicalRetainedSlot<CB, CT>,
            OutcomeEvidenceRetainedSlot<EB, ET>,
        ) -> bool,
    ) -> RetainedAssemblyDecision<CB, EB>
    where
        CT: Copy + Send + Sync + 'static,
        ET: Copy + Send + Sync + 'static,
    {
        let set_id = self.loan.fields.set_id;
        let closure =
            ClosureDecision::Success(self.loan.validate_success(canonical, evidence, validate));
        RetainedAssemblyDecision { set_id, closure }
    }

    pub(crate) fn validate_success_controlled_decision<CT, ET>(
        &mut self,
        work_dimension: D,
        scratch_dimension: D,
        canonical: CanonicalRetainedSlot<CB, CT>,
        evidence: OutcomeEvidenceRetainedSlot<EB, ET>,
        validate: impl for<'session, 'control> FnOnce(
            &mut RetainedClosureSession<'session, CB, EB>,
            CanonicalRetainedSlot<CB, CT>,
            OutcomeEvidenceRetainedSlot<EB, ET>,
            RetainedWorkControl<'control, O, P, PP, U, D>,
        ) -> bool,
    ) -> RetainedAssemblyDecision<CB, EB>
    where
        CT: Copy + Send + Sync + 'static,
        ET: Copy + Send + Sync + 'static,
    {
        let set_id = self.loan.fields.set_id;
        let closure = ClosureDecision::Success(self.loan.validate_success_controlled(
            work_dimension,
            scratch_dimension,
            canonical,
            evidence,
            validate,
        ));
        RetainedAssemblyDecision { set_id, closure }
    }

    pub(crate) fn validate_evidence_decision<ET>(
        &mut self,
        evidence: OutcomeEvidenceRetainedSlot<EB, ET>,
        validate: impl for<'a> FnOnce(
            &mut RetainedEvidenceClosureSession<'a, EB>,
            OutcomeEvidenceRetainedSlot<EB, ET>,
        ) -> bool,
    ) -> RetainedAssemblyDecision<CB, EB>
    where
        ET: Copy + Send + Sync + 'static,
    {
        let set_id = self.loan.fields.set_id;
        let closure =
            ClosureDecision::Evidence(self.loan.validate_rich_failure(evidence, validate));
        RetainedAssemblyDecision { set_id, closure }
    }

    pub(crate) fn validate_evidence_controlled_decision<ET>(
        &mut self,
        evidence: OutcomeEvidenceRetainedSlot<EB, ET>,
        validate: impl for<'session, 'control> FnOnce(
            &mut RetainedEvidenceClosureSession<'session, EB>,
            OutcomeEvidenceRetainedSlot<EB, ET>,
            RetainedFailureWorkControl<'control, O, P, PP, U, D>,
        ) -> bool,
    ) -> RetainedAssemblyDecision<CB, EB>
    where
        ET: Copy + Send + Sync + 'static,
    {
        let set_id = self.loan.fields.set_id;
        let closure = ClosureDecision::Evidence(
            self.loan
                .validate_rich_failure_controlled(evidence, validate),
        );
        RetainedAssemblyDecision { set_id, closure }
    }
}

type SuccessBindingResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    RetainedSuccessValidation<'inv, O, P, PP, U, D, F, CB, EB>,
    (
        RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
        RetainedAssemblyDecision<CB, EB>,
    ),
>;
type EvidenceBindingResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    RetainedFailureValidation<'inv, O, P, PP, U, D, F, CB, EB>,
    (
        RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
        RetainedAssemblyDecision<CB, EB>,
    ),
>;

impl<'inv, O, P, PP, U, D, F, CB, EB> RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    pub(crate) fn bind_success_decision(
        self,
        decision: RetainedAssemblyDecision<CB, EB>,
    ) -> SuccessBindingResult<'inv, O, P, PP, U, D, F, CB, EB> {
        let validation = match self.loan.bind_success_proof(decision) {
            Ok(validation) => validation,
            Err(decision) => return Err((self, decision)),
        };
        Ok(self.map_success_validation(validation))
    }

    pub(crate) fn bind_resolved_failure(
        self,
        validation: RetainedClosureValidation<RetainedEvidenceClosureProof<EB>>,
    ) -> RetainedFailureValidation<'inv, O, P, PP, U, D, F, CB, EB> {
        self.map_failure_validation(validation)
    }

    pub(crate) fn bind_evidence_decision(
        self,
        decision: RetainedAssemblyDecision<CB, EB>,
    ) -> EvidenceBindingResult<'inv, O, P, PP, U, D, F, CB, EB> {
        let validation = match self.loan.bind_evidence_proof(decision) {
            Ok(validation) => validation,
            Err(decision) => return Err((self, decision)),
        };
        Ok(self.map_failure_validation(validation))
    }
}
