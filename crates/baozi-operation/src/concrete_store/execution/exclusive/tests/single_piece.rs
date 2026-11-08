use super::*;

#[test]
fn aliased_single_piece_matches_the_original_child_prefix() {
    for fused in [false, true] {
        with_parser_operation(|operation, key, polls| {
            let physical = Cell::new(0);
            let result = operation
                .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                    let layout = context.bind_group([key, key])?;
                    let mut action = Some(
                        |_storage: StoreOrdinaryStorage<
                            '_,
                            '_,
                            Owner,
                            ParserProgress,
                            ParserUnit,
                        >| {
                            physical.set(physical.get() + 1);
                            Ok::<_, StoreInvocationError>(([17_u8, 29], [1, 2]))
                        },
                    );
                    let result = if fused {
                        context.child_admission().run_piece_with_storage(
                            layout,
                            [1, 2],
                            [Some(1), Some(2)],
                            &mut action,
                            |cause| cause,
                            |_, cause| cause,
                        )
                    } else {
                        context.parser_child(layout, [1, 2], [Some(1), Some(2)], |child| {
                            child.run_returned_prefix_with_storage(
                                1,
                                &mut action,
                                |cause| cause,
                                |_, cause| cause,
                            )
                        })
                    }?;
                    assert!(action.is_none());
                    assert_eq!(context.work_counters(key)?, (3, 3, 0));
                    context.parts.work.check_quiescent().unwrap();
                    assert_eq!(context.parts.ordinary.debt.get(), 1);
                    assert_eq!(context.parts.ordinary.failure.get(), None);
                    assert_eq!(polls.load(Ordering::SeqCst), 1);
                    Ok(result)
                })
                .unwrap();
            assert_eq!(result, [17, 29]);
            assert_eq!(physical.get(), 1);
            assert_eq!(polls.load(Ordering::SeqCst), 2);
        });
    }
}

#[test]
fn partial_single_piece_stays_incomplete_even_when_its_error_is_ignored() {
    for registry in [false, true] {
        for fused in [false, true] {
            with_parser_operation(|operation, key, polls| {
                let boundary = StoreImporterBoundary::default();
                assert!(boundary.begin());
                let physical = Cell::new(0);
                let following = Cell::new(false);
                let invoke = |context: &mut StoreInvocation<
                    '_,
                    Owner,
                    ParserProgress,
                    ParserUnit,
                >|
                 -> Result<(), StoreInvocationError> {
                    let layout = context.bind_group([key])?;
                    let mut action = Some(
                        |_storage: StoreOrdinaryStorage<
                            '_,
                            '_,
                            Owner,
                            ParserProgress,
                            ParserUnit,
                        >| {
                            physical.set(physical.get() + 1);
                            Ok::<_, StoreInvocationError>(((), [1]))
                        },
                    );
                    let result = if fused {
                        context.child_admission().run_piece_with_storage(
                            layout,
                            [2],
                            [None],
                            &mut action,
                            |cause| cause,
                            |_, cause| cause,
                        )
                    } else {
                        context.parser_child(layout, [2], [None], |child| {
                            child.run_returned_prefix_with_storage(
                                1,
                                &mut action,
                                |cause| cause,
                                |_, cause| cause,
                            )
                        })
                    };
                    assert!(matches!(
                        result,
                        Err(StoreInvocationError::Runtime(
                            StoreRuntimeError::ObservedWorkIncomplete
                        ))
                    ));
                    assert_eq!(context.work_counters(key)?, (2, 1, 0));
                    context.parts.work.check_quiescent().unwrap();
                    assert_eq!(
                        context.parts.ordinary.execution_state.get(),
                        StoreExecutionState::IncompleteSuccess
                    );
                    assert_eq!(context.parts.ordinary.debt.get(), u16::from(!registry));
                    assert_eq!(
                        context.parts.ordinary.failure.get(),
                        if registry {
                            None
                        } else {
                            Some(PreparationFailureCode::product_graph(ABANDONED))
                        }
                    );
                    assert!(
                        context
                            .run_action(layout, [1], 1, || following.set(true))
                            .is_err()
                    );
                    assert_eq!(context.work_counters(key)?, (2, 1, 0));
                    Ok(())
                };
                let result = if registry {
                    operation.with_parser_execution(&boundary, invoke)
                } else {
                    operation.with_exclusive_execution(invoke)
                };
                boundary.finish_returned(false);
                assert!(matches!(
                    result,
                    Err(StoreInvocationError::Runtime(
                        StoreRuntimeError::FallbackOnly
                    ))
                ));
                assert_eq!(physical.get(), 1);
                assert!(!following.get());
                assert_eq!(polls.load(Ordering::SeqCst), 1);
            });
        }
    }
}

#[test]
fn excess_single_piece_projects_before_output_drop_and_keeps_prior_completion() {
    struct Output<'a> {
        projected: &'a Cell<bool>,
        observed: &'a Cell<bool>,
        drops: &'a Cell<usize>,
        panic: bool,
    }

    impl Drop for Output<'_> {
        fn drop(&mut self) {
            self.observed.set(self.projected.get());
            self.drops.set(self.drops.get() + 1);
            assert!(!self.panic, "rejected single-piece output destructor");
        }
    }

    for fused in [false, true] {
        for panic in [false, true] {
            with_parser_operation(|operation, key, polls| {
                let projected = Cell::new(false);
                let observed = Cell::new(false);
                let drops = Cell::new(0);
                let physical = Cell::new(0);
                let following = Cell::new(false);
                let result =
                    operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
                        let layout = context.bind_group([key])?;
                        context.parser_child(layout, [1], [None], |child| {
                            child.run_prefix([1], 1, || Ok::<_, StoreInvocationError>(()))
                        })?;
                        let mut action = Some(
                            |_storage: StoreOrdinaryStorage<
                                '_,
                                '_,
                                Owner,
                                ParserProgress,
                                ParserUnit,
                            >| {
                                physical.set(physical.get() + 1);
                                Ok::<_, StoreInvocationError>((
                                    Output {
                                        projected: &projected,
                                        observed: &observed,
                                        drops: &drops,
                                        panic,
                                    },
                                    [3],
                                ))
                            },
                        );
                        let project = |cause| {
                            projected.set(matches!(
                                &cause,
                                StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                                    LedgerCause::InactiveLinearWorkPermit
                                ))
                            ));
                            cause
                        };
                        let outcome = catch_unwind(AssertUnwindSafe(|| {
                            let result = if fused {
                                context.child_admission().run_piece_with_storage(
                                    layout,
                                    [2],
                                    [None],
                                    &mut action,
                                    project,
                                    |_, cause| cause,
                                )
                            } else {
                                context.parser_child(layout, [2], [None], |child| {
                                    child.run_returned_prefix_with_storage(
                                        1,
                                        &mut action,
                                        project,
                                        |_, cause| cause,
                                    )
                                })
                            };
                            result.map(drop)
                        }));
                        assert_eq!(outcome.is_err(), panic);
                        if let Ok(result) = outcome {
                            assert!(matches!(
                                result,
                                Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                                    LedgerCause::InactiveLinearWorkPermit
                                )))
                            ));
                        }
                        assert!(projected.get());
                        assert!(observed.get());
                        assert_eq!(drops.get(), 1);
                        assert_eq!(context.work_counters(key)?, (3, 1, 0));
                        context.parts.work.check_quiescent().unwrap();
                        assert_eq!(
                            context.parts.ordinary.failure.get(),
                            Some(PreparationFailureCode::ledger())
                        );
                        assert!(
                            context
                                .run_action(layout, [1], 1, || following.set(true))
                                .is_err()
                        );
                        assert_eq!(context.work_counters(key)?, (3, 1, 0));
                        Ok(())
                    });
                assert!(result.is_err());
                assert_eq!(physical.get(), 1);
                assert!(!following.get());
                assert_eq!(polls.load(Ordering::SeqCst), 1);
            });
        }
    }
}

#[test]
fn an_externally_caught_single_piece_panic_cannot_reopen_its_channel() {
    for fused in [false, true] {
        with_parser_operation(|operation, key, polls| {
            let physical = Cell::new(0);
            let following = Cell::new(false);
            let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
                let layout = context.bind_group([key])?;
                context.parser_child(layout, [1], [None], |child| {
                    child.run_prefix([1], 1, || {
                        physical.set(1);
                        Ok::<_, StoreInvocationError>(())
                    })
                })?;
                let mut action =
                    Some(
                        |_storage: StoreOrdinaryStorage<
                            '_,
                            '_,
                            Owner,
                            ParserProgress,
                            ParserUnit,
                        >|
                         -> Result<((), [u64; 1]), StoreInvocationError> {
                            physical.set(2);
                            panic!("single physical piece interrupted before returning amounts");
                        },
                    );
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    if fused {
                        context.child_admission().run_piece_with_storage(
                            layout,
                            [2],
                            [None],
                            &mut action,
                            |cause| cause,
                            |_, cause| cause,
                        )
                    } else {
                        context.parser_child(layout, [2], [None], |child| {
                            child.run_returned_prefix_with_storage(
                                1,
                                &mut action,
                                |cause| cause,
                                |_, cause| cause,
                            )
                        })
                    }
                }));
                assert!(outcome.is_err());
                assert!(action.is_none());
                assert_eq!(context.work_counters(key)?, (3, 1, 0));
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
                        .run_action(layout, [1], 1, || following.set(true))
                        .is_err()
                );
                assert!(context.checkpoint().is_err());
                assert_eq!(context.work_counters(key)?, (3, 1, 0));
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(physical.get(), 2);
            assert!(!following.get());
            assert_eq!(polls.load(Ordering::SeqCst), 1);
        });
    }
}

#[test]
fn single_piece_materializes_the_original_error_while_failure_storage_and_frame_are_live() {
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

    for fused in [false, true] {
        with_parser_operation(|operation, key, _polls| {
            let original = Rc::new(41);
            let materialized = Cell::new(0);
            let following = Cell::new(false);
            let result = operation.with_exclusive_execution::<_, Failure>(|context| {
                let catalog = StoreSemanticCatalogId::new(*b"parser-host-test", NonZeroU32::MIN);
                let phase = context
                    .domain
                    .try_admit_phase(crate::StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
                    .unwrap();
                let storage_key = context
                    .domain
                    .try_admit_dimension(StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN))
                    .unwrap();
                let layout = context.bind_group([key])?;
                context.parser_child(layout, [1], [None], |child| {
                    child.run_prefix([1], 1, || Ok::<_, Failure>(()))
                })?;
                let mut action = Some(
                    |_storage: StoreOrdinaryStorage<'_, '_, Owner, ParserProgress, ParserUnit>| {
                        Err::<((), [u64; 1]), _>(Rc::clone(&original))
                    },
                );
                let materialize = |host: &mut StoreInvocationParser<
                    '_,
                    '_,
                    '_,
                    Owner,
                    ParserProgress,
                    ParserUnit,
                    1,
                    true,
                >,
                                   cause| {
                    assert!(host.host.frame.active);
                    assert_eq!(host.host.frame.remaining, [2]);
                    assert_eq!(host.work_counters(key).unwrap(), (3, 1, 0));
                    assert_eq!(
                        host.host.context.parts.ordinary.execution_state.get(),
                        StoreExecutionState::KnownError
                    );
                    assert!(Rc::ptr_eq(&cause, &original));
                    let retained = host
                        .failure_storage()
                        .reserve(phase, storage_key, 8, 1)
                        .unwrap();
                    assert_eq!(retained.raw.bytes(), 8);
                    retained.release_storage([0_u8; 8]);
                    assert!(host.host.frame.active);
                    materialized.set(materialized.get() + 1);
                    Failure::Owned(cause)
                };
                let result = if fused {
                    context.child_admission().run_piece_with_storage(
                        layout,
                        [2],
                        [None],
                        &mut action,
                        Failure::from,
                        materialize,
                    )
                } else {
                    context.parser_child(layout, [2], [None], |child| {
                        child.run_returned_prefix_with_storage(
                            1,
                            &mut action,
                            Failure::from,
                            materialize,
                        )
                    })
                };
                context.parts.work.check_quiescent().unwrap();
                assert_eq!(context.work_counters(key)?, (3, 1, 0));
                assert_eq!(context.parts.ordinary.failure.get(), None);
                assert!(
                    context
                        .run_action(layout, [1], 1, || following.set(true))
                        .is_err()
                );
                result
            });
            match result {
                Err(Failure::Owned(cause)) => assert!(Rc::ptr_eq(&cause, &original)),
                Err(Failure::Runtime(cause)) => panic!("original error replaced: {cause}"),
                Ok(()) => panic!("the failed piece returned success"),
            }
            assert_eq!(materialized.get(), 1);
            assert!(!following.get());
        });
    }
}

#[test]
fn denied_single_piece_keeps_captures_borrowed_until_the_projector_latches_the_cause() {
    struct Capture<'a> {
        latched: &'a Cell<Option<PreparationFailureCode>>,
        observed: &'a Cell<Option<PreparationFailureCode>>,
        drops: &'a Cell<usize>,
    }

    impl Drop for Capture<'_> {
        fn drop(&mut self) {
            self.observed.set(self.latched.get());
            self.drops.set(self.drops.get() + 1);
            panic!("uncalled single-piece capture destructor");
        }
    }

    let cancellation = CancellationToken::new();
    with_cancellable_parser_operation(cancellation.clone(), |operation, key, polls| {
        let latched = Cell::new(None);
        let observed = Cell::new(None);
        let drops = Cell::new(0);
        let physical = Cell::new(0);
        let result = operation.with_exclusive_execution::<_, StoreInvocationError>(|context| {
            let layout = context.bind_group([key])?;
            context.parts.ordinary.debt.set(256);
            cancellation.cancel();
            let capture = Capture {
                latched: &latched,
                observed: &observed,
                drops: &drops,
            };
            let physical = &physical;
            let mut action = Some(
                move |_storage: StoreOrdinaryStorage<'_, '_, Owner, ParserProgress, ParserUnit>| {
                    std::hint::black_box(&capture);
                    physical.set(physical.get() + 1);
                    Ok::<_, StoreInvocationError>(((), [2]))
                },
            );
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                context.child_admission().run_piece_with_storage(
                    layout,
                    [2],
                    [None],
                    &mut action,
                    |cause| {
                        if let StoreInvocationError::Runtime(StoreRuntimeError::Control(control)) =
                            &cause
                        {
                            latched.set(Some(PreparationFailureCode::from_control(control)));
                        }
                        cause
                    },
                    |_, cause| cause,
                )
            }));
            let retained_capture = action.is_some();
            let drops_before_disposal = drops.get();
            let disposal = catch_unwind(AssertUnwindSafe(|| drop(action.take())));
            assert!(matches!(
                outcome,
                Ok(Err(StoreInvocationError::Runtime(
                    StoreRuntimeError::Control(crate::ControlCause::Cancelled {
                        origin: crate::CancellationOrigin::Signal
                    })
                )))
            ));
            assert!(retained_capture);
            assert_eq!(drops_before_disposal, 0);
            assert!(disposal.is_err());
            assert_eq!(drops.get(), 1);
            let original = PreparationFailureCode::from_control(&crate::ControlCause::Cancelled {
                origin: crate::CancellationOrigin::Signal,
            });
            assert_eq!(latched.get(), Some(original));
            assert_eq!(observed.get(), Some(original));
            assert_eq!(context.work_counters(key)?, (2, 0, 0));
            context.parts.work.check_quiescent().unwrap();
            assert_eq!(context.parts.ordinary.failure.get(), Some(original));
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(physical.get(), 0);
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn due_cancellation_keeps_registry_and_direct_admission_in_their_original_order() {
    for registry in [false, true] {
        for fused in [false, true] {
            let cancellation = CancellationToken::new();
            with_cancellable_parser_operation(cancellation.clone(), |operation, key, polls| {
                let boundary = StoreImporterBoundary::default();
                assert!(boundary.begin());
                let physical = Cell::new(0);
                let invoke = |context: &mut StoreInvocation<
                    '_,
                    Owner,
                    ParserProgress,
                    ParserUnit,
                >|
                 -> Result<(), StoreInvocationError> {
                    let layout = context.bind_group([key])?;
                    context.parts.ordinary.debt.set(256);
                    cancellation.cancel();
                    let mut action = Some(
                        |_storage: StoreOrdinaryStorage<
                            '_,
                            '_,
                            Owner,
                            ParserProgress,
                            ParserUnit,
                        >| {
                            physical.set(physical.get() + 1);
                            Ok::<_, StoreInvocationError>(((), [1]))
                        },
                    );
                    let result = if fused {
                        context.child_admission().run_piece_with_storage(
                            layout,
                            [1],
                            [None],
                            &mut action,
                            |cause| cause,
                            |_, cause| cause,
                        )
                    } else {
                        context.parser_child(layout, [1], [None], |child| {
                            child.run_returned_prefix_with_storage(
                                1,
                                &mut action,
                                |cause| cause,
                                |_, cause| cause,
                            )
                        })
                    };
                    assert!(matches!(
                        &result,
                        Err(StoreInvocationError::Runtime(StoreRuntimeError::Control(
                            crate::ControlCause::Cancelled {
                                origin: crate::CancellationOrigin::Signal
                            }
                        )))
                    ));
                    assert!(action.is_some());
                    assert_eq!(context.work_counters(key)?, (u64::from(!registry), 0, 0));
                    context.parts.work.check_quiescent().unwrap();
                    assert_eq!(context.parts.ordinary.debt.get(), 256);
                    assert_eq!(polls.load(Ordering::SeqCst), 2);
                    assert_eq!(
                        context.parts.ordinary.failure.get(),
                        Some(PreparationFailureCode::from_control(
                            &crate::ControlCause::Cancelled {
                                origin: crate::CancellationOrigin::Signal,
                            }
                        ))
                    );
                    assert!(context.checkpoint().is_err());
                    assert_eq!(polls.load(Ordering::SeqCst), 2);
                    result
                };
                let result = if registry {
                    operation.with_parser_execution(&boundary, invoke)
                } else {
                    operation.with_exclusive_execution(invoke)
                };
                boundary.finish_returned(false);
                assert!(result.is_err());
                assert_eq!(physical.get(), 0);
                assert_eq!(polls.load(Ordering::SeqCst), 2);
            });
        }
    }
}

#[test]
fn single_piece_completion_respects_real_internal_pending_and_tighter_consumed_limits() {
    for fused in [false, true] {
        with_parser_operation(|operation, key, _polls| {
            let physical = Cell::new(0);
            let following = Cell::new(false);
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
                context.tighten_dimension_policy(key, DimensionPolicy::counter(512, 4, 0))?;
                let layout = context.bind_group([key])?;
                context.parser_child(layout, [1], [None], |child| {
                    child.run_prefix([1], 1, || Ok::<_, StoreInvocationError>(()))
                })?;
                let mut pending = layout
                    .raw
                    .admit_observed(&mut context.parts.work.work, [2], WorkLane::Internal)
                    .unwrap();
                let mut action = Some(
                    |mut storage: StoreOrdinaryStorage<
                        '_,
                        '_,
                        Owner,
                        ParserProgress,
                        ParserUnit,
                    >| {
                        let reservation = storage.reserve(phase, storage_key, 8)?;
                        reservation.release_storage([7_u8; 8]);
                        physical.set(physical.get() + 1);
                        Ok::<_, StoreInvocationError>(((), [2]))
                    },
                );
                let result = if fused {
                    context.child_admission().run_piece_with_storage(
                        layout,
                        [2],
                        [None],
                        &mut action,
                        |cause| cause,
                        |_, cause| cause,
                    )
                } else {
                    context.parser_child(layout, [2], [None], |child| {
                        child.run_returned_prefix_with_storage(
                            1,
                            &mut action,
                            |cause| cause,
                            |_, cause| cause,
                        )
                    })
                };
                assert!(matches!(
                    &result,
                    Err(StoreInvocationError::Runtime(StoreRuntimeError::Ledger(
                        LedgerCause::LimitExceeded {
                            class: crate::CounterClass::Consumed,
                            value: 5,
                            limit: 4,
                            ..
                        }
                    )))
                ));
                assert_eq!(context.work_counters(key)?, (5, 1, 2));
                assert!(context.parts.work.check_quiescent().is_err());
                assert!(pending.active);
                pending
                    .resolve(&layout.raw, &mut context.parts.work.work, false)
                    .unwrap();
                assert_eq!(context.work_counters(key)?, (5, 1, 0));
                context.parts.work.check_quiescent().unwrap();
                assert_eq!(
                    context.parts.ordinary.failure.get(),
                    Some(PreparationFailureCode::ledger())
                );
                assert!(
                    context
                        .run_action(layout, [1], 1, || following.set(true))
                        .is_err()
                );
                result
            });
            assert!(result.is_err());
            assert_eq!(physical.get(), 1);
            assert!(!following.get());
        });
    }
}
