//! Transitional aliasable parser adapters; all arithmetic and flags live in kernel.
#[cfg(test)]
use super::WorkGrant;
use super::kernel::{ParserChannel, ParserFrame};
use super::{BoundWorkGroup, LedgerCause};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParserWorkError<D> {
    Ledger(LedgerCause<D>),
    ItemLimit {
        index: usize,
        value: u64,
        limit: u64,
    },
}

impl<D> From<LedgerCause<D>> for ParserWorkError<D> {
    fn from(cause: LedgerCause<D>) -> Self {
        Self::Ledger(cause)
    }
}

impl<'op, D: Copy, const N: usize> BoundWorkGroup<'op, D, N> {
    pub(crate) fn begin_parser_parent(
        &self,
        amounts: [u64; N],
        item_limits: [Option<u64>; N],
    ) -> Result<ParserWorkScope<'op, D, N>, ParserWorkError<D>> {
        self.begin_parser(amounts, item_limits, ParserChannel::Parent)
    }
    pub(crate) fn begin_parser_child(
        &self,
        amounts: [u64; N],
        item_limits: [Option<u64>; N],
    ) -> Result<ParserWorkScope<'op, D, N>, ParserWorkError<D>> {
        self.begin_parser(amounts, item_limits, ParserChannel::Child)
    }
    fn begin_parser(
        &self,
        amounts: [u64; N],
        item_limits: [Option<u64>; N],
        channel: ParserChannel,
    ) -> Result<ParserWorkScope<'op, D, N>, ParserWorkError<D>> {
        let frame = self.layout.begin_parser(
            &mut self.borrow_active()?.view(&self.owner.catalog),
            amounts,
            item_limits,
            channel,
        )?;
        Ok(ParserWorkScope {
            group: *self,
            frame,
        })
    }
}

#[must_use = "finish the parser channel explicitly, even after completing every prefix"]
pub(crate) struct ParserWorkScope<'op, D: Copy, const N: usize> {
    group: BoundWorkGroup<'op, D, N>,
    frame: ParserFrame<N>,
}

impl<D: Copy, const N: usize> ParserWorkScope<'_, D, N> {
    pub(crate) fn remaining(&self) -> [u64; N] {
        self.frame.remaining
    }
    pub(crate) fn complete(&mut self, amounts: [u64; N]) -> Result<(), LedgerCause<D>> {
        if !self.frame.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let mut state = self.group.borrow_active()?;
        self.frame.complete(
            &self.group.layout,
            &mut state.view(&self.group.owner.catalog),
            amounts,
        )
    }
    pub(crate) fn complete_all(&mut self) -> Result<(), LedgerCause<D>> {
        self.complete(self.frame.remaining)
    }
    pub(crate) fn finish_exact(mut self) -> Result<(), LedgerCause<D>> {
        self.resolve()?;
        if self.frame.remaining.iter().any(|amount| *amount != 0) {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(())
    }
    pub(crate) fn finish_partial(mut self) -> Result<(), LedgerCause<D>> {
        self.resolve()
    }
    fn resolve(&mut self) -> Result<(), LedgerCause<D>> {
        if !self.frame.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let mut state = self.group.owner.borrow_state_mut()?;
        let validity = self
            .group
            .owner
            .ensure_active(&state, self.group.layout.grant)
            .and_then(|()| {
                self.group
                    .layout
                    .validate_role_header(&self.group.owner.catalog, &state.header)
            });
        let resolution = self
            .frame
            .resolve(&mut state.view(&self.group.owner.catalog));
        validity?;
        resolution
    }
}
impl<D: Copy> ParserWorkScope<'_, D, 1> {
    pub(crate) fn replenish(&mut self, amount: u64) -> Result<(), LedgerCause<D>> {
        if !self.frame.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let mut state = self.group.borrow_active()?;
        self.frame.replenish(
            &self.group.layout,
            &mut state.view(&self.group.owner.catalog),
            amount,
        )
    }
}
impl<D: Copy, const N: usize> Drop for ParserWorkScope<'_, D, N> {
    fn drop(&mut self) {
        if self.frame.active {
            let _ = self.resolve();
        }
    }
}

#[cfg(test)]
mod tests;
