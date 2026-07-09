#![no_main]

use baozi::Importer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut importer = Importer::empty();
    importer.register(baozi_format_ply::PlyImporter).unwrap();

    let _ = importer.read_bytes("fuzz.ply", data);
});
