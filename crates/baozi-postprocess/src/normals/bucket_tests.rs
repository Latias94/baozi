use std::convert::Infallible;

use super::{
    Action, Buffer, Error, Input, Mode, Output, PositionSource, VertexIds, WorkEvent, Workspace,
    generate_into, stable_vertex_order, uniform_smoothing_groups,
};
use crate::GeometryNumericIssue;
use crate::precision::Estimate3;

const POSITIONS: [[f32; 3]; 3] = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
const IDS: [u32; 6] = [2, 0, 1, 1, 2, 0];

fn input<'a>(positions: &'a [[f32; 3]], groups: Option<&'a [u32]>) -> Input<'a> {
    Input {
        positions,
        vertex_ids: VertexIds::Indexed(&IDS),
        smoothing_groups: groups,
    }
}

fn oracle(input: Input<'_, impl PositionSource>) -> Vec<usize> {
    let mut order: Vec<_> = (0..input.corner_count()).collect();
    order.sort_unstable_by_key(|&corner| {
        let vertex = match input.vertex_ids {
            VertexIds::Indexed(ids) => ids[corner] as usize,
            VertexIds::NonIndexed => corner,
        };
        let group = input
            .smoothing_groups
            .map_or(1, |groups| groups[corner / 3]);
        (
            vertex,
            group,
            if group == 0 { corner / 3 } else { 0 },
            corner,
        )
    });
    order
}

#[test]
fn stable_buckets_and_general_groups_match_an_independent_total_order() {
    for groups in [
        None,
        Some([1, 1].as_slice()),
        Some([7, 7].as_slice()),
        Some([u32::MAX, u32::MAX].as_slice()),
        Some([0, 0].as_slice()),
        Some([7, 0].as_slice()),
        Some([0, 7].as_slice()),
        Some([9, 7].as_slice()),
        Some([7, 9].as_slice()),
    ] {
        for cached in [false, true] {
            for supplied in [false, true] {
                let input = input(&POSITIONS, groups);
                let mut output = [[73.0; 3]; 6];
                let mut order = [usize::MAX; 6];
                let mut cursors = [usize::MAX; 3];
                let mut crosses = [Estimate3::default(); 2];
                generate_into(
                    input,
                    Mode::Smooth,
                    Output::F64(&mut output),
                    Workspace {
                        corner_order: &mut order,
                        vertex_cursors: supplied.then_some(&mut cursors),
                        face_crosses: cached.then_some(&mut crosses),
                    },
                    |_| Ok::<_, Infallible>(()),
                )
                .unwrap();
                assert_eq!(order.as_slice(), oracle(input));
                assert_eq!(output, [[0.0, 0.0, 1.0]; 6]);
                if groups.is_none() {
                    assert_eq!(order, [1, 5, 2, 3, 0, 4]);
                }
                if !supplied || groups.is_some_and(|g| g[0] == 0 || g[0] != g[1]) {
                    assert_eq!(cursors, [usize::MAX; 3]);
                }
            }
        }
    }
}

#[test]
fn cursor_domain_bound_includes_unused_rows_without_allocating_for_larger_domains() {
    for vertices in [3, 6, 7] {
        let mut positions = vec![[0.0; 3]; vertices];
        positions[..3].copy_from_slice(&POSITIONS);
        let input = input(&positions, None);
        let expected = if vertices <= IDS.len() { vertices } else { 0 };
        assert_eq!(input.vertex_cursor_count(Mode::Smooth), expected);
        assert_eq!(input.vertex_cursor_count(Mode::Flat), 0);
        let mut order = [usize::MAX; 6];
        let mut cursors = vec![usize::MAX; expected];
        let mut output = [[73.0; 3]; 6];
        generate_into(
            input,
            Mode::Smooth,
            Output::F32(&mut output),
            Workspace {
                corner_order: &mut order,
                vertex_cursors: (!cursors.is_empty()).then_some(cursors.as_mut_slice()),
                face_crosses: None,
            },
            |_| Ok::<_, Infallible>(()),
        )
        .unwrap();
        assert_eq!(order.as_slice(), oracle(input));
        assert_eq!(output, [[0.0, 0.0, 1.0]; 6]);
    }
    let nonindexed = Input {
        positions: POSITIONS.as_slice(),
        vertex_ids: VertexIds::NonIndexed,
        smoothing_groups: None,
    };
    assert_eq!(nonindexed.vertex_cursor_count(Mode::Smooth), 0);
    let empty = Input {
        vertex_ids: VertexIds::Indexed(&[]),
        ..nonindexed
    };
    assert_eq!(empty.vertex_cursor_count(Mode::Smooth), 0);
}

#[test]
fn supplied_cursor_shapes_are_checked_before_any_workspace_mutation() {
    let mut larger_positions = POSITIONS.to_vec();
    larger_positions.extend([[0.0; 3]; 4]);
    for (input, mode, actual, expected) in [
        (input(&POSITIONS, None), Mode::Smooth, 0, 3),
        (input(&POSITIONS, None), Mode::Smooth, 2, 3),
        (input(&POSITIONS, None), Mode::Smooth, 4, 3),
        (input(&POSITIONS, None), Mode::Flat, 3, 0),
        (input(&larger_positions, None), Mode::Smooth, 1, 0),
        (
            Input {
                positions: POSITIONS.as_slice(),
                vertex_ids: VertexIds::NonIndexed,
                smoothing_groups: None,
            },
            Mode::Smooth,
            1,
            0,
        ),
    ] {
        let corners = input.corner_count();
        let mut output = vec![[73.0; 3]; corners];
        let mut order = vec![usize::MAX; if mode == Mode::Smooth { corners } else { 0 }];
        let mut cursors = vec![usize::MAX; actual];
        let mut events = Vec::new();
        assert_eq!(
            generate_into(
                input,
                mode,
                Output::F32(&mut output),
                Workspace {
                    corner_order: &mut order,
                    vertex_cursors: Some(&mut cursors),
                    face_crosses: None,
                },
                |event| {
                    events.push(event);
                    Ok::<_, Infallible>(())
                },
            ),
            Err(Error::BufferLength {
                buffer: Buffer::VertexCursors,
                expected,
                actual,
            })
        );
        assert_eq!(events, [WorkEvent::Checkpoint]);
        assert!(output.iter().all(|row| *row == [73.0; 3]));
        assert!(order.iter().all(|slot| *slot == usize::MAX));
        assert!(cursors.iter().all(|slot| *slot == usize::MAX));
    }
}

#[test]
fn uniform_group_chunks_never_convert_rejection_into_general_sort() {
    for (groups, uniform, chunks) in [
        (None, true, 0),
        (Some([7, 7, 7].as_slice()), true, 1),
        (Some([7, 7, 9].as_slice()), false, 1),
        (Some([7, 0, 7].as_slice()), false, 1),
        (Some([0, 7, 7].as_slice()), false, 1),
    ] {
        let mut events = Vec::new();
        assert_eq!(
            uniform_smoothing_groups(groups, &mut |event| {
                events.push(event);
                Ok::<_, usize>(())
            }),
            Ok(uniform)
        );
        assert_eq!(
            events,
            [
                WorkEvent::Begin(Action::UniformGroupChunk),
                WorkEvent::Complete
            ]
            .repeat(chunks)
        );
        if chunks != 0 {
            for reject_begin in [true, false] {
                assert_eq!(
                    uniform_smoothing_groups(groups, &mut |event| {
                        if matches!(event, WorkEvent::Begin(_)) == reject_begin {
                            Err(1)
                        } else {
                            Ok(())
                        }
                    }),
                    Err(Error::Control(1))
                );
            }
        }
    }
    let mut groups = [7; 33];
    groups[32] = 0;
    for reject_begin in [false, true] {
        let mut begins = 0;
        let result = uniform_smoothing_groups(Some(&groups), &mut |event| {
            begins += usize::from(matches!(event, WorkEvent::Begin(_)));
            if reject_begin && begins == 2 && matches!(event, WorkEvent::Begin(_)) {
                Err(2)
            } else {
                Ok(())
            }
        });
        assert_eq!(begins, 2);
        assert_eq!(
            result,
            if reject_begin {
                Err(Error::Control(2))
            } else {
                Ok(false)
            }
        );
    }
}

#[test]
fn ordering_chunk_denial_and_completion_keep_literal_physical_prefixes() {
    const U: usize = usize::MAX;
    // This fixture has four operations: reset, six histogram records, prefix, six scatter records.
    for (reject_begin, stop, expected_cursors, expected_order) in [
        (true, 1, [U, U, U], [U; 6]),
        (false, 1, [0, 0, 0], [U; 6]),
        (true, 2, [0, 0, 0], [U; 6]),
        (false, 2, [2, 2, 2], [U; 6]),
        (true, 3, [2, 2, 2], [U; 6]),
        (false, 3, [0, 2, 4], [U; 6]),
        (true, 4, [0, 2, 4], [U; 6]),
        (false, 4, [2, 4, 6], [1, 5, 2, 3, 0, 4]),
    ] {
        let mut order = [U; 6];
        let mut cursors = [U; 3];
        let (mut begins, mut completes) = (0, 0);
        let result = stable_vertex_order(
            input(&POSITIONS, None),
            &mut order,
            &mut cursors,
            &mut |event| {
                begins += usize::from(matches!(event, WorkEvent::Begin(_)));
                completes += usize::from(event == WorkEvent::Complete);
                if begins == stop && matches!(event, WorkEvent::Begin(_)) == reject_begin {
                    Err(stop)
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(result, Err(Error::Control(stop)));
        assert_eq!(
            (begins, completes),
            (stop, stop - usize::from(reject_begin))
        );
        assert_eq!(cursors, expected_cursors);
        assert_eq!(order, expected_order);
    }
}

#[test]
fn copy_scan_boundaries_admit_no_part_of_the_denied_chunk() {
    for corners in [0, 1, 31, 32, 33] {
        let ids = vec![0_u32; corners];
        let mut order = vec![usize::MAX; corners];
        let mut cursors = [usize::MAX];
        let mut scatters = 0;
        let result = stable_vertex_order(
            Input {
                positions: &POSITIONS[..1],
                vertex_ids: VertexIds::Indexed(&ids),
                smoothing_groups: None,
            },
            &mut order,
            &mut cursors,
            &mut |event| {
                if matches!(event, WorkEvent::Begin(Action::ScatterChunk)) {
                    scatters += 1;
                    if scatters == 2 {
                        return Err("second scatter denied");
                    }
                }
                Ok(())
            },
        );
        if corners == 33 {
            assert_eq!(result, Err(Error::Control("second scatter denied")));
            assert_eq!(cursors, [32]);
            assert_eq!(order[32], usize::MAX);
        } else {
            result.unwrap();
            assert_eq!(cursors, [corners]);
        }
        for (index, value) in order.iter().take(corners.min(32)).enumerate() {
            assert_eq!(*value, index);
        }
    }
}

fn check_cancelling_bucket<P: PositionSource>(positions: P) {
    let ids = [1, 2, 0, 0, 2, 1];
    for groups in [None, Some([7, 7].as_slice())] {
        for cached in [false, true] {
            for supplied in [false, true] {
                let mut output = [[73.0; 3]; 6];
                let mut order = [usize::MAX; 6];
                let mut cursors = [usize::MAX; 3];
                let mut crosses = [Estimate3::default(); 2];
                let result = generate_into(
                    Input {
                        positions,
                        vertex_ids: VertexIds::Indexed(&ids),
                        smoothing_groups: groups,
                    },
                    Mode::Smooth,
                    Output::F64(&mut output),
                    Workspace {
                        corner_order: &mut order,
                        vertex_cursors: supplied.then_some(&mut cursors),
                        face_crosses: cached.then_some(&mut crosses),
                    },
                    |_| Ok::<_, Infallible>(()),
                );
                let group = groups.map(|groups| groups[0]);
                let expected = if positions.is_f64() {
                    Error::SmoothNumerical {
                        vertex: 0,
                        group,
                        corner: 2,
                        issue: GeometryNumericIssue::UncertainDirection,
                    }
                } else {
                    Error::InvalidSmoothSum {
                        vertex: 0,
                        group,
                        corner: 2,
                    }
                };
                assert_eq!(result, Err(expected));
                assert_eq!(output, [[73.0; 3]; 6]);
            }
        }
    }
}

fn check_weighted_bits<P: PositionSource>(positions: P) {
    let ids = [0, 1, 2, 0, 3, 4];
    for groups in [None, Some([7, 7].as_slice())] {
        let mut expected = None;
        for cached in [false, true] {
            for supplied in [false, true] {
                let mut output = [[73.0; 3]; 6];
                let mut order = [usize::MAX; 6];
                let mut cursors = [usize::MAX; 5];
                let mut crosses = [Estimate3::default(); 2];
                generate_into(
                    Input {
                        positions,
                        vertex_ids: VertexIds::Indexed(&ids),
                        smoothing_groups: groups,
                    },
                    Mode::Smooth,
                    Output::F64(&mut output),
                    Workspace {
                        corner_order: &mut order,
                        vertex_cursors: supplied.then_some(&mut cursors),
                        face_crosses: cached.then_some(&mut crosses),
                    },
                    |_| Ok::<_, Infallible>(()),
                )
                .unwrap();
                let bits = output.map(|row| row.map(f64::to_bits));
                if let Some(expected) = expected {
                    assert_eq!(bits, expected);
                } else {
                    expected = Some(bits);
                }
            }
        }
    }
}

#[test]
fn weighted_f32_and_f64_output_bits_match_general_order_with_and_without_cache() {
    let positions = [
        [0.0_f32; 3],
        [0.5, 0.0, 0.0],
        [0.0, 0.5, 0.0],
        [0.0, 0.0, 0.25],
        [0.25, 0.0, 0.0],
    ];
    check_weighted_bits(positions.as_slice());
    let wide = positions.map(|row| row.map(f64::from));
    check_weighted_bits(wide.as_slice());
}

#[test]
fn f32_and_f64_cancellation_keep_the_original_bucket_error() {
    check_cancelling_bucket(POSITIONS.as_slice());
    let wide = POSITIONS.map(|row| row.map(f64::from));
    check_cancelling_bucket(wide.as_slice());
}

#[test]
fn face_validation_precedes_cursor_access() {
    let ids = [0, 1, 99, 0, 1, 2];
    let mut output = [[73.0; 3]; 6];
    let mut order = [usize::MAX; 6];
    let mut cursors = [usize::MAX; 3];
    assert_eq!(
        generate_into(
            Input {
                positions: POSITIONS.as_slice(),
                vertex_ids: VertexIds::Indexed(&ids),
                smoothing_groups: None,
            },
            Mode::Smooth,
            Output::F32(&mut output),
            Workspace {
                corner_order: &mut order,
                vertex_cursors: Some(&mut cursors),
                face_crosses: None,
            },
            |_| Ok::<_, Infallible>(()),
        ),
        Err(Error::VertexOutOfBounds {
            corner: 2,
            vertex: 99,
            vertex_count: 3,
        })
    );
    assert_eq!(output, [[73.0; 3]; 6]);
    assert_eq!(order, [usize::MAX; 6]);
    assert_eq!(cursors, [usize::MAX; 3]);
}

#[test]
fn final_checkpoint_rejects_completed_bucket_output() {
    let mut output = [[73.0; 3]; 6];
    let mut order = [usize::MAX; 6];
    let mut cursors = [usize::MAX; 3];
    let mut checkpoints = 0;
    let result = generate_into(
        input(&POSITIONS, None),
        Mode::Smooth,
        Output::F32(&mut output),
        Workspace {
            corner_order: &mut order,
            vertex_cursors: Some(&mut cursors),
            face_crosses: None,
        },
        |event| {
            if event == WorkEvent::Checkpoint {
                checkpoints += 1;
                if checkpoints == 5 {
                    return Err("final control");
                }
            }
            Ok(())
        },
    );
    assert_eq!(result, Err(Error::Control("final control")));
    assert_eq!(order, [1, 5, 2, 3, 0, 4]);
    assert_eq!(output, [[0.0, 0.0, 1.0]; 6]);
}
