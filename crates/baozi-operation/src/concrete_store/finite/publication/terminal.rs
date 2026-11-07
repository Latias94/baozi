use super::*;

type SuccessPreparationResult<O, PP, U, CK, CT, EK, ET> =
    Result<PreparedFiniteStoreSuccessOutcome<CK, CT, EK, ET>, FiniteStoreExitError<O, PP, U>>;

/// An exact success proof alongside the original finite terminal fields.
#[must_use = "validated finite success must be prepared or deliberately dropped"]
pub struct ValidatedFiniteStoreSuccess<'inv, O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::finite) fields: FiniteStoreFinalFields<'inv, O, PP, U>,
    pub(in crate::concrete_store::finite) proof:
        RetainedSuccessClosureProof<StoredBlock, StoredBlock>,
    pub(in crate::concrete_store::finite) canonical_root: CanonicalStoreRef<CK, CT>,
    pub(in crate::concrete_store::finite) evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
}

/// Rejected success retains evidence and finite final authority without canonical write access.
#[must_use = "finite failure assembly must be validated or published as fallback"]
pub struct FiniteStoreFailureAssembly<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::finite) fields: FiniteStoreFinalFields<'inv, O, PP, U>,
    pub(in crate::concrete_store::finite) state: StorePublicationState,
    pub(in crate::concrete_store::finite) failure_cause: PreparationFailureCode,
    pub(in crate::concrete_store::finite) forced_cause: Option<PreparationFailureCode>,
    pub(in crate::concrete_store::finite) evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    pub(in crate::concrete_store::finite) error: Option<StorePrepareError>,
}

/// An exact evidence proof alongside the original finite terminal fields.
#[must_use = "validated finite failure must be prepared or deliberately dropped"]
pub struct ValidatedFiniteStoreFailure<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fields: FiniteStoreFinalFields<'inv, O, PP, U>,
    proof: RetainedEvidenceClosureProof<StoredBlock>,
    forced_cause: Option<PreparationFailureCode>,
    evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
}

/// Terminal-only finite authority preserves the original fallback provenance and fields.
#[must_use = "finite fallback-only authority must be prepared or deliberately dropped"]
pub struct FiniteStoreFallbackOnly<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::finite) fields: FiniteStoreFinalFields<'inv, O, PP, U>,
    pub(in crate::concrete_store::finite) cause: PreparationFailureCode,
    pub(in crate::concrete_store::finite) provenance: ordinary::StoreSuccessFallbackProvenance,
    pub(in crate::concrete_store::finite) error: Option<StorePrepareError>,
}

impl<'inv, O, PP, U, CK, CT, EK, ET> ValidatedFiniteStoreSuccess<'inv, O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn from_bound_fields(
        fields: FiniteStoreFinalFields<'inv, O, PP, U>,
        proof: RetainedSuccessClosureProof<StoredBlock, StoredBlock>,
        canonical_root: CanonicalStoreRef<CK, CT>,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    ) -> Self {
        Self {
            fields,
            proof,
            canonical_root,
            evidence_root,
        }
    }

    pub(in crate::concrete_store) fn into_request_error(self) -> FiniteStoreExitError<O, PP, U> {
        self.fields.into_request_error(None)
    }

    /// Runs original terminal classification, control, detach and commit exactly once.
    pub fn try_prepare(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> SuccessPreparationResult<O, PP, U, CK, CT, EK, ET> {
        let Self {
            mut fields,
            proof,
            canonical_root,
            evidence_root,
        } = self;
        match fields
            .raw
            .finish_assembly(RetainedTerminalRequest::Success {
                proof,
                final_progress,
            }) {
            Ok((raw, _control)) => Ok(terminal_output::finite::classify_success(
                raw,
                canonical_root,
                evidence_root,
                fields.take_domain(),
            )),
            Err(failure) => Err(fields.terminal_error(failure)),
        }
    }

    /// Rejects a validated candidate while preserving its exact evidence root.
    pub fn downgrade_to_rich_failure(
        self,
        cause: u32,
    ) -> FiniteStoreFailureAssembly<'inv, O, PP, U, EK, ET> {
        let Self {
            mut fields,
            proof: _,
            canonical_root: _,
            evidence_root,
        } = self;
        fields
            .assembly_borrow()
            .loan_mut()
            .activate_failure_terminal_work();
        let cause = PreparationFailureCode::domain(cause);
        FiniteStoreFailureAssembly {
            fields,
            state: StorePublicationState::default(),
            failure_cause: cause,
            forced_cause: Some(cause),
            evidence_root,
            error: None,
        }
    }

    /// Discards a validated success through the original finite assembly finish.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        self.fields
            .prepare_publication_fallback(PreparationFailureCode::domain(0), None)
    }
}

impl<'inv, O, PP, U, PreservedKind, PreservedTag>
    FiniteStoreFailureAssembly<'inv, O, PP, U, PreservedKind, PreservedTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn from_bound_fields(
        fields: FiniteStoreFinalFields<'inv, O, PP, U>,
        state: StorePublicationState,
        failure_cause: PreparationFailureCode,
        forced_cause: Option<PreparationFailureCode>,
        evidence_root: OutcomeEvidenceStoreRef<PreservedKind, PreservedTag>,
        error: Option<StorePrepareError>,
    ) -> Self {
        Self {
            fields,
            state,
            failure_cause,
            forced_cause,
            evidence_root,
            error,
        }
    }

    pub(in crate::concrete_store) fn into_request_error(self) -> FiniteStoreExitError<O, PP, U> {
        self.fields.into_request_error(self.error)
    }

    pub(in crate::concrete_store) fn evidence_loan(
        &mut self,
    ) -> Result<ordinary::StoreFailureEvidenceLoan<'_, O, PP, U>, StoreRuntimeError> {
        if self.error.is_some() || self.state.fallback_cause.is_some() {
            return Err(StoreRuntimeError::FallbackOnly);
        }
        let raw = RetainedPublicationAssemblyBorrow::from_loan(
            self.fields
                .raw
                .assembly_loan()
                .expect("original finite assembly"),
        );
        Ok(ordinary::StoreFailureEvidenceLoan::new(
            raw,
            self.fields.domain.as_ref().expect("original finite domain"),
            &mut self.state,
        ))
    }

    /// Returns the exact evidence root retained across success rejection.
    pub const fn evidence_root(&self) -> OutcomeEvidenceStoreRef<PreservedKind, PreservedTag> {
        self.evidence_root
    }

    /// Returns the current evidence table length from its original fields.
    pub fn outcome_evidence_len(&mut self) -> usize {
        self.fields.assembly_borrow().loan().outcome_evidence_len()
    }

    /// Revalidates the exact evidence closure without exposing canonical publication work.
    pub fn validate_rich_failure<EK, ET>(
        self,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreEvidenceValidationSession<'session, 'storage>,
            OutcomeEvidenceStoreRef<EK, ET>,
        ) -> bool,
    ) -> FiniteStoreFailureValidation<'inv, O, PP, U, EK, ET>
    where
        ET: Copy + Send + Sync + 'static,
    {
        self.validate_evidence_with(evidence_root, |ordinary| {
            ordinary.validate_evidence_decision(evidence_root.raw, |raw, _| {
                let mut session = StoreEvidenceValidationSession { raw };
                validate(&mut session, evidence_root)
            })
        })
    }

    fn validate_evidence_with<EK, ET>(
        mut self,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
        decide: impl for<'op> FnOnce(
            &mut RawStoreAssemblyBorrow<'op, O, PP, U>,
        ) -> crate::retained::RetainedAssemblyDecision<
            StoredBlock,
            StoredBlock,
        >,
    ) -> FiniteStoreFailureValidation<'inv, O, PP, U, EK, ET>
    where
        ET: Copy + Send + Sync + 'static,
    {
        let validation = catch_unwind(AssertUnwindSafe(|| {
            if let Some(cause) = self
                .state
                .fallback_cause
                .or(self.error.map(|_| self.failure_cause))
            {
                drop(decide);
                return Err((
                    cause,
                    ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                ));
            }
            let mut evidence = self
                .evidence_loan()
                .expect("healthy finite evidence fields");
            let ordinary = &mut evidence.publication_impl().raw;
            let decision = decide(ordinary);
            ordinary
                .loan_mut()
                .bind_evidence_proof(decision)
                .map_err(|_decision| {
                    (
                        PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE),
                        ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                    )
                })
        }));
        let Self {
            mut fields,
            state,
            failure_cause,
            forced_cause,
            evidence_root: _,
            error,
        } = self;
        let validation = match validation {
            Ok(validation) => validation,
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                let error = Some(error.unwrap_or_else(|| {
                    StorePrepareError::FinalizationContainment(fields.contain_publication_fault())
                }));
                return FiniteStoreFailureValidation::FallbackOnly(FiniteStoreFallbackOnly {
                    fields,
                    cause: failure_cause,
                    provenance: ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                    error,
                });
            }
        };
        bind_failure_validation(
            fields,
            state,
            validation,
            evidence_root,
            forced_cause,
            error,
        )
    }

    /// Discards rejected-success evidence and prepares the pre-admitted fallback.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        self.prepare_minimal_fallback_with(0)
    }

    /// Preserves the forced success rejection before using a requested domain cause.
    pub fn prepare_minimal_fallback_with(
        self,
        cause: u32,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        let Self {
            mut fields,
            state,
            forced_cause,
            error,
            ..
        } = self;
        let cause = forced_cause
            .or(state.fallback_cause)
            .unwrap_or(PreparationFailureCode::domain(cause));
        if error.is_none() && state.fallback_cause.is_none() {
            fields
                .assembly_borrow()
                .loan_mut()
                .record_terminal_cause(cause);
        }
        fields.prepare_publication_fallback(cause, error)
    }
}

type EvidenceResolution = Result<
    RetainedClosureValidation<RetainedEvidenceClosureProof<StoredBlock>>,
    (
        PreparationFailureCode,
        ordinary::StoreSuccessFallbackProvenance,
    ),
>;

pub(super) fn bind_failure_validation<'inv, O, PP, U, EK, ET>(
    mut fields: FiniteStoreFinalFields<'inv, O, PP, U>,
    state: StorePublicationState,
    validation: EvidenceResolution,
    evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    forced_cause: Option<PreparationFailureCode>,
    error: Option<StorePrepareError>,
) -> FiniteStoreFailureValidation<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let validation = match validation {
        Ok(validation) => validation,
        Err((cause, provenance)) => {
            return FiniteStoreFailureValidation::FallbackOnly(FiniteStoreFallbackOnly {
                fields,
                cause,
                provenance,
                error,
            });
        }
    };
    if error.is_some() {
        return FiniteStoreFailureValidation::FallbackOnly(FiniteStoreFallbackOnly {
            fields,
            cause: PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE),
            provenance: ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
            error,
        });
    }
    match validation {
        RetainedClosureValidation::Valid(proof) => {
            FiniteStoreFailureValidation::Valid(ValidatedFiniteStoreFailure {
                fields,
                proof,
                forced_cause,
                evidence_root,
            })
        }
        RetainedClosureValidation::Rejected { cause } => {
            if let Some(cause) = forced_cause {
                fields
                    .assembly_borrow()
                    .loan_mut()
                    .record_terminal_cause(cause);
            }
            FiniteStoreFailureValidation::Rejected(FiniteStoreFailureAssembly {
                fields,
                state,
                failure_cause: forced_cause.unwrap_or(cause),
                forced_cause,
                evidence_root,
                error: None,
            })
        }
        RetainedClosureValidation::FallbackOnly {
            cause,
            activation_containment_required,
        } => FiniteStoreFailureValidation::FallbackOnly(FiniteStoreFallbackOnly {
            fields,
            cause,
            provenance: ordinary::StoreSuccessFallbackProvenance::RawValidated {
                activation_containment_required,
            },
            error: None,
        }),
    }
}

impl<O, PP, U, EK, ET> ValidatedFiniteStoreFailure<'_, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Freezes rich-failure evidence through the original finite terminal preparation.
    pub fn try_prepare(
        self,
        cause: u32,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedFiniteStoreFailureOutcome<EK, ET>, FiniteStoreExitError<O, PP, U>> {
        let Self {
            mut fields,
            proof,
            forced_cause,
            evidence_root,
        } = self;
        let request = RetainedTerminalRequest::RichFailure {
            proof,
            requested_cause: forced_cause.unwrap_or(PreparationFailureCode::domain(cause)),
            final_progress,
        };
        match fields.raw.finish_assembly(request) {
            Ok((raw, _control)) => Ok(terminal_output::finite::classify_failure(
                raw,
                evidence_root,
                fields.take_domain(),
            )),
            Err(failure) => Err(fields.terminal_error(failure)),
        }
    }

    /// Discards rich-failure details while preserving any verified success-rejection cause.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        self.fields.prepare_publication_fallback(
            self.forced_cause
                .unwrap_or(PreparationFailureCode::domain(0)),
            None,
        )
    }
}

impl<'inv, O, PP, U> FiniteStoreFallbackOnly<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn from_bound_fields(
        fields: FiniteStoreFinalFields<'inv, O, PP, U>,
        cause: PreparationFailureCode,
        provenance: ordinary::StoreSuccessFallbackProvenance,
        error: Option<StorePrepareError>,
    ) -> Self {
        Self {
            fields,
            cause,
            provenance,
            error,
        }
    }

    pub(in crate::concrete_store) fn into_request_error(self) -> FiniteStoreExitError<O, PP, U> {
        self.fields.into_request_error(self.error)
    }

    /// Returns the original retained validation's activation-containment requirement.
    pub fn activation_containment_required(&self) -> bool {
        matches!(
            self.provenance,
            ordinary::StoreSuccessFallbackProvenance::RawValidated {
                activation_containment_required: true,
            }
        )
    }

    /// Consumes the original fallback disposition and its finite terminal fields.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        self.fields
            .prepare_publication_fallback(self.cause, self.error)
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> FiniteStoreFinalFields<'_, O, PP, U> {
    pub(in crate::concrete_store) fn prepare_publication_fallback(
        mut self,
        cause: PreparationFailureCode,
        error: Option<StorePrepareError>,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        if let Some(error) = error {
            return Err(self.into_error(FiniteStoreExitCause::Finalize(
                FiniteStoreFinalizeCause::Publication(error),
            )));
        }
        let request = RetainedTerminalRequest::MinimalFallback {
            requested_cause: cause,
        };
        match self.raw.finish_assembly(request) {
            Ok((raw, _control)) => Ok(terminal_output::finite::classify_fallback(
                raw,
                self.take_domain(),
            )),
            Err(failure) => Err(self.terminal_error(failure)),
        }
    }
}
