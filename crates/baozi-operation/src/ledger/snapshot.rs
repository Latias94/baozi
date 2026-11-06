#[cfg(test)]
use super::lock;
use super::storage::DynamicStorage;
use super::work::WorkState;
use super::{
    AttributionReservationBatch, CanonicalOutput, DimensionState, LedgerCause, LedgerEvidenceError,
    LedgerEvidencePurpose, LedgerState, OutcomeEvidenceDestination, PhaseState, release_evidence,
};
use crate::{CapacityPlan, CapacityPlanError, OperationOwner};

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
/// Dimension and phase entries are stored in stable key order. Administrative observations count
/// actual vector capacities and the known shared payload at capture, including live progress and
/// terminal staging. Opaque Arc, runtime and allocator overhead is excluded.
#[derive(Debug, PartialEq, Eq)]
pub struct LedgerSnapshot<P, D> {
    dimensions: Vec<(D, DimensionSnapshot)>,
    phases: Vec<(P, PhaseSnapshot)>,
    last_completed_phase: Option<P>,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_administrative_bytes: u64,
    peak_administrative_bytes: u64,
    covered_administrative_bytes: Option<u64>,
}

impl<P: Copy + Ord, D: Copy + Ord> LedgerSnapshot<P, D> {
    pub(crate) fn qualify_administrative_coverage(&mut self, bytes: Option<u64>) {
        self.covered_administrative_bytes =
            bytes.filter(|bytes| *bytes == self.current_administrative_bytes);
    }

    #[cfg(test)]
    pub(crate) fn has_exact_administrative_coverage(&self) -> bool {
        self.covered_administrative_bytes == Some(self.current_administrative_bytes)
    }

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

    /// Returns known administrative payload and capacity bytes at the capture boundary.
    pub const fn current_administrative_bytes(&self) -> u64 {
        self.current_administrative_bytes
    }

    /// Returns the known administrative payload and capacity high-water mark.
    pub const fn peak_administrative_bytes(&self) -> u64 {
        self.peak_administrative_bytes
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

struct SnapshotEvidenceAssembly<'a, P, D> {
    storage: &'a mut DynamicStorage<P, D>,
    dimensions: Vec<(D, DimensionSnapshot)>,
    phases: Vec<(P, PhaseSnapshot)>,
    admitted_bytes: u64,
}

impl<'a, P, D> SnapshotEvidenceAssembly<'a, P, D> {
    fn new(
        storage: &'a mut DynamicStorage<P, D>,
    ) -> Result<(Self, CapacityPlan, CapacityPlan), LedgerEvidenceError> {
        let (dimension_plan, phase_plan, admitted_bytes) = {
            let mut state = storage.write();
            if !phase_catalog_positions_are_valid(&state) {
                return Err(LedgerEvidenceError::PhaseCatalogShapeMismatch);
            }
            if state.dimensions.len() != state.dimension_snapshot_staging.len() {
                return Err(LedgerEvidenceError::SnapshotShapeMismatch);
            }
            let dimension_plan =
                CapacityPlan::for_type::<(D, DimensionSnapshot)>(0, 0, state.dimensions.len())
                    .map_err(snapshot_capacity_error)?;
            let phase_plan = CapacityPlan::for_type::<(P, PhaseSnapshot)>(0, 0, state.phases.len())
                .map_err(snapshot_capacity_error)?;
            let admitted_bytes = dimension_plan
                .envelope_delta_bytes()
                .map_err(snapshot_capacity_error)?
                .checked_add(
                    phase_plan
                        .envelope_delta_bytes()
                        .map_err(snapshot_capacity_error)?,
                )
                .ok_or_else(|| snapshot_capacity_error(CapacityPlanError::CapacityOverflow))?;
            let admitted = state
                .current_administrative_bytes
                .checked_add(admitted_bytes)
                .ok_or_else(|| snapshot_capacity_error(CapacityPlanError::CapacityOverflow))?;
            if admitted > state.evidence_byte_limit {
                return Err(LedgerEvidenceError::LimitExceeded {
                    purpose: LedgerEvidencePurpose::Snapshot,
                    value: admitted,
                    limit: state.evidence_byte_limit,
                });
            }
            state.current_administrative_bytes = admitted;
            state.peak_administrative_bytes = state.peak_administrative_bytes.max(admitted);
            (dimension_plan, phase_plan, admitted_bytes)
        };
        Ok((
            Self {
                storage,
                dimensions: Vec::new(),
                phases: Vec::new(),
                admitted_bytes,
            },
            dimension_plan,
            phase_plan,
        ))
    }

    fn allocate(
        &mut self,
        dimension_plan: CapacityPlan,
        phase_plan: CapacityPlan,
    ) -> Result<(), LedgerEvidenceError> {
        let dimension_bytes = allocate_snapshot_vec(&mut self.dimensions, dimension_plan)?;
        let phase_bytes = allocate_snapshot_vec(&mut self.phases, phase_plan)?;
        let actual_bytes = dimension_bytes
            .checked_add(phase_bytes)
            .ok_or_else(|| snapshot_capacity_error(CapacityPlanError::CapacityOverflow))?;
        let unused_bytes = self.admitted_bytes - actual_bytes;
        let mut state = self.storage.write();
        release_evidence(&mut state, unused_bytes);
        self.admitted_bytes = actual_bytes;
        Ok(())
    }
}

impl<P, D> Drop for SnapshotEvidenceAssembly<'_, P, D> {
    fn drop(&mut self) {
        // Backing must be gone before its owned admission is released against current counters.
        drop(std::mem::take(&mut self.phases));
        drop(std::mem::take(&mut self.dimensions));
        if self.admitted_bytes != 0 {
            release_evidence(&mut self.storage.write(), self.admitted_bytes);
        }
    }
}

fn snapshot_capacity_error(cause: CapacityPlanError) -> LedgerEvidenceError {
    LedgerEvidenceError::Capacity {
        purpose: LedgerEvidencePurpose::Snapshot,
        cause,
    }
}

fn allocate_snapshot_vec<T>(
    values: &mut Vec<T>,
    plan: CapacityPlan,
) -> Result<u64, LedgerEvidenceError> {
    plan.verify_preflight(values.len(), values.capacity())
        .map_err(snapshot_capacity_error)?;
    if plan.desired_capacity() <= values.capacity() {
        return plan
            .finish(values.len(), values.capacity())
            .map_err(snapshot_capacity_error);
    }
    let requested_bytes = plan
        .envelope_delta_bytes()
        .map_err(snapshot_capacity_error)?;
    #[cfg(test)]
    if super::test_take_evidence_allocation_failure() {
        return Err(LedgerEvidenceError::AllocationFailed {
            purpose: LedgerEvidencePurpose::Snapshot,
            requested_bytes,
        });
    }
    if values
        .try_reserve_exact(plan.allocator_additional())
        .is_err()
    {
        return Err(LedgerEvidenceError::AllocationFailed {
            purpose: LedgerEvidencePurpose::Snapshot,
            requested_bytes,
        });
    }
    #[cfg(test)]
    let observed_capacity = if super::test_take_evidence_over_envelope_failure() {
        plan.envelope_capacity().saturating_add(1)
    } else {
        values.capacity()
    };
    #[cfg(not(test))]
    let observed_capacity = values.capacity();
    plan.finish(values.len(), observed_capacity)
        .map_err(snapshot_capacity_error)
}

pub(super) fn try_snapshot<P, D>(
    storage: &mut DynamicStorage<P, D>,
    catalog: &[D],
    work: &WorkState<D>,
) -> Result<LedgerSnapshot<P, D>, LedgerEvidenceError>
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    let (mut assembly, dimension_plan, phase_plan) = SnapshotEvidenceAssembly::new(storage)?;
    assembly.allocate(dimension_plan, phase_plan)?;

    // Sample work only after allocation, then hold the sole joint observation interval.
    work.check_integrity(catalog.len())
        .map_err(|_| LedgerEvidenceError::SnapshotShapeMismatch)?;
    let mut state = assembly.storage.write();
    if !phase_catalog_positions_are_valid(&state) {
        return Err(LedgerEvidenceError::PhaseCatalogShapeMismatch);
    }
    if state.dimensions.len() != work.rows.len()
        || state.dimensions.len() != state.dimension_snapshot_staging.len()
        || state.dimensions.len() != dimension_plan.desired_capacity()
        || state.phases.len() != phase_plan.desired_capacity()
    {
        return Err(LedgerEvidenceError::SnapshotShapeMismatch);
    }
    for (entry, row) in state.dimensions.iter().zip(&work.rows) {
        assembly.dimensions.push((
            entry.dimension,
            dimension_snapshot(DimensionState::join(row.state, entry.state)),
        ));
    }
    for entry in &state.phases {
        assembly.phases.push((
            entry.phase,
            phase_snapshot(state.phase_states[entry.state_position]),
        ));
    }
    release_evidence(&mut state, assembly.admitted_bytes);
    assembly.admitted_bytes = 0;
    let snapshot = LedgerSnapshot {
        dimensions: std::mem::take(&mut assembly.dimensions),
        phases: std::mem::take(&mut assembly.phases),
        last_completed_phase: state.last_completed_phase.map(|completed| completed.phase),
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
        active_reservations: state.active_reservations,
        current_administrative_bytes: state.current_administrative_bytes,
        peak_administrative_bytes: state.peak_administrative_bytes,
        covered_administrative_bytes: None,
    };
    drop(state);
    Ok(snapshot)
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(super) fn validate_final_snapshot_shape<P: Copy + PartialEq, D>(
    state: &LedgerState<P, D>,
    work: &WorkState<D>,
) -> Result<(), LedgerCause<D>> {
    if work.header.integrity_fault {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    if work.header.open_envelopes != 0
        || work.header.producer_open
        || work.header.parser_parent_open
        || work.header.parser_child_open
        || work.header.protected_open
        || work.rows.iter().any(|row| row.state.pending_consumed != 0)
    {
        return Err(LedgerCause::OutstandingLinearWorkPermits {
            count: work.header.open_envelopes.max(1),
        });
    }
    if state.dimensions.len() != work.rows.len()
        || state.dimensions.len() != state.dimension_snapshot_staging.len()
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
pub(super) fn typed_terminal_snapshot_positions_are_valid<P, D>(
    state: &LedgerState<P, D>,
    work: &WorkState<D>,
) -> bool {
    !work.header.integrity_fault
        && work.header.open_envelopes == 0
        && !work.header.producer_open
        && !work.header.parser_parent_open
        && !work.header.parser_child_open
        && !work.header.protected_open
        && work.rows.iter().all(|row| row.state.pending_consumed == 0)
        && state.dimensions.len() == work.rows.len()
        && state.dimensions.len() == state.dimension_snapshot_staging.len()
        && phase_catalog_positions_are_valid(state)
}

fn phase_catalog_positions_are_valid<P, D>(state: &LedgerState<P, D>) -> bool {
    let phase_count = state.phases.len();
    phase_count == state.phase_states.len()
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

pub(super) fn phase_catalog_shape_is_valid<P, D>(state: &LedgerState<P, D>) -> bool {
    // Keys and dense positions are frozen before the storage state is shared.
    phase_catalog_positions_are_valid(state)
}

pub(super) fn take_final_snapshot<P, D>(
    state: &mut LedgerState<P, D>,
    work: &WorkState<D>,
    _clean: bool,
) -> LedgerSnapshot<P, D>
where
    P: Copy + Ord,
    D: Copy + Ord,
{
    for ((entry, row), staging) in state
        .dimensions
        .iter()
        .zip(&work.rows)
        .zip(state.dimension_snapshot_staging.iter_mut())
    {
        staging.0 = entry.dimension;
        staging.1 = dimension_snapshot(DimensionState::join(row.state, entry.state));
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
    let retained_evidence = state.current_administrative_bytes;
    let last_completed_phase = state.last_completed_phase.map(|completed| completed.phase);
    LedgerSnapshot {
        dimensions: std::mem::take(&mut state.dimension_snapshot_staging),
        phases: std::mem::take(&mut state.phase_snapshot_staging),
        last_completed_phase,
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
        active_reservations: state.active_reservations,
        current_administrative_bytes: retained_evidence,
        peak_administrative_bytes: state.peak_administrative_bytes,
        covered_administrative_bytes: None,
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(super) fn take_typed_final_snapshot<P: Copy, D: Copy>(
    state: &mut LedgerState<P, D>,
    work: &WorkState<D>,
    last_completed_phase: Option<P>,
) -> LedgerSnapshot<P, D> {
    for ((entry, row), staging) in state
        .dimensions
        .iter()
        .zip(&work.rows)
        .zip(state.dimension_snapshot_staging.iter_mut())
    {
        staging.0 = entry.dimension;
        staging.1 = dimension_snapshot(DimensionState::join(row.state, entry.state));
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
        current_administrative_bytes: state.current_administrative_bytes,
        peak_administrative_bytes: state.peak_administrative_bytes,
        covered_administrative_bytes: None,
    }
}

/// Moves the pre-admitted terminal staging into an immutable final projection without changing
/// the still-live ledger counters that back physical publication eligibility.
///
/// Qualification has already proved every positioned sum and destination limit. The projection
/// therefore performs no lookup, allocation, checked arithmetic, or owner-defined comparison.
pub(super) fn take_typed_terminal_projection<O, P, D>(
    state: &mut LedgerState<P, D>,
    work: &WorkState<D>,
    last_completed_phase: Option<P>,
    canonical: &AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence: &AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
) -> LedgerSnapshot<P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    for (index, ((entry, row), staging)) in state
        .dimensions
        .iter()
        .zip(&work.rows)
        .zip(state.dimension_snapshot_staging.iter_mut())
        .enumerate()
    {
        let canonical_increment = canonical.qualified_commitment_amount(index);
        let evidence_increment = evidence.qualified_commitment_amount(index);
        let joined = DimensionState::join(row.state, entry.state);
        staging.0 = entry.dimension;
        staging.1 = DimensionSnapshot::from_terminal_evidence(DimensionSnapshotScalars {
            attempted: joined.attempted,
            consumed: joined.consumed,
            current_live_bytes: 0,
            peak_live_bytes: joined.peak_live_bytes,
            pending_publication: joined.pending_publication,
            committed: joined.committed + canonical_increment,
            committed_canonical_output: joined.committed_canonical_output + canonical_increment,
            published_outcome_evidence: joined.published_outcome_evidence + evidence_increment,
        });
    }
    for (entry, staging) in state
        .phases
        .iter()
        .zip(state.phase_snapshot_staging.iter_mut())
    {
        staging.0 = entry.phase;
        staging.1 = PhaseSnapshot::from_terminal_evidence(
            0,
            state.phase_states[entry.state_position].peak_live_bytes,
        );
    }
    LedgerSnapshot {
        dimensions: std::mem::take(&mut state.dimension_snapshot_staging),
        phases: std::mem::take(&mut state.phase_snapshot_staging),
        last_completed_phase,
        current_live_bytes: 0,
        peak_live_bytes: state.peak_live_bytes,
        active_reservations: 0,
        current_administrative_bytes: state.current_administrative_bytes,
        peak_administrative_bytes: state.peak_administrative_bytes,
        covered_administrative_bytes: None,
    }
}

pub(super) const fn dimension_snapshot(state: DimensionState) -> DimensionSnapshot {
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

pub(super) const fn phase_snapshot(state: PhaseState) -> PhaseSnapshot {
    PhaseSnapshot {
        current_live_bytes: state.current_live_bytes,
        peak_live_bytes: state.peak_live_bytes,
    }
}

#[cfg(test)]
mod tests;
