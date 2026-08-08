use super::account::{AccountPoisonReason, AccountPoisoned, AdmissionCharge, AdmissionError};
use super::final_owner::{
    FinalOwnerAllocation, FinalOwnerAllocationError, FinalOwnerInnerDrop,
    unreachable_final_owner_state,
};
#[cfg(test)]
use super::{
    AdmittedRetainedShareWork, take_injected_final_owner_allocation_failure,
    take_injected_final_owner_drop_panic, take_injected_membership_assignment_failure,
};
use super::{
    PendingRetainedAdmission, PhysicalStorageAccount, map_faulted_pending_rollback,
    map_pending_rollback_or,
};
use crate::control::discard_untrusted_panic_payload;
use std::borrow::Borrow;
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use triomphe::UniqueArc;

#[cfg(test)]
std::thread_local! {
    static RETAINED_FINAL_OWNER_ALLOCATOR_ENTRIES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

/// Maximum inline payload header moved out of the final shared allocation.
///
/// Input-proportional storage belongs in separately planned backing allocations and contributes to
/// [`RetainedAllocationPlan`]. This ceiling bounds the final-owner extraction stack move.
pub(crate) const MAX_RETAINED_INLINE_BYTES: usize = 4 * 1024;

pub(super) trait AdmissionOwner {
    fn release_admission(&self, charge: AdmissionCharge);
}

pub(super) struct AdmissionLease<O: AdmissionOwner> {
    pub(super) owner: O,
    charge: AdmissionCharge,
    armed: bool,
}

impl<O: AdmissionOwner> AdmissionLease<O> {
    pub(super) const fn new(owner: O, charge: AdmissionCharge) -> Self {
        Self {
            owner,
            charge,
            armed: true,
        }
    }
}

impl<O: AdmissionOwner> Drop for AdmissionLease<O> {
    fn drop(&mut self) {
        if self.armed {
            self.owner.release_admission(self.charge);
        }
    }
}

impl AdmissionLease<PhysicalStorageAccount> {
    fn release_retained_admission(&mut self) -> Result<(), AccountPoisonEventWitness> {
        let release = self.owner.release_retained_charge(self.charge);
        if release.is_ok() {
            self.armed = false;
        }
        release
    }
}

pub(super) struct RetainedLeaseGuard {
    pub(super) lease: Option<RetainedLease>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedLeaseReleaseOutcome {
    Clean,
    ChargeReleaseUnverified(AccountPoisonEventWitness),
}

impl RetainedLeaseGuard {
    pub(super) const fn unarmed() -> Self {
        Self { lease: None }
    }

    fn arm(&mut self, lease: RetainedLease) {
        self.lease = Some(lease);
    }

    fn get(&self) -> &RetainedLease {
        match self.lease.as_ref() {
            Some(lease) => lease,
            None => unreachable_final_owner_state(),
        }
    }

    fn release_after_verified_storage_drop(&mut self) -> RetainedLeaseReleaseOutcome {
        let Some(mut lease) = self.lease.take() else {
            return RetainedLeaseReleaseOutcome::Clean;
        };
        match catch_unwind(AssertUnwindSafe(|| lease.release_retained_admission())) {
            Ok(Ok(())) => {
                drop(lease);
                RetainedLeaseReleaseOutcome::Clean
            }
            Ok(Err(poisoned)) => {
                self.lease = Some(lease);
                RetainedLeaseReleaseOutcome::ChargeReleaseUnverified(poisoned)
            }
            Err(payload) => {
                let poisoned = record_account_poison_event(
                    &lease.owner,
                    AccountPoisonReason::RetainedStorageReleaseUnverified,
                );
                std::mem::forget(lease);
                discard_untrusted_panic_payload(payload);
                RetainedLeaseReleaseOutcome::ChargeReleaseUnverified(poisoned)
            }
        }
    }
}

impl Drop for RetainedLeaseGuard {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            std::mem::forget(lease);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinalOwnerReleaseOutcome {
    Clean,
    StorageReleaseUnverified(AccountPoisonEventWitness),
    ChargeReleaseUnverified(AccountPoisonEventWitness),
    OwnershipUnverified(AccountPoisonEventWitness),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainedValueReleaseOutcome {
    Clean,
    NestedStorageReleaseUnverified(AccountPoisonEventWitness),
    NestedChargeReleaseUnverified(AccountPoisonEventWitness),
    NestedOwnershipUnverified(AccountPoisonEventWitness),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainedReleaseMode {
    OrdinarySharedDrop,
    BeforeAttribution,
}

pub(super) type RetainedLease = AdmissionLease<PhysicalStorageAccount>;

pub(super) trait RetainedValuePolicy<T> {
    fn observe_pre_forget(value: &T, lease: &RetainedLease) -> AccountPoisonEventWitness;

    fn release(value: T, mode: RetainedReleaseMode) -> RetainedValueReleaseOutcome;
}

pub(super) struct LeafRetainedPolicy;

pub(super) struct RetainedFinalOwnerInner<T, P> {
    pub(super) value: Option<T>,
    lease: RetainedLeaseGuard,
    policy: PhantomData<fn() -> P>,
    #[cfg(test)]
    share_calls: AtomicUsize,
}

impl<T, P> RetainedFinalOwnerInner<T, P>
where
    P: RetainedValuePolicy<T>,
{
    const fn unarmed() -> Self {
        Self {
            value: None,
            lease: RetainedLeaseGuard::unarmed(),
            policy: PhantomData,
            #[cfg(test)]
            share_calls: AtomicUsize::new(0),
        }
    }

    fn value(&self) -> &T {
        match self.value.as_ref() {
            Some(value) => value,
            None => unreachable_final_owner_state(),
        }
    }

    fn lease(&self) -> &RetainedLease {
        self.lease.get()
    }

    fn release_retained(&mut self, mode: RetainedReleaseMode) -> FinalOwnerReleaseOutcome {
        let value = self.value.take();

        #[cfg(test)]
        if take_injected_final_owner_drop_panic() {
            if let Some(value) = value {
                let poisoned = P::observe_pre_forget(&value, self.lease.get());
                std::mem::forget(value);
                return FinalOwnerReleaseOutcome::StorageReleaseUnverified(poisoned);
            }
            return FinalOwnerReleaseOutcome::Clean;
        }

        if std::thread::panicking() {
            if let Some(value) = value {
                let poisoned = P::observe_pre_forget(&value, self.lease.get());
                std::mem::forget(value);
                return FinalOwnerReleaseOutcome::StorageReleaseUnverified(poisoned);
            }
            return FinalOwnerReleaseOutcome::Clean;
        }

        let Some(value) = value else {
            return FinalOwnerReleaseOutcome::Clean;
        };

        match catch_unwind(AssertUnwindSafe(|| P::release(value, mode))) {
            Ok(RetainedValueReleaseOutcome::Clean) => {
                match self.lease.release_after_verified_storage_drop() {
                    RetainedLeaseReleaseOutcome::Clean => FinalOwnerReleaseOutcome::Clean,
                    RetainedLeaseReleaseOutcome::ChargeReleaseUnverified(poisoned) => {
                        FinalOwnerReleaseOutcome::ChargeReleaseUnverified(poisoned)
                    }
                }
            }
            Ok(RetainedValueReleaseOutcome::NestedStorageReleaseUnverified(poisoned)) => {
                let _ = self.lease.release_after_verified_storage_drop();
                FinalOwnerReleaseOutcome::StorageReleaseUnverified(poisoned)
            }
            Ok(RetainedValueReleaseOutcome::NestedChargeReleaseUnverified(poisoned)) => {
                let _ = self.lease.release_after_verified_storage_drop();
                FinalOwnerReleaseOutcome::ChargeReleaseUnverified(poisoned)
            }
            Ok(RetainedValueReleaseOutcome::NestedOwnershipUnverified(poisoned)) => {
                let _ = self.lease.release_after_verified_storage_drop();
                FinalOwnerReleaseOutcome::OwnershipUnverified(poisoned)
            }
            Err(payload) => {
                let poisoned = observe_retained_payload_drop_failure(self.lease.get());
                discard_untrusted_panic_payload(payload);
                FinalOwnerReleaseOutcome::StorageReleaseUnverified(poisoned)
            }
        }
    }
}

impl<T, P> FinalOwnerInnerDrop for RetainedFinalOwnerInner<T, P>
where
    P: RetainedValuePolicy<T>,
{
    fn release_for_drop(&mut self) {
        let _ = self.release_retained(RetainedReleaseMode::OrdinarySharedDrop);
    }

    #[cfg(test)]
    fn record_share(&self) {
        self.share_calls.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn share_call_count(&self) -> usize {
        self.share_calls.load(Ordering::Relaxed)
    }
}

impl<T, P> FinalOwnerAllocation<RetainedFinalOwnerInner<T, P>>
where
    P: RetainedValuePolicy<T>,
{
    fn value(&self) -> &T {
        self.inner().value()
    }

    fn lease(&self) -> &RetainedLease {
        self.inner().lease()
    }

    fn release(self) -> FinalOwnerReleaseOutcome {
        let Some(mut inner) = self.into_unique_inner() else {
            return FinalOwnerReleaseOutcome::Clean;
        };
        let outcome = inner.release_retained(RetainedReleaseMode::OrdinarySharedDrop);
        drop(inner);
        outcome
    }

    pub(super) fn try_release_unique_before_attribution(
        self,
    ) -> Result<FinalOwnerReleaseOutcome, (Self, AccountPoisonEventWitness)> {
        match self.try_release_unique_before_attribution_deferred() {
            Ok(outcome) => Ok(outcome),
            Err(owner) => {
                let poisoned = record_account_poison_event(
                    &owner.lease().owner,
                    AccountPoisonReason::OwnershipGraphUnverified,
                );
                Err((owner, poisoned))
            }
        }
    }

    pub(super) fn try_release_unique_before_attribution_deferred(
        self,
    ) -> Result<FinalOwnerReleaseOutcome, Self> {
        let mut inner = match self.try_into_unique_inner()? {
            Some(inner) => inner,
            None => return Ok(FinalOwnerReleaseOutcome::Clean),
        };
        let outcome = inner.release_retained(RetainedReleaseMode::BeforeAttribution);
        drop(inner);
        Ok(outcome)
    }
}

pub(super) struct UnarmedRetainedFinalOwner<T, P>
where
    P: RetainedValuePolicy<T>,
{
    unique: UniqueArc<RetainedFinalOwnerInner<T, P>>,
}

impl<T, P> UnarmedRetainedFinalOwner<T, P>
where
    P: RetainedValuePolicy<T>,
{
    pub(super) fn try_allocate() -> Result<Self, FinalOwnerAllocationError> {
        #[cfg(test)]
        if take_injected_final_owner_allocation_failure() {
            return Err(FinalOwnerAllocationError::AllocationFailed);
        }

        #[cfg(test)]
        RETAINED_FINAL_OWNER_ALLOCATOR_ENTRIES.with(|entries| {
            entries.set(entries.get().saturating_add(1));
        });

        let unique = UniqueArc::try_new(RetainedFinalOwnerInner::unarmed())
            .map_err(|_| FinalOwnerAllocationError::AllocationFailed)?;
        Ok(Self { unique })
    }

    pub(super) fn arm(
        mut self,
        value: T,
        lease: RetainedLease,
    ) -> FinalOwnerAllocation<RetainedFinalOwnerInner<T, P>> {
        self.unique.lease.arm(lease);
        self.unique.value = Some(value);
        FinalOwnerAllocation::from_shared(self.unique.shareable())
    }
}

#[cfg(test)]
pub(crate) fn test_retained_final_owner_allocator_entries() -> usize {
    RETAINED_FINAL_OWNER_ALLOCATOR_ENTRIES.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_retained_final_owner_allocator_entries() {
    RETAINED_FINAL_OWNER_ALLOCATOR_ENTRIES.with(|entries| entries.set(0));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AccountPoisonEventWitness {
    pub(super) poisoned: AccountPoisoned,
    pub(super) reason: AccountPoisonReason,
}

impl AccountPoisonEventWitness {
    pub(super) const fn new(poisoned: AccountPoisoned, reason: AccountPoisonReason) -> Self {
        Self { poisoned, reason }
    }
}

pub(super) fn record_account_poison_event(
    account: &PhysicalStorageAccount,
    reason: AccountPoisonReason,
) -> AccountPoisonEventWitness {
    AccountPoisonEventWitness::new(account.record_poison(reason), reason)
}

impl PhysicalStorageAccount {
    /// Admits one retained allocation before running its constructing closure or entering the shared
    /// allocator.
    ///
    /// This module-private leaf grants no canonical Scene or publication authority. Its future
    /// operation-owned writer must additionally admit operation attribution before reaching it.
    /// Canonical callers also restrict `T` to audited ownership headers with non-panicking
    /// destructors; safe Rust cannot infer that property for an arbitrary generic type.
    #[allow(
        dead_code,
        reason = "the coupled retained writer will supply operation attribution before this leaf"
    )]
    pub(super) fn try_retain_with_admitted_handle<T>(
        &self,
        admission: &mut PendingRetainedAdmission<'_>,
        plan: RetainedAllocationPlan,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<RetainedAllocation<T>, RetainedAllocationError>
    where
        T: Send + Sync + 'static,
    {
        let charge = match self.checked_retained_charge::<T>(plan) {
            Ok(charge) => charge,
            Err(cause) => {
                return Err(map_pending_rollback_or(
                    admission.rollback_in_place(None),
                    cause,
                ));
            }
        };
        if charge != admission.charge || !self.is_same_account(admission.account) {
            return Err(map_faulted_pending_rollback(admission.rollback_in_place(
                Some(AccountPoisonReason::OwnershipGraphUnverified),
            )));
        }
        self.try_retain_with_admitted_charge::<T, LeafRetainedPolicy>(admission, build)
    }

    pub(super) fn try_retain_with_admitted_charge<T, P>(
        &self,
        admission: &mut PendingRetainedAdmission<'_>,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<PolicyRetainedAllocation<T, P>, RetainedAllocationError>
    where
        T: Send + Sync + 'static,
        P: RetainedValuePolicy<RetainedPayload<T>>,
    {
        if !self.is_same_account(admission.account) {
            return Err(map_faulted_pending_rollback(admission.rollback_in_place(
                Some(AccountPoisonReason::OwnershipGraphUnverified),
            )));
        }
        let unarmed = match UnarmedRetainedFinalOwner::<RetainedPayload<T>, P>::try_allocate() {
            Ok(unarmed) => unarmed,
            Err(error) => {
                return Err(map_pending_rollback_or(
                    admission.rollback_in_place(None),
                    map_retained_final_owner_error(error),
                ));
            }
        };
        let value = match catch_unwind(AssertUnwindSafe(build)) {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                drop(unarmed);
                return Err(map_pending_rollback_or(
                    admission.rollback_in_place(None),
                    RetainedAllocationError::Build(error),
                ));
            }
            Err(payload) => {
                // A builder can move nested storage into a side owner before panicking. The panic
                // payload type cannot prove that storage was released, even for `&str` or
                // `String`, so both physical admission and the caller's operation attribution
                // remain fail-closed.
                let _ = admission
                    .account
                    .record_poison(AccountPoisonReason::RetainedBuilderPanicked);
                let contained = admission.contain_with_lease_owner();
                drop(unarmed);
                drop(contained);
                discard_untrusted_panic_payload(payload);
                return Err(RetainedAllocationError::BuilderPanicked);
            }
        };
        let charge = admission.charge;
        let lease = admission.commit_lease();
        let payload = RetainedPayload { value, charge };
        let inner = unarmed.arm(payload, lease);
        Ok(PolicyRetainedAllocation {
            inner,
            membership: None,
        })
    }

    #[cfg(test)]
    pub(super) fn try_retain_with<T>(
        &self,
        plan: RetainedAllocationPlan,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<RetainedAllocation<T>, RetainedAllocationError>
    where
        T: Send + Sync + 'static,
    {
        let charge = self.checked_retained_charge::<T>(plan)?;
        let mut admission =
            self.try_admit_retained(charge, AdmittedRetainedShareWork { private: () })?;
        self.try_retain_with_admitted_handle(&mut admission, plan, build)
    }

    #[allow(
        dead_code,
        reason = "the coupled retained writer will preflight this charge before both admissions"
    )]
    pub(super) fn checked_retained_charge<T>(
        &self,
        plan: RetainedAllocationPlan,
    ) -> Result<AdmissionCharge, RetainedAllocationError> {
        self.checked_retained_charge_with_policy::<T, LeafRetainedPolicy>(plan)
    }

    pub(super) fn checked_retained_charge_with_policy<T, P>(
        &self,
        plan: RetainedAllocationPlan,
    ) -> Result<AdmissionCharge, RetainedAllocationError>
    where
        P: RetainedValuePolicy<RetainedPayload<T>>,
    {
        let inline_bytes = size_of::<RetainedPayload<T>>();
        if inline_bytes > MAX_RETAINED_INLINE_BYTES {
            return Err(RetainedAllocationError::InlinePayloadTooLarge {
                bytes: inline_bytes,
                limit: MAX_RETAINED_INLINE_BYTES,
            });
        }

        let outer_bytes = FinalOwnerAllocation::<
            RetainedFinalOwnerInner<RetainedPayload<T>, P>,
        >::checked_layout_bytes()
        .map_err(map_retained_final_owner_error)?;
        let bytes = outer_bytes
            .checked_add(plan.nested_bytes)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        let allocations = 1_u64
            .checked_add(plan.nested_allocations)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        Ok(AdmissionCharge::new(bytes, allocations))
    }
}

pub(super) fn observe_retained_payload_drop_failure(
    lease: &AdmissionLease<PhysicalStorageAccount>,
) -> AccountPoisonEventWitness {
    record_account_poison_event(
        &lease.owner,
        AccountPoisonReason::RetainedStorageReleaseUnverified,
    )
}

impl<T> RetainedValuePolicy<RetainedPayload<T>> for LeafRetainedPolicy {
    fn observe_pre_forget(
        _payload: &RetainedPayload<T>,
        lease: &RetainedLease,
    ) -> AccountPoisonEventWitness {
        observe_retained_payload_drop_failure(lease)
    }

    fn release(
        payload: RetainedPayload<T>,
        _mode: RetainedReleaseMode,
    ) -> RetainedValueReleaseOutcome {
        drop(payload);
        RetainedValueReleaseOutcome::Clean
    }
}

impl AdmissionOwner for PhysicalStorageAccount {
    fn release_admission(&self, charge: AdmissionCharge) {
        let _ = self.release_retained_charge(charge);
    }
}

/// Planned nested allocation envelope for one retained ownership header.
///
/// The complete admitted envelope remains charged for this leaf primitive's lifetime. A canonical
/// guard must derive it from qualified capacity plans rather than treating caller estimates as
/// observed allocator evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedAllocationPlan {
    nested_bytes: u64,
    nested_allocations: u64,
}

impl RetainedAllocationPlan {
    pub fn new(
        nested_bytes: u64,
        nested_allocations: u64,
    ) -> Result<Self, RetainedAllocationPlanError> {
        if (nested_bytes == 0) != (nested_allocations == 0) {
            return Err(RetainedAllocationPlanError::MismatchedNestedFootprint {
                bytes: nested_bytes,
                allocations: nested_allocations,
            });
        }
        Ok(Self {
            nested_bytes,
            nested_allocations,
        })
    }
}

/// Invalid nested retained-allocation evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum RetainedAllocationPlanError {
    MismatchedNestedFootprint { bytes: u64, allocations: u64 },
}

impl fmt::Display for RetainedAllocationPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MismatchedNestedFootprint { bytes, allocations } => write!(
                formatter,
                "nested retained footprint has {bytes} bytes but {allocations} allocations"
            ),
        }
    }
}

impl std::error::Error for RetainedAllocationPlanError {}

/// An allocation-free code returned when a retained payload builder fails.
///
/// Returning this code is an explicit recoverable-failure contract: the builder certifies that it
/// did not transfer nested storage to a side owner and that all builder-local storage was destroyed
/// while physical admission remained live. This concrete value type prevents moving an owning
/// `Vec`, `String`, `Arc`, static reference, or interior-mutable storage container through the error
/// value itself; it cannot prevent side effects, so trusted domain adapters must uphold the rest of
/// the contract. A panic supplies no such proof and always retains physical and operation charges
/// fail-closed. Domain layers reconstruct their typed error after this operation seam returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub(crate) struct RetainedBuildFailureCode(u32);

impl RetainedBuildFailureCode {
    pub const fn new(code: u32) -> Self {
        Self(code)
    }

    pub(crate) const fn code(self) -> u32 {
        self.0
    }
}

impl fmt::Display for RetainedBuildFailureCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

pub(super) struct RetainedPayload<T> {
    pub(super) value: T,
    pub(super) charge: AdmissionCharge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RetainedSetId(u64);

static NEXT_RETAINED_SET_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
std::thread_local! {
    static RETAINED_SET_ID_REQUESTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(super) fn test_retained_set_id_requests() -> usize {
    RETAINED_SET_ID_REQUESTS.with(std::cell::Cell::get)
}

pub(super) fn try_next_retained_set_id() -> Option<RetainedSetId> {
    #[cfg(test)]
    RETAINED_SET_ID_REQUESTS.with(|requests| {
        requests.set(requests.get().saturating_add(1));
    });
    NEXT_RETAINED_SET_ID
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .ok()
        .map(RetainedSetId)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainedMembershipDestination {
    Canonical,
    OutcomeEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RetainedMembership {
    pub(super) set_id: RetainedSetId,
    pub(super) destination: RetainedMembershipDestination,
    pub(super) ordinal: u64,
}

/// One module-private immutable payload whose physical charge follows its final shared owner.
pub(super) struct PolicyRetainedAllocation<T, P>
where
    P: RetainedValuePolicy<RetainedPayload<T>>,
{
    pub(super) inner: FinalOwnerAllocation<RetainedFinalOwnerInner<RetainedPayload<T>, P>>,
    pub(super) membership: Option<RetainedMembership>,
}

pub(super) type RetainedAllocation<T> = PolicyRetainedAllocation<T, LeafRetainedPolicy>;

impl<T, P> PolicyRetainedAllocation<T, P>
where
    P: RetainedValuePolicy<RetainedPayload<T>>,
{
    pub(super) fn release_storage(self) -> FinalOwnerReleaseOutcome {
        let Self {
            inner,
            membership: _,
        } = self;
        inner.release()
    }

    pub(super) fn try_release_unique_before_attribution(
        self,
    ) -> Result<FinalOwnerReleaseOutcome, (Self, AccountPoisonEventWitness)> {
        let Self { inner, membership } = self;
        match inner.try_release_unique_before_attribution() {
            Ok(outcome) => Ok(outcome),
            Err((inner, poisoned)) => Err((Self { inner, membership }, poisoned)),
        }
    }

    pub(super) fn try_release_unique_before_attribution_deferred(
        self,
    ) -> Result<FinalOwnerReleaseOutcome, Self> {
        let Self { inner, membership } = self;
        match inner.try_release_unique_before_attribution_deferred() {
            Ok(outcome) => Ok(outcome),
            Err(inner) => Err(Self { inner, membership }),
        }
    }

    pub(super) fn charged_bytes(&self) -> u64 {
        self.inner.value().charge.bytes
    }

    pub(super) fn charged_allocations(&self) -> u64 {
        self.inner.value().charge.allocations
    }

    pub(super) fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        self.inner.lease().owner.is_same_account(account)
    }

    pub(super) fn account(&self) -> &PhysicalStorageAccount {
        &self.inner.lease().owner
    }

    pub(super) fn assign_membership(&mut self, membership: RetainedMembership) -> Result<(), ()> {
        #[cfg(test)]
        if take_injected_membership_assignment_failure() {
            return Err(());
        }

        if self.membership.is_some() {
            return Err(());
        }
        self.membership = Some(membership);
        Ok(())
    }

    pub(super) fn membership(&self) -> Option<RetainedMembership> {
        self.membership
    }

    #[cfg(test)]
    pub(super) fn share_after_test_admission(&self) -> Self {
        Self {
            inner: self.inner.share(),
            membership: None,
        }
    }

    #[cfg(test)]
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.inner.ptr_eq(&other.inner)
    }

    #[cfg(test)]
    pub(super) fn share_call_count(&self) -> usize {
        self.inner.share_call_count()
    }

    #[cfg(test)]
    pub(super) fn allocation_identity(
        &self,
    ) -> *const RetainedFinalOwnerInner<RetainedPayload<T>, P> {
        self.inner.allocation_identity()
    }
}

impl<T, P> AsRef<T> for PolicyRetainedAllocation<T, P>
where
    P: RetainedValuePolicy<RetainedPayload<T>>,
{
    fn as_ref(&self) -> &T {
        &self.inner.value().value
    }
}

impl<T, P> Borrow<T> for PolicyRetainedAllocation<T, P>
where
    P: RetainedValuePolicy<RetainedPayload<T>>,
{
    fn borrow(&self) -> &T {
        self.as_ref()
    }
}

impl<T, P> fmt::Debug for PolicyRetainedAllocation<T, P>
where
    P: RetainedValuePolicy<RetainedPayload<T>>,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedAllocation")
            .field("charged_bytes", &self.charged_bytes())
            .field("charged_allocations", &self.charged_allocations())
            .finish_non_exhaustive()
    }
}

/// Failure before a retained allocation becomes observable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum RetainedAllocationError {
    InlinePayloadTooLarge {
        bytes: usize,
        limit: usize,
    },
    AccountPoisoned(AccountPoisoned),
    /// The account was poisoned after this retain admission; its physical charge remains coupled.
    AccountPoisonedWithRetainedCharge(AccountPoisoned),
    AccountingOverflow,
    ByteLimitExceeded {
        requested: u64,
        limit: u64,
    },
    AllocationLimitExceeded {
        requested: u64,
        limit: u64,
    },
    Build(RetainedBuildFailureCode),
    BuilderPanicked,
    /// A trusted retained payload constructor violated its prequalified shape contract.
    ConstructionInvariantUnverified,
    AllocationFailed,
    ChargeReleaseUnverified,
}

impl RetainedAllocationError {
    /// Whether failure containment retained the physical charge because storage release was not
    /// proven.
    pub const fn charge_retained(&self) -> bool {
        matches!(
            self,
            Self::BuilderPanicked
                | Self::ConstructionInvariantUnverified
                | Self::ChargeReleaseUnverified
                | Self::AccountPoisonedWithRetainedCharge(_)
        )
    }
}

impl fmt::Display for RetainedAllocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InlinePayloadTooLarge { bytes, limit } => write!(
                formatter,
                "retained inline payload uses {bytes} bytes, exceeding limit {limit}"
            ),
            Self::AccountPoisoned(cause) => cause.fmt(formatter),
            Self::AccountPoisonedWithRetainedCharge(cause) => {
                write!(
                    formatter,
                    "{cause}; retained physical charge remains fail-closed"
                )
            }
            Self::AccountingOverflow => {
                formatter.write_str("retained allocation accounting overflowed")
            }
            Self::ByteLimitExceeded { requested, limit } => write!(
                formatter,
                "retained bytes {requested} exceed physical account limit {limit}"
            ),
            Self::AllocationLimitExceeded { requested, limit } => write!(
                formatter,
                "retained allocation count {requested} exceeds physical account limit {limit}"
            ),
            Self::Build(failure) => {
                write!(formatter, "retained payload construction failed: {failure}")
            }
            Self::BuilderPanicked => formatter.write_str(
                "retained payload constructor panicked; physical charge remains retained",
            ),
            Self::ConstructionInvariantUnverified => formatter.write_str(
                "retained payload construction violated a qualified invariant; physical charge remains retained",
            ),
            Self::AllocationFailed => formatter.write_str("retained shared allocation failed"),
            Self::ChargeReleaseUnverified => formatter
                .write_str("retained physical charge release failed and remains fail-closed"),
        }
    }
}

impl std::error::Error for RetainedAllocationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AccountPoisoned(cause) | Self::AccountPoisonedWithRetainedCharge(cause) => {
                Some(cause)
            }
            _ => None,
        }
    }
}

#[allow(
    dead_code,
    reason = "physical retained admission is private until the coupled writer lands"
)]
pub(super) fn map_retained_admission_error(error: AdmissionError) -> RetainedAllocationError {
    match error {
        AdmissionError::AccountingOverflow => RetainedAllocationError::AccountingOverflow,
        AdmissionError::ByteLimitExceeded { requested, limit } => {
            RetainedAllocationError::ByteLimitExceeded { requested, limit }
        }
        AdmissionError::AllocationLimitExceeded { requested, limit } => {
            RetainedAllocationError::AllocationLimitExceeded { requested, limit }
        }
    }
}

#[allow(
    dead_code,
    reason = "physical retained admission is private until the coupled writer lands"
)]
pub(super) fn map_retained_final_owner_error(
    error: FinalOwnerAllocationError,
) -> RetainedAllocationError {
    match error {
        FinalOwnerAllocationError::AccountingOverflow => {
            RetainedAllocationError::AccountingOverflow
        }
        FinalOwnerAllocationError::AllocationFailed => RetainedAllocationError::AllocationFailed,
    }
}
