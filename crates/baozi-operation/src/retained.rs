//! Fallible physical storage accounting for retained operation allocations.

use crate::control::discard_untrusted_panic_payload;
use crate::coordinator::{FinalProgress, OperationCoordinator};
use crate::ledger::{
    AttributionBatchCreateError, AttributionReservationBatch, CanonicalOutput,
    FallbackBatchBaseline, Ledger, LedgerCause, LedgerLimits, LedgerSnapshot, OutputCommitment,
    PublicationDestination, PublishedOutcomeEvidence,
};
use crate::{
    CapacityPlan, ControlCause, OperationControl, OperationOwner, ProgressPhase, ProgressUnit,
};
use std::alloc::Layout;
use std::borrow::Borrow;
use std::fmt;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use triomphe::{Arc, UniqueArc};

/// Maximum inline payload header moved out of the final shared allocation.
///
/// Input-proportional storage belongs in separately planned backing allocations and contributes to
/// [`RetainedAllocationPlan`]. This ceiling bounds the final-owner extraction stack move.
pub const MAX_RETAINED_INLINE_BYTES: usize = 4 * 1024;

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
    let Some(live_bytes) = counters.live_bytes.checked_sub(charge.bytes) else {
        return;
    };
    let Some(live_allocations) = counters.live_allocations.checked_sub(charge.allocations) else {
        return;
    };
    counters.live_bytes = live_bytes;
    counters.live_allocations = live_allocations;
}

trait AdmissionOwner {
    fn release_admission(&self, charge: AdmissionCharge);
}

struct AdmissionLease<O: AdmissionOwner> {
    owner: O,
    charge: AdmissionCharge,
}

impl<O: AdmissionOwner> AdmissionLease<O> {
    const fn new(owner: O, charge: AdmissionCharge) -> Self {
        Self { owner, charge }
    }
}

impl<O: AdmissionOwner> Drop for AdmissionLease<O> {
    fn drop(&mut self) {
        self.owner.release_admission(self.charge);
    }
}

struct FailClosedLease<L> {
    lease: Option<L>,
}

impl<L> FailClosedLease<L> {
    const fn new(lease: L) -> Self {
        Self { lease: Some(lease) }
    }

    fn get(&self) -> &L {
        match self.lease.as_ref() {
            Some(lease) => lease,
            None => unreachable_final_owner_state(),
        }
    }

    fn release_after_verified_storage_drop(&mut self) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(lease))) {
            discard_untrusted_panic_payload(payload);
        }
    }
}

impl<L> Drop for FailClosedLease<L> {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            std::mem::forget(lease);
        }
    }
}

struct FinalOwnerInner<T, L> {
    value: Option<T>,
    lease: FailClosedLease<L>,
    #[cfg(test)]
    share_calls: AtomicUsize,
}

impl<T, L> FinalOwnerInner<T, L> {
    const fn new(value: T, lease: L) -> Self {
        Self {
            value: Some(value),
            lease: FailClosedLease::new(lease),
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

    fn lease(&self) -> &L {
        self.lease.get()
    }
}

impl<T, L> Drop for FinalOwnerInner<T, L> {
    fn drop(&mut self) {
        let value = self.value.take();
        // Running untrusted destructors during an existing unwind could abort the process before
        // containment runs. Retain both storage and admission in that fail-closed state.
        if std::thread::panicking() {
            if let Some(value) = value {
                std::mem::forget(value);
            }
            return;
        }

        let Some(value) = value else {
            return;
        };

        match catch_unwind(AssertUnwindSafe(|| drop(value))) {
            Ok(()) => self.lease.release_after_verified_storage_drop(),
            Err(payload) => {
                // A panicking destructor cannot prove that nested storage was released. Preserve
                // the guard's default-retain lease so physical accounting remains fail-closed.
                discard_untrusted_panic_payload(payload);
            }
        }
    }
}

/// The only final-owner extraction backend for retained storage and account control.
struct FinalOwnerAllocation<T, L> {
    shared: Option<Arc<FinalOwnerInner<T, L>>>,
}

impl<T, L> FinalOwnerAllocation<T, L> {
    fn try_new(value: T, lease: L) -> Result<Self, FinalOwnerAllocationError> {
        let inner = FinalOwnerInner::new(value, lease);

        #[cfg(test)]
        if take_injected_final_owner_allocation_failure() {
            drop(inner);
            return Err(FinalOwnerAllocationError::AllocationFailed);
        }

        let shared =
            Arc::try_new(inner).map_err(|_| FinalOwnerAllocationError::AllocationFailed)?;
        Ok(Self {
            shared: Some(shared),
        })
    }

    fn checked_layout_bytes() -> Result<u64, FinalOwnerAllocationError> {
        let (layout, _) = Layout::new::<AtomicUsize>()
            .extend(Layout::new::<FinalOwnerInner<T, L>>())
            .map_err(|_| FinalOwnerAllocationError::AccountingOverflow)?;
        u64::try_from(layout.pad_to_align().size())
            .map_err(|_| FinalOwnerAllocationError::AccountingOverflow)
    }

    #[allow(
        dead_code,
        reason = "the coupled retained writer will be the production authority for shared handles"
    )]
    fn share(&self) -> Self {
        let shared = match self.shared.as_ref() {
            Some(shared) => {
                #[cfg(test)]
                shared.share_calls.fetch_add(1, Ordering::Relaxed);
                Arc::clone(shared)
            }
            None => unreachable_final_owner_state(),
        };
        Self {
            shared: Some(shared),
        }
    }

    fn value(&self) -> &T {
        match self.shared.as_ref() {
            Some(shared) => shared.value(),
            None => unreachable_final_owner_state(),
        }
    }

    fn lease(&self) -> &L {
        match self.shared.as_ref() {
            Some(shared) => shared.lease(),
            None => unreachable_final_owner_state(),
        }
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        match (self.shared.as_ref(), other.shared.as_ref()) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    #[cfg(test)]
    fn strong_count(&self) -> usize {
        self.shared.as_ref().map_or(0, Arc::strong_count)
    }

    #[cfg(test)]
    fn share_call_count(&self) -> usize {
        self.shared
            .as_ref()
            .map_or(0, |shared| shared.share_calls.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    fn allocation_identity(&self) -> *const FinalOwnerInner<T, L> {
        self.shared.as_ref().map_or(std::ptr::null(), Arc::as_ptr)
    }
}

impl<T, L> Drop for FinalOwnerAllocation<T, L> {
    fn drop(&mut self) {
        let Some(shared) = self.shared.take() else {
            return;
        };
        let Some(unique) = Arc::into_unique(shared) else {
            return;
        };

        // The qualified triomphe implementation deallocates the outer shared block before this
        // value is returned. FinalOwnerInner is the sole contained payload-then-lease boundary.
        drop(UniqueArc::into_inner(unique));
    }
}

fn unreachable_final_owner_state() -> ! {
    panic!("final-owner allocation was observed during destruction")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalOwnerAllocationError {
    AccountingOverflow,
    AllocationFailed,
}

/// Invalid explicit limits for a physical storage account or account factory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhysicalStorageLimitError {
    ZeroByteLimit,
    ZeroAllocationLimit,
}

impl fmt::Display for PhysicalStorageLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroByteLimit => {
                formatter.write_str("physical storage byte limit must be nonzero")
            }
            Self::ZeroAllocationLimit => {
                formatter.write_str("physical storage allocation limit must be nonzero")
            }
        }
    }
}

impl std::error::Error for PhysicalStorageLimitError {}

/// Explicit bounds for account-control storage owned by one factory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalStorageAccountFactoryLimits {
    max_account_control_bytes: u64,
    max_live_accounts: u64,
}

impl PhysicalStorageAccountFactoryLimits {
    pub fn new(
        max_account_control_bytes: u64,
        max_live_accounts: u64,
    ) -> Result<Self, PhysicalStorageLimitError> {
        validate_nonzero_limits(max_account_control_bytes, max_live_accounts)?;
        Ok(Self {
            max_account_control_bytes,
            max_live_accounts,
        })
    }

    pub const fn max_account_control_bytes(self) -> u64 {
        self.max_account_control_bytes
    }

    pub const fn max_live_accounts(self) -> u64 {
        self.max_live_accounts
    }

    const fn counter_limits(self) -> CounterLimits {
        CounterLimits {
            bytes: self.max_account_control_bytes,
            allocations: self.max_live_accounts,
        }
    }
}

/// Explicit retained-storage bounds for one physical account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalStorageAccountLimits {
    max_retained_bytes: u64,
    max_retained_allocations: u64,
}

impl PhysicalStorageAccountLimits {
    pub fn new(
        max_retained_bytes: u64,
        max_retained_allocations: u64,
    ) -> Result<Self, PhysicalStorageLimitError> {
        validate_nonzero_limits(max_retained_bytes, max_retained_allocations)?;
        Ok(Self {
            max_retained_bytes,
            max_retained_allocations,
        })
    }

    pub const fn max_retained_bytes(self) -> u64 {
        self.max_retained_bytes
    }

    pub const fn max_retained_allocations(self) -> u64 {
        self.max_retained_allocations
    }

    const fn counter_limits(self) -> CounterLimits {
        CounterLimits {
            bytes: self.max_retained_bytes,
            allocations: self.max_retained_allocations,
        }
    }
}

fn validate_nonzero_limits(bytes: u64, allocations: u64) -> Result<(), PhysicalStorageLimitError> {
    if bytes == 0 {
        return Err(PhysicalStorageLimitError::ZeroByteLimit);
    }
    if allocations == 0 {
        return Err(PhysicalStorageLimitError::ZeroAllocationLimit);
    }
    Ok(())
}

/// A factory's current account-control accounting evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalStorageAccountFactorySnapshot {
    live_account_control_bytes: u64,
    live_accounts: u64,
    peak_account_control_bytes: u64,
    peak_live_accounts: u64,
    total_admitted_account_control_bytes: u64,
    total_admitted_accounts: u64,
}

impl PhysicalStorageAccountFactorySnapshot {
    pub const fn live_account_control_bytes(self) -> u64 {
        self.live_account_control_bytes
    }

    pub const fn live_accounts(self) -> u64 {
        self.live_accounts
    }

    pub const fn peak_account_control_bytes(self) -> u64 {
        self.peak_account_control_bytes
    }

    pub const fn peak_live_accounts(self) -> u64 {
        self.peak_live_accounts
    }

    pub const fn total_admitted_account_control_bytes(self) -> u64 {
        self.total_admitted_account_control_bytes
    }

    pub const fn total_admitted_accounts(self) -> u64 {
        self.total_admitted_accounts
    }
}

/// A physical account's current retained-allocation accounting evidence.
///
/// Byte values are admitted charged envelopes covering declared shared and nested backing blocks,
/// not allocator RSS, size classes, fragmentation, or page-retention measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalStorageAccountSnapshot {
    live_retained_bytes: u64,
    live_retained_allocations: u64,
    peak_retained_bytes: u64,
    peak_retained_allocations: u64,
    total_admitted_retained_bytes: u64,
    total_admitted_retained_allocations: u64,
}

impl PhysicalStorageAccountSnapshot {
    pub const fn live_retained_bytes(self) -> u64 {
        self.live_retained_bytes
    }

    pub const fn live_retained_allocations(self) -> u64 {
        self.live_retained_allocations
    }

    pub const fn peak_retained_bytes(self) -> u64 {
        self.peak_retained_bytes
    }

    pub const fn peak_retained_allocations(self) -> u64 {
        self.peak_retained_allocations
    }

    pub const fn total_admitted_retained_bytes(self) -> u64 {
        self.total_admitted_retained_bytes
    }

    pub const fn total_admitted_retained_allocations(self) -> u64 {
        self.total_admitted_retained_allocations
    }
}

struct FactoryState {
    limits: PhysicalStorageAccountFactoryLimits,
    counters: Mutex<CounterState>,
}

impl FactoryState {
    fn snapshot(&self) -> PhysicalStorageAccountFactorySnapshot {
        let counters = *lock_recovering_poison(&self.counters);
        PhysicalStorageAccountFactorySnapshot {
            live_account_control_bytes: counters.live_bytes,
            live_accounts: counters.live_allocations,
            peak_account_control_bytes: counters.peak_bytes,
            peak_live_accounts: counters.peak_allocations,
            total_admitted_account_control_bytes: counters.total_admitted_bytes,
            total_admitted_accounts: counters.total_admitted_allocations,
        }
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
        type FactoryControlLease = AdmissionLease<Arc<FactoryState>>;
        let bytes =
            FinalOwnerAllocation::<AccountState, FactoryControlLease>::checked_layout_bytes()
                .map_err(map_account_control_final_owner_error)?;
        let charge = AdmissionCharge::new(bytes, 1);
        let owner = Arc::clone(&self.state);
        try_admit(&owner.counters, owner.limits.counter_limits(), charge)
            .map_err(map_factory_admission_error)?;
        let lease = AdmissionLease::new(owner, charge);
        let state = AccountState {
            limits,
            counters: Mutex::new(CounterState::default()),
        };
        let inner = FinalOwnerAllocation::try_new(state, lease)
            .map_err(map_account_control_final_owner_error)?;
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
    counters: Mutex<CounterState>,
}

/// One opaque physical owner for retained allocations across operation and Scene generations.
///
/// Account identity is observable only through same-account predicates. It is not a semantic Scene
/// value and is intentionally absent from equality, ordering, serialization, and digest APIs.
pub struct PhysicalStorageAccount {
    inner: FinalOwnerAllocation<AccountState, AdmissionLease<Arc<FactoryState>>>,
}

impl PhysicalStorageAccount {
    pub fn limits(&self) -> PhysicalStorageAccountLimits {
        self.inner.value().limits
    }

    pub fn snapshot(&self) -> PhysicalStorageAccountSnapshot {
        let counters = *lock_recovering_poison(&self.inner.value().counters);
        PhysicalStorageAccountSnapshot {
            live_retained_bytes: counters.live_bytes,
            live_retained_allocations: counters.live_allocations,
            peak_retained_bytes: counters.peak_bytes,
            peak_retained_allocations: counters.peak_allocations,
            total_admitted_retained_bytes: counters.total_admitted_bytes,
            total_admitted_retained_allocations: counters.total_admitted_allocations,
        }
    }

    pub fn is_same_account(&self, other: &Self) -> bool {
        self.inner.ptr_eq_for_account(&other.inner)
    }

    /// Shares the opaque account handle inside the retained owner implementation.
    ///
    /// Domain callers must eventually reach this only through the coupled operation-attribution
    /// writer; the raw physical handle share is deliberately module-private.
    #[allow(
        dead_code,
        reason = "the coupled retained writer will gate this physical handle share"
    )]
    fn share_after_admission(&self) -> Self {
        Self {
            inner: self.inner.share(),
        }
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
    fn try_retain_with<T>(
        &self,
        plan: RetainedAllocationPlan,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<RetainedAllocation<T>, RetainedAllocationError>
    where
        T: Send + Sync + 'static,
    {
        let charge = self.checked_retained_charge::<T>(plan)?;
        let owner = self.share_after_admission();
        try_admit(
            &owner.inner.value().counters,
            owner.limits().counter_limits(),
            charge,
        )
        .map_err(map_retained_admission_error)?;
        let lease = AdmissionLease::new(owner, charge);

        let value = match catch_unwind(AssertUnwindSafe(build)) {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                drop(lease);
                return Err(RetainedAllocationError::Build(error));
            }
            Err(payload) => {
                let charge_retained = !payload.is::<String>() && !payload.is::<&'static str>();
                discard_untrusted_panic_payload(payload);
                #[cfg(test)]
                if !charge_retained {
                    observe_known_builder_panic_after_discard(&lease.owner);
                }
                if charge_retained {
                    std::mem::forget(lease);
                } else {
                    drop(lease);
                }
                return Err(RetainedAllocationError::BuilderPanicked { charge_retained });
            }
        };
        let payload = RetainedPayload { value, charge };
        let inner = FinalOwnerAllocation::try_new(payload, lease)
            .map_err(map_retained_final_owner_error)?;
        Ok(RetainedAllocation { inner })
    }

    #[allow(
        dead_code,
        reason = "the coupled retained writer will preflight this charge before both admissions"
    )]
    fn checked_retained_charge<T>(
        &self,
        plan: RetainedAllocationPlan,
    ) -> Result<AdmissionCharge, RetainedAllocationError> {
        let inline_bytes = size_of::<RetainedPayload<T>>();
        if inline_bytes > MAX_RETAINED_INLINE_BYTES {
            return Err(RetainedAllocationError::InlinePayloadTooLarge {
                bytes: inline_bytes,
                limit: MAX_RETAINED_INLINE_BYTES,
            });
        }

        type RetainedLease = AdmissionLease<PhysicalStorageAccount>;
        let outer_bytes =
            FinalOwnerAllocation::<RetainedPayload<T>, RetainedLease>::checked_layout_bytes()
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

impl AdmissionOwner for PhysicalStorageAccount {
    fn release_admission(&self, charge: AdmissionCharge) {
        release(&self.inner.value().counters, charge);
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

impl<T, L> FinalOwnerAllocation<T, L> {
    fn ptr_eq_for_account(&self, other: &Self) -> bool {
        match (self.shared.as_ref(), other.shared.as_ref()) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

/// Planned nested allocation envelope for one retained ownership header.
///
/// The complete admitted envelope remains charged for this leaf primitive's lifetime. A canonical
/// guard must derive it from qualified capacity plans rather than treating caller estimates as
/// observed allocator evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedAllocationPlan {
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

    pub const fn nested_bytes(self) -> u64 {
        self.nested_bytes
    }

    pub const fn nested_allocations(self) -> u64 {
        self.nested_allocations
    }
}

/// Invalid nested retained-allocation evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetainedAllocationPlanError {
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
/// Builder-owned storage is destroyed while physical admission remains live. This concrete value
/// type prevents safe Rust from moving an owning `Vec`, `String`, `Arc`, static reference, or
/// interior-mutable storage container past the point where that admission is released. Domain
/// layers translate their typed failure into a local numeric code inside the builder closure and
/// reconstruct the domain error after this operation seam returns.
///
/// ```compile_fail
/// use baozi_operation::RetainedBuildFailureCode;
/// use std::sync::Mutex;
///
/// let storage: &'static Mutex<Option<Vec<u8>>> =
///     Box::leak(Box::new(Mutex::new(Some(vec![0; 64]))));
/// let code: RetainedBuildFailureCode = storage;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct RetainedBuildFailureCode(u32);

impl RetainedBuildFailureCode {
    pub const fn new(code: u32) -> Self {
        Self(code)
    }

    pub const fn get(self) -> u32 {
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

/// One immutable payload whose physical charge follows its final shared owner.
///
/// This type deliberately has no public `Clone`, detach, rebrand, or manual-release operation.
///
/// ```compile_fail
/// use baozi_operation::RetainedAllocation;
///
/// fn clone_retained(allocation: RetainedAllocation<u64>) {
///     let _duplicate = allocation.clone();
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::RetainedAllocation;
///
/// fn detach_retained(allocation: RetainedAllocation<u64>) {
///     let _value = allocation.detach();
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::RetainedAllocation;
///
/// let _forged: RetainedAllocation<u64> = RetainedAllocation { inner: todo!() };
/// ```
pub struct RetainedAllocation<T> {
    inner: FinalOwnerAllocation<RetainedPayload<T>, AdmissionLease<PhysicalStorageAccount>>,
}

impl<T> RetainedAllocation<T> {
    pub fn charged_bytes(&self) -> u64 {
        self.inner.value().charge.bytes
    }

    pub fn charged_allocations(&self) -> u64 {
        self.inner.value().charge.allocations
    }

    pub fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        self.inner.lease().owner.is_same_account(account)
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    fn share_after_operation_admission(&self) -> Self {
        Self {
            inner: self.inner.share(),
        }
    }

    #[cfg(test)]
    fn share_after_test_admission(&self) -> Self {
        self.share_after_operation_admission()
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        self.inner.ptr_eq(&other.inner)
    }

    #[cfg(test)]
    fn strong_count(&self) -> usize {
        self.inner.strong_count()
    }

    #[cfg(test)]
    fn share_call_count(&self) -> usize {
        self.inner.share_call_count()
    }

    #[cfg(test)]
    fn allocation_identity(
        &self,
    ) -> *const FinalOwnerInner<RetainedPayload<T>, AdmissionLease<PhysicalStorageAccount>> {
        self.inner.allocation_identity()
    }
}

impl<T> AsRef<T> for RetainedAllocation<T> {
    fn as_ref(&self) -> &T {
        &self.inner.value().value
    }
}

impl<T> Borrow<T> for RetainedAllocation<T> {
    fn borrow(&self) -> &T {
        self.as_ref()
    }
}

impl<T> fmt::Debug for RetainedAllocation<T> {
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
pub enum RetainedAllocationError {
    InlinePayloadTooLarge { bytes: usize, limit: usize },
    AccountingOverflow,
    ByteLimitExceeded { requested: u64, limit: u64 },
    AllocationLimitExceeded { requested: u64, limit: u64 },
    Build(RetainedBuildFailureCode),
    BuilderPanicked { charge_retained: bool },
    AllocationFailed,
}

impl fmt::Display for RetainedAllocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InlinePayloadTooLarge { bytes, limit } => write!(
                formatter,
                "retained inline payload uses {bytes} bytes, exceeding limit {limit}"
            ),
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
            Self::BuilderPanicked { charge_retained } => {
                if *charge_retained {
                    formatter.write_str(
                        "retained payload constructor panicked with an opaque leaked payload; physical charge remains retained",
                    )
                } else {
                    formatter.write_str("retained payload constructor panicked")
                }
            }
            Self::AllocationFailed => formatter.write_str("retained shared allocation failed"),
        }
    }
}

impl std::error::Error for RetainedAllocationError {}

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
pub struct RetainedBatchLimits {
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
pub struct RetainedWriterPlan<P, D> {
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

/// Clone, retained-handle, and branch work charged before sharing an inherited allocation.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedSharePlan<D> {
    clone_dimension: D,
    retained_handle_dimension: D,
    branch_dimension: D,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<D: Copy> RetainedSharePlan<D> {
    pub const fn new(
        clone_dimension: D,
        retained_handle_dimension: D,
        branch_dimension: D,
    ) -> Self {
        Self {
            clone_dimension,
            retained_handle_dimension,
            branch_dimension,
        }
    }

    const fn charges(self) -> [(D, u64); 3] {
        [
            (self.clone_dimension, 1),
            (self.retained_handle_dimension, 1),
            (self.branch_dimension, 1),
        ]
    }
}

/// Start-time bounds and fixed terminal commitments for one retained operation.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedOperationStartPlan<P, D> {
    terminal_writer: RetainedWriterPlan<P, D>,
    terminal_account_share_work: RetainedSharePlan<D>,
    fallback_plan: RetainedAllocationPlan,
    fallback_commitment: OutputCommitment<D, PublishedOutcomeEvidence>,
    snapshot_commitment: OutputCommitment<D, PublishedOutcomeEvidence>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<P: Copy, D: Copy> RetainedOperationStartPlan<P, D> {
    pub const fn new(
        terminal_writer: RetainedWriterPlan<P, D>,
        terminal_account_share_work: RetainedSharePlan<D>,
        fallback_plan: RetainedAllocationPlan,
        fallback_commitment: OutputCommitment<D, PublishedOutcomeEvidence>,
        snapshot_commitment: OutputCommitment<D, PublishedOutcomeEvidence>,
    ) -> Self {
        Self {
            terminal_writer,
            terminal_account_share_work,
            fallback_plan,
            fallback_commitment,
            snapshot_commitment,
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug)]
#[non_exhaustive]
pub enum RetainedWriterError<D> {
    BatchCapacity,
    BatchAllocationFailed,
    Ledger(LedgerCause<D>),
    Physical(RetainedAllocationError),
    AttributionBatchFull,
    CommitmentBatchFull,
    ForeignAccount,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug)]
#[non_exhaustive]
pub enum RetainedOperationStartError<D> {
    InvalidTerminalLimits,
    Writer(RetainedWriterError<D>),
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

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn try_share_account_after_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    work: RetainedSharePlan<D>,
) -> Result<PhysicalStorageAccount, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    ledger
        .charge_observed_batch(&work.charges())
        .map_err(RetainedWriterError::Ledger)?;
    Ok(account.share_after_admission())
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
pub struct PublishedDimensionSnapshot<D> {
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

mod product_sealed {
    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub trait Sealed {}
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) trait RetainedProduct: product_sealed::Sealed + Send + Sync + 'static {
    fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool;
    fn attributed_leaf_count(&self) -> Option<usize>;
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) trait NonEmptyRetainedProduct: RetainedProduct {}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct RetainedUnit;

impl product_sealed::Sealed for RetainedUnit {}

impl RetainedProduct for RetainedUnit {
    fn belongs_to(&self, _account: &PhysicalStorageAccount) -> bool {
        true
    }

    fn attributed_leaf_count(&self) -> Option<usize> {
        Some(0)
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct RetainedLeaf<T> {
    allocation: RetainedAllocation<T>,
    attributed: bool,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<T> RetainedLeaf<T> {
    fn newly_allocated(allocation: RetainedAllocation<T>) -> Self {
        Self {
            allocation,
            attributed: true,
        }
    }

    fn inherited(allocation: RetainedAllocation<T>) -> Self {
        Self {
            allocation,
            attributed: false,
        }
    }

    pub(crate) fn allocation(&self) -> &RetainedAllocation<T> {
        &self.allocation
    }
}

impl<T: Send + Sync + 'static> product_sealed::Sealed for RetainedLeaf<T> {}

impl<T: Send + Sync + 'static> RetainedProduct for RetainedLeaf<T> {
    fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        self.allocation.belongs_to(account)
    }

    fn attributed_leaf_count(&self) -> Option<usize> {
        Some(usize::from(self.attributed))
    }
}

impl<T: Send + Sync + 'static> NonEmptyRetainedProduct for RetainedLeaf<T> {}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct RetainedPair<L, R> {
    left: L,
    right: R,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<L, R> RetainedPair<L, R> {
    fn new(left: L, right: R) -> Self {
        Self { left, right }
    }

    pub(crate) fn left(&self) -> &L {
        &self.left
    }

    pub(crate) fn right(&self) -> &R {
        &self.right
    }
}

impl<L: RetainedProduct, R: RetainedProduct> product_sealed::Sealed for RetainedPair<L, R> {}

impl<L: RetainedProduct, R: RetainedProduct> RetainedProduct for RetainedPair<L, R> {
    fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        self.left.belongs_to(account) && self.right.belongs_to(account)
    }

    fn attributed_leaf_count(&self) -> Option<usize> {
        self.left
            .attributed_leaf_count()?
            .checked_add(self.right.attributed_leaf_count()?)
    }
}

impl<L: RetainedProduct, R: NonEmptyRetainedProduct> NonEmptyRetainedProduct
    for RetainedPair<L, R>
{
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
type SingleRetained<T> = RetainedPair<RetainedUnit, RetainedLeaf<T>>;

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
struct RetainedWriterCore<O, P, D, Destination, Bundle>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
    Bundle: RetainedProduct,
{
    bundle: Option<Bundle>,
    batch: Option<AttributionReservationBatch<O, P, D, Destination>>,
    account: Option<PhysicalStorageAccount>,
    phase: P,
    attribution_dimension: D,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
struct RetainedCoreTransitionError<Core, D> {
    core: Core,
    cause: RetainedWriterError<D>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<Core, D> RetainedCoreTransitionError<Core, D> {
    fn into_parts(self) -> (Core, RetainedWriterError<D>) {
        (self.core, self.cause)
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D, Destination> RetainedWriterCore<O, P, D, Destination, RetainedUnit>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Destination: PublicationDestination,
{
    fn try_new(
        ledger: &Ledger<O, P, D>,
        account: PhysicalStorageAccount,
        plan: RetainedWriterPlan<P, D>,
    ) -> Result<Self, RetainedWriterError<D>> {
        let batch = ledger
            .try_attribution_batch(
                plan.phase,
                plan.capacity_dimension,
                plan.limits.attribution_slots,
                plan.limits.commitment_slots,
            )
            .map_err(map_batch_error)?;
        Ok(Self {
            bundle: Some(RetainedUnit),
            batch: Some(batch),
            account: Some(account),
            phase: plan.phase,
            attribution_dimension: plan.attribution_dimension,
        })
    }
}

#[allow(
    dead_code,
    clippy::result_large_err,
    clippy::type_complexity,
    reason = "linear transitions preserve the prior writer on rejection without allocating"
)]
impl<O, P, D, Destination, Bundle> RetainedWriterCore<O, P, D, Destination, Bundle>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
    Destination: PublicationDestination,
    Bundle: RetainedProduct,
{
    fn try_retain<T>(
        mut self,
        ledger: &Ledger<O, P, D>,
        plan: RetainedAllocationPlan,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<
        RetainedWriterCore<O, P, D, Destination, RetainedPair<Bundle, RetainedLeaf<T>>>,
        RetainedCoreTransitionError<Self, D>,
    >
    where
        T: Send + Sync + 'static,
    {
        if !self
            .batch
            .as_ref()
            .is_some_and(AttributionReservationBatch::has_attribution_slot)
        {
            return Err(RetainedCoreTransitionError {
                core: self,
                cause: RetainedWriterError::AttributionBatchFull,
            });
        }
        let Some(account) = self.account.as_ref() else {
            return Err(RetainedCoreTransitionError {
                core: self,
                cause: RetainedWriterError::AttributionBatchFull,
            });
        };
        let charge = match account.checked_retained_charge::<T>(plan) {
            Ok(charge) => charge,
            Err(cause) => {
                return Err(RetainedCoreTransitionError {
                    core: self,
                    cause: RetainedWriterError::Physical(cause),
                });
            }
        };
        let attribution = match ledger.reserve_operation_attribution(
            self.phase,
            self.attribution_dimension,
            charge.bytes,
        ) {
            Ok(attribution) => attribution,
            Err(cause) => {
                return Err(RetainedCoreTransitionError {
                    core: self,
                    cause: RetainedWriterError::Ledger(cause),
                });
            }
        };
        let allocation = match account.try_retain_with(plan, build) {
            Ok(allocation) => allocation,
            Err(cause) => {
                drop(attribution);
                return Err(RetainedCoreTransitionError {
                    core: self,
                    cause: RetainedWriterError::Physical(cause),
                });
            }
        };
        let Some(batch) = self.batch.as_mut() else {
            drop(allocation);
            drop(attribution);
            return Err(RetainedCoreTransitionError {
                core: self,
                cause: RetainedWriterError::AttributionBatchFull,
            });
        };
        if let Err(attribution) = batch.try_push_attribution(attribution) {
            drop(allocation);
            drop(attribution);
            return Err(RetainedCoreTransitionError {
                core: self,
                cause: RetainedWriterError::AttributionBatchFull,
            });
        }
        let bundle = match self.bundle.take() {
            Some(bundle) => bundle,
            None => unreachable_final_owner_state(),
        };
        Ok(RetainedWriterCore {
            bundle: Some(RetainedPair::new(
                bundle,
                RetainedLeaf::newly_allocated(allocation),
            )),
            batch: self.batch.take(),
            account: self.account.take(),
            phase: self.phase,
            attribution_dimension: self.attribution_dimension,
        })
    }

    fn try_share<T>(
        mut self,
        ledger: &Ledger<O, P, D>,
        source: &RetainedAllocation<T>,
        work: RetainedSharePlan<D>,
    ) -> Result<
        RetainedWriterCore<O, P, D, Destination, RetainedPair<Bundle, RetainedLeaf<T>>>,
        RetainedCoreTransitionError<Self, D>,
    >
    where
        T: Send + Sync + 'static,
    {
        let Some(account) = self.account.as_ref() else {
            return Err(RetainedCoreTransitionError {
                core: self,
                cause: RetainedWriterError::ForeignAccount,
            });
        };
        if !source.belongs_to(account) {
            return Err(RetainedCoreTransitionError {
                core: self,
                cause: RetainedWriterError::ForeignAccount,
            });
        }
        if let Err(cause) = ledger.charge_observed_batch(&work.charges()) {
            return Err(RetainedCoreTransitionError {
                core: self,
                cause: RetainedWriterError::Ledger(cause),
            });
        }
        let allocation = source.share_after_operation_admission();
        let bundle = match self.bundle.take() {
            Some(bundle) => bundle,
            None => unreachable_final_owner_state(),
        };
        Ok(RetainedWriterCore {
            bundle: Some(RetainedPair::new(
                bundle,
                RetainedLeaf::inherited(allocation),
            )),
            batch: self.batch.take(),
            account: self.account.take(),
            phase: self.phase,
            attribution_dimension: self.attribution_dimension,
        })
    }

    fn try_commit(
        &mut self,
        commitment: OutputCommitment<D, Destination>,
    ) -> Result<(), RetainedWriterError<D>> {
        self.batch
            .as_mut()
            .ok_or(RetainedWriterError::CommitmentBatchFull)?
            .try_push_commitment(commitment)
            .map_err(|_| RetainedWriterError::CommitmentBatchFull)
    }

    fn finish(mut self) -> RetainedPart<Bundle, Destination, O, P, D>
    where
        Bundle: NonEmptyRetainedProduct,
    {
        RetainedPart {
            aggregate: self.bundle.take(),
            batch: self.batch.take(),
            account: self.account.take(),
        }
    }
}

impl<O, P, D, Destination, Bundle> Drop for RetainedWriterCore<O, P, D, Destination, Bundle>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
    Bundle: RetainedProduct,
{
    fn drop(&mut self) {
        drop(self.bundle.take());
        drop(self.batch.take());
        drop(self.account.take());
    }
}

/// A destination-qualified retained aggregate whose attribution cannot be detached.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub struct RetainedPart<Aggregate, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    aggregate: Option<Aggregate>,
    batch: Option<AttributionReservationBatch<O, P, D, Destination>>,
    account: Option<PhysicalStorageAccount>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<Aggregate, Destination, O, P, D> RetainedPart<Aggregate, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    pub fn aggregate(&self) -> &Aggregate {
        match self.aggregate.as_ref() {
            Some(aggregate) => aggregate,
            None => unreachable_final_owner_state(),
        }
    }

    fn account(&self) -> &PhysicalStorageAccount {
        match self.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }

    fn take_batch_for_prepare(&mut self) -> AttributionReservationBatch<O, P, D, Destination> {
        match self.batch.take() {
            Some(batch) => batch,
            None => unreachable_final_owner_state(),
        }
    }

    fn into_published_part(mut self) -> PublishedPart<Aggregate> {
        PublishedPart {
            aggregate: self.aggregate.take(),
            account: self.account.take(),
        }
    }

    fn discard_aggregate_before_batch_release(&mut self) -> PhysicalStorageAccount {
        drop(self.aggregate.take());
        match self.account.take() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<Aggregate, Destination, O, P, D> RetainedPart<Aggregate, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
    Aggregate: RetainedProduct,
{
    fn has_exact_product_graph(&self) -> bool {
        let (Some(aggregate), Some(batch), Some(account)) = (
            self.aggregate.as_ref(),
            self.batch.as_ref(),
            self.account.as_ref(),
        ) else {
            return false;
        };
        aggregate.belongs_to(account)
            && aggregate.attributed_leaf_count() == Some(batch.attribution_len())
    }
}

impl<Aggregate, Destination, O, P, D> Drop for RetainedPart<Aggregate, Destination, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    fn drop(&mut self) {
        drop(self.aggregate.take());
        drop(self.batch.take());
        drop(self.account.take());
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
struct PublishedPart<Aggregate> {
    aggregate: Option<Aggregate>,
    account: Option<PhysicalStorageAccount>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<Aggregate> PublishedPart<Aggregate> {
    fn aggregate(&self) -> &Aggregate {
        match self.aggregate.as_ref() {
            Some(aggregate) => aggregate,
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

impl<Aggregate> Drop for PublishedPart<Aggregate> {
    fn drop(&mut self) {
        drop(self.aggregate.take());
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
    fallback: Option<RetainedPart<SingleRetained<F>, PublishedOutcomeEvidence, O, P, D>>,
    evidence: Option<
        RetainedPart<
            SingleRetained<PublicationEvidenceCells<D>>,
            PublishedOutcomeEvidence,
            O,
            P,
            D,
        >,
    >,
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
        RetainedPart<SingleRetained<F>, PublishedOutcomeEvidence, O, P, D>,
        RetainedPart<
            SingleRetained<PublicationEvidenceCells<D>>,
            PublishedOutcomeEvidence,
            O,
            P,
            D,
        >,
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
}

impl<O, P, D, F> Drop for FallbackPublicationBaseline<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        let fallback_account = self.fallback.as_mut().and_then(|part| {
            drop(part.aggregate.take());
            part.account.take()
        });
        let evidence_account = self.evidence.as_mut().and_then(|part| {
            drop(part.aggregate.take());
            part.account.take()
        });
        drop(self.batches.take());
        drop(fallback_account);
        drop(evidence_account);
        drop(self.fallback.take());
        drop(self.evidence.take());
    }
}

/// Linear owner for one coordinator, one physical account, and its pre-admitted fallback.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub struct RetainedOperation<O, P, PP, U, D, F>
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
    terminal_account_share_work: RetainedSharePlan<D>,
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

        let coordinator = OperationCoordinator::new(control, ledger_limits);
        let ledger = coordinator.retained_ledger();
        let terminal_slot_plan = RetainedWriterPlan::new(
            plan.terminal_writer.phase,
            plan.terminal_writer.capacity_dimension,
            plan.terminal_writer.attribution_dimension,
            RetainedBatchLimits::new(1, 1),
        );
        let fallback_account =
            try_share_account_after_work(ledger, account, plan.terminal_account_share_work)?;
        let evidence_account =
            try_share_account_after_work(ledger, account, plan.terminal_account_share_work)?;
        let fallback = RetainedWriterCore::try_new(ledger, fallback_account, terminal_slot_plan)?;
        let mut fallback = match fallback.try_retain(ledger, plan.fallback_plan, build_fallback) {
            Ok(fallback) => fallback,
            Err(error) => {
                let (fallback, cause) = error.into_parts();
                drop(fallback);
                return Err(RetainedOperationStartError::Writer(cause));
            }
        };
        fallback.try_commit(plan.fallback_commitment)?;
        let mut fallback = fallback.finish();

        let dimension_count = ledger.dimension_count();
        let evidence_capacity =
            CapacityPlan::for_type::<PublicationDimensionCell<D>>(0, 0, dimension_count).map_err(
                |_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity),
            )?;
        let nested_bytes = evidence_capacity
            .envelope_delta_bytes()
            .map_err(|_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity))?;
        let evidence_plan = RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
            .map_err(|_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity))?;
        let evidence = RetainedWriterCore::try_new(ledger, evidence_account, terminal_slot_plan)?;
        let mut evidence = match evidence.try_retain(ledger, evidence_plan, || {
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
        }) {
            Ok(evidence) => evidence,
            Err(error) => {
                let (evidence, cause) = error.into_parts();
                drop(evidence);
                return Err(RetainedOperationStartError::Writer(cause));
            }
        };
        evidence.try_commit(plan.snapshot_commitment)?;
        let mut evidence = evidence.finish();
        ledger
            .validate_typed_batch_pair(
                fallback
                    .batch
                    .as_ref()
                    .ok_or(RetainedOperationStartError::InvalidTerminalLimits)?,
                evidence
                    .batch
                    .as_ref()
                    .ok_or(RetainedOperationStartError::InvalidTerminalLimits)?,
            )
            .map_err(|cause| {
                RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
            })?;

        let canonical_assembly = ledger
            .try_attribution_batch(
                plan.terminal_writer.phase,
                plan.terminal_writer.capacity_dimension,
                canonical_assembly.attribution_slots,
                canonical_assembly.commitment_slots,
            )
            .map_err(map_batch_error)?;
        let evidence_assembly = ledger
            .try_attribution_batch(
                plan.terminal_writer.phase,
                plan.terminal_writer.capacity_dimension,
                evidence_assembly.attribution_slots,
                evidence_assembly.commitment_slots,
            )
            .map_err(map_batch_error)?;

        if !fallback.has_exact_product_graph()
            || !evidence.has_exact_product_graph()
            || !fallback.account().is_same_account(evidence.account())
        {
            return Err(RetainedOperationStartError::Writer(
                RetainedWriterError::ForeignAccount,
            ));
        }
        let fallback_batch = fallback.take_batch_for_prepare();
        let evidence_batch = evidence.take_batch_for_prepare();
        let batches = match ledger.try_fallback_baseline(
            canonical_assembly,
            evidence_assembly,
            fallback_batch,
            evidence_batch,
        ) {
            Ok(baseline) => baseline,
            Err(rejection) => {
                let (baseline, cause) = rejection.into_parts();
                let (canonical_assembly, evidence_assembly, fallback_batch, evidence_batch) =
                    baseline.into_parts();
                let fallback_account = fallback.discard_aggregate_before_batch_release();
                let evidence_account = evidence.discard_aggregate_before_batch_release();
                drop(fallback);
                drop(evidence);
                drop(fallback_batch);
                drop(evidence_batch);
                drop(fallback_account);
                drop(evidence_account);
                drop(canonical_assembly);
                drop(evidence_assembly);
                return Err(RetainedOperationStartError::Writer(
                    RetainedWriterError::Ledger(cause),
                ));
            }
        };

        Ok(Self {
            coordinator,
            baseline: FallbackPublicationBaseline {
                fallback: Some(fallback),
                evidence: Some(evidence),
                batches: Some(batches),
            },
            terminal_writer: plan.terminal_writer,
            terminal_account_share_work: plan.terminal_account_share_work,
        })
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        self.baseline.account()
    }

    pub fn try_writer<Destination>(
        self,
    ) -> Result<
        RetainedWriter<O, P, PP, U, D, F, Destination, RetainedUnit>,
        RetainedWriterOpenError<Self, D>,
    >
    where
        Destination: PublicationDestination,
    {
        let account = match try_share_account_after_work(
            self.coordinator.retained_ledger(),
            self.account(),
            self.terminal_account_share_work,
        ) {
            Ok(account) => account,
            Err(cause) => {
                return Err(RetainedWriterOpenError {
                    operation: self,
                    cause,
                });
            }
        };
        let core = match RetainedWriterCore::try_new(
            self.coordinator.retained_ledger(),
            account,
            self.terminal_writer,
        ) {
            Ok(core) => core,
            Err(cause) => {
                return Err(RetainedWriterOpenError {
                    operation: self,
                    cause,
                });
            }
        };
        Ok(RetainedWriter {
            core,
            operation: self,
        })
    }
}

mod primary_destination_sealed {
    pub trait Sealed {}
}

pub(crate) struct PrimaryPublicationBatches<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence: Option<AttributionReservationBatch<O, P, D, PublishedOutcomeEvidence>>,
}

pub(crate) trait PrimaryDestinationRoute:
    primary_destination_sealed::Sealed + PublicationDestination + Sized
{
    fn route<O, P, D>(
        batch: AttributionReservationBatch<O, P, D, Self>,
    ) -> PrimaryPublicationBatches<O, P, D>
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord;
}

impl primary_destination_sealed::Sealed for CanonicalOutput {}

impl PrimaryDestinationRoute for CanonicalOutput {
    fn route<O, P, D>(
        batch: AttributionReservationBatch<O, P, D, Self>,
    ) -> PrimaryPublicationBatches<O, P, D>
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord,
    {
        PrimaryPublicationBatches {
            canonical: Some(batch),
            evidence: None,
        }
    }
}

impl primary_destination_sealed::Sealed for PublishedOutcomeEvidence {}

impl PrimaryDestinationRoute for PublishedOutcomeEvidence {
    fn route<O, P, D>(
        batch: AttributionReservationBatch<O, P, D, Self>,
    ) -> PrimaryPublicationBatches<O, P, D>
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord,
    {
        PrimaryPublicationBatches {
            canonical: None,
            evidence: Some(batch),
        }
    }
}

impl<O, P, PP, U, D, F, Destination, Rich> RetainedCandidate<O, P, PP, U, D, F, Destination, Rich>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    Destination: PrimaryDestinationRoute,
    Rich: RetainedProduct,
{
    fn prepare_primary(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> PreparedOperationPublication<Rich, F, D> {
        let Self {
            mut part,
            operation,
        } = self;
        let RetainedOperation {
            mut coordinator,
            baseline,
            ..
        } = operation;
        let (mut fallback, evidence, baseline_batches) = baseline.into_parts();
        let control_cause = coordinator
            .try_prepare_typed_control(final_progress)
            .err()
            .map(|cause| PreparationFailureCode::from_control(&cause));
        let product_graph_valid = part.has_exact_product_graph()
            && part.account().is_same_account(fallback.account())
            && part.account().is_same_account(evidence.account());
        let rich_cause = control_cause
            .or_else(|| (!product_graph_valid).then_some(PreparationFailureCode::product_graph()));
        let ledger = coordinator.into_ledger_for_typed_publication();
        let PrimaryPublicationBatches {
            canonical: rich_canonical,
            evidence: rich_evidence,
        } = Destination::route(part.take_batch_for_prepare());
        if let Some(cause) = rich_cause {
            let rich_account = part.discard_aggregate_before_batch_release();
            let proof = baseline_batches.into_fallback_proof(ledger, rich_canonical, rich_evidence);
            return finish_fallback_from_proof(
                proof,
                fallback,
                evidence,
                cause,
                Some(rich_account),
            );
        }

        let proof =
            match ledger.try_prepare_primary(baseline_batches, rich_canonical, rich_evidence) {
                Ok(proof) => proof,
                Err(rejection) => {
                    let cause = PreparationFailureCode::ledger();
                    let _ = rejection.cause();
                    let rich_account = part.discard_aggregate_before_batch_release();
                    let proof = rejection.into_fallback_proof();
                    return finish_fallback_from_proof(
                        proof,
                        fallback,
                        evidence,
                        cause,
                        Some(rich_account),
                    );
                }
            };

        let fallback_account = fallback.discard_aggregate_before_batch_release();
        let snapshot = proof.freeze();
        drop(fallback_account);
        evidence
            .aggregate()
            .right()
            .allocation()
            .as_ref()
            .freeze_from(&snapshot);
        drop(snapshot);
        PreparedOperationPublication {
            payload: OperationPublicationPayload::Rich(part.into_published_part()),
            evidence: evidence.into_published_part(),
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, PP, U, D, F, Rich> RetainedCandidate<O, P, PP, U, D, F, CanonicalOutput, Rich>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    Rich: RetainedProduct,
{
    pub(crate) fn prepare_success(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> PreparedOperationPublication<Rich, F, D> {
        self.prepare_primary(final_progress)
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, PP, U, D, F, Rich> RetainedCandidate<O, P, PP, U, D, F, PublishedOutcomeEvidence, Rich>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    Rich: RetainedProduct,
{
    pub(crate) fn prepare_failure(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> PreparedOperationPublication<Rich, F, D> {
        self.prepare_primary(final_progress)
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub struct RetainedWriterOpenError<Operation, D> {
    operation: Operation,
    cause: RetainedWriterError<D>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<Operation, D> RetainedWriterOpenError<Operation, D> {
    pub fn cause(&self) -> &RetainedWriterError<D> {
        &self.cause
    }

    pub fn into_operation(self) -> Operation {
        self.operation
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub struct RetainedWriterTransitionError<Writer, D> {
    writer: Writer,
    cause: RetainedWriterError<D>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<Writer, D> RetainedWriterTransitionError<Writer, D> {
    pub fn cause(&self) -> &RetainedWriterError<D> {
        &self.cause
    }

    pub fn into_writer(self) -> Writer {
        self.writer
    }
}

/// A linear retained writer that keeps the operation owner coupled to its candidate.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub struct RetainedWriter<O, P, PP, U, D, F, Destination, Bundle>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
    Destination: PublicationDestination,
    Bundle: RetainedProduct,
{
    core: RetainedWriterCore<O, P, D, Destination, Bundle>,
    operation: RetainedOperation<O, P, PP, U, D, F>,
}

#[allow(
    dead_code,
    clippy::result_large_err,
    clippy::type_complexity,
    reason = "linear writer transitions return the intact writer without failure-path allocation"
)]
impl<O, P, PP, U, D, F, Destination, Bundle> RetainedWriter<O, P, PP, U, D, F, Destination, Bundle>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    Destination: PublicationDestination,
    Bundle: RetainedProduct,
{
    pub fn try_retain<T>(
        self,
        plan: RetainedAllocationPlan,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<
        RetainedWriter<O, P, PP, U, D, F, Destination, RetainedPair<Bundle, RetainedLeaf<T>>>,
        RetainedWriterTransitionError<Self, D>,
    >
    where
        T: Send + Sync + 'static,
    {
        let Self { core, operation } = self;
        match core.try_retain(operation.coordinator.retained_ledger(), plan, build) {
            Ok(core) => Ok(RetainedWriter { core, operation }),
            Err(error) => {
                let (core, cause) = error.into_parts();
                Err(RetainedWriterTransitionError {
                    writer: RetainedWriter { core, operation },
                    cause,
                })
            }
        }
    }

    pub fn try_share<T>(
        self,
        source: &RetainedAllocation<T>,
        work: RetainedSharePlan<D>,
    ) -> Result<
        RetainedWriter<O, P, PP, U, D, F, Destination, RetainedPair<Bundle, RetainedLeaf<T>>>,
        RetainedWriterTransitionError<Self, D>,
    >
    where
        T: Send + Sync + 'static,
    {
        let Self { core, operation } = self;
        match core.try_share(operation.coordinator.retained_ledger(), source, work) {
            Ok(core) => Ok(RetainedWriter { core, operation }),
            Err(error) => {
                let (core, cause) = error.into_parts();
                Err(RetainedWriterTransitionError {
                    writer: RetainedWriter { core, operation },
                    cause,
                })
            }
        }
    }

    pub fn try_commit(
        &mut self,
        commitment: OutputCommitment<D, Destination>,
    ) -> Result<(), RetainedWriterError<D>> {
        self.core.try_commit(commitment)
    }

    pub fn abandon(self) -> RetainedOperation<O, P, PP, U, D, F> {
        let Self { core, operation } = self;
        drop(core);
        operation
    }

    pub fn finish(self) -> RetainedCandidate<O, P, PP, U, D, F, Destination, Bundle>
    where
        Bundle: NonEmptyRetainedProduct,
    {
        let Self { core, operation } = self;
        RetainedCandidate {
            part: core.finish(),
            operation,
        }
    }
}

/// One intended outcome inseparably coupled to the operation that can prepare it.
pub struct RetainedCandidate<O, P, PP, U, D, F, Destination, Aggregate>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
    Destination: PublicationDestination,
{
    part: RetainedPart<Aggregate, Destination, O, P, D>,
    operation: RetainedOperation<O, P, PP, U, D, F>,
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

    const fn product_graph() -> Self {
        Self {
            class: PreparationFailureClass::ProductGraph,
            code: 1,
        }
    }

    const fn ledger() -> Self {
        Self {
            class: PreparationFailureClass::Ledger,
            code: 1,
        }
    }

    pub(crate) const fn class(self) -> PreparationFailureClass {
        self.class
    }

    pub(crate) const fn code(self) -> u32 {
        self.code
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
enum OperationPublicationPayload<Rich, F> {
    Rich(PublishedPart<Rich>),
    Fallback {
        part: PublishedPart<SingleRetained<F>>,
        cause: PreparationFailureCode,
    },
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct PreparedOperationPublication<Rich, F, D> {
    payload: OperationPublicationPayload<Rich, F>,
    evidence: PublishedPart<SingleRetained<PublicationEvidenceCells<D>>>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<Rich, F, D> PreparedOperationPublication<Rich, F, D> {
    pub(crate) fn uses_fallback(&self) -> bool {
        matches!(self.payload, OperationPublicationPayload::Fallback { .. })
    }

    // Stable Rust rejects a const-consuming generic with destructor-bearing fields (E0493).
    // U4's named public wrapper must retain this identity/drop behavior gate; a global
    // no-allocation proof remains intentionally outside this private transition.
    pub(crate) fn handoff(self) -> PublishedOperationPublication<Rich, F, D> {
        let Self { payload, evidence } = self;
        PublishedOperationPublication { payload, evidence }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct PublishedOperationPublication<Rich, F, D> {
    payload: OperationPublicationPayload<Rich, F>,
    evidence: PublishedPart<SingleRetained<PublicationEvidenceCells<D>>>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<Rich, F, D> PublishedOperationPublication<Rich, F, D>
where
    D: Copy + Ord,
{
    pub(crate) fn rich(&self) -> Option<&Rich> {
        match &self.payload {
            OperationPublicationPayload::Rich(part) => Some(part.aggregate()),
            OperationPublicationPayload::Fallback { .. } => None,
        }
    }

    pub(crate) fn fallback(&self) -> Option<&F> {
        match &self.payload {
            OperationPublicationPayload::Rich(_) => None,
            OperationPublicationPayload::Fallback { part, .. } => {
                Some(part.aggregate().right().allocation().as_ref())
            }
        }
    }

    pub(crate) fn fallback_cause(&self) -> Option<PreparationFailureCode> {
        match &self.payload {
            OperationPublicationPayload::Rich(_) => None,
            OperationPublicationPayload::Fallback { cause, .. } => Some(*cause),
        }
    }

    pub(crate) fn dimension(&self, dimension: D) -> Option<PublishedDimensionSnapshot<D>> {
        self.evidence
            .aggregate()
            .right()
            .allocation()
            .as_ref()
            .dimension(dimension)
    }

    pub(crate) fn account(&self) -> &PhysicalStorageAccount {
        self.evidence.account()
    }
}

fn finish_fallback_from_proof<O, P, D, F, Rich>(
    proof: crate::ledger::TypedFreezeProof<O, P, D>,
    fallback: RetainedPart<SingleRetained<F>, PublishedOutcomeEvidence, O, P, D>,
    evidence: RetainedPart<
        SingleRetained<PublicationEvidenceCells<D>>,
        PublishedOutcomeEvidence,
        O,
        P,
        D,
    >,
    cause: PreparationFailureCode,
    discarded_account: Option<PhysicalStorageAccount>,
) -> PreparedOperationPublication<Rich, F, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    let snapshot = proof.freeze();
    drop(discarded_account);
    evidence
        .aggregate()
        .right()
        .allocation()
        .as_ref()
        .freeze_from(&snapshot);
    drop(snapshot);
    PreparedOperationPublication {
        payload: OperationPublicationPayload::Fallback {
            part: fallback.into_published_part(),
            cause,
        },
        evidence: evidence.into_published_part(),
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
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
    pub(crate) fn prepare_fallback(self) -> PreparedOperationPublication<RetainedUnit, F, D> {
        let Self {
            mut coordinator,
            baseline,
            ..
        } = self;
        let cause = coordinator
            .poll()
            .err()
            .map_or_else(PreparationFailureCode::requested, |cause| {
                PreparationFailureCode::from_control(&cause)
            });
        let ledger = coordinator.into_ledger_for_typed_publication();
        let (fallback, evidence, baseline_batches) = baseline.into_parts();
        let proof = baseline_batches.into_fallback_proof(ledger, None, None);
        finish_fallback_from_proof(proof, fallback, evidence, cause, None)
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_FINAL_OWNER_ALLOCATION: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static KNOWN_BUILDER_PANIC_AFTER_DISCARD: std::cell::Cell<
        Option<fn(&PhysicalStorageAccount)>,
    > = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn set_known_builder_panic_after_discard_observer(observer: fn(&PhysicalStorageAccount)) {
    KNOWN_BUILDER_PANIC_AFTER_DISCARD.with(|slot| slot.set(Some(observer)));
}

#[cfg(test)]
fn observe_known_builder_panic_after_discard(account: &PhysicalStorageAccount) {
    KNOWN_BUILDER_PANIC_AFTER_DISCARD.with(|slot| {
        if let Some(observer) = slot.take() {
            observer(account);
        }
    });
}

#[cfg(test)]
fn fail_next_final_owner_allocation() {
    FAIL_NEXT_FINAL_OWNER_ALLOCATION.with(|flag| flag.set(true));
}

#[cfg(test)]
fn take_injected_final_owner_allocation_failure() -> bool {
    FAIL_NEXT_FINAL_OWNER_ALLOCATION.with(|flag| flag.replace(false))
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

    struct PanickingDropProbe(&'static AtomicU64);

    impl Drop for PanickingDropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
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

    type TestOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, u32>;

    fn share_work() -> RetainedSharePlan<TestDimension> {
        RetainedSharePlan::new(
            TestDimension::Bytes,
            TestDimension::Bytes,
            TestDimension::Bytes,
        )
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
        match TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok(7_u32)
        }) {
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

    fn empty_plan() -> RetainedAllocationPlan {
        RetainedAllocationPlan::new(0, 0).unwrap()
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
        type Lease = AdmissionLease<Arc<FactoryState>>;
        let control_bytes =
            FinalOwnerAllocation::<AccountState, Lease>::checked_layout_bytes().unwrap();
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
    fn concurrent_factory_admission_cannot_oversubscribe_one_account_slot() {
        type Lease = AdmissionLease<Arc<FactoryState>>;
        let control_bytes =
            FinalOwnerAllocation::<AccountState, Lease>::checked_layout_bytes().unwrap();
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
        type Lease = AdmissionLease<PhysicalStorageAccount>;
        let outer =
            FinalOwnerAllocation::<RetainedPayload<u64>, Lease>::checked_layout_bytes().unwrap();
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
                    account: account.share_after_admission(),
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
    fn known_panic_payload_is_destroyed_before_charge_release() {
        static OBSERVED_BYTES: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

        fn observe_after_discard(account: &PhysicalStorageAccount) {
            let snapshot = account.snapshot();
            OBSERVED_BYTES.store(snapshot.live_retained_bytes(), Ordering::SeqCst);
            OBSERVED_ALLOCATIONS.store(snapshot.live_retained_allocations(), Ordering::SeqCst);
        }

        let factory = account_factory(1);
        let account = account(&factory, 1_024, 8);

        OBSERVED_BYTES.store(0, Ordering::SeqCst);
        OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
        set_known_builder_panic_after_discard_observer(observe_after_discard);
        let error = account
            .try_retain_with::<u64>(empty_plan(), || panic!("test panic"))
            .unwrap_err();
        assert_eq!(
            error,
            RetainedAllocationError::BuilderPanicked {
                charge_retained: false,
            }
        );
        assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);

        OBSERVED_BYTES.store(0, Ordering::SeqCst);
        OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
        set_known_builder_panic_after_discard_observer(observe_after_discard);
        let error = account
            .try_retain_with::<u64>(empty_plan(), || {
                std::panic::panic_any(String::from("owned test panic"))
            })
            .unwrap_err();
        assert_eq!(
            error,
            RetainedAllocationError::BuilderPanicked {
                charge_retained: false,
            }
        );
        assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn opaque_leaked_panic_payload_keeps_its_charge_fail_closed() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let plan = RetainedAllocationPlan::new(64, 1).unwrap();
        let error = account
            .try_retain_with::<u64>(plan, || std::panic::panic_any(vec![0_u8; 64]))
            .unwrap_err();

        assert_eq!(
            error,
            RetainedAllocationError::BuilderPanicked {
                charge_retained: true,
            }
        );
        let retained_bytes = account.snapshot().live_retained_bytes();
        assert!(retained_bytes > 64);
        assert_eq!(account.snapshot().live_retained_bytes(), retained_bytes);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn injected_shared_allocation_failure_drops_payload_before_refunding_charge() {
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
                account: account.share_after_admission(),
            })
        });

        assert!(matches!(
            result,
            Err(RetainedAllocationError::AllocationFailed)
        ));
        assert!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst) > 0);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }

    #[test]
    fn allocation_failure_contains_payload_drop_panic_and_retains_the_lease() {
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
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert!(account.snapshot().live_retained_bytes() > 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 1);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
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

        type Lease = AdmissionLease<PhysicalStorageAccount>;
        let outer =
            FinalOwnerAllocation::<RetainedPayload<BlockingDrop>, Lease>::checked_layout_bytes()
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
                    account: account.share_after_admission(),
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
    fn writer_candidate_and_baseline_drop_payload_before_releasing_live_ownership() {
        static EXPECTED_ACTIVE: AtomicU64 = AtomicU64::new(0);
        static EXPECTED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
        static EXPECTED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
        static EXPECTED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
        static OBSERVED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

        fn reset_observations() {
            for counter in [
                &EXPECTED_ACTIVE,
                &EXPECTED_OPERATION_LIVE,
                &EXPECTED_PHYSICAL_BYTES,
                &EXPECTED_PHYSICAL_ALLOCATIONS,
                &OBSERVED_ACTIVE,
                &OBSERVED_OPERATION_LIVE,
                &OBSERVED_PHYSICAL_BYTES,
                &OBSERVED_PHYSICAL_ALLOCATIONS,
            ] {
                counter.store(0, Ordering::SeqCst);
            }
        }

        fn assert_live_ownership_observed() {
            assert!(EXPECTED_ACTIVE.load(Ordering::SeqCst) > 0);
            assert!(EXPECTED_OPERATION_LIVE.load(Ordering::SeqCst) > 0);
            assert!(EXPECTED_PHYSICAL_BYTES.load(Ordering::SeqCst) > 0);
            assert!(EXPECTED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst) > 0);
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
        }

        reset_observations();
        let factory = account_factory(1);
        let writer = test_operation(&factory, 64)
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("writer observation operation must open"));
        let ledger = writer.operation.coordinator.retained_ledger().test_probe();
        let observed_account = writer.operation.account().share_after_admission();
        let writer = writer
            .try_retain(empty_plan(), move || {
                EXPECTED_ACTIVE.store(ledger.active_reservations(), Ordering::SeqCst);
                EXPECTED_OPERATION_LIVE.store(ledger.current_live_bytes(), Ordering::SeqCst);
                let physical = observed_account.snapshot();
                EXPECTED_PHYSICAL_BYTES.store(physical.live_retained_bytes(), Ordering::SeqCst);
                EXPECTED_PHYSICAL_ALLOCATIONS
                    .store(physical.live_retained_allocations(), Ordering::SeqCst);
                Ok(LiveDropObservation {
                    ledger,
                    account: observed_account,
                    observed_active: &OBSERVED_ACTIVE,
                    observed_operation_live: &OBSERVED_OPERATION_LIVE,
                    observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
                    observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
                })
            })
            .unwrap_or_else(|_| panic!("writer observation payload must be retained"));
        drop(writer);
        assert_live_ownership_observed();
        assert_eq!(factory.snapshot().live_accounts(), 0);

        reset_observations();
        let factory = account_factory(1);
        let writer = test_operation(&factory, 64)
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("candidate observation operation must open"));
        let ledger = writer.operation.coordinator.retained_ledger().test_probe();
        let observed_account = writer.operation.account().share_after_admission();
        let candidate = writer
            .try_retain(empty_plan(), move || {
                EXPECTED_ACTIVE.store(ledger.active_reservations(), Ordering::SeqCst);
                EXPECTED_OPERATION_LIVE.store(ledger.current_live_bytes(), Ordering::SeqCst);
                let physical = observed_account.snapshot();
                EXPECTED_PHYSICAL_BYTES.store(physical.live_retained_bytes(), Ordering::SeqCst);
                EXPECTED_PHYSICAL_ALLOCATIONS
                    .store(physical.live_retained_allocations(), Ordering::SeqCst);
                Ok(LiveDropObservation {
                    ledger,
                    account: observed_account,
                    observed_active: &OBSERVED_ACTIVE,
                    observed_operation_live: &OBSERVED_OPERATION_LIVE,
                    observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
                    observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
                })
            })
            .unwrap_or_else(|_| panic!("candidate observation payload must be retained"))
            .finish();
        drop(candidate);
        assert_live_ownership_observed();
        assert_eq!(factory.snapshot().live_accounts(), 0);

        reset_observations();
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let deferred_ledger = StdArc::new(StdMutex::new(None));
        let fallback = DeferredLiveDropObservation {
            ledger: StdArc::clone(&deferred_ledger),
            account: account.share_after_admission(),
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
        assert_live_ownership_observed();
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn all_linear_publication_owners_contain_payload_drop_panics() {
        static WRITER_DROPS: AtomicU64 = AtomicU64::new(0);
        static CANDIDATE_DROPS: AtomicU64 = AtomicU64::new(0);
        static PREPARED_DROPS: AtomicU64 = AtomicU64::new(0);
        static PUBLISHED_DROPS: AtomicU64 = AtomicU64::new(0);
        static BASELINE_DROPS: AtomicU64 = AtomicU64::new(0);

        let factory = account_factory(1);
        WRITER_DROPS.store(0, Ordering::SeqCst);
        let writer = test_operation(&factory, 64)
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("writer drop operation must open"))
            .try_retain(empty_plan(), || Ok(PanickingDropProbe(&WRITER_DROPS)))
            .unwrap_or_else(|_| panic!("writer drop probe must be retained"));
        assert!(catch_unwind(AssertUnwindSafe(|| drop(writer))).is_ok());
        assert_eq!(WRITER_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().live_accounts(), 1);

        let factory = account_factory(1);
        CANDIDATE_DROPS.store(0, Ordering::SeqCst);
        let candidate = test_operation(&factory, 64)
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("candidate drop operation must open"))
            .try_retain(empty_plan(), || Ok(PanickingDropProbe(&CANDIDATE_DROPS)))
            .unwrap_or_else(|_| panic!("candidate drop probe must be retained"))
            .finish();
        assert!(catch_unwind(AssertUnwindSafe(|| drop(candidate))).is_ok());
        assert_eq!(CANDIDATE_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().live_accounts(), 1);

        let factory = account_factory(1);
        PREPARED_DROPS.store(0, Ordering::SeqCst);
        let mut writer = test_operation(&factory, 64)
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("prepared drop operation must open"))
            .try_retain(empty_plan(), || Ok(PanickingDropProbe(&PREPARED_DROPS)))
            .unwrap_or_else(|_| panic!("prepared drop probe must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 1))
            .unwrap();
        let prepared = writer.finish().prepare_success(final_progress());
        assert!(catch_unwind(AssertUnwindSafe(|| drop(prepared))).is_ok());
        assert_eq!(PREPARED_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(factory.snapshot().live_accounts(), 1);

        let factory = account_factory(1);
        PUBLISHED_DROPS.store(0, Ordering::SeqCst);
        let mut writer = test_operation(&factory, 64)
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("published drop operation must open"))
            .try_retain(empty_plan(), || Ok(PanickingDropProbe(&PUBLISHED_DROPS)))
            .unwrap_or_else(|_| panic!("published drop probe must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 1))
            .unwrap();
        let published = writer.finish().prepare_success(final_progress()).handoff();
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

        fn reset_observations() {
            DROP_COUNT.store(0, Ordering::SeqCst);
            OBSERVED_BYTES.store(0, Ordering::SeqCst);
            OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
        }

        reset_observations();
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let observed_account = operation.account().share_after_admission();
        let mut writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("prepared physical observation writer must open"))
            .try_retain(empty_plan(), move || {
                Ok(PhysicalDropObservation {
                    account: observed_account,
                    drop_count: &DROP_COUNT,
                    observed_bytes: &OBSERVED_BYTES,
                    observed_allocations: &OBSERVED_ALLOCATIONS,
                })
            })
            .unwrap_or_else(|_| panic!("prepared physical observation must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 1))
            .unwrap();
        let prepared = writer.finish().prepare_success(final_progress());
        drop(prepared);
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
        assert_eq!(factory.snapshot().live_accounts(), 0);

        reset_observations();
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let observed_account = operation.account().share_after_admission();
        let mut writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("handoff observation writer must open"))
            .try_retain(empty_plan(), move || {
                Ok(PhysicalDropObservation {
                    account: observed_account,
                    drop_count: &DROP_COUNT,
                    observed_bytes: &OBSERVED_BYTES,
                    observed_allocations: &OBSERVED_ALLOCATIONS,
                })
            })
            .unwrap_or_else(|_| panic!("handoff observation must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 1))
            .unwrap();
        let prepared = writer.finish().prepare_success(final_progress());
        let (rich_identity, rich_share_calls) = match &prepared.payload {
            OperationPublicationPayload::Rich(part) => {
                let allocation = part.aggregate().right().allocation();
                (
                    allocation.allocation_identity(),
                    allocation.share_call_count(),
                )
            }
            OperationPublicationPayload::Fallback { .. } => {
                panic!("handoff observation must prepare the rich output")
            }
        };
        let evidence_allocation = prepared.evidence.aggregate().right().allocation();
        let evidence_identity = evidence_allocation.allocation_identity();
        let evidence_share_calls = evidence_allocation.share_call_count();

        let published = prepared.handoff();
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
        assert_eq!(OBSERVED_BYTES.load(Ordering::SeqCst), 0);
        assert_eq!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst), 0);
        let rich_allocation = published
            .rich()
            .expect("handoff preserves the rich product")
            .right()
            .allocation();
        assert_eq!(rich_allocation.allocation_identity(), rich_identity);
        assert_eq!(rich_allocation.share_call_count(), rich_share_calls);
        let evidence_allocation = published.evidence.aggregate().right().allocation();
        assert_eq!(evidence_allocation.allocation_identity(), evidence_identity);
        assert_eq!(evidence_allocation.share_call_count(), evidence_share_calls);

        drop(published);
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
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
        assert!(first_account.is_same_account(&first_account.share_after_admission()));
        assert!(!first_account.is_same_account(&second_account));
    }

    #[test]
    fn concurrent_exact_limit_admission_is_linearizable() {
        type Lease = AdmissionLease<PhysicalStorageAccount>;
        let outer =
            FinalOwnerAllocation::<RetainedPayload<u64>, Lease>::checked_layout_bytes().unwrap();
        let factory = account_factory(1);
        let account = account(&factory, outer, 1);
        let start = StdArc::new(Barrier::new(3));
        let attempts_complete = StdArc::new(Barrier::new(3));
        let release_owner = StdArc::new(Barrier::new(2));
        let successes = StdArc::new(AtomicU64::new(0));
        let mut threads = Vec::new();
        for value in [1_u64, 2_u64] {
            let account = account.share_after_admission();
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
        let source = include_str!("retained.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();
        let (production, test_module) = source
            .rsplit_once(&test_attribute)
            .expect("retained source contains one final test module boundary");
        assert!(test_module.trim_start().starts_with("mod tests"));
        assert!(production.contains("Arc::into_unique(shared)"));
        for forbidden in [
            "Arc::try_unique",
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
    fn physical_leaf_mutators_are_not_crate_or_public_api() {
        let source = include_str!("retained.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();
        let (production, _) = source
            .rsplit_once(&test_attribute)
            .expect("retained source contains one final test module boundary");

        for method in ["try_retain_with", "share_after_admission"] {
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
        let _ = <RetainedUnit as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <RetainedLeaf<u64> as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <RetainedPair<RetainedUnit, RetainedLeaf<u64>> as AmbiguousIfClone<_>>::assert_not_clone;
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
        type EvidenceLease = AdmissionLease<PhysicalStorageAccount>;

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
            RetainedSharePlan::new(0, 0, 0),
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
        let allocation = evidence.aggregate().right().allocation();
        let outer_bytes = FinalOwnerAllocation::<
            RetainedPayload<PublicationEvidenceCells<u8>>,
            EvidenceLease,
        >::checked_layout_bytes()
        .unwrap();

        assert_eq!(capacity.envelope_capacity(), 32);
        assert_eq!(allocation.charged_bytes(), outer_bytes + envelope_bytes);
        assert_eq!(allocation.charged_allocations(), 2);
        drop(operation.prepare_fallback().handoff());
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn account_and_retained_shares_charge_fixed_work_before_each_clone() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let start_evidence = operation
            .coordinator
            .retained_ledger()
            .dimension_evidence(TestDimension::Bytes)
            .unwrap();
        assert_eq!(start_evidence.attempted(), 6);
        assert_eq!(start_evidence.consumed(), 6);
        assert_eq!(operation.account().share_call_count(), 4);

        let source = operation
            .account()
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(13_u64))
            .unwrap();
        assert_eq!(source.share_call_count(), 0);
        assert_eq!(operation.account().share_call_count(), 5);
        let writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("canonical writer must open"));
        let open_evidence = writer
            .operation
            .coordinator
            .retained_ledger()
            .dimension_evidence(TestDimension::Bytes)
            .unwrap();
        assert_eq!(open_evidence.attempted(), 9);
        assert_eq!(open_evidence.consumed(), 9);
        assert_eq!(writer.operation.account().share_call_count(), 6);

        let writer = writer
            .try_share(&source, share_work())
            .unwrap_or_else(|_| panic!("same-account share must succeed"));
        let share_evidence = writer
            .operation
            .coordinator
            .retained_ledger()
            .dimension_evidence(TestDimension::Bytes)
            .unwrap();
        assert_eq!(share_evidence.attempted(), 12);
        assert_eq!(share_evidence.consumed(), 12);
        assert_eq!(source.share_call_count(), 1);

        drop(writer.abandon().prepare_fallback().handoff());
        drop(source);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn physical_rejection_rolls_back_writer_attribution() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("canonical writer must open"));
        let before = writer
            .operation
            .coordinator
            .retained_ledger()
            .try_snapshot()
            .unwrap();
        fail_next_final_owner_allocation();
        let error = match writer.try_retain(empty_plan(), || Ok(19_u64)) {
            Ok(_) => panic!("injected physical failure must reject the retained leaf"),
            Err(error) => error,
        };
        assert!(matches!(
            error.cause(),
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
        ));
        let writer = error.into_writer();
        let after = writer
            .operation
            .coordinator
            .retained_ledger()
            .try_snapshot()
            .unwrap();
        assert_eq!(after.active_reservations(), before.active_reservations());
        assert_eq!(after.current_live_bytes(), before.current_live_bytes());

        drop(writer.abandon().prepare_fallback().handoff());
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn foreign_source_rejects_before_work_charge_or_clone() {
        let factory = account_factory(2);
        let operation = test_operation(&factory, 64);
        let foreign_account = account(&factory, 4_096, 8);
        let source = foreign_account
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(23_u64))
            .unwrap();
        let writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("canonical writer must open"));
        let before = writer
            .operation
            .coordinator
            .retained_ledger()
            .dimension_evidence(TestDimension::Bytes)
            .unwrap();
        let strong_count = source.strong_count();
        let share_calls = source.share_call_count();
        let error = match writer.try_share(&source, share_work()) {
            Ok(_) => panic!("foreign retained storage must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error.cause(), RetainedWriterError::ForeignAccount));
        assert_eq!(source.strong_count(), strong_count);
        assert_eq!(source.share_call_count(), share_calls);
        let writer = error.into_writer();
        let after = writer
            .operation
            .coordinator
            .retained_ledger()
            .dimension_evidence(TestDimension::Bytes)
            .unwrap();
        assert_eq!(after.attempted(), before.attempted());
        assert_eq!(after.consumed(), before.consumed());

        drop(writer.abandon().prepare_fallback().handoff());
        drop(source);
        drop(foreign_account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn share_work_rejection_happens_before_the_reference_count_change() {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let policy = DimensionPolicy::reservable_bytes(11, 11, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(64);
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
            .unwrap_or_else(|_| panic!("bounded operation must start"));
        let source = operation
            .account()
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(29_u64))
            .unwrap();
        let writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("canonical writer must open"));
        let strong_count = source.strong_count();
        let share_calls = source.share_call_count();
        let error = match writer.try_share(&source, share_work()) {
            Ok(_) => panic!("work limit must reject the retained share"),
            Err(error) => error,
        };
        assert!(matches!(
            error.cause(),
            RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
        ));
        assert_eq!(source.strong_count(), strong_count);
        assert_eq!(source.share_call_count(), share_calls);
        let writer = error.into_writer();
        let evidence = writer
            .operation
            .coordinator
            .retained_ledger()
            .dimension_evidence(TestDimension::Bytes)
            .unwrap();
        assert_eq!(evidence.attempted(), 12);
        assert_eq!(evidence.consumed(), 11);

        drop(writer.abandon().prepare_fallback().handoff());
        drop(source);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn second_start_share_rejection_preserves_the_borrowed_host_account() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 16);
        let policy = DimensionPolicy::reservable_bytes(5, 5, u64::MAX, u64::MAX)
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
        assert_eq!(account.share_call_count(), 1);
        let retained = account
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(9_u32))
            .expect("borrowed host account remains usable after start rejection");
        drop(retained);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn retained_success_commits_canonical_without_publishing_fallback() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let writer = match operation.try_writer::<CanonicalOutput>() {
            Ok(writer) => writer,
            Err(_) => panic!("canonical writer must open"),
        };
        let mut writer = match writer.try_retain(empty_plan(), || Ok(42_u64)) {
            Ok(writer) => writer,
            Err(_) => panic!("canonical leaf must be retained"),
        };
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 5))
            .unwrap();

        let prepared = writer.finish().prepare_success(final_progress());
        assert!(!prepared.uses_fallback());
        let published = prepared.handoff();
        assert_eq!(*published.rich().unwrap().right().allocation().as_ref(), 42);
        assert!(published.fallback().is_none());
        let dimension = published.dimension(TestDimension::Bytes).unwrap();
        assert_eq!(dimension.committed_canonical_output(), 5);
        assert_eq!(dimension.published_outcome_evidence(), 1);
        assert!(published.account().snapshot().live_retained_bytes() > 0);

        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn canonical_commitment_over_limit_falls_back_without_partial_commit() {
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
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 4, u64::MAX)
            .with_published_outcome_evidence_limit(64);
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
            .unwrap_or_else(|_| panic!("bounded canonical operation must start"));
        let writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("canonical writer must open"));
        let ledger = writer.operation.coordinator.retained_ledger().test_probe();
        let observed_account = writer.operation.account().share_after_admission();
        let mut writer = writer
            .try_retain(empty_plan(), move || {
                EXPECTED_ACTIVE.store(ledger.active_reservations(), Ordering::SeqCst);
                EXPECTED_OPERATION_LIVE.store(ledger.current_live_bytes(), Ordering::SeqCst);
                let physical = observed_account.snapshot();
                EXPECTED_PHYSICAL_BYTES.store(physical.live_retained_bytes(), Ordering::SeqCst);
                EXPECTED_PHYSICAL_ALLOCATIONS
                    .store(physical.live_retained_allocations(), Ordering::SeqCst);
                Ok(LiveDropObservation {
                    ledger,
                    account: observed_account,
                    observed_active: &OBSERVED_ACTIVE,
                    observed_operation_live: &OBSERVED_OPERATION_LIVE,
                    observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
                    observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
                })
            })
            .unwrap_or_else(|_| panic!("canonical leaf must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 5))
            .unwrap();

        let prepared = writer.finish().prepare_success(final_progress());
        assert!(prepared.uses_fallback());
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
        let published = prepared.handoff();
        let dimension = published.dimension(TestDimension::Bytes).unwrap();
        assert_eq!(dimension.committed_canonical_output(), 0);
        assert_eq!(dimension.published_outcome_evidence(), 2);
        drop(published);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn retained_rich_failure_publishes_only_evidence_destination() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let writer = match operation.try_writer::<PublishedOutcomeEvidence>() {
            Ok(writer) => writer,
            Err(_) => panic!("evidence writer must open"),
        };
        let mut writer = match writer.try_retain(empty_plan(), || Ok(99_u64)) {
            Ok(writer) => writer,
            Err(_) => panic!("evidence leaf must be retained"),
        };
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 7))
            .unwrap();

        let published = writer.finish().prepare_failure(final_progress()).handoff();
        let dimension = published.dimension(TestDimension::Bytes).unwrap();
        assert_eq!(dimension.committed_canonical_output(), 0);
        assert_eq!(dimension.published_outcome_evidence(), 8);
        assert!(published.fallback().is_none());
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn evidence_assembly_absorbs_a_full_terminal_writer_plus_snapshot() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let writer = operation
            .try_writer::<PublishedOutcomeEvidence>()
            .unwrap_or_else(|_| panic!("evidence writer must open"));
        let writer = writer
            .try_retain(empty_plan(), || Ok(31_u32))
            .unwrap_or_else(|_| panic!("first evidence leaf must be retained"));
        let mut writer = writer
            .try_retain(empty_plan(), || Ok(37_u64))
            .unwrap_or_else(|_| panic!("second evidence leaf must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 2))
            .unwrap();
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 3))
            .unwrap();

        let prepared = writer.finish().prepare_failure(final_progress());
        assert!(!prepared.uses_fallback());
        let published = prepared.handoff();
        assert_eq!(
            published
                .dimension(TestDimension::Bytes)
                .unwrap()
                .published_outcome_evidence(),
            6
        );
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn prepare_success_contains_fallback_drop_panic() {
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
        let mut writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("canonical writer must open"))
            .try_retain(empty_plan(), || Ok(41_u64))
            .unwrap_or_else(|_| panic!("canonical leaf must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 5))
            .unwrap();

        let prepared = catch_unwind(AssertUnwindSafe(|| {
            writer.finish().prepare_success(final_progress())
        }))
        .expect("fallback destructor panic must be contained");
        assert!(!prepared.uses_fallback());
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        drop(prepared.handoff());
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn rejected_primary_contains_payload_drop_panic_before_token_release() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        struct PanickingRich;

        impl Drop for PanickingRich {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
                std::panic::panic_any(vec![0_u8; 8]);
            }
        }

        let factory = account_factory(1);
        let operation = test_operation(&factory, 4);
        DROP_COUNT.store(0, Ordering::SeqCst);
        let mut writer = operation
            .try_writer::<PublishedOutcomeEvidence>()
            .unwrap_or_else(|_| panic!("evidence writer must open"))
            .try_retain(empty_plan(), || Ok(PanickingRich))
            .unwrap_or_else(|_| panic!("rich failure leaf must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 5))
            .unwrap();

        let prepared = catch_unwind(AssertUnwindSafe(|| {
            writer.finish().prepare_failure(final_progress())
        }))
        .expect("primary payload destructor panic must be contained");
        assert!(prepared.uses_fallback());
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        let published = prepared.handoff();
        assert_eq!(published.fallback(), Some(&7));
        assert_eq!(
            published
                .dimension(TestDimension::Bytes)
                .unwrap()
                .published_outcome_evidence(),
            2
        );
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn direct_and_rejected_primary_paths_use_the_preadmitted_fallback() {
        let factory = account_factory(1);
        let direct = test_operation(&factory, 4).prepare_fallback().handoff();
        assert_eq!(direct.fallback(), Some(&7));
        assert_eq!(
            direct.fallback_cause().unwrap().class(),
            PreparationFailureClass::Requested
        );
        assert_eq!(
            direct
                .dimension(TestDimension::Bytes)
                .unwrap()
                .published_outcome_evidence(),
            2
        );
        drop(direct);

        let operation = test_operation(&factory, 4);
        let writer = match operation.try_writer::<PublishedOutcomeEvidence>() {
            Ok(writer) => writer,
            Err(_) => panic!("evidence writer must open"),
        };
        let mut writer = match writer.try_retain(empty_plan(), || Ok(11_u64)) {
            Ok(writer) => writer,
            Err(_) => panic!("evidence leaf must be retained"),
        };
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 5))
            .unwrap();
        let prepared = writer.finish().prepare_failure(final_progress());
        assert!(prepared.uses_fallback());
        let published = prepared.handoff();
        assert_eq!(published.fallback(), Some(&7));
        let code = published.fallback_cause().unwrap();
        assert_eq!(code.class(), PreparationFailureClass::Ledger);
        assert_eq!(code.code(), 1);
        assert_eq!(
            published
                .dimension(TestDimension::Bytes)
                .unwrap()
                .published_outcome_evidence(),
            2
        );
        drop(published);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn typed_product_extends_heterogeneously_and_full_batch_fails_before_builder() {
        let factory = account_factory(1);
        let operation =
            test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(1, 2));
        let writer = match operation.try_writer::<CanonicalOutput>() {
            Ok(writer) => writer,
            Err(_) => panic!("canonical writer must open"),
        };
        let writer = match writer.try_retain(empty_plan(), || Ok(3_u32)) {
            Ok(writer) => writer,
            Err(_) => panic!("first leaf must be retained"),
        };
        let before = writer.operation.account().snapshot();
        let builder_called = AtomicBool::new(false);
        let error = match writer.try_retain(empty_plan(), || {
            builder_called.store(true, Ordering::SeqCst);
            Ok(5_u64)
        }) {
            Ok(_) => panic!("full attribution batch must reject the second leaf"),
            Err(error) => error,
        };
        assert!(matches!(
            error.cause(),
            RetainedWriterError::AttributionBatchFull
        ));
        assert!(!builder_called.load(Ordering::SeqCst));
        let writer = error.into_writer();
        let after = writer.operation.account().snapshot();
        assert_eq!(
            before.total_admitted_retained_bytes(),
            after.total_admitted_retained_bytes()
        );
        assert_eq!(
            before.total_admitted_retained_allocations(),
            after.total_admitted_retained_allocations()
        );
        let operation = writer.abandon();
        drop(operation.prepare_fallback().handoff());
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn typed_product_preserves_multiple_leaf_types_across_retain_and_share() {
        let factory = account_factory(1);
        let operation = test_operation(&factory, 64);
        let source = operation
            .account()
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(13_u64))
            .expect("shared source must be retained in the operation account");
        let writer = operation
            .try_writer::<CanonicalOutput>()
            .unwrap_or_else(|_| panic!("canonical writer must open"));
        let writer = writer
            .try_retain(empty_plan(), || Ok(3_u32))
            .unwrap_or_else(|_| panic!("first attributed leaf must be retained"));
        let writer = writer
            .try_share(&source, share_work())
            .unwrap_or_else(|_| panic!("same-account leaf must be shared"));
        let mut writer = writer
            .try_retain(empty_plan(), || Ok(true))
            .unwrap_or_else(|_| panic!("second attributed leaf must be retained"));
        writer
            .try_commit(OutputCommitment::new(TestDimension::Bytes, 5))
            .unwrap();

        let published = writer.finish().prepare_success(final_progress()).handoff();
        let rich = published
            .rich()
            .expect("canonical product must be published");
        assert_eq!(*rich.left().left().right().allocation().as_ref(), 3_u32);
        assert_eq!(*rich.left().right().allocation().as_ref(), 13_u64);
        assert!(*rich.right().allocation().as_ref());

        drop(published);
        drop(source);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn typed_product_and_primary_prepare_exclude_opaque_setters_and_duplicate_tail_work() {
        let source = include_str!("retained.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();
        let (production, _) = source
            .rsplit_once(&test_attribute)
            .expect("retained source contains one final test module boundary");
        assert!(!production.contains("set: fn("));
        assert!(!production.contains("AggregateSetterPanicked"));
        assert!(production.contains("struct RetainedUnit"));
        assert!(production.contains("struct RetainedLeaf<T>"));
        assert!(production.contains("struct RetainedPair<L, R>"));
        assert_eq!(production.matches(".try_prepare_typed_control(").count(), 1);
        assert_eq!(production.matches("ledger.try_prepare_primary(").count(), 1);
    }
}
