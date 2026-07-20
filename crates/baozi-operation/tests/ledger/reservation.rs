use super::*;

#[test]
fn reservations_track_overlap_phase_peaks_transfer_and_raii_release() {
    let mut coordinator = coordinator(20);
    let snapshot_bytes = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("snapshot reservation");
    let mut decode_bytes = coordinator
        .checkpoint()
        .reserve(Phase::Decode, Dimension::AssetBytes, 7)
        .expect("decode reservation");

    let overlap = coordinator.ledger_snapshot();
    assert_eq!(overlap.current_live_bytes(), 15);
    assert_eq!(overlap.peak_live_bytes(), 15);
    assert_eq!(
        overlap
            .phase(Phase::Snapshot)
            .expect("snapshot phase")
            .current_live_bytes(),
        8
    );

    coordinator
        .transfer_reservation(&mut decode_bytes, Phase::Output)
        .expect("same-ledger phase transfer");
    let transferred = coordinator.ledger_snapshot();
    assert_eq!(
        transferred
            .phase(Phase::Decode)
            .expect("decode phase")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transferred
            .phase(Phase::Output)
            .expect("output phase")
            .current_live_bytes(),
        7
    );

    drop(snapshot_bytes);
    drop(decode_bytes);
    let released = coordinator.ledger_snapshot();
    assert_eq!(released.current_live_bytes(), 0);
    assert_eq!(released.peak_live_bytes(), 15);
    assert_eq!(
        released
            .phase(Phase::Output)
            .expect("output phase")
            .peak_live_bytes(),
        7
    );
}

#[test]
fn dimension_and_operation_live_limits_are_both_frozen_gates() {
    let mut dimension_limited = coordinator(64);
    let first = dimension_limited
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 12)
        .expect("first dimension reservation");
    assert!(matches!(
        dimension_limited
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 5),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 17,
            limit: 16,
        })
    ));
    drop(first);

    let mut operation_limited = coordinator(10);
    let first = operation_limited
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("first operation reservation");
    assert!(matches!(
        operation_limited
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 3),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 11,
            limit: 10,
        })
    ));
    drop(first);
}

#[test]
fn reservation_resize_tracks_peak_and_preserves_the_token_on_failure() {
    let mut coordinator = coordinator(12);
    let mut reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("initial reservation");

    coordinator
        .resize_reservation(&mut reservation, 10)
        .expect("reservation may grow before allocator entry");
    assert_eq!(reservation.bytes(), 10);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 10);
    assert_eq!(coordinator.ledger_snapshot().peak_live_bytes(), 10);

    coordinator
        .resize_reservation(&mut reservation, 6)
        .expect("unused envelope bytes may be released");
    let lowered = coordinator.ledger_snapshot();
    assert_eq!(reservation.bytes(), 6);
    assert_eq!(lowered.current_live_bytes(), 6);
    assert_eq!(lowered.peak_live_bytes(), 10);

    let before_failure = (
        lowered.current_live_bytes(),
        lowered.peak_live_bytes(),
        *lowered
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence"),
    );
    assert_eq!(
        coordinator.resize_reservation(&mut reservation, 13),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::AssetBytes,
            value: 13,
            limit: 12,
        })
    );
    assert_eq!(reservation.bytes(), 6);
    let after_failure = coordinator.ledger_snapshot();
    assert_eq!(
        (
            after_failure.current_live_bytes(),
            after_failure.peak_live_bytes(),
            *after_failure
                .dimension(Dimension::AssetBytes)
                .expect("asset evidence"),
        ),
        before_failure
    );

    drop(reservation);
    let released = coordinator.ledger_snapshot();
    assert_eq!(released.current_live_bytes(), 0);
    assert_eq!(released.peak_live_bytes(), 10);
}

#[test]
fn reservation_split_conserves_zero_exact_and_excess_bytes() {
    let mut coordinator = coordinator(20);
    let mut reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 10)
        .expect("initial reservation");

    let zero = coordinator
        .split_reservation(&mut reservation, 0)
        .expect("zero split is a no-op token");
    assert_eq!(zero.bytes(), 0);
    assert_eq!(reservation.bytes(), 10);
    drop(zero);

    assert!(matches!(
        coordinator.split_reservation(&mut reservation, 11),
        Err(LedgerCause::SplitExceedsReservation {
            requested: 11,
            available: 10,
        })
    ));
    assert_eq!(reservation.bytes(), 10);

    let exact = coordinator
        .split_reservation(&mut reservation, 10)
        .expect("exact split transfers all bytes");
    assert_eq!(reservation.bytes(), 0);
    assert_eq!(exact.bytes(), 10);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 10);
    drop(exact);
    drop(reservation);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn reservation_transition_atomically_changes_size_dimension_and_phase() {
    let mut coordinator = coordinator(32);
    let mut reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("source reservation");

    coordinator
        .transition_reservation(&mut reservation, Phase::Snapshot, Dimension::AssetBytes, 12)
        .expect("same-class growth");
    let grown = coordinator.ledger_snapshot();
    assert_eq!(grown.current_live_bytes(), 12);
    assert_eq!(grown.peak_live_bytes(), 12);
    assert_eq!(
        grown
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        12
    );

    coordinator
        .checkpoint()
        .transition_reservation(&mut reservation, Phase::Snapshot, Dimension::AssetBytes, 5)
        .expect("same-class shrink");
    let shrunk = coordinator.ledger_snapshot();
    assert_eq!(shrunk.current_live_bytes(), 5);
    assert_eq!(shrunk.peak_live_bytes(), 12);
    assert_eq!(
        shrunk
            .phase(Phase::Snapshot)
            .expect("snapshot phase")
            .current_live_bytes(),
        5
    );

    coordinator
        .transition_reservation(&mut reservation, Phase::Output, Dimension::OutputBytes, 10)
        .expect("cross-class transition");
    let transitioned = coordinator.ledger_snapshot();
    assert_eq!(transitioned.current_live_bytes(), 10);
    assert_eq!(transitioned.peak_live_bytes(), 12);
    assert_eq!(
        transitioned
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .peak_live_bytes(),
        12
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .current_live_bytes(),
        10
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .peak_live_bytes(),
        10
    );
    assert_eq!(
        transitioned
            .phase(Phase::Snapshot)
            .expect("snapshot phase")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .phase(Phase::Snapshot)
            .expect("snapshot phase")
            .peak_live_bytes(),
        12
    );
    assert_eq!(
        transitioned
            .phase(Phase::Output)
            .expect("output phase")
            .current_live_bytes(),
        10
    );
    assert_eq!(
        transitioned
            .phase(Phase::Output)
            .expect("output phase")
            .peak_live_bytes(),
        10
    );

    drop(reservation);
    let released = coordinator.ledger_snapshot();
    assert_eq!(released.current_live_bytes(), 0);
    assert_eq!(released.peak_live_bytes(), 12);
}

#[test]
fn failed_reservation_transition_preserves_source_without_target_pollution() {
    let mut dimension_limited = coordinator(32);
    let mut source = dimension_limited
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("dimension-limit source");
    let dimension_before = dimension_limited.ledger_snapshot();
    assert_eq!(
        dimension_limited
            .transition_reservation(&mut source, Phase::Output, Dimension::OutputBytes, 17)
            .expect_err("target dimension limit must fail"),
        LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 17,
            limit: 16,
        }
    );
    let dimension_failure = dimension_limited.ledger_snapshot();
    assert_eq!(source.bytes(), 8);
    assert_eq!(dimension_failure, dimension_before);
    dimension_limited
        .transition_reservation(&mut source, Phase::Output, Dimension::OutputBytes, 16)
        .expect("preserved source can be retried");
    assert_eq!(source.bytes(), 16);
    drop(source);
    assert_eq!(dimension_limited.ledger_snapshot().current_live_bytes(), 0);

    let mut operation_limited = coordinator(10);
    let mut source = operation_limited
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("operation-limit source");
    let operation_before = operation_limited.ledger_snapshot();
    assert_eq!(
        operation_limited
            .transition_reservation(&mut source, Phase::Output, Dimension::OutputBytes, 11)
            .expect_err("operation limit must fail"),
        LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 11,
            limit: 10,
        }
    );
    let operation_failure = operation_limited.ledger_snapshot();
    assert_eq!(source.bytes(), 8);
    assert_eq!(operation_failure, operation_before);
    drop(source);
    assert_eq!(operation_limited.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn repeated_failed_transitions_leave_predeclared_phase_evidence_unchanged() {
    let mut coordinator = coordinator(32);
    let mut source = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 8)
        .expect("source reservation");
    let before = coordinator.ledger_snapshot();

    for phase in [Phase::Decode, Phase::Output, Phase::Decode, Phase::Output] {
        assert_eq!(
            coordinator
                .transition_reservation(&mut source, phase, Dimension::OutputBytes, 17)
                .expect_err("target dimension limit must reject every transition"),
            LedgerCause::LimitExceeded {
                class: CounterClass::Live,
                dimension: Dimension::OutputBytes,
                value: 17,
                limit: 16,
            }
        );
        assert_eq!(source.bytes(), 8);
        assert_eq!(
            coordinator.ledger_snapshot(),
            before,
            "failed transition to {phase:?} changed the ledger or its evidence"
        );
    }

    drop(source);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn failed_batch_transition_preserves_every_source_token_atomically() {
    let mut coordinator = coordinator(32);
    let first = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("first source reservation");
    let second = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("second source reservation");
    let mut sources = [first, second];
    let before = coordinator.ledger_snapshot();

    assert_eq!(
        coordinator
            .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 17,)
            .expect_err("target dimension limit must reject the whole batch"),
        LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::OutputBytes,
            value: 17,
            limit: 16,
        }
    );
    assert_eq!(sources[0].bytes(), 4);
    assert_eq!(sources[1].bytes(), 4);
    let rejected = coordinator.ledger_snapshot();
    assert_eq!(rejected, before);

    coordinator
        .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 8)
        .expect("preserved batch can be retried");
    assert_eq!(sources[0].bytes(), 4);
    assert_eq!(sources[1].bytes(), 4);
    let transitioned = coordinator.ledger_snapshot();
    assert_eq!(transitioned.current_live_bytes(), 8);
    assert_eq!(
        transitioned
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .current_live_bytes(),
        8
    );

    drop(sources);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn mixed_batch_transition_conserves_operation_dimension_and_phase_totals() {
    let mut coordinator = coordinator(32);
    let first = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 4)
        .expect("asset source reservation");
    let second = coordinator
        .checkpoint()
        .reserve(Phase::Decode, Dimension::OutputBytes, 3)
        .expect("output source reservation");
    let mut sources = [first, second];

    coordinator
        .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 9)
        .expect("mixed batch transition should fit");

    assert_eq!(sources[0].bytes(), 4);
    assert_eq!(sources[1].bytes(), 5);
    let transitioned = coordinator.ledger_snapshot();
    assert_eq!(transitioned.current_live_bytes(), 9);
    assert_eq!(transitioned.peak_live_bytes(), 9);
    assert_eq!(
        transitioned
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .current_live_bytes(),
        9
    );
    assert_eq!(
        transitioned
            .phase(Phase::Snapshot)
            .expect("snapshot evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .phase(Phase::Decode)
            .expect("decode evidence")
            .current_live_bytes(),
        0
    );
    assert_eq!(
        transitioned
            .phase(Phase::Output)
            .expect("output phase evidence")
            .current_live_bytes(),
        9
    );

    drop(sources);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn complete_phase_catalog_must_fit_evidence_before_operation_start() {
    let probe = Coordinator::try_new(OperationControl::default(), limits(20), &[Phase::Snapshot])
        .expect("one phase should fit without starting an operation");
    let one_phase_evidence_limit = probe.ledger_snapshot().current_evidence_bytes();
    probe
        .finish()
        .expect("the evidence probe should finalize cleanly");

    let tight = limits(20)
        .try_with_evidence_byte_limit(one_phase_evidence_limit)
        .expect("one phase should fit exactly");
    let failure = match Coordinator::try_new(OperationControl::default(), tight, COMPLETE_PHASES) {
        Ok(_) => panic!("the complete phase catalog must be admitted before operation start"),
        Err(failure) => failure,
    };

    assert!(matches!(
        failure,
        LedgerCause::Evidence {
            cause: LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::Phases,
                ..
            }
        }
    ));
}

#[test]
fn batch_transition_rejects_owner_mismatch_and_pending_mutation_atomically() {
    let mut owner = coordinator(20);
    let mut other = coordinator(20);
    let first = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
        .expect("first owner reservation");
    let second = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
        .expect("second owner reservation");
    let mut sources = [first, second];
    assert_eq!(
        other
            .transition_reservations(&mut sources, Phase::Output, Dimension::OutputBytes, 4)
            .expect_err("another ledger cannot transition the batch"),
        LedgerCause::OwnerMismatch
    );
    assert_eq!(sources[0].bytes(), 2);
    assert_eq!(sources[1].bytes(), 2);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 4);
    assert_eq!(other.ledger_snapshot().current_live_bytes(), 0);
    drop(sources);

    let first = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("first zero-byte pending fixture");
    let second = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("second zero-byte pending fixture");
    let mut zero_sources = [first, second];
    let mut batch = PublicationBatch::default();
    let pending = owner
        .prepare_publication(&mut batch, &[])
        .expect("zero live bytes are fully covered");
    let before_pending_rejection = owner.ledger_snapshot();
    assert_eq!(
        owner
            .transition_reservations(&mut zero_sources, Phase::Output, Dimension::OutputBytes, 3,)
            .expect_err("pending publication seals batch transitions"),
        LedgerCause::PublicationAlreadyPending
    );
    assert_eq!(zero_sources[0].bytes(), 0);
    assert_eq!(zero_sources[1].bytes(), 0);
    assert_eq!(owner.ledger_snapshot(), before_pending_rejection);
    drop(zero_sources);
    pending.rollback();
}

#[test]
fn reservation_transition_rejects_owner_mismatch_and_pending_mutation() {
    let mut owner = coordinator(20);
    let mut other = coordinator(20);
    let mut source = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 3)
        .expect("owner source");
    assert_eq!(
        other
            .transition_reservation(&mut source, Phase::Output, Dimension::OutputBytes, 3)
            .expect_err("another ledger cannot transition this token"),
        LedgerCause::OwnerMismatch
    );
    assert_eq!(source.bytes(), 3);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 3);
    assert_eq!(other.ledger_snapshot().current_live_bytes(), 0);
    drop(source);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 0);

    let mut zero = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("zero-byte pending fixture");
    let mut batch = PublicationBatch::default();
    let pending = owner
        .prepare_publication(&mut batch, &[])
        .expect("zero live bytes are fully covered");
    let before_pending_rejection = owner.ledger_snapshot();
    assert_eq!(
        owner
            .checkpoint()
            .transition_reservation(&mut zero, Phase::Output, Dimension::OutputBytes, 3)
            .expect_err("pending publication seals transitions"),
        LedgerCause::PublicationAlreadyPending
    );
    let pending_snapshot = owner.ledger_snapshot();
    assert_eq!(pending_snapshot, before_pending_rejection);
    assert_eq!(zero.bytes(), 0);
    drop(zero);
    pending.rollback();
}

#[test]
fn same_owner_type_cannot_transfer_across_ledger_instances() {
    let mut owner = publication_coordinator(20);
    let mut other = coordinator(20);
    let mut reservation = owner
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 3)
        .expect("owner reservation");

    assert_eq!(
        other.transfer_reservation(&mut reservation, Phase::Decode),
        Err(LedgerCause::OwnerMismatch)
    );
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 3);
    assert_eq!(other.ledger_snapshot().current_live_bytes(), 0);

    let mut batch = publication_batch(&mut owner, [reservation]);
    assert!(matches!(
        other.prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 1)],),
        Err(LedgerCause::OwnerMismatch)
    ));
    assert_eq!(
        owner.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 3
    );
    assert_eq!(other.ledger_snapshot().current_live_bytes(), 0);
    drop(batch);
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 0);
}
