//! Fallible physical storage accounting for retained operation allocations.

#![allow(
    clippy::result_large_err,
    reason = "bounded poison snapshots stay inline so containment never needs a failure-path allocation"
)]

mod account;
mod final_owner;

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
use account::quarantined_account_reason;
pub use account::{
    AccountPoisonEvidence, AccountPoisonReason, AccountPoisoned, FinalizationContainmentError,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountFactorySnapshot,
    PhysicalStorageAccountLimits, PhysicalStorageAccountSnapshot, PhysicalStorageLimitError,
};
use final_owner::{
    FinalOwnerAllocation, FinalOwnerAllocationError, FinalOwnerInnerDrop,
    unreachable_final_owner_state,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AdmissionCharge {
    bytes: u64,
    allocations: u64,
}

impl AdmissionCharge {
    const fn new(bytes: u64, allocations: u64) -> Self {
        Self { bytes, allocations }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CounterLimits {
    bytes: u64,
    allocations: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CounterState {
    live_bytes: u64,
    live_allocations: u64,
    peak_bytes: u64,
    peak_allocations: u64,
    total_admitted_bytes: u64,
    total_admitted_allocations: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmissionError {
    AccountingOverflow,
    ByteLimitExceeded { requested: u64, limit: u64 },
    AllocationLimitExceeded { requested: u64, limit: u64 },
}

fn lock_recovering_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn try_admit(
    counters: &Mutex<CounterState>,
    limits: CounterLimits,
    charge: AdmissionCharge,
) -> Result<(), AdmissionError> {
    let mut counters = lock_recovering_poison(counters);
    try_admit_locked(&mut counters, limits, charge)
}

fn try_admit_locked(
    counters: &mut CounterState,
    limits: CounterLimits,
    charge: AdmissionCharge,
) -> Result<(), AdmissionError> {
    let live_bytes = counters
        .live_bytes
        .checked_add(charge.bytes)
        .ok_or(AdmissionError::AccountingOverflow)?;
    let live_allocations = counters
        .live_allocations
        .checked_add(charge.allocations)
        .ok_or(AdmissionError::AccountingOverflow)?;
    if live_bytes > limits.bytes {
        return Err(AdmissionError::ByteLimitExceeded {
            requested: live_bytes,
            limit: limits.bytes,
        });
    }
    if live_allocations > limits.allocations {
        return Err(AdmissionError::AllocationLimitExceeded {
            requested: live_allocations,
            limit: limits.allocations,
        });
    }

    let total_admitted_bytes = counters
        .total_admitted_bytes
        .checked_add(charge.bytes)
        .ok_or(AdmissionError::AccountingOverflow)?;
    let total_admitted_allocations = counters
        .total_admitted_allocations
        .checked_add(charge.allocations)
        .ok_or(AdmissionError::AccountingOverflow)?;

    counters.live_bytes = live_bytes;
    counters.live_allocations = live_allocations;
    counters.peak_bytes = counters.peak_bytes.max(live_bytes);
    counters.peak_allocations = counters.peak_allocations.max(live_allocations);
    counters.total_admitted_bytes = total_admitted_bytes;
    counters.total_admitted_allocations = total_admitted_allocations;
    Ok(())
}

fn release(counters: &Mutex<CounterState>, charge: AdmissionCharge) {
    let mut counters = lock_recovering_poison(counters);
    let _released = release_locked(&mut counters, charge);
}

fn release_locked(counters: &mut CounterState, charge: AdmissionCharge) -> bool {
    let Some(live_bytes) = counters.live_bytes.checked_sub(charge.bytes) else {
        return false;
    };
    let Some(live_allocations) = counters.live_allocations.checked_sub(charge.allocations) else {
        return false;
    };
    counters.live_bytes = live_bytes;
    counters.live_allocations = live_allocations;
    true
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedStorageDropOutcome {
    Clean,
    StorageReleaseUnverified(AccountPoisonEventWitness),
    ChargeReleaseUnverified(AccountPoisonEventWitness),
    StillSharedOwnershipFault(AccountPoisonEventWitness),
    ForeignOwnershipFault(AccountPoisonEventWitness),
    MissingFaultAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RetainedStorageDropSeverity {
    Clean,
    Charge,
    Ownership,
    Storage,
}

impl RetainedStorageDropOutcome {
    const fn severity(self) -> RetainedStorageDropSeverity {
        match self {
            Self::Clean => RetainedStorageDropSeverity::Clean,
            Self::ChargeReleaseUnverified(_) => RetainedStorageDropSeverity::Charge,
            Self::StillSharedOwnershipFault(_)
            | Self::ForeignOwnershipFault(_)
            | Self::MissingFaultAnchor => RetainedStorageDropSeverity::Ownership,
            Self::StorageReleaseUnverified(_) => RetainedStorageDropSeverity::Storage,
        }
    }

    const fn witness(self) -> Option<AccountPoisonEventWitness> {
        match self {
            Self::StorageReleaseUnverified(witness)
            | Self::ChargeReleaseUnverified(witness)
            | Self::StillSharedOwnershipFault(witness)
            | Self::ForeignOwnershipFault(witness) => Some(witness),
            Self::Clean | Self::MissingFaultAnchor => None,
        }
    }

    fn join(self, other: Self) -> Self {
        match self.severity().cmp(&other.severity()) {
            std::cmp::Ordering::Less => other,
            std::cmp::Ordering::Greater => self,
            std::cmp::Ordering::Equal if self.witness().is_none() && other.witness().is_some() => {
                other
            }
            std::cmp::Ordering::Equal => self,
        }
    }
}

fn join_pre_attribution_cleanup_cause<D>(
    outcome: RetainedStorageDropOutcome,
    original: RetainedWriterError<D>,
) -> RetainedWriterError<D> {
    match outcome.severity() {
        RetainedStorageDropSeverity::Clean => original,
        RetainedStorageDropSeverity::Storage => RetainedWriterError::StorageDropUnverified,
        RetainedStorageDropSeverity::Ownership => {
            if matches!(original, RetainedWriterError::StorageDropUnverified) {
                RetainedWriterError::StorageDropUnverified
            } else {
                RetainedWriterError::OwnershipUnverified
            }
        }
        RetainedStorageDropSeverity::Charge => match original {
            RetainedWriterError::StorageDropUnverified => {
                RetainedWriterError::StorageDropUnverified
            }
            RetainedWriterError::OwnershipUnverified => RetainedWriterError::OwnershipUnverified,
            _ => RetainedWriterError::ChargeReleaseUnverified,
        },
    }
}

fn finish_pre_attribution_cleanup<O, P, D, Destination>(
    attribution: OperationAttributionReservation<O, P, D, Destination>,
    outcome: RetainedStorageDropOutcome,
    original: RetainedWriterError<D>,
) -> RetainedWriterError<D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if outcome == RetainedStorageDropOutcome::Clean {
        drop(attribution);
    } else {
        std::mem::forget(attribution);
    }
    join_pre_attribution_cleanup_cause(outcome, original)
}

fn finish_pre_attribution_batch_cleanup<O, P, D, Destination>(
    batch: Option<AttributionReservationBatch<O, P, D, Destination>>,
    outcome: RetainedStorageDropOutcome,
    original: RetainedWriterError<D>,
) -> RetainedWriterError<D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if outcome == RetainedStorageDropOutcome::Clean {
        drop(batch);
    } else if let Some(batch) = batch {
        std::mem::forget(batch);
    }
    join_pre_attribution_cleanup_cause(outcome, original)
}

fn map_inherited_cleanup_error<D>(
    account: &PhysicalStorageAccount,
    outcome: FinalOwnerReleaseOutcome,
    original: RetainedWriterError<D>,
) -> RetainedWriterError<D> {
    match outcome {
        FinalOwnerReleaseOutcome::Clean => {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            original
        }
        FinalOwnerReleaseOutcome::StorageReleaseUnverified(_) => {
            RetainedWriterError::StorageDropUnverified
        }
        FinalOwnerReleaseOutcome::ChargeReleaseUnverified(_) => match original {
            RetainedWriterError::StorageDropUnverified => {
                RetainedWriterError::StorageDropUnverified
            }
            RetainedWriterError::OwnershipUnverified => RetainedWriterError::OwnershipUnverified,
            _ => RetainedWriterError::ChargeReleaseUnverified,
        },
        FinalOwnerReleaseOutcome::OwnershipUnverified(_) => {
            if matches!(original, RetainedWriterError::StorageDropUnverified) {
                RetainedWriterError::StorageDropUnverified
            } else {
                RetainedWriterError::OwnershipUnverified
            }
        }
    }
}

fn drop_retained_allocation_before_attribution<T, P>(
    allocation: PolicyRetainedAllocation<T, P>,
    account: &PhysicalStorageAccount,
) -> RetainedStorageDropOutcome
where
    P: RetainedValuePolicy<RetainedPayload<T>>,
{
    if !allocation.belongs_to(account) {
        let allocation_account = allocation.account();
        let poisoned =
            record_account_poison_event(account, AccountPoisonReason::OwnershipGraphUnverified);
        let _ =
            allocation_account.record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
        std::mem::forget(allocation);
        return RetainedStorageDropOutcome::ForeignOwnershipFault(poisoned);
    }
    let release = match allocation.try_release_unique_before_attribution() {
        Ok(release) => release,
        Err((allocation, poisoned)) => {
            std::mem::forget(allocation);
            return RetainedStorageDropOutcome::StillSharedOwnershipFault(poisoned);
        }
    };
    match release {
        FinalOwnerReleaseOutcome::Clean => RetainedStorageDropOutcome::Clean,
        FinalOwnerReleaseOutcome::StorageReleaseUnverified(poisoned) => {
            RetainedStorageDropOutcome::StorageReleaseUnverified(poisoned)
        }
        FinalOwnerReleaseOutcome::ChargeReleaseUnverified(poisoned) => {
            RetainedStorageDropOutcome::ChargeReleaseUnverified(poisoned)
        }
        FinalOwnerReleaseOutcome::OwnershipUnverified(poisoned) => {
            RetainedStorageDropOutcome::StillSharedOwnershipFault(poisoned)
        }
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

/// A destination-qualified retained allocation whose attribution cannot be detached.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
struct RetainedPart<T, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    allocation: Option<RetainedAllocation<T>>,
    batch: Option<AttributionReservationBatch<O, P, D, Destination>>,
    account: Option<PhysicalStorageAccount>,
}

/// A retained part whose physical owner, attribution batch, and account witness are exact.
///
/// Keeping the original owner intact until detachment makes the exact-to-detached transition
/// infallible while preserving its storage-before-attribution destructor on every earlier exit.
struct ExactRetainedPart<T, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    inner: RetainedPart<T, Destination, O, P, D>,
}

impl<T, Destination, O, P, D> ExactRetainedPart<T, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    fn into_inner(self) -> RetainedPart<T, Destination, O, P, D> {
        self.inner
    }

    fn into_detached(
        mut self,
    ) -> (
        AttributionReservationBatch<O, P, D, Destination>,
        DetachedRetainedPart<T>,
    ) {
        let batch = match self.inner.batch.take() {
            Some(batch) => batch,
            None => unreachable_final_owner_state(),
        };
        let allocation = match self.inner.allocation.take() {
            Some(allocation) => allocation,
            None => unreachable_final_owner_state(),
        };
        let account = match self.inner.account.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        (
            batch,
            DetachedRetainedPart {
                allocation: Some(allocation),
                account: Some(account),
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedPartCleanupError {
    NewFaultAlreadyRecorded(AccountPoisonEventWitness),
    StorageReleaseUnverified(AccountPoisonEventWitness),
    ChargeReleaseUnverified(AccountPoisonEventWitness),
    StillSharedOwnershipFault(AccountPoisonEventWitness),
    DeferredOwnershipFault,
    MissingFaultAnchor,
}

fn try_discard_retained_owner<T>(
    allocation: &mut Option<RetainedAllocation<T>>,
    account: &mut Option<PhysicalStorageAccount>,
    defer_ownership_fault: bool,
) -> Result<PhysicalStorageAccount, RetainedPartCleanupError> {
    let Some(expected_account) = account.as_ref() else {
        let Some(allocation) = allocation.take() else {
            return Err(RetainedPartCleanupError::MissingFaultAnchor);
        };
        let actual_account = allocation.account();
        if defer_ownership_fault {
            std::mem::forget(allocation);
            return Err(RetainedPartCleanupError::DeferredOwnershipFault);
        }
        let poisoned = record_account_poison_event(
            actual_account,
            AccountPoisonReason::OwnershipGraphUnverified,
        );
        std::mem::forget(allocation);
        return Err(RetainedPartCleanupError::NewFaultAlreadyRecorded(poisoned));
    };
    let Some(allocation_owner) = allocation.take() else {
        if defer_ownership_fault {
            drop(account.take());
            return Err(RetainedPartCleanupError::DeferredOwnershipFault);
        }
        let poisoned = record_account_poison_event(
            expected_account,
            AccountPoisonReason::OwnershipGraphUnverified,
        );
        drop(account.take());
        return Err(RetainedPartCleanupError::NewFaultAlreadyRecorded(poisoned));
    };
    if !allocation_owner.belongs_to(expected_account) {
        if defer_ownership_fault {
            let _ = allocation_owner
                .account()
                .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
            std::mem::forget(allocation_owner);
            drop(account.take());
            return Err(RetainedPartCleanupError::DeferredOwnershipFault);
        }
        let poisoned = record_account_poison_event(
            expected_account,
            AccountPoisonReason::OwnershipGraphUnverified,
        );
        let _ = allocation_owner
            .account()
            .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
        std::mem::forget(allocation_owner);
        drop(account.take());
        return Err(RetainedPartCleanupError::NewFaultAlreadyRecorded(poisoned));
    }
    let outcome = if defer_ownership_fault {
        match allocation_owner.try_release_unique_before_attribution_deferred() {
            Ok(outcome) => outcome,
            Err(still_shared) => {
                *allocation = Some(still_shared);
                return Err(RetainedPartCleanupError::DeferredOwnershipFault);
            }
        }
    } else {
        match allocation_owner.try_release_unique_before_attribution() {
            Ok(outcome) => outcome,
            Err((still_shared, poisoned)) => {
                *allocation = Some(still_shared);
                return Err(RetainedPartCleanupError::StillSharedOwnershipFault(
                    poisoned,
                ));
            }
        }
    };
    match outcome {
        FinalOwnerReleaseOutcome::Clean => match account.take() {
            Some(account) => Ok(account),
            None => Err(RetainedPartCleanupError::MissingFaultAnchor),
        },
        FinalOwnerReleaseOutcome::StorageReleaseUnverified(poisoned) => {
            drop(account.take());
            Err(RetainedPartCleanupError::StorageReleaseUnverified(poisoned))
        }
        FinalOwnerReleaseOutcome::ChargeReleaseUnverified(poisoned) => {
            drop(account.take());
            Err(RetainedPartCleanupError::ChargeReleaseUnverified(poisoned))
        }
        FinalOwnerReleaseOutcome::OwnershipUnverified(poisoned) => {
            drop(account.take());
            Err(RetainedPartCleanupError::StillSharedOwnershipFault(
                poisoned,
            ))
        }
    }
}

struct DetachedRetainedPart<T> {
    allocation: Option<RetainedAllocation<T>>,
    account: Option<PhysicalStorageAccount>,
}

impl<T> DetachedRetainedPart<T> {
    fn allocation(&self) -> &RetainedAllocation<T> {
        match self.allocation.as_ref() {
            Some(allocation) => allocation,
            None => unreachable_final_owner_state(),
        }
    }

    fn account(&self) -> &PhysicalStorageAccount {
        match self.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }

    fn has_exact_owner(&self) -> bool {
        matches!(
            (self.allocation.as_ref(), self.account.as_ref()),
            (Some(allocation), Some(account)) if allocation.belongs_to(account)
        )
    }

    fn is_empty(&self) -> bool {
        self.allocation.is_none() && self.account.is_none()
    }

    fn try_discard_allocation(
        &mut self,
    ) -> Result<PhysicalStorageAccount, RetainedPartCleanupError> {
        try_discard_retained_owner(&mut self.allocation, &mut self.account, false)
    }

    fn record_forgotten_physical_edge(&self) {
        if let Some(allocation) = self.allocation.as_ref() {
            let _ = allocation
                .account()
                .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
        }
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(allocation) = self.allocation.take() {
            std::mem::forget(allocation);
        }
        drop(self.account.take());
    }

    fn new_fault_and_fail_closed(&mut self, reason: AccountPoisonReason) {
        if let Some(account) = self.account.as_ref().or_else(|| {
            self.allocation
                .as_ref()
                .map(|allocation| allocation.account())
        }) {
            let _ = account.record_poison(reason);
        }
        self.record_forgotten_physical_edge();
        self.fail_closed_after_recorded_poison();
    }
}

impl<T> Drop for DetachedRetainedPart<T> {
    fn drop(&mut self) {
        if self.allocation.is_none() && self.account.is_none() {
            return;
        }
        if std::thread::panicking() || !self.has_exact_owner() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return;
        }
        let _ = self.try_discard_allocation();
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<T, Destination, O, P, D> RetainedPart<T, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    fn allocation(&self) -> &RetainedAllocation<T> {
        match self.allocation.as_ref() {
            Some(allocation) => allocation,
            None => unreachable_final_owner_state(),
        }
    }

    fn account(&self) -> &PhysicalStorageAccount {
        match self.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }

    fn try_into_exact(self) -> Result<ExactRetainedPart<T, Destination, O, P, D>, Self> {
        if !self.has_exact_product_graph() {
            return Err(self);
        }
        Ok(ExactRetainedPart { inner: self })
    }

    fn try_discard_allocation_before_batch_release(
        &mut self,
    ) -> Result<PhysicalStorageAccount, RetainedPartCleanupError> {
        self.try_discard_allocation_before_batch_release_with_policy(false)
    }

    fn try_discard_allocation_before_batch_release_with_policy(
        &mut self,
        defer_ownership_fault: bool,
    ) -> Result<PhysicalStorageAccount, RetainedPartCleanupError> {
        try_discard_retained_owner(
            &mut self.allocation,
            &mut self.account,
            defer_ownership_fault,
        )
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(allocation) = self.allocation.take() {
            std::mem::forget(allocation);
        }
        if let Some(batch) = self.batch.take() {
            std::mem::forget(batch);
        }
        if let Some(account) = self.account.take() {
            drop(account);
        }
    }

    fn record_forgotten_physical_edges(&self) {
        if let Some(allocation) = self.allocation.as_ref() {
            let _ = allocation
                .account()
                .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
        }
    }

    fn poison_and_fail_closed(&mut self) {
        self.record_forgotten_physical_edges();
        self.fail_closed_after_recorded_poison();
    }

    fn new_fault_and_fail_closed(&mut self, reason: AccountPoisonReason) {
        let account = self.account.as_ref().or_else(|| {
            self.allocation
                .as_ref()
                .map(|allocation| allocation.account())
        });
        if let Some(account) = account {
            let _ = account.record_poison(reason);
        }
        self.poison_and_fail_closed();
    }

    fn has_exact_product_graph(&self) -> bool {
        let (Some(allocation), Some(batch), Some(account)) = (
            self.allocation.as_ref(),
            self.batch.as_ref(),
            self.account.as_ref(),
        ) else {
            return false;
        };
        allocation.belongs_to(account)
            && batch.attribution_len() == 1
            && batch.attributed_bytes() == Some(allocation.charged_bytes())
    }
}

impl<T, Destination, O, P, D> Drop for RetainedPart<T, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    fn drop(&mut self) {
        if self.allocation.is_none() && self.batch.is_none() && self.account.is_none() {
            return;
        }
        if std::thread::panicking() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return;
        }
        if !self.has_exact_product_graph() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return;
        }
        match self.try_discard_allocation_before_batch_release() {
            Ok(account) => {
                drop(self.batch.take());
                drop(account);
            }
            Err(_) => {
                self.fail_closed_after_recorded_poison();
            }
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
struct PublishedPart<T> {
    allocation: Option<RetainedAllocation<T>>,
    account: Option<PhysicalStorageAccount>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<T> PublishedPart<T> {
    /// Moves an owner whose exact shape was established by a typed terminal state before freeze.
    fn from_exact_owner(mut owner: DetachedRetainedPart<T>) -> Self {
        Self {
            allocation: owner.allocation.take(),
            account: owner.account.take(),
        }
    }

    fn allocation(&self) -> &RetainedAllocation<T> {
        match self.allocation.as_ref() {
            Some(allocation) => allocation,
            None => unreachable_final_owner_state(),
        }
    }

    fn account(&self) -> &PhysicalStorageAccount {
        match self.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }
}

impl<T> Drop for PublishedPart<T> {
    fn drop(&mut self) {
        drop(self.allocation.take());
        drop(self.account.take());
    }
}

#[allow(
    dead_code,
    clippy::type_complexity,
    reason = "the baseline keeps payload, evidence, and typed batches under one owner"
)]
struct FallbackPublicationBaseline<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fallback: Option<DetachedRetainedPart<F>>,
    evidence: Option<DetachedRetainedPart<PublicationEvidenceCells<D>>>,
    batches: Option<FallbackBatchBaseline<O, P, D>>,
}

#[allow(
    dead_code,
    clippy::type_complexity,
    reason = "the baseline returns exact destination-qualified parts without erasure"
)]
impl<O, P, D, F> FallbackPublicationBaseline<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn account(&self) -> &PhysicalStorageAccount {
        match self.evidence.as_ref() {
            Some(evidence) => evidence.account(),
            None => unreachable_final_owner_state(),
        }
    }

    fn into_parts(
        mut self,
    ) -> (
        DetachedRetainedPart<F>,
        DetachedRetainedPart<PublicationEvidenceCells<D>>,
        FallbackBatchBaseline<O, P, D>,
    ) {
        let fallback = match self.fallback.take() {
            Some(fallback) => fallback,
            None => unreachable_final_owner_state(),
        };
        let evidence = match self.evidence.take() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        };
        let batches = match self.batches.take() {
            Some(batches) => batches,
            None => unreachable_final_owner_state(),
        };
        (fallback, evidence, batches)
    }

    fn first_account(&self) -> Option<&PhysicalStorageAccount> {
        self.evidence
            .as_ref()
            .and_then(|part| part.account.as_ref())
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(|part| part.account.as_ref())
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(|part| part.allocation.as_ref())
                    .map(RetainedAllocation::account)
            })
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(|part| part.allocation.as_ref())
                    .map(RetainedAllocation::account)
            })
    }

    fn has_exact_product_graph(&self) -> bool {
        let (Some(fallback), Some(evidence), Some(_batches)) = (
            self.fallback.as_ref(),
            self.evidence.as_ref(),
            self.batches.as_ref(),
        ) else {
            return false;
        };
        fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(evidence.account())
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(mut fallback) = self.fallback.take() {
            fallback.fail_closed_after_recorded_poison();
            drop(fallback);
        }
        if let Some(mut evidence) = self.evidence.take() {
            evidence.fail_closed_after_recorded_poison();
            drop(evidence);
        }
        if let Some(batches) = self.batches.take() {
            std::mem::forget(batches);
        }
    }

    fn record_forgotten_physical_edges(&self) {
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.record_forgotten_physical_edge();
        }
        if let Some(evidence) = self.evidence.as_ref() {
            evidence.record_forgotten_physical_edge();
        }
    }

    fn contain_after_recorded_poison(&mut self) {
        self.record_forgotten_physical_edges();
        self.fail_closed_after_recorded_poison();
    }

    fn new_fault_and_fail_closed(&mut self, reason: AccountPoisonReason) {
        if let Some(account) = self.first_account() {
            let _ = account.record_poison(reason);
        }
        self.contain_after_recorded_poison();
    }

    fn try_discard_all(&mut self) -> Result<(), ()> {
        if !self.has_exact_product_graph() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(());
        }

        let fallback_result = match self.fallback.as_mut() {
            Some(fallback) => fallback.try_discard_allocation(),
            None => Err(RetainedPartCleanupError::MissingFaultAnchor),
        };
        let fallback_account = match fallback_result {
            Ok(account) => account,
            Err(cause) => {
                if cause == RetainedPartCleanupError::MissingFaultAnchor
                    && let Some(account) = self.first_account()
                {
                    let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                }
                self.contain_after_recorded_poison();
                return Err(());
            }
        };

        let evidence_result = match self.evidence.as_mut() {
            Some(evidence) => evidence.try_discard_allocation(),
            None => Err(RetainedPartCleanupError::MissingFaultAnchor),
        };
        let evidence_account = match evidence_result {
            Ok(account) => account,
            Err(cause) => {
                if cause == RetainedPartCleanupError::MissingFaultAnchor {
                    let _ = fallback_account
                        .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                }
                drop(fallback_account);
                self.contain_after_recorded_poison();
                return Err(());
            }
        };

        drop(self.batches.take());
        drop(fallback_account);
        drop(evidence_account);
        drop(self.fallback.take());
        drop(self.evidence.take());
        Ok(())
    }

    fn discard_rejected(mut self) -> Result<(), ()> {
        self.try_discard_all()
    }
}

impl<O, P, D, F> Drop for FallbackPublicationBaseline<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if self.fallback.is_none() && self.evidence.is_none() && self.batches.is_none() {
            return;
        }
        if std::thread::panicking() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return;
        }
        let _ = self.try_discard_all();
    }
}

struct StartupFallbackOwner<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fallback: Option<RetainedPart<F, OutcomeEvidenceDestination, O, P, D>>,
    evidence:
        Option<RetainedPart<PublicationEvidenceCells<D>, OutcomeEvidenceDestination, O, P, D>>,
    canonical_assembly: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_assembly: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
}

impl<O, P, D, F> StartupFallbackOwner<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn from_complete(
        fallback: RetainedPart<F, OutcomeEvidenceDestination, O, P, D>,
        evidence: RetainedPart<PublicationEvidenceCells<D>, OutcomeEvidenceDestination, O, P, D>,
        canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
        evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    ) -> Self {
        Self {
            fallback: Some(fallback),
            evidence: Some(evidence),
            canonical_assembly: Some(canonical_assembly),
            evidence_assembly: Some(evidence_assembly),
        }
    }

    fn first_account(&self) -> Option<&PhysicalStorageAccount> {
        self.fallback
            .as_ref()
            .and_then(|part| part.account.as_ref())
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(|part| part.account.as_ref())
            })
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(|part| part.allocation.as_ref())
                    .map(RetainedAllocation::account)
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(|part| part.allocation.as_ref())
                    .map(RetainedAllocation::account)
            })
    }

    fn record_forgotten_physical_edges(&self) {
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.record_forgotten_physical_edges();
        }
        if let Some(evidence) = self.evidence.as_ref() {
            evidence.record_forgotten_physical_edges();
        }
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(mut fallback) = self.fallback.take() {
            fallback.fail_closed_after_recorded_poison();
            drop(fallback);
        }
        if let Some(mut evidence) = self.evidence.take() {
            evidence.fail_closed_after_recorded_poison();
            drop(evidence);
        }
        if let Some(batch) = self.canonical_assembly.take() {
            std::mem::forget(batch);
        }
        if let Some(batch) = self.evidence_assembly.take() {
            std::mem::forget(batch);
        }
    }

    fn contain_after_recorded_poison(&mut self) {
        self.record_forgotten_physical_edges();
        self.fail_closed_after_recorded_poison();
    }

    fn new_fault_and_fail_closed(&mut self, reason: AccountPoisonReason) {
        if let Some(account) = self.first_account() {
            let _ = account.record_poison(reason);
        }
        self.contain_after_recorded_poison();
    }

    fn try_discard_all(&mut self) -> Result<(), ()> {
        let fallback_result = match self.fallback.as_mut() {
            Some(fallback) => fallback.try_discard_allocation_before_batch_release(),
            None => Err(RetainedPartCleanupError::MissingFaultAnchor),
        };
        let fallback_account = match fallback_result {
            Ok(account) => account,
            Err(cause) => {
                if cause == RetainedPartCleanupError::MissingFaultAnchor
                    && let Some(account) = self.first_account()
                {
                    let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                }
                self.contain_after_recorded_poison();
                return Err(());
            }
        };

        let evidence_account = match self.evidence.as_mut() {
            Some(evidence) => match evidence.try_discard_allocation_before_batch_release() {
                Ok(account) => Some(account),
                Err(cause) => {
                    if cause == RetainedPartCleanupError::MissingFaultAnchor {
                        let _ = fallback_account
                            .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                    }
                    drop(fallback_account);
                    self.contain_after_recorded_poison();
                    return Err(());
                }
            },
            None => None,
        };

        if let Some(fallback) = self.fallback.as_mut() {
            drop(fallback.batch.take());
        }
        if let Some(evidence) = self.evidence.as_mut() {
            drop(evidence.batch.take());
        }
        drop(self.canonical_assembly.take());
        drop(self.evidence_assembly.take());
        drop(fallback_account);
        drop(evidence_account);
        drop(self.fallback.take());
        drop(self.evidence.take());
        Ok(())
    }

    fn discard_rejected(mut self) -> Result<(), ()> {
        self.try_discard_all()
    }

    fn try_into_baseline(
        mut self,
    ) -> Result<FallbackPublicationBaseline<O, P, D, F>, RetainedWriterError<D>> {
        let exact = matches!(
            (self.fallback.as_ref(), self.evidence.as_ref()),
            (Some(fallback), Some(evidence))
                if fallback.has_exact_product_graph()
                    && evidence.has_exact_product_graph()
                    && fallback.account().is_same_account(evidence.account())
        ) && self.canonical_assembly.is_some()
            && self.evidence_assembly.is_some();
        if !exact {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedWriterError::StorageDropUnverified);
        }

        let fallback = match self.fallback.take() {
            Some(fallback) => fallback,
            None => unreachable_final_owner_state(),
        };
        let fallback = match fallback.try_into_exact() {
            Ok(fallback) => fallback,
            Err(fallback) => {
                self.fallback = Some(fallback);
                self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedWriterError::StorageDropUnverified);
            }
        };
        let evidence = match self.evidence.take() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        };
        let evidence = match evidence.try_into_exact() {
            Ok(evidence) => evidence,
            Err(evidence) => {
                self.evidence = Some(evidence);
                self.fallback = Some(fallback.into_inner());
                self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedWriterError::StorageDropUnverified);
            }
        };
        let canonical_assembly = match self.canonical_assembly.take() {
            Some(batch) => batch,
            None => {
                self.fallback = Some(fallback.into_inner());
                self.evidence = Some(evidence.into_inner());
                self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedWriterError::StorageDropUnverified);
            }
        };
        let evidence_assembly = match self.evidence_assembly.take() {
            Some(batch) => batch,
            None => {
                self.canonical_assembly = Some(canonical_assembly);
                self.fallback = Some(fallback.into_inner());
                self.evidence = Some(evidence.into_inner());
                self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedWriterError::StorageDropUnverified);
            }
        };
        let (fallback_batch, fallback) = fallback.into_detached();
        let (evidence_batch, evidence) = evidence.into_detached();
        let batches = FallbackBatchBaseline::new_unbound(
            canonical_assembly,
            evidence_assembly,
            fallback_batch,
            evidence_batch,
        );
        Ok(FallbackPublicationBaseline {
            fallback: Some(fallback),
            evidence: Some(evidence),
            batches: Some(batches),
        })
    }
}

impl<O, P, D, F> Drop for StartupFallbackOwner<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if self.fallback.is_none()
            && self.evidence.is_none()
            && self.canonical_assembly.is_none()
            && self.evidence_assembly.is_none()
        {
            return;
        }
        if std::thread::panicking() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return;
        }
        let _ = self.try_discard_all();
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
        RetainedPart {
            allocation: Some(allocation),
            batch: Some(batch),
            account: Some(account),
        }
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
            part.record_forgotten_physical_edges();
            part.fail_closed_after_recorded_poison();
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
        match part.try_discard_allocation_before_batch_release_with_policy(defer_ownership_fault) {
            Ok(account) => {
                drop(part.batch.take());
                drop(account);
                drop(part);
                Ok(())
            }
            Err(cause) => {
                part.fail_closed_after_recorded_poison();
                drop(part);
                Err(cause)
            }
        }
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
                part.record_forgotten_physical_edges();
                part.fail_closed_after_recorded_poison();
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
            drop(part.account.take());
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
                    .and_then(|part| part.account.as_ref())
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(|part| part.account.as_ref())
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
                    .and_then(|part| part.allocation.as_ref())
                    .map(RetainedAllocation::account)
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(|part| part.allocation.as_ref())
                    .map(RetainedAllocation::account)
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
                DetachedRetainedPart {
                    allocation: None,
                    account: None,
                },
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
                DetachedRetainedPart {
                    allocation: None,
                    account: None,
                },
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
    pub fn try_start(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
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
        let coordinator = OperationCoordinator::new(control, ledger_limits);
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
        let mut baseline = startup_owner
            .try_into_baseline()
            .map_err(RetainedOperationStartError::Writer)?;
        let bind_result = catch_unwind(AssertUnwindSafe(|| {
            let batches = match baseline.batches.as_mut() {
                Some(batches) => batches,
                None => unreachable_final_owner_state(),
            };
            ledger.try_bind_fallback_baseline(batches)
        }));
        match bind_result {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                baseline.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedOperationStartError::Writer(
                    RetainedWriterError::StorageDropUnverified,
                ));
            }
            Ok(Err(_cause)) => {
                let poisoned = baseline
                    .account()
                    .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedOperationStartError::Writer(
                    if baseline.discard_rejected().is_ok() {
                        RetainedWriterError::AccountPoisoned(poisoned)
                    } else {
                        RetainedWriterError::StorageDropUnverified
                    },
                ));
            }
            Ok(Ok(())) => {}
        }

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
mod tests {
    use super::*;
    use crate::DimensionPolicy;
    use crate::ledger::LedgerTestProbe;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc as StdArc, Barrier, Mutex as StdMutex};
    use std::thread;

    const BUILD_FAILED: RetainedBuildFailureCode = RetainedBuildFailureCode::new(1);

    fn retained_root_production_source() -> &'static str {
        let source = include_str!("retained.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();
        let (production, test_module) = source
            .rsplit_once(&test_attribute)
            .expect("retained source contains one final test module boundary");
        assert!(test_module.trim_start().starts_with("mod tests"));
        production
    }

    fn retained_production_source() -> String {
        [
            retained_root_production_source(),
            include_str!("retained/account.rs"),
            include_str!("retained/final_owner.rs"),
        ]
        .join("\n")
    }

    struct PanickingDropProbe(&'static AtomicU64);

    impl Drop for PanickingDropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    struct DropCounter(&'static AtomicU64);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct LiveDropObservation {
        ledger: LedgerTestProbe<TestPhase, TestDimension>,
        account: PhysicalStorageAccount,
        observed_active: &'static AtomicU64,
        observed_operation_live: &'static AtomicU64,
        observed_physical_bytes: &'static AtomicU64,
        observed_physical_allocations: &'static AtomicU64,
    }

    impl Drop for LiveDropObservation {
        fn drop(&mut self) {
            self.observed_active
                .store(self.ledger.active_reservations(), Ordering::SeqCst);
            self.observed_operation_live
                .store(self.ledger.current_live_bytes(), Ordering::SeqCst);
            let physical = self.account.snapshot();
            self.observed_physical_bytes
                .store(physical.live_retained_bytes(), Ordering::SeqCst);
            self.observed_physical_allocations
                .store(physical.live_retained_allocations(), Ordering::SeqCst);
        }
    }

    struct DeferredLiveDropObservation {
        ledger: StdArc<StdMutex<Option<LedgerTestProbe<TestPhase, TestDimension>>>>,
        account: PhysicalStorageAccount,
        observed_active: &'static AtomicU64,
        observed_operation_live: &'static AtomicU64,
        observed_physical_bytes: &'static AtomicU64,
        observed_physical_allocations: &'static AtomicU64,
    }

    impl Drop for DeferredLiveDropObservation {
        fn drop(&mut self) {
            let ledger = lock_recovering_poison(&self.ledger);
            let ledger = ledger
                .as_ref()
                .expect("baseline drop probe must be installed before destruction");
            self.observed_active
                .store(ledger.active_reservations(), Ordering::SeqCst);
            self.observed_operation_live
                .store(ledger.current_live_bytes(), Ordering::SeqCst);
            let physical = self.account.snapshot();
            self.observed_physical_bytes
                .store(physical.live_retained_bytes(), Ordering::SeqCst);
            self.observed_physical_allocations
                .store(physical.live_retained_allocations(), Ordering::SeqCst);
        }
    }

    struct PhysicalDropObservation {
        account: PhysicalStorageAccount,
        drop_count: &'static AtomicU64,
        observed_bytes: &'static AtomicU64,
        observed_allocations: &'static AtomicU64,
    }

    impl Drop for PhysicalDropObservation {
        fn drop(&mut self) {
            self.drop_count.fetch_add(1, Ordering::SeqCst);
            let physical = self.account.snapshot();
            self.observed_bytes
                .store(physical.live_retained_bytes(), Ordering::SeqCst);
            self.observed_allocations
                .store(physical.live_retained_allocations(), Ordering::SeqCst);
        }
    }

    struct TestOwner;

    impl OperationOwner for TestOwner {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestPhase {
        Build,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestProgress {
        Build,
    }

    impl ProgressPhase for TestProgress {
        fn order(self) -> u32 {
            0
        }

        fn stable_id(self) -> &'static str {
            "build"
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestUnit {
        Items,
    }

    impl ProgressUnit for TestUnit {
        fn stable_id(self) -> &'static str {
            "items"
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestDimension {
        Bytes,
    }

    static PANIC_ON_PRIMARY_DIMENSION_ORD: AtomicBool = AtomicBool::new(false);
    static PRIMARY_DIMENSION_ORD_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    struct PrimaryDimensionOrdReset;

    impl Drop for PrimaryDimensionOrdReset {
        fn drop(&mut self) {
            PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct PrimaryPanicDimension(u8);

    impl PartialOrd for PrimaryPanicDimension {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for PrimaryPanicDimension {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            assert!(
                !PANIC_ON_PRIMARY_DIMENSION_ORD.load(Ordering::SeqCst),
                "primary bind invoked owner-defined dimension ordering"
            );
            self.0.cmp(&other.0)
        }
    }

    struct PrimaryBindDropObservation {
        ledger: StdArc<StdMutex<Option<LedgerTestProbe<TestPhase, PrimaryPanicDimension>>>>,
        drop_count: &'static AtomicU64,
        observed_active: &'static AtomicU64,
        observed_live: &'static AtomicU64,
    }

    impl Drop for PrimaryBindDropObservation {
        fn drop(&mut self) {
            let ledger = lock_recovering_poison(&self.ledger);
            let ledger = ledger
                .as_ref()
                .expect("primary bind probe must remain installed through storage drop");
            self.drop_count.fetch_add(1, Ordering::SeqCst);
            self.observed_active
                .store(ledger.active_reservations(), Ordering::SeqCst);
            self.observed_live
                .store(ledger.current_live_bytes(), Ordering::SeqCst);
        }
    }

    #[derive(Clone, Copy)]
    struct CanonicalRootTag;

    #[derive(Clone, Copy)]
    struct CanonicalLeafTag;

    #[derive(Clone, Copy)]
    struct EvidenceRootTag;

    #[derive(Clone, Copy)]
    struct EvidenceDetailTag;

    #[derive(Debug, PartialEq, Eq)]
    enum TestCanonicalBlock {
        Root {
            leaf: CanonicalRetainedSlot<TestCanonicalBlock, CanonicalLeafTag>,
        },
        Leaf(u64),
    }

    #[derive(Debug, PartialEq, Eq)]
    enum TestEvidenceBlock {
        Root {
            detail: OutcomeEvidenceRetainedSlot<TestEvidenceBlock, EvidenceDetailTag>,
        },
        Detail(u64),
    }

    type TestOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, u32>;
    type TestPublication<CanonicalBlock, EvidenceBlock> = RetainedPublicationAssembly<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        u32,
        CanonicalBlock,
        EvidenceBlock,
    >;

    enum PublishedRetainedPublicationParts<CanonicalBlock, EvidenceBlock, F, D> {
        Success {
            canonical: PublishedCanonicalOutput<CanonicalBlock>,
            outcome_evidence: PublishedOutcomeEvidenceOutput<EvidenceBlock>,
            ledger_evidence: PublishedLedgerEvidence<D>,
        },
        RichFailure {
            outcome_evidence: PublishedRichFailureOutput<EvidenceBlock>,
            ledger_evidence: PublishedLedgerEvidence<D>,
        },
        MinimalFallback {
            fallback: PublishedFallbackOutput<F>,
            ledger_evidence: PublishedLedgerEvidence<D>,
        },
    }

    fn handoff_for_test<CanonicalBlock, EvidenceBlock, F, D>(
        prepared: PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
    ) -> PublishedRetainedPublicationParts<CanonicalBlock, EvidenceBlock, F, D>
    where
        D: Copy + Ord,
    {
        match prepared.into_outcome() {
            PreparedRetainedPublicationOutcome::Success(prepared) => {
                let published = prepared.handoff();
                PublishedRetainedPublicationParts::Success {
                    canonical: published.canonical,
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                }
            }
            PreparedRetainedPublicationOutcome::RichFailure(prepared) => {
                let published = prepared.handoff();
                PublishedRetainedPublicationParts::RichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                }
            }
            PreparedRetainedPublicationOutcome::MinimalFallback(prepared) => {
                let published = prepared.handoff();
                PublishedRetainedPublicationParts::MinimalFallback {
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                }
            }
        }
    }

    fn share_work() -> RetainedHandleSharePlan<TestDimension> {
        RetainedHandleSharePlan::new(TestDimension::Bytes, TestDimension::Bytes)
    }

    fn writer_plan(
        attributions: usize,
        commitments: usize,
    ) -> RetainedWriterPlan<TestPhase, TestDimension> {
        RetainedWriterPlan::new(
            TestPhase::Build,
            TestDimension::Bytes,
            TestDimension::Bytes,
            RetainedBatchLimits::new(attributions, commitments),
        )
    }

    fn publication_plan(
        canonical_blocks: usize,
        evidence_blocks: usize,
    ) -> RetainedPublicationPlan<TestPhase, TestDimension> {
        RetainedPublicationPlan::success(
            RetainedBlockTablePlan::new(
                writer_plan(canonical_blocks + 1, 2),
                canonical_blocks,
                canonical_blocks,
            ),
            RetainedBlockTablePlan::new(
                writer_plan(evidence_blocks + 1, 2),
                evidence_blocks,
                evidence_blocks,
            ),
        )
    }

    fn test_operation(
        factory: &PhysicalStorageAccountFactory,
        evidence_limit: u64,
    ) -> TestOperation {
        test_operation_with_writer_limits(factory, evidence_limit, RetainedBatchLimits::new(2, 2))
    }

    fn test_operation_with_writer_limits(
        factory: &PhysicalStorageAccountFactory,
        evidence_limit: u64,
        writer_limits: RetainedBatchLimits,
    ) -> TestOperation {
        let account = account(factory, u64::MAX, 128);
        test_operation_on_account_with_control(
            &account,
            evidence_limit,
            writer_limits,
            OperationControl::default(),
        )
    }

    fn test_operation_on_account_with_control(
        account: &PhysicalStorageAccount,
        evidence_limit: u64,
        writer_limits: RetainedBatchLimits,
        control: OperationControl<TestProgress, TestUnit>,
    ) -> TestOperation {
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(evidence_limit);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let plan = RetainedOperationStartPlan::new(
            writer_plan(
                writer_limits.attribution_slots(),
                writer_limits.commitment_slots(),
            ),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        match TestOperation::try_start(control, limits, account, plan, || Ok(7_u32)) {
            Ok(operation) => operation,
            Err(_) => panic!("test operation bootstrap must succeed"),
        }
    }

    fn test_operation_with_fallback<F>(
        factory: &PhysicalStorageAccountFactory,
        fallback: F,
    ) -> RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, F>
    where
        F: Send + Sync + 'static,
    {
        let account = account(factory, u64::MAX, 128);
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let plan = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        RetainedOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok(fallback)
        })
        .unwrap_or_else(|_| panic!("test operation bootstrap must succeed"))
    }

    fn final_progress() -> FinalProgress<TestProgress, TestUnit> {
        FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
    }

    fn account_factory(max_live_accounts: u64) -> PhysicalStorageAccountFactory {
        let limits = PhysicalStorageAccountFactoryLimits::new(u64::MAX, max_live_accounts).unwrap();
        PhysicalStorageAccountFactory::try_new(limits).unwrap()
    }

    fn account(
        factory: &PhysicalStorageAccountFactory,
        bytes: u64,
        allocations: u64,
    ) -> PhysicalStorageAccount {
        factory
            .try_create_account(PhysicalStorageAccountLimits::new(bytes, allocations).unwrap())
            .unwrap()
    }

    fn admitted_startup_share_work(account: &PhysicalStorageAccount) -> StartupAccountOwners {
        let share = || PhysicalStorageAccount {
            inner: account.inner.share(),
        };
        StartupAccountOwners {
            fallback_account: share(),
            evidence_account: share(),
            fallback_lease_owner: share(),
            evidence_lease_owner: share(),
        }
    }

    fn startup_limits_and_plan() -> (
        LedgerLimits<TestDimension>,
        RetainedOperationStartPlan<TestPhase, TestDimension>,
    ) {
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)])
            .unwrap_or_else(|_| panic!("startup test limits must qualify"));
        let plan = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        (limits, plan)
    }

    fn reset_startup_test_observations() {
        let _ = take_injected_start_evidence_build_failure();
        let _ = take_injected_start_evidence_build_panic();
        let _ = take_injected_start_fallback_account_anchor_loss();
        let _ = take_start_evidence_build_calls();
        let _ = take_startup_failure_ledger_snapshot();
    }

    fn empty_plan() -> RetainedAllocationPlan {
        RetainedAllocationPlan::new(0, 0).unwrap()
    }

    fn retain_scalar_success(
        assembly: &mut TestPublication<u64, u64>,
    ) -> (
        CanonicalRetainedSlot<u64, CanonicalRootTag>,
        OutcomeEvidenceRetainedSlot<u64, EvidenceRootTag>,
    ) {
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(11, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("canonical scalar must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence scalar must retain"));
        (canonical, evidence)
    }

    type PublishedScalarSource = (
        PublishedCanonicalOutput<u64>,
        CanonicalRetainedSlot<u64, CanonicalRootTag>,
        PublishedOutcomeEvidenceOutput<u64>,
        OutcomeEvidenceRetainedSlot<u64, EvidenceRootTag>,
    );

    fn publish_scalar_source(account: &PhysicalStorageAccount) -> PublishedScalarSource {
        let operation = test_operation_on_account_with_control(
            account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("source publication must open"));
        let (canonical_slot, evidence_slot) = retain_scalar_success(&mut assembly);
        let validated = match assembly.validate_success(
            canonical_slot,
            evidence_slot,
            |session, canonical, evidence| {
                session.canonical(canonical) == Some(&11)
                    && session.outcome_evidence(evidence) == Some(&7)
            },
        ) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("source publication must validate"),
        };
        let published = handoff_for_test(
            validated
                .try_prepare(final_progress())
                .unwrap_or_else(|_| panic!("source publication must prepare")),
        );
        match published {
            PublishedRetainedPublicationParts::Success {
                canonical,
                outcome_evidence,
                ledger_evidence,
            } => {
                drop(ledger_evidence);
                (canonical, canonical_slot, outcome_evidence, evidence_slot)
            }
            _ => panic!("source publication must succeed"),
        }
    }

    #[test]
    fn block_table_success_freezes_dual_batches_and_splits_owners() {
        let factory = account_factory(1);
        let operation =
            test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
        let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
            .try_publication_assembly(publication_plan(2, 2))
            .unwrap_or_else(|_| panic!("publication assembly must open"));
        let leaf = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestCanonicalBlock::Leaf(11), CanonicalLeafTag))
            })
            .unwrap_or_else(|_| panic!("canonical leaf must retain"));
        let canonical_root = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestCanonicalBlock::Root { leaf }, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("canonical root must retain"));
        let detail = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestEvidenceBlock::Detail(7), EvidenceDetailTag))
            })
            .unwrap_or_else(|_| panic!("evidence detail must retain"));
        let evidence_root = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestEvidenceBlock::Root { detail }, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence root must retain"));

        assert_eq!(assembly.canonical_len(), 2);
        assert_eq!(assembly.outcome_evidence_len(), 2);
        assembly
            .try_commit_canonical(TestDimension::Bytes, 5)
            .unwrap_or_else(|_| panic!("canonical commitment must fit"));
        assembly
            .try_commit_outcome_evidence(TestDimension::Bytes, 6)
            .unwrap_or_else(|_| panic!("evidence commitment must fit"));

        let validated = match assembly.validate_success(
            canonical_root,
            evidence_root,
            |session, canonical_root, evidence_root| {
                let Some(TestCanonicalBlock::Root { leaf }) = session.canonical(canonical_root)
                else {
                    return false;
                };
                if !matches!(session.canonical(*leaf), Some(TestCanonicalBlock::Leaf(11))) {
                    return false;
                }
                // A DAG may reach the same slot more than once; only unique members are counted.
                if !matches!(session.canonical(*leaf), Some(TestCanonicalBlock::Leaf(11))) {
                    return false;
                }
                let Some(TestEvidenceBlock::Root { detail }) =
                    session.outcome_evidence(evidence_root)
                else {
                    return false;
                };
                matches!(
                    session.outcome_evidence(*detail),
                    Some(TestEvidenceBlock::Detail(7))
                )
            },
        ) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("dual publication graph must validate"),
        };
        let prepared = validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("dual publication must prepare"));
        match handoff_for_test(prepared) {
            PublishedRetainedPublicationParts::Success {
                canonical,
                outcome_evidence,
                ledger_evidence,
            } => {
                assert!(matches!(canonical.root(), TestCanonicalBlock::Root { .. }));
                assert!(matches!(
                    outcome_evidence.root(),
                    TestEvidenceBlock::Root { .. }
                ));
                let snapshot = ledger_evidence
                    .dimension(TestDimension::Bytes)
                    .expect("published dimension evidence");
                assert_eq!(snapshot.committed_canonical_output(), 5);
                assert_eq!(snapshot.published_outcome_evidence(), 7);
            }
            _ => panic!("success must not publish a failure owner"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn inherited_blocks_use_fresh_slots_without_new_attribution_or_physical_charge() {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let (source_canonical, source_canonical_slot, source_evidence, source_evidence_slot) =
            publish_scalar_source(&account);
        let canonical_shares_before = source_canonical.aggregate().inner.table.as_ref().members[0]
            .allocation
            .share_call_count();
        let evidence_shares_before = source_evidence.aggregate().inner.table.as_ref().members[0]
            .allocation
            .share_call_count();

        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(3, 2),
            OperationControl::default(),
        );
        let plan = RetainedPublicationPlan::success(
            RetainedBlockTablePlan::new(writer_plan(2, 2), 3, 1),
            RetainedBlockTablePlan::new(writer_plan(1, 2), 3, 0),
        );
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(plan)
            .unwrap_or_else(|_| panic!("inherited publication must open"));
        let physical_before_shares = account.snapshot();

        let canonical_root = assembly
            .try_share_canonical(source_canonical.aggregate(), source_canonical_slot)
            .unwrap_or_else(|_| panic!("first canonical share must succeed"));
        let canonical_second = assembly
            .try_share_canonical(source_canonical.aggregate(), source_canonical_slot)
            .unwrap_or_else(|_| panic!("second canonical share must succeed"));
        let evidence_root = assembly
            .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
            .unwrap_or_else(|_| panic!("first evidence share must succeed"));
        let evidence_second = assembly
            .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
            .unwrap_or_else(|_| panic!("second evidence share must succeed"));
        let evidence_third = assembly
            .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
            .unwrap_or_else(|_| panic!("third evidence share must succeed"));

        assert_eq!(account.snapshot(), physical_before_shares);
        assert_eq!(assembly.canonical_len(), 2);
        assert_eq!(assembly.outcome_evidence_len(), 3);
        assert_eq!(
            assembly
                .canonical_batch
                .as_ref()
                .expect("canonical attribution batch")
                .attribution_len(),
            1
        );
        assert_eq!(
            assembly
                .evidence_batch
                .as_ref()
                .expect("evidence attribution batch")
                .attribution_len(),
            1
        );
        assert_eq!(
            source_canonical.aggregate().inner.table.as_ref().members[0]
                .allocation
                .share_call_count(),
            canonical_shares_before + 2
        );
        assert_eq!(
            source_evidence.aggregate().inner.table.as_ref().members[0]
                .allocation
                .share_call_count(),
            evidence_shares_before + 3
        );
        assert!(source_canonical.aggregate().get(canonical_root).is_none());
        assert!(source_evidence.aggregate().get(evidence_root).is_none());

        let canonical_new = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(13, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("one new canonical block must fit"));
        assert_eq!(assembly.canonical_len(), 3);
        assert_eq!(
            assembly
                .canonical_batch
                .as_ref()
                .expect("canonical attribution batch")
                .attribution_len(),
            2
        );
        assert!(
            account.snapshot().live_retained_bytes() > physical_before_shares.live_retained_bytes()
        );

        let validated = match assembly.validate_success(
            canonical_root,
            evidence_root,
            |session, canonical_root, evidence_root| {
                session.canonical(canonical_root) == Some(&11)
                    && session.canonical(canonical_second) == Some(&11)
                    && session.canonical(canonical_new) == Some(&13)
                    && session.outcome_evidence(evidence_root) == Some(&7)
                    && session.outcome_evidence(evidence_second) == Some(&7)
                    && session.outcome_evidence(evidence_third) == Some(&7)
            },
        ) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("inherited publication must validate"),
        };
        let published = handoff_for_test(
            validated
                .try_prepare(final_progress())
                .unwrap_or_else(|_| panic!("inherited publication must prepare")),
        );
        match &published {
            PublishedRetainedPublicationParts::Success {
                canonical,
                outcome_evidence,
                ..
            } => {
                assert_eq!(canonical.aggregate().get(canonical_root), Some(&11));
                assert!(canonical.aggregate().get(source_canonical_slot).is_none());
                assert_eq!(outcome_evidence.aggregate().get(evidence_root), Some(&7));
                assert!(
                    outcome_evidence
                        .aggregate()
                        .get(source_evidence_slot)
                        .is_none()
                );
            }
            _ => panic!("inherited publication must succeed"),
        }
        drop(published);
        drop(source_canonical);
        drop(source_evidence);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn foreign_aggregate_is_rejected_before_member_access_or_reference_clone() {
        let factory = account_factory(2);
        let source_account = account(&factory, u64::MAX, 128);
        let target_account = account(&factory, u64::MAX, 128);
        let (source_canonical, source_slot, source_evidence, _) =
            publish_scalar_source(&source_account);
        let table_shares_before = source_canonical.aggregate().inner.table.share_call_count();
        let block_shares_before = source_canonical.aggregate().inner.table.as_ref().members[0]
            .allocation
            .share_call_count();
        let _ = take_test_member_lookups();

        let operation = test_operation_on_account_with_control(
            &target_account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("foreign-account target must open"));
        let result = assembly.try_share_canonical(source_canonical.aggregate(), source_slot);

        assert!(matches!(
            result,
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::ForeignAccount
            ))
        ));
        assert_eq!(take_test_member_lookups(), 0);
        assert_eq!(assembly.canonical_len(), 0);
        assert_eq!(
            source_canonical.aggregate().inner.table.share_call_count(),
            table_shares_before
        );
        assert_eq!(
            source_canonical.aggregate().inner.table.as_ref().members[0]
                .allocation
                .share_call_count(),
            block_shares_before
        );
        drop(handoff_for_test(
            assembly
                .prepare_minimal_fallback()
                .unwrap_or_else(|_| panic!("foreign rejection must preserve fallback")),
        ));
        drop(source_canonical);
        drop(source_evidence);
        drop(source_account);
        drop(target_account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn share_work_rejection_does_not_clone_or_mutate_the_target_table() {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let (source_canonical, source_slot, source_evidence, _) = publish_scalar_source(&account);
        let block_shares_before = source_canonical.aggregate().inner.table.as_ref().members[0]
            .allocation
            .share_call_count();
        let physical_before_operation = account.snapshot();
        let policy = DimensionPolicy::reservable_bytes(15, 15, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let start = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        let operation =
            TestOperation::try_start(OperationControl::default(), limits, &account, start, || {
                Ok(7_u32)
            })
            .unwrap_or_else(|_| panic!("exact share-work operation must start"));
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("exact share-work tables must open"));
        let physical_before_share = account.snapshot();

        let result = assembly.try_share_canonical(source_canonical.aggregate(), source_slot);

        assert!(matches!(
            result,
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
            ))
        ));
        assert_eq!(assembly.canonical_len(), 0);
        assert_eq!(account.snapshot(), physical_before_share);
        assert!(
            physical_before_share.live_retained_bytes()
                > physical_before_operation.live_retained_bytes()
        );
        assert_eq!(
            source_canonical.aggregate().inner.table.as_ref().members[0]
                .allocation
                .share_call_count(),
            block_shares_before
        );
        drop(handoff_for_test(
            assembly
                .prepare_minimal_fallback()
                .unwrap_or_else(|_| panic!("share-work rejection must preserve fallback")),
        ));
        drop(source_canonical);
        drop(source_evidence);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn invalid_publication_capacity_is_rejected_before_every_open_side_effect() {
        let factory = account_factory(1);
        let invalid_plans = [
            (
                "attribution capacity overflow",
                RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                    writer_plan(usize::MAX, 1),
                    1,
                    0,
                )),
                false,
            ),
            (
                "commitment capacity overflow",
                RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                    writer_plan(1, usize::MAX),
                    1,
                    0,
                )),
                false,
            ),
            (
                "member capacity overflow",
                RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                    writer_plan(1, 1),
                    usize::MAX,
                    0,
                )),
                true,
            ),
            (
                "new block limit exceeds total blocks",
                RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                    writer_plan(2, 1),
                    1,
                    2,
                )),
                false,
            ),
        ];

        for (index, (case, invalid_plan, expects_batch_capacity)) in
            invalid_plans.into_iter().enumerate()
        {
            let operation = test_operation(&factory, 64);
            let dimension_before = operation
                .coordinator
                .dimension_evidence(TestDimension::Bytes)
                .unwrap_or_else(|_| panic!("{case}: dimension evidence before open"));
            let physical_before = operation.account().snapshot();
            let account_shares_before = operation.account().share_call_count();
            let set_ids_before = test_retained_set_id_requests();
            let batch_allocations_before =
                crate::ledger::test_attribution_batch_allocation_entries();

            let error = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
                Ok(_) => panic!("{case}: invalid plan unexpectedly opened"),
                Err(error) => error,
            };
            assert!(
                if expects_batch_capacity {
                    matches!(error.cause(), RetainedWriterError::BatchCapacity)
                } else {
                    matches!(error.cause(), RetainedWriterError::InvalidPublicationPlan)
                },
                "{case}: unexpected preflight cause: {}",
                error.cause()
            );
            let operation = error
                .try_into_operation()
                .unwrap_or_else(|_| panic!("{case}: pure preflight must return the operation"));

            assert_eq!(test_retained_set_id_requests(), set_ids_before, "{case}");
            assert_eq!(
                crate::ledger::test_attribution_batch_allocation_entries(),
                batch_allocations_before,
                "{case}"
            );
            assert_eq!(
                operation.account().share_call_count(),
                account_shares_before,
                "{case}"
            );
            assert_eq!(operation.account().snapshot(), physical_before, "{case}");
            assert_eq!(
                operation
                    .coordinator
                    .dimension_evidence(TestDimension::Bytes)
                    .unwrap_or_else(|_| panic!("{case}: dimension evidence after rejection")),
                dimension_before,
                "{case}"
            );

            if index == 2 {
                let assembly: TestPublication<u64, u64> = operation
                    .try_publication_assembly(RetainedPublicationPlan::rich_failure(
                        RetainedBlockTablePlan::new(writer_plan(1, 1), 1, 0),
                    ))
                    .unwrap_or_else(|_| panic!("{case}: valid retry must open"));
                drop(handoff_for_test(
                    assembly
                        .prepare_minimal_fallback()
                        .unwrap_or_else(|_| panic!("{case}: retry fallback must prepare")),
                ));
            } else {
                drop(handoff_for_test(
                    operation
                        .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(70))
                        .unwrap_or_else(|_| panic!("direct fallback must prepare")),
                ));
            }
            assert_eq!(factory.snapshot().live_accounts(), 0, "{case}");
        }
    }

    #[test]
    fn recoverable_open_failures_prepare_the_baseline_without_tables_or_roots() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let invalid_plan = RetainedPublicationPlan::success(
            RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 2),
            RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
        );
        let error = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
            Ok(_) => panic!("invalid total/new relation must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error.cause(),
            RetainedWriterError::InvalidPublicationPlan
        ));
        let operation = error
            .try_into_operation()
            .unwrap_or_else(|_| panic!("invalid plan must return the untouched operation"));
        let published = handoff_for_test(
            operation
                .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(71))
                .unwrap_or_else(|_| panic!("direct fallback must prepare")),
        );
        match published {
            PublishedRetainedPublicationParts::MinimalFallback {
                fallback,
                ledger_evidence,
            } => {
                assert_eq!(fallback.cause(), PreparationFailureCode::domain(71));
                assert_eq!(*fallback.value(), 7);
                assert_eq!(
                    ledger_evidence
                        .dimension(TestDimension::Bytes)
                        .expect("direct fallback ledger evidence")
                        .committed_canonical_output(),
                    0
                );
            }
            _ => panic!("invalid plan must publish minimal fallback"),
        }

        let operation = test_operation(&factory, 64);
        fail_next_final_owner_allocation();
        let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
            Ok(_) => panic!("injected table allocation failure must reject open"),
            Err(error) => error,
        };
        assert!(matches!(
            error.cause(),
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
        ));
        let operation = error
            .try_into_operation()
            .unwrap_or_else(|_| panic!("verified allocation failure must return the operation"));
        let published = handoff_for_test(
            operation
                .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(72))
                .unwrap_or_else(|_| panic!("direct fallback must prepare")),
        );
        assert!(matches!(
            &published,
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. }
                if fallback.cause() == PreparationFailureCode::domain(72)
        ));
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn second_table_allocation_failure_releases_canonical_and_returns_operation() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        fail_final_owner_allocation_after(1);

        let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
            Ok(_) => panic!("evidence table allocation failure must reject open"),
            Err(error) => error,
        };
        assert!(matches!(
            error.cause(),
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
        ));
        let operation = error
            .try_into_operation()
            .unwrap_or_else(|_| panic!("clean canonical rollback must return the operation"));
        let published = handoff_for_test(
            operation
                .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(73))
                .unwrap_or_else(|_| panic!("recovered operation must prepare fallback")),
        );
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn second_table_allocation_failure_reports_canonical_storage_cleanup_failure() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        fail_final_owner_allocation_after(1);
        test_panic_final_owner_drop_after(0);

        let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
            Ok(_) => panic!("combined evidence allocation and canonical drop failure must reject"),
            Err(error) => error,
        };
        assert!(matches!(
            error.cause(),
            RetainedWriterError::StorageDropUnverified
        ));
        assert!(matches!(
            error.try_into_operation(),
            Err(RetainedWriterError::StorageDropUnverified)
        ));
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn semantic_rejection_can_append_evidence_and_prepare_a_rich_failure() {
        let factory = account_factory(1);
        let operation =
            test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 2))
            .unwrap_or_else(|_| panic!("semantic rejection publication must open"));
        let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

        let mut failure = match assembly.validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                let _ = session.canonical(canonical);
                let _ = session.outcome_evidence(evidence);
                false
            },
        ) {
            RetainedSuccessValidation::Rejected(failure) => failure,
            _ => panic!("semantic rejection must preserve evidence-building ownership"),
        };
        let detail = failure
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(9, EvidenceDetailTag))
            })
            .unwrap_or_else(|_| panic!("bounded failure detail must retain"));
        failure
            .try_commit_outcome_evidence(TestDimension::Bytes, 3)
            .unwrap_or_else(|_| panic!("failure evidence commitment must fit"));
        let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
            session.outcome_evidence(root) == Some(&7)
                && session.outcome_evidence(detail) == Some(&9)
        }) {
            RetainedFailureValidation::Valid(validated) => validated,
            _ => panic!("augmented failure evidence must validate"),
        };
        let published = handoff_for_test(
            validated
                .try_prepare(PreparationFailureCode::domain(91), final_progress())
                .unwrap_or_else(|_| panic!("rich failure must prepare")),
        );
        match published {
            PublishedRetainedPublicationParts::RichFailure {
                outcome_evidence,
                ledger_evidence,
            } => {
                assert_eq!(outcome_evidence.cause(), PreparationFailureCode::domain(91));
                assert_eq!(outcome_evidence.outcome_evidence().root(), &7);
                assert_eq!(
                    outcome_evidence.outcome_evidence().aggregate().get(detail),
                    Some(&9)
                );
                assert_eq!(
                    ledger_evidence
                        .dimension(TestDimension::Bytes)
                        .expect("rich failure ledger evidence")
                        .committed_canonical_output(),
                    0
                );
            }
            _ => panic!("semantic rejection must publish rich failure only"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn validation_panic_transitions_to_fallback_only() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("validation-panic publication must open"));
        let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

        let fallback_only = match assembly.validate_success(
            canonical_root,
            evidence_root,
            |_session, _canonical, _evidence| panic!("validator panic"),
        ) {
            RetainedSuccessValidation::FallbackOnly(fallback) => fallback,
            _ => panic!("validator panic must expose only fallback"),
        };
        let published = handoff_for_test(
            fallback_only
                .prepare_minimal_fallback()
                .unwrap_or_else(|_| panic!("validation panic fallback must prepare")),
        );
        match &published {
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
                assert_eq!(
                    fallback.cause().class(),
                    PreparationFailureClass::ProductGraph
                );
                assert_eq!(fallback.cause().code(), 5);
            }
            _ => panic!("validation panic must publish minimal fallback"),
        }
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn repeated_semantic_rejection_preserves_the_product_graph_fallback_cause() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("repeated-rejection publication must open"));
        let (canonical, evidence) = retain_scalar_success(&mut assembly);

        let failure = match assembly.validate_success(
            canonical,
            evidence,
            |_session, _canonical, _evidence| false,
        ) {
            RetainedSuccessValidation::Rejected(failure) => failure,
            _ => panic!("first semantic rejection must expose failure building"),
        };
        let failure = match failure.validate_rich_failure(evidence, |_session, _evidence| false) {
            RetainedFailureValidation::Rejected(failure) => failure,
            _ => panic!("second semantic rejection must preserve failure building"),
        };
        let published = handoff_for_test(
            failure
                .prepare_minimal_fallback()
                .unwrap_or_else(|_| panic!("repeated rejection fallback must prepare")),
        );
        match &published {
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
                assert_eq!(
                    fallback.cause().class(),
                    PreparationFailureClass::ProductGraph
                );
                assert_eq!(fallback.cause().code(), 4);
            }
            _ => panic!("repeated semantic rejection must publish only fallback"),
        }
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn consuming_closure_validation_rejects_orphans_and_retagged_slots() {
        let factory = account_factory(1);
        let operation =
            test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
        let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
            .try_publication_assembly(publication_plan(2, 1))
            .unwrap_or_else(|_| panic!("publication assembly must open"));
        let orphan = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
            })
            .unwrap_or_else(|_| panic!("orphan candidate must retain"));
        let root = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestCanonicalBlock::Root { leaf: orphan }, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("root must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestEvidenceBlock::Detail(9), EvidenceDetailTag))
            })
            .unwrap_or_else(|_| panic!("evidence must retain"));

        let rejected = match assembly.validate_success(root, evidence, |session, root, evidence| {
            session.canonical(root).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Rejected(rejected) => rejected,
            _ => panic!("orphan graph must be rejected"),
        };
        let prepared = rejected
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("orphan rejection must prepare minimal fallback"));
        match handoff_for_test(prepared) {
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
                assert_eq!(
                    fallback.cause().class(),
                    PreparationFailureClass::ProductGraph
                );
                assert_eq!(fallback.cause().code(), 4);
            }
            _ => panic!("orphan rejection must publish minimal fallback"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);

        let operation =
            test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
        let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
            .try_publication_assembly(publication_plan(2, 1))
            .unwrap_or_else(|_| panic!("publication assembly must reopen"));
        let orphan = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
            })
            .unwrap_or_else(|_| panic!("orphan candidate must retain"));
        let root = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestCanonicalBlock::Root { leaf: orphan }, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("root must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestEvidenceBlock::Detail(9), EvidenceDetailTag))
            })
            .unwrap_or_else(|_| panic!("evidence must retain"));
        let retagged: CanonicalRetainedSlot<TestCanonicalBlock, CanonicalRootTag> =
            CanonicalRetainedSlot {
                identity: orphan.identity,
                marker: PhantomData,
            };
        let rejected = match assembly.validate_success(root, evidence, |session, root, evidence| {
            session.canonical(root).is_some()
                && session.canonical(retagged).is_some()
                && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Rejected(rejected) => rejected,
            _ => panic!("retagged graph must be rejected"),
        };
        let prepared = rejected
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("retag rejection must prepare minimal fallback"));
        match handoff_for_test(prepared) {
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
                assert_eq!(
                    fallback.cause().class(),
                    PreparationFailureClass::ProductGraph
                );
                assert_eq!(fallback.cause().code(), 4);
            }
            _ => panic!("retag rejection must publish minimal fallback"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn block_limits_reject_before_running_the_builder_and_minimal_fallback_stays_available() {
        static BUILDER_RAN: AtomicBool = AtomicBool::new(false);

        let factory = account_factory(1);
        let operation =
            test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
        let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("publication assembly must open"));
        let _ = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
            })
            .unwrap_or_else(|_| panic!("first block must fit"));
        BUILDER_RAN.store(false, Ordering::SeqCst);
        assert!(matches!(
            assembly.try_retain_canonical(empty_plan(), |emplacer| {
                BUILDER_RAN.store(true, Ordering::SeqCst);
                Ok(emplacer.place(TestCanonicalBlock::Leaf(2), CanonicalLeafTag))
            }),
            Err(RetainedPublicationError::CanonicalSlotLimit)
        ));
        assert!(!BUILDER_RAN.load(Ordering::SeqCst));
        let _ = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(TestEvidenceBlock::Detail(3), EvidenceDetailTag))
            })
            .unwrap_or_else(|_| panic!("evidence block must fit"));
        let prepared = assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("minimal fallback must remain available"));
        match handoff_for_test(prepared) {
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
                assert_eq!(*fallback.value(), 7);
            }
            _ => panic!("direct fallback must publish the minimal owner"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn partial_canonical_downgrades_to_rich_failure_after_storage_drops_first() {
        static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

        let factory = account_factory(1);
        let operation =
            test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
        let mut assembly: TestPublication<LiveDropObservation, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("publication assembly must open"));
        let ledger = assembly
            .operation
            .as_ref()
            .expect("assembly retains operation")
            .coordinator
            .retained_ledger();
        let observation = LiveDropObservation {
            ledger: ledger.test_probe(),
            account: assembly
                .account
                .as_ref()
                .expect("assembly retains account")
                .share_after_test_admission()
                .unwrap_or_else(|_| panic!("test account share must be admitted")),
            observed_active: &OBSERVED_ACTIVE,
            observed_operation_live: &OBSERVED_OPERATION_LIVE,
            observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
            observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
        };
        let _partial = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(observation, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("partial canonical block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(5, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence root must retain"));
        OBSERVED_ACTIVE.store(0, Ordering::SeqCst);
        OBSERVED_OPERATION_LIVE.store(0, Ordering::SeqCst);
        OBSERVED_PHYSICAL_BYTES.store(0, Ordering::SeqCst);
        OBSERVED_PHYSICAL_ALLOCATIONS.store(0, Ordering::SeqCst);
        let validated = match assembly.validate_rich_failure(evidence, |session, root| {
            session.outcome_evidence(root) == Some(&5)
        }) {
            RetainedFailureValidation::Valid(validated) => validated,
            _ => panic!("partial canonical failure graph must validate"),
        };
        let prepared = validated
            .try_prepare(PreparationFailureCode::domain(9), final_progress())
            .unwrap_or_else(|_| panic!("partial canonical must downgrade to rich failure"));

        assert!(OBSERVED_ACTIVE.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_OPERATION_LIVE.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_PHYSICAL_BYTES.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst) > 0);
        match handoff_for_test(prepared) {
            PublishedRetainedPublicationParts::RichFailure {
                outcome_evidence, ..
            } => {
                assert_eq!(*outcome_evidence.outcome_evidence().root(), 5);
                assert_eq!(outcome_evidence.cause(), PreparationFailureCode::domain(9));
            }
            _ => panic!("partial canonical must not publish canonical output"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn failure_only_plan_rejects_canonical_construction_and_publishes_evidence() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let plan = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
            writer_plan(2, 2),
            1,
            1,
        ));
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(plan)
            .unwrap_or_else(|_| panic!("failure-only assembly must open"));
        assert!(matches!(
            assembly.try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(11, CanonicalRootTag))
            }),
            Err(RetainedPublicationError::WrongOutcomeMode)
        ));
        assert!(matches!(
            assembly.try_commit_canonical(TestDimension::Bytes, 1),
            Err(RetainedPublicationError::WrongOutcomeMode)
        ));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("failure evidence must retain"));
        let validated = match assembly.validate_rich_failure(evidence, |session, root| {
            session.outcome_evidence(root) == Some(&7)
        }) {
            RetainedFailureValidation::Valid(validated) => validated,
            _ => panic!("failure-only graph must validate"),
        };
        match handoff_for_test(
            validated
                .try_prepare(PreparationFailureCode::domain(3), final_progress())
                .unwrap_or_else(|_| panic!("failure-only publication must prepare")),
        ) {
            PublishedRetainedPublicationParts::RichFailure {
                outcome_evidence, ..
            } => assert_eq!(*outcome_evidence.outcome_evidence().root(), 7),
            _ => panic!("failure-only plan cannot publish canonical output"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn success_control_rejection_atomically_downgrades_to_rich_failure() {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let control = OperationControl::default().with_progress_observer(
            |_event: crate::ProgressEvent<TestProgress, TestUnit>| crate::ProgressDecision::Cancel,
        );
        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            control,
        );
        drop(account);
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("publication assembly must open"));
        let (canonical, evidence) = retain_scalar_success(&mut assembly);

        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical) == Some(&11)
                    && session.outcome_evidence(evidence) == Some(&7)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("control rejection candidate must validate"),
            };
        match handoff_for_test(
            validated
                .try_prepare(final_progress())
                .unwrap_or_else(|_| panic!("control rejection must produce a rich failure")),
        ) {
            PublishedRetainedPublicationParts::RichFailure {
                outcome_evidence, ..
            } => {
                assert_eq!(
                    outcome_evidence.cause().class(),
                    PreparationFailureClass::Control
                );
                assert_eq!(*outcome_evidence.outcome_evidence().root(), 7);
            }
            _ => panic!("control rejection must not publish canonical output"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn evidence_primary_rejection_uses_minimal_fallback_without_retry() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 4);
        let plan = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
            writer_plan(2, 2),
            1,
            1,
        ));
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(plan)
            .unwrap_or_else(|_| panic!("failure-only assembly must open"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("failure evidence must retain"));
        assembly
            .try_commit_outcome_evidence(TestDimension::Bytes, 4)
            .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
        let validated = match assembly.validate_rich_failure(evidence, |session, root| {
            session.outcome_evidence(root) == Some(&7)
        }) {
            RetainedFailureValidation::Valid(validated) => validated,
            _ => panic!("evidence rejection candidate must validate"),
        };
        match handoff_for_test(
            validated
                .try_prepare(PreparationFailureCode::domain(5), final_progress())
                .unwrap_or_else(|_| panic!("ledger rejection must retain minimal fallback")),
        ) {
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
                assert_eq!(*fallback.value(), 7);
                assert_eq!(fallback.cause().class(), PreparationFailureClass::Ledger);
            }
            _ => panic!("primary ledger rejection must not retry rich failure"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn success_primary_rejection_drops_both_tables_before_minimal_fallback() {
        static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
        static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

        CANONICAL_DROPS.store(0, Ordering::SeqCst);
        EVIDENCE_DROPS.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let operation = test_operation(&factory, 4);
        let mut assembly: TestPublication<DropCounter, DropCounter> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("success rejection assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
        assembly
            .try_commit_outcome_evidence(TestDimension::Bytes, 4)
            .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));

        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence).is_some()
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("success rejection candidate must validate"),
            };
        let prepared = validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("success rejection must prepare minimal fallback"));
        assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
        match handoff_for_test(prepared) {
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
                assert_eq!(fallback.cause().class(), PreparationFailureClass::Ledger);
            }
            _ => panic!("success ledger rejection must publish minimal fallback"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn primary_ordering_panic_drops_storage_while_attribution_remains_live() {
        static BLOCK_DROPS: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_LIVE: AtomicU64 = AtomicU64::new(0);

        let _ord_lock = PRIMARY_DIMENSION_ORD_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _reset = PrimaryDimensionOrdReset;
        PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);
        BLOCK_DROPS.store(0, Ordering::SeqCst);
        OBSERVED_ACTIVE.store(0, Ordering::SeqCst);
        OBSERVED_LIVE.store(0, Ordering::SeqCst);

        type PanicOperation = RetainedOperation<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            PrimaryPanicDimension,
            u32,
        >;
        type PanicPublication = RetainedPublicationAssembly<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            PrimaryPanicDimension,
            u32,
            PrimaryBindDropObservation,
            u64,
        >;

        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let dimension = PrimaryPanicDimension(7);
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
        let limits = LedgerLimits::new(u64::MAX, [(dimension, policy)])
            .unwrap_or_else(|_| panic!("panic-order policy must be valid"));
        let writer = RetainedWriterPlan::new(
            TestPhase::Build,
            dimension,
            dimension,
            RetainedBatchLimits::new(2, 2),
        );
        let start = RetainedOperationStartPlan::new(
            writer,
            RetainedHandleSharePlan::new(dimension, dimension),
            empty_plan(),
            OutputCommitment::new(dimension, 1),
            OutputCommitment::new(dimension, 1),
        );
        let operation =
            PanicOperation::try_start(OperationControl::default(), limits, &account, start, || {
                Ok(7_u32)
            })
            .unwrap_or_else(|_| panic!("panic-order operation must start"));
        let ledger = operation.coordinator.retained_ledger().test_probe();
        let deferred_ledger = StdArc::new(StdMutex::new(Some(ledger)));
        let table_writer = RetainedWriterPlan::new(
            TestPhase::Build,
            dimension,
            dimension,
            RetainedBatchLimits::new(2, 2),
        );
        let publication_plan = RetainedPublicationPlan::success(
            RetainedBlockTablePlan::new(table_writer, 1, 1),
            RetainedBlockTablePlan::new(table_writer, 1, 1),
        );
        let mut assembly: PanicPublication = operation
            .try_publication_assembly(publication_plan)
            .unwrap_or_else(|_| panic!("panic-order publication must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(
                    PrimaryBindDropObservation {
                        ledger: StdArc::clone(&deferred_ledger),
                        drop_count: &BLOCK_DROPS,
                        observed_active: &OBSERVED_ACTIVE,
                        observed_live: &OBSERVED_LIVE,
                    },
                    CanonicalRootTag,
                ))
            })
            .unwrap_or_else(|_| panic!("panic-order canonical block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(9_u64, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("panic-order evidence block must retain"));
        let (expected_active, expected_live) = {
            let ledger = lock_recovering_poison(&deferred_ledger);
            let ledger = ledger.as_ref().expect("panic-order ledger probe");
            (ledger.active_reservations(), ledger.current_live_bytes())
        };

        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence) == Some(&9)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("panic-order candidate must validate"),
            };
        PANIC_ON_PRIMARY_DIMENSION_ORD.store(true, Ordering::SeqCst);
        let result = validated.try_prepare(final_progress());
        PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::PrimaryPublicationBindingPanicked
        ));
        assert_eq!(BLOCK_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(OBSERVED_ACTIVE.load(Ordering::SeqCst), expected_active);
        assert_eq!(OBSERVED_LIVE.load(Ordering::SeqCst), expected_live);
        {
            let ledger = lock_recovering_poison(&deferred_ledger);
            let ledger = ledger.as_ref().expect("retained panic-order ledger probe");
            assert_eq!(ledger.active_reservations(), expected_active);
            assert_eq!(ledger.current_live_bytes(), expected_live);
        }
        assert!(account.snapshot().live_retained_bytes() > 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn cleanup_lattice_is_order_independent_and_preserves_error_precedence() {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 16);
        let storage =
            RetainedStorageDropOutcome::StorageReleaseUnverified(record_account_poison_event(
                &account,
                AccountPoisonReason::RetainedStorageReleaseUnverified,
            ));
        let ownership = RetainedStorageDropOutcome::ForeignOwnershipFault(
            record_account_poison_event(&account, AccountPoisonReason::OwnershipGraphUnverified),
        );
        let charge = RetainedStorageDropOutcome::ChargeReleaseUnverified(
            record_account_poison_event(&account, AccountPoisonReason::CounterInvariantViolated),
        );

        for (left, right, expected) in [
            (storage, ownership, RetainedStorageDropSeverity::Storage),
            (storage, charge, RetainedStorageDropSeverity::Storage),
            (ownership, charge, RetainedStorageDropSeverity::Ownership),
            (
                charge,
                RetainedStorageDropOutcome::Clean,
                RetainedStorageDropSeverity::Charge,
            ),
        ] {
            assert_eq!(left.join(right).severity(), expected);
            assert_eq!(right.join(left).severity(), expected);
        }
        assert_eq!(
            RetainedStorageDropOutcome::MissingFaultAnchor
                .join(ownership)
                .witness()
                .map(|witness| witness.reason),
            Some(AccountPoisonReason::OwnershipGraphUnverified)
        );
        assert_eq!(
            ownership
                .join(RetainedStorageDropOutcome::MissingFaultAnchor)
                .witness()
                .map(|witness| witness.reason),
            Some(AccountPoisonReason::OwnershipGraphUnverified)
        );

        let cause: RetainedWriterError<TestDimension> = join_pre_attribution_cleanup_cause(
            RetainedStorageDropOutcome::Clean,
            RetainedWriterError::InvalidPublicationPlan,
        );
        assert!(matches!(cause, RetainedWriterError::InvalidPublicationPlan));
        let cause = join_pre_attribution_cleanup_cause(
            charge,
            RetainedWriterError::<TestDimension>::OwnershipUnverified,
        );
        assert!(matches!(cause, RetainedWriterError::OwnershipUnverified));
        let cause = join_pre_attribution_cleanup_cause(
            ownership,
            RetainedWriterError::<TestDimension>::StorageDropUnverified,
        );
        assert!(matches!(cause, RetainedWriterError::StorageDropUnverified));
        let cause = join_pre_attribution_cleanup_cause(
            storage,
            RetainedWriterError::<TestDimension>::OwnershipUnverified,
        );
        assert!(matches!(cause, RetainedWriterError::StorageDropUnverified));

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn primary_binding_panic_reports_later_storage_release_failure() {
        let _ord_lock = PRIMARY_DIMENSION_ORD_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _reset = PrimaryDimensionOrdReset;
        PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

        type PanicOperation = RetainedOperation<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            PrimaryPanicDimension,
            u32,
        >;
        type PanicPublication = RetainedPublicationAssembly<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            PrimaryPanicDimension,
            u32,
            u64,
            u64,
        >;

        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let dimension = PrimaryPanicDimension(7);
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
        let limits = LedgerLimits::new(u64::MAX, [(dimension, policy)])
            .unwrap_or_else(|_| panic!("combined-fault policy must be valid"));
        let writer = RetainedWriterPlan::new(
            TestPhase::Build,
            dimension,
            dimension,
            RetainedBatchLimits::new(2, 2),
        );
        let start = RetainedOperationStartPlan::new(
            writer,
            RetainedHandleSharePlan::new(dimension, dimension),
            empty_plan(),
            OutputCommitment::new(dimension, 1),
            OutputCommitment::new(dimension, 1),
        );
        let operation =
            PanicOperation::try_start(OperationControl::default(), limits, &account, start, || {
                Ok(7_u32)
            })
            .unwrap_or_else(|_| panic!("combined-fault operation must start"));
        let publication_plan = RetainedPublicationPlan::success(
            RetainedBlockTablePlan::new(writer, 1, 1),
            RetainedBlockTablePlan::new(writer, 1, 1),
        );
        let mut assembly: PanicPublication = operation
            .try_publication_assembly(publication_plan)
            .unwrap_or_else(|_| panic!("combined-fault publication must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(11_u64, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("combined-fault canonical block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(9_u64, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("combined-fault evidence block must retain"));
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical) == Some(&11)
                    && session.outcome_evidence(evidence) == Some(&9)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("combined-fault candidate must validate"),
            };

        test_panic_final_owner_drop_after(0);
        PANIC_ON_PRIMARY_DIMENSION_ORD.store(true, Ordering::SeqCst);
        let result = validated.try_prepare(final_progress());
        PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
        ));
        let snapshot = account.snapshot();
        assert!(snapshot.poison_event_count() >= 2);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::PrimaryPublicationBindingPanicked)
        );
        assert!(snapshot.live_retained_bytes() > 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn second_table_drop_panic_keeps_success_rejection_accounting_fail_closed() {
        static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
        static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

        CANONICAL_DROPS.store(0, Ordering::SeqCst);
        EVIDENCE_DROPS.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let operation = test_operation(&factory, 4);
        let mut assembly: TestPublication<DropCounter, PanickingDropProbe> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("success rejection assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(PanickingDropProbe(&EVIDENCE_DROPS), EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("panicking evidence must retain"));
        assembly
            .try_commit_outcome_evidence(TestDimension::Bytes, 4)
            .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
        let probe = assembly
            .operation
            .as_ref()
            .expect("assembly retains operation")
            .coordinator
            .retained_ledger()
            .test_probe();
        let before_active = probe.active_reservations();
        let before_live = probe.current_live_bytes();

        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence).is_some()
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("panicking-drop candidate must validate"),
            };
        let result = validated.try_prepare(final_progress());
        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
        ));
        assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(before_active - probe.active_reservations(), 3);
        assert!(probe.active_reservations() > 0);
        assert!(probe.current_live_bytes() > 0);
        assert!(probe.current_live_bytes() < before_live);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn first_table_drop_panic_still_releases_the_clean_evidence_table() {
        static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
        static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

        CANONICAL_DROPS.store(0, Ordering::SeqCst);
        EVIDENCE_DROPS.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let operation = test_operation(&factory, 4);
        let mut assembly: TestPublication<PanickingDropProbe, DropCounter> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("success rejection assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(PanickingDropProbe(&CANONICAL_DROPS), CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("panicking canonical must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
        assembly
            .try_commit_outcome_evidence(TestDimension::Bytes, 4)
            .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
        let probe = assembly
            .operation
            .as_ref()
            .expect("assembly retains operation")
            .coordinator
            .retained_ledger()
            .test_probe();
        let before_active = probe.active_reservations();
        let before_live = probe.current_live_bytes();

        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence).is_some()
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("panicking-drop candidate must validate"),
            };
        let result = validated.try_prepare(final_progress());
        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
        ));
        assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(before_active - probe.active_reservations(), 3);
        assert!(probe.active_reservations() > 0);
        assert!(probe.current_live_bytes() > 0);
        assert!(probe.current_live_bytes() < before_live);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn table_payload_drop_panic_fails_closed_before_attribution_release() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("publication assembly must open"));
        DROP_COUNT.store(0, Ordering::SeqCst);
        let _canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(PanickingDropProbe(&DROP_COUNT), CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("panicking block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence must retain"));
        let validated = match assembly.validate_rich_failure(evidence, |session, root| {
            session.outcome_evidence(root) == Some(&7)
        }) {
            RetainedFailureValidation::Valid(validated) => validated,
            _ => panic!("table-drop failure candidate must validate"),
        };
        assert!(matches!(
            validated.try_prepare(PreparationFailureCode::domain(2), final_progress()),
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn baseline_drop_payload_before_releasing_live_ownership() {
        static EXPECTED_ACTIVE: AtomicU64 = AtomicU64::new(0);
        static EXPECTED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
        static EXPECTED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
        static EXPECTED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let deferred_ledger = StdArc::new(StdMutex::new(None));
        let fallback = DeferredLiveDropObservation {
            ledger: StdArc::clone(&deferred_ledger),
            account: account
                .share_after_test_admission()
                .unwrap_or_else(|_| panic!("test account share must be admitted")),
            observed_active: &OBSERVED_ACTIVE,
            observed_operation_live: &OBSERVED_OPERATION_LIVE,
            observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
            observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
        };
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let plan = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        let operation: RetainedOperation<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            TestDimension,
            DeferredLiveDropObservation,
        > = RetainedOperation::try_start(
            OperationControl::default(),
            limits,
            &account,
            plan,
            || Ok(fallback),
        )
        .unwrap_or_else(|_| panic!("baseline observation operation must start"));
        let ledger = operation.coordinator.retained_ledger().test_probe();
        EXPECTED_ACTIVE.store(ledger.active_reservations(), Ordering::SeqCst);
        EXPECTED_OPERATION_LIVE.store(ledger.current_live_bytes(), Ordering::SeqCst);
        let physical = operation.account().snapshot();
        EXPECTED_PHYSICAL_BYTES.store(physical.live_retained_bytes(), Ordering::SeqCst);
        EXPECTED_PHYSICAL_ALLOCATIONS.store(physical.live_retained_allocations(), Ordering::SeqCst);
        *lock_recovering_poison(&deferred_ledger) = Some(ledger);

        drop(operation);
        assert_eq!(
            OBSERVED_ACTIVE.load(Ordering::SeqCst),
            EXPECTED_ACTIVE.load(Ordering::SeqCst)
        );
        assert_eq!(
            OBSERVED_OPERATION_LIVE.load(Ordering::SeqCst),
            EXPECTED_OPERATION_LIVE.load(Ordering::SeqCst)
        );
        assert_eq!(
            OBSERVED_PHYSICAL_BYTES.load(Ordering::SeqCst),
            EXPECTED_PHYSICAL_BYTES.load(Ordering::SeqCst)
        );
        assert_eq!(
            OBSERVED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst),
            EXPECTED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst)
        );
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn prepared_published_and_baseline_owners_contain_payload_drop_panics() {
        static PREPARED_DROPS: AtomicU64 = AtomicU64::new(0);
        static PUBLISHED_DROPS: AtomicU64 = AtomicU64::new(0);
        static BASELINE_DROPS: AtomicU64 = AtomicU64::new(0);

        let factory = account_factory(1);
        PREPARED_DROPS.store(0, Ordering::SeqCst);
        let operation = test_operation(&factory, 64);
        let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("prepared drop assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(PanickingDropProbe(&PREPARED_DROPS), CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("prepared drop block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("prepared drop evidence must retain"));
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence) == Some(&7)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("prepared-drop graph must validate"),
            };
        let prepared = validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("prepared drop publication must prepare"));
        assert!(catch_unwind(AssertUnwindSafe(|| drop(prepared))).is_ok());
        assert_eq!(PREPARED_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().live_accounts(), 1);

        let factory = account_factory(1);
        PUBLISHED_DROPS.store(0, Ordering::SeqCst);
        let operation = test_operation(&factory, 64);
        let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("published drop assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(PanickingDropProbe(&PUBLISHED_DROPS), CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("published drop block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("published drop evidence must retain"));
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence) == Some(&7)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("published-drop graph must validate"),
            };
        let published = handoff_for_test(
            validated
                .try_prepare(final_progress())
                .unwrap_or_else(|_| panic!("published drop publication must prepare")),
        );
        assert!(catch_unwind(AssertUnwindSafe(|| drop(published))).is_ok());
        assert_eq!(PUBLISHED_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().live_accounts(), 1);

        let factory = account_factory(1);
        BASELINE_DROPS.store(0, Ordering::SeqCst);
        let operation = test_operation_with_fallback(&factory, PanickingDropProbe(&BASELINE_DROPS));
        assert!(catch_unwind(AssertUnwindSafe(|| drop(operation))).is_ok());
        assert_eq!(BASELINE_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn prepared_and_published_drop_keep_physical_charge_and_handoff_is_a_pure_move() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_BYTES: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        OBSERVED_BYTES.store(0, Ordering::SeqCst);
        OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let observed_account = operation
            .account()
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted"));
        let mut assembly: TestPublication<PhysicalDropObservation, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("prepared observation assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(
                    PhysicalDropObservation {
                        account: observed_account,
                        drop_count: &DROP_COUNT,
                        observed_bytes: &OBSERVED_BYTES,
                        observed_allocations: &OBSERVED_ALLOCATIONS,
                    },
                    CanonicalRootTag,
                ))
            })
            .unwrap_or_else(|_| panic!("prepared observation block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("prepared observation evidence must retain"));
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence) == Some(&7)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("prepared observation graph must validate"),
            };
        let prepared = validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("prepared observation publication must prepare"));
        drop(prepared);
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
        assert_eq!(factory.snapshot().live_accounts(), 0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        OBSERVED_BYTES.store(0, Ordering::SeqCst);
        OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let observed_account = operation
            .account()
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted"));
        let mut assembly: TestPublication<PhysicalDropObservation, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("handoff observation assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(
                    PhysicalDropObservation {
                        account: observed_account,
                        drop_count: &DROP_COUNT,
                        observed_bytes: &OBSERVED_BYTES,
                        observed_allocations: &OBSERVED_ALLOCATIONS,
                    },
                    CanonicalRootTag,
                ))
            })
            .unwrap_or_else(|_| panic!("handoff observation block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("handoff observation evidence must retain"));
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence) == Some(&7)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("handoff observation graph must validate"),
            };
        let prepared = validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("handoff observation publication must prepare"));
        let prepared = match prepared.into_outcome() {
            PreparedRetainedPublicationOutcome::Success(prepared) => prepared,
            _ => panic!("handoff observation must prepare success"),
        };
        let (canonical_identity, canonical_shares, evidence_identity, evidence_shares) = (
            prepared.canonical.inner.table.allocation_identity(),
            prepared.canonical.inner.table.share_call_count(),
            prepared.outcome_evidence.inner.table.allocation_identity(),
            prepared.outcome_evidence.inner.table.share_call_count(),
        );
        let ledger_allocation = prepared.evidence.allocation();
        let ledger_identity = ledger_allocation.allocation_identity();
        let ledger_shares = ledger_allocation.share_call_count();

        let published = prepared.handoff();
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
        assert_eq!(OBSERVED_BYTES.load(Ordering::SeqCst), 0);
        assert_eq!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst), 0);
        assert_eq!(
            published
                .canonical
                .aggregate
                .inner
                .table
                .allocation_identity(),
            canonical_identity
        );
        assert_eq!(
            published.canonical.aggregate.inner.table.share_call_count(),
            canonical_shares
        );
        assert_eq!(
            published
                .outcome_evidence
                .aggregate
                .inner
                .table
                .allocation_identity(),
            evidence_identity
        );
        assert_eq!(
            published
                .outcome_evidence
                .aggregate
                .inner
                .table
                .share_call_count(),
            evidence_shares
        );
        let ledger_allocation = published.ledger_evidence.evidence.allocation();
        assert_eq!(ledger_allocation.allocation_identity(), ledger_identity);
        assert_eq!(ledger_allocation.share_call_count(), ledger_shares);

        drop(published);
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn success_preparation_contains_fallback_drop_panic() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        struct PanickingFallback;

        impl Drop for PanickingFallback {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
                std::panic::panic_any(vec![0_u8; 8]);
            }
        }

        type PanicFallbackOperation = RetainedOperation<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            TestDimension,
            PanickingFallback,
        >;
        type PanicFallbackPublication = RetainedPublicationAssembly<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            TestDimension,
            PanickingFallback,
            u64,
            u64,
        >;

        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let plan = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        DROP_COUNT.store(0, Ordering::SeqCst);
        let operation = PanicFallbackOperation::try_start(
            OperationControl::default(),
            limits,
            &account,
            plan,
            || Ok(PanickingFallback),
        )
        .unwrap_or_else(|_| panic!("panic fallback operation must start"));
        let mut assembly: PanicFallbackPublication = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("panic fallback assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(41, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("canonical block must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence block must retain"));
        let probe = assembly
            .operation
            .as_ref()
            .expect("assembly retains operation")
            .coordinator
            .retained_ledger()
            .test_probe();
        let before_active = probe.active_reservations();
        let before_live = probe.current_live_bytes();
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical) == Some(&41)
                    && session.outcome_evidence(evidence) == Some(&7)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("fallback-panic candidate must validate"),
            };
        let result = catch_unwind(AssertUnwindSafe(|| validated.try_prepare(final_progress())))
            .expect("fallback destructor panic must be contained");
        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert!(before_active > 0);
        assert!(before_live > 0);
        assert!(probe.active_reservations() > 0);
        assert!(probe.active_reservations() <= before_active);
        assert!(probe.current_live_bytes() > 0);
        assert!(probe.current_live_bytes() <= before_live);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn rejected_primary_payload_drop_panic_fails_closed_before_token_release() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        struct PanickingEvidence;

        impl Drop for PanickingEvidence {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
                std::panic::panic_any(vec![0_u8; 8]);
            }
        }

        let factory = account_factory(1);
        let operation = test_operation(&factory, 4);
        let plan = RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
            writer_plan(2, 2),
            1,
            1,
        ));
        let mut assembly: TestPublication<u64, PanickingEvidence> = operation
            .try_publication_assembly(plan)
            .unwrap_or_else(|_| panic!("rejected primary assembly must open"));
        DROP_COUNT.store(0, Ordering::SeqCst);
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(PanickingEvidence, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("panicking evidence must retain"));
        assembly
            .try_commit_outcome_evidence(TestDimension::Bytes, 4)
            .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
        let validated = match assembly.validate_rich_failure(evidence, |session, root| {
            session.outcome_evidence(root).is_some()
        }) {
            RetainedFailureValidation::Valid(validated) => validated,
            _ => panic!("rejected-primary candidate must validate"),
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            validated.try_prepare(PreparationFailureCode::domain(5), final_progress())
        }))
        .expect("primary rejection payload panic must be contained");
        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn invalid_and_allocation_failed_open_preserve_operation_for_retry() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let invalid_plan = publication_plan(0, 1);
        let invalid = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
            Ok(_) => panic!("zero-slot table must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            invalid.cause(),
            RetainedWriterError::InvalidPublicationPlan
        ));
        let operation = match invalid.try_into_operation() {
            Ok(operation) => operation,
            Err(_) => panic!("invalid plan must preserve operation ownership"),
        };

        fail_next_final_owner_allocation();
        let allocation =
            match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
                Ok(_) => panic!("injected table allocation must fail"),
                Err(error) => error,
            };
        assert!(matches!(
            allocation.cause(),
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
        ));
        let operation = match allocation.try_into_operation() {
            Ok(operation) => operation,
            Err(_) => panic!("verified allocation failure must preserve operation ownership"),
        };
        let assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("retry must open a valid assembly"));
        drop(handoff_for_test(
            assembly
                .prepare_minimal_fallback()
                .unwrap_or_else(|_| panic!("retry fallback must prepare")),
        ));
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn recoverable_block_allocation_failure_preserves_assembly_for_retry() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("retry assembly must open"));
        let probe = assembly
            .operation
            .as_ref()
            .expect("assembly retains operation")
            .coordinator
            .retained_ledger()
            .test_probe();
        let before_active = probe.active_reservations();
        let before_live = probe.current_live_bytes();
        let before_physical = assembly
            .account
            .as_ref()
            .expect("assembly retains account")
            .snapshot();

        fail_next_final_owner_allocation();
        assert!(matches!(
            assembly.try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(11, CanonicalRootTag))
            }),
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
            ))
        ));
        let after_physical = assembly
            .account
            .as_ref()
            .expect("assembly retains account after recoverable failure")
            .snapshot();
        assert_eq!(probe.active_reservations(), before_active);
        assert_eq!(probe.current_live_bytes(), before_live);
        assert_eq!(
            after_physical.live_retained_bytes(),
            before_physical.live_retained_bytes()
        );
        assert_eq!(
            after_physical.live_retained_allocations(),
            before_physical.live_retained_allocations()
        );

        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(11, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("canonical retry must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence root must retain"));
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical) == Some(&11)
                    && session.outcome_evidence(evidence) == Some(&7)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("retried assembly graph must validate"),
            };
        let published = handoff_for_test(
            validated
                .try_prepare(final_progress())
                .unwrap_or_else(|_| panic!("retried assembly must prepare")),
        );
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn closure_validation_panic_publishes_minimal_fallback() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("publication assembly must open"));
        let canonical = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(11, CanonicalRootTag))
            })
            .unwrap_or_else(|_| panic!("canonical root must retain"));
        let evidence = assembly
            .try_retain_outcome_evidence(empty_plan(), |emplacer| {
                Ok(emplacer.place(7, EvidenceRootTag))
            })
            .unwrap_or_else(|_| panic!("evidence root must retain"));

        let fallback_only = match assembly.validate_success(
            canonical,
            evidence,
            |_session, _canonical, _evidence| std::panic::panic_any(vec![0_u8; 8]),
        ) {
            RetainedSuccessValidation::FallbackOnly(fallback) => fallback,
            _ => panic!("validation panic must expose fallback-only state"),
        };
        let prepared = fallback_only
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("validation panic must prepare minimal fallback"));
        match handoff_for_test(prepared) {
            PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
                assert_eq!(
                    fallback.cause().class(),
                    PreparationFailureClass::ProductGraph
                );
                assert_eq!(fallback.cause().code(), 5);
            }
            _ => panic!("validation panic must publish minimal fallback"),
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn explicit_limits_reject_zero_and_plans_reject_mismatched_nested_evidence() {
        assert_eq!(
            PhysicalStorageAccountFactoryLimits::new(0, 1),
            Err(PhysicalStorageLimitError::ZeroByteLimit)
        );
        assert_eq!(
            PhysicalStorageAccountLimits::new(1, 0),
            Err(PhysicalStorageLimitError::ZeroAllocationLimit)
        );
        assert_eq!(
            RetainedAllocationPlan::new(1, 0),
            Err(RetainedAllocationPlanError::MismatchedNestedFootprint {
                bytes: 1,
                allocations: 0,
            })
        );
    }

    #[test]
    fn inline_ceiling_covers_the_complete_extracted_ownership_header() {
        let factory = account_factory(1);
        let account = account(&factory, 16_384, 8);
        let result = account.try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>([0_u8; MAX_RETAINED_INLINE_BYTES])
        });

        assert!(matches!(
            result,
            Err(RetainedAllocationError::InlinePayloadTooLarge {
                bytes,
                limit: MAX_RETAINED_INLINE_BYTES,
            }) if bytes > MAX_RETAINED_INLINE_BYTES
        ));
    }

    #[test]
    fn factory_enforces_exact_live_account_and_control_byte_limits() {
        let control_bytes =
            FinalOwnerAllocation::<AccountControlInner>::checked_layout_bytes().unwrap();
        let limits = PhysicalStorageAccountFactoryLimits::new(control_bytes, 1).unwrap();
        let factory = PhysicalStorageAccountFactory::try_new(limits).unwrap();
        let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

        let first = factory.try_create_account(account_limits).unwrap();
        assert_eq!(factory.snapshot().live_accounts(), 1);
        assert_eq!(
            factory.snapshot().live_account_control_bytes(),
            control_bytes
        );
        assert!(matches!(
            factory.try_create_account(account_limits),
            Err(PhysicalStorageAccountCreateError::ControlByteLimitExceeded { .. })
                | Err(PhysicalStorageAccountCreateError::LiveAccountLimitExceeded { .. })
        ));

        drop(first);
        assert_eq!(factory.snapshot().live_accounts(), 0);
        assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
    }

    #[test]
    fn account_control_allocation_failure_rolls_back_factory_admission() {
        let factory = account_factory(1);
        let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

        fail_next_final_owner_allocation();
        assert_eq!(
            factory.try_create_account(account_limits).err(),
            Some(PhysicalStorageAccountCreateError::AllocationFailed)
        );
        assert_eq!(factory.snapshot().live_accounts(), 0);
        assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
    }

    #[test]
    fn last_account_control_owner_releases_factory_admission_during_unwind() {
        let factory = account_factory(1);
        let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _account = factory.try_create_account(account_limits).unwrap();
            panic!("drop the final account control owner during unwind");
        }));

        assert!(result.is_err());
        assert_eq!(factory.snapshot().live_accounts(), 0);
        assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
    }

    #[test]
    fn concurrent_factory_admission_cannot_oversubscribe_one_account_slot() {
        let control_bytes =
            FinalOwnerAllocation::<AccountControlInner>::checked_layout_bytes().unwrap();
        let limits = PhysicalStorageAccountFactoryLimits::new(control_bytes, 1).unwrap();
        let factory = PhysicalStorageAccountFactory::try_new(limits).unwrap();
        let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();
        let start = StdArc::new(Barrier::new(3));
        let attempts_complete = StdArc::new(Barrier::new(3));
        let release_owner = StdArc::new(Barrier::new(2));
        let successes = StdArc::new(AtomicU64::new(0));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let factory = factory.clone();
            let start = StdArc::clone(&start);
            let attempts_complete = StdArc::clone(&attempts_complete);
            let release_owner = StdArc::clone(&release_owner);
            let successes = StdArc::clone(&successes);
            threads.push(thread::spawn(move || {
                start.wait();
                let account = factory.try_create_account(account_limits).ok();
                if account.is_some() {
                    successes.fetch_add(1, Ordering::SeqCst);
                }
                attempts_complete.wait();
                if let Some(account) = account {
                    release_owner.wait();
                    drop(account);
                }
            }));
        }

        start.wait();
        attempts_complete.wait();
        assert_eq!(successes.load(Ordering::SeqCst), 1);
        release_owner.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn retained_admission_accepts_exact_limits_and_rejects_excess() {
        let outer = FinalOwnerAllocation::<
            RetainedFinalOwnerInner<RetainedPayload<u64>, LeafRetainedPolicy>,
        >::checked_layout_bytes()
        .unwrap();
        let factory = account_factory(1);
        let account = account(&factory, outer + 11, 2);
        let plan = RetainedAllocationPlan::new(11, 1).unwrap();

        let retained = account
            .try_retain_with(plan, || Ok::<_, RetainedBuildFailureCode>(7_u64))
            .unwrap();
        assert_eq!(retained.charged_bytes(), outer + 11);
        assert_eq!(retained.charged_allocations(), 2);
        assert_eq!(account.snapshot().live_retained_bytes(), outer + 11);
        assert!(matches!(
            account.try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(9_u64)
            }),
            Err(RetainedAllocationError::ByteLimitExceeded { .. })
                | Err(RetainedAllocationError::AllocationLimitExceeded { .. })
        ));

        drop(retained);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn builder_error_code_is_returned_after_charge_release() {
        let factory = account_factory(1);
        let account = account(&factory, 1_024, 8);

        let error = account
            .try_retain_with::<u64>(empty_plan(), || Err(BUILD_FAILED))
            .unwrap_err();
        assert_eq!(error, RetainedAllocationError::Build(BUILD_FAILED));
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn builder_local_storage_drops_before_its_physical_lease() {
        static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

        struct BuilderLocalStorage {
            account: PhysicalStorageAccount,
            _bytes: Vec<u8>,
        }

        impl Drop for BuilderLocalStorage {
            fn drop(&mut self) {
                OBSERVED_LIVE_BYTES.store(
                    self.account.snapshot().live_retained_bytes(),
                    Ordering::SeqCst,
                );
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
        let plan = RetainedAllocationPlan::new(64, 1).unwrap();
        let error = account
            .try_retain_with::<u64>(plan, || {
                let _storage = BuilderLocalStorage {
                    account: account
                        .share_after_test_admission()
                        .unwrap_or_else(|_| panic!("test account share must be admitted")),
                    _bytes: vec![0; 64],
                };
                Err(BUILD_FAILED)
            })
            .unwrap_err();

        assert_eq!(error, RetainedAllocationError::Build(BUILD_FAILED));
        assert!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst) > 64);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn every_builder_panic_payload_keeps_its_physical_charge_fail_closed() {
        let factory = account_factory(3);
        let borrowed_account = account(&factory, 4_096, 16);
        let before = borrowed_account.snapshot();

        let borrowed = borrowed_account
            .try_retain_with::<u64>(empty_plan(), || panic!("test panic"))
            .unwrap_err();
        assert_eq!(borrowed, RetainedAllocationError::BuilderPanicked);
        let after_borrowed = borrowed_account.snapshot();
        assert!(after_borrowed.live_retained_bytes() > before.live_retained_bytes());
        assert!(after_borrowed.live_retained_allocations() > before.live_retained_allocations());
        assert!(after_borrowed.poisoned());
        assert_eq!(after_borrowed.poison_event_count(), 1);
        assert!(after_borrowed.poison_event_count_exact());
        assert!(after_borrowed.poison_event_domain_valid());
        assert_eq!(
            after_borrowed
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::RetainedBuilderPanicked)
        );

        let later_builder_ran = AtomicBool::new(false);
        let later = borrowed_account
            .try_retain_with::<u64>(empty_plan(), || {
                later_builder_ran.store(true, Ordering::SeqCst);
                Ok(7)
            })
            .unwrap_err();
        assert!(matches!(later, RetainedAllocationError::AccountPoisoned(_)));
        assert!(!later_builder_ran.load(Ordering::SeqCst));
        assert_eq!(borrowed_account.snapshot(), after_borrowed);

        let owned_account = account(&factory, 4_096, 16);
        let owned_before = owned_account.snapshot();
        let owned = owned_account
            .try_retain_with::<u64>(empty_plan(), || {
                std::panic::panic_any(String::from("owned test panic"))
            })
            .unwrap_err();
        assert_eq!(owned, RetainedAllocationError::BuilderPanicked);
        let after_owned = owned_account.snapshot();
        assert!(after_owned.live_retained_bytes() > owned_before.live_retained_bytes());
        assert!(after_owned.live_retained_allocations() > owned_before.live_retained_allocations());

        let opaque_account = account(&factory, 4_096, 16);
        let opaque_before = opaque_account.snapshot();
        let plan = RetainedAllocationPlan::new(64, 1).unwrap();
        let opaque = opaque_account
            .try_retain_with::<u64>(plan, || std::panic::panic_any(vec![0_u8; 64]))
            .unwrap_err();
        assert_eq!(opaque, RetainedAllocationError::BuilderPanicked);
        let after_opaque = opaque_account.snapshot();
        assert!(after_opaque.live_retained_bytes() > opaque_before.live_retained_bytes() + 64);
        assert!(
            after_opaque.live_retained_allocations() > opaque_before.live_retained_allocations()
        );
        drop(borrowed_account);
        drop(owned_account);
        drop(opaque_account);
        assert_eq!(factory.snapshot().live_accounts(), 3);
    }

    #[test]
    fn shared_handles_observe_one_latched_first_event_and_exact_event_count() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let observer = account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("observer share must be admitted before poison"));
        let first = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("first poison probe must retain"));
        let second = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("second poison probe must retain"));

        drop(first);
        let after_first = account.snapshot();
        assert_eq!(after_first, observer.snapshot());
        assert_eq!(after_first.poison_event_count(), 1);
        assert_eq!(
            after_first
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
        );

        drop(second);
        let after_second = account.snapshot();
        assert_eq!(after_second, observer.snapshot());
        assert_eq!(after_second.poison_event_count(), 2);
        assert!(after_second.poison_event_count_exact());
        assert!(after_second.poison_event_domain_valid());
        assert_eq!(
            after_second.first_poison_evidence(),
            after_first.first_poison_evidence()
        );
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 2);

        let before_rejected_share = account.snapshot();
        let rejected_share = account.share_after_test_admission().unwrap_err();
        assert!(matches!(
            rejected_share,
            RetainedAllocationError::AccountPoisoned(_)
        ));
        assert_eq!(account.snapshot(), before_rejected_share);
    }

    #[test]
    fn poison_first_start_rejects_without_builder_or_account_side_effects() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 16);
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("poison probe must retain"));
        let started = StdArc::new(Barrier::new(2));
        let poisoned = StdArc::new(Barrier::new(2));
        let thread_started = StdArc::clone(&started);
        let thread_poisoned = StdArc::clone(&poisoned);
        let poison_thread = thread::spawn(move || {
            thread_started.wait();
            drop(poisoner);
            thread_poisoned.wait();
        });
        started.wait();
        poisoned.wait();
        poison_thread
            .join()
            .unwrap_or_else(|_| panic!("poison thread must be contained"));

        let before = account.snapshot();
        let builder_ran = AtomicBool::new(false);
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let plan = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        let result =
            TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
                builder_ran.store(true, Ordering::SeqCst);
                Ok(7)
            });

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::AccountPoisoned(_)
            ))
        ));
        assert!(!builder_ran.load(Ordering::SeqCst));
        assert_eq!(account.snapshot(), before);
    }

    #[test]
    fn admission_first_start_panic_after_external_poison_records_distinct_builder_fault() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, 8_192, 32);
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("startup panic poison probe must retain"));
        let worker_account = account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("startup worker account share must precede poison"));
        let shares_before_start = account.share_call_count();
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let plan = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        let builder_entered = StdArc::new(Barrier::new(2));
        let builder_continue = StdArc::new(Barrier::new(2));
        let worker_entered = StdArc::clone(&builder_entered);
        let worker_continue = StdArc::clone(&builder_continue);
        let worker = thread::spawn(move || {
            TestOperation::try_start(
                OperationControl::default(),
                limits,
                &worker_account,
                plan,
                || {
                    worker_entered.wait();
                    worker_continue.wait();
                    std::panic::panic_any(vec![0_u8; 8]);
                },
            )
        });

        builder_entered.wait();
        drop(poisoner);
        let after_external_poison = account.snapshot();
        assert!(after_external_poison.poisoned());
        let first_evidence = after_external_poison.first_poison_evidence();
        builder_continue.wait();
        let result = worker
            .join()
            .unwrap_or_else(|_| panic!("startup builder panic must be contained"));
        let after_builder_panic = account.snapshot();

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
            ))
        ));
        assert_eq!(
            after_builder_panic.poison_event_count(),
            after_external_poison.poison_event_count() + 1
        );
        assert_eq!(after_builder_panic.first_poison_evidence(), first_evidence);
        assert_eq!(
            after_builder_panic.live_retained_bytes(),
            after_external_poison.live_retained_bytes()
        );
        assert_eq!(
            after_builder_panic.live_retained_allocations(),
            after_external_poison.live_retained_allocations()
        );
        assert_eq!(account.share_call_count(), shares_before_start + 4);
    }

    #[test]
    fn admission_first_physical_retain_completes_then_later_retain_is_rejected() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 16);
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("physical-race poison probe must retain"));
        let worker_account = account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("worker account share must precede poison"));
        let builder_entered = StdArc::new(Barrier::new(2));
        let builder_continue = StdArc::new(Barrier::new(2));
        let worker_entered = StdArc::clone(&builder_entered);
        let worker_continue = StdArc::clone(&builder_continue);
        let worker = thread::spawn(move || {
            worker_account
                .try_retain_with(empty_plan(), || {
                    worker_entered.wait();
                    worker_continue.wait();
                    Ok::<_, RetainedBuildFailureCode>(99_u64)
                })
                .unwrap_or_else(|_| panic!("admission-first retain must complete"))
        });

        builder_entered.wait();
        drop(poisoner);
        let after_poison = account.snapshot();
        assert!(after_poison.poisoned());
        builder_continue.wait();
        let retained = worker
            .join()
            .unwrap_or_else(|_| panic!("retaining worker must finish"));
        assert_eq!(retained.as_ref(), &99);
        assert_eq!(account.snapshot(), after_poison);

        let later_builder_ran = AtomicBool::new(false);
        let later = account
            .try_retain_with(empty_plan(), || {
                later_builder_ran.store(true, Ordering::SeqCst);
                Ok::<_, RetainedBuildFailureCode>(100_u64)
            })
            .unwrap_err();
        assert!(matches!(later, RetainedAllocationError::AccountPoisoned(_)));
        assert!(!later_builder_ran.load(Ordering::SeqCst));
        assert_eq!(account.snapshot(), after_poison);
        drop(retained);
    }

    #[test]
    fn admission_first_builder_error_after_poison_retains_charge_without_rollback_clone() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 16);
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("builder-error poison probe must retain"));
        let worker_account = account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("worker account share must precede poison"));
        let shares_before_retain = account.share_call_count();
        let builder_entered = StdArc::new(Barrier::new(2));
        let builder_continue = StdArc::new(Barrier::new(2));
        let worker_entered = StdArc::clone(&builder_entered);
        let worker_continue = StdArc::clone(&builder_continue);
        let worker = thread::spawn(move || {
            worker_account
                .try_retain_with::<u64>(empty_plan(), || {
                    worker_entered.wait();
                    worker_continue.wait();
                    Err(BUILD_FAILED)
                })
                .expect_err("admission-first builder error must not publish storage")
        });

        builder_entered.wait();
        drop(poisoner);
        let after_poison = account.snapshot();
        assert!(after_poison.poisoned());
        builder_continue.wait();
        let error = worker
            .join()
            .unwrap_or_else(|_| panic!("builder-error worker must finish"));

        assert!(matches!(
            error,
            RetainedAllocationError::AccountPoisonedWithRetainedCharge(_)
        ));
        assert_eq!(account.snapshot(), after_poison);
        assert_eq!(account.share_call_count(), shares_before_retain + 1);
        assert!(matches!(
            account.try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(1_u64)),
            Err(RetainedAllocationError::AccountPoisoned(_))
        ));
    }

    #[test]
    fn admission_first_direct_fallback_ends_in_finalization_containment() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("post-start poison probe must retain"));
        let started = StdArc::new(Barrier::new(2));
        let poisoned = StdArc::new(Barrier::new(2));
        let thread_started = StdArc::clone(&started);
        let thread_poisoned = StdArc::clone(&poisoned);
        let poison_thread = thread::spawn(move || {
            thread_started.wait();
            drop(poisoner);
            thread_poisoned.wait();
        });
        started.wait();
        poisoned.wait();
        poison_thread
            .join()
            .unwrap_or_else(|_| panic!("poison thread must be contained"));
        let before_prepare = account.snapshot();
        let first_evidence = before_prepare.first_poison_evidence();
        let ledger = operation.coordinator.retained_ledger().test_probe();
        let active_before = ledger.active_reservations();
        let live_before = ledger.current_live_bytes();

        let result =
            operation.prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(91));
        let after_prepare = account.snapshot();
        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
                if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
                    && cause.account_poisoned().snapshot() == after_prepare
        ));
        assert_eq!(
            after_prepare.poison_event_count(),
            before_prepare.poison_event_count() + 2
        );
        assert!(after_prepare.poison_event_count_exact());
        assert!(after_prepare.poison_event_domain_valid());
        assert_eq!(after_prepare.first_poison_evidence(), first_evidence);
        assert_eq!(ledger.active_reservations(), active_before - 2);
        assert!(ledger.current_live_bytes() > 0);
        assert!(ledger.current_live_bytes() < live_before);
    }

    #[test]
    fn poisoned_inherited_share_has_no_lookup_work_clone_or_target_side_effect() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let (source, source_slot, source_evidence, _source_evidence_slot) =
            publish_scalar_source(&account);
        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("target publication must open"));
        let source_allocation = source
            .aggregate
            .inner
            .table
            .as_ref()
            .member_allocation(source_slot.identity, source.aggregate.inner.set_id)
            .unwrap_or_else(|| panic!("source slot must resolve before poison"));
        let source_share_calls = source_allocation.share_call_count();
        let target_len = assembly.canonical_len();
        let ledger = assembly.test_ledger_probe();
        let active = ledger.active_reservations();
        let live = ledger.current_live_bytes();
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("share poison probe must retain"));
        drop(poisoner);
        let poisoned = account.snapshot();
        let _ = take_test_member_lookups();

        let result = assembly.try_share_canonical(source.aggregate(), source_slot);
        assert!(matches!(
            result,
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(_)
            ))
        ));
        assert_eq!(assembly.canonical_len(), target_len);
        assert_eq!(ledger.active_reservations(), active);
        assert_eq!(ledger.current_live_bytes(), live);
        assert_eq!(source_allocation.share_call_count(), source_share_calls);
        assert_eq!(take_test_member_lookups(), 0);
        assert_eq!(account.snapshot(), poisoned);
        drop(source_evidence);
    }

    #[test]
    fn admission_first_inherited_share_completes_then_later_share_is_rejected() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let (source, source_slot, source_evidence, _source_evidence_slot) =
            publish_scalar_source(&account);
        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(3, 2),
            OperationControl::default(),
        );
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(2, 1))
            .unwrap_or_else(|_| panic!("admission-first share target must open"));
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("share-race poison probe must retain"));
        let target_account = assembly
            .account
            .as_ref()
            .unwrap_or_else(|| panic!("target retains account"));
        let share_admission = target_account
            .try_admit_account_action()
            .unwrap_or_else(|_| panic!("share action must be admitted before poison"));
        drop(poisoner);
        let poisoned = account.snapshot();
        let source_allocation = source
            .aggregate
            .inner
            .table
            .as_ref()
            .member_allocation(source_slot.identity, source.aggregate.inner.set_id)
            .unwrap_or_else(|| panic!("source slot must resolve"));
        let source_share_calls = source_allocation.share_call_count();
        let ledger = assembly
            .operation
            .as_ref()
            .unwrap_or_else(|| panic!("target retains operation"))
            .coordinator
            .retained_ledger();
        let identity = try_share_retained_block_after_admission::<
            TestOwner,
            TestPhase,
            TestDimension,
            u64,
            CanonicalRootTag,
        >(
            ledger,
            share_admission,
            assembly.handle_share_work,
            assembly
                .canonical_table
                .as_mut()
                .and_then(PolicyRetainedAllocation::table_mut_during_construction)
                .unwrap_or_else(|| panic!("target retains canonical table")),
            assembly.set_id,
            RetainedMembershipDestination::Canonical,
            0,
            1,
            source_allocation,
        )
        .unwrap_or_else(|_| panic!("admission-first share must complete"));
        assert_eq!(identity.index, 0);
        assert_eq!(assembly.canonical_len(), 1);
        assert_eq!(source_allocation.share_call_count(), source_share_calls + 1);
        assert_eq!(account.snapshot(), poisoned);

        let before_later = assembly.canonical_len();
        let later = assembly.try_share_canonical(source.aggregate(), source_slot);
        assert!(matches!(
            later,
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(_)
            ))
        ));
        assert_eq!(assembly.canonical_len(), before_later);
        drop(source_evidence);
    }

    #[test]
    fn prepared_handoff_is_a_pure_move_after_later_account_poison() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("handoff poison probe must retain"));
        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let prepared = operation
            .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(92))
            .unwrap_or_else(|_| panic!("healthy account must prepare fallback"));
        drop(poisoner);
        let before_handoff = account.snapshot();

        let published = handoff_for_test(prepared);
        assert_eq!(account.snapshot(), before_handoff);
        assert!(matches!(
            published,
            PublishedRetainedPublicationParts::MinimalFallback { .. }
        ));
    }

    #[test]
    fn eligibility_first_capability_completes_preparation_after_later_account_poison() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("eligibility race poison probe must retain"));
        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let eligibility_entered = StdArc::new(Barrier::new(2));
        let eligibility_resume = StdArc::new(Barrier::new(2));
        let worker_entered = StdArc::clone(&eligibility_entered);
        let worker_resume = StdArc::clone(&eligibility_resume);
        let worker = thread::spawn(move || {
            install_publication_eligibility_pause(worker_entered, worker_resume);
            operation.prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(93))
        });

        eligibility_entered.wait();
        drop(poisoner);
        let poisoned = account.snapshot();
        assert!(poisoned.poisoned());
        eligibility_resume.wait();

        let prepared = worker
            .join()
            .unwrap_or_else(|_| panic!("eligibility-first worker must remain contained"))
            .unwrap_or_else(|_| panic!("eligibility-first preparation must complete"));
        assert_eq!(account.snapshot(), poisoned);
        let published = handoff_for_test(prepared);
        assert!(matches!(
            published,
            PublishedRetainedPublicationParts::MinimalFallback { .. }
        ));
    }

    #[test]
    fn primary_eligibility_first_capability_completes_after_later_account_poison() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        DROP_COUNT.store(0, Ordering::SeqCst);
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let poisoner = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
            })
            .unwrap_or_else(|_| panic!("primary eligibility race poison probe must retain"));
        let operation = test_operation_on_account_with_control(
            &account,
            64,
            RetainedBatchLimits::new(2, 2),
            OperationControl::default(),
        );
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("primary eligibility publication must open"));
        let (canonical, evidence) = retain_scalar_success(&mut assembly);
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session.canonical(canonical) == Some(&11)
                    && session.outcome_evidence(evidence) == Some(&7)
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("primary eligibility candidate must validate"),
            };
        let eligibility_entered = StdArc::new(Barrier::new(2));
        let eligibility_resume = StdArc::new(Barrier::new(2));
        let worker_entered = StdArc::clone(&eligibility_entered);
        let worker_resume = StdArc::clone(&eligibility_resume);
        let worker = thread::spawn(move || {
            install_publication_eligibility_pause(worker_entered, worker_resume);
            validated.try_prepare(final_progress())
        });

        eligibility_entered.wait();
        drop(poisoner);
        let poisoned = account.snapshot();
        assert!(poisoned.poisoned());
        eligibility_resume.wait();

        let prepared = worker
            .join()
            .unwrap_or_else(|_| panic!("primary eligibility worker must remain contained"))
            .unwrap_or_else(|_| panic!("primary eligibility preparation must complete"));
        assert_eq!(account.snapshot(), poisoned);
        let published = handoff_for_test(prepared);
        assert!(matches!(
            published,
            PublishedRetainedPublicationParts::Success { .. }
        ));
    }

    #[test]
    fn release_underflow_latches_counter_invariant_without_saturating_event_count() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);

        let _ = account.release_retained_charge(AdmissionCharge::new(1, 1));
        let first = account.snapshot();
        assert!(first.poisoned());
        assert_eq!(first.poison_event_count(), 1);
        assert!(first.poison_event_count_exact());
        assert!(!first.poison_event_domain_valid());
        assert_eq!(
            first
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::CounterInvariantViolated)
        );

        let _ = account.release_retained_charge(AdmissionCharge::new(1, 1));
        let second = account.snapshot();
        assert_eq!(second.poison_event_count(), 2);
        assert!(second.poison_event_count_exact());
        assert!(!second.poison_event_domain_valid());
        assert_eq!(
            second.first_poison_evidence(),
            first.first_poison_evidence()
        );
    }

    #[test]
    fn pending_single_rollback_underflow_records_one_in_domain_counter_fault() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let charge = AdmissionCharge::new(1, 1);
        let admission = account
            .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
            .unwrap_or_else(|_| panic!("single pending charge must admit"));
        {
            let mut state = lock_recovering_poison(&account.inner.value().linear);
            state.counters.live_bytes = 0;
            state.counters.live_allocations = 0;
        }

        let rollback =
            admission.rollback_with_fault(Some(AccountPoisonReason::OwnershipGraphUnverified));
        assert!(matches!(
            rollback,
            PendingRetainedRollback::ChargeReleaseUnverified(_)
        ));
        let snapshot = account.snapshot();
        assert_eq!(snapshot.poison_event_count(), 1);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::CounterInvariantViolated)
        );
    }

    #[test]
    fn pending_pair_rollback_underflow_records_one_in_domain_counter_fault() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let pair = RetainedAdmissionPairPlan::try_new(
            AdmissionCharge::new(1, 1),
            AdmissionCharge::new(1, 1),
        )
        .unwrap_or_else(|_| panic!("pair charge must qualify"));
        let (admissions, _owners) = account
            .try_admit_start_retained_pair_after_action(
                account.try_admit_account_action().expect("account action"),
                pair,
                admitted_startup_share_work(&account),
            )
            .unwrap_or_else(|_| panic!("pending pair must admit"));
        {
            let mut state = lock_recovering_poison(&account.inner.value().linear);
            state.counters.live_bytes = 0;
            state.counters.live_allocations = 0;
        }

        let rollback =
            admissions.rollback_with_fault(Some(AccountPoisonReason::OwnershipGraphUnverified));
        assert!(matches!(
            rollback,
            PendingRetainedRollback::ChargeReleaseUnverified(_)
        ));
        let snapshot = account.snapshot();
        assert_eq!(snapshot.poison_event_count(), 1);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::CounterInvariantViolated)
        );
    }

    #[test]
    fn startup_admission_guard_underflow_records_one_counter_fault() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let pair = RetainedAdmissionPairPlan::try_new(
            AdmissionCharge::new(1, 1),
            AdmissionCharge::new(1, 1),
        )
        .unwrap_or_else(|_| panic!("guard pair charge must qualify"));
        let (admissions, owners) = account
            .try_admit_start_retained_pair_after_action(
                account.try_admit_account_action().expect("account action"),
                pair,
                admitted_startup_share_work(&account),
            )
            .unwrap_or_else(|_| panic!("guard pair must admit"));
        let guard = StartupAdmissionGuard::new(&account, admissions, owners);
        {
            let mut state = lock_recovering_poison(&account.inner.value().linear);
            state.counters.live_bytes = 0;
            state.counters.live_allocations = 0;
        }

        drop(guard);
        let snapshot = account.snapshot();
        assert_eq!(snapshot.poison_event_count(), 1);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::CounterInvariantViolated)
        );
    }

    #[test]
    fn startup_admission_guard_unwind_records_exactly_one_outer_fault() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let pair = RetainedAdmissionPairPlan::try_new(
            AdmissionCharge::new(1, 1),
            AdmissionCharge::new(1, 1),
        )
        .unwrap_or_else(|_| panic!("unwind pair charge must qualify"));
        let (admissions, owners) = account
            .try_admit_start_retained_pair_after_action(
                account.try_admit_account_action().expect("account action"),
                pair,
                admitted_startup_share_work(&account),
            )
            .unwrap_or_else(|_| panic!("unwind pair must admit"));

        let unwind = catch_unwind(AssertUnwindSafe(|| {
            let _guard = StartupAdmissionGuard::new(&account, admissions, owners);
            panic!("inject startup host unwind");
        }));
        assert!(unwind.is_err());
        let snapshot = account.snapshot();
        assert_eq!(snapshot.live_retained_bytes(), 0);
        assert_eq!(snapshot.live_retained_allocations(), 0);
        assert_eq!(snapshot.poison_event_count(), 1);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::OwnershipGraphUnverified)
        );
    }

    #[test]
    fn qualified_startup_capacity_failure_rolls_back_without_poisoning() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let pair = RetainedAdmissionPairPlan::try_new(
            AdmissionCharge::new(1, 1),
            AdmissionCharge::new(1, 1),
        )
        .unwrap_or_else(|_| panic!("capacity pair charge must qualify"));
        let (admissions, owners) = account
            .try_admit_start_retained_pair_after_action(
                account.try_admit_account_action().expect("account action"),
                pair,
                admitted_startup_share_work(&account),
            )
            .unwrap_or_else(|_| panic!("capacity pair must admit"));
        let guard = StartupAdmissionGuard::new(&account, admissions, owners);

        let error = map_qualified_startup_batch_error::<TestDimension>(
            AttributionBatchCreateError::CapacityOverflow,
            |outer_reason| guard.rollback(outer_reason),
        );
        assert!(matches!(error, RetainedWriterError::BatchCapacity));
        let snapshot = account.snapshot();
        assert_eq!(snapshot.live_retained_bytes(), 0);
        assert_eq!(snapshot.live_retained_allocations(), 0);
        assert!(!snapshot.poisoned());
        assert_eq!(snapshot.poison_event_count(), 0);
    }

    #[test]
    fn action_admission_counter_exhaustion_is_a_clean_non_poisoning_rejection() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        lock_recovering_poison(&account.inner.value().linear).admitted_account_actions = u64::MAX;
        let before = account.snapshot();

        let result = account.try_admit_account_action();
        assert!(matches!(
            result,
            Err(AccountActionAdmissionError::AccountingOverflow)
        ));
        assert_eq!(account.snapshot(), before);
        assert!(!account.snapshot().poisoned());
    }

    #[test]
    fn side_owner_escape_keeps_physical_and_operation_attribution_fail_closed() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("side-owner publication must open"));
        let ledger = assembly
            .operation
            .as_ref()
            .expect("side-owner assembly retains operation")
            .coordinator
            .retained_ledger()
            .test_probe();
        let account = assembly
            .account
            .as_ref()
            .expect("side-owner assembly retains account")
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted"));
        let side_owner = StdArc::new(StdMutex::new(None));
        let escaped = StdArc::clone(&side_owner);
        let active_before = ledger.active_reservations();
        let live_before = ledger.current_live_bytes();
        let physical_before = account.snapshot();
        let plan = RetainedAllocationPlan::new(64, 1).unwrap();

        let result = assembly.try_retain_canonical::<CanonicalRootTag>(plan, move |_emplacer| {
            *lock_recovering_poison(&escaped) = Some(vec![0_u8; 64]);
            panic!("builder escaped side-owned storage")
        });

        assert!(matches!(
            result,
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
            ))
        ));
        assert!(lock_recovering_poison(&side_owner).is_some());
        assert_eq!(ledger.active_reservations(), active_before + 1);
        assert!(ledger.current_live_bytes() > live_before);
        let physical_after = account.snapshot();
        assert!(physical_after.live_retained_bytes() > physical_before.live_retained_bytes() + 64);
        assert!(
            physical_after.live_retained_allocations()
                > physical_before.live_retained_allocations()
        );
        let retained_live = ledger.current_live_bytes();
        drop(lock_recovering_poison(&side_owner).take());
        assert_eq!(ledger.current_live_bytes(), retained_live);
        drop(assembly);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn injected_shared_allocation_failure_does_not_run_the_builder() {
        static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

        struct DropProbe {
            account: PhysicalStorageAccount,
        }

        impl Drop for DropProbe {
            fn drop(&mut self) {
                OBSERVED_LIVE_BYTES.store(
                    self.account.snapshot().live_retained_bytes(),
                    Ordering::SeqCst,
                );
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
        fail_next_final_owner_allocation();
        let result = account.try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(DropProbe {
                account: account
                    .share_after_test_admission()
                    .unwrap_or_else(|_| panic!("test account share must be admitted")),
            })
        });

        assert!(matches!(
            result,
            Err(RetainedAllocationError::AllocationFailed)
        ));
        assert_eq!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst), 0);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }

    #[test]
    fn allocation_failure_precedes_payload_construction_and_releases_the_lease() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        struct PanickingDrop;

        impl Drop for PanickingDrop {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
                std::panic::panic_any(vec![0_u8; 8]);
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        DROP_COUNT.store(0, Ordering::SeqCst);
        fail_next_final_owner_allocation();
        let result = catch_unwind(AssertUnwindSafe(|| {
            account.try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDrop)
            })
        }));

        assert!(matches!(
            result,
            Ok(Err(RetainedAllocationError::AllocationFailed))
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn allocation_failure_rolls_back_block_attribution_before_build() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        let factory = account_factory(1);
        let operation =
            test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
        let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("publication assembly must open"));
        let probe = assembly
            .operation
            .as_ref()
            .expect("assembly retains the operation")
            .coordinator
            .retained_ledger()
            .test_probe();
        let before_active = probe.active_reservations();
        let before_live = probe.current_live_bytes();

        DROP_COUNT.store(0, Ordering::SeqCst);
        fail_next_final_owner_allocation();
        let error = assembly
            .try_retain_canonical(empty_plan(), |emplacer| {
                Ok(emplacer.place(PanickingDropProbe(&DROP_COUNT), CanonicalRootTag))
            })
            .expect_err("injected shared allocation must fail");

        assert!(matches!(
            error,
            RetainedPublicationError::Writer(RetainedWriterError::Physical(
                RetainedAllocationError::AllocationFailed
            ))
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
        assert_eq!(probe.active_reservations(), before_active);
        assert_eq!(probe.current_live_bytes(), before_live);
        drop(assembly);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn private_shares_charge_once_and_release_only_after_the_final_owner() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let first = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(*b"payload!")
            })
            .unwrap();
        let charged = first.charged_bytes();
        assert_eq!(first.share_call_count(), 0);
        let second = first.share_after_test_admission();

        assert!(first.ptr_eq(&second));
        assert_eq!(first.share_call_count(), 1);
        assert_eq!(second.share_call_count(), 1);
        let third = second.share_after_test_admission();
        assert_eq!(first.share_call_count(), 2);
        assert_eq!(second.share_call_count(), 2);
        assert_eq!(third.share_call_count(), 2);
        assert_eq!(account.snapshot().live_retained_bytes(), charged);
        drop(first);
        assert_eq!(account.snapshot().live_retained_bytes(), charged);
        drop(second);
        assert_eq!(account.snapshot().live_retained_bytes(), charged);
        drop(third);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }

    #[test]
    fn concurrent_shared_drops_select_exactly_one_final_owner() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        struct DropCount;

        impl Drop for DropCount {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        DROP_COUNT.store(0, Ordering::SeqCst);
        let first = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(DropCount)
            })
            .unwrap();
        let second = first.share_after_test_admission();
        let third = first.share_after_test_admission();
        let start = StdArc::new(Barrier::new(4));
        let mut threads = Vec::new();
        for retained in [first, second, third] {
            let start = StdArc::clone(&start);
            threads.push(thread::spawn(move || {
                start.wait();
                drop(retained);
            }));
        }

        start.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn final_shared_owner_contains_payload_drop_panic_and_retains_the_lease() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        struct PanickingDrop;

        impl Drop for PanickingDrop {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
                std::panic::panic_any(vec![0_u8; 8]);
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        DROP_COUNT.store(0, Ordering::SeqCst);
        let first = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(PanickingDrop)
            })
            .unwrap();
        let charged = first.charged_bytes();
        let second = first.share_after_test_admission();
        drop(first);

        assert!(catch_unwind(AssertUnwindSafe(|| drop(second))).is_ok());
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(account.snapshot().live_retained_bytes(), charged);
        assert_eq!(account.snapshot().live_retained_allocations(), 1);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn payload_drop_barrier_keeps_headroom_unavailable_until_cleanup_finishes() {
        static ENTERED: Barrier = Barrier::new(2);
        static RELEASE: Barrier = Barrier::new(2);

        struct BlockingDrop;

        impl Drop for BlockingDrop {
            fn drop(&mut self) {
                ENTERED.wait();
                RELEASE.wait();
            }
        }

        let outer = FinalOwnerAllocation::<
            RetainedFinalOwnerInner<RetainedPayload<BlockingDrop>, LeafRetainedPolicy>,
        >::checked_layout_bytes()
        .unwrap();
        let factory = account_factory(1);
        let account = account(&factory, outer, 1);
        let retained = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(BlockingDrop)
            })
            .unwrap();
        let drop_thread = thread::spawn(move || drop(retained));

        ENTERED.wait();
        assert!(matches!(
            account.try_retain_with::<BlockingDrop>(empty_plan(), || {
                panic!("builder must not run before physical admission")
            }),
            Err(RetainedAllocationError::ByteLimitExceeded { .. })
                | Err(RetainedAllocationError::AllocationLimitExceeded { .. })
        ));
        RELEASE.wait();
        drop_thread.join().unwrap();
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        let replacement = account
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(()))
            .expect("headroom becomes available after payload cleanup and lease release");
        drop(replacement);
    }

    #[test]
    fn payload_drop_observes_charge_and_factory_outlives_public_account_handle() {
        static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

        struct DropProbe {
            account: PhysicalStorageAccount,
        }

        impl Drop for DropProbe {
            fn drop(&mut self) {
                OBSERVED_LIVE_BYTES.store(
                    self.account.snapshot().live_retained_bytes(),
                    Ordering::SeqCst,
                );
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
        let retained = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(DropProbe {
                    account: account
                        .share_after_test_admission()
                        .unwrap_or_else(|_| panic!("test account share must be admitted")),
                })
            })
            .unwrap();
        let charged = retained.charged_bytes();

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
        drop(retained);
        assert_eq!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst), charged);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn account_identity_is_opaque_and_foreign_accounts_do_not_match() {
        let factory = account_factory(2);
        let first_account = account(&factory, 4_096, 8);
        let second_account = account(&factory, 4_096, 8);
        let retained = first_account
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(5_u64))
            .unwrap();

        assert!(retained.belongs_to(&first_account));
        assert!(!retained.belongs_to(&second_account));
        let shared_first = first_account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted"));
        assert!(first_account.is_same_account(&shared_first));
        assert!(!first_account.is_same_account(&second_account));
    }

    #[test]
    fn foreign_account_cannot_authorize_retained_attribution_release() {
        let factory = account_factory(2);
        let first_account = account(&factory, 4_096, 8);
        let second_account = account(&factory, 4_096, 8);
        let retained = first_account
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(5_u64))
            .unwrap_or_else(|_| panic!("foreign-account probe allocation must retain"));
        let first_before = first_account.snapshot();

        assert!(matches!(
            drop_retained_allocation_before_attribution(retained, &second_account),
            RetainedStorageDropOutcome::ForeignOwnershipFault(_)
        ));
        assert_eq!(
            first_account.snapshot().live_retained_bytes(),
            first_before.live_retained_bytes()
        );
        assert_eq!(
            first_account.snapshot().live_retained_allocations(),
            first_before.live_retained_allocations()
        );
        assert_eq!(second_account.snapshot().live_retained_bytes(), 0);
        drop(first_account);
        drop(second_account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn concurrent_exact_limit_admission_is_linearizable() {
        let outer = FinalOwnerAllocation::<
            RetainedFinalOwnerInner<RetainedPayload<u64>, LeafRetainedPolicy>,
        >::checked_layout_bytes()
        .unwrap();
        let factory = account_factory(1);
        let account = account(&factory, outer, 1);
        let start = StdArc::new(Barrier::new(3));
        let attempts_complete = StdArc::new(Barrier::new(3));
        let release_owner = StdArc::new(Barrier::new(2));
        let successes = StdArc::new(AtomicU64::new(0));
        let mut threads = Vec::new();
        for value in [1_u64, 2_u64] {
            let account = account
                .share_after_test_admission()
                .unwrap_or_else(|_| panic!("test account share must be admitted"));
            let start = StdArc::clone(&start);
            let attempts_complete = StdArc::clone(&attempts_complete);
            let release_owner = StdArc::clone(&release_owner);
            let successes = StdArc::clone(&successes);
            threads.push(thread::spawn(move || {
                start.wait();
                let retained = account
                    .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(value))
                    .ok();
                if retained.is_some() {
                    successes.fetch_add(1, Ordering::SeqCst);
                }
                attempts_complete.wait();
                if let Some(retained) = retained {
                    release_owner.wait();
                    drop(retained);
                }
            }));
        }

        start.wait();
        attempts_complete.wait();
        assert_eq!(successes.load(Ordering::SeqCst), 1);
        release_owner.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }

    #[test]
    fn final_owner_source_uses_only_the_qualified_extraction_path() {
        let root_production = retained_root_production_source();
        let final_owner = include_str!("retained/final_owner.rs");
        let production = retained_production_source();

        for extraction in [
            "Arc::into_unique",
            "Arc::try_unique",
            "UniqueArc::into_inner",
        ] {
            assert!(
                !root_production.contains(extraction),
                "final-owner extraction `{extraction}` escaped final_owner.rs"
            );
            assert!(
                final_owner.contains(extraction),
                "final_owner.rs lost required extraction `{extraction}`"
            );
        }
        assert_eq!(
            final_owner.matches("Arc::into_unique(shared)").count(),
            2,
            "ordinary release and final shared-owner drop are the only non-strict transitions"
        );
        assert_eq!(
            final_owner.matches("Arc::try_unique(shared)").count(),
            1,
            "strict pre-attribution cleanup has one qualified uniqueness transition"
        );
        assert_eq!(
            final_owner.matches("UniqueArc::into_inner(unique)").count(),
            3,
            "every qualified uniqueness transition extracts through the generic engine"
        );
        for forbidden in [
            "Arc::try_unwrap",
            "Arc::into_raw",
            "Arc::from_raw",
            "ArcBorrow",
            "ArcUnion",
            "OffsetArc",
            "ThinArc",
            "impl<T> Clone for RetainedAllocation",
            "impl Clone for PhysicalStorageAccount {",
        ] {
            assert!(
                !production.contains(forbidden),
                "forbidden final-owner bypass `{forbidden}` was introduced"
            );
        }
    }

    #[test]
    fn generic_retained_authority_remains_crate_private() {
        let production = retained_production_source();
        let root_exports = include_str!("lib.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();

        assert!(
            !root_exports.contains("pub use retained::*"),
            "the private retained module must never gain a glob re-export"
        );
        let export_prefix = "pub use retained::{";
        assert_eq!(root_exports.matches(export_prefix).count(), 1);
        let export_tail = root_exports
            .split_once(export_prefix)
            .expect("the crate root has one curated retained export block")
            .1;
        let export_body = export_tail
            .split_once("};")
            .expect("the curated retained export block is terminated")
            .0;
        let mut actual_exports: Vec<_> = export_body
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        let mut expected_exports = vec![
            "AccountPoisonEvidence",
            "AccountPoisonReason",
            "AccountPoisoned",
            "FinalizationContainmentError",
            "PhysicalStorageAccount",
            "PhysicalStorageAccountCreateError",
            "PhysicalStorageAccountFactory",
            "PhysicalStorageAccountFactoryCreateError",
            "PhysicalStorageAccountFactoryLimits",
            "PhysicalStorageAccountFactorySnapshot",
            "PhysicalStorageAccountLimits",
            "PhysicalStorageAccountSnapshot",
            "PhysicalStorageLimitError",
        ];
        actual_exports.sort_unstable();
        expected_exports.sort_unstable();
        assert_eq!(actual_exports, expected_exports);

        let mut public_declarations: Vec<_> = production
            .lines()
            .filter(|line| {
                line.starts_with("pub struct ")
                    || line.starts_with("pub enum ")
                    || line.starts_with("pub const ")
                    || line.starts_with("pub type ")
                    || line.starts_with("pub trait ")
            })
            .filter_map(|line| line.split_whitespace().nth(2))
            .map(|name| name.trim_end_matches(['{', ':']))
            .collect();
        public_declarations.sort_unstable();
        assert_eq!(public_declarations, expected_exports);

        assert!(production.contains(
            "type RetainedAllocation<T> = PolicyRetainedAllocation<T, LeafRetainedPolicy>;"
        ));
        assert!(!production.contains("pub(crate) type RetainedAllocation"));
        assert!(production.contains("\nstruct RetainedPart<"));
        assert!(production.contains("\nenum RetainedClosureValidationError"));
        for forbidden in [
            "RetainedAllocation",
            "RetainedPart",
            "RetainedOperation",
            "RetainedAllocationPlan",
            "BlockEmplacer",
            "RetainedClosureValidationError",
            "RetainedSuccessClosureProof",
            "RetainedEvidenceClosureProof",
            "PublishedRetainedAggregate",
            "FinalOwnerAllocation",
            "AdmissionLease",
        ] {
            assert!(
                !actual_exports.contains(&forbidden),
                "raw retained authority `{forbidden}` escaped the crate root"
            );
        }

        {
            let method = "try_retain_with_admitted_handle";
            let needle = format!("fn {method}");
            let signatures: Vec<_> = production
                .lines()
                .filter(|line| line.contains(&needle))
                .collect();
            assert_eq!(
                signatures.len(),
                1,
                "raw physical mutator `{method}` must have one implementation"
            );
            assert!(
                signatures[0].trim_start().starts_with(&needle),
                "raw physical mutator `{method}` escaped the retained module: {}",
                signatures[0]
            );
        }
        assert!(
            !production.contains("fn share_after_admission"),
            "raw account sharing must not exist in production"
        );
        let count_test_only_wrappers = |candidate: &str| {
            candidate
                .lines()
                .zip(candidate.lines().skip(1))
                .filter(|(attribute, signature)| {
                    attribute.trim_start() == test_attribute.as_str()
                        && *signature == "    fn try_retain_with<T>("
                })
                .count()
        };
        assert_eq!(
            count_test_only_wrappers(&production),
            1,
            "unadmitted physical retain wrapper must remain test-only"
        );
        let crlf_production = production.replace("\r\n", "\n").replace('\n', "\r\n");
        assert_eq!(
            count_test_only_wrappers(&crlf_production),
            1,
            "unadmitted physical retain wrapper gate must be line-ending independent"
        );
        for forbidden in [
            "fn try_prepare_success(",
            "fn try_prepare_rich_failure(",
            "pub struct RetainedSuccessClosureProof",
            "pub struct RetainedEvidenceClosureProof",
            "pub fn attach(",
            "pub fn detach(",
            "pub fn get_mut(",
            "pub fn into_block(",
            "pub fn replace(",
            "pub fn retag(",
            "pub fn retire(",
            "fn reset_poison(",
            "fn clear_poison(",
            "fn unpoison(",
            "fn rebrand_account(",
            "fn bypass_account_poison(",
            "RetainedAllocationVisitor",
        ] {
            assert!(
                !production.contains(forbidden),
                "raw retained authority `{forbidden}` escaped the deep module"
            );
        }

        let normalized = production.replace("\r\n", "\n");
        assert!(normalized.contains(
            "fn try_publication_eligibility(&self) -> Result<PublicationEligibility, AccountPoisoned>"
        ));
        assert_eq!(
            normalized.matches(".try_publication_eligibility()").count(),
            2
        );
        let eligible_method = |state_marker: &str| {
            normalized
                .split_once(state_marker)
                .unwrap_or_else(|| panic!("missing eligible state `{state_marker}`"))
                .1
                .split_once("fn into_prepared")
                .unwrap_or_else(|| panic!("missing consuming handoff for `{state_marker}`"))
                .1
                .split_once("\n    }\n}\n\nimpl")
                .unwrap_or_else(|| panic!("unterminated consuming handoff for `{state_marker}`"))
                .0
                .to_owned()
        };
        for eligible_tail in [
            eligible_method("struct EligibleTerminalFreezeGuard"),
            eligible_method("struct EligibleFallbackFreezeGuard"),
        ] {
            assert!(eligible_tail.contains("freeze_terminal_evidence("));
            assert!(eligible_tail.contains("physical.into_publication("));
            for forbidden in [
                "record_poison",
                "contain_",
                "catch_unwind",
                "try_retain",
                "try_discard",
                "drop_retained",
                "release_retained",
                "try_allocate",
                "assign_membership",
                "table_mut",
            ] {
                assert!(
                    !eligible_tail.contains(forbidden),
                    "post-eligibility tail regained fallible authority `{forbidden}`"
                );
            }
        }
        let freeze_end = format!("\n}}\n\n{test_attribute}");
        let freeze_tail = normalized
            .split_once("fn freeze_terminal_evidence")
            .expect("terminal evidence freeze seam exists")
            .1
            .split_once(&freeze_end)
            .expect("terminal evidence freeze seam is bounded")
            .0;
        assert!(freeze_tail.contains("_eligibility: PublicationEligibility"));
        for required in [
            "let snapshot = proof.freeze();",
            "drop(discarded_account);",
            "evidence.allocation().as_ref().freeze_from(&snapshot);",
            "drop(snapshot);",
        ] {
            assert!(
                freeze_tail.contains(required),
                "eligible freeze tail lost required pure step `{required}`"
            );
        }
        for forbidden in [
            "record_poison",
            "catch_unwind",
            "try_retain",
            "try_discard",
            "drop_retained",
            "release_retained",
            "try_allocate",
            "assign_membership",
            "table_mut",
        ] {
            assert!(
                !freeze_tail.contains(forbidden),
                "eligible freeze seam regained fallible authority `{forbidden}`"
            );
        }
    }

    #[test]
    fn retained_handles_do_not_gain_unattributed_clone() {
        trait AmbiguousIfClone<Marker> {
            fn assert_not_clone() {}
        }

        impl<T: ?Sized> AmbiguousIfClone<()> for T {}
        impl<T: Clone> AmbiguousIfClone<u8> for T {}

        let _ = <PhysicalStorageAccount as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <RetainedAllocation<u64> as AmbiguousIfClone<_>>::assert_not_clone;
    }

    #[test]
    fn public_handles_are_send_and_sync_for_thread_safe_payloads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PhysicalStorageAccountFactory>();
        assert_send_sync::<PhysicalStorageAccount>();
        assert_send_sync::<RetainedAllocation<u64>>();
    }

    #[test]
    fn publication_evidence_precharges_the_capacity_envelope() {
        type EvidenceOperation =
            RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, u8, u32>;

        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let dimensions = (0_u8..17).map(|dimension| {
            (
                dimension,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            )
        });
        let limits = LedgerLimits::new(u64::MAX, dimensions).unwrap();
        let capacity = CapacityPlan::for_type::<PublicationDimensionCell<u8>>(0, 0, 17).unwrap();
        let envelope_bytes = capacity.envelope_delta_bytes().unwrap();
        let plan = RetainedOperationStartPlan::new(
            RetainedWriterPlan::new(TestPhase::Build, 0, 0, RetainedBatchLimits::new(2, 2)),
            RetainedHandleSharePlan::new(0, 0),
            empty_plan(),
            OutputCommitment::new(0, 1),
            OutputCommitment::new(0, 1),
        );
        let operation = EvidenceOperation::try_start(
            OperationControl::default(),
            limits,
            &account,
            plan,
            || Ok(7_u32),
        )
        .unwrap_or_else(|_| panic!("evidence operation must start"));
        assert_eq!(account.share_call_count(), 4);
        let evidence = operation
            .baseline
            .evidence
            .as_ref()
            .expect("evidence baseline remains present");
        let allocation = evidence.allocation();
        let outer_bytes = FinalOwnerAllocation::<
            RetainedFinalOwnerInner<
                RetainedPayload<PublicationEvidenceCells<u8>>,
                LeafRetainedPolicy,
            >,
        >::checked_layout_bytes()
        .unwrap();

        assert_eq!(capacity.envelope_capacity(), 32);
        assert_eq!(allocation.charged_bytes(), outer_bytes + envelope_bytes);
        assert_eq!(allocation.charged_allocations(), 2);
        drop(operation);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn startup_share_qualification_rejection_preserves_the_borrowed_host_account() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 16);
        let policy = DimensionPolicy::reservable_bytes(7, 7, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let plan = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        let builder_called = AtomicBool::new(false);
        let result =
            TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
                builder_called.store(true, Ordering::SeqCst);
                Ok(7_u32)
            });

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
            ))
        ));
        assert!(!builder_called.load(Ordering::SeqCst));
        assert_eq!(account.share_call_count(), 0);
        let retained = account
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(9_u32))
            .expect("borrowed host account remains usable after start rejection");
        drop(retained);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn block_handle_clone_is_charged_before_builder_or_account_share() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 16);
        let policy = DimensionPolicy::reservable_bytes(15, 15, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
        let plan = RetainedOperationStartPlan::new(
            writer_plan(2, 2),
            share_work(),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            OutputCommitment::new(TestDimension::Bytes, 1),
        );
        let operation =
            TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
                Ok(7_u32)
            })
            .unwrap_or_else(|_| panic!("exact pre-block handle budget must start"));
        let mut assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("exact pre-block handle budget must open tables"));
        let shares_before = account.share_call_count();
        let builder_called = AtomicBool::new(false);

        assert!(matches!(
            assembly.try_retain_canonical(empty_plan(), |emplacer| {
                builder_called.store(true, Ordering::SeqCst);
                Ok(emplacer.place(11, CanonicalRootTag))
            }),
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
            ))
        ));
        assert!(!builder_called.load(Ordering::SeqCst));
        assert_eq!(account.share_call_count(), shares_before);
        drop(handoff_for_test(
            assembly
                .prepare_minimal_fallback()
                .unwrap_or_else(|_| panic!("handle rejection must preserve minimal fallback")),
        ));
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn startup_fallback_builder_error_returns_original_cause_and_restores_baseline() {
        reset_startup_test_observations();
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let before = account.snapshot();
        let builder_called = AtomicBool::new(false);
        let (limits, plan) = startup_limits_and_plan();

        let result = TestOperation::try_start(
            OperationControl::default(),
            limits,
            &account,
            plan,
            || -> Result<u32, RetainedBuildFailureCode> {
                builder_called.store(true, Ordering::SeqCst);
                Err(BUILD_FAILED)
            },
        );

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
            )) if cause == BUILD_FAILED
        ));
        assert!(builder_called.load(Ordering::SeqCst));
        assert_eq!(take_start_evidence_build_calls(), 0);
        assert_eq!(take_startup_failure_ledger_snapshot(), Some((0, 0)));
        let after = account.snapshot();
        assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
        assert_eq!(
            after.live_retained_allocations(),
            before.live_retained_allocations()
        );
        assert_eq!(after.poison_event_count(), before.poison_event_count());
        assert_eq!(account.share_call_count(), 4);

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn startup_evidence_builder_error_drops_fallback_once_and_restores_baseline() {
        static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

        reset_startup_test_observations();
        FALLBACK_DROPS.store(0, Ordering::SeqCst);
        fail_next_start_evidence_build(BUILD_FAILED);
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let before = account.snapshot();
        let (limits, plan) = startup_limits_and_plan();
        type DropOperation = RetainedOperation<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            TestDimension,
            DropCounter,
        >;

        let result =
            DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
                Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
            });

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
            )) if cause == BUILD_FAILED
        ));
        assert_eq!(take_start_evidence_build_calls(), 1);
        assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(take_startup_failure_ledger_snapshot(), Some((0, 0)));
        let after = account.snapshot();
        assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
        assert_eq!(
            after.live_retained_allocations(),
            before.live_retained_allocations()
        );
        assert_eq!(after.poison_event_count(), before.poison_event_count());
        assert_eq!(account.share_call_count(), 4);

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn existing_poison_does_not_suppress_a_later_startup_ownership_fault() {
        reset_startup_test_observations();
        fail_next_start_evidence_build(BUILD_FAILED);
        lose_next_start_fallback_account_anchor();
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let (limits, plan) = startup_limits_and_plan();

        let result =
            TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
                let _ =
                    account.record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
                Ok::<u32, RetainedBuildFailureCode>(7)
            });

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::OwnershipUnverified
            ))
        ));
        let snapshot = account.snapshot();
        assert_eq!(snapshot.poison_event_count(), 2);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
        );

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn startup_evidence_builder_panic_releases_the_clean_fallback_sibling() {
        static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

        reset_startup_test_observations();
        FALLBACK_DROPS.store(0, Ordering::SeqCst);
        panic_next_start_evidence_build();
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let before = account.snapshot();
        let evidence_capacity =
            CapacityPlan::for_type::<PublicationDimensionCell<TestDimension>>(0, 0, 1)
                .unwrap_or_else(|_| panic!("evidence panic capacity must qualify"));
        let nested_bytes = evidence_capacity
            .envelope_delta_bytes()
            .unwrap_or_else(|_| panic!("evidence panic envelope must fit"));
        let evidence_plan = RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
            .unwrap_or_else(|_| panic!("evidence panic plan must qualify"));
        let evidence_charge = account
            .checked_retained_charge::<PublicationEvidenceCells<TestDimension>>(evidence_plan)
            .unwrap_or_else(|_| panic!("evidence panic charge must qualify"));
        let (limits, plan) = startup_limits_and_plan();
        type DropOperation = RetainedOperation<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            TestDimension,
            DropCounter,
        >;

        let result =
            DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
                Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
            });

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
            ))
        ));
        assert_eq!(take_start_evidence_build_calls(), 1);
        assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
        let snapshot = account.snapshot();
        assert_eq!(
            snapshot.live_retained_bytes() - before.live_retained_bytes(),
            evidence_charge.bytes
        );
        assert_eq!(
            snapshot.live_retained_allocations() - before.live_retained_allocations(),
            evidence_charge.allocations
        );
        assert_eq!(snapshot.poison_event_count(), 1);
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::RetainedBuilderPanicked)
        );
        let (active, live) = take_startup_failure_ledger_snapshot()
            .unwrap_or_else(|| panic!("evidence panic must capture startup ledger state"));
        assert_eq!(active, 2);
        assert!(live > evidence_charge.bytes);

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn startup_evidence_builder_panic_keeps_storage_release_as_r_and_records_builder_f() {
        static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

        reset_startup_test_observations();
        FALLBACK_DROPS.store(0, Ordering::SeqCst);
        panic_next_start_evidence_build();
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let (limits, plan) = startup_limits_and_plan();
        type PanicDropOperation = RetainedOperation<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            TestDimension,
            PanickingDropProbe,
        >;

        let result = PanicDropOperation::try_start(
            OperationControl::default(),
            limits,
            &account,
            plan,
            || {
                Ok::<PanickingDropProbe, RetainedBuildFailureCode>(PanickingDropProbe(
                    &FALLBACK_DROPS,
                ))
            },
        );

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::StorageDropUnverified
            ))
        ));
        assert_eq!(take_start_evidence_build_calls(), 1);
        assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
        let snapshot = account.snapshot();
        assert_eq!(snapshot.poison_event_count(), 2);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
        );
        let (active, live) = take_startup_failure_ledger_snapshot()
            .unwrap_or_else(|| panic!("combined evidence panic must capture startup ledger state"));
        assert_eq!(active, 4);
        assert!(live > 0);

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn startup_evidence_builder_panic_prefers_sibling_counter_f_without_builder_f() {
        static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

        reset_startup_test_observations();
        FALLBACK_DROPS.store(0, Ordering::SeqCst);
        panic_next_start_evidence_build();
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let (limits, plan) = startup_limits_and_plan();
        type DropOperation = RetainedOperation<
            TestOwner,
            TestPhase,
            TestProgress,
            TestUnit,
            TestDimension,
            DropCounter,
        >;

        let result =
            DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
                let mut state = lock_recovering_poison(&account.inner.value().linear);
                state.counters.live_bytes = 0;
                state.counters.live_allocations = 0;
                drop(state);
                Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
            });

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::ChargeReleaseUnverified
            ))
        ));
        assert_eq!(take_start_evidence_build_calls(), 1);
        assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
        let snapshot = account.snapshot();
        assert_eq!(snapshot.poison_event_count(), 1);
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::CounterInvariantViolated)
        );

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn startup_fallback_panic_contains_fallback_and_cleans_evidence() {
        reset_startup_test_observations();
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let fallback_charge = account
            .checked_retained_charge::<u32>(empty_plan())
            .unwrap_or_else(|_| panic!("fallback charge must qualify"));
        let (limits, plan) = startup_limits_and_plan();

        let result = TestOperation::try_start(
            OperationControl::default(),
            limits,
            &account,
            plan,
            || -> Result<u32, RetainedBuildFailureCode> { panic!("inject fallback startup panic") },
        );

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
            ))
        ));
        assert_eq!(take_start_evidence_build_calls(), 0);
        let snapshot = account.snapshot();
        assert!(snapshot.poisoned());
        assert_eq!(snapshot.poison_event_count(), 1);
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::RetainedBuilderPanicked)
        );
        assert_eq!(snapshot.live_retained_bytes(), fallback_charge.bytes);
        assert_eq!(
            snapshot.live_retained_allocations(),
            fallback_charge.allocations
        );
        let (active, live) = take_startup_failure_ledger_snapshot()
            .unwrap_or_else(|| panic!("fallback panic must capture startup ledger state"));
        assert_eq!(active, 2);
        assert!(live > fallback_charge.bytes);
        assert_eq!(account.share_call_count(), 4);

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn startup_fallback_panic_with_sibling_underflow_records_only_counter_fault() {
        reset_startup_test_observations();
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let (limits, plan) = startup_limits_and_plan();

        let result = TestOperation::try_start(
            OperationControl::default(),
            limits,
            &account,
            plan,
            || -> Result<u32, RetainedBuildFailureCode> {
                let mut state = lock_recovering_poison(&account.inner.value().linear);
                state.counters.live_bytes = 0;
                state.counters.live_allocations = 0;
                drop(state);
                panic!("inject fallback panic after counter corruption")
            },
        );

        assert!(matches!(
            result,
            Err(RetainedOperationStartError::Writer(
                RetainedWriterError::ChargeReleaseUnverified
            ))
        ));
        assert_eq!(take_start_evidence_build_calls(), 0);
        let snapshot = account.snapshot();
        assert!(snapshot.poisoned());
        assert_eq!(snapshot.poison_event_count(), 1);
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(AccountPoisonEvidence::reason),
            Some(AccountPoisonReason::CounterInvariantViolated)
        );
        assert_eq!(snapshot.live_retained_bytes(), 0);
        assert_eq!(snapshot.live_retained_allocations(), 0);
        let (active, live) = take_startup_failure_ledger_snapshot()
            .unwrap_or_else(|| panic!("underflow panic must capture startup ledger state"));
        assert_eq!(active, 4);
        assert!(live > 0);
        assert_eq!(account.share_call_count(), 4);

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn single_value_bootstrap_and_primary_prepare_exclude_duplicate_tail_work() {
        let production = retained_production_source();
        assert!(!production.contains("set: fn("));
        assert!(!production.contains("AggregateSetterPanicked"));
        assert!(!production.contains("fn try_create_single_retained_part"));
        assert_eq!(production.matches(".try_prepare_typed_control(").count(), 1);
        assert_eq!(production.matches("ledger.try_bind_primary(").count(), 1);
        assert_eq!(
            production.matches("ledger.finish_bound_primary(").count(),
            1
        );
    }
}
