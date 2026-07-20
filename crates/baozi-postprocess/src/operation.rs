use baozi_operation::{
    DimensionPolicy, LedgerLimits, LedgerLimitsError, LedgerSnapshot, OperationControl,
    OperationOwner, ProgressPhase, ProgressUnit,
};

/// Compile-time owner marker for post-process operation capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PostProcessOwner;

impl OperationOwner for PostProcessOwner {}

/// Post-process live-capacity ownership phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PostProcessLedgerPhase {
    PlanPreflight,
    TransactionStaging,
    ValidationCommit,
    OutcomeFinalization,
}

impl PostProcessLedgerPhase {
    /// Complete canonical accounting-phase vocabulary for one post-process operation.
    pub const ALL: [Self; 4] = [
        Self::PlanPreflight,
        Self::TransactionStaging,
        Self::ValidationCommit,
        Self::OutcomeFinalization,
    ];
}

/// Transient observer phases, intentionally distinct from ledger phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PostProcessProgressPhase {
    Transform,
    Validate,
}

impl ProgressPhase for PostProcessProgressPhase {
    fn order(self) -> u32 {
        match self {
            Self::Transform => 0,
            Self::Validate => 1,
        }
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Transform => "postprocess.transform",
            Self::Validate => "postprocess.validate",
        }
    }
}

/// Stable post-process progress units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PostProcessProgressUnit {
    WorkItems,
}

impl ProgressUnit for PostProcessProgressUnit {
    fn stable_id(self) -> &'static str {
        match self {
            Self::WorkItems => "work_items",
        }
    }
}

/// Post-process operation-ledger dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PostProcessDimension {
    WorkItems,
    StagingBytes,
    CanonicalBytes,
}

/// Runtime controls accepted by a post-process invocation.
pub type PostProcessControl = OperationControl<PostProcessProgressPhase, PostProcessProgressUnit>;

/// Immutable ledger evidence retained by a post-process failure.
pub type PostProcessLedgerSnapshot = LedgerSnapshot<PostProcessLedgerPhase, PostProcessDimension>;

/// Effective hard limits for one post-process invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostProcessLimits {
    max_work_items: u64,
    max_staging_bytes: u64,
}

impl PostProcessLimits {
    /// Creates post-process hard limits.
    pub const fn new(max_work_items: u64, max_staging_bytes: u64) -> Self {
        Self {
            max_work_items,
            max_staging_bytes,
        }
    }

    /// Returns the cumulative work-item limit.
    pub const fn max_work_items(self) -> u64 {
        self.max_work_items
    }

    /// Returns the peak staging-capacity limit in bytes.
    pub const fn max_staging_bytes(self) -> u64 {
        self.max_staging_bytes
    }

    pub(crate) fn try_ledger_limits(
        self,
    ) -> Result<LedgerLimits<PostProcessDimension>, LedgerLimitsError<PostProcessDimension>> {
        let dimensions = [
            (
                PostProcessDimension::WorkItems,
                DimensionPolicy::counter(self.max_work_items, self.max_work_items, 0),
            ),
            (
                PostProcessDimension::StagingBytes,
                DimensionPolicy::reservable_bytes(
                    self.max_staging_bytes,
                    self.max_staging_bytes,
                    0,
                    self.max_staging_bytes,
                ),
            ),
            (
                PostProcessDimension::CanonicalBytes,
                DimensionPolicy::reservable_bytes(
                    self.max_staging_bytes,
                    self.max_staging_bytes,
                    self.max_staging_bytes,
                    self.max_staging_bytes,
                ),
            ),
        ];
        LedgerLimits::new(self.max_staging_bytes, dimensions)
    }
}

impl Default for PostProcessLimits {
    fn default() -> Self {
        Self::new(100_000_000, 1024 * 1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    #[test]
    fn postprocess_ledger_phase_catalog_is_complete_unique_and_canonical() {
        let expected = [
            PostProcessLedgerPhase::PlanPreflight,
            PostProcessLedgerPhase::TransactionStaging,
            PostProcessLedgerPhase::ValidationCommit,
            PostProcessLedgerPhase::OutcomeFinalization,
        ];

        assert_eq!(PostProcessLedgerPhase::ALL, expected);
        for (index, phase) in PostProcessLedgerPhase::ALL.iter().enumerate() {
            assert!(
                !PostProcessLedgerPhase::ALL[..index].contains(phase),
                "post-process ledger phase catalog contains a duplicate: {phase:?}"
            );
        }
        assert!(
            PostProcessLedgerPhase::ALL
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "post-process ledger phase catalog must follow canonical Ord order"
        );
    }

    #[test]
    fn postprocess_ledger_and_progress_phase_vocabularies_are_distinct_types() {
        assert_ne!(
            TypeId::of::<PostProcessLedgerPhase>(),
            TypeId::of::<PostProcessProgressPhase>()
        );
    }
}
