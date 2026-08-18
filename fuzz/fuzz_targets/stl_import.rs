#![no_main]

mod support;

use baozi_format_stl::{FORMAT_ID, StlImporter};
use baozi_import::{ImportOperationControl, PreparedImportRequest, import_asset};
use baozi_io::{AssetPath, MemoryAssetIo};
use libfuzzer_sys::fuzz_target;
use std::sync::{Arc, OnceLock};

fuzz_target!(|data: &[u8]| {
    static REQUEST: OnceLock<PreparedImportRequest> = OnceLock::new();
    let request = REQUEST.get_or_init(|| support::prepared_request(StlImporter, FORMAT_ID));
    let source = AssetPath::new("fuzz.stl").expect("the static fuzz path must be valid");
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::<[u8]>::from(data))
        .expect("the fuzz asset insertion must succeed");

    if let Ok(report) = import_asset(request, &io, source, ImportOperationControl::default()) {
        support::assert_validated_snapshot(report.scene());
    }
});
