#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{Importer, PostProcessPipeline, PostProcessStep, ProcessedImportResult};

fn main() -> ProcessedImportResult<()> {
    let bytes = b"o quad
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
f 1 2 3 4
";
    let pipeline = PostProcessPipeline::new([
        PostProcessStep::Triangulate,
        PostProcessStep::GenerateNormals,
        PostProcessStep::GenerateBoundingBoxes,
    ])
    .expect("the static example post-process plan must be valid");
    let report = Importer::new()?.read_bytes_with_postprocess("quad.obj", bytes, &pipeline)?;
    let Some(mesh) = report.scene().meshes().first() else {
        println!("postprocess completed without meshes");
        return Ok(());
    };
    let Some(bounds) = mesh.bounds else {
        println!("postprocess completed without bounds");
        return Ok(());
    };

    println!(
        "import_phase={:?} topology={:?} vertices={} indices={} normals={} bounds=({}, {}, {})..({}, {}, {})",
        report.import().last_completed().phase(),
        mesh.topology,
        mesh.positions.len(),
        mesh.indices.len(),
        mesh.normals.len(),
        bounds.min.x,
        bounds.min.y,
        bounds.min.z,
        bounds.max.x,
        bounds.max.y,
        bounds.max.z
    );
    Ok(())
}
