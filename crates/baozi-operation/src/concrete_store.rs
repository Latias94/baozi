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
use crate::control::{ControlCause, OperationControl, ProgressPhase, ProgressUnit};
use crate::coordinator::FinalProgress;
#[cfg(test)]
use crate::ledger::PhaseCompletionError;
use crate::ledger::{LedgerCause, LiveReservation};
use crate::owner::OperationOwner;
use crate::retained::{
    CanonicalRetainedSlot, FinalizationContainmentError, OutcomeEvidenceRetainedSlot,
    PhysicalStorageAccount, PreparationFailureCode, RetainedAllocationPlan,
    RetainedBuildFailureCode, RetainedControlCheckpoint, RetainedFailureAssembly,
    RetainedFailureValidation, RetainedFallbackOnly, RetainedOperation,
    RetainedOperationStartError, RetainedOperationStartPlan, RetainedPublicationAssembly,
    RetainedPublicationError, RetainedPublicationPrepareError, RetainedSuccessValidation,
    ValidatedRetainedFailure, ValidatedRetainedSuccess,
};
#[cfg(test)]
use crate::retained::{RetainedAllocationError, RetainedWriterError};
#[cfg(feature = "_u4c-collector")]
use crate::u4c_collector::{
    record_payload_copy_bytes, record_reference_write, record_semantic_cell_write,
    record_semantic_cell_writes,
};
use terminal_output::{
    classify_prepared_failure, classify_prepared_fallback, classify_prepared_success,
};
use validation::{
    map_phase_completion_error, map_publication_error, map_writer_error,
    publication_error_requires_containment, validate_store_dimension,
};

mod batch;
mod terminal_output;
mod validation;
mod vocabulary;

pub use batch::{
    BytesBatchItem, BytesBatchPlan, CanonicalBytesRange, CanonicalSequenceRange,
    CanonicalStoreRange, CanonicalUtf8Range, SequenceBatchItem, SequenceBatchPlan,
    StoreBatchRuntimeError, Utf8BatchItem, Utf8BatchPlan,
};
pub use vocabulary::{
    RetainedStoreBatchLimits, RetainedStoreBlockTablePlan, RetainedStorePublicationPlan,
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

pub use validation::{
    StoreDestination, StoreEvidenceValidationSession, StoreRuntimeError, StoreValidationSession,
};

const MAX_RECORD_FIELDS: usize = 32;
const BUILD_CODE_DOMAIN: u32 = 0x0100_0000;
const BUILD_CODE_ALLOCATION_FAILED: u32 = 1;
const BUILD_CODE_CAPACITY_ENVELOPE_VIOLATED: u32 = 2;
const BUILD_CODE_CAPACITY_EXCEEDED: u32 = 3;
const BUILD_CODE_INCOMPLETE: u32 = 4;
const BUILD_CODE_ROW_WIDTH: u32 = 5;

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
    /// Plans non-empty fixed-width records with at most 32 allocation-free fields per row.
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
}

impl<O, PP, U> RetainedStoreControlCheckpoint<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn validate_phase(&self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        if !self.raw.contains_phase(phase) {
            return Err(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase));
        }
        Ok(())
    }

    fn validate_dimension(&self, dimension: StoreDimensionKey) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        if !self.raw.contains_dimension(dimension) {
            return Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
                dimension,
            }));
        }
        Ok(())
    }

    /// Polls cancellation and deadline state without exposing progress reporting.
    pub fn poll(&mut self) -> Result<(), ControlCause> {
        self.raw.poll()
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
        self.validate_phase(phase)?;
        self.validate_dimension(dimension)?;
        self.raw
            .transition(&mut reservation.raw, phase, dimension, bytes)
            .map_err(StoreRuntimeError::Ledger)
    }

    /// Verifies that a live token belongs to this operation without mutating accounting.
    pub fn validate_live_owner(
        &self,
        reservation: &RetainedStoreReservation<O>,
    ) -> Result<(), StoreRuntimeError> {
        self.raw
            .validate_live_owner(&reservation.raw)
            .map_err(StoreRuntimeError::Ledger)
    }
}

/// Linear owner for one coordinator, physical account, and pre-admitted fallback.
#[must_use = "the retained store operation must be opened or published as fallback"]
pub struct RetainedStoreOperation<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreOperation<O, PP, U>,
    domain: StoreKeyDomain,
}

impl<O, PP, U> RetainedStoreOperation<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Borrows the producer-facing control and budget checkpoint.
    pub fn control_checkpoint(&mut self) -> RetainedStoreControlCheckpoint<'_, O, PP, U> {
        let Self { raw, domain } = self;
        RetainedStoreControlCheckpoint {
            raw: raw.control_checkpoint(),
            domain,
        }
    }

    /// Records one semantically completed phase after concrete domain and catalog validation.
    pub fn try_complete_phase(&self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
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

    fn validate_publication_plan_keys(
        &self,
        plan: &RetainedStorePublicationPlan,
    ) -> Result<(), StoreRuntimeError> {
        for table in plan.tables_for_validation().into_iter().flatten() {
            let (phase, dimensions) = table.keys_for_publication_validation();
            if !self.domain.owns_phase(phase) {
                return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
            }
            if !self.raw.contains_phase(phase) {
                return Err(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase));
            }
            for dimension in dimensions {
                if !self.domain.owns_dimension(dimension) {
                    return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
                }
                if !self.raw.contains_dimension(dimension) {
                    return Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
                        dimension,
                    }));
                }
            }
        }
        Ok(())
    }

    /// Starts one operation and pre-admits its allocation-free minimal fallback.
    pub fn try_start(
        control: OperationControl<PP, U>,
        ledger_plan: StoreLedgerPlan,
        account: &PhysicalStorageAccount,
        plan: RetainedStoreStartPlan,
    ) -> Result<Self, RetainedStoreStartError> {
        let (terminal_phase, dimensions) = plan.keys_for_start_validation();
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
        let allocation_plan = RetainedAllocationPlan::new(0, 0).map_err(|_| {
            RetainedStoreStartError::Runtime(StoreRuntimeError::InvalidPublicationPlan)
        })?;
        let (
            terminal_writer,
            share,
            fallback,
            fallback_dimension,
            fallback_amount,
            snapshot_dimension,
            snapshot_amount,
        ) = plan.into_raw_start_parts();
        let raw_plan = RetainedOperationStartPlan::for_publication(
            terminal_writer,
            share,
            allocation_plan,
            fallback_dimension,
            fallback_amount,
            snapshot_dimension,
            snapshot_amount,
        );
        let (domain, ledger_limits, phases) = ledger_plan.into_parts();
        RetainedOperation::try_start_with_phase_catalog(
            control,
            ledger_limits,
            &phases,
            account,
            raw_plan,
            || Ok(fallback),
        )
        .map(|raw| Self { raw, domain })
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

    /// Returns the shared physical account used by every retained block.
    pub fn account(&self) -> &PhysicalStorageAccount {
        self.raw.account()
    }

    /// Opens fixed canonical/evidence tables after complete structural preflight.
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "rollback-safe open errors preserve the complete allocation-free operation owner"
    )]
    pub fn try_publication_assembly(
        self,
        plan: RetainedStorePublicationPlan,
    ) -> Result<RetainedStorePublicationAssembly<O, PP, U>, RetainedStoreOpenError<O, PP, U>> {
        if let Err(cause) = self.validate_publication_plan_keys(&plan) {
            return Err(RetainedStoreOpenError {
                operation: Some(self),
                cause,
            });
        }
        let domain = self.domain;
        match self.raw.try_publication_assembly(plan.into_raw()) {
            Ok(raw) => Ok(RetainedStorePublicationAssembly {
                raw,
                fallback_only: false,
                domain,
            }),
            Err(error) => {
                let cause = map_writer_error(error.cause());
                match error.try_into_operation() {
                    Ok(raw) => Err(RetainedStoreOpenError {
                        operation: Some(Self { raw, domain }),
                        cause,
                    }),
                    Err(_) => Err(RetainedStoreOpenError {
                        operation: None,
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

/// Failure to open concrete retained tables, preserving the operation only after proven rollback.
#[must_use = "recover the operation when the error reports a rollback-safe failure"]
pub struct RetainedStoreOpenError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: Option<RetainedStoreOperation<O, PP, U>>,
    cause: StoreRuntimeError,
}

impl<O, PP, U> RetainedStoreOpenError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Returns the public concrete-store cause.
    pub const fn cause(&self) -> &StoreRuntimeError {
        &self.cause
    }

    /// Recovers the operation only when every failed open step proved rollback.
    pub fn try_into_operation(self) -> Result<RetainedStoreOperation<O, PP, U>, StoreRuntimeError> {
        self.operation.ok_or(self.cause)
    }
}

impl<O, PP, U> fmt::Debug for RetainedStoreOpenError<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedStoreOpenError")
            .field("cause", &self.cause)
            .field("recoverable", &self.operation.is_some())
            .finish_non_exhaustive()
    }
}

impl<O, PP, U> fmt::Display for RetainedStoreOpenError<O, PP, U>
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

impl<O, PP, U> std::error::Error for RetainedStoreOpenError<O, PP, U>
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
pub struct RetainedStorePublicationAssembly<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreAssembly<O, PP, U>,
    fallback_only: bool,
    domain: StoreKeyDomain,
}

impl<O, PP, U> RetainedStorePublicationAssembly<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Returns the number of canonical table members.
    pub fn canonical_len(&self) -> usize {
        self.raw.canonical_len()
    }

    /// Returns the number of outcome-evidence table members.
    pub fn outcome_evidence_len(&self) -> usize {
        self.raw.outcome_evidence_len()
    }

    fn ensure_writable(&self) -> Result<(), StoreRuntimeError> {
        if self.fallback_only {
            Err(StoreRuntimeError::FallbackOnly)
        } else {
            Ok(())
        }
    }

    fn validate_dimension(&self, dimension: StoreDimensionKey) -> Result<(), StoreRuntimeError> {
        validate_store_dimension(
            &self.domain,
            dimension,
            self.raw.contains_dimension(dimension),
        )
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

    /// Shares one canonical same-account block and returns a fresh destination reference.
    pub fn try_share_canonical<SourceRootKind, SourceRootTag, SlotKind, SlotTag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalStoreRef<SlotKind, SlotTag>,
    ) -> Result<CanonicalStoreRef<SlotKind, SlotTag>, StoreRuntimeError>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let result = self
            .raw
            .try_share_canonical(source.share_source().aggregate(), source_ref.raw);
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
        self.ensure_writable()?;
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
        self.validate_dimension(dimension)?;
        let result = self.raw.try_commit_canonical(dimension, amount);
        self.map_action(result)
    }

    /// Adds one outcome-evidence destination commitment.
    pub fn try_commit_outcome_evidence(
        &mut self,
        dimension: StoreDimensionKey,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.ensure_writable()?;
        self.validate_dimension(dimension)?;
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
        let domain = self.domain;
        if self.fallback_only {
            return StoreSuccessValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Assembly(self.raw),
                domain,
            });
        }
        match self
            .raw
            .validate_success(canonical_root.raw, evidence_root.raw, |raw, _, _| {
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
        let domain = self.domain;
        if self.fallback_only {
            return StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Assembly(self.raw),
                domain,
            });
        }
        map_failure_validation(
            self.raw.validate_rich_failure(evidence_root.raw, |raw, _| {
                let mut session = StoreEvidenceValidationSession { raw };
                validate(&mut session, evidence_root)
            }),
            evidence_root,
            domain,
        )
    }

    /// Discards candidate tables and prepares only the pre-admitted fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        let domain = self.domain;
        self.raw
            .prepare_minimal_fallback()
            .map(|raw| classify_prepared_fallback(raw, domain))
            .map_err(StorePrepareError::from_raw)
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
    /// Consumes a validated success after a later domain admission rejects canonical publication.
    ///
    /// The evidence root is preserved for bounded detail appends and rich-failure revalidation.
    /// Canonical storage remains sealed until the existing terminal transition drops it before
    /// attribution, so this downgrade performs no fallible allocation or publication step.
    pub fn downgrade_to_rich_failure(
        self,
        fallback_cause: u32,
    ) -> StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag> {
        StoreFailureAssembly {
            raw: self
                .raw
                .downgrade_to_rich_failure(PreparationFailureCode::domain(fallback_cause)),
            fallback_only: false,
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

    fn ensure_writable(&self) -> Result<(), StoreRuntimeError> {
        if self.fallback_only {
            Err(StoreRuntimeError::FallbackOnly)
        } else {
            Ok(())
        }
    }

    fn validate_dimension(&self, dimension: StoreDimensionKey) -> Result<(), StoreRuntimeError> {
        validate_store_dimension(
            &self.domain,
            dimension,
            self.raw.contains_dimension(dimension),
        )
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
        self.validate_dimension(dimension)?;
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
        let domain = self.domain;
        if self.fallback_only {
            return StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::FailureAssembly(self.raw),
                domain,
            });
        }
        map_failure_validation(
            self.raw.validate_rich_failure(evidence_root.raw, |raw, _| {
                let mut session = StoreEvidenceValidationSession { raw };
                validate(&mut session, evidence_root)
            }),
            evidence_root,
            domain,
        )
    }

    /// Discards failure details and prepares only minimal fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        let domain = self.domain;
        self.raw
            .prepare_minimal_fallback()
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
                evidence_root,
                domain,
            })
        }
        RetainedFailureValidation::Rejected(raw) => {
            StoreFailureValidation::Rejected(StoreFailureAssembly {
                raw,
                fallback_only: false,
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
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
}

impl<O, PP, U, EvidenceKind, EvidenceTag> ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Freezes rich-failure evidence and its domain cause.
    pub fn try_prepare(
        self,
        cause: u32,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag>, StorePrepareError> {
        let domain = self.domain;
        self.raw
            .try_prepare(PreparationFailureCode::domain(cause), final_progress)
            .map(|raw| classify_prepared_failure(raw, self.evidence_root, domain))
            .map_err(StorePrepareError::from_raw)
    }

    /// Discards rich failure and prepares only minimal fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        let domain = self.domain;
        self.raw
            .prepare_minimal_fallback()
            .map(|raw| classify_prepared_fallback(raw, domain))
            .map_err(StorePrepareError::from_raw)
    }
}

enum StoreFallbackState<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Assembly(RawStoreAssembly<O, PP, U>),
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
    /// Discards every candidate table and prepares the assembly-independent fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback, StorePrepareError> {
        let domain = self.domain;
        let raw = match self.state {
            StoreFallbackState::Assembly(raw) => raw.prepare_minimal_fallback(),
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
