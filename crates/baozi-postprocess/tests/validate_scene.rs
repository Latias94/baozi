use baozi_core::{Mesh, MeshBinding, MeshId, SceneBuilder};
use baozi_postprocess::{PostProcessPipeline, PostProcessStep};

#[test]
fn validate_scene_step_reuses_core_validator() {
    let mut invalid_scene = SceneBuilder::new().finish().unwrap();
    invalid_scene.meshes.push(Mesh::default());
    invalid_scene.nodes[0]
        .mesh_bindings
        .push(MeshBinding::new(MeshId::new(0)));
    let pipeline = PostProcessPipeline::new([PostProcessStep::ValidateScene]);

    let error = pipeline.run(invalid_scene).unwrap_err();

    assert!(error.to_string().contains("empty"));
}
