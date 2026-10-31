use std::fmt;

use super::super::{
    ByteBlock, BytesWriter, OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef,
    OutcomeEvidenceRecordWriter, OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef,
    OutcomeEvidenceUtf8Ref, PreparedStoreFallback, PreparedStoreSuccessOutcome, ScalarSequence,
    SequenceWriter, StoreBuildError, StoreEvidenceValidationSession, StoreFailureAssembly,
    StoreFailureValidation, StoreFallbackOnly, StorePrepareError, StoreRuntimeError,
    StoreSuccessValidation, StoreValidationSession, StoredScalar, Utf8Block, Utf8Writer,
    ValidatedStoreFailure, ValidatedStoreSuccess,
};
use super::execution::RetainedStoreExecution;
use super::{
    CounterDelta, PlannedOutcomeEvidenceBytes, PlannedOutcomeEvidenceCommit,
    PlannedOutcomeEvidenceRecords, PlannedOutcomeEvidenceSequence, PlannedOutcomeEvidenceShare,
    PlannedOutcomeEvidenceUtf8, PlannedStoreInstruction, PrimaryTranscriptParts,
    StoreExecutionError, StoreTranscriptContinuation,
};
use crate::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit};

/// A transcript precondition failure that preserves the complete linear owner.
#[must_use = "recover the state or deliberately consume it into minimal fallback"]
pub struct StoreTranscriptStateError<State> {
    state: State,
    cause: StoreExecutionError,
}

impl<State> StoreTranscriptStateError<State> {
    fn new(state: State, cause: StoreExecutionError) -> Self {
        Self { state, cause }
    }

    /// Returns the transcript failure without consuming the preserved owner.
    pub const fn cause(&self) -> &StoreExecutionError {
        &self.cause
    }

    /// Recovers the complete state for retry or fallback publication.
    pub fn into_state(self) -> State {
        self.state
    }
}

impl<State> fmt::Debug for StoreTranscriptStateError<State> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreTranscriptStateError")
            .field("cause", &self.cause)
            .finish_non_exhaustive()
    }
}

impl<State> fmt::Display for StoreTranscriptStateError<State> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl<State> std::error::Error for StoreTranscriptStateError<State> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Linear result of transcript-bound success validation.
#[must_use = "every branch retains or terminates the complete publication owner"]
pub enum StoreTranscriptSuccessValidation<
    O,
    PP,
    U,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(
        ValidatedTranscriptStoreSuccess<
            O,
            PP,
            U,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
    ),
    Rejected(StoreFailureContinuationExecution<O, PP, U, EvidenceKind, EvidenceTag>),
    FallbackOnly(StoreFallbackOnly<O, PP, U>),
}

/// Validated success paired with its allocation-free downgrade authority.
#[must_use = "prepare success, downgrade it, or deliberately publish minimal fallback"]
pub struct ValidatedTranscriptStoreSuccess<
    O,
    PP,
    U,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: ValidatedStoreSuccess<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>,
    continuation: StoreTranscriptContinuation,
}

impl<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    ValidatedTranscriptStoreSuccess<
        O,
        PP,
        U,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Reports whether semantic rejection or explicit downgrade has a planned evidence tail.
    pub fn has_failure_continuation(&self) -> bool {
        self.continuation.has_failure_continuation()
    }

    /// Prepares the validated success without reopening a rejected control transition.
    pub fn try_prepare(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        PreparedStoreSuccessOutcome<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>,
        StorePrepareError,
    > {
        let Self {
            raw,
            continuation: _,
        } = self;
        raw.try_prepare(final_progress)
    }

    /// Converts a later domain rejection into the same precompiled failure-tail executor.
    pub fn downgrade_to_rich_failure(
        self,
        fallback_cause: u32,
    ) -> StoreFailureContinuationExecution<O, PP, U, EvidenceKind, EvidenceTag> {
        let Self { raw, continuation } = self;
        StoreFailureContinuationExecution::new(
            raw.downgrade_to_rich_failure(fallback_cause),
            continuation,
        )
    }

    /// Discards the candidate and prepares only the pre-admitted fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        let Self {
            raw,
            continuation: _,
        } = self;
        raw.prepare_minimal_fallback()
    }
}

/// Linear result of transcript-bound rich-failure validation.
#[must_use = "every branch retains or terminates the complete publication owner"]
pub enum StoreTranscriptFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>),
    Rejected(StoreFailureContinuationExecution<O, PP, U, EvidenceKind, EvidenceTag>),
    FallbackOnly(StoreFallbackOnly<O, PP, U>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureExecutionState {
    Active,
    Abandoned { failed_instruction: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EvidenceState {
    members: usize,
    new_blocks: usize,
    commitments: usize,
}

impl EvidenceState {
    fn try_apply(self, delta: CounterDelta) -> Option<Self> {
        let mut next = self;
        next.members = next.members.checked_add(delta.blocks)?;
        next.new_blocks = next.new_blocks.checked_add(delta.new_blocks)?;
        next.commitments = next.commitments.checked_add(delta.commitments)?;
        Some(next)
    }

    fn is_prefix_of(self, final_state: Self) -> bool {
        self.members <= final_state.members
            && self.new_blocks <= final_state.new_blocks
            && self.commitments <= final_state.commitments
    }
}

/// Failure-tail executor whose cursor and orchestration bookkeeping allocate no storage.
///
/// Retain instructions still allocate their admitted payload storage according to each ticket.
#[must_use = "complete the planned tail, validate it, or deliberately publish minimal fallback"]
pub struct StoreFailureContinuationExecution<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    continuation: StoreTranscriptContinuation,
    state: FailureExecutionState,
}

macro_rules! execute_tail_block {
    ($name:ident, $ticket:ident, $writer:ty, $output:ident, $retain:ident) => {
        pub fn $name<Tag>(
            &mut self,
            ticket: &$ticket<Tag>,
            build: impl FnOnce(&mut $writer) -> Result<(), StoreBuildError>,
        ) -> Result<$output<Tag>, StoreExecutionError>
        where
            Tag: Copy + Send + Sync + 'static,
        {
            let instruction = ticket.inner();
            let (plan, tag) = (instruction.plan, instruction.tag);
            self.run(
                instruction,
                CounterDelta::new_block(super::super::StoreDestination::OutcomeEvidence),
                |assembly| assembly.$retain(plan, tag, build),
            )
        }
    };
}

impl<O, PP, U, EvidenceKind, EvidenceTag>
    StoreFailureContinuationExecution<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(
        assembly: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
        continuation: StoreTranscriptContinuation,
    ) -> Self {
        let actual = evidence_state(&assembly);
        let expected = expected_evidence_state(&continuation);
        let final_state = evidence_state_at(continuation.final_endpoint);
        let endpoints_are_compatible = continuation.cursor.next_ordinal
            <= continuation.final_endpoint.next_ordinal
            && continuation.cursor.counters.canonical
                == continuation.final_endpoint.counters.canonical
            && expected.is_prefix_of(final_state);
        let state =
            if !assembly.is_fallback_only() && endpoints_are_compatible && actual == expected {
                FailureExecutionState::Active
            } else {
                FailureExecutionState::Abandoned {
                    failed_instruction: continuation.cursor.next_ordinal,
                }
            };
        Self {
            assembly,
            continuation,
            state,
        }
    }

    /// Returns the next planned instruction ordinal.
    pub fn cursor(&self) -> usize {
        self.continuation.cursor.next_ordinal
    }

    /// Returns the evidence root preserved across semantic rejection or explicit downgrade.
    pub const fn evidence_root(&self) -> OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag> {
        self.assembly.evidence_root()
    }

    /// Reports whether every planned failure-tail instruction has committed exactly once.
    pub fn is_complete(&self) -> bool {
        self.state == FailureExecutionState::Active
            && self.continuation.cursor == self.continuation.final_endpoint
            && evidence_state(&self.assembly) == evidence_state_at(self.continuation.final_endpoint)
    }

    fn ensure<Plan, Tag, Destination>(
        &mut self,
        ticket: &PlannedStoreInstruction<Plan, Tag, Destination>,
    ) -> Result<(), StoreExecutionError> {
        if let FailureExecutionState::Abandoned { failed_instruction } = self.state {
            return Err(StoreExecutionError::Abandoned { failed_instruction });
        }
        if ticket.identity != self.continuation.identity {
            return Err(StoreExecutionError::ForeignTicket);
        }
        if self.continuation.cursor.next_ordinal >= self.continuation.final_endpoint.next_ordinal {
            return Err(StoreExecutionError::SegmentComplete);
        }
        if ticket.ordinal != self.continuation.cursor.next_ordinal {
            return Err(StoreExecutionError::OutOfOrder {
                expected: self.continuation.cursor.next_ordinal,
                actual: ticket.ordinal,
            });
        }
        if evidence_state(&self.assembly) != expected_evidence_state(&self.continuation) {
            return self.abandon_invariant();
        }
        Ok(())
    }

    fn advance(&mut self, delta: CounterDelta) -> Result<(), StoreExecutionError> {
        let Some(next) = self.continuation.cursor.try_advance(delta) else {
            return self.abandon_invariant();
        };
        if next.next_ordinal > self.continuation.final_endpoint.next_ordinal
            || next.counters.canonical != self.continuation.final_endpoint.counters.canonical
            || !evidence_state_at(next)
                .is_prefix_of(evidence_state_at(self.continuation.final_endpoint))
        {
            return self.abandon_invariant();
        }
        self.continuation.cursor = next;
        Ok(())
    }

    fn abandon_invariant<T>(&mut self) -> Result<T, StoreExecutionError> {
        self.state = FailureExecutionState::Abandoned {
            failed_instruction: self.continuation.cursor.next_ordinal,
        };
        Err(StoreExecutionError::InvariantUnverified)
    }

    fn run<Plan, Tag, Destination, T>(
        &mut self,
        ticket: &PlannedStoreInstruction<Plan, Tag, Destination>,
        delta: CounterDelta,
        action: impl FnOnce(
            &mut StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
        ) -> Result<T, StoreRuntimeError>,
    ) -> Result<T, StoreExecutionError> {
        self.ensure(ticket)?;
        let before = evidence_state(&self.assembly);
        let result = action(&mut self.assembly);
        let after = evidence_state(&self.assembly);
        match result {
            Ok(value)
                if !self.assembly.is_fallback_only() && before.try_apply(delta) == Some(after) =>
            {
                self.advance(delta)?;
                Ok(value)
            }
            Ok(_) => self.abandon_invariant(),
            Err(cause) if self.assembly.is_fallback_only() => {
                let failed_instruction = self.continuation.cursor.next_ordinal;
                self.state = FailureExecutionState::Abandoned { failed_instruction };
                Err(StoreExecutionError::Contained {
                    failed_instruction,
                    cause,
                })
            }
            Err(cause) if after == before => Err(StoreExecutionError::Runtime(cause)),
            Err(_) => self.abandon_invariant(),
        }
    }

    /// Executes one planned scalar-sequence evidence block.
    pub fn try_execute_sequence<T, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceSequence<T, Tag>,
        build: impl FnOnce(&mut SequenceWriter<T>) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreExecutionError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let instruction = ticket.inner();
        let (plan, tag) = (instruction.plan, instruction.tag);
        self.run(
            instruction,
            CounterDelta::new_block(super::super::StoreDestination::OutcomeEvidence),
            |assembly| assembly.try_retain_sequence(plan, tag, build),
        )
    }

    execute_tail_block!(
        try_execute_bytes,
        PlannedOutcomeEvidenceBytes,
        BytesWriter,
        OutcomeEvidenceBytesRef,
        try_retain_bytes
    );
    execute_tail_block!(
        try_execute_utf8,
        PlannedOutcomeEvidenceUtf8,
        Utf8Writer,
        OutcomeEvidenceUtf8Ref,
        try_retain_utf8
    );
    execute_tail_block!(
        try_execute_records,
        PlannedOutcomeEvidenceRecords,
        OutcomeEvidenceRecordWriter,
        OutcomeEvidenceRecordRef,
        try_retain_records
    );

    /// Executes one planned same-account scalar-sequence evidence share.
    pub fn try_execute_sequence_share<SourceRootKind, SourceRootTag, T, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceShare<
            '_,
            SourceRootKind,
            SourceRootTag,
            ScalarSequence<T>,
            Tag,
        >,
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreExecutionError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_share(ticket)
    }

    /// Executes one planned same-account binary evidence share.
    pub fn try_execute_bytes_share<SourceRootKind, SourceRootTag, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceShare<'_, SourceRootKind, SourceRootTag, ByteBlock, Tag>,
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreExecutionError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_share(ticket)
    }

    /// Executes one planned same-account UTF-8 evidence share.
    pub fn try_execute_utf8_share<SourceRootKind, SourceRootTag, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceShare<'_, SourceRootKind, SourceRootTag, Utf8Block, Tag>,
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreExecutionError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_share(ticket)
    }

    fn execute_share<SourceRootKind, SourceRootTag, Kind, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceShare<'_, SourceRootKind, SourceRootTag, Kind, Tag>,
    ) -> Result<OutcomeEvidenceStoreRef<Kind, Tag>, StoreExecutionError>
    where
        Kind: super::super::ShareableStoreKind,
        Tag: Copy + Send + Sync + 'static,
    {
        let instruction = ticket.inner();
        let (source, source_ref) = (instruction.plan.source, instruction.plan.source_ref);
        self.run(
            instruction,
            CounterDelta::shared_block(super::super::StoreDestination::OutcomeEvidence),
            |assembly| assembly.try_share(source, source_ref),
        )
    }

    /// Executes one planned evidence commitment.
    pub fn try_execute_commit(
        &mut self,
        ticket: &PlannedOutcomeEvidenceCommit,
    ) -> Result<(), StoreExecutionError> {
        let instruction = ticket.inner();
        let (dimension, amount) = (instruction.plan.dimension, instruction.plan.amount);
        self.run(
            instruction,
            CounterDelta::commitment(super::super::StoreDestination::OutcomeEvidence),
            |assembly| assembly.try_commit(dimension, amount),
        )
    }

    /// Validates the exact rich-failure graph after the final transcript endpoint.
    #[allow(
        clippy::result_large_err,
        reason = "ordinary precondition rejection preserves the complete executor"
    )]
    pub fn try_validate_rich_failure<NextEvidenceKind, NextEvidenceTag>(
        mut self,
        evidence_root: OutcomeEvidenceStoreRef<NextEvidenceKind, NextEvidenceTag>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreEvidenceValidationSession<'session, 'storage>,
            OutcomeEvidenceStoreRef<NextEvidenceKind, NextEvidenceTag>,
        ) -> bool,
    ) -> Result<
        StoreTranscriptFailureValidation<O, PP, U, NextEvidenceKind, NextEvidenceTag>,
        StoreTranscriptStateError<Self>,
    >
    where
        NextEvidenceTag: Copy + Send + Sync + 'static,
    {
        if let Err(cause) = self.ensure_final_endpoint() {
            return Err(StoreTranscriptStateError::new(self, cause));
        }
        let Self {
            assembly,
            continuation,
            state: _,
        } = self;
        Ok(map_failure_validation(
            assembly.validate_rich_failure(evidence_root, validate),
            continuation,
        ))
    }

    fn ensure_final_endpoint(&mut self) -> Result<(), StoreExecutionError> {
        if let FailureExecutionState::Abandoned { failed_instruction } = self.state {
            return Err(StoreExecutionError::Abandoned { failed_instruction });
        }
        if self.continuation.cursor != self.continuation.final_endpoint {
            return Err(StoreExecutionError::OutOfOrder {
                expected: self.continuation.final_endpoint.next_ordinal,
                actual: self.continuation.cursor.next_ordinal,
            });
        }
        if evidence_state(&self.assembly) != evidence_state_at(self.continuation.final_endpoint) {
            return self.abandon_invariant();
        }
        Ok(())
    }

    /// Discards rich-failure detail and prepares only the pre-admitted fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        self.assembly.prepare_minimal_fallback()
    }
}

fn evidence_state<O, PP, U, EvidenceKind, EvidenceTag>(
    assembly: &StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
) -> EvidenceState
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    EvidenceState {
        members: assembly.outcome_evidence_len(),
        new_blocks: assembly.outcome_evidence_new_blocks(),
        commitments: assembly.outcome_evidence_commitment_len(),
    }
}

fn expected_evidence_state(continuation: &StoreTranscriptContinuation) -> EvidenceState {
    evidence_state_at(continuation.cursor)
}

fn evidence_state_at(endpoint: super::TranscriptEndpoint) -> EvidenceState {
    let counters = endpoint.counters.evidence;
    EvidenceState {
        members: counters.blocks,
        new_blocks: counters.new_blocks,
        commitments: counters.commitments,
    }
}

fn map_failure_validation<O, PP, U, EvidenceKind, EvidenceTag>(
    validation: StoreFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>,
    continuation: StoreTranscriptContinuation,
) -> StoreTranscriptFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match validation {
        StoreFailureValidation::Valid(validated) => {
            StoreTranscriptFailureValidation::Valid(validated)
        }
        StoreFailureValidation::Rejected(assembly) => StoreTranscriptFailureValidation::Rejected(
            StoreFailureContinuationExecution::new(assembly, continuation),
        ),
        StoreFailureValidation::FallbackOnly(fallback) => {
            StoreTranscriptFailureValidation::FallbackOnly(fallback)
        }
    }
}

impl<O, PP, U, L> RetainedStoreExecution<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Validates a completed success primary and preserves its planned failure continuation.
    #[allow(
        clippy::result_large_err,
        reason = "ordinary precondition rejection preserves the complete executor"
    )]
    #[allow(
        clippy::type_complexity,
        reason = "the result keeps both typed roots and every linear publication owner explicit"
    )]
    pub fn try_validate_success<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>(
        self,
        canonical_root: super::super::CanonicalStoreRef<CanonicalKind, CanonicalTag>,
        evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreValidationSession<'session, 'storage>,
            super::super::CanonicalStoreRef<CanonicalKind, CanonicalTag>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        StoreTranscriptSuccessValidation<
            O,
            PP,
            U,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
        StoreTranscriptStateError<Self>,
    >
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        if !self.success_capable() {
            return Err(StoreTranscriptStateError::new(
                self,
                StoreExecutionError::Runtime(StoreRuntimeError::WrongOutcomeMode),
            ));
        }
        let PrimaryTranscriptParts {
            continuation,
            assembly,
        } = self
            .try_into_primary_parts()
            .map_err(|(state, cause)| StoreTranscriptStateError::new(state, cause))?;
        Ok(
            match assembly.validate_success(canonical_root, evidence_root, validate) {
                StoreSuccessValidation::Valid(raw) => {
                    StoreTranscriptSuccessValidation::Valid(ValidatedTranscriptStoreSuccess {
                        raw,
                        continuation,
                    })
                }
                StoreSuccessValidation::Rejected(assembly) => {
                    StoreTranscriptSuccessValidation::Rejected(
                        StoreFailureContinuationExecution::new(assembly, continuation),
                    )
                }
                StoreSuccessValidation::FallbackOnly(fallback) => {
                    StoreTranscriptSuccessValidation::FallbackOnly(fallback)
                }
            },
        )
    }

    /// Validates a completed rich-failure primary through the shared failure-validation rail.
    #[allow(
        clippy::result_large_err,
        reason = "ordinary precondition rejection preserves the complete executor"
    )]
    pub fn try_validate_rich_failure<EvidenceKind, EvidenceTag>(
        self,
        evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreEvidenceValidationSession<'session, 'storage>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        StoreTranscriptFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>,
        StoreTranscriptStateError<Self>,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        if self.success_capable() {
            return Err(StoreTranscriptStateError::new(
                self,
                StoreExecutionError::Runtime(StoreRuntimeError::WrongOutcomeMode),
            ));
        }
        let PrimaryTranscriptParts {
            continuation,
            assembly,
        } = self
            .try_into_primary_parts()
            .map_err(|(state, cause)| StoreTranscriptStateError::new(state, cause))?;
        Ok(map_failure_validation(
            assembly.validate_rich_failure(evidence_root, validate),
            continuation,
        ))
    }
}
