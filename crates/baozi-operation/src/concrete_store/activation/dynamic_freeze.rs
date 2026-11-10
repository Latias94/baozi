use super::*;
use crate::retained::{RetainedSuccessClosureProof, RetainedSuccessFreezeDecision};

#[cfg(test)]
mod tests;

type FreezeFailureResult<'host, O, PP, U, Cause, EK, ET, State> = Result<
    DynamicRootedFailureEvidence<'host, O, PP, U, Cause, EK, ET, State>,
    StoreActivationContainment,
>;

struct DynamicFreezeTransition<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: Option<super::super::RawStoreAssembly<'host, O, PP, U>>,
    closure: Option<RetainedSuccessClosureProof<StoredBlock, StoredBlock>>,
    domain: Option<StoreDomainLoan<'host>>,
    canonical_root: CanonicalStoreRef<CK, CT>,
    evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    output: Option<PayloadLoan<'host, T>>,
    proof: Option<PayloadLoan<'host, Proof>>,
    recorded_error: Option<StoreActivationContainment>,
}

impl<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
    DynamicFreezeTransition<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(
        proof: PayloadLoan<'host, Proof>,
        output: PayloadLoan<'host, T>,
        validated: ValidatedStoreSuccess<'host, O, PP, U, CK, CT, EK, ET>,
    ) -> Self {
        let (raw, closure, canonical_root, evidence_root, domain) = validated.into_storage_parts();
        Self {
            raw: Some(raw),
            closure: Some(closure),
            domain: Some(domain),
            canonical_root,
            evidence_root,
            output: Some(output),
            proof: Some(proof),
            recorded_error: None,
        }
    }

    fn contain(&mut self) -> StoreActivationContainment {
        let _ = freeze_driver::dispose_pair(
            &mut self.output,
            &mut self.proof,
            payload_owner::PayloadOrder::ProofBeforeOutput,
        );
        if let Some(error) = self.recorded_error.take() {
            drop(self.raw.take());
            error
        } else {
            StoreActivationContainment {
                cause: self
                    .raw
                    .take()
                    .expect("original dynamic freeze assembly")
                    .contain_activation_fault(),
            }
        }
    }

    fn into_ready(mut self) -> FrozenPayloadOwner<'host, O, PP, U, T, Proof, CK, CT, EK, ET> {
        let validated = ValidatedStoreSuccess::from_storage_parts(
            self.raw.take().expect("original assembly"),
            self.closure.take().expect("original success closure"),
            self.canonical_root,
            self.evidence_root,
            self.domain.take().expect("original domain"),
        );
        let output = self.output.take().expect("original output permission");
        output.freeze_order();
        PayloadOwner::new(
            output,
            PayloadOwner::new(
                self.proof.take().expect("original proof permission"),
                validated,
            ),
        )
    }

    fn into_failure<Cause, State>(
        mut self,
        slot: CanonicalCauseSlot<'host, Cause>,
        cause: Cause,
        code: PreparationFailureCode,
        work: FailureEvidenceWorkBudget,
        destination: StoreDimensionKey,
    ) -> FreezeFailureResult<'host, O, PP, U, Cause, EK, ET, State> {
        let mut cause = payload_owner::PayloadValue::new(cause);
        if freeze_driver::dispose_pair(
            &mut self.output,
            &mut self.proof,
            payload_owner::PayloadOrder::ProofBeforeOutput,
        )
        .is_err()
        {
            let _ = cause.dispose();
            return Err(self.contain());
        }
        let cause = match slot.place(cause.take().expect("original interruption cause")) {
            Ok(cause) => cause,
            Err(cause) => {
                let _ = drop_scoped_value(cause);
                return Err(self.contain());
            }
        };
        let raw = self
            .raw
            .take()
            .expect("original discarded assembly")
            .bind_discarded_failure(code);
        Ok(RootedFailureEvidence {
            owner: PayloadOwner::new(
                cause,
                StoreFailureAssembly {
                    raw,
                    state: StorePublicationState::default(),
                    forced_cause: Some(code),
                    evidence_root: self.evidence_root,
                    domain: self.domain.take().expect("original domain"),
                },
            ),
            failure_work: work,
            outcome_evidence_destination_dimension: destination,
            _lifecycle: PhantomData,
        })
    }
}

impl<O, PP, U, T, Proof, CK, CT, EK, ET> Drop
    for DynamicFreezeTransition<'_, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.raw.is_some() {
            let _ = self.contain();
        }
    }
}

#[allow(
    clippy::type_complexity,
    reason = "the concrete result retains both typed roots and exact failure lifecycle"
)]
pub(super) fn issue_dynamic<'host, O, PP, U, T, Proof, CK, CT, EK, ET>(
    candidate: CanonicalCandidateStoreSession<'host, O, PP, U, T, Proof, CK, CT, EK, ET>,
    interruption: CanonicalCauseSlot<'host, CanonicalFreezeInterruption>,
    progress: FinalProgress<PP, U>,
) -> Result<
    CanonicalFreezePermitIssue<'host, O, PP, U, T, Proof, CK, CT, EK, ET>,
    StoreActivationContainment,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let CanonicalCandidateStoreSession {
        owner,
        destinations,
        _failure_work: failure_work,
    } = candidate;
    let (proof, owner) = owner.into_parts();
    let (output, validated) = owner.into_parts();
    let mut transition = DynamicFreezeTransition::new(proof, output, validated);
    if !interruption.is_vacant_for(
        transition
            .output
            .as_ref()
            .expect("original output")
            .identity(),
    ) {
        return Err(transition.contain());
    }
    let decision = {
        let mut assembly = transition
            .raw
            .as_mut()
            .expect("original assembly")
            .loan_mut()
            .borrow_ordinary();
        freeze_driver::issue_freeze(
            &mut assembly,
            transition.closure.as_ref().expect("original closure"),
            &destinations,
            progress,
        )
    };
    let decision = match decision {
        Ok(decision) => decision,
        Err(cause) => {
            transition.recorded_error = Some(StoreActivationContainment { cause });
            return Err(transition.contain());
        }
    };
    match decision {
        RetainedSuccessFreezeDecision::Ready => {
            Ok(CanonicalFreezePermitIssue::Ready(CanonicalFreezePermit {
                owner: transition.into_ready(),
                failure_work,
                destinations,
            }))
        }
        RetainedSuccessFreezeDecision::RichFailure {
            cause,
            control_cause,
        } => transition
            .into_failure(
                interruption,
                CanonicalFreezeInterruption::from_failure_code(cause, control_cause),
                cause,
                failure_work,
                destinations._outcome_evidence_dimension,
            )
            .map(CanonicalFreezePermitIssue::Interrupted),
        RetainedSuccessFreezeDecision::DestinationRejected { cause } => transition
            .into_failure(
                interruption,
                CanonicalFreezeInterruption::from_failure_code(cause, None),
                cause,
                failure_work,
                destinations._outcome_evidence_dimension,
            )
            .map(CanonicalFreezePermitIssue::DestinationRejected),
    }
}
