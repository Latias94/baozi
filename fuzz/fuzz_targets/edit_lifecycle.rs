#![no_main]

use baozi_fuzz::edit_lifecycle;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = edit_lifecycle::run(data);
});
