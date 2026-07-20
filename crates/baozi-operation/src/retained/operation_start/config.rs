use crate::ledger::{
    AttributionBatchCreateError, LedgerCause, OutcomeEvidenceDestination, OutputCommitment,
};
use crate::retained::account::{AccountPoisonReason, AccountPoisoned};
use crate::retained::allocation::{RetainedAllocationError, RetainedAllocationPlan};
use crate::retained::final_owner::unreachable_final_owner_state;
use crate::retained::physical::{AccountActionAdmissionError, PendingRetainedRollback};
use std::fmt;

/// Fixed capacities for one destination-typed retained writer or publication assembly.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedBatchLimits {
    pub(in crate::retained) attribution_slots: usize,
    pub(in crate::retained) commitment_slots: usize,
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
    pub(in crate::retained) phase: P,
    pub(in crate::retained) capacity_dimension: D,
    pub(in crate::retained) attribution_dimension: D,
    pub(in crate::retained) limits: RetainedBatchLimits,
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

    pub(in crate::retained) const fn charges(self) -> [(D, u64); 2] {
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
    pub(in crate::retained) terminal_writer: RetainedWriterPlan<P, D>,
    pub(in crate::retained) terminal_account_share_work: RetainedHandleSharePlan<D>,
    pub(in crate::retained) fallback_plan: RetainedAllocationPlan,
    pub(in crate::retained) fallback_commitment: OutputCommitment<D, OutcomeEvidenceDestination>,
    pub(in crate::retained) snapshot_commitment: OutputCommitment<D, OutcomeEvidenceDestination>,
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
    /// The coordinator did not reproduce the prequalified terminal evidence catalog.
    TerminalEvidenceCatalogInvariant,
    /// A retained writer action failed during operation startup.
    Writer(RetainedWriterError<D>),
}

impl<D: fmt::Debug> fmt::Display for RetainedOperationStartError<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTerminalLimits => {
                formatter.write_str("retained terminal limits cannot contain mandatory owners")
            }
            Self::TerminalEvidenceCatalogInvariant => formatter.write_str(
                "retained terminal evidence catalog violated its qualified shape or ordering",
            ),
            Self::Writer(cause) => write!(formatter, "retained operation startup failed: {cause}"),
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for RetainedOperationStartError<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTerminalLimits | Self::TerminalEvidenceCatalogInvariant => None,
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
pub(in crate::retained) fn map_batch_error<D>(
    error: AttributionBatchCreateError<D>,
) -> RetainedWriterError<D> {
    match error {
        AttributionBatchCreateError::CapacityOverflow => RetainedWriterError::BatchCapacity,
        AttributionBatchCreateError::Ledger(cause) => RetainedWriterError::Ledger(cause),
        AttributionBatchCreateError::AllocationFailed => RetainedWriterError::BatchAllocationFailed,
    }
}

pub(in crate::retained) fn map_pending_rollback_or(
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

pub(in crate::retained) fn map_faulted_pending_rollback(
    rollback: PendingRetainedRollback,
) -> RetainedAllocationError {
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

pub(in crate::retained) fn map_faulted_pending_writer_rollback<D>(
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

pub(in crate::retained) fn map_qualified_startup_batch_error<D>(
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

pub(in crate::retained) fn map_account_action_admission_error<D>(
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
