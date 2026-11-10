use super::*;

struct Owner;

impl OperationOwner for Owner {}

fn policies() -> [(u8, DimensionPolicy); FINITE_DIMENSION_COUNT] {
    [6, 2, 5, 1, 4, 3]
        .map(|dimension| (dimension, DimensionPolicy::reservable_bytes(16, 16, 0, 64)))
}

fn ledger() -> FiniteLedger<Owner, u8, u8> {
    FiniteLedger::try_new(
        FiniteLedgerLimits::try_new(64, 4096, policies()).unwrap(),
        [3, 1, 2],
    )
    .unwrap()
}

#[test]
fn fixed_limits_and_initial_state_own_arrays_with_no_administrative_heap_charge() {
    let limits = FiniteLedgerLimits::try_new(64, 0, policies()).unwrap();
    assert_eq!(limits.dimensions(), &[1, 2, 3, 4, 5, 6]);
    for dimension in 1..=6 {
        assert_eq!(
            limits.dimension(dimension),
            Some(DimensionPolicy::reservable_bytes(16, 16, 0, 64))
        );
    }
    assert_eq!(limits.dimension(9), None);
    let ledger = FiniteLedger::<Owner, _, _>::try_new(limits, [3_u8, 1, 2]).unwrap();
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    assert!(ledger.storage.shared().is_none());
    assert_eq!(ledger.operation_live_byte_limit, 64);
    assert_eq!(ledger.phases.map(|entry| entry.phase), [1, 2, 3]);
    assert_eq!(ledger.phases.map(|entry| entry.state_position), [1, 2, 0]);
    let state = ledger.storage.read();
    let view = state.view();
    assert_eq!(view.current_administrative_bytes, 0);
    assert_eq!(view.peak_administrative_bytes, 0);
    assert_eq!(view.current_live_bytes, 0);
    assert_eq!(view.active_reservations, 0);
    assert_eq!(
        ledger.dimension_snapshot_staging.map(|row| row.0),
        [1, 2, 3, 4, 5, 6]
    );
    assert!(
        ledger
            .dimension_snapshot_staging
            .iter()
            .all(|row| row.1 == DimensionSnapshot::default())
    );
    assert_eq!(ledger.phase_snapshot_staging.map(|row| row.0), [1, 2, 3]);
    assert!(
        ledger
            .phase_snapshot_staging
            .iter()
            .all(|row| row.1 == PhaseSnapshot::default())
    );
}

#[test]
fn fixed_limits_preserve_input_error_order_before_sorting() {
    let mut invalid = policies();
    invalid[1] = (invalid[0].0, DimensionPolicy::counter(1, 2, 0));
    assert!(matches!(
        FiniteLedgerLimits::try_new(64, 0, invalid),
        Err(LedgerLimitsError::ConsumedLimitExceedsAttempted {
            dimension: 6,
            consumed_limit: 2,
            attempted_limit: 1,
        })
    ));
    invalid[1].1 = DimensionPolicy::counter(2, 2, 0);
    assert!(matches!(
        FiniteLedgerLimits::try_new(64, 0, invalid),
        Err(LedgerLimitsError::DuplicateDimension { dimension: 6 })
    ));
    assert!(matches!(
        FiniteLedger::<Owner, _, _>::try_new(
            FiniteLedgerLimits::try_new(64, 0, policies()).unwrap(),
            [1_u8, 2, 1]
        ),
        Err(LedgerCause::DuplicatePhase)
    ));
}

#[test]
fn fixed_work_moves_with_its_owner_and_uses_the_existing_completion_kernel() {
    let mut original = ledger();
    let identity = original.identity;
    let (layout, mut frame) = {
        let (mut internal, _) = original.view().into_internal();
        let layout = internal.bind_ordinary([1, 6]).unwrap();
        let mut work = internal.reborrow().unwrap();
        let frame = layout
            .admit_observed(&mut work.work, [3, 4], WorkLane::Internal)
            .unwrap();
        (layout, frame)
    };
    let mut moved = original;
    assert_eq!(moved.identity, identity);
    {
        let (mut internal, _) = moved.view().into_internal();
        {
            let mut work = internal.reborrow().unwrap();
            frame
                .complete_batch_with_outcome(&layout, &mut work.work, [1, 2])
                .into_result()
                .unwrap();
        }
        internal.resolve(layout, &mut frame, false).unwrap();
    }
    assert_eq!(
        (moved.work[0].state.attempted, moved.work[0].state.consumed),
        (3, 1)
    );
    assert_eq!(
        (moved.work[5].state.attempted, moved.work[5].state.consumed),
        (4, 2)
    );
    assert!(moved.work.iter().all(|row| row.state.pending_consumed == 0));
    assert_eq!(moved.work_header.open_envelopes, 0);
    assert_eq!(moved.known_administrative_bytes(0), Some(0));
}

#[test]
fn shared_promotion_preserves_the_actual_fixed_work_rows_and_counts_only_owned_heap() {
    let mut ledger = ledger();
    ledger.work[2].state = WorkDimensionState {
        attempted: 9,
        consumed: 4,
        pending_consumed: 2,
    };
    let rows = ledger.work.as_ptr();
    let identity = ledger.identity;
    let shared = Arc::clone(ledger.storage.ensure_shared().unwrap());
    assert_eq!(ledger.identity, identity);
    assert_eq!(ledger.work.as_ptr(), rows);
    assert_eq!(ledger.work[2].state.attempted, 9);
    assert_eq!(ledger.work[2].state.consumed, 4);
    assert_eq!(ledger.work[2].state.pending_consumed, 2);
    let actual = ledger.known_administrative_bytes(0).unwrap();
    let current = ledger.storage.read().view().current_administrative_bytes;
    assert_eq!(actual, current);
    assert!(Arc::ptr_eq(
        &shared,
        ledger.storage.ensure_shared().unwrap()
    ));
    assert_eq!(ledger.known_administrative_bytes(0), Some(actual));
    assert_eq!(
        ledger.storage.read().view().current_administrative_bytes,
        actual
    );
}

#[test]
fn snapshot_staging_stays_in_the_finite_root_when_storage_promotes() {
    let mut ledger = ledger();
    ledger.dimension_snapshot_staging[4].1 =
        DimensionSnapshot::from_terminal_evidence(DimensionSnapshotScalars {
            attempted: 101,
            consumed: 89,
            current_live_bytes: 3,
            peak_live_bytes: 9,
            pending_publication: 5,
            committed: 7,
            committed_canonical_output: 4,
            published_outcome_evidence: 2,
        });
    ledger.phase_snapshot_staging[1].1 = PhaseSnapshot::from_terminal_evidence(8, 13);
    let dimensions = ledger.dimension_snapshot_staging;
    let phases = ledger.phase_snapshot_staging;
    let dimension_slots = ledger.dimension_snapshot_staging.as_ptr();
    let phase_slots = ledger.phase_snapshot_staging.as_ptr();
    let start = std::ptr::from_ref(&ledger) as usize;
    let end = start + size_of_val(&ledger);
    assert!((start..end).contains(&(dimension_slots as usize)));
    assert!(dimension_slots as usize + size_of_val(&dimensions) <= end);
    assert!((start..end).contains(&(phase_slots as usize)));
    assert!(phase_slots as usize + size_of_val(&phases) <= end);

    let reservation = ledger.view().reserve(1, 6, 7).unwrap();

    assert_eq!(ledger.dimension_snapshot_staging.as_ptr(), dimension_slots);
    assert_eq!(ledger.phase_snapshot_staging.as_ptr(), phase_slots);
    assert_eq!(ledger.dimension_snapshot_staging, dimensions);
    assert_eq!(ledger.phase_snapshot_staging, phases);
    let shared = ledger.storage.shared().unwrap();
    let state = lock(shared);
    assert!(state.dimension_snapshot_staging.is_empty());
    assert_eq!(state.dimension_snapshot_staging.capacity(), 0);
    assert!(state.phase_snapshot_staging.is_empty());
    assert_eq!(state.phase_snapshot_staging.capacity(), 0);
    let actual = u64::try_from(size_of::<Mutex<Option<LedgerState<u8, u8>>>>()).unwrap()
        + capacity_bytes(&state.dimensions).unwrap()
        + capacity_bytes(&state.phases).unwrap()
        + capacity_bytes(&state.phase_states).unwrap();
    assert_eq!(state.current_administrative_bytes, actual);
    drop(state);
    assert_eq!(ledger.known_administrative_bytes(0), Some(actual));
    drop(reservation);
}

#[test]
fn first_reservation_keeps_numeric_rejection_ahead_of_shared_capacity_admission() {
    for (phase, dimension, bytes, expected) in [
        (9, 9, 1, LedgerCause::UnknownDimension { dimension: 9 }),
        (
            9,
            1,
            65,
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: 1,
                value: 65,
                limit: 64,
            },
        ),
        (9, 1, 1, LedgerCause::UnknownPhase),
    ] {
        let mut ledger = FiniteLedger::<Owner, _, _>::try_new(
            FiniteLedgerLimits::try_new(64, 0, policies()).unwrap(),
            [1_u8, 2, 3],
        )
        .unwrap();
        assert_eq!(
            ledger.view().reserve(phase, dimension, bytes).unwrap_err(),
            expected
        );
        assert!(ledger.storage.shared().is_none());
        let state = ledger.storage.read();
        let view = state.view();
        assert_eq!(view.current_live_bytes, 0);
        assert_eq!(view.active_reservations, 0);
        assert_eq!(view.current_administrative_bytes, 0);
        assert_eq!(view.peak_administrative_bytes, 0);
    }
}

#[test]
fn zero_byte_token_promotes_and_its_actual_credit_releases_without_a_new_owner() {
    let mut ledger = ledger();
    assert!(ledger.storage.shared().is_none());
    let zero = ledger.view().reserve(1, 1, 0).unwrap();
    let shared = Arc::clone(ledger.storage.shared().unwrap());
    assert!(Arc::ptr_eq(zero.core.owner.as_ref().unwrap(), &shared));
    assert_eq!(lock(&shared).active_reservations, 1);
    assert_eq!(lock(&shared).current_live_bytes, 0);
    let actual = ledger.known_administrative_bytes(0).unwrap();
    assert_eq!(lock(&shared).current_administrative_bytes, actual);
    let payload = ledger.view().reserve(2, 6, 7).unwrap();
    assert!(Arc::ptr_eq(payload.core.owner.as_ref().unwrap(), &shared));
    assert_eq!(lock(&shared).active_reservations, 2);
    assert_eq!(lock(&shared).current_live_bytes, 7);
    drop(zero);
    assert_eq!(lock(&shared).active_reservations, 1);
    assert_eq!(lock(&shared).current_live_bytes, 7);
    drop(payload);
    assert_eq!(lock(&shared).active_reservations, 0);
    assert_eq!(lock(&shared).current_live_bytes, 0);
    assert_eq!(lock(&shared).peak_live_bytes, 7);
    assert_eq!(lock(&shared).current_administrative_bytes, actual);
    assert_eq!(ledger.known_administrative_bytes(0), Some(actual));
}

#[test]
fn reservation_transfers_and_drop_follow_original_phase_positions_after_sorting() {
    let mut ledger = ledger();
    let mut reservation = ledger.view().reserve(1, 2, 7).unwrap();
    {
        let state = ledger.storage.read();
        let view = state.view();
        assert_eq!(view.dimensions[1].state.current_live_bytes, 7);
        assert_eq!(view.dimensions[1].state.peak_live_bytes, 7);
        assert_eq!(view.phase_states[1].current_live_bytes, 7);
        assert_eq!(view.phase_states[1].peak_live_bytes, 7);
        assert_eq!(view.phase_states[0].current_live_bytes, 0);
        assert_eq!(view.phase_states[2].current_live_bytes, 0);
    }
    {
        let (_, storage) = ledger.view().into_internal();
        storage.transition(&mut reservation, 3, 6, 11).unwrap();
    }
    {
        let state = ledger.storage.read();
        let view = state.view();
        assert_eq!(
            (
                view.current_live_bytes,
                view.peak_live_bytes,
                view.active_reservations
            ),
            (11, 11, 1)
        );
        assert_eq!(view.dimensions[1].state.current_live_bytes, 0);
        assert_eq!(view.dimensions[1].state.peak_live_bytes, 7);
        assert_eq!(view.dimensions[5].state.current_live_bytes, 11);
        assert_eq!(view.dimensions[5].state.peak_live_bytes, 11);
        assert_eq!(view.phase_states[1].current_live_bytes, 0);
        assert_eq!(view.phase_states[1].peak_live_bytes, 7);
        assert_eq!(view.phase_states[0].current_live_bytes, 11);
        assert_eq!(view.phase_states[0].peak_live_bytes, 11);
        assert_eq!(view.phase_states[2].peak_live_bytes, 0);
    }
    {
        let (_, storage) = ledger.view().into_internal();
        storage.resize(&mut reservation, 5).unwrap();
    }
    drop(reservation);
    let state = ledger.storage.read();
    let view = state.view();
    assert_eq!(
        (
            view.current_live_bytes,
            view.peak_live_bytes,
            view.active_reservations
        ),
        (0, 11, 0)
    );
    assert!(
        view.dimensions
            .iter()
            .all(|row| row.state.current_live_bytes == 0)
    );
    assert!(
        view.phase_states
            .iter()
            .all(|row| row.current_live_bytes == 0)
    );
    assert_eq!(view.dimensions[1].state.peak_live_bytes, 7);
    assert_eq!(view.dimensions[5].state.peak_live_bytes, 11);
    assert_eq!(view.phase_states[1].peak_live_bytes, 7);
    assert_eq!(view.phase_states[0].peak_live_bytes, 11);
}

#[test]
fn split_services_use_current_tightened_policies_before_and_after_promotion() {
    let mut ledger = ledger();
    {
        let (mut internal, mut storage) = ledger.view().into_internal();
        let mut work = internal.reborrow().unwrap();
        work.tighten_policy(
            &mut storage,
            2,
            DimensionPolicy::reservable_bytes(8, 8, 0, 9),
        )
        .unwrap();
        assert_eq!(
            storage.reserve(1, 2, 10).unwrap_err(),
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: 2,
                value: 10,
                limit: 9
            }
        );
    }
    assert!(ledger.storage.shared().is_none());
    let mut reservation = ledger.view().reserve(1, 2, 6).unwrap();
    let owner = Arc::clone(ledger.storage.shared().unwrap());
    {
        let (mut internal, mut storage) = ledger.view().into_internal();
        let mut work = internal.reborrow().unwrap();
        work.tighten_policy(
            &mut storage,
            2,
            DimensionPolicy::reservable_bytes(4, 4, 0, 7),
        )
        .unwrap();
        work.tighten_policy(
            &mut storage,
            6,
            DimensionPolicy::reservable_bytes(4, 4, 0, 5),
        )
        .unwrap();
        assert_eq!(
            storage.resize(&mut reservation, 8),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: 2,
                value: 8,
                limit: 7
            })
        );
        assert_eq!(
            storage.transition(&mut reservation, 3, 6, 6),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: 6,
                value: 6,
                limit: 5
            })
        );
        let layout = work.bind([2]).unwrap();
        assert_eq!(
            work.charge_attempted(layout, [5]),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: 2,
                value: 5,
                limit: 4
            })
        );
        storage.transition(&mut reservation, 3, 6, 5).unwrap();
    }
    assert!(Arc::ptr_eq(ledger.storage.shared().unwrap(), &owner));
    assert_eq!(ledger.work[1].state.attempted, 5);
    assert_eq!(ledger.work[1].state.consumed, 0);
    assert_eq!(ledger.work[1].policy.attempted_limit, 4);
    assert_eq!(lock(&owner).dimensions[1].policy.live_byte_limit, Some(7));
    assert_eq!(lock(&owner).dimensions[5].policy.live_byte_limit, Some(5));
    drop(reservation);
    assert_eq!(lock(&owner).current_live_bytes, 0);
    assert_eq!(lock(&owner).active_reservations, 0);
}

#[test]
fn denied_zero_byte_token_keeps_the_real_inline_owner_and_all_counters_unchanged() {
    let mut ledger = FiniteLedger::<Owner, _, _>::try_new(
        FiniteLedgerLimits::try_new(64, 0, policies()).unwrap(),
        [1_u8, 2, 3],
    )
    .unwrap();
    assert!(matches!(
        ledger.view().reserve(1, 1, 0),
        Err(LedgerCause::Evidence {
            cause: LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::SharedStorage,
                limit: 0,
                ..
            }
        })
    ));
    assert!(ledger.storage.shared().is_none());
    let state = ledger.storage.read();
    let view = state.view();
    assert_eq!(view.current_live_bytes, 0);
    assert_eq!(view.active_reservations, 0);
    assert_eq!(view.current_administrative_bytes, 0);
    assert_eq!(view.peak_administrative_bytes, 0);
    assert!(
        ledger
            .work
            .iter()
            .all(|row| row.state == WorkDimensionState::default())
    );
}

#[test]
fn finite_work_and_storage_use_disjoint_views_of_the_same_fixed_owner() {
    let mut ledger = ledger();
    let (mut internal, mut storage) = ledger.view().into_internal();
    let layout = internal.bind_ordinary([2]).unwrap();
    let mut frame = {
        let mut work = internal.reborrow().unwrap();
        layout
            .admit_observed(&mut work.work, [3], WorkLane::Internal)
            .unwrap()
    };
    let reservation = storage.reserve(3, 1, 7).unwrap();
    {
        let mut work = internal.reborrow().unwrap();
        frame
            .complete_batch_with_outcome(&layout, &mut work.work, [2])
            .into_result()
            .unwrap();
        let evidence = work.dimension_evidence(&storage, 2).unwrap();
        assert_eq!((evidence.attempted(), evidence.consumed()), (3, 2));
    }
    drop(reservation);
    internal.resolve(layout, &mut frame, false).unwrap();
    assert_eq!(storage.planning_state().current_live_bytes(), 0);
    assert_eq!(storage.planning_state().active_reservations(), 0);
    assert_eq!(internal.read_state().rows[1].state.pending_consumed, 0);
    assert_eq!(internal.read_state().header.open_envelopes, 0);
}

#[test]
fn common_lifecycle_refuses_a_zero_pending_open_frame_then_seals_the_actual_owner() {
    let mut ledger = ledger();
    let (layout, mut frame) = {
        let (mut work, _) = ledger.view().into_internal();
        let layout = work.bind_ordinary([1]).unwrap();
        let frame = layout
            .admit_observed(&mut work.reborrow().unwrap().work, [0], WorkLane::Internal)
            .unwrap();
        (layout, frame)
    };
    assert!(
        ledger
            .work
            .iter()
            .all(|row| row.state.pending_consumed == 0)
    );
    assert_eq!(
        ledger.view().seal_ordinary_work(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    assert_eq!(ledger.work_header.mode, WorkMode::Ordinary);
    assert_eq!(
        ledger.storage.read().view().publication,
        PublicationState::Open
    );
    ledger
        .view()
        .into_internal()
        .0
        .resolve(layout, &mut frame, false)
        .unwrap();
    {
        let mut view = ledger.view();
        view.seal_ordinary_work().unwrap();
        assert_eq!(
            view.ensure_work_open(),
            Err(LedgerCause::PublicationAlreadyPending)
        );
        view.close_work().unwrap();
        assert_eq!(
            view.ensure_work_open(),
            Err(LedgerCause::PublicationAlreadyCompleted)
        );
        view.check_work_quiescent().unwrap();
    }
    assert_eq!(ledger.work_header.mode, WorkMode::Closed);
    assert_eq!(
        ledger.storage.read().view().publication,
        PublicationState::Closing
    );
    assert!(ledger.storage.shared().is_none());
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
}

#[test]
fn common_snapshot_rows_use_fixed_staging_before_and_after_real_storage_promotion() {
    let mut ledger = ledger();
    ledger.work[1].state.attempted = 11;
    ledger.work[1].state.consumed = 5;
    let mut reservation = None;
    for promoted in [false, true] {
        if promoted {
            reservation = Some(ledger.view().reserve(3, 2, 7).unwrap());
        }
        let state = ledger.storage.read();
        let rows = snapshot::SnapshotRows {
            storage: state.view(),
            phases: state.phases(),
            work_rows: &ledger.work,
            work_header: &ledger.work_header,
        };
        rows.validate_final_shape(
            ledger.dimension_snapshot_staging.len(),
            ledger.phase_snapshot_staging.len(),
        )
        .unwrap();
        rows.write_current(
            &mut ledger.dimension_snapshot_staging,
            &mut ledger.phase_snapshot_staging,
        );
        let dimension = ledger.dimension_snapshot_staging[1];
        assert_eq!(dimension.0, 2);
        assert_eq!((dimension.1.attempted(), dimension.1.consumed()), (11, 5));
        assert_eq!(
            dimension.1.current_live_bytes(),
            if promoted { 7 } else { 0 }
        );
        assert_eq!(dimension.1.peak_live_bytes(), if promoted { 7 } else { 0 });
        assert_eq!(ledger.phase_snapshot_staging[2].0, 3);
        assert_eq!(
            ledger.phase_snapshot_staging[2].1.current_live_bytes(),
            if promoted { 7 } else { 0 }
        );
        assert_eq!(ledger.phase_snapshot_staging[0].1.current_live_bytes(), 0);
        assert_eq!(ledger.phase_snapshot_staging[1].1.current_live_bytes(), 0);
    }
    drop(reservation);
}

#[test]
fn common_snapshot_shape_rejects_each_unfinished_channel_even_with_zero_pending_rows() {
    for fault in 0..7 {
        let mut ledger = ledger();
        match fault {
            0 => ledger.work_header.integrity_fault = true,
            1 => ledger.work_header.open_envelopes = 2,
            2 => ledger.work_header.producer_open = true,
            3 => ledger.work_header.parser_parent_open = true,
            4 => ledger.work_header.parser_child_open = true,
            5 => ledger.work_header.protected_open = true,
            _ => ledger.work[1].state.pending_consumed = 1,
        }
        let state = ledger.storage.read();
        let rows = snapshot::SnapshotRows {
            storage: state.view(),
            phases: state.phases(),
            work_rows: &ledger.work,
            work_header: &ledger.work_header,
        };
        let expected = if fault == 0 {
            LedgerCause::FinalSnapshotShapeMismatch
        } else {
            LedgerCause::OutstandingLinearWorkPermits {
                count: if fault == 1 { 2 } else { 1 },
            }
        };
        assert_eq!(rows.validate_final_shape(6, 3), Err(expected));
        assert_eq!(
            ledger.dimension_snapshot_staging.map(|row| row.1),
            [DimensionSnapshot::default(); 6]
        );
        assert_eq!(
            ledger.phase_snapshot_staging.map(|row| row.1),
            [PhaseSnapshot::default(); 3]
        );
    }
}

#[test]
fn tokenless_batch_keeps_finite_identity_without_promoting_or_consuming_allocator_failpoint() {
    let _guard = test_evidence_allocation_guard();
    let foreign = ledger();
    let mut ledger = ledger();
    test_fail_next_evidence_allocation();
    let batch = ledger
        .view()
        .try_attribution_batch_from_plan(
            AttributionBatchPlan::<Owner, _, _, CanonicalOutput>::try_new(3, 1, 0, 0).unwrap(),
        )
        .unwrap();
    assert!(test_evidence_allocation_failure_is_armed());
    test_set_evidence_allocation_failure(false);
    assert!(batch.validate_owner_view(&ledger.as_read()));
    assert!(!batch.validate_owner_view(&foreign.as_read()));
    assert!(ledger.storage.shared().is_none());
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
    let token = ledger.view().reserve(1, 1, 0).unwrap();
    assert!(batch.validate_owner_view(&ledger.view().as_read()));
    assert_eq!(
        ledger.as_read().terminal_residue_cause(),
        Some(LedgerCause::ActiveReservationsRemain { count: 1 })
    );
    drop(token);
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
    drop(batch);
    assert_eq!(ledger.storage.read().view().active_reservations, 0);
}

#[test]
fn nonempty_batch_uses_the_finite_shared_identity_and_releases_its_real_capacity() {
    let mut ledger = FiniteLedger::<Owner, _, _>::try_new(
        FiniteLedgerLimits::try_new(
            16_384,
            4096,
            [1_u8, 2, 3, 4, 5, 6].map(|dimension| {
                (
                    dimension,
                    DimensionPolicy::reservable_bytes(16, 16, 16, 16_384),
                )
            }),
        )
        .unwrap(),
        [3_u8, 1, 2],
    )
    .unwrap();
    let batch = ledger
        .view()
        .try_attribution_batch_from_plan(
            AttributionBatchPlan::<Owner, _, _, CanonicalOutput>::try_new(3, 1, 1, 1).unwrap(),
        )
        .unwrap();
    let shared = Arc::clone(ledger.storage.shared().unwrap());
    let capacity = batch.account_capacity_bytes();
    assert!(capacity > 0);
    assert!(batch.validate_owner_view(&ledger.as_read()));
    assert_eq!(lock(&shared).current_live_bytes, capacity);
    assert_eq!(lock(&shared).active_reservations, 1);
    assert_eq!(lock(&shared).phase_states[0].current_live_bytes, capacity);
    assert_eq!(
        lock(&shared).dimensions[0].state.current_live_bytes,
        capacity
    );
    let known = ledger.known_administrative_bytes(0).unwrap();
    assert_eq!(lock(&shared).current_administrative_bytes, known);
    drop(batch);
    assert_eq!(lock(&shared).current_live_bytes, 0);
    assert_eq!(lock(&shared).active_reservations, 0);
    assert_eq!(lock(&shared).phase_states[0].current_live_bytes, 0);
    assert_eq!(lock(&shared).dimensions[0].state.current_live_bytes, 0);
    assert!(Arc::ptr_eq(ledger.storage.shared().unwrap(), &shared));
}

#[test]
fn fixed_terminal_projection_uses_real_positioned_batches_without_rewriting_live_counters() {
    let mut ledger = FiniteLedger::<Owner, _, _>::try_new(
        FiniteLedgerLimits::try_new(
            16_384,
            4096,
            [1_u8, 2, 3, 4, 5, 6].map(|dimension| {
                (
                    dimension,
                    DimensionPolicy::reservable_bytes(16, 16, 16, 16_384)
                        .with_published_outcome_evidence_limit(16),
                )
            }),
        )
        .unwrap(),
        [3_u8, 1, 2],
    )
    .unwrap();
    let mut canonical = ledger
        .view()
        .try_attribution_batch_from_plan(
            AttributionBatchPlan::<Owner, _, _, CanonicalOutput>::try_new(3, 1, 1, 1).unwrap(),
        )
        .unwrap();
    let mut evidence = ledger
        .view()
        .try_attribution_batch_from_plan(
            AttributionBatchPlan::<Owner, _, _, OutcomeEvidenceDestination>::try_new(1, 2, 1, 1)
                .unwrap(),
        )
        .unwrap();
    assert!(
        ledger
            .as_read()
            .try_push_commitment(&mut canonical, OutputCommitment::new(1, 3))
            .is_ok()
    );
    assert!(
        ledger
            .as_read()
            .try_push_commitment(&mut evidence, OutputCommitment::new(2, 2))
            .is_ok()
    );
    ledger.work[0].state.attempted = 11;
    ledger.work[0].state.consumed = 5;
    {
        let mut storage = ledger.storage.write();
        let view = storage.view_mut();
        view.dimensions[0].state.committed = 2;
        view.dimensions[0].state.committed_canonical_output = 1;
    }
    ledger.view().seal_ordinary_work().unwrap();
    ledger.view().close_work().unwrap();
    let read = ledger.storage.read();
    let storage = read.view();
    assert!(canonical.validate_owner_view(&ledger.as_read()));
    assert!(evidence.validate_owner_view(&ledger.as_read()));
    validate_typed_commitments(storage.dimensions, &canonical).unwrap();
    validate_typed_commitments(storage.dimensions, &evidence).unwrap();
    let before_dimensions: [_; 6] = std::array::from_fn(|index| storage.dimensions[index].state);
    let before_phases: [_; 3] = std::array::from_fn(|index| {
        (
            storage.phase_states[index].current_live_bytes,
            storage.phase_states[index].peak_live_bytes,
        )
    });
    let before_header = (
        storage.current_live_bytes,
        storage.peak_live_bytes,
        storage.active_reservations,
        storage.current_administrative_bytes,
        storage.peak_administrative_bytes,
        storage.publication,
    );
    let rows = snapshot::SnapshotRows {
        storage,
        phases: read.phases(),
        work_rows: &ledger.work,
        work_header: &ledger.work_header,
    };
    rows.validate_final_shape(6, 3).unwrap();

    rows.write_terminal(
        &mut ledger.dimension_snapshot_staging,
        &mut ledger.phase_snapshot_staging,
        &canonical,
        &evidence,
    );

    let dimension = ledger.dimension_snapshot_staging[0].1;
    assert_eq!(
        (
            dimension.attempted(),
            dimension.consumed(),
            dimension.current_live_bytes()
        ),
        (11, 5, 0)
    );
    assert_eq!(
        (
            dimension.committed(),
            dimension.committed_canonical_output()
        ),
        (5, 4)
    );
    assert_eq!(
        ledger.dimension_snapshot_staging[1]
            .1
            .published_outcome_evidence(),
        2
    );
    assert!(
        ledger
            .phase_snapshot_staging
            .iter()
            .all(|row| row.1.current_live_bytes() == 0)
    );
    assert_eq!(
        ledger.phase_snapshot_staging[0].1.peak_live_bytes(),
        before_phases[1].1
    );
    assert_eq!(
        ledger.phase_snapshot_staging[2].1.peak_live_bytes(),
        before_phases[0].1
    );
    let storage = read.view();
    assert_eq!(
        std::array::from_fn::<_, 6, _>(|index| storage.dimensions[index].state),
        before_dimensions
    );
    assert_eq!(
        std::array::from_fn::<_, 3, _>(|index| (
            storage.phase_states[index].current_live_bytes,
            storage.phase_states[index].peak_live_bytes
        )),
        before_phases
    );
    assert_eq!(
        (
            storage.current_live_bytes,
            storage.peak_live_bytes,
            storage.active_reservations,
            storage.current_administrative_bytes,
            storage.peak_administrative_bytes,
            storage.publication
        ),
        before_header
    );
    assert!(storage.current_live_bytes > 0);
    assert_eq!(storage.active_reservations, 2);
    assert_eq!(
        (
            ledger.work[0].state.attempted,
            ledger.work[0].state.consumed
        ),
        (11, 5)
    );
    drop(read);
    drop((canonical, evidence));
    assert_eq!(ledger.storage.read().view().current_live_bytes, 0);
    assert_eq!(ledger.storage.read().view().active_reservations, 0);
}

#[test]
fn snapshot_shape_checks_staging_phase_positions_and_multiple_fault_precedence() {
    let mut ledger = ledger();
    let check = |ledger: &FiniteLedger<Owner, u8, u8>, dimensions, phases| {
        let state = ledger.storage.read();
        snapshot::SnapshotRows {
            storage: state.view(),
            phases: state.phases(),
            work_rows: &ledger.work,
            work_header: &ledger.work_header,
        }
        .validate_final_shape(dimensions, phases)
    };
    assert_eq!(
        check(&ledger, 5, 3),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    assert_eq!(
        check(&ledger, 6, 2),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    let FiniteStorage::Inline(Some(state)) = &mut ledger.storage else {
        panic!("fresh finite root stays inline");
    };
    state.phases[1].state_position = 3;
    assert_eq!(
        check(&ledger, 6, 3),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    ledger.work_header.producer_open = true;
    assert_eq!(
        check(&ledger, 5, 2),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    ledger.work_header.integrity_fault = true;
    assert_eq!(
        check(&ledger, 5, 2),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn finite_phase_completion_follows_declared_lifecycle_and_preserves_pending_precedence() {
    let mut ledger = ledger();
    ledger.view().try_complete_phase(3).unwrap();
    ledger.view().try_complete_phase(3).unwrap();
    ledger.view().try_complete_phase(1).unwrap();
    assert_eq!(
        ledger.view().try_complete_phase(3),
        Err(PhaseCompletionError::Regression {
            last_completed_phase: 1,
            phase: 3
        })
    );
    ledger.view().try_complete_phase(2).unwrap();
    assert_eq!(
        ledger.storage.read().view().last_completed_phase,
        Some(CompletedPhase {
            phase: 2,
            state_position: 2
        })
    );
    ledger.view().seal_ordinary_work().unwrap();
    assert_eq!(
        ledger.view().try_complete_phase(9),
        Err(PhaseCompletionError::PublicationPending)
    );
    assert!(ledger.storage.shared().is_none());
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
}

#[test]
fn finite_protected_work_transfers_policy_once_and_uses_the_actual_common_frame() {
    let mut ledger = FiniteLedger::<Owner, _, _>::try_new(
        FiniteLedgerLimits::try_new(
            0,
            0,
            [1_u8, 2, 3, 4, 5, 6].map(|dimension| (dimension, DimensionPolicy::counter(16, 16, 0))),
        )
        .unwrap(),
        [3_u8, 1, 2],
    )
    .unwrap();
    ledger
        .view()
        .transfer_failure_terminal_work(1, 6, 3)
        .unwrap();
    assert_eq!(ledger.work[0].policy.attempted_limit, 13);
    assert_eq!(ledger.work[5].policy.attempted_limit, 19);
    ledger.view().configure_protected_work(6).unwrap();
    assert_eq!(
        ledger.view().configure_protected_work(6),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    ledger
        .as_read()
        .preflight_protected_work_attachment(6, &[(6, 2)], NonZeroU64::new(3).unwrap())
        .unwrap();
    ledger.view().activate_protected_work(6).unwrap();
    {
        let (mut internal, _) = ledger.view().into_internal();
        let layout = internal.bind_protected([6]).unwrap();
        let mut frame = layout
            .admit_observed(
                &mut internal.reborrow().unwrap().work,
                [2],
                WorkLane::Protected,
            )
            .unwrap();
        frame
            .complete_batch_with_outcome(&layout, &mut internal.reborrow().unwrap().work, [2])
            .into_result()
            .unwrap();
        internal.resolve(layout, &mut frame, false).unwrap();
    }
    ledger.view().close_work().unwrap();
    assert_eq!(
        (
            ledger.work[5].state.attempted,
            ledger.work[5].state.consumed
        ),
        (2, 2)
    );
    assert_eq!(ledger.work[5].state.pending_consumed, 0);
    assert_eq!(ledger.work_header.mode, WorkMode::Closed);
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
    assert!(ledger.storage.shared().is_none());
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
}

#[test]
fn finite_common_charge_policy_and_evidence_entries_use_actual_inline_and_shared_rows() {
    for promoted in [false, true] {
        let mut ledger = ledger();
        if promoted {
            drop(ledger.view().reserve(3, 6, 0).unwrap());
        }
        {
            let mut view = ledger.view();
            view.charge_attempted(1, 5).unwrap();
            view.charge_consumed(1, 2).unwrap();
            view.charge_observed_batch(&[(2, 3)]).unwrap();
            view.charge_attempted(2, 2).unwrap();
            view.charge_consumed_batch(&[(1, 1), (2, 2)]).unwrap();
            assert_eq!(
                view.tighten_dimension_policy(1, DimensionPolicy::reservable_bytes(4, 4, 0, 64)),
                Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Attempted,
                    dimension: 1,
                    value: 5,
                    limit: 4
                })
            );
            assert_eq!(
                view.charge_attempted(1, 1),
                Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Attempted,
                    dimension: 1,
                    value: 6,
                    limit: 4
                })
            );
            view.charge_consumed(1, 1).unwrap();
        }
        let read = ledger.as_read();
        assert_eq!(read.dimension_count(), 6);
        assert!(read.catalog_shape_is_strictly_ordered(6, 3));
        assert!(!read.catalog_shape_is_strictly_ordered(5, 3));
        assert!(read.contains_dimension(6));
        assert!(!read.contains_dimension(9));
        assert!(read.contains_phase(3));
        assert!(!read.contains_phase(9));
        assert_eq!(read.dimension_at(0), Some(1));
        assert_eq!(read.phase_at(0), Some(1));
        let first = read.dimension_evidence(1).unwrap();
        assert_eq!(
            (
                first.attempted(),
                first.consumed(),
                first.current_live_bytes()
            ),
            (6, 4, 0)
        );
        let second = read.dimension_evidence(2).unwrap();
        assert_eq!((second.attempted(), second.consumed()), (5, 5));
        assert_eq!(read.phase_evidence(3), Some(PhaseSnapshot::default()));
        assert_eq!(
            read.dimension_evidence(9),
            Err(LedgerCause::UnknownDimension { dimension: 9 })
        );
        assert_eq!(read.phase_evidence(9), None);
        assert_eq!(read.terminal_residue_cause(), None);
        assert_eq!(ledger.work[0].policy.attempted_limit, 4);
        assert_eq!(ledger.work[0].state.pending_consumed, 0);
        assert_eq!(ledger.storage.shared().is_some(), promoted);
    }
}

#[test]
fn shared_read_view_shape_faults_keep_priority_and_leave_prefilled_fixed_staging_untouched() {
    for fault in 0..3 {
        let mut ledger = ledger();
        ledger.phase_snapshot_staging[1].1 = PhaseSnapshot::from_terminal_evidence(8, 13);
        let dimensions = ledger.dimension_snapshot_staging;
        let phases = ledger.phase_snapshot_staging;
        drop(ledger.view().reserve(3, 1, 0).unwrap());
        {
            let mut state = lock(ledger.storage.shared().unwrap());
            match fault {
                0 => state.phases[1].state_position = state.phases[0].state_position,
                1 => state.phases[1].state_position = 3,
                _ => {
                    state.last_completed_phase = Some(CompletedPhase {
                        phase: 1,
                        state_position: 3,
                    })
                }
            }
        }
        assert_eq!(
            ledger.as_read().terminal_residue_cause(),
            Some(LedgerCause::FinalSnapshotShapeMismatch)
        );
        ledger.work_header.producer_open = true;
        assert_eq!(
            ledger.as_read().terminal_residue_cause(),
            Some(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
        ledger.work_header.integrity_fault = true;
        assert_eq!(
            ledger.as_read().terminal_residue_cause(),
            Some(LedgerCause::FinalSnapshotShapeMismatch)
        );
        assert_eq!(ledger.dimension_snapshot_staging, dimensions);
        assert_eq!(ledger.phase_snapshot_staging, phases);
        assert_eq!(ledger.storage.read().view().active_reservations, 0);
    }
}

#[test]
fn finite_view_rejections_use_failed_tightening_without_mutating_live_or_consumed_evidence() {
    let mut ledger = ledger();
    ledger.view().charge_producer_attempted(1, 4, None).unwrap();
    ledger.view().charge_producer_attempted(2, 6, None).unwrap();
    let mut reservation = ledger.view().reserve(1, 2, 6).unwrap();
    let probe = LedgerTestProbe {
        state: Arc::downgrade(ledger.storage.shared().unwrap()),
    };
    let administrative_bytes = probe.current_administrative_bytes();
    {
        let mut view = ledger.view();
        assert_eq!(
            view.tighten_dimension_policy(2, DimensionPolicy::reservable_bytes(4, 3, 0, 5)),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: 2,
                value: 6,
                limit: 4,
            })
        );
        view.tighten_dimension_policy(6, DimensionPolicy::reservable_bytes(16, 16, 0, 5))
            .unwrap();
        assert_eq!(
            view.charge_producer_attempted(2, 1, None),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: 2,
                value: 7,
                limit: 4,
            })
        );
    }
    assert_eq!(ledger.work[1].state.attempted, 7);
    let dimensions_before = ledger
        .catalog
        .map(|dimension| ledger.as_read().dimension_evidence(dimension).unwrap());
    let phases_before = [1, 2, 3].map(|phase| ledger.as_read().phase_evidence(phase).unwrap());
    {
        let mut view = ledger.view();
        let live_rejection = LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: 2,
            value: 7,
            limit: 5,
        };
        assert_eq!(view.reserve(3, 2, 1).unwrap_err(), live_rejection);
        assert_eq!(
            view.resize_reservation(&mut reservation, 7),
            Err(live_rejection.clone())
        );
        assert_eq!(
            view.transition_reservation(&mut reservation, 3, 2, 7),
            Err(live_rejection)
        );
        assert_eq!(
            view.transition_reservation(&mut reservation, 3, 6, 6),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: 6,
                value: 6,
                limit: 5,
            })
        );
        assert_eq!(
            view.charge_producer_consumed_batch(&[(1, 1), (2, 2), (2, 2)], None),
            Err(LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: 2,
                value: 4,
                limit: 3,
            })
        );
        view.validate_live_reservation(&reservation).unwrap();
    }
    assert_eq!(
        ledger
            .catalog
            .map(|dimension| ledger.as_read().dimension_evidence(dimension).unwrap()),
        dimensions_before
    );
    assert_eq!(
        [1, 2, 3].map(|phase| ledger.as_read().phase_evidence(phase).unwrap()),
        phases_before
    );
    assert_eq!(
        (
            reservation.core.phase,
            reservation.core.phase_position,
            reservation.core.dimension,
            reservation.core.dimension_position,
            reservation.bytes(),
            reservation.core.active,
        ),
        (1, 1, 2, 1, 6, true)
    );
    assert_eq!(
        Arc::as_ptr(reservation.core.owner.as_ref().unwrap()),
        probe.state.as_ptr()
    );
    assert_eq!(probe.current_live_bytes(), 6);
    assert_eq!(probe.active_reservations(), 1);
    assert_eq!(probe.current_administrative_bytes(), administrative_bytes);
    ledger
        .view()
        .resize_reservation(&mut reservation, 5)
        .unwrap();
    ledger
        .view()
        .transition_reservation(&mut reservation, 3, 6, 5)
        .unwrap();
    assert_eq!(probe.current_live_bytes(), 5);
    assert_eq!(probe.active_reservations(), 1);
    let source_phase = ledger.as_read().phase_evidence(1).unwrap();
    let target_phase = ledger.as_read().phase_evidence(3).unwrap();
    assert_eq!(source_phase.current_live_bytes(), 0);
    assert_eq!(target_phase.current_live_bytes(), 5);
    drop(reservation);
    assert_eq!(probe.current_live_bytes(), 0);
    assert_eq!(probe.active_reservations(), 0);
    assert_eq!(probe.current_administrative_bytes(), administrative_bytes);
}

#[test]
fn finite_view_stopped_gates_match_dynamic_key_precedence_with_a_real_zero_byte_token() {
    let mut finite = ledger();
    let mut dynamic = Ledger::<Owner, _, _>::try_new(
        LedgerLimits::with_evidence_byte_limit(64, 4096, policies()).unwrap(),
        &[3_u8, 1, 2],
    )
    .unwrap();
    let mut reservation = finite.view().reserve(1, 2, 0).unwrap();
    let mut reference_reservation = dynamic.view().reserve(1, 2, 0).unwrap();
    let probe = LedgerTestProbe {
        state: Arc::downgrade(finite.storage.shared().unwrap()),
    };
    let administrative_bytes = probe.current_administrative_bytes();
    assert!(administrative_bytes > 0);
    assert_eq!(probe.current_live_bytes(), 0);
    assert_eq!(probe.active_reservations(), 1);
    for mut view in [finite.view(), dynamic.view()] {
        view.charge_producer_attempted(1, 2, None).unwrap();
        view.charge_producer_consumed_batch(&[(1, 1)], None)
            .unwrap();
        view.seal_ordinary_work().unwrap();
    }
    let dimensions_before = finite
        .catalog
        .map(|dimension| finite.as_read().dimension_evidence(dimension).unwrap());
    for closed in [false, true] {
        let work_gate = if closed {
            finite.view().close_work().unwrap();
            dynamic.view().close_work().unwrap();
            LedgerCause::PublicationAlreadyCompleted
        } else {
            LedgerCause::PublicationAlreadyPending
        };
        for (mut view, token) in [
            (finite.view(), &mut reservation),
            (dynamic.view(), &mut reference_reservation),
        ] {
            assert_eq!(
                view.charge_producer_attempted(9, 0, None),
                Err(LedgerCause::UnknownDimension { dimension: 9 })
            );
            assert_eq!(
                view.charge_producer_attempted(6, 0, Some(6)),
                Err(LedgerCause::UnknownDimension { dimension: 6 })
            );
            assert_eq!(
                view.charge_producer_consumed_batch(&[(2, 0), (1, 0), (9, 0)], None),
                Err(LedgerCause::UnknownDimension { dimension: 9 })
            );
            assert_eq!(
                view.charge_producer_consumed_batch(&[(1, u64::MAX), (6, 0)], Some(6)),
                Err(LedgerCause::UnknownDimension { dimension: 6 })
            );
            assert_eq!(
                view.charge_producer_attempted(1, u64::MAX, None),
                Err(work_gate.clone())
            );
            assert_eq!(
                view.charge_producer_consumed_batch(&[(2, 0), (1, 0)], None),
                Err(work_gate.clone())
            );
            assert_eq!(
                view.charge_producer_consumed_batch(&[], None),
                Err(work_gate.clone())
            );
            assert_eq!(view.charge_attempted(9, 0), Err(work_gate.clone()));
            assert_eq!(
                view.reserve(9, 9, 0).unwrap_err(),
                LedgerCause::PublicationAlreadyPending
            );
            assert_eq!(
                view.resize_reservation(token, 0),
                Err(LedgerCause::PublicationAlreadyPending)
            );
            assert_eq!(
                view.transition_reservation(token, 9, 9, 0),
                Err(LedgerCause::PublicationAlreadyPending)
            );
            view.validate_live_reservation(token).unwrap();
        }
        for dimension in 1..=6 {
            let expected = dimensions_before[usize::from(dimension - 1)];
            assert_eq!(finite.as_read().dimension_evidence(dimension), Ok(expected));
            assert_eq!(
                dynamic.as_read().dimension_evidence(dimension),
                Ok(expected)
            );
        }
        assert_eq!(probe.current_live_bytes(), 0);
        assert_eq!(probe.active_reservations(), 1);
        assert_eq!(probe.current_administrative_bytes(), administrative_bytes);
        assert_eq!(
            (
                reservation.core.phase,
                reservation.core.dimension,
                reservation.bytes(),
            ),
            (1, 2, 0)
        );
    }
    drop(reservation);
    drop(reference_reservation);
    assert_eq!(probe.active_reservations(), 0);
    assert_eq!(probe.current_live_bytes(), 0);
    drop(finite);
    assert!(!probe.is_available());
}

#[test]
fn finite_view_foreign_rejections_preserve_both_owners_and_the_original_capability() {
    for bytes in [0, 5] {
        let mut owner = ledger();
        let mut foreign = ledger();
        let mut reservation = owner.view().reserve(1, 2, bytes).unwrap();
        let foreign_reservation = foreign.view().reserve(3, 6, 3).unwrap();
        let owner_probe = LedgerTestProbe {
            state: Arc::downgrade(owner.storage.shared().unwrap()),
        };
        let foreign_probe = LedgerTestProbe {
            state: Arc::downgrade(foreign.storage.shared().unwrap()),
        };
        let accounting = |probe: &LedgerTestProbe<u8, u8>| {
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes(),
            )
        };
        let owner_before = accounting(&owner_probe);
        let foreign_before = accounting(&foreign_probe);
        assert_eq!((owner_before.0, owner_before.1), (bytes, 1));
        assert_eq!((foreign_before.0, foreign_before.1), (3, 1));
        assert!(owner_before.2 > 0);
        assert!(foreign_before.2 > 0);
        let owner_dimensions = owner
            .catalog
            .map(|dimension| owner.as_read().dimension_evidence(dimension).unwrap());
        let foreign_dimensions = foreign
            .catalog
            .map(|dimension| foreign.as_read().dimension_evidence(dimension).unwrap());
        foreign.view().seal_ordinary_work().unwrap();
        {
            let view = foreign.view();
            assert_eq!(
                view.resize_reservation(&mut reservation, bytes + 1),
                Err(LedgerCause::OwnerMismatch)
            );
            assert_eq!(
                view.transition_reservation(&mut reservation, 9, 9, u64::MAX),
                Err(LedgerCause::OwnerMismatch)
            );
            assert_eq!(
                view.validate_live_reservation(&reservation),
                Err(LedgerCause::OwnerMismatch)
            );
        }
        assert_eq!(accounting(&owner_probe), owner_before);
        assert_eq!(accounting(&foreign_probe), foreign_before);
        assert_eq!(
            owner
                .catalog
                .map(|dimension| owner.as_read().dimension_evidence(dimension).unwrap()),
            owner_dimensions
        );
        assert_eq!(
            foreign
                .catalog
                .map(|dimension| foreign.as_read().dimension_evidence(dimension).unwrap()),
            foreign_dimensions
        );
        assert_eq!(
            (
                reservation.core.phase,
                reservation.core.phase_position,
                reservation.core.dimension,
                reservation.core.dimension_position,
                reservation.bytes(),
                reservation.core.active,
            ),
            (1, 1, 2, 1, bytes, true)
        );
        assert_eq!(
            Arc::as_ptr(reservation.core.owner.as_ref().unwrap()),
            owner_probe.state.as_ptr()
        );
        owner
            .view()
            .validate_live_reservation(&reservation)
            .unwrap();
        owner
            .view()
            .resize_reservation(&mut reservation, bytes + 1)
            .unwrap();
        owner
            .view()
            .transition_reservation(&mut reservation, 3, 6, bytes + 1)
            .unwrap();
        assert_eq!(owner_probe.current_live_bytes(), bytes + 1);
        assert_eq!(owner_probe.active_reservations(), 1);
        assert_eq!(accounting(&foreign_probe), foreign_before);
        drop(reservation);
        assert_eq!(accounting(&owner_probe), (0, 0, owner_before.2));
        assert_eq!(accounting(&foreign_probe), foreign_before);
        drop(foreign_reservation);
        assert_eq!(accounting(&foreign_probe), (0, 0, foreign_before.2));
        drop(owner);
        drop(foreign);
        assert!(!owner_probe.is_available());
        assert!(!foreign_probe.is_available());
    }
}
