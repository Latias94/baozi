mod common;

use baozi_format_stl::StlFailureCode;
use baozi_import::{DetectionOutcome, ImportCause, ReadConfidence, ResourceLimits};
use common::{
    ascii_triangle, expected_stl_failure, import_bytes_forced_with_limits, stl_string_detail,
    stl_unsigned_detail,
};

#[test]
fn forbidden_utf8_bom_is_terminal_at_raw_offset_zero() {
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(ascii_triangle("bom").as_bytes());

    assert_profile_failure(
        &bytes,
        "stl.non_ascii",
        0,
        "forbidden-bom",
        ReadConfidence::Maybe,
    );
}

#[test]
fn valid_utf8_non_ascii_fails_at_its_first_raw_byte() {
    let bytes = ascii_triangle("\u{e9}");

    assert_profile_failure(
        bytes.as_bytes(),
        "stl.non_ascii",
        6,
        "non-ascii-byte",
        ReadConfidence::Maybe,
    );
}

#[test]
fn invalid_utf8_fails_at_its_first_raw_byte() {
    let mut bytes = ascii_triangle("bad").into_bytes();
    bytes[6] = 0xff;

    assert_profile_failure(
        &bytes,
        StlFailureCode::InvalidUtf8.as_str(),
        6,
        "invalid-utf8",
        ReadConfidence::Maybe,
    );
}

#[test]
fn invalid_utf8_before_ascii_magic_is_terminal_at_raw_offset_zero() {
    let mut bytes = vec![0xff];
    bytes.extend_from_slice(ascii_triangle("bad").as_bytes());

    assert_profile_failure(
        &bytes,
        StlFailureCode::InvalidUtf8.as_str(),
        0,
        "invalid-utf8",
        ReadConfidence::Maybe,
    );
}

#[test]
fn bare_carriage_return_is_terminal_at_its_raw_offset() {
    let bytes = ascii_triangle("bare\rcr");

    assert_profile_failure(
        bytes.as_bytes(),
        "stl.invalid_control",
        10,
        "bare-cr",
        ReadConfidence::Maybe,
    );
}

#[test]
fn nul_is_terminal_at_its_raw_offset() {
    let bytes = ascii_triangle("\0nul");

    assert_profile_failure(
        bytes.as_bytes(),
        "stl.invalid_control",
        6,
        "nul-byte",
        ReadConfidence::Maybe,
    );
}

#[test]
fn forbidden_ascii_control_is_terminal_at_its_raw_offset() {
    let bytes = ascii_triangle("\u{1}control");

    assert_profile_failure(
        bytes.as_bytes(),
        "stl.invalid_control",
        6,
        "forbidden-control",
        ReadConfidence::Maybe,
    );
}

#[test]
fn crlf_and_horizontal_tab_remain_valid_ascii_stl() {
    let bytes = ascii_triangle("tabs")
        .replace("  facet", "\tfacet")
        .replace('\n', "\r\n");

    let report = import_bytes_forced_with_limits(
        "strict-profile.stl",
        bytes.as_bytes(),
        ResourceLimits::default(),
    )
    .expect("the declared CRLF and horizontal-tab forms must remain valid");
    assert_eq!(report.scene().meshes().len(), 1);
}

fn assert_profile_failure(
    bytes: &[u8],
    code: &str,
    offset: u64,
    reason: &str,
    detector_confidence: ReadConfidence,
) {
    let failure =
        import_bytes_forced_with_limits("strict-profile.stl", bytes, ResourceLimits::default())
            .expect_err("strict ASCII STL profile violation must be terminal");

    let evidence = failure
        .detection_evidence()
        .iter()
        .find(|evidence| evidence.format().as_str() == "org.baozi.stl")
        .expect("failure must retain STL detector evidence");
    assert_eq!(
        evidence.outcome(),
        &DetectionOutcome::Match(detector_confidence),
        "profile-invalid text must not be advertised as a likely conforming ASCII STL"
    );

    assert!(matches!(failure.cause(), ImportCause::Format(_)));
    let envelope = expected_stl_failure(&failure);
    assert_eq!(envelope.code(), code);
    assert_eq!(stl_unsigned_detail(envelope, "byte_offset"), Some(offset));
    assert_eq!(
        stl_string_detail(envelope, "subject"),
        Some("ascii-document")
    );
    assert_eq!(stl_string_detail(envelope, "reason"), Some(reason));
}
