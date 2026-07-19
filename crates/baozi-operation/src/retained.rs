//! Fallible physical storage accounting for retained operation allocations.

#![allow(
    clippy::result_large_err,
    reason = "bounded poison snapshots stay inline so containment never needs a failure-path allocation"
)]

mod account;
mod final_owner;
mod ownership;

use crate::control::discard_untrusted_panic_payload;
use crate::coordinator::{FinalProgress, OperationCoordinator};
use crate::ledger::{
    AttributionBatchCreateError, AttributionBatchPlan, AttributionReservationBatch,
    CanonicalOutput, FallbackBatchBaseline, Ledger, LedgerCause, LedgerLimits, LedgerSnapshot,
    OperationAttributionReservation, OutcomeEvidenceDestination, OutputCommitment,
    PublicationDestination, StartupAccountShareEnvelope, StartupAccountShareStep,
    StartupAttributionBatchPlans, StartupRetainedAttribution,
};
use crate::{
    CapacityPlan, ControlCause, OperationControl, OperationOwner, ProgressPhase, ProgressUnit,
};
pub use account::{
    AccountPoisonEvidence, AccountPoisonReason, AccountPoisoned, FinalizationContainmentError,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountFactorySnapshot,
    PhysicalStorageAccountLimits, PhysicalStorageAccountSnapshot, PhysicalStorageLimitError,
};
use account::{
    AdmissionCharge, AdmissionError, CounterState, lock_recovering_poison,
    quarantined_account_reason, release, release_locked, try_admit, try_admit_locked,
};
use final_owner::{
    FinalOwnerAllocation, FinalOwnerAllocationError, FinalOwnerInnerDrop,
    unreachable_final_owner_state,
};
use ownership::{
    DetachedRetainedPart, FallbackPublicationBaseline, PublishedPart, RetainedPart,
    RetainedPartCleanupError, RetainedStorageDropOutcome, StartupFallbackOwner,
    drop_retained_allocation_before_attribution, finish_pre_attribution_batch_cleanup,
    finish_pre_attribution_cleanup, map_inherited_cleanup_error,
};
#[cfg(test)]
use ownership::{RetainedStorageDropSeverity, join_pre_attribution_cleanup_cause};
use std::any::TypeId;
use std::borrow::Borrow;
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use triomphe::{Arc, UniqueArc};

/// Maximum inline payload header moved out of the final shared allocation.
///
/// Input-proportional storage belongs in separately planned backing allocations and contributes to
/// [`RetainedAllocationPlan`]. This ceiling bounds the final-owner extraction stack move.
pub(crate) const MAX_RETAINED_INLINE_BYTES: usize = 4 * 1024;

trait AdmissionOwner {
    fn release_admission(&self, charge: AdmissionCharge);
}

struct AdmissionLease<O: AdmissionOwner> {
    owner: O,
    charge: AdmissionCharge,
    armed: bool,
}

impl<O: AdmissionOwner> AdmissionLease<O> {
    const fn new(owner: O, charge: AdmissionCharge) -> Self {
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

struct RetainedLeaseGuard {
    lease: Option<RetainedLease>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedLeaseReleaseOutcome {
    Clean,
    ChargeReleaseUnverified(AccountPoisonEventWitness),
}

impl RetainedLeaseGuard {
    const fn unarmed() -> Self {
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

struct FactoryControlLeaseGuard {
    lease: Option<FactoryControlLease>,
}

impl FactoryControlLeaseGuard {
    const fn unarmed() -> Self {
        Self { lease: None }
    }

    fn arm(&mut self, lease: FactoryControlLease) {
        self.lease = Some(lease);
    }

    fn release_trusted_after_verified_value_drop(&mut self) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(lease))) {
            discard_untrusted_panic_payload(payload);
        }
    }
}

impl Drop for FactoryControlLeaseGuard {
    fn drop(&mut self) {
        self.release_trusted_after_verified_value_drop();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalOwnerReleaseOutcome {
    Clean,
    StorageReleaseUnverified(AccountPoisonEventWitness),
    ChargeReleaseUnverified(AccountPoisonEventWitness),
    OwnershipUnverified(AccountPoisonEventWitness),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedValueReleaseOutcome {
    Clean,
    NestedStorageReleaseUnverified(AccountPoisonEventWitness),
    NestedChargeReleaseUnverified(AccountPoisonEventWitness),
    NestedOwnershipUnverified(AccountPoisonEventWitness),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedReleaseMode {
    OrdinarySharedDrop,
    BeforeAttribution,
}

type RetainedLease = AdmissionLease<PhysicalStorageAccount>;

trait RetainedValuePolicy<T> {
    fn observe_pre_forget(value: &T, lease: &RetainedLease) -> AccountPoisonEventWitness;

    fn release(value: T, mode: RetainedReleaseMode) -> RetainedValueReleaseOutcome;
}

struct LeafRetainedPolicy;

struct TableRetainedPolicy;

struct RetainedFinalOwnerInner<T, P> {
    value: Option<T>,
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

    fn try_release_unique_before_attribution(
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

    fn try_release_unique_before_attribution_deferred(
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

impl<Block>
    FinalOwnerAllocation<
        RetainedFinalOwnerInner<RetainedPayload<RetainedBlockTable<Block>>, TableRetainedPolicy>,
    >
{
    fn table_mut_during_construction(&mut self) -> Option<&mut RetainedBlockTable<Block>> {
        let payload = self.inner_mut_if_unique()?.value.as_mut()?;
        Some(&mut payload.value)
    }
}

struct UnarmedRetainedFinalOwner<T, P>
where
    P: RetainedValuePolicy<T>,
{
    unique: UniqueArc<RetainedFinalOwnerInner<T, P>>,
}

impl<T, P> UnarmedRetainedFinalOwner<T, P>
where
    P: RetainedValuePolicy<T>,
{
    fn try_allocate() -> Result<Self, FinalOwnerAllocationError> {
        #[cfg(test)]
        if take_injected_final_owner_allocation_failure() {
            return Err(FinalOwnerAllocationError::AllocationFailed);
        }

        let unique = UniqueArc::try_new(RetainedFinalOwnerInner::unarmed())
            .map_err(|_| FinalOwnerAllocationError::AllocationFailed)?;
        Ok(Self { unique })
    }

    fn arm(
        mut self,
        value: T,
        lease: RetainedLease,
    ) -> FinalOwnerAllocation<RetainedFinalOwnerInner<T, P>> {
        self.unique.lease.arm(lease);
        self.unique.value = Some(value);
        FinalOwnerAllocation::from_shared(self.unique.shareable())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AccountPoisonEventWitness {
    poisoned: AccountPoisoned,
    reason: AccountPoisonReason,
}

impl AccountPoisonEventWitness {
    const fn new(poisoned: AccountPoisoned, reason: AccountPoisonReason) -> Self {
        Self { poisoned, reason }
    }
}

fn record_account_poison_event(
    account: &PhysicalStorageAccount,
    reason: AccountPoisonReason,
) -> AccountPoisonEventWitness {
    AccountPoisonEventWitness::new(account.record_poison(reason), reason)
}

struct FactoryState {
    limits: PhysicalStorageAccountFactoryLimits,
    counters: Mutex<CounterState>,
}

impl FactoryState {
    fn snapshot(&self) -> PhysicalStorageAccountFactorySnapshot {
        let counters = *lock_recovering_poison(&self.counters);
        PhysicalStorageAccountFactorySnapshot::from_counters(counters)
    }
}

impl AdmissionOwner for Arc<FactoryState> {
    fn release_admission(&self, charge: AdmissionCharge) {
        release(&self.counters, charge);
    }
}

/// Failure to allocate the host-owned fixed factory control object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhysicalStorageAccountFactoryCreateError {
    AllocationFailed,
}

impl fmt::Display for PhysicalStorageAccountFactoryCreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("physical storage account factory allocation failed")
    }
}

impl std::error::Error for PhysicalStorageAccountFactoryCreateError {}

/// Failure to admit or allocate one account-control block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhysicalStorageAccountCreateError {
    AccountingOverflow,
    ControlByteLimitExceeded { requested: u64, limit: u64 },
    LiveAccountLimitExceeded { requested: u64, limit: u64 },
    AllocationFailed,
}

impl fmt::Display for PhysicalStorageAccountCreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccountingOverflow => {
                formatter.write_str("account-control accounting overflowed")
            }
            Self::ControlByteLimitExceeded { requested, limit } => write!(
                formatter,
                "account-control bytes {requested} exceed factory limit {limit}"
            ),
            Self::LiveAccountLimitExceeded { requested, limit } => write!(
                formatter,
                "live account count {requested} exceeds factory limit {limit}"
            ),
            Self::AllocationFailed => formatter.write_str("account-control allocation failed"),
        }
    }
}

impl std::error::Error for PhysicalStorageAccountCreateError {}

/// Explicit host-owned authority for creating bounded physical storage accounts.
///
/// The factory's own fixed control block is the host recursion boundary. Every account-control block
/// created by it remains charged until the last account handle and retained-allocation lease drop.
pub struct PhysicalStorageAccountFactory {
    state: Arc<FactoryState>,
}

impl PhysicalStorageAccountFactory {
    pub fn try_new(
        limits: PhysicalStorageAccountFactoryLimits,
    ) -> Result<Self, PhysicalStorageAccountFactoryCreateError> {
        let state = Arc::try_new(FactoryState {
            limits,
            counters: Mutex::new(CounterState::default()),
        })
        .map_err(|_| PhysicalStorageAccountFactoryCreateError::AllocationFailed)?;
        Ok(Self { state })
    }

    pub fn limits(&self) -> PhysicalStorageAccountFactoryLimits {
        self.state.limits
    }

    pub fn snapshot(&self) -> PhysicalStorageAccountFactorySnapshot {
        self.state.snapshot()
    }

    pub fn try_create_account(
        &self,
        limits: PhysicalStorageAccountLimits,
    ) -> Result<PhysicalStorageAccount, PhysicalStorageAccountCreateError> {
        let bytes = FinalOwnerAllocation::<AccountControlInner>::checked_layout_bytes()
            .map_err(map_account_control_final_owner_error)?;
        let charge = AdmissionCharge::new(bytes, 1);
        let owner = Arc::clone(&self.state);
        try_admit(&owner.counters, owner.limits.counter_limits(), charge)
            .map_err(map_factory_admission_error)?;
        let lease = AdmissionLease::new(owner, charge);
        let unarmed = UnarmedAccountControlFinalOwner::try_allocate()
            .map_err(map_account_control_final_owner_error)?;
        let state = AccountState {
            limits,
            linear: Mutex::new(AccountLinearState::default()),
        };
        let inner = unarmed.arm(state, lease);
        Ok(PhysicalStorageAccount { inner })
    }
}

impl Clone for PhysicalStorageAccountFactory {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl fmt::Debug for PhysicalStorageAccountFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhysicalStorageAccountFactory")
            .field("limits", &self.limits())
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}

fn map_factory_admission_error(error: AdmissionError) -> PhysicalStorageAccountCreateError {
    match error {
        AdmissionError::AccountingOverflow => PhysicalStorageAccountCreateError::AccountingOverflow,
        AdmissionError::ByteLimitExceeded { requested, limit } => {
            PhysicalStorageAccountCreateError::ControlByteLimitExceeded { requested, limit }
        }
        AdmissionError::AllocationLimitExceeded { requested, limit } => {
            PhysicalStorageAccountCreateError::LiveAccountLimitExceeded { requested, limit }
        }
    }
}

fn map_account_control_final_owner_error(
    error: FinalOwnerAllocationError,
) -> PhysicalStorageAccountCreateError {
    match error {
        FinalOwnerAllocationError::AccountingOverflow => {
            PhysicalStorageAccountCreateError::AccountingOverflow
        }
        FinalOwnerAllocationError::AllocationFailed => {
            PhysicalStorageAccountCreateError::AllocationFailed
        }
    }
}

struct AccountState {
    limits: PhysicalStorageAccountLimits,
    linear: Mutex<AccountLinearState>,
}

type FactoryControlLease = AdmissionLease<Arc<FactoryState>>;

struct AccountControlInner {
    value: Option<AccountState>,
    lease: FactoryControlLeaseGuard,
    #[cfg(test)]
    share_calls: AtomicUsize,
}

impl AccountControlInner {
    const fn unarmed() -> Self {
        Self {
            value: None,
            lease: FactoryControlLeaseGuard::unarmed(),
            #[cfg(test)]
            share_calls: AtomicUsize::new(0),
        }
    }

    fn value(&self) -> &AccountState {
        match self.value.as_ref() {
            Some(value) => value,
            None => unreachable_final_owner_state(),
        }
    }

    fn release_trusted(&mut self) {
        let Some(value) = self.value.take() else {
            return;
        };
        {
            let _value = value;
        }
        self.lease.release_trusted_after_verified_value_drop();
    }
}

impl FinalOwnerInnerDrop for AccountControlInner {
    fn release_for_drop(&mut self) {
        self.release_trusted();
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

impl FinalOwnerAllocation<AccountControlInner> {
    fn value(&self) -> &AccountState {
        self.inner().value()
    }
}

struct UnarmedAccountControlFinalOwner {
    unique: UniqueArc<AccountControlInner>,
}

impl UnarmedAccountControlFinalOwner {
    fn try_allocate() -> Result<Self, FinalOwnerAllocationError> {
        #[cfg(test)]
        if take_injected_final_owner_allocation_failure() {
            return Err(FinalOwnerAllocationError::AllocationFailed);
        }

        let unique = UniqueArc::try_new(AccountControlInner::unarmed())
            .map_err(|_| FinalOwnerAllocationError::AllocationFailed)?;
        Ok(Self { unique })
    }

    fn arm(
        mut self,
        value: AccountState,
        lease: FactoryControlLease,
    ) -> FinalOwnerAllocation<AccountControlInner> {
        self.unique.lease.arm(lease);
        self.unique.value = Some(value);
        FinalOwnerAllocation::from_shared(self.unique.shareable())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum AccountPoisonState {
    #[default]
    Healthy,
    Poisoned {
        event_count: u128,
        count_exact: bool,
        event_domain_valid: bool,
        first_evidence: AccountPoisonEvidence,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AccountLinearState {
    counters: CounterState,
    admitted_retained_owners: u64,
    admitted_account_actions: u64,
    poison: AccountPoisonState,
}

impl AccountLinearState {
    fn snapshot(self) -> PhysicalStorageAccountSnapshot {
        let (
            poisoned,
            poison_event_count,
            poison_event_count_exact,
            poison_event_domain_valid,
            first_poison_evidence,
        ) = match self.poison {
            AccountPoisonState::Healthy => (false, 0, true, true, None),
            AccountPoisonState::Poisoned {
                event_count,
                count_exact,
                event_domain_valid,
                first_evidence,
            } => (
                true,
                event_count,
                count_exact,
                event_domain_valid,
                Some(first_evidence),
            ),
        };
        PhysicalStorageAccountSnapshot::from_parts(
            self.counters,
            poisoned,
            poison_event_count,
            poison_event_count_exact,
            poison_event_domain_valid,
            first_poison_evidence,
        )
    }

    fn poisoned_error(&self) -> Option<AccountPoisoned> {
        matches!(self.poison, AccountPoisonState::Poisoned { .. })
            .then(|| AccountPoisoned::new((*self).snapshot()))
    }

    fn record_poison(&mut self, reason: AccountPoisonReason) -> AccountPoisoned {
        let event_limit =
            u128::from(self.admitted_retained_owners) + u128::from(self.admitted_account_actions);
        self.poison = match self.poison {
            AccountPoisonState::Healthy => AccountPoisonState::Poisoned {
                event_count: 1,
                count_exact: true,
                event_domain_valid: event_limit >= 1,
                first_evidence: AccountPoisonEvidence::new(reason, 1),
            },
            AccountPoisonState::Poisoned {
                event_count,
                count_exact,
                event_domain_valid,
                first_evidence,
            } => match event_count.checked_add(1) {
                Some(next) => AccountPoisonState::Poisoned {
                    event_count: next,
                    count_exact,
                    event_domain_valid: event_domain_valid && next <= event_limit,
                    first_evidence,
                },
                None => AccountPoisonState::Poisoned {
                    event_count,
                    count_exact: false,
                    event_domain_valid: false,
                    first_evidence,
                },
            },
        };
        AccountPoisoned::new(self.snapshot())
    }
}

/// A single-use account-branded admission for work that does not add a physical charge.
///
/// The account lock admits the attempt and is released before generic ledger, ordering, allocator,
/// or reference-count code runs. Poison racing after admission therefore has explicit
/// admission-first semantics without exposing a reusable eligibility permit.
struct PendingAccountActionAdmission<'account> {
    account: &'account PhysicalStorageAccount,
}

/// Single-use proof that final publication eligibility linearized before account poison.
///
/// The proof is minted only while holding the account's linear-state lock. A later poison event is
/// therefore ordered after this capability and cannot invalidate the already admitted pure
/// preparation tail.
#[must_use = "publication eligibility must be consumed by the exact freeze handoff"]
struct PublicationEligibility {
    private: (),
}

/// Holds the account linearization lock while a caller performs non-mutating table/source
/// preflight.  The action admission is minted only after that preflight succeeds, so invalid
/// structure cannot consume account admission-domain evidence and a concurrent poison cannot slip
/// between the health check and the action transition.
struct AccountPreflightGuard<'account> {
    account: &'account PhysicalStorageAccount,
    state: MutexGuard<'account, AccountLinearState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountActionAdmissionError {
    AccountPoisoned(AccountPoisoned),
    AccountingOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationOpenAdmissionError {
    Account(AccountActionAdmissionError),
    MembershipExhausted,
}

impl PendingAccountActionAdmission<'_> {
    fn account(&self) -> &PhysicalStorageAccount {
        self.account
    }

    fn run<T>(self, action: impl FnOnce(&PhysicalStorageAccount) -> T) -> T {
        action(self.account)
    }

    fn release(self) {}

    fn complete(self) {
        let Self { account: _ } = self;
    }
}

impl<'account> AccountPreflightGuard<'account> {
    fn admit_action(
        self,
    ) -> Result<PendingAccountActionAdmission<'account>, AccountActionAdmissionError> {
        let mut this = self;
        let admitted = this
            .state
            .admitted_account_actions
            .checked_add(1)
            .ok_or(AccountActionAdmissionError::AccountingOverflow)?;
        this.state.admitted_account_actions = admitted;
        drop(this.state);
        Ok(PendingAccountActionAdmission {
            account: this.account,
        })
    }
}

struct PendingRetainedAdmission<'account> {
    account: &'account PhysicalStorageAccount,
    charge: AdmissionCharge,
    state: PendingRetainedAdmissionState,
    /// The one account handle reserved for commit, rollback, or containment.
    ///
    /// This owner is created at admission time, after its share work has been admitted.  No
    /// failure path is allowed to acquire another account clone.
    lease_owner: Option<PhysicalStorageAccount>,
}

enum PendingRetainedAdmissionState {
    Pending,
    RolledBack,
    Transferred,
    Contained(RetainedLeaseGuard),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRetainedRollback {
    Clean,
    FaultRecorded(AccountPoisoned),
    ChargeReleaseUnverified(AccountPoisoned),
    /// The admission linearized before an external poison event; its charge is not refundable.
    ChargeRetainedAfterPoison(AccountPoisoned),
}

impl PendingRetainedRollback {
    const fn charge_released(self) -> bool {
        matches!(self, Self::Clean | Self::FaultRecorded(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetainedAdmissionPairPlan {
    first: AdmissionCharge,
    second: AdmissionCharge,
    aggregate: AdmissionCharge,
}

impl RetainedAdmissionPairPlan {
    fn try_new(
        first: AdmissionCharge,
        second: AdmissionCharge,
    ) -> Result<Self, RetainedAllocationError> {
        let bytes = first
            .bytes
            .checked_add(second.bytes)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        let allocations = first
            .allocations
            .checked_add(second.allocations)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        Ok(Self {
            first,
            second,
            aggregate: AdmissionCharge::new(bytes, allocations),
        })
    }
}

struct PendingStartRetainedAdmissions<'account> {
    account: &'account PhysicalStorageAccount,
    plan: RetainedAdmissionPairPlan,
    state: PendingStartRetainedAdmissionState,
    containment_owner: Option<PhysicalStorageAccount>,
}

enum PendingStartRetainedAdmissionState {
    Pending,
    Transferred,
    RolledBack,
    Contained(RetainedLeaseGuard),
}

struct StartupAdmissionGuard<'account> {
    account: &'account PhysicalStorageAccount,
    admissions: Option<PendingStartRetainedAdmissions<'account>>,
    evidence_account: Option<PhysicalStorageAccount>,
    fallback_lease_owner: Option<PhysicalStorageAccount>,
    evidence_lease_owner: Option<PhysicalStorageAccount>,
}

/// One pre-admitted clone/retained-handle pair for an ordinary retained writer.
///
/// The capability is consumed exactly once by the physical admission that performs the matching
/// account/allocation clone.  Keeping it typed prevents rollback paths from manufacturing a new
/// account handle after the work boundary.
struct AdmittedRetainedShareWork {
    private: (),
}

struct StartupAccountOwners {
    fallback_account: PhysicalStorageAccount,
    evidence_account: PhysicalStorageAccount,
    fallback_lease_owner: PhysicalStorageAccount,
    evidence_lease_owner: PhysicalStorageAccount,
}

struct StartupRemainingOwners {
    evidence_account: PhysicalStorageAccount,
    fallback_lease_owner: PhysicalStorageAccount,
    evidence_lease_owner: PhysicalStorageAccount,
}

impl AdmittedRetainedShareWork {
    fn consume_account(self, account: &PhysicalStorageAccount) -> PhysicalStorageAccount {
        let Self { private: () } = self;
        PhysicalStorageAccount {
            inner: account.inner.share(),
        }
    }

    fn consume_allocation<T>(self, allocation: &RetainedAllocation<T>) -> RetainedAllocation<T> {
        let Self { private: () } = self;
        allocation.share_after_admitted_work()
    }
}

impl<'account> StartupAdmissionGuard<'account> {
    fn new(
        account: &'account PhysicalStorageAccount,
        admissions: PendingStartRetainedAdmissions<'account>,
        owners: StartupRemainingOwners,
    ) -> Self {
        let StartupRemainingOwners {
            evidence_account,
            fallback_lease_owner,
            evidence_lease_owner,
        } = owners;
        Self {
            account,
            admissions: Some(admissions),
            evidence_account: Some(evidence_account),
            fallback_lease_owner: Some(fallback_lease_owner),
            evidence_lease_owner: Some(evidence_lease_owner),
        }
    }

    fn rollback(mut self, outer_reason: Option<AccountPoisonReason>) -> PendingRetainedRollback {
        match self.admissions.take() {
            Some(admissions) => admissions.rollback_with_fault(outer_reason),
            None => unreachable_final_owner_state(),
        }
    }
}

impl Drop for StartupAdmissionGuard<'_> {
    fn drop(&mut self) {
        let Some(admissions) = self.admissions.take() else {
            return;
        };
        let _ = admissions.rollback_with_fault(Some(AccountPoisonReason::OwnershipGraphUnverified));
    }
}

impl<'account> PendingStartRetainedAdmissions<'account> {
    fn into_parts(
        mut self,
        second_account: PhysicalStorageAccount,
        first_lease_owner: PhysicalStorageAccount,
        second_lease_owner: PhysicalStorageAccount,
    ) -> (
        PhysicalStorageAccount,
        PendingRetainedAdmission<'account>,
        PhysicalStorageAccount,
        PendingRetainedAdmission<'account>,
    ) {
        let first_account = match self.containment_owner.take() {
            Some(owner) => owner,
            None => unreachable_final_owner_state(),
        };
        self.state = PendingStartRetainedAdmissionState::Transferred;
        (
            first_account,
            PendingRetainedAdmission {
                account: self.account,
                charge: self.plan.first,
                state: PendingRetainedAdmissionState::Pending,
                lease_owner: Some(first_lease_owner),
            },
            second_account,
            PendingRetainedAdmission {
                account: self.account,
                charge: self.plan.second,
                state: PendingRetainedAdmissionState::Pending,
                lease_owner: Some(second_lease_owner),
            },
        )
    }

    fn rollback(self) -> bool {
        self.rollback_with_fault(None).charge_released()
    }

    fn transfer_to_containment(mut self) -> RetainedLeaseGuard {
        let owner = self.containment_owner.take();
        self.state = PendingStartRetainedAdmissionState::Transferred;
        RetainedLeaseGuard {
            lease: owner.map(|owner| AdmissionLease::new(owner, self.plan.aggregate)),
        }
    }

    fn rollback_with_fault(
        mut self,
        outer_reason: Option<AccountPoisonReason>,
    ) -> PendingRetainedRollback {
        let rollback = self
            .account
            .rollback_pending_retained_charge(self.plan.aggregate, outer_reason);
        match rollback {
            PendingRetainedRollback::ChargeReleaseUnverified(_)
            | PendingRetainedRollback::ChargeRetainedAfterPoison(_) => {
                self.state = PendingStartRetainedAdmissionState::Contained(RetainedLeaseGuard {
                    lease: self
                        .containment_owner
                        .take()
                        .map(|owner| AdmissionLease::new(owner, self.plan.aggregate)),
                });
            }
            PendingRetainedRollback::Clean | PendingRetainedRollback::FaultRecorded(_) => {
                self.state = PendingStartRetainedAdmissionState::RolledBack;
                drop(self.containment_owner.take());
            }
        }
        rollback
    }
}

impl Drop for PendingStartRetainedAdmissions<'_> {
    fn drop(&mut self) {
        if matches!(&self.state, PendingStartRetainedAdmissionState::Pending) {
            let rollback = self
                .account
                .rollback_pending_retained_charge(self.plan.aggregate, None);
            if matches!(
                rollback,
                PendingRetainedRollback::ChargeReleaseUnverified(_)
                    | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
            ) {
                self.state = PendingStartRetainedAdmissionState::Contained(RetainedLeaseGuard {
                    lease: self
                        .containment_owner
                        .take()
                        .map(|owner| AdmissionLease::new(owner, self.plan.aggregate)),
                });
            } else {
                self.state = PendingStartRetainedAdmissionState::RolledBack;
                drop(self.containment_owner.take());
            }
        }
    }
}

impl PendingRetainedAdmission<'_> {
    const fn is_pending(&self) -> bool {
        matches!(&self.state, PendingRetainedAdmissionState::Pending)
    }

    fn take_lease_owner(&mut self) -> PhysicalStorageAccount {
        if !self.is_pending() {
            unreachable_final_owner_state();
        }
        match self.lease_owner.take() {
            Some(owner) => owner,
            None => unreachable_final_owner_state(),
        }
    }

    fn commit_lease(&mut self) -> AdmissionLease<PhysicalStorageAccount> {
        let owner = self.take_lease_owner();
        self.state = PendingRetainedAdmissionState::Transferred;
        AdmissionLease::new(owner, self.charge)
    }

    fn rollback(self) -> bool {
        self.rollback_with_fault(None).charge_released()
    }

    fn rollback_with_fault(
        mut self,
        outer_reason: Option<AccountPoisonReason>,
    ) -> PendingRetainedRollback {
        self.rollback_in_place(outer_reason)
    }

    fn rollback_in_place(
        &mut self,
        outer_reason: Option<AccountPoisonReason>,
    ) -> PendingRetainedRollback {
        if !self.is_pending() {
            unreachable_final_owner_state();
        }
        let owner = self.take_lease_owner();
        let rollback = self
            .account
            .rollback_pending_retained_charge(self.charge, outer_reason);
        match rollback {
            PendingRetainedRollback::ChargeReleaseUnverified(_)
            | PendingRetainedRollback::ChargeRetainedAfterPoison(_) => {
                self.state = PendingRetainedAdmissionState::Contained(RetainedLeaseGuard {
                    lease: Some(AdmissionLease::new(owner, self.charge)),
                });
            }
            PendingRetainedRollback::Clean | PendingRetainedRollback::FaultRecorded(_) => {
                self.state = PendingRetainedAdmissionState::RolledBack;
                drop(owner);
            }
        }
        rollback
    }

    /// Releases a startup charge without manufacturing a new account handle. On an unverified
    /// release the admission remains pending so the enclosing side owner can move its pre-held
    /// account witness and typed batch into one containment object.
    fn rollback_startup_in_place(
        &mut self,
        outer_reason: Option<AccountPoisonReason>,
    ) -> PendingRetainedRollback {
        if !self.is_pending() {
            unreachable_final_owner_state();
        }
        let rollback = self
            .account
            .rollback_pending_retained_charge(self.charge, outer_reason);
        if matches!(
            rollback,
            PendingRetainedRollback::Clean | PendingRetainedRollback::FaultRecorded(_)
        ) {
            self.state = PendingRetainedAdmissionState::RolledBack;
        }
        rollback
    }

    fn mark_released_by_aggregate_rollback(&mut self) {
        if self.is_pending() {
            self.state = PendingRetainedAdmissionState::RolledBack;
        }
    }

    fn transfer_to_aggregate_containment(&mut self) -> PhysicalStorageAccount {
        if self.is_pending() {
            let owner = self.take_lease_owner();
            self.state = PendingRetainedAdmissionState::Transferred;
            return owner;
        }
        unreachable_final_owner_state();
    }

    fn contain_with_lease_owner(&mut self) -> RetainedLeaseGuard {
        if !self.is_pending() {
            unreachable_final_owner_state();
        }
        let owner = self.take_lease_owner();
        self.state = PendingRetainedAdmissionState::Transferred;
        RetainedLeaseGuard {
            lease: Some(AdmissionLease::new(owner, self.charge)),
        }
    }

    fn take_contained_lease(&mut self) -> Option<RetainedLeaseGuard> {
        let state = std::mem::replace(&mut self.state, PendingRetainedAdmissionState::Transferred);
        match state {
            PendingRetainedAdmissionState::Contained(lease) => Some(lease),
            state => {
                self.state = state;
                None
            }
        }
    }
}

impl Drop for PendingRetainedAdmission<'_> {
    fn drop(&mut self) {
        if self.is_pending() {
            let owner = match self.lease_owner.take() {
                Some(owner) => owner,
                None => {
                    let _ = self
                        .account
                        .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                    return;
                }
            };
            let rollback = self
                .account
                .rollback_pending_retained_charge(self.charge, None);
            if matches!(
                rollback,
                PendingRetainedRollback::ChargeReleaseUnverified(_)
                    | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
            ) {
                self.state = PendingRetainedAdmissionState::Contained(RetainedLeaseGuard {
                    lease: Some(AdmissionLease::new(owner, self.charge)),
                });
            } else {
                self.state = PendingRetainedAdmissionState::RolledBack;
                drop(owner);
            }
        }
    }
}

/// One opaque physical owner for retained allocations across operation and Scene generations.
///
/// Account identity is observable only through same-account predicates. It is not a semantic Scene
/// value and is intentionally absent from equality, ordering, serialization, and digest APIs.
/// Generic retained construction authority remains crate-private until a concrete canonical store
/// owns that seam:
///
/// ```compile_fail,E0432
/// use baozi_operation::RetainedOperation;
/// ```
///
/// ```compile_fail,E0432
/// use baozi_operation::RetainedAllocationPlan;
/// ```
///
/// ```compile_fail,E0432
/// use baozi_operation::BlockEmplacer;
/// ```
///
/// ```compile_fail,E0432
/// use baozi_operation::RetainedPart;
/// ```
///
/// ```compile_fail,E0432
/// use baozi_operation::RetainedClosureValidationError;
/// ```
///
/// ```compile_fail,E0432
/// use baozi_operation::PublishedRetainedAggregate;
/// ```
///
/// ```compile_fail,E0432
/// use baozi_operation::RetainedSuccessClosureProof;
/// ```
pub struct PhysicalStorageAccount {
    inner: FinalOwnerAllocation<AccountControlInner>,
}

impl PhysicalStorageAccount {
    pub fn limits(&self) -> PhysicalStorageAccountLimits {
        self.inner.value().limits
    }

    pub fn snapshot(&self) -> PhysicalStorageAccountSnapshot {
        (*lock_recovering_poison(&self.inner.value().linear)).snapshot()
    }

    pub fn is_same_account(&self, other: &Self) -> bool {
        self.inner.ptr_eq(&other.inner)
    }

    fn unverified_drop_count(&self) -> Option<u128> {
        Some(self.snapshot().poison_event_count())
    }

    #[cfg(test)]
    pub(crate) fn test_unverified_drop_count(&self) -> Option<u128> {
        self.unverified_drop_count()
    }

    #[cfg(test)]
    pub(super) fn test_record_poison(&self, reason: AccountPoisonReason) -> AccountPoisoned {
        self.record_poison(reason)
    }

    fn record_poison(&self, reason: AccountPoisonReason) -> AccountPoisoned {
        lock_recovering_poison(&self.inner.value().linear).record_poison(reason)
    }

    fn try_admit_account_action(
        &self,
    ) -> Result<PendingAccountActionAdmission<'_>, AccountActionAdmissionError> {
        self.try_begin_account_preflight()
            .map_err(AccountActionAdmissionError::AccountPoisoned)?
            .admit_action()
    }

    fn try_begin_account_preflight(&self) -> Result<AccountPreflightGuard<'_>, AccountPoisoned> {
        let state = lock_recovering_poison(&self.inner.value().linear);
        if let Some(error) = state.poisoned_error() {
            return Err(error);
        }
        Ok(AccountPreflightGuard {
            account: self,
            state,
        })
    }

    fn try_admit_publication_open(
        &self,
    ) -> Result<(PendingAccountActionAdmission<'_>, RetainedSetId), PublicationOpenAdmissionError>
    {
        let mut state = lock_recovering_poison(&self.inner.value().linear);
        if let Some(error) = state.poisoned_error() {
            return Err(PublicationOpenAdmissionError::Account(
                AccountActionAdmissionError::AccountPoisoned(error),
            ));
        }
        let admitted_account_actions = state.admitted_account_actions.checked_add(1).ok_or(
            PublicationOpenAdmissionError::Account(AccountActionAdmissionError::AccountingOverflow),
        )?;
        let set_id =
            try_next_retained_set_id().ok_or(PublicationOpenAdmissionError::MembershipExhausted)?;
        state.admitted_account_actions = admitted_account_actions;
        Ok((PendingAccountActionAdmission { account: self }, set_id))
    }

    fn try_publication_eligibility(&self) -> Result<PublicationEligibility, AccountPoisoned> {
        let state = lock_recovering_poison(&self.inner.value().linear);
        if let Some(error) = state.poisoned_error() {
            return Err(error);
        }
        let eligibility = PublicationEligibility { private: () };
        drop(state);
        #[cfg(test)]
        pause_after_publication_eligibility();
        Ok(eligibility)
    }

    fn try_admit_retained(
        &self,
        charge: AdmissionCharge,
        admitted_share_work: AdmittedRetainedShareWork,
    ) -> Result<PendingRetainedAdmission<'_>, RetainedAllocationError> {
        let mut state = lock_recovering_poison(&self.inner.value().linear);
        if let Some(error) = state.poisoned_error() {
            return Err(RetainedAllocationError::AccountPoisoned(error));
        }
        let admitted_retained_owners = state
            .admitted_retained_owners
            .checked_add(1)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        let limits = self.limits().counter_limits();
        try_admit_locked(&mut state.counters, limits, charge)
            .map_err(map_retained_admission_error)?;
        state.admitted_retained_owners = admitted_retained_owners;
        drop(state);
        let lease_owner = admitted_share_work.consume_account(self);
        Ok(PendingRetainedAdmission {
            account: self,
            charge,
            state: PendingRetainedAdmissionState::Pending,
            lease_owner: Some(lease_owner),
        })
    }

    /// Completes a retained admission whose account poison/action gate already linearized.
    ///
    /// The second account lock only commits the physical counter; it deliberately does not
    /// re-check the poison latch.  The single-use action capability is the proof that this retain
    /// began before any later poison transition.
    fn try_admit_retained_after_action(
        &self,
        action: PendingAccountActionAdmission<'_>,
        charge: AdmissionCharge,
        admitted_share_work: AdmittedRetainedShareWork,
    ) -> Result<PendingRetainedAdmission<'_>, RetainedAllocationError> {
        if !action.account().is_same_account(self) {
            let poisoned = self.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedAllocationError::AccountPoisoned(poisoned));
        }
        let mut state = lock_recovering_poison(&self.inner.value().linear);
        let admitted_retained_owners = state
            .admitted_retained_owners
            .checked_add(1)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        let limits = self.limits().counter_limits();
        try_admit_locked(&mut state.counters, limits, charge)
            .map_err(map_retained_admission_error)?;
        state.admitted_retained_owners = admitted_retained_owners;
        drop(state);
        let lease_owner = admitted_share_work.consume_account(self);
        action.complete();
        Ok(PendingRetainedAdmission {
            account: self,
            charge,
            state: PendingRetainedAdmissionState::Pending,
            lease_owner: Some(lease_owner),
        })
    }

    fn try_admit_start_retained_pair_after_action(
        &self,
        action: PendingAccountActionAdmission<'_>,
        plan: RetainedAdmissionPairPlan,
        owners: StartupAccountOwners,
    ) -> Result<(PendingStartRetainedAdmissions<'_>, StartupRemainingOwners), RetainedAllocationError>
    {
        if !action.account().is_same_account(self) {
            let poisoned = self.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedAllocationError::AccountPoisoned(poisoned));
        }
        let mut state = lock_recovering_poison(&self.inner.value().linear);
        let admitted_retained_owners = state
            .admitted_retained_owners
            .checked_add(2)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        let limits = self.limits().counter_limits();
        try_admit_locked(&mut state.counters, limits, plan.aggregate)
            .map_err(map_retained_admission_error)?;
        state.admitted_retained_owners = admitted_retained_owners;
        drop(state);
        let StartupAccountOwners {
            fallback_account,
            evidence_account,
            fallback_lease_owner,
            evidence_lease_owner,
        } = owners;
        action.complete();
        Ok((
            PendingStartRetainedAdmissions {
                account: self,
                plan,
                state: PendingStartRetainedAdmissionState::Pending,
                containment_owner: Some(fallback_account),
            },
            StartupRemainingOwners {
                evidence_account,
                fallback_lease_owner,
                evidence_lease_owner,
            },
        ))
    }

    fn release_retained_charge(
        &self,
        charge: AdmissionCharge,
    ) -> Result<(), AccountPoisonEventWitness> {
        let mut state = lock_recovering_poison(&self.inner.value().linear);
        if !release_locked(&mut state.counters, charge) {
            let reason = AccountPoisonReason::CounterInvariantViolated;
            return Err(AccountPoisonEventWitness::new(
                state.record_poison(reason),
                reason,
            ));
        }
        Ok(())
    }

    fn rollback_pending_retained_charge(
        &self,
        charge: AdmissionCharge,
        outer_reason: Option<AccountPoisonReason>,
    ) -> PendingRetainedRollback {
        let mut state = lock_recovering_poison(&self.inner.value().linear);
        if state.poisoned_error().is_some() {
            let poisoned = match outer_reason {
                Some(reason) => state.record_poison(reason),
                None => match state.poisoned_error() {
                    Some(poisoned) => poisoned,
                    None => unreachable_final_owner_state(),
                },
            };
            return PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned);
        }
        if !release_locked(&mut state.counters, charge) {
            return PendingRetainedRollback::ChargeReleaseUnverified(
                state.record_poison(AccountPoisonReason::CounterInvariantViolated),
            );
        }
        match outer_reason {
            Some(reason) => PendingRetainedRollback::FaultRecorded(state.record_poison(reason)),
            None => PendingRetainedRollback::Clean,
        }
    }

    #[cfg(test)]
    fn share_after_test_admission(&self) -> Result<Self, RetainedAllocationError> {
        let admission = self
            .try_admit_account_action()
            .map_err(|cause| match cause {
                AccountActionAdmissionError::AccountPoisoned(cause) => {
                    RetainedAllocationError::AccountPoisoned(cause)
                }
                AccountActionAdmissionError::AccountingOverflow => {
                    RetainedAllocationError::AccountingOverflow
                }
            })?;
        Ok(admission.run(|account| Self {
            inner: account.inner.share(),
        }))
    }

    #[cfg(test)]
    fn share_call_count(&self) -> usize {
        self.inner.share_call_count()
    }

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
    fn try_retain_with_admitted_handle<T>(
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

    fn try_retain_with_admitted_charge<T, P>(
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
    fn try_retain_with<T>(
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
    fn checked_retained_charge<T>(
        &self,
        plan: RetainedAllocationPlan,
    ) -> Result<AdmissionCharge, RetainedAllocationError> {
        self.checked_retained_charge_with_policy::<T, LeafRetainedPolicy>(plan)
    }

    fn checked_retained_table_charge<Block>(
        &self,
        plan: RetainedAllocationPlan,
    ) -> Result<AdmissionCharge, RetainedAllocationError> {
        self.checked_retained_charge_with_policy::<RetainedBlockTable<Block>, TableRetainedPolicy>(
            plan,
        )
    }

    fn checked_retained_charge_with_policy<T, P>(
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

fn observe_retained_payload_drop_failure(
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

fn observe_retained_table_pre_forget<Block>(
    payload: &RetainedPayload<RetainedBlockTable<Block>>,
    lease: &RetainedLease,
) -> AccountPoisonEventWitness {
    let poisoned = observe_retained_payload_drop_failure(lease);
    for member in &payload.value.members {
        let _ = member
            .allocation
            .account()
            .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
    }
    poisoned
}

impl<Block> RetainedValuePolicy<RetainedPayload<RetainedBlockTable<Block>>>
    for TableRetainedPolicy
{
    fn observe_pre_forget(
        payload: &RetainedPayload<RetainedBlockTable<Block>>,
        lease: &RetainedLease,
    ) -> AccountPoisonEventWitness {
        observe_retained_table_pre_forget(payload, lease)
    }

    fn release(
        payload: RetainedPayload<RetainedBlockTable<Block>>,
        mode: RetainedReleaseMode,
    ) -> RetainedValueReleaseOutcome {
        release_retained_table_payload(payload, mode)
    }
}

impl AdmissionOwner for PhysicalStorageAccount {
    fn release_admission(&self, charge: AdmissionCharge) {
        let _ = self.release_retained_charge(charge);
    }
}

impl fmt::Debug for PhysicalStorageAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhysicalStorageAccount")
            .field("limits", &self.limits())
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}

fn record_poison_on_forgotten_edges<const N: usize>(
    accounts: [Option<&PhysicalStorageAccount>; N],
    reason: AccountPoisonReason,
) {
    for account in accounts {
        let Some(account) = account else {
            continue;
        };
        let _ = account.record_poison(reason);
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

    pub(super) const fn code(self) -> u32 {
        self.0
    }
}

impl fmt::Display for RetainedBuildFailureCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

struct RetainedPayload<T> {
    value: T,
    charge: AdmissionCharge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RetainedSetId(u64);

static NEXT_RETAINED_SET_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
std::thread_local! {
    static RETAINED_SET_ID_REQUESTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
fn test_retained_set_id_requests() -> usize {
    RETAINED_SET_ID_REQUESTS.with(std::cell::Cell::get)
}

fn try_next_retained_set_id() -> Option<RetainedSetId> {
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
enum RetainedMembershipDestination {
    Canonical,
    OutcomeEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetainedMembership {
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
    ordinal: u64,
}

/// One module-private immutable payload whose physical charge follows its final shared owner.
struct PolicyRetainedAllocation<T, P>
where
    P: RetainedValuePolicy<RetainedPayload<T>>,
{
    inner: FinalOwnerAllocation<RetainedFinalOwnerInner<RetainedPayload<T>, P>>,
    membership: Option<RetainedMembership>,
}

type RetainedAllocation<T> = PolicyRetainedAllocation<T, LeafRetainedPolicy>;

type RetainedTableAllocation<Block> =
    PolicyRetainedAllocation<RetainedBlockTable<Block>, TableRetainedPolicy>;

impl<T, P> PolicyRetainedAllocation<T, P>
where
    P: RetainedValuePolicy<RetainedPayload<T>>,
{
    fn release_storage(self) -> FinalOwnerReleaseOutcome {
        let Self {
            inner,
            membership: _,
        } = self;
        inner.release()
    }

    fn try_release_unique_before_attribution(
        self,
    ) -> Result<FinalOwnerReleaseOutcome, (Self, AccountPoisonEventWitness)> {
        let Self { inner, membership } = self;
        match inner.try_release_unique_before_attribution() {
            Ok(outcome) => Ok(outcome),
            Err((inner, poisoned)) => Err((Self { inner, membership }, poisoned)),
        }
    }

    fn try_release_unique_before_attribution_deferred(
        self,
    ) -> Result<FinalOwnerReleaseOutcome, Self> {
        let Self { inner, membership } = self;
        match inner.try_release_unique_before_attribution_deferred() {
            Ok(outcome) => Ok(outcome),
            Err(inner) => Err(Self { inner, membership }),
        }
    }

    fn charged_bytes(&self) -> u64 {
        self.inner.value().charge.bytes
    }

    fn charged_allocations(&self) -> u64 {
        self.inner.value().charge.allocations
    }

    fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        self.inner.lease().owner.is_same_account(account)
    }

    fn account(&self) -> &PhysicalStorageAccount {
        &self.inner.lease().owner
    }

    fn assign_membership(&mut self, membership: RetainedMembership) -> Result<(), ()> {
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

    fn membership(&self) -> Option<RetainedMembership> {
        self.membership
    }

    /// Shares an already admitted retained allocation after the owning operation has charged all
    /// clone and retained-handle work. The new handle deliberately starts without publication
    /// membership so only the destination table can brand it.
    fn share_after_admitted_work(&self) -> Self {
        Self {
            inner: self.inner.share(),
            membership: None,
        }
    }

    #[cfg(test)]
    fn share_after_test_admission(&self) -> Self {
        Self {
            inner: self.inner.share(),
            membership: None,
        }
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        self.inner.ptr_eq(&other.inner)
    }

    #[cfg(test)]
    fn share_call_count(&self) -> usize {
        self.inner.share_call_count()
    }

    #[cfg(test)]
    fn allocation_identity(&self) -> *const RetainedFinalOwnerInner<RetainedPayload<T>, P> {
        self.inner.allocation_identity()
    }
}

impl<Block> PolicyRetainedAllocation<RetainedBlockTable<Block>, TableRetainedPolicy> {
    fn table_mut_during_construction(&mut self) -> Option<&mut RetainedBlockTable<Block>> {
        self.inner.table_mut_during_construction()
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
fn map_retained_admission_error(error: AdmissionError) -> RetainedAllocationError {
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
fn map_retained_final_owner_error(error: FinalOwnerAllocationError) -> RetainedAllocationError {
    match error {
        FinalOwnerAllocationError::AccountingOverflow => {
            RetainedAllocationError::AccountingOverflow
        }
        FinalOwnerAllocationError::AllocationFailed => RetainedAllocationError::AllocationFailed,
    }
}

/// Fixed capacities for one destination-typed retained writer or publication assembly.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedBatchLimits {
    attribution_slots: usize,
    commitment_slots: usize,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl RetainedBatchLimits {
    pub const fn new(attribution_slots: usize, commitment_slots: usize) -> Self {
        Self {
            attribution_slots,
            commitment_slots,
        }
    }

    pub const fn attribution_slots(self) -> usize {
        self.attribution_slots
    }

    pub const fn commitment_slots(self) -> usize {
        self.commitment_slots
    }
}

/// Ledger placement for one retained writer's fixed-capacity typed batch.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedWriterPlan<P, D> {
    phase: P,
    capacity_dimension: D,
    attribution_dimension: D,
    limits: RetainedBatchLimits,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<P: Copy, D: Copy> RetainedWriterPlan<P, D> {
    pub const fn new(
        phase: P,
        capacity_dimension: D,
        attribution_dimension: D,
        limits: RetainedBatchLimits,
    ) -> Self {
        Self {
            phase,
            capacity_dimension,
            attribution_dimension,
            limits,
        }
    }
}

/// Clone and retained-handle work charged before sharing an inherited owner.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedHandleSharePlan<D> {
    clone_dimension: D,
    retained_handle_dimension: D,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<D: Copy> RetainedHandleSharePlan<D> {
    pub const fn new(clone_dimension: D, retained_handle_dimension: D) -> Self {
        Self {
            clone_dimension,
            retained_handle_dimension,
        }
    }

    const fn charges(self) -> [(D, u64); 2] {
        [
            (self.clone_dimension, 1),
            (self.retained_handle_dimension, 1),
        ]
    }
}

/// Start-time bounds and fixed terminal commitments for one retained operation.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedOperationStartPlan<P, D> {
    terminal_writer: RetainedWriterPlan<P, D>,
    terminal_account_share_work: RetainedHandleSharePlan<D>,
    fallback_plan: RetainedAllocationPlan,
    fallback_commitment: OutputCommitment<D, OutcomeEvidenceDestination>,
    snapshot_commitment: OutputCommitment<D, OutcomeEvidenceDestination>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<P: Copy, D: Copy> RetainedOperationStartPlan<P, D> {
    pub(crate) const fn new(
        terminal_writer: RetainedWriterPlan<P, D>,
        terminal_account_share_work: RetainedHandleSharePlan<D>,
        fallback_plan: RetainedAllocationPlan,
        fallback_commitment: OutputCommitment<D, OutcomeEvidenceDestination>,
        snapshot_commitment: OutputCommitment<D, OutcomeEvidenceDestination>,
    ) -> Self {
        Self {
            terminal_writer,
            terminal_account_share_work,
            fallback_plan,
            fallback_commitment,
            snapshot_commitment,
        }
    }

    pub const fn for_publication(
        terminal_writer: RetainedWriterPlan<P, D>,
        terminal_account_share_work: RetainedHandleSharePlan<D>,
        fallback_plan: RetainedAllocationPlan,
        fallback_dimension: D,
        fallback_amount: u64,
        snapshot_dimension: D,
        snapshot_amount: u64,
    ) -> Self {
        Self::new(
            terminal_writer,
            terminal_account_share_work,
            fallback_plan,
            OutputCommitment::new(fallback_dimension, fallback_amount),
            OutputCommitment::new(snapshot_dimension, snapshot_amount),
        )
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug)]
#[non_exhaustive]
pub(crate) enum RetainedWriterError<D> {
    /// A fixed batch capacity could not be represented.
    BatchCapacity,
    /// Allocation of fixed batch storage failed.
    BatchAllocationFailed,
    /// The central operation ledger rejected the action.
    Ledger(LedgerCause<D>),
    /// The physical retained-storage account rejected the action.
    Physical(RetainedAllocationError),
    /// The shared physical account has been permanently quarantined.
    AccountPoisoned(AccountPoisoned),
    /// A preallocated attribution batch has no remaining slot.
    AttributionBatchFull,
    /// A preallocated commitment batch has no remaining slot.
    CommitmentBatchFull,
    /// Storage belongs to a different physical account.
    ForeignAccount,
    /// A globally unique retained set or member ordinal could not be minted.
    MembershipExhausted,
    /// The publication plan cannot fit its declared table and terminal capacities.
    InvalidPublicationPlan,
    /// Payload destruction could not prove that storage was released before attribution.
    StorageDropUnverified,
    /// The retained ownership graph could not be proven before attribution.
    OwnershipUnverified,
    /// A retained physical charge could not be proven released.
    ChargeReleaseUnverified,
}

impl<D: fmt::Debug> fmt::Display for RetainedWriterError<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BatchCapacity => formatter.write_str("retained batch capacity is invalid"),
            Self::BatchAllocationFailed => {
                formatter.write_str("retained batch storage allocation failed")
            }
            Self::Ledger(cause) => write!(formatter, "retained ledger action failed: {cause}"),
            Self::Physical(cause) => {
                write!(
                    formatter,
                    "retained physical storage action failed: {cause}"
                )
            }
            Self::AccountPoisoned(cause) => cause.fmt(formatter),
            Self::AttributionBatchFull => formatter.write_str("retained attribution batch is full"),
            Self::CommitmentBatchFull => formatter.write_str("retained commitment batch is full"),
            Self::ForeignAccount => {
                formatter.write_str("retained storage belongs to a foreign physical account")
            }
            Self::MembershipExhausted => {
                formatter.write_str("retained membership identity space is exhausted")
            }
            Self::InvalidPublicationPlan => {
                formatter.write_str("retained publication plan is inconsistent with its limits")
            }
            Self::StorageDropUnverified => formatter
                .write_str("retained storage release could not be verified before attribution"),
            Self::OwnershipUnverified => {
                formatter.write_str("retained ownership could not be verified before attribution")
            }
            Self::ChargeReleaseUnverified => {
                formatter.write_str("retained physical charge release could not be verified")
            }
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for RetainedWriterError<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Ledger(cause) => Some(cause),
            Self::Physical(cause) => Some(cause),
            Self::AccountPoisoned(cause) => Some(cause),
            _ => None,
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug)]
#[non_exhaustive]
pub(crate) enum RetainedOperationStartError<D> {
    /// Terminal writer limits cannot contain the mandatory fallback and evidence owners.
    InvalidTerminalLimits,
    /// A retained writer action failed during operation startup.
    Writer(RetainedWriterError<D>),
}

impl<D: fmt::Debug> fmt::Display for RetainedOperationStartError<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTerminalLimits => {
                formatter.write_str("retained terminal limits cannot contain mandatory owners")
            }
            Self::Writer(cause) => write!(formatter, "retained operation startup failed: {cause}"),
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for RetainedOperationStartError<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTerminalLimits => None,
            Self::Writer(cause) => Some(cause),
        }
    }
}

impl<D> From<RetainedWriterError<D>> for RetainedOperationStartError<D> {
    fn from(error: RetainedWriterError<D>) -> Self {
        Self::Writer(error)
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn map_batch_error<D>(error: AttributionBatchCreateError<D>) -> RetainedWriterError<D> {
    match error {
        AttributionBatchCreateError::CapacityOverflow => RetainedWriterError::BatchCapacity,
        AttributionBatchCreateError::Ledger(cause) => RetainedWriterError::Ledger(cause),
        AttributionBatchCreateError::AllocationFailed => RetainedWriterError::BatchAllocationFailed,
    }
}

fn map_pending_rollback_or(
    rollback: PendingRetainedRollback,
    original: RetainedAllocationError,
) -> RetainedAllocationError {
    match rollback {
        PendingRetainedRollback::Clean => original,
        PendingRetainedRollback::FaultRecorded(poisoned) => {
            RetainedAllocationError::AccountPoisoned(poisoned)
        }
        PendingRetainedRollback::ChargeReleaseUnverified(_) => {
            RetainedAllocationError::ChargeReleaseUnverified
        }
        PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
            RetainedAllocationError::AccountPoisonedWithRetainedCharge(poisoned)
        }
    }
}

fn map_faulted_pending_rollback(rollback: PendingRetainedRollback) -> RetainedAllocationError {
    match rollback {
        PendingRetainedRollback::FaultRecorded(poisoned) => {
            RetainedAllocationError::AccountPoisoned(poisoned)
        }
        PendingRetainedRollback::ChargeReleaseUnverified(_) => {
            RetainedAllocationError::ChargeReleaseUnverified
        }
        PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
            RetainedAllocationError::AccountPoisonedWithRetainedCharge(poisoned)
        }
        PendingRetainedRollback::Clean => unreachable_final_owner_state(),
    }
}

fn map_faulted_pending_writer_rollback<D>(
    rollback: PendingRetainedRollback,
) -> RetainedWriterError<D> {
    match rollback {
        PendingRetainedRollback::FaultRecorded(poisoned) => {
            RetainedWriterError::AccountPoisoned(poisoned)
        }
        PendingRetainedRollback::ChargeReleaseUnverified(_) => {
            RetainedWriterError::Physical(RetainedAllocationError::ChargeReleaseUnverified)
        }
        PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
            RetainedWriterError::Physical(
                RetainedAllocationError::AccountPoisonedWithRetainedCharge(poisoned),
            )
        }
        PendingRetainedRollback::Clean => unreachable_final_owner_state(),
    }
}

fn map_qualified_startup_batch_error<D>(
    error: AttributionBatchCreateError<D>,
    rollback: impl FnOnce(Option<AccountPoisonReason>) -> PendingRetainedRollback,
) -> RetainedWriterError<D> {
    match error {
        AttributionBatchCreateError::CapacityOverflow => match rollback(None) {
            PendingRetainedRollback::Clean => RetainedWriterError::BatchCapacity,
            PendingRetainedRollback::ChargeReleaseUnverified(_) => {
                RetainedWriterError::Physical(RetainedAllocationError::ChargeReleaseUnverified)
            }
            PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
                RetainedWriterError::Physical(
                    RetainedAllocationError::AccountPoisonedWithRetainedCharge(poisoned),
                )
            }
            PendingRetainedRollback::FaultRecorded(_) => unreachable_final_owner_state(),
        },
        AttributionBatchCreateError::AllocationFailed => match rollback(None) {
            PendingRetainedRollback::Clean => RetainedWriterError::BatchAllocationFailed,
            PendingRetainedRollback::ChargeReleaseUnverified(_) => {
                RetainedWriterError::Physical(RetainedAllocationError::ChargeReleaseUnverified)
            }
            PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
                RetainedWriterError::Physical(
                    RetainedAllocationError::AccountPoisonedWithRetainedCharge(poisoned),
                )
            }
            PendingRetainedRollback::FaultRecorded(_) => unreachable_final_owner_state(),
        },
        AttributionBatchCreateError::Ledger(_) => {
            match rollback(Some(AccountPoisonReason::OwnershipGraphUnverified)) {
                PendingRetainedRollback::FaultRecorded(poisoned) => {
                    RetainedWriterError::AccountPoisoned(poisoned)
                }
                PendingRetainedRollback::ChargeReleaseUnverified(_) => {
                    RetainedWriterError::Physical(RetainedAllocationError::ChargeReleaseUnverified)
                }
                PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
                    RetainedWriterError::Physical(
                        RetainedAllocationError::AccountPoisonedWithRetainedCharge(poisoned),
                    )
                }
                PendingRetainedRollback::Clean => unreachable_final_owner_state(),
            }
        }
    }
}

fn map_account_action_admission_error<D>(
    error: AccountActionAdmissionError,
) -> RetainedWriterError<D> {
    match error {
        AccountActionAdmissionError::AccountPoisoned(cause) => {
            RetainedWriterError::AccountPoisoned(cause)
        }
        AccountActionAdmissionError::AccountingOverflow => {
            RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow)
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn try_share_account_after_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    work: RetainedHandleSharePlan<D>,
) -> Result<PhysicalStorageAccount, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    let admission = account
        .try_admit_account_action()
        .map_err(map_account_action_admission_error)?;
    share_account_after_admitted_work(admission, ledger, work)
}

fn try_open_publication_account<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    work: RetainedHandleSharePlan<D>,
) -> Result<(PhysicalStorageAccount, RetainedSetId), RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    let (admission, set_id) =
        account
            .try_admit_publication_open()
            .map_err(|cause| match cause {
                PublicationOpenAdmissionError::Account(cause) => {
                    map_account_action_admission_error(cause)
                }
                PublicationOpenAdmissionError::MembershipExhausted => {
                    RetainedWriterError::MembershipExhausted
                }
            })?;
    let account = share_account_after_admitted_work(admission, ledger, work)?;
    Ok((account, set_id))
}

fn share_account_after_admitted_work<O, P, D>(
    admission: PendingAccountActionAdmission<'_>,
    ledger: &Ledger<O, P, D>,
    work: RetainedHandleSharePlan<D>,
) -> Result<PhysicalStorageAccount, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    let admitted = try_admit_retained_share_work(ledger, work)?;
    Ok(admission.run(|account| admitted.consume_account(account)))
}

fn try_admit_retained_share_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    work: RetainedHandleSharePlan<D>,
) -> Result<AdmittedRetainedShareWork, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    ledger
        .charge_observed_batch(&work.charges())
        .map(|()| AdmittedRetainedShareWork { private: () })
        .map_err(RetainedWriterError::Ledger)
}

fn try_admit_retained_share_work_before_physical<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    work: RetainedHandleSharePlan<D>,
) -> Result<AdmittedRetainedShareWork, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    match catch_unwind(AssertUnwindSafe(|| {
        try_admit_retained_share_work(ledger, work)
    })) {
        Ok(result) => result,
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            Err(RetainedWriterError::AccountPoisoned(poisoned))
        }
    }
}

fn try_admit_start_share_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    account_shares: StartupAccountShareEnvelope<D>,
) -> Result<StartupAccountOwners, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    let [
        fallback_account_step,
        evidence_account_step,
        fallback_lease_step,
        evidence_lease_step,
    ] = account_shares.into_steps();
    let fallback_account =
        try_share_startup_account_after_work(ledger, account, fallback_account_step)?;
    let evidence_account =
        try_share_startup_account_after_work(ledger, account, evidence_account_step)?;
    let fallback_lease_owner =
        try_share_startup_account_after_work(ledger, account, fallback_lease_step)?;
    let evidence_lease_owner =
        try_share_startup_account_after_work(ledger, account, evidence_lease_step)?;
    Ok(StartupAccountOwners {
        fallback_account,
        evidence_account,
        fallback_lease_owner,
        evidence_lease_owner,
    })
}

fn try_share_startup_account_after_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    step: StartupAccountShareStep<D>,
) -> Result<PhysicalStorageAccount, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    match catch_unwind(AssertUnwindSafe(|| {
        ledger.charge_observed_batch(&step.into_charges())
    })) {
        Ok(Ok(())) => Ok(PhysicalStorageAccount {
            inner: account.inner.share(),
        }),
        Ok(Err(cause)) => Err(RetainedWriterError::Ledger(cause)),
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            Err(RetainedWriterError::AccountPoisoned(poisoned))
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
struct PublicationDimensionCell<D> {
    dimension: D,
    attempted: AtomicU64,
    consumed: AtomicU64,
    peak_live_bytes: AtomicU64,
    committed_canonical_output: AtomicU64,
    published_outcome_evidence: AtomicU64,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
struct PublicationEvidenceCells<D> {
    dimensions: Vec<PublicationDimensionCell<D>>,
    frozen: AtomicBool,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<D: Copy + Ord> PublicationEvidenceCells<D> {
    fn freeze_from(&self, snapshot: &LedgerSnapshot<impl Copy + Ord, D>) {
        for (cell, (_, evidence)) in self.dimensions.iter().zip(snapshot.dimensions()) {
            cell.attempted
                .store(evidence.attempted(), Ordering::Release);
            cell.consumed.store(evidence.consumed(), Ordering::Release);
            cell.peak_live_bytes
                .store(evidence.peak_live_bytes(), Ordering::Release);
            cell.committed_canonical_output
                .store(evidence.committed_canonical_output(), Ordering::Release);
            cell.published_outcome_evidence
                .store(evidence.published_outcome_evidence(), Ordering::Release);
        }
        self.frozen.store(true, Ordering::Release);
    }

    fn dimension(&self, dimension: D) -> Option<PublishedDimensionSnapshot<D>> {
        if !self.frozen.load(Ordering::Acquire) {
            return None;
        }
        let position = self
            .dimensions
            .binary_search_by_key(&dimension, |cell| cell.dimension)
            .ok()?;
        let cell = &self.dimensions[position];
        Some(PublishedDimensionSnapshot {
            dimension,
            attempted: cell.attempted.load(Ordering::Acquire),
            consumed: cell.consumed.load(Ordering::Acquire),
            peak_live_bytes: cell.peak_live_bytes.load(Ordering::Acquire),
            committed_canonical_output: cell.committed_canonical_output.load(Ordering::Acquire),
            published_outcome_evidence: cell.published_outcome_evidence.load(Ordering::Acquire),
        })
    }
}

/// One copied destination-aware dimension snapshot from a published operation.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PublishedDimensionSnapshot<D> {
    dimension: D,
    attempted: u64,
    consumed: u64,
    peak_live_bytes: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<D: Copy> PublishedDimensionSnapshot<D> {
    pub const fn dimension(self) -> D {
        self.dimension
    }

    pub const fn attempted(self) -> u64 {
        self.attempted
    }

    pub const fn consumed(self) -> u64 {
        self.consumed
    }

    pub const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    pub const fn committed_canonical_output(self) -> u64 {
        self.committed_canonical_output
    }

    pub const fn published_outcome_evidence(self) -> u64 {
        self.published_outcome_evidence
    }
}

struct StartupRetainedPartPlan<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    attribution: StartupRetainedAttribution<P, D>,
    batch: AttributionBatchPlan<O, P, D, Destination>,
    allocation: RetainedAllocationPlan,
    commitment: OutputCommitment<D, Destination>,
}

impl<O, P, D, Destination> StartupRetainedPartPlan<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    const fn new(
        attribution: StartupRetainedAttribution<P, D>,
        batch: AttributionBatchPlan<O, P, D, Destination>,
        allocation: RetainedAllocationPlan,
        commitment: OutputCommitment<D, Destination>,
    ) -> Self {
        Self {
            attribution,
            batch,
            allocation,
            commitment,
        }
    }
}

struct StartupRetainedSideFailure<D> {
    cause: RetainedWriterError<D>,
    fault: Option<AccountPoisonReason>,
    retain_attribution: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupRetainedSide {
    Fallback,
    Evidence,
}

struct StartupPendingResolution {
    rollback: PendingRetainedRollback,
}

impl<D> StartupRetainedSideFailure<D> {
    const fn clean(cause: RetainedWriterError<D>) -> Self {
        Self {
            cause,
            fault: None,
            retain_attribution: false,
        }
    }

    const fn ownership(cause: RetainedWriterError<D>) -> Self {
        Self {
            cause,
            fault: Some(AccountPoisonReason::OwnershipGraphUnverified),
            retain_attribution: false,
        }
    }

    const fn uncertain(cause: RetainedWriterError<D>) -> Self {
        Self {
            cause,
            fault: Some(AccountPoisonReason::OwnershipGraphUnverified),
            retain_attribution: true,
        }
    }
}

struct StartupRetainedSideConstruction<'account, O, P, D, T>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    unarmed: Option<UnarmedRetainedFinalOwner<RetainedPayload<T>, LeafRetainedPolicy>>,
    attribution: Option<OperationAttributionReservation<O, P, D, OutcomeEvidenceDestination>>,
    batch: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    admission: Option<PendingRetainedAdmission<'account>>,
    account: Option<PhysicalStorageAccount>,
}

struct ReadyStartupRetainedSide<'account, O, P, D, T>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    unarmed: UnarmedRetainedFinalOwner<RetainedPayload<T>, LeafRetainedPolicy>,
    admission: PendingRetainedAdmission<'account>,
    batch: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    account: PhysicalStorageAccount,
}

struct ContainedStartupRetainedSide<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    lease: RetainedLeaseGuard,
    batch: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    attribution: Option<OperationAttributionReservation<O, P, D, OutcomeEvidenceDestination>>,
    account: Option<PhysicalStorageAccount>,
}

/// A fail-closed startup pair keeps both attribution batches beside the aggregate physical lease.
///
/// The pair is intentionally opaque: callers cannot drop one charge witness while retaining the
/// corresponding typed batch in a separate orchestration variable.
struct ContainedStartupRetainedPair<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    lease: RetainedLeaseGuard,
    fallback_batch: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    evidence_batch: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    fallback_attribution:
        Option<OperationAttributionReservation<O, P, D, OutcomeEvidenceDestination>>,
    evidence_attribution:
        Option<OperationAttributionReservation<O, P, D, OutcomeEvidenceDestination>>,
    fallback_account: Option<PhysicalStorageAccount>,
    evidence_account: Option<PhysicalStorageAccount>,
    fallback_lease_owner: Option<PhysicalStorageAccount>,
    evidence_lease_owner: Option<PhysicalStorageAccount>,
}

struct StartupSideContainmentComponents<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    batch: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    attribution: Option<OperationAttributionReservation<O, P, D, OutcomeEvidenceDestination>>,
    account: Option<PhysicalStorageAccount>,
    lease_owner: Option<PhysicalStorageAccount>,
}

impl<O, P, D> Drop for ContainedStartupRetainedPair<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(batch) = self.fallback_batch.take() {
            std::mem::forget(batch);
        }
        if let Some(batch) = self.evidence_batch.take() {
            std::mem::forget(batch);
        }
        if let Some(attribution) = self.fallback_attribution.take() {
            std::mem::forget(attribution);
        }
        if let Some(attribution) = self.evidence_attribution.take() {
            std::mem::forget(attribution);
        }
        drop(self.fallback_account.take());
        drop(self.evidence_account.take());
        drop(self.fallback_lease_owner.take());
        drop(self.evidence_lease_owner.take());
        let _ = &self.lease;
    }
}

impl<O, P, D> Drop for ContainedStartupRetainedSide<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(batch) = self.batch.take() {
            std::mem::forget(batch);
        }
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
        drop(self.account.take());
        let _ = &self.lease;
    }
}

impl<'account, O, P, D, T> ReadyStartupRetainedSide<'account, O, P, D, T>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn commit(self, value: T) -> RetainedPart<T, OutcomeEvidenceDestination, O, P, D> {
        let Self {
            unarmed,
            mut admission,
            batch,
            account,
        } = self;
        let charge = admission.charge;
        let lease = admission.commit_lease();
        let allocation = PolicyRetainedAllocation {
            inner: unarmed.arm(RetainedPayload { value, charge }, lease),
            membership: None,
        };
        RetainedPart::from_complete(allocation, batch, account)
    }

    fn contain_after_builder_panic(self) {
        let Self {
            unarmed,
            mut admission,
            batch,
            account,
        } = self;
        drop(unarmed);
        let lease = admission.contain_with_lease_owner();
        drop(ContainedStartupRetainedSide {
            lease,
            batch: Some(batch),
            attribution: None,
            account: Some(account),
        });
    }

    fn rollback(self, outer_reason: Option<AccountPoisonReason>) -> PendingRetainedRollback {
        let Self {
            unarmed,
            mut admission,
            batch,
            account,
        } = self;
        drop(unarmed);
        if !account.is_same_account(admission.account) {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            let lease = admission.contain_with_lease_owner();
            drop(ContainedStartupRetainedSide {
                lease,
                batch: Some(batch),
                attribution: None,
                account: Some(account),
            });
            return PendingRetainedRollback::FaultRecorded(poisoned);
        }
        let rollback = admission.rollback_in_place(outer_reason);
        if matches!(
            rollback,
            PendingRetainedRollback::ChargeReleaseUnverified(_)
                | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
        ) {
            let lease = match admission.take_contained_lease() {
                Some(lease) => lease,
                None => unreachable_final_owner_state(),
            };
            drop(ContainedStartupRetainedSide {
                lease,
                batch: Some(batch),
                attribution: None,
                account: Some(account),
            });
        } else {
            drop(batch);
            drop(account);
        }
        drop(admission);
        rollback
    }
}

enum StartupRetainedSideState<'account, O, P, D, T>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Empty,
    Pending {
        account: PhysicalStorageAccount,
        admission: PendingRetainedAdmission<'account>,
    },
    Constructing(StartupRetainedSideConstruction<'account, O, P, D, T>),
    Ready(ReadyStartupRetainedSide<'account, O, P, D, T>),
    Built(RetainedPart<T, OutcomeEvidenceDestination, O, P, D>),
    Contained(ContainedStartupRetainedSide<O, P, D>),
}

impl<'account, O, P, D, T> StartupRetainedSideState<'account, O, P, D, T>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    T: Send + Sync + 'static,
{
    fn begin_construction(&mut self) {
        let pending = std::mem::replace(self, Self::Empty);
        let (account, admission) = match pending {
            Self::Pending { account, admission } => (account, admission),
            state => {
                *self = state;
                unreachable_final_owner_state();
            }
        };
        *self = Self::Constructing(StartupRetainedSideConstruction {
            unarmed: None,
            attribution: None,
            batch: None,
            admission: Some(admission),
            account: Some(account),
        });
    }

    fn construction_mut(&mut self) -> &mut StartupRetainedSideConstruction<'account, O, P, D, T> {
        match self {
            Self::Constructing(construction) => construction,
            _ => unreachable_final_owner_state(),
        }
    }

    fn try_prepare(
        &mut self,
        ledger: &Ledger<O, P, D>,
        plan: StartupRetainedPartPlan<O, P, D, OutcomeEvidenceDestination>,
    ) -> Result<(), StartupRetainedSideFailure<D>> {
        self.begin_construction();
        let construction = self.construction_mut();
        let admission = match construction.admission.as_ref() {
            Some(admission) => admission,
            None => unreachable_final_owner_state(),
        };
        let account = match construction.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let expected_charge = account
            .checked_retained_charge::<T>(plan.allocation)
            .map_err(|cause| {
                StartupRetainedSideFailure::clean(RetainedWriterError::Physical(cause))
            })?;
        let (phase, attribution_dimension, qualified_bytes) = plan.attribution.into_parts();
        if expected_charge != admission.charge
            || qualified_bytes != admission.charge.bytes
            || !account.is_same_account(admission.account)
        {
            return Err(StartupRetainedSideFailure::ownership(
                RetainedWriterError::InvalidPublicationPlan,
            ));
        }

        let batch = match catch_unwind(AssertUnwindSafe(|| {
            ledger.try_attribution_batch_from_plan(plan.batch)
        })) {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return Err(StartupRetainedSideFailure::uncertain(
                    RetainedWriterError::OwnershipUnverified,
                ));
            }
            Ok(Err(AttributionBatchCreateError::CapacityOverflow)) => {
                return Err(StartupRetainedSideFailure::clean(
                    RetainedWriterError::BatchCapacity,
                ));
            }
            Ok(Err(AttributionBatchCreateError::AllocationFailed)) => {
                return Err(StartupRetainedSideFailure::clean(
                    RetainedWriterError::BatchAllocationFailed,
                ));
            }
            Ok(Err(AttributionBatchCreateError::Ledger(_))) => {
                return Err(StartupRetainedSideFailure::uncertain(
                    RetainedWriterError::OwnershipUnverified,
                ));
            }
            Ok(Ok(batch)) => batch,
        };
        construction.batch = Some(batch);
        let batch = match construction.batch.as_mut() {
            Some(batch) => batch,
            None => unreachable_final_owner_state(),
        };
        if !batch.has_attribution_slot() || batch.try_push_commitment(plan.commitment).is_err() {
            return Err(StartupRetainedSideFailure::uncertain(
                RetainedWriterError::OwnershipUnverified,
            ));
        }

        let attribution = match catch_unwind(AssertUnwindSafe(|| {
            ledger.reserve_operation_attribution::<OutcomeEvidenceDestination>(
                phase,
                attribution_dimension,
                qualified_bytes,
            )
        })) {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return Err(StartupRetainedSideFailure::uncertain(
                    RetainedWriterError::OwnershipUnverified,
                ));
            }
            Ok(Err(_)) => {
                return Err(StartupRetainedSideFailure::uncertain(
                    RetainedWriterError::OwnershipUnverified,
                ));
            }
            Ok(Ok(attribution)) => attribution,
        };
        construction.attribution = Some(attribution);
        let attribution = match construction.attribution.take() {
            Some(attribution) => attribution,
            None => unreachable_final_owner_state(),
        };
        if let Err(attribution) = batch.try_push_attribution(attribution) {
            construction.attribution = Some(attribution);
            return Err(StartupRetainedSideFailure::uncertain(
                RetainedWriterError::OwnershipUnverified,
            ));
        }

        let unarmed =
            UnarmedRetainedFinalOwner::<RetainedPayload<T>, LeafRetainedPolicy>::try_allocate()
                .map_err(|cause| {
                    StartupRetainedSideFailure::clean(RetainedWriterError::Physical(
                        map_retained_final_owner_error(cause),
                    ))
                })?;
        construction.unarmed = Some(unarmed);
        let construction = match std::mem::replace(self, Self::Empty) {
            Self::Constructing(construction) => construction,
            state => {
                *self = state;
                unreachable_final_owner_state();
            }
        };
        let StartupRetainedSideConstruction {
            account,
            admission,
            batch,
            attribution,
            unarmed,
        } = construction;
        if attribution.is_some() {
            unreachable_final_owner_state();
        }
        *self = Self::Ready(ReadyStartupRetainedSide {
            unarmed: match unarmed {
                Some(unarmed) => unarmed,
                None => unreachable_final_owner_state(),
            },
            admission: match admission {
                Some(admission) => admission,
                None => unreachable_final_owner_state(),
            },
            batch: match batch {
                Some(batch) => batch,
                None => unreachable_final_owner_state(),
            },
            account: match account {
                Some(account) => account,
                None => unreachable_final_owner_state(),
            },
        });
        Ok(())
    }

    fn drop_unarmed_before_charge_resolution(&mut self) {
        match self {
            Self::Constructing(construction) => {
                drop(construction.unarmed.take());
            }
            Self::Ready(_) => {
                let ready = match std::mem::replace(self, Self::Empty) {
                    Self::Ready(ready) => ready,
                    state => {
                        *self = state;
                        return;
                    }
                };
                let ReadyStartupRetainedSide {
                    account,
                    admission,
                    batch,
                    unarmed,
                } = ready;
                drop(unarmed);
                *self = Self::Constructing(StartupRetainedSideConstruction {
                    account: Some(account),
                    admission: Some(admission),
                    batch: Some(batch),
                    attribution: None,
                    unarmed: None,
                });
            }
            Self::Empty | Self::Pending { .. } | Self::Built(_) | Self::Contained(_) => {}
        }
    }

    fn contain_ready(&mut self) {
        let ready = match std::mem::replace(self, Self::Empty) {
            Self::Ready(ready) => ready,
            state => {
                *self = state;
                unreachable_final_owner_state();
            }
        };
        let ReadyStartupRetainedSide {
            account,
            mut admission,
            batch,
            unarmed,
        } = ready;
        drop(unarmed);
        let lease = admission.contain_with_lease_owner();
        *self = Self::Contained(ContainedStartupRetainedSide {
            lease,
            batch: Some(batch),
            attribution: None,
            account: Some(account),
        });
    }

    fn pending_admission_mut(&mut self) -> Option<&mut PendingRetainedAdmission<'account>> {
        match self {
            Self::Pending { admission, .. } => Some(admission),
            Self::Constructing(construction) => construction.admission.as_mut(),
            Self::Ready(ready) => Some(&mut ready.admission),
            Self::Empty | Self::Built(_) | Self::Contained(_) => None,
        }
    }

    fn take_ready(&mut self) -> Option<ReadyStartupRetainedSide<'account, O, P, D, T>> {
        match std::mem::replace(self, Self::Empty) {
            Self::Ready(ready) => Some(ready),
            state => {
                *self = state;
                None
            }
        }
    }

    fn pending_admission(&self) -> Option<&PendingRetainedAdmission<'account>> {
        match self {
            Self::Pending { admission, .. } => Some(admission),
            Self::Constructing(construction) => construction.admission.as_ref(),
            Self::Ready(ready) => Some(&ready.admission),
            Self::Empty | Self::Built(_) | Self::Contained(_) => None,
        }
        .filter(|admission| admission.is_pending())
    }

    fn rollback_pending_side(
        &mut self,
        outer_reason: Option<AccountPoisonReason>,
    ) -> PendingRetainedRollback {
        let charge = match self.pending_admission() {
            Some(admission) => admission.charge,
            None => {
                let account = match self {
                    Self::Pending { account, .. } => Some(&*account),
                    Self::Constructing(construction) => construction.account.as_ref(),
                    Self::Ready(ready) => Some(&ready.account),
                    Self::Empty | Self::Built(_) | Self::Contained(_) => None,
                };
                return match account {
                    Some(account) => PendingRetainedRollback::FaultRecorded(
                        account.record_poison(AccountPoisonReason::OwnershipGraphUnverified),
                    ),
                    None => PendingRetainedRollback::Clean,
                };
            }
        };
        let rollback = match self {
            Self::Pending { admission, .. } => admission.rollback_startup_in_place(outer_reason),
            Self::Constructing(construction) => match construction.admission.as_mut() {
                Some(admission) => admission.rollback_startup_in_place(outer_reason),
                None => PendingRetainedRollback::Clean,
            },
            Self::Ready(ready) => ready.admission.rollback_startup_in_place(outer_reason),
            Self::Empty | Self::Built(_) | Self::Contained(_) => PendingRetainedRollback::Clean,
        };
        if !matches!(
            rollback,
            PendingRetainedRollback::ChargeReleaseUnverified(_)
        ) {
            return rollback;
        }
        let _ = charge;
        rollback
    }

    fn contain_pending_charge(&mut self, charge: AdmissionCharge) {
        let mut components = self.take_containment_components();
        let owner = components
            .account
            .take()
            .or_else(|| components.lease_owner.take());
        let Some(owner) = owner else {
            if let Some(batch) = components.batch.take() {
                std::mem::forget(batch);
            }
            if let Some(attribution) = components.attribution.take() {
                std::mem::forget(attribution);
            }
            return;
        };
        *self = Self::Contained(ContainedStartupRetainedSide {
            lease: RetainedLeaseGuard {
                lease: Some(AdmissionLease::new(owner, charge)),
            },
            batch: components.batch,
            attribution: components.attribution,
            account: components.account.or(components.lease_owner),
        });
    }

    fn take_built(&mut self) -> Option<RetainedPart<T, OutcomeEvidenceDestination, O, P, D>> {
        match std::mem::replace(self, Self::Empty) {
            Self::Built(part) => Some(part),
            state => {
                *self = state;
                None
            }
        }
    }

    const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    fn contain_built_after_recorded_fault(&mut self) {
        if let Some(mut part) = self.take_built() {
            part.contain_after_recorded_poison();
        }
    }

    fn try_discard_built_before_batch_release(
        &mut self,
        defer_ownership_fault: bool,
    ) -> Result<(), RetainedPartCleanupError> {
        let mut part = match self.take_built() {
            Some(part) => part,
            None => return Err(RetainedPartCleanupError::MissingFaultAnchor),
        };
        part.try_discard_before_batch_release(defer_ownership_fault)
    }

    fn contain_attribution_after_anchored_fault(&mut self) {
        let mut components = self.take_containment_components();
        *self = Self::Contained(ContainedStartupRetainedSide {
            lease: RetainedLeaseGuard::unarmed(),
            batch: components.batch.take(),
            attribution: components.attribution.take(),
            account: components
                .account
                .take()
                .or_else(|| components.lease_owner.take()),
        });
    }

    fn take_containment_components(&mut self) -> StartupSideContainmentComponents<O, P, D> {
        let state = std::mem::replace(self, Self::Empty);
        match state {
            Self::Pending {
                account,
                mut admission,
            } => {
                let lease_owner = admission.transfer_to_aggregate_containment();
                StartupSideContainmentComponents {
                    batch: None,
                    attribution: None,
                    account: Some(account),
                    lease_owner: Some(lease_owner),
                }
            }
            Self::Constructing(mut construction) => {
                drop(construction.unarmed.take());
                let lease_owner = construction
                    .admission
                    .as_mut()
                    .map(|admission| admission.transfer_to_aggregate_containment());
                StartupSideContainmentComponents {
                    batch: construction.batch.take(),
                    attribution: construction.attribution.take(),
                    account: construction.account.take(),
                    lease_owner,
                }
            }
            Self::Ready(ready) => {
                let ReadyStartupRetainedSide {
                    unarmed,
                    mut admission,
                    batch,
                    account,
                } = ready;
                drop(unarmed);
                let lease_owner = admission.transfer_to_aggregate_containment();
                StartupSideContainmentComponents {
                    batch: Some(batch),
                    attribution: None,
                    account: Some(account),
                    lease_owner: Some(lease_owner),
                }
            }
            Self::Built(mut part) => {
                part.contain_after_recorded_poison();
                StartupSideContainmentComponents {
                    batch: None,
                    attribution: None,
                    account: None,
                    lease_owner: None,
                }
            }
            Self::Contained(contained) => {
                drop(contained);
                StartupSideContainmentComponents {
                    batch: None,
                    attribution: None,
                    account: None,
                    lease_owner: None,
                }
            }
            Self::Empty => StartupSideContainmentComponents {
                batch: None,
                attribution: None,
                account: None,
                lease_owner: None,
            },
        }
    }
}

struct StartupReadyPair<'account, O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    account: &'account PhysicalStorageAccount,
    pair_plan: RetainedAdmissionPairPlan,
    fallback: ReadyStartupRetainedSide<'account, O, P, D, F>,
    evidence: ReadyStartupRetainedSide<'account, O, P, D, PublicationEvidenceCells<D>>,
    canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
}

struct StartupFallbackBuilt<'account, O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    fallback: RetainedPart<F, OutcomeEvidenceDestination, O, P, D>,
    evidence: ReadyStartupRetainedSide<'account, O, P, D, PublicationEvidenceCells<D>>,
    canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    account: &'account PhysicalStorageAccount,
    pair_plan: RetainedAdmissionPairPlan,
}

#[derive(Clone, Copy)]
enum StartupCleanupDisposition {
    Armed,
    FaultAnchored(AccountPoisoned),
    Drained,
}

impl<'account, O, P, D, F> StartupReadyPair<'account, O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    fn into_cleanup_guard(self) -> StartupRetainedPartGuard<'account, O, P, D, F> {
        StartupRetainedPartGuard {
            account: self.account,
            pair_plan: self.pair_plan,
            pending_pair: None,
            evidence_account: None,
            fallback_lease_owner: None,
            evidence_lease_owner: None,
            fallback: StartupRetainedSideState::Ready(self.fallback),
            evidence: StartupRetainedSideState::Ready(self.evidence),
            contained_pending: None,
            canonical_assembly: Some(self.canonical_assembly),
            evidence_assembly: Some(self.evidence_assembly),
            cleanup_disposition: StartupCleanupDisposition::Armed,
        }
    }

    fn try_build(
        self,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
        build_evidence: impl FnOnce() -> Result<PublicationEvidenceCells<D>, RetainedBuildFailureCode>,
    ) -> Result<StartupFallbackOwner<O, P, D, F>, RetainedWriterError<D>> {
        let fallback = match catch_unwind(AssertUnwindSafe(build_fallback)) {
            Ok(Ok(value)) => value,
            Ok(Err(cause)) => {
                let cleanup = self.into_cleanup_guard();
                return Err(cleanup.finish_rejection(
                    StartupRetainedSide::Fallback,
                    StartupRetainedSideFailure::clean(RetainedWriterError::Physical(
                        RetainedAllocationError::Build(cause),
                    )),
                ));
            }
            Err(payload) => {
                return Err(self
                    .into_cleanup_guard()
                    .finish_fallback_builder_panic(payload));
            }
        };
        let Self {
            account,
            pair_plan,
            fallback: ready_fallback,
            evidence,
            canonical_assembly,
            evidence_assembly,
        } = self;
        StartupFallbackBuilt {
            fallback: ready_fallback.commit(fallback),
            evidence,
            canonical_assembly,
            evidence_assembly,
            account,
            pair_plan,
        }
        .try_build_evidence(build_evidence)
    }
}

impl<'account, O, P, D, F> StartupFallbackBuilt<'account, O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    fn into_cleanup_guard(self) -> StartupRetainedPartGuard<'account, O, P, D, F> {
        StartupRetainedPartGuard {
            account: self.account,
            pair_plan: self.pair_plan,
            pending_pair: None,
            evidence_account: None,
            fallback_lease_owner: None,
            evidence_lease_owner: None,
            fallback: StartupRetainedSideState::Built(self.fallback),
            evidence: StartupRetainedSideState::Ready(self.evidence),
            contained_pending: None,
            canonical_assembly: Some(self.canonical_assembly),
            evidence_assembly: Some(self.evidence_assembly),
            cleanup_disposition: StartupCleanupDisposition::Armed,
        }
    }

    fn try_build_evidence(
        self,
        build_evidence: impl FnOnce() -> Result<PublicationEvidenceCells<D>, RetainedBuildFailureCode>,
    ) -> Result<StartupFallbackOwner<O, P, D, F>, RetainedWriterError<D>> {
        let evidence = match catch_unwind(AssertUnwindSafe(build_evidence)) {
            Ok(Ok(value)) => value,
            Ok(Err(cause)) => {
                return Err(self
                    .into_cleanup_guard()
                    .finish_evidence_build_rejection(cause));
            }
            Err(payload) => {
                return Err(self
                    .into_cleanup_guard()
                    .finish_evidence_builder_panic(payload));
            }
        };
        let Self {
            fallback,
            evidence: ready_evidence,
            canonical_assembly,
            evidence_assembly,
            account: _,
            pair_plan: _,
        } = self;
        Ok(StartupFallbackOwner::from_complete(
            fallback,
            ready_evidence.commit(evidence),
            canonical_assembly,
            evidence_assembly,
        ))
    }
}

struct StartupRetainedPartGuard<'account, O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    account: &'account PhysicalStorageAccount,
    pair_plan: RetainedAdmissionPairPlan,
    pending_pair: Option<PendingStartRetainedAdmissions<'account>>,
    evidence_account: Option<PhysicalStorageAccount>,
    fallback_lease_owner: Option<PhysicalStorageAccount>,
    evidence_lease_owner: Option<PhysicalStorageAccount>,
    fallback: StartupRetainedSideState<'account, O, P, D, F>,
    evidence: StartupRetainedSideState<'account, O, P, D, PublicationEvidenceCells<D>>,
    contained_pending: Option<ContainedStartupRetainedPair<O, P, D>>,
    canonical_assembly: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_assembly: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    cleanup_disposition: StartupCleanupDisposition,
}

impl<'account, O, P, D, F> StartupRetainedPartGuard<'account, O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    fn anchor_fault(&mut self, poisoned: AccountPoisoned) {
        self.cleanup_disposition = StartupCleanupDisposition::FaultAnchored(poisoned);
    }

    fn mark_drained(&mut self) {
        self.cleanup_disposition = StartupCleanupDisposition::Drained;
    }

    fn contain_pending_after_anchored_fault(&mut self) {
        if let Some(admissions) = self.pending_pair.take() {
            self.contained_pending = Some(ContainedStartupRetainedPair {
                lease: admissions.transfer_to_containment(),
                fallback_batch: None,
                evidence_batch: None,
                fallback_attribution: None,
                evidence_attribution: None,
                fallback_account: None,
                evidence_account: self.evidence_account.take(),
                fallback_lease_owner: self.fallback_lease_owner.take(),
                evidence_lease_owner: self.evidence_lease_owner.take(),
            });
            return;
        }

        let fallback_charge = self
            .fallback
            .pending_admission()
            .map(|admission| admission.charge);
        let evidence_charge = self
            .evidence
            .pending_admission()
            .map(|admission| admission.charge);
        match (fallback_charge, evidence_charge) {
            (Some(_), Some(_)) => self.install_split_pending_containment(None),
            (Some(charge), None) => self.fallback.contain_pending_charge(charge),
            (None, Some(charge)) => self.evidence.contain_pending_charge(charge),
            (None, None) => {}
        }
    }

    fn new(
        mut admission_guard: StartupAdmissionGuard<'account>,
        canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
        evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    ) -> Self {
        let evidence_account = match admission_guard.evidence_account.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let fallback_lease_owner = match admission_guard.fallback_lease_owner.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let evidence_lease_owner = match admission_guard.evidence_lease_owner.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let pending_pair = match admission_guard.admissions.take() {
            Some(admissions) => admissions,
            None => unreachable_final_owner_state(),
        };
        let pair_plan = pending_pair.plan;
        Self {
            account: admission_guard.account,
            pair_plan,
            pending_pair: Some(pending_pair),
            evidence_account: Some(evidence_account),
            fallback_lease_owner: Some(fallback_lease_owner),
            evidence_lease_owner: Some(evidence_lease_owner),
            fallback: StartupRetainedSideState::Empty,
            evidence: StartupRetainedSideState::Empty,
            contained_pending: None,
            canonical_assembly: Some(canonical_assembly),
            evidence_assembly: Some(evidence_assembly),
            cleanup_disposition: StartupCleanupDisposition::Armed,
        }
    }

    fn split_pending_pair(&mut self) {
        let admissions = match self.pending_pair.take() {
            Some(admissions) => admissions,
            None => return,
        };
        let evidence_account = match self.evidence_account.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let fallback_lease_owner = match self.fallback_lease_owner.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let evidence_lease_owner = match self.evidence_lease_owner.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let (fallback_account, fallback, evidence_account, evidence) =
            admissions.into_parts(evidence_account, fallback_lease_owner, evidence_lease_owner);
        self.fallback = StartupRetainedSideState::Pending {
            account: fallback_account,
            admission: fallback,
        };
        self.evidence = StartupRetainedSideState::Pending {
            account: evidence_account,
            admission: evidence,
        };
    }

    fn mark_split_pending_released(&mut self) {
        if let Some(admission) = self.fallback.pending_admission_mut() {
            admission.mark_released_by_aggregate_rollback();
        }
        if let Some(admission) = self.evidence.pending_admission_mut() {
            admission.mark_released_by_aggregate_rollback();
        }
    }

    fn install_split_pending_containment(&mut self, owner: Option<PhysicalStorageAccount>) {
        let mut fallback = self.fallback.take_containment_components();
        let mut evidence = self.evidence.take_containment_components();
        let owner = owner
            .or_else(|| fallback.account.take())
            .or_else(|| evidence.account.take())
            .or_else(|| fallback.lease_owner.take())
            .or_else(|| evidence.lease_owner.take());
        self.contained_pending = Some(ContainedStartupRetainedPair {
            lease: RetainedLeaseGuard {
                lease: owner.map(|owner| AdmissionLease::new(owner, self.pair_plan.aggregate)),
            },
            fallback_batch: fallback.batch,
            evidence_batch: evidence.batch,
            fallback_attribution: fallback.attribution,
            evidence_attribution: evidence.attribution,
            fallback_account: fallback.account,
            evidence_account: evidence.account,
            fallback_lease_owner: fallback.lease_owner,
            evidence_lease_owner: evidence.lease_owner,
        });
    }

    fn rollback_remaining(
        &mut self,
        outer_reason: Option<AccountPoisonReason>,
    ) -> StartupPendingResolution {
        if let Some(admissions) = self.pending_pair.take() {
            let rollback = admissions.rollback_with_fault(outer_reason);
            if let PendingRetainedRollback::FaultRecorded(poisoned)
            | PendingRetainedRollback::ChargeReleaseUnverified(poisoned)
            | PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) = rollback
            {
                self.anchor_fault(poisoned);
            }
            return StartupPendingResolution { rollback };
        }

        let fallback_pending = self.fallback.pending_admission().map(|admission| {
            (
                admission.account.is_same_account(self.account),
                admission.charge,
            )
        });
        let evidence_pending = self.evidence.pending_admission().map(|admission| {
            (
                admission.account.is_same_account(self.account),
                admission.charge,
            )
        });
        match (fallback_pending, evidence_pending) {
            (
                Some((fallback_same_account, fallback_charge)),
                Some((evidence_same_account, evidence_charge)),
            ) => {
                let exact_pair = fallback_same_account
                    && evidence_same_account
                    && fallback_charge == self.pair_plan.first
                    && evidence_charge == self.pair_plan.second;
                if !exact_pair {
                    let poisoned = self
                        .account
                        .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                    self.anchor_fault(poisoned);
                    self.install_split_pending_containment(None);
                    return StartupPendingResolution {
                        rollback: PendingRetainedRollback::FaultRecorded(poisoned),
                    };
                }
                let rollback = self
                    .account
                    .rollback_pending_retained_charge(self.pair_plan.aggregate, outer_reason);
                if let PendingRetainedRollback::FaultRecorded(poisoned)
                | PendingRetainedRollback::ChargeReleaseUnverified(poisoned)
                | PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) = rollback
                {
                    self.anchor_fault(poisoned);
                }
                match rollback {
                    PendingRetainedRollback::ChargeReleaseUnverified(_)
                    | PendingRetainedRollback::ChargeRetainedAfterPoison(_) => {
                        self.install_split_pending_containment(None);
                    }
                    PendingRetainedRollback::Clean | PendingRetainedRollback::FaultRecorded(_) => {
                        self.mark_split_pending_released();
                    }
                }
                StartupPendingResolution { rollback }
            }
            (Some((fallback_same_account, fallback_charge)), None) => {
                let exact = fallback_same_account && fallback_charge == self.pair_plan.first;
                if !exact {
                    let poisoned = self
                        .account
                        .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                    self.anchor_fault(poisoned);
                    self.fallback.contain_pending_charge(fallback_charge);
                    return StartupPendingResolution {
                        rollback: PendingRetainedRollback::FaultRecorded(poisoned),
                    };
                }
                let rollback = self.fallback.rollback_pending_side(outer_reason);
                if let PendingRetainedRollback::FaultRecorded(poisoned)
                | PendingRetainedRollback::ChargeReleaseUnverified(poisoned)
                | PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) = rollback
                {
                    self.anchor_fault(poisoned);
                }
                if matches!(
                    rollback,
                    PendingRetainedRollback::ChargeReleaseUnverified(_)
                        | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
                ) {
                    self.fallback.contain_pending_charge(self.pair_plan.first);
                }
                StartupPendingResolution { rollback }
            }
            (None, Some((evidence_same_account, evidence_charge))) => {
                let exact = evidence_same_account && evidence_charge == self.pair_plan.second;
                if !exact {
                    let poisoned = self
                        .account
                        .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                    self.anchor_fault(poisoned);
                    self.evidence.contain_pending_charge(evidence_charge);
                    return StartupPendingResolution {
                        rollback: PendingRetainedRollback::FaultRecorded(poisoned),
                    };
                }
                let rollback = self.evidence.rollback_pending_side(outer_reason);
                if let PendingRetainedRollback::FaultRecorded(poisoned)
                | PendingRetainedRollback::ChargeReleaseUnverified(poisoned)
                | PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) = rollback
                {
                    self.anchor_fault(poisoned);
                }
                if matches!(
                    rollback,
                    PendingRetainedRollback::ChargeReleaseUnverified(_)
                        | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
                ) {
                    self.evidence.contain_pending_charge(self.pair_plan.second);
                }
                StartupPendingResolution { rollback }
            }
            (None, None) => {
                let rollback = match outer_reason {
                    Some(reason) => {
                        let poisoned = self.account.record_poison(reason);
                        self.anchor_fault(poisoned);
                        PendingRetainedRollback::FaultRecorded(poisoned)
                    }
                    None => PendingRetainedRollback::Clean,
                };
                StartupPendingResolution { rollback }
            }
        }
    }

    fn retain_side_attribution(&mut self, side: StartupRetainedSide) {
        match side {
            StartupRetainedSide::Fallback => {
                self.fallback.contain_attribution_after_anchored_fault()
            }
            StartupRetainedSide::Evidence => {
                self.evidence.contain_attribution_after_anchored_fault()
            }
        }
    }

    fn is_structurally_empty(&self) -> bool {
        self.pending_pair.is_none()
            && self.evidence_account.is_none()
            && self.fallback_lease_owner.is_none()
            && self.evidence_lease_owner.is_none()
            && self.fallback.is_empty()
            && self.evidence.is_empty()
            && self.contained_pending.is_none()
            && self.canonical_assembly.is_none()
            && self.evidence_assembly.is_none()
    }

    fn drain_handled_state(&mut self) {
        drop(std::mem::replace(
            &mut self.fallback,
            StartupRetainedSideState::Empty,
        ));
        drop(std::mem::replace(
            &mut self.evidence,
            StartupRetainedSideState::Empty,
        ));
        drop(self.pending_pair.take());
        drop(self.contained_pending.take());
        drop(self.evidence_account.take());
        drop(self.fallback_lease_owner.take());
        drop(self.evidence_lease_owner.take());
        drop(self.canonical_assembly.take());
        drop(self.evidence_assembly.take());
    }

    fn try_into_ready_pair(
        mut self,
    ) -> Result<StartupReadyPair<'account, O, P, D, F>, RetainedWriterError<D>> {
        let fallback = match self.fallback.take_ready() {
            Some(ready) => ready,
            None => return Err(self.finish_invalid_complete_state()),
        };
        let evidence = match self.evidence.take_ready() {
            Some(ready) => ready,
            None => {
                self.fallback = StartupRetainedSideState::Ready(fallback);
                return Err(self.finish_invalid_complete_state());
            }
        };
        let canonical_assembly = match self.canonical_assembly.take() {
            Some(batch) => batch,
            None => {
                self.fallback = StartupRetainedSideState::Ready(fallback);
                self.evidence = StartupRetainedSideState::Ready(evidence);
                return Err(self.finish_invalid_complete_state());
            }
        };
        let evidence_assembly = match self.evidence_assembly.take() {
            Some(batch) => batch,
            None => {
                self.fallback = StartupRetainedSideState::Ready(fallback);
                self.evidence = StartupRetainedSideState::Ready(evidence);
                self.canonical_assembly = Some(canonical_assembly);
                return Err(self.finish_invalid_complete_state());
            }
        };
        if self.pending_pair.is_some()
            || self.evidence_account.is_some()
            || self.fallback_lease_owner.is_some()
            || self.evidence_lease_owner.is_some()
            || self.contained_pending.is_some()
        {
            self.fallback = StartupRetainedSideState::Ready(fallback);
            self.evidence = StartupRetainedSideState::Ready(evidence);
            self.canonical_assembly = Some(canonical_assembly);
            self.evidence_assembly = Some(evidence_assembly);
            return Err(self.finish_invalid_complete_state());
        }
        Ok(StartupReadyPair {
            account: self.account,
            pair_plan: self.pair_plan,
            fallback,
            evidence,
            canonical_assembly,
            evidence_assembly,
        })
    }

    fn finish_invalid_complete_state(mut self) -> RetainedWriterError<D> {
        self.fallback.drop_unarmed_before_charge_resolution();
        self.evidence.drop_unarmed_before_charge_resolution();
        let resolution =
            self.rollback_remaining(Some(AccountPoisonReason::OwnershipGraphUnverified));
        match resolution.rollback {
            PendingRetainedRollback::FaultRecorded(poisoned)
            | PendingRetainedRollback::ChargeReleaseUnverified(poisoned)
            | PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
                self.anchor_fault(poisoned);
            }
            PendingRetainedRollback::Clean => {
                let poisoned = self
                    .account
                    .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                self.anchor_fault(poisoned);
            }
        }
        self.fallback.contain_built_after_recorded_fault();
        self.evidence.contain_built_after_recorded_fault();
        self.drain_handled_state();
        self.mark_drained();
        RetainedWriterError::OwnershipUnverified
    }

    fn finish_rejection(
        mut self,
        failed_side: StartupRetainedSide,
        failure: StartupRetainedSideFailure<D>,
    ) -> RetainedWriterError<D> {
        self.fallback.drop_unarmed_before_charge_resolution();
        self.evidence.drop_unarmed_before_charge_resolution();
        let resolution = self.rollback_remaining(failure.fault);
        match resolution.rollback {
            PendingRetainedRollback::FaultRecorded(poisoned)
            | PendingRetainedRollback::ChargeReleaseUnverified(poisoned)
            | PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
                self.anchor_fault(poisoned);
            }
            PendingRetainedRollback::Clean => {}
        }
        if failure.retain_attribution
            && matches!(
                resolution.rollback,
                PendingRetainedRollback::FaultRecorded(_)
                    | PendingRetainedRollback::ChargeReleaseUnverified(_)
                    | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
            )
        {
            self.retain_side_attribution(failed_side);
        }
        let storage_failure = matches!(failure.cause, RetainedWriterError::StorageDropUnverified);
        let ownership_failure = failure.fault.is_some()
            || matches!(
                resolution.rollback,
                PendingRetainedRollback::FaultRecorded(_)
                    | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
            );
        let charge_failure = matches!(
            resolution.rollback,
            PendingRetainedRollback::ChargeReleaseUnverified(_)
                | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
        );
        let error = if storage_failure {
            RetainedWriterError::StorageDropUnverified
        } else if ownership_failure {
            RetainedWriterError::OwnershipUnverified
        } else if charge_failure {
            RetainedWriterError::ChargeReleaseUnverified
        } else {
            failure.cause
        };
        self.drain_handled_state();
        self.mark_drained();
        error
    }

    fn finish_evidence_build_rejection(
        mut self,
        cause: RetainedBuildFailureCode,
    ) -> RetainedWriterError<D> {
        self.evidence.drop_unarmed_before_charge_resolution();
        let mut storage_failure = false;
        let mut ownership_failure = false;
        let mut charge_failure = false;
        let mut deferred_ownership = false;
        let mut cleanup_fault_recorded = false;
        let evidence_exact = self.evidence.pending_admission().is_some_and(|admission| {
            admission.account.is_same_account(self.account)
                && admission.charge == self.pair_plan.second
        });
        let rollback = if evidence_exact {
            self.evidence.rollback_pending_side(None)
        } else {
            let charge = self
                .evidence
                .pending_admission()
                .map(|admission| admission.charge);
            if let Some(charge) = charge {
                self.evidence.contain_pending_charge(charge);
            }
            ownership_failure = true;
            deferred_ownership = true;
            PendingRetainedRollback::Clean
        };
        match rollback {
            PendingRetainedRollback::Clean => {}
            PendingRetainedRollback::FaultRecorded(poisoned) => {
                ownership_failure = true;
                cleanup_fault_recorded = true;
                self.anchor_fault(poisoned);
            }
            PendingRetainedRollback::ChargeReleaseUnverified(poisoned) => {
                charge_failure = true;
                cleanup_fault_recorded = true;
                self.anchor_fault(poisoned);
                self.evidence.contain_pending_charge(self.pair_plan.second);
            }
            PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
                ownership_failure = true;
                self.anchor_fault(poisoned);
                self.evidence.contain_pending_charge(self.pair_plan.second);
            }
        }
        #[cfg(test)]
        if take_injected_start_fallback_account_anchor_loss()
            && let StartupRetainedSideState::Built(part) = &mut self.fallback
        {
            part.test_drop_account_anchor();
        }
        let defer_fallback_ownership = cleanup_fault_recorded || deferred_ownership;
        match self
            .fallback
            .try_discard_built_before_batch_release(defer_fallback_ownership)
        {
            Ok(()) => {}
            Err(RetainedPartCleanupError::ChargeReleaseUnverified(poisoned)) => {
                charge_failure = true;
                cleanup_fault_recorded |=
                    poisoned.reason == AccountPoisonReason::CounterInvariantViolated;
                self.anchor_fault(poisoned.poisoned);
            }
            Err(RetainedPartCleanupError::MissingFaultAnchor) => {
                ownership_failure = true;
                deferred_ownership = true;
            }
            Err(RetainedPartCleanupError::DeferredOwnershipFault) => {
                ownership_failure = true;
                deferred_ownership = true;
            }
            Err(RetainedPartCleanupError::StorageReleaseUnverified(poisoned)) => {
                storage_failure = true;
                self.anchor_fault(poisoned.poisoned);
            }
            Err(
                RetainedPartCleanupError::NewFaultAlreadyRecorded(poisoned)
                | RetainedPartCleanupError::StillSharedOwnershipFault(poisoned),
            ) => {
                ownership_failure = true;
                cleanup_fault_recorded = true;
                self.anchor_fault(poisoned.poisoned);
            }
        }
        if deferred_ownership && !cleanup_fault_recorded {
            let poisoned = self
                .account
                .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            self.anchor_fault(poisoned);
        }
        let error = if storage_failure {
            RetainedWriterError::StorageDropUnverified
        } else if ownership_failure {
            RetainedWriterError::OwnershipUnverified
        } else if charge_failure {
            RetainedWriterError::ChargeReleaseUnverified
        } else {
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        };
        self.drain_handled_state();
        self.mark_drained();
        error
    }

    fn finish_fallback_builder_panic(
        mut self,
        payload: Box<dyn std::any::Any + Send>,
    ) -> RetainedWriterError<D> {
        self.evidence.drop_unarmed_before_charge_resolution();
        let evidence_exact = self.evidence.pending_admission().is_some_and(|admission| {
            admission.account.is_same_account(self.account)
                && admission.charge == self.pair_plan.second
        });
        let sibling_rollback = if evidence_exact {
            self.evidence.rollback_pending_side(None)
        } else {
            let charge = self
                .evidence
                .pending_admission()
                .map(|admission| admission.charge);
            let poisoned = self
                .account
                .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            self.anchor_fault(poisoned);
            if let Some(charge) = charge {
                self.evidence.contain_pending_charge(charge);
            }
            PendingRetainedRollback::FaultRecorded(poisoned)
        };
        match sibling_rollback {
            PendingRetainedRollback::Clean => {
                let poisoned = self
                    .account
                    .record_poison(AccountPoisonReason::RetainedBuilderPanicked);
                self.anchor_fault(poisoned);
            }
            PendingRetainedRollback::FaultRecorded(poisoned)
            | PendingRetainedRollback::ChargeReleaseUnverified(poisoned) => {
                self.anchor_fault(poisoned);
                if matches!(
                    sibling_rollback,
                    PendingRetainedRollback::ChargeReleaseUnverified(_)
                ) {
                    self.evidence.contain_pending_charge(self.pair_plan.second);
                }
            }
            PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
                self.anchor_fault(poisoned);
                self.evidence.contain_pending_charge(self.pair_plan.second);
                let builder_fault = self
                    .account
                    .record_poison(AccountPoisonReason::RetainedBuilderPanicked);
                self.anchor_fault(builder_fault);
            }
        }
        self.fallback.contain_ready();
        discard_untrusted_panic_payload(payload);
        let error = match sibling_rollback {
            PendingRetainedRollback::ChargeReleaseUnverified(_) => {
                RetainedWriterError::ChargeReleaseUnverified
            }
            PendingRetainedRollback::ChargeRetainedAfterPoison(_) => {
                RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
            }
            PendingRetainedRollback::FaultRecorded(_) => RetainedWriterError::OwnershipUnverified,
            PendingRetainedRollback::Clean => {
                RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
            }
        };
        self.drain_handled_state();
        self.mark_drained();
        error
    }

    fn finish_evidence_builder_panic(
        mut self,
        payload: Box<dyn std::any::Any + Send>,
    ) -> RetainedWriterError<D> {
        self.evidence.contain_ready();
        let mut storage_failure = false;
        let mut ownership_failure = false;
        let mut charge_failure = false;
        let mut deferred_ownership = false;
        let mut primary_fault_recorded = false;

        match self.fallback.try_discard_built_before_batch_release(false) {
            Ok(()) => {}
            Err(RetainedPartCleanupError::ChargeReleaseUnverified(poisoned)) => {
                charge_failure = true;
                primary_fault_recorded |=
                    poisoned.reason == AccountPoisonReason::CounterInvariantViolated;
                self.anchor_fault(poisoned.poisoned);
            }
            Err(RetainedPartCleanupError::MissingFaultAnchor)
            | Err(RetainedPartCleanupError::DeferredOwnershipFault) => {
                ownership_failure = true;
                deferred_ownership = true;
            }
            Err(RetainedPartCleanupError::StorageReleaseUnverified(poisoned)) => {
                storage_failure = true;
                self.anchor_fault(poisoned.poisoned);
            }
            Err(
                RetainedPartCleanupError::NewFaultAlreadyRecorded(poisoned)
                | RetainedPartCleanupError::StillSharedOwnershipFault(poisoned),
            ) => {
                ownership_failure = true;
                primary_fault_recorded = true;
                self.anchor_fault(poisoned.poisoned);
            }
        }
        if !primary_fault_recorded {
            let reason = if deferred_ownership {
                AccountPoisonReason::OwnershipGraphUnverified
            } else {
                AccountPoisonReason::RetainedBuilderPanicked
            };
            let poisoned = self.account.record_poison(reason);
            self.anchor_fault(poisoned);
        }
        discard_untrusted_panic_payload(payload);
        let error = if storage_failure {
            RetainedWriterError::StorageDropUnverified
        } else if ownership_failure {
            RetainedWriterError::OwnershipUnverified
        } else if charge_failure {
            RetainedWriterError::ChargeReleaseUnverified
        } else {
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        };
        self.drain_handled_state();
        self.mark_drained();
        error
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "startup retains both physical parts under one continuous owner"
    )]
    fn try_build(
        mut self,
        ledger: &Ledger<O, P, D>,
        fallback_plan: StartupRetainedPartPlan<O, P, D, OutcomeEvidenceDestination>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
        evidence_plan: StartupRetainedPartPlan<O, P, D, OutcomeEvidenceDestination>,
        build_evidence: impl FnOnce() -> Result<PublicationEvidenceCells<D>, RetainedBuildFailureCode>,
    ) -> Result<StartupFallbackOwner<O, P, D, F>, RetainedWriterError<D>> {
        self.split_pending_pair();
        if let Err(failure) = self.fallback.try_prepare(ledger, fallback_plan) {
            return Err(self.finish_rejection(StartupRetainedSide::Fallback, failure));
        }
        if let Err(failure) = self.evidence.try_prepare(ledger, evidence_plan) {
            return Err(self.finish_rejection(StartupRetainedSide::Evidence, failure));
        }
        self.try_into_ready_pair()?
            .try_build(build_fallback, build_evidence)
    }
}

impl<O, P, D, F> Drop for StartupRetainedPartGuard<'_, O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    fn drop(&mut self) {
        if self.is_structurally_empty() {
            return;
        }
        if matches!(self.cleanup_disposition, StartupCleanupDisposition::Drained) {
            self.drain_handled_state();
            return;
        }
        if let StartupCleanupDisposition::FaultAnchored(witness) = self.cleanup_disposition {
            let _ = witness;
            self.fallback.drop_unarmed_before_charge_resolution();
            self.evidence.drop_unarmed_before_charge_resolution();
            self.contain_pending_after_anchored_fault();
            self.fallback.contain_built_after_recorded_fault();
            self.evidence.contain_built_after_recorded_fault();
            self.drain_handled_state();
            return;
        }
        self.fallback.drop_unarmed_before_charge_resolution();
        self.evidence.drop_unarmed_before_charge_resolution();
        let _resolution =
            self.rollback_remaining(Some(AccountPoisonReason::OwnershipGraphUnverified));
        self.fallback.contain_built_after_recorded_fault();
        self.evidence.contain_built_after_recorded_fault();
        self.drain_handled_state();
    }
}

/// One destination's fixed block capacity and ledger placement.
///
/// `total_blocks` is the exact preallocated slot capacity, not a required final length. The table
/// root consumes one attribution slot in addition to every allocated member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedBlockTablePlan<P, D> {
    writer: RetainedWriterPlan<P, D>,
    total_blocks: usize,
    max_new_blocks: usize,
}

impl<P: Copy, D: Copy> RetainedBlockTablePlan<P, D> {
    /// Creates a fixed table plan. `total_blocks` bounds every member while `max_new_blocks` bounds
    /// only allocations attributed to this operation; inherited members consume table capacity but
    /// retain their original physical charge and receive no new allocation attribution.
    pub const fn new(
        writer: RetainedWriterPlan<P, D>,
        total_blocks: usize,
        max_new_blocks: usize,
    ) -> Self {
        Self {
            writer,
            total_blocks,
            max_new_blocks,
        }
    }
}

/// Independent canonical and outcome-evidence plans for one retained publication.
///
/// A success plan is success-capable rather than success-forcing: control or ledger rejection may
/// atomically downgrade it to rich failure or minimal fallback. The outcome-evidence root must be
/// outcome-neutral so the same evidence graph remains valid for success and failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedPublicationPlan<P, D> {
    canonical: Option<RetainedBlockTablePlan<P, D>>,
    outcome_evidence: RetainedBlockTablePlan<P, D>,
}

impl<P: Copy, D: Copy> RetainedPublicationPlan<P, D> {
    /// Creates a success-capable plan with independent canonical and evidence tables.
    pub const fn success(
        canonical: RetainedBlockTablePlan<P, D>,
        outcome_evidence: RetainedBlockTablePlan<P, D>,
    ) -> Self {
        Self {
            canonical: Some(canonical),
            outcome_evidence,
        }
    }

    /// Creates an evidence-only plan that cannot construct or publish canonical output.
    pub const fn rich_failure(outcome_evidence: RetainedBlockTablePlan<P, D>) -> Self {
        Self {
            canonical: None,
            outcome_evidence,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedOutcomeMode {
    SuccessCapable,
    FailureOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssemblyContainmentProvenance {
    Healthy,
    NewFaultPending,
    RetainedFailureAlreadyRecorded(AccountPoisonReason),
}

enum RetainedTerminalRequest<CanonicalBlock, EvidenceBlock, PP, U> {
    Success {
        proof: RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
        final_progress: FinalProgress<PP, U>,
    },
    RichFailure {
        proof: RetainedEvidenceClosureProof<EvidenceBlock>,
        requested_cause: PreparationFailureCode,
        final_progress: FinalProgress<PP, U>,
    },
    MinimalFallback {
        requested_cause: PreparationFailureCode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedTerminalMode {
    Success {
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
    },
    RichFailure {
        evidence_root: RetainedSlotIdentity,
        cause: PreparationFailureCode,
    },
    MinimalFallback {
        cause: PreparationFailureCode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct RetainedSlotIdentity {
    set_id: RetainedSetId,
    index: usize,
    tag: TypeId,
}

impl RetainedSlotIdentity {
    pub(super) fn matches_tag<Tag: 'static>(self) -> bool {
        self.tag == TypeId::of::<Tag>()
    }
}

/// Non-owning projection to one canonical block in a branded retained publication.
///
/// A slot is valid only for the retained set that issued it. Sharing the same physical block into a
/// later generation issues a fresh slot; the old slot is rejected by the new aggregate.
#[must_use = "a retained slot is required to reference and validate its block"]
pub(crate) struct CanonicalRetainedSlot<Block, Tag> {
    identity: RetainedSlotIdentity,
    marker: PhantomData<fn() -> (Block, Tag)>,
}

impl<Block, Tag> Copy for CanonicalRetainedSlot<Block, Tag> {}

impl<Block, Tag> Clone for CanonicalRetainedSlot<Block, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Block, Tag> PartialEq for CanonicalRetainedSlot<Block, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl<Block, Tag> Eq for CanonicalRetainedSlot<Block, Tag> {}

impl<Block, Tag> fmt::Debug for CanonicalRetainedSlot<Block, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalRetainedSlot(..)")
    }
}

impl<Block, Tag> CanonicalRetainedSlot<Block, Tag> {
    pub(super) const fn from_identity(identity: RetainedSlotIdentity) -> Self {
        Self {
            identity,
            marker: PhantomData,
        }
    }

    pub(super) const fn identity(self) -> RetainedSlotIdentity {
        self.identity
    }
}

/// Non-owning projection to one outcome-evidence block in a branded retained publication.
///
/// Like a canonical slot, this identity is set-local and cannot be retagged or persisted as a
/// cross-generation identity.
#[must_use = "a retained slot is required to reference and validate its block"]
pub(crate) struct OutcomeEvidenceRetainedSlot<Block, Tag> {
    identity: RetainedSlotIdentity,
    marker: PhantomData<fn() -> (Block, Tag)>,
}

impl<Block, Tag> Copy for OutcomeEvidenceRetainedSlot<Block, Tag> {}

impl<Block, Tag> Clone for OutcomeEvidenceRetainedSlot<Block, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Block, Tag> PartialEq for OutcomeEvidenceRetainedSlot<Block, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl<Block, Tag> Eq for OutcomeEvidenceRetainedSlot<Block, Tag> {}

impl<Block, Tag> fmt::Debug for OutcomeEvidenceRetainedSlot<Block, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OutcomeEvidenceRetainedSlot(..)")
    }
}

impl<Block, Tag> OutcomeEvidenceRetainedSlot<Block, Tag> {
    pub(super) const fn from_identity(identity: RetainedSlotIdentity) -> Self {
        Self {
            identity,
            marker: PhantomData,
        }
    }

    pub(super) const fn identity(self) -> RetainedSlotIdentity {
        self.identity
    }
}

/// Scoped constructor that couples a complete immutable block to a domain-owned tag value.
///
/// Blocks are constructed exactly once and remain immutable after placement. The table interface
/// deliberately has no replacement, retirement, or generic mutable-access operation.
#[must_use = "the emplacer must place one complete block or be returned as a builder failure"]
pub(crate) struct BlockEmplacer<'a, Block> {
    marker: PhantomData<&'a mut Block>,
}

/// Unforgeable result of placing one complete block with a value witness.
#[must_use = "placement must be returned to the retained table builder"]
pub(crate) struct BlockPlacement<'a, Block, Tag> {
    block: Block,
    block_marker: PhantomData<&'a mut Block>,
    tag_marker: PhantomData<fn() -> Tag>,
}

impl<'a, Block> BlockEmplacer<'a, Block> {
    /// Places one complete immutable block and consumes the scoped emplacer.
    #[must_use = "the placement must be returned to the retained table builder"]
    pub fn place<Tag>(self, block: Block, tag: Tag) -> BlockPlacement<'a, Block, Tag>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let _ = tag;
        BlockPlacement {
            block,
            block_marker: PhantomData,
            tag_marker: PhantomData,
        }
    }
}

impl<Block, Tag> BlockPlacement<'_, Block, Tag> {
    fn into_block(self) -> Block {
        self.block
    }
}

struct RetainedBlockMember<Block> {
    allocation: RetainedAllocation<Block>,
    tag: TypeId,
    origin: RetainedBlockOrigin,
    reachable: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedBlockOrigin {
    New,
    Inherited,
}

struct RetainedBlockTable<Block> {
    members: Vec<RetainedBlockMember<Block>>,
    total_blocks: usize,
    max_new_blocks: usize,
    new_blocks: usize,
    qualified_capacity: usize,
}

impl<Block> RetainedBlockTable<Block> {
    fn member(&self, identity: RetainedSlotIdentity, set_id: RetainedSetId) -> Option<&Block> {
        if identity.set_id != set_id {
            return None;
        }
        self.members
            .get(identity.index)
            .and_then(|member| (member.tag == identity.tag).then(|| member.allocation.as_ref()))
    }

    fn member_allocation(
        &self,
        identity: RetainedSlotIdentity,
        set_id: RetainedSetId,
    ) -> Option<&RetainedAllocation<Block>> {
        #[cfg(test)]
        record_test_member_lookup();
        if identity.set_id != set_id {
            return None;
        }
        self.members
            .get(identity.index)
            .and_then(|member| (member.tag == identity.tag).then_some(&member.allocation))
    }

    fn reset_reachable(&self) {
        for member in &self.members {
            member.reachable.store(false, Ordering::Release);
        }
    }

    fn all_reachable(&self) -> bool {
        self.members
            .iter()
            .all(|member| member.reachable.load(Ordering::Acquire))
    }
}

fn record_forgotten_table_edges<Block>(table: &RetainedTableAllocation<Block>) {
    let _ = table
        .account()
        .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
    for member in &table.as_ref().members {
        let _ = member
            .allocation
            .account()
            .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
    }
}

fn release_retained_table_payload<Block>(
    payload: RetainedPayload<RetainedBlockTable<Block>>,
    mode: RetainedReleaseMode,
) -> RetainedValueReleaseOutcome {
    let RetainedPayload {
        value: mut table,
        charge: _,
    } = payload;
    let members = std::mem::take(&mut table.members);
    let mut nested_failure = None;
    let mut nested_charge_unverified = None;
    let mut nested_ownership_unverified = None;
    for RetainedBlockMember {
        allocation,
        tag: _,
        origin,
        reachable: _,
    } in members
    {
        let outcome = if mode == RetainedReleaseMode::BeforeAttribution
            && origin == RetainedBlockOrigin::New
        {
            match allocation.try_release_unique_before_attribution() {
                Ok(outcome) => outcome,
                Err((allocation, poisoned)) => {
                    std::mem::forget(allocation);
                    FinalOwnerReleaseOutcome::OwnershipUnverified(poisoned)
                }
            }
        } else {
            allocation.release_storage()
        };
        match outcome {
            FinalOwnerReleaseOutcome::Clean => {}
            FinalOwnerReleaseOutcome::StorageReleaseUnverified(poisoned) => {
                nested_failure.get_or_insert(poisoned);
            }
            FinalOwnerReleaseOutcome::ChargeReleaseUnverified(poisoned) => {
                nested_charge_unverified.get_or_insert(poisoned);
            }
            FinalOwnerReleaseOutcome::OwnershipUnverified(poisoned) => {
                nested_ownership_unverified.get_or_insert(poisoned);
            }
        }
    }
    drop(table);
    if let Some(poisoned) = nested_failure {
        RetainedValueReleaseOutcome::NestedStorageReleaseUnverified(poisoned)
    } else if let Some(poisoned) = nested_ownership_unverified {
        RetainedValueReleaseOutcome::NestedOwnershipUnverified(poisoned)
    } else if let Some(poisoned) = nested_charge_unverified {
        RetainedValueReleaseOutcome::NestedChargeReleaseUnverified(poisoned)
    } else {
        RetainedValueReleaseOutcome::Clean
    }
}

/// Read-only reachability session for a success-capable canonical/evidence graph.
///
/// Validation must visit every unique slot in both tables. Repeated DAG edges are valid and count
/// once. Any lookup returning `None` invalidates the whole session even if the callback ignores it.
pub(crate) struct RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock> {
    canonical: &'a RetainedBlockTable<CanonicalBlock>,
    evidence: &'a RetainedBlockTable<EvidenceBlock>,
    set_id: RetainedSetId,
    canonical_seen: usize,
    evidence_seen: usize,
    valid: bool,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'a, CanonicalBlock, EvidenceBlock> RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock> {
    /// Visits one canonical slot, returning `None` and invalidating the session on mismatch.
    pub fn canonical<Tag>(
        &mut self,
        slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
    ) -> Option<&'a CanonicalBlock>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.set_id != self.set_id || slot.identity.tag != TypeId::of::<Tag>() {
            self.valid = false;
            return None;
        }
        let Some(member) = self.canonical.members.get(slot.identity.index) else {
            self.valid = false;
            return None;
        };
        if member.tag != slot.identity.tag {
            self.valid = false;
            return None;
        }
        if !member.reachable.swap(true, Ordering::AcqRel) {
            let Some(seen) = self.canonical_seen.checked_add(1) else {
                self.valid = false;
                return None;
            };
            self.canonical_seen = seen;
        }
        Some(member.allocation.as_ref())
    }

    /// Visits one outcome-evidence slot, returning `None` and invalidating the session on mismatch.
    pub fn outcome_evidence<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<&'a EvidenceBlock>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.set_id != self.set_id || slot.identity.tag != TypeId::of::<Tag>() {
            self.valid = false;
            return None;
        }
        let Some(member) = self.evidence.members.get(slot.identity.index) else {
            self.valid = false;
            return None;
        };
        if member.tag != slot.identity.tag {
            self.valid = false;
            return None;
        }
        if !member.reachable.swap(true, Ordering::AcqRel) {
            let Some(seen) = self.evidence_seen.checked_add(1) else {
                self.valid = false;
                return None;
            };
            self.evidence_seen = seen;
        }
        Some(member.allocation.as_ref())
    }
}

/// Read-only reachability session for an evidence-only graph.
///
/// Validation must visit every unique evidence slot. Repeated DAG edges count once, and a failed
/// lookup invalidates the complete session.
pub(crate) struct RetainedEvidenceClosureSession<'a, EvidenceBlock> {
    evidence: &'a RetainedBlockTable<EvidenceBlock>,
    set_id: RetainedSetId,
    seen: usize,
    valid: bool,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'a, EvidenceBlock> RetainedEvidenceClosureSession<'a, EvidenceBlock> {
    /// Visits one outcome-evidence slot, returning `None` and invalidating the session on mismatch.
    pub fn outcome_evidence<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<&'a EvidenceBlock>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.set_id != self.set_id || slot.identity.tag != TypeId::of::<Tag>() {
            self.valid = false;
            return None;
        }
        let Some(member) = self.evidence.members.get(slot.identity.index) else {
            self.valid = false;
            return None;
        };
        if member.tag != slot.identity.tag {
            self.valid = false;
            return None;
        }
        if !member.reachable.swap(true, Ordering::AcqRel) {
            let Some(seen) = self.seen.checked_add(1) else {
                self.valid = false;
                return None;
            };
            self.seen = seen;
        }
        Some(member.allocation.as_ref())
    }
}

/// Module-private exact-closure witness consumed by success preparation.
#[must_use = "the closure proof must be consumed by terminal preparation"]
struct RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock> {
    set_id: RetainedSetId,
    canonical_count: usize,
    evidence_count: usize,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
    marker: PhantomData<fn() -> (CanonicalBlock, EvidenceBlock)>,
}

/// Module-private exact-closure witness consumed by evidence-only preparation.
#[must_use = "the closure proof must be consumed by terminal preparation"]
struct RetainedEvidenceClosureProof<EvidenceBlock> {
    set_id: RetainedSetId,
    evidence_count: usize,
    evidence_root: RetainedSlotIdentity,
    marker: PhantomData<fn() -> EvidenceBlock>,
}

/// Failure to validate the complete reachable block closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
enum RetainedClosureValidationError {
    /// A prior contained failure poisoned the assembly.
    Poisoned,
    /// Success validation was requested from an evidence-only assembly.
    WrongOutcomeMode,
    /// A supplied root is stale, foreign, or carries the wrong tag.
    InvalidRoot,
    /// The callback rejected the graph or did not visit its complete unique closure.
    Rejected,
    /// The validation callback panicked and the candidate must become a minimal fallback.
    Panicked,
}

impl fmt::Display for RetainedClosureValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Poisoned => formatter.write_str("retained closure validation is poisoned"),
            Self::WrongOutcomeMode => {
                formatter.write_str("success closure validation requires a success-capable plan")
            }
            Self::InvalidRoot => formatter.write_str("retained closure root is invalid"),
            Self::Rejected => formatter.write_str("retained closure validation was rejected"),
            Self::Panicked => formatter.write_str("retained closure validation panicked"),
        }
    }
}

impl std::error::Error for RetainedClosureValidationError {}

struct PublishedRetainedAggregate<Block> {
    table: RetainedTableAllocation<Block>,
    set_id: RetainedSetId,
}

impl<Block> PublishedRetainedAggregate<Block> {
    fn member(&self, identity: RetainedSlotIdentity) -> Option<&Block> {
        self.table.as_ref().member(identity, self.set_id)
    }

    fn len(&self) -> usize {
        self.table.as_ref().members.len()
    }
}

/// Immutable canonical block table after the atomic publication handoff.
pub(crate) struct PublishedCanonicalAggregate<Block> {
    inner: PublishedRetainedAggregate<Block>,
}

impl<Block> PublishedCanonicalAggregate<Block> {
    pub fn get<Tag>(&self, slot: CanonicalRetainedSlot<Block, Tag>) -> Option<&Block>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.tag != TypeId::of::<Tag>() {
            return None;
        }
        self.inner.member(slot.identity)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Immutable outcome-evidence block table after the atomic publication handoff.
pub(crate) struct PublishedOutcomeEvidenceAggregate<Block> {
    inner: PublishedRetainedAggregate<Block>,
}

impl<Block> PublishedOutcomeEvidenceAggregate<Block> {
    pub fn get<Tag>(&self, slot: OutcomeEvidenceRetainedSlot<Block, Tag>) -> Option<&Block>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.tag != TypeId::of::<Tag>() {
            return None;
        }
        self.inner.member(slot.identity)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Failure to open a physically accounted retained publication.
///
/// [`Self::try_into_operation`] returns the operation only when every failed construction step
/// proved that storage and accounting were rolled back. An unverified payload drop deliberately
/// retains the operation and its charges fail-closed inside this error.
#[must_use = "the error owns the operation until recovery is proven or the error is dropped"]
pub(crate) struct RetainedPublicationOpenError<Operation, D> {
    operation: Operation,
    cause: RetainedWriterError<D>,
    recoverable: bool,
}

impl<Operation, D: fmt::Debug> fmt::Debug for RetainedPublicationOpenError<Operation, D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedPublicationOpenError")
            .field("cause", &self.cause)
            .field("recoverable", &self.recoverable)
            .finish_non_exhaustive()
    }
}

impl<Operation, D: fmt::Debug> fmt::Display for RetainedPublicationOpenError<Operation, D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "retained publication could not be opened: {}",
            self.cause
        )
    }
}

impl<Operation, D: fmt::Debug + 'static> std::error::Error
    for RetainedPublicationOpenError<Operation, D>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

impl<Operation, D> RetainedPublicationOpenError<Operation, D> {
    pub fn cause(&self) -> &RetainedWriterError<D> {
        &self.cause
    }

    pub fn try_into_operation(self) -> Result<Operation, RetainedWriterError<D>> {
        if self.recoverable {
            Ok(self.operation)
        } else {
            Err(self.cause)
        }
    }
}

fn retained_open_failure_is_recoverable<D>(cause: &RetainedWriterError<D>) -> bool {
    !matches!(cause, RetainedWriterError::Physical(error) if error.charge_retained())
        && !matches!(
            cause,
            RetainedWriterError::StorageDropUnverified
                | RetainedWriterError::OwnershipUnverified
                | RetainedWriterError::ChargeReleaseUnverified
        )
        && !matches!(cause, RetainedWriterError::AccountPoisoned(_))
}

/// Recoverable construction failure for an operation-owned retained block table.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) enum RetainedPublicationError<D> {
    /// The underlying retained writer rejected an accounted action.
    Writer(RetainedWriterError<D>),
    /// The canonical table has reached its fixed slot limit.
    CanonicalSlotLimit,
    /// The outcome-evidence table has reached its fixed slot limit.
    OutcomeEvidenceSlotLimit,
    /// A canonical action was requested from an evidence-only assembly.
    WrongOutcomeMode,
    /// An inherited source slot is stale, foreign to its aggregate, or carries the wrong tag.
    InvalidSourceSlot,
    /// A prior failure made safe rollback or publication unverifiable.
    Poisoned,
}

impl<D: fmt::Debug> fmt::Display for RetainedPublicationError<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Writer(cause) => write!(formatter, "retained publication action failed: {cause}"),
            Self::CanonicalSlotLimit => formatter.write_str("canonical retained table is full"),
            Self::OutcomeEvidenceSlotLimit => {
                formatter.write_str("outcome-evidence retained table is full")
            }
            Self::WrongOutcomeMode => {
                formatter.write_str("canonical action requires a success-capable plan")
            }
            Self::InvalidSourceSlot => {
                formatter.write_str("retained inherited source slot is invalid")
            }
            Self::Poisoned => formatter.write_str("retained publication assembly is poisoned"),
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for RetainedPublicationError<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Writer(cause) => Some(cause),
            _ => None,
        }
    }
}

fn retained_publication_error_containment<D>(
    error: &RetainedPublicationError<D>,
) -> Option<AssemblyContainmentProvenance> {
    let reason = match error {
        RetainedPublicationError::Poisoned => {
            return Some(AssemblyContainmentProvenance::NewFaultPending);
        }
        RetainedPublicationError::Writer(RetainedWriterError::StorageDropUnverified) => {
            AccountPoisonReason::RetainedStorageReleaseUnverified
        }
        RetainedPublicationError::Writer(RetainedWriterError::OwnershipUnverified) => {
            AccountPoisonReason::OwnershipGraphUnverified
        }
        RetainedPublicationError::Writer(RetainedWriterError::ChargeReleaseUnverified) => {
            AccountPoisonReason::CounterInvariantViolated
        }
        RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(cause)) => {
            quarantined_account_reason(*cause)
        }
        RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::BuilderPanicked,
        )) => AccountPoisonReason::RetainedBuilderPanicked,
        RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::ChargeReleaseUnverified,
        )) => AccountPoisonReason::CounterInvariantViolated,
        RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::AccountPoisonedWithRetainedCharge(cause),
        )) => quarantined_account_reason(*cause),
        _ => return None,
    };
    Some(AssemblyContainmentProvenance::RetainedFailureAlreadyRecorded(reason))
}

pub(super) fn retained_publication_error_requires_containment<D>(
    error: &RetainedPublicationError<D>,
) -> bool {
    retained_publication_error_containment(error).is_some()
}

/// Stable operation owner for two preplanned, physically accounted block tables.
#[must_use = "the assembly owns the operation, retained storage, and live attribution"]
pub(crate) struct RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    canonical_batch: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_batch: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    account: Option<PhysicalStorageAccount>,
    operation: Option<RetainedOperation<O, P, PP, U, D, F>>,
    set_id: RetainedSetId,
    canonical_plan: Option<RetainedBlockTablePlan<P, D>>,
    evidence_plan: RetainedBlockTablePlan<P, D>,
    handle_share_work: RetainedHandleSharePlan<D>,
    mode: RetainedOutcomeMode,
    containment: AssemblyContainmentProvenance,
}

struct TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock> {
    fallback: Option<DetachedRetainedPart<F>>,
    evidence: Option<DetachedRetainedPart<PublicationEvidenceCells<D>>>,
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    account: Option<PhysicalStorageAccount>,
}

type TerminalPhysicalParts<D, F, CanonicalBlock, EvidenceBlock> = (
    DetachedRetainedPart<F>,
    DetachedRetainedPart<PublicationEvidenceCells<D>>,
    Option<RetainedTableAllocation<CanonicalBlock>>,
    Option<RetainedTableAllocation<EvidenceBlock>>,
    Option<PhysicalStorageAccount>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalTableCleanup {
    canonical: RetainedStorageDropOutcome,
    evidence: RetainedStorageDropOutcome,
}

impl TerminalTableCleanup {
    fn outcome(self) -> RetainedStorageDropOutcome {
        self.canonical.join(self.evidence)
    }

    fn failure_reason(self) -> Option<AccountPoisonReason> {
        match self.outcome() {
            RetainedStorageDropOutcome::Clean => None,
            RetainedStorageDropOutcome::StorageReleaseUnverified(witness)
            | RetainedStorageDropOutcome::ChargeReleaseUnverified(witness)
            | RetainedStorageDropOutcome::StillSharedOwnershipFault(witness)
            | RetainedStorageDropOutcome::ForeignOwnershipFault(witness) => Some(witness.reason),
            RetainedStorageDropOutcome::MissingFaultAnchor => {
                Some(AccountPoisonReason::OwnershipGraphUnverified)
            }
        }
    }
}

impl<D, F, CanonicalBlock, EvidenceBlock>
    TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock>
{
    fn new(
        fallback: DetachedRetainedPart<F>,
        evidence: DetachedRetainedPart<PublicationEvidenceCells<D>>,
        canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
        evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
        account: Option<PhysicalStorageAccount>,
    ) -> Self {
        Self {
            fallback: Some(fallback),
            evidence: Some(evidence),
            canonical_table,
            evidence_table,
            account,
        }
    }

    fn account(&self) -> &PhysicalStorageAccount {
        match self.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }

    fn evidence(&self) -> &DetachedRetainedPart<PublicationEvidenceCells<D>> {
        match self.evidence.as_ref() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        }
    }

    fn take_account(&mut self) -> Option<PhysicalStorageAccount> {
        self.account.take()
    }

    fn try_drop_primary_tables(&mut self, expect_canonical: bool) -> TerminalTableCleanup {
        let mut canonical = if self.canonical_table.is_some() == expect_canonical {
            RetainedStorageDropOutcome::Clean
        } else {
            RetainedStorageDropOutcome::ForeignOwnershipFault(
                self.record_new_fault(AccountPoisonReason::OwnershipGraphUnverified),
            )
        };
        let mut evidence = if self.evidence_table.is_some() {
            RetainedStorageDropOutcome::Clean
        } else {
            RetainedStorageDropOutcome::ForeignOwnershipFault(
                self.record_new_fault(AccountPoisonReason::OwnershipGraphUnverified),
            )
        };
        if let Some(table) = self.canonical_table.take() {
            canonical = canonical.join(drop_retained_table_with_account_anchor(
                table,
                self.account(),
            ));
        }
        if let Some(table) = self.evidence_table.take() {
            evidence = evidence.join(drop_retained_table_with_account_anchor(
                table,
                self.account(),
            ));
        }
        TerminalTableCleanup {
            canonical,
            evidence,
        }
    }

    fn try_discard_fallback_allocation(
        &mut self,
    ) -> Result<PhysicalStorageAccount, RetainedPartCleanupError> {
        match self.fallback.as_mut() {
            Some(fallback) => fallback.try_discard_allocation(),
            None => Err(RetainedPartCleanupError::MissingFaultAnchor),
        }
    }

    fn first_account(&self) -> Option<&PhysicalStorageAccount> {
        self.account
            .as_ref()
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(DetachedRetainedPart::account_anchor)
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(DetachedRetainedPart::account_anchor)
            })
            .or_else(|| self.canonical_table.as_ref().map(|table| table.account()))
            .or_else(|| {
                self.canonical_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| self.evidence_table.as_ref().map(|table| table.account()))
            .or_else(|| {
                self.evidence_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(DetachedRetainedPart::allocation_anchor)
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(DetachedRetainedPart::allocation_anchor)
            })
    }

    fn has_exact_fallback_shape(&self) -> bool {
        let (Some(fallback), Some(evidence)) = (self.fallback.as_ref(), self.evidence.as_ref())
        else {
            return false;
        };
        self.canonical_table.is_none()
            && self.evidence_table.is_none()
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(evidence.account())
            && self
                .account
                .as_ref()
                .is_none_or(|account| account.is_same_account(evidence.account()))
    }

    fn record_forgotten_physical_edges(
        &self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let account = match self.first_account() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        if let Some(canonical) = self.canonical_table.as_ref() {
            record_forgotten_table_edges(canonical);
        }
        if let Some(evidence) = self.evidence_table.as_ref() {
            record_forgotten_table_edges(evidence);
        }
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.record_forgotten_physical_edge();
        }
        if let Some(evidence) = self.evidence.as_ref() {
            evidence.record_forgotten_physical_edge();
        }
        FinalizationContainmentError::new(AccountPoisoned::new(account.snapshot()), reason)
    }

    fn record_new_fault(&self, reason: AccountPoisonReason) -> AccountPoisonEventWitness {
        let account = match self.account.as_ref().or_else(|| self.first_account()) {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        record_account_poison_event(account, reason)
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(canonical) = self.canonical_table.take() {
            std::mem::forget(canonical);
        }
        if let Some(evidence) = self.evidence_table.take() {
            std::mem::forget(evidence);
        }
        if let Some(account) = self.account.take() {
            drop(account);
        }
        if let Some(mut fallback) = self.fallback.take() {
            fallback.fail_closed_after_recorded_poison();
            drop(fallback);
        }
        if let Some(mut evidence) = self.evidence.take() {
            evidence.fail_closed_after_recorded_poison();
            drop(evidence);
        }
    }

    fn contain_new_fault(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        self.record_new_fault(reason);
        let cause = self.record_forgotten_physical_edges(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let cause = self.record_forgotten_physical_edges(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    fn contain_after_quarantine(self, cause: AccountPoisoned) -> FinalizationContainmentError {
        let reason = cause.snapshot().first_poison_evidence().map_or(
            AccountPoisonReason::OwnershipGraphUnverified,
            AccountPoisonEvidence::reason,
        );
        self.contain_after_recorded(reason)
    }

    fn into_parts(mut self) -> TerminalPhysicalParts<D, F, CanonicalBlock, EvidenceBlock> {
        let fallback = match self.fallback.take() {
            Some(fallback) => fallback,
            None => unreachable_final_owner_state(),
        };
        let evidence = match self.evidence.take() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        };
        (
            fallback,
            evidence,
            self.canonical_table.take(),
            self.evidence_table.take(),
            self.account.take(),
        )
    }

    fn qualifies_exact_fallback(&self) -> bool {
        self.has_exact_fallback_shape() && self.account.is_none()
    }

    fn finish_exact_fallback(mut self) -> ExactFallbackPhysicalOwnership<D, F> {
        let fallback = match self.fallback.take() {
            Some(fallback) => fallback,
            None => unreachable_final_owner_state(),
        };
        let evidence = match self.evidence.take() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        };
        ExactFallbackPhysicalOwnership { fallback, evidence }
    }

    fn qualifies_exact_terminal(&self, terminal: RetainedTerminalMode) -> bool {
        let Some(account) = self.account.as_ref() else {
            return false;
        };
        let common_exact = self
            .fallback
            .as_ref()
            .is_some_and(DetachedRetainedPart::is_empty)
            && self.evidence.as_ref().is_some_and(|evidence| {
                evidence.has_exact_owner() && evidence.account().is_same_account(account)
            });
        let terminal_exact = match terminal {
            RetainedTerminalMode::Success { .. } => {
                self.canonical_table
                    .as_ref()
                    .is_some_and(|table| table.belongs_to(account))
                    && self
                        .evidence_table
                        .as_ref()
                        .is_some_and(|table| table.belongs_to(account))
            }
            RetainedTerminalMode::RichFailure { .. } => {
                self.canonical_table.is_none()
                    && self
                        .evidence_table
                        .as_ref()
                        .is_some_and(|table| table.belongs_to(account))
            }
            RetainedTerminalMode::MinimalFallback { .. } => false,
        };
        common_exact && terminal_exact
    }

    fn finish_exact_terminal(
        mut self,
        terminal: RetainedTerminalMode,
    ) -> ExactTerminalPhysicalOwnership<D, F, CanonicalBlock, EvidenceBlock> {
        let fallback = match self.fallback.take() {
            Some(fallback) => fallback,
            None => unreachable_final_owner_state(),
        };
        debug_assert!(fallback.is_empty());
        drop(fallback);
        let evidence = match self.evidence.take() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        };
        let account = match self.account.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        match terminal {
            RetainedTerminalMode::Success {
                canonical_root,
                evidence_root,
            } => {
                let canonical_table = match self.canonical_table.take() {
                    Some(table) => table,
                    None => unreachable_final_owner_state(),
                };
                let evidence_table = match self.evidence_table.take() {
                    Some(table) => table,
                    None => unreachable_final_owner_state(),
                };
                ExactTerminalPhysicalOwnership::Success {
                    evidence,
                    canonical_table,
                    evidence_table,
                    account,
                    canonical_root,
                    evidence_root,
                    fallback_type: PhantomData,
                }
            }
            RetainedTerminalMode::RichFailure {
                evidence_root,
                cause,
            } => {
                let evidence_table = match self.evidence_table.take() {
                    Some(table) => table,
                    None => unreachable_final_owner_state(),
                };
                ExactTerminalPhysicalOwnership::RichFailure {
                    evidence,
                    evidence_table,
                    account,
                    evidence_root,
                    cause,
                    fallback_type: PhantomData,
                }
            }
            RetainedTerminalMode::MinimalFallback { .. } => unreachable_final_owner_state(),
        }
    }
}

impl<D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock>
{
    fn drop(&mut self) {
        if self.first_account().is_none() {
            return;
        }
        self.record_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        let _ = self.record_forgotten_physical_edges(AccountPoisonReason::OwnershipGraphUnverified);
        self.fail_closed_after_recorded_poison();
    }
}

/// Exact fallback-only physical graph established before terminal eligibility and freeze.
struct ExactFallbackPhysicalOwnership<D, F> {
    fallback: DetachedRetainedPart<F>,
    evidence: DetachedRetainedPart<PublicationEvidenceCells<D>>,
}

impl<D, F> ExactFallbackPhysicalOwnership<D, F> {
    fn account(&self) -> &PhysicalStorageAccount {
        self.evidence.account()
    }

    fn evidence(&self) -> &DetachedRetainedPart<PublicationEvidenceCells<D>> {
        &self.evidence
    }

    fn into_terminal<CanonicalBlock, EvidenceBlock>(
        self,
    ) -> TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock> {
        TerminalPhysicalOwnershipManifest::new(self.fallback, self.evidence, None, None, None)
    }

    fn into_publication<CanonicalBlock, EvidenceBlock>(
        self,
        cause: PreparationFailureCode,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D> {
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::MinimalFallback {
                part: PublishedPart::from_exact_owner(self.fallback),
                cause,
            },
            evidence: PublishedPart::from_exact_owner(self.evidence),
        }
    }
}

/// Exact success or rich-failure graph proven before terminal eligibility and freeze.
enum ExactTerminalPhysicalOwnership<D, F, CanonicalBlock, EvidenceBlock> {
    Success {
        evidence: DetachedRetainedPart<PublicationEvidenceCells<D>>,
        canonical_table: RetainedTableAllocation<CanonicalBlock>,
        evidence_table: RetainedTableAllocation<EvidenceBlock>,
        account: PhysicalStorageAccount,
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
        fallback_type: PhantomData<fn() -> F>,
    },
    RichFailure {
        evidence: DetachedRetainedPart<PublicationEvidenceCells<D>>,
        evidence_table: RetainedTableAllocation<EvidenceBlock>,
        account: PhysicalStorageAccount,
        evidence_root: RetainedSlotIdentity,
        cause: PreparationFailureCode,
        fallback_type: PhantomData<fn() -> F>,
    },
}

impl<D, F, CanonicalBlock, EvidenceBlock>
    ExactTerminalPhysicalOwnership<D, F, CanonicalBlock, EvidenceBlock>
{
    fn account(&self) -> &PhysicalStorageAccount {
        match self {
            Self::Success { account, .. } | Self::RichFailure { account, .. } => account,
        }
    }

    fn evidence(&self) -> &DetachedRetainedPart<PublicationEvidenceCells<D>> {
        match self {
            Self::Success { evidence, .. } | Self::RichFailure { evidence, .. } => evidence,
        }
    }

    fn into_terminal(
        self,
    ) -> TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock> {
        match self {
            Self::Success {
                evidence,
                canonical_table,
                evidence_table,
                account,
                ..
            } => TerminalPhysicalOwnershipManifest::new(
                DetachedRetainedPart::empty(),
                evidence,
                Some(canonical_table),
                Some(evidence_table),
                Some(account),
            ),
            Self::RichFailure {
                evidence,
                evidence_table,
                account,
                ..
            } => TerminalPhysicalOwnershipManifest::new(
                DetachedRetainedPart::empty(),
                evidence,
                None,
                Some(evidence_table),
                Some(account),
            ),
        }
    }

    fn into_publication(
        self,
        set_id: RetainedSetId,
    ) -> (
        RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F>,
        PublishedPart<PublicationEvidenceCells<D>>,
    ) {
        match self {
            Self::Success {
                evidence,
                canonical_table,
                evidence_table,
                account,
                canonical_root,
                evidence_root,
                fallback_type: _,
            } => (
                RetainedPublicationPayload::Success {
                    canonical: PublishedCanonicalAggregate {
                        inner: PublishedRetainedAggregate {
                            table: canonical_table,
                            set_id,
                        },
                    },
                    outcome_evidence: PublishedOutcomeEvidenceAggregate {
                        inner: PublishedRetainedAggregate {
                            table: evidence_table,
                            set_id,
                        },
                    },
                    canonical_root,
                    outcome_evidence_root: evidence_root,
                    account,
                },
                PublishedPart::from_exact_owner(evidence),
            ),
            Self::RichFailure {
                evidence,
                evidence_table,
                account,
                evidence_root,
                cause,
                fallback_type: _,
            } => (
                RetainedPublicationPayload::RichFailure {
                    outcome_evidence: PublishedOutcomeEvidenceAggregate {
                        inner: PublishedRetainedAggregate {
                            table: evidence_table,
                            set_id,
                        },
                    },
                    outcome_evidence_root: evidence_root,
                    account,
                    cause,
                },
                PublishedPart::from_exact_owner(evidence),
            ),
        }
    }
}

/// Linear owner spanning the primary final-eligibility decision.
///
/// Attribution cannot be released while fallible physical cleanup or account eligibility remains.
/// A successful eligibility check consumes this guard into the pure freeze handoff. Every other
/// exit contains physical storage first and deliberately retains the coupled attribution proof.
struct TerminalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<ExactTerminalPhysicalOwnership<D, F, CanonicalBlock, EvidenceBlock>>,
    discarded_account: Option<PhysicalStorageAccount>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: ExactTerminalPhysicalOwnership<D, F, CanonicalBlock, EvidenceBlock>,
        discarded_account: PhysicalStorageAccount,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
            discarded_account: Some(discarded_account),
        }
    }

    fn contain_after_quarantine(
        mut self,
        quarantine: AccountPoisoned,
    ) -> FinalizationContainmentError {
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let cause = physical
            .into_terminal()
            .contain_after_quarantine(quarantine);
        if let Some(proof) = self.proof.take() {
            std::mem::forget(proof);
        }
        drop(self.discarded_account.take());
        cause
    }

    fn try_into_eligible(
        mut self,
    ) -> Result<
        EligibleTerminalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        FinalizationContainmentError,
    > {
        let eligibility = match self.physical.as_ref() {
            Some(physical) => match physical.account().try_publication_eligibility() {
                Ok(eligibility) => eligibility,
                Err(quarantine) => return Err(self.contain_after_quarantine(quarantine)),
            },
            None => unreachable_final_owner_state(),
        };
        let proof = match self.proof.take() {
            Some(proof) => proof,
            None => unreachable_final_owner_state(),
        };
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        Ok(EligibleTerminalFreezeGuard {
            eligibility: Some(eligibility),
            proof: Some(proof),
            physical: Some(physical),
            discarded_account: self.discarded_account.take(),
        })
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            std::mem::forget(proof);
        }
        drop(self.discarded_account.take());
    }
}

/// Exact primary ownership after final account eligibility has linearized successfully.
///
/// Only this state may freeze terminal evidence and construct a prepared publication.
#[must_use = "eligible terminal ownership must complete preparation or contain its graph"]
struct EligibleTerminalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    eligibility: Option<PublicationEligibility>,
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<ExactTerminalPhysicalOwnership<D, F, CanonicalBlock, EvidenceBlock>>,
    discarded_account: Option<PhysicalStorageAccount>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    EligibleTerminalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn into_prepared(
        mut self,
        set_id: RetainedSetId,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D> {
        let eligibility = match self.eligibility.take() {
            Some(eligibility) => eligibility,
            None => unreachable_final_owner_state(),
        };
        let proof = match self.proof.take() {
            Some(proof) => proof,
            None => unreachable_final_owner_state(),
        };
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        freeze_terminal_evidence(
            eligibility,
            proof,
            self.discarded_account.take(),
            physical.evidence(),
        );
        let (payload, evidence) = physical.into_publication(set_id);
        PreparedRetainedPublication { payload, evidence }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for EligibleTerminalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            std::mem::forget(proof);
        }
        drop(self.discarded_account.take());
        let _ = self.eligibility.take();
    }
}

/// Linear owner for an exact fallback graph until account eligibility succeeds.
struct FallbackFreezeGuard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<ExactFallbackPhysicalOwnership<D, F>>,
    discarded_account: Option<PhysicalStorageAccount>,
}

impl<O, P, D, F> FallbackFreezeGuard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: ExactFallbackPhysicalOwnership<D, F>,
        discarded_account: Option<PhysicalStorageAccount>,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
            discarded_account,
        }
    }

    fn contain_after_quarantine(
        mut self,
        quarantine: AccountPoisoned,
    ) -> FinalizationContainmentError {
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let cause = physical
            .into_terminal::<(), ()>()
            .contain_after_quarantine(quarantine);
        if let Some(proof) = self.proof.take() {
            std::mem::forget(proof);
        }
        drop(self.discarded_account.take());
        cause
    }

    fn try_into_eligible(
        mut self,
    ) -> Result<EligibleFallbackFreezeGuard<O, P, D, F>, FinalizationContainmentError> {
        let eligibility = match self.physical.as_ref() {
            Some(physical) => match physical.account().try_publication_eligibility() {
                Ok(eligibility) => eligibility,
                Err(quarantine) => return Err(self.contain_after_quarantine(quarantine)),
            },
            None => unreachable_final_owner_state(),
        };
        let proof = match self.proof.take() {
            Some(proof) => proof,
            None => unreachable_final_owner_state(),
        };
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        Ok(EligibleFallbackFreezeGuard {
            eligibility: Some(eligibility),
            proof: Some(proof),
            physical: Some(physical),
            discarded_account: self.discarded_account.take(),
        })
    }
}

impl<O, P, D, F> Drop for FallbackFreezeGuard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical
                .into_terminal::<(), ()>()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            std::mem::forget(proof);
        }
        drop(self.discarded_account.take());
    }
}

/// Exact fallback ownership after final account eligibility has linearized successfully.
#[must_use = "eligible fallback ownership must complete preparation or contain its graph"]
struct EligibleFallbackFreezeGuard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    eligibility: Option<PublicationEligibility>,
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<ExactFallbackPhysicalOwnership<D, F>>,
    discarded_account: Option<PhysicalStorageAccount>,
}

impl<O, P, D, F> EligibleFallbackFreezeGuard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn into_prepared<CanonicalBlock, EvidenceBlock>(
        mut self,
        cause: PreparationFailureCode,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D> {
        let eligibility = match self.eligibility.take() {
            Some(eligibility) => eligibility,
            None => unreachable_final_owner_state(),
        };
        let proof = match self.proof.take() {
            Some(proof) => proof,
            None => unreachable_final_owner_state(),
        };
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        freeze_terminal_evidence(
            eligibility,
            proof,
            self.discarded_account.take(),
            physical.evidence(),
        );
        physical.into_publication(cause)
    }
}

impl<O, P, D, F> Drop for EligibleFallbackFreezeGuard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical
                .into_terminal::<(), ()>()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            std::mem::forget(proof);
        }
        drop(self.discarded_account.take());
        let _ = self.eligibility.take();
    }
}

/// Exact success closure whose owning assembly can no longer be mutated.
#[must_use = "validated retained success must be prepared or deliberately dropped"]
pub(crate) struct ValidatedRetainedSuccess<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    assembly: RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    proof: RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
}

/// Evidence-building state after a semantic rejection or before an explicit rich failure.
///
/// This wrapper intentionally exposes no canonical retain, share, or commitment method.
#[must_use = "failure evidence must be validated, downgraded, or deliberately dropped"]
pub(crate) struct RetainedFailureAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    assembly: RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    fallback_cause: PreparationFailureCode,
}

/// Exact rich-failure closure whose owning assembly can no longer be mutated.
#[must_use = "validated retained failure must be prepared or deliberately dropped"]
pub(crate) struct ValidatedRetainedFailure<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    assembly: RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    proof: RetainedEvidenceClosureProof<EvidenceBlock>,
}

/// Terminal-only owner produced by an invalid root, poisoned graph, or contained validator panic.
#[must_use = "fallback-only retained state must be prepared or deliberately dropped"]
pub(crate) struct RetainedFallbackOnly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    assembly: RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    cause: PreparationFailureCode,
}

/// Linear result of validating a success candidate.
#[must_use = "every validation outcome retains the complete publication owner"]
pub(crate) enum RetainedSuccessValidation<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    Valid(ValidatedRetainedSuccess<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    Rejected(RetainedFailureAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    FallbackOnly(RetainedFallbackOnly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
}

/// Linear result of validating a rich-failure candidate.
#[must_use = "every validation outcome retains the complete publication owner"]
pub(crate) enum RetainedFailureValidation<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    Valid(ValidatedRetainedFailure<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    Rejected(RetainedFailureAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    FallbackOnly(RetainedFallbackOnly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
}

struct QualifiedRetainedBlockTableStoragePlan<P, D, Block> {
    phase: P,
    attribution_dimension: D,
    total_blocks: usize,
    max_new_blocks: usize,
    member_capacity: CapacityPlan,
    charge: AdmissionCharge,
    block: PhantomData<fn() -> Block>,
}

struct QualifiedRetainedBlockTablePlan<O, P, D, Destination, Block>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    source: RetainedBlockTablePlan<P, D>,
    batch: AttributionBatchPlan<O, P, D, Destination>,
    storage: QualifiedRetainedBlockTableStoragePlan<P, D, Block>,
}

fn try_qualify_retained_block_table_plan<O, P, D, Destination, Block>(
    account: &PhysicalStorageAccount,
    source: RetainedBlockTablePlan<P, D>,
    terminal_limits: RetainedBatchLimits,
) -> Result<QualifiedRetainedBlockTablePlan<O, P, D, Destination, Block>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    let required_attributions = source
        .max_new_blocks
        .checked_add(1)
        .ok_or(RetainedWriterError::InvalidPublicationPlan)?;
    if source.total_blocks == 0
        || source.max_new_blocks > source.total_blocks
        || u64::try_from(source.total_blocks).is_err()
        || required_attributions > source.writer.limits.attribution_slots
        || required_attributions > terminal_limits.attribution_slots
        || source.writer.limits.attribution_slots > terminal_limits.attribution_slots
        || source.writer.limits.commitment_slots > terminal_limits.commitment_slots
    {
        return Err(RetainedWriterError::InvalidPublicationPlan);
    }
    let batch = AttributionBatchPlan::try_new(
        source.writer.phase,
        source.writer.capacity_dimension,
        source.writer.limits.attribution_slots,
        source.writer.limits.commitment_slots,
    )
    .map_err(map_batch_error)?;
    let member_capacity =
        CapacityPlan::for_type::<RetainedBlockMember<Block>>(0, 0, source.total_blocks)
            .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let nested_bytes = member_capacity
        .envelope_delta_bytes()
        .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let allocation = RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
        .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let charge = account
        .checked_retained_table_charge::<Block>(allocation)
        .map_err(RetainedWriterError::Physical)?;
    Ok(QualifiedRetainedBlockTablePlan {
        source,
        batch,
        storage: QualifiedRetainedBlockTableStoragePlan {
            phase: source.writer.phase,
            attribution_dimension: source.writer.attribution_dimension,
            total_blocks: source.total_blocks,
            max_new_blocks: source.max_new_blocks,
            member_capacity,
            charge,
            block: PhantomData,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn try_create_retained_block_table<O, P, D, Destination, Block>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    batch: &mut AttributionReservationBatch<O, P, D, Destination>,
    plan: QualifiedRetainedBlockTableStoragePlan<P, D, Block>,
    set_id: RetainedSetId,
    membership_destination: RetainedMembershipDestination,
) -> Result<RetainedTableAllocation<Block>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Destination: PublicationDestination,
    Block: Send + Sync + 'static,
{
    let QualifiedRetainedBlockTableStoragePlan {
        phase,
        attribution_dimension,
        total_blocks,
        max_new_blocks,
        member_capacity,
        charge,
        block: _,
    } = plan;
    let preflight = account
        .try_begin_account_preflight()
        .map_err(RetainedWriterError::AccountPoisoned)?;
    if !batch.has_attribution_slot() {
        return Err(RetainedWriterError::AttributionBatchFull);
    }
    let account_admission = preflight
        .admit_action()
        .map_err(map_account_action_admission_error)?;
    let admitted_share_work =
        try_admit_retained_share_work_before_physical(ledger, account, handle_share_work)?;
    let attribution_result = catch_unwind(AssertUnwindSafe(|| {
        ledger.reserve_operation_attribution::<Destination>(
            phase,
            attribution_dimension,
            charge.bytes,
        )
    }));
    let attribution = match attribution_result {
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            return Err(RetainedWriterError::AccountPoisoned(poisoned));
        }
        Ok(Ok(attribution)) => attribution,
        Ok(Err(cause)) => return Err(RetainedWriterError::Ledger(cause)),
    };
    let mut physical_admission = match account.try_admit_retained_after_action(
        account_admission,
        charge,
        admitted_share_work,
    ) {
        Ok(admission) => Some(admission),
        Err(cause) => {
            drop(attribution);
            return Err(RetainedWriterError::Physical(cause));
        }
    };
    let mut table = match account
        .try_retain_with_admitted_charge::<RetainedBlockTable<Block>, TableRetainedPolicy>(
            match physical_admission.as_mut() {
                Some(admission) => admission,
                None => unreachable_final_owner_state(),
            },
            || {
                let mut members = Vec::new();
                if members
                    .try_reserve_exact(member_capacity.allocator_additional())
                    .is_err()
                {
                    return Err(RetainedBuildFailureCode::new(1));
                }
                if member_capacity
                    .finish(members.len(), members.capacity())
                    .is_err()
                {
                    return Err(RetainedBuildFailureCode::new(2));
                }
                Ok(RetainedBlockTable {
                    qualified_capacity: members.capacity(),
                    members,
                    total_blocks,
                    max_new_blocks,
                    new_blocks: 0,
                })
            },
        ) {
        Ok(table) => table,
        Err(cause) => {
            if cause.charge_retained() {
                std::mem::forget(attribution);
            } else {
                drop(attribution);
            }
            return Err(RetainedWriterError::Physical(cause));
        }
    };
    let membership = RetainedMembership {
        set_id,
        destination: membership_destination,
        ordinal: 0,
    };
    if table.assign_membership(membership).is_err() {
        let outcome = drop_retained_allocation_before_attribution(table, account);
        if matches!(
            outcome,
            RetainedStorageDropOutcome::Clean | RetainedStorageDropOutcome::MissingFaultAnchor
        ) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        return Err(finish_pre_attribution_cleanup(
            attribution,
            outcome,
            RetainedWriterError::OwnershipUnverified,
        ));
    }
    if let Err(attribution) = batch.try_push_attribution(attribution) {
        let outcome = drop_retained_allocation_before_attribution(table, account);
        if matches!(
            outcome,
            RetainedStorageDropOutcome::Clean | RetainedStorageDropOutcome::MissingFaultAnchor
        ) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        return Err(finish_pre_attribution_cleanup(
            attribution,
            outcome,
            RetainedWriterError::OwnershipUnverified,
        ));
    }
    Ok(table)
}

#[allow(clippy::too_many_arguments)]
fn try_retain_block<O, P, D, Destination, Block, Tag>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    phase: P,
    attribution_dimension: D,
    batch: &mut AttributionReservationBatch<O, P, D, Destination>,
    table: &mut RetainedTableAllocation<Block>,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
    slot_limit_error: RetainedPublicationError<D>,
    plan: RetainedAllocationPlan,
    build: impl for<'a> FnOnce(
        BlockEmplacer<'a, Block>,
    ) -> Result<BlockPlacement<'a, Block, Tag>, RetainedBuildFailureCode>,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Destination: PublicationDestination,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    let preflight = account
        .try_begin_account_preflight()
        .map_err(RetainedWriterError::AccountPoisoned)
        .map_err(RetainedPublicationError::Writer)?;
    if !table.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount,
        ));
    }
    let (index, next_new_blocks) = {
        let table = table.as_ref();
        if table.members.len() >= table.total_blocks || table.new_blocks >= table.max_new_blocks {
            return Err(slot_limit_error);
        }
        if table.members.len() >= table.members.capacity() {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::BatchCapacity,
            ));
        }
        let next_new_blocks =
            table
                .new_blocks
                .checked_add(1)
                .ok_or(RetainedPublicationError::Writer(
                    RetainedWriterError::MembershipExhausted,
                ))?;
        (table.members.len(), next_new_blocks)
    };
    let ordinal = u64::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::MembershipExhausted,
        ))?;
    if !batch.has_attribution_slot() {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AttributionBatchFull,
        ));
    }
    let charge = account
        .checked_retained_charge::<Block>(plan)
        .map_err(|cause| RetainedPublicationError::Writer(RetainedWriterError::Physical(cause)))?;
    let account_admission = preflight
        .admit_action()
        .map_err(map_account_action_admission_error)
        .map_err(RetainedPublicationError::Writer)?;
    let Some(table) = table.table_mut_during_construction() else {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(poisoned),
        ));
    };
    let admitted_share_work =
        try_admit_retained_share_work_before_physical(ledger, account, handle_share_work)
            .map_err(RetainedPublicationError::Writer)?;
    let attribution_result = catch_unwind(AssertUnwindSafe(|| {
        ledger.reserve_operation_attribution::<Destination>(
            phase,
            attribution_dimension,
            charge.bytes,
        )
    }));
    let attribution = match attribution_result {
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ));
        }
        Ok(Ok(attribution)) => attribution,
        Ok(Err(cause)) => {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Ledger(cause),
            ));
        }
    };
    let mut physical_admission = match account.try_admit_retained_after_action(
        account_admission,
        charge,
        admitted_share_work,
    ) {
        Ok(admission) => Some(admission),
        Err(cause) => {
            drop(attribution);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Physical(cause),
            ));
        }
    };
    let mut allocation = match account.try_retain_with_admitted_handle(
        match physical_admission.as_mut() {
            Some(admission) => admission,
            None => unreachable_final_owner_state(),
        },
        plan,
        || {
            build(BlockEmplacer {
                marker: PhantomData,
            })
            .map(BlockPlacement::into_block)
        },
    ) {
        Ok(allocation) => allocation,
        Err(cause) => {
            if cause.charge_retained() {
                std::mem::forget(attribution);
            } else {
                drop(attribution);
            }
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Physical(cause),
            ));
        }
    };
    let membership = RetainedMembership {
        set_id,
        destination,
        ordinal,
    };
    if allocation.assign_membership(membership).is_err() {
        let outcome = drop_retained_allocation_before_attribution(allocation, account);
        if matches!(
            outcome,
            RetainedStorageDropOutcome::Clean | RetainedStorageDropOutcome::MissingFaultAnchor
        ) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        return Err(RetainedPublicationError::Writer(
            finish_pre_attribution_cleanup(
                attribution,
                outcome,
                RetainedWriterError::OwnershipUnverified,
            ),
        ));
    }
    if let Err(attribution) = batch.try_push_attribution(attribution) {
        let outcome = drop_retained_allocation_before_attribution(allocation, account);
        if matches!(
            outcome,
            RetainedStorageDropOutcome::Clean | RetainedStorageDropOutcome::MissingFaultAnchor
        ) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        return Err(RetainedPublicationError::Writer(
            finish_pre_attribution_cleanup(
                attribution,
                outcome,
                RetainedWriterError::OwnershipUnverified,
            ),
        ));
    };
    table.members.push(RetainedBlockMember {
        allocation,
        tag: TypeId::of::<Tag>(),
        origin: RetainedBlockOrigin::New,
        reachable: AtomicBool::new(false),
    });
    table.new_blocks = next_new_blocks;
    Ok(RetainedSlotIdentity {
        set_id,
        index,
        tag: TypeId::of::<Tag>(),
    })
}

#[allow(clippy::too_many_arguments)]
fn try_share_retained_block<O, P, D, Block, Tag>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    table: &mut RetainedTableAllocation<Block>,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
    slot_limit_error: RetainedPublicationError<D>,
    source: &PublishedRetainedAggregate<Block>,
    source_identity: RetainedSlotIdentity,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    let preflight = account
        .try_begin_account_preflight()
        .map_err(RetainedWriterError::AccountPoisoned)
        .map_err(RetainedPublicationError::Writer)?;
    if !table.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount,
        ));
    }
    let table_value = table.as_ref();
    if table_value.members.len() >= table_value.total_blocks
        || table_value.members.len() >= table_value.members.capacity()
    {
        return Err(slot_limit_error);
    }
    let index = table_value.members.len();
    let ordinal = u64::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::MembershipExhausted,
        ))?;
    if source_identity.tag != TypeId::of::<Tag>() || source_identity.set_id != source.set_id {
        return Err(RetainedPublicationError::InvalidSourceSlot);
    }
    if !source.table.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount,
        ));
    }
    let source_allocation = source
        .table
        .as_ref()
        .member_allocation(source_identity, source.set_id)
        .ok_or(RetainedPublicationError::InvalidSourceSlot)?;
    if !source_allocation.belongs_to(account) {
        drop(preflight);
        let cause = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(cause),
        ));
    }
    let share_admission = preflight
        .admit_action()
        .map_err(map_account_action_admission_error)
        .map_err(RetainedPublicationError::Writer)?;
    let Some(table) = table.table_mut_during_construction() else {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(poisoned),
        ));
    };
    match catch_unwind(AssertUnwindSafe(|| {
        try_share_retained_block_after_admission::<O, P, D, Block, Tag>(
            ledger,
            share_admission,
            handle_share_work,
            table,
            set_id,
            destination,
            index,
            ordinal,
            source_allocation,
        )
    })) {
        Ok(result) => result,
        Err(payload) => {
            let cause = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(cause),
            ))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn try_share_retained_block_after_admission<O, P, D, Block, Tag>(
    ledger: &Ledger<O, P, D>,
    share_admission: PendingAccountActionAdmission<'_>,
    handle_share_work: RetainedHandleSharePlan<D>,
    table: &mut RetainedBlockTable<Block>,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
    index: usize,
    ordinal: u64,
    source_allocation: &RetainedAllocation<Block>,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    let account = share_admission.account;
    let admitted_share_work = try_admit_retained_share_work(ledger, handle_share_work)
        .map_err(RetainedPublicationError::Writer)?;
    let mut allocation =
        share_admission.run(|_| admitted_share_work.consume_allocation(source_allocation));
    let membership = RetainedMembership {
        set_id,
        destination,
        ordinal,
    };
    if allocation.assign_membership(membership).is_err() {
        let outcome = allocation.release_storage();
        return Err(RetainedPublicationError::Writer(
            map_inherited_cleanup_error(account, outcome, RetainedWriterError::OwnershipUnverified),
        ));
    }
    table.members.push(RetainedBlockMember {
        allocation,
        tag: TypeId::of::<Tag>(),
        origin: RetainedBlockOrigin::Inherited,
        reachable: AtomicBool::new(false),
    });
    Ok(RetainedSlotIdentity {
        set_id,
        index,
        tag: TypeId::of::<Tag>(),
    })
}

fn retained_block_table_is_exact<O, P, D, Destination, Block>(
    table: &RetainedTableAllocation<Block>,
    batch: &AttributionReservationBatch<O, P, D, Destination>,
    account: &PhysicalStorageAccount,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
) -> bool
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if !table.belongs_to(account)
        || table.membership()
            != Some(RetainedMembership {
                set_id,
                destination,
                ordinal: 0,
            })
    {
        return false;
    }
    let mut attributed_bytes = table.charged_bytes();
    let table = table.as_ref();
    if table.members.capacity() != table.qualified_capacity
        || table.members.len() > table.total_blocks
        || table.new_blocks > table.max_new_blocks
        || table.members.len() > table.members.capacity()
    {
        return false;
    }
    let mut new_blocks = 0_usize;
    for (index, member) in table.members.iter().enumerate() {
        let Some(ordinal) = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
        else {
            return false;
        };
        if !member.allocation.belongs_to(account)
            || member.allocation.membership()
                != Some(RetainedMembership {
                    set_id,
                    destination,
                    ordinal,
                })
        {
            return false;
        }
        if member.origin == RetainedBlockOrigin::New {
            let Some(next_new_blocks) = new_blocks.checked_add(1) else {
                return false;
            };
            let Some(next_bytes) = attributed_bytes.checked_add(member.allocation.charged_bytes())
            else {
                return false;
            };
            new_blocks = next_new_blocks;
            attributed_bytes = next_bytes;
        }
    }
    let Some(attributed) = new_blocks.checked_add(1) else {
        return false;
    };
    new_blocks == table.new_blocks
        && attributed == batch.attribution_len()
        && batch.attributed_bytes() == Some(attributed_bytes)
}

/// Linear owner for one coordinator, one physical account, and its pre-admitted fallback.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[must_use = "the operation owns its coordinator and pre-admitted terminal fallback"]
pub(crate) struct RetainedOperation<O, P, PP, U, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    coordinator: OperationCoordinator<O, P, PP, U, D>,
    baseline: FallbackPublicationBaseline<O, P, D, F>,
    terminal_writer: RetainedWriterPlan<P, D>,
    terminal_account_share_work: RetainedHandleSharePlan<D>,
}

#[allow(
    dead_code,
    clippy::result_large_err,
    clippy::type_complexity,
    reason = "operation startup preserves every admitted owner on rejection without allocating"
)]
impl<O, P, PP, U, D, F> RetainedOperation<O, P, PP, U, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    pub fn contains_dimension(&self, dimension: D) -> bool {
        self.coordinator
            .retained_ledger()
            .contains_dimension(dimension)
    }

    pub fn contains_phase(&self, phase: P) -> bool {
        self.coordinator.retained_ledger().contains_phase(phase)
    }

    pub fn try_start(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        Self::try_start_inner(control, ledger_limits, None, account, plan, build_fallback)
    }

    pub fn try_start_with_phase_catalog(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        phase_catalog: &[P],
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        Self::try_start_inner(
            control,
            ledger_limits,
            Some(phase_catalog),
            account,
            plan,
            build_fallback,
        )
    }

    fn try_start_inner(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        phase_catalog: Option<&[P]>,
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        if plan.terminal_writer.limits.attribution_slots == 0
            || plan.terminal_writer.limits.commitment_slots == 0
        {
            return Err(RetainedOperationStartError::InvalidTerminalLimits);
        }
        let evidence_assembly = RetainedBatchLimits::new(
            plan.terminal_writer
                .limits
                .attribution_slots
                .checked_add(1)
                .ok_or(RetainedOperationStartError::InvalidTerminalLimits)?
                .max(2),
            plan.terminal_writer
                .limits
                .commitment_slots
                .checked_add(1)
                .ok_or(RetainedOperationStartError::InvalidTerminalLimits)?
                .max(2),
        );
        let canonical_assembly = plan.terminal_writer.limits;
        let terminal_slot_plan = RetainedWriterPlan::new(
            plan.terminal_writer.phase,
            plan.terminal_writer.capacity_dimension,
            plan.terminal_writer.attribution_dimension,
            RetainedBatchLimits::new(1, 1),
        );
        let fallback_batch_plan =
            AttributionBatchPlan::<O, P, D, OutcomeEvidenceDestination>::try_new(
                terminal_slot_plan.phase,
                terminal_slot_plan.capacity_dimension,
                terminal_slot_plan.limits.attribution_slots,
                terminal_slot_plan.limits.commitment_slots,
            )
            .map_err(map_batch_error)?;
        let evidence_batch_plan =
            AttributionBatchPlan::<O, P, D, OutcomeEvidenceDestination>::try_new(
                terminal_slot_plan.phase,
                terminal_slot_plan.capacity_dimension,
                terminal_slot_plan.limits.attribution_slots,
                terminal_slot_plan.limits.commitment_slots,
            )
            .map_err(map_batch_error)?;
        let canonical_assembly_plan = AttributionBatchPlan::<O, P, D, CanonicalOutput>::try_new(
            plan.terminal_writer.phase,
            plan.terminal_writer.capacity_dimension,
            canonical_assembly.attribution_slots,
            canonical_assembly.commitment_slots,
        )
        .map_err(map_batch_error)?;
        let evidence_assembly_plan =
            AttributionBatchPlan::<O, P, D, OutcomeEvidenceDestination>::try_new(
                plan.terminal_writer.phase,
                plan.terminal_writer.capacity_dimension,
                evidence_assembly.attribution_slots,
                evidence_assembly.commitment_slots,
            )
            .map_err(map_batch_error)?;
        let dimension_count = ledger_limits.dimension_count();
        let evidence_capacity =
            CapacityPlan::for_type::<PublicationDimensionCell<D>>(0, 0, dimension_count).map_err(
                |_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity),
            )?;
        let nested_bytes = evidence_capacity
            .envelope_delta_bytes()
            .map_err(|_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity))?;
        let evidence_plan = RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
            .map_err(|_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity))?;
        let fallback_charge = account
            .checked_retained_charge::<F>(plan.fallback_plan)
            .map_err(RetainedWriterError::Physical)?;
        let evidence_charge = account
            .checked_retained_charge::<PublicationEvidenceCells<D>>(evidence_plan)
            .map_err(RetainedWriterError::Physical)?;
        let pair_plan = RetainedAdmissionPairPlan::try_new(fallback_charge, evidence_charge)
            .map_err(RetainedWriterError::Physical)?;
        let startup_plan = ledger_limits
            .try_qualify_startup(
                StartupAttributionBatchPlans::new(
                    canonical_assembly_plan,
                    evidence_assembly_plan,
                    fallback_batch_plan,
                    evidence_batch_plan,
                ),
                [
                    StartupRetainedAttribution::new(
                        terminal_slot_plan.phase,
                        terminal_slot_plan.attribution_dimension,
                        fallback_charge.bytes,
                    ),
                    StartupRetainedAttribution::new(
                        terminal_slot_plan.phase,
                        terminal_slot_plan.attribution_dimension,
                        evidence_charge.bytes,
                    ),
                ],
                StartupAccountShareEnvelope::four_identical(
                    plan.terminal_account_share_work.charges(),
                ),
                [plan.fallback_commitment, plan.snapshot_commitment],
            )
            .map_err(|cause| {
                RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
            })?;
        let (ledger_limits, startup_parts) = startup_plan.into_parts();
        let (
            canonical_assembly_plan,
            evidence_assembly_plan,
            fallback_batch_plan,
            evidence_batch_plan,
        ) = startup_parts.batches.into_parts();
        let [fallback_attribution, evidence_attribution] = startup_parts.retained_attributions;
        let account_shares = startup_parts.account_shares;
        let [fallback_commitment, snapshot_commitment] = startup_parts.commitments;

        let account_admission = account
            .try_admit_account_action()
            .map_err(map_account_action_admission_error)
            .map_err(RetainedOperationStartError::Writer)?;
        let coordinator = match phase_catalog {
            Some(phases) => {
                match OperationCoordinator::try_new_with_phase_catalog(
                    control,
                    ledger_limits,
                    phases,
                ) {
                    Ok(coordinator) => coordinator,
                    Err(cause) => {
                        account_admission.release();
                        return Err(RetainedOperationStartError::Writer(
                            RetainedWriterError::Ledger(cause),
                        ));
                    }
                }
            }
            None => OperationCoordinator::new(control, ledger_limits),
        };
        let ledger = coordinator.retained_ledger();
        let startup_owners = try_admit_start_share_work(ledger, account, account_shares)
            .map_err(RetainedOperationStartError::Writer)?;
        let (start_admissions, startup_owners) = account
            .try_admit_start_retained_pair_after_action(
                account_admission,
                pair_plan,
                startup_owners,
            )
            .map_err(RetainedWriterError::Physical)?;
        let start_guard = StartupAdmissionGuard::new(account, start_admissions, startup_owners);

        let canonical_assembly =
            match ledger.try_attribution_batch_from_plan(canonical_assembly_plan) {
                Ok(batch) => batch,
                Err(cause) => {
                    return Err(RetainedOperationStartError::Writer(
                        map_qualified_startup_batch_error(cause, |outer_reason| {
                            start_guard.rollback(outer_reason)
                        }),
                    ));
                }
            };
        let evidence_assembly = match ledger.try_attribution_batch_from_plan(evidence_assembly_plan)
        {
            Ok(batch) => batch,
            Err(cause) => {
                return Err(RetainedOperationStartError::Writer(
                    map_qualified_startup_batch_error(cause, |outer_reason| {
                        start_guard.rollback(outer_reason)
                    }),
                ));
            }
        };

        let startup_guard =
            StartupRetainedPartGuard::new(start_guard, canonical_assembly, evidence_assembly);
        let startup_owner_result = startup_guard.try_build(
            ledger,
            StartupRetainedPartPlan::new(
                fallback_attribution,
                fallback_batch_plan,
                plan.fallback_plan,
                fallback_commitment,
            ),
            build_fallback,
            StartupRetainedPartPlan::new(
                evidence_attribution,
                evidence_batch_plan,
                evidence_plan,
                snapshot_commitment,
            ),
            || {
                #[cfg(test)]
                {
                    record_start_evidence_build_call();
                    if take_injected_start_evidence_build_panic() {
                        panic!("inject startup evidence builder panic");
                    }
                    if let Some(cause) = take_injected_start_evidence_build_failure() {
                        return Err(cause);
                    }
                }
                let mut dimensions = Vec::new();
                if dimensions
                    .try_reserve_exact(evidence_capacity.allocator_additional())
                    .is_err()
                {
                    return Err(RetainedBuildFailureCode::new(1));
                }
                if evidence_capacity
                    .finish(dimensions.len(), dimensions.capacity())
                    .is_err()
                {
                    return Err(RetainedBuildFailureCode::new(2));
                }
                for index in 0..dimension_count {
                    let Some(dimension) = ledger.dimension_at(index) else {
                        return Err(RetainedBuildFailureCode::new(3));
                    };
                    dimensions.push(PublicationDimensionCell {
                        dimension,
                        attempted: AtomicU64::new(0),
                        consumed: AtomicU64::new(0),
                        peak_live_bytes: AtomicU64::new(0),
                        committed_canonical_output: AtomicU64::new(0),
                        published_outcome_evidence: AtomicU64::new(0),
                    });
                }
                Ok(PublicationEvidenceCells {
                    dimensions,
                    frozen: AtomicBool::new(false),
                })
            },
        );
        #[cfg(test)]
        if startup_owner_result.is_err() {
            let probe = ledger.test_probe();
            STARTUP_FAILURE_LEDGER_SNAPSHOT.with(|snapshot| {
                snapshot.set(Some((
                    probe.active_reservations(),
                    probe.current_live_bytes(),
                )));
            });
        }
        let startup_owner = startup_owner_result.map_err(RetainedOperationStartError::Writer)?;
        let baseline = startup_owner
            .try_into_baseline()
            .map_err(RetainedOperationStartError::Writer)?;
        let baseline = baseline
            .try_bind(ledger)
            .map_err(RetainedOperationStartError::Writer)?;

        Ok(Self {
            coordinator,
            baseline,
            terminal_writer: plan.terminal_writer,
            terminal_account_share_work: plan.terminal_account_share_work,
        })
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        self.baseline.account()
    }

    /// Freezes the pre-admitted terminal baseline without opening a block-table assembly.
    ///
    /// This path performs no allocation, table construction, root validation, control callback, or
    /// other fallible action. It is therefore available after every recoverable assembly-open
    /// failure. Fail-closed open errors deliberately do not return the operation and cannot reach
    /// this method.
    #[must_use = "the prepared fallback must be handed off or deliberately dropped"]
    pub fn prepare_minimal_fallback<CanonicalBlock, EvidenceBlock>(
        self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        let RetainedOperation {
            coordinator,
            baseline,
            ..
        } = self;
        let ledger = coordinator.into_ledger_for_typed_publication();
        let (fallback, evidence, batches) = baseline.into_parts();
        let physical = TerminalPhysicalOwnershipManifest::new(fallback, evidence, None, None, None);
        TerminalFallbackGuard::new(
            TerminalFallbackAttribution {
                ledger,
                baseline: batches,
                discard: PrimaryPublicationBatches {
                    canonical: None,
                    evidence: None,
                },
            },
            physical,
        )
        .try_prepare(cause)
    }

    pub fn try_publication_assembly<CanonicalBlock, EvidenceBlock>(
        self,
        plan: RetainedPublicationPlan<P, D>,
    ) -> Result<
        RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationOpenError<Self, D>,
    >
    where
        CanonicalBlock: Send + Sync + 'static,
        EvidenceBlock: Send + Sync + 'static,
    {
        let terminal_limits = self.terminal_writer.limits;
        let evidence_qualified = match try_qualify_retained_block_table_plan::<
            O,
            P,
            D,
            OutcomeEvidenceDestination,
            EvidenceBlock,
        >(
            self.account(), plan.outcome_evidence, terminal_limits
        ) {
            Ok(qualified) => qualified,
            Err(cause) => {
                return Err(RetainedPublicationOpenError {
                    operation: self,
                    cause,
                    recoverable: true,
                });
            }
        };
        let canonical_qualified = match plan.canonical {
            Some(canonical) => match try_qualify_retained_block_table_plan::<
                O,
                P,
                D,
                CanonicalOutput,
                CanonicalBlock,
            >(self.account(), canonical, terminal_limits)
            {
                Ok(qualified) => Some(qualified),
                Err(cause) => {
                    return Err(RetainedPublicationOpenError {
                        operation: self,
                        cause,
                        recoverable: true,
                    });
                }
            },
            None => None,
        };
        let handle_share_work = self.terminal_account_share_work;
        let open_result = catch_unwind(AssertUnwindSafe(|| {
            try_open_publication_account(
                self.coordinator.retained_ledger(),
                self.account(),
                handle_share_work,
            )
        }));
        let (account, set_id) = match open_result {
            Err(payload) => {
                let cause = RetainedWriterError::AccountPoisoned(
                    self.account()
                        .record_poison(AccountPoisonReason::OwnershipGraphUnverified),
                );
                discard_untrusted_panic_payload(payload);
                return Err(RetainedPublicationOpenError {
                    operation: self,
                    cause,
                    recoverable: false,
                });
            }
            Ok(Err(cause)) => {
                let recoverable = retained_open_failure_is_recoverable(&cause);
                return Err(RetainedPublicationOpenError {
                    operation: self,
                    cause,
                    recoverable,
                });
            }
            Ok(Ok(open)) => open,
        };
        let (canonical_plan, mut canonical_batch, canonical_storage) =
            if let Some(qualified) = canonical_qualified {
                let QualifiedRetainedBlockTablePlan {
                    source,
                    batch: batch_plan,
                    storage,
                } = qualified;
                let batch = match self
                    .coordinator
                    .retained_ledger()
                    .try_attribution_batch_from_plan(batch_plan)
                {
                    Ok(batch) => batch,
                    Err(error) => {
                        drop(account);
                        return Err(RetainedPublicationOpenError {
                            operation: self,
                            cause: map_batch_error(error),
                            recoverable: true,
                        });
                    }
                };
                (Some(source), Some(batch), Some(storage))
            } else {
                (None, None, None)
            };
        let QualifiedRetainedBlockTablePlan {
            source: evidence_plan,
            batch: evidence_batch_plan,
            storage: evidence_storage,
        } = evidence_qualified;
        let mut evidence_batch = match self
            .coordinator
            .retained_ledger()
            .try_attribution_batch_from_plan(evidence_batch_plan)
        {
            Ok(batch) => batch,
            Err(error) => {
                drop(canonical_batch);
                drop(account);
                return Err(RetainedPublicationOpenError {
                    operation: self,
                    cause: map_batch_error(error),
                    recoverable: true,
                });
            }
        };
        let canonical_table = if let Some(storage) = canonical_storage {
            let batch = match canonical_batch.as_mut() {
                Some(batch) => batch,
                None => unreachable_final_owner_state(),
            };
            let table = match try_create_retained_block_table(
                self.coordinator.retained_ledger(),
                &account,
                handle_share_work,
                batch,
                storage,
                set_id,
                RetainedMembershipDestination::Canonical,
            ) {
                Ok(batch) => batch,
                Err(cause) => {
                    drop(evidence_batch);
                    drop(canonical_batch);
                    drop(account);
                    return Err(RetainedPublicationOpenError {
                        operation: self,
                        recoverable: retained_open_failure_is_recoverable(&cause),
                        cause,
                    });
                }
            };
            Some(table)
        } else {
            None
        };
        let evidence_table = match try_create_retained_block_table(
            self.coordinator.retained_ledger(),
            &account,
            handle_share_work,
            &mut evidence_batch,
            evidence_storage,
            set_id,
            RetainedMembershipDestination::OutcomeEvidence,
        ) {
            Ok(table) => table,
            Err(cause) => {
                let canonical_cleanup = match canonical_table {
                    Some(table) => drop_retained_table_with_account_anchor(table, &account),
                    None => RetainedStorageDropOutcome::Clean,
                };
                let cause =
                    finish_pre_attribution_batch_cleanup(canonical_batch, canonical_cleanup, cause);
                drop(evidence_batch);
                drop(account);
                return Err(RetainedPublicationOpenError {
                    operation: self,
                    recoverable: retained_open_failure_is_recoverable(&cause),
                    cause,
                });
            }
        };
        Ok(RetainedPublicationAssembly {
            canonical_table,
            evidence_table: Some(evidence_table),
            canonical_batch,
            evidence_batch: Some(evidence_batch),
            account: Some(account),
            operation: Some(self),
            set_id,
            canonical_plan,
            evidence_plan,
            handle_share_work,
            mode: if canonical_plan.is_some() {
                RetainedOutcomeMode::SuccessCapable
            } else {
                RetainedOutcomeMode::FailureOnly
            },
            containment: AssemblyContainmentProvenance::Healthy,
        })
    }
}

#[allow(clippy::result_large_err)]
impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    /// Validates the exact canonical and outcome-evidence closure without transferring ownership.
    ///
    /// The callback must visit every unique member. Repeated DAG edges are accepted. A callback
    /// panic is contained, returns [`RetainedClosureValidationError::Panicked`], and resets the
    /// reachability marks so the consuming caller can publish the pre-admitted fallback.
    fn try_validate_success<CanonicalTag, EvidenceTag>(
        &mut self,
        canonical_root: CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock>,
            CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
        RetainedClosureValidationError,
    >
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedClosureValidationError::Poisoned);
        }
        if self.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedClosureValidationError::WrongOutcomeMode);
        }
        let (Some(canonical), Some(evidence)) =
            (self.canonical_table.as_ref(), self.evidence_table.as_ref())
        else {
            self.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedClosureValidationError::Poisoned);
        };
        let canonical = canonical.as_ref();
        let evidence = evidence.as_ref();
        canonical.reset_reachable();
        evidence.reset_reachable();
        if canonical_root.identity.tag != TypeId::of::<CanonicalTag>()
            || evidence_root.identity.tag != TypeId::of::<EvidenceTag>()
            || canonical
                .member(canonical_root.identity, self.set_id)
                .is_none()
            || evidence
                .member(evidence_root.identity, self.set_id)
                .is_none()
        {
            return Err(RetainedClosureValidationError::InvalidRoot);
        }
        let mut session = RetainedClosureSession {
            canonical,
            evidence,
            set_id: self.set_id,
            canonical_seen: 0,
            evidence_seen: 0,
            valid: true,
            not_send_or_sync: PhantomData,
        };
        let accepted = match catch_unwind(AssertUnwindSafe(|| {
            validate(&mut session, canonical_root, evidence_root)
        })) {
            Ok(accepted) => accepted,
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                canonical.reset_reachable();
                evidence.reset_reachable();
                return Err(RetainedClosureValidationError::Panicked);
            }
        };
        let exact = accepted
            && session.valid
            && session.canonical_seen == canonical.members.len()
            && session.evidence_seen == evidence.members.len()
            && canonical.all_reachable()
            && evidence.all_reachable();
        if !exact {
            canonical.reset_reachable();
            evidence.reset_reachable();
            return Err(RetainedClosureValidationError::Rejected);
        }
        Ok(RetainedSuccessClosureProof {
            set_id: self.set_id,
            canonical_count: canonical.members.len(),
            evidence_count: evidence.members.len(),
            canonical_root: canonical_root.identity,
            evidence_root: evidence_root.identity,
            marker: PhantomData,
        })
    }

    /// Validates the exact outcome-evidence closure without requiring canonical completeness.
    ///
    /// The evidence root must be outcome-neutral because success control rejection can reuse this
    /// graph as a rich failure. Callback panic containment has the same fallback semantics as
    /// [`Self::try_validate_success`].
    fn try_validate_rich_failure<EvidenceTag>(
        &mut self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedEvidenceClosureSession<'a, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> Result<RetainedEvidenceClosureProof<EvidenceBlock>, RetainedClosureValidationError>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedClosureValidationError::Poisoned);
        }
        let Some(evidence) = self.evidence_table.as_ref() else {
            self.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedClosureValidationError::Poisoned);
        };
        let evidence = evidence.as_ref();
        evidence.reset_reachable();
        if evidence_root.identity.tag != TypeId::of::<EvidenceTag>()
            || evidence
                .member(evidence_root.identity, self.set_id)
                .is_none()
        {
            return Err(RetainedClosureValidationError::InvalidRoot);
        }
        let mut session = RetainedEvidenceClosureSession {
            evidence,
            set_id: self.set_id,
            seen: 0,
            valid: true,
            not_send_or_sync: PhantomData,
        };
        let accepted =
            match catch_unwind(AssertUnwindSafe(|| validate(&mut session, evidence_root))) {
                Ok(accepted) => accepted,
                Err(payload) => {
                    discard_untrusted_panic_payload(payload);
                    evidence.reset_reachable();
                    return Err(RetainedClosureValidationError::Panicked);
                }
            };
        let exact = accepted
            && session.valid
            && session.seen == evidence.members.len()
            && evidence.all_reachable();
        if !exact {
            evidence.reset_reachable();
            return Err(RetainedClosureValidationError::Rejected);
        }
        Ok(RetainedEvidenceClosureProof {
            set_id: self.set_id,
            evidence_count: evidence.members.len(),
            evidence_root: evidence_root.identity,
            marker: PhantomData,
        })
    }

    /// Borrows the complete candidate graph for validation and consumes the writable assembly.
    ///
    /// A semantic rejection returns an evidence-only builder so callers may append bounded failure
    /// details and validate a rich failure. Invalid identity, poisoned state, and panic can only
    /// proceed to the pre-admitted minimal fallback. A valid result owns the private closure proof
    /// and exposes no mutation method, eliminating proof/mutation TOCTOU.
    pub fn validate_success<CanonicalTag, EvidenceTag>(
        mut self,
        canonical_root: CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock>,
            CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> RetainedSuccessValidation<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        match self.try_validate_success(canonical_root, evidence_root, validate) {
            Ok(proof) => RetainedSuccessValidation::Valid(ValidatedRetainedSuccess {
                assembly: self,
                proof,
            }),
            Err(RetainedClosureValidationError::Rejected) => {
                RetainedSuccessValidation::Rejected(RetainedFailureAssembly {
                    assembly: self,
                    fallback_cause: PreparationFailureCode::from_closure_validation(
                        RetainedClosureValidationError::Rejected,
                    ),
                })
            }
            Err(cause) => RetainedSuccessValidation::FallbackOnly(RetainedFallbackOnly {
                assembly: self,
                cause: PreparationFailureCode::from_closure_validation(cause),
            }),
        }
    }

    /// Borrows an evidence graph for validation and consumes the writable assembly.
    pub fn validate_rich_failure<EvidenceTag>(
        mut self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedEvidenceClosureSession<'a, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> RetainedFailureValidation<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        match self.try_validate_rich_failure(evidence_root, validate) {
            Ok(proof) => RetainedFailureValidation::Valid(ValidatedRetainedFailure {
                assembly: self,
                proof,
            }),
            Err(RetainedClosureValidationError::Rejected) => {
                RetainedFailureValidation::Rejected(RetainedFailureAssembly {
                    assembly: self,
                    fallback_cause: PreparationFailureCode::from_closure_validation(
                        RetainedClosureValidationError::Rejected,
                    ),
                })
            }
            Err(cause) => RetainedFailureValidation::FallbackOnly(RetainedFallbackOnly {
                assembly: self,
                cause: PreparationFailureCode::from_closure_validation(cause),
            }),
        }
    }

    pub fn canonical_len(&self) -> usize {
        self.canonical_table
            .as_ref()
            .map_or(0, |table| table.as_ref().members.len())
    }

    pub fn outcome_evidence_len(&self) -> usize {
        self.evidence_table
            .as_ref()
            .map_or(0, |table| table.as_ref().members.len())
    }

    pub fn contains_dimension(&self, dimension: D) -> bool {
        self.operation.as_ref().is_some_and(|operation| {
            operation
                .coordinator
                .retained_ledger()
                .contains_dimension(dimension)
        })
    }

    #[cfg(test)]
    pub(crate) fn test_ledger_probe(&self) -> crate::ledger::LedgerTestProbe<P, D> {
        self.operation
            .as_ref()
            .expect("test assembly retains its operation")
            .coordinator
            .retained_ledger()
            .test_probe()
    }

    pub fn try_retain_canonical<Tag>(
        &mut self,
        plan: RetainedAllocationPlan,
        build: impl for<'a> FnOnce(
            BlockEmplacer<'a, CanonicalBlock>,
        ) -> Result<
            BlockPlacement<'a, CanonicalBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<CanonicalRetainedSlot<CanonicalBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        let canonical_plan = self
            .canonical_plan
            .ok_or(RetainedPublicationError::WrongOutcomeMode)?;
        let result = try_retain_block(
            self.operation
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?
                .coordinator
                .retained_ledger(),
            self.account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.handle_share_work,
            canonical_plan.writer.phase,
            canonical_plan.writer.attribution_dimension,
            self.canonical_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.canonical_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.set_id,
            RetainedMembershipDestination::Canonical,
            RetainedPublicationError::CanonicalSlotLimit,
            plan,
            build,
        );
        match result {
            Ok(identity) => Ok(CanonicalRetainedSlot {
                identity,
                marker: PhantomData,
            }),
            Err(error) => {
                if let Some(containment) = retained_publication_error_containment(&error) {
                    self.containment = containment;
                }
                Err(error)
            }
        }
    }

    /// Inherits one immutable canonical block from a published same-account aggregate.
    ///
    /// The inherited payload keeps its original physical charge and receives no new operation
    /// attribution. Clone and retained-handle work are charged before the reference count changes,
    /// and the returned slot is freshly branded for this assembly.
    pub fn try_share_canonical<Tag>(
        &mut self,
        source: &PublishedCanonicalAggregate<CanonicalBlock>,
        source_slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
    ) -> Result<CanonicalRetainedSlot<CanonicalBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        if self.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedPublicationError::WrongOutcomeMode);
        }
        let result = try_share_retained_block::<O, P, D, CanonicalBlock, Tag>(
            self.operation
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?
                .coordinator
                .retained_ledger(),
            self.account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.handle_share_work,
            self.canonical_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.set_id,
            RetainedMembershipDestination::Canonical,
            RetainedPublicationError::CanonicalSlotLimit,
            &source.inner,
            source_slot.identity,
        );
        match result {
            Ok(identity) => Ok(CanonicalRetainedSlot {
                identity,
                marker: PhantomData,
            }),
            Err(error) => {
                if let Some(containment) = retained_publication_error_containment(&error) {
                    self.containment = containment;
                }
                Err(error)
            }
        }
    }

    pub fn try_retain_outcome_evidence<Tag>(
        &mut self,
        plan: RetainedAllocationPlan,
        build: impl for<'a> FnOnce(
            BlockEmplacer<'a, EvidenceBlock>,
        ) -> Result<
            BlockPlacement<'a, EvidenceBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        let result = try_retain_block(
            self.operation
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?
                .coordinator
                .retained_ledger(),
            self.account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.handle_share_work,
            self.evidence_plan.writer.phase,
            self.evidence_plan.writer.attribution_dimension,
            self.evidence_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.evidence_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.set_id,
            RetainedMembershipDestination::OutcomeEvidence,
            RetainedPublicationError::OutcomeEvidenceSlotLimit,
            plan,
            build,
        );
        match result {
            Ok(identity) => Ok(OutcomeEvidenceRetainedSlot {
                identity,
                marker: PhantomData,
            }),
            Err(error) => {
                if let Some(containment) = retained_publication_error_containment(&error) {
                    self.containment = containment;
                }
                Err(error)
            }
        }
    }

    /// Inherits one immutable outcome-evidence block from a published same-account aggregate.
    pub fn try_share_outcome_evidence<Tag>(
        &mut self,
        source: &PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
        source_slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Result<OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        let result = try_share_retained_block::<O, P, D, EvidenceBlock, Tag>(
            self.operation
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?
                .coordinator
                .retained_ledger(),
            self.account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.handle_share_work,
            self.evidence_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            self.set_id,
            RetainedMembershipDestination::OutcomeEvidence,
            RetainedPublicationError::OutcomeEvidenceSlotLimit,
            &source.inner,
            source_slot.identity,
        );
        match result {
            Ok(identity) => Ok(OutcomeEvidenceRetainedSlot {
                identity,
                marker: PhantomData,
            }),
            Err(error) => {
                if let Some(containment) = retained_publication_error_containment(&error) {
                    self.containment = containment;
                }
                Err(error)
            }
        }
    }

    pub fn try_commit_canonical(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), RetainedPublicationError<D>> {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        if self.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedPublicationError::WrongOutcomeMode);
        }
        self.canonical_batch
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .try_push_commitment(OutputCommitment::new(dimension, amount))
            .map_err(|_| RetainedPublicationError::Writer(RetainedWriterError::CommitmentBatchFull))
    }

    pub fn try_commit_outcome_evidence(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), RetainedPublicationError<D>> {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.evidence_batch
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .try_push_commitment(OutputCommitment::new(dimension, amount))
            .map_err(|_| RetainedPublicationError::Writer(RetainedWriterError::CommitmentBatchFull))
    }

    /// Discards every block table before attribution and publishes the pre-admitted fallback.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        self.try_prepare_terminal(RetainedTerminalRequest::MinimalFallback {
            requested_cause: PreparationFailureCode::requested(),
        })
    }

    fn try_prepare_terminal(
        mut self,
        request: RetainedTerminalRequest<CanonicalBlock, EvidenceBlock, PP, U>,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            let cause = match self.containment {
                AssemblyContainmentProvenance::NewFaultPending => {
                    self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified)
                }
                AssemblyContainmentProvenance::RetainedFailureAlreadyRecorded(reason) => {
                    self.contain_after_recorded_poison(reason)
                }
                AssemblyContainmentProvenance::Healthy => unreachable_final_owner_state(),
            };
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                cause,
            ));
        }
        let exact_storage = self.has_exact_storage_graph()
            && match (self.account.as_ref(), self.operation.as_ref()) {
                (Some(account), Some(operation)) => {
                    account.is_same_account(operation.baseline.account())
                }
                _ => false,
            };
        if !exact_storage {
            let cause = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                cause,
            ));
        }

        let terminal = match request {
            RetainedTerminalRequest::Success {
                proof,
                final_progress,
            } => {
                let proof_valid = self.mode == RetainedOutcomeMode::SuccessCapable
                    && proof.set_id == self.set_id
                    && self.canonical_table.as_ref().is_some_and(|table| {
                        table.as_ref().members.len() == proof.canonical_count
                            && table.as_ref().all_reachable()
                            && table
                                .as_ref()
                                .member(proof.canonical_root, self.set_id)
                                .is_some()
                    })
                    && self.evidence_table.as_ref().is_some_and(|table| {
                        table.as_ref().members.len() == proof.evidence_count
                            && table.as_ref().all_reachable()
                            && table
                                .as_ref()
                                .member(proof.evidence_root, self.set_id)
                                .is_some()
                    });
                if !proof_valid {
                    let cause =
                        self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                    return Err(RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ));
                }
                let control_cause = match self.operation.as_mut() {
                    Some(operation) => {
                        prepare_terminal_control(&mut operation.coordinator, final_progress)
                    }
                    None => {
                        let cause = self
                            .poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                        return Err(RetainedPublicationPrepareError::FinalizationContainment(
                            cause,
                        ));
                    }
                };
                control_cause.map_or(
                    RetainedTerminalMode::Success {
                        canonical_root: proof.canonical_root,
                        evidence_root: proof.evidence_root,
                    },
                    |cause| RetainedTerminalMode::RichFailure {
                        evidence_root: proof.evidence_root,
                        cause,
                    },
                )
            }
            RetainedTerminalRequest::RichFailure {
                proof,
                requested_cause,
                final_progress,
            } => {
                let proof_valid = proof.set_id == self.set_id
                    && self.evidence_table.as_ref().is_some_and(|table| {
                        table.as_ref().members.len() == proof.evidence_count
                            && table.as_ref().all_reachable()
                            && table
                                .as_ref()
                                .member(proof.evidence_root, self.set_id)
                                .is_some()
                    });
                if !proof_valid {
                    let cause =
                        self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                    return Err(RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ));
                }
                let control_cause = match self.operation.as_mut() {
                    Some(operation) => {
                        prepare_terminal_control(&mut operation.coordinator, final_progress)
                    }
                    None => {
                        let cause = self
                            .poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                        return Err(RetainedPublicationPrepareError::FinalizationContainment(
                            cause,
                        ));
                    }
                };
                RetainedTerminalMode::RichFailure {
                    evidence_root: proof.evidence_root,
                    cause: control_cause.unwrap_or(requested_cause),
                }
            }
            RetainedTerminalRequest::MinimalFallback { requested_cause } => {
                let cause = match self.operation.as_mut() {
                    Some(operation) => operation
                        .coordinator
                        .poll()
                        .err()
                        .map_or(requested_cause, |cause| {
                            PreparationFailureCode::from_control(&cause)
                        }),
                    None => {
                        let cause = self
                            .poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                        return Err(RetainedPublicationPrepareError::FinalizationContainment(
                            cause,
                        ));
                    }
                };
                RetainedTerminalMode::MinimalFallback { cause }
            }
        };

        match terminal {
            RetainedTerminalMode::Success { .. } => {}
            RetainedTerminalMode::RichFailure { .. } => {
                if let Some(canonical) = self.canonical_table.take() {
                    let outcome = drop_retained_table_before_attribution(
                        canonical,
                        match self.account.as_ref() {
                            Some(account) => account,
                            None => unreachable_final_owner_state(),
                        },
                    );
                    if outcome != RetainedStorageDropOutcome::Clean {
                        let reason = match outcome {
                            RetainedStorageDropOutcome::Clean => unreachable_final_owner_state(),
                            RetainedStorageDropOutcome::StorageReleaseUnverified(witness)
                            | RetainedStorageDropOutcome::ChargeReleaseUnverified(witness)
                            | RetainedStorageDropOutcome::StillSharedOwnershipFault(witness)
                            | RetainedStorageDropOutcome::ForeignOwnershipFault(witness) => {
                                witness.reason
                            }
                            RetainedStorageDropOutcome::MissingFaultAnchor => {
                                AccountPoisonReason::OwnershipGraphUnverified
                            }
                        };
                        let cause = if matches!(
                            outcome,
                            RetainedStorageDropOutcome::StorageReleaseUnverified(_)
                                | RetainedStorageDropOutcome::ChargeReleaseUnverified(_)
                                | RetainedStorageDropOutcome::StillSharedOwnershipFault(_)
                                | RetainedStorageDropOutcome::ForeignOwnershipFault(_)
                        ) {
                            self.contain_after_recorded_poison(reason)
                        } else {
                            self.poison_and_fail_closed(reason)
                        };
                        return Err(RetainedPublicationPrepareError::FinalizationContainment(
                            cause,
                        ));
                    }
                }
                drop(self.canonical_batch.take());
                self.mode = RetainedOutcomeMode::FailureOnly;
            }
            RetainedTerminalMode::MinimalFallback { .. } => {
                let outcome = self.drop_tables_before_attribution();
                if outcome != RetainedStorageDropOutcome::Clean {
                    let cause = match outcome {
                        RetainedStorageDropOutcome::Clean => unreachable_final_owner_state(),
                        RetainedStorageDropOutcome::StorageReleaseUnverified(witness)
                        | RetainedStorageDropOutcome::ChargeReleaseUnverified(witness)
                        | RetainedStorageDropOutcome::StillSharedOwnershipFault(witness)
                        | RetainedStorageDropOutcome::ForeignOwnershipFault(witness) => {
                            self.contain_after_recorded_poison(witness.reason)
                        }
                        RetainedStorageDropOutcome::MissingFaultAnchor => self
                            .poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified),
                    };
                    return Err(RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ));
                }
            }
        }

        if self.operation.is_none() || self.evidence_batch.is_none() || self.account.is_none() {
            let cause = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                cause,
            ));
        }
        let operation = match self.operation.take() {
            Some(operation) => operation,
            None => unreachable_final_owner_state(),
        };
        let evidence_batch = match self.evidence_batch.take() {
            Some(batch) => batch,
            None => unreachable_final_owner_state(),
        };
        let account = match self.account.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let canonical_table = self.canonical_table.take();
        let evidence_table = self.evidence_table.take();
        let canonical_batch = self.canonical_batch.take();
        let RetainedOperation {
            coordinator,
            baseline,
            ..
        } = operation;
        let ledger = coordinator.into_ledger_for_typed_publication();
        let (fallback, evidence, baseline_batches) = baseline.into_parts();
        let physical = TerminalPhysicalOwnershipManifest::new(
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            Some(account),
        );

        if let RetainedTerminalMode::MinimalFallback { cause } = terminal {
            let attribution = TerminalFallbackAttribution {
                ledger,
                baseline: baseline_batches,
                discard: PrimaryPublicationBatches {
                    canonical: canonical_batch,
                    evidence: Some(evidence_batch),
                },
            };
            return TerminalFallbackGuard::new(attribution, physical).try_prepare(cause);
        }

        let transition = TerminalPrimaryGuard::new(
            ledger,
            baseline_batches,
            PrimaryPublicationBatches {
                canonical: canonical_batch,
                evidence: Some(evidence_batch),
            },
            physical,
        )
        .try_advance(terminal)?;
        let freeze_guard = match transition {
            TerminalPrimaryTransition::Primary(freeze_guard) => freeze_guard,
            TerminalPrimaryTransition::Fallback(fallback_guard) => {
                return fallback_guard.try_prepare(PreparationFailureCode::ledger());
            }
        };
        let eligible = freeze_guard
            .try_into_eligible()
            .map_err(RetainedPublicationPrepareError::FinalizationContainment)?;
        Ok(eligible.into_prepared(self.set_id))
    }

    fn has_exact_storage_graph(&self) -> bool {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return false;
        }
        let (Some(evidence), Some(evidence_batch), Some(account)) = (
            self.evidence_table.as_ref(),
            self.evidence_batch.as_ref(),
            self.account.as_ref(),
        ) else {
            return false;
        };
        let evidence_exact = retained_block_table_is_exact(
            evidence,
            evidence_batch,
            account,
            self.set_id,
            RetainedMembershipDestination::OutcomeEvidence,
        );
        let canonical_exact = match self.mode {
            RetainedOutcomeMode::SuccessCapable => {
                match (self.canonical_table.as_ref(), self.canonical_batch.as_ref()) {
                    (Some(canonical), Some(canonical_batch)) => retained_block_table_is_exact(
                        canonical,
                        canonical_batch,
                        account,
                        self.set_id,
                        RetainedMembershipDestination::Canonical,
                    ),
                    _ => false,
                }
            }
            RetainedOutcomeMode::FailureOnly => {
                self.canonical_table.is_none() && self.canonical_batch.is_none()
            }
        };
        evidence_exact && canonical_exact
    }

    fn first_containment_account(&self) -> Option<&PhysicalStorageAccount> {
        self.account
            .as_ref()
            .or_else(|| {
                self.operation
                    .as_ref()
                    .and_then(|operation| operation.baseline.first_account())
            })
            .or_else(|| self.canonical_table.as_ref().map(|table| table.account()))
            .or_else(|| {
                self.canonical_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| self.evidence_table.as_ref().map(|table| table.account()))
            .or_else(|| {
                self.evidence_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
    }

    fn record_containment(&self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let account = match self.first_containment_account() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        if let Some(canonical) = self.canonical_table.as_ref() {
            record_forgotten_table_edges(canonical);
        }
        if let Some(evidence) = self.evidence_table.as_ref() {
            record_forgotten_table_edges(evidence);
        }
        if let Some(operation) = self.operation.as_ref() {
            operation.baseline.record_forgotten_physical_edges();
        }
        FinalizationContainmentError::new(AccountPoisoned::new(account.snapshot()), reason)
    }

    fn record_new_fault(&self, reason: AccountPoisonReason) {
        let account = match self
            .account
            .as_ref()
            .or_else(|| self.first_containment_account())
        {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        let _ = account.record_poison(reason);
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(canonical) = self.canonical_table.take() {
            std::mem::forget(canonical);
        }
        if let Some(evidence) = self.evidence_table.take() {
            std::mem::forget(evidence);
        }
        if let Some(batch) = self.canonical_batch.take() {
            std::mem::forget(batch);
        }
        if let Some(batch) = self.evidence_batch.take() {
            std::mem::forget(batch);
        }
        if let Some(account) = self.account.take() {
            drop(account);
        }
        if let Some(mut operation) = self.operation.take() {
            operation.baseline.fail_closed_after_recorded_poison();
            std::mem::forget(operation);
        }
    }

    fn poison_and_fail_closed(
        &mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        self.record_new_fault(reason);
        let cause = self.record_containment(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    fn contain_after_recorded_poison(
        &mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let cause = self.record_containment(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    fn drop_tables_before_attribution(&mut self) -> RetainedStorageDropOutcome {
        let Some(account) = self.account.as_ref() else {
            return RetainedStorageDropOutcome::MissingFaultAnchor;
        };
        let canonical = match self.canonical_table.take() {
            Some(canonical) => drop_retained_table_with_account_anchor(canonical, account),
            None => RetainedStorageDropOutcome::Clean,
        };
        let evidence = match self.evidence_table.take() {
            Some(evidence) => drop_retained_table_with_account_anchor(evidence, account),
            None => RetainedStorageDropOutcome::ForeignOwnershipFault(record_account_poison_event(
                account,
                AccountPoisonReason::OwnershipGraphUnverified,
            )),
        };
        let outcome = canonical.join(evidence);
        if outcome != RetainedStorageDropOutcome::Clean {
            if canonical == RetainedStorageDropOutcome::Clean {
                drop(self.canonical_batch.take());
            }
            if evidence == RetainedStorageDropOutcome::Clean {
                drop(self.evidence_batch.take());
            }
        }
        outcome
    }
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    ValidatedRetainedSuccess<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    /// Consumes a valid success proof after a later domain admission rejects the canonical result.
    ///
    /// Canonical ownership remains sealed and inaccessible until the existing rich-failure
    /// terminal transition drops it before attribution. Evidence remains writable and may be
    /// revalidated without forcing the minimal fallback.
    pub fn downgrade_to_rich_failure(
        self,
        fallback_cause: PreparationFailureCode,
    ) -> RetainedFailureAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        RetainedFailureAssembly {
            assembly: self.assembly,
            fallback_cause,
        }
    }

    /// Performs final control and ledger preparation without reopening graph mutation.
    pub fn try_prepare(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly
            .try_prepare_terminal(RetainedTerminalRequest::Success {
                proof: self.proof,
                final_progress,
            })
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly
            .try_prepare_terminal(RetainedTerminalRequest::MinimalFallback {
                requested_cause: PreparationFailureCode::requested(),
            })
    }
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedFailureAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub fn outcome_evidence_len(&self) -> usize {
        self.assembly.outcome_evidence_len()
    }

    pub fn contains_dimension(&self, dimension: D) -> bool {
        self.assembly.contains_dimension(dimension)
    }

    pub fn try_retain_outcome_evidence<Tag>(
        &mut self,
        plan: RetainedAllocationPlan,
        build: impl for<'a> FnOnce(
            BlockEmplacer<'a, EvidenceBlock>,
        ) -> Result<
            BlockPlacement<'a, EvidenceBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.assembly.try_retain_outcome_evidence(plan, build)
    }

    pub fn try_share_outcome_evidence<Tag>(
        &mut self,
        source: &PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
        source_slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Result<OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.assembly
            .try_share_outcome_evidence(source, source_slot)
    }

    pub fn try_commit_outcome_evidence(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), RetainedPublicationError<D>> {
        self.assembly.try_commit_outcome_evidence(dimension, amount)
    }

    pub fn validate_rich_failure<EvidenceTag>(
        self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedEvidenceClosureSession<'a, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> RetainedFailureValidation<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.assembly.validate_rich_failure(evidence_root, validate)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly
            .try_prepare_terminal(RetainedTerminalRequest::MinimalFallback {
                requested_cause: self.fallback_cause,
            })
    }
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    ValidatedRetainedFailure<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub fn try_prepare(
        self,
        cause: PreparationFailureCode,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly
            .try_prepare_terminal(RetainedTerminalRequest::RichFailure {
                proof: self.proof,
                requested_cause: cause,
                final_progress,
            })
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly.prepare_minimal_fallback()
    }
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedFallbackOnly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    /// Discards candidate tables and prepares only the pre-admitted terminal baseline.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly
            .try_prepare_terminal(RetainedTerminalRequest::MinimalFallback {
                requested_cause: self.cause,
            })
    }
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    fn drop(&mut self) {
        if self.canonical_table.is_none()
            && self.evidence_table.is_none()
            && self.canonical_batch.is_none()
            && self.evidence_batch.is_none()
            && self.account.is_none()
            && self.operation.is_none()
        {
            return;
        }
        if std::thread::panicking() {
            let _ = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return;
        }
        match self.containment {
            AssemblyContainmentProvenance::Healthy => {}
            AssemblyContainmentProvenance::NewFaultPending => {
                let _ = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return;
            }
            AssemblyContainmentProvenance::RetainedFailureAlreadyRecorded(reason) => {
                let _ = self.contain_after_recorded_poison(reason);
                return;
            }
        }
        if !self.has_exact_storage_graph() {
            let _ = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return;
        }
        match self.drop_tables_before_attribution() {
            RetainedStorageDropOutcome::Clean => {}
            RetainedStorageDropOutcome::StorageReleaseUnverified(witness)
            | RetainedStorageDropOutcome::ChargeReleaseUnverified(witness)
            | RetainedStorageDropOutcome::StillSharedOwnershipFault(witness)
            | RetainedStorageDropOutcome::ForeignOwnershipFault(witness) => {
                let _ = self.contain_after_recorded_poison(witness.reason);
                return;
            }
            RetainedStorageDropOutcome::MissingFaultAnchor => {
                let _ = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return;
            }
        }
        drop(self.canonical_batch.take());
        drop(self.evidence_batch.take());
        drop(self.account.take());
        drop(self.operation.take());
    }
}

pub(crate) struct PrimaryPublicationBatches<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
}

struct TerminalPrimaryAttribution<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: Ledger<O, P, D>,
    baseline: FallbackBatchBaseline<O, P, D>,
    batches: PrimaryPublicationBatches<O, P, D>,
}

impl<O, P, D> TerminalPrimaryAttribution<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn try_bind(&mut self) -> Result<(), LedgerCause<D>> {
        self.ledger.try_bind_primary(
            &mut self.baseline,
            self.batches.canonical.as_mut(),
            self.batches.evidence.as_mut(),
        )
    }

    fn finish(self) -> crate::ledger::TypedFreezeProof<O, P, D> {
        self.ledger.finish_bound_primary(
            self.baseline,
            self.batches.canonical,
            self.batches.evidence,
        )
    }

    fn release_sources_after_verified_cleanup(&mut self, cleanup: TerminalTableCleanup) {
        if cleanup.canonical == RetainedStorageDropOutcome::Clean {
            drop(self.batches.canonical.take());
        }
        if cleanup.evidence == RetainedStorageDropOutcome::Clean {
            drop(self.batches.evidence.take());
        }
    }

    fn into_fallback(self) -> TerminalFallbackAttribution<O, P, D> {
        TerminalFallbackAttribution {
            ledger: self.ledger,
            baseline: self.baseline,
            discard: self.batches,
        }
    }
}

struct TerminalFallbackAttribution<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: Ledger<O, P, D>,
    baseline: FallbackBatchBaseline<O, P, D>,
    discard: PrimaryPublicationBatches<O, P, D>,
}

impl<O, P, D> TerminalFallbackAttribution<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn try_prepare_sources(&mut self) -> Result<(), ()> {
        self.baseline.try_prepare_fallback_sources()
    }

    fn finish(self) -> crate::ledger::TypedFreezeProof<O, P, D> {
        self.baseline.finish_prepared_fallback_proof(
            self.ledger,
            self.discard.canonical,
            self.discard.evidence,
        )
    }
}

enum TerminalPrimaryTransition<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Primary(TerminalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
    Fallback(TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
}

struct TerminalPrimaryGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    attribution: Option<TerminalPrimaryAttribution<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock>>,
    discarded_account: Option<PhysicalStorageAccount>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPrimaryGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn new(
        ledger: Ledger<O, P, D>,
        baseline: FallbackBatchBaseline<O, P, D>,
        batches: PrimaryPublicationBatches<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            attribution: Some(TerminalPrimaryAttribution {
                ledger,
                baseline,
                batches,
            }),
            physical: Some(physical),
            discarded_account: None,
        }
    }

    fn attribution_mut(&mut self) -> &mut TerminalPrimaryAttribution<O, P, D> {
        match self.attribution.as_mut() {
            Some(attribution) => attribution,
            None => unreachable_final_owner_state(),
        }
    }

    fn physical(&self) -> &TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock> {
        match self.physical.as_ref() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        }
    }

    fn physical_mut(
        &mut self,
    ) -> &mut TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock> {
        match self.physical.as_mut() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        }
    }

    fn retain_attribution_after_physical_containment(&mut self) {
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
    }

    fn contain_new_fault(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let cause = physical.contain_new_fault(reason);
        self.retain_attribution_after_physical_containment();
        drop(self.discarded_account.take());
        cause
    }

    fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let cause = physical.contain_after_recorded(reason);
        self.retain_attribution_after_physical_containment();
        drop(self.discarded_account.take());
        cause
    }

    fn into_fallback(mut self) -> TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        let attribution = match self.attribution.take() {
            Some(attribution) => attribution.into_fallback(),
            None => unreachable_final_owner_state(),
        };
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        TerminalFallbackGuard::new(attribution, physical)
    }

    fn try_advance(
        mut self,
        terminal: RetainedTerminalMode,
    ) -> Result<
        TerminalPrimaryTransition<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        let bind_result = catch_unwind(AssertUnwindSafe(|| self.attribution_mut().try_bind()));
        match bind_result {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                let expect_canonical = matches!(terminal, RetainedTerminalMode::Success { .. });
                self.physical_mut()
                    .record_new_fault(AccountPoisonReason::PrimaryPublicationBindingPanicked);
                let cleanup = self
                    .physical_mut()
                    .try_drop_primary_tables(expect_canonical);
                let reason = cleanup
                    .failure_reason()
                    .unwrap_or(AccountPoisonReason::PrimaryPublicationBindingPanicked);
                let containment = self.contain_after_recorded(reason);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
            Ok(Err(_cause)) => {
                let expect_canonical = matches!(terminal, RetainedTerminalMode::Success { .. });
                let cleanup = self
                    .physical_mut()
                    .try_drop_primary_tables(expect_canonical);
                if let Some(reason) = cleanup.failure_reason() {
                    self.attribution_mut()
                        .release_sources_after_verified_cleanup(cleanup);
                    let containment = self.contain_after_recorded(reason);
                    return Err(RetainedPublicationPrepareError::FinalizationContainment(
                        containment,
                    ));
                }
                return Ok(TerminalPrimaryTransition::Fallback(self.into_fallback()));
            }
            Ok(Ok(())) => {}
        }

        let fallback_account = match self.physical_mut().try_discard_fallback_allocation() {
            Ok(account) => account,
            Err(cause) => {
                let containment =
                    match cause {
                        RetainedPartCleanupError::MissingFaultAnchor => {
                            self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified)
                        }
                        RetainedPartCleanupError::NewFaultAlreadyRecorded(_) => self
                            .contain_after_recorded(AccountPoisonReason::OwnershipGraphUnverified),
                        RetainedPartCleanupError::StorageReleaseUnverified(_) => self
                            .contain_after_recorded(
                                AccountPoisonReason::RetainedStorageReleaseUnverified,
                            ),
                        RetainedPartCleanupError::ChargeReleaseUnverified(witness) => {
                            self.contain_after_recorded(witness.reason)
                        }
                        RetainedPartCleanupError::StillSharedOwnershipFault(_) => self
                            .contain_after_recorded(AccountPoisonReason::OwnershipGraphUnverified),
                        RetainedPartCleanupError::DeferredOwnershipFault => {
                            self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified)
                        }
                    };
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
        };
        self.discarded_account = Some(fallback_account);
        if !self.physical().qualifies_exact_terminal(terminal) {
            let containment = self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                containment,
            ));
        }
        let physical = match self.physical.take() {
            Some(physical) => physical.finish_exact_terminal(terminal),
            None => unreachable_final_owner_state(),
        };
        let proof = match self.attribution.take() {
            Some(attribution) => attribution.finish(),
            None => unreachable_final_owner_state(),
        };
        let discarded_account = match self.discarded_account.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        Ok(TerminalPrimaryTransition::Primary(
            TerminalFreezeGuard::new(proof, physical, discarded_account),
        ))
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPrimaryGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
        drop(self.discarded_account.take());
    }
}

struct TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    attribution: Option<TerminalFallbackAttribution<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock>>,
    discarded_account: Option<PhysicalStorageAccount>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn new(
        attribution: TerminalFallbackAttribution<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            attribution: Some(attribution),
            physical: Some(physical),
            discarded_account: None,
        }
    }

    fn attribution_mut(&mut self) -> &mut TerminalFallbackAttribution<O, P, D> {
        match self.attribution.as_mut() {
            Some(attribution) => attribution,
            None => unreachable_final_owner_state(),
        }
    }

    fn physical(&self) -> &TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock> {
        match self.physical.as_ref() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        }
    }

    fn physical_mut(
        &mut self,
    ) -> &mut TerminalPhysicalOwnershipManifest<D, F, CanonicalBlock, EvidenceBlock> {
        match self.physical.as_mut() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        }
    }

    fn retain_attribution_after_physical_containment(&mut self) {
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
    }

    fn contain_new_fault(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let cause = physical.contain_new_fault(reason);
        self.retain_attribution_after_physical_containment();
        drop(self.discarded_account.take());
        cause
    }

    fn try_prepare(
        mut self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        match catch_unwind(AssertUnwindSafe(|| {
            self.attribution_mut().try_prepare_sources()
        })) {
            Ok(Ok(())) => {}
            Ok(Err(())) => {
                let containment =
                    self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
            Err(payload) => {
                let containment =
                    self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                discard_untrusted_panic_payload(payload);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
        }

        self.discarded_account = self.physical_mut().take_account();
        if !self.physical().qualifies_exact_fallback() {
            let containment = self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                containment,
            ));
        }
        let physical = match self.physical.take() {
            Some(physical) => physical.finish_exact_fallback(),
            None => unreachable_final_owner_state(),
        };
        let proof = match self.attribution.take() {
            Some(attribution) => attribution.finish(),
            None => unreachable_final_owner_state(),
        };
        let freeze_guard = FallbackFreezeGuard::new(proof, physical, self.discarded_account.take());
        let eligible = freeze_guard
            .try_into_eligible()
            .map_err(RetainedPublicationPrepareError::FinalizationContainment)?;
        Ok(eligible.into_prepared(cause))
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
        drop(self.discarded_account.take());
    }
}

fn prepare_terminal_control<O, P, PP, U, D>(
    coordinator: &mut OperationCoordinator<O, P, PP, U, D>,
    final_progress: FinalProgress<PP, U>,
) -> Option<PreparationFailureCode>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    coordinator
        .try_prepare_typed_control(final_progress)
        .err()
        .map(|cause| PreparationFailureCode::from_control(&cause))
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PreparationFailureClass {
    Requested,
    Control,
    ProductGraph,
    Ledger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparationFailureCode {
    class: PreparationFailureClass,
    code: u32,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl PreparationFailureCode {
    pub const fn domain(code: u32) -> Self {
        Self {
            class: PreparationFailureClass::Requested,
            code,
        }
    }

    const fn requested() -> Self {
        Self {
            class: PreparationFailureClass::Requested,
            code: 0,
        }
    }

    fn from_control(cause: &ControlCause) -> Self {
        let code = match cause {
            ControlCause::Cancelled { .. } => 1,
            ControlCause::DeadlineExceeded => 2,
            ControlCause::ObserverPanicked => 3,
            ControlCause::CancellationSignalPanicked => 4,
            ControlCause::InvalidProgress { .. } => 5,
            ControlCause::ProgressStorage { .. } => 6,
        };
        Self {
            class: PreparationFailureClass::Control,
            code,
        }
    }

    const fn from_closure_validation(cause: RetainedClosureValidationError) -> Self {
        let code = match cause {
            RetainedClosureValidationError::Poisoned => 1,
            RetainedClosureValidationError::WrongOutcomeMode => 2,
            RetainedClosureValidationError::InvalidRoot => 3,
            RetainedClosureValidationError::Rejected => 4,
            RetainedClosureValidationError::Panicked => 5,
        };
        Self {
            class: PreparationFailureClass::ProductGraph,
            code,
        }
    }

    const fn ledger() -> Self {
        Self {
            class: PreparationFailureClass::Ledger,
            code: 1,
        }
    }

    pub const fn class(self) -> PreparationFailureClass {
        self.class
    }

    pub const fn code(self) -> u32 {
        self.code
    }
}

enum RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F> {
    Success {
        canonical: PublishedCanonicalAggregate<CanonicalBlock>,
        outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
        canonical_root: RetainedSlotIdentity,
        outcome_evidence_root: RetainedSlotIdentity,
        account: PhysicalStorageAccount,
    },
    RichFailure {
        outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
        outcome_evidence_root: RetainedSlotIdentity,
        account: PhysicalStorageAccount,
        cause: PreparationFailureCode,
    },
    MinimalFallback {
        part: PublishedPart<F>,
        cause: PreparationFailureCode,
    },
}

/// Prepared terminal publication after one atomic ledger transition.
///
/// Success freezes canonical and evidence destinations, rich failure freezes evidence only, and
/// minimal fallback freezes the pre-admitted fallback/evidence baseline. Handoff is a pure
/// allocation-free ownership move.
#[must_use = "prepared publication owns terminal output and frozen evidence until handoff"]
pub(crate) struct PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D> {
    payload: RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F>,
    evidence: PublishedPart<PublicationEvidenceCells<D>>,
}

/// Exact terminal classification consumed before any domain prepared wrapper is constructed.
pub(crate) enum PreparedRetainedPublicationOutcome<CanonicalBlock, EvidenceBlock, F, D> {
    Success(PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, D>),
    RichFailure(PreparedRetainedRichFailure<EvidenceBlock, D>),
    MinimalFallback(PreparedRetainedFallback<F, D>),
}

/// Exact prepared success. Its retained roots remain opaque until handoff.
pub(crate) struct PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, D> {
    canonical: PublishedCanonicalAggregate<CanonicalBlock>,
    outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    canonical_root: RetainedSlotIdentity,
    outcome_evidence_root: RetainedSlotIdentity,
    account: PhysicalStorageAccount,
    evidence: PublishedPart<PublicationEvidenceCells<D>>,
}

/// Exact prepared rich failure. Its retained evidence remains opaque until handoff.
pub(crate) struct PreparedRetainedRichFailure<EvidenceBlock, D> {
    outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    outcome_evidence_root: RetainedSlotIdentity,
    account: PhysicalStorageAccount,
    cause: PreparationFailureCode,
    evidence: PublishedPart<PublicationEvidenceCells<D>>,
}

/// Exact prepared minimal fallback. Its payload remains opaque until handoff.
pub(crate) struct PreparedRetainedFallback<F, D> {
    part: PublishedPart<F>,
    cause: PreparationFailureCode,
    evidence: PublishedPart<PublicationEvidenceCells<D>>,
}

impl<CanonicalBlock, EvidenceBlock, F, D>
    PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>
where
    D: Copy + Ord,
{
    /// Consumes the unclassified private owner into one exact prepared terminal state.
    pub fn into_outcome(
        self,
    ) -> PreparedRetainedPublicationOutcome<CanonicalBlock, EvidenceBlock, F, D> {
        let Self { payload, evidence } = self;
        match payload {
            RetainedPublicationPayload::Success {
                canonical,
                outcome_evidence,
                canonical_root,
                outcome_evidence_root,
                account,
            } => PreparedRetainedPublicationOutcome::Success(PreparedRetainedSuccess {
                canonical,
                outcome_evidence,
                canonical_root,
                outcome_evidence_root,
                account,
                evidence,
            }),
            RetainedPublicationPayload::RichFailure {
                outcome_evidence,
                outcome_evidence_root,
                account,
                cause,
            } => PreparedRetainedPublicationOutcome::RichFailure(PreparedRetainedRichFailure {
                outcome_evidence,
                outcome_evidence_root,
                account,
                cause,
                evidence,
            }),
            RetainedPublicationPayload::MinimalFallback { part, cause } => {
                PreparedRetainedPublicationOutcome::MinimalFallback(PreparedRetainedFallback {
                    part,
                    cause,
                    evidence,
                })
            }
        }
    }
}

/// Canonical owning half ready to move into a domain-owned validated Scene wrapper.
pub(crate) struct PublishedCanonicalOutput<CanonicalBlock> {
    aggregate: PublishedCanonicalAggregate<CanonicalBlock>,
    root: RetainedSlotIdentity,
    account: PhysicalStorageAccount,
}

impl<CanonicalBlock> PublishedCanonicalOutput<CanonicalBlock> {
    pub fn aggregate(&self) -> &PublishedCanonicalAggregate<CanonicalBlock> {
        &self.aggregate
    }

    pub fn root(&self) -> &CanonicalBlock {
        match self.aggregate.inner.member(self.root) {
            Some(root) => root,
            None => unreachable_final_owner_state(),
        }
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        &self.account
    }
}

/// Outcome-evidence owning half ready to move into a domain report.
pub(crate) struct PublishedOutcomeEvidenceOutput<EvidenceBlock> {
    aggregate: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    root: RetainedSlotIdentity,
}

/// Evidence-only terminal owner with its frozen outer failure cause and account lineage.
pub(crate) struct PublishedRichFailureOutput<EvidenceBlock> {
    outcome_evidence: PublishedOutcomeEvidenceOutput<EvidenceBlock>,
    account: PhysicalStorageAccount,
    cause: PreparationFailureCode,
}

impl<EvidenceBlock> PublishedRichFailureOutput<EvidenceBlock> {
    pub fn outcome_evidence(&self) -> &PublishedOutcomeEvidenceOutput<EvidenceBlock> {
        &self.outcome_evidence
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        &self.account
    }

    pub const fn cause(&self) -> PreparationFailureCode {
        self.cause
    }
}

impl<EvidenceBlock> PublishedOutcomeEvidenceOutput<EvidenceBlock> {
    pub fn aggregate(&self) -> &PublishedOutcomeEvidenceAggregate<EvidenceBlock> {
        &self.aggregate
    }

    pub fn root(&self) -> &EvidenceBlock {
        match self.aggregate.inner.member(self.root) {
            Some(root) => root,
            None => unreachable_final_owner_state(),
        }
    }
}

/// Frozen destination-aware ledger evidence moved into a domain report.
pub(crate) struct PublishedLedgerEvidence<D> {
    evidence: PublishedPart<PublicationEvidenceCells<D>>,
}

impl<D: Copy + Ord> PublishedLedgerEvidence<D> {
    pub fn dimension(&self, dimension: D) -> Option<PublishedDimensionSnapshot<D>> {
        self.evidence.allocation().as_ref().dimension(dimension)
    }
}

/// Pre-admitted fallback owner after a failed terminal publication.
pub(crate) struct PublishedFallbackOutput<F> {
    part: PublishedPart<F>,
    cause: PreparationFailureCode,
}

impl<F> PublishedFallbackOutput<F> {
    pub fn value(&self) -> &F {
        self.part.allocation().as_ref()
    }

    pub const fn cause(&self) -> PreparationFailureCode {
        self.cause
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        self.part.account()
    }
}

/// Exact published success owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedSuccessParts<CanonicalBlock, EvidenceBlock, D> {
    pub(crate) canonical: PublishedCanonicalOutput<CanonicalBlock>,
    pub(crate) outcome_evidence: PublishedOutcomeEvidenceOutput<EvidenceBlock>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<D>,
}

/// Exact published rich-failure owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedRichFailureParts<EvidenceBlock, D> {
    pub(crate) outcome_evidence: PublishedRichFailureOutput<EvidenceBlock>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<D>,
}

/// Exact published minimal-fallback owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedFallbackParts<F, D> {
    pub(crate) fallback: PublishedFallbackOutput<F>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<D>,
}

impl<CanonicalBlock, EvidenceBlock, D> PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, D>
where
    D: Copy + Ord,
{
    /// Performs a pure ownership handoff directly into exact success owners.
    pub fn handoff(self) -> PublishedRetainedSuccessParts<CanonicalBlock, EvidenceBlock, D> {
        let Self {
            canonical,
            outcome_evidence,
            canonical_root,
            outcome_evidence_root,
            account,
            evidence,
        } = self;
        let ledger_evidence = PublishedLedgerEvidence { evidence };
        PublishedRetainedSuccessParts {
            canonical: PublishedCanonicalOutput {
                aggregate: canonical,
                root: canonical_root,
                account,
            },
            outcome_evidence: PublishedOutcomeEvidenceOutput {
                aggregate: outcome_evidence,
                root: outcome_evidence_root,
            },
            ledger_evidence,
        }
    }
}

impl<EvidenceBlock, D> PreparedRetainedRichFailure<EvidenceBlock, D>
where
    D: Copy + Ord,
{
    /// Performs a pure ownership handoff directly into exact rich-failure owners.
    pub fn handoff(self) -> PublishedRetainedRichFailureParts<EvidenceBlock, D> {
        let Self {
            outcome_evidence,
            outcome_evidence_root,
            account,
            cause,
            evidence,
        } = self;
        PublishedRetainedRichFailureParts {
            outcome_evidence: PublishedRichFailureOutput {
                outcome_evidence: PublishedOutcomeEvidenceOutput {
                    aggregate: outcome_evidence,
                    root: outcome_evidence_root,
                },
                account,
                cause,
            },
            ledger_evidence: PublishedLedgerEvidence { evidence },
        }
    }
}

impl<F, D> PreparedRetainedFallback<F, D>
where
    D: Copy + Ord,
{
    /// Performs a pure ownership handoff directly into exact fallback owners.
    pub fn handoff(self) -> PublishedRetainedFallbackParts<F, D> {
        let Self {
            part,
            cause,
            evidence,
        } = self;
        PublishedRetainedFallbackParts {
            fallback: PublishedFallbackOutput { part, cause },
            ledger_evidence: PublishedLedgerEvidence { evidence },
        }
    }
}

/// Failure to establish a safe terminal publication; all uncertain accounting is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum RetainedPublicationPrepareError {
    /// Final eligibility failed closed without producing any prepared or fallback owner.
    FinalizationContainment(FinalizationContainmentError),
}

impl fmt::Display for RetainedPublicationPrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FinalizationContainment(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for RetainedPublicationPrepareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FinalizationContainment(cause) => Some(cause),
        }
    }
}

fn drop_retained_table_with_account_anchor<Block>(
    table: RetainedTableAllocation<Block>,
    account: &PhysicalStorageAccount,
) -> RetainedStorageDropOutcome {
    let ownership = if table.belongs_to(account) {
        RetainedStorageDropOutcome::Clean
    } else {
        RetainedStorageDropOutcome::ForeignOwnershipFault(record_account_poison_event(
            account,
            AccountPoisonReason::OwnershipGraphUnverified,
        ))
    };
    ownership.join(drop_retained_table_before_attribution(table, account))
}

fn drop_retained_table_before_attribution<Block>(
    table: RetainedTableAllocation<Block>,
    account: &PhysicalStorageAccount,
) -> RetainedStorageDropOutcome {
    if !table.belongs_to(account) {
        record_forgotten_table_edges(&table);
        std::mem::forget(table);
        return RetainedStorageDropOutcome::MissingFaultAnchor;
    }
    drop_retained_allocation_before_attribution(table, account)
}

fn freeze_terminal_evidence<O, P, D>(
    _eligibility: PublicationEligibility,
    proof: crate::ledger::TypedFreezeProof<O, P, D>,
    discarded_account: Option<PhysicalStorageAccount>,
    evidence: &DetachedRetainedPart<PublicationEvidenceCells<D>>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let snapshot = proof.freeze();
    drop(discarded_account);
    evidence.allocation().as_ref().freeze_from(&snapshot);
    drop(snapshot);
}

#[cfg(test)]
thread_local! {
    static PUBLICATION_ELIGIBILITY_PAUSE: std::cell::RefCell<
        Option<(std::sync::Arc<std::sync::Barrier>, std::sync::Arc<std::sync::Barrier>)>
    > = const { std::cell::RefCell::new(None) };
    static FAIL_FINAL_OWNER_ALLOCATION_AFTER: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
    static FAIL_NEXT_MEMBERSHIP_ASSIGNMENT: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static PANIC_FINAL_OWNER_DROP_AFTER: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
    static TEST_MEMBER_LOOKUPS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static FAIL_NEXT_START_EVIDENCE_BUILD: std::cell::Cell<Option<RetainedBuildFailureCode>> =
        const { std::cell::Cell::new(None) };
    static PANIC_NEXT_START_EVIDENCE_BUILD: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static LOSE_NEXT_START_FALLBACK_ACCOUNT_ANCHOR: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static START_EVIDENCE_BUILD_CALLS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static STARTUP_FAILURE_LEDGER_SNAPSHOT: std::cell::Cell<Option<(u64, u64)>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
fn install_publication_eligibility_pause(
    entered: std::sync::Arc<std::sync::Barrier>,
    resume: std::sync::Arc<std::sync::Barrier>,
) {
    PUBLICATION_ELIGIBILITY_PAUSE.with(|pause| {
        *pause.borrow_mut() = Some((entered, resume));
    });
}

#[cfg(test)]
fn pause_after_publication_eligibility() {
    let pause = PUBLICATION_ELIGIBILITY_PAUSE.with(|slot| slot.borrow_mut().take());
    if let Some((entered, resume)) = pause {
        entered.wait();
        resume.wait();
    }
}

#[cfg(test)]
fn record_test_member_lookup() {
    TEST_MEMBER_LOOKUPS.with(|lookups| lookups.set(lookups.get().saturating_add(1)));
}

#[cfg(test)]
fn take_test_member_lookups() -> usize {
    TEST_MEMBER_LOOKUPS.with(|lookups| lookups.replace(0))
}

#[cfg(test)]
fn fail_next_final_owner_allocation() {
    fail_final_owner_allocation_after(0);
}

#[cfg(test)]
fn fail_final_owner_allocation_after(skipped_allocations: usize) {
    FAIL_FINAL_OWNER_ALLOCATION_AFTER.with(|remaining| remaining.set(Some(skipped_allocations)));
}

#[cfg(test)]
pub(crate) fn test_fail_next_membership_assignment() {
    FAIL_NEXT_MEMBERSHIP_ASSIGNMENT.with(|flag| flag.set(true));
}

#[cfg(test)]
fn take_injected_membership_assignment_failure() -> bool {
    FAIL_NEXT_MEMBERSHIP_ASSIGNMENT.with(|flag| flag.replace(false))
}

#[cfg(test)]
pub(crate) fn test_panic_final_owner_drop_after(skipped_drops: usize) {
    PANIC_FINAL_OWNER_DROP_AFTER.with(|remaining| remaining.set(Some(skipped_drops)));
}

#[cfg(test)]
fn take_injected_final_owner_drop_panic() -> bool {
    PANIC_FINAL_OWNER_DROP_AFTER.with(|remaining| match remaining.get() {
        None => false,
        Some(0) => {
            remaining.set(None);
            true
        }
        Some(count) => {
            remaining.set(Some(count - 1));
            false
        }
    })
}

#[cfg(test)]
fn fail_next_start_evidence_build(cause: RetainedBuildFailureCode) {
    FAIL_NEXT_START_EVIDENCE_BUILD.with(|failure| failure.set(Some(cause)));
}

#[cfg(test)]
fn panic_next_start_evidence_build() {
    PANIC_NEXT_START_EVIDENCE_BUILD.with(|panic| panic.set(true));
}

#[cfg(test)]
fn take_injected_start_evidence_build_panic() -> bool {
    PANIC_NEXT_START_EVIDENCE_BUILD.with(|panic| panic.replace(false))
}

#[cfg(test)]
fn take_injected_start_evidence_build_failure() -> Option<RetainedBuildFailureCode> {
    FAIL_NEXT_START_EVIDENCE_BUILD.with(std::cell::Cell::take)
}

#[cfg(test)]
fn lose_next_start_fallback_account_anchor() {
    LOSE_NEXT_START_FALLBACK_ACCOUNT_ANCHOR.with(|flag| flag.set(true));
}

#[cfg(test)]
fn take_injected_start_fallback_account_anchor_loss() -> bool {
    LOSE_NEXT_START_FALLBACK_ACCOUNT_ANCHOR.with(|flag| flag.replace(false))
}

#[cfg(test)]
fn record_start_evidence_build_call() {
    START_EVIDENCE_BUILD_CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));
}

#[cfg(test)]
fn take_start_evidence_build_calls() -> usize {
    START_EVIDENCE_BUILD_CALLS.with(std::cell::Cell::take)
}

#[cfg(test)]
fn take_startup_failure_ledger_snapshot() -> Option<(u64, u64)> {
    STARTUP_FAILURE_LEDGER_SNAPSHOT.with(std::cell::Cell::take)
}

#[cfg(test)]
fn take_injected_final_owner_allocation_failure() -> bool {
    FAIL_FINAL_OWNER_ALLOCATION_AFTER.with(|remaining| match remaining.get() {
        None => false,
        Some(0) => {
            remaining.set(None);
            true
        }
        Some(count) => {
            remaining.set(Some(count - 1));
            false
        }
    })
}

#[cfg(test)]
mod tests;
