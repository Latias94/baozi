use baozi_core::{
    Mesh, MeshBinding, Node, PrimitiveTopology, ValidatedScene, Vec3, validate_scene,
};
use baozi_postprocess::{
    PostProcessError, PostProcessPipeline, PostProcessPlanError, PostProcessStep,
    PostProcessStepIssue,
};

#[test]
fn pipeline_uses_canonical_order() {
    let pipeline = PostProcessPipeline::new([
        PostProcessStep::GenerateNormals,
        PostProcessStep::Triangulate,
    ])
    .unwrap();

    assert_eq!(
        pipeline.steps(),
        &[
            PostProcessStep::Triangulate,
            PostProcessStep::GenerateNormals,
        ]
    );
}

#[test]
fn plan_compilation_bounds_redundant_or_infinite_iterators() {
    let error =
        PostProcessPipeline::new(std::iter::repeat(PostProcessStep::Triangulate)).unwrap_err();

    assert_eq!(
        error,
        PostProcessPlanError::TooManyRequestedEntries {
            maximum: PostProcessStep::MAX_REQUESTED_ENTRIES,
        }
    );
}

#[test]
fn plan_compilation_stops_after_every_distinct_step_is_known() {
    let requested = PostProcessStep::CANONICAL_ORDER
        .iter()
        .copied()
        .chain(std::iter::once_with(|| {
            panic!("iterator must not be polled")
        }));

    let pipeline = PostProcessPipeline::new(requested).unwrap();

    assert_eq!(pipeline.steps(), PostProcessStep::CANONICAL_ORDER);
}

#[test]
fn triangulation_publishes_a_new_generation() {
    let scene = scene_with_meshes([Mesh {
        topology: PrimitiveTopology::Polygons,
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
        ],
        indices: vec![0, 1, 2, 3, 1, 4, 5],
        face_vertex_counts: vec![4, 3],
        ..Mesh::default()
    }]);

    let output = PostProcessPipeline::new([PostProcessStep::Triangulate])
        .unwrap()
        .run(&scene)
        .unwrap();
    let output = output.scene();

    assert_ne!(output.generation(), scene.generation());
    assert_eq!(scene.meshes()[0].topology, PrimitiveTopology::Polygons);
    assert_eq!(output.meshes()[0].topology, PrimitiveTopology::Triangles);
    assert!(output.meshes()[0].face_vertex_counts.is_empty());
    assert_eq!(output.meshes()[0].indices, vec![0, 1, 2, 0, 2, 3, 1, 4, 5]);
}

#[test]
fn generated_bounds_do_not_mutate_the_input() {
    let scene = scene_with_meshes([Mesh {
        positions: vec![
            Vec3::new(2.0, -1.0, 0.5),
            Vec3::new(-3.0, 4.0, 1.5),
            Vec3::new(0.0, 2.0, -2.0),
        ],
        ..Mesh::default()
    }]);

    let output = PostProcessPipeline::new([PostProcessStep::GenerateBoundingBoxes])
        .unwrap()
        .run(&scene)
        .unwrap();
    let output = output.scene();

    let bounds = output.meshes()[0].bounds.unwrap();
    assert_ne!(output.generation(), scene.generation());
    assert!(scene.meshes()[0].bounds.is_none());
    assert_eq!(bounds.min, Vec3::new(-3.0, -1.0, -2.0));
    assert_eq!(bounds.max, Vec3::new(2.0, 4.0, 1.5));
}

#[test]
fn no_effectful_change_publishes_an_accounted_generation() {
    let scene = scene_with_meshes([Mesh {
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ],
        normals: vec![Vec3::new(0.0, 0.0, 1.0); 3],
        ..Mesh::default()
    }]);

    let output = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run(&scene)
        .unwrap();
    let output = output.scene();

    assert_ne!(output.generation(), scene.generation());
    assert_eq!(output.meshes()[0].normals, scene.meshes()[0].normals);
}

#[test]
fn indexed_triangle_normals_are_generated() {
    let scene = scene_with_meshes([Mesh {
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ],
        indices: vec![0, 1, 2],
        ..Mesh::default()
    }]);

    let output = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run(&scene)
        .unwrap();
    let output = output.scene();

    assert_eq!(
        output.meshes()[0].normals,
        vec![Vec3::new(0.0, 0.0, 1.0); 3]
    );
}

#[test]
fn triangulation_then_normal_generation_handles_a_quad() {
    let scene = scene_with_meshes([Mesh {
        topology: PrimitiveTopology::Polygons,
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ],
        indices: vec![0, 1, 2, 3],
        face_vertex_counts: vec![4],
        ..Mesh::default()
    }]);

    let output = PostProcessPipeline::new([
        PostProcessStep::Triangulate,
        PostProcessStep::GenerateNormals,
    ])
    .unwrap()
    .run(&scene)
    .unwrap();
    let output = output.scene();

    assert_eq!(output.meshes()[0].indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(
        output.meshes()[0].normals,
        vec![Vec3::new(0.0, 0.0, 1.0); 4]
    );
}

#[test]
fn degenerate_triangle_returns_a_typed_step_error() {
    let scene = scene_with_meshes([Mesh {
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ],
        indices: vec![0, 1, 2],
        ..Mesh::default()
    }]);

    let failure = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run(&scene)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert_eq!(
        failure.error(),
        &PostProcessError::StepFailed {
            step: PostProcessStep::GenerateNormals,
            issue: PostProcessStepIssue::DegenerateTriangle {
                mesh: 0,
                triangle: 0,
            },
        }
    );
    assert!(scene.meshes()[0].normals.is_empty());
}

fn scene_with_meshes<const N: usize>(meshes: [Mesh; N]) -> ValidatedScene {
    let mut builder = baozi_core::SceneBuilder::try_new().expect("test scene builder allocation");
    for mesh in meshes {
        let mesh = builder.add_mesh(mesh).expect("test mesh id should fit");
        builder
            .add_child_node(
                builder.root(),
                Node {
                    mesh_bindings: vec![MeshBinding::new(mesh)],
                    ..Node::default()
                },
            )
            .expect("test node should be valid");
    }
    validate_scene(builder.finish_draft()).expect("test scene should be valid")
}
