use baozi_operation::{
    CancellationToken, DimensionPolicy, LedgerLimits, LedgerLimitsError, LedgerSnapshot,
    OperationControl, OperationOwner, ProgressPhase, ProgressUnit,
};
use std::fmt;
use std::time::Instant;

/// Compile-time owner marker for post-process operation capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PostProcessOwner;

impl OperationOwner for PostProcessOwner {}

/// Post-process live-capacity ownership phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PostProcessLedgerPhase {
    PlanPreflight,
    TransactionStaging,
    ValidationCommit,
    OutcomeFinalization,
}

impl PostProcessLedgerPhase {
    pub(crate) const ALL: [Self; 4] = [
        Self::PlanPreflight,
        Self::TransactionStaging,
        Self::ValidationCommit,
        Self::OutcomeFinalization,
    ];
}

/// Transient observer phases, intentionally distinct from ledger phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PostProcessProgressPhase {}

impl ProgressPhase for PostProcessProgressPhase {
    fn order(self) -> u32 {
        match self {}
    }

    fn stable_id(self) -> &'static str {
        match self {}
    }
}

/// Stable post-process progress units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PostProcessProgressUnit {}

impl ProgressUnit for PostProcessProgressUnit {
    fn stable_id(self) -> &'static str {
        match self {}
    }
}

/// Post-process operation-ledger dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PostProcessDimension {
    WorkItems,
    StagingBytes,
    CanonicalBytes,
}

pub(crate) type RuntimePostProcessControl =
    OperationControl<PostProcessProgressPhase, PostProcessProgressUnit>;
pub(crate) type RuntimePostProcessLedger =
    LedgerSnapshot<PostProcessLedgerPhase, PostProcessDimension>;

/// A cloneable cancellation token owned by the post-process API.
#[derive(Debug, Clone, Default)]
pub struct PostProcessCancellationToken {
    inner: CancellationToken,
}

impl PostProcessCancellationToken {
    /// Creates an active token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. This transition cannot be reversed.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
}

/// Invocation-local cancellation and deadline control.
#[derive(Default)]
pub struct PostProcessControl {
    inner: RuntimePostProcessControl,
}

impl PostProcessControl {
    /// Installs a post-process-owned cancellation token.
    pub fn with_cancellation(mut self, cancellation: PostProcessCancellationToken) -> Self {
        self.inner = self.inner.with_cancellation(cancellation.inner);
        self
    }

    /// Installs a deadline backstop.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.inner = self.inner.with_deadline(deadline);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_cancellation_signal(
        mut self,
        cancellation: impl baozi_operation::CancellationSignal,
    ) -> Self {
        self.inner = self.inner.with_cancellation(cancellation);
        self
    }

    pub(crate) fn into_runtime(self) -> RuntimePostProcessControl {
        self.inner
    }
}

impl fmt::Debug for PostProcessControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessControl")
            .finish_non_exhaustive()
    }
}

/// Bounded resource evidence retained by one post-process invocation.
///
/// The underlying operation ledger remains private so callers observe stable
/// post-process concepts rather than runtime phase and dimension keys.
#[derive(Debug, PartialEq, Eq)]
pub struct PostProcessResourceEvidence {
    raw: RuntimePostProcessLedger,
}

impl PostProcessResourceEvidence {
    pub(crate) const fn from_runtime(raw: RuntimePostProcessLedger) -> Self {
        Self { raw }
    }

    /// Returns current operation-owned live bytes.
    pub const fn current_live_bytes(&self) -> u64 {
        self.raw.current_live_bytes()
    }

    /// Returns the operation-wide live-byte high-water mark.
    pub const fn peak_live_bytes(&self) -> u64 {
        self.raw.peak_live_bytes()
    }

    /// Returns active linear reservation capabilities at the terminal boundary.
    pub const fn active_reservations(&self) -> u64 {
        self.raw.active_reservations()
    }

    /// Returns cumulative work units admitted by the post-process compiler.
    pub fn attempted_work_items(&self) -> u64 {
        self.dimension(PostProcessDimension::WorkItems).attempted()
    }

    /// Returns cumulative work units completed by the post-process compiler.
    pub fn consumed_work_items(&self) -> u64 {
        self.dimension(PostProcessDimension::WorkItems).consumed()
    }

    /// Returns the peak retained staging capacity in bytes.
    pub fn peak_staging_bytes(&self) -> u64 {
        self.dimension(PostProcessDimension::StagingBytes)
            .peak_live_bytes()
    }

    /// Returns canonical bytes durably committed by this invocation.
    pub fn committed_canonical_bytes(&self) -> u64 {
        self.dimension(PostProcessDimension::CanonicalBytes)
            .committed()
    }

    /// Returns ledger-owned evidence bytes retained at the terminal boundary.
    pub const fn current_evidence_bytes(&self) -> u64 {
        self.raw.current_evidence_bytes()
    }

    /// Returns the evidence-storage high-water mark.
    pub const fn peak_evidence_bytes(&self) -> u64 {
        self.raw.peak_evidence_bytes()
    }

    fn dimension(&self, dimension: PostProcessDimension) -> &baozi_operation::DimensionSnapshot {
        self.raw
            .dimension(dimension)
            .expect("post-process evidence must retain the frozen dimension catalog")
    }
}

pub(crate) fn default_ledger_limits()
-> Result<LedgerLimits<PostProcessDimension>, LedgerLimitsError<PostProcessDimension>> {
    const MAX_WORK_ITEMS: u64 = 100_000_000;
    const MAX_STAGING_BYTES: u64 = 1024 * 1024 * 1024;
    let dimensions = [
        (
            PostProcessDimension::WorkItems,
            DimensionPolicy::counter(MAX_WORK_ITEMS, MAX_WORK_ITEMS, 0),
        ),
        (
            PostProcessDimension::StagingBytes,
            DimensionPolicy::reservable_bytes(
                MAX_STAGING_BYTES,
                MAX_STAGING_BYTES,
                0,
                MAX_STAGING_BYTES,
            ),
        ),
        (
            PostProcessDimension::CanonicalBytes,
            DimensionPolicy::reservable_bytes(
                MAX_STAGING_BYTES,
                MAX_STAGING_BYTES,
                MAX_STAGING_BYTES,
                MAX_STAGING_BYTES,
            ),
        ),
    ];
    LedgerLimits::new(MAX_STAGING_BYTES, dimensions)
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
