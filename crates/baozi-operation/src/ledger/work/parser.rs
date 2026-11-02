//! Attempted-only parser channels over the invocation's authoritative work rows.

use std::cell::RefMut;

use super::{BoundWorkGroup, LedgerCause, WorkGrant, WorkState, charge_attempted_entry};

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

#[derive(Clone, Copy)]
enum ParserChannel {
    Parent,
    Child,
}

impl ParserChannel {
    fn is_open<D>(self, state: &WorkState<D>) -> bool {
        match self {
            Self::Parent => state.parser_parent_open,
            Self::Child => state.parser_child_open,
        }
    }

    fn set_open<D>(self, state: &mut WorkState<D>, open: bool) {
        match self {
            Self::Parent => state.parser_parent_open = open,
            Self::Child => state.parser_child_open = open,
        }
    }

    fn has_child<D>(self, state: &WorkState<D>) -> bool {
        matches!(self, Self::Parent) && state.parser_child_open
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
        let mut state = self.borrow_active()?;
        if self.grant != WorkGrant::Ordinary
            || state.producer_open
            || state.protected_open
            || channel.is_open(&state)
            || channel.has_child(&state)
        {
            return Err(LedgerCause::InactiveLinearWorkPermit.into());
        }
        for (index, amount) in amounts.into_iter().enumerate() {
            charge_attempted_entry(
                &mut state.rows[self.positions[index]],
                self.dimensions[index],
                amount,
            )?;
            if let Some(limit) = item_limits[index]
                && amount > limit
            {
                return Err(ParserWorkError::ItemLimit {
                    index,
                    value: amount,
                    limit,
                });
            }
        }
        channel.set_open(&mut state, true);
        Ok(ParserWorkScope {
            group: *self,
            remaining: amounts,
            item_limits,
            channel,
            active: true,
        })
    }
}

#[must_use = "finish the parser channel explicitly, even after completing every prefix"]
pub(crate) struct ParserWorkScope<'op, D: Copy, const N: usize> {
    group: BoundWorkGroup<'op, D, N>,
    remaining: [u64; N],
    item_limits: [Option<u64>; N],
    channel: ParserChannel,
    active: bool,
}

impl<'op, D: Copy, const N: usize> ParserWorkScope<'op, D, N> {
    pub(crate) fn remaining(&self) -> [u64; N] {
        self.remaining
    }

    /// Records one exact group prefix without reserving a pending-consumed envelope.
    pub(crate) fn complete(&mut self, amounts: [u64; N]) -> Result<(), LedgerCause<D>> {
        {
            let _state = self.borrow_active()?;
        }
        if amounts
            .iter()
            .zip(self.remaining)
            .any(|(amount, remaining)| *amount > remaining)
        {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        if amounts.iter().all(|amount| *amount == 0) {
            return Ok(());
        }
        // No external action can intervene between channel validation and this shared kernel.
        self.group.consume_direct_atomic(amounts)?;
        for (remaining, amount) in self.remaining.iter_mut().zip(amounts) {
            *remaining -= amount;
        }
        Ok(())
    }

    pub(crate) fn complete_all(&mut self) -> Result<(), LedgerCause<D>> {
        self.complete(self.remaining)
    }

    /// Closes ownership even when incomplete; the domain wrapper must latch that error.
    pub(crate) fn finish_exact(mut self) -> Result<(), LedgerCause<D>> {
        self.resolve()?;
        if self.remaining.iter().any(|amount| *amount != 0) {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(())
    }

    pub(crate) fn finish_partial(mut self) -> Result<(), LedgerCause<D>> {
        self.resolve()
    }

    fn borrow_active(&self) -> Result<RefMut<'op, WorkState<D>>, LedgerCause<D>> {
        if !self.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let mut state = self.group.borrow_active()?;
        if !self.channel.is_open(&state) || state.producer_open || state.protected_open {
            state.integrity_fault = true;
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        if self.channel.has_child(&state) {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(state)
    }

    fn resolve(&mut self) -> Result<(), LedgerCause<D>> {
        if !self.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let mut state = self.group.owner.borrow_state_mut()?;
        // Cleanup remains possible after mode revocation or a prior integrity failure.
        let validity = self
            .group
            .owner
            .ensure_active(&state, self.group.grant)
            .and_then(|()| self.group.validate_role(&state));
        let invalid_channel = !self.channel.is_open(&state)
            || self.channel.has_child(&state)
            || state.producer_open
            || state.protected_open;
        if invalid_channel {
            state.integrity_fault = true;
        }
        self.channel.set_open(&mut state, false);
        self.active = false;
        validity?;
        if invalid_channel {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(())
    }
}

impl<D: Copy> ParserWorkScope<'_, D, 1> {
    /// Admits another chunk only for a fully consumed scope without a per-item limit.
    pub(crate) fn replenish(&mut self, amount: u64) -> Result<(), LedgerCause<D>> {
        {
            let mut state = self.borrow_active()?;
            if self.item_limits[0].is_some() || self.remaining[0] != 0 {
                return Err(LedgerCause::InactiveLinearWorkPermit);
            }
            charge_attempted_entry(
                &mut state.rows[self.group.positions[0]],
                self.group.dimensions[0],
                amount,
            )?;
        }
        self.remaining[0] = amount;
        Ok(())
    }
}

impl<D: Copy, const N: usize> Drop for ParserWorkScope<'_, D, N> {
    fn drop(&mut self) {
        if self.active {
            // Failed borrows latch LocalWork's fault and leave persistent channels untouched.
            let _ = self.resolve();
        }
    }
}

#[cfg(test)]
mod tests;
