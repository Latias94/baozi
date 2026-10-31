#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{AttributeData, Importer, Result};

fn main() -> Result<()> {
    let bytes = b"o triangle
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let report = Importer::new()?.read_bytes("triangle.obj", bytes)?;
    let scene = report.scene();
    let mut index_count = 0;
    for mesh in scene.meshes() {
        for primitive in mesh.primitives() {
            index_count += primitive.indices().map_or(0, <[u32]>::len);
            if let Some(positions) = primitive.positions()
                && let AttributeData::DenseF32 { lanes: 3, values } = positions.data()
            {
                for position in values.as_chunks::<3>().0 {
                    println!("position={position:?}");
                }
            }
        }
    }

    println!(
        "format={} logical_meshes={} primitives={} indices={} diagnostics={}",
        report.format().id(),
        scene.mesh_count(),
        scene.primitive_count(),
        index_count,
        report.diagnostics().len()
    );
    Ok(())
}
