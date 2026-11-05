use super::super::producer_host::operation_with_producer_dimensions;
use super::*;
use baozi_operation::{CancellationOrigin, CapacityPlan, ControlCause, CounterClass};
use std::mem::ManuallyDrop;

#[test]
fn typed_storage_rejects_the_whole_work_envelope_before_the_build_callback() {
    for (policy, class) in [
        (DimensionPolicy::counter(4, 4, 0), CounterClass::Attempted),
        (DimensionPolicy::counter(8, 4, 0), CounterClass::Consumed),
    ] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
        let mut body_keys = None;
        let finish = invocation_host
            .run_body(
                |CanonicalBodyParts {
                     startup,
                     output,
                     activation,
                     ..
                 },
                 disposition| {
                    let (mut operation, keys) = operation_with_producer_dimensions(
                        startup,
                        &account,
                        OperationControl::default(),
                        DimensionPolicy::counter(8, 8, 0),
                    );
                    operation
                        .producer_host()
                        .tighten_dimension_policy(keys.other_dimension, policy)
                        .unwrap();
                    let prepared_dropped = Cell::new(false);
                    let builder_called = Cell::new(false);
                    let capacity = CapacityPlan::for_type::<u8>(0, 0, 4).unwrap();
                    let outcome = plan_scoped_activation!(
                        operation,
                        output,
                        activation,
                        scoped_activation_shape_with_dimensions(
                            keys,
                            1,
                            1,
                            keys.other_dimension,
                            keys.dimension,
                            keys.dimension,
                            keys.dimension,
                        ),
                        |mut lane| {
                            let baseline = account.snapshot();
                            let cause = lane
                                .try_retain_storage_prepared(
                                    [capacity],
                                    4,
                                    0,
                                    |_| {
                                        Ok((
                                            DropProbe(&prepared_dropped),
                                            String::with_capacity(capacity.envelope_capacity()),
                                        ))
                                    },
                                    |(_probe, value), _control| {
                                        builder_called.set(true);
                                        Ok(value)
                                    },
                                )
                                .err()
                                .expect("the complete envelope exceeds the remaining work budget");
                            assert_eq!(
                                cause,
                                StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
                                    class,
                                    dimension: keys.other_dimension,
                                    value: 1 + 4,
                                    limit: 4,
                                })
                            );
                            assert!(!builder_called.get());
                            assert!(prepared_dropped.get());
                            let released = account.snapshot();
                            assert!(!released.poisoned());
                            assert_eq!(
                                released.live_retained_bytes(),
                                baseline.live_retained_bytes()
                            );
                            assert_eq!(
                                released.live_retained_allocations(),
                                baseline.live_retained_allocations()
                            );
                            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                                MaterializationRejection::Semantic,
                                71,
                            )
                        },
                    )
                    .unwrap();
                    body_keys = Some(keys);
                    request_rejected_activation_finish!(outcome, disposition);
                },
            )
            .unwrap();
        let CanonicalInvocationOutcome::Rejected(rejected) = finish.resume().outcome else {
            panic!("denied typed-table work must resume as an actual rejection")
        };
        let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
        let published = prepared.handoff();
        let work = published
            .ledger_evidence
            .dimension(body_keys.unwrap().other_dimension)
            .unwrap();
        assert_eq!((work.attempted(), work.consumed()), (1 + 4, 1));
        assert_eq!(
            store_failure_cause(&published.ledger_evidence).class(),
            StorePreparationFailureClass::Ledger
        );
        assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
        assert_eq!(published.ledger_evidence.header().active_reservations(), 0);
        drop(published);
        drop(invocation_host);
        assert!(!account.snapshot().poisoned());
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}

#[test]
fn typed_storage_forgotten_control_borrow_keeps_exact_partial_and_zero_cleanup_in_the_host() {
    for (admitted, completed, partial) in [
        (2_u64, 2_usize, false),
        (4, 3, true),
        (2, 1, true),
        (0, 0, false),
    ] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
        let mut body_keys = None;
        let finish = invocation_host
            .run_body(
                |CanonicalBodyParts {
                     startup,
                     output,
                     activation,
                     ..
                 },
                 disposition| {
                    let (operation, keys) = operation_with_producer_dimensions(
                        startup,
                        &account,
                        OperationControl::default(),
                        DimensionPolicy::counter(16, 16, 0),
                    );
                    let capacity =
                        CapacityPlan::for_type::<u8>(0, 0, usize::try_from(admitted).unwrap())
                            .unwrap();
                    let outcome = plan_scoped_activation!(
                        operation,
                        output,
                        activation,
                        scoped_activation_shape_with_dimensions(
                            keys,
                            1,
                            1,
                            keys.other_dimension,
                            keys.dimension,
                            keys.dimension,
                            keys.dimension,
                        ),
                        |mut lane| {
                            let baseline = account.snapshot();
                            let result = lane.try_retain_storage(
                                [capacity],
                                admitted,
                                if partial { 0 } else { admitted },
                                |control| {
                                    let mut value =
                                        String::with_capacity(capacity.envelope_capacity());
                                    for _ in 0..completed {
                                        if partial {
                                            control.run_unit(|| value.push('a'))?;
                                        } else {
                                            control.run_string_unit(|| value.push('a'))?;
                                        }
                                    }
                                    let _forgotten_borrow = ManuallyDrop::new(&mut *control);
                                    if partial {
                                        Err(StoreRuntimeError::Build(
                                            StoreBuildError::domain(71).unwrap(),
                                        ))
                                    } else {
                                        Ok(value)
                                    }
                                },
                            );
                            if partial {
                                assert_eq!(
                                    result
                                        .err()
                                        .expect("the builder returns its partial failure"),
                                    StoreRuntimeError::Build(StoreBuildError::domain(71).unwrap())
                                );
                            } else {
                                let storage = result.expect("the exact typed table is retained");
                                assert_eq!(storage.as_ref(), &"a".repeat(completed));
                                assert_eq!(
                                    account.snapshot().live_retained_allocations(),
                                    baseline.live_retained_allocations()
                                        + 1
                                        + u64::from(admitted != 0)
                                );
                                drop(storage);
                            }
                            let released = account.snapshot();
                            assert!(!released.poisoned());
                            assert_eq!(
                                released.live_retained_bytes(),
                                baseline.live_retained_bytes()
                            );
                            assert_eq!(
                                released.live_retained_allocations(),
                                baseline.live_retained_allocations()
                            );
                            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                                MaterializationRejection::Semantic,
                                80,
                            )
                        },
                    )
                    .unwrap();
                    body_keys = Some(keys);
                    request_rejected_activation_finish!(outcome, disposition);
                },
            )
            .unwrap();
        let CanonicalInvocationOutcome::Rejected(rejected) = finish.resume().outcome else {
            panic!("resolved typed-table work must remain terminally publishable")
        };
        let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
        let published = prepared.handoff();
        let work = published
            .ledger_evidence
            .dimension(body_keys.unwrap().other_dimension)
            .unwrap();
        assert_eq!(work.attempted(), 1 + admitted);
        assert_eq!(work.consumed(), 1 + u64::try_from(completed).unwrap());
        assert_eq!(
            store_failure_cause(&published.ledger_evidence).class(),
            if partial {
                StorePreparationFailureClass::ProductGraph
            } else {
                StorePreparationFailureClass::Requested
            }
        );
        assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
        assert_eq!(published.ledger_evidence.header().active_reservations(), 0);
        drop(published);
        drop(invocation_host);
        assert!(!account.snapshot().poisoned());
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}

#[test]
fn typed_storage_rejects_extra_units_before_the_physical_callback() {
    for admitted in [0_u64, 2] {
        let factory = factory(1);
        let account = account(&factory);
        let completed = Cell::new(0_u64);
        let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
        let body = invocation_host.run_body(
            |CanonicalBodyParts {
                 startup,
                 output,
                 activation,
                 ..
             },
             _disposition| {
                let (operation, keys) = operation(startup, &account);
                let capacity =
                    CapacityPlan::for_type::<u8>(0, 0, usize::try_from(admitted).unwrap()).unwrap();
                let outcome = plan_scoped_activation!(
                    operation,
                    output,
                    activation,
                    scoped_activation_shape(keys, 1, 1),
                    |mut lane| {
                        let baseline = account.snapshot();
                        let cause = lane
                            .try_retain_storage([capacity], admitted, 0, |control| {
                                let mut value = Vec::with_capacity(capacity.envelope_capacity());
                                for _ in 0..admitted {
                                    control.run_unit(|| {
                                        value.push(1_u8);
                                        completed.set(completed.get() + 1);
                                    })?;
                                }
                                for _ in 0..2 {
                                    assert_eq!(
                                        control.run_unit(|| {
                                            value.push(2);
                                            completed.set(completed.get() + 1);
                                        }),
                                        Err(StoreRuntimeError::ObservedWorkExceedsAdmitted)
                                    );
                                }
                                Ok(value)
                            })
                            .err()
                            .expect("an ignored extra action cannot retain the table");
                        assert_eq!(cause, StoreRuntimeError::ObservedWorkExceedsAdmitted);
                        assert_eq!(completed.get(), admitted);
                        let released = account.snapshot();
                        assert!(!released.poisoned());
                        assert_eq!(
                            released.live_retained_bytes(),
                            baseline.live_retained_bytes()
                        );
                        assert_eq!(
                            released.live_retained_allocations(),
                            baseline.live_retained_allocations()
                        );
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            74,
                        )
                    },
                );
                let containment = outcome
                    .err()
                    .expect("exceeding typed-table work is a materialization contract fault");
                assert_eq!(
                    containment.cause().reason(),
                    AccountPoisonReason::OwnershipGraphUnverified
                );
                assert!(account.snapshot().poisoned());
            },
        );
        assert!(body.is_err());
        assert_eq!(completed.get(), admitted);
    }
}

#[test]
fn typed_storage_incomplete_units_or_strings_release_the_built_value() {
    for (completed, planned_strings, completed_strings) in [(1_usize, 0_u64, 0_usize), (2, 2, 1)] {
        let factory = factory(1);
        let account = account(&factory);
        let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
        let body = invocation_host.run_body(
            |CanonicalBodyParts {
                 startup,
                 output,
                 activation,
                 ..
             },
             _disposition| {
                let (operation, keys) = operation(startup, &account);
                let capacity = CapacityPlan::for_type::<u8>(0, 0, 2).unwrap();
                let outcome = plan_scoped_activation!(
                    operation,
                    output,
                    activation,
                    scoped_activation_shape(keys, 1, 1),
                    |mut lane| {
                        let baseline = account.snapshot();
                        let cause = lane
                            .try_retain_storage([capacity], 2, planned_strings, |control| {
                                let mut value = String::with_capacity(capacity.envelope_capacity());
                                for index in 0..completed {
                                    if index < completed_strings {
                                        control.run_string_unit(|| value.push('a'))?;
                                    } else {
                                        control.run_unit(|| value.push('b'))?;
                                    }
                                }
                                Ok(value)
                            })
                            .err()
                            .expect("both work and string envelopes must finish exactly");
                        assert_eq!(cause, StoreRuntimeError::ObservedWorkIncomplete);
                        let released = account.snapshot();
                        assert!(!released.poisoned());
                        assert_eq!(
                            released.live_retained_bytes(),
                            baseline.live_retained_bytes()
                        );
                        assert_eq!(
                            released.live_retained_allocations(),
                            baseline.live_retained_allocations()
                        );
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            81,
                        )
                    },
                );
                let containment = outcome
                    .err()
                    .expect("an incomplete typed table cannot escape as a trusted rejection");
                assert_eq!(
                    containment.cause().reason(),
                    AccountPoisonReason::OwnershipGraphUnverified
                );
                assert!(account.snapshot().poisoned());
            },
        );
        assert!(body.is_err());
    }
}

#[test]
fn typed_storage_build_panic_releases_prepared_shared_storage_before_containment() {
    let factory = factory(1);
    let account = account(&factory);
    let completed = Cell::new(0_u64);
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let body = invocation_host.run_body(
        |CanonicalBodyParts {
             startup,
             output,
             activation,
             ..
         },
         _disposition| {
            let (operation, keys) = operation(startup, &account);
            let capacity = CapacityPlan::for_type::<u8>(0, 0, 4).unwrap();
            let outcome = plan_scoped_activation!(
                operation,
                output,
                activation,
                scoped_activation_shape(keys, 1, 1),
                |mut lane| {
                    let baseline = account.snapshot();
                    let mut shared = Some(
                        lane.try_retain_storage([capacity], 2, 0, |control| {
                            let mut value = String::with_capacity(capacity.envelope_capacity());
                            for _ in 0..2 {
                                control.run_unit(|| value.push('a'))?;
                            }
                            Ok(value)
                        })
                        .unwrap(),
                    );
                    assert_eq!(shared.as_ref().unwrap().as_ref(), "aa");
                    let retained = account.snapshot();
                    assert_eq!(
                        retained.live_retained_allocations(),
                        baseline.live_retained_allocations() + 2
                    );
                    let cause = lane
                        .try_retain_storage_prepared(
                            [capacity],
                            4,
                            0,
                            |_| Ok(shared.take().unwrap()),
                            |prepared, control| -> Result<String, StoreRuntimeError> {
                                let _prepared = prepared;
                                let mut value = String::with_capacity(capacity.envelope_capacity());
                                for _ in 0..3 {
                                    control.run_unit(|| {
                                        value.push('b');
                                        completed.set(completed.get() + 1);
                                    })?;
                                }
                                let _forgotten_borrow = ManuallyDrop::new(&mut *control);
                                panic!("physical typed-table builder panicked after its prefix");
                            },
                        )
                        .err()
                        .expect("a panicking physical builder cannot return retained storage");
                    assert_eq!(cause, StoreRuntimeError::ConstructionInvariantUnverified);
                    assert!(shared.is_none());
                    assert_eq!(completed.get(), 3);
                    let contained = account.snapshot();
                    assert!(contained.poisoned());
                    assert_eq!(
                        contained.live_retained_allocations(),
                        retained.live_retained_allocations()
                    );
                    assert_eq!(
                        contained.live_retained_bytes(),
                        retained.live_retained_bytes()
                    );
                    lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                        MaterializationRejection::Semantic,
                        72,
                    )
                },
            );
            let containment = outcome
                .err()
                .expect("unverified physical construction retains its actual containment");
            assert_eq!(
                containment.cause().reason(),
                AccountPoisonReason::OwnershipGraphUnverified
            );
        },
    );
    assert!(body.is_err());
    assert_eq!(completed.get(), 3);
    assert!(account.snapshot().poisoned());
}

#[test]
fn typed_storage_cancellation_is_observed_before_the_4097th_physical_unit() {
    const PLANNED_UNITS: usize = 4_097;
    const COMPLETED_PREFIX: usize = 4_096;

    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let mut control = Some(OperationControl::default().with_cancellation(cancellation.clone()));
    let completed = Cell::new(0_usize);
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let mut body_keys = None;
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup,
                 output,
                 activation,
                 ..
             },
             disposition| {
                let work = u64::try_from(PLANNED_UNITS).unwrap();
                let (operation, keys) = operation_with_producer_dimensions(
                    startup,
                    &account,
                    control.take().unwrap(),
                    DimensionPolicy::counter(1 + work, 1 + work, 0),
                );
                let capacity = CapacityPlan::for_type::<u8>(0, 0, PLANNED_UNITS).unwrap();
                let outcome = plan_scoped_activation!(
                    operation,
                    output,
                    activation,
                    scoped_activation_shape_with_dimensions(
                        keys,
                        1,
                        1,
                        keys.other_dimension,
                        keys.dimension,
                        keys.dimension,
                        keys.dimension,
                    ),
                    |mut lane| {
                        let baseline = account.snapshot();
                        let cause = lane
                            .try_retain_storage([capacity], work, 0, |control| {
                                let mut value = Vec::with_capacity(capacity.envelope_capacity());
                                for _ in 0..PLANNED_UNITS {
                                    control.run_unit(|| {
                                        value.push(1_u8);
                                        let next = completed.get() + 1;
                                        completed.set(next);
                                        if next == 1 {
                                            cancellation.cancel();
                                        }
                                    })?;
                                }
                                Ok(value)
                            })
                            .err()
                            .expect("cancellation must stop before the next bounded action");
                        assert_eq!(
                            cause,
                            StoreRuntimeError::Control(ControlCause::Cancelled {
                                origin: CancellationOrigin::Signal,
                            })
                        );
                        assert_eq!(completed.get(), COMPLETED_PREFIX);
                        let released = account.snapshot();
                        assert!(!released.poisoned());
                        assert_eq!(
                            released.live_retained_bytes(),
                            baseline.live_retained_bytes()
                        );
                        assert_eq!(
                            released.live_retained_allocations(),
                            baseline.live_retained_allocations()
                        );
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            75,
                        )
                    },
                )
                .unwrap();
                body_keys = Some(keys);
                request_rejected_activation_finish!(outcome, disposition);
            },
        )
        .unwrap();
    let CanonicalInvocationOutcome::Rejected(rejected) = finish.resume().outcome else {
        panic!("cancelled typed-table work must resume with its original rejection")
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let work = published
        .ledger_evidence
        .dimension(body_keys.unwrap().other_dimension)
        .unwrap();
    assert_eq!(work.attempted(), 1 + u64::try_from(PLANNED_UNITS).unwrap());
    assert_eq!(
        work.consumed(),
        1 + u64::try_from(COMPLETED_PREFIX).unwrap()
    );
    assert_eq!(work.attempted() - work.consumed(), 1);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(published.ledger_evidence.header().active_reservations(), 0);
    drop(published);
    drop(invocation_host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn typed_storage_prepare_cancellation_releases_its_value_without_admitting_build_work() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let mut control = Some(OperationControl::default().with_cancellation(cancellation.clone()));
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let mut body_keys = None;
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup,
                 output,
                 activation,
                 ..
             },
             disposition| {
                let (operation, keys) = operation_with_producer_dimensions(
                    startup,
                    &account,
                    control.take().unwrap(),
                    DimensionPolicy::counter(8, 8, 0),
                );
                let prepared_dropped = Cell::new(false);
                let builder_called = Cell::new(false);
                let capacity = CapacityPlan::for_type::<u8>(0, 0, 4).unwrap();
                let outcome = plan_scoped_activation!(
                    operation,
                    output,
                    activation,
                    scoped_activation_shape_with_dimensions(
                        keys,
                        1,
                        1,
                        keys.other_dimension,
                        keys.dimension,
                        keys.dimension,
                        keys.dimension,
                    ),
                    |mut lane| {
                        let baseline = account.snapshot();
                        let cause = lane
                            .try_retain_storage_prepared(
                                [capacity],
                                4,
                                0,
                                |_| {
                                    cancellation.cancel();
                                    Ok((
                                        DropProbe(&prepared_dropped),
                                        String::with_capacity(capacity.envelope_capacity()),
                                    ))
                                },
                                |(_probe, value), _control| {
                                    builder_called.set(true);
                                    Ok(value)
                                },
                            )
                            .err()
                            .expect("prepare cancellation must prevent producer admission");
                        assert_eq!(
                            cause,
                            StoreRuntimeError::Control(ControlCause::Cancelled {
                                origin: CancellationOrigin::Signal,
                            })
                        );
                        assert!(!builder_called.get());
                        assert!(prepared_dropped.get());
                        let released = account.snapshot();
                        assert!(!released.poisoned());
                        assert_eq!(
                            released.live_retained_bytes(),
                            baseline.live_retained_bytes()
                        );
                        assert_eq!(
                            released.live_retained_allocations(),
                            baseline.live_retained_allocations()
                        );
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            76,
                        )
                    },
                )
                .unwrap();
                body_keys = Some(keys);
                request_rejected_activation_finish!(outcome, disposition);
            },
        )
        .unwrap();
    let CanonicalInvocationOutcome::Rejected(rejected) = finish.resume().outcome else {
        panic!("cancelled preparation must preserve the rejected invocation")
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let work = published
        .ledger_evidence
        .dimension(body_keys.unwrap().other_dimension)
        .unwrap();
    assert_eq!((work.attempted(), work.consumed()), (1, 1));
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(published.ledger_evidence.header().active_reservations(), 0);
    drop(published);
    drop(invocation_host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
