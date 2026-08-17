#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{Importer, Result, SceneSnapshotLimits};

fn import_from_memory(bytes: &[u8]) -> Result<usize> {
    let report = Importer::new()?.read_bytes("wasm-triangle.obj", bytes)?;
    let limits = SceneSnapshotLimits::try_new(8 * 1024, 32, 500_000)
        .expect("example snapshot limits must be valid");
    let scene = report
        .scene()
        .capture_snapshot(limits)
        .expect("validated Scene snapshot must succeed");
    Ok(scene.logical_mesh_count())
}

fn main() -> Result<()> {
    let bytes = b"o wasm_triangle
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let mesh_count = import_from_memory(bytes)?;
    println!("meshes={mesh_count}");
    Ok(())
}
