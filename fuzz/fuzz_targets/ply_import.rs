#![no_main]

mod support;

use baozi_core::ValidatedScene;
use baozi_format_ply::PlyImporter;
use baozi_import::{ImportOperationControl, PreparedImportRequest, import_asset};
use baozi_io::{AssetPath, MemoryAssetIo};
use libfuzzer_sys::fuzz_target;
use std::sync::{Arc, OnceLock};

fuzz_target!(|data: &[u8]| {
    static REQUEST: OnceLock<PreparedImportRequest> = OnceLock::new();
    let request = REQUEST.get_or_init(|| support::prepared_request(PlyImporter, "org.baozi.ply"));
    let source = AssetPath::new("fuzz.ply").expect("the static fuzz path must be valid");
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::<[u8]>::from(data))
        .expect("the fuzz asset insertion must succeed");

    if let Ok(report) = import_asset(request, &io, source, ImportOperationControl::default()) {
        let _: &ValidatedScene = report.scene();
    }
});
