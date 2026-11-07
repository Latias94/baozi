//! Fallible physical storage accounting for retained operation allocations.

#![allow(
    clippy::result_large_err,
    reason = "bounded poison snapshots stay inline so containment never needs a failure-path allocation"
)]

mod account;
mod allocation;
mod block_table;
mod evidence;

#[cfg(test)]
pub(crate) fn fail_next_canonical_joint_evidence_qualification() {
    evidence::fail_next_terminal_evidence_qualification();
}

#[cfg(test)]
pub(crate) use terminal::panic_next_canonical_primary_bind;
mod final_owner;
pub(crate) mod operation;
mod operation_start;
mod ordinary;
pub(crate) use operation::{RetainedAssemblyOpened, RetainedPublicationBorrowFailure};
pub(crate) use ordinary::{RetainedAssemblyDecision, RetainedPublicationAssemblyBorrow};
mod ownership;
mod physical;
mod publication_contract;
mod range;
mod startup;
mod terminal;
mod terminal_failure;
mod terminal_finite;
pub(crate) use terminal_finite::{
    FiniteRetainedOutput, FiniteRetainedOutputPayload, PreparedFiniteRetainedPublication,
};
mod terminal_loan;
mod terminal_output;
mod terminal_owned;
mod terminal_physical;
pub(crate) use terminal_owned::DeferredAttachedTerminal;
use terminal_owned::{DynamicTerminalCapsule, TerminalDisposition};

use operation::TerminalInvocationLoan;
pub(crate) use operation::{AttachedTerminalDecision, AttachedTerminalStamp};
pub(crate) use operation::{DeferredUnattachedBinding, InvocationRoot, PendingRetainedAttachment};

#[cfg(test)]
use crate::OperationControl;
use crate::control::discard_untrusted_panic_payload;
use crate::coordinator::FinalProgress;
use crate::ledger::CommitmentPairPushError;
#[cfg(test)]
use crate::ledger::{AttributionBatchCreateError, LedgerLimits};
use crate::ledger::{
    AttributionReservationBatch, CanonicalOutput, CommitmentPushError, DimensionSnapshot,
    FreezeReadyTypedProof, Ledger, LedgerCause, OutcomeEvidenceDestination, OutputCommitment,
    PhaseSnapshot, QualifiedBothDiscardFreezeProof, QualifiedEvidenceDiscardFreezeProof,
    QualifiedTypedDiscardFreezeProof,
};
use crate::{ControlCause, OperationOwner, ProgressPhase, ProgressUnit};
pub use account::{
    AccountPoisonEvidence, AccountPoisonReason, AccountPoisoned, FinalizationContainmentError,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountFactorySnapshot,
    PhysicalStorageAccountLimits, PhysicalStorageAccountSnapshot, PhysicalStorageLimitError,
};
use account::{
    AdmissionCharge, AdmissionError, CounterState, lock_recovering_poison,
    quarantined_account_reason, release, release_locked, try_admit, try_admit_locked,
};
pub(crate) use allocation::RetainedStorageLease;
use allocation::{
    AccountPoisonEventWitness, AdmissionLease, AdmissionOwner, FinalOwnerReleaseOutcome,
    PolicyRetainedAllocation, RetainedAllocation, RetainedLeaseGuard,
    RetainedMembershipDestination, RetainedPayload, RetainedSetId, RetainedValuePolicy,
    map_retained_admission_error, record_account_poison_event, try_next_retained_set_id,
};
pub use allocation::{RetainedStorage, SharedRetainedStorage};
pub(crate) fn release_owned_storage<T>(lease: RetainedStorageLease, value: T) -> bool {
    matches!(
        lease.bind(value).release_before_attribution(),
        FinalOwnerReleaseOutcome::Clean
    )
}

/// The logical reservation and its corresponding admitted physical lease.
type RetainedStorageAdmission<O, P, D> = (
    crate::ledger::LiveReservation<O, P, D>,
    RetainedStorageLease,
);

/// The split work owner uses the existing logical/physical admission and share executor.
/// `admit_share` is a crate-owned numeric transaction, never an extension callback.
pub(crate) fn reserve_exclusive_backing<O, P, D, W>(
    storage: &mut crate::ledger::ExclusiveStorage<'_, O, P, D>,
    account: &PhysicalStorageAccount,
    phase: P,
    dimension: D,
    bytes: u64,
    allocations: u64,
    admit_share: impl FnOnce() -> Result<W, RetainedWriterError<D>>,
) -> Result<RetainedStorageAdmission<O, P, D>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
    W: AdmittedShareWork<D>,
{
    let preflight = account
        .try_begin_account_preflight()
        .map_err(RetainedWriterError::AccountPoisoned)?;
    let reservation = storage
        .reserve(phase, dimension, bytes)
        .map_err(RetainedWriterError::Ledger)?;
    let action = preflight
        .admit_action()
        .map_err(map_account_action_admission_error)?;
    let share = match catch_unwind(AssertUnwindSafe(admit_share)) {
        Ok(result) => result?,
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            return Err(RetainedWriterError::AccountPoisoned(poisoned));
        }
    };
    let mut admission = account
        .try_admit_retained_after_action(action, AdmissionCharge::new(bytes, allocations), share)
        .map_err(|cause| match cause {
            RetainedShareAdmissionError::Physical(cause) => RetainedWriterError::Physical(cause),
            RetainedShareAdmissionError::Ledger(cause) => RetainedWriterError::Ledger(cause),
        })?;
    Ok((
        reservation,
        RetainedStorageLease::from_admission(&mut admission),
    ))
}

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
#[cfg(test)]
pub(crate) use allocation::{
    reset_retained_final_owner_allocator_entries, test_retained_final_owner_allocator_entries,
};
pub(super) use block_table::RetainedSlotIdentity;
#[cfg(test)]
use block_table::try_share_retained_block_after_admission;
pub(crate) use block_table::{
    BlockEmplacer, BlockPlacement, CanonicalLeafInheritanceError, CanonicalRetainedLeafPolicy,
    CanonicalRetainedRewriteMap, CanonicalRetainedSlot, CanonicalSourceBindError,
    OutcomeEvidenceRetainedSlot, PublishedCanonicalAggregate, PublishedOutcomeEvidenceAggregate,
    RetainedBlockTablePlan, RetainedClosureSession, RetainedEvidenceClosureSession,
    RetainedLookupMember,
};
use block_table::{
    QualifiedRetainedBlockTablePlan, RetainedBlockOrigin, RetainedTableAllocation,
    record_forgotten_table_edges, retained_block_table_is_exact, try_bind_canonical_source_mapping,
    try_create_retained_block_table, try_inherit_all_canonical_leaves,
    try_qualify_retained_block_table_plan, try_retain_block, try_retain_rewritten_canonical_block,
    try_rewrite_canonical_block_controlled, try_share_retained_block,
};
pub(crate) use block_table::{
    RetainedCanonicalMemberVisit, RetainedEvidenceMemberVisit, RetainedOutcomeEvidenceMemberVisit,
    RetainedPendingCanonicalMember, RetainedPendingEvidenceMember,
    RetainedPendingOutcomeEvidenceMember,
};
pub(crate) use evidence::{
    FallbackHeaderSnapshot, PreparationFailureClass, PreparationFailureCode,
    TERMINAL_LEDGER_EVIDENCE_SCHEMA_REVISION, TerminalEvidenceCause, TerminalEvidenceKey,
    TerminalEvidenceKeySeal,
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
};
#[cfg(test)]
use ownership::{RetainedStorageDropSeverity, join_pre_attribution_cleanup_cause};
#[cfg(test)]
use physical::AccountControlInner;
pub(crate) use physical::DetachedLedgerCommitPermit;
#[cfg(test)]
use physical::RetainedAdmissionPairPlan;
#[cfg(test)]
use physical::{
    AccountActionAdmissionError, AdmittedRetainedShareWork, PendingRetainedRollback,
    PendingStartRetainedAdmissions,
};
use physical::{
    AccountBoundJointCommit, AccountBoundPublicationManifest, AccountLinearState,
    PendingAccountActionAdmission, PendingRetainedAdmission, RetainedShareAdmissionError,
};
pub use physical::{
    PhysicalStorageAccount, PhysicalStorageAccountCreateError, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryCreateError,
};
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard};
pub(crate) use terminal::RetainedPublicationPrepareError;
pub(crate) use terminal::RetainedSuccessJointCommitIntent;
use terminal::{
    BorrowedDetachedTerminalEvidenceQualification, BorrowedFreezeReadyDetachedTerminalEvidence,
    BorrowedQualifiedDetachedTerminalEvidence, EvidenceOnlySuccessPrimaryTransition,
    FreezeReadyDetachedTerminalEvidence, PrimaryPublicationBatches, TerminalFallbackAttribution,
    TerminalFallbackGuard, TerminalPreFreezeTransition, TerminalPrimaryGuard,
    drop_retained_table_with_account_anchor, terminal_evidence_catalog_is_qualified,
};
use terminal_failure::{TerminalMinimalFreezeGuard, TerminalRichFreezeGuard};
use terminal_loan::{
    BorrowedExactPreDiscardSuccess, BorrowedTerminalPhysicalOwnershipManifest, TerminalAccountLoan,
    TerminalPartLoan, TerminalTableLoan,
};
use terminal_output::RetainedPublicationPayload;
#[allow(
    unused_imports,
    reason = "the retained root preserves crate-private paths while canonical consumers migrate"
)]
pub(crate) use terminal_output::{
    PreparedRetainedEvidenceOnlyOutcome, PreparedRetainedEvidenceOnlySuccess,
    PreparedRetainedFallback, PreparedRetainedPublication, PreparedRetainedPublicationOutcome,
    PreparedRetainedRichFailure, PreparedRetainedSuccess, PublishedCanonicalOutput,
    PublishedFallbackOutput, PublishedLedgerEvidence, PublishedOutcomeEvidenceOutput,
    PublishedRetainedEvidenceOnlySuccessParts, PublishedRetainedFallbackParts,
    PublishedRetainedRichFailureParts, PublishedRetainedSuccessParts, PublishedRichFailureOutput,
};
use terminal_physical::{FiniteTerminalPhysicalState, TerminalPhysicalState};
use triomphe::{Arc, UniqueArc};

#[cfg(test)]
pub(crate) use operation::panic_next_publication_account_open;
pub(crate) use operation::{
    BorrowedRetainedWorkPermission, ExclusiveOperationParts, ExecutionError,
    ExecutionOrdinaryStorage, ORDINARY_WORK_PERMIT_ABANDONED_CODE, RetainedFailureWorkConsumeError,
    RetainedFailureWorkFinishError, RetainedFailureWorkPermission, RetainedFailureWorkStartError,
    RetainedPlanningBaseline, RetainedPlanningContainmentAnchor, RetainedProducerHost,
    RetainedProducerWorkStartError, RetainedWorkConsumeError, RetainedWorkFinishError,
    RetainedWorkPermission, RetainedWorkStartError, StoreExecutionState, check_execution_state,
    checked_action_debt, poll_execution, record_terminal_cause_with_precedence,
};
pub(crate) use operation::{
    RetainedControlCheckpoint, RetainedFailureWorkControl, RetainedOperation,
    RetainedOperationLoan, RetainedWorkBinding, RetainedWorkControl, RetainedWorkRoot,
};
pub(crate) use operation_start::AdmittedShareWork;
#[cfg(test)]
use operation_start::map_qualified_startup_batch_error;
pub(crate) use operation_start::{
    FiniteRetainedOperationStartPlan, RetainedBatchLimits, RetainedFailureTerminalWorkPlan,
    RetainedHandleSharePlan, RetainedOperationStartError, RetainedOperationStartPlan,
    RetainedWriterError, RetainedWriterPlan,
};
use operation_start::{
    StartupRetainedPartGuard, StartupTerminalEvidenceBuildError,
    admit_internal_share_before_physical, map_account_action_admission_error, map_batch_error,
    map_faulted_pending_rollback, map_pending_rollback_or, try_open_publication_account,
};
use publication_contract::RetainedPublicationOpenDisposition;
pub(super) use publication_contract::retained_publication_error_requires_containment;
use publication_contract::{
    AssemblyContainmentProvenance, RetainedClosureValidationError, RetainedDeferredStage,
    RetainedOutcomeMode, RetainedTerminalMode, map_commitment_push_error,
    retained_publication_error_containment,
};
pub(crate) use publication_contract::{
    RetainedEvidenceClosureProof, RetainedPublicationError, RetainedPublicationOpenError,
    RetainedPublicationOpenRecovery, RetainedPublicationPlan, RetainedSuccessClosureProof,
    RetainedTerminalRequest,
};
use range::{CanonicalBatchPreflightContext, try_preflight_canonical_batch};
pub(crate) use range::{
    CanonicalRetainBatchError, CanonicalRetainBatchSource, CanonicalRetainedRange,
};
pub(crate) use startup::{AdmittedStartupLedgerGraph, ReadyStartupGraphSplitAuthority};
use startup::{
    StartupCrossPlaneAdmissionFailure, StartupCrossPlanePlan, StartupPhysicalManifest,
    StartupPhysicalNoMutation,
};
#[cfg(test)]
pub(crate) use terminal::panic_next_evidence_only_primary_bind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CanonicalLineageAuthority {
    Fresh,
    Derived {
        source_set: RetainedSetId,
        source_len: usize,
    },
}

impl CanonicalLineageAuthority {
    pub(super) const fn fresh() -> Self {
        Self::Fresh
    }

    pub(super) fn derived<Block>(source: &PublishedCanonicalAggregate<Block>) -> Self {
        Self::Derived {
            source_set: source.set_id(),
            source_len: source.len(),
        }
    }

    pub(super) const fn source_mapping_capacity(self) -> Option<usize> {
        match self {
            Self::Fresh => None,
            Self::Derived { source_len, .. } => Some(source_len),
        }
    }

    // Linearize account health and exact source identity before work or publication mutation.
    fn require_source<D, Block>(
        self,
        account: &PhysicalStorageAccount,
        source: &PublishedCanonicalAggregate<Block>,
    ) -> Result<(), RetainedPublicationError<D>> {
        let preflight = account.try_begin_account_preflight().map_err(|cause| {
            RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(cause))
        })?;
        match self {
            Self::Derived {
                source_set,
                source_len,
            } if source.belongs_to(account) && source_set == source.set_id() => {
                if source_len != source.len() {
                    return Err(RetainedPublicationError::Writer(
                        RetainedWriterError::AccountPoisoned(preflight.poison_ownership_graph()),
                    ));
                }
                Ok(())
            }
            Self::Fresh | Self::Derived { .. } => Err(RetainedPublicationError::InvalidSourceSlot),
        }
    }
}

/// Actual publication tables, charges and plans owned by the outer invocation.
pub(super) struct PublicationAssemblyFields<O, P, D, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    canonical_batch: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_batch: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    account: Option<PhysicalStorageAccount>,
    canonical_lineage: CanonicalLineageAuthority,
    set_id: RetainedSetId,
    canonical_plan: Option<RetainedBlockTablePlan<P, D>>,
    evidence_plan: RetainedBlockTablePlan<P, D>,
    ordinary_handle_share_work: RetainedHandleSharePlan<D>,
    failure_evidence_handle_share_work: Option<RetainedHandleSharePlan<D>>,
    failure_terminal_work_state: FailureTerminalWorkState,
    success_outcome_rejected: bool,
    mode: RetainedOutcomeMode,
    containment: AssemblyContainmentProvenance,
    /// Last issued validation epoch. Zero remains reserved for never-visited table members.
    last_validation_epoch: u64,
}

pub(super) type PublicationAssemblySlot<O, P, D, CanonicalBlock, EvidenceBlock> =
    Option<PublicationAssemblyFields<O, P, D, CanonicalBlock, EvidenceBlock>>;

/// A movable short loan, drained when the terminal classifier lends the original batch slots.
struct PublicationAssemblyLoan<'inv, O, P, D, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fields: Option<&'inv mut PublicationAssemblyFields<O, P, D, CanonicalBlock, EvidenceBlock>>,
}

impl<'inv, O: OperationOwner, P: Copy + Ord, D: Copy + Ord, CanonicalBlock, EvidenceBlock>
    PublicationAssemblyLoan<'inv, O, P, D, CanonicalBlock, EvidenceBlock>
{
    fn take(
        &mut self,
    ) -> &'inv mut PublicationAssemblyFields<O, P, D, CanonicalBlock, EvidenceBlock> {
        self.fields
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }
}

impl<'inv, O: OperationOwner, P: Copy + Ord, D: Copy + Ord, CanonicalBlock, EvidenceBlock>
    From<&'inv mut PublicationAssemblyFields<O, P, D, CanonicalBlock, EvidenceBlock>>
    for PublicationAssemblyLoan<'inv, O, P, D, CanonicalBlock, EvidenceBlock>
{
    fn from(
        fields: &'inv mut PublicationAssemblyFields<O, P, D, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            fields: Some(fields),
        }
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord, CanonicalBlock, EvidenceBlock> std::ops::Deref
    for PublicationAssemblyLoan<'_, O, P, D, CanonicalBlock, EvidenceBlock>
{
    type Target = PublicationAssemblyFields<O, P, D, CanonicalBlock, EvidenceBlock>;

    fn deref(&self) -> &Self::Target {
        self.fields
            .as_deref()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord, CanonicalBlock, EvidenceBlock>
    std::ops::DerefMut for PublicationAssemblyLoan<'_, O, P, D, CanonicalBlock, EvidenceBlock>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.fields
            .as_deref_mut()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }
}

type RetainedOperationSlot<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> =
    Option<RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>>;

impl<O, P, D, CanonicalBlock, EvidenceBlock>
    PublicationAssemblyFields<O, P, D, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn is_empty(&self) -> bool {
        self.canonical_table.is_none()
            && self.evidence_table.is_none()
            && self.canonical_batch.is_none()
            && self.evidence_batch.is_none()
            && self.account.is_none()
    }

    fn retain_batches_fail_closed(&mut self) {
        std::mem::forget(self.canonical_batch.take());
        std::mem::forget(self.evidence_batch.take());
    }

    fn has_deferred_storage(&self) -> bool {
        self.evidence_table.is_some()
            && self.evidence_batch.is_some()
            && self.account.is_some()
            && match self.mode {
                RetainedOutcomeMode::SuccessCapable => {
                    self.canonical_table.is_some()
                        && self.canonical_batch.is_some()
                        && self.canonical_plan.is_some()
                }
                RetainedOutcomeMode::FailureOnly => {
                    self.canonical_table.is_none()
                        && self.canonical_batch.is_none()
                        && self.canonical_plan.is_none()
                }
            }
    }
}

/// Short permission over two outer-owned, physically accounted block tables.
#[must_use = "the assembly permission must be resolved by its invocation"]
pub(crate) struct RetainedPublicationAssembly<
    'inv,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub(super) loan:
        RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
}

pub(crate) struct RetainedPublicationAssemblyLoan<
    'inv,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    fields: PublicationAssemblyLoan<'inv, O, P, D, CanonicalBlock, EvidenceBlock>,
    // Only a short permission; all physical state is retained in the outer root.
    operation: RetainedOperationSlot<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn loan(
        &self,
    ) -> &RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    {
        &self.loan
    }

    pub(crate) fn loan_mut(
        &mut self,
    ) -> &mut RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    {
        &mut self.loan
    }

    pub(in crate::retained) fn into_loan(
        self,
    ) -> RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    {
        self.loan
    }

    pub(in crate::retained) fn from_loan(
        loan: RetainedPublicationAssemblyLoan<
            'inv,
            O,
            P,
            PP,
            U,
            D,
            F,
            CanonicalBlock,
            EvidenceBlock,
        >,
    ) -> Self {
        Self { loan }
    }

    pub(crate) fn defer_until_body_cleanup(self) -> Result<(), Self> {
        self.loan
            .defer_until_body_cleanup()
            .map_err(Self::from_loan)
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    fn prepare_body_deferral(&mut self) -> Option<Result<(), LedgerCause<D>>> {
        if !self.fields.has_deferred_storage() {
            return None;
        }
        let operation = self.operation.as_mut()?;
        let account = self.fields.account.as_ref()?;
        let baseline = operation.retained.baseline.first_account()?;
        if !account.is_same_account(baseline) {
            return None;
        }
        operation.prepare_body_deferral()
    }

    fn install_deferred_stage(
        mut self,
        stage: RetainedDeferredStage<CanonicalBlock, EvidenceBlock>,
    ) {
        self.stage_prepared_body_deferral(stage);
        self.operation = None;
        std::mem::forget(self);
    }

    fn stage_prepared_body_deferral(
        &mut self,
        stage: RetainedDeferredStage<CanonicalBlock, EvidenceBlock>,
    ) {
        let operation = self
            .operation
            .as_mut()
            .expect("a prepared body deferral retains its operation loan");
        operation.retained.metadata.deferred_stage = Some(stage);
    }

    pub(crate) fn defer_until_body_cleanup(mut self) -> Result<(), Self> {
        if !matches!(self.prepare_body_deferral(), Some(Ok(()))) {
            return Err(self);
        }
        self.install_deferred_stage(RetainedDeferredStage::Assembly);
        Ok(())
    }

    #[allow(
        clippy::type_complexity,
        reason = "the existing operation owner remains borrowed with its exact ledger vocabulary"
    )]
    pub(crate) fn storage_operation(
        &mut self,
    ) -> Result<
        &mut RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationError<D>,
    >
    where
        P: TerminalEvidenceKey,
        D: TerminalEvidenceKey,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        self.operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)
    }

    pub(crate) fn storage_destination(&self) -> Result<(P, D), RetainedPublicationError<D>> {
        let plan = self
            .fields
            .canonical_plan
            .as_ref()
            .ok_or(RetainedPublicationError::WrongOutcomeMode)?;
        Ok((plan.writer.phase, plan.writer.capacity_dimension))
    }

    pub(crate) fn share_typed_storage<T: Send + Sync + 'static>(
        &mut self,
        source: &SharedRetainedStorage<T>,
    ) -> Result<SharedRetainedStorage<T>, RetainedPublicationError<D>>
    where
        P: TerminalEvidenceKey,
        D: TerminalEvidenceKey,
    {
        self.ensure_canonical_work_available()?;
        let account = self
            .fields
            .account
            .as_ref()
            .ok_or(RetainedPublicationError::Poisoned)?;
        if !source.belongs_to(account) {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::OwnershipUnverified,
            ));
        }
        let preflight = account.try_begin_account_preflight().map_err(|cause| {
            RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(cause))
        })?;
        let action = preflight.admit_action().map_err(|cause| {
            RetainedPublicationError::Writer(map_account_action_admission_error(cause))
        })?;
        let (_, _, ledger) = self
            .operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .common
            .view()
            .coordinator
            .into_exclusive_progress_and_ledger();
        let admitted = operation_start::try_admit_borrowed_retained_share_work(
            ledger,
            self.fields.ordinary_handle_share_work,
        )
        .map_err(RetainedPublicationError::Writer)?;
        let allocation = action
            .run(|_| admitted.try_share_allocation(source.allocation()))
            .map_err(|failure| {
                let (allocation, cause) = failure.into_parts();
                drop(allocation);
                RetainedPublicationError::Writer(RetainedWriterError::Ledger(cause))
            })?;
        Ok(SharedRetainedStorage::from_allocation(allocation))
    }

    pub(crate) fn require_canonical_source<SourceBlock>(
        &mut self,
        source: &PublishedCanonicalAggregate<SourceBlock>,
    ) -> Result<(), RetainedPublicationError<D>> {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        let Some(account) = self.fields.account.as_ref() else {
            return Err(RetainedPublicationError::Poisoned);
        };
        let result = self
            .fields
            .canonical_lineage
            .require_source(account, source);
        if let Err(cause) = &result
            && let Some(containment) = retained_publication_error_containment(cause)
        {
            self.fields.containment = containment;
        }
        result
    }

    pub(crate) fn report_progress(&mut self, phase: PP, unit: U, completed: u64, total: Option<u64>)
    where
        P: TerminalEvidenceKey,
        D: TerminalEvidenceKey,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return;
        }
        let Some(operation) = self.operation.as_mut() else {
            self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
            return;
        };
        let _ = operation
            .producer_host()
            .report_progress(phase, unit, completed, total);
    }

    pub(crate) fn record_terminal_cause(&mut self, cause: PreparationFailureCode) {
        match self.operation.as_mut() {
            Some(operation) => operation.record_terminal_cause(cause),
            None => self.fields.containment = AssemblyContainmentProvenance::NewFaultPending,
        }
    }

    fn fallback_only<Proof>(
        &mut self,
        cause: PreparationFailureCode,
    ) -> RetainedClosureValidation<Proof> {
        self.record_terminal_cause(cause);
        RetainedClosureValidation::FallbackOnly {
            cause,
            activation_containment_required: false,
        }
    }

    fn closure_validation_fallback<Proof>(
        &mut self,
        cause: RetainedClosureValidationError,
    ) -> RetainedClosureValidation<Proof> {
        let activation_containment_required = cause != RetainedClosureValidationError::Panicked;
        let cause = PreparationFailureCode::from_closure_validation(cause);
        self.record_terminal_cause(cause);
        RetainedClosureValidation::FallbackOnly {
            cause,
            activation_containment_required,
        }
    }
}

pub(crate) enum RetainedClosureValidation<Proof> {
    Valid(Proof),
    Rejected {
        cause: PreparationFailureCode,
    },
    FallbackOnly {
        cause: PreparationFailureCode,
        activation_containment_required: bool,
    },
}

pub(crate) enum RetainedSuccessFreezeDecision {
    Ready,
    RichFailure {
        cause: PreparationFailureCode,
        control_cause: Option<ControlCause>,
    },
    DestinationRejected {
        cause: PreparationFailureCode,
    },
}

const FAILURE_TERMINAL_WORK_SEALED_CODE: u32 = 0x4654_5753;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailureTerminalWorkState {
    Inactive,
    Active,
    Failed,
}

/// Exact success closure whose owning assembly can no longer be mutated.
#[must_use = "validated retained success must be prepared or deliberately dropped"]
pub(crate) struct ValidatedRetainedSuccess<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    assembly: RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    proof: RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
}

/// Success closure after exact destinations and final progress are sealed for generation.
#[must_use = "the freeze permit must be downgraded or converted into a joint-commit contribution"]
pub(crate) struct RetainedSuccessFreezePermit<
    'inv,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    validated: ValidatedRetainedSuccess<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
}

/// Operation-only success owner released after the core accepts one generation identity.
#[must_use = "the freeze contribution must form the exact joint-commit intent"]
pub(crate) struct RetainedSuccessFreezeContribution<
    'inv,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    validated: ValidatedRetainedSuccess<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
}

#[must_use = "every freeze-permit branch retains the exact publication owner"]
pub(crate) enum RetainedSuccessFreezePermitIssue<
    'inv,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    Ready(RetainedSuccessFreezePermit<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    RichFailure {
        failure: RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        cause: PreparationFailureCode,
        control_cause: Option<ControlCause>,
    },
    DestinationRejected {
        failure: RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        cause: PreparationFailureCode,
    },
}

#[cfg(test)]
enum FreezePermitQualificationProbe<D> {
    Ready,
    Rejected {
        cause: CommitmentPairPushError<D>,
        canonical_commitments: usize,
        outcome_evidence_commitments: usize,
        dimension: DimensionSnapshot,
    },
}

#[must_use = "joint-commit formation retains the exact success or failure owner"]
#[allow(
    clippy::large_enum_variant,
    reason = "both variants are the exact linear terminal owner; boxing would add allocation and split containment"
)]
pub(crate) enum RetainedSuccessJointCommitFormation<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Intent(RetainedSuccessJointCommitIntent<O, P, D, F, CanonicalBlock, EvidenceBlock>),
    PreparedFailure(PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>),
}

/// The selected terminal rail before ledger qualification, physical cleanup, or evidence freeze.
#[must_use = "the unqualified terminal owner must be finished or contained with its payloads"]
pub(crate) struct RetainedPreFreezeClassification<
    'inv,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    state: TerminalPreFreezeTransition<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    set_id: RetainedSetId,
    control_cause: Option<ControlCause>,
}

type RetainedPreFreezeResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> = Result<
    RetainedPreFreezeClassification<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    RetainedPublicationPrepareError,
>;

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPreFreezeClassification<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    #[cfg(test)]
    pub(crate) fn is_success(&self) -> bool {
        matches!(&self.state, TerminalPreFreezeTransition::Success { .. })
    }

    #[cfg(test)]
    pub(crate) fn control_cause(&self) -> Option<&ControlCause> {
        self.control_cause.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn check_payload_cleanup(self) -> Result<Self, FinalizationContainmentError> {
        let Self {
            state,
            set_id,
            control_cause,
        } = self;
        Ok(Self {
            state: state.check_payload_cleanup()?,
            set_id,
            control_cause,
        })
    }

    pub(crate) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.state.contain_activation_fault()
    }

    pub(crate) fn require_success(self) -> Result<Self, RetainedPublicationPrepareError> {
        if matches!(&self.state, TerminalPreFreezeTransition::Success { .. }) {
            Ok(self)
        } else {
            Err(RetainedPublicationPrepareError::FinalizationContainment(
                self.contain_activation_fault(),
            ))
        }
    }

    pub(crate) fn finish_deferred(
        self,
    ) -> Result<(DeferredAttachedTerminal, Option<ControlCause>), RetainedPublicationPrepareError>
    {
        Ok((self.state.finish_deferred(self.set_id)?, self.control_cause))
    }
}

type PreparedRetainedPublicationWithControl<CanonicalBlock, EvidenceBlock, F, P, D> = (
    PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
    Option<ControlCause>,
);

type RetainedJointCommitFormationWithControl<O, P, D, F, CanonicalBlock, EvidenceBlock> = (
    RetainedSuccessJointCommitFormation<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    Option<ControlCause>,
);

/// Evidence-building state after a semantic rejection or before an explicit rich failure.
///
/// This wrapper intentionally exposes no canonical retain, share, or commitment method.
#[must_use = "failure evidence must be validated, downgraded, or deliberately dropped"]
pub(crate) struct RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    assembly: RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    fallback_cause: PreparationFailureCode,
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn defer_until_body_cleanup(mut self) -> Result<(), Self> {
        if !matches!(self.assembly.loan.prepare_body_deferral(), Some(Ok(()))) {
            return Err(self);
        }
        let Self {
            assembly,
            fallback_cause,
        } = self;
        assembly
            .loan
            .install_deferred_stage(RetainedDeferredStage::Failure { fallback_cause });
        Ok(())
    }

    pub(crate) fn with_fallback_cause(mut self, cause: PreparationFailureCode) -> Self {
        self.assembly.loan.record_terminal_cause(cause);
        self.fallback_cause = cause;
        self
    }

    pub(crate) fn reject_failure_terminal_work_envelope(&mut self, code: u32) {
        self.assembly
            .loan
            .reject_failure_terminal_work_envelope(code);
    }
}

/// Exact rich-failure closure whose owning assembly can no longer be mutated.
#[must_use = "validated retained failure must be prepared or deliberately dropped"]
pub(crate) struct ValidatedRetainedFailure<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    assembly: RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    proof: RetainedEvidenceClosureProof<EvidenceBlock>,
}

/// Terminal-only owner produced by an invalid root, poisoned graph, or contained validator panic.
#[must_use = "fallback-only retained state must be prepared or deliberately dropped"]
pub(crate) struct RetainedFallbackOnly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    assembly: RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    cause: PreparationFailureCode,
    activation_containment_required: bool,
}

/// Linear result of validating a success candidate.
#[must_use = "every validation outcome retains the complete publication owner"]
pub(crate) enum RetainedSuccessValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    Valid(ValidatedRetainedSuccess<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    Rejected(RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    FallbackOnly(RetainedFallbackOnly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
}

/// Linear result of validating a rich-failure candidate.
#[must_use = "every validation outcome retains the complete publication owner"]
pub(crate) enum RetainedFailureValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    Valid(ValidatedRetainedFailure<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    Rejected(RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
    FallbackOnly(RetainedFallbackOnly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>),
}

#[allow(clippy::result_large_err)]
impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedOperation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.into_loan().contain_activation_fault()
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    /// Quarantines a dormant operation while preserving its account-owned containment evidence.
    ///
    /// This is reserved for activation-contract faults and fail-closed publication opens. The
    /// coordinator and attribution state are deliberately retained after the physical baseline is
    /// detached, so no uncertain charge can be released as if rollback had been proved.
    pub(crate) fn contain_activation_fault(mut self) -> FinalizationContainmentError {
        let reason = self
            .retained
            .baseline
            .first_account()
            .map(PhysicalStorageAccount::snapshot)
            .filter(|snapshot| snapshot.poisoned())
            .and_then(|snapshot| snapshot.first_poison_evidence())
            .map_or(
                AccountPoisonReason::OwnershipGraphUnverified,
                AccountPoisonEvidence::reason,
            );
        let already_poisoned = self
            .retained
            .baseline
            .first_account()
            .map(PhysicalStorageAccount::snapshot)
            .is_some_and(|snapshot| snapshot.poisoned());
        if !already_poisoned {
            let account = match self.retained.baseline.first_account() {
                Some(account) => account,
                None => unreachable_final_owner_state(),
            };
            let _ = account.record_poison(reason);
        }
        self.retained.baseline.record_forgotten_physical_edges();
        let snapshot = match self.retained.baseline.first_account() {
            Some(account) => account.snapshot(),
            None => unreachable_final_owner_state(),
        };
        let cause = FinalizationContainmentError::new(AccountPoisoned::new(snapshot), reason);
        self.record_terminal_cause(PreparationFailureCode::product_graph(
            StoreExecutionState::ABANDONED,
        ));
        self.stop_for_containment();
        self.retained.baseline.fail_closed_after_recorded_poison();
        cause
    }
}

#[must_use = "activation open disposition owns the operation until it is handled"]
pub(crate) enum RetainedActivationOpenDisposition<Operation> {
    Retry(Operation),
    FallbackOnly(Operation),
    FailClosed(Operation),
}

impl<Operation, D> RetainedPublicationOpenError<Operation, D> {
    /// Returns the exact linear owner disposition without finalizing fail-closed containment.
    ///
    /// Activation callers must release every scoped semantic input before consuming a
    /// [`RetainedActivationOpenDisposition::FailClosed`] owner. The physical account may already
    /// be quarantined by the faulting retained action, but the operation graph remains owned until
    /// the caller performs its final containment step.
    pub(crate) fn into_activation_disposition(
        self,
    ) -> RetainedActivationOpenDisposition<Operation> {
        match self.disposition {
            RetainedPublicationOpenDisposition::Retry => {
                RetainedActivationOpenDisposition::Retry(self.operation)
            }
            RetainedPublicationOpenDisposition::FallbackOnly => {
                RetainedActivationOpenDisposition::FallbackOnly(self.operation)
            }
            RetainedPublicationOpenDisposition::FailClosed => {
                RetainedActivationOpenDisposition::FailClosed(self.operation)
            }
        }
    }
}

type RetainedFailureAssemblyResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> = Result<
    RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    FinalizationContainmentError,
>;

#[allow(clippy::result_large_err)]
impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn contain_activation_fault(&mut self) -> FinalizationContainmentError {
        let recorded = self
            .first_containment_account()
            .map(PhysicalStorageAccount::snapshot)
            .filter(|snapshot| snapshot.poisoned());
        match recorded {
            Some(snapshot) => {
                self.contain_after_recorded_poison(snapshot.first_poison_evidence().map_or(
                    AccountPoisonReason::OwnershipGraphUnverified,
                    AccountPoisonEvidence::reason,
                ))
            }
            None => self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified),
        }
    }

    pub(crate) fn try_discard_canonical_for_rejection(
        &mut self,
        fallback_cause: PreparationFailureCode,
    ) -> Result<(), FinalizationContainmentError> {
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable
            || !self.has_exact_storage_graph()
        {
            let cause = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(cause);
        }
        self.record_terminal_cause(fallback_cause);
        self.activate_failure_terminal_work();
        self.try_discard_canonical_ownership()?;

        Ok(())
    }

    /// Converts an already-open evidence-only publication into its rooted-failure construction
    /// state without ever creating canonical ownership.
    pub(crate) fn verify_failure_only(
        &mut self,
        fallback_cause: PreparationFailureCode,
    ) -> Result<(), FinalizationContainmentError> {
        if self.fields.mode != RetainedOutcomeMode::FailureOnly
            || !self.fields.success_outcome_rejected
            || !self.has_exact_storage_graph()
            || !self.canonical_ownership_is_absent()
        {
            let cause = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(cause);
        }
        self.record_terminal_cause(fallback_cause);
        Ok(())
    }

    pub(crate) fn try_discard_canonical_ownership(
        &mut self,
    ) -> Result<(), FinalizationContainmentError> {
        if self.fields.mode == RetainedOutcomeMode::FailureOnly
            && self.canonical_ownership_is_absent()
        {
            return Ok(());
        }
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable
            || !self.has_exact_storage_graph()
        {
            let cause = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(cause);
        }

        let fields = &mut *self.fields;
        let account = match fields.account.as_ref() {
            Some(account) => account,
            None => {
                let cause =
                    self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(cause);
            }
        };
        let canonical = match fields.canonical_table.take() {
            Some(canonical) => canonical,
            None => {
                let cause =
                    self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(cause);
            }
        };
        match drop_retained_table_with_account_anchor(canonical, account) {
            RetainedStorageDropOutcome::Clean => {}
            RetainedStorageDropOutcome::StorageReleaseUnverified(witness)
            | RetainedStorageDropOutcome::ChargeReleaseUnverified(witness)
            | RetainedStorageDropOutcome::StillSharedOwnershipFault(witness)
            | RetainedStorageDropOutcome::ForeignOwnershipFault(witness) => {
                let cause = self.contain_after_recorded_poison(witness.reason);
                return Err(cause);
            }
            RetainedStorageDropOutcome::MissingFaultAnchor => {
                let cause =
                    self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(cause);
            }
        }
        drop(self.fields.canonical_batch.take());
        self.fields.canonical_plan = None;
        self.fields.mode = RetainedOutcomeMode::FailureOnly;
        self.fields.success_outcome_rejected = true;
        if !self.has_exact_storage_graph() || !self.canonical_ownership_is_absent() {
            let cause = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(cause);
        }
        Ok(())
    }

    fn canonical_ownership_is_absent(&self) -> bool {
        self.fields.canonical_plan.is_none()
            && self.fields.canonical_table.is_none()
            && self.fields.canonical_batch.is_none()
            && self.canonical_len() == 0
            && self.canonical_new_blocks() == 0
            && self.canonical_commitment_len() == 0
    }

    pub(crate) fn activate_failure_terminal_work(&mut self) {
        if self.fields.success_outcome_rejected {
            return;
        }
        self.fields.success_outcome_rejected = true;
        let Some(operation) = self.operation.as_mut() else {
            self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
            self.fields.failure_terminal_work_state = FailureTerminalWorkState::Failed;
            return;
        };
        match operation.activate_failure_terminal_work() {
            Ok(Some(handle_share_work)) => {
                self.fields.failure_evidence_handle_share_work = Some(handle_share_work);
                self.fields.failure_terminal_work_state = FailureTerminalWorkState::Active;
            }
            Ok(None) => {
                self.fields.failure_terminal_work_state = FailureTerminalWorkState::Failed;
            }
            Err(_) => {
                operation.record_ledger_failure();
                self.fields.failure_terminal_work_state = FailureTerminalWorkState::Failed;
            }
        }
    }

    fn activate_failure_terminal_work_after_recorded_cause(&mut self) {
        let cause_recorded = self
            .operation
            .as_ref()
            .and_then(|operation| operation.sticky_terminal_cause())
            .is_some();
        if cause_recorded {
            self.activate_failure_terminal_work();
        }
    }

    fn ensure_failure_terminal_work_available(&self) -> Result<(), RetainedPublicationError<D>> {
        if self.fields.failure_terminal_work_state == FailureTerminalWorkState::Failed {
            Err(RetainedPublicationError::FailureTerminalWorkSealed)
        } else {
            Ok(())
        }
    }

    pub(crate) fn reject_failure_terminal_work_envelope(&mut self, code: u32) {
        if let Some(operation) = self.operation.as_mut() {
            operation.record_product_graph_failure(code);
        } else {
            self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
        }
        self.fields.failure_terminal_work_state = FailureTerminalWorkState::Failed;
    }

    fn outcome_evidence_handle_share_work(
        &self,
    ) -> Result<RetainedHandleSharePlan<D>, RetainedPublicationError<D>> {
        match self.fields.failure_terminal_work_state {
            FailureTerminalWorkState::Inactive => Ok(self.fields.ordinary_handle_share_work),
            FailureTerminalWorkState::Active => self
                .fields
                .failure_evidence_handle_share_work
                .ok_or(RetainedPublicationError::FailureTerminalWorkSealed),
            FailureTerminalWorkState::Failed => {
                Err(RetainedPublicationError::FailureTerminalWorkSealed)
            }
        }
    }

    fn ensure_canonical_work_available(&mut self) -> Result<(), RetainedPublicationError<D>> {
        self.activate_failure_terminal_work_after_recorded_cause();
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable {
            Err(RetainedPublicationError::WrongOutcomeMode)
        } else if self.fields.success_outcome_rejected {
            Err(RetainedPublicationError::SuccessOutcomeRejected)
        } else {
            Ok(())
        }
    }

    fn terminal_cause_or(&self, fallback: PreparationFailureCode) -> PreparationFailureCode {
        self.operation
            .as_ref()
            .and_then(|operation| operation.sticky_terminal_cause())
            .unwrap_or(fallback)
    }

    fn seal_failure_terminal_work(&mut self) {
        if self.fields.failure_terminal_work_state != FailureTerminalWorkState::Active {
            return;
        }
        self.fields.failure_terminal_work_state = FailureTerminalWorkState::Failed;
        if let Some(operation) = self.operation.as_mut() {
            operation.record_product_graph_failure(FAILURE_TERMINAL_WORK_SEALED_CODE);
        } else {
            self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
        }
    }

    fn observe_publication_error(&mut self, error: &RetainedPublicationError<D>) {
        if matches!(
            error,
            RetainedPublicationError::Writer(RetainedWriterError::Ledger(_))
        ) {
            if let Some(operation) = self.operation.as_mut() {
                operation.record_ledger_failure();
            }
            self.activate_failure_terminal_work();
        }
        if let Some(containment) = retained_publication_error_containment(error) {
            self.fields.containment = containment;
        }
    }

    pub(crate) fn try_validate_work_dimension(
        &mut self,
        dimension: D,
    ) -> Result<(), RetainedPublicationError<D>> {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .validate_ordinary_dimension(dimension)
            .map_err(|cause| RetainedPublicationError::Writer(RetainedWriterError::Ledger(cause)))
    }

    pub(crate) fn try_control_checkpoint(
        &mut self,
    ) -> Result<RetainedControlCheckpoint<'_, O, P, PP, U, D>, RetainedPublicationError<D>> {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        let Some(operation) = self.operation.as_mut() else {
            self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedPublicationError::Poisoned);
        };
        Ok(operation.control_checkpoint())
    }

    fn try_next_validation_epoch(&mut self) -> Result<NonZeroU64, RetainedClosureValidationError> {
        let Some(next) = self
            .fields
            .last_validation_epoch
            .checked_add(1)
            .and_then(NonZeroU64::new)
        else {
            self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedClosureValidationError::Poisoned);
        };
        self.fields.last_validation_epoch = next.get();
        Ok(next)
    }

    /// Validates the exact canonical and outcome-evidence closure without transferring ownership.
    ///
    /// The callback must visit every unique member. Repeated DAG edges are accepted. A callback
    /// panic is contained and returns [`RetainedClosureValidationError::Panicked`]. Each attempt
    /// uses a fresh assembly-local epoch, so rejection and panic need no table-wide cleanup.
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedClosureValidationError::Poisoned);
        }
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedClosureValidationError::WrongOutcomeMode);
        }
        let epoch = self.try_next_validation_epoch()?;
        let (Some(canonical), Some(evidence)) = (
            self.fields.canonical_table.as_ref(),
            self.fields.evidence_table.as_ref(),
        ) else {
            self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedClosureValidationError::Poisoned);
        };
        let canonical = canonical.as_ref();
        let evidence = evidence.as_ref();
        if canonical
            .member(canonical_root.identity(), self.fields.set_id)
            .is_none()
            || evidence
                .member(evidence_root.identity(), self.fields.set_id)
                .is_none()
        {
            return Err(RetainedClosureValidationError::InvalidRoot);
        }
        let mut session = RetainedClosureSession {
            canonical,
            evidence,
            set_id: self.fields.set_id,
            epoch,
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
                return Err(RetainedClosureValidationError::Panicked);
            }
        };
        let exact = accepted
            && session.valid
            && session.canonical_seen == canonical.member_count()
            && session.evidence_seen == evidence.member_count();
        if !exact {
            return Err(RetainedClosureValidationError::Rejected);
        }
        Ok(RetainedSuccessClosureProof {
            set_id: self.fields.set_id,
            canonical_count: canonical.member_count(),
            evidence_count: evidence.member_count(),
            canonical_root: canonical_root.identity(),
            evidence_root: evidence_root.identity(),
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedClosureValidationError::Poisoned);
        }
        let epoch = self.try_next_validation_epoch()?;
        let Some(evidence) = self.fields.evidence_table.as_ref() else {
            self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedClosureValidationError::Poisoned);
        };
        let evidence = evidence.as_ref();
        if evidence
            .member(evidence_root.identity(), self.fields.set_id)
            .is_none()
        {
            return Err(RetainedClosureValidationError::InvalidRoot);
        }
        let mut session = RetainedEvidenceClosureSession {
            evidence,
            set_id: self.fields.set_id,
            epoch,
            seen: 0,
            valid: true,
            not_send_or_sync: PhantomData,
        };
        let accepted =
            match catch_unwind(AssertUnwindSafe(|| validate(&mut session, evidence_root))) {
                Ok(accepted) => accepted,
                Err(payload) => {
                    discard_untrusted_panic_payload(payload);
                    return Err(RetainedClosureValidationError::Panicked);
                }
            };
        let exact = accepted && session.valid && session.seen == evidence.member_count();
        if !exact {
            return Err(RetainedClosureValidationError::Rejected);
        }
        Ok(RetainedEvidenceClosureProof {
            set_id: self.fields.set_id,
            evidence_count: evidence.member_count(),
            evidence_root: evidence_root.identity(),
            marker: PhantomData,
        })
    }

    fn validate_success<CanonicalTag, EvidenceTag>(
        &mut self,
        canonical_root: CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock>,
            CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> RetainedClosureValidation<RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>>
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.activate_failure_terminal_work_after_recorded_cause();
        if self.fields.failure_terminal_work_state == FailureTerminalWorkState::Failed {
            let cause = self.terminal_cause_or(PreparationFailureCode::product_graph(
                FAILURE_TERMINAL_WORK_SEALED_CODE,
            ));
            return self.fallback_only(cause);
        }
        if self.fields.success_outcome_rejected {
            let fallback_cause = self.terminal_cause_or(PreparationFailureCode::requested());
            return RetainedClosureValidation::Rejected {
                cause: fallback_cause,
            };
        }
        match self.try_validate_success(canonical_root, evidence_root, validate) {
            Ok(proof) => RetainedClosureValidation::Valid(proof),
            Err(RetainedClosureValidationError::Rejected) => {
                self.activate_failure_terminal_work();
                RetainedClosureValidation::Rejected {
                    cause: PreparationFailureCode::from_closure_validation(
                        RetainedClosureValidationError::Rejected,
                    ),
                }
            }
            Err(cause) => self.closure_validation_fallback(cause),
        }
    }

    /// Validates a success graph while lending poll and prebound observed-work authority to the
    /// validator inside the existing panic boundary.
    fn validate_success_controlled<CanonicalTag, EvidenceTag>(
        &mut self,
        work_dimension: D,
        scratch_dimension: D,
        canonical_root: CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'session, 'control> FnOnce(
            &mut RetainedClosureSession<'session, CanonicalBlock, EvidenceBlock>,
            CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
            RetainedWorkControl<'control, O, P, PP, U, D>,
        ) -> bool,
    ) -> RetainedClosureValidation<RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>>
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.activate_failure_terminal_work_after_recorded_cause();
        if self.fields.failure_terminal_work_state == FailureTerminalWorkState::Failed {
            let cause = self.terminal_cause_or(PreparationFailureCode::product_graph(
                FAILURE_TERMINAL_WORK_SEALED_CODE,
            ));
            return self.fallback_only(cause);
        }
        if self.fields.success_outcome_rejected {
            let fallback_cause = self.terminal_cause_or(PreparationFailureCode::requested());
            return RetainedClosureValidation::Rejected {
                cause: fallback_cause,
            };
        }
        let validation = if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            Err(RetainedClosureValidationError::Poisoned)
        } else if self.fields.mode != RetainedOutcomeMode::SuccessCapable {
            Err(RetainedClosureValidationError::WrongOutcomeMode)
        } else {
            let validation_storage_baseline = self
                .operation
                .as_ref()
                .map(RetainedOperationLoan::planning_baseline);
            let epoch = match self.try_next_validation_epoch() {
                Ok(epoch) => epoch,
                Err(cause) => {
                    return self.closure_validation_fallback(cause);
                }
            };
            let control = match self.operation.as_mut() {
                Some(operation) => Some(
                    operation.split_work_and_reservation_control(work_dimension, scratch_dimension),
                ),
                None => {
                    self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
                    None
                }
            };
            let parts = match (
                self.fields.canonical_table.as_ref(),
                self.fields.evidence_table.as_ref(),
                control,
            ) {
                (Some(canonical), Some(evidence), Some(control)) => {
                    Some((canonical.as_ref(), evidence.as_ref(), control, epoch))
                }
                _ => {
                    self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
                    None
                }
            };
            match parts {
                Some((canonical, evidence, (control, mut root), epoch)) => {
                    if canonical
                        .member(canonical_root.identity(), self.fields.set_id)
                        .is_none()
                        || evidence
                            .member(evidence_root.identity(), self.fields.set_id)
                            .is_none()
                    {
                        Err(RetainedClosureValidationError::InvalidRoot)
                    } else {
                        let mut session = RetainedClosureSession {
                            canonical,
                            evidence,
                            set_id: self.fields.set_id,
                            epoch,
                            canonical_seen: 0,
                            evidence_seen: 0,
                            valid: true,
                            not_send_or_sync: PhantomData,
                        };
                        let accepted = match catch_unwind(AssertUnwindSafe(|| {
                            control.validate(&mut root, |control| {
                                validate(&mut session, canonical_root, evidence_root, control)
                            })
                        })) {
                            Ok(accepted) => accepted,
                            Err(payload) => {
                                discard_untrusted_panic_payload(payload);
                                return self.closure_validation_fallback(
                                    RetainedClosureValidationError::Panicked,
                                );
                            }
                        };
                        let operation = self.operation.as_ref();
                        let storage_exact = validation_storage_baseline.is_some_and(|baseline| {
                            operation.is_some_and(|operation| {
                                operation.planning_matches_baseline(baseline)
                            })
                        });
                        let permit_outstanding = operation
                            .is_none_or(|operation| operation.has_outstanding_work_permit());
                        let work_partially_resolved = operation.is_some_and(|operation| {
                            operation.has_partially_resolved_work_permit()
                        });
                        let terminal_cause_recorded = operation
                            .and_then(|operation| operation.sticky_terminal_cause())
                            .is_some();
                        let exact = accepted
                            && session.valid
                            && session.canonical_seen == canonical.member_count()
                            && session.evidence_seen == evidence.member_count();
                        if !storage_exact {
                            self.fields.containment =
                                AssemblyContainmentProvenance::NewFaultPending;
                            Err(RetainedClosureValidationError::Poisoned)
                        } else if permit_outstanding {
                            Err(RetainedClosureValidationError::Poisoned)
                        } else if work_partially_resolved || terminal_cause_recorded {
                            Err(RetainedClosureValidationError::Rejected)
                        } else if exact {
                            Ok(RetainedSuccessClosureProof {
                                set_id: self.fields.set_id,
                                canonical_count: canonical.member_count(),
                                evidence_count: evidence.member_count(),
                                canonical_root: canonical_root.identity(),
                                evidence_root: evidence_root.identity(),
                                marker: PhantomData,
                            })
                        } else {
                            Err(RetainedClosureValidationError::Rejected)
                        }
                    }
                }
                None => Err(RetainedClosureValidationError::Poisoned),
            }
        };
        match validation {
            Ok(proof) => RetainedClosureValidation::Valid(proof),
            Err(RetainedClosureValidationError::Rejected) => {
                self.activate_failure_terminal_work();
                let fallback_cause =
                    self.terminal_cause_or(PreparationFailureCode::from_closure_validation(
                        RetainedClosureValidationError::Rejected,
                    ));
                RetainedClosureValidation::Rejected {
                    cause: fallback_cause,
                }
            }
            Err(cause) => self.closure_validation_fallback(cause),
        }
    }

    fn validate_rich_failure<EvidenceTag>(
        &mut self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedEvidenceClosureSession<'a, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> RetainedClosureValidation<RetainedEvidenceClosureProof<EvidenceBlock>>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.activate_failure_terminal_work();
        if self.fields.failure_terminal_work_state == FailureTerminalWorkState::Failed {
            return self.fallback_only(PreparationFailureCode::product_graph(
                FAILURE_TERMINAL_WORK_SEALED_CODE,
            ));
        }
        match self.try_validate_rich_failure(evidence_root, validate) {
            Ok(proof) => RetainedClosureValidation::Valid(proof),
            Err(RetainedClosureValidationError::Rejected) => RetainedClosureValidation::Rejected {
                cause: PreparationFailureCode::from_closure_validation(
                    RetainedClosureValidationError::Rejected,
                ),
            },
            Err(cause) => self.closure_validation_fallback(cause),
        }
    }

    fn validate_rich_failure_controlled<EvidenceTag>(
        &mut self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'session, 'control> FnOnce(
            &mut RetainedEvidenceClosureSession<'session, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
            RetainedFailureWorkControl<'control, O, P, PP, U, D>,
        ) -> bool,
    ) -> RetainedClosureValidation<RetainedEvidenceClosureProof<EvidenceBlock>>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.activate_failure_terminal_work();
        if self.fields.failure_terminal_work_state == FailureTerminalWorkState::Failed {
            return self.fallback_only(PreparationFailureCode::product_graph(
                FAILURE_TERMINAL_WORK_SEALED_CODE,
            ));
        }
        let validation = if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            Err(RetainedClosureValidationError::Poisoned)
        } else {
            let epoch = match self.try_next_validation_epoch() {
                Ok(epoch) => epoch,
                Err(cause) => {
                    return self.closure_validation_fallback(cause);
                }
            };
            let work = self.fields.failure_evidence_handle_share_work;
            let control = match (self.operation.as_mut(), work) {
                (Some(operation), Some(work)) => {
                    Some(operation.split_failure_terminal_work_control(work))
                }
                _ => {
                    self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
                    None
                }
            };
            let parts = match (self.fields.evidence_table.as_ref(), control) {
                (Some(evidence), Some(control)) => Some((evidence.as_ref(), control, epoch)),
                _ => {
                    self.fields.containment = AssemblyContainmentProvenance::NewFaultPending;
                    None
                }
            };
            match parts {
                Some((evidence, (control, mut root), epoch)) => {
                    if evidence
                        .member(evidence_root.identity(), self.fields.set_id)
                        .is_none()
                    {
                        Err(RetainedClosureValidationError::InvalidRoot)
                    } else {
                        let mut session = RetainedEvidenceClosureSession {
                            evidence,
                            set_id: self.fields.set_id,
                            epoch,
                            seen: 0,
                            valid: true,
                            not_send_or_sync: PhantomData,
                        };
                        let accepted = match catch_unwind(AssertUnwindSafe(|| {
                            control.validate(&mut root, |control| {
                                validate(&mut session, evidence_root, control)
                            })
                        })) {
                            Ok(accepted) => accepted,
                            Err(payload) => {
                                discard_untrusted_panic_payload(payload);
                                return self.closure_validation_fallback(
                                    RetainedClosureValidationError::Panicked,
                                );
                            }
                        };
                        let exact =
                            accepted && session.valid && session.seen == evidence.member_count();
                        if exact {
                            Ok(RetainedEvidenceClosureProof {
                                set_id: self.fields.set_id,
                                evidence_count: evidence.member_count(),
                                evidence_root: evidence_root.identity(),
                                marker: PhantomData,
                            })
                        } else {
                            Err(RetainedClosureValidationError::Rejected)
                        }
                    }
                }
                None => Err(RetainedClosureValidationError::Poisoned),
            }
        };
        match validation {
            Ok(proof) => RetainedClosureValidation::Valid(proof),
            Err(RetainedClosureValidationError::Rejected) => RetainedClosureValidation::Rejected {
                cause: PreparationFailureCode::from_closure_validation(
                    RetainedClosureValidationError::Rejected,
                ),
            },
            Err(cause) => self.closure_validation_fallback(cause),
        }
    }

    pub fn canonical_len(&self) -> usize {
        self.fields
            .canonical_table
            .as_ref()
            .map_or(0, |table| table.as_ref().member_count())
    }

    pub fn canonical_new_blocks(&self) -> usize {
        self.fields
            .canonical_table
            .as_ref()
            .map_or(0, |table| table.as_ref().new_block_count())
    }

    pub fn canonical_commitment_len(&self) -> usize {
        self.fields
            .canonical_batch
            .as_ref()
            .map_or(0, AttributionReservationBatch::commitment_len)
    }

    pub(crate) fn contains_canonical_member<Tag>(
        &self,
        slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
    ) -> bool
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.fields.containment == AssemblyContainmentProvenance::Healthy
            && self.fields.canonical_table.as_ref().is_some_and(|table| {
                table
                    .as_ref()
                    .member(slot.identity(), self.fields.set_id)
                    .is_some()
            })
    }

    pub fn outcome_evidence_len(&self) -> usize {
        self.fields
            .evidence_table
            .as_ref()
            .map_or(0, |table| table.as_ref().member_count())
    }

    pub fn outcome_evidence_new_blocks(&self) -> usize {
        self.fields
            .evidence_table
            .as_ref()
            .map_or(0, |table| table.as_ref().new_block_count())
    }

    pub fn outcome_evidence_commitment_len(&self) -> usize {
        self.fields
            .evidence_batch
            .as_ref()
            .map_or(0, AttributionReservationBatch::commitment_len)
    }

    pub fn contains_dimension(&self, dimension: D) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|operation| operation.contains_dimension(dimension))
    }

    pub(crate) fn try_bind_canonical_source(
        &mut self,
        source: &PublishedCanonicalAggregate<CanonicalBlock>,
        work_dimension: D,
    ) -> Result<(), CanonicalSourceBindError<D>> {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(CanonicalSourceBindError::Publication(
                RetainedPublicationError::Poisoned,
            ));
        }
        self.ensure_canonical_work_available()
            .map_err(CanonicalSourceBindError::Publication)?;
        self.require_canonical_source(source)
            .map_err(CanonicalSourceBindError::Publication)?;
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(CanonicalSourceBindError::Publication(
                RetainedPublicationError::WrongOutcomeMode,
            ));
        }
        let (control, mut root) = self
            .operation
            .as_mut()
            .ok_or(CanonicalSourceBindError::Publication(
                RetainedPublicationError::Poisoned,
            ))?
            .split_work_control(work_dimension);
        let fields = &mut *self.fields;
        let result = try_bind_canonical_source_mapping(
            fields
                .account
                .as_ref()
                .ok_or(CanonicalSourceBindError::Publication(
                    RetainedPublicationError::Poisoned,
                ))?,
            fields
                .canonical_table
                .as_mut()
                .ok_or(CanonicalSourceBindError::Publication(
                    RetainedPublicationError::WrongOutcomeMode,
                ))?,
            fields.set_id,
            source,
            control,
            &mut root,
        );
        if let Err(CanonicalSourceBindError::Publication(error)) = &result {
            self.observe_publication_error(error);
        }
        result
    }

    pub(crate) fn try_inherit_all_canonical_leaves(
        &mut self,
        source: &PublishedCanonicalAggregate<CanonicalBlock>,
        work_dimension: D,
    ) -> Result<(), CanonicalLeafInheritanceError<D>>
    where
        CanonicalBlock: CanonicalRetainedLeafPolicy,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(CanonicalLeafInheritanceError::Publication(
                RetainedPublicationError::Poisoned,
            ));
        }
        self.ensure_canonical_work_available()
            .map_err(CanonicalLeafInheritanceError::Publication)?;
        self.require_canonical_source(source)
            .map_err(CanonicalLeafInheritanceError::Publication)?;
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(CanonicalLeafInheritanceError::Publication(
                RetainedPublicationError::WrongOutcomeMode,
            ));
        }
        let (control, mut root) = self
            .operation
            .as_mut()
            .ok_or(CanonicalLeafInheritanceError::Publication(
                RetainedPublicationError::Poisoned,
            ))?
            .split_work_control(work_dimension);
        let fields = &mut *self.fields;
        let result = try_inherit_all_canonical_leaves(
            &mut root,
            fields
                .account
                .as_ref()
                .ok_or(CanonicalLeafInheritanceError::Publication(
                    RetainedPublicationError::Poisoned,
                ))?,
            fields.ordinary_handle_share_work,
            fields
                .canonical_table
                .as_mut()
                .ok_or(CanonicalLeafInheritanceError::Publication(
                    RetainedPublicationError::WrongOutcomeMode,
                ))?,
            fields.set_id,
            source,
            control,
        );
        if let Err(CanonicalLeafInheritanceError::Publication(error)) = &result {
            self.observe_publication_error(error);
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn test_dimension_evidence(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.operation
            .as_ref()?
            .common
            .as_read()
            .dimension_evidence(dimension)
            .ok()
    }

    #[cfg(test)]
    pub(crate) fn test_ledger_probe(&self) -> crate::ledger::LedgerTestProbe<P, D> {
        self.operation
            .as_ref()
            .expect("test assembly retains its operation")
            .common
            .test_shared_probe()
    }

    #[cfg(test)]
    pub(crate) fn test_canonical_attributed_bytes(&self) -> u64 {
        self.fields
            .canonical_batch
            .as_ref()
            .and_then(AttributionReservationBatch::attributed_bytes)
            .expect("test canonical attribution bytes remain exact")
    }

    #[cfg(test)]
    pub(crate) fn test_take_canonical_source_mapping_probe_count(&self) -> u64 {
        self.fields.canonical_table.as_ref().map_or(0, |table| {
            table.as_ref().take_canonical_source_mapping_probe_count()
        })
    }

    #[cfg(test)]
    pub(crate) fn test_storage_graph_is_exact(&self) -> bool {
        self.has_exact_storage_graph()
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        let canonical_plan = self
            .fields
            .canonical_plan
            .ok_or(RetainedPublicationError::WrongOutcomeMode)?;
        let fields = &mut *self.fields;
        let result = try_retain_block(
            &mut self
                .operation
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?
                .internal_work_root(),
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.ordinary_handle_share_work,
            canonical_plan.writer.phase,
            canonical_plan.writer.attribution_dimension,
            fields
                .canonical_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields
                .canonical_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            RetainedMembershipDestination::Canonical,
            RetainedPublicationError::CanonicalSlotLimit,
            RetainedBlockOrigin::New,
            plan,
            |emplacer, _, _| build(emplacer),
        );
        match result {
            Ok(identity) => Ok(CanonicalRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                Err(error)
            }
        }
    }

    /// Retains one canonical block while lending only poll and prebound observed-work authority to
    /// its constructing closure.
    ///
    /// The callback remains inside the ordinary retained-allocation panic boundary. A returned
    /// build code therefore rolls the current block back cleanly, while a panic preserves the
    /// existing fail-closed containment behavior.
    pub(crate) fn try_retain_canonical_controlled<Tag>(
        &mut self,
        work_dimension: D,
        plan: RetainedAllocationPlan,
        build: impl for<'placement, 'control> FnOnce(
            BlockEmplacer<'placement, CanonicalBlock>,
            RetainedWorkControl<'control, O, P, PP, U, D>,
        ) -> Result<
            BlockPlacement<'placement, CanonicalBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<CanonicalRetainedSlot<CanonicalBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        let canonical_plan = self
            .fields
            .canonical_plan
            .ok_or(RetainedPublicationError::WrongOutcomeMode)?;
        let (control, mut root) = self
            .operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .split_work_control(work_dimension);
        let fields = &mut *self.fields;
        let result = try_retain_block(
            &mut root,
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.ordinary_handle_share_work,
            canonical_plan.writer.phase,
            canonical_plan.writer.attribution_dimension,
            fields
                .canonical_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields
                .canonical_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            RetainedMembershipDestination::Canonical,
            RetainedPublicationError::CanonicalSlotLimit,
            RetainedBlockOrigin::New,
            plan,
            move |emplacer, _, root| control.build(root, |control| build(emplacer, control)),
        );
        match result {
            Ok(identity) => Ok(CanonicalRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                Err(error)
            }
        }
    }

    /// Retains one newly constructed canonical block as the unique replacement mapping for an
    /// exact published same-account source slot.
    pub(crate) fn try_retain_rewritten_canonical<Tag>(
        &mut self,
        source: &PublishedCanonicalAggregate<CanonicalBlock>,
        source_slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        self.require_canonical_source(source)?;
        let canonical_plan = self
            .fields
            .canonical_plan
            .ok_or(RetainedPublicationError::WrongOutcomeMode)?;
        let fields = &mut *self.fields;
        let result = try_retain_rewritten_canonical_block(
            &mut self
                .operation
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?
                .internal_work_root(),
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.ordinary_handle_share_work,
            canonical_plan.writer.phase,
            canonical_plan.writer.attribution_dimension,
            fields
                .canonical_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields
                .canonical_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            source,
            source_slot.identity(),
            plan,
            |emplacer, _| build(emplacer),
        );
        match result {
            Ok(identity) => Ok(CanonicalRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                Err(error)
            }
        }
    }

    /// Retains one controlled canonical replacement while preserving exact source lineage.
    pub(crate) fn try_retain_rewritten_canonical_controlled<Tag>(
        &mut self,
        source: &PublishedCanonicalAggregate<CanonicalBlock>,
        source_slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
        work_dimension: D,
        plan: RetainedAllocationPlan,
        build: impl for<'placement, 'control> FnOnce(
            BlockEmplacer<'placement, CanonicalBlock>,
            RetainedWorkControl<'control, O, P, PP, U, D>,
        ) -> Result<
            BlockPlacement<'placement, CanonicalBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<CanonicalRetainedSlot<CanonicalBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        self.require_canonical_source(source)?;
        let canonical_plan = self
            .fields
            .canonical_plan
            .ok_or(RetainedPublicationError::WrongOutcomeMode)?;
        let (control, mut root) = self
            .operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .split_work_control(work_dimension);
        let fields = &mut *self.fields;
        let result = try_retain_rewritten_canonical_block(
            &mut root,
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.ordinary_handle_share_work,
            canonical_plan.writer.phase,
            canonical_plan.writer.attribution_dimension,
            fields
                .canonical_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields
                .canonical_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            source,
            source_slot.identity(),
            plan,
            move |emplacer, root| control.build(root, |control| build(emplacer, control)),
        );
        match result {
            Ok(identity) => Ok(CanonicalRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                Err(error)
            }
        }
    }

    /// Retains one structurally and aggregate-ledger-preflighted homogeneous canonical batch.
    ///
    /// Every item still traverses the ordinary single-block retain rail, preserving its physical
    /// admission, attribution binding, panic containment, and immutable placement semantics.
    /// Physical-account admission remains race-safe per item rather than reserved by the batch
    /// snapshot. An ordinary execution failure releases the retained prefix storage-first and
    /// leaves the assembly writable. A panic or unverified cleanup returns no range and enters
    /// fail-closed containment.
    pub fn try_retain_canonical_batch<Tag, Source>(
        &mut self,
        source: &Source,
        tag: Tag,
    ) -> Result<CanonicalRetainedRange<CanonicalBlock, Tag>, CanonicalRetainBatchError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
        Source: CanonicalRetainBatchSource<CanonicalBlock, Tag>,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(CanonicalRetainBatchError::Preflight(
                RetainedPublicationError::Poisoned,
            ));
        }
        self.ensure_canonical_work_available()
            .map_err(CanonicalRetainBatchError::Preflight)?;
        let canonical_plan =
            self.fields
                .canonical_plan
                .ok_or(CanonicalRetainBatchError::Preflight(
                    RetainedPublicationError::WrongOutcomeMode,
                ))?;
        let preflight = try_preflight_canonical_batch::<O, P, D, CanonicalBlock, Tag, Source>(
            source,
            CanonicalBatchPreflightContext {
                ledger: self
                    .operation
                    .as_ref()
                    .ok_or(CanonicalRetainBatchError::Preflight(
                        RetainedPublicationError::Poisoned,
                    ))?
                    .common
                    .as_read(),
                account: self.fields.account.as_ref().ok_or(
                    CanonicalRetainBatchError::Preflight(RetainedPublicationError::Poisoned),
                )?,
                table: self.fields.canonical_table.as_ref().ok_or(
                    CanonicalRetainBatchError::Preflight(RetainedPublicationError::Poisoned),
                )?,
                batch: self.fields.canonical_batch.as_ref().ok_or(
                    CanonicalRetainBatchError::Preflight(RetainedPublicationError::Poisoned),
                )?,
                handle_share_work: self.fields.ordinary_handle_share_work,
                table_plan: canonical_plan,
                set_id: self.fields.set_id,
                slot_limit_error: RetainedPublicationError::CanonicalSlotLimit,
            },
        );
        let qualified = match preflight {
            Ok(qualified) => qualified,
            Err(error) => return Err(CanonicalRetainBatchError::Preflight(error)),
        };

        for index in 0..qualified.len() {
            let result = match source.allocation_plan(index) {
                Some(plan) => self
                    .try_retain_canonical(plan, |emplacer| source.build(index, emplacer, tag))
                    .map(drop),
                None => Err(RetainedPublicationError::Writer(
                    RetainedWriterError::InvalidPublicationPlan,
                )),
            };
            let Err(cause) = result else {
                continue;
            };
            let fields = &mut *self.fields;
            let rollback = match (
                fields.account.as_ref(),
                fields.canonical_table.as_mut(),
                fields.canonical_batch.as_mut(),
            ) {
                (Some(account), Some(table), Some(batch)) => {
                    qualified.try_rollback_prefix(account, table, batch)
                }
                _ => Err(RetainedPublicationError::Poisoned),
            };
            if let Err(rollback_cause) = rollback {
                self.fields.containment = retained_publication_error_containment(&rollback_cause)
                    .unwrap_or(AssemblyContainmentProvenance::NewFaultPending);
                return Err(CanonicalRetainBatchError::Abandoned {
                    failed_item: index,
                    cause: rollback_cause,
                });
            }
            if self.fields.containment != AssemblyContainmentProvenance::Healthy
                || retained_publication_error_containment(&cause).is_some()
            {
                return Err(CanonicalRetainBatchError::Abandoned {
                    failed_item: index,
                    cause,
                });
            }
            return Err(CanonicalRetainBatchError::Execution {
                failed_item: index,
                cause,
            });
        }

        let completed_len = qualified.len();
        let completed = match qualified.try_complete(
            self.fields
                .account
                .as_ref()
                .ok_or_else(|| CanonicalRetainBatchError::Abandoned {
                    failed_item: completed_len.saturating_sub(1),
                    cause: RetainedPublicationError::Poisoned,
                })?,
            self.fields.canonical_table.as_ref().ok_or_else(|| {
                CanonicalRetainBatchError::Abandoned {
                    failed_item: completed_len.saturating_sub(1),
                    cause: RetainedPublicationError::Poisoned,
                }
            })?,
            self.fields.canonical_batch.as_ref().ok_or_else(|| {
                CanonicalRetainBatchError::Abandoned {
                    failed_item: completed_len.saturating_sub(1),
                    cause: RetainedPublicationError::Poisoned,
                }
            })?,
        ) {
            Ok(completed) => completed,
            Err(cause) => {
                self.fields.containment = retained_publication_error_containment(&cause)
                    .unwrap_or(AssemblyContainmentProvenance::NewFaultPending);
                return Err(CanonicalRetainBatchError::Abandoned {
                    failed_item: completed_len.saturating_sub(1),
                    cause,
                });
            }
        };
        Ok(CanonicalRetainedRange::from_completed_batch(completed))
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        self.require_canonical_source(source)?;
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedPublicationError::WrongOutcomeMode);
        }
        let fields = &mut *self.fields;
        let result = try_share_retained_block::<O, P, D, CanonicalBlock, Tag>(
            &mut self
                .operation
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?
                .internal_work_root(),
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.ordinary_handle_share_work,
            fields
                .canonical_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            RetainedMembershipDestination::Canonical,
            RetainedPublicationError::CanonicalSlotLimit,
            &source.inner,
            source_slot.identity(),
        );
        match result {
            Ok(identity) => Ok(CanonicalRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                Err(error)
            }
        }
    }

    /// Rebuilds one canonical block through the exact bound source map and operation work control.
    pub(crate) fn try_rewrite_canonical_controlled<Tag>(
        &mut self,
        source: &PublishedCanonicalAggregate<CanonicalBlock>,
        source_slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
        work_dimension: D,
        allocation_plan: RetainedAllocationPlan,
        build: impl for<'placement, 'source, 'destination, 'control> FnOnce(
            BlockEmplacer<'placement, CanonicalBlock>,
            &'source CanonicalBlock,
            CanonicalRetainedRewriteMap<'source, 'destination, CanonicalBlock>,
            RetainedWorkControl<'control, O, P, PP, U, D>,
        ) -> Result<
            BlockPlacement<'placement, CanonicalBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<CanonicalRetainedSlot<CanonicalBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        self.require_canonical_source(source)?;
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedPublicationError::WrongOutcomeMode);
        }
        let canonical_plan = self
            .fields
            .canonical_plan
            .ok_or(RetainedPublicationError::WrongOutcomeMode)?;
        let (control, mut root) = self
            .operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .split_work_control(work_dimension);
        let fields = &mut *self.fields;
        let result = try_rewrite_canonical_block_controlled(
            &mut root,
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.ordinary_handle_share_work,
            canonical_plan.writer.phase,
            canonical_plan.writer.attribution_dimension,
            fields
                .canonical_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields
                .canonical_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            source,
            source_slot.identity(),
            allocation_plan,
            control,
            build,
        );
        match result {
            Ok(identity) => Ok(CanonicalRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.activate_failure_terminal_work_after_recorded_cause();
        self.ensure_failure_terminal_work_available()?;
        let protected_action =
            self.fields.failure_terminal_work_state == FailureTerminalWorkState::Active;
        let handle_share_work = self.outcome_evidence_handle_share_work()?;
        let fields = &mut *self.fields;
        let result = try_retain_block(
            &mut self
                .operation
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?
                .internal_work_root(),
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            handle_share_work,
            fields.evidence_plan.writer.phase,
            fields.evidence_plan.writer.attribution_dimension,
            fields
                .evidence_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields
                .evidence_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            RetainedMembershipDestination::OutcomeEvidence,
            RetainedPublicationError::OutcomeEvidenceSlotLimit,
            RetainedBlockOrigin::New,
            plan,
            |emplacer, _, _| build(emplacer),
        );
        match result {
            Ok(identity) => Ok(OutcomeEvidenceRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                if protected_action {
                    self.seal_failure_terminal_work();
                }
                Err(error)
            }
        }
    }

    /// Retains success-path outcome evidence while lending ordinary linear work authority to its
    /// constructing closure.
    ///
    /// The selected dimension cannot be the protected failure-terminal dimension. Allocation,
    /// attribution, and table membership remain inside the existing single-block rollback and
    /// panic-containment boundary.
    pub(crate) fn try_retain_success_outcome_evidence_controlled<Tag>(
        &mut self,
        work_dimension: D,
        plan: RetainedAllocationPlan,
        build: impl for<'placement, 'control> FnOnce(
            BlockEmplacer<'placement, EvidenceBlock>,
            RetainedWorkControl<'control, O, P, PP, U, D>,
        ) -> Result<
            BlockPlacement<'placement, EvidenceBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        let (control, mut root) = self
            .operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .split_work_control(work_dimension);
        let fields = &mut *self.fields;
        let result = try_retain_block(
            &mut root,
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.ordinary_handle_share_work,
            fields.evidence_plan.writer.phase,
            fields.evidence_plan.writer.attribution_dimension,
            fields
                .evidence_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields
                .evidence_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            RetainedMembershipDestination::OutcomeEvidence,
            RetainedPublicationError::OutcomeEvidenceSlotLimit,
            RetainedBlockOrigin::New,
            plan,
            move |emplacer, _, root| control.build(root, |control| build(emplacer, control)),
        );
        match result {
            Ok(identity) => Ok(OutcomeEvidenceRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                Err(error)
            }
        }
    }

    pub(crate) fn try_retain_outcome_evidence_controlled<Tag>(
        &mut self,
        plan: RetainedAllocationPlan,
        build: impl for<'placement, 'control> FnOnce(
            BlockEmplacer<'placement, EvidenceBlock>,
            RetainedFailureWorkControl<'control, O, P, PP, U, D>,
        ) -> Result<
            BlockPlacement<'placement, EvidenceBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>, RetainedPublicationError<D>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.activate_failure_terminal_work_after_recorded_cause();
        self.ensure_failure_terminal_work_available()?;
        let protected_action =
            self.fields.failure_terminal_work_state == FailureTerminalWorkState::Active;
        let handle_share_work = self.outcome_evidence_handle_share_work()?;
        let (control, mut root) = self
            .operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .split_failure_terminal_work_control(handle_share_work);
        let fields = &mut *self.fields;
        let result = try_retain_block(
            &mut root,
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            handle_share_work,
            fields.evidence_plan.writer.phase,
            fields.evidence_plan.writer.attribution_dimension,
            fields
                .evidence_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields
                .evidence_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            RetainedMembershipDestination::OutcomeEvidence,
            RetainedPublicationError::OutcomeEvidenceSlotLimit,
            RetainedBlockOrigin::New,
            plan,
            move |emplacer, _, root| control.build(root, |control| build(emplacer, control)),
        );
        match result {
            Ok(identity) => Ok(OutcomeEvidenceRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                if protected_action {
                    self.seal_failure_terminal_work();
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.activate_failure_terminal_work_after_recorded_cause();
        self.ensure_failure_terminal_work_available()?;
        let protected_action =
            self.fields.failure_terminal_work_state == FailureTerminalWorkState::Active;
        let handle_share_work = self.outcome_evidence_handle_share_work()?;
        let fields = &mut *self.fields;
        let result = try_share_retained_block::<O, P, D, EvidenceBlock, Tag>(
            &mut self
                .operation
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?
                .internal_work_root(),
            fields
                .account
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?,
            handle_share_work,
            fields
                .evidence_table
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?,
            fields.set_id,
            RetainedMembershipDestination::OutcomeEvidence,
            RetainedPublicationError::OutcomeEvidenceSlotLimit,
            &source.inner,
            source_slot.identity(),
        );
        match result {
            Ok(identity) => Ok(OutcomeEvidenceRetainedSlot::from_identity(identity)),
            Err(error) => {
                self.observe_publication_error(&error);
                if protected_action {
                    self.seal_failure_terminal_work();
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.ensure_canonical_work_available()?;
        if self.fields.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedPublicationError::WrongOutcomeMode);
        }
        self.try_validate_work_dimension(dimension)?;
        let result = {
            let ledger = self
                .operation
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?
                .common
                .as_read();
            let batch = self
                .fields
                .canonical_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?;
            ledger
                .try_push_commitment(batch, OutputCommitment::new(dimension, amount))
                .map_err(map_commitment_push_error)
        };
        if let Err(error) = &result {
            self.observe_publication_error(error);
        }
        result
    }

    pub fn try_commit_outcome_evidence(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), RetainedPublicationError<D>> {
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        self.activate_failure_terminal_work_after_recorded_cause();
        self.ensure_failure_terminal_work_available()?;
        let protected_action =
            self.fields.failure_terminal_work_state == FailureTerminalWorkState::Active;
        if let Err(error) = self.try_validate_work_dimension(dimension) {
            self.observe_publication_error(&error);
            if protected_action {
                self.seal_failure_terminal_work();
            }
            return Err(error);
        }
        let result = {
            let ledger = self
                .operation
                .as_ref()
                .ok_or(RetainedPublicationError::Poisoned)?
                .common
                .as_read();
            let batch = self
                .fields
                .evidence_batch
                .as_mut()
                .ok_or(RetainedPublicationError::Poisoned)?;
            ledger
                .try_push_commitment(batch, OutputCommitment::new(dimension, amount))
                .map_err(map_commitment_push_error)
        };
        if let Err(error) = &result {
            self.observe_publication_error(error);
            if protected_action {
                self.seal_failure_terminal_work();
            }
        }
        result
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn contain_activation_fault(mut self) -> FinalizationContainmentError {
        self.loan.contain_activation_fault()
    }

    /// Consumes a success-capable assembly and eagerly removes every canonical physical owner.
    ///
    /// Storage is released before its attribution batch. Only a verified evidence-only graph can
    /// be returned; any uncertain release poisons the account and retains all remaining authority
    /// fail-closed.
    pub(crate) fn try_discard_canonical_for_rejection(
        mut self,
        fallback_cause: PreparationFailureCode,
    ) -> RetainedFailureAssemblyResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        self.loan
            .try_discard_canonical_for_rejection(fallback_cause)?;
        Ok(RetainedFailureAssembly {
            assembly: self,
            fallback_cause,
        })
    }

    pub(crate) fn into_verified_failure_only(
        mut self,
        fallback_cause: PreparationFailureCode,
    ) -> RetainedFailureAssemblyResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        self.loan.verify_failure_only(fallback_cause)?;
        Ok(RetainedFailureAssembly {
            assembly: self,
            fallback_cause,
        })
    }

    fn map_success_validation(
        self,
        validation: RetainedClosureValidation<
            RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
        >,
    ) -> RetainedSuccessValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        match validation {
            RetainedClosureValidation::Valid(proof) => {
                RetainedSuccessValidation::Valid(ValidatedRetainedSuccess {
                    assembly: self,
                    proof,
                })
            }
            RetainedClosureValidation::Rejected { cause } => {
                RetainedSuccessValidation::Rejected(RetainedFailureAssembly {
                    assembly: self,
                    fallback_cause: cause,
                })
            }
            RetainedClosureValidation::FallbackOnly {
                cause,
                activation_containment_required,
            } => RetainedSuccessValidation::FallbackOnly(RetainedFallbackOnly {
                assembly: self,
                cause,
                activation_containment_required,
            }),
        }
    }

    /// Binds an already resolved private result to the original consuming owner without replay.
    pub(crate) fn bind_resolved_success(
        self,
        validation: RetainedClosureValidation<
            RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
        >,
    ) -> RetainedSuccessValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        self.map_success_validation(validation)
    }

    fn map_failure_validation(
        self,
        validation: RetainedClosureValidation<RetainedEvidenceClosureProof<EvidenceBlock>>,
    ) -> RetainedFailureValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        match validation {
            RetainedClosureValidation::Valid(proof) => {
                RetainedFailureValidation::Valid(ValidatedRetainedFailure {
                    assembly: self,
                    proof,
                })
            }
            RetainedClosureValidation::Rejected { cause } => {
                RetainedFailureValidation::Rejected(RetainedFailureAssembly {
                    assembly: self,
                    fallback_cause: cause,
                })
            }
            RetainedClosureValidation::FallbackOnly {
                cause,
                activation_containment_required,
            } => RetainedFailureValidation::FallbackOnly(RetainedFallbackOnly {
                assembly: self,
                cause,
                activation_containment_required,
            }),
        }
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
    ) -> RetainedSuccessValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let validation = self
            .loan
            .validate_success(canonical_root, evidence_root, validate);
        self.map_success_validation(validation)
    }

    pub(crate) fn validate_success_controlled<CanonicalTag, EvidenceTag>(
        mut self,
        work_dimension: D,
        scratch_dimension: D,
        canonical_root: CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'session, 'control> FnOnce(
            &mut RetainedClosureSession<'session, CanonicalBlock, EvidenceBlock>,
            CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
            RetainedWorkControl<'control, O, P, PP, U, D>,
        ) -> bool,
    ) -> RetainedSuccessValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let validation = self.loan.validate_success_controlled(
            work_dimension,
            scratch_dimension,
            canonical_root,
            evidence_root,
            validate,
        );
        self.map_success_validation(validation)
    }

    /// Borrows an evidence graph for validation and consumes the writable assembly.
    pub fn validate_rich_failure<EvidenceTag>(
        mut self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedEvidenceClosureSession<'a, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> RetainedFailureValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let validation = self.loan.validate_rich_failure(evidence_root, validate);
        self.map_failure_validation(validation)
    }

    pub(crate) fn validate_rich_failure_controlled<EvidenceTag>(
        mut self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'session, 'control> FnOnce(
            &mut RetainedEvidenceClosureSession<'session, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
            RetainedFailureWorkControl<'control, O, P, PP, U, D>,
        ) -> bool,
    ) -> RetainedFailureValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let validation = self
            .loan
            .validate_rich_failure_controlled(evidence_root, validate);
        self.map_failure_validation(validation)
    }

    /// Discards every block table before attribution and publishes the pre-admitted fallback.
    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        self.prepare_minimal_fallback_with(PreparationFailureCode::requested())
    }

    pub(crate) fn prepare_minimal_fallback_with(
        self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        self.try_prepare_terminal(RetainedTerminalRequest::MinimalFallback {
            requested_cause: cause,
        })
    }

    fn try_prepare_terminal(
        self,
        request: RetainedTerminalRequest<CanonicalBlock, EvidenceBlock, PP, U>,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        self.try_prepare_terminal_with_control(request)
            .map(|(prepared, _control_cause)| prepared)
    }

    fn try_prepare_terminal_with_control(
        self,
        request: RetainedTerminalRequest<CanonicalBlock, EvidenceBlock, PP, U>,
    ) -> Result<
        PreparedRetainedPublicationWithControl<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        let (preparation, control_cause) = self.try_form_terminal_preparation(request)?;
        let prepared = match preparation {
            RetainedSuccessJointCommitFormation::Intent(intent) => intent
                .qualify_and_commit()
                .map_err(RetainedPublicationPrepareError::FinalizationContainment)?,
            RetainedSuccessJointCommitFormation::PreparedFailure(prepared) => prepared,
        };
        Ok((prepared, control_cause))
    }

    fn try_form_terminal_preparation(
        self,
        request: RetainedTerminalRequest<CanonicalBlock, EvidenceBlock, PP, U>,
    ) -> Result<
        RetainedJointCommitFormationWithControl<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        let fields = match operation::DynamicTerminalRootFields::from_assembly(self) {
            Ok(fields) => fields,
            Err(mut assembly) => {
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    assembly
                        .loan
                        .poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified),
                ));
            }
        };
        fields.finish_publication(request)
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    /// Preserves the distinct original Valid/Rejected cause-latching effects.
    pub(crate) fn apply_success_rejection(
        &mut self,
        validation: RetainedClosureValidation<
            RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
        >,
        cause: PreparationFailureCode,
    ) -> RetainedClosureValidation<RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>> {
        match validation {
            RetainedClosureValidation::Valid(_) => {
                self.activate_failure_terminal_work();
                RetainedClosureValidation::Rejected { cause }
            }
            RetainedClosureValidation::Rejected { .. } => {
                self.record_terminal_cause(cause);
                RetainedClosureValidation::Rejected { cause }
            }
            fallback @ RetainedClosureValidation::FallbackOnly { .. } => fallback,
        }
    }

    pub(crate) fn try_issue_freeze_decision(
        &mut self,
        proof: &RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
        canonical_dimension: D,
        outcome_evidence_dimension: D,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<RetainedSuccessFreezeDecision, FinalizationContainmentError> {
        if !self.success_proof_is_current(proof) || !self.has_exact_storage_graph() {
            return Err(self.contain_activation_fault());
        }
        let decision = self.try_stage_freeze_permit(
            proof,
            canonical_dimension,
            outcome_evidence_dimension,
            final_progress,
        )?;
        match &decision {
            RetainedSuccessFreezeDecision::Ready => {}
            RetainedSuccessFreezeDecision::RichFailure { cause, .. }
            | RetainedSuccessFreezeDecision::DestinationRejected { cause } => {
                self.try_discard_canonical_for_rejection(*cause)?;
            }
        }
        Ok(decision)
    }

    fn try_stage_freeze_permit(
        &mut self,
        proof: &RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
        canonical_dimension: D,
        outcome_evidence_dimension: D,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<RetainedSuccessFreezeDecision, FinalizationContainmentError> {
        if self.canonical_commitment_len() != 0
            || self.outcome_evidence_commitment_len() != 0
            || !self.contains_dimension(canonical_dimension)
            || !self.contains_dimension(outcome_evidence_dimension)
        {
            return Err(self.contain_activation_fault());
        }
        let Some(canonical_amount) = u64::try_from(proof.canonical_count)
            .ok()
            .and_then(NonZeroU64::new)
        else {
            return Err(self.contain_activation_fault());
        };
        let Some(evidence_amount) = u64::try_from(proof.evidence_count)
            .ok()
            .and_then(NonZeroU64::new)
        else {
            return Err(self.contain_activation_fault());
        };
        let staged = {
            let Some(operation) = self.operation.as_ref() else {
                return Err(self.contain_activation_fault());
            };
            let ledger = operation.common.as_read();
            let fields = &mut *self.fields;
            let (Some(canonical_batch), Some(evidence_batch)) = (
                fields.canonical_batch.as_mut(),
                fields.evidence_batch.as_mut(),
            ) else {
                return Err(self.contain_activation_fault());
            };
            operation.retained.baseline.batch_read().map(|baseline| {
                ledger.try_qualify_and_push_exact_primary_commitment_pair_rows(
                    baseline,
                    canonical_batch,
                    OutputCommitment::new(canonical_dimension, canonical_amount.get()),
                    evidence_batch,
                    OutputCommitment::new(outcome_evidence_dimension, evidence_amount.get()),
                )
            })
        };
        let Some(staged) = staged else {
            return Err(self.contain_activation_fault());
        };
        if let Err(cause) = staged {
            let publication_cause = match cause {
                CommitmentPairPushError::CanonicalBatchFull
                | CommitmentPairPushError::OutcomeEvidenceBatchFull => {
                    RetainedPublicationError::Writer(RetainedWriterError::CommitmentBatchFull)
                }
                CommitmentPairPushError::Ledger(cause) => {
                    RetainedPublicationError::Writer(RetainedWriterError::Ledger(cause))
                }
            };
            self.observe_publication_error(&publication_cause);
            return Ok(RetainedSuccessFreezeDecision::DestinationRejected {
                cause: PreparationFailureCode::ledger(),
            });
        }
        if self.canonical_commitment_len() != 1 || self.outcome_evidence_commitment_len() != 1 {
            return Err(self.contain_activation_fault());
        }

        let (terminal_cause, control_cause) = match self.operation.as_mut() {
            Some(operation) => {
                let control_cause = operation.prepare_terminal_control(final_progress);
                if let Some(cause) = control_cause.as_ref() {
                    operation.record_terminal_cause(PreparationFailureCode::from_control(cause));
                }
                (operation.sticky_terminal_cause(), control_cause)
            }
            None => return Err(self.contain_activation_fault()),
        };
        if let Some(cause) = terminal_cause {
            return Ok(RetainedSuccessFreezeDecision::RichFailure {
                cause,
                control_cause,
            });
        }
        Ok(RetainedSuccessFreezeDecision::Ready)
    }

    fn try_stage_exact_outcome_evidence(
        &mut self,
        proof: &RetainedEvidenceClosureProof<EvidenceBlock>,
        dimension: D,
    ) -> Result<(), RetainedPublicationError<D>> {
        if self.outcome_evidence_commitment_len() != 0 {
            return Err(RetainedPublicationError::Poisoned);
        }
        let Some(amount) = u64::try_from(proof.evidence_count)
            .ok()
            .and_then(NonZeroU64::new)
        else {
            return Err(RetainedPublicationError::Poisoned);
        };
        self.try_commit_outcome_evidence(dimension, amount.get())?;
        if self.outcome_evidence_commitment_len() != 1 {
            return Err(RetainedPublicationError::Poisoned);
        }
        Ok(())
    }

    fn try_classify_terminal_preparation(
        mut self,
        request: RetainedTerminalRequest<CanonicalBlock, EvidenceBlock, PP, U>,
    ) -> RetainedPreFreezeResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        if let Some(operation) = self.operation.as_ref() {
            operation.assert_backend();
        }
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            let cause = match self.fields.containment {
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
            && match (self.fields.account.as_ref(), self.operation.as_ref()) {
                (Some(account), Some(operation)) => {
                    account.is_same_account(operation.retained.baseline.account())
                }
                _ => false,
            };
        if !exact_storage {
            let cause = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                cause,
            ));
        }

        let frozen_success = matches!(&request, RetainedTerminalRequest::FrozenSuccess { .. });
        let mut observed_control_cause = None;
        let mut terminal = match request {
            RetainedTerminalRequest::Success {
                proof,
                final_progress,
            } => {
                let proof_valid = self.fields.mode == RetainedOutcomeMode::SuccessCapable
                    && proof.set_id == self.fields.set_id
                    && self.fields.canonical_table.as_ref().is_some_and(|table| {
                        table.as_ref().member_count() == proof.canonical_count
                            && table
                                .as_ref()
                                .member(proof.canonical_root, self.fields.set_id)
                                .is_some()
                    })
                    && self.fields.evidence_table.as_ref().is_some_and(|table| {
                        table.as_ref().member_count() == proof.evidence_count
                            && table
                                .as_ref()
                                .member(proof.evidence_root, self.fields.set_id)
                                .is_some()
                    });
                if !proof_valid {
                    let cause =
                        self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                    return Err(RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ));
                }
                let terminal_cause = match self.operation.as_mut() {
                    Some(operation) => {
                        let control_cause = operation.prepare_terminal_control(final_progress);
                        observed_control_cause = control_cause.clone();
                        if let Some(cause) = control_cause.as_ref() {
                            operation
                                .record_terminal_cause(PreparationFailureCode::from_control(cause));
                        }
                        operation.sticky_terminal_cause()
                    }
                    None => {
                        let cause = self
                            .poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                        return Err(RetainedPublicationPrepareError::FinalizationContainment(
                            cause,
                        ));
                    }
                };
                terminal_cause.map_or(
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
            RetainedTerminalRequest::FrozenSuccess { proof } => {
                let proof_valid = self.fields.mode == RetainedOutcomeMode::SuccessCapable
                    && proof.set_id == self.fields.set_id
                    && self.canonical_commitment_len() == 1
                    && self.outcome_evidence_commitment_len() == 1
                    && self.fields.canonical_table.as_ref().is_some_and(|table| {
                        table.as_ref().member_count() == proof.canonical_count
                            && table
                                .as_ref()
                                .member(proof.canonical_root, self.fields.set_id)
                                .is_some()
                    })
                    && self.fields.evidence_table.as_ref().is_some_and(|table| {
                        table.as_ref().member_count() == proof.evidence_count
                            && table
                                .as_ref()
                                .member(proof.evidence_root, self.fields.set_id)
                                .is_some()
                    });
                let no_late_cause = self
                    .operation
                    .as_ref()
                    .is_some_and(|operation| operation.sticky_terminal_cause().is_none());
                if !proof_valid || !no_late_cause {
                    let cause =
                        self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                    return Err(RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ));
                }
                RetainedTerminalMode::Success {
                    canonical_root: proof.canonical_root,
                    evidence_root: proof.evidence_root,
                }
            }
            RetainedTerminalRequest::RichFailure {
                proof,
                requested_cause,
                final_progress,
            } => {
                let proof_valid = proof.set_id == self.fields.set_id
                    && self.fields.evidence_table.as_ref().is_some_and(|table| {
                        table.as_ref().member_count() == proof.evidence_count
                            && table
                                .as_ref()
                                .member(proof.evidence_root, self.fields.set_id)
                                .is_some()
                    });
                if !proof_valid {
                    let cause =
                        self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                    return Err(RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ));
                }
                let terminal_cause = match self.operation.as_mut() {
                    Some(operation) => {
                        let control_cause = operation.prepare_terminal_control(final_progress);
                        observed_control_cause = control_cause.clone();
                        if let Some(cause) = control_cause.as_ref() {
                            operation
                                .record_terminal_cause(PreparationFailureCode::from_control(cause));
                        }
                        operation.sticky_terminal_cause()
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
                    cause: terminal_cause.unwrap_or(requested_cause),
                }
            }
            RetainedTerminalRequest::FrozenRichFailure {
                proof,
                requested_cause,
            } => {
                let proof_valid = self.fields.mode == RetainedOutcomeMode::FailureOnly
                    && proof.set_id == self.fields.set_id
                    && self.canonical_ownership_is_absent()
                    && self.outcome_evidence_commitment_len() == 1
                    && self.fields.evidence_table.as_ref().is_some_and(|table| {
                        table.as_ref().member_count() == proof.evidence_count
                            && table
                                .as_ref()
                                .member(proof.evidence_root, self.fields.set_id)
                                .is_some()
                    });
                let terminal_cause = self
                    .operation
                    .as_ref()
                    .and_then(|operation| operation.sticky_terminal_cause())
                    .unwrap_or(requested_cause);
                if !proof_valid {
                    let cause =
                        self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                    return Err(RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ));
                }
                RetainedTerminalMode::RichFailure {
                    evidence_root: proof.evidence_root,
                    cause: terminal_cause,
                }
            }
            RetainedTerminalRequest::MinimalFallback { requested_cause } => {
                let cause = match self.operation.as_mut() {
                    Some(operation) => operation.sticky_terminal_cause().unwrap_or(requested_cause),
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

        if let Some(operation) = self.operation.as_mut() {
            let disposal_cause = operation.close_terminal_owners();
            if observed_control_cause.is_none() {
                observed_control_cause = disposal_cause;
            }
            if let Some(cause) = operation.sticky_terminal_cause() {
                terminal = match terminal {
                    RetainedTerminalMode::Success { evidence_root, .. }
                    | RetainedTerminalMode::RichFailure { evidence_root, .. } => {
                        RetainedTerminalMode::RichFailure {
                            evidence_root,
                            cause,
                        }
                    }
                    RetainedTerminalMode::MinimalFallback { .. } => {
                        RetainedTerminalMode::MinimalFallback { cause }
                    }
                };
            }
        }

        if self.operation.is_none()
            || self.fields.evidence_batch.is_none()
            || self.fields.account.is_none()
        {
            let cause = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                cause,
            ));
        }
        let operation = match self.operation.take() {
            Some(operation) => operation,
            None => unreachable_final_owner_state(),
        };
        let fields = self.fields.take();
        let set_id = fields.set_id;
        let RetainedOperationLoan {
            retained,
            common,
            terminal: terminal_slot,
            ..
        } = operation;
        let ledger = common.into_terminal();
        let (physical, baseline_batches, primary_batches) =
            terminal_slot.install(retained.baseline, Some(fields));

        if let RetainedTerminalMode::MinimalFallback { cause } = terminal {
            let attribution =
                TerminalFallbackAttribution::new(ledger, baseline_batches, primary_batches);
            return Ok(RetainedPreFreezeClassification {
                state: TerminalPreFreezeTransition::Fallback {
                    guard: TerminalFallbackGuard::new(attribution, physical),
                    cause,
                },
                set_id,
                control_cause: observed_control_cause,
            });
        }

        let primary =
            TerminalPrimaryGuard::new(ledger, baseline_batches, primary_batches, physical);
        let state = if frozen_success {
            primary.try_classify_frozen_success(terminal)?
        } else {
            primary.try_classify(terminal)?
        };
        Ok(RetainedPreFreezeClassification {
            state,
            set_id,
            control_cause: observed_control_cause,
        })
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
        if self.fields.containment != AssemblyContainmentProvenance::Healthy {
            return false;
        }
        let (Some(evidence), Some(evidence_batch), Some(account)) = (
            self.fields.evidence_table.as_ref(),
            self.fields.evidence_batch.as_ref(),
            self.fields.account.as_ref(),
        ) else {
            return false;
        };
        let evidence_exact = retained_block_table_is_exact(
            evidence,
            evidence_batch,
            account,
            self.fields.set_id,
            RetainedMembershipDestination::OutcomeEvidence,
        );
        let canonical_exact = match self.fields.mode {
            RetainedOutcomeMode::SuccessCapable => {
                match (
                    self.fields.canonical_table.as_ref(),
                    self.fields.canonical_batch.as_ref(),
                ) {
                    (Some(canonical), Some(canonical_batch)) => retained_block_table_is_exact(
                        canonical,
                        canonical_batch,
                        account,
                        self.fields.set_id,
                        RetainedMembershipDestination::Canonical,
                    ),
                    _ => false,
                }
            }
            RetainedOutcomeMode::FailureOnly => {
                self.fields.canonical_table.is_none() && self.fields.canonical_batch.is_none()
            }
        };
        evidence_exact && canonical_exact
    }

    fn first_containment_account(&self) -> Option<&PhysicalStorageAccount> {
        self.fields
            .account
            .as_ref()
            .or_else(|| {
                self.operation
                    .as_ref()
                    .and_then(|operation| operation.retained.baseline.first_account())
            })
            .or_else(|| {
                self.fields
                    .canonical_table
                    .as_ref()
                    .map(|table| table.account())
            })
            .or_else(|| {
                self.fields.canonical_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| {
                self.fields
                    .evidence_table
                    .as_ref()
                    .map(|table| table.account())
            })
            .or_else(|| {
                self.fields.evidence_table.as_ref().and_then(|table| {
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
        if let Some(canonical) = self.fields.canonical_table.as_ref() {
            record_forgotten_table_edges(canonical);
        }
        if let Some(evidence) = self.fields.evidence_table.as_ref() {
            record_forgotten_table_edges(evidence);
        }
        if let Some(operation) = self.operation.as_ref() {
            operation
                .retained
                .baseline
                .record_forgotten_physical_edges();
        }
        FinalizationContainmentError::new(AccountPoisoned::new(account.snapshot()), reason)
    }

    fn record_new_fault(&self, reason: AccountPoisonReason) {
        let account = match self
            .fields
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
        if let Some(canonical) = self.fields.canonical_table.take() {
            std::mem::forget(canonical);
        }
        if let Some(evidence) = self.fields.evidence_table.take() {
            std::mem::forget(evidence);
        }
        self.fields.retain_batches_fail_closed();
        if let Some(account) = self.fields.account.take() {
            drop(account);
        }
        if let Some(mut operation) = self.operation.take() {
            operation.record_terminal_cause(PreparationFailureCode::product_graph(
                StoreExecutionState::ABANDONED,
            ));
            operation.stop_for_containment();
            operation
                .retained
                .baseline
                .fail_closed_after_recorded_poison();
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
        let fields = &mut *self.fields;
        let Some(account) = fields.account.as_ref() else {
            return RetainedStorageDropOutcome::MissingFaultAnchor;
        };
        let canonical = match fields.canonical_table.take() {
            Some(canonical) => drop_retained_table_with_account_anchor(canonical, account),
            None => RetainedStorageDropOutcome::Clean,
        };
        let evidence = match fields.evidence_table.take() {
            Some(evidence) => drop_retained_table_with_account_anchor(evidence, account),
            None => RetainedStorageDropOutcome::ForeignOwnershipFault(record_account_poison_event(
                account,
                AccountPoisonReason::OwnershipGraphUnverified,
            )),
        };
        let outcome = canonical.join(evidence);
        if outcome != RetainedStorageDropOutcome::Clean {
            if canonical == RetainedStorageDropOutcome::Clean {
                drop(self.fields.canonical_batch.take());
            }
            if evidence == RetainedStorageDropOutcome::Clean {
                drop(self.fields.evidence_batch.take());
            }
        }
        outcome
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    ValidatedRetainedSuccess<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    #[cfg(test)]
    pub(crate) fn test_dimension_evidence(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.assembly.loan.test_dimension_evidence(dimension)
    }

    #[cfg(test)]
    fn test_probe_freeze_permit_qualification(
        mut self,
        canonical_dimension: D,
        outcome_evidence_dimension: D,
    ) -> FreezePermitQualificationProbe<D> {
        let canonical_amount = u64::try_from(self.proof.canonical_count)
            .ok()
            .and_then(NonZeroU64::new)
            .unwrap_or_else(|| panic!("test success must retain canonical members"));
        let evidence_amount = u64::try_from(self.proof.evidence_count)
            .ok()
            .and_then(NonZeroU64::new)
            .unwrap_or_else(|| panic!("test success must retain outcome-evidence members"));
        let result = {
            let operation = self
                .assembly
                .loan
                .operation
                .as_ref()
                .unwrap_or_else(|| panic!("test success must retain its operation"));
            let ledger = operation.common.as_read();
            let fields = &mut *self.assembly.loan.fields;
            let canonical_batch = fields
                .canonical_batch
                .as_mut()
                .unwrap_or_else(|| panic!("test success must retain its canonical batch"));
            let evidence_batch = fields
                .evidence_batch
                .as_mut()
                .unwrap_or_else(|| panic!("test success must retain its evidence batch"));
            operation
                .retained
                .baseline
                .batch_read()
                .map(|baseline| {
                    ledger.try_qualify_and_push_exact_primary_commitment_pair_rows(
                        baseline,
                        canonical_batch,
                        OutputCommitment::new(canonical_dimension, canonical_amount.get()),
                        evidence_batch,
                        OutputCommitment::new(outcome_evidence_dimension, evidence_amount.get()),
                    )
                })
                .unwrap_or_else(|| panic!("test success must retain its startup baseline"))
        };
        match result {
            Ok(()) => FreezePermitQualificationProbe::Ready,
            Err(cause) => {
                let dimension = self
                    .assembly
                    .loan
                    .test_dimension_evidence(canonical_dimension)
                    .unwrap_or_else(|| panic!("test dimension must remain observable"));
                FreezePermitQualificationProbe::Rejected {
                    cause,
                    canonical_commitments: self.assembly.loan.canonical_commitment_len(),
                    outcome_evidence_commitments: self
                        .assembly
                        .loan
                        .outcome_evidence_commitment_len(),
                    dimension,
                }
            }
        }
    }

    pub(crate) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.assembly.contain_activation_fault()
    }

    pub(crate) fn defer_until_body_cleanup(mut self) -> Result<(), Self> {
        match self.assembly.loan.prepare_body_deferral() {
            Some(Ok(())) => {}
            Some(Err(_)) => {
                if let Some(operation) = self.assembly.loan.operation.as_mut() {
                    operation.record_ledger_failure();
                }
                return Err(self);
            }
            None => return Err(self),
        }
        let Self { assembly, proof } = self;
        assembly
            .loan
            .install_deferred_stage(RetainedDeferredStage::Success { proof });
        Ok(())
    }

    #[allow(
        clippy::type_complexity,
        reason = "the private freeze-permit result keeps the full retained ownership graph in each linear branch"
    )]
    pub(crate) fn try_issue_freeze_permit(
        mut self,
        canonical_dimension: D,
        outcome_evidence_dimension: D,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        RetainedSuccessFreezePermitIssue<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        FinalizationContainmentError,
    > {
        let decision = self.assembly.loan.try_issue_freeze_decision(
            &self.proof,
            canonical_dimension,
            outcome_evidence_dimension,
            final_progress,
        )?;
        match decision {
            RetainedSuccessFreezeDecision::Ready => Ok(RetainedSuccessFreezePermitIssue::Ready(
                RetainedSuccessFreezePermit { validated: self },
            )),
            RetainedSuccessFreezeDecision::RichFailure {
                cause,
                control_cause,
            } => {
                let ValidatedRetainedSuccess { assembly, proof: _ } = self;
                Ok(RetainedSuccessFreezePermitIssue::RichFailure {
                    failure: RetainedFailureAssembly {
                        assembly,
                        fallback_cause: cause,
                    },
                    cause,
                    control_cause,
                })
            }
            RetainedSuccessFreezeDecision::DestinationRejected { cause } => {
                let ValidatedRetainedSuccess { assembly, proof: _ } = self;
                Ok(RetainedSuccessFreezePermitIssue::DestinationRejected {
                    failure: RetainedFailureAssembly {
                        assembly,
                        fallback_cause: cause,
                    },
                    cause,
                })
            }
        }
    }

    /// Consumes a valid success proof after a later domain admission rejects the canonical result.
    ///
    /// Canonical ownership remains sealed and inaccessible until the existing rich-failure
    /// terminal transition drops it before attribution. Evidence remains writable and may be
    /// revalidated without forcing the minimal fallback.
    pub fn downgrade_to_rich_failure(
        self,
        fallback_cause: PreparationFailureCode,
    ) -> RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        let mut assembly = self.assembly;
        assembly.loan.activate_failure_terminal_work();
        RetainedFailureAssembly {
            assembly,
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
        match self.try_form_joint_commit_intent(final_progress)? {
            RetainedSuccessJointCommitFormation::Intent(intent) => intent
                .qualify_and_commit()
                .map_err(RetainedPublicationPrepareError::FinalizationContainment),
            RetainedSuccessJointCommitFormation::PreparedFailure(prepared) => Ok(prepared),
        }
    }

    pub(crate) fn try_form_joint_commit_intent(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        RetainedSuccessJointCommitFormation<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        self.assembly
            .try_form_terminal_preparation(RetainedTerminalRequest::Success {
                proof: self.proof,
                final_progress,
            })
            .map(|(formation, _control_cause)| formation)
    }

    #[cfg(test)]
    pub(crate) fn test_classify_pre_freeze(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> RetainedPreFreezeResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        self.assembly.into_loan().try_classify_terminal_preparation(
            RetainedTerminalRequest::Success {
                proof: self.proof,
                final_progress,
            },
        )
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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedSuccessFreezePermit<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.validated.contain_activation_fault()
    }

    pub(crate) fn terminal_observation(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.validated
            .assembly
            .loan
            .operation
            .as_ref()
            .and_then(|operation| {
                operation
                    .common
                    .as_read()
                    .dimension_evidence(dimension)
                    .ok()
            })
    }

    #[allow(
        clippy::type_complexity,
        reason = "generation rejection returns the exact evidence-only retained owner without a detachable alias"
    )]
    pub(crate) fn reject_generation(
        self,
        cause: PreparationFailureCode,
    ) -> Result<
        RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        FinalizationContainmentError,
    > {
        let ValidatedRetainedSuccess { assembly, proof: _ } = self.validated;
        assembly.try_discard_canonical_for_rejection(cause)
    }

    pub(crate) fn into_contribution(
        self,
    ) -> RetainedSuccessFreezeContribution<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    {
        RetainedSuccessFreezeContribution {
            validated: self.validated,
        }
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedSuccessFreezeContribution<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.validated.contain_activation_fault()
    }

    pub(crate) fn try_form_joint_commit_intent(
        self,
    ) -> Result<
        RetainedSuccessJointCommitIntent<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        let ValidatedRetainedSuccess { assembly, proof } = self.validated;
        match assembly
            .try_form_terminal_preparation(RetainedTerminalRequest::FrozenSuccess { proof })?
            .0
        {
            RetainedSuccessJointCommitFormation::Intent(intent) => Ok(intent),
            RetainedSuccessJointCommitFormation::PreparedFailure(_) => {
                unreachable_final_owner_state()
            }
        }
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.assembly.contain_activation_fault()
    }

    pub fn outcome_evidence_len(&self) -> usize {
        self.assembly.loan.outcome_evidence_len()
    }

    pub fn outcome_evidence_new_blocks(&self) -> usize {
        self.assembly.loan.outcome_evidence_new_blocks()
    }

    pub fn outcome_evidence_commitment_len(&self) -> usize {
        self.assembly.loan.outcome_evidence_commitment_len()
    }

    pub fn validate_rich_failure<EvidenceTag>(
        self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'a> FnOnce(
            &mut RetainedEvidenceClosureSession<'a, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        ) -> bool,
    ) -> RetainedFailureValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.assembly.validate_rich_failure(evidence_root, validate)
    }

    pub(crate) fn validate_rich_failure_controlled<EvidenceTag>(
        self,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'session, 'control> FnOnce(
            &mut RetainedEvidenceClosureSession<'session, EvidenceBlock>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
            RetainedFailureWorkControl<'control, O, P, PP, U, D>,
        ) -> bool,
    ) -> RetainedFailureValidation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.assembly
            .validate_rich_failure_controlled(evidence_root, validate)
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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    ValidatedRetainedFailure<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn outcome_evidence_commitment_len(&self) -> usize {
        self.assembly.loan.outcome_evidence_commitment_len()
    }

    pub(crate) fn try_stage_exact_outcome_evidence(
        &mut self,
        dimension: D,
    ) -> Result<(), RetainedPublicationError<D>> {
        self.assembly
            .loan
            .try_stage_exact_outcome_evidence(&self.proof, dimension)
    }

    pub(crate) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.assembly.contain_activation_fault()
    }

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

    pub(crate) fn try_prepare_with_control(
        self,
        cause: PreparationFailureCode,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        PreparedRetainedPublicationWithControl<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly
            .try_prepare_terminal_with_control(RetainedTerminalRequest::RichFailure {
                proof: self.proof,
                requested_cause: cause,
                final_progress,
            })
    }

    pub(crate) fn try_prepare_frozen(
        self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly
            .try_prepare_terminal(RetainedTerminalRequest::FrozenRichFailure {
                proof: self.proof,
                requested_cause: cause,
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

    pub(crate) fn prepare_minimal_fallback_with(
        self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        self.assembly.prepare_minimal_fallback_with(cause)
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedFallbackOnly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) const fn activation_containment_required(&self) -> bool {
        self.activation_containment_required
    }

    pub(crate) fn defer_until_body_cleanup(mut self) -> Result<(), Self> {
        if !matches!(self.assembly.loan.prepare_body_deferral(), Some(Ok(()))) {
            return Err(self);
        }
        let Self {
            assembly,
            cause,
            activation_containment_required,
        } = self;
        assembly
            .loan
            .install_deferred_stage(RetainedDeferredStage::Fallback {
                cause,
                activation_containment_required,
            });
        Ok(())
    }

    pub(crate) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.assembly.contain_activation_fault()
    }

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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
        if self.fields.fields.is_none() {
            return;
        }
        if self.fields.canonical_table.is_none()
            && self.fields.evidence_table.is_none()
            && self.fields.canonical_batch.is_none()
            && self.fields.evidence_batch.is_none()
            && self.fields.account.is_none()
            && self.operation.is_none()
        {
            return;
        }
        match self.fields.containment {
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
        if self
            .operation
            .as_ref()
            .is_some_and(|operation| operation.common.terminal_is_outstanding())
        {
            let _ = self.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return;
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
        drop(self.fields.canonical_batch.take());
        drop(self.fields.evidence_batch.take());
        drop(self.fields.account.take());
        self.operation = None;
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuccessGuardDropStage {
    Qualified,
    Verified,
    PublicationReady,
    EligibilityReady,
    VerifiedInProgress,
    LedgerDetached,
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
    EligibilityReady,
    CanonicalAcknowledged,
    DiscardCleaned,
    FreezeReady,
    LedgerDetached,
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
    static FAIL_NEXT_TERMINAL_EVIDENCE_CATALOG: std::cell::Cell<bool> = const {
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
    static STARTUP_READY_FINAL_OWNER_ALLOCATOR_ENTRIES: std::cell::Cell<Option<usize>> = const {
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
pub(crate) fn fail_next_final_owner_allocation() {
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
fn fail_next_terminal_evidence_catalog_qualification() {
    FAIL_NEXT_TERMINAL_EVIDENCE_CATALOG.with(|failure| failure.set(true));
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
fn take_injected_terminal_evidence_catalog_failure() -> bool {
    FAIL_NEXT_TERMINAL_EVIDENCE_CATALOG.with(|failure| failure.replace(false))
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
pub(crate) fn take_startup_ready_final_owner_allocator_entries() -> Option<usize> {
    STARTUP_READY_FINAL_OWNER_ALLOCATOR_ENTRIES.with(std::cell::Cell::take)
}

#[cfg(test)]
pub(crate) fn reset_startup_payload_allocation_attempts() {
    operation_start::reset_startup_payload_allocation_attempts();
}

#[cfg(test)]
pub(crate) fn test_startup_payload_allocation_attempts() -> (usize, usize) {
    operation_start::test_startup_payload_allocation_attempts()
}

#[cfg(test)]
fn mismatch_next_startup_fallback_footprint() {
    operation_start::mismatch_next_startup_fallback_footprint();
}

#[cfg(test)]
fn mismatch_next_startup_evidence_footprint() {
    operation_start::mismatch_next_startup_evidence_footprint();
}

#[cfg(test)]
fn reset_startup_footprint_mismatches() {
    operation_start::reset_startup_footprint_mismatches();
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
