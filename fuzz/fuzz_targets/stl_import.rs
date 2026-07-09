#![no_main]

use baozi::Importer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = Importer::new().read_bytes("fuzz.stl", data);
});
