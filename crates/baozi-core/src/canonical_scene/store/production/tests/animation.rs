use std::cell::Cell;
use std::rc::Rc;

use super::super::plan::animation::{
    AnimationAuthoringFailure, AnimationAuthoringSemanticError, AnimationTargetCatalog,
    validate_animation_batch_with_control,
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

fn reject_translation(
    input: AnimationAuthoringBatch<'_>,
    control: &mut TestControl,
) -> AnimationAuthoringSemanticError {
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

    match validate_animation_batch_with_control(
        input,
        &targets,
        SceneValidationLimits::default(),
        control,
    ) {
        Err(AnimationAuthoringFailure::Semantic(error)) => error,
        Err(AnimationAuthoringFailure::Control(error)) => {
            panic!("unexpected animation control failure: {error}")
        }
        Ok(_) => panic!("expected animation source-index validation to fail"),
    }
}

#[test]
fn compiled_animation_facts_do_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let name = String::from("步行");
        let times = vec![0.0_f64, 1.0];
        let values = vec![0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0];
        let channels = vec![AnimationChannelAuthoring::new(
            0,
            AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
            AnimationInterpolationAuthoring::Linear,
            AnimationExtrapolationAuthoring::SourceDefault,
            AnimationExtrapolationAuthoring::SourceDefault,
            &times,
            &values,
            Presence::Absent,
            Presence::Absent,
        )];
        let clips = vec![
            AnimationClipAuthoring::new(
                0,
                1.0,
                AnimationTimeDomainAuthoring::Seconds,
                AuthoringRange::new(0, 1),
            )
            .with_name(name.as_str()),
        ];

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
fn borrowed_clip_names_preserve_presence_utf8_and_metered_bytes_without_copying() {
    let unicode = String::from("步行 α");
    let maximum = "x".repeat(crate::entity::MAX_ENTITY_NAME_BYTES);
    let mut absent_work = 0;
    for name in [
        None,
        Some(""),
        Some(unicode.as_str()),
        Some(maximum.as_str()),
    ] {
        let mut clips = [AnimationClipAuthoring::new(
            0,
            0.0,
            AnimationTimeDomainAuthoring::Seconds,
            AuthoringRange::new(0, 0),
        )];
        let allocations = allocation_counter::measure(|| {
            if let Some(name) = name {
                clips[0] = clips[0].with_name(name);
            }
        });
        assert_eq!(allocations.count_total, 0);

        let mut control = TestControl::default();
        let validated =
            validate_translation(AnimationAuthoringBatch::new(&clips, &[]), &mut control);
        let byte_count = name.map_or(0, str::len);
        assert_eq!(validated.clip_name_byte_count() as usize, byte_count);
        if name.is_none() {
            absent_work = validated.validation_work();
        }
        assert_eq!(validated.validation_work() - absent_work, byte_count as u64);
        let allocations = allocation_counter::measure(|| {
            let clip = validated.clip(0).unwrap();
            assert_eq!(clip.name_count() as usize, byte_count);
            match (name, clip.name()) {
                (None, Presence::Absent) => {}
                (Some(expected), Presence::Present(actual)) => {
                    assert_eq!(actual, expected);
                    assert!(std::ptr::eq(actual, expected));
                }
                _ => panic!("a borrowed clip name must retain authored presence"),
            }
        });
        assert_eq!(allocations.count_total, 0);
        drop(validated);
        assert_eq!(control.live.get(), 0);
        assert_eq!(control.current_bytes.get(), 0);
    }
}

#[test]
fn borrowed_clip_names_reject_control_characters_and_oversize_with_exact_errors() {
    use crate::entity::{EntityNameError, MAX_ENTITY_NAME_BYTES};

    let oversized = "x".repeat(MAX_ENTITY_NAME_BYTES + 1);
    for (name, expected) in [
        ("步\n", EntityNameError::ControlCharacter { offset: 3 }),
        (
            oversized.as_str(),
            EntityNameError::TooLong {
                actual: MAX_ENTITY_NAME_BYTES + 1,
                maximum: MAX_ENTITY_NAME_BYTES,
            },
        ),
    ] {
        let clips = [AnimationClipAuthoring::new(
            0,
            0.0,
            AnimationTimeDomainAuthoring::Seconds,
            AuthoringRange::new(0, 0),
        )
        .with_name(name)];
        let mut control = TestControl::default();
        match reject_translation(AnimationAuthoringBatch::new(&clips, &[]), &mut control) {
            AnimationAuthoringSemanticError::InvalidClipName { clip: 0, source } => {
                assert_eq!(source, expected);
            }
            error => panic!("expected the exact borrowed clip-name error, got {error:?}"),
        }
        assert_eq!(
            control.last_work(),
            Some(crate::SceneValidationWork::Entity)
        );
        assert_eq!(control.live.get(), 0);
        assert_eq!(control.current_bytes.get(), 0);
    }
}

#[test]
fn compiled_animation_storage_drops_before_its_reservation() {
    let times = [0.0_f64, 1.0];
    let values = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let channels = [AnimationChannelAuthoring::new(
        0,
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
        0,
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

#[test]
fn clip_source_indexes_must_be_strictly_increasing() {
    for actual in [3, 2] {
        let clips = [
            AnimationClipAuthoring::new(
                3,
                0.0,
                AnimationTimeDomainAuthoring::Seconds,
                AuthoringRange::new(0, 0),
            ),
            AnimationClipAuthoring::new(
                actual,
                0.0,
                AnimationTimeDomainAuthoring::Seconds,
                AuthoringRange::new(0, 0),
            ),
        ];
        let mut control = TestControl::default();
        let error = reject_translation(AnimationAuthoringBatch::new(&clips, &[]), &mut control);

        assert!(matches!(
            error,
            AnimationAuthoringSemanticError::ClipSourceIndexOrder {
                clip: 1,
                previous: 3,
                actual: rejected,
            } if rejected == actual
        ));
        assert_eq!(
            control.last_work(),
            Some(crate::SceneValidationWork::Entity)
        );
        assert_eq!(control.live.get(), 0);
        assert_eq!(control.current_bytes.get(), 0);
    }
}

#[test]
fn channel_source_indexes_must_be_strictly_increasing() {
    let times = [0.0_f64, 1.0];
    let values = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0];

    for actual in [5, 4] {
        let channels = [
            AnimationChannelAuthoring::new(
                5,
                AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
                AnimationInterpolationAuthoring::Linear,
                AnimationExtrapolationAuthoring::SourceDefault,
                AnimationExtrapolationAuthoring::SourceDefault,
                &times,
                &values,
                Presence::Absent,
                Presence::Absent,
            ),
            AnimationChannelAuthoring::new(
                actual,
                AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
                AnimationInterpolationAuthoring::Linear,
                AnimationExtrapolationAuthoring::SourceDefault,
                AnimationExtrapolationAuthoring::SourceDefault,
                &times,
                &values,
                Presence::Absent,
                Presence::Absent,
            ),
        ];
        let clips = [AnimationClipAuthoring::new(
            3,
            1.0,
            AnimationTimeDomainAuthoring::Seconds,
            AuthoringRange::new(0, 2),
        )];
        let mut control = TestControl::default();
        let error = reject_translation(
            AnimationAuthoringBatch::new(&clips, &channels),
            &mut control,
        );

        assert!(matches!(
            error,
            AnimationAuthoringSemanticError::ChannelSourceIndexOrder {
                channel: 1,
                previous: 5,
                actual: rejected,
            } if rejected == actual
        ));
        assert_eq!(
            control.last_work(),
            Some(crate::SceneValidationWork::Entity)
        );
        assert_eq!(control.live.get(), 0);
        assert_eq!(control.current_bytes.get(), 0);
    }
}
