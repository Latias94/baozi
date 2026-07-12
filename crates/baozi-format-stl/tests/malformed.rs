mod common;

use baozi_import::ResourceLimits;
use common::import_bytes_with_limits;
use std::panic;

#[test]
fn malformed_inputs_do_not_panic_through_real_session() {
    let cases: &[&[u8]] = &[
        b"",
        b"solid",
        b"solid bad\nfacet normal 0 0 1\n",
        b"solid bad\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\n",
        b"\xff\xfe\xfd",
        &[0; 83],
    ];

    for case in cases {
        let outcome = panic::catch_unwind(|| {
            let _ = import_bytes_with_limits("malformed.stl", case, ResourceLimits::default());
        });
        assert!(outcome.is_ok());
    }
}
