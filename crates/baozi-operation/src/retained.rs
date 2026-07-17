//! Fallible physical storage accounting for retained operation allocations.

use crate::control::discard_untrusted_panic_payload;
use std::alloc::Layout;
use std::borrow::Borrow;
use std::fmt;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicUsize;
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

struct FinalOwnerInner<T, L> {
    value: T,
    lease: L,
}

/// The only final-owner extraction backend for retained storage and account control.
struct FinalOwnerAllocation<T, L> {
    shared: Option<Arc<FinalOwnerInner<T, L>>>,
}

impl<T, L> FinalOwnerAllocation<T, L> {
    fn try_new(value: T, lease: L) -> Result<Self, FinalOwnerAllocationError> {
        let inner = FinalOwnerInner { value, lease };

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

    fn share(&self) -> Self {
        let shared = match self.shared.as_ref() {
            Some(shared) => Arc::clone(shared),
            None => unreachable_final_owner_state(),
        };
        Self {
            shared: Some(shared),
        }
    }

    fn value(&self) -> &T {
        match self.shared.as_ref() {
            Some(shared) => &shared.value,
            None => unreachable_final_owner_state(),
        }
    }

    fn lease(&self) -> &L {
        match self.shared.as_ref() {
            Some(shared) => &shared.lease,
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
        // value is returned. Keep the explicit payload-then-lease order at this sole boundary.
        let FinalOwnerInner { value, lease } = UniqueArc::into_inner(unique);
        drop(value);
        drop(lease);
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

    /// Shares the opaque account handle after the owning domain admits handle and branch work.
    ///
    /// This Internal/backend-tier method does not perform operation attribution itself. Canonical
    /// callers may invoke it only behind their operation-owned allocation guard.
    pub fn share_after_admission(&self) -> Self {
        Self {
            inner: self.inner.share(),
        }
    }

    /// Admits one retained allocation before running its constructing closure or entering the shared
    /// allocator.
    ///
    /// This is an Internal/backend-tier physical-ownership seam. It grants no canonical Scene or
    /// publication authority. Baozi domain guards must additionally admit operation attribution and
    /// verify the builder's nested envelope before calling it. Canonical callers also restrict `T`
    /// to audited ownership headers with non-panicking destructors; safe Rust cannot infer that
    /// property for an arbitrary public generic type.
    pub fn try_retain_with<T>(
        &self,
        plan: RetainedAllocationPlan,
        build: impl FnOnce() -> Result<T, RetainedBuildFailureCode>,
    ) -> Result<RetainedAllocation<T>, RetainedAllocationError>
    where
        T: Send + Sync + 'static,
    {
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
        let charge = AdmissionCharge::new(bytes, allocations);
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

    #[cfg(test)]
    fn share_after_test_admission(&self) -> Self {
        Self {
            inner: self.inner.share(),
        }
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        self.inner.ptr_eq(&other.inner)
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

fn map_retained_final_owner_error(error: FinalOwnerAllocationError) -> RetainedAllocationError {
    match error {
        FinalOwnerAllocationError::AccountingOverflow => {
            RetainedAllocationError::AccountingOverflow
        }
        FinalOwnerAllocationError::AllocationFailed => RetainedAllocationError::AllocationFailed,
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc as StdArc, Barrier};
    use std::thread;

    const BUILD_FAILED: RetainedBuildFailureCode = RetainedBuildFailureCode::new(1);

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
        struct BuilderLocalStorage {
            account: PhysicalStorageAccount,
            observed_live_bytes: StdArc<AtomicU64>,
            _bytes: Vec<u8>,
        }

        impl Drop for BuilderLocalStorage {
            fn drop(&mut self) {
                self.observed_live_bytes.store(
                    self.account.snapshot().live_retained_bytes(),
                    Ordering::SeqCst,
                );
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let observed = StdArc::new(AtomicU64::new(0));
        let plan = RetainedAllocationPlan::new(64, 1).unwrap();
        let error = account
            .try_retain_with::<u64>(plan, || {
                let _storage = BuilderLocalStorage {
                    account: account.share_after_admission(),
                    observed_live_bytes: StdArc::clone(&observed),
                    _bytes: vec![0; 64],
                };
                Err(BUILD_FAILED)
            })
            .unwrap_err();

        assert_eq!(error, RetainedAllocationError::Build(BUILD_FAILED));
        assert!(observed.load(Ordering::SeqCst) > 64);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn known_panic_payload_is_destroyed_before_charge_release() {
        let factory = account_factory(1);
        let account = account(&factory, 1_024, 8);

        let error = account
            .try_retain_with::<u64>(empty_plan(), || panic!("test panic"))
            .unwrap_err();
        assert_eq!(
            error,
            RetainedAllocationError::BuilderPanicked {
                charge_retained: false,
            }
        );
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
        struct DropProbe {
            account: PhysicalStorageAccount,
            observed_live_bytes: StdArc<AtomicU64>,
        }

        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.observed_live_bytes.store(
                    self.account.snapshot().live_retained_bytes(),
                    Ordering::SeqCst,
                );
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let observed = StdArc::new(AtomicU64::new(0));
        fail_next_final_owner_allocation();
        let result = account.try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(DropProbe {
                account: account.share_after_admission(),
                observed_live_bytes: StdArc::clone(&observed),
            })
        });

        assert!(matches!(
            result,
            Err(RetainedAllocationError::AllocationFailed)
        ));
        assert!(observed.load(Ordering::SeqCst) > 0);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }

    #[test]
    fn private_shares_charge_once_and_release_only_after_the_final_owner() {
        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let first = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(String::from("payload"))
            })
            .unwrap();
        let charged = first.charged_bytes();
        let second = first.share_after_test_admission();

        assert!(first.ptr_eq(&second));
        assert_eq!(account.snapshot().live_retained_bytes(), charged);
        drop(first);
        assert_eq!(account.snapshot().live_retained_bytes(), charged);
        drop(second);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }

    #[test]
    fn concurrent_shared_drops_select_exactly_one_final_owner() {
        struct DropCount(StdArc<AtomicU64>);

        impl Drop for DropCount {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let drops = StdArc::new(AtomicU64::new(0));
        let first = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(DropCount(StdArc::clone(&drops)))
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
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn payload_drop_barrier_keeps_headroom_unavailable_until_cleanup_finishes() {
        struct BlockingDrop {
            entered: StdArc<Barrier>,
            release: StdArc<Barrier>,
        }

        impl Drop for BlockingDrop {
            fn drop(&mut self) {
                self.entered.wait();
                self.release.wait();
            }
        }

        type Lease = AdmissionLease<PhysicalStorageAccount>;
        let outer =
            FinalOwnerAllocation::<RetainedPayload<BlockingDrop>, Lease>::checked_layout_bytes()
                .unwrap();
        let factory = account_factory(1);
        let account = account(&factory, outer, 1);
        let entered = StdArc::new(Barrier::new(2));
        let release_drop = StdArc::new(Barrier::new(2));
        let retained = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(BlockingDrop {
                    entered: StdArc::clone(&entered),
                    release: StdArc::clone(&release_drop),
                })
            })
            .unwrap();
        let drop_thread = thread::spawn(move || drop(retained));

        entered.wait();
        assert!(matches!(
            account.try_retain_with::<BlockingDrop>(empty_plan(), || {
                panic!("builder must not run before physical admission")
            }),
            Err(RetainedAllocationError::ByteLimitExceeded { .. })
                | Err(RetainedAllocationError::AllocationLimitExceeded { .. })
        ));
        release_drop.wait();
        drop_thread.join().unwrap();
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        let replacement = account
            .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(3_u64))
            .expect("headroom becomes available after payload cleanup and lease release");
        drop(replacement);
    }

    #[test]
    fn payload_drop_observes_charge_and_factory_outlives_public_account_handle() {
        struct DropProbe {
            account: PhysicalStorageAccount,
            observed_live_bytes: StdArc<AtomicU64>,
        }

        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.observed_live_bytes.store(
                    self.account.snapshot().live_retained_bytes(),
                    Ordering::SeqCst,
                );
            }
        }

        let factory = account_factory(1);
        let account = account(&factory, 4_096, 8);
        let observed = StdArc::new(AtomicU64::new(0));
        let retained = account
            .try_retain_with(empty_plan(), || {
                Ok::<_, RetainedBuildFailureCode>(DropProbe {
                    account: account.share_after_admission(),
                    observed_live_bytes: StdArc::clone(&observed),
                })
            })
            .unwrap();
        let charged = retained.charged_bytes();

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
        drop(retained);
        assert_eq!(observed.load(Ordering::SeqCst), charged);
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
    fn public_handles_are_send_and_sync_for_thread_safe_payloads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PhysicalStorageAccountFactory>();
        assert_send_sync::<PhysicalStorageAccount>();
        assert_send_sync::<RetainedAllocation<u64>>();
    }
}
