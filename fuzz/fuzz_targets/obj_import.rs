#![no_main]

use baozi::{AssetPath, ExternalReferencePolicy, ImportOptions, Importer, MemoryAssetIo};
use libfuzzer_sys::fuzz_target;
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    let Some(obj_path) = AssetPath::new("fuzz.obj").ok() else {
        return;
    };
    let Some(mtl_path) = AssetPath::new("fuzz.mtl").ok() else {
        return;
    };

    let split = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let obj_bytes = &data[..split];
    let mtl_bytes = data.get(split.saturating_add(1)..).unwrap_or(&[]);

    let mut io = MemoryAssetIo::new();
    io.insert(obj_path.clone(), Arc::<[u8]>::from(obj_bytes));
    io.insert(mtl_path, Arc::<[u8]>::from(mtl_bytes));

    let mut options = ImportOptions::memory();
    options.io.external_references = ExternalReferencePolicy::CustomResolver;

    let mut importer = Importer::empty();
    importer.register(baozi_format_obj::ObjImporter);

    let _ = importer.read_asset_with_options(&io, obj_path, options);
});
