//! The sole work arithmetic and ownership-state kernel. Binding compares catalog keys once.
//! The immutable owner catalog is borrowed by WorkView; layouts contain only positions and roles.
//! Subsequent row transactions perform no dynamic borrow, lock, allocation or user action.

use super::parser::ParserWorkError;
use super::{
    CounterClass, LedgerCause, LinearWorkCompletion, WorkDimensionState, WorkEntry, WorkGrant,
    WorkHeader, WorkLane, WorkPolicy,
};
use std::num::NonZeroU64;

pub(crate) struct WorkView<'a, D> {
    pub(in crate::ledger) catalog: &'a [D],
    pub(in crate::ledger) rows: &'a mut [WorkEntry<D>],
    pub(in crate::ledger) header: &'a mut WorkHeader,
}

#[derive(Clone, Copy)]
pub(crate) struct WorkLayout<const N: usize> {
    pub(super) positions: [usize; N],
    pub(super) representatives: [usize; N],
    pub(super) completion_order: [usize; N],
    pub(super) grant: WorkGrant,
}

impl<const N: usize> WorkLayout<N> {
    pub(in crate::ledger) fn bind<D: Copy + Ord>(
        catalog: &[D],
        dimensions: &[D; N],
        grant: WorkGrant,
    ) -> Result<Self, LedgerCause<D>> {
        let mut positions = [0; N];
        for (index, dimension) in dimensions.iter().copied().enumerate() {
            positions[index] = catalog
                .binary_search(&dimension)
                .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        }
        let mut representatives = [0; N];
        for index in 0..N {
            representatives[index] = positions[..index]
                .iter()
                .position(|position| *position == positions[index])
                .unwrap_or(index);
        }
        let mut completion_order: [usize; N] = std::array::from_fn(|index| index);
        completion_order.sort_unstable_by_key(|index| (positions[*index], *index));
        Ok(Self {
            positions,
            representatives,
            completion_order,
            grant,
        })
    }
}

impl<const N: usize> WorkLayout<N> {
    fn dimension<D: Copy>(&self, catalog: &[D], index: usize) -> D {
        catalog[self.positions[index]]
    }
    pub(super) fn aggregate<D: Copy>(
        &self,
        catalog: &[D],
        amounts: [u64; N],
        class: CounterClass,
    ) -> Result<[u64; N], LedgerCause<D>> {
        let mut totals = [0_u64; N];
        for (index, amount) in amounts.into_iter().enumerate() {
            let first = self.representatives[index];
            totals[first] = totals[first].checked_add(amount).ok_or_else(|| {
                LedgerCause::ArithmeticOverflow {
                    class,
                    dimension: self.dimension(catalog, index),
                }
            })?;
        }
        Ok(totals)
    }

    pub(in crate::ledger) fn validate_role_header<D: Copy>(
        &self,
        catalog: &[D],
        header: &WorkHeader,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_role_with(header, |index| self.dimension(catalog, index))
    }

    pub(in crate::ledger) fn validate_binding<D: Copy>(
        &self,
        header: &WorkHeader,
        supplied: &[D; N],
    ) -> Result<(), LedgerCause<D>> {
        self.validate_role_with(header, |index| supplied[index])
    }

    fn validate_role_with<D>(
        &self,
        header: &WorkHeader,
        dimension: impl FnOnce(usize) -> D,
    ) -> Result<(), LedgerCause<D>> {
        match self.grant {
            WorkGrant::Ordinary => {
                if let Some(index) = self
                    .positions
                    .iter()
                    .position(|position| Some(*position) == header.protected_dimension)
                {
                    return Err(LedgerCause::UnknownDimension {
                        dimension: dimension(index),
                    });
                }
            }
            WorkGrant::Protected => {
                if self
                    .positions
                    .iter()
                    .any(|position| Some(*position) != header.protected_dimension)
                {
                    return Err(LedgerCause::InactiveLinearWorkPermit);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn charge_attempted_in_order<D: Copy>(
        &self,
        state: &mut WorkView<'_, D>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        let catalog = state.catalog;
        for (index, amount) in amounts.into_iter().enumerate() {
            charge_attempted_entry(
                &mut state.rows[self.positions[index]],
                || self.dimension(catalog, index),
                amount,
            )?;
        }
        Ok(())
    }

    pub(crate) fn consume_direct_atomic<D: Copy>(
        &self,
        state: &mut WorkView<'_, D>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        let catalog = state.catalog;
        let mut consumed = [0; N];
        let mut cursor = 0;
        while cursor < N {
            let index = self.completion_order[cursor];
            let position = self.positions[index];
            let dimension = || self.dimension(catalog, index);
            let mut total = 0_u64;
            while cursor < N && self.positions[self.completion_order[cursor]] == position {
                total = add_consumed(total, amounts[self.completion_order[cursor]], dimension)?;
                cursor += 1;
            }
            let row = &state.rows[position];
            consumed[index] = add_consumed(row.state.consumed, total, dimension)?;
            check_effective(
                dimension,
                consumed[index],
                row.state.pending_consumed,
                row.state.attempted,
                row.policy.consumed_limit,
            )?;
        }
        for index in self.completion_order {
            if self.representatives[index] == index {
                state.rows[self.positions[index]].state.consumed = consumed[index];
            }
        }
        Ok(())
    }

    /// Retains every physical prefix, including the failing pair's representable evidence.
    pub(crate) fn charge_observed_ordered<D: Copy>(
        &self,
        state: &mut WorkView<'_, D>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        let catalog = state.catalog;
        for (index, amount) in amounts.into_iter().enumerate() {
            let row = &mut state.rows[self.positions[index]];
            let dimension = || self.dimension(catalog, index);
            charge_attempted_entry(row, dimension, amount)?;
            consume_observed_entry(row, dimension, amount)?;
        }
        Ok(())
    }

    /// Records physical completion without charging another attempted envelope.
    pub(crate) fn consume_observed_in_order<D: Copy>(
        &self,
        state: &mut WorkView<'_, D>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        let catalog = state.catalog;
        for (index, amount) in amounts.into_iter().enumerate() {
            consume_observed_entry(
                &mut state.rows[self.positions[index]],
                || self.dimension(catalog, index),
                amount,
            )?;
        }
        Ok(())
    }

    /// Checks a prebound startup group in the owner's existing joint observation interval.
    pub(super) fn preflight_repeated_observed<D: Copy>(
        &self,
        catalog: &[D],
        rows: &[WorkEntry<D>],
        header: &WorkHeader,
        amounts: [u64; N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_role_header(catalog, header)?;
        let mut repeated = amounts;
        for (index, amount) in repeated.iter_mut().enumerate() {
            *amount = amount.checked_mul(repetitions.get()).ok_or_else(|| {
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Attempted,
                    dimension: self.dimension(catalog, index),
                }
            })?;
        }
        for index in 0..N {
            if self.representatives[index] != index {
                continue;
            }
            let row = &rows[self.positions[index]];
            let dimension = || self.dimension(catalog, index);
            // Preserve caller-first projection precedence, including a later group's overflow.
            let mut amount = 0;
            for (other, repeated_amount) in repeated.iter().copied().enumerate() {
                if self.representatives[other] == index {
                    amount = add_attempted(amount, repeated_amount, dimension)?;
                }
            }
            let attempted = add_attempted(row.state.attempted, amount, dimension)?;
            check_limit(
                CounterClass::Attempted,
                dimension,
                attempted,
                row.policy.attempted_limit,
            )?;
            let effective =
                add_consumed(row.state.consumed, row.state.pending_consumed, dimension)?;
            let effective = add_consumed(effective, amount, dimension)?;
            check_effective(
                dimension,
                effective,
                0,
                attempted,
                row.policy.consumed_limit,
            )?;
        }
        Ok(())
    }

    pub(crate) fn admit_observed<D: Copy>(
        &self,
        state: &mut WorkView<'_, D>,
        amounts: [u64; N],
        lane: WorkLane,
    ) -> Result<WorkFrame<N>, LedgerCause<D>> {
        match (self.grant, lane) {
            (WorkGrant::Ordinary, WorkLane::Producer)
                if !state.header.producer_open
                    && !state.header.parser_parent_open
                    && !state.header.parser_child_open => {}
            (WorkGrant::Protected, WorkLane::Protected) if !state.header.protected_open => {}
            (_, WorkLane::Internal) => {}
            _ => return Err(LedgerCause::InactiveLinearWorkPermit),
        }
        let totals = self.aggregate(state.catalog, amounts, CounterClass::Attempted)?;
        let open_envelopes = state
            .header
            .open_envelopes
            .checked_add(1)
            .ok_or(LedgerCause::FinalSnapshotShapeMismatch)?;
        self.admit_observed_totals(state, totals)?;
        state.header.open_envelopes = open_envelopes;
        match lane {
            WorkLane::Producer => state.header.producer_open = true,
            WorkLane::Protected => state.header.protected_open = true,
            WorkLane::Internal => {}
        }
        Ok(WorkFrame {
            remaining: amounts,
            lane,
            active: true,
            faulted: false,
        })
    }

    /// Shares numeric admission between initial envelopes and singleton replenishment.
    pub(super) fn admit_observed_totals<D: Copy>(
        &self,
        state: &mut WorkView<'_, D>,
        totals: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        let catalog = state.catalog;
        let mut attempted = [0; N];
        let mut pending = [0; N];
        let mut attempted_failure = None;
        let mut consumed_failure = None;
        // Project every arithmetic operation before committing even non-refundable attempts.
        for (index, amount) in totals.into_iter().enumerate() {
            if amount == 0 || self.representatives[index] != index {
                continue;
            }
            let row = &state.rows[self.positions[index]];
            let dimension = || self.dimension(catalog, index);
            attempted[index] = add_attempted(row.state.attempted, amount, dimension)?;
            pending[index] = add_consumed(row.state.pending_consumed, amount, dimension)?;
            let effective = add_consumed(row.state.consumed, pending[index], dimension)?;
            if attempted_failure.is_none() {
                attempted_failure = check_limit(
                    CounterClass::Attempted,
                    dimension,
                    attempted[index],
                    row.policy.attempted_limit,
                )
                .err();
            }
            if consumed_failure.is_none() {
                consumed_failure = check_effective(
                    dimension,
                    effective,
                    0,
                    attempted[index],
                    row.policy.consumed_limit,
                )
                .err();
            }
        }
        for (index, amount) in totals.into_iter().enumerate() {
            if amount != 0 && self.representatives[index] == index {
                state.rows[self.positions[index]].state.attempted = attempted[index];
            }
        }
        if let Some(cause) = attempted_failure.or(consumed_failure) {
            return Err(cause);
        }
        for (index, amount) in totals.into_iter().enumerate() {
            if amount != 0 && self.representatives[index] == index {
                state.rows[self.positions[index]].state.pending_consumed = pending[index];
            }
        }
        Ok(())
    }
}

/// Host metadata. This is deliberately private to the crate, never lent as an owning permit.
pub(crate) struct WorkFrame<const N: usize> {
    pub(crate) remaining: [u64; N],
    pub(super) lane: WorkLane,
    pub(crate) active: bool,
    pub(crate) faulted: bool,
}

impl<const N: usize> WorkFrame<N> {
    pub(crate) fn complete_batch_with_outcome<D: Copy>(
        &mut self,
        group: &WorkLayout<N>,
        state: &mut WorkView<'_, D>,
        amounts: [u64; N],
    ) -> LinearWorkCompletion<D> {
        let catalog = state.catalog;
        if !self.active
            || self.faulted
            || amounts
                .iter()
                .zip(self.remaining)
                .any(|(amount, remaining)| *amount > remaining)
        {
            return LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit);
        }
        if !lane_is_open(state, self.lane) {
            state.header.integrity_fault = true;
            self.faulted = true;
            return LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit);
        }
        let totals = match group.aggregate(state.catalog, amounts, CounterClass::Consumed) {
            Ok(totals) => totals,
            Err(cause) => {
                state.header.integrity_fault = true;
                self.faulted = true;
                return LinearWorkCompletion::NotCompleted(cause);
            }
        };
        let mut consumed = [0; N];
        let mut pending = [0; N];
        let mut failure = None;
        let mut prefix_exceeds_attempted = false;
        for index in group.completion_order {
            let amount = totals[index];
            if amount == 0 || group.representatives[index] != index {
                continue;
            }
            let row = &state.rows[group.positions[index]];
            let dimension = || group.dimension(catalog, index);
            let Some(next_pending) = row.state.pending_consumed.checked_sub(amount) else {
                state.header.integrity_fault = true;
                self.faulted = true;
                return LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit);
            };
            let next_consumed = match add_consumed(row.state.consumed, amount, dimension) {
                Ok(value) => value,
                Err(cause) => {
                    state.header.integrity_fault = true;
                    self.faulted = true;
                    return LinearWorkCompletion::NotCompleted(cause);
                }
            };
            consumed[index] = next_consumed;
            pending[index] = next_pending;
            prefix_exceeds_attempted |= next_consumed > row.state.attempted;
            if failure.is_none() {
                failure = check_effective(
                    dimension,
                    next_consumed,
                    next_pending,
                    row.state.attempted,
                    row.policy.consumed_limit,
                )
                .err();
            }
        }
        // No fallible operation or key comparison remains in the physical-prefix commit.
        for index in group.completion_order {
            if totals[index] != 0 && group.representatives[index] == index {
                let row = &mut state.rows[group.positions[index]];
                row.state.consumed = consumed[index];
                row.state.pending_consumed = pending[index];
            }
        }
        for (remaining, amount) in self.remaining.iter_mut().zip(amounts) {
            *remaining -= amount;
        }
        if let Some(cause) = failure {
            state.header.integrity_fault |= prefix_exceeds_attempted;
            self.faulted = true;
            LinearWorkCompletion::CompletedWithLedgerFailure(cause)
        } else {
            LinearWorkCompletion::Completed
        }
    }

    pub(crate) fn resolve<D: Copy>(
        &mut self,
        group: &WorkLayout<N>,
        state: &mut WorkView<'_, D>,
        abandoned: bool,
    ) -> Result<(), LedgerCause<D>> {
        if !self.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let totals = group.aggregate(state.catalog, self.remaining, CounterClass::Consumed)?;
        if abandoned {
            state.header.integrity_fault = true;
        }
        // Validate the entire release before changing any pending row or channel.
        if !lane_is_open(state, self.lane)
            || totals.iter().enumerate().any(|(index, amount)| {
                group.representatives[index] == index
                    && state.rows[group.positions[index]].state.pending_consumed < *amount
            })
        {
            state.header.integrity_fault = true;
            self.active = false;
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        for (index, amount) in totals.into_iter().enumerate() {
            if group.representatives[index] == index {
                state.rows[group.positions[index]].state.pending_consumed -= amount;
            }
        }
        state.header.open_envelopes -= 1;
        match self.lane {
            WorkLane::Producer => state.header.producer_open = false,
            WorkLane::Protected => state.header.protected_open = false,
            WorkLane::Internal => {}
        }
        self.active = false;
        Ok(())
    }
}

impl WorkFrame<1> {
    /// Admits only the next real leaf, retaining the same explicitly owned envelope.
    pub(crate) fn replenish_one<D: Copy>(
        &mut self,
        group: &WorkLayout<1>,
        state: &mut WorkView<'_, D>,
    ) -> Result<(), LedgerCause<D>> {
        if !self.active
            || self.faulted
            || self.lane != WorkLane::Producer
            || group.grant != WorkGrant::Ordinary
            || self.remaining != [0]
        {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        if !lane_is_open(state, self.lane)
            || state.header.parser_parent_open
            || state.header.parser_child_open
        {
            state.header.integrity_fault = true;
            self.faulted = true;
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        group.admit_observed_totals(state, [1])?;
        self.remaining = [1];
        Ok(())
    }
}

fn lane_is_open<D>(state: &WorkView<'_, D>, lane: WorkLane) -> bool {
    state.header.open_envelopes != 0
        && match lane {
            WorkLane::Producer => state.header.producer_open,
            WorkLane::Protected => state.header.protected_open,
            WorkLane::Internal => true,
        }
}

// Resolvers only copy immutable catalog metadata or a known key; never pass user callbacks.
pub(super) fn add_attempted<D>(
    value: u64,
    amount: u64,
    dimension: impl Fn() -> D + Copy,
) -> Result<u64, LedgerCause<D>> {
    value
        .checked_add(amount)
        .ok_or_else(|| LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: dimension(),
        })
}

pub(super) fn add_consumed<D>(
    value: u64,
    amount: u64,
    dimension: impl Fn() -> D + Copy,
) -> Result<u64, LedgerCause<D>> {
    value
        .checked_add(amount)
        .ok_or_else(|| LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: dimension(),
        })
}

pub(super) fn check_limit<D>(
    class: CounterClass,
    dimension: impl Fn() -> D + Copy,
    value: u64,
    limit: u64,
) -> Result<(), LedgerCause<D>> {
    if value > limit {
        return Err(LedgerCause::LimitExceeded {
            class,
            dimension: dimension(),
            value,
            limit,
        });
    }
    Ok(())
}

pub(super) fn check_effective<D: Copy>(
    dimension: impl Fn() -> D + Copy,
    consumed: u64,
    pending: u64,
    attempted: u64,
    limit: u64,
) -> Result<(), LedgerCause<D>> {
    let effective = add_consumed(consumed, pending, dimension)?;
    if effective > attempted {
        return Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: dimension(),
            consumed: effective,
            attempted,
        });
    }
    check_limit(CounterClass::Consumed, dimension, effective, limit)
}

pub(super) fn charge_attempted_entry<D: Copy>(
    row: &mut WorkEntry<D>,
    dimension: impl Fn() -> D + Copy,
    amount: u64,
) -> Result<(), LedgerCause<D>> {
    row.state.attempted = add_attempted(row.state.attempted, amount, dimension)?;
    check_limit(
        CounterClass::Attempted,
        dimension,
        row.state.attempted,
        row.policy.attempted_limit,
    )
}

pub(super) fn consume_observed_entry<D: Copy>(
    row: &mut WorkEntry<D>,
    dimension: impl Fn() -> D + Copy,
    amount: u64,
) -> Result<(), LedgerCause<D>> {
    let consumed = add_consumed(row.state.consumed, amount, dimension)?;
    if consumed > row.state.attempted {
        return Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: dimension(),
            consumed,
            attempted: row.state.attempted,
        });
    }
    row.state.consumed = consumed;
    check_effective(
        dimension,
        consumed,
        row.state.pending_consumed,
        row.state.attempted,
        row.policy.consumed_limit,
    )
}

/// The work prefix of the parent's joined validation, in the existing ledger precedence.
pub(in crate::ledger) fn validate_work_state<D: Copy>(
    dimension: D,
    policy: WorkPolicy,
    state: WorkDimensionState,
) -> Result<(), LedgerCause<D>> {
    let effective = add_consumed(state.consumed, state.pending_consumed, || dimension)?;
    if effective > state.attempted {
        return Err(LedgerCause::ConsumedExceedsAttempted {
            dimension,
            consumed: effective,
            attempted: state.attempted,
        });
    }
    check_limit(
        CounterClass::Attempted,
        || dimension,
        state.attempted,
        policy.attempted_limit,
    )?;
    check_limit(
        CounterClass::Consumed,
        || dimension,
        effective,
        policy.consumed_limit,
    )
}

#[derive(Clone, Copy)]
pub(crate) enum ParserChannel {
    Parent,
    Child,
}

impl ParserChannel {
    pub(super) fn is_open(self, header: &WorkHeader) -> bool {
        match self {
            Self::Parent => header.parser_parent_open,
            Self::Child => header.parser_child_open,
        }
    }

    fn set_open(self, header: &mut WorkHeader, open: bool) {
        match self {
            Self::Parent => header.parser_parent_open = open,
            Self::Child => header.parser_child_open = open,
        }
    }

    fn has_child(self, header: &WorkHeader) -> bool {
        matches!(self, Self::Parent) && header.parser_child_open
    }
}

impl<const N: usize> WorkLayout<N> {
    pub(crate) fn begin_parser<D: Copy>(
        &self,
        state: &mut WorkView<'_, D>,
        amounts: [u64; N],
        item_limits: [Option<u64>; N],
        channel: ParserChannel,
    ) -> Result<ParserFrame<N>, ParserWorkError<D>> {
        let catalog = state.catalog;
        if self.grant != WorkGrant::Ordinary
            || state.header.producer_open
            || state.header.protected_open
            || channel.is_open(state.header)
            || channel.has_child(state.header)
        {
            return Err(LedgerCause::InactiveLinearWorkPermit.into());
        }
        for (index, amount) in amounts.into_iter().enumerate() {
            charge_attempted_entry(
                &mut state.rows[self.positions[index]],
                || self.dimension(catalog, index),
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
        channel.set_open(state.header, true);
        Ok(ParserFrame {
            remaining: amounts,
            item_limits,
            channel,
            active: true,
        })
    }
}

pub(crate) struct ParserFrame<const N: usize> {
    pub(crate) remaining: [u64; N],
    item_limits: [Option<u64>; N],
    channel: ParserChannel,
    pub(crate) active: bool,
}

impl<const N: usize> ParserFrame<N> {
    /// Records one exact group prefix without reserving a pending-consumed envelope.
    pub(crate) fn complete<D: Copy>(
        &mut self,
        group: &WorkLayout<N>,
        state: &mut WorkView<'_, D>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        self.validate(state)?;
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
        // Channel validation and numeric completion share one short borrow. No callback or
        // physical action runs while it is held; the ordinary group uses this same kernel.
        group.consume_direct_atomic(state, amounts)?;
        for (remaining, amount) in self.remaining.iter_mut().zip(amounts) {
            *remaining -= amount;
        }
        Ok(())
    }

    fn validate<D>(&self, state: &mut WorkView<'_, D>) -> Result<(), LedgerCause<D>> {
        if !self.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        if !self.channel.is_open(state.header)
            || state.header.producer_open
            || state.header.protected_open
        {
            state.header.integrity_fault = true;
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        if self.channel.has_child(state.header) {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(())
    }

    pub(crate) fn resolve<D>(&mut self, state: &mut WorkView<'_, D>) -> Result<(), LedgerCause<D>> {
        if !self.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let invalid_channel = !self.channel.is_open(state.header)
            || self.channel.has_child(state.header)
            || state.header.producer_open
            || state.header.protected_open;
        if invalid_channel {
            state.header.integrity_fault = true;
        }
        self.channel.set_open(state.header, false);
        self.active = false;
        if invalid_channel {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(())
    }
}

impl ParserFrame<1> {
    /// Admits another chunk only for a fully consumed scope without a per-item limit.
    pub(crate) fn replenish<D: Copy>(
        &mut self,
        group: &WorkLayout<1>,
        state: &mut WorkView<'_, D>,
        amount: u64,
    ) -> Result<(), LedgerCause<D>> {
        let catalog = state.catalog;
        {
            self.validate(state)?;
            if self.item_limits[0].is_some() || self.remaining[0] != 0 {
                return Err(LedgerCause::InactiveLinearWorkPermit);
            }
            charge_attempted_entry(
                &mut state.rows[group.positions[0]],
                || group.dimension(catalog, 0),
                amount,
            )?;
        }
        self.remaining[0] = amount;
        Ok(())
    }
}

/// Shared persistent ownership check. The aliasable adapter additionally checks borrow faults.
pub(in crate::ledger) fn check_quiescent<D>(
    rows: &[WorkEntry<D>],
    header: &WorkHeader,
) -> Result<(), LedgerCause<D>> {
    if header.integrity_fault {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    if header.open_envelopes != 0
        || header.producer_open
        || header.parser_parent_open
        || header.parser_child_open
        || header.protected_open
        || rows.iter().any(|row| row.state.pending_consumed != 0)
    {
        return Err(LedgerCause::OutstandingLinearWorkPermits {
            count: header.open_envelopes.max(1),
        });
    }
    Ok(())
}
