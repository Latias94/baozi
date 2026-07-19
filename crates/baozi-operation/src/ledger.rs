use crate::{CapacityPlan, CapacityPlanError, OperationOwner};
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::{Arc, Mutex, MutexGuard};

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
            Self::LimitExceeded { .. } | Self::AllocationFailed { .. } => None,
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

impl<D: fmt::Debug> fmt::Display for LedgerCause<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDimension { dimension } => {
                write!(formatter, "unknown ledger dimension {dimension:?}")
            }
            Self::UnknownPhase => formatter.write_str("unknown ledger phase"),
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
    // Sorted lookup entries may move; their append-only state slots never do.
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
    phase_catalog: PhaseCatalogState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationState {
    Open,
    Pending,
    Published,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseCatalogState {
    Dynamic,
    Frozen,
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
    /// Creates an empty ledger with frozen initial policy and a unique runtime owner.
    pub fn new(limits: LedgerLimits<D>) -> Self {
        let LedgerLimits {
            dimensions,
            dimension_snapshot_staging,
            operation_live_byte_limit,
            evidence_byte_limit,
            current_evidence_bytes,
            peak_evidence_bytes,
        } = limits;
        Self {
            state: Arc::new(Mutex::new(LedgerState {
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
                phase_catalog: PhaseCatalogState::Dynamic,
            })),
            operation_live_byte_limit,
            operation_owner: PhantomData,
        }
    }

    pub(crate) fn try_new_with_phase_catalog(
        limits: LedgerLimits<D>,
        phases: &[P],
    ) -> Result<Self, LedgerCause<D>> {
        #[cfg(test)]
        PHASE_CATALOG_CONSTRUCTION_ENTRIES.with(|entries| {
            entries.set(entries.get().saturating_add(1));
        });
        let ledger = Self::new(limits);
        {
            let mut state = lock(&ledger.state);
            for phase in phases {
                ensure_phase(&mut state, *phase)?;
            }
            state.phase_catalog = PhaseCatalogState::Frozen;
        }
        Ok(ledger)
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
    pub fn charge_consumed_batch(&self, charges: &[(D, u64)]) -> Result<(), LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;

        let group = |start: usize| {
            let dimension = charges[start].0;
            let mut end = start;
            let mut amount = 0_u64;
            while let Some((candidate, next)) = charges.get(end).copied() {
                if candidate != dimension {
                    break;
                }
                amount = amount
                    .checked_add(next)
                    .ok_or(LedgerCause::ArithmeticOverflow {
                        class: CounterClass::Consumed,
                        dimension,
                    })?;
                end += 1;
            }
            Ok((end, dimension, amount))
        };

        let mut index = 0;
        let mut previous = None;
        while index < charges.len() {
            let (next, dimension, amount) = group(index)?;
            if let Some(previous) = previous
                && dimension < previous
            {
                return Err(LedgerCause::ConsumedBatchOutOfOrder {
                    previous,
                    dimension,
                });
            }
            let entry = dimension_entry(&state.dimensions, dimension)
                .ok_or(LedgerCause::UnknownDimension { dimension })?;
            let policy = entry.policy;
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
            if value > policy.consumed_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Consumed,
                    dimension,
                    value,
                    limit: policy.consumed_limit,
                });
            }
            previous = Some(dimension);
            index = next;
        }

        index = 0;
        while index < charges.len() {
            let (next, dimension, amount) =
                group(index).expect("validated consumed batch cannot overflow");
            Self::charge_consumed_open(&mut state, dimension, amount)
                .expect("validated consumed batch must apply without failure");
            index = next;
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
        let current_phase = phase_state(&state, phase).map_or(0, |state| state.current_live_bytes);
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

        let phase_position = ensure_phase(&mut state, phase)?;
        let dimension_state = &mut dimension_entry_mut(&mut state.dimensions, dimension)
            .expect("validated dimensions remain present")
            .state;
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
        })
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
        let target_live = phase_state(&state, phase)
            .map_or(0, |state| state.current_live_bytes)
            .checked_add(reservation.core.bytes)
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension: reservation.core.dimension,
            })?;
        let phase_position = ensure_phase(&mut state, phase)?;
        release_phase(&mut state, reservation.core.phase, reservation.core.bytes);
        let target = &mut state.phase_states[phase_position];
        target.current_live_bytes = target_live;
        target.peak_live_bytes = target.peak_live_bytes.max(target_live);
        reservation.core.phase = phase;
        reservation.core.phase_position = phase_position;
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
        let phase = reservation.phase;
        let dimension_limit = dimension_entry(&state.dimensions, dimension)
            .ok_or(LedgerCause::UnknownDimension { dimension })?
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
            dimension_entry(&state.dimensions, dimension)
                .expect("validated dimensions remain present")
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

        let phase_live =
            project(phase_state(&state, phase).map_or(0, |state| state.current_live_bytes))?;

        let phase_position = ensure_phase(&mut state, phase)?;
        let dimension_state = &mut dimension_entry_mut(&mut state.dimensions, dimension)
            .expect("validated dimensions remain present")
            .state;
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
        let target_dimension_source = reservations
            .iter()
            .filter(|reservation| reservation.core.dimension == dimension)
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
            .filter(|reservation| reservation.core.phase == phase)
            .try_fold(0_u64, |total, reservation| {
                total
                    .checked_add(reservation.core.bytes)
                    .ok_or(LedgerCause::ArithmeticOverflow {
                        class: CounterClass::Live,
                        dimension,
                    })
            })?;

        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        let dimension_limit = dimension_entry(&state.dimensions, dimension)
            .ok_or(LedgerCause::UnknownDimension { dimension })?
            .policy
            .live_byte_limit
            .ok_or(LedgerCause::DimensionNotReservable { dimension })?;
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
            dimension_entry(&state.dimensions, dimension)
                .expect("validated dimensions remain present")
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
            phase_state(&state, phase).map_or(0, |state| state.current_live_bytes),
            target_phase_source,
        )?;
        let dimension_position = state
            .dimensions
            .binary_search_by_key(&dimension, |entry| entry.dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        let phase_position = ensure_phase(&mut state, phase)?;

        for reservation in reservations.iter() {
            release_bytes_locked(
                &mut state,
                reservation.core.phase,
                reservation.core.dimension,
                reservation.core.bytes,
            );
        }
        let dimension_state = &mut dimension_entry_mut(&mut state.dimensions, dimension)
            .expect("validated dimensions remain present")
            .state;
        dimension_state.current_live_bytes = dimension_live;
        dimension_state.peak_live_bytes = dimension_state.peak_live_bytes.max(dimension_live);
        let phase_state = &mut state.phase_states[phase_position];
        phase_state.current_live_bytes = phase_live;
        phase_state.peak_live_bytes = phase_state.peak_live_bytes.max(phase_live);
        state.current_live_bytes = operation_live;
        state.peak_live_bytes = state.peak_live_bytes.max(operation_live);
        drop(state);

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
            PublicationState::Published => return Err(LedgerCause::PublicationAlreadyCompleted),
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
        for (dimension, amount) in &aggregate {
            let entry = &mut dimension_entry_mut(&mut state.dimensions, *dimension)
                .expect("validated commitments originate from frozen policy")
                .state;
            entry.pending_publication = entry
                .pending_publication
                .checked_add(*amount)
                .expect("commitment validation checked pending arithmetic");
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

    pub(crate) fn contains_dimension(&self, dimension: D) -> bool {
        dimension_entry(&lock(&self.state).dimensions, dimension).is_some()
    }

    pub(crate) fn contains_phase(&self, phase: P) -> bool {
        phase_entry(&lock(&self.state).phases, phase).is_some()
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

    #[allow(
        dead_code,
        reason = "crate-private until retained startup consumes the borrowed binding seam"
    )]
    pub(crate) fn try_bind_fallback_baseline(
        &self,
        baseline: &mut FallbackBatchBaseline<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_fallback_baseline(baseline)?;
        self.bind_fallback_baseline(baseline)
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    fn bind_fallback_baseline(
        &self,
        baseline: &mut FallbackBatchBaseline<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        let state = lock(&self.state);
        preflight_typed_publication_batch(&state, &baseline.canonical_assembly)?;
        preflight_typed_publication_batch(&state, &baseline.evidence_assembly)?;
        preflight_typed_publication_batch(&state, &baseline.fallback)?;
        preflight_typed_publication_batch(&state, &baseline.evidence)?;
        bind_preflighted_typed_publication_batch(&state, &mut baseline.canonical_assembly);
        bind_preflighted_typed_publication_batch(&state, &mut baseline.evidence_assembly);
        bind_preflighted_typed_publication_batch(&state, &mut baseline.fallback);
        bind_preflighted_typed_publication_batch(&state, &mut baseline.evidence);
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    fn validate_fallback_baseline(
        &self,
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
        let state = lock(&self.state);
        Self::ensure_open(&state)?;
        validate_final_snapshot_shape(&state)?;
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
        mut canonical_source: Option<&mut AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        mut evidence_source: Option<
            &mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        >,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_primary_publication(
            baseline,
            canonical_source.as_deref(),
            evidence_source.as_deref(),
        )?;
        let state = lock(&self.state);
        preflight_typed_publication_batch(&state, &baseline.canonical_assembly)?;
        preflight_typed_publication_batch(&state, &baseline.evidence_assembly)?;
        preflight_typed_publication_batch(&state, &baseline.evidence)?;
        if let Some(batch) = canonical_source.as_deref() {
            preflight_typed_publication_batch(&state, batch)?;
        }
        if let Some(batch) = evidence_source.as_deref() {
            preflight_typed_publication_batch(&state, batch)?;
        }
        bind_preflighted_typed_publication_batch(&state, &mut baseline.canonical_assembly);
        bind_preflighted_typed_publication_batch(&state, &mut baseline.evidence_assembly);
        bind_preflighted_typed_publication_batch(&state, &mut baseline.evidence);
        if let Some(batch) = canonical_source.as_deref_mut() {
            bind_preflighted_typed_publication_batch(&state, batch);
        }
        if let Some(batch) = evidence_source.as_deref_mut() {
            bind_preflighted_typed_publication_batch(&state, batch);
        }
        drop(state);

        if let Some(source) = canonical_source {
            baseline.canonical_assembly.absorb_proven(source);
        }
        baseline
            .evidence_assembly
            .absorb_proven(&mut baseline.evidence);
        if let Some(source) = evidence_source {
            baseline.evidence_assembly.absorb_proven(source);
        }
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
    ) -> TypedFreezeProof<O, P, D> {
        let FallbackBatchBaseline {
            canonical_assembly,
            evidence_assembly,
            fallback,
            evidence: _,
        } = baseline;
        drop(canonical_source);
        drop(evidence_source);
        TypedFreezeProof {
            canonical: canonical_assembly,
            evidence: evidence_assembly,
            discard_canonical: None,
            discard_evidence: Some(fallback),
            ledger: self,
        }
    }

    #[allow(
        dead_code,
        reason = "crate-private until the U4 canonical consumers switch atomically"
    )]
    fn validate_primary_publication(
        &self,
        baseline: &FallbackBatchBaseline<O, P, D>,
        canonical_source: Option<&AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<&AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
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
        if !baseline
            .canonical_assembly
            .can_absorb_sources([canonical_source])
            || !baseline
                .evidence_assembly
                .can_absorb_sources([Some(&baseline.evidence), evidence_source])
        {
            return Err(LedgerCause::ActiveReservationsRemain { count: u64::MAX });
        }

        let canonical_attribution_count = baseline
            .canonical_assembly
            .combined_attribution_count([canonical_source])
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
            .combined_attribution_bytes([canonical_source])
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
        let released_active =
            u64::from(canonical_source.is_some_and(|batch| batch.capacity_cover.is_some()))
                .checked_add(u64::from(baseline.evidence.capacity_cover.is_some()))
                .and_then(|count| {
                    count.checked_add(u64::from(
                        evidence_source.is_some_and(|batch| batch.capacity_cover.is_some()),
                    ))
                })
                .and_then(|count| count.checked_add(baseline.fallback.active_count()?))
                .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let released_live = canonical_source
            .map_or(0, AttributionReservationBatch::account_capacity_bytes)
            .checked_add(baseline.evidence.account_capacity_bytes())
            .and_then(|bytes| {
                bytes.checked_add(
                    evidence_source.map_or(0, AttributionReservationBatch::account_capacity_bytes),
                )
            })
            .and_then(|bytes| bytes.checked_add(baseline.fallback.live_bytes()?))
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;

        let state = lock(&self.state);
        Self::ensure_open(&state)?;
        validate_final_snapshot_shape(&state)?;
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
            [Some(&baseline.canonical_assembly), canonical_source],
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
        let live_cause =
            (state.current_live_bytes != 0).then_some(LedgerCause::LiveReservationsRemain {
                bytes: state.current_live_bytes,
            });
        let pending_cause = (state.publication == PublicationState::Pending)
            .then_some(LedgerCause::PendingPublicationsRemain);
        let reservation_cause = (state.current_live_bytes == 0 && state.active_reservations != 0)
            .then_some(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            });
        let clean = live_cause.is_none() && pending_cause.is_none() && reservation_cause.is_none();
        let evidence = take_final_snapshot(&mut state, clean);
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

        for (dimension, amount) in &publication.commitments {
            let entry = &dimension_entry(&state.dimensions, *dimension)
                .expect("pending dimensions originate from frozen policy")
                .state;
            entry.pending_publication.checked_sub(*amount).ok_or(
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::PendingPublication,
                    dimension: *dimension,
                },
            )?;
            entry
                .committed
                .checked_add(*amount)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Committed,
                    dimension: *dimension,
                })?;
        }

        for (dimension, amount) in &publication.commitments {
            let entry = &mut dimension_entry_mut(&mut state.dimensions, *dimension)
                .expect("pending dimensions originate from frozen policy")
                .state;
            entry.pending_publication -= *amount;
            entry.committed += *amount;
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
            PublicationState::Published => Err(LedgerCause::PublicationAlreadyCompleted),
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
/// let mut operation = OperationCoordinator::<Owner, Phase, Progress, Unit, Dimension>::new(
///     OperationControl::default(), limits,
/// );
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
/// let mut import = OperationCoordinator::<ImportOwner, Phase, Progress, Unit, Dimension>::new(
///     OperationControl::default(), limits(),
/// );
/// let mut postprocess = OperationCoordinator::<PostProcessOwner, Phase, Progress, Unit, Dimension>::new(
///     OperationControl::default(), limits(),
/// );
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

    fn publication_positions(
        &self,
        state: &LedgerState<P, D>,
    ) -> Result<(usize, usize), LedgerCause<D>> {
        if !self.active {
            return Err(LedgerCause::InactiveReservation);
        }
        let dimension_position = state
            .dimensions
            .binary_search_by_key(&self.dimension, |entry| entry.dimension)
            .map_err(|_| LedgerCause::UnknownDimension {
                dimension: self.dimension,
            })?;
        let phase_position = state
            .phases
            .binary_search_by_key(&self.phase, |entry| entry.phase)
            .map_err(|_| LedgerCause::FinalSnapshotShapeMismatch)
            .map(|position| state.phases[position].state_position)?;
        if state.phase_states.get(phase_position).is_none() {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        Ok((dimension_position, phase_position))
    }

    fn bind_publication_positions(&mut self, state: &LedgerState<P, D>) {
        let (dimension_position, phase_position) = self
            .publication_positions(state)
            .unwrap_or_else(|_| panic!("preflighted publication position became invalid"));
        self.dimension_position = dimension_position;
        self.phase_position = phase_position;
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

const STARTUP_ACCOUNT_SHARE_STEP_COUNT: usize = 4;

/// One retained allocation attribution that startup must be able to reserve.
///
/// The phase remains part of the qualified value even though the current frozen policy has no
/// phase-specific limit. That keeps startup qualification ready for a future phase-qualified
/// `LedgerLimits<P, D>` without weakening today's dimension and operation-wide checks.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartupRetainedAttribution<P, D> {
    phase: P,
    dimension: D,
    bytes: u64,
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<P, D> StartupRetainedAttribution<P, D> {
    pub(crate) const fn new(phase: P, dimension: D, bytes: u64) -> Self {
        Self {
            phase,
            dimension,
            bytes,
        }
    }

    pub(crate) fn into_parts(self) -> (P, D, u64) {
        (self.phase, self.dimension, self.bytes)
    }
}

/// One linear attempted-and-consumed work step paired with one startup account share.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
pub(crate) struct StartupAccountShareStep<D> {
    charges: [(D, u64); 2],
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<D> StartupAccountShareStep<D> {
    pub(crate) fn into_charges(self) -> [(D, u64); 2] {
        self.charges
    }
}

/// The exact linear work steps consumed by retained startup's four account shares.
///
/// The fixed step count is defined only by this envelope. Qualification and execution both consume
/// these stored steps, so adding or removing an account share cannot silently drift from the pure
/// ledger proof.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
pub(crate) struct StartupAccountShareEnvelope<D> {
    steps: [StartupAccountShareStep<D>; STARTUP_ACCOUNT_SHARE_STEP_COUNT],
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<D: Copy> StartupAccountShareEnvelope<D> {
    pub(crate) const fn four_identical(charges: [(D, u64); 2]) -> Self {
        Self {
            steps: [
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
            ],
        }
    }

    pub(crate) fn into_steps(
        self,
    ) -> [StartupAccountShareStep<D>; STARTUP_ACCOUNT_SHARE_STEP_COUNT] {
        self.steps
    }
}

/// The four destination-qualified attribution batches required by retained startup.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
pub(crate) struct StartupAttributionBatchPlans<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical_assembly: AttributionBatchPlan<O, P, D, CanonicalOutput>,
    evidence_assembly: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    fallback: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    evidence: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<O, P, D> StartupAttributionBatchPlans<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) const fn new(
        canonical_assembly: AttributionBatchPlan<O, P, D, CanonicalOutput>,
        evidence_assembly: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
        fallback: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
        evidence: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    ) -> Self {
        Self {
            canonical_assembly,
            evidence_assembly,
            fallback,
            evidence,
        }
    }

    #[allow(
        clippy::type_complexity,
        reason = "the tuple preserves four static destinations"
    )]
    pub(crate) fn into_parts(
        self,
    ) -> (
        AttributionBatchPlan<O, P, D, CanonicalOutput>,
        AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
        AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
        AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    ) {
        (
            self.canonical_assembly,
            self.evidence_assembly,
            self.fallback,
            self.evidence,
        )
    }
}

/// Consuming output of one successful startup-ledger qualification.
///
/// This remains crate-private: outside callers cannot inspect or synthesize startup admission
/// evidence, while the retained owner can consume the exact typed inputs without reconstructing
/// them after crossing the side-effect boundary.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
pub(crate) struct StartupLedgerPlanParts<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) batches: StartupAttributionBatchPlans<O, P, D>,
    pub(crate) retained_attributions: [StartupRetainedAttribution<P, D>; 2],
    pub(crate) account_shares: StartupAccountShareEnvelope<D>,
    pub(crate) commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
}

/// Sealed proof that every deterministic ledger action needed by retained startup fits policy.
///
/// The only constructor is `LedgerLimits::try_qualify_startup`. Qualification is read-only and
/// allocation-free, so ordinary policy rejection happens before a ledger, coordinator, physical
/// account admission, or allocator can observe startup.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
#[must_use = "startup qualification must be consumed before crossing the side-effect boundary"]
pub(crate) struct StartupLedgerPlan<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    limits: LedgerLimits<D>,
    parts: StartupLedgerPlanParts<O, P, D>,
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<O, P, D> StartupLedgerPlan<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn into_parts(self) -> (LedgerLimits<D>, StartupLedgerPlanParts<O, P, D>) {
        (self.limits, self.parts)
    }
}

#[derive(Debug, Clone, Copy)]
struct StartupLiveEnvelope<D> {
    dimension: D,
    bytes: u64,
}

impl<D> StartupLiveEnvelope<D> {
    const fn new(dimension: D, bytes: u64) -> Self {
        Self { dimension, bytes }
    }
}

impl<D> LedgerLimits<D>
where
    D: Copy + Ord + fmt::Debug,
{
    /// Purely qualifies the complete deterministic ledger envelope for retained startup.
    #[allow(
        dead_code,
        reason = "crate-private until retained startup consumes the pure qualification proof"
    )]
    pub(crate) fn try_qualify_startup<O, P>(
        self,
        batches: StartupAttributionBatchPlans<O, P, D>,
        retained_attributions: [StartupRetainedAttribution<P, D>; 2],
        account_shares: StartupAccountShareEnvelope<D>,
        commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
    ) -> Result<StartupLedgerPlan<O, P, D>, LedgerCause<D>>
    where
        O: OperationOwner,
        P: Copy + Ord,
    {
        let live = [
            StartupLiveEnvelope::new(
                batches.fallback.capacity_dimension,
                batches.fallback.planned_bytes,
            ),
            StartupLiveEnvelope::new(
                retained_attributions[0].dimension,
                retained_attributions[0].bytes,
            ),
            StartupLiveEnvelope::new(
                batches.evidence.capacity_dimension,
                batches.evidence.planned_bytes,
            ),
            StartupLiveEnvelope::new(
                retained_attributions[1].dimension,
                retained_attributions[1].bytes,
            ),
            StartupLiveEnvelope::new(
                batches.canonical_assembly.capacity_dimension,
                batches.canonical_assembly.planned_bytes,
            ),
            StartupLiveEnvelope::new(
                batches.evidence_assembly.capacity_dimension,
                batches.evidence_assembly.planned_bytes,
            ),
        ];

        self.validate_startup_live_envelope(&live)?;
        self.validate_startup_account_shares(&account_shares)?;
        self.validate_startup_outcome_commitments(commitments)?;

        Ok(StartupLedgerPlan {
            limits: self,
            parts: StartupLedgerPlanParts {
                batches,
                retained_attributions,
                account_shares,
                commitments,
            },
        })
    }

    fn validate_startup_live_envelope<const N: usize>(
        &self,
        live: &[StartupLiveEnvelope<D>; N],
    ) -> Result<(), LedgerCause<D>> {
        let mut operation_live = 0_u64;
        for (index, envelope) in live.iter().copied().enumerate() {
            let entry = dimension_entry(&self.dimensions, envelope.dimension).ok_or(
                LedgerCause::UnknownDimension {
                    dimension: envelope.dimension,
                },
            )?;
            let dimension_limit =
                entry
                    .policy
                    .live_byte_limit
                    .ok_or(LedgerCause::DimensionNotReservable {
                        dimension: envelope.dimension,
                    })?;
            let dimension_live = live[..=index]
                .iter()
                .filter(|candidate| candidate.dimension == envelope.dimension)
                .try_fold(0_u64, |bytes, candidate| {
                    bytes
                        .checked_add(candidate.bytes)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Live,
                            dimension: envelope.dimension,
                        })
                })?;
            if dimension_live > dimension_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: envelope.dimension,
                    value: dimension_live,
                    limit: dimension_limit,
                });
            }
            operation_live = operation_live.checked_add(envelope.bytes).ok_or(
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: envelope.dimension,
                },
            )?;
            if operation_live > self.operation_live_byte_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: envelope.dimension,
                    value: operation_live,
                    limit: self.operation_live_byte_limit,
                });
            }
        }
        Ok(())
    }

    fn validate_startup_account_shares(
        &self,
        account_shares: &StartupAccountShareEnvelope<D>,
    ) -> Result<(), LedgerCause<D>> {
        let all = || {
            account_shares
                .steps
                .iter()
                .flat_map(|step| step.charges.iter())
                .copied()
        };
        for (index, (dimension, _)) in all().enumerate() {
            let entry = dimension_entry(&self.dimensions, dimension)
                .ok_or(LedgerCause::UnknownDimension { dimension })?;
            if all().take(index).any(|(previous, _)| previous == dimension) {
                continue;
            }

            let attempted = all()
                .filter(|(candidate, _)| *candidate == dimension)
                .try_fold(0_u64, |total, (_, amount)| {
                    total
                        .checked_add(amount)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Attempted,
                            dimension,
                        })
                })?;
            if attempted > entry.policy.attempted_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Attempted,
                    dimension,
                    value: attempted,
                    limit: entry.policy.attempted_limit,
                });
            }

            let consumed = all()
                .filter(|(candidate, _)| *candidate == dimension)
                .try_fold(0_u64, |total, (_, amount)| {
                    total
                        .checked_add(amount)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Consumed,
                            dimension,
                        })
                })?;
            if consumed > attempted {
                return Err(LedgerCause::ConsumedExceedsAttempted {
                    dimension,
                    consumed,
                    attempted,
                });
            }
            if consumed > entry.policy.consumed_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Consumed,
                    dimension,
                    value: consumed,
                    limit: entry.policy.consumed_limit,
                });
            }
        }
        Ok(())
    }

    fn validate_startup_outcome_commitments(
        &self,
        commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
    ) -> Result<(), LedgerCause<D>> {
        for (index, commitment) in commitments.iter().copied().enumerate() {
            let entry = dimension_entry(&self.dimensions, commitment.dimension).ok_or(
                LedgerCause::UnknownDimension {
                    dimension: commitment.dimension,
                },
            )?;
            if commitments[..index]
                .iter()
                .any(|previous| previous.dimension == commitment.dimension)
            {
                continue;
            }
            let amount = commitments
                .iter()
                .filter(|candidate| candidate.dimension == commitment.dimension)
                .try_fold(0_u64, |total, candidate| {
                    total
                        .checked_add(candidate.amount)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::PublishedOutcomeEvidence,
                            dimension: commitment.dimension,
                        })
                })?;
            if amount > entry.policy.published_outcome_evidence_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::PublishedOutcomeEvidence,
                    dimension: commitment.dimension,
                    value: amount,
                    limit: entry.policy.published_outcome_evidence_limit,
                });
            }
        }
        Ok(())
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
        self.evidence_assembly.absorb_proven(&mut self.evidence);
        self.evidence_assembly.absorb_proven(&mut self.fallback);
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
            fallback: _,
            evidence: _,
        } = self;
        TypedFreezeProof {
            canonical: canonical_assembly,
            evidence: evidence_assembly,
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
    discard_canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    discard_evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    ledger: Ledger<O, P, D>,
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
    pub(crate) fn freeze(mut self) -> LedgerSnapshot<P, D> {
        drop(self.discard_canonical.take());
        drop(self.discard_evidence.take());

        {
            let mut state = lock(&self.ledger.state);
            self.canonical.release_bound_attributions(&mut state);
            self.evidence.release_bound_attributions(&mut state);
            apply_typed_commitments(&mut state.dimensions, &self.canonical);
            apply_typed_commitments(&mut state.dimensions, &self.evidence);
            self.canonical
                .destroy_storage_then_release_bound_cover(&mut state);
            self.evidence
                .destroy_storage_then_release_bound_cover(&mut state);
            state.publication = PublicationState::Published;
            take_typed_final_snapshot(&mut state)
        }
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

    pub(crate) fn try_push_attribution(
        &mut self,
        token: OperationAttributionReservation<O, P, D, Destination>,
    ) -> Result<(), OperationAttributionReservation<O, P, D, Destination>> {
        let Some(cover) = self.capacity_cover.as_ref() else {
            return Err(token);
        };
        if !self.has_attribution_slot() || !token.core.is_owned_by(&cover.core.owner) {
            return Err(token);
        }
        self.attributions.push(token);
        Ok(())
    }

    pub(crate) fn try_push_commitment(
        &mut self,
        commitment: OutputCommitment<D, Destination>,
    ) -> Result<(), OutputCommitment<D, Destination>> {
        if self.commitments.len() >= self.commitment_limit
            || self.commitments.len() >= self.commitments.capacity()
        {
            return Err(commitment);
        }
        self.commitments.push(commitment);
        Ok(())
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

    fn absorb_proven(&mut self, other: &mut Self) {
        self.attributions.append(&mut other.attributions);
        self.commitments.append(&mut other.commitments);
        drop(std::mem::take(&mut other.attributions));
        drop(std::mem::take(&mut other.commitments));
        drop(other.capacity_cover.take());
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
        self.attributions
            .iter()
            .all(|token| token.core.active && token.core.is_owned_by(owner))
            && self
                .capacity_cover
                .as_ref()
                .is_none_or(|cover| cover.core.active && cover.core.is_owned_by(owner))
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
    commitments: Vec<(D, u64)>,
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
        for (dimension, amount) in &self.commitments {
            if let Some(entry) = dimension_entry_mut(&mut state.dimensions, *dimension) {
                debug_assert!(entry.state.pending_publication >= *amount);
                entry.state.pending_publication =
                    entry.state.pending_publication.saturating_sub(*amount);
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
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
}

impl<P: Copy + Ord, D: Copy + Ord> LedgerSnapshot<P, D> {
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

fn release_bytes_locked<P, D>(state: &mut LedgerState<P, D>, phase: P, dimension: D, bytes: u64)
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    state.current_live_bytes = state.current_live_bytes.saturating_sub(bytes);
    if let Some(entry) = dimension_entry_mut(&mut state.dimensions, dimension) {
        entry.state.current_live_bytes = entry.state.current_live_bytes.saturating_sub(bytes);
    }
    release_phase(state, phase, bytes);
}

fn release_phase<P: Copy + Ord, D>(state: &mut LedgerState<P, D>, phase: P, bytes: u64) {
    if let Some(phase_state) = phase_state_mut(state, phase) {
        phase_state.current_live_bytes = phase_state.current_live_bytes.saturating_sub(bytes);
    }
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
    phases
        .binary_search_by_key(&phase, |entry| entry.phase)
        .ok()
        .map(|position| &phases[position])
}

fn phase_state<P: Copy + Ord, D>(state: &LedgerState<P, D>, phase: P) -> Option<&PhaseState> {
    let position = phase_entry(&state.phases, phase)?.state_position;
    state.phase_states.get(position)
}

fn phase_state_mut<P: Copy + Ord, D>(
    state: &mut LedgerState<P, D>,
    phase: P,
) -> Option<&mut PhaseState> {
    let position = phase_entry(&state.phases, phase)?.state_position;
    state.phase_states.get_mut(position)
}

fn ensure_phase<P: Copy + Ord, D>(
    state: &mut LedgerState<P, D>,
    phase: P,
) -> Result<usize, LedgerCause<D>> {
    let position = match state
        .phases
        .binary_search_by_key(&phase, |entry| entry.phase)
    {
        Ok(position) => return Ok(state.phases[position].state_position),
        Err(position) => position,
    };
    if state.phase_catalog == PhaseCatalogState::Frozen {
        return Err(LedgerCause::UnknownPhase);
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
    state.phases.insert(
        position,
        PhaseEntry {
            phase,
            state_position,
        },
    );
    state
        .phase_snapshot_staging
        .insert(position, (phase, PhaseSnapshot::default()));
    Ok(state_position)
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
    debug_assert_eq!(capacity_bytes(values).ok(), Some(*retained_bytes));
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
    let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), desired_capacity)
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    plan.verify_preflight(values.len(), values.capacity())
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    let envelope_delta = plan
        .envelope_delta_bytes()
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    let admitted = current_evidence_bytes.checked_add(envelope_delta).ok_or(
        LedgerEvidenceError::Capacity {
            purpose,
            cause: CapacityPlanError::CapacityOverflow,
        },
    )?;
    if admitted > evidence_byte_limit {
        return Err(LedgerEvidenceError::LimitExceeded {
            purpose,
            value: admitted,
            limit: evidence_byte_limit,
        });
    }
    *current_evidence_bytes = admitted;
    *peak_evidence_bytes = (*peak_evidence_bytes).max(admitted);
    let original_capacity = values.capacity();
    #[cfg(test)]
    if test_take_evidence_allocation_failure() {
        *current_evidence_bytes = current_evidence_bytes.saturating_sub(envelope_delta);
        return Err(LedgerEvidenceError::AllocationFailed {
            purpose,
            requested_bytes: envelope_delta,
        });
    }
    if values
        .try_reserve_exact(plan.allocator_additional())
        .is_err()
    {
        *current_evidence_bytes = current_evidence_bytes.saturating_sub(envelope_delta);
        return Err(LedgerEvidenceError::AllocationFailed {
            purpose,
            requested_bytes: envelope_delta,
        });
    }
    let actual_delta = capacity_delta_bytes::<T>(original_capacity, values.capacity())
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    let updated_retained_bytes = retained_bytes
        .as_deref()
        .map(|retained| {
            retained
                .checked_add(actual_delta)
                .ok_or(LedgerEvidenceError::Capacity {
                    purpose,
                    cause: CapacityPlanError::CapacityOverflow,
                })
        })
        .transpose()?;
    *current_evidence_bytes = current_evidence_bytes
        .saturating_sub(envelope_delta)
        .checked_add(actual_delta)
        .ok_or(LedgerEvidenceError::Capacity {
            purpose,
            cause: CapacityPlanError::CapacityOverflow,
        })?;
    if let (Some(retained_bytes), Some(updated_retained_bytes)) =
        (retained_bytes, updated_retained_bytes)
    {
        *retained_bytes = updated_retained_bytes;
    }
    *peak_evidence_bytes = (*peak_evidence_bytes).max(*current_evidence_bytes);
    plan.finish(values.len(), values.capacity())
        .map(|_| ())
        .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })
}

fn capacity_bytes<T>(values: &Vec<T>) -> Result<u64, CapacityPlanError> {
    let bytes = values
        .capacity()
        .checked_mul(size_of::<T>())
        .ok_or(CapacityPlanError::CapacityOverflow)?;
    u64::try_from(bytes).map_err(|_| CapacityPlanError::CapacityOverflow)
}

fn capacity_delta_bytes<T>(
    original_capacity: usize,
    actual_capacity: usize,
) -> Result<u64, CapacityPlanError> {
    let capacity_delta = actual_capacity
        .checked_sub(original_capacity)
        .ok_or(CapacityPlanError::CapacityOverflow)?;
    let bytes = capacity_delta
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
    entries: Vec<(D, u64)>,
    evidence_bytes: u64,
}

fn aggregate_commitments<P, D>(
    state: &mut LedgerState<P, D>,
    commitments: &[Commitment<D>],
) -> Result<CommitmentAggregate<D>, AggregateCommitmentError<D>>
where
    D: Copy + Ord,
{
    let mut aggregate: Vec<(D, u64)> = Vec::new();
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
    let evidence_bytes = capacity_bytes(&aggregate).map_err(|cause| {
        AggregateCommitmentError::Evidence(LedgerEvidenceError::Capacity {
            purpose: LedgerEvidencePurpose::Publication,
            cause,
        })
    })?;
    for commitment in commitments {
        if dimension_entry(&state.dimensions, commitment.dimension).is_none() {
            drop(aggregate);
            release_evidence(state, evidence_bytes);
            return Err(AggregateCommitmentError::Ledger(
                LedgerCause::UnknownDimension {
                    dimension: commitment.dimension,
                },
            ));
        }
        match aggregate.binary_search_by_key(&commitment.dimension, |(dimension, _)| *dimension) {
            Ok(position) => {
                let Some(amount) = aggregate[position].1.checked_add(commitment.amount) else {
                    drop(aggregate);
                    release_evidence(state, evidence_bytes);
                    return Err(AggregateCommitmentError::Ledger(
                        LedgerCause::ArithmeticOverflow {
                            class: CounterClass::PendingPublication,
                            dimension: commitment.dimension,
                        },
                    ));
                };
                aggregate[position].1 = amount;
            }
            Err(position) => aggregate.insert(position, (commitment.dimension, commitment.amount)),
        }
    }
    Ok(CommitmentAggregate {
        entries: aggregate,
        evidence_bytes,
    })
}

fn validate_commitments<D: Copy + Ord>(
    dimensions: &[DimensionEntry<D>],
    commitments: &[(D, u64)],
) -> Result<(), LedgerCause<D>> {
    for (dimension, amount) in commitments {
        let entry =
            dimension_entry(dimensions, *dimension).ok_or(LedgerCause::UnknownDimension {
                dimension: *dimension,
            })?;
        let admitted = entry
            .state
            .committed
            .checked_add(entry.state.pending_publication)
            .and_then(|value| value.checked_add(*amount))
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: CounterClass::PendingPublication,
                dimension: *dimension,
            })?;
        if admitted > entry.policy.committed_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Committed,
                dimension: *dimension,
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
    for commitment in &batch.commitments {
        state
            .dimensions
            .binary_search_by_key(&commitment.dimension, |entry| entry.dimension)
            .map_err(|_| LedgerCause::UnknownDimension {
                dimension: commitment.dimension,
            })?;
    }
    for attribution in &batch.attributions {
        attribution.core.publication_positions(state)?;
    }
    if let Some(capacity_cover) = &batch.capacity_cover {
        capacity_cover.core.publication_positions(state)?;
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn bind_preflighted_typed_publication_batch<O, P, D, Destination>(
    state: &LedgerState<P, D>,
    batch: &mut AttributionReservationBatch<O, P, D, Destination>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    for commitment in &mut batch.commitments {
        commitment.dimension_position = state
            .dimensions
            .binary_search_by_key(&commitment.dimension, |entry| entry.dimension)
            .unwrap_or_else(|_| panic!("preflighted commitment dimension became invalid"));
    }
    for attribution in &mut batch.attributions {
        attribution.core.bind_publication_positions(state);
    }
    if let Some(capacity_cover) = &mut batch.capacity_cover {
        capacity_cover.core.bind_publication_positions(state);
    }
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

fn try_snapshot<P, D>(
    state: &mut LedgerState<P, D>,
) -> Result<LedgerSnapshot<P, D>, LedgerEvidenceError>
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    let mut dimensions = Vec::new();
    if let Err(cause) = try_grow_evidence_vec(
        &mut dimensions,
        state.dimensions.len(),
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Snapshot,
    ) {
        let retained = capacity_bytes(&dimensions).unwrap_or(0);
        release_evidence(state, retained);
        return Err(cause);
    }
    for entry in &state.dimensions {
        dimensions.push((entry.dimension, dimension_snapshot(entry.state)));
    }
    let dimension_bytes =
        capacity_bytes(&dimensions).map_err(|cause| LedgerEvidenceError::Capacity {
            purpose: LedgerEvidencePurpose::Snapshot,
            cause,
        })?;

    let mut phases = Vec::new();
    if let Err(cause) = try_grow_evidence_vec(
        &mut phases,
        state.phases.len(),
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Snapshot,
    ) {
        let phase_bytes = capacity_bytes(&phases).unwrap_or(0);
        release_evidence(state, dimension_bytes.saturating_add(phase_bytes));
        return Err(cause);
    }
    for entry in &state.phases {
        phases.push((
            entry.phase,
            phase_snapshot(state.phase_states[entry.state_position]),
        ));
    }
    let phase_bytes = capacity_bytes(&phases).map_err(|cause| LedgerEvidenceError::Capacity {
        purpose: LedgerEvidencePurpose::Snapshot,
        cause,
    })?;
    let outgoing_bytes = dimension_bytes.saturating_add(phase_bytes);
    let current_evidence_bytes = state.current_evidence_bytes.saturating_sub(outgoing_bytes);
    let snapshot = LedgerSnapshot {
        dimensions,
        phases,
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
        active_reservations: state.active_reservations,
        current_evidence_bytes,
        peak_evidence_bytes: state.peak_evidence_bytes,
    };
    release_evidence(state, outgoing_bytes);
    Ok(snapshot)
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
fn validate_final_snapshot_shape<P, D>(state: &LedgerState<P, D>) -> Result<(), LedgerCause<D>> {
    if state.dimensions.len() != state.dimension_snapshot_staging.len()
        || state.phases.len() != state.phase_snapshot_staging.len()
        || state.phases.len() != state.phase_states.len()
        || state.phases.iter().enumerate().any(|(index, entry)| {
            entry.state_position >= state.phase_states.len()
                || state.phases[..index]
                    .iter()
                    .any(|prior| prior.state_position == entry.state_position)
        })
    {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    Ok(())
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
        staging.1 = phase_snapshot(state.phase_states[entry.state_position]);
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
    LedgerSnapshot {
        dimensions: std::mem::take(&mut state.dimension_snapshot_staging),
        phases: std::mem::take(&mut state.phase_snapshot_staging),
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
        Unknown,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Progress {
        Work,
    }

    impl ProgressPhase for Progress {
        fn order(self) -> u32 {
            0
        }

        fn stable_id(self) -> &'static str {
            "work"
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

    #[test]
    fn frozen_phase_catalog_prebuilds_complete_final_evidence_shape() {
        let ledger = Ledger::<Owner, Phase, Dimension>::try_new_with_phase_catalog(
            limits(),
            &[Phase::Build, Phase::Finalize],
        )
        .expect("frozen phase catalog");

        let state = lock(&ledger.state);
        assert_eq!(state.phase_catalog, PhaseCatalogState::Frozen);
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
        let ledger = Ledger::<Owner, Phase, Dimension>::try_new_with_phase_catalog(
            limits(),
            &[Phase::Build, Phase::Finalize],
        )
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

    fn startup_limits(
        operation_live: u64,
        dimension_live: u64,
        attempted: u64,
        consumed: u64,
        published_evidence: u64,
    ) -> LedgerLimits<Dimension> {
        LedgerLimits::new(
            operation_live,
            [
                (
                    Dimension::Bytes,
                    DimensionPolicy::reservable_bytes(
                        attempted,
                        consumed,
                        u64::MAX,
                        dimension_live,
                    )
                    .with_published_outcome_evidence_limit(published_evidence),
                ),
                (
                    Dimension::Records,
                    DimensionPolicy::counter(attempted, consumed, u64::MAX)
                        .with_published_outcome_evidence_limit(published_evidence),
                ),
            ],
        )
        .expect("startup test policy")
    }

    fn startup_batch_plans(
        capacity_dimension: Dimension,
    ) -> StartupAttributionBatchPlans<Owner, Phase, Dimension> {
        StartupAttributionBatchPlans::new(
            AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 2, 1)
                .expect("canonical assembly plan"),
            AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 2, 2)
                .expect("evidence assembly plan"),
            AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 1, 1)
                .expect("fallback plan"),
            AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 1, 1)
                .expect("evidence plan"),
        )
    }

    fn startup_batch_bytes(batches: &StartupAttributionBatchPlans<Owner, Phase, Dimension>) -> u64 {
        [
            batches.canonical_assembly.planned_bytes,
            batches.evidence_assembly.planned_bytes,
            batches.fallback.planned_bytes,
            batches.evidence.planned_bytes,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .expect("test batch envelope")
    }

    fn startup_attributions(
        first_dimension: Dimension,
        second_dimension: Dimension,
    ) -> [StartupRetainedAttribution<Phase, Dimension>; 2] {
        [
            StartupRetainedAttribution::new(Phase::Build, first_dimension, 3),
            StartupRetainedAttribution::new(Phase::Build, second_dimension, 5),
        ]
    }

    fn startup_share_charges() -> [(Dimension, u64); 2] {
        [(Dimension::Records, 1), (Dimension::Records, 2)]
    }

    fn startup_account_shares() -> StartupAccountShareEnvelope<Dimension> {
        StartupAccountShareEnvelope::four_identical(startup_share_charges())
    }

    fn startup_commitments(
        first_dimension: Dimension,
        second_dimension: Dimension,
    ) -> [OutputCommitment<Dimension, OutcomeEvidenceDestination>; 2] {
        [
            OutputCommitment::new(first_dimension, 4),
            OutputCommitment::new(second_dimension, 6),
        ]
    }

    #[test]
    fn frozen_dimension_count_matches_the_canonical_dimension_vocabulary() {
        let limits = limits();

        assert_eq!(limits.dimension_count(), 2);
        assert!(limits.dimension(Dimension::Bytes).is_some());
        assert!(limits.dimension(Dimension::Records).is_some());
    }

    #[test]
    fn startup_qualification_preserves_the_exact_typed_envelope() {
        let batches = startup_batch_plans(Dimension::Bytes);
        let batch_bytes = startup_batch_bytes(&batches);
        let retained_attributions = startup_attributions(Dimension::Bytes, Dimension::Bytes);
        let total_live = batch_bytes.checked_add(8).expect("test live envelope");
        let limits = startup_limits(total_live, total_live, 12, 12, 10);

        let qualified = limits
            .try_qualify_startup(
                batches,
                retained_attributions,
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            )
            .expect("the exact startup envelope must qualify");
        let (limits, parts) = qualified.into_parts();

        assert_eq!(startup_batch_bytes(&parts.batches), batch_bytes);
        assert_eq!(parts.retained_attributions, retained_attributions);
        let (phase, dimension, bytes) = parts.retained_attributions[0].into_parts();
        assert_eq!(
            (phase, dimension, bytes),
            (Phase::Build, Dimension::Bytes, 3)
        );
        let (_canonical, _evidence_assembly, _fallback, _evidence) = parts.batches.into_parts();
        for share in parts.account_shares.into_steps() {
            assert_eq!(share.into_charges(), startup_share_charges());
        }
        assert_eq!(parts.commitments[0].dimension(), Dimension::Records);
        assert_eq!(parts.commitments[0].amount(), 4);
        assert_eq!(parts.commitments[1].dimension(), Dimension::Records);
        assert_eq!(parts.commitments[1].amount(), 6);
        assert_eq!(limits.dimension_count(), 2);
    }

    #[test]
    fn startup_live_and_observed_work_are_independent_on_the_same_dimension() {
        let batches = startup_batch_plans(Dimension::Bytes);
        let total_live = startup_batch_bytes(&batches)
            .checked_add(8)
            .expect("test live envelope");
        let limits = startup_limits(total_live, total_live, 12, 12, 10);
        let account_shares = StartupAccountShareEnvelope::four_identical([
            (Dimension::Bytes, 1),
            (Dimension::Bytes, 2),
        ]);

        assert!(
            limits
                .try_qualify_startup(
                    batches,
                    startup_attributions(Dimension::Bytes, Dimension::Bytes),
                    account_shares,
                    startup_commitments(Dimension::Bytes, Dimension::Bytes),
                )
                .is_ok(),
            "live reservations and observed work use independent ledger counter classes"
        );
    }

    #[test]
    fn startup_qualification_rejects_unknown_dimensions_without_allocation() {
        let allocations_before = test_attribution_batch_allocation_entries();

        assert!(matches!(
            startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
                startup_batch_plans(Dimension::Unknown),
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::UnknownDimension {
                dimension: Dimension::Unknown,
            })
        ));
        assert!(matches!(
            startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
                startup_batch_plans(Dimension::Bytes),
                startup_attributions(Dimension::Unknown, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::UnknownDimension {
                dimension: Dimension::Unknown,
            })
        ));
        assert!(matches!(
            startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
                startup_batch_plans(Dimension::Bytes),
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                StartupAccountShareEnvelope::four_identical([
                    (Dimension::Unknown, 1),
                    (Dimension::Records, 2),
                ]),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::UnknownDimension {
                dimension: Dimension::Unknown,
            })
        ));
        assert!(matches!(
            startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
                startup_batch_plans(Dimension::Bytes),
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Unknown, Dimension::Records),
            ),
            Err(LedgerCause::UnknownDimension {
                dimension: Dimension::Unknown,
            })
        ));

        assert_eq!(
            test_attribution_batch_allocation_entries(),
            allocations_before,
            "qualification rejection must not allocate a batch"
        );
    }

    #[test]
    fn startup_qualification_checks_live_dimension_and_operation_limits() {
        let batches = startup_batch_plans(Dimension::Bytes);
        let total_live = startup_batch_bytes(&batches)
            .checked_add(8)
            .expect("test live envelope");
        let dimension_limited = startup_limits(
            u64::MAX,
            total_live.checked_sub(1).expect("nonzero envelope"),
            12,
            12,
            10,
        );
        assert!(matches!(
            dimension_limited.try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
                value,
                limit,
            }) if value == total_live && limit == total_live - 1
        ));

        let batches = startup_batch_plans(Dimension::Bytes);
        let operation_limited = startup_limits(
            total_live.checked_sub(1).expect("nonzero envelope"),
            total_live,
            12,
            12,
            10,
        );
        assert!(matches!(
            operation_limited.try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
                value,
                limit,
            }) if value == total_live && limit == total_live - 1
        ));

        let non_reservable = startup_limits(u64::MAX, u64::MAX, 12, 12, 10);
        assert!(matches!(
            non_reservable.try_qualify_startup(
                startup_batch_plans(Dimension::Records),
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::DimensionNotReservable {
                dimension: Dimension::Records,
            })
        ));

        let live_overflow = startup_limits(u64::MAX, u64::MAX, 12, 12, 10);
        assert!(matches!(
            live_overflow.try_qualify_startup(
                startup_batch_plans(Dimension::Bytes),
                [
                    StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, u64::MAX,),
                    StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, 0),
                ],
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension: Dimension::Bytes,
            })
        ));
    }

    #[test]
    fn startup_qualification_aggregates_four_account_shares() {
        let batches = startup_batch_plans(Dimension::Bytes);
        let total_live = startup_batch_bytes(&batches)
            .checked_add(8)
            .expect("test live envelope");
        let attempted_limited = startup_limits(total_live, total_live, 11, 11, 10);
        assert!(matches!(
            attempted_limited.try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: Dimension::Records,
                value: 12,
                limit: 11,
            })
        ));

        let batches = startup_batch_plans(Dimension::Bytes);
        let consumed_limited = startup_limits(total_live, total_live, 12, 11, 10);
        assert!(matches!(
            consumed_limited.try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: Dimension::Records,
                value: 12,
                limit: 11,
            })
        ));

        let batches = startup_batch_plans(Dimension::Bytes);
        let share_overflow = startup_limits(total_live, total_live, u64::MAX, u64::MAX, 10);
        assert!(matches!(
            share_overflow.try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                StartupAccountShareEnvelope::four_identical([
                    (Dimension::Records, u64::MAX),
                    (Dimension::Records, 0),
                ]),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Attempted,
                dimension: Dimension::Records,
            })
        ));
    }

    #[test]
    fn startup_qualification_checked_sums_outcome_evidence_commitments() {
        let batches = startup_batch_plans(Dimension::Bytes);
        let total_live = startup_batch_bytes(&batches)
            .checked_add(8)
            .expect("test live envelope");
        let evidence_limited = startup_limits(total_live, total_live, 12, 12, 9);
        assert!(matches!(
            evidence_limited.try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                startup_commitments(Dimension::Records, Dimension::Records),
            ),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::PublishedOutcomeEvidence,
                dimension: Dimension::Records,
                value: 10,
                limit: 9,
            })
        ));

        let batches = startup_batch_plans(Dimension::Bytes);
        let overflow_limits = startup_limits(total_live, total_live, 12, 12, u64::MAX);
        assert!(matches!(
            overflow_limits.try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                startup_account_shares(),
                [
                    OutputCommitment::new(Dimension::Records, u64::MAX),
                    OutputCommitment::new(Dimension::Records, 1),
                ],
            ),
            Err(LedgerCause::ArithmeticOverflow {
                class: CounterClass::PublishedOutcomeEvidence,
                dimension: Dimension::Records,
            })
        ));
    }

    #[test]
    fn consumed_batch_is_atomic_and_aggregates_duplicate_dimensions() {
        let ledger = Ledger::<Owner, Phase, Dimension>::new(limits());
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
        let ledger = Ledger::<Owner, Phase, Dimension>::new(limits());
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
        let ledger = Ledger::<Owner, Phase, Dimension>::new(limits());
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
    fn injected_progress_allocation_failure_is_atomic_and_latched() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let limits = limits();
        let frozen_evidence_bytes = limits.frozen_evidence_bytes();
        let mut coordinator = OperationCoordinator::<Owner, Phase, Progress, Unit, Dimension>::new(
            OperationControl::default(),
            limits,
        );

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
    fn finalization_never_enters_the_evidence_allocator() {
        let _guard = EVIDENCE_ALLOCATION_TEST_LOCK
            .lock()
            .expect("allocation test lock");
        let ledger = Ledger::<Owner, Phase, Dimension>::new(limits());
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

        let ledger = Ledger::<Owner, Phase, Dimension>::new(limits());
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

    fn probe_values(probe: &LedgerTestProbe<Phase, Dimension>) -> (u64, u64) {
        (probe.active_reservations(), probe.current_live_bytes())
    }

    #[test]
    fn typed_foreign_attribution_push_returns_the_token_without_mutating_either_ledger() {
        let target = Ledger::<Owner, Phase, Dimension>::new(typed_limits());
        let foreign = Ledger::<Owner, Phase, Dimension>::new(typed_limits());
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
    fn typed_foreign_fallback_batch_rejects_the_baseline_without_mutation() {
        let target = Ledger::<Owner, Phase, Dimension>::new(typed_limits());
        let foreign = Ledger::<Owner, Phase, Dimension>::new(typed_limits());
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
    fn typed_foreign_primary_source_rejects_without_mutation_then_discards_safely() {
        let target = Ledger::<Owner, Phase, Dimension>::new(typed_limits());
        let foreign = Ledger::<Owner, Phase, Dimension>::new(typed_limits());
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
            .try_bind_primary(&mut baseline, foreign_source.as_mut(), None)
            .expect_err("foreign primary source must be rejected");

        assert!(matches!(rejection, LedgerCause::OwnerMismatch));
        assert_eq!(probe_values(&target_probe), target_before);
        assert_eq!(probe_values(&foreign_probe), foreign_before);
        let snapshot = baseline
            .into_fallback_proof(target, foreign_source, None)
            .freeze();
        assert_eq!(snapshot.active_reservations(), 0);
        assert_eq!(snapshot.current_live_bytes(), 0);
        assert_eq!(probe_values(&target_probe), (0, 0));
        assert_eq!(probe_values(&foreign_probe), (0, 0));
    }

    static PANIC_ON_BOUND_ORD: AtomicBool = AtomicBool::new(false);
    static BOUND_ORD_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct StatefulOrd(u8);

    impl PartialOrd for StatefulOrd {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for StatefulOrd {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            assert!(
                !PANIC_ON_BOUND_ORD.load(Ordering::SeqCst),
                "typed freeze called owner-defined Ord after positions were bound"
            );
            self.0.cmp(&other.0)
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

    fn freeze_while_ord_panics(
        proof: TypedFreezeProof<Owner, StatefulOrd, StatefulOrd>,
    ) -> LedgerSnapshot<StatefulOrd, StatefulOrd> {
        PANIC_ON_BOUND_ORD.store(true, Ordering::SeqCst);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| proof.freeze()));
        PANIC_ON_BOUND_ORD.store(false, Ordering::SeqCst);
        result.expect("bound typed freeze must not call owner-defined Ord")
    }

    #[test]
    fn fallback_baseline_counts_zero_byte_attribution_capabilities() {
        let ledger = Ledger::<Owner, Phase, Dimension>::new(typed_limits());
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
        let ledger = Ledger::<Owner, Phase, Dimension>::new(typed_limits());
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
        let snapshot = baseline.into_fallback_proof(ledger, None, None).freeze();
        assert_eq!(snapshot.current_live_bytes(), 0);
        assert_eq!(snapshot.active_reservations(), 0);
    }

    #[test]
    fn fallback_freeze_uses_stable_positions_for_attribution_and_capacity_cover() {
        let _guard = BOUND_ORD_TEST_LOCK.lock().expect("bound Ord test lock");
        let terminal_phase = StatefulOrd(2);
        let dimension = StatefulOrd(7);
        let ledger = Ledger::<Owner, StatefulOrd, StatefulOrd>::new(stateful_ord_limits());
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
            .expect("later phase insertion");
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
        let ledger = Ledger::<Owner, StatefulOrd, StatefulOrd>::new(stateful_ord_limits());
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
            .expect("later phase insertion");
        drop(earlier);
        let mut source = ledger
            .try_attribution_batch::<CanonicalOutput>(terminal_phase, dimension, 1, 1)
            .expect("covered canonical source");
        let attribution = ledger
            .reserve_operation_attribution::<CanonicalOutput>(terminal_phase, dimension, 5)
            .expect("canonical attribution");
        assert!(source.try_push_attribution(attribution).is_ok());
        assert!(
            source
                .try_push_commitment(OutputCommitment::new(dimension, 3))
                .is_ok()
        );
        let mut baseline = baseline;
        let mut source = Some(source);
        ledger
            .try_bind_primary(&mut baseline, source.as_mut(), None)
            .unwrap_or_else(|_| panic!("exact primary publication must bind"));
        let proof = ledger.finish_bound_primary(baseline, source, None);
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
    fn typed_binding_preflights_every_batch_before_the_infallible_mutation_tail() {
        let source = include_str!("ledger.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();
        let (production, _) = source
            .rsplit_once(&test_attribute)
            .expect("ledger source contains one final test module boundary");
        let primary = production
            .split("pub(crate) fn try_bind_primary(")
            .nth(1)
            .expect("typed primary binding exists")
            .split("pub(crate) fn finish_bound_primary(")
            .next()
            .expect("typed primary binding boundary exists");
        assert_eq!(
            primary
                .matches("preflight_typed_publication_batch(")
                .count(),
            5
        );
        assert_eq!(
            primary
                .matches("bind_preflighted_typed_publication_batch(")
                .count(),
            5
        );
        assert!(
            primary
                .rfind("preflight_typed_publication_batch(")
                .expect("last primary preflight")
                < primary
                    .find("bind_preflighted_typed_publication_batch(")
                    .expect("first primary mutation")
        );

        let fallback = production
            .split("fn bind_fallback_baseline(")
            .nth(1)
            .expect("fallback binding exists")
            .split("fn validate_fallback_baseline(")
            .next()
            .expect("fallback binding boundary exists");
        assert_eq!(
            fallback
                .matches("preflight_typed_publication_batch(")
                .count(),
            4
        );
        assert_eq!(
            fallback
                .matches("bind_preflighted_typed_publication_batch(")
                .count(),
            4
        );
        assert!(
            fallback
                .rfind("preflight_typed_publication_batch(")
                .expect("last fallback preflight")
                < fallback
                    .find("bind_preflighted_typed_publication_batch(")
                    .expect("first fallback mutation")
        );

        let mutation = production
            .split("fn bind_preflighted_typed_publication_batch")
            .nth(1)
            .expect("infallible binding helper exists")
            .split("fn apply_typed_commitments")
            .next()
            .expect("infallible binding helper boundary exists");
        assert!(!mutation.contains("Result<"));
        assert!(!mutation.contains('?'));
    }

    #[test]
    fn typed_freeze_proof_owns_exact_batches_and_has_no_post_mutation_fault_gate() {
        let source = include_str!("ledger.rs");
        let test_attribute = ["#[cfg(", "test)]"].concat();
        let (production, _) = source
            .rsplit_once(&test_attribute)
            .expect("ledger source contains one final test module boundary");
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

        let freeze = production
            .split("pub(crate) fn freeze(mut self)")
            .nth(1)
            .expect("typed freeze exists")
            .split("\n}\n\n#[allow(")
            .next()
            .expect("typed freeze body exists");
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
                !freeze.contains(forbidden),
                "freeze contains post-mutation fault gate `{forbidden}`"
            );
        }
        assert_eq!(freeze.matches("lock(").count(), 1);
        let release = freeze
            .find("release_bound_attributions")
            .expect("freeze releases bound attributions");
        let discard_canonical = freeze
            .find("drop(self.discard_canonical.take())")
            .expect("freeze drops discarded canonical batch");
        let discard_evidence = freeze
            .find("drop(self.discard_evidence.take())")
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
            .split("fn try_snapshot")
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
