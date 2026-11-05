use super::*;
use baozi_operation::{
    CancellationOrigin, ControlCause, CounterClass, ProgressDecision, ProgressEvent,
    ProgressObserver,
};

struct CancelProgress;

impl ProgressObserver<TestProgress, TestUnit> for CancelProgress {
    fn observe(&mut self, _event: ProgressEvent<TestProgress, TestUnit>) -> ProgressDecision {
        ProgressDecision::Cancel
    }
}

pub(super) fn operation_with_producer_dimensions<'inv, S: TestOperationStart<'inv>>(
    root: S,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    other_policy: DimensionPolicy,
) -> (S::Operation, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = unbounded_failure_terminal_work(keys);
    let primary_attempted_limit = failure_terminal_work
        .ordinary_attempted_work_limit()
        .checked_sub(other_policy.attempted_limit())
        .expect("the secondary producer dimension fits the ordinary attempted budget");
    let primary_consumed_limit = failure_terminal_work
        .ordinary_consumed_work_limit()
        .checked_sub(other_policy.consumed_limit())
        .expect("the secondary producer dimension fits the ordinary consumed budget");
    let primary_policy = DimensionPolicy::reservable_bytes(
        primary_attempted_limit,
        primary_consumed_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, primary_policy),
            (keys.other_dimension, other_policy),
            (
                keys.failure_terminal_dimension,
                failure_terminal_work.requirement().dimension_policy(),
            ),
        ],
    )
    .expect("producer-host ledger limits");
    let ledger_plan =
        StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).expect("producer-host ledger plan");
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, TEST_TERMINAL_ATTRIBUTION_SLOTS, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        TEST_FALLBACK_OUTCOME_EVIDENCE,
        keys.dimension,
        TEST_SNAPSHOT_OUTCOME_EVIDENCE,
        failure_terminal_work,
    );
    (
        root.start(control, ledger_plan, account, start)
            .expect("producer-host operation"),
        keys,
    )
}

fn requested_fallback_evidence<'inv>(
    operation: TestOperation<'inv>,
    cause: u32,
) -> StoreLedgerEvidence {
    operation
        .prepare_minimal_fallback(cause)
        .expect("producer-host operation remains terminally publishable")
        .handoff()
        .ledger_evidence
}

#[test]
fn split_parser_accounting_preserves_attempted_and_consumed_prefixes() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_producer_dimensions(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    let primary = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension state");
    let secondary = operation
        .producer_host()
        .dimension_state(keys.other_dimension)
        .expect("secondary dimension state");

    {
        let mut host = operation.producer_host();
        host.charge_attempted(keys.dimension, 4)
            .expect("the parser admits the primary envelope before work");
        host.charge_attempted(keys.other_dimension, 2)
            .expect("the parser admits the secondary envelope before work");
        host.charge_consumed(keys.dimension, 3)
            .expect("one completed primary prefix is retained");
        host.charge_consumed_batch([(keys.dimension, 1), (keys.other_dimension, 2)])
            .expect("the final grouped prefix is committed atomically");
    }

    let primary_after = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension state after split accounting");
    let secondary_after = operation
        .producer_host()
        .dimension_state(keys.other_dimension)
        .expect("secondary dimension state after split accounting");
    assert_eq!(primary_after.attempted(), primary.attempted() + 4);
    assert_eq!(primary_after.consumed(), primary.consumed() + 4);
    assert_eq!(secondary_after.attempted(), secondary.attempted() + 2);
    assert_eq!(secondary_after.consumed(), secondary.consumed() + 2);

    drop(requested_fallback_evidence(operation, 82));
}

#[test]
fn split_parser_batches_validate_every_dimension_before_charging() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation(&mut invocation_root, &account);
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension state");

    assert!(matches!(
        operation
            .producer_host()
            .charge_observed_batch([(keys.dimension, 1), (foreign_dimension(), 1),]),
        Err(StoreRuntimeError::ForeignDimensionDomain { .. })
    ));
    let after_foreign = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension remains observable");
    assert_eq!(after_foreign.attempted(), baseline.attempted());
    assert_eq!(after_foreign.consumed(), baseline.consumed());

    assert!(matches!(
        operation
            .producer_host()
            .charge_attempted(keys.failure_terminal_dimension, 1),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
            dimension,
        })) if dimension == keys.failure_terminal_dimension
    ));

    let evidence = requested_fallback_evidence(operation, 83);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::Ledger
    );
}

#[test]
fn direct_consumed_batches_reject_late_invalid_keys_without_changing_the_prefix() {
    for invalid_kind in 0..3 {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation(&mut invocation_root, &account);
        let invalid = match invalid_kind {
            0 => foreign_dimension(),
            1 => keys.other_dimension,
            _ => keys.failure_terminal_dimension,
        };
        let baseline = {
            let mut host = operation.producer_host();
            host.charge_attempted(keys.dimension, 3).unwrap();
            host.charge_consumed(keys.dimension, 1).unwrap();
            host.dimension_state(keys.dimension).unwrap()
        };
        let rejected = operation
            .producer_host()
            .charge_consumed_batch([(keys.dimension, 1), (invalid, 0)]);
        if invalid_kind == 0 {
            assert_eq!(
                rejected,
                Err(StoreRuntimeError::ForeignDimensionDomain { dimension: invalid })
            );
            assert_eq!(
                operation.producer_host().charge_consumed_batch([
                    (keys.failure_terminal_dimension, 0),
                    (keys.other_dimension, 0),
                    (invalid, 0),
                ]),
                Err(StoreRuntimeError::ForeignDimensionDomain { dimension: invalid }),
                "complete domain validation precedes every raw membership check"
            );
        } else {
            assert_eq!(
                rejected,
                Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
                    dimension: invalid,
                }))
            );
        }
        {
            let mut host = operation.producer_host();
            assert_eq!(host.dimension_state(keys.dimension).unwrap(), baseline);
            host.charge_consumed_batch([])
                .expect("empty direct accounting preserves sticky evidence");
            assert_eq!(host.dimension_state(keys.dimension).unwrap(), baseline);
        }
        let evidence = requested_fallback_evidence(operation, 84);
        assert_eq!(
            store_failure_cause(&evidence).class(),
            StorePreparationFailureClass::Ledger
        );
        let terminal = evidence.dimension(keys.dimension).unwrap();
        assert_eq!(terminal.attempted(), baseline.attempted());
        assert_eq!(terminal.consumed(), baseline.consumed());
    }
}

#[test]
fn direct_accounting_preserves_domain_then_membership_precedence() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation(&mut invocation_root, &account);
    {
        let mut host = operation.producer_host();
        host.charge_attempted(keys.dimension, 3).unwrap();
        host.charge_consumed(keys.dimension, 1).unwrap();
    }
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    let foreign = foreign_dimension();
    {
        let mut host = operation.producer_host();
        assert_eq!(
            host.charge_consumed_batch([(keys.other_dimension, 0), (foreign, 0)]),
            Err(StoreRuntimeError::ForeignDimensionDomain { dimension: foreign })
        );
        assert_eq!(
            host.charge_attempted(foreign, 0),
            Err(StoreRuntimeError::ForeignDimensionDomain { dimension: foreign })
        );
        for dimension in [keys.other_dimension, keys.failure_terminal_dimension] {
            assert_eq!(
                host.charge_attempted(dimension, 0),
                Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
                    dimension,
                }))
            );
            assert_eq!(
                host.charge_consumed_batch([(dimension, 0)]),
                Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
                    dimension,
                }))
            );
        }
        host.charge_attempted(keys.dimension, 0).unwrap();
        host.charge_consumed_batch([(keys.dimension, 0)]).unwrap();
        host.charge_consumed_batch([]).unwrap();
        assert_eq!(host.dimension_state(keys.dimension).unwrap(), baseline);
    }
    let evidence = requested_fallback_evidence(operation, 85);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::Ledger
    );
    let terminal = evidence.dimension(keys.dimension).unwrap();
    assert_eq!(terminal.attempted(), baseline.attempted());
    assert_eq!(terminal.consumed(), baseline.consumed());
}

#[test]
fn direct_accounting_preserves_duplicate_completed_prefixes() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_producer_dimensions(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    let primary = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    let secondary = operation
        .producer_host()
        .dimension_state(keys.other_dimension)
        .unwrap();
    {
        let mut host = operation.producer_host();
        host.charge_attempted(keys.dimension, 3).unwrap();
        host.charge_attempted(keys.other_dimension, 2).unwrap();
        host.charge_consumed_batch([(keys.dimension, 1), (keys.other_dimension, 1)])
            .unwrap();
        host.charge_attempted(keys.dimension, 1).unwrap();
        host.charge_attempted(keys.other_dimension, 1).unwrap();
        host.charge_consumed_batch([
            (keys.dimension, 1),
            (keys.dimension, 1),
            (keys.other_dimension, 1),
        ])
        .expect("duplicate dimensions retain their complete direct-accounting prefix");
    }
    let primary_after = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    let secondary_after = operation
        .producer_host()
        .dimension_state(keys.other_dimension)
        .unwrap();
    assert_eq!(primary_after.attempted(), primary.attempted() + 4);
    assert_eq!(primary_after.consumed(), primary.consumed() + 3);
    assert_eq!(primary_after.live_bytes(), primary.live_bytes());
    assert_eq!(secondary_after.attempted(), secondary.attempted() + 3);
    assert_eq!(secondary_after.consumed(), secondary.consumed() + 2);
    assert_eq!(secondary_after.live_bytes(), secondary.live_bytes());
    let evidence = requested_fallback_evidence(operation, 86);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::Requested
    );
    let primary_terminal = evidence.dimension(keys.dimension).unwrap();
    let secondary_terminal = evidence.dimension(keys.other_dimension).unwrap();
    assert_eq!(primary_terminal.attempted(), primary_after.attempted());
    assert_eq!(primary_terminal.consumed(), primary_after.consumed());
    assert_eq!(secondary_terminal.attempted(), secondary_after.attempted());
    assert_eq!(secondary_terminal.consumed(), secondary_after.consumed());
}

#[test]
fn progress_observer_cancellation_is_sticky_control_evidence() {
    let factory = factory(1);
    let account = account(&factory);
    let control = OperationControl::default().with_progress_observer(CancelProgress);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, _keys) = operation_with_control(&mut invocation_root, &account, control);

    assert!(matches!(
        operation
            .producer_host()
            .report_progress(TestProgress::Build, TestUnit::Items, 1, Some(2),),
        Err(StoreRuntimeError::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Observer,
        }))
    ));

    let evidence = requested_fallback_evidence(operation, 76);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::Control
    );
}

#[test]
fn producer_policy_can_only_tighten_and_remains_domain_qualified() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_producer_dimensions(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(8, 8, 0),
    );

    {
        let mut host = operation.producer_host();
        host.tighten_dimension_policy(keys.other_dimension, DimensionPolicy::counter(4, 4, 0))
            .expect("policy tightens before work");
        host.tighten_dimension_policy(keys.other_dimension, DimensionPolicy::counter(8, 8, 0))
            .expect("a relaxation request cannot widen the frozen policy");
        host.poll()
            .expect("control permits the direct producer action");
        assert!(matches!(
            host.charge_attempted(keys.other_dimension, 5),
            Err(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension,
                value: 5,
                limit: 4,
            })) if dimension == keys.other_dimension
        ));
        let state = host.dimension_state(keys.other_dimension).unwrap();
        assert_eq!(state.attempted(), 5);
        assert_eq!(state.consumed(), 0);
    }

    let evidence = requested_fallback_evidence(operation, 77);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::Ledger
    );
}

#[test]
fn producer_host_rejects_foreign_and_unplanned_dimensions_before_work() {
    let factory = factory(2);
    let account_a = account(&factory);
    let account_b = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut foreign_operation, _keys) = operation(&mut invocation_root, &account_a);
    assert!(matches!(
        foreign_operation
            .producer_host()
            .charge_attempted(foreign_dimension(), 1),
        Err(StoreRuntimeError::ForeignDimensionDomain { .. })
    ));
    let foreign_evidence = requested_fallback_evidence(foreign_operation, 78);
    assert_eq!(
        store_failure_cause(&foreign_evidence).class(),
        StorePreparationFailureClass::Ledger
    );

    let mut invocation_root_2 = TestInvocationRoot::new();
    let (mut unplanned_operation, keys) = operation(&mut invocation_root_2, &account_b);
    assert!(matches!(
        unplanned_operation
            .producer_host()
            .charge_attempted(keys.other_dimension, 1),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
            dimension,
        })) if dimension == keys.other_dimension
    ));
    let unplanned_evidence = requested_fallback_evidence(unplanned_operation, 79);
    assert_eq!(
        store_failure_cause(&unplanned_evidence).class(),
        StorePreparationFailureClass::Ledger
    );
}
