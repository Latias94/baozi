//! Operation-owned, physically metered retained storage.
//!
//! This module is the only public construction seam for retained publication payloads. The raw
//! generic owner in `retained` remains a private foundation: callers can request semantic element
//! counts, but cannot provide an owning payload or declare its allocator footprint.
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
//! Builder callbacks are contained only for one ordinary unwind. A second panic while unwinding,
//! or a build compiled with `panic = "abort"`, cannot be contained by `catch_unwind`; untrusted
//! adapters that require those boundaries must run in an isolated process.

use std::fmt;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::capacity::{CapacityPlan, CapacityPlanError};
use crate::control::{
    OperationControl, ProgressPhase, ProgressUnit, discard_untrusted_panic_payload,
};
use crate::coordinator::FinalProgress;
use crate::ledger::{LedgerCause, LedgerLimits};
use crate::owner::OperationOwner;
use crate::retained::{
    CanonicalRetainedSlot, OutcomeEvidenceRetainedSlot, PhysicalStorageAccount,
    PreparationFailureClass, PreparationFailureCode, PreparedRetainedFallback,
    PreparedRetainedPublication, PreparedRetainedPublicationOutcome, PreparedRetainedRichFailure,
    PreparedRetainedSuccess, PublishedCanonicalOutput, PublishedFallbackOutput,
    PublishedLedgerEvidence, PublishedOutcomeEvidenceOutput, PublishedRichFailureOutput,
    RetainedAllocationError, RetainedAllocationPlan, RetainedBatchLimits, RetainedBlockTablePlan,
    RetainedBuildFailureCode, RetainedClosureSession, RetainedEvidenceClosureSession,
    RetainedFailureAssembly, RetainedFailureValidation, RetainedFallbackOnly,
    RetainedHandleSharePlan, RetainedOperation, RetainedOperationStartError,
    RetainedOperationStartPlan, RetainedPublicationAssembly, RetainedPublicationError,
    RetainedPublicationPlan, RetainedPublicationPrepareError, RetainedSuccessValidation,
    RetainedWriterError, RetainedWriterPlan, ValidatedRetainedFailure, ValidatedRetainedSuccess,
};

const MAX_RECORD_FIELDS: usize = 32;
const BUILD_CODE_DOMAIN: u32 = 0x0100_0000;
const BUILD_CODE_ALLOCATION_FAILED: u32 = 1;
const BUILD_CODE_CAPACITY_UNQUALIFIED: u32 = 2;
const BUILD_CODE_CAPACITY_EXCEEDED: u32 = 3;
const BUILD_CODE_INCOMPLETE: u32 = 4;
const BUILD_CODE_ROW_WIDTH: u32 = 5;
const BUILD_CODE_BUILDER_PANICKED: u32 = 6;

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
    CapacityUnqualified,
    CapacityExceeded,
    IncompleteWrite,
    InvalidRecordWidth,
    BuilderPanicked,
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

    const fn capacity_unqualified() -> Self {
        Self {
            class: StoreBuildFailureClass::CapacityUnqualified,
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

    const fn builder_panicked() -> Self {
        Self {
            class: StoreBuildFailureClass::BuilderPanicked,
            code: 0,
        }
    }

    const fn into_raw(self) -> RetainedBuildFailureCode {
        let code = match self.class {
            StoreBuildFailureClass::Domain => BUILD_CODE_DOMAIN + self.code,
            StoreBuildFailureClass::AllocationFailed => BUILD_CODE_ALLOCATION_FAILED,
            StoreBuildFailureClass::CapacityUnqualified => BUILD_CODE_CAPACITY_UNQUALIFIED,
            StoreBuildFailureClass::CapacityExceeded => BUILD_CODE_CAPACITY_EXCEEDED,
            StoreBuildFailureClass::IncompleteWrite => BUILD_CODE_INCOMPLETE,
            StoreBuildFailureClass::InvalidRecordWidth => BUILD_CODE_ROW_WIDTH,
            StoreBuildFailureClass::BuilderPanicked => BUILD_CODE_BUILDER_PANICKED,
        };
        RetainedBuildFailureCode::new(code)
    }

    const fn from_raw(code: u32) -> Self {
        match code {
            BUILD_CODE_ALLOCATION_FAILED => Self::allocation_failed(),
            BUILD_CODE_CAPACITY_UNQUALIFIED => Self::capacity_unqualified(),
            BUILD_CODE_CAPACITY_EXCEEDED => Self::capacity_exceeded(),
            BUILD_CODE_INCOMPLETE => Self::incomplete_write(),
            BUILD_CODE_ROW_WIDTH => Self::invalid_record_width(),
            BUILD_CODE_BUILDER_PANICKED => Self::builder_panicked(),
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
            StoreBuildFailureClass::CapacityUnqualified => {
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
            StoreBuildFailureClass::BuilderPanicked => {
                formatter.write_str("retained store builder panicked")
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
    marker: PhantomData<fn() -> T>,
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
    capacity
        .finish(values.len(), values.capacity())
        .map_err(|_| StoreBuildError::capacity_unqualified())?;
    let qualified_capacity = values.capacity();
    Ok((values, qualified_capacity))
}

fn finish_scoped_writer<Writer>(
    mut writer: Writer,
    build: impl FnOnce(&mut Writer) -> Result<(), StoreBuildError>,
    finish: impl FnOnce(Writer) -> Result<StoredBlock, StoreBuildError>,
) -> Result<StoredBlock, StoreBuildError> {
    match catch_unwind(AssertUnwindSafe(|| build(&mut writer))) {
        Ok(result) => {
            result?;
            finish(writer)
        }
        Err(payload) => {
            // Every concrete writer owns only sealed primitive storage whose destructor cannot
            // execute caller code. Destroy it before touching the untrusted panic payload.
            drop(writer);
            discard_untrusted_panic_payload(payload);
            Err(StoreBuildError::builder_panicked())
        }
    }
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
    marker: PhantomData<fn() -> T>,
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
            return Err(StoreBuildError::capacity_unqualified());
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
                        Ok(())
                    }
                    _ => Err(StoreBuildError::capacity_unqualified()),
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
            return Err(StoreBuildError::capacity_unqualified());
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
        plan.capacity
            .finish(value.len(), value.capacity())
            .map_err(|_| StoreBuildError::capacity_unqualified())?;
        let qualified_capacity = value.capacity();
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
            return Err(StoreBuildError::capacity_unqualified());
        }
        Ok(StoredBlock::Utf8(self.value))
    }
}

/// Marker for a scalar-sequence block reference.
pub struct ScalarSequence<T: StoredScalar>(PhantomData<fn() -> T>);

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
    marker: PhantomData<fn() -> Kind>,
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
    marker: PhantomData<fn() -> Kind>,
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
            return Err(StoreBuildError::capacity_unqualified());
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

fn sequence_view<T: StoredScalar>(block: &StoredBlock) -> Option<SequenceView<'_, T>> {
    let StoredBlock::Sequence(storage) = block else {
        return None;
    };
    if storage.kind() != <T as sealed::StoredScalar>::KIND {
        return None;
    }
    let values = <T as sealed::StoredScalar>::as_slice(storage)?;
    Some(SequenceView { values })
}

fn bytes_view(block: &StoredBlock) -> Option<&[u8]> {
    match block {
        StoredBlock::Bytes(values) => Some(values),
        _ => None,
    }
}

fn utf8_view(block: &StoredBlock) -> Option<&str> {
    match block {
        StoredBlock::Utf8(value) => Some(value),
        _ => None,
    }
}

fn canonical_record_view(block: &StoredBlock) -> Option<CanonicalRecordView<'_>> {
    match block {
        StoredBlock::Records(storage) => Some(CanonicalRecordView { storage }),
        _ => None,
    }
}

fn outcome_evidence_record_view(block: &StoredBlock) -> Option<OutcomeEvidenceRecordView<'_>> {
    match block {
        StoredBlock::Records(storage) => Some(OutcomeEvidenceRecordView { storage }),
        _ => None,
    }
}

/// Read-only candidate session used to prove exact canonical and evidence reachability.
pub struct StoreValidationSession<'session, 'storage> {
    raw: &'session mut RetainedClosureSession<'storage, StoredBlock, StoredBlock>,
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
    raw: &'session mut RetainedEvidenceClosureSession<'storage, StoredBlock>,
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

/// Allocation-free terminal fallback payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StoreFallbackEnvelope {
    code: u32,
}

impl StoreFallbackEnvelope {
    /// Creates an application-defined fallback code.
    pub const fn new(code: u32) -> Self {
        Self { code }
    }

    /// Returns the application-defined fallback code.
    pub const fn code(self) -> u32 {
        self.code
    }
}

/// Fixed capacities for one concrete retained writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreBatchLimits {
    attribution_slots: usize,
    commitment_slots: usize,
}

impl RetainedStoreBatchLimits {
    /// Creates non-zero fixed capacities.
    pub fn new(attribution_slots: usize, commitment_slots: usize) -> Result<Self, StorePlanError> {
        if attribution_slots == 0 || commitment_slots == 0 {
            return Err(StorePlanError::InvalidBlockTable);
        }
        Ok(Self {
            attribution_slots,
            commitment_slots,
        })
    }

    /// Returns the attribution-slot capacity.
    pub const fn attribution_slots(self) -> usize {
        self.attribution_slots
    }

    /// Returns the commitment-slot capacity.
    pub const fn commitment_slots(self) -> usize {
        self.commitment_slots
    }

    const fn into_raw(self) -> RetainedBatchLimits {
        RetainedBatchLimits::new(self.attribution_slots, self.commitment_slots)
    }
}

/// Ledger placement and fixed capacities for one concrete retained writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreWriterPlan<P, D> {
    phase: P,
    capacity_dimension: D,
    attribution_dimension: D,
    limits: RetainedStoreBatchLimits,
}

impl<P: Copy, D: Copy> RetainedStoreWriterPlan<P, D> {
    /// Creates a writer plan without entering the ledger or allocator.
    pub const fn new(
        phase: P,
        capacity_dimension: D,
        attribution_dimension: D,
        limits: RetainedStoreBatchLimits,
    ) -> Self {
        Self {
            phase,
            capacity_dimension,
            attribution_dimension,
            limits,
        }
    }

    const fn into_raw(self) -> RetainedWriterPlan<P, D> {
        RetainedWriterPlan::new(
            self.phase,
            self.capacity_dimension,
            self.attribution_dimension,
            self.limits.into_raw(),
        )
    }
}

/// Work dimensions charged before a same-account retained handle is shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreSharePlan<D> {
    clone_dimension: D,
    retained_handle_dimension: D,
}

impl<D: Copy> RetainedStoreSharePlan<D> {
    /// Creates the work plan used by every inherited block share.
    pub const fn new(clone_dimension: D, retained_handle_dimension: D) -> Self {
        Self {
            clone_dimension,
            retained_handle_dimension,
        }
    }

    const fn into_raw(self) -> RetainedHandleSharePlan<D> {
        RetainedHandleSharePlan::new(self.clone_dimension, self.retained_handle_dimension)
    }
}

/// Start-time policy for the operation-owned concrete retained store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreStartPlan<P, D> {
    terminal_writer: RetainedStoreWriterPlan<P, D>,
    share: RetainedStoreSharePlan<D>,
    fallback: StoreFallbackEnvelope,
    fallback_dimension: D,
    fallback_amount: u64,
    snapshot_dimension: D,
    snapshot_amount: u64,
}

impl<P: Copy, D: Copy> RetainedStoreStartPlan<P, D> {
    /// Creates start policy for the pre-admitted fallback and frozen ledger evidence.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        terminal_writer: RetainedStoreWriterPlan<P, D>,
        share: RetainedStoreSharePlan<D>,
        fallback: StoreFallbackEnvelope,
        fallback_dimension: D,
        fallback_amount: u64,
        snapshot_dimension: D,
        snapshot_amount: u64,
    ) -> Self {
        Self {
            terminal_writer,
            share,
            fallback,
            fallback_dimension,
            fallback_amount,
            snapshot_dimension,
            snapshot_amount,
        }
    }
}

/// Fixed table capacity and maximum new allocations for one publication destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreBlockTablePlan<P, D> {
    writer: RetainedStoreWriterPlan<P, D>,
    total_blocks: usize,
    max_new_blocks: usize,
}

impl<P: Copy, D: Copy> RetainedStoreBlockTablePlan<P, D> {
    /// Creates a non-empty table plan and rejects impossible new-block bounds.
    pub fn new(
        writer: RetainedStoreWriterPlan<P, D>,
        total_blocks: usize,
        max_new_blocks: usize,
    ) -> Result<Self, StorePlanError> {
        let required_attributions = max_new_blocks
            .checked_add(1)
            .ok_or(StorePlanError::InvalidBlockTable)?;
        if total_blocks == 0
            || max_new_blocks > total_blocks
            || u64::try_from(total_blocks).is_err()
            || required_attributions > writer.limits.attribution_slots
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        Ok(Self {
            writer,
            total_blocks,
            max_new_blocks,
        })
    }

    const fn into_raw(self) -> RetainedBlockTablePlan<P, D> {
        RetainedBlockTablePlan::new(
            self.writer.into_raw(),
            self.total_blocks,
            self.max_new_blocks,
        )
    }
}

/// Independent canonical and outcome-evidence table plans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStorePublicationPlan<P, D> {
    canonical: Option<RetainedStoreBlockTablePlan<P, D>>,
    outcome_evidence: RetainedStoreBlockTablePlan<P, D>,
}

impl<P: Copy, D: Copy> RetainedStorePublicationPlan<P, D> {
    /// Creates a success-capable dual-table publication plan.
    pub const fn success(
        canonical: RetainedStoreBlockTablePlan<P, D>,
        outcome_evidence: RetainedStoreBlockTablePlan<P, D>,
    ) -> Self {
        Self {
            canonical: Some(canonical),
            outcome_evidence,
        }
    }

    /// Creates an evidence-only publication plan.
    pub const fn rich_failure(outcome_evidence: RetainedStoreBlockTablePlan<P, D>) -> Self {
        Self {
            canonical: None,
            outcome_evidence,
        }
    }

    const fn into_raw(self) -> RetainedPublicationPlan<P, D> {
        match self.canonical {
            Some(canonical) => RetainedPublicationPlan::success(
                canonical.into_raw(),
                self.outcome_evidence.into_raw(),
            ),
            None => RetainedPublicationPlan::rich_failure(self.outcome_evidence.into_raw()),
        }
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
pub enum StoreRuntimeError<D> {
    Plan(StorePlanError),
    BatchCapacity,
    BatchAllocationFailed,
    Ledger(LedgerCause<D>),
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
    BuilderPanicked {
        charge_retained: bool,
    },
    AllocationFailed {
        charge_retained: bool,
    },
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
    FallbackOnly,
    Poisoned,
}

impl<D: fmt::Debug> fmt::Display for StoreRuntimeError<D> {
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
            Self::BuilderPanicked { charge_retained } => write!(
                formatter,
                "retained store builder panicked; fail-closed charge retained: {charge_retained}"
            ),
            Self::AllocationFailed { charge_retained } => write!(
                formatter,
                "retained store allocation failed; fail-closed charge retained: {charge_retained}"
            ),
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
            Self::FallbackOnly => {
                formatter.write_str("retained store is restricted to minimal fallback")
            }
            Self::Poisoned => formatter.write_str("retained store ownership is poisoned"),
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for StoreRuntimeError<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Plan(cause) => Some(cause),
            Self::Ledger(cause) => Some(cause),
            Self::Build(cause) => Some(cause),
            _ => None,
        }
    }
}

fn map_allocation_error<D>(error: &RetainedAllocationError) -> StoreRuntimeError<D> {
    match error {
        RetainedAllocationError::InlinePayloadTooLarge { .. } => {
            StoreRuntimeError::InvalidPublicationPlan
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
            let cause = StoreBuildError::from_raw(code.code());
            if cause.class() == StoreBuildFailureClass::BuilderPanicked {
                StoreRuntimeError::BuilderPanicked {
                    charge_retained: false,
                }
            } else {
                StoreRuntimeError::Build(cause)
            }
        }
        RetainedAllocationError::BuilderPanicked { charge_retained } => {
            StoreRuntimeError::BuilderPanicked {
                charge_retained: *charge_retained,
            }
        }
        RetainedAllocationError::AllocationFailed { charge_retained } => {
            StoreRuntimeError::AllocationFailed {
                charge_retained: *charge_retained,
            }
        }
    }
}

fn map_writer_error<D: Copy>(error: &RetainedWriterError<D>) -> StoreRuntimeError<D> {
    match error {
        RetainedWriterError::BatchCapacity => StoreRuntimeError::BatchCapacity,
        RetainedWriterError::BatchAllocationFailed => StoreRuntimeError::BatchAllocationFailed,
        RetainedWriterError::Ledger(cause) => StoreRuntimeError::Ledger(cause.clone()),
        RetainedWriterError::Physical(cause) => map_allocation_error(cause),
        RetainedWriterError::AttributionBatchFull
        | RetainedWriterError::CommitmentBatchFull
        | RetainedWriterError::InvalidPublicationPlan => StoreRuntimeError::InvalidPublicationPlan,
        RetainedWriterError::ForeignAccount => StoreRuntimeError::ForeignAccount,
        RetainedWriterError::MembershipExhausted => StoreRuntimeError::MembershipExhausted,
        RetainedWriterError::StorageDropUnverified => StoreRuntimeError::StorageDropUnverified,
    }
}

fn map_publication_error<D: Copy>(error: &RetainedPublicationError<D>) -> StoreRuntimeError<D> {
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

fn publication_error_is_builder_panic<D>(error: &RetainedPublicationError<D>) -> bool {
    matches!(
        error,
        RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::BuilderPanicked { .. }
        ))
    ) || matches!(
        error,
        RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::Build(code)
        )) if StoreBuildError::from_raw(code.code()).class()
            == StoreBuildFailureClass::BuilderPanicked
    )
}

/// Failure while starting a concrete retained store operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetainedStoreStartError<D> {
    InvalidTerminalLimits,
    Runtime(StoreRuntimeError<D>),
}

impl<D: fmt::Debug> fmt::Display for RetainedStoreStartError<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTerminalLimits => {
                formatter.write_str("retained store terminal limits are invalid")
            }
            Self::Runtime(cause) => cause.fmt(formatter),
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for RetainedStoreStartError<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(cause) => Some(cause),
            Self::InvalidTerminalLimits => None,
        }
    }
}

type RawStoreOperation<O, P, PP, U, D> = RetainedOperation<O, P, PP, U, D, StoreFallbackEnvelope>;
type RawStoreAssembly<O, P, PP, U, D> =
    RetainedPublicationAssembly<O, P, PP, U, D, StoreFallbackEnvelope, StoredBlock, StoredBlock>;
type RawStoreFailureAssembly<O, P, PP, U, D> =
    RetainedFailureAssembly<O, P, PP, U, D, StoreFallbackEnvelope, StoredBlock, StoredBlock>;
type RawValidatedStoreSuccess<O, P, PP, U, D> =
    ValidatedRetainedSuccess<O, P, PP, U, D, StoreFallbackEnvelope, StoredBlock, StoredBlock>;
type RawValidatedStoreFailure<O, P, PP, U, D> =
    ValidatedRetainedFailure<O, P, PP, U, D, StoreFallbackEnvelope, StoredBlock, StoredBlock>;
type RawStoreFallbackOnly<O, P, PP, U, D> =
    RetainedFallbackOnly<O, P, PP, U, D, StoreFallbackEnvelope, StoredBlock, StoredBlock>;
type RawPreparedStore<D> =
    PreparedRetainedPublication<StoredBlock, StoredBlock, StoreFallbackEnvelope, D>;
type RawPreparedStoreSuccess<D> = PreparedRetainedSuccess<StoredBlock, StoredBlock, D>;
type RawPreparedStoreFailure<D> = PreparedRetainedRichFailure<StoredBlock, D>;
type RawPreparedStoreFallback<D> = PreparedRetainedFallback<StoreFallbackEnvelope, D>;

fn classify_prepared_success<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>(
    raw: RawPreparedStore<D>,
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
) -> PreparedStoreSuccessOutcome<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    match raw.into_outcome() {
        PreparedRetainedPublicationOutcome::Success(raw) => {
            PreparedStoreSuccessOutcome::Success(PreparedStoreSuccess {
                raw,
                canonical_root,
                evidence_root,
            })
        }
        PreparedRetainedPublicationOutcome::RichFailure(raw) => {
            PreparedStoreSuccessOutcome::RichFailure(PreparedStoreFailure { raw, evidence_root })
        }
        PreparedRetainedPublicationOutcome::MinimalFallback(raw) => {
            PreparedStoreSuccessOutcome::MinimalFallback(PreparedStoreFallback { raw })
        }
    }
}

fn classify_prepared_failure<EvidenceKind, EvidenceTag, D>(
    raw: RawPreparedStore<D>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
) -> PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    match raw.into_outcome() {
        PreparedRetainedPublicationOutcome::RichFailure(raw) => {
            PreparedStoreFailureOutcome::RichFailure(PreparedStoreFailure { raw, evidence_root })
        }
        PreparedRetainedPublicationOutcome::MinimalFallback(raw) => {
            PreparedStoreFailureOutcome::MinimalFallback(PreparedStoreFallback { raw })
        }
        PreparedRetainedPublicationOutcome::Success(_) => unreachable_store_state(),
    }
}

fn classify_prepared_fallback<D>(raw: RawPreparedStore<D>) -> PreparedStoreFallback<D>
where
    D: Copy + Ord,
{
    match raw.into_outcome() {
        PreparedRetainedPublicationOutcome::MinimalFallback(raw) => PreparedStoreFallback { raw },
        PreparedRetainedPublicationOutcome::Success(_)
        | PreparedRetainedPublicationOutcome::RichFailure(_) => unreachable_store_state(),
    }
}

/// Linear owner for one coordinator, physical account, and pre-admitted fallback.
#[must_use = "the retained store operation must be opened or published as fallback"]
pub struct RetainedStoreOperation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    raw: RawStoreOperation<O, P, PP, U, D>,
}

impl<O, P, PP, U, D> RetainedStoreOperation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    /// Starts one operation and pre-admits its allocation-free minimal fallback.
    pub fn try_start(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        account: &PhysicalStorageAccount,
        plan: RetainedStoreStartPlan<P, D>,
    ) -> Result<Self, RetainedStoreStartError<D>> {
        let allocation_plan = RetainedAllocationPlan::new(0, 0).map_err(|_| {
            RetainedStoreStartError::Runtime(StoreRuntimeError::InvalidPublicationPlan)
        })?;
        let raw_plan = RetainedOperationStartPlan::for_publication(
            plan.terminal_writer.into_raw(),
            plan.share.into_raw(),
            allocation_plan,
            plan.fallback_dimension,
            plan.fallback_amount,
            plan.snapshot_dimension,
            plan.snapshot_amount,
        );
        RetainedOperation::try_start(control, ledger_limits, account, raw_plan, || {
            Ok(plan.fallback)
        })
        .map(|raw| Self { raw })
        .map_err(|cause| match cause {
            RetainedOperationStartError::InvalidTerminalLimits => {
                RetainedStoreStartError::InvalidTerminalLimits
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
        plan: RetainedStorePublicationPlan<P, D>,
    ) -> Result<
        RetainedStorePublicationAssembly<O, P, PP, U, D>,
        RetainedStoreOpenError<O, P, PP, U, D>,
    > {
        match self.raw.try_publication_assembly(plan.into_raw()) {
            Ok(raw) => Ok(RetainedStorePublicationAssembly {
                raw,
                fallback_only: false,
            }),
            Err(error) => {
                let cause = map_writer_error(error.cause());
                match error.try_into_operation() {
                    Ok(raw) => Err(RetainedStoreOpenError {
                        operation: Some(Self { raw }),
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
    pub fn prepare_minimal_fallback(self, cause: u32) -> PreparedStoreFallback<D> {
        classify_prepared_fallback(
            self.raw
                .prepare_minimal_fallback(PreparationFailureCode::domain(cause)),
        )
    }
}

/// Failure to open concrete retained tables, preserving the operation only after proven rollback.
#[must_use = "recover the operation when the error reports a rollback-safe failure"]
pub struct RetainedStoreOpenError<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    operation: Option<RetainedStoreOperation<O, P, PP, U, D>>,
    cause: StoreRuntimeError<D>,
}

impl<O, P, PP, U, D> RetainedStoreOpenError<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    /// Returns the public concrete-store cause.
    pub const fn cause(&self) -> &StoreRuntimeError<D> {
        &self.cause
    }

    /// Recovers the operation only when every failed open step proved rollback.
    pub fn try_into_operation(
        self,
    ) -> Result<RetainedStoreOperation<O, P, PP, U, D>, StoreRuntimeError<D>> {
        self.operation.ok_or(self.cause)
    }
}

impl<O, P, PP, U, D> fmt::Debug for RetainedStoreOpenError<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedStoreOpenError")
            .field("cause", &self.cause)
            .field("recoverable", &self.operation.is_some())
            .finish_non_exhaustive()
    }
}

impl<O, P, PP, U, D> fmt::Display for RetainedStoreOpenError<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "concrete retained publication could not be opened: {}",
            self.cause
        )
    }
}

impl<O, P, PP, U, D> std::error::Error for RetainedStoreOpenError<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + 'static,
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
pub struct RetainedStorePublicationAssembly<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    raw: RawStoreAssembly<O, P, PP, U, D>,
    fallback_only: bool,
}

impl<O, P, PP, U, D> RetainedStorePublicationAssembly<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    /// Returns the number of canonical table members.
    pub fn canonical_len(&self) -> usize {
        self.raw.canonical_len()
    }

    /// Returns the number of outcome-evidence table members.
    pub fn outcome_evidence_len(&self) -> usize {
        self.raw.outcome_evidence_len()
    }

    fn ensure_writable(&self) -> Result<(), StoreRuntimeError<D>> {
        if self.fallback_only {
            Err(StoreRuntimeError::FallbackOnly)
        } else {
            Ok(())
        }
    }

    fn map_action<T>(
        &mut self,
        result: Result<T, RetainedPublicationError<D>>,
    ) -> Result<T, StoreRuntimeError<D>> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if publication_error_is_builder_panic(&error)
                    || matches!(
                        error,
                        RetainedPublicationError::Poisoned
                            | RetainedPublicationError::Writer(
                                RetainedWriterError::StorageDropUnverified
                            )
                    )
                {
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
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError<D>>
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
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreRuntimeError<D>>
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
    ) -> Result<CanonicalBytesRef<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<CanonicalUtf8Ref<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<OutcomeEvidenceRecordRef<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<CanonicalStoreRef<SlotKind, SlotTag>, StoreRuntimeError<D>>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let result = self
            .raw
            .try_share_canonical(source.raw.aggregate(), source_ref.raw);
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
    ) -> Result<OutcomeEvidenceStoreRef<SlotKind, SlotTag>, StoreRuntimeError<D>>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let result = self
            .raw
            .try_share_outcome_evidence(source.raw.aggregate(), source_ref.raw);
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Adds one canonical destination commitment.
    pub fn try_commit_canonical(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), StoreRuntimeError<D>> {
        self.ensure_writable()?;
        let result = self.raw.try_commit_canonical(dimension, amount);
        self.map_action(result)
    }

    /// Adds one outcome-evidence destination commitment.
    pub fn try_commit_outcome_evidence(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), StoreRuntimeError<D>> {
        self.ensure_writable()?;
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
    ) -> StoreSuccessValidation<
        O,
        P,
        PP,
        U,
        D,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
    >
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        if self.fallback_only {
            return StoreSuccessValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Assembly(self.raw),
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
                })
            }
            RetainedSuccessValidation::Rejected(raw) => {
                StoreSuccessValidation::Rejected(StoreFailureAssembly {
                    raw,
                    fallback_only: false,
                    evidence_root,
                })
            }
            RetainedSuccessValidation::FallbackOnly(raw) => {
                StoreSuccessValidation::FallbackOnly(StoreFallbackOnly {
                    state: StoreFallbackState::Validated(raw),
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
    ) -> StoreFailureValidation<O, P, PP, U, D, EvidenceKind, EvidenceTag>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        if self.fallback_only {
            return StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Assembly(self.raw),
            });
        }
        map_failure_validation(
            self.raw.validate_rich_failure(evidence_root.raw, |raw, _| {
                let mut session = StoreEvidenceValidationSession { raw };
                validate(&mut session, evidence_root)
            }),
            evidence_root,
        )
    }

    /// Discards candidate tables and prepares only the pre-admitted fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback<D>, StorePrepareError> {
        self.raw
            .prepare_minimal_fallback()
            .map(classify_prepared_fallback)
            .map_err(StorePrepareError::from_raw)
    }
}

/// Linear result of exact success-graph validation.
#[must_use = "every validation branch retains the complete publication owner"]
pub enum StoreSuccessValidation<
    O,
    P,
    PP,
    U,
    D,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    Valid(
        ValidatedStoreSuccess<
            O,
            P,
            PP,
            U,
            D,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
    ),
    Rejected(StoreFailureAssembly<O, P, PP, U, D, EvidenceKind, EvidenceTag>),
    FallbackOnly(StoreFallbackOnly<O, P, PP, U, D>),
}

/// Exact success closure whose graph can no longer be mutated.
#[must_use = "validated retained success must be prepared or deliberately dropped"]
pub struct ValidatedStoreSuccess<
    O,
    P,
    PP,
    U,
    D,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    raw: RawValidatedStoreSuccess<O, P, PP, U, D>,
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
}

impl<O, P, PP, U, D, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    ValidatedStoreSuccess<O, P, PP, U, D, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    /// Consumes a validated success after a later domain admission rejects canonical publication.
    ///
    /// The evidence root is preserved for bounded detail appends and rich-failure revalidation.
    /// Canonical storage remains sealed until the existing terminal transition drops it before
    /// attribution, so this downgrade performs no fallible allocation or publication step.
    pub fn downgrade_to_rich_failure(
        self,
        fallback_cause: u32,
    ) -> StoreFailureAssembly<O, P, PP, U, D, EvidenceKind, EvidenceTag> {
        StoreFailureAssembly {
            raw: self
                .raw
                .downgrade_to_rich_failure(PreparationFailureCode::domain(fallback_cause)),
            fallback_only: false,
            evidence_root: self.evidence_root,
        }
    }

    /// Freezes the terminal ledger destinations and transfers operation live bytes to zero.
    pub fn try_prepare(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        PreparedStoreSuccessOutcome<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>,
        StorePrepareError,
    > {
        self.raw
            .try_prepare(final_progress)
            .map(|raw| classify_prepared_success(raw, self.canonical_root, self.evidence_root))
            .map_err(StorePrepareError::from_raw)
    }

    /// Discards the validated candidate and prepares only minimal fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback<D>, StorePrepareError> {
        self.raw
            .prepare_minimal_fallback()
            .map(classify_prepared_fallback)
            .map_err(StorePrepareError::from_raw)
    }
}

/// Evidence-only builder returned by semantic success rejection.
///
/// Its retained-block builder callbacks have the same single-unwind containment boundary as the
/// success-capable assembly; double panics and aborting runtimes require process isolation.
#[must_use = "the failure assembly must be validated or published as fallback"]
pub struct StoreFailureAssembly<O, P, PP, U, D, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    raw: RawStoreFailureAssembly<O, P, PP, U, D>,
    fallback_only: bool,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
}

impl<O, P, PP, U, D, PreservedEvidenceKind, PreservedEvidenceTag>
    StoreFailureAssembly<O, P, PP, U, D, PreservedEvidenceKind, PreservedEvidenceTag>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
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

    fn ensure_writable(&self) -> Result<(), StoreRuntimeError<D>> {
        if self.fallback_only {
            Err(StoreRuntimeError::FallbackOnly)
        } else {
            Ok(())
        }
    }

    fn map_action<T>(
        &mut self,
        result: Result<T, RetainedPublicationError<D>>,
    ) -> Result<T, StoreRuntimeError<D>> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if publication_error_is_builder_panic(&error)
                    || matches!(
                        error,
                        RetainedPublicationError::Poisoned
                            | RetainedPublicationError::Writer(
                                RetainedWriterError::StorageDropUnverified
                            )
                    )
                {
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
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreRuntimeError<D>>
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
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<OutcomeEvidenceRecordRef<Tag>, StoreRuntimeError<D>>
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
    ) -> Result<OutcomeEvidenceStoreRef<SlotKind, SlotTag>, StoreRuntimeError<D>>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()?;
        let result = self
            .raw
            .try_share_outcome_evidence(source.raw.aggregate(), source_ref.raw);
        self.map_action(result).map(|raw| OutcomeEvidenceStoreRef {
            raw,
            marker: PhantomData,
        })
    }

    /// Adds one rich-failure evidence commitment.
    pub fn try_commit(&mut self, dimension: D, amount: u64) -> Result<(), StoreRuntimeError<D>> {
        self.ensure_writable()?;
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
    ) -> StoreFailureValidation<O, P, PP, U, D, EvidenceKind, EvidenceTag>
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        if self.fallback_only {
            return StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::FailureAssembly(self.raw),
            });
        }
        map_failure_validation(
            self.raw.validate_rich_failure(evidence_root.raw, |raw, _| {
                let mut session = StoreEvidenceValidationSession { raw };
                validate(&mut session, evidence_root)
            }),
            evidence_root,
        )
    }

    /// Discards failure details and prepares only minimal fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback<D>, StorePrepareError> {
        self.raw
            .prepare_minimal_fallback()
            .map(classify_prepared_fallback)
            .map_err(StorePrepareError::from_raw)
    }
}

/// Linear result of exact rich-failure validation.
#[must_use = "every failure-validation branch retains the publication owner"]
pub enum StoreFailureValidation<O, P, PP, U, D, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    Valid(ValidatedStoreFailure<O, P, PP, U, D, EvidenceKind, EvidenceTag>),
    Rejected(StoreFailureAssembly<O, P, PP, U, D, EvidenceKind, EvidenceTag>),
    FallbackOnly(StoreFallbackOnly<O, P, PP, U, D>),
}

fn map_failure_validation<O, P, PP, U, D, EvidenceKind, EvidenceTag>(
    validation: RetainedFailureValidation<
        O,
        P,
        PP,
        U,
        D,
        StoreFallbackEnvelope,
        StoredBlock,
        StoredBlock,
    >,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
) -> StoreFailureValidation<O, P, PP, U, D, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    match validation {
        RetainedFailureValidation::Valid(raw) => {
            StoreFailureValidation::Valid(ValidatedStoreFailure { raw, evidence_root })
        }
        RetainedFailureValidation::Rejected(raw) => {
            StoreFailureValidation::Rejected(StoreFailureAssembly {
                raw,
                fallback_only: false,
                evidence_root,
            })
        }
        RetainedFailureValidation::FallbackOnly(raw) => {
            StoreFailureValidation::FallbackOnly(StoreFallbackOnly {
                state: StoreFallbackState::Validated(raw),
            })
        }
    }
}

/// Exact rich-failure closure whose graph can no longer be mutated.
#[must_use = "validated rich failure must be prepared or deliberately dropped"]
pub struct ValidatedStoreFailure<O, P, PP, U, D, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    raw: RawValidatedStoreFailure<O, P, PP, U, D>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
}

impl<O, P, PP, U, D, EvidenceKind, EvidenceTag>
    ValidatedStoreFailure<O, P, PP, U, D, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    /// Freezes rich-failure evidence and its domain cause.
    pub fn try_prepare(
        self,
        cause: u32,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag, D>, StorePrepareError> {
        self.raw
            .try_prepare(PreparationFailureCode::domain(cause), final_progress)
            .map(|raw| classify_prepared_failure(raw, self.evidence_root))
            .map_err(StorePrepareError::from_raw)
    }

    /// Discards rich failure and prepares only minimal fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback<D>, StorePrepareError> {
        self.raw
            .prepare_minimal_fallback()
            .map(classify_prepared_fallback)
            .map_err(StorePrepareError::from_raw)
    }
}

enum StoreFallbackState<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    Assembly(RawStoreAssembly<O, P, PP, U, D>),
    FailureAssembly(RawStoreFailureAssembly<O, P, PP, U, D>),
    Validated(RawStoreFallbackOnly<O, P, PP, U, D>),
}

/// Terminal-only state after panic, invalid root, or poisoned graph containment.
#[must_use = "fallback-only state can publish only the pre-admitted minimal fallback"]
pub struct StoreFallbackOnly<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    state: StoreFallbackState<O, P, PP, U, D>,
}

impl<O, P, PP, U, D> StoreFallbackOnly<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    /// Discards every candidate table and prepares the assembly-independent fallback.
    pub fn prepare_minimal_fallback(self) -> Result<PreparedStoreFallback<D>, StorePrepareError> {
        let raw = match self.state {
            StoreFallbackState::Assembly(raw) => raw.prepare_minimal_fallback(),
            StoreFallbackState::FailureAssembly(raw) => raw.prepare_minimal_fallback(),
            StoreFallbackState::Validated(raw) => raw.prepare_minimal_fallback(),
        };
        raw.map(classify_prepared_fallback)
            .map_err(StorePrepareError::from_raw)
    }
}

/// Failure while atomically preparing a retained store publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorePrepareError {
    Poisoned,
    InvalidStorageGraph,
    /// Terminal preparation consumed the operation but could not prove that discarded storage,
    /// attribution, and admission leases were released.
    ///
    /// This failure is non-retryable and returns no success, rich-failure, or fallback owner. The
    /// uncertain charge is deliberately retained fail-closed, so the affected physical account
    /// and operation runtime must be quarantined rather than reused.
    StorageDropUnverified,
    PrimaryPanicked,
}

impl StorePrepareError {
    const fn from_raw(error: RetainedPublicationPrepareError) -> Self {
        match error {
            RetainedPublicationPrepareError::Poisoned => Self::Poisoned,
            RetainedPublicationPrepareError::InvalidStorageGraph => Self::InvalidStorageGraph,
            RetainedPublicationPrepareError::StorageDropUnverified => Self::StorageDropUnverified,
            RetainedPublicationPrepareError::PrimaryPanicked => Self::PrimaryPanicked,
        }
    }
}

impl fmt::Display for StorePrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Poisoned => formatter.write_str("retained store preparation is poisoned"),
            Self::InvalidStorageGraph => {
                formatter.write_str("retained store ownership graph is invalid")
            }
            Self::StorageDropUnverified => {
                formatter.write_str("retained store release could not be verified")
            }
            Self::PrimaryPanicked => {
                formatter.write_str("retained store primary ledger preparation panicked")
            }
        }
    }
}

impl std::error::Error for StorePrepareError {}

/// Exact preparation outcome for a success candidate.
#[must_use = "every exact terminal branch owns the frozen publication"]
pub enum PreparedStoreSuccessOutcome<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    Success(PreparedStoreSuccess<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>),
    RichFailure(PreparedStoreFailure<EvidenceKind, EvidenceTag, D>),
    MinimalFallback(PreparedStoreFallback<D>),
}

/// Exact preparation outcome for a rich-failure candidate.
#[must_use = "every exact terminal branch owns the frozen publication"]
pub enum PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    RichFailure(PreparedStoreFailure<EvidenceKind, EvidenceTag, D>),
    MinimalFallback(PreparedStoreFallback<D>),
}

/// Exact prepared success after attribution has been frozen.
#[must_use = "prepared success must be handed off or deliberately dropped"]
pub struct PreparedStoreSuccess<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    raw: RawPreparedStoreSuccess<D>,
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
}

impl<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>
    PreparedStoreSuccess<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    /// Performs the allocation-free handoff directly into exact success owners.
    pub fn handoff(
        self,
    ) -> PublishedStoreSuccessParts<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D> {
        let raw = self.raw.handoff();
        PublishedStoreSuccessParts {
            canonical: PublishedCanonicalStore {
                raw: raw.canonical,
                root: self.canonical_root,
            },
            outcome_evidence: PublishedOutcomeEvidenceStore {
                raw: raw.outcome_evidence,
                root: self.evidence_root,
            },
            ledger_evidence: StoreLedgerEvidence {
                raw: raw.ledger_evidence,
            },
        }
    }
}

/// Exact prepared rich failure after attribution has been frozen.
#[must_use = "prepared rich failure must be handed off or deliberately dropped"]
pub struct PreparedStoreFailure<EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    raw: RawPreparedStoreFailure<D>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
}

impl<EvidenceKind, EvidenceTag, D> PreparedStoreFailure<EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    /// Performs the allocation-free handoff directly into exact rich-failure owners.
    pub fn handoff(self) -> PublishedStoreFailureParts<EvidenceKind, EvidenceTag, D> {
        let raw = self.raw.handoff();
        PublishedStoreFailureParts {
            outcome_evidence: PublishedRichFailureStore {
                raw: raw.outcome_evidence,
                root: self.evidence_root,
            },
            ledger_evidence: StoreLedgerEvidence {
                raw: raw.ledger_evidence,
            },
        }
    }
}

/// Exact prepared minimal fallback after attribution has been frozen.
#[must_use = "prepared fallback must be handed off or deliberately dropped"]
pub struct PreparedStoreFallback<D>
where
    D: Copy + Ord,
{
    raw: RawPreparedStoreFallback<D>,
}

impl<D: Copy + Ord> PreparedStoreFallback<D> {
    /// Performs the allocation-free handoff directly into exact fallback owners.
    pub fn handoff(self) -> PublishedStoreFallbackParts<D> {
        let raw = self.raw.handoff();
        PublishedStoreFallbackParts {
            fallback: PublishedStoreFallbackOutput { raw: raw.fallback },
            ledger_evidence: StoreLedgerEvidence {
                raw: raw.ledger_evidence,
            },
        }
    }
}

/// Exact published success owners after pure handoff.
#[must_use = "the split contains every success owner and frozen ledger evidence"]
pub struct PublishedStoreSuccessParts<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    pub canonical: PublishedCanonicalStore<CanonicalKind, CanonicalTag>,
    pub outcome_evidence: PublishedOutcomeEvidenceStore<EvidenceKind, EvidenceTag>,
    pub ledger_evidence: StoreLedgerEvidence<D>,
}

/// Exact published rich-failure owners after pure handoff.
#[must_use = "the split contains rich-failure evidence and frozen ledger evidence"]
pub struct PublishedStoreFailureParts<EvidenceKind, EvidenceTag, D>
where
    D: Copy + Ord,
{
    pub outcome_evidence: PublishedRichFailureStore<EvidenceKind, EvidenceTag>,
    pub ledger_evidence: StoreLedgerEvidence<D>,
}

/// Exact published fallback owners after pure handoff.
#[must_use = "the split contains fallback ownership and frozen ledger evidence"]
pub struct PublishedStoreFallbackParts<D>
where
    D: Copy + Ord,
{
    pub fallback: PublishedStoreFallbackOutput,
    pub ledger_evidence: StoreLedgerEvidence<D>,
}

/// Canonical owning half with a typed, non-owning root reference.
pub struct PublishedCanonicalStore<Kind, Tag> {
    raw: PublishedCanonicalOutput<StoredBlock>,
    root: CanonicalStoreRef<Kind, Tag>,
}

impl<Kind, Tag> PublishedCanonicalStore<Kind, Tag> {
    /// Returns the typed root reference without transferring ownership.
    pub const fn root_ref(&self) -> CanonicalStoreRef<Kind, Tag> {
        self.root
    }

    /// Returns the number of retained canonical blocks.
    pub fn len(&self) -> usize {
        self.raw.aggregate().len()
    }

    /// Returns whether the canonical table is empty.
    pub fn is_empty(&self) -> bool {
        self.raw.aggregate().is_empty()
    }

    /// Returns the physical account that structurally owns this canonical output.
    pub fn account(&self) -> &PhysicalStorageAccount {
        self.raw.account()
    }

    /// Resolves one typed scalar sequence.
    pub fn sequence<T: StoredScalar, SlotTag>(
        &self,
        slot: CanonicalSequenceRef<T, SlotTag>,
    ) -> Option<SequenceView<'_, T>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        sequence_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one binary block.
    pub fn bytes<SlotTag>(&self, slot: CanonicalBytesRef<SlotTag>) -> Option<&[u8]>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        bytes_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one UTF-8 block.
    pub fn utf8<SlotTag>(&self, slot: CanonicalUtf8Ref<SlotTag>) -> Option<&str>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        utf8_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one canonical record block.
    pub fn records<SlotTag>(
        &self,
        slot: CanonicalRecordRef<SlotTag>,
    ) -> Option<CanonicalRecordView<'_>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        canonical_record_view(self.raw.aggregate().get(slot.raw)?)
    }
}

impl<T: StoredScalar, Tag> PublishedCanonicalStore<ScalarSequence<T>, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the typed scalar-sequence root.
    pub fn root(&self) -> SequenceView<'_, T> {
        self.sequence(self.root)
            .unwrap_or_else(|| published_root_invariant("canonical scalar sequence"))
    }
}

impl<Tag> PublishedCanonicalStore<ByteBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the binary root.
    pub fn root(&self) -> &[u8] {
        self.bytes(self.root)
            .unwrap_or_else(|| published_root_invariant("canonical bytes"))
    }
}

impl<Tag> PublishedCanonicalStore<Utf8Block, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the UTF-8 root.
    pub fn root(&self) -> &str {
        self.utf8(self.root)
            .unwrap_or_else(|| published_root_invariant("canonical UTF-8"))
    }
}

impl<Tag> PublishedCanonicalStore<RecordBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the record root.
    pub fn root(&self) -> CanonicalRecordView<'_> {
        self.records(self.root)
            .unwrap_or_else(|| published_root_invariant("canonical records"))
    }
}

/// Outcome-evidence owning half with a typed, non-owning root reference.
pub struct PublishedOutcomeEvidenceStore<Kind, Tag> {
    raw: PublishedOutcomeEvidenceOutput<StoredBlock>,
    root: OutcomeEvidenceStoreRef<Kind, Tag>,
}

impl<Kind, Tag> PublishedOutcomeEvidenceStore<Kind, Tag> {
    /// Returns the typed root reference without transferring ownership.
    pub const fn root_ref(&self) -> OutcomeEvidenceStoreRef<Kind, Tag> {
        self.root
    }

    /// Returns the number of retained outcome-evidence blocks.
    pub fn len(&self) -> usize {
        self.raw.aggregate().len()
    }

    /// Returns whether the evidence table is empty.
    pub fn is_empty(&self) -> bool {
        self.raw.aggregate().is_empty()
    }

    /// Resolves one typed scalar sequence.
    pub fn sequence<T: StoredScalar, SlotTag>(
        &self,
        slot: OutcomeEvidenceSequenceRef<T, SlotTag>,
    ) -> Option<SequenceView<'_, T>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        sequence_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one binary block.
    pub fn bytes<SlotTag>(&self, slot: OutcomeEvidenceBytesRef<SlotTag>) -> Option<&[u8]>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        bytes_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one UTF-8 block.
    pub fn utf8<SlotTag>(&self, slot: OutcomeEvidenceUtf8Ref<SlotTag>) -> Option<&str>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        utf8_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one outcome-evidence record block.
    pub fn records<SlotTag>(
        &self,
        slot: OutcomeEvidenceRecordRef<SlotTag>,
    ) -> Option<OutcomeEvidenceRecordView<'_>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        outcome_evidence_record_view(self.raw.aggregate().get(slot.raw)?)
    }
}

impl<T: StoredScalar, Tag> PublishedOutcomeEvidenceStore<ScalarSequence<T>, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the typed scalar-sequence root.
    pub fn root(&self) -> SequenceView<'_, T> {
        self.sequence(self.root)
            .unwrap_or_else(|| published_root_invariant("outcome-evidence scalar sequence"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStore<ByteBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the binary root.
    pub fn root(&self) -> &[u8] {
        self.bytes(self.root)
            .unwrap_or_else(|| published_root_invariant("outcome-evidence bytes"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStore<Utf8Block, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the UTF-8 root.
    pub fn root(&self) -> &str {
        self.utf8(self.root)
            .unwrap_or_else(|| published_root_invariant("outcome-evidence UTF-8"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStore<RecordBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the record root.
    pub fn root(&self) -> OutcomeEvidenceRecordView<'_> {
        self.records(self.root)
            .unwrap_or_else(|| published_root_invariant("outcome-evidence records"))
    }
}

/// Rich-failure evidence owner with frozen cause and physical account lineage.
pub struct PublishedRichFailureStore<Kind, Tag> {
    raw: PublishedRichFailureOutput<StoredBlock>,
    root: OutcomeEvidenceStoreRef<Kind, Tag>,
}

impl<Kind, Tag> PublishedRichFailureStore<Kind, Tag> {
    /// Returns the typed root reference.
    pub const fn root_ref(&self) -> OutcomeEvidenceStoreRef<Kind, Tag> {
        self.root
    }

    /// Returns the physical account retained by failure evidence.
    pub fn account(&self) -> &PhysicalStorageAccount {
        self.raw.account()
    }

    /// Returns the frozen terminal cause.
    pub fn cause(&self) -> StorePreparationFailure {
        StorePreparationFailure::from_raw(self.raw.cause())
    }

    /// Borrows the evidence table through a typed wrapper.
    pub fn outcome_evidence(&self) -> PublishedOutcomeEvidenceStoreRef<'_, Kind, Tag> {
        PublishedOutcomeEvidenceStoreRef {
            raw: self.raw.outcome_evidence(),
            root: self.root,
        }
    }
}

/// Borrowed typed view of a rich-failure evidence owner.
pub struct PublishedOutcomeEvidenceStoreRef<'a, Kind, Tag> {
    raw: &'a PublishedOutcomeEvidenceOutput<StoredBlock>,
    root: OutcomeEvidenceStoreRef<Kind, Tag>,
}

impl<Kind, Tag> PublishedOutcomeEvidenceStoreRef<'_, Kind, Tag> {
    /// Resolves one typed scalar sequence.
    pub fn sequence<T: StoredScalar, SlotTag>(
        &self,
        slot: OutcomeEvidenceSequenceRef<T, SlotTag>,
    ) -> Option<SequenceView<'_, T>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        sequence_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one binary block.
    pub fn bytes<SlotTag>(&self, slot: OutcomeEvidenceBytesRef<SlotTag>) -> Option<&[u8]>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        bytes_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one UTF-8 block.
    pub fn utf8<SlotTag>(&self, slot: OutcomeEvidenceUtf8Ref<SlotTag>) -> Option<&str>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        utf8_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Resolves one outcome-evidence record block.
    pub fn records<SlotTag>(
        &self,
        slot: OutcomeEvidenceRecordRef<SlotTag>,
    ) -> Option<OutcomeEvidenceRecordView<'_>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        outcome_evidence_record_view(self.raw.aggregate().get(slot.raw)?)
    }

    /// Returns the typed root reference.
    pub const fn root_ref(&self) -> OutcomeEvidenceStoreRef<Kind, Tag> {
        self.root
    }
}

impl<T: StoredScalar, Tag> PublishedOutcomeEvidenceStoreRef<'_, ScalarSequence<T>, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the exact rich-failure scalar-sequence root.
    pub fn root(&self) -> SequenceView<'_, T> {
        self.sequence(self.root)
            .unwrap_or_else(|| published_root_invariant("rich-failure scalar sequence"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStoreRef<'_, ByteBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the exact rich-failure binary root.
    pub fn root(&self) -> &[u8] {
        self.bytes(self.root)
            .unwrap_or_else(|| published_root_invariant("rich-failure bytes"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStoreRef<'_, Utf8Block, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the exact rich-failure UTF-8 root.
    pub fn root(&self) -> &str {
        self.utf8(self.root)
            .unwrap_or_else(|| published_root_invariant("rich-failure UTF-8"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStoreRef<'_, RecordBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the exact rich-failure record root.
    pub fn root(&self) -> OutcomeEvidenceRecordView<'_> {
        self.records(self.root)
            .unwrap_or_else(|| published_root_invariant("rich-failure records"))
    }
}

/// Frozen destination-aware ledger evidence.
pub struct StoreLedgerEvidence<D> {
    raw: PublishedLedgerEvidence<D>,
}

impl<D: Copy + Ord> StoreLedgerEvidence<D> {
    /// Returns one frozen dimension snapshot.
    pub fn dimension(&self, dimension: D) -> Option<StoreDimensionSnapshot<D>> {
        self.raw
            .dimension(dimension)
            .map(|raw| StoreDimensionSnapshot {
                dimension: raw.dimension(),
                attempted: raw.attempted(),
                consumed: raw.consumed(),
                peak_live_bytes: raw.peak_live_bytes(),
                committed_canonical_output: raw.committed_canonical_output(),
                published_outcome_evidence: raw.published_outcome_evidence(),
            })
    }
}

/// Copied destination-aware dimension snapshot from a published operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreDimensionSnapshot<D> {
    dimension: D,
    attempted: u64,
    consumed: u64,
    peak_live_bytes: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

impl<D: Copy> StoreDimensionSnapshot<D> {
    /// Returns the owner-defined dimension.
    pub const fn dimension(self) -> D {
        self.dimension
    }

    /// Returns attempted work.
    pub const fn attempted(self) -> u64 {
        self.attempted
    }

    /// Returns consumed work.
    pub const fn consumed(self) -> u64 {
        self.consumed
    }

    /// Returns peak operation-live bytes.
    pub const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    /// Returns committed canonical output.
    pub const fn committed_canonical_output(self) -> u64 {
        self.committed_canonical_output
    }

    /// Returns published outcome evidence.
    pub const fn published_outcome_evidence(self) -> u64 {
        self.published_outcome_evidence
    }
}

/// Published allocation-free minimal fallback owner.
pub struct PublishedStoreFallbackOutput {
    raw: PublishedFallbackOutput<StoreFallbackEnvelope>,
}

impl PublishedStoreFallbackOutput {
    /// Returns the application-defined fallback envelope.
    pub fn value(&self) -> StoreFallbackEnvelope {
        *self.raw.value()
    }

    /// Returns the physical account that owns the fallback.
    pub fn account(&self) -> &PhysicalStorageAccount {
        self.raw.account()
    }

    /// Returns the frozen terminal cause.
    pub fn cause(&self) -> StorePreparationFailure {
        StorePreparationFailure::from_raw(self.raw.cause())
    }
}

/// Public classification of the frozen terminal preparation cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorePreparationFailureClass {
    Requested,
    Control,
    ProductGraph,
    Ledger,
}

/// Allocation-free frozen terminal cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorePreparationFailure {
    class: StorePreparationFailureClass,
    code: u32,
}

impl StorePreparationFailure {
    const fn from_raw(raw: PreparationFailureCode) -> Self {
        let class = match raw.class() {
            PreparationFailureClass::Requested => StorePreparationFailureClass::Requested,
            PreparationFailureClass::Control => StorePreparationFailureClass::Control,
            PreparationFailureClass::ProductGraph => StorePreparationFailureClass::ProductGraph,
            PreparationFailureClass::Ledger => StorePreparationFailureClass::Ledger,
        };
        Self {
            class,
            code: raw.code(),
        }
    }

    /// Returns the frozen cause class.
    pub const fn class(self) -> StorePreparationFailureClass {
        self.class
    }

    /// Returns the class-specific code.
    pub const fn code(self) -> u32 {
        self.code
    }
}

fn published_root_invariant(expected: &str) -> ! {
    panic!("validated published store violated its typed root invariant: expected {expected}")
}

fn unreachable_store_state() -> ! {
    panic!("concrete retained store reached an impossible private typestate")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retained::{
        test_fail_next_membership_assignment, test_panic_final_owner_drop_after,
    };
    use crate::{
        DimensionPolicy, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits,
    };

    struct TestOwner;

    impl OperationOwner for TestOwner {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestPhase {
        Build,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestProgress {
        Build,
    }

    impl ProgressPhase for TestProgress {
        fn order(self) -> u32 {
            0
        }

        fn stable_id(self) -> &'static str {
            "build"
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestUnit {
        Items,
    }

    impl ProgressUnit for TestUnit {
        fn stable_id(self) -> &'static str {
            "items"
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestDimension {
        Bytes,
    }

    #[derive(Clone, Copy)]
    struct TestTag;

    #[derive(Clone, Copy)]
    struct TestRootTag;

    type TestOperation =
        RetainedStoreOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension>;

    fn test_factory() -> PhysicalStorageAccountFactory {
        PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).expect("test factory limits"),
        )
        .expect("test factory")
    }

    fn test_account(factory: &PhysicalStorageAccountFactory) -> PhysicalStorageAccount {
        factory
            .try_create_account(
                PhysicalStorageAccountLimits::new(u64::MAX, 256).expect("test account limits"),
            )
            .expect("test account")
    }

    fn test_writer_plan(
        attributions: usize,
        commitments: usize,
    ) -> RetainedStoreWriterPlan<TestPhase, TestDimension> {
        RetainedStoreWriterPlan::new(
            TestPhase::Build,
            TestDimension::Bytes,
            TestDimension::Bytes,
            RetainedStoreBatchLimits::new(attributions, commitments).expect("test writer limits"),
        )
    }

    fn test_operation(account: &PhysicalStorageAccount, evidence_limit: u64) -> TestOperation {
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(evidence_limit);
        let ledger_limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)])
            .expect("test ledger limits");
        let start = RetainedStoreStartPlan::new(
            test_writer_plan(32, 8),
            RetainedStoreSharePlan::new(TestDimension::Bytes, TestDimension::Bytes),
            StoreFallbackEnvelope::new(91),
            TestDimension::Bytes,
            1,
            TestDimension::Bytes,
            1,
        );
        TestOperation::try_start(OperationControl::default(), ledger_limits, account, start)
            .expect("test operation")
    }

    fn test_publication_plan(
        canonical_blocks: usize,
        evidence_blocks: usize,
    ) -> RetainedStorePublicationPlan<TestPhase, TestDimension> {
        RetainedStorePublicationPlan::success(
            RetainedStoreBlockTablePlan::new(
                test_writer_plan(canonical_blocks + 1, 2),
                canonical_blocks,
                canonical_blocks,
            )
            .expect("test canonical table plan"),
            RetainedStoreBlockTablePlan::new(
                test_writer_plan(evidence_blocks + 1, 2),
                evidence_blocks,
                evidence_blocks,
            )
            .expect("test evidence table plan"),
        )
    }

    fn test_final_progress() -> FinalProgress<TestProgress, TestUnit> {
        FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
    }

    fn test_physical_delta(
        before: crate::PhysicalStorageAccountSnapshot,
        after: crate::PhysicalStorageAccountSnapshot,
    ) -> (u64, u64) {
        (
            after
                .live_retained_bytes()
                .checked_sub(before.live_retained_bytes())
                .expect("test retained bytes grow monotonically"),
            after
                .live_retained_allocations()
                .checked_sub(before.live_retained_allocations())
                .expect("test retained allocations grow monotonically"),
        )
    }

    fn public_method_block<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("missing public method signature `{signature}`"));
        let tail = &source[start..];
        let end = tail
            .find("\n    }\n")
            .unwrap_or_else(|| panic!("unterminated public method `{signature}`"));
        &tail[..end + "\n    }\n".len()]
    }

    #[test]
    fn scalar_vocabulary_is_closed_and_record_cells_are_allocation_free() {
        fn assert_scalar<T: StoredScalar>() {}
        fn assert_copy<T: Copy>() {}

        assert_scalar::<u8>();
        assert_scalar::<i8>();
        assert_scalar::<u16>();
        assert_scalar::<i16>();
        assert_scalar::<u32>();
        assert_scalar::<i32>();
        assert_scalar::<u64>();
        assert_scalar::<i64>();
        assert_scalar::<f32>();
        assert_scalar::<f64>();
        assert_copy::<RecordCell>();
    }

    #[test]
    fn record_capacity_plan_allocator_and_physical_accounting_are_exact() {
        for (fields, desired, envelope, actual) in [(3, 3, 4, 4), (4, 4, 4, 4), (5, 5, 8, 8)] {
            let plan = RecordPlan::new(1, fields).expect("record plan");
            assert_eq!(plan.cells, desired);
            assert_eq!(plan.capacity.desired_capacity(), desired);
            assert_eq!(plan.capacity.envelope_capacity(), envelope);
            let writer = CanonicalRecordWriter::new(plan).expect("record writer");
            assert_eq!(writer.core.storage.cells.capacity(), actual);
            assert_eq!(writer.core.storage.qualified_capacity, actual);
        }

        let factory = test_factory();
        let account = test_account(&factory);
        let mut assembly = test_operation(&account, u64::MAX)
            .try_publication_assembly(test_publication_plan(3, 1))
            .expect("record accounting assembly");
        let mut snapshots = Vec::with_capacity(4);
        snapshots.push(account.snapshot());
        for fields in [3, 4, 5] {
            let _ = assembly
                .try_retain_canonical_records(
                    RecordPlan::new(1, fields).expect("record plan"),
                    TestTag,
                    |writer| {
                        writer.write_row(|row| {
                            for value in 0..fields {
                                row.write_scalar(u8::try_from(value).expect("small test value"))?;
                            }
                            Ok(())
                        })
                    },
                )
                .expect("record block");
            snapshots.push(account.snapshot());
        }
        let records_three = test_physical_delta(snapshots[0], snapshots[1]);
        let records_four = test_physical_delta(snapshots[1], snapshots[2]);
        let records_five = test_physical_delta(snapshots[2], snapshots[3]);
        assert_eq!(records_three, records_four);
        assert_eq!(
            records_five.0 - records_four.0,
            u64::try_from(4 * std::mem::size_of::<RecordCell>())
                .expect("record-cell delta fits u64")
        );
        assert_eq!(records_three.1, 2);
        assert_eq!(records_four.1, 2);
        assert_eq!(records_five.1, 2);

        drop(
            assembly
                .prepare_minimal_fallback()
                .expect("record assembly fallback")
                .handoff(),
        );
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn membership_rollback_storage_failure_poisoned_the_store_and_quarantines_accounting() {
        let factory = test_factory();
        let account = test_account(&factory);
        let mut assembly = test_operation(&account, u64::MAX)
            .try_publication_assembly(test_publication_plan(1, 1))
            .expect("membership rollback assembly");
        let ledger = assembly.raw.test_ledger_probe();
        let before_active = ledger.active_reservations();
        let before_live = ledger.current_live_bytes();
        let before_physical = account.snapshot();
        let before_unverified = account
            .test_unverified_drop_count()
            .expect("unverified-drop counter");

        test_fail_next_membership_assignment();
        test_panic_final_owner_drop_after(0);
        let error = assembly
            .try_retain_canonical_sequence(
                SequencePlan::<u8>::new(1).expect("sequence plan"),
                TestTag,
                |writer| writer.push(7),
            )
            .expect_err("membership rollback storage release must be unverified");
        assert!(matches!(error, StoreRuntimeError::StorageDropUnverified));
        assert_eq!(
            account.test_unverified_drop_count(),
            Some(before_unverified + 1)
        );
        assert_eq!(ledger.active_reservations(), before_active + 1);
        assert!(ledger.current_live_bytes() > before_live);
        let after_physical = account.snapshot();
        assert!(after_physical.live_retained_bytes() > before_physical.live_retained_bytes());
        assert!(
            after_physical.live_retained_allocations()
                > before_physical.live_retained_allocations()
        );
        assert!(matches!(
            assembly.try_retain_outcome_evidence_sequence(
                SequencePlan::<u8>::new(1).expect("sequence plan"),
                TestTag,
                |writer| writer.push(8),
            ),
            Err(StoreRuntimeError::FallbackOnly)
        ));
        assert!(matches!(
            assembly.prepare_minimal_fallback(),
            Err(StorePrepareError::Poisoned)
        ));

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn primary_rejection_dependency_unwind_maps_to_non_retryable_storage_failure() {
        let factory = test_factory();
        let account = test_account(&factory);
        let mut assembly = test_operation(&account, 4)
            .try_publication_assembly(test_publication_plan(2, 1))
            .expect("primary rejection assembly");
        let leaf = assembly
            .try_retain_canonical_sequence(
                SequencePlan::<u8>::new(1).expect("sequence plan"),
                TestTag,
                |writer| writer.push(7),
            )
            .expect("canonical leaf");
        let canonical_root = assembly
            .try_retain_canonical_records(
                RecordPlan::new(1, 1).expect("record plan"),
                TestRootTag,
                |writer| writer.write_row(|row| row.write_sequence_ref(leaf)),
            )
            .expect("canonical root");
        let evidence_root = assembly
            .try_retain_outcome_evidence_sequence(
                SequencePlan::<u8>::new(1).expect("sequence plan"),
                TestTag,
                |writer| writer.push(9),
            )
            .expect("evidence root");
        assembly
            .try_commit_outcome_evidence(TestDimension::Bytes, 4)
            .expect("primary commitment");
        let ledger = assembly.raw.test_ledger_probe();
        let before_active = ledger.active_reservations();
        let before_live = ledger.current_live_bytes();
        let before_physical = account.snapshot();
        let before_unverified = account
            .test_unverified_drop_count()
            .expect("unverified-drop counter");
        let validated = match assembly.validate_success(
            canonical_root,
            evidence_root,
            |session, canonical_root, evidence_root| {
                let Some(root) = session.canonical_records(canonical_root) else {
                    return false;
                };
                let Some(leaf) = root.sequence_ref::<u8, TestTag>(0, 0) else {
                    return false;
                };
                session.canonical_sequence(leaf).is_some()
                    && session.outcome_evidence_sequence(evidence_root).is_some()
            },
        ) {
            StoreSuccessValidation::Valid(validated) => validated,
            _ => panic!("primary rejection graph must validate"),
        };

        test_panic_final_owner_drop_after(1);
        let result = validated.try_prepare(test_final_progress());
        assert!(matches!(
            result,
            Err(StorePrepareError::StorageDropUnverified)
        ));
        assert!(
            account
                .test_unverified_drop_count()
                .is_some_and(|count| count > before_unverified)
        );
        assert_eq!(ledger.active_reservations(), before_active);
        assert_eq!(ledger.current_live_bytes(), before_live);
        assert_eq!(account.snapshot(), before_physical);

        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }

    #[test]
    fn public_store_surface_does_not_export_raw_or_generic_owning_authority() {
        let root = include_str!("lib.rs");
        let source = include_str!("concrete_store.rs");
        for forbidden in [
            "RetainedAllocation",
            "RetainedAllocationPlan",
            "BlockEmplacer",
            "StoredBlock,",
        ] {
            assert!(
                !root.contains(forbidden),
                "raw retained authority `{forbidden}` escaped the crate root"
            );
        }
        assert!(source.contains("pub trait StoredScalar: sealed::StoredScalar {}"));
        assert!(!source.contains("pub trait StoredScalar:\n"));
        assert!(!root.contains("CanonicalBlockRef"));
        assert!(!root.contains("OutcomeEvidenceBlockRef"));
    }

    #[test]
    fn canonical_share_api_requires_the_sealed_shareable_kind_at_its_boundary() {
        let source = include_str!("concrete_store.rs");
        let method = public_method_block(
            source,
            "pub fn try_share_canonical<SourceRootKind, SourceRootTag, SlotKind, SlotTag>",
        );
        assert!(method.contains("source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>"));
        assert!(method.contains("source_ref: CanonicalStoreRef<SlotKind, SlotTag>"));
        assert!(method.contains("SlotKind: ShareableStoreKind"));
    }

    #[test]
    fn success_evidence_share_api_requires_the_sealed_shareable_kind_at_its_boundary() {
        let source = include_str!("concrete_store.rs");
        let method = public_method_block(
            source,
            "pub fn try_share_outcome_evidence<SourceRootKind, SourceRootTag, SlotKind, SlotTag>",
        );
        assert!(
            method
                .contains("source: &PublishedOutcomeEvidenceStore<SourceRootKind, SourceRootTag>")
        );
        assert!(method.contains("source_ref: OutcomeEvidenceStoreRef<SlotKind, SlotTag>"));
        assert!(method.contains("SlotKind: ShareableStoreKind"));
    }

    #[test]
    fn failure_evidence_share_api_requires_the_sealed_shareable_kind_at_its_boundary() {
        let source = include_str!("concrete_store.rs");
        let method = public_method_block(
            source,
            "pub fn try_share<SourceRootKind, SourceRootTag, SlotKind, SlotTag>",
        );
        assert!(
            method
                .contains("source: &PublishedOutcomeEvidenceStore<SourceRootKind, SourceRootTag>")
        );
        assert!(method.contains("source_ref: OutcomeEvidenceStoreRef<SlotKind, SlotTag>"));
        assert!(method.contains("SlotKind: ShareableStoreKind"));
    }
}
