use super::super::payload_owner::drop_captured_value;
use super::validation::{
    EvidenceDecision, EvidenceValidation, FailureValidationCustody, FailureValidationFacts,
    FailureValidationPayloads, FinalFailureResolution, run_rooted_failure_validation,
};
use super::*;
use crate::concrete_store::finite::{
    FiniteStoreExitError, FiniteStoreFailureAssembly, FiniteStoreFallbackOnly,
    FiniteStoreFinalFields, ValidatedFiniteStoreFailure,
};
use crate::concrete_store::ordinary::StoreSuccessFallbackProvenance;
use crate::concrete_store::{PreparedFiniteStoreFailureOutcome, PreparedFiniteStoreFallback};
use crate::retained::RetainedClosureValidation;

/// Final-only rooted evidence coupled to its original finite failure assembly.
pub type FiniteRootedFailureEvidence<'host, O, PP, U, Cause, EK, ET, State = OrdinaryFailureState> =
    CanonicalFinal<
        RootedFailureEvidence<
            'host,
            Cause,
            FiniteStoreFailureAssembly<'host, O, PP, U, EK, ET>,
            State,
        >,
    >;

/// Final-only evidence with the original finite closure proof and commitment.
pub type FiniteValidatedFailureEvidence<
    'host,
    O,
    PP,
    U,
    Cause,
    EK,
    ET,
    State = OrdinaryFailureState,
> = CanonicalFinal<
    ValidatedFailureEvidence<
        'host,
        Cause,
        ValidatedFiniteStoreFailure<'host, O, PP, U, EK, ET>,
        State,
    >,
>;

/// Final-only semantic rejection retaining its original finite evidence fields.
pub type FiniteRejectedFailureEvidence<'host, O, PP, U, Cause, EK, ET> = CanonicalFinal<
    RejectedFailureEvidence<'host, Cause, FiniteStoreFailureAssembly<'host, O, PP, U, EK, ET>>,
>;

/// One shared validation result bound to the original finite owner.
pub type FiniteRootedFailureValidation<
    'host,
    O,
    PP,
    U,
    Cause,
    EK,
    ET,
    State = OrdinaryFailureState,
> = RootedFailureValidation<
    FiniteValidatedFailureEvidence<'host, O, PP, U, Cause, EK, ET, State>,
    FiniteRejectedFailureEvidence<'host, O, PP, U, Cause, EK, ET>,
    VerifiedMinimalFallback<StoreRuntimeError, PreparedFiniteStoreFallback>,
>;

/// A typed cause and the concrete finite terminal result, including its final control cause.
pub type FinitePreparedFailureEvidence<Cause, EK, ET> =
    PreparedFailureEvidence<Cause, PreparedFiniteStoreFailureOutcome<EK, ET>>;

/// Final-only rich evidence or the original cause coupled to finite minimal fallback.
pub type FiniteFailureEvidenceOutcome<
    'host,
    O,
    PP,
    U,
    Cause,
    EK,
    ET,
    State = OrdinaryFailureState,
> = FailureEvidenceOutcome<
    FiniteRootedFailureEvidence<'host, O, PP, U, Cause, EK, ET, State>,
    VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
>;

struct FiniteFailureTransition<'host, O, PP, U, Cause, EK, ET, State>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fields: Option<FiniteStoreFinalFields<'host, O, PP, U>>,
    state: StorePublicationState,
    source: Option<PayloadLoan<'host, Cause>>,
    runtime: Option<PayloadLoan<'host, StoreRuntimeError>>,
    failure_cause: PreparationFailureCode,
    forced_cause: Option<PreparationFailureCode>,
    root: OutcomeEvidenceStoreRef<EK, ET>,
    error: Option<StorePrepareError>,
    original_freeze_control: Option<ControlCause>,
    resolved: Option<EvidenceValidation>,
    unmatched: Option<EvidenceDecision>,
    failure_work: FailureEvidenceWorkBudget,
    destination: StoreDimensionKey,
    _lifecycle: PhantomData<State>,
}

impl<'host, O, PP, U, Cause, EK, ET, State>
    FiniteFailureTransition<'host, O, PP, U, Cause, EK, ET, State>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(
        owner: PayloadOwner<
            PayloadLoan<'host, Cause>,
            FiniteStoreFailureAssembly<'host, O, PP, U, EK, ET>,
        >,
        failure_work: FailureEvidenceWorkBudget,
        destination: StoreDimensionKey,
    ) -> Self {
        let (source, failure) = owner.into_parts();
        let (fields, state, failure_cause, forced_cause, root, error, original_freeze_control) =
            failure.into_failure_parts();
        Self {
            fields: Some(fields),
            state,
            source: Some(source),
            runtime: None,
            failure_cause,
            forced_cause,
            root,
            error,
            original_freeze_control,
            resolved: None,
            unmatched: None,
            failure_work,
            destination,
            _lifecycle: PhantomData,
        }
    }

    fn take_error(
        &mut self,
    ) -> (
        FiniteStoreExitError<O, PP, U>,
        Option<Box<dyn std::any::Any + Send>>,
    ) {
        let mut error = self
            .fields
            .take()
            .expect("the finite failure guard retains its original fields until transfer")
            .into_request_error(self.error);
        error.attach_original_freeze_control(self.original_freeze_control.take());
        let mut first_panic = None;
        drop_finite_error_value(&mut self.source, &mut error, &mut first_panic);
        drop_finite_error_value(&mut self.runtime, &mut error, &mut first_panic);
        drop_finite_error_value(&mut self.resolved, &mut error, &mut first_panic);
        drop_finite_error_value(&mut self.unmatched, &mut error, &mut first_panic);
        (error, first_panic)
    }

    fn into_error(mut self) -> FiniteStoreExitError<O, PP, U> {
        let (error, first_panic) = self.take_error();
        finish_finite_error_cleanup(error, first_panic)
    }

    fn retains_source(&self) -> bool {
        self.source.is_some()
            && self.runtime.is_none()
            && self.unmatched.is_none()
            && self.error.is_none()
    }

    fn validate(
        mut self,
        fallback: CanonicalCauseSlot<'host, StoreRuntimeError>,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EK, ET>,
        ) -> bool,
    ) -> FiniteFailureValidationResult<'host, O, PP, U, Cause, EK, ET, State>
    where
        ET: Copy + Send + Sync + 'static,
        State: FailureLifecycleState,
    {
        let resolution = run_rooted_failure_validation::<O, PP, U, Cause, EK, ET, State, _>(
            self.fields
                .as_mut()
                .expect("the original finite assembly remains installed")
                .failure_decision_loan(&mut self.state),
            FailureValidationFacts {
                root: self.root,
                failure_cause: self.failure_cause,
                forced_cause: self.forced_cause,
                original_error: self.error,
                destination: self.destination,
            },
            &self.failure_work,
            FailureValidationPayloads {
                source: &mut self.source,
                runtime: &mut self.runtime,
                fallback: Some(fallback),
            },
            FailureValidationCustody {
                resolved: &mut self.resolved,
                unmatched: &mut self.unmatched,
            },
            validate,
        );
        match resolution {
            FinalFailureResolution::Contain { error } => {
                self.error = self.error.or(error);
                Err(self.into_error())
            }
            FinalFailureResolution::Valid => {
                if !self.retains_source()
                    || !matches!(&self.resolved, Some(RetainedClosureValidation::Valid(_)))
                {
                    return Err(self.into_error());
                }
                let Some(RetainedClosureValidation::Valid(proof)) = self.resolved.take() else {
                    unreachable!("the guarded original result was checked before transfer")
                };
                let storage = ValidatedFiniteStoreFailure::from_bound_fields(
                    self.fields.take().expect("original finite fields"),
                    proof,
                    self.forced_cause,
                    self.root,
                )
                .with_original_freeze_control(self.original_freeze_control.take());
                Ok(RootedFailureValidation::Valid(CanonicalFinal::new(
                    ValidatedFailureEvidence {
                        owner: PayloadOwner::new(
                            self.source.take().expect("original typed cause permission"),
                            storage,
                        ),
                        _lifecycle: PhantomData,
                    },
                )))
            }
            FinalFailureResolution::Rejected => {
                if !self.retains_source() {
                    return Err(self.into_error());
                }
                let failure_cause = match self.resolved.as_ref() {
                    Some(RetainedClosureValidation::Rejected { cause }) => *cause,
                    _ => return Err(self.into_error()),
                };
                self.resolved = None;
                let storage = FiniteStoreFailureAssembly::from_bound_fields(
                    self.fields.take().expect("original finite fields"),
                    std::mem::take(&mut self.state),
                    failure_cause,
                    self.forced_cause,
                    self.root,
                    None,
                )
                .with_original_freeze_control(self.original_freeze_control.take());
                Ok(RootedFailureValidation::Rejected(CanonicalFinal::new(
                    RejectedFailureEvidence {
                        owner: PayloadOwner::new(
                            self.source.take().expect("original typed cause permission"),
                            storage,
                        ),
                    },
                )))
            }
            FinalFailureResolution::MinimalRuntime { code, provenance } => {
                if self.source.is_some()
                    || self.runtime.is_none()
                    || self.unmatched.is_some()
                    || self.error.is_some()
                    || matches!(
                        provenance,
                        StoreSuccessFallbackProvenance::RawValidated {
                            activation_containment_required: true,
                        }
                    )
                {
                    return Err(self.into_error());
                }
                self.resolved = None;
                let cause = ManuallyDrop::new(
                    self.runtime
                        .take()
                        .expect("original runtime cause permission"),
                );
                let fallback = FiniteStoreFallbackOnly::from_bound_fields(
                    self.fields.take().expect("original finite fields"),
                    code,
                    provenance,
                    self.error,
                )
                .with_original_freeze_control(self.original_freeze_control.take());
                finish_finite_minimal(cause, fallback.prepare_minimal_fallback())
                    .map(RootedFailureValidation::Fallback)
            }
        }
    }
}

impl<O, PP, U, Cause, EK, ET, State> Drop
    for FiniteFailureTransition<'_, O, PP, U, Cause, EK, ET, State>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.fields.is_some() {
            let already_unwinding = std::thread::panicking();
            let (error, first_panic) = self.take_error();
            if let Some(payload) = first_panic {
                if already_unwinding {
                    discard_untrusted_panic_payload(payload);
                } else {
                    let _ = drop_scoped_value(error);
                    std::panic::resume_unwind(payload);
                }
            }
        }
    }
}

fn drop_finite_error_value<T, O, PP, U>(
    value: &mut Option<T>,
    error: &mut FiniteStoreExitError<O, PP, U>,
    first_panic: &mut Option<Box<dyn std::any::Any + Send>>,
) where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    drop_captured_value(value, first_panic, || {
        error.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
        error.contain_payload_failure();
    });
}

fn finish_finite_error_cleanup<O, PP, U>(
    error: FiniteStoreExitError<O, PP, U>,
    first_panic: Option<Box<dyn std::any::Any + Send>>,
) -> FiniteStoreExitError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if let Some(payload) = first_panic {
        discard_untrusted_panic_payload(payload);
    }
    error
}

fn dispose_failed_finite_cause<O, PP, U, Cause>(
    cause: PayloadLoan<'_, Cause>,
    mut error: FiniteStoreExitError<O, PP, U>,
) -> FiniteStoreExitError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut cause = Some(cause);
    let mut first_panic = None;
    drop_finite_error_value(&mut cause, &mut error, &mut first_panic);
    finish_finite_error_cleanup(error, first_panic)
}

fn finish_finite_minimal<O, PP, U, Cause>(
    cause: ManuallyDrop<PayloadLoan<'_, Cause>>,
    prepared: Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>>,
) -> Result<
    VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
    FiniteStoreExitError<O, PP, U>,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match prepared {
        Ok(prepared) => Ok(VerifiedMinimalFallback::new(
            prepared,
            ManuallyDrop::into_inner(cause).into_owned(),
        )),
        Err(error) => Err(dispose_failed_finite_cause(
            ManuallyDrop::into_inner(cause),
            error,
        )),
    }
}

fn prepare_rooted_minimal<O, PP, U, Cause, EK, ET>(
    owner: PayloadOwner<PayloadLoan<'_, Cause>, FiniteStoreFailureAssembly<'_, O, PP, U, EK, ET>>,
) -> Result<
    VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
    FiniteStoreExitError<O, PP, U>,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (cause, failure) = owner.into_parts();
    let cause = ManuallyDrop::new(cause);
    let (fields, state, failure_cause, forced_cause, _root, error, original_freeze_control) =
        failure.into_failure_parts();
    let code = forced_cause
        .or(state.fallback_cause)
        .unwrap_or(failure_cause);
    let prepared = match fields.prepare_publication_fallback(code, error) {
        Ok(mut prepared) => {
            prepared.attach_original_freeze_control(original_freeze_control);
            Ok(prepared)
        }
        Err(mut error) => {
            error.attach_original_freeze_control(original_freeze_control);
            Err(error)
        }
    };
    finish_finite_minimal(cause, prepared)
}

#[allow(
    private_bounds,
    reason = "public failure typestates are sealed by the crate-private lifecycle contract"
)]
impl<'host, O, PP, U, Cause, EK, ET, State>
    CanonicalFinal<
        RootedFailureEvidence<
            'host,
            Cause,
            FiniteStoreFailureAssembly<'host, O, PP, U, EK, ET>,
            State,
        >,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    State: FailureLifecycleState,
{
    pub const fn cause(&self) -> &Cause {
        self.state.owner.value().value()
    }

    /// Validates the original root through the shared protected failure validator.
    pub fn validate_rich_failure(
        self,
        fallback: CanonicalCauseSlot<'host, StoreRuntimeError>,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EK, ET>,
        ) -> bool,
    ) -> FiniteFailureValidationResult<'host, O, PP, U, Cause, EK, ET, State>
    where
        ET: Copy + Send + Sync + 'static,
    {
        let RootedFailureEvidence {
            owner,
            failure_work,
            outcome_evidence_destination_dimension,
            _lifecycle: _,
        } = self.state;
        FiniteFailureTransition::<O, PP, U, Cause, EK, ET, State>::new(
            owner,
            failure_work,
            outcome_evidence_destination_dimension,
        )
        .validate(fallback, validate)
    }

    /// Prepares minimal fallback with the original effective finite failure code.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
        FiniteStoreExitError<O, PP, U>,
    > {
        prepare_rooted_minimal(self.state.owner)
    }
}

impl<'host, O, PP, U, Cause, EK, ET>
    CanonicalFinal<
        ValidatedFailureEvidence<
            'host,
            Cause,
            ValidatedFiniteStoreFailure<'host, O, PP, U, EK, ET>,
            OrdinaryFailureState,
        >,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Prepares once and retains the actual terminal control cause for every physical outcome.
    pub fn try_prepare(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<FinitePreparedFailureEvidence<Cause, EK, ET>, FiniteStoreExitError<O, PP, U>> {
        let (cause, validated) = self.state.owner.into_parts();
        let cause = ManuallyDrop::new(cause);
        match validated.try_prepare_with_control(0, final_progress) {
            Ok((prepared, control)) => Ok(PreparedFailureEvidence::new(
                prepared,
                ManuallyDrop::into_inner(cause).into_owned(),
                control,
            )),
            Err(error) => Err(dispose_failed_finite_cause(
                ManuallyDrop::into_inner(cause),
                error,
            )),
        }
    }
}

impl<'host, O, PP, U, Cause, EK, ET>
    CanonicalFinal<
        ValidatedFailureEvidence<
            'host,
            Cause,
            ValidatedFiniteStoreFailure<'host, O, PP, U, EK, ET>,
            FrozenFailureState,
        >,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Prepares the frozen failure without another final control observation.
    pub fn try_prepare_frozen(
        self,
    ) -> Result<FinitePreparedFailureEvidence<Cause, EK, ET>, FiniteStoreExitError<O, PP, U>> {
        let (cause, validated) = self.state.owner.into_parts();
        let cause = ManuallyDrop::new(cause);
        match validated.try_prepare_frozen() {
            Ok(prepared) => Ok(PreparedFailureEvidence::new(
                prepared,
                ManuallyDrop::into_inner(cause).into_owned(),
                None,
            )),
            Err(error) => Err(dispose_failed_finite_cause(
                ManuallyDrop::into_inner(cause),
                error,
            )),
        }
    }
}

impl<'host, O, PP, U, Cause, EK, ET, State>
    CanonicalFinal<
        ValidatedFailureEvidence<
            'host,
            Cause,
            ValidatedFiniteStoreFailure<'host, O, PP, U, EK, ET>,
            State,
        >,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        self.state.owner.value().value()
    }

    /// Discards the validated details while retaining the original cause and finite owners.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
        FiniteStoreExitError<O, PP, U>,
    > {
        let (cause, validated) = self.state.owner.into_parts();
        finish_finite_minimal(
            ManuallyDrop::new(cause),
            validated.prepare_minimal_fallback(),
        )
    }
}

impl<'host, O, PP, U, Cause, EK, ET>
    CanonicalFinal<
        RejectedFailureEvidence<'host, Cause, FiniteStoreFailureAssembly<'host, O, PP, U, EK, ET>>,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        self.state.owner.value().value()
    }

    /// Retires rejected evidence through the original finite minimal preparation.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
        FiniteStoreExitError<O, PP, U>,
    > {
        prepare_rooted_minimal(self.state.owner)
    }
}

impl<'host, O, PP, U, Cause, EK, ET>
    CanonicalFinal<FiniteCentralFailure<'host, O, PP, U, Cause, EK, ET>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Validates the restored central rejection or interruption without admitting work again.
    pub fn validate_rich_failure(
        self,
        fallback: CanonicalCauseSlot<'host, StoreRuntimeError>,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EK, ET>,
        ) -> bool,
    ) -> FiniteFailureValidationResult<'host, O, PP, U, Cause, EK, ET>
    where
        ET: Copy + Send + Sync + 'static,
    {
        let FiniteCentralFailure {
            owner,
            failure_work,
            evidence_destination,
        } = self.state;
        FiniteFailureTransition::new(owner, failure_work, evidence_destination)
            .validate(fallback, validate)
    }

    /// Uses the already-rooted failure owner and its original terminal cause.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
        FiniteStoreExitError<O, PP, U>,
    > {
        prepare_rooted_minimal(self.state.owner)
    }
}
