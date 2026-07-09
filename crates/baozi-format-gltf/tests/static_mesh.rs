mod common;

use baozi_core::{
    AlphaMode, BaoziError, ColorSpace, PrimitiveTopology, Result, TextureFilterMode, TextureRole,
    TextureSource, TextureWrapMode, Vec2, Vec3,
};
use baozi_test_support::SceneSnapshot;
use common::{
    data_uri_gltf, expected_error, import_assets, import_assets_result, sidecar_options,
    triangle_bin, triangle_glb, triangle_gltf,
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
    assert_eq!(scene.nodes[2].meshes, vec![baozi_core::MeshId::new(0)]);

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
    assert!(snapshot.contains("node 2 name=TriangleNode parent=1 children=[] meshes=[0]"));
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
fn data_uri_buffers_are_explicitly_unsupported() -> Result<()> {
    let (result, diagnostics) = import_assets_result(
        "scene.gltf",
        [("scene.gltf", data_uri_gltf())],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::FeatureUnsupported { .. }));
    assert!(error.to_string().contains("data URIs"));
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
