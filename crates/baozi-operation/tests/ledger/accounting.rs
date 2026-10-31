use super::*;

#[test]
fn ledger_limits_reject_invalid_or_duplicate_dimension_policy() {
    assert_eq!(
        LedgerLimits::new(
            4,
            [(Dimension::Records, DimensionPolicy::counter(3, 4, 0),)],
        ),
        Err(LedgerLimitsError::ConsumedLimitExceedsAttempted {
            dimension: Dimension::Records,
            consumed_limit: 4,
            attempted_limit: 3,
        })
    );

    assert_eq!(
        LedgerLimits::new(
            4,
            [
                (Dimension::Records, DimensionPolicy::counter(4, 4, 0)),
                (Dimension::Records, DimensionPolicy::counter(5, 5, 0)),
            ],
        ),
        Err(LedgerLimitsError::DuplicateDimension {
            dimension: Dimension::Records,
        })
    );
}

#[test]
fn attempted_and_consumed_are_non_refundable_and_preserve_their_invariant() {
    let mut coordinator = coordinator(32);
    let mut checkpoint = coordinator.checkpoint();

    checkpoint
        .charge_attempted(Dimension::AssetBytes, 8)
        .expect("first attempt fits");
    let attempted_error = checkpoint
        .charge_attempted(Dimension::AssetBytes, 4)
        .expect_err("crossing the frozen limit fails");
    assert_eq!(
        attempted_error,
        LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::AssetBytes,
            value: 12,
            limit: 10,
        }
    );

    checkpoint
        .charge_consumed(Dimension::AssetBytes, 5)
        .expect("first consumption fits");
    let consumed_error = checkpoint
        .charge_consumed(Dimension::AssetBytes, 4)
        .expect_err("crossing the frozen consumed limit fails");
    assert_eq!(
        consumed_error,
        LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::AssetBytes,
            value: 9,
            limit: 8,
        }
    );

    let invariant_error = checkpoint
        .charge_consumed(Dimension::AssetBytes, 4)
        .expect_err("consumption cannot outrun attempted work");
    assert_eq!(
        invariant_error,
        LedgerCause::ConsumedExceedsAttempted {
            dimension: Dimension::AssetBytes,
            consumed: 13,
            attempted: 12,
        }
    );
    let snapshot = coordinator.ledger_snapshot();
    let asset = snapshot
        .dimension(Dimension::AssetBytes)
        .expect("asset evidence");
    assert_eq!(asset.attempted(), 12);
    assert_eq!(asset.consumed(), 9);
    assert!(asset.consumed() <= asset.attempted());
}

#[test]
fn checked_counter_overflow_keeps_the_last_valid_evidence() {
    let limits = LedgerLimits::new(
        0,
        [(
            Dimension::Records,
            DimensionPolicy::counter(u64::MAX, u64::MAX, 0),
        )],
    )
    .expect("maximum counters are valid");
    let mut coordinator = coordinator_with(OperationControl::default(), limits);
    let mut checkpoint = coordinator.checkpoint();

    checkpoint
        .charge_attempted(Dimension::Records, u64::MAX)
        .expect("maximum attempted value fits");
    assert_eq!(
        checkpoint.charge_attempted(Dimension::Records, 1),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
        })
    );
    checkpoint
        .charge_consumed(Dimension::Records, u64::MAX)
        .expect("maximum consumed value fits attempted");
    assert_eq!(
        checkpoint.charge_consumed(Dimension::Records, 1),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
        })
    );
    let evidence = coordinator
        .ledger_snapshot()
        .dimension(Dimension::Records)
        .copied()
        .expect("record evidence");
    assert_eq!(evidence.attempted(), u64::MAX);
    assert_eq!(evidence.consumed(), u64::MAX);
}

#[test]
fn unknown_and_counter_only_dimensions_reject_live_reservations() {
    let mut coordinator = coordinator(32);
    let mut checkpoint = coordinator.checkpoint();

    assert_eq!(
        checkpoint.charge_attempted(Dimension::Unknown, 1),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    );
    assert!(matches!(
        checkpoint.reserve(Phase::Snapshot, Dimension::Records, 1),
        Err(LedgerCause::DimensionNotReservable {
            dimension: Dimension::Records,
        })
    ));
}
