use super::{
    AttributionBatchCreateError, AttributionBatchPlan, AttributionReservationBatch,
    CanonicalOutput, CounterClass, FallbackBatchBaseline, Ledger, LedgerCause, LedgerLimits,
    LedgerState, OperationAttributionReservation, OutcomeEvidenceDestination, OutputCommitment,
    PublicationDestination, ReservationCore, StorageCoveringReservation, dimension_entry, lock,
    phase_entry,
};
use crate::retained::ReadyStartupGraphSplitAuthority;
use crate::{CapacityPlan, OperationOwner};
use std::fmt;
use std::marker::PhantomData;
use std::mem;
use std::sync::{Arc, Mutex};

#[cfg(test)]
std::thread_local! {
    static STARTUP_GRAPH_ALLOCATION_ENTRIES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static FAIL_STARTUP_GRAPH_ALLOCATION_AT: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
    static STARTUP_GRAPH_STORAGE_RELEASES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static STARTUP_GRAPH_LEDGER_RELEASE_LOCKS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static STARTUP_GRAPH_STORAGE_RELEASES_AT_LEDGER_LOCK: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
fn test_reset_startup_graph_probes(fail_at: Option<usize>) {
    STARTUP_GRAPH_ALLOCATION_ENTRIES.with(|entries| entries.set(0));
    FAIL_STARTUP_GRAPH_ALLOCATION_AT.with(|failpoint| failpoint.set(fail_at));
    STARTUP_GRAPH_STORAGE_RELEASES.with(|releases| releases.set(0));
    STARTUP_GRAPH_LEDGER_RELEASE_LOCKS.with(|locks| locks.set(0));
    STARTUP_GRAPH_STORAGE_RELEASES_AT_LEDGER_LOCK.with(|releases| releases.set(None));
}

#[cfg(test)]
pub(crate) fn test_startup_graph_allocation_entries() -> usize {
    STARTUP_GRAPH_ALLOCATION_ENTRIES.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn test_startup_graph_storage_releases() -> usize {
    STARTUP_GRAPH_STORAGE_RELEASES.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn test_startup_graph_ledger_release_locks() -> usize {
    STARTUP_GRAPH_LEDGER_RELEASE_LOCKS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn test_startup_graph_storage_releases_at_ledger_lock() -> Option<usize> {
    STARTUP_GRAPH_STORAGE_RELEASES_AT_LEDGER_LOCK.with(std::cell::Cell::get)
}

const STARTUP_ACCOUNT_SHARE_STEP_COUNT: usize = 4;

/// One retained allocation attribution that startup must be able to reserve.
///
/// The phase remains part of the qualified value even though the current frozen policy has no
/// phase-specific limit. That keeps startup qualification ready for a future phase-qualified
/// `LedgerLimits<P, D>` without weakening today's dimension and operation-wide checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartupRetainedAttribution<P, D> {
    phase: P,
    dimension: D,
    bytes: u64,
}

impl<P, D> StartupRetainedAttribution<P, D> {
    pub(crate) const fn new(phase: P, dimension: D, bytes: u64) -> Self {
        Self {
            phase,
            dimension,
            bytes,
        }
    }
}

/// One linear attempted-and-consumed work step paired with one startup account share.
pub(crate) struct StartupAccountShareStep<D> {
    charges: [(D, u64); 2],
}

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
pub(crate) struct StartupAccountShareEnvelope<D> {
    steps: [StartupAccountShareStep<D>; STARTUP_ACCOUNT_SHARE_STEP_COUNT],
}

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
}

/// Consuming output of one successful startup-ledger qualification.
///
/// This remains crate-private: outside callers cannot inspect or synthesize startup admission
/// evidence, while the retained owner can consume the exact typed inputs without reconstructing
/// them after crossing the side-effect boundary.
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

impl<O, P, D> StartupLedgerPlan<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) const fn retained_bytes(&self) -> [u64; 2] {
        [
            self.parts.retained_attributions[0].bytes,
            self.parts.retained_attributions[1].bytes,
        ]
    }

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

const STARTUP_RESERVATION_COUNT: u64 = 6;
const STARTUP_WORK_CHARGE_COUNT: usize = STARTUP_ACCOUNT_SHARE_STEP_COUNT * 2;

#[derive(Clone, Copy)]
struct StartupReservationSpec<P, D> {
    phase: P,
    dimension: D,
    bytes: u64,
}

impl<P: Copy, D: Copy> StartupReservationSpec<P, D> {
    const fn for_batch<O, Destination>(plan: &AttributionBatchPlan<O, P, D, Destination>) -> Self
    where
        O: OperationOwner,
        P: Ord,
        D: Ord,
        Destination: PublicationDestination,
    {
        Self {
            phase: plan.phase,
            dimension: plan.capacity_dimension,
            bytes: plan.planned_bytes,
        }
    }

    const fn for_retained(attribution: StartupRetainedAttribution<P, D>) -> Self {
        Self {
            phase: attribution.phase,
            dimension: attribution.dimension,
            bytes: attribution.bytes,
        }
    }
}

#[derive(Clone, Copy)]
struct StartupReservationProjection<P, D> {
    spec: StartupReservationSpec<P, D>,
    phase_position: usize,
    dimension_position: usize,
}

#[derive(Clone, Copy)]
struct StartupWorkProjection<D> {
    dimension: D,
    dimension_position: usize,
    amount: u64,
}

struct StartupLedgerAdmission<P, D> {
    reservations: [StartupReservationProjection<P, D>; STARTUP_RESERVATION_COUNT as usize],
    work: [StartupWorkProjection<D>; STARTUP_WORK_CHARGE_COUNT],
    commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
    operation_live_after: u64,
    active_reservations_after: u64,
}

struct StartupLedgerCores<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical_assembly_cover: ReservationCore<O, P, D>,
    evidence_assembly_cover: ReservationCore<O, P, D>,
    fallback_cover: ReservationCore<O, P, D>,
    evidence_cover: ReservationCore<O, P, D>,
    fallback_attribution: ReservationCore<O, P, D>,
    evidence_attribution: ReservationCore<O, P, D>,
}

struct StartupLedgerCommit<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    cores: StartupLedgerCores<O, P, D>,
    commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
}

impl<O, P, D> StartupLedgerCores<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn from_array(cores: [ReservationCore<O, P, D>; STARTUP_RESERVATION_COUNT as usize]) -> Self {
        let [
            canonical_assembly_cover,
            evidence_assembly_cover,
            fallback_cover,
            evidence_cover,
            fallback_attribution,
            evidence_attribution,
        ] = cores;
        Self {
            canonical_assembly_cover,
            evidence_assembly_cover,
            fallback_cover,
            evidence_cover,
            fallback_attribution,
            evidence_attribution,
        }
    }

    fn release_locked(&mut self, state: &mut LedgerState<P, D>) {
        self.canonical_assembly_cover.release_locked(state);
        self.evidence_assembly_cover.release_locked(state);
        self.fallback_cover.release_locked(state);
        self.evidence_cover.release_locked(state);
        self.fallback_attribution.release_locked(state);
        self.evidence_attribution.release_locked(state);
    }
}

#[derive(Clone, Copy)]
struct StartupBatchAllocationPlans {
    attributions: CapacityPlan,
    commitments: CapacityPlan,
    planned_bytes: u64,
}

struct StartupAttributionBatchBackings<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    attributions: Vec<OperationAttributionReservation<O, P, D, Destination>>,
    commitments: Vec<OutputCommitment<D, Destination>>,
    attribution_limit: usize,
    commitment_limit: usize,
}

fn into_runtime_batch<O, P, D, Destination>(
    backing: StartupAttributionBatchBackings<O, P, D, Destination>,
    capacity_core: ReservationCore<O, P, D>,
    owner: Arc<Mutex<LedgerState<P, D>>>,
) -> AttributionReservationBatch<O, P, D, Destination>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    let StartupAttributionBatchBackings {
        attributions,
        commitments,
        attribution_limit,
        commitment_limit,
    } = backing;
    AttributionReservationBatch {
        attributions,
        commitments,
        capacity_cover: Some(StorageCoveringReservation {
            core: capacity_core,
        }),
        attribution_limit,
        commitment_limit,
        owner,
    }
}

fn drop_startup_graph_ownership<O, P, D>(mut ownership: StartupAttributionGraphOwnership<O, P, D>)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    drop_startup_batch_backings(&mut ownership.canonical_assembly);
    drop_startup_batch_backings(&mut ownership.evidence_assembly);
    drop_startup_batch_backings(&mut ownership.fallback);
    drop_startup_batch_backings(&mut ownership.evidence);

    let mut state = lock(&ownership.runtime_owners[0]);
    test_note_startup_graph_ledger_release_lock();
    ownership.cores.release_locked(&mut state);
}

/// Ready-owned ledger half of the complete cross-plane startup attribution graph.
///
/// The cross-plane transition consumes this value into the production retained baseline only after
/// physical admission commits. Before that split, its destructor owns the rollback path: all eight
/// vector backings are destroyed before one ledger lock releases the four capacity covers and two
/// retained attributions.
#[must_use = "dropping the startup graph releases all live startup reservations atomically"]
pub(crate) struct StartupAttributionGraph<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ownership: Option<StartupAttributionGraphOwnership<O, P, D>>,
}

struct StartupAttributionGraphOwnership<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    runtime_owners: [Arc<Mutex<LedgerState<P, D>>>; 4],
    canonical_assembly: StartupAttributionBatchBackings<O, P, D, CanonicalOutput>,
    evidence_assembly: StartupAttributionBatchBackings<O, P, D, OutcomeEvidenceDestination>,
    fallback: StartupAttributionBatchBackings<O, P, D, OutcomeEvidenceDestination>,
    evidence: StartupAttributionBatchBackings<O, P, D, OutcomeEvidenceDestination>,
    cores: StartupLedgerCores<O, P, D>,
    _account_shares: StartupAccountShareEnvelope<D>,
}

impl<O, P, D> StartupAttributionGraph<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn try_allocate_backings(
        &mut self,
        canonical_assembly: StartupBatchAllocationPlans,
        evidence_assembly: StartupBatchAllocationPlans,
        fallback: StartupBatchAllocationPlans,
        evidence: StartupBatchAllocationPlans,
    ) -> Result<(), AttributionBatchCreateError<D>> {
        let Some(ownership) = self.ownership.as_mut() else {
            return Err(AttributionBatchCreateError::CapacityOverflow);
        };
        try_allocate_startup_batch(&mut ownership.canonical_assembly, canonical_assembly)?;
        try_allocate_startup_batch(&mut ownership.evidence_assembly, evidence_assembly)?;
        try_allocate_startup_batch(&mut ownership.fallback, fallback)?;
        try_allocate_startup_batch(&mut ownership.evidence, evidence)?;
        Ok(())
    }

    /// Converts the complete admitted graph into the existing runtime batches.
    ///
    /// The unforgeable authority is constructed only by the Ready cross-plane owner. All backing
    /// capacity and ledger cores already exist, so this transition only moves ownership and fills
    /// two preallocated attribution slots.
    pub(crate) fn into_runtime_baseline(
        mut self,
        _authority: ReadyStartupGraphSplitAuthority,
    ) -> FallbackBatchBaseline<O, P, D> {
        let ownership = self
            .ownership
            .take()
            .expect("ready startup graph owns its runtime parts");
        let StartupAttributionGraphOwnership {
            runtime_owners:
                [
                    canonical_owner,
                    evidence_assembly_owner,
                    fallback_owner,
                    evidence_owner,
                ],
            canonical_assembly,
            evidence_assembly,
            mut fallback,
            mut evidence,
            cores,
            _account_shares: _,
        } = ownership;
        let StartupLedgerCores {
            canonical_assembly_cover,
            evidence_assembly_cover,
            fallback_cover,
            evidence_cover,
            fallback_attribution,
            evidence_attribution,
        } = cores;

        fallback.attributions.push(OperationAttributionReservation {
            core: fallback_attribution,
            destination: PhantomData,
        });
        evidence.attributions.push(OperationAttributionReservation {
            core: evidence_attribution,
            destination: PhantomData,
        });

        FallbackBatchBaseline::new_unbound(
            into_runtime_batch(
                canonical_assembly,
                canonical_assembly_cover,
                canonical_owner,
            ),
            into_runtime_batch(
                evidence_assembly,
                evidence_assembly_cover,
                evidence_assembly_owner,
            ),
            into_runtime_batch(fallback, fallback_cover, fallback_owner),
            into_runtime_batch(evidence, evidence_cover, evidence_owner),
        )
    }
}

impl<O, P, D> Drop for StartupAttributionGraph<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(ownership) = self.ownership.take() {
            drop_startup_graph_ownership(ownership);
        }
    }
}

impl<O, P, D> Ledger<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    /// Atomically admits the complete ledger side of retained startup before allocator entry.
    pub(crate) fn try_admit_startup_attribution_graph(
        &self,
        parts: StartupLedgerPlanParts<O, P, D>,
    ) -> Result<StartupAttributionGraph<O, P, D>, AttributionBatchCreateError<D>> {
        let StartupLedgerPlanParts {
            batches,
            retained_attributions,
            account_shares,
            commitments,
        } = parts;
        let StartupAttributionBatchPlans {
            canonical_assembly,
            evidence_assembly,
            fallback,
            evidence,
        } = batches;

        if fallback.attribution_limit == 0
            || evidence.attribution_limit == 0
            || fallback.commitment_limit == 0
            || evidence.commitment_limit == 0
        {
            return Err(AttributionBatchCreateError::CapacityOverflow);
        }

        let reservation_specs = [
            StartupReservationSpec::for_batch(&canonical_assembly),
            StartupReservationSpec::for_batch(&evidence_assembly),
            StartupReservationSpec::for_batch(&fallback),
            StartupReservationSpec::for_batch(&evidence),
            StartupReservationSpec::for_retained(retained_attributions[0]),
            StartupReservationSpec::for_retained(retained_attributions[1]),
        ];
        let StartupLedgerCommit { cores, commitments } = self
            .try_admit_startup_ledger(reservation_specs, &account_shares, commitments)
            .map_err(AttributionBatchCreateError::Ledger)?;

        let (canonical_assembly, canonical_allocation) = empty_startup_batch(canonical_assembly);
        let (evidence_assembly, evidence_assembly_allocation) =
            empty_startup_batch(evidence_assembly);
        let (fallback, fallback_allocation) = empty_startup_batch(fallback);
        let (evidence, evidence_allocation) = empty_startup_batch(evidence);
        let mut graph = StartupAttributionGraph {
            ownership: Some(StartupAttributionGraphOwnership {
                runtime_owners: [
                    Arc::clone(&self.state),
                    Arc::clone(&self.state),
                    Arc::clone(&self.state),
                    Arc::clone(&self.state),
                ],
                canonical_assembly,
                evidence_assembly,
                fallback,
                evidence,
                cores,
                _account_shares: account_shares,
            }),
        };
        graph.try_allocate_backings(
            canonical_allocation,
            evidence_assembly_allocation,
            fallback_allocation,
            evidence_allocation,
        )?;

        let Some(ownership) = graph.ownership.as_mut() else {
            return Err(AttributionBatchCreateError::CapacityOverflow);
        };
        ownership.fallback.commitments.push(commitments[0]);
        ownership.evidence.commitments.push(commitments[1]);
        Ok(graph)
    }

    fn try_admit_startup_ledger(
        &self,
        reservation_specs: [StartupReservationSpec<P, D>; STARTUP_RESERVATION_COUNT as usize],
        account_shares: &StartupAccountShareEnvelope<D>,
        commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
    ) -> Result<StartupLedgerCommit<O, P, D>, LedgerCause<D>> {
        let mut state = lock(&self.state);
        Self::ensure_open(&state)?;
        let admission =
            self.preflight_startup_ledger(&state, reservation_specs, account_shares, commitments)?;
        commit_startup_ledger(&mut state, &admission);
        let StartupLedgerAdmission {
            reservations,
            commitments,
            ..
        } = admission;
        Ok(StartupLedgerCommit {
            cores: StartupLedgerCores::from_array(reservations.map(|projection| {
                ReservationCore::new(
                    Arc::clone(&self.state),
                    projection.spec.phase,
                    projection.phase_position,
                    projection.spec.dimension,
                    projection.dimension_position,
                    projection.spec.bytes,
                )
            })),
            commitments,
        })
    }

    fn preflight_startup_ledger(
        &self,
        state: &LedgerState<P, D>,
        reservation_specs: [StartupReservationSpec<P, D>; STARTUP_RESERVATION_COUNT as usize],
        account_shares: &StartupAccountShareEnvelope<D>,
        mut commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
    ) -> Result<StartupLedgerAdmission<P, D>, LedgerCause<D>> {
        let mut reservations = reservation_specs.map(|spec| StartupReservationProjection {
            spec,
            phase_position: usize::MAX,
            dimension_position: usize::MAX,
        });
        for projection in &mut reservations {
            let phase_position = phase_entry(&state.phases, projection.spec.phase)
                .ok_or(LedgerCause::UnknownPhase)?
                .state_position;
            if state.phase_states.get(phase_position).is_none() {
                return Err(LedgerCause::FinalSnapshotShapeMismatch);
            }
            let dimension_position = state
                .dimensions
                .binary_search_by_key(&projection.spec.dimension, |entry| entry.dimension)
                .map_err(|_| LedgerCause::UnknownDimension {
                    dimension: projection.spec.dimension,
                })?;
            if state.dimensions[dimension_position]
                .policy
                .live_byte_limit
                .is_none()
            {
                return Err(LedgerCause::DimensionNotReservable {
                    dimension: projection.spec.dimension,
                });
            }
            projection.phase_position = phase_position;
            projection.dimension_position = dimension_position;
        }

        for (index, projection) in reservations.iter().enumerate() {
            if reservations[..index]
                .iter()
                .any(|prior| prior.dimension_position == projection.dimension_position)
            {
                continue;
            }
            let added = reservations
                .iter()
                .filter(|candidate| candidate.dimension_position == projection.dimension_position)
                .try_fold(0_u64, |total, candidate| {
                    total
                        .checked_add(candidate.spec.bytes)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Live,
                            dimension: projection.spec.dimension,
                        })
                })?;
            let entry = &state.dimensions[projection.dimension_position];
            let value = entry.state.current_live_bytes.checked_add(added).ok_or(
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: projection.spec.dimension,
                },
            )?;
            let limit = entry
                .policy
                .live_byte_limit
                .expect("startup reservation dimensions were proven reservable");
            if value > limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: projection.spec.dimension,
                    value,
                    limit,
                });
            }
        }

        for (index, projection) in reservations.iter().enumerate() {
            if reservations[..index]
                .iter()
                .any(|prior| prior.phase_position == projection.phase_position)
            {
                continue;
            }
            let added = reservations
                .iter()
                .filter(|candidate| candidate.phase_position == projection.phase_position)
                .try_fold(0_u64, |total, candidate| {
                    total
                        .checked_add(candidate.spec.bytes)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Live,
                            dimension: projection.spec.dimension,
                        })
                })?;
            state.phase_states[projection.phase_position]
                .current_live_bytes
                .checked_add(added)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: projection.spec.dimension,
                })?;
        }

        let mut operation_live_after = state.current_live_bytes;
        for projection in &reservations {
            operation_live_after = operation_live_after
                .checked_add(projection.spec.bytes)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: projection.spec.dimension,
                })?;
            if operation_live_after > self.operation_live_byte_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: projection.spec.dimension,
                    value: operation_live_after,
                    limit: self.operation_live_byte_limit,
                });
            }
        }
        let active_reservations_after = state
            .active_reservations
            .checked_add(STARTUP_RESERVATION_COUNT)
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Live,
                dimension: reservations[0].spec.dimension,
            })?;

        let steps = &account_shares.steps;
        let work_specs = [
            steps[0].charges[0],
            steps[0].charges[1],
            steps[1].charges[0],
            steps[1].charges[1],
            steps[2].charges[0],
            steps[2].charges[1],
            steps[3].charges[0],
            steps[3].charges[1],
        ];
        let mut work = work_specs.map(|(dimension, amount)| StartupWorkProjection {
            dimension,
            dimension_position: usize::MAX,
            amount,
        });
        for projection in &mut work {
            projection.dimension_position = state
                .dimensions
                .binary_search_by_key(&projection.dimension, |entry| entry.dimension)
                .map_err(|_| LedgerCause::UnknownDimension {
                    dimension: projection.dimension,
                })?;
        }
        for (index, projection) in work.iter().enumerate() {
            if work[..index]
                .iter()
                .any(|prior| prior.dimension_position == projection.dimension_position)
            {
                continue;
            }
            let added = work
                .iter()
                .filter(|candidate| candidate.dimension_position == projection.dimension_position)
                .try_fold(0_u64, |total, candidate| {
                    total
                        .checked_add(candidate.amount)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Attempted,
                            dimension: projection.dimension,
                        })
                })?;
            let entry = &state.dimensions[projection.dimension_position];
            let attempted = entry.state.attempted.checked_add(added).ok_or(
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Attempted,
                    dimension: projection.dimension,
                },
            )?;
            if attempted > entry.policy.attempted_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Attempted,
                    dimension: projection.dimension,
                    value: attempted,
                    limit: entry.policy.attempted_limit,
                });
            }
            let consumed =
                entry
                    .state
                    .consumed
                    .checked_add(added)
                    .ok_or(LedgerCause::ArithmeticOverflow {
                        class: CounterClass::Consumed,
                        dimension: projection.dimension,
                    })?;
            if consumed > attempted {
                return Err(LedgerCause::ConsumedExceedsAttempted {
                    dimension: projection.dimension,
                    consumed,
                    attempted,
                });
            }
            if consumed > entry.policy.consumed_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Consumed,
                    dimension: projection.dimension,
                    value: consumed,
                    limit: entry.policy.consumed_limit,
                });
            }
        }

        for index in 0..commitments.len() {
            let commitment = commitments[index];
            let dimension_position = state
                .dimensions
                .binary_search_by_key(&commitment.dimension, |entry| entry.dimension)
                .map_err(|_| LedgerCause::UnknownDimension {
                    dimension: commitment.dimension,
                })?;
            commitments[index].dimension_position = dimension_position;
            if commitments[..index]
                .iter()
                .any(|prior| prior.dimension == commitment.dimension)
            {
                continue;
            }
            let added = commitments
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
            let entry = &state.dimensions[dimension_position];
            let value = entry
                .state
                .published_outcome_evidence
                .checked_add(added)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class: CounterClass::PublishedOutcomeEvidence,
                    dimension: commitment.dimension,
                })?;
            if value > entry.policy.published_outcome_evidence_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::PublishedOutcomeEvidence,
                    dimension: commitment.dimension,
                    value,
                    limit: entry.policy.published_outcome_evidence_limit,
                });
            }
        }

        Ok(StartupLedgerAdmission {
            reservations,
            work,
            commitments,
            operation_live_after,
            active_reservations_after,
        })
    }
}

fn commit_startup_ledger<P: Copy + Ord, D: Copy + Ord>(
    state: &mut LedgerState<P, D>,
    admission: &StartupLedgerAdmission<P, D>,
) {
    for projection in &admission.reservations {
        let dimension = &mut state.dimensions[projection.dimension_position].state;
        dimension.current_live_bytes += projection.spec.bytes;
        dimension.peak_live_bytes = dimension.peak_live_bytes.max(dimension.current_live_bytes);
        let phase = &mut state.phase_states[projection.phase_position];
        phase.current_live_bytes += projection.spec.bytes;
        phase.peak_live_bytes = phase.peak_live_bytes.max(phase.current_live_bytes);
    }
    state.current_live_bytes = admission.operation_live_after;
    state.peak_live_bytes = state.peak_live_bytes.max(admission.operation_live_after);
    state.active_reservations = admission.active_reservations_after;
    for projection in &admission.work {
        let dimension = &mut state.dimensions[projection.dimension_position].state;
        dimension.attempted += projection.amount;
        dimension.consumed += projection.amount;
    }
}

fn empty_startup_batch<O, P, D, Destination>(
    plan: AttributionBatchPlan<O, P, D, Destination>,
) -> (
    StartupAttributionBatchBackings<O, P, D, Destination>,
    StartupBatchAllocationPlans,
)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    let AttributionBatchPlan {
        phase: _,
        capacity_dimension: _,
        attribution_capacity,
        commitment_capacity,
        planned_bytes,
        attribution_limit,
        commitment_limit,
        destination: _,
    } = plan;
    (
        StartupAttributionBatchBackings {
            attributions: Vec::new(),
            commitments: Vec::new(),
            attribution_limit,
            commitment_limit,
        },
        StartupBatchAllocationPlans {
            attributions: attribution_capacity,
            commitments: commitment_capacity,
            planned_bytes,
        },
    )
}

fn try_allocate_startup_batch<O, P, D, Destination>(
    batch: &mut StartupAttributionBatchBackings<O, P, D, Destination>,
    plans: StartupBatchAllocationPlans,
) -> Result<(), AttributionBatchCreateError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    let attribution_bytes =
        try_allocate_startup_backing(&mut batch.attributions, plans.attributions)?;
    let commitment_bytes = try_allocate_startup_backing(&mut batch.commitments, plans.commitments)?;
    let actual_bytes = attribution_bytes
        .checked_add(commitment_bytes)
        .ok_or(AttributionBatchCreateError::CapacityOverflow)?;
    if actual_bytes > plans.planned_bytes
        || batch.attributions.capacity() < batch.attribution_limit
        || batch.commitments.capacity() < batch.commitment_limit
    {
        return Err(AttributionBatchCreateError::CapacityOverflow);
    }
    Ok(())
}

fn try_allocate_startup_backing<T, D>(
    values: &mut Vec<T>,
    plan: CapacityPlan,
) -> Result<u64, AttributionBatchCreateError<D>> {
    plan.verify_preflight(values.len(), values.capacity())
        .map_err(|_| AttributionBatchCreateError::CapacityOverflow)?;
    if enter_startup_graph_allocator() {
        return Err(AttributionBatchCreateError::AllocationFailed);
    }
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| AttributionBatchCreateError::AllocationFailed)?;
    plan.finish(values.len(), values.capacity())
        .map_err(|_| AttributionBatchCreateError::CapacityOverflow)
}

fn enter_startup_graph_allocator() -> bool {
    #[cfg(test)]
    {
        let index = STARTUP_GRAPH_ALLOCATION_ENTRIES.with(|entries| {
            let index = entries.get();
            entries.set(index.saturating_add(1));
            index
        });
        FAIL_STARTUP_GRAPH_ALLOCATION_AT.with(|failpoint| failpoint.get() == Some(index))
    }
    #[cfg(not(test))]
    {
        false
    }
}

fn drop_startup_batch_backings<O, P, D, Destination>(
    batch: &mut StartupAttributionBatchBackings<O, P, D, Destination>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    drop(mem::take(&mut batch.attributions));
    test_note_startup_graph_storage_release();
    drop(mem::take(&mut batch.commitments));
    test_note_startup_graph_storage_release();
}

fn test_note_startup_graph_storage_release() {
    #[cfg(test)]
    STARTUP_GRAPH_STORAGE_RELEASES.with(|releases| {
        releases.set(releases.get().saturating_add(1));
    });
}

fn test_note_startup_graph_ledger_release_lock() {
    #[cfg(test)]
    {
        STARTUP_GRAPH_LEDGER_RELEASE_LOCKS.with(|locks| {
            locks.set(locks.get().saturating_add(1));
        });
        let releases = STARTUP_GRAPH_STORAGE_RELEASES.with(std::cell::Cell::get);
        STARTUP_GRAPH_STORAGE_RELEASES_AT_LEDGER_LOCK.with(|observed| observed.set(Some(releases)));
    }
}

#[cfg(test)]
mod tests;
