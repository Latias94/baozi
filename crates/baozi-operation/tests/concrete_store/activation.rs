use std::cell::Cell;

use baozi_operation::__canonical_activation::{
    CanonicalActivationOutcome, CanonicalActivationPermit, CanonicalActivationPolicy,
    CanonicalActivationRequestSealer, CanonicalActivationRequirements, CanonicalActivationSeal,
    CanonicalActivationSealFailure, CanonicalCentralValidationOutcome,
    CanonicalFailureRequirements, CanonicalFreezePermit, CanonicalJointCommit, CanonicalJointSeal,
    CanonicalMaterialization, CanonicalPreflightDecision, CanonicalReservation,
    CanonicalScopedDisposition, CanonicalSemanticCompletion, CanonicalSuccessRequirements,
    CanonicalTableShape, CanonicalValidationDecision, CanonicalValidationMember,
    CanonicalValidationSession, FailureEvidenceDecision, FailureEvidenceMember,
    FailureEvidenceOutcome, OutcomeEvidenceDecision, OutcomeEvidenceValidationMember,
    RootedFailureValidation, SealedCanonicalActivationRequest, StoreCanonicalRejection,
    StoreNeverActivatedCause,
};
use baozi_operation::{ScalarSequence, StoreDestination, StorePlanError, StoreWorkError};

use super::*;

/// Test-only pair of operation-owned role policy and keyless core requirements.
///
/// Keeping this pair in the parent module lets the child integration-test modules exercise the
/// same sealing path without reconstructing ledger roles or commitment amounts locally.
struct TestActivation {
    policy: CanonicalActivationPolicy,
    requirements: CanonicalActivationRequirements,
}

struct TestScopedActivation {
    policy: CanonicalActivationPolicy,
    failure: CanonicalFailureRequirements,
    success: CanonicalSuccessRequirements,
}

macro_rules! plan_activation {
    ($operation:expr, $activation:expr, $preflight:expr, $materialize:expr $(,)?) => {{
        let TestActivation {
            policy,
            requirements,
        } = $activation;
        let (owner, sealer) = $operation.into_session_outcome_owner(policy);
        owner.plan_and_activate(sealer.seal(requirements), $preflight, $materialize)
    }};
}

#[path = "activation/central_validation.rs"]
mod central_validation;
#[path = "activation/controlled_writers.rs"]
mod controlled_writers;
#[path = "activation/failure_evidence.rs"]
mod failure_evidence;
#[path = "activation/joint_commit.rs"]
mod joint_commit;
#[path = "activation/preflight_materialization.rs"]
mod preflight_materialization;
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

    let _ = <CanonicalActivationRequirements as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <CanonicalActivationRequirements as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <CanonicalActivationRequestSealer as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <CanonicalActivationRequestSealer as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <SealedCanonicalActivationRequest as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <SealedCanonicalActivationRequest as AmbiguousIfClone<_>>::assert_not_clone;

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

fn activation_shape_with_validation_work(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
    validation_work: NonZeroU64,
) -> TestActivation {
    activation_shape_with_capacity_and_validation_work(
        keys,
        canonical_blocks,
        canonical_blocks,
        keys,
        evidence_blocks,
        validation_work,
    )
}

fn activation_shape_with_semantic_preflight_work(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
    semantic_preflight_work: u64,
) -> TestActivation {
    activation_shape_with_capacity_and_preflight_and_validation_work(
        keys,
        canonical_blocks,
        canonical_blocks,
        keys,
        evidence_blocks,
        semantic_preflight_work,
        NonZeroU64::MIN,
    )
}

fn activation_shape(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
) -> TestActivation {
    activation_shape_with_validation_work(keys, canonical_blocks, evidence_blocks, NonZeroU64::MIN)
}

fn activation_shape_with_capacity(
    canonical_keys: TestKeys,
    canonical_capacity: usize,
    expected_materialized_blocks: usize,
    evidence_keys: TestKeys,
    evidence_blocks: usize,
) -> TestActivation {
    activation_shape_with_capacity_and_validation_work(
        canonical_keys,
        canonical_capacity,
        expected_materialized_blocks,
        evidence_keys,
        evidence_blocks,
        NonZeroU64::MIN,
    )
}

fn activation_shape_with_capacity_and_validation_work(
    canonical_keys: TestKeys,
    canonical_capacity: usize,
    expected_materialized_blocks: usize,
    evidence_keys: TestKeys,
    evidence_blocks: usize,
    validation_work: NonZeroU64,
) -> TestActivation {
    activation_shape_with_capacity_and_preflight_and_validation_work(
        canonical_keys,
        canonical_capacity,
        expected_materialized_blocks,
        evidence_keys,
        evidence_blocks,
        0,
        validation_work,
    )
}

fn activation_shape_with_capacity_and_preflight_and_validation_work(
    canonical_keys: TestKeys,
    canonical_capacity: usize,
    expected_materialized_blocks: usize,
    evidence_keys: TestKeys,
    evidence_blocks: usize,
    semantic_preflight_work: u64,
    validation_work: NonZeroU64,
) -> TestActivation {
    let canonical = CanonicalTableShape::new(canonical_capacity, canonical_capacity).unwrap();
    let evidence = CanonicalTableShape::new(evidence_blocks, evidence_blocks).unwrap();
    let requirements = CanonicalActivationRequirements::new(
        canonical,
        evidence,
        expected_materialized_blocks,
        semantic_preflight_work,
        validation_work,
    )
    .unwrap();
    let policy = CanonicalActivationPolicy::new(
        writer_plan(canonical_keys, canonical_capacity + 1, 2),
        writer_plan(evidence_keys, evidence_blocks + 1, 2),
        canonical_keys.dimension,
        canonical_keys.dimension,
        canonical_keys.dimension,
        evidence_keys.dimension,
    );
    TestActivation {
        policy,
        requirements,
    }
}

fn activation_shape_with_dimensions(
    keys: TestKeys,
    materialization_work_dimension: StoreDimensionKey,
    validation_work_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
) -> TestActivation {
    let requirements = CanonicalActivationRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
        CanonicalTableShape::new(1, 1).unwrap(),
        1,
        0,
        NonZeroU64::MIN,
    )
    .unwrap();
    let policy = CanonicalActivationPolicy::new(
        writer_plan(keys, 2, 2),
        writer_plan(keys, 2, 2),
        materialization_work_dimension,
        validation_work_dimension,
        canonical_destination_dimension,
        outcome_evidence_destination_dimension,
    );
    TestActivation {
        policy,
        requirements,
    }
}

fn scoped_activation_shape(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
) -> TestScopedActivation {
    scoped_activation_shape_with_dimensions(
        keys,
        canonical_blocks,
        evidence_blocks,
        keys.dimension,
        keys.dimension,
        keys.dimension,
        keys.dimension,
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
            NonZeroU64::MIN,
            NonZeroU64::MIN,
        )
        .unwrap(),
    }
}

#[test]
fn activation_requirements_reject_impossible_exact_shapes() {
    let factory = factory(1);
    let account = account(&factory);
    let (_operation, _keys) = operation(&account);

    assert!(matches!(
        CanonicalActivationRequirements::new(
            CanonicalTableShape::new(1, 1).unwrap(),
            CanonicalTableShape::new(1, 1).unwrap(),
            0,
            0,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
    assert!(matches!(
        CanonicalActivationRequirements::new(
            CanonicalTableShape::new(1, 1).unwrap(),
            CanonicalTableShape::new(1, 1).unwrap(),
            2,
            0,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
    assert!(matches!(
        CanonicalActivationRequirements::new(
            CanonicalTableShape::new(1, 0).unwrap(),
            CanonicalTableShape::new(1, 1).unwrap(),
            1,
            0,
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
    let legacy = activation_shape(keys, 4, 1).requirements;
    let scoped = scoped_activation_shape(keys, 4, 1);
    let expanded_canonical = scoped_activation_shape(keys, 5, 1);
    let expanded_evidence = scoped_activation_shape(keys, 4, 2);
    let work = legacy.structural_work_requirement();
    let legacy_expanded_canonical_work = activation_shape_with_capacity(keys, 5, 5, keys, 1)
        .requirements
        .structural_work_requirement();
    let legacy_expanded_evidence_work = activation_shape(keys, 4, 2)
        .requirements
        .structural_work_requirement();
    let pre_activation_work = legacy.pre_activation_structural_work_requirement();
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

    assert_eq!(work, 24);
    assert_eq!(scoped_work, work);
    assert_eq!(expanded_canonical_work, 26);
    assert_eq!(expanded_canonical_work, legacy_expanded_canonical_work);
    assert_eq!(expanded_evidence_work, 26);
    assert_eq!(expanded_evidence_work, legacy_expanded_evidence_work);
    assert_eq!(pre_activation_work, 8);
    assert_eq!(scoped_pre_activation_work, pre_activation_work);
    drop(operation);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn structural_work_requirement_rejects_extreme_shape_overflow() {
    let canonical = CanonicalTableShape::new(usize::MAX, usize::MAX).unwrap();
    let evidence = CanonicalTableShape::new(1, 1).unwrap();
    let failure = CanonicalFailureRequirements::new(evidence);
    let success =
        CanonicalSuccessRequirements::new(canonical, usize::MAX, NonZeroU64::MIN, NonZeroU64::MIN)
            .unwrap();

    assert_eq!(failure.pre_activation_structural_work_requirement(), Ok(8));
    assert_eq!(
        success.structural_work_requirement(&failure),
        Err(StorePlanError::SizeOverflow)
    );
    assert!(matches!(
        CanonicalActivationRequirements::new(canonical, evidence, usize::MAX, 0, NonZeroU64::MIN,),
        Err(StorePlanError::SizeOverflow)
    ));
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

    let result = plan_activation!(
        operation,
        activation_shape(keys, 1, 1),
        |preflight| preflight.seal::<()>(),
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

    let result = plan_activation!(
        operation,
        activation_shape(keys, 1, 1),
        |preflight| preflight.seal::<()>(),
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
    let outcome = plan_activation!(
        operation,
        activation_shape(keys, 1, evidence_blocks),
        |preflight| preflight.reject(PreflightRejection::Unsupported, 11),
        |_lane| -> CanonicalMaterialization<
            '_,
            (),
            PreflightRejection,
            ScalarSequence<u8>,
            SequenceTag,
        > { unreachable!("preflight rejection cannot materialize") },
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
