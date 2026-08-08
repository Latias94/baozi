use super::*;

#[cfg(target_pointer_width = "64")]
// This remains a valid capacity plan but exceeds the supported targets' user address spaces.
const UNALLOCATABLE_SEQUENCE_ELEMENTS: usize = 1_usize << 62;

#[cfg(target_pointer_width = "64")]
fn operation_with_materialization_work_limit(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    materialization_work_limit: u64,
) -> (TestOperation, TestKeys) {
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
        TestOperation::try_start(control, ledger_plan, account, start).unwrap(),
        keys,
    )
}

#[cfg(target_pointer_width = "64")]
fn publish_rejected_writer(
    outcome: CanonicalActivationOutcome<
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        MaterializationRejection,
        ScalarSequence<u8>,
        SequenceTag,
    >,
) -> baozi_operation::PublishedStoreFallbackParts {
    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a rejected canonical writer must not activate")
        }
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    prepared.handoff()
}

#[cfg(target_pointer_width = "64")]
#[test]
fn pre_cancelled_work_rejects_before_the_writer_factory() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_materialization_work_limit(
        &account,
        control,
        u64::try_from(UNALLOCATABLE_SEQUENCE_ELEMENTS).unwrap(),
    );
    cancellation.cancel();
    let builder_called = std::cell::Cell::new(false);

    let outcome = plan_activation!(
        operation,
        activation_shape_with_dimensions(
            keys,
            keys.other_dimension,
            keys.dimension,
            keys.dimension,
            keys.dimension,
        ),
        |preflight| preflight.seal(),
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
            assert!(matches!(cause, StoreRuntimeError::Control(_)));
            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                MaterializationRejection::Semantic,
                51,
            )
        },
    )
    .unwrap();

    assert!(!builder_called.get());
    let published = publish_rejected_writer(outcome);
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn ledger_work_rejection_precedes_the_writer_factory() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) =
        operation_with_materialization_work_limit(&account, OperationControl::default(), 1_024);
    let builder_called = std::cell::Cell::new(false);

    let outcome = plan_activation!(
        operation,
        activation_shape_with_dimensions(
            keys,
            keys.other_dimension,
            keys.dimension,
            keys.dimension,
            keys.dimension,
        ),
        |preflight| preflight.seal(),
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
    let published = publish_rejected_writer(outcome);
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn writer_factory_build_failure_preserves_only_attempted_work() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_materialization_work_limit(
        &account,
        OperationControl::default(),
        u64::try_from(UNALLOCATABLE_SEQUENCE_ELEMENTS).unwrap(),
    );
    let builder_called = std::cell::Cell::new(false);

    let outcome = plan_activation!(
        operation,
        activation_shape_with_dimensions(
            keys,
            keys.other_dimension,
            keys.dimension,
            keys.dimension,
            keys.dimension,
        ),
        |preflight| preflight.seal(),
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
    let published = publish_rejected_writer(outcome);
    let work = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .expect("writer work remains queryable");
    assert_eq!(
        work.attempted(),
        u64::try_from(UNALLOCATABLE_SEQUENCE_ELEMENTS).unwrap()
    );
    assert_eq!(work.consumed(), 0);
    drop(published);
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
    let (operation, keys) = operation(&account);
    let elements = match fault {
        ExactSequenceFault::Underwrite => 2,
        ExactSequenceFault::Overrun => 1,
    };

    let outcome = plan_activation!(
        operation,
        activation_shape(keys, 1, 1),
        |preflight| preflight.seal(),
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
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("an inexact canonical writer must not activate")
        }
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
fn activation_reservation_survives_reborrows_for_multiple_canonical_writes() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_activation!(
        operation,
        activation_shape(keys, 2, 1),
        |preflight| preflight.seal::<()>(),
        |mut lane| {
            let mut reservation = {
                let mut checkpoint = lane.control_checkpoint().unwrap();
                checkpoint.reserve(keys.phase, keys.dimension, 2).unwrap()
            };

            let _first = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(1)?;
                    Ok(())
                })
                .unwrap();

            {
                let mut checkpoint = lane.control_checkpoint().unwrap();
                checkpoint.validate_live_owner(&reservation).unwrap();
                checkpoint.resize(&mut reservation, 4).unwrap();
            }

            let second = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(2)?;
                    Ok(())
                })
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
    drop(outcome);
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
