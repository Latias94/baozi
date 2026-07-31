use super::{
    ByteBlock, BytesPlan, BytesWriter, CanonicalStoreRef, RetainedStorePublicationAssembly,
    ScalarSequence, SequencePlan, SequenceWriter, StoreBuildError, StoreDimensionKey,
    StorePlanError, StoreRuntimeError, StoredBlock, StoredScalar, Utf8Block, Utf8Plan, Utf8Writer,
    finish_scoped_writer,
};
use crate::retained::{
    BlockEmplacer, BlockPlacement, CanonicalRetainBatchError, CanonicalRetainBatchSource,
    CanonicalRetainedRange, RetainedAllocationPlan, RetainedBuildFailureCode,
};
use crate::{OperationOwner, ProgressPhase, ProgressUnit};
use std::fmt;
use std::marker::PhantomData;

/// Stage-qualified failure from a canonical retained-store batch.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreBatchRuntimeError {
    /// The complete batch was rejected before physical, ledger, or table mutation.
    Preflight(StoreRuntimeError),
    /// Execution failed after the completed prefix was released storage-first.
    ///
    /// Non-refundable work remains recorded, but retained storage and live attribution return to
    /// their exact pre-batch shape and the assembly remains writable.
    Execution {
        failed_item: usize,
        cause: StoreRuntimeError,
    },
    /// Execution or prefix cleanup entered fail-closed containment.
    ///
    /// No range was created and the assembly cannot publish a clean fallback.
    Abandoned {
        failed_item: usize,
        cause: StoreRuntimeError,
    },
}

impl fmt::Display for StoreBatchRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preflight(cause) => write!(formatter, "retained batch preflight failed: {cause}"),
            Self::Execution { failed_item, cause } => write!(
                formatter,
                "retained batch execution failed at item {failed_item}: {cause}"
            ),
            Self::Abandoned { failed_item, cause } => write!(
                formatter,
                "retained batch was abandoned at item {failed_item}: {cause}"
            ),
        }
    }
}

impl std::error::Error for StoreBatchRuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Preflight(cause)
            | Self::Execution { cause, .. }
            | Self::Abandoned { cause, .. } => Some(cause),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum TestBatchBuildFault {
    Fail,
    Panic,
}

#[cfg(test)]
thread_local! {
    static TEST_BATCH_BUILD_FAULT: std::cell::Cell<Option<(usize, TestBatchBuildFault)>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn test_fail_batch_build_at(index: usize) {
    TEST_BATCH_BUILD_FAULT.with(|fault| fault.set(Some((index, TestBatchBuildFault::Fail))));
}

#[cfg(test)]
fn test_panic_batch_build_at(index: usize) {
    TEST_BATCH_BUILD_FAULT.with(|fault| fault.set(Some((index, TestBatchBuildFault::Panic))));
}

#[cfg(test)]
fn inject_test_batch_build_fault(index: usize) -> Result<(), StoreBuildError> {
    TEST_BATCH_BUILD_FAULT.with(|fault| match fault.get() {
        Some((target, kind)) if target == index => {
            fault.set(None);
            match kind {
                TestBatchBuildFault::Fail => Err(StoreBuildError::incomplete_write()),
                TestBatchBuildFault::Panic => panic!("injected retained batch builder panic"),
            }
        }
        _ => Ok(()),
    })
}

/// One borrowed, non-empty scalar sequence in a canonical retain batch.
#[derive(Debug, Clone, Copy)]
pub struct SequenceBatchItem<'a, T: StoredScalar> {
    values: &'a [T],
    plan: SequencePlan<T>,
    allocation: RetainedAllocationPlan,
}

impl<'a, T: StoredScalar> SequenceBatchItem<'a, T> {
    /// Qualifies one borrowed sequence without allocating or retaining storage.
    pub fn new(values: &'a [T]) -> Result<Self, StorePlanError> {
        let plan = SequencePlan::new(values.len())?;
        Ok(Self {
            values,
            allocation: plan.allocation_plan()?,
            plan,
        })
    }
}

/// Allocation-free plan for a non-empty batch of borrowed scalar sequences.
#[derive(Debug, Clone, Copy)]
pub struct SequenceBatchPlan<'a, T: StoredScalar> {
    items: &'a [SequenceBatchItem<'a, T>],
}

impl<'a, T: StoredScalar> SequenceBatchPlan<'a, T> {
    /// Preflights every item plan and all aggregate size arithmetic.
    pub fn new(items: &'a [SequenceBatchItem<'a, T>]) -> Result<Self, StorePlanError> {
        let mut total_elements = 0_usize;
        let mut total_nested_bytes = 0_u64;
        let mut total_nested_allocations = 0_u64;
        if items.is_empty() {
            return Err(StorePlanError::EmptyPayload);
        }
        for item in items {
            if item.values.is_empty() {
                return Err(StorePlanError::EmptyPayload);
            }
            let item_plan = SequencePlan::<T>::new(item.values.len())?;
            let allocation = item_plan.allocation_plan()?;
            if item_plan != item.plan || allocation != item.allocation {
                return Err(StorePlanError::SizeOverflow);
            }
            let nested_bytes = item_plan.capacity.envelope_delta_bytes()?;
            total_elements = total_elements
                .checked_add(item_plan.len)
                .ok_or(StorePlanError::SizeOverflow)?;
            total_nested_bytes = total_nested_bytes
                .checked_add(nested_bytes)
                .ok_or(StorePlanError::SizeOverflow)?;
            total_nested_allocations = total_nested_allocations
                .checked_add(u64::from(nested_bytes != 0))
                .ok_or(StorePlanError::SizeOverflow)?;
        }
        if total_elements == 0 || total_nested_bytes == 0 || total_nested_allocations == 0 {
            return Err(StorePlanError::SizeOverflow);
        }
        Ok(Self { items })
    }

    /// Returns the number of blocks in the batch.
    pub const fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the plan contains no blocks.
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// One borrowed, non-empty byte slice in a canonical retain batch.
#[derive(Debug, Clone, Copy)]
pub struct BytesBatchItem<'a> {
    values: &'a [u8],
    plan: BytesPlan,
    allocation: RetainedAllocationPlan,
}

impl<'a> BytesBatchItem<'a> {
    /// Qualifies one borrowed byte slice without allocating or retaining storage.
    pub fn new(values: &'a [u8]) -> Result<Self, StorePlanError> {
        let plan = BytesPlan::new(values.len())?;
        Ok(Self {
            values,
            allocation: plan.allocation_plan()?,
            plan,
        })
    }
}

/// Allocation-free plan for a non-empty batch of borrowed byte slices.
#[derive(Debug, Clone, Copy)]
pub struct BytesBatchPlan<'a> {
    items: &'a [BytesBatchItem<'a>],
}

impl<'a> BytesBatchPlan<'a> {
    /// Preflights every item plan and all aggregate size arithmetic.
    pub fn new(items: &'a [BytesBatchItem<'a>]) -> Result<Self, StorePlanError> {
        let mut total_bytes = 0_usize;
        let mut total_nested_bytes = 0_u64;
        let mut total_nested_allocations = 0_u64;
        if items.is_empty() {
            return Err(StorePlanError::EmptyPayload);
        }
        for item in items {
            if item.values.is_empty() {
                return Err(StorePlanError::EmptyPayload);
            }
            let item_plan = BytesPlan::new(item.values.len())?;
            let allocation = item_plan.allocation_plan()?;
            if item_plan != item.plan || allocation != item.allocation {
                return Err(StorePlanError::SizeOverflow);
            }
            let nested_bytes = item_plan.capacity.envelope_delta_bytes()?;
            total_bytes = total_bytes
                .checked_add(item_plan.len)
                .ok_or(StorePlanError::SizeOverflow)?;
            total_nested_bytes = total_nested_bytes
                .checked_add(nested_bytes)
                .ok_or(StorePlanError::SizeOverflow)?;
            total_nested_allocations = total_nested_allocations
                .checked_add(u64::from(nested_bytes != 0))
                .ok_or(StorePlanError::SizeOverflow)?;
        }
        if total_bytes == 0 || total_nested_bytes == 0 || total_nested_allocations == 0 {
            return Err(StorePlanError::SizeOverflow);
        }
        Ok(Self { items })
    }

    /// Returns the number of blocks in the batch.
    pub const fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the plan contains no blocks.
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// One borrowed, non-empty UTF-8 string in a canonical retain batch.
#[derive(Debug, Clone, Copy)]
pub struct Utf8BatchItem<'a> {
    value: &'a str,
    plan: Utf8Plan,
    allocation: RetainedAllocationPlan,
}

impl<'a> Utf8BatchItem<'a> {
    /// Qualifies one borrowed string without allocating or retaining storage.
    pub fn new(value: &'a str) -> Result<Self, StorePlanError> {
        let plan = Utf8Plan::new(value.len())?;
        Ok(Self {
            value,
            allocation: plan.allocation_plan()?,
            plan,
        })
    }
}

/// Allocation-free plan for a non-empty batch of borrowed UTF-8 strings.
#[derive(Debug, Clone, Copy)]
pub struct Utf8BatchPlan<'a> {
    items: &'a [Utf8BatchItem<'a>],
}

impl<'a> Utf8BatchPlan<'a> {
    /// Preflights every item plan and all aggregate size arithmetic.
    pub fn new(items: &'a [Utf8BatchItem<'a>]) -> Result<Self, StorePlanError> {
        let mut total_bytes = 0_usize;
        let mut total_nested_bytes = 0_u64;
        let mut total_nested_allocations = 0_u64;
        if items.is_empty() {
            return Err(StorePlanError::EmptyPayload);
        }
        for item in items {
            if item.value.is_empty() {
                return Err(StorePlanError::EmptyPayload);
            }
            let item_plan = Utf8Plan::new(item.value.len())?;
            let allocation = item_plan.allocation_plan()?;
            if item_plan != item.plan || allocation != item.allocation {
                return Err(StorePlanError::SizeOverflow);
            }
            let nested_bytes = item_plan.capacity.envelope_delta_bytes()?;
            total_bytes = total_bytes
                .checked_add(item_plan.bytes)
                .ok_or(StorePlanError::SizeOverflow)?;
            total_nested_bytes = total_nested_bytes
                .checked_add(nested_bytes)
                .ok_or(StorePlanError::SizeOverflow)?;
            total_nested_allocations = total_nested_allocations
                .checked_add(u64::from(nested_bytes != 0))
                .ok_or(StorePlanError::SizeOverflow)?;
        }
        if total_bytes == 0 || total_nested_bytes == 0 || total_nested_allocations == 0 {
            return Err(StorePlanError::SizeOverflow);
        }
        Ok(Self { items })
    }

    /// Returns the number of blocks in the batch.
    pub const fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the plan contains no blocks.
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T, Tag> CanonicalRetainBatchSource<StoredBlock, Tag> for SequenceBatchPlan<'_, T>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    fn len(&self) -> usize {
        self.items.len()
    }

    fn allocation_plan(&self, index: usize) -> Option<RetainedAllocationPlan> {
        self.items.get(index).map(|item| item.allocation)
    }

    fn build<'a>(
        &self,
        index: usize,
        emplacer: BlockEmplacer<'a, StoredBlock>,
        tag: Tag,
    ) -> Result<BlockPlacement<'a, StoredBlock, Tag>, RetainedBuildFailureCode> {
        #[cfg(test)]
        inject_test_batch_build_fault(index)?;
        let item = self
            .items
            .get(index)
            .ok_or_else(StoreBuildError::capacity_envelope_violated)?;
        let writer =
            <T as super::sealed::StoredScalar>::new_writer(item.plan.capacity, item.plan.len)?;
        let block = finish_scoped_writer(
            writer,
            |writer| writer.extend_from_slice(item.values),
            SequenceWriter::finish,
        )?;
        Ok(emplacer.place(block, tag))
    }
}

impl<Tag> CanonicalRetainBatchSource<StoredBlock, Tag> for BytesBatchPlan<'_>
where
    Tag: Copy + Send + Sync + 'static,
{
    fn len(&self) -> usize {
        self.items.len()
    }

    fn allocation_plan(&self, index: usize) -> Option<RetainedAllocationPlan> {
        self.items.get(index).map(|item| item.allocation)
    }

    fn build<'a>(
        &self,
        index: usize,
        emplacer: BlockEmplacer<'a, StoredBlock>,
        tag: Tag,
    ) -> Result<BlockPlacement<'a, StoredBlock, Tag>, RetainedBuildFailureCode> {
        #[cfg(test)]
        inject_test_batch_build_fault(index)?;
        let item = self
            .items
            .get(index)
            .ok_or_else(StoreBuildError::capacity_envelope_violated)?;
        let writer = BytesWriter::new(item.plan)?;
        let block = finish_scoped_writer(
            writer,
            |writer| writer.write_bytes(item.values),
            BytesWriter::finish,
        )?;
        Ok(emplacer.place(block, tag))
    }
}

impl<Tag> CanonicalRetainBatchSource<StoredBlock, Tag> for Utf8BatchPlan<'_>
where
    Tag: Copy + Send + Sync + 'static,
{
    fn len(&self) -> usize {
        self.items.len()
    }

    fn allocation_plan(&self, index: usize) -> Option<RetainedAllocationPlan> {
        self.items.get(index).map(|item| item.allocation)
    }

    fn build<'a>(
        &self,
        index: usize,
        emplacer: BlockEmplacer<'a, StoredBlock>,
        tag: Tag,
    ) -> Result<BlockPlacement<'a, StoredBlock, Tag>, RetainedBuildFailureCode> {
        #[cfg(test)]
        inject_test_batch_build_fault(index)?;
        let item = self
            .items
            .get(index)
            .ok_or_else(StoreBuildError::capacity_envelope_violated)?;
        let writer = Utf8Writer::new(item.plan)?;
        let block = finish_scoped_writer(
            writer,
            |writer| writer.write_str(item.value),
            Utf8Writer::finish,
        )?;
        Ok(emplacer.place(block, tag))
    }
}

/// Non-owning, immutable range of homogeneous canonical store references.
#[must_use = "a canonical store range identifies a completed batch in one publication set"]
pub struct CanonicalStoreRange<Kind, Tag> {
    raw: CanonicalRetainedRange<StoredBlock, Tag>,
    marker: PhantomData<fn(Kind) -> Kind>,
}

impl<Kind, Tag: 'static> CanonicalStoreRange<Kind, Tag> {
    pub(super) const fn from_raw(raw: CanonicalRetainedRange<StoredBlock, Tag>) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    /// Returns the number of references in the range.
    pub const fn len(&self) -> usize {
        self.raw.len()
    }

    /// Returns whether the range contains no references.
    pub const fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Returns one non-owning reference, or `None` without probing storage when out of bounds.
    pub fn get(&self, index: usize) -> Option<CanonicalStoreRef<Kind, Tag>> {
        self.raw.get(index).map(|raw| CanonicalStoreRef {
            raw,
            marker: PhantomData,
        })
    }
}

impl<Kind, Tag> Copy for CanonicalStoreRange<Kind, Tag> {}

impl<Kind, Tag> Clone for CanonicalStoreRange<Kind, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Kind, Tag> PartialEq for CanonicalStoreRange<Kind, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<Kind, Tag> Eq for CanonicalStoreRange<Kind, Tag> {}

impl<Kind, Tag> fmt::Debug for CanonicalStoreRange<Kind, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalStoreRange(..)")
    }
}

/// Canonical range of typed scalar sequences.
pub type CanonicalSequenceRange<T, Tag> = CanonicalStoreRange<ScalarSequence<T>, Tag>;
/// Canonical range of binary blocks.
pub type CanonicalBytesRange<Tag> = CanonicalStoreRange<ByteBlock, Tag>;
/// Canonical range of UTF-8 blocks.
pub type CanonicalUtf8Range<Tag> = CanonicalStoreRange<Utf8Block, Tag>;

impl<O, PP, U, L> RetainedStorePublicationAssembly<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn map_batch_result<Kind, Tag>(
        &mut self,
        result: Result<
            CanonicalRetainedRange<StoredBlock, Tag>,
            CanonicalRetainBatchError<StoreDimensionKey>,
        >,
    ) -> Result<CanonicalStoreRange<Kind, Tag>, StoreBatchRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        match result {
            Ok(raw) => Ok(CanonicalStoreRange::from_raw(raw)),
            Err(CanonicalRetainBatchError::Preflight(cause)) => self
                .map_action(Err(cause))
                .map_err(StoreBatchRuntimeError::Preflight),
            Err(CanonicalRetainBatchError::Execution { failed_item, cause }) => self
                .map_action(Err(cause))
                .map_err(|cause| StoreBatchRuntimeError::Execution { failed_item, cause }),
            Err(CanonicalRetainBatchError::Abandoned { failed_item, cause }) => self
                .map_action(Err(cause))
                .map_err(|cause| StoreBatchRuntimeError::Abandoned { failed_item, cause }),
        }
    }

    /// Retains a preflighted batch of borrowed scalar sequences.
    pub fn try_retain_canonical_sequence_batch<T, Tag>(
        &mut self,
        plan: SequenceBatchPlan<'_, T>,
        tag: Tag,
    ) -> Result<CanonicalSequenceRange<T, Tag>, StoreBatchRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()
            .map_err(StoreBatchRuntimeError::Preflight)?;
        let result = self.raw.try_retain_canonical_batch(&plan, tag);
        self.map_batch_result(result)
    }

    /// Retains a preflighted batch of borrowed byte slices.
    pub fn try_retain_canonical_bytes_batch<Tag>(
        &mut self,
        plan: BytesBatchPlan<'_>,
        tag: Tag,
    ) -> Result<CanonicalBytesRange<Tag>, StoreBatchRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()
            .map_err(StoreBatchRuntimeError::Preflight)?;
        let result = self.raw.try_retain_canonical_batch(&plan, tag);
        self.map_batch_result(result)
    }

    /// Retains a preflighted batch of borrowed UTF-8 strings.
    pub fn try_retain_canonical_utf8_batch<Tag>(
        &mut self,
        plan: Utf8BatchPlan<'_>,
        tag: Tag,
    ) -> Result<CanonicalUtf8Range<Tag>, StoreBatchRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.ensure_writable()
            .map_err(StoreBatchRuntimeError::Preflight)?;
        let result = self.raw.try_retain_canonical_batch(&plan, tag);
        self.map_batch_result(result)
    }
}

#[cfg(test)]
mod tests;
