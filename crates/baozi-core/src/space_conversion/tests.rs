use super::*;
use crate::entity::AuthoredTransformKind;
use crate::scene_space::{TangentBasisConvention, TangentHandednessEncoding, UvOrigin};

const AXES: [Axis; 6] = [
    Axis::PositiveX,
    Axis::NegativeX,
    Axis::PositiveY,
    Axis::NegativeY,
    Axis::PositiveZ,
    Axis::NegativeZ,
];

fn scene(space: TargetSpace) -> SceneSpace {
    SceneSpace {
        handedness: space.handedness(),
        up_axis: space.up_axis(),
        front_axis: space.front_axis(),
        unit_scale_to_meters: Some(space.unit_scale_to_meters()),
        front_face_winding: space.front_face_winding(),
        ..SceneSpace::UNKNOWN
    }
}

fn with_units(space: TargetSpace, units: f64) -> TargetSpace {
    TargetSpace::try_new(
        space.handedness(),
        space.up_axis(),
        space.front_axis(),
        units,
        space.front_face_winding(),
    )
    .unwrap()
}

fn all_bases() -> [TargetSpace; 48] {
    let mut output = [TargetSpace::STANDARD; 48];
    let mut index = 0;
    for handedness in [Handedness::Right, Handedness::Left] {
        for up in AXES {
            for front in AXES {
                if up.shares_underlying_axis(front) {
                    continue;
                }
                output[index] = TargetSpace::try_new(
                    handedness,
                    up,
                    front,
                    1.0,
                    FrontFaceWinding::CounterClockwise,
                )
                .unwrap();
                index += 1;
            }
        }
    }
    assert_eq!(index, output.len());
    output
}

fn axis(axis: Axis) -> [f32; 3] {
    match axis {
        Axis::PositiveX => [1.0, 0.0, 0.0],
        Axis::NegativeX => [-1.0, 0.0, 0.0],
        Axis::PositiveY => [0.0, 1.0, 0.0],
        Axis::NegativeY => [0.0, -1.0, 0.0],
        Axis::PositiveZ => [0.0, 0.0, 1.0],
        Axis::NegativeZ => [0.0, 0.0, -1.0],
        Axis::Unknown => panic!("test axes are known"),
    }
}

#[test]
fn intrinsic_frames_compose_on_the_left_keep_reflection_and_roundtrip_all_48_states() {
    let mut frames = Vec::new();
    for right in AXES {
        for up in AXES {
            for forward in AXES {
                if let Ok(frame) = crate::LocalAxes::try_new(right, up, forward) {
                    frames.push(frame);
                }
            }
        }
    }
    assert_eq!(frames.len(), 48);
    for target in all_bases() {
        let target = with_units(target, 0.5);
        let conversion =
            SpaceConversion::try_new(&scene(TargetSpace::STANDARD), target, None).unwrap();
        let inverse =
            SpaceConversion::try_new(&scene(target), TargetSpace::STANDARD, None).unwrap();
        let measured = allocation_counter::measure(|| {
            for &frame in &frames {
                let converted = conversion.local_axes(frame).unwrap();
                assert_eq!(
                    axis(converted.right()),
                    conversion.direction(axis(frame.right())).unwrap()
                );
                assert_eq!(
                    axis(converted.up()),
                    conversion.direction(axis(frame.up())).unwrap()
                );
                assert_eq!(
                    axis(converted.forward()),
                    conversion.direction(axis(frame.forward())).unwrap()
                );
                assert_eq!(inverse.local_axes(converted), Some(frame));
            }
        });
        assert_eq!(measured.count_total, 0);
    }
    let mirror = TargetSpace::try_new(
        Handedness::Left,
        Axis::PositiveY,
        Axis::NegativeZ,
        1.0,
        FrontFaceWinding::CounterClockwise,
    )
    .unwrap();
    let conversion = SpaceConversion::try_new(&scene(TargetSpace::STANDARD), mirror, None).unwrap();
    assert_eq!(
        conversion.local_axes(crate::LocalAxes::STANDARD),
        Some(crate::LocalAxes::try_new(Axis::NegativeX, Axis::PositiveY, Axis::NegativeZ).unwrap()),
        "reconstructing right from the other two axes would lose this reflection",
    );
}

#[test]
fn scene_unit_length_is_independent_of_basis_and_keeps_zero_sign_and_numeric_failures() {
    let conversion = SpaceConversion::try_new(
        &scene(TargetSpace::STANDARD),
        reflection_target(FrontFaceWinding::CounterClockwise),
        None,
    )
    .unwrap();
    assert_eq!(conversion.length(3.0), Ok(6.0));
    assert_eq!(conversion.length(-3.0), Ok(-6.0));
    assert_eq!(
        conversion.length(-0.0).unwrap().to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        conversion.length(f32::MAX),
        Err(NumericConversionError::NonFiniteOutput)
    );
    assert_eq!(
        conversion.length(f32::NAN),
        Err(NumericConversionError::NonFiniteOutput)
    );
    let half = SpaceConversion::try_new(
        &scene(TargetSpace::STANDARD),
        with_units(TargetSpace::STANDARD, 2.0),
        None,
    )
    .unwrap();
    assert_eq!(
        half.length(f32::from_bits(1)),
        Err(NumericConversionError::UnderflowToZero)
    );
}

fn bits(value: [f32; 3]) -> [u32; 3] {
    value.map(f32::to_bits)
}

fn quat_bits(value: Quat) -> [u32; 4] {
    [value.x, value.y, value.z, value.w].map(f32::to_bits)
}

fn matrix_bits(value: Mat4) -> [[u32; 4]; 4] {
    value.cols.map(|column| column.map(f32::to_bits))
}

fn reflection_target(winding: FrontFaceWinding) -> TargetSpace {
    TargetSpace::try_new(
        Handedness::Left,
        Axis::PositiveZ,
        Axis::PositiveY,
        0.5,
        winding,
    )
    .unwrap()
}

#[test]
fn target_space_validates_all_five_facts_and_positive_finite_unit_boundaries() {
    let standard = TargetSpace::STANDARD;
    assert_eq!(standard.handedness(), Handedness::Right);
    assert_eq!(standard.up_axis(), Axis::PositiveY);
    assert_eq!(standard.front_axis(), Axis::NegativeZ);
    assert_eq!(standard.unit_scale_to_meters(), 1.0);
    assert_eq!(
        standard.front_face_winding(),
        FrontFaceWinding::CounterClockwise
    );
    for (hand, up, front, winding, field) in [
        (
            Handedness::Unknown,
            Axis::PositiveY,
            Axis::NegativeZ,
            FrontFaceWinding::CounterClockwise,
            SceneSpaceField::Handedness,
        ),
        (
            Handedness::Right,
            Axis::Unknown,
            Axis::NegativeZ,
            FrontFaceWinding::CounterClockwise,
            SceneSpaceField::UpAxis,
        ),
        (
            Handedness::Right,
            Axis::PositiveY,
            Axis::Unknown,
            FrontFaceWinding::CounterClockwise,
            SceneSpaceField::FrontAxis,
        ),
        (
            Handedness::Right,
            Axis::PositiveY,
            Axis::NegativeZ,
            FrontFaceWinding::Unknown,
            SceneSpaceField::FrontFaceWinding,
        ),
    ] {
        assert_eq!(
            TargetSpace::try_new(hand, up, front, 1.0, winding),
            Err(SceneSpaceValidationError::UnknownField { field })
        );
    }
    for front in [Axis::PositiveY, Axis::NegativeY] {
        assert_eq!(
            TargetSpace::try_new(
                Handedness::Right,
                Axis::PositiveY,
                front,
                1.0,
                FrontFaceWinding::CounterClockwise,
            ),
            Err(SceneSpaceValidationError::CoincidentAxes {
                up_axis: Axis::PositiveY,
                front_axis: front,
            })
        );
    }
    for units in [f64::from_bits(1), f64::MIN_POSITIVE, f64::MAX] {
        assert_eq!(with_units(standard, units).unit_scale_to_meters(), units);
    }
    for (units, expected) in [
        (f64::NAN, SceneSpaceValidationError::NonFiniteUnitScale),
        (f64::INFINITY, SceneSpaceValidationError::NonFiniteUnitScale),
        (
            f64::NEG_INFINITY,
            SceneSpaceValidationError::NonFiniteUnitScale,
        ),
        (0.0, SceneSpaceValidationError::NonPositiveUnitScale),
        (-0.0, SceneSpaceValidationError::NonPositiveUnitScale),
        (-1.0, SceneSpaceValidationError::NonPositiveUnitScale),
    ] {
        assert_eq!(
            TargetSpace::try_new(
                standard.handedness(),
                standard.up_axis(),
                standard.front_axis(),
                units,
                standard.front_face_winding(),
            ),
            Err(expected)
        );
    }
}

#[test]
fn every_pair_of_the_48_bases_maps_up_front_and_preserves_roundtrip_bits() {
    let bases = all_bases();
    let value = [f32::from_bits(1), -0.0, -f32::MAX];
    for source in bases {
        for target in bases {
            let forward = SpaceConversion::try_new(&scene(source), target, None).unwrap();
            let inverse = SpaceConversion::try_new(&scene(target), source, None).unwrap();
            assert_eq!(forward.changes_coordinates(), source != target);
            assert_eq!(forward.is_identity(), source == target);
            assert_eq!(
                forward.direction(axis(source.up_axis())),
                Ok(axis(target.up_axis()))
            );
            assert_eq!(
                forward.direction(axis(source.front_axis())),
                Ok(axis(target.front_axis()))
            );
            assert_eq!(
                bits(
                    inverse
                        .direction(forward.direction(value).unwrap())
                        .unwrap()
                ),
                bits(value)
            );
            assert_eq!(
                bits(inverse.point(forward.point(value).unwrap()).unwrap()),
                bits(value)
            );
            assert_eq!(
                bits(inverse.scale(forward.scale(value).unwrap()).unwrap()),
                bits(value)
            );

            let x = forward.direction([1.0, 0.0, 0.0]).unwrap();
            let y = forward.direction([0.0, 1.0, 0.0]).unwrap();
            let z = forward.direction([0.0, 0.0, 1.0]).unwrap();
            let determinant = x[0] * (y[1] * z[2] - y[2] * z[1])
                - y[0] * (x[1] * z[2] - x[2] * z[1])
                + z[0] * (x[1] * y[2] - x[2] * y[1]);
            let expected = if source.handedness() == target.handedness() {
                1
            } else {
                -1
            };
            assert_eq!(determinant, f32::from(expected));
            assert_eq!(forward.determinant_sign(), expected);
        }
    }
}

#[test]
fn reflection_and_winding_combine_by_parity_independently_of_units() {
    let source = scene(TargetSpace::STANDARD);
    let target = reflection_target(FrontFaceWinding::CounterClockwise);
    let conversion = SpaceConversion::try_new(&source, target, None).unwrap();
    // The independently specified map is (x, y, z) -> (-x, -z, y), with two target units per meter.
    assert_eq!(conversion.direction([2.0, 3.0, 5.0]), Ok([-2.0, -5.0, 3.0]));
    assert_eq!(conversion.point([2.0, 3.0, 5.0]), Ok([-4.0, -10.0, 6.0]));
    assert_eq!(conversion.unit_ratio(), 2.0);
    assert_eq!(conversion.source(), TargetSpace::STANDARD);
    assert_eq!(conversion.target(), target);
    assert_eq!(conversion.determinant_sign(), -1);
    assert!(conversion.reverse_winding());
    let both = SpaceConversion::try_new(
        &source,
        reflection_target(FrontFaceWinding::Clockwise),
        None,
    )
    .unwrap();
    assert!(!both.reverse_winding());
    assert!(both.changes_coordinates());
    assert!(!both.is_identity());
    let winding_only = TargetSpace::try_new(
        source.handedness,
        source.up_axis,
        source.front_axis,
        1.0,
        FrontFaceWinding::Clockwise,
    )
    .unwrap();
    let winding_only = SpaceConversion::try_new(&source, winding_only, None).unwrap();
    assert!(winding_only.reverse_winding());
    assert!(!winding_only.changes_coordinates());
    assert!(!winding_only.is_identity());
    assert!(
        !SpaceConversion::try_new(&source, with_units(TargetSpace::STANDARD, 2.0), None)
            .unwrap()
            .reverse_winding()
    );
}

#[test]
fn direction_scale_and_quaternion_only_permute_or_flip_finite_component_bits() {
    let conversion = SpaceConversion::try_new(
        &scene(TargetSpace::STANDARD),
        reflection_target(FrontFaceWinding::CounterClockwise),
        None,
    )
    .unwrap();
    let value = [-0.0, -2.0, f32::from_bits(1)];
    assert_eq!(
        bits(conversion.direction(value).unwrap()),
        bits([0.0, -f32::from_bits(1), -2.0])
    );
    assert_eq!(
        bits(conversion.scale(value).unwrap()),
        bits([-0.0, f32::from_bits(1), -2.0])
    );
    let rotation = Quat::from_xyzw(-0.0, 2.0, -3.0, -0.0);
    assert_eq!(
        quat_bits(conversion.rotation(rotation).unwrap()),
        quat_bits(Quat::from_xyzw(-0.0, -3.0, -2.0, -0.0))
    );
}

#[test]
fn units_apply_only_to_points_and_accept_nonzero_subnormal_outputs() {
    let centimeters = with_units(TargetSpace::STANDARD, 0.01);
    let conversion =
        SpaceConversion::try_new(&scene(centimeters), TargetSpace::STANDARD, None).unwrap();
    assert_eq!(
        conversion.point([100.0, -250.0, 75.0]),
        Ok([1.0, -2.5, 0.75])
    );
    assert_eq!(
        conversion.direction([100.0, -250.0, 75.0]),
        Ok([100.0, -250.0, 75.0])
    );
    assert_eq!(
        conversion.scale([100.0, -250.0, 75.0]),
        Ok([100.0, -250.0, 75.0])
    );
    let double = SpaceConversion::try_new(
        &scene(with_units(TargetSpace::STANDARD, 2.0)),
        TargetSpace::STANDARD,
        None,
    )
    .unwrap();
    assert_eq!(
        bits(
            double
                .point([f32::from_bits(1), f32::from_bits(0x007f_ffff), -0.0])
                .unwrap()
        ),
        [2, 0x00ff_fffe, (-0.0_f32).to_bits()]
    );
}

#[test]
fn unknown_source_facts_are_never_inferred_from_the_target() {
    assert_eq!(
        SpaceConversion::try_new(&SceneSpace::UNKNOWN, TargetSpace::STANDARD, None),
        Err(SourceSpaceError::MissingSourceConvention {
            field: SceneSpaceField::Handedness
        })
    );
    let standard = scene(TargetSpace::STANDARD);
    for field in [
        SceneSpaceField::Handedness,
        SceneSpaceField::UpAxis,
        SceneSpaceField::FrontAxis,
        SceneSpaceField::UnitScaleToMeters,
        SceneSpaceField::FrontFaceWinding,
    ] {
        let mut source = standard.clone();
        match field {
            SceneSpaceField::Handedness => source.handedness = Handedness::Unknown,
            SceneSpaceField::UpAxis => source.up_axis = Axis::Unknown,
            SceneSpaceField::FrontAxis => source.front_axis = Axis::Unknown,
            SceneSpaceField::UnitScaleToMeters => source.unit_scale_to_meters = None,
            SceneSpaceField::FrontFaceWinding => {
                source.front_face_winding = FrontFaceWinding::Unknown
            }
            _ => unreachable!(),
        }
        assert_eq!(
            SpaceConversion::try_new(&source, TargetSpace::STANDARD, None),
            Err(SourceSpaceError::MissingSourceConvention { field })
        );
    }
}

#[test]
fn explicit_complete_assumption_fills_only_unknown_fields_and_records_the_actual_mask() {
    let assumed = reflection_target(FrontFaceWinding::Clockwise);
    let conversion =
        SpaceConversion::try_new(&SceneSpace::UNKNOWN, TargetSpace::STANDARD, Some(assumed))
            .unwrap();
    assert_eq!(conversion.source(), assumed);
    for field in [
        SceneSpaceField::Handedness,
        SceneSpaceField::UpAxis,
        SceneSpaceField::FrontAxis,
        SceneSpaceField::UnitScaleToMeters,
        SceneSpaceField::FrontFaceWinding,
    ] {
        assert!(conversion.assumed_fields().contains(field));
    }
    assert!(
        !conversion
            .assumed_fields()
            .contains(SceneSpaceField::UvOrigin)
    );
    assert!(
        !conversion
            .assumed_fields()
            .contains(SceneSpaceField::TangentBasis)
    );

    let source = SceneSpace {
        up_axis: Axis::Unknown,
        unit_scale_to_meters: None,
        ..scene(assumed)
    };
    let conversion =
        SpaceConversion::try_new(&source, TargetSpace::STANDARD, Some(assumed)).unwrap();
    assert_eq!(conversion.source(), assumed);
    assert!(
        conversion
            .assumed_fields()
            .contains(SceneSpaceField::UpAxis)
    );
    assert!(
        conversion
            .assumed_fields()
            .contains(SceneSpaceField::UnitScaleToMeters)
    );
    assert!(
        !conversion
            .assumed_fields()
            .contains(SceneSpaceField::Handedness)
    );
    assert!(
        !conversion
            .assumed_fields()
            .contains(SceneSpaceField::FrontAxis)
    );
    assert!(
        !conversion
            .assumed_fields()
            .contains(SceneSpaceField::FrontFaceWinding)
    );
    let matching =
        SpaceConversion::try_new(&scene(assumed), TargetSpace::STANDARD, Some(assumed)).unwrap();
    assert!(matching.assumed_fields().is_empty());
}

#[test]
fn assumption_conflicts_retain_the_field_and_exact_typed_values() {
    let assumed = TargetSpace::STANDARD;
    let adjacent_units = f64::from_bits(1.0_f64.to_bits() + 1);
    for (field, declared, expected_assumed) in [
        (
            SceneSpaceField::Handedness,
            ConventionValue::Handedness(Handedness::Left),
            ConventionValue::Handedness(Handedness::Right),
        ),
        (
            SceneSpaceField::UpAxis,
            ConventionValue::Axis(Axis::PositiveX),
            ConventionValue::Axis(Axis::PositiveY),
        ),
        (
            SceneSpaceField::FrontAxis,
            ConventionValue::Axis(Axis::PositiveZ),
            ConventionValue::Axis(Axis::NegativeZ),
        ),
        (
            SceneSpaceField::UnitScaleToMeters,
            ConventionValue::UnitScaleToMeters(adjacent_units),
            ConventionValue::UnitScaleToMeters(1.0),
        ),
        (
            SceneSpaceField::FrontFaceWinding,
            ConventionValue::FrontFaceWinding(FrontFaceWinding::Clockwise),
            ConventionValue::FrontFaceWinding(FrontFaceWinding::CounterClockwise),
        ),
    ] {
        let mut source = scene(assumed);
        match (field, declared) {
            (SceneSpaceField::Handedness, ConventionValue::Handedness(value)) => {
                source.handedness = value
            }
            (SceneSpaceField::UpAxis, ConventionValue::Axis(value)) => source.up_axis = value,
            (SceneSpaceField::FrontAxis, ConventionValue::Axis(value)) => source.front_axis = value,
            (SceneSpaceField::UnitScaleToMeters, ConventionValue::UnitScaleToMeters(value)) => {
                source.unit_scale_to_meters = Some(value)
            }
            (SceneSpaceField::FrontFaceWinding, ConventionValue::FrontFaceWinding(value)) => {
                source.front_face_winding = value
            }
            _ => unreachable!(),
        }
        assert_eq!(
            SpaceConversion::try_new(&source, assumed, Some(assumed)),
            Err(SourceSpaceError::SourceAssumptionConflict {
                field,
                declared,
                assumed: expected_assumed
            })
        );
    }
}

#[test]
fn invalid_source_facts_are_rejected_while_uv_and_tangent_conventions_are_preserved() {
    let source = SceneSpace {
        unit_scale_to_meters: Some(-0.0),
        ..scene(TargetSpace::STANDARD)
    };
    assert_eq!(
        SpaceConversion::try_new(&source, TargetSpace::STANDARD, Some(TargetSpace::STANDARD)),
        Err(SourceSpaceError::InvalidResolvedSource {
            source: SceneSpaceValidationError::NonPositiveUnitScale
        })
    );
    let source = SceneSpace {
        front_axis: Axis::NegativeY,
        ..scene(TargetSpace::STANDARD)
    };
    assert_eq!(
        SpaceConversion::try_new(&source, TargetSpace::STANDARD, None),
        Err(SourceSpaceError::InvalidResolvedSource {
            source: SceneSpaceValidationError::CoincidentAxes {
                up_axis: Axis::PositiveY,
                front_axis: Axis::NegativeY
            },
        })
    );
    let source = SceneSpace {
        uv_origin: UvOrigin::LowerLeft,
        tangent_basis: TangentBasisConvention::MikkTspace,
        tangent_handedness_encoding: TangentHandednessEncoding::SignInW,
        ..scene(TargetSpace::STANDARD)
    };
    let before = source.clone();
    let conversion = SpaceConversion::try_new(
        &source,
        reflection_target(FrontFaceWinding::Clockwise),
        None,
    )
    .unwrap();
    assert!(conversion.assumed_fields().is_empty());
    assert_eq!(source, before);
}

#[test]
fn unit_ratio_rejects_overflow_and_zero_but_does_not_require_a_representable_reciprocal() {
    for (source_units, target_units) in
        [(f64::MAX, f64::from_bits(1)), (f64::from_bits(1), f64::MAX)]
    {
        assert_eq!(
            SpaceConversion::try_new(
                &scene(with_units(TargetSpace::STANDARD, source_units)),
                with_units(TargetSpace::STANDARD, target_units),
                None,
            ),
            Err(SourceSpaceError::UnrepresentableUnitScale)
        );
    }
    let conversion = SpaceConversion::try_new(
        &scene(with_units(TargetSpace::STANDARD, f64::from_bits(1))),
        TargetSpace::STANDARD,
        None,
    )
    .unwrap();
    assert_eq!(conversion.unit_ratio(), f64::from_bits(1));
    assert_eq!(
        bits(conversion.point([-0.0, 0.0, -0.0]).unwrap()),
        bits([-0.0, 0.0, -0.0])
    );
    assert_eq!(conversion.matrix(Mat4::IDENTITY), Ok(Mat4::IDENTITY));
}

#[test]
fn numeric_errors_reject_output_overflow_and_both_f32_and_f64_underflow_to_zero() {
    let source = scene(TargetSpace::STANDARD);
    let double =
        SpaceConversion::try_new(&source, with_units(TargetSpace::STANDARD, 0.5), None).unwrap();
    assert_eq!(
        double.point([f32::MAX, 0.0, 0.0]),
        Err(NumericConversionError::NonFiniteOutput)
    );
    let half =
        SpaceConversion::try_new(&source, with_units(TargetSpace::STANDARD, 2.0), None).unwrap();
    assert_eq!(
        half.point([f32::from_bits(1), 0.0, 0.0]),
        Err(NumericConversionError::UnderflowToZero)
    );
    let tiny = SpaceConversion::try_new(
        &scene(with_units(TargetSpace::STANDARD, f64::from_bits(1))),
        TargetSpace::STANDARD,
        None,
    )
    .unwrap();
    assert_eq!(
        tiny.point([f32::from_bits(1), 0.0, 0.0]),
        Err(NumericConversionError::UnderflowToZero)
    );
    let huge = SpaceConversion::try_new(
        &scene(with_units(TargetSpace::STANDARD, f64::MAX)),
        TargetSpace::STANDARD,
        None,
    )
    .unwrap();
    assert_eq!(
        huge.point([f32::MAX, 0.0, 0.0]),
        Err(NumericConversionError::NonFiniteOutput)
    );

    let mut matrix = Mat4::IDENTITY;
    matrix.cols[0][3] = f32::from_bits(1);
    assert_eq!(
        double.matrix(matrix),
        Err(NumericConversionError::UnderflowToZero)
    );
    matrix.cols[0][3] = f32::MAX;
    assert_eq!(
        half.matrix(matrix),
        Err(NumericConversionError::NonFiniteOutput)
    );
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            double.direction([value, 0.0, 0.0]),
            Err(NumericConversionError::NonFiniteOutput)
        );
        assert_eq!(
            double.scale([value, 0.0, 0.0]),
            Err(NumericConversionError::NonFiniteOutput)
        );
        assert_eq!(
            double.rotation(Quat::from_xyzw(0.0, 0.0, 0.0, value)),
            Err(NumericConversionError::NonFiniteOutput)
        );
    }
}

#[test]
fn matrix_conversion_preserves_shear_singular_linear_blocks_and_the_authored_variant() {
    let conversion = SpaceConversion::try_new(
        &scene(TargetSpace::STANDARD),
        reflection_target(FrontFaceWinding::CounterClockwise),
        None,
    )
    .unwrap();
    let matrix = Mat4 {
        cols: [
            [0.0, -0.0, 0.0, -0.0],
            [2.0, 3.0, 4.0, 0.0],
            [-5.0, 6.0, 7.0, -0.0],
            [8.0, -9.0, 10.0, 1.0],
        ],
    };
    let expected = Mat4 {
        cols: [
            [0.0, 0.0, 0.0, 0.0],
            [-5.0, 7.0, -6.0, 0.0],
            [-2.0, -4.0, 3.0, 0.0],
            [-16.0, -20.0, -18.0, 1.0],
        ],
    };
    assert_eq!(
        matrix_bits(conversion.matrix(matrix).unwrap()),
        matrix_bits(expected)
    );
    let authored = AuthoredTransform::try_matrix(matrix).unwrap();
    let output = conversion.authored_transform(authored).unwrap();
    assert_eq!(output.kind(), AuthoredTransformKind::Matrix);
    assert_eq!(
        matrix_bits(output.authored_matrix().unwrap()),
        matrix_bits(expected)
    );
    let inverse =
        SpaceConversion::try_new(&scene(conversion.target()), TargetSpace::STANDARD, None).unwrap();
    assert!(
        inverse
            .authored_transform(output)
            .unwrap()
            .has_same_authored_bits(authored)
    );
}

// Dense f64 reference arithmetic checks the matrix equation independently of the production
// implementation's indexed block permutation. Matrices here use row-major arrays.
fn multiply(left: [[f64; 4]; 4], right: [[f64; 4]; 4]) -> [[f64; 4]; 4] {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            (0..4)
                .map(|index| left[row][index] * right[index][column])
                .sum()
        })
    })
}

fn conjugate(matrix: Mat4, q: [[f64; 3]; 3], units: f64) -> [[f64; 4]; 4] {
    let mut h = [[0.0; 4]; 4];
    let mut inverse = [[0.0; 4]; 4];
    for row in 0..3 {
        for column in 0..3 {
            h[row][column] = units * q[row][column];
            inverse[row][column] = q[column][row] / units;
        }
    }
    h[3][3] = 1.0;
    inverse[3][3] = 1.0;
    let source = std::array::from_fn(|row| {
        std::array::from_fn(|column| f64::from(matrix.cols[column][row]))
    });
    multiply(multiply(h, source), inverse)
}

fn assert_matrix_near(actual: Mat4, expected: [[f64; 4]; 4]) {
    for (row, values) in expected.into_iter().enumerate() {
        for (column, value) in values.into_iter().enumerate() {
            assert!(
                (f64::from(actual.cols[column][row]) - value).abs() <= 1.0e-6,
                "matrix entry ({row}, {column}) differs: {} vs {value}",
                actual.cols[column][row]
            );
        }
    }
}

#[test]
fn all_48_bases_preserve_trs_variant_negative_zero_scale_and_quaternion_matrix_equivalence() {
    let rotation = Quat::from_xyzw(0.5, -0.5, 0.5, 0.5);
    let authored = AuthoredTransform::try_trs(
        Vec3::new(2.0, -3.0, 5.0),
        rotation,
        Vec3::new(-2.0, -0.0, 3.0),
    )
    .unwrap();
    for target in all_bases() {
        let target = with_units(target, 0.5);
        let conversion =
            SpaceConversion::try_new(&scene(TargetSpace::STANDARD), target, None).unwrap();
        let output = conversion.authored_transform(authored).unwrap();
        assert_eq!(output.kind(), AuthoredTransformKind::Trs);
        let converted_rotation = conversion.rotation(rotation).unwrap();
        assert_eq!(converted_rotation.w.to_bits(), rotation.w.to_bits());
        let columns = [
            conversion.direction([1.0, 0.0, 0.0]).unwrap(),
            conversion.direction([0.0, 1.0, 0.0]).unwrap(),
            conversion.direction([0.0, 0.0, 1.0]).unwrap(),
        ];
        let q = std::array::from_fn(|row| {
            std::array::from_fn(|column| f64::from(columns[column][row]))
        });
        assert_matrix_near(
            output.local_matrix(),
            conjugate(authored.local_matrix(), q, 2.0),
        );
        assert_matrix_near(
            Mat4::from_trs(Vec3::ZERO, converted_rotation, Vec3::new(1.0, 1.0, 1.0)),
            conjugate(
                Mat4::from_trs(Vec3::ZERO, rotation, Vec3::new(1.0, 1.0, 1.0)),
                q,
                1.0,
            ),
        );
        let inverse =
            SpaceConversion::try_new(&scene(target), TargetSpace::STANDARD, None).unwrap();
        assert!(
            inverse
                .authored_transform(output)
                .unwrap()
                .has_same_authored_bits(authored)
        );
    }
}

#[test]
fn general_matrix_conjugation_matches_dense_reference_without_an_affine_requirement() {
    let conversion = SpaceConversion::try_new(
        &scene(TargetSpace::STANDARD),
        reflection_target(FrontFaceWinding::CounterClockwise),
        None,
    )
    .unwrap();
    let matrix = Mat4 {
        cols: [
            [1.0, -2.0, 3.0, 0.5],
            [4.0, 5.0, -6.0, -1.0],
            [-7.0, 8.0, 9.0, 2.0],
            [10.0, -11.0, 12.0, -0.0],
        ],
    };
    let q = [[-1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]];
    let converted = conversion.matrix(matrix).unwrap();
    assert_matrix_near(converted, conjugate(matrix, q, 2.0));
    assert_eq!(converted.cols[3][3].to_bits(), (-0.0_f32).to_bits());
    let inverse =
        SpaceConversion::try_new(&scene(conversion.target()), TargetSpace::STANDARD, None).unwrap();
    assert_eq!(
        matrix_bits(inverse.matrix(converted).unwrap()),
        matrix_bits(matrix)
    );
}

#[test]
fn conversion_planning_arithmetic_and_errors_allocate_nothing() {
    let source = scene(TargetSpace::STANDARD);
    let target = reflection_target(FrontFaceWinding::CounterClockwise);
    let authored = AuthoredTransform::try_trs(
        Vec3::new(2.0, 3.0, 5.0),
        Quat::IDENTITY,
        Vec3::new(-1.0, -0.0, 2.0),
    )
    .unwrap();
    let allocations = allocation_counter::measure(|| {
        let conversion = SpaceConversion::try_new(&source, target, None).unwrap();
        std::hint::black_box(conversion.point([2.0, 3.0, 5.0]).unwrap());
        std::hint::black_box(conversion.direction([2.0, 3.0, 5.0]).unwrap());
        std::hint::black_box(conversion.scale([-2.0, -0.0, 5.0]).unwrap());
        std::hint::black_box(conversion.rotation(Quat::IDENTITY).unwrap());
        std::hint::black_box(conversion.matrix(Mat4::IDENTITY).unwrap());
        std::hint::black_box(conversion.authored_transform(authored).unwrap());
        assert!(SpaceConversion::try_new(&source, target, Some(target)).is_err());
        assert!(SpaceConversion::try_new(&SceneSpace::UNKNOWN, target, None).is_err());
        assert_eq!(
            conversion.point([f32::MAX, 0.0, 0.0]),
            Err(NumericConversionError::NonFiniteOutput)
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}
