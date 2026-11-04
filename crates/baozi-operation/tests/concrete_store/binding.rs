#![allow(
    clippy::result_large_err,
    reason = "tests preserve the actual inline startup and invocation errors"
)]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
use baozi_operation::__retained_store_integration::activation::{
    CanonicalBoundBodyError, CanonicalInvocationHost, CanonicalInvocationOutcome,
    CanonicalOperationPublication,
};
use baozi_operation::__retained_store_integration::authority::{
    PreparedStoreEvidenceOnlyOutcome, RetainedStorage,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, ProgressDecision, ProgressEvent,
    StoreTerminalCause,
};

use super::*;

fn binding_plans() -> (StoreLedgerPlan, RetainedStoreStartPlan, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_work = unbounded_failure_terminal_work(keys);
    let storage_work_limit = failure_work.ordinary_attempted_work_limit() - 16;
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
            (keys.other_dimension, DimensionPolicy::counter(16, 16, 0)),
            (
                keys.failure_terminal_dimension,
                failure_work.requirement().dimension_policy(),
            ),
        ],
    )
    .unwrap();
    let ledger = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
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
    (ledger, start, keys)
}

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

#[test]
fn real_prework_and_live_scratch_survive_attachment_and_the_next_physical_child() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let progress_calls = Arc::new(AtomicUsize::new(0));
    let observed_progress = Arc::clone(&progress_calls);
    let control = OperationControl::default()
        .with_cancellation(signal.clone())
        .with_progress_observer(move |event: ProgressEvent<TestProgress, TestUnit>| {
            assert_eq!(event.phase(), TestProgress::Build);
            assert_eq!(event.unit(), TestUnit::Items);
            assert_eq!(event.completed(), 1);
            observed_progress.fetch_add(1, Ordering::SeqCst);
            ProgressDecision::Continue
        });
    let mut invocation_root = TestInvocationRoot::new();
    let (ledger, start, keys) = binding_plans();
    let mut bound =
        TestOperation::bind(&mut invocation_root, control, ledger, &account, start).unwrap();
    let before_prework = account.snapshot();
    let mut output = [0; 4];
    let (scratch, before_attachment) = bound
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let scratch = context.allocate_bytes(keys.phase, keys.dimension, 5)?;
            let capacity = scratch.as_ref().capacity() as u64;
            assert_eq!(
                physical_delta(before_prework, account.snapshot()),
                (capacity, 1)
            );
            context.report_progress(TestProgress::Build, TestUnit::Items, 1, Some(1))?;
            let layout = context.bind_group([keys.other_dimension])?;
            let before_actions = signal.polls.load(Ordering::SeqCst);
            context.run_action(layout, [1], 128, || output[0] = 17)?;
            context.run_action(layout, [1], 128, || output[1] = 29)?;
            assert_eq!(signal.polls.load(Ordering::SeqCst), before_actions);
            context.run_action(layout, [1], 1, || output[2] = 41)?;
            assert_eq!(signal.polls.load(Ordering::SeqCst), before_actions + 1);
            assert_eq!(context.work_counters(keys.other_dimension)?, (3, 3, 0));
            assert_eq!(context.work_counters(keys.dimension)?, (2, 2, 0));
            Ok((scratch, context.dimension_state(keys.dimension)?))
        })
        .unwrap();
    let capacity = scratch.as_ref().capacity() as u64;
    let before_startup = account.snapshot();
    assert_eq!(output, [17, 29, 41, 0]);
    assert_eq!(progress_calls.load(Ordering::SeqCst), 1);
    assert_eq!(before_attachment.live_bytes(), capacity);

    let mut operation = bound.attach().unwrap();
    let after_startup = account.snapshot();
    let attached = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    assert_eq!(attached.attempted(), before_attachment.attempted() + 8);
    assert_eq!(attached.consumed(), before_attachment.consumed() + 8);
    assert!(attached.live_bytes() > before_attachment.live_bytes());
    assert!(after_startup.live_retained_bytes() > before_startup.live_retained_bytes());
    assert!(after_startup.live_retained_allocations() > before_startup.live_retained_allocations());
    assert_eq!(progress_calls.load(Ordering::SeqCst), 1);

    let polls_before_child = signal.polls.load(Ordering::SeqCst);
    operation
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            assert_eq!(signal.polls.load(Ordering::SeqCst), polls_before_child + 1);
            assert_eq!(context.work_counters(keys.other_dimension)?, (3, 3, 0));
            assert_eq!(context.work_counters(keys.dimension)?, (10, 10, 0));
            assert_eq!(context.dimension_state(keys.dimension)?, attached);
            let layout = context.bind_group([keys.other_dimension])?;
            context.parser_child::<_, StoreInvocationError, 1>(
                layout,
                [2],
                [Some(2)],
                |child| {
                    child.run_prefix([2], 7, || {
                        assert_eq!(scratch.as_ref().capacity() as u64, capacity);
                        output[3] = 53;
                        Ok(())
                    })
                },
            )?;
            assert_eq!(context.work_counters(keys.other_dimension)?, (5, 5, 0));
            assert_eq!(context.work_counters(keys.dimension)?, (10, 10, 0));
            Ok(())
        })
        .unwrap();
    assert_eq!(signal.polls.load(Ordering::SeqCst), polls_before_child + 2);
    assert_eq!(output, [17, 29, 41, 53]);
    assert_eq!(account.snapshot(), after_startup);
    drop(scratch);
    assert_eq!(
        physical_delta(account.snapshot(), after_startup),
        (capacity, 1)
    );
    assert_eq!(
        operation
            .producer_host()
            .dimension_state(keys.dimension)
            .unwrap()
            .live_bytes(),
        attached.live_bytes() - capacity
    );
    let prepared = operation
        .prepare_evidence_only_success(final_progress())
        .unwrap();
    let PreparedStoreEvidenceOnlyOutcome::Success(prepared) = prepared else {
        panic!("completed prework and released scratch must allow publication");
    };
    let evidence = prepared.handoff().ledger_evidence;
    let work = evidence.dimension(keys.other_dimension).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (5, 5));
    drop(evidence);
    drop(invocation_root);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn bound_action_regions_keep_their_actual_work_and_control_debt_through_attachment() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let (ledger, start, keys) = binding_plans();
    let mut root = TestInvocationRoot::new();
    let mut bound = TestOperation::bind(
        &mut root,
        OperationControl::default().with_cancellation(signal.clone()),
        ledger,
        &account,
        start,
    )
    .unwrap();
    let before = account.snapshot();
    let mut output = [0; 4];
    bound
        .with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |actions| {
            for (index, cost) in [128, 128, 1].into_iter().enumerate() {
                actions.begin_action(cost)?;
                output[index] = index + 1;
                actions.complete_action()?;
            }
            assert_eq!(actions.work_counters(keys.other_dimension)?, (3, 3, 0));
            Ok(())
        })
        .unwrap();
    assert_eq!(signal.polls.load(Ordering::SeqCst), 3);
    assert_eq!(account.snapshot(), before);
    let mut operation = bound.attach().unwrap();
    assert_eq!(signal.polls.load(Ordering::SeqCst), 3);
    operation
        .with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |actions| {
            assert_eq!(actions.work_counters(keys.other_dimension)?, (3, 3, 0));
            actions.begin_action(1)?;
            output[3] = 4;
            actions.complete_action()?;
            Ok(())
        })
        .unwrap();
    assert_eq!(signal.polls.load(Ordering::SeqCst), 5);
    assert_eq!(output, [1, 2, 3, 4]);
    let PreparedStoreEvidenceOnlyOutcome::Success(prepared) = operation
        .prepare_evidence_only_success(final_progress())
        .unwrap()
    else {
        panic!("bound and attached action regions completed their actual work");
    };
    let evidence = prepared.handoff().ledger_evidence;
    let units = evidence.dimension(keys.other_dimension).unwrap();
    assert_eq!((units.attempted(), units.consumed()), (4, 4));
    drop(evidence);
    drop(root);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn ignored_bound_action_error_prevents_following_work_and_attachment() {
    let factory = factory(1);
    let account = account(&factory);
    let (ledger, start, keys) = binding_plans();
    let mut root = TestInvocationRoot::new();
    let mut bound = TestOperation::bind(
        &mut root,
        OperationControl::default(),
        ledger,
        &account,
        start,
    )
    .unwrap();
    let before = account.snapshot();
    assert!(matches!(
        bound.with_action_execution::<(), StoreRuntimeError>(keys.other_dimension, |actions| {
            actions.begin_action(1)?;
            actions.complete_action()?;
            Err(StoreRuntimeError::InvalidSourceSlot)
        },),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    let called = Cell::new(false);
    assert!(matches!(
        bound.with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |_| {
            called.set(true);
            Ok(())
        },),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    assert!(!called.get());
    assert!(matches!(
        bound.attach(),
        Err(RetainedStoreStartError::Runtime(
            StoreRuntimeError::OwnershipUnverified
        ))
    ));
    drop(root);
    assert_eq!(account.snapshot(), before);
}

#[derive(Debug)]
enum PreworkError {
    Runtime(StoreInvocationError),
    Semantic { code: u32, identity: Rc<()> },
}

impl From<StoreInvocationError> for PreworkError {
    fn from(cause: StoreInvocationError) -> Self {
        Self::Runtime(cause)
    }
}

#[test]
fn ignored_prework_parser_error_preserves_its_identity_and_prevents_attachment() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (ledger, start, keys) = binding_plans();
    let mut bound = TestOperation::bind(
        &mut invocation_root,
        OperationControl::default(),
        ledger,
        &account,
        start,
    )
    .unwrap();
    let physical_before = account.snapshot();
    let identity = Rc::new(());
    let mut output = [0; 3];
    let result = bound.with_exclusive_execution::<_, PreworkError>(|context| {
        let layout = context.bind_group([keys.other_dimension])?;
        let result = context.parser_child::<_, PreworkError, 1>(layout, [2], [None], |child| {
            child.run_prefix([1], 7, || {
                output[0] = 17;
                Ok::<_, PreworkError>(())
            })?;
            child.run_prefix([1], 11, || -> Result<(), PreworkError> {
                output[1] = 29;
                Err(PreworkError::Semantic {
                    code: 41,
                    identity: Rc::clone(&identity),
                })
            })
        });
        assert_eq!(context.work_counters(keys.other_dimension)?, (2, 1, 0));
        assert!(
            context
                .run_action(layout, [1], 1, || output[2] = 1)
                .is_err()
        );
        result
    });
    match result {
        Err(PreworkError::Semantic {
            code: 41,
            identity: returned,
        }) => {
            assert!(Rc::ptr_eq(&returned, &identity));
        }
        Err(PreworkError::Runtime(cause)) => panic!("cleanup replaced the callback error: {cause}"),
        other => panic!("unexpected prework result: {other:?}"),
    }
    let next_callback = Cell::new(false);
    assert!(matches!(
        bound.with_exclusive_execution::<_, StoreInvocationError>(|_| {
            next_callback.set(true);
            Ok(())
        }),
        Err(StoreInvocationError::Runtime(
            StoreRuntimeError::FallbackOnly
        ))
    ));
    assert!(matches!(
        bound.attach(),
        Err(RetainedStoreStartError::Runtime(
            StoreRuntimeError::OwnershipUnverified
        ))
    ));
    assert!(!next_callback.get());
    assert_eq!(output, [17, 29, 0]);
    assert_eq!(account.snapshot(), physical_before);
    drop(invocation_root);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn caught_prework_unwind_keeps_the_completed_prefix_and_prevents_attachment() {
    let factory = factory(1);
    let account = account(&factory);
    let mut root = TestInvocationRoot::new();
    let (ledger, start, keys) = binding_plans();
    let mut bound = TestOperation::bind(
        &mut root,
        OperationControl::default(),
        ledger,
        &account,
        start,
    )
    .unwrap();
    let before = account.snapshot();
    let mut written = [false; 3];
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bound.with_exclusive_execution::<_, StoreInvocationError>(|work| {
            let scratch = work.allocate_bytes(keys.phase, keys.dimension, 5)?;
            let layout = work.bind_group([keys.other_dimension])?;
            work.run_action(layout, [1], 1, || written[0] = true)?;
            work.run_action(layout, [1], 1, || {
                std::hint::black_box(&scratch);
                written[1] = true;
                panic!("bound prework callback");
            })
        })
    }));
    assert!(panic.is_err());
    assert_eq!(physical_delta(before, account.snapshot()), (0, 0));
    assert!(matches!(
        bound.with_exclusive_execution::<_, StoreInvocationError>(|_| {
            written[2] = true;
            Ok(())
        }),
        Err(StoreInvocationError::Runtime(
            StoreRuntimeError::FallbackOnly
        ))
    ));
    assert!(matches!(
        bound.attach(),
        Err(RetainedStoreStartError::Runtime(
            StoreRuntimeError::OwnershipUnverified
        ))
    ));
    assert_eq!(written, [true, true, false]);
    drop(root);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

struct BoundReturnedOwner {
    storage: Option<RetainedStorage<Vec<u8>>>,
    drops: Rc<Cell<usize>>,
}

impl Drop for BoundReturnedOwner {
    fn drop(&mut self) {
        drop(self.storage.take());
        self.drops.set(self.drops.get() + 1);
        panic!("rejected bound prework result drop");
    }
}

#[test]
fn rejected_prework_result_drop_unwind_preserves_cancellation_and_releases_storage_once() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let (ledger, start, keys) = binding_plans();
    let mut root = TestInvocationRoot::new();
    let mut bound = TestOperation::bind(
        &mut root,
        OperationControl::default().with_cancellation(signal.clone()),
        ledger,
        &account,
        start,
    )
    .unwrap();
    let before = account.snapshot();
    let drops = Rc::new(Cell::new(0));
    let returned = Cell::new(false);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let result = bound.with_exclusive_execution::<_, StoreInvocationError>(|work| {
            let storage = work.allocate_bytes(keys.phase, keys.dimension, 5)?;
            let layout = work.bind_group([keys.other_dimension])?;
            let owner = work.run_action(layout, [1], 1, || BoundReturnedOwner {
                storage: Some(storage),
                drops: Rc::clone(&drops),
            })?;
            signal.token.cancel();
            Ok(owner)
        });
        returned.set(true);
        drop(result);
    }));
    assert!(panic.is_err());
    assert!(
        !returned.get(),
        "the region must reject and drop its result before returning"
    );
    assert_eq!(drops.get(), 1);
    assert_eq!(physical_delta(before, account.snapshot()), (0, 0));
    let called = Cell::new(false);
    assert!(matches!(
        bound.with_exclusive_execution::<_, StoreInvocationError>(|_| {
            called.set(true);
            Ok(())
        }),
        Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        )))
    ));
    assert!(!called.get());
    assert!(matches!(
        bound.attach(),
        Err(RetainedStoreStartError::Runtime(
            StoreRuntimeError::Control(ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            })
        ))
    ));
    drop(root);
    assert_eq!(drops.get(), 1);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn ignored_prework_cancellation_keeps_control_priority_and_denies_following_callbacks() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let mut invocation_root = TestInvocationRoot::new();
    let (ledger, start, keys) = binding_plans();
    let mut bound = TestOperation::bind(
        &mut invocation_root,
        OperationControl::default().with_cancellation(signal.clone()),
        ledger,
        &account,
        start,
    )
    .unwrap();
    let physical_before = account.snapshot();
    let mut output = [0; 3];
    let result = bound.with_exclusive_execution::<_, StoreInvocationError>(|context| {
        let layout = context.bind_group([keys.other_dimension])?;
        context.run_action(layout, [1], 256, || {
            output[0] = 17;
            signal.token.cancel();
        })?;
        assert!(matches!(
            context.run_action(layout, [1], 1, || output[1] = 29),
            Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
                ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal
                }
            )))
        ));
        assert_eq!(context.work_counters(keys.other_dimension)?, (1, 1, 0));
        assert!(
            context
                .run_action(layout, [1], 1, || output[2] = 41)
                .is_err()
        );
        Ok(())
    });
    assert!(matches!(
        result,
        Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        )))
    ));
    assert!(matches!(
        bound.attach(),
        Err(RetainedStoreStartError::Runtime(
            StoreRuntimeError::Control(ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            })
        ))
    ));
    assert_eq!(output, [17, 0, 0]);
    assert_eq!(account.snapshot(), physical_before);
    drop(invocation_root);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn ordinary_prework_cannot_bind_or_spend_the_reserved_failure_pool() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (ledger, start, keys) = binding_plans();
    let mut bound = TestOperation::bind(
        &mut invocation_root,
        OperationControl::default(),
        ledger,
        &account,
        start,
    )
    .unwrap();
    let physical_before = account.snapshot();
    let callback = Cell::new(false);
    let result = bound.with_exclusive_execution::<_, StoreInvocationError>(|context| {
        let ordinary = context.bind_group([keys.other_dimension])?;
        assert_eq!(
            context.work_counters(keys.failure_terminal_dimension)?,
            (0, 0, 0)
        );
        assert!(matches!(
            context.bind_group([keys.failure_terminal_dimension]),
            Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                LedgerCause::UnknownDimension { dimension }
            ))) if dimension == keys.failure_terminal_dimension
        ));
        assert!(
            context
                .run_action(ordinary, [1], 1, || callback.set(true))
                .is_err()
        );
        assert_eq!(
            context.work_counters(keys.failure_terminal_dimension)?,
            (0, 0, 0)
        );
        assert_eq!(context.work_counters(keys.other_dimension)?, (0, 0, 0));
        assert_eq!(context.work_counters(keys.dimension)?, (0, 0, 0));
        Ok(())
    });
    assert!(result.is_err());
    assert!(bound.attach().is_err());
    assert!(!callback.get());
    assert_eq!(account.snapshot(), physical_before);
    drop(invocation_root);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

struct DropProbe {
    drops: Arc<AtomicUsize>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "the adversarial client forgets the bound permission while its actual root stays alive"
)]
fn forgotten_bound_view_keeps_control_in_the_root_and_escaping_storage_charged() {
    let factory = factory(1);
    let account = account(&factory);
    let control_drops = Arc::new(AtomicUsize::new(0));
    let probe = DropProbe {
        drops: Arc::clone(&control_drops),
    };
    let control = OperationControl::default().with_progress_observer(
        move |_: ProgressEvent<TestProgress, TestUnit>| {
            std::hint::black_box(&probe);
            ProgressDecision::Continue
        },
    );
    let mut invocation_root = TestInvocationRoot::new();
    let (ledger, start, keys) = binding_plans();
    let mut bound =
        TestOperation::bind(&mut invocation_root, control, ledger, &account, start).unwrap();
    let physical_before = account.snapshot();
    let scratch = bound
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            context.allocate_bytes(keys.phase, keys.dimension, 5)
        })
        .unwrap();
    let capacity = scratch.as_ref().capacity() as u64;
    let physical_live = account.snapshot();
    assert_eq!(
        physical_delta(physical_before, physical_live),
        (capacity, 1)
    );
    assert_eq!(control_drops.load(Ordering::SeqCst), 0);
    std::mem::forget(bound);
    assert_eq!(control_drops.load(Ordering::SeqCst), 0);
    assert_eq!(account.snapshot(), physical_live);
    drop(invocation_root);
    assert_eq!(control_drops.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot(), physical_live);
    assert_eq!(scratch.as_ref().capacity() as u64, capacity);
    drop(scratch);
    let released = account.snapshot();
    assert_eq!(
        released.live_retained_bytes(),
        physical_before.live_retained_bytes()
    );
    assert_eq!(
        released.live_retained_allocations(),
        physical_before.live_retained_allocations()
    );
    assert_eq!(
        released.peak_retained_bytes(),
        physical_live.peak_retained_bytes()
    );
    assert_eq!(
        released.peak_retained_allocations(),
        physical_live.peak_retained_allocations()
    );
    assert_eq!(
        released.total_admitted_retained_bytes(),
        physical_live.total_admitted_retained_bytes()
    );
    assert_eq!(
        released.total_admitted_retained_allocations(),
        physical_live.total_admitted_retained_allocations()
    );
    assert!(!released.poisoned());
}

type BoundHost = CanonicalInvocationHost<TestOwner, TestProgress, TestUnit, (), ()>;

#[test]
fn unattached_success_finishes_after_captures_with_original_work_and_no_retained_startup() {
    let factory = factory(1);
    let account = account(&factory);
    let before = account.snapshot();
    let drops = Arc::new(AtomicUsize::new(0));
    let capture = DropProbe {
        drops: Arc::clone(&drops),
    };
    let observed = Arc::clone(&drops);
    let control = OperationControl::default().with_progress_observer(move |_| {
        assert_eq!(observed.load(Ordering::SeqCst), 1);
        ProgressDecision::Continue
    });
    let (ledger, start, keys) = binding_plans();
    let mut host = BoundHost::new();
    let finish = host
        .run_bound_body(
            control,
            ledger,
            &account,
            start,
            move |mut parts, disposition| {
                std::hint::black_box(&capture);
                parts
                    .startup
                    .with_action_execution::<_, StoreRuntimeError>(
                        keys.other_dimension,
                        |actions| {
                            for _ in 0..3 {
                                actions.begin_action(1)?;
                                actions.complete_action()?;
                            }
                            Ok(())
                        },
                    )
                    .unwrap();
                parts.startup.request_unchanged(disposition).unwrap();
            },
        )
        .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot(), before);
    let finish = match finish.into_attached() {
        Ok(_) => panic!("no retained graph was attached"),
        Err(finish) => finish,
    };
    let evidence = finish.finish_unattached(final_progress()).unwrap();
    assert_eq!(
        evidence.header().terminal_cause(),
        StoreTerminalCause::Success
    );
    let units = evidence.dimension(keys.other_dimension).unwrap();
    assert_eq!((units.attempted(), units.consumed()), (3, 3));
    let storage_work = evidence.dimension(keys.dimension).unwrap();
    assert_eq!((storage_work.attempted(), storage_work.consumed()), (0, 0));
    assert_eq!(
        evidence.dimension_by_semantic(keys.other_dimension.semantic_id()),
        Some(units)
    );
    assert_eq!(evidence.header().dimension_count(), 3);
    assert_eq!(evidence.dimension_at(0), Some(storage_work));
    assert!(evidence.dimension_at(3).is_none());
    assert_eq!(evidence.header().phase_count(), 1);
    assert_eq!(
        evidence.phase_at(0),
        Some(evidence.phase(keys.phase).unwrap())
    );
    assert!(evidence.phase_at(1).is_none());
    assert_eq!(evidence.header().current_live_bytes(), 0);
    assert_eq!(evidence.header().active_reservations(), 0);
    let (_, _, foreign) = binding_plans();
    assert!(matches!(
        evidence.dimension(foreign.dimension),
        Err(StoreRuntimeError::ForeignDimensionDomain { .. })
    ));
    assert_eq!(account.snapshot(), before);
    drop(evidence);
    drop(host);
    assert_eq!(account.snapshot(), before);
}

#[test]
fn unattached_handled_error_keeps_requested_cause_while_ignored_error_cannot_succeed() {
    for handled in [false, true] {
        let factory = factory(1);
        let account = account(&factory);
        let before = account.snapshot();
        let (ledger, start, keys) = binding_plans();
        let mut host = BoundHost::new();
        let finish = host
            .run_bound_body(
                OperationControl::default(),
                ledger,
                &account,
                start,
                |mut parts, disposition| {
                    assert!(matches!(
                        parts
                            .startup
                            .with_action_execution::<(), StoreRuntimeError>(
                                keys.other_dimension,
                                |actions| {
                                    actions.begin_action(1)?;
                                    actions.complete_action()?;
                                    Err(StoreRuntimeError::InvalidSourceSlot)
                                },
                            ),
                        Err(StoreRuntimeError::InvalidSourceSlot)
                    ));
                    if handled {
                        parts.startup.request_failure(disposition, 41).unwrap();
                    } else {
                        parts.startup.request_unchanged(disposition).unwrap();
                    }
                },
            )
            .unwrap();
        let evidence = finish.finish_unattached(final_progress()).unwrap();
        let StoreTerminalCause::Failure(cause) = evidence.header().terminal_cause() else {
            panic!("an actual outer error cannot publish direct success");
        };
        if handled {
            assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
            assert_eq!(cause.code(), 41);
        } else {
            assert_eq!(cause.class(), StorePreparationFailureClass::ProductGraph);
        }
        let units = evidence.dimension(keys.other_dimension).unwrap();
        assert_eq!((units.attempted(), units.consumed()), (1, 1));
        assert_eq!(account.snapshot(), before);
        drop(evidence);
        drop(host);
        assert_eq!(account.snapshot(), before);
    }
}

#[test]
fn unattached_capture_panic_prevents_finish_and_terminal_progress() {
    let factory = factory(1);
    let account = account(&factory);
    let before = account.snapshot();
    let drops = Arc::new(AtomicUsize::new(0));
    let capture = PanickingCapture(Arc::clone(&drops));
    let progress = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&progress);
    let control = OperationControl::default().with_progress_observer(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        ProgressDecision::Continue
    });
    let (ledger, start, _) = binding_plans();
    let mut host = BoundHost::new();
    let returned = Cell::new(false);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let result = host.run_bound_body(
            control,
            ledger,
            &account,
            start,
            move |parts, disposition| {
                std::hint::black_box(&capture);
                parts.startup.request_unchanged(disposition).unwrap();
            },
        );
        returned.set(true);
        drop(result);
    }));
    assert!(panic.is_err());
    assert!(!returned.get());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(progress.load(Ordering::SeqCst), 0);
    assert!(
        host.run_body(|_, _| panic!("an interrupted body cannot reenter"))
            .is_err()
    );
    drop(host);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot(), before);
}

struct DeniedEntryCapture {
    drops: Rc<Cell<usize>>,
    panic: bool,
}

impl Drop for DeniedEntryCapture {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
        if self.panic {
            panic!("capture drop after bound entry denial");
        }
    }
}

#[test]
fn denied_bound_entry_disposes_the_actual_body_captures_without_entering_client_work() {
    for panic in [false, true] {
        let factory = factory(1);
        let account = account(&factory);
        let before = account.snapshot();
        let drops = Rc::new(Cell::new(0));
        let capture = DeniedEntryCapture {
            drops: Rc::clone(&drops),
            panic,
        };
        let called = Rc::new(Cell::new(false));
        let called_in_body = Rc::clone(&called);
        let signal = CountedCancellation::default();
        let control = OperationControl::default().with_cancellation(signal.clone());
        let (ledger, _, _) = binding_plans();
        let (_, foreign_start, _) = binding_plans();
        let mut host = BoundHost::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            matches!(
                host.run_bound_body(control, ledger, &account, foreign_start, move |_, _| {
                    std::hint::black_box(&capture);
                    called_in_body.set(true);
                }),
                Err(CanonicalBoundBodyError::Start(
                    RetainedStoreStartError::Runtime(StoreRuntimeError::ForeignPhaseDomain { .. })
                ))
            )
        }));
        if panic {
            assert!(result.is_err());
        } else {
            assert!(matches!(result, Ok(true)));
        }
        assert!(!called.get());
        assert_eq!(drops.get(), 1);
        assert_eq!(signal.polls.load(Ordering::SeqCst), 0);
        assert!(
            host.run_body(|_, _| panic!("a denied bound body cannot reenter"))
                .is_err()
        );
        drop(host);
        assert_eq!(drops.get(), 1);
        assert_eq!(account.snapshot(), before);
    }
}

#[test]
fn canonical_bound_entry_drops_the_actual_callback_before_terminal_progress() {
    let factory = factory(1);
    let account = account(&factory);
    let captures = Arc::new(AtomicUsize::new(0));
    let capture = DropProbe {
        drops: Arc::clone(&captures),
    };
    let observed_captures = Arc::clone(&captures);
    let control = OperationControl::default().with_progress_observer(move |_| {
        assert_eq!(observed_captures.load(Ordering::SeqCst), 1);
        ProgressDecision::Continue
    });
    let (ledger, start, keys) = binding_plans();
    let mut host = BoundHost::new();
    let finish = host
        .run_bound_body(
            control,
            ledger,
            &account,
            start,
            move |mut parts, disposition| {
                std::hint::black_box(&capture);
                parts
                    .startup
                    .with_exclusive_execution::<_, StoreInvocationError>(|work| {
                        let units = work.bind_group([keys.other_dimension])?;
                        work.run_action(units, [2], 2, || ())
                    })
                    .unwrap();
                parts
                    .startup
                    .attach()
                    .unwrap()
                    .request_unchanged(disposition)
                    .unwrap();
            },
        )
        .unwrap();
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    let CanonicalInvocationOutcome::Operation(operation) = finish
        .into_attached()
        .unwrap_or_else(|_| panic!("the body attached its original operation"))
        .resume()
        .outcome
    else {
        panic!("the bound body requested unchanged publication");
    };
    let CanonicalOperationPublication::Unchanged(PreparedStoreEvidenceOnlyOutcome::Success(
        prepared,
    )) = operation.finish(final_progress()).unwrap()
    else {
        panic!("successful bound prework must permit unchanged publication");
    };
    let evidence = prepared.handoff().ledger_evidence;
    let units = evidence.dimension(keys.other_dimension).unwrap();
    assert_eq!((units.attempted(), units.consumed()), (2, 2));
    drop(evidence);
    drop(host);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn canonical_bound_action_prework_uses_the_same_terminal_and_capture_order() {
    let factory = factory(1);
    let account = account(&factory);
    let drops = Arc::new(AtomicUsize::new(0));
    let capture = DropProbe {
        drops: Arc::clone(&drops),
    };
    let observed = Arc::clone(&drops);
    let control = OperationControl::default().with_progress_observer(move |_| {
        assert_eq!(observed.load(Ordering::SeqCst), 1);
        ProgressDecision::Continue
    });
    let (ledger, start, keys) = binding_plans();
    let mut host = BoundHost::new();
    let finish = host
        .run_bound_body(
            control,
            ledger,
            &account,
            start,
            move |mut parts, disposition| {
                std::hint::black_box(&capture);
                parts
                    .startup
                    .with_action_execution::<_, StoreRuntimeError>(
                        keys.other_dimension,
                        |actions| {
                            actions.begin_action(1)?;
                            actions.complete_action()?;
                            Ok(())
                        },
                    )
                    .unwrap();
                parts
                    .startup
                    .attach()
                    .unwrap()
                    .request_unchanged(disposition)
                    .unwrap();
            },
        )
        .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let CanonicalInvocationOutcome::Operation(operation) = finish
        .into_attached()
        .unwrap_or_else(|_| panic!("the body attached its original operation"))
        .resume()
        .outcome
    else {
        panic!("bound actions requested the original operation terminal");
    };
    let CanonicalOperationPublication::Unchanged(PreparedStoreEvidenceOnlyOutcome::Success(
        prepared,
    )) = operation.finish(final_progress()).unwrap()
    else {
        panic!("completed bound actions must permit successful publication");
    };
    let evidence = prepared.handoff().ledger_evidence;
    let units = evidence.dimension(keys.other_dimension).unwrap();
    assert_eq!((units.attempted(), units.consumed()), (1, 1));
    drop(evidence);
    drop(host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

struct PanickingCapture(Arc<AtomicUsize>);

impl Drop for PanickingCapture {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("bound callback capture drop");
    }
}

#[test]
fn canonical_bound_entry_contains_capture_drop_before_issuing_finish() {
    let factory = factory(1);
    let account = account(&factory);
    let drops = Arc::new(AtomicUsize::new(0));
    let capture = PanickingCapture(Arc::clone(&drops));
    let progress = Arc::new(AtomicUsize::new(0));
    let progress_observation = Arc::clone(&progress);
    let control = OperationControl::default().with_progress_observer(move |_| {
        progress_observation.fetch_add(1, Ordering::SeqCst);
        ProgressDecision::Continue
    });
    let (ledger, start, _) = binding_plans();
    let mut host = BoundHost::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = host.run_bound_body(
            control,
            ledger,
            &account,
            start,
            move |parts, disposition| {
                std::hint::black_box(&capture);
                parts
                    .startup
                    .attach()
                    .unwrap()
                    .request_unchanged(disposition)
                    .unwrap();
            },
        );
    }));
    assert!(result.is_err());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(progress.load(Ordering::SeqCst), 0);
    assert!(
        host.run_body(|_, _| panic!("a failed canonical body cannot reenter"))
            .is_err()
    );
    drop(host);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "a forgotten short view must leave the actual bound frame in its canonical entry"
)]
fn canonical_bound_entry_retains_authority_when_the_client_forgets_its_startup_view() {
    let factory = factory(1);
    let account = account(&factory);
    let (ledger, start, keys) = binding_plans();
    let mut host = BoundHost::new();
    let mut escaped_scratch = None;
    let result = host.run_bound_body(
        OperationControl::default(),
        ledger,
        &account,
        start,
        |mut parts, _| {
            escaped_scratch = Some(
                parts
                    .startup
                    .with_exclusive_execution::<_, StoreInvocationError>(|work| {
                        work.allocate_bytes(keys.phase, keys.dimension, 5)
                    })
                    .unwrap(),
            );
            std::mem::forget(parts.startup);
        },
    );
    assert!(matches!(result, Err(CanonicalBoundBodyError::Body(_))));
    drop(result);
    let scratch = escaped_scratch.take().unwrap();
    let bytes = scratch.as_ref().capacity() as u64;
    assert_eq!(account.snapshot().live_retained_bytes(), bytes);
    assert_eq!(account.snapshot().live_retained_allocations(), 1);
    drop(host);
    assert_eq!(account.snapshot().live_retained_bytes(), bytes);
    drop(scratch);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}
