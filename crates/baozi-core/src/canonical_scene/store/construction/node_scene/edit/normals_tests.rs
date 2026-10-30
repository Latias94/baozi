//! Regression coverage for typed edits against an actual retained, centrally validated source.

use std::convert::Infallible;
use std::sync::atomic::AtomicU64;

use baozi_operation::__retained_store_integration::activation::{
    ActivatedCanonicalStoreSession, CanonicalActivationOutcome, CanonicalActivationSeal,
};
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::__retained_store_integration::producer::{
    CanonicalActivationDescription, CanonicalActivationPlanSealer, CanonicalActivationRequest,
    CanonicalSealedActivationPlan, CanonicalSuccessRequirements, CanonicalTableShape,
};
use baozi_operation::{OperationControl, PhysicalStorageAccount, RecordBlock, StoreRuntimeError};

use crate::attribute::{AttributeDomain, AttributeSemantic, CustomAttributeSemantic};
use crate::canonical_scene::store::construction::node_scene::edit::graph::try_rewrite_node_scene;
use crate::canonical_scene::store::construction::node_scene::edit::plan::BoundPendingNodeSceneEdit;
use crate::canonical_scene::store::construction::node_scene::materialization::{
    NodeSceneSessionActivationBootstrap, node_scene_activation_roles,
};
use crate::canonical_scene::store::construction::node_scene::tests::{
    single_trs_batch, successful_parts, test_account, test_factory, test_limits,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneSuccessParts, NodeSceneTestTransaction, SceneStorage,
};
use crate::canonical_scene::store::construction::{
    ConstructionBrand, SceneConstructionOwner, SceneConstructionPhase, SceneConstructionUnit,
    SceneFailureEvidenceWork, SceneStoreOperation, try_plan_scene_operation,
};
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AuthoringRange, CanonicalProfileOwner,
    DeformationAuthoringBatch, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    LineageAuthoringBatch, LogicalMorphAuthoring, MetadataAuthoringBatch, MetadataAuthoringEntry,
    NodeLocalId, ObservedResourceAuthoring, OwnerProvenanceAuthoring, Presence,
    PrimitiveDeformationAuthoring, ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ProvenanceOperationAuthoring, ResourceIdentityLocalId,
    SceneAuthoringBatch, SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId,
    SpatialAuthoringBatch,
};
use crate::canonical_scene::store::profile_provenance::ProfileProvenanceRef;
use crate::canonical_scene::store::schema::{
    NODE_SCENE_EVIDENCE_FIELD_COUNT, SceneDirectorySchema,
};
use crate::canonical_scene::store::typed_storage::SceneTables;
use crate::canonical_scene::store::typed_storage::build::TypedStorageBuildControl;
use crate::canonical_scene::store::typed_storage::edit::normals::{
    GeometryNormalsPlan, NormalWritePolicy, TangentPolicy,
};
use crate::canonical_scene::store::typed_storage::profile::validation::ProfileMeteredScratch;
use crate::canonical_scene::store::typed_storage::profile::{
    ProfileStorage, StoredMetadataEntry, StoredMetadataValue, StoredOperationKind,
    StoredProvenanceKind,
};
use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
use crate::extension::{ExtensionArrayRef, ExtensionMediaType, ExtensionValueRef};
use crate::identity::SceneGenerationId;
use crate::namespaced::NamespacedName;
use crate::provenance::{
    OperationKind, OptionsDigest, ResourceIdentity, SourceObjectKind, SourceObjectLocator,
};
use crate::scene_space::SceneSpace;
use crate::{FormatId, SceneValidationLimits};

use super::NodeSceneEditProfilePlan;

const GENERATED_NORMALS: [[f32; 3]; 6] = [[0.0, 0.0, 1.0]; 6];
const SECOND_PAYLOAD: [u8; 3] = [0x41, 0x42, 0x43];
const METADATA_SCALARS: [f64; 2] = [-0.0, f64::from_bits(0x3ff0_0000_0000_0001)];

#[derive(Default)]
struct Control(u64);

impl TypedStorageBuildControl for Control {
    type Error = Infallible;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        let result = action();
        self.0 += 1;
        Ok(result)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        TypedStorageBuildControl::run_unit(self, action)
    }
}

impl TypedStorageValidationControl for Control {
    type Error = Infallible;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        TypedStorageBuildControl::run_unit(self, action).map(Some)
    }
}

fn source_scene(
    account: &PhysicalStorageAccount,
    generation: &AtomicU64,
    normal: bool,
    tangent: bool,
) -> NodeSceneSuccessParts {
    let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let old_normals = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let tangents = [
        1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0,
    ];
    let texcoords = [0.0, -0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let groups = [7, 7];
    let custom = [1.0_f64, -0.0, f64::from_bits(0x3ff0_0000_0000_0001), 4.0];
    let second_positions = [0.0, 0.0, 2.0, 1.0, 0.0, 2.0, 0.0, 1.0, 2.0];
    let second_custom = [9.0_f64, -0.0, 11.0];
    let indices = [0, 1, 2, 0, 2, 3];
    let semantic = CustomAttributeSemantic::new("test:precision").expect("custom semantic");
    let mut attributes = vec![GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let normal_index = normal.then(|| {
        let index = attributes.len() as u32;
        attributes.push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            &old_normals,
            Presence::Absent,
        ));
        index
    });
    let tangent_index = tangent.then(|| {
        let index = attributes.len() as u32;
        attributes.push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            4,
            &tangents,
            Presence::Absent,
        ));
        index
    });
    let texcoord_index = attributes.len() as u32;
    attributes.push(GeometryAttributeAuthoring::f32(
        AttributeSemantic::Texcoord(0),
        AttributeDomain::Corner,
        2,
        &texcoords,
        Presence::Absent,
    ));
    let group_index = attributes.len() as u32;
    attributes.push(GeometryAttributeAuthoring::u32(
        AttributeSemantic::SmoothingGroup,
        AttributeDomain::Face,
        1,
        &groups,
    ));
    attributes.push(GeometryAttributeAuthoring::f64(
        AttributeSemantic::Custom(semantic.clone()),
        AttributeDomain::Vertex,
        1,
        &custom,
    ));
    let second_start = attributes.len() as u32;
    attributes.push(GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &second_positions,
        Presence::Absent,
    ));
    attributes.push(GeometryAttributeAuthoring::f64(
        AttributeSemantic::Custom(semantic),
        AttributeDomain::Vertex,
        1,
        &second_custom,
    ));
    let primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&indices),
            Presence::Absent,
            AuthoringRange::new(0, second_start),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(second_start, 2),
        ),
    ];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 2))];
    let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[]); 2];
    let deformation =
        DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &primitive_deformations, &[]);

    let resource = ResourceIdentity::new("fixtures/normals/two-primitives").expect("resource");
    let format = FormatId::new("org.baozi.fixture").expect("format");
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
    let kind = OperationKind::Generated;
    let first_parent = [ProvenanceLocalId::new(0)];
    let second_parent = [ProvenanceLocalId::new(2)];
    let mut provenance = vec![
        ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(0)),
        ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(0)),
        ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
            &kind,
            OptionsDigest::sha256([1; 32]),
            &first_parent,
            &[],
        )),
        ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
            &kind,
            OptionsDigest::sha256([2; 32]),
            &second_parent,
            &[],
        )),
    ];
    let mut owners = vec![
        ProfileOwner::Scene,
        ProfileOwner::SyntheticRoot,
        ProfileOwner::Node(NodeLocalId::new(0)),
        ProfileOwner::LogicalMesh(0),
        ProfileOwner::Primitive(0),
        ProfileOwner::Primitive(1),
    ];
    owners.extend((0..attributes.len() as u32).map(ProfileOwner::Attribute));
    // Every owner has its own marker and a shared ancestor. Position and old Normal overlap
    // at depth two; only SmoothingGroup carries the depth-three ancestor.
    let attachments: Vec<Vec<_>> = owners
        .iter()
        .enumerate()
        .map(|(ordinal, &owner)| {
            let unique = provenance.len() as u32;
            provenance.push(ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
                0,
            )));
            assert_eq!(unique, 4 + ordinal as u32);
            let mut parents = vec![ProvenanceLocalId::new(0)];
            if owner == ProfileOwner::Attribute(0)
                || normal_index.is_some_and(|index| owner == ProfileOwner::Attribute(index))
            {
                parents.push(ProvenanceLocalId::new(2));
            }
            if owner == ProfileOwner::Attribute(group_index) {
                parents.push(ProvenanceLocalId::new(3));
            }
            parents.push(ProvenanceLocalId::new(unique));
            parents
        })
        .collect();
    let owners: Vec<_> = owners
        .into_iter()
        .zip(&attachments)
        .map(|(owner, parents)| OwnerProvenanceAuthoring::new(owner, parents))
        .collect();
    let keys = ["test:normal", "test:tangent", "test:uv", "test:second"]
        .map(|key| NamespacedName::new(key).expect("metadata key"));
    let media_type = ExtensionMediaType::new("application/octet-stream").expect("media type");
    let removed_payload = [0xde, 0xad, 0xbe, 0xef];
    let mut metadata = Vec::new();
    if let Some(index) = normal_index {
        metadata.push(MetadataAuthoringEntry::new(
            ProfileOwner::Attribute(index),
            &keys[0],
            ExtensionValueRef::String("normal α"),
        ));
    }
    if let Some(index) = tangent_index {
        metadata.push(MetadataAuthoringEntry::new(
            ProfileOwner::Attribute(index),
            &keys[1],
            ExtensionValueRef::Bytes {
                media_type: &media_type,
                payload: &removed_payload,
            },
        ));
    }
    metadata.push(MetadataAuthoringEntry::new(
        ProfileOwner::Attribute(texcoord_index),
        &keys[2],
        ExtensionValueRef::DenseArray(ExtensionArrayRef::F64(&METADATA_SCALARS)),
    ));
    metadata.push(MetadataAuthoringEntry::new(
        ProfileOwner::Attribute(second_start + 1),
        &keys[3],
        ExtensionValueRef::Bytes {
            media_type: &media_type,
            payload: &SECOND_PAYLOAD,
        },
    ));
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
        MetadataAuthoringBatch::new(&metadata),
    );
    let input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        single_trs_batch(),
        AppearanceAuthoringBatch::empty(),
        geometry,
        deformation,
        AnimationAuthoringBatch::empty(),
        profile,
    );
    let draft = NodeSceneTestTransaction::with_account(
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("normals source admission")
    .finish_draft()
    .expect("normals source construction");
    successful_parts(
        draft
            .publish_with_generation_source_for_test(|| SceneGenerationId::reserve_from(generation))
            .expect("normals source publication"),
    )
}

type RetainedEdit = ActivatedCanonicalStoreSession<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    SceneTables,
    RecordBlock,
    SceneDirectorySchema,
    CanonicalInheritanceSource,
>;

struct EditActivation<'scene> {
    request: CanonicalActivationRequest,
    requirements: CanonicalSuccessRequirements,
    pending: BoundPendingNodeSceneEdit<'scene>,
}

impl<'scene> CanonicalActivationDescription for EditActivation<'scene> {
    type Execution = BoundPendingNodeSceneEdit<'scene>;

    fn seal_activation<'activation>(
        self,
        sealer: CanonicalActivationPlanSealer<'activation>,
    ) -> CanonicalSealedActivationPlan<'activation, Self::Execution> {
        sealer.seal(self.request, self.requirements, self.pending)
    }
}

/// Uses the production graph/profile retention adapters and the existing same-account lane.
/// Publication and postprocess orchestration are covered by their own transaction tests.
fn retain_edit(pending: BoundPendingNodeSceneEdit<'_>) -> RetainedEdit {
    let counts = pending.counts;
    let start = try_plan_scene_operation(
        test_limits(),
        OperationControl::default(),
        counts.canonical_attribution_slots.as_usize().max(2),
        SceneFailureEvidenceWork::exact_single_record(NODE_SCENE_EVIDENCE_FIELD_COUNT),
    )
    .expect("normals derived start plan");
    let raw = SceneStoreOperation::try_start_derived(
        start.control,
        start.ledger_plan,
        &pending.input.canonical,
        start.start_plan,
    )
    .expect("normals same-account start");
    let (evidence_limits, failure, _) = NodeSceneSessionActivationBootstrap::try_new()
        .expect("normals failure envelope")
        .into_parts();
    let owner = raw
        .into_session_outcome_owner(
            node_scene_activation_roles(start.keys).session_policy(),
            evidence_limits,
            failure,
        )
        .expect("normals session owner");
    let (brand, request) =
        ConstructionBrand::reserve_with_activation_request().expect("normals activation identity");
    let total = counts.canonical_blocks.as_usize();
    let requirements = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(total, total).expect("normals canonical shape"),
        4,
        total,
        counts.central_validation_plan.work(),
    )
    .expect("normals success envelope")
    .with_request_identity(brand.activation_request_identity())
    .with_semantic_bytes(pending.input.read_geometry().names.len() as u64);
    let mut description = Some(EditActivation {
        request,
        requirements,
        pending,
    });
    let outcome: CanonicalActivationOutcome<
        _,
        _,
        _,
        SceneTables,
        StoreRuntimeError,
        RecordBlock,
        SceneDirectorySchema,
        CanonicalInheritanceSource,
    > = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning
                .begin_semantic_work()
                .expect("normals semantic lane");
            semantic.run_unit(|| ()).expect("normals semantic action");
            let completion = semantic
                .finish_exact()
                .expect("normals semantic completion");
            let permit = match planning.seal_activation(
                description.take().expect("one normals activation"),
                completion,
            ) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
            };
            permit.activate(
                |mut lane, pending| match try_rewrite_node_scene(&mut lane, pending) {
                    Ok(graph) => lane.complete(graph.directory.into_store(), graph.tables),
                    Err(cause) => lane.reject(cause, 0x4e4f_5201),
                },
            )
        })
        .expect("normals scoped retention");
    match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        _ => panic!("valid normals profile must reach a retained candidate"),
    }
}

fn attachments(profile: &ProfileStorage, owner: CanonicalProfileOwner) -> &[ProfileProvenanceRef] {
    let state = profile.state.as_ref();
    state
        .owners
        .iter()
        .find(|row| row.owner == owner)
        .expect("owner row")
        .attachments
        .get(&state.attachments)
        .expect("owner attachments")
}

fn expected_parents(
    source: &SceneStorage,
    owners: &[CanonicalProfileOwner],
) -> Vec<ProfileProvenanceRef> {
    let mut parents: Vec<_> = owners
        .iter()
        .flat_map(|&owner| attachments(&source.tables.profile, owner).iter().copied())
        .collect();
    parents.sort_unstable_by_key(|reference| (reference.segment(), reference.local()));
    parents.dedup();
    parents
}

fn assert_valid_profile(profile: &ProfileStorage, plan: NodeSceneEditProfilePlan) {
    let mut depths = vec![0; plan.output_counts.provenance_count as usize];
    let mut control = Control::default();
    let proof = profile
        .validate(
            &mut control,
            plan.output_counts,
            plan.output_layout,
            plan.maximum_depth,
            &mut ProfileMeteredScratch::new(&mut depths),
        )
        .expect("infallible test control")
        .expect("retained edited profile passes central validator");
    assert_eq!(proof.maximum_provenance_depth, plan.maximum_depth);
    assert_eq!(
        control.0,
        plan.output_counts
            .validation_work()
            .expect("validation work")
    );
}

fn assert_generated_lineage(
    source: &SceneStorage,
    output: &ProfileStorage,
    normal_parents: &[ProfileProvenanceRef],
    normal_depth: u32,
    options: OptionsDigest,
) {
    let state = output.state.as_ref();
    let segment = state.segments.last().expect("generated segment");
    let lineage = segment.lineage.as_ref();
    assert_eq!(
        state.segments.len(),
        source.tables.profile.state.as_ref().segments.len() + 1
    );
    assert_eq!(lineage.provenance.len(), 2);
    assert_eq!(lineage.operations.len(), 2);
    assert_eq!(
        lineage.provenance[0].parents.get(&lineage.parents),
        Some(attachments(
            &source.tables.profile,
            CanonicalProfileOwner::Primitive(0)
        ),)
    );
    assert_eq!(lineage.provenance[0].depth, 2);
    assert_eq!(
        lineage.provenance[1].parents.get(&lineage.parents),
        Some(normal_parents)
    );
    assert_eq!(lineage.provenance[1].depth, normal_depth);
    for (local, (node, operation)) in lineage
        .provenance
        .iter()
        .zip(&lineage.operations)
        .enumerate()
    {
        assert!(
            matches!(node.kind, StoredProvenanceKind::Operation(index) if index == local as u32)
        );
        assert!(matches!(operation.kind, StoredOperationKind::Generated));
        assert_eq!(operation.options, options);
    }
    let reference = attachments(output, CanonicalProfileOwner::Attribute(1));
    assert_eq!(reference.len(), 1);
    assert_eq!(reference[0].local(), 1);
    let (_, node) = output
        .resolve(reference[0])
        .expect("qualified generated reference");
    assert_eq!(node.depth, normal_depth);
    let owner = state
        .owners
        .iter()
        .find(|row| row.owner == CanonicalProfileOwner::Attribute(1))
        .expect("normal owner");
    assert_eq!(owner.maximum_depth, normal_depth);
    for (before, after) in source
        .tables
        .profile
        .state
        .as_ref()
        .segments
        .iter()
        .zip(&state.segments)
    {
        assert!(std::ptr::eq(
            before.lineage.as_ref(),
            after.lineage.as_ref()
        ));
    }
}

fn metadata<'a>(profile: &'a ProfileStorage, key: &str) -> &'a StoredMetadataEntry {
    let pools = profile.metadata.as_ref();
    profile
        .state
        .as_ref()
        .metadata_entries
        .iter()
        .find(|entry| entry.key.get_str(&pools.text) == Some(key))
        .expect("retained metadata entry")
}

fn assert_metadata(profile: &ProfileStorage, normal: bool, uv_owner: u32, second_owner: u32) {
    let pools = profile.metadata.as_ref();
    assert!(
        !profile
            .state
            .as_ref()
            .metadata_entries
            .iter()
            .any(|entry| { entry.key.get_str(&pools.text) == Some("test:tangent") })
    );
    let uv = metadata(profile, "test:uv");
    assert_eq!(uv.owner, CanonicalProfileOwner::Attribute(uv_owner));
    let StoredMetadataValue::F64Array(range) = uv.value else {
        panic!("F64 metadata type");
    };
    let values = range.get(&pools.f64s).expect("F64 metadata range");
    assert_eq!(
        values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        METADATA_SCALARS.map(f64::to_bits)
    );
    let second = metadata(profile, "test:second");
    assert_eq!(second.owner, CanonicalProfileOwner::Attribute(second_owner));
    let StoredMetadataValue::Bytes {
        media_type,
        payload,
    } = second.value
    else {
        panic!("second primitive bytes metadata");
    };
    assert_eq!(
        media_type.get_str(&pools.text),
        Some("application/octet-stream")
    );
    assert_eq!(payload.get(&pools.bytes), Some(SECOND_PAYLOAD.as_slice()));
    if normal {
        let entry = metadata(profile, "test:normal");
        assert_eq!(entry.owner, CanonicalProfileOwner::Attribute(1));
        let StoredMetadataValue::String(value) = entry.value else {
            panic!("normal metadata type");
        };
        assert_eq!(value.get_str(&pools.text), Some("normal α"));
    }
}

#[test]
fn smooth_replacement_retains_distinct_parent_sets_and_compacts_removed_tangent_metadata() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation = AtomicU64::new(0);
    let source = source_scene(&account, &generation, true, true);
    let geometry = GeometryNormalsPlan::try_new(
        source.scene.read_geometry(),
        0,
        &GENERATED_NORMALS,
        NormalWritePolicy::Replace,
        TangentPolicy::Remove,
        &mut Control::default(),
    )
    .expect("normal replacement plan");
    let mapping = geometry.attribute_mapping();
    assert_eq!(mapping.source_normal(), Some(1));
    assert_eq!(mapping.source_tangent(), Some(2));
    assert_eq!(mapping.output_attribute(2), None);
    assert_eq!(mapping.output_attribute(3), Some(2));
    assert_eq!(mapping.output_attribute(7), Some(6));
    let parents = expected_parents(
        &source.scene,
        &[
            CanonicalProfileOwner::Primitive(0),
            CanonicalProfileOwner::Attribute(0),
            CanonicalProfileOwner::Attribute(1),
            CanonicalProfileOwner::Attribute(4),
        ],
    );
    assert_eq!(
        parents.len(),
        7,
        "shared parents must be deduplicated across four owners"
    );
    let options = OptionsDigest::sha256([41; 32]);
    let pending =
        BoundPendingNodeSceneEdit::try_normals(&source.scene, geometry, true, options, &mut || {
            Ok(())
        })
        .expect("smooth profile preflight");
    let plan = pending.counts.profile;
    assert_eq!(plan.target_count.as_u32(), 2);
    assert_eq!(plan.target_attachment_count.as_u32(), 9);
    assert_eq!(plan.maximum_depth, 4);
    assert_eq!(
        plan.output_counts.owner_count,
        source.scene.proof.profile_counts().owner_count - 1
    );
    assert_eq!(plan.output_counts.metadata_entry_count, 3);
    assert_eq!(
        plan.output_counts.metadata_payload_bytes,
        SECOND_PAYLOAD.len() as u32
    );
    assert!(plan.metadata.rewrites_payloads());
    let output = retain_edit(pending);
    let profile = &output.output().profile;
    assert_generated_lineage(&source.scene, profile, &parents, 4, options);
    assert_valid_profile(profile, plan);
    assert_metadata(profile, true, 2, 6);
    assert_eq!(profile.metadata.as_ref().bytes, SECOND_PAYLOAD);
    assert!(!std::ptr::eq(
        source.scene.tables.profile.metadata.as_ref(),
        profile.metadata.as_ref()
    ));
    assert_eq!(
        attachments(profile, CanonicalProfileOwner::Primitive(1)),
        attachments(
            &source.scene.tables.profile,
            CanonicalProfileOwner::Primitive(1)
        )
    );
    assert_eq!(
        attachments(profile, CanonicalProfileOwner::Attribute(6)),
        attachments(
            &source.scene.tables.profile,
            CanonicalProfileOwner::Attribute(7)
        )
    );
    assert!(
        geometry
            .validate_correspondence(output.output().geometry.as_ref(), &mut Control::default(),)
            .expect("infallible correspondence control")
    );

    drop(source);
    assert_valid_profile(&output.output().profile, plan);
    assert_metadata(&output.output().profile, true, 2, 6);
    drop(output);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn adding_smooth_normals_reindexes_following_owners_without_copying_metadata_payloads() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation = AtomicU64::new(0);
    let source = source_scene(&account, &generation, false, false);
    let geometry = GeometryNormalsPlan::try_new(
        source.scene.read_geometry(),
        0,
        &GENERATED_NORMALS,
        NormalWritePolicy::Add,
        TangentPolicy::Reject,
        &mut Control::default(),
    )
    .expect("normal addition plan");
    let mapping = geometry.attribute_mapping();
    assert_eq!(mapping.normal_attribute(), 1);
    assert_eq!(mapping.source_attribute(1), None);
    assert_eq!(mapping.output_attribute(1), Some(2));
    assert_eq!(mapping.output_attribute(5), Some(6));
    let parents = expected_parents(
        &source.scene,
        &[
            CanonicalProfileOwner::Primitive(0),
            CanonicalProfileOwner::Attribute(0),
            CanonicalProfileOwner::Attribute(2),
        ],
    );
    assert_eq!(parents.len(), 6);
    let options = OptionsDigest::sha256([42; 32]);
    let pending =
        BoundPendingNodeSceneEdit::try_normals(&source.scene, geometry, true, options, &mut || {
            Ok(())
        })
        .expect("normal addition profile preflight");
    let plan = pending.counts.profile;
    assert_eq!(plan.target_count.as_u32(), 2);
    assert_eq!(plan.target_attachment_count.as_u32(), 8);
    assert_eq!(
        plan.output_counts.owner_count,
        source.scene.proof.profile_counts().owner_count + 1
    );
    assert_eq!(plan.output_counts.metadata_entry_count, 2);
    assert!(!plan.metadata.rewrites_payloads());
    let output = retain_edit(pending);
    let profile = &output.output().profile;
    assert_generated_lineage(&source.scene, profile, &parents, 4, options);
    assert_valid_profile(profile, plan);
    assert_metadata(profile, false, 2, 6);
    assert!(std::ptr::eq(
        source.scene.tables.profile.metadata.as_ref(),
        profile.metadata.as_ref()
    ));
    assert_eq!(
        attachments(profile, CanonicalProfileOwner::Attribute(2)),
        attachments(
            &source.scene.tables.profile,
            CanonicalProfileOwner::Attribute(1)
        )
    );
    assert_eq!(
        attachments(profile, CanonicalProfileOwner::Attribute(6)),
        attachments(
            &source.scene.tables.profile,
            CanonicalProfileOwner::Attribute(5)
        )
    );
    assert_eq!(
        output.output().geometry.as_ref().primitives[1]
            .attributes
            .start(),
        5
    );
    assert!(
        geometry
            .validate_correspondence(output.output().geometry.as_ref(), &mut Control::default(),)
            .expect("infallible correspondence control")
    );

    drop(source);
    assert_valid_profile(&output.output().profile, plan);
    assert_metadata(&output.output().profile, false, 2, 6);
    drop(output);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn flat_replacement_excludes_smoothing_group_and_unrelated_attribute_provenance() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation = AtomicU64::new(0);
    let source = source_scene(&account, &generation, true, false);
    let geometry = GeometryNormalsPlan::try_new(
        source.scene.read_geometry(),
        0,
        &GENERATED_NORMALS,
        NormalWritePolicy::Replace,
        TangentPolicy::Reject,
        &mut Control::default(),
    )
    .expect("flat replacement plan");
    let parents = expected_parents(
        &source.scene,
        &[
            CanonicalProfileOwner::Primitive(0),
            CanonicalProfileOwner::Attribute(0),
            CanonicalProfileOwner::Attribute(1),
        ],
    );
    assert_eq!(parents.len(), 5);
    let smoothing = attachments(
        &source.scene.tables.profile,
        CanonicalProfileOwner::Attribute(3),
    );
    assert!(smoothing.iter().any(|parent| !parents.contains(parent)));
    let options = OptionsDigest::sha256([43; 32]);
    let pending = BoundPendingNodeSceneEdit::try_normals(
        &source.scene,
        geometry,
        false,
        options,
        &mut || Ok(()),
    )
    .expect("flat replacement profile preflight");
    let plan = pending.counts.profile;
    assert_eq!(plan.target_attachment_count.as_u32(), 7);
    assert_eq!(
        plan.maximum_depth, 3,
        "an unused smoothing ancestor must not deepen the new normal"
    );
    assert_eq!(
        plan.output_counts.owner_count,
        source.scene.proof.profile_counts().owner_count
    );
    let output = retain_edit(pending);
    let profile = &output.output().profile;
    assert_generated_lineage(&source.scene, profile, &parents, 3, options);
    assert_valid_profile(profile, plan);
    assert_metadata(profile, true, 2, 6);
    assert_eq!(
        attachments(profile, CanonicalProfileOwner::Attribute(3)),
        smoothing
    );
    assert_eq!(
        attachments(profile, CanonicalProfileOwner::Attribute(6)),
        attachments(
            &source.scene.tables.profile,
            CanonicalProfileOwner::Attribute(6)
        )
    );
    assert!(std::ptr::eq(
        source.scene.tables.profile.metadata.as_ref(),
        profile.metadata.as_ref()
    ));
    assert!(
        geometry
            .validate_correspondence(output.output().geometry.as_ref(), &mut Control::default(),)
            .expect("infallible correspondence control")
    );

    drop(source);
    assert_valid_profile(&output.output().profile, plan);
    assert_metadata(&output.output().profile, true, 2, 6);
    drop(output);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn winding_conversion_keeps_owner_indices_and_unions_each_owners_scene_parents() {
    use crate::canonical_scene::store::typed_storage::edit::space_geometry::GeometrySpacePlan;
    use crate::canonical_scene::store::typed_storage::edit::space_nodes::NodeSpacePlan;
    use crate::scene_space::{Axis, FrontFaceWinding, Handedness};
    use crate::space_conversion::{SpaceConversion, TargetSpace};
    let factory = test_factory();
    let account = test_account(&factory, 1_024);
    let source = source_scene(&account, &AtomicU64::new(1), false, false);
    let target = TargetSpace::try_new(
        Handedness::Right,
        Axis::PositiveY,
        Axis::NegativeZ,
        1.0,
        FrontFaceWinding::Clockwise,
    )
    .unwrap();
    let conversion =
        SpaceConversion::try_new(&SceneSpace::UNKNOWN, target, Some(TargetSpace::STANDARD))
            .unwrap();
    let nodes = NodeSpacePlan::try_new(
        source.scene.read_nodes(),
        conversion,
        &mut Control::default(),
    )
    .unwrap();
    let geometry = GeometrySpacePlan::try_new(
        source.scene.read_geometry(),
        conversion,
        &mut Control::default(),
    )
    .unwrap();
    let options = OptionsDigest::sha256([44; 32]);
    let pending = BoundPendingNodeSceneEdit::try_space_conversion(
        &source.scene,
        nodes,
        geometry,
        conversion,
        options,
        &mut || Ok(()),
    )
    .unwrap();
    let plan = pending.counts.profile;
    let selected = [
        CanonicalProfileOwner::Scene,
        CanonicalProfileOwner::Primitive(0),
        CanonicalProfileOwner::Primitive(1),
        CanonicalProfileOwner::Attribute(1),
    ];
    assert_eq!(plan.target_count.as_usize(), selected.len());
    let output = retain_edit(pending);
    let profile = &output.output().profile;
    assert!(std::ptr::eq(
        output.output().nodes.as_ref(),
        source.scene.tables.nodes.as_ref()
    ));
    let state = profile.state.as_ref();
    let lineage = state.segments.last().unwrap().lineage.as_ref();
    for (local, owner) in selected.into_iter().enumerate() {
        assert_eq!(
            lineage.provenance[local].parents.get(&lineage.parents),
            Some(
                expected_parents(&source.scene, &[owner, CanonicalProfileOwner::Scene]).as_slice()
            )
        );
        assert!(matches!(
            lineage.operations[local].kind,
            StoredOperationKind::Transform
        ));
        assert_eq!(lineage.operations[local].options, options);
        assert_eq!(attachments(profile, owner).len(), 1);
    }
    for owner in [
        CanonicalProfileOwner::Attribute(0),
        CanonicalProfileOwner::Attribute(2),
        CanonicalProfileOwner::Attribute(3),
        CanonicalProfileOwner::Attribute(4),
        CanonicalProfileOwner::Attribute(5),
    ] {
        assert_eq!(
            attachments(profile, owner),
            attachments(&source.scene.tables.profile, owner)
        );
    }
    assert!(std::ptr::eq(
        profile.metadata.as_ref(),
        source.scene.tables.profile.metadata.as_ref()
    ));
    assert_metadata(profile, false, 1, 5);
    assert_valid_profile(profile, plan);
    assert!(
        geometry
            .validate_correspondence(output.output().geometry.as_ref(), &mut Control::default())
            .unwrap()
    );
    drop(output);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn declaration_only_conversion_shares_node_geometry_and_metadata_owners() {
    use crate::canonical_scene::store::typed_storage::edit::space_geometry::GeometrySpacePlan;
    use crate::canonical_scene::store::typed_storage::edit::space_nodes::NodeSpacePlan;
    use crate::space_conversion::{SpaceConversion, TargetSpace};
    let factory = test_factory();
    let account = test_account(&factory, 1_024);
    let source = source_scene(&account, &AtomicU64::new(1), false, false);
    let conversion = SpaceConversion::try_new(
        &SceneSpace::UNKNOWN,
        TargetSpace::STANDARD,
        Some(TargetSpace::STANDARD),
    )
    .unwrap();
    let nodes = NodeSpacePlan::try_new(
        source.scene.read_nodes(),
        conversion,
        &mut Control::default(),
    )
    .unwrap();
    let geometry = GeometrySpacePlan::try_new(
        source.scene.read_geometry(),
        conversion,
        &mut Control::default(),
    )
    .unwrap();
    let pending = BoundPendingNodeSceneEdit::try_space_conversion(
        &source.scene,
        nodes,
        geometry,
        conversion,
        OptionsDigest::sha256([45; 32]),
        &mut || Ok(()),
    )
    .unwrap();
    let plan = pending.counts.profile;
    assert_eq!(plan.target_count.as_u32(), 1);
    let output = retain_edit(pending);
    let tables = output.output();
    assert!(std::ptr::eq(
        tables.nodes.as_ref(),
        source.scene.tables.nodes.as_ref()
    ));
    assert!(std::ptr::eq(
        tables.geometry.as_ref(),
        source.scene.tables.geometry.as_ref()
    ));
    assert!(!std::ptr::eq(
        tables.spatial.as_ref(),
        source.scene.tables.spatial.as_ref()
    ));
    assert!(std::ptr::eq(
        tables.profile.metadata.as_ref(),
        source.scene.tables.profile.metadata.as_ref()
    ));
    assert_eq!(tables.spatial.as_ref().space, SceneSpace::STANDARD_TARGET);
    assert_valid_profile(&tables.profile, plan);
    assert_eq!(
        source.scene.tables.spatial.as_ref().space,
        SceneSpace::UNKNOWN
    );
    drop(output);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
