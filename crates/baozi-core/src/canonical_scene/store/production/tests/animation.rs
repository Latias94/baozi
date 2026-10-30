use std::cell::Cell;
use std::rc::Rc;

use crate::animation::{AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain};
use crate::canonical_scene::store::typed_storage::animation::build::AnimationStoragePlan;
use crate::canonical_scene::store::typed_storage::animation::{
    AnimationStorage, StoredAnimationChannel, StoredAnimationClip,
};

use super::super::plan::animation::{
    AnimationAuthoringFailure, AnimationAuthoringLimit, AnimationAuthoringSemanticError,
    AnimationTargetCatalog, ValidatedAnimationBatch, validate_animation_batch_with_control,
};
use super::super::plan::deformation::validate_deformation_batch_with_control;
use super::super::plan::geometry::validate_geometry_batch_with_control;
use super::super::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationTargetAuthoring, AuthoringRange, DeformationAuthoringBatch, GeometryAuthoringBatch,
    NodeLocalId, Presence,
};
use super::{SceneValidationLimits, TestControl, TestControlError, TestReservation, trs, validate};

fn validate_translation<'input>(
    input: AnimationAuthoringBatch<'input>,
    control: &mut TestControl,
) -> ValidatedAnimationBatch<'input, TestReservation> {
    validate_input(input, SceneValidationLimits::default(), control)
        .expect("the translation animation fixture must validate")
}

fn reject_translation(
    input: AnimationAuthoringBatch<'_>,
    control: &mut TestControl,
) -> AnimationAuthoringSemanticError {
    match validate_input(input, SceneValidationLimits::default(), control) {
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
            AnimationInterpolation::Linear,
            AnimationExtrapolation::SourceDefault,
            AnimationExtrapolation::SourceDefault,
            &times,
            &values,
            Presence::Absent,
            Presence::Absent,
        )];
        let clips = vec![
            AnimationClipAuthoring::new(
                0,
                1.0,
                AnimationTimeDomain::Seconds,
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
    assert_eq!(compiled.1, 1);

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
            AnimationTimeDomain::Seconds,
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
            AnimationTimeDomain::Seconds,
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
        AnimationInterpolation::Linear,
        AnimationExtrapolation::SourceDefault,
        AnimationExtrapolation::SourceDefault,
        &times,
        &values,
        Presence::Absent,
        Presence::Absent,
    )];
    let clips = [AnimationClipAuthoring::new(
        0,
        1.0,
        AnimationTimeDomain::Seconds,
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
                AnimationTimeDomain::Seconds,
                AuthoringRange::new(0, 0),
            ),
            AnimationClipAuthoring::new(
                actual,
                0.0,
                AnimationTimeDomain::Seconds,
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
                AnimationInterpolation::Linear,
                AnimationExtrapolation::SourceDefault,
                AnimationExtrapolation::SourceDefault,
                &times,
                &values,
                Presence::Absent,
                Presence::Absent,
            ),
            AnimationChannelAuthoring::new(
                actual,
                AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
                AnimationInterpolation::Linear,
                AnimationExtrapolation::SourceDefault,
                AnimationExtrapolation::SourceDefault,
                &times,
                &values,
                Presence::Absent,
                Presence::Absent,
            ),
        ];
        let clips = [AnimationClipAuthoring::new(
            3,
            1.0,
            AnimationTimeDomain::Seconds,
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

fn validate_input<'input>(
    input: AnimationAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    control: &mut TestControl,
) -> Result<
    ValidatedAnimationBatch<'input, TestReservation>,
    AnimationAuthoringFailure<TestControlError>,
> {
    let mut setup = TestControl::default();
    let roots = [NodeLocalId::new(0)];
    let source_nodes = [trs(Presence::Absent)];
    let nodes = validate(
        Presence::Present(&roots),
        &source_nodes,
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        1,
        0,
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let deformation = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::empty(),
        &nodes,
        &geometry,
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let targets = AnimationTargetCatalog::new(&nodes, &deformation);
    validate_animation_batch_with_control(input, &targets, limits, control)
}

fn expected_bytes(rows: [usize; 2], payloads: [usize; 5]) -> u64 {
    (std::mem::size_of::<AnimationStorage>()
        + rows[0] * std::mem::size_of::<StoredAnimationClip>()
        + rows[1] * std::mem::size_of::<StoredAnimationChannel>()
        + payloads[0] * std::mem::size_of::<f64>()
        + (payloads[1] + payloads[2] + payloads[3]) * std::mem::size_of::<f32>()
        + payloads[4]) as u64
}

#[test]
fn initialized_owner_header_is_charged_before_scratch_or_payload_work() {
    let expected = expected_bytes([0; 2], [0; 5]);
    let limits = SceneValidationLimits::default().with_max_canonical_bytes(expected);
    let mut control = TestControl::default();
    let batch = validate_input(AnimationAuthoringBatch::empty(), limits, &mut control).unwrap();
    assert_eq!(batch.canonical_bytes(), expected);
    assert_eq!(batch.semantic_cell_count(), 0);
    assert_eq!(batch.scratch_bytes(), 0);
    assert_eq!(
        AnimationStoragePlan::new(&batch)
            .unwrap()
            .canonical_bytes()
            .unwrap(),
        expected
    );
    drop(batch);
    assert_eq!(control.live.get(), 0);
    assert_eq!(control.current_bytes.get(), 0);

    let mut control = TestControl::default();
    assert!(matches!(
        validate_input(AnimationAuthoringBatch::empty(), limits.with_max_canonical_bytes(expected - 1), &mut control),
        Err(AnimationAuthoringFailure::Semantic(
            AnimationAuthoringSemanticError::LimitExceeded {
                limit: AnimationAuthoringLimit::CanonicalBytes, actual, maximum,
            }
        )) if actual == expected && maximum == expected - 1
    ));
    assert_eq!(control.work, 0);
    assert_eq!(control.reserves, 0);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn inline_rates_and_name_presence_do_not_change_rows_or_retain_clip_scratch() {
    let domains = [
        AnimationTimeDomain::Seconds,
        AnimationTimeDomain::Frames { rate: 23.976 },
        AnimationTimeDomain::Ticks { rate: None },
        AnimationTimeDomain::Ticks { rate: Some(59.94) },
    ];
    let mut nameless_work = None;
    let mut nameless_scratch = None;
    for domain in domains {
        for name in [None, Some(""), Some("步行 α")] {
            let mut clip = AnimationClipAuthoring::new(7, -0.0, domain, AuthoringRange::new(0, 0));
            if let Some(name) = name {
                clip = clip.with_name(name);
            }
            let clips = [clip];
            let bytes = name.map_or(0, str::len);
            let expected = expected_bytes([1, 0], [0, 0, 0, 0, bytes]);
            let mut control = TestControl::default();
            let batch = validate_input(
                AnimationAuthoringBatch::new(&clips, &[]),
                SceneValidationLimits::default().with_max_canonical_bytes(expected),
                &mut control,
            )
            .unwrap();
            assert_eq!(batch.canonical_bytes(), expected);
            assert_eq!(batch.clip_name_byte_count(), bytes as u64);
            assert_eq!(batch.semantic_cell_count(), 0);
            assert_eq!(batch.entity_count(), 1);
            assert_eq!(
                batch.scratch_bytes(),
                0,
                "clips have no compiled descriptor allocation"
            );
            let work = *nameless_work.get_or_insert(batch.validation_work());
            let scratch = *nameless_scratch.get_or_insert(control.peak_bytes());
            assert_eq!(batch.validation_work(), work + bytes as u64);
            assert_eq!(control.peak_bytes(), scratch);
            let actual = batch.clip(0).unwrap();
            assert_eq!(actual.duration().to_bits(), (-0.0_f64).to_bits());
            assert_eq!(actual.time_domain(), domain);
            assert_eq!(
                actual.name(),
                name.map_or(Presence::Absent, Presence::Present)
            );
            assert_eq!(actual.channels().len(), 0);
            drop(batch);
            assert_eq!(control.current_bytes.get(), 0);
        }
    }
}

#[test]
fn f64_times_and_cubic_derivatives_charge_the_actual_pools_at_exact_limits() {
    let times = [-3.5, -0.0, f64::from_bits(1.0_f64.to_bits() + 1)];
    let values = [0.0_f32, -0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let input_tangents = [-2.0_f32; 9];
    let output_tangents = [3.0_f32; 9];
    let channels = [AnimationChannelAuthoring::new(
        8,
        AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
        AnimationInterpolation::CubicSpline,
        AnimationExtrapolation::Linear,
        AnimationExtrapolation::Repeat,
        &times,
        &values,
        Presence::Present(&input_tangents),
        Presence::Present(&output_tangents),
    )];
    let clips = [AnimationClipAuthoring::new(
        2,
        1.5,
        AnimationTimeDomain::Ticks { rate: None },
        AuthoringRange::new(0, 1),
    )];
    let input = AnimationAuthoringBatch::new(&clips, &channels);
    let expected = expected_bytes([1, 1], [3, 9, 9, 9, 0]);
    let limits = SceneValidationLimits::default().with_max_canonical_bytes(expected);
    let mut control = TestControl::default();
    let batch = validate_input(input, limits, &mut control).unwrap();
    assert_eq!(batch.canonical_bytes(), expected);
    assert_eq!(batch.semantic_cell_count(), 30);
    assert_eq!(batch.edge_count(), 2);
    assert_eq!(batch.reference_count(), 2);
    assert_eq!(batch.validation_work(), control.work);
    let clip = batch.clip(0).unwrap();
    let channel = clip.channels().next().unwrap();
    assert_eq!(channel.canonical_target_index(), 1);
    assert_eq!(channel.value_width(), 3);
    assert_eq!(channel.times().as_ptr(), times.as_ptr());
    assert_eq!(channel.times()[0].to_bits(), (-3.5_f64).to_bits());
    assert_eq!(channel.times()[1].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(channel.times()[2].to_bits(), 1.0_f64.to_bits() + 1);
    assert_eq!(
        channel.in_tangents(),
        Presence::Present(input_tangents.as_slice())
    );
    assert_eq!(
        channel.out_tangents(),
        Presence::Present(output_tangents.as_slice())
    );
    drop(batch);
    assert_eq!(control.current_bytes.get(), 0);
    assert!(matches!(
        validate_input(input, limits.with_max_canonical_bytes(expected - 1), &mut control),
        Err(AnimationAuthoringFailure::Semantic(
            AnimationAuthoringSemanticError::LimitExceeded {
                limit: AnimationAuthoringLimit::CanonicalBytes, actual, maximum,
            }
        )) if actual == expected && maximum == expected - 1
    ));
    assert_eq!(control.current_bytes.get(), 0);
    assert!(matches!(
        validate_input(input, limits.with_max_payload_cells(29), &mut control),
        Err(AnimationAuthoringFailure::Semantic(
            AnimationAuthoringSemanticError::LimitExceeded {
                limit: AnimationAuthoringLimit::SemanticCells,
                actual: 30,
                maximum: 29,
            }
        ))
    ));
    assert_eq!(control.current_bytes.get(), 0);
}

#[test]
fn flat_targets_release_at_each_work_resize_and_poll_failure() {
    let times = [-2.0_f64, -0.0, 1.0];
    let rotation = [
        0.0_f32, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
    ];
    let derivatives = [3.0_f32; 12];
    let channels = [AnimationChannelAuthoring::new(
        1,
        AnimationTargetAuthoring::NodeRotation(NodeLocalId::new(0)),
        AnimationInterpolation::CubicSpline,
        AnimationExtrapolation::SourceDefault,
        AnimationExtrapolation::Constant,
        &times,
        &rotation,
        Presence::Present(&derivatives),
        Presence::Present(&derivatives),
    )];
    let clips = [AnimationClipAuthoring::new(
        1,
        3.0,
        AnimationTimeDomain::Seconds,
        AuthoringRange::new(0, 1),
    )
    .with_name("回転")];
    let input = AnimationAuthoringBatch::new(&clips, &channels);
    let limits = SceneValidationLimits::default();
    let mut baseline = TestControl::default();
    let batch = validate_input(input, limits, &mut baseline).unwrap();
    assert_eq!(batch.scratch_bytes(), baseline.current_bytes.get());
    drop(batch);
    assert_eq!(baseline.current_bytes.get(), 0);
    for allowed in 0..baseline.work {
        let mut control = TestControl {
            reject_work_after: Some(allowed),
            ..TestControl::default()
        };
        assert!(matches!(
            validate_input(input, limits, &mut control),
            Err(AnimationAuthoringFailure::Control(
                TestControlError::WorkRejected
            ))
        ));
        assert_eq!(control.current_bytes.get(), 0, "work {allowed}");
        assert_eq!(control.live.get(), 0, "work {allowed}");
    }
    for resize in 1..=baseline.resizes {
        let mut control = TestControl {
            reject_resize_on: Some(resize),
            ..TestControl::default()
        };
        assert!(matches!(
            validate_input(input, limits, &mut control),
            Err(AnimationAuthoringFailure::Control(
                TestControlError::ResizeRejected
            ))
        ));
        assert_eq!(control.current_bytes.get(), 0, "resize {resize}");
        assert_eq!(control.live.get(), 0, "resize {resize}");
    }
    for poll in 1..=baseline.polls {
        let mut control = TestControl {
            cancel_on_poll: Some(poll),
            ..TestControl::default()
        };
        assert!(matches!(
            validate_input(input, limits, &mut control),
            Err(AnimationAuthoringFailure::Control(
                TestControlError::Cancelled
            ))
        ));
        assert_eq!(control.current_bytes.get(), 0, "poll {poll}");
        assert_eq!(control.live.get(), 0, "poll {poll}");
    }
}

#[test]
fn scratch_limit_rejects_before_visiting_invalid_channel_payloads() {
    let invalid_times = [f64::NAN];
    let values = [0.0_f32; 3];
    let channels = [AnimationChannelAuthoring::new(
        1,
        AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
        AnimationInterpolation::Linear,
        AnimationExtrapolation::SourceDefault,
        AnimationExtrapolation::SourceDefault,
        &invalid_times,
        &values,
        Presence::Absent,
        Presence::Absent,
    )];
    let clips = [AnimationClipAuthoring::new(
        1,
        1.0,
        AnimationTimeDomain::Seconds,
        AuthoringRange::new(0, 1),
    )];
    let mut control = TestControl::default();
    assert!(matches!(
        validate_input(
            AnimationAuthoringBatch::new(&clips, &channels),
            SceneValidationLimits::default().with_max_scratch_bytes(0),
            &mut control,
        ),
        Err(AnimationAuthoringFailure::Semantic(
            AnimationAuthoringSemanticError::LimitExceeded {
                limit: AnimationAuthoringLimit::ScratchBytes,
                ..
            }
        ))
    ));
    assert_eq!(control.work, 0);
    assert_eq!(control.reserves, 0);
    assert_eq!(control.live.get(), 0);
}
