use super::account::AccountPoisonReason;
use super::final_owner::unreachable_final_owner_state;
use super::{
    AccountPoisonEventWitness, FinalOwnerReleaseOutcome, PhysicalStorageAccount,
    PolicyRetainedAllocation, RetainedAllocation, RetainedPayload, RetainedValuePolicy,
    RetainedWriterError, TerminalLedgerEvidence, record_account_poison_event,
};
use crate::OperationOwner;
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{
    AttributionReservationBatch, CanonicalOutput, FallbackBatchBaseline, Ledger,
    OperationAttributionReservation, OutcomeEvidenceDestination, PublicationDestination,
};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainedStorageDropOutcome {
    Clean,
    StorageReleaseUnverified(AccountPoisonEventWitness),
    ChargeReleaseUnverified(AccountPoisonEventWitness),
    StillSharedOwnershipFault(AccountPoisonEventWitness),
    ForeignOwnershipFault(AccountPoisonEventWitness),
    MissingFaultAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RetainedStorageDropSeverity {
    Clean,
    Charge,
    Ownership,
    Storage,
}

impl RetainedStorageDropOutcome {
    pub(super) const fn severity(self) -> RetainedStorageDropSeverity {
        match self {
            Self::Clean => RetainedStorageDropSeverity::Clean,
            Self::ChargeReleaseUnverified(_) => RetainedStorageDropSeverity::Charge,
            Self::StillSharedOwnershipFault(_)
            | Self::ForeignOwnershipFault(_)
            | Self::MissingFaultAnchor => RetainedStorageDropSeverity::Ownership,
            Self::StorageReleaseUnverified(_) => RetainedStorageDropSeverity::Storage,
        }
    }

    pub(super) const fn witness(self) -> Option<AccountPoisonEventWitness> {
        match self {
            Self::StorageReleaseUnverified(witness)
            | Self::ChargeReleaseUnverified(witness)
            | Self::StillSharedOwnershipFault(witness)
            | Self::ForeignOwnershipFault(witness) => Some(witness),
            Self::Clean | Self::MissingFaultAnchor => None,
        }
    }

    pub(super) fn join(self, other: Self) -> Self {
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

pub(super) fn join_pre_attribution_cleanup_cause<D>(
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

pub(super) fn finish_pre_attribution_cleanup<O, P, D, Destination>(
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

pub(super) fn finish_pre_attribution_batch_cleanup<O, P, D, Destination>(
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

pub(super) fn map_inherited_cleanup_error<D>(
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

pub(super) fn drop_retained_allocation_before_attribution<T, P>(
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

/// A destination-qualified retained allocation whose attribution cannot be detached.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(super) struct RetainedPart<T, Destination, O, P, D>
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
pub(super) enum RetainedPartCleanupError {
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

pub(super) struct DetachedRetainedPart<T> {
    allocation: Option<RetainedAllocation<T>>,
    account: Option<PhysicalStorageAccount>,
}

impl<T> DetachedRetainedPart<T> {
    pub(super) const fn empty() -> Self {
        Self {
            allocation: None,
            account: None,
        }
    }

    pub(super) fn allocation(&self) -> &RetainedAllocation<T> {
        match self.allocation.as_ref() {
            Some(allocation) => allocation,
            None => unreachable_final_owner_state(),
        }
    }

    pub(super) fn account(&self) -> &PhysicalStorageAccount {
        match self.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }

    pub(super) fn has_exact_owner(&self) -> bool {
        matches!(
            (self.allocation.as_ref(), self.account.as_ref()),
            (Some(allocation), Some(account)) if allocation.belongs_to(account)
        )
    }

    pub(super) fn is_empty(&self) -> bool {
        self.allocation.is_none() && self.account.is_none()
    }

    pub(super) fn account_anchor(&self) -> Option<&PhysicalStorageAccount> {
        self.account.as_ref()
    }

    pub(super) fn allocation_anchor(&self) -> Option<&PhysicalStorageAccount> {
        self.allocation.as_ref().map(RetainedAllocation::account)
    }

    pub(super) fn try_discard_allocation(
        &mut self,
    ) -> Result<PhysicalStorageAccount, RetainedPartCleanupError> {
        try_discard_retained_owner(&mut self.allocation, &mut self.account, false)
    }

    pub(super) fn record_forgotten_physical_edge(&self) {
        if let Some(allocation) = self.allocation.as_ref() {
            let _ = allocation
                .account()
                .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
        }
    }

    pub(super) fn fail_closed_after_recorded_poison(&mut self) {
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
    pub(super) fn from_complete(
        allocation: RetainedAllocation<T>,
        batch: AttributionReservationBatch<O, P, D, Destination>,
        account: PhysicalStorageAccount,
    ) -> Self {
        Self {
            allocation: Some(allocation),
            batch: Some(batch),
            account: Some(account),
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

    pub(super) fn try_discard_before_batch_release(
        &mut self,
        defer_ownership_fault: bool,
    ) -> Result<(), RetainedPartCleanupError> {
        match self.try_discard_allocation_before_batch_release_with_policy(defer_ownership_fault) {
            Ok(account) => {
                drop(self.batch.take());
                drop(account);
                Ok(())
            }
            Err(cause) => {
                self.fail_closed_after_recorded_poison();
                Err(cause)
            }
        }
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

    pub(super) fn contain_after_recorded_poison(&mut self) {
        self.record_forgotten_physical_edges();
        self.fail_closed_after_recorded_poison();
    }

    #[cfg(test)]
    pub(super) fn test_drop_account_anchor(&mut self) {
        drop(self.account.take());
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
pub(super) struct PublishedPart<T> {
    allocation: Option<RetainedAllocation<T>>,
    account: Option<PhysicalStorageAccount>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<T> PublishedPart<T> {
    /// Moves an owner whose exact shape was established by a typed terminal state before freeze.
    pub(super) fn from_exact_owner(mut owner: DetachedRetainedPart<T>) -> Self {
        Self {
            allocation: owner.allocation.take(),
            account: owner.account.take(),
        }
    }

    pub(super) fn allocation(&self) -> &RetainedAllocation<T> {
        match self.allocation.as_ref() {
            Some(allocation) => allocation,
            None => unreachable_final_owner_state(),
        }
    }

    pub(super) fn account(&self) -> &PhysicalStorageAccount {
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
pub(super) struct FallbackPublicationBaseline<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fallback: Option<DetachedRetainedPart<F>>,
    evidence: Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>,
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
    pub(super) fn account(&self) -> &PhysicalStorageAccount {
        match self.evidence.as_ref() {
            Some(evidence) => evidence.account(),
            None => unreachable_final_owner_state(),
        }
    }

    pub(super) fn with_batches<R>(
        &self,
        inspect: impl FnOnce(&FallbackBatchBaseline<O, P, D>) -> R,
    ) -> Option<R> {
        self.batches.as_ref().map(inspect)
    }

    pub(super) fn into_parts(
        mut self,
    ) -> (
        DetachedRetainedPart<F>,
        DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
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

    pub(super) fn try_replace_canonical_assembly(
        &mut self,
        replacement: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    ) -> Result<(), RetainedWriterError<D>> {
        if !self.has_exact_product_graph() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedWriterError::StorageDropUnverified);
        }
        let Some(batches) = self.batches.as_mut() else {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedWriterError::StorageDropUnverified);
        };
        match batches.replace_empty_canonical_assembly(replacement) {
            Ok(previous) => {
                drop(previous);
                Ok(())
            }
            Err(replacement) => {
                drop(replacement);
                self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                Err(RetainedWriterError::StorageDropUnverified)
            }
        }
    }

    pub(super) fn first_account(&self) -> Option<&PhysicalStorageAccount> {
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

    pub(super) fn fail_closed_after_recorded_poison(&mut self) {
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

    pub(super) fn record_forgotten_physical_edges(&self) {
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.record_forgotten_physical_edge();
        }
        if let Some(evidence) = self.evidence.as_ref() {
            evidence.record_forgotten_physical_edge();
        }
    }

    #[cfg(test)]
    pub(super) fn test_evidence_allocation(
        &self,
    ) -> &RetainedAllocation<TerminalLedgerEvidence<P, D>> {
        match self.evidence.as_ref() {
            Some(evidence) => evidence.allocation(),
            None => unreachable_final_owner_state(),
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

    pub(super) fn try_bind(
        mut self,
        ledger: &Ledger<O, P, D>,
    ) -> Result<Self, RetainedWriterError<D>>
    where
        D: fmt::Debug,
    {
        let bind_result = catch_unwind(AssertUnwindSafe(|| {
            let batches = match self.batches.as_mut() {
                Some(batches) => batches,
                None => unreachable_final_owner_state(),
            };
            ledger.try_bind_fallback_baseline(batches)
        }));
        match bind_result {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                Err(RetainedWriterError::StorageDropUnverified)
            }
            Ok(Err(_cause)) => {
                let poisoned = self
                    .account()
                    .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                Err(if self.discard_rejected().is_ok() {
                    RetainedWriterError::AccountPoisoned(poisoned)
                } else {
                    RetainedWriterError::StorageDropUnverified
                })
            }
            Ok(Ok(())) => Ok(self),
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
        // An unrelated host unwind does not invalidate this exact storage graph. The ordinary
        // discard path already catches payload failure and preserves uncertain charges.
        let _ = self.try_discard_all();
    }
}

pub(super) struct StartupFallbackOwner<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fallback: Option<RetainedPart<F, OutcomeEvidenceDestination, O, P, D>>,
    #[allow(
        clippy::type_complexity,
        reason = "the concrete linear owner type documents the exact evidence destination and ledger brand"
    )]
    evidence:
        Option<RetainedPart<TerminalLedgerEvidence<P, D>, OutcomeEvidenceDestination, O, P, D>>,
    canonical_assembly: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_assembly: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
}

impl<O, P, D, F> StartupFallbackOwner<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) fn from_complete(
        fallback: RetainedPart<F, OutcomeEvidenceDestination, O, P, D>,
        evidence: RetainedPart<TerminalLedgerEvidence<P, D>, OutcomeEvidenceDestination, O, P, D>,
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

    pub(super) fn try_into_baseline(
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
