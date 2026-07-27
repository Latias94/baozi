use std::fmt;

use super::super::{
    ByteBlock, BytesBatchPlan, BytesPlan, CanonicalBytesRef, CanonicalSequenceRef,
    CanonicalUtf8Ref, OutcomeEvidenceBytesRef, OutcomeEvidenceSequenceRef, OutcomeEvidenceUtf8Ref,
    PreparedStoreFallback, PublishedCanonicalStore, PublishedOutcomeEvidenceStore, RecordPlan,
    RetainedStoreBlockTablePlan, RetainedStoreOperation, RetainedStorePublicationPlan,
    RetainedStoreWriterPlan, ScalarSequence, SequenceBatchPlan, SequencePlan, StoreDestination,
    StorePlanError, StorePrepareError, StoreRuntimeError, StoredScalar, Utf8BatchPlan, Utf8Plan,
};
use super::execution::RetainedStoreExecution;
use super::{
    CanonicalStoreSharePlan, CounterDelta, OutcomeEvidenceStoreSharePlan, PlannedCanonicalBytes,
    PlannedCanonicalBytesBatch, PlannedCanonicalCommit, PlannedCanonicalRecords,
    PlannedCanonicalSequence, PlannedCanonicalSequenceBatch, PlannedCanonicalShare,
    PlannedCanonicalUtf8, PlannedCanonicalUtf8Batch, PlannedOutcomeEvidenceBytes,
    PlannedOutcomeEvidenceCommit, PlannedOutcomeEvidenceRecords, PlannedOutcomeEvidenceSequence,
    PlannedOutcomeEvidenceShare, PlannedOutcomeEvidenceUtf8, PlannedStoreInstruction,
    PlanningSegment, StoreCommitPlan, StoreTranscriptPlanError, StoreTranscriptStartCause,
    StoreTranscriptStartError, TranscriptEndpoint, TranscriptId, try_issue_transcript_id,
};
use crate::{OperationOwner, ProgressPhase, ProgressUnit};

/// Allocation-free operation-owned structural transcript planner.
///
/// Tickets seal the physical plan, destination, schema tag, source reference, and instruction
/// order. Builder callbacks still supply semantic values at execution time; canonical validation,
/// rather than this structural transcript, remains the authority for those values.
///
/// Tickets are linear capabilities and cannot be copied:
///
/// ```compile_fail,E0277
/// use baozi_operation::PlannedCanonicalBytes;
///
/// struct Tag;
///
/// fn require_clone<T: Clone>() {}
/// require_clone::<PlannedCanonicalBytes<Tag>>();
/// ```
///
/// Destination-specific tickets cannot be interchanged:
///
/// ```compile_fail,E0308
/// use baozi_operation::{PlannedCanonicalBytes, PlannedOutcomeEvidenceBytes};
///
/// struct Tag;
///
/// fn accept_evidence(_: &PlannedOutcomeEvidenceBytes<Tag>) {}
///
/// fn reject_canonical(ticket: &PlannedCanonicalBytes<Tag>) {
///     accept_evidence(ticket);
/// }
/// ```
#[must_use = "open the transcript, recover it, or publish the pre-admitted fallback"]
pub struct RetainedStoreTranscript<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: RetainedStoreOperation<O, PP, U>,
    identity: TranscriptId,
    canonical_writer: Option<RetainedStoreWriterPlan>,
    evidence_writer: RetainedStoreWriterPlan,
    progress: TranscriptEndpoint,
    primary_endpoint: Option<TranscriptEndpoint>,
    segment: PlanningSegment,
}

impl<O, PP, U> RetainedStoreOperation<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn try_begin_success_transcript(
        self,
        canonical_writer: RetainedStoreWriterPlan,
        evidence_writer: RetainedStoreWriterPlan,
    ) -> Result<RetainedStoreTranscript<O, PP, U>, StoreTranscriptStartError<O, PP, U>> {
        begin_transcript(self, Some(canonical_writer), evidence_writer)
    }

    pub fn try_begin_rich_failure_transcript(
        self,
        evidence_writer: RetainedStoreWriterPlan,
    ) -> Result<RetainedStoreTranscript<O, PP, U>, StoreTranscriptStartError<O, PP, U>> {
        begin_transcript(self, None, evidence_writer)
    }
}

fn begin_transcript<O, PP, U>(
    operation: RetainedStoreOperation<O, PP, U>,
    canonical_writer: Option<RetainedStoreWriterPlan>,
    evidence_writer: RetainedStoreWriterPlan,
) -> Result<RetainedStoreTranscript<O, PP, U>, StoreTranscriptStartError<O, PP, U>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if let Some(writer) = canonical_writer
        && let Err(cause) = operation.validate_writer_plan(writer)
    {
        return Err(StoreTranscriptStartError::new(
            operation,
            StoreTranscriptStartCause::Runtime(cause),
        ));
    }
    if let Err(cause) = operation.validate_writer_plan(evidence_writer) {
        return Err(StoreTranscriptStartError::new(
            operation,
            StoreTranscriptStartCause::Runtime(cause),
        ));
    }
    let Some(identity) = try_issue_transcript_id() else {
        return Err(StoreTranscriptStartError::new(
            operation,
            StoreTranscriptStartCause::IdentityExhausted(super::StoreTranscriptIdentityExhausted),
        ));
    };
    Ok(RetainedStoreTranscript {
        operation,
        identity,
        canonical_writer,
        evidence_writer,
        progress: TranscriptEndpoint::default(),
        primary_endpoint: None,
        segment: PlanningSegment::Primary,
    })
}

macro_rules! plan_sequence {
    ($name:ident, $ticket:ident, $destination:expr) => {
        pub fn $name<T, Tag>(
            &mut self,
            elements: usize,
            tag: Tag,
        ) -> Result<$ticket<T, Tag>, StoreTranscriptPlanError>
        where
            T: StoredScalar,
            Tag: Copy + Send + Sync + 'static,
        {
            self.mint_one($destination, SequencePlan::new(elements)?, tag)
                .map($ticket::new)
        }
    };
}

macro_rules! plan_sized_block {
    ($name:ident, $ticket:ident, $plan:ident, $destination:expr) => {
        pub fn $name<Tag>(
            &mut self,
            size: usize,
            tag: Tag,
        ) -> Result<$ticket<Tag>, StoreTranscriptPlanError>
        where
            Tag: Copy + Send + Sync + 'static,
        {
            self.mint_one($destination, $plan::new(size)?, tag)
                .map($ticket::new)
        }
    };
}

macro_rules! plan_records {
    ($name:ident, $ticket:ident, $destination:expr) => {
        pub fn $name<Tag>(
            &mut self,
            rows: usize,
            fields: usize,
            tag: Tag,
        ) -> Result<$ticket<Tag>, StoreTranscriptPlanError>
        where
            Tag: Copy + Send + Sync + 'static,
        {
            self.mint_one($destination, RecordPlan::new(rows, fields)?, tag)
                .map($ticket::new)
        }
    };
}

macro_rules! plan_simple_batch {
    ($name:ident, $ticket:ident, $plan:ident) => {
        pub fn $name<'a, Tag>(
            &mut self,
            plan: $plan<'a>,
            tag: Tag,
        ) -> Result<$ticket<'a, Tag>, StoreTranscriptPlanError>
        where
            Tag: Copy + Send + Sync + 'static,
        {
            let blocks = plan.len();
            self.mint_batch(plan, tag, blocks).map($ticket::new)
        }
    };
}

impl<O, PP, U> RetainedStoreTranscript<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn instruction_count(&self) -> usize {
        self.progress.next_ordinal
    }

    pub fn begin_failure_continuation(&mut self) -> Result<(), StoreTranscriptPlanError> {
        if self.canonical_writer.is_none() {
            return Err(StoreTranscriptPlanError::FailureContinuationRequiresSuccessMode);
        }
        if self.segment == PlanningSegment::FailureContinuation {
            return Err(StoreTranscriptPlanError::PrimaryAlreadySealed);
        }
        self.validate_success_primary(self.progress)?;
        self.primary_endpoint = Some(self.progress);
        self.segment = PlanningSegment::FailureContinuation;
        Ok(())
    }

    fn validate_success_primary(
        &self,
        endpoint: TranscriptEndpoint,
    ) -> Result<(), StoreTranscriptPlanError> {
        let counters = endpoint.counters;
        if counters.canonical.blocks == 0 || counters.evidence.blocks == 0 {
            return Err(StoreTranscriptPlanError::SuccessPrimaryRequiresCanonicalAndEvidence);
        }
        Ok(())
    }

    fn writer(
        &self,
        destination: StoreDestination,
    ) -> Result<RetainedStoreWriterPlan, StoreTranscriptPlanError> {
        match destination {
            StoreDestination::Canonical => {
                self.canonical_writer
                    .ok_or(StoreTranscriptPlanError::Runtime(
                        StoreRuntimeError::WrongOutcomeMode,
                    ))
            }
            StoreDestination::OutcomeEvidence => Ok(self.evidence_writer),
        }
    }

    fn validate_endpoint(
        &self,
        endpoint: TranscriptEndpoint,
        destination: StoreDestination,
    ) -> Result<(), StoreTranscriptPlanError> {
        let counters = endpoint.counters.for_destination(destination);
        let (_, _, _, limits) = self.writer(destination)?.transcript_parts();
        let attributions = counters
            .new_blocks
            .checked_add(1)
            .ok_or(StoreTranscriptPlanError::SizeOverflow)?;
        if attributions > limits.attribution_slots() {
            return Err(StoreTranscriptPlanError::AttributionCapacityExceeded { destination });
        }
        if counters.commitments > limits.commitment_slots() {
            return Err(StoreTranscriptPlanError::CommitmentCapacityExceeded { destination });
        }
        u64::try_from(counters.blocks)
            .map(|_| ())
            .map_err(|_| StoreTranscriptPlanError::SizeOverflow)
    }

    fn mint<Plan, Tag, Destination>(
        &mut self,
        delta: CounterDelta,
        plan: Plan,
        tag: Tag,
    ) -> Result<PlannedStoreInstruction<Plan, Tag, Destination>, StoreTranscriptPlanError> {
        if self.segment == PlanningSegment::FailureContinuation
            && delta.destination == StoreDestination::Canonical
        {
            return Err(StoreTranscriptPlanError::CanonicalFailureContinuationForbidden);
        }
        let next = self
            .progress
            .try_advance(delta)
            .ok_or(StoreTranscriptPlanError::SizeOverflow)?;
        self.validate_endpoint(next, delta.destination)?;
        let ticket =
            PlannedStoreInstruction::new(self.identity, self.progress.next_ordinal, plan, tag);
        self.progress = next;
        Ok(ticket)
    }

    fn mint_one<Plan, Tag, Destination>(
        &mut self,
        destination: StoreDestination,
        plan: Plan,
        tag: Tag,
    ) -> Result<PlannedStoreInstruction<Plan, Tag, Destination>, StoreTranscriptPlanError> {
        self.mint(CounterDelta::new_block(destination), plan, tag)
    }

    plan_sequence!(
        plan_canonical_sequence,
        PlannedCanonicalSequence,
        StoreDestination::Canonical
    );
    plan_sized_block!(
        plan_canonical_bytes,
        PlannedCanonicalBytes,
        BytesPlan,
        StoreDestination::Canonical
    );
    plan_sized_block!(
        plan_canonical_utf8,
        PlannedCanonicalUtf8,
        Utf8Plan,
        StoreDestination::Canonical
    );
    plan_records!(
        plan_canonical_records,
        PlannedCanonicalRecords,
        StoreDestination::Canonical
    );
    plan_sequence!(
        plan_outcome_evidence_sequence,
        PlannedOutcomeEvidenceSequence,
        StoreDestination::OutcomeEvidence
    );
    plan_sized_block!(
        plan_outcome_evidence_bytes,
        PlannedOutcomeEvidenceBytes,
        BytesPlan,
        StoreDestination::OutcomeEvidence
    );
    plan_sized_block!(
        plan_outcome_evidence_utf8,
        PlannedOutcomeEvidenceUtf8,
        Utf8Plan,
        StoreDestination::OutcomeEvidence
    );
    plan_records!(
        plan_outcome_evidence_records,
        PlannedOutcomeEvidenceRecords,
        StoreDestination::OutcomeEvidence
    );

    fn mint_batch<Plan, Tag, Destination>(
        &mut self,
        plan: Plan,
        tag: Tag,
        blocks: usize,
    ) -> Result<PlannedStoreInstruction<Plan, Tag, Destination>, StoreTranscriptPlanError> {
        self.mint(
            CounterDelta::new_blocks(StoreDestination::Canonical, blocks),
            plan,
            tag,
        )
    }

    pub fn plan_canonical_sequence_batch<'a, T, Tag>(
        &mut self,
        plan: SequenceBatchPlan<'a, T>,
        tag: Tag,
    ) -> Result<PlannedCanonicalSequenceBatch<'a, T, Tag>, StoreTranscriptPlanError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let blocks = plan.len();
        self.mint_batch(plan, tag, blocks)
            .map(PlannedCanonicalSequenceBatch::new)
    }

    plan_simple_batch!(
        plan_canonical_bytes_batch,
        PlannedCanonicalBytesBatch,
        BytesBatchPlan
    );
    plan_simple_batch!(
        plan_canonical_utf8_batch,
        PlannedCanonicalUtf8Batch,
        Utf8BatchPlan
    );

    fn mint_commit<Destination>(
        &mut self,
        destination: StoreDestination,
        dimension: super::super::StoreDimensionKey,
        amount: u64,
    ) -> Result<PlannedStoreInstruction<StoreCommitPlan, (), Destination>, StoreTranscriptPlanError>
    {
        self.mint(
            CounterDelta::commitment(destination),
            StoreCommitPlan { dimension, amount },
            (),
        )
    }

    pub fn plan_canonical_commit(
        &mut self,
        dimension: super::super::StoreDimensionKey,
        amount: u64,
    ) -> Result<PlannedCanonicalCommit, StoreTranscriptPlanError> {
        self.operation
            .validate_commitment_dimension(dimension)
            .map_err(StoreTranscriptPlanError::Runtime)?;
        self.mint_commit(StoreDestination::Canonical, dimension, amount)
            .map(PlannedCanonicalCommit::new)
    }

    pub fn plan_outcome_evidence_commit(
        &mut self,
        dimension: super::super::StoreDimensionKey,
        amount: u64,
    ) -> Result<PlannedOutcomeEvidenceCommit, StoreTranscriptPlanError> {
        self.operation
            .validate_commitment_dimension(dimension)
            .map_err(StoreTranscriptPlanError::Runtime)?;
        self.mint_commit(StoreDestination::OutcomeEvidence, dimension, amount)
            .map(PlannedOutcomeEvidenceCommit::new)
    }

    fn mint_canonical_share<'a, SRK, SRT, Kind, Tag>(
        &mut self,
        source: &'a PublishedCanonicalStore<SRK, SRT>,
        source_ref: super::super::CanonicalStoreRef<Kind, Tag>,
    ) -> Result<PlannedCanonicalShare<'a, SRK, SRT, Kind, Tag>, StoreTranscriptPlanError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if !source.belongs_to_account(self.operation.account()) {
            return Err(StoreTranscriptPlanError::Runtime(
                StoreRuntimeError::ForeignAccount,
            ));
        }
        let member = source
            .share_source()
            .aggregate()
            .lookup_member(source_ref.raw);
        if member.is_none_or(|member| !member.schema_tag_matches()) {
            return Err(StoreTranscriptPlanError::Runtime(
                StoreRuntimeError::InvalidSourceSlot,
            ));
        }
        self.mint(
            CounterDelta::shared_block(StoreDestination::Canonical),
            CanonicalStoreSharePlan { source, source_ref },
            (),
        )
        .map(PlannedCanonicalShare::new)
    }

    fn mint_evidence_share<'a, SRK, SRT, Kind, Tag>(
        &mut self,
        source: &'a PublishedOutcomeEvidenceStore<SRK, SRT>,
        source_ref: super::super::OutcomeEvidenceStoreRef<Kind, Tag>,
    ) -> Result<PlannedOutcomeEvidenceShare<'a, SRK, SRT, Kind, Tag>, StoreTranscriptPlanError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if !source.belongs_to_account(self.operation.account()) {
            return Err(StoreTranscriptPlanError::Runtime(
                StoreRuntimeError::ForeignAccount,
            ));
        }
        let member = source
            .share_source()
            .aggregate()
            .lookup_member(source_ref.raw);
        if member.is_none_or(|member| !member.schema_tag_matches()) {
            return Err(StoreTranscriptPlanError::Runtime(
                StoreRuntimeError::InvalidSourceSlot,
            ));
        }
        self.mint(
            CounterDelta::shared_block(StoreDestination::OutcomeEvidence),
            OutcomeEvidenceStoreSharePlan { source, source_ref },
            (),
        )
        .map(PlannedOutcomeEvidenceShare::new)
    }

    pub fn plan_canonical_sequence_share<'a, SRK, SRT, T, Tag>(
        &mut self,
        source: &'a PublishedCanonicalStore<SRK, SRT>,
        source_ref: CanonicalSequenceRef<T, Tag>,
    ) -> Result<PlannedCanonicalShare<'a, SRK, SRT, ScalarSequence<T>, Tag>, StoreTranscriptPlanError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        if source.sequence(source_ref).is_none() {
            return Err(invalid_source_slot());
        }
        self.mint_canonical_share(source, source_ref)
    }

    pub fn plan_canonical_bytes_share<'a, SRK, SRT, Tag>(
        &mut self,
        source: &'a PublishedCanonicalStore<SRK, SRT>,
        source_ref: CanonicalBytesRef<Tag>,
    ) -> Result<PlannedCanonicalShare<'a, SRK, SRT, ByteBlock, Tag>, StoreTranscriptPlanError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if source.bytes(source_ref).is_none() {
            return Err(invalid_source_slot());
        }
        self.mint_canonical_share(source, source_ref)
    }

    pub fn plan_canonical_utf8_share<'a, SRK, SRT, Tag>(
        &mut self,
        source: &'a PublishedCanonicalStore<SRK, SRT>,
        source_ref: CanonicalUtf8Ref<Tag>,
    ) -> Result<
        PlannedCanonicalShare<'a, SRK, SRT, super::super::Utf8Block, Tag>,
        StoreTranscriptPlanError,
    >
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if source.utf8(source_ref).is_none() {
            return Err(invalid_source_slot());
        }
        self.mint_canonical_share(source, source_ref)
    }

    pub fn plan_outcome_evidence_sequence_share<'a, SRK, SRT, T, Tag>(
        &mut self,
        source: &'a PublishedOutcomeEvidenceStore<SRK, SRT>,
        source_ref: OutcomeEvidenceSequenceRef<T, Tag>,
    ) -> Result<
        PlannedOutcomeEvidenceShare<'a, SRK, SRT, ScalarSequence<T>, Tag>,
        StoreTranscriptPlanError,
    >
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        if source.sequence(source_ref).is_none() {
            return Err(invalid_source_slot());
        }
        self.mint_evidence_share(source, source_ref)
    }

    pub fn plan_outcome_evidence_bytes_share<'a, SRK, SRT, Tag>(
        &mut self,
        source: &'a PublishedOutcomeEvidenceStore<SRK, SRT>,
        source_ref: OutcomeEvidenceBytesRef<Tag>,
    ) -> Result<PlannedOutcomeEvidenceShare<'a, SRK, SRT, ByteBlock, Tag>, StoreTranscriptPlanError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if source.bytes(source_ref).is_none() {
            return Err(invalid_source_slot());
        }
        self.mint_evidence_share(source, source_ref)
    }

    pub fn plan_outcome_evidence_utf8_share<'a, SRK, SRT, Tag>(
        &mut self,
        source: &'a PublishedOutcomeEvidenceStore<SRK, SRT>,
        source_ref: OutcomeEvidenceUtf8Ref<Tag>,
    ) -> Result<
        PlannedOutcomeEvidenceShare<'a, SRK, SRT, super::super::Utf8Block, Tag>,
        StoreTranscriptPlanError,
    >
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if source.utf8(source_ref).is_none() {
            return Err(invalid_source_slot());
        }
        self.mint_evidence_share(source, source_ref)
    }

    fn publication_plan(&self) -> Result<RetainedStorePublicationPlan, StoreTranscriptPlanError> {
        let canonical = self.progress.counters.canonical;
        let evidence = self.progress.counters.evidence;
        if self.canonical_writer.is_some() {
            self.validate_success_primary(self.primary_endpoint.unwrap_or(self.progress))?;
        }
        if evidence.blocks == 0 {
            return Err(StoreTranscriptPlanError::Store(
                StorePlanError::InvalidBlockTable,
            ));
        }
        let evidence = RetainedStoreBlockTablePlan::new(
            self.evidence_writer,
            evidence.blocks,
            evidence.new_blocks,
        )?;
        match self.canonical_writer {
            Some(writer) => {
                let canonical = RetainedStoreBlockTablePlan::new(
                    writer,
                    canonical.blocks,
                    canonical.new_blocks,
                )?;
                Ok(RetainedStorePublicationPlan::success(canonical, evidence))
            }
            None => Ok(RetainedStorePublicationPlan::rich_failure(evidence)),
        }
    }

    pub fn try_open(
        self,
    ) -> Result<RetainedStoreExecution<O, PP, U>, StoreTranscriptOpenError<O, PP, U>> {
        let plan = match self.publication_plan() {
            Ok(plan) => plan,
            Err(cause) => {
                return Err(StoreTranscriptOpenError {
                    transcript: Some(self),
                    cause: plan_runtime_error(cause),
                });
            }
        };
        let RetainedStoreTranscript {
            operation,
            identity,
            canonical_writer,
            evidence_writer,
            progress,
            primary_endpoint,
            segment,
        } = self;
        let primary = primary_endpoint.unwrap_or(progress);
        match operation.try_publication_assembly(plan) {
            Ok(assembly) => Ok(RetainedStoreExecution::new(
                assembly, identity, primary, progress,
            )),
            Err(error) => {
                let cause = error.cause().clone();
                let transcript = error.try_into_operation().ok().map(|operation| Self {
                    operation,
                    identity,
                    canonical_writer,
                    evidence_writer,
                    progress,
                    primary_endpoint: (segment == PlanningSegment::FailureContinuation)
                        .then_some(primary),
                    segment,
                });
                Err(StoreTranscriptOpenError { transcript, cause })
            }
        }
    }

    pub fn prepare_minimal_fallback(
        self,
        cause: u32,
    ) -> Result<PreparedStoreFallback, StorePrepareError> {
        self.operation.prepare_minimal_fallback(cause)
    }
}

fn invalid_source_slot() -> StoreTranscriptPlanError {
    StoreTranscriptPlanError::Runtime(StoreRuntimeError::InvalidSourceSlot)
}

fn plan_runtime_error(error: StoreTranscriptPlanError) -> StoreRuntimeError {
    match error {
        StoreTranscriptPlanError::Store(cause) => StoreRuntimeError::Plan(cause),
        StoreTranscriptPlanError::Runtime(cause) => cause,
        StoreTranscriptPlanError::PrimaryAlreadySealed
        | StoreTranscriptPlanError::FailureContinuationRequiresSuccessMode
        | StoreTranscriptPlanError::CanonicalFailureContinuationForbidden
        | StoreTranscriptPlanError::SuccessPrimaryRequiresCanonicalAndEvidence
        | StoreTranscriptPlanError::AttributionCapacityExceeded { .. }
        | StoreTranscriptPlanError::CommitmentCapacityExceeded { .. }
        | StoreTranscriptPlanError::SizeOverflow => StoreRuntimeError::InvalidPublicationPlan,
    }
}

/// Failure to open transcript-derived block tables.
#[must_use = "recover the transcript when table opening proved rollback"]
pub struct StoreTranscriptOpenError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    transcript: Option<RetainedStoreTranscript<O, PP, U>>,
    cause: StoreRuntimeError,
}

impl<O, PP, U> StoreTranscriptOpenError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreRuntimeError {
        &self.cause
    }

    pub fn try_into_transcript(
        self,
    ) -> Result<RetainedStoreTranscript<O, PP, U>, StoreRuntimeError> {
        self.transcript.ok_or(self.cause)
    }
}

impl<O, PP, U> fmt::Debug for StoreTranscriptOpenError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreTranscriptOpenError")
            .field("cause", &self.cause)
            .field("recoverable", &self.transcript.is_some())
            .finish_non_exhaustive()
    }
}

impl<O, PP, U> fmt::Display for StoreTranscriptOpenError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "retained transcript could not open: {}",
            self.cause
        )
    }
}

impl<O, PP, U> std::error::Error for StoreTranscriptOpenError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
