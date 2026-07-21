use crate::{CapacityPlan, CapacityPlanError, OperationOwner, ReplacementCapacityPlan};
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::{Arc, Mutex, MutexGuard};

mod startup;

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

/// A linear live-byte reservation issued by one ledger.
///
/// Reservations cannot be cloned or constructed outside this crate.
///
/// ```compile_fail
/// use baozi_operation::{LiveReservation, OperationOwner};
/// struct Owner;
/// impl OperationOwner for Owner {}
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Phase { Build }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Dimension { Bytes }
/// let reservation: LiveReservation<Owner, Phase, Dimension> = LiveReservation {};
/// ```
///
/// ```compile_fail
/// use baozi_operation::{
///     DimensionPolicy, LedgerLimits, OperationControl, OperationCoordinator, OperationOwner,
///     ProgressPhase, ProgressUnit,
/// };
/// struct Owner;
/// impl OperationOwner for Owner {}
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Phase { Build }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Progress { Work }
/// impl ProgressPhase for Progress {
///     fn order(self) -> u32 { 0 }
///     fn stable_id(self) -> &'static str { "work" }
/// }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Unit { Work }
/// impl ProgressUnit for Unit { fn stable_id(self) -> &'static str { "work" } }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Dimension { Bytes }
/// let limits = LedgerLimits::new(
///     8,
///     [(Dimension::Bytes, DimensionPolicy::reservable_bytes(8, 8, 8, 8))],
/// ).unwrap();
/// let mut operation = OperationCoordinator::<Owner, Phase, Progress, Unit, Dimension>::try_new(
///     OperationControl::default(), limits, &[Phase::Build],
/// ).unwrap();
/// let reservation = operation.checkpoint().reserve(Phase::Build, Dimension::Bytes, 1).unwrap();
/// let duplicate = reservation.clone();
/// ```
///
/// ```compile_fail
/// use baozi_operation::{
///     DimensionPolicy, LedgerLimits, OperationControl, OperationCoordinator, OperationOwner,
///     ProgressPhase, ProgressUnit,
/// };
/// struct ImportOwner;
/// impl OperationOwner for ImportOwner {}
/// struct PostProcessOwner;
/// impl OperationOwner for PostProcessOwner {}
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Phase { Build }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Progress { Work }
/// impl ProgressPhase for Progress {
///     fn order(self) -> u32 { 0 }
///     fn stable_id(self) -> &'static str { "work" }
/// }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Unit { Work }
/// impl ProgressUnit for Unit { fn stable_id(self) -> &'static str { "work" } }
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Dimension { Bytes }
/// let limits = || LedgerLimits::new(
///     8,
///     [(Dimension::Bytes, DimensionPolicy::reservable_bytes(8, 8, 8, 8))],
/// ).unwrap();
/// let mut import = OperationCoordinator::<ImportOwner, Phase, Progress, Unit, Dimension>::try_new(
///     OperationControl::default(), limits(), &[Phase::Build],
/// ).unwrap();
/// let mut postprocess = OperationCoordinator::<PostProcessOwner, Phase, Progress, Unit, Dimension>::try_new(
///     OperationControl::default(), limits(), &[Phase::Build],
/// ).unwrap();
/// let mut reservation = import.checkpoint().reserve(Phase::Build, Dimension::Bytes, 1).unwrap();
/// postprocess.transfer_reservation(&mut reservation, Phase::Build).unwrap();
/// ```
#[must_use = "dropping a reservation releases its live byte capacity"]
pub struct LiveReservation<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    core: ReservationCore<O, P, D>,
}

struct ReservationCore<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    owner: Arc<Mutex<LedgerState<P, D>>>,
    phase: P,
    phase_position: usize,
    dimension: D,
    dimension_position: usize,
    bytes: u64,
    active: bool,
    operation_owner: PhantomData<fn() -> O>,
}

impl<O, P, D> ReservationCore<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn new(
        owner: Arc<Mutex<LedgerState<P, D>>>,
        phase: P,
        phase_position: usize,
        dimension: D,
        dimension_position: usize,
        bytes: u64,
    ) -> Self {
        Self {
            owner,
            phase,
            phase_position,
            dimension,
            dimension_position,
            bytes,
            active: true,
            operation_owner: PhantomData,
        }
    }

    fn is_owned_by(&self, owner: &Arc<Mutex<LedgerState<P, D>>>) -> bool {
        Arc::ptr_eq(&self.owner, owner)
    }

    fn release(&mut self) {
        if !self.active {
            return;
        }
        let mut state = lock(&self.owner);
        release_positioned_locked(
            &mut state,
            self.phase_position,
            self.dimension_position,
            self.bytes,
        );
        self.active = false;
    }

    fn release_locked(&mut self, state: &mut LedgerState<P, D>) {
        if !self.active {
            return;
        }
        release_positioned_locked(
            state,
            self.phase_position,
            self.dimension_position,
            self.bytes,
        );
        self.active = false;
    }

    fn release_bound_locked(&mut self, state: &mut LedgerState<P, D>) {
        state.active_reservations -= 1;
        state.current_live_bytes -= self.bytes;
        state.dimensions[self.dimension_position]
            .state
            .current_live_bytes -= self.bytes;
        state.phase_states[self.phase_position].current_live_bytes -= self.bytes;
        self.active = false;
    }
}

impl<O, P, D> Drop for ReservationCore<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        self.release();
    }
}

impl<O, P, D> fmt::Debug for LiveReservation<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord + fmt::Debug,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveReservation")
            .field("phase", &self.core.phase)
            .field("dimension", &self.core.dimension)
            .field("bytes", &self.core.bytes)
            .field("active", &self.core.active)
            .finish_non_exhaustive()
    }
}

impl<O, P, D> LiveReservation<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    /// Returns the live bytes owned by this token.
    pub const fn bytes(&self) -> u64 {
        self.core.bytes
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct StorageCoveringReservation<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    core: ReservationCore<O, P, D>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct OperationAttributionReservation<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    core: ReservationCore<O, P, D>,
    destination: PhantomData<fn() -> Destination>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D, Destination> OperationAttributionReservation<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    pub(crate) const fn bytes(&self) -> u64 {
        self.core.bytes
    }
}

/// Allocation-free qualification for one destination-specific attribution batch.
///
/// Construction performs every capacity and byte calculation. Consuming allocation may still fail
/// because the allocator or live ledger admission rejects the qualified request, but it does not
/// repeat plan arithmetic after the caller crosses its side-effect boundary.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct AttributionBatchPlan<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    phase: P,
    capacity_dimension: D,
    attribution_capacity: CapacityPlan,
    commitment_capacity: CapacityPlan,
    planned_bytes: u64,
    attribution_limit: usize,
    commitment_limit: usize,
    destination: PhantomData<fn() -> (O, Destination)>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D, Destination> AttributionBatchPlan<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    pub(crate) fn try_new(
        phase: P,
        capacity_dimension: D,
        attribution_limit: usize,
        commitment_limit: usize,
    ) -> Result<Self, AttributionBatchCreateError<D>> {
        let attribution_capacity = CapacityPlan::for_type::<
            OperationAttributionReservation<O, P, D, Destination>,
        >(0, 0, attribution_limit)
        .map_err(|_| AttributionBatchCreateError::CapacityOverflow)?;
        let commitment_capacity =
            CapacityPlan::for_type::<OutputCommitment<D, Destination>>(0, 0, commitment_limit)
                .map_err(|_| AttributionBatchCreateError::CapacityOverflow)?;
        let planned_bytes = attribution_capacity
            .envelope_delta_bytes()
            .ok()
            .and_then(|bytes| {
                commitment_capacity
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|commitment_bytes| bytes.checked_add(commitment_bytes))
            })
            .ok_or(AttributionBatchCreateError::CapacityOverflow)?;
        Ok(Self {
            phase,
            capacity_dimension,
            attribution_capacity,
            commitment_capacity,
            planned_bytes,
            attribution_limit,
            commitment_limit,
            destination: PhantomData,
        })
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct AttributionReservationBatch<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    attributions: Vec<OperationAttributionReservation<O, P, D, Destination>>,
    commitments: Vec<OutputCommitment<D, Destination>>,
    capacity_cover: Option<StorageCoveringReservation<O, P, D>>,
    attribution_limit: usize,
    commitment_limit: usize,
    owner: Arc<Mutex<LedgerState<P, D>>>,
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
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct FallbackBatchBaseline<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    fallback: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    evidence: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(crate) struct FallbackBaselineRejection<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    baseline: FallbackBatchBaseline<O, P, D>,
    cause: LedgerCause<D>,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D> FallbackBaselineRejection<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn into_parts(self) -> (FallbackBatchBaseline<O, P, D>, LedgerCause<D>) {
        (self.baseline, self.cause)
    }
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
    pub(crate) const fn new_unbound(
        canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
        evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        fallback: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        evidence: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    ) -> Self {
        Self {
            canonical_assembly,
            evidence_assembly,
            fallback,
            evidence,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        AttributionReservationBatch<O, P, D, CanonicalOutput>,
        AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    ) {
        (
            self.canonical_assembly,
            self.evidence_assembly,
            self.fallback,
            self.evidence,
        )
    }

    pub(crate) fn try_prepare_fallback_sources(&mut self) -> Result<(), ()> {
        if !self
            .evidence_assembly
            .can_absorb_sources([Some(&self.evidence), Some(&self.fallback)])
        {
            return Err(());
        }
        self.evidence_assembly
            .absorb_proven_retaining_source_storage(&mut self.evidence);
        self.evidence_assembly
            .absorb_proven_retaining_source_storage(&mut self.fallback);
        Ok(())
    }

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

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<O, P, D, Destination> AttributionReservationBatch<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    pub(crate) fn has_attribution_slot(&self) -> bool {
        self.attributions.len() < self.attribution_limit
            && self.attributions.len() < self.attributions.capacity()
    }

    fn has_commitment_slot(&self) -> bool {
        self.commitments.len() < self.commitment_limit
            && self.commitments.len() < self.commitments.capacity()
    }

    pub(crate) fn try_push_attribution(
        &mut self,
        token: OperationAttributionReservation<O, P, D, Destination>,
    ) -> Result<(), OperationAttributionReservation<O, P, D, Destination>> {
        let Some(cover) = self.capacity_cover.as_ref() else {
            return Err(token);
        };
        if !self.has_attribution_slot()
            || !Arc::ptr_eq(&self.owner, &cover.core.owner)
            || !token.core.is_owned_by(&self.owner)
        {
            return Err(token);
        }
        self.attributions.push(token);
        Ok(())
    }

    fn push_positioned_commitment(&mut self, commitment: OutputCommitment<D, Destination>) {
        debug_assert!(self.has_commitment_slot());
        debug_assert_ne!(commitment.dimension_position, usize::MAX);
        self.commitments.push(commitment);
    }

    pub(crate) fn attribution_len(&self) -> usize {
        self.attributions.len()
    }

    pub(crate) fn attributed_bytes(&self) -> Option<u64> {
        self.attributions
            .iter()
            .try_fold(0_u64, |total, attribution| {
                total.checked_add(attribution.bytes())
            })
    }

    fn absorb_proven_retaining_source_storage(&mut self, other: &mut Self) {
        self.attributions.append(&mut other.attributions);
        self.commitments.append(&mut other.commitments);
    }

    fn can_absorb_sources<const N: usize>(&self, sources: [Option<&Self>; N]) -> bool {
        let Some(attribution_len) = sources
            .into_iter()
            .flatten()
            .try_fold(self.attributions.len(), |length, source| {
                length.checked_add(source.attributions.len())
            })
        else {
            return false;
        };
        let Some(commitment_len) = sources
            .into_iter()
            .flatten()
            .try_fold(self.commitments.len(), |length, source| {
                length.checked_add(source.commitments.len())
            })
        else {
            return false;
        };
        if attribution_len > self.attribution_limit
            || attribution_len > self.attributions.capacity()
            || commitment_len > self.commitment_limit
            || commitment_len > self.commitments.capacity()
        {
            return false;
        }
        true
    }

    fn combined_attribution_count<const N: usize>(
        &self,
        sources: [Option<&Self>; N],
    ) -> Option<u64> {
        sources.into_iter().flatten().try_fold(
            u64::try_from(self.attributions.len()).ok()?,
            |count, source| count.checked_add(u64::try_from(source.attributions.len()).ok()?),
        )
    }

    fn combined_attribution_bytes<const N: usize>(
        &self,
        sources: [Option<&Self>; N],
    ) -> Option<u64> {
        sources
            .into_iter()
            .flatten()
            .try_fold(self.attribution_bytes()?, |bytes, source| {
                bytes.checked_add(source.attribution_bytes()?)
            })
    }

    pub(crate) fn account_capacity_bytes(&self) -> u64 {
        self.capacity_cover
            .as_ref()
            .map_or(0, |cover| cover.core.bytes)
    }

    fn validate_owner(&self, owner: &Arc<Mutex<LedgerState<P, D>>>) -> bool {
        Arc::ptr_eq(&self.owner, owner)
            && self
                .attributions
                .iter()
                .all(|token| token.core.active && token.core.is_owned_by(owner))
            && self
                .capacity_cover
                .as_ref()
                .is_none_or(|cover| cover.core.active && cover.core.is_owned_by(owner))
    }

    fn is_empty_storage_shell(&self) -> bool {
        self.attributions.is_empty() && self.commitments.is_empty()
    }

    fn attribution_bytes(&self) -> Option<u64> {
        self.attributions
            .iter()
            .try_fold(0_u64, |total, token| total.checked_add(token.core.bytes))
    }

    fn active_count(&self) -> Option<u64> {
        u64::try_from(self.attributions.len())
            .ok()?
            .checked_add(u64::from(self.capacity_cover.is_some()))
    }

    fn live_bytes(&self) -> Option<u64> {
        self.attribution_bytes()?
            .checked_add(self.account_capacity_bytes())
    }

    fn positioned_dimension_bytes(&self, position: usize) -> Option<u64> {
        let attribution = self
            .attributions
            .iter()
            .filter(|token| token.core.dimension_position == position)
            .try_fold(0_u64, |total, token| total.checked_add(token.core.bytes))?;
        let capacity = self
            .capacity_cover
            .as_ref()
            .filter(|cover| cover.core.dimension_position == position)
            .map_or(0, |cover| cover.core.bytes);
        attribution.checked_add(capacity)
    }

    fn positioned_phase_bytes(&self, position: usize) -> Option<u64> {
        let attribution = self
            .attributions
            .iter()
            .filter(|token| token.core.phase_position == position)
            .try_fold(0_u64, |total, token| total.checked_add(token.core.bytes))?;
        let capacity = self
            .capacity_cover
            .as_ref()
            .filter(|cover| cover.core.phase_position == position)
            .map_or(0, |cover| cover.core.bytes);
        attribution.checked_add(capacity)
    }

    fn positioned_commitment_amount(&self, position: usize) -> Option<u64> {
        self.commitments
            .iter()
            .filter(|commitment| commitment.dimension_position == position)
            .try_fold(0_u64, |total, commitment| {
                total.checked_add(commitment.amount)
            })
    }

    fn bound_positions_are_in_bounds(&self, dimension_count: usize, phase_count: usize) -> bool {
        self.attributions.iter().all(|token| {
            token.core.dimension_position < dimension_count
                && token.core.phase_position < phase_count
        }) && self.capacity_cover.as_ref().is_none_or(|cover| {
            cover.core.dimension_position < dimension_count
                && cover.core.phase_position < phase_count
        }) && self
            .commitments
            .iter()
            .all(|commitment| commitment.dimension_position < dimension_count)
    }

    fn discard_positions_are_in_bounds(&self, dimension_count: usize, phase_count: usize) -> bool {
        self.attributions.iter().all(|token| {
            token.core.dimension_position < dimension_count
                && token.core.phase_position < phase_count
        }) && self.capacity_cover.as_ref().is_none_or(|cover| {
            cover.core.dimension_position < dimension_count
                && cover.core.phase_position < phase_count
        })
    }

    fn release_bound_attributions(&mut self, state: &mut LedgerState<P, D>) {
        for token in &mut self.attributions {
            token.core.release_bound_locked(state);
        }
    }

    fn destroy_storage_then_release_bound_cover(&mut self, state: &mut LedgerState<P, D>) {
        drop(std::mem::take(&mut self.attributions));
        drop(std::mem::take(&mut self.commitments));
        if let Some(mut capacity_cover) = self.capacity_cover.take() {
            capacity_cover.core.release_bound_locked(state);
            drop(capacity_cover);
        }
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

/// Immutable evidence for one accounting dimension.
pub(crate) struct DimensionSnapshotScalars {
    pub(crate) attempted: u64,
    pub(crate) consumed: u64,
    pub(crate) current_live_bytes: u64,
    pub(crate) peak_live_bytes: u64,
    pub(crate) pending_publication: u64,
    pub(crate) committed: u64,
    pub(crate) committed_canonical_output: u64,
    pub(crate) published_outcome_evidence: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DimensionSnapshot {
    attempted: u64,
    consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

impl DimensionSnapshot {
    pub(crate) const fn from_terminal_evidence(values: DimensionSnapshotScalars) -> Self {
        Self {
            attempted: values.attempted,
            consumed: values.consumed,
            current_live_bytes: values.current_live_bytes,
            peak_live_bytes: values.peak_live_bytes,
            pending_publication: values.pending_publication,
            committed: values.committed,
            committed_canonical_output: values.committed_canonical_output,
            published_outcome_evidence: values.published_outcome_evidence,
        }
    }

    /// Returns cumulative attempted work.
    pub const fn attempted(&self) -> u64 {
        self.attempted
    }

    /// Returns cumulative consumed work.
    pub const fn consumed(&self) -> u64 {
        self.consumed
    }

    /// Returns currently retained live bytes.
    pub const fn current_live_bytes(&self) -> u64 {
        self.current_live_bytes
    }

    /// Returns the dimension's live-byte high-water mark.
    pub const fn peak_live_bytes(&self) -> u64 {
        self.peak_live_bytes
    }

    /// Returns output admitted by pending transactions but not yet published.
    pub const fn pending_publication(&self) -> u64 {
        self.pending_publication
    }

    /// Returns durable published output amounts in this dimension's own unit.
    pub const fn committed(&self) -> u64 {
        self.committed
    }

    /// Returns output committed through the destination-typed canonical path.
    pub const fn committed_canonical_output(&self) -> u64 {
        self.committed_canonical_output
    }

    /// Returns evidence published through the destination-typed failure/report path.
    pub const fn published_outcome_evidence(&self) -> u64 {
        self.published_outcome_evidence
    }
}

/// Immutable live-byte evidence for one owner-defined ledger phase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhaseSnapshot {
    current_live_bytes: u64,
    peak_live_bytes: u64,
}

impl PhaseSnapshot {
    pub(crate) const fn from_terminal_evidence(
        current_live_bytes: u64,
        peak_live_bytes: u64,
    ) -> Self {
        Self {
            current_live_bytes,
            peak_live_bytes,
        }
    }

    /// Returns currently retained live bytes in this phase.
    pub const fn current_live_bytes(&self) -> u64 {
        self.current_live_bytes
    }

    /// Returns this phase's live-byte high-water mark.
    pub const fn peak_live_bytes(&self) -> u64 {
        self.peak_live_bytes
    }
}

/// Immutable owner-neutral operation ledger evidence.
///
/// Dimension and phase entries are stored in stable key order. A clean final snapshot reports zero
/// current evidence bytes because its pre-admitted staging vectors transfer to the outcome while
/// the consumed ledger releases its private indexes. The evidence peak still includes every
/// staging, interim snapshot, and pending-publication envelope used by the operation.
#[derive(Debug, PartialEq, Eq)]
pub struct LedgerSnapshot<P, D> {
    dimensions: Vec<(D, DimensionSnapshot)>,
    phases: Vec<(P, PhaseSnapshot)>,
    last_completed_phase: Option<P>,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
}

impl<P: Copy + Ord, D: Copy + Ord> LedgerSnapshot<P, D> {
    /// Returns the latest phase explicitly completed by the operation owner.
    pub const fn last_completed_phase(&self) -> Option<P> {
        self.last_completed_phase
    }

    /// Returns evidence for one owner-defined dimension.
    pub fn dimension(&self, dimension: D) -> Option<&DimensionSnapshot> {
        self.dimensions
            .binary_search_by_key(&dimension, |(key, _)| *key)
            .ok()
            .map(|position| &self.dimensions[position].1)
    }

    /// Returns live-byte evidence for one owner-defined ledger phase.
    pub fn phase(&self, phase: P) -> Option<&PhaseSnapshot> {
        self.phases
            .binary_search_by_key(&phase, |(key, _)| *key)
            .ok()
            .map(|position| &self.phases[position].1)
    }

    /// Returns current operation-owned live bytes.
    pub const fn current_live_bytes(&self) -> u64 {
        self.current_live_bytes
    }

    /// Returns the operation-wide live-byte high-water mark.
    pub const fn peak_live_bytes(&self) -> u64 {
        self.peak_live_bytes
    }

    /// Returns active linear reservation capabilities at the capture boundary.
    pub const fn active_reservations(&self) -> u64 {
        self.active_reservations
    }

    /// Returns ledger-owned evidence bytes retained at the capture boundary.
    pub const fn current_evidence_bytes(&self) -> u64 {
        self.current_evidence_bytes
    }

    /// Returns the ledger evidence storage high-water mark.
    pub const fn peak_evidence_bytes(&self) -> u64 {
        self.peak_evidence_bytes
    }

    /// Iterates dimension evidence in stable key order.
    pub fn dimensions(&self) -> impl ExactSizeIterator<Item = (D, &DimensionSnapshot)> {
        self.dimensions
            .iter()
            .map(|(dimension, snapshot)| (*dimension, snapshot))
    }

    /// Iterates phase evidence in stable key order.
    pub fn phases(&self) -> impl ExactSizeIterator<Item = (P, &PhaseSnapshot)> {
        self.phases
            .iter()
            .map(|(phase, snapshot)| (*phase, snapshot))
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

struct SnapshotEvidenceAssembly<'a, P, D> {
    state: &'a mut LedgerState<P, D>,
    dimensions: Vec<(D, DimensionSnapshot)>,
    phases: Vec<(P, PhaseSnapshot)>,
    evidence_before: u64,
    armed: bool,
}

impl<'a, P, D> SnapshotEvidenceAssembly<'a, P, D> {
    fn new(state: &'a mut LedgerState<P, D>) -> Self {
        Self {
            evidence_before: state.current_evidence_bytes,
            state,
            dimensions: Vec::new(),
            phases: Vec::new(),
            armed: true,
        }
    }
}

impl<P: Copy, D> SnapshotEvidenceAssembly<'_, P, D> {
    fn finish(mut self) -> LedgerSnapshot<P, D> {
        let outgoing_bytes = self
            .state
            .current_evidence_bytes
            .saturating_sub(self.evidence_before);
        let snapshot = LedgerSnapshot {
            dimensions: std::mem::take(&mut self.dimensions),
            phases: std::mem::take(&mut self.phases),
            last_completed_phase: self
                .state
                .last_completed_phase
                .map(|completed| completed.phase),
            current_live_bytes: self.state.current_live_bytes,
            peak_live_bytes: self.state.peak_live_bytes,
            active_reservations: self.state.active_reservations,
            current_evidence_bytes: self
                .state
                .current_evidence_bytes
                .saturating_sub(outgoing_bytes),
            peak_evidence_bytes: self.state.peak_evidence_bytes,
        };
        self.armed = false;
        release_evidence(self.state, outgoing_bytes);
        snapshot
    }
}

impl<P, D> Drop for SnapshotEvidenceAssembly<'_, P, D> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        drop(std::mem::take(&mut self.phases));
        drop(std::mem::take(&mut self.dimensions));
        let outgoing_bytes = self
            .state
            .current_evidence_bytes
            .saturating_sub(self.evidence_before);
        release_evidence(self.state, outgoing_bytes);
    }
}

fn try_snapshot<P, D>(
    state: &mut LedgerState<P, D>,
) -> Result<LedgerSnapshot<P, D>, LedgerEvidenceError>
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    if !phase_catalog_shape_is_valid(state) {
        return Err(LedgerEvidenceError::PhaseCatalogShapeMismatch);
    }
    let mut assembly = SnapshotEvidenceAssembly::new(state);
    let dimension_count = assembly.state.dimensions.len();
    let evidence_byte_limit = assembly.state.evidence_byte_limit;
    try_grow_evidence_vec(
        &mut assembly.dimensions,
        dimension_count,
        &mut assembly.state.current_evidence_bytes,
        &mut assembly.state.peak_evidence_bytes,
        evidence_byte_limit,
        LedgerEvidencePurpose::Snapshot,
    )?;
    for entry in &assembly.state.dimensions {
        assembly
            .dimensions
            .push((entry.dimension, dimension_snapshot(entry.state)));
    }

    let phase_count = assembly.state.phases.len();
    try_grow_evidence_vec(
        &mut assembly.phases,
        phase_count,
        &mut assembly.state.current_evidence_bytes,
        &mut assembly.state.peak_evidence_bytes,
        evidence_byte_limit,
        LedgerEvidencePurpose::Snapshot,
    )?;
    for entry in &assembly.state.phases {
        assembly.phases.push((
            entry.phase,
            phase_snapshot(assembly.state.phase_states[entry.state_position]),
        ));
    }
    Ok(assembly.finish())
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn validate_final_snapshot_shape<P: Copy + PartialEq, D>(
    state: &LedgerState<P, D>,
) -> Result<(), LedgerCause<D>> {
    if state.dimensions.len() != state.dimension_snapshot_staging.len()
        || !phase_catalog_shape_is_valid(state)
    {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn typed_terminal_snapshot_positions_are_valid<P, D>(state: &LedgerState<P, D>) -> bool {
    let phase_count = state.phases.len();
    state.dimensions.len() == state.dimension_snapshot_staging.len()
        && phase_count == state.phase_states.len()
        && phase_count == state.phase_snapshot_staging.len()
        && (0..phase_count).all(|state_position| {
            state
                .phases
                .iter()
                .filter(|entry| entry.state_position == state_position)
                .count()
                == 1
        })
        && state
            .last_completed_phase
            .as_ref()
            .is_none_or(|completed| completed.state_position < phase_count)
}

fn phase_catalog_shape_is_valid<P: Copy + PartialEq, D>(state: &LedgerState<P, D>) -> bool {
    let phase_count = state.phases.len();
    if phase_count != state.phase_states.len()
        || phase_count != state.phase_snapshot_staging.len()
        || state
            .phases
            .iter()
            .zip(&state.phase_snapshot_staging)
            .any(|(entry, staging)| entry.phase != staging.0)
        || (0..phase_count).any(|state_position| {
            state
                .phases
                .iter()
                .filter(|entry| entry.state_position == state_position)
                .count()
                != 1
        })
    {
        return false;
    }
    state.last_completed_phase.is_none_or(|completed| {
        state.phases.iter().any(|entry| {
            entry.state_position == completed.state_position && entry.phase == completed.phase
        })
    })
}

fn take_final_snapshot<P, D>(state: &mut LedgerState<P, D>, clean: bool) -> LedgerSnapshot<P, D>
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    for (entry, staging) in state
        .dimensions
        .iter()
        .zip(state.dimension_snapshot_staging.iter_mut())
    {
        staging.0 = entry.dimension;
        staging.1 = dimension_snapshot(entry.state);
    }
    for (entry, staging) in state
        .phases
        .iter()
        .zip(state.phase_snapshot_staging.iter_mut())
    {
        staging.0 = entry.phase;
        staging.1 = state
            .phase_states
            .get(entry.state_position)
            .copied()
            .map(phase_snapshot)
            .unwrap_or_default();
    }
    let outgoing_bytes = capacity_bytes(&state.dimension_snapshot_staging)
        .unwrap_or(0)
        .saturating_add(capacity_bytes(&state.phase_snapshot_staging).unwrap_or(0));
    release_evidence(state, outgoing_bytes);
    let retained_evidence = if clean {
        0
    } else {
        state.current_evidence_bytes
    };
    let last_completed_phase = state.last_completed_phase.map(|completed| completed.phase);
    LedgerSnapshot {
        dimensions: std::mem::take(&mut state.dimension_snapshot_staging),
        phases: std::mem::take(&mut state.phase_snapshot_staging),
        last_completed_phase,
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
        active_reservations: state.active_reservations,
        current_evidence_bytes: retained_evidence,
        peak_evidence_bytes: state.peak_evidence_bytes,
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn take_typed_final_snapshot<P: Copy, D: Copy>(
    state: &mut LedgerState<P, D>,
    last_completed_phase: Option<P>,
) -> LedgerSnapshot<P, D> {
    for (entry, staging) in state
        .dimensions
        .iter()
        .zip(state.dimension_snapshot_staging.iter_mut())
    {
        staging.0 = entry.dimension;
        staging.1 = dimension_snapshot(entry.state);
    }
    for (entry, staging) in state
        .phases
        .iter()
        .zip(state.phase_snapshot_staging.iter_mut())
    {
        staging.0 = entry.phase;
        staging.1 = phase_snapshot(state.phase_states[entry.state_position]);
    }
    LedgerSnapshot {
        dimensions: std::mem::take(&mut state.dimension_snapshot_staging),
        phases: std::mem::take(&mut state.phase_snapshot_staging),
        last_completed_phase,
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
        active_reservations: state.active_reservations,
        current_evidence_bytes: 0,
        peak_evidence_bytes: state.peak_evidence_bytes,
    }
}

const fn dimension_snapshot(state: DimensionState) -> DimensionSnapshot {
    DimensionSnapshot {
        attempted: state.attempted,
        consumed: state.consumed,
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
        pending_publication: state.pending_publication,
        committed: state.committed,
        committed_canonical_output: state.committed_canonical_output,
        published_outcome_evidence: state.published_outcome_evidence,
    }
}

const fn phase_snapshot(state: PhaseState) -> PhaseSnapshot {
    PhaseSnapshot {
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ControlCause, OperationControl, OperationCoordinator, ProgressPhase, ProgressStorageError,
        ProgressUnit,
    };
    use std::sync::atomic::AtomicUsize;

    fn ledger_production_source() -> String {
        let test_attribute = ["#[cfg(", "test)]"].concat();
        [include_str!("ledger.rs"), include_str!("ledger/startup.rs")]
            .into_iter()
            .map(|source| {
                source
                    .rsplit_once(&test_attribute)
                    .map_or(source, |(production, _)| production)
            })
            .collect()
    }

    #[derive(Debug)]
    struct Owner;

    impl OperationOwner for Owner {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Phase {
        Build,
        Finalize,
        Unknown,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Dimension {
        Bytes,
        Records,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Progress {
        Work,
        Other,
    }

    impl ProgressPhase for Progress {
        fn order(self) -> u32 {
            match self {
                Self::Work => 0,
                Self::Other => 1,
            }
        }

        fn stable_id(self) -> &'static str {
            match self {
                Self::Work => "work",
                Self::Other => "other",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Unit {
        Work,
    }

    impl ProgressUnit for Unit {
        fn stable_id(self) -> &'static str {
            "work"
        }
    }

    fn limits() -> LedgerLimits<Dimension> {
        LedgerLimits::new(
            16,
            [
                (
                    Dimension::Bytes,
                    DimensionPolicy::reservable_bytes(16, 16, 16, 16),
                ),
                (Dimension::Records, DimensionPolicy::counter(16, 16, 16)),
            ],
        )
        .expect("test policy")
    }

    const COMPLETE_PHASES: &[Phase] = &[Phase::Build, Phase::Finalize, Phase::Unknown];

    fn test_ledger(limits: LedgerLimits<Dimension>) -> Ledger<Owner, Phase, Dimension> {
        Ledger::try_new(limits, COMPLETE_PHASES).expect("complete test phase catalog")
    }

    #[test]
    fn runtime_ledger_has_no_dynamic_phase_growth_rail() {
        let production = ledger_production_source();
        assert!(!production.contains("PhaseCatalogState"));
        assert!(!production.contains("fn ensure_phase"));
        assert!(!production.contains("try_new_with_phase_catalog"));
        let lookup = production
            .split_once("fn phase_position")
            .expect("frozen phase lookup exists")
            .1
            .split_once("fn try_insert_phase_catalog_entry")
            .expect("frozen phase lookup is bounded")
            .0;
        assert!(lookup.contains(".iter()"));
        assert!(lookup.contains(".find("));
        assert!(lookup.contains("LedgerCause::UnknownPhase"));
        assert!(!lookup.contains("binary_search"));
        assert!(!lookup.contains("try_grow_evidence_vec"));
        assert!(!lookup.contains("push("));
        assert!(!lookup.contains("insert("));
    }

    #[test]
    fn frozen_phase_catalog_rejects_duplicates_instead_of_rewriting_lifecycle_order() {
        let rejected = Ledger::<Owner, Phase, Dimension>::try_new(
            limits(),
            &[Phase::Build, Phase::Finalize, Phase::Build],
        );

        assert!(matches!(rejected, Err(LedgerCause::DuplicatePhase)));
    }

    #[test]
    fn frozen_phase_catalog_prebuilds_complete_final_evidence_shape() {
        let ledger =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("frozen phase catalog");

        let state = lock(&ledger.state);
        assert_eq!(state.phases.len(), 2);
        assert_eq!(state.phase_states.len(), 2);
        assert_eq!(state.phase_snapshot_staging.len(), 2);
        assert_eq!(state.phases[0].phase, Phase::Build);
        assert_eq!(state.phases[1].phase, Phase::Finalize);
        assert_eq!(state.phase_snapshot_staging[0].0, Phase::Build);
        assert_eq!(state.phase_snapshot_staging[1].0, Phase::Finalize);
        drop(state);

        assert_eq!(
            ledger.phase_evidence(Phase::Build),
            Some(PhaseSnapshot::default())
        );
        assert_eq!(
            ledger.phase_evidence(Phase::Finalize),
            Some(PhaseSnapshot::default())
        );
    }

    #[test]
    fn frozen_phase_catalog_rejects_unknown_phase_without_allocation_or_counter_mutation() {
        let ledger =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("frozen phase catalog");
        let before = {
            let state = lock(&ledger.state);
            (
                state.phases.len(),
                state.phases.capacity(),
                state.phase_states.len(),
                state.phase_states.capacity(),
                state.phase_snapshot_staging.len(),
                state.phase_snapshot_staging.capacity(),
                state.current_evidence_bytes,
                state.peak_evidence_bytes,
                state.current_live_bytes,
                state.peak_live_bytes,
                state.active_reservations,
                dimension_snapshot(state.dimensions[0].state),
            )
        };

        let rejected = ledger.reserve(Phase::Unknown, Dimension::Bytes, 1);

        assert!(matches!(rejected, Err(LedgerCause::UnknownPhase)));
        let after = {
            let state = lock(&ledger.state);
            (
                state.phases.len(),
                state.phases.capacity(),
                state.phase_states.len(),
                state.phase_states.capacity(),
                state.phase_snapshot_staging.len(),
                state.phase_snapshot_staging.capacity(),
                state.current_evidence_bytes,
                state.peak_evidence_bytes,
                state.current_live_bytes,
                state.peak_live_bytes,
                state.active_reservations,
                dimension_snapshot(state.dimensions[0].state),
            )
        };
        assert_eq!(after, before);
    }

    #[test]
    fn frozen_phase_completion_is_monotonic_by_catalog_position_and_preserves_rejections() {
        let ledger =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Finalize, Phase::Build])
                .expect("frozen phase catalog");

        let initial = ledger.try_snapshot().expect("initial snapshot fits");
        assert_eq!(initial.last_completed_phase(), None);
        assert_eq!(
            initial.phases().map(|(phase, _)| phase).collect::<Vec<_>>(),
            vec![Phase::Finalize, Phase::Build],
            "the caller-declared catalog order is the operation lifecycle"
        );

        ledger
            .try_complete_phase(Phase::Finalize)
            .expect("first catalog phase completes");
        ledger
            .try_complete_phase(Phase::Finalize)
            .expect("repeating the completed phase is idempotent");
        assert_eq!(
            ledger
                .try_snapshot()
                .expect("idempotent snapshot fits")
                .last_completed_phase(),
            Some(Phase::Finalize)
        );

        ledger
            .try_complete_phase(Phase::Build)
            .expect("later catalog position completes even when its key sorts earlier");
        assert_eq!(
            ledger.try_complete_phase(Phase::Finalize),
            Err(PhaseCompletionError::Regression {
                last_completed_phase: Phase::Build,
                phase: Phase::Finalize,
            })
        );
        assert_eq!(
            ledger.try_complete_phase(Phase::Unknown),
            Err(PhaseCompletionError::UnknownPhase {
                phase: Phase::Unknown,
            })
        );
        assert_eq!(
            ledger
                .try_snapshot()
                .expect("rejection-preserving snapshot fits")
                .last_completed_phase(),
            Some(Phase::Build)
        );
    }

    #[test]
    fn phase_completion_obeys_pending_rollback_and_published_boundaries() {
        let ledger =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("frozen phase catalog");
        let mut first_batch = PublicationReservationBatch::default();
        let pending = ledger
            .prepare_publication(&mut first_batch, &[])
            .expect("empty publication covers zero live bytes");

        assert_eq!(
            ledger.try_complete_phase(Phase::Build),
            Err(PhaseCompletionError::PublicationPending)
        );
        assert_eq!(
            ledger
                .try_snapshot()
                .expect("pending snapshot fits")
                .last_completed_phase(),
            None
        );

        pending.rollback();
        ledger
            .try_complete_phase(Phase::Build)
            .expect("rollback reopens semantic completion");

        let mut replacement_batch = PublicationReservationBatch::default();
        let mut published = ledger
            .prepare_publication(&mut replacement_batch, &[])
            .expect("replacement publication");
        ledger
            .publish(&mut published)
            .expect("publication becomes durable");
        assert_eq!(
            ledger.try_complete_phase(Phase::Finalize),
            Err(PhaseCompletionError::PublicationCompleted)
        );
        assert_eq!(
            ledger
                .try_snapshot()
                .expect("published snapshot fits")
                .last_completed_phase(),
            Some(Phase::Build)
        );
    }

    fn phase_shape_probe(
        ledger: &Ledger<Owner, Phase, Dimension>,
    ) -> (Vec<usize>, Option<(Phase, usize)>, u64, u64) {
        let state = lock(&ledger.state);
        (
            state
                .phases
                .iter()
                .map(|entry| entry.state_position)
                .collect(),
            state
                .last_completed_phase
                .map(|completed| (completed.phase, completed.state_position)),
            state.current_evidence_bytes,
            state.peak_evidence_bytes,
        )
    }

    #[test]
    fn phase_catalog_shape_faults_reject_completion_and_snapshot_without_mutation() {
        let duplicate =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("duplicate fault ledger");
        {
            let mut state = lock(&duplicate.state);
            state.phases[1].state_position = state.phases[0].state_position;
        }
        let duplicate_before = phase_shape_probe(&duplicate);
        assert_eq!(
            duplicate.try_complete_phase(Phase::Build),
            Err(PhaseCompletionError::CatalogShapeMismatch)
        );
        assert_eq!(
            duplicate.try_snapshot(),
            Err(LedgerEvidenceError::PhaseCatalogShapeMismatch)
        );
        assert_eq!(phase_shape_probe(&duplicate), duplicate_before);

        let out_of_range =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("out-of-range fault ledger");
        out_of_range
            .try_complete_phase(Phase::Build)
            .expect("build completes before fault injection");
        {
            let mut state = lock(&out_of_range.state);
            state.phases[1].state_position = state.phase_states.len();
        }
        let out_of_range_before = phase_shape_probe(&out_of_range);
        assert_eq!(
            out_of_range.try_complete_phase(Phase::Build),
            Err(PhaseCompletionError::CatalogShapeMismatch),
            "idempotent completion still validates the whole catalog"
        );
        assert_eq!(
            out_of_range.try_complete_phase(Phase::Finalize),
            Err(PhaseCompletionError::CatalogShapeMismatch),
            "forward completion still validates the whole catalog"
        );
        assert_eq!(
            out_of_range.try_snapshot(),
            Err(LedgerEvidenceError::PhaseCatalogShapeMismatch)
        );
        assert_eq!(phase_shape_probe(&out_of_range), out_of_range_before);

        let missing_last =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("missing-last fault ledger");
        missing_last
            .try_complete_phase(Phase::Finalize)
            .expect("finalize completes before fault injection");
        {
            let mut state = lock(&missing_last.state);
            state
                .last_completed_phase
                .as_mut()
                .expect("completed phase")
                .state_position = state.phase_states.len();
        }
        let missing_last_before = phase_shape_probe(&missing_last);
        assert_eq!(
            missing_last.try_complete_phase(Phase::Build),
            Err(PhaseCompletionError::CatalogShapeMismatch)
        );
        assert_eq!(
            missing_last.try_snapshot(),
            Err(LedgerEvidenceError::PhaseCatalogShapeMismatch)
        );
        assert_eq!(phase_shape_probe(&missing_last), missing_last_before);
    }

    #[test]
    fn final_shape_failure_preserves_the_stored_completed_phase_key() {
        let ledger =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("frozen phase catalog");
        ledger
            .try_complete_phase(Phase::Finalize)
            .expect("finalize completes before fault injection");
        {
            let mut state = lock(&ledger.state);
            state
                .last_completed_phase
                .as_mut()
                .expect("completed phase")
                .state_position = state.phase_states.len();
        }

        let error = ledger
            .finish()
            .expect_err("invalid completed pair rejects finalization");

        assert_eq!(error.cause(), &LedgerCause::FinalSnapshotShapeMismatch);
        assert_eq!(
            error.snapshot().last_completed_phase(),
            Some(Phase::Finalize)
        );
    }

    #[test]
    fn frozen_dimension_count_matches_the_canonical_dimension_vocabulary() {
        let limits = limits();

        assert_eq!(limits.dimension_count(), 2);
        assert!(limits.dimension(Dimension::Bytes).is_some());
        assert!(limits.dimension(Dimension::Records).is_some());
    }

    #[test]
    fn consumed_batch_is_atomic_and_aggregates_duplicate_dimensions() {
        let ledger = test_ledger(limits());
        ledger
            .charge_attempted(Dimension::Bytes, 2)
            .expect("byte work must be admitted");
        ledger
            .charge_attempted(Dimension::Records, 1)
            .expect("record work must be admitted");

        assert!(matches!(
            ledger.charge_consumed_batch(&[
                (Dimension::Bytes, 1),
                (Dimension::Bytes, 1),
                (Dimension::Records, 2),
            ]),
            Err(LedgerCause::ConsumedExceedsAttempted {
                dimension: Dimension::Records,
                consumed: 2,
                attempted: 1,
            })
        ));
        assert_eq!(
            ledger
                .dimension_evidence(Dimension::Bytes)
                .expect("byte evidence")
                .consumed(),
            0,
            "a later batch failure must not retain an earlier charge"
        );
        assert_eq!(
            ledger
                .dimension_evidence(Dimension::Records)
                .expect("record evidence")
                .consumed(),
            0
        );

        assert!(matches!(
            ledger.charge_consumed_batch(&[(Dimension::Records, 1), (Dimension::Bytes, 1),]),
            Err(LedgerCause::ConsumedBatchOutOfOrder {
                previous: Dimension::Records,
                dimension: Dimension::Bytes,
            })
        ));

        ledger
            .charge_consumed_batch(&[
                (Dimension::Bytes, 1),
                (Dimension::Bytes, 1),
                (Dimension::Records, 1),
            ])
            .expect("the admitted batch must commit");
        assert_eq!(
            ledger
                .dimension_evidence(Dimension::Bytes)
                .expect("byte evidence")
                .consumed(),
            2
        );
        assert_eq!(
            ledger
                .dimension_evidence(Dimension::Records)
                .expect("record evidence")
                .consumed(),
            1
        );
    }

    #[test]
    fn observed_batch_preserves_the_ordered_non_refundable_prefix_on_failure() {
        let ledger = test_ledger(limits());
        ledger
            .charge_attempted(Dimension::Records, 16)
            .expect("record attempted budget must reach its exact limit");

        assert!(matches!(
            ledger.charge_observed_batch(&[(Dimension::Bytes, 1), (Dimension::Records, 1)]),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Records,
                value: 17,
                limit: 16,
            })
        ));
        let bytes = ledger
            .dimension_evidence(Dimension::Bytes)
            .expect("byte evidence");
        assert_eq!(bytes.attempted(), 1);
        assert_eq!(bytes.consumed(), 1);
        let records = ledger
            .dimension_evidence(Dimension::Records)
            .expect("record evidence");
        assert_eq!(records.attempted(), 17);
        assert_eq!(records.consumed(), 0);
    }

    #[test]
    fn dimension_policy_tightening_is_monotonic_and_preserves_existing_evidence() {
        let ledger = test_ledger(limits());
        ledger
            .tighten_dimension_policy(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(8, 8, 8, 8),
            )
            .expect("the initial policy must tighten");
        ledger
            .tighten_dimension_policy(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(12, 12, 12, 12),
            )
            .expect("a numerical relaxation request must preserve the tighter policy");
        ledger
            .charge_attempted(Dimension::Bytes, 8)
            .expect("the tightened attempted limit remains usable");
        assert!(matches!(
            ledger.charge_attempted(Dimension::Bytes, 1),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Bytes,
                value: 9,
                limit: 8,
            })
        ));

        let reservation = ledger
            .reserve(Phase::Build, Dimension::Bytes, 4)
            .expect("live state below the tightened bound must remain admitted");
        let before = ledger
            .dimension_evidence(Dimension::Bytes)
            .expect("dimension evidence must remain available");
        assert!(matches!(
            ledger.tighten_dimension_policy(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(7, 7, 7, 3),
            ),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Bytes,
                value: 9,
                limit: 7,
            })
        ));
        assert_eq!(
            ledger
                .dimension_evidence(Dimension::Bytes)
                .expect("failed admission must preserve exact evidence"),
            before,
            "policy tightening must not fabricate a second ledger or rewrite prior evidence"
        );
        assert!(matches!(
            ledger.charge_attempted(Dimension::Bytes, 0),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Bytes,
                value: 9,
                limit: 7,
            })
        ));
        assert!(matches!(
            ledger.tighten_dimension_policy(Dimension::Bytes, DimensionPolicy::counter(7, 7, 7)),
            Err(LedgerCause::DimensionPolicyShapeMismatch {
                dimension: Dimension::Bytes,
            })
        ));
        drop(reservation);
    }

    #[test]
    fn snapshot_failure_cleanup_is_structurally_storage_before_accounting() {
        let production = ledger_production_source();
        let cleanup = production
            .split_once("impl<P, D> Drop for SnapshotEvidenceAssembly")
            .expect("snapshot evidence cleanup owner exists")
            .1
            .split_once("fn try_snapshot")
            .expect("snapshot cleanup precedes snapshot construction")
            .0;
        let phase_storage = cleanup
            .find("drop(std::mem::take(&mut self.phases))")
            .expect("phase snapshot storage is destroyed explicitly");
        let dimension_storage = cleanup
            .find("drop(std::mem::take(&mut self.dimensions))")
            .expect("dimension snapshot storage is destroyed explicitly");
        let accounting_release = cleanup
            .find("release_evidence(self.state, outgoing_bytes)")
            .expect("snapshot evidence accounting is released explicitly");
        assert!(phase_storage < accounting_release);
        assert!(dimension_storage < accounting_release);

        let snapshot = production
            .split_once("fn try_snapshot<P, D>")
            .expect("snapshot constructor exists")
            .1
            .split_once("fn validate_final_snapshot_shape")
            .expect("snapshot constructor has a bounded body")
            .0;
        assert!(snapshot.contains("SnapshotEvidenceAssembly::new(state)"));
        assert!(!snapshot.contains("capacity_bytes("));
    }

    #[test]
    fn phase_snapshot_admission_failure_restores_exact_evidence_accounting() {
        let ledger = test_ledger(limits());
        let (evidence_before, peak_before, original_limit) = {
            let mut state = lock(&ledger.state);
            let dimension_plan = CapacityPlan::for_type::<(Dimension, DimensionSnapshot)>(
                0,
                0,
                state.dimensions.len(),
            )
            .expect("dimension snapshot capacity plan");
            let dimension_envelope = dimension_plan
                .envelope_delta_bytes()
                .expect("dimension snapshot envelope");
            let evidence_before = state.current_evidence_bytes;
            let peak_before = state.peak_evidence_bytes;
            let original_limit = state.evidence_byte_limit;
            state.evidence_byte_limit = evidence_before
                .checked_add(dimension_envelope)
                .expect("dimension snapshot fits the test limit");
            (evidence_before, peak_before, original_limit)
        };

        let error = ledger
            .try_snapshot()
            .expect_err("phase snapshot storage must exceed the tightened limit");
        assert!(matches!(
            error,
            LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::Snapshot,
                ..
            }
        ));

        {
            let mut state = lock(&ledger.state);
            assert_eq!(
                state.current_evidence_bytes, evidence_before,
                "failed snapshot storage must release its complete local accounting"
            );
            assert!(
                state.peak_evidence_bytes >= peak_before,
                "failed transient storage remains visible in peak evidence"
            );
            state.evidence_byte_limit = original_limit;
        }
        assert_eq!(
            ledger
                .try_snapshot()
                .expect("a later snapshot remains usable")
                .current_evidence_bytes(),
            evidence_before
        );
    }

    #[test]
    fn injected_allocator_failure_releases_current_evidence_but_preserves_peak() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let mut values = Vec::<u64>::new();
        let mut current = 0;
        let mut peak = 0;
        test_set_evidence_allocation_failure(true);
        let error = try_grow_evidence_vec(
            &mut values,
            1,
            &mut current,
            &mut peak,
            1024,
            LedgerEvidencePurpose::Snapshot,
        )
        .expect_err("injected allocation failure");
        assert!(matches!(
            error,
            LedgerEvidenceError::AllocationFailed {
                purpose: LedgerEvidencePurpose::Snapshot,
                requested_bytes: 8,
            }
        ));
        assert_eq!(values.capacity(), 0);
        assert_eq!(current, 0);
        assert_eq!(peak, 8);
    }

    #[test]
    fn injected_over_envelope_failure_preserves_non_empty_local_vec_and_accounting() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let mut values = Vec::with_capacity(2);
        values.extend([11_u64, 22]);
        let original_values = values.clone();
        let original_len = values.len();
        let original_capacity = values.capacity();
        let original_bytes = capacity_bytes(&values).expect("test capacity bytes");
        let mut current = original_bytes;
        let mut peak = original_bytes;

        test_fail_next_evidence_over_envelope();
        let error = try_grow_evidence_vec(
            &mut values,
            original_capacity.saturating_add(1),
            &mut current,
            &mut peak,
            u64::MAX,
            LedgerEvidencePurpose::Snapshot,
        )
        .expect_err("injected post-allocation envelope violation");

        assert!(matches!(
            error,
            LedgerEvidenceError::Capacity {
                purpose: LedgerEvidencePurpose::Snapshot,
                cause: CapacityPlanError::CapacityEnvelopeViolated { .. },
            }
        ));
        assert_eq!(values, original_values);
        assert_eq!(values.len(), original_len);
        assert_eq!(values.capacity(), original_capacity);
        assert_eq!(current, original_bytes);
        assert!(peak > original_bytes);
    }

    #[test]
    fn replacement_growth_accounts_the_complete_transient_candidate() {
        let mut values = Vec::with_capacity(2);
        values.extend([11_u64, 22]);
        let original_values = values.clone();
        let original_bytes = capacity_bytes(&values).expect("test capacity bytes");
        let desired_capacity = values.capacity().saturating_add(1);
        let replacement_plan = CapacityPlan::for_type::<u64>(0, 0, desired_capacity)
            .expect("replacement capacity plan");
        let replacement_envelope_bytes = replacement_plan
            .envelope_delta_bytes()
            .expect("replacement envelope bytes");
        let mut current = original_bytes;
        let mut peak = original_bytes;

        try_grow_evidence_vec(
            &mut values,
            desired_capacity,
            &mut current,
            &mut peak,
            u64::MAX,
            LedgerEvidencePurpose::Snapshot,
        )
        .expect("replacement growth");

        assert_eq!(values, original_values);
        assert!(values.capacity() >= desired_capacity);
        assert_eq!(
            current,
            capacity_bytes(&values).expect("replacement capacity bytes")
        );
        assert_eq!(
            peak,
            original_bytes + replacement_envelope_bytes,
            "the old backing and complete candidate envelope overlap at allocation time"
        );
    }

    #[test]
    fn tracked_coverage_mismatch_rejects_before_allocator_or_accounting_mutation() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let mut values = Vec::with_capacity(2);
        values.extend([11_u64, 22]);
        let original_values = values.clone();
        let original_capacity = values.capacity();
        let owned_capacity_bytes = capacity_bytes(&values).expect("test capacity bytes");
        let mut retained_bytes = 0;
        let mut current = owned_capacity_bytes;
        let mut peak = owned_capacity_bytes;

        test_set_evidence_allocation_failure(true);
        let error = try_grow_tracked_evidence_vec(
            &mut values,
            original_capacity.saturating_add(1),
            &mut retained_bytes,
            &mut current,
            &mut peak,
            u64::MAX,
            LedgerEvidencePurpose::Progress,
        )
        .expect_err("mismatched retained owner must reject");

        assert_eq!(
            error,
            LedgerEvidenceError::TrackedStorageCoverageMismatch {
                purpose: LedgerEvidencePurpose::Progress,
                retained_bytes: 0,
                capacity_bytes: owned_capacity_bytes,
            }
        );
        assert_eq!(values, original_values);
        assert_eq!(values.capacity(), original_capacity);
        assert_eq!(retained_bytes, 0);
        assert_eq!(current, owned_capacity_bytes);
        assert_eq!(peak, owned_capacity_bytes);
        assert!(
            test_evidence_allocation_failure_is_armed(),
            "coverage rejection must precede the allocator failpoint"
        );
        test_set_evidence_allocation_failure(false);
    }

    #[test]
    fn zero_sized_tracked_storage_needs_no_candidate_or_evidence_bytes() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let mut values = vec![(), ()];
        let original_capacity = values.capacity();
        let mut retained_bytes = 0;
        let mut current = 0;
        let mut peak = 0;

        test_set_evidence_allocation_failure(true);
        try_grow_tracked_evidence_vec(
            &mut values,
            usize::MAX,
            &mut retained_bytes,
            &mut current,
            &mut peak,
            0,
            LedgerEvidencePurpose::Progress,
        )
        .expect("zero-sized storage already has logical capacity");

        assert_eq!(values, [(), ()]);
        assert_eq!(values.capacity(), original_capacity);
        assert_eq!(retained_bytes, 0);
        assert_eq!(current, 0);
        assert_eq!(peak, 0);
        assert!(
            test_evidence_allocation_failure_is_armed(),
            "zero-sized growth must not create a replacement backing"
        );
        test_set_evidence_allocation_failure(false);
    }

    #[test]
    fn injected_progress_allocation_failure_is_atomic_and_latched() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let limits = limits();
        let mut coordinator =
            OperationCoordinator::<Owner, Phase, Progress, Unit, Dimension>::try_new(
                OperationControl::default(),
                limits,
                COMPLETE_PHASES,
            )
            .expect("complete test phase catalog");
        let frozen_evidence_bytes = coordinator
            .try_ledger_snapshot()
            .expect("complete frozen evidence baseline")
            .current_evidence_bytes();

        test_set_evidence_allocation_failure(true);
        let error = coordinator
            .report_progress(Progress::Work, Unit::Work, 1, None)
            .expect_err("injected progress allocation failure");
        assert!(matches!(
            error,
            ControlCause::ProgressStorage {
                cause: ProgressStorageError::Evidence {
                    cause: LedgerEvidenceError::AllocationFailed {
                        purpose: LedgerEvidencePurpose::Progress,
                        ..
                    }
                }
            }
        ));
        assert_eq!(coordinator.poll(), Err(error));

        let interim = coordinator
            .try_ledger_snapshot()
            .expect("allocation failure leaves capturable evidence");
        assert_eq!(
            interim.current_evidence_bytes(),
            frozen_evidence_bytes,
            "failed growth must roll back current evidence bytes"
        );
        assert!(
            interim.peak_evidence_bytes() > frozen_evidence_bytes,
            "the rejected allocator envelope remains visible in the high-water mark"
        );

        let final_snapshot = coordinator
            .finish()
            .expect("failed growth finalizes cleanly");
        assert_eq!(final_snapshot.current_evidence_bytes(), 0);
        assert!(final_snapshot.peak_evidence_bytes() > frozen_evidence_bytes);
    }

    #[test]
    fn injected_progress_over_envelope_failure_preserves_storage_and_releases_on_finish() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let limits = limits();
        let mut coordinator =
            OperationCoordinator::<Owner, Phase, Progress, Unit, Dimension>::try_new(
                OperationControl::default(),
                limits,
                COMPLETE_PHASES,
            )
            .expect("complete test phase catalog");
        coordinator
            .report_progress(Progress::Work, Unit::Work, 1, None)
            .expect("initial progress state");
        let frozen_evidence_bytes = coordinator
            .try_ledger_snapshot()
            .expect("progress evidence baseline")
            .current_evidence_bytes();

        test_fail_next_evidence_over_envelope();
        let error = coordinator
            .report_progress(Progress::Other, Unit::Work, 1, None)
            .expect_err("injected post-allocation envelope violation");
        assert!(matches!(
            &error,
            ControlCause::ProgressStorage {
                cause: ProgressStorageError::Evidence {
                    cause: LedgerEvidenceError::Capacity {
                        purpose: LedgerEvidencePurpose::Progress,
                        cause: CapacityPlanError::CapacityEnvelopeViolated { .. },
                    }
                }
            }
        ));
        assert_eq!(coordinator.poll(), Err(error));
        assert_eq!(
            coordinator.reported_progress(Progress::Work, Unit::Work),
            Some((1, None)),
            "failed replacement must leave the previous progress entry intact"
        );
        let interim = coordinator
            .try_ledger_snapshot()
            .expect("terminal coordinator retains capturable evidence");
        assert_eq!(
            interim.current_evidence_bytes(),
            frozen_evidence_bytes,
            "failed growth must restore current evidence exactly"
        );
        assert!(
            interim.peak_evidence_bytes() > frozen_evidence_bytes,
            "the transient candidate envelope remains visible in the high-water mark"
        );

        let final_snapshot = coordinator
            .finish()
            .expect("terminal progress storage must release on finish");
        assert_eq!(final_snapshot.current_evidence_bytes(), 0);
    }

    #[test]
    fn finalization_never_enters_the_evidence_allocator() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let ledger = test_ledger(limits());
        let reservation = ledger
            .reserve(Phase::Build, Dimension::Bytes, 1)
            .expect("live reservation");
        drop(reservation);
        test_set_evidence_allocation_failure(true);
        let evidence = ledger
            .dimension_evidence(Dimension::Bytes)
            .expect("scalar evidence is allocation-free");
        assert_eq!(evidence.current_live_bytes(), 0);
        assert!(test_evidence_allocation_failure_is_armed());
        let snapshot = ledger.finish().expect("clean finalization");
        assert_eq!(snapshot.current_evidence_bytes(), 0);
        assert!(test_evidence_allocation_failure_is_armed());
        test_set_evidence_allocation_failure(false);

        let ledger = test_ledger(limits());
        let reservation = ledger
            .reserve(Phase::Build, Dimension::Bytes, 1)
            .expect("retained reservation");
        test_set_evidence_allocation_failure(true);
        let error = ledger
            .finish()
            .expect_err("live state is an invariant failure");
        assert_eq!(error.snapshot().current_live_bytes(), 1);
        assert!(error.snapshot().current_evidence_bytes() > 0);
        assert!(test_evidence_allocation_failure_is_armed());
        test_set_evidence_allocation_failure(false);
        drop(reservation);
    }

    #[test]
    fn completed_phase_survives_clean_and_non_clean_final_snapshots() {
        let clean =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("clean frozen catalog");
        clean
            .try_complete_phase(Phase::Build)
            .expect("build completes");
        assert_eq!(
            clean
                .finish()
                .expect("clean ledger finishes")
                .last_completed_phase(),
            Some(Phase::Build)
        );

        let non_clean =
            Ledger::<Owner, Phase, Dimension>::try_new(limits(), &[Phase::Build, Phase::Finalize])
                .expect("non-clean frozen catalog");
        non_clean
            .try_complete_phase(Phase::Build)
            .expect("build completes");
        let reservation = non_clean
            .reserve(Phase::Finalize, Dimension::Bytes, 1)
            .expect("live finalize reservation");
        let error = non_clean
            .finish()
            .expect_err("live reservation makes finalization non-clean");
        assert_eq!(error.snapshot().last_completed_phase(), Some(Phase::Build));
        drop(reservation);
    }

    fn typed_limits() -> LedgerLimits<Dimension> {
        LedgerLimits::new(
            u64::MAX,
            [(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            )],
        )
        .expect("typed test policy")
    }

    fn probe_values<P, D>(probe: &LedgerTestProbe<P, D>) -> (u64, u64) {
        (probe.active_reservations(), probe.current_live_bytes())
    }

    #[test]
    fn typed_foreign_attribution_push_returns_the_token_without_mutating_either_ledger() {
        let target = test_ledger(typed_limits());
        let foreign = test_ledger(typed_limits());
        let target_probe = target.test_probe();
        let foreign_probe = foreign.test_probe();
        let mut target_batch = target
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 0)
            .expect("target batch");
        let foreign_token = foreign
            .reserve_operation_attribution::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 5)
            .expect("foreign attribution");
        let target_before = probe_values(&target_probe);
        let foreign_before = probe_values(&foreign_probe);
        assert_eq!(target_before.0, 1);
        assert_eq!(foreign_before.0, 1);
        assert!(target_before.1 > 0);
        assert!(foreign_before.1 >= 5);

        let recovered = match target_batch.try_push_attribution(foreign_token) {
            Ok(()) => panic!("foreign attribution must not enter the target batch"),
            Err(token) => token,
        };

        assert_eq!(target_batch.attribution_len(), 0);
        assert_eq!(probe_values(&target_probe), target_before);
        assert_eq!(probe_values(&foreign_probe), foreign_before);
        drop(recovered);
        drop(target_batch);
        assert_eq!(probe_values(&target_probe), (0, 0));
        assert_eq!(probe_values(&foreign_probe), (0, 0));
    }

    #[test]
    fn typed_commitment_resolves_its_dimension_before_entering_the_batch() {
        let ledger = test_ledger(typed_limits());
        let probe = ledger.test_probe();
        let mut batch = ledger
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 1)
            .expect("commitment batch");
        let before = probe_values(&probe);

        let recovered = match ledger
            .try_push_commitment(&mut batch, OutputCommitment::new(Dimension::Records, 5))
        {
            Err(CommitmentPushError::Ledger {
                commitment,
                cause:
                    LedgerCause::UnknownDimension {
                        dimension: Dimension::Records,
                    },
            }) => commitment,
            Ok(()) => panic!("an unknown commitment dimension must not enter the batch"),
            Err(_) => panic!("unknown commitment dimension returned the wrong rejection"),
        };

        assert_eq!(recovered.dimension(), Dimension::Records);
        assert_eq!(recovered.amount(), 5);
        assert_eq!(recovered.dimension_position, usize::MAX);
        assert!(batch.commitments.is_empty());
        assert_eq!(probe_values(&probe), before);
        drop(batch);
        assert_eq!(probe_values(&probe), (0, 0));
    }

    #[test]
    fn empty_typed_batch_still_rejects_a_foreign_ledger_owner() {
        let target = test_ledger(typed_limits());
        let foreign = test_ledger(typed_limits());
        let target_probe = target.test_probe();
        let foreign_probe = foreign.test_probe();
        let mut target_batch = target
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 1)
            .expect("target commitment batch");
        let target_before = probe_values(&target_probe);
        let foreign_before = probe_values(&foreign_probe);

        let recovered = match foreign.try_push_commitment(
            &mut target_batch,
            OutputCommitment::new(Dimension::Bytes, 3),
        ) {
            Err(CommitmentPushError::Ledger {
                commitment,
                cause: LedgerCause::OwnerMismatch,
            }) => commitment,
            Ok(()) => panic!("an empty foreign batch must not bypass owner validation"),
            Err(_) => panic!("foreign empty batch returned the wrong rejection"),
        };

        assert_eq!(recovered.dimension(), Dimension::Bytes);
        assert_eq!(recovered.amount(), 3);
        assert!(target_batch.commitments.is_empty());
        assert_eq!(probe_values(&target_probe), target_before);
        assert_eq!(probe_values(&foreign_probe), foreign_before);
        drop(target_batch);
        assert_eq!(probe_values(&target_probe), (0, 0));
        assert_eq!(probe_values(&foreign_probe), (0, 0));
    }

    #[test]
    fn typed_foreign_fallback_batch_rejects_the_baseline_without_mutation() {
        let target = test_ledger(typed_limits());
        let foreign = test_ledger(typed_limits());
        let target_probe = target.test_probe();
        let foreign_probe = foreign.test_probe();
        let canonical = target
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 0)
            .expect("target canonical assembly");
        let evidence_assembly = target
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("target evidence assembly");
        let fallback = foreign
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                1,
                0,
            )
            .expect("foreign fallback batch");
        let evidence = target
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("target evidence batch");
        let target_before = probe_values(&target_probe);
        let foreign_before = probe_values(&foreign_probe);
        assert_eq!(target_before.0, 1);
        assert_eq!(foreign_before.0, 1);

        let rejection =
            match target.try_fallback_baseline(canonical, evidence_assembly, fallback, evidence) {
                Ok(_) => panic!("foreign fallback batch must reject the baseline"),
                Err(rejection) => rejection,
            };

        assert_eq!(probe_values(&target_probe), target_before);
        assert_eq!(probe_values(&foreign_probe), foreign_before);
        let (baseline, cause) = rejection.into_parts();
        assert!(matches!(cause, LedgerCause::OwnerMismatch));
        let (canonical, evidence_assembly, fallback, evidence) = baseline.into_parts();
        drop(canonical);
        drop(evidence_assembly);
        drop(fallback);
        drop(evidence);
        assert_eq!(probe_values(&target_probe), (0, 0));
        assert_eq!(probe_values(&foreign_probe), (0, 0));
    }

    #[test]
    fn typed_terminal_qualification_rejects_a_foreign_discard_batch() {
        let target = test_ledger(typed_limits());
        let foreign = test_ledger(typed_limits());
        let target_probe = target.test_probe();
        let foreign_probe = foreign.test_probe();
        let canonical = target
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 0)
            .expect("target canonical assembly");
        let evidence_assembly = target
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("target evidence assembly");
        let fallback = target
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("target fallback batch");
        let evidence = target
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("target evidence batch");
        let baseline = target
            .try_fallback_baseline(canonical, evidence_assembly, fallback, evidence)
            .unwrap_or_else(|_| panic!("target baseline must be admitted"));
        let mut foreign_source = foreign
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 0)
            .expect("foreign source batch");
        let foreign_token = foreign
            .reserve_operation_attribution::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 5)
            .expect("foreign source attribution");
        assert!(foreign_source.try_push_attribution(foreign_token).is_ok());
        let target_before = probe_values(&target_probe);
        let foreign_before = probe_values(&foreign_probe);
        assert_eq!(target_before.0, 1);
        assert_eq!(foreign_before.0, 2);

        let mut baseline = baseline;
        let mut foreign_source = Some(foreign_source);
        let rejection = target
            .try_bind_primary(&mut baseline, foreign_source.as_mut(), None, true)
            .expect_err("foreign primary source must be rejected");

        assert!(matches!(rejection, LedgerCause::OwnerMismatch));
        assert_eq!(probe_values(&target_probe), target_before);
        assert_eq!(probe_values(&foreign_probe), foreign_before);
        let proof = baseline.into_fallback_proof(target, foreign_source, None);
        let proof = match proof.try_into_qualified_discards() {
            Ok(_) => panic!("a foreign discard batch must not qualify for terminal mutation"),
            Err(proof) => proof,
        };
        drop(proof);
        assert_eq!(probe_values(&target_probe), (0, 0));
        assert_eq!(probe_values(&foreign_probe), (0, 0));
    }

    static PANIC_ON_BOUND_ORD: AtomicBool = AtomicBool::new(false);
    static REVERSE_BOUND_ORD: AtomicBool = AtomicBool::new(false);
    static BOUND_ORD_COMPARISONS_REMAINING: AtomicUsize = AtomicUsize::new(usize::MAX);
    static BOUND_ORD_COMPARISON_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BOUND_ORD_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug, Clone, Copy)]
    struct StatefulOrd(u8);

    impl PartialEq for StatefulOrd {
        fn eq(&self, other: &Self) -> bool {
            self.cmp(other).is_eq()
        }
    }

    impl Eq for StatefulOrd {}

    impl PartialOrd for StatefulOrd {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for StatefulOrd {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            BOUND_ORD_COMPARISON_COUNT.fetch_add(1, Ordering::SeqCst);
            assert!(
                !PANIC_ON_BOUND_ORD.load(Ordering::SeqCst),
                "typed freeze called owner-defined Ord after positions were bound"
            );
            let remaining = BOUND_ORD_COMPARISONS_REMAINING.load(Ordering::SeqCst);
            if remaining != usize::MAX {
                assert_ne!(
                    remaining, 0,
                    "operation called owner-defined Ord after its lookup preflight"
                );
                BOUND_ORD_COMPARISONS_REMAINING.fetch_sub(1, Ordering::SeqCst);
            }
            let ordering = self.0.cmp(&other.0);
            if REVERSE_BOUND_ORD.load(Ordering::SeqCst) {
                ordering.reverse()
            } else {
                ordering
            }
        }
    }

    fn stateful_ord_limits() -> LedgerLimits<StatefulOrd> {
        LedgerLimits::new(
            u64::MAX,
            [(
                StatefulOrd(7),
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            )],
        )
        .expect("stateful Ord test policy")
    }

    fn stateful_multi_dimension_limits() -> LedgerLimits<StatefulOrd> {
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        LedgerLimits::new(
            u64::MAX,
            [(StatefulOrd(7), policy), (StatefulOrd(9), policy)],
        )
        .expect("stateful multi-dimension test policy")
    }

    fn stateful_ledger(
        limits: LedgerLimits<StatefulOrd>,
        phases: &[StatefulOrd],
    ) -> Ledger<Owner, StatefulOrd, StatefulOrd> {
        Ledger::try_new(limits, phases).expect("complete stateful phase catalog")
    }

    fn freeze_while_ord_panics(
        proof: TypedFreezeProof<Owner, StatefulOrd, StatefulOrd>,
    ) -> LedgerSnapshot<StatefulOrd, StatefulOrd> {
        PANIC_ON_BOUND_ORD.store(true, Ordering::SeqCst);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match proof.try_into_qualified_discards() {
                Ok(QualifiedTypedDiscardFreezeProof::None(proof)) => proof.freeze(),
                Ok(proof) => {
                    proof.retain_fail_closed();
                    panic!("test proof unexpectedly requires physical discard acknowledgement")
                }
                Err(_) => panic!("bound typed proof must qualify"),
            }
        }));
        PANIC_ON_BOUND_ORD.store(false, Ordering::SeqCst);
        result.expect("bound typed freeze must not call owner-defined Ord")
    }

    #[test]
    fn consumed_batch_never_compares_owner_keys_after_its_lookup_preflight() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let first = StatefulOrd(7);
        let second = StatefulOrd(9);
        let ledger = stateful_ledger(stateful_multi_dimension_limits(), &[StatefulOrd(2)]);
        ledger
            .charge_attempted(first, 1)
            .expect("first attempted work");
        ledger
            .charge_attempted(second, 1)
            .expect("second attempted work");

        let lookup_comparisons = {
            let state = lock(&ledger.state);
            BOUND_ORD_COMPARISON_COUNT.store(0, Ordering::SeqCst);
            for dimension in [first, second] {
                state
                    .dimensions
                    .binary_search_by_key(&dimension, |entry| entry.dimension)
                    .expect("predeclared dimension");
            }
            BOUND_ORD_COMPARISON_COUNT.load(Ordering::SeqCst)
        };
        BOUND_ORD_COMPARISONS_REMAINING.store(lookup_comparisons, Ordering::SeqCst);
        let charged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ledger.charge_consumed_batch(&[(first, 1), (second, 1)])
        }));
        let remaining = BOUND_ORD_COMPARISONS_REMAINING.swap(usize::MAX, Ordering::SeqCst);

        assert_eq!(remaining, 0, "every admitted lookup must consume its quota");
        charged
            .expect("batch compared owner keys after resolving numeric positions")
            .expect("predeclared completed work must fit");
        assert_eq!(lock(&ledger.state).dimensions[0].state.consumed, 1);
        assert_eq!(lock(&ledger.state).dimensions[1].state.consumed, 1);
    }

    #[test]
    fn reservation_transition_never_compares_owner_keys_after_its_lookup_preflight() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let phase = StatefulOrd(2);
        let dimension = StatefulOrd(7);
        let ledger = stateful_ledger(stateful_ord_limits(), &[phase]);
        let probe = ledger.test_probe();
        let first = ledger
            .reserve(phase, dimension, 2)
            .expect("first transition source");
        let second = ledger
            .reserve(phase, dimension, 3)
            .expect("second transition source");
        let mut reservations = [first, second];

        BOUND_ORD_COMPARISONS_REMAINING.store(2, Ordering::SeqCst);
        let transition = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ledger.transition_reservations(&mut reservations, phase, dimension, 4)
        }));
        let remaining = BOUND_ORD_COMPARISONS_REMAINING.swap(usize::MAX, Ordering::SeqCst);

        assert_eq!(
            remaining, 0,
            "dimension and phase lookup must each compare once"
        );
        transition
            .expect("transition compared owner keys after releasing source bytes")
            .expect("same-dimension transition must succeed");
        assert_eq!(reservations[0].bytes(), 2);
        assert_eq!(reservations[1].bytes(), 2);
        assert_eq!(probe_values(&probe), (2, 4));
        drop(reservations);
        assert_eq!(probe_values(&probe), (0, 0));
    }

    #[test]
    fn reservation_construction_binds_keys_once_and_resize_uses_only_bound_positions() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let phase = StatefulOrd(2);
        let dimension = StatefulOrd(7);
        let ledger = stateful_ledger(stateful_ord_limits(), &[phase]);

        BOUND_ORD_COMPARISONS_REMAINING.store(2, Ordering::SeqCst);
        let reservation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ledger.reserve(phase, dimension, 3)
        }));
        let remaining = BOUND_ORD_COMPARISONS_REMAINING.swap(usize::MAX, Ordering::SeqCst);
        assert_eq!(
            remaining, 0,
            "reservation admission resolves dimension and phase exactly once"
        );
        let mut reservation = reservation
            .expect("reservation admission repeated an owner-key lookup")
            .expect("predeclared reservation must fit");

        PANIC_ON_BOUND_ORD.store(true, Ordering::SeqCst);
        let resize = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ledger.resize_reservation(&mut reservation, 5)
        }));
        PANIC_ON_BOUND_ORD.store(false, Ordering::SeqCst);
        resize
            .expect("resize compared keys already bound into the reservation")
            .expect("bound reservation resize must fit");
        assert_eq!(reservation.bytes(), 5);
        assert_eq!(ledger.test_probe().current_live_bytes(), 5);
        drop(reservation);
        assert_eq!(ledger.test_probe().current_live_bytes(), 0);
    }

    #[test]
    fn reservation_transfer_uses_bound_positions_after_its_lookup_preflight() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let source_phase = StatefulOrd(1);
        let target_phase = StatefulOrd(2);
        let dimension = StatefulOrd(7);
        let ledger = stateful_ledger(stateful_ord_limits(), &[source_phase, target_phase]);
        let mut reservation = ledger
            .reserve(source_phase, dimension, 5)
            .expect("transfer source");

        BOUND_ORD_COMPARISONS_REMAINING.store(3, Ordering::SeqCst);
        let transfer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ledger.transfer(&mut reservation, target_phase)
        }));
        let remaining = BOUND_ORD_COMPARISONS_REMAINING.swap(usize::MAX, Ordering::SeqCst);

        assert_eq!(
            remaining, 0,
            "source equality and target phase lookup must finish before mutation"
        );
        transfer
            .expect("transfer compared owner keys after its target lookup")
            .expect("predeclared target phase must accept the transfer");
        assert_eq!(
            ledger
                .phase_evidence(source_phase)
                .expect("source phase evidence")
                .current_live_bytes(),
            0
        );
        assert_eq!(
            ledger
                .phase_evidence(target_phase)
                .expect("target phase evidence")
                .current_live_bytes(),
            5
        );

        REVERSE_BOUND_ORD.store(true, Ordering::SeqCst);
        let release = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(reservation);
        }));
        REVERSE_BOUND_ORD.store(false, Ordering::SeqCst);
        release.expect("bound reservation release must not compare owner keys");
        assert_eq!(ledger.test_probe().current_live_bytes(), 0);
    }

    #[test]
    fn commitment_aggregate_panic_releases_current_evidence_before_unwind() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let phase = StatefulOrd(2);
        let dimension = StatefulOrd(7);
        let ledger = stateful_ledger(stateful_ord_limits(), &[phase]);
        let before = lock(&ledger.state).current_evidence_bytes;
        let mut reservations = PublicationReservationBatch::default();

        PANIC_ON_BOUND_ORD.store(true, Ordering::SeqCst);
        let aggregation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ledger.prepare_publication(&mut reservations, &[Commitment::new(dimension, 3)])
        }));
        PANIC_ON_BOUND_ORD.store(false, Ordering::SeqCst);

        assert!(
            aggregation.is_err(),
            "owner comparator must reach the test panic"
        );
        let state = lock(&ledger.state);
        assert_eq!(state.current_evidence_bytes, before);
        assert_eq!(state.publication, PublicationState::Open);
        assert_eq!(state.dimensions[0].state.pending_publication, 0);
    }

    #[test]
    fn pending_publication_publish_and_drop_use_positions_after_ord_reverses() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let phase = StatefulOrd(2);
        let dimension = StatefulOrd(9);
        let ledger = stateful_ledger(stateful_multi_dimension_limits(), &[phase]);
        let mut reservations = PublicationReservationBatch::default();
        let mut pending = ledger
            .prepare_publication(&mut reservations, &[Commitment::new(dimension, 3)])
            .expect("pending publication");

        REVERSE_BOUND_ORD.store(true, Ordering::SeqCst);
        let published = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ledger.publish(&mut pending)
        }));
        REVERSE_BOUND_ORD.store(false, Ordering::SeqCst);
        published
            .expect("publish compared a dimension after admission")
            .expect("bound publication must succeed");
        assert_eq!(lock(&ledger.state).dimensions[1].state.committed, 3);

        let rollback_ledger = stateful_ledger(stateful_multi_dimension_limits(), &[phase]);
        let baseline_evidence = lock(&rollback_ledger.state).current_evidence_bytes;
        let mut rollback_reservations = PublicationReservationBatch::default();
        let rollback = rollback_ledger
            .prepare_publication(&mut rollback_reservations, &[Commitment::new(dimension, 4)])
            .expect("rollback publication");
        REVERSE_BOUND_ORD.store(true, Ordering::SeqCst);
        let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(rollback)));
        REVERSE_BOUND_ORD.store(false, Ordering::SeqCst);
        dropped.expect("rollback compared a dimension after admission");
        let state = lock(&rollback_ledger.state);
        assert_eq!(state.dimensions[1].state.pending_publication, 0);
        assert_eq!(state.current_evidence_bytes, baseline_evidence);
        assert_eq!(state.publication, PublicationState::Open);
    }

    #[test]
    fn fallback_baseline_counts_zero_byte_attribution_capabilities() {
        let ledger = test_ledger(typed_limits());
        let extra = ledger
            .reserve_operation_attribution::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
            )
            .expect("zero-byte attribution");
        let canonical = ledger
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 0)
            .expect("empty canonical assembly");
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("empty evidence assembly");
        let fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("empty fallback batch");
        let snapshot = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("empty snapshot batch");

        let rejection = match ledger.try_fallback_baseline(canonical, evidence, fallback, snapshot)
        {
            Ok(_) => panic!("unowned zero-byte capability must reject the baseline"),
            Err(rejection) => rejection,
        };
        assert!(matches!(
            rejection.into_parts().1,
            LedgerCause::ActiveReservationsRemain { count: 1 }
        ));
        drop(extra);
    }

    #[test]
    fn included_zero_byte_attribution_freezes_to_zero_active_capabilities() {
        let ledger = Ledger::<Owner, Phase, Dimension>::try_new(typed_limits(), &[Phase::Build])
            .expect("typed frozen catalog");
        let racing_ledger = Ledger::<Owner, Phase, Dimension> {
            state: Arc::clone(&ledger.state),
            operation_live_byte_limit: ledger.operation_live_byte_limit,
            operation_owner: PhantomData,
        };
        ledger
            .try_complete_phase(Phase::Build)
            .expect("build completes before typed freeze");
        let canonical = ledger
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 0)
            .expect("empty canonical assembly");
        let evidence_assembly = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                1,
                0,
            )
            .expect("evidence assembly");
        let mut fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                1,
                0,
            )
            .expect("fallback batch");
        let zero = ledger
            .reserve_operation_attribution::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
            )
            .expect("zero-byte attribution");
        assert!(fallback.try_push_attribution(zero).is_ok());
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("snapshot batch");
        let baseline =
            match ledger.try_fallback_baseline(canonical, evidence_assembly, fallback, evidence) {
                Ok(baseline) => baseline,
                Err(_) => panic!("exact zero-byte baseline must be admitted"),
            };
        let qualified = match baseline
            .into_fallback_proof(ledger, None, None)
            .try_into_qualified_discards()
        {
            Ok(QualifiedTypedDiscardFreezeProof::None(proof)) => proof,
            Ok(proof) => {
                proof.retain_fail_closed();
                panic!("zero-discard proof changed terminal variant")
            }
            Err(_) => panic!("exact typed proof must qualify"),
        };
        assert_eq!(
            lock(&racing_ledger.state).publication,
            PublicationState::TerminalQualified
        );
        let racing_mutation = std::thread::spawn(move || {
            racing_ledger.tighten_dimension_policy(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            )
        })
        .join()
        .expect("racing policy mutation must not panic");
        assert_eq!(
            racing_mutation,
            Err(LedgerCause::PublicationAlreadyCompleted)
        );
        let state = Arc::clone(
            &qualified
                .proof
                .as_ref()
                .expect("qualified proof")
                .proof
                .as_ref()
                .expect("typed proof")
                .ledger
                .state,
        );
        let snapshot = qualified.freeze();
        assert_eq!(lock(&state).publication, PublicationState::Published);
        assert_eq!(snapshot.current_live_bytes(), 0);
        assert_eq!(snapshot.active_reservations(), 0);
        assert_eq!(snapshot.last_completed_phase(), Some(Phase::Build));
    }

    #[test]
    fn terminal_qualification_rejects_an_out_of_bounds_zero_byte_attribution() {
        let ledger = Ledger::<Owner, Phase, Dimension>::try_new(typed_limits(), &[Phase::Build])
            .expect("typed frozen catalog");
        let probe = ledger.test_probe();
        let canonical = ledger
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 0)
            .expect("empty canonical assembly");
        let evidence_assembly = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                1,
                0,
            )
            .expect("evidence assembly");
        let mut fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                1,
                0,
            )
            .expect("fallback batch");
        let zero = ledger
            .reserve_operation_attribution::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
            )
            .expect("zero-byte attribution");
        assert!(fallback.try_push_attribution(zero).is_ok());
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("snapshot batch");
        let baseline = ledger
            .try_fallback_baseline(canonical, evidence_assembly, fallback, evidence)
            .unwrap_or_else(|_| panic!("exact zero-byte baseline must bind"));
        let mut proof = baseline.into_fallback_proof(ledger, None, None);
        assert_eq!(proof.evidence.attributions[0].core.bytes, 0);
        proof.evidence.attributions[0].core.dimension_position = usize::MAX;
        proof.evidence.attributions[0].core.phase_position = usize::MAX;

        let proof = match proof.try_into_qualified_discards() {
            Ok(_) => panic!("out-of-bounds zero-byte attribution must not qualify"),
            Err(proof) => proof,
        };
        drop(proof);
        assert_eq!(probe_values(&probe), (0, 0));
    }

    #[test]
    fn terminal_qualification_rechecks_bound_commitments_after_policy_tightening() {
        let ledger = test_ledger(typed_limits());
        let probe = ledger.test_probe();
        let canonical_assembly = ledger
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 1)
            .expect("canonical assembly");
        let evidence_assembly = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("evidence assembly");
        let fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("fallback batch");
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("evidence batch");
        let baseline = ledger
            .try_fallback_baseline(canonical_assembly, evidence_assembly, fallback, evidence)
            .unwrap_or_else(|_| panic!("exact primary baseline must bind"));
        let mut source = ledger
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 1, 1)
            .expect("canonical source");
        let attribution = ledger
            .reserve_operation_attribution::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0)
            .expect("zero-byte canonical attribution");
        assert!(source.try_push_attribution(attribution).is_ok());
        assert!(
            ledger
                .try_push_commitment(&mut source, OutputCommitment::new(Dimension::Bytes, 5),)
                .is_ok()
        );
        let mut baseline = baseline;
        let mut source = Some(source);
        ledger
            .try_bind_primary(&mut baseline, source.as_mut(), None, true)
            .unwrap_or_else(|_| panic!("commitment must bind under the original policy"));
        ledger
            .tighten_dimension_policy(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 4, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            )
            .expect("policy may tighten before terminal qualification");
        let proof = ledger.finish_bound_primary(baseline, source, None, true);

        let proof = match proof.try_into_qualified_discards() {
            Ok(_) => panic!("commitment above the final policy must not qualify"),
            Err(proof) => proof,
        };
        drop(proof);
        assert_eq!(probe_values(&probe), (0, 0));
    }

    #[test]
    fn typed_final_preflight_rejects_an_invalid_completed_phase_pair_without_mutation() {
        let ledger = Ledger::<Owner, Phase, Dimension>::try_new(typed_limits(), &[Phase::Build])
            .expect("typed frozen catalog");
        ledger
            .try_complete_phase(Phase::Build)
            .expect("build completes before fault injection");
        let canonical = ledger
            .try_attribution_batch::<CanonicalOutput>(Phase::Build, Dimension::Bytes, 0, 0)
            .expect("empty canonical assembly");
        let evidence_assembly = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                1,
                0,
            )
            .expect("evidence assembly");
        let fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                1,
                0,
            )
            .expect("fallback batch");
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Build,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("evidence batch");
        let probe = ledger.test_probe();
        let before = probe_values(&probe);
        {
            let mut state = lock(&ledger.state);
            state
                .last_completed_phase
                .as_mut()
                .expect("completed phase")
                .state_position = state.phase_states.len();
        }

        let rejection =
            match ledger.try_fallback_baseline(canonical, evidence_assembly, fallback, evidence) {
                Ok(_) => panic!("invalid completed pair cannot produce a typed freeze proof"),
                Err(rejection) => rejection,
            };

        assert_eq!(probe_values(&probe), before);
        let (baseline, cause) = rejection.into_parts();
        assert_eq!(cause, LedgerCause::FinalSnapshotShapeMismatch);
        let (canonical, evidence_assembly, fallback, evidence) = baseline.into_parts();
        drop(canonical);
        drop(evidence_assembly);
        drop(fallback);
        drop(evidence);
        assert_eq!(probe_values(&probe), (0, 0));
    }

    #[test]
    fn fallback_freeze_uses_stable_positions_for_attribution_and_capacity_cover() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let terminal_phase = StatefulOrd(2);
        let dimension = StatefulOrd(7);
        let ledger = stateful_ledger(stateful_ord_limits(), &[terminal_phase, StatefulOrd(1)]);
        let canonical = ledger
            .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 0, 0)
            .expect("empty canonical assembly");
        let evidence_assembly = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 1, 0)
            .expect("covered evidence assembly");
        let mut fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 1, 0)
            .expect("covered fallback batch");
        let attribution = ledger
            .reserve_operation_attribution::<OutcomeEvidenceDestination>(
                terminal_phase,
                dimension,
                3,
            )
            .expect("fallback attribution");
        assert!(fallback.try_push_attribution(attribution).is_ok());
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
            .expect("empty evidence batch");
        let baseline = ledger
            .try_fallback_baseline(canonical, evidence_assembly, fallback, evidence)
            .unwrap_or_else(|_| panic!("exact fallback baseline must bind"));

        let earlier = ledger
            .reserve(StatefulOrd(1), dimension, 1)
            .expect("predeclared earlier phase");
        drop(earlier);
        let proof = baseline.into_fallback_proof(ledger, None, None);
        let snapshot = freeze_while_ord_panics(proof);

        assert_eq!(snapshot.current_live_bytes(), 0);
        assert_eq!(snapshot.active_reservations(), 0);
    }

    #[test]
    fn primary_freeze_uses_bound_source_attribution_cover_and_commitment_positions() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let terminal_phase = StatefulOrd(2);
        let dimension = StatefulOrd(7);
        let ledger = stateful_ledger(stateful_ord_limits(), &[terminal_phase, StatefulOrd(1)]);
        let canonical_assembly = ledger
            .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
            .expect("covered canonical assembly");
        let evidence_assembly = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
            .expect("empty evidence assembly");
        let fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
            .expect("empty fallback batch");
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
            .expect("empty evidence batch");
        let baseline = ledger
            .try_fallback_baseline(canonical_assembly, evidence_assembly, fallback, evidence)
            .unwrap_or_else(|_| panic!("exact primary baseline must bind"));

        let earlier = ledger
            .reserve(StatefulOrd(1), dimension, 1)
            .expect("predeclared earlier phase");
        drop(earlier);
        let mut source = ledger
            .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
            .expect("covered canonical source");
        let attribution = ledger
            .reserve_operation_attribution::<CanonicalOutput>(terminal_phase, dimension, 5)
            .expect("canonical attribution");
        assert!(source.try_push_attribution(attribution).is_ok());
        assert!(
            ledger
                .try_push_commitment(&mut source, OutputCommitment::new(dimension, 3))
                .is_ok()
        );
        let mut baseline = baseline;
        let mut source = Some(source);
        ledger
            .try_bind_primary(&mut baseline, source.as_mut(), None, true)
            .unwrap_or_else(|_| panic!("exact primary publication must bind"));
        let proof = ledger.finish_bound_primary(baseline, source, None, true);
        let snapshot = freeze_while_ord_panics(proof);

        assert_eq!(snapshot.current_live_bytes(), 0);
        assert_eq!(snapshot.active_reservations(), 0);
        assert_eq!(
            snapshot
                .dimensions()
                .next()
                .expect("one dimension")
                .1
                .committed_canonical_output(),
            3
        );
    }

    #[test]
    fn terminal_freeze_keeps_commitments_on_bound_positions_after_ord_reverses() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let earlier_phase = StatefulOrd(1);
        let terminal_phase = StatefulOrd(2);
        let dimension = StatefulOrd(9);
        let ledger = stateful_ledger(
            stateful_multi_dimension_limits(),
            &[earlier_phase, terminal_phase],
        );
        let earlier = ledger
            .reserve(earlier_phase, StatefulOrd(7), 1)
            .expect("predeclared earlier phase");
        drop(earlier);
        let canonical_assembly = ledger
            .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
            .expect("covered canonical assembly");
        let evidence_assembly = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
            .expect("empty evidence assembly");
        let fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
            .expect("empty fallback batch");
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(terminal_phase, dimension, 0, 0)
            .expect("empty evidence batch");
        let baseline = ledger
            .try_fallback_baseline(canonical_assembly, evidence_assembly, fallback, evidence)
            .unwrap_or_else(|_| panic!("exact primary baseline must bind"));
        let mut source = ledger
            .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
            .expect("covered canonical source");
        let attribution = ledger
            .reserve_operation_attribution::<CanonicalOutput>(terminal_phase, dimension, 5)
            .expect("canonical attribution");
        assert!(source.try_push_attribution(attribution).is_ok());
        assert!(
            ledger
                .try_push_commitment(&mut source, OutputCommitment::new(dimension, 3))
                .is_ok()
        );
        let mut baseline = baseline;
        let mut source = Some(source);
        ledger
            .try_bind_primary(&mut baseline, source.as_mut(), None, true)
            .unwrap_or_else(|_| panic!("exact primary publication must bind"));
        let proof = ledger.finish_bound_primary(baseline, source, None, true);

        REVERSE_BOUND_ORD.store(true, Ordering::SeqCst);
        let frozen = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match proof.try_into_qualified_discards() {
                Ok(QualifiedTypedDiscardFreezeProof::None(proof)) => proof.freeze(),
                Ok(proof) => {
                    proof.retain_fail_closed();
                    panic!("primary publication unexpectedly requires a discard")
                }
                Err(_) => panic!("bound typed proof must qualify after Ord reverses"),
            }
        }));
        REVERSE_BOUND_ORD.store(false, Ordering::SeqCst);
        let snapshot = frozen.expect("terminal freeze must use bound numeric positions");

        let mut dimensions = snapshot.dimensions();
        let (first_key, first) = dimensions.next().expect("first dimension");
        let (second_key, second) = dimensions.next().expect("second dimension");
        assert_eq!(first_key, StatefulOrd(7));
        assert_eq!(first.committed_canonical_output(), 0);
        assert_eq!(second_key, StatefulOrd(9));
        assert_eq!(second.committed_canonical_output(), 3);
        assert!(dimensions.next().is_none());
        assert_eq!(snapshot.current_live_bytes(), 0);
        assert_eq!(snapshot.active_reservations(), 0);
    }

    #[test]
    fn typed_terminal_binding_uses_prestored_numeric_positions_without_a_mutation_tail() {
        let production = ledger_production_source();
        let primary = production
            .split("pub(crate) fn try_bind_primary(")
            .nth(1)
            .expect("typed primary binding exists")
            .split("pub(crate) fn try_bind_fallback_discards(")
            .next()
            .expect("typed primary binding boundary exists");
        assert_eq!(
            primary
                .matches("preflight_typed_publication_batch(")
                .count(),
            5
        );
        assert_eq!(primary.matches("lock(&self.state)").count(), 1);
        assert!(primary.contains("validate_primary_publication_locked("));
        assert!(!primary.contains("bind_preflighted"));

        let fallback = production
            .split("pub(crate) fn try_bind_fallback_baseline(")
            .nth(1)
            .expect("fallback binding exists")
            .split("fn validate_fallback_baseline_locked(")
            .next()
            .expect("fallback binding boundary exists");
        assert_eq!(
            fallback
                .matches("preflight_typed_publication_batch(")
                .count(),
            4
        );
        assert_eq!(fallback.matches("lock(&self.state)").count(), 1);
        assert!(fallback.contains("validate_fallback_baseline_locked("));
        assert!(!fallback.contains("bind_preflighted"));

        let preflight = production
            .split("fn preflight_typed_publication_batch")
            .nth(1)
            .expect("numeric preflight exists")
            .split("fn preflight_typed_discard_batch")
            .next()
            .expect("numeric preflight boundary exists");
        assert!(preflight.contains("bound_positions_are_in_bounds"));
        for forbidden in [
            "binary_search",
            "publication_positions",
            "bind_publication_positions",
            "&mut AttributionReservationBatch",
        ] {
            assert!(
                !preflight.contains(forbidden),
                "terminal preflight regained a semantic lookup or mutation through `{forbidden}`"
            );
        }

        let push = production
            .split("pub(crate) fn try_push_commitment")
            .nth(1)
            .expect("ledger commitment insertion exists")
            .split("/// Transfers live ownership")
            .next()
            .expect("ledger commitment insertion boundary exists");
        let search = push
            .find("binary_search_by_key")
            .expect("commitment dimension is resolved at insertion");
        let store = push
            .find("commitment.dimension_position = dimension_position")
            .expect("resolved numeric position is stored in the commitment");
        let insertion = push
            .find("push_positioned_commitment(commitment)")
            .expect("positioned commitment is inserted without allocation");
        assert!(search < store && store < insertion);
        for forbidden in [
            "?",
            "binary_search",
            "dimension_entry(",
            "dimension_entry_mut(",
            "try_reserve",
            ".reserve(",
            ".insert(",
            "expect(",
            "unwrap",
            "panic!",
        ] {
            assert!(
                !push[store..].contains(forbidden),
                "commitment insertion regained a lookup, allocation, or fault gate after binding its position through `{forbidden}`"
            );
        }

        let batch = production
            .split("pub(crate) struct AttributionReservationBatch")
            .nth(1)
            .expect("typed publication batch exists")
            .split("impl<O, P, D, Destination> AttributionReservationBatch")
            .next()
            .expect("typed publication batch boundary exists");
        assert!(batch.contains("owner: Arc<Mutex<LedgerState<P, D>>>"));
        for removed in [
            "fn bind_preflighted_typed_publication_batch",
            "fn bind_preflighted_typed_discard_batch",
            "fn publication_positions",
            "fn bind_publication_positions",
        ] {
            assert!(
                !production.contains(removed),
                "removed two-pass binding seam returned through `{removed}`"
            );
        }
    }

    #[test]
    fn consumed_batch_binds_every_dimension_before_its_numeric_mutation_tail() {
        let production = ledger_production_source();
        let batch = production
            .split("pub fn charge_consumed_batch<const N: usize>(")
            .nth(1)
            .expect("fixed-array consumed batch exists")
            .split("/// Reserves byte capacity")
            .next()
            .expect("consumed batch boundary exists");
        assert!(batch.contains("charges: &[(D, u64); N]"));
        assert!(batch.contains("let mut positions = [0_usize; N]"));
        assert!(batch.contains("let mut projected = [0_u64; N]"));

        let first_mutation = batch
            .find("state.dimensions[position].state.consumed = projected[index]")
            .expect("consumed batch has one numeric mutation tail");
        let preflight = &batch[..first_mutation];
        assert!(preflight.contains("binary_search_by_key"));
        assert!(preflight.contains("position < positions[index - 1]"));
        let mutation_tail = &batch[first_mutation..];
        for forbidden in [
            "?",
            "charges[",
            "binary_search",
            "dimension_entry(",
            "charge_consumed_open(",
            "checked_",
            "try_reserve",
            ".reserve(",
            ".push(",
            ".insert(",
            "expect(",
            "unwrap",
            "panic!",
        ] {
            assert!(
                !mutation_tail.contains(forbidden),
                "consumed batch regained a semantic, fallible, or allocating operation after its first mutation through `{forbidden}`"
            );
        }
    }

    #[test]
    fn reservation_transition_finishes_all_fallible_lookup_before_releasing_source_bytes() {
        let production = ledger_production_source();
        let transition = production
            .split("pub fn transition_reservations(")
            .nth(1)
            .expect("reservation transition exists")
            .split("/// Splits bytes")
            .next()
            .expect("reservation transition boundary exists");
        let dimension_lookup = transition
            .find("binary_search_by_key")
            .expect("target dimension is resolved before mutation");
        let phase_lookup = transition
            .find("phase_position(&state, phase)")
            .expect("target phase is resolved before mutation");
        let first_release = transition
            .find("release_positioned_bytes_locked")
            .expect("source bytes are released by numeric position");
        assert!(dimension_lookup < first_release);
        assert!(phase_lookup < first_release);
        assert!(
            transition[..first_release]
                .contains("reservation.core.dimension_position >= state.dimensions.len()")
        );
        assert!(
            transition[..first_release]
                .contains("reservation.core.phase_position >= state.phase_states.len()")
        );

        let mutation_tail = &transition[first_release..];
        for forbidden in [
            "?",
            "binary_search",
            "phase_position(",
            "dimension_entry(",
            "phase_state(",
            "release_phase(",
            "release_bytes_locked(",
            "try_reserve",
            ".reserve(",
            ".push(",
            ".insert(",
            "expect(",
            "unwrap",
            "panic!",
        ] {
            assert!(
                !mutation_tail.contains(forbidden),
                "transition regained a fallible or semantic operation after its first release through `{forbidden}`"
            );
        }
    }

    #[test]
    fn reservation_admission_and_resize_use_one_preflight_then_numeric_slots() {
        let production = ledger_production_source();
        let reserve = production
            .split("fn reserve_core(")
            .nth(1)
            .expect("reservation admission exists")
            .split("pub(crate) fn reserve_operation_attribution")
            .next()
            .expect("reservation admission boundary exists");
        let first_mutation = reserve
            .find("dimension_state.current_live_bytes = dimension_live")
            .expect("dimension accounting mutation exists");
        assert!(reserve[..first_mutation].contains("binary_search_by_key"));
        assert!(reserve[..first_mutation].contains("phase_position(&state, phase)"));
        for forbidden in [
            "?",
            "binary_search",
            "dimension_entry(",
            "dimension_entry_mut(",
            "phase_position(",
            "phase_state(",
            "try_reserve",
            ".reserve(",
            ".push(",
            ".insert(",
            "expect(",
            "unwrap",
            "panic!",
        ] {
            assert!(
                !reserve[first_mutation..].contains(forbidden),
                "reservation admission regained a fallible or semantic operation after its first write through `{forbidden}`"
            );
        }

        let resize = production
            .split("fn resize_core(")
            .nth(1)
            .expect("reservation resize exists")
            .split("/// Atomically replaces one reservation")
            .next()
            .expect("reservation resize boundary exists");
        assert!(resize.contains("let dimension_position = reservation.dimension_position"));
        assert!(resize.contains("let phase_position = reservation.phase_position"));
        assert!(resize.contains("dimension_position >= state.dimensions.len()"));
        assert!(resize.contains("phase_position >= state.phase_states.len()"));
        let first_resize_mutation = resize
            .find("dimension_state.current_live_bytes = dimension_live")
            .expect("resize dimension mutation exists");
        for forbidden in [
            "binary_search",
            "dimension_entry(",
            "dimension_entry_mut(",
            "phase_position(",
            "phase_state(",
        ] {
            assert!(
                !resize.contains(forbidden),
                "reservation resize ignored its bound numeric slot through `{forbidden}`"
            );
        }
        for forbidden in [
            "?",
            "try_reserve",
            ".reserve(",
            ".push(",
            ".insert(",
            "expect(",
            "unwrap",
            "panic!",
        ] {
            assert!(
                !resize[first_resize_mutation..].contains(forbidden),
                "reservation resize regained an allocation or fault gate after its first write through `{forbidden}`"
            );
        }
    }

    #[test]
    fn reservation_transfer_finishes_all_fallible_lookup_before_mutating_phase_bytes() {
        let production = ledger_production_source();
        let transfer = production
            .split("pub fn transfer(")
            .nth(1)
            .expect("reservation transfer exists")
            .split("/// Resizes one live reservation")
            .next()
            .expect("reservation transfer boundary exists");
        let target_lookup = transfer
            .find("phase_position(&state, phase)")
            .expect("target phase is resolved before mutation");
        let first_mutation = transfer
            .find("state.phase_states[source_position].current_live_bytes = source_live")
            .expect("source bytes are released by numeric position");
        assert!(target_lookup < first_mutation);
        assert!(transfer[..first_mutation].contains("source_position >= state.phase_states.len()"));

        let mutation_tail = &transfer[first_mutation..];
        for forbidden in [
            "?",
            "binary_search",
            "phase_position(",
            "phase_state(",
            "release_phase(",
            "try_reserve",
            ".reserve(",
            ".push(",
            ".insert(",
            "expect(",
            "unwrap",
            "panic!",
        ] {
            assert!(
                !mutation_tail.contains(forbidden),
                "transfer regained a fallible or semantic operation after its first write through `{forbidden}`"
            );
        }
    }

    #[test]
    fn legacy_publication_binds_commitment_positions_before_counter_mutation() {
        let production = ledger_production_source();
        let aggregate = production
            .split("struct AggregatedCommitment")
            .nth(1)
            .expect("commitment unwind guard exists")
            .split("fn validate_commitments")
            .next()
            .expect("commitment aggregate boundary exists");
        assert!(aggregate.contains("dimension_position: usize"));
        assert!(aggregate.contains("drop(std::mem::take(&mut self.entries))"));
        assert!(aggregate.contains("release_evidence(self.state, self.evidence_bytes)"));

        let prepare = production
            .split("pub fn prepare_publication(")
            .nth(1)
            .expect("legacy publication prepare exists")
            .split("/// Captures immutable owner-neutral evidence")
            .next()
            .expect("legacy prepare boundary exists");
        let prepare_write = prepare
            .find("entry.pending_publication += commitment.amount")
            .expect("pending commitment write exists");
        assert!(prepare[..prepare_write].contains("validate_commitments"));
        for forbidden in [
            "?",
            "binary_search",
            "dimension_entry(",
            "dimension_entry_mut(",
            "try_reserve",
            ".reserve(",
            ".push(",
            ".insert(",
            "expect(",
            "unwrap",
            "panic!",
        ] {
            assert!(
                !prepare[prepare_write..].contains(forbidden),
                "publication prepare regained a lookup, allocation, or fault gate after its first counter write through `{forbidden}`"
            );
        }

        let publish = production
            .split("pub(crate) fn publish(")
            .nth(1)
            .expect("legacy publication publish exists")
            .split("fn ensure_open")
            .next()
            .expect("legacy publish boundary exists");
        let publish_write = publish
            .find("entry.pending_publication -= commitment.amount")
            .expect("published commitment write exists");
        assert!(publish[..publish_write].contains("commitment.dimension_position"));
        assert!(!publish.contains("dimension_entry("));
        assert!(!publish.contains("dimension_entry_mut("));

        let rollback = production
            .split("impl<O, P, D> Drop for PendingPublication")
            .nth(1)
            .expect("pending publication rollback exists")
            .split("/// Immutable evidence")
            .next()
            .expect("pending publication rollback boundary exists");
        assert!(rollback.contains("get_mut(commitment.dimension_position)"));
        assert!(!rollback.contains("dimension_entry("));
        assert!(!rollback.contains("dimension_entry_mut("));
    }

    #[test]
    fn typed_freeze_proof_owns_exact_batches_and_has_no_post_mutation_fault_gate() {
        let production = ledger_production_source();
        let proof = production
            .split("pub(crate) struct TypedFreezeProof")
            .nth(1)
            .expect("typed proof exists")
            .split("impl<O, P, D> TypedFreezeProof")
            .next()
            .expect("typed proof fields exist");
        assert!(proof.contains("canonical: AttributionReservationBatch"));
        assert!(proof.contains("evidence: AttributionReservationBatch"));
        assert!(proof.contains("ledger: Ledger"));
        assert!(!proof.contains("PhantomData"));
        assert!(!production.contains("struct TypedFreezeQualification"));
        assert!(production.contains("struct QualifiedTypedFreezeProof"));
        assert!(!production.contains("pub(crate) struct QualifiedTypedFreezeProof"));
        assert!(production.contains("    fn qualify_terminal(self)"));
        assert!(!production.contains("pub(crate) fn qualify_terminal"));
        assert!(!production.contains("pub(crate) fn try_into_qualified("));
        assert_eq!(
            production.matches("pub(crate) fn freeze_in_place").count(),
            1,
            "only the freeze-ready typed wrapper may expose in-place publication"
        );
        assert_eq!(
            production.matches("pub(crate) fn freeze(mut self)").count(),
            1,
            "only the freeze-ready typed wrapper may expose consuming publication"
        );
        let generic_qualified = production
            .split("impl<O, P, D> QualifiedTypedFreezeProof")
            .nth(1)
            .expect("generic qualified proof implementation exists")
            .split("impl<O, P, D> QualifiedEvidenceDiscardFreezeProof")
            .next()
            .expect("generic qualified proof implementation is bounded");
        assert!(generic_qualified.contains("    fn freeze_in_place(&mut self)"));
        assert!(generic_qualified.contains("    fn freeze(mut self)"));
        assert!(!generic_qualified.contains("pub(crate) fn freeze"));

        let qualification = production
            .split("fn validate_terminal(")
            .nth(1)
            .expect("typed terminal qualification exists")
            .split("pub(crate) fn retain_fail_closed")
            .next()
            .expect("typed terminal qualification boundary exists");
        assert!(qualification.contains("typed_terminal_snapshot_positions_are_valid"));
        assert_eq!(
            qualification
                .matches("bound_positions_are_in_bounds")
                .count(),
            5,
            "published and deferred-storage batches must prove attribution, cover, and commitment positions"
        );
        assert_eq!(
            qualification
                .matches("discard_positions_are_in_bounds")
                .count(),
            2,
            "canonical and evidence discards must prove only physically released attribution and cover positions"
        );
        assert_eq!(
            qualification
                .matches("positioned_commitment_amount")
                .count(),
            2,
            "canonical and evidence commitments must be requalified under the final lock"
        );
        for required in [
            "checked_add(canonical_increment)",
            "entry.policy.committed_limit",
            "committed_canonical_output > committed",
            "checked_add(outcome_evidence_increment)",
            "entry.policy.published_outcome_evidence_limit",
            "state.publication = PublicationState::TerminalQualified",
        ] {
            assert!(
                qualification.contains(required),
                "terminal qualification lost final commitment/state gate `{required}`"
            );
        }
        for forbidden in [
            "validate_final_snapshot_shape",
            "binary_search",
            "PartialEq",
        ] {
            assert!(
                !qualification.contains(forbidden),
                "self-consuming qualification calls owner-defined key behavior through `{forbidden}`"
            );
        }
        let bound_positions = production
            .split("fn bound_positions_are_in_bounds")
            .nth(1)
            .expect("bound-position qualification exists")
            .split("fn release_bound_attributions")
            .next()
            .expect("bound-position qualification boundary exists");
        for required in [
            "token.core.dimension_position < dimension_count",
            "token.core.phase_position < phase_count",
            "cover.core.dimension_position < dimension_count",
            "cover.core.phase_position < phase_count",
            "commitment.dimension_position < dimension_count",
        ] {
            assert!(
                bound_positions.contains(required),
                "terminal qualification lost bound-position gate `{required}`"
            );
        }
        let discard_positions = production
            .split("fn discard_positions_are_in_bounds")
            .nth(1)
            .expect("discard-position qualification exists")
            .split("fn release_bound_attributions")
            .next()
            .expect("discard-position qualification boundary exists");
        for required in [
            "token.core.dimension_position < dimension_count",
            "token.core.phase_position < phase_count",
            "cover.core.dimension_position < dimension_count",
            "cover.core.phase_position < phase_count",
        ] {
            assert!(
                discard_positions.contains(required),
                "discard qualification lost physical-accounting position gate `{required}`"
            );
        }
        assert!(
            !discard_positions.contains("commitments"),
            "discard qualification must not apply a payload commitment that is being destroyed"
        );

        let freeze = production
            .split("fn freeze_in_place(&mut self)")
            .nth(1)
            .expect("in-place typed freeze exists")
            .split("    fn freeze(mut self)")
            .next()
            .expect("in-place typed freeze body exists");
        let mutation_tail = freeze
            .split("let snapshot = {")
            .nth(1)
            .expect("typed mutation tail exists");
        for forbidden in [
            "Result",
            "?",
            "assert",
            "checked_",
            "binary_search",
            "saturating_",
            "unwrap",
            "expect",
            "unreachable",
        ] {
            assert!(
                !mutation_tail.contains(forbidden),
                "freeze contains post-mutation fault gate `{forbidden}`"
            );
        }
        assert_eq!(freeze.matches("lock(").count(), 1);
        let release = freeze
            .find("release_bound_attributions")
            .expect("freeze releases bound attributions");
        let discard_canonical = freeze
            .find("drop(proof.discard_canonical.take())")
            .expect("freeze drops discarded canonical batch");
        let discard_evidence = freeze
            .find("drop(proof.discard_evidence.take())")
            .expect("freeze drops discarded evidence batch");
        let commitments = freeze
            .find("apply_typed_commitments")
            .expect("freeze applies bound commitments");
        let destroy_storage = freeze
            .find("destroy_storage_then_release_bound_cover")
            .expect("freeze destroys batch storage before releasing its cover");
        let published = freeze
            .find("PublicationState::Published")
            .expect("freeze publishes atomically");
        let final_snapshot = freeze
            .find("take_typed_final_snapshot")
            .expect("freeze moves preallocated final evidence");
        assert!(discard_canonical < release);
        assert!(discard_evidence < release);
        assert!(release < commitments);
        assert!(commitments < destroy_storage);
        assert!(destroy_storage < published);
        assert!(published < final_snapshot);
        let after_snapshot = &freeze[final_snapshot + "take_typed_final_snapshot".len()..];
        assert!(!after_snapshot.contains("lock("));
        assert!(!after_snapshot.contains("release_bound_attributions"));
        assert!(!after_snapshot.contains("destroy_storage_then_release_bound_cover"));
        assert!(!after_snapshot.contains("apply_typed_commitments"));

        let bound_release = production
            .split("fn release_bound_locked")
            .nth(1)
            .expect("bound release helper exists")
            .split("impl<O, P, D> Drop for ReservationCore")
            .next()
            .expect("bound release helper body exists");
        let apply = production
            .split("fn apply_typed_commitments")
            .nth(1)
            .expect("bound commitment helper exists")
            .split("struct SnapshotEvidenceAssembly")
            .next()
            .expect("bound commitment helper body exists");
        let batch_release = production
            .split("fn release_bound_attributions")
            .nth(1)
            .expect("bound batch release helper exists")
            .split("fn destroy_storage_then_release_bound_cover")
            .next()
            .expect("bound batch release helper body exists");
        assert!(!batch_release.contains("capacity_cover.core.release_bound_locked"));
        let storage_release = production
            .split("fn destroy_storage_then_release_bound_cover")
            .nth(1)
            .expect("storage-before-cover helper exists")
            .split("/// Live reservations submitted")
            .next()
            .expect("storage-before-cover helper body exists");
        let attribution_storage = storage_release
            .find("take(&mut self.attributions)")
            .expect("attribution vector storage is destroyed");
        let commitment_storage = storage_release
            .find("take(&mut self.commitments)")
            .expect("commitment vector storage is destroyed");
        let capacity_release = storage_release
            .find("capacity_cover.core.release_bound_locked")
            .expect("capacity cover is released under the ledger lock");
        assert!(attribution_storage < capacity_release);
        assert!(commitment_storage < capacity_release);
        let snapshot = production
            .split("fn take_typed_final_snapshot")
            .nth(1)
            .expect("typed snapshot helper exists")
            .split("const fn dimension_snapshot")
            .next()
            .expect("typed snapshot helper body exists");
        for (name, helper) in [
            ("bound release", bound_release),
            ("bound batch release", batch_release),
            ("commitment apply", apply),
            ("typed snapshot", snapshot),
        ] {
            for forbidden in [
                "Result",
                "?",
                "assert",
                "checked_",
                "binary_search",
                "saturating_",
                "unwrap",
                "expect",
                "unreachable",
                "lock(",
            ] {
                assert!(
                    !helper.contains(forbidden),
                    "{name} contains post-binding fault gate `{forbidden}`"
                );
            }
        }
        assert!(!freeze.contains("canonical:"));
        assert!(!freeze.contains("evidence:"));
    }
}
