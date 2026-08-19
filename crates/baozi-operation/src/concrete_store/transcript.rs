use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    BytesBatchPlan, BytesPlan, CanonicalStoreRef, FreshStoreLineage, OutcomeEvidenceStoreRef,
    PublishedOutcomeEvidenceStore, RecordPlan, RetainedStoreOperation,
    RetainedStorePublicationAssembly, SequenceBatchPlan, SequencePlan, StoreBatchRuntimeError,
    StoreDestination, StorePlanError, StoreRuntimeError, Utf8BatchPlan, Utf8Plan,
};
use crate::{OperationOwner, ProgressPhase, ProgressUnit};

mod execution;
mod planning;
mod validation;

pub use validation::{
    StoreFailureContinuationExecution, StoreTranscriptFailureValidation,
    StoreTranscriptSuccessValidation, ValidatedTranscriptStoreSuccess,
};

static NEXT_TRANSCRIPT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TranscriptId(NonZeroU64);

fn try_issue_transcript_id() -> Option<TranscriptId> {
    NEXT_TRANSCRIPT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, checked_add_one)
        .ok()
        .and_then(NonZeroU64::new)
        .map(TranscriptId)
}

fn checked_add_one(value: u64) -> Option<u64> {
    value.checked_add(1)
}

enum CanonicalTranscriptDestination {}

enum OutcomeEvidenceTranscriptDestination {}

struct CanonicalStoreSharePlan<SlotKind, SlotTag> {
    source_ref: CanonicalStoreRef<SlotKind, SlotTag>,
}

struct OutcomeEvidenceStoreSharePlan<'source, SourceRootKind, SourceRootTag, SlotKind, SlotTag> {
    source: &'source PublishedOutcomeEvidenceStore<SourceRootKind, SourceRootTag>,
    source_ref: OutcomeEvidenceStoreRef<SlotKind, SlotTag>,
}

struct StoreCommitPlan {
    dimension: super::StoreDimensionKey,
    amount: u64,
}

struct PlannedStoreInstruction<Plan, Tag, Destination> {
    identity: TranscriptId,
    ordinal: usize,
    plan: Plan,
    tag: Tag,
    marker: PhantomData<fn(Destination) -> Destination>,
}

impl<Plan, Tag, Destination> PlannedStoreInstruction<Plan, Tag, Destination> {
    fn new(identity: TranscriptId, ordinal: usize, plan: Plan, tag: Tag) -> Self {
        Self {
            identity,
            ordinal,
            plan,
            tag,
            marker: PhantomData,
        }
    }
}

macro_rules! define_ticket {
    ($name:ident, [$($declaration:tt)+], [$($argument:tt)+], $inner:ty) => {
        #[doc = concat!("Opaque transcript ticket `", stringify!($name), "`.")]
        #[must_use = "execute the instruction in transcript order or leave the endpoint unreachable"]
        pub struct $name<$($declaration)+> {
            inner: $inner,
        }

        impl<$($declaration)+> $name<$($argument)+> {
            fn new(inner: $inner) -> Self {
                Self { inner }
            }

            fn inner(&self) -> &$inner {
                &self.inner
            }
        }

        impl<$($declaration)+> fmt::Debug for $name<$($argument)+> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("ordinal", &self.inner.ordinal)
                    .finish_non_exhaustive()
            }
        }
    };
    ($name:ident, $inner:ty) => {
        #[doc = concat!("Opaque transcript ticket `", stringify!($name), "`.")]
        #[must_use = "execute the instruction in transcript order or leave the endpoint unreachable"]
        pub struct $name {
            inner: $inner,
        }

        impl $name {
            fn new(inner: $inner) -> Self {
                Self { inner }
            }

            fn inner(&self) -> &$inner {
                &self.inner
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("ordinal", &self.inner.ordinal)
                    .finish_non_exhaustive()
            }
        }
    };
}

define_ticket!(
    PlannedCanonicalSequence,
    [T: super::StoredScalar, Tag],
    [T, Tag],
    PlannedStoreInstruction<SequencePlan<T>, Tag, CanonicalTranscriptDestination>
);

define_ticket!(
    PlannedCanonicalBytes,
    [Tag],
    [Tag],
    PlannedStoreInstruction<BytesPlan, Tag, CanonicalTranscriptDestination>
);

define_ticket!(
    PlannedCanonicalUtf8,
    [Tag],
    [Tag],
    PlannedStoreInstruction<Utf8Plan, Tag, CanonicalTranscriptDestination>
);

define_ticket!(
    PlannedCanonicalRecords,
    [Tag],
    [Tag],
    PlannedStoreInstruction<RecordPlan, Tag, CanonicalTranscriptDestination>
);

define_ticket!(
    PlannedOutcomeEvidenceSequence,
    [T: super::StoredScalar, Tag],
    [T, Tag],
    PlannedStoreInstruction<SequencePlan<T>, Tag, OutcomeEvidenceTranscriptDestination>
);

define_ticket!(
    PlannedOutcomeEvidenceBytes,
    [Tag],
    [Tag],
    PlannedStoreInstruction<BytesPlan, Tag, OutcomeEvidenceTranscriptDestination>
);

define_ticket!(
    PlannedOutcomeEvidenceUtf8,
    [Tag],
    [Tag],
    PlannedStoreInstruction<Utf8Plan, Tag, OutcomeEvidenceTranscriptDestination>
);

define_ticket!(
    PlannedOutcomeEvidenceRecords,
    [Tag],
    [Tag],
    PlannedStoreInstruction<RecordPlan, Tag, OutcomeEvidenceTranscriptDestination>
);

define_ticket!(
    PlannedCanonicalSequenceBatch,
    ['source, T: super::StoredScalar, Tag],
    ['source, T, Tag],
    PlannedStoreInstruction<
        SequenceBatchPlan<'source, T>,
        Tag,
        CanonicalTranscriptDestination,
    >
);

define_ticket!(
    PlannedCanonicalBytesBatch,
    ['source, Tag],
    ['source, Tag],
    PlannedStoreInstruction<BytesBatchPlan<'source>, Tag, CanonicalTranscriptDestination>
);

define_ticket!(
    PlannedCanonicalUtf8Batch,
    ['source, Tag],
    ['source, Tag],
    PlannedStoreInstruction<Utf8BatchPlan<'source>, Tag, CanonicalTranscriptDestination>
);

define_ticket!(
    PlannedCanonicalCommit,
    PlannedStoreInstruction<StoreCommitPlan, (), CanonicalTranscriptDestination>
);

define_ticket!(
    PlannedOutcomeEvidenceCommit,
    PlannedStoreInstruction<StoreCommitPlan, (), OutcomeEvidenceTranscriptDestination>
);

define_ticket!(
    PlannedCanonicalShare,
    [Kind, Tag],
    [Kind, Tag],
    PlannedStoreInstruction<CanonicalStoreSharePlan<Kind, Tag>, (), CanonicalTranscriptDestination>
);

define_ticket!(
    PlannedOutcomeEvidenceShare,
    ['source, SourceRootKind, SourceRootTag, Kind, Tag],
    ['source, SourceRootKind, SourceRootTag, Kind, Tag],
    PlannedStoreInstruction<
        OutcomeEvidenceStoreSharePlan<'source, SourceRootKind, SourceRootTag, Kind, Tag>,
        (),
        OutcomeEvidenceTranscriptDestination,
    >
);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DestinationCounters {
    blocks: usize,
    new_blocks: usize,
    commitments: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TranscriptCounters {
    canonical: DestinationCounters,
    evidence: DestinationCounters,
}

impl TranscriptCounters {
    fn for_destination(self, destination: StoreDestination) -> DestinationCounters {
        match destination {
            StoreDestination::Canonical => self.canonical,
            StoreDestination::OutcomeEvidence => self.evidence,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TranscriptEndpoint {
    next_ordinal: usize,
    counters: TranscriptCounters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanningSegment {
    Primary,
    FailureContinuation,
}

#[derive(Debug, Clone, Copy)]
struct CounterDelta {
    destination: StoreDestination,
    blocks: usize,
    new_blocks: usize,
    commitments: usize,
}

impl CounterDelta {
    const fn new_block(destination: StoreDestination) -> Self {
        Self::new_blocks(destination, 1)
    }

    const fn new_blocks(destination: StoreDestination, blocks: usize) -> Self {
        Self {
            destination,
            blocks,
            new_blocks: blocks,
            commitments: 0,
        }
    }

    const fn shared_block(destination: StoreDestination) -> Self {
        Self {
            destination,
            blocks: 1,
            new_blocks: 0,
            commitments: 0,
        }
    }

    const fn commitment(destination: StoreDestination) -> Self {
        Self {
            destination,
            blocks: 0,
            new_blocks: 0,
            commitments: 1,
        }
    }
}

impl TranscriptEndpoint {
    fn try_advance(self, delta: CounterDelta) -> Option<Self> {
        let mut next = self;
        next.next_ordinal = next.next_ordinal.checked_add(1)?;
        let destination = match delta.destination {
            StoreDestination::Canonical => &mut next.counters.canonical,
            StoreDestination::OutcomeEvidence => &mut next.counters.evidence,
        };
        destination.blocks = destination.blocks.checked_add(delta.blocks)?;
        destination.new_blocks = destination.new_blocks.checked_add(delta.new_blocks)?;
        destination.commitments = destination.commitments.checked_add(delta.commitments)?;
        (destination.new_blocks <= destination.blocks).then_some(next)
    }
}

/// Allocation-free failure while minting a transcript ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreTranscriptPlanError {
    Store(StorePlanError),
    Runtime(StoreRuntimeError),
    PrimaryAlreadySealed,
    FailureContinuationRequiresSuccessMode,
    CanonicalFailureContinuationForbidden,
    SuccessPrimaryRequiresCanonicalAndEvidence,
    AttributionCapacityExceeded { destination: StoreDestination },
    CommitmentCapacityExceeded { destination: StoreDestination },
    SizeOverflow,
}

impl From<StorePlanError> for StoreTranscriptPlanError {
    fn from(error: StorePlanError) -> Self {
        Self::Store(error)
    }
}

impl fmt::Display for StoreTranscriptPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(cause) => cause.fmt(formatter),
            Self::Runtime(cause) => cause.fmt(formatter),
            Self::PrimaryAlreadySealed => {
                formatter.write_str("retained transcript primary execution is already sealed")
            }
            Self::FailureContinuationRequiresSuccessMode => formatter
                .write_str("a failure continuation requires a success-capable retained transcript"),
            Self::CanonicalFailureContinuationForbidden => formatter
                .write_str("failure-continuation instructions may target only outcome evidence"),
            Self::SuccessPrimaryRequiresCanonicalAndEvidence => formatter.write_str(
                "a success primary requires both canonical and outcome-evidence members",
            ),
            Self::AttributionCapacityExceeded { destination } => write!(
                formatter,
                "{destination:?} transcript exceeds fixed attribution capacity"
            ),
            Self::CommitmentCapacityExceeded { destination } => write!(
                formatter,
                "{destination:?} transcript exceeds fixed commitment capacity"
            ),
            Self::SizeOverflow => formatter.write_str("retained transcript counters overflowed"),
        }
    }
}

impl std::error::Error for StoreTranscriptPlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(cause) => Some(cause),
            Self::Runtime(cause) => Some(cause),
            _ => None,
        }
    }
}

/// The process-local transcript identity space is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreTranscriptIdentityExhausted;

impl fmt::Display for StoreTranscriptIdentityExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("retained transcript identity space is exhausted")
    }
}

impl std::error::Error for StoreTranscriptIdentityExhausted {}

/// Cause reported before an operation enters a retained transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreTranscriptStartCause {
    /// The process-local transcript identity space is exhausted.
    IdentityExhausted(StoreTranscriptIdentityExhausted),
    /// The operation rejected a writer phase or dimension before transcript construction.
    Runtime(StoreRuntimeError),
}

impl fmt::Display for StoreTranscriptStartCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdentityExhausted(cause) => cause.fmt(formatter),
            Self::Runtime(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for StoreTranscriptStartCause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IdentityExhausted(cause) => Some(cause),
            Self::Runtime(cause) => Some(cause),
        }
    }
}

/// Start failure that reports its exact cause and preserves the untouched operation.
#[must_use = "recover the operation or deliberately publish its pre-admitted fallback"]
pub struct StoreTranscriptStartError<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: RetainedStoreOperation<O, PP, U, L>,
    cause: StoreTranscriptStartCause,
}

impl<O, PP, U, L> StoreTranscriptStartError<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(
        operation: RetainedStoreOperation<O, PP, U, L>,
        cause: StoreTranscriptStartCause,
    ) -> Self {
        Self { operation, cause }
    }

    /// Returns the exact start failure without consuming the preserved operation.
    pub const fn cause(&self) -> &StoreTranscriptStartCause {
        &self.cause
    }

    /// Recovers the untouched operation so the caller can provide corrected writer plans.
    pub fn into_operation(self) -> RetainedStoreOperation<O, PP, U, L> {
        self.operation
    }
}

impl<O, PP, U, L> fmt::Debug for StoreTranscriptStartError<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreTranscriptStartError")
            .field("cause", &self.cause)
            .finish_non_exhaustive()
    }
}

impl<O, PP, U, L> fmt::Display for StoreTranscriptStartError<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "retained transcript could not start: {}",
            self.cause
        )
    }
}

impl<O, PP, U, L> std::error::Error for StoreTranscriptStartError<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Failure while advancing the primary cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreExecutionError {
    ForeignTicket,
    OutOfOrder {
        expected: usize,
        actual: usize,
    },
    SegmentComplete,
    Runtime(StoreRuntimeError),
    Batch(StoreBatchRuntimeError),
    Contained {
        failed_instruction: usize,
        cause: StoreRuntimeError,
    },
    BatchContained {
        failed_instruction: usize,
        cause: StoreBatchRuntimeError,
    },
    Abandoned {
        failed_instruction: usize,
    },
    InvariantUnverified,
}

impl fmt::Display for StoreExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForeignTicket => {
                formatter.write_str("planned instruction belongs to another transcript")
            }
            Self::OutOfOrder { expected, actual } => write!(
                formatter,
                "planned instruction {actual} cannot execute at cursor {expected}"
            ),
            Self::SegmentComplete => formatter.write_str("transcript segment is complete"),
            Self::Runtime(cause) => cause.fmt(formatter),
            Self::Batch(cause) => cause.fmt(formatter),
            Self::Contained {
                failed_instruction,
                cause,
            } => write!(
                formatter,
                "instruction {failed_instruction} entered containment: {cause}"
            ),
            Self::BatchContained {
                failed_instruction,
                cause,
            } => write!(
                formatter,
                "batch instruction {failed_instruction} entered containment: {cause}"
            ),
            Self::Abandoned { failed_instruction } => {
                write!(
                    formatter,
                    "transcript was abandoned at instruction {failed_instruction}"
                )
            }
            Self::InvariantUnverified => {
                formatter.write_str("retained transcript execution invariant is unverified")
            }
        }
    }
}

impl std::error::Error for StoreExecutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(cause) | Self::Contained { cause, .. } => Some(cause),
            Self::Batch(cause) | Self::BatchContained { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

/// Allocation-free orchestration authority retained through validation and downgrade.
#[must_use = "route the continuation through validation, downgrade, or terminal containment"]
pub(super) struct StoreTranscriptContinuation {
    identity: TranscriptId,
    cursor: TranscriptEndpoint,
    primary_endpoint: TranscriptEndpoint,
    final_endpoint: TranscriptEndpoint,
}

impl StoreTranscriptContinuation {
    pub(super) fn has_failure_continuation(&self) -> bool {
        self.primary_endpoint != self.final_endpoint
    }
}

pub(super) struct PrimaryTranscriptParts<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) continuation: StoreTranscriptContinuation,
    pub(super) assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
}
