use std::fmt;

use super::{
    CanonicalBytesRef, CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef,
    CanonicalUtf8Ref, OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef, OutcomeEvidenceRecordView,
    OutcomeEvidenceSequenceRef, OutcomeEvidenceUtf8Ref, SequenceView, StoreBuildError,
    StoreDimensionKey, StorePhaseKey, StorePlanError, StoredBlock, StoredScalar, sealed,
};
use crate::control::ControlCause;
use crate::ledger::{LedgerCause, PhaseCompletionError};
use crate::retained::{
    AccountPoisoned, RetainedAllocationError, RetainedClosureSession,
    RetainedEvidenceClosureSession, RetainedPublicationError, RetainedWriterError,
    retained_publication_error_requires_containment,
};
#[cfg(feature = "_u4-activation")]
use crate::retained::{RetainedEvidenceMemberVisit, RetainedPendingEvidenceMember};

pub(super) fn sequence_view<T: StoredScalar>(block: &StoredBlock) -> Option<SequenceView<'_, T>> {
    if !sequence_kind_matches::<T>(block) {
        return None;
    }
    let StoredBlock::Sequence(storage) = block else {
        return None;
    };
    let values = <T as sealed::StoredScalar>::as_slice(storage)?;
    Some(SequenceView { values })
}

pub(super) fn sequence_kind_matches<T: StoredScalar>(block: &StoredBlock) -> bool {
    let StoredBlock::Sequence(storage) = block else {
        return false;
    };
    storage.kind() == <T as sealed::StoredScalar>::KIND
}

pub(super) fn bytes_view(block: &StoredBlock) -> Option<&[u8]> {
    match block {
        StoredBlock::Bytes(values) => Some(values),
        _ => None,
    }
}

pub(super) fn utf8_view(block: &StoredBlock) -> Option<&str> {
    match block {
        StoredBlock::Utf8(value) => Some(value),
        _ => None,
    }
}

pub(super) fn canonical_record_view(block: &StoredBlock) -> Option<CanonicalRecordView<'_>> {
    match block {
        StoredBlock::Records(storage) => Some(CanonicalRecordView { storage }),
        _ => None,
    }
}

pub(super) fn outcome_evidence_record_view(
    block: &StoredBlock,
) -> Option<OutcomeEvidenceRecordView<'_>> {
    match block {
        StoredBlock::Records(storage) => Some(OutcomeEvidenceRecordView { storage }),
        _ => None,
    }
}

/// Read-only candidate session used to prove exact canonical and evidence reachability.
pub struct StoreValidationSession<'session, 'storage> {
    pub(super) raw: &'session mut RetainedClosureSession<'storage, StoredBlock, StoredBlock>,
}

impl<'storage> StoreValidationSession<'_, 'storage> {
    /// Visits one canonical scalar sequence.
    pub fn canonical_sequence<T: StoredScalar, Tag>(
        &mut self,
        slot: CanonicalSequenceRef<T, Tag>,
    ) -> Option<SequenceView<'storage, T>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        sequence_view(self.raw.canonical(slot.raw)?)
    }

    /// Visits one canonical binary block.
    pub fn canonical_bytes<Tag>(&mut self, slot: CanonicalBytesRef<Tag>) -> Option<&'storage [u8]>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        bytes_view(self.raw.canonical(slot.raw)?)
    }

    /// Visits one canonical UTF-8 block.
    pub fn canonical_utf8<Tag>(&mut self, slot: CanonicalUtf8Ref<Tag>) -> Option<&'storage str>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        utf8_view(self.raw.canonical(slot.raw)?)
    }

    /// Visits one canonical record block.
    pub fn canonical_records<Tag>(
        &mut self,
        slot: CanonicalRecordRef<Tag>,
    ) -> Option<CanonicalRecordView<'storage>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        canonical_record_view(self.raw.canonical(slot.raw)?)
    }

    /// Visits one outcome-evidence scalar sequence.
    pub fn outcome_evidence_sequence<T: StoredScalar, Tag>(
        &mut self,
        slot: OutcomeEvidenceSequenceRef<T, Tag>,
    ) -> Option<SequenceView<'storage, T>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        sequence_view(self.raw.outcome_evidence(slot.raw)?)
    }

    /// Visits one outcome-evidence binary block.
    pub fn outcome_evidence_bytes<Tag>(
        &mut self,
        slot: OutcomeEvidenceBytesRef<Tag>,
    ) -> Option<&'storage [u8]>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        bytes_view(self.raw.outcome_evidence(slot.raw)?)
    }

    /// Visits one outcome-evidence UTF-8 block.
    pub fn outcome_evidence_utf8<Tag>(
        &mut self,
        slot: OutcomeEvidenceUtf8Ref<Tag>,
    ) -> Option<&'storage str>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        utf8_view(self.raw.outcome_evidence(slot.raw)?)
    }

    /// Visits one outcome-evidence record block.
    pub fn outcome_evidence_records<Tag>(
        &mut self,
        slot: OutcomeEvidenceRecordRef<Tag>,
    ) -> Option<OutcomeEvidenceRecordView<'storage>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        outcome_evidence_record_view(self.raw.outcome_evidence(slot.raw)?)
    }
}

/// Read-only candidate session used to prove exact outcome-evidence reachability.
pub struct StoreEvidenceValidationSession<'session, 'storage> {
    pub(super) raw: &'session mut RetainedEvidenceClosureSession<'storage, StoredBlock>,
}

#[cfg(feature = "_u4-activation")]
pub(super) enum StoreEvidenceMemberVisit<'storage, View, Tag> {
    Completed,
    Pending {
        view: View,
        completion: StoreEvidenceMemberCompletion<'storage, Tag>,
    },
}

#[cfg(feature = "_u4-activation")]
pub(super) struct StoreEvidenceMemberCompletion<'storage, Tag> {
    raw: RetainedPendingEvidenceMember<'storage, StoredBlock, Tag>,
}

impl<'storage> StoreEvidenceValidationSession<'_, 'storage> {
    #[cfg(feature = "_u4-activation")]
    fn project_visit<View, Tag>(
        &mut self,
        visit: RetainedEvidenceMemberVisit<'storage, StoredBlock, Tag>,
        project: impl FnOnce(&'storage StoredBlock) -> Option<View>,
    ) -> Option<StoreEvidenceMemberVisit<'storage, View, Tag>> {
        match visit {
            RetainedEvidenceMemberVisit::Completed(block) => {
                if project(block).is_none() {
                    self.raw.invalidate();
                    return None;
                }
                Some(StoreEvidenceMemberVisit::Completed)
            }
            RetainedEvidenceMemberVisit::Pending(raw) => {
                let Some(view) = project(raw.block()) else {
                    self.raw.invalidate();
                    return None;
                };
                Some(StoreEvidenceMemberVisit::Pending {
                    view,
                    completion: StoreEvidenceMemberCompletion { raw },
                })
            }
        }
    }

    /// Peeks at one typed scalar sequence without completing its retained member.
    #[cfg(feature = "_u4-activation")]
    pub(super) fn peek_sequence<T: StoredScalar, Tag>(
        &mut self,
        slot: OutcomeEvidenceSequenceRef<T, Tag>,
    ) -> Option<StoreEvidenceMemberVisit<'storage, SequenceView<'storage, T>, Tag>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let visit = self.raw.peek_outcome_evidence(slot.raw)?;
        self.project_visit(visit, sequence_view::<T>)
    }

    /// Peeks at one binary block without completing its retained member.
    #[cfg(feature = "_u4-activation")]
    pub(super) fn peek_bytes<Tag>(
        &mut self,
        slot: OutcomeEvidenceBytesRef<Tag>,
    ) -> Option<StoreEvidenceMemberVisit<'storage, &'storage [u8], Tag>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let visit = self.raw.peek_outcome_evidence(slot.raw)?;
        self.project_visit(visit, bytes_view)
    }

    /// Peeks at one UTF-8 block without completing its retained member.
    #[cfg(feature = "_u4-activation")]
    pub(super) fn peek_utf8<Tag>(
        &mut self,
        slot: OutcomeEvidenceUtf8Ref<Tag>,
    ) -> Option<StoreEvidenceMemberVisit<'storage, &'storage str, Tag>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let visit = self.raw.peek_outcome_evidence(slot.raw)?;
        self.project_visit(visit, utf8_view)
    }

    /// Peeks at one record block without completing its retained member.
    #[cfg(feature = "_u4-activation")]
    pub(super) fn peek_records<Tag>(
        &mut self,
        slot: OutcomeEvidenceRecordRef<Tag>,
    ) -> Option<StoreEvidenceMemberVisit<'storage, OutcomeEvidenceRecordView<'storage>, Tag>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let visit = self.raw.peek_outcome_evidence(slot.raw)?;
        self.project_visit(visit, outcome_evidence_record_view)
    }

    /// Completes one typed evidence member after its complete contents were validated.
    #[cfg(feature = "_u4-activation")]
    pub(super) fn complete_member<Tag>(
        &mut self,
        completion: StoreEvidenceMemberCompletion<'storage, Tag>,
    ) -> bool
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.raw.complete_outcome_evidence(completion.raw)
    }

    /// Visits one outcome-evidence scalar sequence.
    pub fn sequence<T: StoredScalar, Tag>(
        &mut self,
        slot: OutcomeEvidenceSequenceRef<T, Tag>,
    ) -> Option<SequenceView<'storage, T>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        sequence_view(self.raw.outcome_evidence(slot.raw)?)
    }

    /// Visits one outcome-evidence binary block.
    pub fn bytes<Tag>(&mut self, slot: OutcomeEvidenceBytesRef<Tag>) -> Option<&'storage [u8]>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        bytes_view(self.raw.outcome_evidence(slot.raw)?)
    }

    /// Visits one outcome-evidence UTF-8 block.
    pub fn utf8<Tag>(&mut self, slot: OutcomeEvidenceUtf8Ref<Tag>) -> Option<&'storage str>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        utf8_view(self.raw.outcome_evidence(slot.raw)?)
    }

    /// Visits one outcome-evidence record block.
    pub fn records<Tag>(
        &mut self,
        slot: OutcomeEvidenceRecordRef<Tag>,
    ) -> Option<OutcomeEvidenceRecordView<'storage>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        outcome_evidence_record_view(self.raw.outcome_evidence(slot.raw)?)
    }
}

/// Destination that rejected a concrete retained store action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreDestination {
    Canonical,
    OutcomeEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoreWorkErrorCause {
    Control(ControlCause),
    Ledger(LedgerCause<StoreDimensionKey>),
}

/// Failure issued by one borrowed operation-work capability.
///
/// Values of this type can be cloned for reporting, but a controlled callback accepts them as
/// authoritative only when its own capability observed the same sticky failure. This prevents a
/// cause captured from another operation from being replayed as current cancellation or budget
/// evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreWorkError {
    cause: StoreWorkErrorCause,
}

impl StoreWorkError {
    pub(super) const fn control(cause: ControlCause) -> Self {
        Self {
            cause: StoreWorkErrorCause::Control(cause),
        }
    }

    pub(super) const fn ledger(cause: LedgerCause<StoreDimensionKey>) -> Self {
        Self {
            cause: StoreWorkErrorCause::Ledger(cause),
        }
    }

    /// Returns the operation control failure, when polling observed a terminal state.
    pub const fn control_cause(&self) -> Option<&ControlCause> {
        match &self.cause {
            StoreWorkErrorCause::Control(cause) => Some(cause),
            StoreWorkErrorCause::Ledger(_) => None,
        }
    }

    /// Returns the work-ledger failure, when observed work could not be charged.
    pub const fn ledger_cause(&self) -> Option<&LedgerCause<StoreDimensionKey>> {
        match &self.cause {
            StoreWorkErrorCause::Ledger(cause) => Some(cause),
            StoreWorkErrorCause::Control(_) => None,
        }
    }

    pub(super) fn into_runtime(self) -> StoreRuntimeError {
        match self.cause {
            StoreWorkErrorCause::Control(cause) => StoreRuntimeError::Control(cause),
            StoreWorkErrorCause::Ledger(cause) => StoreRuntimeError::Ledger(cause),
        }
    }
}

impl fmt::Display for StoreWorkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            StoreWorkErrorCause::Control(cause) => {
                write!(formatter, "retained store control rejected work: {cause}")
            }
            StoreWorkErrorCause::Ledger(cause) => {
                write!(formatter, "retained store ledger rejected work: {cause}")
            }
        }
    }
}

impl std::error::Error for StoreWorkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.cause {
            StoreWorkErrorCause::Control(cause) => Some(cause),
            StoreWorkErrorCause::Ledger(cause) => Some(cause),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoreControlledErrorCause {
    Build(StoreBuildError),
    Work(StoreWorkError),
}

/// Failure vocabulary available inside one operation-controlled store callback.
///
/// Downstream code can create only ordinary writer failures through [`StoreBuildError`].
/// Cancellation and budget failures originate from the borrowed
/// [`crate::RetainedStoreWorkControl`] as [`StoreWorkError`] values and remain subject to sticky
/// same-capability verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreControlledError {
    cause: StoreControlledErrorCause,
}

impl StoreControlledError {
    /// Returns the ordinary writer failure, when construction rejected its input.
    pub const fn build_cause(&self) -> Option<&StoreBuildError> {
        match &self.cause {
            StoreControlledErrorCause::Build(cause) => Some(cause),
            StoreControlledErrorCause::Work(_) => None,
        }
    }

    /// Returns the operation control failure, when polling observed a terminal state.
    pub const fn control_cause(&self) -> Option<&ControlCause> {
        match &self.cause {
            StoreControlledErrorCause::Work(cause) => cause.control_cause(),
            StoreControlledErrorCause::Build(_) => None,
        }
    }

    /// Returns the work-ledger failure, when observed work could not be charged.
    pub const fn ledger_cause(&self) -> Option<&LedgerCause<StoreDimensionKey>> {
        match &self.cause {
            StoreControlledErrorCause::Work(cause) => cause.ledger_cause(),
            StoreControlledErrorCause::Build(_) => None,
        }
    }

    pub(super) fn into_build_or_work(self) -> Result<StoreBuildError, StoreWorkError> {
        match self.cause {
            StoreControlledErrorCause::Build(cause) => Ok(cause),
            StoreControlledErrorCause::Work(cause) => Err(cause),
        }
    }
}

impl From<StoreBuildError> for StoreControlledError {
    fn from(cause: StoreBuildError) -> Self {
        Self {
            cause: StoreControlledErrorCause::Build(cause),
        }
    }
}

impl From<StoreWorkError> for StoreControlledError {
    fn from(cause: StoreWorkError) -> Self {
        Self {
            cause: StoreControlledErrorCause::Work(cause),
        }
    }
}

impl fmt::Display for StoreControlledError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            StoreControlledErrorCause::Build(cause) => cause.fmt(formatter),
            StoreControlledErrorCause::Work(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for StoreControlledError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.cause {
            StoreControlledErrorCause::Build(cause) => Some(cause),
            StoreControlledErrorCause::Work(cause) => Some(cause),
        }
    }
}

/// Public failure vocabulary for the concrete retained store.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreRuntimeError {
    Plan(StorePlanError),
    BatchCapacity,
    BatchAllocationFailed,
    Control(ControlCause),
    Ledger(LedgerCause<StoreDimensionKey>),
    ForeignPhaseDomain {
        phase: StorePhaseKey,
    },
    ForeignDimensionDomain {
        dimension: StoreDimensionKey,
    },
    /// Semantic phase completion attempted to move backward in the frozen catalog.
    PhaseCompletionRegression {
        last_completed_phase: StorePhaseKey,
        phase: StorePhaseKey,
    },
    /// A reversible publication transaction currently seals semantic phase completion.
    PhaseCompletionPublicationPending,
    /// Durable publication permanently sealed semantic phase completion.
    PhaseCompletionPublicationCompleted,
    /// The frozen semantic phase catalog violated its internal position invariant.
    PhaseCompletionCatalogShapeMismatch,
    /// The physical account is permanently quarantined; the snapshot retains its first evidence.
    AccountPoisoned(AccountPoisoned),
    /// The account was quarantined after this action's physical admission. The retained charge
    /// remains coupled fail-closed and must not be reported as an ordinary poison-first rejection.
    AccountPoisonedWithRetainedCharge(AccountPoisoned),
    AccountingOverflow,
    PhysicalByteLimit {
        requested: u64,
        limit: u64,
    },
    PhysicalAllocationLimit {
        requested: u64,
        limit: u64,
    },
    Build(StoreBuildError),
    BuilderPanicked,
    /// Trusted retained-store construction violated a prequalified internal invariant.
    ConstructionInvariantUnverified,
    AllocationFailed,
    SlotLimit {
        destination: StoreDestination,
    },
    WrongOutcomeMode,
    InvalidSourceSlot,
    ForeignAccount,
    MembershipExhausted,
    InvalidPublicationPlan,
    /// Retained storage destruction could not prove that its storage, attribution, and admission
    /// lease were released.
    ///
    /// The assembly is poisoned and cannot be retried. No success, rich-failure, or fallback
    /// owner can subsequently be produced from it. The affected physical account and operation
    /// runtime must be quarantined because the uncertain charge is deliberately retained
    /// fail-closed.
    StorageDropUnverified,
    /// The retained ownership graph could not be proven before attribution. The affected account
    /// is quarantined and any structurally coupled storage remains fail-closed.
    OwnershipUnverified,
    /// A retained physical charge could not be proven released. This does not assert that payload
    /// or nested storage destruction failed.
    ChargeReleaseUnverified,
    /// A controlled callback rejected the success path while preserving evidence publication.
    SuccessOutcomeRejected,
    /// A callback returned a control or ledger cause that its current capability did not issue.
    UnverifiedControlledCause,
    FallbackOnly,
    Poisoned,
}

impl fmt::Display for StoreRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(cause) => write!(formatter, "invalid retained store plan: {cause}"),
            Self::BatchCapacity => formatter.write_str("retained store batch capacity is invalid"),
            Self::BatchAllocationFailed => {
                formatter.write_str("retained store batch allocation failed")
            }
            Self::Control(cause) => write!(formatter, "retained store control rejected work: {cause}"),
            Self::Ledger(cause) => {
                write!(formatter, "retained store ledger rejected action: {cause}")
            }
            Self::ForeignPhaseDomain { phase } => write!(
                formatter,
                "retained store phase {phase:?} belongs to another key domain"
            ),
            Self::ForeignDimensionDomain { dimension } => write!(
                formatter,
                "retained store dimension {dimension:?} belongs to another key domain"
            ),
            Self::PhaseCompletionRegression {
                last_completed_phase,
                phase,
            } => write!(
                formatter,
                "retained store phase {phase:?} precedes completed phase {last_completed_phase:?}"
            ),
            Self::PhaseCompletionPublicationPending => formatter
                .write_str("retained store phase completion is sealed by pending publication"),
            Self::PhaseCompletionPublicationCompleted => formatter
                .write_str("retained store phase completion is sealed by durable publication"),
            Self::PhaseCompletionCatalogShapeMismatch => {
                formatter.write_str("retained store phase completion catalog shape is inconsistent")
            }
            Self::AccountPoisoned(cause) => cause.fmt(formatter),
            Self::AccountPoisonedWithRetainedCharge(cause) => write!(
                formatter,
                "{cause}; retained physical charge remains fail-closed"
            ),
            Self::AccountingOverflow => formatter.write_str("retained store accounting overflowed"),
            Self::PhysicalByteLimit { requested, limit } => write!(
                formatter,
                "retained store requested {requested} physical bytes above limit {limit}"
            ),
            Self::PhysicalAllocationLimit { requested, limit } => write!(
                formatter,
                "retained store requested {requested} allocations above limit {limit}"
            ),
            Self::Build(cause) => cause.fmt(formatter),
            Self::BuilderPanicked => formatter.write_str(
                "retained store builder panicked; its physical charge remains fail-closed",
            ),
            Self::ConstructionInvariantUnverified => formatter.write_str(
                "retained store construction violated a qualified invariant; its physical charge remains fail-closed",
            ),
            Self::AllocationFailed => {
                formatter.write_str("retained store allocation failed before payload construction")
            }
            Self::SlotLimit { destination } => {
                write!(formatter, "retained {destination:?} block table is full")
            }
            Self::WrongOutcomeMode => {
                formatter.write_str("canonical storage requires a success-capable publication")
            }
            Self::InvalidSourceSlot => formatter.write_str("retained source reference is invalid"),
            Self::ForeignAccount => formatter.write_str("retained source uses a foreign account"),
            Self::MembershipExhausted => {
                formatter.write_str("retained publication identity space is exhausted")
            }
            Self::InvalidPublicationPlan => {
                formatter.write_str("retained publication plan is inconsistent")
            }
            Self::StorageDropUnverified => {
                formatter.write_str("retained storage release could not be verified")
            }
            Self::OwnershipUnverified => {
                formatter.write_str("retained ownership could not be verified")
            }
            Self::ChargeReleaseUnverified => {
                formatter.write_str("retained physical charge release could not be verified")
            }
            Self::SuccessOutcomeRejected => {
                formatter.write_str("retained store success outcome was already rejected")
            }
            Self::UnverifiedControlledCause => formatter.write_str(
                "retained store callback returned an unverified control or ledger cause",
            ),
            Self::FallbackOnly => {
                formatter.write_str("retained store is restricted to minimal fallback")
            }
            Self::Poisoned => formatter.write_str("retained store ownership is poisoned"),
        }
    }
}

impl std::error::Error for StoreRuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Plan(cause) => Some(cause),
            Self::Control(cause) => Some(cause),
            Self::Ledger(cause) => Some(cause),
            Self::Build(cause) => Some(cause),
            Self::AccountPoisoned(cause) | Self::AccountPoisonedWithRetainedCharge(cause) => {
                Some(cause)
            }
            _ => None,
        }
    }
}

pub(super) fn map_allocation_error(error: &RetainedAllocationError) -> StoreRuntimeError {
    match error {
        RetainedAllocationError::InlinePayloadTooLarge { .. } => {
            StoreRuntimeError::InvalidPublicationPlan
        }
        RetainedAllocationError::AccountPoisoned(cause) => {
            StoreRuntimeError::AccountPoisoned(*cause)
        }
        RetainedAllocationError::AccountPoisonedWithRetainedCharge(cause) => {
            StoreRuntimeError::AccountPoisonedWithRetainedCharge(*cause)
        }
        RetainedAllocationError::AccountingOverflow => StoreRuntimeError::AccountingOverflow,
        RetainedAllocationError::ByteLimitExceeded { requested, limit } => {
            StoreRuntimeError::PhysicalByteLimit {
                requested: *requested,
                limit: *limit,
            }
        }
        RetainedAllocationError::AllocationLimitExceeded { requested, limit } => {
            StoreRuntimeError::PhysicalAllocationLimit {
                requested: *requested,
                limit: *limit,
            }
        }
        RetainedAllocationError::Build(code) => {
            StoreRuntimeError::Build(StoreBuildError::from_raw(code.code()))
        }
        RetainedAllocationError::BuilderPanicked => StoreRuntimeError::BuilderPanicked,
        RetainedAllocationError::ConstructionInvariantUnverified => {
            StoreRuntimeError::ConstructionInvariantUnverified
        }
        RetainedAllocationError::AllocationFailed => StoreRuntimeError::AllocationFailed,
        RetainedAllocationError::ChargeReleaseUnverified => {
            StoreRuntimeError::ChargeReleaseUnverified
        }
    }
}

pub(super) fn map_writer_error(
    error: &RetainedWriterError<StoreDimensionKey>,
) -> StoreRuntimeError {
    match error {
        RetainedWriterError::BatchCapacity => StoreRuntimeError::BatchCapacity,
        RetainedWriterError::BatchAllocationFailed => StoreRuntimeError::BatchAllocationFailed,
        RetainedWriterError::Ledger(cause) => StoreRuntimeError::Ledger(cause.clone()),
        RetainedWriterError::AccountPoisoned(cause) => StoreRuntimeError::AccountPoisoned(*cause),
        RetainedWriterError::Physical(cause) => map_allocation_error(cause),
        RetainedWriterError::AttributionBatchFull
        | RetainedWriterError::CommitmentBatchFull
        | RetainedWriterError::InvalidPublicationPlan => StoreRuntimeError::InvalidPublicationPlan,
        RetainedWriterError::ForeignAccount => StoreRuntimeError::ForeignAccount,
        RetainedWriterError::MembershipExhausted => StoreRuntimeError::MembershipExhausted,
        RetainedWriterError::StorageDropUnverified => StoreRuntimeError::StorageDropUnverified,
        RetainedWriterError::OwnershipUnverified => StoreRuntimeError::OwnershipUnverified,
        RetainedWriterError::ChargeReleaseUnverified => StoreRuntimeError::ChargeReleaseUnverified,
    }
}

pub(super) fn map_phase_completion_error(
    error: PhaseCompletionError<StorePhaseKey>,
) -> StoreRuntimeError {
    match error {
        PhaseCompletionError::PublicationPending => {
            StoreRuntimeError::PhaseCompletionPublicationPending
        }
        PhaseCompletionError::PublicationCompleted => {
            StoreRuntimeError::PhaseCompletionPublicationCompleted
        }
        PhaseCompletionError::UnknownPhase { .. } => {
            StoreRuntimeError::Ledger(LedgerCause::UnknownPhase)
        }
        PhaseCompletionError::Regression {
            last_completed_phase,
            phase,
        } => StoreRuntimeError::PhaseCompletionRegression {
            last_completed_phase,
            phase,
        },
        PhaseCompletionError::CatalogShapeMismatch => {
            StoreRuntimeError::PhaseCompletionCatalogShapeMismatch
        }
    }
}

pub(super) fn map_publication_error(
    error: &RetainedPublicationError<StoreDimensionKey>,
) -> StoreRuntimeError {
    match error {
        RetainedPublicationError::Writer(cause) => map_writer_error(cause),
        RetainedPublicationError::CanonicalSlotLimit => StoreRuntimeError::SlotLimit {
            destination: StoreDestination::Canonical,
        },
        RetainedPublicationError::OutcomeEvidenceSlotLimit => StoreRuntimeError::SlotLimit {
            destination: StoreDestination::OutcomeEvidence,
        },
        RetainedPublicationError::WrongOutcomeMode => StoreRuntimeError::WrongOutcomeMode,
        RetainedPublicationError::SuccessOutcomeRejected => {
            StoreRuntimeError::SuccessOutcomeRejected
        }
        RetainedPublicationError::InvalidSourceSlot => StoreRuntimeError::InvalidSourceSlot,
        RetainedPublicationError::FailureTerminalWorkSealed => StoreRuntimeError::FallbackOnly,
        RetainedPublicationError::Poisoned => StoreRuntimeError::Poisoned,
    }
}

pub(super) fn publication_error_requires_containment(
    error: &RetainedPublicationError<StoreDimensionKey>,
) -> bool {
    retained_publication_error_requires_containment(error)
}
