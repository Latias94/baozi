#![allow(
    clippy::result_large_err,
    reason = "tests retain the exact inline callback and accounting errors"
)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use baozi_core::__scene_integration::integration::try_start_scene_edit_operation;
use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, CounterClass, LedgerCause,
    StorePreparationFailureClass, StoreTerminalCause,
};

use super::*;
use crate::{PostProcessBudgets, PostProcessControl, test_fixtures};

struct CountPolls {
    polls: Arc<AtomicUsize>,
    cancel_at: Arc<AtomicUsize>,
}

impl CancellationSignal for CountPolls {
    fn is_cancelled(&self) -> bool {
        let poll = self.polls.fetch_add(1, Ordering::Relaxed) + 1;
        poll >= self.cancel_at.load(Ordering::Relaxed)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TestError {
    Callback(&'static str),
    Runtime(StoreRuntimeError),
}

impl From<StoreRuntimeError> for TestError {
    fn from(cause: StoreRuntimeError) -> Self {
        Self::Runtime(cause)
    }
}

fn start(polls: &Arc<AtomicUsize>) -> (EditOperation, RuntimeKeys, u64) {
    start_with_cancel_at(polls, &Arc::new(AtomicUsize::new(usize::MAX)))
}

fn start_with_cancel_at(
    polls: &Arc<AtomicUsize>,
    cancel_at: &Arc<AtomicUsize>,
) -> (EditOperation, RuntimeKeys, u64) {
    let scene = test_fixtures::canonical_triangle_scene();
    let (writer, _, _) = SceneEditBootstrap::try_new().unwrap().into_parts();
    let (keys, ledger, plan) =
        plan_selected_runtime(writer, PostProcessBudgets::default()).unwrap();
    let control = PostProcessControl::default().with_cancellation_signal(CountPolls {
        polls: Arc::clone(polls),
        cancel_at: Arc::clone(cancel_at),
    });
    let mut operation = try_start_scene_edit_operation::<PostProcessOwner, _, _>(
        &scene,
        control.into_runtime(),
        ledger,
        plan,
    )
    .unwrap();
    let initial = operation
        .producer_host()
        .dimension_state(keys.work)
        .unwrap();
    assert_eq!(initial.attempted(), initial.consumed());
    let completed = initial.consumed();
    (operation, keys, completed)
}

fn assert_failed_prefix(
    operation: EditOperation,
    keys: RuntimeKeys,
    class: StorePreparationFailureClass,
    attempted: u64,
    consumed: u64,
) {
    // An ignored adapter failure must still refuse success and retain usable evidence.
    let prepared = operation
        .prepare_evidence_only_success(final_progress())
        .unwrap();
    let PreparedStoreEvidenceOnlyOutcome::MinimalFallback(fallback) = prepared else {
        panic!("unfinished or rejected runtime work must prevent success");
    };
    let evidence = fallback.handoff().ledger_evidence;
    let StoreTerminalCause::Failure(cause) = evidence.header().terminal_cause() else {
        panic!("the terminal owner must retain its failure");
    };
    assert_eq!(cause.class(), class);
    let work = evidence.dimension(keys.work).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (attempted, consumed));
    assert_eq!(evidence.header().current_live_bytes(), 0);
    assert_eq!(evidence.header().active_reservations(), 0);
}

#[test]
fn pending_callback_error_survives_cleanup_with_its_completed_prefix() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let mut physical = [0; 2];
    let mut polls_at_exit = 0;
    let result = with_execution(&mut operation, keys, |active| {
        run_unit(active, || physical[0] = 1)?;
        active.begin_action(1)?;
        physical[1] = 2;
        polls_at_exit = polls.load(Ordering::Relaxed);
        Err::<(), _>(TestError::Callback("callback rejected the leaf"))
    });
    assert_eq!(
        result,
        Err(TestError::Callback("callback rejected the leaf"))
    );
    assert_eq!(physical, [1, 2]);
    assert_eq!(
        polls.load(Ordering::Relaxed),
        polls_at_exit,
        "cleanup must not poll"
    );
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::ProductGraph,
        initial + 2,
        initial + 1,
    );
}

#[test]
fn actual_leaf_unwind_releases_pending_without_completing_or_polling() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let mut physical = [0; 2];
    let mut polls_at_exit = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_execution(&mut operation, keys, |active| {
            run_unit(active, || physical[0] = 1)?;
            run_unit(active, || -> () {
                physical[1] = 2;
                polls_at_exit = polls.load(Ordering::Relaxed);
                std::panic::panic_any("injected physical leaf panic");
            })
        })
    }));
    let payload = result.expect_err("the physical leaf must unwind through the adapter");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"injected physical leaf panic")
    );
    assert_eq!(physical, [1, 2]);
    assert_eq!(
        polls.load(Ordering::Relaxed),
        polls_at_exit,
        "unwind must not poll"
    );
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::ProductGraph,
        initial + 2,
        initial + 1,
    );
}

#[test]
fn successful_return_with_pending_ownership_is_a_sticky_contract_failure() {
    for physically_complete in [false, true] {
        let polls = Arc::new(AtomicUsize::new(0));
        let (mut operation, keys, initial) = start(&polls);
        let mut physical = 0;
        let mut polls_at_exit = 0;
        let result = with_execution(&mut operation, keys, |active| {
            active.begin_action(1)?;
            if physically_complete {
                physical = 1;
                // Even zero remaining work still requires explicit ownership resolution.
                active.pending.as_mut().unwrap().complete([1])?;
            }
            polls_at_exit = polls.load(Ordering::Relaxed);
            Ok::<_, StoreRuntimeError>(())
        });
        assert_eq!(result, Err(StoreRuntimeError::ObservedWorkIncomplete));
        assert_eq!(physical, u64::from(physically_complete));
        assert_eq!(
            polls.load(Ordering::Relaxed),
            polls_at_exit,
            "contract cleanup must not poll"
        );
        assert_failed_prefix(
            operation,
            keys,
            StorePreparationFailureClass::ProductGraph,
            initial + 1,
            initial + physical,
        );
    }
}

#[test]
fn failed_completion_keeps_the_actual_write_and_original_ledger_cause() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let mut physical = 0;
    let mut polls_at_exit = 0;
    let expected = StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
        dimension: keys.work,
        class: CounterClass::Consumed,
        value: initial + 1,
        limit: initial,
    });
    let result = with_execution(&mut operation, keys, |active| {
        active.begin_action(118)?;
        assert_eq!(active.action_pending, Some(118));
        physical = 1;
        // A policy tightened after admission rejects the physical completion, not its prefix.
        let tightened = active
            .work
            .tighten_dimension_policy(keys.work, DimensionPolicy::counter(MAX_WORK, initial, 0));
        assert_eq!(tightened, Err(expected.clone()));
        let completed = active.complete_action();
        assert_eq!(completed, Err(expected.clone()));
        assert!(active.pending.is_none());
        assert!(active.action_pending.is_none());
        active.work.finish_region().unwrap();
        polls_at_exit = polls.load(Ordering::Relaxed);
        completed
    });
    assert_eq!(result, Err(expected));
    assert_eq!(physical, 1);
    assert_eq!(
        polls.load(Ordering::Relaxed),
        polls_at_exit,
        "failed completion must not poll"
    );
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::Ledger,
        initial + 1,
        initial + 1,
    );
}

#[test]
fn idle_success_and_known_error_explicitly_close_without_abandonment() {
    for known_error in [false, true] {
        let polls = Arc::new(AtomicUsize::new(0));
        let (mut operation, keys, initial) = start(&polls);
        let mut physical = [0; 3];
        let mut polls_at_exit = 0;
        let result = with_execution(&mut operation, keys, |active| {
            assert!(active.pending.is_none());
            for (index, destination) in physical.iter_mut().enumerate() {
                run_unit(active, || *destination = index + 1)?;
                assert!(active.action_pending.is_none());
                assert_eq!(active.pending.as_ref().unwrap().remaining(), [0]);
            }
            polls_at_exit = polls.load(Ordering::Relaxed);
            if known_error {
                Err(TestError::Callback("known idle error"))
            } else {
                Ok(())
            }
        });
        assert_eq!(physical, [1, 2, 3]);
        if known_error {
            assert_eq!(result, Err(TestError::Callback("known idle error")));
        } else {
            assert_eq!(result, Ok(()));
        }
        assert_eq!(
            polls.load(Ordering::Relaxed),
            polls_at_exit + usize::from(!known_error)
        );
        let prepared = operation
            .prepare_evidence_only_success(final_progress())
            .unwrap();
        let PreparedStoreEvidenceOnlyOutcome::Success(success) = prepared else {
            panic!("a resolved normal return must not introduce ProductGraph failure");
        };
        let evidence = success.handoff().ledger_evidence;
        assert_eq!(
            evidence.header().terminal_cause(),
            StoreTerminalCause::Success
        );
        let work = evidence.dimension(keys.work).unwrap();
        assert_eq!(
            (work.attempted(), work.consumed()),
            (initial + 3, initial + 3)
        );
    }
}

#[test]
fn panic_between_completed_leaves_keeps_abandonment_without_polling() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let mut physical = 0;
    let mut polls_at_exit = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_execution(
            &mut operation,
            keys,
            |active| -> Result<(), StoreRuntimeError> {
                run_unit(active, || physical = 1)?;
                assert_eq!(active.pending.as_ref().unwrap().remaining(), [0]);
                assert!(active.action_pending.is_none());
                polls_at_exit = polls.load(Ordering::Relaxed);
                std::panic::panic_any("injected idle region panic");
            },
        )
    }));
    let payload = result.expect_err("idle region unwind must not become a verified error return");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"injected idle region panic")
    );
    assert_eq!(physical, 1);
    assert_eq!(polls.load(Ordering::Relaxed), polls_at_exit);
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::ProductGraph,
        initial + 1,
        initial + 1,
    );
}

#[test]
fn callback_capture_drop_unwinds_through_idle_sequence_without_polling() {
    struct CaptureDrop(Arc<AtomicUsize>);
    impl Drop for CaptureDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
            std::panic::panic_any("callback capture drop");
        }
    }

    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let mut physical = 0;
    let mut polls_at_exit = 0;
    let captured = CaptureDrop(Arc::clone(&drops));
    let destination = &mut physical;
    let observed_polls = &mut polls_at_exit;
    let counter = Arc::clone(&polls);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_execution(
            &mut operation,
            keys,
            move |active| -> Result<(), StoreRuntimeError> {
                run_unit(active, || *destination = 1)?;
                assert!(active.action_pending.is_none());
                assert_eq!(active.pending.as_ref().unwrap().remaining(), [0]);
                *observed_polls = counter.load(Ordering::Relaxed);
                std::hint::black_box(&captured);
                Ok(())
            },
        )
    }));
    let payload = result.expect_err("the owned callback capture must unwind at return");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"callback capture drop")
    );
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(physical, 1);
    assert_eq!(polls.load(Ordering::Relaxed), polls_at_exit);
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::ProductGraph,
        initial + 1,
        initial + 1,
    );
}

#[test]
fn completed_leaf_value_drop_unwinds_through_idle_sequence_without_polling() {
    struct ReturnedDrop(Arc<AtomicUsize>);
    impl Drop for ReturnedDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
            std::panic::panic_any("completed leaf value drop");
        }
    }

    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let mut physical = 0;
    let mut polls_at_exit = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_execution(
            &mut operation,
            keys,
            |active| -> Result<(), StoreRuntimeError> {
                let returned = run_unit(active, || {
                    physical = 1;
                    ReturnedDrop(Arc::clone(&drops))
                })?;
                assert!(active.action_pending.is_none());
                assert_eq!(active.pending.as_ref().unwrap().remaining(), [0]);
                polls_at_exit = polls.load(Ordering::Relaxed);
                std::hint::black_box(&returned);
                Ok(())
            },
        )
    }));
    let payload = result.expect_err("a completed leaf value must unwind during normal destruction");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"completed leaf value drop")
    );
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(physical, 1);
    assert_eq!(polls.load(Ordering::Relaxed), polls_at_exit);
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::ProductGraph,
        initial + 1,
        initial + 1,
    );
}

#[test]
fn runtime_refill_denial_keeps_the_literal_write_prefix_and_original_ledger_cause() {
    for class in [CounterClass::Attempted, CounterClass::Consumed] {
        let polls = Arc::new(AtomicUsize::new(0));
        let (mut operation, keys, initial) = start(&polls);
        let mut physical = [0; 2];
        let mut polls_at_exit = 0;
        let expected = StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
            dimension: keys.work,
            class,
            value: initial + 2,
            limit: initial + 1,
        });
        let result = with_execution(&mut operation, keys, |active| {
            run_unit(active, || physical[0] = 1)?;
            let attempted = if class == CounterClass::Attempted {
                initial + 1
            } else {
                MAX_WORK
            };
            active.work.tighten_dimension_policy(
                keys.work,
                DimensionPolicy::counter(attempted, initial + 1, 0),
            )?;
            let denied = active.begin_action(1);
            if denied.is_ok() {
                physical[1] = 1;
            }
            assert_eq!(denied, Err(expected.clone()));
            assert!(active.action_pending.is_none());
            assert_eq!(active.pending.as_ref().unwrap().remaining(), [0]);
            polls_at_exit = polls.load(Ordering::Relaxed);
            denied
        });
        assert_eq!(result, Err(expected));
        assert_eq!(physical, [1, 0]);
        assert_eq!(polls.load(Ordering::Relaxed), polls_at_exit);
        assert_failed_prefix(
            operation,
            keys,
            StorePreparationFailureClass::Ledger,
            initial + 2,
            initial + 1,
        );
    }
}

#[test]
fn scratch_share_between_leaves_keeps_one_authority_and_releases_physical_storage() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let before = operation.account().snapshot();
    let mut physical = [0; 2];
    with_execution(
        &mut operation,
        keys,
        |active| -> Result<(), StoreRuntimeError> {
            run_unit(active, || physical[0] = 1)?;
            let mut scratch = ScratchVec::<u8>::new(active, keys, 3)?;
            run_unit(active, || {
                scratch.values_mut().extend_from_slice(&[4, 5, 6])
            })?;
            assert_eq!(scratch.values().as_slice(), [4, 5, 6]);
            assert_eq!(active.pending.as_ref().unwrap().remaining(), [0]);
            drop(scratch);
            run_unit(active, || physical[1] = 2)?;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(physical, [1, 2]);
    let after = operation.account().snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert!(!after.poisoned());
    let prepared = operation
        .prepare_evidence_only_success(final_progress())
        .unwrap();
    let PreparedStoreEvidenceOnlyOutcome::Success(success) = prepared else {
        panic!("scratch sharing must not abandon the idle producer sequence");
    };
    let evidence = success.handoff().ledger_evidence;
    let work = evidence.dimension(keys.work).unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (initial + 5, initial + 5)
    );
}

#[test]
fn scratch_share_rejection_closes_idle_sequence_without_completing_future_work() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let before = operation.account().snapshot();
    let mut physical = 0;
    let mut polls_at_exit = 0;
    let result = with_execution(&mut operation, keys, |active| {
        run_unit(active, || physical = 1)?;
        active.work.tighten_dimension_policy(
            keys.work,
            DimensionPolicy::counter(MAX_WORK, initial + 2, 0),
        )?;
        let scratch = ScratchVec::<u8>::new(active, keys, 3);
        polls_at_exit = polls.load(Ordering::Relaxed);
        scratch.map(drop)
    });
    assert!(matches!(
        result,
        Err(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension,
            value,
            limit,
        })) if dimension == keys.work && value == initial + 3 && limit == initial + 2
    ));
    assert_eq!(physical, 1);
    assert_eq!(polls.load(Ordering::Relaxed), polls_at_exit);
    let after = operation.account().snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert!(!after.poisoned());
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::Ledger,
        initial + 3,
        initial + 1,
    );
}

#[test]
fn final_cancellation_closes_idle_sequences_and_preserves_short_and_full_prefixes() {
    for actions in [0, 1, 31, 32, 33, 255, 256, 257] {
        for inside_checkpoint in [false, true] {
            let polls = Arc::new(AtomicUsize::new(0));
            let cancel_at = Arc::new(AtomicUsize::new(usize::MAX));
            let (mut operation, keys, initial) = start_with_cancel_at(&polls, &cancel_at);
            let mut physical = [0_u8; 257];
            let mut polls_at_exit = 0;
            let result = with_execution(&mut operation, keys, |active| {
                for destination in physical.iter_mut().take(actions) {
                    run_unit(active, || *destination = 1)?;
                }
                assert!(active.action_pending.is_none());
                assert_eq!(active.pending.is_some(), actions != 0);
                polls_at_exit = polls.load(Ordering::Relaxed);
                cancel_at.store(polls_at_exit + 1, Ordering::Relaxed);
                if inside_checkpoint {
                    active.checkpoint()?;
                }
                Ok::<(), StoreRuntimeError>(())
            });
            assert_eq!(
                result,
                Err(StoreRuntimeError::Control(ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                }))
            );
            assert!(physical[..actions].iter().all(|value| *value == 1));
            assert!(physical[actions..].iter().all(|value| *value == 0));
            assert_eq!(polls.load(Ordering::Relaxed), polls_at_exit + 1);
            assert_failed_prefix(
                operation,
                keys,
                StorePreparationFailureClass::Control,
                initial + actions as u64,
                initial + actions as u64,
            );
        }
    }
}

#[test]
fn weighted_begin_captures_cost_only_after_admission_and_counts_one_action() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (mut operation, keys, initial) = start(&polls);
    let mut physical = [0; 2];
    let result = with_execution(&mut operation, keys, |active| {
        active.begin_action(118)?;
        assert_eq!(active.action_pending, Some(118));
        physical[0] = 1;
        active.complete_action()?;
        assert!(active.action_pending.is_none());
        active.work.tighten_dimension_policy(
            keys.work,
            DimensionPolicy::counter(initial + 1, initial + 1, 0),
        )?;
        let denied = active.begin_action(35);
        if denied.is_ok() {
            physical[1] = 1;
        }
        assert!(matches!(denied, Err(StoreRuntimeError::Ledger(_))));
        assert!(active.action_pending.is_none());
        denied
    });
    assert!(matches!(result, Err(StoreRuntimeError::Ledger(_))));
    assert_eq!(physical, [1, 0]);
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::Ledger,
        initial + 2,
        initial + 1,
    );
}

#[test]
fn weighted_next_action_polls_before_reading_and_idle_error_cleanup_adds_no_poll() {
    let polls = Arc::new(AtomicUsize::new(0));
    let cancel_at = Arc::new(AtomicUsize::new(usize::MAX));
    let (mut operation, keys, initial) = start_with_cancel_at(&polls, &cancel_at);
    let mut physical = [0; 3];
    let mut before = 0;
    let result = with_execution(&mut operation, keys, |active| {
        for destination in physical.iter_mut().take(2) {
            active.begin_action(118)?;
            *destination = 1;
            active.complete_action()?;
        }
        before = polls.load(Ordering::Relaxed);
        cancel_at.store(before + 1, Ordering::Relaxed);
        active.begin_action(35)?;
        physical[2] = 1;
        active.complete_action()
    });
    assert_eq!(
        result,
        Err(StoreRuntimeError::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal
        }))
    );
    assert_eq!(physical, [1, 1, 0]);
    assert_eq!(polls.load(Ordering::Relaxed), before + 1);
    assert_failed_prefix(
        operation,
        keys,
        StorePreparationFailureClass::Control,
        initial + 2,
        initial + 2,
    );
}
