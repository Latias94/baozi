//! Invocation-local work accounting. Catalog storage is prepared and metered by the parent.
//!
//! The parent must keep row identities and positions immutable, and may change numeric policy
//! only through its joint work/storage transaction. No user code may run while a guard is held.
//! Numeric errors after successful binding name the registered catalog key; absent-key binding
//! errors retain the supplied key.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::num::NonZeroU64;

use super::{CounterClass, DimensionPolicy, LedgerCause, LinearWorkCompletion};

mod parser;

pub(crate) use parser::{ParserWorkError, ParserWorkScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WorkPolicy {
    pub attempted_limit: u64,
    pub consumed_limit: u64,
}

impl From<DimensionPolicy> for WorkPolicy {
    fn from(policy: DimensionPolicy) -> Self {
        Self {
            attempted_limit: policy.attempted_limit(),
            consumed_limit: policy.consumed_limit(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct WorkDimensionState {
    pub attempted: u64,
    pub consumed: u64,
    pub pending_consumed: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct WorkEntry<D> {
    pub dimension: D,
    pub policy: WorkPolicy,
    pub state: WorkDimensionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkMode {
    Ordinary,
    OrdinarySealed,
    Protected,
    Closed,
}

/// Only the invocation owner can select the protected role at binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkGrant {
    Ordinary,
    Protected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkLane {
    Producer,
    Internal,
    Protected,
}

pub(super) struct WorkState<D> {
    pub(super) rows: Vec<WorkEntry<D>>,
    pub(super) mode: WorkMode,
    pub(super) open_envelopes: u64,
    pub(super) producer_open: bool,
    pub(super) parser_parent_open: bool,
    pub(super) parser_child_open: bool,
    pub(super) protected_open: bool,
    pub(super) protected_dimension: Option<usize>,
    pub(super) integrity_fault: bool,
}

pub(super) struct LocalWork<D> {
    catalog: Vec<D>,
    pub(super) state: RefCell<WorkState<D>>,
    borrow_fault: Cell<bool>,
}

impl<D> LocalWork<D> {
    /// Takes ownership of both preallocated buffers without allocating or comparing keys.
    pub(super) fn new(catalog: Vec<D>, rows: Vec<WorkEntry<D>>) -> Self {
        let integrity_fault = catalog.len() != rows.len();
        Self {
            catalog,
            state: RefCell::new(WorkState {
                rows,
                mode: WorkMode::Ordinary,
                open_envelopes: 0,
                producer_open: false,
                parser_parent_open: false,
                parser_child_open: false,
                protected_open: false,
                protected_dimension: None,
                integrity_fault,
            }),
            borrow_fault: Cell::new(false),
        }
    }

    pub(super) fn catalogue(&self) -> &[D] {
        &self.catalog
    }

    /// Consumes the exact numeric state before final composition takes any storage guard.
    pub(super) fn into_state(self) -> (Vec<D>, WorkState<D>) {
        // A forgotten exclusive guard must remain a fault even though into_inner can recover T.
        let unavailable = self.state.try_borrow().is_err();
        let Self {
            catalog,
            state,
            borrow_fault,
        } = self;
        let expected_rows = catalog.len();
        let mut state = state.into_inner();
        state.integrity_fault |=
            borrow_fault.into_inner() || unavailable || state.rows.len() != expected_rows;
        (catalog, state)
    }

    /// Reserved for short numeric or joint cold-observation transactions in the parent.
    pub(super) fn borrow_state(&self) -> Result<Ref<'_, WorkState<D>>, LedgerCause<D>> {
        self.state.try_borrow().map_err(|_| {
            self.borrow_fault.set(true);
            LedgerCause::FinalSnapshotShapeMismatch
        })
    }

    pub(super) fn borrow_state_mut(&self) -> Result<RefMut<'_, WorkState<D>>, LedgerCause<D>> {
        self.state.try_borrow_mut().map_err(|_| {
            self.borrow_fault.set(true);
            LedgerCause::FinalSnapshotShapeMismatch
        })
    }

    fn check_integrity(&self, state: &WorkState<D>) -> Result<(), LedgerCause<D>> {
        if self.borrow_fault.get()
            || state.integrity_fault
            || state.rows.len() != self.catalog.len()
        {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        Ok(())
    }

    /// Allows interim observations with active work, but never after an integrity/borrow fault.
    pub(super) fn validate_integrity(&self) -> Result<(), LedgerCause<D>> {
        let state = self.borrow_state()?;
        self.check_integrity(&state)
    }

    fn ensure_active(&self, state: &WorkState<D>, grant: WorkGrant) -> Result<(), LedgerCause<D>> {
        self.check_integrity(state)?;
        match (state.mode, grant) {
            (WorkMode::Ordinary, WorkGrant::Ordinary)
            | (WorkMode::Protected, WorkGrant::Protected) => Ok(()),
            (WorkMode::Closed, _) => Err(LedgerCause::PublicationAlreadyCompleted),
            (WorkMode::OrdinarySealed, _) | (WorkMode::Protected, WorkGrant::Ordinary) => {
                Err(LedgerCause::PublicationAlreadyPending)
            }
            (WorkMode::Ordinary, WorkGrant::Protected) => {
                Err(LedgerCause::InactiveLinearWorkPermit)
            }
        }
    }

    /// Outstanding ownership is independent of remaining amounts, including zero envelopes.
    pub(super) fn check_quiescent(&self) -> Result<(), LedgerCause<D>> {
        let state = self.borrow_state()?;
        self.check_integrity(&state)?;
        if state.open_envelopes != 0
            || state.producer_open
            || state.parser_parent_open
            || state.parser_child_open
            || state.protected_open
            || state.rows.iter().any(|row| row.state.pending_consumed != 0)
        {
            return Err(LedgerCause::OutstandingLinearWorkPermits {
                count: state.open_envelopes.max(1),
            });
        }
        Ok(())
    }
}

impl<D: Copy + Ord> LocalWork<D> {
    /// All key comparisons precede the work borrow. Numeric calls reuse this fixed layout.
    pub(super) fn bind<const N: usize>(
        &self,
        dimensions: [D; N],
        grant: WorkGrant,
    ) -> Result<BoundWorkGroup<'_, D, N>, LedgerCause<D>> {
        let mut positions = [0; N];
        for (index, dimension) in dimensions.iter().copied().enumerate() {
            positions[index] = self
                .catalog
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
        let state = self.borrow_state()?;
        self.ensure_active(&state, grant)?;
        if grant == WorkGrant::Ordinary
            && let Some(index) = positions
                .iter()
                .position(|position| Some(*position) == state.protected_dimension)
        {
            return Err(LedgerCause::UnknownDimension {
                dimension: dimensions[index],
            });
        }
        if grant == WorkGrant::Protected
            && positions
                .iter()
                .any(|position| Some(*position) != state.protected_dimension)
        {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(BoundWorkGroup {
            owner: self,
            positions,
            representatives,
            completion_order,
            grant,
        })
    }
}

/// A capability for exactly one owner; no method accepts a replacement owner.
#[derive(Clone, Copy)]
pub(crate) struct BoundWorkGroup<'op, D: Copy, const N: usize> {
    owner: &'op LocalWork<D>,
    positions: [usize; N],
    representatives: [usize; N],
    completion_order: [usize; N],
    grant: WorkGrant,
}

impl<'op, D: Copy, const N: usize> BoundWorkGroup<'op, D, N> {
    fn dimension(&self, index: usize) -> D {
        self.owner.catalog[self.positions[index]]
    }

    fn aggregate(
        &self,
        amounts: [u64; N],
        class: CounterClass,
    ) -> Result<[u64; N], LedgerCause<D>> {
        let mut totals = [0_u64; N];
        for (index, amount) in amounts.into_iter().enumerate() {
            let first = self.representatives[index];
            totals[first] = totals[first].checked_add(amount).ok_or_else(|| {
                LedgerCause::ArithmeticOverflow {
                    class,
                    dimension: self.dimension(index),
                }
            })?;
        }
        Ok(totals)
    }

    fn borrow_active(&self) -> Result<RefMut<'op, WorkState<D>>, LedgerCause<D>> {
        let state = self.owner.borrow_state_mut()?;
        self.owner.ensure_active(&state, self.grant)?;
        self.validate_role(&state)?;
        Ok(state)
    }

    fn validate_role(&self, state: &WorkState<D>) -> Result<(), LedgerCause<D>> {
        match self.grant {
            WorkGrant::Ordinary => {
                if let Some(index) = self
                    .positions
                    .iter()
                    .position(|position| Some(*position) == state.protected_dimension)
                {
                    return Err(LedgerCause::UnknownDimension {
                        dimension: self.dimension(index),
                    });
                }
            }
            WorkGrant::Protected => {
                if self
                    .positions
                    .iter()
                    .any(|position| Some(*position) != state.protected_dimension)
                {
                    return Err(LedgerCause::InactiveLinearWorkPermit);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn charge_attempted_in_order(
        &self,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        let mut state = self.borrow_active()?;
        for (index, amount) in amounts.into_iter().enumerate() {
            charge_attempted_entry(
                &mut state.rows[self.positions[index]],
                || self.dimension(index),
                amount,
            )?;
        }
        Ok(())
    }

    /// Validates distinct dimensions in catalog order and commits the complete group or nothing.
    pub(crate) fn consume_direct_atomic(&self, amounts: [u64; N]) -> Result<(), LedgerCause<D>> {
        let mut state = self.borrow_active()?;
        self.consume_direct_atomic_in_state(&mut state, amounts)
    }

    fn consume_direct_atomic_in_state(
        &self,
        state: &mut WorkState<D>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        let mut consumed = [0; N];
        let mut cursor = 0;
        while cursor < N {
            let index = self.completion_order[cursor];
            let position = self.positions[index];
            let dimension = || self.dimension(index);
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
    pub(crate) fn charge_observed_ordered(&self, amounts: [u64; N]) -> Result<(), LedgerCause<D>> {
        let mut state = self.borrow_active()?;
        for (index, amount) in amounts.into_iter().enumerate() {
            let row = &mut state.rows[self.positions[index]];
            let dimension = || self.dimension(index);
            charge_attempted_entry(row, dimension, amount)?;
            consume_observed_entry(row, dimension, amount)?;
        }
        Ok(())
    }

    /// Records physical completion without charging another attempted envelope.
    pub(crate) fn consume_observed_in_order(
        &self,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        let mut state = self.borrow_active()?;
        for (index, amount) in amounts.into_iter().enumerate() {
            consume_observed_entry(
                &mut state.rows[self.positions[index]],
                || self.dimension(index),
                amount,
            )?;
        }
        Ok(())
    }

    pub(crate) fn preflight_repeated_observed(
        &self,
        amounts: [u64; N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        let state = self.owner.borrow_state()?;
        self.preflight_repeated_observed_state(&state, amounts, repetitions)
    }

    /// Checks a prebound startup group in the owner's existing joint observation interval.
    pub(super) fn preflight_repeated_observed_state(
        &self,
        state: &WorkState<D>,
        amounts: [u64; N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        self.owner.ensure_active(state, self.grant)?;
        self.validate_role(state)?;
        let mut repeated = amounts;
        for (index, amount) in repeated.iter_mut().enumerate() {
            *amount = amount.checked_mul(repetitions.get()).ok_or_else(|| {
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Attempted,
                    dimension: self.dimension(index),
                }
            })?;
        }
        for index in 0..N {
            if self.representatives[index] != index {
                continue;
            }
            let row = &state.rows[self.positions[index]];
            let dimension = || self.dimension(index);
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

    pub(crate) fn admit_observed(
        &self,
        amounts: [u64; N],
        lane: WorkLane,
    ) -> Result<LinearWorkBatchPermit<'op, D, N>, LedgerCause<D>> {
        let mut state = self.borrow_active()?;
        match (self.grant, lane) {
            (WorkGrant::Ordinary, WorkLane::Producer)
                if !state.producer_open
                    && !state.parser_parent_open
                    && !state.parser_child_open => {}
            (WorkGrant::Protected, WorkLane::Protected) if !state.protected_open => {}
            (_, WorkLane::Internal) => {}
            _ => return Err(LedgerCause::InactiveLinearWorkPermit),
        }
        let totals = self.aggregate(amounts, CounterClass::Attempted)?;
        let open_envelopes = state
            .open_envelopes
            .checked_add(1)
            .ok_or(LedgerCause::FinalSnapshotShapeMismatch)?;
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
            let dimension = || self.dimension(index);
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
        state.open_envelopes = open_envelopes;
        match lane {
            WorkLane::Producer => state.producer_open = true,
            WorkLane::Protected => state.protected_open = true,
            WorkLane::Internal => {}
        }
        Ok(LinearWorkBatchPermit {
            group: *self,
            remaining: amounts,
            lane,
            active: true,
            faulted: false,
        })
    }
}

#[must_use = "finish the work envelope explicitly, even after completing every prefix"]
pub(crate) struct LinearWorkBatchPermit<'op, D: Copy, const N: usize> {
    group: BoundWorkGroup<'op, D, N>,
    remaining: [u64; N],
    lane: WorkLane,
    active: bool,
    faulted: bool,
}

impl<D: Copy, const N: usize> LinearWorkBatchPermit<'_, D, N> {
    pub(crate) fn remaining(&self) -> [u64; N] {
        self.remaining
    }

    pub(crate) fn complete(&mut self, index: usize, amount: u64) -> Result<(), LedgerCause<D>> {
        self.complete_with_outcome(index, amount).into_result()
    }

    pub(crate) fn complete_with_outcome(
        &mut self,
        index: usize,
        amount: u64,
    ) -> LinearWorkCompletion<D> {
        if index >= N {
            return LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit);
        }
        let mut amounts = [0; N];
        amounts[index] = amount;
        self.complete_batch_with_outcome(amounts)
    }

    #[cfg(test)]
    pub(crate) fn complete_batch(&mut self, amounts: [u64; N]) -> Result<(), LedgerCause<D>> {
        self.complete_batch_with_outcome(amounts).into_result()
    }

    pub(crate) fn complete_batch_with_outcome(
        &mut self,
        amounts: [u64; N],
    ) -> LinearWorkCompletion<D> {
        if !self.active
            || self.faulted
            || amounts
                .iter()
                .zip(self.remaining)
                .any(|(amount, remaining)| *amount > remaining)
        {
            return LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit);
        }
        let mut state = match self.group.borrow_active() {
            Ok(state) => state,
            Err(cause) => {
                self.faulted = true;
                return LinearWorkCompletion::NotCompleted(cause);
            }
        };
        if !lane_is_open(&state, self.lane) {
            state.integrity_fault = true;
            self.faulted = true;
            return LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit);
        }
        let totals = match self.group.aggregate(amounts, CounterClass::Consumed) {
            Ok(totals) => totals,
            Err(cause) => {
                state.integrity_fault = true;
                self.faulted = true;
                return LinearWorkCompletion::NotCompleted(cause);
            }
        };
        let mut consumed = [0; N];
        let mut pending = [0; N];
        let mut failure = None;
        let mut prefix_exceeds_attempted = false;
        for index in self.group.completion_order {
            let amount = totals[index];
            if amount == 0 || self.group.representatives[index] != index {
                continue;
            }
            let row = &state.rows[self.group.positions[index]];
            let group = &self.group;
            let dimension = || group.dimension(index);
            let Some(next_pending) = row.state.pending_consumed.checked_sub(amount) else {
                state.integrity_fault = true;
                self.faulted = true;
                return LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit);
            };
            let next_consumed = match add_consumed(row.state.consumed, amount, dimension) {
                Ok(value) => value,
                Err(cause) => {
                    state.integrity_fault = true;
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
        for index in self.group.completion_order {
            if totals[index] != 0 && self.group.representatives[index] == index {
                let row = &mut state.rows[self.group.positions[index]];
                row.state.consumed = consumed[index];
                row.state.pending_consumed = pending[index];
            }
        }
        for (remaining, amount) in self.remaining.iter_mut().zip(amounts) {
            *remaining -= amount;
        }
        if let Some(cause) = failure {
            state.integrity_fault |= prefix_exceeds_attempted;
            self.faulted = true;
            LinearWorkCompletion::CompletedWithLedgerFailure(cause)
        } else {
            LinearWorkCompletion::Completed
        }
    }

    /// Full completion does not close ownership until this explicit resolution.
    pub(crate) fn finish(mut self) -> Result<(), LedgerCause<D>> {
        if self.remaining.iter().any(|amount| *amount != 0) {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        self.resolve(false)
    }

    /// Releases only the unfinished pending suffix; attempts and physical prefixes survive.
    pub(crate) fn finish_partial(mut self) -> Result<(), LedgerCause<D>> {
        self.resolve(false)
    }

    fn resolve(&mut self, abandoned: bool) -> Result<(), LedgerCause<D>> {
        if !self.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let totals = self
            .group
            .aggregate(self.remaining, CounterClass::Consumed)?;
        let mut state = self.group.owner.borrow_state_mut()?;
        if abandoned {
            state.integrity_fault = true;
        }
        // Validate the entire release before changing any pending row or channel.
        if state.rows.len() != self.group.owner.catalog.len()
            || !lane_is_open(&state, self.lane)
            || totals.iter().enumerate().any(|(index, amount)| {
                self.group.representatives[index] == index
                    && state.rows[self.group.positions[index]]
                        .state
                        .pending_consumed
                        < *amount
            })
        {
            state.integrity_fault = true;
            self.active = false;
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        for (index, amount) in totals.into_iter().enumerate() {
            if self.group.representatives[index] == index {
                state.rows[self.group.positions[index]]
                    .state
                    .pending_consumed -= amount;
            }
        }
        state.open_envelopes -= 1;
        match self.lane {
            WorkLane::Producer => state.producer_open = false,
            WorkLane::Protected => state.protected_open = false,
            WorkLane::Internal => {}
        }
        self.active = false;
        Ok(())
    }
}

impl<D: Copy, const N: usize> Drop for LinearWorkBatchPermit<'_, D, N> {
    fn drop(&mut self) {
        if self.active {
            // A failed borrow latches the owner's fault and leaves all persistent ownership intact.
            let _ = self.resolve(true);
        }
    }
}

fn lane_is_open<D>(state: &WorkState<D>, lane: WorkLane) -> bool {
    state.open_envelopes != 0
        && match lane {
            WorkLane::Producer => state.producer_open,
            WorkLane::Protected => state.protected_open,
            WorkLane::Internal => true,
        }
}

// Resolvers only copy immutable catalog metadata or a known key; never pass user callbacks.
fn add_attempted<D>(
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

fn add_consumed<D>(
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

fn check_limit<D>(
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

fn check_effective<D: Copy>(
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

fn charge_attempted_entry<D: Copy>(
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

fn consume_observed_entry<D: Copy>(
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
pub(super) fn validate_work_state<D: Copy>(
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

#[cfg(test)]
mod tests;
