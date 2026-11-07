use super::super::payload_owner::PayloadValue;
use super::*;
use crate::concrete_store::ordinary::StoreSuccessFallbackProvenance;
use crate::retained::{
    RetainedAssemblyDecision, RetainedClosureValidation, RetainedEvidenceClosureProof,
};

pub(super) type EvidenceValidation =
    RetainedClosureValidation<RetainedEvidenceClosureProof<StoredBlock>>;
pub(super) type EvidenceDecision = RetainedAssemblyDecision<StoredBlock, StoredBlock>;

pub(super) struct FailureValidationFacts<EK, ET> {
    pub(super) root: OutcomeEvidenceStoreRef<EK, ET>,
    pub(super) failure_cause: PreparationFailureCode,
    pub(super) forced_cause: Option<PreparationFailureCode>,
    pub(super) original_error: Option<StorePrepareError>,
    pub(super) destination: StoreDimensionKey,
}

pub(super) struct FailureValidationPayloads<'loan, 'host, Cause> {
    pub(super) source: &'loan mut Option<PayloadLoan<'host, Cause>>,
    pub(super) runtime: &'loan mut Option<PayloadLoan<'host, StoreRuntimeError>>,
    pub(super) fallback: Option<CanonicalCauseSlot<'host, StoreRuntimeError>>,
}

impl<Cause> FailureValidationPayloads<'_, '_, Cause> {
    fn replace_source(&mut self, cause: StoreRuntimeError) -> bool {
        if drop_scoped_value(self.source.take()).is_err() {
            return false;
        }
        let Some(slot) = self.fallback.take() else {
            return false;
        };
        match slot.place(cause) {
            Ok(cause) => {
                *self.runtime = Some(cause);
                true
            }
            Err(cause) => {
                let _ = drop_scoped_value(cause);
                false
            }
        }
    }
}

pub(super) enum FinalFailureResolution {
    Valid,
    Rejected,
    MinimalRuntime {
        code: PreparationFailureCode,
        provenance: StoreSuccessFallbackProvenance,
    },
    Contain {
        error: Option<StorePrepareError>,
    },
}

/// The concrete caller guards both the bound proof and an unmatched original decision.
pub(super) struct FailureValidationCustody<'loan> {
    pub(super) resolved: &'loan mut Option<EvidenceValidation>,
    pub(super) unmatched: &'loan mut Option<EvidenceDecision>,
}

pub(super) fn run_rooted_failure_validation<O, PP, U, Cause, EK, ET, State, Validate>(
    mut evidence: StoreFailureEvidenceLoan<'_, O, PP, U>,
    facts: FailureValidationFacts<EK, ET>,
    work: &FailureEvidenceWorkBudget,
    mut payloads: FailureValidationPayloads<'_, '_, Cause>,
    custody: FailureValidationCustody<'_>,
    validate: Validate,
) -> FinalFailureResolution
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    ET: Copy + Send + Sync + 'static,
    State: FailureLifecycleState,
    Validate: for<'session, 'storage, 'control> FnMut(
        &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
        FailureEvidenceMember<'session, EK, ET>,
    ) -> bool,
{
    let contain = FinalFailureResolution::Contain {
        error: facts.original_error,
    };
    let mut callback = PayloadValue::new(validate);
    let valid_slot = payloads.source.as_ref().is_some_and(|source| {
        payloads
            .fallback
            .as_ref()
            .is_some_and(|slot| slot.is_vacant_for(source.identity()))
    });
    if facts.original_error.is_some() || !valid_slot {
        let _ = callback.dispose();
        return contain;
    }
    let publication = evidence.publication_impl();
    if let Some(code) = publication.state.fallback_cause {
        if callback.dispose().is_err() || !payloads.replace_source(activation_product_graph_cause())
        {
            return contain;
        }
        return FinalFailureResolution::MinimalRuntime {
            code,
            provenance: StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
        };
    }
    let mut controlled_failure = None;
    let decision =
        publication
            .raw
            .validate_evidence_controlled_decision(facts.root.raw, |raw, _, control| {
                failure_evidence_validator(work, &mut controlled_failure, &mut callback.value)(
                    StoreEvidenceValidationSession { raw },
                    facts.root,
                    control,
                )
            });
    match publication.raw.loan().bind_evidence_proof(decision) {
        Ok(validation) => *custody.resolved = Some(validation),
        Err(decision) => {
            *custody.unmatched = Some(decision);
            let _ = callback.dispose();
            return contain;
        }
    }
    if callback.dispose().is_err() {
        return contain;
    }
    // Semantic rejection preserves an earlier verified cause at the actual assembly once.
    if let Some(RetainedClosureValidation::Rejected { cause }) = custody.resolved.as_mut()
        && let Some(forced) = facts.forced_cause
    {
        publication.latch_activation_terminal_cause(forced);
        *cause = forced;
    }
    let validation = custody
        .resolved
        .as_ref()
        .expect("the original bound failure result stays in concrete guarded custody");
    let minimal_code = match validation {
        RetainedClosureValidation::Valid(_) => facts.forced_cause.unwrap_or(facts.failure_cause),
        RetainedClosureValidation::Rejected { cause }
        | RetainedClosureValidation::FallbackOnly { cause, .. } => *cause,
    };
    if let Some(cause) = controlled_failure {
        return if payloads.replace_source(cause) {
            FinalFailureResolution::MinimalRuntime {
                code: minimal_code,
                provenance: match validation {
                    RetainedClosureValidation::FallbackOnly {
                        activation_containment_required,
                        ..
                    } => StoreSuccessFallbackProvenance::RawValidated {
                        activation_containment_required: *activation_containment_required,
                    },
                    _ => StoreSuccessFallbackProvenance::FailureUnverified,
                },
            }
        } else {
            contain
        };
    }
    match validation {
        RetainedClosureValidation::Valid(proof) => {
            match State::validate_commitment(&mut publication.raw, proof, facts.destination) {
                Ok(()) => FinalFailureResolution::Valid,
                Err(cause) => {
                    if matches!(
                        classify_activation_fault(&cause),
                        ActivationFaultClassification::Contract
                    ) || !payloads.replace_source(cause)
                    {
                        contain
                    } else {
                        FinalFailureResolution::MinimalRuntime {
                            code: minimal_code,
                            provenance: StoreSuccessFallbackProvenance::FailureUnverified,
                        }
                    }
                }
            }
        }
        RetainedClosureValidation::Rejected { .. } => FinalFailureResolution::Rejected,
        RetainedClosureValidation::FallbackOnly {
            cause,
            activation_containment_required,
        } => {
            if *activation_containment_required
                || !payloads.replace_source(activation_product_graph_cause())
            {
                contain
            } else {
                FinalFailureResolution::MinimalRuntime {
                    code: *cause,
                    provenance: StoreSuccessFallbackProvenance::RawValidated {
                        activation_containment_required: false,
                    },
                }
            }
        }
    }
}

struct DynamicFailureTransition<'host, O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    source: Option<PayloadLoan<'host, Cause>>,
    runtime: Option<PayloadLoan<'host, StoreRuntimeError>>,
    raw: Option<super::super::super::RawStoreAssembly<'host, O, PP, U>>,
    resolved: Option<EvidenceValidation>,
    unmatched: Option<EvidenceDecision>,
}

impl<O, PP, U, Cause> DynamicFailureTransition<'_, O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain(&mut self) -> StoreActivationContainment {
        let _ = drop_scoped_value(self.source.take());
        let _ = drop_scoped_value(self.runtime.take());
        let _ = drop_scoped_value(self.resolved.take());
        let _ = drop_scoped_value(self.unmatched.take());
        StoreActivationContainment {
            cause: self
                .raw
                .take()
                .expect("containment retains the original dynamic assembly")
                .contain_activation_fault(),
        }
    }
}

impl<O, PP, U, Cause> Drop for DynamicFailureTransition<'_, O, PP, U, Cause>
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

type DynamicFailureValidationResult<'host, O, PP, U, Cause, EK, ET, State> = Result<
    DynamicRootedFailureValidation<'host, O, PP, U, Cause, EK, ET, State>,
    StoreActivationContainment,
>;

pub(super) fn validate_dynamic<'host, O, PP, U, Cause, EK, ET, State>(
    rooted: DynamicRootedFailureEvidence<'host, O, PP, U, Cause, EK, ET, State>,
    fallback: CanonicalCauseSlot<'host, StoreRuntimeError>,
    validate: impl for<'session, 'storage, 'control> FnMut(
        &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
        FailureEvidenceMember<'session, EK, ET>,
    ) -> bool,
) -> DynamicFailureValidationResult<'host, O, PP, U, Cause, EK, ET, State>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    ET: Copy + Send + Sync + 'static,
    State: FailureLifecycleState,
{
    let RootedFailureEvidence {
        owner,
        failure_work,
        outcome_evidence_destination_dimension,
        _lifecycle: _,
    } = rooted;
    let (source, failure) = owner.into_parts();
    let StoreFailureAssembly {
        raw,
        mut state,
        forced_cause,
        evidence_root,
        domain,
    } = failure;
    let (raw, failure_cause) = raw.into_failure_parts();
    let mut transition = DynamicFailureTransition {
        source: Some(source),
        runtime: None,
        raw: Some(raw),
        resolved: None,
        unmatched: None,
    };
    let resolution = run_rooted_failure_validation::<_, _, _, _, _, _, State, _>(
        StoreFailureEvidenceLoan::new(
            transition
                .raw
                .as_mut()
                .expect("original assembly")
                .loan_mut()
                .borrow_ordinary(),
            &domain,
            &mut state,
        ),
        FailureValidationFacts {
            root: evidence_root,
            failure_cause,
            forced_cause,
            original_error: None,
            destination: outcome_evidence_destination_dimension,
        },
        &failure_work,
        FailureValidationPayloads {
            source: &mut transition.source,
            runtime: &mut transition.runtime,
            fallback: Some(fallback),
        },
        FailureValidationCustody {
            resolved: &mut transition.resolved,
            unmatched: &mut transition.unmatched,
        },
        validate,
    );
    match resolution {
        FinalFailureResolution::Contain { .. } => Err(transition.contain()),
        FinalFailureResolution::MinimalRuntime {
            code,
            provenance: _,
        } => {
            let runtime = transition.runtime.take().expect("original runtime cause");
            let mut runtime = PayloadValue::new(runtime);
            let raw = transition.raw.take().expect("original assembly");
            let prepared = raw.prepare_minimal_fallback_with(code).map_err(|error| {
                StoreActivationContainment {
                    cause: map_raw_prepare_error(error),
                }
            })?;
            Ok(RootedFailureValidation::Fallback(
                VerifiedMinimalFallback::new(
                    classify_prepared_fallback(prepared, domain.into_owned()),
                    runtime
                        .take()
                        .expect("original runtime permission")
                        .into_owned(),
                ),
            ))
        }
        FinalFailureResolution::Valid | FinalFailureResolution::Rejected => {
            let raw = transition.raw.take().expect("original assembly");
            let validation = raw
                .bind_resolved_failure(transition.resolved.take().expect("original bound result"));
            let source = transition.source.take().expect("original typed cause");
            match super::super::super::map_resolved_failure_validation(
                validation,
                evidence_root,
                domain,
                state,
                forced_cause,
            ) {
                StoreFailureValidation::Valid(validated) => {
                    Ok(RootedFailureValidation::Valid(ValidatedFailureEvidence {
                        owner: PayloadOwner::new(source, validated),
                        _lifecycle: PhantomData,
                    }))
                }
                StoreFailureValidation::Rejected(failure) => {
                    Ok(RootedFailureValidation::Rejected(RejectedFailureEvidence {
                        owner: PayloadOwner::new(source, failure),
                    }))
                }
                StoreFailureValidation::FallbackOnly(_) => {
                    unreachable!("the concrete owner binds the original non-fallback result")
                }
            }
        }
    }
}
