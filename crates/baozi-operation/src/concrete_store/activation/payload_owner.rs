//! Keep callback-owned values coupled to the storage they can invalidate while dropping.

use super::*;
use crate::control::discard_untrusted_panic_payload;

/// The concrete stage owner can quarantine its storage after a callback value fails to drop.
pub(super) trait PayloadStorage {
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

impl<O, PP, U, L> PayloadStorage for RetainedStorePublicationAssembly<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U> PayloadStorage for RawStoreFailureAssembly<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U, CK, CT, EK, ET> PayloadStorage for ValidatedStoreSuccess<O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U, CK, CT, EK, ET> PayloadStorage for StoreSuccessFreezePermit<O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U, EK, ET> PayloadStorage for StoreFailureAssembly<O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}

impl<O, PP, U, EK, ET> PayloadStorage for ValidatedStoreFailure<O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn contain_payload_failure(self) {
        let _ = self.contain_activation_fault();
    }
}
