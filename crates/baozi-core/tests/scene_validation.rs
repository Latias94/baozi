use baozi_core::{
    Material, MaterialId, Mesh, MeshId, Node, NodeId, PrimitiveTopology, SceneBuilder, Vec3,
    validate_scene,
};

fn valid_triangle_scene() -> baozi_core::Scene {
    let mut builder = SceneBuilder::new();
    let material = builder.add_material(Material::default());
    let mesh = builder.add_mesh(Mesh {
        topology: PrimitiveTopology::Triangles,
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ],
        material: Some(material),
        ..Mesh::default()
    });
    builder
        .add_child_node(
            builder.root(),
            Node {
                meshes: vec![mesh],
                ..Node::default()
            },
        )
        .unwrap();
    builder.finish().unwrap()
}

fn assert_invalid(scene: &baozi_core::Scene, expected: &str) {
    let error = validate_scene(scene).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains(expected),
        "expected {message:?} to contain {expected:?}"
    );
}

#[test]
fn root_only_scene_is_valid() {
    let scene = SceneBuilder::new().finish().unwrap();

    validate_scene(&scene).unwrap();
}

#[test]
fn valid_triangle_scene_is_valid() {
    let scene = valid_triangle_scene();

    validate_scene(&scene).unwrap();
}

#[test]
fn missing_material_reference_fails() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].material = Some(MaterialId::new(99));

    assert_invalid(&scene, "material");
}

#[test]
fn mesh_reference_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.nodes[1].meshes[0] = MeshId::new(99);

    assert_invalid(&scene, "mesh");
}

#[test]
fn node_child_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.nodes[0].children.push(NodeId::new(99));

    assert_invalid(&scene, "child");
}

#[test]
fn mesh_index_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].indices = vec![0, 1, 99];

    assert_invalid(&scene, "index");
}

#[test]
fn non_finite_position_fails() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].positions[1] = Vec3::new(f32::NAN, 0.0, 0.0);

    assert_invalid(&scene, "finite");
}

#[test]
fn referenced_empty_mesh_fails() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].positions.clear();

    assert_invalid(&scene, "empty");
}
