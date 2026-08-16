#![allow(
    clippy::result_large_err,
    reason = "central-validation errors keep bounded fail-closed evidence inline without failure-path allocation"
)]

use std::cell::Cell;

use baozi_operation::__canonical_activation::{
    CanonicalActivationOutcome, CanonicalActivationPermit, CanonicalActivationPolicy,
    CanonicalActivationSeal, CanonicalActivationSealFailure, CanonicalCentralValidationOutcome,
    CanonicalFailureRequirements, CanonicalFreezePermit, CanonicalJointCommit, CanonicalJointSeal,
    CanonicalLane, CanonicalMaterialization, CanonicalReservation, CanonicalScopedDisposition,
    CanonicalSemanticCompletion, CanonicalSuccessRequirements, CanonicalTableShape,
    CanonicalValidationDecision, CanonicalValidationMember, CanonicalValidationSession,
    FailureEvidenceDecision, FailureEvidenceMember, FailureEvidenceOutcome,
    OutcomeEvidenceDecision, OutcomeEvidenceValidationMember, RootedFailureValidation,
    StoreActivationContainment, StoreCanonicalRejection, StoreNeverActivatedCause,
};
use baozi_operation::{ScalarSequence, StoreDestination, StorePlanError, StoreWorkError};

use super::*;

struct TestScopedActivation {
    policy: CanonicalActivationPolicy,
    failure: CanonicalFailureRequirements,
    success: CanonicalSuccessRequirements,
}

type TestActivationOutcome<T, E, CanonicalKind, CanonicalTag> = CanonicalActivationOutcome<
    TestOwner,
    TestProgress,
    TestUnit,
    T,
    E,
    CanonicalKind,
    CanonicalTag,
>;

fn activate_scoped<'scope, T, E, CanonicalKind, CanonicalTag>(
    permit: CanonicalActivationPermit<
        'scope,
        TestOwner,
        TestProgress,
        TestUnit,
        T,
        E,
        CanonicalKind,
        CanonicalTag,
    >,
    mut materialize: impl for<'lane> FnMut(
        CanonicalLane<'lane, TestOwner, TestProgress, TestUnit>,
    ) -> CanonicalMaterialization<
        'lane,
        T,
        E,
        CanonicalKind,
        CanonicalTag,
    >,
) -> CanonicalScopedDisposition<'scope> {
    permit.activate((), |lane, _plan| materialize(lane))
}

fn run_scoped_activation<T, E, CanonicalKind, CanonicalTag>(
    operation: TestOperation,
    activation: TestScopedActivation,
    materialize: impl for<'lane> FnMut(
        CanonicalLane<'lane, TestOwner, TestProgress, TestUnit>,
    ) -> CanonicalMaterialization<
        'lane,
        T,
        E,
        CanonicalKind,
        CanonicalTag,
    >,
) -> Result<TestActivationOutcome<T, E, CanonicalKind, CanonicalTag>, StoreActivationContainment> {
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = activation;
    let owner = operation
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();
    let mut materialize = Some(materialize);
    owner.plan_and_activate_scoped(move |mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        match planning.seal_activation(success, completion) {
            CanonicalActivationSeal::Sealed(permit) => activate_scoped(
                permit,
                materialize
                    .take()
                    .expect("the scoped materializer is transferred exactly once"),
            ),
            CanonicalActivationSeal::Unsealed(failure) => failure.finish_after_plan_drop(),
        }
    })
}

macro_rules! plan_scoped_activation {
    ($operation:expr, $activation:expr, $materialize:expr $(,)?) => {{ run_scoped_activation($operation, $activation, $materialize) }};
}

#[path = "activation/central_validation.rs"]
mod central_validation;
#[path = "activation/controlled_writers.rs"]
mod controlled_writers;
#[path = "activation/failure_evidence.rs"]
mod failure_evidence;
#[path = "activation/joint_commit.rs"]
mod joint_commit;
#[path = "activation/scoped_plan.rs"]
mod scoped_plan;

#[test]
fn failure_evidence_member_remains_linear() {
    trait AmbiguousIfCopy<Marker> {
        fn assert_not_copy() {}
    }
    impl<T: ?Sized> AmbiguousIfCopy<()> for T {}
    struct CopyMarker;
    impl<T: Copy> AmbiguousIfCopy<CopyMarker> for T {}

    trait AmbiguousIfClone<Marker> {
        fn assert_not_clone() {}
    }
    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    struct CloneMarker;
    impl<T: Clone> AmbiguousIfClone<CloneMarker> for T {}

    type Member =
        FailureEvidenceMember<'static, baozi_operation::ScalarSequence<u8>, EvidenceLeafTag>;
    let _ = <Member as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Member as AmbiguousIfClone<_>>::assert_not_clone;
}

#[test]
fn canonical_activation_authorities_remain_linear() {
    trait AmbiguousIfCopy<Marker> {
        fn assert_not_copy() {}
    }
    impl<T: ?Sized> AmbiguousIfCopy<()> for T {}
    struct CopyMarker;
    impl<T: Copy> AmbiguousIfCopy<CopyMarker> for T {}

    trait AmbiguousIfClone<Marker> {
        fn assert_not_clone() {}
    }
    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    struct CloneMarker;
    impl<T: Clone> AmbiguousIfClone<CloneMarker> for T {}

    type Completion = CanonicalSemanticCompletion<'static>;
    type ActivationPermit = CanonicalActivationPermit<
        'static,
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        ScalarSequence<u8>,
        SequenceTag,
    >;
    type ActivationFailure = CanonicalActivationSealFailure<
        'static,
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        ScalarSequence<u8>,
        SequenceTag,
    >;
    let _ = <Completion as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Completion as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <ActivationPermit as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <ActivationPermit as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <ActivationFailure as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <ActivationFailure as AmbiguousIfClone<_>>::assert_not_clone;

    type Permit = CanonicalFreezePermit<
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        ScalarSequence<u8>,
        SequenceTag,
        ScalarSequence<u8>,
        EvidenceLeafTag,
    >;
    type Joint = CanonicalJointCommit<
        'static,
        TestOwner,
        TestProgress,
        TestUnit,
        ScalarSequence<u8>,
        SequenceTag,
        ScalarSequence<u8>,
        EvidenceLeafTag,
    >;
    type Seal = CanonicalJointSeal<'static>;
    let _ = <Permit as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Permit as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <Joint as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Joint as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <Seal as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Seal as AmbiguousIfClone<_>>::assert_not_clone;
}

fn scoped_activation_shape(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
) -> TestScopedActivation {
    scoped_activation_shape_with_validation_work(
        keys,
        canonical_blocks,
        evidence_blocks,
        NonZeroU64::MIN,
    )
}

fn scoped_activation_shape_with_validation_work(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
    validation_work: NonZeroU64,
) -> TestScopedActivation {
    scoped_activation_shape_with_dimensions_and_validation_work(
        keys,
        canonical_blocks,
        evidence_blocks,
        [keys.dimension; 4],
        validation_work,
    )
}

fn scoped_activation_shape_with_dimensions(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
    materialization_work_dimension: StoreDimensionKey,
    validation_work_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
) -> TestScopedActivation {
    scoped_activation_shape_with_dimensions_and_validation_work(
        keys,
        canonical_blocks,
        evidence_blocks,
        [
            materialization_work_dimension,
            validation_work_dimension,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
        ],
        NonZeroU64::MIN,
    )
}

fn scoped_activation_shape_with_dimensions_and_validation_work(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
    dimensions: [StoreDimensionKey; 4],
    validation_work: NonZeroU64,
) -> TestScopedActivation {
    let [
        materialization_work_dimension,
        validation_work_dimension,
        canonical_destination_dimension,
        outcome_evidence_destination_dimension,
    ] = dimensions;
    let policy = CanonicalActivationPolicy::new(
        writer_plan(keys, canonical_blocks + 1, 2),
        writer_plan(keys, evidence_blocks + 1, 2),
        materialization_work_dimension,
        validation_work_dimension,
        canonical_destination_dimension,
        outcome_evidence_destination_dimension,
    );
    TestScopedActivation {
        policy,
        failure: CanonicalFailureRequirements::new(
            CanonicalTableShape::new(evidence_blocks, evidence_blocks).unwrap(),
        ),
        success: CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(canonical_blocks, canonical_blocks).unwrap(),
            canonical_blocks,
            validation_work,
        )
        .unwrap(),
    }
}

#[test]
fn activation_requirements_reject_impossible_exact_shapes() {
    assert!(matches!(
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(1, 1).unwrap(),
            0,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
    assert!(matches!(
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(1, 1).unwrap(),
            2,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
    assert!(matches!(
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(1, 0).unwrap(),
            1,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
}

#[test]
fn structural_work_requirement_counts_operation_owned_share_actions() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let scoped = scoped_activation_shape(keys, 4, 1);
    let expanded_canonical = scoped_activation_shape(keys, 5, 1);
    let expanded_evidence = scoped_activation_shape(keys, 4, 2);
    let scoped_pre_activation_work = scoped
        .failure
        .pre_activation_structural_work_requirement()
        .unwrap();
    let scoped_work = scoped
        .success
        .structural_work_requirement(&scoped.failure)
        .unwrap();
    let expanded_canonical_work = expanded_canonical
        .success
        .structural_work_requirement(&expanded_canonical.failure)
        .unwrap();
    let expanded_evidence_work = expanded_evidence
        .success
        .structural_work_requirement(&expanded_evidence.failure)
        .unwrap();

    assert_eq!(scoped_work, 24);
    assert_eq!(expanded_canonical_work, 26);
    assert_eq!(expanded_evidence_work, 26);
    assert_eq!(scoped_pre_activation_work, 8);
    drop(operation);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn structural_work_requirement_rejects_extreme_shape_overflow() {
    let canonical = CanonicalTableShape::new(usize::MAX, usize::MAX).unwrap();
    let evidence = CanonicalTableShape::new(1, 1).unwrap();
    let failure = CanonicalFailureRequirements::new(evidence);
    let success =
        CanonicalSuccessRequirements::new(canonical, usize::MAX, NonZeroU64::MIN).unwrap();

    assert_eq!(failure.pre_activation_structural_work_requirement(), Ok(8));
    assert_eq!(
        success.structural_work_requirement(&failure),
        Err(StorePlanError::SizeOverflow)
    );
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

    let result = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(1)?;
                    Ok(())
                })
                .unwrap();
            match kind {
                ControlledWriterKind::Sequence => {
                    let ignored = lane.try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer| {
                            writer.push(2)?;
                            Ok(())
                        },
                    );
                    assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                }
                ControlledWriterKind::Bytes => {
                    let ignored =
                        lane.try_retain_bytes(BytesPlan::new(1).unwrap(), BytesTag, |writer| {
                            writer.write_bytes(&[2])?;
                            Ok(())
                        });
                    assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                }
                ControlledWriterKind::Utf8 => {
                    let ignored =
                        lane.try_retain_utf8(Utf8Plan::new(1).unwrap(), TextTag, |writer| {
                            writer.write_str("b")?;
                            Ok(())
                        });
                    assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                }
                ControlledWriterKind::Records => {
                    let ignored = lane.try_retain_records(
                        RecordPlan::new(1, 1).unwrap(),
                        CanonicalRootTag,
                        |writer| {
                            writer.write_row(|row| row.write_scalar(2_u8))?;
                            Ok(())
                        },
                    );
                    assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                }
            }
            lane.complete::<_, (), _, _>(root, root)
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

    let result = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            match kind {
                ControlledWriterKind::Sequence => {
                    let capture = PanicOnDrop;
                    let result = lane.try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        move |_writer| {
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
                        BytesTag,
                        move |_writer| {
                            let _capture = &capture;
                            panic!("contained canonical bytes builder panic")
                        },
                    );
                    assert!(matches!(result, Err(StoreRuntimeError::BuilderPanicked)));
                }
                ControlledWriterKind::Utf8 => {
                    let capture = PanicOnDrop;
                    let result =
                        lane.try_retain_utf8(Utf8Plan::new(1).unwrap(), TextTag, move |_writer| {
                            let _capture = &capture;
                            panic!("contained canonical UTF-8 builder panic")
                        });
                    assert!(matches!(result, Err(StoreRuntimeError::BuilderPanicked)));
                }
                ControlledWriterKind::Records => {
                    let capture = PanicOnDrop;
                    let result = lane.try_retain_records(
                        RecordPlan::new(1, 1).unwrap(),
                        CanonicalRootTag,
                        move |_writer| {
                            let _capture = &capture;
                            panic!("contained canonical record builder panic")
                        },
                    );
                    assert!(matches!(result, Err(StoreRuntimeError::BuilderPanicked)));
                }
            }
            lane.reject::<(), (), ScalarSequence<u8>, SequenceTag>((), 1)
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
    rejected_session_from_operation(operation, keys, 1)
}

fn rejected_session_with_failure_work_limits(
    account: &PhysicalStorageAccount,
    construction_work_limit: u64,
    validation_work_limit: u64,
) -> (
    baozi_operation::__canonical_activation::NeverActivatedStoreSession<
        TestOwner,
        TestProgress,
        TestUnit,
        PreflightRejection,
    >,
    TestKeys,
) {
    let (operation, keys) =
        operation_with_failure_work_limits(account, construction_work_limit, validation_work_limit);
    rejected_session_from_operation(operation, keys, 1)
}

fn rejected_session_from_operation(
    operation: TestOperation,
    keys: TestKeys,
    evidence_blocks: usize,
) -> (
    baozi_operation::__canonical_activation::NeverActivatedStoreSession<
        TestOwner,
        TestProgress,
        TestUnit,
        PreflightRejection,
    >,
    TestKeys,
) {
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, evidence_blocks);
    let owner = operation
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();
    let outcome: CanonicalActivationOutcome<
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        PreflightRejection,
        ScalarSequence<u8>,
        SequenceTag,
    > = owner
        .plan_and_activate_scoped(|planning| planning.reject(PreflightRejection::Unsupported, 11))
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
