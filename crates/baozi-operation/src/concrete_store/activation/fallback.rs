use super::*;

impl<O, PP, U, E, L> NeverActivatedStoreSession<O, PP, U, E, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn requested(
        mut operation: RetainedStoreOperation<O, PP, U, L>,
        outcome_evidence_plan: RetainedStoreBlockTablePlan,
        failure_work: FailureEvidenceWorkBudget,
        outcome_evidence_destination_dimension: StoreDimensionKey,
        rejection: VerifiedRejection<E>,
    ) -> Self {
        operation.latch_activation_terminal_cause(rejection.failure_code);
        Self {
            owner: NeverActivatedOwner {
                cause: Some(StoreNeverActivatedCause::Requested(rejection.cause)),
                authority: Some(NeverActivatedAuthority::Operation(operation)),
            },
            failure_code: rejection.failure_code,
            outcome_evidence_plan,
            failure_work,
            outcome_evidence_destination_dimension,
        }
    }

    pub(super) fn runtime(
        mut operation: RetainedStoreOperation<O, PP, U, L>,
        outcome_evidence_plan: RetainedStoreBlockTablePlan,
        failure_work: FailureEvidenceWorkBudget,
        outcome_evidence_destination_dimension: StoreDimensionKey,
        cause: StoreRuntimeError,
        failure_code: PreparationFailureCode,
        fallback_only: bool,
    ) -> Self {
        operation.latch_activation_terminal_cause(failure_code);
        Self {
            owner: NeverActivatedOwner {
                cause: Some(StoreNeverActivatedCause::Runtime(cause)),
                authority: Some(if fallback_only {
                    NeverActivatedAuthority::FallbackOnly(operation)
                } else {
                    NeverActivatedAuthority::Operation(operation)
                }),
            },
            failure_code,
            outcome_evidence_plan,
            failure_work,
            outcome_evidence_destination_dimension,
        }
    }

    pub fn cause(&self) -> &StoreNeverActivatedCause<E> {
        self.owner
            .cause
            .as_ref()
            .expect("a live session retains its cause")
    }

    #[allow(clippy::type_complexity, clippy::result_large_err)]
    pub fn build_failure_evidence<EvidenceKind, EvidenceTag>(
        self,
        build: impl for<'lane> FnMut(
            &mut FailureEvidenceLane<'lane, O, PP, U>,
        ) -> FailureEvidenceDecision<EvidenceKind, EvidenceTag>,
    ) -> Result<
        FailureEvidenceOutcome<O, PP, U, StoreNeverActivatedCause<E>, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let build = ManuallyDrop::new(Some(build));
        let Self {
            owner,
            failure_code,
            outcome_evidence_plan,
            failure_work,
            outcome_evidence_destination_dimension,
        } = self;
        let (cause, authority) = owner.into_parts();
        match authority {
            NeverActivatedAuthority::FallbackOnly(operation) => {
                if drop_optional_callback(build).is_err() {
                    let _ = drop_scoped_value(cause);
                    return Err(contain_operation(operation));
                }
                prepare_operation_minimal(operation, failure_code, cause)
                    .map(FailureEvidenceOutcome::MinimalFallback)
            }
            NeverActivatedAuthority::Operation(operation) => {
                let plan = RetainedStorePublicationPlan::rich_failure(outcome_evidence_plan);
                match try_open_activation(operation, plan) {
                    Ok(assembly) => {
                        let RetainedStorePublicationAssembly {
                            raw,
                            fallback_cause,
                            success_rejection,
                            domain,
                            lineage,
                        } = assembly;
                        if fallback_cause.is_some() || success_rejection.is_some() {
                            let _ = drop_optional_callback(build);
                            let _ = drop_scoped_value(cause);
                            return Err(StoreActivationContainment {
                                cause: raw.contain_activation_fault(),
                            });
                        }
                        let raw = match raw.into_verified_failure_only(failure_code) {
                            Ok(raw) => raw,
                            Err(containment) => {
                                let _ = drop_optional_callback(build);
                                let _ = drop_scoped_value(cause);
                                let _ = drop_scoped_value(lineage);
                                return Err(StoreActivationContainment { cause: containment });
                            }
                        };
                        if drop_scoped_value(lineage).is_err() {
                            let _ = drop_optional_callback(build);
                            let _ = drop_scoped_value(cause);
                            return Err(StoreActivationContainment {
                                cause: raw.contain_activation_fault(),
                            });
                        }
                        UnrootedFailureEvidence {
                            raw,
                            domain,
                            failure_code,
                            cause,
                            failure_work,
                            outcome_evidence_destination_dimension,
                        }
                        .build(
                            ManuallyDrop::into_inner(build)
                                .expect("the failure-evidence callback is transferred once"),
                        )
                    }
                    Err(ActivationOpenFailure::DeferredContainment { operation }) => {
                        let _ = drop_optional_callback(build);
                        let _ = drop_scoped_value(cause);
                        Err(contain_operation(operation))
                    }
                    Err(ActivationOpenFailure::Recoverable {
                        operation,
                        cause: _,
                        failure_code: _,
                        fallback_only: _,
                    }) => {
                        if drop_optional_callback(build).is_err() {
                            let _ = drop_scoped_value(cause);
                            return Err(contain_operation(operation));
                        }
                        prepare_operation_minimal(operation, failure_code, cause)
                            .map(FailureEvidenceOutcome::MinimalFallback)
                    }
                }
            }
        }
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreNeverActivatedCause<E>>, StorePrepareError> {
        let Self {
            owner,
            failure_code,
            outcome_evidence_plan: _,
            failure_work: _,
            outcome_evidence_destination_dimension: _,
        } = self;
        let (cause, authority) = owner.into_parts();
        let operation = match authority {
            NeverActivatedAuthority::Operation(operation)
            | NeverActivatedAuthority::FallbackOnly(operation) => operation,
        };
        let cause = ManuallyDrop::new(cause);
        match prepare_operation_minimal_direct(operation, failure_code) {
            Ok(prepared) => Ok(VerifiedMinimalFallback::new(
                prepared,
                ManuallyDrop::into_inner(cause),
            )),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }
}

impl StoreActivationContainment {
    pub const fn cause(self) -> FinalizationContainmentError {
        self.cause
    }
}

impl fmt::Display for StoreActivationContainment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl std::error::Error for StoreActivationContainment {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
