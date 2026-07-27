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
mod operation;
mod operation_start;
mod ownership;
mod physical;
mod publication_contract;
mod range;
mod startup;
mod terminal;
mod terminal_failure;
mod terminal_output;

#[cfg(test)]
use crate::OperationControl;
use crate::control::discard_untrusted_panic_payload;
use crate::coordinator::{FinalProgress, OperationCoordinator};
#[cfg(test)]
use crate::ledger::{AttributionBatchCreateError, LedgerLimits};
use crate::ledger::{
    AttributionReservationBatch, CanonicalOutput, CommitmentPushError, DimensionSnapshot,
    FallbackBatchBaseline, FreezeReadyTypedProof, Ledger, LedgerCause, OutcomeEvidenceDestination,
    OutputCommitment, PhaseSnapshot, QualifiedBothDiscardFreezeProof,
    QualifiedEvidenceDiscardFreezeProof, QualifiedTypedDiscardFreezeProof,
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
#[cfg(test)]
pub(crate) use allocation::{
    reset_retained_final_owner_allocator_entries, test_retained_final_owner_allocator_entries,
};
pub(super) use block_table::RetainedSlotIdentity;
#[cfg(test)]
use block_table::try_share_retained_block_after_admission;
pub(crate) use block_table::{
    BlockEmplacer, BlockPlacement, CanonicalRetainedSlot, OutcomeEvidenceRetainedSlot,
    PublishedCanonicalAggregate, PublishedOutcomeEvidenceAggregate, RetainedBlockTablePlan,
    RetainedClosureSession, RetainedEvidenceClosureSession, RetainedLookupMember,
};
use block_table::{
    QualifiedRetainedBlockTablePlan, RetainedTableAllocation, record_forgotten_table_edges,
    retained_block_table_is_exact, try_create_retained_block_table,
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
};
#[cfg(test)]
use ownership::{RetainedStorageDropSeverity, join_pre_attribution_cleanup_cause};
#[cfg(test)]
use physical::AccountControlInner;
#[cfg(test)]
use physical::RetainedAdmissionPairPlan;
#[cfg(test)]
use physical::{
    AccountActionAdmissionError, AdmittedRetainedShareWork, PendingRetainedRollback,
    PendingStartRetainedAdmissions,
};
use physical::{
    AccountLinearState, PendingAccountActionAdmission, PendingRetainedAdmission,
    PublicationEligibility,
};
pub use physical::{
    PhysicalStorageAccount, PhysicalStorageAccountCreateError, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryCreateError,
};
use std::any::TypeId;
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
use terminal::{
    DetachedTerminalEvidenceQualification, FreezeReadyDetachedTerminalEvidence,
    PrimaryPublicationBatches, QualifiedDetachedTerminalEvidence, TerminalFallbackAttribution,
    TerminalFallbackGuard, TerminalPhysicalOwnershipManifest, TerminalPrimaryGuard,
    TerminalPrimaryTransition, drop_retained_table_with_account_anchor, prepare_terminal_control,
    terminal_evidence_catalog_is_qualified,
};
use terminal_failure::{TerminalMinimalFreezeGuard, TerminalRichFreezeGuard};
use terminal_output::RetainedPublicationPayload;
#[allow(
    unused_imports,
    reason = "the retained root preserves crate-private paths while canonical consumers migrate"
)]
pub(crate) use terminal_output::{
    PreparedRetainedFallback, PreparedRetainedPublication, PreparedRetainedPublicationOutcome,
    PreparedRetainedRichFailure, PreparedRetainedSuccess, PublishedCanonicalOutput,
    PublishedFallbackOutput, PublishedLedgerEvidence, PublishedOutcomeEvidenceOutput,
    PublishedRetainedFallbackParts, PublishedRetainedRichFailureParts,
    PublishedRetainedSuccessParts, PublishedRichFailureOutput,
};
use triomphe::{Arc, UniqueArc};

pub(crate) use operation::{RetainedControlCheckpoint, RetainedOperation, RetainedWorkControl};
#[cfg(test)]
use operation_start::map_qualified_startup_batch_error;
pub(crate) use operation_start::{
    RetainedBatchLimits, RetainedHandleSharePlan, RetainedOperationStartError,
    RetainedOperationStartPlan, RetainedWriterError, RetainedWriterPlan,
};
use operation_start::{
    StartupRetainedPartGuard, StartupTerminalEvidenceBuildError,
    map_account_action_admission_error, map_batch_error, map_faulted_pending_rollback,
    map_pending_rollback_or, try_admit_retained_share_work,
    try_admit_retained_share_work_before_physical, try_open_publication_account,
};
pub(super) use publication_contract::retained_publication_error_requires_containment;
use publication_contract::{
    AssemblyContainmentProvenance, RetainedClosureValidationError, RetainedEvidenceClosureProof,
    RetainedOutcomeMode, RetainedSuccessClosureProof, RetainedTerminalMode,
    RetainedTerminalRequest, map_commitment_push_error, retained_publication_error_containment,
};
pub(crate) use publication_contract::{
    RetainedPublicationError, RetainedPublicationOpenError, RetainedPublicationPlan,
};
use range::{CanonicalBatchPreflightContext, try_preflight_canonical_batch};
pub(crate) use range::{
    CanonicalRetainBatchError, CanonicalRetainBatchSource, CanonicalRetainedRange,
};
pub(crate) use startup::{ReadyStartupGraphSplitAuthority, StartedLedger};
use startup::{
    StartupCrossPlaneAdmissionFailure, StartupCrossPlanePlan, StartupPhysicalManifest,
    StartupPhysicalNoMutation,
};

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
    /// Last issued validation epoch. Zero remains reserved for never-visited table members.
    last_validation_epoch: u64,
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

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedFailureAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(crate) fn with_fallback_cause(mut self, cause: PreparationFailureCode) -> Self {
        self.fallback_cause = cause;
        self
    }
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
    fn try_next_validation_epoch(&mut self) -> Result<NonZeroU64, RetainedClosureValidationError> {
        let Some(next) = self
            .last_validation_epoch
            .checked_add(1)
            .and_then(NonZeroU64::new)
        else {
            self.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedClosureValidationError::Poisoned);
        };
        self.last_validation_epoch = next.get();
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
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedClosureValidationError::Poisoned);
        }
        if self.mode != RetainedOutcomeMode::SuccessCapable {
            return Err(RetainedClosureValidationError::WrongOutcomeMode);
        }
        let epoch = self.try_next_validation_epoch()?;
        let (Some(canonical), Some(evidence)) =
            (self.canonical_table.as_ref(), self.evidence_table.as_ref())
        else {
            self.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedClosureValidationError::Poisoned);
        };
        let canonical = canonical.as_ref();
        let evidence = evidence.as_ref();
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
            && session.canonical_seen == canonical.members.len()
            && session.evidence_seen == evidence.members.len();
        if !exact {
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
        let epoch = self.try_next_validation_epoch()?;
        let Some(evidence) = self.evidence_table.as_ref() else {
            self.containment = AssemblyContainmentProvenance::NewFaultPending;
            return Err(RetainedClosureValidationError::Poisoned);
        };
        let evidence = evidence.as_ref();
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
        let exact = accepted && session.valid && session.seen == evidence.members.len();
        if !exact {
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

    /// Validates a success graph while lending poll and prebound observed-work authority to the
    /// validator inside the existing panic boundary.
    pub(crate) fn validate_success_controlled<CanonicalTag, EvidenceTag>(
        mut self,
        work_dimension: D,
        canonical_root: CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
        evidence_root: OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
        validate: impl for<'session, 'control> FnOnce(
            &mut RetainedClosureSession<'session, CanonicalBlock, EvidenceBlock>,
            CanonicalRetainedSlot<CanonicalBlock, CanonicalTag>,
            OutcomeEvidenceRetainedSlot<EvidenceBlock, EvidenceTag>,
            RetainedWorkControl<'control, O, P, PP, U, D>,
        ) -> bool,
    ) -> RetainedSuccessValidation<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let validation = if self.containment != AssemblyContainmentProvenance::Healthy {
            Err(RetainedClosureValidationError::Poisoned)
        } else if self.mode != RetainedOutcomeMode::SuccessCapable {
            Err(RetainedClosureValidationError::WrongOutcomeMode)
        } else {
            let epoch = match self.try_next_validation_epoch() {
                Ok(epoch) => epoch,
                Err(cause) => {
                    return RetainedSuccessValidation::FallbackOnly(RetainedFallbackOnly {
                        assembly: self,
                        cause: PreparationFailureCode::from_closure_validation(cause),
                    });
                }
            };
            let control = match self.operation.as_mut() {
                Some(operation) => Some(operation.split_work_control(work_dimension).0),
                None => {
                    self.containment = AssemblyContainmentProvenance::NewFaultPending;
                    None
                }
            };
            let parts = match (
                self.canonical_table.as_ref(),
                self.evidence_table.as_ref(),
                control,
            ) {
                (Some(canonical), Some(evidence), Some(control)) => {
                    Some((canonical.as_ref(), evidence.as_ref(), control, epoch))
                }
                _ => {
                    self.containment = AssemblyContainmentProvenance::NewFaultPending;
                    None
                }
            };
            match parts {
                Some((canonical, evidence, control, epoch)) => {
                    if canonical_root.identity.tag != TypeId::of::<CanonicalTag>()
                        || evidence_root.identity.tag != TypeId::of::<EvidenceTag>()
                        || canonical
                            .member(canonical_root.identity, self.set_id)
                            .is_none()
                        || evidence
                            .member(evidence_root.identity, self.set_id)
                            .is_none()
                    {
                        Err(RetainedClosureValidationError::InvalidRoot)
                    } else {
                        let mut session = RetainedClosureSession {
                            canonical,
                            evidence,
                            set_id: self.set_id,
                            epoch,
                            canonical_seen: 0,
                            evidence_seen: 0,
                            valid: true,
                            not_send_or_sync: PhantomData,
                        };
                        let accepted = match catch_unwind(AssertUnwindSafe(|| {
                            validate(&mut session, canonical_root, evidence_root, control)
                        })) {
                            Ok(accepted) => accepted,
                            Err(payload) => {
                                discard_untrusted_panic_payload(payload);
                                return RetainedSuccessValidation::FallbackOnly(
                                    RetainedFallbackOnly {
                                        assembly: self,
                                        cause: PreparationFailureCode::from_closure_validation(
                                            RetainedClosureValidationError::Panicked,
                                        ),
                                    },
                                );
                            }
                        };
                        let exact = accepted
                            && session.valid
                            && session.canonical_seen == canonical.members.len()
                            && session.evidence_seen == evidence.members.len();
                        if exact {
                            Ok(RetainedSuccessClosureProof {
                                set_id: self.set_id,
                                canonical_count: canonical.members.len(),
                                evidence_count: evidence.members.len(),
                                canonical_root: canonical_root.identity,
                                evidence_root: evidence_root.identity,
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

    pub fn canonical_new_blocks(&self) -> usize {
        self.canonical_table
            .as_ref()
            .map_or(0, |table| table.as_ref().new_block_count())
    }

    pub fn canonical_commitment_len(&self) -> usize {
        self.canonical_batch
            .as_ref()
            .map_or(0, AttributionReservationBatch::commitment_len)
    }

    pub fn outcome_evidence_len(&self) -> usize {
        self.evidence_table
            .as_ref()
            .map_or(0, |table| table.as_ref().members.len())
    }

    pub fn outcome_evidence_new_blocks(&self) -> usize {
        self.evidence_table
            .as_ref()
            .map_or(0, |table| table.as_ref().new_block_count())
    }

    pub fn outcome_evidence_commitment_len(&self) -> usize {
        self.evidence_batch
            .as_ref()
            .map_or(0, AttributionReservationBatch::commitment_len)
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

    #[cfg(test)]
    pub(crate) fn test_canonical_attributed_bytes(&self) -> u64 {
        self.canonical_batch
            .as_ref()
            .and_then(AttributionReservationBatch::attributed_bytes)
            .expect("test canonical attribution bytes remain exact")
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
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(RetainedPublicationError::Poisoned);
        }
        let canonical_plan = self
            .canonical_plan
            .ok_or(RetainedPublicationError::WrongOutcomeMode)?;
        let (control, ledger) = self
            .operation
            .as_mut()
            .ok_or(RetainedPublicationError::Poisoned)?
            .split_work_control(work_dimension);
        let result = try_retain_block(
            ledger,
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
            move |emplacer| build(emplacer, control),
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
        if self.containment != AssemblyContainmentProvenance::Healthy {
            return Err(CanonicalRetainBatchError::Preflight(
                RetainedPublicationError::Poisoned,
            ));
        }
        let canonical_plan = self
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
                    .coordinator
                    .retained_ledger(),
                account: self
                    .account
                    .as_ref()
                    .ok_or(CanonicalRetainBatchError::Preflight(
                        RetainedPublicationError::Poisoned,
                    ))?,
                table: self.canonical_table.as_ref().ok_or(
                    CanonicalRetainBatchError::Preflight(RetainedPublicationError::Poisoned),
                )?,
                batch: self.canonical_batch.as_ref().ok_or(
                    CanonicalRetainBatchError::Preflight(RetainedPublicationError::Poisoned),
                )?,
                handle_share_work: self.handle_share_work,
                table_plan: canonical_plan,
                set_id: self.set_id,
                slot_limit_error: RetainedPublicationError::CanonicalSlotLimit,
            },
        );
        let qualified = match preflight {
            Ok(qualified) => qualified,
            Err(error) => {
                if let Some(containment) = retained_publication_error_containment(&error) {
                    self.containment = containment;
                }
                return Err(CanonicalRetainBatchError::Preflight(error));
            }
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
            let rollback = match (
                self.account.as_ref(),
                self.canonical_table.as_mut(),
                self.canonical_batch.as_mut(),
            ) {
                (Some(account), Some(table), Some(batch)) => {
                    qualified.try_rollback_prefix(account, table, batch)
                }
                _ => Err(RetainedPublicationError::Poisoned),
            };
            if let Err(rollback_cause) = rollback {
                self.containment = retained_publication_error_containment(&rollback_cause)
                    .unwrap_or(AssemblyContainmentProvenance::NewFaultPending);
                return Err(CanonicalRetainBatchError::Abandoned {
                    failed_item: index,
                    cause: rollback_cause,
                });
            }
            if self.containment != AssemblyContainmentProvenance::Healthy
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
            self.account
                .as_ref()
                .ok_or_else(|| CanonicalRetainBatchError::Abandoned {
                    failed_item: completed_len.saturating_sub(1),
                    cause: RetainedPublicationError::Poisoned,
                })?,
            self.canonical_table
                .as_ref()
                .ok_or_else(|| CanonicalRetainBatchError::Abandoned {
                    failed_item: completed_len.saturating_sub(1),
                    cause: RetainedPublicationError::Poisoned,
                })?,
            self.canonical_batch
                .as_ref()
                .ok_or_else(|| CanonicalRetainBatchError::Abandoned {
                    failed_item: completed_len.saturating_sub(1),
                    cause: RetainedPublicationError::Poisoned,
                })?,
        ) {
            Ok(completed) => completed,
            Err(cause) => {
                self.containment = retained_publication_error_containment(&cause)
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
                            && table
                                .as_ref()
                                .member(proof.canonical_root, self.set_id)
                                .is_some()
                    })
                    && self.evidence_table.as_ref().is_some_and(|table| {
                        table.as_ref().members.len() == proof.evidence_count
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
            let attribution = TerminalFallbackAttribution::new(
                ledger,
                baseline_batches,
                PrimaryPublicationBatches::new(canonical_batch, Some(evidence_batch)),
            );
            return TerminalFallbackGuard::new(attribution, physical).try_prepare(cause);
        }

        let transition = TerminalPrimaryGuard::new(
            ledger,
            baseline_batches,
            PrimaryPublicationBatches::new(canonical_batch, Some(evidence_batch)),
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

    pub fn outcome_evidence_new_blocks(&self) -> usize {
        self.assembly.outcome_evidence_new_blocks()
    }

    pub fn outcome_evidence_commitment_len(&self) -> usize {
        self.assembly.outcome_evidence_commitment_len()
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
