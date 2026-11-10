use super::*;

struct Owner;

impl OperationOwner for Owner {}

type TestLedger = FiniteLedger<Owner, u8, u8>;

fn ledger() -> TestLedger {
    FiniteLedger::try_new(
        FiniteLedgerLimits::try_new(
            16_384,
            16_384,
            [6, 2, 5, 1, 4, 3].map(|dimension| {
                (
                    dimension,
                    DimensionPolicy::reservable_bytes(64, 64, 64, 16_384)
                        .with_published_outcome_evidence_limit(64),
                )
            }),
        )
        .unwrap(),
        [3, 1, 2],
    )
    .unwrap()
}

fn close(owner: &mut TestLedger) {
    owner.view().try_complete_phase(1).unwrap();
    owner.view().seal_ordinary_work().unwrap();
    owner.view().close_work().unwrap();
}

fn batch<Destination: PublicationDestination>(
    owner: &mut TestLedger,
    phase: u8,
    dimension: u8,
    positioned: bool,
    bytes: u64,
    commitment: u64,
) -> AttributionReservationBatch<Owner, u8, u8, Destination> {
    let slots = usize::from(positioned);
    let mut batch = owner
        .view()
        .try_attribution_batch_from_plan(
            AttributionBatchPlan::try_new(phase, dimension, slots, slots).unwrap(),
        )
        .unwrap();
    if positioned {
        let attribution = {
            let (work, mut storage) = owner.view().into_exclusive().unwrap();
            work.reserve_operation_attribution::<Destination, _, _>(
                &mut storage,
                phase,
                dimension,
                bytes,
            )
            .unwrap()
        };
        assert!(batch.try_push_attribution(attribution).is_ok());
        assert!(
            owner
                .as_read()
                .try_push_commitment(&mut batch, OutputCommitment::new(dimension, commitment))
                .is_ok()
        );
    }
    batch
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    dimensions: [DimensionState; FINITE_DIMENSION_COUNT],
    phases: [(u64, u64, u64); FINITE_PHASE_COUNT],
    totals: [u64; 8],
    publication: PublicationState,
    completed: Option<CompletedPhase<u8>>,
    work_mode: WorkMode,
    open_envelopes: u64,
    open_roles: [bool; 4],
    protected_dimension: Option<usize>,
    integrity_fault: bool,
}

fn observe(owner: &TestLedger) -> Observation {
    let selected = owner.storage.read();
    let state = selected.view();
    Observation {
        dimensions: std::array::from_fn(|position| {
            DimensionState::join(owner.work[position].state, state.dimensions[position].state)
        }),
        phases: std::array::from_fn(|position| {
            let phase = state.phase_states[position];
            (
                phase.current_live_bytes,
                phase.peak_live_bytes,
                phase.retained_output_bytes,
            )
        }),
        totals: [
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            state.retained_output_bytes,
            state.retained_output_reservations,
            state.evidence_byte_limit,
            state.current_administrative_bytes,
            state.peak_administrative_bytes,
        ],
        publication: state.publication,
        completed: state.last_completed_phase,
        work_mode: owner.work_header.mode,
        open_envelopes: owner.work_header.open_envelopes,
        open_roles: [
            owner.work_header.producer_open,
            owner.work_header.parser_parent_open,
            owner.work_header.parser_child_open,
            owner.work_header.protected_open,
        ],
        protected_dimension: owner.work_header.protected_dimension,
        integrity_fault: owner.work_header.integrity_fault,
    }
}

fn fixed_buffers(owner: &TestLedger) -> [usize; 5] {
    [
        owner.catalog.as_ptr() as usize,
        owner.work.as_ptr() as usize,
        owner.phases.as_ptr() as usize,
        owner.dimension_snapshot_staging.as_ptr() as usize,
        owner.phase_snapshot_staging.as_ptr() as usize,
    ]
}

fn shared_buffers(owner: &TestLedger) -> Option<[(usize, usize); 5]> {
    owner.storage.shared().map(|shared| {
        let state = lock(shared);
        [
            (
                state.dimensions.as_ptr() as usize,
                state.dimensions.capacity(),
            ),
            (state.phases.as_ptr() as usize, state.phases.capacity()),
            (
                state.phase_states.as_ptr() as usize,
                state.phase_states.capacity(),
            ),
            (
                state.dimension_snapshot_staging.as_ptr() as usize,
                state.dimension_snapshot_staging.capacity(),
            ),
            (
                state.phase_snapshot_staging.as_ptr() as usize,
                state.phase_snapshot_staging.capacity(),
            ),
        ]
    })
}

#[test]
fn closed_inline_and_shared_capture_own_fixed_facts_without_allocating_or_moving_buffers() {
    let _guard = test_evidence_allocation_guard();
    for promoted in [false, true] {
        let mut owner = ledger();
        owner.view().charge_attempted(2, 11).unwrap();
        owner.view().charge_consumed(2, 5).unwrap();
        if promoted {
            drop(owner.view().reserve(3, 2, 7).unwrap());
            drop(owner.view().reserve(1, 4, 11).unwrap());
        }
        close(&mut owner);
        let before = observe(&owner);
        let buffers = (fixed_buffers(&owner), shared_buffers(&owner));
        let identity = owner.identity;
        let known = owner.known_administrative_bytes(0);
        test_fail_next_evidence_allocation();

        let (snapshot, cause) = owner.take_terminal_snapshot(0);

        assert!(test_evidence_allocation_failure_is_armed());
        test_set_evidence_allocation_failure(false);
        assert_eq!(cause, None);
        assert_eq!(observe(&owner), before);
        assert_eq!((fixed_buffers(&owner), shared_buffers(&owner)), buffers);
        assert_eq!(owner.identity, identity);
        assert_eq!(owner.storage.shared().is_some(), promoted);
        assert_eq!(owner.known_administrative_bytes(0), known);
        assert_eq!(snapshot.dimensions, owner.dimension_snapshot_staging);
        assert_eq!(snapshot.phases, owner.phase_snapshot_staging);
        assert_eq!(snapshot.dimensions().len(), FINITE_DIMENSION_COUNT);
        assert_eq!(snapshot.phases().len(), FINITE_PHASE_COUNT);
        assert_eq!(snapshot.dimensions.map(|row| row.0), [1, 2, 3, 4, 5, 6]);
        assert_eq!(snapshot.phases.map(|row| row.0), [1, 2, 3]);
        assert_eq!(snapshot.dimension(9), None);
        assert_eq!(snapshot.phase(9), None);
        assert_eq!(snapshot.last_completed_phase(), Some(1));
        assert_eq!(snapshot.current_live_bytes(), 0);
        assert_eq!(snapshot.active_reservations(), 0);
        assert_eq!(snapshot.peak_live_bytes(), if promoted { 11 } else { 0 });
        assert_eq!(snapshot.current_administrative_bytes(), before.totals[6]);
        assert_eq!(snapshot.peak_administrative_bytes(), before.totals[7]);
        assert_eq!(snapshot.covered_administrative_bytes(), known);
        assert_eq!(
            snapshot.phase(1).unwrap().peak_live_bytes(),
            if promoted { 11 } else { 0 }
        );
        assert_eq!(
            snapshot.phase(3).unwrap().peak_live_bytes(),
            if promoted { 7 } else { 0 }
        );
        assert!(
            snapshot
                .phases()
                .all(|(_, row)| row.current_live_bytes() == 0)
        );
        drop(owner);
        let dimension = snapshot.dimension(2).unwrap();
        assert_eq!((dimension.attempted(), dimension.consumed()), (11, 5));
        assert_eq!(dimension.peak_live_bytes(), if promoted { 7 } else { 0 });
    }
}

#[test]
fn completed_but_unresolved_frame_keeps_its_exact_cause_and_nonzero_observations() {
    for promoted in [false, true] {
        let mut owner = ledger();
        let reservation = promoted.then(|| owner.view().reserve(3, 2, 7).unwrap());
        let (layout, mut frame) = {
            let (mut internal, _) = owner.view().into_internal();
            let layout = internal.bind_ordinary([2]).unwrap();
            let mut permission = internal.reborrow().unwrap();
            let mut frame = layout
                .admit_observed(&mut permission.work, [5], WorkLane::Internal)
                .unwrap();
            frame
                .complete_batch_with_outcome(&layout, &mut permission.work, [5])
                .into_result()
                .unwrap();
            (layout, frame)
        };
        assert!(owner.work.iter().all(|row| row.state.pending_consumed == 0));
        assert_eq!(owner.work_header.open_envelopes, 1);
        let before = observe(&owner);

        let (snapshot, cause) = owner.take_terminal_snapshot(1);

        assert_eq!(
            cause,
            Some(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
        assert_eq!(observe(&owner), before);
        let dimension = snapshot.dimension(2).unwrap();
        assert_eq!((dimension.attempted(), dimension.consumed()), (5, 5));
        assert_eq!(dimension.current_live_bytes(), if promoted { 7 } else { 0 });
        assert_eq!(snapshot.current_live_bytes(), if promoted { 7 } else { 0 });
        assert_eq!(snapshot.active_reservations(), u64::from(promoted));
        assert_eq!(
            snapshot.phase(3).unwrap().current_live_bytes(),
            if promoted { 7 } else { 0 }
        );
        assert_eq!(snapshot.current_administrative_bytes(), before.totals[6]);
        assert_eq!(snapshot.covered_administrative_bytes(), None);
        owner
            .view()
            .into_internal()
            .0
            .resolve(layout, &mut frame, false)
            .unwrap();
        drop(reservation);
    }
}

#[test]
fn qualified_projection_uses_actual_batches_and_preserves_live_accounts_and_buffer_identity() {
    for positioned in [false, true] {
        let mut owner = ledger();
        owner.view().charge_attempted(1, 11).unwrap();
        owner.view().charge_consumed(1, 5).unwrap();
        owner.view().charge_observed_batch(&[(2, 4)]).unwrap();
        let canonical = batch::<CanonicalOutput>(&mut owner, 3, 1, positioned, 5, 3);
        let evidence = batch::<OutcomeEvidenceDestination>(&mut owner, 1, 2, positioned, 7, 2);
        close(&mut owner);
        let known = owner.known_administrative_bytes(0);
        let completed = owner
            .view()
            .try_qualify_terminal(
                TerminalBatchView {
                    canonical: &canonical,
                    evidence: &evidence,
                    deferred_canonical_storage: None,
                    deferred_evidence_storage_first: None,
                    deferred_evidence_storage_second: None,
                    discard_canonical: None,
                    discard_evidence: None,
                },
                known,
            )
            .unwrap();
        assert_eq!(completed, Some(1));
        let before = observe(&owner);
        let identity = owner.identity;
        let buffers = (fixed_buffers(&owner), shared_buffers(&owner));
        let batch_buffers = (
            canonical.attributions.as_ptr(),
            canonical.commitments.as_ptr(),
            evidence.attributions.as_ptr(),
            evidence.commitments.as_ptr(),
        );
        let accounts = (
            canonical.account_capacity_bytes(),
            evidence.account_capacity_bytes(),
        );
        if positioned {
            assert!(accounts.0 > 0 && accounts.1 > 0);
            assert!(known.unwrap() > 0);
            assert_eq!(before.totals[0], accounts.0 + accounts.1 + 12);
            assert_eq!(before.totals[2], 4);
        }

        let projection =
            owner.take_typed_terminal_projection(completed, &canonical, &evidence, known);

        assert_eq!(projection.last_completed_phase(), Some(1));
        assert_eq!(projection.current_live_bytes(), 0);
        assert_eq!(projection.active_reservations(), 0);
        assert_eq!(projection.peak_live_bytes(), before.totals[1]);
        assert_eq!(projection.current_administrative_bytes(), before.totals[6]);
        assert_eq!(projection.peak_administrative_bytes(), before.totals[7]);
        assert_eq!(projection.covered_administrative_bytes(), known);
        assert_eq!(projection.dimensions, owner.dimension_snapshot_staging);
        assert_eq!(projection.phases, owner.phase_snapshot_staging);
        let dimension = projection.dimension(1).unwrap();
        assert_eq!((dimension.attempted(), dimension.consumed()), (11, 5));
        assert_eq!(dimension.committed(), if positioned { 3 } else { 0 });
        assert_eq!(
            dimension.committed_canonical_output(),
            if positioned { 3 } else { 0 }
        );
        assert_eq!(
            projection
                .dimension(2)
                .unwrap()
                .published_outcome_evidence(),
            if positioned { 2 } else { 0 }
        );
        for (position, (_, dimension)) in projection.dimensions().enumerate() {
            assert_eq!(dimension.current_live_bytes(), 0);
            assert_eq!(
                dimension.peak_live_bytes(),
                before.dimensions[position].peak_live_bytes
            );
        }
        assert!(
            projection
                .phases()
                .all(|(_, row)| row.current_live_bytes() == 0)
        );
        assert_eq!(
            projection.phase(1).unwrap().peak_live_bytes(),
            before.phases[1].1
        );
        assert_eq!(
            projection.phase(3).unwrap().peak_live_bytes(),
            before.phases[0].1
        );
        for cover in [None, known.map(|bytes| bytes + 1)] {
            let uncovered =
                owner.take_typed_terminal_projection(completed, &canonical, &evidence, cover);
            assert_eq!(
                uncovered,
                FiniteLedgerSnapshot {
                    covered_administrative_bytes: None,
                    ..projection
                }
            );
        }
        assert_eq!(observe(&owner), before);
        assert_eq!(owner.identity, identity);
        assert_eq!((fixed_buffers(&owner), shared_buffers(&owner)), buffers);
        assert_eq!(owner.storage.shared().is_some(), positioned);
        assert_eq!(owner.known_administrative_bytes(0), known);
        assert_eq!(
            (
                canonical.attributions.as_ptr(),
                canonical.commitments.as_ptr(),
                evidence.attributions.as_ptr(),
                evidence.commitments.as_ptr(),
            ),
            batch_buffers
        );
        assert_eq!(
            (
                canonical.account_capacity_bytes(),
                evidence.account_capacity_bytes()
            ),
            accounts
        );
        assert!(canonical.validate_owner_view(&owner.as_read()));
        assert!(evidence.validate_owner_view(&owner.as_read()));
        drop((canonical, evidence));
        assert_eq!(owner.storage.read().view().current_live_bytes, 0);
        assert_eq!(owner.storage.read().view().active_reservations, 0);
        drop(owner);
        assert_eq!(
            projection.dimension(1).unwrap().committed(),
            if positioned { 3 } else { 0 }
        );
    }
}

#[test]
fn zero_byte_active_reservation_remains_distinct_from_a_clean_terminal_capture() {
    let mut owner = ledger();
    let reservation = owner.view().reserve(1, 6, 0).unwrap();
    close(&mut owner);
    let before = observe(&owner);

    let (snapshot, cause) = owner.take_terminal_snapshot(1);

    assert_eq!(
        cause,
        Some(LedgerCause::ActiveReservationsRemain { count: 1 })
    );
    assert_eq!(observe(&owner), before);
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 1);
    assert_eq!(snapshot.peak_live_bytes(), 0);
    assert!(
        snapshot
            .dimensions()
            .all(|(_, row)| row.current_live_bytes() == 0)
    );
    assert!(
        snapshot
            .phases()
            .all(|(_, row)| row.current_live_bytes() == 0)
    );
    assert_eq!(snapshot.covered_administrative_bytes(), None);
    drop(reservation);
    let (clean, cause) = owner.take_terminal_snapshot(0);
    assert_eq!(cause, None);
    assert_eq!(clean.active_reservations(), 0);
    assert_eq!(
        clean.covered_administrative_bytes(),
        owner.known_administrative_bytes(0)
    );
    assert_eq!(snapshot.active_reservations(), 1);
}

#[test]
fn exact_administrative_coverage_and_residue_error_order_match_terminal_capture() {
    for promoted in [false, true] {
        let mut owner = ledger();
        if promoted {
            drop(owner.view().reserve(3, 1, 0).unwrap());
        }
        close(&mut owner);
        let before = observe(&owner);
        let known = owner.known_administrative_bytes(0);
        let (exact, cause) = owner.take_terminal_snapshot(0);
        assert_eq!(cause, None);
        assert_eq!(exact.covered_administrative_bytes(), known);
        for progress_bytes in [1, u64::MAX] {
            let (uncovered, cause) = owner.take_terminal_snapshot(progress_bytes);
            assert_eq!(cause, Some(LedgerCause::FinalSnapshotShapeMismatch));
            assert_eq!(
                uncovered,
                FiniteLedgerSnapshot {
                    covered_administrative_bytes: None,
                    ..exact
                }
            );
        }
        assert_eq!(observe(&owner), before);
    }
    for bytes in [0, 7] {
        let mut owner = ledger();
        let reservation = owner.view().reserve(3, 1, bytes).unwrap();
        close(&mut owner);
        owner.storage.write().view_mut().publication = PublicationState::Pending;
        let expected = if bytes == 0 {
            LedgerCause::PendingPublicationsRemain
        } else {
            LedgerCause::LiveReservationsRemain { bytes }
        };
        let (snapshot, cause) = owner.take_terminal_snapshot(1);
        assert_eq!(cause, Some(expected));
        assert_eq!(snapshot.current_live_bytes(), bytes);
        assert_eq!(snapshot.active_reservations(), 1);
        assert_eq!(snapshot.dimension(1).unwrap().current_live_bytes(), bytes);
        assert_eq!(snapshot.covered_administrative_bytes(), None);
        owner.work_header.integrity_fault = true;
        let (dirty, cause) = owner.take_terminal_snapshot(1);
        assert_eq!(cause, Some(LedgerCause::FinalSnapshotShapeMismatch));
        assert_eq!(dirty, snapshot);
        drop(reservation);
    }
}
