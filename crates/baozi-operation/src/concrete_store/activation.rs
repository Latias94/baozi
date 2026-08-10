//! Scoped session activation for the canonical retained-store lane.
//!
//! The public crate surface keeps this module behind `__canonical_activation` during U4. One
//! call owns preflight, physical open, materialization, rejection cleanup, and containment. No
//! request token or dormant authority can escape the call.

use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::{
    ByteBlock, BytesPlan, BytesWriter, CanonicalBytesRef, CanonicalRecordRef,
    CanonicalRecordRowWriter, CanonicalRecordWriter, CanonicalSequenceRef, CanonicalStoreRef,
    CanonicalUtf8Ref, FreshStoreLineage, OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef,
    OutcomeEvidenceRecordRowWriter, OutcomeEvidenceRecordView, OutcomeEvidenceRecordWriter,
    OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef, OutcomeEvidenceUtf8Ref,
    PreparedStoreFailureOutcome, PreparedStoreFallback, RawStoreBorrowedWorkPermit,
    RawStoreFailureAssembly, RecordBlock, RecordPlan, RetainedAllocationPlan,
    RetainedBuildFailureCode, RetainedStoreBlockTablePlan, RetainedStoreControlCheckpoint,
    RetainedStoreFailureTerminalWorkPlan, RetainedStoreFailureTerminalWorkRequirement,
    RetainedStoreOperation, RetainedStorePublicationAssembly, RetainedStorePublicationPlan,
    RetainedStoreReservation, RetainedStoreWorkControl, RetainedStoreWriterPlan, ScalarSequence,
    SequencePlan, SequenceWriter, StoreActivationBrand, StoreBuildCallbackFailure, StoreBuildError,
    StoreControlledError, StoreControlledSuccessValidation, StoreDimensionKey,
    StoreEvidenceValidationSession, StoreFailureAssembly, StoreFailureValidation,
    StoreFallbackOnly, StoreKeyDomain, StorePhaseKey, StorePlanError, StorePrepareError,
    StoreRuntimeError, StoreValidationSession, StoreWorkError, StoredBlock, StoredScalar,
    Utf8Block, Utf8Plan, Utf8Writer, ValidatedStoreFailure, ValidatedStoreSuccess,
    classify_prepared_fallback, map_publication_error, map_writer_error,
};
use crate::control::{ProgressPhase, ProgressUnit, discard_untrusted_panic_payload};
use crate::coordinator::FinalProgress;
use crate::ledger::STARTUP_ACCOUNT_SHARE_STEP_COUNT;
use crate::owner::OperationOwner;
use crate::retained::{
    FinalizationContainmentError, PreparationFailureCode, RetainedFailureWorkConsumeError,
    RetainedFailureWorkControl, RetainedFailureWorkFinishError, RetainedFailureWorkPermit,
    RetainedFailureWorkStartError, RetainedHandleSharePlan, RetainedPublicationOpenRecovery,
    RetainedPublicationPrepareError, RetainedWorkConsumeError, RetainedWorkControl,
    RetainedWorkFinishError, RetainedWorkPermit, RetainedWorkStartError,
};

mod canonical;
mod canonical_validation;
mod failure_evidence;
mod fallback;
mod preflight;
mod work_batch;

pub use canonical_validation::{
    CanonicalValidationMember, CanonicalValidationRecordRow, CanonicalValidationSession,
    OutcomeEvidenceValidationMember, OutcomeEvidenceValidationRecordRow,
};

const ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE: u32 = 0x4143_5047;
const CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE: u32 = 0x4356_4346;
const FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE: u32 = 0x4657_4545;
const FAILURE_EVIDENCE_BUILD_CHUNK_UNITS: usize = 256;

/// Operation-derived protected work plan for the canonical activation failure path.
///
/// The constructor accepts policy ceilings, not producer-attested work. Actual construction and
/// validation work are derived from sealed store plans. Each derived unit reserves the protected
/// charge pair used by the retained terminal rail, so producers never provide either the protected
/// dimension key or a self-attested member work amount.
#[derive(Debug, PartialEq, Eq)]
pub struct CanonicalFailureTerminalWorkPlan {
    terminal_work: RetainedStoreFailureTerminalWorkPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureEvidenceWorkEnvelope {
    max_evidence_members: usize,
    construction_work_limit: u64,
    validation_work_limit: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureEvidenceWorkBudget {
    remaining_members: usize,
    remaining_construction_work: u64,
    remaining_validation_work: u64,
    planned_validation_work: u64,
}

impl FailureEvidenceWorkBudget {
    const fn new(envelope: FailureEvidenceWorkEnvelope) -> Self {
        Self {
            remaining_members: envelope.max_evidence_members,
            remaining_construction_work: envelope.construction_work_limit,
            remaining_validation_work: envelope.validation_work_limit,
            planned_validation_work: 0,
        }
    }

    fn reserve_construction_member(&mut self, amount: u64) -> Result<(), StoreRuntimeError> {
        if amount == 0
            || self.remaining_members == 0
            || amount > self.remaining_construction_work
            || amount > self.remaining_validation_work
        {
            return Err(failure_work_envelope_exhausted());
        }
        let Some(planned) = self.planned_validation_work.checked_add(amount) else {
            return Err(failure_work_envelope_exhausted());
        };
        self.remaining_members -= 1;
        self.remaining_construction_work -= amount;
        self.remaining_validation_work -= amount;
        self.planned_validation_work = planned;
        Ok(())
    }

    fn with_existing_member_validation(mut self, amount: u64) -> Result<Self, StoreRuntimeError> {
        if amount == 0 || self.remaining_members == 0 || amount > self.remaining_validation_work {
            return Err(failure_work_envelope_exhausted());
        }
        let Some(planned) = self.planned_validation_work.checked_add(amount) else {
            return Err(failure_work_envelope_exhausted());
        };
        self.remaining_members -= 1;
        self.remaining_validation_work -= amount;
        self.planned_validation_work = planned;
        Ok(self)
    }

    fn planned_validation_work(&self) -> Result<NonZeroU64, StoreRuntimeError> {
        NonZeroU64::new(self.planned_validation_work).ok_or_else(failure_work_envelope_exhausted)
    }
}

type RawFailureEvidenceWorkControl<'a, O, PP, U> =
    RetainedFailureWorkControl<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>;
type RawFailureEvidenceWorkPermit<'a, O, PP, U> =
    RetainedFailureWorkPermit<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>;
type RawOutcomeEvidenceWorkControl<'a, O, PP, U> =
    RetainedWorkControl<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>;
type RawOutcomeEvidenceWorkPermit<'a, O, PP, U> =
    RetainedWorkPermit<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>;

/// Internal linear completion permit shared by plan-bound builders and validators.
#[must_use = "completed work must be recorded before resolving the permit"]
struct CanonicalWorkPermit<'borrow, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: Option<RawStoreBorrowedWorkPermit<'borrow, 'operation, O, PP, U>>,
    sticky: &'borrow mut Option<StoreWorkError>,
}

/// Plan-derived writer for one canonical block.
///
/// The raw writer and caller-selected work amounts never cross this boundary. Every mutating method
/// advances a local completion prefix only after the corresponding write succeeds. The prefix is
/// flushed and control is polled at bounded checkpoints, explicit polls, semantic boundaries, and
/// final completion.
pub struct CanonicalBuildControl<'writer, 'permit, 'operation, O, PP, U, W>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    writer: &'writer mut W,
    permit: Option<CanonicalWorkPermit<'permit, 'operation, O, PP, U>>,
    work_batch: work_batch::CompletionPrefixBatch,
    sticky: Option<StoreControlledError>,
}

/// Permit-bound row writer for one canonical fixed-width record.
///
/// A row is one bounded semantic batch: control is polled before the row and its completed field
/// prefix is flushed when the row succeeds, rejects, unwinds, or is abandoned.
pub struct CanonicalRecordBuildRow<'row, 'storage, 'permit, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    writer: &'row mut CanonicalRecordRowWriter<'storage>,
    permit: &'row mut CanonicalWorkPermit<'permit, 'operation, O, PP, U>,
    work_batch: &'row mut work_batch::CompletionPrefixBatch,
    sticky: &'row mut Option<StoreControlledError>,
}

fn try_begin_canonical_work<'borrow, 'operation, O, PP, U>(
    raw: &'borrow mut RetainedStoreWorkControl<'operation, O, PP, U>,
    amount: NonZeroU64,
) -> Result<CanonicalWorkPermit<'borrow, 'operation, O, PP, U>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if let Some(cause) = raw.sticky.clone() {
        return Err(cause);
    }
    let super::RetainedStoreWorkControl { raw, sticky } = raw;
    match raw.try_begin_borrowed(amount) {
        Ok(raw) => Ok(CanonicalWorkPermit {
            raw: Some(raw),
            sticky,
        }),
        Err(RetainedWorkStartError::Control(cause)) => {
            let cause = StoreWorkError::control(cause);
            sticky.get_or_insert_with(|| cause.clone());
            Err(cause)
        }
        Err(RetainedWorkStartError::Ledger(cause)) => {
            let cause = StoreWorkError::ledger(cause);
            sticky.get_or_insert_with(|| cause.clone());
            Err(cause)
        }
        Err(RetainedWorkStartError::Outstanding) => {
            panic!("canonical work control cannot start a nested or abandoned permit")
        }
    }
}

fn preserve_canonical_controlled<T>(
    sticky: &mut Option<StoreControlledError>,
    result: Result<T, StoreControlledError>,
) -> Result<T, StoreControlledError> {
    if let Err(cause) = &result {
        sticky.get_or_insert_with(|| cause.clone());
    }
    result
}

fn canonical_completion_contract_failure<O, PP, U>(
    permit: &mut CanonicalWorkPermit<'_, '_, O, PP, U>,
    sticky: &mut Option<StoreControlledError>,
) -> StoreControlledError
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    permit.record_product_graph_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
    let cause = StoreControlledError::from(StoreBuildError::incomplete_write());
    sticky.get_or_insert_with(|| cause.clone());
    cause
}

fn flush_canonical_completed<O, PP, U>(
    permit: &mut CanonicalWorkPermit<'_, '_, O, PP, U>,
    work_batch: &mut work_batch::CompletionPrefixBatch,
    sticky: &mut Option<StoreControlledError>,
) -> Result<(), StoreControlledError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let pending = work_batch.unflushed_completed();
    if pending == 0 {
        return Ok(());
    }
    if pending > permit.remaining_units() {
        let _ = work_batch.take_completed();
        let cause = canonical_completion_contract_failure(permit, sticky);
        permit.resolve_partial();
        return Err(cause);
    }
    let completed = work_batch
        .take_completed()
        .expect("a nonzero canonical completion prefix remains flushable");
    let completion = permit
        .complete_units(completed)
        .map_err(StoreControlledError::from);
    preserve_canonical_controlled(sticky, completion)
}

fn record_canonical_completed<O, PP, U>(
    permit: &mut CanonicalWorkPermit<'_, '_, O, PP, U>,
    work_batch: &mut work_batch::CompletionPrefixBatch,
    sticky: &mut Option<StoreControlledError>,
    completed: NonZeroU64,
) -> Result<(), StoreControlledError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if work_batch.note_completed(completed).is_ok() {
        return Ok(());
    }

    // The action already completed. Preserve it even if an internal batching invariant drifted.
    let cause = canonical_completion_contract_failure(permit, sticky);
    if flush_canonical_completed(permit, work_batch, sticky).is_ok()
        && completed.get() <= permit.remaining_units()
    {
        let completion = permit
            .complete_units(completed)
            .map_err(StoreControlledError::from);
        let _ = preserve_canonical_controlled(sticky, completion);
    }
    permit.resolve_partial();
    Err(cause)
}

impl<'writer, 'permit, 'operation, O, PP, U, W>
    CanonicalBuildControl<'writer, 'permit, 'operation, O, PP, U, W>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn from_admitted(
        writer: &'writer mut W,
        permit: CanonicalWorkPermit<'permit, 'operation, O, PP, U>,
    ) -> Self {
        Self {
            writer,
            permit: Some(permit),
            work_batch: work_batch::CompletionPrefixBatch::new(),
            sticky: None,
        }
    }

    fn resolve_partial(&mut self) {
        if let Some(mut permit) = self.permit.take() {
            let _ = flush_canonical_completed(&mut permit, &mut self.work_batch, &mut self.sticky);
            permit.resolve_partial();
        }
    }

    fn poll_now(&mut self) -> Result<(), StoreControlledError> {
        let permit = self
            .permit
            .as_mut()
            .expect("a fault-free canonical builder retains its work permit");
        flush_canonical_completed(permit, &mut self.work_batch, &mut self.sticky)?;
        let poll = permit.poll().map_err(StoreControlledError::from);
        preserve_canonical_controlled(&mut self.sticky, poll)?;
        self.work_batch.note_poll();
        Ok(())
    }

    fn reject_incomplete(&mut self) -> StoreControlledError {
        let permit = self
            .permit
            .as_mut()
            .expect("a fault-free canonical builder retains its work permit");
        let cause = canonical_completion_contract_failure(permit, &mut self.sticky);
        self.resolve_partial();
        cause
    }

    fn prepare_action(&mut self, amount: NonZeroU64) -> Result<(), StoreControlledError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        if self.work_batch.poll_due() || amount.get() > self.work_batch.units_until_checkpoint() {
            self.poll_now()?;
        }
        if amount.get() > self.work_batch.units_until_checkpoint() {
            return Err(self.reject_incomplete());
        }
        let permit_remaining = self
            .permit
            .as_ref()
            .expect("a fault-free canonical builder retains its work permit")
            .remaining_units();
        let Some(remaining) = self.work_batch.effective_remaining(permit_remaining) else {
            return Err(self.reject_incomplete());
        };
        if amount.get() > remaining {
            return Err(self.reject_incomplete());
        }
        Ok(())
    }

    fn perform<T>(
        &mut self,
        amount: u64,
        action: impl FnOnce(&mut W) -> Result<T, StoreBuildError>,
    ) -> Result<T, StoreControlledError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        if amount == 0 {
            let result = action(self.writer).map_err(StoreControlledError::from);
            return preserve_canonical_controlled(&mut self.sticky, result);
        }
        let amount = NonZeroU64::new(amount).expect("zero work is handled before completion");
        self.prepare_action(amount)?;
        let value = match action(self.writer) {
            Ok(value) => value,
            Err(cause) => {
                let cause = StoreControlledError::from(cause);
                self.sticky.get_or_insert_with(|| cause.clone());
                self.resolve_partial();
                return Err(cause);
            }
        };
        let permit = self
            .permit
            .as_mut()
            .expect("a fault-free canonical builder retains its work permit");
        record_canonical_completed(permit, &mut self.work_batch, &mut self.sticky, amount)?;
        Ok(value)
    }

    fn finish(
        mut self,
        callback: Result<(), StoreControlledError>,
    ) -> Result<(), StoreControlledError> {
        if let Some(cause) = self.sticky.take() {
            self.resolve_partial();
            return Err(cause);
        }
        if let Err(cause) = callback {
            self.sticky.get_or_insert_with(|| cause.clone());
            self.resolve_partial();
            return Err(cause);
        }
        self.poll_now()?;
        let remaining = self
            .permit
            .as_ref()
            .expect("a successful canonical builder retains its work permit")
            .remaining_units();
        if remaining != 0 {
            let cause = {
                let permit = self
                    .permit
                    .as_mut()
                    .expect("an underwritten canonical builder retains its work permit");
                canonical_completion_contract_failure(permit, &mut self.sticky)
            };
            self.resolve_partial();
            return Err(cause);
        }
        let exact = self
            .permit
            .take()
            .expect("a successful canonical builder completes its permit")
            .finish_exact();
        if exact {
            Ok(())
        } else {
            Err(StoreBuildError::incomplete_write().into())
        }
    }

    /// Polls cancellation and deadline state without exposing the work dimension.
    pub fn poll(&mut self) -> Result<(), StoreControlledError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        self.poll_now()
    }
}

impl<O, PP, U, W> Drop for CanonicalBuildControl<'_, '_, '_, O, PP, U, W>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.resolve_partial())) {
            discard_untrusted_panic_payload(payload);
        }
    }
}

fn finish_activation_writer_callback<O, PP, U, T>(
    control: RetainedStoreWorkControl<'_, O, PP, U>,
    callback: Result<T, StoreControlledError>,
    callback_failure: &mut Option<StoreBuildCallbackFailure>,
) -> Result<T, RetainedBuildFailureCode>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match control.finish_build(callback) {
        Ok(value) => Ok(value),
        Err(cause) => {
            *callback_failure = Some(cause);
            Err(RetainedBuildFailureCode::new(super::BUILD_CODE_DOMAIN))
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the private activation helper keeps one admitted permit coupled to one writer factory"
)]
fn try_retain_activation_canonical_writer<O, PP, U, L, W, Kind, Tag>(
    assembly: &mut RetainedStorePublicationAssembly<O, PP, U, L>,
    allocation: RetainedAllocationPlan,
    work_dimension: StoreDimensionKey,
    work: NonZeroU64,
    tag: Tag,
    make_writer: impl FnOnce() -> Result<W, StoreBuildError>,
    build: impl for<'writer, 'permit, 'operation> FnOnce(
        &mut CanonicalBuildControl<'writer, 'permit, 'operation, O, PP, U, W>,
    ) -> Result<(), StoreControlledError>,
    finish_writer: impl FnOnce(W) -> Result<StoredBlock, StoreBuildError>,
) -> Result<CanonicalStoreRef<Kind, Tag>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: Copy + Send + Sync + 'static,
{
    assembly.ensure_writable()?;
    assembly.validate_dimension(work_dimension)?;
    let mut callback_failure = None;
    let result = assembly.raw.try_retain_canonical_controlled(
        work_dimension,
        allocation,
        |emplacer, raw| {
            let mut raw_control = RetainedStoreWorkControl { raw, sticky: None };
            let mut permit = match try_begin_canonical_work(&mut raw_control, work) {
                Ok(permit) => permit,
                Err(cause) => {
                    callback_failure = Some(StoreBuildCallbackFailure::VerifiedWork(cause));
                    return Err(RetainedBuildFailureCode::new(super::BUILD_CODE_DOMAIN));
                }
            };
            let mut writer = match make_writer() {
                Ok(writer) => writer,
                Err(cause) => {
                    permit.resolve_partial();
                    drop(permit);
                    return finish_activation_writer_callback(
                        raw_control,
                        Err(cause.into()),
                        &mut callback_failure,
                    );
                }
            };
            let mut control = CanonicalBuildControl::from_admitted(&mut writer, permit);
            let callback = build(&mut control);
            let callback = control
                .finish(callback)
                .and_then(|()| finish_writer(writer).map_err(StoreControlledError::from));
            let block =
                finish_activation_writer_callback(raw_control, callback, &mut callback_failure)?;
            Ok(emplacer.place(block, tag))
        },
    );
    assembly
        .map_controlled_action(result, callback_failure)
        .map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
}

impl<O, PP, U, T> CanonicalBuildControl<'_, '_, '_, O, PP, U, SequenceWriter<T>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    T: StoredScalar,
{
    /// Appends one scalar and accounts for exactly one completed element.
    pub fn push(&mut self, value: T) -> Result<(), StoreControlledError> {
        self.perform(1, |writer| writer.push(value))
    }

    /// Appends a slice in bounded, cancellation-aware chunks.
    pub fn extend_from_slice(&mut self, values: &[T]) -> Result<(), StoreControlledError> {
        for chunk in values.chunks(FAILURE_EVIDENCE_BUILD_CHUNK_UNITS) {
            let amount = u64::try_from(chunk.len())
                .expect("the fixed canonical build chunk length fits in u64");
            self.perform(amount, |writer| writer.extend_from_slice(chunk))?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.writer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.writer.is_empty()
    }
}

impl<O, PP, U> CanonicalBuildControl<'_, '_, '_, O, PP, U, BytesWriter>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Appends bytes in bounded, cancellation-aware chunks.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), StoreControlledError> {
        for chunk in bytes.chunks(FAILURE_EVIDENCE_BUILD_CHUNK_UNITS) {
            let amount = u64::try_from(chunk.len())
                .expect("the fixed canonical build chunk length fits in u64");
            self.perform(amount, |writer| writer.write_bytes(chunk))?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.writer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.writer.is_empty()
    }
}

impl<O, PP, U> CanonicalBuildControl<'_, '_, '_, O, PP, U, Utf8Writer>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Appends UTF-8 in bounded chunks without splitting a scalar value.
    pub fn write_str(&mut self, value: &str) -> Result<(), StoreControlledError> {
        let mut start = 0;
        while start < value.len() {
            let mut end = start
                .saturating_add(FAILURE_EVIDENCE_BUILD_CHUNK_UNITS)
                .min(value.len());
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            let chunk = &value[start..end];
            let amount = u64::try_from(chunk.len())
                .expect("the fixed canonical build chunk length fits in u64");
            self.perform(amount, |writer| writer.write_str(chunk))?;
            start = end;
        }
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.writer.bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.writer.is_empty()
    }
}

fn perform_canonical_record_field<O, PP, U>(
    writer: &mut CanonicalRecordRowWriter<'_>,
    permit: &mut CanonicalWorkPermit<'_, '_, O, PP, U>,
    work_batch: &mut work_batch::CompletionPrefixBatch,
    sticky: &mut Option<StoreControlledError>,
    action: impl FnOnce(&mut CanonicalRecordRowWriter<'_>) -> Result<(), StoreBuildError>,
) -> Result<(), StoreControlledError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if let Some(cause) = sticky.clone() {
        return Err(cause);
    }
    let remaining = work_batch.effective_remaining(permit.remaining_units());
    if remaining.is_none_or(|remaining| remaining == 0) || work_batch.units_until_checkpoint() == 0
    {
        let cause = canonical_completion_contract_failure(permit, sticky);
        let _ = flush_canonical_completed(permit, work_batch, sticky);
        permit.resolve_partial();
        return Err(cause);
    }
    if let Err(cause) = action(writer) {
        let cause = StoreControlledError::from(cause);
        sticky.get_or_insert_with(|| cause.clone());
        return Err(cause);
    }
    record_canonical_completed(permit, work_batch, sticky, NonZeroU64::MIN)
}

impl<O, PP, U> CanonicalRecordBuildRow<'_, '_, '_, '_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn write_scalar<T: StoredScalar>(&mut self, value: T) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(
            self.writer,
            self.permit,
            self.work_batch,
            self.sticky,
            |writer| writer.write_scalar(value),
        )
    }

    pub fn write_sequence_ref<T: StoredScalar, Tag>(
        &mut self,
        value: CanonicalSequenceRef<T, Tag>,
    ) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(
            self.writer,
            self.permit,
            self.work_batch,
            self.sticky,
            |writer| writer.write_sequence_ref(value),
        )
    }

    pub fn write_bytes_ref<Tag>(
        &mut self,
        value: CanonicalBytesRef<Tag>,
    ) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(
            self.writer,
            self.permit,
            self.work_batch,
            self.sticky,
            |writer| writer.write_bytes_ref(value),
        )
    }

    pub fn write_utf8_ref<Tag>(
        &mut self,
        value: CanonicalUtf8Ref<Tag>,
    ) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(
            self.writer,
            self.permit,
            self.work_batch,
            self.sticky,
            |writer| writer.write_utf8_ref(value),
        )
    }

    pub fn write_record_ref<Tag>(
        &mut self,
        value: CanonicalRecordRef<Tag>,
    ) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(
            self.writer,
            self.permit,
            self.work_batch,
            self.sticky,
            |writer| writer.write_record_ref(value),
        )
    }
}

impl<O, PP, U> CanonicalBuildControl<'_, '_, '_, O, PP, U, CanonicalRecordWriter>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Writes one exact-width row while accounting for every completed field.
    pub fn write_row(
        &mut self,
        write: impl FnOnce(
            &mut CanonicalRecordBuildRow<'_, '_, '_, '_, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<(), StoreControlledError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        self.poll_now()?;
        let mut callback_error = None;
        let result = self.writer.write_row(|writer| {
            let permit = self
                .permit
                .as_mut()
                .expect("a fault-free canonical record builder retains its work permit");
            let mut row = CanonicalRecordBuildRow {
                writer,
                permit,
                work_batch: &mut self.work_batch,
                sticky: &mut self.sticky,
            };
            match write(&mut row) {
                Ok(()) => Ok(()),
                Err(cause) => {
                    callback_error = Some(cause);
                    Err(StoreBuildError::incomplete_write())
                }
            }
        });
        if let Some(cause) = callback_error {
            self.sticky.get_or_insert_with(|| cause.clone());
            self.resolve_partial();
            return Err(cause);
        }
        if let Err(cause) = result {
            let cause = StoreControlledError::from(cause);
            self.sticky.get_or_insert_with(|| cause.clone());
            self.resolve_partial();
            return Err(cause);
        }
        let permit = self
            .permit
            .as_mut()
            .expect("a successful canonical record row retains its work permit");
        flush_canonical_completed(permit, &mut self.work_batch, &mut self.sticky)
    }

    pub fn rows(&self) -> usize {
        self.writer.rows()
    }
}

impl<O, PP, U> CanonicalWorkPermit<'_, '_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn record_product_graph_failure(&mut self, code: u32) {
        if let Some(raw) = self.raw.as_mut() {
            raw.record_product_graph_failure(code);
        }
    }

    fn resolve_partial(&mut self) {
        if let Some(raw) = self.raw.take() {
            raw.finish_partial();
        }
    }

    pub fn poll(&mut self) -> Result<(), StoreWorkError> {
        let raw = self
            .raw
            .as_mut()
            .expect("an unresolved canonical work permit retains its authority");
        match raw.poll() {
            Ok(()) => Ok(()),
            Err(cause) => {
                self.resolve_partial();
                let cause = StoreWorkError::control(cause);
                self.sticky.get_or_insert_with(|| cause.clone());
                Err(cause)
            }
        }
    }

    fn remaining_units(&self) -> u64 {
        self.raw
            .as_ref()
            .expect("an unresolved canonical work permit retains its authority")
            .remaining_units()
    }

    fn complete_units(&mut self, amount: NonZeroU64) -> Result<(), StoreWorkError> {
        let raw = self
            .raw
            .as_mut()
            .expect("an unresolved canonical work permit retains its authority");
        match raw.complete_units(amount) {
            Ok(()) => Ok(()),
            Err(RetainedWorkConsumeError::Ledger(cause)) => {
                self.resolve_partial();
                let cause = StoreWorkError::ledger(cause);
                self.sticky.get_or_insert_with(|| cause.clone());
                Err(cause)
            }
            Err(RetainedWorkConsumeError::ExceedsAttempted) => {
                panic!("canonical work completion exceeded its admitted envelope")
            }
        }
    }

    fn finish_exact(mut self) -> bool {
        self.raw
            .take()
            .is_some_and(|raw| matches!(raw.complete(), Ok(())))
    }
}

enum OutcomeEvidenceWorkPermit<'a, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Ordinary(RawOutcomeEvidenceWorkPermit<'a, O, PP, U>),
    Protected(RawFailureEvidenceWorkPermit<'a, O, PP, U>),
}

/// Permit-bound writer for one admitted outcome-evidence member.
///
/// The complete sealed member plan is recorded as attempted before the builder runs. Each mutating
/// method polls control state and records only the payload it actually advances as consumed. A
/// writer error, cancellation, panic, or abandoned permit therefore preserves the unfinished
/// attempted-only suffix. Success construction uses ordinary work; failure-only construction uses
/// the protected terminal authority without exposing either distinction to the builder.
pub struct OutcomeEvidenceBuildControl<'a, O, PP, U, W>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    writer: W,
    permit: Option<OutcomeEvidenceWorkPermit<'a, O, PP, U>>,
    record_action_work: u64,
    sticky: Option<EvidenceBuildCallbackError>,
}

#[derive(Clone)]
enum EvidenceBuildCallbackError {
    Build(StoreBuildError),
    Runtime(StoreRuntimeError),
}

/// Operation-owned validator for one exact rich-failure evidence graph.
///
/// No raw slice, string, record view, protected dimension, or caller-selected work amount crosses
/// this boundary. Inspectors advance one concrete data unit at a time; completed prefixes are
/// flushed to the linear permit at bounded checkpoints and semantic boundaries. A repeated DAG
/// edge to an already completed member succeeds without running that member's inspector again.
pub struct FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: StoreEvidenceValidationSession<'session, 'storage>,
    permit: Option<RawFailureEvidenceWorkPermit<'control, O, PP, U>>,
    sticky: &'control mut Option<StoreRuntimeError>,
    completed_units: u64,
    rejected: bool,
    work_batch: work_batch::CompletionPrefixBatch,
}

/// Session-branded authority to validate one root-reachable failure-evidence member.
///
/// Values are issued only for the validation root or a typed record edge. They are deliberately
/// neither `Copy` nor constructible by callers, so a slot captured during construction cannot be
/// substituted for a member reached from the validation root.
#[must_use = "every root-reachable failure-evidence member must enter controlled validation"]
pub struct FailureEvidenceMember<'session, Kind, Tag> {
    raw: OutcomeEvidenceStoreRef<Kind, Tag>,
    validation: PhantomData<fn(&'session mut ()) -> &'session mut ()>,
}

type FailureEvidenceSequenceMember<'session, T, Tag> =
    FailureEvidenceMember<'session, ScalarSequence<T>, Tag>;
type FailureEvidenceBytesMember<'session, Tag> = FailureEvidenceMember<'session, ByteBlock, Tag>;
type FailureEvidenceUtf8Member<'session, Tag> = FailureEvidenceMember<'session, Utf8Block, Tag>;
type FailureEvidenceRecordMember<'session, Tag> = FailureEvidenceMember<'session, RecordBlock, Tag>;

/// One fixed-width record row whose fields can only be inspected through metered typed accessors.
pub struct FailureEvidenceRecordRow<'row, 'session, 'storage, 'control, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    session: &'row mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
    view: OutcomeEvidenceRecordView<'storage>,
    row: usize,
    fields: usize,
    visited: u64,
}

/// Linear owner for one session outcome and one still-dormant canonical authority.
#[must_use = "the session outcome owner must run its scoped activation transition"]
pub struct RetainedStoreSessionOutcomeOwner<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: Option<RetainedStoreOperation<O, PP, U, L>>,
    roles: Option<BoundActivationRoles>,
    failure_work: FailureEvidenceWorkBudget,
}

#[allow(
    clippy::type_complexity,
    reason = "the typed canonical root must remain coupled to every activation outcome"
)]
fn finish_rejected<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>(
    assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    cause: StoreCanonicalRejection<E>,
    failure_code: PreparationFailureCode,
) -> Result<
    CanonicalActivationOutcome<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    StoreActivationContainment,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let cause = ManuallyDrop::new(cause);
    let RetainedStorePublicationAssembly {
        raw,
        fallback_cause: _,
        success_rejection: _,
        domain,
        lineage,
    } = assembly;
    let lineage = ManuallyDrop::new(lineage);
    match raw.try_discard_canonical_for_rejection(failure_code) {
        Ok(raw) => {
            if drop_scoped_value(ManuallyDrop::into_inner(lineage)).is_err() {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
            Ok(CanonicalActivationOutcome::Rejected(
                RejectedCanonicalStoreSession {
                    owner: UnrootedFailureEvidence {
                        raw,
                        domain,
                        failure_code,
                        cause: ManuallyDrop::into_inner(cause),
                        failure_work,
                        outcome_evidence_destination_dimension,
                    },
                },
            ))
        }
        Err(containment) => {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            let _ = drop_scoped_value(ManuallyDrop::into_inner(lineage));
            Err(StoreActivationContainment { cause: containment })
        }
    }
}

struct ActivationFirstFault {
    cause: StoreRuntimeError,
    contract_seen: bool,
}

impl ActivationFirstFault {
    fn recoverable_code(&self) -> Option<PreparationFailureCode> {
        match classify_activation_fault(&self.cause) {
            ActivationFaultClassification::Recoverable(code) => Some(code),
            ActivationFaultClassification::Contract => None,
        }
    }
}

fn preserve_first_fault<T>(
    first_fault: &mut Option<ActivationFirstFault>,
    result: Result<T, StoreRuntimeError>,
) -> Result<T, StoreRuntimeError> {
    if let Err(cause) = &result {
        let classification = classify_activation_fault(cause);
        let contract = matches!(classification, ActivationFaultClassification::Contract);
        match first_fault {
            Some(first) => first.contract_seen |= contract,
            None => {
                *first_fault = Some(ActivationFirstFault {
                    cause: cause.clone(),
                    contract_seen: contract,
                });
            }
        }
    }
    result
}

fn preserved_fault(first_fault: &Option<ActivationFirstFault>) -> Option<StoreRuntimeError> {
    first_fault.as_ref().map(|fault| fault.cause.clone())
}

fn note_later_contract<T>(
    first_fault: &mut Option<ActivationFirstFault>,
    result: Result<T, StoreRuntimeError>,
) {
    if let Err(cause) = result {
        let _ = preserve_first_fault::<()>(first_fault, Err(cause));
    }
}

fn drop_verified_rejection<E>(rejection: VerifiedRejection<E>) -> Result<(), ()> {
    let VerifiedRejection {
        cause,
        failure_code: _,
    } = rejection;
    drop_scoped_value(cause)
}

fn drop_materialization<T, E, Kind, Tag>(
    materialized: CanonicalMaterializationState<T, E, Kind, Tag>,
) -> Result<(), ()> {
    match materialized {
        CanonicalMaterializationState::Activated { root: _, output } => drop_scoped_value(output),
        CanonicalMaterializationState::Rejected(rejection) => drop_verified_rejection(rejection),
    }
}

fn drop_scoped_value<T>(value: T) -> Result<(), ()> {
    match catch_unwind(AssertUnwindSafe(|| drop(value))) {
        Ok(()) => Ok(()),
        Err(payload) => {
            discard_untrusted_panic_payload(payload);
            Err(())
        }
    }
}

fn drop_optional_callback<F>(callback: ManuallyDrop<Option<F>>) -> Result<(), ()> {
    match ManuallyDrop::into_inner(callback) {
        Some(callback) => drop_scoped_value(callback),
        None => Ok(()),
    }
}

fn finish_scoped_callback<F, T>(
    callback: ManuallyDrop<Option<F>>,
    result: Result<T, StoreRuntimeError>,
) -> Result<T, StoreRuntimeError> {
    if drop_optional_callback(callback).is_ok() {
        result
    } else {
        let _ = drop_scoped_value(result);
        Err(StoreRuntimeError::BuilderPanicked)
    }
}

#[derive(Clone, Copy)]
enum ActivationFaultClassification {
    Contract,
    Recoverable(PreparationFailureCode),
}

fn classify_activation_fault(cause: &StoreRuntimeError) -> ActivationFaultClassification {
    match cause {
        StoreRuntimeError::Plan(_)
        | StoreRuntimeError::BatchCapacity
        | StoreRuntimeError::ForeignPhaseDomain { .. }
        | StoreRuntimeError::ForeignDimensionDomain { .. }
        | StoreRuntimeError::PhaseCompletionRegression { .. }
        | StoreRuntimeError::PhaseCompletionPublicationPending
        | StoreRuntimeError::PhaseCompletionPublicationCompleted
        | StoreRuntimeError::PhaseCompletionCatalogShapeMismatch
        | StoreRuntimeError::AccountPoisoned(_)
        | StoreRuntimeError::AccountPoisonedWithRetainedCharge(_)
        | StoreRuntimeError::BuilderPanicked
        | StoreRuntimeError::ConstructionInvariantUnverified
        | StoreRuntimeError::WrongOutcomeMode
        | StoreRuntimeError::InvalidSourceSlot
        | StoreRuntimeError::ForeignAccount
        | StoreRuntimeError::MembershipExhausted
        | StoreRuntimeError::InvalidPublicationPlan
        | StoreRuntimeError::StorageDropUnverified
        | StoreRuntimeError::OwnershipUnverified
        | StoreRuntimeError::ChargeReleaseUnverified
        | StoreRuntimeError::SuccessOutcomeRejected
        | StoreRuntimeError::CanonicalValidationContractRejected
        | StoreRuntimeError::UnverifiedControlledCause
        | StoreRuntimeError::FallbackOnly
        | StoreRuntimeError::Poisoned => ActivationFaultClassification::Contract,
        StoreRuntimeError::Control(cause) => {
            ActivationFaultClassification::Recoverable(PreparationFailureCode::from_control(cause))
        }
        StoreRuntimeError::Ledger(_) | StoreRuntimeError::AccountingOverflow => {
            ActivationFaultClassification::Recoverable(PreparationFailureCode::ledger())
        }
        StoreRuntimeError::Build(cause) => ActivationFaultClassification::Recoverable(
            PreparationFailureCode::product_graph(if cause.code() == 0 {
                ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE
            } else {
                cause.code()
            }),
        ),
        StoreRuntimeError::BatchAllocationFailed
        | StoreRuntimeError::PhysicalByteLimit { .. }
        | StoreRuntimeError::PhysicalAllocationLimit { .. }
        | StoreRuntimeError::AllocationFailed
        | StoreRuntimeError::SlotLimit { .. } => ActivationFaultClassification::Recoverable(
            PreparationFailureCode::product_graph(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE),
        ),
    }
}

fn activation_product_graph_cause() -> StoreRuntimeError {
    StoreRuntimeError::Build(
        StoreBuildError::domain(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE)
            .expect("the activation product-graph code is inside the domain range"),
    )
}

fn failure_work_envelope_exhausted() -> StoreRuntimeError {
    StoreRuntimeError::Build(
        StoreBuildError::domain(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE)
            .expect("the failure-work envelope code is inside the domain range"),
    )
}

fn prepare_product_graph_fallback<O, PP, U>(
    fallback: StoreFallbackOnly<O, PP, U>,
) -> Result<VerifiedMinimalFallback<StoreRuntimeError>, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match fallback.prepare_minimal_fallback() {
        Ok(prepared) => Ok(VerifiedMinimalFallback {
            prepared,
            cause: activation_product_graph_cause(),
        }),
        Err(StorePrepareError::FinalizationContainment(cause)) => {
            Err(StoreActivationContainment { cause })
        }
    }
}

fn contain_operation<O, PP, U, L>(
    operation: RetainedStoreOperation<O, PP, U, L>,
) -> StoreActivationContainment
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let RetainedStoreOperation {
        raw,
        domain: _,
        lineage,
    } = operation;
    let _ = drop_scoped_value(lineage);
    StoreActivationContainment {
        cause: raw.contain_activation_fault(),
    }
}

// Keeping the linear operation owner inline avoids an unmetered allocation on activation failure.
#[allow(clippy::large_enum_variant)]
enum ActivationOpenFailure<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Recoverable {
        operation: RetainedStoreOperation<O, PP, U, L>,
        cause: StoreRuntimeError,
        failure_code: PreparationFailureCode,
        fallback_only: bool,
    },
    Contained(FinalizationContainmentError),
}

#[allow(clippy::result_large_err, clippy::type_complexity)]
fn try_open_activation<O, PP, U, L>(
    operation: RetainedStoreOperation<O, PP, U, L>,
    plan: RetainedStorePublicationPlan,
) -> Result<RetainedStorePublicationAssembly<O, PP, U, L>, ActivationOpenFailure<O, PP, U, L>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if operation.validate_publication_plan_keys(&plan).is_err() {
        return Err(ActivationOpenFailure::Contained(
            contain_operation(operation).cause,
        ));
    }
    let RetainedStoreOperation {
        raw,
        domain,
        lineage,
    } = operation;
    match raw.try_publication_assembly(plan.into_raw()) {
        Ok(raw) => Ok(RetainedStorePublicationAssembly {
            raw,
            fallback_cause: None,
            success_rejection: None,
            domain,
            lineage,
        }),
        Err(error) => {
            let cause = map_writer_error(error.cause());
            let classification = classify_activation_fault(&cause);
            match error.into_activation_recovery() {
                Ok(recovery) => {
                    let (raw, fallback_only) = match recovery {
                        RetainedPublicationOpenRecovery::Retry(raw) => (raw, false),
                        RetainedPublicationOpenRecovery::FallbackOnly(raw) => (raw, true),
                    };
                    let mut operation = RetainedStoreOperation {
                        raw,
                        domain,
                        lineage,
                    };
                    match classification {
                        ActivationFaultClassification::Contract => Err(
                            ActivationOpenFailure::Contained(contain_operation(operation).cause),
                        ),
                        ActivationFaultClassification::Recoverable(failure_code) => {
                            operation.latch_activation_terminal_cause(failure_code);
                            Err(ActivationOpenFailure::Recoverable {
                                operation,
                                cause,
                                failure_code,
                                fallback_only,
                            })
                        }
                    }
                }
                Err(containment) => Err(ActivationOpenFailure::Contained(containment)),
            }
        }
    }
}

/// Borrowed preflight capability. It cannot open or mutate retained tables.
pub struct CanonicalPreflight<'scope, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: &'scope mut RetainedStoreOperation<O, PP, U, L>,
    request: &'scope BoundActivationRequest,
    first_fault: &'scope mut Option<ActivationFirstFault>,
    semantic_work_complete: &'scope mut bool,
}

/// Control-and-budget facade whose every failure is preserved by the activation transaction.
pub struct CanonicalControlCheckpoint<'borrow, 'activation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RetainedStoreControlCheckpoint<'borrow, O, PP, U>,
    first_fault: &'borrow mut Option<ActivationFirstFault>,
    activation: PhantomData<fn(&'activation ()) -> &'activation ()>,
}

/// Materialization checkpoint for control polling and scoped scratch reservations.
pub struct CanonicalMaterializationCheckpoint<'borrow, 'activation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    inner: CanonicalControlCheckpoint<'borrow, 'activation, O, PP, U>,
}

/// Scoped live-storage reservation for canonical construction scratch.
///
/// The invariant lifetime brand prevents a reservation from becoming part of a preflight decision
/// or materialization output. The type remains private to the scoped callback surface, so the
/// activation integration tests exercise the real API rather than importing an unexported type.
#[must_use = "dropping a canonical reservation releases its live-byte accounting"]
pub struct CanonicalReservation<'scope, O>
where
    O: OperationOwner,
{
    raw: RetainedStoreReservation<O>,
    scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

/// Opaque result returned by the scoped preflight callback.
#[must_use = "return the sealed activation or verified rejection to the owning session"]
pub struct CanonicalPreflightDecision<'scope, E> {
    state: CanonicalPreflightState<E>,
    scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

impl<E> CanonicalPreflightDecision<'_, E> {
    fn into_state(self) -> CanonicalPreflightState<E> {
        self.state
    }
}

enum CanonicalPreflightState<E> {
    Activate,
    Rejected(VerifiedRejection<E>),
}

/// Operation-neutral retained table shape used by one canonical activation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalTableShape {
    total_blocks: usize,
    max_new_blocks: usize,
}

impl CanonicalTableShape {
    pub fn new(total_blocks: usize, max_new_blocks: usize) -> Result<Self, StorePlanError> {
        if total_blocks == 0
            || max_new_blocks > total_blocks
            || u64::try_from(total_blocks).is_err()
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        Ok(Self {
            total_blocks,
            max_new_blocks,
        })
    }

    fn bind(
        self,
        writer: RetainedStoreWriterPlan,
    ) -> Result<RetainedStoreBlockTablePlan, StorePlanError> {
        RetainedStoreBlockTablePlan::new(writer, self.total_blocks, self.max_new_blocks)
    }
}

/// Keyless requirements produced by canonical core preflight.
///
/// The request describes exact physical shape plus semantic-preflight and validation work. Ledger
/// roles, writer keys, destination dimensions, publication authority, and commitment amounts
/// remain operation-owned.
#[must_use = "canonical requirements must be sealed by their operation-issued sealer"]
pub struct CanonicalActivationRequirements {
    canonical: CanonicalTableShape,
    outcome_evidence: CanonicalTableShape,
    expected_materialized_blocks: usize,
    semantic_preflight_work: u64,
    validation_work: NonZeroU64,
    pre_activation_structural_work_requirement: u64,
    structural_work_requirement: u64,
}

impl CanonicalActivationRequirements {
    pub fn new(
        canonical: CanonicalTableShape,
        outcome_evidence: CanonicalTableShape,
        expected_materialized_blocks: usize,
        semantic_preflight_work: u64,
        validation_work: NonZeroU64,
    ) -> Result<Self, StorePlanError> {
        if expected_materialized_blocks == 0
            || expected_materialized_blocks > canonical.max_new_blocks
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let actions = STARTUP_ACCOUNT_SHARE_STEP_COUNT
            .checked_add(1)
            .and_then(|actions| actions.checked_add(2))
            .and_then(|actions| actions.checked_add(canonical.total_blocks))
            .and_then(|actions| actions.checked_add(outcome_evidence.total_blocks))
            .ok_or(StorePlanError::SizeOverflow)?;
        let work_per_action = RetainedHandleSharePlan::new((), ())
            .charges()
            .into_iter()
            .try_fold(0_u64, |total, (_, amount)| {
                total
                    .checked_add(amount)
                    .ok_or(StorePlanError::SizeOverflow)
            })?;
        let pre_activation_structural_work_requirement =
            u64::try_from(STARTUP_ACCOUNT_SHARE_STEP_COUNT)
                .ok()
                .and_then(|actions| actions.checked_mul(work_per_action))
                .ok_or(StorePlanError::SizeOverflow)?;
        let structural_work_requirement = u64::try_from(actions)
            .ok()
            .and_then(|actions| actions.checked_mul(work_per_action))
            .ok_or(StorePlanError::SizeOverflow)?;
        Ok(Self {
            canonical,
            outcome_evidence,
            expected_materialized_blocks,
            semantic_preflight_work,
            validation_work,
            pre_activation_structural_work_requirement,
            structural_work_requirement,
        })
    }

    /// Returns the operation-owned work that is complete before the activation lane can open.
    pub const fn pre_activation_structural_work_requirement(&self) -> u64 {
        self.pre_activation_structural_work_requirement
    }

    /// Returns structural startup plus the exact producer semantic-preflight envelope.
    pub const fn pre_activation_work_requirement(&self) -> Option<u64> {
        self.pre_activation_structural_work_requirement
            .checked_add(self.semantic_preflight_work)
    }

    /// Returns the work bound derived from the exact table plans this shape owns.
    pub const fn structural_work_requirement(&self) -> u64 {
        self.structural_work_requirement
    }
}

/// Host-owned role policy consumed before any canonical activation request is accepted.
///
/// Core and format code never receive this value. The session owner validates and seals all role
/// keys before binding a keyless request to concrete retained-table plans.
#[must_use = "the activation policy must be consumed by one retained-store operation"]
pub struct CanonicalActivationPolicy {
    canonical_writer: RetainedStoreWriterPlan,
    outcome_evidence_writer: RetainedStoreWriterPlan,
    materialization_work_dimension: StoreDimensionKey,
    validation_work_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
}

impl CanonicalActivationPolicy {
    pub const fn new(
        canonical_writer: RetainedStoreWriterPlan,
        outcome_evidence_writer: RetainedStoreWriterPlan,
        materialization_work_dimension: StoreDimensionKey,
        validation_work_dimension: StoreDimensionKey,
        canonical_destination_dimension: StoreDimensionKey,
        outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            canonical_writer,
            outcome_evidence_writer,
            materialization_work_dimension,
            validation_work_dimension,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
        }
    }
}

struct BoundActivationRoles {
    policy: CanonicalActivationPolicy,
    brand: StoreActivationBrand,
}

/// Linear sealer issued by the operation owner for one matching keyless request.
#[must_use = "the activation sealer must seal exactly one canonical request"]
pub struct CanonicalActivationRequestSealer {
    brand: StoreActivationBrand,
}

impl CanonicalActivationRequestSealer {
    pub fn seal(
        self,
        requirements: CanonicalActivationRequirements,
    ) -> SealedCanonicalActivationRequest {
        SealedCanonicalActivationRequest {
            brand: self.brand,
            requirements,
        }
    }
}

/// Single-use keyless request accepted only by the outcome owner that issued its sealer.
#[must_use = "the sealed activation request must be consumed by its session outcome owner"]
pub struct SealedCanonicalActivationRequest {
    brand: StoreActivationBrand,
    requirements: CanonicalActivationRequirements,
}

struct BoundActivationRequest {
    canonical_plan: RetainedStoreBlockTablePlan,
    outcome_evidence_plan: RetainedStoreBlockTablePlan,
    materialization_work_dimension: StoreDimensionKey,
    validation_work_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    semantic_preflight_work: u64,
    validation_work: NonZeroU64,
    expected_materialized_blocks: usize,
}

impl BoundActivationRoles {
    fn bind(
        self,
        request: SealedCanonicalActivationRequest,
    ) -> Result<BoundActivationRequest, StorePlanError> {
        if request.brand != self.brand {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let CanonicalActivationRequirements {
            canonical,
            outcome_evidence,
            expected_materialized_blocks,
            semantic_preflight_work,
            validation_work,
            pre_activation_structural_work_requirement: _,
            structural_work_requirement: _,
        } = request.requirements;
        Ok(BoundActivationRequest {
            canonical_plan: canonical.bind(self.policy.canonical_writer)?,
            outcome_evidence_plan: outcome_evidence.bind(self.policy.outcome_evidence_writer)?,
            materialization_work_dimension: self.policy.materialization_work_dimension,
            validation_work_dimension: self.policy.validation_work_dimension,
            canonical_destination_dimension: self.policy.canonical_destination_dimension,
            outcome_evidence_destination_dimension: self
                .policy
                .outcome_evidence_destination_dimension,
            semantic_preflight_work,
            validation_work,
            expected_materialized_blocks,
        })
    }
}

struct VerifiedRejection<E> {
    cause: E,
    failure_code: PreparationFailureCode,
}

/// Canonical-only lane borrowed for exactly one materialization callback.
pub struct CanonicalLane<'lane, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly: &'lane mut RetainedStorePublicationAssembly<O, PP, U, L>,
    first_fault: &'lane mut Option<ActivationFirstFault>,
    materialization_work_dimension: StoreDimensionKey,
}

/// Opaque result returned by the scoped materialization callback.
#[must_use = "return the materialized candidate or typed rejection to the owning session"]
pub struct CanonicalMaterialization<'lane, T, E, Kind, Tag> {
    state: CanonicalMaterializationState<T, E, Kind, Tag>,
    lane: PhantomData<fn(&'lane ()) -> &'lane ()>,
}

impl<T, E, Kind, Tag> CanonicalMaterialization<'_, T, E, Kind, Tag> {
    fn into_state(self) -> CanonicalMaterializationState<T, E, Kind, Tag> {
        self.state
    }
}

enum CanonicalMaterializationState<T, E, Kind, Tag> {
    Activated {
        root: CanonicalStoreRef<Kind, Tag>,
        output: T,
    },
    Rejected(VerifiedRejection<E>),
}

/// Exact pre-activation rejection or recoverable open cause coupled to its continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreCanonicalRejection<E> {
    Requested(E),
    Runtime(StoreRuntimeError),
}

/// A materialized canonical candidate whose physical graph has not yet been published.
///
/// The activation seam deliberately exposes no writable assembly or raw ownership bridge.
#[must_use = "the candidate must be prepared or explicitly downgraded"]
pub struct ActivatedCanonicalStoreSession<
    O,
    PP,
    U,
    T,
    CanonicalKind,
    CanonicalTag,
    L = FreshStoreLineage,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    output: T,
    failure_work: FailureEvidenceWorkBudget,
    validation_work_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    validation_work: NonZeroU64,
}

/// Typed semantic outcome returned by the central validator callback.
pub enum CanonicalValidationDecision<Proof, E> {
    Accept(Proof),
    Reject { cause: E, stable_code: u32 },
}

fn drop_validation_decision<Proof, E>(
    decision: Option<CanonicalValidationDecision<Proof, E>>,
) -> Result<(), ()> {
    match decision {
        Some(CanonicalValidationDecision::Accept(proof)) => drop_scoped_value(proof),
        Some(CanonicalValidationDecision::Reject {
            cause,
            stable_code: _,
        }) => drop_scoped_value(cause),
        None => Ok(()),
    }
}

/// One-shot evidence decision used by the central success path.
pub enum OutcomeEvidenceDecision<Kind, Tag> {
    Complete(OutcomeEvidenceStoreRef<Kind, Tag>),
    Abort,
}

/// Candidate after central semantic validation. It contains no writable or raw store seam.
///
/// This activation-only disposition deliberately has no prepare or handoff method. The later
/// cross-crate joint-commit slice must combine an operation-issued freeze permit with the exact
/// core-owned generation contribution before either side can prepare a success.
#[must_use = "the validated candidate must enter the later joint-commit transition"]
pub struct CanonicalCandidateStoreSession<
    O,
    PP,
    U,
    T,
    Proof,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    destinations: ZeroCommitDestinationFacts,
    proof: Proof,
    output: T,
    _failure_work: FailureEvidenceWorkBudget,
    // This owner must drop last so payload panics still trigger unwind-aware account containment.
    _validated:
        ValidatedStoreSuccess<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>,
}

/// Exact destination facts derived from a validated physical closure.
///
/// These facts contain no applied or staged ledger commitment. U4e consumes them only after the
/// matching freeze contribution and final eligibility are ready.
struct ZeroCommitDestinationFacts {
    _canonical_dimension: StoreDimensionKey,
    canonical_members: NonZeroU64,
    _outcome_evidence_dimension: StoreDimensionKey,
    outcome_evidence_members: NonZeroU64,
}

impl ZeroCommitDestinationFacts {
    fn try_new(
        canonical_dimension: StoreDimensionKey,
        canonical_members: usize,
        outcome_evidence_dimension: StoreDimensionKey,
        outcome_evidence_members: usize,
    ) -> Result<Self, StorePlanError> {
        let canonical_members = u64::try_from(canonical_members)
            .ok()
            .and_then(NonZeroU64::new)
            .ok_or(StorePlanError::InvalidBlockTable)?;
        let outcome_evidence_members = u64::try_from(outcome_evidence_members)
            .ok()
            .and_then(NonZeroU64::new)
            .ok_or(StorePlanError::InvalidBlockTable)?;
        Ok(Self {
            _canonical_dimension: canonical_dimension,
            canonical_members,
            _outcome_evidence_dimension: outcome_evidence_dimension,
            outcome_evidence_members,
        })
    }
}

/// A centrally rejected canonical candidate with an eager-cleaned, rooted failure owner.
#[must_use = "the rejected candidate must validate rich evidence or prepare fallback"]
pub struct CentrallyRejectedCanonicalStoreSession<O, PP, U, E, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    root: RootedFailureEvidence<O, PP, U, StoreCanonicalRejection<E>, EvidenceKind, EvidenceTag>,
}

/// Rich-failure validation result for a centrally rejected canonical candidate.
pub type CanonicalRejectionFailureValidation<O, PP, U, E, EvidenceKind, EvidenceTag> =
    RootedFailureValidation<O, PP, U, StoreCanonicalRejection<E>, EvidenceKind, EvidenceTag>;

/// A centrally interrupted candidate with a trusted control or ledger cause.
#[must_use = "the interrupted candidate must validate evidence or prepare fallback"]
pub struct InterruptedCanonicalStoreSession<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    root: RootedFailureEvidence<O, PP, U, StoreWorkError, EvidenceKind, EvidenceTag>,
}

/// Opaque validation outcome for a materialized canonical session.
#[must_use = "every central-validation branch retains its exact owner"]
pub enum CanonicalCentralValidationOutcome<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Candidate(
        CanonicalCandidateStoreSession<
            O,
            PP,
            U,
            T,
            Proof,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
    ),
    Rejected(CentrallyRejectedCanonicalStoreSession<O, PP, U, E, EvidenceKind, EvidenceTag>),
    Fallback(VerifiedMinimalFallback<StoreRuntimeError>),
    Interrupted(InterruptedCanonicalStoreSession<O, PP, U, EvidenceKind, EvidenceTag>),
}

/// One-shot canonical outcome-evidence writer used only before central validation.
pub struct CanonicalOutcomeEvidenceLane<'lane, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly: &'lane mut RetainedStorePublicationAssembly<O, PP, U, L>,
    first_fault: &'lane mut Option<ActivationFirstFault>,
    failure_work: &'lane mut FailureEvidenceWorkBudget,
    work_dimension: StoreDimensionKey,
}

/// One-shot decision for rejected or never-activated evidence.
pub enum FailureEvidenceDecision<Kind, Tag> {
    Complete(OutcomeEvidenceStoreRef<Kind, Tag>),
    Abort,
}

/// Unrooted failure owner shared by materialization rejection and never-activation.
struct UnrootedFailureEvidence<O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreFailureAssembly<O, PP, U>,
    domain: StoreKeyDomain,
    failure_code: PreparationFailureCode,
    cause: Cause,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
}

/// Verified post-activation rejection after eager canonical cleanup.
#[must_use = "rejected canonical state must build rich evidence or prepare minimal fallback"]
pub struct RejectedCanonicalStoreSession<O, PP, U, E>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    owner: UnrootedFailureEvidence<O, PP, U, StoreCanonicalRejection<E>>,
}

/// Failure-evidence construction result for a rejected canonical candidate.
pub type CanonicalRejectionEvidenceOutcome<O, PP, U, E, EvidenceKind, EvidenceTag> =
    FailureEvidenceOutcome<O, PP, U, StoreCanonicalRejection<E>, EvidenceKind, EvidenceTag>;

/// One-shot failure-evidence lane. It is consumed together with the owner and cannot be reopened.
pub struct FailureEvidenceLane<'lane, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: &'lane mut RawStoreFailureAssembly<O, PP, U>,
    first_fault: &'lane mut Option<ActivationFirstFault>,
    failure_work: &'lane mut FailureEvidenceWorkBudget,
}

/// Rooted failure evidence. No mutation API is exposed after the one-shot lane completes.
#[must_use = "rooted failure evidence must be validated or prepared"]
pub struct RootedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: Cause,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
}

/// Result of validating an already-rooted failure graph.
#[must_use = "the validated or rejected failure owner must be consumed"]
pub enum RootedFailureValidation<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(ValidatedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>),
    Rejected(RejectedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>),
    Fallback(VerifiedMinimalFallback<StoreRuntimeError>),
}

#[must_use = "validated failure evidence must be prepared or downgraded"]
pub struct ValidatedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    validated: ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: Cause,
}

#[must_use = "prepared failure evidence must be handed to the terminal publisher"]
pub struct PreparedFailureEvidence<Cause, EvidenceKind, EvidenceTag> {
    prepared: PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag>,
    cause: Cause,
}

#[must_use = "rejected failure evidence must prepare the minimal fallback"]
pub struct RejectedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: Cause,
}

/// Minimal fallback whose typed source cause remains available without exposing physical state.
#[must_use = "the verified minimal fallback must be handed to the terminal publisher"]
pub struct VerifiedMinimalFallback<Cause> {
    prepared: PreparedStoreFallback,
    cause: Cause,
}

// Both variants are linear terminal owners. Boxing the larger owner would bypass the store ledger.
#[allow(clippy::large_enum_variant)]
#[must_use = "the rooted evidence or minimal fallback owner must be consumed"]
pub enum FailureEvidenceOutcome<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Rooted(RootedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>),
    MinimalFallback(VerifiedMinimalFallback<Cause>),
}

/// Exact post-activation rejection cause coupled to its evidence-only continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreNeverActivatedCause<E> {
    Requested(E),
    Runtime(StoreRuntimeError),
}

enum NeverActivatedAuthority<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Operation(RetainedStoreOperation<O, PP, U, L>),
    FallbackOnly(RetainedStoreOperation<O, PP, U, L>),
}

/// Session state proving that no canonical lane was exposed.
#[must_use = "never-activated state must publish rich evidence or minimal fallback"]
pub struct NeverActivatedStoreSession<O, PP, U, E, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    authority: NeverActivatedAuthority<O, PP, U, L>,
    cause: StoreNeverActivatedCause<E>,
    failure_code: PreparationFailureCode,
    outcome_evidence_plan: RetainedStoreBlockTablePlan,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
}

fn prepare_operation_minimal<O, PP, U, L, Cause>(
    mut operation: RetainedStoreOperation<O, PP, U, L>,
    failure_code: PreparationFailureCode,
    cause: Cause,
) -> Result<VerifiedMinimalFallback<Cause>, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation.raw.poll_terminal_control();
    let RetainedStoreOperation {
        raw,
        domain,
        lineage,
    } = operation;
    if drop_scoped_value(lineage).is_err() {
        let _ = drop_scoped_value(cause);
        return Err(StoreActivationContainment {
            cause: raw.contain_activation_fault(),
        });
    }
    let prepared = raw
        .prepare_minimal_fallback::<super::StoredBlock, super::StoredBlock>(failure_code)
        .map(|raw| classify_prepared_fallback(raw, domain));
    match prepared {
        Ok(prepared) => Ok(VerifiedMinimalFallback { prepared, cause }),
        Err(error) => {
            let _ = drop_scoped_value(cause);
            Err(StoreActivationContainment {
                cause: map_raw_prepare_error(error),
            })
        }
    }
}

fn prepare_assembly_minimal<O, PP, U, L>(
    mut assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    cause: StoreRuntimeError,
    failure_code: PreparationFailureCode,
) -> Result<VerifiedMinimalFallback<StoreRuntimeError>, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly.latch_activation_terminal_cause(failure_code);
    let RetainedStorePublicationAssembly {
        raw,
        fallback_cause,
        success_rejection,
        domain,
        lineage,
    } = assembly;
    let cause = ManuallyDrop::new(cause);
    let lineage = ManuallyDrop::new(lineage);
    let failure_code = fallback_cause.or(success_rejection).unwrap_or(failure_code);
    if drop_scoped_value(ManuallyDrop::into_inner(lineage)).is_err() {
        let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
        return Err(StoreActivationContainment {
            cause: raw.contain_activation_fault(),
        });
    }
    match raw
        .prepare_minimal_fallback_with(failure_code)
        .map(|prepared| classify_prepared_fallback(prepared, domain))
    {
        Ok(prepared) => Ok(VerifiedMinimalFallback {
            prepared,
            cause: ManuallyDrop::into_inner(cause),
        }),
        Err(error) => {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            Err(StoreActivationContainment {
                cause: map_raw_prepare_error(error),
            })
        }
    }
}

const fn map_raw_prepare_error(
    error: RetainedPublicationPrepareError,
) -> FinalizationContainmentError {
    match error {
        RetainedPublicationPrepareError::FinalizationContainment(cause) => cause,
    }
}

fn prepare_operation_minimal_direct<O, PP, U, L>(
    mut operation: RetainedStoreOperation<O, PP, U, L>,
    failure_code: PreparationFailureCode,
) -> Result<PreparedStoreFallback, StorePrepareError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation.raw.poll_terminal_control();
    let RetainedStoreOperation {
        raw,
        domain,
        lineage,
    } = operation;
    if drop_scoped_value(lineage).is_err() {
        return Err(StorePrepareError::FinalizationContainment(
            raw.contain_activation_fault(),
        ));
    }
    raw.prepare_minimal_fallback::<super::StoredBlock, super::StoredBlock>(failure_code)
        .map(|raw| classify_prepared_fallback(raw, domain))
        .map_err(StorePrepareError::from_raw)
}

/// Scoped activation disposition. No raw assembly is present in any outcome.
#[must_use = "every branch retains its exact linear session owner"]
pub enum CanonicalActivationOutcome<
    O,
    PP,
    U,
    T,
    E,
    CanonicalKind,
    CanonicalTag,
    L = FreshStoreLineage,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Activated(ActivatedCanonicalStoreSession<O, PP, U, T, CanonicalKind, CanonicalTag, L>),
    Rejected(RejectedCanonicalStoreSession<O, PP, U, E>),
    NeverActivated(NeverActivatedStoreSession<O, PP, U, E, L>),
    Fallback(VerifiedMinimalFallback<StoreRuntimeError>),
}

/// Fail-closed activation result retaining the quarantined account snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreActivationContainment {
    cause: FinalizationContainmentError,
}
