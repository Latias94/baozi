use super::super::operation::RetainedPlanningBaseline;
use super::*;
use std::num::NonZeroU64;

mod panic;

fn ordinary_assembly<'inv>(
    root: &'inv mut TestInvocationRoot<u64, u64>,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
) -> TestPublication<'inv, u64, u64> {
    test_operation_on_account_with_control(
        root,
        account,
        64,
        RetainedBatchLimits::new(2, 2),
        control,
    )
    .try_publication_assembly(publication_plan(1, 1))
    .unwrap_or_else(|_| panic!("connected borrowed-work assembly must open"))
}

fn assert_canonical_rollback(
    assembly: &TestPublication<'_, u64, u64>,
    storage_before: RetainedPlanningBaseline,
    physical_before: PhysicalStorageAccountSnapshot,
    attributed_before: u64,
    members_before: usize,
) {
    let operation = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation");
    assert!(operation.planning_matches_baseline(storage_before));
    assert!(!operation.has_outstanding_work_permit());
    assert_eq!(assembly.canonical_len(), members_before);
    assert_eq!(assembly.canonical_new_blocks(), members_before);
    assert_eq!(
        assembly
            .fields
            .canonical_batch
            .as_ref()
            .expect("canonical batch remains owned")
            .attribution_len(),
        members_before + 1
    );
    assert_eq!(
        assembly.test_canonical_attributed_bytes(),
        attributed_before
    );
    assert!(assembly.test_storage_graph_is_exact());
    let physical = assembly
        .fields
        .account
        .as_ref()
        .expect("assembly retains account")
        .snapshot();
    assert_eq!(
        physical.live_retained_bytes(),
        physical_before.live_retained_bytes()
    );
    assert_eq!(
        physical.live_retained_allocations(),
        physical_before.live_retained_allocations()
    );
    assert!(!physical.poisoned());
}

fn finish_minimal(assembly: TestPublication<'_, u64, u64>) -> PreparationFailureCode {
    let published = handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("resolved borrowed work must permit clean fallback")),
    );
    let PublishedRetainedPublicationParts::MinimalFallback {
        ledger_evidence, ..
    } = &published
    else {
        panic!("borrowed-work rejection must publish minimal fallback");
    };
    let cause = published_failure_cause(ledger_evidence);
    drop(published);
    cause
}

#[test]
fn earlier_table_or_physical_capacity_failure_precedes_deferred_work_binding() {
    for table_full in [true, false] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::new();
        let mut assembly = ordinary_assembly(&mut root, &account, OperationControl::default());
        if table_full {
            let _slot = assembly
                .try_retain_canonical(empty_plan(), |emplacer| {
                    Ok(emplacer.place(11, CanonicalRootTag))
                })
                .unwrap_or_else(|_| panic!("the first canonical block must fill the table"));
        }
        let storage_before = assembly
            .operation
            .as_ref()
            .expect("assembly retains operation")
            .planning_baseline();
        let physical_before = account.snapshot();
        let attributed_before = assembly.test_canonical_attributed_bytes();
        let work_before = assembly
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        let shares_before = account.share_call_count();
        let mut builder_ran = false;
        let plan = if table_full {
            empty_plan()
        } else {
            RetainedAllocationPlan::new(1, 128).expect("finite physical over-capacity plan")
        };

        let result = assembly.try_retain_canonical_controlled(
            TestDimension::Unregistered,
            plan,
            |emplacer, control| {
                builder_ran = true;
                assert!(matches!(
                    control.try_begin(NonZeroU64::MIN),
                    Err(RetainedWorkStartError::Ledger(
                        LedgerCause::UnknownDimension {
                            dimension: TestDimension::Unregistered,
                        }
                    ))
                ));
                Ok(emplacer.place(13, CanonicalRootTag))
            },
        );

        if table_full {
            assert!(matches!(
                result,
                Err(RetainedPublicationError::CanonicalSlotLimit)
            ));
        } else {
            assert!(matches!(
                result,
                Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
                    RetainedAllocationError::AllocationLimitExceeded { requested, limit: 128 }
                ))) if requested > 128
            ));
        }
        assert!(!builder_ran);
        assert_eq!(account.share_call_count(), shares_before);
        let work = assembly
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        assert_eq!(
            work.attempted(),
            work_before.attempted() + if table_full { 0 } else { 2 }
        );
        assert_eq!(work.consumed(), work_before.consumed());
        assert!(
            assembly
                .test_dimension_evidence(TestDimension::Unregistered)
                .is_none()
        );
        assert_eq!(
            assembly.operation.as_ref().unwrap().sticky_terminal_cause(),
            None,
            "a deferred invalid key must not latch a failure before the builder runs"
        );
        assert_canonical_rollback(
            &assembly,
            storage_before,
            physical_before,
            attributed_before,
            usize::from(table_full),
        );
        assert_eq!(
            finish_minimal(assembly).class(),
            PreparationFailureClass::Requested
        );
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }
}

#[test]
fn fresh_cancellation_precedes_deferred_binding_at_connected_builder_begin() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let cancellation = crate::CancellationToken::new();
    let mut root = TestInvocationRoot::new();
    let mut assembly = ordinary_assembly(
        &mut root,
        &account,
        OperationControl::default().with_cancellation(cancellation.clone()),
    );
    let storage_before = assembly.operation.as_ref().unwrap().planning_baseline();
    let physical_before = account.snapshot();
    let attributed_before = assembly.test_canonical_attributed_bytes();
    let work_before = assembly
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap();
    let shares_before = account.share_call_count();
    let mut builder_ran = false;

    let result = assembly.try_retain_canonical_controlled::<CanonicalRootTag>(
        TestDimension::Unregistered,
        empty_plan(),
        |_emplacer, control| {
            builder_ran = true;
            assert_eq!(account.share_call_count(), shares_before + 1);
            cancellation.cancel();
            assert!(matches!(
                control.try_begin(NonZeroU64::MIN),
                Err(RetainedWorkStartError::Control(
                    crate::ControlCause::Cancelled { .. }
                ))
            ));
            Err(BUILD_FAILED)
        },
    );

    assert!(builder_ran);
    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::Build(cause)
        ))) if cause == BUILD_FAILED
    ));
    let work = assembly
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (work_before.attempted() + 2, work_before.consumed() + 2)
    );
    assert!(
        assembly
            .test_dimension_evidence(TestDimension::Unregistered)
            .is_none()
    );
    assert_canonical_rollback(
        &assembly,
        storage_before,
        physical_before,
        attributed_before,
        0,
    );
    assert_eq!(
        finish_minimal(assembly).class(),
        PreparationFailureClass::Control
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn completed_same_row_share_reduces_connected_builder_consumed_headroom() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let mut root = TestInvocationRoot::new();
    let mut assembly = ordinary_assembly(&mut root, &account, OperationControl::default());
    let storage_before = assembly.operation.as_ref().unwrap().planning_baseline();
    let physical_before = account.snapshot();
    let attributed_before = assembly.test_canonical_attributed_bytes();
    let work_before = assembly
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap();
    let shares_before = account.share_call_count();
    let consumed_limit = work_before.consumed() + 2;
    let attempted_limit = work_before.attempted() + 5;
    assert!(consumed_limit <= attempted_limit);
    assembly
        .operation
        .as_ref()
        .unwrap()
        .common
        .coordinator
        .retained_ledger()
        .tighten_dimension_policy(
            TestDimension::Bytes,
            DimensionPolicy::reservable_bytes(attempted_limit, consumed_limit, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(64),
        )
        .unwrap_or_else(|_| panic!("legal tightening must leave exactly one completed share"));
    let mut builder_ran = false;

    let result = assembly.try_retain_canonical_controlled::<CanonicalRootTag>(
        TestDimension::Bytes,
        empty_plan(),
        |_emplacer, control| {
            builder_ran = true;
            assert_eq!(account.share_call_count(), shares_before + 1);
            assert!(matches!(
                control.try_begin(NonZeroU64::new(3).unwrap()),
                Err(RetainedWorkStartError::Ledger(LedgerCause::LimitExceeded {
                    class: CounterClass::Consumed,
                    dimension: TestDimension::Bytes,
                    value,
                    limit,
                })) if value == work_before.consumed() + 5 && limit == consumed_limit
            ));
            Err(BUILD_FAILED)
        },
    );

    assert!(builder_ran);
    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::Build(cause)
        ))) if cause == BUILD_FAILED
    ));
    let work = assembly
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (attempted_limit, consumed_limit)
    );
    assert_canonical_rollback(
        &assembly,
        storage_before,
        physical_before,
        attributed_before,
        0,
    );
    assert_eq!(
        finish_minimal(assembly).class(),
        PreparationFailureClass::Ledger
    );
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn forgotten_builder_permission_rejects_ok_before_commit_and_host_resolves_frame() {
    for (borrowed, completed) in [(false, 1), (false, 3), (true, 1), (true, 3)] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::new();
        let mut assembly = ordinary_assembly(&mut root, &account, OperationControl::default());
        let storage_before = assembly.operation.as_ref().unwrap().planning_baseline();
        let physical_before = account.snapshot();
        let attributed_before = assembly.test_canonical_attributed_bytes();
        let work_before = assembly
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        let shares_before = account.share_call_count();
        let mut builder_returned_ok = false;

        let result = assembly.try_retain_canonical_controlled(
            TestDimension::Bytes,
            empty_plan(),
            |emplacer, mut control| {
                if borrowed {
                    let mut permission = control
                        .try_begin_borrowed(NonZeroU64::new(3).unwrap())
                        .unwrap_or_else(|_| panic!("borrowed builder envelope must be admitted"));
                    assert!(
                        permission
                            .complete_units(NonZeroU64::new(completed).unwrap())
                            .is_ok()
                    );
                    assert_eq!(permission.remaining_units(), 3 - completed);
                    std::mem::forget(permission);
                } else {
                    let mut permission = control
                        .try_begin(NonZeroU64::new(3).unwrap())
                        .unwrap_or_else(|_| panic!("builder envelope must be admitted"));
                    assert!(
                        permission
                            .complete_units(NonZeroU64::new(completed).unwrap())
                            .is_ok()
                    );
                    assert_eq!(permission.remaining_units(), 3 - completed);
                    std::mem::forget(permission);
                }
                builder_returned_ok = true;
                Ok(emplacer.place(11, CanonicalRootTag))
            },
        );

        assert!(builder_returned_ok);
        assert!(matches!(
            result,
            Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
                RetainedAllocationError::Build(cause)
            ))) if cause == RetainedBuildFailureCode::new(ORDINARY_WORK_PERMIT_ABANDONED_CODE)
        ));
        assert_eq!(account.share_call_count(), shares_before + 1);
        let work = assembly
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        assert_eq!(
            (work.attempted(), work.consumed()),
            (
                work_before.attempted() + 5,
                work_before.consumed() + 2 + completed
            )
        );
        assert_canonical_rollback(
            &assembly,
            storage_before,
            physical_before,
            attributed_before,
            0,
        );
        assert_eq!(
            finish_minimal(assembly),
            PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE)
        );
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }
}

#[test]
fn ignored_builder_completion_or_overrun_error_rejects_ok_before_commit() {
    for overrun in [false, true] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::new();
        let mut assembly = ordinary_assembly(&mut root, &account, OperationControl::default());
        let storage_before = assembly.operation.as_ref().unwrap().planning_baseline();
        let physical_before = account.snapshot();
        let attributed_before = assembly.test_canonical_attributed_bytes();
        let work_before = assembly
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        let shares_before = account.share_call_count();
        let mut builder_returned_ok = false;

        let result = assembly.try_retain_canonical_controlled(
            TestDimension::Bytes,
            empty_plan(),
            |emplacer, control| {
                let mut permission = control
                    .try_begin(NonZeroU64::new(3).unwrap())
                    .unwrap_or_else(|_| {
                        panic!("faulted builder envelope must initially be admitted")
                    });
                assert!(permission.complete_units(NonZeroU64::MIN).is_ok());
                if overrun {
                    assert!(matches!(
                        permission.complete_units(NonZeroU64::new(3).unwrap()),
                        Err(RetainedWorkConsumeError::ExceedsAttempted)
                    ));
                    assert!(
                        permission
                            .complete_units(NonZeroU64::new(2).unwrap())
                            .is_ok()
                    );
                    assert!(permission.complete().is_ok());
                } else {
                    assert!(matches!(
                        permission.complete(),
                        Err(RetainedWorkFinishError::Incomplete)
                    ));
                }
                builder_returned_ok = true;
                Ok(emplacer.place(11, CanonicalRootTag))
            },
        );

        assert!(builder_returned_ok);
        assert!(matches!(
            result,
            Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(
                RetainedAllocationError::Build(cause)
            ))) if cause == RetainedBuildFailureCode::new(ORDINARY_WORK_PERMIT_ABANDONED_CODE)
        ));
        assert_eq!(account.share_call_count(), shares_before + 1);
        let work = assembly
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        assert_eq!(work.attempted(), work_before.attempted() + 5);
        assert_eq!(
            work.consumed(),
            work_before.consumed() + 2 + if overrun { 3 } else { 1 }
        );
        assert_canonical_rollback(
            &assembly,
            storage_before,
            physical_before,
            attributed_before,
            0,
        );
        let cause = finish_minimal(assembly);
        assert_eq!(cause.class(), PreparationFailureClass::ProductGraph);
        assert_eq!(
            cause.code(),
            if overrun {
                0x4f57_504f
            } else {
                ORDINARY_WORK_PERMIT_ABANDONED_CODE
            }
        );
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }
}

#[test]
fn protected_builder_distinguishes_accepted_control_from_fresh_cancellation() {
    for accepted_control in [true, false] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let cancellation = crate::CancellationToken::new();
        let mut root = TestInvocationRoot::new();
        let operation = test_operation_on_account_with_failure_work(
            &mut root,
            &account,
            64,
            RetainedBatchLimits::new(3, 2),
            OperationControl::default().with_cancellation(cancellation.clone()),
            2,
            7_u32,
        );
        let mut assembly = operation
            .try_publication_assembly(publication_plan(1, 2))
            .unwrap_or_else(|error| {
                panic!("protected borrowing fixture must open: {}", error.cause())
            });
        let (canonical, evidence) = retain_scalar_success(&mut assembly);
        if accepted_control {
            cancellation.cancel();
            assembly.operation.as_mut().unwrap().poll_terminal_control();
        }
        let mut failure = match assembly.validate_success(canonical, evidence, |_, _, _| false) {
            RetainedSuccessValidation::Rejected(failure) => failure,
            _ => panic!("both rejected success paths must retain the failure assembly"),
        };
        let operation = failure.assembly.operation.as_ref().unwrap();
        assert_eq!(
            operation
                .common
                .coordinator
                .recorded_control_cause()
                .is_some(),
            accepted_control
        );
        assert!(
            operation
                .common
                .coordinator
                .retained_ledger()
                .ordinary_work_is_sealed()
        );
        if !accepted_control {
            assert_eq!(
                failure.fallback_cause.class(),
                PreparationFailureClass::ProductGraph
            );
        }
        let storage_before = operation.planning_baseline();
        let physical_before = account.snapshot();
        let ordinary_before = failure
            .assembly
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        let protected_before = failure
            .assembly
            .test_dimension_evidence(TestDimension::FailureTerminalWork)
            .unwrap();
        let attributed_before = failure
            .assembly
            .fields
            .evidence_batch
            .as_ref()
            .unwrap()
            .attributed_bytes();
        let shares_before = account.share_call_count();
        let mut builder_returned_ok = false;

        let result =
            failure.try_retain_outcome_evidence_controlled(empty_plan(), |emplacer, control| {
                assert_eq!(account.share_call_count(), shares_before + 1);
                if accepted_control {
                    let mut permission = control.try_begin(NonZeroU64::MIN).unwrap_or_else(|_| {
                        panic!("accepted cancellation must allow bounded evidence work")
                    });
                    assert!(permission.complete_units(NonZeroU64::MIN).is_ok());
                    assert!(permission.complete().is_ok());
                } else {
                    cancellation.cancel();
                    assert!(matches!(
                        control.try_begin(NonZeroU64::MIN),
                        Err(RetainedFailureWorkStartError::Control(
                            crate::ControlCause::Cancelled { .. }
                        ))
                    ));
                }
                builder_returned_ok = true;
                Ok(emplacer.place(9, EvidenceDetailTag))
            });

        assert!(builder_returned_ok);
        let ordinary = failure
            .assembly
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        let protected = failure
            .assembly
            .test_dimension_evidence(TestDimension::FailureTerminalWork)
            .unwrap();
        assert_eq!(
            (ordinary.attempted(), ordinary.consumed()),
            (ordinary_before.attempted(), ordinary_before.consumed())
        );
        let completed = if accepted_control { 3 } else { 2 };
        assert_eq!(
            (protected.attempted(), protected.consumed()),
            (
                protected_before.attempted() + completed,
                protected_before.consumed() + completed
            )
        );
        assert!(failure.assembly.test_storage_graph_is_exact());
        assert!(
            !failure
                .assembly
                .operation
                .as_ref()
                .unwrap()
                .has_outstanding_work_permit()
        );
        let published = if accepted_control {
            let detail = result.unwrap_or_else(|_| {
                panic!("bounded evidence must commit after accepted cancellation")
            });
            assert_eq!(failure.outcome_evidence_len(), 2);
            assert!(ordinary.current_live_bytes() > ordinary_before.current_live_bytes());
            assert_eq!(
                account.snapshot().live_retained_allocations(),
                physical_before.live_retained_allocations() + 1
            );
            let validated = match failure.validate_rich_failure(evidence, |session, root| {
                session.outcome_evidence(root) == Some(&7)
                    && session.outcome_evidence(detail) == Some(&9)
            }) {
                RetainedFailureValidation::Valid(validated) => validated,
                _ => panic!("evidence built after accepted cancellation must validate"),
            };
            handoff_for_test(
                validated
                    .try_prepare(PreparationFailureCode::domain(711), final_progress())
                    .unwrap_or_else(|_| panic!("accepted cancellation must preserve rich failure")),
            )
        } else {
            assert!(matches!(
                result,
                Err(RetainedPublicationError::Writer(
                    RetainedWriterError::Physical(RetainedAllocationError::Build(_))
                ))
            ));
            assert_eq!(failure.outcome_evidence_len(), 1);
            assert_eq!(
                failure
                    .assembly
                    .fields
                    .evidence_batch
                    .as_ref()
                    .unwrap()
                    .attributed_bytes(),
                attributed_before
            );
            assert!(
                failure
                    .assembly
                    .operation
                    .as_ref()
                    .unwrap()
                    .planning_matches_baseline(storage_before)
            );
            assert_eq!(
                account.snapshot().live_retained_bytes(),
                physical_before.live_retained_bytes()
            );
            assert_eq!(
                account.snapshot().live_retained_allocations(),
                physical_before.live_retained_allocations()
            );
            handoff_for_test(
                failure
                    .prepare_minimal_fallback()
                    .unwrap_or_else(|_| panic!("fresh cancellation must retain clean fallback")),
            )
        };
        let ledger_evidence = match &published {
            PublishedRetainedPublicationParts::RichFailure {
                ledger_evidence, ..
            } if accepted_control => ledger_evidence,
            PublishedRetainedPublicationParts::MinimalFallback {
                ledger_evidence, ..
            } if !accepted_control => ledger_evidence,
            _ => panic!("protected cancellation must select the corresponding terminal outcome"),
        };
        assert_eq!(
            published_failure_cause(ledger_evidence).class(),
            PreparationFailureClass::Control
        );
        drop(published);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert!(!account.snapshot().poisoned());
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }
}
