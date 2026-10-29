use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super::super::validation::{
    StoreCanonicalMemberCompletion, StoreCanonicalMemberVisit,
    StoreOutcomeEvidenceMemberCompletion, StoreOutcomeEvidenceMemberVisit,
};
use super::super::{
    ByteBlock, CanonicalRecordView, CanonicalStoreRef, OutcomeEvidenceRecordView,
    OutcomeEvidenceStoreRef, RecordBlock, RetainedStoreWorkControl, ScalarSequence,
    StoreRuntimeError, StoreValidationSession, StoreWorkError, StoredScalar, Utf8Block,
};
use super::work_batch::CompletionPrefixBatch;
use super::{
    CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE, CanonicalReservation, CanonicalValidationDecision,
    CanonicalWorkPermit, drop_scoped_value, drop_validation_decision, try_begin_canonical_work,
};
use crate::{OperationOwner, ProgressPhase, ProgressUnit, StorePhaseKey};

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
/// Raw views and work completion are deliberately private. Every exposed visit polls control;
/// inspectors read one concrete unit at a time, while completed prefixes are flushed to the
/// linear permit at bounded checkpoints and semantic boundaries.
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
    work_batch: CompletionPrefixBatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CanonicalValidationFirstFault {
    Contract,
    Runtime(StoreWorkError),
}

pub(super) enum CanonicalValidationAttempt<Proof, E> {
    Decision(CanonicalValidationDecision<Proof, E>),
    Runtime {
        cause: StoreRuntimeError,
        stable_code: u32,
    },
}

/// Failure while preparing bounded central-validation scratch before validation work starts.
pub enum CanonicalValidationPreparationError {
    Work(StoreWorkError),
    Runtime {
        cause: StoreRuntimeError,
        stable_code: u32,
    },
}

impl CanonicalValidationPreparationError {
    /// Creates one trusted local-runtime rejection with its bounded domain code.
    pub const fn runtime(cause: StoreRuntimeError, stable_code: u32) -> Self {
        Self::Runtime { cause, stable_code }
    }
}

impl From<StoreWorkError> for CanonicalValidationPreparationError {
    fn from(cause: StoreWorkError) -> Self {
        Self::Work(cause)
    }
}

/// Role-bound scratch checkpoint available before central validation work admission.
pub struct CanonicalValidationCheckpoint<'borrow, 'scope, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    control: &'borrow mut RetainedStoreWorkControl<'operation, O, PP, U>,
    phase: StorePhaseKey,
    first_fault: &'borrow mut Option<CanonicalValidationFirstFault>,
    scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

/// One-shot central-validation planning stage that owns scratch preparation and permit admission.
pub struct CanonicalValidationPlanning<
    'session,
    'storage,
    'scope,
    'operation,
    O,
    PP,
    U,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
    Proof,
    E,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: Option<StoreValidationSession<'session, 'storage>>,
    control: Option<&'scope mut RetainedStoreWorkControl<'operation, O, PP, U>>,
    phase: StorePhaseKey,
    validation_work: NonZeroU64,
    canonical: CanonicalValidationMember<'session, CanonicalKind, CanonicalTag>,
    evidence: OutcomeEvidenceValidationMember<'session, EvidenceKind, EvidenceTag>,
    first_fault: &'scope mut Option<CanonicalValidationFirstFault>,
    attempt: &'scope mut Option<CanonicalValidationAttempt<Proof, E>>,
    exact: &'scope mut Option<bool>,
    resolved: bool,
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
    /// Returns a cache key confined to this validation session and schema.
    ///
    /// The index grants no lookup authority and must not be compared across sessions.
    pub fn index_in_session(self) -> usize
    where
        Tag: 'static,
    {
        self.raw.raw.index_in_set()
    }

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

fn preserve_planning_fault<T>(
    first_fault: &mut Option<CanonicalValidationFirstFault>,
    result: Result<T, StoreWorkError>,
) -> Result<T, StoreWorkError> {
    if let Err(cause) = &result {
        first_fault.get_or_insert_with(|| CanonicalValidationFirstFault::Runtime(cause.clone()));
    }
    result
}

impl<'borrow, 'scope, 'operation, O, PP, U>
    CanonicalValidationCheckpoint<'borrow, 'scope, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Polls control before a validation-scratch allocation or ownership action.
    pub fn poll(&mut self) -> Result<(), StoreWorkError> {
        preserve_planning_fault(self.first_fault, self.control.poll())
    }

    /// Reserves validation scratch in the operation-bound validation role.
    pub fn reserve(
        &mut self,
        bytes: u64,
    ) -> Result<CanonicalReservation<'scope, O>, StoreWorkError> {
        self.poll()?;
        let result = self
            .control
            .reserve(self.phase, bytes)
            .map(|raw| CanonicalReservation {
                raw,
                scope: PhantomData,
            });
        preserve_planning_fault(self.first_fault, result)
    }

    /// Resizes one validation-scratch reservation without exposing ledger keys.
    pub fn resize(
        &mut self,
        reservation: &mut CanonicalReservation<'scope, O>,
        bytes: u64,
    ) -> Result<(), StoreWorkError> {
        self.poll()?;
        preserve_planning_fault(
            self.first_fault,
            self.control.resize(&mut reservation.raw, bytes),
        )
    }

    /// Proves that one validation-scratch reservation belongs to this operation.
    pub fn validate_live_owner(
        &mut self,
        reservation: &CanonicalReservation<'scope, O>,
    ) -> Result<(), StoreWorkError> {
        self.poll()?;
        preserve_planning_fault(
            self.first_fault,
            self.control.validate_live_owner(&reservation.raw),
        )
    }
}

impl<
    'session,
    'storage,
    'scope,
    'operation,
    O,
    PP,
    U,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
    Proof,
    E,
>
    CanonicalValidationPlanning<
        'session,
        'storage,
        'scope,
        'operation,
        O,
        PP,
        U,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
        Proof,
        E,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        raw: StoreValidationSession<'session, 'storage>,
        control: &'scope mut RetainedStoreWorkControl<'operation, O, PP, U>,
        phase: StorePhaseKey,
        validation_work: NonZeroU64,
        canonical: CanonicalValidationMember<'session, CanonicalKind, CanonicalTag>,
        evidence: OutcomeEvidenceValidationMember<'session, EvidenceKind, EvidenceTag>,
        first_fault: &'scope mut Option<CanonicalValidationFirstFault>,
        attempt: &'scope mut Option<CanonicalValidationAttempt<Proof, E>>,
        exact: &'scope mut Option<bool>,
    ) -> Self {
        Self {
            raw: Some(raw),
            control: Some(control),
            phase,
            validation_work,
            canonical,
            evidence,
            first_fault,
            attempt,
            exact,
            resolved: false,
        }
    }

    /// Prepares bounded scratch, then starts and fully resolves one exact validation session.
    ///
    /// Scratch preparation runs before validation work admission. Scratch is destroyed before the
    /// validation permit resolves and before the typed attempt becomes visible to the outer state
    /// machine.
    pub fn validate_with<S>(
        mut self,
        build_scratch: impl FnOnce(
            &mut CanonicalValidationCheckpoint<'_, 'scope, 'operation, O, PP, U>,
        ) -> Result<S, CanonicalValidationPreparationError>,
        validate: impl for<'permit> FnOnce(
            &mut CanonicalValidationSession<'session, 'storage, 'permit, 'operation, O, PP, U>,
            CanonicalValidationMember<'session, CanonicalKind, CanonicalTag>,
            OutcomeEvidenceValidationMember<'session, EvidenceKind, EvidenceTag>,
            &mut S,
        ) -> Result<
            CanonicalValidationDecision<Proof, E>,
            StoreWorkError,
        >,
    ) -> Result<(), StoreWorkError> {
        let scratch = {
            let control = self
                .control
                .as_deref_mut()
                .expect("unresolved validation planning retains its control");
            let mut checkpoint = CanonicalValidationCheckpoint {
                control,
                phase: self.phase,
                first_fault: self.first_fault,
                scope: PhantomData,
            };
            build_scratch(&mut checkpoint)
        };
        let scratch = match scratch {
            Ok(scratch) => scratch,
            Err(CanonicalValidationPreparationError::Work(cause)) => {
                self.resolved = true;
                return Err(cause);
            }
            Err(CanonicalValidationPreparationError::Runtime { cause, stable_code }) => {
                if matches!(
                    cause,
                    StoreRuntimeError::Control(_) | StoreRuntimeError::Ledger(_)
                ) {
                    self.first_fault
                        .get_or_insert(CanonicalValidationFirstFault::Contract);
                    if let Some(control) = self.control.as_deref_mut() {
                        control.record_product_graph_failure(
                            CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE,
                        );
                    }
                    if let Some(raw) = self.raw.as_mut() {
                        raw.invalidate();
                    }
                    *self.exact = Some(false);
                    self.resolved = true;
                    return Ok(());
                }
                *self.attempt = Some(CanonicalValidationAttempt::Runtime { cause, stable_code });
                *self.exact = Some(false);
                self.resolved = true;
                return Ok(());
            }
        };

        let raw = self
            .raw
            .take()
            .expect("validation planning starts its session at most once");
        let control = self
            .control
            .take()
            .expect("validation planning starts its work permit at most once");
        let mut session = match CanonicalValidationSession::try_start(
            raw,
            control,
            self.validation_work,
            self.first_fault,
        ) {
            Ok(session) => session,
            Err(cause) => {
                drop(scratch);
                self.resolved = true;
                return Err(cause);
            }
        };

        let mut scratch = ManuallyDrop::new(scratch);
        let result = match catch_unwind(AssertUnwindSafe(|| {
            validate(&mut session, self.canonical, self.evidence, &mut *scratch)
        })) {
            Ok(result) => result,
            Err(payload) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(scratch));
                self.resolved = true;
                resume_unwind(payload)
            }
        };
        let decision = match result {
            Ok(decision) => ManuallyDrop::new(decision),
            Err(cause) => {
                if drop_scoped_value(ManuallyDrop::into_inner(scratch)).is_err() {
                    self.resolved = true;
                    panic!("canonical validation scratch destructor panicked");
                }
                let _ = session.finish(false);
                self.resolved = true;
                return Err(cause);
            }
        };
        if drop_scoped_value(ManuallyDrop::into_inner(scratch)).is_err() {
            let _ = drop_validation_decision(Some(ManuallyDrop::into_inner(decision)));
            self.resolved = true;
            panic!("canonical validation scratch destructor panicked");
        }
        let accepted = matches!(&*decision, CanonicalValidationDecision::Accept(_));
        let exact = match session.finish(accepted) {
            Ok(exact) => exact,
            Err(cause) => {
                drop(ManuallyDrop::into_inner(decision));
                self.resolved = true;
                return Err(cause);
            }
        };
        *self.attempt = Some(CanonicalValidationAttempt::Decision(
            ManuallyDrop::into_inner(decision),
        ));
        *self.exact = Some(exact);
        self.resolved = true;
        Ok(())
    }
}

impl<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, Proof, E> Drop
    for CanonicalValidationPlanning<
        '_,
        '_,
        '_,
        '_,
        O,
        PP,
        U,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
        Proof,
        E,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.resolved {
            return;
        }
        if std::thread::panicking() {
            return;
        }
        self.first_fault
            .get_or_insert(CanonicalValidationFirstFault::Contract);
        if let Some(control) = self.control.as_deref_mut() {
            control.record_product_graph_failure(CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE);
        }
        if let Some(raw) = self.raw.as_mut() {
            raw.invalidate();
        }
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
            work_batch: CompletionPrefixBatch::new(),
        })
    }

    fn flush_completed(&mut self) -> Result<(), StoreWorkError> {
        let Some(completed) = self.work_batch.take_completed() else {
            return Ok(());
        };
        let completion = self
            .permit
            .as_mut()
            .expect("an unresolved canonical validation retains its work permit")
            .complete_units(completed);
        self.preserve(completion)
    }

    fn poll_now(&mut self) -> Result<(), StoreWorkError> {
        self.flush_completed()?;
        let poll = self
            .permit
            .as_mut()
            .expect("an unresolved canonical validation retains its work permit")
            .poll();
        self.preserve(poll)?;
        self.work_batch.note_poll();
        Ok(())
    }

    fn reject_contract(&mut self) {
        let _ = self.flush_completed();
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

    fn reject_semantic(&mut self) -> Result<(), StoreWorkError> {
        self.flush_completed()?;
        self.rejected = true;
        Ok(())
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
        self.poll_now()
    }

    fn run_unit<R>(
        &mut self,
        inspect: impl FnOnce(&mut StoreValidationSession<'session, 'storage>) -> Option<R>,
    ) -> Result<Option<R>, StoreWorkError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        if self.work_batch.poll_due() {
            self.poll_now()?;
        }
        let permit_remaining = self
            .permit
            .as_ref()
            .expect("an unresolved canonical validation retains its work permit")
            .remaining_units();
        let Some(remaining) = self.work_batch.effective_remaining(permit_remaining) else {
            self.reject_contract();
            return Ok(None);
        };
        if remaining == 0 {
            self.reject_contract();
            return Ok(None);
        }
        let Some(value) = inspect(&mut self.raw) else {
            self.reject_contract();
            return Ok(None);
        };
        if self.work_batch.note_completed_unit().is_err() {
            self.reject_contract();
            return Ok(None);
        }
        Ok(Some(value))
    }

    /// Runs one non-read validation action under the exact central-validation work envelope.
    ///
    /// The action is not executed when cancellation, budget admission, or the sealed work envelope
    /// rejects it. A normally returned action advances the same bounded completed prefix used by
    /// typed canonical-member visits.
    pub fn run_validation_unit<R>(
        &mut self,
        action: impl FnOnce() -> R,
    ) -> Result<Option<R>, StoreWorkError> {
        self.run_unit(|_| Some(action()))
    }

    fn complete_canonical<Tag>(
        &mut self,
        completion: StoreCanonicalMemberCompletion<'storage, Tag>,
    ) -> Result<bool, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.flush_completed()?;
        if self.raw.complete_canonical_member(completion) {
            Ok(true)
        } else {
            self.reject_contract();
            Ok(false)
        }
    }

    fn complete_outcome_evidence<Tag>(
        &mut self,
        completion: StoreOutcomeEvidenceMemberCompletion<'storage, Tag>,
    ) -> Result<bool, StoreWorkError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.flush_completed()?;
        if self.raw.complete_outcome_evidence_member(completion) {
            Ok(true)
        } else {
            self.reject_contract();
            Ok(false)
        }
    }

    /// Visits every scalar in one canonical sequence exactly once.
    pub fn visit_canonical_sequence<T, Tag>(
        &mut self,
        member: CanonicalValidationMember<'session, ScalarSequence<T>, Tag>,
        expected_len: usize,
        inspect: impl FnMut(usize, T) -> bool,
    ) -> Result<bool, StoreWorkError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.validate_canonical_sequence(member, expected_len, inspect)
            .map(|values| values.is_some())
    }

    /// Borrows an immutable canonical sequence after its exact metered visit succeeds.
    ///
    /// No slice is returned until every pending scalar passes inspection and member completion
    /// succeeds. An already completed member still polls control and checks its length without
    /// repeating inspection or charging work. The slice borrows storage, so other validation
    /// actions can continue while it is read; it cannot outlive the canonical storage owner.
    pub fn validate_canonical_sequence<T, Tag>(
        &mut self,
        member: CanonicalValidationMember<'session, ScalarSequence<T>, Tag>,
        expected_len: usize,
        mut inspect: impl FnMut(usize, T) -> bool,
    ) -> Result<Option<&'storage [T]>, StoreWorkError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.poll_visit()?;
        let Some(visit) = self.raw.peek_canonical_sequence(member.raw) else {
            self.reject_contract();
            return Ok(None);
        };
        let (view, completion) = match visit {
            StoreCanonicalMemberVisit::Completed(view) => (view, None),
            StoreCanonicalMemberVisit::Pending { view, completion } => (view, Some(completion)),
        };
        if view.as_slice().len() != expected_len {
            self.reject_contract();
            return Ok(None);
        }
        let Some(completion) = completion else {
            return Ok(Some(view.as_slice()));
        };
        for index in 0..expected_len {
            let Some(accepted) =
                self.run_unit(|_raw| view.get(index).map(|value| inspect(index, value)))?
            else {
                return Ok(None);
            };
            if !accepted {
                self.reject_semantic()?;
                return Ok(None);
            }
        }
        Ok(self
            .complete_canonical(completion)?
            .then_some(view.as_slice()))
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
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_canonical(completion)
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
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_canonical(completion)
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
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_canonical(completion)
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
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_outcome_evidence(completion)
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
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_outcome_evidence(completion)
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
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_outcome_evidence(completion)
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
                self.reject_semantic()?;
                return Ok(false);
            }
        }
        self.complete_outcome_evidence(completion)
    }

    pub(super) fn finish(mut self, accepted: bool) -> Result<bool, StoreWorkError> {
        self.flush_completed()?;
        if let Some(cause) = self.sticky.take() {
            return Err(cause);
        }
        let accepted = accepted && !self.rejected;
        if accepted {
            self.poll_now()?;
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

impl<O, PP, U> Drop for CanonicalValidationSession<'_, '_, '_, '_, O, PP, U>
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
        let inspected = self
            .session
            .run_unit(|raw| Some(read(view, row, field).and_then(|value| inspect(raw, value))))?;
        let Some(value) = inspected else {
            return Ok(None);
        };
        self.visited |= 1_u64 << field;
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
        let inspected = self
            .session
            .run_unit(|raw| Some(read(view, row, field).and_then(|value| inspect(raw, value))))?;
        let Some(value) = inspected else {
            return Ok(None);
        };
        self.visited |= 1_u64 << field;
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
