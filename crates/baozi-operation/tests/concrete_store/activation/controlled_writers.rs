use super::*;
use baozi_operation::__retained_store_integration::activation::CanonicalDispositionSlot;

#[cfg(target_pointer_width = "64")]
// This remains a valid capacity plan but exceeds the supported targets' user address spaces.
const UNALLOCATABLE_SEQUENCE_ELEMENTS: usize = 1_usize << 62;

#[cfg(target_pointer_width = "64")]
fn operation_with_materialization_work_limit<'inv, S: TestOperationStart<'inv>>(
    root: S,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    materialization_work_limit: u64,
) -> (S::Operation, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = unbounded_failure_terminal_work(keys);
    let operation_attempted_work_limit = failure_terminal_work
        .ordinary_attempted_work_limit()
        .checked_sub(materialization_work_limit)
        .expect("materialization work fits the ordinary attempted budget");
    let operation_consumed_work_limit = failure_terminal_work
        .ordinary_consumed_work_limit()
        .checked_sub(materialization_work_limit)
        .expect("materialization work fits the ordinary consumed budget");
    let operation_policy = DimensionPolicy::reservable_bytes(
        operation_attempted_work_limit,
        operation_consumed_work_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let materialization_policy = DimensionPolicy::reservable_bytes(
        materialization_work_limit,
        materialization_work_limit,
        u64::MAX,
        u64::MAX,
    );
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, operation_policy),
            (keys.other_dimension, materialization_policy),
            (
                keys.failure_terminal_dimension,
                failure_terminal_work.requirement().dimension_policy(),
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
        failure_terminal_work,
    );
    (
        root.start(control, ledger_plan, account, start).unwrap(),
        keys,
    )
}

#[allow(
    clippy::type_complexity,
    reason = "the fixture consumes the restored state with its exact canonical and evidence brands"
)]
fn publish_rejected_writer(
    outcome: CanonicalInvocationOutcome<
        '_,
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        MaterializationRejection,
        ValidationRejection,
        ScalarSequence<u8>,
        SequenceTag,
        ScalarSequence<u8>,
        EvidenceLeafTag,
    >,
) -> baozi_operation::PublishedStoreFallbackParts {
    let rejected = match outcome {
        CanonicalInvocationOutcome::Rejected(rejected) => rejected,
        _ => {
            panic!("a rejected canonical writer must not activate")
        }
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    prepared.handoff()
}

fn request_completed_writer_finish<'inv, T, CanonicalKind, CanonicalTag>(
    outcome: TestActivationOutcome<'inv, T, (), CanonicalKind, CanonicalTag>,
    proof_slot: CanonicalProofSlot<'inv, ()>,
    validation_slots: CanonicalValidationSlots<'inv, ValidationRejection>,
    disposition: CanonicalDispositionSlot<
        'inv,
        CanonicalKind,
        CanonicalTag,
        ScalarSequence<u8>,
        EvidenceLeafTag,
    >,
) where
    CanonicalTag: Copy + Send + Sync + 'static,
{
    let CanonicalActivationOutcome::Activated(activated) = outcome else {
        panic!("a complete canonical writer must activate")
    };
    let central = activated
        .validate_centrally(
            proof_slot,
            validation_slots,
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, _canonical, evidence, &mut ()| {
                        assert!(session.visit_outcome_evidence_sequence(
                            evidence,
                            1,
                            |_index, value| value == 1,
                        )?);
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            73,
                        ))
                    },
                )
            },
        )
        .unwrap();
    let CanonicalCentralValidationOutcome::Rejected(rejected) = central else {
        panic!("the writer cleanup must retain its actual central rejection")
    };
    rejected.request_finish(disposition).unwrap();
}

fn rejected_sequence_prefix_work(completed: usize) -> (u64, u64) {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let mut body_keys = None;
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 activation: activation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) = operation(invocation_root, &account);
                let planned = completed.checked_add(1).expect("test plan remains bounded");

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
                    scoped_activation_shape(keys, 1, 1),
                    |mut lane| {
                        let cause = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(planned).unwrap(),
                                SequenceTag,
                                |writer| {
                                    for value in 0..completed {
                                        writer.push(u8::try_from(value % 251).unwrap())?;
                                    }
                                    Err(StoreBuildError::domain(73).unwrap().into())
                                },
                            )
                            .unwrap_err();
                        assert!(matches!(cause, StoreRuntimeError::Build(_)));
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            74,
                        )
                    },
                )
                .unwrap();

                body_keys = Some(keys);
                request_rejected_activation_finish!(outcome, disposition);
            },
        )
        .unwrap();
    let keys = body_keys.unwrap();
    let published = publish_rejected_writer(finish.resume().outcome);
    let dimension = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("canonical writer work remains queryable");
    let work = (dimension.attempted(), dimension.consumed());
    drop(published);
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    work
}

#[cfg(target_pointer_width = "64")]
#[test]
fn pre_cancelled_work_rejects_before_the_writer_factory() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let materialization_work_limit = u64::try_from(UNALLOCATABLE_SEQUENCE_ELEMENTS)
        .unwrap()
        .checked_add(1)
        .expect("the scoped semantic unit fits beside writer work");
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let mut control = Some(control);
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 activation: activation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) = operation_with_materialization_work_limit(
                    invocation_root,
                    &account,
                    control.take().unwrap(),
                    materialization_work_limit,
                );
                let builder_called = std::cell::Cell::new(false);

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
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
                        cancellation.cancel();
                        let cause = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(UNALLOCATABLE_SEQUENCE_ELEMENTS).unwrap(),
                                SequenceTag,
                                |_writer| {
                                    builder_called.set(true);
                                    Ok(())
                                },
                            )
                            .unwrap_err();
                        assert!(matches!(cause, StoreRuntimeError::Control(_)));
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            51,
                        )
                    },
                )
                .unwrap();

                assert!(!builder_called.get());
                request_rejected_activation_finish!(outcome, disposition);
            },
        )
        .unwrap();
    let published = publish_rejected_writer(finish.resume().outcome);
    drop(published);
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn ledger_work_rejection_precedes_the_writer_factory() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 activation: activation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) = operation_with_materialization_work_limit(
                    invocation_root,
                    &account,
                    OperationControl::default(),
                    1_024,
                );
                let builder_called = std::cell::Cell::new(false);

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
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
                        let cause = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(UNALLOCATABLE_SEQUENCE_ELEMENTS).unwrap(),
                                SequenceTag,
                                |_writer| {
                                    builder_called.set(true);
                                    Ok(())
                                },
                            )
                            .unwrap_err();
                        assert!(matches!(cause, StoreRuntimeError::Ledger(_)));
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            52,
                        )
                    },
                )
                .unwrap();

                assert!(!builder_called.get());
                request_rejected_activation_finish!(outcome, disposition);
            },
        )
        .unwrap();
    let published = publish_rejected_writer(finish.resume().outcome);
    drop(published);
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn writer_factory_build_failure_preserves_the_writer_as_attempted_only() {
    let factory = factory(1);
    let account = account(&factory);
    let writer_work = u64::try_from(UNALLOCATABLE_SEQUENCE_ELEMENTS).unwrap();
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let mut body_keys = None;
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 activation: activation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) = operation_with_materialization_work_limit(
                    invocation_root,
                    &account,
                    OperationControl::default(),
                    writer_work
                        .checked_add(1)
                        .expect("the scoped semantic unit fits beside writer work"),
                );
                let builder_called = std::cell::Cell::new(false);

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
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
                        let cause = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(UNALLOCATABLE_SEQUENCE_ELEMENTS).unwrap(),
                                SequenceTag,
                                |_writer| {
                                    builder_called.set(true);
                                    Ok(())
                                },
                            )
                            .unwrap_err();
                        assert!(matches!(
                            cause,
                            StoreRuntimeError::Build(cause)
                                if cause.class() == StoreBuildFailureClass::AllocationFailed
                        ));
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            53,
                        )
                    },
                )
                .unwrap();

                assert!(!builder_called.get());
                body_keys = Some(keys);
                request_rejected_activation_finish!(outcome, disposition);
            },
        )
        .unwrap();
    let keys = body_keys.unwrap();
    let published = publish_rejected_writer(finish.resume().outcome);
    let work = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .expect("writer work remains queryable");
    assert_eq!(work.attempted() - work.consumed(), writer_work);
    assert_eq!(work.consumed(), 1);
    drop(published);
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[derive(Clone, Copy)]
enum ExactSequenceFault {
    Underwrite,
    Overrun,
}

fn rejected_sequence_work(fault: ExactSequenceFault) -> (u64, u64) {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let work = {
        let mut body_keys = None;
        let finish = invocation_host
            .run_body(
                |CanonicalBodyParts {
                     startup: invocation_root,
                     output: output_slot,
                     activation: activation_slots,
                     ..
                 },
                 disposition| {
                    let (operation, keys) = operation(invocation_root, &account);
                    let elements = match fault {
                        ExactSequenceFault::Underwrite => 2,
                        ExactSequenceFault::Overrun => 1,
                    };

                    let outcome = plan_scoped_activation!(
                        operation,
                        output_slot,
                        activation_slots,
                        scoped_activation_shape(keys, 1, 1),
                        |mut lane| {
                            let cause = lane
                                .try_retain_sequence(
                                    SequencePlan::<u8>::new(elements).unwrap(),
                                    SequenceTag,
                                    |writer| {
                                        writer.push(1)?;
                                        if matches!(fault, ExactSequenceFault::Overrun) {
                                            let ignored = writer.push(2);
                                            assert!(ignored.is_err());
                                        }
                                        Ok(())
                                    },
                                )
                                .unwrap_err();
                            assert!(matches!(cause, StoreRuntimeError::Build(_)));
                            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                                MaterializationRejection::Semantic,
                                41,
                            )
                        },
                    )
                    .unwrap();
                    let rejected = match outcome {
                        CanonicalActivationOutcome::Rejected(rejected) => rejected,
                        CanonicalActivationOutcome::Activated(_)
                        | CanonicalActivationOutcome::NeverActivated(_) => {
                            panic!("an inexact canonical writer must not activate")
                        }
                    };
                    body_keys = Some(keys);
                    rejected.request_finish(disposition).unwrap();
                },
            )
            .unwrap();
        let keys = body_keys.unwrap();
        let CanonicalInvocationOutcome::Rejected(rejected) = finish.resume().outcome else {
            panic!("an inexact canonical writer must resume as Rejected")
        };
        let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
        let published = prepared.handoff();
        let dimension = published
            .ledger_evidence
            .dimension(keys.dimension)
            .expect("canonical writer work remains queryable");
        let work = (dimension.attempted(), dimension.consumed());
        assert!(!account.snapshot().poisoned());
        drop(published);
        work
    };
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    work
}

#[test]
fn canonical_sequence_underwrite_preserves_the_attempted_suffix() {
    let (attempted, consumed) = rejected_sequence_work(ExactSequenceFault::Underwrite);
    assert_eq!(attempted.checked_sub(consumed), Some(1));
}

#[test]
fn canonical_sequence_overrun_cannot_escape_an_ignored_writer_error() {
    let (attempted, consumed) = rejected_sequence_work(ExactSequenceFault::Overrun);
    assert_eq!(attempted, consumed);
}

#[test]
fn canonical_sequence_batches_255_256_and_257_completed_units_exactly() {
    for completed in [255, 256, 257] {
        let (attempted, consumed) = rejected_sequence_prefix_work(completed);
        assert_eq!(
            attempted.checked_sub(consumed),
            Some(1),
            "completed prefix {completed} must leave only the planned suffix attempted-only"
        );
    }
}

#[test]
fn canonical_sequence_cancellation_is_observed_at_the_256_unit_boundary() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let mut control = Some(control);
    let mut body_keys = None;
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 activation: activation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) =
                    operation_with_control(invocation_root, &account, control.take().unwrap());
                let completed = Cell::new(0_usize);

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
                    scoped_activation_shape(keys, 1, 1),
                    |mut lane| {
                        let cause = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(300).unwrap(),
                                SequenceTag,
                                |writer| loop {
                                    match writer.push(1) {
                                        Ok(()) => {
                                            let next = completed.get() + 1;
                                            completed.set(next);
                                            if next == 1 {
                                                cancellation.cancel();
                                            }
                                        }
                                        Err(cause) => return Err(cause),
                                    }
                                },
                            )
                            .unwrap_err();
                        assert!(matches!(cause, StoreRuntimeError::Control(_)));
                        lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                            MaterializationRejection::Semantic,
                            75,
                        )
                    },
                )
                .unwrap();

                assert_eq!(completed.get(), 256);
                body_keys = Some(keys);
                request_rejected_activation_finish!(outcome, disposition);
            },
        )
        .unwrap();
    let keys = body_keys.unwrap();
    let published = publish_rejected_writer(finish.resume().outcome);
    let dimension = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("cancelled canonical writer work remains queryable");
    assert_eq!(dimension.attempted() - dimension.consumed(), 44);
    drop(published);
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_sequence_finish_polls_after_flushing_the_final_prefix() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    let mut control = Some(control);
    let mut body_keys = None;
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 activation: activation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) =
                    operation_with_control(invocation_root, &account, control.take().unwrap());

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
                    scoped_activation_shape(keys, 1, 1),
                    |mut lane| {
                        let cause = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(1).unwrap(),
                                SequenceTag,
                                |writer| {
                                    writer.push(1)?;
                                    cancellation.cancel();
                                    Ok(())
                                },
                            )
                            .unwrap_err();
                        assert!(matches!(cause, StoreRuntimeError::Control(_)));
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
    let keys = body_keys.unwrap();
    let published = publish_rejected_writer(finish.resume().outcome);
    let dimension = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("finish-time cancellation work remains queryable");
    assert_eq!(dimension.attempted(), dimension.consumed());
    drop(published);
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_sequence_and_bytes_split_writes_honor_the_shared_checkpoint() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<
        _,
        (),
        (),
        ValidationRejection,
        GenerationRejection,
        baozi_operation::ByteBlock,
        BytesTag,
    >::new();
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 proof: proof_slot,
                 activation: activation_slots,
                 validation: validation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) = operation(invocation_root, &account);
                let sequence_tail = [2_u8; 256];
                let bytes_tail = [4_u8; 256];

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
                    scoped_activation_shape(keys, 2, 1),
                    |mut lane| {
                        let _sequence = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(257).unwrap(),
                                SequenceTag,
                                |writer| {
                                    writer.push(1)?;
                                    writer.extend_from_slice(&sequence_tail)?;
                                    Ok(())
                                },
                            )
                            .unwrap();
                        let bytes = lane
                            .try_retain_bytes(BytesPlan::new(257).unwrap(), BytesTag, |writer| {
                                writer.write_bytes(&[3])?;
                                writer.write_bytes(&bytes_tail)?;
                                Ok(())
                            })
                            .unwrap();
                        lane.complete::<_, (), _, _>(bytes, ())
                    },
                )
                .unwrap();

                assert!(matches!(outcome, CanonicalActivationOutcome::Activated(_)));
                request_completed_writer_finish(outcome, proof_slot, validation_slots, disposition);
            },
        )
        .unwrap();
    let CanonicalInvocationOutcome::CentralRejected(rejected) = finish.resume().outcome else {
        panic!("completed split writes must restore their actual cleanup disposition")
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    drop(prepared.handoff());
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_utf8_preserves_a_scalar_across_the_256_byte_boundary() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<
        _,
        (),
        (),
        ValidationRejection,
        GenerationRejection,
        baozi_operation::Utf8Block,
        TextTag,
    >::new();
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 proof: proof_slot,
                 activation: activation_slots,
                 validation: validation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) = operation(invocation_root, &account);
                let value = format!("{}é", "a".repeat(255));
                assert_eq!(value.len(), 257);

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
                    scoped_activation_shape(keys, 1, 1),
                    |mut lane| {
                        let root = lane
                            .try_retain_utf8(
                                Utf8Plan::new(value.len()).unwrap(),
                                TextTag,
                                |writer| {
                                    writer.write_str(&value)?;
                                    Ok(())
                                },
                            )
                            .unwrap();
                        lane.complete::<_, (), _, _>(root, ())
                    },
                )
                .unwrap();

                assert!(matches!(outcome, CanonicalActivationOutcome::Activated(_)));
                request_completed_writer_finish(outcome, proof_slot, validation_slots, disposition);
            },
        )
        .unwrap();
    let CanonicalInvocationOutcome::CentralRejected(rejected) = finish.resume().outcome else {
        panic!("completed UTF-8 writes must restore their actual cleanup disposition")
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    drop(prepared.handoff());
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_record_rollback_does_not_account_uncommitted_fields() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    {
        let mut body_keys = None;
        let finish = invocation_host
            .run_body(
                |CanonicalBodyParts {
                     startup: invocation_root,
                     output: output_slot,
                     activation: activation_slots,
                     ..
                 },
                 disposition| {
                    let (operation, keys) = operation(invocation_root, &account);

                    let outcome = plan_scoped_activation!(
                        operation,
                        output_slot,
                        activation_slots,
                        scoped_activation_shape(keys, 1, 1),
                        |mut lane| {
                            let cause = lane
                                .try_retain_records(
                                    RecordPlan::new(1, 64).unwrap(),
                                    CanonicalRootTag,
                                    |writer| {
                                        writer.write_row(|row| {
                                            for value in 0_u8..63 {
                                                row.write_scalar(value)?;
                                            }
                                            Err(StoreBuildError::domain(76).unwrap().into())
                                        })?;
                                        Ok(())
                                    },
                                )
                                .unwrap_err();
                            assert!(matches!(cause, StoreRuntimeError::Build(_)));
                            lane.reject::<(), _, baozi_operation::RecordBlock, CanonicalRootTag>(
                                MaterializationRejection::Semantic,
                                77,
                            )
                        },
                    )
                    .unwrap();

                    let rejected = match outcome {
                        CanonicalActivationOutcome::Rejected(rejected) => rejected,
                        CanonicalActivationOutcome::Activated(_)
                        | CanonicalActivationOutcome::NeverActivated(_) => {
                            panic!("a partial canonical record row must not activate")
                        }
                    };
                    body_keys = Some(keys);
                    rejected.request_finish(disposition).unwrap();
                },
            )
            .unwrap();
        let keys = body_keys.unwrap();
        let CanonicalInvocationOutcome::Rejected(rejected) = finish.resume().outcome else {
            panic!("a partial canonical record row must resume as Rejected")
        };
        let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
        let published = prepared.handoff();
        let dimension = published
            .ledger_evidence
            .dimension(keys.dimension)
            .expect("canonical record work remains queryable");
        assert_eq!(dimension.attempted() - dimension.consumed(), 64);
        drop(published);
    }
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_record_accepts_one_complete_64_field_row() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<
        _,
        (),
        (),
        ValidationRejection,
        GenerationRejection,
        baozi_operation::RecordBlock,
        CanonicalRootTag,
    >::new();
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 proof: proof_slot,
                 activation: activation_slots,
                 validation: validation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) = operation(invocation_root, &account);

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
                    scoped_activation_shape(keys, 1, 1),
                    |mut lane| {
                        let root = lane
                            .try_retain_records(
                                RecordPlan::new(1, 64).unwrap(),
                                CanonicalRootTag,
                                |writer| {
                                    writer.write_row(|row| {
                                        for value in 0_u8..64 {
                                            row.write_scalar(value)?;
                                        }
                                        Ok(())
                                    })?;
                                    Ok(())
                                },
                            )
                            .unwrap();
                        lane.complete::<_, (), _, _>(root, ())
                    },
                )
                .unwrap();

                assert!(matches!(outcome, CanonicalActivationOutcome::Activated(_)));
                request_completed_writer_finish(outcome, proof_slot, validation_slots, disposition);
            },
        )
        .unwrap();
    let CanonicalInvocationOutcome::CentralRejected(rejected) = finish.resume().outcome else {
        panic!("completed record writes must restore their actual cleanup disposition")
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    drop(prepared.handoff());
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_record_cancellation_is_observed_at_the_256_field_boundary() {
    const PLANNED_FIELDS: usize = 300;
    const COMPLETED_PREFIX: usize = 256;

    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let mut invocation_host = TestInvocationHost::<_, (), MaterializationRejection>::new();
    {
        let mut control = Some(control);
        let mut body_keys = None;
        let finish = invocation_host
            .run_body(
                |CanonicalBodyParts {
                     startup: invocation_root,
                     output: output_slot,
                     activation: activation_slots,
                     ..
                 },
                 disposition| {
                    let (operation, keys) =
                        operation_with_control(invocation_root, &account, control.take().unwrap());
                    let completed = Cell::new(0_usize);

                    let outcome = plan_scoped_activation!(
                        operation,
                        output_slot,
                        activation_slots,
                        scoped_activation_shape(keys, 1, 1),
                        |mut lane| {
                            let cause = lane
                                .try_retain_records(
                                    RecordPlan::new(PLANNED_FIELDS, 1).unwrap(),
                                    CanonicalRootTag,
                                    |writer| {
                                        for value in 0..PLANNED_FIELDS {
                                            writer.write_row(|row| {
                                                row.write_scalar(u16::try_from(value).unwrap())?;
                                                completed.set(completed.get() + 1);
                                                if completed.get() == 2 {
                                                    cancellation.cancel();
                                                }
                                                Ok(())
                                            })?;
                                        }
                                        Ok(())
                                    },
                                )
                                .unwrap_err();
                            assert!(matches!(cause, StoreRuntimeError::Control(_)));
                            lane.reject::<(), _, baozi_operation::RecordBlock, CanonicalRootTag>(
                                MaterializationRejection::Semantic,
                                78,
                            )
                        },
                    )
                    .unwrap();

                    assert_eq!(completed.get(), COMPLETED_PREFIX);
                    let rejected = match outcome {
                        CanonicalActivationOutcome::Rejected(rejected) => rejected,
                        CanonicalActivationOutcome::Activated(_)
                        | CanonicalActivationOutcome::NeverActivated(_) => {
                            panic!("a cancelled canonical record writer must reject")
                        }
                    };
                    body_keys = Some(keys);
                    rejected.request_finish(disposition).unwrap();
                },
            )
            .unwrap();
        let keys = body_keys.unwrap();
        let CanonicalInvocationOutcome::Rejected(rejected) = finish.resume().outcome else {
            panic!("a cancelled canonical record writer must resume as Rejected")
        };
        let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
        let published = prepared.handoff();
        let dimension = published
            .ledger_evidence
            .dimension(keys.dimension)
            .expect("cancelled canonical record work remains queryable");
        assert_eq!(
            dimension.attempted() - dimension.consumed(),
            u64::try_from(PLANNED_FIELDS - COMPLETED_PREFIX).unwrap()
        );
        drop(published);
    }
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn activation_reservation_survives_reborrows_for_multiple_canonical_writes() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_host = TestInvocationHost::<_, (), ()>::new();
    let finish = invocation_host
        .run_body(
            |CanonicalBodyParts {
                 startup: invocation_root,
                 output: output_slot,
                 proof: proof_slot,
                 activation: activation_slots,
                 validation: validation_slots,
                 ..
             },
             disposition| {
                let (operation, keys) = operation(invocation_root, &account);

                let outcome = plan_scoped_activation!(
                    operation,
                    output_slot,
                    activation_slots,
                    scoped_activation_shape(keys, 2, 1),
                    |mut lane| {
                        let mut reservation = {
                            let mut checkpoint = lane.control_checkpoint().unwrap();
                            checkpoint.reserve(2).unwrap()
                        };

                        let _first = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(1).unwrap(),
                                SequenceTag,
                                |writer| {
                                    writer.push(1)?;
                                    Ok(())
                                },
                            )
                            .unwrap();

                        {
                            let mut checkpoint = lane.control_checkpoint().unwrap();
                            checkpoint.validate_live_owner(&reservation).unwrap();
                            checkpoint.resize(&mut reservation, 4).unwrap();
                        }

                        let second = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(1).unwrap(),
                                SequenceTag,
                                |writer| {
                                    writer.push(2)?;
                                    Ok(())
                                },
                            )
                            .unwrap();

                        {
                            let mut checkpoint = lane.control_checkpoint().unwrap();
                            checkpoint.resize(&mut reservation, 0).unwrap();
                        }
                        drop(reservation);
                        lane.complete::<_, (), _, _>(second, second)
                    },
                )
                .unwrap();

                assert!(matches!(outcome, CanonicalActivationOutcome::Activated(_)));
                request_completed_writer_finish(outcome, proof_slot, validation_slots, disposition);
            },
        )
        .unwrap();
    let CanonicalInvocationOutcome::CentralRejected(rejected) = finish.resume().outcome else {
        panic!("completed reservation reborrows must restore their actual cleanup disposition")
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    drop(prepared.handoff());
    drop(invocation_host);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn controlled_sequence_writer_preserves_its_first_fault() {
    assert_ignored_controlled_writer_fault_is_contained(ControlledWriterKind::Sequence);
}

#[test]
fn controlled_bytes_writer_preserves_its_first_fault() {
    assert_ignored_controlled_writer_fault_is_contained(ControlledWriterKind::Bytes);
}

#[test]
fn controlled_utf8_writer_preserves_its_first_fault() {
    assert_ignored_controlled_writer_fault_is_contained(ControlledWriterKind::Utf8);
}

#[test]
fn controlled_record_writer_preserves_its_first_fault() {
    assert_ignored_controlled_writer_fault_is_contained(ControlledWriterKind::Records);
}

#[test]
fn controlled_sequence_builder_body_and_drop_panics_are_isolated() {
    assert_controlled_writer_body_and_drop_panics_are_isolated(ControlledWriterKind::Sequence);
}

#[test]
fn controlled_bytes_builder_body_and_drop_panics_are_isolated() {
    assert_controlled_writer_body_and_drop_panics_are_isolated(ControlledWriterKind::Bytes);
}

#[test]
fn controlled_utf8_builder_body_and_drop_panics_are_isolated() {
    assert_controlled_writer_body_and_drop_panics_are_isolated(ControlledWriterKind::Utf8);
}

#[test]
fn controlled_record_builder_body_and_drop_panics_are_isolated() {
    assert_controlled_writer_body_and_drop_panics_are_isolated(ControlledWriterKind::Records);
}
