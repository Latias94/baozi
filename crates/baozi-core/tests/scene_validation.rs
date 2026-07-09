use baozi_core::{
    ColorSpace, Material, MaterialId, Mesh, MeshId, Node, NodeId, PrimitiveTopology, SceneBuilder,
    Texture, TextureId, TextureRole, TextureSlot, TextureSource, Vec3, validate_scene,
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

fn valid_polygon_scene() -> baozi_core::Scene {
    let mut builder = SceneBuilder::new();
    let mesh = builder.add_mesh(Mesh {
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
fn valid_polygon_scene_is_valid() {
    let scene = valid_polygon_scene();

    validate_scene(&scene).unwrap();
}

#[test]
fn polygon_without_face_counts_fails() {
    let mut scene = valid_polygon_scene();
    scene.meshes[0].face_vertex_counts.clear();

    assert_invalid(&scene, "face range");
}

#[test]
fn polygon_face_count_mismatch_fails() {
    let mut scene = valid_polygon_scene();
    scene.meshes[0].face_vertex_counts = vec![3];

    assert_invalid(&scene, "does not match");
}

#[test]
fn polygon_face_with_too_few_vertices_fails() {
    let mut scene = valid_polygon_scene();
    scene.meshes[0].indices = vec![0, 1];
    scene.meshes[0].face_vertex_counts = vec![2];

    assert_invalid(&scene, "fewer than 3 vertices");
}

#[test]
fn non_polygon_face_counts_fail() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].face_vertex_counts = vec![3];

    assert_invalid(&scene, "only valid for polygon");
}

#[test]
fn missing_material_reference_fails() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].material = Some(MaterialId::new(99));

    assert_invalid(&scene, "material");
}

#[test]
fn material_texture_slot_reference_is_valid() {
    let mut builder = SceneBuilder::new();
    let texture = builder.add_texture(Texture {
        name: Some("diffuse".to_owned()),
        source: TextureSource::External {
            uri: "textures/diffuse.png".to_owned(),
        },
    });
    let material = builder.add_material(Material {
        texture_slots: vec![TextureSlot {
            texture,
            role: TextureRole::Diffuse,
            color_space: ColorSpace::Srgb,
            uv_set: 0,
            scale: 1.0,
            source_key: Some("map_Kd".to_owned()),
        }],
        ..Material::default()
    });
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
    let scene = builder.finish().unwrap();

    validate_scene(&scene).unwrap();
}

#[test]
fn material_texture_slot_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.materials[0].texture_slots.push(TextureSlot {
        texture: TextureId::new(99),
        role: TextureRole::Diffuse,
        color_space: ColorSpace::Srgb,
        uv_set: 0,
        scale: 1.0,
        source_key: Some("map_Kd".to_owned()),
    });

    assert_invalid(&scene, "texture reference");
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
