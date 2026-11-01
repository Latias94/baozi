use super::{TangentIssue, triangle};

const FACE: usize = 7;
const POSITIONS: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
const NORMALS: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
const UVS: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
const BASIC: [[f32; 4]; 3] = [[1.0, 0.0, 0.0, 1.0]; 3];
const HUGE: f32 = f32::from_bits((100 + 127) << 23);
const PERMUTATIONS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [1, 2, 0],
    [2, 0, 1],
    [0, 2, 1],
    [2, 1, 0],
    [1, 0, 2],
];

fn bits(values: [[f32; 4]; 3]) -> [[u32; 4]; 3] {
    values.map(|value| value.map(f32::to_bits))
}

fn assert_basis(values: [[f32; 4]; 3], normals: [[f32; 3]; 3], v_derivative: [f64; 3]) {
    let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    for (value, normal) in values.into_iter().zip(normals) {
        assert!(value.iter().all(|value| value.is_finite()));
        assert!(value[3] == 1.0 || value[3] == -1.0);
        let normal = normal.map(f64::from);
        let length = dot(normal, normal).sqrt();
        let normal = normal.map(|value| value / length);
        let tangent = [value[0], value[1], value[2]].map(f64::from);
        let handedness = f64::from(value[3]);
        let bitangent = [
            normal[1] * tangent[2] - normal[2] * tangent[1],
            normal[2] * tangent[0] - normal[0] * tangent[2],
            normal[0] * tangent[1] - normal[1] * tangent[0],
        ]
        .map(|component| component * handedness);
        assert!((dot(tangent, tangent) - 1.0).abs() < 1.0e-6);
        assert!((dot(bitangent, bitangent) - 1.0).abs() < 1.0e-6);
        assert!(dot(normal, tangent).abs() < 1.0e-6);
        assert!(dot(normal, bitangent).abs() < 1.0e-6);
        assert!(dot(tangent, bitangent).abs() < 1.0e-6);
        // These fixtures have well-conditioned V alignment. It is not a general
        // postcondition of the rounded frame; the analytic-W fixture below differs.
        assert!(dot(bitangent, v_derivative) > 0.0);
    }
}

#[test]
fn basic_triangle_has_the_expected_encoding_and_positive_zero_bits() {
    let values = triangle(FACE, POSITIONS, NORMALS, UVS).expect("the basic triangle is valid");
    assert_eq!(bits(values), bits(BASIC));
    assert_basis(values, NORMALS, [0.0, 1.0, 0.0]);
}

#[test]
fn mirrored_u_and_v_have_independently_checkable_signs() {
    let cases = [
        (
            [[0.0, 0.0], [-1.0, 0.0], [0.0, 1.0]],
            [-1.0, 0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
        ),
        (
            [[0.0, 0.0], [1.0, 0.0], [0.0, -1.0]],
            [1.0, 0.0, 0.0, -1.0],
            [0.0, -1.0, 0.0],
        ),
    ];
    for (uvs, expected, v_derivative) in cases {
        let values = triangle(FACE, POSITIONS, NORMALS, uvs).expect("UV mirroring is supported");
        assert_eq!(bits(values), bits([expected; 3]));
        assert_basis(values, NORMALS, v_derivative);
    }
}

#[test]
fn reversing_the_normal_changes_w_without_reversing_u() {
    let normals = [[0.0, 0.0, -1.0]; 3];
    let values = triangle(FACE, POSITIONS, normals, UVS).expect("reverse normals are supported");
    assert_eq!(bits(values), bits([[1.0, 0.0, 0.0, -1.0]; 3]));
    assert_basis(values, normals, [0.0, 1.0, 0.0]);
}

#[test]
fn each_corner_uses_its_own_normal_without_changing_output_order() {
    let normals = [[0.0, 0.0, 2.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]];
    let values = triangle(FACE, POSITIONS, normals, UVS).expect("shading normals are supported");
    assert_eq!(bits(values)[0], bits(BASIC)[0]);
    assert_eq!(bits(values)[1], [0x3f35_04f3, 0, 0xbf35_04f3, 0x3f80_0000]);
    assert_eq!(bits(values)[2], bits(BASIC)[0]);
    assert_basis(values, normals, [0.0, 1.0, 0.0]);
}

#[test]
fn skew_uvs_and_shading_normals_reconstruct_a_unit_orthogonal_frame() {
    let positions = [[0.0, 0.0, 0.0], [2.0, 0.0, 1.0], [0.0, 3.0, 0.0]];
    let normals = [[0.0, 0.0, 1.0], [1.0, 0.0, 2.0], [0.0, 1.0, 2.0]];
    let uvs = [[0.0, 0.0], [2.0, 0.0], [1.0, 3.0]];
    let values = triangle(FACE, positions, normals, uvs).expect("the skew UV map is valid");
    // Directly solving the two UV equations gives dP/dv = (-1/3, 1, -1/6).
    assert_basis(values, normals, [-1.0 / 3.0, 1.0, -1.0 / 6.0]);
}

#[test]
fn shared_positions_do_not_average_a_mirrored_corner_seam() {
    let first = triangle(0, POSITIONS, NORMALS, UVS).expect("the first face is valid");
    let positions = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]];
    let uvs = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]];
    let second = triangle(1, positions, NORMALS, uvs).expect("the mirrored face is valid");
    assert_eq!(bits(first), bits(BASIC));
    assert_eq!(bits(second), bits([[-1.0, 0.0, 0.0, -1.0]; 3]));
}

#[test]
fn continuous_uvs_still_use_each_triangles_derivative() {
    let first = triangle(0, POSITIONS, NORMALS, UVS).expect("the first face is valid");
    let positions = [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
    let uvs = [[1.0, 0.0], [1.0, 2.0], [0.0, 1.0]];
    let second = triangle(1, positions, NORMALS, uvs).expect("the second face is valid");
    assert_eq!(bits(first), bits(BASIC));
    // (2, -1, 0) / sqrt(5), without a cross-face smoothing step.
    assert_eq!(
        bits(second),
        [[0x3f64_f92e, 0xbee4_f92e, 0, 0x3f80_0000]; 3]
    );
}

#[test]
fn extreme_uv_cancellation_retains_the_exact_nonzero_determinant() {
    let uvs = [[HUGE, HUGE], [-HUGE, -HUGE], [1.0, 0.0]];
    let values = triangle(FACE, POSITIONS, NORMALS, uvs).expect("D is exactly 2 * 2^100");
    // Tu is proportional to (-1, 2, 0), and W is positive.
    assert_eq!(
        bits(values),
        [[0xbee4_f92e, 0x3f64_f92e, 0, 0x3f80_0000]; 3]
    );
}

#[test]
fn analytic_handedness_does_not_flip_to_match_a_rounded_v_dot_product() {
    let uvs = [
        [0.0, 0.0],
        [1_048_576.0, 1_048_575.0],
        [1_048_575.0, 1_048_574.0],
    ];
    let values = triangle(FACE, POSITIONS, NORMALS, uvs).expect("nonzero D is exactly -1");
    assert_eq!(
        bits(values),
        [[0xbf35_04ee, 0x3f35_04f9, 0, 0xbf80_0000]; 3]
    );
    // Exactly Tu=(-1048574,1048575,0), Tv=(1048575,-1048576,0).
    // The analytic orientation is negative. F32 rounding changes the nearly
    // cancelling reconstructed-B dot raw-Tv sign, without changing that W.
    for tangent in values {
        let [x, y, z, w] = tangent.map(f64::from);
        assert!((x * x + y * y + z * z - 1.0).abs() <= super::OUTPUT_EPSILON);
        assert_eq!(z, 0.0);
        assert_eq!(w, -1.0);
        let bitangent = [-y * w, x * w, 0.0];
        let rounded_dot_raw_v = bitangent[0] * 1_048_575.0 - bitangent[1] * 1_048_576.0;
        assert_eq!(rounded_dot_raw_v, -328_953.0 / 16_777_216.0);
    }
}

#[test]
fn extreme_position_cancellation_retains_area_and_handedness() {
    let positions = [[HUGE, HUGE, 0.0], [-HUGE, -HUGE, 0.0], [1.0, 0.0, 0.0]];
    let values = triangle(FACE, positions, NORMALS, UVS).expect("G.z is exactly 2 * 2^100");
    assert_eq!(
        bits(values),
        [[0xbf35_04f3, 0xbf35_04f3, 0, 0x3f80_0000]; 3]
    );
}

#[test]
fn all_face_permutations_preserve_bits_and_the_original_corner_mapping() {
    let uvs = [[HUGE, HUGE], [-HUGE, -HUGE], [1.0, 0.0]];
    let normals = [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]];
    let expected = triangle(FACE, POSITIONS, normals, uvs).expect("the baseline is valid");
    for permutation in PERMUTATIONS {
        let values = triangle(
            FACE,
            permutation.map(|corner| POSITIONS[corner]),
            permutation.map(|corner| normals[corner]),
            permutation.map(|corner| uvs[corner]),
        )
        .expect("cyclic and reversed face orders preserve the same derivatives");
        assert_eq!(
            bits(values),
            bits(permutation.map(|corner| expected[corner]))
        );
    }
}

#[test]
fn finite_f32_exponent_extremes_need_no_area_or_uv_epsilon_cutoff() {
    for scale in [f32::from_bits(1), f32::MIN_POSITIVE, 1.0, f32::MAX] {
        let positions = [[0.0, 0.0, 0.0], [scale, 0.0, 0.0], [0.0, scale, 0.0]];
        let normals = [[0.0, 0.0, scale]; 3];
        let uvs = [[0.0, 0.0], [scale, 0.0], [0.0, scale]];
        let values = triangle(FACE, positions, normals, uvs).expect("all products fit in F64");
        assert_eq!(bits(values), bits(BASIC));
        assert_basis(values, normals, [0.0, 1.0, 0.0]);
    }
}

#[test]
fn signed_input_zeros_produce_only_positive_output_zeros() {
    let positions = [[-0.0, 0.0, -0.0], [1.0, -0.0, 0.0], [-0.0, 1.0, -0.0]];
    let normals = [[-0.0, 0.0, 1.0]; 3];
    let uvs = [[-0.0, 0.0], [1.0, -0.0], [-0.0, 1.0]];
    let values = triangle(FACE, positions, normals, uvs).expect("signed zeros are finite");
    assert_eq!(bits(values), bits(BASIC));
}

#[test]
fn collinear_and_repeated_positions_are_rejected_under_every_permutation() {
    let cases = [
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        [POSITIONS[0], POSITIONS[0], POSITIONS[2]],
        [[HUGE, HUGE, 0.0], [-HUGE, -HUGE, 0.0], [0.0, 0.0, 0.0]],
    ];
    for positions in cases {
        for permutation in PERMUTATIONS {
            assert_eq!(
                triangle(
                    FACE,
                    permutation.map(|corner| positions[corner]),
                    NORMALS,
                    UVS
                ),
                Err(TangentIssue::DegenerateFace { face: FACE }),
            );
        }
    }
}

#[test]
fn collinear_uvs_and_constant_broadcast_uvs_are_rejected() {
    // A hypothetical Face UV broadcast is a kernel-only fixture. Public domain
    // selection admits Vertex and Corner and rejects Face before kernel entry.
    let cases = [
        [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]],
        [[2.0, 3.0]; 3],
        [[HUGE, HUGE], [-HUGE, -HUGE], [0.0, 0.0]],
    ];
    for uvs in cases {
        for permutation in PERMUTATIONS {
            assert_eq!(
                triangle(
                    FACE,
                    POSITIONS,
                    NORMALS,
                    permutation.map(|corner| uvs[corner])
                ),
                Err(TangentIssue::DegenerateUv { face: FACE }),
            );
        }
    }
}

#[test]
fn zero_normals_report_original_global_corners() {
    for local in 0..3 {
        let mut normals = NORMALS;
        normals[local] = [0.0, -0.0, 0.0];
        assert_eq!(
            triangle(FACE, POSITIONS, normals, UVS),
            Err(TangentIssue::ZeroNormal {
                corner: FACE * 3 + local,
            }),
        );
    }
}

#[test]
fn relative_frame_threshold_is_inclusive_and_scale_independent() {
    let threshold = 64.0 * f32::EPSILON;
    for z in [0.0, threshold * 0.5, threshold] {
        for scale in [1.0, 1024.0] {
            let mut normals = NORMALS;
            normals[1] = [scale, 0.0, scale * z];
            assert_eq!(
                triangle(FACE, POSITIONS, normals, UVS),
                Err(TangentIssue::DegenerateFrame {
                    corner: FACE * 3 + 1,
                }),
            );
        }
    }
    for z in [threshold.next_up(), threshold * 2.0] {
        let normals = [[1.0, 0.0, z]; 3];
        let values =
            triangle(FACE, POSITIONS, normals, UVS).expect("both relative tests exceed tau");
        assert_basis(values, normals, [0.0, 1.0, 0.0]);
    }
}

#[test]
fn non_axis_aligned_ill_conditioned_projection_is_rejected() {
    let normals = [[1.0, 2.0, 3.0]; 3];
    for exponent in [16, 43] {
        let delta = f32::from_bits((127 - exponent) << 23);
        let positions = [[delta, 0.0, 0.0], [1.0, 2.0, 3.0], [0.0, 1.0, 0.0]];
        assert_eq!(
            triangle(FACE, positions, normals, UVS),
            Err(TangentIssue::DegenerateFrame { corner: FACE * 3 }),
        );
    }
}

#[test]
fn admitted_non_axis_aligned_projections_match_the_analytic_direction() {
    let normals = [[1.0, 2.0, 3.0]; 3];
    let reference = [-13.0, 2.0, 3.0].map(|value| value / 182.0_f64.sqrt());
    for exponent in [13, 14, 15] {
        let delta = f32::from_bits((127 - exponent) << 23);
        let positions = [[delta, 0.0, 0.0], [1.0, 2.0, 3.0], [0.0, 1.0, 0.0]];
        let values =
            triangle(FACE, positions, normals, UVS).expect("both relative tests exceed tau");
        assert_basis(values, normals, [0.0, 1.0, 0.0]);
        // Exact Gram-Schmidt is delta/14 * (-13,2,3), independently of delta.
        for tangent in values {
            assert_eq!(tangent[3], -1.0);
            let [x, y, z] = [tangent[0], tangent[1], tangent[2]].map(f64::from);
            let perpendicular = [
                y * reference[2] - z * reference[1],
                z * reference[0] - x * reference[2],
                x * reference[1] - y * reference[0],
            ];
            let sine = perpendicular
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            let cosine = x * reference[0] + y * reference[1] + z * reference[2];
            let angular_error = sine.atan2(cosine);
            assert!(
                angular_error <= 8.0 * f64::from(f32::EPSILON),
                "delta=2^-{exponent}, angular error {angular_error} rad",
            );
        }
    }
}

#[test]
fn an_in_plane_normal_is_rejected_even_with_a_nonzero_projected_u() {
    let mut normals = NORMALS;
    normals[2] = [0.0, 1.0, 0.0];
    assert_eq!(
        triangle(FACE, POSITIONS, normals, UVS),
        Err(TangentIssue::DegenerateFrame {
            corner: FACE * 3 + 2,
        }),
    );
}

#[test]
fn mixed_normal_orientations_reject_the_whole_face() {
    let normals = [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [0.0, 0.0, 1.0]];
    assert_eq!(
        triangle(FACE, POSITIONS, normals, UVS),
        Err(TangentIssue::MixedHandedness { face: FACE }),
    );
}

#[test]
fn nonfinite_values_in_each_input_report_the_original_corner() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for local in 0..3 {
            let expected = Err(TangentIssue::NonFiniteInput {
                corner: FACE * 3 + local,
            });
            let mut positions = POSITIONS;
            positions[local][0] = invalid;
            assert_eq!(triangle(FACE, positions, NORMALS, UVS), expected);
            let mut normals = NORMALS;
            normals[local][1] = invalid;
            assert_eq!(triangle(FACE, POSITIONS, normals, UVS), expected);
            let mut uvs = UVS;
            uvs[local][1] = invalid;
            assert_eq!(triangle(FACE, POSITIONS, NORMALS, uvs), expected);
        }
    }
}

#[test]
fn the_largest_admitted_face_keeps_corner_locations_in_range() {
    let face = (usize::MAX - 2) / 3;
    let mut normals = NORMALS;
    normals[2] = [0.0; 3];
    assert_eq!(
        triangle(face, POSITIONS, normals, UVS),
        Err(TangentIssue::ZeroNormal {
            corner: face * 3 + 2
        }),
    );
}

#[test]
fn success_and_rejection_paths_allocate_nothing() {
    use std::hint::black_box;

    let huge_uvs = [[HUGE, HUGE], [-HUGE, -HUGE], [1.0, 0.0]];
    let mut zero_normals = NORMALS;
    zero_normals[1] = [0.0; 3];
    let mut mixed_normals = NORMALS;
    mixed_normals[1] = [0.0, 0.0, -1.0];
    let mut in_plane_normals = NORMALS;
    in_plane_normals[1] = [0.0, 1.0, 0.0];
    let mut invalid_uvs = UVS;
    invalid_uvs[2][1] = f32::NAN;
    let ill_conditioned = f32::from_bits((127 - 43) << 23);
    let admitted = f32::from_bits((127 - 15) << 23);
    let non_axis_aligned = |delta| [[delta, 0.0, 0.0], [1.0, 2.0, 3.0], [0.0, 1.0, 0.0]];
    let cases = [
        (POSITIONS, NORMALS, UVS),
        (POSITIONS, NORMALS, huge_uvs),
        ([POSITIONS[0]; 3], NORMALS, UVS),
        (POSITIONS, NORMALS, [[2.0, 3.0]; 3]),
        (POSITIONS, zero_normals, UVS),
        (POSITIONS, mixed_normals, UVS),
        (POSITIONS, in_plane_normals, UVS),
        (POSITIONS, NORMALS, invalid_uvs),
        (
            POSITIONS,
            NORMALS,
            [
                [0.0, 0.0],
                [1_048_576.0, 1_048_575.0],
                [1_048_575.0, 1_048_574.0],
            ],
        ),
        (non_axis_aligned(ill_conditioned), [[1.0, 2.0, 3.0]; 3], UVS),
        (non_axis_aligned(admitted), [[1.0, 2.0, 3.0]; 3], UVS),
    ];
    let allocations = allocation_counter::measure(|| {
        for case in cases {
            let (positions, normals, uvs) = black_box(case);
            let _ = black_box(triangle(black_box(FACE), positions, normals, uvs));
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}
