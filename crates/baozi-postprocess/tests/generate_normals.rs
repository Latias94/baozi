use baozi_core::{BaoziError, Mesh, MeshBinding, Node, PrimitiveTopology, Result, Scene, Vec3};
use baozi_postprocess::{PostProcessPipeline, PostProcessStep};

#[test]
fn shared_indexed_vertex_with_conflicting_flat_normals_is_rejected() -> Result<()> {
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
    let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateNormals]);

    let error = pipeline.run(scene).unwrap_err();

    assert_eq!(
        error,
        BaoziError::PostProcess {
            step: "GenerateNormals",
            message: "mesh 0 shared vertex 0 needs multiple generated normals".to_owned()
        }
    );
    Ok(())
}

fn scene_with_mesh(mesh: Mesh) -> Scene {
    let mut builder = baozi_core::SceneBuilder::new();
    let mesh = builder.add_mesh(mesh);
    builder
        .add_child_node(
            builder.root(),
            Node {
                mesh_bindings: vec![MeshBinding::new(mesh)],
                ..Node::default()
            },
        )
        .expect("test scene node should be valid");
    builder.finish().expect("test scene should be valid")
}
