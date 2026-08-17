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

fn operation_with_producer_dimensions(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    other_policy: DimensionPolicy,
) -> (TestOperation, TestKeys) {
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
        TestOperation::try_start(control, ledger_plan, account, start)
            .expect("producer-host operation"),
        keys,
    )
}

fn requested_fallback_evidence(operation: TestOperation, cause: u32) -> StoreLedgerEvidence {
    operation
        .prepare_minimal_fallback(cause)
        .expect("producer-host operation remains terminally publishable")
        .handoff()
        .ledger_evidence
}

#[test]
fn explicit_partial_work_preserves_attempted_and_completed_prefix() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension state");

    {
        let mut host = operation.producer_host();
        let mut work = host
            .try_begin_observed_work([(keys.dimension, 4)])
            .expect("four work units are admitted");
        work.complete([3])
            .expect("three physically completed units are recorded");
        work.finish_partial();
    }

    let state = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("partial dimension state");
    assert_eq!(state.attempted(), baseline.attempted() + 4);
    assert_eq!(state.consumed(), baseline.consumed() + 3);
    assert_eq!(state.live_bytes(), baseline.live_bytes());

    let evidence = requested_fallback_evidence(operation, 71);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::Requested
    );
    let terminal = evidence
        .dimension(keys.dimension)
        .expect("terminal primary dimension");
    assert_eq!(terminal.attempted(), baseline.attempted() + 4);
    assert_eq!(terminal.consumed(), baseline.consumed() + 3);
}

#[test]
fn exact_work_completion_resolves_without_changing_the_requested_terminal_cause() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);

    {
        let mut host = operation.producer_host();
        let mut work = host
            .try_begin_observed_work([(keys.dimension, 2)])
            .expect("two work units are admitted");
        work.complete([2])
            .expect("the exact physical prefix is recorded");
        work.finish()
            .expect("the exact observed-work envelope resolves cleanly");
    }

    let evidence = requested_fallback_evidence(operation, 80);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::Requested
    );
}

#[test]
fn incomplete_exact_finish_latches_product_graph_and_preserves_the_prefix() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension state");

    {
        let mut host = operation.producer_host();
        let mut work = host
            .try_begin_observed_work([(keys.dimension, 2)])
            .expect("two work units are admitted");
        work.complete([1])
            .expect("the completed physical prefix is recorded");
        assert!(matches!(
            work.finish(),
            Err(StoreRuntimeError::ObservedWorkIncomplete)
        ));
    }

    let evidence = requested_fallback_evidence(operation, 81);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    let terminal = evidence
        .dimension(keys.dimension)
        .expect("terminal primary dimension");
    assert_eq!(terminal.attempted(), baseline.attempted() + 2);
    assert_eq!(terminal.consumed(), baseline.consumed() + 1);
}

#[test]
fn unresolved_drop_latches_product_graph_without_refunding_attempted_work() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension state");

    {
        let mut host = operation.producer_host();
        let mut work = host
            .try_begin_observed_work([(keys.dimension, 4)])
            .expect("four work units are admitted");
        work.complete([3])
            .expect("three physically completed units are recorded");
    }

    let evidence = requested_fallback_evidence(operation, 72);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    let terminal = evidence
        .dimension(keys.dimension)
        .expect("terminal primary dimension");
    assert_eq!(terminal.attempted(), baseline.attempted() + 4);
    assert_eq!(terminal.consumed(), baseline.consumed() + 3);
}

#[test]
fn grouped_completion_records_two_dimension_prefixes_together() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation_with_producer_dimensions(
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
        let mut work = host
            .try_begin_observed_work([(keys.dimension, 2), (keys.other_dimension, 3)])
            .expect("two-dimensional action is admitted");
        work.complete([1, 2])
            .expect("one physical action records both dimension prefixes");
        work.finish_partial();
    }

    let primary_after = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension state after completion");
    let secondary_after = operation
        .producer_host()
        .dimension_state(keys.other_dimension)
        .expect("secondary dimension state after completion");
    assert_eq!(primary_after.attempted(), primary.attempted() + 2);
    assert_eq!(primary_after.consumed(), primary.consumed() + 1);
    assert_eq!(secondary_after.attempted(), secondary.attempted() + 3);
    assert_eq!(secondary_after.consumed(), secondary.consumed() + 2);

    drop(requested_fallback_evidence(operation, 73));
}

#[test]
fn split_parser_accounting_preserves_attempted_and_consumed_prefixes() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation_with_producer_dimensions(
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
    let (mut operation, keys) = operation(&account);
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
fn group_overrun_is_side_effect_free_and_latches_product_graph() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation_with_producer_dimensions(
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
        let mut work = host
            .try_begin_observed_work([(keys.dimension, 2), (keys.other_dimension, 3)])
            .expect("two-dimensional action is admitted");
        assert!(matches!(
            work.complete([1, 4]),
            Err(StoreRuntimeError::ObservedWorkExceedsAdmitted)
        ));
        work.finish_partial();
    }

    let primary_after = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .expect("primary dimension state after rejection");
    let secondary_after = operation
        .producer_host()
        .dimension_state(keys.other_dimension)
        .expect("secondary dimension state after rejection");
    assert_eq!(primary_after.consumed(), primary.consumed());
    assert_eq!(secondary_after.consumed(), secondary.consumed());

    let evidence = requested_fallback_evidence(operation, 74);
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
}

#[test]
fn forgotten_observed_work_permit_blocks_terminal_preparation() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    {
        let mut host = operation.producer_host();
        let permit = host
            .try_begin_observed_work([(keys.dimension, 1)])
            .expect("one work unit is admitted");
        std::mem::forget(permit);
    }

    assert!(matches!(
        operation.prepare_minimal_fallback(75),
        Err(StorePrepareError::FinalizationContainment(_))
    ));
}

#[test]
fn progress_observer_cancellation_is_sticky_control_evidence() {
    let factory = factory(1);
    let account = account(&factory);
    let control = OperationControl::default().with_progress_observer(CancelProgress);
    let (mut operation, _keys) = operation_with_control(&account, control);

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
    let (mut operation, keys) = operation_with_producer_dimensions(
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
        assert!(matches!(
            host.try_begin_observed_work([(keys.other_dimension, 5)]),
            Err(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension,
                value: 5,
                limit: 4,
            })) if dimension == keys.other_dimension
        ));
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
    let (mut foreign_operation, _keys) = operation(&account_a);
    assert!(matches!(
        foreign_operation
            .producer_host()
            .try_begin_observed_work([(foreign_dimension(), 1)]),
        Err(StoreRuntimeError::ForeignDimensionDomain { .. })
    ));
    let foreign_evidence = requested_fallback_evidence(foreign_operation, 78);
    assert_eq!(
        store_failure_cause(&foreign_evidence).class(),
        StorePreparationFailureClass::Ledger
    );

    let (mut unplanned_operation, keys) = operation(&account_b);
    assert!(matches!(
        unplanned_operation
            .producer_host()
            .try_begin_observed_work([(keys.other_dimension, 1)]),
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
