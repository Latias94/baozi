use super::super::ordinary::{StoreSuccessFallbackProvenance, StoreSuccessResolution};
use super::canonical_validation::{CanonicalValidationAttempt, CanonicalValidationFirstFault};
use super::payload_owner::PayloadValue as CentralValue;
use super::*;
use crate::retained::RetainedSuccessClosureProof;

pub(super) struct CentralValidationFacts<CK, CT> {
    pub(super) canonical_root: CanonicalStoreRef<CK, CT>,
    pub(super) validation: ScopedActivationValidation,
    pub(super) evidence_destination: StoreDimensionKey,
}

pub(super) struct CentralValidationRequest<'host, Proof, E, CK, CT, PP, U> {
    pub(super) proof_slot: CanonicalProofSlot<'host, Proof>,
    pub(super) validation_slots: CanonicalValidationSlots<'host, E>,
    pub(super) facts: CentralValidationFacts<CK, CT>,
    pub(super) progress: CanonicalValidationProgress<PP, U>,
}

pub(super) struct CentralFailureFacts<EK, ET> {
    pub(super) evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    pub(super) failure_cause: PreparationFailureCode,
    pub(super) forced_cause: Option<PreparationFailureCode>,
}

pub(super) enum CentralPendingDecision<'host, T, Proof, E, CK, CT, EK, ET> {
    Candidate {
        output: PayloadLoan<'host, T>,
        proof: PayloadLoan<'host, Proof>,
        closure: RetainedSuccessClosureProof<StoredBlock, StoredBlock>,
        canonical_root: CanonicalStoreRef<CK, CT>,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
        destinations: ZeroCommitDestinationFacts,
    },
    Rejected {
        cause: PayloadLoan<'host, StoreCanonicalRejection<E>>,
        failure: CentralFailureFacts<EK, ET>,
    },
    Interrupted {
        cause: PayloadLoan<'host, StoreWorkError>,
        failure: CentralFailureFacts<EK, ET>,
    },
    Fallback {
        cause: PayloadLoan<'host, StoreRuntimeError>,
        code: PreparationFailureCode,
        provenance: StoreSuccessFallbackProvenance,
    },
    Contain {
        recorded: Option<StoreActivationContainment>,
    },
}

pub(super) fn drop_pending_central_decision<T, Proof, E, CK, CT, EK, ET>(
    pending: Option<CentralPendingDecision<'_, T, Proof, E, CK, CT, EK, ET>>,
) -> Result<(), ()> {
    match pending {
        Some(CentralPendingDecision::Candidate { output, proof, .. }) => {
            let proof_drop_failed = drop_scoped_value(proof).is_err();
            let output_drop_failed = drop_scoped_value(output).is_err();
            if proof_drop_failed || output_drop_failed {
                Err(())
            } else {
                Ok(())
            }
        }
        Some(CentralPendingDecision::Rejected { cause, .. }) => drop_scoped_value(cause),
        Some(CentralPendingDecision::Interrupted { cause, .. }) => drop_scoped_value(cause),
        Some(CentralPendingDecision::Fallback { cause, .. }) => drop_scoped_value(cause),
        Some(CentralPendingDecision::Contain { .. }) | None => Ok(()),
    }
}

struct CentralCallbacks<Build, Validate> {
    build: Option<Build>,
    validate: Option<Validate>,
}

impl<Build, Validate> CentralCallbacks<Build, Validate> {
    fn dispose_build(&mut self) -> Result<(), ()> {
        drop_scoped_value(self.build.take())
    }

    fn dispose_validate(&mut self) -> Result<(), ()> {
        drop_scoped_value(self.validate.take())
    }
}

impl<Build, Validate> Drop for CentralCallbacks<Build, Validate> {
    fn drop(&mut self) {
        let _ = self.dispose_build();
        let _ = self.dispose_validate();
    }
}

struct CentralResolver<'driver, 'host, 'op, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    publication: &'driver mut StorePublicationLoan<'op, O, PP, U, L>,
    output: &'driver mut Option<PayloadLoan<'host, T>>,
    pending: &'driver mut Option<CentralPendingDecision<'host, T, Proof, E, CK, CT, EK, ET>>,
    proof_slot: CanonicalProofSlot<'host, Proof>,
    validation_slots: CanonicalValidationSlots<'host, E>,
    fallback_slot: CanonicalCauseSlot<'host, StoreRuntimeError>,
    canonical_root: CanonicalStoreRef<CK, CT>,
    evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    canonical_destination: StoreDimensionKey,
    evidence_destination: StoreDimensionKey,
}

impl<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
    CentralResolver<'_, 'host, '_, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain(self, recorded: Option<StoreActivationContainment>) -> bool {
        let _ = drop_scoped_value(self.output.take());
        *self.pending = Some(CentralPendingDecision::Contain { recorded });
        false
    }

    #[inline(never)]
    fn resolve(
        self,
        validation: Result<StoreSuccessResolution, StoreRuntimeError>,
        mut attempt: CentralValue<CanonicalValidationAttempt<Proof, E>>,
        first_fault: Option<CanonicalValidationFirstFault>,
    ) -> bool {
        let contract_failed_first = matches!(
            first_fault.as_ref(),
            Some(CanonicalValidationFirstFault::Contract)
        );
        match validation {
            Err(_cause) => {
                let _ = attempt.dispose();
                self.contain(None)
            }
            Ok(StoreSuccessResolution::FallbackOnly { cause, provenance }) => {
                self.resolve_fallback(attempt, first_fault, cause, provenance)
            }
            Ok(StoreSuccessResolution::Valid(closure)) => self.resolve_valid(attempt, closure),
            Ok(StoreSuccessResolution::Rejected {
                failure_cause,
                forced_cause,
            }) => {
                let failure = CentralFailureFacts {
                    evidence_root: self.evidence_root,
                    failure_cause,
                    forced_cause,
                };
                match attempt.value.take() {
                    Some(CanonicalValidationAttempt::Decision(
                        CanonicalValidationDecision::Reject { cause, stable_code },
                    )) if !contract_failed_first => self.resolve_trusted_rejection(
                        failure,
                        StoreCanonicalRejection::Requested(cause),
                        stable_code,
                    ),
                    Some(CanonicalValidationAttempt::Runtime { cause, stable_code })
                        if !contract_failed_first =>
                    {
                        self.resolve_trusted_rejection(
                            failure,
                            StoreCanonicalRejection::Runtime(cause),
                            stable_code,
                        )
                    }
                    remaining => {
                        attempt.value = remaining;
                        self.resolve_contract_rejection(failure, attempt)
                    }
                }
            }
            Ok(StoreSuccessResolution::Interrupted {
                failure_cause,
                cause,
            }) => {
                let failure = CentralFailureFacts {
                    evidence_root: self.evidence_root,
                    failure_cause,
                    forced_cause: Some(failure_cause),
                };
                if contract_failed_first {
                    self.resolve_contract_interruption(failure, cause, attempt)
                } else {
                    self.resolve_trusted_interruption(failure, cause, attempt)
                }
            }
        }
    }

    #[inline(never)]
    fn resolve_fallback(
        self,
        mut attempt: CentralValue<CanonicalValidationAttempt<Proof, E>>,
        first_fault: Option<CanonicalValidationFirstFault>,
        code: PreparationFailureCode,
        provenance: StoreSuccessFallbackProvenance,
    ) -> bool {
        let containment_required = matches!(
            provenance,
            StoreSuccessFallbackProvenance::RawValidated {
                activation_containment_required: true,
            }
        );
        let attempt_drop_failed = attempt.dispose().is_err();
        let output_drop_failed = drop_scoped_value(self.output.take()).is_err();
        if containment_required || attempt_drop_failed || output_drop_failed {
            return self.contain(None);
        }
        let cause = match first_fault {
            Some(CanonicalValidationFirstFault::Contract) => {
                StoreRuntimeError::CanonicalValidationContractRejected
            }
            Some(CanonicalValidationFirstFault::Runtime(cause)) => cause.into_runtime(),
            None => activation_product_graph_cause(),
        };
        match self.fallback_slot.place(cause) {
            Ok(cause) => {
                *self.pending = Some(CentralPendingDecision::Fallback {
                    cause,
                    code,
                    provenance,
                });
                true
            }
            Err(cause) => {
                let _ = drop_scoped_value(cause);
                *self.pending = Some(CentralPendingDecision::Contain { recorded: None });
                false
            }
        }
    }

    #[inline(never)]
    fn resolve_valid(
        self,
        mut attempt: CentralValue<CanonicalValidationAttempt<Proof, E>>,
        closure: RetainedSuccessClosureProof<StoredBlock, StoredBlock>,
    ) -> bool {
        let mut proof = match attempt.value.take() {
            Some(CanonicalValidationAttempt::Decision(CanonicalValidationDecision::Accept(
                proof,
            ))) => CentralValue::new(proof),
            remaining => {
                attempt.value = remaining;
                let _ = attempt.dispose();
                return self.contain(None);
            }
        };
        let destinations = match ZeroCommitDestinationFacts::try_new(
            self.canonical_destination,
            closure.canonical_member_count(),
            self.evidence_destination,
            closure.outcome_evidence_member_count(),
        ) {
            Ok(destinations) => destinations,
            Err(_) => {
                let _ = proof.dispose();
                return self.contain(None);
            }
        };
        let proof = match self.proof_slot.place(
            proof
                .value
                .take()
                .expect("an accepted attempt retains its proof"),
        ) {
            Ok(proof) => proof,
            Err(proof) => {
                let _ = drop_scoped_value(proof);
                let _ = drop_scoped_value(self.output.take());
                *self.pending = Some(CentralPendingDecision::Contain { recorded: None });
                return false;
            }
        };
        let Some(output) = self.output.take() else {
            let _ = drop_scoped_value(proof);
            *self.pending = Some(CentralPendingDecision::Contain { recorded: None });
            return false;
        };
        *self.pending = Some(CentralPendingDecision::Candidate {
            output,
            proof,
            closure,
            canonical_root: self.canonical_root,
            evidence_root: self.evidence_root,
            destinations,
        });
        true
    }

    #[inline(never)]
    fn resolve_trusted_rejection(
        self,
        mut failure: CentralFailureFacts<EK, ET>,
        cause: StoreCanonicalRejection<E>,
        stable_code: u32,
    ) -> bool {
        let mut cause = CentralValue::new(cause);
        let forced_cause = PreparationFailureCode::domain(stable_code);
        failure.failure_cause = forced_cause;
        failure.forced_cause = Some(forced_cause);
        self.publication
            .latch_activation_terminal_cause(forced_cause);
        if let Err(containment) = self.publication.try_discard_canonical_after_resolution() {
            let _ = drop_scoped_value(self.output.take());
            let _ = cause.dispose();
            return self.contain(Some(StoreActivationContainment { cause: containment }));
        }
        if drop_scoped_value(self.output.take()).is_err() {
            let _ = cause.dispose();
            return self.contain(None);
        }
        match self.validation_slots.rejection.place(
            cause
                .value
                .take()
                .expect("a trusted rejection retains its cause"),
        ) {
            Ok(cause) => {
                *self.pending = Some(CentralPendingDecision::Rejected { cause, failure });
                true
            }
            Err(cause) => {
                let _ = drop_scoped_value(cause);
                *self.pending = Some(CentralPendingDecision::Contain { recorded: None });
                false
            }
        }
    }

    #[inline(never)]
    fn resolve_contract_rejection(
        self,
        mut failure: CentralFailureFacts<EK, ET>,
        mut attempt: CentralValue<CanonicalValidationAttempt<Proof, E>>,
    ) -> bool {
        let attempt_drop_failed = attempt.dispose().is_err();
        let forced_cause =
            PreparationFailureCode::product_graph(CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE);
        failure.failure_cause = forced_cause;
        failure.forced_cause = Some(forced_cause);
        self.publication
            .latch_activation_terminal_cause(forced_cause);
        if let Err(containment) = self.publication.try_discard_canonical_after_resolution() {
            return self.contain(Some(StoreActivationContainment { cause: containment }));
        }
        let output_drop_failed = drop_scoped_value(self.output.take()).is_err();
        if attempt_drop_failed || output_drop_failed {
            return self.contain(None);
        }
        match self
            .validation_slots
            .rejection
            .place(StoreCanonicalRejection::Runtime(
                StoreRuntimeError::CanonicalValidationContractRejected,
            )) {
            Ok(cause) => {
                *self.pending = Some(CentralPendingDecision::Rejected { cause, failure });
                true
            }
            Err(cause) => {
                let _ = drop_scoped_value(cause);
                *self.pending = Some(CentralPendingDecision::Contain { recorded: None });
                false
            }
        }
    }

    #[inline(never)]
    fn resolve_contract_interruption(
        self,
        failure: CentralFailureFacts<EK, ET>,
        cause: StoreWorkError,
        mut attempt: CentralValue<CanonicalValidationAttempt<Proof, E>>,
    ) -> bool {
        let mut cause = CentralValue::new(cause);
        let attempt_drop_failed = attempt.dispose().is_err();
        let cause_drop_failed = cause.dispose().is_err();
        if attempt_drop_failed || cause_drop_failed {
            return self.contain(None);
        }
        self.resolve_contract_rejection(failure, attempt)
    }

    #[inline(never)]
    fn resolve_trusted_interruption(
        self,
        failure: CentralFailureFacts<EK, ET>,
        cause: StoreWorkError,
        mut attempt: CentralValue<CanonicalValidationAttempt<Proof, E>>,
    ) -> bool {
        let mut cause = CentralValue::new(cause);
        if attempt.dispose().is_err() {
            return self.contain(None);
        }
        if let Err(containment) = self.publication.try_discard_canonical_after_resolution() {
            return self.contain(Some(StoreActivationContainment { cause: containment }));
        }
        if drop_scoped_value(self.output.take()).is_err() {
            return self.contain(None);
        }
        match self.validation_slots.interruption.place(
            cause
                .value
                .take()
                .expect("a trusted interruption retains its cause"),
        ) {
            Ok(cause) => {
                *self.pending = Some(CentralPendingDecision::Interrupted { cause, failure });
                true
            }
            Err(cause) => {
                let _ = drop_scoped_value(cause);
                *self.pending = Some(CentralPendingDecision::Contain { recorded: None });
                false
            }
        }
    }
}

#[inline(never)]
pub(super) fn run_central_validation<
    'host,
    'op,
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CK,
    CT,
    EK,
    ET,
    L,
    Build,
    Validate,
>(
    mut publication: StorePublicationLoan<'op, O, PP, U, L>,
    output: &mut Option<PayloadLoan<'host, T>>,
    request: CentralValidationRequest<'host, Proof, E, CK, CT, PP, U>,
    failure_work: &mut FailureEvidenceWorkBudget,
    pending: &mut Option<CentralPendingDecision<'host, T, Proof, E, CK, CT, EK, ET>>,
    build_evidence: Build,
    validate: Validate,
) -> bool
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    CT: Copy + Send + Sync + 'static,
    ET: Copy + Send + Sync + 'static,
    Build: for<'borrow, 'lane> FnMut(
        &mut CanonicalOutcomeEvidenceLane<'borrow, 'lane, O, PP, U, L>,
        &T,
    ) -> OutcomeEvidenceDecision<EK, ET>,
    Validate: for<'session, 'storage, 'scope, 'operation> FnMut(
        CanonicalValidationPlanning<
            'session,
            'storage,
            'scope,
            'operation,
            O,
            PP,
            U,
            CK,
            CT,
            EK,
            ET,
            Proof,
            E,
        >,
        &T,
    ) -> Result<(), StoreWorkError>,
{
    let CentralValidationRequest {
        mut proof_slot,
        validation_slots,
        facts,
        progress,
    } = request;
    let mut callbacks = CentralCallbacks {
        build: Some(build_evidence),
        validate: Some(validate),
    };
    let fallback_slot = output.as_ref().and_then(|output| {
        if pending.is_none()
            && proof_slot.is_vacant_for(output.identity())
            && validation_slots.is_vacant_for(output.identity())
        {
            proof_slot.take_fallback_slot()
        } else {
            None
        }
    });
    let Some(fallback_slot) = fallback_slot else {
        return false;
    };
    let CentralValidationFacts {
        canonical_root,
        validation,
        evidence_destination,
    } = facts;
    let mut evidence_fault = None;
    let evidence = catch_unwind(AssertUnwindSafe(|| {
        let mut lane = CanonicalOutcomeEvidenceLane {
            publication: &mut publication,
            first_fault: &mut evidence_fault,
            failure_work,
            work_dimension: validation.work_dimension,
        };
        let build = callbacks
            .build
            .as_mut()
            .expect("the evidence callback runs once");
        build(
            &mut lane,
            output
                .as_ref()
                .expect("the transition retains its output")
                .value(),
        )
    }));
    let build_drop_failed = callbacks.dispose_build().is_err();
    let evidence = match evidence {
        Ok(evidence) if !build_drop_failed => evidence,
        Ok(evidence) => {
            let _ = drop_scoped_value(evidence);
            return false;
        }
        Err(payload) => {
            discard_untrusted_panic_payload(payload);
            return false;
        }
    };
    let evidence_root = match (evidence_fault, evidence) {
        (Some(fault), OutcomeEvidenceDecision::Abort) if !fault.contract_seen => {
            let validate_drop_failed = callbacks.dispose_validate().is_err();
            let output_drop_failed = drop_scoped_value(output.take()).is_err();
            if validate_drop_failed || output_drop_failed {
                return false;
            }
            let Some(code) = fault.recoverable_code() else {
                return false;
            };
            publication.latch_activation_terminal_cause(code);
            let code = publication
                .state
                .fallback_cause
                .or(publication.state.success_rejection)
                .unwrap_or(code);
            match fallback_slot.place(fault.cause) {
                Ok(cause) => {
                    *pending = Some(CentralPendingDecision::Fallback {
                        cause,
                        code,
                        provenance: StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                    });
                    return true;
                }
                Err(cause) => {
                    let _ = drop_scoped_value(cause);
                    return false;
                }
            }
        }
        (Some(_), _) | (None, OutcomeEvidenceDecision::Abort) => return false,
        (None, OutcomeEvidenceDecision::Complete(root)) => root,
    };
    if publication.is_fallback_only() {
        return false;
    }
    publication.report_progress(
        progress.phase(),
        progress.unit(),
        0,
        Some(validation.work.get()),
    );
    let mut attempt = CentralValue { value: None };
    let mut exact = None;
    let mut first_fault = None;
    let resolution = publication
        .validate_success_controlled_decision(
            validation.work_dimension,
            validation.scratch_dimension,
            canonical_root,
            evidence_root,
            |session, canonical, evidence, control| {
                let validate = callbacks
                    .validate
                    .as_mut()
                    .expect("the validator runs once");
                let planning = CanonicalValidationPlanning::new(
                    StoreValidationSession {
                        raw: &mut *session.raw,
                    },
                    control,
                    validation.phase,
                    validation.work,
                    CanonicalValidationMember::from_store(canonical),
                    OutcomeEvidenceValidationMember::from_store(evidence),
                    &mut first_fault,
                    &mut attempt.value,
                    &mut exact,
                );
                validate(
                    planning,
                    output
                        .as_ref()
                        .expect("the transition retains its output")
                        .value(),
                )?;
                Ok(exact.unwrap_or(false))
            },
        )
        .map(
            |decision| match publication.resolve_success_decision(decision) {
                Ok(resolution) => resolution,
                Err(_decision) => StoreSuccessResolution::FallbackOnly {
                    cause: PreparationFailureCode::domain(super::super::CONTAINMENT_FALLBACK_CODE),
                    provenance: StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                },
            },
        );
    if callbacks.dispose_validate().is_err() {
        let _ = attempt.dispose();
        return false;
    }
    CentralResolver {
        publication: &mut publication,
        output,
        pending,
        proof_slot,
        validation_slots,
        fallback_slot,
        canonical_root,
        evidence_root,
        canonical_destination: validation.canonical_destination,
        evidence_destination,
    }
    .resolve(resolution, attempt, first_fault)
}
