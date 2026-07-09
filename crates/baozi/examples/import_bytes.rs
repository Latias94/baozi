use baozi::{Importer, Result};

fn main() -> Result<()> {
    let bytes = b"o triangle
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let report = Importer::new().read_bytes("triangle.obj", bytes)?;

    println!(
        "format={} meshes={} vertices={} faces={} diagnostics={}",
        report.format().id(),
        report.scene().meshes.len(),
        report.stats().generated_vertices(),
        report.stats().generated_faces(),
        report.diagnostics().len()
    );
    Ok(())
}
