//! Concrete finite publication envelopes over the original root and shared ordinary kernels.

use super::*;
use crate::control::discard_untrusted_panic_payload;
use crate::retained::{
    RetainedClosureValidation, RetainedEvidenceClosureProof, RetainedPublicationAssemblyBorrow,
    RetainedSuccessClosureProof, RetainedTerminalRequest,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

mod validation;
pub use validation::{
    FiniteStoreControlledSuccessValidation, FiniteStoreFailureValidation,
    FiniteStoreSuccessValidation,
};
mod terminal;
pub use terminal::{
    FiniteStoreFailureAssembly, FiniteStoreFallbackOnly, ValidatedFiniteStoreFailure,
    ValidatedFiniteStoreSuccess,
};

/// Writable canonical and evidence tables owned by the original finite invocation.
#[must_use = "the finite assembly must be validated or published as fallback"]
pub struct FiniteRetainedStorePublicationAssembly<'inv, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fields: FiniteStoreFinalFields<'inv, O, PP, U>,
    state: StorePublicationState,
    lineage: L,
}

impl<'inv, O, PP, U, L> FiniteRetainedStorePublicationAssembly<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Borrows the shared Store algorithms without moving any finite owner or lineage.
    pub fn ordinary(&mut self) -> StorePublicationLoan<'_, O, PP, U, L> {
        StorePublicationLoan {
            raw: RetainedPublicationAssemblyBorrow::from_loan(
                self.fields
                    .raw
                    .assembly_loan()
                    .expect("original finite assembly"),
            ),
            domain: self.fields.domain.as_ref().expect("original finite domain"),
            state: &mut self.state,
            lineage: PhantomData,
        }
    }

    pub(in crate::concrete_store) fn from_bound_fields(
        fields: FiniteStoreFinalFields<'inv, O, PP, U>,
        state: StorePublicationState,
        lineage: L,
    ) -> Self {
        Self {
            fields,
            state,
            lineage,
        }
    }

    pub(in crate::concrete_store) fn contain_activation_fault(
        self,
    ) -> FinalizationContainmentError {
        self.retire_activation(true)
            .expect("a requested finite containment records its cause")
    }

    pub(in crate::concrete_store) fn retire_activation(
        self,
        contain: bool,
    ) -> Option<FinalizationContainmentError> {
        let mut transition = FinitePublicationTransition::from_assembly(self);
        if contain {
            transition.contain();
        }
        let (_, _, error) = transition.finish();
        error.map(|StorePrepareError::FinalizationContainment(cause)| cause)
    }

    /// Consumes the writable candidate and validates its exact success closure.
    pub fn validate_success<CK, CT, EK, ET>(
        self,
        canonical_root: CanonicalStoreRef<CK, CT>,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreValidationSession<'session, 'storage>,
            CanonicalStoreRef<CK, CT>,
            OutcomeEvidenceStoreRef<EK, ET>,
        ) -> bool,
    ) -> FiniteStoreSuccessValidation<'inv, O, PP, U, CK, CT, EK, ET>
    where
        CT: Copy + Send + Sync + 'static,
        ET: Copy + Send + Sync + 'static,
    {
        let mut transition = FinitePublicationTransition::from_assembly(self);
        let resolution = catch_unwind(AssertUnwindSafe(|| {
            let mut ordinary = transition.ordinary();
            let decision =
                ordinary.validate_success_decision(canonical_root, evidence_root, validate);
            ordinary.resolve_success_decision(decision)
        }));
        let resolution = match resolution {
            Ok(resolution) => resolution,
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return FiniteStoreSuccessValidation::FallbackOnly(
                    transition.into_contained_fallback(),
                );
            }
        };
        match transition.bind_success_resolution(resolution, canonical_root, evidence_root) {
            FiniteStoreControlledSuccessValidation::Valid(success) => {
                FiniteStoreSuccessValidation::Valid(success)
            }
            FiniteStoreControlledSuccessValidation::Rejected(failure) => {
                FiniteStoreSuccessValidation::Rejected(failure)
            }
            FiniteStoreControlledSuccessValidation::FallbackOnly(fallback) => {
                FiniteStoreSuccessValidation::FallbackOnly(fallback)
            }
            FiniteStoreControlledSuccessValidation::Interrupted { .. }
            | FiniteStoreControlledSuccessValidation::InvalidWorkDimension { .. } => {
                unreachable!("uncontrolled validation has no work dimension or work error")
            }
        }
    }

    /// Validates success through the same operation-bound controlled decision normalizer.
    pub fn validate_success_controlled<CK, CT, EK, ET>(
        self,
        work_dimension: StoreDimensionKey,
        scratch_dimension: StoreDimensionKey,
        canonical_root: CanonicalStoreRef<CK, CT>,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
        validate: impl for<'session, 'storage, 'control> FnOnce(
            &mut StoreValidationSession<'session, 'storage>,
            CanonicalStoreRef<CK, CT>,
            OutcomeEvidenceStoreRef<EK, ET>,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<bool, StoreWorkError>,
    ) -> FiniteStoreControlledSuccessValidation<'inv, O, PP, U, CK, CT, EK, ET, L>
    where
        CT: Copy + Send + Sync + 'static,
        ET: Copy + Send + Sync + 'static,
    {
        let mut transition = FinitePublicationTransition::from_assembly(self);
        let resolution = catch_unwind(AssertUnwindSafe(|| {
            let mut ordinary = transition.ordinary();
            ordinary
                .validate_success_controlled_decision(
                    work_dimension,
                    scratch_dimension,
                    canonical_root,
                    evidence_root,
                    validate,
                )
                .map(|decision| ordinary.resolve_success_decision(decision))
        }));
        match resolution {
            Ok(Ok(resolution)) => {
                transition.bind_success_resolution(resolution, canonical_root, evidence_root)
            }
            Ok(Err(cause)) => FiniteStoreControlledSuccessValidation::InvalidWorkDimension {
                assembly: transition.into_assembly(),
                cause,
            },
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                FiniteStoreControlledSuccessValidation::FallbackOnly(
                    transition.into_contained_fallback(),
                )
            }
        }
    }

    /// Consumes the candidate and validates only its exact outcome-evidence closure.
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
        let mut transition = FinitePublicationTransition::from_assembly(self);
        let validation = catch_unwind(AssertUnwindSafe(|| {
            let mut ordinary = transition.ordinary();
            match ordinary.validate_evidence_decision(evidence_root, validate) {
                ordinary::StoreClosureDecision::FallbackOnly { cause } => Err((
                    cause,
                    ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                )),
                ordinary::StoreClosureDecision::Closure { retained, .. } => ordinary
                    .raw
                    .loan_mut()
                    .bind_evidence_proof(retained)
                    .map_err(|_decision| {
                        (
                            PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE),
                            ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                        )
                    }),
            }
        }));
        let validation = match validation {
            Ok(validation) => validation,
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return FiniteStoreFailureValidation::FallbackOnly(
                    transition.into_contained_fallback(),
                );
            }
        };
        let (fields, state, error) = transition.finish();
        let forced_cause = state.success_rejection;
        terminal::bind_failure_validation(
            fields,
            state,
            validation,
            evidence_root,
            forced_cause,
            error,
        )
    }

    /// Discards all candidates and prepares the pre-admitted minimal fallback.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        self.prepare_minimal_fallback_with(0)
    }

    /// Preserves an existing verified rejection before using the requested domain cause.
    pub fn prepare_minimal_fallback_with(
        self,
        cause: u32,
    ) -> Result<PreparedFiniteStoreFallback, FiniteStoreExitError<O, PP, U>> {
        let (fields, state, error) = FinitePublicationTransition::from_assembly(self).finish();
        let cause = state
            .fallback_cause
            .or(state.success_rejection)
            .unwrap_or(PreparationFailureCode::domain(cause));
        fields.prepare_publication_fallback(cause, error)
    }
}

pub(in crate::concrete_store) struct FinitePublicationTransition<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fields: Option<FiniteStoreFinalFields<'inv, O, PP, U>>,
    pub(in crate::concrete_store) state: StorePublicationState,
    lineage: Option<L>,
    containment: Option<StorePrepareError>,
}

impl<'inv, O, PP, U, L> FinitePublicationTransition<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn new(
        fields: FiniteStoreFinalFields<'inv, O, PP, U>,
        state: StorePublicationState,
        lineage: L,
    ) -> Self {
        Self {
            fields: Some(fields),
            state,
            lineage: Some(lineage),
            containment: None,
        }
    }

    pub(in crate::concrete_store) fn from_assembly(
        assembly: FiniteRetainedStorePublicationAssembly<'inv, O, PP, U, L>,
    ) -> Self {
        let FiniteRetainedStorePublicationAssembly {
            fields,
            state,
            lineage,
        } = assembly;
        Self::new(fields, state, lineage)
    }

    pub(in crate::concrete_store) fn ordinary(&mut self) -> StorePublicationLoan<'_, O, PP, U, L> {
        let fields = self.fields.as_mut().expect("original finite fields");
        StorePublicationLoan {
            raw: RetainedPublicationAssemblyBorrow::from_loan(
                fields
                    .raw
                    .assembly_loan()
                    .expect("original finite assembly"),
            ),
            domain: fields.domain.as_ref().expect("original finite domain"),
            state: &mut self.state,
            lineage: PhantomData,
        }
    }

    pub(in crate::concrete_store) fn contain(&mut self) -> StorePrepareError {
        if let Some(error) = self.containment {
            return error;
        }
        let fields = self.fields.as_mut().expect("original finite fields");
        let error = StorePrepareError::FinalizationContainment(fields.contain_publication_fault());
        self.containment = Some(error);
        error
    }

    pub(in crate::concrete_store) fn record_containment(&mut self, original: StorePrepareError) {
        if self.containment.is_none() {
            self.containment = Some(original);
        }
    }

    pub(in crate::concrete_store) fn retire_lineage(&mut self) {
        if activation::drop_scoped_value(self.lineage.take()).is_err() {
            self.contain();
        }
    }

    pub(in crate::concrete_store) fn take_lineage(&mut self) -> Option<L> {
        self.lineage.take()
    }

    pub(super) fn into_assembly(
        mut self,
    ) -> FiniteRetainedStorePublicationAssembly<'inv, O, PP, U, L> {
        FiniteRetainedStorePublicationAssembly {
            fields: self.fields.take().expect("original finite fields"),
            state: std::mem::take(&mut self.state),
            lineage: self.lineage.take().expect("original finite lineage"),
        }
    }

    pub(super) fn into_open_error(
        mut self,
        cause: StoreRuntimeError,
    ) -> FiniteStoreExitError<O, PP, U> {
        self.retire_lineage();
        self.fields
            .take()
            .expect("original finite fields")
            .into_error(FiniteStoreExitCause::Finalize(
                FiniteStoreFinalizeCause::PublicationOpen(cause),
            ))
    }

    pub(in crate::concrete_store) fn finish(
        mut self,
    ) -> (
        FiniteStoreFinalFields<'inv, O, PP, U>,
        StorePublicationState,
        Option<StorePrepareError>,
    ) {
        self.retire_lineage();
        (
            self.fields.take().expect("original finite fields"),
            std::mem::take(&mut self.state),
            self.containment,
        )
    }

    fn into_contained_fallback(mut self) -> FiniteStoreFallbackOnly<'inv, O, PP, U> {
        self.contain();
        let (fields, _, error) = self.finish();
        FiniteStoreFallbackOnly {
            fields,
            cause: PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE),
            provenance: ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
            error,
        }
    }
}

impl<O, PP, U, L> Drop for FinitePublicationTransition<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.fields.is_some() {
            self.retire_lineage();
            self.contain();
        }
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> FiniteStoreFinalFields<'_, O, PP, U> {
    pub(in crate::concrete_store) fn failure_decision_loan<'op>(
        &'op mut self,
        state: &'op mut StorePublicationState,
    ) -> ordinary::StoreFailureEvidenceLoan<'op, O, PP, U> {
        let raw = RetainedPublicationAssemblyBorrow::from_loan(
            self.raw
                .assembly_loan()
                .expect("the original finite assembly remains installed"),
        );
        ordinary::StoreFailureEvidenceLoan::new(
            raw,
            self.domain.as_ref().expect("original finite domain"),
            state,
        )
    }

    fn assembly_borrow(&mut self) -> RawStoreAssemblyBorrow<'_, O, PP, U> {
        RetainedPublicationAssemblyBorrow::from_loan(
            self.raw
                .assembly_loan()
                .expect("the original finite assembly remains installed"),
        )
    }

    pub(in crate::concrete_store) fn contain_publication_fault(
        &mut self,
    ) -> FinalizationContainmentError {
        if let Some(assembly) = self.raw.assembly_loan() {
            let mut ordinary = RetainedPublicationAssemblyBorrow::from_loan(assembly);
            return ordinary.loan_mut().contain_activation_fault();
        }
        self.raw
            .loan()
            .expect("the original attached finite operation remains installed")
            .contain_activation_fault()
    }
}

pub(in crate::concrete_store) struct FiniteUnrootedStoreFailure<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fields: FiniteStoreFinalFields<'inv, O, PP, U>,
    pub(super) state: StorePublicationState,
    pub(super) failure_cause: PreparationFailureCode,
}

impl<'inv, O, PP, U> FiniteStoreFinalFields<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn bind_unrooted_failure(
        mut self,
        state: StorePublicationState,
        cause: PreparationFailureCode,
    ) -> Result<FiniteUnrootedStoreFailure<'inv, O, PP, U>, FiniteStoreExitError<O, PP, U>> {
        let result = if let Some(raw) = self.raw.assembly_loan() {
            let mut assembly = RetainedPublicationAssemblyBorrow::from_loan(raw);
            assembly.loan_mut().verify_failure_only(cause)
        } else {
            Err(self.contain_publication_fault())
        };
        match result {
            Ok(()) => Ok(FiniteUnrootedStoreFailure {
                fields: self,
                state,
                failure_cause: cause,
            }),
            Err(cause) => Err(self
                .into_activation_error(activation::StoreActivationContainment::from_cause(cause))),
        }
    }
}

impl<'inv, O, PP, U> FiniteUnrootedStoreFailure<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn into_failure_parts(
        self,
    ) -> (
        FiniteStoreFinalFields<'inv, O, PP, U>,
        StorePublicationState,
        PreparationFailureCode,
    ) {
        (self.fields, self.state, self.failure_cause)
    }

    pub(in crate::concrete_store) fn contain_activation_fault(
        mut self,
    ) -> FinalizationContainmentError {
        self.fields.contain_publication_fault()
    }

    pub(in crate::concrete_store) fn into_request_error(self) -> FiniteStoreExitError<O, PP, U> {
        self.fields.into_request_error(None)
    }
}
