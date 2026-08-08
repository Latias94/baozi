use super::super::validation::{StoreEvidenceMemberCompletion, StoreEvidenceMemberVisit};
use super::validation_batch::ValidationWorkBatch;
use super::*;

impl CanonicalFailureTerminalWorkPlan {
    /// Qualifies caller policy ceilings into the protected requirement inside the total work cap.
    ///
    /// The member count and work limits bound later operation-derived plans; they are not accepted
    /// as evidence that a producer performed any particular amount of work.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dimension: StoreDimensionKey,
        max_evidence_members: usize,
        construction_work_limit: u64,
        validation_work_limit: u64,
        total_attempted_work_limit: u64,
        total_consumed_work_limit: u64,
    ) -> Result<Self, StorePlanError> {
        let terminal_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
            dimension,
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
            total_attempted_work_limit,
            total_consumed_work_limit,
        )?;
        Ok(Self { terminal_work })
    }

    pub const fn requirement(&self) -> &RetainedStoreFailureTerminalWorkRequirement {
        self.terminal_work.requirement()
    }

    pub const fn ordinary_attempted_work_limit(&self) -> u64 {
        self.terminal_work.ordinary_attempted_work_limit()
    }

    pub const fn ordinary_consumed_work_limit(&self) -> u64 {
        self.terminal_work.ordinary_consumed_work_limit()
    }

    pub fn into_store_plan(self) -> RetainedStoreFailureTerminalWorkPlan {
        self.terminal_work
    }
}

impl<O, PP, U> OutcomeEvidenceWorkPermit<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn record_product_graph_failure(&mut self, code: u32) {
        match self {
            Self::Ordinary(permit) => permit.record_product_graph_failure(code),
            Self::Protected(permit) => permit.record_product_graph_failure(code),
        }
    }

    fn poll(&mut self) -> Result<(), StoreRuntimeError> {
        match self {
            Self::Ordinary(permit) => permit.poll().map_err(StoreRuntimeError::Control),
            Self::Protected(permit) => permit.poll().map_err(StoreRuntimeError::Control),
        }
    }

    const fn remaining_units(&self) -> u64 {
        match self {
            Self::Ordinary(permit) => permit.remaining_units(),
            Self::Protected(permit) => permit.remaining_units(),
        }
    }

    fn complete_units(&mut self, amount: NonZeroU64) -> Result<(), StoreRuntimeError> {
        match self {
            Self::Ordinary(permit) => permit.complete_units(amount).map_err(|cause| match cause {
                RetainedWorkConsumeError::Ledger(cause) => StoreRuntimeError::Ledger(cause),
                RetainedWorkConsumeError::ExceedsAttempted => failure_work_envelope_exhausted(),
            }),
            Self::Protected(permit) => permit.complete_units(amount).map_err(|cause| match cause {
                RetainedFailureWorkConsumeError::Ledger(cause) => StoreRuntimeError::Ledger(cause),
                RetainedFailureWorkConsumeError::ExceedsAttempted => {
                    failure_work_envelope_exhausted()
                }
            }),
        }
    }

    fn complete(self) -> Result<(), StoreRuntimeError> {
        match self {
            Self::Ordinary(permit) => permit
                .complete()
                .map(|_| ())
                .map_err(|RetainedWorkFinishError::Incomplete| failure_work_envelope_exhausted()),
            Self::Protected(permit) => permit.complete().map(|_| ()).map_err(
                |RetainedFailureWorkFinishError::Incomplete| failure_work_envelope_exhausted(),
            ),
        }
    }

    fn finish_partial(self) {
        match self {
            Self::Ordinary(permit) => permit.finish_partial(),
            Self::Protected(permit) => permit.finish_partial(),
        }
    }
}

fn evidence_build_failure_code(cause: StoreBuildError) -> u32 {
    if cause.code() == 0 {
        ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE
    } else {
        cause.code()
    }
}

impl<O, PP, U, W> OutcomeEvidenceBuildControl<'_, O, PP, U, W>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn try_start_protected<'control>(
        mut raw: RawFailureEvidenceWorkControl<'control, O, PP, U>,
        construction_work: u64,
        record_action_work: u64,
        create_writer: impl FnOnce() -> Result<W, StoreBuildError>,
    ) -> Result<OutcomeEvidenceBuildControl<'control, O, PP, U, W>, EvidenceBuildCallbackError>
    {
        if construction_work == 0 || record_action_work == 0 {
            raw.record_product_graph_failure(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return Err(EvidenceBuildCallbackError::Runtime(
                failure_work_envelope_exhausted(),
            ));
        }
        let amount = NonZeroU64::new(construction_work)
            .expect("nonzero construction work was checked before admission");
        let permit = raw.try_begin(amount).map_err(|cause| {
            let cause = match cause {
                RetainedFailureWorkStartError::Control(cause) => StoreRuntimeError::Control(cause),
                RetainedFailureWorkStartError::Ledger(cause) => StoreRuntimeError::Ledger(cause),
                RetainedFailureWorkStartError::Outstanding => failure_work_envelope_exhausted(),
            };
            EvidenceBuildCallbackError::Runtime(cause)
        })?;
        Self::from_permit(
            OutcomeEvidenceWorkPermit::Protected(permit),
            record_action_work,
            create_writer,
        )
    }

    pub(super) fn try_start_ordinary<'control>(
        raw: RawOutcomeEvidenceWorkControl<'control, O, PP, U>,
        construction_work: u64,
        record_action_work: u64,
        create_writer: impl FnOnce() -> Result<W, StoreBuildError>,
    ) -> Result<OutcomeEvidenceBuildControl<'control, O, PP, U, W>, EvidenceBuildCallbackError>
    {
        if construction_work == 0 || record_action_work == 0 {
            return Err(EvidenceBuildCallbackError::Runtime(
                failure_work_envelope_exhausted(),
            ));
        }
        let amount = NonZeroU64::new(construction_work)
            .expect("nonzero construction work was checked before admission");
        let permit = raw.try_begin(amount).map_err(|cause| {
            let cause = match cause {
                RetainedWorkStartError::Control(cause) => StoreRuntimeError::Control(cause),
                RetainedWorkStartError::Ledger(cause) => StoreRuntimeError::Ledger(cause),
                RetainedWorkStartError::Outstanding => failure_work_envelope_exhausted(),
            };
            EvidenceBuildCallbackError::Runtime(cause)
        })?;
        Self::from_permit(
            OutcomeEvidenceWorkPermit::Ordinary(permit),
            record_action_work,
            create_writer,
        )
    }

    fn from_permit<'control>(
        mut permit: OutcomeEvidenceWorkPermit<'control, O, PP, U>,
        record_action_work: u64,
        create_writer: impl FnOnce() -> Result<W, StoreBuildError>,
    ) -> Result<OutcomeEvidenceBuildControl<'control, O, PP, U, W>, EvidenceBuildCallbackError>
    {
        let writer = match create_writer() {
            Ok(writer) => writer,
            Err(cause) => {
                permit.record_product_graph_failure(evidence_build_failure_code(cause));
                permit.finish_partial();
                return Err(EvidenceBuildCallbackError::Build(cause));
            }
        };
        Ok(OutcomeEvidenceBuildControl {
            writer,
            permit: Some(permit),
            record_action_work,
            sticky: None,
        })
    }

    /// Polls cancellation and deadline state without exposing the underlying work dimension.
    pub fn poll(&mut self) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = &self.sticky {
            return Err(match cause {
                EvidenceBuildCallbackError::Build(cause) => StoreRuntimeError::Build(*cause),
                EvidenceBuildCallbackError::Runtime(cause) => cause.clone(),
            });
        }
        let result = self
            .permit
            .as_mut()
            .expect("a fault-free outcome-evidence writer retains its work permit")
            .poll();
        if let Err(cause) = &result {
            self.sticky
                .get_or_insert_with(|| EvidenceBuildCallbackError::Runtime(cause.clone()));
            self.resolve_partial();
        }
        result
    }

    fn record_build_failure(&mut self, cause: StoreBuildError) {
        if let Some(permit) = self.permit.as_mut() {
            permit.record_product_graph_failure(evidence_build_failure_code(cause));
        }
        self.sticky
            .get_or_insert(EvidenceBuildCallbackError::Build(cause));
    }

    fn record_runtime_failure(&mut self, cause: StoreRuntimeError) {
        if let StoreRuntimeError::Build(build) = &cause
            && let Some(permit) = self.permit.as_mut()
        {
            permit.record_product_graph_failure(evidence_build_failure_code(*build));
        }
        self.sticky
            .get_or_insert(EvidenceBuildCallbackError::Runtime(cause));
    }

    fn resolve_partial(&mut self) {
        if let Some(permit) = self.permit.take() {
            permit.finish_partial();
        }
    }

    fn perform<T>(
        &mut self,
        amount: u64,
        action: impl FnOnce(&mut W) -> Result<T, StoreBuildError>,
    ) -> Result<T, StoreBuildError> {
        if let Some(cause) = &self.sticky {
            return Err(match cause {
                EvidenceBuildCallbackError::Build(cause) => *cause,
                EvidenceBuildCallbackError::Runtime(_) => failure_work_build_sentinel(),
            });
        }
        if self.poll().is_err() {
            return Err(failure_work_build_sentinel());
        }
        if amount == 0 {
            return match action(&mut self.writer) {
                Ok(value) => Ok(value),
                Err(cause) => {
                    self.record_build_failure(cause);
                    self.resolve_partial();
                    Err(cause)
                }
            };
        }
        let remaining = self
            .permit
            .as_ref()
            .expect("a fault-free outcome-evidence writer retains its work permit")
            .remaining_units();
        if amount > remaining {
            let cause = failure_work_envelope_exhausted();
            self.record_runtime_failure(cause);
            self.resolve_partial();
            return Err(failure_work_build_sentinel());
        }
        let amount =
            NonZeroU64::new(amount).expect("zero-sized writes are handled before admission");

        let value = match action(&mut self.writer) {
            Ok(value) => value,
            Err(cause) => {
                self.record_build_failure(cause);
                self.resolve_partial();
                return Err(cause);
            }
        };
        let completion = self
            .permit
            .as_mut()
            .expect("a fault-free outcome-evidence writer retains its work permit")
            .complete_units(amount);
        match completion {
            Ok(()) => Ok(value),
            Err(cause) => {
                self.record_runtime_failure(cause);
                self.resolve_partial();
                Err(failure_work_build_sentinel())
            }
        }
    }

    pub(super) fn finish(
        self,
        callback: Result<(), StoreBuildError>,
        finish_writer: impl FnOnce(W) -> Result<super::StoredBlock, StoreBuildError>,
    ) -> Result<super::StoredBlock, EvidenceBuildCallbackError> {
        let Self {
            writer,
            mut permit,
            record_action_work: _,
            sticky,
        } = self;
        if let Some(cause) = sticky {
            if let Some(permit) = permit.take() {
                permit.finish_partial();
            }
            return Err(cause);
        }
        if let Err(cause) = callback {
            permit
                .as_mut()
                .expect("a callback failure retains its outcome-evidence work permit")
                .record_product_graph_failure(evidence_build_failure_code(cause));
            permit
                .take()
                .expect("a callback failure resolves its outcome-evidence work permit")
                .finish_partial();
            return Err(EvidenceBuildCallbackError::Build(cause));
        }
        let block = match finish_writer(writer) {
            Ok(block) => block,
            Err(cause) => {
                permit
                    .as_mut()
                    .expect("a writer failure retains its outcome-evidence work permit")
                    .record_product_graph_failure(evidence_build_failure_code(cause));
                permit
                    .take()
                    .expect("a writer failure resolves its outcome-evidence work permit")
                    .finish_partial();
                return Err(EvidenceBuildCallbackError::Build(cause));
            }
        };
        let mut permit =
            permit.expect("a completed outcome-evidence writer retains its work permit");
        if permit.remaining_units() != 0 {
            let cause = failure_work_envelope_exhausted();
            if let StoreRuntimeError::Build(build) = &cause {
                permit.record_product_graph_failure(evidence_build_failure_code(*build));
            }
            permit.finish_partial();
            return Err(EvidenceBuildCallbackError::Runtime(cause));
        }
        permit
            .complete()
            .map_err(EvidenceBuildCallbackError::Runtime)?;
        Ok(block)
    }
}

impl<O, PP, U, T> OutcomeEvidenceBuildControl<'_, O, PP, U, SequenceWriter<T>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    T: StoredScalar,
{
    /// Appends one scalar and accounts for exactly one completed element.
    pub fn push(&mut self, value: T) -> Result<(), StoreBuildError> {
        self.perform(1, |writer| writer.push(value))
    }

    /// Appends a slice in bounded, cancellation-aware chunks.
    pub fn extend_from_slice(&mut self, values: &[T]) -> Result<(), StoreBuildError> {
        for chunk in values.chunks(FAILURE_EVIDENCE_BUILD_CHUNK_UNITS) {
            let amount = u64::try_from(chunk.len())
                .expect("the fixed outcome-evidence chunk length fits in u64");
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

impl<O, PP, U> OutcomeEvidenceBuildControl<'_, O, PP, U, BytesWriter>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Appends bytes in bounded, cancellation-aware chunks.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), StoreBuildError> {
        for chunk in bytes.chunks(FAILURE_EVIDENCE_BUILD_CHUNK_UNITS) {
            let amount = u64::try_from(chunk.len())
                .expect("the fixed outcome-evidence chunk length fits in u64");
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

impl<O, PP, U> OutcomeEvidenceBuildControl<'_, O, PP, U, Utf8Writer>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Appends UTF-8 in bounded chunks without splitting a scalar value.
    pub fn write_str(&mut self, value: &str) -> Result<(), StoreBuildError> {
        let mut start = 0;
        while start < value.len() {
            let mut end = start
                .saturating_add(FAILURE_EVIDENCE_BUILD_CHUNK_UNITS)
                .min(value.len());
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            debug_assert!(
                end > start,
                "the chunk size exceeds the maximum UTF-8 scalar width"
            );
            let chunk = &value[start..end];
            let amount = u64::try_from(chunk.len())
                .expect("the fixed outcome-evidence chunk length fits in u64");
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

impl<O, PP, U> OutcomeEvidenceBuildControl<'_, O, PP, U, OutcomeEvidenceRecordWriter>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Writes one exact-width row under a permit sized to its admitted cell count.
    pub fn write_row(
        &mut self,
        write: impl FnOnce(&mut OutcomeEvidenceRecordRowWriter<'_>) -> Result<(), StoreBuildError>,
    ) -> Result<(), StoreBuildError> {
        self.perform(self.record_action_work, |writer| writer.write_row(write))
    }

    pub fn rows(&self) -> usize {
        self.writer.rows()
    }
}

pub(super) fn failure_work_build_sentinel() -> StoreBuildError {
    StoreBuildError::domain(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE)
        .expect("the failure-work envelope code is inside the domain range")
}

fn exact_failure_construction_work(units: usize) -> Result<u64, StoreRuntimeError> {
    u64::try_from(units).map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))
}

impl<'session, 'storage, 'control, O, PP, U>
    FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn flush_completed(&mut self) -> Result<(), StoreRuntimeError> {
        let Some(completed) = self.work_batch.take_completed() else {
            return Ok(());
        };
        let result = self
            .permit
            .as_mut()
            .expect("an unresolved failure validation retains its work permit")
            .complete_units(completed);
        match result {
            Ok(()) => {
                let Some(total) = self.completed_units.checked_add(completed.get()) else {
                    return Err(self.contract_failure());
                };
                self.completed_units = total;
                Ok(())
            }
            Err(RetainedFailureWorkConsumeError::Ledger(cause)) => {
                let cause = StoreRuntimeError::Ledger(cause);
                self.sticky.get_or_insert_with(|| cause.clone());
                Err(cause)
            }
            Err(RetainedFailureWorkConsumeError::ExceedsAttempted) => Err(self.contract_failure()),
        }
    }

    fn poll_now(&mut self) -> Result<(), StoreRuntimeError> {
        self.flush_completed()?;
        let poll = self
            .permit
            .as_mut()
            .expect("an unresolved failure validation retains its work permit")
            .poll()
            .map_err(StoreRuntimeError::Control);
        if let Err(cause) = &poll {
            self.sticky.get_or_insert_with(|| cause.clone());
        }
        poll?;
        self.work_batch.note_poll();
        Ok(())
    }

    fn reject_semantic(&mut self) -> Result<(), StoreRuntimeError> {
        self.flush_completed()?;
        self.rejected = true;
        Ok(())
    }

    fn contract_failure(&mut self) -> StoreRuntimeError {
        if let Some(cause) = self.sticky.as_ref().cloned() {
            return cause;
        }
        if let Err(cause) = self.flush_completed() {
            return cause;
        }
        if let Some(permit) = self.permit.as_mut() {
            permit.record_product_graph_failure(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
        }
        let cause = failure_work_envelope_exhausted();
        self.sticky.get_or_insert_with(|| cause.clone());
        cause
    }

    fn poll_visit(&mut self) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = self.sticky.as_ref().cloned() {
            return Err(cause);
        }
        self.poll_now()
    }

    fn complete_member<Tag>(
        &mut self,
        completion: StoreEvidenceMemberCompletion<'storage, Tag>,
    ) -> Result<(), StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.flush_completed()?;
        if self.raw.complete_member(completion) {
            Ok(())
        } else {
            Err(self.contract_failure())
        }
    }

    fn run_unit<R>(
        &mut self,
        inspect: impl FnOnce(&mut StoreEvidenceValidationSession<'session, 'storage>) -> Result<R, ()>,
    ) -> Result<R, StoreRuntimeError> {
        if self.work_batch.poll_due() {
            self.poll_now()?;
        }
        let remaining = self
            .permit
            .as_ref()
            .expect("an unresolved failure validation retains its work permit")
            .remaining_units();
        let Some(remaining) = self.work_batch.effective_remaining(remaining) else {
            return Err(self.contract_failure());
        };
        if remaining == 0 {
            return Err(self.contract_failure());
        }
        let value = match inspect(&mut self.raw) {
            Ok(value) => value,
            Err(()) => return Err(self.contract_failure()),
        };
        self.work_batch
            .note_completed_unit()
            .map_err(|()| self.contract_failure())?;
        Ok(value)
    }

    /// Visits every scalar element until the inspector returns false.
    pub fn visit_sequence<T, Tag>(
        &mut self,
        slot: FailureEvidenceSequenceMember<'session, T, Tag>,
        mut inspect: impl FnMut(usize, T) -> bool,
    ) -> Result<bool, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        if self.rejected {
            return Ok(false);
        }
        let Some(visit) = self.raw.peek_sequence(slot.raw) else {
            return Err(self.contract_failure());
        };
        let (view, completion) = match visit {
            StoreEvidenceMemberVisit::Completed => return Ok(true),
            StoreEvidenceMemberVisit::Pending { view, completion } => (view, completion),
        };
        for (index, value) in view.as_slice().iter().copied().enumerate() {
            if !self.run_unit(|_raw| Ok(inspect(index, value)))? {
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_member(completion)?;
        Ok(true)
    }

    /// Visits every binary byte until the inspector returns false.
    pub fn visit_bytes<Tag>(
        &mut self,
        slot: FailureEvidenceBytesMember<'session, Tag>,
        mut inspect: impl FnMut(usize, u8) -> bool,
    ) -> Result<bool, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        if self.rejected {
            return Ok(false);
        }
        let Some(visit) = self.raw.peek_bytes(slot.raw) else {
            return Err(self.contract_failure());
        };
        let (view, completion) = match visit {
            StoreEvidenceMemberVisit::Completed => return Ok(true),
            StoreEvidenceMemberVisit::Pending { view, completion } => (view, completion),
        };
        for (index, value) in view.iter().copied().enumerate() {
            if !self.run_unit(|_raw| Ok(inspect(index, value)))? {
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_member(completion)?;
        Ok(true)
    }

    /// Visits every encoded UTF-8 byte until the inspector returns false.
    pub fn visit_utf8_bytes<Tag>(
        &mut self,
        slot: FailureEvidenceUtf8Member<'session, Tag>,
        mut inspect: impl FnMut(usize, u8) -> bool,
    ) -> Result<bool, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        if self.rejected {
            return Ok(false);
        }
        let Some(visit) = self.raw.peek_utf8(slot.raw) else {
            return Err(self.contract_failure());
        };
        let (view, completion) = match visit {
            StoreEvidenceMemberVisit::Completed => return Ok(true),
            StoreEvidenceMemberVisit::Pending { view, completion } => (view, completion),
        };
        for (index, value) in view.as_bytes().iter().copied().enumerate() {
            if !self.run_unit(|_raw| Ok(inspect(index, value)))? {
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_member(completion)?;
        Ok(true)
    }

    /// Visits every fixed-width record row until the row inspector returns false.
    pub fn visit_records<Tag>(
        &mut self,
        slot: FailureEvidenceRecordMember<'session, Tag>,
        mut inspect: impl for<'row> FnMut(
            usize,
            &mut FailureEvidenceRecordRow<'row, 'session, 'storage, 'control, O, PP, U>,
        ) -> bool,
    ) -> Result<bool, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        if self.rejected {
            return Ok(false);
        }
        let Some(visit) = self.raw.peek_records(slot.raw) else {
            return Err(self.contract_failure());
        };
        let (view, completion) = match visit {
            StoreEvidenceMemberVisit::Completed => return Ok(true),
            StoreEvidenceMemberVisit::Pending { view, completion } => (view, completion),
        };
        let fields = view.fields();
        let expected = if fields == 64 {
            u64::MAX
        } else {
            (1_u64 << fields) - 1
        };
        for row_index in 0..view.rows() {
            let (accepted, visited) = {
                let mut row = FailureEvidenceRecordRow {
                    session: self,
                    view,
                    row: row_index,
                    fields,
                    visited: 0,
                };
                let accepted = inspect(row_index, &mut row);
                (accepted, row.visited)
            };
            if visited != expected {
                return Err(self.contract_failure());
            }
            if !accepted {
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_member(completion)?;
        Ok(true)
    }

    fn finish(mut self, accepted: bool) -> Result<bool, StoreRuntimeError> {
        self.flush_completed()?;
        if let Some(cause) = self.sticky.as_ref().cloned() {
            return Err(cause);
        }
        if self.permit.is_none() || self.completed_units == 0 {
            return Err(self.contract_failure());
        }
        let accepted = accepted && !self.rejected;
        if accepted {
            self.poll_now()?;
        }
        let permit = self
            .permit
            .take()
            .expect("a finished validation retains its work permit");
        let result = permit.finish_validation(accepted).map_err(
            |RetainedFailureWorkFinishError::Incomplete| failure_work_envelope_exhausted(),
        );
        if let Err(cause) = &result {
            self.sticky.get_or_insert_with(|| cause.clone());
        }
        result?;
        Ok(accepted)
    }
}

impl<O, PP, U> Drop for FailureEvidenceValidationSession<'_, '_, '_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.permit.is_some() {
            let _ = self.flush_completed();
        }
    }
}

impl<'row, 'session, 'storage, 'control, O, PP, U>
    FailureEvidenceRecordRow<'row, 'session, 'storage, 'control, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn row(&self) -> usize {
        self.row
    }

    pub const fn fields(&self) -> usize {
        self.fields
    }

    fn inspect_cell<V, Resolved, R>(
        &mut self,
        field: usize,
        read: impl FnOnce(OutcomeEvidenceRecordView<'storage>, usize, usize) -> Option<V>,
        resolve: impl FnOnce(
            &mut StoreEvidenceValidationSession<'session, 'storage>,
            V,
        ) -> Option<Resolved>,
        inspect: impl FnOnce(Resolved) -> R,
    ) -> Result<R, StoreRuntimeError> {
        if field >= self.fields || (self.visited & (1_u64 << field)) != 0 {
            return Err(self.session.contract_failure());
        }
        let view = self.view;
        let row = self.row;
        let value = self.session.run_unit(|raw| {
            let value = read(view, row, field).ok_or(())?;
            let value = resolve(raw, value).ok_or(())?;
            Ok(inspect(value))
        });
        if value.is_ok() {
            self.visited |= 1_u64 << field;
        }
        value
    }

    /// Reads one exact scalar cell and inspects its value under the row's validation permit.
    pub fn scalar<T, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(T) -> R,
    ) -> Result<R, StoreRuntimeError>
    where
        T: StoredScalar,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::scalar::<T>,
            |_raw, value| Some(value),
            inspect,
        )
    }

    /// Resolves one in-set sequence edge before lending its typed slot to the inspector.
    pub fn sequence_ref<T, Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(FailureEvidenceSequenceMember<'session, T, Tag>) -> R,
    ) -> Result<R, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::sequence_ref::<T, Tag>,
            |raw, slot| {
                raw.peek_sequence(slot).map(|_| FailureEvidenceMember {
                    raw: slot,
                    validation: PhantomData,
                })
            },
            inspect,
        )
    }

    /// Resolves one in-set byte edge before lending its typed slot to the inspector.
    pub fn bytes_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(FailureEvidenceBytesMember<'session, Tag>) -> R,
    ) -> Result<R, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::bytes_ref::<Tag>,
            |raw, slot| {
                raw.peek_bytes(slot).map(|_| FailureEvidenceMember {
                    raw: slot,
                    validation: PhantomData,
                })
            },
            inspect,
        )
    }

    /// Resolves one in-set UTF-8 edge before lending its typed slot to the inspector.
    pub fn utf8_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(FailureEvidenceUtf8Member<'session, Tag>) -> R,
    ) -> Result<R, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::utf8_ref::<Tag>,
            |raw, slot| {
                raw.peek_utf8(slot).map(|_| FailureEvidenceMember {
                    raw: slot,
                    validation: PhantomData,
                })
            },
            inspect,
        )
    }

    /// Resolves one in-set record edge before lending its typed slot to the inspector.
    pub fn record_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(FailureEvidenceRecordMember<'session, Tag>) -> R,
    ) -> Result<R, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::record_ref::<Tag>,
            |raw, slot| {
                raw.peek_records(slot).map(|_| FailureEvidenceMember {
                    raw: slot,
                    validation: PhantomData,
                })
            },
            inspect,
        )
    }
}

impl<O, PP, U, E> RejectedCanonicalStoreSession<O, PP, U, E>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreCanonicalRejection<E> {
        &self.owner.cause
    }

    pub const fn canonical_len(&self) -> usize {
        0
    }

    pub fn outcome_evidence_len(&self) -> usize {
        self.owner.raw.outcome_evidence_len()
    }

    pub fn build_failure_evidence<EvidenceKind, EvidenceTag>(
        self,
        build: impl for<'lane> FnMut(
            &mut FailureEvidenceLane<'lane, O, PP, U>,
        ) -> FailureEvidenceDecision<EvidenceKind, EvidenceTag>,
    ) -> Result<
        CanonicalRejectionEvidenceOutcome<O, PP, U, E, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.owner.build(build)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreCanonicalRejection<E>>, StorePrepareError> {
        self.owner.prepare_minimal_fallback()
    }
}

struct FailureEvidenceMemberBuild<Tag, CreateWriter, FinishWriter> {
    allocation: crate::retained::RetainedAllocationPlan,
    construction_work: u64,
    action_work: u64,
    tag: Tag,
    missing_builder: &'static str,
    create_writer: CreateWriter,
    finish_writer: FinishWriter,
}

fn recover_failure_member_build<T>(
    result: Result<T, EvidenceBuildCallbackError>,
    controlled_failure: &mut Option<StoreRuntimeError>,
) -> Result<T, crate::retained::RetainedBuildFailureCode> {
    match result {
        Ok(value) => Ok(value),
        Err(EvidenceBuildCallbackError::Build(cause)) => Err(cause.into()),
        Err(EvidenceBuildCallbackError::Runtime(cause)) => {
            *controlled_failure = Some(cause);
            Err(failure_work_build_sentinel().into())
        }
    }
}

impl<O, PP, U> FailureEvidenceLane<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn try_retain_failure_member<Kind, W, Tag, Build, CreateWriter, FinishWriter>(
        &mut self,
        member: FailureEvidenceMemberBuild<Tag, CreateWriter, FinishWriter>,
        build: &mut ManuallyDrop<Option<Build>>,
    ) -> Result<OutcomeEvidenceStoreRef<Kind, Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
        Build: for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, W>,
        ) -> Result<(), StoreBuildError>,
        CreateWriter: FnOnce() -> Result<W, StoreBuildError>,
        FinishWriter: FnOnce(W) -> Result<StoredBlock, StoreBuildError>,
    {
        let FailureEvidenceMemberBuild {
            allocation,
            construction_work,
            action_work,
            tag,
            missing_builder,
            create_writer,
            finish_writer,
        } = member;
        let mut controlled_failure = None;
        let result = self
            .raw
            .try_retain_outcome_evidence_controlled(allocation, |emplacer, raw_control| {
                let mut control = recover_failure_member_build(
                    OutcomeEvidenceBuildControl::try_start_protected(
                        raw_control,
                        construction_work,
                        action_work,
                        create_writer,
                    ),
                    &mut controlled_failure,
                )?;
                let built = build.as_mut().expect(missing_builder)(&mut control);
                let block = recover_failure_member_build(
                    control.finish(built, finish_writer),
                    &mut controlled_failure,
                )?;
                Ok(emplacer.place(block, tag))
            })
            .map(|raw| OutcomeEvidenceStoreRef {
                raw,
                marker: PhantomData,
            })
            .map_err(|cause| map_publication_error(&cause));
        controlled_failure.map_or(result, Err)
    }

    pub fn try_retain_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, SequenceWriter<T>>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let construction_work = match exact_failure_construction_work(plan.element_count()) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        if let Err(cause) = self
            .failure_work
            .reserve_construction_member(construction_work)
        {
            self.raw
                .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let result: Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreRuntimeError> = self
            .try_retain_failure_member(
                FailureEvidenceMemberBuild {
                    allocation,
                    construction_work,
                    action_work: 1,
                    tag,
                    missing_builder:
                        "the failure-evidence sequence builder is executed at most once",
                    create_writer: || {
                        <T as super::super::sealed::StoredScalar>::new_writer(
                            plan.capacity,
                            plan.len,
                        )
                    },
                    finish_writer: SequenceWriter::finish,
                },
                &mut build,
            );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, BytesWriter>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let construction_work = match exact_failure_construction_work(plan.bytes()) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        if let Err(cause) = self
            .failure_work
            .reserve_construction_member(construction_work)
        {
            self.raw
                .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let result: Result<OutcomeEvidenceBytesRef<Tag>, StoreRuntimeError> = self
            .try_retain_failure_member(
                FailureEvidenceMemberBuild {
                    allocation,
                    construction_work,
                    action_work: 1,
                    tag,
                    missing_builder: "the failure-evidence bytes builder is executed at most once",
                    create_writer: || BytesWriter::new(plan),
                    finish_writer: BytesWriter::finish,
                },
                &mut build,
            );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, Utf8Writer>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let construction_work = match exact_failure_construction_work(plan.bytes()) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        if let Err(cause) = self
            .failure_work
            .reserve_construction_member(construction_work)
        {
            self.raw
                .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let result: Result<OutcomeEvidenceUtf8Ref<Tag>, StoreRuntimeError> = self
            .try_retain_failure_member(
                FailureEvidenceMemberBuild {
                    allocation,
                    construction_work,
                    action_work: 1,
                    tag,
                    missing_builder: "the failure-evidence UTF-8 builder is executed at most once",
                    create_writer: || Utf8Writer::new(plan),
                    finish_writer: Utf8Writer::finish,
                },
                &mut build,
            );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_records<Tag>(
        &mut self,
        plan: RecordPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, OutcomeEvidenceRecordWriter>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let cells = match plan.rows().checked_mul(plan.fields()) {
            Some(cells) => cells,
            None => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(
                        build,
                        Err(StoreRuntimeError::Plan(StorePlanError::SizeOverflow)),
                    ),
                );
            }
        };
        let construction_work = match exact_failure_construction_work(cells) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        let record_action_work = match exact_failure_construction_work(plan.fields()) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        if let Err(cause) = self
            .failure_work
            .reserve_construction_member(construction_work)
        {
            self.raw
                .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let result: Result<OutcomeEvidenceRecordRef<Tag>, StoreRuntimeError> = self
            .try_retain_failure_member(
                FailureEvidenceMemberBuild {
                    allocation,
                    construction_work,
                    action_work: record_action_work,
                    tag,
                    missing_builder: "the failure-evidence record builder is executed at most once",
                    create_writer: || OutcomeEvidenceRecordWriter::new(plan),
                    finish_writer: OutcomeEvidenceRecordWriter::finish,
                },
                &mut build,
            );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub const fn complete<Kind, Tag>(
        &mut self,
        root: OutcomeEvidenceStoreRef<Kind, Tag>,
    ) -> FailureEvidenceDecision<Kind, Tag> {
        FailureEvidenceDecision::Complete(root)
    }

    pub const fn abort<Kind, Tag>(&mut self) -> FailureEvidenceDecision<Kind, Tag> {
        FailureEvidenceDecision::Abort
    }
}

impl<O, PP, U, Cause, EvidenceKind, EvidenceTag>
    RootedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn outcome_evidence_len(&self) -> usize {
        self.failure.outcome_evidence_len()
    }

    pub fn validate_rich_failure(
        self,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        RootedFailureValidation<O, PP, U, Cause, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let mut validate = ManuallyDrop::new(Some(validate));
        let Self {
            failure,
            cause,
            failure_work,
            outcome_evidence_destination_dimension,
        } = self;
        let cause = ManuallyDrop::new(cause);
        let evidence_root = failure.evidence_root();
        let mut controlled_failure = None;
        let validation =
            failure.validate_rich_failure_controlled(evidence_root, |raw, root, raw_control| {
                let validation_work = match failure_work.planned_validation_work() {
                    Ok(work) => work,
                    Err(cause) => {
                        controlled_failure.get_or_insert(cause);
                        return false;
                    }
                };
                let permit = match raw_control.try_begin(validation_work) {
                    Ok(permit) => permit,
                    Err(cause) => {
                        let cause = match cause {
                            RetainedFailureWorkStartError::Control(cause) => {
                                StoreRuntimeError::Control(cause)
                            }
                            RetainedFailureWorkStartError::Ledger(cause) => {
                                StoreRuntimeError::Ledger(cause)
                            }
                            RetainedFailureWorkStartError::Outstanding => {
                                failure_work_envelope_exhausted()
                            }
                        };
                        controlled_failure.get_or_insert(cause);
                        return false;
                    }
                };
                let mut session = FailureEvidenceValidationSession {
                    raw,
                    permit: Some(permit),
                    sticky: &mut controlled_failure,
                    completed_units: 0,
                    rejected: false,
                    work_batch: ValidationWorkBatch::new(),
                };
                let validate = validate
                    .as_mut()
                    .expect("the rich-failure validation callback is executed at most once");
                let root = FailureEvidenceMember {
                    raw: root,
                    validation: PhantomData,
                };
                let accepted = validate(&mut session, root);
                match session.finish(accepted) {
                    Ok(accepted) => accepted,
                    Err(cause) => {
                        controlled_failure.get_or_insert(cause);
                        false
                    }
                }
            });
        if drop_optional_callback(validate).is_err() {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            let cause = match validation {
                StoreFailureValidation::Valid(validated) => validated.contain_activation_fault(),
                StoreFailureValidation::Rejected(failure) => failure.contain_activation_fault(),
                StoreFailureValidation::FallbackOnly(fallback) => {
                    fallback.contain_activation_fault()
                }
            };
            return Err(StoreActivationContainment { cause });
        }
        if let Some(runtime_cause) = controlled_failure {
            if drop_scoped_value(ManuallyDrop::into_inner(cause)).is_err() {
                let cause = match validation {
                    StoreFailureValidation::Valid(validated) => {
                        validated.contain_activation_fault()
                    }
                    StoreFailureValidation::Rejected(failure) => failure.contain_activation_fault(),
                    StoreFailureValidation::FallbackOnly(fallback) => {
                        fallback.contain_activation_fault()
                    }
                };
                return Err(StoreActivationContainment { cause });
            }
            let prepared = match validation {
                StoreFailureValidation::Valid(validated) => validated.prepare_minimal_fallback(),
                StoreFailureValidation::Rejected(failure) => failure.prepare_minimal_fallback(),
                StoreFailureValidation::FallbackOnly(fallback) => {
                    fallback.prepare_minimal_fallback()
                }
            };
            return match prepared {
                Ok(prepared) => Ok(RootedFailureValidation::Fallback(VerifiedMinimalFallback {
                    prepared,
                    cause: runtime_cause,
                })),
                Err(StorePrepareError::FinalizationContainment(cause)) => {
                    Err(StoreActivationContainment { cause })
                }
            };
        }
        match validation {
            StoreFailureValidation::Valid(mut validated) => {
                match validated
                    .try_stage_exact_outcome_evidence(outcome_evidence_destination_dimension)
                {
                    Ok(()) => Ok(RootedFailureValidation::Valid(ValidatedFailureEvidence {
                        validated,
                        cause: ManuallyDrop::into_inner(cause),
                    })),
                    Err(runtime_cause) => {
                        if matches!(
                            classify_activation_fault(&runtime_cause),
                            ActivationFaultClassification::Contract
                        ) {
                            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                            return Err(StoreActivationContainment {
                                cause: validated.contain_activation_fault(),
                            });
                        }
                        if drop_scoped_value(ManuallyDrop::into_inner(cause)).is_err() {
                            return Err(StoreActivationContainment {
                                cause: validated.contain_activation_fault(),
                            });
                        }
                        match validated.prepare_minimal_fallback() {
                            Ok(prepared) => {
                                Ok(RootedFailureValidation::Fallback(VerifiedMinimalFallback {
                                    prepared,
                                    cause: runtime_cause,
                                }))
                            }
                            Err(StorePrepareError::FinalizationContainment(cause)) => {
                                Err(StoreActivationContainment { cause })
                            }
                        }
                    }
                }
            }
            StoreFailureValidation::Rejected(failure) => {
                Ok(RootedFailureValidation::Rejected(RejectedFailureEvidence {
                    failure,
                    cause: ManuallyDrop::into_inner(cause),
                }))
            }
            StoreFailureValidation::FallbackOnly(fallback) => {
                let containment_required = fallback.activation_containment_required();
                if drop_scoped_value(ManuallyDrop::into_inner(cause)).is_err() {
                    return Err(StoreActivationContainment {
                        cause: fallback.contain_activation_fault(),
                    });
                }
                if containment_required {
                    return Err(StoreActivationContainment {
                        cause: fallback.contain_activation_fault(),
                    });
                }
                prepare_product_graph_fallback(fallback).map(RootedFailureValidation::Fallback)
            }
        }
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<Cause>, StorePrepareError> {
        let Self {
            failure,
            cause,
            failure_work: _,
            outcome_evidence_destination_dimension: _,
        } = self;
        let cause = ManuallyDrop::new(cause);
        let prepared = failure.prepare_minimal_fallback();
        match prepared {
            Ok(prepared) => Ok(VerifiedMinimalFallback {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }
}

impl<O, PP, U, Cause, EvidenceKind, EvidenceTag>
    ValidatedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn try_prepare(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedFailureEvidence<Cause, EvidenceKind, EvidenceTag>, StorePrepareError> {
        let Self { validated, cause } = self;
        let cause = ManuallyDrop::new(cause);
        let prepared = validated.try_prepare(0, final_progress);
        match prepared {
            Ok(prepared) => Ok(PreparedFailureEvidence {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<Cause>, StorePrepareError> {
        let Self { validated, cause } = self;
        let cause = ManuallyDrop::new(cause);
        let prepared = validated.prepare_minimal_fallback();
        match prepared {
            Ok(prepared) => Ok(VerifiedMinimalFallback {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }
}

impl<Cause, EvidenceKind, EvidenceTag> PreparedFailureEvidence<Cause, EvidenceKind, EvidenceTag> {
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn into_parts(
        self,
    ) -> (
        PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag>,
        Cause,
    ) {
        (self.prepared, self.cause)
    }
}

impl<O, PP, U, Cause, EvidenceKind, EvidenceTag>
    RejectedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<Cause>, StorePrepareError> {
        let Self { failure, cause } = self;
        let cause = ManuallyDrop::new(cause);
        match failure.prepare_minimal_fallback() {
            Ok(prepared) => Ok(VerifiedMinimalFallback {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }
}

impl<Cause> VerifiedMinimalFallback<Cause> {
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn into_parts(self) -> (PreparedStoreFallback, Cause) {
        (self.prepared, self.cause)
    }
}

impl<O, PP, U, Cause> UnrootedFailureEvidence<O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn prepare_minimal_fallback(self) -> Result<VerifiedMinimalFallback<Cause>, StorePrepareError> {
        let Self {
            raw,
            domain,
            failure_code: _,
            cause,
            failure_work: _,
            outcome_evidence_destination_dimension: _,
        } = self;
        let cause = ManuallyDrop::new(cause);
        match raw
            .prepare_minimal_fallback()
            .map(|prepared| classify_prepared_fallback(prepared, domain))
            .map_err(StorePrepareError::from_raw)
        {
            Ok(prepared) => Ok(VerifiedMinimalFallback {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }

    #[allow(clippy::type_complexity, clippy::result_large_err)]
    pub fn build<EvidenceKind, EvidenceTag>(
        self,
        build: impl for<'lane> FnMut(
            &mut FailureEvidenceLane<'lane, O, PP, U>,
        ) -> FailureEvidenceDecision<EvidenceKind, EvidenceTag>,
    ) -> Result<
        FailureEvidenceOutcome<O, PP, U, Cause, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let Self {
            raw,
            domain,
            failure_code,
            cause,
            mut failure_work,
            outcome_evidence_destination_dimension,
        } = self;
        let cause = ManuallyDrop::new(cause);
        let mut raw = raw;
        let mut build = ManuallyDrop::new(Some(build));
        let mut first_fault = None;
        let decision = catch_unwind(AssertUnwindSafe(|| {
            let mut lane = FailureEvidenceLane {
                raw: &mut raw,
                first_fault: &mut first_fault,
                failure_work: &mut failure_work,
            };
            let build = build
                .as_mut()
                .expect("the failure-evidence callback is executed at most once");
            build(&mut lane)
        }));
        let build_drop_failed = drop_optional_callback(build).is_err();
        let decision = match decision {
            Ok(decision) if !build_drop_failed => decision,
            Ok(decision) => {
                let _ = drop_scoped_value(decision);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
        };
        if let Some(fault) = first_fault {
            match decision {
                FailureEvidenceDecision::Abort if !fault.contract_seen => {
                    let Some(runtime_failure_code) = fault.recoverable_code() else {
                        let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                        return Err(StoreActivationContainment {
                            cause: raw.contain_activation_fault(),
                        });
                    };
                    let prepared = raw
                        .with_fallback_cause(runtime_failure_code)
                        .prepare_minimal_fallback()
                        .map(|prepared| classify_prepared_fallback(prepared, domain));
                    return match prepared {
                        Ok(prepared) => Ok(FailureEvidenceOutcome::MinimalFallback(
                            VerifiedMinimalFallback {
                                prepared,
                                cause: ManuallyDrop::into_inner(cause),
                            },
                        )),
                        Err(error) => {
                            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                            Err(StoreActivationContainment {
                                cause: map_raw_prepare_error(error),
                            })
                        }
                    };
                }
                FailureEvidenceDecision::Complete(_) | FailureEvidenceDecision::Abort => {
                    let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                    return Err(StoreActivationContainment {
                        cause: raw.contain_activation_fault(),
                    });
                }
            }
        }
        let evidence_root = match decision {
            FailureEvidenceDecision::Complete(root) => root,
            FailureEvidenceDecision::Abort => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
        };
        Ok(FailureEvidenceOutcome::Rooted(RootedFailureEvidence {
            failure: StoreFailureAssembly {
                raw,
                fallback_only: false,
                forced_cause: Some(failure_code),
                evidence_root,
                domain,
            },
            cause: ManuallyDrop::into_inner(cause),
            failure_work,
            outcome_evidence_destination_dimension,
        }))
    }
}
