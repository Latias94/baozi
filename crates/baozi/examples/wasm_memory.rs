use baozi::{Importer, Result};

fn import_from_memory(bytes: &[u8]) -> Result<usize> {
    let report = Importer::new().read_bytes("wasm-triangle.obj", bytes)?;
    Ok(report.scene().meshes.len())
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
