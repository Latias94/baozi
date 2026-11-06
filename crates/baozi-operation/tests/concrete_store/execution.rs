#![allow(
    clippy::result_large_err,
    reason = "tests retain the exact inline operation failure vocabulary"
)]

use std::sync::atomic::{AtomicUsize, Ordering};

use baozi_operation::__retained_store_integration::activation::{
    StoreActionExecution, StoreInvocationError, StoreScanReturn,
};
use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;
use baozi_operation::{
    CancellationOrigin, CancellationSignal, CapacityPlan, ControlCause, CounterClass,
    ProgressDecision, ProgressEvent,
};

use super::*;

#[derive(Clone, Default)]
struct CountedCancellation {
    token: CancellationToken,
    polls: Arc<AtomicUsize>,
}

impl CancellationSignal for CountedCancellation {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::SeqCst);
        self.token.is_cancelled()
    }
}

fn operation_with_work_policy<'inv>(
    root: &'inv mut TestInvocationRoot,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    work_policy: DimensionPolicy,
) -> (TestOperation<'inv>, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let unconsumable_work = work_policy.attempted_limit() - work_policy.consumed_limit();
    let failure_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        keys.failure_terminal_dimension,
        TEST_MAX_FAILURE_EVIDENCE_MEMBERS,
        TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT,
        TEST_FAILURE_EVIDENCE_VALIDATION_WORK_LIMIT,
        u64::MAX,
        u64::MAX - unconsumable_work,
    )
    .unwrap();
    let storage_work_limit = failure_work
        .ordinary_attempted_work_limit()
        .checked_sub(work_policy.attempted_limit())
        .unwrap();
    assert_eq!(
        storage_work_limit,
        failure_work.ordinary_consumed_work_limit() - work_policy.consumed_limit()
    );
    let storage_policy = DimensionPolicy::reservable_bytes(
        storage_work_limit,
        storage_work_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, storage_policy),
            (keys.other_dimension, work_policy),
            (
                keys.failure_terminal_dimension,
                failure_work.requirement().dimension_policy(),
            ),
        ],
    )
    .unwrap();
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, TEST_TERMINAL_ATTRIBUTION_SLOTS, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        TEST_FALLBACK_OUTCOME_EVIDENCE,
        keys.dimension,
        TEST_SNAPSHOT_OUTCOME_EVIDENCE,
        failure_work,
    );
    (
        TestOperation::try_start(root, control, ledger_plan, account, start).unwrap(),
        keys,
    )
}

fn write_unit(
    execution: &mut StoreActionExecution<'_, TestOwner, TestProgress, TestUnit>,
    destination: &mut u8,
) -> Result<(), StoreRuntimeError> {
    execution.begin_action(1)?;
    *destination = 1;
    execution.complete_action()
}

fn rejected_success_evidence<'inv>(
    operation: TestOperation<'inv>,
    expected: StorePreparationFailureClass,
) -> StoreLedgerEvidence {
    let prepared = operation
        .prepare_evidence_only_success(final_progress())
        .expect("a resolved execution failure still owns its terminal outcome");
    let evidence = match prepared {
        PreparedStoreEvidenceOnlyOutcome::MinimalFallback(fallback) => {
            fallback.handoff().ledger_evidence
        }
        PreparedStoreEvidenceOnlyOutcome::Success(_) => {
            panic!("an ignored execution failure must prevent operation success")
        }
    };
    assert_eq!(store_failure_cause(&evidence).class(), expected);
    evidence
}

fn assert_work(
    evidence: &StoreLedgerEvidence,
    dimension: StoreDimensionKey,
    attempted: u64,
    consumed: u64,
) {
    let state = evidence.dimension(dimension).unwrap();
    assert_eq!((state.attempted(), state.consumed()), (attempted, consumed));
}

#[test]
fn borrowed_storage_observation_reads_logical_capacity_before_operation_detach() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation(&mut invocation_root, &account);
    let physical = account.snapshot();
    operation
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let baseline = context.dimension_state(keys.dimension)?.live_bytes();
            let observation = context.storage_observation(keys.dimension)?;
            let mut first =
                context
                    .services()
                    .ordinary_storage()
                    .reserve(keys.phase, keys.dimension, 17)?;
            let cloned = observation;
            assert_eq!(observation.live_bytes(context), baseline + 17);
            context.services().resize(&mut first, 9)?;
            assert_eq!(cloned.live_bytes(context), baseline + 9);
            let second = context
                .ordinary_storage()
                .reserve(keys.phase, keys.dimension, 31)?;
            assert_eq!(observation.live_bytes(context), baseline + 40);
            assert_eq!(account.snapshot(), physical);
            drop(first);
            assert_eq!(cloned.live_bytes(context), baseline + 31);
            drop(second);
            assert_eq!(observation.live_bytes(context), baseline);
            Ok(())
        })
        .unwrap();
    let prepared = operation
        .prepare_evidence_only_success(final_progress())
        .unwrap();
    assert!(matches!(
        prepared,
        PreparedStoreEvidenceOnlyOutcome::Success(_)
    ));
    drop(prepared);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn group_and_partial_permit_survive_real_storage_and_progress() {
    let factory = factory(1);
    let account = Arc::new(account(&factory));
    let callback_account = Arc::clone(&account);
    let callbacks = Arc::new(AtomicUsize::new(0));
    let callback_count = Arc::clone(&callbacks);
    let control = OperationControl::default().with_progress_observer(
        move |event: ProgressEvent<TestProgress, TestUnit>| {
            assert_eq!(event.completed(), 1);
            assert!(!callback_account.snapshot().poisoned());
            let mut callback_storage = Vec::<u8>::new();
            callback_storage.try_reserve_exact(3).unwrap();
            callback_storage.extend_from_slice(&[4, 5, 6]);
            assert_eq!(callback_storage, [4, 5, 6]);
            callback_count.fetch_add(1, Ordering::SeqCst);
            ProgressDecision::Continue
        },
    );
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        control,
        DimensionPolicy::counter(16, 16, 0),
    );
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    let physical_before = account.snapshot();
    let mut output = [0; 3];

    let (retained, retained_bytes) = operation
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([keys.other_dimension])?;
            let retained = context.parser_scan_child::<_, StoreInvocationError, 1>(
                layout,
                [3],
                [None],
                |child| {
                    child.run_prefix([1], 1, || {
                        output[0] = 1;
                        Ok::<_, StoreInvocationError>(())
                    })?;
                    child.services().report_progress(
                        TestProgress::Build,
                        TestUnit::Items,
                        1,
                        None,
                    )?;
                    assert_eq!(callbacks.load(Ordering::SeqCst), 1);

                    let temporary_plan = CapacityPlan::for_type::<u8>(0, 0, 17).unwrap();
                    let mut temporary = child
                        .services()
                        .ordinary_storage()
                        .reserve(
                            keys.phase,
                            keys.dimension,
                            temporary_plan.envelope_delta_bytes().unwrap(),
                        )
                        .unwrap();
                    let mut scratch = Vec::<u8>::new();
                    scratch
                        .try_reserve_exact(temporary_plan.allocator_additional())
                        .unwrap();
                    let scratch_bytes = temporary_plan
                        .finish(scratch.len(), scratch.capacity())
                        .unwrap();
                    child.services().resize(&mut temporary, scratch_bytes)?;
                    scratch.resize(temporary_plan.desired_capacity(), 7);
                    assert_eq!(scratch, [7; 17]);
                    drop(scratch);
                    drop(temporary);

                    let retained_plan = CapacityPlan::for_type::<u8>(0, 0, 31).unwrap();
                    let mut reservation = child
                        .services()
                        .reserve_retained(
                            keys.phase,
                            keys.dimension,
                            retained_plan.envelope_delta_bytes().unwrap(),
                            1,
                        )
                        .expect("the internal account share can nest inside a parser channel");
                    let mut value = Vec::<u8>::new();
                    value
                        .try_reserve_exact(retained_plan.allocator_additional())
                        .unwrap();
                    let retained_bytes =
                        retained_plan.finish(value.len(), value.capacity()).unwrap();
                    child.services().resize(&mut reservation, retained_bytes)?;
                    value.resize(retained_plan.desired_capacity(), 9);
                    let retained = reservation.bind_storage(value).unwrap();

                    assert_eq!(child.remaining(), [2]);
                    child.run_prefix([1], 1, || {
                        output[1] = 2;
                        Ok::<_, StoreInvocationError>(())
                    })?;
                    Ok(StoreScanReturn::Boundary((retained, retained_bytes)))
                },
            )?;
            context.run_action(layout, [1], 1, || output[2] = 1)?;
            Ok(retained)
        })
        .unwrap();

    assert_eq!(output, [1, 2, 1]);
    assert_eq!(retained.as_ref(), &[9; 31]);
    assert_eq!(
        physical_delta(physical_before, account.snapshot()),
        (retained_bytes, 1)
    );
    let work = operation
        .producer_host()
        .dimension_state(keys.other_dimension)
        .unwrap();
    assert_eq!((work.attempted(), work.consumed()), (4, 3));
    let storage = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    assert_eq!(storage.attempted(), baseline.attempted() + 2);
    assert_eq!(storage.consumed(), baseline.consumed() + 2);
    assert_eq!(storage.live_bytes(), baseline.live_bytes() + retained_bytes);

    drop(retained);
    let storage_after_drop = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    assert_eq!(storage_after_drop.live_bytes(), baseline.live_bytes());
    assert_eq!(storage_after_drop.attempted(), baseline.attempted() + 2);
    assert_eq!(storage_after_drop.consumed(), baseline.consumed() + 2);
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        physical_before.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        physical_before.live_retained_allocations()
    );

    let published = operation.prepare_minimal_fallback(71).unwrap().handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    assert_work(&published.ledger_evidence, keys.other_dimension, 4, 3);
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(callbacks.load(Ordering::SeqCst), 1);
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn idle_sequence_keeps_retained_share_work_and_storage_on_the_same_authority() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    let physical = account.snapshot();
    let mut output = [0; 2];
    operation
        .with_action_execution::<_, StoreRuntimeError>(keys.dimension, |execution| {
            execution.begin_action(1)?;
            output[0] = 1;
            execution.complete_action()?;

            let plan = CapacityPlan::for_type::<u8>(0, 0, 5).unwrap();
            let mut reservation = execution
                .reserve_retained(
                    keys.phase,
                    keys.dimension,
                    plan.envelope_delta_bytes().unwrap(),
                    1,
                )
                .unwrap();
            let mut values = Vec::<u8>::new();
            values
                .try_reserve_exact(plan.allocator_additional())
                .unwrap();
            let bytes = plan.finish(values.len(), values.capacity()).unwrap();
            execution.resize(&mut reservation, bytes)?;
            values.extend_from_slice(&[2, 3, 4, 5, 6]);
            let retained = reservation.bind_storage(values).unwrap();
            assert_eq!(retained.as_ref(), &[2, 3, 4, 5, 6]);
            assert_eq!(physical_delta(physical, account.snapshot()), (bytes, 1));
            execution.begin_action(1)?;
            output[1] = retained.as_ref()[0];
            execution.complete_action()?;
            drop(retained);
            assert_eq!(
                execution.work_counters(keys.dimension)?,
                (baseline.attempted() + 4, baseline.consumed() + 4, 0)
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        operation
            .producer_host()
            .dimension_state(keys.dimension)
            .unwrap()
            .live_bytes(),
        baseline.live_bytes()
    );
    assert_eq!(output, [1, 2]);
    assert_eq!(physical_delta(physical, account.snapshot()), (0, 0));
    let published = operation.prepare_minimal_fallback(71).unwrap().handoff();
    assert_work(
        &published.ledger_evidence,
        keys.dimension,
        baseline.attempted() + 4,
        baseline.consumed() + 4,
    );
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn tightened_policy_rejects_refill_once_and_preserves_the_written_prefix() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(8, 8, 0),
    );
    let mut output = [73; 3];
    let result = operation.with_action_execution::<_, StoreRuntimeError>(
        keys.other_dimension,
        |execution| {
            for destination in output.iter_mut().take(2) {
                write_unit(execution, destination)?;
            }
            execution.tighten_dimension_policy(
                keys.other_dimension,
                DimensionPolicy::counter(2, 2, 0),
            )?;
            let denied = execution.begin_action(1);
            if denied.is_ok() {
                output[2] = 1;
            }
            assert!(matches!(
                denied,
                Err(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
                    dimension,
                    class: CounterClass::Attempted,
                    value: 3,
                    limit: 2,
                })) if dimension == keys.other_dimension
            ));
            assert!(matches!(
                execution.begin_action(1),
                Err(StoreRuntimeError::FallbackOnly)
            ));
            assert_eq!(execution.work_counters(keys.other_dimension)?, (3, 2, 0));
            denied
        },
    );
    assert!(result.is_err());
    assert_eq!(output, [1, 1, 73]);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
    assert_work(&evidence, keys.other_dimension, 3, 2);
}

#[test]
fn exact_work_is_independent_of_entry_periodic_and_final_poll_counts() {
    for (actions, expected_polls) in [(0, 2), (1, 2), (255, 2), (256, 2), (257, 3)] {
        let factory = factory(1);
        let account = account(&factory);
        let signal = CountedCancellation::default();
        let progress_events = Arc::new(AtomicUsize::new(0));
        let observed_events = Arc::clone(&progress_events);
        let control = OperationControl::default()
            .with_cancellation(signal.clone())
            .with_progress_observer(move |_: ProgressEvent<TestProgress, TestUnit>| {
                observed_events.fetch_add(1, Ordering::SeqCst);
                ProgressDecision::Continue
            });
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            control,
            DimensionPolicy::counter(512, 512, 0),
        );
        signal.polls.store(0, Ordering::SeqCst);
        let mut physical = [0; 257];
        operation
            .with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |execution| {
                assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
                for destination in physical.iter_mut().take(actions) {
                    write_unit(execution, destination)?;
                }
                assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls - 1);
                assert_eq!(progress_events.load(Ordering::SeqCst), 0);
                Ok(())
            })
            .unwrap();
        assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls);
        assert!(physical[..actions].iter().all(|value| *value == 1));
        assert!(physical[actions..].iter().all(|value| *value == 0));
        let state = operation
            .producer_host()
            .dimension_state(keys.other_dimension)
            .unwrap();
        assert_eq!(
            (state.attempted(), state.consumed()),
            (actions as u64, actions as u64)
        );
        let success = match operation
            .prepare_evidence_only_success(final_progress())
            .unwrap()
        {
            PreparedStoreEvidenceOnlyOutcome::Success(success) => success.handoff(),
            PreparedStoreEvidenceOnlyOutcome::MinimalFallback(_) => {
                panic!("a resolved execution region must remain eligible for success")
            }
        };
        assert_eq!(
            success.ledger_evidence.header().terminal_cause(),
            StoreTerminalCause::Success
        );
        assert_work(
            &success.ledger_evidence,
            keys.other_dimension,
            actions as u64,
            actions as u64,
        );
        assert_eq!(progress_events.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn cancellation_is_observed_at_entry_or_within_the_explicit_action_bound() {
    for (cancel_after, expected_actions, expected_polls) in [
        (0, 0, 1),
        (1, 256, 2),
        (255, 256, 2),
        (256, 256, 2),
        (257, 512, 3),
    ] {
        let factory = factory(1);
        let account = account(&factory);
        let signal = CountedCancellation::default();
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default().with_cancellation(signal.clone()),
            DimensionPolicy::counter(1024, 1024, 0),
        );
        signal.polls.store(0, Ordering::SeqCst);
        let mut physical = [0; 513];
        if cancel_after == 0 {
            signal.token.cancel();
        }
        let result = operation.with_action_execution::<(), StoreRuntimeError>(
            keys.other_dimension,
            |execution| {
                assert_ne!(cancel_after, 0);
                for (index, destination) in physical.iter_mut().enumerate() {
                    if let Err(cause) = execution.begin_action(1) {
                        assert!(matches!(
                            cause,
                            StoreRuntimeError::Control(ControlCause::Cancelled {
                                origin: CancellationOrigin::Signal,
                            })
                        ));
                        assert_eq!(index, expected_actions);
                        assert!(expected_actions - cancel_after <= 256);
                        assert!(matches!(
                            execution.begin_action(1),
                            Err(StoreRuntimeError::Control(_))
                        ));
                        assert!(matches!(
                            execution.checkpoint(),
                            Err(StoreRuntimeError::Control(_))
                        ));
                        return Err(cause);
                    }
                    *destination = 1;
                    execution.complete_action()?;
                    if index + 1 == cancel_after {
                        signal.token.cancel();
                    }
                }
                panic!("cancellation must be observed within the finite action bound")
            },
        );
        assert!(matches!(
            result,
            Err(StoreRuntimeError::Control(ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            }))
        ));
        assert!(matches!(
            operation
                .with_action_execution::<(), StoreRuntimeError>(keys.other_dimension, |_| panic!(
                    "cancelled regions cannot reenter"
                )),
            Err(StoreRuntimeError::Control(_))
        ));
        assert!(physical[..expected_actions].iter().all(|value| *value == 1));
        assert!(physical[expected_actions..].iter().all(|value| *value == 0));
        assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls);
        let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Control);
        assert_work(
            &evidence,
            keys.other_dimension,
            expected_actions as u64,
            expected_actions as u64,
        );
    }
}

#[test]
fn final_checkpoint_preserves_partial_envelope_prefixes_including_a_zero_action_region() {
    for (actions, attempted, expected_polls) in [
        (0, 1, 2),
        (1, 2, 2),
        (255, 256, 2),
        (256, 257, 2),
        (257, 258, 3),
    ] {
        let factory = factory(1);
        let account = account(&factory);
        let signal = CountedCancellation::default();
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default().with_cancellation(signal.clone()),
            DimensionPolicy::counter(512, 512, 0),
        );
        signal.polls.store(0, Ordering::SeqCst);
        let mut physical = [0; 257];
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([keys.other_dimension])?;
            context.parser_child::<_, StoreInvocationError, 1>(
                layout,
                [attempted],
                [None],
                |child| {
                    for destination in physical.iter_mut().take(actions) {
                        child.run_prefix([1], 1, || {
                            *destination = 1;
                            Ok::<_, StoreInvocationError>(())
                        })?;
                    }
                    signal.token.cancel();
                    assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls - 1);
                    let checkpoint = child.checkpoint();
                    assert!(matches!(
                        checkpoint,
                        Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
                            ControlCause::Cancelled {
                                origin: CancellationOrigin::Signal,
                            }
                        )))
                    ));
                    assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls);
                    assert_eq!(child.remaining(), [1]);
                    checkpoint
                },
            )
        });
        assert!(matches!(
            result,
            Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(_)))
        ));
        assert!(physical[..actions].iter().all(|value| *value == 1));
        assert!(physical[actions..].iter().all(|value| *value == 0));
        let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Control);
        assert_work(&evidence, keys.other_dimension, attempted, actions as u64);
    }
}

#[test]
fn explicit_checkpoint_resets_the_debt_shared_by_nested_control_reborrows() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default().with_cancellation(signal.clone()),
        DimensionPolicy::counter(1024, 1024, 0),
    );
    signal.polls.store(0, Ordering::SeqCst);
    let mut physical = [0; 512];
    operation
        .with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |execution| {
            for destination in &mut physical[..255] {
                write_unit(&mut *execution, destination)?;
            }
            assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
            {
                let nested = &mut *execution;
                nested.checkpoint()?;
            }
            assert_eq!(signal.polls.load(Ordering::SeqCst), 2);
            for destination in &mut physical[255..511] {
                write_unit(&mut *execution, destination)?;
            }
            assert_eq!(signal.polls.load(Ordering::SeqCst), 2);
            write_unit(execution, &mut physical[511])?;
            assert_eq!(signal.polls.load(Ordering::SeqCst), 3);
            Ok(())
        })
        .unwrap();
    assert_eq!(signal.polls.load(Ordering::SeqCst), 4);
    assert_eq!(physical, [1; 512]);
    let evidence = operation
        .prepare_minimal_fallback(72)
        .unwrap()
        .handoff()
        .ledger_evidence;
    assert_work(&evidence, keys.other_dimension, 512, 512);
}

#[test]
fn every_action_checks_attempted_and_consumed_budgets_between_control_polls() {
    for allowed in [0_usize, 1, 255] {
        for class in [CounterClass::Attempted, CounterClass::Consumed] {
            let factory = factory(1);
            let account = account(&factory);
            let signal = CountedCancellation::default();
            let policy = match class {
                CounterClass::Attempted => {
                    DimensionPolicy::counter(allowed as u64, allowed as u64, 0)
                }
                CounterClass::Consumed => DimensionPolicy::counter(512, allowed as u64, 0),
                _ => unreachable!(),
            };
            let mut invocation_root = TestInvocationRoot::new();
            let (mut operation, keys) = operation_with_work_policy(
                &mut invocation_root,
                &account,
                OperationControl::default().with_cancellation(signal.clone()),
                policy,
            );
            signal.polls.store(0, Ordering::SeqCst);
            let mut physical = [0; 256];
            let result = operation.with_action_execution::<_, StoreRuntimeError>(
                keys.other_dimension,
                |execution| {
                    for destination in physical.iter_mut().take(allowed) {
                        write_unit(execution, destination)?;
                    }
                    let rejected = execution.begin_action(1);
                    if rejected.is_ok() {
                        physical[allowed] = 1;
                    }
                    assert!(matches!(
                        rejected,
                        Err(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
                            class: rejected_class,
                            dimension,
                            value,
                            limit,
                        })) if rejected_class == class
                            && dimension == keys.other_dimension
                            && value == allowed as u64 + 1
                            && limit == allowed as u64
                    ));
                    assert!(matches!(
                        execution.begin_action(1),
                        Err(StoreRuntimeError::FallbackOnly)
                    ));
                    assert!(matches!(
                        execution.checkpoint(),
                        Err(StoreRuntimeError::FallbackOnly)
                    ));
                    assert_eq!(
                        execution.work_counters(keys.other_dimension)?,
                        (allowed as u64 + 1, allowed as u64, 0)
                    );
                    rejected
                },
            );
            assert!(result.is_err());
            assert!(physical[..allowed].iter().all(|value| *value == 1));
            assert!(physical[allowed..].iter().all(|value| *value == 0));
            assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
            let evidence =
                rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
            assert_work(
                &evidence,
                keys.other_dimension,
                allowed as u64 + 1,
                allowed as u64,
            );
        }
    }
}

#[test]
#[allow(
    forgetting_references,
    reason = "the real host retains ownership of forgotten client borrows"
)]
fn host_resolves_forgotten_fully_completed_and_zero_action_borrows() {
    for amount in [0, 3] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        operation
            .with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |execution| {
                let mut physical = [0; 3];
                for destination in physical.iter_mut().take(amount as usize) {
                    write_unit(execution, destination)?;
                }
                assert_eq!(u64::from(physical.iter().sum::<u8>()), amount);
                std::mem::forget(execution);
                Ok(())
            })
            .unwrap();
        operation
            .with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |_| Ok(()))
            .unwrap();
        let success = match operation
            .prepare_evidence_only_success(final_progress())
            .unwrap()
        {
            PreparedStoreEvidenceOnlyOutcome::Success(success) => success.handoff(),
            PreparedStoreEvidenceOnlyOutcome::MinimalFallback(_) => {
                panic!("host-resolved actions must allow success")
            }
        };
        assert_work(
            &success.ledger_evidence,
            keys.other_dimension,
            amount,
            amount,
        );
    }
}

#[test]
fn unwinding_zero_and_completed_action_hosts_resolves_work_and_prevents_success() {
    for amount in [0, 3] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<(), StoreRuntimeError> =
                operation.with_action_execution(keys.other_dimension, |execution| {
                    let mut physical = [0; 3];
                    for destination in physical.iter_mut().take(amount as usize) {
                        write_unit(execution, destination)?;
                    }
                    assert_eq!(u64::from(physical.iter().sum::<u8>()), amount);
                    panic!("unwind the actual host after its completed prefix")
                });
        }));
        assert!(panic.is_err());
        assert!(matches!(
            operation
                .with_action_execution::<(), StoreRuntimeError>(keys.other_dimension, |_| panic!(
                    "an unwound region cannot reenter"
                )),
            Err(StoreRuntimeError::FallbackOnly)
        ));
        let evidence =
            rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
        assert_work(&evidence, keys.other_dimension, amount, amount);
    }
}

#[test]
fn ignored_incomplete_finish_keeps_the_exact_prefix_and_sticky_failure() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    let mut physical = 0;
    let result = operation.with_action_execution::<_, StoreRuntimeError>(
        keys.other_dimension,
        |execution| {
            execution.begin_action(1)?;
            physical += 1;
            execution.complete_action()?;
            execution.begin_action(1)?;
            Ok(())
        },
    );
    assert!(matches!(
        result,
        Err(StoreRuntimeError::ObservedWorkIncomplete)
    ));
    assert!(matches!(
        operation.with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |_| {
            physical += 1;
            Ok(())
        }),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    assert_eq!(physical, 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
    assert_work(&evidence, keys.other_dimension, 2, 1);
}

#[test]
fn binding_rejects_foreign_unplanned_and_protected_keys_before_zero_work() {
    for invalid_role in 0..3 {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation(&mut invocation_root, &account);
        let baseline = operation
            .producer_host()
            .dimension_state(keys.dimension)
            .unwrap();
        let invalid_dimension = match invalid_role {
            0 => foreign_dimension(),
            1 => keys.other_dimension,
            2 => keys.failure_terminal_dimension,
            _ => unreachable!(),
        };
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            match context.bind_group([keys.dimension, invalid_dimension]) {
                Err(StoreInvocationError::Runtime(StoreRuntimeError::ForeignDimensionDomain {
                    dimension,
                })) => {
                    assert_eq!(invalid_role, 0);
                    assert_eq!(dimension, invalid_dimension);
                }
                Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                    LedgerCause::UnknownDimension { dimension },
                ))) => {
                    assert_ne!(invalid_role, 0);
                    assert_eq!(dimension, invalid_dimension);
                }
                _ => panic!("binding must validate every lane before any amount, including zero"),
            }
            assert!(matches!(
                context.bind_group([keys.dimension]),
                Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::FallbackOnly
                ))
            ));
            assert!(matches!(
                context.checkpoint(),
                Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::FallbackOnly
                ))
            ));
            Ok(())
        });
        assert!(matches!(
            result,
            Err(StoreInvocationError::Runtime(
                StoreRuntimeError::FallbackOnly
            ))
        ));
        let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
        assert_work(
            &evidence,
            keys.dimension,
            baseline.attempted(),
            baseline.consumed(),
        );
    }
}

#[test]
fn parser_parent_child_and_storage_share_one_owner_without_callback_borrows() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    let physical_before = account.snapshot();
    let mut decoded = [0; 3];
    operation
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([keys.other_dimension])?;
            context.parser_parent::<_, StoreInvocationError, 1>(layout, [2], [None], |parent| {
                parent.child::<_, StoreInvocationError, 1>(layout, [3], [None], |child| {
                    child.run_prefix([1], 1, || {
                        decoded[0] = 8;
                        Ok::<_, StoreInvocationError>(())
                    })?;
                    let retained = child.allocate_bytes(keys.phase, keys.dimension, 8)?;
                    child.report_progress(TestProgress::Build, TestUnit::Items, 1, Some(3))?;
                    child.run_prefix([2], 1, || {
                        decoded[1..].copy_from_slice(&[9, 10]);
                        Ok::<_, StoreInvocationError>(())
                    })?;
                    drop(retained);
                    Ok(())
                })?;
                parent.checkpoint()?;
                parent.complete_parent()
            })?;
            assert_eq!(context.work_counters(keys.other_dimension)?, (5, 5, 0));
            Ok(())
        })
        .unwrap();
    assert_eq!(decoded, [8, 9, 10]);
    assert_eq!(physical_delta(physical_before, account.snapshot()), (0, 0));
    let published = operation.prepare_minimal_fallback(71).unwrap().handoff();
    assert_work(&published.ledger_evidence, keys.other_dimension, 5, 5);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn ignored_parser_item_rejection_keeps_ordered_attempts_and_stops_success() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
        let layout = context.bind_group([keys.other_dimension; 3])?;
        assert!(matches!(
            context.parser_child::<(), StoreInvocationError, 3>(
                layout,
                [1, 3, 2],
                [None, Some(2), None],
                |_| panic!("a rejected parser admission cannot run its body"),
            ),
            Err(StoreInvocationError::ItemLimit {
                index: 1,
                value: 3,
                limit: 2
            })
        ));
        assert!(matches!(
            context.checkpoint(),
            Err(StoreInvocationError::Runtime(
                StoreRuntimeError::FallbackOnly
            ))
        ));
        Ok(())
    });
    assert!(matches!(
        result,
        Err(StoreInvocationError::Runtime(
            StoreRuntimeError::FallbackOnly
        ))
    ));
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
    assert_work(&evidence, keys.other_dimension, 4, 0);
}

#[test]
fn ignored_parser_completion_and_incomplete_finish_errors_remain_terminal() {
    for incomplete_finish in [false, true] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        let mut decoded = [0; 4];
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([keys.other_dimension])?;
            let rejected = context.parser_child::<_, StoreInvocationError, 1>(
                layout,
                [4],
                [None],
                |channel| {
                    channel.run_prefix([2], 1, || {
                        decoded[..2].copy_from_slice(&[7, 8]);
                        Ok::<_, StoreInvocationError>(())
                    })?;
                    assert_eq!(channel.remaining(), [2]);
                    if incomplete_finish {
                        Ok(())
                    } else {
                        let rejected = channel.run_prefix([3], 1, || {
                            decoded[2] = 1;
                            Ok::<_, StoreInvocationError>(())
                        });
                        assert_eq!(channel.remaining(), [2]);
                        rejected
                    }
                },
            );
            if incomplete_finish {
                assert!(matches!(
                    rejected,
                    Err(StoreInvocationError::Runtime(
                        StoreRuntimeError::ObservedWorkIncomplete
                    ))
                ));
            } else {
                assert!(matches!(
                    rejected,
                    Err(StoreInvocationError::Runtime(
                        StoreRuntimeError::ObservedWorkExceedsAdmitted
                    ))
                ));
            }
            assert!(matches!(
                context.run_action(layout, [0], 1, || decoded[3] = 1),
                Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::FallbackOnly
                ))
            ));
            assert!(matches!(
                context.checkpoint(),
                Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::FallbackOnly
                ))
            ));
            Ok(())
        });
        assert!(matches!(
            result,
            Err(StoreInvocationError::Runtime(
                StoreRuntimeError::FallbackOnly
            ))
        ));
        assert_eq!(decoded, [7, 8, 0, 0]);
        let class = if incomplete_finish {
            StorePreparationFailureClass::ProductGraph
        } else {
            StorePreparationFailureClass::Ledger
        };
        let evidence = rejected_success_evidence(operation, class);
        assert_work(&evidence, keys.other_dimension, 4, 2);
    }
}

#[test]
#[allow(
    forgetting_references,
    reason = "forgetting a client borrow must leave the actual frame with its host"
)]
fn forgotten_zero_and_completed_parser_borrows_leave_host_channels_resolved() {
    for amount in [0, 1] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([keys.other_dimension])?;
                context.parser_child::<_, StoreInvocationError, 1>(
                    layout,
                    [amount],
                    [None],
                    |channel| {
                        channel.run_prefix([amount], 1, || Ok::<_, StoreInvocationError>(()))?;
                        assert_eq!(channel.remaining(), [0]);
                        std::mem::forget(channel);
                        Ok(())
                    },
                )?;
                assert_eq!(
                    context.work_counters(keys.other_dimension)?,
                    (amount, amount, 0)
                );
                context.parser_child::<_, StoreInvocationError, 1>(layout, [0], [None], |_| Ok(()))
            })
            .unwrap();
        let success = match operation
            .prepare_evidence_only_success(final_progress())
            .unwrap()
        {
            PreparedStoreEvidenceOnlyOutcome::Success(success) => success.handoff(),
            PreparedStoreEvidenceOnlyOutcome::MinimalFallback(_) => {
                panic!("a host-resolved parser remains eligible for success")
            }
        };
        assert_work(
            &success.ledger_evidence,
            keys.other_dimension,
            amount,
            amount,
        );
    }
}

#[test]
fn bound_storage_rejects_foreign_keys_before_polling_or_admitting_bytes() {
    for retained in [false, true] {
        for invalid_phase in [false, true] {
            let factory = factory(1);
            let account = account(&factory);
            let signal = CountedCancellation::default();
            let mut invocation_root = TestInvocationRoot::new();
            let (mut operation, keys) = operation_with_work_policy(
                &mut invocation_root,
                &account,
                OperationControl::default().with_cancellation(signal.clone()),
                DimensionPolicy::counter(16, 16, 0),
            );
            let baseline = operation
                .producer_host()
                .dimension_state(keys.dimension)
                .unwrap();
            let before = account.snapshot();
            let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let polls = signal.polls.load(Ordering::SeqCst);
                let phase = if invalid_phase {
                    foreign_phase()
                } else {
                    keys.phase
                };
                let dimension = foreign_dimension();
                let result = if retained {
                    context.services().reserve_retained(phase, dimension, 8, 1)
                } else {
                    context
                        .ordinary_storage()
                        .reserve(phase, dimension, 8)
                        .map_err(StoreInvocationError::from)
                };
                match result {
                    Err(StoreInvocationError::Runtime(StoreRuntimeError::ForeignPhaseDomain {
                        phase: rejected,
                    })) => {
                        assert!(invalid_phase);
                        assert_eq!(rejected, phase);
                    }
                    Err(StoreInvocationError::Runtime(
                        StoreRuntimeError::ForeignDimensionDomain {
                            dimension: rejected,
                        },
                    )) => {
                        assert!(!invalid_phase);
                        assert_eq!(rejected, dimension);
                    }
                    _ => panic!("foreign keys must reject in phase/dimension order"),
                }
                assert_eq!(signal.polls.load(Ordering::SeqCst), polls);
                assert!(matches!(
                    context.services().checkpoint(),
                    Err(StoreInvocationError::Runtime(
                        StoreRuntimeError::FallbackOnly
                    ))
                ));
                assert_eq!(signal.polls.load(Ordering::SeqCst), polls);
                Ok(())
            });
            assert!(matches!(
                result,
                Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::FallbackOnly
                ))
            ));
            assert_eq!(physical_delta(before, account.snapshot()), (0, 0));
            let evidence =
                rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
            assert_work(
                &evidence,
                keys.dimension,
                baseline.attempted(),
                baseline.consumed(),
            );
            assert!(!account.snapshot().poisoned());
        }
    }
}

#[test]
fn bound_services_reject_foreign_live_tokens_without_changing_the_source_capacity() {
    for retained in [false, true] {
        for action in 0..4 {
            let factory = factory(1);
            let account = account(&factory);
            let mut invocation_root = TestInvocationRoot::new();
            let (mut source, source_keys) = operation_with_work_policy(
                &mut invocation_root,
                &account,
                OperationControl::default(),
                DimensionPolicy::counter(16, 16, 0),
            );
            let signal = CountedCancellation::default();
            let mut invocation_root_2 = TestInvocationRoot::new();
            let (mut receiver, receiver_keys) = operation_with_work_policy(
                &mut invocation_root_2,
                &account,
                OperationControl::default().with_cancellation(signal.clone()),
                DimensionPolicy::counter(16, 16, 0),
            );
            let before = account.snapshot();
            source
                .with_exclusive_execution::<_, StoreInvocationError>(|source_context| {
                    let observation = source_context.storage_observation(source_keys.dimension)?;
                    let baseline = observation.live_bytes(source_context);
                    let mut reservation = if retained {
                        source_context
                            .services()
                            .reserve_retained(source_keys.phase, source_keys.dimension, 8, 1)
                            .unwrap()
                    } else {
                        source_context
                            .ordinary_storage()
                            .reserve(source_keys.phase, source_keys.dimension, 8)
                            .unwrap()
                    };
                    let expected_physical = if retained { (8, 1) } else { (0, 0) };
                    assert_eq!(observation.live_bytes(source_context), baseline + 8);
                    assert_eq!(
                        physical_delta(before, account.snapshot()),
                        expected_physical
                    );
                    let rejected = receiver.with_exclusive_execution::<_, StoreInvocationError>(
                        |receiver_context| {
                            let polls = signal.polls.load(Ordering::SeqCst);
                            let result = match action {
                                0 => receiver_context
                                    .services()
                                    .validate_live_owner(&reservation),
                                1 => receiver_context
                                    .ordinary_storage()
                                    .resize(&mut reservation, 4)
                                    .map_err(StoreInvocationError::from),
                                2 => receiver_context.services().resize(&mut reservation, 4),
                                3 => receiver_context.services().transition(
                                    &mut reservation,
                                    receiver_keys.phase,
                                    receiver_keys.dimension,
                                    4,
                                ),
                                _ => unreachable!(),
                            };
                            assert!(matches!(
                                result,
                                Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                                    LedgerCause::OwnerMismatch
                                )))
                            ));
                            assert_eq!(signal.polls.load(Ordering::SeqCst), polls);
                            assert_eq!(observation.live_bytes(source_context), baseline + 8);
                            assert_eq!(
                                physical_delta(before, account.snapshot()),
                                expected_physical
                            );
                            assert!(matches!(
                                receiver_context.services().checkpoint(),
                                Err(StoreInvocationError::Runtime(
                                    StoreRuntimeError::FallbackOnly
                                ))
                            ));
                            assert_eq!(signal.polls.load(Ordering::SeqCst), polls);
                            Ok(())
                        },
                    );
                    assert!(matches!(
                        rejected,
                        Err(StoreInvocationError::Runtime(
                            StoreRuntimeError::FallbackOnly
                        ))
                    ));
                    source_context.services().checkpoint()?;
                    source_context
                        .services()
                        .validate_live_owner(&reservation)?;
                    source_context.services().resize(&mut reservation, 4)?;
                    assert_eq!(observation.live_bytes(source_context), baseline + 4);
                    assert_eq!(
                        physical_delta(before, account.snapshot()),
                        if retained { (4, 1) } else { (0, 0) }
                    );
                    reservation.release_storage(());
                    assert_eq!(observation.live_bytes(source_context), baseline);
                    assert_eq!(physical_delta(before, account.snapshot()), (0, 0));
                    Ok(())
                })
                .unwrap();
            assert_eq!(physical_delta(before, account.snapshot()), (0, 0));
            let _ = rejected_success_evidence(receiver, StorePreparationFailureClass::Ledger);
            let published = source.prepare_minimal_fallback(71).unwrap().handoff();
            assert_eq!(
                store_failure_cause(&published.ledger_evidence).class(),
                StorePreparationFailureClass::Requested
            );
            assert_eq!(store_failure_cause(&published.ledger_evidence).code(), 71);
            assert!(!account.snapshot().poisoned());
        }
    }
}

#[test]
fn ordinary_loans_end_before_progress_and_keep_the_same_poll_frontiers() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let callbacks = Arc::new(AtomicUsize::new(0));
    let callback_count = Arc::clone(&callbacks);
    let control = OperationControl::default()
        .with_cancellation(signal.clone())
        .with_progress_observer(move |event: ProgressEvent<TestProgress, TestUnit>| {
            let index = callback_count.fetch_add(1, Ordering::SeqCst);
            assert_eq!(event.completed(), (index + 1) as u64);
            ProgressDecision::Continue
        });
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        control,
        DimensionPolicy::counter(16, 16, 0),
    );
    let physical = account.snapshot();
    operation
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let observation = context.storage_observation(keys.dimension)?;
            let baseline = observation.live_bytes(context);
            let polls = signal.polls.load(Ordering::SeqCst);
            let first = {
                let mut services = context.services();
                let mut loan = services.ordinary_storage();
                assert_eq!(signal.polls.load(Ordering::SeqCst), polls);
                let first = {
                    let mut short = loan.reborrow();
                    let mut reservation = short.reserve(keys.phase, keys.dimension, 8).unwrap();
                    short.resize(&mut reservation, 4).unwrap();
                    reservation
                };
                loan.checkpoint().unwrap();
                first
            };
            context
                .services()
                .report_progress(TestProgress::Build, TestUnit::Items, 1, Some(2))
                .unwrap();
            assert_eq!(signal.polls.load(Ordering::SeqCst), polls + 4);
            assert_eq!(callbacks.load(Ordering::SeqCst), 1);
            assert_eq!(observation.live_bytes(context), baseline + 4);
            let second = context
                .services()
                .ordinary_storage()
                .reserve(keys.phase, keys.dimension, 16)
                .unwrap();
            context
                .services()
                .report_progress(TestProgress::Build, TestUnit::Items, 2, Some(2))
                .unwrap();
            assert_eq!(signal.polls.load(Ordering::SeqCst), polls + 7);
            assert_eq!(callbacks.load(Ordering::SeqCst), 2);
            assert_eq!(observation.live_bytes(context), baseline + 20);
            drop((first, second));
            assert_eq!(observation.live_bytes(context), baseline);
            Ok(())
        })
        .unwrap();
    assert_eq!(physical_delta(physical, account.snapshot()), (0, 0));
    let _ = operation.prepare_minimal_fallback(71).unwrap().handoff();
    assert!(!account.snapshot().poisoned());
}

#[test]
fn weighted_control_frontiers_do_not_charge_estimated_inner_work() {
    for costs in [
        vec![118_u16, 118, 35],
        vec![255, 1, 1],
        vec![32; 9],
        vec![9; 29],
        vec![19; 14],
        vec![35; 8],
    ] {
        let factory = factory(1);
        let account = account(&factory);
        let signal = CountedCancellation::default();
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default().with_cancellation(signal.clone()),
            DimensionPolicy::counter(64, 64, 0),
        );
        let mut physical = vec![0; costs.len()];
        operation
            .with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |execution| {
                let entry_polls = signal.polls.load(Ordering::SeqCst);
                for (index, (&cost, destination)) in costs.iter().zip(&mut physical).enumerate() {
                    execution.begin_action(cost)?;
                    assert_eq!(
                        signal.polls.load(Ordering::SeqCst),
                        entry_polls + usize::from(index + 1 == costs.len())
                    );
                    *destination = 1;
                    execution.complete_action()?;
                }
                Ok(())
            })
            .unwrap();
        assert!(physical.iter().all(|value| *value == 1));
        let published = operation.prepare_minimal_fallback(71).unwrap().handoff();
        assert_work(
            &published.ledger_evidence,
            keys.other_dimension,
            costs.len() as u64,
            costs.len() as u64,
        );
    }
}

#[test]
fn weighted_frontier_cancellation_precedes_next_action_and_retains_cause_on_region_reentry() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default().with_cancellation(signal.clone()),
        DimensionPolicy::counter(8, 8, 0),
    );
    let mut output = [0; 3];
    let result = operation.with_action_execution::<_, StoreRuntimeError>(
        keys.other_dimension,
        |execution| {
            for destination in output.iter_mut().take(2) {
                execution.begin_action(118)?;
                *destination = 1;
                execution.complete_action()?;
            }
            let polls = signal.polls.load(Ordering::SeqCst);
            signal.token.cancel();
            let next = execution.begin_action(35);
            if next.is_ok() {
                output[2] = 1;
            }
            assert!(matches!(
                next,
                Err(StoreRuntimeError::Control(ControlCause::Cancelled { .. }))
            ));
            assert_eq!(signal.polls.load(Ordering::SeqCst), polls + 1);
            next
        },
    );
    assert!(matches!(result, Err(StoreRuntimeError::Control(_))));
    assert_eq!(output, [1, 1, 0]);
    let polls = signal.polls.load(Ordering::SeqCst);
    assert!(matches!(
        operation.with_action_execution::<(), StoreRuntimeError>(keys.other_dimension, |_| panic!(
            "a cancelled region cannot reenter"
        )),
        Err(StoreRuntimeError::Control(_))
    ));
    assert_eq!(signal.polls.load(Ordering::SeqCst), polls);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Control);
    assert_work(&evidence, keys.other_dimension, 2, 2);
}

#[test]
fn invalid_control_cost_is_sticky_without_running_or_charging_work() {
    for invalid in [0_u16, 257, u16::MAX] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(8, 8, 0),
        );
        let result = operation.with_action_execution::<_, StoreRuntimeError>(
            keys.other_dimension,
            |execution| {
                let polls = execution.work_counters(keys.other_dimension)?;
                assert!(matches!(
                    execution.begin_action(invalid),
                    Err(StoreRuntimeError::FallbackOnly)
                ));
                assert_eq!(execution.work_counters(keys.other_dimension)?, polls);
                assert!(matches!(
                    execution.begin_action(1),
                    Err(StoreRuntimeError::FallbackOnly)
                ));
                Ok(())
            },
        );
        assert!(matches!(result, Err(StoreRuntimeError::FallbackOnly)));
        let evidence =
            rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
        assert_work(&evidence, keys.other_dimension, 0, 0);
    }
}

#[path = "execution/exclusive.rs"]
mod exclusive;
