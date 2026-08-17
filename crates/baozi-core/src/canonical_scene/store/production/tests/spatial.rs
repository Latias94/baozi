use crate::SceneValidationLimits;
use crate::canonical_scene::store::production::plan::spatial::{
    SpatialAuthoringFailure, SpatialAuthoringSemanticError, validate_spatial_batch_with_control,
};
use crate::canonical_scene::store::production::scene::node_limits_after_spatial;
use crate::canonical_scene::store::production::{
    CameraAuthoring, CameraLocalId, LightAuthoring, LightLocalId, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, NodeParent, Presence, SpatialAuthoringBatch,
};
use crate::entity::{AuthoredTransform, CameraProjection, LightEmission, LightKind};
use crate::math::Color;
use crate::scene_space::SceneSpace;

use super::TestControl;

fn camera() -> CameraAuthoring {
    CameraAuthoring::new(
        CameraProjection::perspective(1.0, Some(16.0 / 9.0), 0.1, Some(100.0))
            .expect("test perspective projection"),
    )
}

fn emission() -> LightEmission {
    LightEmission::try_from_scaled_linear_srgb(Color::WHITE, 10.0)
        .expect("test canonical light emission")
}

fn node(
    parent: Presence<NodeParent>,
    camera: Presence<CameraLocalId>,
    light: Presence<LightLocalId>,
) -> NodeAuthoringNode<'static> {
    NodeAuthoringNode::new(parent, AuthoredTransform::identity_trs(), camera, light)
}

fn semantic_error(
    result: Result<
        super::super::plan::spatial::ValidatedSpatialBatch<'_, super::TestReservation>,
        SpatialAuthoringFailure<super::TestControlError>,
    >,
) -> SpatialAuthoringSemanticError {
    match result {
        Err(SpatialAuthoringFailure::Semantic(error)) => error,
        Err(SpatialAuthoringFailure::Control(error)) => {
            panic!("expected spatial semantic error, got control error {error:?}")
        }
        Ok(_) => panic!("expected spatial authoring rejection"),
    }
}

#[test]
fn multiple_node_placements_are_preserved_for_each_spatial_object() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [
        node(
            Presence::Absent,
            Presence::Present(CameraLocalId::new(0)),
            Presence::Present(LightLocalId::new(0)),
        ),
        node(
            Presence::Present(NodeParent::new(NodeLocalId::new(0), 0)),
            Presence::Present(CameraLocalId::new(0)),
            Presence::Present(LightLocalId::new(0)),
        ),
    ];
    let cameras = [camera()];
    let lights = [LightAuthoring::new(
        LightKind::point(Some(25.0)).expect("test point light"),
        emission(),
    )];
    let mut control = TestControl::default();

    let validated = validate_spatial_batch_with_control(
        SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &cameras, &lights),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("multiple placements are valid canonical spatial semantics");

    assert_eq!(validated.camera_count(), 1);
    assert_eq!(validated.light_count(), 1);
    assert_eq!(validated.edge_count(), 4);
    assert_eq!(validated.reference_count(), 4);
}

#[test]
fn compiled_spatial_facts_do_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let roots = vec![NodeLocalId::new(0)];
        let nodes = vec![node(
            Presence::Absent,
            Presence::Present(CameraLocalId::new(0)),
            Presence::Absent,
        )];
        let cameras = vec![camera()];

        validate_spatial_batch_with_control(
            SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &cameras, &[]),
            NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("temporary authored storage must compile into owned spatial facts")
        .into_compiled_facts_for_test()
    };

    assert_eq!(compiled.0, 1);
    assert_eq!(compiled.1, 0);
    assert_eq!(compiled.2, 1);
    assert_eq!(compiled.3, 1);

    drop(compiled);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn transient_spatial_scratch_does_not_reduce_later_node_scratch() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [node(
        Presence::Absent,
        Presence::Present(CameraLocalId::new(0)),
        Presence::Absent,
    )];
    let cameras = [camera()];
    let defaults = SceneValidationLimits::default();

    let mut baseline_spatial_control = TestControl::default();
    let baseline_spatial = validate_spatial_batch_with_control(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &cameras, &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        defaults,
        &mut baseline_spatial_control,
    )
    .expect("camera placement scratch should validate under default limits");
    let spatial_peak = baseline_spatial.peak_scratch_bytes();
    assert!(spatial_peak > 0);
    assert_eq!(baseline_spatial_control.peak_bytes(), spatial_peak);
    assert_eq!(baseline_spatial.scratch_bytes(), 0);
    drop(baseline_spatial);
    assert_eq!(baseline_spatial_control.live.get(), 0);

    let mut baseline_node_control = TestControl::default();
    let baseline_nodes = super::validate(
        Presence::Present(&roots),
        &nodes,
        defaults,
        &mut baseline_node_control,
    )
    .expect("node scratch should validate under default limits");
    let node_retained = baseline_nodes.scratch_bytes();
    let node_peak = baseline_node_control.peak_bytes();
    assert!(node_retained > 0);
    assert!(node_peak >= node_retained);

    let shared_scratch_limit = spatial_peak.max(node_peak);
    let sequential_sum = spatial_peak
        .checked_add(node_peak)
        .expect("test scratch total should be representable");
    assert!(shared_scratch_limit < sequential_sum);

    drop(baseline_nodes);
    assert_eq!(baseline_node_control.live.get(), 0);

    let limits = defaults.with_max_scratch_bytes(shared_scratch_limit);
    let mut control = TestControl::default();
    let spatial = validate_spatial_batch_with_control(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &cameras, &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        limits,
        &mut control,
    )
    .expect("the shared limit should admit the spatial transient peak");
    assert_eq!(spatial.peak_scratch_bytes(), spatial_peak);
    assert_eq!(spatial.scratch_bytes(), 0);

    let node_limits = node_limits_after_spatial(&spatial, limits)
        .expect("released spatial scratch should not consume the Scene scratch limit");
    assert_eq!(node_limits.max_scratch_bytes(), shared_scratch_limit);
    let validated_nodes =
        super::validate(Presence::Present(&roots), &nodes, node_limits, &mut control)
            .expect("the same shared limit should admit the node validation peak");
    assert_eq!(validated_nodes.scratch_bytes(), node_retained);
    assert_eq!(control.peak_bytes(), shared_scratch_limit);

    drop(validated_nodes);
    drop(spatial);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn orphan_camera_is_rejected_before_materialization() {
    let cameras = [camera()];
    let mut control = TestControl::default();
    let error = semantic_error(validate_spatial_batch_with_control(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &cameras, &[]),
        NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
        SceneValidationLimits::default(),
        &mut control,
    ));

    assert!(matches!(
        error,
        SpatialAuthoringSemanticError::OrphanCamera { camera: 0 }
    ));
}

#[test]
fn orphan_light_is_rejected_before_materialization() {
    let lights = [LightAuthoring::new(LightKind::directional(), emission())];
    let mut control = TestControl::default();
    let error = semantic_error(validate_spatial_batch_with_control(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &lights),
        NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
        SceneValidationLimits::default(),
        &mut control,
    ));

    assert!(matches!(
        error,
        SpatialAuthoringSemanticError::OrphanLight { light: 0 }
    ));
}

#[test]
fn out_of_range_camera_attachment_is_rejected() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [node(
        Presence::Absent,
        Presence::Present(CameraLocalId::new(1)),
        Presence::Absent,
    )];
    let cameras = [camera()];
    let mut control = TestControl::default();
    let error = semantic_error(validate_spatial_batch_with_control(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &cameras, &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        SceneValidationLimits::default(),
        &mut control,
    ));

    assert!(matches!(
        error,
        SpatialAuthoringSemanticError::CameraOutOfRange {
            node: 0,
            camera: 1,
            camera_count: 1,
        }
    ));
}

#[test]
fn out_of_range_light_attachment_is_rejected() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [node(
        Presence::Absent,
        Presence::Absent,
        Presence::Present(LightLocalId::new(1)),
    )];
    let lights = [LightAuthoring::new(LightKind::directional(), emission())];
    let mut control = TestControl::default();
    let error = semantic_error(validate_spatial_batch_with_control(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &lights),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        SceneValidationLimits::default(),
        &mut control,
    ));

    assert!(matches!(
        error,
        SpatialAuthoringSemanticError::LightOutOfRange {
            node: 0,
            light: 1,
            light_count: 1,
        }
    ));
}
