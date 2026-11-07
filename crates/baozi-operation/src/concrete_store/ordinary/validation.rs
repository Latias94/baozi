use super::super::{
    StoreEvidenceValidationSession, StoreValidationSession, StoreWorkCallbackFailure,
};
use super::*;
use crate::retained::{
    RetainedAssemblyDecision, RetainedClosureValidation, RetainedSuccessClosureProof,
};

pub(in crate::concrete_store) enum StoreClosureDecision {
    FallbackOnly {
        cause: PreparationFailureCode,
    },
    Closure {
        retained: RetainedAssemblyDecision<StoredBlock, StoredBlock>,
        callback_failure: Option<StoreWorkCallbackFailure>,
        forced_cause: Option<PreparationFailureCode>,
    },
}

pub(in crate::concrete_store) enum StoreSuccessResolution {
    Valid(RetainedSuccessClosureProof<StoredBlock, StoredBlock>),
    Rejected {
        failure_cause: PreparationFailureCode,
        forced_cause: Option<PreparationFailureCode>,
    },
    Interrupted {
        failure_cause: PreparationFailureCode,
        cause: StoreWorkError,
    },
    FallbackOnly {
        cause: PreparationFailureCode,
        provenance: StoreSuccessFallbackProvenance,
    },
}

pub(in crate::concrete_store) enum StoreSuccessFallbackProvenance {
    EarlyAssemblyLatch,
    RawValidated {
        activation_containment_required: bool,
    },
    FailureUnverified,
}

impl<O, PP, U, L> StorePublicationLoan<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn try_discard_canonical_after_resolution(
        &mut self,
    ) -> Result<(), crate::retained::FinalizationContainmentError> {
        self.raw.loan_mut().try_discard_canonical_ownership()
    }

    pub(in crate::concrete_store) fn validate_success_decision<CK, CT, EK, ET>(
        &mut self,
        canonical: CanonicalStoreRef<CK, CT>,
        evidence: OutcomeEvidenceStoreRef<EK, ET>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreValidationSession<'session, 'storage>,
            CanonicalStoreRef<CK, CT>,
            OutcomeEvidenceStoreRef<EK, ET>,
        ) -> bool,
    ) -> StoreClosureDecision
    where
        CT: Copy + Send + Sync + 'static,
        ET: Copy + Send + Sync + 'static,
    {
        if let Some(cause) = self.state.fallback_cause {
            return StoreClosureDecision::FallbackOnly { cause };
        }
        let forced_cause = self.state.success_rejection;
        let retained = if forced_cause.is_some() {
            self.raw
                .validate_success_decision(canonical.raw, evidence.raw, |_, _, _| false)
        } else {
            self.raw
                .validate_success_decision(canonical.raw, evidence.raw, |raw, _, _| {
                    let mut session = StoreValidationSession { raw };
                    validate(&mut session, canonical, evidence)
                })
        };
        StoreClosureDecision::Closure {
            retained,
            callback_failure: None,
            forced_cause,
        }
    }

    pub(in crate::concrete_store) fn validate_success_controlled_decision<CK, CT, EK, ET>(
        &mut self,
        work_dimension: StoreDimensionKey,
        scratch_dimension: StoreDimensionKey,
        canonical: CanonicalStoreRef<CK, CT>,
        evidence: OutcomeEvidenceStoreRef<EK, ET>,
        validate: impl for<'session, 'storage, 'control> FnOnce(
            &mut StoreValidationSession<'session, 'storage>,
            CanonicalStoreRef<CK, CT>,
            OutcomeEvidenceStoreRef<EK, ET>,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<bool, StoreWorkError>,
    ) -> Result<StoreClosureDecision, StoreRuntimeError>
    where
        CT: Copy + Send + Sync + 'static,
        ET: Copy + Send + Sync + 'static,
    {
        self.validate_dimension(work_dimension)?;
        self.validate_dimension(scratch_dimension)?;
        if let Some(cause) = self.state.fallback_cause {
            return Ok(StoreClosureDecision::FallbackOnly { cause });
        }
        let forced_cause = self.state.success_rejection;
        if forced_cause.is_some() {
            return Ok(StoreClosureDecision::Closure {
                retained: self.raw.validate_success_decision(
                    canonical.raw,
                    evidence.raw,
                    |_, _, _| false,
                ),
                callback_failure: None,
                forced_cause,
            });
        }
        let mut callback_failure = None;
        let retained = self.raw.validate_success_controlled_decision(
            work_dimension,
            scratch_dimension,
            canonical.raw,
            evidence.raw,
            |raw, _, _, raw_control| {
                let mut session = StoreValidationSession { raw };
                let mut control = RetainedStoreWorkControl {
                    raw: raw_control,
                    sticky: None,
                };
                let callback = validate(&mut session, canonical, evidence, &mut control);
                match control.finish_validation(callback) {
                    Ok(accepted) => accepted,
                    Err(cause) => {
                        callback_failure = Some(cause);
                        false
                    }
                }
            },
        );
        Ok(StoreClosureDecision::Closure {
            retained,
            callback_failure,
            forced_cause,
        })
    }

    pub(in crate::concrete_store) fn resolve_success_decision(
        &mut self,
        decision: StoreClosureDecision,
    ) -> Result<StoreSuccessResolution, StoreClosureDecision> {
        let (retained, callback_failure, forced_cause) = match decision {
            StoreClosureDecision::FallbackOnly { cause } => {
                return Ok(StoreSuccessResolution::FallbackOnly {
                    cause,
                    provenance: StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                });
            }
            StoreClosureDecision::Closure {
                retained,
                callback_failure,
                forced_cause,
            } => (retained, callback_failure, forced_cause),
        };
        let validation = match self.raw.loan_mut().bind_success_proof(retained) {
            Ok(validation) => validation,
            Err(retained) => {
                return Err(StoreClosureDecision::Closure {
                    retained,
                    callback_failure,
                    forced_cause,
                });
            }
        };
        let rejection_cause = forced_cause.or_else(|| match &callback_failure {
            Some(StoreWorkCallbackFailure::Verified(cause)) => Some(work_failure_code(cause)),
            Some(StoreWorkCallbackFailure::UnverifiedCause) => Some(
                PreparationFailureCode::domain(UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE),
            ),
            None => None,
        });
        let validation = match rejection_cause {
            Some(cause) => self
                .raw
                .loan_mut()
                .apply_success_rejection(validation, cause),
            None => validation,
        };
        Ok(match validation {
            RetainedClosureValidation::Valid(proof) => StoreSuccessResolution::Valid(proof),
            RetainedClosureValidation::Rejected {
                cause: failure_cause,
            } => match (forced_cause, callback_failure) {
                (None, Some(StoreWorkCallbackFailure::Verified(cause))) => {
                    StoreSuccessResolution::Interrupted {
                        failure_cause,
                        cause,
                    }
                }
                (None, Some(StoreWorkCallbackFailure::UnverifiedCause)) => {
                    StoreSuccessResolution::FallbackOnly {
                        cause: failure_cause,
                        provenance: StoreSuccessFallbackProvenance::FailureUnverified,
                    }
                }
                (forced_cause, _) => StoreSuccessResolution::Rejected {
                    failure_cause,
                    forced_cause,
                },
            },
            RetainedClosureValidation::FallbackOnly {
                cause,
                activation_containment_required,
            } => StoreSuccessResolution::FallbackOnly {
                cause,
                provenance: StoreSuccessFallbackProvenance::RawValidated {
                    activation_containment_required,
                },
            },
        })
    }

    pub(in crate::concrete_store) fn validate_evidence_decision<EK, ET>(
        &mut self,
        evidence: OutcomeEvidenceStoreRef<EK, ET>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreEvidenceValidationSession<'session, 'storage>,
            OutcomeEvidenceStoreRef<EK, ET>,
        ) -> bool,
    ) -> StoreClosureDecision
    where
        ET: Copy + Send + Sync + 'static,
    {
        if let Some(cause) = self.state.fallback_cause {
            return StoreClosureDecision::FallbackOnly { cause };
        }
        let retained = self.raw.validate_evidence_decision(evidence.raw, |raw, _| {
            let mut session = StoreEvidenceValidationSession { raw };
            validate(&mut session, evidence)
        });
        StoreClosureDecision::Closure {
            retained,
            callback_failure: None,
            forced_cause: self.state.success_rejection,
        }
    }
}
