//! Real operation/storage traces for the isolated exclusive-owner proof.
use super::*;
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use baozi_operation::__retained_store_integration::activation::StoreInvocationError;
use baozi_operation::__retained_store_integration::authority::RetainedStorage;

#[derive(Debug)]
enum ParsePieceError {
    Runtime(StoreInvocationError),
    Semantic { code: u32, identity: Rc<()> },
}

impl From<StoreInvocationError> for ParsePieceError {
    fn from(cause: StoreInvocationError) -> Self {
        Self::Runtime(cause)
    }
}

#[derive(Debug)]
enum ActionBoundaryError {
    Runtime(StoreRuntimeError),
    Semantic { code: u32, identity: Rc<()> },
}

impl From<StoreRuntimeError> for ActionBoundaryError {
    fn from(cause: StoreRuntimeError) -> Self {
        Self::Runtime(cause)
    }
}

#[test]
fn ignored_action_region_semantic_error_keeps_its_owner_and_prevents_reuse() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let mut root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut root,
        &account,
        OperationControl::default().with_cancellation(signal.clone()),
        DimensionPolicy::counter(8, 8, 0),
    );
    signal.polls.store(0, Ordering::SeqCst);
    let identity = Rc::new(());
    let physical = Cell::new(false);
    let result = operation.with_action_execution::<(), ActionBoundaryError>(
        keys.other_dimension,
        |actions| {
            actions.begin_action(1)?;
            physical.set(true);
            actions.complete_action()?;
            Err(ActionBoundaryError::Semantic {
                code: 41,
                identity: Rc::clone(&identity),
            })
        },
    );
    match result {
        Err(ActionBoundaryError::Semantic {
            code: 41,
            identity: returned,
        }) => {
            assert!(Rc::ptr_eq(&returned, &identity));
        }
        Err(ActionBoundaryError::Runtime(cause)) => {
            panic!("the action error was replaced: {cause}")
        }
        other => panic!("unexpected action result: {other:?}"),
    }
    assert!(physical.get());
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let next_called = Cell::new(false);
    assert!(matches!(
        operation.with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |_| {
            next_called.set(true);
            Ok(())
        }),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    assert!(!next_called.get());
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
    assert_work(&evidence, keys.other_dimension, 1, 1);
    drop(evidence);
    drop(root);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn caught_action_region_unwind_before_first_action_prevents_reuse_and_success() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let mut root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut root,
        &account,
        OperationControl::default().with_cancellation(signal.clone()),
        DimensionPolicy::counter(8, 8, 0),
    );
    signal.polls.store(0, Ordering::SeqCst);
    let before = account.snapshot();
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _: Result<(), StoreRuntimeError> = operation
            .with_action_execution(keys.other_dimension, |_| {
                panic!("action region before its first numeric action")
            });
    }));
    assert!(panic.is_err());
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot(), before);
    let next_called = Cell::new(false);
    assert!(matches!(
        operation.with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |_| {
            next_called.set(true);
            Ok(())
        }),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    assert!(!next_called.get());
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
    assert_work(&evidence, keys.other_dimension, 0, 0);
    drop(evidence);
    drop(root);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

struct ReturnedActionOwner {
    values: Option<Vec<u8>>,
    reservation: Option<
        baozi_operation::__retained_store_integration::authority::RetainedStoreReservation<
            TestOwner,
        >,
    >,
    drops: Rc<Cell<usize>>,
}

impl Drop for ReturnedActionOwner {
    fn drop(&mut self) {
        self.reservation
            .take()
            .unwrap()
            .release_storage(self.values.take());
        self.drops.set(self.drops.get() + 1);
        panic!("rejected action result destructor");
    }
}

#[test]
fn rejected_action_result_drop_unwind_keeps_original_control_and_disposes_storage_once() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let mut root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut root,
        &account,
        OperationControl::default().with_cancellation(signal.clone()),
        DimensionPolicy::counter(8, 8, 0),
    );
    let before = account.snapshot();
    let drops = Rc::new(Cell::new(0));
    let returned = Cell::new(false);
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let result = operation.with_action_execution::<_, StoreRuntimeError>(
            keys.other_dimension,
            |actions| {
                let mut reservation = actions.reserve_retained(keys.phase, keys.dimension, 5, 1)?;
                let mut values = Vec::<u8>::new();
                values.try_reserve_exact(5).unwrap();
                actions.resize(&mut reservation, values.capacity() as u64)?;
                actions.begin_action(1)?;
                values.push(17);
                actions.complete_action()?;
                signal.token.cancel();
                Ok(ReturnedActionOwner {
                    values: Some(values),
                    reservation: Some(reservation),
                    drops: Rc::clone(&drops),
                })
            },
        );
        returned.set(true);
        drop(result);
    }));
    assert!(panic.is_err());
    assert!(!returned.get());
    assert_eq!(drops.get(), 1);
    assert_eq!(physical_delta(before, account.snapshot()), (0, 0));
    let next_called = Cell::new(false);
    assert!(matches!(
        operation.with_action_execution::<_, StoreRuntimeError>(keys.other_dimension, |_| {
            next_called.set(true);
            Ok(())
        },),
        Err(StoreRuntimeError::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal
        }))
    ));
    assert!(!next_called.get());
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Control);
    assert_work(&evidence, keys.other_dimension, 1, 1);
    drop(evidence);
    drop(root);
    assert_eq!(drops.get(), 1);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn normal_parser_error_retains_only_verified_piece_and_the_original_semantic_owner() {
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
    signal.polls.store(0, Ordering::SeqCst);
    let semantic_owner = Rc::new(());
    let mut physical = [0; 3];
    let result = operation.with_exclusive_execution::<_, ParsePieceError>(|context| {
        let layout = context.bind_group([keys.other_dimension])?;
        let result = context.parser_child::<_, ParsePieceError, 1>(
            layout,
            [2],
            [None],
            |child| -> Result<(), ParsePieceError> {
                child.run_prefix([1], 7, || {
                    physical[0] = 17;
                    Ok::<_, ParsePieceError>(())
                })?;
                child.run_prefix([1], 11, || -> Result<(), ParsePieceError> {
                    physical[1] = 29;
                    Err(ParsePieceError::Semantic {
                        code: 41,
                        identity: Rc::clone(&semantic_owner),
                    })
                })?;
                child.run_prefix([0], 1, || {
                    physical[2] = 1;
                    Ok::<_, ParsePieceError>(())
                })?;
                Ok(())
            },
        );
        assert_eq!(context.work_counters(keys.other_dimension)?, (2, 1, 0));
        assert!(
            context
                .run_action(layout, [1], 1, || physical[2] = 1)
                .is_err()
        );
        result
    });
    match result {
        Err(ParsePieceError::Semantic { code: 41, identity }) => {
            assert!(Rc::ptr_eq(&identity, &semantic_owner))
        }
        Err(ParsePieceError::Runtime(cause)) => {
            panic!("runtime cleanup replaced semantic error: {cause}")
        }
        other => panic!("unexpected parser result: {other:?}"),
    }
    assert_eq!(physical, [17, 29, 0]);
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let evidence = operation
        .prepare_minimal_fallback(41)
        .unwrap()
        .handoff()
        .ledger_evidence;
    assert_eq!(
        store_failure_cause(&evidence).class(),
        StorePreparationFailureClass::Requested
    );
    assert_work(&evidence, keys.other_dimension, 2, 1);
}

/// Controlled observation becomes cancelled at the first post-allocation checkpoint. Setup
/// leaves the threshold disabled; no allocator hook or substitute accounting path is involved.
#[derive(Clone)]
struct BoundaryCancellation {
    polls: Arc<AtomicUsize>,
    cancel_at: Arc<AtomicUsize>,
}

impl BoundaryCancellation {
    fn new() -> Self {
        Self {
            polls: Arc::new(AtomicUsize::new(0)),
            cancel_at: Arc::new(AtomicUsize::new(usize::MAX)),
        }
    }
}

impl CancellationSignal for BoundaryCancellation {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::SeqCst) + 1 >= self.cancel_at.load(Ordering::SeqCst)
    }
}

#[test]
fn postallocation_control_rejection_drops_real_backing_before_the_next_child_piece() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = BoundaryCancellation::new();
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default().with_cancellation(signal.clone()),
        DimensionPolicy::counter(8, 8, 0),
    );
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    let physical = account.snapshot();
    signal.polls.store(0, Ordering::SeqCst);
    signal.cancel_at.store(3, Ordering::SeqCst);
    let next_piece = Cell::new(0);
    let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
        let layout = context.bind_group([keys.other_dimension])?;
        context.parser_parent::<_, StoreInvocationError, 1>(layout, [1], [None], |parent| {
            parent.child::<_, StoreInvocationError, 1>(layout, [1], [None], |child| {
                let _storage = child.allocate_bytes(keys.phase, keys.dimension, 5)?;
                child.run_prefix([1], 2, || {
                    next_piece.set(1);
                    Ok(())
                })
            })?;
            parent.run_prefix([1], 3, || Ok(()))
        })
    });
    assert!(matches!(
        result,
        Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        )))
    ));
    assert_eq!(signal.polls.load(Ordering::SeqCst), 3);
    assert_eq!(next_piece.get(), 0);
    assert!(matches!(
        operation.with_action_execution::<(), StoreRuntimeError>(keys.other_dimension, |_| panic!(
            "a cancelled invocation cannot reenter its action host"
        )),
        Err(StoreRuntimeError::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal
        }))
    ));
    let reentered = Cell::new(0);
    let reentry = operation.with_exclusive_execution::<(), StoreInvocationError>(|_| {
        reentered.set(1);
        Ok(())
    });
    assert!(matches!(
        reentry,
        Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        )))
    ));
    assert_eq!(reentered.get(), 0);
    assert_eq!(signal.polls.load(Ordering::SeqCst), 3);
    assert_eq!(physical_delta(physical, account.snapshot()), (0, 0));
    let after = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    assert_eq!(
        (after.attempted(), after.consumed(), after.live_bytes()),
        (
            baseline.attempted() + 2,
            baseline.consumed() + 2,
            baseline.live_bytes()
        )
    );
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Control);
    assert_work(
        &evidence,
        keys.dimension,
        baseline.attempted() + 2,
        baseline.consumed() + 2,
    );
    assert_work(&evidence, keys.other_dimension, 2, 0);
    assert!(!account.snapshot().poisoned());
}

struct ReturnedStorageOwner {
    storage: Option<RetainedStorage<Vec<u8>>>,
    drops: Rc<Cell<usize>>,
    panic: bool,
}

impl Drop for ReturnedStorageOwner {
    fn drop(&mut self) {
        drop(self.storage.take());
        self.drops.set(self.drops.get() + 1);
        if self.panic {
            panic!("returned storage owner destructor");
        }
    }
}

#[test]
fn final_poll_rejection_drops_returned_real_owner_and_preserves_control_through_drop_unwind() {
    for panic in [false, true] {
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
        let baseline = operation
            .producer_host()
            .dimension_state(keys.dimension)
            .unwrap();
        let physical = account.snapshot();
        let drops = Rc::new(Cell::new(0));
        signal.polls.store(0, Ordering::SeqCst);
        let result = catch_unwind(AssertUnwindSafe(|| {
            operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let storage = context.allocate_bytes(keys.phase, keys.dimension, 5)?;
                let layout = context.bind_group([keys.other_dimension])?;
                let owner = context.run_action(layout, [1], 7, || ReturnedStorageOwner {
                    storage: Some(storage),
                    drops: Rc::clone(&drops),
                    panic,
                })?;
                signal.token.cancel();
                Ok(owner)
            })
        }));
        if panic {
            assert!(result.is_err());
        } else {
            assert!(matches!(
                result,
                Ok(Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::Control(ControlCause::Cancelled {
                        origin: CancellationOrigin::Signal
                    })
                )))
            ));
        }
        assert_eq!(drops.get(), 1);
        assert_eq!(signal.polls.load(Ordering::SeqCst), 4);
        assert_eq!(physical_delta(physical, account.snapshot()), (0, 0));
        let after = operation
            .producer_host()
            .dimension_state(keys.dimension)
            .unwrap();
        assert_eq!(after.live_bytes(), baseline.live_bytes());
        let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Control);
        assert_work(&evidence, keys.other_dimension, 1, 1);
        assert_work(
            &evidence,
            keys.dimension,
            baseline.attempted() + 2,
            baseline.consumed() + 2,
        );
        assert!(!account.snapshot().poisoned());
    }
}

#[test]
fn nested_parser_allocation_internal_share_and_escaping_storage_use_one_real_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(32, 32, 0),
    );
    let baseline = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    let physical = account.snapshot();
    let mut output = [0; 3];
    let storage = operation
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let parent_layout = context.bind_group([keys.other_dimension])?;
            let storage = context.parser_parent::<_, StoreInvocationError, 1>(
                parent_layout,
                [2],
                [None],
                |parent| {
                    let child_layout = parent.bind_group([keys.other_dimension])?;
                    let storage = parent.child::<_, StoreInvocationError, 1>(
                        child_layout,
                        [3],
                        [Some(3)],
                        |child| {
                            child.run_prefix([1], 7, || {
                                output[0] = 1;
                                Ok::<(), StoreInvocationError>(())
                            })?;
                            let storage = child.allocate_bytes(keys.phase, keys.dimension, 5)?;
                            assert!(storage.as_ref().capacity() >= 5);
                            assert!(storage.as_ref().capacity() <= 8);
                            assert_eq!(
                                physical_delta(physical, account.snapshot()),
                                (storage.as_ref().capacity() as u64, 1)
                            );
                            child.run_prefix([2], 11, || {
                                output[1] = 2;
                                Ok::<(), StoreInvocationError>(())
                            })?;
                            Ok(storage)
                        },
                    )?;
                    parent.run_prefix([2], 5, || {
                        output[2] = 3;
                        Ok::<(), StoreInvocationError>(())
                    })?;
                    Ok(storage)
                },
            )?;
            assert_eq!(context.work_counters(keys.other_dimension)?, (5, 5, 0));
            assert_eq!(
                context.work_counters(keys.dimension)?,
                (baseline.attempted() + 2, baseline.consumed() + 2, 0)
            );
            Ok(storage)
        })
        .unwrap();
    assert_eq!(output, [1, 2, 3]);
    let live = operation
        .producer_host()
        .dimension_state(keys.dimension)
        .unwrap();
    assert_eq!(
        live.live_bytes(),
        baseline.live_bytes() + storage.as_ref().capacity() as u64
    );
    // A genuine storage owner leaves and reenters execution; no work/context/layout escapes.
    operation
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([keys.other_dimension])?;
            context.run_action(layout, [1], 2, || assert!(storage.as_ref().is_empty()))?;
            assert_eq!(context.work_counters(keys.other_dimension)?, (6, 6, 0));
            Ok(())
        })
        .unwrap();
    drop(storage);
    assert_eq!(physical_delta(physical, account.snapshot()), (0, 0));
    assert_eq!(
        operation
            .producer_host()
            .dimension_state(keys.dimension)
            .unwrap()
            .live_bytes(),
        baseline.live_bytes()
    );
    let success = match operation
        .prepare_evidence_only_success(final_progress())
        .unwrap()
    {
        PreparedStoreEvidenceOnlyOutcome::Success(success) => success.handoff(),
        PreparedStoreEvidenceOnlyOutcome::MinimalFallback(_) => {
            panic!("a resolved trace remains success eligible")
        }
    };
    assert_work(&success.ledger_evidence, keys.other_dimension, 6, 6);
    drop(success);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn returned_semantic_error_is_completed_before_inspection_and_has_no_false_failure() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default(),
        DimensionPolicy::counter(8, 8, 0),
    );
    let mut physical = 0;
    let value: Result<(), u8> = operation
        .with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([keys.other_dimension])?;
            let value = context.run_action(layout, [1], 9, || {
                physical = 1;
                Err(17)
            })?;
            assert_eq!(context.work_counters(keys.other_dimension)?, (1, 1, 0));
            Ok(value)
        })
        .unwrap();
    assert_eq!(value, Err(17));
    assert_eq!(physical, 1);
    assert!(matches!(
        operation
            .prepare_evidence_only_success(final_progress())
            .unwrap(),
        PreparedStoreEvidenceOnlyOutcome::Success(_)
    ));
}

#[test]
fn weighted_actions_poll_before_crossing_the_bound_and_never_change_numeric_work() {
    for (costs, expected_polls) in [([128, 128], 2), ([255, 2], 3), ([256, 1], 3)] {
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
        signal.polls.store(0, Ordering::SeqCst);
        let mut physical = 0;
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([keys.other_dimension])?;
                for cost in costs {
                    context.run_action(layout, [1], cost, || physical += 1)?;
                }
                assert_eq!(context.work_counters(keys.other_dimension)?, (2, 2, 0));
                Ok(())
            })
            .unwrap();
        assert_eq!(physical, 2);
        assert_eq!(signal.polls.load(Ordering::SeqCst), expected_polls);
    }
}

#[test]
fn weighted_frontier_observes_real_cancellation_before_the_next_admission() {
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
    signal.polls.store(0, Ordering::SeqCst);
    let mut physical = 0;
    let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
        let layout = context.bind_group([keys.other_dimension])?;
        context.run_action(layout, [1], 128, || {
            physical += 1;
            signal.token.cancel();
        })?;
        context.run_action(layout, [1], 128, || physical += 1)?;
        context.run_action(layout, [1], 1, || physical += 1)
    });
    assert!(matches!(
        result,
        Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        )))
    ));
    assert_eq!(physical, 2);
    assert_eq!(signal.polls.load(Ordering::SeqCst), 2);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Control);
    assert_work(&evidence, keys.other_dimension, 2, 2);
}

#[test]
fn invalid_action_weights_deny_the_leaf_without_attempting_numeric_work() {
    for cost in [0_u16, 257, u16::MAX] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(8, 8, 0),
        );
        let physical = Cell::new(0);
        assert!(matches!(
            operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([keys.other_dimension])?;
                context.run_action(layout, [1], cost, || physical.set(1))
            }),
            Err(StoreInvocationError::Runtime(
                StoreRuntimeError::FallbackOnly
            ))
        ));
        assert_eq!(physical.get(), 0);
        let evidence =
            rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
        assert_work(&evidence, keys.other_dimension, 0, 0);
    }
}

#[test]
fn direct_parser_denial_keeps_the_literal_item_index_and_caller_ordered_attempts() {
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
    signal.polls.store(0, Ordering::SeqCst);
    let physical = Cell::new(0);
    let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
        let layout = context.bind_group([
            keys.other_dimension,
            keys.other_dimension,
            keys.other_dimension,
        ])?;
        context.parser_child::<_, StoreInvocationError, 3>(
            layout,
            [1, 3, 2],
            [None, Some(2), None],
            |_child| {
                physical.set(1);
                Ok(())
            },
        )
    });
    assert!(matches!(
        result,
        Err(StoreInvocationError::ItemLimit {
            index: 1,
            value: 3,
            limit: 2
        })
    ));
    assert_eq!(physical.get(), 0);
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
    assert_work(&evidence, keys.other_dimension, 4, 0);
}

#[test]
fn caught_leaf_panic_immediately_resolves_pending_and_prevents_more_physical_work() {
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
    signal.polls.store(0, Ordering::SeqCst);
    let mut physical = 0;
    assert!(
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([keys.other_dimension])?;
                context.run_action(layout, [1], 3, || physical += 1)?;
                let panic = catch_unwind(AssertUnwindSafe(|| {
                    context.run_action(layout, [1], 3, || {
                        physical += 1;
                        panic!("leaf");
                    })
                }));
                assert!(panic.is_err());
                assert_eq!(context.work_counters(keys.other_dimension)?, (2, 1, 0));
                assert!(
                    context
                        .run_action(layout, [1], 3, || physical += 1)
                        .is_err()
                );
                Ok(())
            })
            .is_err()
    );
    assert_eq!(physical, 2);
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
    assert_work(&evidence, keys.other_dimension, 2, 1);
}

#[test]
fn parser_prefix_panic_caught_inside_the_child_closes_its_channel_immediately() {
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
    signal.polls.store(0, Ordering::SeqCst);
    let mut physical = 0;
    let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
        let layout = context.bind_group([keys.other_dimension])?;
        context.parser_parent::<_, StoreInvocationError, 1>(layout, [1], [None], |parent| {
            parent.child::<_, StoreInvocationError, 1>(layout, [2], [None], |child| {
                child.run_prefix([1], 2, || {
                    physical += 1;
                    Ok::<(), StoreInvocationError>(())
                })?;
                assert!(
                    catch_unwind(AssertUnwindSafe(|| child.run_prefix(
                        [1],
                        2,
                        || -> Result<(), StoreInvocationError> {
                            physical += 1;
                            panic!("parser prefix");
                        }
                    )))
                    .is_err()
                );
                child.run_prefix([1], 2, || {
                    physical += 1;
                    Ok::<(), StoreInvocationError>(())
                })
            })?;
            parent.run_prefix([1], 2, || Ok(()))
        })
    });
    assert!(result.is_err());
    assert_eq!(physical, 2);
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
    assert_work(&evidence, keys.other_dimension, 3, 1);
}

struct DropProbe {
    drops: Rc<Cell<usize>>,
    panic: bool,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
        if self.panic {
            panic!("real capture/result destructor");
        }
    }
}

#[test]
fn denied_capture_drop_cannot_replace_the_original_ledger_cause() {
    let factory = factory(1);
    let account = account(&factory);
    let signal = CountedCancellation::default();
    let mut invocation_root = TestInvocationRoot::new();
    let (mut operation, keys) = operation_with_work_policy(
        &mut invocation_root,
        &account,
        OperationControl::default().with_cancellation(signal.clone()),
        DimensionPolicy::counter(0, 0, 0),
    );
    signal.polls.store(0, Ordering::SeqCst);
    let drops = Rc::new(Cell::new(0));
    let captured = DropProbe {
        drops: Rc::clone(&drops),
        panic: true,
    };
    let physical = Rc::new(Cell::new(0));
    let written = Rc::clone(&physical);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            operation.with_exclusive_execution::<_, StoreInvocationError>(move |context| {
                let layout = context.bind_group([keys.other_dimension])?;
                context.run_action(layout, [1], 1, move || {
                    std::hint::black_box(&captured);
                    written.set(written.get() + 1);
                })
            })
        }))
        .is_err()
    );
    assert_eq!(physical.get(), 0);
    assert_eq!(drops.get(), 1);
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
    assert_work(&evidence, keys.other_dimension, 1, 0);
}

#[test]
fn real_idle_capture_and_completed_result_destructors_keep_the_exact_prefix() {
    for capture in [false, true] {
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
        signal.polls.store(0, Ordering::SeqCst);
        let drops = Rc::new(Cell::new(0));
        let probe = DropProbe {
            drops: Rc::clone(&drops),
            panic: true,
        };
        let physical = Rc::new(Cell::new(0));
        let written = Rc::clone(&physical);
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                operation.with_exclusive_execution::<_, StoreInvocationError>(move |context| {
                    let layout = context.bind_group([keys.other_dimension])?;
                    if capture {
                        context.run_action(layout, [1], 1, || written.set(1))?;
                        std::hint::black_box(&probe);
                    } else {
                        let _result = context.run_action(layout, [1], 1, move || {
                            written.set(1);
                            probe
                        })?;
                        // Natural local result destruction happens with no action frame alive.
                    }
                    Ok(())
                })
            }))
            .is_err()
        );
        assert_eq!(physical.get(), 1);
        assert_eq!(drops.get(), 1);
        assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
        let evidence =
            rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
        assert_work(&evidence, keys.other_dimension, 1, 1);
    }
}

#[test]
fn explicit_policy_tightening_denies_the_next_capture_without_polling_again() {
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
    signal.polls.store(0, Ordering::SeqCst);
    let mut physical = 0;
    let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
        let layout = context.bind_group([keys.other_dimension])?;
        context.run_action(layout, [1], 1, || physical += 1)?;
        context
            .tighten_dimension_policy(keys.other_dimension, DimensionPolicy::counter(1, 1, 0))?;
        context.run_action(layout, [1], 1, || physical += 1)
    });
    assert!(matches!(
        result,
        Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                value: 2,
                limit: 1,
                ..
            }
        )))
    ));
    assert_eq!(physical, 1);
    assert_eq!(signal.polls.load(Ordering::SeqCst), 1);
    let evidence = rejected_success_evidence(operation, StorePreparationFailureClass::Ledger);
    assert_work(&evidence, keys.other_dimension, 2, 1);
}

#[test]
#[allow(
    forgetting_references,
    reason = "a client may forget its borrow but cannot forget the host's frame"
)]
fn forgetting_a_parser_borrow_cannot_forget_zero_or_incomplete_host_ownership() {
    for amount in [0, 1] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_root = TestInvocationRoot::new();
        let (mut operation, keys) = operation_with_work_policy(
            &mut invocation_root,
            &account,
            OperationControl::default(),
            DimensionPolicy::counter(8, 8, 0),
        );
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([keys.other_dimension])?;
            context.parser_parent::<_, StoreInvocationError, 1>(
                layout,
                [amount],
                [None],
                |parent| {
                    std::mem::forget(parent);
                    Ok(())
                },
            )
        });
        if amount == 0 {
            result.unwrap();
            assert!(matches!(
                operation
                    .prepare_evidence_only_success(final_progress())
                    .unwrap(),
                PreparedStoreEvidenceOnlyOutcome::Success(_)
            ));
        } else {
            assert!(result.is_err());
            let evidence =
                rejected_success_evidence(operation, StorePreparationFailureClass::ProductGraph);
            assert_work(&evidence, keys.other_dimension, 1, 0);
        }
    }
}

#[test]
fn host_caught_extension_panic_is_not_a_normally_returned_error_or_eof() {
    use baozi_operation::__retained_store_integration::activation::StoreExtensionResult;
    for panic in [false, true] {
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
        signal.polls.store(0, Ordering::SeqCst);
        let physical = Cell::new(0);
        let next = Cell::new(0);
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([keys.other_dimension])?;
            let value = context.run_extension(layout, [1], 3, || -> Result<usize, u8> {
                physical.set(1);
                if panic {
                    panic!("host-caught extension");
                }
                Err(7)
            })?;
            assert_eq!(
                context.work_counters(keys.other_dimension)?,
                (1, if panic { 0 } else { 1 }, 0)
            );
            if panic {
                assert!(matches!(value, StoreExtensionResult::Panicked));
                assert!(context.run_action(layout, [1], 1, || next.set(1)).is_err());
            } else {
                assert!(matches!(value, StoreExtensionResult::Returned(Err(7))));
            }
            Ok(())
        });
        assert_eq!(physical.get(), 1);
        assert_eq!(next.get(), 0);
        assert_eq!(
            signal.polls.load(Ordering::SeqCst),
            if panic { 1 } else { 2 }
        );
        if panic {
            assert!(result.is_err());
            let evidence = operation
                .prepare_minimal_fallback(74)
                .unwrap()
                .handoff()
                .ledger_evidence;
            assert_eq!(
                store_failure_cause(&evidence).class(),
                StorePreparationFailureClass::Requested
            );
            assert_work(&evidence, keys.other_dimension, 1, 0);
        } else {
            result.unwrap();
        }
    }
}
