#![allow(
    clippy::result_large_err,
    reason = "tests retain the exact inline operation failure vocabulary"
)]

use std::sync::atomic::{AtomicUsize, Ordering};

use baozi_operation::__retained_store_integration::activation::{
    StoreBoundWorkGroup, StoreExecutionControl,
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

fn operation_with_work_policy(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    work_policy: DimensionPolicy,
) -> (TestOperation, TestKeys) {
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
        TestOperation::try_start(control, ledger_plan, account, start).unwrap(),
        keys,
    )
}

fn write_unit(
    group: &StoreBoundWorkGroup<'_, 1>,
    control: &mut StoreExecutionControl<'_, TestOwner, TestProgress, TestUnit>,
    destination: &mut u8,
) -> Result<(), StoreRuntimeError> {
    control.before_action()?;
    let mut permit = group.admit_observed([1])?;
    *destination = 1;
    permit.complete([1])?;
    control.completed_action();
    permit.finish()
}

fn rejected_success_evidence(
    operation: TestOperation,
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
    let (mut operation, keys) = operation(&account);
    let physical = account.snapshot();
    {
        let (work, storage, mut control) = operation.execution().unwrap();
        let baseline = work.dimension_state(keys.dimension).unwrap().live_bytes();
        let observation = work.storage_observation(keys.dimension).unwrap();
        let mut first = storage
            .reserve(&mut control, keys.phase, keys.dimension, 17)
            .unwrap();
        let cloned = observation;
        assert_eq!(observation.live_bytes(), baseline + 17);
        storage.resize(&mut first, 9).unwrap();
        assert_eq!(cloned.live_bytes(), baseline + 9);
        let second = storage
            .reserve(&mut control, keys.phase, keys.dimension, 31)
            .unwrap();
        assert_eq!(observation.live_bytes(), baseline + 40);
        assert_eq!(account.snapshot(), physical);
        drop(first);
        assert_eq!(cloned.live_bytes(), baseline + 31);
        drop(second);
        assert_eq!(observation.live_bytes(), baseline);
        work.finish_region().unwrap();
    }
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
    let (mut operation, keys) =
        operation_with_work_policy(&account, control, DimensionPolicy::counter(16, 16, 0));
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    let physical_before = account.snapshot();
    let mut output = [0; 3];

    let (retained, retained_bytes) = {
        let (work, storage, mut control) = operation.execution().unwrap();
        let group = work.bind_group([keys.other_dimension]).unwrap();
        // Moving the view proves that the group borrows the invocation, not this wrapper.
        let region = work;
        let mut permit = group.admit_observed([3]).unwrap();

        control.before_action().unwrap();
        output[0] = 1;
        permit.complete([1]).unwrap();
        control.completed_action();
        control
            .report_progress(TestProgress::Build, TestUnit::Items, 1, None)
            .unwrap();
        assert_eq!(callbacks.load(Ordering::SeqCst), 1);

        let temporary_plan = CapacityPlan::for_type::<u8>(0, 0, 17).unwrap();
        let mut temporary = storage
            .reserve(
                &mut control,
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
        storage.resize(&mut temporary, scratch_bytes).unwrap();
        scratch.resize(temporary_plan.desired_capacity(), 7);
        assert_eq!(scratch, [7; 17]);
        drop(scratch);
        drop(temporary);

        let retained_plan = CapacityPlan::for_type::<u8>(0, 0, 31).unwrap();
        let mut reservation = storage
            .reserve_retained(
                &mut control,
                keys.phase,
                keys.dimension,
                retained_plan.envelope_delta_bytes().unwrap(),
                1,
            )
            .expect("the internal account share can nest inside an ordinary permit");
        let mut value = Vec::<u8>::new();
        value
            .try_reserve_exact(retained_plan.allocator_additional())
            .unwrap();
        let retained_bytes = retained_plan.finish(value.len(), value.capacity()).unwrap();
        storage.resize(&mut reservation, retained_bytes).unwrap();
        value.resize(retained_plan.desired_capacity(), 9);
        let retained = reservation.bind_storage(value).unwrap();

        assert_eq!(permit.remaining(), [2]);
        control.before_action().unwrap();
        output[1] = 2;
        permit.complete([1]).unwrap();
        control.completed_action();
        permit.finish_partial().unwrap();
        write_unit(&group, &mut control, &mut output[2]).unwrap();
        region.finish_region().unwrap();
        control.checkpoint().unwrap();
        (retained, retained_bytes)
    };

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
    let (mut operation, keys) = operation_with_work_policy(
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
    {
        let (work, storage, mut control) = operation.execution().unwrap();
        let group = work.bind_group([keys.dimension]).unwrap();
        control.before_action().unwrap();
        let mut permit = group.admit_observed([1]).unwrap();
        output[0] = 1;
        permit.complete([1]).unwrap();
        control.completed_action();

        let plan = CapacityPlan::for_type::<u8>(0, 0, 5).unwrap();
        let mut reservation = storage
            .reserve_retained(
                &mut control,
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
        storage.resize(&mut reservation, bytes).unwrap();
        values.extend_from_slice(&[2, 3, 4, 5, 6]);
        let retained = reservation.bind_storage(values).unwrap();
        assert_eq!(retained.as_ref(), &[2, 3, 4, 5, 6]);
        assert_eq!(physical_delta(physical, account.snapshot()), (bytes, 1));
        assert_eq!(permit.remaining(), [0]);

        control.before_action().unwrap();
        permit.replenish_one().unwrap();
        output[1] = retained.as_ref()[0];
        permit.complete([1]).unwrap();
        control.completed_action();
        permit.finish().unwrap();
        drop(retained);
        let state = work.dimension_state(keys.dimension).unwrap();
        assert_eq!(state.attempted(), baseline.attempted() + 4);
        assert_eq!(state.consumed(), baseline.consumed() + 4);
        assert_eq!(state.live_bytes(), baseline.live_bytes());
        work.finish_region().unwrap();
        control.checkpoint().unwrap();
    }
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
    let (mut operation, keys) = operation_with_work_policy(
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(8, 8, 0),
    );
    let mut output = [73; 3];
    {
        let (work, _storage, mut control) = operation.execution().unwrap();
        let group = work.bind_group([keys.other_dimension]).unwrap();
        let mut permit = group.admit_observed([0]).unwrap();
        for destination in output.iter_mut().take(2) {
            control.before_action().unwrap();
            permit.replenish_one().unwrap();
            *destination = 1;
            permit.complete([1]).unwrap();
            control.completed_action();
        }
        work.tighten_dimension_policy(keys.other_dimension, DimensionPolicy::counter(2, 2, 0))
            .unwrap();
        let denied = permit.replenish_one();
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
            permit.replenish_one(),
            Err(StoreRuntimeError::FallbackOnly)
        ));
        assert_eq!(permit.remaining(), [0]);
        permit.finish_partial().unwrap();
        work.finish_region().unwrap();
    }
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
        let (mut operation, keys) =
            operation_with_work_policy(&account, control, DimensionPolicy::counter(512, 512, 0));
        signal.polls.store(0, Ordering::SeqCst);
        let mut physical = [0; 257];
        {
            let (work, _storage, mut control) = operation.execution().unwrap();
            assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
            let group = work.bind_group([keys.other_dimension]).unwrap();
            let mut permit = group.admit_observed([0]).unwrap();
            for destination in physical.iter_mut().take(actions) {
                control.before_action().unwrap();
                permit.replenish_one().unwrap();
                *destination = 1;
                permit.complete([1]).unwrap();
                control.completed_action();
            }
            permit.finish().unwrap();
            assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls - 1);
            work.finish_region().unwrap();
            assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls - 1);
            control.checkpoint().unwrap();
            assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls);
            assert_eq!(progress_events.load(Ordering::SeqCst), 0);
        }
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
        let (mut operation, keys) = operation_with_work_policy(
            &account,
            OperationControl::default().with_cancellation(signal.clone()),
            DimensionPolicy::counter(1024, 1024, 0),
        );
        signal.polls.store(0, Ordering::SeqCst);
        let mut physical = [0; 513];
        if cancel_after == 0 {
            signal.token.cancel();
            assert!(matches!(
                operation.execution(),
                Err(StoreRuntimeError::Control(ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                }))
            ));
        } else {
            let (work, _storage, mut control) = operation.execution().unwrap();
            let group = work.bind_group([keys.other_dimension]).unwrap();
            let mut permit = group.admit_observed([0]).unwrap();
            let mut stopped = false;
            for (index, destination) in physical.iter_mut().enumerate() {
                let result = control
                    .before_action()
                    .and_then(|()| permit.replenish_one());
                if result.is_err() {
                    assert!(matches!(
                        result,
                        Err(StoreRuntimeError::Control(ControlCause::Cancelled {
                            origin: CancellationOrigin::Signal,
                        }))
                    ));
                    assert_eq!(index, expected_actions);
                    stopped = true;
                    break;
                }
                *destination = 1;
                permit.complete([1]).unwrap();
                control.completed_action();
                if index + 1 == cancel_after {
                    signal.token.cancel();
                }
            }
            assert!(
                stopped,
                "cancellation must be observed within the finite action bound"
            );
            assert!(expected_actions - cancel_after <= 256);
            assert!(matches!(
                permit.replenish_one(),
                Err(StoreRuntimeError::FallbackOnly)
            ));
            permit.finish_partial().unwrap();
            assert!(matches!(
                group.admit_observed([0]),
                Err(StoreRuntimeError::FallbackOnly)
            ));
            work.finish_region().unwrap();
            assert!(matches!(
                control.checkpoint(),
                Err(StoreRuntimeError::Control(ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                }))
            ));
            assert!(matches!(
                control.before_action(),
                Err(StoreRuntimeError::Control(ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                }))
            ));
        }
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
        let (mut operation, keys) = operation_with_work_policy(
            &account,
            OperationControl::default().with_cancellation(signal.clone()),
            DimensionPolicy::counter(512, 512, 0),
        );
        signal.polls.store(0, Ordering::SeqCst);
        let mut physical = [0; 257];
        {
            let (work, _storage, mut control) = operation.execution().unwrap();
            let group = work.bind_group([keys.other_dimension]).unwrap();
            let mut permit = group.admit_observed([attempted]).unwrap();
            for destination in physical.iter_mut().take(actions) {
                control.before_action().unwrap();
                *destination = 1;
                permit.complete([1]).unwrap();
                control.completed_action();
            }
            signal.token.cancel();
            assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls - 1);
            assert!(matches!(
                control.checkpoint(),
                Err(StoreRuntimeError::Control(ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                }))
            ));
            assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls);
            assert_eq!(permit.remaining(), [1]);
            permit.finish_partial().unwrap();
            work.finish_region().unwrap();
        }
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
    let (mut operation, keys) = operation_with_work_policy(
        &account,
        OperationControl::default().with_cancellation(signal.clone()),
        DimensionPolicy::counter(1024, 1024, 0),
    );
    signal.polls.store(0, Ordering::SeqCst);
    let mut physical = [0; 512];
    {
        let (work, _storage, mut control) = operation.execution().unwrap();
        let group = work.bind_group([keys.other_dimension]).unwrap();
        for destination in &mut physical[..255] {
            write_unit(&group, &mut control, destination).unwrap();
        }
        assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
        control.checkpoint().unwrap();
        assert_eq!(signal.polls.load(Ordering::SeqCst), 2);
        for destination in &mut physical[255..511] {
            write_unit(&group, &mut control, destination).unwrap();
        }
        assert_eq!(signal.polls.load(Ordering::SeqCst), 2);
        write_unit(&group, &mut control, &mut physical[511]).unwrap();
        assert_eq!(signal.polls.load(Ordering::SeqCst), 3);
        work.finish_region().unwrap();
        control.checkpoint().unwrap();
        assert_eq!(signal.polls.load(Ordering::SeqCst), 4);
    }
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
            let (mut operation, keys) = operation_with_work_policy(
                &account,
                OperationControl::default().with_cancellation(signal.clone()),
                policy,
            );
            signal.polls.store(0, Ordering::SeqCst);
            let mut physical = [0; 256];
            {
                let (work, _storage, mut control) = operation.execution().unwrap();
                let group = work.bind_group([keys.other_dimension]).unwrap();
                let mut permit = group.admit_observed([0]).unwrap();
                for destination in physical.iter_mut().take(allowed) {
                    control.before_action().unwrap();
                    permit.replenish_one().unwrap();
                    *destination = 1;
                    permit.complete([1]).unwrap();
                    control.completed_action();
                }
                control.before_action().unwrap();
                let rejected = permit.replenish_one();
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
                assert_eq!(permit.remaining(), [0]);
                permit.finish_partial().unwrap();
                assert!(matches!(
                    group.admit_observed([0]),
                    Err(StoreRuntimeError::FallbackOnly)
                ));
                assert!(matches!(
                    control.before_action(),
                    Err(StoreRuntimeError::FallbackOnly)
                ));
                work.finish_region().unwrap();
                assert!(matches!(
                    control.checkpoint(),
                    Err(StoreRuntimeError::FallbackOnly)
                ));
            }
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
fn finish_region_rejects_forgotten_fully_completed_and_zero_amount_permits() {
    for amount in [0, 3] {
        let factory = factory(1);
        let account = account(&factory);
        let (mut operation, keys) = operation_with_work_policy(
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        {
            let (work, _storage, mut control) = operation.execution().unwrap();
            let group = work.bind_group([keys.other_dimension]).unwrap();
            let mut permit = group.admit_observed([0]).unwrap();
            let mut physical = [0; 3];
            for destination in physical.iter_mut().take(amount as usize) {
                control.before_action().unwrap();
                permit.replenish_one().unwrap();
                *destination = 1;
                permit.complete([1]).unwrap();
                control.completed_action();
            }
            assert_eq!(u64::from(physical.iter().sum::<u8>()), amount);
            assert_eq!(permit.remaining(), [0]);
            std::mem::forget(permit);
            assert!(matches!(
                work.finish_region(),
                Err(StoreRuntimeError::Ledger(
                    LedgerCause::OutstandingLinearWorkPermits { count: 1 }
                ))
            ));
            assert!(matches!(
                control.checkpoint(),
                Err(StoreRuntimeError::FallbackOnly)
            ));
        }
        assert!(matches!(
            operation.prepare_evidence_only_success(final_progress()),
            Err(StorePrepareError::FinalizationContainment(_))
        ));
    }
}

#[test]
fn dropping_fully_completed_and_zero_amount_permits_still_prevents_success() {
    for amount in [0, 3] {
        let factory = factory(1);
        let account = account(&factory);
        let (mut operation, keys) = operation_with_work_policy(
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        {
            let (work, _storage, mut control) = operation.execution().unwrap();
            let group = work.bind_group([keys.other_dimension]).unwrap();
            let mut permit = group.admit_observed([0]).unwrap();
            let mut physical = [0; 3];
            for destination in physical.iter_mut().take(amount as usize) {
                control.before_action().unwrap();
                permit.replenish_one().unwrap();
                *destination = 1;
                permit.complete([1]).unwrap();
                control.completed_action();
            }
            assert_eq!(u64::from(physical.iter().sum::<u8>()), amount);
            drop(permit);
            work.finish_region().unwrap();
            assert!(matches!(
                group.admit_observed([0]),
                Err(StoreRuntimeError::FallbackOnly)
            ));
            assert!(matches!(
                control.checkpoint(),
                Err(StoreRuntimeError::FallbackOnly)
            ));
        }
        let evidence =
            rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
        assert_work(&evidence, keys.other_dimension, amount, amount);
    }
}

#[test]
fn ignored_incomplete_finish_keeps_the_exact_prefix_and_sticky_failure() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation_with_work_policy(
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    let mut physical = 0;
    {
        let (work, _storage, mut control) = operation.execution().unwrap();
        let group = work.bind_group([keys.other_dimension]).unwrap();
        let mut permit = group.admit_observed([3]).unwrap();
        control.before_action().unwrap();
        physical += 1;
        permit.complete([1]).unwrap();
        control.completed_action();
        assert!(matches!(
            permit.finish(),
            Err(StoreRuntimeError::FallbackOnly)
        ));
        assert!(matches!(
            group.admit_observed([0]),
            Err(StoreRuntimeError::FallbackOnly)
        ));
        work.finish_region().unwrap();
        assert!(matches!(
            control.checkpoint(),
            Err(StoreRuntimeError::FallbackOnly)
        ));
    }
    assert_eq!(physical, 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
    assert_work(&evidence, keys.other_dimension, 3, 1);
}

#[test]
fn binding_rejects_foreign_unplanned_and_protected_keys_before_zero_work() {
    for invalid_role in 0..3 {
        let factory = factory(1);
        let account = account(&factory);
        let (mut operation, keys) = operation(&account);
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
        {
            let (work, _storage, mut control) = operation.execution().unwrap();
            match work.bind_group([keys.dimension, invalid_dimension]) {
                Err(StoreRuntimeError::ForeignDimensionDomain { dimension }) => {
                    assert_eq!(invalid_role, 0);
                    assert_eq!(dimension, invalid_dimension);
                }
                Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension { dimension })) => {
                    assert_ne!(invalid_role, 0);
                    assert_eq!(dimension, invalid_dimension);
                }
                _ => panic!("binding must validate every lane before any amount, including zero"),
            }
            assert!(matches!(
                work.bind_group([keys.dimension]),
                Err(StoreRuntimeError::FallbackOnly)
            ));
            work.finish_region().unwrap();
            assert!(matches!(
                control.checkpoint(),
                Err(StoreRuntimeError::FallbackOnly)
            ));
        }
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
    let (mut operation, keys) = operation_with_work_policy(
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    let physical_before = account.snapshot();
    {
        let (work, storage, mut control) = operation.execution().unwrap();
        let group = work.bind_group([keys.other_dimension]).unwrap();
        let mut parent = group.begin_parser_parent([2], [None]).unwrap();
        let mut child = group.begin_parser_child([3], [None]).unwrap();
        let mut decoded = [0; 3];
        decoded[0] = 8;
        child.complete([1]).unwrap();
        let plan = CapacityPlan::for_type::<u8>(0, 0, 8).unwrap();
        let mut reservation = storage
            .reserve_retained(
                &mut control,
                keys.phase,
                keys.dimension,
                plan.envelope_delta_bytes().unwrap(),
                1,
            )
            .unwrap();
        let mut bytes = Vec::<u8>::new();
        bytes
            .try_reserve_exact(plan.allocator_additional())
            .unwrap();
        storage
            .resize(
                &mut reservation,
                plan.finish(bytes.len(), bytes.capacity()).unwrap(),
            )
            .unwrap();
        let retained = reservation.bind_storage(bytes).unwrap();
        control
            .report_progress(TestProgress::Build, TestUnit::Items, 1, Some(3))
            .unwrap();
        decoded[1..].copy_from_slice(&[9, 10]);
        child.complete_all().unwrap();
        child.finish_exact().unwrap();
        control.checkpoint().unwrap();
        parent.complete_all().unwrap();
        parent.finish_exact().unwrap();
        work.finish_region().unwrap();
        assert_eq!(decoded, [8, 9, 10]);
        drop(retained);
    }
    assert_eq!(physical_delta(physical_before, account.snapshot()), (0, 0));
    let published = operation.prepare_minimal_fallback(71).unwrap().handoff();
    assert_work(&published.ledger_evidence, keys.other_dimension, 5, 5);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn ignored_parser_item_rejection_keeps_ordered_attempts_and_stops_success() {
    use baozi_operation::__retained_store_integration::activation::StoreParserWorkError;
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation_with_work_policy(
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(16, 16, 0),
    );
    {
        let (work, _storage, mut control) = operation.execution().unwrap();
        let group = work.bind_group([keys.other_dimension; 2]).unwrap();
        assert!(matches!(
            group.begin_parser_child([1, 3], [None, Some(2)]),
            Err(StoreParserWorkError::ItemLimit {
                index: 1,
                value: 3,
                limit: 2
            })
        ));
        work.finish_region().unwrap();
        assert!(matches!(
            control.checkpoint(),
            Err(StoreRuntimeError::FallbackOnly)
        ));
    }
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
    assert_work(&evidence, keys.other_dimension, 4, 0);
}

#[test]
fn ignored_parser_completion_and_incomplete_finish_errors_remain_terminal() {
    for incomplete_finish in [false, true] {
        let factory = factory(1);
        let account = account(&factory);
        let (mut operation, keys) = operation_with_work_policy(
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        {
            let (work, _storage, mut control) = operation.execution().unwrap();
            let group = work.bind_group([keys.other_dimension]).unwrap();
            let mut channel = group.begin_parser_child([4], [None]).unwrap();
            let mut decoded = [0; 4];
            decoded[..2].copy_from_slice(&[7, 8]);
            channel.complete([2]).unwrap();
            let result = if incomplete_finish {
                channel.finish_exact()
            } else {
                let rejected = channel.complete([3]);
                assert_eq!(channel.remaining(), [2]);
                channel.finish_partial().unwrap();
                rejected
            };
            assert!(matches!(
                result,
                Err(StoreRuntimeError::Ledger(
                    LedgerCause::InactiveLinearWorkPermit
                ))
            ));
            assert_eq!(decoded, [7, 8, 0, 0]);
            work.finish_region().unwrap();
            assert!(matches!(
                group.admit_observed([0]),
                Err(StoreRuntimeError::FallbackOnly)
            ));
            assert!(matches!(
                control.checkpoint(),
                Err(StoreRuntimeError::FallbackOnly)
            ));
        }
        let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
        assert_work(&evidence, keys.other_dimension, 4, 2);
    }
}

#[test]
fn forgotten_zero_and_completed_parser_channels_prevent_finalization() {
    for amount in [0, 1] {
        let factory = factory(1);
        let account = account(&factory);
        let (mut operation, keys) = operation_with_work_policy(
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        {
            let (work, _storage, _control) = operation.execution().unwrap();
            let group = work.bind_group([keys.other_dimension]).unwrap();
            let mut channel = group.begin_parser_child([amount], [None]).unwrap();
            channel.complete_all().unwrap();
            std::mem::forget(channel);
            assert!(matches!(
                work.finish_region(),
                Err(StoreRuntimeError::Ledger(
                    LedgerCause::OutstandingLinearWorkPermits { count: 1 }
                ))
            ));
        }
        assert!(matches!(
            operation.prepare_evidence_only_success(final_progress()),
            Err(StorePrepareError::FinalizationContainment(_))
        ));
    }
}

#[test]
fn storage_rejects_foreign_control_before_polling_or_admitting_bytes() {
    for retained in [false, true] {
        let factory = factory(1);
        let account = account(&factory);
        let (mut source, keys) = operation_with_work_policy(
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(16, 16, 0),
        );
        let signal = CountedCancellation::default();
        let (mut foreign, _) = operation_with_work_policy(
            &account,
            OperationControl::default().with_cancellation(signal.clone()),
            DimensionPolicy::counter(16, 16, 0),
        );
        let before = account.snapshot();
        {
            let (_work, storage, mut source_control) = source.execution().unwrap();
            let (_foreign_work, _foreign_storage, mut foreign_control) =
                foreign.execution().unwrap();
            let polls = signal.polls.load(Ordering::SeqCst);
            let result = if retained {
                storage.reserve_retained(&mut foreign_control, keys.phase, keys.dimension, 8, 1)
            } else {
                storage.reserve(&mut foreign_control, keys.phase, keys.dimension, 8)
            };
            assert!(matches!(
                result,
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            ));
            assert_eq!(signal.polls.load(Ordering::SeqCst), polls);
            assert!(matches!(
                source_control.checkpoint(),
                Err(StoreRuntimeError::FallbackOnly)
            ));
            foreign_control.checkpoint().unwrap();
        }
        assert_eq!(physical_delta(before, account.snapshot()), (0, 0));
        let _ = rejected_success_evidence(source, StorePreparationFailureClass::Ledger);
        let _ = foreign.prepare_minimal_fallback(71).unwrap().handoff();
        assert!(!account.snapshot().poisoned());
    }
}
