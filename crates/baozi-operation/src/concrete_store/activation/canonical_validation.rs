use std::marker::PhantomData;
use std::num::NonZeroU64;

use super::super::validation::{
    StoreCanonicalMemberCompletion, StoreCanonicalMemberVisit,
    StoreOutcomeEvidenceMemberCompletion, StoreOutcomeEvidenceMemberVisit,
};
use super::super::{
    ByteBlock, CanonicalRecordView, CanonicalStoreRef, OutcomeEvidenceRecordView,
    OutcomeEvidenceStoreRef, RecordBlock, RetainedStoreWorkControl, ScalarSequence,
    StoreValidationSession, StoreWorkError, StoredScalar, Utf8Block,
};
use super::{
    CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE, CanonicalWorkPermit, try_begin_canonical_work,
};
use crate::{OperationOwner, ProgressPhase, ProgressUnit};

/// Session-scoped canonical member that can only be inspected through metered validation.
pub struct CanonicalValidationMember<'session, Kind, Tag> {
    raw: CanonicalStoreRef<Kind, Tag>,
    session: PhantomData<&'session mut ()>,
}

/// Session-scoped outcome-evidence member that can only be inspected through metered validation.
pub struct OutcomeEvidenceValidationMember<'session, Kind, Tag> {
    raw: OutcomeEvidenceStoreRef<Kind, Tag>,
    session: PhantomData<&'session mut ()>,
}

impl<Kind, Tag> Copy for CanonicalValidationMember<'_, Kind, Tag> {}

impl<Kind, Tag> Clone for CanonicalValidationMember<'_, Kind, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Kind, Tag> PartialEq for CanonicalValidationMember<'_, Kind, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<Kind, Tag> Eq for CanonicalValidationMember<'_, Kind, Tag> {}

impl<Kind, Tag> Copy for OutcomeEvidenceValidationMember<'_, Kind, Tag> {}

impl<Kind, Tag> Clone for OutcomeEvidenceValidationMember<'_, Kind, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Kind, Tag> PartialEq for OutcomeEvidenceValidationMember<'_, Kind, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<Kind, Tag> Eq for OutcomeEvidenceValidationMember<'_, Kind, Tag> {}

/// Metered central-validation session for one canonical and outcome-evidence graph.
///
/// Raw views and work completion are deliberately private. Every exposed visit polls control,
/// reads one concrete unit, invokes the inspector, and only then records that unit as consumed.
pub struct CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: StoreValidationSession<'session, 'storage>,
    permit: Option<CanonicalWorkPermit<'control, 'operation, O, PP, U>>,
    sticky: Option<StoreWorkError>,
    rejected: bool,
    first_fault: &'control mut Option<CanonicalValidationFirstFault>,
    contract_failed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CanonicalValidationFirstFault {
    Contract,
    Runtime(StoreWorkError),
}

/// Metered row inspector for one canonical fixed-width record.
pub struct CanonicalValidationRecordRow<'row, 'session, 'storage, 'control, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    session:
        &'row mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    view: CanonicalRecordView<'storage>,
    row: usize,
    fields: usize,
    visited: u64,
}

/// Metered row inspector for one outcome-evidence fixed-width record.
pub struct OutcomeEvidenceValidationRecordRow<
    'row,
    'session,
    'storage,
    'control,
    'operation,
    O,
    PP,
    U,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    session:
        &'row mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    view: OutcomeEvidenceRecordView<'storage>,
    row: usize,
    fields: usize,
    visited: u64,
}

impl<'session, Kind, Tag> CanonicalValidationMember<'session, Kind, Tag> {
    pub(super) const fn from_store(raw: CanonicalStoreRef<Kind, Tag>) -> Self {
        Self {
            raw,
            session: PhantomData,
        }
    }

    /// Tests whether this validation-scoped member identifies the supplied typed store slot.
    pub fn identifies(&self, slot: CanonicalStoreRef<Kind, Tag>) -> bool {
        self.raw == slot
    }
}

impl<'session, Kind, Tag> OutcomeEvidenceValidationMember<'session, Kind, Tag> {
    pub(super) const fn from_store(raw: OutcomeEvidenceStoreRef<Kind, Tag>) -> Self {
        Self {
            raw,
            session: PhantomData,
        }
    }

    /// Tests whether this validation-scoped member identifies the supplied typed store slot.
    pub fn identifies(&self, slot: OutcomeEvidenceStoreRef<Kind, Tag>) -> bool {
        self.raw == slot
    }
}

impl<'session, 'storage, 'control, 'operation, O, PP, U>
    CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn try_start(
        raw: StoreValidationSession<'session, 'storage>,
        control: &'control mut RetainedStoreWorkControl<'operation, O, PP, U>,
        work: NonZeroU64,
        first_fault: &'control mut Option<CanonicalValidationFirstFault>,
    ) -> Result<Self, StoreWorkError> {
        Ok(Self {
            raw,
            permit: Some(try_begin_canonical_work(control, work)?),
            sticky: None,
            rejected: false,
            first_fault,
            contract_failed: false,
        })
    }

    fn reject_contract(&mut self) {
        self.first_fault
            .get_or_insert(CanonicalValidationFirstFault::Contract);
        if !self.contract_failed
            && let Some(permit) = self.permit.as_mut()
        {
            permit.record_product_graph_failure(CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE);
        }
        self.rejected = true;
        self.contract_failed = true;
        self.raw.invalidate();
    }

    fn preserve<T>(&mut self, result: Result<T, StoreWorkError>) -> Result<T, StoreWorkError> {
        if let Err(cause) = &result {
            self.first_fault
                .get_or_insert_with(|| CanonicalValidationFirstFault::Runtime(cause.clone()));
            self.sticky.get_or_insert_with(|| cause.clone());
        }
        result
    }

    fn poll_visit(&mut self) -> Result<(), StoreWorkError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        let poll = self
            .permit
            .as_mut()
            .expect("an unresolved canonical validation retains its work permit")
            .poll();
        self.preserve(poll)
    }

    fn run_unit<R>(
        &mut self,
        inspect: impl FnOnce(&mut StoreValidationSession<'session, 'storage>) -> Option<R>,
    ) -> Result<Option<R>, StoreWorkError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        let remaining = self
            .permit
            .as_ref()
            .expect("an unresolved canonical validation retains its work permit")
            .remaining_units();
        if remaining == 0 {
            self.reject_contract();
            return Ok(None);
        }
        let poll = self
            .permit
            .as_mut()
            .expect("an unresolved canonical validation retains its work permit")
            .poll();
        self.preserve(poll)?;
        let Some(value) = inspect(&mut self.raw) else {
            self.reject_contract();
            return Ok(None);
        };
        let completion = self
            .permit
            .as_mut()
            .expect("an unresolved canonical validation retains its work permit")
            .complete_units(NonZeroU64::MIN);
        self.preserve(completion)?;
        Ok(Some(value))
    }

    fn complete_canonical<Tag>(
        &mut self,
        completion: StoreCanonicalMemberCompletion<'storage, Tag>,
    ) -> bool
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.raw.complete_canonical_member(completion) {
            true
        } else {
            self.reject_contract();
            false
        }
    }

    fn complete_outcome_evidence<Tag>(
        &mut self,
        completion: StoreOutcomeEvidenceMemberCompletion<'storage, Tag>,
    ) -> bool
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if self.raw.complete_outcome_evidence_member(completion) {
            true
        } else {
            self.reject_contract();
            false
        }
    }

    /// Visits every scalar in one canonical sequence exactly once.
    pub fn visit_canonical_sequence<T, Tag>(
        &mut self,
        member: CanonicalValidationMember<'session, ScalarSequence<T>, Tag>,
        expected_len: usize,
        mut inspect: impl FnMut(usize, T) -> bool,
    ) -> Result<bool, StoreWorkError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_canonical_sequence(member.raw) else {
            self.reject_contract();
            return Ok(false);
        };
        let (view, completion) = match visit {
            StoreCanonicalMemberVisit::Completed(view) => (view, None),
            StoreCanonicalMemberVisit::Pending { view, completion } => (view, Some(completion)),
        };
        if view.as_slice().len() != expected_len {
            self.reject_contract();
            return Ok(false);
        }
        let Some(completion) = completion else {
            return Ok(true);
        };
        for index in 0..expected_len {
            let Some(accepted) =
                self.run_unit(|_raw| view.get(index).map(|value| inspect(index, value)))?
            else {
                return Ok(false);
            };
            if !accepted {
                self.rejected = true;
                return Ok(false);
            }
        }
        Ok(self.complete_canonical(completion))
    }

    /// Visits every byte in one canonical binary block exactly once.
    pub fn visit_canonical_bytes<Tag>(
        &mut self,
        member: CanonicalValidationMember<'session, ByteBlock, Tag>,
        expected_len: usize,
        mut inspect: impl FnMut(usize, u8) -> bool,
    ) -> Result<bool, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_canonical_bytes(member.raw) else {
            self.reject_contract();
            return Ok(false);
        };
        let (view, completion) = match visit {
            StoreCanonicalMemberVisit::Completed(view) => (view, None),
            StoreCanonicalMemberVisit::Pending { view, completion } => (view, Some(completion)),
        };
        if view.len() != expected_len {
            self.reject_contract();
            return Ok(false);
        }
        let Some(completion) = completion else {
            return Ok(true);
        };
        for index in 0..expected_len {
            let Some(accepted) =
                self.run_unit(|_raw| view.get(index).copied().map(|value| inspect(index, value)))?
            else {
                return Ok(false);
            };
            if !accepted {
                self.rejected = true;
                return Ok(false);
            }
        }
        Ok(self.complete_canonical(completion))
    }

    /// Visits every encoded byte in one canonical UTF-8 block exactly once.
    pub fn visit_canonical_utf8_bytes<Tag>(
        &mut self,
        member: CanonicalValidationMember<'session, Utf8Block, Tag>,
        expected_len: usize,
        mut inspect: impl FnMut(usize, u8) -> bool,
    ) -> Result<bool, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_canonical_utf8(member.raw) else {
            self.reject_contract();
            return Ok(false);
        };
        let (view, completion) = match visit {
            StoreCanonicalMemberVisit::Completed(view) => (view, None),
            StoreCanonicalMemberVisit::Pending { view, completion } => (view, Some(completion)),
        };
        if view.len() != expected_len {
            self.reject_contract();
            return Ok(false);
        }
        let Some(completion) = completion else {
            return Ok(true);
        };
        for index in 0..expected_len {
            let Some(accepted) = self.run_unit(|_raw| {
                view.as_bytes()
                    .get(index)
                    .copied()
                    .map(|value| inspect(index, value))
            })?
            else {
                return Ok(false);
            };
            if !accepted {
                self.rejected = true;
                return Ok(false);
            }
        }
        Ok(self.complete_canonical(completion))
    }

    /// Visits every row and cell in one canonical record block exactly once.
    pub fn visit_canonical_records<Tag>(
        &mut self,
        member: CanonicalValidationMember<'session, RecordBlock, Tag>,
        expected_rows: usize,
        expected_fields: usize,
        mut inspect: impl for<'row> FnMut(
            usize,
            &mut CanonicalValidationRecordRow<
                'row,
                'session,
                'storage,
                'control,
                'operation,
                O,
                PP,
                U,
            >,
        ) -> Result<bool, StoreWorkError>,
    ) -> Result<bool, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_canonical_records(member.raw) else {
            self.reject_contract();
            return Ok(false);
        };
        let (view, completion) = match visit {
            StoreCanonicalMemberVisit::Completed(view) => (view, None),
            StoreCanonicalMemberVisit::Pending { view, completion } => (view, Some(completion)),
        };
        if view.rows() != expected_rows || view.fields() != expected_fields || expected_fields > 64
        {
            self.reject_contract();
            return Ok(false);
        }
        let Some(completion) = completion else {
            return Ok(true);
        };
        let expected = field_mask(expected_fields);
        for row_index in 0..expected_rows {
            let (accepted, visited) = {
                let mut row = CanonicalValidationRecordRow {
                    session: self,
                    view,
                    row: row_index,
                    fields: expected_fields,
                    visited: 0,
                };
                let accepted = inspect(row_index, &mut row)?;
                (accepted, row.visited)
            };
            if visited != expected {
                self.reject_contract();
                return Ok(false);
            }
            if !accepted {
                self.rejected = true;
                return Ok(false);
            }
        }
        Ok(self.complete_canonical(completion))
    }

    /// Visits every scalar in one outcome-evidence sequence exactly once.
    pub fn visit_outcome_evidence_sequence<T, Tag>(
        &mut self,
        member: OutcomeEvidenceValidationMember<'session, ScalarSequence<T>, Tag>,
        expected_len: usize,
        mut inspect: impl FnMut(usize, T) -> bool,
    ) -> Result<bool, StoreWorkError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_outcome_evidence_sequence(member.raw) else {
            self.reject_contract();
            return Ok(false);
        };
        let (view, completion) = match visit {
            StoreOutcomeEvidenceMemberVisit::Completed(view) => (view, None),
            StoreOutcomeEvidenceMemberVisit::Pending { view, completion } => {
                (view, Some(completion))
            }
        };
        if view.as_slice().len() != expected_len {
            self.reject_contract();
            return Ok(false);
        }
        let Some(completion) = completion else {
            return Ok(true);
        };
        for index in 0..expected_len {
            let Some(accepted) =
                self.run_unit(|_raw| view.get(index).map(|value| inspect(index, value)))?
            else {
                return Ok(false);
            };
            if !accepted {
                self.rejected = true;
                return Ok(false);
            }
        }
        Ok(self.complete_outcome_evidence(completion))
    }

    /// Visits every byte in one outcome-evidence binary block exactly once.
    pub fn visit_outcome_evidence_bytes<Tag>(
        &mut self,
        member: OutcomeEvidenceValidationMember<'session, ByteBlock, Tag>,
        expected_len: usize,
        mut inspect: impl FnMut(usize, u8) -> bool,
    ) -> Result<bool, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_outcome_evidence_bytes(member.raw) else {
            self.reject_contract();
            return Ok(false);
        };
        let (view, completion) = match visit {
            StoreOutcomeEvidenceMemberVisit::Completed(view) => (view, None),
            StoreOutcomeEvidenceMemberVisit::Pending { view, completion } => {
                (view, Some(completion))
            }
        };
        if view.len() != expected_len {
            self.reject_contract();
            return Ok(false);
        }
        let Some(completion) = completion else {
            return Ok(true);
        };
        for index in 0..expected_len {
            let Some(accepted) =
                self.run_unit(|_raw| view.get(index).copied().map(|value| inspect(index, value)))?
            else {
                return Ok(false);
            };
            if !accepted {
                self.rejected = true;
                return Ok(false);
            }
        }
        Ok(self.complete_outcome_evidence(completion))
    }

    /// Visits every encoded byte in one outcome-evidence UTF-8 block exactly once.
    pub fn visit_outcome_evidence_utf8_bytes<Tag>(
        &mut self,
        member: OutcomeEvidenceValidationMember<'session, Utf8Block, Tag>,
        expected_len: usize,
        mut inspect: impl FnMut(usize, u8) -> bool,
    ) -> Result<bool, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_outcome_evidence_utf8(member.raw) else {
            self.reject_contract();
            return Ok(false);
        };
        let (view, completion) = match visit {
            StoreOutcomeEvidenceMemberVisit::Completed(view) => (view, None),
            StoreOutcomeEvidenceMemberVisit::Pending { view, completion } => {
                (view, Some(completion))
            }
        };
        if view.len() != expected_len {
            self.reject_contract();
            return Ok(false);
        }
        let Some(completion) = completion else {
            return Ok(true);
        };
        for index in 0..expected_len {
            let Some(accepted) = self.run_unit(|_raw| {
                view.as_bytes()
                    .get(index)
                    .copied()
                    .map(|value| inspect(index, value))
            })?
            else {
                return Ok(false);
            };
            if !accepted {
                self.rejected = true;
                return Ok(false);
            }
        }
        Ok(self.complete_outcome_evidence(completion))
    }

    /// Visits every row and cell in one outcome-evidence record block exactly once.
    pub fn visit_outcome_evidence_records<Tag>(
        &mut self,
        member: OutcomeEvidenceValidationMember<'session, RecordBlock, Tag>,
        expected_rows: usize,
        expected_fields: usize,
        mut inspect: impl for<'row> FnMut(
            usize,
            &mut OutcomeEvidenceValidationRecordRow<
                'row,
                'session,
                'storage,
                'control,
                'operation,
                O,
                PP,
                U,
            >,
        ) -> Result<bool, StoreWorkError>,
    ) -> Result<bool, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_outcome_evidence_records(member.raw) else {
            self.reject_contract();
            return Ok(false);
        };
        let (view, completion) = match visit {
            StoreOutcomeEvidenceMemberVisit::Completed(view) => (view, None),
            StoreOutcomeEvidenceMemberVisit::Pending { view, completion } => {
                (view, Some(completion))
            }
        };
        if view.rows() != expected_rows || view.fields() != expected_fields || expected_fields > 64
        {
            self.reject_contract();
            return Ok(false);
        }
        let Some(completion) = completion else {
            return Ok(true);
        };
        let expected = field_mask(expected_fields);
        for row_index in 0..expected_rows {
            let (accepted, visited) = {
                let mut row = OutcomeEvidenceValidationRecordRow {
                    session: self,
                    view,
                    row: row_index,
                    fields: expected_fields,
                    visited: 0,
                };
                let accepted = inspect(row_index, &mut row)?;
                (accepted, row.visited)
            };
            if visited != expected {
                self.reject_contract();
                return Ok(false);
            }
            if !accepted {
                self.rejected = true;
                return Ok(false);
            }
        }
        Ok(self.complete_outcome_evidence(completion))
    }

    pub(super) fn finish(mut self, accepted: bool) -> Result<bool, StoreWorkError> {
        if let Some(cause) = self.sticky.take() {
            return Err(cause);
        }
        let accepted = accepted && !self.rejected;
        if accepted {
            let poll = self
                .permit
                .as_mut()
                .expect("a finished canonical validation retains its work permit")
                .poll();
            self.preserve(poll)?;
        }
        let mut permit = self
            .permit
            .take()
            .expect("a finished canonical validation retains its work permit");
        if accepted {
            Ok(permit.finish_exact())
        } else {
            permit.resolve_partial();
            Ok(false)
        }
    }
}

impl<'row, 'session, 'storage, 'control, 'operation, O, PP, U>
    CanonicalValidationRecordRow<'row, 'session, 'storage, 'control, 'operation, O, PP, U>
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

    fn inspect_cell<V, R>(
        &mut self,
        field: usize,
        read: impl FnOnce(CanonicalRecordView<'storage>, usize, usize) -> Option<V>,
        inspect: impl FnOnce(&mut StoreValidationSession<'session, 'storage>, V) -> Option<R>,
    ) -> Result<Option<R>, StoreWorkError> {
        if field >= self.fields || (self.visited & (1_u64 << field)) != 0 {
            self.session.reject_contract();
            return Ok(None);
        }
        let view = self.view;
        let row = self.row;
        let value = self.session.run_unit(|raw| {
            let value = read(view, row, field)?;
            inspect(raw, value)
        })?;
        if value.is_some() {
            self.visited |= 1_u64 << field;
        }
        Ok(value)
    }

    pub fn scalar<T, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(T) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        T: StoredScalar,
    {
        self.inspect_cell(field, CanonicalRecordView::scalar::<T>, |_raw, value| {
            Some(inspect(value))
        })
    }

    pub fn sequence_ref<T, Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(CanonicalValidationMember<'session, ScalarSequence<T>, Tag>) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            CanonicalRecordView::sequence_ref::<T, Tag>,
            |raw, slot| {
                raw.peek_canonical_sequence(slot)?;
                Some(inspect(CanonicalValidationMember::from_store(slot)))
            },
        )
    }

    pub fn bytes_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(CanonicalValidationMember<'session, ByteBlock, Tag>) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(field, CanonicalRecordView::bytes_ref::<Tag>, |raw, slot| {
            raw.peek_canonical_bytes(slot)?;
            Some(inspect(CanonicalValidationMember::from_store(slot)))
        })
    }

    pub fn utf8_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(CanonicalValidationMember<'session, Utf8Block, Tag>) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(field, CanonicalRecordView::utf8_ref::<Tag>, |raw, slot| {
            raw.peek_canonical_utf8(slot)?;
            Some(inspect(CanonicalValidationMember::from_store(slot)))
        })
    }

    pub fn record_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(CanonicalValidationMember<'session, RecordBlock, Tag>) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            CanonicalRecordView::record_ref::<Tag>,
            |raw, slot| {
                raw.peek_canonical_records(slot)?;
                Some(inspect(CanonicalValidationMember::from_store(slot)))
            },
        )
    }
}

impl<'row, 'session, 'storage, 'control, 'operation, O, PP, U>
    OutcomeEvidenceValidationRecordRow<'row, 'session, 'storage, 'control, 'operation, O, PP, U>
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

    fn inspect_cell<V, R>(
        &mut self,
        field: usize,
        read: impl FnOnce(OutcomeEvidenceRecordView<'storage>, usize, usize) -> Option<V>,
        inspect: impl FnOnce(&mut StoreValidationSession<'session, 'storage>, V) -> Option<R>,
    ) -> Result<Option<R>, StoreWorkError> {
        if field >= self.fields || (self.visited & (1_u64 << field)) != 0 {
            self.session.reject_contract();
            return Ok(None);
        }
        let view = self.view;
        let row = self.row;
        let value = self.session.run_unit(|raw| {
            let value = read(view, row, field)?;
            inspect(raw, value)
        })?;
        if value.is_some() {
            self.visited |= 1_u64 << field;
        }
        Ok(value)
    }

    pub fn scalar<T, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(T) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        T: StoredScalar,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::scalar::<T>,
            |_raw, value| Some(inspect(value)),
        )
    }

    pub fn sequence_ref<T, Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(OutcomeEvidenceValidationMember<'session, ScalarSequence<T>, Tag>) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::sequence_ref::<T, Tag>,
            |raw, slot| {
                raw.peek_outcome_evidence_sequence(slot)?;
                Some(inspect(OutcomeEvidenceValidationMember::from_store(slot)))
            },
        )
    }

    pub fn bytes_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(OutcomeEvidenceValidationMember<'session, ByteBlock, Tag>) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::bytes_ref::<Tag>,
            |raw, slot| {
                raw.peek_outcome_evidence_bytes(slot)?;
                Some(inspect(OutcomeEvidenceValidationMember::from_store(slot)))
            },
        )
    }

    pub fn utf8_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(OutcomeEvidenceValidationMember<'session, Utf8Block, Tag>) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::utf8_ref::<Tag>,
            |raw, slot| {
                raw.peek_outcome_evidence_utf8(slot)?;
                Some(inspect(OutcomeEvidenceValidationMember::from_store(slot)))
            },
        )
    }

    pub fn record_ref<Tag, R>(
        &mut self,
        field: usize,
        inspect: impl FnOnce(OutcomeEvidenceValidationMember<'session, RecordBlock, Tag>) -> R,
    ) -> Result<Option<R>, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inspect_cell(
            field,
            OutcomeEvidenceRecordView::record_ref::<Tag>,
            |raw, slot| {
                raw.peek_outcome_evidence_records(slot)?;
                Some(inspect(OutcomeEvidenceValidationMember::from_store(slot)))
            },
        )
    }
}

const fn field_mask(fields: usize) -> u64 {
    if fields == 64 {
        u64::MAX
    } else {
        (1_u64 << fields) - 1
    }
}
