use super::super::work::{WorkDimensionState, WorkEntry, WorkPolicy};
use super::super::{
    DimensionEntry, PhaseEntry, PublicationState, StorageDimensionState, StoragePolicy,
    test_evidence_allocation_guard, test_fail_next_evidence_allocation,
    test_fail_next_evidence_over_envelope,
};
use super::*;
use std::sync::{Arc, Mutex};

const FROZEN_EVIDENCE: u64 = 4096;

type SnapshotFixture<K> = (Arc<Mutex<Option<LedgerState<K, K>>>>, Vec<K>, WorkState<K>);

fn fixture<K: Copy>(key: K) -> SnapshotFixture<K> {
    let storage = Arc::new(Mutex::new(Some(LedgerState {
        dimensions: vec![DimensionEntry {
            dimension: key,
            policy: StoragePolicy {
                committed_limit: u64::MAX,
                published_outcome_evidence_limit: u64::MAX,
                live_byte_limit: Some(u64::MAX),
            },
            state: StorageDimensionState {
                current_live_bytes: 7,
                peak_live_bytes: 9,
                committed: 2,
                ..StorageDimensionState::default()
            },
        }],
        phases: vec![PhaseEntry {
            phase: key,
            state_position: 0,
        }],
        phase_states: vec![PhaseState {
            current_live_bytes: 7,
            peak_live_bytes: 9,
            retained_output_bytes: 0,
        }],
        dimension_snapshot_staging: vec![(key, DimensionSnapshot::default())],
        phase_snapshot_staging: vec![(key, PhaseSnapshot::default())],
        header: super::super::LedgerHeader {
            current_live_bytes: 7,
            peak_live_bytes: 9,
            active_reservations: 1,
            retained_output_bytes: 0,
            retained_output_reservations: 0,
            evidence_byte_limit: u64::MAX,
            current_administrative_bytes: FROZEN_EVIDENCE,
            peak_administrative_bytes: FROZEN_EVIDENCE,
            publication: PublicationState::Open,
            last_completed_phase: None,
        },
    })));
    let catalog = vec![key];
    let work = WorkState::new(
        catalog.len(),
        vec![WorkEntry {
            dimension: key,
            policy: WorkPolicy {
                attempted_limit: 100,
                consumed_limit: 100,
            },
            state: WorkDimensionState {
                attempted: 11,
                consumed: 5,
                pending_consumed: 3,
            },
        }],
    );
    (storage, catalog, work)
}

fn try_snapshot<P: Copy + Ord, D: Copy + Ord>(
    storage: &Arc<Mutex<Option<LedgerState<P, D>>>>,
    catalog: &[D],
    work: &WorkState<D>,
) -> Result<LedgerSnapshot<P, D>, LedgerEvidenceError> {
    let mut selected = DynamicStorage::Shared(Arc::clone(storage));
    super::try_snapshot(&mut selected, catalog, work)
}

#[derive(Clone, Copy)]
struct NoComparison;

impl PartialEq for NoComparison {
    fn eq(&self, _: &Self) -> bool {
        panic!("snapshot capture must not compare owner keys");
    }
}

impl Eq for NoComparison {}

impl PartialOrd for NoComparison {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NoComparison {
    fn cmp(&self, _: &Self) -> std::cmp::Ordering {
        panic!("snapshot capture must not compare owner keys");
    }
}

#[test]
fn interim_joins_active_work_without_key_comparison_or_debug_bounds() {
    let (storage, catalog, mut work) = fixture(NoComparison);
    work.header.open_envelopes = 1;
    work.header.producer_open = true;
    let snapshot = try_snapshot(&storage, &catalog, &work).expect("active work remains observable");
    let dimension = snapshot.dimensions[0].1;
    assert_eq!(dimension.attempted(), 11);
    assert_eq!(dimension.consumed(), 5);
    assert_eq!(dimension.current_live_bytes(), 7);
    assert_eq!(dimension.committed(), 2);
    assert_eq!(snapshot.phases[0].1.current_live_bytes(), 7);
    assert_eq!(snapshot.current_administrative_bytes(), FROZEN_EVIDENCE);
    assert!(snapshot.peak_administrative_bytes() > FROZEN_EVIDENCE);
    assert_eq!(lock(&storage).current_administrative_bytes, FROZEN_EVIDENCE);
    assert_eq!(work.rows[0].state.pending_consumed, 3);
    assert_eq!(work.header.open_envelopes, 1);
    assert!(work.header.producer_open);
}

#[test]
fn allocation_failure_releases_only_owned_admission_after_intervening_changes() {
    let _guard = test_evidence_allocation_guard();
    let (storage, _catalog, _work) = fixture(0_u8);
    let mut selected = DynamicStorage::Shared(Arc::clone(&storage));
    let (mut assembly, dimension_plan, phase_plan) =
        SnapshotEvidenceAssembly::new(&mut selected).expect("snapshot admission");
    allocate_snapshot_vec(&mut assembly.dimensions, dimension_plan)
        .expect("first backing is allocated before the second fails");
    {
        let mut state = lock(&storage);
        state.current_administrative_bytes += 29;
        state.peak_administrative_bytes = state
            .peak_administrative_bytes
            .max(state.current_administrative_bytes);
        release_evidence(&mut state, 17);
    }
    test_fail_next_evidence_allocation();
    assert!(matches!(
        allocate_snapshot_vec(&mut assembly.phases, phase_plan),
        Err(LedgerEvidenceError::AllocationFailed {
            purpose: LedgerEvidencePurpose::Snapshot,
            ..
        })
    ));
    let peak = lock(&storage).peak_administrative_bytes;
    drop(assembly);
    let state = lock(&storage);
    assert_eq!(state.current_administrative_bytes, FROZEN_EVIDENCE + 12);
    assert_eq!(state.peak_administrative_bytes, peak);
}

#[test]
fn over_envelope_failure_releases_snapshot_backing() {
    let _guard = test_evidence_allocation_guard();
    let (storage, catalog, work) = fixture(0_u8);
    test_fail_next_evidence_over_envelope();
    assert!(matches!(
        try_snapshot(&storage, &catalog, &work),
        Err(LedgerEvidenceError::Capacity {
            purpose: LedgerEvidencePurpose::Snapshot,
            cause: CapacityPlanError::CapacityEnvelopeViolated { .. },
        })
    ));
    assert_eq!(lock(&storage).current_administrative_bytes, FROZEN_EVIDENCE);
}

#[test]
fn allocation_failure_precedes_work_fault_and_both_release_admission() {
    let _guard = test_evidence_allocation_guard();
    for shape_fault in [false, true] {
        let (storage, mut catalog, mut work) = fixture(0_u8);
        if shape_fault {
            catalog.clear();
        } else {
            work.header.integrity_fault = true;
        }
        test_fail_next_evidence_allocation();
        assert!(matches!(
            try_snapshot(&storage, &catalog, &work),
            Err(LedgerEvidenceError::AllocationFailed {
                purpose: LedgerEvidencePurpose::Snapshot,
                ..
            })
        ));
        assert_eq!(lock(&storage).current_administrative_bytes, FROZEN_EVIDENCE);
        assert!(matches!(
            try_snapshot(&storage, &catalog, &work),
            Err(LedgerEvidenceError::SnapshotShapeMismatch)
        ));
        let state = lock(&storage);
        assert_eq!(state.current_administrative_bytes, FROZEN_EVIDENCE);
        assert!(state.peak_administrative_bytes > FROZEN_EVIDENCE);
        assert_eq!(state.current_live_bytes, 7);
        assert_eq!(state.active_reservations, 1);
    }
}

#[test]
fn terminal_checks_reject_pending_and_completed_unfinished_channels() {
    let (storage, _catalog, mut work) = fixture(0_u8);
    let mut state = lock(&storage);
    assert!(!typed_terminal_snapshot_positions_are_valid(&state, &work));
    work.rows[0].state.pending_consumed = 0;
    work.header.parser_parent_open = true;
    assert!(!typed_terminal_snapshot_positions_are_valid(&state, &work));
    assert_eq!(
        validate_final_snapshot_shape(&state, &work),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    work.header.parser_parent_open = false;
    assert!(typed_terminal_snapshot_positions_are_valid(&state, &work));
    let snapshot = take_typed_final_snapshot(&mut state, &work, None);
    drop(state);
    assert_eq!(snapshot.dimensions[0].1.consumed(), 5);
    assert_eq!(snapshot.current_administrative_bytes(), FROZEN_EVIDENCE);
    assert_eq!(snapshot.peak_administrative_bytes(), FROZEN_EVIDENCE);
}
