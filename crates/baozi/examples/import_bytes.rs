#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{Importer, Result, SceneSnapshotLimits};

fn main() -> Result<()> {
    let bytes = b"o triangle
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let report = Importer::new()?.read_bytes("triangle.obj", bytes)?;
    let limits = SceneSnapshotLimits::try_new(16 * 1024, 64, 1_000_000)
        .expect("example snapshot limits must be valid");
    let scene = report
        .scene()
        .capture_snapshot(limits)
        .expect("validated Scene snapshot must succeed");

    println!(
        "format={} logical_meshes={} primitives={} indices={} diagnostics={}",
        report.format().id(),
        scene.logical_mesh_count(),
        scene.primitive_count(),
        scene.primitive_index_count(),
        report.diagnostics().len()
    );
    Ok(())
}
