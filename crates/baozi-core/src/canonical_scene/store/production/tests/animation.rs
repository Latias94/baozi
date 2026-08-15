use std::cell::Cell;
use std::rc::Rc;

use super::super::plan::animation::{
    AnimationTargetCatalog, validate_animation_batch_with_control,
};
use super::super::plan::deformation::validate_deformation_batch_with_control;
use super::super::plan::geometry::validate_geometry_batch_with_control;
use super::super::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolationAuthoring, AnimationInterpolationAuthoring, AnimationTargetAuthoring,
    AnimationTimeDomainAuthoring, AuthoringRange, DeformationAuthoringBatch,
    GeometryAuthoringBatch, NodeLocalId, Presence,
};
use super::{SceneValidationLimits, TestControl, trs, validate};

fn validate_translation<'input>(
    input: AnimationAuthoringBatch<'input>,
    control: &mut TestControl,
) -> super::super::ValidatedAnimationBatch<'input, super::TestReservation> {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut setup_control = TestControl::default();
    let validated_nodes = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("the animation fixture node must validate");
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        nodes.len(),
        0,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty geometry must validate for the animation fixture");
    let deformation = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::empty(),
        &validated_nodes,
        &geometry,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty deformation must validate for the animation fixture");
    let targets = AnimationTargetCatalog::new(&validated_nodes, &deformation);

    validate_animation_batch_with_control(
        input,
        &targets,
        SceneValidationLimits::default(),
        control,
    )
    .expect("the translation animation fixture must validate")
}

#[test]
fn compiled_animation_facts_do_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let times = vec![0.0_f64, 1.0];
        let values = vec![0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0];
        let channels = vec![AnimationChannelAuthoring::new(
            AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
            AnimationInterpolationAuthoring::Linear,
            AnimationExtrapolationAuthoring::SourceDefault,
            AnimationExtrapolationAuthoring::SourceDefault,
            &times,
            &values,
            Presence::Absent,
            Presence::Absent,
        )];
        let clips = vec![AnimationClipAuthoring::new(
            1.0,
            AnimationTimeDomainAuthoring::Seconds,
            AuthoringRange::new(0, 1),
        )];

        validate_translation(
            AnimationAuthoringBatch::new(&clips, &channels),
            &mut control,
        )
        .into_compiled_facts_for_test()
    };

    assert_eq!(compiled.0, [1, 1]);
    assert_eq!(compiled.1, [1, 1]);

    drop(compiled);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn compiled_animation_storage_drops_before_its_reservation() {
    let times = [0.0_f64, 1.0];
    let values = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let channels = [AnimationChannelAuthoring::new(
        AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
        AnimationInterpolationAuthoring::Linear,
        AnimationExtrapolationAuthoring::SourceDefault,
        AnimationExtrapolationAuthoring::SourceDefault,
        &times,
        &values,
        Presence::Absent,
        Presence::Absent,
    )];
    let clips = [AnimationClipAuthoring::new(
        1.0,
        AnimationTimeDomainAuthoring::Seconds,
        AuthoringRange::new(0, 1),
    )];
    let stage = Rc::new(Cell::new(0));
    let mut control = TestControl {
        reservation_drop_stage: Some(Rc::clone(&stage)),
        ..TestControl::default()
    };
    let mut validated = validate_translation(
        AnimationAuthoringBatch::new(&clips, &channels),
        &mut control,
    );

    validated.install_storage_drop_probe(Rc::clone(&stage));
    drop(validated);

    assert_eq!(stage.get(), 2);
    assert_eq!(control.live.get(), 0);
}
