use baozi_core::MeshBinding;
use baozi_core::{
    Animation, AnimationChannel, AnimationInterpolation, AnimationProperty, AnimationTarget,
    AnimationValues, Camera, CameraProjection, ColorSpace, Diagnostic, DiagnosticCode,
    DiagnosticSeverity, Light, LightKind, Material, Mesh, MetadataValue, Node, PrimitiveTopology,
    SceneBuilder, Skin, SourceLocation, Texture, TextureRole, TextureSlot, TextureSource,
    ValidatedScene, Vec2, Vec3, Vec4, VertexAttribute, VertexAttributeData,
    VertexAttributeSemantic, validate_scene,
};
use baozi_test_support::SceneSnapshot;

fn publish(builder: SceneBuilder) -> ValidatedScene {
    validate_scene(builder.finish_draft()).expect("test scene should validate")
}

fn triangle_scene() -> ValidatedScene {
    let mut builder = SceneBuilder::try_new().expect("test scene builder allocation");
    let material = builder
        .add_material(Material::default())
        .expect("test material id should fit");
    let mesh = builder
        .add_mesh(Mesh {
            topology: PrimitiveTopology::Triangles,
            positions: vec![
                Vec3::new(-0.0, 0.0, 0.0),
                Vec3::new(1.25, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            normals: vec![
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            indices: vec![0, 1, 2],
            material: Some(material),
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
        .unwrap();
    publish(builder)
}

#[test]
fn empty_root_scene_snapshot_is_deterministic() {
    let scene = publish(SceneBuilder::try_new().expect("test scene builder allocation"));

    let first = SceneSnapshot::from_scene(&scene);
    let second = SceneSnapshot::from_scene(&scene);

    assert_eq!(first, second);
    assert!(first.as_str().contains("scene nodes=1 meshes=0"));
    assert!(first.as_str().contains("root 0"));
}

#[test]
fn triangle_snapshot_includes_core_mesh_fields() {
    let snapshot = SceneSnapshot::from_scene(&triangle_scene()).into_string();

    assert!(snapshot.contains("mesh 0"));
    assert!(snapshot.contains("topology=Triangles"));
    assert!(snapshot.contains("material=0"));
    assert!(snapshot.contains("positions count=3 shown=3"));
    assert!(snapshot.contains("normals count=3 shown=3"));
    assert!(snapshot.contains("indices=[0,1,2]"));
    assert!(snapshot.contains("face_vertex_counts=[]"));
}

#[test]
fn polygon_snapshot_includes_face_counts() {
    let mut builder = SceneBuilder::try_new().expect("test scene builder allocation");
    let mesh = builder
        .add_mesh(Mesh {
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
        .unwrap();
    let scene = publish(builder);

    let snapshot = SceneSnapshot::from_scene(&scene).into_string();

    assert!(snapshot.contains("topology=Polygons"));
    assert!(snapshot.contains("faces=1"));
    assert!(snapshot.contains("face_vertex_counts=[4]"));
}

#[test]
fn material_texture_snapshot_includes_reviewable_fields() {
    let mut builder = SceneBuilder::try_new().expect("test scene builder allocation");
    let texture = builder
        .add_texture(Texture {
            name: Some("diffuse".to_owned()),
            source: TextureSource::External {
                uri: "textures/diffuse.png".to_owned(),
            },
            sampler: Default::default(),
            metadata: Default::default(),
        })
        .expect("test texture id should fit");
    let mut material = Material {
        name: Some("paint".to_owned()),
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
    };
    material
        .metadata
        .try_insert("obj:illum".to_owned(), MetadataValue::I64(2))
        .expect("test material metadata should reserve");
    let material = builder
        .add_material(material)
        .expect("test material id should fit");
    let mesh = builder
        .add_mesh(Mesh {
            topology: PrimitiveTopology::Triangles,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            texcoords: vec![vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
            ]],
            indices: vec![0, 1, 2],
            material: Some(material),
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
        .unwrap();
    let scene = publish(builder);

    let snapshot = SceneSnapshot::from_scene(&scene).into_string();

    assert!(snapshot.contains("textures=1"));
    assert!(snapshot.contains("texcoords[0] count=3 shown=3"));
    assert!(snapshot.contains("texture 0 name=diffuse source=external:textures/diffuse.png"));
    assert!(snapshot.contains("material 0 name=paint"));
    assert!(snapshot.contains("metadata=[obj:illum]"));
    assert!(snapshot.contains(
        "slot 0 texture=0 role=Diffuse color_space=Srgb uv_set=0 scale=1.000000 transform=offset(0.000000,0.000000) rotation=0.000000 scale(1.000000,1.000000) texcoord=<none> source_key=map_Kd"
    ));
}

#[test]
fn extended_ir_snapshot_includes_reviewable_fields() {
    let mut builder = SceneBuilder::try_new().expect("test scene builder allocation");
    let camera = builder
        .add_camera(Camera {
            name: Some("MainCamera".to_owned()),
            projection: CameraProjection::Perspective {
                yfov_radians: 1.0,
                aspect_ratio: Some(1.777778),
                znear: 0.1,
                zfar: Some(1000.0),
            },
            metadata: Default::default(),
        })
        .expect("test camera id should fit");
    let light = builder
        .add_light(Light {
            name: Some("Key".to_owned()),
            kind: LightKind::Directional,
            intensity: 10.0,
            ..Light::default()
        })
        .expect("test light id should fit");
    let child = builder
        .add_child_node(
            builder.root(),
            Node {
                name: Some("RigRoot".to_owned()),
                camera: Some(camera),
                light: Some(light),
                ..Node::default()
            },
        )
        .unwrap();
    let skin = builder
        .add_skin(Skin {
            name: Some("Skin".to_owned()),
            joints: vec![child],
            inverse_bind_matrices: vec![baozi_core::Mat4::IDENTITY],
            skeleton_root: Some(child),
            metadata: Default::default(),
        })
        .expect("test skin id should fit");
    let mesh = builder
        .add_mesh(Mesh {
            topology: PrimitiveTopology::Triangles,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            tangents: vec![
                Vec4::new(1.0, 0.0, 0.0, 1.0),
                Vec4::new(1.0, 0.0, 0.0, 1.0),
                Vec4::new(1.0, 0.0, 0.0, 1.0),
            ],
            joint_indices: vec![[0, 0, 0, 0]; 3],
            joint_weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
            custom_attributes: vec![VertexAttribute {
                name: "ply:temperature".to_owned(),
                semantic: VertexAttributeSemantic::Custom("temperature".to_owned()),
                data: VertexAttributeData::F32(vec![0.5, 0.75, 1.0]),
                metadata: Default::default(),
            }],
            ..Mesh::default()
        })
        .expect("test mesh id should fit");
    builder
        .add_child_node(
            child,
            Node {
                name: Some("MeshNode".to_owned()),
                mesh_bindings: vec![MeshBinding::skinned(mesh, skin)],
                ..Node::default()
            },
        )
        .unwrap();
    builder
        .add_animation(Animation {
            name: Some("Move".to_owned()),
            channels: vec![AnimationChannel {
                target: AnimationTarget {
                    node: child,
                    property: AnimationProperty::Translation,
                },
                interpolation: AnimationInterpolation::Linear,
                times_seconds: vec![0.0, 1.0],
                values: AnimationValues::Translations(vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                ]),
                metadata: Default::default(),
            }],
            metadata: Default::default(),
        })
        .expect("test animation id should fit");
    let scene = publish(builder);

    let snapshot = SceneSnapshot::from_scene(&scene).into_string();

    assert!(snapshot.contains("skins=1"));
    assert!(snapshot.contains("camera=0 light=0"));
    assert!(snapshot.contains("skin 0 name=Skin joints=[1] inverse_bind_matrices=1"));
    assert!(snapshot.contains("camera 0 name=MainCamera projection=perspective"));
    assert!(snapshot.contains("light 0 name=Key kind=Directional"));
    assert!(snapshot.contains("tangents count=3 shown=3"));
    assert!(snapshot.contains("custom_attribute 0 name=ply:temperature"));
    assert!(snapshot.contains("animation 0 name=Move channels=1"));
}

#[test]
fn float_formatting_is_stable() {
    let snapshot = SceneSnapshot::from_scene(&triangle_scene()).into_string();

    assert!(snapshot.contains("(0.000000,0.000000,0.000000)"));
    assert!(snapshot.contains("(1.250000,0.000000,0.000000)"));
    assert!(!snapshot.contains("-0.000000"));
}

#[test]
fn metadata_keys_are_sorted() {
    let scene = triangle_scene();
    let mut edit = scene.begin_edit().expect("test edit should reserve");
    edit.metadata_mut()
        .try_insert("test:z-key".to_owned(), MetadataValue::Bool(true))
        .expect("test metadata should reserve");
    edit.metadata_mut()
        .try_insert(
            "test:a-key".to_owned(),
            MetadataValue::String("first".to_owned()),
        )
        .expect("test metadata should reserve");
    let scene = edit.commit().expect("edited scene should validate");

    let snapshot = SceneSnapshot::from_scene(&scene).into_string();

    assert!(snapshot.contains("metadata keys=[test:a-key,test:z-key]"));
}

#[test]
fn diagnostics_sort_deterministically() {
    let scene = triangle_scene();
    let diagnostics = vec![
        Diagnostic {
            severity: DiagnosticSeverity::Info,
            code: DiagnosticCode("z.info"),
            source: Some("mesh.stl".to_owned()),
            location: Some(SourceLocation::line_column(3, 1)),
            message: "later".to_owned(),
        },
        Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: DiagnosticCode("a.error"),
            source: Some("mesh.stl".to_owned()),
            location: Some(SourceLocation::line_column(1, 1)),
            message: "first".to_owned(),
        },
        Diagnostic::warning("m.warning", "middle"),
    ];

    let snapshot = SceneSnapshot::from_scene_with_diagnostics(&scene, &diagnostics).into_string();
    let error_pos = snapshot.find("code=a.error").unwrap();
    let warning_pos = snapshot.find("code=m.warning").unwrap();
    let info_pos = snapshot.find("code=z.info").unwrap();

    assert!(error_pos < warning_pos);
    assert!(warning_pos < info_pos);
}

#[test]
fn topology_changes_snapshot_text() {
    let original = triangle_scene();
    let mut edit = original.begin_edit().expect("test edit should reserve");
    edit.meshes_mut()[0].topology = PrimitiveTopology::Points;
    let changed = edit.commit().expect("edited scene should validate");

    let original = SceneSnapshot::from_scene(&original);
    let changed = SceneSnapshot::from_scene(&changed);

    assert_ne!(original, changed);
}
