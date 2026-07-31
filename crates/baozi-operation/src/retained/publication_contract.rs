//! Publication-plan vocabulary shared by retained assembly and terminal preparation.

use super::{
    AccountPoisonReason, CommitmentPushError, PhantomData, RetainedAllocationError,
    RetainedBlockTablePlan, RetainedSlotIdentity, RetainedWriterError, fmt,
    quarantined_account_reason,
};

/// Independent canonical and outcome-evidence plans for one retained publication.
///
/// A success plan is success-capable rather than success-forcing: control or ledger rejection may
/// atomically downgrade it to rich failure or minimal fallback. The outcome-evidence root must be
/// outcome-neutral so the same evidence graph remains valid for success and failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedPublicationPlan<P, D> {
    pub(super) canonical: Option<RetainedBlockTablePlan<P, D>>,
    pub(super) outcome_evidence: RetainedBlockTablePlan<P, D>,
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
pub(super) enum RetainedOutcomeMode {
    SuccessCapable,
    FailureOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AssemblyContainmentProvenance {
    Healthy,
    NewFaultPending,
    RetainedFailureAlreadyRecorded(AccountPoisonReason),
}

pub(super) enum RetainedTerminalRequest<CanonicalBlock, EvidenceBlock, PP, U> {
    Success {
        proof: RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock>,
        final_progress: super::FinalProgress<PP, U>,
    },
    RichFailure {
        proof: RetainedEvidenceClosureProof<EvidenceBlock>,
        requested_cause: super::PreparationFailureCode,
        final_progress: super::FinalProgress<PP, U>,
    },
    MinimalFallback {
        requested_cause: super::PreparationFailureCode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainedTerminalMode {
    Success {
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
    },
    RichFailure {
        evidence_root: RetainedSlotIdentity,
        cause: super::PreparationFailureCode,
    },
    MinimalFallback {
        cause: super::PreparationFailureCode,
    },
}

/// Module-private exact-closure witness consumed by success preparation.
#[must_use = "the closure proof must be consumed by terminal preparation"]
pub(super) struct RetainedSuccessClosureProof<CanonicalBlock, EvidenceBlock> {
    pub(super) set_id: super::RetainedSetId,
    pub(super) canonical_count: usize,
    pub(super) evidence_count: usize,
    pub(super) canonical_root: RetainedSlotIdentity,
    pub(super) evidence_root: RetainedSlotIdentity,
    pub(super) marker: PhantomData<fn() -> (CanonicalBlock, EvidenceBlock)>,
}

/// Module-private exact-closure witness consumed by evidence-only preparation.
#[must_use = "the closure proof must be consumed by terminal preparation"]
pub(super) struct RetainedEvidenceClosureProof<EvidenceBlock> {
    pub(super) set_id: super::RetainedSetId,
    pub(super) evidence_count: usize,
    pub(super) evidence_root: RetainedSlotIdentity,
    pub(super) marker: PhantomData<fn() -> EvidenceBlock>,
}

/// Failure to validate the complete reachable block closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(super) enum RetainedClosureValidationError {
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
    pub(super) operation: Operation,
    pub(super) cause: RetainedWriterError<D>,
    pub(super) recoverable: bool,
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

pub(super) fn retained_open_failure_is_recoverable<D>(cause: &RetainedWriterError<D>) -> bool {
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
    /// A source slot, rewrite plan, or source-to-destination mapping is stale or inconsistent.
    InvalidSourceSlot,
    /// A prior failure made safe rollback or publication unverifiable.
    Poisoned,
}

pub(super) fn map_commitment_push_error<D, Destination>(
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
                formatter.write_str("retained source slot or rewrite mapping is invalid")
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

pub(super) fn retained_publication_error_containment<D>(
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

pub(crate) fn retained_publication_error_requires_containment<D>(
    error: &RetainedPublicationError<D>,
) -> bool {
    retained_publication_error_containment(error).is_some()
}
