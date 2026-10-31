#[cfg(test)]
use super::RetainedSlotIdentity;
use super::account::AdmissionCharge;
use super::allocation::RetainedAllocationPlan;
use super::block_table::{
    BlockEmplacer, BlockPlacement, CompletedCanonicalRetainBatch, QualifiedCanonicalRetainBatch,
    RetainedTableAllocation, try_qualify_canonical_retain_batch,
};
use super::{
    CanonicalRetainedSlot, PhysicalStorageAccount, RetainedAllocationError, RetainedBlockTablePlan,
    RetainedBuildFailureCode, RetainedHandleSharePlan, RetainedPublicationError, RetainedSetId,
    RetainedWriterError,
};
use crate::OperationOwner;
use crate::ledger::{AttributionReservationBatch, CanonicalOutput, Ledger};
#[cfg(test)]
use std::any::TypeId;
use std::fmt;
use std::num::NonZeroU64;
use std::num::NonZeroUsize;

/// Crate-private, allocation-free source for one homogeneous canonical retain batch.
///
/// Implementations are trusted adapters over immutable qualified descriptor slices. This trait is
/// not public, so untrusted callers cannot vary item plans between preflight and execution.
pub(crate) trait CanonicalRetainBatchSource<Block, Tag>
where
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    fn len(&self) -> usize;

    fn allocation_plan(&self, index: usize) -> Option<RetainedAllocationPlan>;

    fn build<'a>(
        &self,
        index: usize,
        emplacer: BlockEmplacer<'a, Block>,
        tag: Tag,
    ) -> Result<BlockPlacement<'a, Block, Tag>, RetainedBuildFailureCode>;
}

/// Stage-qualified batch error distinguishing zero-mutation preflight rejection, verified
/// execution rollback, and fail-closed containment.
pub(crate) enum CanonicalRetainBatchError<D> {
    Preflight(RetainedPublicationError<D>),
    Execution {
        failed_item: usize,
        cause: RetainedPublicationError<D>,
    },
    Abandoned {
        failed_item: usize,
        cause: RetainedPublicationError<D>,
    },
}

pub(in crate::retained) struct CanonicalBatchPreflightContext<'a, O, P, D, Block>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(in crate::retained) ledger: &'a Ledger<O, P, D>,
    pub(in crate::retained) account: &'a PhysicalStorageAccount,
    pub(in crate::retained) table: &'a RetainedTableAllocation<Block>,
    pub(in crate::retained) batch: &'a AttributionReservationBatch<O, P, D, CanonicalOutput>,
    pub(in crate::retained) handle_share_work: RetainedHandleSharePlan<D>,
    pub(in crate::retained) table_plan: RetainedBlockTablePlan<P, D>,
    pub(in crate::retained) set_id: RetainedSetId,
    pub(in crate::retained) slot_limit_error: RetainedPublicationError<D>,
}

pub(in crate::retained) fn try_preflight_canonical_batch<O, P, D, Block, Tag, Source>(
    source: &Source,
    context: CanonicalBatchPreflightContext<'_, O, P, D, Block>,
) -> Result<QualifiedCanonicalRetainBatch<Block, Tag>, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
    Source: CanonicalRetainBatchSource<Block, Tag>,
{
    let CanonicalBatchPreflightContext {
        ledger,
        account,
        table,
        batch,
        handle_share_work,
        table_plan,
        set_id,
        slot_limit_error,
    } = context;
    let count = NonZeroUsize::new(source.len()).ok_or(RetainedPublicationError::Writer(
        RetainedWriterError::InvalidPublicationPlan,
    ))?;
    drop(
        account
            .try_begin_account_preflight()
            .map_err(RetainedWriterError::AccountPoisoned)
            .map_err(RetainedPublicationError::Writer)?,
    );

    let mut aggregate = AdmissionCharge::new(0, 0);
    for index in 0..count.get() {
        let allocation_plan =
            source
                .allocation_plan(index)
                .ok_or(RetainedPublicationError::Writer(
                    RetainedWriterError::InvalidPublicationPlan,
                ))?;
        let charge = account
            .checked_retained_charge::<Block>(allocation_plan)
            .map_err(|cause| {
                RetainedPublicationError::Writer(RetainedWriterError::Physical(cause))
            })?;
        aggregate.bytes =
            aggregate
                .bytes
                .checked_add(charge.bytes)
                .ok_or(RetainedPublicationError::Writer(
                    RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow),
                ))?;
        aggregate.allocations = aggregate
            .allocations
            .checked_add(charge.allocations)
            .ok_or(RetainedPublicationError::Writer(
                RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow),
            ))?;
    }
    if aggregate.bytes == 0 || aggregate.allocations == 0 {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::InvalidPublicationPlan,
        ));
    }
    let reservation_count =
        NonZeroU64::new(u64::try_from(count.get()).map_err(|_| {
            RetainedPublicationError::Writer(RetainedWriterError::MembershipExhausted)
        })?)
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::InvalidPublicationPlan,
        ))?;
    let qualified = try_qualify_canonical_retain_batch::<O, P, D, Block, Tag>(
        account,
        table,
        batch,
        table_plan,
        set_id,
        count,
        aggregate.bytes,
        slot_limit_error,
    )?;
    ledger
        .preflight_repeated_observed_batch(&handle_share_work.charges(), reservation_count)
        .map_err(RetainedWriterError::Ledger)
        .map_err(RetainedPublicationError::Writer)?;
    ledger
        .preflight_reservation_batch(
            table_plan.writer.phase,
            table_plan.writer.attribution_dimension,
            aggregate.bytes,
            reservation_count,
        )
        .map_err(RetainedWriterError::Ledger)
        .map_err(RetainedPublicationError::Writer)?;

    let snapshot = account.snapshot();
    let limits = account.limits();
    let requested_bytes = snapshot
        .live_retained_bytes()
        .checked_add(aggregate.bytes)
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow),
        ))?;
    let requested_allocations = snapshot
        .live_retained_allocations()
        .checked_add(aggregate.allocations)
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow),
        ))?;
    if requested_bytes > limits.max_retained_bytes() {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::ByteLimitExceeded {
                requested: requested_bytes,
                limit: limits.max_retained_bytes(),
            }),
        ));
    }
    if requested_allocations > limits.max_retained_allocations() {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::AllocationLimitExceeded {
                requested: requested_allocations,
                limit: limits.max_retained_allocations(),
            }),
        ));
    }
    Ok(qualified)
}

/// Non-owning contiguous canonical slots minted by one completed retained batch.
///
/// Construction stays inside the retained publication rail. The range carries no table,
/// allocation, mutation, retagging, or ownership authority.
pub(crate) struct CanonicalRetainedRange<Block, Tag> {
    first: CanonicalRetainedSlot<Block, Tag>,
    len: NonZeroUsize,
}

impl<Block, Tag: 'static> CanonicalRetainedRange<Block, Tag> {
    pub(in crate::retained) fn from_completed_batch(
        completed: CompletedCanonicalRetainBatch<Block, Tag>,
    ) -> Self {
        let (first, len) = completed.into_parts();
        Self {
            first: CanonicalRetainedSlot::from_identity(first),
            len,
        }
    }

    #[cfg(test)]
    fn from_identity_for_test(first: RetainedSlotIdentity, len: usize) -> Option<Self> {
        if first.tag != TypeId::of::<Tag>() {
            return None;
        }
        let len = NonZeroUsize::new(len)?;
        let last_offset = len.get().checked_sub(1)?;
        first.index.checked_add(last_offset)?;
        Some(Self {
            first: CanonicalRetainedSlot::from_identity(first),
            len,
        })
    }

    pub(crate) const fn len(&self) -> usize {
        self.len.get()
    }

    pub(crate) const fn is_empty(&self) -> bool {
        false
    }

    pub(crate) fn get(&self, index: usize) -> Option<CanonicalRetainedSlot<Block, Tag>> {
        if index >= self.len.get() {
            return None;
        }
        let mut slot = self.first;
        slot.index = slot.index.checked_add(index)?;
        Some(slot)
    }
}

impl<Block, Tag> Copy for CanonicalRetainedRange<Block, Tag> {}

impl<Block, Tag> Clone for CanonicalRetainedRange<Block, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Block, Tag> PartialEq for CanonicalRetainedRange<Block, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.first == other.first && self.len == other.len
    }
}

impl<Block, Tag> Eq for CanonicalRetainedRange<Block, Tag> {}

impl<Block, Tag> fmt::Debug for CanonicalRetainedRange<Block, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalRetainedRange(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retained::allocation::try_next_retained_set_id;

    struct Block;
    struct ExpectedTag;
    struct ForeignTag;

    #[test]
    fn completed_range_rejects_empty_overflowed_and_retagged_identity() {
        let set_id = try_next_retained_set_id().expect("test retained set id");
        let expected = RetainedSlotIdentity {
            set_id,
            index: 4,
            tag: TypeId::of::<ExpectedTag>(),
        };

        assert!(
            CanonicalRetainedRange::<Block, ExpectedTag>::from_identity_for_test(expected, 0)
                .is_none()
        );
        assert!(
            CanonicalRetainedRange::<Block, ExpectedTag>::from_identity_for_test(
                RetainedSlotIdentity {
                    index: usize::MAX,
                    ..expected
                },
                2,
            )
            .is_none()
        );
        assert!(
            CanonicalRetainedRange::<Block, ForeignTag>::from_identity_for_test(expected, 1)
                .is_none()
        );

        let range =
            CanonicalRetainedRange::<Block, ExpectedTag>::from_identity_for_test(expected, 3)
                .expect("qualified test range");
        assert_eq!(range.len(), 3);
        assert_eq!(range.get(0).map(|slot| slot.identity()), Some(expected));
        assert_eq!(
            range.get(2).map(|slot| slot.identity()),
            Some(RetainedSlotIdentity {
                index: 6,
                ..expected
            })
        );
        assert!(range.get(3).is_none());
    }
}
