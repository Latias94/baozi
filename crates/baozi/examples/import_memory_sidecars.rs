#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{AssetPath, ImportDimension, Importer, MemoryAssetIo, Result, SceneSnapshotLimits};
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
    let limits = SceneSnapshotLimits::try_new(16 * 1024, 64, 1_000_000)
        .expect("example snapshot limits must be valid");
    let scene = report
        .scene()
        .capture_snapshot(limits)
        .expect("validated Scene snapshot must succeed");
    let opened_assets = report
        .ledger()
        .dimension(ImportDimension::AssetOpen)
        .map_or(0, |evidence| evidence.consumed());

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
