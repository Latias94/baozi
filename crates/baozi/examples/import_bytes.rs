#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{Importer, Result};

fn main() -> Result<()> {
    let bytes = b"o triangle
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let report = Importer::new()?.read_bytes("triangle.obj", bytes)?;
    let vertices = report
        .scene()
        .meshes()
        .iter()
        .map(|mesh| mesh.positions.len())
        .sum::<usize>();
    let elements = report
        .scene()
        .meshes()
        .iter()
        .map(|mesh| mesh.element_count())
        .sum::<usize>();

    println!(
        "format={} meshes={} vertices={} elements={} diagnostics={}",
        report.format().id(),
        report.scene().meshes().len(),
        vertices,
        elements,
        report.diagnostics().len()
    );
    Ok(())
}
