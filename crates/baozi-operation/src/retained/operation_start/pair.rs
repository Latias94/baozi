use super::config::RetainedWriterError;
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{
    AttributionReservationBatch, CanonicalOutput, FallbackBatchBaseline,
    OperationAttributionReservation, OutcomeEvidenceDestination,
};
use crate::owner::OperationOwner;
use crate::retained::account::{AccountPoisonReason, AccountPoisoned, AdmissionCharge};
use crate::retained::allocation::{
    AdmissionLease, LeafRetainedPolicy, PolicyRetainedAllocation, RetainedAllocationError,
    RetainedAllocationPlan, RetainedBuildFailureCode, RetainedLeaseGuard, RetainedPayload,
    UnarmedRetainedFinalOwner, map_retained_final_owner_error,
};
use crate::retained::evidence::{
    TerminalEvidenceBuildError, TerminalEvidenceKey, TerminalLedgerEvidence,
};
use crate::retained::final_owner::unreachable_final_owner_state;
use crate::retained::ownership::{RetainedPart, RetainedPartCleanupError, StartupFallbackOwner};
use crate::retained::physical::{
    PendingRetainedAdmission, PendingRetainedRollback, PendingStartRetainedAdmissions,
    PhysicalStorageAccount, RetainedAdmissionPairPlan, StartupAdmissionGuard,
};
#[cfg(test)]
use crate::retained::take_injected_start_fallback_account_anchor_loss;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

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

#[cfg(test)]
std::thread_local! {
    static STARTUP_FALLBACK_ALLOCATION_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static STARTUP_EVIDENCE_ALLOCATION_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static MISMATCH_NEXT_STARTUP_FALLBACK_FOOTPRINT: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static MISMATCH_NEXT_STARTUP_EVIDENCE_FOOTPRINT: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

fn note_startup_payload_allocation_attempt(side: StartupRetainedSide) {
    #[cfg(test)]
    match side {
        StartupRetainedSide::Fallback => STARTUP_FALLBACK_ALLOCATION_ATTEMPTS.with(|entries| {
            entries.set(entries.get().saturating_add(1));
        }),
        StartupRetainedSide::Evidence => STARTUP_EVIDENCE_ALLOCATION_ATTEMPTS.with(|entries| {
            entries.set(entries.get().saturating_add(1));
        }),
    }
    #[cfg(not(test))]
    let _ = side;
}

#[cfg(test)]
pub(crate) fn test_startup_payload_allocation_attempts() -> (usize, usize) {
    (
        STARTUP_FALLBACK_ALLOCATION_ATTEMPTS.with(std::cell::Cell::get),
        STARTUP_EVIDENCE_ALLOCATION_ATTEMPTS.with(std::cell::Cell::get),
    )
}

#[cfg(test)]
pub(crate) fn reset_startup_payload_allocation_attempts() {
    STARTUP_FALLBACK_ALLOCATION_ATTEMPTS.with(|entries| entries.set(0));
    STARTUP_EVIDENCE_ALLOCATION_ATTEMPTS.with(|entries| entries.set(0));
}

#[cfg(test)]
pub(crate) fn mismatch_next_startup_fallback_footprint() {
    MISMATCH_NEXT_STARTUP_FALLBACK_FOOTPRINT.with(|mismatch| mismatch.set(true));
}

#[cfg(test)]
pub(crate) fn mismatch_next_startup_evidence_footprint() {
    MISMATCH_NEXT_STARTUP_EVIDENCE_FOOTPRINT.with(|mismatch| mismatch.set(true));
}

#[cfg(test)]
pub(crate) fn reset_startup_footprint_mismatches() {
    MISMATCH_NEXT_STARTUP_FALLBACK_FOOTPRINT.with(|mismatch| mismatch.set(false));
    MISMATCH_NEXT_STARTUP_EVIDENCE_FOOTPRINT.with(|mismatch| mismatch.set(false));
}

fn take_startup_footprint_mismatch(side: StartupRetainedSide) -> bool {
    #[cfg(test)]
    {
        match side {
            StartupRetainedSide::Fallback => {
                MISMATCH_NEXT_STARTUP_FALLBACK_FOOTPRINT.with(|mismatch| mismatch.replace(false))
            }
            StartupRetainedSide::Evidence => {
                MISMATCH_NEXT_STARTUP_EVIDENCE_FOOTPRINT.with(|mismatch| mismatch.replace(false))
            }
        }
    }
    #[cfg(not(test))]
    {
        let _ = side;
        false
    }
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
        batch: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
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
        let (account, admission, batch) = match pending {
            Self::Pending {
                account,
                admission,
                batch,
            } => (account, admission, batch),
            state => {
                *self = state;
                unreachable_final_owner_state();
            }
        };
        *self = Self::Constructing(StartupRetainedSideConstruction {
            unarmed: None,
            attribution: None,
            batch: Some(batch),
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

    fn try_preflight(
        &self,
        side: StartupRetainedSide,
        allocation_plan: RetainedAllocationPlan,
    ) -> Result<(), StartupRetainedSideFailure<D>> {
        let Self::Pending {
            account,
            admission,
            batch,
        } = self
        else {
            return Err(StartupRetainedSideFailure::ownership(
                RetainedWriterError::InvalidPublicationPlan,
            ));
        };
        let expected_charge = account
            .checked_retained_charge::<T>(allocation_plan)
            .map_err(|cause| {
                StartupRetainedSideFailure::clean(RetainedWriterError::Physical(cause))
            })?;
        if take_startup_footprint_mismatch(side)
            || expected_charge != admission.charge
            || batch.attribution_len() != 1
            || batch.attributed_bytes() != Some(admission.charge.bytes)
            || !account.is_same_account(admission.account)
        {
            return Err(StartupRetainedSideFailure::ownership(
                RetainedWriterError::InvalidPublicationPlan,
            ));
        }
        Ok(())
    }

    fn try_prepare(
        &mut self,
        side: StartupRetainedSide,
    ) -> Result<(), StartupRetainedSideFailure<D>> {
        self.begin_construction();
        let construction = self.construction_mut();
        note_startup_payload_allocation_attempt(side);
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
                batch,
            } => {
                let lease_owner = admission.transfer_to_aggregate_containment();
                StartupSideContainmentComponents {
                    batch: Some(batch),
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
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
    F: Send + Sync + 'static,
{
    account: &'account PhysicalStorageAccount,
    pair_plan: RetainedAdmissionPairPlan,
    fallback_allocation_plan: RetainedAllocationPlan,
    evidence_allocation_plan: RetainedAllocationPlan,
    fallback: ReadyStartupRetainedSide<'account, O, P, D, F>,
    evidence: ReadyStartupRetainedSide<'account, O, P, D, TerminalLedgerEvidence<P, D>>,
    canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
}

struct StartupFallbackBuilt<'account, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
    F: Send + Sync + 'static,
{
    fallback: RetainedPart<F, OutcomeEvidenceDestination, O, P, D>,
    evidence: ReadyStartupRetainedSide<'account, O, P, D, TerminalLedgerEvidence<P, D>>,
    canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    account: &'account PhysicalStorageAccount,
    pair_plan: RetainedAdmissionPairPlan,
    fallback_allocation_plan: RetainedAllocationPlan,
    evidence_allocation_plan: RetainedAllocationPlan,
}

#[derive(Clone, Copy)]
enum StartupCleanupDisposition {
    Armed,
    FaultAnchored(AccountPoisoned),
    Drained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::retained) enum StartupTerminalEvidenceBuildError {
    Rejected(RetainedBuildFailureCode),
    StructuralInvariant,
}

impl From<TerminalEvidenceBuildError> for StartupTerminalEvidenceBuildError {
    fn from(error: TerminalEvidenceBuildError) -> Self {
        match error {
            TerminalEvidenceBuildError::DimensionAllocationFailed => {
                Self::Rejected(RetainedBuildFailureCode::new(3))
            }
            TerminalEvidenceBuildError::PhaseAllocationFailed => {
                Self::Rejected(RetainedBuildFailureCode::new(4))
            }
            TerminalEvidenceBuildError::ShapeMismatch { .. }
            | TerminalEvidenceBuildError::Catalog(_)
            | TerminalEvidenceBuildError::DimensionCapacity(_)
            | TerminalEvidenceBuildError::PhaseCapacity(_) => Self::StructuralInvariant,
        }
    }
}

impl<'account, O, P, D, F> StartupReadyPair<'account, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
{
    fn into_cleanup_guard(self) -> StartupRetainedPartGuard<'account, O, P, D, F> {
        StartupRetainedPartGuard {
            account: self.account,
            pair_plan: self.pair_plan,
            fallback_allocation_plan: self.fallback_allocation_plan,
            evidence_allocation_plan: self.evidence_allocation_plan,
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
        build_evidence: impl FnOnce() -> Result<
            TerminalLedgerEvidence<P, D>,
            StartupTerminalEvidenceBuildError,
        >,
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
            fallback_allocation_plan,
            evidence_allocation_plan,
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
            fallback_allocation_plan,
            evidence_allocation_plan,
        }
        .try_build_evidence(build_evidence)
    }
}

impl<'account, O, P, D, F> StartupFallbackBuilt<'account, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
{
    fn into_cleanup_guard(self) -> StartupRetainedPartGuard<'account, O, P, D, F> {
        StartupRetainedPartGuard {
            account: self.account,
            pair_plan: self.pair_plan,
            fallback_allocation_plan: self.fallback_allocation_plan,
            evidence_allocation_plan: self.evidence_allocation_plan,
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
        build_evidence: impl FnOnce() -> Result<
            TerminalLedgerEvidence<P, D>,
            StartupTerminalEvidenceBuildError,
        >,
    ) -> Result<StartupFallbackOwner<O, P, D, F>, RetainedWriterError<D>> {
        let evidence = match catch_unwind(AssertUnwindSafe(build_evidence)) {
            Ok(Ok(value)) => value,
            Ok(Err(StartupTerminalEvidenceBuildError::Rejected(cause))) => {
                return Err(self
                    .into_cleanup_guard()
                    .finish_evidence_build_rejection(cause));
            }
            Ok(Err(StartupTerminalEvidenceBuildError::StructuralInvariant)) => {
                return Err(self
                    .into_cleanup_guard()
                    .finish_evidence_construction_invariant());
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
            fallback_allocation_plan: _,
            evidence_allocation_plan: _,
        } = self;
        Ok(StartupFallbackOwner::from_complete(
            fallback,
            ready_evidence.commit(evidence),
            canonical_assembly,
            evidence_assembly,
        ))
    }
}

pub(in crate::retained) struct StartupRetainedPartGuard<'account, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
{
    account: &'account PhysicalStorageAccount,
    pair_plan: RetainedAdmissionPairPlan,
    fallback_allocation_plan: RetainedAllocationPlan,
    evidence_allocation_plan: RetainedAllocationPlan,
    pending_pair: Option<PendingStartRetainedAdmissions<'account>>,
    evidence_account: Option<PhysicalStorageAccount>,
    fallback_lease_owner: Option<PhysicalStorageAccount>,
    evidence_lease_owner: Option<PhysicalStorageAccount>,
    fallback: StartupRetainedSideState<'account, O, P, D, F>,
    evidence: StartupRetainedSideState<'account, O, P, D, TerminalLedgerEvidence<P, D>>,
    contained_pending: Option<ContainedStartupRetainedPair<O, P, D>>,
    canonical_assembly: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_assembly: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    cleanup_disposition: StartupCleanupDisposition,
}

impl<'account, O, P, D, F> StartupRetainedPartGuard<'account, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
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

    pub(in crate::retained) fn new(
        mut admission_guard: StartupAdmissionGuard<'account>,
        baseline: FallbackBatchBaseline<O, P, D>,
        fallback_allocation_plan: RetainedAllocationPlan,
        evidence_allocation_plan: RetainedAllocationPlan,
    ) -> Self {
        let (canonical_assembly, evidence_assembly, fallback_batch, evidence_batch) =
            baseline.into_parts();
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
        let mut this = Self {
            account: admission_guard.account,
            pair_plan,
            fallback_allocation_plan,
            evidence_allocation_plan,
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
        };
        this.split_pending_pair(fallback_batch, evidence_batch);
        this
    }

    fn split_pending_pair(
        &mut self,
        fallback_batch: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        evidence_batch: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    ) {
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
            batch: fallback_batch,
        };
        self.evidence = StartupRetainedSideState::Pending {
            account: evidence_account,
            admission: evidence,
            batch: evidence_batch,
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
            fallback_allocation_plan: self.fallback_allocation_plan,
            evidence_allocation_plan: self.evidence_allocation_plan,
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
        self,
        payload: Box<dyn std::any::Any + Send>,
    ) -> RetainedWriterError<D> {
        discard_untrusted_panic_payload(payload);
        self.finish_uncertain_evidence_construction(
            AccountPoisonReason::RetainedBuilderPanicked,
            RetainedAllocationError::BuilderPanicked,
        )
    }

    fn finish_evidence_construction_invariant(self) -> RetainedWriterError<D> {
        self.finish_uncertain_evidence_construction(
            AccountPoisonReason::TerminalEvidenceConstructionUnverified,
            RetainedAllocationError::ConstructionInvariantUnverified,
        )
    }

    fn finish_uncertain_evidence_construction(
        mut self,
        primary_reason: AccountPoisonReason,
        primary_error: RetainedAllocationError,
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
                primary_reason
            };
            let poisoned = self.account.record_poison(reason);
            self.anchor_fault(poisoned);
        }
        let error = if storage_failure {
            RetainedWriterError::StorageDropUnverified
        } else if ownership_failure {
            RetainedWriterError::OwnershipUnverified
        } else if charge_failure {
            RetainedWriterError::ChargeReleaseUnverified
        } else {
            RetainedWriterError::Physical(primary_error)
        };
        self.drain_handled_state();
        self.mark_drained();
        error
    }

    pub(in crate::retained) fn try_build(
        mut self,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
        build_evidence: impl FnOnce() -> Result<
            TerminalLedgerEvidence<P, D>,
            StartupTerminalEvidenceBuildError,
        >,
    ) -> Result<StartupFallbackOwner<O, P, D, F>, RetainedWriterError<D>> {
        if let Err(failure) = self
            .fallback
            .try_preflight(StartupRetainedSide::Fallback, self.fallback_allocation_plan)
        {
            return Err(self.finish_rejection(StartupRetainedSide::Fallback, failure));
        }
        if let Err(failure) = self
            .evidence
            .try_preflight(StartupRetainedSide::Evidence, self.evidence_allocation_plan)
        {
            return Err(self.finish_rejection(StartupRetainedSide::Evidence, failure));
        }
        if let Err(failure) = self.fallback.try_prepare(StartupRetainedSide::Fallback) {
            return Err(self.finish_rejection(StartupRetainedSide::Fallback, failure));
        }
        if let Err(failure) = self.evidence.try_prepare(StartupRetainedSide::Evidence) {
            return Err(self.finish_rejection(StartupRetainedSide::Evidence, failure));
        }
        self.try_into_ready_pair()?
            .try_build(build_fallback, build_evidence)
    }
}

impl<O, P, D, F> Drop for StartupRetainedPartGuard<'_, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
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
