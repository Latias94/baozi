//! Concrete finite binding for the one shared central validation algorithm.

use super::super::payload_owner::PayloadValue;
use super::super::*;
use crate::concrete_store::finite::{
    FiniteRetainedStorePublicationAssembly, FiniteStoreExitError, FiniteStoreFailureAssembly,
    FiniteStoreFallbackOnly, ValidatedFiniteStoreSuccess,
};

pub(in crate::concrete_store::activation) struct FiniteActivatedParts<'host, O, PP, U, T, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::activation) output: PayloadLoan<'host, T>,
    pub(in crate::concrete_store::activation) assembly:
        FiniteRetainedStorePublicationAssembly<'host, O, PP, U, L>,
    pub(in crate::concrete_store::activation) canonical_root: CanonicalStoreRef<CK, CT>,
    pub(in crate::concrete_store::activation) validation: ScopedActivationValidation,
    pub(in crate::concrete_store::activation) failure_work: FailureEvidenceWorkBudget,
    pub(in crate::concrete_store::activation) evidence_destination: StoreDimensionKey,
}

type CandidateOwner<'host, O, PP, U, T, Proof, CK, CT, EK, ET> = PayloadOwner<
    PayloadLoan<'host, Proof>,
    PayloadOwner<
        PayloadLoan<'host, T>,
        ValidatedFiniteStoreSuccess<'host, O, PP, U, CK, CT, EK, ET>,
    >,
>;

type FiniteCentralResult<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET> = Result<
    FiniteCanonicalCentralValidationOutcome<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET>,
    FiniteStoreExitError<O, PP, U>,
>;
type RootedDispositionConstructor<CK, CT, EK, ET> =
    fn(
        host::driver::PendingRootedFailure<EK, ET>,
    ) -> host::finite::FiniteDisposition<CK, CT, EK, ET>;

/// Original output and proof bound to the finite validated closure; no body finalizer is exposed.
///
/// A body candidate cannot prepare an owned publication before Host cleanup:
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::FiniteCentralCandidate;
/// use baozi_operation::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit};
/// fn prepare_early<O: OperationOwner, P: ProgressPhase, U: ProgressUnit, T, Proof>(
///     candidate: FiniteCentralCandidate<'_, O, P, U, T, Proof, (), (), (), ()>,
///     progress: FinalProgress<P, U>,
/// ) {
///     let _ = candidate.try_prepare(progress);
/// }
/// ```
#[must_use = "the finite candidate must record a finish request"]
pub struct FiniteCentralCandidate<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::activation) owner:
        CandidateOwner<'host, O, PP, U, T, Proof, CK, CT, EK, ET>,
    pub(in crate::concrete_store::activation) destinations: ZeroCommitDestinationFacts,
    pub(in crate::concrete_store::activation) failure_work: FailureEvidenceWorkBudget,
}

/// A typed rejection or interruption coupled to its original finite evidence storage.
#[must_use = "the finite failure must record a finish request"]
pub struct FiniteCentralFailure<'host, O, PP, U, Cause, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::activation) owner: PayloadOwner<
        PayloadLoan<'host, Cause>,
        FiniteStoreFailureAssembly<'host, O, PP, U, EK, ET>,
    >,
    pub(in crate::concrete_store::activation) failure_work: FailureEvidenceWorkBudget,
    pub(in crate::concrete_store::activation) evidence_destination: StoreDimensionKey,
}

/// The actual runtime cause and finite fallback provenance, still unprepared during the body.
#[must_use = "the finite fallback must record a finish request"]
pub struct FiniteCentralFallback<'host, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::activation) owner: PayloadOwner<
        PayloadLoan<'host, StoreRuntimeError>,
        FiniteStoreFallbackOnly<'host, O, PP, U>,
    >,
}

/// One normalized central decision bound to its concrete finite owner.
#[must_use = "the finite central outcome must record a finish request"]
pub enum FiniteCanonicalCentralValidationOutcome<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Candidate(FiniteCentralCandidate<'host, O, PP, U, T, Proof, CK, CT, EK, ET>),
    Rejected(FiniteCentralFailure<'host, O, PP, U, StoreCanonicalRejection<E>, EK, ET>),
    Interrupted(FiniteCentralFailure<'host, O, PP, U, StoreWorkError, EK, ET>),
    Fallback(FiniteCentralFallback<'host, O, PP, U>),
}

impl<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET>
    FiniteCanonicalCentralValidationOutcome<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn request_finish(
        self,
        disposition: FiniteCanonicalDispositionSlot<'host, CK, CT, EK, ET>,
    ) -> Result<(), FiniteStoreExitError<O, PP, U>> {
        use host::finite::FiniteDisposition;
        match self {
            Self::Candidate(candidate) => candidate.request_finish(disposition),
            Self::Rejected(failure) => {
                failure.request(disposition, FiniteDisposition::CentralRejected)
            }
            Self::Interrupted(failure) => {
                failure.request(disposition, FiniteDisposition::Interrupted)
            }
            Self::Fallback(fallback) => fallback.request(disposition),
        }
    }
}

impl<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
    FiniteCentralCandidate<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        self.owner.storage().value().value()
    }
    pub const fn proof(&self) -> &Proof {
        self.owner.value().value()
    }
    pub const fn canonical_member_count(&self) -> u64 {
        self.destinations.canonical_members.get()
    }
    pub const fn outcome_evidence_member_count(&self) -> u64 {
        self.destinations.outcome_evidence_members.get()
    }

    /// Leaves the original candidate and proof in the Host until body cleanup completes.
    pub fn request_finish(
        self,
        disposition: FiniteCanonicalDispositionSlot<'host, CK, CT, EK, ET>,
    ) -> Result<(), FiniteStoreExitError<O, PP, U>> {
        let Self {
            owner,
            destinations,
            failure_work,
        } = self;
        let (proof, output_owner) = owner.into_parts();
        let (output, storage) = output_owner.into_parts();
        let mut proof = PayloadValue::new(proof);
        let mut output = PayloadValue::new(output);
        let output_identity = output
            .value
            .as_ref()
            .expect("original output permission")
            .identity();
        if !disposition.is_vacant_for(output_identity)
            || proof
                .value
                .as_ref()
                .expect("original proof permission")
                .identity()
                != output_identity
        {
            let _ = proof.dispose();
            let _ = output.dispose();
            return Err(storage.into_request_error());
        }
        let store = match storage.defer_until_body_cleanup() {
            Ok(store) => store,
            Err(storage) => {
                let _ = proof.dispose();
                let _ = output.dispose();
                return Err(storage.into_request_error());
            }
        };
        disposition.install_pending(host::finite::FiniteDisposition::Candidate(
            host::driver::PendingCandidate {
                store,
                destinations,
                failure_work,
            },
        ));
        proof
            .take()
            .expect("original proof permission")
            .retain_in_host();
        output
            .take()
            .expect("original output permission")
            .retain_in_host();
        Ok(())
    }
}

impl<'host, O, PP, U, Cause, EK, ET> FiniteCentralFailure<'host, O, PP, U, Cause, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        self.owner.value().value()
    }

    fn request<CK, CT>(
        self,
        disposition: FiniteCanonicalDispositionSlot<'host, CK, CT, EK, ET>,
        wrap: RootedDispositionConstructor<CK, CT, EK, ET>,
    ) -> Result<(), FiniteStoreExitError<O, PP, U>> {
        let Self {
            owner,
            failure_work,
            evidence_destination,
        } = self;
        let (cause, storage) = owner.into_parts();
        let mut cause = PayloadValue::new(cause);
        if !disposition.is_vacant_for(
            cause
                .value
                .as_ref()
                .expect("original cause permission")
                .identity(),
        ) {
            let _ = cause.dispose();
            return Err(storage.into_request_error());
        }
        let store = match storage.defer_until_body_cleanup() {
            Ok(store) => store,
            Err(storage) => {
                let _ = cause.dispose();
                return Err(storage.into_request_error());
            }
        };
        disposition.install_pending(wrap(host::driver::PendingRootedFailure {
            store,
            failure_work,
            destination: evidence_destination,
        }));
        cause
            .take()
            .expect("original cause permission")
            .retain_in_host();
        Ok(())
    }
}

impl<'host, O, PP, U> FiniteCentralFallback<'host, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreRuntimeError {
        self.owner.value().value()
    }

    fn request<CK, CT, EK, ET>(
        self,
        disposition: FiniteCanonicalDispositionSlot<'host, CK, CT, EK, ET>,
    ) -> Result<(), FiniteStoreExitError<O, PP, U>> {
        let (cause, storage) = self.owner.into_parts();
        let mut cause = PayloadValue::new(cause);
        if !disposition.is_vacant_for(
            cause
                .value
                .as_ref()
                .expect("original cause permission")
                .identity(),
        ) {
            let _ = cause.dispose();
            return Err(storage.into_request_error());
        }
        let store = match storage.defer_until_body_cleanup() {
            Ok(store) => store,
            Err(storage) => {
                let _ = cause.dispose();
                return Err(storage.into_request_error());
            }
        };
        disposition.install_pending(host::finite::FiniteDisposition::Fallback(store));
        cause
            .take()
            .expect("original cause permission")
            .retain_in_host();
        Ok(())
    }
}

impl<O, PP, U, T, Proof, CK, CT, EK, ET>
    CanonicalFinal<FiniteCentralCandidate<'_, O, PP, U, T, Proof, CK, CT, EK, ET>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        self.state.output()
    }
    pub const fn proof(&self) -> &Proof {
        self.state.proof()
    }
}

impl<O, PP, U, Cause, EK, ET> CanonicalFinal<FiniteCentralFailure<'_, O, PP, U, Cause, EK, ET>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        self.state.cause()
    }
}

impl<'host, O, PP, U> CanonicalFinal<FiniteCentralFallback<'host, O, PP, U>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreRuntimeError {
        self.state.cause()
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        VerifiedMinimalFallback<
            StoreRuntimeError,
            crate::concrete_store::PreparedFiniteStoreFallback,
        >,
        FiniteStoreExitError<O, PP, U>,
    > {
        let (cause, fallback) = self.state.owner.into_parts();
        let mut cause = PayloadValue::new(cause);
        match fallback.prepare_minimal_fallback() {
            Ok(prepared) => Ok(VerifiedMinimalFallback::new(
                prepared,
                cause
                    .take()
                    .expect("original finite runtime cause")
                    .into_owned(),
            )),
            Err(mut error) => {
                if cause.dispose().is_err() {
                    error.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
                    error.contain_payload_failure();
                }
                Err(error)
            }
        }
    }
}

use super::super::central_driver::{
    CentralPendingDecision, CentralValidationFacts, CentralValidationRequest,
    drop_pending_central_decision, run_central_validation,
};
use crate::concrete_store::finite::{
    FinitePublicationTransition, FiniteStoreExitCause, FiniteStoreFinalizeCause,
};

struct FiniteCentralTransition<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    publication: Option<FinitePublicationTransition<'host, O, PP, U, L>>,
    output: Option<PayloadLoan<'host, T>>,
    pending: Option<CentralPendingDecision<'host, T, Proof, E, CK, CT, EK, ET>>,
    failure_work: FailureEvidenceWorkBudget,
    evidence_destination: StoreDimensionKey,
}

impl<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
    FiniteCentralTransition<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain(&mut self) -> StorePrepareError {
        let recorded = match self.pending.take() {
            Some(CentralPendingDecision::Contain { recorded }) => recorded,
            pending => {
                let _ = drop_pending_central_decision(pending);
                None
            }
        };
        let _ = drop_scoped_value(self.output.take());
        let publication = self
            .publication
            .as_mut()
            .expect("original finite central fields");
        let _ = drop_scoped_value(publication.take_lineage());
        match recorded {
            Some(recorded) => {
                let error = StorePrepareError::FinalizationContainment(recorded.cause());
                publication.record_containment(error);
                error
            }
            None => publication.contain(),
        }
    }

    fn into_error(mut self) -> FiniteStoreExitError<O, PP, U> {
        let error = self.contain();
        let (fields, _, _) = self
            .publication
            .take()
            .expect("original finite central fields")
            .finish();
        fields.into_error(FiniteStoreExitCause::Finalize(
            FiniteStoreFinalizeCause::Publication(error),
        ))
    }

    #[inline(never)]
    fn finish(
        mut self,
        valid: bool,
    ) -> FiniteCentralResult<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET> {
        if !valid
            || matches!(
                self.pending,
                None | Some(CentralPendingDecision::Contain { .. })
            )
        {
            return Err(self.into_error());
        }
        // The actual lineage is disposed while both pending permissions and fields remain armed.
        let lineage = self
            .publication
            .as_mut()
            .expect("original finite central fields")
            .take_lineage();
        if drop_scoped_value(lineage).is_err() {
            return Err(self.into_error());
        }
        let (fields, state, error) = self
            .publication
            .take()
            .expect("original finite central fields")
            .finish();
        if let Some(error) = error {
            let _ = drop_pending_central_decision(self.pending.take());
            let _ = drop_scoped_value(self.output.take());
            return Err(fields.into_error(FiniteStoreExitCause::Finalize(
                FiniteStoreFinalizeCause::Publication(error),
            )));
        }
        let failure_work = self.failure_work;
        let evidence_destination = self.evidence_destination;
        Ok(
            match self
                .pending
                .take()
                .expect("a valid central run selects one decision")
            {
                CentralPendingDecision::Candidate {
                    output,
                    proof,
                    closure,
                    canonical_root,
                    evidence_root,
                    destinations,
                } => FiniteCanonicalCentralValidationOutcome::Candidate(FiniteCentralCandidate {
                    owner: PayloadOwner::new(
                        proof,
                        PayloadOwner::new(
                            output,
                            ValidatedFiniteStoreSuccess::from_bound_fields(
                                fields,
                                closure,
                                canonical_root,
                                evidence_root,
                            ),
                        ),
                    ),
                    destinations,
                    failure_work,
                }),
                CentralPendingDecision::Rejected { cause, failure } => {
                    FiniteCanonicalCentralValidationOutcome::Rejected(FiniteCentralFailure {
                        owner: PayloadOwner::new(
                            cause,
                            FiniteStoreFailureAssembly::from_bound_fields(
                                fields,
                                state,
                                failure.failure_cause,
                                failure.forced_cause,
                                failure.evidence_root,
                                None,
                            ),
                        ),
                        failure_work,
                        evidence_destination,
                    })
                }
                CentralPendingDecision::Interrupted { cause, failure } => {
                    FiniteCanonicalCentralValidationOutcome::Interrupted(FiniteCentralFailure {
                        owner: PayloadOwner::new(
                            cause,
                            FiniteStoreFailureAssembly::from_bound_fields(
                                fields,
                                state,
                                failure.failure_cause,
                                failure.forced_cause,
                                failure.evidence_root,
                                None,
                            ),
                        ),
                        failure_work,
                        evidence_destination,
                    })
                }
                CentralPendingDecision::Fallback {
                    cause,
                    code,
                    provenance,
                } => FiniteCanonicalCentralValidationOutcome::Fallback(FiniteCentralFallback {
                    owner: PayloadOwner::new(
                        cause,
                        FiniteStoreFallbackOnly::from_bound_fields(fields, code, provenance, None),
                    ),
                }),
                CentralPendingDecision::Contain { .. } => {
                    unreachable!("containment is resolved before fields transfer")
                }
            },
        )
    }
}

impl<O, PP, U, T, Proof, E, CK, CT, EK, ET, L> Drop
    for FiniteCentralTransition<'_, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.publication.is_some() {
            self.contain();
        }
    }
}

impl<'host, O, PP, U, T, CK, CT, L> FiniteActivatedSession<'host, O, PP, U, T, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Runs shared evidence authoring and central validation against the actual finite fields.
    pub fn validate_centrally<EK, ET, Proof, E>(
        self,
        proof_slot: CanonicalProofSlot<'host, Proof>,
        validation_slots: CanonicalValidationSlots<'host, E>,
        progress: CanonicalValidationProgress<PP, U>,
        build_evidence: impl for<'borrow, 'lane> FnMut(
            &mut CanonicalOutcomeEvidenceLane<'borrow, 'lane, O, PP, U, L>,
            &T,
        ) -> OutcomeEvidenceDecision<EK, ET>,
        validate: impl for<'session, 'storage, 'scope, 'operation> FnMut(
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
        )
            -> Result<(), StoreWorkError>,
    ) -> FiniteCentralResult<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET>
    where
        CT: Copy + Send + Sync + 'static,
        ET: Copy + Send + Sync + 'static,
    {
        let FiniteActivatedParts {
            output,
            assembly,
            canonical_root,
            validation,
            failure_work,
            evidence_destination,
        } = self.into_central_parts();
        let mut transition = FiniteCentralTransition {
            publication: Some(FinitePublicationTransition::from_assembly(assembly)),
            output: Some(output),
            pending: None,
            failure_work,
            evidence_destination,
        };
        let valid = run_central_validation(
            transition
                .publication
                .as_mut()
                .expect("original finite central fields")
                .ordinary(),
            &mut transition.output,
            CentralValidationRequest {
                proof_slot,
                validation_slots,
                facts: CentralValidationFacts {
                    canonical_root,
                    validation,
                    evidence_destination,
                },
                progress,
            },
            &mut transition.failure_work,
            &mut transition.pending,
            build_evidence,
            validate,
        );
        transition.finish(valid)
    }
}
