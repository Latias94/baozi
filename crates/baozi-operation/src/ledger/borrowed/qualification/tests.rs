use super::*;
use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};

struct Owner;

impl OperationOwner for Owner {}

type TestLedger = FiniteLedger<Owner, u8, u8>;

fn policy(committed: u64, evidence: u64) -> DimensionPolicy {
    DimensionPolicy::reservable_bytes(64, 64, committed, 16_384)
        .with_published_outcome_evidence_limit(evidence)
}

fn ledger() -> TestLedger {
    FiniteLedger::try_new(
        FiniteLedgerLimits::try_new(
            16_384,
            16_384,
            [6_u8, 2, 5, 1, 4, 3].map(|dimension| (dimension, policy(64, 64))),
        )
        .unwrap(),
        [3, 1, 2],
    )
    .unwrap()
}

fn batch<Destination: PublicationDestination>(
    owner: &mut TestLedger,
    phase: u8,
    dimension: u8,
    attributions: usize,
    commitments: usize,
) -> AttributionReservationBatch<Owner, u8, u8, Destination> {
    owner
        .view()
        .try_attribution_batch_from_plan(
            AttributionBatchPlan::try_new(phase, dimension, attributions, commitments).unwrap(),
        )
        .unwrap()
}

fn positioned_batch<Destination: PublicationDestination>(
    owner: &mut TestLedger,
    phase: u8,
    dimension: u8,
    bytes: u64,
    commitment: u64,
) -> AttributionReservationBatch<Owner, u8, u8, Destination> {
    let mut batch = batch(owner, phase, dimension, 1, 1);
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
    batch
}

fn batches<'batch>(
    canonical: &'batch AttributionReservationBatch<Owner, u8, u8, CanonicalOutput>,
    evidence: &'batch AttributionReservationBatch<Owner, u8, u8, OutcomeEvidenceDestination>,
) -> TerminalBatchView<'batch, Owner, u8, u8> {
    TerminalBatchView {
        canonical,
        evidence,
        deferred_canonical_storage: None,
        deferred_evidence_storage_first: None,
        deferred_evidence_storage_second: None,
        discard_canonical: None,
        discard_evidence: None,
    }
}

fn close(owner: &mut TestLedger) {
    owner.view().try_complete_phase(1).unwrap();
    owner.view().seal_ordinary_work().unwrap();
    owner.view().close_work().unwrap();
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    dimensions: [DimensionState; 6],
    phases: [(u64, u64, u64); 3],
    totals: (u64, u64, u64, u64, u64),
    administration: (u64, u64, u64),
    publication: PublicationState,
    completed: Option<CompletedPhase<u8>>,
    work_mode: WorkMode,
    open_envelopes: u64,
    open_roles: [bool; 4],
    protected_dimension: Option<usize>,
    integrity_fault: bool,
    dimension_staging: [(u8, DimensionSnapshot); 6],
    phase_staging: [(u8, PhaseSnapshot); 3],
}

fn observe(owner: &TestLedger, state: &StorageView<'_, u8, u8>) -> Observation {
    Observation {
        dimensions: std::array::from_fn(|index| {
            DimensionState::join(owner.work[index].state, state.dimensions[index].state)
        }),
        phases: std::array::from_fn(|index| {
            let phase = state.phase_states[index];
            (
                phase.current_live_bytes,
                phase.peak_live_bytes,
                phase.retained_output_bytes,
            )
        }),
        totals: (
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            state.retained_output_bytes,
            state.retained_output_reservations,
        ),
        administration: (
            state.current_administrative_bytes,
            state.peak_administrative_bytes,
            state.evidence_byte_limit,
        ),
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
        dimension_staging: owner.dimension_snapshot_staging,
        phase_staging: owner.phase_snapshot_staging,
    }
}

fn assert_validation(
    owner: &TestLedger,
    batches: &TerminalBatchView<'_, Owner, u8, u8>,
    expected: Result<Option<u8>, TypedFreezeQualificationError>,
) {
    let selected = owner.storage.read();
    let before = observe(owner, &selected.view());
    assert_eq!(
        batches.validate(
            &owner.as_read(),
            SnapshotRows {
                storage: selected.view(),
                phases: selected.phases(),
                work_rows: &owner.work,
                work_header: &owner.work_header,
            },
            (
                owner.dimension_snapshot_staging.len(),
                owner.phase_snapshot_staging.len(),
            ),
        ),
        expected
    );
    assert_eq!(observe(owner, &selected.view()), before);
}

#[test]
fn finite_qualification_covers_all_seven_actual_batch_roles_and_maps_the_completed_position() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
    let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
    let deferred_canonical = batch(&mut owner, 2, 3, 1, 1);
    let deferred_evidence_first = batch(&mut owner, 2, 4, 1, 1);
    let deferred_evidence_second = batch(&mut owner, 3, 5, 1, 1);
    let mut discard_canonical = positioned_batch(&mut owner, 2, 6, 11, 9);
    let discard_evidence = positioned_batch(&mut owner, 1, 3, 13, 7);
    discard_canonical.commitments[0].dimension_position = usize::MAX;
    close(&mut owner);
    owner
        .storage
        .write()
        .view_mut()
        .last_completed_phase
        .as_mut()
        .unwrap()
        .phase = 99;
    let batches = TerminalBatchView {
        deferred_canonical_storage: Some(&deferred_canonical),
        deferred_evidence_storage_first: Some(&deferred_evidence_first),
        deferred_evidence_storage_second: Some(&deferred_evidence_second),
        discard_canonical: Some(&discard_canonical),
        discard_evidence: Some(&discard_evidence),
        ..batches(&canonical, &evidence)
    };
    assert_validation(&owner, &batches, Ok(Some(1)));
    drop((
        canonical,
        evidence,
        deferred_canonical,
        deferred_evidence_first,
        deferred_evidence_second,
        discard_canonical,
        discard_evidence,
    ));
    let selected = owner.storage.read();
    assert_eq!(selected.view().current_live_bytes, 0);
    assert_eq!(selected.view().active_reservations, 0);
}

#[test]
fn finite_qualification_requires_exact_live_and_active_coverage() {
    for extra_bytes in [0, 4] {
        let mut owner = ledger();
        let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
        let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
        let extra = owner.view().reserve(2, 6, extra_bytes).unwrap();
        close(&mut owner);
        let expected = if extra_bytes == 0 {
            TypedFreezeQualificationError::ActiveReservationMismatch {
                expected: canonical.active_count().unwrap() + evidence.active_count().unwrap(),
                actual: owner.storage.read().view().active_reservations,
            }
        } else {
            TypedFreezeQualificationError::GlobalLiveMismatch {
                expected: canonical.live_bytes().unwrap() + evidence.live_bytes().unwrap(),
                actual: owner.storage.read().view().current_live_bytes,
            }
        };
        assert_validation(&owner, &batches(&canonical, &evidence), Err(expected));
        drop(extra);
        assert_validation(&owner, &batches(&canonical, &evidence), Ok(Some(1)));
    }
}

#[test]
fn finite_qualification_rechecks_both_current_destination_limits_after_tightening() {
    for evidence_limit in [false, true] {
        let mut owner = ledger();
        let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
        let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
        let (dimension, bound, expected) = if evidence_limit {
            (
                2,
                policy(64, 1),
                TypedFreezeQualificationError::OutcomeEvidenceCommitmentLimitExceeded {
                    index: 1,
                    value: 2,
                    limit: 1,
                },
            )
        } else {
            (
                1,
                policy(2, 64),
                TypedFreezeQualificationError::CanonicalCommitmentLimitExceeded {
                    index: 0,
                    value: 3,
                    limit: 2,
                },
            )
        };
        owner
            .view()
            .tighten_dimension_policy(dimension, bound)
            .unwrap();
        close(&mut owner);
        assert_validation(&owner, &batches(&canonical, &evidence), Err(expected));
    }
}

#[test]
fn finite_qualification_preserves_dimension_coverage_pending_limit_and_subset_error_order() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
    let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
    close(&mut owner);
    let expected_live = canonical.positioned_dimension_bytes(0).unwrap();
    {
        let mut selected = owner.storage.write();
        let state = selected.view_mut();
        state.dimensions[0].state.current_live_bytes += 1;
        state.dimensions[0].state.pending_publication = 1;
        state.dimensions[0].state.committed_canonical_output = 1;
        state.dimensions[0].policy.committed_limit = 2;
    }
    let batches = batches(&canonical, &evidence);
    assert_validation(
        &owner,
        &batches,
        Err(TypedFreezeQualificationError::DimensionLiveMismatch {
            index: 0,
            expected: expected_live,
            actual: expected_live + 1,
        }),
    );
    owner.storage.write().view_mut().dimensions[0]
        .state
        .current_live_bytes -= 1;
    assert_validation(
        &owner,
        &batches,
        Err(TypedFreezeQualificationError::PendingPublication {
            index: 0,
            amount: 1,
        }),
    );
    owner.storage.write().view_mut().dimensions[0]
        .state
        .pending_publication = 0;
    assert_validation(
        &owner,
        &batches,
        Err(
            TypedFreezeQualificationError::CanonicalCommitmentLimitExceeded {
                index: 0,
                value: 3,
                limit: 2,
            },
        ),
    );
    owner.storage.write().view_mut().dimensions[0]
        .policy
        .committed_limit = 64;
    assert_validation(
        &owner,
        &batches,
        Err(
            TypedFreezeQualificationError::CanonicalCommitmentSubsetViolation {
                index: 0,
                canonical: 4,
                committed: 3,
            },
        ),
    );
}

#[test]
fn finite_qualification_rejects_a_real_zero_pending_work_frame() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 0, 0);
    let evidence = positioned_batch(&mut owner, 1, 2, 0, 0);
    let (layout, mut frame) = {
        let (mut internal, _) = owner.view().into_internal();
        let layout = internal.bind_ordinary([1]).unwrap();
        let mut work = internal.reborrow().unwrap();
        let frame = layout
            .admit_observed(&mut work.work, [0], WorkLane::Internal)
            .unwrap();
        (layout, frame)
    };
    assert!(owner.work.iter().all(|row| row.state.pending_consumed == 0));
    assert_eq!(owner.work_header.open_envelopes, 1);
    owner.work_header.mode = WorkMode::Closed;
    owner.storage.write().view_mut().publication = PublicationState::Closing;
    assert_validation(
        &owner,
        &batches(&canonical, &evidence),
        Err(TypedFreezeQualificationError::SnapshotShape),
    );
    owner.work_header.mode = WorkMode::Ordinary;
    owner.storage.write().view_mut().publication = PublicationState::Open;
    owner
        .view()
        .into_internal()
        .0
        .resolve(layout, &mut frame, false)
        .unwrap();
    close(&mut owner);
    assert_validation(&owner, &batches(&canonical, &evidence), Ok(Some(1)));
}

#[test]
fn finite_qualification_uses_the_guarded_phase_catalog_and_phase_counters() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
    let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
    close(&mut owner);
    let shared = Arc::clone(owner.storage.shared().unwrap());
    lock(&shared).phases[0].state_position = 2;
    assert_validation(
        &owner,
        &batches(&canonical, &evidence),
        Err(TypedFreezeQualificationError::SnapshotShape),
    );
    lock(&shared).phases[0].state_position = 1;
    owner.storage.write().view_mut().phase_states[2].current_live_bytes = 1;
    assert_validation(
        &owner,
        &batches(&canonical, &evidence),
        Err(TypedFreezeQualificationError::PhaseLiveMismatch {
            index: 2,
            expected: 0,
            actual: 1,
        }),
    );
    owner.storage.write().view_mut().phase_states[2].current_live_bytes = 0;
    assert_validation(&owner, &batches(&canonical, &evidence), Ok(Some(1)));
}

#[test]
fn finite_qualification_rejects_foreign_empty_and_shared_batch_owners() {
    for positioned in [false, true] {
        let mut owner = ledger();
        let mut foreign = ledger();
        let canonical = if positioned {
            positioned_batch(&mut foreign, 3, 1, 5, 3)
        } else {
            batch(&mut foreign, 3, 1, 0, 0)
        };
        let evidence = if positioned {
            positioned_batch(&mut owner, 1, 2, 7, 2)
        } else {
            batch(&mut owner, 1, 2, 0, 0)
        };
        close(&mut owner);
        assert_validation(
            &owner,
            &batches(&canonical, &evidence),
            Err(TypedFreezeQualificationError::Ownership),
        );
    }
}

#[test]
fn finite_qualification_rejects_zero_byte_positions_and_nonempty_deferred_shells_first() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 0, 0);
    let evidence = positioned_batch(&mut owner, 1, 2, 0, 0);
    let mut deferred = positioned_batch(&mut owner, 2, 3, 0, 0);
    close(&mut owner);
    deferred.attributions[0].core.dimension_position = usize::MAX;
    let batches = TerminalBatchView {
        deferred_canonical_storage: Some(&deferred),
        ..batches(&canonical, &evidence)
    };
    assert_validation(
        &owner,
        &batches,
        Err(TypedFreezeQualificationError::Ownership),
    );
    let batches = TerminalBatchView {
        discard_canonical: Some(&deferred),
        deferred_canonical_storage: None,
        ..batches
    };
    assert_validation(
        &owner,
        &batches,
        Err(TypedFreezeQualificationError::BoundPositionOutOfBounds),
    );
    deferred.attributions[0].core.dimension_position = 2;
}

#[test]
fn finite_qualification_accounts_for_real_retained_output_and_its_release() {
    for retained_bytes in [0, 13] {
        let mut owner = ledger();
        let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
        let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
        let mut output = owner.view().reserve(3, 1, retained_bytes).unwrap();
        close(&mut owner);
        let batches = batches(&canonical, &evidence);
        let expected = if retained_bytes == 0 {
            TypedFreezeQualificationError::ActiveReservationMismatch {
                expected: canonical.active_count().unwrap() + evidence.active_count().unwrap(),
                actual: owner.storage.read().view().active_reservations,
            }
        } else {
            TypedFreezeQualificationError::GlobalLiveMismatch {
                expected: canonical.live_bytes().unwrap() + evidence.live_bytes().unwrap(),
                actual: owner.storage.read().view().current_live_bytes,
            }
        };
        assert_validation(&owner, &batches, Err(expected));
        output.retain_for_output().unwrap();
        {
            let selected = owner.storage.read();
            let state = selected.view();
            assert_eq!(state.retained_output_bytes, retained_bytes);
            assert_eq!(state.retained_output_reservations, 1);
            assert_eq!(
                state.dimensions[0].state.retained_output_bytes,
                retained_bytes
            );
            assert_eq!(state.phase_states[0].retained_output_bytes, retained_bytes);
        }
        assert_validation(&owner, &batches, Ok(Some(1)));
        drop(output);
        assert_validation(&owner, &batches, Ok(Some(1)));
        drop((canonical, evidence));
        let selected = owner.storage.read();
        let state = selected.view();
        assert_eq!(state.current_live_bytes, 0);
        assert_eq!(state.active_reservations, 0);
        assert_eq!(state.retained_output_bytes, 0);
        assert_eq!(state.retained_output_reservations, 0);
        assert!(state.dimensions.iter().all(|entry| {
            entry.state.current_live_bytes == 0 && entry.state.retained_output_bytes == 0
        }));
        assert!(
            state
                .phase_states
                .iter()
                .all(|phase| { phase.current_live_bytes == 0 && phase.retained_output_bytes == 0 })
        );
    }
}

#[test]
fn finite_qualification_distinguishes_retained_addition_boundaries_from_overflow() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
    let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
    close(&mut owner);
    let (live, active, dimension_live, phase_live) = {
        let selected = owner.storage.read();
        let state = selected.view();
        (
            state.current_live_bytes,
            state.active_reservations,
            state.dimensions[0].state.current_live_bytes,
            state.phase_states[0].current_live_bytes,
        )
    };
    let batches = batches(&canonical, &evidence);
    for (retained, expected) in [
        (
            [u64::MAX - live, 0, 0, 0],
            TypedFreezeQualificationError::GlobalLiveMismatch {
                expected: u64::MAX,
                actual: live,
            },
        ),
        (
            [u64::MAX - live + 1, 0, 0, 0],
            TypedFreezeQualificationError::ArithmeticOverflow,
        ),
        (
            [0, u64::MAX - active, 0, 0],
            TypedFreezeQualificationError::ActiveReservationMismatch {
                expected: u64::MAX,
                actual: active,
            },
        ),
        (
            [0, u64::MAX - active + 1, 0, 0],
            TypedFreezeQualificationError::ArithmeticOverflow,
        ),
        (
            [0, 0, u64::MAX - dimension_live, 0],
            TypedFreezeQualificationError::DimensionLiveMismatch {
                index: 0,
                expected: u64::MAX,
                actual: dimension_live,
            },
        ),
        (
            [0, 0, u64::MAX - dimension_live + 1, 0],
            TypedFreezeQualificationError::ArithmeticOverflow,
        ),
        (
            [0, 0, 0, u64::MAX - phase_live],
            TypedFreezeQualificationError::PhaseLiveMismatch {
                index: 0,
                expected: u64::MAX,
                actual: phase_live,
            },
        ),
        (
            [0, 0, 0, u64::MAX - phase_live + 1],
            TypedFreezeQualificationError::ArithmeticOverflow,
        ),
    ] {
        {
            let mut selected = owner.storage.write();
            let mut state = selected.view_mut();
            state.retained_output_bytes = retained[0];
            state.retained_output_reservations = retained[1];
            state.dimensions[0].state.retained_output_bytes = retained[2];
            state.phase_states[0].retained_output_bytes = retained[3];
        }
        assert_validation(&owner, &batches, Err(expected));
    }
    {
        let mut selected = owner.storage.write();
        let mut state = selected.view_mut();
        state.retained_output_bytes = 0;
        state.retained_output_reservations = 0;
        state.dimensions[0].state.retained_output_bytes = 0;
        state.phase_states[0].retained_output_bytes = 0;
    }
    assert_validation(&owner, &batches, Ok(Some(1)));
}

#[test]
fn finite_qualification_checks_commitment_overflow_before_limits_and_later_destinations() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
    let evidence = positioned_batch(&mut owner, 1, 1, 7, 2);
    close(&mut owner);
    {
        let mut selected = owner.storage.write();
        let entry = &mut selected.view_mut().dimensions[0];
        entry.state.committed = u64::MAX - 3;
        entry.state.committed_canonical_output = u64::MAX - 3;
        entry.state.published_outcome_evidence = u64::MAX - 2;
        entry.policy.committed_limit = u64::MAX;
        entry.policy.published_outcome_evidence_limit = u64::MAX;
    }
    let batches = batches(&canonical, &evidence);
    assert_validation(&owner, &batches, Ok(Some(1)));
    {
        let mut selected = owner.storage.write();
        let entry = &mut selected.view_mut().dimensions[0];
        entry.state.committed += 1;
        entry.state.committed_canonical_output += 1;
        entry.state.published_outcome_evidence += 1;
        entry.policy.committed_limit -= 1;
        entry.policy.published_outcome_evidence_limit -= 1;
    }
    assert_validation(
        &owner,
        &batches,
        Err(TypedFreezeQualificationError::CanonicalCommitmentOverflow { index: 0 }),
    );
    owner.storage.write().view_mut().dimensions[0]
        .state
        .committed -= 1;
    assert_validation(
        &owner,
        &batches,
        Err(TypedFreezeQualificationError::CanonicalCommitmentOverflow { index: 0 }),
    );
    owner.storage.write().view_mut().dimensions[0]
        .state
        .committed_canonical_output -= 1;
    assert_validation(
        &owner,
        &batches,
        Err(
            TypedFreezeQualificationError::CanonicalCommitmentLimitExceeded {
                index: 0,
                value: u64::MAX,
                limit: u64::MAX - 1,
            },
        ),
    );
    owner.storage.write().view_mut().dimensions[0]
        .policy
        .committed_limit = u64::MAX;
    assert_validation(
        &owner,
        &batches,
        Err(TypedFreezeQualificationError::OutcomeEvidenceCommitmentOverflow { index: 0 }),
    );
    owner.storage.write().view_mut().dimensions[0]
        .state
        .published_outcome_evidence -= 1;
    assert_validation(
        &owner,
        &batches,
        Err(
            TypedFreezeQualificationError::OutcomeEvidenceCommitmentLimitExceeded {
                index: 0,
                value: u64::MAX,
                limit: u64::MAX - 1,
            },
        ),
    );
    owner.storage.write().view_mut().dimensions[0]
        .policy
        .published_outcome_evidence_limit = u64::MAX;
    assert_validation(&owner, &batches, Ok(Some(1)));
}

#[test]
fn finite_qualification_succeeds_without_a_completed_phase() {
    for positioned in [false, true] {
        let mut owner = ledger();
        let canonical = if positioned {
            positioned_batch(&mut owner, 3, 1, 5, 3)
        } else {
            batch(&mut owner, 3, 1, 0, 0)
        };
        let evidence = if positioned {
            positioned_batch(&mut owner, 1, 2, 7, 2)
        } else {
            batch(&mut owner, 1, 2, 0, 0)
        };
        owner.view().seal_ordinary_work().unwrap();
        owner.view().close_work().unwrap();
        assert_validation(&owner, &batches(&canonical, &evidence), Ok(None));
    }
}

#[test]
fn terminal_transition_qualifies_the_actual_inline_or_shared_owner_without_promotion() {
    for positioned in [false, true] {
        let mut owner = ledger();
        let canonical = if positioned {
            positioned_batch(&mut owner, 3, 1, 5, 3)
        } else {
            batch(&mut owner, 3, 1, 0, 0)
        };
        let evidence = if positioned {
            positioned_batch(&mut owner, 1, 2, 7, 2)
        } else {
            batch(&mut owner, 1, 2, 0, 0)
        };
        close(&mut owner);
        let known = owner.known_administrative_bytes(0);
        let before = observe(&owner, &owner.storage.read().view());
        let staging = (
            owner.dimension_snapshot_staging.as_ptr(),
            owner.phase_snapshot_staging.as_ptr(),
        );
        assert_eq!(owner.storage.shared().is_some(), positioned);
        assert_eq!(
            owner
                .view()
                .try_qualify_terminal(batches(&canonical, &evidence), known),
            Ok(Some(1))
        );
        let mut expected = before;
        expected.publication = PublicationState::TerminalQualified;
        assert_eq!(observe(&owner, &owner.storage.read().view()), expected);
        assert_eq!(owner.known_administrative_bytes(0), known);
        assert_eq!(owner.storage.shared().is_some(), positioned);
        assert_eq!(
            (
                owner.dimension_snapshot_staging.as_ptr(),
                owner.phase_snapshot_staging.as_ptr(),
            ),
            staging
        );
    }
}

#[test]
fn terminal_transition_rejects_unknown_cover_dirty_work_and_foreign_batches_transactionally() {
    for fault in 0..5 {
        let mut owner = ledger();
        let mut foreign = ledger();
        let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
        let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
        let foreign_canonical = positioned_batch(&mut foreign, 3, 1, 5, 3);
        close(&mut owner);
        let mut known = owner.known_administrative_bytes(0);
        match fault {
            0 => known = None,
            1 => known = known.and_then(|bytes| bytes.checked_add(1)),
            2 => owner.work_header.integrity_fault = true,
            3 => owner.work_header.producer_open = true,
            _ => {}
        }
        let before = observe(&owner, &owner.storage.read().view());
        let foreign_before = observe(&foreign, &foreign.storage.read().view());
        let selected = if fault == 4 {
            batches(&foreign_canonical, &evidence)
        } else {
            batches(&canonical, &evidence)
        };
        assert_eq!(
            owner.view().try_qualify_terminal(selected, known),
            Err(if fault == 4 {
                TypedFreezeQualificationError::Ownership
            } else {
                TypedFreezeQualificationError::SnapshotShape
            })
        );
        assert_eq!(observe(&owner, &owner.storage.read().view()), before);
        assert_eq!(
            observe(&foreign, &foreign.storage.read().view()),
            foreign_before
        );
        assert_eq!(canonical.attribution_len(), 1);
        assert_eq!(evidence.attribution_len(), 1);
        assert_eq!(foreign_canonical.attribution_len(), 1);
    }
}

#[test]
fn terminal_transition_checks_current_destination_limits_before_changing_publication() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
    let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
    owner
        .view()
        .tighten_dimension_policy(1, policy(2, 64))
        .unwrap();
    close(&mut owner);
    let known = owner.known_administrative_bytes(0);
    let before = observe(&owner, &owner.storage.read().view());
    assert_eq!(
        owner
            .view()
            .try_qualify_terminal(batches(&canonical, &evidence), known),
        Err(
            TypedFreezeQualificationError::CanonicalCommitmentLimitExceeded {
                index: 0,
                value: 3,
                limit: 2,
            }
        )
    );
    assert_eq!(observe(&owner, &owner.storage.read().view()), before);
}

#[test]
fn terminal_transition_counts_an_unrepresented_zero_byte_token_and_can_retry_after_its_drop() {
    let mut owner = ledger();
    let canonical = positioned_batch(&mut owner, 3, 1, 5, 3);
    let evidence = positioned_batch(&mut owner, 1, 2, 7, 2);
    let zero = owner.view().reserve(1, 1, 0).unwrap();
    close(&mut owner);
    let known = owner.known_administrative_bytes(0);
    let before = observe(&owner, &owner.storage.read().view());
    let expected = canonical.active_count().unwrap() + evidence.active_count().unwrap();
    assert_eq!(
        owner
            .view()
            .try_qualify_terminal(batches(&canonical, &evidence), known),
        Err(TypedFreezeQualificationError::ActiveReservationMismatch {
            expected,
            actual: expected + 1,
        })
    );
    assert_eq!(observe(&owner, &owner.storage.read().view()), before);
    drop(zero);
    assert_eq!(
        owner
            .view()
            .try_qualify_terminal(batches(&canonical, &evidence), known),
        Ok(Some(1))
    );
}

#[test]
fn qualified_batch_commit_retires_actual_inline_or_shared_rows_without_reconstructing_a_ledger() {
    for positioned in [false, true] {
        let mut owner = ledger();
        let mut canonical = if positioned {
            positioned_batch(&mut owner, 3, 1, 5, 3)
        } else {
            batch(&mut owner, 3, 1, 0, 0)
        };
        let mut evidence = if positioned {
            positioned_batch(&mut owner, 1, 2, 7, 2)
        } else {
            batch(&mut owner, 1, 2, 0, 0)
        };
        close(&mut owner);
        let known = owner.known_administrative_bytes(0);
        owner
            .view()
            .try_qualify_terminal(batches(&canonical, &evidence), known)
            .unwrap();
        let before = observe(&owner, &owner.storage.read().view());
        let mut batches = DetachedTypedBatches {
            canonical: canonical.detach_in_place(owner.storage.shared(), owner.identity),
            evidence: evidence.detach_in_place(owner.storage.shared(), owner.identity),
            deferred_canonical_storage: None,
            deferred_evidence_storage_first: None,
            deferred_evidence_storage_second: None,
            discard_canonical: None,
            discard_evidence: None,
        };
        if let Some(shared) = owner.storage.shared() {
            assert_eq!(Arc::strong_count(shared), 1);
        }
        {
            let mut selected = owner.storage.write();
            let mut state = selected.view_mut();
            batches.apply_commitments(&mut state);
            batches.release_attributions(&mut state);
            release_retained_output_attributions(&mut state);
            batches.destroy_storage_then_release_covers(&mut state);
            assert_eq!(state.current_live_bytes, 0);
            assert_eq!(state.active_reservations, 0);
            assert_eq!(state.retained_output_bytes, 0);
            assert_eq!(state.retained_output_reservations, 0);
            assert!(
                state
                    .dimensions
                    .iter()
                    .all(|entry| entry.state.current_live_bytes == 0)
            );
            assert!(
                state
                    .phase_states
                    .iter()
                    .all(|phase| phase.current_live_bytes == 0)
            );
            assert_eq!(state.peak_live_bytes, before.totals.1);
            assert_eq!(state.current_administrative_bytes, known.unwrap());
            assert_eq!(state.publication, PublicationState::TerminalQualified);
        }
        assert_eq!(owner.storage.shared().is_some(), positioned);
        let canonical = owner.as_read().dimension_evidence(1).unwrap();
        let evidence = owner.as_read().dimension_evidence(2).unwrap();
        assert_eq!(canonical.committed(), if positioned { 3 } else { 0 });
        assert_eq!(
            canonical.committed_canonical_output(),
            canonical.committed()
        );
        assert_eq!(
            evidence.published_outcome_evidence(),
            if positioned { 2 } else { 0 }
        );
        assert_eq!(batches.active_count(), 0);
        assert_eq!(batches.live_bytes(), 0);
    }
}
