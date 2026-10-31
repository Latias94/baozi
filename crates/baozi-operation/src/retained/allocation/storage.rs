//! Inline retained storage using the same release mechanism as shared allocations.
//!
//! This module is a child of `allocation`: construction stays inside the physical subsystem, while
//! the allocation's header can live in a containing table or output owner. A separately admitted
//! shared header is unnecessary for each owned path or vector.

use super::{
    AccountPoisonReason, AdmissionCharge, FinalOwnerReleaseOutcome, LeafRetainedPolicy,
    MAX_RETAINED_INLINE_BYTES, PendingRetainedAdmission, PhysicalStorageAccount,
    RetainedAllocation, RetainedAllocationError, RetainedAllocationPlan, RetainedBuildFailureCode,
    RetainedFinalOwnerInner, RetainedLeaseGuard, RetainedPayload, RetainedReleaseMode,
    UnarmedRetainedFinalOwner, map_faulted_pending_rollback, map_pending_rollback_or,
    map_retained_final_owner_error,
};
use crate::concrete_store::{StoreDimensionKey, StorePhaseKey};
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{LiveReservation, RetainedStorageOperation};
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// An inline storage header inseparable from its physical lease.
///
/// Its containing allocation covers the inline header. Its physical admission covers only
/// the planned backing allocations owned by `T`. Children with
/// their own storage owners must not also contribute to the parent's allocation plan.
///
/// No consuming payload accessor or generic mutable payload borrow is provided: either could move
/// backing storage out while leaving its lease behind. This owner is not a publication capability;
/// its operation attribution must remain coupled by the enclosing operation-owned output.
#[must_use = "retained storage must keep its payload and physical lease together"]
pub struct RetainedStorage<T> {
    inner: RetainedFinalOwnerInner<RetainedPayload<T>, LeafRetainedPolicy>,
    attribution:
        Option<LiveReservation<RetainedStorageOperation, StorePhaseKey, StoreDimensionKey>>,
}

/// A typed table whose immutable allocation can be shared across Scene generations.
///
/// The shared header uses the existing final-owner mechanism and is admitted once with the
/// table's backing capacities. Ordinary inline evidence storage does not require this header.
pub struct SharedRetainedStorage<T> {
    allocation: Option<RetainedAllocation<T>>,
    attribution:
        Option<LiveReservation<RetainedStorageOperation, StorePhaseKey, StoreDimensionKey>>,
}

impl<T> SharedRetainedStorage<T> {
    pub(crate) fn header_bytes() -> Result<u64, RetainedAllocationError> {
        let bytes = size_of::<RetainedPayload<T>>();
        if bytes > MAX_RETAINED_INLINE_BYTES {
            return Err(RetainedAllocationError::InlinePayloadTooLarge {
                bytes,
                limit: MAX_RETAINED_INLINE_BYTES,
            });
        }
        super::FinalOwnerAllocation::<RetainedFinalOwnerInner<RetainedPayload<T>, LeafRetainedPolicy>>::checked_layout_bytes()
            .map_err(map_retained_final_owner_error)
    }

    pub(crate) fn with_attribution(
        mut self,
        attribution: LiveReservation<RetainedStorageOperation, StorePhaseKey, StoreDimensionKey>,
    ) -> Self {
        self.attribution = Some(attribution);
        self
    }

    pub(crate) fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        self.allocation
            .as_ref()
            .is_some_and(|allocation| allocation.belongs_to(account))
    }

    pub(in crate::retained) fn allocation(&self) -> &RetainedAllocation<T> {
        self.allocation
            .as_ref()
            .expect("a live typed owner retains its allocation")
    }

    pub(in crate::retained) fn from_allocation(allocation: RetainedAllocation<T>) -> Self {
        Self {
            allocation: Some(allocation),
            attribution: None,
        }
    }

    pub fn prepare_retained_output(
        &mut self,
    ) -> Result<(), crate::concrete_store::StoreRuntimeError> {
        if let Some(attribution) = &mut self.attribution {
            attribution
                .retain_for_output()
                .map_err(crate::concrete_store::StoreRuntimeError::Ledger)?;
        }
        Ok(())
    }
}

impl<T> AsRef<T> for SharedRetainedStorage<T> {
    fn as_ref(&self) -> &T {
        self.allocation
            .as_ref()
            .expect("a live typed owner retains its allocation")
            .as_ref()
    }
}

impl<T> Drop for SharedRetainedStorage<T> {
    fn drop(&mut self) {
        let outcome = self
            .allocation
            .take()
            .map(RetainedAllocation::release_storage);
        if outcome.is_some_and(|outcome| !matches!(outcome, FinalOwnerReleaseOutcome::Clean))
            && let Some(attribution) = self.attribution.take()
        {
            std::mem::forget(attribution);
        }
    }
}

/// A pre-allocation admission consumed when its actual payload is bound.
pub(crate) struct RetainedStorageLease {
    guard: RetainedLeaseGuard,
    charge: AdmissionCharge,
}

impl RetainedStorageLease {
    pub(crate) fn try_bind_shared<T>(
        mut self,
        value: T,
    ) -> Result<SharedRetainedStorage<T>, RetainedAllocationError>
    where
        T: Send + Sync + 'static,
    {
        let unarmed =
            match UnarmedRetainedFinalOwner::<RetainedPayload<T>, LeafRetainedPolicy>::try_allocate(
            ) {
                Ok(unarmed) => unarmed,
                Err(cause) => {
                    let release = self.bind(value).release_before_attribution();
                    return Err(if matches!(release, FinalOwnerReleaseOutcome::Clean) {
                        map_retained_final_owner_error(cause)
                    } else {
                        RetainedAllocationError::ConstructionInvariantUnverified
                    });
                }
            };
        let lease = self
            .guard
            .lease
            .take()
            .expect("a retained admission owns its physical lease");
        let inner = unarmed.arm(
            RetainedPayload {
                value,
                charge: self.charge,
            },
            lease,
        );
        Ok(SharedRetainedStorage {
            allocation: Some(RetainedAllocation {
                inner,
                membership: None,
            }),
            attribution: None,
        })
    }
    pub(crate) fn record_unresolved(&self) {
        if let Some(lease) = &self.guard.lease {
            let _ = lease
                .owner
                .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
    }
    pub(crate) fn admitted_bytes(&self) -> u64 {
        self.charge.bytes
    }

    /// Qualifies unused backing-capacity headroom after the allocator's actual capacity is known.
    pub(crate) fn shrink_to(&mut self, bytes: u64) -> Result<(), RetainedAllocationError> {
        let released = self
            .charge
            .bytes
            .checked_sub(bytes)
            .ok_or(RetainedAllocationError::ConstructionInvariantUnverified)?;
        if released == 0 {
            return Ok(());
        }
        let lease = self
            .guard
            .lease
            .as_mut()
            .ok_or(RetainedAllocationError::ConstructionInvariantUnverified)?;
        lease
            .owner
            .release_retained_charge(AdmissionCharge::new(released, 0))
            .map_err(|_| RetainedAllocationError::ChargeReleaseUnverified)?;
        self.charge.bytes = bytes;
        lease.charge.bytes = bytes;
        Ok(())
    }
    pub(in crate::retained) fn from_admission(
        admission: &mut PendingRetainedAdmission<'_>,
    ) -> Self {
        let charge = admission.charge;
        let mut guard = RetainedLeaseGuard::unarmed();
        guard.arm(admission.commit_lease());
        Self { guard, charge }
    }

    pub(crate) fn bind<T>(mut self, value: T) -> RetainedStorage<T> {
        let mut inner = RetainedFinalOwnerInner::unarmed();
        inner.lease = std::mem::replace(&mut self.guard, RetainedLeaseGuard::unarmed());
        inner.value = Some(RetainedPayload {
            value,
            charge: self.charge,
        });
        RetainedStorage {
            inner,
            attribution: None,
        }
    }
}

impl<T> RetainedStorage<T> {
    /// Computes the physical charge for backing storage without a second shared header.
    ///
    /// The plan must come from the same qualified capacity plans consumed by the trusted builder.
    /// This is a physical-subsystem helper, not a caller-declared footprint API.
    pub(in crate::retained) fn checked_charge(
        plan: RetainedAllocationPlan,
    ) -> Result<AdmissionCharge, RetainedAllocationError> {
        let inline_bytes = size_of::<RetainedPayload<T>>();
        if inline_bytes > MAX_RETAINED_INLINE_BYTES {
            return Err(RetainedAllocationError::InlinePayloadTooLarge {
                bytes: inline_bytes,
                limit: MAX_RETAINED_INLINE_BYTES,
            });
        }
        Ok(AdmissionCharge::new(
            plan.nested_bytes,
            plan.nested_allocations,
        ))
    }

    /// Builds storage only after its operation and physical admissions have succeeded.
    ///
    /// Production callers obtain `admission` through the existing operation-owned writer path.
    /// The builder may allocate only its qualified backing storage, and a returned build failure
    /// certifies that every partially constructed allocation was destroyed. A builder panic has
    /// no such guarantee and uses the existing conservative containment path.
    pub(in crate::retained) fn try_build(
        account: &PhysicalStorageAccount,
        admission: &mut PendingRetainedAdmission<'_>,
        plan: RetainedAllocationPlan,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedAllocationError> {
        let charge = match Self::checked_charge(plan) {
            Ok(charge) => charge,
            Err(cause) => {
                return Err(map_pending_rollback_or(
                    admission.rollback_in_place(None),
                    cause,
                ));
            }
        };
        if charge != admission.charge || !account.is_same_account(admission.account) {
            return Err(map_faulted_pending_rollback(admission.rollback_in_place(
                Some(AccountPoisonReason::OwnershipGraphUnverified),
            )));
        }

        let value = match catch_unwind(AssertUnwindSafe(build)) {
            Ok(Ok(value)) => value,
            Ok(Err(cause)) => {
                return Err(map_pending_rollback_or(
                    admission.rollback_in_place(None),
                    RetainedAllocationError::Build(cause),
                ));
            }
            Err(payload) => {
                let _ = account.record_poison(AccountPoisonReason::RetainedBuilderPanicked);
                drop(admission.contain_with_lease_owner());
                discard_untrusted_panic_payload(payload);
                return Err(RetainedAllocationError::BuilderPanicked);
            }
        };

        let mut inner = RetainedFinalOwnerInner::unarmed();
        inner.lease.arm(admission.commit_lease());
        inner.value = Some(RetainedPayload { value, charge });
        Ok(Self {
            inner,
            attribution: None,
        })
    }

    pub(crate) fn with_attribution(
        mut self,
        attribution: LiveReservation<RetainedStorageOperation, StorePhaseKey, StoreDimensionKey>,
    ) -> Self {
        self.attribution = Some(attribution);
        self
    }

    /// Freezes the actual owner's operation attribution without ending its physical lease.
    pub fn prepare_retained_output(
        &mut self,
    ) -> Result<(), crate::concrete_store::StoreRuntimeError> {
        if let Some(attribution) = &mut self.attribution {
            attribution
                .retain_for_output()
                .map_err(crate::concrete_store::StoreRuntimeError::Ledger)?;
        }
        Ok(())
    }

    pub(crate) fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        self.inner.lease().owner.is_same_account(account)
    }

    pub(crate) fn charged_bytes(&self) -> u64 {
        self.inner.value().charge.bytes
    }

    pub(crate) fn charged_allocations(&self) -> u64 {
        self.inner.value().charge.allocations
    }

    /// Destroys backing storage before the enclosing operation releases its attribution.
    pub(in crate::retained) fn release_before_attribution(mut self) -> FinalOwnerReleaseOutcome {
        let outcome = self
            .inner
            .release_retained(RetainedReleaseMode::BeforeAttribution);
        if !matches!(outcome, FinalOwnerReleaseOutcome::Clean)
            && let Some(attribution) = self.attribution.take()
        {
            std::mem::forget(attribution);
        }
        outcome
    }
}

impl<T> AsRef<T> for RetainedStorage<T> {
    fn as_ref(&self) -> &T {
        &self.inner.value().value
    }
}

impl<T> Drop for RetainedStorage<T> {
    fn drop(&mut self) {
        let outcome = self
            .inner
            .release_retained(RetainedReleaseMode::OrdinarySharedDrop);
        if !matches!(outcome, FinalOwnerReleaseOutcome::Clean)
            && let Some(attribution) = self.attribution.take()
        {
            std::mem::forget(attribution);
        }
    }
}

#[cfg(test)]
mod tests;
