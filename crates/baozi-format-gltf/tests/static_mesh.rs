mod common;

use baozi_core::{
    AlphaMode, BaoziError, ColorSpace, MeshBinding, PrimitiveTopology, Result, TextureFilterMode,
    TextureRole, TextureSource, TextureWrapMode, Vec2, Vec3,
};
use baozi_test_support::SceneSnapshot;
use common::{
    data_uri_gltf, expected_error, import_assets, import_assets_result, interleaved_triangle_bin,
    interleaved_triangle_gltf, sidecar_options, triangle_bin, triangle_glb, triangle_gltf,
    triangle_gltf_with_buffer_uri,
};

#[test]
fn imports_static_mesh_material_texture_uri_and_hierarchy() -> Result<()> {
    let (scene, diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.nodes.len(), 3);
    assert_eq!(scene.nodes[1].name.as_deref(), Some("Root"));
    assert_eq!(scene.nodes[2].name.as_deref(), Some("TriangleNode"));
    assert_eq!(scene.nodes[1].children, vec![baozi_core::NodeId::new(2)]);
    assert_eq!(
        scene.nodes[2].mesh_bindings,
        vec![MeshBinding::new(baozi_core::MeshId::new(0))]
    );

    let mesh = &scene.meshes[0];
    assert_eq!(mesh.name.as_deref(), Some("Triangle"));
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions.len(), 3);
    assert_eq!(mesh.positions[1], Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(mesh.normals[0], Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(mesh.texcoords[0][2], Vec2::new(0.0, 1.0));
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(mesh.material, Some(baozi_core::MaterialId::new(0)));
    assert_eq!(mesh.bounds.unwrap().max, Vec3::new(1.0, 1.0, 0.0));

    let material = &scene.materials[0];
    assert_eq!(material.name.as_deref(), Some("Red"));
    assert_eq!(material.alpha_mode, AlphaMode::Blend);
    assert!(material.double_sided);
    assert_eq!(material.base_color.r, 0.8);
    assert_eq!(material.base_color.a, 0.7);
    assert_eq!(material.metallic, 0.5);
    assert_eq!(material.roughness, 0.25);
    assert_eq!(material.texture_slots.len(), 1);
    assert_eq!(material.texture_slots[0].role, TextureRole::BaseColor);
    assert_eq!(material.texture_slots[0].color_space, ColorSpace::Srgb);
    assert_eq!(
        material.texture_slots[0].source_key.as_deref(),
        Some("baseColorTexture")
    );

    let texture = &scene.textures[0];
    assert_eq!(texture.name.as_deref(), Some("BaseTex"));
    assert_eq!(texture.sampler.mag_filter, Some(TextureFilterMode::Linear));
    assert_eq!(
        texture.sampler.min_filter,
        Some(TextureFilterMode::LinearMipmapLinear)
    );
    assert_eq!(texture.sampler.wrap_s, TextureWrapMode::ClampToEdge);
    assert_eq!(texture.sampler.wrap_t, TextureWrapMode::MirroredRepeat);
    match &texture.source {
        TextureSource::External { uri } => assert_eq!(uri, "models/textures/base.png"),
        other => panic!("expected external texture, got {other:?}"),
    }
    Ok(())
}

#[test]
fn imports_glb_static_mesh_from_bin_chunk() -> Result<()> {
    let (scene, diagnostics) = import_assets(
        "models/scene.glb",
        [("models/scene.glb", triangle_glb())],
        baozi_import::ImportOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(scene.nodes[1].name.as_deref(), Some("TriangleNode"));
    assert_eq!(scene.meshes[0].positions.len(), 3);
    assert_eq!(scene.meshes[0].positions[1], Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(scene.meshes[0].indices, vec![0, 1, 2]);
    Ok(())
}

#[test]
fn imports_interleaved_byte_stride_mesh() -> Result<()> {
    let (scene, diagnostics) = import_assets(
        "models/interleaved.gltf",
        [
            ("models/interleaved.gltf", interleaved_triangle_gltf()),
            ("models/interleaved.bin", interleaved_triangle_bin()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.name.as_deref(), Some("InterleavedTriangle"));
    assert_eq!(mesh.positions.len(), 3);
    assert_eq!(mesh.positions[1], Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(mesh.normals.len(), 3);
    assert_eq!(mesh.normals[2], Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    Ok(())
}

#[test]
fn gltf_scene_snapshot_covers_imported_ir() -> Result<()> {
    let (scene, diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;

    let snapshot = SceneSnapshot::from_scene_with_diagnostics(&scene, &diagnostics).into_string();

    assert!(snapshot.contains("scene nodes=3 meshes=1 materials=1 textures=1"));
    assert!(snapshot.contains("space handedness=Right up=Some(PositiveY)"));
    assert!(snapshot.contains("metadata keys=[gltf:version]"));
    assert!(
        snapshot.contains("node 2 name=TriangleNode parent=1 children=[] meshes=[mesh:0 skin:-]")
    );
    assert!(snapshot.contains("mesh 0 name=Triangle topology=Triangles vertices=3"));
    assert!(snapshot.contains("texcoords[0] count=3 shown=3"));
    assert!(snapshot.contains("texture 0 name=BaseTex source=external:models/textures/base.png"));
    assert!(snapshot.contains("material 0 name=Red shading=PbrMetallicRoughness"));
    assert!(snapshot.contains("diagnostics count=0"));
    Ok(())
}

#[test]
fn external_buffer_denied_is_fatal() -> Result<()> {
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", triangle_gltf())],
        baozi_import::ImportOptions::memory(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("external buffer"));
    Ok(())
}

#[test]
fn missing_external_buffer_is_fatal() -> Result<()> {
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", triangle_gltf())],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Io { .. }));
    assert!(error.to_string().contains("triangle.bin"));
    Ok(())
}

#[test]
fn short_external_buffer_is_fatal() -> Result<()> {
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", vec![0; 10]),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("declares 104 bytes"));
    Ok(())
}

#[test]
fn unsupported_primitive_mode_is_fatal() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .map_err(|error| BaoziError::parse("test", None, error.to_string()))?
        .replace("\"mode\": 4", "\"mode\": 5")
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::FeatureUnsupported { .. }));
    assert!(error.to_string().contains("TriangleStrip"));
    Ok(())
}

#[test]
fn malformed_accessor_root_is_parse_error_not_panic() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(r#""accessors""#, r#""acceOsors""#)
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn invalid_index_accessor_type_is_parse_error_not_panic() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }"#,
            r#"{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "SCALAR" }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("indices accessor type"));
    Ok(())
}

#[test]
fn mismatched_attribute_count_is_parse_error_before_reader() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }"#,
            r#"{ "bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3" }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("NORMAL accessor count"));
    Ok(())
}

#[test]
fn integer_texcoord_must_be_normalized_before_reader() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" }"#,
            r#"{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "VEC2" }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("integer TEXCOORD accessor"));
    Ok(())
}

#[test]
fn short_buffer_view_is_parse_error_before_reader() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 }"#,
            r#"{ "buffer": 0, "byteOffset": 0, "byteLength": 24, "target": 34962 }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("byteLength does not cover"));
    Ok(())
}

#[test]
fn index_accessor_byte_stride_is_parse_error_before_reader() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }"#,
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "byteStride": 4, "target": 34963 }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("byteStride"));
    Ok(())
}

#[test]
fn sparse_accessor_is_explicitly_unsupported_before_reader() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(r#""byteLength": 104"#, r#""byteLength": 116"#)
        .replace(
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }"#,
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 },
    { "buffer": 0, "byteOffset": 102, "byteLength": 2 },
    { "buffer": 0, "byteOffset": 104, "byteLength": 12 }"#,
        )
        .replace(
            r#"{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] }"#,
            r#"{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0], "sparse": { "count": 1, "indices": { "bufferView": 4, "componentType": 5123 }, "values": { "bufferView": 5 } } }"#,
        )
        .into_bytes();
    let mut bin = triangle_bin();
    bin.resize(116, 0);
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", gltf), ("models/triangle.bin", bin)],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::FeatureUnsupported { .. }));
    assert!(error.to_string().contains("sparse accessor"));
    Ok(())
}

#[test]
fn float_accessor_must_not_be_normalized_before_reader() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] }"#,
            r#"{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "normalized": true, "min": [0, 0, 0], "max": [1, 1, 0] }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("float POSITION accessor"));
    Ok(())
}

#[test]
fn imports_base64_buffer_data_uri() -> Result<()> {
    let (scene, diagnostics) = import_assets(
        "scene.gltf",
        [("scene.gltf", data_uri_gltf())],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(scene.meshes[0].positions.len(), 3);
    assert_eq!(scene.meshes[0].indices, vec![0, 1, 2]);
    Ok(())
}

#[test]
fn data_uri_byte_limit_is_enforced_before_decode() -> Result<()> {
    let mut options = sidecar_options();
    options.limits.max_data_uri_bytes = 103;
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", data_uri_gltf())], options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_data_uri_bytes"
        }
    ));
    Ok(())
}

#[test]
fn data_uri_counts_against_total_asset_limit() -> Result<()> {
    let gltf = data_uri_gltf();
    let mut options = sidecar_options();
    options.limits.max_total_asset_bytes = gltf.len() as u64;
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_total_asset_bytes"
        }
    ));
    Ok(())
}

#[test]
fn malformed_data_uri_is_parse_error() -> Result<()> {
    let gltf = triangle_gltf_with_buffer_uri("data:application/octet-stream;base64AAAA", 104);
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], sidecar_options())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("comma"));
    Ok(())
}

#[test]
fn non_base64_data_uri_is_unsupported() -> Result<()> {
    let gltf = triangle_gltf_with_buffer_uri("data:application/octet-stream,AAAA", 104);
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], sidecar_options())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::FeatureUnsupported { .. }));
    assert!(error.to_string().contains("non-base64"));
    Ok(())
}

#[test]
fn invalid_base64_data_uri_is_parse_error() -> Result<()> {
    let gltf = triangle_gltf_with_buffer_uri("data:application/octet-stream;base64,@@@=", 104);
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], sidecar_options())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("invalid base64"));
    Ok(())
}

#[test]
fn short_data_uri_buffer_is_parse_error() -> Result<()> {
    let gltf = triangle_gltf_with_buffer_uri("data:application/octet-stream;base64,AAAA", 104);
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], sidecar_options())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("declares 104 bytes"));
    Ok(())
}

#[test]
fn vertex_limit_is_enforced_from_accessor_count() -> Result<()> {
    let mut options = sidecar_options();
    options.limits.max_vertices = 2;
    let (result, _) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", triangle_bin()),
        ],
        options,
    )?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_vertices"
        }
    ));
    Ok(())
}

#[test]
fn face_limit_is_enforced_from_index_accessor_count() -> Result<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(r#""byteLength": 104"#, r#""byteLength": 700"#)
        .replace(
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }"#,
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 600, "target": 34963 }"#,
        )
        .replace(
            r#"{ "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }"#,
            r#"{ "bufferView": 3, "componentType": 5123, "count": 300, "type": "SCALAR" }"#,
        )
        .into_bytes();
    let mut options = sidecar_options();
    options.limits.max_faces = 1;
    let (result, _) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", vec![0; 700]),
        ],
        options,
    )?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded { limit: "max_faces" }
    ));
    Ok(())
}
