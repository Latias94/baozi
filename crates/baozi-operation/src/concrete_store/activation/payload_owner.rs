//! Keep callback-owned values coupled to the storage they can invalidate while dropping.

use super::*;
use crate::control::discard_untrusted_panic_payload;
use std::cell::Cell;

/// Disposes one original capture or payload without replacing an earlier unwind cause.
pub(super) fn drop_captured_value<T>(
    value: &mut Option<T>,
    first_panic: &mut Option<Box<dyn std::any::Any + Send>>,
    record_failure: impl FnOnce(),
) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(value.take()))) {
        record_failure();
        if first_panic.is_some() {
            discard_untrusted_panic_payload(payload);
        } else {
            *first_panic = Some(payload);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum PayloadOrder {
    ProofBeforeOutput,
    OutputBeforeProof,
}

pub(super) struct PayloadLoan<'inv, T> {
    value: &'inv mut Option<T>,
    order: &'inv Cell<PayloadOrder>,
    identity: *const (),
}

/// Keeps an in-flight permission under caught cleanup before its concrete owner is rebound.
pub(super) struct PayloadValue<T> {
    pub(super) value: Option<T>,
}

impl<T> PayloadValue<T> {
    pub(super) fn new(value: T) -> Self {
        Self { value: Some(value) }
    }

    pub(super) fn take(&mut self) -> Option<T> {
        self.value.take()
    }

    pub(super) fn dispose(&mut self) -> Result<(), ()> {
        drop_scoped_value(self.value.take())
    }
}

impl<T> Drop for PayloadValue<T> {
    fn drop(&mut self) {
        let _ = self.dispose();
    }
}

impl<'inv, T> PayloadLoan<'inv, T> {
    pub(super) const fn new(
        value: &'inv mut Option<T>,
        order: &'inv Cell<PayloadOrder>,
        identity: *const (),
    ) -> Self {
        Self {
            value,
            order,
            identity,
        }
    }

    pub(super) const fn value(&self) -> &T {
        match &*self.value {
            Some(value) => value,
            None => panic!("a live payload permission retains its host-owned value"),
        }
    }

    pub(super) const fn identity(&self) -> *const () {
        self.identity
    }

    pub(super) fn freeze_order(&self) {
        self.order.set(PayloadOrder::OutputBeforeProof);
    }

    pub(super) fn into_owned(self) -> T {
        self.value
            .take()
            .expect("the outer driver detaches a payload once")
    }

    pub(super) fn retain_in_host(self) {
        // This loan owns no value or storage. The invocation keeps its actual Option populated.
        let _loan = ManuallyDrop::new(self);
    }
}

impl<T> std::ops::Deref for PayloadLoan<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value()
    }
}

impl<T> Drop for PayloadLoan<'_, T> {
    fn drop(&mut self) {
        drop(self.value.take());
    }
}

/// A transition keeps both payloads alive while its storage owner changes typestate.
///
/// Both destructors run under separate unwind boundaries, including when a terminal guard
/// has already contained its physical graph. A secondary destructor cannot replace the
/// original transition panic or prevent the other payload from being destroyed.
pub(super) struct PayloadPair<T, Proof> {
    output: Option<T>,
    proof: Option<Proof>,
}

impl<T, Proof> PayloadPair<T, Proof> {
    pub(super) const fn new(output: T, proof: Proof) -> Self {
        Self {
            output: Some(output),
            proof: Some(proof),
        }
    }

    pub(super) fn into_parts(mut self) -> (T, Proof) {
        (
            self.output.take().expect("joint output is detached once"),
            self.proof.take().expect("joint proof is detached once"),
        )
    }
}

impl<T, Proof> Drop for PayloadPair<T, Proof> {
    fn drop(&mut self) {
        let already_unwinding = std::thread::panicking();
        let output_panic = catch_unwind(AssertUnwindSafe(|| drop(self.output.take()))).err();
        let proof_panic = catch_unwind(AssertUnwindSafe(|| drop(self.proof.take()))).err();
        let first = match (output_panic, proof_panic) {
            (Some(first), Some(second)) => {
                discard_untrusted_panic_payload(second);
                Some(first)
            }
            (first @ Some(_), None) | (None, first @ Some(_)) => first,
            (None, None) => None,
        };
        if let Some(payload) = first {
            if already_unwinding {
                discard_untrusted_panic_payload(payload);
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

/// The concrete stage owner can quarantine its storage after a callback value fails to drop.
pub trait PayloadStorage {
    fn contain_payload_failure(self);
}

/// No allocation or extra account handle: the value and its current storage owner move together.
pub(super) struct PayloadOwner<T, S: PayloadStorage> {
    parts: Option<(T, S)>,
}

impl<T, S: PayloadStorage> PayloadOwner<T, S> {
    pub(super) const fn new(value: T, storage: S) -> Self {
        Self {
            parts: Some((value, storage)),
        }
    }

    pub(super) const fn value(&self) -> &T {
        match &self.parts {
            Some((value, _)) => value,
            None => panic!("payload owner is consumed only as a whole"),
        }
    }

    pub(super) const fn storage(&self) -> &S {
        match &self.parts {
            Some((_, storage)) => storage,
            None => panic!("payload owner is consumed only as a whole"),
        }
    }

    pub(super) fn into_parts(mut self) -> (T, S) {
        self.parts.take().expect("payload owner is consumed once")
    }
}

impl<T, S: PayloadStorage> PayloadStorage for PayloadOwner<T, S> {
    fn contain_payload_failure(self) {
        let (value, storage) = self.into_parts();
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(value))) {
            discard_untrusted_panic_payload(payload);
        }
        storage.contain_payload_failure();
    }
}

impl<T, S: PayloadStorage> Drop for PayloadOwner<T, S> {
    fn drop(&mut self) {
        let Some((value, storage)) = self.parts.take() else {
            return;
        };
        let already_unwinding = std::thread::panicking();
        match catch_unwind(AssertUnwindSafe(|| drop(value))) {
            Ok(()) => drop(storage),
            Err(payload) => {
                storage.contain_payload_failure();
                if already_unwinding {
                    discard_untrusted_panic_payload(payload);
                } else {
                    std::panic::resume_unwind(payload);
                }
            }
        }
    }
}

impl<'inv, O, PP, U, L> PayloadStorage for RetainedStorePublicationAssembly<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U, L> PayloadStorage
    for crate::concrete_store::finite::FiniteRetainedStoreOperation<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U, L> PayloadStorage
    for crate::concrete_store::finite::FiniteRetainedStorePublicationAssembly<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U> PayloadStorage
    for crate::concrete_store::finite::FiniteUnrootedStoreFailure<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<'inv, O, PP, U> PayloadStorage for RawStoreFailureAssembly<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U> PayloadStorage for StoreFallbackOnly<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<'inv, O, PP, U, CK, CT, EK, ET> PayloadStorage
    for ValidatedStoreSuccess<'inv, O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<'inv, O, PP, U, CK, CT, EK, ET> PayloadStorage
    for StoreSuccessFreezePermit<'inv, O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<'inv, O, PP, U, EK, ET> PayloadStorage for StoreFailureAssembly<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<'inv, O, PP, U, CK, CT, EK, ET> PayloadStorage
    for StoreSuccessFreezeContribution<'inv, O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<'inv, O, PP, U, EK, ET> PayloadStorage for ValidatedStoreFailure<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U, CK, CT, EK, ET> PayloadStorage
    for crate::concrete_store::finite::ValidatedFiniteStoreSuccess<'_, O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.into_request_error();
    }
}

impl<O, PP, U, EK, ET> PayloadStorage
    for crate::concrete_store::finite::FiniteStoreFailureAssembly<'_, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.into_request_error();
    }
}

impl<O, PP, U> PayloadStorage
    for crate::concrete_store::finite::FiniteStoreFallbackOnly<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.into_request_error();
    }
}

impl<EK, ET> PayloadStorage for PreparedStoreFailureOutcome<EK, ET> {
    fn contain_payload_failure(self) {
        self.quarantine_cause_drop_failure();
    }
}
impl PayloadStorage for PreparedStoreFallback {
    fn contain_payload_failure(self) {
        self.quarantine_cause_drop_failure();
    }
}
impl<EK, ET> PayloadStorage for crate::concrete_store::PreparedFiniteStoreFailureOutcome<EK, ET> {
    fn contain_payload_failure(self) {
        self.quarantine_cause_drop_failure();
    }
}
impl PayloadStorage for crate::concrete_store::PreparedFiniteStoreFallback {
    fn contain_payload_failure(self) {
        self.quarantine_cause_drop_failure();
    }
}
impl<O, PP, U, EK, ET> PayloadStorage
    for crate::concrete_store::finite::ValidatedFiniteStoreFailure<'_, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.into_request_error();
    }
}
