use std::convert::Infallible;

use super::{InputError, NormalMode, kernel, work_bound};

#[test]
fn work_envelope_covers_kernel_callbacks_and_scratch_initialization() {
    // Distinct Face groups maximize bucket boundary probes. Rotated vertex IDs vary the
    // heap order; unused positions confirm that the envelope includes the full Vertex scan.
    for faces in [1, 2, 3, 5, 8, 13, 21, 34, 55] {
        let positions = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [7.0; 3]];
        let corners = faces * 3;
        let indices: Vec<u32> = (0..faces)
            .flat_map(|face| (0..3).map(move |corner| ((face + corner) % 3) as u32))
            .collect();
        let distinct: Vec<u32> = (1..=faces as u32).collect();
        let hard = vec![0; faces];
        let shared = vec![1; faces];
        for groups in [
            None,
            Some(&distinct[..]),
            Some(&hard[..]),
            Some(&shared[..]),
        ] {
            for mode in [NormalMode::Flat, NormalMode::Smooth] {
                let smooth = mode == NormalMode::Smooth;
                let mut output = vec![[0.0; 3]; corners];
                let mut order = vec![0; if smooth { corners } else { 0 }];
                let mut crosses =
                    smooth.then(|| vec![crate::precision::Estimate3::default(); faces]);
                let mut callbacks = 0_u64;
                kernel::generate(
                    kernel::Input {
                        positions: positions.as_slice(),
                        vertex_ids: kernel::VertexIds::Indexed(&indices),
                        smoothing_groups: if smooth { groups } else { None },
                    },
                    if smooth {
                        kernel::Mode::Smooth
                    } else {
                        kernel::Mode::Flat
                    },
                    &mut output,
                    kernel::Workspace {
                        corner_order: &mut order,
                        face_crosses: crosses.as_deref_mut(),
                    },
                    || {
                        callbacks += 1;
                        Ok::<_, Infallible>(())
                    },
                )
                .unwrap();
                let initialization = corners + if smooth { corners + faces } else { 0 };
                let attributes = if groups.is_some() { 2 } else { 1 };
                // Four inspection callbacks in addition to descriptor rows, scratch rows,
                // and the actual kernel callbacks. Generation retains the inspected counts.
                let observed = callbacks + initialization as u64 + attributes as u64 + 4;
                let bound = work_bound(positions.len(), corners, attributes, mode).unwrap();
                assert!(
                    bound >= observed,
                    "{mode:?}, {faces} faces: observed {observed}, bound {bound}"
                );
            }
        }
    }
}

#[test]
fn work_envelope_includes_unused_vertices_and_attribute_descriptors() {
    for mode in [NormalMode::Flat, NormalMode::Smooth] {
        let baseline = work_bound(3, 3, 1, mode).unwrap();
        assert_eq!(work_bound(3 + 1000, 3, 1, mode).unwrap(), baseline + 1000);
        assert_eq!(work_bound(3, 3, 1 + 1000, mode).unwrap(), baseline + 1000);
    }
}

#[test]
fn work_envelope_checks_overflow() {
    for mode in [NormalMode::Flat, NormalMode::Smooth] {
        let bound = work_bound(usize::MAX, usize::MAX, usize::MAX, mode);
        if usize::BITS == 64 {
            assert!(matches!(bound, Err(InputError::SizeOverflow)));
        } else {
            // On 32-bit targets every representable input shape fits the u64 envelope.
            assert!(bound.is_ok());
        }
    }
}

#[test]
fn work_envelope_grows_past_the_published_limit() {
    for mode in [NormalMode::Flat, NormalMode::Smooth] {
        assert!(work_bound(3, 99_999_999, 1, mode).unwrap() > crate::operation::runtime::MAX_WORK);
    }
}

#[test]
fn borrowed_dense_reads_add_exactly_one_callback_per_logical_position_read() {
    let scene = crate::test_fixtures::import_obj(
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nv 7 7 7\nf 1 2 3\nf 1 4 2\n",
    )
    .into_parts()
    .0;
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let positions = primitive.positions().unwrap();
    let baozi_core::AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
        panic!("dense F32 fixture");
    };
    let (rows, remainder) = values.as_chunks::<3>();
    assert!(remainder.is_empty());
    let corners = primitive.corner_count() as usize;
    let faces = corners / 3;
    let reads = (positions.len() + corners) as u64;
    assert_eq!(
        super::position_read_work_bound(positions, corners).unwrap(),
        reads
    );
    for mode in [NormalMode::Flat, NormalMode::Smooth] {
        let smooth = mode == NormalMode::Smooth;
        let mode_kernel = if smooth {
            kernel::Mode::Smooth
        } else {
            kernel::Mode::Flat
        };
        let mut output = vec![[0.0; 3]; corners];
        let mut order = vec![0; if smooth { corners } else { 0 }];
        let mut crosses = smooth.then(|| vec![crate::precision::Estimate3::default(); faces]);
        let mut baseline = 0_u64;
        kernel::generate(
            kernel::Input {
                positions: rows,
                vertex_ids: kernel::VertexIds::Indexed(primitive.indices().unwrap()),
                smoothing_groups: None,
            },
            mode_kernel,
            &mut output,
            kernel::Workspace {
                corner_order: &mut order,
                face_crosses: crosses.as_deref_mut(),
            },
            || {
                baseline += 1;
                Ok::<_, Infallible>(())
            },
        )
        .unwrap();
        let expected = output.clone();
        let mut borrowed = 0_u64;
        kernel::generate_into(
            kernel::Input {
                positions,
                vertex_ids: kernel::VertexIds::Indexed(primitive.indices().unwrap()),
                smoothing_groups: None,
            },
            mode_kernel,
            kernel::Output::F32(&mut output),
            kernel::Workspace {
                corner_order: &mut order,
                face_crosses: crosses.as_deref_mut(),
            },
            || {
                borrowed += 1;
                Ok::<_, Infallible>(())
            },
        )
        .unwrap();
        assert_eq!(output, expected);
        assert_eq!(borrowed, baseline + reads);
        let initialization = corners + if smooth { corners + faces } else { 0 };
        let attributes = primitive.attributes().len();
        let observed = borrowed + initialization as u64 + attributes as u64 + 4;
        let bound = work_bound(positions.len(), corners, attributes, mode).unwrap() + reads;
        assert!(observed <= bound, "{mode:?}: {observed} > {bound}");
    }
}
