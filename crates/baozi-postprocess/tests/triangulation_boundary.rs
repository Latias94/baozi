mod common;

use baozi_core::TopologyKind;
use baozi_postprocess::{
    PostProcessCancellationOrigin, PostProcessCancellationToken, PostProcessControl,
    PostProcessControlCause, PostProcessOperation, PostProcessRunDisposition, PostProcessRunError,
    PrimitiveCorner, PrimitiveFace, TriangulationRemapError, triangulate_primitive,
};
use common::{canonical_quad_import, canonical_quad_scene, canonical_triangle_scene, import_obj};

#[test]
fn triangulation_publishes_changed_scene_and_retains_rooted_remap() {
    let import = canonical_quad_import();
    let (scene, import_evidence) = import.into_parts();
    let input_generation = scene.generation();
    assert!(import_evidence.matches_scene(&scene));
    let input_primitive = scene
        .meshes()
        .next()
        .expect("quad import should contain one mesh")
        .primitives()
        .next()
        .expect("quad import should contain one primitive");
    assert_eq!(input_primitive.topology(), TopologyKind::Polygons);
    let input_id = input_primitive.id();

    let result = triangulate_primitive(scene, input_id, PostProcessControl::default())
        .expect("a valid quad should triangulate");
    let (output, report) = result.into_parts();

    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_eq!(
        report.operation(),
        PostProcessOperation::Triangulate { target: input_id }
    );
    assert_eq!(report.input_generation(), input_generation);
    assert_eq!(report.output_generation(), output.generation());
    assert!(report.matches_scene(&output));
    assert_ne!(output.generation(), input_generation);
    assert!(!import_evidence.matches_scene(&output));
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
    assert!(report.resources().peak_staging_bytes() > 0);
    assert!(report.resources().committed_canonical_bytes() > 0);
    assert!(report.resources().attempted_work_items() > 0);

    let remap = report
        .triangulation_remap()
        .expect("changed triangulation must expose rooted remap evidence");
    assert_eq!(remap.input_target(), input_id);
    assert_eq!(remap.source_corner_count(), 4);
    assert_eq!(remap.vertex_count(), 4);
    assert_eq!(remap.source_corner_origins().len(), 6);
    assert_eq!(remap.source_face_triangle_offsets(), &[0, 2]);

    let output_id = remap.output_target();
    let output_primitive = output
        .primitive(output_id)
        .expect("remap output target must belong to the published scene");
    assert_eq!(output_primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(output_primitive.vertex_count(), 4);
    assert_eq!(output_primitive.corner_count(), 6);
    assert_eq!(output_primitive.face_count(), 2);
    assert!(output_primitive.polygon_face_sizes().is_none());

    for source_corner in 0..4 {
        let images = remap
            .corner_images(PrimitiveCorner {
                primitive: input_id,
                index: source_corner,
            })
            .expect("source corner must belong to the input primitive")
            .collect::<Vec<_>>();
        assert!(!images.is_empty());
        for image in images {
            let origin = remap
                .corner_origin(image)
                .expect("output corner must belong to the output primitive");
            assert_eq!(
                origin,
                PrimitiveCorner {
                    primitive: input_id,
                    index: source_corner
                }
            );
        }
    }
    assert_eq!(
        remap
            .face_range(PrimitiveFace {
                primitive: input_id,
                index: 0
            })
            .expect("source face must have a triangle range"),
        0..2
    );
    assert_eq!(
        remap
            .face_origin(PrimitiveFace {
                primitive: output_id,
                index: 1
            })
            .expect("output face must have a source face"),
        PrimitiveFace {
            primitive: input_id,
            index: 0
        }
    );
    assert_eq!(
        remap.corner_origin(PrimitiveCorner {
            primitive: input_id,
            index: 0
        }),
        Err(TriangulationRemapError::ForeignOwner)
    );
    let origins = remap.source_corner_origins().to_vec();
    drop(output);
    drop(import_evidence);
    assert_eq!(remap.source_corner_origins(), origins);
    assert_eq!(remap.source_face_triangle_offsets(), &[0, 2]);
}

#[test]
fn triangle_list_returns_the_original_scene_unchanged() {
    let scene = canonical_triangle_scene();
    let generation = scene.generation();
    let primitive = scene
        .meshes()
        .next()
        .expect("triangle import should contain one mesh")
        .primitives()
        .next()
        .expect("triangle import should contain one primitive");
    let primitive_id = primitive.id();
    let result = triangulate_primitive(scene, primitive_id, PostProcessControl::default())
        .expect("triangle lists should be unchanged");
    let (scene, report) = result.into_parts();

    assert_eq!(scene.generation(), generation);
    assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
    assert_eq!(
        report.operation(),
        PostProcessOperation::Triangulate {
            target: primitive_id
        }
    );
    assert_eq!(report.input_generation(), generation);
    assert!(report.triangulation_remap().is_none());
    assert_eq!(report.output_generation(), generation);
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
}

#[test]
fn cancelled_triangulation_returns_the_original_scene() {
    let (scene, import_evidence) = canonical_quad_import().into_parts();
    let generation = scene.generation();
    let primitive = scene
        .meshes()
        .next()
        .expect("quad import should contain one mesh")
        .primitives()
        .next()
        .expect("quad import should contain one primitive");
    let primitive_id = primitive.id();
    let cancellation = PostProcessCancellationToken::new();
    cancellation.cancel();
    let failure = triangulate_primitive(
        scene,
        primitive_id,
        PostProcessControl::default().with_cancellation(cancellation),
    )
    .expect_err("pre-cancelled triangulation must fail closed");
    let (scene, failure) = failure.into_parts();
    let failure = match failure {
        PostProcessRunError::Triangulation { failure } => failure,
        other => panic!("expected a triangulation failure, got {other:?}"),
    };

    assert_eq!(scene.generation(), generation);
    assert!(import_evidence.matches_scene(&scene));
    assert!(scene.primitive(primitive_id).is_ok());
    assert!(matches!(
        failure.cause(),
        baozi_postprocess::TriangulationCause::Control {
            cause: PostProcessControlCause::Cancelled {
                origin: PostProcessCancellationOrigin::Signal
            }
        }
    ));
    assert_eq!(
        failure.resources().map(|value| value.current_live_bytes()),
        Some(0)
    );
}

#[test]
fn malformed_polygon_rejection_preserves_the_original_primitive() {
    let (scene, import_evidence) =
        import_obj(b"v 0 0 0\nv 1 1 0\nv 0 1 0\nv 1 0 0\nf 1 2 3 4\n").into_parts();
    let generation = scene.generation();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let target = primitive.id();
    let original_indices = primitive.indices().unwrap().to_vec();
    let failure = triangulate_primitive(scene, target, PostProcessControl::default())
        .expect_err("a self-intersecting ring must not publish an output");
    let (scene, error) = failure.into_parts();
    assert_eq!(scene.generation(), generation);
    assert!(import_evidence.matches_scene(&scene));
    let original = scene.primitive(target).unwrap();
    assert_eq!(original.topology(), TopologyKind::Polygons);
    assert_eq!(original.indices().unwrap(), original_indices);
    let PostProcessRunError::Triangulation { failure } = error else {
        panic!("the malformed ring must retain a typed triangulation failure");
    };
    assert!(matches!(
        failure.cause(),
        baozi_postprocess::TriangulationCause::Polygon { .. }
    ));
    let resources = failure
        .resources()
        .expect("rejection must retain terminal evidence");
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert_eq!(resources.committed_canonical_bytes(), 0);
}

#[test]
fn success_and_failure_release_all_allocations_after_the_final_owner() {
    let run = || {
        let scene = canonical_quad_scene();
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let (scene, report) = triangulate_primitive(scene, target, PostProcessControl::default())
            .expect("the lifetime fixture must triangulate")
            .into_parts();
        drop(scene);
        assert_eq!(
            report
                .triangulation_remap()
                .unwrap()
                .source_face_triangle_offsets(),
            &[0, 2]
        );
        drop(report);

        let scene = canonical_quad_scene();
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let cancellation = PostProcessCancellationToken::new();
        cancellation.cancel();
        drop(
            triangulate_primitive(
                scene,
                target,
                PostProcessControl::default().with_cancellation(cancellation),
            )
            .expect_err("the cancelled fixture must fail"),
        );
    };
    // Initialize thread-local runtime helpers before observing the invocation's own allocations.
    run();
    let allocations = allocation_counter::measure(run);
    assert!(allocations.count_total > 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}

#[test]
fn triangulation_preserves_corner_attributes_and_face_smoothing() {
    let (scene, _evidence) = import_obj(
        b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
          vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\n\
          s 7\nf 1/1/1 2/2/1 3/3/1 4/4/1\n",
    )
    .into_parts();
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let result = triangulate_primitive(scene, target, PostProcessControl::default())
        .expect("Corner and Face attributes must not prevent triangulation");
    let remap = result.report().triangulation_remap().unwrap();
    let primitive = result.scene().primitive(remap.output_target()).unwrap();
    let baozi_core::AttributeData::DenseU32 { lanes: 1, values } = primitive
        .attribute(baozi_core::AttributeSemanticRef::SmoothingGroup)
        .unwrap()
        .data()
    else {
        panic!("Face smoothing groups must remain dense U32");
    };
    assert_eq!(values, &[7, 7]);
    assert_eq!(primitive.texcoords(0).unwrap().len(), 6);
    assert_eq!(primitive.normals().unwrap().len(), 6);
}
