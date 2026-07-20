use super::*;

#[test]
fn finish_error_preserves_real_live_and_pending_evidence() {
    let mut live_coordinator = coordinator(20);
    let live = live_coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 7)
        .expect("live fixture reservation");
    let live_error = live_coordinator
        .finish()
        .expect_err("live bytes prevent clean finalization");
    assert_eq!(
        live_error.cause(),
        &LedgerCause::LiveReservationsRemain { bytes: 7 }
    );
    assert_eq!(live_error.snapshot().current_live_bytes(), 7);
    assert_eq!(live_error.snapshot().peak_live_bytes(), 7);
    drop(live);

    let mut combined_coordinator = publication_coordinator(20);
    let combined_live = combined_coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("combined fixture reservation");
    let mut combined_batch = publication_batch(&mut combined_coordinator, [combined_live]);
    let combined_pending = combined_coordinator
        .prepare_publication(
            &mut combined_batch,
            &[Commitment::new(Dimension::Records, 2)],
        )
        .expect("combined pending transaction");
    let combined_error = combined_coordinator
        .finish()
        .expect_err("pending live output prevents finalization");
    assert_eq!(
        combined_error.snapshot().current_live_bytes(),
        publication_capacity_bytes() + 4
    );
    assert_eq!(
        combined_error
            .snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .pending_publication(),
        2
    );
    assert_eq!(
        combined_error
            .snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    drop(combined_pending);

    let mut pending_coordinator = coordinator(20);
    let mut pending_batch = PublicationBatch::default();
    let pending = pending_coordinator
        .prepare_publication(&mut pending_batch, &[])
        .expect("even an empty transaction has linear pending state");
    let pending_error = pending_coordinator
        .finish()
        .expect_err("empty pending transaction cannot disappear from evidence");
    let (cause, snapshot) = pending_error.into_parts();
    assert_eq!(cause, LedgerCause::PendingPublicationsRemain);
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert!(
        snapshot
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .pending_publication()
            == 0
    );
    drop(pending);
}

#[test]
fn checkpoint_exposes_control_and_accounting_without_state_authority() {
    let mut coordinator = coordinator(20);
    {
        let mut checkpoint = coordinator.checkpoint();
        poll_core_work(&mut checkpoint).expect("object-safe polling remains active");
        checkpoint
            .report_progress(ObserverPhase::Work, Unit::Work, 1, None)
            .expect("owner-defined progress");
        checkpoint
            .charge_attempted(Dimension::AssetBytes, 2)
            .expect("checked attempted work");
        let mut reservation = checkpoint
            .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
            .expect("checked storage");
        let split = checkpoint
            .split(&mut reservation, 1)
            .expect("checkpoint may partition live ownership");
        drop(split);
        drop(reservation);
    }

    let snapshot = coordinator.ledger_snapshot();
    assert_eq!(
        snapshot
            .dimension(Dimension::AssetBytes)
            .expect("asset evidence")
            .attempted(),
        2
    );
    assert_eq!(snapshot.current_live_bytes(), 0);
}

#[test]
fn frozen_dimension_and_complete_phase_evidence_obey_the_independent_hard_limit() {
    assert!(matches!(
        LedgerLimits::with_evidence_byte_limit(
            20,
            0,
            [(
                Dimension::AssetBytes,
                DimensionPolicy::reservable_bytes(20, 20, 0, 20),
            )],
        ),
        Err(LedgerLimitsError::Evidence {
            cause: LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::Dimensions,
                ..
            },
        })
    ));

    let probe = coordinator(20);
    let frozen_bytes = probe.ledger_snapshot().current_evidence_bytes();
    probe
        .finish()
        .expect("the complete phase evidence probe finalizes cleanly");
    let tight = limits(20)
        .try_with_evidence_byte_limit(frozen_bytes)
        .expect("the complete frozen allocation exactly fits its own bound");
    let mut coordinator = coordinator_with(OperationControl::default(), tight);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 1)
        .expect("runtime reservation reuses the predeclared phase evidence");
    drop(reservation);
    let snapshot = coordinator
        .finish()
        .expect("predeclared metadata leaves no live reservation");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.current_evidence_bytes(), 0);
    assert!(snapshot.peak_evidence_bytes() >= frozen_bytes);
}

#[test]
fn zero_byte_reservation_capabilities_must_be_released_before_clean_finalization() {
    let mut coordinator = coordinator(20);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("zero-byte reservation remains a linear capability");
    let error = coordinator
        .finish()
        .expect_err("an active capability retains ledger state even without payload bytes");
    assert_eq!(
        error.cause(),
        &LedgerCause::ActiveReservationsRemain { count: 1 }
    );
    assert_eq!(error.snapshot().current_live_bytes(), 0);
    assert_eq!(error.snapshot().active_reservations(), 1);
    assert!(error.snapshot().current_evidence_bytes() > 0);
    drop(reservation);
}

#[test]
fn evidence_peak_covers_snapshot_and_publication_storage_then_finishes_at_zero() {
    let mut coordinator = publication_coordinator(20);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("output reservation");
    let before_publication = coordinator
        .try_ledger_snapshot()
        .expect("bounded interim evidence");
    assert!(before_publication.current_evidence_bytes() > 0);
    assert!(before_publication.peak_evidence_bytes() > before_publication.current_evidence_bytes());

    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let before_prepare = coordinator
        .try_ledger_snapshot()
        .expect("accounted publication batch evidence");
    let pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 4)])
        .expect("bounded sorted commitment evidence");
    let while_pending = coordinator
        .try_ledger_snapshot()
        .expect("pending evidence snapshot");
    assert!(while_pending.current_evidence_bytes() > before_prepare.current_evidence_bytes());
    drop(pending);
    let after_rollback = coordinator
        .try_ledger_snapshot()
        .expect("rollback evidence snapshot");
    assert_eq!(
        after_rollback.current_evidence_bytes(),
        before_prepare.current_evidence_bytes()
    );
    let snapshot = coordinator.finish().expect("clean final evidence");
    assert_eq!(snapshot.current_evidence_bytes(), 0);
    assert!(
        snapshot.peak_evidence_bytes() >= while_pending.peak_evidence_bytes(),
        "publication and snapshot envelopes remain visible in the high-water mark"
    );
}

#[test]
fn snapshot_iteration_preserves_sorted_dimensions_and_declared_phase_lifecycle() {
    let limits = LedgerLimits::new(
        20,
        [
            (Dimension::Records, DimensionPolicy::counter(8, 8, 8)),
            (
                Dimension::OutputBytes,
                DimensionPolicy::reservable_bytes(20, 20, 20, 20),
            ),
            (
                Dimension::AssetBytes,
                DimensionPolicy::reservable_bytes(20, 20, 20, 20),
            ),
        ],
    )
    .expect("out-of-order policy is valid");
    let mut coordinator = coordinator_with(OperationControl::default(), limits);
    let output = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("output phase");
    let snapshot = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 1)
        .expect("snapshot phase");
    drop(output);
    drop(snapshot);
    let evidence = coordinator.finish().expect("sorted final evidence");
    assert_eq!(
        evidence
            .dimensions()
            .map(|(dimension, _)| dimension)
            .collect::<Vec<_>>(),
        vec![
            Dimension::AssetBytes,
            Dimension::OutputBytes,
            Dimension::Records,
        ]
    );
    assert_eq!(
        evidence
            .phases()
            .map(|(phase, _)| phase)
            .collect::<Vec<_>>(),
        vec![Phase::Snapshot, Phase::Decode, Phase::Output]
    );
}

fn poll_core_work(poll: &mut dyn OperationPoll) -> Result<(), baozi_operation::ControlCause> {
    poll.poll()
}
