use super::startup::ReadyStartupPhysicalSplitAuthority;
use super::*;

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

pub(super) struct AccountState {
    pub(super) limits: PhysicalStorageAccountLimits,
    pub(super) linear: Mutex<AccountLinearState>,
}

type FactoryControlLease = AdmissionLease<Arc<FactoryState>>;

pub(super) struct AccountControlInner {
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
    pub(super) fn value(&self) -> &AccountState {
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
pub(super) struct AccountLinearState {
    pub(super) counters: CounterState,
    pub(super) admitted_retained_owners: u64,
    pub(super) admitted_account_actions: u64,
    poison: AccountPoisonState,
}

impl AccountLinearState {
    pub(super) fn snapshot(self) -> PhysicalStorageAccountSnapshot {
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

    pub(super) fn poisoned_error(&self) -> Option<AccountPoisoned> {
        matches!(self.poison, AccountPoisonState::Poisoned { .. })
            .then(|| AccountPoisoned::new((*self).snapshot()))
    }

    pub(super) fn record_poison(&mut self, reason: AccountPoisonReason) -> AccountPoisoned {
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
pub(super) struct PendingAccountActionAdmission<'account> {
    pub(super) account: &'account PhysicalStorageAccount,
}

/// Single-use proof that final publication eligibility linearized before account poison.
///
/// The proof is minted only while holding the account's linear-state lock. A later poison event is
/// therefore ordered after this capability and cannot invalidate the already admitted pure
/// preparation tail.
#[must_use = "publication eligibility must be consumed by the exact freeze handoff"]
pub(super) struct PublicationEligibility {
    pub(super) private: (),
}

/// Holds the account linearization lock while a caller performs non-mutating table/source
/// preflight.  The action admission is minted only after that preflight succeeds, so invalid
/// structure cannot consume account admission-domain evidence and a concurrent poison cannot slip
/// between the health check and the action transition.
pub(super) struct AccountPreflightGuard<'account> {
    account: &'account PhysicalStorageAccount,
    state: MutexGuard<'account, AccountLinearState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AccountActionAdmissionError {
    AccountPoisoned(AccountPoisoned),
    AccountingOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicationOpenAdmissionError {
    Account(AccountActionAdmissionError),
    MembershipExhausted,
}

impl PendingAccountActionAdmission<'_> {
    pub(super) fn account(&self) -> &PhysicalStorageAccount {
        self.account
    }

    pub(super) fn run<T>(self, action: impl FnOnce(&PhysicalStorageAccount) -> T) -> T {
        action(self.account)
    }

    pub(super) fn release(self) {}

    pub(super) fn complete(self) {
        let Self { account: _ } = self;
    }
}

impl<'account> AccountPreflightGuard<'account> {
    pub(super) fn admit_action(
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

pub(super) struct PendingRetainedAdmission<'account> {
    pub(super) account: &'account PhysicalStorageAccount,
    pub(super) charge: AdmissionCharge,
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
pub(super) enum PendingRetainedRollback {
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
pub(super) struct RetainedAdmissionPairPlan {
    pub(super) first: AdmissionCharge,
    pub(super) second: AdmissionCharge,
    pub(super) aggregate: AdmissionCharge,
}

impl RetainedAdmissionPairPlan {
    #[cfg(test)]
    pub(super) fn try_new(
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

pub(super) struct PendingStartRetainedAdmissions<'account> {
    account: &'account PhysicalStorageAccount,
    pub(super) plan: RetainedAdmissionPairPlan,
    state: PendingStartRetainedAdmissionState,
    containment_owner: Option<PhysicalStorageAccount>,
}

enum PendingStartRetainedAdmissionState {
    Pending,
    Transferred,
    RolledBack,
    Contained(RetainedLeaseGuard),
}

pub(super) struct StartupAdmissionGuard<'account> {
    pub(super) account: &'account PhysicalStorageAccount,
    pub(super) admissions: Option<PendingStartRetainedAdmissions<'account>>,
    pub(super) evidence_account: Option<PhysicalStorageAccount>,
    pub(super) fallback_lease_owner: Option<PhysicalStorageAccount>,
    pub(super) evidence_lease_owner: Option<PhysicalStorageAccount>,
}

/// Test-only proof for physical-account fixtures that bypass operation-ledger work.
///
/// Production writers use `RetainedShareWorkPermit`, which admits Attempted separately and records
/// Consumed only after the physical clone and handle construction complete.
#[cfg(test)]
pub(super) struct AdmittedRetainedShareWork {
    pub(super) private: (),
}

pub(super) enum RetainedShareAdmissionError<D> {
    Ledger(LedgerCause<D>),
    Physical(RetainedAllocationError),
}

pub(super) struct StartupAccountOwners {
    pub(super) fallback_account: PhysicalStorageAccount,
    pub(super) evidence_account: PhysicalStorageAccount,
    pub(super) fallback_lease_owner: PhysicalStorageAccount,
    pub(super) evidence_lease_owner: PhysicalStorageAccount,
}

#[cfg(test)]
impl AdmittedRetainedShareWork {
    pub(super) fn consume_account(
        self,
        account: &PhysicalStorageAccount,
    ) -> PhysicalStorageAccount {
        let Self { private: () } = self;
        PhysicalStorageAccount {
            inner: account.inner.share(),
        }
    }
}

impl<'account> StartupAdmissionGuard<'account> {
    pub(super) fn from_precommitted(
        account: &'account PhysicalStorageAccount,
        plan: RetainedAdmissionPairPlan,
        owners: StartupAccountOwners,
        _authority: ReadyStartupPhysicalSplitAuthority,
    ) -> Self {
        let StartupAccountOwners {
            fallback_account,
            evidence_account,
            fallback_lease_owner,
            evidence_lease_owner,
        } = owners;
        Self {
            account,
            admissions: Some(PendingStartRetainedAdmissions {
                account,
                plan,
                state: PendingStartRetainedAdmissionState::Pending,
                containment_owner: Some(fallback_account),
            }),
            evidence_account: Some(evidence_account),
            fallback_lease_owner: Some(fallback_lease_owner),
            evidence_lease_owner: Some(evidence_lease_owner),
        }
    }

    #[allow(
        dead_code,
        reason = "the Ready guard keeps an explicit recoverable exit for pre-handoff failures"
    )]
    pub(super) fn rollback(
        mut self,
        outer_reason: Option<AccountPoisonReason>,
    ) -> PendingRetainedRollback {
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
        let _ = self
            .account
            .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        std::mem::forget(admissions);
        if let Some(account) = self.evidence_account.take() {
            std::mem::forget(account);
        }
        if let Some(owner) = self.fallback_lease_owner.take() {
            std::mem::forget(owner);
        }
        if let Some(owner) = self.evidence_lease_owner.take() {
            std::mem::forget(owner);
        }
    }
}

impl<'account> PendingStartRetainedAdmissions<'account> {
    #[cfg(test)]
    pub(super) fn try_admit_for_test(
        account: &'account PhysicalStorageAccount,
        plan: RetainedAdmissionPairPlan,
    ) -> Result<Self, RetainedAllocationError> {
        let containment_owner = PhysicalStorageAccount {
            inner: account.inner.share(),
        };
        let mut state = lock_recovering_poison(&account.inner.value().linear);
        if let Some(error) = state.poisoned_error() {
            return Err(RetainedAllocationError::AccountPoisoned(error));
        }
        let admitted_retained_owners = state
            .admitted_retained_owners
            .checked_add(2)
            .ok_or(RetainedAllocationError::AccountingOverflow)?;
        let limits = account.limits().counter_limits();
        try_admit_locked(&mut state.counters, limits, plan.aggregate)
            .map_err(map_retained_admission_error)?;
        state.admitted_retained_owners = admitted_retained_owners;
        drop(state);
        Ok(Self {
            account,
            plan,
            state: PendingStartRetainedAdmissionState::Pending,
            containment_owner: Some(containment_owner),
        })
    }

    pub(super) fn into_parts(
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

    pub(super) fn transfer_to_containment(mut self) -> RetainedLeaseGuard {
        let owner = self.containment_owner.take();
        self.state = PendingStartRetainedAdmissionState::Transferred;
        RetainedLeaseGuard {
            lease: owner.map(|owner| AdmissionLease::new(owner, self.plan.aggregate)),
        }
    }

    pub(super) fn rollback_with_fault(
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
    pub(super) const fn is_pending(&self) -> bool {
        matches!(&self.state, PendingRetainedAdmissionState::Pending)
    }

    pub(super) fn take_lease_owner(&mut self) -> PhysicalStorageAccount {
        if !self.is_pending() {
            unreachable_final_owner_state();
        }
        match self.lease_owner.take() {
            Some(owner) => owner,
            None => unreachable_final_owner_state(),
        }
    }

    pub(super) fn commit_lease(&mut self) -> AdmissionLease<PhysicalStorageAccount> {
        let owner = self.take_lease_owner();
        self.state = PendingRetainedAdmissionState::Transferred;
        AdmissionLease::new(owner, self.charge)
    }

    pub(super) fn rollback(self) -> bool {
        self.rollback_with_fault(None).charge_released()
    }

    pub(super) fn rollback_with_fault(
        mut self,
        outer_reason: Option<AccountPoisonReason>,
    ) -> PendingRetainedRollback {
        self.rollback_in_place(outer_reason)
    }

    pub(super) fn rollback_in_place(
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
    pub(super) fn rollback_startup_in_place(
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

    pub(super) fn mark_released_by_aggregate_rollback(&mut self) {
        if self.is_pending() {
            self.state = PendingRetainedAdmissionState::RolledBack;
        }
    }

    pub(super) fn transfer_to_aggregate_containment(&mut self) -> PhysicalStorageAccount {
        if self.is_pending() {
            let owner = self.take_lease_owner();
            self.state = PendingRetainedAdmissionState::Transferred;
            return owner;
        }
        unreachable_final_owner_state();
    }

    pub(super) fn contain_with_lease_owner(&mut self) -> RetainedLeaseGuard {
        if !self.is_pending() {
            unreachable_final_owner_state();
        }
        let owner = self.take_lease_owner();
        self.state = PendingRetainedAdmissionState::Transferred;
        RetainedLeaseGuard {
            lease: Some(AdmissionLease::new(owner, self.charge)),
        }
    }

    pub(super) fn take_contained_lease(&mut self) -> Option<RetainedLeaseGuard> {
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
    pub(super) inner: FinalOwnerAllocation<AccountControlInner>,
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
    pub(crate) fn test_record_poison(&self, reason: AccountPoisonReason) -> AccountPoisoned {
        self.record_poison(reason)
    }

    pub(super) fn record_poison(&self, reason: AccountPoisonReason) -> AccountPoisoned {
        lock_recovering_poison(&self.inner.value().linear).record_poison(reason)
    }

    pub(super) fn try_admit_account_action(
        &self,
    ) -> Result<PendingAccountActionAdmission<'_>, AccountActionAdmissionError> {
        self.try_begin_account_preflight()
            .map_err(AccountActionAdmissionError::AccountPoisoned)?
            .admit_action()
    }

    pub(super) fn try_begin_account_preflight(
        &self,
    ) -> Result<AccountPreflightGuard<'_>, AccountPoisoned> {
        let state = lock_recovering_poison(&self.inner.value().linear);
        if let Some(error) = state.poisoned_error() {
            return Err(error);
        }
        Ok(AccountPreflightGuard {
            account: self,
            state,
        })
    }

    pub(super) fn try_admit_publication_open(
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

    pub(super) fn try_publication_eligibility(
        &self,
    ) -> Result<PublicationEligibility, AccountPoisoned> {
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

    #[cfg(test)]
    pub(super) fn try_admit_retained(
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
    pub(super) fn try_admit_retained_after_action<O, P, D>(
        &self,
        action: PendingAccountActionAdmission<'_>,
        charge: AdmissionCharge,
        admitted_share_work: RetainedShareWorkPermit<'_, O, P, D>,
    ) -> Result<PendingRetainedAdmission<'_>, RetainedShareAdmissionError<D>>
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord + fmt::Debug,
    {
        if !action.account().is_same_account(self) {
            let poisoned = self.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedShareAdmissionError::Physical(
                RetainedAllocationError::AccountPoisoned(poisoned),
            ));
        }
        let mut state = lock_recovering_poison(&self.inner.value().linear);
        let admitted_retained_owners = state.admitted_retained_owners.checked_add(1).ok_or(
            RetainedShareAdmissionError::Physical(RetainedAllocationError::AccountingOverflow),
        )?;
        let limits = self.limits().counter_limits();
        try_admit_locked(&mut state.counters, limits, charge)
            .map_err(map_retained_admission_error)
            .map_err(RetainedShareAdmissionError::Physical)?;
        state.admitted_retained_owners = admitted_retained_owners;
        drop(state);
        let lease_owner = match admitted_share_work.try_share_account(self) {
            Ok(owner) => owner,
            Err(failure) => {
                let (owner, cause) = failure.into_parts();
                let pending = PendingRetainedAdmission {
                    account: self,
                    charge,
                    state: PendingRetainedAdmissionState::Pending,
                    lease_owner: Some(owner),
                };
                return Err(match pending.rollback_with_fault(None) {
                    PendingRetainedRollback::Clean => RetainedShareAdmissionError::Ledger(cause),
                    PendingRetainedRollback::FaultRecorded(poisoned) => {
                        RetainedShareAdmissionError::Physical(
                            RetainedAllocationError::AccountPoisoned(poisoned),
                        )
                    }
                    PendingRetainedRollback::ChargeReleaseUnverified(_) => {
                        RetainedShareAdmissionError::Physical(
                            RetainedAllocationError::ChargeReleaseUnverified,
                        )
                    }
                    PendingRetainedRollback::ChargeRetainedAfterPoison(poisoned) => {
                        RetainedShareAdmissionError::Physical(
                            RetainedAllocationError::AccountPoisonedWithRetainedCharge(poisoned),
                        )
                    }
                });
            }
        };
        action.complete();
        Ok(PendingRetainedAdmission {
            account: self,
            charge,
            state: PendingRetainedAdmissionState::Pending,
            lease_owner: Some(lease_owner),
        })
    }

    pub(super) fn release_retained_charge(
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

    pub(super) fn rollback_pending_retained_charge(
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
    pub(super) fn share_after_test_admission(&self) -> Result<Self, RetainedAllocationError> {
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
    pub(super) fn share_call_count(&self) -> usize {
        self.inner.share_call_count()
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
