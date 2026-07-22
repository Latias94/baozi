use crate::{CapacityPlan, CapacityPlanError, OperationOwner, ReplacementCapacityPlan};
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::{Arc, Mutex, MutexGuard};

mod reservation;
mod snapshot;
mod startup;

pub use reservation::LiveReservation;
use reservation::ReservationCore;
pub(crate) use reservation::{
    AttributionBatchPlan, AttributionReservationBatch, FallbackBaselineRejection,
    FallbackBatchBaseline, OperationAttributionReservation, StorageCoveringReservation,
};

pub(crate) use snapshot::DimensionSnapshotScalars;
pub use snapshot::{DimensionSnapshot, LedgerSnapshot, PhaseSnapshot};
use snapshot::{
    dimension_snapshot, phase_catalog_shape_is_valid, phase_snapshot, take_final_snapshot,
    take_typed_final_snapshot, try_snapshot, typed_terminal_snapshot_positions_are_valid,
    validate_final_snapshot_shape,
};

#[allow(
    unused_imports,
    reason = "preserve the crate-private ledger path across the startup module extraction"
)]
pub(crate) use startup::{
    StartupAccountShareEnvelope, StartupAccountShareStep, StartupAttributionBatchPlans,
    StartupAttributionGraph, StartupLedgerPlan, StartupLedgerPlanParts, StartupRetainedAttribution,
};

#[cfg(test)]
pub(crate) use startup::{
    test_startup_graph_allocation_entries, test_startup_graph_ledger_release_locks,
    test_startup_graph_storage_releases,
};

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
static EVIDENCE_ALLOCATION_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn test_evidence_allocation_guard() -> MutexGuard<'static, ()> {
    EVIDENCE_ALLOCATION_TEST_LOCK
        .lock()
        .expect("ledger evidence allocation test lock")
}

#[cfg(test)]
pub(crate) fn test_fail_next_evidence_allocation() {
    test_set_evidence_allocation_failure(true);
}

#[cfg(test)]
std::thread_local! {
    static FAIL_NEXT_EVIDENCE_ALLOCATION: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static FAIL_NEXT_EVIDENCE_OVER_ENVELOPE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static ATTRIBUTION_BATCH_ALLOCATION_ENTRIES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static PHASE_CATALOG_CONSTRUCTION_ENTRIES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
fn test_set_evidence_allocation_failure(armed: bool) {
    FAIL_NEXT_EVIDENCE_ALLOCATION.with(|failpoint| failpoint.set(armed));
}

#[cfg(test)]
fn test_evidence_allocation_failure_is_armed() -> bool {
    FAIL_NEXT_EVIDENCE_ALLOCATION.with(std::cell::Cell::get)
}

#[cfg(test)]
fn test_take_evidence_allocation_failure() -> bool {
    FAIL_NEXT_EVIDENCE_ALLOCATION.with(|failpoint| failpoint.replace(false))
}

#[cfg(test)]
fn test_fail_next_evidence_over_envelope() {
    FAIL_NEXT_EVIDENCE_OVER_ENVELOPE.with(|failpoint| failpoint.set(true));
}

#[cfg(test)]
fn test_take_evidence_over_envelope_failure() -> bool {
    FAIL_NEXT_EVIDENCE_OVER_ENVELOPE.with(|failpoint| failpoint.replace(false))
}

#[cfg(test)]
pub(crate) fn test_attribution_batch_allocation_entries() -> usize {
    ATTRIBUTION_BATCH_ALLOCATION_ENTRIES.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn test_phase_catalog_construction_entries() -> usize {
    PHASE_CATALOG_CONSTRUCTION_ENTRIES.with(std::cell::Cell::get)
}

/// Default hard limit for ledger-owned evidence indexes and staging storage.
pub const DEFAULT_LEDGER_EVIDENCE_BYTE_LIMIT: u64 = 1024 * 1024;

/// Identifies the internal evidence storage that could not be admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LedgerEvidencePurpose {
    /// Frozen dimension policy and final dimension evidence.
    Dimensions,
    /// Phase accounting and final phase evidence.
    Phases,
    /// An immutable non-final evidence capture.
    Snapshot,
    /// A sorted pending-publication commitment batch.
    Publication,
    /// Retained semantic progress state.
    Progress,
}

/// A bounded ledger evidence allocation could not be completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LedgerEvidenceError {
    /// The phase catalog no longer maps its append-only state positions exactly once.
    PhaseCatalogShapeMismatch,
    /// A tracked storage owner no longer matches the exact capacity bytes it must release.
    TrackedStorageCoverageMismatch {
        purpose: LedgerEvidencePurpose,
        retained_bytes: u64,
        capacity_bytes: u64,
    },
    /// Checked capacity planning rejected the requested storage.
    Capacity {
        purpose: LedgerEvidencePurpose,
        cause: CapacityPlanError,
    },
    /// The deterministic evidence envelope exceeded its independent hard limit.
    LimitExceeded {
        purpose: LedgerEvidencePurpose,
        value: u64,
        limit: u64,
    },
    /// The allocator rejected an already-admitted deterministic envelope.
    AllocationFailed {
        purpose: LedgerEvidencePurpose,
        requested_bytes: u64,
    },
}

impl fmt::Display for LedgerEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PhaseCatalogShapeMismatch => {
                formatter.write_str("ledger phase catalog shape is inconsistent")
            }
            Self::TrackedStorageCoverageMismatch {
                purpose,
                retained_bytes,
                capacity_bytes,
            } => write!(
                formatter,
                "{purpose:?} ledger evidence owns {capacity_bytes} capacity bytes but its retained owner records {retained_bytes} bytes"
            ),
            Self::Capacity { purpose, cause } => {
                write!(
                    formatter,
                    "{purpose:?} ledger evidence capacity failed: {cause}"
                )
            }
            Self::LimitExceeded {
                purpose,
                value,
                limit,
            } => write!(
                formatter,
                "{purpose:?} ledger evidence reached {value} bytes, above limit {limit}"
            ),
            Self::AllocationFailed {
                purpose,
                requested_bytes,
            } => write!(
                formatter,
                "allocator rejected {requested_bytes} admitted bytes for {purpose:?} ledger evidence"
            ),
        }
    }
}

impl std::error::Error for LedgerEvidenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Capacity { cause, .. } => Some(cause),
            Self::PhaseCatalogShapeMismatch
            | Self::TrackedStorageCoverageMismatch { .. }
            | Self::LimitExceeded { .. }
            | Self::AllocationFailed { .. } => None,
        }
    }
}

/// Identifies one accounting evidence class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CounterClass {
    /// Work requested before an action begins.
    Attempted,
    /// Work actually completed or observed.
    Consumed,
    /// Byte capacity retained by an active reservation.
    Live,
    /// Output waiting for atomic publication.
    PendingPublication,
    /// Canonical output published durably from the operation.
    Committed,
    /// Non-canonical outcome evidence published durably from the operation.
    PublishedOutcomeEvidence,
}

/// Value policy for one owner-defined accounting dimension.
///
/// A ledger freezes this value at creation and may only replace it with a monotonically tighter
/// policy through its existing state owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionPolicy {
    attempted_limit: u64,
    consumed_limit: u64,
    committed_limit: u64,
    published_outcome_evidence_limit: u64,
    live_byte_limit: Option<u64>,
}

impl DimensionPolicy {
    /// Defines a counter-only dimension that cannot reserve byte capacity.
    pub const fn counter(attempted_limit: u64, consumed_limit: u64, committed_limit: u64) -> Self {
        Self {
            attempted_limit,
            consumed_limit,
            committed_limit,
            published_outcome_evidence_limit: 0,
            live_byte_limit: None,
        }
    }

    /// Defines a byte-capacity dimension that may also carry counters and commitments.
    pub const fn reservable_bytes(
        attempted_limit: u64,
        consumed_limit: u64,
        committed_limit: u64,
        live_byte_limit: u64,
    ) -> Self {
        Self {
            attempted_limit,
            consumed_limit,
            committed_limit,
            published_outcome_evidence_limit: 0,
            live_byte_limit: Some(live_byte_limit),
        }
    }

    /// Adds an independent limit for retained published outcome evidence.
    pub const fn with_published_outcome_evidence_limit(mut self, limit: u64) -> Self {
        self.published_outcome_evidence_limit = limit;
        self
    }

    /// Returns the immutable attempted-work limit.
    pub const fn attempted_limit(self) -> u64 {
        self.attempted_limit
    }

    /// Returns the immutable consumed-work limit.
    pub const fn consumed_limit(self) -> u64 {
        self.consumed_limit
    }

    /// Returns the immutable committed-output limit.
    pub const fn committed_limit(self) -> u64 {
        self.committed_limit
    }

    /// Returns the immutable published-outcome-evidence limit.
    pub const fn published_outcome_evidence_limit(self) -> u64 {
        self.published_outcome_evidence_limit
    }

    /// Returns the live-byte limit, or `None` when the dimension is not reservable.
    pub const fn live_byte_limit(self) -> Option<u64> {
        self.live_byte_limit
    }

    fn tighten_with(self, upper_bound: Self) -> Option<Self> {
        let live_byte_limit = match (self.live_byte_limit, upper_bound.live_byte_limit) {
            (None, None) => None,
            (Some(current), Some(upper_bound)) => Some(current.min(upper_bound)),
            (None, Some(_)) | (Some(_), None) => return None,
        };
        let attempted_limit = self.attempted_limit.min(upper_bound.attempted_limit);
        Some(Self {
            attempted_limit,
            consumed_limit: self
                .consumed_limit
                .min(upper_bound.consumed_limit)
                .min(attempted_limit),
            committed_limit: self.committed_limit.min(upper_bound.committed_limit),
            published_outcome_evidence_limit: self
                .published_outcome_evidence_limit
                .min(upper_bound.published_outcome_evidence_limit),
            live_byte_limit,
        })
    }
}

/// A failure while freezing central ledger policy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LedgerLimitsError<D> {
    /// The same owner-defined dimension appeared more than once.
    DuplicateDimension { dimension: D },
    /// The consumed limit cannot exceed the corresponding attempted limit.
    ConsumedLimitExceedsAttempted {
        dimension: D,
        consumed_limit: u64,
        attempted_limit: u64,
    },
    /// Frozen policy evidence could not be stored within its independent bound.
    Evidence { cause: LedgerEvidenceError },
}

impl<D: fmt::Debug> fmt::Display for LedgerLimitsError<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateDimension { dimension } => {
                write!(formatter, "duplicate ledger dimension {dimension:?}")
            }
            Self::ConsumedLimitExceedsAttempted {
                dimension,
                consumed_limit,
                attempted_limit,
            } => write!(
                formatter,
                "consumed limit {consumed_limit} exceeds attempted limit {attempted_limit} for {dimension:?}"
            ),
            Self::Evidence { cause } => write!(formatter, "cannot freeze ledger limits: {cause}"),
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for LedgerLimitsError<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Evidence { cause } => Some(cause),
            Self::DuplicateDimension { .. } | Self::ConsumedLimitExceedsAttempted { .. } => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct DimensionEntry<D> {
    dimension: D,
    policy: DimensionPolicy,
    state: DimensionState,
}

/// Frozen aggregate limits for one ledger instance.
#[derive(Debug, PartialEq, Eq)]
pub struct LedgerLimits<D> {
    dimensions: Vec<DimensionEntry<D>>,
    dimension_snapshot_staging: Vec<(D, DimensionSnapshot)>,
    operation_live_byte_limit: u64,
    evidence_byte_limit: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
}

impl<D> LedgerLimits<D>
where
    D: Copy + Ord + fmt::Debug,
{
    /// Validates and freezes central dimension policy and the operation-wide live-byte limit.
    pub fn new(
        operation_live_byte_limit: u64,
        dimensions: impl IntoIterator<Item = (D, DimensionPolicy)>,
    ) -> Result<Self, LedgerLimitsError<D>> {
        Self::with_evidence_byte_limit(
            operation_live_byte_limit,
            DEFAULT_LEDGER_EVIDENCE_BYTE_LIMIT,
            dimensions,
        )
    }

    /// Validates policy with an explicit independent bound for ledger evidence storage.
    pub fn with_evidence_byte_limit(
        operation_live_byte_limit: u64,
        evidence_byte_limit: u64,
        dimensions: impl IntoIterator<Item = (D, DimensionPolicy)>,
    ) -> Result<Self, LedgerLimitsError<D>> {
        let mut frozen: Vec<DimensionEntry<D>> = Vec::new();
        let mut staging: Vec<(D, DimensionSnapshot)> = Vec::new();
        let mut current_evidence_bytes = 0;
        let mut peak_evidence_bytes = 0;
        for (dimension, policy) in dimensions {
            if policy.consumed_limit > policy.attempted_limit {
                return Err(LedgerLimitsError::ConsumedLimitExceedsAttempted {
                    dimension,
                    consumed_limit: policy.consumed_limit,
                    attempted_limit: policy.attempted_limit,
                });
            }
            let position = match frozen.binary_search_by_key(&dimension, |entry| entry.dimension) {
                Ok(_) => {
                    return Err(LedgerLimitsError::DuplicateDimension { dimension });
                }
                Err(position) => position,
            };
            let desired_dimension_capacity = frozen.len().saturating_add(1);
            try_grow_evidence_vec(
                &mut frozen,
                desired_dimension_capacity,
                &mut current_evidence_bytes,
                &mut peak_evidence_bytes,
                evidence_byte_limit,
                LedgerEvidencePurpose::Dimensions,
            )
            .map_err(|cause| LedgerLimitsError::Evidence { cause })?;
            let desired_staging_capacity = staging.len().saturating_add(1);
            try_grow_evidence_vec(
                &mut staging,
                desired_staging_capacity,
                &mut current_evidence_bytes,
                &mut peak_evidence_bytes,
                evidence_byte_limit,
                LedgerEvidencePurpose::Dimensions,
            )
            .map_err(|cause| LedgerLimitsError::Evidence { cause })?;
            frozen.insert(
                position,
                DimensionEntry {
                    dimension,
                    policy,
                    state: DimensionState::default(),
                },
            );
            staging.insert(position, (dimension, DimensionSnapshot::default()));
        }
        Ok(Self {
            dimensions: frozen,
            dimension_snapshot_staging: staging,
            operation_live_byte_limit,
            evidence_byte_limit,
            current_evidence_bytes,
            peak_evidence_bytes,
        })
    }

    /// Returns one frozen dimension policy.
    pub fn dimension(&self, dimension: D) -> Option<DimensionPolicy> {
        dimension_entry(&self.dimensions, dimension).map(|entry| entry.policy)
    }

    pub(crate) fn dimension_count(&self) -> usize {
        self.dimensions.len()
    }

    pub(crate) fn dimensions(&self) -> impl ExactSizeIterator<Item = D> + '_ {
        self.dimensions.iter().map(|entry| entry.dimension)
    }

    /// Returns the frozen operation-wide live-byte limit.
    pub const fn operation_live_byte_limit(&self) -> u64 {
        self.operation_live_byte_limit
    }

    /// Returns the independent hard limit for ledger evidence storage.
    pub const fn evidence_byte_limit(&self) -> u64 {
        self.evidence_byte_limit
    }

    /// Returns evidence bytes retained by the frozen policy and final staging buffers.
    pub const fn frozen_evidence_bytes(&self) -> u64 {
        self.current_evidence_bytes
    }

    /// Tightens the independent evidence bound without reallocating frozen policy.
    pub fn try_with_evidence_byte_limit(
        mut self,
        evidence_byte_limit: u64,
    ) -> Result<Self, LedgerLimitsError<D>> {
        if self.current_evidence_bytes > evidence_byte_limit {
            return Err(LedgerLimitsError::Evidence {
                cause: LedgerEvidenceError::LimitExceeded {
                    purpose: LedgerEvidencePurpose::Dimensions,
                    value: self.current_evidence_bytes,
                    limit: evidence_byte_limit,
                },
            });
        }
        self.evidence_byte_limit = evidence_byte_limit;
        Ok(self)
    }
}

/// An owner-neutral checked-ledger cause.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LedgerCause<D> {
    /// The requested dimension is absent from the frozen policy.
    UnknownDimension { dimension: D },
    /// The requested phase is absent from a frozen phase catalog.
    UnknownPhase,
    /// The complete phase catalog declared the same phase more than once.
    DuplicatePhase,
    /// A policy update attempted to change a dimension between counter and reservable shapes.
    DimensionPolicyShapeMismatch { dimension: D },
    /// The dimension is a counter and cannot reserve byte capacity.
    DimensionNotReservable { dimension: D },
    /// Checked arithmetic overflowed without mutating the prior snapshot.
    ArithmeticOverflow { class: CounterClass, dimension: D },
    /// A checked counter crossed its frozen effective bound.
    LimitExceeded {
        class: CounterClass,
        dimension: D,
        value: u64,
        limit: u64,
    },
    /// Consumed work exceeded previously attempted work.
    ConsumedExceedsAttempted {
        dimension: D,
        consumed: u64,
        attempted: u64,
    },
    /// An atomic consumed batch was not sorted by dimension.
    ConsumedBatchOutOfOrder { previous: D, dimension: D },
    /// A reservation was presented to a different ledger instance.
    OwnerMismatch,
    /// A consumed or otherwise inactive reservation was reused.
    InactiveReservation,
    /// A split requested more bytes than the reservation owns.
    SplitExceedsReservation { requested: u64, available: u64 },
    /// Another atomic publication transaction is already pending.
    PublicationAlreadyPending,
    /// This ledger already published its one durable output batch.
    PublicationAlreadyCompleted,
    /// A consumed or otherwise inactive publication transaction was reused.
    InactivePublication,
    /// A pending transaction did not take ownership of every operation-owned live byte.
    PublicationLiveMismatch {
        transaction_bytes: u64,
        ledger_bytes: u64,
    },
    /// Final evidence was requested while byte capacity remained operation-owned.
    LiveReservationsRemain { bytes: u64 },
    /// Final evidence was requested while zero-byte reservation capabilities remained active.
    ActiveReservationsRemain { count: u64 },
    /// Final evidence was requested while a publication transaction remained pending.
    PendingPublicationsRemain,
    /// Internal evidence storage could not be admitted or allocated safely.
    Evidence { cause: LedgerEvidenceError },
    /// Final snapshot staging no longer matches the frozen dimension and phase indexes.
    FinalSnapshotShapeMismatch,
}

/// Failure while recording semantic completion against a frozen phase catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhaseCompletionError<P> {
    /// A reversible publication transaction currently seals semantic completion.
    PublicationPending,
    /// The ledger already published its durable output and is permanently sealed.
    PublicationCompleted,
    /// Dynamic accounting phases cannot establish a stable semantic completion order.
    /// The requested phase is absent from the frozen catalog.
    UnknownPhase { phase: P },
    /// The requested phase precedes the last completed catalog position.
    Regression { last_completed_phase: P, phase: P },
    /// The frozen lookup catalog no longer maps its append-only state positions exactly once.
    CatalogShapeMismatch,
}

impl<D: fmt::Debug> fmt::Display for LedgerCause<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDimension { dimension } => {
                write!(formatter, "unknown ledger dimension {dimension:?}")
            }
            Self::UnknownPhase => formatter.write_str("unknown ledger phase"),
            Self::DuplicatePhase => formatter.write_str("duplicate ledger phase"),
            Self::DimensionPolicyShapeMismatch { dimension } => write!(
                formatter,
                "ledger dimension {dimension:?} cannot change between counter and reservable policy shapes"
            ),
            Self::DimensionNotReservable { dimension } => {
                write!(
                    formatter,
                    "ledger dimension {dimension:?} cannot reserve bytes"
                )
            }
            Self::ArithmeticOverflow { class, dimension } => {
                write!(formatter, "{class:?} counter overflowed for {dimension:?}")
            }
            Self::LimitExceeded {
                class,
                dimension,
                value,
                limit,
            } => write!(
                formatter,
                "{class:?} counter for {dimension:?} reached {value}, above limit {limit}"
            ),
            Self::ConsumedExceedsAttempted {
                dimension,
                consumed,
                attempted,
            } => write!(
                formatter,
                "consumed {consumed} exceeds attempted {attempted} for {dimension:?}"
            ),
            Self::ConsumedBatchOutOfOrder {
                previous,
                dimension,
            } => write!(
                formatter,
                "consumed batch dimension {dimension:?} follows later dimension {previous:?}"
            ),
            Self::OwnerMismatch => formatter.write_str("reservation belongs to another ledger"),
            Self::InactiveReservation => formatter.write_str("reservation is no longer active"),
            Self::SplitExceedsReservation {
                requested,
                available,
            } => write!(
                formatter,
                "cannot split {requested} bytes from a {available}-byte reservation"
            ),
            Self::PublicationAlreadyPending => {
                formatter.write_str("a publication transaction is already pending")
            }
            Self::PublicationAlreadyCompleted => {
                formatter.write_str("the ledger already published its durable output")
            }
            Self::InactivePublication => {
                formatter.write_str("publication transaction is no longer active")
            }
            Self::PublicationLiveMismatch {
                transaction_bytes,
                ledger_bytes,
            } => write!(
                formatter,
                "publication owns {transaction_bytes} live bytes but the ledger owns {ledger_bytes}"
            ),
            Self::LiveReservationsRemain { bytes } => {
                write!(formatter, "{bytes} live bytes remain reserved")
            }
            Self::ActiveReservationsRemain { count } => {
                write!(
                    formatter,
                    "{count} live reservation capabilities remain active"
                )
            }
            Self::PendingPublicationsRemain => {
                formatter.write_str("publication transactions remain pending")
            }
            Self::Evidence { cause } => write!(formatter, "ledger evidence failed: {cause}"),
            Self::FinalSnapshotShapeMismatch => {
                formatter.write_str("final snapshot staging does not match ledger indexes")
            }
        }
    }
}

impl<D: fmt::Debug + 'static> std::error::Error for LedgerCause<D> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Evidence { cause } => Some(cause),
            _ => None,
        }
    }
}

/// A finalization invariant failure that preserves the ledger's exact last snapshot.
#[derive(Debug, PartialEq, Eq)]
pub struct LedgerFinishError<P, D> {
    cause: LedgerCause<D>,
    snapshot: LedgerSnapshot<P, D>,
}

impl<P, D> LedgerFinishError<P, D> {
    /// Returns the invariant that prevented clean finalization.
    pub const fn cause(&self) -> &LedgerCause<D> {
        &self.cause
    }

    /// Returns the exact non-normalized evidence observed at finalization.
    pub const fn snapshot(&self) -> &LedgerSnapshot<P, D> {
        &self.snapshot
    }

    /// Separates the invariant cause from its exact evidence for outcome composition.
    pub fn into_parts(self) -> (LedgerCause<D>, LedgerSnapshot<P, D>) {
        (self.cause, self.snapshot)
    }
}

impl<P, D: fmt::Debug> fmt::Display for LedgerFinishError<P, D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "ledger finalization invariant failed: {}",
            self.cause
        )
    }
}

impl<P, D> std::error::Error for LedgerFinishError<P, D>
where
    P: fmt::Debug + 'static,
    D: fmt::Debug + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// One owner-defined durable output amount proposed for atomic publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Commitment<D> {
    dimension: D,
    amount: u64,
}

impl<D: Copy> Commitment<D> {
    /// Creates a commitment for an owner-defined byte or count dimension.
    pub const fn new(dimension: D, amount: u64) -> Self {
        Self { dimension, amount }
    }

    /// Returns the owner-defined dimension.
    pub const fn dimension(self) -> D {
        self.dimension
    }

    /// Returns the durable output amount.
    pub const fn amount(self) -> u64 {
        self.amount
    }
}

mod destination_sealed {
    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub trait Sealed {}
}

/// Destination marker for committed canonical output.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalOutput;

/// Destination marker for frozen non-canonical outcome evidence.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutcomeEvidenceDestination;

impl destination_sealed::Sealed for CanonicalOutput {}
impl destination_sealed::Sealed for OutcomeEvidenceDestination {}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DestinationKind {
    CanonicalOutput,
    OutcomeEvidence,
}

/// Sealed vocabulary for one retained publication destination.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub trait PublicationDestination: destination_sealed::Sealed + Send + Sync + 'static {}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) trait DestinationState: PublicationDestination {
    const KIND: DestinationKind;
}

impl PublicationDestination for CanonicalOutput {}

impl DestinationState for CanonicalOutput {
    const KIND: DestinationKind = DestinationKind::CanonicalOutput;
}

impl PublicationDestination for OutcomeEvidenceDestination {}

impl DestinationState for OutcomeEvidenceDestination {
    const KIND: DestinationKind = DestinationKind::OutcomeEvidence;
}

/// One typed logical commitment independent of physical allocation attribution.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputCommitment<D, Destination> {
    dimension: D,
    amount: u64,
    dimension_position: usize,
    destination: PhantomData<fn() -> Destination>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<D: Copy, Destination: PublicationDestination> OutputCommitment<D, Destination> {
    /// Creates one logical commitment for the destination in the type parameter.
    pub const fn new(dimension: D, amount: u64) -> Self {
        Self {
            dimension,
            amount,
            dimension_position: usize::MAX,
            destination: PhantomData,
        }
    }

    /// Returns the owner-defined dimension.
    pub const fn dimension(self) -> D {
        self.dimension
    }

    /// Returns the logical amount.
    pub const fn amount(self) -> u64 {
        self.amount
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DimensionState {
    attempted: u64,
    consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct PhaseState {
    current_live_bytes: u64,
    peak_live_bytes: u64,
}

#[derive(Debug)]
struct PhaseEntry<P> {
    phase: P,
    // The caller-declared lifecycle position is also the immutable accounting slot.
    state_position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompletedPhase<P> {
    phase: P,
    state_position: usize,
}

#[derive(Debug)]
struct LedgerState<P, D> {
    dimensions: Vec<DimensionEntry<D>>,
    phases: Vec<PhaseEntry<P>>,
    phase_states: Vec<PhaseState>,
    dimension_snapshot_staging: Vec<(D, DimensionSnapshot)>,
    phase_snapshot_staging: Vec<(P, PhaseSnapshot)>,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    evidence_byte_limit: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
    publication: PublicationState,
    last_completed_phase: Option<CompletedPhase<P>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationState {
    Open,
    Pending,
    TerminalQualified,
    Published,
}

/// Shared accounting state branded by one operation domain.
pub(crate) struct Ledger<O, P, D> {
    state: Arc<Mutex<LedgerState<P, D>>>,
    operation_live_byte_limit: u64,
    operation_owner: PhantomData<fn() -> O>,
}

/// Test-only scalar observation of one ledger's live reservation state.
#[cfg(test)]
pub(crate) struct LedgerTestProbe<P, D> {
    state: Arc<Mutex<LedgerState<P, D>>>,
}

#[cfg(test)]
impl<P, D> LedgerTestProbe<P, D> {
    pub(crate) fn active_reservations(&self) -> u64 {
        lock(&self.state).active_reservations
    }

    pub(crate) fn current_live_bytes(&self) -> u64 {
        lock(&self.state).current_live_bytes
    }

    pub(crate) fn is_terminal_qualified(&self) -> bool {
        lock(&self.state).publication == PublicationState::TerminalQualified
    }
}

impl<O, P, D> fmt::Debug for Ledger<O, P, D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Ledger").finish_non_exhaustive()
    }
}

impl<O, P, D> Ledger<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    /// Creates a ledger with one complete, immutable phase vocabulary.
    pub(crate) fn try_new(limits: LedgerLimits<D>, phases: &[P]) -> Result<Self, LedgerCause<D>> {
        #[cfg(test)]
        PHASE_CATALOG_CONSTRUCTION_ENTRIES.with(|entries| {
            entries.set(entries.get().saturating_add(1));
        });
        let LedgerLimits {
            dimensions,
            dimension_snapshot_staging,
            operation_live_byte_limit,
            evidence_byte_limit,
            current_evidence_bytes,
            peak_evidence_bytes,
        } = limits;
        let mut state = LedgerState {
            dimensions,
            phases: Vec::new(),
            phase_states: Vec::new(),
            dimension_snapshot_staging,
            phase_snapshot_staging: Vec::new(),
            current_live_bytes: 0,
            peak_live_bytes: 0,
            active_reservations: 0,
            evidence_byte_limit,
            current_evidence_bytes,
            peak_evidence_bytes,
            publication: PublicationState::Open,
            last_completed_phase: None,
        };
        for phase in phases {
            try_insert_phase_catalog_entry(&mut state, *phase)?;
        }
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            operation_live_byte_limit,
            operation_owner: PhantomData,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_probe(&self) -> LedgerTestProbe<P, D> {
        LedgerTestProbe {
            state: Arc::clone(&self.state),
        }
    }

    /// Charges non-refundable attempted work before an action.
    pub fn charge_attempted(&self, dimension: D, amount: u64) -> Result<(), LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        Self::charge_attempted_open(&mut state, dimension, amount)
    }

    fn charge_attempted_open(
        state: &mut LedgerState<P, D>,
        dimension: D,
        amount: u64,
    ) -> Result<(), LedgerCause<D>> {
        let entry = dimension_entry_mut(&mut state.dimensions, dimension)
            .ok_or(LedgerCause::UnknownDimension { dimension })?;
        let policy = entry.policy;
        let value =
            entry
                .state
                .attempted
                .checked_add(amount)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Attempted,
                    dimension,
                })?;
        entry.state.attempted = value;
        if value > policy.attempted_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension,
                value,
                limit: policy.attempted_limit,
            });
        }
        Ok(())
    }

    /// Monotonically tightens one existing dimension policy without allocating or replacing state.
    ///
    /// Numerical relaxation requests are harmless: every field remains bounded by its current
    /// value. Counter and reservable shapes cannot be changed. The tighter policy is retained even
    /// when existing non-refundable evidence already exceeds it; in that case this returns the
    /// corresponding typed limit failure without rewriting any evidence.
    pub fn tighten_dimension_policy(
        &self,
        dimension: D,
        upper_bound: DimensionPolicy,
    ) -> Result<(), LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        let entry = dimension_entry_mut(&mut state.dimensions, dimension)
            .ok_or(LedgerCause::UnknownDimension { dimension })?;
        let policy = entry
            .policy
            .tighten_with(upper_bound)
            .ok_or(LedgerCause::DimensionPolicyShapeMismatch { dimension })?;
        entry.policy = policy;
        validate_dimension_state(dimension, policy, entry.state)
    }

    /// Charges non-refundable work that was actually completed or observed.
    pub fn charge_consumed(&self, dimension: D, amount: u64) -> Result<(), LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        Self::charge_consumed_open(&mut state, dimension, amount)
    }

    fn charge_consumed_open(
        state: &mut LedgerState<P, D>,
        dimension: D,
        amount: u64,
    ) -> Result<(), LedgerCause<D>> {
        let entry = dimension_entry_mut(&mut state.dimensions, dimension)
            .ok_or(LedgerCause::UnknownDimension { dimension })?;
        let policy = entry.policy;
        let value =
            entry
                .state
                .consumed
                .checked_add(amount)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Consumed,
                    dimension,
                })?;
        if value > entry.state.attempted {
            return Err(LedgerCause::ConsumedExceedsAttempted {
                dimension,
                consumed: value,
                attempted: entry.state.attempted,
            });
        }
        entry.state.consumed = value;
        if value > policy.consumed_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension,
                value,
                limit: policy.consumed_limit,
            });
        }
        Ok(())
    }

    /// Charges ordered attempted-and-consumed pairs under one ledger lock.
    ///
    /// This preserves the non-refundable semantics of sequential calls: a failure retains every
    /// completed prefix and the failing counter exactly as observed.
    pub fn charge_observed_batch(&self, charges: &[(D, u64)]) -> Result<(), LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        for &(dimension, amount) in charges {
            Self::charge_attempted_open(&mut state, dimension, amount)?;
            Self::charge_consumed_open(&mut state, dimension, amount)?;
        }
        Ok(())
    }

    /// Atomically charges a batch of completed work without allocating.
    ///
    /// Dimensions must be non-decreasing; adjacent repetitions are aggregated before validation.
    /// Any ordering, dimension, arithmetic, attempted-work, or limit failure leaves every consumed
    /// counter unchanged.
    pub fn charge_consumed_batch<const N: usize>(
        &self,
        charges: &[(D, u64); N],
    ) -> Result<(), LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;

        let mut positions = [0_usize; N];
        for (index, &(dimension, _)) in charges.iter().enumerate() {
            let position = state
                .dimensions
                .binary_search_by_key(&dimension, |entry| entry.dimension)
                .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
            if index > 0 && position < positions[index - 1] {
                return Err(LedgerCause::ConsumedBatchOutOfOrder {
                    previous: charges[index - 1].0,
                    dimension,
                });
            }
            positions[index] = position;
        }

        let mut projected = [0_u64; N];
        let mut index = 0;
        while index < N {
            let position = positions[index];
            let dimension = charges[index].0;
            let mut next = index;
            let mut amount = 0_u64;
            while next < N && positions[next] == position {
                amount =
                    amount
                        .checked_add(charges[next].1)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Consumed,
                            dimension,
                        })?;
                next += 1;
            }
            let entry = &state.dimensions[position];
            let value = entry.state.consumed.checked_add(amount).ok_or(
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Consumed,
                    dimension,
                },
            )?;
            if value > entry.state.attempted {
                return Err(LedgerCause::ConsumedExceedsAttempted {
                    dimension,
                    consumed: value,
                    attempted: entry.state.attempted,
                });
            }
            if value > entry.policy.consumed_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Consumed,
                    dimension,
                    value,
                    limit: entry.policy.consumed_limit,
                });
            }
            projected[index] = value;
            index = next;
        }

        index = 0;
        while index < N {
            let position = positions[index];
            state.dimensions[position].state.consumed = projected[index];
            index += 1;
            while index < N && positions[index] == position {
                index += 1;
            }
        }
        Ok(())
    }

    /// Reserves byte capacity before allocation or backend entry.
    pub fn reserve(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        self.reserve_core(phase, dimension, bytes)
            .map(|core| LiveReservation { core })
    }

    fn reserve_core(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<ReservationCore<O, P, D>, LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        let dimension_position = state
            .dimensions
            .binary_search_by_key(&dimension, |entry| entry.dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        let dimension_entry = &state.dimensions[dimension_position];
        let dimension_limit = dimension_entry
            .policy
            .live_byte_limit
            .ok_or(LedgerCause::DimensionNotReservable { dimension })?;
        let current_dimension = dimension_entry.state.current_live_bytes;
        let dimension_live =
            current_dimension
                .checked_add(bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })?;
        if dimension_live > dimension_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: dimension_live,
                limit: dimension_limit,
            });
        }
        let operation_live =
            state
                .current_live_bytes
                .checked_add(bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })?;
        if operation_live > self.operation_live_byte_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: operation_live,
                limit: self.operation_live_byte_limit,
            });
        }
        let phase_position = phase_position(&state, phase)?;
        let current_phase = state.phase_states[phase_position].current_live_bytes;
        let phase_live =
            current_phase
                .checked_add(bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })?;
        let active_reservations =
            state
                .active_reservations
                .checked_add(1)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })?;

        let dimension_state = &mut state.dimensions[dimension_position].state;
        dimension_state.current_live_bytes = dimension_live;
        dimension_state.peak_live_bytes = dimension_state.peak_live_bytes.max(dimension_live);
        let phase_state = &mut state.phase_states[phase_position];
        phase_state.current_live_bytes = phase_live;
        phase_state.peak_live_bytes = phase_state.peak_live_bytes.max(phase_live);
        state.current_live_bytes = operation_live;
        state.peak_live_bytes = state.peak_live_bytes.max(operation_live);
        state.active_reservations = active_reservations;

        Ok(ReservationCore::new(
            Arc::clone(&self.state),
            phase,
            phase_position,
            dimension,
            dimension_position,
            bytes,
        ))
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn reserve_operation_attribution<Destination>(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<OperationAttributionReservation<O, P, D, Destination>, LedgerCause<D>>
    where
        Destination: PublicationDestination,
    {
        self.reserve_core(phase, dimension, bytes)
            .map(|core| OperationAttributionReservation {
                core,
                destination: PhantomData,
            })
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn try_attribution_batch<Destination>(
        &self,
        phase: P,
        capacity_dimension: D,
        attribution_limit: usize,
        commitment_limit: usize,
    ) -> Result<AttributionReservationBatch<O, P, D, Destination>, AttributionBatchCreateError<D>>
    where
        Destination: PublicationDestination,
    {
        let plan = AttributionBatchPlan::try_new(
            phase,
            capacity_dimension,
            attribution_limit,
            commitment_limit,
        )?;
        self.try_attribution_batch_from_plan(plan)
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn try_attribution_batch_from_plan<Destination>(
        &self,
        plan: AttributionBatchPlan<O, P, D, Destination>,
    ) -> Result<AttributionReservationBatch<O, P, D, Destination>, AttributionBatchCreateError<D>>
    where
        Destination: PublicationDestination,
    {
        #[cfg(test)]
        ATTRIBUTION_BATCH_ALLOCATION_ENTRIES.with(|entries| {
            entries.set(entries.get().saturating_add(1));
        });
        let AttributionBatchPlan {
            phase,
            capacity_dimension,
            attribution_capacity,
            commitment_capacity,
            planned_bytes,
            attribution_limit,
            commitment_limit,
            destination: _,
        } = plan;
        let mut capacity_cover = if planned_bytes == 0 {
            None
        } else {
            Some(StorageCoveringReservation {
                core: self
                    .reserve_core(phase, capacity_dimension, planned_bytes)
                    .map_err(AttributionBatchCreateError::Ledger)?,
            })
        };

        let mut attributions = Vec::new();
        attributions
            .try_reserve_exact(attribution_capacity.allocator_additional())
            .map_err(|_| AttributionBatchCreateError::AllocationFailed)?;
        let mut commitments = Vec::new();
        commitments
            .try_reserve_exact(commitment_capacity.allocator_additional())
            .map_err(|_| AttributionBatchCreateError::AllocationFailed)?;
        let attribution_bytes = attribution_capacity
            .finish(attributions.len(), attributions.capacity())
            .map_err(|_| AttributionBatchCreateError::CapacityOverflow)?;
        let commitment_bytes = commitment_capacity
            .finish(commitments.len(), commitments.capacity())
            .map_err(|_| AttributionBatchCreateError::CapacityOverflow)?;
        let actual_bytes = attribution_bytes
            .checked_add(commitment_bytes)
            .ok_or(AttributionBatchCreateError::CapacityOverflow)?;
        if actual_bytes > planned_bytes {
            return Err(AttributionBatchCreateError::CapacityOverflow);
        }
        match (&mut capacity_cover, actual_bytes) {
            (Some(_), 0) => {
                drop(capacity_cover.take());
            }
            (Some(cover), bytes) => self
                .resize_core(&mut cover.core, bytes)
                .map_err(AttributionBatchCreateError::Ledger)?,
            (None, 0) => {}
            (None, _) => return Err(AttributionBatchCreateError::CapacityOverflow),
        }

        Ok(AttributionReservationBatch {
            attributions,
            commitments,
            capacity_cover,
            attribution_limit,
            commitment_limit,
            owner: Arc::clone(&self.state),
        })
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn try_push_commitment<Destination>(
        &self,
        batch: &mut AttributionReservationBatch<O, P, D, Destination>,
        mut commitment: OutputCommitment<D, Destination>,
    ) -> Result<(), CommitmentPushError<D, Destination>>
    where
        Destination: PublicationDestination,
    {
        if !Arc::ptr_eq(&batch.owner, &self.state) {
            return Err(CommitmentPushError::Ledger {
                commitment,
                cause: LedgerCause::OwnerMismatch,
            });
        }
        if !batch.has_commitment_slot() {
            return Err(CommitmentPushError::BatchFull(commitment));
        }

        let state = lock(&self.state);
        if let Err(cause) = Self::ensure_open(&state) {
            return Err(CommitmentPushError::Ledger { commitment, cause });
        }
        let dimension_position = match state
            .dimensions
            .binary_search_by_key(&commitment.dimension, |entry| entry.dimension)
        {
            Ok(position) => position,
            Err(_) => {
                let dimension = commitment.dimension;
                return Err(CommitmentPushError::Ledger {
                    commitment,
                    cause: LedgerCause::UnknownDimension { dimension },
                });
            }
        };
        commitment.dimension_position = dimension_position;
        batch.push_positioned_commitment(commitment);
        Ok(())
    }

    /// Transfers live ownership to another owner-defined ledger phase without changing totals.
    pub fn transfer(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        phase: P,
    ) -> Result<(), LedgerCause<D>> {
        self.check_reservation(reservation)?;
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        if reservation.core.phase == phase {
            return Ok(());
        }
        let source_position = reservation.core.phase_position;
        if source_position >= state.phase_states.len() {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        let target_position = phase_position(&state, phase)?;
        if source_position == target_position {
            reservation.core.phase = phase;
            return Ok(());
        }
        let source_live = state.phase_states[source_position]
            .current_live_bytes
            .checked_sub(reservation.core.bytes)
            .ok_or(LedgerCause::FinalSnapshotShapeMismatch)?;
        let target_live = state.phase_states[target_position]
            .current_live_bytes
            .checked_add(reservation.core.bytes)
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension: reservation.core.dimension,
            })?;

        state.phase_states[source_position].current_live_bytes = source_live;
        let target = &mut state.phase_states[target_position];
        target.current_live_bytes = target_live;
        target.peak_live_bytes = target.peak_live_bytes.max(target_live);
        reservation.core.phase = phase;
        reservation.core.phase_position = target_position;
        Ok(())
    }

    /// Resizes one live reservation without changing its phase or dimension.
    ///
    /// Admission is atomic: a failed increase leaves both the token and every
    /// ledger counter unchanged, so callers may retain an already-live buffer.
    pub fn resize_reservation(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.check_reservation(reservation)?;
        self.resize_core(&mut reservation.core, bytes)
    }

    fn resize_core(
        &self,
        reservation: &mut ReservationCore<O, P, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        if bytes == reservation.bytes {
            return Ok(());
        }

        let dimension = reservation.dimension;
        let dimension_position = reservation.dimension_position;
        let phase_position = reservation.phase_position;
        if dimension_position >= state.dimensions.len()
            || phase_position >= state.phase_states.len()
        {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        let dimension_limit = state.dimensions[dimension_position]
            .policy
            .live_byte_limit
            .ok_or(LedgerCause::DimensionNotReservable { dimension })?;
        let project = |current: u64| {
            current
                .checked_sub(reservation.bytes)
                .and_then(|without_source| without_source.checked_add(bytes))
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })
        };

        let dimension_live = project(
            state.dimensions[dimension_position]
                .state
                .current_live_bytes,
        )?;
        if dimension_live > dimension_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: dimension_live,
                limit: dimension_limit,
            });
        }

        let operation_live = project(state.current_live_bytes)?;
        if operation_live > self.operation_live_byte_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: operation_live,
                limit: self.operation_live_byte_limit,
            });
        }

        let phase_live = project(state.phase_states[phase_position].current_live_bytes)?;

        let dimension_state = &mut state.dimensions[dimension_position].state;
        dimension_state.current_live_bytes = dimension_live;
        dimension_state.peak_live_bytes = dimension_state.peak_live_bytes.max(dimension_live);
        let phase_state = &mut state.phase_states[phase_position];
        phase_state.current_live_bytes = phase_live;
        phase_state.peak_live_bytes = phase_state.peak_live_bytes.max(phase_live);
        state.current_live_bytes = operation_live;
        state.peak_live_bytes = state.peak_live_bytes.max(operation_live);
        reservation.bytes = bytes;
        Ok(())
    }

    /// Atomically replaces one reservation's phase, dimension, and byte capacity.
    ///
    /// Admission failure leaves the source token and every live/peak counter unchanged, so the
    /// caller may keep storage covered by the original reservation while unwinding or retrying.
    pub fn transition_reservation(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.transition_reservations(std::slice::from_mut(reservation), phase, dimension, bytes)
    }

    /// Atomically reclassifies a reservation batch into one phase and dimension.
    ///
    /// `bytes` is distributed in source order, preserving each token's current size until the
    /// requested total is exhausted. The last token absorbs any requested growth. Admission or
    /// evidence failure leaves every token and every live current/peak counter unchanged.
    pub fn transition_reservations(
        &self,
        reservations: &mut [LiveReservation<O, P, D>],
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        if reservations.is_empty() && bytes != 0 {
            return Err(LedgerCause::SplitExceedsReservation {
                requested: bytes,
                available: 0,
            });
        }
        for reservation in reservations.iter() {
            self.check_reservation(reservation)?;
        }

        let source_bytes = reservations.iter().try_fold(0_u64, |total, reservation| {
            total
                .checked_add(reservation.core.bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })
        })?;
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        let dimension_position = state
            .dimensions
            .binary_search_by_key(&dimension, |entry| entry.dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        let dimension_limit = state.dimensions[dimension_position]
            .policy
            .live_byte_limit
            .ok_or(LedgerCause::DimensionNotReservable { dimension })?;
        let phase_position = phase_position(&state, phase)?;
        if reservations.iter().any(|reservation| {
            reservation.core.dimension_position >= state.dimensions.len()
                || reservation.core.phase_position >= state.phase_states.len()
        }) {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        let target_dimension_source = reservations
            .iter()
            .filter(|reservation| reservation.core.dimension_position == dimension_position)
            .try_fold(0_u64, |total, reservation| {
                total
                    .checked_add(reservation.core.bytes)
                    .ok_or(LedgerCause::ArithmeticOverflow {
                        class: CounterClass::Live,
                        dimension,
                    })
            })?;
        let target_phase_source = reservations
            .iter()
            .filter(|reservation| reservation.core.phase_position == phase_position)
            .try_fold(0_u64, |total, reservation| {
                total
                    .checked_add(reservation.core.bytes)
                    .ok_or(LedgerCause::ArithmeticOverflow {
                        class: CounterClass::Live,
                        dimension,
                    })
            })?;
        let project = |current: u64, source: u64| {
            current
                .checked_sub(source)
                .and_then(|without_source| without_source.checked_add(bytes))
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })
        };

        let dimension_live = project(
            state.dimensions[dimension_position]
                .state
                .current_live_bytes,
            target_dimension_source,
        )?;
        if dimension_live > dimension_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: dimension_live,
                limit: dimension_limit,
            });
        }
        let operation_live = project(state.current_live_bytes, source_bytes)?;
        if operation_live > self.operation_live_byte_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: operation_live,
                limit: self.operation_live_byte_limit,
            });
        }
        let phase_live = project(
            state.phase_states[phase_position].current_live_bytes,
            target_phase_source,
        )?;

        for reservation in reservations.iter() {
            release_positioned_bytes_locked(
                &mut state,
                reservation.core.phase_position,
                reservation.core.dimension_position,
                reservation.core.bytes,
            );
        }
        let dimension_state = &mut state.dimensions[dimension_position].state;
        dimension_state.current_live_bytes = dimension_live;
        dimension_state.peak_live_bytes = dimension_state.peak_live_bytes.max(dimension_live);
        let phase_state = &mut state.phase_states[phase_position];
        phase_state.current_live_bytes = phase_live;
        phase_state.peak_live_bytes = phase_state.peak_live_bytes.max(phase_live);
        state.current_live_bytes = operation_live;
        state.peak_live_bytes = state.peak_live_bytes.max(operation_live);
        let last = reservations.len().saturating_sub(1);
        let mut remaining = bytes;
        for (index, reservation) in reservations.iter_mut().enumerate() {
            let target_bytes = if index == last {
                remaining
            } else {
                reservation.core.bytes.min(remaining)
            };
            remaining -= target_bytes;
            reservation.core.phase = phase;
            reservation.core.phase_position = phase_position;
            reservation.core.dimension = dimension;
            reservation.core.dimension_position = dimension_position;
            reservation.core.bytes = target_bytes;
        }
        debug_assert_eq!(remaining, 0);
        drop(state);
        Ok(())
    }

    /// Splits bytes into a second reservation without changing live or peak accounting.
    pub fn split(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        self.check_reservation(reservation)?;
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        if bytes > reservation.core.bytes {
            return Err(LedgerCause::SplitExceedsReservation {
                requested: bytes,
                available: reservation.core.bytes,
            });
        }
        state.active_reservations =
            state
                .active_reservations
                .checked_add(1)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: reservation.core.dimension,
                })?;
        drop(state);

        reservation.core.bytes -= bytes;
        Ok(LiveReservation {
            core: ReservationCore::new(
                Arc::clone(&reservation.core.owner),
                reservation.core.phase,
                reservation.core.phase_position,
                reservation.core.dimension,
                reservation.core.dimension_position,
                bytes,
            ),
        })
    }

    /// Verifies that a live reservation belongs to this operation without mutating accounting.
    pub fn validate_live_reservation(
        &self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.check_reservation(reservation)
    }

    /// Atomically validates a durable commitment batch and begins pending publication.
    ///
    /// Durable committed evidence is unchanged until the owning coordinator publishes the returned
    /// transaction. Success moves every reservation into the returned transaction and empties the
    /// submitted batch. Failure leaves the batch untouched so its owner can drop the covered
    /// storage before releasing the reservations. The batch must own every current live byte; once
    /// admitted, no accounting mutation can race the final control boundary.
    pub fn prepare_publication(
        &self,
        reservations: &mut PublicationReservationBatch<O, P, D>,
        commitments: &[Commitment<D>],
    ) -> Result<PendingPublication<O, P, D>, LedgerCause<D>> {
        for reservation in reservations.iter() {
            self.check_reservation(reservation)?;
        }
        let transaction_live_bytes = Self::reservation_bytes(reservations)?;

        let mut state = lock(&self.state);
        match state.publication {
            PublicationState::Open => {}
            PublicationState::Pending => return Err(LedgerCause::PublicationAlreadyPending),
            PublicationState::TerminalQualified | PublicationState::Published => {
                return Err(LedgerCause::PublicationAlreadyCompleted);
            }
        }
        if transaction_live_bytes != state.current_live_bytes {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: transaction_live_bytes,
                ledger_bytes: state.current_live_bytes,
            });
        }
        let CommitmentAggregate {
            entries: aggregate,
            evidence_bytes,
        } = aggregate_commitments(&mut state, commitments).map_err(|cause| match cause {
            AggregateCommitmentError::Ledger(cause) => cause,
            AggregateCommitmentError::Evidence(cause) => LedgerCause::Evidence { cause },
        })?;
        if let Err(cause) = validate_commitments(&state.dimensions, &aggregate) {
            drop(aggregate);
            release_evidence(&mut state, evidence_bytes);
            return Err(cause);
        }
        for commitment in &aggregate {
            let entry = &mut state.dimensions[commitment.dimension_position].state;
            entry.pending_publication += commitment.amount;
        }
        state.publication = PublicationState::Pending;
        drop(state);

        Ok(PendingPublication {
            owner: Arc::clone(&self.state),
            reservations: std::mem::take(reservations),
            commitments: aggregate,
            evidence_bytes,
            pending: true,
            operation_owner: PhantomData,
        })
    }

    /// Captures immutable owner-neutral evidence without finalizing the ledger.
    pub fn try_snapshot(&self) -> Result<LedgerSnapshot<P, D>, LedgerEvidenceError> {
        try_snapshot(&mut lock(&self.state))
    }

    pub(crate) fn try_grow_progress_evidence<T>(
        &self,
        values: &mut Vec<T>,
        desired_capacity: usize,
        retained_bytes: &mut u64,
    ) -> Result<(), LedgerEvidenceError> {
        let mut state = lock(&self.state);
        let evidence_byte_limit = state.evidence_byte_limit;
        let LedgerState {
            current_evidence_bytes,
            peak_evidence_bytes,
            ..
        } = &mut *state;
        try_grow_tracked_evidence_vec(
            values,
            desired_capacity,
            retained_bytes,
            current_evidence_bytes,
            peak_evidence_bytes,
            evidence_byte_limit,
            LedgerEvidencePurpose::Progress,
        )
    }

    pub(crate) fn release_progress_evidence(&self, retained_bytes: u64) {
        let mut state = lock(&self.state);
        debug_assert!(state.current_evidence_bytes >= retained_bytes);
        release_evidence(&mut state, retained_bytes);
    }

    /// Copies one dimension's scalar evidence without allocating a full snapshot.
    pub fn dimension_evidence(&self, dimension: D) -> Result<DimensionSnapshot, LedgerCause<D>> {
        let state = lock(&self.state);
        dimension_entry(&state.dimensions, dimension)
            .map(|entry| dimension_snapshot(entry.state))
            .ok_or(LedgerCause::UnknownDimension { dimension })
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn dimension_count(&self) -> usize {
        lock(&self.state).dimensions.len()
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn dimension_at(&self, index: usize) -> Option<D> {
        lock(&self.state)
            .dimensions
            .get(index)
            .map(|entry| entry.dimension)
    }

    pub(crate) fn phase_at(&self, index: usize) -> Option<P> {
        lock(&self.state).phases.get(index).map(|entry| entry.phase)
    }

    pub(crate) fn catalog_shape_is_strictly_ordered(
        &self,
        expected_dimensions: usize,
        expected_phases: usize,
    ) -> bool {
        let state = lock(&self.state);
        state.dimensions.len() == expected_dimensions
            && state.phases.len() == expected_phases
            && state
                .dimensions
                .windows(2)
                .all(|pair| pair[0].dimension < pair[1].dimension)
            && state
                .phases
                .windows(2)
                .all(|pair| pair[0].phase < pair[1].phase)
    }

    pub(crate) fn contains_dimension(&self, dimension: D) -> bool {
        dimension_entry(&lock(&self.state).dimensions, dimension).is_some()
    }

    pub(crate) fn contains_phase(&self, phase: P) -> bool {
        phase_entry(&lock(&self.state).phases, phase).is_some()
    }

    /// Records one semantically completed phase in frozen catalog insertion order.
    ///
    /// Repeating the current phase is idempotent. Every rejection occurs before the stored
    /// completion position changes.
    pub(crate) fn try_complete_phase(&self, phase: P) -> Result<(), PhaseCompletionError<P>> {
        let mut state = lock(&self.state);
        match state.publication {
            PublicationState::Open => {}
            PublicationState::Pending => return Err(PhaseCompletionError::PublicationPending),
            PublicationState::TerminalQualified | PublicationState::Published => {
                return Err(PhaseCompletionError::PublicationCompleted);
            }
        }
        if !phase_catalog_shape_is_valid(&state) {
            return Err(PhaseCompletionError::CatalogShapeMismatch);
        }
        let phase_position = phase_entry(&state.phases, phase)
            .map(|entry| entry.state_position)
            .ok_or(PhaseCompletionError::UnknownPhase { phase })?;
        let Some(last_completed) = state.last_completed_phase else {
            state.last_completed_phase = Some(CompletedPhase {
                phase,
                state_position: phase_position,
            });
            return Ok(());
        };
        if phase_position == last_completed.state_position {
            return Ok(());
        }
        if phase_position < last_completed.state_position {
            return Err(PhaseCompletionError::Regression {
                last_completed_phase: last_completed.phase,
                phase,
            });
        }
        state.last_completed_phase = Some(CompletedPhase {
            phase,
            state_position: phase_position,
        });
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn validate_typed_batch_pair<Destination>(
        &self,
        first: &AttributionReservationBatch<O, P, D, Destination>,
        second: &AttributionReservationBatch<O, P, D, Destination>,
    ) -> Result<(), LedgerCause<D>>
    where
        Destination: DestinationState,
    {
        if !first.validate_owner(&self.state) || !second.validate_owner(&self.state) {
            return Err(LedgerCause::OwnerMismatch);
        }
        validate_typed_commitment_pair(&lock(&self.state).dimensions, first, second)
    }

    #[allow(
        dead_code,
        clippy::result_large_err,
        reason = "the staged seam preserves every reservation on rejection without allocating"
    )]
    pub(crate) fn try_fallback_baseline(
        &self,
        canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
        evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        fallback: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        evidence: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    ) -> Result<FallbackBatchBaseline<O, P, D>, FallbackBaselineRejection<O, P, D>> {
        let mut baseline = FallbackBatchBaseline {
            canonical_assembly,
            evidence_assembly,
            fallback,
            evidence,
        };
        if let Err(cause) = self.try_bind_fallback_baseline(&mut baseline) {
            return Err(FallbackBaselineRejection { baseline, cause });
        }
        Ok(baseline)
    }

    pub(crate) fn try_bind_fallback_baseline(
        &self,
        baseline: &mut FallbackBatchBaseline<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        let state = lock(&self.state);
        self.validate_fallback_baseline_locked(&state, baseline)?;
        preflight_typed_publication_batch(&state, &baseline.canonical_assembly)?;
        preflight_typed_publication_batch(&state, &baseline.evidence_assembly)?;
        preflight_typed_publication_batch(&state, &baseline.fallback)?;
        preflight_typed_publication_batch(&state, &baseline.evidence)?;
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    fn validate_fallback_baseline_locked(
        &self,
        state: &LedgerState<P, D>,
        baseline: &FallbackBatchBaseline<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        if !baseline.canonical_assembly.validate_owner(&self.state)
            || !baseline.evidence_assembly.validate_owner(&self.state)
            || !baseline.fallback.validate_owner(&self.state)
            || !baseline.evidence.validate_owner(&self.state)
        {
            return Err(LedgerCause::OwnerMismatch);
        }
        if !baseline
            .evidence_assembly
            .can_absorb_sources([Some(&baseline.evidence), Some(&baseline.fallback)])
        {
            return Err(LedgerCause::ActiveReservationsRemain { count: u64::MAX });
        }
        let expected_active = baseline
            .canonical_assembly
            .active_count()
            .and_then(|count| {
                baseline
                    .evidence_assembly
                    .active_count()?
                    .checked_add(count)
            })
            .and_then(|count| baseline.fallback.active_count()?.checked_add(count))
            .and_then(|count| baseline.evidence.active_count()?.checked_add(count))
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let expected_live = baseline
            .canonical_assembly
            .live_bytes()
            .and_then(|bytes| baseline.evidence_assembly.live_bytes()?.checked_add(bytes))
            .and_then(|bytes| baseline.fallback.live_bytes()?.checked_add(bytes))
            .and_then(|bytes| baseline.evidence.live_bytes()?.checked_add(bytes))
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        Self::ensure_open(state)?;
        validate_final_snapshot_shape(state)?;
        if state.active_reservations != expected_active {
            return Err(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            });
        }
        if state.current_live_bytes != expected_live {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: expected_live,
                ledger_bytes: state.current_live_bytes,
            });
        }
        validate_typed_commitments(&state.dimensions, &baseline.canonical_assembly)?;
        validate_typed_commitment_batches(
            &state.dimensions,
            [
                Some(&baseline.evidence_assembly),
                Some(&baseline.evidence),
                Some(&baseline.fallback),
            ],
        )
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn try_bind_primary(
        &self,
        baseline: &mut FallbackBatchBaseline<O, P, D>,
        canonical_source: Option<&mut AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<
            &mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        >,
        publish_canonical: bool,
    ) -> Result<(), LedgerCause<D>> {
        let state = lock(&self.state);
        self.validate_primary_publication_locked(
            &state,
            baseline,
            canonical_source.as_deref(),
            evidence_source.as_deref(),
            publish_canonical,
        )?;
        preflight_typed_publication_batch(&state, &baseline.canonical_assembly)?;
        preflight_typed_publication_batch(&state, &baseline.evidence_assembly)?;
        preflight_typed_publication_batch(&state, &baseline.evidence)?;
        if let Some(batch) = canonical_source.as_deref() {
            if publish_canonical {
                preflight_typed_publication_batch(&state, batch)?;
            } else {
                preflight_typed_discard_batch(&state, batch)?;
            }
        }
        if let Some(batch) = evidence_source.as_deref() {
            preflight_typed_publication_batch(&state, batch)?;
        }
        drop(state);

        if publish_canonical && let Some(source) = canonical_source {
            baseline
                .canonical_assembly
                .absorb_proven_retaining_source_storage(source);
        }
        baseline
            .evidence_assembly
            .absorb_proven_retaining_source_storage(&mut baseline.evidence);
        if let Some(source) = evidence_source {
            baseline
                .evidence_assembly
                .absorb_proven_retaining_source_storage(source);
        }
        Ok(())
    }

    pub(crate) fn try_bind_fallback_discards(
        &self,
        baseline: &mut FallbackBatchBaseline<O, P, D>,
        canonical_discard: Option<&mut AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_discard: Option<
            &mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        >,
    ) -> Result<(), LedgerCause<D>> {
        if !baseline.canonical_assembly.validate_owner(&self.state)
            || !baseline.evidence_assembly.validate_owner(&self.state)
            || !baseline.fallback.validate_owner(&self.state)
            || !baseline.evidence.validate_owner(&self.state)
            || canonical_discard
                .as_deref()
                .is_some_and(|batch| !batch.validate_owner(&self.state))
            || evidence_discard
                .as_deref()
                .is_some_and(|batch| !batch.validate_owner(&self.state))
        {
            return Err(LedgerCause::OwnerMismatch);
        }
        if !baseline
            .evidence_assembly
            .can_absorb_sources([Some(&baseline.evidence), Some(&baseline.fallback)])
        {
            return Err(LedgerCause::ActiveReservationsRemain { count: u64::MAX });
        }

        let state = lock(&self.state);
        Self::ensure_open(&state)?;
        validate_final_snapshot_shape(&state)?;
        if let Some(batch) = canonical_discard.as_deref() {
            preflight_typed_discard_batch(&state, batch)?;
        }
        if let Some(batch) = evidence_discard.as_deref() {
            preflight_typed_discard_batch(&state, batch)?;
        }
        drop(state);

        baseline
            .evidence_assembly
            .absorb_proven_retaining_source_storage(&mut baseline.evidence);
        baseline
            .evidence_assembly
            .absorb_proven_retaining_source_storage(&mut baseline.fallback);
        Ok(())
    }

    /// Consumes a primary batch graph only after [`Self::try_bind_primary`] has completed.
    ///
    /// Binding and source absorption already completed while the caller retained the coupled
    /// physical owner. This step performs only field moves and can therefore consume the graph
    /// without opening a storage-before-attribution unwind window.
    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn finish_bound_primary(
        self,
        baseline: FallbackBatchBaseline<O, P, D>,
        canonical_source: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
        publish_canonical: bool,
    ) -> TypedFreezeProof<O, P, D> {
        let FallbackBatchBaseline {
            canonical_assembly,
            evidence_assembly,
            fallback,
            evidence,
        } = baseline;
        let discard_evidence = if fallback.active_count() == Some(0) {
            drop(fallback);
            None
        } else {
            Some(fallback)
        };
        let (deferred_canonical_storage, discard_canonical) = if publish_canonical {
            (canonical_source, None)
        } else {
            (None, canonical_source)
        };
        TypedFreezeProof {
            canonical: canonical_assembly,
            evidence: evidence_assembly,
            deferred_canonical_storage,
            deferred_evidence_storage_first: Some(evidence),
            deferred_evidence_storage_second: evidence_source,
            discard_canonical,
            discard_evidence,
            ledger: self,
        }
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    fn validate_primary_publication_locked(
        &self,
        state: &LedgerState<P, D>,
        baseline: &FallbackBatchBaseline<O, P, D>,
        canonical_source: Option<&AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<&AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
        publish_canonical: bool,
    ) -> Result<(), LedgerCause<D>> {
        if !baseline.canonical_assembly.validate_owner(&self.state)
            || !baseline.evidence_assembly.validate_owner(&self.state)
            || !baseline.fallback.validate_owner(&self.state)
            || !baseline.evidence.validate_owner(&self.state)
            || canonical_source.is_some_and(|batch| !batch.validate_owner(&self.state))
            || evidence_source.is_some_and(|batch| !batch.validate_owner(&self.state))
        {
            return Err(LedgerCause::OwnerMismatch);
        }
        if (publish_canonical
            && !baseline
                .canonical_assembly
                .can_absorb_sources([canonical_source]))
            || !baseline
                .evidence_assembly
                .can_absorb_sources([Some(&baseline.evidence), evidence_source])
        {
            return Err(LedgerCause::ActiveReservationsRemain { count: u64::MAX });
        }

        let canonical_attribution_count = baseline
            .canonical_assembly
            .combined_attribution_count([publish_canonical.then_some(canonical_source).flatten()])
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let evidence_attribution_count = baseline
            .evidence_assembly
            .combined_attribution_count([Some(&baseline.evidence), evidence_source])
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let projected_active = canonical_attribution_count
            .checked_add(evidence_attribution_count)
            .and_then(|count| {
                count.checked_add(u64::from(
                    baseline.canonical_assembly.capacity_cover.is_some(),
                ))
            })
            .and_then(|count| {
                count.checked_add(u64::from(
                    baseline.evidence_assembly.capacity_cover.is_some(),
                ))
            })
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let canonical_attribution_bytes = baseline
            .canonical_assembly
            .combined_attribution_bytes([publish_canonical.then_some(canonical_source).flatten()])
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        let evidence_attribution_bytes = baseline
            .evidence_assembly
            .combined_attribution_bytes([Some(&baseline.evidence), evidence_source])
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        let projected_live = canonical_attribution_bytes
            .checked_add(evidence_attribution_bytes)
            .and_then(|bytes| {
                bytes.checked_add(baseline.canonical_assembly.account_capacity_bytes())
            })
            .and_then(|bytes| {
                bytes.checked_add(baseline.evidence_assembly.account_capacity_bytes())
            })
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        let released_canonical_active = if publish_canonical {
            canonical_source.map_or(Some(0), |batch| {
                Some(u64::from(batch.capacity_cover.is_some()))
            })
        } else {
            canonical_source.map_or(Some(0), AttributionReservationBatch::active_count)
        }
        .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let released_active = released_canonical_active
            .checked_add(u64::from(baseline.evidence.capacity_cover.is_some()))
            .and_then(|count| {
                count.checked_add(u64::from(
                    evidence_source.is_some_and(|batch| batch.capacity_cover.is_some()),
                ))
            })
            .and_then(|count| count.checked_add(baseline.fallback.active_count()?))
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let released_canonical_live = if publish_canonical {
            Some(canonical_source.map_or(0, AttributionReservationBatch::account_capacity_bytes))
        } else {
            canonical_source.map_or(Some(0), AttributionReservationBatch::live_bytes)
        }
        .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        let released_live = released_canonical_live
            .checked_add(baseline.evidence.account_capacity_bytes())
            .and_then(|bytes| {
                bytes.checked_add(
                    evidence_source.map_or(0, AttributionReservationBatch::account_capacity_bytes),
                )
            })
            .and_then(|bytes| bytes.checked_add(baseline.fallback.live_bytes()?))
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;

        Self::ensure_open(state)?;
        validate_final_snapshot_shape(state)?;
        let active_after_transition = state
            .active_reservations
            .checked_sub(released_active)
            .ok_or(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            })?;
        if active_after_transition != projected_active {
            return Err(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            });
        }
        let live_after_transition = state.current_live_bytes.checked_sub(released_live).ok_or(
            LedgerCause::LiveReservationsRemain {
                bytes: state.current_live_bytes,
            },
        )?;
        if live_after_transition != projected_live {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: projected_live,
                ledger_bytes: live_after_transition,
            });
        }
        validate_typed_commitment_batches(
            &state.dimensions,
            [
                Some(&baseline.canonical_assembly),
                publish_canonical.then_some(canonical_source).flatten(),
            ],
        )?;
        validate_typed_commitment_batches(
            &state.dimensions,
            [
                Some(&baseline.evidence_assembly),
                Some(&baseline.evidence),
                evidence_source,
            ],
        )
    }

    /// Copies one phase's scalar evidence without allocating a full snapshot.
    pub fn phase_evidence(&self, phase: P) -> Option<PhaseSnapshot> {
        let state = lock(&self.state);
        phase_state(&state, phase).map(|state| phase_snapshot(*state))
    }

    /// Finalizes evidence only after every live reservation and pending transaction is gone.
    #[allow(
        clippy::result_large_err,
        reason = "boxing final evidence would allocate on the invariant-failure path"
    )]
    pub fn finish(self) -> Result<LedgerSnapshot<P, D>, LedgerFinishError<P, D>> {
        let mut state = lock(&self.state);
        let shape_cause = validate_final_snapshot_shape(&state).err();
        let live_cause =
            (state.current_live_bytes != 0).then_some(LedgerCause::LiveReservationsRemain {
                bytes: state.current_live_bytes,
            });
        let pending_cause = matches!(
            state.publication,
            PublicationState::Pending | PublicationState::TerminalQualified
        )
        .then_some(LedgerCause::PendingPublicationsRemain);
        let reservation_cause = (state.current_live_bytes == 0 && state.active_reservations != 0)
            .then_some(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            });
        let clean = shape_cause.is_none()
            && live_cause.is_none()
            && pending_cause.is_none()
            && reservation_cause.is_none();
        let evidence = take_final_snapshot(&mut state, clean);
        if let Some(cause) = shape_cause {
            return Err(LedgerFinishError {
                cause,
                snapshot: evidence,
            });
        }
        if let Some(cause) = live_cause {
            return Err(LedgerFinishError {
                cause,
                snapshot: evidence,
            });
        }
        if let Some(cause) = pending_cause {
            return Err(LedgerFinishError {
                cause,
                snapshot: evidence,
            });
        }
        if let Some(cause) = reservation_cause {
            return Err(LedgerFinishError {
                cause,
                snapshot: evidence,
            });
        }
        Ok(evidence)
    }

    fn check_reservation(
        &self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        if !reservation.core.is_owned_by(&self.state) {
            return Err(LedgerCause::OwnerMismatch);
        }
        if !reservation.core.active {
            return Err(LedgerCause::InactiveReservation);
        }
        Ok(())
    }

    pub(crate) fn check_publication(
        &self,
        publication: &PendingPublication<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        if !Arc::ptr_eq(&self.state, &publication.owner) {
            return Err(LedgerCause::OwnerMismatch);
        }
        if !publication.pending {
            return Err(LedgerCause::InactivePublication);
        }
        let transaction_live_bytes = Self::reservation_bytes(&publication.reservations)?;
        let state = lock(&self.state);
        if state.publication != PublicationState::Pending {
            return Err(LedgerCause::InactivePublication);
        }
        if transaction_live_bytes != state.current_live_bytes {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: transaction_live_bytes,
                ledger_bytes: state.current_live_bytes,
            });
        }
        Ok(())
    }

    pub(crate) fn publish(
        &self,
        publication: &mut PendingPublication<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.check_publication(publication)?;
        let mut state = lock(&self.state);
        if state.publication != PublicationState::Pending {
            return Err(LedgerCause::InactivePublication);
        }
        let transaction_live_bytes = Self::reservation_bytes(&publication.reservations)?;
        if transaction_live_bytes != state.current_live_bytes {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: transaction_live_bytes,
                ledger_bytes: state.current_live_bytes,
            });
        }

        for commitment in &publication.commitments {
            let entry = state
                .dimensions
                .get(commitment.dimension_position)
                .ok_or(LedgerCause::FinalSnapshotShapeMismatch)?;
            entry
                .state
                .pending_publication
                .checked_sub(commitment.amount)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::PendingPublication,
                    dimension: commitment.dimension,
                })?;
            entry.state.committed.checked_add(commitment.amount).ok_or(
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Committed,
                    dimension: commitment.dimension,
                },
            )?;
        }

        for commitment in &publication.commitments {
            let entry = &mut state.dimensions[commitment.dimension_position].state;
            entry.pending_publication -= commitment.amount;
            entry.committed += commitment.amount;
        }
        publication.reservations.release_locked(&mut state);
        drop(std::mem::take(&mut publication.commitments));
        release_evidence(&mut state, publication.evidence_bytes);
        publication.evidence_bytes = 0;
        state.publication = PublicationState::Published;
        publication.pending = false;
        Ok(())
    }

    fn ensure_open(state: &LedgerState<P, D>) -> Result<(), LedgerCause<D>> {
        match state.publication {
            PublicationState::Open => Ok(()),
            PublicationState::Pending => Err(LedgerCause::PublicationAlreadyPending),
            PublicationState::TerminalQualified | PublicationState::Published => {
                Err(LedgerCause::PublicationAlreadyCompleted)
            }
        }
    }

    fn reservation_bytes(
        reservations: &PublicationReservationBatch<O, P, D>,
    ) -> Result<u64, LedgerCause<D>> {
        reservations.iter().try_fold(0u64, |total, reservation| {
            total
                .checked_add(reservation.core.bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: reservation.core.dimension,
                })
        })
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug)]
pub(crate) enum AttributionBatchCreateError<D> {
    CapacityOverflow,
    Ledger(LedgerCause<D>),
    AllocationFailed,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) enum CommitmentPushError<D, Destination> {
    BatchFull(OutputCommitment<D, Destination>),
    Ledger {
        commitment: OutputCommitment<D, Destination>,
        cause: LedgerCause<D>,
    },
}

#[allow(
    dead_code,
    clippy::type_complexity,
    reason = "the exact fallback batch product remains statically destination-qualified"
)]
impl<O, P, D> FallbackBatchBaseline<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn finish_prepared_fallback_proof(
        self,
        ledger: Ledger<O, P, D>,
        discard_canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        discard_evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    ) -> TypedFreezeProof<O, P, D> {
        let Self {
            canonical_assembly,
            evidence_assembly,
            fallback,
            evidence,
        } = self;
        TypedFreezeProof {
            canonical: canonical_assembly,
            evidence: evidence_assembly,
            deferred_canonical_storage: None,
            deferred_evidence_storage_first: Some(evidence),
            deferred_evidence_storage_second: Some(fallback),
            discard_canonical,
            discard_evidence,
            ledger,
        }
    }

    #[cfg(test)]
    pub(crate) fn into_fallback_proof(
        mut self,
        ledger: Ledger<O, P, D>,
        discard_canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        discard_evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    ) -> TypedFreezeProof<O, P, D> {
        assert!(self.try_prepare_fallback_sources().is_ok());
        self.finish_prepared_fallback_proof(ledger, discard_canonical, discard_evidence)
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct TypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    deferred_canonical_storage: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    deferred_evidence_storage_first:
        Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    deferred_evidence_storage_second:
        Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    discard_canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    discard_evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    ledger: Ledger<O, P, D>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypedFreezeQualificationError {
    PublicationState,
    SnapshotShape,
    Ownership,
    BoundPositionOutOfBounds,
    ArithmeticOverflow,
    CanonicalCommitmentOverflow {
        index: usize,
    },
    CanonicalCommitmentLimitExceeded {
        index: usize,
        value: u64,
        limit: u64,
    },
    CanonicalCommitmentSubsetViolation {
        index: usize,
        canonical: u64,
        committed: u64,
    },
    OutcomeEvidenceCommitmentOverflow {
        index: usize,
    },
    OutcomeEvidenceCommitmentLimitExceeded {
        index: usize,
        value: u64,
        limit: u64,
    },
    GlobalLiveMismatch {
        expected: u64,
        actual: u64,
    },
    ActiveReservationMismatch {
        expected: u64,
        actual: u64,
    },
    DimensionLiveMismatch {
        index: usize,
        expected: u64,
        actual: u64,
    },
    PendingPublication {
        index: usize,
        amount: u64,
    },
    PhaseLiveMismatch {
        index: usize,
        expected: u64,
        actual: u64,
    },
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
struct QualifiedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<TypedFreezeProof<O, P, D>>,
    last_completed_phase: Option<P>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct QualifiedEvidenceDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<QualifiedTypedFreezeProof<O, P, D>>,
    discard_evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    acknowledgement: QualifiedDiscardAcknowledgement,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QualifiedDiscardAcknowledgement {
    NotStarted,
    InProgress,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct QualifiedCanonicalDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<QualifiedTypedFreezeProof<O, P, D>>,
    discard_canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    acknowledgement: QualifiedDiscardAcknowledgement,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct QualifiedBothDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<QualifiedTypedFreezeProof<O, P, D>>,
    discard_canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    discard_evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    acknowledgement: QualifiedDiscardAcknowledgement,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) enum QualifiedTypedDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    None(FreezeReadyTypedProof<O, P, D>),
    Canonical(QualifiedCanonicalDiscardFreezeProof<O, P, D>),
    Evidence(QualifiedEvidenceDiscardFreezeProof<O, P, D>),
    Both(QualifiedBothDiscardFreezeProof<O, P, D>),
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct FreezeReadyTypedProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<QualifiedTypedFreezeProof<O, P, D>>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D> TypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn validate_terminal(
        &self,
        state: &LedgerState<P, D>,
    ) -> Result<Option<P>, TypedFreezeQualificationError> {
        if state.publication != PublicationState::Open {
            return Err(TypedFreezeQualificationError::PublicationState);
        }
        if !typed_terminal_snapshot_positions_are_valid(state) {
            return Err(TypedFreezeQualificationError::SnapshotShape);
        }
        if !self.canonical.validate_owner(&self.ledger.state)
            || !self.evidence.validate_owner(&self.ledger.state)
            || self
                .deferred_canonical_storage
                .as_ref()
                .is_some_and(|batch| {
                    !batch.validate_owner(&self.ledger.state) || !batch.is_empty_storage_shell()
                })
            || self
                .deferred_evidence_storage_first
                .as_ref()
                .is_some_and(|batch| {
                    !batch.validate_owner(&self.ledger.state) || !batch.is_empty_storage_shell()
                })
            || self
                .deferred_evidence_storage_second
                .as_ref()
                .is_some_and(|batch| {
                    !batch.validate_owner(&self.ledger.state) || !batch.is_empty_storage_shell()
                })
            || self
                .discard_canonical
                .as_ref()
                .is_some_and(|batch| !batch.validate_owner(&self.ledger.state))
            || self
                .discard_evidence
                .as_ref()
                .is_some_and(|batch| !batch.validate_owner(&self.ledger.state))
        {
            return Err(TypedFreezeQualificationError::Ownership);
        }
        let dimension_count = state.dimensions.len();
        let phase_count = state.phase_states.len();
        if !self
            .canonical
            .bound_positions_are_in_bounds(dimension_count, phase_count)
            || !self
                .evidence
                .bound_positions_are_in_bounds(dimension_count, phase_count)
            || self
                .deferred_canonical_storage
                .as_ref()
                .is_some_and(|batch| {
                    !batch.bound_positions_are_in_bounds(dimension_count, phase_count)
                })
            || self
                .deferred_evidence_storage_first
                .as_ref()
                .is_some_and(|batch| {
                    !batch.bound_positions_are_in_bounds(dimension_count, phase_count)
                })
            || self
                .deferred_evidence_storage_second
                .as_ref()
                .is_some_and(|batch| {
                    !batch.bound_positions_are_in_bounds(dimension_count, phase_count)
                })
            || self.discard_canonical.as_ref().is_some_and(|batch| {
                !batch.discard_positions_are_in_bounds(dimension_count, phase_count)
            })
            || self.discard_evidence.as_ref().is_some_and(|batch| {
                !batch.discard_positions_are_in_bounds(dimension_count, phase_count)
            })
        {
            return Err(TypedFreezeQualificationError::BoundPositionOutOfBounds);
        }

        let expected_live = self
            .canonical
            .live_bytes()
            .and_then(|value| value.checked_add(self.evidence.live_bytes()?))
            .and_then(|value| {
                value.checked_add(
                    self.deferred_canonical_storage
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.deferred_evidence_storage_first
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.deferred_evidence_storage_second
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.discard_canonical
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.discard_evidence
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
        if state.current_live_bytes != expected_live {
            return Err(TypedFreezeQualificationError::GlobalLiveMismatch {
                expected: expected_live,
                actual: state.current_live_bytes,
            });
        }

        let expected_active = self
            .canonical
            .active_count()
            .and_then(|value| value.checked_add(self.evidence.active_count()?))
            .and_then(|value| {
                value.checked_add(
                    self.deferred_canonical_storage
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.deferred_evidence_storage_first
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.deferred_evidence_storage_second
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.discard_canonical
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.discard_evidence
                        .as_ref()
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
        if state.active_reservations != expected_active {
            return Err(TypedFreezeQualificationError::ActiveReservationMismatch {
                expected: expected_active,
                actual: state.active_reservations,
            });
        }

        for (index, entry) in state.dimensions.iter().enumerate() {
            let expected = self
                .canonical
                .positioned_dimension_bytes(index)
                .and_then(|value| {
                    value.checked_add(self.evidence.positioned_dimension_bytes(index)?)
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_canonical_storage
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_evidence_storage_first
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_evidence_storage_second
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.discard_canonical
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.discard_evidence
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
            if entry.state.current_live_bytes != expected {
                return Err(TypedFreezeQualificationError::DimensionLiveMismatch {
                    index,
                    expected,
                    actual: entry.state.current_live_bytes,
                });
            }
            if entry.state.pending_publication != 0 {
                return Err(TypedFreezeQualificationError::PendingPublication {
                    index,
                    amount: entry.state.pending_publication,
                });
            }

            let canonical_increment = self
                .canonical
                .positioned_commitment_amount(index)
                .ok_or(TypedFreezeQualificationError::CanonicalCommitmentOverflow { index })?;
            let committed = entry
                .state
                .committed
                .checked_add(canonical_increment)
                .ok_or(TypedFreezeQualificationError::CanonicalCommitmentOverflow { index })?;
            let committed_canonical_output = entry
                .state
                .committed_canonical_output
                .checked_add(canonical_increment)
                .ok_or(TypedFreezeQualificationError::CanonicalCommitmentOverflow { index })?;
            if committed > entry.policy.committed_limit {
                return Err(
                    TypedFreezeQualificationError::CanonicalCommitmentLimitExceeded {
                        index,
                        value: committed,
                        limit: entry.policy.committed_limit,
                    },
                );
            }
            if committed_canonical_output > committed {
                return Err(
                    TypedFreezeQualificationError::CanonicalCommitmentSubsetViolation {
                        index,
                        canonical: committed_canonical_output,
                        committed,
                    },
                );
            }

            let outcome_evidence_increment =
                self.evidence.positioned_commitment_amount(index).ok_or(
                    TypedFreezeQualificationError::OutcomeEvidenceCommitmentOverflow { index },
                )?;
            let published_outcome_evidence = entry
                .state
                .published_outcome_evidence
                .checked_add(outcome_evidence_increment)
                .ok_or(
                    TypedFreezeQualificationError::OutcomeEvidenceCommitmentOverflow { index },
                )?;
            if published_outcome_evidence > entry.policy.published_outcome_evidence_limit {
                return Err(
                    TypedFreezeQualificationError::OutcomeEvidenceCommitmentLimitExceeded {
                        index,
                        value: published_outcome_evidence,
                        limit: entry.policy.published_outcome_evidence_limit,
                    },
                );
            }
        }

        for (index, phase) in state.phase_states.iter().enumerate() {
            let expected = self
                .canonical
                .positioned_phase_bytes(index)
                .and_then(|value| value.checked_add(self.evidence.positioned_phase_bytes(index)?))
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_canonical_storage
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_evidence_storage_first
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_evidence_storage_second
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.discard_canonical
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.discard_evidence
                            .as_ref()
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
            if phase.current_live_bytes != expected {
                return Err(TypedFreezeQualificationError::PhaseLiveMismatch {
                    index,
                    expected,
                    actual: phase.current_live_bytes,
                });
            }
        }

        Ok(state.last_completed_phase.and_then(|completed| {
            state
                .phases
                .iter()
                .find(|entry| entry.state_position == completed.state_position)
                .map(|entry| entry.phase)
        }))
    }

    #[allow(
        clippy::result_large_err,
        reason = "qualification rejection must return the intact linear proof without allocation or indirection"
    )]
    fn qualify_terminal(self) -> Result<QualifiedTypedFreezeProof<O, P, D>, Self> {
        let qualification = {
            let mut state = lock(&self.ledger.state);
            self.validate_terminal(&state).inspect(|_| {
                state.publication = PublicationState::TerminalQualified;
            })
        };
        let last_completed_phase = match qualification {
            Ok(last_completed_phase) => last_completed_phase,
            Err(_) => return Err(self),
        };
        Ok(QualifiedTypedFreezeProof {
            proof: Some(self),
            last_completed_phase,
        })
    }

    #[allow(
        clippy::result_large_err,
        reason = "qualification rejection must return the intact linear proof without allocation or indirection"
    )]
    pub(crate) fn try_into_qualified_evidence_discard(
        self,
    ) -> Result<QualifiedEvidenceDiscardFreezeProof<O, P, D>, Self> {
        if self.discard_canonical.is_some() || self.discard_evidence.is_none() {
            return Err(self);
        }
        let mut qualified = self.qualify_terminal()?;
        let discard_evidence = match qualified
            .proof
            .as_mut()
            .and_then(|proof| proof.discard_evidence.take())
        {
            Some(discard_evidence) => discard_evidence,
            None => unreachable!("prequalified evidence discard disappeared"),
        };
        Ok(QualifiedEvidenceDiscardFreezeProof {
            proof: Some(qualified),
            discard_evidence: Some(discard_evidence),
            acknowledgement: QualifiedDiscardAcknowledgement::NotStarted,
        })
    }

    #[allow(
        clippy::result_large_err,
        reason = "qualification rejection must return the intact linear proof without allocation or indirection"
    )]
    pub(crate) fn try_into_qualified_discards(
        self,
    ) -> Result<QualifiedTypedDiscardFreezeProof<O, P, D>, Self> {
        let mut qualified = self.qualify_terminal()?;
        let typed = qualified
            .proof
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified typed proof disappeared"));
        let discard_canonical = typed.discard_canonical.take();
        let discard_evidence = typed.discard_evidence.take();
        Ok(match (discard_canonical, discard_evidence) {
            (None, None) => QualifiedTypedDiscardFreezeProof::None(FreezeReadyTypedProof {
                proof: Some(qualified),
            }),
            (Some(discard_canonical), None) => {
                QualifiedTypedDiscardFreezeProof::Canonical(QualifiedCanonicalDiscardFreezeProof {
                    proof: Some(qualified),
                    discard_canonical: Some(discard_canonical),
                    acknowledgement: QualifiedDiscardAcknowledgement::NotStarted,
                })
            }
            (None, Some(discard_evidence)) => {
                QualifiedTypedDiscardFreezeProof::Evidence(QualifiedEvidenceDiscardFreezeProof {
                    proof: Some(qualified),
                    discard_evidence: Some(discard_evidence),
                    acknowledgement: QualifiedDiscardAcknowledgement::NotStarted,
                })
            }
            (Some(discard_canonical), Some(discard_evidence)) => {
                QualifiedTypedDiscardFreezeProof::Both(QualifiedBothDiscardFreezeProof {
                    proof: Some(qualified),
                    discard_canonical: Some(discard_canonical),
                    discard_evidence: Some(discard_evidence),
                    acknowledgement: QualifiedDiscardAcknowledgement::NotStarted,
                })
            }
        })
    }

    pub(crate) fn retain_fail_closed(self) {
        std::mem::forget(self);
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D> QualifiedTypedDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn dimension_count(&self) -> usize {
        match self {
            Self::None(proof) => proof.dimension_count(),
            Self::Canonical(proof) => proof.dimension_count(),
            Self::Evidence(proof) => proof.dimension_count(),
            Self::Both(proof) => proof.dimension_count(),
        }
    }

    pub(crate) fn dimension_at(&self, index: usize) -> Option<D> {
        match self {
            Self::None(proof) => proof.dimension_at(index),
            Self::Canonical(proof) => proof.dimension_at(index),
            Self::Evidence(proof) => proof.dimension_at(index),
            Self::Both(proof) => proof.dimension_at(index),
        }
    }

    pub(crate) fn phase_count(&self) -> usize {
        match self {
            Self::None(proof) => proof.phase_count(),
            Self::Canonical(proof) => proof.phase_count(),
            Self::Evidence(proof) => proof.phase_count(),
            Self::Both(proof) => proof.phase_count(),
        }
    }

    pub(crate) fn phase_at(&self, index: usize) -> Option<P> {
        match self {
            Self::None(proof) => proof.phase_at(index),
            Self::Canonical(proof) => proof.phase_at(index),
            Self::Evidence(proof) => proof.phase_at(index),
            Self::Both(proof) => proof.phase_at(index),
        }
    }

    pub(crate) fn last_completed_phase(&self) -> Option<P> {
        match self {
            Self::None(proof) => proof.last_completed_phase(),
            Self::Canonical(proof) => proof.last_completed_phase(),
            Self::Evidence(proof) => proof.last_completed_phase(),
            Self::Both(proof) => proof.last_completed_phase(),
        }
    }

    pub(crate) fn retain_fail_closed(self) {
        match self {
            Self::None(proof) => proof.retain_fail_closed(),
            Self::Canonical(proof) => proof.retain_fail_closed(),
            Self::Evidence(proof) => proof.retain_fail_closed(),
            Self::Both(proof) => proof.retain_fail_closed(),
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D> QualifiedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn dimension_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, |proof| lock(&proof.ledger.state).dimensions.len())
    }

    fn dimension_at(&self, index: usize) -> Option<D> {
        self.proof.as_ref().and_then(|proof| {
            lock(&proof.ledger.state)
                .dimensions
                .get(index)
                .map(|entry| entry.dimension)
        })
    }

    fn phase_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, |proof| lock(&proof.ledger.state).phases.len())
    }

    fn phase_at(&self, index: usize) -> Option<P> {
        self.proof.as_ref().and_then(|proof| {
            lock(&proof.ledger.state)
                .phases
                .get(index)
                .map(|entry| entry.phase)
        })
    }

    const fn last_completed_phase(&self) -> Option<P> {
        self.last_completed_phase
    }

    fn retain_fail_closed(self) {
        std::mem::forget(self);
    }

    fn freeze_in_place(&mut self) -> LedgerSnapshot<P, D> {
        let last_completed_phase = self.last_completed_phase;
        let proof = self
            .proof
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified proof disappeared"));
        drop(proof.discard_canonical.take());
        drop(proof.discard_evidence.take());
        let deferred_canonical_storage = proof.deferred_canonical_storage.take();
        let deferred_evidence_storage_first = proof.deferred_evidence_storage_first.take();
        let deferred_evidence_storage_second = proof.deferred_evidence_storage_second.take();

        let snapshot = {
            let mut state = lock(&proof.ledger.state);
            release_deferred_typed_storage_locked(
                deferred_canonical_storage,
                deferred_evidence_storage_first,
                deferred_evidence_storage_second,
                &mut state,
            );
            proof.canonical.release_bound_attributions(&mut state);
            proof.evidence.release_bound_attributions(&mut state);
            apply_typed_commitments(&mut state.dimensions, &proof.canonical);
            apply_typed_commitments(&mut state.dimensions, &proof.evidence);
            proof
                .canonical
                .destroy_storage_then_release_bound_cover(&mut state);
            proof
                .evidence
                .destroy_storage_then_release_bound_cover(&mut state);
            state.publication = PublicationState::Published;
            take_typed_final_snapshot(&mut state, last_completed_phase)
        };
        drop(self.proof.take());
        snapshot
    }

    fn freeze(mut self) -> LedgerSnapshot<P, D> {
        self.freeze_in_place()
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D> QualifiedEvidenceDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn dimension_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, QualifiedTypedFreezeProof::dimension_count)
    }

    pub(crate) fn dimension_at(&self, index: usize) -> Option<D> {
        self.proof
            .as_ref()
            .and_then(|proof| proof.dimension_at(index))
    }

    pub(crate) fn phase_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, QualifiedTypedFreezeProof::phase_count)
    }

    pub(crate) fn phase_at(&self, index: usize) -> Option<P> {
        self.proof.as_ref().and_then(|proof| proof.phase_at(index))
    }

    pub(crate) const fn last_completed_phase(&self) -> Option<P> {
        match self.proof.as_ref() {
            Some(proof) => proof.last_completed_phase(),
            None => None,
        }
    }

    pub(crate) fn retain_fail_closed(self) {
        std::mem::forget(self);
    }

    pub(crate) fn release_discard_after_physical_cleanup_in_place(
        &mut self,
    ) -> FreezeReadyTypedProof<O, P, D> {
        self.acknowledgement = QualifiedDiscardAcknowledgement::InProgress;
        let proof = self
            .proof
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified proof owner disappeared"));
        let discard_evidence = self
            .discard_evidence
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified discard owner disappeared"));
        let typed = proof
            .proof
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified typed proof disappeared"));
        let deferred_canonical_storage = typed.deferred_canonical_storage.take();
        let deferred_evidence_storage_first = typed.deferred_evidence_storage_first.take();
        let deferred_evidence_storage_second = typed.deferred_evidence_storage_second.take();
        {
            let mut state = lock(&typed.ledger.state);
            discard_evidence.release_bound_attributions(&mut state);
            discard_evidence.destroy_storage_then_release_bound_cover(&mut state);
            release_deferred_typed_storage_locked(
                deferred_canonical_storage,
                deferred_evidence_storage_first,
                deferred_evidence_storage_second,
                &mut state,
            );
        }
        drop(self.discard_evidence.take());
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable!("acknowledged proof owner disappeared"));
        FreezeReadyTypedProof { proof: Some(proof) }
    }

    pub(crate) fn release_discard_after_physical_cleanup(
        mut self,
    ) -> FreezeReadyTypedProof<O, P, D> {
        self.release_discard_after_physical_cleanup_in_place()
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D> QualifiedCanonicalDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn dimension_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, QualifiedTypedFreezeProof::dimension_count)
    }

    pub(crate) fn dimension_at(&self, index: usize) -> Option<D> {
        self.proof
            .as_ref()
            .and_then(|proof| proof.dimension_at(index))
    }

    pub(crate) fn phase_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, QualifiedTypedFreezeProof::phase_count)
    }

    pub(crate) fn phase_at(&self, index: usize) -> Option<P> {
        self.proof.as_ref().and_then(|proof| proof.phase_at(index))
    }

    pub(crate) fn last_completed_phase(&self) -> Option<P> {
        self.proof
            .as_ref()
            .and_then(QualifiedTypedFreezeProof::last_completed_phase)
    }

    pub(crate) fn release_discard_after_physical_cleanup_in_place(
        &mut self,
    ) -> FreezeReadyTypedProof<O, P, D> {
        self.acknowledgement = QualifiedDiscardAcknowledgement::InProgress;
        let proof = self
            .proof
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified proof owner disappeared"));
        let discard = self
            .discard_canonical
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified canonical discard disappeared"));
        release_qualified_discard_after_physical_cleanup(proof, discard);
        drop(self.discard_canonical.take());
        FreezeReadyTypedProof {
            proof: self.proof.take(),
        }
    }

    pub(crate) fn release_discard_after_physical_cleanup(
        mut self,
    ) -> FreezeReadyTypedProof<O, P, D> {
        self.release_discard_after_physical_cleanup_in_place()
    }

    pub(crate) fn retain_fail_closed(self) {
        std::mem::forget(self);
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D> QualifiedBothDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn dimension_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, QualifiedTypedFreezeProof::dimension_count)
    }

    pub(crate) fn dimension_at(&self, index: usize) -> Option<D> {
        self.proof
            .as_ref()
            .and_then(|proof| proof.dimension_at(index))
    }

    pub(crate) fn phase_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, QualifiedTypedFreezeProof::phase_count)
    }

    pub(crate) fn phase_at(&self, index: usize) -> Option<P> {
        self.proof.as_ref().and_then(|proof| proof.phase_at(index))
    }

    pub(crate) fn last_completed_phase(&self) -> Option<P> {
        self.proof
            .as_ref()
            .and_then(QualifiedTypedFreezeProof::last_completed_phase)
    }

    pub(crate) fn release_canonical_after_physical_cleanup_in_place(
        &mut self,
    ) -> QualifiedEvidenceDiscardFreezeProof<O, P, D> {
        self.acknowledgement = QualifiedDiscardAcknowledgement::InProgress;
        let proof = self
            .proof
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified proof owner disappeared"));
        let discard = self
            .discard_canonical
            .as_mut()
            .unwrap_or_else(|| unreachable!("qualified canonical discard disappeared"));
        release_qualified_discard_after_physical_cleanup(proof, discard);
        drop(self.discard_canonical.take());
        QualifiedEvidenceDiscardFreezeProof {
            proof: self.proof.take(),
            discard_evidence: self.discard_evidence.take(),
            acknowledgement: QualifiedDiscardAcknowledgement::NotStarted,
        }
    }

    pub(crate) fn release_canonical_after_physical_cleanup(
        mut self,
    ) -> QualifiedEvidenceDiscardFreezeProof<O, P, D> {
        self.release_canonical_after_physical_cleanup_in_place()
    }

    pub(crate) fn retain_fail_closed(self) {
        std::mem::forget(self);
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D> FreezeReadyTypedProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn dimension_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, QualifiedTypedFreezeProof::dimension_count)
    }

    pub(crate) fn dimension_at(&self, index: usize) -> Option<D> {
        self.proof
            .as_ref()
            .and_then(|proof| proof.dimension_at(index))
    }

    pub(crate) fn phase_count(&self) -> usize {
        self.proof
            .as_ref()
            .map_or(0, QualifiedTypedFreezeProof::phase_count)
    }

    pub(crate) fn phase_at(&self, index: usize) -> Option<P> {
        self.proof.as_ref().and_then(|proof| proof.phase_at(index))
    }

    pub(crate) fn last_completed_phase(&self) -> Option<P> {
        self.proof
            .as_ref()
            .and_then(QualifiedTypedFreezeProof::last_completed_phase)
    }

    pub(crate) fn freeze_in_place(&mut self) -> LedgerSnapshot<P, D> {
        let snapshot = self
            .proof
            .as_mut()
            .unwrap_or_else(|| unreachable!("freeze-ready proof disappeared"))
            .freeze_in_place();
        drop(self.proof.take());
        snapshot
    }

    pub(crate) fn freeze(mut self) -> LedgerSnapshot<P, D> {
        self.freeze_in_place()
    }

    pub(crate) fn retain_fail_closed(self) {
        std::mem::forget(self);
    }
}

fn release_qualified_discard_after_physical_cleanup<O, P, D, Destination>(
    proof: &mut QualifiedTypedFreezeProof<O, P, D>,
    discard: &mut AttributionReservationBatch<O, P, D, Destination>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    let typed = proof
        .proof
        .as_mut()
        .unwrap_or_else(|| unreachable!("qualified typed proof disappeared"));
    let mut state = lock(&typed.ledger.state);
    discard.release_bound_attributions(&mut state);
    discard.destroy_storage_then_release_bound_cover(&mut state);
}

impl<O, P, D> Drop for QualifiedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(proof) = self.proof.take() {
            std::mem::forget(proof);
        }
    }
}

impl<O, P, D> Drop for QualifiedEvidenceDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        if let Some(discard) = self.discard_evidence.take() {
            std::mem::forget(discard);
        }
    }
}

impl<O, P, D> Drop for QualifiedCanonicalDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        if let Some(discard) = self.discard_canonical.take() {
            std::mem::forget(discard);
        }
    }
}

impl<O, P, D> Drop for QualifiedBothDiscardFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        if let Some(discard) = self.discard_canonical.take() {
            std::mem::forget(discard);
        }
        if let Some(discard) = self.discard_evidence.take() {
            std::mem::forget(discard);
        }
    }
}

impl<O, P, D> Drop for FreezeReadyTypedProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
    }
}

fn release_deferred_typed_storage_locked<O, P, D>(
    deferred_canonical_storage: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    deferred_evidence_storage_first: Option<
        AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    >,
    deferred_evidence_storage_second: Option<
        AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    >,
    state: &mut LedgerState<P, D>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    if let Some(mut storage) = deferred_canonical_storage {
        storage.destroy_storage_then_release_bound_cover(state);
    }
    if let Some(mut storage) = deferred_evidence_storage_first {
        storage.destroy_storage_then_release_bound_cover(state);
    }
    if let Some(mut storage) = deferred_evidence_storage_second {
        storage.destroy_storage_then_release_bound_cover(state);
    }
}

/// Live reservations submitted as one publication transaction.
///
/// A reservation that accounts for the backing allocation of `reservations` must remain outside
/// that vector. This type preserves the required storage-before-reservation drop order on
/// admission failure, rollback, and successful publication.
#[must_use = "submit the batch for publication or drop it to release its reservations"]
pub struct PublicationReservationBatch<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    reservations: Vec<LiveReservation<O, P, D>>,
    capacity_reservation: Option<LiveReservation<O, P, D>>,
}

/// Why a reservation batch could not prove coverage of its backing allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PublicationReservationBatchErrorKind {
    CapacityOverflow {
        capacity: usize,
        element_size: usize,
    },
    MissingCapacityReservation {
        required_bytes: u64,
    },
    UnexpectedCapacityReservation {
        reserved_bytes: u64,
    },
    InsufficientCapacityReservation {
        required_bytes: u64,
        reserved_bytes: u64,
    },
}

impl fmt::Display for PublicationReservationBatchErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityOverflow {
                capacity,
                element_size,
            } => write!(
                formatter,
                "reservation batch capacity {capacity} at {element_size} bytes per element overflowed"
            ),
            Self::MissingCapacityReservation { required_bytes } => write!(
                formatter,
                "reservation batch allocation requires a {required_bytes}-byte capacity reservation"
            ),
            Self::UnexpectedCapacityReservation { reserved_bytes } => write!(
                formatter,
                "an allocation-free reservation batch carried an unexpected {reserved_bytes}-byte capacity reservation"
            ),
            Self::InsufficientCapacityReservation {
                required_bytes,
                reserved_bytes,
            } => write!(
                formatter,
                "reservation batch allocation requires {required_bytes} bytes but its capacity reservation covers {reserved_bytes}"
            ),
        }
    }
}

impl std::error::Error for PublicationReservationBatchErrorKind {}

/// A rejected batch that keeps its backing allocation ahead of its capacity token in drop order.
#[must_use = "recover or drop the rejected batch so every reservation is released"]
pub struct PublicationReservationBatchError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    reservations: Vec<LiveReservation<O, P, D>>,
    capacity_reservation: Option<LiveReservation<O, P, D>>,
    kind: PublicationReservationBatchErrorKind,
}

impl<O, P, D> PublicationReservationBatchError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub const fn kind(&self) -> PublicationReservationBatchErrorKind {
        self.kind
    }
}

impl<O, P, D> Drop for PublicationReservationBatchError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        drop(std::mem::take(&mut self.reservations));
        drop(self.capacity_reservation.take());
    }
}

impl<O, P, D> fmt::Debug for PublicationReservationBatchError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicationReservationBatchError")
            .field("reservation_count", &self.reservations.len())
            .field("reservation_capacity", &self.reservations.capacity())
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl<O, P, D> fmt::Display for PublicationReservationBatchError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(formatter)
    }
}

impl<O, P, D> std::error::Error for PublicationReservationBatchError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
}

impl<O, P, D> PublicationReservationBatch<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    /// Checks that the supplied token covers the complete reservations-vector allocation.
    pub fn try_new(
        reservations: Vec<LiveReservation<O, P, D>>,
        capacity_reservation: Option<LiveReservation<O, P, D>>,
    ) -> Result<Self, PublicationReservationBatchError<O, P, D>> {
        let Some(required_bytes) = reservations
            .capacity()
            .checked_mul(size_of::<LiveReservation<O, P, D>>())
            .and_then(|bytes| u64::try_from(bytes).ok())
        else {
            return Err(PublicationReservationBatchError {
                kind: PublicationReservationBatchErrorKind::CapacityOverflow {
                    capacity: reservations.capacity(),
                    element_size: size_of::<LiveReservation<O, P, D>>(),
                },
                reservations,
                capacity_reservation,
            });
        };
        let invalid = match (required_bytes, capacity_reservation.as_ref()) {
            (0, None) => None,
            (0, Some(capacity)) => Some(
                PublicationReservationBatchErrorKind::UnexpectedCapacityReservation {
                    reserved_bytes: capacity.bytes(),
                },
            ),
            (_, None) => Some(
                PublicationReservationBatchErrorKind::MissingCapacityReservation { required_bytes },
            ),
            (_, Some(capacity)) if capacity.bytes() < required_bytes => Some(
                PublicationReservationBatchErrorKind::InsufficientCapacityReservation {
                    required_bytes,
                    reserved_bytes: capacity.bytes(),
                },
            ),
            (_, Some(_)) => None,
        };
        if let Some(kind) = invalid {
            return Err(PublicationReservationBatchError {
                reservations,
                capacity_reservation,
                kind,
            });
        }
        Ok(Self {
            reservations,
            capacity_reservation,
        })
    }

    /// Returns whether publication has moved every reservation out of this batch.
    pub fn is_empty(&self) -> bool {
        self.reservations.is_empty() && self.capacity_reservation.is_none()
    }

    fn iter(&self) -> impl Iterator<Item = &LiveReservation<O, P, D>> {
        self.reservations
            .iter()
            .chain(self.capacity_reservation.iter())
    }

    fn release_locked(&mut self, state: &mut LedgerState<P, D>) {
        for reservation in &mut self.reservations {
            reservation.core.release_locked(state);
        }

        // Free the accounted vector allocation before releasing the token that covers it.
        drop(std::mem::take(&mut self.reservations));
        if let Some(capacity_reservation) = &mut self.capacity_reservation {
            capacity_reservation.core.release_locked(state);
        }
    }
}

impl<O, P, D> Default for PublicationReservationBatch<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn default() -> Self {
        Self {
            reservations: Vec::new(),
            capacity_reservation: None,
        }
    }
}

impl<O, P, D> fmt::Debug for PublicationReservationBatch<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord + fmt::Debug,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicationReservationBatch")
            .field("reservations", &self.reservations)
            .field("capacity_reservation", &self.capacity_reservation)
            .finish()
    }
}

/// A reversible publication transaction that still owns its live byte capacity.
#[must_use = "publish or roll back the pending output transaction"]
pub struct PendingPublication<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    owner: Arc<Mutex<LedgerState<P, D>>>,
    reservations: PublicationReservationBatch<O, P, D>,
    commitments: Vec<AggregatedCommitment<D>>,
    evidence_bytes: u64,
    pending: bool,
    operation_owner: PhantomData<fn() -> O>,
}

impl<O, P, D> fmt::Debug for PendingPublication<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord + fmt::Debug,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingPublication")
            .field("reservations", &self.reservations)
            .field("commitments", &self.commitments)
            .field("evidence_bytes", &self.evidence_bytes)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl<O, P, D> PendingPublication<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    /// Explicitly rolls back pending commitment headroom and releases all live reservations.
    pub fn rollback(self) {
        drop(self);
    }
}

impl<O, P, D> Drop for PendingPublication<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if !self.pending {
            return;
        }
        let mut state = lock(&self.owner);
        for commitment in &self.commitments {
            if let Some(entry) = state.dimensions.get_mut(commitment.dimension_position) {
                debug_assert!(entry.state.pending_publication >= commitment.amount);
                entry.state.pending_publication = entry
                    .state
                    .pending_publication
                    .saturating_sub(commitment.amount);
            }
        }
        self.reservations.release_locked(&mut state);
        if state.publication == PublicationState::Pending {
            state.publication = PublicationState::Open;
        }
        drop(std::mem::take(&mut self.commitments));
        release_evidence(&mut state, self.evidence_bytes);
        self.evidence_bytes = 0;
        self.pending = false;
    }
}

fn lock<P, D>(state: &Arc<Mutex<LedgerState<P, D>>>) -> MutexGuard<'_, LedgerState<P, D>> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn release_positioned_locked<P, D>(
    state: &mut LedgerState<P, D>,
    phase_position: usize,
    dimension_position: usize,
    bytes: u64,
) {
    state.active_reservations = state.active_reservations.saturating_sub(1);
    state.current_live_bytes = state.current_live_bytes.saturating_sub(bytes);
    if let Some(entry) = state.dimensions.get_mut(dimension_position) {
        entry.state.current_live_bytes = entry.state.current_live_bytes.saturating_sub(bytes);
    }
    if let Some(phase_state) = state.phase_states.get_mut(phase_position) {
        phase_state.current_live_bytes = phase_state.current_live_bytes.saturating_sub(bytes);
    }
}

fn release_positioned_bytes_locked<P, D>(
    state: &mut LedgerState<P, D>,
    phase_position: usize,
    dimension_position: usize,
    bytes: u64,
) {
    state.current_live_bytes = state.current_live_bytes.saturating_sub(bytes);
    state.dimensions[dimension_position]
        .state
        .current_live_bytes = state.dimensions[dimension_position]
        .state
        .current_live_bytes
        .saturating_sub(bytes);
    state.phase_states[phase_position].current_live_bytes = state.phase_states[phase_position]
        .current_live_bytes
        .saturating_sub(bytes);
}

fn dimension_entry<D: Copy + Ord>(
    dimensions: &[DimensionEntry<D>],
    dimension: D,
) -> Option<&DimensionEntry<D>> {
    dimensions
        .binary_search_by_key(&dimension, |entry| entry.dimension)
        .ok()
        .map(|position| &dimensions[position])
}

fn dimension_entry_mut<D: Copy + Ord>(
    dimensions: &mut [DimensionEntry<D>],
    dimension: D,
) -> Option<&mut DimensionEntry<D>> {
    dimensions
        .binary_search_by_key(&dimension, |entry| entry.dimension)
        .ok()
        .map(|position| &mut dimensions[position])
}

fn validate_dimension_state<D: Copy>(
    dimension: D,
    policy: DimensionPolicy,
    state: DimensionState,
) -> Result<(), LedgerCause<D>> {
    for (class, value, limit) in [
        (
            CounterClass::Attempted,
            state.attempted,
            policy.attempted_limit,
        ),
        (
            CounterClass::Consumed,
            state.consumed,
            policy.consumed_limit,
        ),
    ] {
        if value > limit {
            return Err(LedgerCause::LimitExceeded {
                class,
                dimension,
                value,
                limit,
            });
        }
    }
    if let Some(limit) = policy.live_byte_limit
        && state.current_live_bytes > limit
    {
        return Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension,
            value: state.current_live_bytes,
            limit,
        });
    }
    let committed = state
        .committed
        .checked_add(state.pending_publication)
        .ok_or(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Committed,
            dimension,
        })?;
    if committed > policy.committed_limit {
        return Err(LedgerCause::LimitExceeded {
            class: CounterClass::Committed,
            dimension,
            value: committed,
            limit: policy.committed_limit,
        });
    }
    if state.published_outcome_evidence > policy.published_outcome_evidence_limit {
        return Err(LedgerCause::LimitExceeded {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension,
            value: state.published_outcome_evidence,
            limit: policy.published_outcome_evidence_limit,
        });
    }
    Ok(())
}

fn phase_entry<P: Copy + Ord>(phases: &[PhaseEntry<P>], phase: P) -> Option<&PhaseEntry<P>> {
    phases.iter().find(|entry| entry.phase == phase)
}

fn phase_state<P: Copy + Ord, D>(state: &LedgerState<P, D>, phase: P) -> Option<&PhaseState> {
    let position = phase_entry(&state.phases, phase)?.state_position;
    state.phase_states.get(position)
}

fn phase_position<P: Copy + Ord, D>(
    state: &LedgerState<P, D>,
    phase: P,
) -> Result<usize, LedgerCause<D>> {
    state
        .phases
        .iter()
        .find(|entry| entry.phase == phase)
        .map(|entry| entry.state_position)
        .ok_or(LedgerCause::UnknownPhase)
}

fn try_insert_phase_catalog_entry<P: Copy + Ord, D>(
    state: &mut LedgerState<P, D>,
    phase: P,
) -> Result<(), LedgerCause<D>> {
    if state.phases.iter().any(|entry| entry.phase == phase) {
        return Err(LedgerCause::DuplicatePhase);
    }
    let desired_phase_capacity = state.phases.len().saturating_add(1);
    try_grow_evidence_vec(
        &mut state.phases,
        desired_phase_capacity,
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Phases,
    )
    .map_err(|cause| LedgerCause::Evidence { cause })?;
    let desired_state_capacity = state.phase_states.len().saturating_add(1);
    try_grow_evidence_vec(
        &mut state.phase_states,
        desired_state_capacity,
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Phases,
    )
    .map_err(|cause| LedgerCause::Evidence { cause })?;
    let desired_staging_capacity = state.phase_snapshot_staging.len().saturating_add(1);
    try_grow_evidence_vec(
        &mut state.phase_snapshot_staging,
        desired_staging_capacity,
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Phases,
    )
    .map_err(|cause| LedgerCause::Evidence { cause })?;
    let state_position = state.phase_states.len();
    state.phase_states.push(PhaseState::default());
    state.phases.push(PhaseEntry {
        phase,
        state_position,
    });
    state
        .phase_snapshot_staging
        .push((phase, PhaseSnapshot::default()));
    Ok(())
}

fn try_grow_evidence_vec<T>(
    values: &mut Vec<T>,
    desired_capacity: usize,
    current_evidence_bytes: &mut u64,
    peak_evidence_bytes: &mut u64,
    evidence_byte_limit: u64,
    purpose: LedgerEvidencePurpose,
) -> Result<(), LedgerEvidenceError> {
    try_grow_evidence_vec_impl(
        values,
        desired_capacity,
        None,
        current_evidence_bytes,
        peak_evidence_bytes,
        evidence_byte_limit,
        purpose,
    )
}

#[allow(clippy::too_many_arguments)]
fn try_grow_tracked_evidence_vec<T>(
    values: &mut Vec<T>,
    desired_capacity: usize,
    retained_bytes: &mut u64,
    current_evidence_bytes: &mut u64,
    peak_evidence_bytes: &mut u64,
    evidence_byte_limit: u64,
    purpose: LedgerEvidencePurpose,
) -> Result<(), LedgerEvidenceError> {
    let owned_capacity_bytes =
        capacity_bytes(values).map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    if owned_capacity_bytes != *retained_bytes {
        return Err(LedgerEvidenceError::TrackedStorageCoverageMismatch {
            purpose,
            retained_bytes: *retained_bytes,
            capacity_bytes: owned_capacity_bytes,
        });
    }
    try_grow_evidence_vec_impl(
        values,
        desired_capacity,
        Some(retained_bytes),
        current_evidence_bytes,
        peak_evidence_bytes,
        evidence_byte_limit,
        purpose,
    )
}

#[allow(clippy::too_many_arguments)]
fn try_grow_evidence_vec_impl<T>(
    values: &mut Vec<T>,
    desired_capacity: usize,
    retained_bytes: Option<&mut u64>,
    current_evidence_bytes: &mut u64,
    peak_evidence_bytes: &mut u64,
    evidence_byte_limit: u64,
    purpose: LedgerEvidencePurpose,
) -> Result<(), LedgerEvidenceError> {
    let original_len = values.len();
    let original_capacity = values.capacity();
    let original_current = *current_evidence_bytes;
    let source_plan =
        CapacityPlan::for_type::<T>(original_len, original_capacity, desired_capacity)
            .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    source_plan
        .verify_preflight(original_len, original_capacity)
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;

    // Existing spare capacity already satisfies the request. Keep the original owner untouched;
    // in particular, do not enter the allocator or change any accounting state.
    if source_plan.desired_capacity() <= original_capacity {
        source_plan
            .finish(original_len, original_capacity)
            .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
        return Ok(());
    }

    // A replacement backing is live alongside the old backing until the owner swap completes.
    // Admit the complete candidate envelope, not merely the eventual capacity delta, so a failed
    // allocation cannot create an uncharged transient peak.
    let original_bytes =
        capacity_bytes(values).map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    let current_without_original =
        original_current
            .checked_sub(original_bytes)
            .ok_or(LedgerEvidenceError::Capacity {
                purpose,
                cause: CapacityPlanError::CapacityOverflow,
            })?;
    let replacement_plan = ReplacementCapacityPlan::for_type::<T>(
        original_len,
        original_capacity,
        source_plan.desired_capacity(),
    )
    .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    replacement_plan
        .verify_source(original_len, original_capacity)
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    let candidate_plan = replacement_plan.candidate();
    candidate_plan
        .verify_preflight(0, 0)
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    let replacement_envelope_bytes = replacement_plan
        .candidate_envelope_bytes()
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    let admitted = original_current
        .checked_add(replacement_envelope_bytes)
        .ok_or(LedgerEvidenceError::Capacity {
            purpose,
            cause: CapacityPlanError::CapacityOverflow,
        })?;
    if admitted > evidence_byte_limit {
        return Err(LedgerEvidenceError::LimitExceeded {
            purpose,
            value: admitted,
            limit: evidence_byte_limit,
        });
    }
    let final_current_upper_bound = current_without_original
        .checked_add(replacement_envelope_bytes)
        .ok_or(LedgerEvidenceError::Capacity {
            purpose,
            cause: CapacityPlanError::CapacityOverflow,
        })?;

    *current_evidence_bytes = admitted;
    *peak_evidence_bytes = (*peak_evidence_bytes).max(admitted);

    let mut replacement = Vec::new();
    #[cfg(test)]
    if test_take_evidence_allocation_failure() {
        drop(replacement);
        *current_evidence_bytes = original_current;
        return Err(LedgerEvidenceError::AllocationFailed {
            purpose,
            requested_bytes: replacement_envelope_bytes,
        });
    }
    if replacement
        .try_reserve_exact(candidate_plan.allocator_additional())
        .is_err()
    {
        drop(replacement);
        *current_evidence_bytes = original_current;
        return Err(LedgerEvidenceError::AllocationFailed {
            purpose,
            requested_bytes: replacement_envelope_bytes,
        });
    }

    #[cfg(test)]
    let observed_capacity = if test_take_evidence_over_envelope_failure() {
        candidate_plan.envelope_capacity().saturating_add(1)
    } else {
        replacement.capacity()
    };
    #[cfg(not(test))]
    let observed_capacity = replacement.capacity();
    let actual_bytes = match candidate_plan.finish(0, observed_capacity) {
        Ok(bytes) => bytes,
        Err(cause) => {
            drop(replacement);
            *current_evidence_bytes = original_current;
            return Err(LedgerEvidenceError::Capacity { purpose, cause });
        }
    };
    debug_assert!(actual_bytes <= replacement_envelope_bytes);
    let unused_envelope_bytes = replacement_envelope_bytes - actual_bytes;
    let final_current = final_current_upper_bound - unused_envelope_bytes;

    // `replacement` has been fully admitted and qualified. Moving elements and swapping owners
    // are allocation-free; dropping the old backing happens before any accounting is reduced.
    replacement.append(values);
    std::mem::swap(values, &mut replacement);
    drop(replacement);
    *current_evidence_bytes = final_current;
    if let Some(retained_bytes) = retained_bytes {
        *retained_bytes = actual_bytes;
    }
    Ok(())
}

fn capacity_bytes<T>(values: &Vec<T>) -> Result<u64, CapacityPlanError> {
    let bytes = values
        .capacity()
        .checked_mul(size_of::<T>())
        .ok_or(CapacityPlanError::CapacityOverflow)?;
    u64::try_from(bytes).map_err(|_| CapacityPlanError::CapacityOverflow)
}

fn release_evidence<P, D>(state: &mut LedgerState<P, D>, bytes: u64) {
    state.current_evidence_bytes = state.current_evidence_bytes.saturating_sub(bytes);
}

enum AggregateCommitmentError<D> {
    Ledger(LedgerCause<D>),
    Evidence(LedgerEvidenceError),
}

struct CommitmentAggregate<D> {
    entries: Vec<AggregatedCommitment<D>>,
    evidence_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AggregatedCommitment<D> {
    dimension: D,
    dimension_position: usize,
    amount: u64,
}

struct CommitmentAggregateAssembly<'a, P, D> {
    state: &'a mut LedgerState<P, D>,
    entries: Vec<AggregatedCommitment<D>>,
    evidence_bytes: u64,
    armed: bool,
}

impl<P, D> CommitmentAggregateAssembly<'_, P, D> {
    fn finish(mut self) -> CommitmentAggregate<D> {
        self.armed = false;
        CommitmentAggregate {
            entries: std::mem::take(&mut self.entries),
            evidence_bytes: self.evidence_bytes,
        }
    }
}

impl<P, D> Drop for CommitmentAggregateAssembly<'_, P, D> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        drop(std::mem::take(&mut self.entries));
        release_evidence(self.state, self.evidence_bytes);
    }
}

fn aggregate_commitments<P, D>(
    state: &mut LedgerState<P, D>,
    commitments: &[Commitment<D>],
) -> Result<CommitmentAggregate<D>, AggregateCommitmentError<D>>
where
    D: Copy + Ord,
{
    let mut aggregate = Vec::new();
    let evidence_before = state.current_evidence_bytes;
    if let Err(cause) = try_grow_evidence_vec(
        &mut aggregate,
        commitments.len(),
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Publication,
    ) {
        let retained = capacity_bytes(&aggregate).unwrap_or(0);
        drop(aggregate);
        release_evidence(state, retained);
        return Err(AggregateCommitmentError::Evidence(cause));
    }
    let evidence_bytes = state.current_evidence_bytes.saturating_sub(evidence_before);
    let mut assembly = CommitmentAggregateAssembly {
        state,
        entries: aggregate,
        evidence_bytes,
        armed: true,
    };
    for commitment in commitments {
        let dimension_position = assembly
            .state
            .dimensions
            .binary_search_by_key(&commitment.dimension, |entry| entry.dimension)
            .map_err(|_| {
                AggregateCommitmentError::Ledger(LedgerCause::UnknownDimension {
                    dimension: commitment.dimension,
                })
            })?;
        match assembly
            .entries
            .binary_search_by_key(&dimension_position, |entry| entry.dimension_position)
        {
            Ok(position) => {
                let Some(amount) = assembly.entries[position]
                    .amount
                    .checked_add(commitment.amount)
                else {
                    return Err(AggregateCommitmentError::Ledger(
                        LedgerCause::ArithmeticOverflow {
                            class: CounterClass::PendingPublication,
                            dimension: commitment.dimension,
                        },
                    ));
                };
                assembly.entries[position].amount = amount;
            }
            Err(position) => assembly.entries.insert(
                position,
                AggregatedCommitment {
                    dimension: commitment.dimension,
                    dimension_position,
                    amount: commitment.amount,
                },
            ),
        }
    }
    Ok(assembly.finish())
}

fn validate_commitments<D: Copy>(
    dimensions: &[DimensionEntry<D>],
    commitments: &[AggregatedCommitment<D>],
) -> Result<(), LedgerCause<D>> {
    for commitment in commitments {
        let entry = dimensions
            .get(commitment.dimension_position)
            .ok_or(LedgerCause::FinalSnapshotShapeMismatch)?;
        let admitted = entry
            .state
            .committed
            .checked_add(entry.state.pending_publication)
            .and_then(|value| value.checked_add(commitment.amount))
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: CounterClass::PendingPublication,
                dimension: commitment.dimension,
            })?;
        if admitted > entry.policy.committed_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Committed,
                dimension: commitment.dimension,
                value: admitted,
                limit: entry.policy.committed_limit,
            });
        }
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn validate_typed_commitments<O, P, D, Destination>(
    dimensions: &[DimensionEntry<D>],
    batch: &AttributionReservationBatch<O, P, D, Destination>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    validate_typed_commitment_batches(dimensions, [Some(batch)])
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn validate_typed_commitment_pair<O, P, D, Destination>(
    dimensions: &[DimensionEntry<D>],
    first: &AttributionReservationBatch<O, P, D, Destination>,
    second: &AttributionReservationBatch<O, P, D, Destination>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    validate_typed_commitment_batches(dimensions, [Some(first), Some(second)])
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn validate_typed_commitment_batches<O, P, D, Destination, const N: usize>(
    dimensions: &[DimensionEntry<D>],
    batches: [Option<&AttributionReservationBatch<O, P, D, Destination>>; N],
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    let all = || {
        batches
            .into_iter()
            .flatten()
            .flat_map(|batch| batch.commitments.iter())
    };
    for (index, commitment) in all().enumerate() {
        if all()
            .take(index)
            .any(|prior| prior.dimension == commitment.dimension)
        {
            continue;
        }
        let amount = all()
            .filter(|candidate| candidate.dimension == commitment.dimension)
            .try_fold(0_u64, |total, candidate| {
                total.checked_add(candidate.amount)
            })
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: match Destination::KIND {
                    DestinationKind::CanonicalOutput => CounterClass::Committed,
                    DestinationKind::OutcomeEvidence => CounterClass::PublishedOutcomeEvidence,
                },
                dimension: commitment.dimension,
            })?;
        let entry = dimension_entry(dimensions, commitment.dimension).ok_or(
            LedgerCause::UnknownDimension {
                dimension: commitment.dimension,
            },
        )?;
        let (current, limit, class) = match Destination::KIND {
            DestinationKind::CanonicalOutput => (
                entry.state.committed,
                entry.policy.committed_limit,
                CounterClass::Committed,
            ),
            DestinationKind::OutcomeEvidence => (
                entry.state.published_outcome_evidence,
                entry.policy.published_outcome_evidence_limit,
                CounterClass::PublishedOutcomeEvidence,
            ),
        };
        let value = current
            .checked_add(amount)
            .ok_or(LedgerCause::ArithmeticOverflow {
                class,
                dimension: commitment.dimension,
            })?;
        if value > limit {
            return Err(LedgerCause::LimitExceeded {
                class,
                dimension: commitment.dimension,
                value,
                limit,
            });
        }
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn preflight_typed_publication_batch<O, P, D, Destination>(
    state: &LedgerState<P, D>,
    batch: &AttributionReservationBatch<O, P, D, Destination>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if !batch.bound_positions_are_in_bounds(state.dimensions.len(), state.phase_states.len()) {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    Ok(())
}

fn preflight_typed_discard_batch<O, P, D, Destination>(
    state: &LedgerState<P, D>,
    batch: &AttributionReservationBatch<O, P, D, Destination>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if !batch.discard_positions_are_in_bounds(state.dimensions.len(), state.phase_states.len()) {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn apply_typed_commitments<O, P, D, Destination>(
    dimensions: &mut [DimensionEntry<D>],
    batch: &AttributionReservationBatch<O, P, D, Destination>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    for commitment in &batch.commitments {
        let entry = &mut dimensions[commitment.dimension_position];
        match Destination::KIND {
            DestinationKind::CanonicalOutput => {
                entry.state.committed += commitment.amount;
                entry.state.committed_canonical_output += commitment.amount;
            }
            DestinationKind::OutcomeEvidence => {
                entry.state.published_outcome_evidence += commitment.amount;
            }
        }
    }
}

#[cfg(test)]
mod tests;
