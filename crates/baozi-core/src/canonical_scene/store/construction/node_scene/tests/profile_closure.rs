use std::sync::atomic::AtomicU64;

use baozi_operation::{PhysicalStorageAccount, PhysicalStorageAccountSnapshot, StoreRuntimeError};

use crate::SceneValidationLimits;
use crate::canonical_scene::store::construction::node_scene::semantic::{
    NodeSceneProfileMetadataValue, NodeSceneProfileProvenanceKind,
    NodeSceneProfileResourceStability, NodeSceneProfileSourceLocator, NodeSceneProfileStep,
    NodeSceneSemanticError,
};
use crate::canonical_scene::store::construction::node_scene::snapshot::{
    NodeSceneSnapshot, NodeSceneSnapshotLimits,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneConstructionRejection, NodeSceneConstructionTerminal, NodeSceneDraftBuild,
    NodeSceneSuccessParts, NodeSceneTestTransaction, PreparedNodeSceneFailure, SceneStorage,
};
use crate::canonical_scene::store::construction::{STORE_DIMENSION, SceneConstructionError};
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, CameraAuthoring, CameraLocalId,
    CanonicalProfileOwner, DeformationAuthoringBatch, GeometryAuthoringBatch, LightAuthoring,
    LightLocalId, MetadataAuthoringBatch, MetadataAuthoringEntry, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, OwnerProvenanceAuthoring, Presence, ProfileAuthoringFailure,
    ProfileAuthoringLimit, ProfileAuthoringSemanticError, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ResourceIdentityLocalId, SceneAuthoringBatch, SessionOpaqueReason,
    SourceIdentityAuthoring, SourceIdentityLocalId, SpatialAuthoringBatch, SpatialAuthoringFailure,
    SpatialAuthoringSemanticError,
};
use crate::entity::{AuthoredTransform, CameraProjection, LightEmission, LightKind};
use crate::extension::{ExtensionMediaType, ExtensionSchemaId, ExtensionValueRef};
use crate::identity::SceneGenerationId;
use crate::math::Color;
use crate::namespaced::NamespacedName;
use crate::provenance::SourceObjectKind;
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpace, TangentBasisConvention,
    TangentHandednessEncoding, UvOrigin,
};

use super::{profile_fixture, terminal_failure_class, terminal_failure_code, test_limits};

const OUT_OF_RANGE_PROFILE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(1)];
const PROFILE_CLOSURE_METADATA_BYTES: u64 = 170;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileClosureInputVariant {
    Baseline,
    MetadataMutation,
    SceneSpaceMutation,
    CameraOutOfRange,
    LightOutOfRange,
    SourceResourceOutOfRange,
    ProvenanceSourceOutOfRange,
    OwnerProvenanceOutOfRange,
}

impl ProfileClosureInputVariant {
    const fn camera(self) -> CameraLocalId {
        if matches!(self, Self::CameraOutOfRange) {
            CameraLocalId::new(1)
        } else {
            CameraLocalId::new(0)
        }
    }

    const fn light(self) -> LightLocalId {
        if matches!(self, Self::LightOutOfRange) {
            LightLocalId::new(1)
        } else {
            LightLocalId::new(0)
        }
    }

    const fn metadata_version(self) -> u64 {
        if matches!(self, Self::MetadataMutation) {
            8
        } else {
            7
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct ProfileClosureCanonicalShapeObservation {
    pub(in crate::canonical_scene::store) node_count: usize,
    pub(in crate::canonical_scene::store) root_count: usize,
    pub(in crate::canonical_scene::store) camera_count: usize,
    pub(in crate::canonical_scene::store) light_count: usize,
    pub(in crate::canonical_scene::store) camera_attachment_count: usize,
    pub(in crate::canonical_scene::store) light_attachment_count: usize,
    pub(in crate::canonical_scene::store) unit_scale_to_meters: f64,
    pub(in crate::canonical_scene::store) profile_resource_count: usize,
    pub(in crate::canonical_scene::store) profile_source_count: usize,
    pub(in crate::canonical_scene::store) profile_provenance_count: usize,
    pub(in crate::canonical_scene::store) profile_owner_count: usize,
    pub(in crate::canonical_scene::store) profile_attachment_count: usize,
    pub(in crate::canonical_scene::store) profile_metadata_entry_count: usize,
}

pub(in crate::canonical_scene::store) struct ProfileClosureDigestVectors {
    pub(in crate::canonical_scene::store) baseline: [u8; 32],
    pub(in crate::canonical_scene::store) metadata_u64_mutation: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct ProfileClosureNegativeCaseObservation {
    pub(in crate::canonical_scene::store) scenario_id: &'static str,
    pub(in crate::canonical_scene::store) preflight_disposition: &'static str,
    pub(in crate::canonical_scene::store) activation_eligibility_reached: bool,
    pub(in crate::canonical_scene::store) failure_class: &'static str,
    pub(in crate::canonical_scene::store) failure_kind: &'static str,
    pub(in crate::canonical_scene::store) failure_code: u32,
    pub(in crate::canonical_scene::store) preflight_storage:
        ProfileClosurePreflightStorageObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct ProfileClosurePreflightStorageObservation {
    pub(in crate::canonical_scene::store) peak_retained_bytes_delta: u64,
    pub(in crate::canonical_scene::store) peak_retained_allocations_delta: u64,
    pub(in crate::canonical_scene::store) total_admitted_retained_bytes_delta: u64,
    pub(in crate::canonical_scene::store) total_admitted_retained_allocations_delta: u64,
    pub(in crate::canonical_scene::store) live_retained_bytes_delta: u64,
    pub(in crate::canonical_scene::store) live_retained_allocations_delta: u64,
    pub(in crate::canonical_scene::store) final_live_retained_bytes: u64,
    pub(in crate::canonical_scene::store) final_live_retained_allocations: u64,
}

enum ProfileClosureAuthoringPreflightDisposition {
    Rejected {
        rejection: NodeSceneConstructionRejection<StoreRuntimeError>,
        // Keep the physical failure owner last so it remains the final containment boundary.
        prepared: PreparedNodeSceneFailure,
    },
}

impl ProfileClosureAuthoringPreflightDisposition {
    fn from_build_result(
        scenario_id: &'static str,
        result: Result<NodeSceneDraftBuild, SceneConstructionError>,
    ) -> Self {
        match result.unwrap_or_else(|error| {
            panic!("profile-closure negative case {scenario_id} failed to finish: {error:?}")
        }) {
            NodeSceneDraftBuild::Failure {
                authoring_rejection: Some(rejection),
                prepared,
            } => Self::Rejected {
                rejection,
                prepared,
            },
            NodeSceneDraftBuild::Failure {
                authoring_rejection: None,
                prepared,
            } => {
                drop(prepared.handoff_for_test());
                panic!(
                    "profile-closure negative case {scenario_id} lost its typed authoring rejection"
                )
            }
            NodeSceneDraftBuild::Draft { owner, .. } => {
                drop(owner);
                panic!(
                    "profile-closure negative case {scenario_id} unexpectedly reached activation eligibility"
                )
            }
        }
    }

    const fn report_name(&self) -> &'static str {
        match self {
            Self::Rejected { .. } => "rejected",
        }
    }

    const fn activation_eligibility_reached(&self) -> bool {
        match self {
            Self::Rejected { .. } => false,
        }
    }

    fn rejection(&self) -> &NodeSceneConstructionRejection<StoreRuntimeError> {
        match self {
            Self::Rejected { rejection, .. } => rejection,
        }
    }

    fn handoff(self) -> NodeSceneConstructionTerminal {
        match self {
            Self::Rejected { prepared, .. } => prepared.handoff_for_test(),
        }
    }
}

fn retained_counter_delta(after: u64, before: u64, counter: &str) -> u64 {
    after.checked_sub(before).unwrap_or_else(|| {
        panic!("profile-closure retained counter {counter} regressed during preflight")
    })
}

fn freeze_preflight_storage(
    before_preflight: PhysicalStorageAccountSnapshot,
    while_rejected: PhysicalStorageAccountSnapshot,
    after_rejection_drop: PhysicalStorageAccountSnapshot,
) -> ProfileClosurePreflightStorageObservation {
    ProfileClosurePreflightStorageObservation {
        peak_retained_bytes_delta: retained_counter_delta(
            while_rejected.peak_retained_bytes(),
            before_preflight.peak_retained_bytes(),
            "peak retained bytes",
        ),
        peak_retained_allocations_delta: retained_counter_delta(
            while_rejected.peak_retained_allocations(),
            before_preflight.peak_retained_allocations(),
            "peak retained allocations",
        ),
        total_admitted_retained_bytes_delta: retained_counter_delta(
            while_rejected.total_admitted_retained_bytes(),
            before_preflight.total_admitted_retained_bytes(),
            "total admitted retained bytes",
        ),
        total_admitted_retained_allocations_delta: retained_counter_delta(
            while_rejected.total_admitted_retained_allocations(),
            before_preflight.total_admitted_retained_allocations(),
            "total admitted retained allocations",
        ),
        live_retained_bytes_delta: retained_counter_delta(
            while_rejected.live_retained_bytes(),
            before_preflight.live_retained_bytes(),
            "live retained bytes",
        ),
        live_retained_allocations_delta: retained_counter_delta(
            while_rejected.live_retained_allocations(),
            before_preflight.live_retained_allocations(),
            "live retained allocations",
        ),
        final_live_retained_bytes: after_rejection_drop.live_retained_bytes(),
        final_live_retained_allocations: after_rejection_drop.live_retained_allocations(),
    }
}

fn extension_schema(name: &str, version: u32) -> ExtensionSchemaId {
    ExtensionSchemaId::new(name, version).expect("profile-closure extension schema")
}

fn profile_scene_space(variant: ProfileClosureInputVariant) -> SceneSpace {
    SceneSpace {
        handedness: Handedness::Left,
        up_axis: Axis::NegativeY,
        front_axis: Axis::PositiveZ,
        unit_scale_to_meters: Some(
            if matches!(variant, ProfileClosureInputVariant::SceneSpaceMutation) {
                0.02
            } else {
                0.01
            },
        ),
        front_face_winding: FrontFaceWinding::Clockwise,
        uv_origin: UvOrigin::LowerLeft,
        tangent_basis: TangentBasisConvention::try_namespaced(Some(extension_schema(
            "baozi-tests:profile-closure-tangent-basis",
            1,
        )))
        .expect("profile-closure tangent-basis convention"),
        tangent_handedness_encoding: TangentHandednessEncoding::try_namespaced(Some(
            extension_schema("baozi-tests:profile-closure-tangent-sign", 1),
        ))
        .expect("profile-closure tangent encoding convention"),
    }
}

fn with_profile_closure_input<R>(
    use_mutated_metadata: bool,
    inspect: impl FnOnce(SceneAuthoringBatch<'_>) -> R,
) -> R {
    let variant = if use_mutated_metadata {
        ProfileClosureInputVariant::MetadataMutation
    } else {
        ProfileClosureInputVariant::Baseline
    };
    with_profile_closure_input_variant(variant, inspect)
}

fn with_profile_closure_input_variant<R>(
    variant: ProfileClosureInputVariant,
    inspect: impl FnOnce(SceneAuthoringBatch<'_>) -> R,
) -> R {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Present(variant.camera()),
        Presence::Present(variant.light()),
    )];
    let camera_projection = CameraProjection::perspective(0.75, Some(16.0 / 9.0), 0.1, Some(500.0))
        .expect("profile-closure camera projection");
    let cameras = [CameraAuthoring::new(camera_projection)];
    let light = LightKind::point(Some(25.0)).expect("profile-closure point light");
    let emission = LightEmission::try_from_scaled_linear_srgb(Color::WHITE, 4.0)
        .expect("profile-closure light emission");
    let lights = [LightAuthoring::new(light, emission)];
    let scene_space = profile_scene_space(variant);
    let spatial = SpatialAuthoringBatch::new(&scene_space, &cameras, &lights);
    let node_batch = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::new(&[], &[], &[], &[], &[]);
    let animation = AnimationAuthoringBatch::empty();
    let mut profile = profile_fixture(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animation,
    );
    match variant {
        ProfileClosureInputVariant::SourceResourceOutOfRange => {
            let source = profile.sources[0];
            profile.sources[0] = SourceIdentityAuthoring::new(
                ResourceIdentityLocalId::new(1),
                source.format(),
                source.locator(),
            );
        }
        ProfileClosureInputVariant::ProvenanceSourceOutOfRange => {
            profile.provenance[0] = ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(1));
        }
        ProfileClosureInputVariant::OwnerProvenanceOutOfRange => {
            profile.owners[0] = OwnerProvenanceAuthoring::new(
                ProfileOwner::Scene,
                &OUT_OF_RANGE_PROFILE_PROVENANCE,
            );
        }
        ProfileClosureInputVariant::Baseline
        | ProfileClosureInputVariant::MetadataMutation
        | ProfileClosureInputVariant::SceneSpaceMutation
        | ProfileClosureInputVariant::CameraOutOfRange
        | ProfileClosureInputVariant::LightOutOfRange => {}
    }

    let metadata_keys = [
        NamespacedName::new("baozi-tests:profile-name").expect("profile name metadata key"),
        NamespacedName::new("baozi-tests:profile-version").expect("profile version metadata key"),
        NamespacedName::new("baozi-tests:profile-enabled").expect("profile bool metadata key"),
        NamespacedName::new("baozi-tests:profile-payload").expect("profile bytes metadata key"),
    ];
    let media_type = ExtensionMediaType::new("application/vnd.baozi.profile-closure")
        .expect("profile-closure metadata media type");
    let metadata_payload = [0x42_u8, 0x5a, 0x49, 0x52];
    let version = variant.metadata_version();
    let metadata = [
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &metadata_keys[0],
            ExtensionValueRef::String("profile-closure"),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &metadata_keys[1],
            ExtensionValueRef::U64(version),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Node(NodeLocalId::new(0)),
            &metadata_keys[2],
            ExtensionValueRef::Bool(true),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Camera(0),
            &metadata_keys[3],
            ExtensionValueRef::Bytes {
                media_type: &media_type,
                payload: &metadata_payload,
            },
        ),
    ];
    inspect(SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animation,
        profile.batch_with_metadata(MetadataAuthoringBatch::new(&metadata)),
    ))
}

fn build_profile_closure_scene(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
    use_mutated_metadata: bool,
) -> NodeSceneSuccessParts {
    let variant = if use_mutated_metadata {
        ProfileClosureInputVariant::MetadataMutation
    } else {
        ProfileClosureInputVariant::Baseline
    };
    build_profile_closure_scene_variant(account, generation_source, variant)
}

fn build_profile_closure_scene_variant(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
    variant: ProfileClosureInputVariant,
) -> NodeSceneSuccessParts {
    with_profile_closure_input_variant(variant, |input| {
        let build = NodeSceneTestTransaction::with_account(
            account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
        )
        .expect("profile-closure Scene admission")
        .finish_draft()
        .expect("profile-closure Scene draft build");
        let terminal = build
            .publish_with_generation_source_for_test(|| {
                SceneGenerationId::reserve_from(generation_source)
            })
            .expect("profile-closure Scene publication");
        match terminal {
            NodeSceneConstructionTerminal::Success(parts) => parts,
            NodeSceneConstructionTerminal::RichFailure(_) => {
                panic!("profile-closure Scene published rich failure")
            }
            NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
                panic!(
                    "profile-closure Scene published minimal fallback: class={:?}, code={}",
                    terminal_failure_class(fallback.ledger_evidence()),
                    terminal_failure_code(fallback.ledger_evidence()),
                )
            }
        }
    })
}

pub(in crate::canonical_scene::store) fn build_profile_closure_scene_for_probe(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
) -> NodeSceneSuccessParts {
    build_profile_closure_scene(account, generation_source, false)
}

fn next_profile_item<T>(
    mut step: impl FnMut() -> Result<NodeSceneProfileStep<T>, NodeSceneSemanticError>,
    context: &'static str,
) -> Option<T> {
    loop {
        match step().unwrap_or_else(|error| panic!("{context}: {error:?}")) {
            NodeSceneProfileStep::Checkpoint => {}
            NodeSceneProfileStep::Item(item) => return Some(item),
            NodeSceneProfileStep::End => return None,
        }
    }
}

pub(in crate::canonical_scene::store) fn inspect_profile_closure_semantics_for_probe(
    scene: &SceneStorage,
) -> ProfileClosureCanonicalShapeObservation {
    let view = scene
        .semantic_view()
        .expect("profile-closure semantic view");
    let node_count = view.node_count();
    let root_count = view.root_count();
    let camera_count = view.camera_count();
    let light_count = view.light_count();
    let scene_space = view.scene_space().expect("profile-closure SceneSpace");
    assert_eq!(scene_space.handedness(), Handedness::Left);
    assert_eq!(scene_space.up_axis(), Axis::NegativeY);
    assert_eq!(scene_space.front_axis(), Axis::PositiveZ);
    assert_eq!(
        scene_space.front_face_winding(),
        FrontFaceWinding::Clockwise
    );
    assert_eq!(scene_space.uv_origin(), UvOrigin::LowerLeft);
    let unit_scale_to_meters = scene_space
        .unit_scale_to_meters()
        .expect("profile-closure unit scale");

    let camera = view
        .camera_at(0)
        .expect("profile-closure camera read")
        .expect("profile-closure camera");
    let light = view
        .light_at(0)
        .expect("profile-closure light read")
        .expect("profile-closure light");
    assert_eq!(camera.source_index(), 0);
    assert_eq!(light.source_index(), 0);

    let mut placements = view.spatial_placements();
    let placement = placements
        .next()
        .expect("profile-closure placement read")
        .expect("profile-closure placement");
    assert_eq!(placement.source_node(), 0);
    assert_eq!(placement.camera(), Some(0));
    assert_eq!(placement.light(), Some(0));
    assert!(
        placements
            .next()
            .expect("profile-closure placement end")
            .is_none()
    );
    placements
        .finish()
        .expect("profile-closure placements finish");

    let mut cursor = view.into_cursor().expect("profile-closure semantic cursor");
    let mut roots = cursor.take_roots().expect("profile-closure roots");
    assert_eq!(roots.next().expect("profile-closure root"), Some(0));
    assert!(roots.next().expect("profile-closure root end").is_none());
    roots.finish().expect("profile-closure roots finish");
    let node = cursor
        .next_node()
        .expect("profile-closure node read")
        .expect("profile-closure node");
    assert_eq!(node.camera(), Some(0));
    assert_eq!(node.light(), Some(0));
    node.into_children()
        .finish()
        .expect("profile-closure node children finish");
    assert!(
        cursor
            .next_node()
            .expect("profile-closure node end")
            .is_none()
    );

    let appearance = cursor.appearance().expect("profile-closure appearance");
    assert!(
        appearance
            .next_image()
            .expect("profile image end")
            .is_none()
    );
    assert!(
        appearance
            .next_sampler()
            .expect("profile sampler end")
            .is_none()
    );
    assert!(
        appearance
            .next_texture()
            .expect("profile texture end")
            .is_none()
    );
    assert!(
        appearance
            .next_material()
            .expect("profile material end")
            .is_none()
    );
    assert!(
        appearance
            .next_binding()
            .expect("profile binding end")
            .is_none()
    );

    let geometry = cursor.geometry().expect("profile-closure geometry");
    assert!(
        geometry
            .next_logical_mesh()
            .expect("profile mesh end")
            .is_none()
    );
    assert!(
        geometry
            .next_primitive()
            .expect("profile primitive end")
            .is_none()
    );
    assert!(
        geometry
            .next_attribute()
            .expect("profile attribute end")
            .is_none()
    );
    assert!(
        geometry
            .next_mesh_instance()
            .expect("profile instance end")
            .is_none()
    );

    let deformation = cursor.deformation().expect("profile-closure deformation");
    assert!(deformation.next_skin().expect("profile skin end").is_none());
    assert!(
        deformation
            .next_logical_morph()
            .expect("profile morph end")
            .is_none()
    );
    assert!(
        deformation
            .next_morph_definition()
            .expect("profile morph definition end")
            .is_none()
    );
    assert!(
        deformation
            .next_primitive()
            .expect("profile deformation primitive end")
            .is_none()
    );
    assert!(
        deformation
            .next_morph_target()
            .expect("profile morph target end")
            .is_none()
    );
    assert!(
        deformation
            .next_morph_delta()
            .expect("profile morph delta end")
            .is_none()
    );
    assert!(
        deformation
            .next_mesh_instance()
            .expect("profile deformation instance end")
            .is_none()
    );

    let animation = cursor.animation().expect("profile-closure animation");
    assert!(animation.next_clip().expect("profile clip end").is_none());
    assert!(
        animation
            .next_channel()
            .expect("profile channel end")
            .is_none()
    );

    let profile = cursor.profile().expect("profile-closure profile");
    let observation = ProfileClosureCanonicalShapeObservation {
        node_count,
        root_count,
        camera_count,
        light_count,
        camera_attachment_count: 1,
        light_attachment_count: 1,
        unit_scale_to_meters,
        profile_resource_count: profile.resource_count(),
        profile_source_count: profile.source_count(),
        profile_provenance_count: profile.provenance_count(),
        profile_owner_count: profile.owner_count(),
        profile_attachment_count: profile.attachment_count(),
        profile_metadata_entry_count: profile.metadata_entry_count(),
    };

    let resource = next_profile_item(
        || profile.step_resource(),
        "profile-closure resource traversal",
    )
    .expect("profile-closure resource");
    assert_eq!(resource.index(), 0);
    assert_eq!(resource.identity(), "fixtures/node-scene/input");
    assert_eq!(
        resource.stability(),
        NodeSceneProfileResourceStability::SessionOpaque(SessionOpaqueReason::CallerLabel)
    );
    assert!(
        next_profile_item(|| profile.step_resource(), "profile-closure resource end",).is_none()
    );

    let source = next_profile_item(|| profile.step_source(), "profile-closure source traversal")
        .expect("profile-closure source");
    assert_eq!(source.index(), 0);
    assert_eq!(source.resource(), 0);
    assert_eq!(source.format(), "org.baozi.fixture");
    assert_eq!(
        source.locator(),
        NodeSceneProfileSourceLocator::Indexed {
            kind: SourceObjectKind::Document,
            index: 0,
        }
    );
    assert_eq!(source.display_name(), None);
    assert_eq!(source.content_digest(), None);
    assert!(next_profile_item(|| profile.step_source(), "profile-closure source end").is_none());

    let provenance = next_profile_item(
        || profile.step_provenance(),
        "profile-closure provenance traversal",
    )
    .expect("profile-closure provenance");
    assert_eq!(provenance.index(), 0);
    assert_eq!(provenance.depth(), 1);
    assert_eq!(provenance.parent_offset(), 0);
    assert_eq!(provenance.parents().len(), 0);
    assert_eq!(provenance.parents().iter().next(), None);
    assert_eq!(
        provenance.kind(),
        NodeSceneProfileProvenanceKind::Source { source: 0 }
    );
    assert!(
        next_profile_item(
            || profile.step_provenance(),
            "profile-closure provenance end",
        )
        .is_none()
    );
    assert!(
        next_profile_item(|| profile.step_evidence(), "profile-closure evidence end").is_none()
    );

    let expected_owners = [
        CanonicalProfileOwner::Scene,
        CanonicalProfileOwner::SyntheticRoot,
        CanonicalProfileOwner::Node(1),
        CanonicalProfileOwner::Camera(0),
        CanonicalProfileOwner::Light(0),
    ];
    for (attachment_offset, expected_owner) in expected_owners.into_iter().enumerate() {
        let owner = profile
            .next_owner()
            .expect("profile-closure owner read")
            .expect("profile-closure owner");
        assert_eq!(owner.owner(), expected_owner);
        assert_eq!(owner.attachment_offset(), attachment_offset);
        let attachments = owner.attachments();
        assert_eq!(attachments.len(), 1);
        let mut attachments = attachments.iter();
        assert_eq!(attachments.next(), Some(Some(0)));
        assert_eq!(attachments.next(), None);
    }
    assert!(
        profile
            .next_owner()
            .expect("profile-closure owner end")
            .is_none()
    );

    let expected_metadata = [
        (
            CanonicalProfileOwner::Scene,
            "baozi-tests:profile-name",
            NodeSceneProfileMetadataValue::String("profile-closure"),
        ),
        (
            CanonicalProfileOwner::Scene,
            "baozi-tests:profile-version",
            NodeSceneProfileMetadataValue::U64(7),
        ),
        (
            CanonicalProfileOwner::Node(1),
            "baozi-tests:profile-enabled",
            NodeSceneProfileMetadataValue::Bool(true),
        ),
        (
            CanonicalProfileOwner::Camera(0),
            "baozi-tests:profile-payload",
            NodeSceneProfileMetadataValue::Bytes {
                media_type: "application/vnd.baozi.profile-closure",
                payload: &[0x42, 0x5a, 0x49, 0x52],
            },
        ),
    ];
    for (expected_owner, expected_key, expected_value) in expected_metadata {
        let entry = profile
            .next_metadata_entry()
            .expect("profile-closure metadata read")
            .expect("profile-closure metadata entry");
        assert_eq!(entry.owner(), expected_owner);
        assert_eq!(entry.key(), expected_key);
        assert_eq!(entry.value(), expected_value);
    }
    assert!(
        profile
            .next_metadata_entry()
            .expect("profile-closure metadata end")
            .is_none()
    );
    cursor.finish().expect("profile-closure semantic traversal");
    observation
}

pub(in crate::canonical_scene::store) fn capture_profile_closure_snapshots_for_probe(
    parts: &NodeSceneSuccessParts,
) -> (NodeSceneSnapshot, NodeSceneSnapshot) {
    let full = parts
        .scene
        .capture_snapshot(
            NodeSceneSnapshotLimits::try_new(16 * 1024, 128, 1_000_000)
                .expect("profile-closure full snapshot limits"),
            None,
        )
        .expect("profile-closure full snapshot");
    let bounded = parts
        .scene
        .capture_snapshot(
            NodeSceneSnapshotLimits::try_new(96, 1, 1_000_000)
                .expect("profile-closure bounded snapshot limits"),
            None,
        )
        .expect("profile-closure bounded snapshot");
    (full, bounded)
}

pub(in crate::canonical_scene::store) fn collect_profile_closure_digest_vectors_for_probe()
-> ProfileClosureDigestVectors {
    let factory = super::test_factory();
    let account = super::test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let baseline = build_profile_closure_scene(&account, &generation_source, false);
    let mutation = build_profile_closure_scene(&account, &generation_source, true);
    let digest_limits =
        NodeSceneSnapshotLimits::try_new(0, 0, 1_000_000).expect("profile-closure digest limits");
    let baseline_digest = baseline
        .scene
        .capture_snapshot(digest_limits, None)
        .expect("profile-closure baseline digest")
        .digest;
    let metadata_u64_mutation = mutation
        .scene
        .capture_snapshot(digest_limits, None)
        .expect("profile-closure metadata mutation digest")
        .digest;
    drop(mutation);
    drop(baseline);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
    ProfileClosureDigestVectors {
        baseline: baseline_digest,
        metadata_u64_mutation,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileClosureNegativeCase {
    CameraOutOfRange,
    LightOutOfRange,
    SourceResourceOutOfRange,
    ProvenanceSourceOutOfRange,
    OwnerProvenanceOutOfRange,
    MetadataBytesOneShort,
}

impl ProfileClosureNegativeCase {
    const ALL: [Self; 6] = [
        Self::CameraOutOfRange,
        Self::LightOutOfRange,
        Self::SourceResourceOutOfRange,
        Self::ProvenanceSourceOutOfRange,
        Self::OwnerProvenanceOutOfRange,
        Self::MetadataBytesOneShort,
    ];

    const fn scenario_id(self) -> &'static str {
        match self {
            Self::CameraOutOfRange => "canonical-profile-camera-reference-out-of-range-v1",
            Self::LightOutOfRange => "canonical-profile-light-reference-out-of-range-v1",
            Self::SourceResourceOutOfRange => {
                "canonical-profile-source-resource-reference-out-of-range-v1"
            }
            Self::ProvenanceSourceOutOfRange => {
                "canonical-profile-provenance-source-reference-out-of-range-v1"
            }
            Self::OwnerProvenanceOutOfRange => {
                "canonical-profile-owner-provenance-reference-out-of-range-v1"
            }
            Self::MetadataBytesOneShort => "canonical-profile-metadata-bytes-one-short-v1",
        }
    }

    const fn input_variant(self) -> ProfileClosureInputVariant {
        match self {
            Self::CameraOutOfRange => ProfileClosureInputVariant::CameraOutOfRange,
            Self::LightOutOfRange => ProfileClosureInputVariant::LightOutOfRange,
            Self::SourceResourceOutOfRange => ProfileClosureInputVariant::SourceResourceOutOfRange,
            Self::ProvenanceSourceOutOfRange => {
                ProfileClosureInputVariant::ProvenanceSourceOutOfRange
            }
            Self::OwnerProvenanceOutOfRange => {
                ProfileClosureInputVariant::OwnerProvenanceOutOfRange
            }
            Self::MetadataBytesOneShort => ProfileClosureInputVariant::Baseline,
        }
    }

    fn validation_limits(self) -> SceneValidationLimits {
        let limits = SceneValidationLimits::default();
        match self {
            Self::MetadataBytesOneShort => {
                limits.with_max_metadata_bytes(PROFILE_CLOSURE_METADATA_BYTES - 1)
            }
            Self::CameraOutOfRange
            | Self::LightOutOfRange
            | Self::SourceResourceOutOfRange
            | Self::ProvenanceSourceOutOfRange
            | Self::OwnerProvenanceOutOfRange => limits,
        }
    }
}

pub(in crate::canonical_scene::store) fn collect_profile_closure_negative_cases_for_probe()
-> Vec<ProfileClosureNegativeCaseObservation> {
    ProfileClosureNegativeCase::ALL
        .into_iter()
        .map(observe_profile_closure_negative_case)
        .collect()
}

fn observe_profile_closure_negative_case(
    case: ProfileClosureNegativeCase,
) -> ProfileClosureNegativeCaseObservation {
    let factory = super::test_factory();
    let account = super::test_account(&factory, 512);

    let (disposition, before_preflight) = with_profile_closure_input_variant(
        case.input_variant(),
        |input| {
            let operation = NodeSceneTestTransaction::with_account(
                &account,
                test_limits(),
                input,
                case.validation_limits(),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "profile-closure negative case {} failed before operation-owned preflight: {error:?}",
                    case.scenario_id()
                )
            });
            let before_preflight = account.snapshot();
            (
                ProfileClosureAuthoringPreflightDisposition::from_build_result(
                    case.scenario_id(),
                    operation.finish_draft(),
                ),
                before_preflight,
            )
        },
    );
    let while_rejected = account.snapshot();

    let (failure_class, failure_kind) =
        classify_profile_closure_rejection(case, disposition.rejection());
    let failure_code = disposition.rejection().stable_code();
    let preflight_disposition = disposition.report_name();
    let activation_eligibility_reached = disposition.activation_eligibility_reached();
    match disposition.handoff() {
        NodeSceneConstructionTerminal::RichFailure(failure) => {
            assert_eq!(
                terminal_failure_code(failure.ledger_evidence()),
                failure_code,
                "profile-closure rich failure must preserve the typed authoring rejection code"
            );
            assert_eq!(
                failure
                    .ledger_evidence()
                    .dimension_by_semantic(STORE_DIMENSION)
                    .expect("profile-closure rejection ordinary dimension")
                    .committed_canonical_output(),
                0,
                "authoring rejection must not commit canonical output"
            );
            drop(failure);
        }
        NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
            assert_eq!(
                fallback
                    .ledger_evidence()
                    .dimension_by_semantic(STORE_DIMENSION)
                    .expect("profile-closure fallback ordinary dimension")
                    .committed_canonical_output(),
                0,
                "authoring fallback must not commit canonical output"
            );
            drop(fallback);
        }
        NodeSceneConstructionTerminal::Success(parts) => {
            drop(parts);
            panic!(
                "profile-closure negative case {} unexpectedly published a Scene",
                case.scenario_id()
            );
        }
    }

    let preflight_storage =
        freeze_preflight_storage(before_preflight, while_rejected, account.snapshot());
    assert_eq!(preflight_storage.final_live_retained_bytes, 0);
    assert_eq!(preflight_storage.final_live_retained_allocations, 0);
    let observation = ProfileClosureNegativeCaseObservation {
        scenario_id: case.scenario_id(),
        preflight_disposition,
        activation_eligibility_reached,
        failure_class,
        failure_kind,
        failure_code,
        preflight_storage,
    };
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
    observation
}

fn classify_profile_closure_rejection(
    case: ProfileClosureNegativeCase,
    rejection: &NodeSceneConstructionRejection<StoreRuntimeError>,
) -> (&'static str, &'static str) {
    match (case, rejection) {
        (
            ProfileClosureNegativeCase::CameraOutOfRange,
            NodeSceneConstructionRejection::SpatialAuthoring(SpatialAuthoringFailure::Semantic(
                SpatialAuthoringSemanticError::CameraOutOfRange {
                    node,
                    camera,
                    camera_count,
                },
            )),
        ) => {
            assert_eq!((*node, *camera, *camera_count), (0, 1, 1));
            ("spatial-authoring", "camera-out-of-range")
        }
        (
            ProfileClosureNegativeCase::LightOutOfRange,
            NodeSceneConstructionRejection::SpatialAuthoring(SpatialAuthoringFailure::Semantic(
                SpatialAuthoringSemanticError::LightOutOfRange {
                    node,
                    light,
                    light_count,
                },
            )),
        ) => {
            assert_eq!((*node, *light, *light_count), (0, 1, 1));
            ("spatial-authoring", "light-out-of-range")
        }
        (
            ProfileClosureNegativeCase::SourceResourceOutOfRange,
            NodeSceneConstructionRejection::ProfileAuthoring(ProfileAuthoringFailure::Semantic(
                ProfileAuthoringSemanticError::ResourceOutOfRange {
                    source_index,
                    resource,
                    resource_count,
                },
            )),
        ) => {
            assert_eq!((*source_index, *resource, *resource_count), (0, 1, 1));
            ("profile-authoring", "source-resource-out-of-range")
        }
        (
            ProfileClosureNegativeCase::ProvenanceSourceOutOfRange,
            NodeSceneConstructionRejection::ProfileAuthoring(ProfileAuthoringFailure::Semantic(
                ProfileAuthoringSemanticError::SourceIdentityOutOfRange {
                    node,
                    source_index,
                    source_count,
                },
            )),
        ) => {
            assert_eq!((*node, *source_index, *source_count), (0, 1, 1));
            ("profile-authoring", "provenance-source-out-of-range")
        }
        (
            ProfileClosureNegativeCase::OwnerProvenanceOutOfRange,
            NodeSceneConstructionRejection::ProfileAuthoring(ProfileAuthoringFailure::Semantic(
                ProfileAuthoringSemanticError::ProvenanceOutOfRange {
                    usage,
                    provenance,
                    provenance_count,
                },
            )),
        ) => {
            assert_eq!(*usage, "owner provenance attachment");
            assert_eq!((*provenance, *provenance_count), (1, 1));
            ("profile-authoring", "owner-provenance-out-of-range")
        }
        (
            ProfileClosureNegativeCase::MetadataBytesOneShort,
            NodeSceneConstructionRejection::ProfileAuthoring(ProfileAuthoringFailure::Semantic(
                ProfileAuthoringSemanticError::LimitExceeded {
                    limit,
                    actual,
                    maximum,
                },
            )),
        ) => {
            assert_eq!(*limit, ProfileAuthoringLimit::MetadataBytes);
            assert_eq!((*actual, *maximum), (PROFILE_CLOSURE_METADATA_BYTES, 169));
            ("profile-authoring", "metadata-bytes-limit-exceeded")
        }
        _ => panic!(
            "profile-closure negative case {} produced unexpected rejection: {rejection:?}",
            case.scenario_id()
        ),
    }
}

#[test]
fn profile_closure_round_trips_spatial_lineage_and_metadata() {
    let factory = super::test_factory();
    let account = super::test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_profile_closure_scene_for_probe(&account, &generation_source);
    let observation = inspect_profile_closure_semantics_for_probe(&parts.scene);
    assert_eq!(observation.node_count, 1);
    assert_eq!(observation.root_count, 1);
    assert_eq!(observation.camera_count, 1);
    assert_eq!(observation.light_count, 1);
    assert_eq!(observation.camera_attachment_count, 1);
    assert_eq!(observation.light_attachment_count, 1);
    assert_eq!(observation.unit_scale_to_meters, 0.01);
    assert_eq!(observation.profile_resource_count, 1);
    assert_eq!(observation.profile_source_count, 1);
    assert_eq!(observation.profile_provenance_count, 1);
    assert_eq!(observation.profile_owner_count, 5);
    assert_eq!(observation.profile_attachment_count, 5);
    assert_eq!(observation.profile_metadata_entry_count, 4);

    let (full, bounded) = capture_profile_closure_snapshots_for_probe(&parts);
    assert_eq!(full.totals.camera_count, 1);
    assert_eq!(full.totals.light_count, 1);
    assert_eq!(full.totals.profile_metadata_entry_count, 4);
    assert!(bounded.preview.bytes_truncated || bounded.preview.items_truncated);
    assert_eq!(
        parts
            .report
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("profile-closure ordinary dimension")
            .committed_canonical_output(),
        u64::try_from(parts.scene.canonical_block_count())
            .expect("profile-closure canonical block count")
    );

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn profile_and_scene_space_mutations_change_semantic_receipts() {
    let factory = super::test_factory();
    let account = super::test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let baseline = build_profile_closure_scene(&account, &generation_source, false);
    let mutation = build_profile_closure_scene(&account, &generation_source, true);
    let scene_space_mutation = build_profile_closure_scene_variant(
        &account,
        &generation_source,
        ProfileClosureInputVariant::SceneSpaceMutation,
    );
    let baseline_snapshot = baseline
        .scene
        .capture_snapshot(
            NodeSceneSnapshotLimits::try_new(0, 0, 1_000_000)
                .expect("profile baseline digest limits"),
            None,
        )
        .expect("profile baseline digest");
    let mutation_snapshot = mutation
        .scene
        .capture_snapshot(
            NodeSceneSnapshotLimits::try_new(0, 0, 1_000_000)
                .expect("profile mutation digest limits"),
            None,
        )
        .expect("profile mutation digest");
    let baseline_facade = super::project_gltf_facade_observation_v1_for_probe(&baseline.scene)
        .expect("profile baseline facade observation");
    let metadata_facade = super::project_gltf_facade_observation_v1_for_probe(&mutation.scene)
        .expect("profile metadata facade observation");
    let scene_space_facade =
        super::project_gltf_facade_observation_v1_for_probe(&scene_space_mutation.scene)
            .expect("profile SceneSpace facade observation");
    assert_ne!(baseline_snapshot.digest, mutation_snapshot.digest);
    assert_eq!(baseline_facade.totals, metadata_facade.totals);
    assert_eq!(baseline_facade.totals, scene_space_facade.totals);
    assert_ne!(
        baseline_facade.transcript_digest,
        metadata_facade.transcript_digest
    );
    assert_ne!(
        baseline_facade.transcript_digest,
        scene_space_facade.transcript_digest
    );
    assert_ne!(
        metadata_facade.transcript_digest,
        scene_space_facade.transcript_digest
    );
    drop(scene_space_mutation);
    drop(mutation);
    drop(baseline);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn preflight_storage_observation_detects_nonzero_admission_deltas() {
    let factory = super::test_factory();
    let account = super::test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let before = account.snapshot();
    let parts = build_profile_closure_scene_for_probe(&account, &generation_source);
    let while_retained = account.snapshot();
    drop(parts);
    let after_release = account.snapshot();
    let storage = freeze_preflight_storage(before, while_retained, after_release);

    assert!(storage.peak_retained_bytes_delta > 0);
    assert!(storage.peak_retained_allocations_delta > 0);
    assert!(storage.total_admitted_retained_bytes_delta > 0);
    assert!(storage.total_admitted_retained_allocations_delta > 0);
    assert!(storage.live_retained_bytes_delta > 0);
    assert!(storage.live_retained_allocations_delta > 0);
    assert_eq!(storage.final_live_retained_bytes, 0);
    assert_eq!(storage.final_live_retained_allocations, 0);
}

#[test]
fn profile_closure_negative_cases_reject_before_activation_and_release_failure_publication() {
    let observations = collect_profile_closure_negative_cases_for_probe();
    assert_eq!(observations.len(), ProfileClosureNegativeCase::ALL.len());
    for observation in observations {
        assert_eq!(observation.preflight_disposition, "rejected");
        assert!(!observation.activation_eligibility_reached);
        assert_eq!(observation.preflight_storage.final_live_retained_bytes, 0);
        assert_eq!(
            observation
                .preflight_storage
                .final_live_retained_allocations,
            0
        );
        assert!(matches!(
            observation.failure_class,
            "spatial-authoring" | "profile-authoring"
        ));
        assert_eq!(
            observation.failure_code,
            match observation.failure_class {
                "spatial-authoring" => 0x5343_1107,
                "profile-authoring" => 0x5343_1108,
                _ => unreachable!("failure class was checked above"),
            }
        );
    }
}
