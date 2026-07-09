mod common;

use baozi_core::{AlphaMode, ColorSpace, MetadataValue, Result, TextureRole, TextureSource, Vec3};
use baozi_import::{ExternalReferencePolicy, ImportOptions};
use common::import_assets;

fn sidecar_options() -> ImportOptions {
    let mut options = ImportOptions::memory();
    options.io.external_references = ExternalReferencePolicy::CustomResolver;
    options
}

#[test]
fn loads_mtllib_sidecar_and_binds_usemtl() -> Result<()> {
    let obj = b"mtllib materials/cube.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\n";
    let mtl = b"newmtl red\nKd 0.8 0.1 0.2\nd 0.5\nKe 0.1 0.2 0.3\nNs 32\nNi 1.5\nillum 2\nmap_Kd -s 1 1 1 textures/diffuse.png\n";

    let (scene, diagnostics) = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("materials/cube.mtl", mtl.as_slice()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.materials.len(), 1);
    assert_eq!(scene.textures.len(), 1);
    assert_eq!(scene.meshes[0].material.map(|id| id.index()), Some(0));

    let material = &scene.materials[0];
    assert_eq!(material.name.as_deref(), Some("red"));
    assert_eq!(material.base_color.r, 0.8);
    assert_eq!(material.base_color.g, 0.1);
    assert_eq!(material.base_color.b, 0.2);
    assert_eq!(material.base_color.a, 0.5);
    assert_eq!(material.alpha_mode, AlphaMode::Blend);
    assert_eq!(
        material.emissive,
        baozi_core::Color::linear_rgba(0.1, 0.2, 0.3, 1.0)
    );
    assert_eq!(
        material.metadata.get("obj:illum"),
        Some(&MetadataValue::I64(2))
    );
    assert_eq!(
        material.metadata.get("obj:Ns"),
        Some(&MetadataValue::F64(32.0))
    );
    assert_eq!(
        material.metadata.get("obj:Ni"),
        Some(&MetadataValue::F64(1.5))
    );
    assert_eq!(material.texture_slots.len(), 1);
    assert_eq!(material.texture_slots[0].role, TextureRole::Diffuse);
    assert_eq!(material.texture_slots[0].color_space, ColorSpace::Srgb);
    assert_eq!(
        material.texture_slots[0].source_key.as_deref(),
        Some("map_Kd")
    );

    match &scene.textures[0].source {
        TextureSource::External { uri } => {
            assert_eq!(uri, "materials/textures/diffuse.png");
        }
        other => panic!("expected external texture, got {other:?}"),
    }
    assert_eq!(scene.meshes[0].normals[0], Vec3::new(0.0, 0.0, 1.0));
    Ok(())
}

#[test]
fn denied_mtl_is_warning_not_geometry_failure() -> Result<()> {
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let (scene, diagnostics) = import_assets(
        "model.obj",
        [("model.obj", obj.as_slice())],
        ImportOptions::memory(),
    )?;

    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "obj.mtl_denied")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "obj.material_missing")
    );
    assert_eq!(scene.materials[0].name.as_deref(), Some("red"));
    Ok(())
}

#[test]
fn missing_mtl_is_warning_not_geometry_failure() -> Result<()> {
    let obj = b"mtllib missing.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let (scene, diagnostics) = import_assets(
        "model.obj",
        [("model.obj", obj.as_slice())],
        sidecar_options(),
    )?;

    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code.0, "obj.mtl_missing");
    Ok(())
}

#[test]
fn sidecar_byte_limit_warns_and_skips_mtl() -> Result<()> {
    let obj = b"mtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl red\nKd 1 0 0\n";
    let mut options = sidecar_options();
    options.limits.max_sidecar_asset_bytes = 4;

    let (scene, diagnostics) = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        options,
    )?;

    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code.0, "obj.mtl_limit_exceeded");
    Ok(())
}
