//! Invocation-local work accounting. Catalog storage is prepared and metered by the parent.
//!
//! The parent must keep row identities and positions immutable, and may change numeric policy
//! only through its joint work/storage transaction. No user code may run while a guard is held.
//! Numeric errors after successful binding name the registered catalog key; absent-key binding
//! errors retain the supplied key.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::num::NonZeroU64;

use super::{CounterClass, DimensionPolicy, LedgerCause, LinearWorkCompletion};

mod kernel;
mod parser;

pub(crate) use kernel::ParserChannel;
pub(crate) use kernel::{ParserFrame, WorkFrame, WorkLayout, WorkView};
#[cfg(test)]
use kernel::{
    add_attempted, add_consumed, charge_attempted_entry, check_limit, consume_observed_entry,
};
pub(super) use kernel::{check_quiescent, validate_work_state};

pub(crate) use parser::ParserWorkError;

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

pub(super) struct WorkHeader {
    pub(super) mode: WorkMode,
    pub(super) open_envelopes: u64,
    pub(super) producer_open: bool,
    pub(super) parser_parent_open: bool,
    pub(super) parser_child_open: bool,
    pub(super) protected_open: bool,
    pub(super) protected_dimension: Option<usize>,
    pub(super) integrity_fault: bool,
}

pub(super) struct WorkState<D> {
    pub(super) rows: Vec<WorkEntry<D>>,
    pub(super) header: WorkHeader,
}

impl<D> WorkState<D> {
    fn check_integrity(
        &self,
        catalog_len: usize,
        borrow_fault: bool,
    ) -> Result<(), LedgerCause<D>> {
        if borrow_fault || self.header.integrity_fault || self.rows.len() != catalog_len {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        Ok(())
    }

    pub(super) fn view<'a>(&'a mut self, catalog: &'a [D]) -> WorkView<'a, D> {
        WorkView {
            catalog,
            rows: &mut self.rows,
            header: &mut self.header,
        }
    }
}

pub(super) struct LocalWork<D> {
    catalog: Vec<D>,
    pub(super) state: RefCell<WorkState<D>>,
    borrow_fault: Cell<bool>,
}

pub(crate) struct InternalWork<'op, D> {
    pub(super) catalog: &'op [D],
    state: &'op mut WorkState<D>,
    borrow_fault: &'op Cell<bool>,
    shared_available: bool,
    mutable_available: bool,
}

impl<D> LocalWork<D> {
    /// Takes ownership of both preallocated buffers without allocating or comparing keys.
    pub(super) fn new(catalog: Vec<D>, rows: Vec<WorkEntry<D>>) -> Self {
        let integrity_fault = catalog.len() != rows.len();
        Self {
            catalog,
            state: RefCell::new(WorkState {
                rows,
                header: WorkHeader {
                    mode: WorkMode::Ordinary,
                    open_envelopes: 0,
                    producer_open: false,
                    parser_parent_open: false,
                    parser_child_open: false,
                    protected_open: false,
                    protected_dimension: None,
                    integrity_fault,
                },
            }),
            borrow_fault: Cell::new(false),
        }
    }

    pub(super) fn catalogue(&self) -> &[D] {
        &self.catalog
    }

    pub(super) fn split_internal(&mut self) -> InternalWork<'_, D> {
        let shared_available = self.state.try_borrow().is_ok();
        let mutable_available = self.state.try_borrow_mut().is_ok();
        InternalWork {
            catalog: &self.catalog,
            state: self.state.get_mut(),
            borrow_fault: &self.borrow_fault,
            shared_available,
            mutable_available,
        }
    }

    /// Exclusive entry is possible only at quiescence. A forgotten dynamic guard remains a
    /// terminal integrity fault; get_mut is never used as a recovery operation.
    pub(super) fn exclusive(&mut self) -> Result<(&[D], WorkView<'_, D>), LedgerCause<D>> {
        self.check_quiescent()?;
        {
            // A leaked shared Ref permits another shared borrow. Only this short exclusive
            // availability probe rejects both kinds of forgotten guard and latches borrow_fault.
            // The probe ends before get_mut lends ordinary references or any action runs.
            let state = self.borrow_state_mut()?;
            self.ensure_active(&state, WorkGrant::Ordinary)?;
        }
        Ok((&self.catalog, self.state.get_mut().view(&self.catalog)))
    }

    /// Reborrows the existing owner for a library-owned Internal action. Its enclosing frame may
    /// remain active; this is not an external invocation entry or recovery of a forgotten guard.
    pub(super) fn reborrow(&mut self) -> Result<(&[D], WorkView<'_, D>), LedgerCause<D>> {
        {
            let state = self.borrow_state_mut()?;
            self.check_integrity(&state)?;
        }
        Ok((&self.catalog, self.state.get_mut().view(&self.catalog)))
    }

    /// Consumes the exact numeric state before final composition takes any storage guard.
    pub(super) fn into_state(self) -> (Vec<D>, WorkState<D>) {
        // Either kind of forgotten guard remains a fault even though into_inner can recover T.
        let unavailable = self.state.try_borrow_mut().is_err();
        let Self {
            catalog,
            state,
            borrow_fault,
        } = self;
        let expected_rows = catalog.len();
        let mut state = state.into_inner();
        state.header.integrity_fault |=
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
        state.check_integrity(self.catalog.len(), self.borrow_fault.get())
    }

    /// Allows interim observations with active work, but never after an integrity/borrow fault.
    pub(super) fn validate_integrity(&self) -> Result<(), LedgerCause<D>> {
        let state = self.borrow_state()?;
        self.check_integrity(&state)
    }

    pub(super) fn ensure_active(
        &self,
        state: &WorkState<D>,
        grant: WorkGrant,
    ) -> Result<(), LedgerCause<D>> {
        self.check_integrity(state)?;
        kernel::check_active_mode(&state.header, grant)
    }

    /// Outstanding ownership is independent of remaining amounts, including zero envelopes.
    pub(super) fn check_quiescent(&self) -> Result<(), LedgerCause<D>> {
        let state = self.borrow_state()?;
        self.check_integrity(&state)?;
        kernel::check_quiescent(&state.rows, &state.header)
    }
}

impl<D> InternalWork<'_, D> {
    pub(crate) fn loan(&mut self) -> InternalWork<'_, D> {
        InternalWork {
            catalog: self.catalog,
            state: self.state,
            borrow_fault: self.borrow_fault,
            shared_available: self.shared_available,
            mutable_available: self.mutable_available,
        }
    }

    pub(super) fn read_state(&self) -> Result<&WorkState<D>, LedgerCause<D>> {
        if !self.shared_available {
            self.borrow_fault.set(true);
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        Ok(self.state)
    }

    pub(super) fn ensure_active(
        &self,
        state: &WorkState<D>,
        grant: WorkGrant,
    ) -> Result<(), LedgerCause<D>> {
        state.check_integrity(self.catalog.len(), self.borrow_fault.get())?;
        kernel::check_active_mode(&state.header, grant)
    }

    pub(crate) fn reborrow(&mut self) -> Result<super::ExclusiveWork<'_, D>, LedgerCause<D>> {
        if !self.mutable_available {
            self.borrow_fault.set(true);
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        self.state
            .check_integrity(self.catalog.len(), self.borrow_fault.get())?;
        Ok(super::ExclusiveWork {
            catalog: self.catalog,
            work: self.state.view(self.catalog),
        })
    }
}

impl<D: Copy> InternalWork<'_, D> {
    pub(crate) fn resolve<const N: usize>(
        &mut self,
        layout: WorkLayout<N>,
        frame: &mut WorkFrame<N>,
        abandoned: bool,
    ) -> Result<(), LedgerCause<D>> {
        if !frame.active {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        layout.aggregate(self.catalog, frame.remaining, CounterClass::Consumed)?;
        if !self.mutable_available {
            self.borrow_fault.set(true);
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        if self.state.rows.len() != self.catalog.len() {
            self.state.header.integrity_fault = true;
            frame.active = false;
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        frame.resolve(&layout, &mut self.state.view(self.catalog), abandoned)
    }
}

impl<D: Copy + Ord> InternalWork<'_, D> {
    pub(crate) fn bind_ordinary<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        self.bind(dimensions, WorkGrant::Ordinary)
    }

    pub(crate) fn bind_protected<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        self.bind(dimensions, WorkGrant::Protected)
    }

    pub(crate) fn bind_share<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        let grant = match self.read_state()?.header.mode {
            WorkMode::Protected => WorkGrant::Protected,
            _ => WorkGrant::Ordinary,
        };
        self.bind(dimensions, grant)
    }

    fn bind<const N: usize>(
        &self,
        dimensions: [D; N],
        grant: WorkGrant,
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        let layout = WorkLayout::bind(self.catalog, &dimensions, grant)?;
        let state = self.read_state()?;
        self.ensure_active(state, grant)?;
        layout.validate_binding(&state.header, &dimensions)?;
        Ok(layout)
    }
}

impl<D: Copy + Ord> LocalWork<D> {
    /// All key comparisons precede the dynamic borrow. Both entry paths use this layout.
    pub(super) fn bind<const N: usize>(
        &self,
        dimensions: [D; N],
        grant: WorkGrant,
    ) -> Result<BoundWorkGroup<'_, D, N>, LedgerCause<D>> {
        let layout = self.bind_layout(dimensions, grant)?;
        Ok(BoundWorkGroup {
            owner: self,
            layout,
        })
    }

    pub(super) fn bind_layout<const N: usize>(
        &self,
        dimensions: [D; N],
        grant: WorkGrant,
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        let layout = WorkLayout::bind(&self.catalog, &dimensions, grant)?;
        let state = self.borrow_state()?;
        self.ensure_active(&state, grant)?;
        layout.validate_binding(&state.header, &dimensions)?;
        Ok(layout)
    }
}

/// Transitional aliasable adapter; removed with the phase-2 caller migration.
#[derive(Clone, Copy)]
pub(crate) struct BoundWorkGroup<'op, D: Copy, const N: usize> {
    owner: &'op LocalWork<D>,
    layout: WorkLayout<N>,
}

impl<'op, D: Copy, const N: usize> BoundWorkGroup<'op, D, N> {
    fn borrow_active(&self) -> Result<RefMut<'op, WorkState<D>>, LedgerCause<D>> {
        let state = self.owner.borrow_state_mut()?;
        self.owner.ensure_active(&state, self.layout.grant)?;
        self.layout
            .validate_role_header(&self.owner.catalog, &state.header)?;
        Ok(state)
    }

    pub(crate) fn charge_attempted_in_order(
        &self,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        self.layout.charge_attempted_in_order(
            &mut self.borrow_active()?.view(&self.owner.catalog),
            amounts,
        )
    }

    pub(crate) fn consume_direct_atomic(&self, amounts: [u64; N]) -> Result<(), LedgerCause<D>> {
        self.layout.consume_direct_atomic(
            &mut self.borrow_active()?.view(&self.owner.catalog),
            amounts,
        )
    }

    pub(crate) fn charge_observed_ordered(&self, amounts: [u64; N]) -> Result<(), LedgerCause<D>> {
        self.layout.charge_observed_ordered(
            &mut self.borrow_active()?.view(&self.owner.catalog),
            amounts,
        )
    }

    pub(crate) fn consume_observed_in_order(
        &self,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        self.layout.consume_observed_in_order(
            &mut self.borrow_active()?.view(&self.owner.catalog),
            amounts,
        )
    }

    pub(crate) fn preflight_repeated_observed(
        &self,
        amounts: [u64; N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        let state = self.owner.borrow_state()?;
        self.preflight_repeated_observed_state(&state, amounts, repetitions)
    }

    pub(super) fn preflight_repeated_observed_state(
        &self,
        state: &WorkState<D>,
        amounts: [u64; N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        self.owner.ensure_active(state, self.layout.grant)?;
        self.layout.preflight_repeated_observed(
            &self.owner.catalog,
            &state.rows,
            &state.header,
            amounts,
            repetitions,
        )
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod exclusive_tests;
