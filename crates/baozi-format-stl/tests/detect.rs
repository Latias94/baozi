mod common;

use baozi_format_stl::StlImporter;
use baozi_import::{
    DetectionFailure, DetectionInconclusive, DetectionOptions, DetectionOutcome, ImporterRegistry,
    ReadConfidence, ReadHint,
};
use baozi_io::AssetPath;
use common::{BinaryFacet, ascii_triangle, binary_stl};
use std::io::Cursor;

fn detect(
    source: &str,
    bytes: Vec<u8>,
    max_probe_bytes: u64,
) -> std::result::Result<ReadConfidence, DetectionFailure> {
    let mut registry = ImporterRegistry::new();
    registry.register(StlImporter).expect("unique STL importer");
    let hint = ReadHint::from_source(AssetPath::new(source).expect("valid test path"));
    let mut input = Cursor::new(bytes);
    registry
        .detect_with_options(&mut input, &hint, &DetectionOptions { max_probe_bytes })
        .map(|selected| selected.confidence)
}

#[test]
fn binary_exact_size_is_certain() {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let confidence = detect("triangle.stl", bytes, 4096).expect("binary STL is detected");

    assert_eq!(confidence, ReadConfidence::Certain);
}

#[test]
fn binary_header_starting_with_solid_is_still_binary() {
    let bytes = binary_stl(b"solid binary header", &[BinaryFacet::unit_triangle()]);
    let confidence = detect("solid-header.stl", bytes, 4096)
        .expect("binary size evidence outranks an ASCII-looking header");

    assert_eq!(confidence, ReadConfidence::Certain);
}

#[test]
fn ascii_solid_after_whitespace_is_likely() {
    let bytes = format!("\n\t{}", ascii_triangle("triangle"));
    let confidence =
        detect("triangle.stl", bytes.into_bytes(), 4096).expect("ASCII STL is detected");

    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn ascii_solid_after_utf8_bom_is_likely() {
    let bytes = format!("\u{feff}{}", ascii_triangle("triangle"));
    let confidence = detect("triangle.stl", bytes.into_bytes(), 4096)
        .expect("BOM-prefixed ASCII STL is detected");

    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn ascii_solid_requires_a_complete_keyword_boundary() {
    let bytes = ascii_triangle("triangle").into_bytes();
    let error = detect("triangle.mesh", bytes.clone(), 5)
        .expect_err("the byte after solid may still continue the keyword");
    let DetectionFailure::NoMatch { evidence, .. } = error else {
        panic!("expected structured detection evidence");
    };
    assert!(matches!(
        evidence.as_slice(),
        [evidence]
            if evidence.outcome()
                == &DetectionOutcome::Inconclusive(
                    DetectionInconclusive::ProbeExhausted
                )
    ));

    let confidence =
        detect("triangle.mesh", bytes, 6).expect("solid followed by whitespace is sufficient");
    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn random_bytes_are_not_stl() {
    let error = detect("triangle.stl", b"baozi is not an stl".to_vec(), 4096)
        .expect_err("random bytes must not match STL");

    assert!(matches!(error, DetectionFailure::NoMatch { .. }));
}

#[test]
fn short_binary_probe_is_inconclusive() {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let error =
        detect("triangle.stl", bytes, 83).expect_err("the binary facet count is outside the probe");
    let DetectionFailure::NoMatch { evidence, .. } = error else {
        panic!("expected structured detection evidence");
    };
    assert!(matches!(
        evidence.as_slice(),
        [evidence]
            if evidence.outcome()
                == &DetectionOutcome::Inconclusive(
                    DetectionInconclusive::ProbeExhausted
                )
    ));
}
