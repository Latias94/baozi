use super::{
    AttributionBatchCreateError, CanonicalOutput, LedgerCause, LedgerState,
    OutcomeEvidenceDestination, OutputCommitment, PublicationDestination, lock,
    release_positioned_locked,
};
use crate::{CapacityPlan, OperationOwner};
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

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
    pub(super) core: ReservationCore<O, P, D>,
}

pub(super) struct ReservationCore<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) owner: Arc<Mutex<LedgerState<P, D>>>,
    pub(super) phase: P,
    pub(super) phase_position: usize,
    pub(super) dimension: D,
    pub(super) dimension_position: usize,
    pub(super) bytes: u64,
    pub(super) active: bool,
    operation_owner: PhantomData<fn() -> O>,
}

impl<O, P, D> ReservationCore<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) fn new(
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

    pub(super) fn is_owned_by(&self, owner: &Arc<Mutex<LedgerState<P, D>>>) -> bool {
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

    pub(super) fn release_locked(&mut self, state: &mut LedgerState<P, D>) {
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
    pub(super) core: ReservationCore<O, P, D>,
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
    pub(super) core: ReservationCore<O, P, D>,
    pub(super) destination: PhantomData<fn() -> Destination>,
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
    pub(super) phase: P,
    pub(super) capacity_dimension: D,
    pub(super) attribution_capacity: CapacityPlan,
    pub(super) commitment_capacity: CapacityPlan,
    pub(super) planned_bytes: u64,
    pub(super) attribution_limit: usize,
    pub(super) commitment_limit: usize,
    pub(super) destination: PhantomData<fn() -> (O, Destination)>,
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
    pub(super) attributions: Vec<OperationAttributionReservation<O, P, D, Destination>>,
    pub(super) commitments: Vec<OutputCommitment<D, Destination>>,
    pub(super) capacity_cover: Option<StorageCoveringReservation<O, P, D>>,
    pub(super) attribution_limit: usize,
    pub(super) commitment_limit: usize,
    pub(super) owner: Arc<Mutex<LedgerState<P, D>>>,
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
    pub(super) canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    pub(super) evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    pub(super) fallback: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    pub(super) evidence: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
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
    pub(super) baseline: FallbackBatchBaseline<O, P, D>,
    pub(super) cause: LedgerCause<D>,
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

    pub(super) fn try_prepare_fallback_sources(&mut self) -> Result<(), ()> {
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

    pub(super) fn has_commitment_slot(&self) -> bool {
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

    /// Resolves the newest attribution after its retained storage rollback was observed.
    ///
    /// A verified storage release drops the matching token. Uncertain cleanup keeps the live
    /// attribution fail-closed. This narrow suffix operation exists only for construction
    /// transaction rollback; publication paths never detach attribution.
    pub(crate) fn finish_last_attribution_after_storage_rollback(
        &mut self,
        storage_released: bool,
    ) -> bool {
        let Some(attribution) = self.attributions.pop() else {
            return false;
        };
        if storage_released {
            drop(attribution);
        } else {
            std::mem::forget(attribution);
        }
        true
    }

    pub(super) fn push_positioned_commitment(
        &mut self,
        commitment: OutputCommitment<D, Destination>,
    ) {
        debug_assert!(self.has_commitment_slot());
        debug_assert_ne!(commitment.dimension_position, usize::MAX);
        self.commitments.push(commitment);
    }

    pub(crate) fn attribution_len(&self) -> usize {
        self.attributions.len()
    }

    pub(crate) fn commitment_len(&self) -> usize {
        self.commitments.len()
    }

    pub(crate) fn attributed_bytes(&self) -> Option<u64> {
        self.attributions
            .iter()
            .try_fold(0_u64, |total, attribution| {
                total.checked_add(attribution.bytes())
            })
    }

    pub(super) fn absorb_proven_retaining_source_storage(&mut self, other: &mut Self) {
        self.attributions.append(&mut other.attributions);
        self.commitments.append(&mut other.commitments);
    }

    pub(super) fn can_absorb_sources<const N: usize>(&self, sources: [Option<&Self>; N]) -> bool {
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

    pub(super) fn combined_attribution_count<const N: usize>(
        &self,
        sources: [Option<&Self>; N],
    ) -> Option<u64> {
        sources.into_iter().flatten().try_fold(
            u64::try_from(self.attributions.len()).ok()?,
            |count, source| count.checked_add(u64::try_from(source.attributions.len()).ok()?),
        )
    }

    pub(super) fn combined_attribution_bytes<const N: usize>(
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

    pub(super) fn validate_owner(&self, owner: &Arc<Mutex<LedgerState<P, D>>>) -> bool {
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

    pub(super) fn is_empty_storage_shell(&self) -> bool {
        self.attributions.is_empty() && self.commitments.is_empty()
    }

    fn attribution_bytes(&self) -> Option<u64> {
        self.attributions
            .iter()
            .try_fold(0_u64, |total, token| total.checked_add(token.core.bytes))
    }

    pub(super) fn active_count(&self) -> Option<u64> {
        u64::try_from(self.attributions.len())
            .ok()?
            .checked_add(u64::from(self.capacity_cover.is_some()))
    }

    pub(super) fn live_bytes(&self) -> Option<u64> {
        self.attribution_bytes()?
            .checked_add(self.account_capacity_bytes())
    }

    pub(super) fn positioned_dimension_bytes(&self, position: usize) -> Option<u64> {
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

    pub(super) fn positioned_phase_bytes(&self, position: usize) -> Option<u64> {
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

    pub(super) fn positioned_commitment_amount(&self, position: usize) -> Option<u64> {
        self.commitments
            .iter()
            .filter(|commitment| commitment.dimension_position == position)
            .try_fold(0_u64, |total, commitment| {
                total.checked_add(commitment.amount)
            })
    }

    pub(super) fn bound_positions_are_in_bounds(
        &self,
        dimension_count: usize,
        phase_count: usize,
    ) -> bool {
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

    pub(super) fn discard_positions_are_in_bounds(
        &self,
        dimension_count: usize,
        phase_count: usize,
    ) -> bool {
        self.attributions.iter().all(|token| {
            token.core.dimension_position < dimension_count
                && token.core.phase_position < phase_count
        }) && self.capacity_cover.as_ref().is_none_or(|cover| {
            cover.core.dimension_position < dimension_count
                && cover.core.phase_position < phase_count
        })
    }

    pub(super) fn release_bound_attributions(&mut self, state: &mut LedgerState<P, D>) {
        for token in &mut self.attributions {
            token.core.release_bound_locked(state);
        }
    }

    pub(super) fn destroy_storage_then_release_bound_cover(
        &mut self,
        state: &mut LedgerState<P, D>,
    ) {
        drop(std::mem::take(&mut self.attributions));
        drop(std::mem::take(&mut self.commitments));
        if let Some(mut capacity_cover) = self.capacity_cover.take() {
            capacity_cover.core.release_bound_locked(state);
            drop(capacity_cover);
        }
    }
}
