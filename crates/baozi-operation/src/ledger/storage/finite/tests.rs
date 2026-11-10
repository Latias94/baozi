use super::*;
use crate::ledger::{
    CompletedPhase, PublicationState, StorageDimensionState, StoragePolicy, capacity_bytes,
    test_evidence_allocation_failure_is_armed, test_evidence_allocation_guard,
    test_fail_next_evidence_allocation, test_fail_next_evidence_over_envelope,
    test_set_evidence_allocation_failure,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};

const BASE_BYTES: u64 = 128;

fn header(limit: u64) -> LedgerHeader<u8> {
    LedgerHeader {
        current_live_bytes: 21,
        peak_live_bytes: 34,
        active_reservations: 7,
        retained_output_bytes: 5,
        retained_output_reservations: 2,
        evidence_byte_limit: limit,
        current_administrative_bytes: BASE_BYTES,
        peak_administrative_bytes: BASE_BYTES + 17,
        publication: PublicationState::Pending,
        last_completed_phase: Some(CompletedPhase {
            phase: 20,
            state_position: 1,
        }),
    }
}

fn state(limit: u64) -> FiniteStorageState<u8, u8> {
    let dimensions = std::array::from_fn(|position| DimensionEntry {
        dimension: position as u8,
        policy: StoragePolicy {
            committed_limit: 100 + position as u64,
            published_outcome_evidence_limit: 50 + position as u64,
            live_byte_limit: Some(1_024 + position as u64),
        },
        state: StorageDimensionState {
            current_live_bytes: position as u64 + 1,
            peak_live_bytes: position as u64 + 2,
            pending_publication: position as u64 + 3,
            committed: position as u64 + 4,
            committed_canonical_output: position as u64 + 5,
            published_outcome_evidence: position as u64 + 6,
            retained_output_bytes: position as u64,
        },
    });
    FiniteStorageState::new(
        dimensions,
        [
            PhaseEntry {
                phase: 30,
                state_position: 2,
            },
            PhaseEntry {
                phase: 10,
                state_position: 0,
            },
            PhaseEntry {
                phase: 20,
                state_position: 1,
            },
        ],
        header(limit),
    )
}

fn inline(storage: &FiniteStorage<u8, u8>) -> &FiniteStorageState<u8, u8> {
    match storage {
        FiniteStorage::Inline(Some(state)) => state,
        _ => panic!("the actual finite source must remain installed"),
    }
}

fn backings(state: &FiniteStorageState<u8, u8>) -> [(usize, usize); 3] {
    [
        (state.dimensions.as_ptr() as usize, state.dimensions.len()),
        (state.phases.as_ptr() as usize, state.phases.len()),
        (
            state.phase_states.as_ptr() as usize,
            state.phase_states.len(),
        ),
    ]
}

fn shared_capacity_bytes<P, D>(state: &LedgerState<P, D>) -> u64 {
    u64::try_from(size_of::<Mutex<Option<LedgerState<P, D>>>>()).unwrap()
        + capacity_bytes(&state.dimensions).unwrap()
        + capacity_bytes(&state.phases).unwrap()
        + capacity_bytes(&state.phase_states).unwrap()
        + capacity_bytes(&state.dimension_snapshot_staging).unwrap()
        + capacity_bytes(&state.phase_snapshot_staging).unwrap()
}

#[test]
fn fixed_backings_live_inside_the_actual_owner_and_views_borrow_them() {
    let mut storage = FiniteStorage::Inline(Some(state(u64::MAX)));
    let original = inline(&storage);
    let start = std::ptr::from_ref(original) as usize;
    let end = start + size_of::<FiniteStorageState<u8, u8>>();
    let element_sizes = [
        size_of::<DimensionEntry<u8>>(),
        size_of::<PhaseEntry<u8>>(),
        size_of::<PhaseState>(),
    ];
    for ((address, length), element_size) in backings(original).into_iter().zip(element_sizes) {
        assert!(address >= start);
        assert!(address + length * element_size <= end);
    }
    let read = storage.read();
    assert!(std::ptr::eq(read.view().header, &original.header));
    assert_eq!(
        read.view().dimensions.as_ptr(),
        original.dimensions.as_ptr()
    );
    assert_eq!(
        read.view().phase_states.as_ptr(),
        original.phase_states.as_ptr()
    );
    assert_eq!(read.phases().as_ptr(), original.phases.as_ptr());
    assert_eq!(read.view().current_administrative_bytes, BASE_BYTES);
    assert_eq!(read.view().peak_administrative_bytes, BASE_BYTES + 17);
    for state in read.view().phase_states {
        assert_eq!(state.current_live_bytes, 0);
        assert_eq!(state.peak_live_bytes, 0);
        assert_eq!(state.retained_output_bytes, 0);
    }
    drop(read);
    {
        let mut write = storage.write();
        let view = write.view_mut();
        view.header.current_live_bytes = 11;
        view.dimensions[4].state.committed = 19;
        view.phase_states[2].retained_output_bytes = 3;
        assert_eq!(write.view().current_live_bytes, 11);
    }
    assert_eq!(inline(&storage).header.current_live_bytes, 11);
    assert_eq!(inline(&storage).dimensions[4].state.committed, 19);
    assert_eq!(inline(&storage).phase_states[2].retained_output_bytes, 3);
    assert!(storage.shared().is_none());
}

#[test]
fn first_promotion_moves_current_values_and_charges_known_payload_and_three_capacities_once() {
    let mut source = state(u64::MAX);
    source.dimensions[2].policy.live_byte_limit = Some(7);
    source.phase_states = std::array::from_fn(|position| PhaseState {
        current_live_bytes: position as u64 + 2,
        peak_live_bytes: position as u64 + 4,
        retained_output_bytes: position as u64 + 1,
    });
    let dimensions: [_; FINITE_DIMENSION_COUNT] = std::array::from_fn(|position| {
        let entry = &source.dimensions[position];
        (entry.dimension, entry.policy, entry.state)
    });
    let phase_states = source.phase_states;
    let completed = source.header.last_completed_phase;
    let plan = FinitePromotionPlan::new(&source).unwrap();
    source.header.evidence_byte_limit = BASE_BYTES + plan.envelope_bytes;
    let mut storage = FiniteStorage::Inline(Some(source));

    storage.ensure_shared().unwrap();

    assert_eq!(Arc::strong_count(storage.shared().unwrap()), 1);
    assert_eq!(Arc::weak_count(storage.shared().unwrap()), 0);
    let read = storage.read();
    let FiniteStorageRead::Shared(state) = &read else {
        panic!("the real source must have moved to Shared");
    };
    let actual_bytes = shared_capacity_bytes(state);
    assert!(actual_bytes <= plan.envelope_bytes);
    assert_eq!(
        state.header.current_administrative_bytes,
        BASE_BYTES + actual_bytes
    );
    assert_eq!(
        state.header.peak_administrative_bytes,
        BASE_BYTES + plan.envelope_bytes
    );
    assert_eq!(
        state.header.evidence_byte_limit,
        BASE_BYTES + plan.envelope_bytes
    );
    assert_eq!(state.header.current_live_bytes, 21);
    assert_eq!(state.header.peak_live_bytes, 34);
    assert_eq!(state.header.active_reservations, 7);
    assert_eq!(state.header.retained_output_bytes, 5);
    assert_eq!(state.header.retained_output_reservations, 2);
    assert_eq!(state.header.publication, PublicationState::Pending);
    assert_eq!(state.header.last_completed_phase, completed);
    assert_eq!(state.dimensions.len(), FINITE_DIMENSION_COUNT);
    assert_eq!(state.phases.len(), FINITE_PHASE_COUNT);
    for (entry, (dimension, policy, values)) in state.dimensions.iter().zip(dimensions) {
        assert_eq!(entry.dimension, dimension);
        assert_eq!(entry.policy, policy);
        assert_eq!(entry.state, values);
    }
    for (actual, expected) in state.phase_states.iter().zip(phase_states) {
        assert_eq!(actual.current_live_bytes, expected.current_live_bytes);
        assert_eq!(actual.peak_live_bytes, expected.peak_live_bytes);
        assert_eq!(actual.retained_output_bytes, expected.retained_output_bytes);
    }
    assert!(state.dimension_snapshot_staging.is_empty());
    assert_eq!(state.dimension_snapshot_staging.capacity(), 0);
    assert!(state.phase_snapshot_staging.is_empty());
    assert_eq!(state.phase_snapshot_staging.capacity(), 0);
    let phases: [_; FINITE_PHASE_COUNT] =
        std::array::from_fn(|position| read.phases()[position].phase);
    assert_eq!(phases, [30, 10, 20]);
    let positions: [_; FINITE_PHASE_COUNT] =
        std::array::from_fn(|position| read.phases()[position].state_position);
    assert_eq!(positions, [2, 0, 1]);
}

#[test]
fn one_byte_denial_precedes_allocation_and_preserves_actual_source() {
    let _guard = test_evidence_allocation_guard();
    let mut source = state(u64::MAX);
    let plan = FinitePromotionPlan::new(&source).unwrap();
    let limit = BASE_BYTES + plan.envelope_bytes - 1;
    source.header.evidence_byte_limit = limit;
    let mut storage = FiniteStorage::Inline(Some(source));
    let before = backings(inline(&storage));
    test_fail_next_evidence_allocation();

    assert_eq!(
        storage.ensure_shared().unwrap_err(),
        LedgerEvidenceError::LimitExceeded {
            purpose: LedgerEvidencePurpose::SharedStorage,
            value: BASE_BYTES + plan.envelope_bytes,
            limit,
        }
    );

    assert!(test_evidence_allocation_failure_is_armed());
    test_set_evidence_allocation_failure(false);
    assert_eq!(backings(inline(&storage)), before);
    assert_eq!(
        inline(&storage).header.current_administrative_bytes,
        BASE_BYTES
    );
    assert_eq!(
        inline(&storage).header.peak_administrative_bytes,
        BASE_BYTES + 17
    );
    assert_eq!(inline(&storage).dimensions[4].state.committed, 8);
    assert!(storage.shared().is_none());
}

#[test]
fn admission_overflow_leaves_the_actual_header_unchanged() {
    let _guard = test_evidence_allocation_guard();
    let mut source = state(u64::MAX);
    source.header.current_administrative_bytes = u64::MAX;
    source.header.peak_administrative_bytes = u64::MAX;
    let mut storage = FiniteStorage::Inline(Some(source));
    let before = backings(inline(&storage));
    test_fail_next_evidence_allocation();

    assert_eq!(
        storage.ensure_shared().unwrap_err(),
        promotion_capacity_error(CapacityPlanError::CapacityOverflow)
    );

    assert!(test_evidence_allocation_failure_is_armed());
    test_set_evidence_allocation_failure(false);
    assert_eq!(backings(inline(&storage)), before);
    assert_eq!(
        inline(&storage).header.current_administrative_bytes,
        u64::MAX
    );
    assert_eq!(inline(&storage).header.peak_administrative_bytes, u64::MAX);
}

#[test]
fn missing_source_and_actual_phase_shape_faults_precede_admission() {
    let _guard = test_evidence_allocation_guard();
    let mut missing: FiniteStorage<u8, u8> = FiniteStorage::Inline(None);
    assert_eq!(
        missing.ensure_shared().unwrap_err(),
        LedgerEvidenceError::SnapshotShapeMismatch
    );
    for fault in 0..3 {
        let mut source = state(0);
        match fault {
            0 => source.phases[1].state_position = source.phases[0].state_position,
            1 => source.phases[2].state_position = FINITE_PHASE_COUNT,
            _ => {
                source
                    .header
                    .last_completed_phase
                    .as_mut()
                    .unwrap()
                    .state_position = 3
            }
        }
        let mut storage = FiniteStorage::Inline(Some(source));
        let before = backings(inline(&storage));
        test_fail_next_evidence_allocation();
        assert_eq!(
            storage.ensure_shared().unwrap_err(),
            LedgerEvidenceError::SnapshotShapeMismatch
        );
        assert!(test_evidence_allocation_failure_is_armed());
        test_set_evidence_allocation_failure(false);
        assert_eq!(backings(inline(&storage)), before);
        assert_eq!(
            inline(&storage).header.current_administrative_bytes,
            BASE_BYTES
        );
        assert_eq!(
            inline(&storage).header.peak_administrative_bytes,
            BASE_BYTES + 17
        );
    }
}

#[test]
fn allocation_and_envelope_errors_leave_arrays_installed_and_keep_admitted_peak() {
    let _guard = test_evidence_allocation_guard();
    for over_envelope in [false, true] {
        let mut storage = FiniteStorage::Inline(Some(state(u64::MAX)));
        let plan = FinitePromotionPlan::new(inline(&storage)).unwrap();
        let before = backings(inline(&storage));
        if over_envelope {
            test_fail_next_evidence_over_envelope();
        } else {
            test_fail_next_evidence_allocation();
        }
        let error = storage.ensure_shared().unwrap_err();
        if over_envelope {
            assert!(matches!(
                error,
                LedgerEvidenceError::Capacity {
                    purpose: LedgerEvidencePurpose::SharedStorage,
                    cause: CapacityPlanError::CapacityEnvelopeViolated { .. },
                }
            ));
        } else {
            assert_eq!(
                error,
                LedgerEvidenceError::AllocationFailed {
                    purpose: LedgerEvidencePurpose::SharedStorage,
                    requested_bytes: plan.payload_bytes,
                }
            );
        }
        assert_eq!(backings(inline(&storage)), before);
        assert_eq!(
            inline(&storage).header.current_administrative_bytes,
            BASE_BYTES
        );
        assert_eq!(
            inline(&storage).header.peak_administrative_bytes,
            BASE_BYTES + plan.envelope_bytes
        );
        assert_eq!(
            inline(&storage).header.publication,
            PublicationState::Pending
        );
        assert_eq!(inline(&storage).header.active_reservations, 7);
    }
}

#[test]
fn an_existing_higher_peak_survives_success_and_allocation_rejection() {
    let _guard = test_evidence_allocation_guard();
    for reject in [false, true] {
        let mut source = state(u64::MAX);
        source.header.peak_administrative_bytes = u64::MAX;
        let mut storage = FiniteStorage::Inline(Some(source));
        if reject {
            test_fail_next_evidence_allocation();
            assert!(storage.ensure_shared().is_err());
            assert_eq!(
                inline(&storage).header.current_administrative_bytes,
                BASE_BYTES
            );
        } else {
            storage.ensure_shared().unwrap();
        }
        assert_eq!(storage.read().view().peak_administrative_bytes, u64::MAX);
    }
}

#[test]
fn partial_candidate_failure_refunds_only_its_credit_after_other_header_changes() {
    let _guard = test_evidence_allocation_guard();
    let mut source = Some(state(u64::MAX));
    let plan = FinitePromotionPlan::new(source.as_ref().unwrap()).unwrap();
    let before = backings(source.as_ref().unwrap());
    let mut candidate = FinitePromotionCandidate::new(&mut source, &plan).unwrap();
    allocate_promotion_vec(&mut candidate.dimensions, plan.dimensions).unwrap();
    let header = &mut candidate.admission.source.as_mut().unwrap().header;
    header.current_administrative_bytes += 29;
    header.peak_administrative_bytes = header
        .peak_administrative_bytes
        .max(header.current_administrative_bytes);
    header.current_administrative_bytes -= 17;
    let peak = header.peak_administrative_bytes;
    test_fail_next_evidence_allocation();
    assert!(matches!(
        allocate_promotion_vec(&mut candidate.phases, plan.phases),
        Err(LedgerEvidenceError::AllocationFailed {
            purpose: LedgerEvidencePurpose::SharedStorage,
            ..
        })
    ));
    assert!(candidate.admission.source.is_some());
    assert!(candidate.shared.lock().unwrap().is_none());
    drop(candidate);

    let source = source.as_ref().unwrap();
    assert_eq!(backings(source), before);
    assert_eq!(source.header.current_administrative_bytes, BASE_BYTES + 12);
    assert_eq!(source.header.peak_administrative_bytes, peak);
}

#[test]
fn each_destination_capacity_is_qualified_before_taking_the_source() {
    for omitted in 0..3 {
        let mut source = Some(state(u64::MAX));
        let plan = FinitePromotionPlan::new(source.as_ref().unwrap()).unwrap();
        let before = backings(source.as_ref().unwrap());
        let mut candidate = FinitePromotionCandidate::new(&mut source, &plan).unwrap();
        if omitted != 0 {
            allocate_promotion_vec(&mut candidate.dimensions, plan.dimensions).unwrap();
        }
        if omitted != 1 {
            allocate_promotion_vec(&mut candidate.phases, plan.phases).unwrap();
        }
        if omitted != 2 {
            allocate_promotion_vec(&mut candidate.phase_states, plan.phase_states).unwrap();
        }
        assert!(matches!(
            candidate.install(&plan),
            Err(LedgerEvidenceError::Capacity {
                purpose: LedgerEvidencePurpose::SharedStorage,
                cause: CapacityPlanError::CapacityEnvelopeViolated {
                    actual_capacity: 0,
                    ..
                },
            })
        ));
        let source = source.as_ref().unwrap();
        assert_eq!(backings(source), before);
        assert_eq!(source.header.current_administrative_bytes, BASE_BYTES);
        assert_eq!(
            source.header.peak_administrative_bytes,
            BASE_BYTES + plan.envelope_bytes
        );
    }
}

#[test]
fn installation_rechecks_current_source_shape_and_budget() {
    for corrupt_shape in [false, true] {
        let mut source = Some(state(u64::MAX));
        let plan = FinitePromotionPlan::new(source.as_ref().unwrap()).unwrap();
        let before = backings(source.as_ref().unwrap());
        let mut candidate = FinitePromotionCandidate::new(&mut source, &plan).unwrap();
        candidate.allocate(&plan).unwrap();
        let current = candidate.admission.source.as_mut().unwrap();
        let error = if corrupt_shape {
            current.phases[1].state_position = current.phases[0].state_position;
            LedgerEvidenceError::SnapshotShapeMismatch
        } else {
            current.header.evidence_byte_limit = current.header.current_administrative_bytes - 1;
            LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::SharedStorage,
                value: current.header.current_administrative_bytes,
                limit: current.header.evidence_byte_limit,
            }
        };
        assert_eq!(candidate.install(&plan).unwrap_err(), error);
        let source = source.as_ref().unwrap();
        assert_eq!(backings(source), before);
        assert_eq!(source.header.current_administrative_bytes, BASE_BYTES);
        assert_eq!(
            source.header.peak_administrative_bytes,
            BASE_BYTES + plan.envelope_bytes
        );
    }
}

#[test]
fn an_actual_poisoned_vacant_shell_is_rejected_before_source_move() {
    let mut source = Some(state(u64::MAX));
    let plan = FinitePromotionPlan::new(source.as_ref().unwrap()).unwrap();
    let before = backings(source.as_ref().unwrap());
    let mut candidate = FinitePromotionCandidate::new(&mut source, &plan).unwrap();
    candidate.allocate(&plan).unwrap();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let _guard = candidate.shared.lock().unwrap();
            panic!("poison the actual unissued promotion shell");
        }))
        .is_err()
    );

    assert_eq!(
        candidate.install(&plan).unwrap_err(),
        LedgerEvidenceError::SnapshotShapeMismatch
    );

    let source = source.as_ref().unwrap();
    assert_eq!(backings(source), before);
    assert_eq!(source.header.current_administrative_bytes, BASE_BYTES);
    assert_eq!(
        source.header.peak_administrative_bytes,
        BASE_BYTES + plan.envelope_bytes
    );
}

#[test]
fn shared_shell_aliases_and_occupied_destinations_are_rejected_before_source_move() {
    for fault in 0..3 {
        let mut source = Some(state(u64::MAX));
        let plan = FinitePromotionPlan::new(source.as_ref().unwrap()).unwrap();
        let before = backings(source.as_ref().unwrap());
        let mut candidate = FinitePromotionCandidate::new(&mut source, &plan).unwrap();
        candidate.allocate(&plan).unwrap();
        let strong = (fault == 0).then(|| Arc::clone(&candidate.shared));
        let weak = (fault == 1).then(|| Arc::downgrade(&candidate.shared));
        if fault == 2 {
            *candidate.shared.lock().unwrap() = Some(LedgerState {
                dimensions: Vec::new(),
                phases: Vec::new(),
                phase_states: Vec::new(),
                dimension_snapshot_staging: Vec::new(),
                phase_snapshot_staging: Vec::new(),
                header: header(0),
            });
        }

        assert_eq!(
            candidate.install(&plan).unwrap_err(),
            LedgerEvidenceError::SnapshotShapeMismatch
        );

        if let Some(shared) = strong {
            assert!(shared.lock().unwrap().is_none());
        }
        if let Some(weak) = weak {
            assert!(weak.upgrade().is_none());
        }
        let source = source.as_ref().unwrap();
        assert_eq!(backings(source), before);
        assert_eq!(source.header.current_administrative_bytes, BASE_BYTES);
        assert_eq!(
            source.header.peak_administrative_bytes,
            BASE_BYTES + plan.envelope_bytes
        );
        assert_eq!(source.header.active_reservations, 7);
        assert_eq!(source.header.current_live_bytes, 21);
    }
}

#[test]
fn candidate_unwind_preserves_source_and_refunds_only_its_own_admission() {
    let mut source = Some(state(u64::MAX));
    let plan = FinitePromotionPlan::new(source.as_ref().unwrap()).unwrap();
    let before = backings(source.as_ref().unwrap());
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let mut candidate = FinitePromotionCandidate::new(&mut source, &plan).unwrap();
            allocate_promotion_vec(&mut candidate.dimensions, plan.dimensions).unwrap();
            let header = &mut candidate.admission.source.as_mut().unwrap().header;
            header.current_administrative_bytes += 11;
            header.peak_administrative_bytes = header
                .peak_administrative_bytes
                .max(header.current_administrative_bytes);
            panic!("unwind with the real inline source and partial candidate alive");
        }))
        .is_err()
    );

    let source = source.as_ref().unwrap();
    assert_eq!(backings(source), before);
    assert_eq!(source.header.current_administrative_bytes, BASE_BYTES + 11);
    assert_eq!(
        source.header.peak_administrative_bytes,
        BASE_BYTES + plan.envelope_bytes + 11
    );
}

#[test]
fn later_shared_access_keeps_the_same_handle_and_does_not_replay_admission() {
    let _guard = test_evidence_allocation_guard();
    let mut storage = FiniteStorage::Inline(Some(state(u64::MAX)));
    let reference = storage.ensure_shared().unwrap() as *const SharedStorage<u8, u8>;
    let token = Arc::clone(storage.shared().unwrap());
    let before = storage.read().view().current_administrative_bytes;
    {
        let mut write = storage.write();
        let view = write.view_mut();
        view.header.evidence_byte_limit = 0;
        view.dimensions[3].state.committed = 37;
    }
    test_fail_next_evidence_allocation();

    assert_eq!(
        storage.ensure_shared().unwrap() as *const SharedStorage<u8, u8>,
        reference
    );

    assert!(test_evidence_allocation_failure_is_armed());
    test_set_evidence_allocation_failure(false);
    assert!(Arc::ptr_eq(storage.shared().unwrap(), &token));
    assert_eq!(storage.read().view().current_administrative_bytes, before);
    assert_eq!(storage.read().view().dimensions[3].state.committed, 37);
    drop(storage);
    let state = lock_storage(&token);
    assert_eq!(state.header.current_administrative_bytes, before);
    assert_eq!(state.dimensions[3].state.committed, 37);
}

#[derive(Debug)]
struct DropObservedKey {
    position: usize,
    drops: Arc<AtomicUsize>,
}

impl Drop for DropObservedKey {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn actual_source_keys_survive_rejection_promotion_and_the_original_owner_exit() {
    let _guard = test_evidence_allocation_guard();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut source = FiniteStorageState {
        dimensions: std::array::from_fn(|position| DimensionEntry {
            dimension: DropObservedKey {
                position,
                drops: Arc::clone(&drops),
            },
            policy: StoragePolicy {
                committed_limit: 100,
                published_outcome_evidence_limit: 50,
                live_byte_limit: Some(1_024),
            },
            state: StorageDimensionState::default(),
        }),
        phases: std::array::from_fn(|position| PhaseEntry {
            phase: position as u8,
            state_position: position,
        }),
        phase_states: [PhaseState::default(); FINITE_PHASE_COUNT],
        header: LedgerHeader {
            current_live_bytes: 0,
            peak_live_bytes: 0,
            active_reservations: 0,
            retained_output_bytes: 0,
            retained_output_reservations: 0,
            last_completed_phase: None,
            ..header(u64::MAX)
        },
    };
    let plan = FinitePromotionPlan::new(&source).unwrap();
    source.header.evidence_byte_limit = BASE_BYTES + plan.envelope_bytes - 1;
    let mut storage = FiniteStorage::Inline(Some(source));
    assert!(matches!(
        storage.ensure_shared(),
        Err(LedgerEvidenceError::LimitExceeded { .. })
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    storage.write().view_mut().header.evidence_byte_limit = u64::MAX;
    test_fail_next_evidence_allocation();
    assert!(matches!(
        storage.ensure_shared(),
        Err(LedgerEvidenceError::AllocationFailed { .. })
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    test_fail_next_evidence_over_envelope();
    assert!(matches!(
        storage.ensure_shared(),
        Err(LedgerEvidenceError::Capacity { .. })
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 0);

    let token = Arc::clone(storage.ensure_shared().unwrap());

    assert_eq!(drops.load(Ordering::SeqCst), 0);
    {
        let state = lock_storage(&token);
        for (position, entry) in state.dimensions.iter().enumerate() {
            assert_eq!(entry.dimension.position, position);
        }
        assert_eq!(state.dimension_snapshot_staging.capacity(), 0);
        assert_eq!(state.phase_snapshot_staging.capacity(), 0);
    }
    drop(storage);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(token);
    assert_eq!(drops.load(Ordering::SeqCst), FINITE_DIMENSION_COUNT);
}
