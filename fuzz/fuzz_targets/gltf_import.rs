#![no_main]

use baozi::{AssetPath, ExternalReferencePolicy, ImportOptions, Importer, MemoryAssetIo};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let split = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let primary_bytes = &data[..split];
    let buffer_bytes = data.get(split.saturating_add(1)..).unwrap_or(&[]);
    let source_name = if primary_bytes.starts_with(b"glTF") {
        "fuzz.glb"
    } else {
        "fuzz.gltf"
    };

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

    let mut importer = Importer::empty();
    if importer.register(baozi_format_gltf::GltfImporter).is_err() {
        return;
    }

    let _ = importer.read_asset_with_options(&io, source_path, options);
});
