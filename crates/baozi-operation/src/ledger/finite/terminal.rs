use super::super::snapshot::SnapshotRows;
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FiniteLedgerSnapshot<P, D> {
    dimensions: [(D, DimensionSnapshot); FINITE_DIMENSION_COUNT],
    phases: [(P, PhaseSnapshot); FINITE_PHASE_COUNT],
    last_completed_phase: Option<P>,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_administrative_bytes: u64,
    peak_administrative_bytes: u64,
    covered_administrative_bytes: Option<u64>,
}

impl<P: Copy + Ord, D: Copy + Ord> FiniteLedgerSnapshot<P, D> {
    pub(crate) const fn last_completed_phase(&self) -> Option<P> {
        self.last_completed_phase
    }

    pub(crate) fn dimension(&self, dimension: D) -> Option<&DimensionSnapshot> {
        self.dimensions
            .binary_search_by_key(&dimension, |(key, _)| *key)
            .ok()
            .map(|position| &self.dimensions[position].1)
    }

    pub(crate) fn phase(&self, phase: P) -> Option<&PhaseSnapshot> {
        self.phases
            .binary_search_by_key(&phase, |(key, _)| *key)
            .ok()
            .map(|position| &self.phases[position].1)
    }

    pub(crate) const fn current_live_bytes(&self) -> u64 {
        self.current_live_bytes
    }

    pub(crate) const fn peak_live_bytes(&self) -> u64 {
        self.peak_live_bytes
    }

    pub(crate) const fn active_reservations(&self) -> u64 {
        self.active_reservations
    }

    pub(crate) const fn current_administrative_bytes(&self) -> u64 {
        self.current_administrative_bytes
    }

    pub(crate) const fn peak_administrative_bytes(&self) -> u64 {
        self.peak_administrative_bytes
    }

    pub(crate) const fn covered_administrative_bytes(&self) -> Option<u64> {
        self.covered_administrative_bytes
    }

    pub(crate) fn dimensions(&self) -> impl ExactSizeIterator<Item = (D, &DimensionSnapshot)> {
        self.dimensions
            .iter()
            .map(|(dimension, snapshot)| (*dimension, snapshot))
    }

    pub(crate) fn phases(&self) -> impl ExactSizeIterator<Item = (P, &PhaseSnapshot)> {
        self.phases
            .iter()
            .map(|(phase, snapshot)| (*phase, snapshot))
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> FiniteLedger<O, P, D> {
    pub(crate) fn take_terminal_snapshot(
        &mut self,
        progress_bytes: u64,
    ) -> (FiniteLedgerSnapshot<P, D>, Option<LedgerCause<D>>) {
        let known = self.known_administrative_bytes(progress_bytes);
        let cause = self.as_read().terminal_residue_cause();
        let selected = self.storage.read();
        let rows = SnapshotRows {
            storage: selected.view(),
            phases: selected.phases(),
            work_rows: &self.work,
            work_header: &self.work_header,
        };
        let cause = cause.or_else(|| {
            (known != Some(rows.storage.current_administrative_bytes))
                .then_some(LedgerCause::FinalSnapshotShapeMismatch)
        });
        rows.write_current(
            &mut self.dimension_snapshot_staging,
            &mut self.phase_snapshot_staging,
        );
        (
            FiniteLedgerSnapshot {
                dimensions: self.dimension_snapshot_staging,
                phases: self.phase_snapshot_staging,
                last_completed_phase: rows
                    .storage
                    .last_completed_phase
                    .map(|completed| completed.phase),
                current_live_bytes: rows.storage.current_live_bytes,
                peak_live_bytes: rows.storage.peak_live_bytes,
                active_reservations: rows.storage.active_reservations,
                current_administrative_bytes: rows.storage.current_administrative_bytes,
                peak_administrative_bytes: rows.storage.peak_administrative_bytes,
                covered_administrative_bytes: known
                    .filter(|bytes| *bytes == rows.storage.current_administrative_bytes),
            },
            cause,
        )
    }

    pub(crate) fn take_typed_terminal_projection(
        &mut self,
        last_completed_phase: Option<P>,
        canonical: &AttributionReservationBatch<O, P, D, CanonicalOutput>,
        evidence: &AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        known_administrative_bytes: Option<u64>,
    ) -> FiniteLedgerSnapshot<P, D> {
        let selected = self.storage.read();
        let rows = SnapshotRows {
            storage: selected.view(),
            phases: selected.phases(),
            work_rows: &self.work,
            work_header: &self.work_header,
        };
        rows.write_terminal(
            &mut self.dimension_snapshot_staging,
            &mut self.phase_snapshot_staging,
            canonical,
            evidence,
        );
        FiniteLedgerSnapshot {
            dimensions: self.dimension_snapshot_staging,
            phases: self.phase_snapshot_staging,
            last_completed_phase,
            current_live_bytes: 0,
            peak_live_bytes: rows.storage.peak_live_bytes,
            active_reservations: 0,
            current_administrative_bytes: rows.storage.current_administrative_bytes,
            peak_administrative_bytes: rows.storage.peak_administrative_bytes,
            covered_administrative_bytes: known_administrative_bytes
                .filter(|bytes| *bytes == rows.storage.current_administrative_bytes),
        }
    }
}

#[cfg(test)]
mod tests;
