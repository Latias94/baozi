use crate::{CapacityPlan, CapacityPlanError, OperationOwner};
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
static FAIL_NEXT_EVIDENCE_ALLOCATION: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
static EVIDENCE_ALLOCATION_TEST_LOCK: Mutex<()> = Mutex::new(());

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
    live_byte_limit: Option<u64>,
}

impl DimensionPolicy {
    /// Defines a counter-only dimension that cannot reserve byte capacity.
    pub const fn counter(attempted_limit: u64, consumed_limit: u64, committed_limit: u64) -> Self {
        Self {
            attempted_limit,
            consumed_limit,
            committed_limit,
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
            live_byte_limit: Some(live_byte_limit),
        }
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
}

impl<D: fmt::Debug> fmt::Display for LedgerCause<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDimension { dimension } => {
                write!(formatter, "unknown ledger dimension {dimension:?}")
            }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DimensionState {
    attempted: u64,
    consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct PhaseState {
    current_live_bytes: u64,
    peak_live_bytes: u64,
}

#[derive(Debug)]
struct PhaseEntry<P> {
    phase: P,
    state: PhaseState,
}

#[derive(Debug)]
struct LedgerState<P, D> {
    dimensions: Vec<DimensionEntry<D>>,
    phases: Vec<PhaseEntry<P>>,
    dimension_snapshot_staging: Vec<(D, DimensionSnapshot)>,
    phase_snapshot_staging: Vec<(P, PhaseSnapshot)>,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    evidence_byte_limit: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
    publication: PublicationState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationState {
    Open,
    Pending,
    Published,
}

/// Shared accounting state branded by one operation domain.
pub(crate) struct Ledger<O, P, D> {
    state: Arc<Mutex<LedgerState<P, D>>>,
    operation_live_byte_limit: u64,
    operation_owner: PhantomData<fn() -> O>,
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
                dimension_snapshot_staging,
                phase_snapshot_staging: Vec::new(),
                current_live_bytes: 0,
                peak_live_bytes: 0,
                active_reservations: 0,
                evidence_byte_limit,
                current_evidence_bytes,
                peak_evidence_bytes,
                publication: PublicationState::Open,
            })),
            operation_live_byte_limit,
            operation_owner: PhantomData,
        }
    }

    /// Charges non-refundable attempted work before an action.
    pub fn charge_attempted(&self, dimension: D, amount: u64) -> Result<(), LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
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

    /// Reserves byte capacity before allocation or backend entry.
    pub fn reserve(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        let dimension_entry = dimension_entry(&state.dimensions, dimension)
            .ok_or(LedgerCause::UnknownDimension { dimension })?;
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
        let current_phase =
            phase_entry(&state.phases, phase).map_or(0, |entry| entry.state.current_live_bytes);
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

        let phase_position =
            ensure_phase(&mut state, phase).map_err(|cause| LedgerCause::Evidence { cause })?;
        let dimension_state = &mut dimension_entry_mut(&mut state.dimensions, dimension)
            .expect("validated dimensions remain present")
            .state;
        dimension_state.current_live_bytes = dimension_live;
        dimension_state.peak_live_bytes = dimension_state.peak_live_bytes.max(dimension_live);
        let phase_state = &mut state.phases[phase_position].state;
        phase_state.current_live_bytes = phase_live;
        phase_state.peak_live_bytes = phase_state.peak_live_bytes.max(phase_live);
        state.current_live_bytes = operation_live;
        state.peak_live_bytes = state.peak_live_bytes.max(operation_live);
        state.active_reservations = active_reservations;

        Ok(LiveReservation {
            owner: Arc::clone(&self.state),
            phase,
            dimension,
            bytes,
            active: true,
            operation_owner: PhantomData,
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
        if reservation.phase == phase {
            return Ok(());
        }
        let target_live = phase_entry(&state.phases, phase)
            .map_or(0, |entry| entry.state.current_live_bytes)
            .checked_add(reservation.bytes)
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension: reservation.dimension,
            })?;
        let phase_position =
            ensure_phase(&mut state, phase).map_err(|cause| LedgerCause::Evidence { cause })?;
        release_phase(&mut state, reservation.phase, reservation.bytes);
        let target = &mut state.phases[phase_position].state;
        target.current_live_bytes = target_live;
        target.peak_live_bytes = target.peak_live_bytes.max(target_live);
        reservation.phase = phase;
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

        let phase_live = project(
            phase_entry(&state.phases, phase).map_or(0, |entry| entry.state.current_live_bytes),
        )?;

        let phase_position =
            ensure_phase(&mut state, phase).map_err(|cause| LedgerCause::Evidence { cause })?;
        let dimension_state = &mut dimension_entry_mut(&mut state.dimensions, dimension)
            .expect("validated dimensions remain present")
            .state;
        dimension_state.current_live_bytes = dimension_live;
        dimension_state.peak_live_bytes = dimension_state.peak_live_bytes.max(dimension_live);
        let phase_state = &mut state.phases[phase_position].state;
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
                .checked_add(reservation.bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension,
                })
        })?;
        let target_dimension_source = reservations
            .iter()
            .filter(|reservation| reservation.dimension == dimension)
            .try_fold(0_u64, |total, reservation| {
                total
                    .checked_add(reservation.bytes)
                    .ok_or(LedgerCause::ArithmeticOverflow {
                        class: CounterClass::Live,
                        dimension,
                    })
            })?;
        let target_phase_source = reservations
            .iter()
            .filter(|reservation| reservation.phase == phase)
            .try_fold(0_u64, |total, reservation| {
                total
                    .checked_add(reservation.bytes)
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
            phase_entry(&state.phases, phase).map_or(0, |entry| entry.state.current_live_bytes),
            target_phase_source,
        )?;
        let phase_position =
            ensure_phase(&mut state, phase).map_err(|cause| LedgerCause::Evidence { cause })?;

        for reservation in reservations.iter() {
            release_bytes_locked(
                &mut state,
                reservation.phase,
                reservation.dimension,
                reservation.bytes,
            );
        }
        let dimension_state = &mut dimension_entry_mut(&mut state.dimensions, dimension)
            .expect("validated dimensions remain present")
            .state;
        dimension_state.current_live_bytes = dimension_live;
        dimension_state.peak_live_bytes = dimension_state.peak_live_bytes.max(dimension_live);
        let phase_state = &mut state.phases[phase_position].state;
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
                reservation.bytes.min(remaining)
            };
            remaining -= target_bytes;
            reservation.phase = phase;
            reservation.dimension = dimension;
            reservation.bytes = target_bytes;
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
        if bytes > reservation.bytes {
            return Err(LedgerCause::SplitExceedsReservation {
                requested: bytes,
                available: reservation.bytes,
            });
        }
        state.active_reservations =
            state
                .active_reservations
                .checked_add(1)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: reservation.dimension,
                })?;
        drop(state);

        reservation.bytes -= bytes;
        Ok(LiveReservation {
            owner: Arc::clone(&reservation.owner),
            phase: reservation.phase,
            dimension: reservation.dimension,
            bytes,
            active: true,
            operation_owner: PhantomData,
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

    /// Copies one phase's scalar evidence without allocating a full snapshot.
    pub fn phase_evidence(&self, phase: P) -> Option<PhaseSnapshot> {
        let state = lock(&self.state);
        phase_entry(&state.phases, phase).map(|entry| phase_snapshot(entry.state))
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
        if !Arc::ptr_eq(&self.state, &reservation.owner) {
            return Err(LedgerCause::OwnerMismatch);
        }
        if !reservation.active {
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
                .checked_add(reservation.bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: reservation.dimension,
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
    owner: Arc<Mutex<LedgerState<P, D>>>,
    phase: P,
    dimension: D,
    bytes: u64,
    active: bool,
    operation_owner: PhantomData<fn() -> O>,
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
            .field("phase", &self.phase)
            .field("dimension", &self.dimension)
            .field("bytes", &self.bytes)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl<O, P, D> Drop for LiveReservation<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        self.release();
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
        self.bytes
    }

    fn release(&mut self) {
        if !self.active {
            return;
        }
        release(&self.owner, self.phase, self.dimension, self.bytes);
        self.active = false;
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
            release_locked(
                state,
                reservation.phase,
                reservation.dimension,
                reservation.bytes,
            );
            reservation.active = false;
        }

        // Free the accounted vector allocation before releasing the token that covers it.
        drop(std::mem::take(&mut self.reservations));
        if let Some(capacity_reservation) = &mut self.capacity_reservation {
            release_locked(
                state,
                capacity_reservation.phase,
                capacity_reservation.dimension,
                capacity_reservation.bytes,
            );
            capacity_reservation.active = false;
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

fn release<P, D>(owner: &Arc<Mutex<LedgerState<P, D>>>, phase: P, dimension: D, bytes: u64)
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    let mut state = lock(owner);
    release_locked(&mut state, phase, dimension, bytes);
}

fn release_locked<P, D>(state: &mut LedgerState<P, D>, phase: P, dimension: D, bytes: u64)
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    state.active_reservations = state.active_reservations.saturating_sub(1);
    release_bytes_locked(state, phase, dimension, bytes);
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
    if let Some(entry) = phase_entry_mut(&mut state.phases, phase) {
        entry.state.current_live_bytes = entry.state.current_live_bytes.saturating_sub(bytes);
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
    Ok(())
}

fn phase_entry<P: Copy + Ord>(phases: &[PhaseEntry<P>], phase: P) -> Option<&PhaseEntry<P>> {
    phases
        .binary_search_by_key(&phase, |entry| entry.phase)
        .ok()
        .map(|position| &phases[position])
}

fn phase_entry_mut<P: Copy + Ord>(
    phases: &mut [PhaseEntry<P>],
    phase: P,
) -> Option<&mut PhaseEntry<P>> {
    phases
        .binary_search_by_key(&phase, |entry| entry.phase)
        .ok()
        .map(|position| &mut phases[position])
}

fn ensure_phase<P: Copy + Ord, D>(
    state: &mut LedgerState<P, D>,
    phase: P,
) -> Result<usize, LedgerEvidenceError> {
    let position = match state
        .phases
        .binary_search_by_key(&phase, |entry| entry.phase)
    {
        Ok(position) => return Ok(position),
        Err(position) => position,
    };
    let desired_phase_capacity = state.phases.len().saturating_add(1);
    try_grow_evidence_vec(
        &mut state.phases,
        desired_phase_capacity,
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Phases,
    )?;
    let desired_staging_capacity = state.phase_snapshot_staging.len().saturating_add(1);
    try_grow_evidence_vec(
        &mut state.phase_snapshot_staging,
        desired_staging_capacity,
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Phases,
    )?;
    state.phases.insert(
        position,
        PhaseEntry {
            phase,
            state: PhaseState::default(),
        },
    );
    state
        .phase_snapshot_staging
        .insert(position, (phase, PhaseSnapshot::default()));
    Ok(position)
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
    if FAIL_NEXT_EVIDENCE_ALLOCATION.swap(false, Ordering::SeqCst) {
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
        phases.push((entry.phase, phase_snapshot(entry.state)));
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

fn take_final_snapshot<P, D>(state: &mut LedgerState<P, D>, clean: bool) -> LedgerSnapshot<P, D>
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    debug_assert_eq!(
        state.dimensions.len(),
        state.dimension_snapshot_staging.len()
    );
    debug_assert_eq!(state.phases.len(), state.phase_snapshot_staging.len());
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
        staging.1 = phase_snapshot(entry.state);
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

const fn dimension_snapshot(state: DimensionState) -> DimensionSnapshot {
    DimensionSnapshot {
        attempted: state.attempted,
        consumed: state.consumed,
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
        pending_publication: state.pending_publication,
        committed: state.committed,
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
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Dimension {
        Bytes,
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
            [(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(16, 16, 16, 16),
            )],
        )
        .expect("test policy")
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
        FAIL_NEXT_EVIDENCE_ALLOCATION.store(true, Ordering::SeqCst);
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

        FAIL_NEXT_EVIDENCE_ALLOCATION.store(true, Ordering::SeqCst);
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
        FAIL_NEXT_EVIDENCE_ALLOCATION.store(true, Ordering::SeqCst);
        let evidence = ledger
            .dimension_evidence(Dimension::Bytes)
            .expect("scalar evidence is allocation-free");
        assert_eq!(evidence.current_live_bytes(), 0);
        assert!(FAIL_NEXT_EVIDENCE_ALLOCATION.load(Ordering::SeqCst));
        let snapshot = ledger.finish().expect("clean finalization");
        assert_eq!(snapshot.current_evidence_bytes(), 0);
        assert!(FAIL_NEXT_EVIDENCE_ALLOCATION.load(Ordering::SeqCst));
        FAIL_NEXT_EVIDENCE_ALLOCATION.store(false, Ordering::SeqCst);

        let ledger = Ledger::<Owner, Phase, Dimension>::new(limits());
        let reservation = ledger
            .reserve(Phase::Build, Dimension::Bytes, 1)
            .expect("retained reservation");
        FAIL_NEXT_EVIDENCE_ALLOCATION.store(true, Ordering::SeqCst);
        let error = ledger
            .finish()
            .expect_err("live state is an invariant failure");
        assert_eq!(error.snapshot().current_live_bytes(), 1);
        assert!(error.snapshot().current_evidence_bytes() > 0);
        assert!(FAIL_NEXT_EVIDENCE_ALLOCATION.load(Ordering::SeqCst));
        FAIL_NEXT_EVIDENCE_ALLOCATION.store(false, Ordering::SeqCst);
        drop(reservation);
    }
}
