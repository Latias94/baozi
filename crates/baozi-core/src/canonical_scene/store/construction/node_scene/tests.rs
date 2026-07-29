use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use baozi_operation::{
    CancellationSignal, CancellationToken, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
    StorePreparationFailureClass, StoreTerminalCause,
};

use crate::SceneValidationLimits;
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::construction::{
    STORE_DIMENSION, SceneConstructionLimits, SceneConstructionOperation,
    SceneConstructionTerminal, TwoNodeAuthoredMatrices,
};
use crate::canonical_scene::store::production::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    NodeAuthoringBatch, NodeAuthoringFailure, NodeAuthoringLimit, NodeAuthoringNode,
    NodeAuthoringSemanticError, NodeLocalId, NodeParent, Presence, SceneAuthoringBatch,
};
use crate::entity::AuthoredTransform;
use crate::identity::{SceneGenerationError, SceneGenerationId};
use crate::math::{Mat4, Quat, Vec3};

use super::publication::post_evidence_build_failure_code;
use super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_STORAGE_REJECTION_CODE, NodeSceneBuildFaults,
    NodeSceneConstructionRejection, NodeSceneConstructionTerminal, NodeSceneRichFailure,
    PreparedNodeSceneConstruction,
};

mod obj_capability;

pub(in crate::canonical_scene::store) use obj_capability::{
    OBJ_SOURCE_VECTOR_SCHEMA_ID, ObjCapabilityCanonicalShapeObservation,
    ObjCapabilitySourceShapeObservation, build_obj_capability_scene_for_probe,
    capture_obj_capability_snapshots_for_probe, collect_obj_capability_vectors_for_probe,
    inspect_obj_capability_semantics_for_probe,
};

const SINGLE_TRS_AUTHORING_VALIDATION_WORK: u64 = 41;
const SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE: u64 = 43;
const SINGLE_TRS_BUILD_INVARIANT_FAILURE_WORK: u64 = 80;
const SINGLE_TRS_GENERATION_FAILURE_WORK: u64 = 82;
const SINGLE_TRS_ROOTS: [NodeLocalId; 1] = [NodeLocalId::new(0)];
const SINGLE_TRS_NODES: [NodeAuthoringNode; 1] = [NodeAuthoringNode::new(
    Presence::Absent,
    AuthoredTransform::identity_trs(),
)];

#[derive(Clone)]
struct CancelOnSignalPoll {
    polls: Arc<AtomicUsize>,
    cancel_on: Arc<AtomicUsize>,
}

impl CancelOnSignalPoll {
    fn new() -> Self {
        Self {
            polls: Arc::new(AtomicUsize::new(0)),
            cancel_on: Arc::new(AtomicUsize::new(usize::MAX)),
        }
    }

    fn polls(&self) -> usize {
        self.polls.load(Ordering::Acquire)
    }

    fn cancel_after_additional_polls(&self, additional: usize) {
        self.cancel_on.store(
            self.polls()
                .checked_add(additional)
                .expect("test cancellation poll threshold"),
            Ordering::Release,
        );
    }
}

impl CancellationSignal for CancelOnSignalPoll {
    fn is_cancelled(&self) -> bool {
        let poll = self.polls.fetch_add(1, Ordering::AcqRel) + 1;
        poll >= self.cancel_on.load(Ordering::Acquire)
    }
}

fn test_limits() -> SceneConstructionLimits {
    SceneConstructionLimits {
        operation_live_bytes: u64::MAX,
        dimension_live_bytes: u64::MAX,
        attempted_work: u64::MAX,
        consumed_work: u64::MAX,
        committed_output: u64::MAX,
        published_evidence: u64::MAX,
    }
}

fn test_factory() -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
            .expect("node Scene test factory limits"),
    )
    .expect("node Scene test factory")
}

fn test_limits_with_work(work: u64) -> SceneConstructionLimits {
    SceneConstructionLimits {
        attempted_work: work,
        consumed_work: work,
        ..test_limits()
    }
}

fn single_trs_batch() -> NodeAuthoringBatch<'static> {
    NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &SINGLE_TRS_NODES)
}

fn parent(node: u32, child_ordinal: u32) -> Presence<NodeParent> {
    Presence::Present(NodeParent::new(NodeLocalId::new(node), child_ordinal))
}

fn test_account(
    factory: &PhysicalStorageAccountFactory,
    max_live_allocations: u64,
) -> baozi_operation::PhysicalStorageAccount {
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, max_live_allocations)
                .expect("node Scene test account limits"),
        )
        .expect("node Scene test account")
}

fn successful_parts(prepared: PreparedNodeSceneConstruction) -> super::NodeSceneSuccessParts {
    match prepared.handoff() {
        NodeSceneConstructionTerminal::Success(parts) => parts,
        NodeSceneConstructionTerminal::RichFailure(_) => {
            panic!("valid node Scene must not publish rich failure")
        }
        NodeSceneConstructionTerminal::MinimalFallback(_) => {
            panic!("valid node Scene must not publish minimal fallback")
        }
    }
}

fn rich_failure(prepared: PreparedNodeSceneConstruction) -> NodeSceneRichFailure {
    match prepared.handoff() {
        NodeSceneConstructionTerminal::Success(_) => {
            panic!("rejected node Scene must not publish canonical storage")
        }
        NodeSceneConstructionTerminal::RichFailure(failure) => failure,
        NodeSceneConstructionTerminal::MinimalFallback(_) => {
            panic!("recoverable node Scene rejection must preserve rich evidence")
        }
    }
}

fn assert_released(
    failure: NodeSceneRichFailure,
    account: &baozi_operation::PhysicalStorageAccount,
) {
    assert!(failure.account().is_same_account(account));
    assert_eq!(failure.outcome_evidence_block_count(), 1);
    drop(failure);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

fn terminal_failure_code(evidence: &baozi_operation::StoreLedgerEvidence) -> u32 {
    match evidence.header().terminal_cause() {
        StoreTerminalCause::Failure(cause) => cause.code(),
        StoreTerminalCause::Success => {
            panic!("rejected node Scene must freeze a failure cause")
        }
        _ => panic!("unknown terminal cause cannot prove a node Scene failure"),
    }
}

fn terminal_failure_class(
    evidence: &baozi_operation::StoreLedgerEvidence,
) -> StorePreparationFailureClass {
    match evidence.header().terminal_cause() {
        StoreTerminalCause::Failure(cause) => cause.class(),
        StoreTerminalCause::Success => {
            panic!("rejected node Scene must freeze a failure cause")
        }
        _ => panic!("unknown terminal cause cannot prove a node Scene failure"),
    }
}

fn matrix_transform(translation_x: f32) -> AuthoredTransform {
    AuthoredTransform::try_matrix(Mat4 {
        cols: [
            [1.0, -0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [translation_x, 2.0, -3.0, 1.0],
        ],
    })
    .expect("finite affine authored matrix")
}

fn trs_transform(translation_x: f32) -> AuthoredTransform {
    AuthoredTransform::try_trs(
        Vec3::new(translation_x, -0.0, 3.5),
        Quat::IDENTITY,
        Vec3::new(1.0, 2.0, 0.5),
    )
    .expect("valid authored TRS")
}

#[test]
fn source_ordered_forest_materializes_canonical_graph_and_preserves_authored_bits() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let generation_source = AtomicU64::new(0);
    let generation_before = SceneGenerationId::reserve_from(&generation_source)
        .expect("generation before node Scene construction")
        .into_id();
    let transforms = [
        matrix_transform(-4.0),
        trs_transform(1.0),
        trs_transform(2.0),
        matrix_transform(4.0),
        trs_transform(5.0),
    ];
    let nodes = [
        NodeAuthoringNode::new(parent(3, 1), transforms[0]),
        NodeAuthoringNode::new(Presence::Absent, transforms[1]),
        NodeAuthoringNode::new(parent(3, 0), transforms[2]),
        NodeAuthoringNode::new(Presence::Absent, transforms[3]),
        NodeAuthoringNode::new(parent(1, 0), transforms[4]),
    ];
    let roots = [NodeLocalId::new(3), NodeLocalId::new(1)];
    let prepared = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("fresh node Scene operation")
        .try_validate_node_batch(
            NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
            SceneValidationLimits::default(),
        )
        .unwrap_or_else(|_| panic!("valid source-ordered forest must validate"))
        .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&generation_source))
        .expect("valid source-ordered forest must prepare");
    let before_handoff = account.snapshot();
    let parts = successful_parts(prepared);
    let after_handoff = account.snapshot();
    let generation_after = SceneGenerationId::reserve_from(&generation_source)
        .expect("generation after node Scene construction")
        .into_id();

    assert_eq!(parts.scene.source_node_count(), Some(5));
    assert_eq!(parts.scene.source_root_count(), Some(2));
    assert_eq!(parts.scene.canonical_block_count(), 5);
    assert!(parts.scene.account().is_same_account(&account));
    assert!(generation_before < parts.scene.generation());
    assert!(parts.scene.generation() < generation_after);
    assert_eq!(generation_source.load(Ordering::Relaxed), 3);
    assert_eq!(before_handoff, after_handoff);
    assert_eq!(parts.scene.parent(0), Some(None));
    assert_eq!(parts.scene.parent(1), Some(None));
    assert_eq!(parts.scene.parent(2), Some(Some(0)));
    assert_eq!(parts.scene.parent(3), Some(Some(0)));
    assert_eq!(parts.scene.parent(4), Some(Some(1)));
    assert_eq!(parts.scene.parent(5), None);
    assert_eq!(parts.scene.child(None, 0), Some(0));
    assert_eq!(parts.scene.child(None, 1), Some(1));
    assert_eq!(parts.scene.child(None, 2), None);
    assert_eq!(parts.scene.child(Some(0), 0), Some(2));
    assert_eq!(parts.scene.child(Some(0), 1), Some(3));
    assert_eq!(parts.scene.child(Some(0), 2), None);
    assert_eq!(parts.scene.child(Some(1), 0), Some(4));
    assert_eq!(parts.scene.child(Some(2), 0), None);

    let canonical_local_order = [3, 1, 2, 0, 4];
    for (canonical_source, local) in canonical_local_order.into_iter().enumerate() {
        let inspected = parts
            .scene
            .authored_transform(canonical_source)
            .expect("canonical authored transform");
        assert!(inspected.has_same_authored_bits(transforms[local]));
    }
    assert_eq!(parts.report.outcome_evidence_block_count(), 1);
    let ledger = parts.report.ledger_evidence();
    let header = ledger.header();
    assert!(matches!(
        header.terminal_cause(),
        StoreTerminalCause::Success
    ));
    assert_eq!(header.dimension_count(), 1);
    assert_eq!(header.phase_count(), 1);
    assert_eq!(header.current_live_bytes(), 0);
    assert!(header.peak_live_bytes() > 0);
    assert_eq!(header.active_reservations(), 0);
    let dimension = ledger
        .dimension_at(0)
        .expect("node Scene dimension evidence");
    assert!(dimension.attempted() > 0);
    assert_eq!(dimension.consumed(), dimension.attempted());
    assert_eq!(dimension.current_live_bytes(), 0);
    assert!(dimension.peak_live_bytes() > 0);
    assert_eq!(dimension.pending_publication(), 0);
    assert_eq!(dimension.committed(), 5);
    assert_eq!(dimension.committed_canonical_output(), 5);
    assert_eq!(
        dimension.published_outcome_evidence(),
        2,
        "startup fallback and successful outcome evidence are both accounted"
    );

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn scene_input_admission_exceeds_the_legacy_fixed_slot_bound() {
    const PRIMITIVE_COUNT: usize = 12;

    let factory = test_factory();
    let account = test_account(&factory, 256);
    let roots = [];
    let nodes = [];
    let position = [0.0_f32, 0.0, 0.0];
    let index = [0_u32];
    let attributes = (0..PRIMITIVE_COUNT)
        .map(|_| {
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &position,
                Presence::Absent,
            )
        })
        .collect::<Vec<_>>();
    let primitives = (0..PRIMITIVE_COUNT)
        .map(|primitive| {
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::Points,
                Presence::Present(&index),
                Presence::Absent,
                AuthoringRange::new(u32::try_from(primitive).expect("test primitive index"), 1),
            )
        })
        .collect::<Vec<_>>();
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
        0,
        u32::try_from(PRIMITIVE_COUNT).expect("test primitive count"),
    ))];
    let input = SceneAuthoringBatch::new(
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        GeometryAuthoringBatch::new(0, &logical_meshes, &primitives, &attributes, &[]),
    );

    let prepared = SceneConstructionOperation::with_account_for_scene_input(
        &account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("dynamic Scene attribution admission")
    .try_validate()
    .unwrap_or_else(|_| panic!("dynamic Scene attribution validation"))
    .prepare()
    .expect("dynamic Scene attribution publication");
    let parts = successful_parts(prepared);

    assert_eq!(parts.scene.canonical_block_count(), 30);
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn present_empty_scene_publishes_scene_node_and_geometry_directories() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let roots = [];
    let nodes = [];
    let prepared = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("empty node Scene operation")
        .try_validate_node_batch(
            NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
            SceneValidationLimits::default(),
        )
        .unwrap_or_else(|_| panic!("present empty node Scene must validate"))
        .prepare()
        .expect("present empty node Scene must prepare");
    let parts = successful_parts(prepared);

    assert_eq!(parts.scene.source_node_count(), Some(0));
    assert_eq!(parts.scene.source_root_count(), Some(0));
    assert_eq!(parts.scene.canonical_block_count(), 3);
    assert_eq!(parts.scene.parent(0), None);
    assert_eq!(parts.scene.child(None, 0), None);
    assert_eq!(parts.scene.authored_transform(0), None);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_logical_byte_rejection_never_opens_publication() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let operation = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("canonical-byte-bounded node Scene operation");
    let before = account.snapshot();

    let rejected = match operation.try_validate_node_batch(
        single_trs_batch(),
        SceneValidationLimits::default().with_max_canonical_bytes(167),
    ) {
        Ok(_) => panic!("one byte below the canonical requirement must reject"),
        Err(rejected) => rejected,
    };
    assert!(matches!(
        rejected.failure(),
        NodeSceneConstructionRejection::NodeAuthoring(NodeAuthoringFailure::Semantic(
            NodeAuthoringSemanticError::LimitExceeded {
                limit: NodeAuthoringLimit::CanonicalBytes,
                actual: 168,
                maximum: 167,
            }
        ))
    ));
    let after_rejection = account.snapshot();
    assert_eq!(
        after_rejection, before,
        "authoring rejection must not enter physical publication"
    );

    let terminal = rejected
        .prepare_minimal_fallback()
        .expect("canonical byte rejection fallback")
        .handoff();
    assert!(matches!(
        &terminal,
        NodeSceneConstructionTerminal::MinimalFallback(_)
    ));
    drop(terminal);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn cancellation_after_authoring_validation_publishes_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let cancellation = CancellationToken::new();
    let validated = SceneConstructionOperation::with_account_and_cancellation_for_test(
        &account,
        test_limits(),
        cancellation.clone(),
    )
    .expect("cancellable node Scene operation")
    .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
    .unwrap_or_else(|_| panic!("single-node input must validate before cancellation"));

    cancellation.cancel();
    let failure = rich_failure(
        validated
            .prepare()
            .expect("post-validation cancellation must prepare rich failure"),
    );
    let work = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("node Scene work evidence");
    assert_eq!(work.attempted(), SINGLE_TRS_AUTHORING_VALIDATION_WORK);
    assert_eq!(work.consumed(), SINGLE_TRS_AUTHORING_VALIDATION_WORK);
    assert_released(failure, &account);
}

#[test]
fn empty_geometry_leaf_scan_observes_periodic_cancellation() {
    const PRIMITIVE_COUNT: usize = 300;
    const PRE_LEAF_AND_PERIODIC_POLLS: usize = 6;

    let factory = test_factory();
    let account = test_account(&factory, 256);
    let roots = [];
    let nodes = [];
    let primitives = (0..PRIMITIVE_COUNT)
        .map(|_| {
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::Points,
                Presence::Absent,
                Presence::Absent,
                AuthoringRange::new(0, 0),
            )
        })
        .collect::<Vec<_>>();
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
        0,
        u32::try_from(PRIMITIVE_COUNT).expect("test primitive count"),
    ))];
    let input = SceneAuthoringBatch::new(
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        GeometryAuthoringBatch::new(0, &logical_meshes, &primitives, &[], &[]),
    );
    let cancellation = CancelOnSignalPoll::new();
    let validated =
        SceneConstructionOperation::with_account_and_cancellation_for_scene_input_for_test(
            &account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
            cancellation.clone(),
        )
        .expect("empty-primitive Scene admission")
        .try_validate()
        .unwrap_or_else(|_| panic!("empty-primitive Scene validation"));
    let validation_polls = cancellation.polls();

    // Scratch contributes four polls, the synthetic node table starts the shared work meter,
    // and the sixth poll is reached by the 256-unit node-plus-empty-leaf work interval.
    cancellation.cancel_after_additional_polls(PRE_LEAF_AND_PERIODIC_POLLS);
    let failure = rich_failure(
        validated
            .prepare()
            .expect("empty geometry leaf cancellation must prepare rich failure"),
    );

    assert_eq!(
        terminal_failure_class(failure.ledger_evidence()),
        StorePreparationFailureClass::Control,
    );
    assert_eq!(
        cancellation.polls(),
        validation_polls + PRE_LEAF_AND_PERIODIC_POLLS,
    );
    let work = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("empty geometry cancellation work evidence");
    assert_eq!(work.attempted(), work.consumed());
    assert_released(failure, &account);
}

#[test]
fn materialization_work_budget_interruption_publishes_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let validated = SceneConstructionOperation::with_account(
        &account,
        test_limits_with_work(SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE),
    )
    .expect("work-bounded node Scene operation")
    .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
    .unwrap_or_else(|_| panic!("work budget must admit authoring validation exactly"));

    let failure = rich_failure(
        validated
            .prepare()
            .expect("materialization budget interruption must prepare rich failure"),
    );
    let work = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("node Scene work evidence");
    assert_eq!(
        work.attempted(),
        SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE + 10
    );
    assert_eq!(work.consumed(), SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE);
    assert_released(failure, &account);
}

#[test]
fn central_validation_work_budget_interruption_publishes_rich_failure() {
    const WORK_LIMIT: u64 = 61;

    let factory = test_factory();
    let account = test_account(&factory, 256);
    let validated =
        SceneConstructionOperation::with_account(&account, test_limits_with_work(WORK_LIMIT))
            .expect("validation-bounded node Scene operation")
            .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
            .unwrap_or_else(|_| panic!("work budget must admit authoring validation"));

    let failure = rich_failure(
        validated
            .prepare()
            .expect("central validation budget interruption must prepare rich failure"),
    );
    let work = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("node Scene work evidence");
    assert_eq!(work.attempted(), WORK_LIMIT + 1);
    assert_eq!(work.consumed(), WORK_LIMIT);
    assert_released(failure, &account);
}

#[test]
fn generation_exhaustion_discards_canonical_storage_and_preserves_evidence() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let validated = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("node Scene generation test operation")
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
        .unwrap_or_else(|_| panic!("single-node input must validate"));

    let failure = rich_failure(
        validated
            .prepare_with_generation_source(|| Err(SceneGenerationError::Exhausted))
            .expect("generation exhaustion must prepare rich failure"),
    );
    let work = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("node Scene work evidence");
    assert_eq!(work.attempted(), SINGLE_TRS_GENERATION_FAILURE_WORK);
    assert_released(failure, &account);
}

#[test]
fn authored_transform_bit_fault_is_a_build_invariant_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let validated = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("node Scene semantic fault operation")
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
        .unwrap_or_else(|_| panic!("single-node input must validate"));

    let failure = rich_failure(
        validated
            .prepare_with_faults(NodeSceneBuildFaults::altered_authored_transform())
            .expect("authored transform mismatch must prepare rich failure"),
    );
    let work = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("node Scene work evidence");
    assert_eq!(work.attempted(), SINGLE_TRS_BUILD_INVARIANT_FAILURE_WORK);
    assert_eq!(
        terminal_failure_code(failure.ledger_evidence()),
        NODE_SCENE_BUILD_INVARIANT_CODE
    );
    assert_released(failure, &account);
}

#[test]
fn geometry_payload_bit_fault_is_a_build_invariant_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let roots = [];
    let nodes = [];
    let positions = [0.0_f32, 0.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let input = SceneAuthoringBatch::new(
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        GeometryAuthoringBatch::new(0, &logical_meshes, &primitives, &attributes, &[]),
    );
    let validated = SceneConstructionOperation::with_account_for_scene_input(
        &account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("geometry semantic fault operation")
    .try_validate()
    .unwrap_or_else(|_| panic!("single-point geometry must validate"));

    let failure = rich_failure(
        validated
            .prepare_with_faults(NodeSceneBuildFaults::altered_geometry_f32())
            .expect("geometry payload mismatch must prepare rich failure"),
    );
    assert_eq!(
        terminal_failure_code(failure.ledger_evidence()),
        NODE_SCENE_BUILD_INVARIANT_CODE,
    );
    assert_released(failure, &account);
}

#[test]
fn evidence_publication_limit_rejection_fails_closed() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let limits = SceneConstructionLimits {
        published_evidence: 2,
        ..test_limits()
    };
    let validated = SceneConstructionOperation::with_account(&account, limits)
        .expect("baseline evidence limit must admit operation startup")
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
        .unwrap_or_else(|_| panic!("single-node input must validate"));

    let terminal = validated
        .prepare_with_faults(NodeSceneBuildFaults::excess_outcome_evidence_commitment())
        .expect("evidence publication rejection must prepare fail closed")
        .handoff();
    let NodeSceneConstructionTerminal::MinimalFallback(fallback) = terminal else {
        panic!("unbound evidence must terminate as minimal fallback");
    };
    assert_eq!(
        terminal_failure_class(fallback.ledger_evidence()),
        StorePreparationFailureClass::Ledger
    );
    drop(fallback);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_publication_limit_rejection_preserves_evidence_and_burns_generation() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let generation_source = AtomicU64::new(0);
    let limits = SceneConstructionLimits {
        committed_output: 0,
        ..test_limits()
    };
    let validated = SceneConstructionOperation::with_account(&account, limits)
        .expect("zero canonical commitment limit operation")
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
        .unwrap_or_else(|_| panic!("single-node input must validate"));

    let terminal = validated
        .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&generation_source))
        .expect("canonical publication rejection must prepare fail closed")
        .handoff();
    let NodeSceneConstructionTerminal::RichFailure(failure) = terminal else {
        panic!("canonical rejection must preserve validated outcome evidence");
    };
    assert_eq!(
        terminal_failure_class(failure.ledger_evidence()),
        StorePreparationFailureClass::Ledger
    );
    assert_eq!(generation_source.load(Ordering::Relaxed), 1);
    assert_released(failure, &account);

    let _next_generation = SceneGenerationId::reserve_from(&generation_source)
        .expect("generation after burned gap must remain reservable");
    assert_eq!(generation_source.load(Ordering::Relaxed), 2);
}

#[test]
fn cycle_and_derived_lineage_can_only_terminate_as_fallback() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let cycle_nodes = [
        NodeAuthoringNode::new(Presence::Absent, AuthoredTransform::identity_trs()),
        NodeAuthoringNode::new(parent(2, 0), AuthoredTransform::identity_trs()),
        NodeAuthoringNode::new(parent(1, 0), AuthoredTransform::identity_trs()),
    ];
    let cycle_roots = [NodeLocalId::new(0)];
    let rejected = match SceneConstructionOperation::with_account(&account, test_limits())
        .expect("cycle rejection operation")
        .try_validate_node_batch(
            NodeAuthoringBatch::new(Presence::Present(&cycle_roots), &cycle_nodes),
            SceneValidationLimits::default(),
        ) {
        Ok(_) => panic!("cycle must not reach validated construction"),
        Err(rejected) => rejected,
    };
    assert!(matches!(
        rejected.failure(),
        &NodeSceneConstructionRejection::NodeAuthoring(NodeAuthoringFailure::Semantic(
            NodeAuthoringSemanticError::Cycle { .. }
        ))
    ));
    let cycle_terminal = rejected
        .prepare_minimal_fallback()
        .expect("cycle rejection fallback")
        .handoff();
    assert!(matches!(
        cycle_terminal,
        NodeSceneConstructionTerminal::MinimalFallback(_)
    ));
    drop(cycle_terminal);

    let legacy = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("legacy source operation")
        .prepare_fresh_two_node(TwoNodeAuthoredMatrices::new(Mat4::IDENTITY, Mat4::IDENTITY))
        .expect("legacy source preparation");
    let legacy_scene = match legacy.handoff() {
        SceneConstructionTerminal::Success(parts) => parts.scene,
        SceneConstructionTerminal::RichFailure(_) => {
            panic!("legacy source must succeed")
        }
        SceneConstructionTerminal::MinimalFallback(_) => {
            panic!("legacy source must succeed")
        }
    };
    let empty_roots = [];
    let empty_nodes = [];
    let rejected =
        match SceneConstructionOperation::derived_from_exact_input(&legacy_scene, test_limits())
            .expect("derived node Scene operation")
            .try_validate_node_batch(
                NodeAuthoringBatch::new(Presence::Present(&empty_roots), &empty_nodes),
                SceneValidationLimits::default(),
            ) {
            Ok(_) => panic!("derived lineage must not reach validated construction"),
            Err(rejected) => rejected,
        };
    assert!(matches!(
        rejected.failure(),
        &NodeSceneConstructionRejection::DerivedLineage
    ));
    let derived_terminal = rejected
        .prepare_minimal_fallback()
        .expect("derived rejection fallback")
        .handoff();
    assert!(matches!(
        derived_terminal,
        NodeSceneConstructionTerminal::MinimalFallback(_)
    ));
    drop(derived_terminal);
    drop(legacy_scene);

    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn physical_budget_boundaries_release_storage_and_preserve_committed_evidence() {
    let factory = test_factory();
    let mut observed_fail_closed_publication = false;
    let mut observed_rich_storage_rejection = false;

    for max_live_allocations in 1..=128 {
        let account = test_account(&factory, max_live_allocations);
        let operation = match SceneConstructionOperation::with_account(&account, test_limits()) {
            Ok(operation) => operation,
            Err(_) => {
                assert_eq!(account.snapshot().live_retained_bytes(), 0);
                assert_eq!(account.snapshot().live_retained_allocations(), 0);
                drop(account);
                continue;
            }
        };
        let roots = [NodeLocalId::new(0)];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
        )];
        let validated = operation
            .try_validate_node_batch(
                NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
                SceneValidationLimits::default(),
            )
            .unwrap_or_else(|_| panic!("physical output limits must not reject semantics"));
        let prepared = validated
            .prepare()
            .expect("physical output failure must retain terminal fallback");
        let terminal = prepared.handoff();
        match terminal {
            NodeSceneConstructionTerminal::Success(parts) => drop(parts),
            NodeSceneConstructionTerminal::RichFailure(failure) => {
                if terminal_failure_code(failure.ledger_evidence())
                    == NODE_SCENE_STORAGE_REJECTION_CODE
                {
                    assert_eq!(failure.outcome_evidence_block_count(), 1);
                    observed_rich_storage_rejection = true;
                }
                drop(failure);
            }
            NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
                observed_fail_closed_publication = true;
                drop(fallback);
            }
        }
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);

        if observed_fail_closed_publication && observed_rich_storage_rejection {
            break;
        }
    }

    assert!(
        observed_fail_closed_publication,
        "at least one low physical allocation budget must fail closed during publication"
    );
    assert!(
        observed_rich_storage_rejection,
        "a budget that admits evidence but rejects canonical storage must retain rich evidence"
    );
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn internal_block_allocation_failure_is_a_storage_rejection() {
    assert_eq!(
        post_evidence_build_failure_code(baozi_operation::StoreBuildFailureClass::AllocationFailed,),
        Some(NODE_SCENE_STORAGE_REJECTION_CODE)
    );
    assert_eq!(
        post_evidence_build_failure_code(
            baozi_operation::StoreBuildFailureClass::CapacityEnvelopeViolated,
        ),
        None
    );
}
