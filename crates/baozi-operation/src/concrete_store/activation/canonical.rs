use super::canonical_validation::{CanonicalValidationAttempt, CanonicalValidationFirstFault};
use super::*;

fn drop_validation_attempt<Proof, E>(
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
) -> Result<(), ()> {
    match attempt {
        Some(CanonicalValidationAttempt::Decision(decision)) => {
            drop_validation_decision(Some(decision))
        }
        Some(CanonicalValidationAttempt::Runtime {
            cause,
            stable_code: _,
        }) => drop_scoped_value(cause),
        None => Ok(()),
    }
}

type CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> = Result<
    CanonicalCentralValidationOutcome<
        O,
        PP,
        U,
        T,
        Proof,
        E,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
    >,
    StoreActivationContainment,
>;

#[inline(never)]
fn contain_central_validation_callback_drop<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
    L,
>(
    validation: StoreControlledSuccessValidation<
        O,
        PP,
        U,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
        L,
    >,
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
    output: ManuallyDrop<T>,
) -> StoreActivationContainment
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let _ = drop_validation_attempt(attempt);
    let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
    let cause = match validation {
        StoreControlledSuccessValidation::InvalidWorkDimension { assembly, cause: _ } => {
            assembly.contain_activation_fault()
        }
        StoreControlledSuccessValidation::FallbackOnly(fallback) => {
            fallback.contain_activation_fault()
        }
        StoreControlledSuccessValidation::Valid(validated) => validated.contain_activation_fault(),
        StoreControlledSuccessValidation::Rejected(failure)
        | StoreControlledSuccessValidation::Interrupted { failure, cause: _ } => {
            failure.contain_activation_fault()
        }
    };
    StoreActivationContainment { cause }
}

// Keep mutually exclusive allocation-free terminal owners out of the active validator frame.
// Windows test threads use a 2 MiB stack, so these phase boundaries are part of the API's
// operational footprint rather than an optimization hint.
#[inline(never)]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn resolve_central_validation<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
    L,
>(
    validation: StoreControlledSuccessValidation<
        O,
        PP,
        U,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
        L,
    >,
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
    validation_first_fault: Option<CanonicalValidationFirstFault>,
    output: ManuallyDrop<T>,
    failure_work: FailureEvidenceWorkBudget,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
) -> CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let validation_contract_failed_first = matches!(
        validation_first_fault.as_ref(),
        Some(CanonicalValidationFirstFault::Contract)
    );
    match validation {
        StoreControlledSuccessValidation::InvalidWorkDimension { assembly, cause: _ } => {
            resolve_invalid_central_validation(assembly, attempt, output)
        }
        StoreControlledSuccessValidation::FallbackOnly(fallback) => {
            resolve_fallback_central_validation(fallback, attempt, validation_first_fault, output)
        }
        StoreControlledSuccessValidation::Valid(validated) => resolve_valid_central_validation(
            validated,
            attempt,
            output,
            failure_work,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
        ),
        StoreControlledSuccessValidation::Rejected(failure) => match attempt {
            Some(CanonicalValidationAttempt::Decision(CanonicalValidationDecision::Reject {
                cause,
                stable_code,
            })) if !validation_contract_failed_first => resolve_trusted_central_rejection(
                failure,
                StoreCanonicalRejection::Requested(cause),
                stable_code,
                output,
                failure_work,
                outcome_evidence_destination_dimension,
            ),
            Some(CanonicalValidationAttempt::Runtime { cause, stable_code })
                if !validation_contract_failed_first =>
            {
                resolve_trusted_central_rejection(
                    failure,
                    StoreCanonicalRejection::Runtime(cause),
                    stable_code,
                    output,
                    failure_work,
                    outcome_evidence_destination_dimension,
                )
            }
            attempt => resolve_contract_central_rejection(
                failure,
                attempt,
                output,
                failure_work,
                outcome_evidence_destination_dimension,
            ),
        },
        StoreControlledSuccessValidation::Interrupted { failure, cause } => {
            if validation_contract_failed_first {
                resolve_contract_central_interruption(
                    failure,
                    cause,
                    attempt,
                    output,
                    failure_work,
                    outcome_evidence_destination_dimension,
                )
            } else {
                resolve_trusted_central_interruption(
                    failure,
                    cause,
                    attempt,
                    output,
                    failure_work,
                    outcome_evidence_destination_dimension,
                )
            }
        }
    }
}

#[inline(never)]
fn resolve_invalid_central_validation<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
    L,
>(
    assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
    output: ManuallyDrop<T>,
) -> CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let _ = drop_validation_attempt(attempt);
    let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
    Err(StoreActivationContainment {
        cause: assembly.contain_activation_fault(),
    })
}

#[inline(never)]
fn resolve_fallback_central_validation<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>(
    fallback: StoreFallbackOnly<O, PP, U>,
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
    validation_first_fault: Option<CanonicalValidationFirstFault>,
    output: ManuallyDrop<T>,
) -> CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let containment_required = fallback.activation_containment_required();
    let decision_drop_failed = drop_validation_attempt(attempt).is_err();
    let output_drop_failed = drop_scoped_value(ManuallyDrop::into_inner(output)).is_err();
    if containment_required || decision_drop_failed || output_drop_failed {
        return Err(StoreActivationContainment {
            cause: fallback.contain_activation_fault(),
        });
    }
    let cause = match validation_first_fault {
        Some(CanonicalValidationFirstFault::Contract) => {
            StoreRuntimeError::CanonicalValidationContractRejected
        }
        Some(CanonicalValidationFirstFault::Runtime(cause)) => cause.into_runtime(),
        None => activation_product_graph_cause(),
    };
    Ok(CanonicalCentralValidationOutcome::Fallback(
        PendingMinimalFallback::new(fallback, cause),
    ))
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn resolve_valid_central_validation<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>(
    validated: ValidatedStoreSuccess<
        O,
        PP,
        U,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
    >,
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
    output: ManuallyDrop<T>,
    failure_work: FailureEvidenceWorkBudget,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
) -> CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(CanonicalValidationAttempt::Decision(CanonicalValidationDecision::Accept(proof))) =
        attempt
    else {
        let _ = drop_validation_attempt(attempt);
        let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
        return Err(StoreActivationContainment {
            cause: validated.contain_activation_fault(),
        });
    };
    let destinations = match ZeroCommitDestinationFacts::try_new(
        canonical_destination_dimension,
        validated.canonical_member_count(),
        outcome_evidence_destination_dimension,
        validated.outcome_evidence_member_count(),
    ) {
        Ok(destinations) => destinations,
        Err(_) => {
            let _ = drop_scoped_value(proof);
            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
            return Err(StoreActivationContainment {
                cause: validated.contain_activation_fault(),
            });
        }
    };
    Ok(CanonicalCentralValidationOutcome::Candidate(
        CanonicalCandidateStoreSession {
            destinations,
            owner: PayloadOwner::new(
                proof,
                PayloadOwner::new(ManuallyDrop::into_inner(output), validated),
            ),
            _failure_work: failure_work,
        },
    ))
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn resolve_trusted_central_rejection<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>(
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: StoreCanonicalRejection<E>,
    stable_code: u32,
    output: ManuallyDrop<T>,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
) -> CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let failure = failure.with_forced_cause(PreparationFailureCode::domain(stable_code));
    match failure.try_discard_canonical_for_activation() {
        Ok(failure) => {
            if drop_scoped_value(ManuallyDrop::into_inner(output)).is_err() {
                let _ = drop_scoped_value(cause);
                return Err(StoreActivationContainment {
                    cause: failure.contain_activation_fault(),
                });
            }
            Ok(CanonicalCentralValidationOutcome::Rejected(
                CentrallyRejectedCanonicalStoreSession {
                    root: RootedFailureEvidence {
                        owner: PayloadOwner::new(cause, failure),
                        failure_work,
                        outcome_evidence_destination_dimension,
                        _lifecycle: PhantomData::<OrdinaryFailureState>,
                    },
                },
            ))
        }
        Err(containment) => {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
            let _ = drop_scoped_value(cause);
            Err(StoreActivationContainment { cause: containment })
        }
    }
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn resolve_contract_central_rejection<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>(
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
    output: ManuallyDrop<T>,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
) -> CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let decision_drop_failed = drop_validation_attempt(attempt).is_err();
    let failure = failure.with_forced_cause(PreparationFailureCode::product_graph(
        CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE,
    ));
    let failure = match failure.try_discard_canonical_for_activation() {
        Ok(failure) => failure,
        Err(containment) => {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
            return Err(StoreActivationContainment { cause: containment });
        }
    };
    let output_drop_failed = drop_scoped_value(ManuallyDrop::into_inner(output)).is_err();
    if decision_drop_failed || output_drop_failed {
        return Err(StoreActivationContainment {
            cause: failure.contain_activation_fault(),
        });
    }
    Ok(CanonicalCentralValidationOutcome::Rejected(
        CentrallyRejectedCanonicalStoreSession {
            root: RootedFailureEvidence {
                owner: PayloadOwner::new(
                    StoreCanonicalRejection::Runtime(
                        StoreRuntimeError::CanonicalValidationContractRejected,
                    ),
                    failure,
                ),
                failure_work,
                outcome_evidence_destination_dimension,
                _lifecycle: PhantomData::<OrdinaryFailureState>,
            },
        },
    ))
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn resolve_contract_central_interruption<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>(
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: StoreWorkError,
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
    output: ManuallyDrop<T>,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
) -> CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let decision_drop_failed = drop_validation_attempt(attempt).is_err();
    let cause_drop_failed = drop_scoped_value(cause).is_err();
    if decision_drop_failed || cause_drop_failed {
        let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
        return Err(StoreActivationContainment {
            cause: failure.contain_activation_fault(),
        });
    }
    let failure = failure.with_forced_cause(PreparationFailureCode::product_graph(
        CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE,
    ));
    let failure = match failure.try_discard_canonical_for_activation() {
        Ok(failure) => failure,
        Err(containment) => {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
            return Err(StoreActivationContainment { cause: containment });
        }
    };
    if drop_scoped_value(ManuallyDrop::into_inner(output)).is_err() {
        return Err(StoreActivationContainment {
            cause: failure.contain_activation_fault(),
        });
    }
    Ok(CanonicalCentralValidationOutcome::Rejected(
        CentrallyRejectedCanonicalStoreSession {
            root: RootedFailureEvidence {
                owner: PayloadOwner::new(
                    StoreCanonicalRejection::Runtime(
                        StoreRuntimeError::CanonicalValidationContractRejected,
                    ),
                    failure,
                ),
                failure_work,
                outcome_evidence_destination_dimension,
                _lifecycle: PhantomData::<OrdinaryFailureState>,
            },
        },
    ))
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn resolve_trusted_central_interruption<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>(
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: StoreWorkError,
    attempt: Option<CanonicalValidationAttempt<Proof, E>>,
    output: ManuallyDrop<T>,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
) -> CentralValidationResult<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if drop_validation_attempt(attempt).is_err() {
        let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
        return Err(StoreActivationContainment {
            cause: failure.contain_activation_fault(),
        });
    }
    match failure.try_discard_canonical_for_activation() {
        Ok(failure) => {
            if drop_scoped_value(ManuallyDrop::into_inner(output)).is_err() {
                return Err(StoreActivationContainment {
                    cause: failure.contain_activation_fault(),
                });
            }
            Ok(CanonicalCentralValidationOutcome::Interrupted(
                InterruptedCanonicalStoreSession {
                    root: RootedFailureEvidence {
                        owner: PayloadOwner::new(cause, failure),
                        failure_work,
                        outcome_evidence_destination_dimension,
                        _lifecycle: PhantomData::<OrdinaryFailureState>,
                    },
                },
            ))
        }
        Err(containment) => {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
            Err(StoreActivationContainment { cause: containment })
        }
    }
}

impl<O, PP, U, T, CanonicalKind, CanonicalTag, L>
    ActivatedCanonicalStoreSession<O, PP, U, T, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        self.owner.value()
    }

    pub fn canonical_len(&self) -> usize {
        self.owner.storage().canonical_len()
    }

    /// Consumes the candidate through the operation-owned evidence and validation seam.
    ///
    /// The caller never receives the writable assembly. Work and destination roles come from the
    /// session owner while exact materialized shape comes from its matching keyless request.
    /// Evidence is built in a one-shot lane, then the controlled validator produces either an
    /// opaque zero-commit candidate or a failure owner. Recoverable fallback-only failures remain
    /// unprepared so callers can end borrowed input lifetimes and release charged scratch first.
    /// Any invalid root, ignored control fault, panic, or uncertain cleanup is quarantined instead
    /// of becoming a fallback result.
    #[allow(clippy::type_complexity, clippy::result_large_err)]
    pub fn validate_centrally<EvidenceKind, EvidenceTag, Proof, E>(
        self,
        progress: CanonicalValidationProgress<PP, U>,
        build_evidence: impl for<'lane> FnMut(
            &mut CanonicalOutcomeEvidenceLane<'lane, O, PP, U, L>,
            &T,
        )
            -> OutcomeEvidenceDecision<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage, 'scope, 'operation> FnMut(
            CanonicalValidationPlanning<
                'session,
                'storage,
                'scope,
                'operation,
                O,
                PP,
                U,
                CanonicalKind,
                CanonicalTag,
                EvidenceKind,
                EvidenceTag,
                Proof,
                E,
            >,
            &T,
        )
            -> Result<(), StoreWorkError>,
    ) -> Result<
        CanonicalCentralValidationOutcome<
            O,
            PP,
            U,
            T,
            Proof,
            E,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
        StoreActivationContainment,
    >
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let mut build_evidence = ManuallyDrop::new(Some(build_evidence));
        let mut validate = ManuallyDrop::new(Some(validate));
        let Self {
            owner,
            canonical_root,
            failure_work,
            validation_phase,
            validation_work_dimension,
            validation_scratch_dimension,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
            validation_work,
        } = self;
        let (output, assembly) = owner.into_parts();
        let output = ManuallyDrop::new(output);
        let mut assembly = assembly;
        let mut failure_work = failure_work;
        let work_dimension = validation_work_dimension;
        let mut evidence_fault = None;
        let evidence = catch_unwind(AssertUnwindSafe(|| {
            let mut lane = CanonicalOutcomeEvidenceLane {
                assembly: &mut assembly,
                first_fault: &mut evidence_fault,
                failure_work: &mut failure_work,
                work_dimension,
            };
            let build_evidence = build_evidence
                .as_mut()
                .expect("the outcome-evidence callback is executed at most once");
            build_evidence(&mut lane, &*output)
        }));
        let build_drop_failed = drop_optional_callback(build_evidence).is_err();
        let evidence = match evidence {
            Ok(evidence) if !build_drop_failed => evidence,
            Ok(evidence) => {
                let _ = drop_scoped_value(evidence);
                let _ = drop_optional_callback(validate);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                return Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                });
            }
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                let _ = drop_optional_callback(validate);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                return Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                });
            }
        };
        let evidence_root = match (evidence_fault, evidence) {
            (Some(fault), OutcomeEvidenceDecision::Abort) if !fault.contract_seen => {
                let validate_drop_failed = drop_optional_callback(validate).is_err();
                let output_drop_failed =
                    drop_scoped_value(ManuallyDrop::into_inner(output)).is_err();
                if validate_drop_failed || output_drop_failed {
                    return Err(StoreActivationContainment {
                        cause: assembly.contain_activation_fault(),
                    });
                }
                let Some(failure_code) = fault.recoverable_code() else {
                    return Err(StoreActivationContainment {
                        cause: assembly.contain_activation_fault(),
                    });
                };
                return defer_assembly_minimal(assembly, fault.cause, failure_code)
                    .map(CanonicalCentralValidationOutcome::Fallback);
            }
            (Some(_), _) | (None, OutcomeEvidenceDecision::Abort) => {
                let _ = drop_optional_callback(validate);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                return Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                });
            }
            (None, OutcomeEvidenceDecision::Complete(root)) => root,
        };
        if assembly.is_fallback_only() {
            let _ = drop_optional_callback(validate);
            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
            return Err(StoreActivationContainment {
                cause: assembly.contain_activation_fault(),
            });
        }

        assembly.report_progress(
            progress.phase(),
            progress.unit(),
            0,
            Some(validation_work.get()),
        );

        let mut attempt = None;
        let mut exact = None;
        let mut validation_first_fault = None;
        let validation = assembly.validate_success_controlled(
            work_dimension,
            validation_scratch_dimension,
            canonical_root,
            evidence_root,
            |session, canonical, evidence, control| {
                let validate = validate
                    .as_mut()
                    .expect("the central validation callback is executed at most once");
                let planning = CanonicalValidationPlanning::new(
                    StoreValidationSession {
                        raw: &mut *session.raw,
                    },
                    control,
                    validation_phase,
                    validation_work,
                    CanonicalValidationMember::from_store(canonical),
                    OutcomeEvidenceValidationMember::from_store(evidence),
                    &mut validation_first_fault,
                    &mut attempt,
                    &mut exact,
                );
                validate(planning, &*output)?;
                Ok(exact.unwrap_or(false))
            },
        );
        if drop_optional_callback(validate).is_err() {
            return Err(contain_central_validation_callback_drop(
                validation, attempt, output,
            ));
        }
        resolve_central_validation(
            validation,
            attempt,
            validation_first_fault,
            output,
            failure_work,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
        )
    }
}

impl<Proof, E> CanonicalValidationDecision<Proof, E> {
    pub const fn accept(proof: Proof) -> Self {
        Self::Accept(proof)
    }

    pub const fn reject(cause: E, stable_code: u32) -> Self {
        Self::Reject { cause, stable_code }
    }
}

impl<O, PP, U, T, Proof, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    CanonicalCandidateStoreSession<
        O,
        PP,
        U,
        T,
        Proof,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        self.owner.storage().value()
    }

    pub const fn proof(&self) -> &Proof {
        self.owner.value()
    }

    pub const fn canonical_member_count(&self) -> u64 {
        self.destinations.canonical_members.get()
    }

    pub const fn outcome_evidence_member_count(&self) -> u64 {
        self.destinations.outcome_evidence_members.get()
    }

    /// Seals exact destinations and final operation progress before core reserves a generation.
    #[allow(clippy::type_complexity)]
    pub fn try_issue_freeze_permit(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        CanonicalFreezePermitIssue<
            O,
            PP,
            U,
            T,
            Proof,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
        StoreActivationContainment,
    > {
        let Self {
            destinations,
            owner,
            _failure_work: failure_work,
        } = self;
        let (proof, output_owner) = owner.into_parts();
        let (output, validated) = output_owner.into_parts();
        let ZeroCommitDestinationFacts {
            _canonical_dimension: canonical_dimension,
            canonical_members,
            _outcome_evidence_dimension: outcome_evidence_dimension,
            outcome_evidence_members,
        } = destinations;
        let issued = match validated.try_issue_freeze_permit(
            canonical_dimension,
            outcome_evidence_dimension,
            final_progress,
        ) {
            Ok(issued) => issued,
            Err(cause) => {
                let _ = drop_scoped_value(proof);
                let _ = drop_scoped_value(output);
                return Err(StoreActivationContainment { cause });
            }
        };
        match issued {
            StoreSuccessFreezePermitIssue::Ready(store) => {
                Ok(CanonicalFreezePermitIssue::Ready(CanonicalFreezePermit {
                    owner: PayloadOwner::new(output, PayloadOwner::new(proof, store)),
                    failure_work,
                    destinations: ZeroCommitDestinationFacts {
                        _canonical_dimension: canonical_dimension,
                        canonical_members,
                        _outcome_evidence_dimension: outcome_evidence_dimension,
                        outcome_evidence_members,
                    },
                }))
            }
            StoreSuccessFreezePermitIssue::RichFailure {
                failure,
                cause,
                control_cause,
            } => {
                let proof_drop_failed = drop_scoped_value(proof).is_err();
                let output_drop_failed = drop_scoped_value(output).is_err();
                if proof_drop_failed || output_drop_failed {
                    return Err(StoreActivationContainment {
                        cause: failure.contain_activation_fault(),
                    });
                }
                Ok(CanonicalFreezePermitIssue::Interrupted(
                    RootedFailureEvidence {
                        owner: PayloadOwner::new(
                            CanonicalFreezeInterruption::from_failure_code(cause, control_cause),
                            failure,
                        ),
                        failure_work,
                        outcome_evidence_destination_dimension: outcome_evidence_dimension,
                        _lifecycle: PhantomData::<FrozenFailureState>,
                    },
                ))
            }
            StoreSuccessFreezePermitIssue::DestinationRejected { failure, cause } => {
                let proof_drop_failed = drop_scoped_value(proof).is_err();
                let output_drop_failed = drop_scoped_value(output).is_err();
                if proof_drop_failed || output_drop_failed {
                    return Err(StoreActivationContainment {
                        cause: failure.contain_activation_fault(),
                    });
                }
                Ok(CanonicalFreezePermitIssue::DestinationRejected(
                    RootedFailureEvidence {
                        owner: PayloadOwner::new(
                            CanonicalFreezeInterruption::from_failure_code(cause, None),
                            failure,
                        ),
                        failure_work,
                        outcome_evidence_destination_dimension: outcome_evidence_dimension,
                        _lifecycle: PhantomData::<OrdinaryFailureState>,
                    },
                ))
            }
        }
    }
}

impl CanonicalFreezeInterruption {
    fn from_failure_code(
        cause: PreparationFailureCode,
        control_cause: Option<ControlCause>,
    ) -> Self {
        let class = match cause.class() {
            PreparationFailureClass::Control => CanonicalFreezeInterruptionClass::Control,
            PreparationFailureClass::Ledger => CanonicalFreezeInterruptionClass::Ledger,
            PreparationFailureClass::Requested | PreparationFailureClass::ProductGraph => {
                CanonicalFreezeInterruptionClass::ProductGraph
            }
        };
        Self {
            class,
            stable_code: cause.code(),
            control_cause,
        }
    }

    pub const fn class(&self) -> CanonicalFreezeInterruptionClass {
        self.class
    }

    pub const fn stable_code(&self) -> u32 {
        self.stable_code
    }

    pub const fn control_cause(&self) -> Option<&ControlCause> {
        self.control_cause.as_ref()
    }
}

impl<O, PP, U, T, Proof, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    CanonicalFreezePermit<
        O,
        PP,
        U,
        T,
        Proof,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Borrows the exact candidate output frozen into this permit.
    ///
    /// Callers can validate descriptor pairing before generation admission without accepting a
    /// second, independently movable copy of the output.
    pub const fn output(&self) -> &T {
        self.owner.value()
    }

    /// Contains an unexpected failure of the core-owned generation admission boundary.
    ///
    /// Ordinary generation exhaustion uses `reject_generation`; this consumes the complete
    /// frozen owner only after the generation source itself violated its execution contract.
    pub fn contain_generation_fault(self) -> StoreActivationContainment {
        let Self {
            owner,
            failure_work: _,
            destinations: _,
        } = self;
        let (output, proof_owner) = owner.into_parts();
        let (proof, store) = proof_owner.into_parts();
        let _ = drop_scoped_value(output);
        let _ = drop_scoped_value(proof);
        StoreActivationContainment {
            cause: store.contain_activation_fault(),
        }
    }

    pub fn destination_counters_are_zero(&self) -> bool {
        let canonical = self
            .owner
            .storage()
            .storage()
            .terminal_observation(self.destinations._canonical_dimension)
            .expect("the sealed canonical destination remains registered");
        let evidence = self
            .owner
            .storage()
            .storage()
            .terminal_observation(self.destinations._outcome_evidence_dimension)
            .expect("the sealed outcome-evidence destination remains registered");
        [canonical, evidence].into_iter().all(|snapshot| {
            snapshot.pending_publication() == 0
                && snapshot.committed() == 0
                && snapshot.committed_canonical_output() == 0
                && snapshot.published_outcome_evidence() == 0
        })
    }

    #[allow(
        clippy::type_complexity,
        reason = "The typestate result carries the frozen failure state and its exact cause brand."
    )]
    pub fn reject_generation<E>(
        self,
        cause: E,
        stable_code: u32,
    ) -> Result<
        CentrallyRejectedCanonicalStoreSession<
            O,
            PP,
            U,
            E,
            EvidenceKind,
            EvidenceTag,
            FrozenFailureState,
        >,
        StoreActivationContainment,
    > {
        let Self {
            owner,
            failure_work,
            destinations,
        } = self;
        let (output, proof_owner) = owner.into_parts();
        let (proof, store) = proof_owner.into_parts();
        let cause = ManuallyDrop::new(cause);
        let proof_drop_failed = drop_scoped_value(proof).is_err();
        let output_drop_failed = drop_scoped_value(output).is_err();
        if proof_drop_failed || output_drop_failed {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            return Err(StoreActivationContainment {
                cause: store.contain_activation_fault(),
            });
        }
        let failure = match store.reject_generation(stable_code) {
            Ok(failure) => failure,
            Err(containment) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment { cause: containment });
            }
        };
        let cause = ManuallyDrop::into_inner(cause);
        Ok(CentrallyRejectedCanonicalStoreSession {
            root: RootedFailureEvidence {
                owner: PayloadOwner::new(StoreCanonicalRejection::Requested(cause), failure),
                failure_work,
                outcome_evidence_destination_dimension: destinations._outcome_evidence_dimension,
                _lifecycle: PhantomData::<FrozenFailureState>,
            },
        })
    }

    /// Opens one unforgeable joint-commit scope.
    ///
    /// The higher-ranked callback prevents either branded capability from escaping. The actual
    /// typed output moves into the core handoff together with the proof; its retained owners keep
    /// their physical leases through success, rejection, and ordinary caller unwind.
    #[allow(clippy::type_complexity)]
    pub fn bind_joint_commit<R>(
        self,
        bind: impl for<'joint> FnOnce(
            T,
            Proof,
            CanonicalJointCommit<
                'joint,
                O,
                PP,
                U,
                CanonicalKind,
                CanonicalTag,
                EvidenceKind,
                EvidenceTag,
            >,
            CanonicalJointSeal<'joint>,
        ) -> R,
    ) -> R
    where
        Proof: Copy,
    {
        let Self {
            owner,
            failure_work,
            destinations,
        } = self;
        let (output, proof_owner) = owner.into_parts();
        let (proof, store) = proof_owner.into_parts();
        bind(
            output,
            proof,
            CanonicalJointCommit {
                _failure_work: failure_work,
                _destinations: destinations,
                joint: PhantomData,
                store: store.into_contribution(),
            },
            CanonicalJointSeal { joint: PhantomData },
        )
    }
}

impl<'joint, O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    CanonicalJointCommit<'joint, O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Consumes the operation contribution and the seal from the same binder scope.
    ///
    /// The exact operation preparation outcome is returned without attaching a generation token.
    /// A defensive late prepared failure remains a first-class outcome.
    pub fn prepare(
        self,
        seal: CanonicalJointSeal<'joint>,
    ) -> Result<
        PreparedStoreSuccessOutcome<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    > {
        let Self {
            _failure_work: _,
            _destinations: _,
            joint: _,
            store,
        } = self;
        let CanonicalJointSeal { joint: _ } = seal;
        store
            .prepare()
            .map_err(|cause| StoreActivationContainment { cause })
    }
}

#[allow(
    private_bounds,
    reason = "public failure typestates are sealed by the crate-private lifecycle contract"
)]
impl<O, PP, U, E, EvidenceKind, EvidenceTag, State>
    CentrallyRejectedCanonicalStoreSession<O, PP, U, E, EvidenceKind, EvidenceTag, State>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    State: FailureLifecycleState,
{
    pub const fn cause(&self) -> &StoreCanonicalRejection<E> {
        self.root.cause()
    }

    pub const fn canonical_len(&self) -> usize {
        0
    }

    #[allow(
        clippy::type_complexity,
        reason = "The typestate result preserves the rejection cause and lifecycle state."
    )]
    pub fn validate_rich_failure(
        self,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        CanonicalRejectionFailureValidation<O, PP, U, E, EvidenceKind, EvidenceTag, State>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.root.validate_rich_failure(validate)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreCanonicalRejection<E>>, StorePrepareError> {
        self.root.prepare_minimal_fallback()
    }
}

impl<O, PP, U, EvidenceKind, EvidenceTag>
    InterruptedCanonicalStoreSession<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreWorkError {
        self.root.cause()
    }

    pub fn validate_rich_failure(
        self,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        RootedFailureValidation<O, PP, U, StoreWorkError, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.root.validate_rich_failure(validate)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreWorkError>, StorePrepareError> {
        self.root.prepare_minimal_fallback()
    }
}

struct CanonicalEvidenceMemberBuild<Tag, CreateWriter, FinishWriter> {
    allocation: crate::retained::RetainedAllocationPlan,
    construction_work: u64,
    action_work: u64,
    tag: Tag,
    missing_builder: &'static str,
    create_writer: CreateWriter,
    finish_writer: FinishWriter,
}

fn recover_canonical_evidence_build<T>(
    result: Result<T, EvidenceBuildCallbackError>,
    controlled_failure: &mut Option<StoreRuntimeError>,
) -> Result<T, crate::retained::RetainedBuildFailureCode> {
    match result {
        Ok(value) => Ok(value),
        Err(EvidenceBuildCallbackError::Build(cause)) => Err(cause.into()),
        Err(EvidenceBuildCallbackError::Runtime(cause)) => {
            *controlled_failure = Some(cause);
            Err(super::failure_evidence::failure_work_build_sentinel().into())
        }
    }
}

impl<O, PP, U, L> CanonicalOutcomeEvidenceLane<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn qualify_failure_validation_member(
        &mut self,
        amount: u64,
    ) -> Result<FailureEvidenceWorkBudget, StoreRuntimeError> {
        match (*self.failure_work).with_existing_member_validation(amount) {
            Ok(qualified) => Ok(qualified),
            Err(cause) => {
                self.assembly
                    .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
                Err(cause)
            }
        }
    }

    fn qualify_evidence_member(
        &mut self,
        allocation: Result<crate::retained::RetainedAllocationPlan, StorePlanError>,
        units: usize,
    ) -> Result<
        (
            crate::retained::RetainedAllocationPlan,
            u64,
            FailureEvidenceWorkBudget,
        ),
        StoreRuntimeError,
    > {
        let allocation = allocation.map_err(StoreRuntimeError::Plan)?;
        let construction_work = u64::try_from(units)
            .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let qualified = self.qualify_failure_validation_member(construction_work)?;
        Ok((allocation, construction_work, qualified))
    }

    fn try_retain_evidence_member<Kind, W, Tag, Build, CreateWriter, FinishWriter>(
        &mut self,
        member: CanonicalEvidenceMemberBuild<Tag, CreateWriter, FinishWriter>,
        build: &mut ManuallyDrop<Option<Build>>,
    ) -> Result<OutcomeEvidenceStoreRef<Kind, Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
        Build: for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, W>,
        ) -> Result<(), StoreBuildError>,
        CreateWriter: FnOnce() -> Result<W, StoreBuildError>,
        FinishWriter: FnOnce(W) -> Result<StoredBlock, StoreBuildError>,
    {
        let CanonicalEvidenceMemberBuild {
            allocation,
            construction_work,
            action_work,
            tag,
            missing_builder,
            create_writer,
            finish_writer,
        } = member;
        let mut controlled_failure = None;
        let result = self
            .assembly
            .try_retain_success_outcome_evidence_controlled(
                self.work_dimension,
                allocation,
                |emplacer, raw_control| {
                    let mut control = recover_canonical_evidence_build(
                        OutcomeEvidenceBuildControl::try_start_ordinary(
                            raw_control,
                            construction_work,
                            action_work,
                            create_writer,
                        ),
                        &mut controlled_failure,
                    )?;
                    let built = build.as_mut().expect(missing_builder)(&mut control);
                    let block = recover_canonical_evidence_build(
                        control.finish(built, finish_writer),
                        &mut controlled_failure,
                    )?;
                    Ok(emplacer.place(block, tag))
                },
            );
        controlled_failure.map_or(result, Err)
    }

    pub fn try_retain_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, SequenceWriter<T>>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let (allocation, construction_work, qualified) =
            match self.qualify_evidence_member(plan.allocation_plan(), plan.element_count()) {
                Ok(qualified) => qualified,
                Err(cause) => {
                    return preserve_first_fault(
                        self.first_fault,
                        finish_scoped_callback(build, Err(cause)),
                    );
                }
            };
        let result = self.try_retain_evidence_member::<ScalarSequence<T>, _, _, _, _, _>(
            CanonicalEvidenceMemberBuild {
                allocation,
                construction_work,
                action_work: 1,
                tag,
                missing_builder: "the outcome-evidence sequence builder is executed at most once",
                create_writer: || {
                    <T as super::super::sealed::StoredScalar>::new_writer(plan.capacity, plan.len)
                },
                finish_writer: SequenceWriter::finish,
            },
            &mut build,
        );
        let result = finish_scoped_callback(build, result);
        if result.is_ok() {
            *self.failure_work = qualified;
        }
        preserve_first_fault(self.first_fault, result)
    }

    pub fn try_retain_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, BytesWriter>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let (allocation, construction_work, qualified) =
            match self.qualify_evidence_member(plan.allocation_plan(), plan.bytes()) {
                Ok(qualified) => qualified,
                Err(cause) => {
                    return preserve_first_fault(
                        self.first_fault,
                        finish_scoped_callback(build, Err(cause)),
                    );
                }
            };
        let result = self.try_retain_evidence_member::<ByteBlock, _, _, _, _, _>(
            CanonicalEvidenceMemberBuild {
                allocation,
                construction_work,
                action_work: 1,
                tag,
                missing_builder: "the outcome-evidence bytes builder is executed at most once",
                create_writer: || BytesWriter::new(plan),
                finish_writer: BytesWriter::finish,
            },
            &mut build,
        );
        let result = finish_scoped_callback(build, result);
        if result.is_ok() {
            *self.failure_work = qualified;
        }
        preserve_first_fault(self.first_fault, result)
    }

    pub fn try_retain_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, Utf8Writer>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let (allocation, construction_work, qualified) =
            match self.qualify_evidence_member(plan.allocation_plan(), plan.bytes()) {
                Ok(qualified) => qualified,
                Err(cause) => {
                    return preserve_first_fault(
                        self.first_fault,
                        finish_scoped_callback(build, Err(cause)),
                    );
                }
            };
        let result = self.try_retain_evidence_member::<Utf8Block, _, _, _, _, _>(
            CanonicalEvidenceMemberBuild {
                allocation,
                construction_work,
                action_work: 1,
                tag,
                missing_builder: "the outcome-evidence UTF-8 builder is executed at most once",
                create_writer: || Utf8Writer::new(plan),
                finish_writer: Utf8Writer::finish,
            },
            &mut build,
        );
        let result = finish_scoped_callback(build, result);
        if result.is_ok() {
            *self.failure_work = qualified;
        }
        preserve_first_fault(self.first_fault, result)
    }

    pub fn try_retain_records<Tag>(
        &mut self,
        plan: RecordPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, OutcomeEvidenceRecordWriter>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let cells = match plan.rows().checked_mul(plan.fields()) {
            Some(cells) => cells,
            None => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(
                        build,
                        Err(StoreRuntimeError::Plan(StorePlanError::SizeOverflow)),
                    ),
                );
            }
        };
        let action_work =
            u64::try_from(plan.fields()).expect("the fixed record field limit fits in u64");
        let (allocation, construction_work, qualified) =
            match self.qualify_evidence_member(plan.allocation_plan(), cells) {
                Ok(qualified) => qualified,
                Err(cause) => {
                    return preserve_first_fault(
                        self.first_fault,
                        finish_scoped_callback(build, Err(cause)),
                    );
                }
            };
        let result = self.try_retain_evidence_member::<RecordBlock, _, _, _, _, _>(
            CanonicalEvidenceMemberBuild {
                allocation,
                construction_work,
                action_work,
                tag,
                missing_builder: "the outcome-evidence record builder is executed at most once",
                create_writer: || OutcomeEvidenceRecordWriter::new(plan),
                finish_writer: OutcomeEvidenceRecordWriter::finish,
            },
            &mut build,
        );
        let result = finish_scoped_callback(build, result);
        if result.is_ok() {
            *self.failure_work = qualified;
        }
        preserve_first_fault(self.first_fault, result)
    }

    pub const fn complete<Kind, Tag>(
        &mut self,
        root: OutcomeEvidenceStoreRef<Kind, Tag>,
    ) -> OutcomeEvidenceDecision<Kind, Tag> {
        OutcomeEvidenceDecision::Complete(root)
    }

    pub const fn abort<Kind, Tag>(&mut self) -> OutcomeEvidenceDecision<Kind, Tag> {
        OutcomeEvidenceDecision::Abort
    }
}
