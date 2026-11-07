use std::cell::Cell;

use super::payload_owner::{PayloadLoan, PayloadOrder};
use super::{
    ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE, CanonicalFreezeInterruption, OperationOwner,
    ProgressPhase, ProgressUnit, StoreCanonicalRejection, StoreNeverActivatedCause, StoreWorkError,
    discard_untrusted_panic_payload,
};
use crate::concrete_store::{StoreInvocationRoot, StoreRuntimeError};

mod body;
pub(in crate::concrete_store) mod finite;
pub use finite::{
    FiniteCanonicalBodyOperation, FiniteCanonicalBoundBodyParts, FiniteCanonicalBoundBodyStart,
    FiniteCanonicalBoundInvocationFinish, FiniteCanonicalDispositionSlot,
    FiniteCanonicalInvocationCompletion, FiniteCanonicalInvocationHost,
    FiniteCanonicalInvocationOutcome, FiniteCanonicalOperationCompletion,
    FiniteCanonicalOperationPublication,
};
pub(super) mod driver;
mod requests;
pub use body::{CanonicalBodyOperation, CanonicalBodyStart, CanonicalBoundBodyStart};
pub use driver::{
    CanonicalBoundBodyError, CanonicalBoundInvocationFinish, CanonicalDispositionSlot,
    CanonicalInvocationBodyError, CanonicalInvocationCompletion, CanonicalInvocationFinish,
    CanonicalInvocationOutcome, CanonicalOperationCompletion, CanonicalOperationPublication,
};

/// Owns actual canonical values beside their physical storage through every permission loan.
///
/// The startup permission cannot be taken as an independently owned root:
///
/// ```compile_fail,E0277
/// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn extract<O: OperationOwner, P: ProgressPhase, U: ProgressUnit, T, Proof>(
///     host: &mut CanonicalInvocationHost<O, P, U, T, Proof>,
/// ) {
///     let _ = host.run_body(|mut parts, _disposition| {
///         let _root = std::mem::take(&mut parts.startup);
///     });
/// }
/// ```
///
/// Nor can a fresh root replace the host's installed physical owner:
///
/// ```compile_fail,E0308
/// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
/// use baozi_operation::__retained_store_integration::authority::StoreInvocationRoot;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn replace<O: OperationOwner, P: ProgressPhase, U: ProgressUnit, T, Proof>(
///     host: &mut CanonicalInvocationHost<O, P, U, T, Proof>,
/// ) {
///     let _ = host.run_body(|mut parts, _disposition| {
///         let _root = std::mem::replace(
///             &mut parts.startup, StoreInvocationRoot::<O, P, U>::new());
///     });
/// }
/// ```
///
/// Body capabilities are lent only by `run_body`; no disjoint terminal shortcut remains:
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn bypass<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
/// ) {
///     let _ = host.split();
/// }
/// ```
#[doc = include_str!("host/contracts.md")]
pub struct CanonicalInvocationHost<
    O,
    PP,
    U,
    T,
    Proof,
    EA = (),
    EV = (),
    EG = (),
    CK = (),
    CT = (),
    EK = (),
    ET = (),
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    never: Option<StoreNeverActivatedCause<EA>>,
    activation_rejected: Option<StoreCanonicalRejection<EA>>,
    validation_rejected: Option<StoreCanonicalRejection<EV>>,
    interrupted: Option<StoreWorkError>,
    freeze_interrupted: Option<CanonicalFreezeInterruption>,
    generation_rejected: Option<StoreCanonicalRejection<EG>>,
    failure_fallback: Option<StoreRuntimeError>,
    central_fallback: Option<StoreRuntimeError>,
    output: Option<T>,
    proof: Option<Proof>,
    pending: Option<driver::PendingDisposition<CK, CT, EK, ET>>,
    body_entered: bool,
    order: Cell<PayloadOrder>,
    store: StoreInvocationRoot<O, PP, U>,
}

/// Short body permissions exclude final generation and fallback preparation.
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::CanonicalBodyParts;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn body<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     parts: CanonicalBodyParts<'_, O, P, U, (), (), (), ()>,
/// ) {
///     let _ = (parts.startup, parts.output, parts.proof, parts.activation, parts.validation);
/// }
/// ```
///
/// ```compile_fail,E0609
/// use baozi_operation::__retained_store_integration::activation::CanonicalBodyParts;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn body<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     parts: CanonicalBodyParts<'_, O, P, U, (), (), (), ()>,
/// ) {
///     let _ = parts.generation;
/// }
/// ```
///
/// ```compile_fail,E0609
/// use baozi_operation::__retained_store_integration::activation::CanonicalBodyParts;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn body<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     parts: CanonicalBodyParts<'_, O, P, U, (), (), (), ()>,
/// ) {
///     let _ = parts.failure_fallback;
/// }
/// ```
pub struct CanonicalInvocationParts<'inv, Startup, T, Proof, EA, EV> {
    pub startup: Startup,
    pub output: CanonicalOutputSlot<'inv, T>,
    pub proof: CanonicalProofSlot<'inv, Proof>,
    pub activation: CanonicalActivationSlots<'inv, EA>,
    pub validation: CanonicalValidationSlots<'inv, EV>,
}

pub type CanonicalBodyParts<'inv, O, PP, U, T, Proof, EA, EV> =
    CanonicalInvocationParts<'inv, CanonicalBodyStart<'inv, O, PP, U>, T, Proof, EA, EV>;

pub type CanonicalBoundBodyParts<
    'call,
    'entry,
    'account,
    O,
    PP,
    U,
    T,
    Proof,
    EA,
    EV,
    L = crate::concrete_store::FreshStoreLineage,
> = CanonicalInvocationParts<
    'call,
    CanonicalBoundBodyStart<'call, 'entry, 'account, O, PP, U, L>,
    T,
    Proof,
    EA,
    EV,
>;

pub struct CanonicalActivationSlots<'inv, E> {
    pub(super) never: CanonicalCauseSlot<'inv, StoreNeverActivatedCause<E>>,
    pub(super) rejection: CanonicalCauseSlot<'inv, StoreCanonicalRejection<E>>,
}

pub struct CanonicalValidationSlots<'inv, E> {
    pub(super) rejection: CanonicalCauseSlot<'inv, StoreCanonicalRejection<E>>,
    pub(super) interruption: CanonicalCauseSlot<'inv, StoreWorkError>,
}

pub struct CanonicalGenerationSlots<'inv, E> {
    pub interruption: CanonicalCauseSlot<'inv, CanonicalFreezeInterruption>,
    pub rejection: CanonicalCauseSlot<'inv, StoreCanonicalRejection<E>>,
}

pub struct CanonicalOutputSlot<'inv, T> {
    value: &'inv mut Option<T>,
    order: &'inv Cell<PayloadOrder>,
    identity: *const (),
}

pub struct CanonicalProofSlot<'inv, Proof> {
    value: &'inv mut Option<Proof>,
    order: &'inv Cell<PayloadOrder>,
    identity: *const (),
    fallback: Option<CanonicalCauseSlot<'inv, StoreRuntimeError>>,
}

/// Permission to install one typed recovery cause beside its physical invocation root.
pub struct CanonicalCauseSlot<'inv, Cause> {
    value: &'inv mut Option<Cause>,
    order: &'inv Cell<PayloadOrder>,
    identity: *const (),
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
    CanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn new() -> Self {
        Self {
            never: None,
            activation_rejected: None,
            validation_rejected: None,
            interrupted: None,
            freeze_interrupted: None,
            generation_rejected: None,
            failure_fallback: None,
            central_fallback: None,
            output: None,
            proof: None,
            pending: None,
            body_entered: false,
            order: Cell::new(PayloadOrder::ProofBeforeOutput),
            store: StoreInvocationRoot::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn store_for_test(&mut self) -> &mut StoreInvocationRoot<O, PP, U> {
        &mut self.store
    }

    #[allow(
        clippy::type_complexity,
        reason = "disjoint body permissions retain their actual payload, cause and root brands"
    )]
    fn parts_and_disposition(
        &mut self,
    ) -> (
        CanonicalBodyParts<'_, O, PP, U, T, Proof, EA, EV>,
        CanonicalDispositionSlot<'_, CK, CT, EK, ET>,
    ) {
        let identity = self.store.invocation_identity();
        (
            CanonicalBodyParts {
                startup: CanonicalBodyStart::new(self.store.startup()),
                output: CanonicalOutputSlot {
                    value: &mut self.output,
                    order: &self.order,
                    identity,
                },
                proof: CanonicalProofSlot {
                    value: &mut self.proof,
                    order: &self.order,
                    identity,
                    fallback: Some(CanonicalCauseSlot {
                        value: &mut self.central_fallback,
                        order: &self.order,
                        identity,
                    }),
                },
                activation: CanonicalActivationSlots {
                    never: CanonicalCauseSlot {
                        value: &mut self.never,
                        order: &self.order,
                        identity,
                    },
                    rejection: CanonicalCauseSlot {
                        value: &mut self.activation_rejected,
                        order: &self.order,
                        identity,
                    },
                },
                validation: CanonicalValidationSlots {
                    rejection: CanonicalCauseSlot {
                        value: &mut self.validation_rejected,
                        order: &self.order,
                        identity,
                    },
                    interruption: CanonicalCauseSlot {
                        value: &mut self.interrupted,
                        order: &self.order,
                        identity,
                    },
                },
            },
            CanonicalDispositionSlot {
                value: &mut self.pending,
                identity,
            },
        )
    }

    #[cfg(test)]
    pub(super) fn test_pending_fallback_cause(&self) -> Option<&StoreRuntimeError> {
        self.central_fallback.as_ref()
    }
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> Default
    for CanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<E> CanonicalActivationSlots<'_, E> {
    pub(super) fn is_vacant_for(&self, identity: *const ()) -> bool {
        self.never.is_vacant_for(identity) && self.rejection.is_vacant_for(identity)
    }
}

impl<E> CanonicalValidationSlots<'_, E> {
    pub(super) fn is_vacant_for(&self, identity: *const ()) -> bool {
        self.rejection.is_vacant_for(identity) && self.interruption.is_vacant_for(identity)
    }
}

impl<T> CanonicalOutputSlot<'_, T> {
    pub(super) fn is_vacant_for(&self, identity: *const ()) -> bool {
        self.identity == identity && self.value.is_none()
    }
}

impl<'inv, T> CanonicalOutputSlot<'inv, T> {
    pub(super) fn place(self, value: T) -> Result<PayloadLoan<'inv, T>, T> {
        if self.value.is_some() {
            return Err(value);
        }
        *self.value = Some(value);
        Ok(PayloadLoan::new(self.value, self.order, self.identity))
    }
}

impl<Proof> CanonicalProofSlot<'_, Proof> {
    pub(super) fn is_vacant_for(&self, identity: *const ()) -> bool {
        self.identity == identity
            && self.value.is_none()
            && self
                .fallback
                .as_ref()
                .is_some_and(|slot| slot.is_vacant_for(identity))
    }
}

impl<'inv, Proof> CanonicalProofSlot<'inv, Proof> {
    pub(super) fn take_fallback_slot(
        &mut self,
    ) -> Option<CanonicalCauseSlot<'inv, StoreRuntimeError>> {
        self.fallback.take()
    }

    pub(super) fn place(self, proof: Proof) -> Result<PayloadLoan<'inv, Proof>, Proof> {
        if self.value.is_some() {
            return Err(proof);
        }
        *self.value = Some(proof);
        Ok(PayloadLoan::new(self.value, self.order, self.identity))
    }
}

impl<Cause> CanonicalCauseSlot<'_, Cause> {
    pub(super) fn is_vacant_for(&self, identity: *const ()) -> bool {
        self.identity == identity && self.value.is_none()
    }
}

impl<'inv, Cause> CanonicalCauseSlot<'inv, Cause> {
    pub(super) fn place(self, cause: Cause) -> Result<PayloadLoan<'inv, Cause>, Cause> {
        if self.value.is_some() {
            return Err(cause);
        }
        *self.value = Some(cause);
        Ok(PayloadLoan::new(self.value, self.order, self.identity))
    }
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
    CanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn clear_owned_values(&mut self) -> Option<Box<dyn std::any::Any + Send>> {
        self.pending = None;
        self.central_fallback = None;
        self.failure_fallback = None;
        self.interrupted = None;
        self.freeze_interrupted = None;
        let mut first_panic = None;
        drop_host_value(&mut self.never, &mut self.store, &mut first_panic);
        drop_host_value(
            &mut self.activation_rejected,
            &mut self.store,
            &mut first_panic,
        );
        drop_host_value(
            &mut self.validation_rejected,
            &mut self.store,
            &mut first_panic,
        );
        drop_host_value(
            &mut self.generation_rejected,
            &mut self.store,
            &mut first_panic,
        );
        match self.order.get() {
            PayloadOrder::ProofBeforeOutput => {
                drop_host_value(&mut self.proof, &mut self.store, &mut first_panic);
                drop_host_value(&mut self.output, &mut self.store, &mut first_panic);
            }
            PayloadOrder::OutputBeforeProof => {
                drop_host_value(&mut self.output, &mut self.store, &mut first_panic);
                drop_host_value(&mut self.proof, &mut self.store, &mut first_panic);
            }
        }
        first_panic
    }
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> Drop
    for CanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let already_unwinding = std::thread::panicking();
        let first_panic = self.clear_owned_values();
        if first_panic.is_some() {
            // Every value is gone before its physical storage can be contained.
            self.store.contain_payload_failure();
        }
        if let Some(payload) = first_panic {
            if already_unwinding {
                discard_untrusted_panic_payload(payload);
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

fn drop_host_value<T, O, PP, U>(
    value: &mut Option<T>,
    store: &mut StoreInvocationRoot<O, PP, U>,
    first_panic: &mut Option<Box<dyn std::any::Any + Send>>,
) where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    super::payload_owner::drop_captured_value(value, first_panic, || {
        store.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
    });
}
