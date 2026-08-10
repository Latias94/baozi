use super::{
    AttributionReservationBatch, CanonicalOutput, DimensionState, LedgerCause, LedgerEvidenceError,
    LedgerEvidencePurpose, LedgerState, OutcomeEvidenceDestination, PhaseState, capacity_bytes,
    release_evidence, try_grow_evidence_vec,
};
use crate::OperationOwner;

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

pub(super) fn try_snapshot<P, D>(
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
pub(super) fn validate_final_snapshot_shape<P: Copy + PartialEq, D>(
    state: &LedgerState<P, D>,
) -> Result<(), LedgerCause<D>> {
    if state.linear_work_faulted {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    if state.active_linear_work_permits != 0 {
        return Err(LedgerCause::OutstandingLinearWorkPermits {
            count: state.active_linear_work_permits,
        });
    }
    if state.dimensions.len() != state.dimension_snapshot_staging.len()
        || !phase_catalog_shape_is_valid(state)
        || state
            .dimensions
            .iter()
            .any(|entry| entry.state.pending_consumed != 0)
    {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(super) fn typed_terminal_snapshot_positions_are_valid<P, D>(state: &LedgerState<P, D>) -> bool {
    let phase_count = state.phases.len();
    !state.linear_work_faulted
        && state.active_linear_work_permits == 0
        && state
            .dimensions
            .iter()
            .all(|entry| entry.state.pending_consumed == 0)
        && state.dimensions.len() == state.dimension_snapshot_staging.len()
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

pub(super) fn phase_catalog_shape_is_valid<P: Copy + PartialEq, D>(
    state: &LedgerState<P, D>,
) -> bool {
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

pub(super) fn take_final_snapshot<P, D>(
    state: &mut LedgerState<P, D>,
    clean: bool,
) -> LedgerSnapshot<P, D>
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
pub(super) fn take_typed_final_snapshot<P: Copy, D: Copy>(
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

/// Moves the pre-admitted terminal staging into an immutable final projection without changing
/// the still-live ledger counters that back physical publication eligibility.
///
/// Qualification has already proved every positioned sum and destination limit. The projection
/// therefore performs no lookup, allocation, checked arithmetic, or owner-defined comparison.
pub(super) fn take_typed_terminal_projection<O, P, D>(
    state: &mut LedgerState<P, D>,
    last_completed_phase: Option<P>,
    canonical: &AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence: &AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
) -> LedgerSnapshot<P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    for (index, (entry, staging)) in state
        .dimensions
        .iter()
        .zip(state.dimension_snapshot_staging.iter_mut())
        .enumerate()
    {
        let canonical_increment = canonical.qualified_commitment_amount(index);
        let evidence_increment = evidence.qualified_commitment_amount(index);
        staging.0 = entry.dimension;
        staging.1 = DimensionSnapshot::from_terminal_evidence(DimensionSnapshotScalars {
            attempted: entry.state.attempted,
            consumed: entry.state.consumed,
            current_live_bytes: 0,
            peak_live_bytes: entry.state.peak_live_bytes,
            pending_publication: entry.state.pending_publication,
            committed: entry.state.committed + canonical_increment,
            committed_canonical_output: entry.state.committed_canonical_output
                + canonical_increment,
            published_outcome_evidence: entry.state.published_outcome_evidence + evidence_increment,
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
        current_evidence_bytes: 0,
        peak_evidence_bytes: state.peak_evidence_bytes,
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
