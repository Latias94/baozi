#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{AssetPath, ImportDimension, Importer, MemoryAssetIo, Result, TextureSource};
use std::sync::Arc;

fn main() -> Result<()> {
    let model_path = AssetPath::new("models/triangle.obj")?;
    let material_path = AssetPath::new("models/materials/triangle.mtl")?;

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

    let report = Importer::new()?.read_asset(&io, model_path)?;
    let scene = report.scene();
    let texture_uri = scene
        .textures()
        .first()
        .and_then(|texture| match &texture.source {
            TextureSource::External { uri } => Some(uri.as_str()),
            TextureSource::Embedded { .. } => None,
        })
        .unwrap_or("<none>");
    let opened_assets = report
        .ledger()
        .dimension(ImportDimension::AssetOpen)
        .map_or(0, |evidence| evidence.consumed());

    println!(
        "format={} materials={} textures={} texture_uri={} opened_assets={}",
        report.format().id(),
        scene.materials().len(),
        scene.textures().len(),
        texture_uri,
        opened_assets
    );
    Ok(())
}
