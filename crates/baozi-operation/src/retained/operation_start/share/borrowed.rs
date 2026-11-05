//! Internal retained shares borrow the numeric owner only through the physical clone and handle.

use super::*;
use crate::ledger::{ExclusiveWork, InternalWork, WorkFrame, WorkLane, WorkLayout};

pub(in crate::retained) fn admit_internal_share<'op, D: Copy + Ord + fmt::Debug>(
    work: &'op mut InternalWork<'_, D>,
    plan: RetainedHandleSharePlan<D>,
) -> Result<BorrowedRetainedShareWork<'op, D>, RetainedWriterError<D>> {
    let charges = plan.charges();
    let layout = work
        .bind_share(charges.map(|(dimension, _)| dimension))
        .map_err(RetainedWriterError::Ledger)?;
    let mut work = work.reborrow().map_err(RetainedWriterError::Ledger)?;
    let amounts = charges.map(|(_, amount)| amount);
    let frame = layout
        .admit_observed(&mut work.work, amounts, WorkLane::Internal)
        .map_err(RetainedWriterError::Ledger)?;
    Ok(BorrowedRetainedShareWork {
        work,
        layout,
        frame,
        amounts,
    })
}

pub(in crate::retained) struct BorrowedRetainedShareWork<'op, D: Copy> {
    work: ExclusiveWork<'op, D>,
    layout: WorkLayout<2>,
    frame: WorkFrame<2>,
    amounts: [u64; 2],
}

pub(in crate::retained) fn try_admit_borrowed_retained_share_work<O, P, D>(
    ledger: &mut Ledger<O, P, D>,
    plan: RetainedHandleSharePlan<D>,
) -> Result<BorrowedRetainedShareWork<'_, D>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    let charges = plan.charges();
    let (mut work, layout) = ledger
        .bind_internal_work(charges.map(|(dimension, _)| dimension))
        .map_err(RetainedWriterError::Ledger)?;
    let amounts = charges.map(|(_, amount)| amount);
    let frame = layout
        .admit_observed(&mut work.work, amounts, WorkLane::Internal)
        .map_err(RetainedWriterError::Ledger)?;
    Ok(BorrowedRetainedShareWork {
        work,
        layout,
        frame,
        amounts,
    })
}

impl<D: Copy> BorrowedRetainedShareWork<'_, D> {
    pub(in crate::retained) fn work(&self) -> &ExclusiveWork<'_, D> {
        &self.work
    }
    pub(in crate::retained) fn try_share_allocation<T>(
        self,
        allocation: &RetainedAllocation<T>,
    ) -> Result<RetainedAllocation<T>, RetainedShareWorkCompletionFailure<RetainedAllocation<T>, D>>
    {
        share_allocation_after_admitted_work(self, allocation)
    }

    fn complete(&mut self, amounts: [u64; 2]) -> Result<(), crate::ledger::LedgerCause<D>> {
        self.frame
            .complete_batch_with_outcome(&self.layout, &mut self.work.work, amounts)
            .into_result()
    }

    fn resolve(&mut self, abandoned: bool) -> Result<(), crate::ledger::LedgerCause<D>> {
        self.frame
            .resolve(&self.layout, &mut self.work.work, abandoned)
    }
}

impl<D: Copy> AdmittedShareWork<D> for BorrowedRetainedShareWork<'_, D> {
    fn complete_reference_clone(&mut self) -> Result<(), crate::ledger::LedgerCause<D>> {
        self.complete([self.amounts[0], 0])
    }

    fn complete_typed_handle(mut self) -> Result<(), crate::ledger::LedgerCause<D>> {
        if let Err(cause) = self.complete([0, self.amounts[1]]) {
            self.resolve(false)?;
            return Err(cause);
        }
        self.resolve(false)
    }

    fn finish_partial(mut self) -> Result<(), crate::ledger::LedgerCause<D>> {
        self.resolve(false)
    }
}

impl<D: Copy> Drop for BorrowedRetainedShareWork<'_, D> {
    fn drop(&mut self) {
        if self.frame.active {
            let _ = self.resolve(true);
        }
    }
}

#[cfg(test)]
mod tests;
