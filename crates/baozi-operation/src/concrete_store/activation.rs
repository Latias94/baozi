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
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    ByteBlock, BytesPlan, BytesWriter, CanonicalBytesRef, CanonicalInheritanceSource,
    CanonicalRecordRef, CanonicalRecordRowWriter, CanonicalRecordWriter, CanonicalSequenceRef,
    CanonicalStoreRef, CanonicalUtf8Ref, FreshStoreLineage, OutcomeEvidenceBytesRef,
    OutcomeEvidenceRecordRef, OutcomeEvidenceRecordRowWriter, OutcomeEvidenceRecordView,
    OutcomeEvidenceRecordWriter, OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef,
    OutcomeEvidenceUtf8Ref, PreparedStoreFailureOutcome, PreparedStoreFallback,
    PreparedStoreSuccessOutcome, PublishedCanonicalStore, RawStoreBorrowedWorkPermit,
    RawStoreFailureAssembly, RecordBlock, RecordPlan, RetainedAllocationPlan,
    RetainedBuildFailureCode, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStoreControlCheckpoint, RetainedStoreFailureTerminalWorkPlan,
    RetainedStoreFailureTerminalWorkRequirement, RetainedStoreOperation, RetainedStoreProducerHost,
    RetainedStorePublicationAssembly, RetainedStorePublicationPlan, RetainedStoreReservation,
    RetainedStoreWorkControl, RetainedStoreWriterPlan, RetainedStoreWriterRole, ScalarSequence,
    SequencePlan, SequenceWriter, ShareableStoreKind, StoreBuildCallbackFailure, StoreBuildError,
    StoreControlledError, StoreControlledSuccessValidation, StoreDimensionKey,
    StoreEvidenceValidationSession, StoreFailureAssembly, StoreFailureValidation,
    StoreFallbackOnly, StoreKeyDomain, StorePhaseKey, StorePlanError, StorePrepareError,
    StoreRuntimeError, StoreSuccessFreezeContribution, StoreSuccessFreezePermit,
    StoreSuccessFreezePermitIssue, StoreValidationSession, StoreWorkError, StoredBlock,
    StoredScalar, Utf8Block, Utf8Plan, Utf8Writer, ValidatedStoreFailure, ValidatedStoreSuccess,
    classify_prepared_fallback, map_publication_error, map_writer_error,
};
use crate::control::{ControlCause, ProgressPhase, ProgressUnit, discard_untrusted_panic_payload};
use crate::coordinator::FinalProgress;
use crate::ledger::STARTUP_ACCOUNT_SHARE_STEP_COUNT;
use crate::owner::OperationOwner;
use crate::retained::{
    CanonicalRetainedRewriteMap, FinalizationContainmentError, PreparationFailureClass,
    PreparationFailureCode, RetainedActivationOpenDisposition, RetainedFailureWorkConsumeError,
    RetainedFailureWorkControl, RetainedFailureWorkFinishError, RetainedFailureWorkPermit,
    RetainedFailureWorkStartError, RetainedHandleSharePlan, RetainedPlanningBaseline,
    RetainedPlanningContainmentAnchor, RetainedPublicationPrepareError, RetainedWorkConsumeError,
    RetainedWorkControl, RetainedWorkFinishError, RetainedWorkPermit, RetainedWorkStartError,
};

mod canonical;
mod canonical_validation;
mod failure_evidence;
mod fallback;
mod preflight;
mod work_batch;

pub use canonical_validation::{
    CanonicalValidationCheckpoint, CanonicalValidationMember, CanonicalValidationPlanning,
    CanonicalValidationPreparationError, CanonicalValidationRecordRow, CanonicalValidationSession,
    OutcomeEvidenceValidationMember, OutcomeEvidenceValidationRecordRow,
};

const ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE: u32 = 0x4143_5047;
const CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE: u32 = 0x4356_4346;
const FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE: u32 = 0x4657_4545;
const FAILURE_EVIDENCE_BUILD_CHUNK_UNITS: usize = 256;

static NEXT_CANONICAL_ACTIVATION_REQUEST_IDENTITY: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopedTransitionPanicPoint {
    TryOpenActivation,
    FinishRejectedSession,
}

#[cfg(test)]
thread_local! {
    static SCOPED_TRANSITION_PANIC: std::cell::Cell<Option<ScopedTransitionPanicPoint>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn test_panic_scoped_transition_at(point: ScopedTransitionPanicPoint) {
    SCOPED_TRANSITION_PANIC.with(|injected| injected.set(Some(point)));
}

#[cfg(test)]
fn test_scoped_transition_panic_is_armed() -> bool {
    SCOPED_TRANSITION_PANIC.with(|injected| injected.get().is_some())
}

#[cfg(test)]
fn inject_scoped_transition_panic(point: ScopedTransitionPanicPoint) {
    let panic = SCOPED_TRANSITION_PANIC.with(|injected| {
        if injected.get() == Some(point) {
            injected.set(None);
            true
        } else {
            false
        }
    });
    if panic {
        panic!("injected owner-consuming scoped activation transition panic");
    }
}

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
/// A row is one atomic semantic batch. Complete rows share the same bounded completion prefix as
/// sequence and byte writers; a row's fields are never split across a checkpoint. The pending
/// prefix is flushed when a checkpoint, explicit boundary, rejection, unwind, or abandonment is
/// observed.
pub struct CanonicalRecordBuildRow<'row, 'storage> {
    writer: &'row mut CanonicalRecordRowWriter<'storage>,
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

#[allow(
    clippy::too_many_arguments,
    reason = "the private activation helper keeps exact source lineage, one admitted permit, and one writer factory coupled"
)]
fn try_rewrite_activation_canonical_writer<O, PP, U, SourceRootKind, SourceRootTag, W, Kind, Tag>(
    assembly: &mut RetainedStorePublicationAssembly<O, PP, U, CanonicalInheritanceSource>,
    source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
    source_ref: CanonicalStoreRef<Kind, Tag>,
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
    let result = assembly.raw.try_retain_rewritten_canonical_controlled(
        source.share_source().aggregate(),
        source_ref.raw,
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

fn copy_mapped_record_prefix<O, PP, U>(
    control: &mut CanonicalBuildControl<'_, '_, '_, O, PP, U, CanonicalRecordWriter>,
    source: &super::RecordStorage,
    mapping: &CanonicalRetainedRewriteMap<'_, '_, StoredBlock>,
) -> Result<(), StoreControlledError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    for source_row in source.cells.chunks(source.fields) {
        control.write_row(|row| {
            for source_cell in source_row.iter().copied() {
                let mapped = super::mapped_canonical_record_cell(source_cell, mapping)
                    .ok_or_else(StoreBuildError::capacity_envelope_violated)?;
                row.write_mapped_cell(mapped)?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "the private mapped-copy helper binds both exact source and destination record plans to one linear writer"
)]
fn try_copy_activation_canonical_records<O, PP, U, SourceRootKind, SourceRootTag, Tag>(
    assembly: &mut RetainedStorePublicationAssembly<O, PP, U, CanonicalInheritanceSource>,
    source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
    source_ref: CanonicalRecordRef<Tag>,
    source_plan: RecordPlan,
    destination_plan: RecordPlan,
    allocation: RetainedAllocationPlan,
    work_dimension: StoreDimensionKey,
    work: NonZeroU64,
    tag: Tag,
    append: impl for<'writer, 'permit, 'operation> FnOnce(
        &mut CanonicalBuildControl<'writer, 'permit, 'operation, O, PP, U, CanonicalRecordWriter>,
    ) -> Result<(), StoreControlledError>,
) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: Copy + Send + Sync + 'static,
{
    assembly.ensure_writable()?;
    assembly.validate_dimension(work_dimension)?;
    let mut callback_failure = None;
    let result = assembly.raw.try_rewrite_canonical_controlled(
        source.share_source().aggregate(),
        source_ref.raw,
        work_dimension,
        allocation,
        |emplacer, source_block, mapping, raw| {
            let mut raw_control = RetainedStoreWorkControl { raw, sticky: None };
            let source_records = match super::checked_source_record(source_block, source_plan) {
                Some(source_records) => source_records,
                None => {
                    return finish_activation_writer_callback(
                        raw_control,
                        Err(StoreBuildError::capacity_envelope_violated().into()),
                        &mut callback_failure,
                    );
                }
            };
            let mut permit = match try_begin_canonical_work(&mut raw_control, work) {
                Ok(permit) => permit,
                Err(cause) => {
                    callback_failure = Some(StoreBuildCallbackFailure::VerifiedWork(cause));
                    return Err(RetainedBuildFailureCode::new(super::BUILD_CODE_DOMAIN));
                }
            };
            let mut writer = match CanonicalRecordWriter::new(destination_plan) {
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
            let callback = copy_mapped_record_prefix(&mut control, source_records, &mapping)
                .and_then(|()| append(&mut control));
            let callback = control.finish(callback).and_then(|()| {
                CanonicalRecordWriter::finish(writer).map_err(StoreControlledError::from)
            });
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

fn perform_canonical_record_field(
    writer: &mut CanonicalRecordRowWriter<'_>,
    sticky: &mut Option<StoreControlledError>,
    action: impl FnOnce(&mut CanonicalRecordRowWriter<'_>) -> Result<(), StoreBuildError>,
) -> Result<(), StoreControlledError> {
    if let Some(cause) = sticky.clone() {
        return Err(cause);
    }
    let result = action(writer).map_err(StoreControlledError::from);
    preserve_canonical_controlled(sticky, result)
}

impl CanonicalRecordBuildRow<'_, '_> {
    fn write_mapped_cell(&mut self, value: super::RecordCell) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(self.writer, self.sticky, |writer| writer.push_cell(value))
    }

    pub fn write_scalar<T: StoredScalar>(&mut self, value: T) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(self.writer, self.sticky, |writer| {
            writer.write_scalar(value)
        })
    }

    pub fn write_sequence_ref<T: StoredScalar, Tag>(
        &mut self,
        value: CanonicalSequenceRef<T, Tag>,
    ) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(self.writer, self.sticky, |writer| {
            writer.write_sequence_ref(value)
        })
    }

    pub fn write_bytes_ref<Tag>(
        &mut self,
        value: CanonicalBytesRef<Tag>,
    ) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(self.writer, self.sticky, |writer| {
            writer.write_bytes_ref(value)
        })
    }

    pub fn write_utf8_ref<Tag>(
        &mut self,
        value: CanonicalUtf8Ref<Tag>,
    ) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(self.writer, self.sticky, |writer| {
            writer.write_utf8_ref(value)
        })
    }

    pub fn write_record_ref<Tag>(
        &mut self,
        value: CanonicalRecordRef<Tag>,
    ) -> Result<(), StoreControlledError> {
        perform_canonical_record_field(self.writer, self.sticky, |writer| {
            writer.write_record_ref(value)
        })
    }
}

impl<O, PP, U> CanonicalBuildControl<'_, '_, '_, O, PP, U, CanonicalRecordWriter>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Writes one exact-width row and accounts its fields only after the row commits physically.
    pub fn write_row(
        &mut self,
        write: impl FnOnce(&mut CanonicalRecordBuildRow<'_, '_>) -> Result<(), StoreControlledError>,
    ) -> Result<(), StoreControlledError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        let row_fields = self.writer.core.storage.fields;
        let row_work =
            NonZeroU64::new(u64::try_from(row_fields).expect("record field count fits in u64"))
                .expect("record rows always contain at least one field");
        self.prepare_action(row_work)?;
        let mut callback_error = None;
        let result = self.writer.write_row(|writer| {
            let mut row = CanonicalRecordBuildRow {
                writer,
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
        if let Some(cause) = self.sticky.clone() {
            self.resolve_partial();
            return Err(cause);
        }
        let permit = self
            .permit
            .as_mut()
            .expect("a successful canonical record row retains its work permit");
        record_canonical_completed(permit, &mut self.work_batch, &mut self.sticky, row_work)?;
        // Keep complete rows in the shared prefix. `prepare_action` polls and flushes before the
        // next row when this row reaches the bounded checkpoint; `finish` handles the final
        // partial prefix.
        Ok(())
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

struct ScopedDormantActivation<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: Option<RetainedStoreOperation<O, PP, U, L>>,
    containment_anchor: RetainedPlanningContainmentAnchor,
    armed: bool,
}

impl<O, PP, U, L> ScopedDormantActivation<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn operation(&self) -> &RetainedStoreOperation<O, PP, U, L> {
        self.operation
            .as_ref()
            .expect("an armed dormant activation owns exactly one retained-store operation")
    }

    fn operation_mut(&mut self) -> &mut RetainedStoreOperation<O, PP, U, L> {
        self.operation
            .as_mut()
            .expect("an armed dormant activation owns exactly one retained-store operation")
    }

    fn into_operation(
        mut self,
    ) -> Result<RetainedStoreOperation<O, PP, U, L>, StoreActivationContainment> {
        let Some(operation) = self.operation.take() else {
            let containment_anchor = self.containment_anchor.share();
            self.armed = false;
            return Err(StoreActivationContainment {
                cause: containment_anchor.contain(),
            });
        };
        self.armed = false;
        Ok(operation)
    }
}

impl<O, PP, U, L> Drop for ScopedDormantActivation<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(operation) = self.operation.take() {
            let _ = contain_operation(operation);
        } else {
            let _ = self.containment_anchor.contain();
        }
    }
}

/// Session-lifetime owner of the sole retained operation and dormant activation authority.
///
/// The owner binds stable store roles and failure-evidence capacity before activation. Producer work
/// remains available only through its narrow borrowed capabilities. Exact canonical writer capacity
/// stays inside the single-use success request emitted by a sealed construction plan.
#[must_use = "the session outcome owner must resolve or activate its retained operation"]
pub struct RetainedStoreSessionOutcomeOwner<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    dormant: ScopedDormantActivation<O, PP, U, L>,
    policy: CanonicalActivationSessionPolicy,
    scoped_failure: BoundScopedFailureRequest,
    failure_work: FailureEvidenceWorkBudget,
}

fn finish_rejected_session<O, PP, U, E, L>(
    assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    cause: StoreCanonicalRejection<E>,
    failure_code: PreparationFailureCode,
) -> Result<RejectedCanonicalStoreSession<O, PP, U, E>, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[cfg(test)]
    inject_scoped_transition_panic(ScopedTransitionPanicPoint::FinishRejectedSession);

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
            Ok(RejectedCanonicalStoreSession {
                owner: UnrootedFailureEvidence {
                    raw,
                    domain,
                    failure_code,
                    cause: ManuallyDrop::into_inner(cause),
                    failure_work,
                    outcome_evidence_destination_dimension,
                },
            })
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

fn preserve_runtime_fault_then_drop_result<R>(
    first_fault: &mut Option<ActivationFirstFault>,
    cause: StoreRuntimeError,
    result: R,
) -> Result<R, StoreRuntimeError> {
    let failure = preserve_first_fault(first_fault, Err(cause));
    if drop_scoped_value(result).is_err() {
        note_later_contract::<()>(
            first_fault,
            Err(StoreRuntimeError::ConstructionInvariantUnverified),
        );
    }
    failure
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
        | StoreRuntimeError::ObservedWorkOutstanding
        | StoreRuntimeError::ObservedWorkExceedsAdmitted
        | StoreRuntimeError::ObservedWorkIncomplete
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
    // The account may already be quarantined by the faulting physical action. Keeping the
    // operation here delays final ownership-graph containment until caller-owned inputs drop.
    DeferredContainment {
        operation: RetainedStoreOperation<O, PP, U, L>,
    },
    Recoverable {
        cause: StoreRuntimeError,
        failure_code: PreparationFailureCode,
        fallback_only: bool,
        // Keep the recoverable operation last so its physical baseline observes any unwind.
        operation: RetainedStoreOperation<O, PP, U, L>,
    },
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
    let pre_open = catch_unwind(AssertUnwindSafe(|| {
        #[cfg(test)]
        inject_scoped_transition_panic(ScopedTransitionPanicPoint::TryOpenActivation);
        operation.validate_publication_plan_keys(&plan)
    }));
    match pre_open {
        Err(payload) => {
            discard_untrusted_panic_payload(payload);
            return Err(ActivationOpenFailure::DeferredContainment { operation });
        }
        Ok(Err(_)) => {
            return Err(ActivationOpenFailure::DeferredContainment { operation });
        }
        Ok(Ok(())) => {}
    }
    if plan.is_success_capable()
        && let Some(failure_code) = operation.activation_terminal_cause()
    {
        return Err(ActivationOpenFailure::Recoverable {
            operation,
            cause: StoreRuntimeError::SuccessOutcomeRejected,
            failure_code,
            fallback_only: false,
        });
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
            let (raw, fallback_only) = match error.into_activation_disposition() {
                RetainedActivationOpenDisposition::Retry(raw) => (raw, false),
                RetainedActivationOpenDisposition::FallbackOnly(raw) => (raw, true),
                RetainedActivationOpenDisposition::FailClosed(raw) => {
                    return Err(ActivationOpenFailure::DeferredContainment {
                        operation: RetainedStoreOperation {
                            raw,
                            domain,
                            lineage,
                        },
                    });
                }
            };
            let mut operation = RetainedStoreOperation {
                raw,
                domain,
                lineage,
            };
            match classification {
                ActivationFaultClassification::Contract => {
                    Err(ActivationOpenFailure::DeferredContainment { operation })
                }
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
    }
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
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
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

/// One scoped canonical planning transaction owned by the import session.
///
/// The scope exposes only role-bound control plus the terminal `seal_activation` and `reject`
/// transitions. No ledger key, operation owner, publication authority, or generation identity can
/// escape this interface.
pub struct CanonicalPlanning<
    'scope,
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
    driver: &'scope mut ScopedActivationDriver<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    receipt_brand: &'scope ScopedReceiptBrand,
}

/// Linear role-bound semantic-work session for authority-free semantic preflight.
///
/// Each `run_unit` admits its own attempted unit immediately before the action and advances
/// consumed work only after that action returns normally. Scratch storage remains metered by the
/// same operation without exposing a raw phase, dimension, ledger, or operation owner.
pub struct CanonicalSemanticWorkSession<'borrow, 'scope, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: &'borrow mut RetainedStoreOperation<O, PP, U, L>,
    first_fault: &'borrow mut Option<ActivationFirstFault>,
    phase: StorePhaseKey,
    work_dimension: StoreDimensionKey,
    scratch_dimension: StoreDimensionKey,
    state: &'borrow mut ScopedSemanticState,
    receipt_brand: &'scope ScopedReceiptBrand,
    work_batch: work_batch::CompletionPrefixBatch,
    completed_any: bool,
    resolved: bool,
    scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

/// Linear proof that one scoped semantic pass completed at least one metered action.
///
/// The private brand prevents completion from one planning transaction from sealing another. The
/// token carries no Scene payload, retained owner, ledger key, or publication authority.
#[must_use = "semantic completion must seal its matching activation exactly once"]
pub struct CanonicalSemanticCompletion<'scope> {
    receipt_brand: &'scope ScopedReceiptBrand,
    invariant: PhantomData<fn(&'scope mut ()) -> &'scope mut ()>,
}

/// Transaction-scoped authority that seals one complete activation description.
///
/// The constructor is private. A description receives this value only while the matching planning
/// transaction consumes it. The receipt brand prevents a sealed value from crossing planning
/// transactions; the producer remains responsible for deriving the request, success facts, and
/// execution payload passed to [`Self::seal`] from one opaque construction description.
#[must_use = "the plan sealer must return one sealed activation plan"]
pub struct CanonicalActivationPlanSealer<'scope> {
    receipt_brand: &'scope ScopedReceiptBrand,
    invariant: PhantomData<fn(&'scope mut ()) -> &'scope mut ()>,
}

impl<'scope> CanonicalActivationPlanSealer<'scope> {
    /// Seals the request, success facts, and execution payload owned by one construction
    /// description.
    pub fn seal<P>(
        self,
        request: CanonicalActivationRequest,
        requirements: CanonicalSuccessRequirements,
        plan: P,
    ) -> CanonicalSealedActivationPlan<'scope, P> {
        CanonicalSealedActivationPlan {
            request,
            requirements,
            plan,
            receipt_brand: self.receipt_brand,
        }
    }
}

/// Single-use construction description that owns both success facts and execution data.
///
/// This is a trusted migration seam for deep canonical construction modules. Implementations must
/// move success facts and execution data out of the same opaque description without running
/// fallible user code. [`CanonicalPlanning::seal_activation`] proves the transaction receipt; it
/// intentionally does not attempt to validate an arbitrary third-party implementation of this
/// producer contract.
pub trait CanonicalActivationDescription {
    type Execution;

    fn seal_activation<'scope>(
        self,
        sealer: CanonicalActivationPlanSealer<'scope>,
    ) -> CanonicalSealedActivationPlan<'scope, Self::Execution>;
}

/// One opaque activation plan whose success facts and execution payload travel together.
///
/// Operation mints this value only while consuming one complete construction description. The
/// transaction brand prevents a sealed value from crossing planning transactions, while private
/// fields prevent downstream code from splitting and recombining a producer's opaque plan.
#[must_use = "the sealed activation plan must seal or reject exactly once"]
pub struct CanonicalSealedActivationPlan<'scope, P> {
    request: CanonicalActivationRequest,
    requirements: CanonicalSuccessRequirements,
    plan: P,
    receipt_brand: &'scope ScopedReceiptBrand,
}

impl<'scope, P> CanonicalSealedActivationPlan<'scope, P> {
    fn into_parts(
        self,
    ) -> (
        CanonicalActivationRequest,
        CanonicalSuccessRequirements,
        P,
        &'scope ScopedReceiptBrand,
    ) {
        (
            self.request,
            self.requirements,
            self.plan,
            self.receipt_brand,
        )
    }
}

/// Permit for one scoped canonical activation with its execution plan already bound.
///
/// Materialization receives only a shared plan view. The transaction drops the plan in a separate
/// panic boundary before final containment. Operation proves only the semantic completion receipt,
/// transaction brand, physical shape, ledger roles, and retained publication authority; it never
/// interprets the core plan.
#[must_use = "the activation permit must activate exactly once"]
pub struct CanonicalActivationPermit<
    'scope,
    O,
    PP,
    U,
    T,
    E,
    CanonicalKind,
    CanonicalTag,
    L = FreshStoreLineage,
    P = (),
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[allow(
        clippy::type_complexity,
        reason = "the U4e permit borrows the complete inline linear driver without allocation"
    )]
    driver:
        Option<&'scope mut ScopedActivationDriver<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>>,
    request: Option<BoundScopedSuccessRequest>,
    plan: Option<P>,
    receipt_brand: &'scope ScopedReceiptBrand,
}

/// Linear resolver for one activation that could not be sealed.
///
/// The resolver owns the exact core plan paired with the rejected success envelope. It releases
/// that plan and every scoped reservation before verifying the restored planning baseline. A
/// recoverable fault may then install `NeverActivated`; any drop, contract, or ownership failure is
/// contained fail closed.
#[must_use = "resolve the unsealed activation exactly once"]
pub struct CanonicalActivationSealFailure<
    'scope,
    O,
    PP,
    U,
    T,
    E,
    CanonicalKind,
    CanonicalTag,
    L = FreshStoreLineage,
    P = (),
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[allow(
        clippy::type_complexity,
        reason = "the U4e resolver borrows the complete inline linear driver without allocation"
    )]
    driver:
        Option<&'scope mut ScopedActivationDriver<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>>,
    plan: Option<P>,
    receipt_brand: &'scope ScopedReceiptBrand,
}

/// Result of sealing one activation plan with its payload already bound.
#[must_use = "consume the activation permit or resolve the unsealed activation"]
#[allow(
    clippy::large_enum_variant,
    reason = "the linear activation permit remains inline so no unmetered allocation is introduced"
)]
pub enum CanonicalActivationSeal<
    'scope,
    O,
    PP,
    U,
    T,
    E,
    CanonicalKind,
    CanonicalTag,
    L = FreshStoreLineage,
    P = (),
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Sealed(CanonicalActivationPermit<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P>),
    Unsealed(
        CanonicalActivationSealFailure<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P>,
    ),
}

/// Zero-owner receipt proving that the matching scoped driver installed one terminal state.
///
/// The invariant lifetime and private brand make cross-transaction substitution unrepresentable.
#[must_use = "return the scoped disposition to its owning transaction"]
pub struct CanonicalScopedDisposition<'scope> {
    brand: &'scope ScopedReceiptBrand,
    invariant: PhantomData<fn(&'scope mut ()) -> &'scope mut ()>,
}

struct ScopedReceiptBrand {
    _private: u8,
}

#[derive(Clone, Copy)]
enum ScopedSemanticState {
    NotStarted,
    Active,
    Complete,
    Rejected,
}

enum ScopedPendingActivation<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Activated(ActivatedCanonicalStoreSession<O, PP, U, T, CanonicalKind, CanonicalTag, L>),
    Rejected(RejectedCanonicalStoreSession<O, PP, U, E>),
    NeverActivated(NeverActivatedStoreSession<O, PP, U, E, L>),
}

#[allow(
    clippy::large_enum_variant,
    reason = "the temporary U4e state keeps every linear terminal owner inline for fail-closed containment without allocation and is deleted at core cutover"
)]
enum ScopedActivationState<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Dormant(ScopedDormantActivation<O, PP, U, L>),
    Transitioning,
    Pending(ScopedPendingActivation<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>),
    Contained(StoreActivationContainment),
    Finished,
}

struct ScopedActivationDriver<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    state: ScopedActivationState<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    containment_anchor: RetainedPlanningContainmentAnchor,
    roles: Option<BoundActivationRoles>,
    failure: Option<BoundScopedFailureRequest>,
    planning_baseline: RetainedPlanningBaseline,
    failure_work: FailureEvidenceWorkBudget,
    first_fault: Option<ActivationFirstFault>,
    semantic_state: ScopedSemanticState,
}

/// Operation-neutral retained table shape used by one canonical activation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalTableShape {
    total_blocks: usize,
    max_new_blocks: usize,
}

/// Opaque identity shared by one producer's failure and success activation requirements.
///
/// Consumers may carry this value but cannot mint it. Trusted canonical producers create one
/// identity per construction description through the producer-only facade, then bind the same
/// identity to the failure envelope and the sealed success facts. The activation driver rejects
/// any cross-description pairing before canonical storage can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalActivationRequestIdentity(u64);

/// Linear consumer token for one producer-owned canonical activation request.
///
/// The token exposes no identity value and is neither cloneable nor copyable. A trusted producer
/// retains the paired identity inside its sealed success description, while the operation-owning
/// consumer can only transfer this token into the session outcome owner.
#[must_use = "the activation request must bind exactly one session outcome owner"]
pub struct CanonicalActivationRequest {
    identity: CanonicalActivationRequestIdentity,
}

/// Mints one producer-owned activation request identity.
///
/// This function is exported only through the trusted canonical producer facade. Operation
/// consumers receive only the opaque identity already embedded in requirements.
fn mint_canonical_activation_request_identity() -> Option<CanonicalActivationRequestIdentity> {
    NEXT_CANONICAL_ACTIVATION_REQUEST_IDENTITY
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |observed| {
            observed.checked_add(1)
        })
        .ok()
        .map(CanonicalActivationRequestIdentity)
}

/// Mints the paired producer identity and linear consumer request for one construction.
pub fn mint_canonical_activation_request() -> Option<(
    CanonicalActivationRequestIdentity,
    CanonicalActivationRequest,
)> {
    mint_canonical_activation_request_identity()
        .map(|identity| (identity, CanonicalActivationRequest { identity }))
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

fn retained_share_actions_work_requirement(actions: usize) -> Result<u64, StorePlanError> {
    let work_per_action = RetainedHandleSharePlan::new((), ())
        .charges()
        .into_iter()
        .try_fold(0_u64, |total, (_, amount)| {
            total
                .checked_add(amount)
                .ok_or(StorePlanError::SizeOverflow)
        })?;
    u64::try_from(actions)
        .ok()
        .and_then(|actions| actions.checked_mul(work_per_action))
        .ok_or(StorePlanError::SizeOverflow)
}

fn canonical_pre_activation_structural_work_requirement() -> Result<u64, StorePlanError> {
    retained_share_actions_work_requirement(STARTUP_ACCOUNT_SHARE_STEP_COUNT)
}

fn canonical_structural_work_requirement(
    canonical: CanonicalTableShape,
    outcome_evidence: CanonicalTableShape,
) -> Result<u64, StorePlanError> {
    let actions = STARTUP_ACCOUNT_SHARE_STEP_COUNT
        .checked_add(1)
        .and_then(|actions| actions.checked_add(2))
        .and_then(|actions| actions.checked_add(canonical.total_blocks))
        .and_then(|actions| actions.checked_add(outcome_evidence.total_blocks))
        .ok_or(StorePlanError::SizeOverflow)?;
    retained_share_actions_work_requirement(actions)
}

/// Failure-only retained shape bound before a scoped producer receives authoring authority.
#[must_use = "failure requirements must be bound to one scoped session outcome owner"]
pub struct CanonicalFailureRequirements {
    outcome_evidence: CanonicalTableShape,
    request_identity: Option<CanonicalActivationRequestIdentity>,
}

impl CanonicalFailureRequirements {
    pub const fn new(outcome_evidence: CanonicalTableShape) -> Self {
        Self {
            outcome_evidence,
            request_identity: None,
        }
    }

    /// Binds this failure envelope to one opaque producer construction.
    pub const fn with_request_identity(
        mut self,
        request_identity: CanonicalActivationRequestIdentity,
    ) -> Self {
        self.request_identity = Some(request_identity);
        self
    }

    /// Returns the exact operation-owned work complete before scoped authoring can begin.
    pub fn pre_activation_structural_work_requirement(&self) -> Result<u64, StorePlanError> {
        canonical_pre_activation_structural_work_requirement()
    }
}

/// Success-only activation facts derived after complete semantic preflight.
///
/// Failure evidence remains session-bound. The operation-issued completion token proves that the
/// matching transaction resolved a nonempty action-derived semantic pass. Canonical writer
/// capacity, table shape, materialization shape, and validation work are moved into one opaque plan
/// together with the execution payload before operation may seal activation.
#[must_use = "success requirements must be bound to their exact execution plan"]
pub struct CanonicalSuccessRequirements {
    canonical_writer_limits: RetainedStoreBatchLimits,
    canonical: CanonicalTableShape,
    expected_materialized_blocks: usize,
    validation_work: NonZeroU64,
    semantic_bytes: u64,
    request_identity: Option<CanonicalActivationRequestIdentity>,
}

impl CanonicalSuccessRequirements {
    pub fn new(
        canonical: CanonicalTableShape,
        canonical_commitment_slots: usize,
        expected_materialized_blocks: usize,
        validation_work: NonZeroU64,
    ) -> Result<Self, StorePlanError> {
        if expected_materialized_blocks == 0
            || expected_materialized_blocks > canonical.total_blocks
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let canonical_writer_limits = RetainedStoreBatchLimits::new(
            canonical
                .max_new_blocks
                .checked_add(1)
                .ok_or(StorePlanError::InvalidBlockTable)?,
            canonical_commitment_slots,
        )?;
        Ok(Self {
            canonical_writer_limits,
            canonical,
            expected_materialized_blocks,
            validation_work,
            semantic_bytes: 0,
            request_identity: None,
        })
    }

    /// Binds these success facts to the same opaque producer construction as its failure envelope.
    pub const fn with_request_identity(
        mut self,
        request_identity: CanonicalActivationRequestIdentity,
    ) -> Self {
        self.request_identity = Some(request_identity);
        self
    }

    /// Binds one exact semantic-byte observation derived from the same sealed construction plan.
    pub const fn with_semantic_bytes(mut self, semantic_bytes: u64) -> Self {
        self.semantic_bytes = semantic_bytes;
        self
    }

    /// Returns the exact structural work for this success shape and its session-bound failure shape.
    pub fn structural_work_requirement(
        &self,
        failure: &CanonicalFailureRequirements,
    ) -> Result<u64, StorePlanError> {
        if failure.request_identity.is_some() && self.request_identity != failure.request_identity {
            return Err(StorePlanError::InvalidBlockTable);
        }
        canonical_structural_work_requirement(self.canonical, failure.outcome_evidence)
    }
}

struct BoundScopedFailureRequest {
    outcome_evidence_plan: RetainedStoreBlockTablePlan,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    request_identity: Option<CanonicalActivationRequestIdentity>,
}

struct BoundScopedSuccessRequest {
    canonical_plan: RetainedStoreBlockTablePlan,
    materialization_work_dimension: StoreDimensionKey,
    materialization_scratch_dimension: StoreDimensionKey,
    validation_phase: StorePhaseKey,
    validation_work_dimension: StoreDimensionKey,
    validation_scratch_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    validation_work: NonZeroU64,
    expected_materialized_blocks: usize,
    semantic_bytes: u64,
    semantic_bytes_dimension: Option<StoreDimensionKey>,
}

/// Host-owned dimensions for semantic planning and retained materialization work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalMaterializationRole {
    work_dimension: StoreDimensionKey,
    scratch_dimension: StoreDimensionKey,
}

impl CanonicalMaterializationRole {
    pub const fn new(
        work_dimension: StoreDimensionKey,
        scratch_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            work_dimension,
            scratch_dimension,
        }
    }
}

/// Host-owned phase and dimensions for central validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalValidationRole {
    phase: StorePhaseKey,
    work_dimension: StoreDimensionKey,
    scratch_dimension: StoreDimensionKey,
}

/// Owner-defined progress vocabulary emitted exactly when central validation begins.
///
/// The sealed success requirements remain the sole authority for the total validation work; the
/// owner supplies only the phase and unit labels understood by its observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalValidationProgress<PP, U> {
    phase: PP,
    unit: U,
}

impl<PP: Copy, U: Copy> CanonicalValidationProgress<PP, U> {
    pub const fn new(phase: PP, unit: U) -> Self {
        Self { phase, unit }
    }

    const fn phase(self) -> PP {
        self.phase
    }

    const fn unit(self) -> U {
        self.unit
    }
}

impl CanonicalValidationRole {
    pub const fn new(
        phase: StorePhaseKey,
        work_dimension: StoreDimensionKey,
        scratch_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            phase,
            work_dimension,
            scratch_dimension,
        }
    }
}

/// Session-stable host roles retained across producer work and canonical activation.
///
/// Core and format code never receive this value. Writer placement and failure-evidence capacity are
/// session-bound, while canonical success capacity remains absent until a sealed activation request
/// is consumed.
#[must_use = "the activation session policy must be bound to one retained-store operation"]
pub struct CanonicalActivationSessionPolicy {
    canonical_writer: RetainedStoreWriterRole,
    outcome_evidence_writer: RetainedStoreWriterRole,
    materialization_work_dimension: StoreDimensionKey,
    materialization_scratch_dimension: StoreDimensionKey,
    validation_phase: StorePhaseKey,
    validation_work_dimension: StoreDimensionKey,
    validation_scratch_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    semantic_bytes_dimension: Option<StoreDimensionKey>,
}

impl CanonicalActivationSessionPolicy {
    pub const fn new(
        canonical_writer: RetainedStoreWriterRole,
        outcome_evidence_writer: RetainedStoreWriterRole,
        materialization: CanonicalMaterializationRole,
        validation: CanonicalValidationRole,
        canonical_destination_dimension: StoreDimensionKey,
        outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            canonical_writer,
            outcome_evidence_writer,
            materialization_work_dimension: materialization.work_dimension,
            materialization_scratch_dimension: materialization.scratch_dimension,
            validation_phase: validation.phase,
            validation_work_dimension: validation.work_dimension,
            validation_scratch_dimension: validation.scratch_dimension,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
            semantic_bytes_dimension: None,
        }
    }

    /// Projects exact sealed semantic bytes into one host-owned accounting dimension.
    pub const fn with_semantic_bytes_dimension(mut self, dimension: StoreDimensionKey) -> Self {
        self.semantic_bytes_dimension = Some(dimension);
        self
    }

    fn authoring_keys(&self) -> (StorePhaseKey, StoreDimensionKey, StoreDimensionKey) {
        let (phase, _) = self.canonical_writer.keys_for_publication_validation();
        (
            phase,
            self.materialization_work_dimension,
            self.materialization_scratch_dimension,
        )
    }

    fn bind_scoped_failure(
        &self,
        writer_limits: RetainedStoreBatchLimits,
        requirements: CanonicalFailureRequirements,
    ) -> Result<BoundScopedFailureRequest, StorePlanError> {
        Ok(BoundScopedFailureRequest {
            outcome_evidence_plan: requirements
                .outcome_evidence
                .bind(self.outcome_evidence_writer.bind(writer_limits))?,
            outcome_evidence_destination_dimension: self.outcome_evidence_destination_dimension,
            request_identity: requirements.request_identity,
        })
    }

    fn bind_scoped_success(
        &self,
        requirements: CanonicalSuccessRequirements,
    ) -> Result<BoundScopedSuccessRequest, StorePlanError> {
        Ok(BoundScopedSuccessRequest {
            canonical_plan: requirements.canonical.bind(
                self.canonical_writer
                    .bind(requirements.canonical_writer_limits),
            )?,
            materialization_work_dimension: self.materialization_work_dimension,
            materialization_scratch_dimension: self.materialization_scratch_dimension,
            validation_phase: self.validation_phase,
            validation_work_dimension: self.validation_work_dimension,
            validation_scratch_dimension: self.validation_scratch_dimension,
            canonical_destination_dimension: self.canonical_destination_dimension,
            validation_work: requirements.validation_work,
            expected_materialized_blocks: requirements.expected_materialized_blocks,
            semantic_bytes: requirements.semantic_bytes,
            semantic_bytes_dimension: self.semantic_bytes_dimension,
        })
    }
}

struct BoundActivationRoles {
    policy: CanonicalActivationSessionPolicy,
    expected_request_identity: Option<CanonicalActivationRequestIdentity>,
}

impl BoundActivationRoles {
    fn authoring_keys(&self) -> (StorePhaseKey, StoreDimensionKey, StoreDimensionKey) {
        self.policy.authoring_keys()
    }

    fn bind_scoped_success(
        &self,
        activation_request: CanonicalActivationRequest,
        requirements: CanonicalSuccessRequirements,
    ) -> Result<BoundScopedSuccessRequest, StorePlanError> {
        let request_identity = activation_request.identity;
        if requirements.request_identity != Some(request_identity)
            || self
                .expected_request_identity
                .is_some_and(|expected| expected != request_identity)
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        self.policy.bind_scoped_success(requirements)
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
    materialization_phase: StorePhaseKey,
    materialization_work_dimension: StoreDimensionKey,
    materialization_scratch_dimension: StoreDimensionKey,
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
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    output: T,
    failure_work: FailureEvidenceWorkBudget,
    validation_phase: StorePhaseKey,
    validation_work_dimension: StoreDimensionKey,
    validation_scratch_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    validation_work: NonZeroU64,
    // The physical owner must drop last so an untrusted output panic is observed fail-closed.
    assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
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
/// core-owned generation admission before either side can prepare a success.
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

/// Linear candidate whose final operation progress and exact destinations are frozen.
///
/// Payload and proof drop before the operation owner, so an unwind while dropping either
/// value is still contained by the last-dropping physical owner.
#[must_use = "the freeze permit must be rejected or bound to one scoped joint commit"]
pub struct CanonicalFreezePermit<
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
    output: T,
    proof: Proof,
    failure_work: FailureEvidenceWorkBudget,
    destinations: ZeroCommitDestinationFacts,
    // This owner must drop last so payload panics remain physically contained.
    store:
        StoreSuccessFreezePermit<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>,
}

/// Scoped operation contribution for one core-owned joint commit.
///
/// The invariant lifetime brand prevents this contribution from escaping its binder or being
/// paired with a seal from another invocation. The physical owner remains last for fail-closed
/// containment if the scoped aggregate is dropped before preparation.
#[must_use = "the scoped joint commit must be prepared with its matching seal"]
pub struct CanonicalJointCommit<
    'joint,
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
    _failure_work: FailureEvidenceWorkBudget,
    _destinations: ZeroCommitDestinationFacts,
    joint: PhantomData<fn(&'joint mut ()) -> &'joint mut ()>,
    // This owner must drop last so the complete success graph remains fail-closed.
    store: StoreSuccessFreezeContribution<
        O,
        PP,
        U,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
    >,
}

/// Single-use seal paired with one scoped joint-commit contribution.
///
/// This type is invariant and deliberately neither `Clone` nor `Copy`; only the matching scoped
/// contribution can consume it.
#[must_use = "the scoped seal must be consumed by its matching joint commit"]
pub struct CanonicalJointSeal<'joint> {
    joint: PhantomData<fn(&'joint mut ()) -> &'joint mut ()>,
}

/// Exact result of issuing the generation freeze permit.
#[must_use = "every freeze-permit branch retains the exact terminal owner"]
pub enum CanonicalFreezePermitIssue<
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
    Ready(
        CanonicalFreezePermit<
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
    Interrupted(
        RootedFailureEvidence<
            O,
            PP,
            U,
            CanonicalFreezeInterruption,
            EvidenceKind,
            EvidenceTag,
            FrozenFailureState,
        >,
    ),
    DestinationRejected(
        RootedFailureEvidence<
            O,
            PP,
            U,
            CanonicalFreezeInterruption,
            EvidenceKind,
            EvidenceTag,
            OrdinaryFailureState,
        >,
    ),
}

/// Stable typed classification for a final-progress or already-latched runtime interruption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalFreezeInterruption {
    class: CanonicalFreezeInterruptionClass,
    stable_code: u32,
    control_cause: Option<ControlCause>,
}

/// Trusted terminal class retained across generation admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalFreezeInterruptionClass {
    Control,
    ProductGraph,
    Ledger,
}

/// Exact destination facts derived from a validated physical closure.
///
/// These facts contain no applied or staged ledger commitment. U4e consumes them only after the
/// matching freeze permit and final eligibility are ready.
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
pub struct CentrallyRejectedCanonicalStoreSession<
    O,
    PP,
    U,
    E,
    EvidenceKind,
    EvidenceTag,
    State = OrdinaryFailureState,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    root: RootedFailureEvidence<
        O,
        PP,
        U,
        StoreCanonicalRejection<E>,
        EvidenceKind,
        EvidenceTag,
        State,
    >,
}

/// Rich-failure validation result for a centrally rejected canonical candidate.
pub type CanonicalRejectionFailureValidation<
    O,
    PP,
    U,
    E,
    EvidenceKind,
    EvidenceTag,
    State = OrdinaryFailureState,
> = RootedFailureValidation<O, PP, U, StoreCanonicalRejection<E>, EvidenceKind, EvidenceTag, State>;

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
    cause: Cause,
    domain: StoreKeyDomain,
    failure_code: PreparationFailureCode,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    // The physical owner must drop last so an untrusted cause panic is observed fail-closed.
    raw: RawStoreFailureAssembly<O, PP, U>,
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

/// Marker for failure evidence whose exact commitment and final progress are not frozen yet.
#[doc(hidden)]
pub struct OrdinaryFailureState {
    _private: (),
}

/// Marker for failure evidence frozen by the success-permit transition.
#[doc(hidden)]
pub struct FrozenFailureState {
    _private: (),
}

trait FailureLifecycleState {
    fn validate_commitment<O, PP, U, EvidenceKind, EvidenceTag>(
        validated: &mut ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>,
        outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit;
}

impl FailureLifecycleState for OrdinaryFailureState {
    fn validate_commitment<O, PP, U, EvidenceKind, EvidenceTag>(
        validated: &mut ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>,
        outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        if validated.outcome_evidence_commitment_len() != 0 {
            return Err(StoreRuntimeError::InvalidPublicationPlan);
        }
        validated.try_stage_exact_outcome_evidence(outcome_evidence_destination_dimension)
    }
}

impl FailureLifecycleState for FrozenFailureState {
    fn validate_commitment<O, PP, U, EvidenceKind, EvidenceTag>(
        validated: &mut ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>,
        _outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        if validated.outcome_evidence_commitment_len() == 1 {
            Ok(())
        } else {
            Err(StoreRuntimeError::InvalidPublicationPlan)
        }
    }
}

/// Rooted failure evidence. No mutation API is exposed after the one-shot lane completes.
#[must_use = "rooted failure evidence must be validated or prepared"]
pub struct RootedFailureEvidence<
    O,
    PP,
    U,
    Cause,
    EvidenceKind,
    EvidenceTag,
    State = OrdinaryFailureState,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    cause: Cause,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    _lifecycle: PhantomData<State>,
    // The physical owner must drop last so an untrusted cause panic is observed fail-closed.
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
}

/// Result of validating an already-rooted failure graph.
#[must_use = "the validated or rejected failure owner must be consumed"]
pub enum RootedFailureValidation<
    O,
    PP,
    U,
    Cause,
    EvidenceKind,
    EvidenceTag,
    State = OrdinaryFailureState,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(ValidatedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag, State>),
    Rejected(RejectedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>),
    Fallback(VerifiedMinimalFallback<StoreRuntimeError>),
}

#[must_use = "validated failure evidence must be prepared or downgraded"]
pub struct ValidatedFailureEvidence<
    O,
    PP,
    U,
    Cause,
    EvidenceKind,
    EvidenceTag,
    State = OrdinaryFailureState,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    cause: Cause,
    _lifecycle: PhantomData<State>,
    // The physical owner must drop last so an untrusted cause panic is observed fail-closed.
    validated: ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>,
}

#[must_use = "prepared failure evidence must be handed to the terminal publisher"]
pub struct PreparedFailureEvidence<Cause, EvidenceKind, EvidenceTag> {
    cause: Cause,
    final_control_cause: Option<ControlCause>,
    // The physical owner must drop last so an untrusted cause panic is observed fail-closed.
    prepared: PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag>,
}

#[must_use = "rejected failure evidence must prepare the minimal fallback"]
pub struct RejectedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    cause: Cause,
    // The physical owner must drop last so an untrusted cause panic is observed fail-closed.
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
}

/// Minimal fallback whose typed source cause remains available without exposing physical state.
#[must_use = "the verified minimal fallback must be handed to the terminal publisher"]
pub struct VerifiedMinimalFallback<Cause> {
    cause: Cause,
    // The physical owner must drop last so an untrusted cause panic is observed fail-closed.
    prepared: PreparedStoreFallback,
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
    cause: StoreNeverActivatedCause<E>,
    failure_code: PreparationFailureCode,
    outcome_evidence_plan: RetainedStoreBlockTablePlan,
    failure_work: FailureEvidenceWorkBudget,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    // The physical owner must drop last so an untrusted requested cause is observed fail-closed.
    authority: NeverActivatedAuthority<O, PP, U, L>,
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

#[cfg(test)]
mod scoped_transition_tests {
    use std::cell::Cell;
    use std::num::{NonZeroU32, NonZeroU64};

    use super::*;
    use crate::{
        DimensionPolicy, LedgerLimits, OperationControl, PhysicalStorageAccount,
        PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits, RetainedStoreBatchLimits,
        RetainedStoreFailureTerminalWorkPlan, RetainedStoreSharePlan, RetainedStoreStartPlan,
        StoreDimensionSemanticId, StoreFallbackEnvelope, StoreLedgerPlan, StorePhaseSemanticId,
        StoreSemanticCatalogId,
    };

    struct TestOwner;

    impl OperationOwner for TestOwner {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestProgress {}

    impl ProgressPhase for TestProgress {
        fn order(self) -> u32 {
            match self {}
        }

        fn stable_id(self) -> &'static str {
            match self {}
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestUnit {}

    impl ProgressUnit for TestUnit {
        fn stable_id(self) -> &'static str {
            match self {}
        }
    }

    #[derive(Clone, Copy)]
    struct TestKeys {
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        failure_terminal_dimension: StoreDimensionKey,
    }

    #[derive(Clone, Copy)]
    struct CanonicalTag;

    #[derive(Clone, Copy)]
    enum TestRejection {
        Requested,
    }

    struct TransitionDropMarker<'probe> {
        dropped: &'probe Cell<bool>,
        account: &'probe PhysicalStorageAccount,
        poison_seen_while_dropping: &'probe Cell<bool>,
        poison_event_count_while_dropping: &'probe Cell<u128>,
    }

    impl Drop for TransitionDropMarker<'_> {
        fn drop(&mut self) {
            let snapshot = self.account.snapshot();
            self.poison_seen_while_dropping.set(snapshot.poisoned());
            self.poison_event_count_while_dropping
                .set(snapshot.poison_event_count());
            self.dropped.set(true);
        }
    }

    struct PanickingTrackedPlan<'scope, 'probe> {
        scratch: Option<CanonicalReservation<'scope, TestOwner>>,
        reservation_released: &'probe Cell<bool>,
        marker: Option<TransitionDropMarker<'probe>>,
    }

    struct PanickingResultDrop<'probe>(&'probe Cell<bool>);

    struct TestActivationDescription<P> {
        request: CanonicalActivationRequest,
        requirements: CanonicalSuccessRequirements,
        execution: P,
    }

    impl<P> TestActivationDescription<P> {
        fn new(requirements: CanonicalSuccessRequirements, execution: P) -> Self {
            let (identity, request) = mint_canonical_activation_request()
                .expect("the test activation request identity must remain available");
            Self {
                request,
                requirements: requirements.with_request_identity(identity),
                execution,
            }
        }
    }

    impl<P> CanonicalActivationDescription for TestActivationDescription<P> {
        type Execution = P;

        fn seal_activation<'scope>(
            self,
            sealer: CanonicalActivationPlanSealer<'scope>,
        ) -> CanonicalSealedActivationPlan<'scope, Self::Execution> {
            sealer.seal(self.request, self.requirements, self.execution)
        }
    }

    impl Drop for PanickingResultDrop<'_> {
        fn drop(&mut self) {
            self.0.set(true);
            panic!("injected semantic result drop panic");
        }
    }

    impl Drop for PanickingTrackedPlan<'_, '_> {
        fn drop(&mut self) {
            drop(
                self.scratch
                    .take()
                    .expect("the tracked plan releases its reservation once"),
            );
            self.reservation_released.set(true);
            drop(
                self.marker
                    .take()
                    .expect("the tracked plan drops its marker once"),
            );
            panic!("injected tracked-plan drop panic");
        }
    }

    type TestOperation = RetainedStoreOperation<TestOwner, TestProgress, TestUnit>;
    type TestOutcome = CanonicalActivationOutcome<
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        TestRejection,
        ScalarSequence<u8>,
        CanonicalTag,
    >;

    const fn test_catalog() -> StoreSemanticCatalogId {
        StoreSemanticCatalogId::new(
            *b"baozi-scoped-tst",
            NonZeroU32::new(1).expect("test catalog revision is nonzero"),
        )
    }

    const fn phase_semantic(value: u64) -> StorePhaseSemanticId {
        StorePhaseSemanticId::new(
            test_catalog(),
            NonZeroU64::new(value).expect("test phase semantic id is nonzero"),
        )
    }

    const fn dimension_semantic(value: u64) -> StoreDimensionSemanticId {
        StoreDimensionSemanticId::new(
            test_catalog(),
            NonZeroU64::new(value).expect("test dimension semantic id is nonzero"),
        )
    }

    fn account() -> PhysicalStorageAccount {
        PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
        )
        .unwrap()
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 256).unwrap())
        .unwrap()
    }

    fn writer_plan(
        keys: TestKeys,
        attributions: usize,
        commitments: usize,
    ) -> RetainedStoreWriterPlan {
        RetainedStoreWriterPlan::new(
            keys.phase,
            keys.dimension,
            keys.dimension,
            RetainedStoreBatchLimits::new(attributions, commitments).unwrap(),
        )
    }

    fn operation(account: &PhysicalStorageAccount) -> (TestOperation, TestKeys) {
        let domain = StoreKeyDomain::try_new(test_catalog()).unwrap();
        let keys = TestKeys {
            phase: domain.try_admit_phase(phase_semantic(1)).unwrap(),
            dimension: domain.try_admit_dimension(dimension_semantic(1)).unwrap(),
            failure_terminal_dimension: domain.try_admit_dimension(dimension_semantic(2)).unwrap(),
        };
        let failure_terminal_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
            keys.failure_terminal_dimension,
            1,
            64,
            64,
            u64::MAX,
            u64::MAX,
        )
        .unwrap();
        let ordinary_policy = DimensionPolicy::reservable_bytes(
            failure_terminal_work.ordinary_attempted_work_limit(),
            failure_terminal_work.ordinary_consumed_work_limit(),
            u64::MAX,
            u64::MAX,
        )
        .with_published_outcome_evidence_limit(u64::MAX);
        let limits = LedgerLimits::new(
            u64::MAX,
            [
                (keys.dimension, ordinary_policy),
                (
                    keys.failure_terminal_dimension,
                    failure_terminal_work.requirement().dimension_policy(),
                ),
            ],
        )
        .unwrap();
        let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
        let start = RetainedStoreStartPlan::new(
            writer_plan(keys, 32, 8),
            RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
            StoreFallbackEnvelope::new(91),
            keys.dimension,
            1,
            keys.dimension,
            1,
            failure_terminal_work,
        );
        (
            TestOperation::try_start(OperationControl::default(), ledger_plan, account, start)
                .unwrap(),
            keys,
        )
    }

    fn scoped_shape(
        keys: TestKeys,
    ) -> (
        CanonicalActivationSessionPolicy,
        RetainedStoreBatchLimits,
        CanonicalFailureRequirements,
        CanonicalSuccessRequirements,
    ) {
        (
            CanonicalActivationSessionPolicy::new(
                RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
                RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
                CanonicalMaterializationRole::new(keys.dimension, keys.dimension),
                CanonicalValidationRole::new(keys.phase, keys.dimension, keys.dimension),
                keys.dimension,
                keys.dimension,
            ),
            RetainedStoreBatchLimits::new(2, 2).unwrap(),
            CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap()),
            CanonicalSuccessRequirements::new(
                CanonicalTableShape::new(1, 1).unwrap(),
                2,
                1,
                NonZeroU64::MIN,
            )
            .unwrap(),
        )
    }

    #[test]
    fn semantic_result_drop_fault_preserves_runtime_first_cause_and_latches_contract() {
        let dropped = Cell::new(false);
        let mut first_fault = None;

        let result = preserve_runtime_fault_then_drop_result(
            &mut first_fault,
            StoreRuntimeError::Control(crate::ControlCause::DeadlineExceeded),
            PanickingResultDrop(&dropped),
        );

        assert!(matches!(
            result,
            Err(StoreRuntimeError::Control(
                crate::ControlCause::DeadlineExceeded
            ))
        ));
        assert!(dropped.get());
        let first_fault = first_fault.expect("the runtime first cause remains recorded");
        assert!(matches!(
            first_fault.cause,
            StoreRuntimeError::Control(crate::ControlCause::DeadlineExceeded)
        ));
        assert!(first_fault.contract_seen);
    }

    fn assert_owner_consuming_transition_panic_is_contained(point: ScopedTransitionPanicPoint) {
        let account = account();
        let (operation, keys) = operation(&account);
        let (policy, evidence_limits, failure, success) = scoped_shape(keys);
        let (request_identity, request) = mint_canonical_activation_request().unwrap();
        let success = success.with_request_identity(request_identity);
        let owner = operation
            .into_session_outcome_owner(policy, evidence_limits, failure)
            .unwrap();
        let mut success = Some(success);

        let mut request = Some(request);
        let result: Result<TestOutcome, StoreActivationContainment> = owner
            .plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                semantic.run_unit(|| ()).unwrap();
                let completion = semantic.finish_exact().unwrap();
                let permit = match planning.seal_activation(
                    TestActivationDescription {
                        request: request
                            .take()
                            .expect("the test activation request is consumed exactly once"),
                        requirements: success
                            .take()
                            .expect("the test success requirements are consumed exactly once"),
                        execution: (),
                    },
                    completion,
                ) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
                test_panic_scoped_transition_at(point);
                permit.activate(|lane, _plan| match point {
                    ScopedTransitionPanicPoint::TryOpenActivation => {
                        panic!("try_open_activation fault must run before materialization")
                    }
                    ScopedTransitionPanicPoint::FinishRejectedSession => lane
                        .reject::<(), _, ScalarSequence<u8>, CanonicalTag>(
                            TestRejection::Requested,
                            7,
                        ),
                })
            });

        assert!(result.is_err());
        assert!(!test_scoped_transition_panic_is_armed());
        let snapshot = account.snapshot();
        assert!(snapshot.poisoned());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(|evidence| evidence.reason()),
            Some(crate::AccountPoisonReason::OwnershipGraphUnverified)
        );
    }

    #[test]
    fn panic_inside_try_open_activation_uses_the_transition_account_anchor() {
        assert_owner_consuming_transition_panic_is_contained(
            ScopedTransitionPanicPoint::TryOpenActivation,
        );
    }

    #[test]
    fn panic_inside_try_open_activation_drops_scoped_inputs_before_containment() {
        let account = account();
        let (operation, keys) = operation(&account);
        let (policy, evidence_limits, failure, success) = scoped_shape(keys);
        let owner = operation
            .into_session_outcome_owner(policy, evidence_limits, failure)
            .unwrap();
        let mut success = Some(success);
        let physical_baseline = account.snapshot();
        assert!(!physical_baseline.poisoned());
        assert!(physical_baseline.live_retained_allocations() > 0);
        let reservation_released = Cell::new(false);
        let plan_marker_dropped = Cell::new(false);
        let plan_poison_seen_while_dropping = Cell::new(false);
        let plan_poison_events_while_dropping = Cell::new(u128::MAX);
        let materializer_dropped = Cell::new(false);
        let materializer_poison_seen_while_dropping = Cell::new(false);
        let materializer_poison_events_while_dropping = Cell::new(u128::MAX);

        let result: Result<TestOutcome, StoreActivationContainment> = owner
            .plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                let scratch = semantic.reserve(1).unwrap();
                semantic.validate_live_owner(&scratch).unwrap();
                semantic.run_unit(|| ()).unwrap();
                let completion = semantic.finish_exact().unwrap();
                let plan = PanickingTrackedPlan {
                    scratch: Some(scratch),
                    reservation_released: &reservation_released,
                    marker: Some(TransitionDropMarker {
                        dropped: &plan_marker_dropped,
                        account: &account,
                        poison_seen_while_dropping: &plan_poison_seen_while_dropping,
                        poison_event_count_while_dropping: &plan_poison_events_while_dropping,
                    }),
                };
                let permit = match planning.seal_activation(
                    TestActivationDescription::new(
                        success
                            .take()
                            .expect("the test success requirements are consumed exactly once"),
                        plan,
                    ),
                    completion,
                ) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
                let materializer_marker = TransitionDropMarker {
                    dropped: &materializer_dropped,
                    account: &account,
                    poison_seen_while_dropping: &materializer_poison_seen_while_dropping,
                    poison_event_count_while_dropping: &materializer_poison_events_while_dropping,
                };
                test_panic_scoped_transition_at(ScopedTransitionPanicPoint::TryOpenActivation);
                permit.activate(
                    move |_lane,
                          _plan|
                          -> CanonicalMaterialization<
                        '_,
                        (),
                        TestRejection,
                        ScalarSequence<u8>,
                        CanonicalTag,
                    > {
                        let _ = &materializer_marker;
                        panic!("try_open_activation fault must run before materialization")
                    },
                )
            });

        assert!(result.is_err());
        assert!(!test_scoped_transition_panic_is_armed());
        assert!(reservation_released.get());
        assert!(plan_marker_dropped.get());
        assert!(!plan_poison_seen_while_dropping.get());
        assert_eq!(plan_poison_events_while_dropping.get(), 0);
        assert!(materializer_dropped.get());
        assert!(!materializer_poison_seen_while_dropping.get());
        assert_eq!(materializer_poison_events_while_dropping.get(), 0);
        let snapshot = account.snapshot();
        assert!(snapshot.poisoned());
        assert!(snapshot.poison_event_count() > 0);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(|evidence| evidence.reason()),
            Some(crate::AccountPoisonReason::OwnershipGraphUnverified)
        );
        assert_eq!(
            snapshot.live_retained_bytes(),
            physical_baseline.live_retained_bytes()
        );
        assert_eq!(
            snapshot.live_retained_allocations(),
            physical_baseline.live_retained_allocations()
        );
    }

    #[test]
    fn fail_closed_publication_open_releases_scoped_inputs_before_final_containment() {
        let account = account();
        let (operation, keys) = operation(&account);
        let (policy, evidence_limits, failure, success) = scoped_shape(keys);
        let owner = operation
            .into_session_outcome_owner(policy, evidence_limits, failure)
            .unwrap();
        let mut success = Some(success);
        let physical_baseline = account.snapshot();
        let reservation_released = Cell::new(false);
        let plan_marker_dropped = Cell::new(false);
        let plan_poison_seen_while_dropping = Cell::new(false);
        let plan_poison_events_while_dropping = Cell::new(u128::MAX);
        let materializer_dropped = Cell::new(false);
        let materializer_poison_seen_while_dropping = Cell::new(false);
        let materializer_poison_events_while_dropping = Cell::new(u128::MAX);

        let result: Result<TestOutcome, StoreActivationContainment> = owner
            .plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                let scratch = semantic.reserve(1).unwrap();
                semantic.validate_live_owner(&scratch).unwrap();
                semantic.run_unit(|| ()).unwrap();
                let completion = semantic.finish_exact().unwrap();
                let plan = PanickingTrackedPlan {
                    scratch: Some(scratch),
                    reservation_released: &reservation_released,
                    marker: Some(TransitionDropMarker {
                        dropped: &plan_marker_dropped,
                        account: &account,
                        poison_seen_while_dropping: &plan_poison_seen_while_dropping,
                        poison_event_count_while_dropping: &plan_poison_events_while_dropping,
                    }),
                };
                let permit = match planning.seal_activation(
                    TestActivationDescription::new(
                        success
                            .take()
                            .expect("the test success requirements are consumed exactly once"),
                        plan,
                    ),
                    completion,
                ) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
                let materializer_marker = TransitionDropMarker {
                    dropped: &materializer_dropped,
                    account: &account,
                    poison_seen_while_dropping: &materializer_poison_seen_while_dropping,
                    poison_event_count_while_dropping: &materializer_poison_events_while_dropping,
                };
                crate::retained::panic_next_publication_account_open();
                permit.activate(
                    move |_lane,
                          _plan|
                          -> CanonicalMaterialization<
                        '_,
                        (),
                        TestRejection,
                        ScalarSequence<u8>,
                        CanonicalTag,
                    > {
                        let _ = &materializer_marker;
                        panic!("a fail-closed publication open must not enter materialization")
                    },
                )
            });

        assert!(result.is_err());
        assert!(reservation_released.get());
        assert!(plan_marker_dropped.get());
        assert!(plan_poison_seen_while_dropping.get());
        assert!(materializer_dropped.get());
        assert!(materializer_poison_seen_while_dropping.get());
        let poison_events_before_final_containment = plan_poison_events_while_dropping.get();
        assert!(poison_events_before_final_containment > 0);
        assert_eq!(
            materializer_poison_events_while_dropping.get(),
            poison_events_before_final_containment
        );
        let snapshot = account.snapshot();
        assert!(snapshot.poisoned());
        assert!(snapshot.poison_event_count() > poison_events_before_final_containment);
        assert!(snapshot.poison_event_count_exact());
        assert!(snapshot.poison_event_domain_valid());
        assert_eq!(
            snapshot
                .first_poison_evidence()
                .map(|evidence| evidence.reason()),
            Some(crate::AccountPoisonReason::OwnershipGraphUnverified)
        );
        assert_eq!(
            snapshot.live_retained_bytes(),
            physical_baseline.live_retained_bytes()
        );
        assert_eq!(
            snapshot.live_retained_allocations(),
            physical_baseline.live_retained_allocations()
        );
    }

    #[test]
    fn panic_inside_finish_rejected_session_uses_the_transition_account_anchor() {
        assert_owner_consuming_transition_panic_is_contained(
            ScopedTransitionPanicPoint::FinishRejectedSession,
        );
    }
}
