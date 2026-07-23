use std::fmt;

use super::{
    CanonicalBytesRef, CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef,
    CanonicalUtf8Ref, OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef, OutcomeEvidenceRecordView,
    OutcomeEvidenceSequenceRef, OutcomeEvidenceUtf8Ref, SequenceView, StoreBuildError,
    StoreDimensionKey, StoreKeyDomain, StorePhaseKey, StorePlanError, StoredBlock, StoredScalar,
    sealed,
};
use crate::ledger::{LedgerCause, PhaseCompletionError};
use crate::retained::{
    AccountPoisoned, RetainedAllocationError, RetainedClosureSession,
    RetainedEvidenceClosureSession, RetainedPublicationError, RetainedWriterError,
    retained_publication_error_requires_containment,
};

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

impl<'storage> StoreEvidenceValidationSession<'_, 'storage> {
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

/// Public failure vocabulary for the concrete retained store.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreRuntimeError {
    Plan(StorePlanError),
    BatchCapacity,
    BatchAllocationFailed,
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
        RetainedPublicationError::InvalidSourceSlot => StoreRuntimeError::InvalidSourceSlot,
        RetainedPublicationError::Poisoned => StoreRuntimeError::Poisoned,
    }
}

pub(super) fn publication_error_requires_containment(
    error: &RetainedPublicationError<StoreDimensionKey>,
) -> bool {
    retained_publication_error_requires_containment(error)
}

pub(super) fn validate_store_dimension(
    domain: &StoreKeyDomain,
    dimension: StoreDimensionKey,
    known: bool,
) -> Result<(), StoreRuntimeError> {
    if !domain.owns_dimension(dimension) {
        return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
    }
    if !known {
        return Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
            dimension,
        }));
    }
    Ok(())
}
