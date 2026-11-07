use std::marker::PhantomData;

use super::{
    BUILD_CODE_DOMAIN, BlockEmplacer, BlockPlacement, BytesPlan, BytesWriter,
    CONTAINMENT_FALLBACK_CODE, CanonicalBytesRef, CanonicalInheritanceSource,
    CanonicalLeafInheritanceError, CanonicalRecordRef, CanonicalRecordWriter, CanonicalSequenceRef,
    CanonicalSourceBindError, CanonicalStoreRef, CanonicalUtf8Ref, FreshStoreLineage,
    OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef, OutcomeEvidenceRecordWriter,
    OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef, OutcomeEvidenceUtf8Ref,
    PreparationFailureCode, PublishedCanonicalOutput, PublishedCanonicalStore,
    PublishedOutcomeEvidenceStore, RawStoreAssemblyBorrow, RecordPlan, RecordWriterCore,
    RetainedAllocationPlan, RetainedBuildFailureCode, RetainedPublicationError,
    RetainedStoreControlCheckpoint, RetainedStoreWorkControl, RetainedWorkControl,
    RetainedWriterError, SequencePlan, SequenceWriter, ShareableStoreKind,
    StoreBuildCallbackFailure, StoreBuildError, StoreControlledError, StoreDimensionKey,
    StoreKeyDomain, StorePhaseKey, StorePublicationState, StoreRuntimeError, StoreWorkError,
    StoredBlock, StoredScalar, UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE, Utf8Plan, Utf8Writer,
    canonical_source_slot_matches_kind, checked_source_record, finish_scoped_writer,
    map_publication_error, mapped_canonical_record_cell, publication_error_requires_containment,
    read_canonical_source_records, read_canonical_source_sequence, sealed, work_failure_code,
};
use crate::control::{ProgressPhase, ProgressUnit};
use crate::owner::OperationOwner;

mod failure;
pub(super) use failure::StoreFailureEvidenceLoan;
mod open;
pub(super) use open::{
    StoreOperationLoan, StorePublicationOpenDecision, StorePublicationOpenDisposition,
};
mod validation;
pub(super) use validation::{
    StoreClosureDecision, StoreSuccessFallbackProvenance, StoreSuccessResolution,
};

/// Borrows ordinary publication work without taking terminal ownership.
///
/// Checkpoints borrow this value, so callers must keep it bound while a checkpoint is live.
pub struct StorePublicationLoan<'op, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) raw: RawStoreAssemblyBorrow<'op, O, PP, U>,
    pub(super) domain: &'op StoreKeyDomain,
    pub(super) state: &'op mut StorePublicationState,
    // The concrete root retains lineage; ordinary methods need only its type authority.
    pub(super) lineage: PhantomData<&'op L>,
}

impl<'op, O, PP, U, L> StorePublicationLoan<'op, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) {
        self.raw
            .loan_mut()
            .report_progress(phase, unit, completed, total);
    }

    /// Borrows transient storage and control authority from the operation owned by this assembly.
    ///
    /// The checkpoint cannot inspect or mutate either publication table. Callers must release every
    /// returned reservation before preparing a terminal publication.
    pub fn try_control_checkpoint(
        &mut self,
    ) -> Result<RetainedStoreControlCheckpoint<'_, O, PP, U>, StoreRuntimeError> {
        self.ensure_writable()?;
        let raw = match self.raw.loan_mut().try_control_checkpoint() {
            Ok(raw) => raw,
            Err(error) => {
                if publication_error_requires_containment(&error) {
                    self.state.fallback_cause =
                        Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                }
                return Err(map_publication_error(&error));
            }
        };
        Ok(RetainedStoreControlCheckpoint {
            raw,
            domain: self.domain,
            terminal_only: false,
        })
    }

    /// Returns the number of canonical table members.
    pub fn canonical_len(&self) -> usize {
        self.raw.loan().canonical_len()
    }

    pub(super) fn canonical_new_blocks(&self) -> usize {
        self.raw.loan().canonical_new_blocks()
    }

    pub(super) fn canonical_commitment_len(&self) -> usize {
        self.raw.loan().canonical_commitment_len()
    }

    pub(super) fn contains_activation_canonical_root<Kind, Tag>(
        &self,
        root: CanonicalStoreRef<Kind, Tag>,
    ) -> bool
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.raw.loan().contains_canonical_member(root.raw)
    }

    /// Returns the number of outcome-evidence table members.
    pub fn outcome_evidence_len(&self) -> usize {
        self.raw.loan().outcome_evidence_len()
    }

    pub(super) fn outcome_evidence_new_blocks(&self) -> usize {
        self.raw.loan().outcome_evidence_new_blocks()
    }

    pub(super) fn outcome_evidence_commitment_len(&self) -> usize {
        self.raw.loan().outcome_evidence_commitment_len()
    }

    pub(super) fn is_fallback_only(&self) -> bool {
        self.state.fallback_cause.is_some()
    }

    pub(super) fn latch_activation_terminal_cause(&mut self, cause: PreparationFailureCode) {
        self.raw.loan_mut().record_terminal_cause(cause);
    }

    pub(super) fn reject_failure_terminal_work_envelope(&mut self, code: u32) {
        self.raw
            .loan_mut()
            .reject_failure_terminal_work_envelope(code);
    }

    pub(super) fn ensure_writable(&self) -> Result<(), StoreRuntimeError> {
        if self.is_fallback_only() {
            Err(StoreRuntimeError::FallbackOnly)
        } else if self.state.success_rejection.is_some() {
            Err(StoreRuntimeError::SuccessOutcomeRejected)
        } else {
            Ok(())
        }
    }

    pub(super) fn ensure_evidence_writable(&self) -> Result<(), StoreRuntimeError> {
        if self.is_fallback_only() {
            Err(StoreRuntimeError::FallbackOnly)
        } else {
            Ok(())
        }
    }

    pub(super) fn validate_dimension(
        &mut self,
        dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        let result = self.raw.loan_mut().try_validate_work_dimension(dimension);
        self.map_action(result)
    }

    pub(super) fn record_success_rejection(&mut self, cause: PreparationFailureCode) {
        if self.state.success_rejection.is_some() {
            return;
        }
        self.raw.loan_mut().activate_failure_terminal_work();
        self.state.success_rejection = Some(cause);
    }

    pub(super) fn map_action<T>(
        &mut self,
        result: Result<T, RetainedPublicationError<StoreDimensionKey>>,
    ) -> Result<T, StoreRuntimeError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if matches!(
                    &error,
                    RetainedPublicationError::Writer(RetainedWriterError::Ledger(_))
                ) {
                    self.record_success_rejection(PreparationFailureCode::ledger());
                }
                if publication_error_requires_containment(&error) {
                    self.state.fallback_cause =
                        Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                }
                Err(map_publication_error(&error))
            }
        }
    }

    pub(super) fn try_retain_success_outcome_evidence_controlled<Kind, Tag>(
        &mut self,
        work_dimension: StoreDimensionKey,
        allocation: RetainedAllocationPlan,
        build: impl for<'placement, 'control> FnOnce(
            BlockEmplacer<'placement, StoredBlock>,
            RetainedWorkControl<'control, O, StorePhaseKey, PP, U, StoreDimensionKey>,
        ) -> Result<
            BlockPlacement<'placement, StoredBlock, Tag>,
            RetainedBuildFailureCode,
        >,
    ) -> Result<OutcomeEvidenceStoreRef<Kind, Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_evidence_writable()?;
        self.validate_dimension(work_dimension)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_success_outcome_evidence_controlled(work_dimension, allocation, build);
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    pub(super) fn map_pure_preflight<T>(
        &mut self,
        result: Result<T, RetainedPublicationError<StoreDimensionKey>>,
    ) -> Result<T, StoreRuntimeError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if publication_error_requires_containment(&error) {
                    self.state.fallback_cause =
                        Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                }
                Err(map_publication_error(&error))
            }
        }
    }

    pub(super) fn map_controlled_action<T>(
        &mut self,
        result: Result<T, RetainedPublicationError<StoreDimensionKey>>,
        callback_failure: Option<StoreBuildCallbackFailure>,
    ) -> Result<T, StoreRuntimeError> {
        let Some(callback_failure) = callback_failure else {
            return self.map_action(result);
        };
        match result {
            Ok(_) => {
                self.state.fallback_cause =
                    Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            }
            Err(error) if publication_error_requires_containment(&error) => {
                self.state.fallback_cause =
                    Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                Err(map_publication_error(&error))
            }
            Err(_) => match callback_failure {
                StoreBuildCallbackFailure::VerifiedBuild(cause) => {
                    Err(StoreRuntimeError::Build(cause))
                }
                StoreBuildCallbackFailure::VerifiedWork(cause) => {
                    self.record_success_rejection(work_failure_code(&cause));
                    Err(cause.into_runtime())
                }
                StoreBuildCallbackFailure::UnverifiedCause => {
                    self.state.fallback_cause = Some(PreparationFailureCode::domain(
                        UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE,
                    ));
                    Err(StoreRuntimeError::UnverifiedControlledCause)
                }
            },
        }
    }

    pub(super) fn map_source_bind_action(
        &mut self,
        result: Result<(), CanonicalSourceBindError<StoreDimensionKey>>,
    ) -> Result<(), StoreRuntimeError> {
        match result {
            Ok(()) => Ok(()),
            Err(CanonicalSourceBindError::Publication(cause)) => self.map_action(Err(cause)),
            Err(CanonicalSourceBindError::Control(cause)) => {
                let cause = StoreWorkError::control(cause);
                self.record_success_rejection(work_failure_code(&cause));
                Err(cause.into_runtime())
            }
            Err(CanonicalSourceBindError::Ledger(cause)) => {
                let cause = StoreWorkError::ledger(cause);
                self.record_success_rejection(work_failure_code(&cause));
                Err(cause.into_runtime())
            }
        }
    }

    pub(super) fn map_leaf_inheritance_action(
        &mut self,
        result: Result<(), CanonicalLeafInheritanceError<StoreDimensionKey>>,
    ) -> Result<(), StoreRuntimeError> {
        match result {
            Ok(()) => Ok(()),
            Err(CanonicalLeafInheritanceError::Publication(cause)) => self.map_action(Err(cause)),
            Err(CanonicalLeafInheritanceError::Control(cause)) => {
                let cause = StoreWorkError::control(cause);
                self.record_success_rejection(work_failure_code(&cause));
                Err(cause.into_runtime())
            }
            Err(CanonicalLeafInheritanceError::Ledger(cause)) => {
                let cause = StoreWorkError::ledger(cause);
                self.record_success_rejection(work_failure_code(&cause));
                Err(cause.into_runtime())
            }
        }
    }

    /// Binds the exact same-account canonical source used by inheritance and rewrite operations.
    ///
    /// Binding initializes the preallocated dense source map once, polling and charging one work
    /// unit per source member. A cancelled or budget-rejected bind clears the partial map before
    /// rejecting this assembly's success outcome; a different source set cannot replace a
    /// successful binding.
    pub(super) fn try_bind_canonical_source_from(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        work_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let result = self
            .raw
            .loan_mut()
            .try_bind_canonical_source(source.aggregate(), work_dimension);
        self.map_source_bind_action(result)
    }

    pub(super) fn require_canonical_source_from(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_writable()?;
        let result = self
            .raw
            .loan_mut()
            .require_canonical_source(source.aggregate());
        self.map_pure_preflight(result)
    }

    pub(super) fn validate_canonical_share_source_from<SlotKind, SlotTag>(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        source_ref: CanonicalStoreRef<SlotKind, SlotTag>,
    ) -> Result<(), StoreRuntimeError>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        self.require_canonical_source_from(source)?;
        if canonical_source_slot_matches_kind(source, source_ref) {
            Ok(())
        } else {
            Err(StoreRuntimeError::InvalidSourceSlot)
        }
    }

    /// Inherits every eligible immutable leaf from the exact bound canonical source.
    ///
    /// Scalar sequences, bytes, and UTF-8 blocks are shared once, while record blocks are skipped.
    /// Historical source lineage does not affect eligibility: a leaf rewritten in an earlier
    /// generation can be inherited when unchanged in this destination. The operation polls and
    /// charges once per source member and rolls back the newly inherited prefix after an ordinary
    /// failure.
    pub(super) fn try_inherit_all_canonical_leaves_from(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        work_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let result = self
            .raw
            .loan_mut()
            .try_inherit_all_canonical_leaves(source.aggregate(), work_dimension);
        self.map_leaf_inheritance_action(result)
    }

    /// Retains one exact canonical scalar sequence.
    pub fn try_retain_canonical_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl FnOnce(&mut SequenceWriter<T>) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_canonical(allocation, |emplacer| {
                let writer = <T as sealed::StoredScalar>::new_writer(plan.capacity, plan.len)?;
                let block = finish_scoped_writer(writer, build, SequenceWriter::finish)?;
                Ok(emplacer.place(block, tag))
            });
        self.map_action(result).map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Replaces one exact same-account canonical sequence with newly constructed immutable
    /// storage while preserving its source lineage.
    ///
    /// The exact typed source reference is checked before retained allocation or work charging.
    /// The replacement may have a different element count, which permits structural edits such as
    /// replacing a matrix representation with TRS components. One source slot can map to only one
    /// destination slot; sharing and rewriting the same source in one destination are mutually
    /// exclusive.
    pub(super) fn try_rewrite_canonical_sequence_from<T, Tag>(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        source_ref: CanonicalSequenceRef<T, Tag>,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl FnOnce(&mut SequenceWriter<T>) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        if read_canonical_source_sequence(source, source_ref).is_none() {
            return Err(StoreRuntimeError::InvalidSourceSlot);
        }
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self.raw.loan_mut().try_retain_rewritten_canonical(
            source.aggregate(),
            source_ref.raw,
            allocation,
            |emplacer| {
                let writer = <T as sealed::StoredScalar>::new_writer(plan.capacity, plan.len)?;
                let block = finish_scoped_writer(writer, build, SequenceWriter::finish)?;
                Ok(emplacer.place(block, tag))
            },
        );
        self.map_action(result).map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Retains one canonical scalar sequence with operation-controlled polling and work charging.
    pub fn try_retain_canonical_sequence_controlled<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnOnce(
            &mut SequenceWriter<T>,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let mut callback_cause = None;
        let result = self.raw.loan_mut().try_retain_canonical_controlled(
            work_dimension,
            allocation,
            |emplacer, raw| {
                let mut writer = <T as sealed::StoredScalar>::new_writer(plan.capacity, plan.len)?;
                let mut control = RetainedStoreWorkControl { raw, sticky: None };
                let callback = build(&mut writer, &mut control);
                if let Err(cause) = control.finish_build(callback) {
                    callback_cause = Some(cause);
                    return Err(RetainedBuildFailureCode::new(BUILD_CODE_DOMAIN));
                }
                Ok(emplacer.place(writer.finish()?, tag))
            },
        );
        self.map_controlled_action(result, callback_cause)
            .map(|raw| CanonicalStoreRef {
                raw,
                marker: PhantomData,
            })
    }

    /// Replaces one exact same-account canonical sequence with operation-controlled polling and
    /// work charging while preserving its source lineage.
    pub(super) fn try_rewrite_canonical_sequence_controlled_from<T, Tag>(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        source_ref: CanonicalSequenceRef<T, Tag>,
        plan: SequencePlan<T>,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnOnce(
            &mut SequenceWriter<T>,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        if read_canonical_source_sequence(source, source_ref).is_none() {
            return Err(StoreRuntimeError::InvalidSourceSlot);
        }
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let mut callback_cause = None;
        let result = self
            .raw
            .loan_mut()
            .try_retain_rewritten_canonical_controlled(
                source.aggregate(),
                source_ref.raw,
                work_dimension,
                allocation,
                |emplacer, raw| {
                    let mut writer =
                        <T as sealed::StoredScalar>::new_writer(plan.capacity, plan.len)?;
                    let mut control = RetainedStoreWorkControl { raw, sticky: None };
                    let callback = build(&mut writer, &mut control);
                    if let Err(cause) = control.finish_build(callback) {
                        callback_cause = Some(cause);
                        return Err(RetainedBuildFailureCode::new(BUILD_CODE_DOMAIN));
                    }
                    Ok(emplacer.place(writer.finish()?, tag))
                },
            );
        self.map_controlled_action(result, callback_cause)
            .map(|raw| CanonicalStoreRef {
                raw,
                marker: PhantomData,
            })
    }

    /// Retains one exact outcome-evidence scalar sequence.
    pub fn try_retain_outcome_evidence_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl FnOnce(&mut SequenceWriter<T>) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_evidence_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_outcome_evidence(allocation, |emplacer| {
                let writer = <T as sealed::StoredScalar>::new_writer(plan.capacity, plan.len)?;
                let block = finish_scoped_writer(writer, build, SequenceWriter::finish)?;
                Ok(emplacer.place(block, tag))
            });
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Retains one exact canonical binary block.
    pub fn try_retain_canonical_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        tag: Tag,
        build: impl FnOnce(&mut BytesWriter) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalBytesRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_canonical(allocation, |emplacer| {
                let writer = BytesWriter::new(plan)?;
                let block = finish_scoped_writer(writer, build, BytesWriter::finish)?;
                Ok(emplacer.place(block, tag))
            });
        self.map_action(result).map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Retains one canonical binary block with operation-controlled polling and work charging.
    pub fn try_retain_canonical_bytes_controlled<Tag>(
        &mut self,
        plan: BytesPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnOnce(
            &mut BytesWriter,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalBytesRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let mut callback_cause = None;
        let result = self.raw.loan_mut().try_retain_canonical_controlled(
            work_dimension,
            allocation,
            |emplacer, raw| {
                let mut writer = BytesWriter::new(plan)?;
                let mut control = RetainedStoreWorkControl { raw, sticky: None };
                let callback = build(&mut writer, &mut control);
                if let Err(cause) = control.finish_build(callback) {
                    callback_cause = Some(cause);
                    return Err(RetainedBuildFailureCode::new(BUILD_CODE_DOMAIN));
                }
                Ok(emplacer.place(writer.finish()?, tag))
            },
        );
        self.map_controlled_action(result, callback_cause)
            .map(|raw| CanonicalStoreRef {
                raw,
                marker: PhantomData,
            })
    }

    /// Retains one exact outcome-evidence binary block.
    pub fn try_retain_outcome_evidence_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        tag: Tag,
        build: impl FnOnce(&mut BytesWriter) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_evidence_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_outcome_evidence(allocation, |emplacer| {
                let writer = BytesWriter::new(plan)?;
                let block = finish_scoped_writer(writer, build, BytesWriter::finish)?;
                Ok(emplacer.place(block, tag))
            });
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Retains one exact canonical UTF-8 block.
    pub fn try_retain_canonical_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        tag: Tag,
        build: impl FnOnce(&mut Utf8Writer) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalUtf8Ref<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_canonical(allocation, |emplacer| {
                let writer = Utf8Writer::new(plan)?;
                let block = finish_scoped_writer(writer, build, Utf8Writer::finish)?;
                Ok(emplacer.place(block, tag))
            });
        self.map_action(result).map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Retains one canonical UTF-8 block with operation-controlled polling and work charging.
    pub fn try_retain_canonical_utf8_controlled<Tag>(
        &mut self,
        plan: Utf8Plan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnOnce(
            &mut Utf8Writer,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalUtf8Ref<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let mut callback_cause = None;
        let result = self.raw.loan_mut().try_retain_canonical_controlled(
            work_dimension,
            allocation,
            |emplacer, raw| {
                let mut writer = Utf8Writer::new(plan)?;
                let mut control = RetainedStoreWorkControl { raw, sticky: None };
                let callback = build(&mut writer, &mut control);
                if let Err(cause) = control.finish_build(callback) {
                    callback_cause = Some(cause);
                    return Err(RetainedBuildFailureCode::new(BUILD_CODE_DOMAIN));
                }
                Ok(emplacer.place(writer.finish()?, tag))
            },
        );
        self.map_controlled_action(result, callback_cause)
            .map(|raw| CanonicalStoreRef {
                raw,
                marker: PhantomData,
            })
    }

    /// Retains one exact outcome-evidence UTF-8 block.
    pub fn try_retain_outcome_evidence_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        tag: Tag,
        build: impl FnOnce(&mut Utf8Writer) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_evidence_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_outcome_evidence(allocation, |emplacer| {
                let writer = Utf8Writer::new(plan)?;
                let block = finish_scoped_writer(writer, build, Utf8Writer::finish)?;
                Ok(emplacer.place(block, tag))
            });
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Retains one canonical fixed-width record block.
    pub fn try_retain_canonical_records<Tag>(
        &mut self,
        plan: RecordPlan,
        tag: Tag,
        build: impl FnOnce(&mut CanonicalRecordWriter) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_canonical(allocation, |emplacer| {
                let writer = CanonicalRecordWriter::new(plan)?;
                let block = finish_scoped_writer(writer, build, CanonicalRecordWriter::finish)?;
                Ok(emplacer.place(block, tag))
            });
        self.map_action(result).map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Replaces one exact same-account canonical record block with newly authored immutable
    /// records while preserving its source lineage.
    ///
    /// `plan` describes the destination shape, which may differ from the source. Embedded
    /// references can only be written through the typed canonical record writer, so callers may
    /// target already admitted destination-local members without observing physical identities.
    pub(super) fn try_rewrite_canonical_records_with_from<Tag>(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        tag: Tag,
        build: impl FnOnce(&mut CanonicalRecordWriter) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        if read_canonical_source_records(source, source_ref).is_none() {
            return Err(StoreRuntimeError::InvalidSourceSlot);
        }
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self.raw.loan_mut().try_retain_rewritten_canonical(
            source.aggregate(),
            source_ref.raw,
            allocation,
            |emplacer| {
                let writer = CanonicalRecordWriter::new(plan)?;
                let block = finish_scoped_writer(writer, build, CanonicalRecordWriter::finish)?;
                Ok(emplacer.place(block, tag))
            },
        );
        self.map_action(result).map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Copies one same-account canonical record block and rewrites every embedded canonical
    /// reference through the exact bound source map.
    ///
    /// Every source cell is polled and charged exactly once inside the retained builder boundary.
    /// Record blocks themselves are never shared.
    pub(super) fn try_rewrite_canonical_records_controlled_from<Tag>(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        if read_canonical_source_records(source, source_ref).is_none() {
            return Err(StoreRuntimeError::InvalidSourceSlot);
        }
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let mut callback_cause = None;
        let result = self.raw.loan_mut().try_rewrite_canonical_controlled(
            source.aggregate(),
            source_ref.raw,
            work_dimension,
            allocation,
            |emplacer, source_block, mapping, raw| {
                let mut control = RetainedStoreWorkControl { raw, sticky: None };
                let callback = (|| -> Result<StoredBlock, StoreControlledError> {
                    let storage = checked_source_record(source_block, plan)
                        .ok_or_else(StoreBuildError::capacity_envelope_violated)?;
                    let mut writer = RecordWriterCore::new(plan)?;
                    for source_cell in storage.cells.iter().copied() {
                        control.poll()?;
                        control.charge_observed(1)?;
                        if writer.storage.cells.len() >= plan.cells {
                            return Err(StoreBuildError::capacity_exceeded().into());
                        }
                        let cell = mapped_canonical_record_cell(source_cell, &mapping)
                            .ok_or_else(StoreBuildError::capacity_envelope_violated)?;
                        writer.storage.cells.push(cell);
                    }
                    writer.finish().map_err(Into::into)
                })();
                let block = match control.finish_build(callback) {
                    Ok(block) => block,
                    Err(cause) => {
                        callback_cause = Some(cause);
                        return Err(RetainedBuildFailureCode::new(BUILD_CODE_DOMAIN));
                    }
                };
                Ok(emplacer.place(block, tag))
            },
        );
        self.map_controlled_action(result, callback_cause)
            .map(|raw| CanonicalStoreRef {
                raw,
                marker: PhantomData,
            })
    }

    /// Retains one canonical record block with operation-controlled polling and work charging.
    pub fn try_retain_canonical_records_controlled<Tag>(
        &mut self,
        plan: RecordPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnOnce(
            &mut CanonicalRecordWriter,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let mut callback_cause = None;
        let result = self.raw.loan_mut().try_retain_canonical_controlled(
            work_dimension,
            allocation,
            |emplacer, raw| {
                let mut writer = CanonicalRecordWriter::new(plan)?;
                let mut control = RetainedStoreWorkControl { raw, sticky: None };
                let callback = build(&mut writer, &mut control);
                if let Err(cause) = control.finish_build(callback) {
                    callback_cause = Some(cause);
                    return Err(RetainedBuildFailureCode::new(BUILD_CODE_DOMAIN));
                }
                Ok(emplacer.place(writer.finish()?, tag))
            },
        );
        self.map_controlled_action(result, callback_cause)
            .map(|raw| CanonicalStoreRef {
                raw,
                marker: PhantomData,
            })
    }

    /// Replaces one exact same-account canonical record block through a typed writer with
    /// operation-controlled polling and work charging.
    pub(super) fn try_rewrite_canonical_records_with_controlled_from<Tag>(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnOnce(
            &mut CanonicalRecordWriter,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        if read_canonical_source_records(source, source_ref).is_none() {
            return Err(StoreRuntimeError::InvalidSourceSlot);
        }
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let mut callback_cause = None;
        let result = self
            .raw
            .loan_mut()
            .try_retain_rewritten_canonical_controlled(
                source.aggregate(),
                source_ref.raw,
                work_dimension,
                allocation,
                |emplacer, raw| {
                    let mut writer = CanonicalRecordWriter::new(plan)?;
                    let mut control = RetainedStoreWorkControl { raw, sticky: None };
                    let callback = build(&mut writer, &mut control);
                    if let Err(cause) = control.finish_build(callback) {
                        callback_cause = Some(cause);
                        return Err(RetainedBuildFailureCode::new(BUILD_CODE_DOMAIN));
                    }
                    Ok(emplacer.place(writer.finish()?, tag))
                },
            );
        self.map_controlled_action(result, callback_cause)
            .map(|raw| CanonicalStoreRef {
                raw,
                marker: PhantomData,
            })
    }

    /// Retains one outcome-evidence fixed-width record block.
    pub fn try_retain_outcome_evidence_records<Tag>(
        &mut self,
        plan: RecordPlan,
        tag: Tag,
        build: impl FnOnce(&mut OutcomeEvidenceRecordWriter) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_evidence_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
            .loan_mut()
            .try_retain_outcome_evidence(allocation, |emplacer| {
                let writer = OutcomeEvidenceRecordWriter::new(plan)?;
                let block =
                    finish_scoped_writer(writer, build, OutcomeEvidenceRecordWriter::finish)?;
                Ok(emplacer.place(block, tag))
            });
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Shares one canonical same-account block and returns a fresh destination reference.
    pub(super) fn try_share_canonical_from<SlotKind, SlotTag>(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        source_ref: CanonicalStoreRef<SlotKind, SlotTag>,
    ) -> Result<CanonicalStoreRef<SlotKind, SlotTag>, StoreRuntimeError>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let result = self
            .raw
            .loan_mut()
            .try_share_canonical(source.aggregate(), source_ref.raw);
        self.map_action(result).map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Shares one outcome-evidence same-account block and returns a fresh destination reference.
    pub fn try_share_outcome_evidence<SourceRootKind, SourceRootTag, SlotKind, SlotTag>(
        &mut self,
        source: &PublishedOutcomeEvidenceStore<SourceRootKind, SourceRootTag>,
        source_ref: OutcomeEvidenceStoreRef<SlotKind, SlotTag>,
    ) -> Result<OutcomeEvidenceStoreRef<SlotKind, SlotTag>, StoreRuntimeError>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        self.ensure_evidence_writable()?;
        let result = self
            .raw
            .loan_mut()
            .try_share_outcome_evidence(source.share_source().aggregate(), source_ref.raw);
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Adds one canonical destination commitment.
    pub fn try_commit_canonical(
        &mut self,
        dimension: StoreDimensionKey,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_writable()?;
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        let result = self.raw.loan_mut().try_commit_canonical(dimension, amount);
        self.map_action(result)
    }

    /// Adds one outcome-evidence destination commitment.
    pub fn try_commit_outcome_evidence(
        &mut self,
        dimension: StoreDimensionKey,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_evidence_writable()?;
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        let result = self
            .raw
            .loan_mut()
            .try_commit_outcome_evidence(dimension, amount);
        self.map_action(result)
    }
}

impl<'op, O, PP, U> StorePublicationLoan<'op, O, PP, U, CanonicalInheritanceSource>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Binds the exact canonical source selected when this derived operation started.
    pub fn try_bind_canonical_source<SourceRootKind, SourceRootTag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        work_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.try_bind_canonical_source_from(source, work_dimension)
    }

    /// Inherits every eligible immutable leaf from the exact bound canonical source.
    pub fn try_inherit_all_canonical_leaves<SourceRootKind, SourceRootTag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        work_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.try_inherit_all_canonical_leaves_from(source, work_dimension)
    }

    /// Replaces one exact-source canonical sequence with newly constructed immutable storage.
    pub fn try_rewrite_canonical_sequence<SourceRootKind, SourceRootTag, T, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalSequenceRef<T, Tag>,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl FnOnce(&mut SequenceWriter<T>) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.try_rewrite_canonical_sequence_from(source, source_ref, plan, tag, build)
    }

    /// Replaces one exact-source canonical sequence with controlled polling and work charging.
    pub fn try_rewrite_canonical_sequence_controlled<SourceRootKind, SourceRootTag, T, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalSequenceRef<T, Tag>,
        plan: SequencePlan<T>,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnOnce(
            &mut SequenceWriter<T>,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.try_rewrite_canonical_sequence_controlled_from(
            source,
            source_ref,
            plan,
            work_dimension,
            tag,
            build,
        )
    }

    /// Replaces one exact-source canonical record block through a typed writer.
    pub fn try_rewrite_canonical_records_with<SourceRootKind, SourceRootTag, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        tag: Tag,
        build: impl FnOnce(&mut CanonicalRecordWriter) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.try_rewrite_canonical_records_with_from(source, source_ref, plan, tag, build)
    }

    /// Rewrites one exact-source canonical record block through the bound source map.
    pub fn try_rewrite_canonical_records_controlled<SourceRootKind, SourceRootTag, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.try_rewrite_canonical_records_controlled_from(
            source,
            source_ref,
            plan,
            work_dimension,
            tag,
        )
    }

    /// Rewrites one exact-source canonical record block as a mapped prefix and appends only the
    /// remaining rows through a controlled typed writer.
    ///
    /// `source_plan` must exactly describe the source block. `destination_plan` must use the same
    /// field width and have at least as many rows. Historical canonical references are rewritten
    /// through the bound source map before `append` receives the destination writer. Every copied
    /// source cell is polled and charged exactly once.
    #[allow(clippy::too_many_arguments)]
    pub fn try_rewrite_canonical_records_with_mapped_prefix_controlled<
        SourceRootKind,
        SourceRootTag,
        Tag,
    >(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalRecordRef<Tag>,
        source_plan: RecordPlan,
        destination_plan: RecordPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        append: impl for<'control> FnOnce(
            &mut CanonicalRecordWriter,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let source_records = read_canonical_source_records(source, source_ref)
            .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
        if source_records.rows() != source_plan.rows()
            || source_records.fields() != source_plan.fields()
        {
            return Err(StoreRuntimeError::Build(
                StoreBuildError::capacity_envelope_violated(),
            ));
        }
        if source_plan.fields != destination_plan.fields {
            return Err(StoreRuntimeError::Build(
                StoreBuildError::invalid_record_width(),
            ));
        }
        if source_plan.rows > destination_plan.rows {
            return Err(StoreRuntimeError::Build(
                StoreBuildError::capacity_exceeded(),
            ));
        }
        let allocation = destination_plan
            .allocation_plan()
            .map_err(StoreRuntimeError::Plan)?;
        let mut callback_cause = None;
        let result = self.raw.loan_mut().try_rewrite_canonical_controlled(
            source.aggregate(),
            source_ref.raw,
            work_dimension,
            allocation,
            |emplacer, source_block, mapping, raw| {
                let mut writer = CanonicalRecordWriter::new(destination_plan)?;
                let mut control = RetainedStoreWorkControl { raw, sticky: None };
                let callback = (|| -> Result<(), StoreControlledError> {
                    let storage = checked_source_record(source_block, source_plan)
                        .ok_or_else(StoreBuildError::capacity_envelope_violated)?;
                    for source_cell in storage.cells.iter().copied() {
                        control.poll()?;
                        control.charge_observed(1)?;
                        if writer.core.storage.cells.len() >= destination_plan.cells {
                            return Err(StoreBuildError::capacity_exceeded().into());
                        }
                        let cell = mapped_canonical_record_cell(source_cell, &mapping)
                            .ok_or_else(StoreBuildError::capacity_envelope_violated)?;
                        writer.core.storage.cells.push(cell);
                    }
                    append(&mut writer, &mut control)
                })();
                if let Err(cause) = control.finish_build(callback) {
                    callback_cause = Some(cause);
                    return Err(RetainedBuildFailureCode::new(BUILD_CODE_DOMAIN));
                }
                Ok(emplacer.place(writer.finish()?, tag))
            },
        );
        self.map_controlled_action(result, callback_cause)
            .map(|raw| CanonicalStoreRef {
                raw,
                marker: PhantomData,
            })
    }

    /// Replaces one exact-source canonical record block with controlled authored output.
    pub fn try_rewrite_canonical_records_with_controlled<SourceRootKind, SourceRootTag, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnOnce(
            &mut CanonicalRecordWriter,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.try_rewrite_canonical_records_with_controlled_from(
            source,
            source_ref,
            plan,
            work_dimension,
            tag,
            build,
        )
    }

    /// Shares one exact-source canonical immutable leaf into a fresh destination slot.
    pub fn try_share_canonical<SourceRootKind, SourceRootTag, SlotKind, SlotTag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalStoreRef<SlotKind, SlotTag>,
    ) -> Result<CanonicalStoreRef<SlotKind, SlotTag>, StoreRuntimeError>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        let source = source.share_source();
        self.require_canonical_source_from(source)?;
        self.try_share_canonical_from(source, source_ref)
    }
}
