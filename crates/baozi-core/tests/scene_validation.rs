use baozi_core::{
    Animation, AnimationChannel, AnimationInterpolation, AnimationProperty, AnimationTarget,
    AnimationValues, Camera, CameraId, CameraProjection, ColorSpace, Light, LightId, LightKind,
    Material, MaterialId, MaterialProperty, Mesh, MeshBinding, MeshId, Node, NodeId,
    PrimitiveTopology, SceneBuilder, Skin, Texture, TextureId, TextureRole, TextureSlot,
    TextureSource, Vec3, VertexAttribute, VertexAttributeData, VertexAttributeSemantic,
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
                mesh_bindings: vec![MeshBinding::new(mesh)],
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
                mesh_bindings: vec![MeshBinding::new(mesh)],
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
        sampler: Default::default(),
        metadata: Default::default(),
    });
    let material = builder.add_material(Material {
        texture_slots: vec![TextureSlot {
            texture,
            role: TextureRole::Diffuse,
            color_space: ColorSpace::Srgb,
            uv_set: 0,
            scale: 1.0,
            transform: Default::default(),
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
                mesh_bindings: vec![MeshBinding::new(mesh)],
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
        transform: Default::default(),
        source_key: Some("map_Kd".to_owned()),
    });

    assert_invalid(&scene, "texture reference");
}

#[test]
fn material_property_texture_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.materials[0].properties.insert(
        "obj:diffuse_texture".to_owned(),
        MaterialProperty::Texture(TextureId::new(99)),
    );

    assert_invalid(&scene, "texture reference");
}

#[test]
fn unnamespaced_material_property_key_fails() {
    let mut scene = valid_triangle_scene();
    scene.materials[0]
        .properties
        .insert("roughness".to_owned(), MaterialProperty::F64(1.0));

    assert_invalid(&scene, "must be namespaced");
}

#[test]
fn custom_attribute_length_mismatch_fails() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].custom_attributes.push(VertexAttribute {
        name: "ply:temperature".to_owned(),
        semantic: VertexAttributeSemantic::Custom("temperature".to_owned()),
        data: VertexAttributeData::F32(vec![1.0, 2.0]),
        metadata: Default::default(),
    });

    assert_invalid(&scene, "length");
}

#[test]
fn joint_channels_must_match_vertex_count() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].joint_indices = vec![[0, 1, 2, 3]];
    scene.meshes[0].joint_weights = vec![[1.0, 0.0, 0.0, 0.0]];

    assert_invalid(&scene, "joint indices and weights");
}

#[test]
fn joint_channels_require_skin() {
    let mut scene = valid_triangle_scene();
    scene.meshes[0].joint_indices = vec![[0, 0, 0, 0]; 3];
    scene.meshes[0].joint_weights = vec![[1.0, 0.0, 0.0, 0.0]; 3];

    assert_invalid(&scene, "require a skin");
}

#[test]
fn joint_channels_reject_unskinned_mesh_binding() {
    let mut scene = valid_triangle_scene();
    scene.skins.push(Skin {
        joints: vec![NodeId::new(1)],
        ..Skin::default()
    });
    scene.nodes[1].mesh_bindings[0].skin = Some(baozi_core::SkinId::new(0));
    scene.nodes[0]
        .mesh_bindings
        .push(MeshBinding::new(MeshId::new(0)));
    scene.meshes[0].joint_indices = vec![[0, 0, 0, 0]; 3];
    scene.meshes[0].joint_weights = vec![[1.0, 0.0, 0.0, 0.0]; 3];

    assert_invalid(&scene, "unskinned mesh binding");
}

#[test]
fn node_skin_reference_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.nodes[1].mesh_bindings[0].skin = Some(baozi_core::SkinId::new(99));

    assert_invalid(&scene, "skin reference");
}

#[test]
fn mesh_joint_indices_must_reference_skin_joints() {
    let mut scene = valid_triangle_scene();
    scene.skins.push(Skin {
        joints: vec![NodeId::new(1)],
        ..Skin::default()
    });
    scene.nodes[1].mesh_bindings[0].skin = Some(baozi_core::SkinId::new(0));
    scene.meshes[0].joint_indices = vec![[0, 1, 0, 0]; 3];
    scene.meshes[0].joint_weights = vec![[1.0, 0.0, 0.0, 0.0]; 3];

    assert_invalid(&scene, "joint index");
}

#[test]
fn mesh_joint_indices_must_reference_every_bound_skin() {
    let mut scene = valid_triangle_scene();
    scene.skins.push(Skin {
        joints: vec![NodeId::new(1), NodeId::new(0)],
        ..Skin::default()
    });
    scene.skins.push(Skin {
        joints: vec![NodeId::new(1)],
        ..Skin::default()
    });
    scene.nodes[1].mesh_bindings[0].skin = Some(baozi_core::SkinId::new(0));
    scene.nodes[0]
        .mesh_bindings
        .push(baozi_core::MeshBinding::skinned(
            MeshId::new(0),
            baozi_core::SkinId::new(1),
        ));
    scene.meshes[0].joint_indices = vec![[1, 0, 0, 0]; 3];
    scene.meshes[0].joint_weights = vec![[1.0, 0.0, 0.0, 0.0]; 3];

    assert_invalid(&scene, "bound skin");
}

#[test]
fn joint_weights_must_be_finite() {
    let mut scene = valid_triangle_scene();
    scene.skins.push(Skin {
        joints: vec![NodeId::new(1)],
        ..Skin::default()
    });
    scene.nodes[1].mesh_bindings[0].skin = Some(baozi_core::SkinId::new(0));
    scene.meshes[0].joint_indices = vec![[0, 0, 0, 0]; 3];
    scene.meshes[0].joint_weights = vec![
        [1.0, 0.0, 0.0, 0.0],
        [f32::NAN, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
    ];

    assert_invalid(&scene, "joint weights");
}

#[test]
fn skin_joint_reference_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.skins.push(Skin {
        joints: vec![NodeId::new(99)],
        ..Skin::default()
    });
    scene.nodes[1].mesh_bindings[0].skin = Some(baozi_core::SkinId::new(0));

    assert_invalid(&scene, "joint reference");
}

#[test]
fn skin_skeleton_root_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.skins.push(Skin {
        joints: vec![NodeId::new(1)],
        skeleton_root: Some(NodeId::new(99)),
        ..Skin::default()
    });

    assert_invalid(&scene, "skeleton root");
}

#[test]
fn skin_inverse_bind_matrix_count_must_match_joints() {
    let mut scene = valid_triangle_scene();
    scene.skins.push(Skin {
        joints: vec![NodeId::new(1)],
        inverse_bind_matrices: vec![baozi_core::Mat4::IDENTITY, baozi_core::Mat4::IDENTITY],
        ..Skin::default()
    });

    assert_invalid(&scene, "inverse bind matrix count");
}

#[test]
fn skin_inverse_bind_matrices_must_be_finite() {
    let mut scene = valid_triangle_scene();
    let mut matrix = baozi_core::Mat4::IDENTITY;
    matrix.cols[0][0] = f32::NAN;
    scene.skins.push(Skin {
        joints: vec![NodeId::new(1)],
        inverse_bind_matrices: vec![matrix],
        ..Skin::default()
    });

    assert_invalid(&scene, "inverse bind matrices");
}

#[test]
fn node_camera_reference_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.nodes[1].camera = Some(CameraId::new(99));

    assert_invalid(&scene, "camera reference");
}

#[test]
fn node_light_reference_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.nodes[1].light = Some(LightId::new(99));

    assert_invalid(&scene, "light reference");
}

#[test]
fn camera_projection_values_are_validated() {
    let mut scene = valid_triangle_scene();
    scene.cameras.push(Camera {
        projection: CameraProjection::Perspective {
            yfov_radians: f32::NAN,
            aspect_ratio: None,
            znear: 0.1,
            zfar: Some(100.0),
        },
        ..Camera::default()
    });

    assert_invalid(&scene, "yfov");
}

#[test]
fn light_values_are_validated() {
    let mut scene = valid_triangle_scene();
    scene.lights.push(Light {
        kind: LightKind::Point,
        intensity: -1.0,
        ..Light::default()
    });

    assert_invalid(&scene, "intensity");
}

#[test]
fn animation_target_and_sample_counts_are_validated() {
    let mut scene = valid_triangle_scene();
    scene.animations.push(Animation {
        channels: vec![AnimationChannel {
            target: AnimationTarget {
                node: NodeId::new(99),
                property: AnimationProperty::Translation,
            },
            interpolation: AnimationInterpolation::Linear,
            times_seconds: vec![0.0],
            values: AnimationValues::Translations(vec![Vec3::new(0.0, 0.0, 0.0)]),
            metadata: Default::default(),
        }],
        ..Animation::default()
    });

    assert_invalid(&scene, "target node");
}

#[test]
fn animation_value_count_mismatch_fails() {
    let mut scene = valid_triangle_scene();
    scene.animations.push(Animation {
        channels: vec![AnimationChannel {
            target: AnimationTarget {
                node: NodeId::new(0),
                property: AnimationProperty::Translation,
            },
            interpolation: AnimationInterpolation::Linear,
            times_seconds: vec![0.0, 1.0],
            values: AnimationValues::Translations(vec![Vec3::new(0.0, 0.0, 0.0)]),
            metadata: Default::default(),
        }],
        ..Animation::default()
    });

    assert_invalid(&scene, "value count");
}

#[test]
fn animation_value_kind_must_match_target_property() {
    let mut scene = valid_triangle_scene();
    scene.animations.push(Animation {
        channels: vec![AnimationChannel {
            target: AnimationTarget {
                node: NodeId::new(0),
                property: AnimationProperty::Translation,
            },
            interpolation: AnimationInterpolation::Linear,
            times_seconds: vec![0.0],
            values: AnimationValues::Rotations(vec![baozi_core::Vec4::new(0.0, 0.0, 0.0, 1.0)]),
            metadata: Default::default(),
        }],
        ..Animation::default()
    });

    assert_invalid(&scene, "value kind");
}

#[test]
fn mesh_reference_out_of_range_fails() {
    let mut scene = valid_triangle_scene();
    scene.nodes[1].mesh_bindings[0].mesh = MeshId::new(99);

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
