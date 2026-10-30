use crate::SceneValidationLimits;
use crate::appearance::{
    AlphaMode, ColorInterpretation, MaterialFactors, MaterialSemanticsRef, ShadingModel,
    TextureBindingSemanticsRef, TextureBlendOperation, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength, TextureTransform,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneConstructionRejection, validate_node_scene_batches_for_test,
};

use super::super::plan::animation::{
    AnimationTargetCatalog, validate_animation_batch_with_control,
};
use super::super::plan::appearance::validate_appearance_batch_with_control;
use super::super::plan::deformation::validate_deformation_batch_with_control;
use super::super::plan::geometry::validate_geometry_batch_with_control;
use super::super::plan::profile::{ProfileOwnerCatalog, validate_profile_batch_with_control};
use super::super::plan::spatial::validate_spatial_batch_with_control;
use super::super::scene::{
    SceneAuthoringAggregateError, SceneAuthoringAggregateLimit, SceneInputEnvelope,
    SceneInputEnvelopeError, animation_limits_after_deformation, appearance_limits_after_nodes,
    deformation_limits_after_geometry, geometry_limits_after_appearance, node_limits_after_spatial,
    profile_limits_after_animation, validate_scene_aggregate,
};
use super::super::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationExtrapolation,
    AnimationInterpolation, AnimationTargetAuthoring, AppearanceAuthoringBatch, AuthoringRange,
    DeformationAuthoringBatch, GeometryAuthoringBatch, MaterialAuthoring, MaterialBindingAuthoring,
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, Presence, ProfileAuthoringBatch,
    SceneAuthoringBatch, SpatialAuthoringBatch, TextureLocalId,
};
use super::{
    LineageAuthoringBatch, MetadataAuthoringBatch, TestControl, profile_fixture, trs, validate,
};
use crate::scene_space::SceneSpace;

struct EmptySceneInput<'input> {
    nodes: NodeAuthoringBatch<'input>,
    profile: super::ProfileFixture,
}

impl EmptySceneInput<'_> {
    fn batch(&self) -> SceneAuthoringBatch<'_> {
        let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
        let appearance = AppearanceAuthoringBatch::empty();
        let geometry = GeometryAuthoringBatch::empty();
        let deformation = DeformationAuthoringBatch::empty();
        let animations = AnimationAuthoringBatch::empty();
        SceneAuthoringBatch::new(
            spatial,
            self.nodes,
            appearance,
            geometry,
            deformation,
            animations,
            self.profile.batch(),
        )
    }
}

fn empty_scene_input<'input>(
    roots: Presence<&'input [NodeLocalId]>,
    nodes: &'input [NodeAuthoringNode],
) -> EmptySceneInput<'input> {
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let nodes = NodeAuthoringBatch::new(roots, nodes);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::empty();
    let animations = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animations,
    );
    EmptySceneInput { nodes, profile }
}

fn checked_total<const N: usize>(values: [u64; N]) -> u64 {
    values
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .expect("test metric total is representable")
}

#[test]
fn generic_scene_preflight_releases_prior_domains_after_profile_rejection() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let node_input = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::empty();
    let animation = AnimationAuthoringBatch::empty();
    let invalid_profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(&[], &[], &[], &[]),
        MetadataAuthoringBatch::empty(),
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        node_input,
        appearance,
        geometry,
        deformation,
        animation,
        invalid_profile,
    );
    let mut control = TestControl::default();

    let rejection = match validate_node_scene_batches_for_test(
        input,
        SceneValidationLimits::default(),
        &mut control,
    ) {
        Ok(_) => panic!("incomplete profile lineage must reject full-scene preflight"),
        Err(rejection) => rejection,
    };

    assert!(matches!(
        rejection,
        NodeSceneConstructionRejection::ProfileAuthoring(_)
    ));
    assert!(
        control.peak_bytes() > 0,
        "earlier domains must have acquired validation storage before the late rejection"
    );
    assert_eq!(control.live.get(), 0);
    assert_eq!(control.current_bytes.get(), 0);
}

#[test]
fn scene_envelope_accounts_for_the_complete_scene_shape() {
    let empty_roots = [];
    let empty_nodes = [];
    let empty_input = empty_scene_input(Presence::Present(&empty_roots), &empty_nodes);
    let empty = SceneInputEnvelope::try_new(empty_input.batch(), SceneValidationLimits::default())
        .expect("the empty Scene shape is representable");
    // Node, geometry, profile, and spatial use typed owners outside the record-store table.
    // The remaining Scene/appearance/deformation/animation roots and terminal slot need five.
    assert_eq!(empty.terminal_attribution_slots(), 5);

    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let populated_input = empty_scene_input(Presence::Present(&roots), &nodes);
    let populated_nodes =
        SceneInputEnvelope::try_new(populated_input.batch(), SceneValidationLimits::default())
            .expect("the populated node shape is representable");
    assert_eq!(
        populated_nodes.terminal_attribution_slots(),
        empty.terminal_attribution_slots()
    );
}

#[test]
fn scene_envelope_rejects_top_level_input_limit_overflow() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let node_limit_input = empty_scene_input(Presence::Present(&roots), &nodes);

    assert!(matches!(
        SceneInputEnvelope::try_new(
            node_limit_input.batch(),
            SceneValidationLimits::default().with_max_nodes(1),
        ),
        Err(SceneInputEnvelopeError::LimitExceeded {
            limit: "nodes",
            actual: 2,
            maximum: 1,
        })
    ));
    let entity_limit_input = empty_scene_input(Presence::Present(&roots), &nodes);
    assert!(matches!(
        SceneInputEnvelope::try_new(
            entity_limit_input.batch(),
            SceneValidationLimits::default().with_max_entities(1),
        ),
        Err(SceneInputEnvelopeError::LimitExceeded {
            limit: "entity lower bound",
            actual: 5,
            maximum: 1,
        })
    ));
}

#[test]
fn scene_envelope_reserves_nested_worst_case_without_deep_validation() {
    let channels = [AnimationChannelAuthoring::new(
        0,
        AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(99)),
        AnimationInterpolation::CubicSpline,
        AnimationExtrapolation::SourceDefault,
        AnimationExtrapolation::SourceDefault,
        &[],
        &[],
        Presence::Absent,
        Presence::Absent,
    )];
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let nodes = NodeAuthoringBatch::new(Presence::Absent, &[]);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::empty();
    let animations = AnimationAuthoringBatch::new(&[], &channels);
    let profile = profile_fixture(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animations,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animations,
        profile.batch(),
    );
    let limits = SceneValidationLimits::default()
        .with_max_nodes(1)
        .with_max_entities(5)
        .with_max_payload_cells(0)
        .with_max_work(0)
        .with_max_scratch_bytes(0);

    let envelope = SceneInputEnvelope::try_new(input, limits)
        .expect("the envelope must not inspect invalid nested animation semantics");

    // One channel conservatively admits its table and all four possible payload blocks.
    assert_eq!(envelope.terminal_attribution_slots(), 10);
}

#[test]
fn scene_envelope_reserves_complete_appearance_payload_envelope() {
    let role = TextureRole::BaseColor;
    let coordinate_source =
        TextureCoordinateSource::uv_set(0).expect("test UV set should be valid");
    let channel_mapping = TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA);
    let color_interpretation = ColorInterpretation::SrgbColor;
    let blend = TextureBlendOperation::Replace;
    let binding_semantics = TextureBindingSemanticsRef::new(
        &role,
        &coordinate_source,
        &channel_mapping,
        &color_interpretation,
        TextureTransform::IDENTITY,
        TextureStrength::ONE,
        &blend,
        None,
    );
    let shading = ShadingModel::Unlit;
    let alpha = AlphaMode::Opaque;
    let material_semantics =
        MaterialSemanticsRef::new(&shading, &alpha, false, MaterialFactors::empty());
    let bindings = [MaterialBindingAuthoring::new(
        TextureLocalId::new(0),
        binding_semantics,
    )];
    let materials = [MaterialAuthoring::new(
        AuthoringRange::new(0, 1),
        material_semantics,
    )];
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let nodes = NodeAuthoringBatch::new(Presence::Absent, &[]);
    let appearance = AppearanceAuthoringBatch::new(&[], &[], &[], &bindings, &materials);
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::empty();
    let animations = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animations,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animations,
        profile.batch(),
    );

    let envelope = SceneInputEnvelope::try_new(input, SceneValidationLimits::default())
        .expect("appearance envelope should be representable without deep validation");

    // Binding semantics, optional source keys, material factors, and material names each own an
    // independently attributable aggregate payload block.
    assert_eq!(envelope.terminal_attribution_slots(), 11);
}

#[test]
fn scene_aggregate_accepts_exact_limits_and_rejects_each_one_short() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let defaults = SceneValidationLimits::default();
    let spatial_input = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let node_input = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance_input = AppearanceAuthoringBatch::empty();
    let geometry_input = GeometryAuthoringBatch::empty();
    let deformation_input = DeformationAuthoringBatch::empty();
    let animation_input = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial_input,
        node_input,
        appearance_input,
        geometry_input,
        deformation_input,
        animation_input,
    );

    let spatial_batch =
        validate_spatial_batch_with_control(spatial_input, node_input, defaults, &mut control)
            .expect("empty spatial domain validates independently");
    let node_batch = validate(Presence::Present(&roots), &nodes, defaults, &mut control)
        .expect("node domain validates independently");
    let appearance_batch = validate_appearance_batch_with_control(
        appearance_input,
        super::super::ResourceSnapshotAuthoringTable::empty(),
        defaults,
        &mut control,
    )
    .expect("empty appearance domain validates independently");
    let geometry_batch = validate_geometry_batch_with_control(
        geometry_input,
        nodes.len(),
        appearance_batch.material_count(),
        defaults,
        &mut control,
    )
    .expect("empty geometry domain validates independently");
    let deformation_batch = validate_deformation_batch_with_control(
        deformation_input,
        &node_batch,
        &geometry_batch,
        defaults,
        &mut control,
    )
    .expect("empty deformation domain validates independently");
    let animation_targets = AnimationTargetCatalog::new(&node_batch, &deformation_batch);
    let animation_batch = validate_animation_batch_with_control(
        animation_input,
        &animation_targets,
        defaults,
        &mut control,
    )
    .expect("empty animation domain validates independently");
    let profile_owners = ProfileOwnerCatalog::from_validated(
        &spatial_batch,
        &node_batch,
        &appearance_batch,
        &geometry_batch,
        &deformation_batch,
        &animation_batch,
    )
    .expect("profile owner catalog is representable");
    let profile_batch = validate_profile_batch_with_control(
        profile.batch(),
        profile_owners,
        defaults,
        &mut control,
    )
    .expect("profile domain validates independently");

    let entities = checked_total([
        spatial_batch.entity_count(),
        node_batch.canonical_node_count(),
        appearance_batch.entity_count(),
        geometry_batch.entity_count(),
        deformation_batch.entity_count(),
        animation_batch.entity_count(),
        profile_batch.entity_count(),
    ]);
    let edges = checked_total([
        spatial_batch.edge_count(),
        node_batch.reference_count(),
        appearance_batch.edge_count(),
        geometry_batch.edge_count(),
        deformation_batch.edge_count(),
        animation_batch.edge_count(),
        profile_batch.edge_count(),
    ]);
    let references = checked_total([
        spatial_batch.reference_count(),
        node_batch.reference_count(),
        appearance_batch.reference_count(),
        geometry_batch.reference_count(),
        deformation_batch.reference_count(),
        animation_batch.reference_count(),
        profile_batch.reference_count(),
    ]);
    let semantic_cells = checked_total([
        spatial_batch.semantic_cell_count(),
        node_batch.semantic_cell_count(),
        appearance_batch.semantic_cell_count(),
        geometry_batch.semantic_cell_count(),
        deformation_batch.semantic_cell_count(),
        animation_batch.semantic_cell_count(),
        profile_batch.payload_cell_count(),
    ]);
    let work = checked_total([
        spatial_batch.validation_work(),
        node_batch.validation_work(),
        appearance_batch.validation_work(),
        geometry_batch.validation_work(),
        deformation_batch.validation_work(),
        animation_batch.validation_work(),
        profile_batch.validation_work(),
    ]);
    let canonical_bytes = checked_total([
        crate::canonical_scene::store::schema::SCENE_DIRECTORY_CANONICAL_BYTES,
        spatial_batch.canonical_bytes(),
        node_batch.canonical_bytes(),
        appearance_batch.canonical_bytes(),
        geometry_batch.canonical_bytes(),
        deformation_batch.canonical_bytes(),
        animation_batch.canonical_bytes(),
        profile_batch.canonical_bytes(),
    ]);
    let scratch_bytes = checked_total([
        spatial_batch.scratch_bytes(),
        node_batch.scratch_bytes(),
        appearance_batch.scratch_bytes(),
        geometry_batch.scratch_bytes(),
        deformation_batch.scratch_bytes(),
        animation_batch.scratch_bytes(),
        profile_batch.scratch_bytes(),
    ]);
    let exact = defaults
        .with_max_entities(entities)
        .with_max_edges(edges)
        .with_max_references(references)
        .with_max_payload_cells(semantic_cells)
        .with_max_work(work)
        .with_max_canonical_bytes(canonical_bytes)
        .with_max_scratch_bytes(scratch_bytes);

    let node_limits = node_limits_after_spatial(&spatial_batch, exact)
        .expect("spatial metrics fit the exact Scene limits");
    let appearance_limits = appearance_limits_after_nodes(&node_batch, node_limits)
        .expect("node metrics fit the exact Scene limits");
    let geometry_limits = geometry_limits_after_appearance(&appearance_batch, appearance_limits)
        .expect("appearance metrics fit the remaining Scene limits");
    let deformation_limits = deformation_limits_after_geometry(&geometry_batch, geometry_limits)
        .expect("geometry metrics fit the remaining Scene limits");
    let animation_limits =
        animation_limits_after_deformation(&deformation_batch, deformation_limits)
            .expect("deformation metrics fit the remaining Scene limits");
    let profile_limits = profile_limits_after_animation(&animation_batch, animation_limits)
        .expect("animation metrics fit the remaining Scene limits");
    assert_eq!(profile_limits.max_entities(), profile_batch.entity_count());
    assert_eq!(profile_limits.max_edges(), profile_batch.edge_count());
    assert_eq!(
        profile_limits.max_references(),
        profile_batch.reference_count()
    );
    assert_eq!(
        profile_limits.max_payload_cells(),
        profile_batch.payload_cell_count()
    );
    assert_eq!(profile_limits.max_work(), profile_batch.validation_work());
    assert_eq!(
        profile_limits.max_canonical_bytes(),
        profile_batch.canonical_bytes()
    );
    assert_eq!(
        profile_limits.max_scratch_bytes(),
        profile_batch.scratch_bytes()
    );

    assert!(
        validate_scene_aggregate(
            &spatial_batch,
            &node_batch,
            &appearance_batch,
            &geometry_batch,
            &deformation_batch,
            &animation_batch,
            &profile_batch,
            exact,
        )
        .is_ok()
    );

    for (limit, one_short) in [
        (
            SceneAuthoringAggregateLimit::Entities,
            exact.with_max_entities(entities - 1),
        ),
        (
            SceneAuthoringAggregateLimit::Edges,
            exact.with_max_edges(edges - 1),
        ),
        (
            SceneAuthoringAggregateLimit::References,
            exact.with_max_references(references - 1),
        ),
        (
            SceneAuthoringAggregateLimit::SemanticCells,
            exact.with_max_payload_cells(semantic_cells - 1),
        ),
        (
            SceneAuthoringAggregateLimit::Work,
            exact.with_max_work(work - 1),
        ),
        (
            SceneAuthoringAggregateLimit::CanonicalBytes,
            exact.with_max_canonical_bytes(canonical_bytes - 1),
        ),
        (
            SceneAuthoringAggregateLimit::ScratchBytes,
            exact.with_max_scratch_bytes(scratch_bytes - 1),
        ),
    ] {
        assert!(matches!(
            validate_scene_aggregate(
                &spatial_batch,
                &node_batch,
                &appearance_batch,
                &geometry_batch,
                &deformation_batch,
                &animation_batch,
                &profile_batch,
                one_short,
            ),
            Err(SceneAuthoringAggregateError::LimitExceeded {
                limit: actual,
                ..
            }) if actual == limit
        ));
    }

    drop(profile_batch);
    drop(animation_batch);
    drop(deformation_batch);
    drop(geometry_batch);
    drop(appearance_batch);
    drop(node_batch);
    drop(spatial_batch);
    assert_eq!(control.live.get(), 0);
}
