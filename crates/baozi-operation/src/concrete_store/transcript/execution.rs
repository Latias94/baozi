use super::super::{
    ByteBlock, BytesWriter, CanonicalBytesRange, CanonicalBytesRef, CanonicalInheritanceSource,
    CanonicalRecordRef, CanonicalRecordWriter, CanonicalSequenceRange, CanonicalSequenceRef,
    CanonicalStoreRef, CanonicalUtf8Range, CanonicalUtf8Ref, FreshStoreLineage,
    OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef, OutcomeEvidenceRecordWriter,
    OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef, OutcomeEvidenceUtf8Ref,
    PreparedStoreFallback, RetainedStorePublicationAssembly, ScalarSequence, SequenceWriter,
    StoreBatchRuntimeError, StoreBuildError, StoreDestination, StoreDimensionKey,
    StorePrepareError, StoreRuntimeError, StoredScalar, Utf8Writer,
};
use super::{
    CounterDelta, DestinationCounters, PlannedCanonicalBytes, PlannedCanonicalBytesBatch,
    PlannedCanonicalCommit, PlannedCanonicalRecords, PlannedCanonicalSequence,
    PlannedCanonicalSequenceBatch, PlannedCanonicalShare, PlannedCanonicalUtf8,
    PlannedCanonicalUtf8Batch, PlannedOutcomeEvidenceBytes, PlannedOutcomeEvidenceCommit,
    PlannedOutcomeEvidenceRecords, PlannedOutcomeEvidenceSequence, PlannedOutcomeEvidenceShare,
    PlannedOutcomeEvidenceUtf8, PlannedStoreInstruction, PrimaryTranscriptParts,
    StoreExecutionError, StoreTranscriptContinuation, TranscriptEndpoint, TranscriptId,
};
use crate::{OperationOwner, ProgressPhase, ProgressUnit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionState {
    Active,
    Abandoned { failed_instruction: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DestinationState {
    members: usize,
    new_blocks: usize,
    commitments: usize,
}

impl From<DestinationCounters> for DestinationState {
    fn from(counters: DestinationCounters) -> Self {
        Self {
            members: counters.blocks,
            new_blocks: counters.new_blocks,
            commitments: counters.commitments,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TableState {
    canonical: DestinationState,
    evidence: DestinationState,
}

impl TableState {
    fn try_apply(self, delta: CounterDelta) -> Option<Self> {
        let mut next = self;
        let destination = match delta.destination {
            StoreDestination::Canonical => &mut next.canonical,
            StoreDestination::OutcomeEvidence => &mut next.evidence,
        };
        destination.members = destination.members.checked_add(delta.blocks)?;
        destination.new_blocks = destination.new_blocks.checked_add(delta.new_blocks)?;
        destination.commitments = destination.commitments.checked_add(delta.commitments)?;
        Some(next)
    }
}

/// Primary executor whose cursor and orchestration bookkeeping allocate no storage.
///
/// Retain instructions still allocate their admitted payload storage according to each ticket.
#[must_use = "complete primary execution or consume it into terminal fallback"]
pub struct RetainedStoreExecution<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    identity: TranscriptId,
    cursor: TranscriptEndpoint,
    primary_endpoint: TranscriptEndpoint,
    final_endpoint: TranscriptEndpoint,
    canonical_source_binding_dimension: Option<StoreDimensionKey>,
    state: ExecutionState,
}

macro_rules! execute_sequence {
    ($name:ident, $ticket:ident, $output:ident, $retain:ident, $destination:expr) => {
        pub fn $name<T, Tag>(
            &mut self,
            ticket: &$ticket<T, Tag>,
            build: impl FnOnce(&mut SequenceWriter<T>) -> Result<(), StoreBuildError>,
        ) -> Result<$output<T, Tag>, StoreExecutionError>
        where
            T: StoredScalar,
            Tag: Copy + Send + Sync + 'static,
        {
            let instruction = ticket.inner();
            let (plan, tag) = (instruction.plan, instruction.tag);
            self.run(
                instruction,
                CounterDelta::new_block($destination),
                |assembly| assembly.$retain(plan, tag, build),
            )
        }
    };
}

macro_rules! execute_block {
    ($name:ident, $ticket:ident, $writer:ty, $output:ident, $retain:ident, $destination:expr) => {
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
                CounterDelta::new_block($destination),
                |assembly| assembly.$retain(plan, tag, build),
            )
        }
    };
}

macro_rules! execute_simple_batch {
    ($name:ident, $ticket:ident, $output:ident, $retain:ident) => {
        pub fn $name<Tag>(
            &mut self,
            ticket: &$ticket<'_, Tag>,
        ) -> Result<$output<Tag>, StoreExecutionError>
        where
            Tag: Copy + Send + Sync + 'static,
        {
            let instruction = ticket.inner();
            let (plan, tag) = (instruction.plan, instruction.tag);
            self.run_batch(
                instruction,
                CounterDelta::new_blocks(StoreDestination::Canonical, plan.len()),
                |assembly| assembly.$retain(plan, tag),
            )
        }
    };
}

impl<O, PP, U, L> RetainedStoreExecution<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn new(
        assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
        identity: TranscriptId,
        primary_endpoint: TranscriptEndpoint,
        final_endpoint: TranscriptEndpoint,
        canonical_source_binding_dimension: Option<StoreDimensionKey>,
    ) -> Self {
        Self {
            assembly,
            identity,
            cursor: TranscriptEndpoint::default(),
            primary_endpoint,
            final_endpoint,
            canonical_source_binding_dimension,
            state: ExecutionState::Active,
        }
    }

    pub fn primary_cursor(&self) -> usize {
        self.cursor.next_ordinal
    }

    pub fn primary_is_complete(&self) -> bool {
        self.state == ExecutionState::Active && self.cursor == self.primary_endpoint
    }

    pub(super) fn success_capable(&self) -> bool {
        self.primary_endpoint.counters.canonical.blocks != 0
    }

    fn ensure<Plan, Tag, Destination>(
        &self,
        ticket: &PlannedStoreInstruction<Plan, Tag, Destination>,
    ) -> Result<(), StoreExecutionError> {
        if let ExecutionState::Abandoned { failed_instruction } = self.state {
            return Err(StoreExecutionError::Abandoned { failed_instruction });
        }
        if ticket.identity != self.identity {
            return Err(StoreExecutionError::ForeignTicket);
        }
        if self.cursor.next_ordinal >= self.primary_endpoint.next_ordinal {
            return Err(StoreExecutionError::SegmentComplete);
        }
        if ticket.ordinal != self.cursor.next_ordinal {
            return Err(StoreExecutionError::OutOfOrder {
                expected: self.cursor.next_ordinal,
                actual: ticket.ordinal,
            });
        }
        Ok(())
    }

    fn advance(&mut self, delta: CounterDelta) -> Result<(), StoreExecutionError> {
        let Some(next) = self.cursor.try_advance(delta) else {
            return self.abandon_invariant();
        };
        self.cursor = next;
        Ok(())
    }

    fn table_state(&self) -> TableState {
        TableState {
            canonical: DestinationState {
                members: self.assembly.canonical_len(),
                new_blocks: self.assembly.canonical_new_blocks(),
                commitments: self.assembly.canonical_commitment_len(),
            },
            evidence: DestinationState {
                members: self.assembly.outcome_evidence_len(),
                new_blocks: self.assembly.outcome_evidence_new_blocks(),
                commitments: self.assembly.outcome_evidence_commitment_len(),
            },
        }
    }

    fn abandon_invariant<T>(&mut self) -> Result<T, StoreExecutionError> {
        self.state = ExecutionState::Abandoned {
            failed_instruction: self.cursor.next_ordinal,
        };
        Err(StoreExecutionError::InvariantUnverified)
    }

    fn run<Plan, Tag, Destination, T>(
        &mut self,
        ticket: &PlannedStoreInstruction<Plan, Tag, Destination>,
        delta: CounterDelta,
        action: impl FnOnce(
            &mut RetainedStorePublicationAssembly<O, PP, U, L>,
        ) -> Result<T, StoreRuntimeError>,
    ) -> Result<T, StoreExecutionError> {
        self.ensure(ticket)?;
        let before = self.table_state();
        let result = action(&mut self.assembly);
        self.finish(result, before, delta)
    }

    fn finish<T>(
        &mut self,
        result: Result<T, StoreRuntimeError>,
        before: TableState,
        delta: CounterDelta,
    ) -> Result<T, StoreExecutionError> {
        let after = self.table_state();
        match result {
            Ok(value)
                if !self.assembly.is_fallback_only() && before.try_apply(delta) == Some(after) =>
            {
                self.advance(delta)?;
                Ok(value)
            }
            Ok(_) => self.abandon_invariant(),
            Err(cause) if self.assembly.is_fallback_only() => {
                let failed_instruction = self.cursor.next_ordinal;
                self.state = ExecutionState::Abandoned { failed_instruction };
                Err(StoreExecutionError::Contained {
                    failed_instruction,
                    cause,
                })
            }
            Err(cause) if after == before => Err(StoreExecutionError::Runtime(cause)),
            Err(_) => self.abandon_invariant(),
        }
    }

    fn run_batch<Plan, Tag, Destination, T>(
        &mut self,
        ticket: &PlannedStoreInstruction<Plan, Tag, Destination>,
        delta: CounterDelta,
        action: impl FnOnce(
            &mut RetainedStorePublicationAssembly<O, PP, U, L>,
        ) -> Result<T, StoreBatchRuntimeError>,
    ) -> Result<T, StoreExecutionError> {
        self.ensure(ticket)?;
        let before = self.table_state();
        let result = action(&mut self.assembly);
        self.finish_batch(result, before, delta)
    }

    fn finish_batch<T>(
        &mut self,
        result: Result<T, StoreBatchRuntimeError>,
        before: TableState,
        delta: CounterDelta,
    ) -> Result<T, StoreExecutionError> {
        let after = self.table_state();
        match result {
            Ok(value)
                if !self.assembly.is_fallback_only() && before.try_apply(delta) == Some(after) =>
            {
                self.advance(delta)?;
                Ok(value)
            }
            Ok(_) => self.abandon_invariant(),
            Err(cause @ StoreBatchRuntimeError::Abandoned { .. }) => {
                let failed_instruction = self.cursor.next_ordinal;
                self.state = ExecutionState::Abandoned { failed_instruction };
                Err(StoreExecutionError::BatchContained {
                    failed_instruction,
                    cause,
                })
            }
            Err(cause) if self.assembly.is_fallback_only() => {
                let failed_instruction = self.cursor.next_ordinal;
                self.state = ExecutionState::Abandoned { failed_instruction };
                Err(StoreExecutionError::BatchContained {
                    failed_instruction,
                    cause,
                })
            }
            Err(cause) if after == before => Err(StoreExecutionError::Batch(cause)),
            Err(_) => self.abandon_invariant(),
        }
    }

    execute_sequence!(
        try_execute_canonical_sequence,
        PlannedCanonicalSequence,
        CanonicalSequenceRef,
        try_retain_canonical_sequence,
        StoreDestination::Canonical
    );
    execute_block!(
        try_execute_canonical_bytes,
        PlannedCanonicalBytes,
        BytesWriter,
        CanonicalBytesRef,
        try_retain_canonical_bytes,
        StoreDestination::Canonical
    );
    execute_block!(
        try_execute_canonical_utf8,
        PlannedCanonicalUtf8,
        Utf8Writer,
        CanonicalUtf8Ref,
        try_retain_canonical_utf8,
        StoreDestination::Canonical
    );
    execute_block!(
        try_execute_canonical_records,
        PlannedCanonicalRecords,
        CanonicalRecordWriter,
        CanonicalRecordRef,
        try_retain_canonical_records,
        StoreDestination::Canonical
    );
    execute_sequence!(
        try_execute_outcome_evidence_sequence,
        PlannedOutcomeEvidenceSequence,
        OutcomeEvidenceSequenceRef,
        try_retain_outcome_evidence_sequence,
        StoreDestination::OutcomeEvidence
    );
    execute_block!(
        try_execute_outcome_evidence_bytes,
        PlannedOutcomeEvidenceBytes,
        BytesWriter,
        OutcomeEvidenceBytesRef,
        try_retain_outcome_evidence_bytes,
        StoreDestination::OutcomeEvidence
    );
    execute_block!(
        try_execute_outcome_evidence_utf8,
        PlannedOutcomeEvidenceUtf8,
        Utf8Writer,
        OutcomeEvidenceUtf8Ref,
        try_retain_outcome_evidence_utf8,
        StoreDestination::OutcomeEvidence
    );
    execute_block!(
        try_execute_outcome_evidence_records,
        PlannedOutcomeEvidenceRecords,
        OutcomeEvidenceRecordWriter,
        OutcomeEvidenceRecordRef,
        try_retain_outcome_evidence_records,
        StoreDestination::OutcomeEvidence
    );

    pub fn try_execute_canonical_sequence_batch<T, Tag>(
        &mut self,
        ticket: &PlannedCanonicalSequenceBatch<'_, T, Tag>,
    ) -> Result<CanonicalSequenceRange<T, Tag>, StoreExecutionError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let instruction = ticket.inner();
        let (plan, tag) = (instruction.plan, instruction.tag);
        self.run_batch(
            instruction,
            CounterDelta::new_blocks(StoreDestination::Canonical, plan.len()),
            |assembly| assembly.try_retain_canonical_sequence_batch(plan, tag),
        )
    }

    execute_simple_batch!(
        try_execute_canonical_bytes_batch,
        PlannedCanonicalBytesBatch,
        CanonicalBytesRange,
        try_retain_canonical_bytes_batch
    );
    execute_simple_batch!(
        try_execute_canonical_utf8_batch,
        PlannedCanonicalUtf8Batch,
        CanonicalUtf8Range,
        try_retain_canonical_utf8_batch
    );

    pub fn try_execute_canonical_commit(
        &mut self,
        ticket: &PlannedCanonicalCommit,
    ) -> Result<(), StoreExecutionError> {
        let instruction = ticket.inner();
        let (dimension, amount) = (instruction.plan.dimension, instruction.plan.amount);
        self.run(
            instruction,
            CounterDelta::commitment(StoreDestination::Canonical),
            |assembly| assembly.try_commit_canonical(dimension, amount),
        )
    }

    pub fn try_execute_outcome_evidence_commit(
        &mut self,
        ticket: &PlannedOutcomeEvidenceCommit,
    ) -> Result<(), StoreExecutionError> {
        let instruction = ticket.inner();
        let (dimension, amount) = (instruction.plan.dimension, instruction.plan.amount);
        self.run(
            instruction,
            CounterDelta::commitment(StoreDestination::OutcomeEvidence),
            |assembly| assembly.try_commit_outcome_evidence(dimension, amount),
        )
    }

    fn execute_evidence_share<SRK, SRT, Kind, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceShare<'_, SRK, SRT, Kind, Tag>,
    ) -> Result<OutcomeEvidenceStoreRef<Kind, Tag>, StoreExecutionError>
    where
        Kind: super::super::ShareableStoreKind,
        Tag: Copy + Send + Sync + 'static,
    {
        let instruction = ticket.inner();
        let (source, source_ref) = (instruction.plan.source, instruction.plan.source_ref);
        self.run(
            instruction,
            CounterDelta::shared_block(StoreDestination::OutcomeEvidence),
            |assembly| assembly.try_share_outcome_evidence(source, source_ref),
        )
    }

    pub fn try_execute_outcome_evidence_sequence_share<SRK, SRT, T, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceShare<'_, SRK, SRT, ScalarSequence<T>, Tag>,
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreExecutionError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_evidence_share(ticket)
    }

    pub fn try_execute_outcome_evidence_bytes_share<SRK, SRT, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceShare<'_, SRK, SRT, ByteBlock, Tag>,
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreExecutionError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_evidence_share(ticket)
    }

    pub fn try_execute_outcome_evidence_utf8_share<SRK, SRT, Tag>(
        &mut self,
        ticket: &PlannedOutcomeEvidenceShare<'_, SRK, SRT, super::super::Utf8Block, Tag>,
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreExecutionError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_evidence_share(ticket)
    }

    #[allow(
        clippy::result_large_err,
        reason = "an ordinary pre-handoff rejection preserves the complete executor"
    )]
    pub(super) fn try_into_primary_parts(
        self,
    ) -> Result<PrimaryTranscriptParts<O, PP, U, L>, (Self, StoreExecutionError)> {
        if let ExecutionState::Abandoned { failed_instruction } = self.state {
            return Err((self, StoreExecutionError::Abandoned { failed_instruction }));
        }
        if self.cursor != self.primary_endpoint {
            let expected = self.primary_endpoint.next_ordinal;
            let actual = self.cursor.next_ordinal;
            return Err((self, StoreExecutionError::OutOfOrder { expected, actual }));
        }
        let expected = TableState {
            canonical: self.primary_endpoint.counters.canonical.into(),
            evidence: self.primary_endpoint.counters.evidence.into(),
        };
        if self.table_state() != expected {
            return Err((self, StoreExecutionError::InvariantUnverified));
        }
        Ok(PrimaryTranscriptParts {
            continuation: StoreTranscriptContinuation {
                identity: self.identity,
                cursor: self.cursor,
                primary_endpoint: self.primary_endpoint,
                final_endpoint: self.final_endpoint,
            },
            assembly: self.assembly,
        })
    }

    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        self.assembly.prepare_minimal_fallback()
    }
}

impl<'source, O, PP, U> RetainedStoreExecution<O, PP, U, CanonicalInheritanceSource<'source>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn execute_canonical_share<Kind, Tag>(
        &mut self,
        ticket: &PlannedCanonicalShare<Kind, Tag>,
    ) -> Result<CanonicalStoreRef<Kind, Tag>, StoreExecutionError>
    where
        Kind: super::super::ShareableStoreKind,
        Tag: Copy + Send + Sync + 'static,
    {
        let Some(source_binding_dimension) = self.canonical_source_binding_dimension else {
            return self.abandon_invariant();
        };
        let instruction = ticket.inner();
        let source_ref = instruction.plan.source_ref;
        self.run(
            instruction,
            CounterDelta::shared_block(StoreDestination::Canonical),
            |assembly| {
                let members_before = assembly.canonical_len();
                assembly.try_bind_canonical_source(source_binding_dimension)?;
                let shared = assembly.try_share_canonical(source_ref)?;
                if assembly.canonical_len() == members_before {
                    return Err(StoreRuntimeError::InvalidSourceSlot);
                }
                Ok(shared)
            },
        )
    }

    pub fn try_execute_canonical_sequence_share<T, Tag>(
        &mut self,
        ticket: &PlannedCanonicalShare<ScalarSequence<T>, Tag>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreExecutionError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_canonical_share(ticket)
    }

    pub fn try_execute_canonical_bytes_share<Tag>(
        &mut self,
        ticket: &PlannedCanonicalShare<ByteBlock, Tag>,
    ) -> Result<CanonicalBytesRef<Tag>, StoreExecutionError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_canonical_share(ticket)
    }

    pub fn try_execute_canonical_utf8_share<Tag>(
        &mut self,
        ticket: &PlannedCanonicalShare<super::super::Utf8Block, Tag>,
    ) -> Result<CanonicalUtf8Ref<Tag>, StoreExecutionError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.execute_canonical_share(ticket)
    }
}
