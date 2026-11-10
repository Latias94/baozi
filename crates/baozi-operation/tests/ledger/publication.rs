use super::*;

fn publication_limits_with_four_commitment_dimensions(
    payload_live_bytes: u64,
) -> LedgerLimits<Dimension> {
    LedgerLimits::new(
        payload_live_bytes
            .checked_add(publication_capacity_bytes())
            .expect("publication test live limit must be representable"),
        [
            (
                Dimension::AssetBytes,
                DimensionPolicy::reservable_bytes(
                    64,
                    64,
                    16,
                    publication_capacity_bytes()
                        .checked_add(20)
                        .expect("test asset live limit must be representable"),
                ),
            ),
            (
                Dimension::OutputBytes,
                DimensionPolicy::reservable_bytes(64, 64, 16, 16),
            ),
            (Dimension::Records, DimensionPolicy::counter(32, 32, 8)),
            (Dimension::Unknown, DimensionPolicy::counter(32, 32, 8)),
        ],
    )
    .expect("valid four-dimension publication policy")
}

#[test]
fn publication_must_take_every_live_byte_or_commit_nothing() {
    let mut coordinator = publication_coordinator(20);
    let retained = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 2)
        .expect("retained snapshot reservation");
    let submitted = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 3)
        .expect("submitted output reservation");

    let mut batch = publication_batch(&mut coordinator, [submitted]);
    assert!(matches!(
        coordinator.prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)],),
        Err(LedgerCause::PublicationLiveMismatch {
            transaction_bytes,
            ledger_bytes,
        }) if transaction_bytes == publication_capacity_bytes() + 3
            && ledger_bytes == publication_capacity_bytes() + 5
    ));
    let evidence = coordinator.ledger_snapshot();
    assert_eq!(
        evidence.current_live_bytes(),
        publication_capacity_bytes() + 5
    );
    assert_eq!(
        evidence
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    drop(batch);
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 2);
    drop(retained);
}

#[test]
fn pending_and_rolled_back_boundaries_seal_all_accounting_mutation() {
    let mut coordinator = coordinator(20);
    let mut zero = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("zero-byte linear token");
    let mut first_batch = PublicationBatch::default();
    let pending = coordinator
        .prepare_publication(&mut first_batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("zero live bytes are fully covered");

    assert_eq!(
        coordinator
            .checkpoint()
            .charge_attempted(Dimension::Records, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert!(matches!(
        coordinator
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    assert_eq!(
        coordinator.transition_reservation(&mut zero, Phase::Output, Dimension::AssetBytes, 0),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert!(matches!(
        coordinator.split_reservation(&mut zero, 0),
        Err(LedgerCause::PublicationAlreadyPending)
    ));

    pending.rollback();
    assert_eq!(
        coordinator.transition_reservation(&mut zero, Phase::Output, Dimension::AssetBytes, 0),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert!(matches!(
        coordinator.split_reservation(&mut zero, 0),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    assert_eq!(
        coordinator
            .checkpoint()
            .charge_attempted(Dimension::Records, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(
        coordinator
            .checkpoint()
            .charge_consumed(Dimension::Records, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert!(matches!(
        coordinator
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    ));

    let mut replacement_batch = PublicationBatch::default();
    assert!(matches!(
        coordinator.prepare_publication(
            &mut replacement_batch,
            &[Commitment::new(Dimension::Records, 1)],
        ),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    let records = coordinator
        .dimension_evidence(Dimension::Records)
        .expect("record evidence");
    assert_eq!((records.attempted(), records.consumed()), (0, 0));
    assert_eq!(records.pending_publication(), 0);
    assert_eq!(records.committed(), 0);
    drop(zero);
    let expected_administrative_bytes =
        coordinator.ledger_snapshot().current_administrative_bytes();
    let snapshot = coordinator
        .finish()
        .expect("rollback releases every token without reopening");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
    assert_eq!(
        snapshot.current_administrative_bytes(),
        expected_administrative_bytes
    );
}

#[test]
fn published_boundary_seals_all_accounting_mutation_on_a_fresh_operation() {
    let mut coordinator = coordinator(20);
    let mut zero = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 0)
        .expect("fresh zero-byte linear token");
    let mut batch = PublicationBatch::default();
    let mut pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("fresh operation publication transaction");
    coordinator
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("durable publication");
    assert_eq!(
        coordinator
            .checkpoint()
            .charge_attempted(Dimension::Records, 1),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    assert_eq!(
        coordinator.transition_reservation(&mut zero, Phase::Snapshot, Dimension::AssetBytes, 0),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    drop(zero);
    let snapshot = coordinator
        .finish()
        .expect("published operation finalizes cleanly");
    assert_eq!(
        snapshot.dimension(Dimension::Records).unwrap().committed(),
        1
    );
    assert_eq!(snapshot.active_reservations(), 0);
}

#[test]
fn pending_drop_releases_live_and_commitments_without_reopening_the_operation() {
    let mut coordinator = publication_coordinator(20);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 6)
        .expect("output reservation");
    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let pending = coordinator
        .prepare_publication(
            &mut batch,
            &[
                Commitment::new(Dimension::OutputBytes, 5),
                Commitment::new(Dimension::Records, 2),
            ],
        )
        .expect("pending output transaction");

    let before_drop = coordinator.ledger_snapshot();
    assert_eq!(
        before_drop.current_live_bytes(),
        publication_capacity_bytes() + 6
    );
    assert_eq!(
        before_drop
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .pending_publication(),
        5
    );
    assert_eq!(
        before_drop
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .committed(),
        0
    );

    drop(pending);
    let rolled_back = coordinator.ledger_snapshot();
    assert_eq!(rolled_back.current_live_bytes(), 0);
    assert_eq!(rolled_back.active_reservations(), 0);
    assert_eq!(
        rolled_back
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .pending_publication(),
        0
    );
    assert_eq!(
        rolled_back
            .dimension(Dimension::Records)
            .expect("record evidence")
            .pending_publication(),
        0
    );
    assert_eq!(
        rolled_back
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    assert_eq!(
        rolled_back
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .committed(),
        0
    );
    assert!(matches!(
        coordinator
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 4),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    let mut blocked_batch = PublicationBatch::default();
    assert!(matches!(
        coordinator.prepare_publication(
            &mut blocked_batch,
            &[Commitment::new(Dimension::Records, 1)],
        ),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    let snapshot = coordinator
        .finish()
        .expect("rolled-back operation can finalize");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.peak_live_bytes(), publication_capacity_bytes() + 6);
    assert_eq!(
        snapshot.current_administrative_bytes(),
        rolled_back.current_administrative_bytes()
    );

    let mut fresh = publication_coordinator(20);
    let replacement = fresh
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("fresh operation reservation");
    let mut replacement_batch = publication_batch(&mut fresh, [replacement]);
    let mut replacement = fresh
        .prepare_publication(
            &mut replacement_batch,
            &[Commitment::new(Dimension::Records, 1)],
        )
        .expect("a separate ordinary operation admits publication");
    fresh
        .publish(
            &mut replacement,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("fresh operation publishes");
    assert_eq!(
        fresh
            .ledger_snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        1
    );
    let snapshot = fresh
        .finish()
        .expect("fresh publication releases all live storage");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.peak_live_bytes(), publication_capacity_bytes() + 4);
}

#[test]
fn publication_batch_releases_accounted_vector_capacity_on_rollback() {
    let mut coordinator = publication_coordinator(20);
    let payload = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 4)
        .expect("payload reservation");
    let mut batch = publication_batch(&mut coordinator, [payload]);
    let pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("the batch owns every live byte");

    assert_eq!(
        coordinator.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 4
    );
    pending.rollback();
    let rolled_back = coordinator.ledger_snapshot();
    assert_eq!(rolled_back.current_live_bytes(), 0);
    assert!(matches!(
        coordinator
            .checkpoint()
            .reserve(Phase::Output, Dimension::OutputBytes, 5),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    let snapshot = coordinator
        .finish()
        .expect("rollback releases payload and vector capacity");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
    assert_eq!(snapshot.peak_live_bytes(), publication_capacity_bytes() + 4);
    assert_eq!(
        snapshot
            .dimension(Dimension::AssetBytes)
            .unwrap()
            .current_live_bytes(),
        0
    );
    assert_eq!(
        snapshot
            .dimension(Dimension::AssetBytes)
            .unwrap()
            .peak_live_bytes(),
        publication_capacity_bytes()
    );
    assert_eq!(
        snapshot
            .dimension(Dimension::OutputBytes)
            .unwrap()
            .current_live_bytes(),
        0
    );
    assert_eq!(
        snapshot.dimension(Dimension::Records).unwrap().committed(),
        0
    );
    assert_eq!(
        snapshot.current_administrative_bytes(),
        rolled_back.current_administrative_bytes()
    );
}

#[test]
fn publication_batch_releases_accounted_vector_capacity_on_fresh_operation_publish() {
    let mut coordinator = publication_coordinator(20);
    let payload = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 5)
        .expect("fresh payload reservation");
    let mut batch = publication_batch(&mut coordinator, [payload]);
    let mut pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("fresh batch owns every live byte");
    coordinator
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("publication releases both reservation classes");

    let snapshot = coordinator
        .finish()
        .expect("published capacity leaves no live token");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.active_reservations(), 0);
    assert_eq!(snapshot.peak_live_bytes(), publication_capacity_bytes() + 5);
    assert_eq!(
        snapshot
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        1
    );
}

#[test]
fn publication_batch_rejects_missing_or_insufficient_capacity_coverage() {
    let mut missing = publication_coordinator(8);
    let payload = missing
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("missing-capacity payload reservation");
    let error = PublicationReservationBatch::try_new(vec![payload], None)
        .expect_err("an allocated batch requires a capacity token");
    assert!(matches!(
        error.kind(),
        PublicationReservationBatchErrorKind::MissingCapacityReservation { required_bytes }
            if required_bytes == publication_capacity_bytes()
    ));
    drop(error);
    assert_eq!(missing.ledger_snapshot().current_live_bytes(), 0);

    let mut insufficient = publication_coordinator(8);
    let capacity = insufficient
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 1)
        .expect("insufficient capacity token");
    let payload = insufficient
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("insufficient-capacity payload reservation");
    let error = PublicationReservationBatch::try_new(vec![payload], Some(capacity))
        .expect_err("an undersized capacity token cannot cover the batch allocation");
    assert!(matches!(
        error.kind(),
        PublicationReservationBatchErrorKind::InsufficientCapacityReservation {
            required_bytes,
            reserved_bytes: 1,
        } if required_bytes == publication_capacity_bytes()
    ));
    drop(error);
    assert_eq!(insufficient.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn rejected_publication_batch_releases_its_complete_owner_during_unwind() {
    let mut coordinator = publication_coordinator(8);
    let capacity = coordinator
        .checkpoint()
        .reserve(Phase::Snapshot, Dimension::AssetBytes, 1)
        .expect("rejected batch capacity token");
    let payload = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("rejected batch payload token");

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _error = PublicationReservationBatch::try_new(vec![payload], Some(capacity))
            .expect_err("the capacity token deliberately under-covers the vector");
        panic!("exercise rejected batch unwind ownership");
    }));

    assert!(unwind.is_err());
    assert_eq!(coordinator.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn publication_aggregates_duplicate_commitments_atomically() {
    let mut coordinator = publication_coordinator(20);
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 9)
        .expect("output reservation");
    let mut batch = publication_batch(&mut coordinator, [reservation]);
    let mut pending = coordinator
        .prepare_publication(
            &mut batch,
            &[
                Commitment::new(Dimension::OutputBytes, 3),
                Commitment::new(Dimension::OutputBytes, 4),
                Commitment::new(Dimension::Records, 2),
            ],
        )
        .expect("duplicate dimensions aggregate");

    coordinator
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 9),
        )
        .expect("atomic publication");

    let snapshot = coordinator
        .finish()
        .expect("publishing every live token makes finalization infallible");
    assert_eq!(snapshot.current_live_bytes(), 0);
    assert_eq!(snapshot.peak_live_bytes(), publication_capacity_bytes() + 9);
    assert_eq!(
        snapshot
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .committed(),
        7
    );
    assert_eq!(
        snapshot
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        2
    );
}

#[test]
fn commitment_failures_preserve_live_batch_for_retry() {
    let overflow_limits = LedgerLimits::new(
        8 + publication_capacity_bytes(),
        [
            (
                Dimension::AssetBytes,
                DimensionPolicy::reservable_bytes(0, 0, 0, publication_capacity_bytes()),
            ),
            (
                Dimension::OutputBytes,
                DimensionPolicy::reservable_bytes(8, 8, u64::MAX, 8),
            ),
        ],
    )
    .expect("overflow fixture policy");
    let mut overflow = coordinator_with(OperationControl::default(), overflow_limits);
    let reservation = overflow
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("overflow fixture reservation");
    let mut batch = publication_batch(&mut overflow, [reservation]);
    assert!(matches!(
        overflow.prepare_publication(
            &mut batch,
            &[
                Commitment::new(Dimension::OutputBytes, u64::MAX),
                Commitment::new(Dimension::OutputBytes, 1),
            ],
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::PendingPublication,
            dimension: Dimension::OutputBytes,
        })
    ));
    assert_eq!(
        overflow.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 1
    );
    assert!(!batch.is_empty());
    let pending = overflow
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 1)])
        .expect("overflow failure leaves the batch retryable");
    assert!(batch.is_empty());
    pending.rollback();
    assert_eq!(overflow.ledger_snapshot().current_live_bytes(), 0);

    let mut unknown = publication_coordinator(20);
    let reservation = unknown
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("unknown fixture reservation");
    let mut batch = publication_batch(&mut unknown, [reservation]);
    assert!(matches!(
        unknown.prepare_publication(&mut batch, &[Commitment::new(Dimension::Unknown, 1)],),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert_eq!(
        unknown.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 1
    );
    assert!(!batch.is_empty());
    let pending = unknown
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("unknown-dimension failure leaves the batch retryable");
    pending.rollback();
    assert_eq!(unknown.ledger_snapshot().current_live_bytes(), 0);

    let mut limited = publication_coordinator(20);
    let reservation = limited
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("limit fixture reservation");
    let mut batch = publication_batch(&mut limited, [reservation]);
    assert!(matches!(
        limited.prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 17)],),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Committed,
            dimension: Dimension::OutputBytes,
            value: 17,
            limit: 16,
        })
    ));
    let evidence = limited.ledger_snapshot();
    assert_eq!(
        evidence.current_live_bytes(),
        publication_capacity_bytes() + 1
    );
    assert_eq!(
        evidence
            .dimension(Dimension::OutputBytes)
            .expect("output evidence")
            .committed(),
        0
    );
    assert!(!batch.is_empty());
    let pending = limited
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::OutputBytes, 1)])
        .expect("commitment-limit failure leaves the batch retryable");
    pending.rollback();
    assert_eq!(limited.ledger_snapshot().current_live_bytes(), 0);
}

#[test]
fn publication_evidence_failure_preserves_live_batch_for_retry() {
    let mut coordinator = coordinator_with_tight_evidence_limit(|| {
        publication_limits_with_four_commitment_dimensions(20)
    });
    let reservation = coordinator
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 1)
        .expect("the prepublication envelope should fit");
    let mut batch = publication_batch(&mut coordinator, [reservation]);

    let failure = coordinator
        .prepare_publication(
            &mut batch,
            &[
                Commitment::new(Dimension::AssetBytes, 1),
                Commitment::new(Dimension::OutputBytes, 1),
                Commitment::new(Dimension::Records, 1),
                Commitment::new(Dimension::Unknown, 1),
            ],
        )
        .expect_err("the four-entry aggregate must exceed the tight evidence bound");
    assert!(matches!(
        failure,
        LedgerCause::Evidence {
            cause: LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::Publication,
                ..
            }
        }
    ));
    assert!(!batch.is_empty());
    let output = coordinator
        .dimension_evidence(Dimension::OutputBytes)
        .expect("output evidence");
    assert_eq!(output.current_live_bytes(), 1);
    assert_eq!(output.pending_publication(), 0);

    let pending = coordinator
        .prepare_publication(&mut batch, &[])
        .expect("the preserved batch remains retryable without commitment evidence");
    assert!(batch.is_empty());
    pending.rollback();
    assert_eq!(
        coordinator
            .dimension_evidence(Dimension::OutputBytes)
            .expect("rolled-back output evidence")
            .current_live_bytes(),
        0
    );
}

#[test]
fn only_one_pending_or_published_batch_is_admitted() {
    let mut coordinator = coordinator(20);
    let mut first_batch = PublicationBatch::default();
    let pending = coordinator
        .prepare_publication(&mut first_batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("first pending batch");
    let mut blocked_batch = PublicationBatch::default();
    assert!(matches!(
        coordinator.prepare_publication(&mut blocked_batch, &[]),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    pending.rollback();

    assert!(matches!(
        coordinator.prepare_publication(
            &mut blocked_batch,
            &[Commitment::new(Dimension::Records, 1)],
        ),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    assert_eq!(
        coordinator
            .dimension_evidence(Dimension::Records)
            .unwrap()
            .pending_publication(),
        0
    );
    assert_eq!(
        coordinator
            .dimension_evidence(Dimension::Records)
            .unwrap()
            .committed(),
        0
    );
    coordinator
        .finish()
        .expect("rolled-back operation has no outstanding owner");

    let mut fresh = publication_coordinator(20);
    let mut pending = fresh
        .prepare_publication(
            &mut blocked_batch,
            &[Commitment::new(Dimension::Records, 1)],
        )
        .expect("fresh operation admits its first batch");
    fresh
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("first durable batch");
    let mut completed_batch = PublicationBatch::default();
    assert!(matches!(
        fresh.prepare_publication(
            &mut completed_batch,
            &[Commitment::new(Dimension::Unknown, u64::MAX)],
        ),
        Err(LedgerCause::PublicationAlreadyCompleted)
    ));
    let snapshot = fresh.finish().expect("fresh durable publication finalizes");
    assert_eq!(
        snapshot.dimension(Dimension::Records).unwrap().committed(),
        1
    );
}

#[test]
fn finish_with_pending_publication_preserves_its_typed_cause_and_exact_evidence() {
    let mut coordinator = coordinator(20);
    let mut batch = PublicationBatch::default();
    let pending = coordinator
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("one pending commitment without live storage");

    let failure = coordinator
        .finish()
        .expect_err("pending publication prevents clean generic finalization");
    assert_eq!(
        failure.cause(),
        &PublicationCause::Ledger(LedgerCause::PendingPublicationsRemain)
    );
    let records = failure
        .snapshot()
        .dimension(Dimension::Records)
        .expect("record evidence");
    assert_eq!(records.pending_publication(), 1);
    assert_eq!(records.committed(), 0);
    assert_eq!(failure.snapshot().current_live_bytes(), 0);

    pending.rollback();
    assert_eq!(
        failure
            .snapshot()
            .dimension(Dimension::Records)
            .unwrap()
            .pending_publication(),
        1,
        "later rollback cannot rewrite the immutable evidence captured at finish"
    );
}

#[test]
fn wrong_coordinator_rejects_pending_before_progress_and_preserves_it_for_owner() {
    use std::sync::{Arc, Mutex};

    let observed = Arc::new(Mutex::new(0usize));
    let observed_by_other = Arc::clone(&observed);
    let mut owner = publication_coordinator(20);
    let mut other = coordinator_with(
        OperationControl::default().with_progress_observer(move |_| {
            *observed_by_other.lock().expect("observer lock") += 1;
            baozi_operation::ProgressDecision::Continue
        }),
        limits(20),
    );
    let reservation = owner
        .checkpoint()
        .reserve(Phase::Output, Dimension::OutputBytes, 3)
        .expect("owner reservation");
    let mut batch = publication_batch(&mut owner, [reservation]);
    let mut pending = owner
        .prepare_publication(&mut batch, &[Commitment::new(Dimension::Records, 1)])
        .expect("owner pending transaction");

    assert_eq!(
        other.publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        ),
        Err(PublicationCause::Ledger(LedgerCause::OwnerMismatch))
    );
    assert_eq!(*observed.lock().expect("observer lock"), 0);
    assert_eq!(
        owner.ledger_snapshot().current_live_bytes(),
        publication_capacity_bytes() + 3
    );
    assert_eq!(
        owner
            .ledger_snapshot()
            .dimension(Dimension::Records)
            .expect("record evidence")
            .committed(),
        0
    );
    owner
        .publish(
            &mut pending,
            FinalProgress::new(ObserverPhase::Publish, Unit::Work, 1),
        )
        .expect("the owning coordinator can publish the preserved transaction");
    assert_eq!(owner.ledger_snapshot().current_live_bytes(), 0);
}
