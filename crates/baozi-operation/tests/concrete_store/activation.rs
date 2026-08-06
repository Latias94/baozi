use std::cell::Cell;

use baozi_operation::__canonical_activation::{
    CanonicalActivationOutcome, CanonicalCentralValidationOutcome, CanonicalMaterialization,
    CanonicalOutputIntent, CanonicalPreflightDecision, CanonicalValidationDecision,
    FailureEvidenceDecision, FailureEvidenceOutcome, OutcomeEvidenceDecision,
    RootedFailureValidation, StoreCanonicalRejection, StoreNeverActivatedCause,
};
use baozi_operation::{RetainedStoreBlockTablePlan, StoreDestination, StoreWorkError};

use super::*;

#[path = "activation/central_validation.rs"]
mod central_validation;
#[path = "activation/controlled_writers.rs"]
mod controlled_writers;
#[path = "activation/failure_evidence.rs"]
mod failure_evidence;
#[path = "activation/preflight_materialization.rs"]
mod preflight_materialization;

fn canonical_plan(keys: TestKeys, blocks: usize) -> RetainedStoreBlockTablePlan {
    RetainedStoreBlockTablePlan::new(writer_plan(keys, blocks + 1, 2), blocks, blocks).unwrap()
}

fn evidence_plan(keys: TestKeys, blocks: usize) -> RetainedStoreBlockTablePlan {
    RetainedStoreBlockTablePlan::new(writer_plan(keys, blocks + 1, 2), blocks, blocks).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
enum PreflightRejection {
    Unsupported,
}

#[derive(Debug, PartialEq, Eq)]
enum MaterializationRejection {
    Semantic,
}

#[derive(Debug, PartialEq, Eq)]
enum ValidationRejection {
    Semantic,
}

struct DropProbe<'a>(&'a Cell<bool>);

impl Drop for DropProbe<'_> {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("contained materialization callback drop panic");
    }
}

#[derive(Clone, Copy)]
enum ControlledWriterKind {
    Sequence,
    Bytes,
    Utf8,
    Records,
}

fn assert_ignored_controlled_writer_fault_is_contained(kind: ControlledWriterKind) {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |lane| {
                match kind {
                    ControlledWriterKind::Sequence => {
                        let _ = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(1).unwrap(),
                                keys.dimension,
                                SequenceTag,
                                |writer, _control| {
                                    writer.push(1)?;
                                    Ok(())
                                },
                            )
                            .unwrap();
                        let ignored = lane.try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            keys.dimension,
                            SequenceTag,
                            |writer, _control| {
                                writer.push(2)?;
                                Ok(())
                            },
                        );
                        assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                    }
                    ControlledWriterKind::Bytes => {
                        let _ = lane
                            .try_retain_bytes(
                                BytesPlan::new(1).unwrap(),
                                keys.dimension,
                                BytesTag,
                                |writer, _control| {
                                    writer.write_bytes(&[1])?;
                                    Ok(())
                                },
                            )
                            .unwrap();
                        let ignored = lane.try_retain_bytes(
                            BytesPlan::new(1).unwrap(),
                            keys.dimension,
                            BytesTag,
                            |writer, _control| {
                                writer.write_bytes(&[2])?;
                                Ok(())
                            },
                        );
                        assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                    }
                    ControlledWriterKind::Utf8 => {
                        let _ = lane
                            .try_retain_utf8(
                                Utf8Plan::new(1).unwrap(),
                                keys.dimension,
                                TextTag,
                                |writer, _control| {
                                    writer.write_str("a")?;
                                    Ok(())
                                },
                            )
                            .unwrap();
                        let ignored = lane.try_retain_utf8(
                            Utf8Plan::new(1).unwrap(),
                            keys.dimension,
                            TextTag,
                            |writer, _control| {
                                writer.write_str("b")?;
                                Ok(())
                            },
                        );
                        assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                    }
                    ControlledWriterKind::Records => {
                        let _ = lane
                            .try_retain_records(
                                RecordPlan::new(1, 1).unwrap(),
                                keys.dimension,
                                CanonicalRootTag,
                                |writer, _control| {
                                    writer.write_row(|row| row.write_scalar(1_u8))?;
                                    Ok(())
                                },
                            )
                            .unwrap();
                        let ignored = lane.try_retain_records(
                            RecordPlan::new(1, 1).unwrap(),
                            keys.dimension,
                            CanonicalRootTag,
                            |writer, _control| {
                                writer.write_row(|row| row.write_scalar(2_u8))?;
                                Ok(())
                            },
                        );
                        assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                    }
                }
                lane.accept::<_, ()>(())
            },
        );
    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("ignored first fault must be contained"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account.snapshot().poisoned());
}

fn assert_slot_limit(result: Result<(), StoreRuntimeError>, destination: StoreDestination) {
    assert!(matches!(
        result,
        Err(StoreRuntimeError::SlotLimit {
            destination: actual
        }) if actual == destination
    ));
}

fn assert_controlled_writer_body_and_drop_panics_are_isolated(kind: ControlledWriterKind) {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |lane| {
                match kind {
                    ControlledWriterKind::Sequence => {
                        let capture = PanicOnDrop;
                        let result = lane.try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            keys.dimension,
                            SequenceTag,
                            move |_writer, _control| {
                                let _capture = &capture;
                                panic!("contained canonical sequence builder panic")
                            },
                        );
                        assert!(matches!(result, Err(StoreRuntimeError::BuilderPanicked)));
                    }
                    ControlledWriterKind::Bytes => {
                        let capture = PanicOnDrop;
                        let result = lane.try_retain_bytes(
                            BytesPlan::new(1).unwrap(),
                            keys.dimension,
                            BytesTag,
                            move |_writer, _control| {
                                let _capture = &capture;
                                panic!("contained canonical bytes builder panic")
                            },
                        );
                        assert!(matches!(result, Err(StoreRuntimeError::BuilderPanicked)));
                    }
                    ControlledWriterKind::Utf8 => {
                        let capture = PanicOnDrop;
                        let result = lane.try_retain_utf8(
                            Utf8Plan::new(1).unwrap(),
                            keys.dimension,
                            TextTag,
                            move |_writer, _control| {
                                let _capture = &capture;
                                panic!("contained canonical UTF-8 builder panic")
                            },
                        );
                        assert!(matches!(result, Err(StoreRuntimeError::BuilderPanicked)));
                    }
                    ControlledWriterKind::Records => {
                        let capture = PanicOnDrop;
                        let result = lane.try_retain_records(
                            RecordPlan::new(1, 1).unwrap(),
                            keys.dimension,
                            CanonicalRootTag,
                            move |_writer, _control| {
                                let _capture = &capture;
                                panic!("contained canonical record builder panic")
                            },
                        );
                        assert!(matches!(result, Err(StoreRuntimeError::BuilderPanicked)));
                    }
                }
                lane.accept::<_, ()>(())
            },
        );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

fn rejected_session_with_one_evidence_slot(
    account: &PhysicalStorageAccount,
) -> (
    baozi_operation::__canonical_activation::NeverActivatedStoreSession<
        TestOwner,
        TestProgress,
        TestUnit,
        PreflightRejection,
    >,
    TestKeys,
) {
    rejected_session_with_control(account, OperationControl::default())
}

fn rejected_session_with_control(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
) -> (
    baozi_operation::__canonical_activation::NeverActivatedStoreSession<
        TestOwner,
        TestProgress,
        TestUnit,
        PreflightRejection,
    >,
    TestKeys,
) {
    let (operation, keys) = operation_with_control(account, control);
    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.reject(PreflightRejection::Unsupported, 11),
            |_lane| -> CanonicalMaterialization<(), PreflightRejection> {
                unreachable!("preflight rejection cannot materialize")
            },
        )
        .unwrap();
    let state = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("preflight rejection must remain NeverActivated")
        }
    };
    (state, keys)
}
