#![no_main]

use baozi::{ImportOptions, Importer, PostProcessPipeline, PostProcessStep};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut importer = Importer::empty();
    importer.register(baozi_format_obj::ObjImporter).unwrap();
    let pipeline =
        PostProcessPipeline::new([PostProcessStep::Triangulate, PostProcessStep::GenerateBoundingBoxes]);

    let _ = importer.read_bytes_with_postprocess(
        "fuzz.obj",
        data,
        ImportOptions::memory(),
        &pipeline,
    );
});
