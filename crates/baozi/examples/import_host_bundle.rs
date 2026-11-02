//! Run with the directory containing the three explicitly approved relative paths below.
//! Host snapshots and their allocations belong to this application; import limits apply later.

use std::{error::Error, fs::File, io::Read, path::Path, sync::Arc};

use baozi::{
    AssetPath, ImportConfigOverlay, ImportOperationControl, ImportRequest, ImportRequestLayer,
    Importer, MemoryAssetIo, ObjImportOptions, OptionCompileLimits, ResourceLimits,
    ResourceLimitsOverlay,
};

const APPROVED: [&str; 3] = [
    "models/triangle.obj",
    "models/materials/triangle.mtl",
    "models/materials/textures/red.png",
];
const PER_FILE_BYTES: u64 = 8 * 1024 * 1024;
const TOTAL_BYTES: u64 = 16 * 1024 * 1024;

fn snapshot_bundle(directory: &Path) -> Result<MemoryAssetIo, Box<dyn Error>> {
    let io = MemoryAssetIo::new();
    let mut total = 0_u64;
    for approved in APPROVED {
        let path = AssetPath::new(approved)?;
        // The host deliberately opens only this finite approved list. Filesystem resolution,
        // including links, follows the host's policy; the importer receives only memory IO.
        let file = File::open(directory.join(approved))
            .map_err(|error| format!("could not snapshot approved file {approved}: {error}"))?;
        let allowed = PER_FILE_BYTES.min(TOTAL_BYTES - total);
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(usize::try_from(allowed + 1)?)?;
        file.take(allowed + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > allowed {
            return Err(
                format!("approved file {approved} exceeds the remaining snapshot cap").into(),
            );
        }
        total += bytes.len() as u64;
        io.try_insert(path, Arc::from(bytes))?;
    }
    Ok(io)
}

fn main() -> Result<(), Box<dyn Error>> {
    let directory = std::env::args_os().nth(1).ok_or(
        "usage: cargo run -p baozi --example import_host_bundle -- <approved-bundle-directory>",
    )?;
    let io = snapshot_bundle(Path::new(&directory))?;
    let importer = Importer::new()?;
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = PER_FILE_BYTES;
    limits.max_dependency_asset_bytes = PER_FILE_BYTES;
    limits.max_total_asset_bytes = TOTAL_BYTES;
    limits.max_live_bytes = 128 * 1024 * 1024;
    limits.max_open_assets = APPROVED.len() as u64;
    let layer = ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits)),
    )
    .try_with_format_option(ObjImportOptions::default().to_envelope()?)?;
    let plan = importer.plan(
        &ImportRequest::new().with_caller_layer(layer),
        OptionCompileLimits::default(),
    )?;
    let report = plan.read_asset(
        &io,
        AssetPath::new(APPROVED[0])?,
        ImportOperationControl::default(),
    )?;
    println!(
        "meshes={} primitives={} images={} diagnostics={}",
        report.scene().mesh_count(),
        report.scene().primitive_count(),
        report.scene().image_count(),
        report.diagnostics().len(),
    );
    Ok(())
}
