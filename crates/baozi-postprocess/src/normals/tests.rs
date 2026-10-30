use std::convert::Infallible;

use super::{Buffer, Error, Input, Mode, VertexIds, Workspace, face_cross, generate, unit};

const POSITIONS: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
];
const IDS: [u32; 6] = [0, 1, 2, 0, 3, 1];
const Z: [f32; 3] = [0.0, 0.0, 1.0];
const Y: [f32; 3] = [0.0, 1.0, 0.0];
const YZ: [f32; 3] = [
    0.0,
    std::f32::consts::FRAC_1_SQRT_2,
    std::f32::consts::FRAC_1_SQRT_2,
];

fn indexed<'a>(positions: &'a [[f32; 3]], ids: &'a [u32], groups: Option<&'a [u32]>) -> Input<'a> {
    Input {
        positions,
        vertex_ids: VertexIds::Indexed(ids),
        smoothing_groups: groups,
    }
}

fn run(input: Input<'_>, mode: Mode, cached: bool) -> Result<Vec<[f32; 3]>, Error<Infallible>> {
    let corners = input.corner_count();
    let mut output = vec![[f32::NAN; 3]; corners];
    let mut order = vec![usize::MAX; if mode == Mode::Smooth { corners } else { 0 }];
    let mut crosses = cached.then(|| vec![[f64::NAN; 3]; corners / 3]);
    generate(
        input,
        mode,
        &mut output,
        Workspace {
            corner_order: &mut order,
            face_crosses: crosses.as_deref_mut(),
        },
        || Ok::<(), Infallible>(()),
    )?;
    Ok(output)
}

fn assert_normal(actual: [f32; 3], expected: [f32; 3]) {
    for axis in 0..3 {
        assert!(actual[axis].is_finite(), "{actual:?}");
        assert!(
            (actual[axis] - expected[axis]).abs() < 2.0e-6,
            "normal {actual:?}, expected {expected:?}"
        );
    }
    let length_squared: f64 = actual.iter().map(|&v| f64::from(v).powi(2)).sum();
    assert!((length_squared - 1.0).abs() < 2.0e-6, "{actual:?}");
}

fn assert_normals(actual: &[[f32; 3]], expected: &[[f32; 3]]) {
    assert_eq!(actual.len(), expected.len());
    for (&actual, &expected) in actual.iter().zip(expected) {
        assert_normal(actual, expected);
    }
}

#[test]
fn winding_controls_flat_and_smooth_orientation() {
    let positions = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 1.0]];
    let forward = [0, 1, 2];
    let reverse = [0, 2, 1];
    let diagonal = std::f32::consts::FRAC_1_SQRT_2;
    for (ids, expected) in [
        (&forward, [0.0, -diagonal, diagonal]),
        (&reverse, [0.0, diagonal, -diagonal]),
    ] {
        for mode in [Mode::Flat, Mode::Smooth] {
            for cached in [false, true] {
                let normals = run(indexed(&positions, ids, None), mode, cached).unwrap();
                assert_normals(&normals, &[expected; 3]);
            }
        }
    }
}

#[test]
fn flat_and_smooth_differ_at_a_shared_fold() {
    for cached in [false, true] {
        let input = indexed(&POSITIONS, &IDS, None);
        assert_normals(
            &run(input, Mode::Flat, cached).unwrap(),
            &[Z, Z, Z, Y, Y, Y],
        );
        assert_normals(
            &run(input, Mode::Smooth, cached).unwrap(),
            &[YZ, YZ, Z, YZ, Y, YZ],
        );
    }
}

#[test]
fn smooth_normals_weight_unnormalized_face_areas() {
    let mut positions = POSITIONS;
    positions[3][2] = 2.0;
    let shared = [0.0, 2.0 / 5.0_f32.sqrt(), 1.0 / 5.0_f32.sqrt()];
    for cached in [false, true] {
        assert_normals(
            &run(indexed(&positions, &IDS, None), Mode::Smooth, cached).unwrap(),
            &[shared, shared, Z, shared, Y, shared],
        );
    }
}

#[test]
fn group_zero_is_hard_even_between_two_zero_group_faces() {
    for cached in [false, true] {
        assert_normals(
            &run(
                indexed(&POSITIONS, &IDS, Some(&[0, 0])),
                Mode::Smooth,
                cached,
            )
            .unwrap(),
            &[Z, Z, Z, Y, Y, Y],
        );
    }
}

#[test]
fn equal_nonzero_groups_smooth_shared_identities() {
    for group in [1, 7, u32::MAX] {
        for cached in [false, true] {
            assert_normals(
                &run(
                    indexed(&POSITIONS, &IDS, Some(&[group, group])),
                    Mode::Smooth,
                    cached,
                )
                .unwrap(),
                &[YZ, YZ, Z, YZ, Y, YZ],
            );
        }
    }
}

#[test]
fn differing_groups_and_mixed_hard_groups_do_not_mix() {
    for groups in [[7, 9], [0, 7], [7, 0], [1, u32::MAX]] {
        for cached in [false, true] {
            assert_normals(
                &run(
                    indexed(&POSITIONS, &IDS, Some(&groups)),
                    Mode::Smooth,
                    cached,
                )
                .unwrap(),
                &[Z, Z, Z, Y, Y, Y],
            );
        }
    }
}

#[test]
fn interleaved_faces_merge_only_the_same_group_and_vertex() {
    let ids = [0, 1, 2, 0, 3, 1, 0, 2, 3];
    let x = [1.0, 0.0, 0.0];
    let xz = [
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    for cached in [false, true] {
        assert_normals(
            &run(
                indexed(&POSITIONS, &ids, Some(&[5, 6, 5])),
                Mode::Smooth,
                cached,
            )
            .unwrap(),
            &[xz, Z, xz, Y, Y, Y, xz, xz, x],
        );
    }
}

#[test]
fn a_corner_uv_seam_keeps_shared_vertex_normal_smoothing() {
    // These two faces have distinct Corner UVs at the same vertex identities.
    let corner_uvs = [
        [0.0, 0.0],
        [1.0, 0.0],
        [0.0, 1.0],
        [0.5, 0.5],
        [1.0, 1.0],
        [0.0, 0.5],
    ];
    assert_eq!(IDS[0], IDS[3]);
    assert_eq!(IDS[1], IDS[5]);
    assert_ne!(corner_uvs[0], corner_uvs[3]);
    assert_ne!(corner_uvs[1], corner_uvs[5]);
    for cached in [false, true] {
        let normals = run(indexed(&POSITIONS, &IDS, None), Mode::Smooth, cached).unwrap();
        assert_normal(normals[0], YZ);
        assert_normal(normals[3], YZ);
        assert_normal(normals[1], YZ);
        assert_normal(normals[5], YZ);
    }
}

#[test]
fn coincident_positions_with_different_ids_are_not_welded() {
    let positions = [
        POSITIONS[0],
        POSITIONS[1],
        POSITIONS[2],
        POSITIONS[0],
        POSITIONS[3],
        POSITIONS[1],
    ];
    let ids = [0, 1, 2, 3, 4, 5];
    assert_eq!(positions[0], positions[3]);
    for cached in [false, true] {
        assert_normals(
            &run(indexed(&positions, &ids, None), Mode::Smooth, cached).unwrap(),
            &[Z, Z, Z, Y, Y, Y],
        );
    }
}

#[test]
fn nonindexed_corners_have_distinct_vertex_identity() {
    let positions = [
        POSITIONS[0],
        POSITIONS[1],
        POSITIONS[2],
        POSITIONS[0],
        POSITIONS[3],
        POSITIONS[1],
    ];
    for groups in [None, Some([7, 7].as_slice())] {
        let input = Input {
            positions: &positions,
            vertex_ids: VertexIds::NonIndexed,
            smoothing_groups: groups,
        };
        for mode in [Mode::Flat, Mode::Smooth] {
            for cached in [false, true] {
                assert_normals(&run(input, mode, cached).unwrap(), &[Z, Z, Z, Y, Y, Y]);
            }
        }
    }
}

#[test]
fn source_position_bits_ids_and_groups_are_preserved() {
    let mut positions = POSITIONS;
    positions[0] = [-0.0, 0.0, -0.0];
    let ids = IDS;
    let groups = [7, 7];
    let position_bits = positions.map(|position| position.map(f32::to_bits));
    for mode in [Mode::Flat, Mode::Smooth] {
        for cached in [false, true] {
            run(indexed(&positions, &ids, Some(&groups)), mode, cached).unwrap();
            assert_eq!(positions.map(|p| p.map(f32::to_bits)), position_bits);
            assert_eq!(ids, IDS);
            assert_eq!(groups, [7, 7]);
        }
    }
}

#[test]
fn empty_triangle_lists_succeed() {
    for input in [
        indexed(&[], &[], None),
        indexed(&POSITIONS, &[], Some(&[])),
        Input {
            positions: &[],
            vertex_ids: VertexIds::NonIndexed,
            smoothing_groups: None,
        },
    ] {
        for mode in [Mode::Flat, Mode::Smooth] {
            for cached in [false, true] {
                assert!(run(input, mode, cached).unwrap().is_empty());
            }
        }
    }
}

#[test]
fn malformed_triangle_cardinality_is_rejected() {
    for count in [1, 2, 4, 5] {
        for input in [
            indexed(&POSITIONS, &IDS[..count], None),
            Input {
                positions: &[[0.0; 3]; 5][..count],
                vertex_ids: VertexIds::NonIndexed,
                smoothing_groups: None,
            },
        ] {
            for mode in [Mode::Flat, Mode::Smooth] {
                assert_eq!(
                    run(input, mode, false),
                    Err(Error::CornerCount { actual: count })
                );
            }
        }
    }
}

#[test]
fn out_of_bounds_ids_report_original_corner_and_vertex() {
    for corner in 0..IDS.len() {
        for bad_id in [POSITIONS.len() as u32, u32::MAX] {
            let mut ids = IDS;
            ids[corner] = bad_id;
            for mode in [Mode::Flat, Mode::Smooth] {
                for cached in [false, true] {
                    assert_eq!(
                        run(indexed(&POSITIONS, &ids, None), mode, cached),
                        Err(Error::VertexOutOfBounds {
                            corner,
                            vertex: bad_id as usize,
                            vertex_count: POSITIONS.len(),
                        })
                    );
                }
            }
        }
    }
    assert_eq!(
        run(indexed(&[], &[0, 1, 2], None), Mode::Smooth, false),
        Err(Error::VertexOutOfBounds {
            corner: 0,
            vertex: 0,
            vertex_count: 0,
        })
    );
}

#[test]
fn nonfinite_positions_are_rejected_even_when_unreferenced() {
    let base = [
        POSITIONS[0],
        POSITIONS[1],
        POSITIONS[2],
        POSITIONS[3],
        [12.0, 13.0, 14.0],
    ];
    for vertex in 0..base.len() {
        for component in 0..3 {
            for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut positions = base;
                positions[vertex][component] = invalid;
                for mode in [Mode::Flat, Mode::Smooth] {
                    for cached in [false, true] {
                        assert_eq!(
                            run(indexed(&positions, &IDS, None), mode, cached),
                            Err(Error::NonFinitePosition { vertex, component })
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn repeated_collinear_and_coincident_triangles_are_rejected() {
    let collinear = [[0.0; 3], [1.0, 2.0, 3.0], [2.0, 4.0, 6.0]];
    let coincident = [[0.0; 3], [0.0; 3], [1.0, 0.0, 0.0]];
    for input in [
        indexed(&POSITIONS, &[0, 0, 1], None),
        indexed(&collinear, &[0, 1, 2], None),
        indexed(&coincident, &[0, 1, 2], None),
    ] {
        for mode in [Mode::Flat, Mode::Smooth] {
            for cached in [false, true] {
                assert_eq!(
                    run(input, mode, cached),
                    Err(Error::DegenerateFace { face: 0 })
                );
            }
        }
    }
    for mode in [Mode::Flat, Mode::Smooth] {
        assert_eq!(
            run(indexed(&POSITIONS, &[0, 1, 2, 0, 0, 1], None), mode, false,),
            Err(Error::DegenerateFace { face: 1 })
        );
    }
}

#[test]
fn cancelling_face_sums_are_errors_without_a_fallback_normal() {
    let ids = [0, 1, 2, 0, 2, 1];
    for groups in [None, Some([9, 9].as_slice())] {
        for cached in [false, true] {
            let mut output = [[73.0; 3]; 6];
            let mut order = [usize::MAX; 6];
            let mut crosses = [[f64::NAN; 3]; 2];
            assert_eq!(
                generate(
                    indexed(&POSITIONS, &ids, groups),
                    Mode::Smooth,
                    &mut output,
                    Workspace {
                        corner_order: &mut order,
                        face_crosses: cached.then_some(&mut crosses[..]),
                    },
                    || Ok::<(), Infallible>(()),
                ),
                Err(Error::InvalidSmoothSum {
                    vertex: 0,
                    group: groups.map(|g| g[0]),
                    corner: 0,
                })
            );
            assert_eq!(output, [[73.0; 3]; 6]);
        }
    }
    for groups in [None, Some([0, 0].as_slice()), Some([3, 4].as_slice())] {
        for cached in [false, true] {
            let input = indexed(&POSITIONS, &ids, groups);
            let expected = [
                Z,
                Z,
                Z,
                [0.0, 0.0, -1.0],
                [0.0, 0.0, -1.0],
                [0.0, 0.0, -1.0],
            ];
            assert_normals(&run(input, Mode::Flat, cached).unwrap(), &expected);
            if groups.is_some() {
                assert_normals(&run(input, Mode::Smooth, cached).unwrap(), &expected);
            }
        }
    }
}

#[test]
fn group_cardinality_is_exact_in_both_modes() {
    for count in [0, 1, 3] {
        let groups = vec![7; count];
        for mode in [Mode::Flat, Mode::Smooth] {
            assert_eq!(
                run(indexed(&POSITIONS, &IDS, Some(&groups)), mode, false),
                Err(Error::GroupCount {
                    expected: 2,
                    actual: count,
                })
            );
        }
    }
}

#[test]
fn output_and_workspace_shapes_are_exact() {
    for mode in [Mode::Flat, Mode::Smooth] {
        let expected_order = if mode == Mode::Smooth { 6 } else { 0 };
        let mut cases = vec![
            (Buffer::Output, 5),
            (Buffer::Output, 7),
            (Buffer::FaceCrosses, 0),
            (Buffer::FaceCrosses, 1),
            (Buffer::FaceCrosses, 3),
            (Buffer::CornerOrder, expected_order + 1),
        ];
        if mode == Mode::Smooth {
            cases.extend([(Buffer::CornerOrder, 0), (Buffer::CornerOrder, 5)]);
        }
        for (buffer, actual) in cases {
            let output_count = if buffer == Buffer::Output { actual } else { 6 };
            let order_count = if buffer == Buffer::CornerOrder {
                actual
            } else {
                expected_order
            };
            let cross_count = if buffer == Buffer::FaceCrosses {
                actual
            } else {
                2
            };
            let mut output = vec![[73.0; 3]; output_count];
            let mut order = vec![usize::MAX; order_count];
            let mut crosses = vec![[73.0; 3]; cross_count];
            assert_eq!(
                generate(
                    indexed(&POSITIONS, &IDS, None),
                    mode,
                    &mut output,
                    Workspace {
                        corner_order: &mut order,
                        face_crosses: Some(&mut crosses),
                    },
                    || Ok::<(), Infallible>(()),
                ),
                Err(Error::BufferLength {
                    buffer,
                    expected: match buffer {
                        Buffer::Output => 6,
                        Buffer::CornerOrder => expected_order,
                        Buffer::FaceCrosses => 2,
                    },
                    actual,
                })
            );
            assert_eq!(output, vec![[73.0; 3]; output_count]);
            assert_eq!(order, vec![usize::MAX; order_count]);
            assert_eq!(crosses, vec![[73.0; 3]; cross_count]);
        }
    }
}

#[test]
fn finite_extreme_triangles_do_not_overflow_or_underflow_f32_edges() {
    let max = f32::MAX;
    let min = f32::from_bits(1);
    let diagonal = std::f32::consts::FRAC_1_SQRT_2;
    for (positions, expected) in [
        ([[-max, -max, 0.0], [max, -max, 0.0], [-max, max, 0.0]], Z),
        (
            [[-max, -max, -max], [max, -max, -max], [-max, max, max]],
            [0.0, -diagonal, diagonal],
        ),
        ([[0.0; 3], [min, 0.0, 0.0], [0.0, min, 0.0]], Z),
        (
            [
                [0.0; 3],
                [f32::MIN_POSITIVE, 0.0, 0.0],
                [0.0, f32::MIN_POSITIVE, 0.0],
            ],
            Z,
        ),
    ] {
        for mode in [Mode::Flat, Mode::Smooth] {
            for cached in [false, true] {
                assert_normals(
                    &run(indexed(&positions, &[0, 1, 2], None), mode, cached).unwrap(),
                    &[expected; 3],
                );
            }
        }
    }
}

#[test]
fn widely_separated_coordinates_preserve_direction_under_cyclic_permutation() {
    for exponent in [-30, -60, -100, -149] {
        let epsilon = 2.0_f64.powi(exponent) as f32;
        let positions = [
            [1.0, 1.0, 0.0],
            [epsilon, 0.0, epsilon],
            [0.0, epsilon, 0.0],
        ];
        // Exact cross is epsilon * (1 - epsilon, -1, epsilon - 2).
        let epsilon = f64::from(epsilon);
        let forward = unit([1.0 - epsilon, -1.0, epsilon - 2.0]).unwrap();
        for (ids, expected) in [
            ([0, 1, 2], forward),
            ([1, 2, 0], forward),
            ([2, 0, 1], forward),
            ([0, 2, 1], forward.map(|value| -value)),
            ([2, 1, 0], forward.map(|value| -value)),
            ([1, 0, 2], forward.map(|value| -value)),
        ] {
            for mode in [Mode::Flat, Mode::Smooth] {
                for cached in [false, true] {
                    assert_normals(
                        &run(indexed(&positions, &ids, None), mode, cached).unwrap(),
                        &[expected; 3],
                    );
                }
            }
        }
    }
}

#[test]
fn exact_collinearity_survives_widely_separated_coordinates() {
    let epsilon = 2.0_f32.powi(-60);
    let positions = [[1.0; 3], [epsilon; 3], [0.0; 3]];
    for ids in [[0, 1, 2], [1, 2, 0], [2, 0, 1]] {
        for mode in [Mode::Flat, Mode::Smooth] {
            for cached in [false, true] {
                assert_eq!(
                    run(indexed(&positions, &ids, None), mode, cached),
                    Err(Error::DegenerateFace { face: 0 })
                );
            }
        }
    }
}

#[test]
fn opposite_face_permutations_cancel_without_estimate_rounding_noise() {
    let positions = [
        [
            f32::from_bits(0xbc40_0000),
            f32::from_bits(0xdcf0_0000),
            0.0,
        ],
        [
            f32::from_bits(0x5580_0000),
            f32::from_bits(0x28a0_0000),
            0.0,
        ],
        [
            f32::from_bits(0xdd80_0000),
            f32::from_bits(0xca00_0000),
            0.0,
        ],
    ];
    let expected = face_cross::<Infallible>(indexed(&positions, &[0, 1, 2], None), 0).unwrap();
    for (ids, expected) in [
        ([0, 1, 2], expected),
        ([1, 2, 0], expected),
        ([2, 0, 1], expected),
        ([0, 2, 1], expected.map(|value| -value)),
        ([2, 1, 0], expected.map(|value| -value)),
        ([1, 0, 2], expected.map(|value| -value)),
    ] {
        let actual = face_cross::<Infallible>(indexed(&positions, &ids, None), 0).unwrap();
        assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
    }
    let ids = [1, 2, 0, 0, 2, 1];
    for groups in [None, Some([9, 9].as_slice())] {
        for cached in [false, true] {
            assert_eq!(
                run(indexed(&positions, &ids, groups), Mode::Smooth, cached),
                Err(Error::InvalidSmoothSum {
                    vertex: 0,
                    group: groups.map(|groups| groups[0]),
                    corner: 2,
                })
            );
        }
    }
}

#[test]
fn smoothing_handles_extreme_finite_face_areas() {
    for scale in [f32::MAX, f32::MIN_POSITIVE, f32::from_bits(1)] {
        let positions = [
            [0.0; 3],
            [scale, 0.0, 0.0],
            [0.0, scale, 0.0],
            [0.0, 0.0, scale],
        ];
        for cached in [false, true] {
            assert_normals(
                &run(indexed(&positions, &IDS, None), Mode::Smooth, cached).unwrap(),
                &[YZ, YZ, Z, YZ, Y, YZ],
            );
        }
    }
}

#[test]
fn small_face_survives_large_opposing_area_contributions() {
    for small in [1.0, f32::from_bits(1)] {
        let large = f32::MAX;
        let positions = [
            [0.0; 3],
            [large, 0.0, 0.0],
            [0.0, large, 0.0],
            [small, 0.0, 0.0],
            [0.0, small, 0.0],
            [0.0, large, 0.0],
            [large, 0.0, 0.0],
        ];
        let ids = [0, 1, 2, 0, 3, 4, 0, 5, 6];
        for cached in [false, true] {
            assert_normals(
                &run(indexed(&positions, &ids, None), Mode::Smooth, cached).unwrap(),
                &[Z, Z, Z, Z, Z, Z, Z, [0.0, 0.0, -1.0], [0.0, 0.0, -1.0]],
            );
        }
    }
}

#[test]
fn normalization_rejects_zero_and_nonfinite_vectors() {
    assert_eq!(unit([0.0, -0.0, 0.0]), None);
    for axis in 0..3 {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut vector = [1.0; 3];
            vector[axis] = invalid;
            assert_eq!(unit(vector), None);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Stop {
    call: usize,
    marker: &'static str,
}

#[test]
fn every_control_threshold_stops_immediately_in_all_paths() {
    let nonindexed = [
        POSITIONS[0],
        POSITIONS[1],
        POSITIONS[2],
        POSITIONS[0],
        POSITIONS[3],
        POSITIONS[1],
    ];
    for input in [
        indexed(&POSITIONS, &IDS, None),
        indexed(&POSITIONS, &IDS, Some(&[7, 7])),
        indexed(&POSITIONS, &IDS, Some(&[0, 7])),
        indexed(&POSITIONS, &IDS, Some(&[7, 9])),
        Input {
            positions: &nonindexed,
            vertex_ids: VertexIds::NonIndexed,
            smoothing_groups: Some(&[7, 7]),
        },
        indexed(&[], &[], None),
    ] {
        for mode in [Mode::Flat, Mode::Smooth] {
            for cached in [false, true] {
                let corners = input.corner_count();
                let mut output = vec![[73.0; 3]; corners];
                let mut order = vec![usize::MAX; if mode == Mode::Smooth { corners } else { 0 }];
                let mut crosses = vec![[73.0; 3]; corners / 3];
                let mut total = 0;
                generate(
                    input,
                    mode,
                    &mut output,
                    Workspace {
                        corner_order: &mut order,
                        face_crosses: cached.then_some(&mut crosses[..]),
                    },
                    || {
                        total += 1;
                        Ok::<(), Stop>(())
                    },
                )
                .unwrap();
                let expected = output.clone();
                assert!(total >= 2);
                for fail_at in 1..=total + 1 {
                    output.fill([73.0; 3]);
                    order.fill(usize::MAX);
                    crosses.fill([73.0; 3]);
                    let mut calls = 0;
                    let result = generate(
                        input,
                        mode,
                        &mut output,
                        Workspace {
                            corner_order: &mut order,
                            face_crosses: cached.then_some(&mut crosses[..]),
                        },
                        || {
                            calls += 1;
                            if calls == fail_at {
                                Err(Stop {
                                    call: calls,
                                    marker: "exact control error",
                                })
                            } else {
                                Ok(())
                            }
                        },
                    );
                    if fail_at <= total {
                        assert_eq!(
                            result,
                            Err(Error::Control(Stop {
                                call: fail_at,
                                marker: "exact control error",
                            })),
                            "mode={mode:?}, cached={cached}, threshold={fail_at}"
                        );
                        assert_eq!(calls, fail_at);
                        assert!(output.iter().flatten().all(|v| v.is_finite()));
                    } else {
                        assert_eq!(result, Ok(()));
                        assert_eq!(calls, total);
                        assert_eq!(output, expected);
                    }
                }
            }
        }
    }
}
