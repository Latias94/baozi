use baozi::{
    AttributeData, AttributeDomain, AttributeRef, Handedness, SceneLookupError, SceneSpaceField,
    SourceSpaceError, TopologyKind,
    postprocess::{
        NormalMode, PostProcessReport, PostProcessRunDisposition, PostProcessRunError,
        SpaceConversionCause, TriangulationCause,
    },
};

use crate::edit_lifecycle::{LifecycleRun, assert_token_cancellation, read_digest, run};

fn changed(result: &Option<Result<PostProcessReport, PostProcessRunError>>) -> &PostProcessReport {
    let report = result
        .as_ref()
        .expect("the seed must reach the operation")
        .as_ref()
        .expect("the seed must publish a successful edit");
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_ne!(report.input_generation(), report.output_generation());
    report
}

fn assert_original(run: &LifecycleRun) {
    assert_eq!(run.scene.generation(), run.input_generation);
    assert_eq!(read_digest(&run.scene), run.input_digest);
}

fn dense_f32(attribute: AttributeRef<'_>) -> &[f32] {
    let AttributeData::DenseF32 { values, .. } = attribute.data() else {
        panic!("the authored seed must expose F32 storage");
    };
    values
}

#[test]
fn quad_seed_really_changes_triangulation() {
    let run = run(include_bytes!(
        "../corpus/edit_lifecycle/triangulate_quad.bin"
    ))
    .unwrap();
    let report = changed(&run.triangulation);
    assert!(run.normals.is_none());
    assert!(run.conversion.is_none());
    let remap = report
        .triangulation_remap()
        .expect("changed triangulation origins");
    let primitive = run
        .scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(primitive.id(), remap.output_target());
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(
        (
            primitive.vertex_count(),
            primitive.corner_count(),
            primitive.face_count()
        ),
        (4, 6, 2)
    );
    assert!(matches!(
        run.scene.primitive(remap.input_target()),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
}

#[test]
fn smooth_and_flat_seeds_really_generate_normals() {
    for (seed, mode, corners) in [
        (
            include_bytes!("../corpus/edit_lifecycle/normals_smooth.bin").as_slice(),
            NormalMode::Smooth,
            6,
        ),
        (
            include_bytes!("../corpus/edit_lifecycle/normals_flat.bin").as_slice(),
            NormalMode::Flat,
            3,
        ),
    ] {
        let run = run(seed).unwrap();
        let report = changed(&run.normals);
        assert!(run.triangulation.is_none());
        assert!(run.conversion.is_none());
        let normals = report.normal_generation().expect("changed normal field");
        assert_eq!(normals.mode(), mode);
        assert!(!normals.replaced_authored_normals());
        let primitive = run.scene.primitive(normals.output_target()).unwrap();
        assert_eq!(primitive.corner_count(), corners);
        for normal in dense_f32(primitive.normals(AttributeDomain::Corner).unwrap()).chunks_exact(3)
        {
            assert_eq!(normal, &[0.0, 0.0, 1.0]);
        }
    }
}

#[test]
fn replacement_seed_changes_authored_normals() {
    let run = run(include_bytes!(
        "../corpus/edit_lifecycle/normals_replace.bin"
    ))
    .unwrap();
    let normals = changed(&run.normals).normal_generation().unwrap();
    assert!(normals.replaced_authored_normals());
    let primitive = run.scene.primitive(normals.output_target()).unwrap();
    assert_eq!(primitive.corner_count(), 3);
    for normal in dense_f32(primitive.normals(AttributeDomain::Corner).unwrap()).chunks_exact(3) {
        assert_eq!(normal, &[0.0, 0.0, 1.0]);
    }
}

#[test]
fn coordinate_seed_really_changes_numeric_positions() {
    let run = run(include_bytes!(
        "../corpus/edit_lifecycle/space_centimeters.bin"
    ))
    .unwrap();
    let report = changed(&run.conversion);
    assert!(report.space_conversion_effects().is_some());
    assert_eq!(run.scene.space().handedness, Handedness::Left);
    assert_eq!(run.scene.space().unit_scale_to_meters, Some(0.01));
    let primitive = run
        .scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(
        dense_f32(primitive.positions().unwrap()),
        &[
            -100.0, 200.0, 300.0, -200.0, 200.0, 300.0, -100.0, 400.0, 300.0
        ]
    );
}

#[test]
fn complete_workflows_reacquire_ids_across_all_three_publications() {
    for (seed, selected) in [
        (
            include_bytes!("../corpus/edit_lifecycle/workflow_two_meshes.bin").as_slice(),
            1,
        ),
        (
            include_bytes!("../corpus/edit_lifecycle/workflow_quad_ply.bin").as_slice(),
            0,
        ),
    ] {
        let run = run(seed).unwrap();
        let triangulation = changed(&run.triangulation);
        let normals = changed(&run.normals);
        let conversion = changed(&run.conversion);
        assert_eq!(
            triangulation.output_generation(),
            normals.input_generation()
        );
        assert_eq!(normals.output_generation(), conversion.input_generation());
        assert_eq!(conversion.output_generation(), run.scene.generation());
        assert_eq!(
            triangulation.triangulation_remap().unwrap().output_target(),
            normals.normal_generation().unwrap().input_target()
        );
        let primitive = run
            .scene
            .meshes()
            .flat_map(|mesh| mesh.primitives())
            .nth(selected)
            .unwrap();
        assert_eq!(primitive.id().generation(), run.scene.generation());
        assert_eq!(
            (
                primitive.vertex_count(),
                primitive.corner_count(),
                primitive.face_count()
            ),
            (4, 6, 2)
        );
        assert!(primitive.normals(AttributeDomain::Corner).is_some());
        read_digest(&run.scene);
    }
}

#[test]
fn rejected_polygon_preserves_generation_digest_and_borrowed_readability() {
    // Repeat the exact seed to establish a deterministic geometric rejection, without a deadline.
    for _ in 0..2 {
        let run = run(include_bytes!(
            "../corpus/edit_lifecycle/rejected_polygon.bin"
        ))
        .unwrap();
        let error = run.triangulation.as_ref().unwrap().as_ref().unwrap_err();
        let PostProcessRunError::Triangulation { failure } = error else {
            panic!("the seed must reach triangulation rejection");
        };
        assert!(matches!(
            failure.cause(),
            TriangulationCause::Polygon { .. }
        ));
        assert_eq!(failure.resources().unwrap().committed_canonical_bytes(), 0);
        assert_original(&run);
        let primitive = run
            .scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert_eq!(primitive.topology(), TopologyKind::Polygons);
        assert_eq!(primitive.indices(), Some([0, 1, 2, 3].as_slice()));
    }
}

#[test]
fn missing_source_conventions_reject_without_changing_the_input() {
    let run = run(include_bytes!(
        "../corpus/edit_lifecycle/rejected_space.bin"
    ))
    .unwrap();
    let error = run.conversion.as_ref().unwrap().as_ref().unwrap_err();
    let PostProcessRunError::SpaceConversion { failure } = error else {
        panic!("the seed must reach source convention rejection");
    };
    assert!(matches!(
        failure.cause(),
        SpaceConversionCause::Source {
            source: SourceSpaceError::MissingSourceConvention {
                field: SceneSpaceField::Handedness
            }
        }
    ));
    assert_original(&run);
}

#[test]
fn pre_execution_token_cancellation_preserves_each_original_scene() {
    for seed in [
        include_bytes!("../corpus/edit_lifecycle/cancel_triangulate.bin").as_slice(),
        include_bytes!("../corpus/edit_lifecycle/cancel_normals.bin").as_slice(),
        include_bytes!("../corpus/edit_lifecycle/cancel_space.bin").as_slice(),
    ] {
        let run = run(seed).unwrap();
        let result = run
            .triangulation
            .as_ref()
            .or(run.normals.as_ref())
            .or(run.conversion.as_ref())
            .unwrap();
        assert_token_cancellation(result.as_ref().unwrap_err());
        assert_original(&run);
    }
}

#[test]
fn cancellation_after_publication_returns_the_latest_generation_and_work_can_continue() {
    let run = run(include_bytes!(
        "../corpus/edit_lifecycle/workflow_cancel_normals.bin"
    ))
    .unwrap();
    let triangulation = changed(&run.triangulation);
    assert_token_cancellation(run.normals.as_ref().unwrap().as_ref().unwrap_err());
    let conversion = changed(&run.conversion);
    // The shared workflow compares the intermediate digest and reads before continuing the edit.
    assert_eq!(
        triangulation.output_generation(),
        conversion.input_generation()
    );
    assert_eq!(conversion.output_generation(), run.scene.generation());
}

#[test]
fn malformed_and_oversize_inputs_stop_before_edits() {
    assert!(run(&[]).is_none());
    assert!(run(&[0, 7, 0]).is_none());
    assert!(run(&[0, 7, 0, 0, 0xff]).is_none());
    assert!(run(&vec![0; 4 + 8 * 1024 + 1]).is_none());
}
