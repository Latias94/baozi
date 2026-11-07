use super::*;
use crate::concrete_store::TerminalStoreLineage;
use crate::concrete_store::activation::canonical::finite::{
    FiniteCentralCandidate, FiniteCentralFailure, FiniteCentralFallback,
};
use crate::concrete_store::activation::final_state::CanonicalFinal;
use crate::concrete_store::activation::payload_owner::{PayloadLoan, PayloadOwner};
use crate::concrete_store::activation::{FiniteNeverActivatedSession, FiniteRejectedSession};

type FinalFiniteCandidate<'host, O, PP, U, T, Proof, CK, CT, EK, ET> =
    CanonicalFinal<FiniteCentralCandidate<'host, O, PP, U, T, Proof, CK, CT, EK, ET>>;
type FiniteCompletionResult<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> = Result<
    FiniteCanonicalInvocationCompletion<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>,
    FiniteStoreExitError<O, PP, U>,
>;

/// A semantic permission restored after the actual body and its captures have been disposed.
pub enum FiniteCanonicalInvocationOutcome<'host, O, PP, U, T, Proof, EA, EV, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Operation(FiniteCanonicalOperationCompletion<'host, O, PP, U>),
    Candidate(FinalFiniteCandidate<'host, O, PP, U, T, Proof, CK, CT, EK, ET>),
    NeverActivated(
        CanonicalFinal<FiniteNeverActivatedSession<'host, O, PP, U, EA, TerminalStoreLineage>>,
    ),
    Rejected(CanonicalFinal<FiniteRejectedSession<'host, O, PP, U, EA>>),
    CentralRejected(
        CanonicalFinal<FiniteCentralFailure<'host, O, PP, U, StoreCanonicalRejection<EV>, EK, ET>>,
    ),
    Interrupted(CanonicalFinal<FiniteCentralFailure<'host, O, PP, U, StoreWorkError, EK, ET>>),
    Fallback(CanonicalFinal<FiniteCentralFallback<'host, O, PP, U>>),
}

/// The original typed outcome and the Host's final generation and fallback slots.
pub struct FiniteCanonicalInvocationCompletion<
    'host,
    O,
    PP,
    U,
    T,
    Proof,
    EA,
    EV,
    EG,
    CK,
    CT,
    EK,
    ET,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub outcome:
        FiniteCanonicalInvocationOutcome<'host, O, PP, U, T, Proof, EA, EV, CK, CT, EK, ET>,
    pub generation: CanonicalGenerationSlots<'host, EG>,
    pub failure_fallback: CanonicalCauseSlot<'host, StoreRuntimeError>,
}

enum OperationRequest {
    EvidenceOnly,
    Minimal { code: u32 },
}

/// The original finite root and a request that needs no canonical activation.
pub struct FiniteCanonicalOperationCompletion<'host, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    root: &'host mut FiniteStoreInvocationRoot<O, PP, U>,
    request: OperationRequest,
}

impl<O, PP, U> FiniteCanonicalOperationCompletion<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn finish(
        self,
        progress: FinalProgress<PP, U>,
    ) -> Result<FiniteCanonicalOperationPublication, FiniteStoreExitError<O, PP, U>> {
        match self.request {
            OperationRequest::EvidenceOnly => self
                .root
                .fields()
                .prepare_evidence_only(progress)
                .map(FiniteCanonicalOperationPublication::EvidenceOnly),
            OperationRequest::Minimal { code } => self
                .root
                .fields()
                .prepare_minimal(code)
                .map(FiniteCanonicalOperationPublication::MinimalFallback),
        }
    }
}

fn clear_restoration_error<O, PP, U, T, Proof, EA, EV, EG>(
    fields: FinitePayloadFields<'_, T, Proof, EA, EV, EG>,
    mut error: FiniteStoreExitError<O, PP, U>,
) -> FiniteStoreExitError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let panic = fields.clear(|| {
        error.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
    });
    if let Some(payload) = panic {
        drop(error);
        std::panic::resume_unwind(payload);
    }
    error
}

impl<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
    FiniteCanonicalBoundInvocationFinish<'host, '_, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    CT: Copy + Send + Sync + 'static,
    ET: Copy + Send + Sync + 'static,
{
    /// Restores the exact original payload slots and finite stage without repeating validation.
    pub fn resume(
        self,
    ) -> FiniteCompletionResult<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> {
        let host = self.host;
        if matches!(host.pending, None | Some(FiniteDisposition::Unattached(_))) {
            host.record_payload_failure();
            let panic = host.clear_owned_values();
            host.contain_payload_failure();
            let error = host.take_error(FiniteStoreExitCause::Body(
                driver::CanonicalInvocationBodyError::invalid_disposition(),
            ));
            if let Some(payload) = panic {
                drop(error);
                std::panic::resume_unwind(payload);
            }
            return Err(error);
        }
        let FiniteCanonicalInvocationHost {
            never,
            activation_rejected,
            validation_rejected,
            interrupted,
            freeze_interrupted,
            generation_rejected,
            failure_fallback,
            central_fallback,
            output,
            proof,
            pending,
            order,
            store,
            ..
        } = host;
        let fields = FinitePayloadFields {
            never,
            activation_rejected,
            validation_rejected,
            interrupted,
            freeze_interrupted,
            generation_rejected,
            failure_fallback,
            central_fallback,
            output,
            proof,
            order,
        };
        let store = store
            .as_mut()
            .expect("the finish retains its original root shell");
        let identity = store.invocation_identity();
        macro_rules! restore {
            ($result:expr) => {
                match $result {
                    Ok(value) => value,
                    Err(error) => return Err(clear_restoration_error(fields, error)),
                }
            };
        }
        let outcome = match pending
            .take()
            .expect("the checked request remains installed")
        {
            FiniteDisposition::EvidenceOnly => {
                FiniteCanonicalInvocationOutcome::Operation(FiniteCanonicalOperationCompletion {
                    root: store,
                    request: OperationRequest::EvidenceOnly,
                })
            }
            FiniteDisposition::Minimal { code } => {
                FiniteCanonicalInvocationOutcome::Operation(FiniteCanonicalOperationCompletion {
                    root: store,
                    request: OperationRequest::Minimal { code },
                })
            }
            FiniteDisposition::Candidate(pending) => {
                let storage = restore!(store.resume_deferred_success(pending.store));
                FiniteCanonicalInvocationOutcome::Candidate(CanonicalFinal::new(
                    FiniteCentralCandidate {
                        owner: PayloadOwner::new(
                            PayloadLoan::new(fields.proof, fields.order, identity),
                            PayloadOwner::new(
                                PayloadLoan::new(fields.output, fields.order, identity),
                                storage,
                            ),
                        ),
                        destinations: pending.destinations,
                        failure_work: pending.failure_work,
                    },
                ))
            }
            FiniteDisposition::NeverActivated(pending) => {
                let storage = restore!(store.resume_deferred_operation(pending.store));
                FiniteCanonicalInvocationOutcome::NeverActivated(CanonicalFinal::new(
                    FiniteNeverActivatedSession::from_deferred(
                        PayloadLoan::new(fields.never, fields.order, identity),
                        storage,
                        pending.fallback_only,
                        pending.failure_code,
                        pending.outcome_evidence_plan,
                        pending.failure_work,
                        pending.destination,
                    ),
                ))
            }
            FiniteDisposition::Rejected(pending) => {
                let storage = restore!(store.resume_deferred_unrooted_failure(pending.store));
                FiniteCanonicalInvocationOutcome::Rejected(CanonicalFinal::new(
                    FiniteRejectedSession::from_deferred(
                        PayloadLoan::new(fields.activation_rejected, fields.order, identity),
                        storage,
                        pending.failure_code,
                        pending.failure_work,
                        pending.destination,
                    ),
                ))
            }
            FiniteDisposition::CentralRejected(pending) => {
                let storage = restore!(store.resume_deferred_failure(pending.store));
                FiniteCanonicalInvocationOutcome::CentralRejected(CanonicalFinal::new(
                    FiniteCentralFailure {
                        owner: PayloadOwner::new(
                            PayloadLoan::new(fields.validation_rejected, fields.order, identity),
                            storage,
                        ),
                        failure_work: pending.failure_work,
                        evidence_destination: pending.destination,
                    },
                ))
            }
            FiniteDisposition::Interrupted(pending) => {
                let storage = restore!(store.resume_deferred_failure(pending.store));
                FiniteCanonicalInvocationOutcome::Interrupted(CanonicalFinal::new(
                    FiniteCentralFailure {
                        owner: PayloadOwner::new(
                            PayloadLoan::new(fields.interrupted, fields.order, identity),
                            storage,
                        ),
                        failure_work: pending.failure_work,
                        evidence_destination: pending.destination,
                    },
                ))
            }
            FiniteDisposition::Fallback(pending) => {
                let storage = restore!(store.resume_deferred_fallback(pending));
                FiniteCanonicalInvocationOutcome::Fallback(CanonicalFinal::new(
                    FiniteCentralFallback {
                        owner: PayloadOwner::new(
                            PayloadLoan::new(fields.central_fallback, fields.order, identity),
                            storage,
                        ),
                    },
                ))
            }
            FiniteDisposition::Unattached(_) => {
                unreachable!("unattached is handled before lending")
            }
        };
        Ok(FiniteCanonicalInvocationCompletion {
            outcome,
            generation: CanonicalGenerationSlots {
                interruption: CanonicalCauseSlot {
                    value: fields.freeze_interrupted,
                    order: fields.order,
                    identity,
                },
                rejection: CanonicalCauseSlot {
                    value: fields.generation_rejected,
                    order: fields.order,
                    identity,
                },
            },
            failure_fallback: CanonicalCauseSlot {
                value: fields.failure_fallback,
                order: fields.order,
                identity,
            },
        })
    }
}
