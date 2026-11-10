//! Concrete execution views for trusted import and geometry adapters.

use super::*;
mod exclusive;
use crate::retained::{ExecutionError, ExecutionOrdinaryStorage};
pub use exclusive::{
    StoreActionExecution, StoreChildAdmission, StoreExtensionResult, StoreFailureStorage,
    StoreImporterBoundary, StoreInvocation, StoreInvocationError, StoreInvocationLayout,
    StoreInvocationParser, StoreInvocationServices, StoreParserReadRow, StoreScanReturn,
};
pub(super) use exclusive::{execute_store_action_parts, execute_store_parts};

/// Ordinary storage and real poll authority, borrowing no numeric work or progress state.
pub struct StoreOrdinaryStorage<'b, 'op: 'b, O, PP, U> {
    raw: &'b mut ExecutionOrdinaryStorage<'op, O, StorePhaseKey, PP, U, StoreDimensionKey>,
    domain: &'op StoreKeyDomain,
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
