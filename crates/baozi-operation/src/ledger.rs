use crate::{CapacityPlan, CapacityPlanError, OperationOwner, ReplacementCapacityPlan};
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::num::NonZeroU64;
#[cfg(test)]
use std::sync::Weak;
use std::sync::{Arc, Mutex, MutexGuard};

mod commitment;
mod evidence;
pub(crate) use evidence::ProgressEvidenceStorage;
mod exclusive;
pub(crate) use exclusive::{ExclusiveStorage, ExclusiveWork};
mod reservation;
mod snapshot;
mod startup;
mod work;

pub(crate) use work::{
    InternalWork, ParserChannel, ParserFrame, ParserReadRow, ParserWorkError, ReservedProducerRow,
    WorkFrame, WorkLane, WorkLayout, WorkView,
};
use work::{WorkDimensionState, WorkEntry, WorkGrant, WorkMode, WorkPolicy, WorkState};

use commitment::{
    AggregateCommitmentError, AggregatedCommitment, aggregate_commitments,
    preflight_typed_discard_batch, preflight_typed_publication_batch, validate_commitments,
    validate_typed_commitment_batches, validate_typed_commitment_pair, validate_typed_commitments,
};

pub use reservation::LiveReservation;
pub(crate) use reservation::{
    AttributionBatchPlan, AttributionReservationBatch, FallbackBaselineRejection,
    FallbackBatchBaseline, OperationAttributionReservation, RetainedStorageOperation,
    StorageCoveringReservation,
};
use reservation::{DetachedAttributionBatch, ReservationCore};

pub(crate) use snapshot::DimensionSnapshotScalars;
pub use snapshot::{DimensionSnapshot, LedgerSnapshot, PhaseSnapshot};
use snapshot::{
    dimension_snapshot, phase_catalog_shape_is_valid, phase_snapshot, take_final_snapshot,
    take_typed_terminal_projection, try_snapshot, typed_terminal_snapshot_positions_are_valid,
    validate_final_snapshot_shape,
};

#[allow(
    unused_imports,
    reason = "preserve the crate-private ledger path across the startup module extraction"
)]
pub(crate) use startup::{
    STARTUP_ACCOUNT_SHARE_STEP_COUNT, StartupAccountShareEnvelope, StartupAccountShareStep,
    StartupAttributionBatchPlans, StartupAttributionGraph, StartupLedgerGraphSeal,
    StartupLedgerPlanParts, StartupRetainedAttribution,
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
    static DETACHED_COVER_RELEASE_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static DETACHED_COVER_RELEASES_WERE_STORAGE_FIRST: std::cell::Cell<bool> = const {
        std::cell::Cell::new(true)
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

#[cfg(test)]
fn test_reset_detached_cover_release_observation() {
    DETACHED_COVER_RELEASE_COUNT.with(|count| count.set(0));
    DETACHED_COVER_RELEASES_WERE_STORAGE_FIRST.with(|storage_first| storage_first.set(true));
}

#[cfg(test)]
fn test_record_detached_cover_release(storage_first: bool) {
    DETACHED_COVER_RELEASE_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    if !storage_first {
        DETACHED_COVER_RELEASES_WERE_STORAGE_FIRST.with(|observed| observed.set(false));
    }
}

#[cfg(test)]
fn test_detached_cover_release_observation() -> (usize, bool) {
    (
        DETACHED_COVER_RELEASE_COUNT.with(std::cell::Cell::get),
        DETACHED_COVER_RELEASES_WERE_STORAGE_FIRST.with(std::cell::Cell::get),
    )
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
    /// Frozen work and storage rows no longer match their preadmitted snapshot buffers.
    SnapshotShapeMismatch,
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
            Self::SnapshotShapeMismatch => {
                formatter.write_str("ledger snapshot shape is inconsistent")
            }
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
            Self::SnapshotShapeMismatch
            | Self::PhaseCatalogShapeMismatch
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoragePolicy {
    committed_limit: u64,
    published_outcome_evidence_limit: u64,
    live_byte_limit: Option<u64>,
}

impl StoragePolicy {
    const fn from_policy(policy: DimensionPolicy) -> Self {
        Self {
            committed_limit: policy.committed_limit,
            published_outcome_evidence_limit: policy.published_outcome_evidence_limit,
            live_byte_limit: policy.live_byte_limit,
        }
    }

    const fn with_work(self, work: WorkPolicy) -> DimensionPolicy {
        DimensionPolicy {
            attempted_limit: work.attempted_limit,
            consumed_limit: work.consumed_limit,
            committed_limit: self.committed_limit,
            published_outcome_evidence_limit: self.published_outcome_evidence_limit,
            live_byte_limit: self.live_byte_limit,
        }
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
    policy: StoragePolicy,
    state: StorageDimensionState,
}

/// Frozen aggregate limits for one ledger instance.
#[derive(Debug, PartialEq, Eq)]
pub struct LedgerLimits<D> {
    dimensions: Vec<DimensionEntry<D>>,
    work_catalog: Vec<D>,
    work_dimensions: Vec<WorkEntry<D>>,
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
        let mut work_catalog = Vec::new();
        let mut work_dimensions = Vec::new();
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
            try_grow_evidence_vec(
                &mut work_catalog,
                desired_dimension_capacity,
                &mut current_evidence_bytes,
                &mut peak_evidence_bytes,
                evidence_byte_limit,
                LedgerEvidencePurpose::Dimensions,
            )
            .map_err(|cause| LedgerLimitsError::Evidence { cause })?;
            try_grow_evidence_vec(
                &mut work_dimensions,
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
                    policy: StoragePolicy::from_policy(policy),
                    state: StorageDimensionState::default(),
                },
            );
            work_catalog.insert(position, dimension);
            work_dimensions.insert(
                position,
                WorkEntry {
                    dimension,
                    policy: WorkPolicy {
                        attempted_limit: policy.attempted_limit,
                        consumed_limit: policy.consumed_limit,
                    },
                    state: WorkDimensionState::default(),
                },
            );
            staging.insert(position, (dimension, DimensionSnapshot::default()));
        }
        Ok(Self {
            dimensions: frozen,
            work_catalog,
            work_dimensions,
            dimension_snapshot_staging: staging,
            operation_live_byte_limit,
            evidence_byte_limit,
            current_evidence_bytes,
            peak_evidence_bytes,
        })
    }

    /// Returns one frozen dimension policy.
    pub fn dimension(&self, dimension: D) -> Option<DimensionPolicy> {
        let position = self.work_catalog.binary_search(&dimension).ok()?;
        Some(
            self.dimensions[position]
                .policy
                .with_work(self.work_dimensions[position].policy),
        )
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
    /// A linear work permit was completed, released, or otherwise resolved more than once.
    InactiveLinearWorkPermit,
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
    /// Final evidence was requested while linear work still reserved consumed capacity.
    OutstandingLinearWorkPermits { count: u64 },
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
            Self::InactiveLinearWorkPermit => {
                formatter.write_str("linear work permit is no longer active")
            }
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
            Self::OutstandingLinearWorkPermits { count } => {
                write!(formatter, "{count} linear work permits remain active")
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
    pub trait Sealed {
        const IS_OUTCOME_EVIDENCE: bool;
    }
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

impl destination_sealed::Sealed for CanonicalOutput {
    const IS_OUTCOME_EVIDENCE: bool = false;
}
impl destination_sealed::Sealed for OutcomeEvidenceDestination {
    const IS_OUTCOME_EVIDENCE: bool = true;
}

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
    pending_consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
    retained_output_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StorageDimensionState {
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
    retained_output_bytes: u64,
}

impl DimensionState {
    const fn join(work: WorkDimensionState, storage: StorageDimensionState) -> Self {
        Self {
            attempted: work.attempted,
            consumed: work.consumed,
            pending_consumed: work.pending_consumed,
            current_live_bytes: storage.current_live_bytes,
            peak_live_bytes: storage.peak_live_bytes,
            pending_publication: storage.pending_publication,
            committed: storage.committed,
            committed_canonical_output: storage.committed_canonical_output,
            published_outcome_evidence: storage.published_outcome_evidence,
            retained_output_bytes: storage.retained_output_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PhaseState {
    current_live_bytes: u64,
    peak_live_bytes: u64,
    retained_output_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
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
    retained_output_bytes: u64,
    retained_output_reservations: u64,
    evidence_byte_limit: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
    publication: PublicationState,
    last_completed_phase: Option<CompletedPhase<P>>,
}

struct ReservationPositions<D> {
    dimension: Result<usize, LedgerCause<D>>,
    phase: Result<usize, LedgerCause<D>>,
}

struct ReservationProjection {
    dimension_position: usize,
    dimension_live: u64,
    operation_live: u64,
    phase_position: usize,
    phase_live: u64,
    active_reservations: u64,
}

#[derive(Clone, Copy)]
enum StorageAdmission {
    Ordinary,
    ProtectedEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationState {
    Open,
    Closing,
    Pending,
    TerminalQualified,
    Published,
}

#[derive(Clone, Copy)]
enum FallbackBaselineQualification {
    Attachment,
    Terminal,
}

impl FallbackBaselineQualification {
    fn accepts_coverage(self, actual: u64, expected: u64) -> bool {
        match self {
            Self::Attachment => actual >= expected,
            Self::Terminal => actual == expected,
        }
    }
}

/// Shared accounting state branded by one operation domain.
pub(crate) struct Ledger<O, P, D> {
    work_catalog: Vec<D>,
    work: WorkState<D>,
    phase_catalogue: Vec<PhaseEntry<P>>,
    state: Arc<Mutex<LedgerState<P, D>>>,
    operation_live_byte_limit: u64,
    operation_owner: PhantomData<fn() -> O>,
}

/// Allocation-free scalar observation used only to prove scoped planning cleanup.
/// Read-only storage evidence borrowed from one local invocation, with no shared ownership.
#[derive(Clone, Copy)]
pub(crate) struct LedgerStorageObservation<'op, P, D> {
    state: &'op Mutex<LedgerState<P, D>>,
    position: usize,
    local: PhantomData<&'op std::cell::Cell<()>>,
}

impl<P, D> LedgerStorageObservation<'_, P, D> {
    pub(crate) fn live_bytes(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .dimensions[self.position]
            .state
            .current_live_bytes
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LedgerPlanningState {
    current_live_bytes: u64,
    active_reservations: u64,
}

impl LedgerPlanningState {
    pub(crate) const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    pub(crate) const fn active_reservations(self) -> u64 {
        self.active_reservations
    }
}

/// Result of transferring one completed work prefix out of a linear permit.
///
/// A ledger limit is checked against both public consumed evidence and the still-pending suffix.
/// The check can therefore fail after the requested prefix has already completed in the caller.
/// That prefix remains non-refundable and this result tells wrapper permits whether their mirrored
/// remaining count must advance even though the ledger cause remains terminal.
#[must_use = "linear work completion must preserve both progress and its ledger outcome"]
pub(crate) enum LinearWorkCompletion<D> {
    Completed,
    CompletedWithLedgerFailure(LedgerCause<D>),
    NotCompleted(LedgerCause<D>),
}

impl<D> LinearWorkCompletion<D> {
    pub(crate) fn into_result(self) -> Result<(), LedgerCause<D>> {
        match self {
            Self::Completed => Ok(()),
            Self::CompletedWithLedgerFailure(cause) | Self::NotCompleted(cause) => Err(cause),
        }
    }
}

/// Test-only scalar observation of one ledger's live reservation state.
#[cfg(test)]
pub(crate) struct LedgerTestProbe<P, D> {
    state: Weak<Mutex<LedgerState<P, D>>>,
}

#[cfg(test)]
impl<P, D> LedgerTestProbe<P, D> {
    pub(crate) fn is_available(&self) -> bool {
        self.state.strong_count() != 0
    }

    pub(crate) fn active_reservations(&self) -> u64 {
        self.state
            .upgrade()
            .map_or(0, |state| lock(&state).active_reservations)
    }

    pub(crate) fn current_live_bytes(&self) -> u64 {
        self.state
            .upgrade()
            .map_or(0, |state| lock(&state).current_live_bytes)
    }

    pub(crate) fn dimension_evidence(&self, dimension: D) -> Option<DimensionSnapshot>
    where
        D: Copy + Ord,
    {
        let state = self.state.upgrade()?;
        let state = lock(&state);
        // This weak probe observes independently surviving storage, not invocation-local work.
        dimension_entry(&state.dimensions, dimension).map(|entry| {
            dimension_snapshot(DimensionState::join(
                WorkDimensionState::default(),
                entry.state,
            ))
        })
    }

    pub(crate) fn is_terminal_qualified(&self) -> bool {
        self.state
            .upgrade()
            .is_some_and(|state| lock(&state).publication == PublicationState::TerminalQualified)
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
            work_catalog,
            work_dimensions,
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
            retained_output_bytes: 0,
            retained_output_reservations: 0,
            evidence_byte_limit,
            current_evidence_bytes,
            peak_evidence_bytes,
            publication: PublicationState::Open,
            last_completed_phase: None,
        };
        for phase in phases {
            try_insert_phase_catalog_entry(&mut state, *phase)?;
        }
        let mut phase_catalogue = Vec::new();
        try_grow_evidence_vec(
            &mut phase_catalogue,
            state.phases.len(),
            &mut state.current_evidence_bytes,
            &mut state.peak_evidence_bytes,
            state.evidence_byte_limit,
            LedgerEvidencePurpose::Phases,
        )
        .map_err(|cause| LedgerCause::Evidence { cause })?;
        phase_catalogue.extend_from_slice(&state.phases);
        Ok(Self {
            work: WorkState::new(work_catalog.len(), work_dimensions),
            work_catalog,
            phase_catalogue,
            state: Arc::new(Mutex::new(state)),
            operation_live_byte_limit,
            operation_owner: PhantomData,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_probe(&self) -> LedgerTestProbe<P, D> {
        LedgerTestProbe {
            state: Arc::downgrade(&self.state),
        }
    }

    pub(crate) fn ensure_work_open(&self) -> Result<(), LedgerCause<D>> {
        self.work.check_integrity(self.work_catalog.len())?;
        match self.work.header.mode {
            WorkMode::Ordinary => Ok(()),
            WorkMode::OrdinarySealed => Err(LedgerCause::PublicationAlreadyPending),
            WorkMode::Protected | WorkMode::Closed => Err(LedgerCause::PublicationAlreadyCompleted),
        }
    }

    fn bind_work_layout<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        self.work
            .bind_layout(&self.work_catalog, dimensions, WorkGrant::Ordinary)
    }

    fn bound_dimension_position(&self, dimension: D) -> Result<usize, LedgerCause<D>> {
        self.work_catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })
    }

    fn bound_phase_position(&self, phase: P) -> Result<usize, LedgerCause<D>> {
        self.phase_catalogue
            .iter()
            .find(|entry| entry.phase == phase)
            .map(|entry| entry.state_position)
            .ok_or(LedgerCause::UnknownPhase)
    }

    pub(crate) fn ordinary_work_is_sealed(&self) -> bool {
        self.work.header.mode != WorkMode::Ordinary
    }

    pub(crate) fn check_work_quiescent(&self) -> Result<(), LedgerCause<D>> {
        self.work.check_quiescent(self.work_catalog.len())
    }

    /// Irreversibly revokes ordinary work before any terminal callback is delivered.
    pub(crate) fn seal_ordinary_work(&mut self) -> Result<(), LedgerCause<D>> {
        self.work.check_quiescent(self.work_catalog.len())?;
        let work = &mut self.work;
        let mut storage = lock(&self.state);
        match work.header.mode {
            WorkMode::Ordinary => {
                if storage.publication == PublicationState::Open {
                    storage.publication = PublicationState::Closing;
                }
                work.header.mode = WorkMode::OrdinarySealed;
            }
            WorkMode::OrdinarySealed | WorkMode::Protected => {}
            WorkMode::Closed => {}
        }
        Ok(())
    }

    fn protected_work_dimension_position(&self, dimension: D) -> Result<usize, LedgerCause<D>> {
        let position = self.bound_dimension_position(dimension)?;
        self.work.check_quiescent(self.work_catalog.len())?;
        Ok(position)
    }

    fn validate_protected_work_configuration(work: &WorkState<D>) -> Result<(), LedgerCause<D>> {
        if work.header.mode != WorkMode::Ordinary || work.header.protected_dimension.is_some() {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(())
    }

    /// Checks protected-work eligibility without installing a dimension or changing work state.
    #[cfg(test)]
    pub(crate) fn preflight_configure_protected_work(
        &self,
        dimension: D,
    ) -> Result<(), LedgerCause<D>> {
        self.protected_work_dimension_position(dimension)?;
        let work = &self.work;
        Self::validate_protected_work_configuration(work)
    }

    pub(crate) fn configure_protected_work(&mut self, dimension: D) -> Result<(), LedgerCause<D>> {
        let position = self.protected_work_dimension_position(dimension)?;
        let work = &mut self.work;
        Self::validate_protected_work_configuration(work)?;
        work.header.protected_dimension = Some(position);
        Ok(())
    }

    /// Qualifies the bound dormant failure pool without granting work or changing its mode.
    pub(crate) fn preflight_protected_work_attachment<const N: usize>(
        &self,
        dimension: D,
        charges: &[(D, u64); N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        let position = self.protected_work_dimension_position(dimension)?;
        let work = &self.work;
        if work.header.mode != WorkMode::Ordinary
            || work.header.protected_dimension != Some(position)
        {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let layout = work::WorkLayout::bind(
            &self.work_catalog,
            &charges.map(|(dimension, _)| dimension),
            WorkGrant::Protected,
        )?;
        layout.preflight_repeated_observed(
            &self.work_catalog,
            &work.rows,
            &work.header,
            charges.map(|(_, amount)| amount),
            repetitions,
        )
    }

    /// Activates the one prequalified private failure dimension after ordinary sealing.
    pub(crate) fn activate_protected_work(&mut self, dimension: D) -> Result<(), LedgerCause<D>> {
        let position = self
            .work_catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        self.seal_ordinary_work()?;
        let work = &mut self.work;
        if work.header.mode != WorkMode::OrdinarySealed
            || work.header.protected_dimension != Some(position)
        {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        work.header.mode = WorkMode::Protected;
        Ok(())
    }

    /// Closes the invocation only after all ordinary and protected owners explicitly resolve.
    pub(crate) fn close_work(&mut self) -> Result<(), LedgerCause<D>> {
        self.work.check_quiescent(self.work_catalog.len())?;
        let work = &mut self.work;
        match work.header.mode {
            WorkMode::Ordinary => return Err(LedgerCause::InactiveLinearWorkPermit),
            WorkMode::OrdinarySealed | WorkMode::Protected | WorkMode::Closed => {
                work.header.mode = WorkMode::Closed;
            }
        }
        Ok(())
    }

    /// Charges non-refundable attempted work before an action.
    pub fn charge_attempted(&mut self, dimension: D, amount: u64) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        let layout = self.bind_work_layout([dimension])?;
        layout.charge_attempted_in_order(&mut self.work.view(&self.work_catalog), [amount])
    }

    pub(crate) fn charge_producer_attempted(
        &mut self,
        dimension: D,
        amount: u64,
        excluded_dimension: Option<D>,
    ) -> Result<(), LedgerCause<D>> {
        if excluded_dimension == Some(dimension) {
            return Err(LedgerCause::UnknownDimension { dimension });
        }
        let layout = self.bind_work_layout([dimension])?;
        layout.charge_attempted_in_order(&mut self.work.view(&self.work_catalog), [amount])
    }

    /// Installs both halves of a tighter mixed policy at one coherent observation point.
    pub fn tighten_dimension_policy(
        &mut self,
        dimension: D,
        upper_bound: DimensionPolicy,
    ) -> Result<(), LedgerCause<D>> {
        let position = self
            .work_catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        let work = &mut self.work;
        let mut storage = lock(&self.state);
        Self::tighten_policy_in(
            &mut work.view(&self.work_catalog),
            &mut storage,
            position,
            dimension,
            upper_bound,
        )
    }

    fn tighten_policy_in(
        work: &mut work::WorkView<'_, D>,
        storage: &mut LedgerState<P, D>,
        position: usize,
        dimension: D,
        upper_bound: DimensionPolicy,
    ) -> Result<(), LedgerCause<D>> {
        Self::ensure_open(storage)?;
        if work.header.mode != WorkMode::Ordinary {
            return Err(LedgerCause::PublicationAlreadyCompleted);
        }
        let work_row = &mut work.rows[position];
        let storage_row = &mut storage.dimensions[position];
        let policy = storage_row
            .policy
            .with_work(work_row.policy)
            .tighten_with(upper_bound)
            .ok_or(LedgerCause::DimensionPolicyShapeMismatch { dimension })?;
        work_row.policy = WorkPolicy {
            attempted_limit: policy.attempted_limit,
            consumed_limit: policy.consumed_limit,
        };
        storage_row.policy = StoragePolicy::from_policy(policy);
        validate_dimension_state(
            dimension,
            policy,
            DimensionState::join(work_row.state, storage_row.state),
        )
    }

    /// Transfers unused ordinary counter policy into the private terminal counter atomically.
    pub(crate) fn transfer_failure_terminal_work(
        &mut self,
        ordinary_dimension: D,
        terminal_dimension: D,
        additional_work: u64,
    ) -> Result<(), LedgerCause<D>> {
        let position = |dimension| {
            self.work_catalog
                .binary_search(&dimension)
                .map_err(|_| LedgerCause::UnknownDimension { dimension })
        };
        let ordinary = position(ordinary_dimension)?;
        let terminal = position(terminal_dimension)?;
        if ordinary == terminal {
            return Err(LedgerCause::DimensionPolicyShapeMismatch {
                dimension: ordinary_dimension,
            });
        }
        self.work.check_quiescent(self.work_catalog.len())?;
        let work = &mut self.work;
        let storage = lock(&self.state);
        Self::ensure_open(&storage)?;
        if work.header.mode != WorkMode::Ordinary {
            return Err(LedgerCause::PublicationAlreadyPending);
        }
        for index in [ordinary, terminal] {
            let row = &storage.dimensions[index];
            if row.policy != StoragePolicy::from_policy(DimensionPolicy::counter(0, 0, 0)) {
                return Err(LedgerCause::DimensionPolicyShapeMismatch {
                    dimension: row.dimension,
                });
            }
            validate_dimension_state(
                row.dimension,
                row.policy.with_work(work.rows[index].policy),
                DimensionState::join(work.rows[index].state, row.state),
            )?;
        }
        let ordinary_policy = work.rows[ordinary].policy;
        let terminal_policy = work.rows[terminal].policy;
        let subtract = |class, limit: u64| {
            limit
                .checked_sub(additional_work)
                .ok_or(LedgerCause::LimitExceeded {
                    class,
                    dimension: ordinary_dimension,
                    value: additional_work,
                    limit,
                })
        };
        let add = |class, limit: u64| {
            limit
                .checked_add(additional_work)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class,
                    dimension: terminal_dimension,
                })
        };
        let ordinary_policy = WorkPolicy {
            attempted_limit: subtract(CounterClass::Attempted, ordinary_policy.attempted_limit)?,
            consumed_limit: subtract(CounterClass::Consumed, ordinary_policy.consumed_limit)?,
        };
        let terminal_policy = WorkPolicy {
            attempted_limit: add(CounterClass::Attempted, terminal_policy.attempted_limit)?,
            consumed_limit: add(CounterClass::Consumed, terminal_policy.consumed_limit)?,
        };
        for (index, policy) in [(ordinary, ordinary_policy), (terminal, terminal_policy)] {
            let row = &storage.dimensions[index];
            validate_dimension_state(
                row.dimension,
                row.policy.with_work(policy),
                DimensionState::join(work.rows[index].state, row.state),
            )?;
        }
        work.rows[ordinary].policy = ordinary_policy;
        work.rows[terminal].policy = terminal_policy;
        Ok(())
    }

    /// Records a non-refundable physically completed scalar prefix.
    pub fn charge_consumed(&mut self, dimension: D, amount: u64) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        let layout = self.bind_work_layout([dimension])?;
        layout.consume_observed_in_order(&mut self.work.view(&self.work_catalog), [amount])
    }

    pub fn charge_observed_batch(&mut self, charges: &[(D, u64)]) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        for &(dimension, amount) in charges {
            let layout = self.bind_work_layout([dimension])?;
            layout.charge_observed_ordered(&mut self.work.view(&self.work_catalog), [amount])?;
        }
        Ok(())
    }

    pub(crate) fn preflight_repeated_observed_batch<const N: usize>(
        &self,
        charges: &[(D, u64); N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        self.bind_work_layout(charges.map(|(dimension, _)| dimension))?
            .preflight_repeated_observed(
                &self.work_catalog,
                &self.work.rows,
                &self.work.header,
                charges.map(|(_, amount)| amount),
                repetitions,
            )
    }

    pub fn charge_consumed_batch<const N: usize>(
        &mut self,
        charges: &[(D, u64); N],
    ) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        let mut previous = None;
        for &(dimension, _) in charges {
            let position = self
                .work_catalog
                .binary_search(&dimension)
                .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
            if let Some((previous_position, previous_dimension)) = previous
                && position < previous_position
            {
                return Err(LedgerCause::ConsumedBatchOutOfOrder {
                    previous: previous_dimension,
                    dimension,
                });
            }
            previous = Some((position, dimension));
        }
        let layout = self.bind_work_layout(charges.map(|(dimension, _)| dimension))?;
        layout.consume_direct_atomic(
            &mut self.work.view(&self.work_catalog),
            charges.map(|(_, amount)| amount),
        )
    }

    pub(crate) fn charge_producer_consumed_batch<const N: usize>(
        &mut self,
        charges: &[(D, u64); N],
        excluded_dimension: Option<D>,
    ) -> Result<(), LedgerCause<D>> {
        for &(dimension, _) in charges {
            if excluded_dimension == Some(dimension)
                || self.work_catalog.binary_search(&dimension).is_err()
            {
                return Err(LedgerCause::UnknownDimension { dimension });
            }
        }
        let bound = self.bind_work_layout(charges.map(|(dimension, _)| dimension))?;
        self.ensure_work_open()?;
        for pair in charges.windows(2) {
            if pair[1].0 < pair[0].0 {
                return Err(LedgerCause::ConsumedBatchOutOfOrder {
                    previous: pair[0].0,
                    dimension: pair[1].0,
                });
            }
        }
        bound.consume_direct_atomic(
            &mut self.work.view(&self.work_catalog),
            charges.map(|(_, amount)| amount),
        )
    }

    fn storage_admission<Destination: PublicationDestination>(
        &self,
    ) -> Result<StorageAdmission, LedgerCause<D>> {
        self.work.check_integrity(self.work_catalog.len())?;
        Self::storage_admission_for_mode::<Destination>(self.work.header.mode)
    }

    fn storage_admission_for_mode<Destination: PublicationDestination>(
        mode: WorkMode,
    ) -> Result<StorageAdmission, LedgerCause<D>> {
        match mode {
            WorkMode::Ordinary => Ok(StorageAdmission::Ordinary),
            WorkMode::Protected if Destination::IS_OUTCOME_EVIDENCE => {
                Ok(StorageAdmission::ProtectedEvidence)
            }
            WorkMode::OrdinarySealed | WorkMode::Protected => {
                Err(LedgerCause::PublicationAlreadyPending)
            }
            WorkMode::Closed => Err(LedgerCause::PublicationAlreadyCompleted),
        }
    }

    fn ensure_storage_admission(
        state: &LedgerState<P, D>,
        admission: StorageAdmission,
    ) -> Result<(), LedgerCause<D>> {
        match (state.publication, admission) {
            (PublicationState::Open, StorageAdmission::Ordinary)
            | (PublicationState::Closing, StorageAdmission::ProtectedEvidence) => Ok(()),
            (PublicationState::Closing | PublicationState::Pending, _) => {
                Err(LedgerCause::PublicationAlreadyPending)
            }
            _ => Err(LedgerCause::PublicationAlreadyCompleted),
        }
    }

    fn ensure_terminal_preflight(
        state: &LedgerState<P, D>,
        work: &WorkState<D>,
    ) -> Result<(), LedgerCause<D>> {
        if state.publication == PublicationState::Closing && work.header.mode == WorkMode::Closed {
            Ok(())
        } else {
            Self::ensure_open(state)
        }
    }

    /// Reserves byte capacity before allocation or backend entry.
    pub fn reserve(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        self.reserve_core(phase, dimension, bytes, StorageAdmission::Ordinary)
            .map(|core| LiveReservation { core })
    }

    fn reserve_core(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
        admission: StorageAdmission,
    ) -> Result<ReservationCore<O, P, D>, LedgerCause<D>> {
        Self::reserve_core_in(
            &self.state,
            self.operation_live_byte_limit,
            phase,
            dimension,
            bytes,
            admission,
            ReservationPositions {
                dimension: self.bound_dimension_position(dimension),
                phase: self.bound_phase_position(phase),
            },
        )
    }

    fn reserve_core_in(
        storage: &Arc<Mutex<LedgerState<P, D>>>,
        operation_live_byte_limit: u64,
        phase: P,
        dimension: D,
        bytes: u64,
        admission: StorageAdmission,
        positions: ReservationPositions<D>,
    ) -> Result<ReservationCore<O, P, D>, LedgerCause<D>> {
        let mut state = lock(storage);
        let projection = Self::project_reservation_in(
            operation_live_byte_limit,
            &state,
            dimension,
            bytes,
            NonZeroU64::MIN,
            admission,
            positions,
        )?;
        let ReservationProjection {
            dimension_position,
            dimension_live,
            operation_live,
            phase_position,
            phase_live,
            active_reservations,
        } = projection;

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
            Arc::clone(storage),
            phase,
            phase_position,
            dimension,
            dimension_position,
            bytes,
        ))
    }

    fn project_reservation_in(
        operation_live_byte_limit: u64,
        state: &LedgerState<P, D>,
        dimension: D,
        bytes: u64,
        reservation_count: NonZeroU64,
        admission: StorageAdmission,
        positions: ReservationPositions<D>,
    ) -> Result<ReservationProjection, LedgerCause<D>> {
        Self::ensure_storage_admission(state, admission)?;
        let dimension_position = positions.dimension?;
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
        if operation_live > operation_live_byte_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: operation_live,
                limit: operation_live_byte_limit,
            });
        }
        let phase_position = positions.phase?;
        let current_phase = state.phase_states[phase_position].current_live_bytes;
        let phase_live =
            current_phase
                .checked_add(bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })?;
        let active_reservations = state
            .active_reservations
            .checked_add(reservation_count.get())
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension,
            })?;

        Ok(ReservationProjection {
            dimension_position,
            dimension_live,
            operation_live,
            phase_position,
            phase_live,
            active_reservations,
        })
    }

    /// Checks aggregate reservation headroom under the same lock and rules as [`Self::reserve`].
    ///
    /// This qualification does not reserve capacity. A later reservation can still lose a race to
    /// another owner of this ledger, so callers must preserve their ordinary execution-failure
    /// containment after a successful preflight.
    pub(crate) fn preflight_reservation_batch(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
        reservation_count: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        let dimension_position = self.bound_dimension_position(dimension);
        let phase_position = self.bound_phase_position(phase);
        let state = lock(&self.state);
        Self::project_reservation_in(
            self.operation_live_byte_limit,
            &state,
            dimension,
            bytes,
            reservation_count,
            StorageAdmission::Ordinary,
            ReservationPositions {
                dimension: dimension_position,
                phase: phase_position,
            },
        )
        .map(drop)
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
        let admission = self.storage_admission::<Destination>()?;
        self.reserve_core(phase, dimension, bytes, admission)
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
        let admission = self
            .storage_admission::<Destination>()
            .map_err(AttributionBatchCreateError::Ledger)?;
        let mut capacity_cover = if planned_bytes == 0 {
            None
        } else {
            Some(StorageCoveringReservation {
                core: self
                    .reserve_core(phase, capacity_dimension, planned_bytes, admission)
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
                .resize_core(&mut cover.core, bytes, admission)
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

        let admission = match self.storage_admission::<Destination>() {
            Ok(admission) => admission,
            Err(cause) => return Err(CommitmentPushError::Ledger { commitment, cause }),
        };
        let bound_position = self.bound_dimension_position(commitment.dimension);
        let state = lock(&self.state);
        if let Err(cause) = Self::ensure_storage_admission(&state, admission) {
            return Err(CommitmentPushError::Ledger { commitment, cause });
        }
        let dimension_position = match bound_position {
            Ok(position) => position,
            Err(cause) => return Err(CommitmentPushError::Ledger { commitment, cause }),
        };
        commitment.dimension_position = dimension_position;
        batch.push_positioned_commitment(commitment);
        Ok(())
    }

    /// Atomically qualifies and positions one canonical and one outcome-evidence commitment.
    ///
    /// The startup baseline, both source batches, and both exact destinations are preflighted as
    /// the future primary publication under one ledger lock before either preallocated vector
    /// changes. This is the U4 freeze-permit seam: it stages no publication counters and returns
    /// no detached positioned token.
    pub(crate) fn try_qualify_and_push_exact_primary_commitment_pair(
        &self,
        baseline: &FallbackBatchBaseline<O, P, D>,
        canonical_batch: &mut AttributionReservationBatch<O, P, D, CanonicalOutput>,
        mut canonical: OutputCommitment<D, CanonicalOutput>,
        evidence_batch: &mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        mut evidence: OutputCommitment<D, OutcomeEvidenceDestination>,
    ) -> Result<(), CommitmentPairPushError<D>> {
        let canonical_position = self.bound_dimension_position(canonical.dimension);
        let evidence_position = self.bound_dimension_position(evidence.dimension);
        self.work
            .check_integrity(self.work_catalog.len())
            .map_err(CommitmentPairPushError::Ledger)?;
        let work = &self.work;
        let state = lock(&self.state);
        if !canonical_batch.validate_owner(&self.state)
            || !evidence_batch.validate_owner(&self.state)
        {
            return Err(CommitmentPairPushError::Ledger(LedgerCause::OwnerMismatch));
        }
        if !canonical_batch.has_commitment_slot() {
            return Err(CommitmentPairPushError::CanonicalBatchFull);
        }
        if !evidence_batch.has_commitment_slot() {
            return Err(CommitmentPairPushError::OutcomeEvidenceBatchFull);
        }

        canonical.dimension_position =
            canonical_position.map_err(CommitmentPairPushError::Ledger)?;
        evidence.dimension_position = evidence_position.map_err(CommitmentPairPushError::Ledger)?;
        self.preflight_primary_publication_locked(
            &state,
            work,
            baseline,
            Some(canonical_batch),
            Some(evidence_batch),
            true,
            Some(&canonical),
            Some(&evidence),
        )
        .map_err(CommitmentPairPushError::Ledger)?;

        canonical_batch.push_positioned_commitment(canonical);
        evidence_batch.push_positioned_commitment(evidence);
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
        self.resize_core(&mut reservation.core, bytes, StorageAdmission::Ordinary)
    }

    fn resize_core(
        &self,
        reservation: &mut ReservationCore<O, P, D>,
        bytes: u64,
        admission: StorageAdmission,
    ) -> Result<(), LedgerCause<D>> {
        Self::resize_core_in(
            &self.state,
            self.operation_live_byte_limit,
            reservation,
            bytes,
            admission,
        )
    }

    fn resize_core_in(
        storage: &Arc<Mutex<LedgerState<P, D>>>,
        operation_live_byte_limit: u64,
        reservation: &mut ReservationCore<O, P, D>,
        bytes: u64,
        admission: StorageAdmission,
    ) -> Result<(), LedgerCause<D>> {
        let mut state = lock(storage);
        Self::ensure_storage_admission(&state, admission)?;
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
        if operation_live > operation_live_byte_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: operation_live,
                limit: operation_live_byte_limit,
            });
        }

        let phase_live = project(state.phase_states[phase_position].current_live_bytes)?;
        let retained = if reservation.retained_output {
            Some((
                project(state.retained_output_bytes)?,
                project(
                    state.dimensions[dimension_position]
                        .state
                        .retained_output_bytes,
                )?,
                project(state.phase_states[phase_position].retained_output_bytes)?,
            ))
        } else {
            None
        };

        if let Some((operation, dimension, phase)) = retained {
            state.retained_output_bytes = operation;
            state.dimensions[dimension_position]
                .state
                .retained_output_bytes = dimension;
            state.phase_states[phase_position].retained_output_bytes = phase;
        }

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
    /// evidence failure leaves every token and every live current/peak counter unchanged. A
    /// projection that preserves the dimension or operation live total does not re-admit that
    /// unchanged total against a policy tightened below already-live evidence.
    pub fn transition_reservations(
        &self,
        reservations: &mut [LiveReservation<O, P, D>],
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        Self::transition_reservations_in(
            &self.state,
            self.operation_live_byte_limit,
            reservations,
            phase,
            dimension,
            bytes,
            ReservationPositions {
                dimension: self.bound_dimension_position(dimension),
                phase: self.bound_phase_position(phase),
            },
        )
    }

    fn transition_reservations_in(
        storage: &Arc<Mutex<LedgerState<P, D>>>,
        operation_live_byte_limit: u64,
        reservations: &mut [LiveReservation<O, P, D>],
        phase: P,
        dimension: D,
        bytes: u64,
        positions: ReservationPositions<D>,
    ) -> Result<(), LedgerCause<D>> {
        if reservations.is_empty() && bytes != 0 {
            return Err(LedgerCause::SplitExceedsReservation {
                requested: bytes,
                available: 0,
            });
        }
        for reservation in reservations.iter() {
            Self::check_reservation_in(storage, reservation)?;
            if reservation.core.retained_output {
                return Err(LedgerCause::OwnerMismatch);
            }
        }

        let source_bytes = reservations.iter().try_fold(0_u64, |total, reservation| {
            total
                .checked_add(reservation.core.bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })
        })?;
        let mut state = lock(storage);
        Self::ensure_open(&state)?;
        let dimension_position = positions.dimension?;
        let dimension_limit = state.dimensions[dimension_position]
            .policy
            .live_byte_limit
            .ok_or(LedgerCause::DimensionNotReservable { dimension })?;
        let phase_position = positions.phase?;
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
        let dimension_live_is_unchanged = target_dimension_source == bytes;
        let operation_live_is_unchanged = source_bytes == bytes;
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
        if !dimension_live_is_unchanged && dimension_live > dimension_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: dimension_live,
                limit: dimension_limit,
            });
        }
        let operation_live = project(state.current_live_bytes, source_bytes)?;
        if !operation_live_is_unchanged && operation_live > operation_live_byte_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension,
                value: operation_live,
                limit: operation_live_byte_limit,
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
        if reservation.core.retained_output {
            return Err(LedgerCause::OwnerMismatch);
        }
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
        let owner = reservation
            .core
            .owner
            .as_ref()
            .cloned()
            .ok_or(LedgerCause::OwnerMismatch)?;
        Ok(LiveReservation {
            core: ReservationCore::new(
                owner,
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
        &mut self,
        reservations: &mut PublicationReservationBatch<O, P, D>,
        commitments: &[Commitment<D>],
    ) -> Result<PendingPublication<O, P, D>, LedgerCause<D>> {
        for reservation in reservations.iter() {
            self.check_reservation(reservation)?;
        }
        let transaction_live_bytes = Self::reservation_bytes(reservations)?;
        self.work.check_integrity(self.work_catalog.len())?;
        {
            let work = &self.work;
            let storage = lock(&self.state);
            Self::preflight_publication(&storage, work, transaction_live_bytes)?;
        }
        // Allocation and key binding occur before either guard. This owner remains armed
        // until every current-state check succeeds and the pending transaction takes it.
        let aggregate = aggregate_commitments(&self.state, &self.work_catalog, commitments)
            .map_err(|cause| match cause {
                AggregateCommitmentError::Ledger(cause) => cause,
                AggregateCommitmentError::Evidence(cause) => LedgerCause::Evidence { cause },
            })?;
        let work = &mut self.work;
        let mut storage = lock(&self.state);
        Self::preflight_publication(&storage, work, transaction_live_bytes)?;
        validate_commitments(&storage.dimensions, aggregate.entries())?;
        for commitment in aggregate.entries() {
            storage.dimensions[commitment.dimension_position]
                .state
                .pending_publication += commitment.amount;
        }
        storage.publication = PublicationState::Pending;
        work.header.mode = WorkMode::OrdinarySealed;
        drop(storage);
        let (commitments, evidence_bytes) = aggregate.into_parts();
        Ok(PendingPublication {
            owner: Arc::clone(&self.state),
            reservations: std::mem::take(reservations),
            commitments,
            evidence_bytes,
            pending: true,
            operation_owner: PhantomData,
        })
    }

    fn preflight_publication(
        storage: &LedgerState<P, D>,
        work: &WorkState<D>,
        transaction_live_bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        Self::ensure_open(storage)?;
        if work.header.mode != WorkMode::Ordinary {
            return Err(LedgerCause::PublicationAlreadyPending);
        }
        validate_final_snapshot_shape(storage, work)?;
        if transaction_live_bytes != storage.current_live_bytes {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: transaction_live_bytes,
                ledger_bytes: storage.current_live_bytes,
            });
        }
        Ok(())
    }

    /// Captures immutable owner-neutral evidence without finalizing the ledger.
    pub fn try_snapshot(&self) -> Result<LedgerSnapshot<P, D>, LedgerEvidenceError> {
        try_snapshot(&self.state, &self.work_catalog, &self.work)
    }

    /// Observes only the transaction-temporary live state under one ledger lock.
    pub(crate) fn planning_state(&self) -> LedgerPlanningState {
        self.storage_view().planning_state()
    }

    pub(crate) fn release_progress_evidence(&self, retained_bytes: u64) {
        let mut state = lock(&self.state);
        debug_assert!(state.current_evidence_bytes >= retained_bytes);
        release_evidence(&mut state, retained_bytes);
    }

    /// Copies one dimension's scalar evidence without allocating a full snapshot.
    pub fn dimension_evidence(&self, dimension: D) -> Result<DimensionSnapshot, LedgerCause<D>> {
        let position = self
            .work_catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        self.work.check_integrity(self.work_catalog.len())?;
        let work = &self.work;
        let storage = lock(&self.state);
        Ok(dimension_snapshot(DimensionState::join(
            work.rows[position].state,
            storage.dimensions[position].state,
        )))
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn dimension_count(&self) -> usize {
        self.work_catalog.len()
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    pub(crate) fn dimension_at(&self, index: usize) -> Option<D> {
        self.work_catalog.get(index).copied()
    }

    pub(crate) fn phase_at(&self, index: usize) -> Option<P> {
        self.phase_catalogue.get(index).map(|entry| entry.phase)
    }

    pub(crate) fn catalog_shape_is_strictly_ordered(
        &self,
        expected_dimensions: usize,
        expected_phases: usize,
    ) -> bool {
        self.work_catalog.len() == expected_dimensions
            && self.phase_catalogue.len() == expected_phases
            && self.work_catalog.windows(2).all(|pair| pair[0] < pair[1])
            && self
                .phase_catalogue
                .windows(2)
                .all(|pair| pair[0].phase < pair[1].phase)
    }

    pub(crate) fn contains_dimension(&self, dimension: D) -> bool {
        self.work_catalog.binary_search(&dimension).is_ok()
    }

    pub(crate) fn contains_phase(&self, phase: P) -> bool {
        self.bound_phase_position(phase).is_ok()
    }

    /// Records one semantically completed phase in frozen catalog insertion order.
    ///
    /// Repeating the current phase is idempotent. Every rejection occurs before the stored
    /// completion position changes.
    pub(crate) fn try_complete_phase(&self, phase: P) -> Result<(), PhaseCompletionError<P>> {
        let bound_position = self.bound_phase_position(phase);
        let mut state = lock(&self.state);
        match state.publication {
            PublicationState::Open => {}
            PublicationState::Closing | PublicationState::Pending => {
                return Err(PhaseCompletionError::PublicationPending);
            }
            PublicationState::TerminalQualified | PublicationState::Published => {
                return Err(PhaseCompletionError::PublicationCompleted);
            }
        }
        if !phase_catalog_shape_is_valid(&state) {
            return Err(PhaseCompletionError::CatalogShapeMismatch);
        }
        let phase_position =
            bound_position.map_err(|_| PhaseCompletionError::UnknownPhase { phase })?;
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
        self.try_bind_fallback_baseline_with(baseline, FallbackBaselineQualification::Terminal)
    }

    /// Qualifies an actual startup baseline while unrelated storage and work remain owned.
    ///
    /// Success leaves all accounting and batch ownership unchanged. Exact terminal residue and
    /// protected-work eligibility are checked separately at their respective boundaries.
    pub(crate) fn try_attach_fallback_baseline(
        &self,
        baseline: &mut FallbackBatchBaseline<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.try_bind_fallback_baseline_with(baseline, FallbackBaselineQualification::Attachment)
    }

    fn try_bind_fallback_baseline_with(
        &self,
        baseline: &mut FallbackBatchBaseline<O, P, D>,
        qualification: FallbackBaselineQualification,
    ) -> Result<(), LedgerCause<D>> {
        match qualification {
            FallbackBaselineQualification::Attachment => self.ensure_work_open()?,
            FallbackBaselineQualification::Terminal => {
                self.work.check_integrity(self.work_catalog.len())?
            }
        }
        let work = &self.work;
        let state = lock(&self.state);
        self.validate_fallback_baseline_locked(&state, work, baseline, qualification)?;
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
        work: &WorkState<D>,
        baseline: &FallbackBatchBaseline<O, P, D>,
        qualification: FallbackBaselineQualification,
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
        match qualification {
            FallbackBaselineQualification::Attachment => {
                Self::ensure_open(state)?;
                if state.dimensions.len() != work.rows.len()
                    || state.dimensions.len() != state.dimension_snapshot_staging.len()
                    || !phase_catalog_shape_is_valid(state)
                {
                    return Err(LedgerCause::FinalSnapshotShapeMismatch);
                }
            }
            FallbackBaselineQualification::Terminal => {
                Self::ensure_terminal_preflight(state, work)?;
                validate_final_snapshot_shape(state, work)?;
            }
        }
        let covered_active = expected_active
            .checked_add(state.retained_output_reservations)
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        if !qualification.accepts_coverage(state.active_reservations, covered_active) {
            return Err(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            });
        }
        let covered_live = expected_live
            .checked_add(state.retained_output_bytes)
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        if !qualification.accepts_coverage(state.current_live_bytes, covered_live) {
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
    /// Attempts to bind the typed primary publication graph without consuming its owners.
    ///
    /// An ordinary `Err` is transactional: the ledger, `baseline`, and every supplied
    /// canonical or outcome-evidence source batch remain unchanged. In particular, their
    /// reservations, capacity covers, and commitment entries remain owned by the caller and may
    /// be reused for a later compatible bind attempt. The first mutation is source absorption,
    /// which occurs only after every validation and preflight check succeeds.
    ///
    /// A panic is not an ordinary rejection; callers must contain it before deciding whether any
    /// retained physical ownership can be released.
    pub(crate) fn try_bind_primary(
        &self,
        baseline: &mut FallbackBatchBaseline<O, P, D>,
        canonical_source: Option<&mut AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<
            &mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        >,
        publish_canonical: bool,
    ) -> Result<(), LedgerCause<D>> {
        self.work.check_integrity(self.work_catalog.len())?;
        let work = &self.work;
        let state = lock(&self.state);
        self.preflight_primary_publication_locked(
            &state,
            work,
            baseline,
            canonical_source.as_deref(),
            evidence_source.as_deref(),
            publish_canonical,
            None,
            None,
        )?;
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

    #[allow(
        dead_code,
        clippy::too_many_arguments,
        reason = "the private preflight keeps the exact primary graph and optional U4 pair explicit"
    )]
    fn preflight_primary_publication_locked(
        &self,
        state: &LedgerState<P, D>,
        work: &WorkState<D>,
        baseline: &FallbackBatchBaseline<O, P, D>,
        canonical_source: Option<&AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<&AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
        publish_canonical: bool,
        canonical_extra: Option<&OutputCommitment<D, CanonicalOutput>>,
        evidence_extra: Option<&OutputCommitment<D, OutcomeEvidenceDestination>>,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_primary_publication_locked(
            state,
            work,
            baseline,
            canonical_source,
            evidence_source,
            publish_canonical,
            canonical_extra,
            evidence_extra,
        )?;
        preflight_typed_publication_batch(state, &baseline.canonical_assembly)?;
        preflight_typed_publication_batch(state, &baseline.evidence_assembly)?;
        preflight_typed_publication_batch(state, &baseline.evidence)?;
        preflight_typed_discard_batch(state, &baseline.fallback)?;
        if let Some(batch) = canonical_source {
            if publish_canonical {
                preflight_typed_publication_batch(state, batch)?;
            } else {
                preflight_typed_discard_batch(state, batch)?;
            }
        }
        if let Some(batch) = evidence_source {
            preflight_typed_publication_batch(state, batch)?;
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

        self.work.check_integrity(self.work_catalog.len())?;
        let work = &self.work;
        let state = lock(&self.state);
        Self::ensure_terminal_preflight(&state, work)?;
        validate_final_snapshot_shape(&state, work)?;
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
        clippy::too_many_arguments,
        reason = "the private validation keeps the exact primary graph and optional U4 pair explicit"
    )]
    fn validate_primary_publication_locked(
        &self,
        state: &LedgerState<P, D>,
        work: &WorkState<D>,
        baseline: &FallbackBatchBaseline<O, P, D>,
        canonical_source: Option<&AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<&AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
        publish_canonical: bool,
        canonical_extra: Option<&OutputCommitment<D, CanonicalOutput>>,
        evidence_extra: Option<&OutputCommitment<D, OutcomeEvidenceDestination>>,
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
            && !can_absorb_sources_with_additional_commitments(
                &baseline.canonical_assembly,
                [canonical_source],
                usize::from(canonical_extra.is_some()),
            ))
            || !can_absorb_sources_with_additional_commitments(
                &baseline.evidence_assembly,
                [Some(&baseline.evidence), evidence_source],
                usize::from(evidence_extra.is_some()),
            )
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

        // The exact pair only stages destinations after the body requests cleanup. It neither
        // publishes nor reopens ordinary work; actual primary binding still requires closed work.
        let sealed_destination_pair = canonical_extra.is_some()
            && evidence_extra.is_some()
            && state.publication == PublicationState::Closing
            && work.header.mode == WorkMode::OrdinarySealed;
        if !sealed_destination_pair {
            Self::ensure_terminal_preflight(state, work)?;
        }
        validate_final_snapshot_shape(state, work)?;
        let active_after_transition = state
            .active_reservations
            .checked_sub(released_active)
            .ok_or(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            })?;
        if active_after_transition
            != projected_active
                .checked_add(state.retained_output_reservations)
                .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?
        {
            return Err(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            });
        }
        let live_after_transition = state.current_live_bytes.checked_sub(released_live).ok_or(
            LedgerCause::LiveReservationsRemain {
                bytes: state.current_live_bytes,
            },
        )?;
        if live_after_transition
            != projected_live
                .checked_add(state.retained_output_bytes)
                .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?
        {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: projected_live,
                ledger_bytes: live_after_transition,
            });
        }
        validate_typed_commitment_batches_with_extra(
            &state.dimensions,
            [
                Some(&baseline.canonical_assembly),
                publish_canonical.then_some(canonical_source).flatten(),
            ],
            publish_canonical.then_some(canonical_extra).flatten(),
        )?;
        validate_typed_commitment_batches_with_extra(
            &state.dimensions,
            [
                Some(&baseline.evidence_assembly),
                Some(&baseline.evidence),
                evidence_source,
            ],
            evidence_extra,
        )
    }

    /// Copies one phase's scalar evidence without allocating a full snapshot.
    pub fn phase_evidence(&self, phase: P) -> Option<PhaseSnapshot> {
        let position = self.bound_phase_position(phase).ok()?;
        let state = lock(&self.state);
        state
            .phase_states
            .get(position)
            .map(|state| phase_snapshot(*state))
    }

    /// Finalizes evidence only after every live reservation and pending transaction is gone.
    #[allow(
        clippy::result_large_err,
        reason = "boxing final evidence would allocate on the invariant-failure path"
    )]
    pub fn finish(self) -> Result<LedgerSnapshot<P, D>, LedgerFinishError<P, D>> {
        let Self {
            mut work,
            work_catalog,
            phase_catalogue: _phase_catalogue,
            state,
            ..
        } = self;
        work.header.integrity_fault |= work.rows.len() != work_catalog.len();
        let mut state = lock(&state);
        let shape_cause = validate_final_snapshot_shape(&state, &work).err();
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
        let evidence = take_final_snapshot(&mut state, &work, clean);
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
        Self::check_reservation_in(&self.state, reservation)
    }

    fn check_reservation_in(
        storage: &Arc<Mutex<LedgerState<P, D>>>,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        if !reservation.core.is_owned_by(storage) {
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
        &mut self,
        publication: &mut PendingPublication<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.check_publication(publication)?;
        self.seal_ordinary_work()?;
        self.close_work()?;
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
        state.publication = PublicationState::Published;
        publication.pending = false;
        drop(state);
        publication
            .reservations
            .destroy_backing_then_release_cover();
        drop(std::mem::take(&mut publication.commitments));
        release_evidence(&mut lock(&self.state), publication.evidence_bytes);
        publication.evidence_bytes = 0;
        Ok(())
    }

    fn ensure_open(state: &LedgerState<P, D>) -> Result<(), LedgerCause<D>> {
        match state.publication {
            PublicationState::Open => Ok(()),
            PublicationState::Closing => Err(LedgerCause::PublicationAlreadyPending),
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

pub(crate) enum CommitmentPairPushError<D> {
    CanonicalBatchFull,
    OutcomeEvidenceBatchFull,
    Ledger(LedgerCause<D>),
}

fn can_absorb_sources_with_additional_commitments<O, P, D, Destination, const N: usize>(
    target: &AttributionReservationBatch<O, P, D, Destination>,
    sources: [Option<&AttributionReservationBatch<O, P, D, Destination>>; N],
    additional_commitments: usize,
) -> bool
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if !target.can_absorb_sources(sources) {
        return false;
    }
    sources
        .into_iter()
        .flatten()
        .try_fold(target.commitments.len(), |length, source| {
            length.checked_add(source.commitments.len())
        })
        .and_then(|length| length.checked_add(additional_commitments))
        .is_some_and(|length| {
            length <= target.commitment_limit && length <= target.commitments.capacity()
        })
}

fn validate_typed_commitment_batches_with_extra<O, P, D, Destination, const N: usize>(
    dimensions: &[DimensionEntry<D>],
    batches: [Option<&AttributionReservationBatch<O, P, D, Destination>>; N],
    extra: Option<&OutputCommitment<D, Destination>>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    let class = match Destination::KIND {
        DestinationKind::CanonicalOutput => CounterClass::Committed,
        DestinationKind::OutcomeEvidence => CounterClass::PublishedOutcomeEvidence,
    };
    let all = || {
        batches
            .into_iter()
            .flatten()
            .flat_map(|batch| batch.commitments.iter())
            .chain(extra)
    };
    for (index, commitment) in all().enumerate() {
        if all()
            .take(index)
            .any(|prior| prior.dimension_position == commitment.dimension_position)
        {
            continue;
        }
        let amount = all()
            .filter(|candidate| candidate.dimension_position == commitment.dimension_position)
            .try_fold(0_u64, |total, candidate| {
                total.checked_add(candidate.amount)
            })
            .ok_or(LedgerCause::ArithmeticOverflow {
                class,
                dimension: commitment.dimension,
            })?;
        let entry = dimensions
            .get(commitment.dimension_position)
            .ok_or(LedgerCause::FinalSnapshotShapeMismatch)?;
        let (current, limit) = match Destination::KIND {
            DestinationKind::CanonicalOutput => {
                (entry.state.committed, entry.policy.committed_limit)
            }
            DestinationKind::OutcomeEvidence => (
                entry.state.published_outcome_evidence,
                entry.policy.published_outcome_evidence_limit,
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

/// Why a terminal-qualified ledger could not become an exclusively owned publication capsule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypedFreezeDetachErrorKind {
    SharedLedgerOwner,
    PoisonedLedger,
    UnexpectedDiscardOwner,
}

struct DetachedTypedBatches<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical: DetachedAttributionBatch<O, P, D, CanonicalOutput>,
    evidence: DetachedAttributionBatch<O, P, D, OutcomeEvidenceDestination>,
    deferred_canonical_storage: Option<DetachedAttributionBatch<O, P, D, CanonicalOutput>>,
    deferred_evidence_storage_first:
        Option<DetachedAttributionBatch<O, P, D, OutcomeEvidenceDestination>>,
    deferred_evidence_storage_second:
        Option<DetachedAttributionBatch<O, P, D, OutcomeEvidenceDestination>>,
    discard_canonical: Option<DetachedAttributionBatch<O, P, D, CanonicalOutput>>,
    discard_evidence: Option<DetachedAttributionBatch<O, P, D, OutcomeEvidenceDestination>>,
}

struct DetachedLiveLedgerCover<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: LedgerState<P, D>,
    batches: DetachedTypedBatches<O, P, D>,
}

enum FailedDetachedLedgerState<P, D> {
    Shared(Arc<Mutex<LedgerState<P, D>>>),
    Owned(LedgerState<P, D>),
}

struct FailedDetachedLiveLedgerCover<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: FailedDetachedLedgerState<P, D>,
    batches: DetachedTypedBatches<O, P, D>,
}

impl<O, P, D> FailedDetachedLiveLedgerCover<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn retain_fail_closed(self) {
        let Self { state, batches } = self;
        match state {
            FailedDetachedLedgerState::Shared(state) => std::mem::forget(state),
            FailedDetachedLedgerState::Owned(state) => std::mem::forget(state),
        }
        std::mem::forget(batches);
    }
}

/// A terminal-qualified ledger whose physical accounting graph and mutable state are exclusively
/// owned, but whose physical account has not yet crossed publication eligibility.
///
/// Dropping this capability is fail-closed: the complete live cover and terminal projection are
/// retained. Accounting can retire only through the account-bound joint-commit permit.
pub(crate) struct DetachedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    projection: Option<LedgerSnapshot<P, D>>,
    cover: Option<DetachedLiveLedgerCover<O, P, D>>,
}

/// A consuming detach failure that still owns every ledger and reservation component.
///
/// This value cannot retry or refund. Dropping it deliberately retains the complete graph.
pub(crate) struct TypedFreezeDetachError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    kind: TypedFreezeDetachErrorKind,
    projection: Option<LedgerSnapshot<P, D>>,
    cover: Option<Box<FailedDetachedLiveLedgerCover<O, P, D>>>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetachedLedgerTerminalScalars {
    pub(crate) current_live_bytes: u64,
    pub(crate) active_reservations: u64,
    pub(crate) current_evidence_bytes: u64,
    pub(crate) published: bool,
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
        work: &WorkState<D>,
    ) -> Result<Option<P>, TypedFreezeQualificationError> {
        if state.publication != PublicationState::Closing || work.header.mode != WorkMode::Closed {
            return Err(TypedFreezeQualificationError::PublicationState);
        }
        if !typed_terminal_snapshot_positions_are_valid(state, work) {
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
        let expected_live = expected_live
            .checked_add(state.retained_output_bytes)
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
        let expected_active = expected_active
            .checked_add(state.retained_output_reservations)
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
            let expected = expected
                .checked_add(entry.state.retained_output_bytes)
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
            let expected = expected
                .checked_add(phase.retained_output_bytes)
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
        let qualification = (|| {
            self.ledger
                .work
                .check_integrity(self.ledger.work_catalog.len())
                .map_err(|_| TypedFreezeQualificationError::SnapshotShape)?;
            let work = &self.ledger.work;
            let mut state = lock(&self.ledger.state);
            self.validate_terminal(&state, work).inspect(|_| {
                state.publication = PublicationState::TerminalQualified;
            })
        })();
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

    fn try_into_detached(
        mut self,
    ) -> Result<DetachedTypedFreezeProof<O, P, D>, TypedFreezeDetachError<O, P, D>> {
        let last_completed_phase = self.last_completed_phase;
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable!("qualified proof disappeared"));
        let TypedFreezeProof {
            canonical,
            evidence,
            deferred_canonical_storage,
            deferred_evidence_storage_first,
            deferred_evidence_storage_second,
            discard_canonical,
            discard_evidence,
            ledger,
        } = proof;
        let unexpected_discard = discard_canonical.is_some() || discard_evidence.is_some();
        let Ledger {
            mut work,
            work_catalog,
            phase_catalogue: _phase_catalogue,
            state,
            ..
        } = ledger;
        work.header.integrity_fault |= work.rows.len() != work_catalog.len();
        let projection = {
            let mut storage = lock(&state);
            take_typed_terminal_projection(
                &mut storage,
                &work,
                last_completed_phase,
                &canonical,
                &evidence,
            )
        };
        drop(work);
        let batches = DetachedTypedBatches {
            canonical: canonical.into_detached(&state),
            evidence: evidence.into_detached(&state),
            deferred_canonical_storage: deferred_canonical_storage
                .map(|batch| batch.into_detached(&state)),
            deferred_evidence_storage_first: deferred_evidence_storage_first
                .map(|batch| batch.into_detached(&state)),
            deferred_evidence_storage_second: deferred_evidence_storage_second
                .map(|batch| batch.into_detached(&state)),
            discard_canonical: discard_canonical.map(|batch| batch.into_detached(&state)),
            discard_evidence: discard_evidence.map(|batch| batch.into_detached(&state)),
        };
        let state = match Arc::try_unwrap(state) {
            Ok(state) => match state.into_inner() {
                Ok(state) => state,
                Err(poisoned) => {
                    return Err(TypedFreezeDetachError::new(
                        TypedFreezeDetachErrorKind::PoisonedLedger,
                        projection,
                        FailedDetachedLedgerState::Owned(poisoned.into_inner()),
                        batches,
                    ));
                }
            },
            Err(state) => {
                return Err(TypedFreezeDetachError::new(
                    TypedFreezeDetachErrorKind::SharedLedgerOwner,
                    projection,
                    FailedDetachedLedgerState::Shared(state),
                    batches,
                ));
            }
        };
        if unexpected_discard {
            return Err(TypedFreezeDetachError::new(
                TypedFreezeDetachErrorKind::UnexpectedDiscardOwner,
                projection,
                FailedDetachedLedgerState::Owned(state),
                batches,
            ));
        }
        Ok(DetachedTypedFreezeProof {
            projection: Some(projection),
            cover: Some(DetachedLiveLedgerCover { state, batches }),
        })
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
        }
        discard_evidence.destroy_storage_then_release_bound_cover();
        release_deferred_typed_storage(
            deferred_canonical_storage,
            deferred_evidence_storage_first,
            deferred_evidence_storage_second,
        );
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

    pub(crate) fn try_into_detached(
        mut self,
    ) -> Result<DetachedTypedFreezeProof<O, P, D>, TypedFreezeDetachError<O, P, D>> {
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable!("freeze-ready proof disappeared"));
        proof.try_into_detached()
    }

    pub(crate) fn retain_fail_closed(self) {
        std::mem::forget(self);
    }
}

impl<O, P, D> DetachedTypedBatches<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn apply_commitments(&self, state: &mut LedgerState<P, D>) {
        self.canonical.apply_commitments(&mut state.dimensions);
        self.evidence.apply_commitments(&mut state.dimensions);
    }

    fn release_attributions(&mut self, state: &mut LedgerState<P, D>) {
        self.canonical.release_attributions(state);
        self.evidence.release_attributions(state);
        if let Some(batch) = self.deferred_canonical_storage.as_mut() {
            batch.release_attributions(state);
        }
        if let Some(batch) = self.deferred_evidence_storage_first.as_mut() {
            batch.release_attributions(state);
        }
        if let Some(batch) = self.deferred_evidence_storage_second.as_mut() {
            batch.release_attributions(state);
        }
        if let Some(batch) = self.discard_canonical.as_mut() {
            batch.release_attributions(state);
        }
        if let Some(batch) = self.discard_evidence.as_mut() {
            batch.release_attributions(state);
        }
    }

    fn destroy_storage_then_release_covers(&mut self, state: &mut LedgerState<P, D>) {
        self.canonical.destroy_storage_then_release_cover(state);
        self.evidence.destroy_storage_then_release_cover(state);
        if let Some(batch) = self.deferred_canonical_storage.as_mut() {
            batch.destroy_storage_then_release_cover(state);
        }
        if let Some(batch) = self.deferred_evidence_storage_first.as_mut() {
            batch.destroy_storage_then_release_cover(state);
        }
        if let Some(batch) = self.deferred_evidence_storage_second.as_mut() {
            batch.destroy_storage_then_release_cover(state);
        }
        if let Some(batch) = self.discard_canonical.as_mut() {
            batch.destroy_storage_then_release_cover(state);
        }
        if let Some(batch) = self.discard_evidence.as_mut() {
            batch.destroy_storage_then_release_cover(state);
        }
    }

    #[cfg(test)]
    fn active_count(&self) -> u64 {
        self.canonical.active_count()
            + self.evidence.active_count()
            + self
                .deferred_canonical_storage
                .as_ref()
                .map_or(0, DetachedAttributionBatch::active_count)
            + self
                .deferred_evidence_storage_first
                .as_ref()
                .map_or(0, DetachedAttributionBatch::active_count)
            + self
                .deferred_evidence_storage_second
                .as_ref()
                .map_or(0, DetachedAttributionBatch::active_count)
            + self
                .discard_canonical
                .as_ref()
                .map_or(0, DetachedAttributionBatch::active_count)
            + self
                .discard_evidence
                .as_ref()
                .map_or(0, DetachedAttributionBatch::active_count)
    }

    #[cfg(test)]
    fn live_bytes(&self) -> u64 {
        self.canonical.live_bytes()
            + self.evidence.live_bytes()
            + self
                .deferred_canonical_storage
                .as_ref()
                .map_or(0, DetachedAttributionBatch::live_bytes)
            + self
                .deferred_evidence_storage_first
                .as_ref()
                .map_or(0, DetachedAttributionBatch::live_bytes)
            + self
                .deferred_evidence_storage_second
                .as_ref()
                .map_or(0, DetachedAttributionBatch::live_bytes)
            + self
                .discard_canonical
                .as_ref()
                .map_or(0, DetachedAttributionBatch::live_bytes)
            + self
                .discard_evidence
                .as_ref()
                .map_or(0, DetachedAttributionBatch::live_bytes)
    }
}

impl<O, P, D> DetachedLiveLedgerCover<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn commit_account_bound(&mut self) {
        self.batches.apply_commitments(&mut self.state);
        self.batches.release_attributions(&mut self.state);
        release_retained_output_attributions(&mut self.state);
        self.batches
            .destroy_storage_then_release_covers(&mut self.state);
    }

    fn retire_evidence(&mut self) {
        drop(std::mem::take(&mut self.state.dimension_snapshot_staging));
        drop(std::mem::take(&mut self.state.phase_snapshot_staging));
        drop(std::mem::take(&mut self.state.dimensions));
        drop(std::mem::take(&mut self.state.phases));
        drop(std::mem::take(&mut self.state.phase_states));
        self.state.current_evidence_bytes = 0;
        self.state.publication = PublicationState::Published;
    }
}

impl<O, P, D> DetachedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn terminal_projection(&self) -> &LedgerSnapshot<P, D> {
        self.projection
            .as_ref()
            .unwrap_or_else(|| unreachable!("detached terminal projection disappeared"))
    }

    fn commit_inner(&mut self) -> (u64, u64, u64, bool) {
        let mut cover = self
            .cover
            .take()
            .unwrap_or_else(|| unreachable!("detached live ledger cover disappeared"));
        cover.commit_account_bound();
        drop(self.projection.take());
        cover.retire_evidence();
        let terminal = (
            cover.state.current_live_bytes,
            cover.state.active_reservations,
            cover.state.current_evidence_bytes,
            cover.state.publication == PublicationState::Published,
        );
        drop(cover);
        terminal
    }

    pub(crate) fn commit_account_bound(
        mut self,
        _permit: crate::retained::DetachedLedgerCommitPermit<'_>,
    ) {
        let _ = self.commit_inner();
    }

    pub(crate) fn retain_fail_closed(self) {
        drop(self);
    }

    #[cfg(test)]
    pub(crate) fn test_current_live_bytes(&self) -> u64 {
        self.cover
            .as_ref()
            .map_or(0, |cover| cover.state.current_live_bytes)
    }

    #[cfg(test)]
    pub(crate) fn test_active_reservations(&self) -> u64 {
        self.cover
            .as_ref()
            .map_or(0, |cover| cover.state.active_reservations)
    }

    #[cfg(test)]
    pub(crate) fn test_current_evidence_bytes(&self) -> u64 {
        self.cover
            .as_ref()
            .map_or(0, |cover| cover.state.current_evidence_bytes)
    }

    #[cfg(test)]
    pub(crate) fn test_storage_dimension_evidence_at(
        &self,
        position: usize,
    ) -> Option<DimensionSnapshot> {
        let row = self.cover.as_ref()?.state.dimensions.get(position)?;
        Some(dimension_snapshot(DimensionState::join(
            WorkDimensionState::default(),
            row.state,
        )))
    }

    #[cfg(test)]
    pub(crate) fn test_dimension_evidence(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.cover.as_ref().and_then(|cover| {
            dimension_entry(&cover.state.dimensions, dimension).map(|entry| {
                dimension_snapshot(DimensionState::join(
                    WorkDimensionState::default(),
                    entry.state,
                ))
            })
        })
    }

    #[cfg(test)]
    pub(crate) fn test_cover_matches_state(&self) -> bool {
        self.cover.as_ref().is_some_and(|cover| {
            cover
                .batches
                .live_bytes()
                .checked_add(cover.state.retained_output_bytes)
                == Some(cover.state.current_live_bytes)
                && cover
                    .batches
                    .active_count()
                    .checked_add(cover.state.retained_output_reservations)
                    == Some(cover.state.active_reservations)
        })
    }

    #[cfg(test)]
    fn commit_account_bound_for_test(mut self) -> DetachedLedgerTerminalScalars {
        let (current_live_bytes, active_reservations, current_evidence_bytes, published) =
            self.commit_inner();
        DetachedLedgerTerminalScalars {
            current_live_bytes,
            active_reservations,
            current_evidence_bytes,
            published,
        }
    }
}

impl<O, P, D> TypedFreezeDetachError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn new(
        kind: TypedFreezeDetachErrorKind,
        projection: LedgerSnapshot<P, D>,
        state: FailedDetachedLedgerState<P, D>,
        batches: DetachedTypedBatches<O, P, D>,
    ) -> Self {
        Self {
            kind,
            projection: Some(projection),
            cover: Some(Box::new(FailedDetachedLiveLedgerCover { state, batches })),
        }
    }

    #[cfg(test)]
    pub(crate) const fn kind(&self) -> TypedFreezeDetachErrorKind {
        self.kind
    }

    fn retain_parts_fail_closed(&mut self) {
        let _ = self.kind;
        if let Some(projection) = self.projection.take() {
            std::mem::forget(projection);
        }
        if let Some(cover) = self.cover.take() {
            (*cover).retain_fail_closed();
        }
    }

    pub(crate) fn retain_fail_closed(mut self) {
        self.retain_parts_fail_closed();
    }

    #[cfg(test)]
    pub(crate) fn test_current_live_bytes(&self) -> u64 {
        let Some(cover) = self.cover.as_ref() else {
            return 0;
        };
        match &cover.state {
            FailedDetachedLedgerState::Shared(state) => lock(state).current_live_bytes,
            FailedDetachedLedgerState::Owned(state) => state.current_live_bytes,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_active_reservations(&self) -> u64 {
        let Some(cover) = self.cover.as_ref() else {
            return 0;
        };
        match &cover.state {
            FailedDetachedLedgerState::Shared(state) => lock(state).active_reservations,
            FailedDetachedLedgerState::Owned(state) => state.active_reservations,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_projected_live_bytes(&self) -> u64 {
        self.projection
            .as_ref()
            .map_or(0, LedgerSnapshot::current_live_bytes)
    }

    #[cfg(test)]
    pub(crate) fn test_cover_matches_state(&self) -> bool {
        let Some(cover) = self.cover.as_ref() else {
            return false;
        };
        let (live, active) = match &cover.state {
            FailedDetachedLedgerState::Shared(state) => {
                let state = lock(state);
                (state.current_live_bytes, state.active_reservations)
            }
            FailedDetachedLedgerState::Owned(state) => {
                (state.current_live_bytes, state.active_reservations)
            }
        };
        cover.batches.live_bytes() == live && cover.batches.active_count() == active
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
    {
        let mut state = lock(&typed.ledger.state);
        discard.release_bound_attributions(&mut state);
    }
    discard.destroy_storage_then_release_bound_cover();
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

impl<O, P, D> Drop for DetachedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        let projection = self.projection.take();
        let cover = self.cover.take();
        if projection.is_some() || cover.is_some() {
            std::mem::forget((projection, cover));
        }
    }
}

impl<O, P, D> Drop for TypedFreezeDetachError<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        self.retain_parts_fail_closed();
    }
}

fn release_deferred_typed_storage<O, P, D>(
    deferred_canonical_storage: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    deferred_evidence_storage_first: Option<
        AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    >,
    deferred_evidence_storage_second: Option<
        AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    >,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    if let Some(mut storage) = deferred_canonical_storage {
        storage.destroy_storage_then_release_bound_cover();
    }
    if let Some(mut storage) = deferred_evidence_storage_first {
        storage.destroy_storage_then_release_bound_cover();
    }
    if let Some(mut storage) = deferred_evidence_storage_second {
        storage.destroy_storage_then_release_bound_cover();
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
    }

    fn destroy_backing_then_release_cover(&mut self) {
        drop(std::mem::take(&mut self.reservations));
        drop(self.capacity_reservation.take());
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
            state.publication = PublicationState::Closing;
        }
        drop(state);
        self.reservations.destroy_backing_then_release_cover();
        drop(std::mem::take(&mut self.commitments));
        release_evidence(&mut lock(&self.owner), self.evidence_bytes);
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

#[cfg(test)]
fn dimension_entry<D: Copy + Ord>(
    dimensions: &[DimensionEntry<D>],
    dimension: D,
) -> Option<&DimensionEntry<D>> {
    dimensions
        .binary_search_by_key(&dimension, |entry| entry.dimension)
        .ok()
        .map(|position| &dimensions[position])
}

fn validate_dimension_state<D: Copy>(
    dimension: D,
    policy: DimensionPolicy,
    state: DimensionState,
) -> Result<(), LedgerCause<D>> {
    work::validate_work_state(
        dimension,
        WorkPolicy::from(policy),
        WorkDimensionState {
            attempted: state.attempted,
            consumed: state.consumed,
            pending_consumed: state.pending_consumed,
        },
    )?;
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

/// Retires operation attribution at joint publication; allocation-bound physical leases remain
/// inside the actual output owners. Classification is accumulated in the existing catalog rows,
/// so terminal storage grows with neither dependency count nor path count.
fn release_retained_output_attributions<P, D>(state: &mut LedgerState<P, D>) {
    state.current_live_bytes -= state.retained_output_bytes;
    state.active_reservations -= state.retained_output_reservations;
    for entry in &mut state.dimensions {
        entry.state.current_live_bytes -= entry.state.retained_output_bytes;
        entry.state.retained_output_bytes = 0;
    }
    for phase in &mut state.phase_states {
        phase.current_live_bytes -= phase.retained_output_bytes;
        phase.retained_output_bytes = 0;
    }
    state.retained_output_bytes = 0;
    state.retained_output_reservations = 0;
}

#[cfg(test)]
mod baseline_attachment_tests;

#[cfg(test)]
mod tests;
