#![no_main]

mod support;

use baozi_core::ValidatedScene;
use baozi_format_gltf::{FORMAT_ID, GltfImporter};
use baozi_import::{
    ImportOperationControl, PreparedImportRequest, SameAuthorityResolver,
    import_asset_with_resolver,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use libfuzzer_sys::fuzz_target;
use std::sync::{Arc, OnceLock};

fuzz_target!(|data: &[u8]| {
    static REQUEST: OnceLock<PreparedImportRequest> = OnceLock::new();
    let request = REQUEST.get_or_init(|| support::prepared_request(GltfImporter, FORMAT_ID));
    let (source_name, primary_bytes, buffer_bytes) = split_fuzz_input(data);

    let source_path = AssetPath::new(source_name).expect("the static fuzz path must be valid");
    let buffer_path = AssetPath::new("buffer.bin").expect("the static fuzz path must be valid");

    let io = MemoryAssetIo::new();
    io.try_insert(source_path.clone(), Arc::<[u8]>::from(primary_bytes))
        .expect("the primary fuzz asset insertion must succeed");
    io.try_insert(buffer_path, Arc::<[u8]>::from(buffer_bytes))
        .expect("the dependency fuzz asset insertion must succeed");

    if let Ok(report) = import_asset_with_resolver(
        request,
        &io,
        source_path,
        &SameAuthorityResolver,
        ImportOperationControl::default(),
    ) {
        let _: &ValidatedScene = report.scene();
    }
});

fn split_fuzz_input(data: &[u8]) -> (&'static str, &[u8], &[u8]) {
    if data.starts_with(b"glTF") {
        return ("fuzz.glb", data, &[]);
    }

    let split = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let primary_bytes = &data[..split];
    let buffer_bytes = data.get(split.saturating_add(1)..).unwrap_or(&[]);
    ("fuzz.gltf", primary_bytes, buffer_bytes)
}
