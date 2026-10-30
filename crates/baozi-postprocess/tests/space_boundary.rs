mod common;

use baozi_core::{
    AttributeData, AttributeDomain, Axis, ConventionValue, FrontFaceWinding, Handedness,
    SceneSpaceField, SourceSpaceError, TangentBasisRef, TangentEncodingRef, TargetSpace,
    ValidatedScene,
};
use baozi_postprocess::{
    NormalOptions, PostProcessCancellationToken, PostProcessControl, PostProcessControlCause,
    PostProcessOperation, PostProcessRunDisposition, PostProcessRunError, SpaceConversionCause,
    SpaceConversionOptions, convert_scene_space, generate_normals, triangulate_primitive,
};

const QUAD: &[u8] = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
    vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n";

fn reflected_centimeters() -> TargetSpace {
    TargetSpace::try_new(
        Handedness::Left,
        Axis::PositiveY,
        Axis::NegativeZ,
        0.01,
        FrontFaceWinding::CounterClockwise,
    )
    .unwrap()
}

fn options(target: TargetSpace) -> SpaceConversionOptions {
    SpaceConversionOptions {
        target,
        source_assumptions: Some(TargetSpace::STANDARD),
    }
}

fn primitive(scene: &ValidatedScene) -> baozi_core::PrimitiveRef<'_> {
    scene.meshes().next().unwrap().primitives().next().unwrap()
}

fn dense_f32(attribute: baozi_core::AttributeRef<'_>, expected_lanes: u8) -> &[f32] {
    let AttributeData::DenseF32 { values, lanes } = attribute.data() else {
        panic!("fixture must retain dense F32 attributes");
    };
    assert_eq!(lanes, expected_lanes);
    values
}

#[test]
fn triangulate_generate_and_convert_preserves_vertex_identity_and_corner_pairing() {
    let (scene, import_evidence) = common::import_obj(QUAD).into_parts();
    let target = primitive(&scene).id();
    let (scene, triangles) = triangulate_primitive(scene, target, PostProcessControl::default())
        .unwrap()
        .into_parts();
    let target = triangles.triangulation_remap().unwrap().output_target();
    let (scene, normals) = generate_normals(
        scene,
        target,
        NormalOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let input_generation = scene.generation();
    let source = primitive(&scene);
    let source_indices = source.indices().unwrap().to_vec();
    let source_uvs = dense_f32(source.texcoords(0, AttributeDomain::Corner).unwrap(), 2).to_vec();
    let source_space = scene.space();
    let uv_origin = source_space.uv_origin;
    assert_eq!(source_space.tangent_basis, TangentBasisRef::Unknown);
    assert_eq!(
        source_space.tangent_handedness_encoding,
        TangentEncodingRef::Unknown
    );
    let conversion = options(reflected_centimeters());
    let (scene, report) = convert_scene_space(scene, conversion, PostProcessControl::default())
        .expect("the whole static Scene must convert atomically")
        .into_parts();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_eq!(
        report.operation(),
        PostProcessOperation::ConvertSpace {
            input_generation,
            options: conversion
        }
    );
    assert_ne!(scene.generation(), input_generation);
    assert!(report.matches_scene(&scene));
    assert!(!import_evidence.matches_scene(&scene));
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
    assert!(report.resources().committed_canonical_bytes() > 0);
    let effects = report.space_conversion_effects().unwrap();
    assert_eq!(effects.input_generation(), input_generation);
    assert_eq!(effects.output_generation(), scene.generation());
    assert_eq!(effects.source(), TargetSpace::STANDARD);
    assert_eq!(effects.target(), conversion.target);
    assert!(effects.reverses_triangle_winding());
    for field in [
        SceneSpaceField::Handedness,
        SceneSpaceField::UpAxis,
        SceneSpaceField::FrontAxis,
        SceneSpaceField::UnitScaleToMeters,
        SceneSpaceField::FrontFaceWinding,
    ] {
        assert!(effects.assumed_source_field(field));
    }
    assert!(!effects.assumed_source_field(SceneSpaceField::UvOrigin));
    let space = scene.space();
    assert_eq!(space.handedness, Handedness::Left);
    assert_eq!(space.up_axis, Axis::PositiveY);
    assert_eq!(space.front_axis, Axis::NegativeZ);
    assert_eq!(space.unit_scale_to_meters, Some(0.01));
    assert_eq!(space.front_face_winding, FrontFaceWinding::CounterClockwise);
    assert_eq!(space.uv_origin, uv_origin);
    assert_eq!(space.tangent_basis, TangentBasisRef::Unknown);
    assert_eq!(
        space.tangent_handedness_encoding,
        TangentEncodingRef::Unknown
    );
    let output = primitive(&scene);
    assert_eq!(
        (
            output.vertex_count(),
            output.corner_count(),
            output.face_count()
        ),
        (4, 6, 2)
    );
    assert_eq!(
        dense_f32(output.positions().unwrap(), 3),
        &[
            0.0, 0.0, 0.0, -100.0, 0.0, 0.0, -100.0, 100.0, 0.0, 0.0, 100.0, 0.0
        ]
    );
    let indices = output.indices().unwrap();
    let uvs = dense_f32(output.texcoords(0, AttributeDomain::Corner).unwrap(), 2);
    for triangle in 0..2 {
        for (output_lane, source_lane) in [0, 2, 1].into_iter().enumerate() {
            let output_corner = triangle * 3 + output_lane;
            let source_corner = triangle * 3 + source_lane;
            assert_eq!(indices[output_corner], source_indices[source_corner]);
            assert_eq!(
                &uvs[output_corner * 2..output_corner * 2 + 2],
                &source_uvs[source_corner * 2..source_corner * 2 + 2]
            );
        }
    }
    for normal in dense_f32(output.normals(AttributeDomain::Corner).unwrap(), 3).chunks_exact(3) {
        assert_eq!(normal, [0.0, 0.0, 1.0]);
    }
    let output_generation = scene.generation();
    drop(scene);
    drop(import_evidence);
    drop(triangles);
    drop(normals);
    assert_eq!(effects.output_generation(), output_generation);
    assert_eq!(effects.target(), reflected_centimeters());
}

#[test]
fn declaring_unknown_source_facts_is_a_change_and_a_second_conversion_is_unchanged() {
    let (scene, evidence) = common::canonical_triangle_import().into_parts();
    let original = scene.generation();
    let (scene, first) = convert_scene_space(
        scene,
        options(TargetSpace::STANDARD),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_ne!(scene.generation(), original);
    assert!(!evidence.matches_scene(&scene));
    assert!(
        !first
            .space_conversion_effects()
            .unwrap()
            .reverses_triangle_winding()
    );
    let declared = scene.generation();
    drop(first);
    let (scene, unchanged) = convert_scene_space(
        scene,
        SpaceConversionOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_eq!(scene.generation(), declared);
    assert_eq!(
        unchanged.disposition(),
        PostProcessRunDisposition::Unchanged
    );
    assert!(unchanged.space_conversion_effects().is_none());
    assert_eq!(unchanged.resources().committed_canonical_bytes(), 0);
    assert_eq!(unchanged.resources().current_live_bytes(), 0);
    assert_eq!(
        dense_f32(primitive(&scene).positions().unwrap(), 3),
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
    );
}

#[test]
fn missing_source_facts_return_the_original_scene_and_typed_reason() {
    let (scene, evidence) = common::canonical_triangle_import().into_parts();
    let generation = scene.generation();
    let failure = convert_scene_space(
        scene,
        SpaceConversionOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.input().generation(), generation);
    assert!(evidence.matches_scene(failure.input()));
    let PostProcessRunError::SpaceConversion { failure } = failure.error() else {
        panic!("space failure");
    };
    assert!(matches!(
        failure.cause(),
        SpaceConversionCause::Source {
            source: SourceSpaceError::MissingSourceConvention {
                field: SceneSpaceField::Handedness
            }
        }
    ));
    assert_eq!(failure.resources().unwrap().committed_canonical_bytes(), 0);
    assert_eq!(failure.resources().unwrap().current_live_bytes(), 0);
}

#[test]
fn source_assumptions_cannot_override_known_scene_facts() {
    let scene = common::canonical_triangle_scene();
    let (scene, report) = convert_scene_space(
        scene,
        options(TargetSpace::STANDARD),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let generation = scene.generation();
    let failure = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: TargetSpace::STANDARD,
            source_assumptions: Some(reflected_centimeters()),
        },
        PostProcessControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.input().generation(), generation);
    assert!(report.matches_scene(failure.input()));
    let PostProcessRunError::SpaceConversion { failure } = failure.error() else {
        panic!("space failure");
    };
    assert!(matches!(
        failure.cause(),
        SpaceConversionCause::Source {
            source: SourceSpaceError::SourceAssumptionConflict {
                field: SceneSpaceField::Handedness,
                declared: ConventionValue::Handedness(Handedness::Right),
                assumed: ConventionValue::Handedness(Handedness::Left),
            }
        }
    ));
    assert_eq!(failure.resources().unwrap().committed_canonical_bytes(), 0);
}

#[test]
fn cancelled_and_expired_conversion_preserve_the_original_scene() {
    for cancelled in [true, false] {
        let (scene, evidence) = common::canonical_triangle_import().into_parts();
        let generation = scene.generation();
        let cancellation = PostProcessCancellationToken::new();
        let control = if cancelled {
            cancellation.cancel();
            PostProcessControl::default().with_cancellation(cancellation)
        } else {
            PostProcessControl::default()
                .with_deadline(std::time::Instant::now() - std::time::Duration::from_secs(1))
        };
        let failure =
            convert_scene_space(scene, options(reflected_centimeters()), control).unwrap_err();
        assert_eq!(failure.input().generation(), generation);
        assert!(evidence.matches_scene(failure.input()));
        let PostProcessRunError::SpaceConversion { failure } = failure.error() else {
            panic!("space failure");
        };
        assert!(matches!(
            failure.cause(),
            SpaceConversionCause::Control { .. }
        ));
        if !cancelled {
            assert!(matches!(
                failure.cause(),
                SpaceConversionCause::Control {
                    cause: PostProcessControlCause::DeadlineExceeded
                }
            ));
        }
        assert_eq!(failure.resources().unwrap().committed_canonical_bytes(), 0);
        assert_eq!(failure.resources().unwrap().current_live_bytes(), 0);
    }
}

#[test]
fn whole_scene_conversion_and_detached_evidence_fit_a_one_mib_stack() {
    let scene = common::canonical_triangle_scene();
    let success = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            convert_scene_space(
                scene,
                options(reflected_centimeters()),
                PostProcessControl::default(),
            )
            .expect("conversion must fit an ordinary caller stack")
        })
        .unwrap()
        .join()
        .expect("conversion thread");
    let (scene, report) = success.into_parts();
    let output = scene.generation();
    drop(scene);
    std::thread::spawn(move || {
        let effects = report.space_conversion_effects().unwrap();
        assert_eq!(effects.output_generation(), output);
        assert_eq!(effects.source(), TargetSpace::STANDARD);
        assert_eq!(effects.target(), reflected_centimeters());
        assert_eq!(report.resources().current_live_bytes(), 0);
    })
    .join()
    .unwrap();
}
