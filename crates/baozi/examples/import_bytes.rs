use baozi::{ImportOptions, Importer, PostProcessPipeline, PostProcessStep, Result};

fn main() -> Result<()> {
    let bytes = b"o triangle
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let pipeline = PostProcessPipeline::new([
        PostProcessStep::Triangulate,
        PostProcessStep::GenerateBoundingBoxes,
    ]);
    let report = Importer::new().read_bytes_with_postprocess(
        "triangle.obj",
        bytes,
        ImportOptions::memory(),
        &pipeline,
    )?;

    println!(
        "format={} meshes={} vertices={} diagnostics={}",
        report.format().id(),
        report.scene().meshes.len(),
        report.stats().generated_vertices(),
        report.diagnostics().len()
    );
    Ok(())
}
