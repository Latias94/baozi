use std::fmt;

use super::{
    RetainedStoreFailureTerminalWorkPlan, RetainedStoreSharePlan, RetainedStoreWriterPlan,
    StoreDimensionKey, StoreFallbackEnvelope, StoreKeyDomain, StoreLedgerPlanError, StorePhaseKey,
    failure_terminal_budget_matches, validate_store_ledger_catalog,
};
use crate::ledger::finite::FiniteLedgerLimits;
use crate::ledger::{DimensionPolicy, LedgerLimitsError};

/// Failure while freezing the fixed concrete-store vocabulary or dimension policies.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FiniteStoreLedgerPlanError {
    /// Key-domain ownership or canonical phase order is invalid.
    Catalog { cause: StoreLedgerPlanError },
    /// A dimension policy is invalid or a dimension occurs more than once.
    Limits {
        cause: LedgerLimitsError<StoreDimensionKey>,
    },
}

impl fmt::Display for FiniteStoreLedgerPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog { cause } => {
                write!(formatter, "cannot freeze finite store catalog: {cause}")
            }
            Self::Limits { cause } => {
                write!(formatter, "cannot freeze finite store limits: {cause}")
            }
        }
    }
}

impl std::error::Error for FiniteStoreLedgerPlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog { cause } => Some(cause),
            Self::Limits { cause } => Some(cause),
        }
    }
}

impl From<StoreLedgerPlanError> for FiniteStoreLedgerPlanError {
    fn from(cause: StoreLedgerPlanError) -> Self {
        Self::Catalog { cause }
    }
}

impl From<LedgerLimitsError<StoreDimensionKey>> for FiniteStoreLedgerPlanError {
    fn from(cause: LedgerLimitsError<StoreDimensionKey>) -> Self {
        Self::Limits { cause }
    }
}

/// Owned six-dimension, three-phase plan for the closed scene-edit invocation.
///
/// The original domain, fixed limits and phase array move together into the invocation.
/// Construction retains no dynamic policy or phase buffers.
#[doc(hidden)]
pub struct FiniteStoreLedgerPlan {
    domain: StoreKeyDomain,
    limits: FiniteLedgerLimits<StoreDimensionKey>,
    phases: [StorePhaseKey; 3],
}

impl fmt::Debug for FiniteStoreLedgerPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FiniteStoreLedgerPlan")
            .field("dimension_count", &self.dimension_count())
            .field("phases", &self.phases)
            .finish()
    }
}

impl FiniteStoreLedgerPlan {
    /// Freezes six dimension policies and three canonically ordered phases without allocation.
    ///
    /// Policies are validated in input order, checking each policy before its duplicate key,
    /// before the fixed limits sort their dimension catalog. Domain validation then checks
    /// dimensions in that sorted order and all phases in input order. Only after every key belongs
    /// to the domain are adjacent phase pairs checked for duplicates and descending order.
    ///
    /// This is an attempt-once authority transfer. Every failure consumes the domain and burns
    /// its admitted keys for plan construction. A retry must create a fresh domain and re-admit
    /// the complete vocabulary. Byte budgets move unchanged into the actual fixed limits.
    pub fn try_new(
        domain: StoreKeyDomain,
        operation_live_byte_limit: u64,
        evidence_byte_limit: u64,
        policies: [(StoreDimensionKey, DimensionPolicy); 6],
        phases: [StorePhaseKey; 3],
    ) -> Result<Self, FiniteStoreLedgerPlanError> {
        let limits =
            FiniteLedgerLimits::try_new(operation_live_byte_limit, evidence_byte_limit, policies)?;
        validate_store_ledger_catalog(&domain, limits.dimensions().iter().copied(), &phases)?;
        Ok(Self {
            domain,
            limits,
            phases,
        })
    }

    /// Returns the number of frozen ledger dimensions.
    pub fn dimension_count(&self) -> usize {
        self.limits.dimensions().len()
    }

    /// Returns the number of frozen ledger phases.
    pub fn phase_count(&self) -> usize {
        self.phases.len()
    }

    #[cfg(test)]
    pub(in crate::concrete_store) fn contains_phase(&self, phase: StorePhaseKey) -> bool {
        self.owns_phase(phase) && self.phases.binary_search(&phase).is_ok()
    }

    #[cfg(test)]
    pub(in crate::concrete_store) fn contains_dimension(
        &self,
        dimension: StoreDimensionKey,
    ) -> bool {
        self.owns_dimension(dimension) && self.limits.dimension(dimension).is_some()
    }

    #[cfg(test)]
    pub(in crate::concrete_store) fn owns_phase(&self, phase: StorePhaseKey) -> bool {
        self.domain.owns_phase(phase)
    }

    #[cfg(test)]
    pub(in crate::concrete_store) fn owns_dimension(&self, dimension: StoreDimensionKey) -> bool {
        self.domain.owns_dimension(dimension)
    }

    pub(in crate::concrete_store) fn failure_terminal_budget_matches(
        &self,
        plan: &RetainedStoreFailureTerminalWorkPlan,
    ) -> bool {
        failure_terminal_budget_matches(
            self.limits.dimension(plan.dimension()),
            self.limits
                .dimensions()
                .iter()
                .copied()
                .filter(|dimension| *dimension != plan.dimension())
                .map(|dimension| self.limits.dimension(dimension)),
            plan,
        )
    }

    pub(in crate::concrete_store) fn into_parts(
        self,
    ) -> (
        StoreKeyDomain,
        FiniteLedgerLimits<StoreDimensionKey>,
        [StorePhaseKey; 3],
    ) {
        (self.domain, self.limits, self.phases)
    }
}

/// Start policy for a fixed invocation with one real fallback and no numerical evidence part.
#[derive(Debug, PartialEq, Eq)]
pub struct FiniteRetainedStoreStartPlan {
    terminal_writer: RetainedStoreWriterPlan,
    share: RetainedStoreSharePlan,
    fallback: StoreFallbackEnvelope,
    fallback_dimension: StoreDimensionKey,
    fallback_amount: u64,
    failure_terminal_work: RetainedStoreFailureTerminalWorkPlan,
}

impl FiniteRetainedStoreStartPlan {
    /// Declares the original terminal writer, three account shares and fallback commitment.
    pub const fn new(
        terminal_writer: RetainedStoreWriterPlan,
        share: RetainedStoreSharePlan,
        fallback: StoreFallbackEnvelope,
        fallback_dimension: StoreDimensionKey,
        fallback_amount: u64,
        failure_terminal_work: RetainedStoreFailureTerminalWorkPlan,
    ) -> Self {
        Self {
            terminal_writer,
            share,
            fallback,
            fallback_dimension,
            fallback_amount,
            failure_terminal_work,
        }
    }

    pub(in crate::concrete_store) fn validate(
        &self,
        ledger: &FiniteStoreLedgerPlan,
    ) -> Result<(), super::super::RetainedStoreStartError> {
        let phase = self.terminal_writer.phase;
        let dimensions = [
            self.terminal_writer.capacity_dimension,
            self.terminal_writer.attribution_dimension,
            self.share.clone_dimension,
            self.share.retained_handle_dimension,
            self.fallback_dimension,
        ];
        super::super::validate_store_start_keys(
            &ledger.domain,
            &ledger.phases,
            |dimension| ledger.limits.dimension(dimension),
            (phase, dimensions, self.failure_terminal_work.dimension()),
            !dimensions.contains(&self.failure_terminal_work.dimension()),
            || ledger.failure_terminal_budget_matches(&self.failure_terminal_work),
        )
    }

    pub(in crate::concrete_store) fn into_raw(
        self,
    ) -> (
        crate::retained::FiniteRetainedOperationStartPlan<StorePhaseKey, StoreDimensionKey>,
        StoreFallbackEnvelope,
    ) {
        let allocation = crate::retained::RetainedAllocationPlan::new(0, 0)
            .expect("the empty fallback allocation plan is representable");
        (
            crate::retained::FiniteRetainedOperationStartPlan::for_publication(
                self.terminal_writer.into_raw(),
                self.share.into_raw(),
                allocation,
                self.fallback_dimension,
                self.fallback_amount,
            )
            .with_failure_terminal_work(self.failure_terminal_work.into_raw()),
            self.fallback,
        )
    }
}

#[cfg(test)]
mod tests;
