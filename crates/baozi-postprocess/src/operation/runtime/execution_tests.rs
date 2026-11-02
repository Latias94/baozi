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
    CancellationSignal, CounterClass, LedgerCause, StorePreparationFailureClass, StoreTerminalCause,
};

use super::*;
use crate::{PostProcessBudgets, PostProcessControl, test_fixtures};

struct CountPolls(Arc<AtomicUsize>);

impl CancellationSignal for CountPolls {
    fn is_cancelled(&self) -> bool {
        self.0.fetch_add(1, Ordering::Relaxed);
        false
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
    let scene = test_fixtures::canonical_triangle_scene();
    let (writer, _, _) = SceneEditBootstrap::try_new().unwrap().into_parts();
    let (keys, ledger, plan) =
        plan_selected_runtime(writer, PostProcessBudgets::default()).unwrap();
    let control =
        PostProcessControl::default().with_cancellation_signal(CountPolls(Arc::clone(polls)));
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
        active.begin_action()?;
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
            active.begin_action()?;
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
        active.begin_action()?;
        physical = 1;
        // A policy tightened after admission rejects the physical completion, not its prefix.
        let tightened = active
            .work
            .tighten_dimension_policy(keys.work, DimensionPolicy::counter(MAX_WORK, initial, 0));
        assert_eq!(tightened, Err(expected.clone()));
        let completed = active.complete_action();
        assert_eq!(completed, Err(expected.clone()));
        assert!(active.pending.is_none());
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
