use super::*;

const PLANE: [[f64; 3]; 3] = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

fn scaled_positions(positions: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut magnitude = 0.0;
    for row in positions {
        for component in row {
            observe_magnitude(&mut magnitude, component).unwrap();
        }
    }
    let scale = common_scale(magnitude);
    positions.map(|row| row.map(|component| scale_exact(component, scale).unwrap()))
}

fn assert_direction(actual: [f64; 3], expected: [f64; 3]) {
    for axis in 0..3 {
        assert!((actual[axis] - expected[axis]).abs() <= 8.0 * f64::EPSILON);
    }
}

#[test]
fn common_scale_covers_the_entire_finite_binary64_exponent_range() {
    for maximum in [
        ETA,
        f64::from_bits(2),
        f64::from_bits((1_u64 << 52) - 1),
        f64::MIN_POSITIVE,
        2.0_f64.powi(-1000),
        1.0,
        2.0_f64.powi(1000),
        f64::MAX,
    ] {
        let scale = common_scale(maximum);
        let scaled = scale_exact(maximum, scale).unwrap();
        assert!((0.5..1.0).contains(&scaled));
        assert_eq!(scale_exact(-maximum, scale).unwrap(), -scaled);
        assert_eq!(
            scale_exact(-0.0, scale).unwrap().to_bits(),
            (-0.0_f64).to_bits()
        );
    }
    assert_eq!(common_scale(ETA).shift, 1073);
    assert_eq!(common_scale(f64::MAX).shift, -1024);
    assert_eq!(scale_exact(0.0, common_scale(0.0)).unwrap(), 0.0);
}

#[test]
fn scaling_detects_zero_and_nonzero_subnormal_precision_loss() {
    assert_eq!(
        scale_exact(ETA, common_scale(2.0_f64.powi(1000))),
        Err(GeometryNumericIssue::ScalingUnderflow),
    );
    assert_eq!(
        scale_exact(f64::from_bits(0x0010_0000_0000_0001), common_scale(2.0)),
        Err(GeometryNumericIssue::ScalingPrecisionLoss),
    );
}

#[test]
fn nonfinite_inputs_and_intermediates_are_named_errors() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut maximum = 0.0;
        assert_eq!(
            observe_magnitude(&mut maximum, value),
            Err(GeometryNumericIssue::NonFiniteInput)
        );
        assert_eq!(
            scale_exact(value, common_scale(1.0)),
            Err(GeometryNumericIssue::NonFiniteInput)
        );
    }
    assert!(matches!(
        diff_products_f64(f64::MAX, f64::MAX, 0.0, 0.0),
        Err(GeometryNumericIssue::NonFiniteIntermediate)
    ));
}

#[test]
fn fma_recovers_a_product_cancellation_sign_without_a_product_relative_cutoff() {
    let h = f64::EPSILON;
    let determinant = diff_products_f64(1.0 + h, 1.0 - h, 1.0, 1.0).unwrap();
    assert_eq!(determinant.value, -h * h);
    assert!(determinant.abs_error < determinant.value.abs());
    assert_eq!(certified_uv_sign(determinant).unwrap(), -1.0);

    let minor = orient2_f64([0.0, 1.0 + h, 1.0], [0.0, 1.0, 1.0 - h]).unwrap();
    assert_eq!(minor.value, -h * h);
    assert!(minor.abs_error < minor.value.abs());
}

#[test]
fn a_rounded_zero_underflow_is_not_a_witnessed_zero_determinant() {
    let determinant = diff_products_f64(ETA, 0.5, 0.0, 0.0).unwrap();
    assert_eq!(determinant.value, 0.0);
    assert!(determinant.abs_error > 0.0);
    assert_eq!(
        certified_uv_sign(determinant),
        Err(GeometryNumericIssue::UncertainUvDeterminant)
    );
    let witnessed = orient2_f64([0.0, 1.0, 2.0], [0.0, 1.0, 2.0]).unwrap();
    assert!(witnessed.is_exact_zero());
    assert_eq!(
        certified_uv_sign(witnessed),
        Err(GeometryNumericIssue::ExactDegenerateUv)
    );
}

#[test]
fn uniformly_tiny_and_huge_planes_keep_the_analytic_normal() {
    for scale in [ETA, 2.0_f64.powi(-1000), 1.0, 2.0_f64.powi(1000)] {
        let positions = PLANE.map(|row| row.map(|value| value * scale));
        let cross = face_cross_f64(scaled_positions(positions)).unwrap();
        assert_eq!(cross.value, [0.0, 0.0, 0.25]);
        assert_direction(unit_estimate(cross).unwrap().value, [0.0, 0.0, 1.0]);
    }
}

#[test]
fn adjacent_large_coordinates_keep_their_exact_representable_spacing() {
    let base = 2.0_f64.powi(1000);
    let step = 2.0_f64.powi(948);
    let positions = [
        [base, base, base],
        [base + step, base, base],
        [base, base + step, base],
    ];
    let cross = face_cross_f64(scaled_positions(positions)).unwrap();
    assert_eq!(cross.value, [0.0, 0.0, 2.0_f64.powi(-106)]);
    assert_direction(unit_estimate(cross).unwrap().value, [0.0, 0.0, 1.0]);
}

#[test]
fn opposite_extremes_do_not_require_an_overflowing_raw_difference() {
    let positions = [
        [-f64::MAX, 0.0, 0.0],
        [f64::MAX, 0.0, 0.0],
        [0.0, f64::MAX, 0.0],
    ];
    let cross = face_cross_f64(scaled_positions(positions)).unwrap();
    assert_direction(unit_estimate(cross).unwrap().value, [0.0, 0.0, 1.0]);
}

#[test]
fn edge_residuals_force_uncertainty_for_the_analytic_near_collinear_plane() {
    let t = 2.0_f64.powi(-54);
    let d = 2.0_f64.powi(-51);
    let positions = [[t, t, 0.0], [1.0, 1.0, 0.0], [2.0, 2.0 + d, 0.0]];
    let cross = face_cross_f64(scaled_positions(positions)).unwrap();
    assert!(cross.value[2] > 0.0);
    assert!(cross.abs_error[2] > 0.5 * cross.value[2]);
    assert!(matches!(
        unit_estimate(cross),
        Err(GeometryNumericIssue::UncertainDirection)
    ));
}

#[test]
fn compensated_area_sum_retains_a_small_exact_contribution() {
    let mut accumulator = AreaAccumulator3::default();
    for z in [2.0_f64.powi(53), 1.0, -2.0_f64.powi(53)] {
        area_add(
            &mut accumulator,
            Estimate3 {
                value: [0.0, 0.0, z],
                abs_error: [0.0; 3],
            },
        )
        .unwrap();
    }
    let total = area_finish(&accumulator).unwrap();
    assert_eq!(total.value, [0.0, 0.0, 1.0]);
    assert_direction(unit_estimate(total).unwrap().value, [0.0, 0.0, 1.0]);
}

#[test]
fn smooth_sum_keeps_area_weights_in_a_common_coordinate_scale() {
    let first = [[0.0; 3], [0.5, 0.0, 0.0], [0.0, 0.5, 0.0]];
    let second = [[0.0; 3], [0.0, 0.0, 0.25], [0.25, 0.0, 0.0]];
    let mut accumulator = AreaAccumulator3::default();
    area_add(&mut accumulator, face_cross_f64(first).unwrap()).unwrap();
    area_add(&mut accumulator, face_cross_f64(second).unwrap()).unwrap();
    let length = 17.0_f64.sqrt();
    assert_direction(
        unit_estimate(area_finish(&accumulator).unwrap())
            .unwrap()
            .value,
        [0.0, 1.0 / length, 4.0 / length],
    );
}

#[test]
fn uncertain_opposing_faces_never_publish_a_residual_normal() {
    let mut accumulator = AreaAccumulator3::default();
    for z in [0.25, -0.25] {
        area_add(
            &mut accumulator,
            Estimate3 {
                value: [0.0, 0.0, z],
                abs_error: [0.0, 0.0, f64::EPSILON],
            },
        )
        .unwrap();
    }
    assert!(matches!(
        unit_estimate(area_finish(&accumulator).unwrap()),
        Err(GeometryNumericIssue::UncertainDirection)
    ));
}

#[test]
fn unit_vector_handles_uniform_exponent_limits_and_names_ratio_underflow() {
    let expected = std::f64::consts::FRAC_1_SQRT_2;
    for magnitude in [ETA, f64::MAX] {
        assert_direction(
            unit_f64([magnitude, magnitude, 0.0]).unwrap(),
            [expected, expected, 0.0],
        );
    }
    assert_eq!(
        unit_f64([f64::MAX, ETA, 0.0]),
        Err(GeometryNumericIssue::NormalizationUnderflow)
    );
    assert_eq!(unit_f64([0.0; 3]), Err(GeometryNumericIssue::ZeroVector));
}
