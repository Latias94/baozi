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
use std::any::TypeId;
use std::borrow::Borrow;
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
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
    observe_unverified_drop: fn(&L),
    #[cfg(test)]
    share_calls: AtomicUsize,
}

impl<T, L> FinalOwnerInner<T, L> {
    const fn new(value: T, lease: L, observe_unverified_drop: fn(&L)) -> Self {
        Self {
            value: Some(value),
            lease: FailClosedLease::new(lease),
            observe_unverified_drop,
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
            (self.observe_unverified_drop)(self.lease.get());
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
                (self.observe_unverified_drop)(self.lease.get());
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
        Self::try_new_observed(value, lease, |_| {})
    }

    fn try_new_observed(
        value: T,
        lease: L,
        observe_unverified_drop: fn(&L),
    ) -> Result<Self, FinalOwnerAllocationError> {
        let inner = FinalOwnerInner::new(value, lease, observe_unverified_drop);

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

    fn value_mut(&mut self) -> Option<&mut T> {
        let shared = self.shared.as_mut()?;
        Arc::get_mut(shared)?.value.as_mut()
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
            unverified_drop_count: AtomicU64::new(0),
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
    unverified_drop_count: AtomicU64,
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

    fn unverified_drop_count(&self) -> Option<u64> {
        let count = self
            .inner
            .value()
            .unverified_drop_count
            .load(Ordering::Acquire);
        (count != u64::MAX).then_some(count)
    }

    fn record_unverified_drop(&self) {
        let _ = self.inner.value().unverified_drop_count.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |count| count.checked_add(1),
        );
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
    fn try_retain_with_admitted_handle<T>(
        &self,
        owner: PhysicalStorageAccount,
        plan: RetainedAllocationPlan,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<RetainedAllocation<T>, RetainedAllocationError>
    where
        T: Send + Sync + 'static,
    {
        let charge = self.checked_retained_charge::<T>(plan)?;
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
                discard_untrusted_panic_payload(payload);
                // A builder can move nested storage into a side owner before panicking. The panic
                // payload type cannot prove that storage was released, even for `&str` or
                // `String`, so both physical admission and the caller's operation attribution
                // remain fail-closed.
                std::mem::forget(lease);
                return Err(RetainedAllocationError::BuilderPanicked {
                    charge_retained: true,
                });
            }
        };
        let payload = RetainedPayload { value, charge };
        let unverified_before = self.unverified_drop_count();
        let inner = match FinalOwnerAllocation::try_new_observed(
            payload,
            lease,
            observe_retained_payload_drop_failure,
        ) {
            Ok(inner) => inner,
            Err(error) => {
                let charge_retained = unverified_before.is_none()
                    || self.unverified_drop_count() != unverified_before;
                return Err(map_retained_final_owner_error(error, charge_retained));
            }
        };
        Ok(RetainedAllocation {
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
        self.try_retain_with_admitted_handle(self.share_after_admission(), plan, build)
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
                .map_err(|error| map_retained_final_owner_error(error, false))?;
        let bytes = outer_bytes
            .checked_add(plan.nested_bytes)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        let allocations = 1_u64
            .checked_add(plan.nested_allocations)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        Ok(AdmissionCharge::new(bytes, allocations))
    }
}

fn observe_retained_payload_drop_failure(lease: &AdmissionLease<PhysicalStorageAccount>) {
    lease.owner.record_unverified_drop();
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
/// Returning this code is an explicit recoverable-failure contract: the builder certifies that it
/// did not transfer nested storage to a side owner and that all builder-local storage was destroyed
/// while physical admission remained live. This concrete value type prevents moving an owning
/// `Vec`, `String`, `Arc`, static reference, or interior-mutable storage container through the error
/// value itself; it cannot prevent side effects, so trusted domain adapters must uphold the rest of
/// the contract. A panic supplies no such proof and always retains physical and operation charges
/// fail-closed. Domain layers reconstruct their typed error after this operation seam returns.
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

fn try_next_retained_set_id() -> Option<RetainedSetId> {
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
pub(crate) struct RetainedAllocation<T> {
    inner: FinalOwnerAllocation<RetainedPayload<T>, AdmissionLease<PhysicalStorageAccount>>,
    membership: Option<RetainedMembership>,
}

impl<T> RetainedAllocation<T> {
    fn charged_bytes(&self) -> u64 {
        self.inner.value().charge.bytes
    }

    fn charged_allocations(&self) -> u64 {
        self.inner.value().charge.allocations
    }

    fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        self.inner.lease().owner.is_same_account(account)
    }

    fn assign_membership(&mut self, membership: RetainedMembership) -> Result<(), ()> {
        if self.membership.is_some() {
            return Err(());
        }
        self.membership = Some(membership);
        Ok(())
    }

    fn membership(&self) -> Option<RetainedMembership> {
        self.membership
    }

    fn value_mut_during_construction(&mut self) -> Option<&mut T> {
        Some(&mut self.inner.value_mut()?.value)
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

fn drop_retained_allocation_before_attribution<T>(
    allocation: RetainedAllocation<T>,
    account: &PhysicalStorageAccount,
) -> bool {
    if !allocation.belongs_to(account) {
        std::mem::forget(allocation);
        return false;
    }
    let Some(before) = account.unverified_drop_count() else {
        std::mem::forget(allocation);
        return false;
    };
    match catch_unwind(AssertUnwindSafe(|| drop(allocation))) {
        Ok(()) => account.unverified_drop_count() == Some(before),
        Err(payload) => {
            discard_untrusted_panic_payload(payload);
            false
        }
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
    AllocationFailed { charge_retained: bool },
}

impl RetainedAllocationError {
    /// Whether failure containment retained the physical charge because storage release was not
    /// proven.
    pub const fn charge_retained(&self) -> bool {
        match self {
            Self::BuilderPanicked { charge_retained }
            | Self::AllocationFailed { charge_retained } => *charge_retained,
            _ => false,
        }
    }
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
                        "retained payload constructor panicked and storage release was not proven; physical charge remains retained",
                    )
                } else {
                    formatter.write_str("retained payload constructor panicked")
                }
            }
            Self::AllocationFailed { charge_retained } => {
                if *charge_retained {
                    formatter.write_str(
                        "retained shared allocation failed and payload release was not proven; physical charge remains retained",
                    )
                } else {
                    formatter.write_str("retained shared allocation failed")
                }
            }
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
fn map_retained_final_owner_error(
    error: FinalOwnerAllocationError,
    charge_retained: bool,
) -> RetainedAllocationError {
    match error {
        FinalOwnerAllocationError::AccountingOverflow => {
            RetainedAllocationError::AccountingOverflow
        }
        FinalOwnerAllocationError::AllocationFailed => {
            RetainedAllocationError::AllocationFailed { charge_retained }
        }
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

/// Clone and retained-handle work charged before sharing an inherited owner.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedHandleSharePlan<D> {
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
pub struct RetainedOperationStartPlan<P, D> {
    terminal_writer: RetainedWriterPlan<P, D>,
    terminal_account_share_work: RetainedHandleSharePlan<D>,
    fallback_plan: RetainedAllocationPlan,
    fallback_commitment: OutputCommitment<D, PublishedOutcomeEvidence>,
    snapshot_commitment: OutputCommitment<D, PublishedOutcomeEvidence>,
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
pub enum RetainedWriterError<D> {
    /// A fixed batch capacity could not be represented.
    BatchCapacity,
    /// Allocation of fixed batch storage failed.
    BatchAllocationFailed,
    /// The central operation ledger rejected the action.
    Ledger(LedgerCause<D>),
    /// The physical retained-storage account rejected the action.
    Physical(RetainedAllocationError),
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
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for RetainedWriterError<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Ledger(cause) => Some(cause),
            Self::Physical(cause) => Some(cause),
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
pub enum RetainedOperationStartError<D> {
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

#[allow(
    clippy::too_many_arguments,
    reason = "single-value bootstrap keeps storage, attribution, and commitment construction atomic"
)]
fn try_create_single_retained_part<O, P, D, Destination, T>(
    ledger: &Ledger<O, P, D>,
    account: PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    writer: RetainedWriterPlan<P, D>,
    allocation_plan: RetainedAllocationPlan,
    commitment: OutputCommitment<D, Destination>,
    build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
) -> Result<RetainedPart<T, Destination, O, P, D>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Destination: PublicationDestination,
    T: Send + Sync + 'static,
{
    let mut batch = ledger
        .try_attribution_batch(
            writer.phase,
            writer.capacity_dimension,
            writer.limits.attribution_slots,
            writer.limits.commitment_slots,
        )
        .map_err(map_batch_error)?;
    if !batch.has_attribution_slot() {
        return Err(RetainedWriterError::AttributionBatchFull);
    }
    batch
        .try_push_commitment(commitment)
        .map_err(|_| RetainedWriterError::CommitmentBatchFull)?;

    let charge = account
        .checked_retained_charge::<T>(allocation_plan)
        .map_err(RetainedWriterError::Physical)?;
    let attribution = ledger
        .reserve_operation_attribution::<Destination>(
            writer.phase,
            writer.attribution_dimension,
            charge.bytes,
        )
        .map_err(RetainedWriterError::Ledger)?;
    let allocation_owner = try_share_account_after_work(ledger, &account, handle_share_work)?;
    let allocation =
        match account.try_retain_with_admitted_handle(allocation_owner, allocation_plan, build) {
            Ok(allocation) => allocation,
            Err(cause) => {
                if cause.charge_retained() {
                    std::mem::forget(attribution);
                } else {
                    drop(attribution);
                }
                return Err(RetainedWriterError::Physical(cause));
            }
        };
    if let Err(attribution) = batch.try_push_attribution(attribution) {
        std::mem::forget(allocation);
        std::mem::forget(attribution);
        return Err(RetainedWriterError::AttributionBatchFull);
    }

    Ok(RetainedPart {
        allocation: Some(allocation),
        batch: Some(batch),
        account: Some(account),
    })
}

/// A destination-qualified retained allocation whose attribution cannot be detached.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub struct RetainedPart<T, Destination, O, P, D>
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

    fn take_batch_for_prepare(&mut self) -> AttributionReservationBatch<O, P, D, Destination> {
        match self.batch.take() {
            Some(batch) => batch,
            None => unreachable_final_owner_state(),
        }
    }

    fn into_published_part(mut self) -> PublishedPart<T> {
        PublishedPart {
            allocation: self.allocation.take(),
            account: self.account.take(),
        }
    }

    fn try_discard_allocation_before_batch_release(
        &mut self,
    ) -> Result<PhysicalStorageAccount, ()> {
        let account = match self.account.as_ref() {
            Some(account) => account,
            None => return Err(()),
        };
        let allocation = match self.allocation.take() {
            Some(allocation) => allocation,
            None => return Err(()),
        };
        if !drop_retained_allocation_before_attribution(allocation, account) {
            return Err(());
        }
        self.account.take().ok_or(())
    }

    fn fail_closed(&mut self) {
        if let Some(allocation) = self.allocation.take() {
            std::mem::forget(allocation);
        }
        if let Some(batch) = self.batch.take() {
            std::mem::forget(batch);
        }
        if let Some(account) = self.account.take() {
            std::mem::forget(account);
        }
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
        if std::thread::panicking() {
            self.fail_closed();
            return;
        }
        if let Some(allocation) = self.allocation.take() {
            let Some(account) = self.account.as_ref() else {
                std::mem::forget(allocation);
                self.fail_closed();
                return;
            };
            if !drop_retained_allocation_before_attribution(allocation, account) {
                self.fail_closed();
                return;
            }
        } else if self.batch.is_some() || self.account.is_some() {
            self.fail_closed();
            return;
        }
        drop(self.batch.take());
        drop(self.account.take());
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
    fallback: Option<RetainedPart<F, PublishedOutcomeEvidence, O, P, D>>,
    evidence: Option<RetainedPart<PublicationEvidenceCells<D>, PublishedOutcomeEvidence, O, P, D>>,
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
        RetainedPart<F, PublishedOutcomeEvidence, O, P, D>,
        RetainedPart<PublicationEvidenceCells<D>, PublishedOutcomeEvidence, O, P, D>,
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

    fn fail_closed(&mut self) {
        if let Some(fallback) = self.fallback.take() {
            std::mem::forget(fallback);
        }
        if let Some(evidence) = self.evidence.take() {
            std::mem::forget(evidence);
        }
        if let Some(batches) = self.batches.take() {
            std::mem::forget(batches);
        }
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
            self.fail_closed();
            return;
        }
        let fallback_account = match self
            .fallback
            .as_mut()
            .and_then(|part| part.try_discard_allocation_before_batch_release().ok())
        {
            Some(account) => account,
            None => {
                self.fail_closed();
                return;
            }
        };
        let evidence_account = match self
            .evidence
            .as_mut()
            .and_then(|part| part.try_discard_allocation_before_batch_release().ok())
        {
            Some(account) => account,
            None => {
                std::mem::forget(fallback_account);
                self.fail_closed();
                return;
            }
        };
        drop(self.batches.take());
        drop(fallback_account);
        drop(evidence_account);
        drop(self.fallback.take());
        drop(self.evidence.take());
    }
}

/// One destination's fixed block capacity and ledger placement.
///
/// `total_blocks` is the exact preallocated slot capacity, not a required final length. The table
/// root consumes one attribution slot in addition to every allocated member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedBlockTablePlan<P, D> {
    writer: RetainedWriterPlan<P, D>,
    total_blocks: usize,
}

impl<P: Copy, D: Copy> RetainedBlockTablePlan<P, D> {
    /// Creates a fixed table plan. Invalid relationships are rejected before opening the assembly.
    pub const fn new(writer: RetainedWriterPlan<P, D>, total_blocks: usize) -> Self {
        Self {
            writer,
            total_blocks,
        }
    }
}

/// Independent canonical and outcome-evidence plans for one retained publication.
///
/// A success plan is success-capable rather than success-forcing: control or ledger rejection may
/// atomically downgrade it to rich failure or minimal fallback. The outcome-evidence root must be
/// outcome-neutral so the same evidence graph remains valid for success and failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedPublicationPlan<P, D> {
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
struct RetainedSlotIdentity {
    set_id: RetainedSetId,
    index: usize,
    tag: TypeId,
}

/// Non-owning projection to one canonical block in a branded retained publication.
///
/// A slot is valid only for the retained set that issued it. Sharing the same physical block into a
/// later generation issues a fresh slot; the old slot is rejected by the new aggregate.
///
/// ```compile_fail
/// use baozi_operation::CanonicalRetainedSlot;
/// use std::marker::PhantomData;
///
/// struct Block;
/// struct Tag;
/// let _slot: CanonicalRetainedSlot<Block, Tag> = CanonicalRetainedSlot {
///     identity: todo!(),
///     marker: PhantomData,
/// };
/// ```
///
/// ```compile_fail
/// use baozi_operation::CanonicalRetainedSlot;
///
/// struct OldTag;
/// struct NewTag;
/// fn retag(slot: CanonicalRetainedSlot<u64, OldTag>)
///     -> CanonicalRetainedSlot<u64, NewTag>
/// {
///     slot
/// }
/// ```
#[must_use = "a retained slot is required to reference and validate its block"]
pub struct CanonicalRetainedSlot<Block, Tag> {
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

/// Non-owning projection to one outcome-evidence block in a branded retained publication.
///
/// Like a canonical slot, this identity is set-local and cannot be retagged or persisted as a
/// cross-generation identity.
#[must_use = "a retained slot is required to reference and validate its block"]
pub struct OutcomeEvidenceRetainedSlot<Block, Tag> {
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

/// Scoped constructor that couples a complete immutable block to a domain-owned tag value.
///
/// Blocks are constructed exactly once and remain immutable after placement. The table interface
/// deliberately has no replacement, retirement, or generic mutable-access operation.
#[must_use = "the emplacer must place one complete block or be returned as a builder failure"]
pub struct BlockEmplacer<'a, Block> {
    marker: PhantomData<&'a mut Block>,
}

/// Unforgeable result of placing one complete block with a value witness.
///
/// ```compile_fail
/// use baozi_operation::BlockPlacement;
/// use std::marker::PhantomData;
///
/// struct Tag;
/// let _placement: BlockPlacement<'static, u64, Tag> = BlockPlacement {
///     block: 1,
///     block_marker: PhantomData,
///     tag_marker: PhantomData,
/// };
/// ```
#[must_use = "placement must be returned to the retained table builder"]
pub struct BlockPlacement<'a, Block, Tag> {
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
    reachable: AtomicBool,
}

struct RetainedBlockTable<Block> {
    members: Vec<RetainedBlockMember<Block>>,
    slot_limit: usize,
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

/// Read-only reachability session for a success-capable canonical/evidence graph.
///
/// Validation must visit every unique slot in both tables. Repeated DAG edges are valid and count
/// once. Any lookup returning `None` invalidates the whole session even if the callback ignores it.
pub struct RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock> {
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
pub struct RetainedEvidenceClosureSession<'a, EvidenceBlock> {
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

/// Unforgeable exact-closure witness consumed by success preparation.
///
/// ```compile_fail
/// use baozi_operation::RetainedSuccessClosureProof;
/// use std::marker::PhantomData;
///
/// let _proof: RetainedSuccessClosureProof<u64, u64> = RetainedSuccessClosureProof {
///     set_id: todo!(),
///     canonical_count: 0,
///     evidence_count: 0,
///     canonical_root: todo!(),
///     evidence_root: todo!(),
///     marker: PhantomData,
/// };
/// ```
#[must_use = "the closure proof must be consumed by terminal preparation"]
pub struct RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock> {
    set_id: RetainedSetId,
    canonical_count: usize,
    evidence_count: usize,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
    marker: PhantomData<fn() -> (CanonicalBlock, EvidenceBlock)>,
}

/// Unforgeable exact-closure witness consumed by evidence-only preparation.
#[must_use = "the closure proof must be consumed by terminal preparation"]
pub struct RetainedEvidenceClosureProof<EvidenceBlock> {
    set_id: RetainedSetId,
    evidence_count: usize,
    evidence_root: RetainedSlotIdentity,
    marker: PhantomData<fn() -> EvidenceBlock>,
}

/// Failure to validate the complete reachable block closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetainedClosureValidationError {
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
    table: RetainedAllocation<RetainedBlockTable<Block>>,
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
///
/// ```compile_fail
/// use baozi_operation::{CanonicalRetainedSlot, PublishedCanonicalAggregate};
///
/// #[derive(Clone, Copy)]
/// struct Tag;
/// fn mutate(
///     aggregate: &mut PublishedCanonicalAggregate<u64>,
///     slot: CanonicalRetainedSlot<u64, Tag>,
/// ) {
///     let _ = aggregate.get_mut(slot);
/// }
/// ```
pub struct PublishedCanonicalAggregate<Block> {
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
pub struct PublishedOutcomeEvidenceAggregate<Block> {
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
pub struct RetainedPublicationOpenError<Operation, D> {
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
        && !matches!(cause, RetainedWriterError::StorageDropUnverified)
}

/// Recoverable construction failure for an operation-owned retained block table.
#[derive(Debug)]
#[non_exhaustive]
pub enum RetainedPublicationError<D> {
    /// The underlying retained writer rejected an accounted action.
    Writer(RetainedWriterError<D>),
    /// The canonical table has reached its fixed slot limit.
    CanonicalSlotLimit,
    /// The outcome-evidence table has reached its fixed slot limit.
    OutcomeEvidenceSlotLimit,
    /// A canonical action was requested from an evidence-only assembly.
    WrongOutcomeMode,
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

fn retained_publication_error_is_poisoning<D>(error: &RetainedPublicationError<D>) -> bool {
    matches!(error, RetainedPublicationError::Poisoned)
        || matches!(
            error,
            RetainedPublicationError::Writer(RetainedWriterError::StorageDropUnverified)
        )
        || matches!(
            error,
            RetainedPublicationError::Writer(RetainedWriterError::Physical(cause))
                if cause.charge_retained()
        )
}

/// Stable operation owner for two preplanned, physically accounted block tables.
#[must_use = "the assembly owns the operation, retained storage, and live attribution"]
pub struct RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    canonical_table: Option<RetainedAllocation<RetainedBlockTable<CanonicalBlock>>>,
    evidence_table: Option<RetainedAllocation<RetainedBlockTable<EvidenceBlock>>>,
    canonical_batch: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_batch: Option<AttributionReservationBatch<O, P, D, PublishedOutcomeEvidence>>,
    account: Option<PhysicalStorageAccount>,
    operation: Option<RetainedOperation<O, P, PP, U, D, F>>,
    set_id: RetainedSetId,
    canonical_plan: Option<RetainedBlockTablePlan<P, D>>,
    evidence_plan: RetainedBlockTablePlan<P, D>,
    handle_share_work: RetainedHandleSharePlan<D>,
    mode: RetainedOutcomeMode,
    poisoned: bool,
}

#[allow(clippy::too_many_arguments)]
fn try_create_retained_block_table<O, P, D, Destination, Block>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    phase: P,
    attribution_dimension: D,
    batch: &mut AttributionReservationBatch<O, P, D, Destination>,
    slot_limit: usize,
    set_id: RetainedSetId,
    membership_destination: RetainedMembershipDestination,
) -> Result<RetainedAllocation<RetainedBlockTable<Block>>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Destination: PublicationDestination,
    Block: Send + Sync + 'static,
{
    if u64::try_from(slot_limit).is_err() {
        return Err(RetainedWriterError::MembershipExhausted);
    }
    if !batch.has_attribution_slot() {
        return Err(RetainedWriterError::AttributionBatchFull);
    }
    let capacity = CapacityPlan::for_type::<RetainedBlockMember<Block>>(0, 0, slot_limit)
        .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let nested_bytes = capacity
        .envelope_delta_bytes()
        .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let table_plan = RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
        .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let charge = account
        .checked_retained_charge::<RetainedBlockTable<Block>>(table_plan)
        .map_err(RetainedWriterError::Physical)?;
    let attribution = ledger
        .reserve_operation_attribution::<Destination>(phase, attribution_dimension, charge.bytes)
        .map_err(RetainedWriterError::Ledger)?;
    let allocation_owner = try_share_account_after_work(ledger, account, handle_share_work)?;
    let mut table =
        match account.try_retain_with_admitted_handle(allocation_owner, table_plan, || {
            let mut members = Vec::new();
            if members
                .try_reserve_exact(capacity.allocator_additional())
                .is_err()
            {
                return Err(RetainedBuildFailureCode::new(1));
            }
            if capacity.finish(members.len(), members.capacity()).is_err() {
                return Err(RetainedBuildFailureCode::new(2));
            }
            Ok(RetainedBlockTable {
                qualified_capacity: members.capacity(),
                members,
                slot_limit,
            })
        }) {
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
        if drop_retained_allocation_before_attribution(table, account) {
            drop(attribution);
            return Err(RetainedWriterError::MembershipExhausted);
        }
        std::mem::forget(attribution);
        return Err(RetainedWriterError::StorageDropUnverified);
    }
    if let Err(attribution) = batch.try_push_attribution(attribution) {
        if drop_retained_allocation_before_attribution(table, account) {
            drop(attribution);
            return Err(RetainedWriterError::AttributionBatchFull);
        }
        std::mem::forget(attribution);
        return Err(RetainedWriterError::StorageDropUnverified);
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
    table: &mut RetainedAllocation<RetainedBlockTable<Block>>,
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
    let index = {
        let table = table
            .value_mut_during_construction()
            .ok_or(RetainedPublicationError::Poisoned)?;
        if table.members.len() >= table.slot_limit {
            return Err(slot_limit_error);
        }
        if table.members.len() >= table.members.capacity() {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::BatchCapacity,
            ));
        }
        table.members.len()
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
    let attribution = ledger
        .reserve_operation_attribution::<Destination>(phase, attribution_dimension, charge.bytes)
        .map_err(|cause| RetainedPublicationError::Writer(RetainedWriterError::Ledger(cause)))?;
    let allocation_owner = try_share_account_after_work(ledger, account, handle_share_work)
        .map_err(RetainedPublicationError::Writer)?;
    let mut allocation =
        match account.try_retain_with_admitted_handle(allocation_owner, plan, || {
            build(BlockEmplacer {
                marker: PhantomData,
            })
            .map(BlockPlacement::into_block)
        }) {
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
        if drop_retained_allocation_before_attribution(allocation, account) {
            drop(attribution);
            return Err(RetainedPublicationError::Poisoned);
        }
        std::mem::forget(attribution);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::StorageDropUnverified,
        ));
    }
    if let Err(attribution) = batch.try_push_attribution(attribution) {
        if drop_retained_allocation_before_attribution(allocation, account) {
            drop(attribution);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AttributionBatchFull,
            ));
        }
        std::mem::forget(attribution);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::StorageDropUnverified,
        ));
    }
    let Some(table) = table.value_mut_during_construction() else {
        std::mem::forget(allocation);
        return Err(RetainedPublicationError::Poisoned);
    };
    table.members.push(RetainedBlockMember {
        allocation,
        tag: TypeId::of::<Tag>(),
        reachable: AtomicBool::new(false),
    });
    Ok(RetainedSlotIdentity {
        set_id,
        index,
        tag: TypeId::of::<Tag>(),
    })
}

fn retained_block_table_is_exact<O, P, D, Destination, Block>(
    table: &RetainedAllocation<RetainedBlockTable<Block>>,
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
        || table.members.len() > table.slot_limit
        || table.members.len() > table.members.capacity()
    {
        return false;
    }
    let Some(attributed) = table.members.len().checked_add(1) else {
        return false;
    };
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
        let Some(next_bytes) = attributed_bytes.checked_add(member.allocation.charged_bytes())
        else {
            return false;
        };
        attributed_bytes = next_bytes;
    }
    attributed == batch.attribution_len() && batch.attributed_bytes() == Some(attributed_bytes)
}

/// Linear owner for one coordinator, one physical account, and its pre-admitted fallback.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[must_use = "the operation owns its coordinator and pre-admitted terminal fallback"]
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
        let mut fallback = try_create_single_retained_part(
            ledger,
            fallback_account,
            plan.terminal_account_share_work,
            terminal_slot_plan,
            plan.fallback_plan,
            plan.fallback_commitment,
            build_fallback,
        )?;

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
        let mut evidence = try_create_single_retained_part(
            ledger,
            evidence_account,
            plan.terminal_account_share_work,
            terminal_slot_plan,
            evidence_plan,
            plan.snapshot_commitment,
            || {
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
        )?;
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
                let fallback_account = match fallback.try_discard_allocation_before_batch_release()
                {
                    Ok(account) => account,
                    Err(()) => {
                        std::mem::forget(fallback);
                        std::mem::forget(evidence);
                        std::mem::forget(fallback_batch);
                        std::mem::forget(evidence_batch);
                        std::mem::forget(canonical_assembly);
                        std::mem::forget(evidence_assembly);
                        return Err(RetainedOperationStartError::Writer(
                            RetainedWriterError::StorageDropUnverified,
                        ));
                    }
                };
                let evidence_account = match evidence.try_discard_allocation_before_batch_release()
                {
                    Ok(account) => account,
                    Err(()) => {
                        std::mem::forget(fallback_account);
                        std::mem::forget(evidence);
                        std::mem::forget(fallback_batch);
                        std::mem::forget(evidence_batch);
                        std::mem::forget(canonical_assembly);
                        std::mem::forget(evidence_assembly);
                        return Err(RetainedOperationStartError::Writer(
                            RetainedWriterError::StorageDropUnverified,
                        ));
                    }
                };
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
        let canonical_plan = plan.canonical;
        let evidence_plan = plan.outcome_evidence;
        let handle_share_work = self.terminal_account_share_work;
        let terminal_limits = self.terminal_writer.limits;
        let plan_valid = |table: RetainedBlockTablePlan<P, D>| {
            table.total_blocks > 0
                && table.total_blocks.checked_add(1).is_some_and(|required| {
                    required <= table.writer.limits.attribution_slots
                        && required <= terminal_limits.attribution_slots
                })
                && table.writer.limits.commitment_slots <= terminal_limits.commitment_slots
        };
        if !plan_valid(evidence_plan)
            || canonical_plan.is_some_and(|canonical| !plan_valid(canonical))
        {
            return Err(RetainedPublicationOpenError {
                operation: self,
                cause: RetainedWriterError::InvalidPublicationPlan,
                recoverable: true,
            });
        }
        let Some(set_id) = try_next_retained_set_id() else {
            return Err(RetainedPublicationOpenError {
                operation: self,
                cause: RetainedWriterError::MembershipExhausted,
                recoverable: true,
            });
        };
        let account = match try_share_account_after_work(
            self.coordinator.retained_ledger(),
            self.account(),
            self.terminal_account_share_work,
        ) {
            Ok(account) => account,
            Err(cause) => {
                return Err(RetainedPublicationOpenError {
                    operation: self,
                    cause,
                    recoverable: true,
                });
            }
        };

        let (canonical_batch, canonical_table) = if let Some(canonical_plan) = canonical_plan {
            let mut batch = match self.coordinator.retained_ledger().try_attribution_batch(
                canonical_plan.writer.phase,
                canonical_plan.writer.capacity_dimension,
                canonical_plan.writer.limits.attribution_slots,
                canonical_plan.writer.limits.commitment_slots,
            ) {
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
            let table = match try_create_retained_block_table(
                self.coordinator.retained_ledger(),
                &account,
                handle_share_work,
                canonical_plan.writer.phase,
                canonical_plan.writer.attribution_dimension,
                &mut batch,
                canonical_plan.total_blocks,
                set_id,
                RetainedMembershipDestination::Canonical,
            ) {
                Ok(table) => table,
                Err(cause) => {
                    drop(batch);
                    drop(account);
                    return Err(RetainedPublicationOpenError {
                        operation: self,
                        recoverable: retained_open_failure_is_recoverable(&cause),
                        cause,
                    });
                }
            };
            (Some(batch), Some(table))
        } else {
            (None, None)
        };

        let mut evidence_batch = match self.coordinator.retained_ledger().try_attribution_batch(
            evidence_plan.writer.phase,
            evidence_plan.writer.capacity_dimension,
            evidence_plan.writer.limits.attribution_slots,
            evidence_plan.writer.limits.commitment_slots,
        ) {
            Ok(batch) => batch,
            Err(error) => {
                drop(canonical_table);
                drop(canonical_batch);
                drop(account);
                return Err(RetainedPublicationOpenError {
                    operation: self,
                    cause: map_batch_error(error),
                    recoverable: true,
                });
            }
        };
        let evidence_table = match try_create_retained_block_table(
            self.coordinator.retained_ledger(),
            &account,
            handle_share_work,
            evidence_plan.writer.phase,
            evidence_plan.writer.attribution_dimension,
            &mut evidence_batch,
            evidence_plan.total_blocks,
            set_id,
            RetainedMembershipDestination::OutcomeEvidence,
        ) {
            Ok(table) => table,
            Err(cause) => {
                drop(canonical_table);
                drop(canonical_batch);
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
            poisoned: false,
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
        if self.poisoned {
            return Err(RetainedClosureValidationError::Poisoned);
        }
        if self.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedClosureValidationError::WrongOutcomeMode);
        }
        let (Some(canonical), Some(evidence)) =
            (self.canonical_table.as_ref(), self.evidence_table.as_ref())
        else {
            self.poisoned = true;
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
        if self.poisoned {
            return Err(RetainedClosureValidationError::Poisoned);
        }
        let Some(evidence) = self.evidence_table.as_ref() else {
            self.poisoned = true;
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
        if self.poisoned {
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
                if retained_publication_error_is_poisoning(&error) {
                    self.poisoned = true;
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
        if self.poisoned {
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
                if retained_publication_error_is_poisoning(&error) {
                    self.poisoned = true;
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
        if self.poisoned {
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
        if self.poisoned {
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

    /// Validates the exact closure and attempts success publication in one consuming transaction.
    ///
    /// Final control rejection atomically downgrades to rich failure. Primary ledger rejection
    /// drops both block tables before attribution and publishes minimal fallback without retrying a
    /// second primary mode.
    pub fn try_prepare_success<CanonicalTag, EvidenceTag>(
        mut self,
        canonical_root: CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        final_progress: FinalProgress<PP, U>,
        validate: impl for<'a> FnOnce(
            &mut RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock>,
            CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    >
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let proof = match self.try_validate_success(canonical_root, evidence_root, validate) {
            Ok(proof) => proof,
            Err(cause) => {
                return self.try_prepare_terminal(RetainedTerminalRequest::MinimalFallback {
                    requested_cause: PreparationFailureCode::from_closure_validation(cause),
                });
            }
        };
        self.try_prepare_terminal(RetainedTerminalRequest::Success {
            proof,
            final_progress,
        })
    }

    /// Validates the exact evidence closure and attempts failure publication atomically.
    ///
    /// A final control cause overrides `cause`. Primary ledger rejection drops the evidence table
    /// before attribution and publishes minimal fallback without another primary attempt.
    pub fn try_prepare_rich_failure<EvidenceTag>(
        mut self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        cause: PreparationFailureCode,
        final_progress: FinalProgress<PP, U>,
        validate: impl for<'a> FnOnce(
            &mut RetainedEvidenceClosureSession<'a, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let proof = match self.try_validate_rich_failure(evidence_root, validate) {
            Ok(proof) => proof,
            Err(cause) => {
                return self.try_prepare_terminal(RetainedTerminalRequest::MinimalFallback {
                    requested_cause: PreparationFailureCode::from_closure_validation(cause),
                });
            }
        };
        self.try_prepare_terminal(RetainedTerminalRequest::RichFailure {
            proof,
            requested_cause: cause,
            final_progress,
        })
    }

    fn try_prepare_terminal(
        mut self,
        request: RetainedTerminalRequest<CanonicalBlock, EvidenceBlock, PP, U>,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
        RetainedPublicationPrepareError,
    > {
        if self.poisoned {
            self.fail_closed();
            return Err(RetainedPublicationPrepareError::Poisoned);
        }
        let exact_storage = self.has_exact_storage_graph()
            && match (self.account.as_ref(), self.operation.as_ref()) {
                (Some(account), Some(operation)) => {
                    account.is_same_account(operation.baseline.account())
                }
                _ => false,
            };
        if !exact_storage {
            self.fail_closed();
            return Err(RetainedPublicationPrepareError::InvalidStorageGraph);
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
                    self.fail_closed();
                    return Err(RetainedPublicationPrepareError::InvalidStorageGraph);
                }
                let control_cause = match self.operation.as_mut() {
                    Some(operation) => {
                        prepare_terminal_control(&mut operation.coordinator, final_progress)
                    }
                    None => {
                        self.fail_closed();
                        return Err(RetainedPublicationPrepareError::Poisoned);
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
                    self.fail_closed();
                    return Err(RetainedPublicationPrepareError::InvalidStorageGraph);
                }
                let control_cause = match self.operation.as_mut() {
                    Some(operation) => {
                        prepare_terminal_control(&mut operation.coordinator, final_progress)
                    }
                    None => {
                        self.fail_closed();
                        return Err(RetainedPublicationPrepareError::Poisoned);
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
                        self.fail_closed();
                        return Err(RetainedPublicationPrepareError::Poisoned);
                    }
                };
                RetainedTerminalMode::MinimalFallback { cause }
            }
        };

        match terminal {
            RetainedTerminalMode::Success { .. } => {}
            RetainedTerminalMode::RichFailure { .. } => {
                if let Some(canonical) = self.canonical_table.take()
                    && !drop_retained_table_before_attribution(
                        canonical,
                        match self.account.as_ref() {
                            Some(account) => account,
                            None => unreachable_final_owner_state(),
                        },
                    )
                {
                    self.fail_closed();
                    return Err(RetainedPublicationPrepareError::StorageDropUnverified);
                }
                drop(self.canonical_batch.take());
                self.mode = RetainedOutcomeMode::FailureOnly;
            }
            RetainedTerminalMode::MinimalFallback { .. } => {
                if !self.drop_tables_before_attribution() {
                    self.fail_closed();
                    return Err(RetainedPublicationPrepareError::StorageDropUnverified);
                }
            }
        }

        if self.operation.is_none() || self.evidence_batch.is_none() || self.account.is_none() {
            self.fail_closed();
            return Err(RetainedPublicationPrepareError::Poisoned);
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
        let (mut fallback, evidence, baseline_batches) = baseline.into_parts();

        if let RetainedTerminalMode::MinimalFallback { cause } = terminal {
            let proof =
                baseline_batches.into_fallback_proof(ledger, canonical_batch, Some(evidence_batch));
            return Ok(finish_retained_publication_fallback(
                proof, fallback, evidence, cause, account,
            ));
        }

        let mut baseline_batches = baseline_batches;
        let mut primary_batches = PrimaryPublicationBatches {
            canonical: canonical_batch,
            evidence: Some(evidence_batch),
        };
        let bind_result = catch_unwind(AssertUnwindSafe(|| {
            try_bind_terminal_primary(&ledger, &mut baseline_batches, &mut primary_batches)
        }));
        match bind_result {
            Err(payload) => {
                let expect_canonical = matches!(terminal, RetainedTerminalMode::Success { .. });
                let _storage_dropped = drop_primary_tables_before_attribution(
                    expect_canonical,
                    canonical_table,
                    evidence_table,
                    &account,
                );
                discard_untrusted_panic_payload(payload);
                // Binding may have updated only a prefix of cached positions. Even when both
                // tables dropped cleanly, no remaining batch can safely release or publish.
                std::mem::forget(baseline_batches);
                std::mem::forget(primary_batches);
                std::mem::forget(ledger);
                std::mem::forget(account);
                std::mem::forget(fallback);
                std::mem::forget(evidence);
                return Err(RetainedPublicationPrepareError::PrimaryPanicked);
            }
            Ok(Err(_cause)) => {
                let expect_canonical = matches!(terminal, RetainedTerminalMode::Success { .. });
                if !drop_primary_tables_before_attribution(
                    expect_canonical,
                    canonical_table,
                    evidence_table,
                    &account,
                ) {
                    std::mem::forget(baseline_batches);
                    std::mem::forget(primary_batches);
                    std::mem::forget(ledger);
                    std::mem::forget(account);
                    std::mem::forget(fallback);
                    std::mem::forget(evidence);
                    return Err(RetainedPublicationPrepareError::StorageDropUnverified);
                }
                let proof = baseline_batches.into_fallback_proof(
                    ledger,
                    primary_batches.canonical,
                    primary_batches.evidence,
                );
                return Ok(finish_retained_publication_fallback(
                    proof,
                    fallback,
                    evidence,
                    PreparationFailureCode::ledger(),
                    account,
                ));
            }
            Ok(Ok(())) => {}
        }

        let proof = finish_bound_terminal_primary(ledger, baseline_batches, primary_batches);
        let fallback_account = match fallback.try_discard_allocation_before_batch_release() {
            Ok(account) => account,
            Err(()) => {
                std::mem::forget(proof);
                std::mem::forget(fallback);
                std::mem::forget(evidence);
                std::mem::forget(canonical_table);
                std::mem::forget(evidence_table);
                std::mem::forget(account);
                return Err(RetainedPublicationPrepareError::StorageDropUnverified);
            }
        };
        freeze_terminal_evidence(proof, Some(fallback_account), &evidence);
        let payload = match terminal {
            RetainedTerminalMode::Success {
                canonical_root,
                evidence_root,
            } => {
                let canonical_table = match canonical_table {
                    Some(table) => table,
                    None => unreachable_final_owner_state(),
                };
                let evidence_table = match evidence_table {
                    Some(table) => table,
                    None => unreachable_final_owner_state(),
                };
                RetainedPublicationPayload::Success {
                    canonical: PublishedCanonicalAggregate {
                        inner: PublishedRetainedAggregate {
                            table: canonical_table,
                            set_id: self.set_id,
                        },
                    },
                    outcome_evidence: PublishedOutcomeEvidenceAggregate {
                        inner: PublishedRetainedAggregate {
                            table: evidence_table,
                            set_id: self.set_id,
                        },
                    },
                    canonical_root,
                    outcome_evidence_root: evidence_root,
                    account,
                }
            }
            RetainedTerminalMode::RichFailure {
                evidence_root,
                cause,
            } => {
                let evidence_table = match evidence_table {
                    Some(table) => table,
                    None => unreachable_final_owner_state(),
                };
                RetainedPublicationPayload::RichFailure {
                    outcome_evidence: PublishedOutcomeEvidenceAggregate {
                        inner: PublishedRetainedAggregate {
                            table: evidence_table,
                            set_id: self.set_id,
                        },
                    },
                    outcome_evidence_root: evidence_root,
                    account,
                    cause,
                }
            }
            RetainedTerminalMode::MinimalFallback { .. } => unreachable_final_owner_state(),
        };
        Ok(PreparedRetainedPublication {
            payload,
            evidence: evidence.into_published_part(),
        })
    }

    fn has_exact_storage_graph(&self) -> bool {
        if self.poisoned {
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

    fn fail_closed(&mut self) {
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
            std::mem::forget(account);
        }
        if let Some(operation) = self.operation.take() {
            std::mem::forget(operation);
        }
    }

    fn drop_tables_before_attribution(&mut self) -> bool {
        let Some(account) = self.account.as_ref() else {
            return false;
        };
        if let Some(canonical) = self.canonical_table.take()
            && !drop_retained_table_before_attribution(canonical, account)
        {
            return false;
        }
        let Some(evidence) = self.evidence_table.take() else {
            return false;
        };
        drop_retained_table_before_attribution(evidence, account)
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
        if std::thread::panicking() || !self.has_exact_storage_graph() {
            self.fail_closed();
            return;
        }
        if !self.drop_tables_before_attribution() {
            self.fail_closed();
            return;
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
    evidence: Option<AttributionReservationBatch<O, P, D, PublishedOutcomeEvidence>>,
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

fn try_bind_terminal_primary<O, P, D>(
    ledger: &Ledger<O, P, D>,
    baseline: &mut FallbackBatchBaseline<O, P, D>,
    batches: &mut PrimaryPublicationBatches<O, P, D>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    ledger.try_bind_primary(
        baseline,
        batches.canonical.as_mut(),
        batches.evidence.as_mut(),
    )
}

fn finish_bound_terminal_primary<O, P, D>(
    ledger: Ledger<O, P, D>,
    baseline: FallbackBatchBaseline<O, P, D>,
    batches: PrimaryPublicationBatches<O, P, D>,
) -> crate::ledger::TypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    ledger.finish_bound_primary(baseline, batches.canonical, batches.evidence)
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PreparationFailureClass {
    Requested,
    Control,
    ProductGraph,
    Ledger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparationFailureCode {
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
pub struct PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D> {
    payload: RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F>,
    evidence: PublishedPart<PublicationEvidenceCells<D>>,
}

impl<CanonicalBlock, EvidenceBlock, F, D>
    PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>
where
    D: Copy + Ord,
{
    /// Returns whether preparation selected the minimal pre-admitted fallback.
    #[must_use]
    pub fn uses_fallback(&self) -> bool {
        matches!(
            self.payload,
            RetainedPublicationPayload::MinimalFallback { .. }
        )
    }

    /// Moves the prepared ownership graph into its published wrapper without further fallibility.
    #[must_use = "the published owner must be consumed or deliberately dropped"]
    pub fn handoff(self) -> PublishedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D> {
        let Self { payload, evidence } = self;
        PublishedRetainedPublication { payload, evidence }
    }
}

/// Published terminal owner after the allocation-free ownership handoff.
///
/// Success and rich failure carry retained block tables; minimal fallback carries only the
/// pre-admitted fallback and frozen ledger evidence.
#[must_use = "the published owner must be split or deliberately dropped"]
pub struct PublishedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D> {
    payload: RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F>,
    evidence: PublishedPart<PublicationEvidenceCells<D>>,
}

/// Canonical owning half ready to move into a domain-owned validated Scene wrapper.
pub struct PublishedCanonicalOutput<CanonicalBlock> {
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
pub struct PublishedOutcomeEvidenceOutput<EvidenceBlock> {
    aggregate: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    root: RetainedSlotIdentity,
}

/// Evidence-only terminal owner with its frozen outer failure cause and account lineage.
pub struct PublishedRichFailureOutput<EvidenceBlock> {
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
pub struct PublishedLedgerEvidence<D> {
    evidence: PublishedPart<PublicationEvidenceCells<D>>,
}

impl<D: Copy + Ord> PublishedLedgerEvidence<D> {
    pub fn dimension(&self, dimension: D) -> Option<PublishedDimensionSnapshot<D>> {
        self.evidence.allocation().as_ref().dimension(dimension)
    }
}

/// Pre-admitted fallback owner after a failed terminal publication.
pub struct PublishedFallbackOutput<F> {
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

/// Consuming split that removes import-only generics from the canonical owner path.
#[must_use = "the split contains all terminal owners and frozen ledger evidence"]
pub enum PublishedRetainedPublicationParts<CanonicalBlock, EvidenceBlock, F, D> {
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
impl<CanonicalBlock, EvidenceBlock, F, D>
    PublishedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>
where
    D: Copy + Ord,
{
    /// Returns whether the published result selected minimal fallback.
    #[must_use]
    pub fn uses_fallback(&self) -> bool {
        matches!(
            self.payload,
            RetainedPublicationPayload::MinimalFallback { .. }
        )
    }

    /// Consumes the published wrapper and returns outcome-specific owners.
    #[must_use = "the split contains all terminal owners and frozen ledger evidence"]
    pub fn into_parts(
        self,
    ) -> PublishedRetainedPublicationParts<CanonicalBlock, EvidenceBlock, F, D> {
        let Self { payload, evidence } = self;
        let ledger_evidence = PublishedLedgerEvidence { evidence };
        match payload {
            RetainedPublicationPayload::Success {
                canonical,
                outcome_evidence,
                canonical_root,
                outcome_evidence_root,
                account,
            } => PublishedRetainedPublicationParts::Success {
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
            },
            RetainedPublicationPayload::RichFailure {
                outcome_evidence,
                outcome_evidence_root,
                account,
                cause,
            } => PublishedRetainedPublicationParts::RichFailure {
                outcome_evidence: PublishedRichFailureOutput {
                    outcome_evidence: PublishedOutcomeEvidenceOutput {
                        aggregate: outcome_evidence,
                        root: outcome_evidence_root,
                    },
                    account,
                    cause,
                },
                ledger_evidence,
            },
            RetainedPublicationPayload::MinimalFallback { part, cause } => {
                PublishedRetainedPublicationParts::MinimalFallback {
                    fallback: PublishedFallbackOutput { part, cause },
                    ledger_evidence,
                }
            }
        }
    }
}

/// Failure to establish a safe terminal publication; all uncertain accounting is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetainedPublicationPrepareError {
    /// Required terminal ownership was already consumed or poisoned.
    Poisoned,
    /// Storage, membership, account, proof, and attribution no longer form one exact graph.
    InvalidStorageGraph,
    /// Payload destruction could not prove that physical storage was released.
    StorageDropUnverified,
    /// Owner-defined ledger ordering panicked; all uncertain attribution remains retained.
    PrimaryPanicked,
}

impl fmt::Display for RetainedPublicationPrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Poisoned => formatter.write_str("retained publication preparation is poisoned"),
            Self::InvalidStorageGraph => {
                formatter.write_str("retained publication storage graph is invalid")
            }
            Self::StorageDropUnverified => formatter
                .write_str("retained storage release could not be verified before attribution"),
            Self::PrimaryPanicked => formatter.write_str(
                "retained primary ledger preparation panicked; attribution remains retained fail-closed",
            ),
        }
    }
}

impl std::error::Error for RetainedPublicationPrepareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

fn drop_primary_tables_before_attribution<CanonicalBlock, EvidenceBlock>(
    expect_canonical: bool,
    canonical: Option<RetainedAllocation<RetainedBlockTable<CanonicalBlock>>>,
    evidence: Option<RetainedAllocation<RetainedBlockTable<EvidenceBlock>>>,
    account: &PhysicalStorageAccount,
) -> bool {
    let exact_shape = canonical.is_some() == expect_canonical && evidence.is_some();
    if let Some(canonical) = canonical
        && !drop_retained_table_before_attribution(canonical, account)
    {
        if let Some(evidence) = evidence {
            std::mem::forget(evidence);
        }
        return false;
    }
    let evidence_dropped =
        evidence.is_some_and(|evidence| drop_retained_table_before_attribution(evidence, account));
    exact_shape && evidence_dropped
}

fn drop_retained_table_before_attribution<Block>(
    table: RetainedAllocation<RetainedBlockTable<Block>>,
    account: &PhysicalStorageAccount,
) -> bool {
    drop_retained_allocation_before_attribution(table, account)
}

fn freeze_terminal_evidence<O, P, D>(
    proof: crate::ledger::TypedFreezeProof<O, P, D>,
    discarded_account: Option<PhysicalStorageAccount>,
    evidence: &RetainedPart<PublicationEvidenceCells<D>, PublishedOutcomeEvidence, O, P, D>,
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

fn finish_retained_publication_fallback<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    proof: crate::ledger::TypedFreezeProof<O, P, D>,
    fallback: RetainedPart<F, PublishedOutcomeEvidence, O, P, D>,
    evidence: RetainedPart<PublicationEvidenceCells<D>, PublishedOutcomeEvidence, O, P, D>,
    cause: PreparationFailureCode,
    discarded_account: PhysicalStorageAccount,
) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
{
    freeze_terminal_evidence(proof, Some(discarded_account), &evidence);
    PreparedRetainedPublication {
        payload: RetainedPublicationPayload::MinimalFallback {
            part: fallback.into_published_part(),
            cause,
        },
        evidence: evidence.into_published_part(),
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_FINAL_OWNER_ALLOCATION: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
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
            RetainedBlockTablePlan::new(writer_plan(canonical_blocks + 1, 2), canonical_blocks),
            RetainedBlockTablePlan::new(writer_plan(evidence_blocks + 1, 2), evidence_blocks),
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

        let prepared = assembly
            .try_prepare_success(
                canonical_root,
                evidence_root,
                final_progress(),
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
            )
            .unwrap_or_else(|_| panic!("dual publication must prepare"));
        assert!(!prepared.uses_fallback());
        match prepared.handoff().into_parts() {
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

        let prepared = assembly
            .try_prepare_success(
                root,
                evidence,
                final_progress(),
                |session, root, evidence| {
                    session.canonical(root).is_some()
                        && session.outcome_evidence(evidence).is_some()
                },
            )
            .unwrap_or_else(|_| panic!("orphan rejection must prepare minimal fallback"));
        match prepared.handoff().into_parts() {
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
        let prepared = assembly
            .try_prepare_success(
                root,
                evidence,
                final_progress(),
                |session, root, evidence| {
                    session.canonical(root).is_some()
                        && session.canonical(retagged).is_some()
                        && session.outcome_evidence(evidence).is_some()
                },
            )
            .unwrap_or_else(|_| panic!("retag rejection must prepare minimal fallback"));
        match prepared.handoff().into_parts() {
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
        match prepared.handoff().into_parts() {
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
                .share_after_admission(),
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
        let prepared = assembly
            .try_prepare_rich_failure(
                evidence,
                PreparationFailureCode::domain(9),
                final_progress(),
                |session, root| session.outcome_evidence(root) == Some(&5),
            )
            .unwrap_or_else(|_| panic!("partial canonical must downgrade to rich failure"));

        assert!(OBSERVED_ACTIVE.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_OPERATION_LIVE.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_PHYSICAL_BYTES.load(Ordering::SeqCst) > 0);
        assert!(OBSERVED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst) > 0);
        match prepared.handoff().into_parts() {
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
        match assembly
            .try_prepare_rich_failure(
                evidence,
                PreparationFailureCode::domain(3),
                final_progress(),
                |session, root| session.outcome_evidence(root) == Some(&7),
            )
            .unwrap_or_else(|_| panic!("failure-only publication must prepare"))
            .handoff()
            .into_parts()
        {
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

        match assembly
            .try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |session, canonical, evidence| {
                    session.canonical(canonical) == Some(&11)
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            )
            .unwrap_or_else(|_| panic!("control rejection must produce a rich failure"))
            .handoff()
            .into_parts()
        {
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
        match assembly
            .try_prepare_rich_failure(
                evidence,
                PreparationFailureCode::domain(5),
                final_progress(),
                |session, root| session.outcome_evidence(root) == Some(&7),
            )
            .unwrap_or_else(|_| panic!("ledger rejection must retain minimal fallback"))
            .handoff()
            .into_parts()
        {
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

        let prepared = assembly
            .try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |session, canonical, evidence| {
                    session.canonical(canonical).is_some()
                        && session.outcome_evidence(evidence).is_some()
                },
            )
            .unwrap_or_else(|_| panic!("success rejection must prepare minimal fallback"));
        assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
        match prepared.handoff().into_parts() {
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
            RetainedBlockTablePlan::new(table_writer, 1),
            RetainedBlockTablePlan::new(table_writer, 1),
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

        PANIC_ON_PRIMARY_DIMENSION_ORD.store(true, Ordering::SeqCst);
        let result = assembly.try_prepare_success(
            canonical,
            evidence,
            final_progress(),
            |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence) == Some(&9)
            },
        );
        PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::PrimaryPanicked)
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

        let result = assembly.try_prepare_success(
            canonical,
            evidence,
            final_progress(),
            |session, canonical, evidence| {
                session.canonical(canonical).is_some()
                    && session.outcome_evidence(evidence).is_some()
            },
        );
        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::StorageDropUnverified)
        ));
        assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(probe.active_reservations(), before_active);
        assert_eq!(probe.current_live_bytes(), before_live);
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
        assert!(matches!(
            assembly.try_prepare_rich_failure(
                evidence,
                PreparationFailureCode::domain(2),
                final_progress(),
                |session, root| session.outcome_evidence(root) == Some(&7),
            ),
            Err(RetainedPublicationPrepareError::StorageDropUnverified)
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
        let prepared = assembly
            .try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |session, canonical, evidence| {
                    session.canonical(canonical).is_some()
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            )
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
        let published = assembly
            .try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |session, canonical, evidence| {
                    session.canonical(canonical).is_some()
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            )
            .unwrap_or_else(|_| panic!("published drop publication must prepare"))
            .handoff();
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
        let observed_account = operation.account().share_after_admission();
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
        let prepared = assembly
            .try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |session, canonical, evidence| {
                    session.canonical(canonical).is_some()
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            )
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
        let observed_account = operation.account().share_after_admission();
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
        let prepared = assembly
            .try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |session, canonical, evidence| {
                    session.canonical(canonical).is_some()
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            )
            .unwrap_or_else(|_| panic!("handoff observation publication must prepare"));
        let (canonical_identity, canonical_shares, evidence_identity, evidence_shares) =
            match &prepared.payload {
                RetainedPublicationPayload::Success {
                    canonical,
                    outcome_evidence,
                    ..
                } => (
                    canonical.inner.table.allocation_identity(),
                    canonical.inner.table.share_call_count(),
                    outcome_evidence.inner.table.allocation_identity(),
                    outcome_evidence.inner.table.share_call_count(),
                ),
                _ => panic!("handoff observation must prepare success"),
            };
        let ledger_allocation = prepared.evidence.allocation();
        let ledger_identity = ledger_allocation.allocation_identity();
        let ledger_shares = ledger_allocation.share_call_count();

        let published = prepared.handoff();
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
        assert_eq!(OBSERVED_BYTES.load(Ordering::SeqCst), 0);
        assert_eq!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst), 0);
        match &published.payload {
            RetainedPublicationPayload::Success {
                canonical,
                outcome_evidence,
                ..
            } => {
                assert_eq!(
                    canonical.inner.table.allocation_identity(),
                    canonical_identity
                );
                assert_eq!(canonical.inner.table.share_call_count(), canonical_shares);
                assert_eq!(
                    outcome_evidence.inner.table.allocation_identity(),
                    evidence_identity
                );
                assert_eq!(
                    outcome_evidence.inner.table.share_call_count(),
                    evidence_shares
                );
            }
            _ => panic!("handoff must preserve success"),
        }
        let ledger_allocation = published.evidence.allocation();
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
        let result = catch_unwind(AssertUnwindSafe(|| {
            assembly.try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |session, canonical, evidence| {
                    session.canonical(canonical) == Some(&41)
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            )
        }))
        .expect("fallback destructor panic must be contained");
        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::StorageDropUnverified)
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
        let result = catch_unwind(AssertUnwindSafe(|| {
            assembly.try_prepare_rich_failure(
                evidence,
                PreparationFailureCode::domain(5),
                final_progress(),
                |session, root| session.outcome_evidence(root).is_some(),
            )
        }))
        .expect("primary rejection payload panic must be contained");
        assert!(matches!(
            result,
            Err(RetainedPublicationPrepareError::StorageDropUnverified)
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
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed {
                charge_retained: false,
            })
        ));
        let operation = match allocation.try_into_operation() {
            Ok(operation) => operation,
            Err(_) => panic!("verified allocation failure must preserve operation ownership"),
        };
        let assembly: TestPublication<u64, u64> = operation
            .try_publication_assembly(publication_plan(1, 1))
            .unwrap_or_else(|_| panic!("retry must open a valid assembly"));
        drop(
            assembly
                .prepare_minimal_fallback()
                .unwrap_or_else(|_| panic!("retry fallback must prepare"))
                .handoff(),
        );
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
                RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed {
                    charge_retained: false,
                })
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
        let published = assembly
            .try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |session, canonical, evidence| {
                    session.canonical(canonical) == Some(&11)
                        && session.outcome_evidence(evidence) == Some(&7)
                },
            )
            .unwrap_or_else(|_| panic!("retried assembly must prepare"))
            .handoff();
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

        let prepared = assembly
            .try_prepare_success(
                canonical,
                evidence,
                final_progress(),
                |_session, _canonical, _evidence| std::panic::panic_any(vec![0_u8; 8]),
            )
            .unwrap_or_else(|_| panic!("validation panic must prepare minimal fallback"));
        match prepared.handoff().into_parts() {
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
    fn every_builder_panic_payload_keeps_its_physical_charge_fail_closed() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 16);
        let before = account.snapshot();

        let borrowed = account
            .try_retain_with::<u64>(empty_plan(), || panic!("test panic"))
            .unwrap_err();
        assert_eq!(
            borrowed,
            RetainedAllocationError::BuilderPanicked {
                charge_retained: true,
            }
        );
        let after_borrowed = account.snapshot();
        assert!(after_borrowed.live_retained_bytes() > before.live_retained_bytes());
        assert!(after_borrowed.live_retained_allocations() > before.live_retained_allocations());

        let owned = account
            .try_retain_with::<u64>(empty_plan(), || {
                std::panic::panic_any(String::from("owned test panic"))
            })
            .unwrap_err();
        assert_eq!(
            owned,
            RetainedAllocationError::BuilderPanicked {
                charge_retained: true,
            }
        );
        let after_owned = account.snapshot();
        assert!(after_owned.live_retained_bytes() > after_borrowed.live_retained_bytes());
        assert!(
            after_owned.live_retained_allocations() > after_borrowed.live_retained_allocations()
        );

        let plan = RetainedAllocationPlan::new(64, 1).unwrap();
        let opaque = account
            .try_retain_with::<u64>(plan, || std::panic::panic_any(vec![0_u8; 64]))
            .unwrap_err();
        assert_eq!(
            opaque,
            RetainedAllocationError::BuilderPanicked {
                charge_retained: true,
            }
        );
        let after_opaque = account.snapshot();
        assert!(after_opaque.live_retained_bytes() > after_owned.live_retained_bytes() + 64);
        assert!(after_opaque.live_retained_allocations() > after_owned.live_retained_allocations());
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
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
            .share_after_admission();
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
                RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked {
                    charge_retained: true,
                })
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
            Err(RetainedAllocationError::AllocationFailed {
                charge_retained: false,
            })
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
            Ok(Err(RetainedAllocationError::AllocationFailed {
                charge_retained: true,
            }))
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert!(account.snapshot().live_retained_bytes() > 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 1);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn allocation_failure_with_unverified_drop_keeps_single_part_attribution() {
        static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        let ledger = Ledger::<TestOwner, TestPhase, TestDimension>::new(
            LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap(),
        );
        let probe = ledger.test_probe();
        let before_active = probe.active_reservations();
        let before_live = probe.current_live_bytes();

        DROP_COUNT.store(0, Ordering::SeqCst);
        fail_next_final_owner_allocation();
        let error = match try_create_single_retained_part::<_, _, _, CanonicalOutput, _>(
            &ledger,
            account.share_after_admission(),
            share_work(),
            writer_plan(1, 1),
            empty_plan(),
            OutputCommitment::new(TestDimension::Bytes, 1),
            || Ok(PanickingDropProbe(&DROP_COUNT)),
        ) {
            Ok(_) => panic!("injected single-part allocation must fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed {
                charge_retained: true,
            })
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(probe.active_reservations(), before_active + 1);
        assert!(probe.current_live_bytes() > before_live);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn allocation_failure_with_unverified_drop_keeps_block_table_attribution() {
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
                RetainedAllocationError::AllocationFailed {
                    charge_retained: true,
                }
            ))
        ));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(probe.active_reservations(), before_active + 1);
        assert!(probe.current_live_bytes() > before_live);
        drop(assembly);
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
    fn foreign_account_cannot_authorize_retained_attribution_release() {
        let factory = account_factory(2);
        let first_account = account(&factory, 4_096, 8);
        let second_account = account(&factory, 4_096, 8);
        let retained = first_account
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(5_u64))
            .unwrap_or_else(|_| panic!("foreign-account probe allocation must retain"));
        let first_before = first_account.snapshot();

        assert!(!drop_retained_allocation_before_attribution(
            retained,
            &second_account
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
        let root_exports = include_str!("lib.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();
        let (production, _) = source
            .rsplit_once(&test_attribute)
            .expect("retained source contains one final test module boundary");

        assert!(production.contains("pub(crate) struct RetainedAllocation<T>"));
        for forbidden in [
            "CanonicalRetainedSlot",
            "OutcomeEvidenceRetainedSlot",
            "RetainedAllocation",
            "RetainedClosureSession",
            "RetainedEvidenceClosureSession",
            "RetainedOperation",
            "RetainedPart",
            "RetainedPublication",
        ] {
            assert!(
                !root_exports.contains(forbidden),
                "retained write authority `{forbidden}` escaped the crate root"
            );
        }

        for method in ["try_retain_with_admitted_handle", "share_after_admission"] {
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
            count_test_only_wrappers(production),
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
            "pub fn attach(",
            "pub fn detach(",
            "pub fn get_mut(",
            "pub fn into_block(",
            "pub fn replace(",
            "pub fn retag(",
            "pub fn retire(",
            "RetainedAllocationVisitor",
        ] {
            assert!(
                !production.contains(forbidden),
                "raw retained authority `{forbidden}` escaped the deep module"
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
            RetainedPayload<PublicationEvidenceCells<u8>>,
            EvidenceLease,
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
    fn second_start_share_rejection_preserves_the_borrowed_host_account() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 16);
        let policy = DimensionPolicy::reservable_bytes(3, 3, u64::MAX, u64::MAX)
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
        drop(
            assembly
                .prepare_minimal_fallback()
                .unwrap_or_else(|_| panic!("handle rejection must preserve minimal fallback"))
                .handoff(),
        );
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn single_value_bootstrap_and_primary_prepare_exclude_duplicate_tail_work() {
        let source = include_str!("retained.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();
        let (production, _) = source
            .rsplit_once(&test_attribute)
            .expect("retained source contains one final test module boundary");
        assert!(!production.contains("set: fn("));
        assert!(!production.contains("AggregateSetterPanicked"));
        assert_eq!(
            production
                .matches("fn try_create_single_retained_part")
                .count(),
            1
        );
        assert_eq!(production.matches(".try_prepare_typed_control(").count(), 1);
        assert_eq!(production.matches("ledger.try_bind_primary(").count(), 1);
        assert_eq!(
            production.matches("ledger.finish_bound_primary(").count(),
            1
        );
    }
}
