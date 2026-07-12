use baozi_core::{
    Mesh, MeshBinding, Node, SceneValidationCode, SceneValidationError, Vec3, validate_scene,
};
use baozi_postprocess::PostProcessPipeline;

#[test]
fn empty_pipeline_publishes_a_fresh_validated_generation() {
    let scene = triangle_scene();

    let output = PostProcessPipeline::new([]).unwrap().run(&scene).unwrap();

    assert_ne!(output.scene().generation(), scene.generation());
    assert_eq!(output.scene().meshes(), scene.meshes());
}

#[test]
fn scene_edit_commit_is_the_only_revalidation_boundary() {
    let scene = triangle_scene();
    let mut edit = scene.begin_edit().unwrap();
    edit.meshes_mut()[0].positions.clear();

    let error = edit.commit().unwrap_err();

    assert!(matches!(
        error,
        SceneValidationError::InvalidScene { issue }
            if issue.code() == SceneValidationCode::Empty
    ));
    assert_eq!(scene.meshes()[0].positions.len(), 3);
}

fn triangle_scene() -> baozi_core::ValidatedScene {
    let mut builder = baozi_core::SceneBuilder::try_new().expect("test scene builder allocation");
    let mesh = builder
        .add_mesh(Mesh {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            ..Mesh::default()
        })
        .expect("test mesh id should fit");
    builder
        .add_child_node(
            builder.root(),
            Node {
                mesh_bindings: vec![MeshBinding::new(mesh)],
                ..Node::default()
            },
        )
        .expect("test node should be valid");
    validate_scene(builder.finish_draft()).expect("test scene should be valid")
}
