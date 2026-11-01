#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{
    AttributeData, ImportConfigOverlay, ImportOperationControl, ImportRequest, ImportRequestLayer,
    Importer, ObjImportOptions, ObjTextDecodingMode, OptionCompileLimits, ResourceLimits,
    ResourceLimitsOverlay, Result,
};

fn main() -> Result<()> {
    let bytes = b"o triangle
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let importer = Importer::new()?;
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = 1024 * 1024;
    limits.max_total_asset_bytes = 1024 * 1024;
    limits.max_live_bytes = 16 * 1024 * 1024;
    let layer = ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits)),
    )
    .try_with_format_option(
        ObjImportOptions::default()
            .with_text_decoding_mode(ObjTextDecodingMode::RejectInvalid)
            .to_envelope()
            .expect("static OBJ option envelope is valid"),
    )
    .expect("one OBJ option fits the request");
    let request = importer.prepare_request(
        &ImportRequest::new().with_caller_layer(layer),
        OptionCompileLimits::default(),
    )?;
    let report = importer.read_bytes_with_request(
        "triangle.obj",
        bytes,
        &request,
        ImportOperationControl::default(),
    )?;
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
