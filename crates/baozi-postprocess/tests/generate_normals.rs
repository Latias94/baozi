use baozi_core::{
    Mesh, MeshBinding, Node, PrimitiveTopology, ValidatedScene, Vec3, validate_scene,
};
use baozi_postprocess::{
    PostProcessError, PostProcessPipeline, PostProcessStep, PostProcessStepIssue,
};

#[test]
fn shared_indexed_vertex_with_conflicting_flat_normals_is_rejected() {
    let scene = scene_with_mesh(Mesh {
        topology: PrimitiveTopology::Triangles,
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
        indices: vec![0, 1, 2, 0, 3, 1],
        ..Mesh::default()
    });
    let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateNormals]).unwrap();

    let failure = pipeline
        .run(&scene)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert_eq!(
        failure.error(),
        &PostProcessError::StepFailed {
            step: PostProcessStep::GenerateNormals,
            issue: PostProcessStepIssue::ConflictingGeneratedNormal { mesh: 0, vertex: 0 },
        }
    );
    assert!(scene.meshes()[0].normals.is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

fn scene_with_mesh(mesh: Mesh) -> ValidatedScene {
    let mut builder = baozi_core::SceneBuilder::try_new().expect("test scene builder allocation");
    let mesh = builder.add_mesh(mesh).expect("test mesh id should fit");
    builder
        .add_child_node(
            builder.root(),
            Node {
                mesh_bindings: vec![MeshBinding::new(mesh)],
                ..Node::default()
            },
        )
        .expect("test scene node should be valid");
    validate_scene(builder.finish_draft()).expect("test scene should be valid")
}
