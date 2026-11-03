//! Concrete execution views for trusted import and geometry adapters.

use super::*;
mod exclusive;
use crate::retained::{
    ExecutionError, ExecutionGroup, ExecutionOrdinaryStorage, ExecutionParserScope,
    ExecutionPermit, ExecutionServices, ExecutionWork,
};
pub use exclusive::{
    StoreExtensionResult, StoreFailureStorage, StoreImporterBoundary, StoreInvocation,
    StoreInvocationError, StoreInvocationLayout, StoreInvocationParser,
};

/// Work authority borrowed from one active invocation, separate from its storage and control.
pub struct StoreExecutionWork<'op, O> {
    raw: ExecutionWork<'op, O, StorePhaseKey, StoreDimensionKey>,
    domain: &'op StoreKeyDomain,
}

/// Bound storage, control and progress services from one actual invocation.
///
/// An ordinary loan ends before the services are used again:
/// ```
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};
/// use baozi_operation::__retained_store_integration::activation::StoreExecutionServices;
/// fn sequential<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     services: &mut StoreExecutionServices<'_, O, P, U>,
/// ) -> Result<(), StoreRuntimeError> {
///     { let mut loan = services.ordinary(); loan.checkpoint()?; }
///     services.checkpoint()
/// }
/// ```
///
/// A live loan excludes reuse of its services owner:
/// ```compile_fail,E0499
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};
/// use baozi_operation::__retained_store_integration::activation::StoreExecutionServices;
/// fn overlap<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     services: &mut StoreExecutionServices<'_, O, P, U>,
/// ) -> Result<(), StoreRuntimeError> {
///     let mut loan = services.ordinary();
///     services.checkpoint()?;
///     loan.checkpoint()
/// }
/// ```
pub struct StoreExecutionServices<'op, O, PP, U> {
    raw: ExecutionServices<'op, O, StorePhaseKey, PP, U, StoreDimensionKey>,
    domain: &'op StoreKeyDomain,
}

/// Ordinary storage and real poll authority, borrowing no numeric work or progress state.
pub struct StoreOrdinaryStorage<'b, 'op: 'b, O, PP, U> {
    raw: &'b mut ExecutionOrdinaryStorage<'op, O, StorePhaseKey, PP, U, StoreDimensionKey>,
    domain: &'op StoreKeyDomain,
}

/// Fixed owner-bound work layout with no per-action key lookup or comparison.
pub struct StoreBoundWorkGroup<'op, const N: usize> {
    raw: ExecutionGroup<'op, StoreDimensionKey, N>,
}

/// One borrowed observed envelope; full completion still requires explicit finish.
#[must_use = "finish exact or explicitly resolve verified partial work"]
pub struct StoreExecutionPermit<'op, const N: usize> {
    raw: ExecutionPermit<'op, StoreDimensionKey, N>,
}

/// An attempted-only parent or child parser channel on the invocation's existing work owner.
#[must_use = "resolve the parser channel before leaving the execution region"]
pub struct StoreParserWorkScope<'op, const N: usize> {
    raw: ExecutionParserScope<'op, StoreDimensionKey, N>,
}

/// Parser admission projects per-item limits outside the numeric owner's short borrow.
#[derive(Debug)]
pub enum StoreParserWorkError {
    /// Binding, control, or aggregate accounting rejected the channel.
    Runtime(StoreRuntimeError),
    /// The caller-ordered item was attempted but exceeds its local limit.
    ItemLimit {
        index: usize,
        value: u64,
        limit: u64,
    },
}

impl fmt::Display for StoreParserWorkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(cause) => cause.fmt(formatter),
            Self::ItemLimit {
                index,
                value,
                limit,
            } => write!(
                formatter,
                "parser item at lane {index} attempted {value}, above limit {limit}"
            ),
        }
    }
}

impl std::error::Error for StoreParserWorkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(cause) => Some(cause),
            Self::ItemLimit { .. } => None,
        }
    }
}

pub(super) fn map_execution_error(cause: ExecutionError<StoreDimensionKey>) -> StoreRuntimeError {
    match cause {
        ExecutionError::Control(cause) => StoreRuntimeError::Control(cause),
        ExecutionError::Ledger(cause) => StoreRuntimeError::Ledger(cause),
        ExecutionError::Stopped => StoreRuntimeError::FallbackOnly,
    }
}

impl<'op, O: OperationOwner> StoreExecutionWork<'op, O> {
    /// Binds membership, private-role exclusion, duplicates and completion order once.
    pub fn bind_group<const N: usize>(
        &self,
        dimensions: [StoreDimensionKey; N],
    ) -> Result<StoreBoundWorkGroup<'op, N>, StoreRuntimeError> {
        for dimension in dimensions {
            if !self.domain.owns_dimension(dimension) {
                self.raw.reject_binding();
                return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
            }
        }
        self.raw
            .bind(dimensions)
            .map(|raw| StoreBoundWorkGroup { raw })
            .map_err(map_execution_error)
    }

    /// Checks that every envelope lent by the region has resolved, including forgotten ones.
    pub fn finish_region(&self) -> Result<(), StoreRuntimeError> {
        self.raw.finish_region().map_err(map_execution_error)
    }

    /// Observes a qualified ordinary dimension without allocating a complete snapshot.
    pub fn dimension_state(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<RetainedStoreDimensionState, StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            self.raw.reject_binding();
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        self.raw
            .dimension_evidence(dimension)
            .map(|snapshot| RetainedStoreDimensionState {
                attempted: snapshot.attempted(),
                consumed: snapshot.consumed(),
                live_bytes: snapshot.current_live_bytes(),
            })
            .map_err(map_execution_error)
    }

    /// Borrows only storage evidence; the view cannot outlive or cross the invocation.
    pub fn storage_observation(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<StoreStorageObservation<'op>, StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            self.raw.reject_binding();
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        self.raw
            .storage_observation(dimension)
            .map(|raw| StoreStorageObservation { raw })
            .map_err(map_execution_error)
    }

    /// Tightens the sole owner's current policy before selected execution begins.
    pub fn tighten_dimension_policy(
        &self,
        dimension: StoreDimensionKey,
        policy: DimensionPolicy,
    ) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            self.raw.reject_binding();
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        self.raw
            .tighten_policy(dimension, policy)
            .map_err(map_execution_error)
    }
}

impl<'op, const N: usize> StoreBoundWorkGroup<'op, N> {
    /// Admits a semantic parent; its completion must follow every child and a full checkpoint.
    pub fn begin_parser_parent(
        &self,
        amounts: [u64; N],
        item_limits: [Option<u64>; N],
    ) -> Result<StoreParserWorkScope<'op, N>, StoreParserWorkError> {
        self.begin_parser(amounts, item_limits, true)
    }

    /// Admits one child/direct channel, optionally inside a semantic parent.
    pub fn begin_parser_child(
        &self,
        amounts: [u64; N],
        item_limits: [Option<u64>; N],
    ) -> Result<StoreParserWorkScope<'op, N>, StoreParserWorkError> {
        self.begin_parser(amounts, item_limits, false)
    }

    fn begin_parser(
        &self,
        amounts: [u64; N],
        item_limits: [Option<u64>; N],
        parent: bool,
    ) -> Result<StoreParserWorkScope<'op, N>, StoreParserWorkError> {
        self.raw
            .begin_parser(amounts, item_limits, parent)
            .map(|raw| StoreParserWorkScope { raw })
            .map_err(|cause| match cause {
                crate::ledger::ParserWorkError::Ledger(cause) => {
                    StoreParserWorkError::Runtime(StoreRuntimeError::Ledger(cause))
                }
                crate::ledger::ParserWorkError::ItemLimit {
                    index,
                    value,
                    limit,
                } => StoreParserWorkError::ItemLimit {
                    index,
                    value,
                    limit,
                },
            })
    }

    /// Records the complete observed envelope before the physical action starts.
    pub fn admit_observed(
        &self,
        amounts: [u64; N],
    ) -> Result<StoreExecutionPermit<'op, N>, StoreRuntimeError> {
        self.raw
            .admit_observed(amounts)
            .map(|raw| StoreExecutionPermit { raw })
            .map_err(map_execution_error)
    }

    /// Records caller-ordered attempted work for a direct parser action.
    pub fn charge_attempted(&self, amounts: [u64; N]) -> Result<(), StoreRuntimeError> {
        self.raw
            .charge_attempted(amounts)
            .map_err(map_execution_error)
    }

    /// Atomically records a physically completed direct group in prebound numeric order.
    pub fn complete_direct(&self, amounts: [u64; N]) -> Result<(), StoreRuntimeError> {
        self.raw
            .complete_direct(amounts)
            .map_err(map_execution_error)
    }

    /// Records observed physical prefixes in caller order, including applied failure.
    pub fn complete_observed(&self, amounts: [u64; N]) -> Result<(), StoreRuntimeError> {
        self.raw
            .complete_observed(amounts)
            .map_err(map_execution_error)
    }

    /// Records attempted and observed completion for an already performed bounded action.
    pub fn charge_observed(&self, amounts: [u64; N]) -> Result<(), StoreRuntimeError> {
        self.raw
            .charge_observed(amounts)
            .map_err(map_execution_error)
    }
}

impl<const N: usize> StoreParserWorkScope<'_, N> {
    /// Returns the admitted suffix without constructing an evidence snapshot.
    pub fn remaining(&self) -> [u64; N] {
        self.raw.remaining()
    }

    /// Atomically records the exact physical prefix across the prebound group.
    pub fn complete(&mut self, amounts: [u64; N]) -> Result<(), StoreRuntimeError> {
        self.raw.complete(amounts).map_err(map_execution_error)
    }

    /// Records the entire unfinished prefix.
    pub fn complete_all(&mut self) -> Result<(), StoreRuntimeError> {
        self.raw.complete_all().map_err(map_execution_error)
    }

    /// Closes a fully completed channel; an incomplete channel latches failure.
    pub fn finish_exact(self) -> Result<(), StoreRuntimeError> {
        self.raw.finish_exact().map_err(map_execution_error)
    }

    /// Closes ownership after a verified partial parse, retaining every completed prefix.
    pub fn finish_partial(self) -> Result<(), StoreRuntimeError> {
        self.raw.finish_partial().map_err(map_execution_error)
    }
}

impl StoreParserWorkScope<'_, 1> {
    /// Reuses a fully completed singleton without a per-item limit for the next chunk.
    pub fn replenish(&mut self, amount: u64) -> Result<(), StoreRuntimeError> {
        self.raw.replenish(amount).map_err(map_execution_error)
    }
}

impl<const N: usize> StoreExecutionPermit<'_, N> {
    /// Returns each unfinished amount without a snapshot allocation.
    pub fn remaining(&self) -> [u64; N] {
        self.raw.remaining()
    }

    /// Transfers actual physical completion from pending to non-refundable consumed work.
    pub fn complete(&mut self, amounts: [u64; N]) -> Result<(), StoreRuntimeError> {
        self.raw.complete(amounts).map_err(map_execution_error)
    }

    /// Resolves a fully completed envelope.
    pub fn finish(self) -> Result<(), StoreRuntimeError> {
        self.raw.finish().map_err(map_execution_error)
    }

    /// Releases only the uncompleted pending suffix after verified partial execution.
    pub fn finish_partial(self) -> Result<(), StoreRuntimeError> {
        self.raw.finish_partial().map_err(map_execution_error)
    }
}

impl StoreExecutionPermit<'_, 1> {
    /// Admits one next ordinary producer unit only after the prior unit completed.
    /// The envelope remains outstanding until explicitly finished, including at zero remaining.
    pub fn replenish_one(&mut self) -> Result<(), StoreRuntimeError> {
        self.raw.replenish_one().map_err(map_execution_error)
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> StoreExecutionServices<'_, O, PP, U> {
    /// Performs a full poll at entry, external, allocation, IO, semantic and terminal boundaries.
    pub fn checkpoint(&mut self) -> Result<(), StoreRuntimeError> {
        self.raw.checkpoint().map_err(map_execution_error)
    }

    /// Polls before the action's conservative control cost would exceed the invocation bound.
    /// Cost affects cancellation cadence only; each real action has its own work admission.
    pub fn before_action(&mut self, max_cost: u16) -> Result<(), StoreRuntimeError> {
        self.raw
            .before_action(max_cost)
            .map_err(map_execution_error)
    }

    /// Advances only the cadence debt after exact physical completion has been recorded.
    pub fn completed_action(&mut self, max_cost: u16) -> Result<(), StoreRuntimeError> {
        self.raw
            .completed_action(max_cost)
            .map_err(map_execution_error)
    }

    /// Reports progress at an explicit full checkpoint.
    pub fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), StoreRuntimeError> {
        self.raw
            .report_progress(phase, unit, completed, total)
            .map_err(map_execution_error)
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> StoreExecutionServices<'_, O, PP, U> {
    /// Verifies a live reservation belongs to this invocation's shared storage owner.
    pub fn validate_live_owner(
        &self,
        reservation: &RetainedStoreReservation<O>,
    ) -> Result<(), StoreRuntimeError> {
        self.raw
            .retained
            .validate_live_owner(&reservation.raw)
            .map_err(map_execution_error)
    }

    /// Moves live attribution while preserving the existing reservation and physical backing.
    pub fn transition(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.validate(phase, dimension)?;
        if reservation
            .physical
            .as_ref()
            .is_some_and(|lease| bytes > lease.admitted_bytes())
        {
            self.raw.retained.reject_binding();
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        self.raw
            .retained
            .transition(&mut reservation.raw, phase, dimension, bytes)
            .map_err(map_execution_error)?;
        if let Some(physical) = &mut reservation.physical {
            physical
                .shrink_to(bytes)
                .map_err(|cause| map_writer_error(&RetainedWriterError::Physical(cause)))?;
        }
        Ok(())
    }

    fn validate(
        &self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
    ) -> Result<(), StoreRuntimeError> {
        if !self.domain.owns_phase(phase) {
            self.raw.retained.reject_binding();
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        if !self.domain.owns_dimension(dimension) {
            self.raw.retained.reject_binding();
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        Ok(())
    }

    /// Polls, then admits physical backing without borrowing the work or control state.
    pub fn reserve_retained(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        self.validate(phase, dimension)?;
        self.checkpoint()?;
        self.raw
            .retained
            .reserve_retained(phase, dimension, bytes, allocations)
            .and_then(|(mut raw, physical)| {
                if let Err(cause) = raw.retain_for_output() {
                    let _ = crate::retained::release_owned_storage(physical, ());
                    return Err(RetainedWriterError::Ledger(cause));
                }
                Ok(RetainedStoreReservation {
                    raw,
                    physical: Some(physical),
                })
            })
            .map_err(|cause| map_writer_error(&cause))
    }

    /// Reconciles actual capacity after allocation, preserving the source on rejection.
    pub fn resize(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        resize_store_reservation(
            &self.raw.retained.storage_view(),
            self.raw.retained.failure(),
            reservation,
            bytes,
        )
    }
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit>
    StoreExecutionServices<'op, O, PP, U>
{
    /// Borrows the already bound ordinary field without polling or rebuilding its descriptors.
    pub fn ordinary(&mut self) -> StoreOrdinaryStorage<'_, 'op, O, PP, U> {
        StoreOrdinaryStorage {
            raw: &mut self.raw.ordinary,
            domain: self.domain,
        }
    }
}

fn resize_store_reservation<O: OperationOwner>(
    storage: &crate::ledger::ExclusiveStorage<'_, O, StorePhaseKey, StoreDimensionKey>,
    failure: &std::cell::Cell<Option<crate::retained::PreparationFailureCode>>,
    reservation: &mut RetainedStoreReservation<O>,
    bytes: u64,
) -> Result<(), StoreRuntimeError> {
    if reservation
        .physical
        .as_ref()
        .is_some_and(|lease| bytes > lease.admitted_bytes())
    {
        crate::retained::record_terminal_cause_with_precedence(
            failure,
            crate::retained::PreparationFailureCode::ledger(),
        );
        return Err(StoreRuntimeError::ConstructionInvariantUnverified);
    }
    storage
        .resize(&mut reservation.raw, bytes)
        .map_err(|cause| {
            crate::retained::record_terminal_cause_with_precedence(
                failure,
                crate::retained::PreparationFailureCode::ledger(),
            );
            StoreRuntimeError::Ledger(cause)
        })?;
    if let Some(physical) = &mut reservation.physical {
        physical
            .shrink_to(bytes)
            .map_err(|cause| map_writer_error(&RetainedWriterError::Physical(cause)))?;
    }
    Ok(())
}

impl<'op, O: OperationOwner, PP: ProgressPhase, U: ProgressUnit>
    StoreOrdinaryStorage<'_, 'op, O, PP, U>
{
    /// Shortens only storage/poll fields for a local compound service.
    pub fn reborrow(&mut self) -> StoreOrdinaryStorage<'_, 'op, O, PP, U> {
        StoreOrdinaryStorage {
            domain: self.domain,
            raw: &mut *self.raw,
        }
    }
    /// Uses this invocation's actual ordinary checkpoint without delivering progress.
    pub fn checkpoint(&mut self) -> Result<(), StoreRuntimeError> {
        self.raw.checkpoint().map_err(map_execution_error)
    }
    /// Ordinary live capacity; no retained physical lease or Internal share work.
    pub fn reserve(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        let foreign = if !self.domain.owns_phase(phase) {
            Some(StoreRuntimeError::ForeignPhaseDomain { phase })
        } else if !self.domain.owns_dimension(dimension) {
            Some(StoreRuntimeError::ForeignDimensionDomain { dimension })
        } else {
            None
        };
        if let Some(cause) = foreign {
            crate::retained::record_terminal_cause_with_precedence(
                self.raw.failure,
                crate::retained::PreparationFailureCode::ledger(),
            );
            return Err(cause);
        }
        self.raw
            .reserve(phase, dimension, bytes)
            .map(RetainedStoreReservation::live)
            .map_err(map_execution_error)
    }
    /// Reconciles real capacity without adding a poll or permitting stopped body execution.
    pub fn resize(
        &self,
        reservation: &mut RetainedStoreReservation<O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.raw.running().map_err(map_execution_error)?;
        resize_store_reservation(&self.raw.storage, self.raw.failure, reservation, bytes)
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit, L> RetainedStoreOperation<O, PP, U, L> {
    /// Lends concrete disjoint execution capabilities until the consuming transition begins.
    #[allow(
        clippy::type_complexity,
        reason = "the tuple carries the same invocation lifetime through distinct capabilities"
    )]
    pub fn execution(
        &mut self,
    ) -> Result<
        (
            StoreExecutionWork<'_, O>,
            StoreExecutionServices<'_, O, PP, U>,
        ),
        StoreRuntimeError,
    > {
        self.raw
            .check_execution_running()
            .map_err(map_execution_error)?;
        if self.raw.is_terminal_only() {
            return Err(StoreRuntimeError::FallbackOnly);
        }
        let (work, services) = self.raw.execution();
        let mut services = StoreExecutionServices {
            raw: services,
            domain: &self.domain,
        };
        services.checkpoint()?;
        Ok((
            StoreExecutionWork {
                raw: work,
                domain: &self.domain,
            },
            services,
        ))
    }
}
