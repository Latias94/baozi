use baozi::{ImportOptions, Importer, PostProcessPipeline, PostProcessStep, Result};

fn main() -> Result<()> {
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
    ]);
    let report = Importer::new().read_bytes_with_postprocess(
        "quad.obj",
        bytes,
        ImportOptions::memory(),
        &pipeline,
    )?;
    let Some(mesh) = report.scene().meshes.first() else {
        println!("postprocess completed without meshes");
        return Ok(());
    };
    let Some(bounds) = mesh.bounds else {
        println!("postprocess completed without bounds");
        return Ok(());
    };

    println!(
        "stage={:?} topology={:?} vertices={} indices={} normals={} bounds=({}, {}, {})..({}, {}, {})",
        report.stage(),
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
