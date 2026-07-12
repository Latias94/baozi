mod common;

use baozi_format_ply::PlyFailureCode;
use baozi_import::{
    CauseOwner, DetectionInconclusive, DetectionOptions, DetectionOutcome, ImportCause,
    ReadConfidence, ResourceLimits,
};
use common::{
    TestResult, expected_ply_failure, import_bytes_with_detection, operation_failure,
    ply_unsigned_detail,
};

fn detect(
    source: &str,
    bytes: &[u8],
    max_probe_bytes: u64,
) -> TestResult<Result<ReadConfidence, baozi_import::ImportFailure>> {
    let detection = DetectionOptions::new(max_probe_bytes)
        .expect("test probe size stays within the cancellation bound");
    let result = import_bytes_with_detection(source, bytes, ResourceLimits::default(), detection)?;
    Ok(match result {
        Ok(report) => {
            let selected = report
                .detection_evidence()
                .iter()
                .find(|evidence| {
                    evidence.format().as_str() == "org.baozi.ply"
                        && matches!(evidence.outcome(), DetectionOutcome::Match(_))
                })
                .expect("successful session retains matching PLY evidence");
            let DetectionOutcome::Match(confidence) = selected.outcome() else {
                unreachable!("filter guarantees matching evidence")
            };
            Ok(*confidence)
        }
        Err(error) => Err(operation_failure(error)?),
    })
}

#[test]
fn detects_ply_magic_through_the_session() -> TestResult<()> {
    let confidence = detect(
        "mesh.bin",
        b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        4_096,
    )?
    .expect("PLY magic is detected and parsed");

    assert_eq!(confidence, ReadConfidence::Certain);
    Ok(())
}

#[test]
fn detects_crlf_ply_magic_at_the_exact_boundary() -> TestResult<()> {
    let bytes = b"ply\r\nformat ascii 1.0\r\nelement vertex 1\r\nproperty float x\r\nproperty float y\r\nproperty float z\r\nend_header\r\n0 0 0\r\n";

    let failure = detect("mesh.bin", bytes, 4)?
        .expect_err("the CRLF magic is incomplete before the line feed");
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert!(matches!(failure.cause(), ImportCause::Detection(_)));
    assert!(matches!(
        failure.detection_evidence(),
        [evidence]
            if evidence.outcome()
                == &DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    ));

    let confidence = detect("mesh.bin", bytes, 5)?.expect("complete CRLF magic is sufficient");
    assert_eq!(confidence, ReadConfidence::Certain);
    Ok(())
}

#[test]
fn extension_is_maybe_without_magic() -> TestResult<()> {
    let failure = detect("mesh.ply", b"not-ply", 4_096)?
        .expect_err("extension selects PLY, then parsing rejects the invalid document");

    assert_eq!(
        failure
            .detection_evidence()
            .iter()
            .find(|evidence| evidence.format().as_str() == "org.baozi.ply")
            .map(|evidence| evidence.outcome()),
        Some(&DetectionOutcome::Match(ReadConfidence::Maybe))
    );
    assert!(failure.selected_format().is_some());
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidHeader);
    assert_eq!(ply_unsigned_detail(envelope, "byte_offset"), Some(0));
    assert_eq!(envelope.details().len(), 1);
    Ok(())
}

#[test]
fn truncated_magic_is_inconclusive_when_probe_budget_is_exhausted() -> TestResult<()> {
    let result = detect("mesh.bin", b"ply\nformat ascii 1.0\n", 3)?;
    let failure = result.expect_err("three bytes cannot establish the PLY magic");

    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert!(matches!(failure.cause(), ImportCause::Detection(_)));
    assert!(matches!(
        failure.detection_evidence(),
        [evidence]
            if evidence.outcome()
                == &DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    ));
    Ok(())
}

#[test]
fn impossible_truncated_magic_is_a_definitive_non_match() -> TestResult<()> {
    let result = detect("mesh.bin", b"plx and more input", 3)?;
    let failure = result.expect_err("mismatching magic is not selectable");

    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert!(matches!(failure.cause(), ImportCause::UnsupportedFormat));
    assert!(matches!(
        failure.detection_evidence(),
        [evidence] if evidence.outcome() == &DetectionOutcome::NoMatch
    ));
    Ok(())
}
