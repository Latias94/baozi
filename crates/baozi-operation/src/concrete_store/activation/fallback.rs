use super::final_state::CanonicalFinal;
use super::*;

impl<'inv, O, PP, U> PendingMinimalFallback<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn new(
        fallback: StoreFallbackOnly<'inv, O, PP, U>,
        cause_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
        cause: StoreRuntimeError,
    ) -> Result<Self, StoreActivationContainment> {
        match cause_slot.place(cause) {
            Ok(cause) => Ok(Self {
                owner: PayloadOwner::new(cause, fallback),
            }),
            Err(cause) => {
                let _ = drop_scoped_value(cause);
                Err(StoreActivationContainment {
                    cause: fallback.contain_activation_fault(),
                })
            }
        }
    }

    /// The original evidence-construction or validation failure, before terminal preparation.
    pub const fn cause(&self) -> &StoreRuntimeError {
        self.owner.value().value()
    }

    /// Discards candidate storage and freezes the pre-admitted fallback after caller cleanup.
    ///
    /// Caller-owned ordinary scratch reservations must be released first. Valid retained-output
    /// reservations may remain live under their existing protocol. Preparation uses the existing
    /// fallback owner without a cleanup callback, payload copy, allocation, or control poll.
    pub(super) fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreRuntimeError>, StorePrepareError> {
        let Self { owner } = self;
        let (cause, fallback) = owner.into_parts();
        let cause = ManuallyDrop::new(cause);
        match fallback.prepare_minimal_fallback() {
            Ok(prepared) => Ok(VerifiedMinimalFallback::new(
                prepared,
                ManuallyDrop::into_inner(cause).into_owned(),
            )),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }
}

impl<'inv, O, PP, U> CanonicalFinal<PendingMinimalFallback<'inv, O, PP, U>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// The original evidence-construction or validation failure, before terminal preparation.
    pub const fn cause(&self) -> &StoreRuntimeError {
        self.state.cause()
    }

    /// Discards candidate storage and freezes the pre-admitted fallback after caller cleanup.
    ///
    /// Caller-owned ordinary scratch reservations must be released first. Valid retained-output
    /// reservations may remain live under their existing protocol. Preparation uses the existing
    /// fallback owner without a cleanup callback, payload copy, allocation, or control poll.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreRuntimeError>, StorePrepareError> {
        self.state.prepare_minimal_fallback()
    }
}

impl<'inv, O, PP, U, E, L> NeverActivatedStoreSession<'inv, O, PP, U, E, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn cause(&self) -> &StoreNeverActivatedCause<E> {
        self.owner
            .cause
            .as_ref()
            .expect("a live session retains its cause")
    }
}

impl<'inv, O, PP, U, E, L> CanonicalFinal<NeverActivatedStoreSession<'inv, O, PP, U, E, L>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn cause(&self) -> &StoreNeverActivatedCause<E> {
        self.state.cause()
    }

    #[allow(clippy::type_complexity, clippy::result_large_err)]
    pub fn build_failure_evidence<EvidenceKind, EvidenceTag>(
        self,
        build: impl for<'evidence, 'lane> FnMut(
            &mut FailureEvidenceLane<'evidence, 'lane, O, PP, U>,
        )
            -> FailureEvidenceDecision<EvidenceKind, EvidenceTag>,
    ) -> Result<
        FailureEvidenceOutcome<
            'inv,
            O,
            PP,
            U,
            StoreNeverActivatedCause<E>,
            EvidenceKind,
            EvidenceTag,
        >,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let build = ManuallyDrop::new(Some(build));
        let NeverActivatedStoreSession {
            owner,
            failure_code,
            outcome_evidence_plan,
            failure_work,
            outcome_evidence_destination_dimension,
        } = self.state;
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
                            state,
                            domain,
                            lineage,
                        } = assembly;
                        if state.fallback_cause.is_some() || state.success_rejection.is_some() {
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
                            owner: PayloadOwner::new(cause, raw),
                            domain,
                            state,
                            failure_code,
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
                    Err(ActivationOpenFailure::Recoverable { operation }) => {
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
        let NeverActivatedStoreSession {
            owner,
            failure_code,
            outcome_evidence_plan: _,
            failure_work: _,
            outcome_evidence_destination_dimension: _,
        } = self.state;
        let (cause, authority) = owner.into_parts();
        let operation = match authority {
            NeverActivatedAuthority::Operation(operation)
            | NeverActivatedAuthority::FallbackOnly(operation) => operation,
        };
        let cause = ManuallyDrop::new(cause);
        match prepare_operation_minimal_direct(operation, failure_code) {
            Ok(prepared) => Ok(VerifiedMinimalFallback::new(
                prepared,
                ManuallyDrop::into_inner(cause).into_owned(),
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
