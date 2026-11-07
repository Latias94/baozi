//! Invocation-local work accounting. Catalog storage is prepared and metered by the parent.
//!
//! The parent must keep row identities and positions immutable, and may change numeric policy
//! only through its joint work/storage transaction.
//! Numeric errors after successful binding name the registered catalog key; absent-key binding
//! errors retain the supplied key.

#[cfg(test)]
use std::num::NonZeroU64;

use super::{CounterClass, DimensionPolicy, LedgerCause, LinearWorkCompletion};

mod kernel;
mod parser;

pub(crate) use kernel::ParserChannel;
pub(crate) use kernel::{
    ParserFrame, ParserReadRow, ReservedProducerRow, WorkFrame, WorkLayout, WorkReadView, WorkView,
};
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

impl WorkHeader {
    pub(super) const fn new() -> Self {
        Self {
            mode: WorkMode::Ordinary,
            open_envelopes: 0,
            producer_open: false,
            parser_parent_open: false,
            parser_child_open: false,
            protected_open: false,
            protected_dimension: None,
            integrity_fault: false,
        }
    }
}

pub(super) struct WorkState<D> {
    pub(super) rows: Vec<WorkEntry<D>>,
    pub(super) header: WorkHeader,
}

impl<D> WorkState<D> {
    pub(super) fn new(catalog_len: usize, rows: Vec<WorkEntry<D>>) -> Self {
        let integrity_fault = catalog_len != rows.len();
        Self {
            rows,
            header: WorkHeader {
                integrity_fault,
                ..WorkHeader::new()
            },
        }
    }

    pub(super) fn check_integrity(&self, catalog_len: usize) -> Result<(), LedgerCause<D>> {
        kernel::check_row_shape(&self.rows, &self.header, catalog_len)
    }

    pub(super) fn view<'a>(&'a mut self, catalog: &'a [D]) -> WorkView<'a, D> {
        WorkView {
            catalog,
            rows: &mut self.rows,
            header: &mut self.header,
        }
    }

    pub(super) fn as_read<'a>(&'a self, catalog: &'a [D]) -> WorkReadView<'a, D> {
        WorkReadView {
            catalog,
            rows: &self.rows,
            header: &self.header,
        }
    }
}

pub(crate) struct InternalWork<'op, D> {
    pub(super) state: WorkView<'op, D>,
}

impl<'op, D> InternalWork<'op, D> {
    pub(super) fn new(state: WorkView<'op, D>) -> Self {
        Self { state }
    }

    pub(super) fn catalog(&self) -> &[D] {
        self.state.catalog
    }

    pub(crate) fn loan(&mut self) -> InternalWork<'_, D> {
        InternalWork::new(self.state.reborrow())
    }

    pub(super) fn read_state(&self) -> WorkReadView<'_, D> {
        self.state.as_read()
    }

    pub(crate) fn reborrow(&mut self) -> Result<super::ExclusiveWork<'_, D>, LedgerCause<D>> {
        self.read_state().check_integrity()?;
        Ok(super::ExclusiveWork {
            catalog: self.state.catalog,
            work: self.state.reborrow(),
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
        layout.aggregate(self.catalog(), frame.remaining, CounterClass::Consumed)?;
        if self.state.rows.len() != self.catalog().len() {
            self.state.header.integrity_fault = true;
            frame.active = false;
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        frame.resolve(&layout, &mut self.state, abandoned)
    }
}

impl<D: Copy + Ord> InternalWork<'_, D> {
    pub(crate) fn bind_ordinary<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        self.read_state()
            .bind_layout(dimensions, WorkGrant::Ordinary)
    }

    pub(crate) fn bind_protected<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        self.read_state()
            .bind_layout(dimensions, WorkGrant::Protected)
    }

    pub(crate) fn bind_share<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        let grant = match self.read_state().header.mode {
            WorkMode::Protected => WorkGrant::Protected,
            _ => WorkGrant::Ordinary,
        };
        self.read_state().bind_layout(dimensions, grant)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod exclusive_tests;
