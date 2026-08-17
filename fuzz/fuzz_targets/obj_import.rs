#![no_main]

mod support;

use baozi_format_obj::{FORMAT_ID, ObjImporter};
use baozi_import::{
    ImportOperationControl, PreparedImportRequest, SameAuthorityResolver, ValidatedScene,
    import_asset_with_resolver,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use libfuzzer_sys::fuzz_target;
use std::sync::{Arc, OnceLock};

fuzz_target!(|data: &[u8]| {
    static REQUEST: OnceLock<PreparedImportRequest> = OnceLock::new();
    let request = REQUEST.get_or_init(|| support::prepared_request(ObjImporter, FORMAT_ID));
    let obj_path = AssetPath::new("fuzz.obj").expect("the static fuzz path must be valid");
    let mtl_path = AssetPath::new("fuzz.mtl").expect("the static fuzz path must be valid");

    let split = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let obj_bytes = &data[..split];
    let mtl_bytes = data.get(split.saturating_add(1)..).unwrap_or(&[]);

    let io = MemoryAssetIo::new();
    io.try_insert(obj_path.clone(), Arc::<[u8]>::from(obj_bytes))
        .expect("the primary fuzz asset insertion must succeed");
    io.try_insert(mtl_path, Arc::<[u8]>::from(mtl_bytes))
        .expect("the dependency fuzz asset insertion must succeed");

    if let Ok(report) = import_asset_with_resolver(
        request,
        &io,
        obj_path,
        &SameAuthorityResolver,
        ImportOperationControl::default(),
    ) {
        let _: &ValidatedScene = report.scene();
    }
});
