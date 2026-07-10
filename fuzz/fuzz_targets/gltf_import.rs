#![no_main]

use baozi::{AssetPath, ExternalReferencePolicy, ImportOptions, MemoryAssetIo, validate_scene};
use baozi_format_gltf::read_asset_for_development;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (source_name, primary_bytes, buffer_bytes) = split_fuzz_input(data);

    let Some(source_path) = AssetPath::new(source_name).ok() else {
        return;
    };
    let Some(buffer_path) = AssetPath::new("buffer.bin").ok() else {
        return;
    };

    let mut io = MemoryAssetIo::new();
    io.insert(source_path.clone(), primary_bytes);
    io.insert(buffer_path, buffer_bytes);

    let mut options = ImportOptions::memory();
    options.io.external_references = ExternalReferencePolicy::CustomResolver;

    if let Ok(scene) = read_asset_for_development(&io, source_path, options) {
        let _ = validate_scene(&scene);
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
