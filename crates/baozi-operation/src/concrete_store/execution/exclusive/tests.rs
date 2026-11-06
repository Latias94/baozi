//! Private host-boundary probes use an actual Ledger, not a shadow numeric implementation.
use super::*;
use crate::ledger::{Ledger, LedgerLimits};
use crate::retained::RetainedHandleSharePlan;
use crate::{
    CancellationSignal, CancellationToken, OperationControl, OperationCoordinator,
    PhysicalStorageAccountFactory, PhysicalStorageAccountLimits,
};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Owner;
impl OperationOwner for Owner {}

fn actual_ledger() -> (Ledger<Owner, u8, StoreDimensionKey>, StoreDimensionKey) {
    let catalog = StoreSemanticCatalogId::new(*b"baozi-test-store", NonZeroU32::new(1).unwrap());
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let key = domain
        .try_admit_dimension(StoreDimensionSemanticId::new(
            catalog,
            NonZeroU64::new(1).unwrap(),
        ))
        .unwrap();
    let limits = LedgerLimits::new(u64::MAX, [(key, DimensionPolicy::counter(8, 8, 0))]).unwrap();
    (Ledger::try_new(limits, &[1]).unwrap(), key)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ParserProgress;

impl ProgressPhase for ParserProgress {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "parser"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ParserUnit;

impl ProgressUnit for ParserUnit {
    fn stable_id(self) -> &'static str {
        "comparisons"
    }
}

struct ParserPolls(Arc<AtomicUsize>, CancellationToken);

impl CancellationSignal for ParserPolls {
    fn is_cancelled(&self) -> bool {
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1.is_cancelled()
    }
}

fn with_parser_operation(
    body: impl FnOnce(
        &mut RetainedStoreOperation<'_, Owner, ParserProgress, ParserUnit>,
        StoreDimensionKey,
        &AtomicUsize,
    ),
) {
    with_cancellable_parser_operation(CancellationToken::new(), body);
}

fn with_cancellable_parser_operation(
    cancellation: CancellationToken,
    body: impl FnOnce(
        &mut RetainedStoreOperation<'_, Owner, ParserProgress, ParserUnit>,
        StoreDimensionKey,
        &AtomicUsize,
    ),
) {
    let catalog = StoreSemanticCatalogId::new(*b"parser-host-test", NonZeroU32::MIN);
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let phase = domain
        .try_admit_phase(crate::StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
        .unwrap();
    let [storage_key, work_key, failure_key] = [1, 2, 3].map(|local| {
        domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    });
    let failure_work =
        RetainedStoreFailureTerminalWorkPlan::new(failure_key, 1, u64::MAX, u64::MAX).unwrap();
    let storage_policy = DimensionPolicy::reservable_bytes(
        failure_work.ordinary_attempted_work_limit() - 512,
        failure_work.ordinary_consumed_work_limit() - 512,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (storage_key, storage_policy),
            (work_key, DimensionPolicy::counter(512, 512, 0)),
            (failure_key, failure_work.requirement().dimension_policy()),
        ],
    )
    .unwrap();
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        RetainedStoreWriterPlan::new(
            phase,
            storage_key,
            storage_key,
            RetainedStoreBatchLimits::new(32, 8).unwrap(),
        ),
        RetainedStoreSharePlan::new(storage_key, storage_key),
        StoreFallbackEnvelope::new(91),
        storage_key,
        1,
        storage_key,
        1,
        failure_work,
    );
    let factory =
        PhysicalStorageAccountFactory::try_new_for_live_accounts(NonZeroU64::MIN).unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(1 << 20, 32).unwrap())
        .unwrap();
    let polls = Arc::new(AtomicUsize::new(0));
    let control = OperationControl::default()
        .with_cancellation(ParserPolls(Arc::clone(&polls), cancellation));
    let mut root = StoreInvocationRoot::<Owner, ParserProgress, ParserUnit>::new();
    let mut operation =
        RetainedStoreOperation::try_start(&mut root, control, ledger_plan, &account, start)
            .unwrap();
    polls.store(0, Ordering::SeqCst);
    body(&mut operation, work_key, &polls);
    drop(operation);
    drop(root);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn read_row_handback_keeps_actual_remainder_and_direct_debt_until_checkpoint() {
    with_parser_operation(|operation, key, polls| {
        let mut reads = 0;
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([key])?;
                context.parser_child::<_, StoreInvocationError, 1>(
                    layout,
                    [2],
                    [None],
                    |child| {
                        child.host.context.parts.ordinary.debt.set(255);
                        child.with_read_row(
                            &mut |row| -> Result<(), StoreInvocationError> {
                                assert!(row.ready_for_read()?);
                                reads += 1;
                                row.complete_read(1)?;
                                assert!(!row.ready_for_read()?);
                                Ok(())
                            },
                            |cause| cause,
                            |_, cause| cause,
                        )?;
                        assert_eq!(reads, 1);
                        assert_eq!(child.host.frame.remaining, [1]);
                        assert!(child.host.frame.active);
                        assert_eq!(child.work_counters(key)?, (2, 1, 0));
                        assert_eq!(child.host.context.parts.ordinary.debt.get(), 256);
                        assert_eq!(polls.load(Ordering::SeqCst), 1);
                        child.checkpoint()?;
                        assert_eq!(polls.load(Ordering::SeqCst), 2);
                        child.with_read_row(
                            &mut |row| -> Result<(), StoreInvocationError> {
                                assert!(row.ready_for_read()?);
                                reads += 1;
                                row.complete_read(1)
                            },
                            |cause| cause,
                            |_, cause| cause,
                        )?;
                        assert_eq!(child.host.frame.remaining, [0]);
                        assert_eq!(child.work_counters(key)?, (2, 2, 0));
                        assert_eq!(child.host.context.parts.ordinary.debt.get(), 1);
                        Ok(())
                    },
                )?;
                context.parts.work.check_quiescent().unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(reads, 2);
        assert_eq!(polls.load(Ordering::SeqCst), 3);
    });
}

#[test]
fn read_row_debt_checkpoint_cancellation_retains_only_the_first_classification() {
    let cancellation = CancellationToken::new();
    with_cancellable_parser_operation(cancellation.clone(), |operation, key, polls| {
        let classified = Cell::new(0);
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([key])?;
            let result =
                context.parser_child::<_, StoreInvocationError, 1>(layout, [2], [None], |child| {
                    child.host.context.parts.ordinary.debt.set(255);
                    child.with_read_row(
                        &mut |row| -> Result<(), StoreInvocationError> {
                            assert!(row.ready_for_read()?);
                            classified.set(classified.get() + 1);
                            row.complete_read(1)?;
                            assert!(!row.ready_for_read()?);
                            Ok(())
                        },
                        |cause| cause,
                        |_, cause| cause,
                    )?;
                    cancellation.cancel();
                    let cause = child.checkpoint().unwrap_err();
                    assert!(matches!(
                        &cause,
                        StoreInvocationError::Runtime(StoreRuntimeError::Control(
                            crate::ControlCause::Cancelled {
                                origin: crate::CancellationOrigin::Signal
                            }
                        ))
                    ));
                    assert_eq!(polls.load(Ordering::SeqCst), 2);
                    assert_eq!(classified.get(), 1);
                    assert_eq!(child.host.frame.remaining, [1]);
                    assert_eq!(child.work_counters(key)?, (2, 1, 0));
                    assert_eq!(child.host.context.parts.ordinary.debt.get(), 256);
                    let next = child.with_read_row(
                        &mut |_| -> Result<(), StoreInvocationError> {
                            panic!("cancelled row body entered")
                        },
                        |cause| cause,
                        |_, cause| cause,
                    );
                    assert!(matches!(
                        next,
                        Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
                            crate::ControlCause::Cancelled {
                                origin: crate::CancellationOrigin::Signal
                            }
                        )))
                    ));
                    assert_eq!(child.host.frame.remaining, [1]);
                    assert_eq!(child.work_counters(key)?, (2, 1, 0));
                    Err::<(), _>(cause)
                });
            context.parts.work.check_quiescent().unwrap();
            assert_eq!(context.work_counters(key)?, (2, 1, 0));
            assert_eq!(
                context.parts.ordinary.failure.get(),
                Some(PreparationFailureCode::from_control(
                    &crate::ControlCause::Cancelled {
                        origin: crate::CancellationOrigin::Signal
                    }
                ))
            );
            result
        });
        assert!(matches!(
            result,
            Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
                crate::ControlCause::Cancelled {
                    origin: crate::CancellationOrigin::Signal
                }
            )))
        ));
        assert_eq!(classified.get(), 1);
    });
}

#[test]
fn read_row_registry_units_do_not_change_direct_debt() {
    with_parser_operation(|operation, key, _| {
        let boundary = StoreImporterBoundary::default();
        assert!(boundary.begin());
        operation
            .with_parser_execution::<_, StoreInvocationError>(&boundary, |context| {
                let layout = context.bind_group([key])?;
                context.parser_child::<_, StoreInvocationError, 1>(
                    layout,
                    [257],
                    [None],
                    |child| {
                        child.with_read_row(
                            &mut |row| -> Result<(), StoreInvocationError> {
                                for _ in 0..257 {
                                    assert!(row.ready_for_read()?);
                                    row.complete_read(1)?;
                                }
                                Ok(())
                            },
                            |cause| cause,
                            |_, cause| cause,
                        )?;
                        assert_eq!(child.host.context.parts.ordinary.debt.get(), 0);
                        assert_eq!(child.work_counters(key)?, (257, 257, 0));
                        Ok(())
                    },
                )?;
                assert_eq!(context.parts.ordinary.debt.get(), 1);
                context.parts.work.check_quiescent().unwrap();
                Ok(())
            })
            .unwrap();
    });
}

#[test]
fn read_row_numeric_failure_stops_zero_writes_and_normal_or_error_returns() {
    for propagated in [false, true] {
        with_parser_operation(|operation, key, _| {
            let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([key])?;
                let result = context.parser_child::<_, StoreInvocationError, 1>(
                    layout,
                    [2],
                    [None],
                    |child| {
                        let result = child.with_read_row(
                            &mut |row| -> Result<(), StoreInvocationError> {
                                row.complete_read(1)?;
                                let cause = row.complete_read(2).unwrap_err();
                                assert!(matches!(
                                    &cause,
                                    StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                                        LedgerCause::InactiveLinearWorkPermit
                                    ))
                                ));
                                assert!(row.complete_read(0).is_err());
                                assert!(row.complete_read(1).is_err());
                                assert!(row.ready_for_read().is_err());
                                if propagated { Err(cause) } else { Ok(()) }
                            },
                            |cause| cause,
                            |host, cause| {
                                assert!(host.host.frame.active);
                                assert_eq!(host.host.frame.remaining, [1]);
                                cause
                            },
                        );
                        if propagated {
                            assert!(matches!(
                                &result,
                                Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                                    LedgerCause::InactiveLinearWorkPermit
                                )))
                            ));
                        } else {
                            assert!(matches!(
                                &result,
                                Err(StoreInvocationError::Runtime(
                                    StoreRuntimeError::FallbackOnly
                                ))
                            ));
                        }
                        assert!(!child.host.frame.active);
                        assert_eq!(child.host.frame.remaining, [1]);
                        assert_eq!(child.work_counters(key)?, (2, 1, 0));
                        assert!(
                            child
                                .with_read_row(
                                    &mut |_| -> Result<(), StoreInvocationError> {
                                        panic!("stopped host reentered")
                                    },
                                    |cause| cause,
                                    |_, cause| cause,
                                )
                                .is_err()
                        );
                        // Even an integration caller ignoring this result cannot manufacture success.
                        Ok(())
                    },
                );
                assert!(result.is_err());
                assert_ne!(
                    context.parts.ordinary.execution_state.get(),
                    StoreExecutionState::IncompleteSuccess
                );
                assert_eq!(
                    context.parts.ordinary.failure.get(),
                    Some(PreparationFailureCode::ledger())
                );
                assert_eq!(context.work_counters(key)?, (2, 1, 0));
                context.parts.work.check_quiescent().unwrap();
                result
            });
            assert!(result.is_err());
        });
    }
}

#[test]
fn read_row_unwind_keeps_only_completed_units_and_the_scope_guard_armed() {
    with_parser_operation(|operation, key, _| {
        let physical = Cell::new(0);
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([key])?;
            let result =
                context.parser_child::<_, StoreInvocationError, 1>(layout, [3], [None], |child| {
                    let unwind = catch_unwind(AssertUnwindSafe(|| {
                        let _: Result<(), StoreInvocationError> = child.with_read_row(
                            &mut |row| -> Result<(), StoreInvocationError> {
                                physical.set(1);
                                row.complete_read(1)?;
                                physical.set(2);
                                panic!("second classification interrupted before completion")
                            },
                            |cause| cause,
                            |_, cause| cause,
                        );
                    }));
                    assert!(unwind.is_err());
                    assert!(!child.host.frame.active);
                    assert_eq!(child.host.frame.remaining, [2]);
                    assert_eq!(child.work_counters(key)?, (3, 1, 0));
                    assert!(child.checkpoint().is_err());
                    Ok(())
                });
            assert!(result.is_err());
            assert_eq!(physical.get(), 2);
            assert_eq!(context.work_counters(key)?, (3, 1, 0));
            context.parts.work.check_quiescent().unwrap();
            result
        });
        assert!(result.is_err());
    });
}

#[test]
fn borrowed_parser_replenishes_twice_from_zero_without_resetting_comparison_cadence() {
    with_parser_operation(|operation, key, polls| {
        let values: [u16; 258] = std::array::from_fn(|index| u16::try_from(index).unwrap());
        let mut comparisons = 0;
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([key])?;
                context.parser_child::<_, StoreInvocationError, 1>(
                    layout,
                    [0],
                    [None],
                    |child| {
                        assert_eq!(child.work_counters(key)?, (0, 0, 0));
                        assert_eq!(polls.load(Ordering::SeqCst), 1);
                        child.replenish(256)?;
                        assert_eq!(child.work_counters(key)?, (256, 0, 0));
                        assert_eq!(polls.load(Ordering::SeqCst), 1);
                        for pair in values[..257].windows(2) {
                            let ordering = child.run_prefix([1], 1, || {
                                comparisons += 1;
                                Ok::<_, StoreInvocationError>(pair[0].cmp(&pair[1]))
                            })?;
                            assert_eq!(ordering, std::cmp::Ordering::Less);
                        }
                        assert_eq!(comparisons, 256);
                        assert_eq!(child.work_counters(key)?, (256, 256, 0));
                        assert_eq!(polls.load(Ordering::SeqCst), 1);
                        child.replenish(1)?;
                        assert_eq!(child.work_counters(key)?, (257, 256, 0));
                        assert_eq!(polls.load(Ordering::SeqCst), 1);
                        let ordering = child.run_prefix([1], 1, || {
                            assert_eq!(polls.load(Ordering::SeqCst), 2);
                            comparisons += 1;
                            Ok::<_, StoreInvocationError>(values[256].cmp(&values[257]))
                        })?;
                        assert_eq!(ordering, std::cmp::Ordering::Less);
                        assert_eq!(child.work_counters(key)?, (257, 257, 0));
                        assert_eq!(polls.load(Ordering::SeqCst), 2);
                        Ok(())
                    },
                )?;
                context.parts.work.check_quiescent().unwrap();
                assert_eq!(context.work_counters(key)?, (257, 257, 0));
                assert_eq!(context.parts.ordinary.failure.get(), None);
                Ok(())
            })
            .unwrap();
        assert_eq!(comparisons, 257);
        assert_eq!(polls.load(Ordering::SeqCst), 3);
    });
}

#[test]
fn borrowed_parser_replenishment_rejects_a_remaining_suffix_without_new_work_or_poll() {
    with_parser_operation(|operation, key, polls| {
        let mut comparisons = 0;
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([key])?;
            let result =
                context.parser_child::<_, StoreInvocationError, 1>(layout, [2], [None], |child| {
                    let ordering = child.run_prefix([1], 1, || {
                        comparisons += 1;
                        Ok::<_, StoreInvocationError>(3_u16.cmp(&5))
                    })?;
                    assert_eq!(ordering, std::cmp::Ordering::Less);
                    assert_eq!(child.work_counters(key)?, (2, 1, 0));
                    let result = child.replenish(2);
                    assert!(matches!(
                        &result,
                        Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                            LedgerCause::InactiveLinearWorkPermit
                        )))
                    ));
                    assert!(matches!(
                        child.replenish(1),
                        Err(StoreInvocationError::Runtime(
                            StoreRuntimeError::FallbackOnly
                        ))
                    ));
                    assert_eq!(child.work_counters(key)?, (2, 1, 0));
                    assert_eq!(polls.load(Ordering::SeqCst), 1);
                    result
                });
            context.parts.work.check_quiescent().unwrap();
            assert_eq!(context.work_counters(key)?, (2, 1, 0));
            assert_eq!(
                context.parts.ordinary.failure.get(),
                Some(PreparationFailureCode::ledger())
            );
            result
        });
        assert!(matches!(
            result,
            Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                LedgerCause::InactiveLinearWorkPermit
            )))
        ));
        assert_eq!(comparisons, 1);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn borrowed_parser_replenishment_cannot_split_an_item_limited_comparison() {
    with_parser_operation(|operation, key, polls| {
        let mut comparisons = 0;
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([key])?;
            let result = context.parser_child::<_, StoreInvocationError, 1>(
                layout,
                [1],
                [Some(1)],
                |child| {
                    let ordering = child.run_prefix([1], 1, || {
                        comparisons += 1;
                        Ok::<_, StoreInvocationError>(5_u16.cmp(&3))
                    })?;
                    assert_eq!(ordering, std::cmp::Ordering::Greater);
                    assert_eq!(child.work_counters(key)?, (1, 1, 0));
                    let result = child.replenish(1);
                    assert!(matches!(
                        &result,
                        Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                            LedgerCause::InactiveLinearWorkPermit
                        )))
                    ));
                    assert_eq!(child.work_counters(key)?, (1, 1, 0));
                    assert_eq!(polls.load(Ordering::SeqCst), 1);
                    result
                },
            );
            context.parts.work.check_quiescent().unwrap();
            assert_eq!(context.work_counters(key)?, (1, 1, 0));
            assert_eq!(
                context.parts.ordinary.failure.get(),
                Some(PreparationFailureCode::ledger())
            );
            result
        });
        assert!(matches!(
            result,
            Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                LedgerCause::InactiveLinearWorkPermit
            )))
        ));
        assert_eq!(comparisons, 1);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn scan_boundary_keeps_the_parent_available_for_the_real_row_and_next_child() {
    with_parser_operation(|operation, key, polls| {
        let input = b"a\nrest";
        let mut classified = 0;
        let mut row_writes = 0;
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([key])?;
                context.parser_parent::<_, StoreInvocationError, 1>(
                    layout,
                    [1],
                    [None],
                    |parent| {
                        let end = parent.scan_child::<_, StoreInvocationError, 1>(
                            layout,
                            [u64::try_from(input.len()).unwrap()],
                            [None],
                            |scan| {
                                for (index, byte) in input.iter().copied().enumerate() {
                                    let newline = scan.run_prefix([1], 1, || {
                                        classified += 1;
                                        Ok::<_, StoreInvocationError>(byte == b'\n')
                                    })?;
                                    if newline {
                                        return Ok(StoreScanReturn::Boundary(index + 1));
                                    }
                                }
                                Ok(StoreScanReturn::Exhausted(input.len()))
                            },
                        )?;
                        assert_eq!(end, 2);
                        assert_eq!(parent.work_counters(key)?, (7, 2, 0));
                        parent.child::<_, StoreInvocationError, 1>(layout, [2], [None], |row| {
                            row.run_prefix([2], 1, || {
                                row_writes += 2;
                                Ok::<_, StoreInvocationError>(())
                            })
                        })?;
                        assert_eq!(parent.work_counters(key)?, (9, 4, 0));
                        parent.prescribed_exit_checkpoint()?;
                        parent.complete_parent()
                    },
                )?;
                assert_eq!(context.work_counters(key)?, (9, 5, 0));
                context.parser_child::<_, StoreInvocationError, 1>(
                    layout,
                    [1],
                    [None],
                    |next| {
                        next.run_prefix([1], 1, || {
                            row_writes += 1;
                            Ok::<_, StoreInvocationError>(())
                        })
                    },
                )?;
                assert_eq!(context.work_counters(key)?, (10, 6, 0));
                assert_eq!(context.parts.ordinary.failure.get(), None);
                context.parts.work.check_quiescent().unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(classified, 2);
        assert_eq!(row_writes, 3);
        assert_eq!(polls.load(Ordering::SeqCst), 3);
    });
}

#[test]
fn scan_boundary_does_not_add_successful_import_channel_debt() {
    with_parser_operation(|operation, key, polls| {
        let boundary = StoreImporterBoundary::default();
        assert!(boundary.begin());
        let mut classified = 0;
        operation
            .with_parser_execution::<_, StoreInvocationError>(&boundary, |context| {
                let layout = context.bind_group([key])?;
                for byte in [b'\n'; 257] {
                    let newline = context.parser_scan_child::<_, StoreInvocationError, 1>(
                        layout,
                        [1],
                        [None],
                        |scan| {
                            let newline = scan.run_prefix([1], 1, || {
                                classified += 1;
                                Ok::<_, StoreInvocationError>(byte == b'\n')
                            })?;
                            Ok(StoreScanReturn::Boundary(newline))
                        },
                    )?;
                    assert!(newline);
                    assert_eq!(polls.load(Ordering::SeqCst), 1);
                }
                assert_eq!(context.work_counters(key)?, (257, 257, 0));
                context.parser_child::<_, StoreInvocationError, 1>(layout, [1], [None], |row| {
                    row.run_prefix([1], 1, || {
                        assert_eq!(polls.load(Ordering::SeqCst), 1);
                        Ok::<_, StoreInvocationError>(())
                    })
                })?;
                assert_eq!(context.work_counters(key)?, (258, 258, 0));
                assert_eq!(context.parts.ordinary.debt.get(), 1);
                assert_eq!(context.parts.ordinary.failure.get(), None);
                context.prescribed_exit_checkpoint()
            })
            .unwrap();
        boundary.finish_returned(false);
        assert_eq!(classified, 257);
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn scan_exhausted_and_ordinary_success_still_reject_an_unfinished_suffix() {
    for scan in [false, true] {
        with_parser_operation(|operation, key, polls| {
            let mut completed = 0;
            let mut reused = false;
            let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([key])?;
                let result = if scan {
                    context.parser_scan_child::<_, StoreInvocationError, 1>(
                        layout,
                        [2],
                        [None],
                        |child| {
                            child.run_prefix([1], 1, || {
                                completed += 1;
                                Ok::<_, StoreInvocationError>(())
                            })?;
                            Ok(StoreScanReturn::Exhausted(()))
                        },
                    )
                } else {
                    context.parser_child::<_, StoreInvocationError, 1>(
                        layout,
                        [2],
                        [None],
                        |child| {
                            child.run_prefix([1], 1, || {
                                completed += 1;
                                Ok::<_, StoreInvocationError>(())
                            })
                        },
                    )
                };
                assert!(matches!(
                    &result,
                    Err(StoreInvocationError::Runtime(
                        StoreRuntimeError::ObservedWorkIncomplete
                    ))
                ));
                assert_eq!(context.work_counters(key)?, (2, 1, 0));
                context.parts.work.check_quiescent().unwrap();
                assert!(
                    context
                        .parser_child::<_, StoreInvocationError, 1>(layout, [1], [None], |_child| {
                            reused = true;
                            Ok(())
                        })
                        .is_err()
                );
                assert_eq!(context.work_counters(key)?, (2, 1, 0));
                result
            });
            assert!(matches!(
                result,
                Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::ObservedWorkIncomplete
                ))
            ));
            assert_eq!(completed, 1);
            assert!(!reused);
            assert_eq!(polls.load(Ordering::SeqCst), 1);
        });
    }
}

#[test]
fn a_scan_boundary_cannot_heal_a_caught_physical_prefix_panic() {
    with_parser_operation(|operation, key, polls| {
        let mut caught = false;
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([key])?;
            let result = context.parser_scan_child::<_, StoreInvocationError, 1>(
                layout,
                [2],
                [None],
                |child| {
                    child.run_prefix([1], 1, || Ok::<_, StoreInvocationError>(()))?;
                    let panic = catch_unwind(AssertUnwindSafe(|| {
                        child.run_prefix([1], 1, || -> Result<(), StoreInvocationError> {
                            panic!("physical text classification panic");
                        })
                    }));
                    assert!(panic.is_err());
                    caught = true;
                    Ok(StoreScanReturn::Boundary(()))
                },
            );
            assert!(matches!(
                &result,
                Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::FallbackOnly
                ))
            ));
            assert_eq!(context.work_counters(key)?, (2, 1, 0));
            context.parts.work.check_quiescent().unwrap();
            assert_eq!(
                context.parts.ordinary.failure.get(),
                Some(PreparationFailureCode::product_graph(ABANDONED))
            );
            result
        });
        assert!(result.is_err());
        assert!(caught);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn host_completion_failure_cleans_the_suffix_before_any_result_is_disposed() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, mut storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [4], WorkLane::Producer)
        .unwrap();
    // The actual joint policy method installs the tighter limits before returning its current
    // pending-policy violation. This probes the same completion category as the legacy live alias.
    assert!(
        exclusive
            .tighten_policy(&mut storage, key, DimensionPolicy::counter(8, 2, 0))
            .is_err()
    );
    let physical = Cell::new(0);
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [4],
        failure: &failure,
        execution_state: &execution_state,
        boundary: None,
    };
    physical.set(1);
    assert!(matches!(
        host.complete([1]),
        Err(LedgerCause::LimitExceeded {
            class: crate::CounterClass::Consumed,
            value: 4,
            limit: 2,
            ..
        })
    ));
    assert!(!host.frame.active);
    assert_eq!(host.frame.remaining, [3]);
    drop(host);
    assert_eq!(physical.get(), 1);
    assert_eq!(exclusive.counters(key).unwrap(), (4, 1, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(failure.get(), Some(PreparationFailureCode::ledger()));
}

#[test]
fn unapplied_completion_failure_has_no_invented_consumption_and_no_later_abandonment() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, _storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [1], WorkLane::Producer)
        .unwrap();
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [1],
        failure: &failure,
        execution_state: &execution_state,
        boundary: None,
    };
    assert!(matches!(
        host.complete([2]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert!(!host.frame.active);
    drop(host);
    assert_eq!(exclusive.counters(key).unwrap(), (1, 0, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(failure.get(), Some(PreparationFailureCode::ledger()));
}

#[test]
fn an_internal_frame_does_not_claim_or_close_the_attempted_only_parser_channels() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, _storage) = owner.exclusive().unwrap();
    let parent_layout = exclusive.bind([key]).unwrap();
    let mut parent = parent_layout
        .begin_parser(&mut exclusive.work, [2], [None], ParserChannel::Parent)
        .unwrap();
    let mut child = parent_layout
        .begin_parser(&mut exclusive.work, [1], [None], ParserChannel::Child)
        .unwrap();
    let layout = exclusive.bind([key, key]).unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [1, 1], WorkLane::Internal)
        .unwrap();
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [1, 1],
        failure: &failure,
        execution_state: &execution_state,
        boundary: None,
    };
    host.complete([1, 0]).unwrap();
    host.complete([0, 1]).unwrap();
    host.finish().unwrap();
    child
        .complete(&parent_layout, &mut exclusive.work, [1])
        .unwrap();
    child.resolve(&mut exclusive.work).unwrap();
    parent
        .complete(&parent_layout, &mut exclusive.work, [2])
        .unwrap();
    parent.resolve(&mut exclusive.work).unwrap();
    assert_eq!(exclusive.counters(key).unwrap(), (5, 5, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(failure.get(), None);
}

#[test]
fn real_import_boundary_defers_only_unclassified_unwind_and_caught_return_is_sticky() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, _storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let boundary = StoreImporterBoundary::default();
    assert!(boundary.begin());
    let physical = Cell::new(0);
    let first = layout
        .admit_observed(&mut exclusive.work, [1], WorkLane::Producer)
        .unwrap();
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame: first,
        admitted: [1],
        failure: &failure,
        execution_state: &execution_state,
        boundary: Some(&boundary),
    };
    physical.set(1);
    host.complete([1]).unwrap();
    host.finish().unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [1], WorkLane::Producer)
        .unwrap();
    let unwind = catch_unwind(AssertUnwindSafe(|| {
        let _host = ActionHost {
            work: &mut exclusive.work,
            layout,
            frame,
            admitted: [1],
            failure: &failure,
            execution_state: &execution_state,
            boundary: Some(&boundary),
        };
        physical.set(2);
        panic!("actual numeric leaf");
    }));
    assert!(unwind.is_err());
    assert_eq!(physical.get(), 2);
    assert_eq!(exclusive.counters(key).unwrap(), (2, 1, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(execution_state.get(), StoreExecutionState::UnwindPending);
    assert_eq!(
        failure.get(),
        None,
        "a real importer catch has not classified this unwind"
    );
    StoreExecutionState::settle_caught_unwind(&execution_state, &failure);
    assert_eq!(
        failure.get(),
        Some(PreparationFailureCode::product_graph(ABANDONED))
    );
    record(&failure, PreparationFailureCode::domain(41));
    assert_eq!(
        failure.get(),
        Some(PreparationFailureCode::product_graph(ABANDONED))
    );
}

#[test]
fn cleanup_fault_precedes_later_import_disposition_and_preserves_the_actual_prefix() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, _storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let failure = Cell::new(Some(PreparationFailureCode::ledger()));
    let state = Cell::new(StoreExecutionState::Running);
    let boundary = StoreImporterBoundary::default();
    assert!(boundary.begin());
    let frame = layout
        .admit_observed(&mut exclusive.work, [2], WorkLane::Producer)
        .unwrap();
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [2],
        failure: &failure,
        execution_state: &state,
        boundary: Some(&boundary),
    };
    host.complete([1]).unwrap();
    drop(host);
    assert_eq!(exclusive.counters(key).unwrap(), (2, 1, 0));
    exclusive.check_quiescent().unwrap();
    boundary.finish_panicked();
    assert_eq!(failure.get(), Some(PreparationFailureCode::ledger()));
}

#[test]
fn failure_materializer_observes_active_frame_before_resolution() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct Progress;
    impl ProgressPhase for Progress {
        fn order(self) -> u32 {
            0
        }
        fn stable_id(self) -> &'static str {
            "materialize"
        }
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct Unit;
    impl ProgressUnit for Unit {
        fn stable_id(self) -> &'static str {
            "pieces"
        }
    }
    #[derive(Debug)]
    enum Failure {
        Runtime(StoreInvocationError),
        Owned(Rc<u8>),
    }
    impl From<StoreInvocationError> for Failure {
        fn from(cause: StoreInvocationError) -> Self {
            Self::Runtime(cause)
        }
    }

    let catalog = StoreSemanticCatalogId::new(*b"frame-order-test", NonZeroU32::MIN);
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let phase = domain
        .try_admit_phase(crate::StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
        .unwrap();
    let [parent_key, child_key, internal_key] = [1, 2, 3].map(|local| {
        domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    });
    let limits = LedgerLimits::new(
        u64::MAX,
        [parent_key, child_key, internal_key].map(|key| (key, DimensionPolicy::counter(8, 8, 0))),
    )
    .unwrap();
    let mut coordinator = OperationCoordinator::<Owner, _, Progress, Unit, _>::try_new(
        OperationControl::default(),
        limits,
        &[phase],
    )
    .unwrap();
    let factory =
        PhysicalStorageAccountFactory::try_new_for_live_accounts(NonZeroU64::MIN).unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(1 << 20, 8).unwrap())
        .unwrap();
    let failure = Cell::new(None);
    let debt = Cell::new(0);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let share_work = RetainedHandleSharePlan::new(internal_key, internal_key);
    let (poll, progress, ledger) = coordinator.split_exclusive_progress_and_ledger();
    let (work, storage) = ledger.exclusive().unwrap();
    let share_layout = work.bind(share_work.charges().map(|(key, _)| key)).unwrap();
    let mut context = StoreInvocation {
        parts: ExclusiveOperationParts {
            work,
            progress,
            ordinary: crate::retained::ExecutionOrdinaryStorage {
                storage,
                poll,
                failure: &failure,
                debt: &debt,
                execution_state: &execution_state,
            },
            account: &account,
            share_work,
        },
        domain: &domain,
        share_layout,
        boundary: None,
        brand: PhantomData,
        local: PhantomData,
    };
    context.checkpoint().unwrap();
    let parent_layout = context.bind_group([parent_key]).unwrap();
    let child_layout = context.bind_group([child_key]).unwrap();
    let semantic_owner = Rc::new(41);
    let materialized = Cell::new(0);
    let mut physical = [0; 3];
    let result = context.parser_parent::<_, Failure, 1>(parent_layout, [1], [None], |parent| {
        assert!(parent.host.frame.active);
        assert_eq!(parent.host.frame.remaining, [1]);
        let result = parent.child::<_, Failure, 1>(child_layout, [2], [None], |child| {
            child.run_prefix([1], 7, || {
                physical[0] = 17;
                Ok::<_, Failure>(())
            })?;
            let result = child.run_prefix_with_failure(
                [1],
                11,
                || -> Result<(), Rc<u8>> {
                    physical[1] = 29;
                    Err(Rc::clone(&semantic_owner))
                },
                |host, cause| {
                    assert!(
                        host.host.frame.active,
                        "materialization must precede frame resolution"
                    );
                    assert_eq!(host.host.frame.remaining, [1]);
                    // Parser suffixes are frame metadata; they do not reserve pending_consumed.
                    assert_eq!(host.work_counters(child_key).unwrap(), (2, 1, 0));
                    assert_eq!(host.work_counters(parent_key).unwrap(), (1, 0, 0));
                    assert_eq!(host.work_counters(internal_key).unwrap(), (0, 0, 0));
                    materialized.set(materialized.get() + 1);
                    Failure::Owned(cause)
                },
            );
            assert!(
                !child.host.frame.active,
                "the failed frame resolves before returning"
            );
            assert_eq!(child.host.frame.remaining, [1]);
            result?;
            physical[2] = 1;
            Ok(())
        });
        assert!(parent.host.frame.active);
        assert_eq!(parent.host.frame.remaining, [1]);
        assert_eq!(parent.work_counters(parent_key).unwrap(), (1, 0, 0));
        assert_eq!(parent.work_counters(child_key).unwrap(), (2, 1, 0));
        result
    });
    context.parts.work.check_quiescent().unwrap();
    assert_eq!(context.work_counters(parent_key).unwrap(), (1, 0, 0));
    assert_eq!(context.work_counters(child_key).unwrap(), (2, 1, 0));
    assert_eq!(context.work_counters(internal_key).unwrap(), (0, 0, 0));
    match result {
        Err(Failure::Owned(identity)) => {
            assert!(Rc::ptr_eq(&identity, &semantic_owner));
            assert_eq!(*identity, 41);
        }
        Err(Failure::Runtime(cause)) => panic!("runtime cleanup replaced the owned error: {cause}"),
        Ok(()) => panic!("the failed physical piece cannot complete successfully"),
    }
    assert_eq!(physical, [17, 29, 0]);
    assert_eq!(materialized.get(), 1);
    assert_eq!(execution_state.get(), StoreExecutionState::KnownError);
    assert_eq!(failure.get(), None);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn parser_completion_failure_after_real_write_resolves_suffix_without_abandonment() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, mut storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let frame = layout
        .begin_parser(&mut exclusive.work, [2], [None], ParserChannel::Child)
        .unwrap();
    // Tightening uses the actual owner before borrowing its action guard. No outside alias.
    exclusive
        .tighten_policy(&mut storage, key, DimensionPolicy::counter(8, 0, 0))
        .unwrap();
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let mut frame = frame;
    let mut host = ParserActionHost {
        work: &mut exclusive.work,
        frame: &mut frame,
        layout,
        failure: &failure,
        execution_state: &execution_state,
        boundary: None,
        armed: true,
    };
    let mut physical = [0_u8; 2];
    physical[0] = 37;
    assert!(matches!(
        host.complete([1]),
        Err(LedgerCause::LimitExceeded {
            class: crate::CounterClass::Consumed,
            value: 1,
            limit: 0,
            ..
        })
    ));
    assert!(!host.frame.active);
    assert_eq!(host.frame.remaining, [2]);
    drop(host);
    assert_eq!(execution_state.get(), StoreExecutionState::Running);
    assert_eq!(physical, [37, 0]);
    assert_eq!(exclusive.counters(key).unwrap(), (2, 0, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(failure.get(), Some(PreparationFailureCode::ledger()));
}

#[test]
fn scan_exhausted_consumes_the_actual_chunk_and_one_import_channel_after_due_poll() {
    with_parser_operation(|operation, key, polls| {
        let boundary = StoreImporterBoundary::default();
        assert!(boundary.begin());
        let mut before_scan = 0;
        let mut classified = 0;
        let mut following = 0;
        operation
            .with_parser_execution::<_, StoreInvocationError>(&boundary, |context| {
                let layout = context.bind_group([key])?;
                for _ in 0..256 {
                    context.parser_child::<_, StoreInvocationError, 1>(
                        layout,
                        [1],
                        [None],
                        |child| {
                            child.run_prefix([1], 1, || {
                                before_scan += 1;
                                Ok::<_, StoreInvocationError>(())
                            })
                        },
                    )?;
                }
                assert_eq!(context.work_counters(key)?, (256, 256, 0));
                assert_eq!(context.parts.ordinary.debt.get(), 256);
                assert_eq!(polls.load(Ordering::SeqCst), 1);
                let input = b"ab";
                let end = context.parser_scan_child::<_, StoreInvocationError, 1>(
                    layout,
                    [2],
                    [None],
                    |scan| {
                        assert_eq!(polls.load(Ordering::SeqCst), 2);
                        for (index, byte) in input.iter().copied().enumerate() {
                            let newline = scan.run_prefix([1], 1, || {
                                classified += 1;
                                Ok::<_, StoreInvocationError>(byte == b'\n')
                            })?;
                            if newline {
                                return Ok(StoreScanReturn::Boundary(index + 1));
                            }
                        }
                        Ok(StoreScanReturn::Exhausted(input.len()))
                    },
                )?;
                assert_eq!(end, input.len());
                assert_eq!(context.work_counters(key)?, (258, 258, 0));
                assert_eq!(context.parts.ordinary.debt.get(), 1);
                context.parts.work.check_quiescent().unwrap();
                context.parser_child::<_, StoreInvocationError, 1>(
                    layout,
                    [1],
                    [None],
                    |child| {
                        child.run_prefix([1], 1, || {
                            following += 1;
                            Ok::<_, StoreInvocationError>(())
                        })
                    },
                )?;
                assert_eq!(context.work_counters(key)?, (259, 259, 0));
                assert_eq!(context.parts.ordinary.debt.get(), 2);
                assert_eq!(context.parts.ordinary.failure.get(), None);
                assert_eq!(polls.load(Ordering::SeqCst), 2);
                context.prescribed_exit_checkpoint()
            })
            .unwrap();
        boundary.finish_returned(false);
        assert_eq!(before_scan, 256);
        assert_eq!(classified, 2);
        assert_eq!(following, 1);
        assert_eq!(polls.load(Ordering::SeqCst), 3);
    });
}

#[test]
fn a_scan_boundary_settles_caught_import_unwind_before_denying_the_next_child() {
    with_parser_operation(|operation, key, polls| {
        let boundary = StoreImporterBoundary::default();
        assert!(boundary.begin());
        let mut caught = false;
        let mut following = false;
        let result =
            operation.with_parser_execution::<_, StoreInvocationError>(&boundary, |context| {
                let layout = context.bind_group([key])?;
                let result = context.parser_scan_child::<_, StoreInvocationError, 1>(
                    layout,
                    [2],
                    [None],
                    |scan| {
                        scan.run_prefix([1], 1, || Ok::<_, StoreInvocationError>(()))?;
                        let panic = catch_unwind(AssertUnwindSafe(|| {
                            scan.run_prefix([1], 1, || -> Result<(), StoreInvocationError> {
                                panic!("physical import text classification panic");
                            })
                        }));
                        assert!(panic.is_err());
                        assert_eq!(
                            scan.host.context.parts.ordinary.execution_state.get(),
                            StoreExecutionState::UnwindPending
                        );
                        assert_eq!(scan.host.context.parts.ordinary.failure.get(), None);
                        assert_eq!(scan.work_counters(key)?, (2, 1, 0));
                        caught = true;
                        Ok(StoreScanReturn::Boundary(()))
                    },
                );
                assert!(matches!(
                    &result,
                    Err(StoreInvocationError::Runtime(
                        StoreRuntimeError::FallbackOnly
                    ))
                ));
                assert_eq!(context.work_counters(key)?, (2, 1, 0));
                context.parts.work.check_quiescent().unwrap();
                assert_eq!(
                    context.parts.ordinary.execution_state.get(),
                    StoreExecutionState::UnwindPending
                );
                assert_eq!(
                    context.parts.ordinary.failure.get(),
                    Some(PreparationFailureCode::product_graph(ABANDONED))
                );
                assert!(
                    context
                        .parser_child::<_, StoreInvocationError, 1>(layout, [1], [None], |_child| {
                            following = true;
                            Ok(())
                        })
                        .is_err()
                );
                assert_eq!(context.work_counters(key)?, (2, 1, 0));
                result
            });
        boundary.finish_returned(false);
        assert!(matches!(
            result,
            Err(StoreInvocationError::Runtime(
                StoreRuntimeError::FallbackOnly
            ))
        ));
        assert!(caught);
        assert!(!following);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn borrowed_services_and_child_admission_share_the_real_parent_scan_and_retained_owners() {
    with_parser_operation(|operation, key, polls| {
        let mut physical_calls = 0;
        let mut scanned = 0;
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let catalog = StoreSemanticCatalogId::new(*b"parser-host-test", NonZeroU32::MIN);
                let phase = context
                    .domain
                    .try_admit_phase(crate::StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
                    .unwrap();
                let storage_key = context
                    .domain
                    .try_admit_dimension(StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN))
                    .unwrap();
                let layout = context.child_admission().bind_group([key])?;
                context.parser_parent(
                    layout,
                    [1],
                    [None],
                    |parent| -> Result<(), StoreInvocationError> {
                        parent.child_admission().scan_child(
                            layout,
                            [7],
                            [None],
                            |scan| -> Result<StoreScanReturn<()>, StoreInvocationError> {
                                let mut action = Some(
                                    |_storage: StoreOrdinaryStorage<
                                        '_,
                                        '_,
                                        Owner,
                                        ParserProgress,
                                        ParserUnit,
                                    >| {
                                        physical_calls += 1;
                                        let completed = b"ab\nrest"
                                            .iter()
                                            .take_while(|byte| {
                                                scanned += 1;
                                                **byte != b'\n'
                                            })
                                            .count()
                                            + 1;
                                        Ok::<_, StoreInvocationError>((
                                            completed,
                                            [completed as u64],
                                        ))
                                    },
                                );
                                let completed = scan.run_returned_prefix_with_storage(
                                    1,
                                    &mut action,
                                    |cause| cause,
                                    |_, cause| cause,
                                )?;
                                assert_eq!(completed, 3);
                                assert_eq!(scan.work_counters(key)?, (8, 3, 0));
                                let before = polls.load(Ordering::SeqCst);
                                {
                                    let mut services = scan.services();
                                    services.report_progress(
                                        ParserProgress,
                                        ParserUnit,
                                        3,
                                        Some(7),
                                    )?;
                                    let mut retained =
                                        services.reserve_retained(phase, storage_key, 8, 1)?;
                                    assert_eq!(retained.raw.bytes(), 8);
                                    assert!(retained.physical.is_some());
                                    services.resize(&mut retained, 4)?;
                                    assert_eq!(retained.raw.bytes(), 4);
                                    retained.release_storage([17_u8; 4]);
                                    services.charge_attempted([key], [1])?;
                                    services.complete_observed([key], [1])?;
                                }
                                assert_eq!(polls.load(Ordering::SeqCst) - before, 3);
                                assert_eq!(scan.work_counters(key)?, (9, 4, 0));
                                Ok(StoreScanReturn::Boundary(()))
                            },
                        )?;
                        parent.complete_parent()
                    },
                )?;
                assert_eq!(context.work_counters(key)?, (9, 5, 0));
                context
                    .child_admission()
                    .child(layout, [1], [None], |child| {
                        child.run_prefix([1], 1, || Ok::<_, StoreInvocationError>(()))
                    })?;
                assert_eq!(context.work_counters(key)?, (10, 6, 0));
                context.check_work_quiescent()?;
                Ok(())
            })
            .unwrap();
        assert_eq!((physical_calls, scanned), (1, 3));
    });
}

#[test]
fn returned_parser_prefix_excess_stops_the_actual_frame_without_fabricating_completion() {
    with_parser_operation(|operation, key, _polls| {
        let mut physical_calls = 0;
        let mut following = false;
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([key])?;
            let result = context.parser_child(layout, [2], [None], |child| {
                child.run_prefix([1], 1, || Ok::<_, StoreInvocationError>(()))?;
                let mut action = Some(
                    |_storage: StoreOrdinaryStorage<'_, '_, Owner, ParserProgress, ParserUnit>| {
                        physical_calls += 1;
                        Ok::<_, StoreInvocationError>(((), [2]))
                    },
                );
                child.run_returned_prefix_with_storage(
                    1,
                    &mut action,
                    |cause| cause,
                    |_, cause| cause,
                )
            });
            assert!(result.is_err());
            assert_eq!(context.work_counters(key)?, (2, 1, 0));
            context.parts.work.check_quiescent().unwrap();
            assert_eq!(
                context.parts.ordinary.failure.get(),
                Some(PreparationFailureCode::ledger())
            );
            assert!(
                context
                    .child_admission()
                    .child(
                        layout,
                        [1],
                        [None],
                        |_| -> Result<(), StoreInvocationError> {
                            following = true;
                            Ok(())
                        }
                    )
                    .is_err()
            );
            result
        });
        assert!(result.is_err());
        assert_eq!(physical_calls, 1);
        assert!(!following);
    });
}

#[test]
fn fallible_chunks_and_normally_returned_io_errors_keep_distinct_real_completion_policies() {
    with_parser_operation(|operation, key, _polls| {
        let cause = Rc::new(17_u8);
        let mut calls = 0;
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([key])?;
                let failure = context
                    .run_fallible_action(layout, [1], 1, || {
                        calls += 1;
                        Err::<(), _>(Rc::clone(&cause))
                    })?
                    .unwrap_err();
                assert!(Rc::ptr_eq(&failure, &cause));
                assert_eq!(context.work_counters(key)?, (1, 0, 0));
                assert_eq!(context.parts.ordinary.failure.get(), None);
                let returned_io_error = context
                    .run_action(layout, [1], 1, || {
                        calls += 1;
                        Err::<(), _>(Rc::clone(&cause))
                    })?
                    .unwrap_err();
                assert!(Rc::ptr_eq(&returned_io_error, &cause));
                assert_eq!(context.work_counters(key)?, (2, 1, 0));
                let value = context
                    .run_fallible_action(layout, [1], 1, || {
                        calls += 1;
                        Ok::<_, Rc<u8>>(29_u8)
                    })?
                    .unwrap();
                assert_eq!(value, 29);
                assert_eq!(context.work_counters(key)?, (3, 2, 0));
                context.parts.work.check_quiescent().unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(calls, 3);
    });
}

#[test]
fn a_fallible_chunk_panic_reaches_the_actual_caller_catch_and_blocks_reuse() {
    with_parser_operation(|operation, key, _polls| {
        let mut calls = 0;
        let mut following = false;
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([key])?;
            context
                .run_fallible_action(layout, [1], 1, || Ok::<_, ()>(()))?
                .unwrap();
            let panic = catch_unwind(AssertUnwindSafe(|| {
                context.run_fallible_action(layout, [1], 1, || -> Result<(), ()> {
                    calls += 1;
                    panic!("the actual detector callback panicked");
                })
            }));
            assert!(panic.is_err());
            assert_eq!(context.work_counters(key)?, (2, 1, 0));
            context.parts.work.check_quiescent().unwrap();
            assert_eq!(
                context.parts.ordinary.failure.get(),
                Some(PreparationFailureCode::product_graph(ABANDONED))
            );
            let result = context.run_fallible_action(layout, [1], 1, || {
                following = true;
                Ok::<_, ()>(())
            });
            assert!(result.is_err());
            assert_eq!(context.work_counters(key)?, (2, 1, 0));
            result.map(|_| ())
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);
        assert!(!following);
    });
}

#[test]
fn observed_root_calls_preserve_their_explicit_poll_policy_and_bound_numeric_prefixes() {
    with_parser_operation(|operation, key, polls| {
        let mut calls = 0;
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([key])?;
                context.run_action(layout, [1], 7, || ())?;
                let debt = context.parts.ordinary.debt.get();
                let before = polls.load(Ordering::SeqCst);
                assert_eq!(debt, 7);
                let value = context.run_observed_action(layout, [1], || {
                    calls += 1;
                    Err::<(), _>("returned-io-error")
                })?;
                assert_eq!(value, Err("returned-io-error"));
                let value = context.run_observed_fallible_action(layout, [1], || {
                    calls += 1;
                    Err::<(), _>("detector-error")
                })?;
                assert_eq!(value, Err("detector-error"));
                assert_eq!(context.parts.ordinary.debt.get(), debt);
                assert_eq!(polls.load(Ordering::SeqCst), before);
                context.charge_attempted_bound(layout, [2])?;
                context.complete_observed_bound(layout, [1])?;
                context.complete_direct_bound(layout, [1])?;
                context.charge_observed_bound(layout, [1])?;
                let snapshot = context.dimension_state(key)?;
                assert_eq!((snapshot.attempted(), snapshot.consumed()), (6, 5));
                assert_eq!(context.work_counters(key)?, (6, 5, 0));
                context.parts.work.check_quiescent().unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(calls, 2);
    });
}

#[test]
fn ordinary_root_loans_preserve_entry_and_explicit_control_frontiers() {
    with_parser_operation(|operation, key, polls| {
        let boundary = StoreImporterBoundary::default();
        operation
            .with_root_execution::<_, StoreInvocationError>(&boundary, |context| {
                let layout = context.bind_group([key])?;
                context.run_action(layout, [1], 7, || ())?;
                assert_eq!(context.parts.ordinary.debt.get(), 7);
                assert_eq!(polls.load(Ordering::SeqCst), 1);
                Ok(())
            })
            .unwrap();
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        operation
            .with_root_execution::<_, StoreInvocationError>(&boundary, |context| {
                assert_eq!(context.parts.ordinary.debt.get(), 0);
                context.checkpoint()?;
                assert_eq!(context.parts.ordinary.debt.get(), 0);
                assert_eq!(context.work_counters(key)?, (1, 1, 0));
                Ok(())
            })
            .unwrap();
        assert_eq!(polls.load(Ordering::SeqCst), 3);
        operation
            .with_parser_execution::<_, StoreInvocationError>(&boundary, |_context| Ok(()))
            .unwrap();
        assert_eq!(polls.load(Ordering::SeqCst), 4);
    });
}

#[test]
fn ordinary_root_denial_returns_the_original_ledger_error_without_an_exit_poll() {
    with_parser_operation(|operation, key, polls| {
        let boundary = StoreImporterBoundary::default();
        let denied =
            operation.with_root_execution::<_, StoreInvocationError>(&boundary, |context| {
                let layout = context.bind_group([key])?;
                context.charge_attempted_bound(layout, [513])
            });
        assert!(matches!(
            denied,
            Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                LedgerCause::LimitExceeded {
                    dimension,
                    value: 513,
                    limit: 512,
                    ..
                }
            ))) if dimension == key
        ));
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        let mut following = false;
        assert!(
            operation
                .with_root_execution::<_, StoreInvocationError>(&boundary, |_context| {
                    following = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(!following);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn ordinary_root_preserves_a_caught_extension_result_until_the_next_real_frontier() {
    with_parser_operation(|operation, key, polls| {
        let boundary = StoreImporterBoundary::default();
        let result = operation
            .with_root_execution::<_, StoreInvocationError>(&boundary, |context| {
                let layout = context.bind_group([key])?;
                let panic = catch_unwind(AssertUnwindSafe(|| {
                    context.run_observed_fallible_action(layout, [2], || -> Result<(), ()> {
                        panic!("actual detector callback panic")
                    })
                }));
                assert!(panic.is_err());
                assert_eq!(context.work_counters(key)?, (2, 0, 0));
                context.parts.work.check_quiescent().unwrap();
                Ok("extension-fault")
            })
            .unwrap();
        assert_eq!(result, "extension-fault");
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        let mut following = false;
        assert!(
            operation
                .with_root_execution::<_, StoreInvocationError>(&boundary, |_| {
                    following = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(!following);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn borrowed_storage_observations_follow_atomic_transitions_and_actual_token_release() {
    with_parser_operation(|operation, _key, _polls| {
        operation
            .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let catalog = StoreSemanticCatalogId::new(*b"parser-host-test", NonZeroU32::MIN);
                let phase = context
                    .domain
                    .try_admit_phase(crate::StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
                    .unwrap();
                let storage_key = context
                    .domain
                    .try_admit_dimension(StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN))
                    .unwrap();
                let observation = context.storage_observation(storage_key)?;
                let baseline = observation.live_bytes(context);
                let mut reservation = context.ordinary_storage().reserve(phase, storage_key, 8)?;
                assert_eq!(observation.live_bytes(context), baseline + 8);
                {
                    let services = context.services();
                    services.validate_live_owner(&reservation)?;
                    services.transition(&mut reservation, phase, storage_key, 4)?;
                }
                assert_eq!(reservation.raw.bytes(), 4);
                assert_eq!(observation.live_bytes(context), baseline + 4);
                reservation.release_storage([0_u8; 4]);
                assert_eq!(observation.live_bytes(context), baseline);
                Ok(())
            })
            .unwrap();
    });
}

#[test]
fn borrowed_storage_transition_keeps_the_original_retained_output_rejection() {
    with_parser_operation(|operation, _key, _polls| {
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let catalog = StoreSemanticCatalogId::new(*b"parser-host-test", NonZeroU32::MIN);
            let phase = context
                .domain
                .try_admit_phase(crate::StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
                .unwrap();
            let storage_key = context
                .domain
                .try_admit_dimension(StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN))
                .unwrap();
            let observation = context.storage_observation(storage_key)?;
            let baseline = observation.live_bytes(context);
            let mut retained = context
                .services()
                .reserve_retained(phase, storage_key, 8, 1)?;
            let before = observation.live_bytes(context);
            let result = context
                .services()
                .transition(&mut retained, phase, storage_key, 4);
            assert!(matches!(
                &result,
                Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                    LedgerCause::OwnerMismatch
                )))
            ));
            assert_eq!(retained.raw.bytes(), 8);
            assert_eq!(retained.physical.as_ref().unwrap().admitted_bytes(), 8);
            assert_eq!(observation.live_bytes(context), before);
            retained.release_storage([0_u8; 8]);
            assert_eq!(observation.live_bytes(context), baseline);
            result
        });
        assert!(matches!(
            result,
            Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                LedgerCause::OwnerMismatch
            )))
        ));
    });
}
