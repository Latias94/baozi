#![allow(
    clippy::result_large_err,
    reason = "central-validation errors keep bounded fail-closed evidence inline without failure-path allocation"
)]

use std::cell::Cell;

use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationPermit, CanonicalActivationSeal,
    CanonicalActivationSealFailure, CanonicalActivationSessionPolicy, CanonicalActivationSlots,
    CanonicalBodyParts, CanonicalCauseSlot, CanonicalCentralValidationOutcome,
    CanonicalFailureRequirements, CanonicalFinal, CanonicalFreezeInterruption,
    CanonicalFreezePermit, CanonicalGenerationSlots, CanonicalInvocationHost,
    CanonicalInvocationOutcome, CanonicalJointCommit, CanonicalJointSeal, CanonicalJointSuccess,
    CanonicalLane, CanonicalMaterialization, CanonicalMaterializationRole, CanonicalOutputSlot,
    CanonicalProofSlot, CanonicalReservation, CanonicalScopedDisposition,
    CanonicalSemanticCompletion, CanonicalSemanticWorkError, CanonicalValidationDecision,
    CanonicalValidationMember, CanonicalValidationProgress, CanonicalValidationRole,
    CanonicalValidationSession, CanonicalValidationSlots, FailureEvidenceDecision,
    FailureEvidenceMember, FailureEvidenceOutcome, OutcomeEvidenceDecision,
    OutcomeEvidenceValidationMember, RetainedStoreSessionOutcomeOwner, RetainedStoreWriterRole,
    RootedFailureValidation, StoreActivationContainment, StoreCanonicalRejection,
    StoreNeverActivatedCause,
};
use baozi_operation::__retained_store_integration::authority::{
    FreshStoreLineage, TerminalStoreLineage,
};
use baozi_operation::__retained_store_integration::producer::{
    CanonicalActivationDescription, CanonicalActivationPlanSealer, CanonicalActivationRequest,
    CanonicalSealedActivationPlan, CanonicalSuccessRequirements, CanonicalTableShape,
    mint_canonical_activation_request,
};
use baozi_operation::{ScalarSequence, StoreDestination, StorePlanError, StoreWorkError};

use super::*;

struct TestScopedActivation {
    policy: TestActivationPolicy,
    failure: CanonicalFailureRequirements,
    success: CanonicalSuccessRequirements,
}

#[derive(Clone, Copy)]
struct TestEvidenceWriter {
    role: RetainedStoreWriterRole,
    limits: RetainedStoreBatchLimits,
}

impl TestEvidenceWriter {
    fn new(keys: TestKeys, attributions: usize, commitments: usize) -> Self {
        Self {
            role: RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
            limits: RetainedStoreBatchLimits::new(attributions, commitments).unwrap(),
        }
    }
}

struct TestActivationPolicy {
    session: CanonicalActivationSessionPolicy,
    outcome_evidence_writer_limits: RetainedStoreBatchLimits,
}

impl TestActivationPolicy {
    fn new(
        canonical_writer: RetainedStoreWriterRole,
        outcome_evidence_writer: TestEvidenceWriter,
        materialization: CanonicalMaterializationRole,
        validation: CanonicalValidationRole,
        canonical_destination_dimension: StoreDimensionKey,
        outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            session: CanonicalActivationSessionPolicy::new(
                canonical_writer,
                outcome_evidence_writer.role,
                materialization,
                validation,
                canonical_destination_dimension,
                outcome_evidence_destination_dimension,
            ),
            outcome_evidence_writer_limits: outcome_evidence_writer.limits,
        }
    }

    fn with_semantic_bytes_dimension(mut self, dimension: StoreDimensionKey) -> Self {
        self.session = self.session.with_semantic_bytes_dimension(dimension);
        self
    }
}

trait IntoTestSessionOutcomeOwner<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn into_test_session_outcome_owner(
        self,
        policy: TestActivationPolicy,
        failure: CanonicalFailureRequirements,
    ) -> Result<RetainedStoreSessionOutcomeOwner<'inv, O, PP, U, L>, StoreActivationContainment>;
}

impl<'inv, O, PP, U, L> IntoTestSessionOutcomeOwner<'inv, O, PP, U, L>
    for RetainedStoreOperation<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn into_test_session_outcome_owner(
        self,
        policy: TestActivationPolicy,
        failure: CanonicalFailureRequirements,
    ) -> Result<RetainedStoreSessionOutcomeOwner<'inv, O, PP, U, L>, StoreActivationContainment>
    {
        let TestActivationPolicy {
            session,
            outcome_evidence_writer_limits,
        } = policy;
        self.into_session_outcome_owner(session, outcome_evidence_writer_limits, failure)
    }
}

impl<'inv, O, PP, U, L> IntoTestSessionOutcomeOwner<'inv, O, PP, U, L>
    for CanonicalBodyOperation<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn into_test_session_outcome_owner(
        self,
        policy: TestActivationPolicy,
        failure: CanonicalFailureRequirements,
    ) -> Result<RetainedStoreSessionOutcomeOwner<'inv, O, PP, U, L>, StoreActivationContainment>
    {
        let TestActivationPolicy {
            session,
            outcome_evidence_writer_limits,
        } = policy;
        self.into_session_outcome_owner(session, outcome_evidence_writer_limits, failure)
    }
}

struct TestActivationDescription<P> {
    request: CanonicalActivationRequest,
    requirements: CanonicalSuccessRequirements,
    execution: P,
}

impl<P> TestActivationDescription<P> {
    fn new(requirements: CanonicalSuccessRequirements, execution: P) -> Self {
        let (identity, request) = mint_canonical_activation_request()
            .expect("the test activation request identity must remain available");
        Self::with_request(
            request,
            requirements.with_request_identity(identity),
            execution,
        )
    }

    fn with_request(
        request: CanonicalActivationRequest,
        requirements: CanonicalSuccessRequirements,
        execution: P,
    ) -> Self {
        Self {
            request,
            requirements,
            execution,
        }
    }
}

impl<P> CanonicalActivationDescription for TestActivationDescription<P> {
    type Execution = P;

    fn seal_activation<'scope>(
        self,
        sealer: CanonicalActivationPlanSealer<'scope>,
    ) -> CanonicalSealedActivationPlan<'scope, Self::Execution> {
        sealer.seal(self.request, self.requirements, self.execution)
    }
}

type TestActivationOutcome<'inv, T, E, CanonicalKind, CanonicalTag> = CanonicalActivationOutcome<
    'inv,
    TestOwner,
    TestProgress,
    TestUnit,
    T,
    E,
    CanonicalKind,
    CanonicalTag,
>;

type TestInvocationHost<
    T,
    Proof = (),
    EA = (),
    EV = ValidationRejection,
    EG = GenerationRejection,
    CK = ScalarSequence<u8>,
    CT = SequenceTag,
    EK = ScalarSequence<u8>,
    ET = EvidenceLeafTag,
> = CanonicalInvocationHost<
    TestOwner,
    TestProgress,
    TestUnit,
    T,
    Proof,
    EA,
    EV,
    EG,
    CK,
    CT,
    EK,
    ET,
>;

type TestFinalNeverActivated<'inv, E> = CanonicalFinal<
    baozi_operation::__retained_store_integration::activation::NeverActivatedStoreSession<
        'inv,
        TestOwner,
        TestProgress,
        TestUnit,
        E,
        TerminalStoreLineage,
    >,
>;

fn activate_scoped<'inv, 'scope, T, E, CanonicalKind, CanonicalTag>(
    permit: CanonicalActivationPermit<
        'inv,
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
        CanonicalLane<'inv, 'lane, TestOwner, TestProgress, TestUnit>,
    ) -> CanonicalMaterialization<
        'lane,
        T,
        E,
        CanonicalKind,
        CanonicalTag,
    >,
) -> CanonicalScopedDisposition<'scope> {
    permit.activate(|lane, _plan| materialize(lane))
}

fn run_scoped_activation<'inv, T, E, CanonicalKind, CanonicalTag>(
    operation: impl IntoTestSessionOutcomeOwner<
        'inv,
        TestOwner,
        TestProgress,
        TestUnit,
        FreshStoreLineage,
    >,
    output_slot: CanonicalOutputSlot<'inv, T>,
    activation_slots: CanonicalActivationSlots<'inv, E>,
    activation: TestScopedActivation,
    materialize: impl for<'lane> FnMut(
        CanonicalLane<'inv, 'lane, TestOwner, TestProgress, TestUnit>,
    ) -> CanonicalMaterialization<
        'lane,
        T,
        E,
        CanonicalKind,
        CanonicalTag,
    >,
) -> Result<
    TestActivationOutcome<'inv, T, E, CanonicalKind, CanonicalTag>,
    StoreActivationContainment,
> {
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = activation;
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut activation_plan = Some(TestActivationDescription::new(success, ()));
    let mut materialize = Some(materialize);
    owner.plan_and_activate_scoped(output_slot, activation_slots, move |mut planning| {
        let ((), completion) = planning
            .with_semantic_work(|semantic| {
                semantic.run_unit(|| ()).unwrap();
                Ok::<(), StoreRuntimeError>(())
            })
            .unwrap();
        let activation_plan = activation_plan
            .take()
            .expect("the scoped activation plan is consumed exactly once");
        match planning.seal_activation(activation_plan, completion) {
            CanonicalActivationSeal::Sealed(permit) => activate_scoped(
                permit,
                materialize
                    .take()
                    .expect("the scoped materializer is transferred exactly once"),
            ),
            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
        }
    })
}

macro_rules! plan_scoped_activation {
    ($operation:expr, $output_slot:expr, $activation_slots:expr, $activation:expr, $materialize:expr $(,)?) => {{
        run_scoped_activation(
            $operation,
            $output_slot,
            $activation_slots,
            $activation,
            $materialize,
        )
    }};
}

macro_rules! request_rejected_activation_finish {
    ($outcome:expr, $disposition:expr) => {
        match $outcome {
            CanonicalActivationOutcome::Rejected(state) => state.request_finish($disposition),
            CanonicalActivationOutcome::NeverActivated(state) => state.request_finish($disposition),
            CanonicalActivationOutcome::Activated(_) => {
                panic!("the rejection fixture must not retain an activated success")
            }
        }
        .unwrap()
    };
}

#[path = "activation/central_validation.rs"]
mod central_validation;
#[path = "activation/controlled_writers.rs"]
mod controlled_writers;
#[path = "activation/failure_evidence.rs"]
mod failure_evidence;
#[path = "activation/host_states.rs"]
mod host_states;
#[path = "activation/joint_commit.rs"]
mod joint_commit;
#[path = "activation/scoped_plan.rs"]
mod scoped_plan;
#[path = "activation/typed_storage.rs"]
mod typed_storage;

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
    type SuccessRequest = CanonicalSuccessRequirements;
    type SessionOwner<'inv> =
        RetainedStoreSessionOutcomeOwner<'inv, TestOwner, TestProgress, TestUnit>;
    type ActivationPermit<'inv> = CanonicalActivationPermit<
        'inv,
        'static,
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        ScalarSequence<u8>,
        SequenceTag,
    >;
    type ActivationFailure<'inv> = CanonicalActivationSealFailure<
        'inv,
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
    let _ = <SuccessRequest as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <SuccessRequest as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <SessionOwner<'_> as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <SessionOwner<'_> as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <ActivationPermit<'_> as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <ActivationPermit<'_> as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <ActivationFailure<'_> as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <ActivationFailure<'_> as AmbiguousIfClone<_>>::assert_not_clone;

    type Permit<'inv> = CanonicalFreezePermit<
        'inv,
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
    type Joint<'inv> = CanonicalJointCommit<
        'inv,
        'static,
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
    type Seal = CanonicalJointSeal<'static>;
    let _ = <Permit<'_> as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Permit<'_> as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <Joint<'_> as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Joint<'_> as AmbiguousIfClone<_>>::assert_not_clone;
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
    let policy = TestActivationPolicy::new(
        RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
        TestEvidenceWriter::new(keys, evidence_blocks + 1, 2),
        CanonicalMaterializationRole::new(
            materialization_work_dimension,
            materialization_work_dimension,
        ),
        CanonicalValidationRole::new(
            keys.phase,
            validation_work_dimension,
            validation_work_dimension,
        ),
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
            2,
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
            1,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
    assert!(matches!(
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(1, 1).unwrap(),
            2,
            0,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
    assert!(matches!(
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(1, 1).unwrap(),
            2,
            2,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
    assert!(matches!(
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(3, 1).unwrap(),
            2,
            4,
            NonZeroU64::MIN,
        ),
        Err(StorePlanError::InvalidBlockTable)
    ));
    assert!(
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(3, 1).unwrap(),
            2,
            3,
            NonZeroU64::MIN,
        )
        .is_ok()
    );
}

#[test]
fn structural_work_requirement_counts_operation_owned_share_actions() {
    let factory = factory(1);
    let account = account(&factory);
    let mut invocation_root = TestInvocationRoot::new();
    let (operation, keys) = operation(&mut invocation_root, &account);
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
    drop(invocation_root);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn structural_work_requirement_rejects_extreme_shape_overflow() {
    let canonical = CanonicalTableShape::new(usize::MAX - 1, usize::MAX - 1).unwrap();
    let evidence = CanonicalTableShape::new(1, 1).unwrap();
    let failure = CanonicalFailureRequirements::new(evidence);
    let success =
        CanonicalSuccessRequirements::new(canonical, 2, usize::MAX - 1, NonZeroU64::MIN).unwrap();

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

#[derive(Debug, PartialEq, Eq)]
enum GenerationRejection {
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
    let mut invocation_host = TestInvocationHost::<_, ()>::new();
    let body = invocation_host.run_body(
        |CanonicalBodyParts {
             startup: invocation_root,
             output: output_slot,
             activation: activation_slots,
             ..
         },
         _disposition| {
            let (operation, keys) = operation(invocation_root, &account);

            let result = plan_scoped_activation!(
                operation,
                output_slot,
                activation_slots,
                scoped_activation_shape(keys, 1, 1),
                |mut lane| {
                    let root = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            SequenceTag,
                            |writer| {
                                writer.push(1)?;
                                Ok(())
                            },
                        )
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
                            let ignored = lane.try_retain_bytes(
                                BytesPlan::new(1).unwrap(),
                                BytesTag,
                                |writer| {
                                    writer.write_bytes(&[2])?;
                                    Ok(())
                                },
                            );
                            assert_slot_limit(ignored.map(|_| ()), StoreDestination::Canonical);
                        }
                        ControlledWriterKind::Utf8 => {
                            let ignored = lane.try_retain_utf8(
                                Utf8Plan::new(1).unwrap(),
                                TextTag,
                                |writer| {
                                    writer.write_str("b")?;
                                    Ok(())
                                },
                            );
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
        },
    );
    assert!(body.is_err());
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
    let mut invocation_host = TestInvocationHost::<_, ()>::new();
    let body = invocation_host.run_body(
        |CanonicalBodyParts {
             startup: invocation_root,
             output: output_slot,
             activation: activation_slots,
             ..
         },
         _disposition| {
            let (operation, keys) = operation(invocation_root, &account);

            let result = plan_scoped_activation!(
                operation,
                output_slot,
                activation_slots,
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
                            let result = lane.try_retain_utf8(
                                Utf8Plan::new(1).unwrap(),
                                TextTag,
                                move |_writer| {
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
        },
    );
    assert!(body.is_err());
}

fn rejected_session_with_one_evidence_slot<'inv>(
    host: &'inv mut TestInvocationHost<(), (), PreflightRejection>,
    account: &PhysicalStorageAccount,
) -> (
    TestFinalNeverActivated<'inv, PreflightRejection>,
    TestKeys,
    CanonicalCauseSlot<'inv, StoreRuntimeError>,
) {
    rejected_session_with_control(host, account, OperationControl::default())
}

fn rejected_session_with_control<'inv>(
    host: &'inv mut TestInvocationHost<(), (), PreflightRejection>,
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
) -> (
    TestFinalNeverActivated<'inv, PreflightRejection>,
    TestKeys,
    CanonicalCauseSlot<'inv, StoreRuntimeError>,
) {
    let mut control = Some(control);
    let mut saved_keys = None;
    let finish = host
        .run_body(|parts, disposition| {
            let (operation, keys) =
                operation_with_control(parts.startup, account, control.take().unwrap());
            let (state, keys) =
                rejected_session_from_operation(operation, parts.output, parts.activation, keys, 1);
            saved_keys = Some(keys);
            state.request_finish(disposition).unwrap();
        })
        .unwrap();
    let completion = finish.resume();
    let CanonicalInvocationOutcome::NeverActivated(state) = completion.outcome else {
        panic!("preflight rejection must resume as NeverActivated")
    };
    (state, saved_keys.unwrap(), completion.failure_fallback)
}

fn rejected_session_with_failure_work_limits<'inv>(
    host: &'inv mut TestInvocationHost<(), (), PreflightRejection>,
    account: &PhysicalStorageAccount,
    construction_work_limit: u64,
    validation_work_limit: u64,
) -> (
    TestFinalNeverActivated<'inv, PreflightRejection>,
    TestKeys,
    CanonicalCauseSlot<'inv, StoreRuntimeError>,
) {
    let mut saved_keys = None;
    let finish = host
        .run_body(|parts, disposition| {
            let (operation, keys) = operation_with_failure_work_limits(
                parts.startup,
                account,
                construction_work_limit,
                validation_work_limit,
            );
            let (state, keys) =
                rejected_session_from_operation(operation, parts.output, parts.activation, keys, 1);
            saved_keys = Some(keys);
            state.request_finish(disposition).unwrap();
        })
        .unwrap();
    let completion = finish.resume();
    let CanonicalInvocationOutcome::NeverActivated(state) = completion.outcome else {
        panic!("preflight rejection must resume as NeverActivated")
    };
    (state, saved_keys.unwrap(), completion.failure_fallback)
}

fn rejected_session_from_operation<'inv>(
    operation: impl IntoTestSessionOutcomeOwner<
        'inv,
        TestOwner,
        TestProgress,
        TestUnit,
        FreshStoreLineage,
    >,
    output_slot: CanonicalOutputSlot<'inv, ()>,
    activation_slots: CanonicalActivationSlots<'inv, PreflightRejection>,
    keys: TestKeys,
    evidence_blocks: usize,
) -> (
    baozi_operation::__retained_store_integration::activation::NeverActivatedStoreSession<
        'inv,
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
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let outcome: CanonicalActivationOutcome<
        '_,
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        PreflightRejection,
        ScalarSequence<u8>,
        SequenceTag,
    > = owner
        .plan_and_activate_scoped(output_slot, activation_slots, |planning| {
            planning.reject(PreflightRejection::Unsupported, 11)
        })
        .unwrap();
    let state = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_) | CanonicalActivationOutcome::Rejected(_) => {
            panic!("preflight rejection must remain NeverActivated")
        }
    };
    (state, keys)
}
