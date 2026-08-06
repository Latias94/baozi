//! Operation-owned, physically metered retained storage.
//!
//! This module is the only public construction seam for retained publication payloads. The raw
//! generic owner in `retained` remains a private foundation: callers can request semantic element
//! counts, but cannot provide an owning payload or declare its allocator footprint.

#![allow(
    clippy::result_large_err,
    reason = "bounded poison snapshots stay inline so containment never needs a failure-path allocation"
)]
//!
//! Raw ownership and arbitrary payloads are intentionally unreachable. Each forbidden symbol has
//! an independent compile-time gate so one missing export cannot mask another:
//!
//! ```compile_fail,E0432
//! use baozi_operation::BlockEmplacer;
//! ```
//!
//! ```compile_fail,E0432
//! use baozi_operation::RetainedAllocation;
//! ```
//!
//! ```compile_fail,E0432
//! use baozi_operation::RetainedAllocationPlan;
//! ```
//!
//! The closed scalar vocabulary cannot be extended downstream:
//!
//! ```compile_fail,E0277
//! use baozi_operation::StoredScalar;
//!
//! #[derive(Debug, Clone, Copy, PartialEq)]
//! struct ForeignScalar(u32);
//!
//! impl StoredScalar for ForeignScalar {}
//! ```
//!
//! No generic stored block is public:
//!
//! ```compile_fail,E0432
//! use baozi_operation::StoredBlock;
//! ```
//!
//! Publication references cannot be forged from public types:
//!
//! ```compile_fail,E0451
//! use std::marker::PhantomData;
//! use baozi_operation::{ByteBlock, CanonicalStoreRef};
//!
//! struct Tag;
//! let _forged = CanonicalStoreRef::<ByteBlock, Tag> {
//!     raw: panic!("private raw slot"),
//!     marker: PhantomData,
//! };
//! ```
//!
//! Published storage has no mutable-backing escape hatch:
//!
//! ```compile_fail,E0599
//! use baozi_operation::PublishedCanonicalStore;
//!
//! fn mutate<Kind, Tag>(published: &mut PublishedCanonicalStore<Kind, Tag>) {
//!     let _ = published.root_mut();
//! }
//! ```
//!
//! Controlled callbacks cannot forge fail-closed runtime states or operation-work causes:
//!
//! ```compile_fail,E0451
//! use baozi_operation::{StoreControlledError, StoreWorkError};
//!
//! let _forged = StoreControlledError {
//!     cause: panic!("private controlled cause"),
//! };
//! let _forged_work = StoreWorkError {
//!     cause: panic!("private operation-work cause"),
//! };
//! ```
//!
//! Fixed-width `RecordBlock` storage is permanently excluded from cross-generation sharing
//! because it may contain publication-local references:
//!
//! ```compile_fail,E0277
//! use baozi_operation::{RecordBlock, ShareableStoreKind};
//!
//! fn require_shareable<Kind: ShareableStoreKind>() {}
//! require_shareable::<RecordBlock>();
//! ```
//!
//! Store reference brands are invariant, so function-pointer lifetime subtyping cannot retag a
//! reference kind or its domain tag:
//!
//! ```compile_fail
//! use std::marker::PhantomData;
//! use baozi_operation::CanonicalStoreRange;
//!
//! struct LifetimeKind<'a>(PhantomData<&'a ()>);
//! struct Tag;
//!
//! fn narrow_kind<'short>(
//!     range: CanonicalStoreRange<LifetimeKind<'static>, Tag>,
//! ) -> CanonicalStoreRange<LifetimeKind<'short>, Tag> {
//!     range
//! }
//! ```
//!
//! ```compile_fail
//! use std::marker::PhantomData;
//! use baozi_operation::{ByteBlock, CanonicalStoreRange};
//!
//! struct LifetimeTag<'a>(PhantomData<&'a ()>);
//!
//! fn narrow_tag<'short>(
//!     range: CanonicalStoreRange<ByteBlock, LifetimeTag<'static>>,
//! ) -> CanonicalStoreRange<ByteBlock, LifetimeTag<'short>> {
//!     range
//! }
//! ```
//!
//! Builder callbacks are contained only for one ordinary unwind. A second panic while unwinding,
//! or a build compiled with `panic = "abort"`, cannot be contained by `catch_unwind`; untrusted
//! adapters that require those boundaries must run in an isolated process.

use std::fmt;
use std::marker::PhantomData;

use crate::capacity::{CapacityPlan, CapacityPlanError};
use crate::control::{OperationControl, ProgressPhase, ProgressUnit};
use crate::coordinator::FinalProgress;
#[cfg(test)]
use crate::ledger::PhaseCompletionError;
use crate::ledger::{LedgerCause, LiveReservation};
use crate::owner::OperationOwner;
#[cfg(test)]
use crate::retained::RetainedAllocationError;
use crate::retained::{
    CanonicalLeafInheritanceError, CanonicalRetainedLeafPolicy, CanonicalRetainedRewriteMap,
    CanonicalRetainedSlot, CanonicalSourceBindError, FinalizationContainmentError,
    OutcomeEvidenceRetainedSlot, PhysicalStorageAccount, PreparationFailureCode,
    PublishedCanonicalOutput, RetainedAllocationPlan, RetainedBuildFailureCode,
    RetainedControlCheckpoint, RetainedFailureAssembly, RetainedFailureValidation,
    RetainedFallbackOnly, RetainedOperation, RetainedOperationStartError,
    RetainedOperationStartPlan, RetainedPublicationAssembly, RetainedPublicationError,
    RetainedPublicationOpenRecovery, RetainedPublicationPrepareError, RetainedSuccessValidation,
    RetainedWorkControl, RetainedWriterError, ValidatedRetainedFailure, ValidatedRetainedSuccess,
};
#[cfg(feature = "_u4c-collector")]
use crate::u4c_collector::{
    record_payload_copy_bytes, record_reference_write, record_semantic_cell_write,
    record_semantic_cell_writes,
};
use terminal_output::{
    classify_prepared_failure, classify_prepared_fallback, classify_prepared_success,
    read_canonical_source_bytes, read_canonical_source_records, read_canonical_source_sequence,
    read_canonical_source_utf8,
};
use validation::{
    map_phase_completion_error, map_publication_error, map_writer_error,
    publication_error_requires_containment,
};

#[cfg(feature = "_u4-activation")]
#[doc(hidden)]
pub mod activation;
mod batch;
mod terminal_output;
mod transcript;
mod validation;
mod vocabulary;

pub use batch::{
    BytesBatchItem, BytesBatchPlan, CanonicalBytesRange, CanonicalSequenceRange,
    CanonicalStoreRange, CanonicalUtf8Range, SequenceBatchItem, SequenceBatchPlan,
    StoreBatchRuntimeError, Utf8BatchItem, Utf8BatchPlan,
};
pub use vocabulary::{
    RetainedStoreBatchLimits, RetainedStoreBlockTablePlan, RetainedStoreFailureTerminalWorkPlan,
    RetainedStoreFailureTerminalWorkRequirement, RetainedStorePublicationPlan,
    RetainedStoreSharePlan, RetainedStoreStartPlan, RetainedStoreWriterPlan, StoreDimensionKey,
    StoreDimensionSemanticId, StoreFallbackEnvelope, StoreKeyDomain, StoreKeyDomainCreateError,
    StoreLedgerPlan, StoreLedgerPlanError, StorePhaseKey, StorePhaseSemanticId,
    StoreSemanticCatalogId,
};

pub use terminal_output::{
    PreparedStoreFailure, PreparedStoreFailureOutcome, PreparedStoreFallback, PreparedStoreSuccess,
    PreparedStoreSuccessOutcome, PublishedCanonicalStore, PublishedOutcomeEvidenceStore,
    PublishedOutcomeEvidenceStoreRef, PublishedRichFailureStore, PublishedStoreFailureParts,
    PublishedStoreFallbackOutput, PublishedStoreFallbackParts, PublishedStoreSuccessParts,
    StoreDimensionSnapshot, StoreLedgerEvidence, StoreLedgerEvidenceHeader, StorePhaseSnapshot,
    StorePreparationFailure, StorePreparationFailureClass, StoreTerminalCause,
};

pub use transcript::{
    PlannedCanonicalBytes, PlannedCanonicalBytesBatch, PlannedCanonicalCommit,
    PlannedCanonicalRecords, PlannedCanonicalSequence, PlannedCanonicalSequenceBatch,
    PlannedCanonicalShare, PlannedCanonicalUtf8, PlannedCanonicalUtf8Batch,
    PlannedOutcomeEvidenceBytes, PlannedOutcomeEvidenceCommit, PlannedOutcomeEvidenceRecords,
    PlannedOutcomeEvidenceSequence, PlannedOutcomeEvidenceShare, PlannedOutcomeEvidenceUtf8,
    RetainedStoreExecution, RetainedStoreTranscript, StoreExecutionError,
    StoreFailureContinuationExecution, StoreTranscriptFailureValidation,
    StoreTranscriptIdentityExhausted, StoreTranscriptOpenError, StoreTranscriptOpenRecovery,
    StoreTranscriptOpenResult, StoreTranscriptPlanError, StoreTranscriptStartCause,
    StoreTranscriptStartError, StoreTranscriptStartResult, StoreTranscriptStateError,
    StoreTranscriptSuccessValidation, ValidatedTranscriptStoreSuccess,
};

pub use validation::{
    StoreControlledError, StoreDestination, StoreEvidenceValidationSession, StoreRuntimeError,
    StoreValidationSession, StoreWorkError,
};

// Flat records stay deliberately bounded; schemas approaching this ceiling need a cohesion review.
const MAX_RECORD_FIELDS: usize = 64;
const BUILD_CODE_DOMAIN: u32 = 0x0100_0000;
const BUILD_CODE_ALLOCATION_FAILED: u32 = 1;
const BUILD_CODE_CAPACITY_ENVELOPE_VIOLATED: u32 = 2;
const BUILD_CODE_CAPACITY_EXCEEDED: u32 = 3;
const BUILD_CODE_INCOMPLETE: u32 = 4;
const BUILD_CODE_ROW_WIDTH: u32 = 5;
const CONTAINMENT_FALLBACK_CODE: u32 = 0x434f_4e54;
const UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE: u32 = 0x554e_5652;

fn work_failure_code(cause: &StoreWorkError) -> PreparationFailureCode {
    if let Some(cause) = cause.control_cause() {
        PreparationFailureCode::from_control(cause)
    } else if cause.ledger_cause().is_some() {
        PreparationFailureCode::ledger()
    } else {
        PreparationFailureCode::domain(UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE)
    }
}

#[allow(
    private_interfaces,
    reason = "the private sealed scalar projection accepts only the module-owned storage enum"
)]
mod sealed {
    use super::{
        CanonicalRecordRowWriter, CapacityPlan, OutcomeEvidenceRecordRowWriter, SequenceStorage,
        SequenceWriter, StoreBuildError,
    };

    pub trait StoredScalar: Copy + Send + Sync + std::fmt::Debug + PartialEq + 'static {
        const KIND: u8;

        fn new_writer(
            plan: CapacityPlan,
            expected_len: usize,
        ) -> Result<SequenceWriter<Self>, StoreBuildError>;

        fn push(writer: &mut SequenceWriter<Self>, value: Self) -> Result<(), StoreBuildError>;

        fn as_slice(storage: &SequenceStorage) -> Option<&[Self]>;

        fn write_canonical_record(
            self,
            row: &mut CanonicalRecordRowWriter<'_>,
        ) -> Result<(), StoreBuildError>;

        fn write_outcome_evidence_record(
            self,
            row: &mut OutcomeEvidenceRecordRowWriter<'_>,
        ) -> Result<(), StoreBuildError>;

        fn read_canonical_record(
            view: &super::CanonicalRecordView<'_>,
            row: usize,
            field: usize,
        ) -> Option<Self>;

        fn read_outcome_evidence_record(
            view: &super::OutcomeEvidenceRecordView<'_>,
            row: usize,
            field: usize,
        ) -> Option<Self>;
    }

    pub trait ShareableStoreKind {}
}

/// A scalar type supported by the concrete retained store.
///
/// This trait is sealed. Downstream crates cannot add representations whose allocation or drop
/// behavior is outside the store's accounting model.
pub trait StoredScalar: sealed::StoredScalar {}

/// Classification of an allocation-free retained payload construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreBuildFailureClass {
    Domain,
    AllocationFailed,
    CapacityEnvelopeViolated,
    CapacityExceeded,
    IncompleteWrite,
    InvalidRecordWidth,
}

/// Allocation-free failure returned from a scoped store writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreBuildError {
    class: StoreBuildFailureClass,
    code: u32,
}

impl StoreBuildError {
    /// Creates a domain-owned failure code.
    pub fn domain(code: u32) -> Result<Self, StorePlanError> {
        if code >= u32::MAX - BUILD_CODE_DOMAIN {
            return Err(StorePlanError::DomainCodeOutOfRange { code });
        }
        Ok(Self {
            class: StoreBuildFailureClass::Domain,
            code,
        })
    }

    /// Returns the failure class.
    pub const fn class(self) -> StoreBuildFailureClass {
        self.class
    }

    /// Returns the domain code, or zero for a store-owned failure.
    pub const fn code(self) -> u32 {
        self.code
    }

    const fn allocation_failed() -> Self {
        Self {
            class: StoreBuildFailureClass::AllocationFailed,
            code: 0,
        }
    }

    const fn capacity_envelope_violated() -> Self {
        Self {
            class: StoreBuildFailureClass::CapacityEnvelopeViolated,
            code: 0,
        }
    }

    const fn capacity_exceeded() -> Self {
        Self {
            class: StoreBuildFailureClass::CapacityExceeded,
            code: 0,
        }
    }

    const fn incomplete_write() -> Self {
        Self {
            class: StoreBuildFailureClass::IncompleteWrite,
            code: 0,
        }
    }

    const fn invalid_record_width() -> Self {
        Self {
            class: StoreBuildFailureClass::InvalidRecordWidth,
            code: 0,
        }
    }

    const fn into_raw(self) -> RetainedBuildFailureCode {
        let code = match self.class {
            StoreBuildFailureClass::Domain => BUILD_CODE_DOMAIN + self.code,
            StoreBuildFailureClass::AllocationFailed => BUILD_CODE_ALLOCATION_FAILED,
            StoreBuildFailureClass::CapacityEnvelopeViolated => {
                BUILD_CODE_CAPACITY_ENVELOPE_VIOLATED
            }
            StoreBuildFailureClass::CapacityExceeded => BUILD_CODE_CAPACITY_EXCEEDED,
            StoreBuildFailureClass::IncompleteWrite => BUILD_CODE_INCOMPLETE,
            StoreBuildFailureClass::InvalidRecordWidth => BUILD_CODE_ROW_WIDTH,
        };
        RetainedBuildFailureCode::new(code)
    }

    const fn from_raw(code: u32) -> Self {
        match code {
            BUILD_CODE_ALLOCATION_FAILED => Self::allocation_failed(),
            BUILD_CODE_CAPACITY_ENVELOPE_VIOLATED => Self::capacity_envelope_violated(),
            BUILD_CODE_CAPACITY_EXCEEDED => Self::capacity_exceeded(),
            BUILD_CODE_INCOMPLETE => Self::incomplete_write(),
            BUILD_CODE_ROW_WIDTH => Self::invalid_record_width(),
            code if code >= BUILD_CODE_DOMAIN => Self {
                class: StoreBuildFailureClass::Domain,
                code: code - BUILD_CODE_DOMAIN,
            },
            code => Self {
                class: StoreBuildFailureClass::Domain,
                code,
            },
        }
    }
}

impl fmt::Display for StoreBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.class {
            StoreBuildFailureClass::Domain => {
                write!(
                    formatter,
                    "retained store domain construction failed: {}",
                    self.code
                )
            }
            StoreBuildFailureClass::AllocationFailed => {
                formatter.write_str("retained store allocation failed")
            }
            StoreBuildFailureClass::CapacityEnvelopeViolated => {
                formatter.write_str("allocator capacity is outside the qualified envelope")
            }
            StoreBuildFailureClass::CapacityExceeded => {
                formatter.write_str("retained store writer exceeded its planned element count")
            }
            StoreBuildFailureClass::IncompleteWrite => {
                formatter.write_str("retained store writer did not fill its exact plan")
            }
            StoreBuildFailureClass::InvalidRecordWidth => {
                formatter.write_str("retained record row does not match its planned width")
            }
        }
    }
}

impl std::error::Error for StoreBuildError {}

impl From<StoreBuildError> for RetainedBuildFailureCode {
    fn from(error: StoreBuildError) -> Self {
        error.into_raw()
    }
}

/// A concrete store plan is invalid before accounting or allocator entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorePlanError {
    EmptyPayload,
    Capacity(CapacityPlanError),
    InvalidRecordFieldCount { fields: usize, limit: usize },
    SizeOverflow,
    InvalidBlockTable,
    InvalidFailureTerminalWork,
    DomainCodeOutOfRange { code: u32 },
}

impl fmt::Display for StorePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPayload => formatter.write_str("empty retained payloads use no slot"),
            Self::Capacity(cause) => write!(formatter, "invalid retained capacity plan: {cause}"),
            Self::InvalidRecordFieldCount { fields, limit } => write!(
                formatter,
                "retained records contain {fields} fields per row; the limit is {limit}"
            ),
            Self::SizeOverflow => formatter.write_str("retained store size calculation overflowed"),
            Self::InvalidBlockTable => {
                formatter.write_str("retained block-table plan is inconsistent")
            }
            Self::InvalidFailureTerminalWork => formatter.write_str(
                "retained rich-failure terminal work requires a finite non-zero evidence-member bound",
            ),
            Self::DomainCodeOutOfRange { code } => {
                write!(
                    formatter,
                    "retained store domain failure code {code} is out of range"
                )
            }
        }
    }
}

impl std::error::Error for StorePlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Capacity(cause) => Some(cause),
            _ => None,
        }
    }
}

impl From<CapacityPlanError> for StorePlanError {
    fn from(error: CapacityPlanError) -> Self {
        Self::Capacity(error)
    }
}

/// Exact element-count plan for one scalar sequence block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencePlan<T: StoredScalar> {
    len: usize,
    capacity: CapacityPlan,
    marker: PhantomData<fn(T) -> T>,
}

impl<T: StoredScalar> SequencePlan<T> {
    /// Plans one non-empty scalar sequence without entering the allocator.
    pub fn new(len: usize) -> Result<Self, StorePlanError> {
        if len == 0 {
            return Err(StorePlanError::EmptyPayload);
        }
        Ok(Self {
            len,
            capacity: CapacityPlan::for_type::<T>(0, 0, len)?,
            marker: PhantomData,
        })
    }

    /// Returns the exact logical element count.
    pub const fn element_count(self) -> usize {
        self.len
    }

    fn allocation_plan(self) -> Result<RetainedAllocationPlan, StorePlanError> {
        allocation_plan_from_capacity(self.capacity)
    }
}

/// Exact byte-count plan for a binary block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BytesPlan {
    len: usize,
    capacity: CapacityPlan,
}

impl BytesPlan {
    /// Plans one non-empty binary block.
    pub fn new(len: usize) -> Result<Self, StorePlanError> {
        if len == 0 {
            return Err(StorePlanError::EmptyPayload);
        }
        Ok(Self {
            len,
            capacity: CapacityPlan::for_type::<u8>(0, 0, len)?,
        })
    }

    /// Returns the exact byte count.
    pub const fn bytes(self) -> usize {
        self.len
    }

    fn allocation_plan(self) -> Result<RetainedAllocationPlan, StorePlanError> {
        allocation_plan_from_capacity(self.capacity)
    }
}

/// Exact UTF-8 byte-count plan for one text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utf8Plan {
    bytes: usize,
    capacity: CapacityPlan,
}

impl Utf8Plan {
    /// Plans one non-empty UTF-8 block by encoded byte length.
    pub fn new(bytes: usize) -> Result<Self, StorePlanError> {
        if bytes == 0 {
            return Err(StorePlanError::EmptyPayload);
        }
        Ok(Self {
            bytes,
            capacity: CapacityPlan::for_type::<u8>(0, 0, bytes)?,
        })
    }

    /// Returns the exact encoded byte count.
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    fn allocation_plan(self) -> Result<RetainedAllocationPlan, StorePlanError> {
        allocation_plan_from_capacity(self.capacity)
    }
}

/// Exact row and field-count plan for a flat small-record block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordPlan {
    rows: usize,
    fields: usize,
    cells: usize,
    capacity: CapacityPlan,
}

impl RecordPlan {
    /// Plans non-empty fixed-width records with at most 64 allocation-free fields per row.
    pub fn new(rows: usize, fields: usize) -> Result<Self, StorePlanError> {
        if rows == 0 {
            return Err(StorePlanError::EmptyPayload);
        }
        if fields == 0 || fields > MAX_RECORD_FIELDS {
            return Err(StorePlanError::InvalidRecordFieldCount {
                fields,
                limit: MAX_RECORD_FIELDS,
            });
        }
        let cells = rows
            .checked_mul(fields)
            .ok_or(StorePlanError::SizeOverflow)?;
        Ok(Self {
            rows,
            fields,
            cells,
            capacity: CapacityPlan::for_type::<RecordCell>(0, 0, cells)?,
        })
    }

    /// Returns the exact row count.
    pub const fn rows(self) -> usize {
        self.rows
    }

    /// Returns the fixed fields per row.
    pub const fn fields(self) -> usize {
        self.fields
    }

    fn allocation_plan(self) -> Result<RetainedAllocationPlan, StorePlanError> {
        allocation_plan_from_capacity(self.capacity)
    }
}

fn allocation_plan_from_capacity(
    capacity: CapacityPlan,
) -> Result<RetainedAllocationPlan, StorePlanError> {
    let nested_bytes = capacity.envelope_delta_bytes()?;
    RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
        .map_err(|_| StorePlanError::SizeOverflow)
}

fn try_allocate_vec<T>(capacity: CapacityPlan) -> Result<(Vec<T>, usize), StoreBuildError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity.allocator_additional())
        .map_err(|_| StoreBuildError::allocation_failed())?;
    let len = values.len();
    let qualified_capacity = values.capacity();
    let (values, _) = capacity
        .finish_owned(values, len, qualified_capacity)
        .map_err(|_| StoreBuildError::capacity_envelope_violated())?;
    Ok((values, qualified_capacity))
}

fn finish_scoped_writer<Writer>(
    mut writer: Writer,
    build: impl FnOnce(&mut Writer) -> Result<(), StoreBuildError>,
    finish: impl FnOnce(Writer) -> Result<StoredBlock, StoreBuildError>,
) -> Result<StoredBlock, StoreBuildError> {
    // Natural unwind drops the sealed primitive writer before the raw retained owner catches the
    // panic while it still owns the physical admission, lease owner, and attribution.
    build(&mut writer)?;
    finish(writer)
}

enum SequenceStorage {
    U8(Vec<u8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    U64(Vec<u64>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl SequenceStorage {
    const fn kind(&self) -> u8 {
        match self {
            Self::U8(_) => 1,
            Self::I8(_) => 2,
            Self::U16(_) => 3,
            Self::I16(_) => 4,
            Self::U32(_) => 5,
            Self::I32(_) => 6,
            Self::U64(_) => 7,
            Self::I64(_) => 8,
            Self::F32(_) => 9,
            Self::F64(_) => 10,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::U8(values) => values.len(),
            Self::I8(values) => values.len(),
            Self::U16(values) => values.len(),
            Self::I16(values) => values.len(),
            Self::U32(values) => values.len(),
            Self::I32(values) => values.len(),
            Self::U64(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    fn capacity(&self) -> usize {
        match self {
            Self::U8(values) => values.capacity(),
            Self::I8(values) => values.capacity(),
            Self::U16(values) => values.capacity(),
            Self::I16(values) => values.capacity(),
            Self::U32(values) => values.capacity(),
            Self::I32(values) => values.capacity(),
            Self::U64(values) => values.capacity(),
            Self::I64(values) => values.capacity(),
            Self::F32(values) => values.capacity(),
            Self::F64(values) => values.capacity(),
        }
    }
}

/// Scoped writer for one exact scalar sequence.
pub struct SequenceWriter<T> {
    storage: SequenceStorage,
    expected_len: usize,
    qualified_capacity: usize,
    marker: PhantomData<fn(T) -> T>,
}

impl<T: StoredScalar> SequenceWriter<T> {
    /// Appends one scalar while preserving the exact admitted capacity.
    pub fn push(&mut self, value: T) -> Result<(), StoreBuildError> {
        if self.storage.len() >= self.expected_len {
            return Err(StoreBuildError::capacity_exceeded());
        }
        <T as sealed::StoredScalar>::push(self, value)
    }

    /// Appends a borrowed scalar slice without exposing the backing allocation.
    pub fn extend_from_slice(&mut self, values: &[T]) -> Result<(), StoreBuildError> {
        let remaining = self
            .expected_len
            .checked_sub(self.storage.len())
            .ok_or_else(StoreBuildError::capacity_exceeded)?;
        if values.len() > remaining {
            return Err(StoreBuildError::capacity_exceeded());
        }
        for value in values {
            <T as sealed::StoredScalar>::push(self, *value)?;
        }
        Ok(())
    }

    /// Returns the number of written elements.
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns whether no element has been written yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn finish(self) -> Result<StoredBlock, StoreBuildError> {
        if self.storage.len() != self.expected_len {
            return Err(StoreBuildError::incomplete_write());
        }
        if self.storage.capacity() != self.qualified_capacity {
            return Err(StoreBuildError::capacity_envelope_violated());
        }
        Ok(StoredBlock::Sequence(self.storage))
    }
}

/// Immutable typed view over one retained scalar sequence.
#[derive(Clone, Copy)]
pub struct SequenceView<'a, T> {
    values: &'a [T],
}

impl<'a, T: StoredScalar> SequenceView<'a, T> {
    /// Returns the complete typed slice.
    pub const fn as_slice(self) -> &'a [T] {
        self.values
    }

    /// Returns the number of elements.
    pub fn len(self) -> usize {
        self.values.len()
    }

    /// Returns whether the sequence is empty.
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Returns one copied scalar.
    pub fn get(self, index: usize) -> Option<T> {
        self.values.get(index).copied()
    }
}

#[derive(Clone, Copy)]
enum ScalarCell {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Sequence(u8),
    Bytes,
    Utf8,
    Records,
}

#[derive(Clone, Copy)]
enum RecordCell {
    Scalar(ScalarCell),
    CanonicalReference {
        kind: BlockKind,
        identity: crate::retained::RetainedSlotIdentity,
    },
    OutcomeEvidenceReference {
        kind: BlockKind,
        identity: crate::retained::RetainedSlotIdentity,
    },
}

struct RecordStorage {
    cells: Vec<RecordCell>,
    rows: usize,
    fields: usize,
    qualified_capacity: usize,
}

enum StoredBlock {
    Sequence(SequenceStorage),
    Bytes(Vec<u8>),
    Utf8(String),
    Records(RecordStorage),
}

impl StoredBlock {
    const fn kind(&self) -> BlockKind {
        match self {
            Self::Sequence(storage) => BlockKind::Sequence(storage.kind()),
            Self::Bytes(_) => BlockKind::Bytes,
            Self::Utf8(_) => BlockKind::Utf8,
            Self::Records(_) => BlockKind::Records,
        }
    }
}

impl CanonicalRetainedLeafPolicy for StoredBlock {
    fn is_shareable_canonical_leaf(&self) -> bool {
        matches!(
            Self::kind(self),
            BlockKind::Sequence(_) | BlockKind::Bytes | BlockKind::Utf8
        )
    }
}

macro_rules! impl_stored_scalar {
    ($type:ty, $kind:literal, $sequence:ident, $cell:ident) => {
        #[allow(
            private_interfaces,
            reason = "the sealed projection consumes only the private sequence-storage enum"
        )]
        impl sealed::StoredScalar for $type {
            const KIND: u8 = $kind;

            fn new_writer(
                plan: CapacityPlan,
                expected_len: usize,
            ) -> Result<SequenceWriter<Self>, StoreBuildError> {
                let (values, qualified_capacity) = try_allocate_vec::<Self>(plan)?;
                Ok(SequenceWriter {
                    storage: SequenceStorage::$sequence(values),
                    expected_len,
                    qualified_capacity,
                    marker: PhantomData,
                })
            }

            fn push(writer: &mut SequenceWriter<Self>, value: Self) -> Result<(), StoreBuildError> {
                match &mut writer.storage {
                    SequenceStorage::$sequence(values) => {
                        values.push(value);
                        #[cfg(feature = "_u4c-collector")]
                        {
                            record_semantic_cell_write();
                            record_payload_copy_bytes(std::mem::size_of::<$type>());
                        }
                        Ok(())
                    }
                    _ => Err(StoreBuildError::capacity_envelope_violated()),
                }
            }

            fn as_slice(storage: &SequenceStorage) -> Option<&[Self]> {
                match storage {
                    SequenceStorage::$sequence(values) => Some(values.as_slice()),
                    _ => None,
                }
            }

            fn write_canonical_record(
                self,
                row: &mut CanonicalRecordRowWriter<'_>,
            ) -> Result<(), StoreBuildError> {
                row.push_cell(RecordCell::Scalar(ScalarCell::$cell(self)))
            }

            fn write_outcome_evidence_record(
                self,
                row: &mut OutcomeEvidenceRecordRowWriter<'_>,
            ) -> Result<(), StoreBuildError> {
                row.push_cell(RecordCell::Scalar(ScalarCell::$cell(self)))
            }

            fn read_canonical_record(
                view: &CanonicalRecordView<'_>,
                row: usize,
                field: usize,
            ) -> Option<Self> {
                match view.cell(row, field)? {
                    RecordCell::Scalar(ScalarCell::$cell(value)) => Some(*value),
                    _ => None,
                }
            }

            fn read_outcome_evidence_record(
                view: &OutcomeEvidenceRecordView<'_>,
                row: usize,
                field: usize,
            ) -> Option<Self> {
                match view.cell(row, field)? {
                    RecordCell::Scalar(ScalarCell::$cell(value)) => Some(*value),
                    _ => None,
                }
            }
        }

        impl StoredScalar for $type {}
    };
}

impl_stored_scalar!(u8, 1, U8, U8);
impl_stored_scalar!(i8, 2, I8, I8);
impl_stored_scalar!(u16, 3, U16, U16);
impl_stored_scalar!(i16, 4, I16, I16);
impl_stored_scalar!(u32, 5, U32, U32);
impl_stored_scalar!(i32, 6, I32, I32);
impl_stored_scalar!(u64, 7, U64, U64);
impl_stored_scalar!(i64, 8, I64, I64);
impl_stored_scalar!(f32, 9, F32, F32);
impl_stored_scalar!(f64, 10, F64, F64);

/// Scoped writer for one exact binary block.
pub struct BytesWriter {
    values: Vec<u8>,
    expected_len: usize,
    qualified_capacity: usize,
}

impl BytesWriter {
    fn new(plan: BytesPlan) -> Result<Self, StoreBuildError> {
        let (values, qualified_capacity) = try_allocate_vec::<u8>(plan.capacity)?;
        Ok(Self {
            values,
            expected_len: plan.len,
            qualified_capacity,
        })
    }

    /// Appends bytes without exposing the backing allocation.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), StoreBuildError> {
        let remaining = self
            .expected_len
            .checked_sub(self.values.len())
            .ok_or_else(StoreBuildError::capacity_exceeded)?;
        if bytes.len() > remaining {
            return Err(StoreBuildError::capacity_exceeded());
        }
        self.values.extend_from_slice(bytes);
        #[cfg(feature = "_u4c-collector")]
        {
            record_semantic_cell_writes(bytes.len());
            record_payload_copy_bytes(bytes.len());
        }
        Ok(())
    }

    /// Returns the number of written bytes.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether no byte has been written yet.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn finish(self) -> Result<StoredBlock, StoreBuildError> {
        if self.values.len() != self.expected_len {
            return Err(StoreBuildError::incomplete_write());
        }
        if self.values.capacity() != self.qualified_capacity {
            return Err(StoreBuildError::capacity_envelope_violated());
        }
        Ok(StoredBlock::Bytes(self.values))
    }
}

/// Scoped writer for one exact UTF-8 block.
pub struct Utf8Writer {
    value: String,
    expected_bytes: usize,
    qualified_capacity: usize,
}

impl Utf8Writer {
    fn new(plan: Utf8Plan) -> Result<Self, StoreBuildError> {
        let mut value = String::new();
        value
            .try_reserve_exact(plan.capacity.allocator_additional())
            .map_err(|_| StoreBuildError::allocation_failed())?;
        let len = value.len();
        let qualified_capacity = value.capacity();
        let (value, _) = plan
            .capacity
            .finish_owned(value, len, qualified_capacity)
            .map_err(|_| StoreBuildError::capacity_envelope_violated())?;
        Ok(Self {
            value,
            expected_bytes: plan.bytes,
            qualified_capacity,
        })
    }

    /// Appends valid UTF-8 text without exposing the backing string.
    pub fn write_str(&mut self, value: &str) -> Result<(), StoreBuildError> {
        let remaining = self
            .expected_bytes
            .checked_sub(self.value.len())
            .ok_or_else(StoreBuildError::capacity_exceeded)?;
        if value.len() > remaining {
            return Err(StoreBuildError::capacity_exceeded());
        }
        self.value.push_str(value);
        #[cfg(feature = "_u4c-collector")]
        {
            record_semantic_cell_writes(value.len());
            record_payload_copy_bytes(value.len());
        }
        Ok(())
    }

    /// Returns the encoded byte count written so far.
    pub fn bytes(&self) -> usize {
        self.value.len()
    }

    /// Returns whether no text has been written yet.
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    fn finish(self) -> Result<StoredBlock, StoreBuildError> {
        if self.value.len() != self.expected_bytes {
            return Err(StoreBuildError::incomplete_write());
        }
        if self.value.capacity() != self.qualified_capacity {
            return Err(StoreBuildError::capacity_envelope_violated());
        }
        Ok(StoredBlock::Utf8(self.value))
    }
}

/// Marker for a scalar-sequence block reference.
pub struct ScalarSequence<T: StoredScalar>(PhantomData<fn(T) -> T>);

/// Marker for a binary block reference.
pub enum ByteBlock {}

/// Marker for a UTF-8 block reference.
pub enum Utf8Block {}

/// Marker for a fixed-width record block reference.
pub enum RecordBlock {}

/// A sealed block kind that may be inherited across publication generations.
///
/// This marker carries no construction or ownership authority. The current [`RecordBlock`] is
/// permanently excluded because it may contain publication-local references. Any future
/// shareable record representation requires a separate sealed, reference-free marker and an ADR
/// amendment; it must not widen this marker for `RecordBlock`.
pub trait ShareableStoreKind: sealed::ShareableStoreKind {}

impl<T: StoredScalar> sealed::ShareableStoreKind for ScalarSequence<T> {}
impl sealed::ShareableStoreKind for ByteBlock {}
impl sealed::ShareableStoreKind for Utf8Block {}

impl<T: StoredScalar> ShareableStoreKind for ScalarSequence<T> {}
impl ShareableStoreKind for ByteBlock {}
impl ShareableStoreKind for Utf8Block {}

/// Destination-qualified, non-owning reference to one canonical retained block.
#[must_use = "a canonical block reference identifies a member of one publication set"]
pub struct CanonicalStoreRef<Kind, Tag> {
    raw: CanonicalRetainedSlot<StoredBlock, Tag>,
    marker: PhantomData<fn(Kind) -> Kind>,
}

impl<Kind, Tag> Copy for CanonicalStoreRef<Kind, Tag> {}

impl<Kind, Tag> Clone for CanonicalStoreRef<Kind, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Kind, Tag> PartialEq for CanonicalStoreRef<Kind, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<Kind, Tag> Eq for CanonicalStoreRef<Kind, Tag> {}

impl<Kind, Tag> fmt::Debug for CanonicalStoreRef<Kind, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalStoreRef(..)")
    }
}

/// Destination-qualified, non-owning reference to one outcome-evidence retained block.
#[must_use = "an outcome-evidence block reference identifies one publication-set member"]
pub struct OutcomeEvidenceStoreRef<Kind, Tag> {
    raw: OutcomeEvidenceRetainedSlot<StoredBlock, Tag>,
    marker: PhantomData<fn(Kind) -> Kind>,
}

impl<Kind, Tag> Copy for OutcomeEvidenceStoreRef<Kind, Tag> {}

impl<Kind, Tag> Clone for OutcomeEvidenceStoreRef<Kind, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Kind, Tag> PartialEq for OutcomeEvidenceStoreRef<Kind, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<Kind, Tag> Eq for OutcomeEvidenceStoreRef<Kind, Tag> {}

impl<Kind, Tag> fmt::Debug for OutcomeEvidenceStoreRef<Kind, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OutcomeEvidenceStoreRef(..)")
    }
}

/// Canonical reference to a typed scalar sequence.
pub type CanonicalSequenceRef<T, Tag> = CanonicalStoreRef<ScalarSequence<T>, Tag>;
/// Canonical reference to a binary block.
pub type CanonicalBytesRef<Tag> = CanonicalStoreRef<ByteBlock, Tag>;
/// Canonical reference to a UTF-8 block.
pub type CanonicalUtf8Ref<Tag> = CanonicalStoreRef<Utf8Block, Tag>;
/// Canonical reference to a fixed-width record block.
pub type CanonicalRecordRef<Tag> = CanonicalStoreRef<RecordBlock, Tag>;
/// Outcome-evidence reference to a typed scalar sequence.
pub type OutcomeEvidenceSequenceRef<T, Tag> = OutcomeEvidenceStoreRef<ScalarSequence<T>, Tag>;
/// Outcome-evidence reference to a binary block.
pub type OutcomeEvidenceBytesRef<Tag> = OutcomeEvidenceStoreRef<ByteBlock, Tag>;
/// Outcome-evidence reference to a UTF-8 block.
pub type OutcomeEvidenceUtf8Ref<Tag> = OutcomeEvidenceStoreRef<Utf8Block, Tag>;
/// Outcome-evidence reference to a fixed-width record block.
pub type OutcomeEvidenceRecordRef<Tag> = OutcomeEvidenceStoreRef<RecordBlock, Tag>;

/// Scoped canonical record-row writer.
pub struct CanonicalRecordRowWriter<'a> {
    cells: &'a mut Vec<RecordCell>,
    expected_fields: usize,
    start: usize,
}

impl CanonicalRecordRowWriter<'_> {
    fn push_cell(&mut self, cell: RecordCell) -> Result<(), StoreBuildError> {
        let written = self.cells.len().saturating_sub(self.start);
        if written >= self.expected_fields {
            return Err(StoreBuildError::invalid_record_width());
        }
        self.cells.push(cell);
        #[cfg(feature = "_u4c-collector")]
        {
            match cell {
                RecordCell::Scalar(_) => record_semantic_cell_write(),
                RecordCell::CanonicalReference { .. }
                | RecordCell::OutcomeEvidenceReference { .. } => record_reference_write(),
            }
            record_payload_copy_bytes(std::mem::size_of::<RecordCell>());
        }
        Ok(())
    }

    /// Writes one supported scalar field.
    pub fn write_scalar<T: StoredScalar>(&mut self, value: T) -> Result<(), StoreBuildError> {
        <T as sealed::StoredScalar>::write_canonical_record(value, self)
    }

    /// Writes a canonical scalar-sequence reference field.
    pub fn write_sequence_ref<T: StoredScalar, Tag>(
        &mut self,
        value: CanonicalSequenceRef<T, Tag>,
    ) -> Result<(), StoreBuildError> {
        self.push_cell(RecordCell::CanonicalReference {
            kind: BlockKind::Sequence(<T as sealed::StoredScalar>::KIND),
            identity: value.raw.identity(),
        })
    }

    /// Writes a canonical binary reference field.
    pub fn write_bytes_ref<Tag>(
        &mut self,
        value: CanonicalBytesRef<Tag>,
    ) -> Result<(), StoreBuildError> {
        self.push_cell(RecordCell::CanonicalReference {
            kind: BlockKind::Bytes,
            identity: value.raw.identity(),
        })
    }

    /// Writes a canonical UTF-8 reference field.
    pub fn write_utf8_ref<Tag>(
        &mut self,
        value: CanonicalUtf8Ref<Tag>,
    ) -> Result<(), StoreBuildError> {
        self.push_cell(RecordCell::CanonicalReference {
            kind: BlockKind::Utf8,
            identity: value.raw.identity(),
        })
    }

    /// Writes a canonical record reference field.
    pub fn write_record_ref<Tag>(
        &mut self,
        value: CanonicalRecordRef<Tag>,
    ) -> Result<(), StoreBuildError> {
        self.push_cell(RecordCell::CanonicalReference {
            kind: BlockKind::Records,
            identity: value.raw.identity(),
        })
    }
}

/// Scoped outcome-evidence record-row writer.
pub struct OutcomeEvidenceRecordRowWriter<'a> {
    cells: &'a mut Vec<RecordCell>,
    expected_fields: usize,
    start: usize,
}

impl OutcomeEvidenceRecordRowWriter<'_> {
    fn push_cell(&mut self, cell: RecordCell) -> Result<(), StoreBuildError> {
        let written = self.cells.len().saturating_sub(self.start);
        if written >= self.expected_fields {
            return Err(StoreBuildError::invalid_record_width());
        }
        self.cells.push(cell);
        #[cfg(feature = "_u4c-collector")]
        {
            match cell {
                RecordCell::Scalar(_) => record_semantic_cell_write(),
                RecordCell::CanonicalReference { .. }
                | RecordCell::OutcomeEvidenceReference { .. } => record_reference_write(),
            }
            record_payload_copy_bytes(std::mem::size_of::<RecordCell>());
        }
        Ok(())
    }

    /// Writes one supported scalar field.
    pub fn write_scalar<T: StoredScalar>(&mut self, value: T) -> Result<(), StoreBuildError> {
        <T as sealed::StoredScalar>::write_outcome_evidence_record(value, self)
    }

    /// Writes an outcome-evidence scalar-sequence reference field.
    pub fn write_sequence_ref<T: StoredScalar, Tag>(
        &mut self,
        value: OutcomeEvidenceSequenceRef<T, Tag>,
    ) -> Result<(), StoreBuildError> {
        self.push_cell(RecordCell::OutcomeEvidenceReference {
            kind: BlockKind::Sequence(<T as sealed::StoredScalar>::KIND),
            identity: value.raw.identity(),
        })
    }

    /// Writes an outcome-evidence binary reference field.
    pub fn write_bytes_ref<Tag>(
        &mut self,
        value: OutcomeEvidenceBytesRef<Tag>,
    ) -> Result<(), StoreBuildError> {
        self.push_cell(RecordCell::OutcomeEvidenceReference {
            kind: BlockKind::Bytes,
            identity: value.raw.identity(),
        })
    }

    /// Writes an outcome-evidence UTF-8 reference field.
    pub fn write_utf8_ref<Tag>(
        &mut self,
        value: OutcomeEvidenceUtf8Ref<Tag>,
    ) -> Result<(), StoreBuildError> {
        self.push_cell(RecordCell::OutcomeEvidenceReference {
            kind: BlockKind::Utf8,
            identity: value.raw.identity(),
        })
    }

    /// Writes an outcome-evidence record reference field.
    pub fn write_record_ref<Tag>(
        &mut self,
        value: OutcomeEvidenceRecordRef<Tag>,
    ) -> Result<(), StoreBuildError> {
        self.push_cell(RecordCell::OutcomeEvidenceReference {
            kind: BlockKind::Records,
            identity: value.raw.identity(),
        })
    }
}

struct RecordWriterCore {
    storage: RecordStorage,
}

impl RecordWriterCore {
    fn new(plan: RecordPlan) -> Result<Self, StoreBuildError> {
        let (cells, qualified_capacity) = try_allocate_vec::<RecordCell>(plan.capacity)?;
        Ok(Self {
            storage: RecordStorage {
                cells,
                rows: plan.rows,
                fields: plan.fields,
                qualified_capacity,
            },
        })
    }

    fn written_rows(&self) -> usize {
        self.storage.cells.len() / self.storage.fields
    }

    fn can_start_row(&self) -> bool {
        self.storage.cells.len().is_multiple_of(self.storage.fields)
            && self.written_rows() < self.storage.rows
    }

    fn finish(self) -> Result<StoredBlock, StoreBuildError> {
        let expected_cells = self
            .storage
            .rows
            .checked_mul(self.storage.fields)
            .ok_or_else(StoreBuildError::capacity_exceeded)?;
        if self.storage.cells.len() != expected_cells {
            return Err(StoreBuildError::incomplete_write());
        }
        if self.storage.cells.capacity() != self.storage.qualified_capacity {
            return Err(StoreBuildError::capacity_envelope_violated());
        }
        Ok(StoredBlock::Records(self.storage))
    }
}

fn checked_source_record(source: &StoredBlock, plan: RecordPlan) -> Option<&RecordStorage> {
    let StoredBlock::Records(storage) = source else {
        return None;
    };
    let parsed = RecordPlan::new(storage.rows, storage.fields).ok()?;
    (parsed.rows == plan.rows
        && parsed.fields == plan.fields
        && parsed.cells == plan.cells
        && parsed.cells == storage.cells.len()
        && storage.cells.len() <= storage.qualified_capacity
        && storage.cells.capacity() == storage.qualified_capacity)
        .then_some(storage)
}

fn mapped_canonical_record_cell(
    cell: RecordCell,
    mapping: &CanonicalRetainedRewriteMap<'_, '_, StoredBlock>,
) -> Option<RecordCell> {
    match cell {
        RecordCell::Scalar(value) => Some(RecordCell::Scalar(value)),
        RecordCell::CanonicalReference { kind, identity } => {
            let target = mapping.resolve(identity)?;
            (target.source_block().kind() == kind && target.destination_block().kind() == kind)
                .then_some(RecordCell::CanonicalReference {
                    kind,
                    identity: target.destination_identity(),
                })
        }
        RecordCell::OutcomeEvidenceReference { .. } => None,
    }
}

/// Scoped writer for canonical fixed-width records.
pub struct CanonicalRecordWriter {
    core: RecordWriterCore,
}

impl CanonicalRecordWriter {
    fn new(plan: RecordPlan) -> Result<Self, StoreBuildError> {
        Ok(Self {
            core: RecordWriterCore::new(plan)?,
        })
    }

    /// Writes one exact-width row. A failed row is rolled back before returning.
    pub fn write_row(
        &mut self,
        write: impl FnOnce(&mut CanonicalRecordRowWriter<'_>) -> Result<(), StoreBuildError>,
    ) -> Result<(), StoreBuildError> {
        if !self.core.can_start_row() {
            return Err(StoreBuildError::capacity_exceeded());
        }
        let start = self.core.storage.cells.len();
        let mut row = CanonicalRecordRowWriter {
            cells: &mut self.core.storage.cells,
            expected_fields: self.core.storage.fields,
            start,
        };
        let result = write(&mut row);
        let exact = row.cells.len().saturating_sub(start) == row.expected_fields;
        if result.is_err() || !exact {
            row.cells.truncate(start);
            return result.and(Err(StoreBuildError::invalid_record_width()));
        }
        Ok(())
    }

    /// Returns the complete rows written so far.
    pub fn rows(&self) -> usize {
        self.core.written_rows()
    }

    fn finish(self) -> Result<StoredBlock, StoreBuildError> {
        self.core.finish()
    }
}

/// Scoped writer for outcome-evidence fixed-width records.
pub struct OutcomeEvidenceRecordWriter {
    core: RecordWriterCore,
}

impl OutcomeEvidenceRecordWriter {
    fn new(plan: RecordPlan) -> Result<Self, StoreBuildError> {
        Ok(Self {
            core: RecordWriterCore::new(plan)?,
        })
    }

    /// Writes one exact-width row. A failed row is rolled back before returning.
    pub fn write_row(
        &mut self,
        write: impl FnOnce(&mut OutcomeEvidenceRecordRowWriter<'_>) -> Result<(), StoreBuildError>,
    ) -> Result<(), StoreBuildError> {
        if !self.core.can_start_row() {
            return Err(StoreBuildError::capacity_exceeded());
        }
        let start = self.core.storage.cells.len();
        let mut row = OutcomeEvidenceRecordRowWriter {
            cells: &mut self.core.storage.cells,
            expected_fields: self.core.storage.fields,
            start,
        };
        let result = write(&mut row);
        let exact = row.cells.len().saturating_sub(start) == row.expected_fields;
        if result.is_err() || !exact {
            row.cells.truncate(start);
            return result.and(Err(StoreBuildError::invalid_record_width()));
        }
        Ok(())
    }

    /// Returns the complete rows written so far.
    pub fn rows(&self) -> usize {
        self.core.written_rows()
    }

    fn finish(self) -> Result<StoredBlock, StoreBuildError> {
        self.core.finish()
    }
}

/// Immutable view over canonical fixed-width records.
#[derive(Clone, Copy)]
pub struct CanonicalRecordView<'a> {
    storage: &'a RecordStorage,
}

impl<'a> CanonicalRecordView<'a> {
    /// Returns the row count.
    pub const fn rows(self) -> usize {
        self.storage.rows
    }

    /// Returns the fixed field count per row.
    pub const fn fields(self) -> usize {
        self.storage.fields
    }

    /// Reads one scalar field with an exact scalar type match.
    pub fn scalar<T: StoredScalar>(self, row: usize, field: usize) -> Option<T> {
        <T as sealed::StoredScalar>::read_canonical_record(&self, row, field)
    }

    /// Reads one canonical scalar-sequence reference with an exact element-kind match.
    pub fn sequence_ref<T: StoredScalar, Tag: 'static>(
        self,
        row: usize,
        field: usize,
    ) -> Option<CanonicalSequenceRef<T, Tag>> {
        let RecordCell::CanonicalReference {
            kind: BlockKind::Sequence(kind),
            identity,
        } = self.cell(row, field)?
        else {
            return None;
        };
        (*kind == <T as sealed::StoredScalar>::KIND && identity.matches_tag::<Tag>()).then(|| {
            CanonicalStoreRef {
                raw: CanonicalRetainedSlot::from_identity(*identity),
                marker: PhantomData,
            }
        })
    }

    /// Reads one canonical binary reference.
    pub fn bytes_ref<Tag: 'static>(
        self,
        row: usize,
        field: usize,
    ) -> Option<CanonicalBytesRef<Tag>> {
        self.canonical_ref(row, field, BlockKind::Bytes)
    }

    /// Reads one canonical UTF-8 reference.
    pub fn utf8_ref<Tag: 'static>(self, row: usize, field: usize) -> Option<CanonicalUtf8Ref<Tag>> {
        self.canonical_ref(row, field, BlockKind::Utf8)
    }

    /// Reads one canonical record reference.
    pub fn record_ref<Tag: 'static>(
        self,
        row: usize,
        field: usize,
    ) -> Option<CanonicalRecordRef<Tag>> {
        self.canonical_ref(row, field, BlockKind::Records)
    }

    fn canonical_ref<Kind, Tag: 'static>(
        self,
        row: usize,
        field: usize,
        expected: BlockKind,
    ) -> Option<CanonicalStoreRef<Kind, Tag>> {
        let RecordCell::CanonicalReference { kind, identity } = self.cell(row, field)? else {
            return None;
        };
        (*kind == expected && identity.matches_tag::<Tag>()).then(|| CanonicalStoreRef {
            raw: CanonicalRetainedSlot::from_identity(*identity),
            marker: PhantomData,
        })
    }

    fn cell(self, row: usize, field: usize) -> Option<&'a RecordCell> {
        record_cell(self.storage, row, field)
    }
}

/// Immutable view over outcome-evidence fixed-width records.
#[derive(Clone, Copy)]
pub struct OutcomeEvidenceRecordView<'a> {
    storage: &'a RecordStorage,
}

impl<'a> OutcomeEvidenceRecordView<'a> {
    /// Returns the row count.
    pub const fn rows(self) -> usize {
        self.storage.rows
    }

    /// Returns the fixed field count per row.
    pub const fn fields(self) -> usize {
        self.storage.fields
    }

    /// Reads one scalar field with an exact scalar type match.
    pub fn scalar<T: StoredScalar>(self, row: usize, field: usize) -> Option<T> {
        <T as sealed::StoredScalar>::read_outcome_evidence_record(&self, row, field)
    }

    /// Reads one outcome-evidence scalar-sequence reference with an exact element-kind match.
    pub fn sequence_ref<T: StoredScalar, Tag: 'static>(
        self,
        row: usize,
        field: usize,
    ) -> Option<OutcomeEvidenceSequenceRef<T, Tag>> {
        let RecordCell::OutcomeEvidenceReference {
            kind: BlockKind::Sequence(kind),
            identity,
        } = self.cell(row, field)?
        else {
            return None;
        };
        (*kind == <T as sealed::StoredScalar>::KIND && identity.matches_tag::<Tag>()).then(|| {
            OutcomeEvidenceStoreRef {
                raw: OutcomeEvidenceRetainedSlot::from_identity(*identity),
                marker: PhantomData,
            }
        })
    }

    /// Reads one outcome-evidence binary reference.
    pub fn bytes_ref<Tag: 'static>(
        self,
        row: usize,
        field: usize,
    ) -> Option<OutcomeEvidenceBytesRef<Tag>> {
        self.outcome_evidence_ref(row, field, BlockKind::Bytes)
    }

    /// Reads one outcome-evidence UTF-8 reference.
    pub fn utf8_ref<Tag: 'static>(
        self,
        row: usize,
        field: usize,
    ) -> Option<OutcomeEvidenceUtf8Ref<Tag>> {
        self.outcome_evidence_ref(row, field, BlockKind::Utf8)
    }

    /// Reads one outcome-evidence record reference.
    pub fn record_ref<Tag: 'static>(
        self,
        row: usize,
        field: usize,
    ) -> Option<OutcomeEvidenceRecordRef<Tag>> {
        self.outcome_evidence_ref(row, field, BlockKind::Records)
    }

    fn outcome_evidence_ref<Kind, Tag: 'static>(
        self,
        row: usize,
        field: usize,
        expected: BlockKind,
    ) -> Option<OutcomeEvidenceStoreRef<Kind, Tag>> {
        let RecordCell::OutcomeEvidenceReference { kind, identity } = self.cell(row, field)? else {
            return None;
        };
        (*kind == expected && identity.matches_tag::<Tag>()).then(|| OutcomeEvidenceStoreRef {
            raw: OutcomeEvidenceRetainedSlot::from_identity(*identity),
            marker: PhantomData,
        })
    }

    fn cell(self, row: usize, field: usize) -> Option<&'a RecordCell> {
        record_cell(self.storage, row, field)
    }
}

fn record_cell(storage: &RecordStorage, row: usize, field: usize) -> Option<&RecordCell> {
    if row >= storage.rows || field >= storage.fields {
        return None;
    }
    let index = row.checked_mul(storage.fields)?.checked_add(field)?;
    storage.cells.get(index)
}

/// Failure while starting a concrete retained store operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetainedStoreStartError {
    InvalidTerminalLimits,
    Runtime(StoreRuntimeError),
}

impl fmt::Display for RetainedStoreStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTerminalLimits => {
                formatter.write_str("retained store terminal limits are invalid")
            }
            Self::Runtime(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for RetainedStoreStartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(cause) => Some(cause),
            Self::InvalidTerminalLimits => None,
        }
    }
}

type RawStoreOperation<O, PP, U> =
    RetainedOperation<O, StorePhaseKey, PP, U, StoreDimensionKey, StoreFallbackEnvelope>;
type RawStoreControlCheckpoint<'a, O, PP, U> =
    RetainedControlCheckpoint<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>;
type RawStoreWorkControl<'a, O, PP, U> =
    RetainedWorkControl<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>;
type RawStoreAssembly<O, PP, U> = RetainedPublicationAssembly<
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;
type RawStoreFailureAssembly<O, PP, U> = RetainedFailureAssembly<
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;
type RawValidatedStoreSuccess<O, PP, U> = ValidatedRetainedSuccess<
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;
type RawValidatedStoreFailure<O, PP, U> = ValidatedRetainedFailure<
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;
type RawStoreFallbackOnly<O, PP, U> = RetainedFallbackOnly<
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;

/// Typestate marker for a store operation that cannot inherit canonical storage.
///
/// Canonical inheritance methods do not exist on a fresh assembly:
///
/// ```compile_fail,E0599
/// use baozi_operation::{
///     FreshStoreLineage, OperationOwner, ProgressPhase, ProgressUnit,
///     RetainedStorePublicationAssembly, StoreDimensionKey,
/// };
///
/// fn bind_without_source<O, PP, U>(
///     assembly: &mut RetainedStorePublicationAssembly<O, PP, U, FreshStoreLineage>,
///     dimension: StoreDimensionKey,
/// ) where
///     O: OperationOwner,
///     PP: ProgressPhase,
///     U: ProgressUnit,
/// {
///     assembly.try_bind_canonical_source(dimension);
/// }
/// ```
pub struct FreshStoreLineage {
    _private: (),
}

/// Unforgeable authority to inherit only from one exact published canonical generation.
///
/// The authority borrows the source for the complete writable operation lifetime. It has no public
/// constructor and can only be obtained through [`RetainedStoreOperation::try_start_derived`].
pub struct CanonicalInheritanceSource<'source> {
    source: &'source PublishedCanonicalOutput<StoredBlock>,
}

/// Store operation statically bound to one exact canonical source generation.
pub type DerivedRetainedStoreOperation<'source, O, PP, U> =
    RetainedStoreOperation<O, PP, U, CanonicalInheritanceSource<'source>>;

/// Writable publication assembly statically bound to one exact canonical source generation.
pub type DerivedRetainedStorePublicationAssembly<'source, O, PP, U> =
    RetainedStorePublicationAssembly<O, PP, U, CanonicalInheritanceSource<'source>>;

/// Opaque live-storage reservation owned by the concrete retained-store protocol.
///
/// The raw ledger token remains private. Producers can only return this capability to the
/// checkpoint that created it for owner validation, resizing, transition, or RAII release.
#[must_use = "dropping a retained-store reservation releases its live-byte accounting"]
pub struct RetainedStoreReservation<O>
where
    O: OperationOwner,
{
    raw: LiveReservation<O, StorePhaseKey, StoreDimensionKey>,
}

#[cfg(test)]
impl<O> RetainedStoreReservation<O>
where
    O: OperationOwner,
{
    fn covered_bytes_for_test(&self) -> u64 {
        self.raw.bytes()
    }
}

/// Borrowed control-and-budget capability for one retained-store producer.
///
/// The checkpoint cannot report progress, open or publish output, or mutate retained storage.
/// Every supplied phase and dimension must belong to the store domain and the operation's frozen
/// catalog before it reaches the raw ledger.
pub struct RetainedStoreControlCheckpoint<'a, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreControlCheckpoint<'a, O, PP, U>,
    domain: &'a StoreKeyDomain,
    terminal_only: bool,
}

/// Poll-and-work capability lent to one controlled canonical builder or validator.
///
/// The work dimension is validated and bound before the callback begins. This capability cannot
/// reserve storage, report progress, mutate retained tables, or enter publication. Any rejected
/// control action is sticky: ignoring the returned error still rejects the enclosing operation.
pub struct RetainedStoreWorkControl<'a, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreWorkControl<'a, O, PP, U>,
    sticky: Option<StoreWorkError>,
}

enum StoreBuildCallbackFailure {
    VerifiedBuild(StoreBuildError),
    VerifiedWork(StoreWorkError),
    UnverifiedCause,
}

enum StoreWorkCallbackFailure {
    Verified(StoreWorkError),
    UnverifiedCause,
}

impl<O, PP, U> RetainedStoreWorkControl<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Polls cancellation and deadline state.
    pub fn poll(&mut self) -> Result<(), StoreWorkError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        let result = self.raw.poll().map_err(StoreWorkError::control);
        self.preserve(result)
    }

    /// Records attempted-and-consumed work against the prebound dimension.
    pub fn charge_observed(&mut self, amount: u64) -> Result<(), StoreWorkError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        let result = self
            .raw
            .charge_observed(amount)
            .map_err(StoreWorkError::ledger);
        self.preserve(result)
    }

    fn preserve(&mut self, result: Result<(), StoreWorkError>) -> Result<(), StoreWorkError> {
        if let Err(cause) = &result {
            self.sticky.get_or_insert_with(|| cause.clone());
        }
        result
    }

    fn finish_build<T>(
        self,
        callback: Result<T, StoreControlledError>,
    ) -> Result<T, StoreBuildCallbackFailure> {
        match (self.sticky, callback) {
            (Some(cause), _) => Err(StoreBuildCallbackFailure::VerifiedWork(cause)),
            (None, Ok(value)) => Ok(value),
            (None, Err(cause)) => match cause.into_build_or_work() {
                Ok(cause) => Err(StoreBuildCallbackFailure::VerifiedBuild(cause)),
                Err(_) => Err(StoreBuildCallbackFailure::UnverifiedCause),
            },
        }
    }

    fn finish_validation<T>(
        self,
        callback: Result<T, StoreWorkError>,
    ) -> Result<T, StoreWorkCallbackFailure> {
        match (self.sticky, callback) {
            (Some(cause), _) => Err(StoreWorkCallbackFailure::Verified(cause)),
            (None, Ok(value)) => Ok(value),
            (None, Err(_)) => Err(StoreWorkCallbackFailure::UnverifiedCause),
        }
    }
}

impl<O, PP, U> RetainedStoreControlCheckpoint<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn ensure_mutable(&self) -> Result<(), StoreRuntimeError> {
        if self.terminal_only {
            Err(StoreRuntimeError::FallbackOnly)
        } else {
            Ok(())
        }
    }

    fn validate_phase(&self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        Ok(())
    }

    fn validate_dimension(&self, dimension: StoreDimensionKey) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        Ok(())
    }

    /// Polls cancellation and deadline state without exposing progress reporting.
    pub fn poll(&mut self) -> Result<(), StoreRuntimeError> {
        self.ensure_mutable()?;
        self.raw.poll().map_err(StoreRuntimeError::Control)
    }

    /// Records one non-refundable attempted-and-consumed work unit.
    ///
    /// If attempted charging succeeds and consumed charging fails, the attempted prefix remains
    /// recorded.
    pub fn charge_observed(
        &mut self,
        dimension: StoreDimensionKey,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_mutable()?;
        self.validate_dimension(dimension)?;
        self.raw
            .charge_observed(dimension, amount)
            .map_err(StoreRuntimeError::Ledger)
    }

    /// Reserves live bytes before allocation or backend entry.
    pub fn reserve(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        self.ensure_mutable()?;
        self.validate_phase(phase)?;
        self.validate_dimension(dimension)?;
        self.raw
            .reserve(phase, dimension, bytes)
            .map(|raw| RetainedStoreReservation { raw })
            .map_err(StoreRuntimeError::Ledger)
    }

    /// Resizes one live token while preserving it unchanged on rejection.
    pub fn resize(
        &mut self,
        reservation: &mut RetainedStoreReservation<O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_mutable()?;
        self.raw
            .resize(&mut reservation.raw, bytes)
            .map_err(StoreRuntimeError::Ledger)
    }

    /// Atomically replaces a live token's phase, dimension, and byte capacity.
    pub fn transition(
        &mut self,
        reservation: &mut RetainedStoreReservation<O>,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_mutable()?;
        self.validate_phase(phase)?;
        self.validate_dimension(dimension)?;
        self.raw
            .transition(&mut reservation.raw, phase, dimension, bytes)
            .map_err(StoreRuntimeError::Ledger)
    }

    /// Verifies that a live token belongs to this operation without mutating its accounting.
    ///
    /// Rejection latches the operation's first terminal cause as a ledger failure, so ignoring the
    /// returned error cannot later publish a requested-domain fallback.
    pub fn validate_live_owner(
        &mut self,
        reservation: &RetainedStoreReservation<O>,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_mutable()?;
        self.raw
            .validate_live_owner(&reservation.raw)
            .map_err(StoreRuntimeError::Ledger)
    }
}

/// Linear owner for one coordinator, physical account, and pre-admitted fallback.
#[must_use = "the retained store operation must be opened or published as fallback"]
pub struct RetainedStoreOperation<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreOperation<O, PP, U>,
    domain: StoreKeyDomain,
    lineage: L,
}

impl<O, PP, U, L> RetainedStoreOperation<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[cfg(feature = "_u4-activation")]
    fn latch_activation_terminal_cause(&mut self, cause: PreparationFailureCode) {
        self.raw.record_terminal_cause(cause);
    }

    /// Borrows the producer-facing control and budget checkpoint.
    pub fn control_checkpoint(&mut self) -> RetainedStoreControlCheckpoint<'_, O, PP, U> {
        let Self {
            raw,
            domain,
            lineage: _,
        } = self;
        let terminal_only = raw.is_terminal_only();
        RetainedStoreControlCheckpoint {
            raw: raw.control_checkpoint(),
            domain,
            terminal_only,
        }
    }

    /// Records one semantically completed phase after concrete domain and catalog validation.
    pub fn try_complete_phase(&self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        if self.raw.is_terminal_only() {
            return Err(StoreRuntimeError::FallbackOnly);
        }
        if !self.domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        if !self.raw.contains_phase(phase) {
            return Err(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase));
        }
        self.raw
            .try_complete_phase(phase)
            .map_err(map_phase_completion_error)
    }

    fn validate_phase_key(&self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        Ok(())
    }

    fn validate_dimension_key(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        Ok(())
    }

    fn validate_writer_plan(
        &self,
        writer: RetainedStoreWriterPlan,
    ) -> Result<(), StoreRuntimeError> {
        let (phase, dimensions) = writer.keys_for_publication_validation();
        self.validate_writer_keys(phase, dimensions)
    }

    fn validate_writer_keys(
        &self,
        phase: StorePhaseKey,
        dimensions: [StoreDimensionKey; 2],
    ) -> Result<(), StoreRuntimeError> {
        self.validate_phase_key(phase)?;
        for dimension in dimensions {
            self.validate_dimension_key(dimension)?;
        }
        if !self.raw.contains_phase(phase)
            || dimensions
                .into_iter()
                .any(|dimension| !self.raw.contains_dimension(dimension))
        {
            return Err(StoreRuntimeError::InvalidPublicationPlan);
        }
        Ok(())
    }

    fn validate_commitment_dimension(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        self.validate_dimension_key(dimension)
    }

    fn validate_transcript_commitment_dimension(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        self.validate_commitment_dimension(dimension)?;
        if !self.raw.contains_dimension(dimension) {
            return Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
                dimension,
            }));
        }
        Ok(())
    }

    fn validate_publication_plan_keys(
        &self,
        plan: &RetainedStorePublicationPlan,
    ) -> Result<(), StoreRuntimeError> {
        for table in plan.tables_for_validation().into_iter().flatten() {
            let (phase, dimensions) = table.keys_for_publication_validation();
            self.validate_writer_keys(phase, dimensions)?;
        }
        Ok(())
    }

    /// Returns the shared physical account used by every retained block.
    pub fn account(&self) -> &PhysicalStorageAccount {
        self.raw.account()
    }

    /// Opens fixed canonical/evidence tables after complete structural preflight.
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "rollback-safe open errors preserve bounded allocation-free recovery authority"
    )]
    pub fn try_publication_assembly(
        self,
        plan: RetainedStorePublicationPlan,
    ) -> Result<RetainedStorePublicationAssembly<O, PP, U, L>, RetainedStoreOpenError<O, PP, U, L>>
    {
        if self.raw.is_terminal_only() {
            return Err(RetainedStoreOpenError {
                recovery: Some(StoreOpenRecoveryAuthority::FallbackOnly(
                    StoreOpenFallbackOnly { operation: self },
                )),
                cause: StoreRuntimeError::FallbackOnly,
            });
        }
        if let Err(cause) = self.validate_publication_plan_keys(&plan) {
            return Err(RetainedStoreOpenError {
                recovery: Some(StoreOpenRecoveryAuthority::Retry(self)),
                cause,
            });
        }
        let Self {
            raw,
            domain,
            lineage,
        } = self;
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
                match error.into_recovery() {
                    Ok(RetainedPublicationOpenRecovery::Retry(raw)) => {
                        Err(RetainedStoreOpenError {
                            recovery: Some(StoreOpenRecoveryAuthority::Retry(Self {
                                raw,
                                domain,
                                lineage,
                            })),
                            cause,
                        })
                    }
                    Ok(RetainedPublicationOpenRecovery::FallbackOnly(raw)) => {
                        Err(RetainedStoreOpenError {
                            recovery: Some(StoreOpenRecoveryAuthority::FallbackOnly(
                                StoreOpenFallbackOnly {
                                    operation: Self {
                                        raw,
                                        domain,
                                        lineage,
                                    },
                                },
                            )),
                            cause,
                        })
                    }
                    Err(_) => Err(RetainedStoreOpenError {
                        recovery: None,
                        cause,
                    }),
                }
            }
        }
    }

    /// Publishes the pre-admitted minimal fallback without opening block tables.
    pub fn prepare_minimal_fallback(
        self,
        cause: u32,
    ) -> Result<PreparedStoreFallback, StorePrepareError> {
        let domain = self.domain;
        self.raw
            .prepare_minimal_fallback(PreparationFailureCode::domain(cause))
            .map(|raw| classify_prepared_fallback(raw, domain))
            .map_err(StorePrepareError::from_raw)
    }
}

impl<O, PP, U> RetainedStoreOperation<O, PP, U, FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Starts a fresh operation that cannot inherit from any canonical source.
    pub fn try_start(
        control: OperationControl<PP, U>,
        ledger_plan: StoreLedgerPlan,
        account: &PhysicalStorageAccount,
        plan: RetainedStoreStartPlan,
    ) -> Result<Self, RetainedStoreStartError> {
        try_start_store_raw(control, ledger_plan, account, None, plan).map(|(raw, domain)| Self {
            raw,
            domain,
            lineage: FreshStoreLineage { _private: () },
        })
    }

    /// Starts an operation that may inherit only from `source`'s exact canonical generation.
    ///
    /// The physical account and canonical source-map capacity are derived from `source`.
    pub fn try_start_derived<'source, SourceRootKind, SourceRootTag>(
        control: OperationControl<PP, U>,
        ledger_plan: StoreLedgerPlan,
        source: &'source PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        plan: RetainedStoreStartPlan,
    ) -> Result<DerivedRetainedStoreOperation<'source, O, PP, U>, RetainedStoreStartError> {
        let source = source.share_source();
        try_start_store_raw(control, ledger_plan, source.account(), Some(source), plan).map(
            |(raw, domain)| RetainedStoreOperation {
                raw,
                domain,
                lineage: CanonicalInheritanceSource { source },
            },
        )
    }
}

fn try_start_store_raw<O, PP, U>(
    control: OperationControl<PP, U>,
    ledger_plan: StoreLedgerPlan,
    account: &PhysicalStorageAccount,
    source: Option<&PublishedCanonicalOutput<StoredBlock>>,
    plan: RetainedStoreStartPlan,
) -> Result<(RawStoreOperation<O, PP, U>, StoreKeyDomain), RetainedStoreStartError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (terminal_phase, dimensions, failure_terminal_dimension) = plan.keys_for_start_validation();
    if !ledger_plan.owns_phase(terminal_phase) {
        return Err(RetainedStoreStartError::Runtime(
            StoreRuntimeError::ForeignPhaseDomain {
                phase: terminal_phase,
            },
        ));
    }
    if !ledger_plan.contains_phase(terminal_phase) {
        return Err(RetainedStoreStartError::Runtime(StoreRuntimeError::Ledger(
            LedgerCause::UnknownPhase,
        )));
    }
    for dimension in dimensions {
        if !ledger_plan.owns_dimension(dimension) {
            return Err(RetainedStoreStartError::Runtime(
                StoreRuntimeError::ForeignDimensionDomain { dimension },
            ));
        }
        if !ledger_plan.contains_dimension(dimension) {
            return Err(RetainedStoreStartError::Runtime(StoreRuntimeError::Ledger(
                LedgerCause::UnknownDimension { dimension },
            )));
        }
    }
    if !plan.failure_terminal_work_is_separate() {
        return Err(RetainedStoreStartError::InvalidTerminalLimits);
    }
    if !ledger_plan.owns_dimension(failure_terminal_dimension) {
        return Err(RetainedStoreStartError::Runtime(
            StoreRuntimeError::ForeignDimensionDomain {
                dimension: failure_terminal_dimension,
            },
        ));
    }
    if !ledger_plan.contains_dimension(failure_terminal_dimension) {
        return Err(RetainedStoreStartError::Runtime(StoreRuntimeError::Ledger(
            LedgerCause::UnknownDimension {
                dimension: failure_terminal_dimension,
            },
        )));
    }
    if !plan.failure_terminal_budget_matches(&ledger_plan) {
        return Err(RetainedStoreStartError::InvalidTerminalLimits);
    }
    let allocation_plan = RetainedAllocationPlan::new(0, 0)
        .map_err(|_| RetainedStoreStartError::Runtime(StoreRuntimeError::InvalidPublicationPlan))?;
    let vocabulary::RawRetainedStoreStartParts {
        terminal_writer,
        share,
        fallback,
        fallback_dimension,
        fallback_amount,
        snapshot_dimension,
        snapshot_amount,
        failure_terminal_work,
    } = plan.into_raw_start_parts();
    let raw_plan = RetainedOperationStartPlan::for_publication(
        terminal_writer,
        share,
        allocation_plan,
        fallback_dimension,
        fallback_amount,
        snapshot_dimension,
        snapshot_amount,
    )
    .with_failure_terminal_work(failure_terminal_work);
    let (domain, ledger_limits, phases) = ledger_plan.into_parts();
    let result = match source {
        Some(source) => RetainedOperation::try_start_derived_with_phase_catalog(
            control,
            ledger_limits,
            &phases,
            account,
            source.aggregate(),
            raw_plan,
            || Ok(fallback),
        ),
        None => RetainedOperation::try_start_with_phase_catalog(
            control,
            ledger_limits,
            &phases,
            account,
            raw_plan,
            || Ok(fallback),
        ),
    };
    result
        .map(|raw| (raw, domain))
        .map_err(|cause| match cause {
            RetainedOperationStartError::InvalidTerminalLimits => {
                RetainedStoreStartError::InvalidTerminalLimits
            }
            RetainedOperationStartError::TerminalEvidenceCatalogInvariant => {
                RetainedStoreStartError::Runtime(StoreRuntimeError::ConstructionInvariantUnverified)
            }
            RetainedOperationStartError::Writer(cause) => {
                RetainedStoreStartError::Runtime(map_writer_error(&cause))
            }
        })
}

/// Narrow owner returned after the single-use failure-terminal authority has been consumed.
///
/// This owner deliberately exposes only the pre-admitted minimal fallback. It cannot reopen block
/// tables, create checkpoints, or begin a new transcript.
#[must_use = "publish the pre-admitted minimal fallback"]
pub(crate) struct StoreOpenFallbackOnly<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: RetainedStoreOperation<O, PP, U, L>,
}

impl<O, PP, U, L> StoreOpenFallbackOnly<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Publishes the pre-admitted minimal fallback without reopening retained tables.
    pub(crate) fn prepare_minimal_fallback(
        self,
        cause: u32,
    ) -> Result<PreparedStoreFallback, StorePrepareError> {
        self.operation.prepare_minimal_fallback(cause)
    }
}

impl<O, PP, U, L> fmt::Debug for StoreOpenFallbackOnly<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreOpenFallbackOnly")
            .finish_non_exhaustive()
    }
}

enum StoreOpenRecoveryAuthority<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Retry(RetainedStoreOperation<O, PP, U, L>),
    FallbackOnly(StoreOpenFallbackOnly<O, PP, U, L>),
}

/// Exact open cause coupled to the authority proven safe after rollback.
///
/// The linear operation and fallback-only owner remain private so callers cannot discard the
/// cause and publish an unrelated domain code. Retry is available only through
/// [`Self::retry_publication_assembly`], while fallback consumes this complete wrapper through
/// [`Self::prepare_minimal_fallback_with`].
#[must_use = "retry the publication or map the exact cause before publishing minimal fallback"]
pub struct RetainedStoreOpenRecovery<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    cause: StoreRuntimeError,
    authority: StoreOpenRecoveryAuthority<O, PP, U, L>,
}

impl<O, PP, U, L> RetainedStoreOpenRecovery<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Returns the exact typed open cause without separating it from linear authority.
    pub const fn cause(&self) -> &StoreRuntimeError {
        &self.cause
    }

    /// Returns whether pure preflight or verified rollback preserved retry authority.
    pub const fn is_retryable(&self) -> bool {
        matches!(&self.authority, StoreOpenRecoveryAuthority::Retry(_))
    }

    /// Returns whether opening consumed the single-use failure-terminal authority.
    pub const fn is_fallback_only(&self) -> bool {
        matches!(&self.authority, StoreOpenRecoveryAuthority::FallbackOnly(_))
    }

    /// Retries publication without exposing the retained operation owner.
    ///
    /// Calling this on a fallback-only disposition returns the same exact cause and authority as a
    /// new open error; no plan work is attempted.
    #[allow(
        clippy::type_complexity,
        reason = "the exact typestate pair keeps retry authority and its cause coupled without a public alias"
    )]
    pub fn retry_publication_assembly(
        self,
        plan: RetainedStorePublicationPlan,
    ) -> Result<RetainedStorePublicationAssembly<O, PP, U, L>, RetainedStoreOpenError<O, PP, U, L>>
    {
        match self.authority {
            StoreOpenRecoveryAuthority::Retry(operation) => {
                operation.try_publication_assembly(plan)
            }
            StoreOpenRecoveryAuthority::FallbackOnly(owner) => Err(RetainedStoreOpenError {
                recovery: Some(StoreOpenRecoveryAuthority::FallbackOnly(owner)),
                cause: self.cause,
            }),
        }
    }

    /// Maps the exact open cause before publishing the pre-admitted minimal fallback.
    pub fn prepare_minimal_fallback_with(
        self,
        classify: impl FnOnce(&StoreRuntimeError) -> u32,
    ) -> Result<PreparedStoreFallback, StorePrepareError> {
        match self.authority {
            StoreOpenRecoveryAuthority::Retry(operation) => {
                let code = classify(&self.cause);
                operation.prepare_minimal_fallback(code)
            }
            StoreOpenRecoveryAuthority::FallbackOnly(owner) => {
                let code = classify(&self.cause);
                owner.prepare_minimal_fallback(code)
            }
        }
    }
}

impl<O, PP, U, L> fmt::Debug for RetainedStoreOpenRecovery<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let disposition = if self.is_retryable() {
            "retry"
        } else {
            "fallback-only"
        };
        formatter
            .debug_struct("RetainedStoreOpenRecovery")
            .field("cause", &self.cause)
            .field("disposition", &disposition)
            .finish_non_exhaustive()
    }
}

/// Failure to open concrete retained tables, preserving only the authority proven safe to return.
#[must_use = "recover the remaining authority or let the fail-closed owner drop"]
pub struct RetainedStoreOpenError<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    recovery: Option<StoreOpenRecoveryAuthority<O, PP, U, L>>,
    cause: StoreRuntimeError,
}

impl<O, PP, U, L> RetainedStoreOpenError<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Returns the public concrete-store cause.
    pub const fn cause(&self) -> &StoreRuntimeError {
        &self.cause
    }

    /// Couples the exact cause to recoverable authority, or returns the fail-closed cause.
    pub fn into_disposition(
        self,
    ) -> Result<RetainedStoreOpenRecovery<O, PP, U, L>, StoreRuntimeError> {
        match self.recovery {
            Some(authority) => Ok(RetainedStoreOpenRecovery {
                cause: self.cause,
                authority,
            }),
            None => Err(self.cause),
        }
    }
}

impl<O, PP, U, L> fmt::Debug for RetainedStoreOpenError<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let disposition = match &self.recovery {
            Some(StoreOpenRecoveryAuthority::Retry(_)) => "retry",
            Some(StoreOpenRecoveryAuthority::FallbackOnly(_)) => "fallback-only",
            None => "fail-closed",
        };
        formatter
            .debug_struct("RetainedStoreOpenError")
            .field("cause", &self.cause)
            .field("disposition", &disposition)
            .finish_non_exhaustive()
    }
}

impl<O, PP, U, L> fmt::Display for RetainedStoreOpenError<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "concrete retained publication could not be opened: {}",
            self.cause
        )
    }
}

impl<O, PP, U, L> std::error::Error for RetainedStoreOpenError<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Writable concrete canonical/evidence table owner.
///
/// Every retained-block builder callback runs behind a single-unwind containment boundary. Double
/// panics and aborting panic runtimes require process isolation and are outside this in-process
/// contract.
#[must_use = "the retained store assembly must be validated or published as fallback"]
pub struct RetainedStorePublicationAssembly<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreAssembly<O, PP, U>,
    fallback_cause: Option<PreparationFailureCode>,
    success_rejection: Option<PreparationFailureCode>,
    domain: StoreKeyDomain,
    lineage: L,
}

impl<O, PP, U, L> RetainedStorePublicationAssembly<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Borrows transient storage and control authority from the operation owned by this assembly.
    ///
    /// The checkpoint cannot inspect or mutate either publication table. Callers must release every
    /// returned reservation before preparing a terminal publication.
    pub fn try_control_checkpoint(
        &mut self,
    ) -> Result<RetainedStoreControlCheckpoint<'_, O, PP, U>, StoreRuntimeError> {
        self.ensure_writable()?;
        let raw = match self.raw.try_control_checkpoint() {
            Ok(raw) => raw,
            Err(error) => {
                if publication_error_requires_containment(&error) {
                    self.fallback_cause =
                        Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                }
                return Err(map_publication_error(&error));
            }
        };
        Ok(RetainedStoreControlCheckpoint {
            raw,
            domain: &self.domain,
            terminal_only: false,
        })
    }

    /// Returns the number of canonical table members.
    pub fn canonical_len(&self) -> usize {
        self.raw.canonical_len()
    }

    pub(super) fn canonical_new_blocks(&self) -> usize {
        self.raw.canonical_new_blocks()
    }

    pub(super) fn canonical_commitment_len(&self) -> usize {
        self.raw.canonical_commitment_len()
    }

    /// Returns the number of outcome-evidence table members.
    pub fn outcome_evidence_len(&self) -> usize {
        self.raw.outcome_evidence_len()
    }

    pub(super) fn outcome_evidence_new_blocks(&self) -> usize {
        self.raw.outcome_evidence_new_blocks()
    }

    pub(super) fn outcome_evidence_commitment_len(&self) -> usize {
        self.raw.outcome_evidence_commitment_len()
    }

    fn is_fallback_only(&self) -> bool {
        self.fallback_cause.is_some()
    }

    #[cfg(feature = "_u4-activation")]
    fn contain_activation_fault(self) -> FinalizationContainmentError {
        let Self {
            raw,
            fallback_cause: _,
            success_rejection: _,
            domain: _,
            lineage: _,
        } = self;
        raw.contain_activation_fault()
    }

    #[cfg(feature = "_u4-activation")]
    fn latch_activation_terminal_cause(&mut self, cause: PreparationFailureCode) {
        self.raw.record_terminal_cause(cause);
    }

    fn ensure_writable(&self) -> Result<(), StoreRuntimeError> {
        if self.is_fallback_only() {
            Err(StoreRuntimeError::FallbackOnly)
        } else if self.success_rejection.is_some() {
            Err(StoreRuntimeError::SuccessOutcomeRejected)
        } else {
            Ok(())
        }
    }

    fn ensure_evidence_writable(&self) -> Result<(), StoreRuntimeError> {
        if self.is_fallback_only() {
            Err(StoreRuntimeError::FallbackOnly)
        } else {
            Ok(())
        }
    }

    fn validate_dimension(
        &mut self,
        dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        let result = self.raw.try_validate_work_dimension(dimension);
        self.map_action(result)
    }

    fn record_success_rejection(&mut self, cause: PreparationFailureCode) {
        if self.success_rejection.is_some() {
            return;
        }
        self.raw.activate_failure_terminal_work();
        self.success_rejection = Some(cause);
    }

    fn map_action<T>(
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
                    self.fallback_cause =
                        Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                }
                Err(map_publication_error(&error))
            }
        }
    }

    fn map_pure_preflight<T>(
        &mut self,
        result: Result<T, RetainedPublicationError<StoreDimensionKey>>,
    ) -> Result<T, StoreRuntimeError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if publication_error_requires_containment(&error) {
                    self.fallback_cause =
                        Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                }
                Err(map_publication_error(&error))
            }
        }
    }

    fn map_controlled_action<T>(
        &mut self,
        result: Result<T, RetainedPublicationError<StoreDimensionKey>>,
        callback_failure: Option<StoreBuildCallbackFailure>,
    ) -> Result<T, StoreRuntimeError> {
        let Some(callback_failure) = callback_failure else {
            return self.map_action(result);
        };
        match result {
            Ok(_) => {
                self.fallback_cause =
                    Some(PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE));
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            }
            Err(error) if publication_error_requires_containment(&error) => {
                self.fallback_cause =
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
                    self.fallback_cause = Some(PreparationFailureCode::domain(
                        UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE,
                    ));
                    Err(StoreRuntimeError::UnverifiedControlledCause)
                }
            },
        }
    }

    fn map_source_bind_action(
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

    fn map_leaf_inheritance_action(
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
    fn try_bind_canonical_source_from(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        work_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let result = self
            .raw
            .try_bind_canonical_source(source.aggregate(), work_dimension);
        self.map_source_bind_action(result)
    }

    /// Inherits every eligible immutable leaf from the exact bound canonical source.
    ///
    /// Scalar sequences, bytes, and UTF-8 blocks are shared once, while record blocks are skipped.
    /// Historical source lineage does not affect eligibility: a leaf rewritten in an earlier
    /// generation can be inherited when unchanged in this destination. The operation polls and
    /// charges once per source member and rolls back the newly inherited prefix after an ordinary
    /// failure.
    fn try_inherit_all_canonical_leaves_from(
        &mut self,
        source: &PublishedCanonicalOutput<StoredBlock>,
        work_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_writable()?;
        self.validate_dimension(work_dimension)?;
        let result = self
            .raw
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
        let result = self.raw.try_retain_canonical(allocation, |emplacer| {
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
    fn try_rewrite_canonical_sequence_from<T, Tag>(
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
        let result = self.raw.try_retain_rewritten_canonical(
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
        let result = self.raw.try_retain_canonical_controlled(
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
    fn try_rewrite_canonical_sequence_controlled_from<T, Tag>(
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
        let result = self.raw.try_retain_rewritten_canonical_controlled(
            source.aggregate(),
            source_ref.raw,
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
        let result = self.raw.try_retain_canonical(allocation, |emplacer| {
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
        let result = self.raw.try_retain_canonical_controlled(
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
        let result = self.raw.try_retain_canonical(allocation, |emplacer| {
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
        let result = self.raw.try_retain_canonical_controlled(
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
        let result = self.raw.try_retain_canonical(allocation, |emplacer| {
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
    fn try_rewrite_canonical_records_with_from<Tag>(
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
        let result = self.raw.try_retain_rewritten_canonical(
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
    fn try_rewrite_canonical_records_controlled_from<Tag>(
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
        let result = self.raw.try_rewrite_canonical_controlled(
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
                        #[cfg(feature = "_u4c-collector")]
                        {
                            match cell {
                                RecordCell::Scalar(_) => record_semantic_cell_write(),
                                RecordCell::CanonicalReference { .. }
                                | RecordCell::OutcomeEvidenceReference { .. } => {
                                    record_reference_write();
                                }
                            }
                            record_payload_copy_bytes(std::mem::size_of::<RecordCell>());
                        }
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
        let result = self.raw.try_retain_canonical_controlled(
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
    fn try_rewrite_canonical_records_with_controlled_from<Tag>(
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
        let result = self.raw.try_retain_rewritten_canonical_controlled(
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
    fn try_share_canonical_from<SlotKind, SlotTag>(
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
        let result = self.raw.try_commit_canonical(dimension, amount);
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
        let result = self.raw.try_commit_outcome_evidence(dimension, amount);
        self.map_action(result)
    }

    /// Consumes the writable graph and validates exact success reachability.
    pub fn validate_success<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>(
        self,
        canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
        evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreValidationSession<'session, 'storage>,
            CanonicalStoreRef<CanonicalKind, CanonicalTag>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> StoreSuccessValidation<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let Self {
            raw,
            fallback_cause,
            success_rejection,
            domain,
            lineage: _,
        } = self;
        if let Some(cause) = fallback_cause {
            return StoreSuccessValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Assembly { raw, cause },
                domain,
            });
        }
        if let Some(cause) = success_rejection {
            return match raw
                .validate_success(canonical_root.raw, evidence_root.raw, |_, _, _| false)
            {
                RetainedSuccessValidation::Valid(raw) => {
                    StoreSuccessValidation::Rejected(StoreFailureAssembly {
                        raw: raw.downgrade_to_rich_failure(cause),
                        fallback_only: false,
                        forced_cause: Some(cause),
                        evidence_root,
                        domain,
                    })
                }
                RetainedSuccessValidation::Rejected(raw) => {
                    StoreSuccessValidation::Rejected(StoreFailureAssembly {
                        raw: raw.with_fallback_cause(cause),
                        fallback_only: false,
                        forced_cause: Some(cause),
                        evidence_root,
                        domain,
                    })
                }
                RetainedSuccessValidation::FallbackOnly(raw) => {
                    StoreSuccessValidation::FallbackOnly(StoreFallbackOnly {
                        state: StoreFallbackState::Validated(raw),
                        domain,
                    })
                }
            };
        }
        match raw.validate_success(canonical_root.raw, evidence_root.raw, |raw, _, _| {
            let mut session = StoreValidationSession { raw };
            validate(&mut session, canonical_root, evidence_root)
        }) {
            RetainedSuccessValidation::Valid(raw) => {
                StoreSuccessValidation::Valid(ValidatedStoreSuccess {
                    raw,
                    canonical_root,
                    evidence_root,
                    domain,
                })
            }
            RetainedSuccessValidation::Rejected(raw) => {
                StoreSuccessValidation::Rejected(StoreFailureAssembly {
                    raw,
                    fallback_only: false,
                    forced_cause: None,
                    evidence_root,
                    domain,
                })
            }
            RetainedSuccessValidation::FallbackOnly(raw) => {
                StoreSuccessValidation::FallbackOnly(StoreFallbackOnly {
                    state: StoreFallbackState::Validated(raw),
                    domain,
                })
            }
        }
    }

    /// Consumes the writable graph and validates it with prebound operation work control.
    ///
    /// Semantic `Ok(false)` is a rejection. A returned or ignored control error is an
    /// interruption and preserves the evidence-only failure owner. Invalid roots, poisoned state,
    /// and callback panic retain the existing fallback-only containment semantics.
    pub fn validate_success_controlled<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>(
        mut self,
        work_dimension: StoreDimensionKey,
        canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
        evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage, 'control> FnOnce(
            &mut StoreValidationSession<'session, 'storage>,
            CanonicalStoreRef<CanonicalKind, CanonicalTag>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<bool, StoreWorkError>,
    ) -> StoreControlledSuccessValidation<
        O,
        PP,
        U,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
        L,
    >
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        if let Err(cause) = self.validate_dimension(work_dimension) {
            return StoreControlledSuccessValidation::InvalidWorkDimension {
                assembly: self,
                cause,
            };
        }
        let Self {
            raw,
            fallback_cause,
            success_rejection,
            domain,
            lineage: _,
        } = self;
        if let Some(cause) = fallback_cause {
            return StoreControlledSuccessValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Assembly { raw, cause },
                domain,
            });
        }
        if let Some(cause) = success_rejection {
            return match raw
                .validate_success(canonical_root.raw, evidence_root.raw, |_, _, _| false)
            {
                RetainedSuccessValidation::Valid(raw) => {
                    StoreControlledSuccessValidation::Rejected(StoreFailureAssembly {
                        raw: raw.downgrade_to_rich_failure(cause),
                        fallback_only: false,
                        forced_cause: Some(cause),
                        evidence_root,
                        domain,
                    })
                }
                RetainedSuccessValidation::Rejected(raw) => {
                    StoreControlledSuccessValidation::Rejected(StoreFailureAssembly {
                        raw: raw.with_fallback_cause(cause),
                        fallback_only: false,
                        forced_cause: Some(cause),
                        evidence_root,
                        domain,
                    })
                }
                RetainedSuccessValidation::FallbackOnly(raw) => {
                    StoreControlledSuccessValidation::FallbackOnly(StoreFallbackOnly {
                        state: StoreFallbackState::Validated(raw),
                        domain,
                    })
                }
            };
        }
        let mut callback_cause = None;
        let validation = raw.validate_success_controlled(
            work_dimension,
            canonical_root.raw,
            evidence_root.raw,
            |raw, _, _, raw_control| {
                let mut session = StoreValidationSession { raw };
                let mut control = RetainedStoreWorkControl {
                    raw: raw_control,
                    sticky: None,
                };
                let callback = validate(&mut session, canonical_root, evidence_root, &mut control);
                match control.finish_validation(callback) {
                    Ok(accepted) => accepted,
                    Err(cause) => {
                        callback_cause = Some(cause);
                        false
                    }
                }
            },
        );
        match validation {
            RetainedSuccessValidation::Valid(raw) => match callback_cause {
                Some(StoreWorkCallbackFailure::Verified(cause)) => {
                    let forced_cause = work_failure_code(&cause);
                    let failure = StoreFailureAssembly {
                        raw: raw.downgrade_to_rich_failure(forced_cause),
                        fallback_only: false,
                        forced_cause: Some(forced_cause),
                        evidence_root,
                        domain,
                    };
                    StoreControlledSuccessValidation::Interrupted { failure, cause }
                }
                Some(StoreWorkCallbackFailure::UnverifiedCause) => {
                    StoreControlledSuccessValidation::FallbackOnly(StoreFallbackOnly {
                        state: StoreFallbackState::FailureAssembly(raw.downgrade_to_rich_failure(
                            PreparationFailureCode::domain(
                                UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE,
                            ),
                        )),
                        domain,
                    })
                }
                None => StoreControlledSuccessValidation::Valid(ValidatedStoreSuccess {
                    raw,
                    canonical_root,
                    evidence_root,
                    domain,
                }),
            },
            RetainedSuccessValidation::Rejected(raw) => match callback_cause {
                Some(StoreWorkCallbackFailure::Verified(cause)) => {
                    let forced_cause = work_failure_code(&cause);
                    let failure = StoreFailureAssembly {
                        raw: raw.with_fallback_cause(forced_cause),
                        fallback_only: false,
                        forced_cause: Some(forced_cause),
                        evidence_root,
                        domain,
                    };
                    StoreControlledSuccessValidation::Interrupted { failure, cause }
                }
                Some(StoreWorkCallbackFailure::UnverifiedCause) => {
                    StoreControlledSuccessValidation::FallbackOnly(StoreFallbackOnly {
                        state: StoreFallbackState::FailureAssembly(raw.with_fallback_cause(
                            PreparationFailureCode::domain(
                                UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE,
                            ),
                        )),
                        domain,
                    })
                }
                None => StoreControlledSuccessValidation::Rejected(StoreFailureAssembly {
                    raw,
                    fallback_only: false,
                    forced_cause: None,
                    evidence_root,
                    domain,
                }),
            },
            RetainedSuccessValidation::FallbackOnly(raw) => {
                StoreControlledSuccessValidation::FallbackOnly(StoreFallbackOnly {
                    state: StoreFallbackState::Validated(raw),
                    domain,
                })
            }
        }
    }

    /// Consumes the writable graph and validates an evidence-only outcome.
    pub fn validate_rich_failure<EvidenceKind, EvidenceTag>(
        self,
        evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreEvidenceValidationSession<'session, 'storage>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> StoreFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let Self {
            raw,
            fallback_cause,
            success_rejection,
            domain,
            lineage: _,
        } = self;
        if let Some(cause) = fallback_cause {
            return StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Assembly { raw, cause },
                domain,
            });
        }
        map_failure_validation(
            raw.validate_rich_failure(evidence_root.raw, |raw, _| {
                let mut session = StoreEvidenceValidationSession { raw };
                validate(&mut session, evidence_root)
            }),
            evidence_root,
            domain,
            success_rejection,
        )
    }

    /// Discards candidate tables and prepares only the pre-admitted fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        self.prepare_minimal_fallback_with(0)
    }

    /// Discards candidate tables and freezes a caller-owned inert domain cause in the fallback.
    ///
    /// A previously verified control, ledger, or containment cause takes precedence over this
    /// requested code.
    pub fn prepare_minimal_fallback_with(
        self,
        cause: u32,
    ) -> Result<PreparedStoreFallback, StorePrepareError> {
        let Self {
            raw,
            fallback_cause,
            success_rejection,
            domain,
            lineage: _,
        } = self;
        let cause = fallback_cause
            .or(success_rejection)
            .unwrap_or(PreparationFailureCode::domain(cause));
        let raw = raw.prepare_minimal_fallback_with(cause);
        raw.map(|raw| classify_prepared_fallback(raw, domain))
            .map_err(StorePrepareError::from_raw)
    }
}

impl<'source, O, PP, U>
    RetainedStorePublicationAssembly<O, PP, U, CanonicalInheritanceSource<'source>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Binds the exact canonical source selected when this derived operation started.
    pub fn try_bind_canonical_source(
        &mut self,
        work_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        let source = self.lineage.source;
        self.try_bind_canonical_source_from(source, work_dimension)
    }

    /// Inherits every eligible immutable leaf from the exact bound canonical source.
    pub fn try_inherit_all_canonical_leaves(
        &mut self,
        work_dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        let source = self.lineage.source;
        self.try_inherit_all_canonical_leaves_from(source, work_dimension)
    }

    /// Replaces one exact-source canonical sequence with newly constructed immutable storage.
    pub fn try_rewrite_canonical_sequence<T, Tag>(
        &mut self,
        source_ref: CanonicalSequenceRef<T, Tag>,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl FnOnce(&mut SequenceWriter<T>) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let source = self.lineage.source;
        self.try_rewrite_canonical_sequence_from(source, source_ref, plan, tag, build)
    }

    /// Replaces one exact-source canonical sequence with controlled polling and work charging.
    pub fn try_rewrite_canonical_sequence_controlled<T, Tag>(
        &mut self,
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
        let source = self.lineage.source;
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
    pub fn try_rewrite_canonical_records_with<Tag>(
        &mut self,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        tag: Tag,
        build: impl FnOnce(&mut CanonicalRecordWriter) -> Result<(), StoreBuildError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let source = self.lineage.source;
        self.try_rewrite_canonical_records_with_from(source, source_ref, plan, tag, build)
    }

    /// Rewrites one exact-source canonical record block through the bound source map.
    pub fn try_rewrite_canonical_records_controlled<Tag>(
        &mut self,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let source = self.lineage.source;
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
    pub fn try_rewrite_canonical_records_with_mapped_prefix_controlled<Tag>(
        &mut self,
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
        let source = self.lineage.source;
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
        let result = self.raw.try_rewrite_canonical_controlled(
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
                        #[cfg(feature = "_u4c-collector")]
                        {
                            match cell {
                                RecordCell::Scalar(_) => record_semantic_cell_write(),
                                RecordCell::CanonicalReference { .. }
                                | RecordCell::OutcomeEvidenceReference { .. } => {
                                    record_reference_write();
                                }
                            }
                            record_payload_copy_bytes(std::mem::size_of::<RecordCell>());
                        }
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
    pub fn try_rewrite_canonical_records_with_controlled<Tag>(
        &mut self,
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
        let source = self.lineage.source;
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
    pub fn try_share_canonical<SlotKind, SlotTag>(
        &mut self,
        source_ref: CanonicalStoreRef<SlotKind, SlotTag>,
    ) -> Result<CanonicalStoreRef<SlotKind, SlotTag>, StoreRuntimeError>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        let source = self.lineage.source;
        self.try_share_canonical_from(source, source_ref)
    }
}

/// Linear result of exact success-graph validation.
#[must_use = "every validation branch retains the complete publication owner"]
pub enum StoreSuccessValidation<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(ValidatedStoreSuccess<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>),
    Rejected(StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>),
    FallbackOnly(StoreFallbackOnly<O, PP, U>),
}

/// Linear result of controlled exact success-graph validation.
#[must_use = "every controlled validation branch retains the complete publication owner"]
pub enum StoreControlledSuccessValidation<
    O,
    PP,
    U,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
    L = FreshStoreLineage,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    InvalidWorkDimension {
        assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
        cause: StoreRuntimeError,
    },
    Valid(ValidatedStoreSuccess<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>),
    Rejected(StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>),
    Interrupted {
        failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
        cause: StoreWorkError,
    },
    FallbackOnly(StoreFallbackOnly<O, PP, U>),
}

/// Exact success closure whose graph can no longer be mutated.
#[must_use = "validated retained success must be prepared or deliberately dropped"]
pub struct ValidatedStoreSuccess<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawValidatedStoreSuccess<O, PP, U>,
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
}

impl<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    ValidatedStoreSuccess<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[cfg(feature = "_u4-activation")]
    pub(super) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.raw.contain_activation_fault()
    }

    /// Consumes a validated success after a later domain admission rejects canonical publication.
    ///
    /// The evidence root is preserved for bounded detail appends and rich-failure revalidation.
    /// Canonical storage remains sealed until the existing terminal transition drops it before
    /// attribution, so this downgrade performs no fallible allocation or publication step.
    pub fn downgrade_to_rich_failure(
        self,
        fallback_cause: u32,
    ) -> StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag> {
        let fallback_cause = PreparationFailureCode::domain(fallback_cause);
        StoreFailureAssembly {
            raw: self.raw.downgrade_to_rich_failure(fallback_cause),
            fallback_only: false,
            forced_cause: Some(fallback_cause),
            evidence_root: self.evidence_root,
            domain: self.domain,
        }
    }

    /// Freezes the terminal ledger destinations and transfers operation live bytes to zero.
    pub fn try_prepare(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        PreparedStoreSuccessOutcome<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>,
        StorePrepareError,
    > {
        let domain = self.domain;
        self.raw
            .try_prepare(final_progress)
            .map(|raw| {
                classify_prepared_success(raw, self.canonical_root, self.evidence_root, domain)
            })
            .map_err(StorePrepareError::from_raw)
    }

    /// Discards the validated candidate and prepares only minimal fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        let domain = self.domain;
        self.raw
            .prepare_minimal_fallback()
            .map(|raw| classify_prepared_fallback(raw, domain))
            .map_err(StorePrepareError::from_raw)
    }
}

/// Evidence-only builder returned by semantic success rejection.
///
/// Its retained-block builder callbacks have the same single-unwind containment boundary as the
/// success-capable assembly; double panics and aborting runtimes require process isolation.
#[must_use = "the failure assembly must be validated or published as fallback"]
pub struct StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreFailureAssembly<O, PP, U>,
    fallback_only: bool,
    forced_cause: Option<PreparationFailureCode>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
}

impl<O, PP, U, PreservedEvidenceKind, PreservedEvidenceTag>
    StoreFailureAssembly<O, PP, U, PreservedEvidenceKind, PreservedEvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[cfg(feature = "_u4-activation")]
    pub(super) fn with_forced_cause(mut self, cause: PreparationFailureCode) -> Self {
        self.raw = self.raw.with_fallback_cause(cause);
        self.forced_cause = Some(cause);
        self
    }

    #[cfg(feature = "_u4-activation")]
    pub(super) fn try_discard_canonical_for_activation(
        mut self,
    ) -> Result<Self, FinalizationContainmentError> {
        self.raw = self.raw.try_discard_canonical_for_rejection()?;
        debug_assert!(self.raw.canonical_ownership_is_absent());
        Ok(self)
    }

    #[cfg(feature = "_u4-activation")]
    pub(super) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.raw.contain_activation_fault()
    }

    /// Returns the evidence root preserved across semantic or admission downgrade.
    pub const fn evidence_root(
        &self,
    ) -> OutcomeEvidenceStoreRef<PreservedEvidenceKind, PreservedEvidenceTag> {
        self.evidence_root
    }

    /// Returns the current outcome-evidence member count.
    pub fn outcome_evidence_len(&self) -> usize {
        self.raw.outcome_evidence_len()
    }

    pub(super) fn outcome_evidence_new_blocks(&self) -> usize {
        self.raw.outcome_evidence_new_blocks()
    }

    pub(super) fn outcome_evidence_commitment_len(&self) -> usize {
        self.raw.outcome_evidence_commitment_len()
    }

    fn is_fallback_only(&self) -> bool {
        self.fallback_only
    }

    fn ensure_writable(&self) -> Result<(), StoreRuntimeError> {
        if self.fallback_only {
            Err(StoreRuntimeError::FallbackOnly)
        } else {
            Ok(())
        }
    }

    fn map_action<T>(
        &mut self,
        result: Result<T, RetainedPublicationError<StoreDimensionKey>>,
    ) -> Result<T, StoreRuntimeError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if publication_error_requires_containment(&error) {
                    self.fallback_only = true;
                }
                Err(map_publication_error(&error))
            }
        }
    }

    /// Appends one exact scalar-sequence block to rich-failure evidence.
    pub fn try_retain_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl FnOnce(&mut SequenceWriter<T>) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
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

    /// Appends one exact binary block to rich-failure evidence.
    pub fn try_retain_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        tag: Tag,
        build: impl FnOnce(&mut BytesWriter) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
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

    /// Appends one exact UTF-8 block to rich-failure evidence.
    pub fn try_retain_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        tag: Tag,
        build: impl FnOnce(&mut Utf8Writer) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
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

    /// Appends one fixed-width record block to rich-failure evidence.
    pub fn try_retain_records<Tag>(
        &mut self,
        plan: RecordPlan,
        tag: Tag,
        build: impl FnOnce(&mut OutcomeEvidenceRecordWriter) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let allocation = plan.allocation_plan().map_err(StoreRuntimeError::Plan)?;
        let result = self
            .raw
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

    /// Shares one same-account outcome-evidence block into this failure graph.
    pub fn try_share<SourceRootKind, SourceRootTag, SlotKind, SlotTag>(
        &mut self,
        source: &PublishedOutcomeEvidenceStore<SourceRootKind, SourceRootTag>,
        source_ref: OutcomeEvidenceStoreRef<SlotKind, SlotTag>,
    ) -> Result<OutcomeEvidenceStoreRef<SlotKind, SlotTag>, StoreRuntimeError>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let result = self
            .raw
            .try_share_outcome_evidence(source.share_source().aggregate(), source_ref.raw);
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Adds one rich-failure evidence commitment.
    pub fn try_commit(
        &mut self,
        dimension: StoreDimensionKey,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_writable()?;
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        let result = self.raw.try_commit_outcome_evidence(dimension, amount);
        self.map_action(result)
    }

    /// Consumes and validates the exact rich-failure evidence graph.
    pub fn validate_rich_failure<EvidenceKind, EvidenceTag>(
        self,
        evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage> FnOnce(
            &mut StoreEvidenceValidationSession<'session, 'storage>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> StoreFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let Self {
            raw,
            fallback_only,
            forced_cause,
            evidence_root: _,
            domain,
        } = self;
        if fallback_only {
            return StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::FailureAssembly(raw),
                domain,
            });
        }
        map_failure_validation(
            raw.validate_rich_failure(evidence_root.raw, |raw, _| {
                let mut session = StoreEvidenceValidationSession { raw };
                validate(&mut session, evidence_root)
            }),
            evidence_root,
            domain,
            forced_cause,
        )
    }

    #[cfg(feature = "_u4-activation")]
    pub(super) fn validate_rich_failure_controlled<EvidenceKind, EvidenceTag>(
        self,
        evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage, 'control> FnOnce(
            StoreEvidenceValidationSession<'session, 'storage>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
            crate::retained::RetainedFailureWorkControl<
                'control,
                O,
                StorePhaseKey,
                PP,
                U,
                StoreDimensionKey,
            >,
        ) -> bool,
    ) -> StoreFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let Self {
            raw,
            fallback_only,
            forced_cause,
            evidence_root: _,
            domain,
        } = self;
        if fallback_only {
            return StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::FailureAssembly(raw),
                domain,
            });
        }
        map_failure_validation(
            raw.validate_rich_failure_controlled(evidence_root.raw, |raw, _, control| {
                let session = StoreEvidenceValidationSession { raw };
                validate(session, evidence_root, control)
            }),
            evidence_root,
            domain,
            forced_cause,
        )
    }

    /// Discards failure details and prepares only minimal fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        self.prepare_minimal_fallback_with(0)
    }

    /// Discards failure details and freezes a caller-owned inert domain cause in the fallback.
    ///
    /// A verified rejected-success cause or an existing fallback-only cause takes precedence.
    pub fn prepare_minimal_fallback_with(
        self,
        cause: u32,
    ) -> Result<PreparedStoreFallback, StorePrepareError> {
        let Self {
            raw,
            fallback_only,
            forced_cause,
            evidence_root: _,
            domain,
        } = self;
        let raw = match forced_cause {
            Some(cause) => raw.with_fallback_cause(cause),
            None if fallback_only => raw,
            None => raw.with_fallback_cause(PreparationFailureCode::domain(cause)),
        };
        raw.prepare_minimal_fallback()
            .map(|raw| classify_prepared_fallback(raw, domain))
            .map_err(StorePrepareError::from_raw)
    }
}

/// Linear result of exact rich-failure validation.
#[must_use = "every failure-validation branch retains the publication owner"]
pub enum StoreFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>),
    Rejected(StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>),
    FallbackOnly(StoreFallbackOnly<O, PP, U>),
}

fn map_failure_validation<O, PP, U, EvidenceKind, EvidenceTag>(
    validation: RetainedFailureValidation<
        O,
        StorePhaseKey,
        PP,
        U,
        StoreDimensionKey,
        StoreFallbackEnvelope,
        StoredBlock,
        StoredBlock,
    >,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
    forced_cause: Option<PreparationFailureCode>,
) -> StoreFailureValidation<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match validation {
        RetainedFailureValidation::Valid(raw) => {
            StoreFailureValidation::Valid(ValidatedStoreFailure {
                raw,
                forced_cause,
                evidence_root,
                domain,
            })
        }
        RetainedFailureValidation::Rejected(raw) => {
            StoreFailureValidation::Rejected(StoreFailureAssembly {
                raw: match forced_cause {
                    Some(cause) => raw.with_fallback_cause(cause),
                    None => raw,
                },
                fallback_only: false,
                forced_cause,
                evidence_root,
                domain,
            })
        }
        RetainedFailureValidation::FallbackOnly(raw) => {
            StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Validated(raw),
                domain,
            })
        }
    }
}

/// Exact rich-failure closure whose graph can no longer be mutated.
#[must_use = "validated rich failure must be prepared or deliberately dropped"]
pub struct ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawValidatedStoreFailure<O, PP, U>,
    forced_cause: Option<PreparationFailureCode>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
}

impl<O, PP, U, EvidenceKind, EvidenceTag> ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[cfg(feature = "_u4-activation")]
    pub(super) fn contain_activation_fault(self) -> FinalizationContainmentError {
        self.raw.contain_activation_fault()
    }

    /// Freezes rich-failure evidence and its terminal cause.
    ///
    /// A verified control, ledger, or admission-downgrade cause carried by this linear owner takes
    /// precedence over the caller's domain code. Ordinary semantic rejection has no forced cause,
    /// so its caller-supplied code remains authoritative.
    pub fn try_prepare(
        self,
        cause: u32,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag>, StorePrepareError> {
        let Self {
            raw,
            forced_cause,
            evidence_root,
            domain,
        } = self;
        raw.try_prepare(
            forced_cause.unwrap_or(PreparationFailureCode::domain(cause)),
            final_progress,
        )
        .map(|raw| classify_prepared_failure(raw, evidence_root, domain))
        .map_err(StorePrepareError::from_raw)
    }

    /// Discards rich failure and prepares only minimal fallback.
    ///
    /// Any verified cause carried from the rejected success path remains the frozen fallback
    /// cause.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        let Self {
            raw,
            forced_cause,
            evidence_root: _,
            domain,
        } = self;
        let raw = match forced_cause {
            Some(cause) => raw.prepare_minimal_fallback_with(cause),
            None => raw.prepare_minimal_fallback(),
        };
        raw.map(|raw| classify_prepared_fallback(raw, domain))
            .map_err(StorePrepareError::from_raw)
    }
}

enum StoreFallbackState<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Assembly {
        raw: RawStoreAssembly<O, PP, U>,
        cause: PreparationFailureCode,
    },
    FailureAssembly(RawStoreFailureAssembly<O, PP, U>),
    Validated(RawStoreFallbackOnly<O, PP, U>),
}

/// Terminal-only state after panic, invalid root, or poisoned graph containment.
#[must_use = "fallback-only state can publish only the pre-admitted minimal fallback"]
pub struct StoreFallbackOnly<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    state: StoreFallbackState<O, PP, U>,
    domain: StoreKeyDomain,
}

impl<O, PP, U> StoreFallbackOnly<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[cfg(feature = "_u4-activation")]
    pub(super) fn activation_containment_required(&self) -> bool {
        match &self.state {
            StoreFallbackState::Validated(raw) => raw.activation_containment_required(),
            StoreFallbackState::Assembly { .. } | StoreFallbackState::FailureAssembly(_) => false,
        }
    }

    #[cfg(feature = "_u4-activation")]
    pub(super) fn contain_activation_fault(self) -> FinalizationContainmentError {
        match self.state {
            StoreFallbackState::Assembly { raw, cause: _ } => raw.contain_activation_fault(),
            StoreFallbackState::FailureAssembly(raw) => raw.contain_activation_fault(),
            StoreFallbackState::Validated(raw) => raw.contain_activation_fault(),
        }
    }

    /// Discards every candidate table and prepares the assembly-independent fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        let domain = self.domain;
        let raw = match self.state {
            StoreFallbackState::Assembly { raw, cause } => raw.prepare_minimal_fallback_with(cause),
            StoreFallbackState::FailureAssembly(raw) => raw.prepare_minimal_fallback(),
            StoreFallbackState::Validated(raw) => raw.prepare_minimal_fallback(),
        };
        raw.map(|raw| classify_prepared_fallback(raw, domain))
            .map_err(StorePrepareError::from_raw)
    }
}

/// Failure while atomically preparing a retained store publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorePrepareError {
    /// Final eligibility failed closed; no prepared publication or fallback owner was produced.
    FinalizationContainment(FinalizationContainmentError),
}

impl StorePrepareError {
    const fn from_raw(error: RetainedPublicationPrepareError) -> Self {
        match error {
            RetainedPublicationPrepareError::FinalizationContainment(cause) => {
                Self::FinalizationContainment(cause)
            }
        }
    }
}

impl fmt::Display for StorePrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FinalizationContainment(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for StorePrepareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FinalizationContainment(cause) => Some(cause),
        }
    }
}

#[cfg(test)]
mod tests;
