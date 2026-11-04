use std::num::NonZeroU64;

use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalActivationSessionPolicy,
    CanonicalBodyOperation, CanonicalBodyStart, CanonicalFreezePermitIssue,
    CanonicalInvocationOutcome, CanonicalMaterializationRole, CanonicalValidationProgress,
    CanonicalValidationRole, RetainedStoreSessionOutcomeOwner, RetainedStoreWriterRole,
};
use baozi_operation::__retained_store_integration::authority::{
    RetainedStoreSharePlan, RetainedStoreStartPlan, RetainedStoreWriterPlan,
};
use baozi_operation::{
    DimensionPolicy, FinalProgress, LedgerLimits, OperationControl, OperationOwner,
    PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, ProgressPhase, ProgressUnit, RetainedStoreBatchLimits,
    RetainedStoreFailureTerminalWorkPlan, StoreDimensionSemanticId, StoreFallbackEnvelope,
    StoreKeyDomain, StoreLedgerPlan, StorePhaseSemanticId, StoreSemanticCatalogId,
};

use super::*;
use crate::__scene_integration::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain, AttributeSemantic,
    AuthoredTransform, AuthoringRange, DeformationAuthoringBatch, FormatId,
    GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    LineageAuthoringBatch, LogicalMeshLocalId, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MetadataAuthoringBatch, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, NodeParent, ObservedResourceAuthoring,
    OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentity,
    ResourceIdentityLocalId, SceneAuthoringBatch, SceneSpace, SessionOpaqueReason,
    SourceIdentityAuthoring, SourceIdentityLocalId, SourceObjectKind, SourceObjectLocator,
    SpatialAuthoringBatch,
};
use crate::__scene_integration::integration::{
    PreparedScenePublication, SceneConstruction, SceneOutcomeEvidenceTag, SceneParseOutcome,
    SceneSessionActivationBootstrap, consume_scene_freeze_permit,
    scene_failure_evidence_field_work, scene_failure_evidence_member_count,
};
use crate::SceneValidationLimits;

const UNLIMITED: RelationsQueryLimits = RelationsQueryLimits::new(u64::MAX, u64::MAX);

struct TestOwner;

impl OperationOwner for TestOwner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Validate,
    Publish,
}

impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        self as u32
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Publish => "publish",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    Work,
    Scene,
}

impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Scene => "scene",
        }
    }
}

// Store-local test helpers are private to that module. Use the existing integration rail here
// so these fixtures still undergo central validation and retain real accounting owners.
fn test_owner<'inv>(
    root: CanonicalBodyStart<'inv, TestOwner, Phase, Unit>,
    account: &PhysicalStorageAccount,
) -> RetainedStoreSessionOutcomeOwner<'inv, TestOwner, Phase, Unit> {
    let catalog = StoreSemanticCatalogId::new(*b"baozi-relations!", NonZeroU32::MIN);
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let phase = domain
        .try_admit_phase(StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
        .unwrap();
    let dimension = domain
        .try_admit_dimension(StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN))
        .unwrap();
    let terminal_dimension = domain
        .try_admit_dimension(StoreDimensionSemanticId::new(
            catalog,
            NonZeroU64::new(2).unwrap(),
        ))
        .unwrap();
    let failure_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        terminal_dimension,
        scene_failure_evidence_member_count(),
        scene_failure_evidence_field_work(),
        scene_failure_evidence_field_work(),
        u64::MAX,
        u64::MAX,
    )
    .unwrap();
    let policy = DimensionPolicy::reservable_bytes(
        failure_work.ordinary_attempted_work_limit(),
        failure_work.ordinary_consumed_work_limit(),
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (dimension, policy),
            (
                terminal_dimension,
                failure_work.requirement().dimension_policy(),
            ),
        ],
    )
    .unwrap();
    let ledger = StoreLedgerPlan::try_new(domain, limits, &[phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        RetainedStoreWriterPlan::new(
            phase,
            dimension,
            dimension,
            RetainedStoreBatchLimits::new(64, 8).unwrap(),
        ),
        RetainedStoreSharePlan::new(dimension, dimension),
        StoreFallbackEnvelope::new(1),
        dimension,
        1,
        dimension,
        1,
        failure_work,
    );
    let operation = CanonicalBodyOperation::<TestOwner, Phase, Unit>::try_start(
        root,
        OperationControl::default(),
        ledger,
        account,
        start,
    )
    .unwrap();
    let policy = CanonicalActivationSessionPolicy::new(
        RetainedStoreWriterRole::new(phase, dimension, dimension),
        RetainedStoreWriterRole::new(phase, dimension, dimension),
        CanonicalMaterializationRole::new(dimension, dimension),
        CanonicalValidationRole::new(phase, dimension, dimension),
        dimension,
        dimension,
    )
    .with_semantic_bytes_dimension(dimension);
    let (evidence_limits, failure, _) = SceneSessionActivationBootstrap::try_new()
        .unwrap()
        .into_parts();
    operation
        .into_session_outcome_owner(policy, evidence_limits, failure)
        .unwrap()
}

#[allow(
    clippy::result_large_err,
    reason = "the semantic fixture preserves its exact inline domain rejection without allocating another failure owner"
)]
fn publish_scene(
    nodes: NodeAuthoringBatch<'_>,
    node_count: usize,
    instance_nodes: &[u32],
) -> (ValidatedScene, PhysicalStorageAccount) {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 1024).unwrap())
        .unwrap();
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 1),
    )];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &[1.0, 2.0, 3.0],
        Presence::Absent,
    )];
    let mut instances = Vec::new();
    for &node in instance_nodes {
        instances.push(GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(node),
            LogicalMeshLocalId::new(0),
        ));
    }
    let geometry = if instances.is_empty() {
        GeometryAuthoringBatch::empty()
    } else {
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances)
    };
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let instance_deformations =
        vec![
            MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent);
            instances.len()
        ];
    let deformation = if instances.is_empty() {
        DeformationAuthoringBatch::empty()
    } else {
        DeformationAuthoringBatch::new(
            &[],
            &[],
            &logical_morphs,
            &primitive_deformations,
            &instance_deformations,
        )
    };
    let resource = ResourceIdentity::new("relations/fixture").unwrap();
    let format = FormatId::new("org.baozi.relations-test").unwrap();
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let resources = [ObservedResourceAuthoring::session_opaque(
        &resource,
        SessionOpaqueReason::CallerLabel,
    )];
    let sources = [SourceIdentityAuthoring::new(
        ResourceIdentityLocalId::new(0),
        &format,
        &locator,
    )];
    let provenance = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        0,
    ))];
    let source_provenance = [ProvenanceLocalId::new(0)];
    let mut owners = vec![
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &source_provenance),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &source_provenance),
    ];
    for index in 0..node_count {
        owners.push(OwnerProvenanceAuthoring::new(
            ProfileOwner::Node(NodeLocalId::new(index as u32)),
            &source_provenance,
        ));
    }
    if !instances.is_empty() {
        for owner in [
            ProfileOwner::LogicalMesh(0),
            ProfileOwner::Primitive(0),
            ProfileOwner::Attribute(0),
        ] {
            owners.push(OwnerProvenanceAuthoring::new(owner, &source_provenance));
        }
    }
    for index in 0..instances.len() {
        owners.push(OwnerProvenanceAuthoring::new(
            ProfileOwner::MeshInstance(index as u32),
            &source_provenance,
        ));
    }
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
        MetadataAuthoringBatch::empty(),
    );
    let batch = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        nodes,
        AppearanceAuthoringBatch::empty(),
        geometry,
        deformation,
        AnimationAuthoringBatch::empty(),
        profile,
    );
    let transaction = SceneConstruction::try_new(batch, SceneValidationLimits::default())
        .unwrap()
        .into_transaction();
    let mut transaction = Some(transaction);
    let mut host =
        baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost::<
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            baozi_operation::RecordBlock,
            crate::__scene_integration::integration::SceneCanonicalRootTag,
            baozi_operation::RecordBlock,
            SceneOutcomeEvidenceTag,
        >::new();
    let mut recipe = None;
    let finish = host
        .run_body(|parts, disposition| {
            let mut transaction = transaction.take().unwrap();
            let activation = test_owner(parts.startup, &account)
                .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                    let (plan, completion) = planning
                        .with_semantic_work(|semantic| transaction.preflight(semantic))
                        .unwrap();
                    match planning.seal_activation(plan, completion) {
                        CanonicalActivationSeal::Sealed(permit) => {
                            permit.activate(|lane, plan| plan.materialize(lane))
                        }
                        CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
                    }
                })
                .unwrap();
            let CanonicalActivationOutcome::Activated(activated) = activation else {
                panic!("relations fixture activation failed");
            };
            let postflight = transaction.finish();
            let evidence = postflight.candidate_evidence(activated.output());
            let central = activated
                .validate_centrally(
                    parts.proof,
                    parts.validation,
                    CanonicalValidationProgress::new(Phase::Validate, Unit::Work),
                    |lane, _| match lane.try_retain_records(
                        evidence.record_plan(),
                        SceneOutcomeEvidenceTag,
                        |writer| writer.write_row(|row| evidence.write_row(row)),
                    ) {
                        Ok(root) => lane.complete(root),
                        Err(_) => lane.abort(),
                    },
                    |planning, output| postflight.validate_candidate(planning, output),
                )
                .unwrap();
            recipe = Some(evidence);
            central.request_finish(disposition).unwrap();
        })
        .unwrap();
    let completion = finish.resume();
    let CanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
        panic!("relations fixture central validation failed");
    };
    let CanonicalFreezePermitIssue::Ready(permit) = candidate
        .try_issue_freeze_permit(
            completion.generation.interruption,
            FinalProgress::new(Phase::Publish, Unit::Scene, 1),
        )
        .unwrap()
    else {
        panic!("relations fixture freeze failed");
    };
    let publication = consume_scene_freeze_permit(
        permit,
        completion.generation.rejection,
        completion.failure_fallback,
        recipe.unwrap(),
        SceneParseOutcome::Accepted,
    )
    .unwrap();
    let PreparedScenePublication::Success(prepared) = publication else {
        panic!("relations fixture publication failed")
    };
    let (scene, evidence, ledger) = prepared.handoff().into_parts();
    drop(evidence);
    drop(ledger);
    (scene, account)
}

fn forest_scene() -> (ValidatedScene, PhysicalStorageAccount) {
    let node = |parent, name: &'static str| {
        NodeAuthoringNode::new(
            parent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        )
        .with_name(name)
    };
    let nodes = [
        node(Presence::Absent, "root-a"),
        node(
            Presence::Present(NodeParent::new(NodeLocalId::new(0), 0)),
            "empty-child",
        ),
        node(
            Presence::Present(NodeParent::new(NodeLocalId::new(0), 1)),
            "branch",
        ),
        node(Presence::Absent, "root-b"),
        node(
            Presence::Present(NodeParent::new(NodeLocalId::new(2), 0)),
            "leaf",
        ),
    ];
    let roots = [NodeLocalId::new(3), NodeLocalId::new(0)];
    publish_scene(
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        nodes.len(),
        &[2, 0, 2, 3, 0],
    )
}

fn named_node(scene: &ValidatedScene, name: &str) -> NodeId {
    scene
        .nodes()
        .find(|node| node.name() == Some(name))
        .unwrap()
        .id()
}

#[test]
fn forest_and_instance_rows_remain_readable_after_scene_teardown_without_allocating() {
    let (scene, account) = forest_scene();
    let generation = scene.generation();
    let root_a = named_node(&scene, "root-a");
    let root_b = named_node(&scene, "root-b");
    let child = named_node(&scene, "empty-child");
    let branch = named_node(&scene, "branch");
    let leaf = named_node(&scene, "leaf");
    assert_eq!(scene.root_count(), 2);
    assert_eq!(scene.mesh_count(), 1);
    let mut inventory = scene.mesh_instances();
    let instances: [_; 5] = std::array::from_fn(|_| inventory.next().unwrap().id());
    drop(inventory);
    let before = account.snapshot();
    let relations = scene.relations(UNLIMITED, None).unwrap();
    assert_eq!(relations.generation(), generation);
    assert_eq!(relations.node_count(), 5);
    assert_eq!(relations.mesh_instance_count(), 5);
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        before.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        before.live_retained_allocations()
    );
    for node in scene.nodes() {
        let mut row = relations.instances(node.id()).unwrap();
        for instance in scene
            .mesh_instances()
            .filter(|instance| instance.node().id() == node.id())
        {
            let id = row.next().unwrap();
            assert_eq!(id, instance.id());
            assert_eq!(scene.mesh_instance(id).unwrap().node().id(), node.id());
        }
        assert_eq!(row.next(), None);
    }
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
    drop(account);

    let allocations = allocation_counter::measure(|| {
        assert_eq!(relations.parent(root_a).unwrap(), None);
        assert_eq!(relations.parent(root_b).unwrap(), None);
        assert_eq!(relations.parent(child).unwrap(), Some(root_a));
        assert_eq!(relations.parent(branch).unwrap(), Some(root_a));
        assert_eq!(relations.parent(leaf).unwrap(), Some(branch));
        let mut row = relations.instances(root_a).unwrap();
        assert_eq!(row.size_hint(), (2, Some(2)));
        assert_eq!(row.next(), Some(instances[1]));
        assert_eq!(row.len(), 1);
        assert_eq!(row.next(), Some(instances[4]));
        assert_eq!(row.len(), 0);
        assert_eq!(row.next(), None);
        assert_eq!(row.next(), None);
        assert!(
            relations
                .instances(branch)
                .unwrap()
                .eq([instances[0], instances[2]])
        );
        assert!(relations.instances(root_b).unwrap().eq([instances[3]]));
        assert_eq!(relations.instances(child).unwrap().len(), 0);
        assert_eq!(relations.instances(leaf).unwrap().len(), 0);
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn exact_work_and_capacity_envelope_limits_include_all_construction_scratch() {
    let (scene, account) = forest_scene();
    let work = 44;
    let envelope_slots = 8;
    let bytes = (envelope_slots * size_of::<Option<NonZeroU32>>()
        + envelope_slots * size_of::<usize>()
        + envelope_slots * size_of::<u32>()
        + envelope_slots * size_of::<usize>()) as u64;
    let limits = RelationsQueryLimits::new(work, bytes);
    assert_eq!(limits.max_work(), work);
    assert_eq!(limits.max_bytes(), bytes);
    let mut relations = None;
    let allocations = allocation_counter::measure(|| {
        relations = Some(scene.relations(limits, None).unwrap());
    });
    let relations = relations.unwrap();
    assert_eq!(allocations.count_total, 4);
    assert_eq!(allocations.count_current, 3);
    assert_eq!(
        allocations.bytes_current,
        relations.allocated_bytes() as i64
    );
    assert_eq!(
        relations.allocated_bytes(),
        relations.parents.capacity() * size_of::<Option<NonZeroU32>>()
            + relations.instance_offsets.capacity() * size_of::<usize>()
            + relations.instance_ordinals.capacity() * size_of::<u32>()
    );
    assert!(allocations.bytes_total <= bytes);

    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            scene
                .relations(RelationsQueryLimits::new(work - 1, bytes), None)
                .unwrap_err(),
            RelationsQueryError::LimitExceeded {
                limit: RelationsQueryLimit::Work,
                actual: work,
                maximum: work - 1,
            }
        );
        assert_eq!(
            scene
                .relations(RelationsQueryLimits::new(work, bytes - 1), None)
                .unwrap_err(),
            RelationsQueryError::LimitExceeded {
                limit: RelationsQueryLimit::Bytes,
                actual: bytes,
                maximum: bytes - 1,
            }
        );
    });
    assert_eq!(allocations.count_total, 0);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn empty_scene_has_only_a_bounded_csr_sentinel_and_no_synthetic_node() {
    let (scene, account) =
        publish_scene(NodeAuthoringBatch::new(Presence::Present(&[]), &[]), 0, &[]);
    let bytes = size_of::<usize>() as u64;
    let relations = scene
        .relations(RelationsQueryLimits::new(1, bytes), None)
        .unwrap();
    let invalid = NodeId::new(scene.generation(), 0);
    assert_eq!(relations.node_count(), 0);
    assert_eq!(relations.mesh_instance_count(), 0);
    assert_eq!(relations.instance_offsets, [0]);
    assert_eq!(relations.allocated_bytes(), size_of::<usize>());
    assert_eq!(
        relations.parent(invalid),
        Err(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Node,
            length: 0,
        })
    );
    assert!(matches!(
        relations.instances(invalid),
        Err(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Node,
            length: 0,
        })
    ));
    assert!(matches!(
        scene.relations(RelationsQueryLimits::new(0, bytes), None),
        Err(RelationsQueryError::LimitExceeded {
            limit: RelationsQueryLimit::Work,
            actual: 1,
            maximum: 0,
        })
    ));
    assert!(matches!(
        scene.relations(RelationsQueryLimits::new(1, bytes - 1), None),
        Err(RelationsQueryError::LimitExceeded {
            limit: RelationsQueryLimit::Bytes,
            ..
        })
    ));
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn nodes_without_any_geometry_have_empty_exact_size_rows() {
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let roots = [NodeLocalId::new(0)];
    let (scene, account) = publish_scene(
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        nodes.len(),
        &[],
    );
    let node = scene.nodes().next().unwrap().id();
    let relations = scene.relations(UNLIMITED, None).unwrap();
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(relations.parent(node).unwrap(), None);
    let mut row = relations.instances(node).unwrap();
    assert_eq!(row.size_hint(), (0, Some(0)));
    assert_eq!(row.len(), 0);
    assert_eq!(row.next(), None);
}

#[test]
fn both_lookups_reject_generation_before_checking_range() {
    let (scene, account) = forest_scene();
    let (other, other_account) = forest_scene();
    let relations = scene.relations(UNLIMITED, None).unwrap();
    let other_relations = other.relations(UNLIMITED, None).unwrap();
    let root = scene.roots().next().unwrap().id();
    let other_root = other.roots().next().unwrap().id();
    let expected = scene.generation();
    let actual = other.generation();
    for node in [other_root, NodeId::new(actual, u32::MAX)] {
        let mismatch = SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Node,
            expected,
            actual,
        };
        assert_eq!(relations.parent(node), Err(mismatch));
        assert!(matches!(relations.instances(node), Err(error) if error == mismatch));
    }
    for index in [relations.node_count() as u32, u32::MAX] {
        let node = NodeId::new(expected, index);
        let out_of_range = SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Node,
            length: relations.node_count(),
        };
        assert_eq!(relations.parent(node), Err(out_of_range));
        assert!(matches!(relations.instances(node), Err(error) if error == out_of_range));
    }
    drop(scene);
    drop(other);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(other_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(relations.parent(root).unwrap(), None);
    assert_eq!(other_relations.parent(other_root).unwrap(), None);
    assert!(matches!(
        other_relations.parent(root),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        other_relations.instances(root),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
}

#[test]
fn cancellation_precedes_admission_and_allocation_and_interrupts_work_units() {
    let (scene, account) = forest_scene();
    let token = CancellationToken::new();
    token.cancel();
    let before = account.snapshot().live_retained_bytes();
    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            scene
                .relations(RelationsQueryLimits::new(0, 0), Some(&token))
                .unwrap_err(),
            RelationsQueryError::Cancelled
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(account.snapshot().live_retained_bytes(), before);

    let token = CancellationToken::new();
    let mut query = Query::new(UNLIMITED, Some(&token)).unwrap();
    let allocations = allocation_counter::measure(|| {
        let mut slots =
            allocate::<usize>(CapacityPlan::for_type::<usize>(0, 0, 4).unwrap(), &query).unwrap();
        query.step().unwrap();
        slots.push(0);
        token.cancel();
        assert_eq!(query.step(), Err(RelationsQueryError::Cancelled));
        assert_eq!(query.poll(), Err(RelationsQueryError::Cancelled));
        assert_eq!(query.bytes(0), Err(RelationsQueryError::Cancelled));
        assert_eq!(
            allocate::<usize>(CapacityPlan::for_type::<usize>(0, 0, 1).unwrap(), &query),
            Err(RelationsQueryError::Cancelled)
        );
    });
    assert_eq!(allocations.count_total, 1);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn overflow_is_reported_before_any_allocation() {
    let allocations = allocation_counter::measure(|| {
        assert!(matches!(
            RelationsPlan::new(usize::MAX, 0),
            Err(RelationsQueryError::SizeOverflow)
        ));
        assert_eq!(
            storage_bytes([(usize::MAX, 2)]),
            Err(RelationsQueryError::SizeOverflow)
        );
        let mut query = Query::new(UNLIMITED, None).unwrap();
        query.work = u64::MAX;
        assert_eq!(query.step(), Err(RelationsQueryError::SizeOverflow));
        if usize::BITS == 64 {
            assert_eq!(
                construction_work(usize::MAX, 0, 0),
                Err(RelationsQueryError::SizeOverflow)
            );
        }
    });
    assert_eq!(allocations.count_total, 0);
}
