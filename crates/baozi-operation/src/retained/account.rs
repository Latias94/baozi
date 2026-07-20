use std::fmt;
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdmissionCharge {
    pub(super) bytes: u64,
    pub(super) allocations: u64,
}

impl AdmissionCharge {
    pub(super) const fn new(bytes: u64, allocations: u64) -> Self {
        Self { bytes, allocations }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CounterLimits {
    pub(super) bytes: u64,
    pub(super) allocations: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct CounterState {
    pub(super) live_bytes: u64,
    pub(super) live_allocations: u64,
    pub(super) peak_bytes: u64,
    pub(super) peak_allocations: u64,
    pub(super) total_admitted_bytes: u64,
    pub(super) total_admitted_allocations: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdmissionError {
    AccountingOverflow,
    ByteLimitExceeded { requested: u64, limit: u64 },
    AllocationLimitExceeded { requested: u64, limit: u64 },
}

pub(super) fn lock_recovering_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn try_admit(
    counters: &Mutex<CounterState>,
    limits: CounterLimits,
    charge: AdmissionCharge,
) -> Result<(), AdmissionError> {
    let mut counters = lock_recovering_poison(counters);
    try_admit_locked(&mut counters, limits, charge)
}

pub(super) fn try_admit_locked(
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

pub(super) fn release(counters: &Mutex<CounterState>, charge: AdmissionCharge) {
    let mut counters = lock_recovering_poison(counters);
    let _released = release_locked(&mut counters, charge);
}

pub(super) fn release_locked(counters: &mut CounterState, charge: AdmissionCharge) -> bool {
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

    pub(super) const fn counter_limits(self) -> CounterLimits {
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

    pub(super) const fn counter_limits(self) -> CounterLimits {
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
    pub(super) const fn from_counters(counters: CounterState) -> Self {
        Self {
            live_account_control_bytes: counters.live_bytes,
            live_accounts: counters.live_allocations,
            peak_account_control_bytes: counters.peak_bytes,
            peak_live_accounts: counters.peak_allocations,
            total_admitted_account_control_bytes: counters.total_admitted_bytes,
            total_admitted_accounts: counters.total_admitted_allocations,
        }
    }

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
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccountPoisonReason {
    /// A retained payload, backing allocation, or final owner could not prove release.
    RetainedStorageReleaseUnverified = 1,
    /// A retained builder unwound after admission and could have escaped storage.
    RetainedBuilderPanicked = 2,
    /// A coupled storage/attribution ownership graph had to be abandoned intact.
    OwnershipGraphUnverified = 3,
    /// Primary publication binding unwound after it could have mutated a prefix.
    PrimaryPublicationBindingPanicked = 4,
    /// A verified storage release found inconsistent physical accounting counters.
    CounterInvariantViolated = 5,
    /// Terminal ledger evidence could not prove a complete frozen snapshot before publication.
    TerminalEvidenceFreezeUnverified = 6,
    /// Terminal ledger evidence construction violated its prequalified catalog or capacity.
    TerminalEvidenceConstructionUnverified = 7,
}

/// Fixed, inert evidence retained for the first physical-account poison event.
///
/// This value owns no allocation or executable backend object and has no fallible destructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountPoisonEvidence {
    reason: AccountPoisonReason,
    event_ordinal: u128,
}

impl AccountPoisonEvidence {
    pub(super) const fn new(reason: AccountPoisonReason, event_ordinal: u128) -> Self {
        Self {
            reason,
            event_ordinal,
        }
    }

    /// Returns the fixed containment reason retained for the first event.
    pub const fn reason(self) -> AccountPoisonReason {
        self.reason
    }

    /// Returns the one-based ordinal of the retained event.
    pub const fn event_ordinal(self) -> u128 {
        self.event_ordinal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalStorageAccountSnapshot {
    live_retained_bytes: u64,
    live_retained_allocations: u64,
    peak_retained_bytes: u64,
    peak_retained_allocations: u64,
    total_admitted_retained_bytes: u64,
    total_admitted_retained_allocations: u64,
    poisoned: bool,
    poison_event_count: u128,
    poison_event_count_exact: bool,
    poison_event_domain_valid: bool,
    first_poison_evidence: Option<AccountPoisonEvidence>,
}

impl PhysicalStorageAccountSnapshot {
    pub(super) const fn from_parts(
        counters: CounterState,
        poisoned: bool,
        poison_event_count: u128,
        poison_event_count_exact: bool,
        poison_event_domain_valid: bool,
        first_poison_evidence: Option<AccountPoisonEvidence>,
    ) -> Self {
        Self {
            live_retained_bytes: counters.live_bytes,
            live_retained_allocations: counters.live_allocations,
            peak_retained_bytes: counters.peak_bytes,
            peak_retained_allocations: counters.peak_allocations,
            total_admitted_retained_bytes: counters.total_admitted_bytes,
            total_admitted_retained_allocations: counters.total_admitted_allocations,
            poisoned,
            poison_event_count,
            poison_event_count_exact,
            poison_event_domain_valid,
            first_poison_evidence,
        }
    }

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

    /// Returns whether this account is permanently quarantined.
    pub const fn poisoned(self) -> bool {
        self.poisoned
    }

    /// Returns the representable number of recorded unverified containment events.
    ///
    /// The value is exact while [`Self::poison_event_count_exact`] is `true`. A false exactness
    /// flag is an internal-invariant failure, not a saturating success state.
    pub const fn poison_event_count(self) -> u128 {
        self.poison_event_count
    }

    /// Returns whether the event count remains exactly representable.
    pub const fn poison_event_count_exact(self) -> bool {
        self.poison_event_count_exact
    }

    /// Returns whether every recorded event fits the account's pre-admitted containment domain.
    ///
    /// This flag is monotonic: after an unexpected event exceeds the admitted owner/operation
    /// domain, the account remains invalid even though later events continue to be counted.
    pub const fn poison_event_domain_valid(self) -> bool {
        self.poison_event_domain_valid
    }

    /// Returns the fixed evidence retained for the first poison event.
    pub const fn first_poison_evidence(self) -> Option<AccountPoisonEvidence> {
        self.first_poison_evidence
    }
}

/// Typed rejection returned when a physical account has been quarantined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountPoisoned {
    snapshot: PhysicalStorageAccountSnapshot,
}

impl AccountPoisoned {
    pub(super) const fn new(snapshot: PhysicalStorageAccountSnapshot) -> Self {
        Self { snapshot }
    }

    /// Returns the atomic account snapshot observed by the rejected transition.
    pub const fn snapshot(self) -> PhysicalStorageAccountSnapshot {
        self.snapshot
    }
}

impl fmt::Display for AccountPoisoned {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "physical storage account is poisoned after {} containment event(s)",
            self.snapshot.poison_event_count
        )
    }
}

impl std::error::Error for AccountPoisoned {}

pub(super) fn quarantined_account_reason(account: AccountPoisoned) -> AccountPoisonReason {
    account.snapshot().first_poison_evidence().map_or(
        AccountPoisonReason::OwnershipGraphUnverified,
        AccountPoisonEvidence::reason,
    )
}

/// Fatal finalization containment after the account became quarantined.
///
/// This error certifies that no exact prepared publication, minimal fallback, or handoff owner was
/// produced. All uncertain retained ownership remains contained fail-closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationContainmentError {
    account: AccountPoisoned,
    reason: AccountPoisonReason,
}

impl FinalizationContainmentError {
    pub(super) const fn new(account: AccountPoisoned, reason: AccountPoisonReason) -> Self {
        Self { account, reason }
    }

    pub(super) fn from_quarantine(account: AccountPoisoned) -> Self {
        let reason = quarantined_account_reason(account);
        Self::new(account, reason)
    }

    /// Returns the fixed account quarantine evidence observed at final eligibility.
    pub const fn account_poisoned(self) -> AccountPoisoned {
        self.account
    }

    /// Returns the event class that forced this finalization to fail closed.
    pub const fn reason(self) -> AccountPoisonReason {
        self.reason
    }
}

impl fmt::Display for FinalizationContainmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("retained publication finalization was contained after account quarantine")
    }
}

impl std::error::Error for FinalizationContainmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.account)
    }
}
