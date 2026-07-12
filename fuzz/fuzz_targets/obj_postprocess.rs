#![no_main]

mod support;

use baozi_core::ValidatedScene;
use baozi_format_obj::{FORMAT_ID, ObjImporter};
use baozi_import::{
    DenyDependencies, ImportOperationControl, PreparedImportRequest, import_asset_with_resolver,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use baozi_postprocess::{PostProcessPipeline, PostProcessStep};
use libfuzzer_sys::fuzz_target;
use std::sync::{Arc, OnceLock};

fuzz_target!(|data: &[u8]| {
    static REQUEST: OnceLock<PreparedImportRequest> = OnceLock::new();
    let request = REQUEST.get_or_init(|| support::prepared_request(ObjImporter, FORMAT_ID));
    let source = AssetPath::new("fuzz.obj").expect("the static fuzz path must be valid");
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::<[u8]>::from(data))
        .expect("the fuzz asset insertion must succeed");
    let pipeline = PostProcessPipeline::new([
        PostProcessStep::Triangulate,
        PostProcessStep::GenerateNormals,
        PostProcessStep::GenerateBoundingBoxes,
    ])
    .expect("the static fuzz post-process plan must be valid");

    if let Ok(import_report) = import_asset_with_resolver(
        request,
        &io,
        source,
        &DenyDependencies,
        ImportOperationControl::default(),
    ) {
        let imported_scene: &ValidatedScene = import_report.scene();
        if let Ok(postprocess_report) = pipeline.run(imported_scene) {
            let _: &ValidatedScene = postprocess_report.scene();
        }
    }
});
