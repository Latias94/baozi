use baozi_format_ply::PlyImporter;
use baozi_import::{
    DetectionFailure, DetectionInconclusive, DetectionOptions, DetectionOutcome, ImporterRegistry,
    ReadConfidence, ReadHint,
};
use baozi_io::AssetPath;
use std::io::Cursor;

fn detect(
    source: &str,
    bytes: &[u8],
    max_probe_bytes: u64,
) -> std::result::Result<ReadConfidence, DetectionFailure> {
    let mut registry = ImporterRegistry::new();
    registry.register(PlyImporter).expect("unique PLY importer");
    let hint = ReadHint::from_source(AssetPath::new(source).expect("valid test path"));
    let mut input = Cursor::new(bytes.to_vec());
    registry
        .detect_with_options(&mut input, &hint, &DetectionOptions { max_probe_bytes })
        .map(|selected| selected.confidence)
}

#[test]
fn detects_ply_magic() {
    let confidence =
        detect("mesh.bin", b"ply\nformat ascii 1.0\n", 4096).expect("PLY magic is detected");

    assert_eq!(confidence, ReadConfidence::Certain);
}

#[test]
fn detects_crlf_ply_magic_at_the_exact_boundary() {
    let bytes = b"ply\r\nformat ascii 1.0\r\n";

    let error = detect("mesh.bin", bytes, 4)
        .expect_err("the CRLF magic is incomplete before the line feed");
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

    let confidence = detect("mesh.bin", bytes, 5).expect("the complete CRLF magic is sufficient");
    assert_eq!(confidence, ReadConfidence::Certain);
}

#[test]
fn extension_is_maybe_without_magic() {
    let confidence = detect("mesh.ply", b"not-ply", 4096).expect("the extension is weak evidence");

    assert_eq!(confidence, ReadConfidence::Maybe);
}

#[test]
fn truncated_magic_is_inconclusive_when_probe_budget_is_exhausted() {
    let error = detect("mesh.bin", b"ply\nformat ascii 1.0\n", 3)
        .expect_err("three bytes cannot establish the PLY magic");
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

#[test]
fn impossible_truncated_magic_is_a_definitive_non_match() {
    let error = detect("mesh.bin", b"plx and more input", 3)
        .expect_err("a mismatching third byte rules out both PLY magic variants");
    let DetectionFailure::NoMatch { evidence, .. } = error else {
        panic!("expected structured detection evidence");
    };
    assert!(matches!(
        evidence.as_slice(),
        [evidence] if evidence.outcome() == &DetectionOutcome::NoMatch
    ));
}
