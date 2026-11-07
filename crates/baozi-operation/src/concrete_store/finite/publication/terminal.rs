use super::*;

type SuccessPreparationResult<O, PP, U, CK, CT, EK, ET> =
    Result<PreparedFiniteStoreSuccessOutcome<CK, CT, EK, ET>, FiniteStoreExitError<O, PP, U>>;

type FrozenSuccessPreparationResult<O, PP, U, CK, CT, EK, ET> =
    Result<PreparedFiniteStoreSuccess<CK, CT, EK, ET>, FiniteStoreExitError<O, PP, U>>;

type SuccessStorageParts<'inv, O, PP, U, CK, CT, EK, ET> = (
    FiniteStoreFinalFields<'inv, O, PP, U>,
    RetainedSuccessClosureProof<StoredBlock, StoredBlock>,
    CanonicalStoreRef<CK, CT>,
    OutcomeEvidenceStoreRef<EK, ET>,
);

type FailureAssemblyParts<'inv, O, PP, U, EK, ET> = (
    FiniteStoreFinalFields<'inv, O, PP, U>,
    StorePublicationState,
    PreparationFailureCode,
    Option<PreparationFailureCode>,
    OutcomeEvidenceStoreRef<EK, ET>,
    Option<StorePrepareError>,
    Option<ControlCause>,
);

type FailurePreparationWithControlResult<O, PP, U, EK, ET> = Result<
    (
        PreparedFiniteStoreFailureOutcome<EK, ET>,
        Option<ControlCause>,
    ),
    FiniteStoreExitError<O, PP, U>,
>;

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
    pub(in crate::concrete_store::finite) original_freeze_control: Option<ControlCause>,
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
    original_freeze_control: Option<ControlCause>,
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
    pub(in crate::concrete_store::finite) original_freeze_control: Option<ControlCause>,
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

    pub(in crate::concrete_store) fn into_storage_parts(
        self,
    ) -> SuccessStorageParts<'inv, O, PP, U, CK, CT, EK, ET> {
        (
            self.fields,
            self.proof,
            self.canonical_root,
            self.evidence_root,
        )
    }

    pub(in crate::concrete_store) fn roots_match(
        &self,
        canonical_root: CanonicalStoreRef<CK, CT>,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    ) -> bool {
        self.canonical_root == canonical_root && self.evidence_root == evidence_root
    }

    pub(in crate::concrete_store) fn ordinary_assembly(
        &mut self,
    ) -> RawStoreAssemblyBorrow<'_, O, PP, U> {
        self.fields.assembly_borrow()
    }

    pub(in crate::concrete_store) fn try_prepare_frozen(
        self,
    ) -> FrozenSuccessPreparationResult<O, PP, U, CK, CT, EK, ET> {
        let Self {
            mut fields,
            proof,
            canonical_root,
            evidence_root,
        } = self;
        match fields
            .raw
            .finish_assembly(RetainedTerminalRequest::FrozenSuccess { proof })
        {
            Ok((raw, _control)) => Ok(terminal_output::finite::classify_frozen_success(
                raw,
                canonical_root,
                evidence_root,
                fields.take_domain(),
            )),
            Err(failure) => Err(fields.terminal_error(failure)),
        }
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
            original_freeze_control: None,
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
    pub(in crate::concrete_store) fn into_failure_parts(
        self,
    ) -> FailureAssemblyParts<'inv, O, PP, U, PreservedKind, PreservedTag> {
        (
            self.fields,
            self.state,
            self.failure_cause,
            self.forced_cause,
            self.evidence_root,
            self.error,
            self.original_freeze_control,
        )
    }

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
            original_freeze_control: None,
        }
    }

    pub(in crate::concrete_store) fn with_original_freeze_control(
        mut self,
        control: Option<ControlCause>,
    ) -> Self {
        if self.original_freeze_control.is_none() {
            self.original_freeze_control = control;
        }
        self
    }

    pub(in crate::concrete_store) fn into_request_error(self) -> FiniteStoreExitError<O, PP, U> {
        let mut error = self.fields.into_request_error(self.error);
        error.attach_original_freeze_control(self.original_freeze_control);
        error
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
            original_freeze_control,
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
                    original_freeze_control,
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
            original_freeze_control,
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
            original_freeze_control,
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
        fields.prepare_publication_fallback_with_original_freeze_control(
            cause,
            error,
            original_freeze_control,
        )
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
    original_freeze_control: Option<ControlCause>,
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
                original_freeze_control,
            });
        }
    };
    if error.is_some() {
        return FiniteStoreFailureValidation::FallbackOnly(FiniteStoreFallbackOnly {
            fields,
            cause: PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE),
            provenance: ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
            error,
            original_freeze_control,
        });
    }
    match validation {
        RetainedClosureValidation::Valid(proof) => {
            FiniteStoreFailureValidation::Valid(ValidatedFiniteStoreFailure {
                fields,
                proof,
                forced_cause,
                evidence_root,
                original_freeze_control,
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
                original_freeze_control,
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
            original_freeze_control,
        }),
    }
}

impl<'inv, O, PP, U, EK, ET> ValidatedFiniteStoreFailure<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn from_bound_fields(
        fields: FiniteStoreFinalFields<'inv, O, PP, U>,
        proof: RetainedEvidenceClosureProof<StoredBlock>,
        forced_cause: Option<PreparationFailureCode>,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    ) -> Self {
        Self {
            fields,
            proof,
            forced_cause,
            evidence_root,
            original_freeze_control: None,
        }
    }

    pub(in crate::concrete_store) fn with_original_freeze_control(
        mut self,
        control: Option<ControlCause>,
    ) -> Self {
        if self.original_freeze_control.is_none() {
            self.original_freeze_control = control;
        }
        self
    }

    pub(in crate::concrete_store) fn into_request_error(self) -> FiniteStoreExitError<O, PP, U> {
        let mut error = self.fields.into_request_error(None);
        error.attach_original_freeze_control(self.original_freeze_control);
        error
    }

    /// Freezes rich-failure evidence through the original finite terminal preparation.
    pub fn try_prepare(
        self,
        cause: u32,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedFiniteStoreFailureOutcome<EK, ET>, FiniteStoreExitError<O, PP, U>> {
        self.try_prepare_with_control(cause, final_progress)
            .map(|(prepared, _control)| prepared)
    }

    pub(in crate::concrete_store) fn try_prepare_with_control(
        self,
        cause: u32,
        final_progress: FinalProgress<PP, U>,
    ) -> FailurePreparationWithControlResult<O, PP, U, EK, ET> {
        let Self {
            mut fields,
            proof,
            forced_cause,
            evidence_root,
            original_freeze_control,
        } = self;
        let request = RetainedTerminalRequest::RichFailure {
            proof,
            requested_cause: forced_cause.unwrap_or(PreparationFailureCode::domain(cause)),
            final_progress,
        };
        match fields.raw.finish_assembly(request) {
            Ok((raw, control)) => Ok((
                terminal_output::finite::classify_failure(
                    raw,
                    evidence_root,
                    fields.take_domain(),
                    original_freeze_control,
                ),
                control,
            )),
            Err(failure) => {
                let mut error = fields.terminal_error(failure);
                error.attach_original_freeze_control(original_freeze_control);
                Err(error)
            }
        }
    }

    pub(in crate::concrete_store) fn try_prepare_frozen(
        self,
    ) -> Result<PreparedFiniteStoreFailureOutcome<EK, ET>, FiniteStoreExitError<O, PP, U>> {
        let Self {
            mut fields,
            proof,
            forced_cause,
            evidence_root,
            original_freeze_control,
        } = self;
        let request = RetainedTerminalRequest::FrozenRichFailure {
            proof,
            requested_cause: forced_cause.unwrap_or(PreparationFailureCode::domain(0)),
        };
        match fields.raw.finish_assembly(request) {
            Ok((raw, _control)) => Ok(terminal_output::finite::classify_failure(
                raw,
                evidence_root,
                fields.take_domain(),
                original_freeze_control,
            )),
            Err(failure) => {
                let mut error = fields.terminal_error(failure);
                error.attach_original_freeze_control(original_freeze_control);
                Err(error)
            }
        }
    }

    /// Discards rich-failure details while preserving any verified success-rejection cause.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        self.fields
            .prepare_publication_fallback_with_original_freeze_control(
                self.forced_cause
                    .unwrap_or(PreparationFailureCode::domain(0)),
                None,
                self.original_freeze_control,
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
            original_freeze_control: None,
        }
    }

    pub(in crate::concrete_store) fn with_original_freeze_control(
        mut self,
        control: Option<ControlCause>,
    ) -> Self {
        if self.original_freeze_control.is_none() {
            self.original_freeze_control = control;
        }
        self
    }

    pub(in crate::concrete_store) fn into_request_error(self) -> FiniteStoreExitError<O, PP, U> {
        let mut error = self.fields.into_request_error(self.error);
        error.attach_original_freeze_control(self.original_freeze_control);
        error
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
            .prepare_publication_fallback_with_original_freeze_control(
                self.cause,
                self.error,
                self.original_freeze_control,
            )
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> FiniteStoreFinalFields<'_, O, PP, U> {
    pub(in crate::concrete_store) fn prepare_publication_fallback(
        self,
        cause: PreparationFailureCode,
        error: Option<StorePrepareError>,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        self.prepare_publication_fallback_with_original_freeze_control(cause, error, None)
    }

    pub(in crate::concrete_store) fn prepare_publication_fallback_with_original_freeze_control(
        mut self,
        cause: PreparationFailureCode,
        error: Option<StorePrepareError>,
        original_freeze_control: Option<ControlCause>,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        if let Some(error) = error {
            let mut error = self.into_error(FiniteStoreExitCause::Finalize(
                FiniteStoreFinalizeCause::Publication(error),
            ));
            error.attach_original_freeze_control(original_freeze_control);
            return Err(error);
        }
        let request = RetainedTerminalRequest::MinimalFallback {
            requested_cause: cause,
        };
        match self.raw.finish_assembly(request) {
            Ok((raw, _control)) => Ok(
                terminal_output::finite::classify_fallback_with_original_freeze_control(
                    raw,
                    self.take_domain(),
                    original_freeze_control,
                ),
            ),
            Err(failure) => {
                let mut error = self.terminal_error(failure);
                error.attach_original_freeze_control(original_freeze_control);
                Err(error)
            }
        }
    }
}
