use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use baozi_operation::{
    CancellationSignal, CancellationToken, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
    StorePreparationFailureClass, StoreRuntimeError, StoreTerminalCause,
};

use crate::FormatId;
use crate::SceneValidationLimits;
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::construction::node_scene::semantic::{
    NodeSceneProfileCursor, NodeSceneProfileStep,
};
use crate::canonical_scene::store::construction::{
    FAILURE_TERMINAL_WORK_DIMENSION, FALLBACK_CODE, OPEN_PLAN_REJECTION_CODE,
    OPEN_RESOURCE_REJECTION_CODE, STORE_DIMENSION, SceneConstructionLimits,
    SceneConstructionOperation, failure_terminal_work_requirement, tracer_scene_open_failure_code,
};
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AuthoringRange, DeformationAuthoringBatch,
    GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, LineageAuthoringBatch,
    LogicalMorphAuthoring, MetadataAuthoringBatch, MetadataAuthoringEntry, NodeAuthoringBatch,
    NodeAuthoringFailure, NodeAuthoringLimit, NodeAuthoringNode, NodeAuthoringSemanticError,
    NodeLocalId, NodeParent, ObservedResourceAuthoring, OperationAuthoringControlError,
    OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentityLocalId,
    SceneAuthoringBatch, SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId,
    SpatialAuthoringBatch, SpatialAuthoringFailure,
};
use crate::canonical_scene::store::profile_provenance::{
    PROFILE_PROVENANCE_ADDRESS_SCALARS, corrupt_profile_address_for_test,
};
use crate::entity::AuthoredTransform;
use crate::extension::ExtensionValueRef;
use crate::identity::{SceneGenerationError, SceneGenerationId};
use crate::math::{Mat4, Quat, Vec3};
use crate::namespaced::NamespacedName;
use crate::provenance::{ResourceIdentity, SourceObjectKind, SourceObjectLocator};
use crate::scene_space::SceneSpace;

use super::profile::ProfileBuildFault;
use super::publication::post_evidence_build_failure_code;
use super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_NODE_AUTHORING_REJECTION_CODE,
    NODE_SCENE_STORAGE_REJECTION_CODE, NodeSceneBuildFaults, NodeSceneConstructionRejection,
    NodeSceneConstructionTerminal, NodeSceneDraft, NodeSceneDraftBuild, NodeSceneRichFailure,
    PendingNodeSceneDraft, PreparedNodeSceneConstruction,
};

mod chunking;
mod gltf_capability;
mod obj_capability;

fn exhaust_profile_for_test(profile: &mut NodeSceneProfileCursor<'_>) {
    while !matches!(
        profile.step_resource().expect("profile resource step"),
        NodeSceneProfileStep::End
    ) {}
    while !matches!(
        profile.step_source().expect("profile source step"),
        NodeSceneProfileStep::End
    ) {}
    while !matches!(
        profile.step_provenance().expect("profile provenance step"),
        NodeSceneProfileStep::End
    ) {}
    while !matches!(
        profile.step_evidence().expect("profile evidence step"),
        NodeSceneProfileStep::End
    ) {}
    while profile.next_owner().expect("profile owner read").is_some() {}
    while profile
        .next_metadata_entry()
        .expect("profile metadata read")
        .is_some()
    {}
}

pub(in crate::canonical_scene::store) use gltf_capability::{
    GLTF_SOURCE_VECTOR_SCHEMA_ID, GltfCapabilityCanonicalShapeObservation,
    GltfCapabilitySourceShapeObservation, build_gltf_capability_scene_for_probe,
    capture_gltf_capability_snapshots_for_probe, collect_gltf_capability_vectors_for_probe,
    inspect_gltf_capability_semantics_for_probe,
    inspect_gltf_capability_semantics_with_first_transform_for_probe,
};

pub(in crate::canonical_scene::store) use obj_capability::{
    OBJ_SOURCE_VECTOR_SCHEMA_ID, ObjCapabilityCanonicalShapeObservation,
    ObjCapabilitySourceShapeObservation, build_obj_capability_scene_for_probe,
    capture_obj_capability_snapshots_for_probe, collect_obj_capability_vectors_for_probe,
    inspect_obj_capability_semantics_for_probe,
};

const SINGLE_TRS_DOMAIN_AUTHORING_VALIDATION_WORK: u64 = 43;
const SINGLE_TRS_PROFILE_OWNER_COUNT: u64 = 3;
const SINGLE_TRS_PROFILE_PROVENANCE_COUNT: u64 = 1;
const SINGLE_TRS_PROFILE_ATTACHMENT_COUNT: u64 = 3;
const SINGLE_TRS_PROFILE_OPERATION_SEMANTIC_CELLS: u64 = 5;
const SINGLE_TRS_PROFILE_DIGEST_BYTES: u64 = 32;
const NODE_ONLY_PROFILE_CANONICAL_BLOCK_COUNT: u64 = 8;
const SOURCE_BACKED_PROFILE_CANONICAL_BLOCK_COUNT: u64 = 10;
const RETAINED_HANDLE_SHARE_WORK_PER_BLOCK: u64 = 2;
const SINGLE_TRS_PROFILE_AUTHORING_VALIDATION_WORK: u64 = SINGLE_TRS_PROFILE_OWNER_COUNT
    + SINGLE_TRS_PROFILE_PROVENANCE_COUNT
    + SINGLE_TRS_PROFILE_OPERATION_SEMANTIC_CELLS
    + SINGLE_TRS_PROFILE_OWNER_COUNT
    + SINGLE_TRS_PROFILE_ATTACHMENT_COUNT * 2
    + SINGLE_TRS_PROFILE_OWNER_COUNT * 3
    + SINGLE_TRS_PROFILE_OWNER_COUNT;
const SINGLE_TRS_AUTHORING_VALIDATION_WORK: u64 =
    SINGLE_TRS_DOMAIN_AUTHORING_VALIDATION_WORK + SINGLE_TRS_PROFILE_AUTHORING_VALIDATION_WORK;
const SINGLE_TRS_PROFILE_CONTENT_MATERIALIZATION_WORK: u64 = SINGLE_TRS_PROFILE_PROVENANCE_COUNT
    * 2
    + SINGLE_TRS_PROFILE_DIGEST_BYTES
    + SINGLE_TRS_PROFILE_OWNER_COUNT
    + SINGLE_TRS_PROFILE_ATTACHMENT_COUNT * (1 + PROFILE_PROVENANCE_ADDRESS_SCALARS as u64)
    + SINGLE_TRS_PROFILE_PROVENANCE_COUNT * 6
    + SINGLE_TRS_PROFILE_PROVENANCE_COUNT * 9
    + 1
    + 1
    + SINGLE_TRS_PROFILE_OWNER_COUNT
    + 1;
const SINGLE_TRS_PROFILE_MATERIALIZATION_WORK: u64 = SINGLE_TRS_PROFILE_CONTENT_MATERIALIZATION_WORK
    + NODE_ONLY_PROFILE_CANONICAL_BLOCK_COUNT * RETAINED_HANDLE_SHARE_WORK_PER_BLOCK;
const SINGLE_TRS_PROFILE_SCRATCH_CONSTRUCTION_WORK: u64 = SINGLE_TRS_PROFILE_PROVENANCE_COUNT;
const SINGLE_TRS_PROFILE_SCRATCH_VALIDATION_RESET_WORK: u64 = SINGLE_TRS_PROFILE_PROVENANCE_COUNT;
const SINGLE_TRS_PROFILE_GRAPH_VALIDATION_WORK: u64 = 1
    + 2
    + SINGLE_TRS_PROFILE_PROVENANCE_COUNT * 2
    + SINGLE_TRS_PROFILE_PROVENANCE_COUNT * 2
    + SINGLE_TRS_PROFILE_DIGEST_BYTES
    + SINGLE_TRS_PROFILE_OWNER_COUNT
    + SINGLE_TRS_PROFILE_ATTACHMENT_COUNT
    + 1;
const SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE: u64 = SINGLE_TRS_AUTHORING_VALIDATION_WORK + 3;
const SINGLE_TRS_DOMAIN_BUILD_INVARIANT_WORK_AFTER_AUTHORING: u64 = 64;
const SINGLE_TRS_BUILD_INVARIANT_FAILURE_WORK: u64 = SINGLE_TRS_AUTHORING_VALIDATION_WORK
    + SINGLE_TRS_PROFILE_MATERIALIZATION_WORK
    + SINGLE_TRS_PROFILE_SCRATCH_CONSTRUCTION_WORK
    + SINGLE_TRS_PROFILE_SCRATCH_VALIDATION_RESET_WORK
    + SINGLE_TRS_DOMAIN_BUILD_INVARIANT_WORK_AFTER_AUTHORING;
const SINGLE_TRS_DOMAIN_GENERATION_FAILURE_WORK_AFTER_AUTHORING: u64 = 69;
const SINGLE_TRS_GENERATION_FAILURE_WORK: u64 = SINGLE_TRS_AUTHORING_VALIDATION_WORK
    + SINGLE_TRS_PROFILE_MATERIALIZATION_WORK
    + SINGLE_TRS_PROFILE_SCRATCH_CONSTRUCTION_WORK
    + SINGLE_TRS_PROFILE_SCRATCH_VALIDATION_RESET_WORK
    + SINGLE_TRS_PROFILE_GRAPH_VALIDATION_WORK
    + SINGLE_TRS_DOMAIN_GENERATION_FAILURE_WORK_AFTER_AUTHORING;
const SINGLE_TRS_DOMAIN_CENTRAL_VALIDATION_INTERRUPTION_WORK_AFTER_AUTHORING: u64 = 18;
const SINGLE_TRS_CENTRAL_VALIDATION_INTERRUPTION_WORK: u64 = SINGLE_TRS_AUTHORING_VALIDATION_WORK
    + SINGLE_TRS_PROFILE_MATERIALIZATION_WORK
    + SINGLE_TRS_PROFILE_SCRATCH_CONSTRUCTION_WORK
    + SINGLE_TRS_PROFILE_SCRATCH_VALIDATION_RESET_WORK
    + SINGLE_TRS_DOMAIN_CENTRAL_VALIDATION_INTERRUPTION_WORK_AFTER_AUTHORING;
const SINGLE_TRS_ROOTS: [NodeLocalId; 1] = [NodeLocalId::new(0)];
const SINGLE_TRS_NODES: [NodeAuthoringNode; 1] = [NodeAuthoringNode::new(
    Presence::Absent,
    AuthoredTransform::identity_trs(),
    Presence::Absent,
    Presence::Absent,
)];

const PROFILE_SOURCE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];

struct ProfileFixtureVocabulary {
    resource: ResourceIdentity,
    format: FormatId,
    locator: SourceObjectLocator,
}

fn profile_fixture_vocabulary() -> &'static ProfileFixtureVocabulary {
    static VOCABULARY: OnceLock<ProfileFixtureVocabulary> = OnceLock::new();
    VOCABULARY.get_or_init(|| ProfileFixtureVocabulary {
        resource: ResourceIdentity::new("fixtures/node-scene/input")
            .expect("test profile resource identity"),
        format: FormatId::new("org.baozi.fixture").expect("test profile format"),
        locator: SourceObjectLocator::indexed(SourceObjectKind::Document, 0),
    })
}

pub(super) struct ProfileFixture {
    resources: [ObservedResourceAuthoring<'static>; 1],
    sources: [SourceIdentityAuthoring<'static>; 1],
    provenance: [ProvenanceNodeAuthoring<'static>; 1],
    owners: Vec<OwnerProvenanceAuthoring<'static>>,
}

impl ProfileFixture {
    pub(super) fn batch(&self) -> ProfileAuthoringBatch<'_> {
        self.batch_with_metadata(MetadataAuthoringBatch::empty())
    }

    pub(super) fn batch_with_metadata<'fixture>(
        &'fixture self,
        metadata: MetadataAuthoringBatch<'fixture>,
    ) -> ProfileAuthoringBatch<'fixture> {
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(
                &self.resources,
                &self.sources,
                &self.provenance,
                &self.owners,
            ),
            metadata,
        )
    }
}

pub(super) fn profile_fixture(
    spatial: SpatialAuthoringBatch<'_>,
    nodes: NodeAuthoringBatch<'_>,
    appearance: AppearanceAuthoringBatch<'_>,
    geometry: GeometryAuthoringBatch<'_>,
    deformation: DeformationAuthoringBatch<'_>,
    animation: AnimationAuthoringBatch<'_>,
) -> ProfileFixture {
    let vocabulary = profile_fixture_vocabulary();
    let mut owners = vec![ProfileOwner::Scene, ProfileOwner::SyntheticRoot];
    owners.extend(nodes.nodes().iter().enumerate().map(|(index, _)| {
        ProfileOwner::Node(NodeLocalId::new(
            u32::try_from(index).expect("test node index"),
        ))
    }));
    owners.extend(indexed_profile_owners(
        spatial.cameras().len(),
        ProfileOwner::Camera,
    ));
    owners.extend(indexed_profile_owners(
        spatial.lights().len(),
        ProfileOwner::Light,
    ));
    owners.extend(indexed_profile_owners(
        appearance.images().len(),
        ProfileOwner::Image,
    ));
    owners.extend(indexed_profile_owners(
        appearance.samplers().len(),
        ProfileOwner::Sampler,
    ));
    owners.extend(indexed_profile_owners(
        appearance.textures().len(),
        ProfileOwner::Texture,
    ));
    owners.extend(indexed_profile_owners(
        appearance.bindings().len(),
        ProfileOwner::TextureBinding,
    ));
    owners.extend(indexed_profile_owners(
        appearance.materials().len(),
        ProfileOwner::Material,
    ));
    owners.extend(indexed_profile_owners(
        geometry.logical_meshes().len(),
        ProfileOwner::LogicalMesh,
    ));
    owners.extend(indexed_profile_owners(
        geometry.primitives().len(),
        ProfileOwner::Primitive,
    ));
    owners.extend(indexed_profile_owners(
        geometry.attributes().len(),
        ProfileOwner::Attribute,
    ));
    owners.extend(indexed_profile_owners(
        geometry.mesh_instances().len(),
        ProfileOwner::MeshInstance,
    ));
    owners.extend(indexed_profile_owners(
        deformation.skins().len(),
        ProfileOwner::Skin,
    ));
    owners.extend(indexed_profile_owners(
        deformation.morph_target_definitions().len(),
        ProfileOwner::MorphDefinition,
    ));
    let morph_delta_count = deformation
        .primitives()
        .iter()
        .flat_map(|primitive| primitive.morph_targets())
        .map(|target| target.deltas().len())
        .sum();
    owners.extend(indexed_profile_owners(
        morph_delta_count,
        ProfileOwner::MorphDelta,
    ));
    owners.extend(indexed_profile_owners(
        animation.clips().len(),
        ProfileOwner::AnimationClip,
    ));
    owners.extend(indexed_profile_owners(
        animation.channels().len(),
        ProfileOwner::AnimationChannel,
    ));

    ProfileFixture {
        resources: [ObservedResourceAuthoring::session_opaque(
            &vocabulary.resource,
            SessionOpaqueReason::CallerLabel,
        )],
        sources: [SourceIdentityAuthoring::new(
            ResourceIdentityLocalId::new(0),
            &vocabulary.format,
            &vocabulary.locator,
        )],
        provenance: [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
            0,
        ))],
        owners: owners
            .into_iter()
            .map(|owner| OwnerProvenanceAuthoring::new(owner, &PROFILE_SOURCE_PROVENANCE))
            .collect(),
    }
}

fn indexed_profile_owners(
    len: usize,
    owner: impl Fn(u32) -> ProfileOwner,
) -> impl Iterator<Item = ProfileOwner> {
    (0..len).map(move |index| owner(u32::try_from(index).expect("test profile owner index")))
}

#[test]
fn tracer_open_recovery_classifies_exact_cause_before_fallback() {
    let plan_code = tracer_scene_open_failure_code(&StoreRuntimeError::InvalidPublicationPlan);
    let allocation_code = tracer_scene_open_failure_code(&StoreRuntimeError::AllocationFailed);

    assert_eq!(plan_code, OPEN_PLAN_REJECTION_CODE);
    assert_eq!(allocation_code, OPEN_RESOURCE_REJECTION_CODE);
    assert_ne!(plan_code, FALLBACK_CODE);
    assert_ne!(allocation_code, FALLBACK_CODE);
    assert_ne!(plan_code, allocation_code);
}

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

pub(super) fn test_limits() -> SceneConstructionLimits {
    SceneConstructionLimits {
        operation_live_bytes: u64::MAX,
        dimension_live_bytes: u64::MAX,
        attempted_work: u64::MAX,
        consumed_work: u64::MAX,
        committed_output: u64::MAX,
        published_evidence: u64::MAX,
    }
}

pub(super) fn test_factory() -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
            .expect("node Scene test factory limits"),
    )
    .expect("node Scene test factory")
}

fn test_limits_with_work(work: u64) -> SceneConstructionLimits {
    let terminal_work = failure_terminal_work_requirement()
        .expect("node Scene failure-terminal work requirement")
        .attempted_work_limit();
    let total_work = work
        .checked_add(terminal_work)
        .expect("test ordinary and terminal work limits must fit");
    SceneConstructionLimits {
        attempted_work: total_work,
        consumed_work: total_work,
        ..test_limits()
    }
}

pub(super) fn single_trs_batch() -> NodeAuthoringBatch<'static> {
    NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &SINGLE_TRS_NODES)
}

fn parent(node: u32, child_ordinal: u32) -> Presence<NodeParent> {
    Presence::Present(NodeParent::new(NodeLocalId::new(node), child_ordinal))
}

pub(super) fn test_account(
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

pub(super) fn successful_parts(
    prepared: PreparedNodeSceneConstruction,
) -> super::NodeSceneSuccessParts {
    match prepared.handoff() {
        NodeSceneConstructionTerminal::Success(parts) => parts,
        NodeSceneConstructionTerminal::RichFailure(_) => {
            panic!("valid node Scene must not publish rich failure")
        }
        NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
            panic!(
                "valid node Scene must not publish minimal fallback: class={:?}, code={}",
                terminal_failure_class(fallback.ledger_evidence()),
                terminal_failure_code(fallback.ledger_evidence()),
            )
        }
    }
}

pub(in crate::canonical_scene::store) fn build_evidence_publication_limit_terminal_for_probe(
    account: &baozi_operation::PhysicalStorageAccount,
) -> PreparedNodeSceneConstruction {
    let limits = SceneConstructionLimits {
        published_evidence: 2,
        ..test_limits()
    };
    SceneConstructionOperation::with_account(account, limits)
        .expect("baseline evidence limit must admit operation startup")
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
        .unwrap_or_else(|_| panic!("single-node input must validate"))
        .prepare_with_faults(NodeSceneBuildFaults::excess_outcome_evidence_commitment())
        .expect("evidence publication rejection must prepare fail closed")
}

pub(in crate::canonical_scene::store) fn build_protected_terminal_activation_for_probe(
    account: &baozi_operation::PhysicalStorageAccount,
) -> PreparedNodeSceneConstruction {
    const ORDINARY_WORK_LIMIT: u64 = 10;

    let operation = SceneConstructionOperation::with_account(
        account,
        test_limits_with_work(ORDINARY_WORK_LIMIT),
    )
    .expect("protected-terminal probe operation must start");
    let rejected = match operation
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
    {
        Ok(_) => panic!("ordinary work exhaustion must reject authoring"),
        Err(rejected) => rejected,
    };
    let (prepared, rejection) = rejected
        .prepare_rich_failure()
        .expect("ordinary work exhaustion must retain protected rich evidence");
    assert!(matches!(
        rejection,
        NodeSceneConstructionRejection::NodeAuthoring(NodeAuthoringFailure::Control(
            OperationAuthoringControlError::Store(_)
        ))
    ));
    prepared
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

fn finish_draft_from_local_authoring_vecs(
    account: &baozi_operation::PhysicalStorageAccount,
) -> NodeSceneDraftBuild {
    let roots = vec![NodeLocalId::new(0)];
    let nodes = vec![NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    SceneConstructionOperation::with_account(account, test_limits())
        .expect("local-buffer node Scene operation")
        .try_validate_node_batch(
            NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
            SceneValidationLimits::default(),
        )
        .unwrap_or_else(|_| panic!("local authoring buffers must validate"))
        .finish_draft()
        .expect("local authoring buffers must materialize an owned draft")
}

fn exact_draft_pair_from_local_authoring_vecs(
    account: &baozi_operation::PhysicalStorageAccount,
) -> (NodeSceneDraft, PendingNodeSceneDraft) {
    match finish_draft_from_local_authoring_vecs(account) {
        NodeSceneDraftBuild::Draft { draft, owner } => (draft, owner),
        NodeSceneDraftBuild::Failure(_) => {
            panic!("valid materialization must preserve an exact canonical draft pair")
        }
    }
}

#[test]
fn materialized_draft_outlives_source_buffers_before_central_prepare_and_handoff() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let generation_source = AtomicU64::new(0);

    // The helper's Vec-backed authoring storage is gone before central validation starts.
    let (draft, owner) = exact_draft_pair_from_local_authoring_vecs(&account);
    let prepared = owner
        .prepare_with_generation_source(draft, || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("owned draft must centrally prepare without source buffers");
    let parts = successful_parts(prepared);

    assert_eq!(parts.scene.source_node_count(), Some(1));
    assert_eq!(parts.scene.source_root_count(), Some(1));
    assert_eq!(generation_source.load(Ordering::Relaxed), 1);
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn dropping_draft_descriptor_does_not_release_its_retained_owner() {
    let factory = test_factory();
    let account = test_account(&factory, 256);

    let (draft, owner) = exact_draft_pair_from_local_authoring_vecs(&account);
    let retained = account.snapshot();
    assert!(retained.live_retained_bytes() > 0);
    assert!(retained.live_retained_allocations() > 0);

    {
        let _descriptor = draft;
    }

    assert_eq!(account.snapshot(), retained);
    drop(owner);

    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn dropping_draft_owner_releases_storage_while_descriptor_remains_inert() {
    let factory = test_factory();
    let account = test_account(&factory, 256);

    let (_descriptor, owner) = exact_draft_pair_from_local_authoring_vecs(&account);
    assert!(account.snapshot().live_retained_bytes() > 0);
    assert!(account.snapshot().live_retained_allocations() > 0);

    drop(owner);

    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn foreign_brand_descriptor_fails_before_generation_reservation() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let (local_draft, local_owner) = exact_draft_pair_from_local_authoring_vecs(&account);
    let (foreign_draft, foreign_owner) = exact_draft_pair_from_local_authoring_vecs(&account);
    let mismatched = NodeSceneDraft {
        brand: foreign_draft.brand,
        directory: local_draft.directory,
    };
    drop(foreign_owner);

    let failure = rich_failure(
        local_owner
            .prepare_with_generation_source(mismatched, || {
                panic!("foreign brand must fail before generation reservation")
            })
            .expect("foreign brand must preserve rich build-invariant evidence"),
    );
    assert_eq!(
        terminal_failure_code(failure.ledger_evidence()),
        NODE_SCENE_BUILD_INVARIANT_CODE
    );
    assert_eq!(
        failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("foreign-brand failure dimension")
            .committed_canonical_output(),
        0
    );
    assert_released(failure, &account);
}

#[test]
fn foreign_directory_descriptor_fails_before_generation_reservation() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let (local_draft, local_owner) = exact_draft_pair_from_local_authoring_vecs(&account);
    let (foreign_draft, foreign_owner) = exact_draft_pair_from_local_authoring_vecs(&account);
    let mismatched = NodeSceneDraft {
        brand: local_draft.brand,
        directory: foreign_draft.directory,
    };
    drop(foreign_owner);

    let failure = rich_failure(
        local_owner
            .prepare_with_generation_source(mismatched, || {
                panic!("foreign directory must fail before generation reservation")
            })
            .expect("foreign directory must preserve rich build-invariant evidence"),
    );
    assert_eq!(
        terminal_failure_code(failure.ledger_evidence()),
        NODE_SCENE_BUILD_INVARIANT_CODE
    );
    assert_eq!(
        failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("foreign-directory failure dimension")
            .committed_canonical_output(),
        0
    );
    assert_released(failure, &account);
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
        NodeAuthoringNode::new(
            parent(3, 1),
            transforms[0],
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            transforms[1],
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            parent(3, 0),
            transforms[2],
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            transforms[3],
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            parent(1, 0),
            transforms[4],
            Presence::Absent,
            Presence::Absent,
        ),
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

    let expected_canonical_block_count =
        9 + usize::try_from(NODE_ONLY_PROFILE_CANONICAL_BLOCK_COUNT)
            .expect("node-only profile canonical block count");
    assert_eq!(parts.scene.source_node_count(), Some(5));
    assert_eq!(parts.scene.source_root_count(), Some(2));
    assert_eq!(
        parts.scene.canonical_block_count(),
        expected_canonical_block_count
    );
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
    assert_eq!(header.dimension_count(), 2);
    assert_eq!(header.phase_count(), 1);
    assert_eq!(header.current_live_bytes(), 0);
    assert!(header.peak_live_bytes() > 0);
    assert_eq!(header.active_reservations(), 0);
    let dimension = ledger
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("node Scene dimension evidence");
    assert!(dimension.attempted() > 0);
    assert_eq!(dimension.consumed(), dimension.attempted());
    assert_eq!(dimension.current_live_bytes(), 0);
    assert!(dimension.peak_live_bytes() > 0);
    assert_eq!(dimension.pending_publication(), 0);
    assert_eq!(
        dimension.committed(),
        u64::try_from(expected_canonical_block_count).expect("node Scene canonical block count")
    );
    assert_eq!(
        dimension.committed_canonical_output(),
        u64::try_from(expected_canonical_block_count).expect("node Scene canonical block count")
    );
    assert_eq!(
        dimension.published_outcome_evidence(),
        2,
        "startup fallback and successful outcome evidence are both accounted"
    );
    let failure_terminal_work = ledger
        .dimension_by_semantic(FAILURE_TERMINAL_WORK_DIMENSION)
        .expect("failure-terminal work dimension evidence");
    assert_eq!(failure_terminal_work.attempted(), 0);
    assert_eq!(failure_terminal_work.consumed(), 0);
    assert_eq!(failure_terminal_work.current_live_bytes(), 0);
    assert_eq!(failure_terminal_work.peak_live_bytes(), 0);
    assert_eq!(failure_terminal_work.pending_publication(), 0);
    assert_eq!(failure_terminal_work.committed(), 0);
    assert_eq!(failure_terminal_work.committed_canonical_output(), 0);
    assert_eq!(failure_terminal_work.published_outcome_evidence(), 0);

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
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations =
        vec![PrimitiveDeformationAuthoring::new(Presence::Absent, &[]); PRIMITIVE_COUNT];
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let node_batch = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]);
    let deformation =
        DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &primitive_deformations, &[]);
    let animations = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile.batch(),
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

    let expected_canonical_block_count = 36
        + usize::try_from(SOURCE_BACKED_PROFILE_CANONICAL_BLOCK_COUNT)
            .expect("source-backed profile canonical block count");
    assert_eq!(
        parts.scene.canonical_block_count(),
        expected_canonical_block_count
    );
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn present_empty_scene_publishes_every_domain_directory() {
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
    assert_eq!(parts.scene.canonical_block_count(), 15);
    assert_eq!(parts.scene.parent(0), None);
    assert_eq!(parts.scene.child(None, 0), None);
    assert_eq!(parts.scene.authored_transform(0), None);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_logical_byte_rejection_publishes_evidence_only_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let operation = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("canonical-byte-bounded node Scene operation");
    let before = account.snapshot();

    let rejected = match operation.try_validate_node_batch(
        single_trs_batch(),
        SceneValidationLimits::default().with_max_canonical_bytes(227),
    ) {
        Ok(_) => panic!("one byte below the canonical requirement must reject"),
        Err(rejected) => rejected,
    };
    assert!(
        matches!(
            rejected.failure(),
            NodeSceneConstructionRejection::NodeAuthoring(NodeAuthoringFailure::Semantic(
                NodeAuthoringSemanticError::LimitExceeded {
                    limit: NodeAuthoringLimit::CanonicalBytes,
                    actual: 168,
                    maximum: 167,
                }
            ))
        ),
        "unexpected canonical-byte rejection: {:#?}",
        rejected.failure()
    );
    let after_rejection = account.snapshot();
    assert_eq!(
        after_rejection, before,
        "authoring rejection must not enter physical publication"
    );

    let (prepared, rejection) = rejected
        .prepare_rich_failure()
        .expect("canonical byte rejection rich failure");
    assert!(matches!(
        rejection,
        NodeSceneConstructionRejection::NodeAuthoring(NodeAuthoringFailure::Semantic(
            NodeAuthoringSemanticError::LimitExceeded {
                limit: NodeAuthoringLimit::CanonicalBytes,
                actual: 168,
                maximum: 167,
            }
        ))
    ));
    let failure = rich_failure(prepared);
    assert_eq!(
        terminal_failure_code(failure.ledger_evidence()),
        NODE_SCENE_NODE_AUTHORING_REJECTION_CODE
    );
    assert_eq!(
        failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("authoring rejection dimension")
            .committed_canonical_output(),
        0
    );
    assert_released(failure, &account);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn cancelled_authoring_rejection_preserves_exact_error_and_rich_evidence() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let cancellation = CancelOnSignalPoll::new();
    let operation = SceneConstructionOperation::with_account_and_cancellation_for_test(
        &account,
        test_limits(),
        cancellation.clone(),
    )
    .expect("cancelled node Scene operation");
    cancellation.cancel_after_additional_polls(1);

    let rejected = match operation
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
    {
        Ok(_) => panic!("pre-cancelled authoring must reject"),
        Err(rejected) => rejected,
    };
    let (prepared, rejection) = rejected
        .prepare_rich_failure()
        .expect("cancelled authoring rich failure");
    assert!(matches!(
        rejection,
        NodeSceneConstructionRejection::SpatialAuthoring(SpatialAuthoringFailure::Control(
            OperationAuthoringControlError::Control(_)
        ))
    ));
    let failure = rich_failure(prepared);
    assert_eq!(
        terminal_failure_class(failure.ledger_evidence()),
        StorePreparationFailureClass::Control
    );
    assert_released(failure, &account);
}

#[test]
fn authoring_work_budget_rejection_preserves_exact_error_and_rich_evidence() {
    const AUTHORING_WORK_LIMIT: u64 = 10;

    let factory = test_factory();
    let account = test_account(&factory, 256);
    let operation = SceneConstructionOperation::with_account(
        &account,
        test_limits_with_work(AUTHORING_WORK_LIMIT),
    )
    .expect("work-bounded node Scene operation");

    let rejected = match operation
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
    {
        Ok(_) => panic!("exhausted authoring work budget must reject"),
        Err(rejected) => rejected,
    };
    let (prepared, rejection) = rejected
        .prepare_rich_failure()
        .expect("work-budget authoring rich failure");
    assert!(matches!(
        rejection,
        NodeSceneConstructionRejection::NodeAuthoring(NodeAuthoringFailure::Control(
            OperationAuthoringControlError::Store(_)
        ))
    ));
    let failure = rich_failure(prepared);
    assert_eq!(
        terminal_failure_class(failure.ledger_evidence()),
        StorePreparationFailureClass::Ledger
    );
    let work = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("authoring budget rejection dimension");
    assert_eq!(work.attempted(), AUTHORING_WORK_LIMIT + 1);
    assert_eq!(work.consumed(), AUTHORING_WORK_LIMIT);
    let terminal_work = failure
        .ledger_evidence()
        .dimension_by_semantic(FAILURE_TERMINAL_WORK_DIMENSION)
        .expect("authoring failure-terminal work dimension");
    let terminal_requirement =
        failure_terminal_work_requirement().expect("node Scene failure-terminal work requirement");
    assert_eq!(
        terminal_work.attempted(),
        terminal_requirement.attempted_work_limit()
    );
    assert_eq!(
        terminal_work.consumed(),
        terminal_requirement.consumed_work_limit()
    );
    assert_released(failure, &account);
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
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations =
        vec![PrimitiveDeformationAuthoring::new(Presence::Absent, &[]); PRIMITIVE_COUNT];
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let node_batch = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::new(&logical_meshes, &primitives, &[], &[]);
    let deformation =
        DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &primitive_deformations, &[]);
    let animations = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile.batch(),
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
        SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE + 1
    );
    assert_eq!(work.consumed(), SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE);
    assert_released(failure, &account);
}

#[test]
fn central_validation_work_budget_interruption_publishes_rich_failure() {
    const WORK_LIMIT: u64 = SINGLE_TRS_CENTRAL_VALIDATION_INTERRUPTION_WORK;

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
fn invalid_authored_transform_fault_is_a_build_invariant_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let validated = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("node Scene semantic fault operation")
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
        .unwrap_or_else(|_| panic!("single-node input must validate"));

    let failure = rich_failure(
        validated
            .prepare_with_faults(NodeSceneBuildFaults::altered_authored_transform())
            .expect("invalid authored transform must prepare rich failure"),
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
fn central_validation_rejects_each_corrupted_owner_attachment_component() {
    let factory = test_factory();
    let account = test_account(&factory, 256);

    for (component, replacement) in [(0, 1), (1, u64::MAX), (2, 1), (3, 1)] {
        let validated = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("profile-address corruption operation")
            .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
            .unwrap_or_else(|_| panic!("single-node input must validate"));
        corrupt_profile_address_for_test(0, component, replacement);

        let failure = rich_failure(
            validated
                .prepare_with_faults(NodeSceneBuildFaults::none())
                .expect("corrupted owner attachment must prepare rich failure"),
        );
        assert_eq!(
            terminal_failure_code(failure.ledger_evidence()),
            NODE_SCENE_BUILD_INVARIANT_CODE,
        );
        assert_released(failure, &account);
    }
}

#[test]
fn central_validation_rejects_each_corrupted_profile_structure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);

    for fault in ProfileBuildFault::ALL {
        let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
        let nodes = single_trs_batch();
        let appearance = AppearanceAuthoringBatch::empty();
        let geometry = GeometryAuthoringBatch::new(&[], &[], &[], &[]);
        let deformation = DeformationAuthoringBatch::new(&[], &[], &[], &[], &[]);
        let animations = AnimationAuthoringBatch::empty();
        let profile = profile_fixture(
            spatial,
            nodes,
            appearance,
            geometry,
            deformation,
            animations,
        );
        let metadata_key =
            NamespacedName::new("test:profile_fault").expect("profile fault metadata key");
        let metadata = [MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &metadata_key,
            ExtensionValueRef::Bool(true),
        )];
        let input = SceneAuthoringBatch::new(
            spatial,
            nodes,
            appearance,
            geometry,
            deformation,
            animations,
            profile.batch_with_metadata(MetadataAuthoringBatch::new(&metadata)),
        );
        let validated = SceneConstructionOperation::with_account_for_scene_input(
            &account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
        )
        .unwrap_or_else(|_| panic!("{fault:?} profile fault operation must be admitted"))
        .try_validate()
        .unwrap_or_else(|_| panic!("{fault:?} profile fixture must validate before retention"));

        let failure = rich_failure(
            validated
                .prepare_with_faults(NodeSceneBuildFaults::altered_profile(fault))
                .unwrap_or_else(|_| panic!("{fault:?} must prepare a rich failure")),
        );
        assert_eq!(
            terminal_failure_code(failure.ledger_evidence()),
            NODE_SCENE_BUILD_INVARIANT_CODE,
            "{fault:?} must fail at the central canonical boundary",
        );
        assert_released(failure, &account);
    }
}

#[test]
fn invalid_geometry_payload_fault_is_a_build_invariant_rich_failure() {
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
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let node_batch = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]);
    let deformation =
        DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &primitive_deformations, &[]);
    let animations = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile.batch(),
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
            .expect("invalid geometry payload must prepare rich failure"),
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
    let terminal = build_evidence_publication_limit_terminal_for_probe(&account).handoff();
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
fn cycle_authoring_rejection_preserves_rich_evidence() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let cycle_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            parent(2, 0),
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            parent(1, 0),
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
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
    let (prepared, rejection) = rejected
        .prepare_rich_failure()
        .expect("cycle rejection rich failure");
    assert!(matches!(
        rejection,
        NodeSceneConstructionRejection::NodeAuthoring(NodeAuthoringFailure::Semantic(
            NodeAuthoringSemanticError::Cycle { .. }
        ))
    ));
    assert_released(rich_failure(prepared), &account);

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
            Presence::Absent,
            Presence::Absent,
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
fn materialization_failure_never_calls_the_generation_source() {
    let factory = test_factory();
    let mut observed_materialization_failure = false;

    for max_live_allocations in 1..=128 {
        let account = test_account(&factory, max_live_allocations);
        let operation = match SceneConstructionOperation::with_account(&account, test_limits()) {
            Ok(operation) => operation,
            Err(_) => {
                drop(account);
                continue;
            }
        };
        let roots = [NodeLocalId::new(0)];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        )];
        let validated = operation
            .try_validate_node_batch(
                NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
                SceneValidationLimits::default(),
            )
            .unwrap_or_else(|_| panic!("physical output limits must not reject semantics"));
        let build = validated
            .finish_draft()
            .expect("materialization failure must retain an exact failure owner");

        if matches!(&build, NodeSceneDraftBuild::Failure(_)) {
            let prepared = build
                .prepare_with_generation_source(|| {
                    panic!("materialization failure must not call the generation source")
                })
                .expect("materialization failure must remain prepared");
            let terminal = prepared.handoff();
            assert!(!matches!(
                &terminal,
                NodeSceneConstructionTerminal::Success(_)
            ));
            drop(terminal);
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
            assert_eq!(account.snapshot().live_retained_allocations(), 0);
            observed_materialization_failure = true;
            drop(account);
            break;
        }

        drop(build);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);
    }

    assert!(observed_materialization_failure);
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
