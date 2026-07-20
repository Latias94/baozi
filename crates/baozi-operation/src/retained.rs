//! Fallible physical storage accounting for retained operation allocations.

#![allow(
    clippy::result_large_err,
    reason = "bounded poison snapshots stay inline so containment never needs a failure-path allocation"
)]

mod account;
mod allocation;
mod block_table;
mod evidence;
mod final_owner;
mod operation_start;
mod ownership;
mod physical;
mod startup;

use crate::control::discard_untrusted_panic_payload;
use crate::coordinator::{FinalProgress, OperationCoordinator};
#[cfg(test)]
use crate::ledger::AttributionBatchCreateError;
use crate::ledger::{
    AttributionBatchPlan, AttributionReservationBatch, CanonicalOutput, CommitmentPushError,
    DimensionSnapshot, FallbackBatchBaseline, FreezeReadyTypedProof, Ledger, LedgerCause,
    LedgerLimits, OutcomeEvidenceDestination, OutputCommitment, PhaseCompletionError,
    PhaseSnapshot, QualifiedBothDiscardFreezeProof, QualifiedEvidenceDiscardFreezeProof,
    QualifiedTypedDiscardFreezeProof, StartupAccountShareEnvelope, StartupAttributionBatchPlans,
    StartupRetainedAttribution,
};
use crate::{ControlCause, OperationControl, OperationOwner, ProgressPhase, ProgressUnit};
pub use account::{
    AccountPoisonEvidence, AccountPoisonReason, AccountPoisoned, FinalizationContainmentError,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountFactorySnapshot,
    PhysicalStorageAccountLimits, PhysicalStorageAccountSnapshot, PhysicalStorageLimitError,
};
use account::{
    AdmissionCharge, AdmissionError, CounterState, lock_recovering_poison,
    quarantined_account_reason, release, release_locked, try_admit, try_admit_locked,
};
use allocation::{
    AccountPoisonEventWitness, AdmissionLease, AdmissionOwner, FinalOwnerReleaseOutcome,
    PolicyRetainedAllocation, RetainedAllocation, RetainedLeaseGuard,
    RetainedMembershipDestination, RetainedPayload, RetainedSetId, RetainedValuePolicy,
    map_retained_admission_error, record_account_poison_event, try_next_retained_set_id,
};
#[cfg(test)]
use allocation::{LeafRetainedPolicy, RetainedFinalOwnerInner, test_retained_set_id_requests};
#[allow(
    unused_imports,
    reason = "the retained root preserves crate-private paths while canonical consumers migrate"
)]
pub(crate) use allocation::{
    MAX_RETAINED_INLINE_BYTES, RetainedAllocationError, RetainedAllocationPlan,
    RetainedAllocationPlanError, RetainedBuildFailureCode,
};
pub(super) use block_table::RetainedSlotIdentity;
#[cfg(test)]
use block_table::try_share_retained_block_after_admission;
pub(crate) use block_table::{
    BlockEmplacer, BlockPlacement, CanonicalRetainedSlot, OutcomeEvidenceRetainedSlot,
    PublishedCanonicalAggregate, PublishedOutcomeEvidenceAggregate, RetainedBlockTablePlan,
    RetainedClosureSession, RetainedEvidenceClosureSession,
};
use block_table::{
    PublishedRetainedAggregate, QualifiedRetainedBlockTablePlan, RetainedTableAllocation,
    record_forgotten_table_edges, retained_block_table_is_exact, try_create_retained_block_table,
    try_qualify_retained_block_table_plan, try_retain_block, try_share_retained_block,
};
pub(crate) use evidence::{
    FallbackHeaderSnapshot, PreparationFailureClass, PreparationFailureCode, TerminalEvidenceCause,
    TerminalEvidenceKey, TerminalEvidenceKeySeal,
};
use evidence::{
    TerminalEvidenceBackingPlan, TerminalEvidenceFreezeQualification, TerminalLedgerEvidence,
};
use final_owner::{
    FinalOwnerAllocation, FinalOwnerAllocationError, FinalOwnerInnerDrop,
    unreachable_final_owner_state,
};
use ownership::{
    DetachedRetainedPart, FallbackPublicationBaseline, PublishedPart, RetainedPartCleanupError,
    RetainedStorageDropOutcome, drop_retained_allocation_before_attribution,
    finish_pre_attribution_batch_cleanup,
};
#[cfg(test)]
use ownership::{RetainedStorageDropSeverity, join_pre_attribution_cleanup_cause};
#[cfg(test)]
use physical::AccountControlInner;
#[cfg(test)]
use physical::{
    AccountActionAdmissionError, AdmittedRetainedShareWork, PendingRetainedRollback,
    StartupAccountOwners,
};
use physical::{
    AccountLinearState, PendingAccountActionAdmission, PendingRetainedAdmission,
    PublicationEligibility, RetainedAdmissionPairPlan, StartupAdmissionGuard,
};
pub use physical::{
    PhysicalStorageAccount, PhysicalStorageAccountCreateError, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryCreateError,
};
use std::any::TypeId;
use std::fmt;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard};
use triomphe::{Arc, UniqueArc};

pub(crate) use operation_start::{
    RetainedBatchLimits, RetainedHandleSharePlan, RetainedOperationStartError,
    RetainedOperationStartPlan, RetainedWriterError, RetainedWriterPlan,
};
use operation_start::{
    StartupRetainedPartGuard, StartupRetainedPartPlan, StartupTerminalEvidenceBuildError,
    map_account_action_admission_error, map_batch_error, map_faulted_pending_rollback,
    map_pending_rollback_or, map_qualified_startup_batch_error, try_admit_retained_share_work,
    try_admit_retained_share_work_before_physical, try_admit_start_share_work,
    try_open_publication_account,
};

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

fn map_commitment_push_error<D, Destination>(
    error: CommitmentPushError<D, Destination>,
) -> RetainedPublicationError<D> {
    match error {
        CommitmentPushError::BatchFull(_) => {
            RetainedPublicationError::Writer(RetainedWriterError::CommitmentBatchFull)
        }
        CommitmentPushError::Ledger { cause, .. } => {
            RetainedPublicationError::Writer(RetainedWriterError::Ledger(cause))
        }
    }
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

struct TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
    fallback: Option<DetachedRetainedPart<F>>,
    evidence: Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>,
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    account: Option<PhysicalStorageAccount>,
}

struct ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock> {
    fallback: DetachedRetainedPart<F>,
    evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    canonical_table: RetainedTableAllocation<CanonicalBlock>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
}

type TerminalPhysicalParts<P, D, F, CanonicalBlock, EvidenceBlock> = (
    DetachedRetainedPart<F>,
    DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    Option<RetainedTableAllocation<CanonicalBlock>>,
    Option<RetainedTableAllocation<EvidenceBlock>>,
    Option<PhysicalStorageAccount>,
);

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn new(
        fallback: DetachedRetainedPart<F>,
        evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
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

    fn evidence(&self) -> &DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        match self.evidence.as_ref() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
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

    fn into_parts(mut self) -> TerminalPhysicalParts<P, D, F, CanonicalBlock, EvidenceBlock> {
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

    fn qualifies_pre_discard_success(&self) -> bool {
        let (
            Some(fallback),
            Some(evidence),
            Some(canonical),
            Some(outcome_evidence),
            Some(account),
        ) = (
            self.fallback.as_ref(),
            self.evidence.as_ref(),
            self.canonical_table.as_ref(),
            self.evidence_table.as_ref(),
            self.account.as_ref(),
        )
        else {
            return false;
        };
        fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(account)
            && evidence.account().is_same_account(account)
            && canonical.belongs_to(account)
            && outcome_evidence.belongs_to(account)
    }

    fn qualifies_pre_cleanup_rich(&self, expect_canonical: bool) -> bool {
        let (Some(fallback), Some(evidence), Some(evidence_table), Some(account)) = (
            self.fallback.as_ref(),
            self.evidence.as_ref(),
            self.evidence_table.as_ref(),
            self.account.as_ref(),
        ) else {
            return false;
        };
        self.canonical_table.is_some() == expect_canonical
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(account)
            && evidence.account().is_same_account(account)
            && evidence_table.belongs_to(account)
            && self
                .canonical_table
                .as_ref()
                .is_none_or(|table| table.belongs_to(account))
    }

    fn qualifies_pre_cleanup_fallback(
        &self,
        expect_canonical: bool,
        expect_evidence: bool,
    ) -> bool {
        let (Some(fallback), Some(evidence)) = (self.fallback.as_ref(), self.evidence.as_ref())
        else {
            return false;
        };
        let table_expected = expect_canonical || expect_evidence;
        self.canonical_table.is_some() == expect_canonical
            && self.evidence_table.is_some() == expect_evidence
            && self.account.is_some() == table_expected
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(evidence.account())
            && self.account.as_ref().is_none_or(|account| {
                fallback.account().is_same_account(account)
                    && self
                        .canonical_table
                        .as_ref()
                        .is_none_or(|table| table.belongs_to(account))
                    && self
                        .evidence_table
                        .as_ref()
                        .is_none_or(|table| table.belongs_to(account))
            })
    }

    fn finish_pre_discard_success(
        mut self,
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
    ) -> ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock> {
        ExactPreDiscardSuccess {
            fallback: self
                .fallback
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            evidence: self
                .evidence
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            canonical_table: self
                .canonical_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            evidence_table: self
                .evidence_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            account: self
                .account
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            canonical_root,
            evidence_root,
        }
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>
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

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn account(&self) -> &PhysicalStorageAccount {
        &self.account
    }

    fn into_terminal(
        self,
    ) -> TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
        TerminalPhysicalOwnershipManifest::new(
            self.fallback,
            self.evidence,
            Some(self.canonical_table),
            Some(self.evidence_table),
            Some(self.account),
        )
    }
}

struct TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct QualifiedSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<QualifiedSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct QualifiedSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
    proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
}

struct QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock> {
    fallback: DetachedRetainedPart<F>,
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
    canonical_table: RetainedTableAllocation<CanonicalBlock>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
}

struct EligibleSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibleSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct EligibleSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
    eligibility: PublicationEligibility,
    proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
    set_id: RetainedSetId,
}

struct VerifiedSuccessPhysicalDiscard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: VerifiedSuccessPhysicalDiscardState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "the pending variant is the single linear owner; boxing would add allocation and split the ownership proof"
)]
enum VerifiedSuccessPhysicalDiscardState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Pending {
        acknowledgement: SuccessDiscardAcknowledgement,
        physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
        eligibility: PublicationEligibility,
        set_id: RetainedSetId,
        discarded_account: PhysicalStorageAccount,
    },
    Acknowledged,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SuccessDiscardAcknowledgement {
    NotStarted,
    InProgress,
}

struct FreezeReadyDetachedTerminalEvidence<P, D> {
    owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    qualification: TerminalEvidenceFreezeQualification,
}

struct FreezeReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<FreezeReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct FreezeReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical_table: RetainedTableAllocation<CanonicalBlock>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
    eligibility: PublicationEligibility,
    proof: FreezeReadyTypedProof<O, P, D>,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
    set_id: RetainedSetId,
    fallback_type: PhantomData<fn() -> F>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
        }
    }

    fn contain_unqualified(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let cause = physical.into_terminal().contain_new_fault(reason);
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        cause
    }

    fn into_prepared(
        mut self,
        set_id: RetainedSetId,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match proof.try_into_qualified_evidence_discard() {
            Ok(proof) => proof,
            Err(proof) => {
                self.proof = Some(proof);
                return Err(
                    self.contain_unqualified(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
                );
            }
        };
        let ExactPreDiscardSuccess {
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            account,
            canonical_root,
            evidence_root,
        } = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());

        let qualified_evidence =
            match QualifiedDetachedTerminalEvidence::try_from_success_proof(evidence, &proof) {
                DetachedTerminalEvidenceQualification::Qualified(evidence) => evidence,
                DetachedTerminalEvidenceQualification::Rejected(evidence)
                | DetachedTerminalEvidenceQualification::Panicked(evidence) => {
                    let physical = ExactPreDiscardSuccess {
                        fallback,
                        evidence,
                        canonical_table,
                        evidence_table,
                        account,
                        canonical_root,
                        evidence_root,
                    };
                    let cause = physical
                        .into_terminal()
                        .contain_new_fault(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
                    proof.retain_fail_closed();
                    return Err(cause);
                }
            };

        let qualified = QualifiedSuccessGraph {
            state: Some(QualifiedSuccessState {
                physical: QualifiedSuccessPhysical {
                    fallback,
                    evidence: qualified_evidence.into_freeze_ready(),
                    canonical_table,
                    evidence_table,
                    account,
                    canonical_root,
                    evidence_root,
                },
                proof,
            }),
        };
        #[cfg(test)]
        let qualified = maybe_drop_success_guard(SuccessGuardDropStage::Qualified, qualified);
        qualified.into_prepared(set_id)
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn take_state(&mut self) -> QualifiedSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        self.state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }

    fn contain_after_quarantine(
        mut self,
        quarantine: AccountPoisoned,
    ) -> FinalizationContainmentError {
        let QualifiedSuccessState { physical, proof } = self.take_state();
        let cause = physical
            .into_terminal()
            .contain_after_quarantine(quarantine);
        proof.retain_fail_closed();
        cause
    }

    fn into_prepared(
        mut self,
        set_id: RetainedSetId,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let eligibility = match self
            .state
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .physical
            .account
            .try_publication_eligibility()
        {
            Ok(eligibility) => eligibility,
            Err(quarantine) => return Err(self.contain_after_quarantine(quarantine)),
        };
        let QualifiedSuccessState { physical, proof } = self.take_state();
        let eligible = EligibleSuccessGraph {
            state: Some(EligibleSuccessState {
                physical,
                eligibility,
                proof,
                set_id,
            }),
        };
        #[cfg(test)]
        let eligible = maybe_drop_success_guard(SuccessGuardDropStage::Eligible, eligible);
        eligible.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for QualifiedSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(QualifiedSuccessState { physical, proof }) = self.state.take() {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
        }
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn into_terminal(
        self,
    ) -> TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
        let Self {
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            account,
            canonical_root: _,
            evidence_root: _,
        } = self;
        TerminalPhysicalOwnershipManifest::new(
            fallback,
            evidence.into_unfrozen_owner(),
            Some(canonical_table),
            Some(evidence_table),
            Some(account),
        )
    }

    fn into_freeze_ready<O>(
        self,
        proof: FreezeReadyTypedProof<O, P, D>,
        eligibility: PublicationEligibility,
        set_id: RetainedSetId,
    ) -> FreezeReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord,
    {
        let Self {
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            account,
            canonical_root,
            evidence_root,
        } = self;
        drop(fallback);
        FreezeReadySuccessGraph {
            state: Some(FreezeReadySuccessState {
                canonical_table,
                evidence_table,
                account,
                evidence,
                eligibility,
                proof,
                canonical_root,
                evidence_root,
                set_id,
                fallback_type: PhantomData,
            }),
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    EligibleSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn take_state(&mut self) -> EligibleSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        self.state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }

    fn contain_new_fault(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let EligibleSuccessState {
            physical,
            eligibility,
            proof,
            set_id: _,
        } = self.take_state();
        let cause = physical.into_terminal().contain_new_fault(reason);
        proof.retain_fail_closed();
        drop(eligibility);
        cause
    }

    fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let EligibleSuccessState {
            physical,
            eligibility,
            proof,
            set_id: _,
        } = self.take_state();
        let cause = physical.into_terminal().contain_after_recorded(reason);
        proof.retain_fail_closed();
        drop(eligibility);
        cause
    }

    fn contain_cleanup(self, cleanup: RetainedPartCleanupError) -> FinalizationContainmentError {
        match cleanup {
            RetainedPartCleanupError::MissingFaultAnchor
            | RetainedPartCleanupError::DeferredOwnershipFault => {
                self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified)
            }
            RetainedPartCleanupError::NewFaultAlreadyRecorded(_)
            | RetainedPartCleanupError::StillSharedOwnershipFault(_) => {
                self.contain_after_recorded(AccountPoisonReason::OwnershipGraphUnverified)
            }
            RetainedPartCleanupError::StorageReleaseUnverified(_) => {
                self.contain_after_recorded(AccountPoisonReason::RetainedStorageReleaseUnverified)
            }
            RetainedPartCleanupError::ChargeReleaseUnverified(witness) => {
                self.contain_after_recorded(witness.reason)
            }
        }
    }

    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let discarded_account = match self
            .state
            .as_mut()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .physical
            .fallback
            .try_discard_allocation()
        {
            Ok(account) => account,
            Err(cleanup) => return Err(self.contain_cleanup(cleanup)),
        };
        let EligibleSuccessState {
            physical,
            eligibility,
            proof,
            set_id,
        } = self.take_state();
        let verified = VerifiedSuccessPhysicalDiscard::new(
            physical,
            proof,
            eligibility,
            set_id,
            discarded_account,
        );
        #[cfg(test)]
        let verified = maybe_drop_success_guard(SuccessGuardDropStage::Verified, verified);
        let freeze_ready = verified.acknowledge();
        #[cfg(test)]
        let freeze_ready =
            maybe_drop_success_guard(SuccessGuardDropStage::FreezeReady, freeze_ready);
        Ok(freeze_ready.freeze())
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for EligibleSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(EligibleSuccessState {
            physical,
            eligibility,
            proof,
            set_id: _,
        }) = self.state.take()
        {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            drop(eligibility);
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    VerifiedSuccessPhysicalDiscard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn new(
        physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
        eligibility: PublicationEligibility,
        set_id: RetainedSetId,
        discarded_account: PhysicalStorageAccount,
    ) -> Self {
        Self {
            state: VerifiedSuccessPhysicalDiscardState::Pending {
                acknowledgement: SuccessDiscardAcknowledgement::NotStarted,
                physical,
                proof,
                eligibility,
                set_id,
                discarded_account,
            },
        }
    }

    fn acknowledge(mut self) -> FreezeReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        let freeze_ready = match &mut self.state {
            VerifiedSuccessPhysicalDiscardState::Pending {
                acknowledgement,
                proof,
                ..
            } => {
                *acknowledgement = SuccessDiscardAcknowledgement::InProgress;
                #[cfg(test)]
                maybe_panic_success_guard_stage(SuccessGuardDropStage::VerifiedInProgress);
                proof.release_discard_after_physical_cleanup_in_place()
            }
            VerifiedSuccessPhysicalDiscardState::Acknowledged => unreachable_final_owner_state(),
        };
        let state = std::mem::replace(
            &mut self.state,
            VerifiedSuccessPhysicalDiscardState::Acknowledged,
        );
        match state {
            VerifiedSuccessPhysicalDiscardState::Pending {
                acknowledgement: _,
                physical,
                proof: _,
                eligibility,
                set_id,
                discarded_account,
            } => {
                drop(discarded_account);
                physical.into_freeze_ready(freeze_ready, eligibility, set_id)
            }
            VerifiedSuccessPhysicalDiscardState::Acknowledged => unreachable_final_owner_state(),
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for VerifiedSuccessPhysicalDiscard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        let state = std::mem::replace(
            &mut self.state,
            VerifiedSuccessPhysicalDiscardState::Acknowledged,
        );
        if let VerifiedSuccessPhysicalDiscardState::Pending {
            acknowledgement: _,
            physical,
            proof,
            eligibility,
            set_id: _,
            discarded_account,
        } = state
        {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            drop(discarded_account);
            drop(eligibility);
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    FreezeReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn freeze(mut self) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
        let Some(state) = self.state.take() else {
            unreachable_final_owner_state()
        };
        let mut state = scopeguard::guard(
            state,
            contain_freeze_ready_success_state::<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        );
        let snapshot = state.proof.freeze_in_place();
        #[cfg(test)]
        maybe_panic_success_guard_stage(SuccessGuardDropStage::LedgerPublished);
        state
            .evidence
            .freeze_in_place(TerminalEvidenceCause::Success, &snapshot);
        let state = scopeguard::ScopeGuard::into_inner(state);
        let FreezeReadySuccessState {
            canonical_table,
            evidence_table,
            account,
            evidence,
            eligibility: _eligibility,
            proof,
            canonical_root,
            evidence_root,
            set_id,
            fallback_type: _,
        } = state;
        drop(proof);
        let evidence = evidence.into_frozen_owner();
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::Success {
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
            evidence: PublishedPart::from_exact_owner(evidence),
        }
    }
}

fn contain_freeze_ready_success_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: FreezeReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let FreezeReadySuccessState {
        canonical_table,
        evidence_table,
        account,
        evidence,
        eligibility,
        proof,
        canonical_root: _,
        evidence_root: _,
        set_id: _,
        fallback_type: _,
    } = state;
    let physical = TerminalPhysicalOwnershipManifest::new(
        DetachedRetainedPart::<F>::empty(),
        evidence.into_unfrozen_owner(),
        Some(canonical_table),
        Some(evidence_table),
        Some(account),
    );
    let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
    proof.retain_fail_closed();
    drop(eligibility);
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for FreezeReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            contain_freeze_ready_success_state(state);
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
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
            proof.retain_fail_closed();
        }
    }
}

struct TerminalRichFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
    evidence_root: RetainedSlotIdentity,
    cause: PreparationFailureCode,
}

struct QualifiedRichOutput<P, D, F, EvidenceBlock> {
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    evidence_root: RetainedSlotIdentity,
    cause: PreparationFailureCode,
    set_id: RetainedSetId,
    fallback_type: PhantomData<fn() -> F>,
}

enum QualifiedRichDiscard<O, P, D, F, CanonicalBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Evidence {
        fallback: DetachedRetainedPart<F>,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
    },
    Both {
        canonical_table: RetainedTableAllocation<CanonicalBlock>,
        fallback: DetachedRetainedPart<F>,
        proof: QualifiedBothDiscardFreezeProof<O, P, D>,
    },
}

struct QualifiedRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    discard: QualifiedRichDiscard<O, P, D, F, CanonicalBlock>,
}

struct QualifiedRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<QualifiedRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct EligibleRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    discard: QualifiedRichDiscard<O, P, D, F, CanonicalBlock>,
    eligibility: PublicationEligibility,
}

struct EligibleRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibleRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct FreezeReadyRichState<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    proof: FreezeReadyTypedProof<O, P, D>,
    eligibility: PublicationEligibility,
}

struct FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<FreezeReadyRichState<O, P, D, F, EvidenceBlock>>,
}

#[allow(
    clippy::type_complexity,
    reason = "the tuple moves one exact physical graph beside its matching typed ledger proof"
)]
fn rich_state_into_terminal<P, D, F, CanonicalBlock, EvidenceBlock, O>(
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    discard: QualifiedRichDiscard<O, P, D, F, CanonicalBlock>,
) -> (
    TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    QualifiedTypedDiscardFreezeProof<O, P, D>,
)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let QualifiedRichOutput {
        evidence,
        evidence_table,
        account,
        ..
    } = output;
    match discard {
        QualifiedRichDiscard::Evidence { fallback, proof } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                None,
                Some(evidence_table),
                Some(account),
            ),
            QualifiedTypedDiscardFreezeProof::Evidence(proof),
        ),
        QualifiedRichDiscard::Both {
            canonical_table,
            fallback,
            proof,
        } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                Some(canonical_table),
                Some(evidence_table),
                Some(account),
            ),
            QualifiedTypedDiscardFreezeProof::Both(proof),
        ),
    }
}

fn contain_qualified_rich_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: QualifiedRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let (physical, proof) = rich_state_into_terminal(state.output, state.discard);
    let cause = if already_recorded {
        physical.contain_after_recorded(reason)
    } else {
        physical.contain_new_fault(reason)
    };
    proof.retain_fail_closed();
    cause
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalRichFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
        evidence_root: RetainedSlotIdentity,
        cause: PreparationFailureCode,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
            evidence_root,
            cause,
        }
    }

    fn contain_unqualified(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let cause = physical.contain_new_fault(reason);
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        cause
    }

    fn into_prepared(
        mut self,
        set_id: RetainedSetId,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match proof.try_into_qualified_discards() {
            Ok(proof) => proof,
            Err(proof) => {
                self.proof = Some(proof);
                return Err(
                    self.contain_unqualified(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
                );
            }
        };
        let expect_canonical = match &proof {
            QualifiedTypedDiscardFreezeProof::Evidence(_) => false,
            QualifiedTypedDiscardFreezeProof::Both(_) => true,
            _ => {
                let physical = self
                    .physical
                    .take()
                    .unwrap_or_else(|| unreachable_final_owner_state());
                let cause =
                    physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                proof.retain_fail_closed();
                return Err(cause);
            }
        };
        if !self
            .physical
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .qualifies_pre_cleanup_rich(expect_canonical)
        {
            let physical = self
                .physical
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let cause = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            return Err(cause);
        }
        let (fallback, evidence, canonical_table, evidence_table, account) = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .into_parts();
        let qualified_evidence =
            match QualifiedDetachedTerminalEvidence::try_from_discard_proof(evidence, &proof) {
                DetachedTerminalEvidenceQualification::Qualified(evidence) => {
                    evidence.into_freeze_ready()
                }
                DetachedTerminalEvidenceQualification::Rejected(evidence)
                | DetachedTerminalEvidenceQualification::Panicked(evidence) => {
                    let physical = TerminalPhysicalOwnershipManifest::new(
                        fallback,
                        evidence,
                        canonical_table,
                        evidence_table,
                        account,
                    );
                    let cause = physical
                        .contain_new_fault(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
                    proof.retain_fail_closed();
                    return Err(cause);
                }
            };
        let evidence_table = evidence_table.unwrap_or_else(|| unreachable_final_owner_state());
        let account = account.unwrap_or_else(|| unreachable_final_owner_state());
        let output = QualifiedRichOutput {
            evidence: qualified_evidence,
            evidence_table,
            account,
            evidence_root: self.evidence_root,
            cause: self.cause,
            set_id,
            fallback_type: PhantomData,
        };
        let discard = match (proof, canonical_table) {
            (QualifiedTypedDiscardFreezeProof::Evidence(proof), None) => {
                QualifiedRichDiscard::Evidence { fallback, proof }
            }
            (QualifiedTypedDiscardFreezeProof::Both(proof), Some(canonical_table)) => {
                QualifiedRichDiscard::Both {
                    canonical_table,
                    fallback,
                    proof,
                }
            }
            _ => unreachable_final_owner_state(),
        };
        let qualified = QualifiedRichGraph {
            state: Some(QualifiedRichState { output, discard }),
        };
        #[cfg(test)]
        let qualified = maybe_drop_failure_guard(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::Qualified,
            qualified,
        );
        qualified.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalRichFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let eligibility = match state.output.account.try_publication_eligibility() {
            Ok(eligibility) => eligibility,
            Err(quarantine) => {
                let reason = quarantine.snapshot().first_poison_evidence().map_or(
                    AccountPoisonReason::OwnershipGraphUnverified,
                    AccountPoisonEvidence::reason,
                );
                return Err(contain_qualified_rich_state(state, reason, true));
            }
        };
        let eligible = EligibleRichGraph {
            state: Some(EligibleRichState {
                output: state.output,
                discard: state.discard,
                eligibility,
            }),
        };
        #[cfg(test)]
        let eligible = maybe_drop_failure_guard(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::Eligible,
            eligible,
        );
        eligible.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for QualifiedRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = contain_qualified_rich_state(
                state,
                AccountPoisonReason::OwnershipGraphUnverified,
                false,
            );
        }
    }
}

fn contain_eligible_rich_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: EligibleRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let EligibleRichState {
        output,
        discard,
        eligibility,
    } = state;
    let cause = contain_qualified_rich_state(
        QualifiedRichState { output, discard },
        reason,
        already_recorded,
    );
    drop(eligibility);
    cause
}

enum RichDiscardContinuationProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Both(QualifiedBothDiscardFreezeProof<O, P, D>),
    Evidence(QualifiedEvidenceDiscardFreezeProof<O, P, D>),
}

impl<O, P, D> RichDiscardContinuationProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn retain_fail_closed(self) {
        match self {
            Self::Both(proof) => proof.retain_fail_closed(),
            Self::Evidence(proof) => proof.retain_fail_closed(),
        }
    }
}

struct RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    fallback: DetachedRetainedPart<F>,
    proof: RichDiscardContinuationProof<O, P, D>,
    eligibility: PublicationEligibility,
    discarded_account: Option<PhysicalStorageAccount>,
}

fn contain_rich_discard_continuation_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let RichDiscardContinuationState {
        output,
        canonical_table,
        fallback,
        proof,
        eligibility,
        discarded_account,
    } = state;
    let QualifiedRichOutput {
        evidence,
        evidence_table,
        account,
        ..
    } = output;
    let physical = TerminalPhysicalOwnershipManifest::new(
        fallback,
        evidence.into_unfrozen_owner(),
        canonical_table,
        Some(evidence_table),
        Some(account),
    );
    let cause = if already_recorded {
        physical.contain_after_recorded(reason)
    } else {
        physical.contain_new_fault(reason)
    };
    proof.retain_fail_closed();
    drop(discarded_account);
    drop(eligibility);
    cause
}

fn contain_rich_discard_continuation_on_drop<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let _ = contain_rich_discard_continuation_state(
        state,
        AccountPoisonReason::OwnershipGraphUnverified,
        false,
    );
}

fn retained_part_cleanup_reason(cleanup: RetainedPartCleanupError) -> (AccountPoisonReason, bool) {
    match cleanup {
        RetainedPartCleanupError::MissingFaultAnchor
        | RetainedPartCleanupError::DeferredOwnershipFault => {
            (AccountPoisonReason::OwnershipGraphUnverified, false)
        }
        RetainedPartCleanupError::NewFaultAlreadyRecorded(_)
        | RetainedPartCleanupError::StillSharedOwnershipFault(_) => {
            (AccountPoisonReason::OwnershipGraphUnverified, true)
        }
        RetainedPartCleanupError::StorageReleaseUnverified(_) => {
            (AccountPoisonReason::RetainedStorageReleaseUnverified, true)
        }
        RetainedPartCleanupError::ChargeReleaseUnverified(witness) => (witness.reason, true),
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    EligibleRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn finish_discard(
        state: RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Result<FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>, FinalizationContainmentError> {
        let mut state = scopeguard::guard(
            state,
            contain_rich_discard_continuation_on_drop::<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        );
        if matches!(&state.proof, RichDiscardContinuationProof::Both(_)) {
            let canonical_table = state
                .canonical_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let outcome =
                drop_retained_table_with_account_anchor(canonical_table, &state.output.account);
            if let Some((reason, already_recorded)) = table_cleanup_reason(outcome) {
                return Err(contain_rich_discard_continuation_state(
                    scopeguard::ScopeGuard::into_inner(state),
                    reason,
                    already_recorded,
                ));
            }
            let evidence_proof = match &mut state.proof {
                RichDiscardContinuationProof::Both(proof) => {
                    proof.release_canonical_after_physical_cleanup_in_place()
                }
                RichDiscardContinuationProof::Evidence(_) => unreachable_final_owner_state(),
            };
            state.proof = RichDiscardContinuationProof::Evidence(evidence_proof);
            #[cfg(test)]
            maybe_panic_failure_guard_stage(
                FailureTerminalRail::Rich,
                FailureGuardDropStage::CanonicalAcknowledged,
            );
        }

        let discarded_account = match state.fallback.try_discard_allocation() {
            Ok(account) => account,
            Err(cleanup) => {
                let (reason, already_recorded) = retained_part_cleanup_reason(cleanup);
                return Err(contain_rich_discard_continuation_state(
                    scopeguard::ScopeGuard::into_inner(state),
                    reason,
                    already_recorded,
                ));
            }
        };
        state.discarded_account = Some(discarded_account);
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::DiscardCleaned,
        );
        let freeze_ready = match &mut state.proof {
            RichDiscardContinuationProof::Evidence(proof) => {
                proof.release_discard_after_physical_cleanup_in_place()
            }
            RichDiscardContinuationProof::Both(_) => unreachable_final_owner_state(),
        };
        let state = scopeguard::ScopeGuard::into_inner(state);
        let RichDiscardContinuationState {
            output,
            canonical_table,
            fallback,
            proof: _,
            eligibility,
            discarded_account,
        } = state;
        drop(canonical_table);
        drop(fallback);
        drop(discarded_account);
        Ok(FreezeReadyRichGraph {
            state: Some(FreezeReadyRichState {
                output,
                proof: freeze_ready,
                eligibility,
            }),
        })
    }

    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let continuation = match state.discard {
            QualifiedRichDiscard::Evidence { fallback, proof } => RichDiscardContinuationState {
                output: state.output,
                canonical_table: None,
                fallback,
                proof: RichDiscardContinuationProof::Evidence(proof),
                eligibility: state.eligibility,
                discarded_account: None,
            },
            QualifiedRichDiscard::Both {
                canonical_table,
                fallback,
                proof,
            } => RichDiscardContinuationState {
                output: state.output,
                canonical_table: Some(canonical_table),
                fallback,
                proof: RichDiscardContinuationProof::Both(proof),
                eligibility: state.eligibility,
                discarded_account: None,
            },
        };
        let freeze_ready = Self::finish_discard(continuation)?;
        #[cfg(test)]
        let freeze_ready = maybe_drop_failure_guard(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::FreezeReady,
            freeze_ready,
        );
        Ok(freeze_ready.freeze())
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for EligibleRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = contain_eligible_rich_state(
                state,
                AccountPoisonReason::OwnershipGraphUnverified,
                false,
            );
        }
    }
}

fn contain_freeze_ready_rich_state<O, P, D, F, EvidenceBlock>(
    state: FreezeReadyRichState<O, P, D, F, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let FreezeReadyRichState {
        output,
        proof,
        eligibility,
    } = state;
    let QualifiedRichOutput {
        evidence,
        evidence_table,
        account,
        ..
    } = output;
    let physical = TerminalPhysicalOwnershipManifest::<P, D, F, (), EvidenceBlock>::new(
        DetachedRetainedPart::empty(),
        evidence.into_unfrozen_owner(),
        None,
        Some(evidence_table),
        Some(account),
    );
    let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
    proof.retain_fail_closed();
    drop(eligibility);
}

impl<O, P, D, F, EvidenceBlock> FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn freeze<CanonicalBlock>(
        mut self,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let mut state = scopeguard::guard(state, contain_freeze_ready_rich_state);
        let snapshot = state.proof.freeze_in_place();
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::LedgerPublished,
        );
        let terminal_cause = TerminalEvidenceCause::Failure(state.output.cause);
        state
            .evidence_mut()
            .freeze_in_place(terminal_cause, &snapshot);
        let state = scopeguard::ScopeGuard::into_inner(state);
        let FreezeReadyRichState {
            output,
            proof: _,
            eligibility: _,
        } = state;
        let QualifiedRichOutput {
            evidence,
            evidence_table,
            account,
            evidence_root,
            cause: _,
            set_id,
            fallback_type: _,
        } = output;
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::RichFailure {
                outcome_evidence: PublishedOutcomeEvidenceAggregate {
                    inner: PublishedRetainedAggregate {
                        table: evidence_table,
                        set_id,
                    },
                },
                outcome_evidence_root: evidence_root,
                account,
            },
            evidence: PublishedPart::from_exact_owner(evidence.into_frozen_owner()),
        }
    }
}

impl<O, P, D, F, EvidenceBlock> FreezeReadyRichState<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn evidence_mut(&mut self) -> &mut FreezeReadyDetachedTerminalEvidence<P, D> {
        &mut self.output.evidence
    }
}

impl<O, P, D, F, EvidenceBlock> Drop for FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            contain_freeze_ready_rich_state(state);
        }
    }
}

struct TerminalMinimalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
    cause: PreparationFailureCode,
}

struct QualifiedMinimalOutput<P, D, F> {
    fallback: DetachedRetainedPart<F>,
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
    cause: PreparationFailureCode,
}

enum QualifiedMinimalDiscard<O, P, D, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    None {
        proof: FreezeReadyTypedProof<O, P, D>,
    },
    Evidence {
        table: RetainedTableAllocation<EvidenceBlock>,
        account: PhysicalStorageAccount,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
    },
    Both {
        canonical_table: RetainedTableAllocation<CanonicalBlock>,
        evidence_table: RetainedTableAllocation<EvidenceBlock>,
        account: PhysicalStorageAccount,
        proof: QualifiedBothDiscardFreezeProof<O, P, D>,
    },
}

struct QualifiedMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    discard: QualifiedMinimalDiscard<O, P, D, CanonicalBlock, EvidenceBlock>,
}

struct QualifiedMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<QualifiedMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct EligibleMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    discard: QualifiedMinimalDiscard<O, P, D, CanonicalBlock, EvidenceBlock>,
    eligibility: PublicationEligibility,
}

struct EligibleMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibleMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct FreezeReadyMinimalState<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    proof: FreezeReadyTypedProof<O, P, D>,
    eligibility: PublicationEligibility,
}

struct FreezeReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<FreezeReadyMinimalState<O, P, D, F>>,
}

#[allow(
    clippy::type_complexity,
    reason = "the tuple moves one exact physical graph beside its matching typed ledger proof"
)]
fn minimal_state_into_terminal<P, D, F, CanonicalBlock, EvidenceBlock, O>(
    output: QualifiedMinimalOutput<P, D, F>,
    discard: QualifiedMinimalDiscard<O, P, D, CanonicalBlock, EvidenceBlock>,
) -> (
    TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    QualifiedTypedDiscardFreezeProof<O, P, D>,
)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let QualifiedMinimalOutput {
        fallback,
        evidence,
        cause: _,
    } = output;
    match discard {
        QualifiedMinimalDiscard::None { proof } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                None,
                None,
                None,
            ),
            QualifiedTypedDiscardFreezeProof::None(proof),
        ),
        QualifiedMinimalDiscard::Evidence {
            table,
            account,
            proof,
        } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                None,
                Some(table),
                Some(account),
            ),
            QualifiedTypedDiscardFreezeProof::Evidence(proof),
        ),
        QualifiedMinimalDiscard::Both {
            canonical_table,
            evidence_table,
            account,
            proof,
        } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                Some(canonical_table),
                Some(evidence_table),
                Some(account),
            ),
            QualifiedTypedDiscardFreezeProof::Both(proof),
        ),
    }
}

fn contain_qualified_minimal_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: QualifiedMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let (physical, proof) = minimal_state_into_terminal(state.output, state.discard);
    let cause = if already_recorded {
        physical.contain_after_recorded(reason)
    } else {
        physical.contain_new_fault(reason)
    };
    proof.retain_fail_closed();
    cause
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalMinimalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
        cause: PreparationFailureCode,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
            cause,
        }
    }

    fn contain_unqualified(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let cause = physical.contain_new_fault(reason);
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        cause
    }

    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match proof.try_into_qualified_discards() {
            Ok(proof) => proof,
            Err(proof) => {
                self.proof = Some(proof);
                return Err(
                    self.contain_unqualified(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
                );
            }
        };
        if matches!(&proof, QualifiedTypedDiscardFreezeProof::Canonical(_)) {
            let physical = self
                .physical
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let cause = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            return Err(cause);
        }
        let (expect_canonical, expect_evidence) = match &proof {
            QualifiedTypedDiscardFreezeProof::None(_) => (false, false),
            QualifiedTypedDiscardFreezeProof::Evidence(_) => (false, true),
            QualifiedTypedDiscardFreezeProof::Both(_) => (true, true),
            QualifiedTypedDiscardFreezeProof::Canonical(_) => unreachable_final_owner_state(),
        };
        if !self
            .physical
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .qualifies_pre_cleanup_fallback(expect_canonical, expect_evidence)
        {
            let physical = self
                .physical
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let cause = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            return Err(cause);
        }
        let (fallback, evidence, canonical_table, evidence_table, account) = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .into_parts();
        let qualified_evidence =
            match QualifiedDetachedTerminalEvidence::try_from_discard_proof(evidence, &proof) {
                DetachedTerminalEvidenceQualification::Qualified(evidence) => {
                    evidence.into_freeze_ready()
                }
                DetachedTerminalEvidenceQualification::Rejected(evidence)
                | DetachedTerminalEvidenceQualification::Panicked(evidence) => {
                    let physical = TerminalPhysicalOwnershipManifest::new(
                        fallback,
                        evidence,
                        canonical_table,
                        evidence_table,
                        account,
                    );
                    let cause = physical
                        .contain_new_fault(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
                    proof.retain_fail_closed();
                    return Err(cause);
                }
            };
        let output = QualifiedMinimalOutput {
            fallback,
            evidence: qualified_evidence,
            cause: self.cause,
        };
        let discard = match (proof, canonical_table, evidence_table, account) {
            (QualifiedTypedDiscardFreezeProof::None(proof), None, None, None) => {
                QualifiedMinimalDiscard::None { proof }
            }
            (
                QualifiedTypedDiscardFreezeProof::Evidence(proof),
                None,
                Some(table),
                Some(account),
            ) => QualifiedMinimalDiscard::Evidence {
                table,
                account,
                proof,
            },
            (
                QualifiedTypedDiscardFreezeProof::Both(proof),
                Some(canonical_table),
                Some(evidence_table),
                Some(account),
            ) => QualifiedMinimalDiscard::Both {
                canonical_table,
                evidence_table,
                account,
                proof,
            },
            _ => unreachable_final_owner_state(),
        };
        let qualified = QualifiedMinimalGraph {
            state: Some(QualifiedMinimalState { output, discard }),
        };
        #[cfg(test)]
        let qualified = maybe_drop_failure_guard(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::Qualified,
            qualified,
        );
        qualified.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalMinimalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let eligibility = match state
            .output
            .evidence
            .owner
            .account()
            .try_publication_eligibility()
        {
            Ok(eligibility) => eligibility,
            Err(quarantine) => {
                let reason = quarantine.snapshot().first_poison_evidence().map_or(
                    AccountPoisonReason::OwnershipGraphUnverified,
                    AccountPoisonEvidence::reason,
                );
                return Err(contain_qualified_minimal_state(state, reason, true));
            }
        };
        let eligible = EligibleMinimalGraph {
            state: Some(EligibleMinimalState {
                output: state.output,
                discard: state.discard,
                eligibility,
            }),
        };
        #[cfg(test)]
        let eligible = maybe_drop_failure_guard(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::Eligible,
            eligible,
        );
        eligible.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for QualifiedMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = contain_qualified_minimal_state(
                state,
                AccountPoisonReason::OwnershipGraphUnverified,
                false,
            );
        }
    }
}

fn contain_eligible_minimal_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: EligibleMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let EligibleMinimalState {
        output,
        discard,
        eligibility,
    } = state;
    let cause = contain_qualified_minimal_state(
        QualifiedMinimalState { output, discard },
        reason,
        already_recorded,
    );
    drop(eligibility);
    cause
}

enum MinimalDiscardContinuationProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Both(QualifiedBothDiscardFreezeProof<O, P, D>),
    Evidence(QualifiedEvidenceDiscardFreezeProof<O, P, D>),
}

impl<O, P, D> MinimalDiscardContinuationProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn retain_fail_closed(self) {
        match self {
            Self::Both(proof) => proof.retain_fail_closed(),
            Self::Evidence(proof) => proof.retain_fail_closed(),
        }
    }
}

struct MinimalDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    account: PhysicalStorageAccount,
    proof: MinimalDiscardContinuationProof<O, P, D>,
    eligibility: PublicationEligibility,
}

fn contain_minimal_discard_continuation_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: MinimalDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let MinimalDiscardContinuationState {
        output,
        canonical_table,
        evidence_table,
        account,
        proof,
        eligibility,
    } = state;
    let QualifiedMinimalOutput {
        fallback,
        evidence,
        cause: _,
    } = output;
    let physical = TerminalPhysicalOwnershipManifest::new(
        fallback,
        evidence.into_unfrozen_owner(),
        canonical_table,
        evidence_table,
        Some(account),
    );
    let cause = if already_recorded {
        physical.contain_after_recorded(reason)
    } else {
        physical.contain_new_fault(reason)
    };
    proof.retain_fail_closed();
    drop(eligibility);
    cause
}

fn contain_minimal_discard_continuation_on_drop<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: MinimalDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let _ = contain_minimal_discard_continuation_state(
        state,
        AccountPoisonReason::OwnershipGraphUnverified,
        false,
    );
}

fn table_cleanup_reason(
    outcome: RetainedStorageDropOutcome,
) -> Option<(AccountPoisonReason, bool)> {
    match outcome {
        RetainedStorageDropOutcome::Clean => None,
        RetainedStorageDropOutcome::MissingFaultAnchor => {
            Some((AccountPoisonReason::OwnershipGraphUnverified, false))
        }
        RetainedStorageDropOutcome::StorageReleaseUnverified(witness)
        | RetainedStorageDropOutcome::ChargeReleaseUnverified(witness)
        | RetainedStorageDropOutcome::StillSharedOwnershipFault(witness)
        | RetainedStorageDropOutcome::ForeignOwnershipFault(witness) => {
            Some((witness.reason, true))
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    EligibleMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn finish_discard(
        state: MinimalDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Result<FreezeReadyMinimalGraph<O, P, D, F>, FinalizationContainmentError> {
        let mut state = scopeguard::guard(
            state,
            contain_minimal_discard_continuation_on_drop::<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        );
        if matches!(&state.proof, MinimalDiscardContinuationProof::Both(_)) {
            let canonical_table = state
                .canonical_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let outcome = drop_retained_table_with_account_anchor(canonical_table, &state.account);
            if let Some((reason, already_recorded)) = table_cleanup_reason(outcome) {
                return Err(contain_minimal_discard_continuation_state(
                    scopeguard::ScopeGuard::into_inner(state),
                    reason,
                    already_recorded,
                ));
            }
            let evidence_proof = match &mut state.proof {
                MinimalDiscardContinuationProof::Both(proof) => {
                    proof.release_canonical_after_physical_cleanup_in_place()
                }
                MinimalDiscardContinuationProof::Evidence(_) => unreachable_final_owner_state(),
            };
            state.proof = MinimalDiscardContinuationProof::Evidence(evidence_proof);
            #[cfg(test)]
            maybe_panic_failure_guard_stage(
                FailureTerminalRail::Minimal,
                FailureGuardDropStage::CanonicalAcknowledged,
            );
        }

        let evidence_table = state
            .evidence_table
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let outcome = drop_retained_table_with_account_anchor(evidence_table, &state.account);
        if let Some((reason, already_recorded)) = table_cleanup_reason(outcome) {
            return Err(contain_minimal_discard_continuation_state(
                scopeguard::ScopeGuard::into_inner(state),
                reason,
                already_recorded,
            ));
        }
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::DiscardCleaned,
        );
        let freeze_ready = match &mut state.proof {
            MinimalDiscardContinuationProof::Evidence(proof) => {
                proof.release_discard_after_physical_cleanup_in_place()
            }
            MinimalDiscardContinuationProof::Both(_) => unreachable_final_owner_state(),
        };
        let state = scopeguard::ScopeGuard::into_inner(state);
        let MinimalDiscardContinuationState {
            output,
            canonical_table,
            evidence_table,
            account,
            proof: _,
            eligibility,
        } = state;
        drop(canonical_table);
        drop(evidence_table);
        drop(account);
        Ok(FreezeReadyMinimalGraph {
            state: Some(FreezeReadyMinimalState {
                output,
                proof: freeze_ready,
                eligibility,
            }),
        })
    }

    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let freeze_ready = match state.discard {
            QualifiedMinimalDiscard::None { proof } => FreezeReadyMinimalGraph {
                state: Some(FreezeReadyMinimalState {
                    output: state.output,
                    proof,
                    eligibility: state.eligibility,
                }),
            },
            QualifiedMinimalDiscard::Evidence {
                table,
                account,
                proof,
            } => Self::finish_discard(MinimalDiscardContinuationState {
                output: state.output,
                canonical_table: None,
                evidence_table: Some(table),
                account,
                proof: MinimalDiscardContinuationProof::Evidence(proof),
                eligibility: state.eligibility,
            })?,
            QualifiedMinimalDiscard::Both {
                canonical_table,
                evidence_table,
                account,
                proof,
            } => Self::finish_discard(MinimalDiscardContinuationState {
                output: state.output,
                canonical_table: Some(canonical_table),
                evidence_table: Some(evidence_table),
                account,
                proof: MinimalDiscardContinuationProof::Both(proof),
                eligibility: state.eligibility,
            })?,
        };
        #[cfg(test)]
        let freeze_ready = maybe_drop_failure_guard(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::FreezeReady,
            freeze_ready,
        );
        Ok(freeze_ready.freeze())
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for EligibleMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = contain_eligible_minimal_state(
                state,
                AccountPoisonReason::OwnershipGraphUnverified,
                false,
            );
        }
    }
}

fn contain_freeze_ready_minimal_state<O, P, D, F>(state: FreezeReadyMinimalState<O, P, D, F>)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let FreezeReadyMinimalState {
        output,
        proof,
        eligibility,
    } = state;
    let QualifiedMinimalOutput {
        fallback,
        evidence,
        cause: _,
    } = output;
    let physical = TerminalPhysicalOwnershipManifest::<P, D, F, (), ()>::new(
        fallback,
        evidence.into_unfrozen_owner(),
        None,
        None,
        None,
    );
    let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
    proof.retain_fail_closed();
    drop(eligibility);
}

impl<O, P, D, F> FreezeReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn freeze<CanonicalBlock, EvidenceBlock>(
        mut self,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let mut state = scopeguard::guard(state, contain_freeze_ready_minimal_state);
        let snapshot = state.proof.freeze_in_place();
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::LedgerPublished,
        );
        let terminal_cause = TerminalEvidenceCause::Failure(state.output.cause);
        state
            .output
            .evidence
            .freeze_in_place(terminal_cause, &snapshot);
        let state = scopeguard::ScopeGuard::into_inner(state);
        let FreezeReadyMinimalState {
            output,
            proof: _,
            eligibility: _,
        } = state;
        let QualifiedMinimalOutput {
            fallback,
            evidence,
            cause: _,
        } = output;
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::MinimalFallback {
                part: PublishedPart::from_exact_owner(fallback),
            },
            evidence: PublishedPart::from_exact_owner(evidence.into_frozen_owner()),
        }
    }
}

impl<O, P, D, F> Drop for FreezeReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            contain_freeze_ready_minimal_state(state);
        }
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
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
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

    /// Records semantic completion against the operation's frozen phase catalog.
    pub(crate) fn try_complete_phase(&self, phase: P) -> Result<(), PhaseCompletionError<P>> {
        self.coordinator.retained_ledger().try_complete_phase(phase)
    }

    pub fn try_start(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        let phase_catalog = [plan.terminal_writer.phase];
        Self::try_start_inner(
            control,
            ledger_limits,
            &phase_catalog,
            account,
            plan,
            build_fallback,
        )
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
            phase_catalog,
            account,
            plan,
            build_fallback,
        )
    }

    fn try_start_inner(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        phase_catalog: &[P],
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
        let terminal_evidence_backing =
            TerminalEvidenceBackingPlan::<P, D>::try_new(dimension_count, phase_catalog.len())
                .map_err(|_| {
                    RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity)
                })?;
        let terminal_evidence_footprint = terminal_evidence_backing.footprint();
        let evidence_plan = RetainedAllocationPlan::new(
            terminal_evidence_footprint.bytes(),
            terminal_evidence_footprint.allocations(),
        )
        .map_err(|_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity))?;
        let fallback_charge = account
            .checked_retained_charge::<F>(plan.fallback_plan)
            .map_err(RetainedWriterError::Physical)?;
        let evidence_charge = account
            .checked_retained_charge::<TerminalLedgerEvidence<P, D>>(evidence_plan)
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
        let coordinator = match OperationCoordinator::try_new(control, ledger_limits, phase_catalog)
        {
            Ok(coordinator) => coordinator,
            Err(cause) => {
                account_admission.release();
                return Err(RetainedOperationStartError::Writer(
                    RetainedWriterError::Ledger(cause),
                ));
            }
        };
        let ledger = coordinator.retained_ledger();
        if !terminal_evidence_catalog_is_qualified(ledger, dimension_count, phase_catalog.len()) {
            account_admission.release();
            return Err(RetainedOperationStartError::TerminalEvidenceCatalogInvariant);
        }
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
                        return Err(StartupTerminalEvidenceBuildError::Rejected(cause));
                    }
                    if take_injected_start_evidence_structural_failure() {
                        return Err(StartupTerminalEvidenceBuildError::StructuralInvariant);
                    }
                }
                terminal_evidence_backing
                    .try_build_indexed(
                        |index| ledger.dimension_at(index),
                        |index| ledger.phase_at(index),
                    )
                    .map_err(StartupTerminalEvidenceBuildError::from)
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
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
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
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
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
        let ledger = self
            .operation
            .as_ref()
            .ok_or(RetainedPublicationError::Poisoned)?
            .coordinator
            .retained_ledger();
        let batch = self
            .canonical_batch
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?;
        ledger
            .try_push_commitment(batch, OutputCommitment::new(dimension, amount))
            .map_err(map_commitment_push_error)
    }

    pub fn try_commit_outcome_evidence(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), RetainedPublicationError<D>> {
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        let ledger = self
            .operation
            .as_ref()
            .ok_or(RetainedPublicationError::Poisoned)?
            .coordinator
            .retained_ledger();
        let batch = self
            .evidence_batch
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?;
        ledger
            .try_push_commitment(batch, OutputCommitment::new(dimension, amount))
            .map_err(map_commitment_push_error)
    }

    /// Discards every block table before attribution and publishes the pre-admitted fallback.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
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
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
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
        match transition {
            TerminalPrimaryTransition::Success(success) => success
                .into_prepared(self.set_id)
                .map_err(RetainedPublicationPrepareError::FinalizationContainment),
            TerminalPrimaryTransition::Rich(rich) => rich
                .into_prepared(self.set_id)
                .map_err(RetainedPublicationPrepareError::FinalizationContainment),
            TerminalPrimaryTransition::Fallback(fallback_guard) => {
                fallback_guard.try_prepare(PreparationFailureCode::ledger())
            }
        }
    }
}

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
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
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
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
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
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
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
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
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
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
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
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub fn try_prepare(
        self,
        cause: PreparationFailureCode,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
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
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly.prepare_minimal_fallback()
    }
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedFallbackOnly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    /// Discards candidate tables and prepares only the pre-admitted terminal baseline.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
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
    fn try_bind(&mut self, publish_canonical: bool) -> Result<(), LedgerCause<D>> {
        self.ledger.try_bind_primary(
            &mut self.baseline,
            self.batches.canonical.as_mut(),
            self.batches.evidence.as_mut(),
            publish_canonical,
        )
    }

    fn finish(self, publish_canonical: bool) -> crate::ledger::TypedFreezeProof<O, P, D> {
        self.ledger.finish_bound_primary(
            self.baseline,
            self.batches.canonical,
            self.batches.evidence,
            publish_canonical,
        )
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
    fn try_prepare_sources(&mut self) -> Result<(), LedgerCause<D>> {
        self.ledger.try_bind_fallback_discards(
            &mut self.baseline,
            self.discard.canonical.as_mut(),
            self.discard.evidence.as_mut(),
        )
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
    Success(TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
    Rich(TerminalRichFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
    Fallback(TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
}

struct TerminalPrimaryGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    attribution: Option<TerminalPrimaryAttribution<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPrimaryGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        ledger: Ledger<O, P, D>,
        baseline: FallbackBatchBaseline<O, P, D>,
        batches: PrimaryPublicationBatches<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            attribution: Some(TerminalPrimaryAttribution {
                ledger,
                baseline,
                batches,
            }),
            physical: Some(physical),
        }
    }

    fn attribution_mut(&mut self) -> &mut TerminalPrimaryAttribution<O, P, D> {
        match self.attribution.as_mut() {
            Some(attribution) => attribution,
            None => unreachable_final_owner_state(),
        }
    }

    fn physical(
        &self,
    ) -> &TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
        match self.physical.as_ref() {
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
        let publish_canonical = matches!(terminal, RetainedTerminalMode::Success { .. });
        let bind_result = catch_unwind(AssertUnwindSafe(|| {
            self.attribution_mut().try_bind(publish_canonical)
        }));
        match bind_result {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                self.physical()
                    .record_new_fault(AccountPoisonReason::PrimaryPublicationBindingPanicked);
                let containment = self
                    .contain_after_recorded(AccountPoisonReason::PrimaryPublicationBindingPanicked);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
            Ok(Err(_cause)) => {
                return Ok(TerminalPrimaryTransition::Fallback(self.into_fallback()));
            }
            Ok(Ok(())) => {}
        }

        if let RetainedTerminalMode::Success {
            canonical_root,
            evidence_root,
        } = terminal
        {
            if !self.physical().qualifies_pre_discard_success() {
                let containment =
                    self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
            let physical = self
                .physical
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state())
                .finish_pre_discard_success(canonical_root, evidence_root);
            let proof = self
                .attribution
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state())
                .finish(true);
            return Ok(TerminalPrimaryTransition::Success(
                TerminalSuccessFreezeGuard::new(proof, physical),
            ));
        }

        let (evidence_root, cause) = match terminal {
            RetainedTerminalMode::RichFailure {
                evidence_root,
                cause,
            } => (evidence_root, cause),
            RetainedTerminalMode::Success { .. } | RetainedTerminalMode::MinimalFallback { .. } => {
                unreachable_final_owner_state()
            }
        };
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match self.attribution.take() {
            Some(attribution) => attribution.finish(false),
            None => unreachable_final_owner_state(),
        };
        Ok(TerminalPrimaryTransition::Rich(
            TerminalRichFreezeGuard::new(proof, physical, evidence_root, cause),
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
    }
}

struct TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    attribution: Option<TerminalFallbackAttribution<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        attribution: TerminalFallbackAttribution<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            attribution: Some(attribution),
            physical: Some(physical),
        }
    }

    fn attribution_mut(&mut self) -> &mut TerminalFallbackAttribution<O, P, D> {
        match self.attribution.as_mut() {
            Some(attribution) => attribution,
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
        cause
    }

    fn try_prepare(
        mut self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        match catch_unwind(AssertUnwindSafe(|| {
            self.attribution_mut().try_prepare_sources()
        })) {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
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

        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let proof = match self.attribution.take() {
            Some(attribution) => attribution.finish(),
            None => unreachable_final_owner_state(),
        };
        TerminalMinimalFreezeGuard::new(proof, physical, cause)
            .into_prepared()
            .map_err(RetainedPublicationPrepareError::FinalizationContainment)
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
impl PreparationFailureCode {
    fn from_control(cause: &ControlCause) -> Self {
        let code = match cause {
            ControlCause::Cancelled { .. } => 1,
            ControlCause::DeadlineExceeded => 2,
            ControlCause::ObserverPanicked => 3,
            ControlCause::CancellationSignalPanicked => 4,
            ControlCause::InvalidProgress { .. } => 5,
            ControlCause::ProgressStorage { .. } => 6,
        };
        Self::control(code)
    }

    const fn from_closure_validation(cause: RetainedClosureValidationError) -> Self {
        let code = match cause {
            RetainedClosureValidationError::Poisoned => 1,
            RetainedClosureValidationError::WrongOutcomeMode => 2,
            RetainedClosureValidationError::InvalidRoot => 3,
            RetainedClosureValidationError::Rejected => 4,
            RetainedClosureValidationError::Panicked => 5,
        };
        Self::product_graph(code)
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
    },
    MinimalFallback {
        part: PublishedPart<F>,
    },
}

/// Prepared terminal publication after one atomic ledger transition.
///
/// Success freezes canonical and evidence destinations, rich failure freezes evidence only, and
/// minimal fallback freezes the pre-admitted fallback/evidence baseline. Handoff is a pure
/// allocation-free ownership move.
#[must_use = "prepared publication owns terminal output and frozen evidence until handoff"]
pub(crate) struct PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
    payload: RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F>,
    evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

/// Exact terminal classification consumed before any domain prepared wrapper is constructed.
pub(crate) enum PreparedRetainedPublicationOutcome<CanonicalBlock, EvidenceBlock, F, P, D> {
    Success(PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D>),
    RichFailure(PreparedRetainedRichFailure<EvidenceBlock, P, D>),
    MinimalFallback(PreparedRetainedFallback<F, P, D>),
}

/// Exact prepared success. Its retained roots remain opaque until handoff.
pub(crate) struct PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D> {
    canonical: PublishedCanonicalAggregate<CanonicalBlock>,
    outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    canonical_root: RetainedSlotIdentity,
    outcome_evidence_root: RetainedSlotIdentity,
    account: PhysicalStorageAccount,
    evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

/// Exact prepared rich failure. Its retained evidence remains opaque until handoff.
pub(crate) struct PreparedRetainedRichFailure<EvidenceBlock, P, D> {
    outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    outcome_evidence_root: RetainedSlotIdentity,
    account: PhysicalStorageAccount,
    evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

/// Exact prepared minimal fallback. Its payload remains opaque until handoff.
pub(crate) struct PreparedRetainedFallback<F, P, D> {
    part: PublishedPart<F>,
    evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

impl<CanonicalBlock, EvidenceBlock, F, P, D>
    PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Consumes the unclassified private owner into one exact prepared terminal state.
    pub fn into_outcome(
        self,
    ) -> PreparedRetainedPublicationOutcome<CanonicalBlock, EvidenceBlock, F, P, D> {
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
            } => PreparedRetainedPublicationOutcome::RichFailure(PreparedRetainedRichFailure {
                outcome_evidence,
                outcome_evidence_root,
                account,
                evidence,
            }),
            RetainedPublicationPayload::MinimalFallback { part } => {
                PreparedRetainedPublicationOutcome::MinimalFallback(PreparedRetainedFallback {
                    part,
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
}

impl<EvidenceBlock> PublishedRichFailureOutput<EvidenceBlock> {
    pub fn outcome_evidence(&self) -> &PublishedOutcomeEvidenceOutput<EvidenceBlock> {
        &self.outcome_evidence
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        &self.account
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
pub(crate) struct PublishedLedgerEvidence<P, D> {
    evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

impl<P, D> PublishedLedgerEvidence<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    pub fn header(&self) -> Option<FallbackHeaderSnapshot<P>> {
        self.evidence.allocation().as_ref().header()
    }

    pub fn terminal_cause(&self) -> Option<TerminalEvidenceCause> {
        self.header().map(FallbackHeaderSnapshot::terminal_cause)
    }

    pub fn dimension(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.evidence.allocation().as_ref().dimension(dimension)
    }

    pub fn phase(&self, phase: P) -> Option<PhaseSnapshot> {
        self.evidence.allocation().as_ref().phase(phase)
    }

    pub fn dimension_key_at(&self, index: usize) -> Option<D> {
        self.evidence.allocation().as_ref().dimension_key_at(index)
    }

    pub fn phase_key_at(&self, index: usize) -> Option<P> {
        self.evidence.allocation().as_ref().phase_key_at(index)
    }
}

/// Pre-admitted fallback owner after a failed terminal publication.
pub(crate) struct PublishedFallbackOutput<F> {
    part: PublishedPart<F>,
}

impl<F> PublishedFallbackOutput<F> {
    pub fn value(&self) -> &F {
        self.part.allocation().as_ref()
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        self.part.account()
    }
}

/// Exact published success owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedSuccessParts<CanonicalBlock, EvidenceBlock, P, D> {
    pub(crate) canonical: PublishedCanonicalOutput<CanonicalBlock>,
    pub(crate) outcome_evidence: PublishedOutcomeEvidenceOutput<EvidenceBlock>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<P, D>,
}

/// Exact published rich-failure owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedRichFailureParts<EvidenceBlock, P, D> {
    pub(crate) outcome_evidence: PublishedRichFailureOutput<EvidenceBlock>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<P, D>,
}

/// Exact published minimal-fallback owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedFallbackParts<F, P, D> {
    pub(crate) fallback: PublishedFallbackOutput<F>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<P, D>,
}

impl<CanonicalBlock, EvidenceBlock, P, D>
    PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Performs a pure ownership handoff directly into exact success owners.
    pub fn handoff(self) -> PublishedRetainedSuccessParts<CanonicalBlock, EvidenceBlock, P, D> {
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

impl<EvidenceBlock, P, D> PreparedRetainedRichFailure<EvidenceBlock, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Performs a pure ownership handoff directly into exact rich-failure owners.
    pub fn handoff(self) -> PublishedRetainedRichFailureParts<EvidenceBlock, P, D> {
        let Self {
            outcome_evidence,
            outcome_evidence_root,
            account,
            evidence,
        } = self;
        PublishedRetainedRichFailureParts {
            outcome_evidence: PublishedRichFailureOutput {
                outcome_evidence: PublishedOutcomeEvidenceOutput {
                    aggregate: outcome_evidence,
                    root: outcome_evidence_root,
                },
                account,
            },
            ledger_evidence: PublishedLedgerEvidence { evidence },
        }
    }
}

impl<F, P, D> PreparedRetainedFallback<F, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Performs a pure ownership handoff directly into exact fallback owners.
    pub fn handoff(self) -> PublishedRetainedFallbackParts<F, P, D> {
        let Self { part, evidence } = self;
        PublishedRetainedFallbackParts {
            fallback: PublishedFallbackOutput { part },
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

fn terminal_evidence_catalog_is_qualified<O, P, D>(
    ledger: &Ledger<O, P, D>,
    expected_dimensions: usize,
    expected_phases: usize,
) -> bool
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    if ledger.dimension_count() != expected_dimensions || ledger.phase_count() != expected_phases {
        return false;
    }

    let mut previous_dimension = None;
    for index in 0..expected_dimensions {
        let Some(dimension) = ledger.dimension_at(index) else {
            return false;
        };
        if previous_dimension.is_some_and(|previous| previous >= dimension) {
            return false;
        }
        previous_dimension = Some(dimension);
    }

    let mut previous_phase = None;
    for index in 0..expected_phases {
        let Some(phase) = ledger.phase_at(index) else {
            return false;
        };
        if previous_phase.is_some_and(|previous| previous >= phase) {
            return false;
        }
        previous_phase = Some(phase);
    }
    true
}

struct QualifiedDetachedTerminalEvidence<P, D> {
    owner: Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>,
    qualification: Option<TerminalEvidenceFreezeQualification>,
}

enum DetachedTerminalEvidenceQualification<P, D> {
    Qualified(QualifiedDetachedTerminalEvidence<P, D>),
    Rejected(DetachedRetainedPart<TerminalLedgerEvidence<P, D>>),
    Panicked(DetachedRetainedPart<TerminalLedgerEvidence<P, D>>),
}

impl<P, D> QualifiedDetachedTerminalEvidence<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn try_from_success_proof<O>(
        owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
        proof: &QualifiedEvidenceDiscardFreezeProof<O, P, D>,
    ) -> DetachedTerminalEvidenceQualification<P, D>
    where
        O: OperationOwner,
    {
        let qualification = catch_unwind(AssertUnwindSafe(|| {
            owner.allocation().as_ref().try_qualify(
                proof.dimension_count(),
                |index| proof.dimension_at(index),
                proof.phase_count(),
                |index| proof.phase_at(index),
                proof.last_completed_phase(),
            )
        }));
        match qualification {
            Ok(Ok(qualification)) => DetachedTerminalEvidenceQualification::Qualified(Self {
                owner: Some(owner),
                qualification: Some(qualification),
            }),
            Ok(Err(_)) => DetachedTerminalEvidenceQualification::Rejected(owner),
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                DetachedTerminalEvidenceQualification::Panicked(owner)
            }
        }
    }

    fn try_from_discard_proof<O>(
        owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
        proof: &QualifiedTypedDiscardFreezeProof<O, P, D>,
    ) -> DetachedTerminalEvidenceQualification<P, D>
    where
        O: OperationOwner,
    {
        let qualification = catch_unwind(AssertUnwindSafe(|| {
            owner.allocation().as_ref().try_qualify(
                proof.dimension_count(),
                |index| proof.dimension_at(index),
                proof.phase_count(),
                |index| proof.phase_at(index),
                proof.last_completed_phase(),
            )
        }));
        match qualification {
            Ok(Ok(qualification)) => DetachedTerminalEvidenceQualification::Qualified(Self {
                owner: Some(owner),
                qualification: Some(qualification),
            }),
            Ok(Err(_)) => DetachedTerminalEvidenceQualification::Rejected(owner),
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                DetachedTerminalEvidenceQualification::Panicked(owner)
            }
        }
    }
}

impl<P, D> QualifiedDetachedTerminalEvidence<P, D> {
    fn into_freeze_ready(mut self) -> FreezeReadyDetachedTerminalEvidence<P, D> {
        let qualification = self
            .qualification
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let owner = self
            .owner
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        FreezeReadyDetachedTerminalEvidence {
            owner,
            qualification,
        }
    }

    fn into_unfrozen_owner(mut self) -> DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        self.owner
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }
}

impl<P, D> FreezeReadyDetachedTerminalEvidence<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn freeze_in_place(
        &mut self,
        terminal_cause: TerminalEvidenceCause,
        snapshot: &crate::ledger::LedgerSnapshot<P, D>,
    ) {
        self.owner
            .allocation()
            .as_ref()
            .freeze_from(&self.qualification, terminal_cause, snapshot);
    }
}

impl<P, D> FreezeReadyDetachedTerminalEvidence<P, D> {
    fn into_frozen_owner(self) -> DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        self.owner
    }

    fn into_unfrozen_owner(self) -> DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        self.owner
    }
}

impl<P, D> Drop for QualifiedDetachedTerminalEvidence<P, D> {
    fn drop(&mut self) {
        if let Some(mut owner) = self.owner.take() {
            owner.record_forgotten_physical_edge();
            owner.fail_closed_after_recorded_poison();
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuccessGuardDropStage {
    Qualified,
    Eligible,
    Verified,
    FreezeReady,
    VerifiedInProgress,
    LedgerPublished,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureTerminalRail {
    Rich,
    Minimal,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureGuardDropStage {
    Qualified,
    Eligible,
    CanonicalAcknowledged,
    DiscardCleaned,
    FreezeReady,
    LedgerPublished,
}

#[cfg(test)]
#[derive(Debug)]
struct InjectedTerminalGuardDrop;

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
    static FAIL_NEXT_START_EVIDENCE_STRUCTURAL: std::cell::Cell<bool> = const {
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
    static DROP_NEXT_SUCCESS_GUARD: std::cell::Cell<Option<SuccessGuardDropStage>> = const {
        std::cell::Cell::new(None)
    };
    static DROP_NEXT_FAILURE_GUARD: std::cell::Cell<
        Option<(FailureTerminalRail, FailureGuardDropStage)>
    > = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn drop_next_success_guard(stage: SuccessGuardDropStage) {
    DROP_NEXT_SUCCESS_GUARD.with(|slot| slot.set(Some(stage)));
}

#[cfg(test)]
fn maybe_drop_success_guard<T>(stage: SuccessGuardDropStage, guard: T) -> T {
    if take_success_guard_injection(stage) {
        drop(guard);
        std::panic::panic_any(InjectedTerminalGuardDrop);
    }
    guard
}

#[cfg(test)]
fn maybe_panic_success_guard_stage(stage: SuccessGuardDropStage) {
    if take_success_guard_injection(stage) {
        std::panic::panic_any(InjectedTerminalGuardDrop);
    }
}

#[cfg(test)]
fn take_success_guard_injection(stage: SuccessGuardDropStage) -> bool {
    DROP_NEXT_SUCCESS_GUARD.with(|slot| {
        if slot.get() == Some(stage) {
            slot.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
fn drop_next_failure_guard(rail: FailureTerminalRail, stage: FailureGuardDropStage) {
    DROP_NEXT_FAILURE_GUARD.with(|slot| slot.set(Some((rail, stage))));
}

#[cfg(test)]
fn maybe_drop_failure_guard<T>(
    rail: FailureTerminalRail,
    stage: FailureGuardDropStage,
    guard: T,
) -> T {
    if take_failure_guard_injection(rail, stage) {
        drop(guard);
        std::panic::panic_any(InjectedTerminalGuardDrop);
    }
    guard
}

#[cfg(test)]
fn maybe_panic_failure_guard_stage(rail: FailureTerminalRail, stage: FailureGuardDropStage) {
    if take_failure_guard_injection(rail, stage) {
        std::panic::panic_any(InjectedTerminalGuardDrop);
    }
}

#[cfg(test)]
fn take_failure_guard_injection(rail: FailureTerminalRail, stage: FailureGuardDropStage) -> bool {
    DROP_NEXT_FAILURE_GUARD.with(|slot| {
        if slot.get() == Some((rail, stage)) {
            slot.set(None);
            true
        } else {
            false
        }
    })
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
fn fail_next_start_evidence_structural() {
    FAIL_NEXT_START_EVIDENCE_STRUCTURAL.with(|failure| failure.set(true));
}

#[cfg(test)]
fn take_injected_start_evidence_build_panic() -> bool {
    PANIC_NEXT_START_EVIDENCE_BUILD.with(|panic| panic.replace(false))
}

#[cfg(test)]
fn take_injected_start_evidence_structural_failure() -> bool {
    FAIL_NEXT_START_EVIDENCE_STRUCTURAL.with(|failure| failure.replace(false))
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
