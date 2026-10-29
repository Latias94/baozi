#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{AssetPath, ImportDimension, Importer, MemoryAssetIo, Result};
use std::sync::Arc;

// A complete 1x1 RGB PNG containing one red pixel.
const RED_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xc9, 0xfe, 0x92, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

fn main() -> Result<()> {
    let model_path = AssetPath::new("models/triangle.obj")?;
    let material_path = AssetPath::new("models/materials/triangle.mtl")?;
    let texture_path = AssetPath::new("models/materials/textures/red.png")?;

    let io = MemoryAssetIo::new();
    io.try_insert(
        model_path.clone(),
        Arc::from(
            b"mtllib materials/triangle.mtl
usemtl red
v 0 0 0
v 1 0 0
v 0 1 0
vt 0 0
vt 1 0
vt 0 1
f 1/1 2/2 3/3
"
            .as_slice(),
        ),
    )?;
    io.try_insert(
        material_path,
        Arc::from(
            b"newmtl red
Kd 1 0 0
map_Kd textures/red.png
"
            .as_slice(),
        ),
    )?;
    io.try_insert(texture_path, Arc::from(RED_PNG))?;

    let report = Importer::new()?.read_asset(&io, model_path)?;
    let scene = report.scene();
    let opened_assets = report
        .ledger()
        .dimension(ImportDimension::AssetOpen)
        .map_or(0, |evidence| evidence.consumed());

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert!(report.diagnostics().is_empty());
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    assert_eq!(scene.material_count(), 1);
    assert_eq!(scene.image_count(), 1);
    assert_eq!(scene.sampler_count(), 1);
    assert_eq!(scene.texture_count(), 1);
    assert_eq!(scene.material_binding_count(), 1);
    let material = scene.materials().next().expect("one material");
    assert_eq!(material.name(), Some("red"));
    let binding = material.bindings().next().expect("one texture binding");
    assert_eq!(binding.texture().image().encoded_bytes(), RED_PNG);
    assert_eq!(opened_assets, 3);

    println!(
        "format={} materials={} images={} textures={} bindings={} opened_assets={}",
        report.format().id(),
        scene.material_count(),
        scene.image_count(),
        scene.texture_count(),
        scene.material_binding_count(),
        opened_assets
    );
    Ok(())
}
