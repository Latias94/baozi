mod common;

use baozi_format_ply::PlyFailureCode;
use baozi_import::{FormatFailureEvidence, ImportDimension, ResourceLimits};
use common::{
    TestResult, assert_requested_terminal_failure, assert_scope_resource_limit, binary_triangle,
    expected_error, expected_ply_failure, expected_report, import_bytes_forced_result,
    ply_string_detail, ply_unsigned_detail, scene_snapshot, snapshot_line,
};
use std::error::Error;

const SCAN_CHUNK_BYTES: usize = 4 * 1024;
const MAX_NUMERIC_LITERAL_BYTES: usize = SCAN_CHUNK_BYTES;
type HeaderSurfaceFixture = (&'static str, &'static [u8], &'static [u8], &'static [u8]);

fn assert_header_profile_failure(
    source: &str,
    bytes: &[u8],
    expected_offset: u64,
    expected_context: &str,
) -> TestResult<()> {
    let failure = expected_error(import_bytes_forced_result(
        source,
        bytes,
        ResourceLimits::default(),
    )?)?;
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidHeader);
    assert_eq!(
        ply_string_detail(envelope, "context"),
        Some(expected_context)
    );
    assert_eq!(
        ply_unsigned_detail(envelope, "byte_offset"),
        Some(expected_offset)
    );
    assert_eq!(failure.source_identity().as_str(), source);
    Ok(())
}

#[test]
fn bare_cr_is_rejected_at_its_first_raw_offset() -> TestResult<()> {
    let bytes = b"ply\rformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n";
    assert_header_profile_failure("bare-cr.ply", bytes, 3, "header-newline")
}

#[test]
fn forbidden_header_controls_are_rejected_at_their_first_raw_offset() -> TestResult<()> {
    for (label, control) in [
        ("nul-byte", 0x00),
        ("vertical-tab", 0x0b),
        ("form-feed", 0x0c),
        ("del-byte", 0x7f),
    ] {
        let mut bytes = b"ply\nformat ascii 1.0\ncomment before".to_vec();
        let offending_offset = bytes.len() as u64;
        bytes.push(control);
        bytes.extend_from_slice(
            b"after\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        );
        let source = format!("{label}.ply");
        assert_header_profile_failure(&source, &bytes, offending_offset, "header-control")?;
    }
    Ok(())
}

#[test]
fn non_ascii_ascii_body_grammar_is_rejected_at_first_raw_byte() -> TestResult<()> {
    for (label, offending) in [
        ("unicode-whitespace", "\u{00a0}".as_bytes()),
        ("utf8-bom", [0xef, 0xbb, 0xbf].as_slice()),
        ("invalid-utf8", [0xff].as_slice()),
    ] {
        let mut bytes = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0".to_vec();
        let offending_offset = bytes.len() as u64;
        bytes.extend_from_slice(offending);
        bytes.extend_from_slice(b"0 0\n");
        let source = format!("body-{label}.ply");

        let failure = expected_error(import_bytes_forced_result(
            &source,
            &bytes,
            ResourceLimits::default(),
        )?)?;
        let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidRecord);
        assert_eq!(ply_string_detail(envelope, "context"), Some("ascii-body"));
        assert_eq!(
            ply_unsigned_detail(envelope, "byte_offset"),
            Some(offending_offset)
        );
        assert_eq!(failure.source_identity().as_str(), source);
    }
    Ok(())
}

#[test]
fn trailing_invalid_utf8_preserves_inert_evidence_offset_and_failed_record_accounting()
-> TestResult<()> {
    const HEADER: &[u8] = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n";

    let mut bytes = HEADER.to_vec();
    let line_start = bytes.len();
    bytes.extend_from_slice(b"0 0 ");
    bytes.resize(line_start + SCAN_CHUNK_BYTES * 2 + 17, b'0');
    let invalid_offset = bytes.len();
    bytes.push(u8::MAX);
    bytes.push(b'\n');

    let failure = expected_error(import_bytes_forced_result(
        "trailing-invalid-utf8.ply",
        &bytes,
        ResourceLimits::default(),
    )?)?;
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidRecord);

    assert_eq!(ply_string_detail(envelope, "context"), Some("ascii-body"));
    assert_eq!(
        ply_unsigned_detail(envelope, "byte_offset"),
        Some(invalid_offset as u64)
    );
    assert_eq!(
        ply_string_detail(envelope, "reason"),
        Some("PLY input violates its ASCII text profile")
    );
    let evidence = Error::source(envelope)
        .and_then(|source| source.downcast_ref::<FormatFailureEvidence>())
        .expect("PLY envelope must expose only Baozi-owned format evidence");
    assert!(Error::source(evidence).is_none());
    assert_eq!(
        failure.source_identity().as_str(),
        "trailing-invalid-utf8.ply"
    );
    let records = failure
        .ledger()
        .dimension(ImportDimension::Records)
        .expect("record accounting is configured");
    assert_eq!(records.attempted(), 1);
    assert_eq!(records.consumed(), 0);
    assert_requested_terminal_failure(&failure);
    Ok(())
}

#[test]
fn forbidden_bom_is_terminal_at_raw_offset_zero() -> TestResult<()> {
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend(std::iter::repeat_n(b'x', SCAN_CHUNK_BYTES * 3));

    let failure = expected_error(import_bytes_forced_result(
        "bom.ply",
        &bytes,
        ResourceLimits::default(),
    )?)?;
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidHeader);

    assert_eq!(ply_string_detail(envelope, "context"), Some("header-bom"));
    assert_eq!(ply_unsigned_detail(envelope, "byte_offset"), Some(0));
    assert_eq!(failure.source_identity().as_str(), "bom.ply");
    let decode = failure
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .expect("forbidden BOM classification must publish bounded decode work");
    assert_eq!((decode.attempted(), decode.consumed()), (3, 3));
    Ok(())
}

#[test]
fn unicode_whitespace_in_header_is_rejected_at_first_raw_byte() -> TestResult<()> {
    let mut bytes = b"ply\nformat".to_vec();
    let offending_offset = bytes.len() as u64;
    bytes.extend_from_slice("\u{00a0}".as_bytes());
    bytes.extend_from_slice(
        b"ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
    );

    let failure = expected_error(import_bytes_forced_result(
        "unicode-whitespace.ply",
        &bytes,
        ResourceLimits::default(),
    )?)?;
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidHeader);

    assert_eq!(
        ply_unsigned_detail(envelope, "byte_offset"),
        Some(offending_offset)
    );
    assert_eq!(failure.source_identity().as_str(), "unicode-whitespace.ply");
    Ok(())
}

#[test]
fn every_header_surface_rejects_non_ascii_and_invalid_utf8_at_the_first_raw_byte() -> TestResult<()>
{
    let fixtures: &[HeaderSurfaceFixture] = &[
        (
            "magic",
            b"pl",
            b"\xc3\xa9",
            b"y\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "format-keyword",
            b"ply\nfor",
            b"\xc3\xa9",
            b"mat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "format-value",
            b"ply\nformat asc",
            b"\xc3\xa9",
            b"ii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "element-keyword",
            b"ply\nformat ascii 1.0\nele",
            b"\xc3\xa9",
            b"ment vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "element-name",
            b"ply\nformat ascii 1.0\nelement vert",
            b"\xc3\xa9",
            b"x 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "property-keyword",
            b"ply\nformat ascii 1.0\nelement vertex 1\npro",
            b"\xc3\xa9",
            b"perty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "property-type",
            b"ply\nformat ascii 1.0\nelement vertex 1\nproperty flo",
            b"\xc3\xa9",
            b"at x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "property-name",
            b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float te",
            b"\xc3\xa9",
            b"mperature\nproperty float z\nend_header\n0 0 1 0\n",
        ),
        (
            "comment",
            b"ply\nformat ascii 1.0\ncomment legacy ",
            b"\xc3\xa9",
            b" text\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "obj-info",
            b"ply\nformat ascii 1.0\nobj_info legacy ",
            b"\xc3\xa9",
            b" text\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
        (
            "end-header",
            b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_hea",
            b"\xc3\xa9",
            b"der\n0 0 0\n",
        ),
        (
            "invalid-utf8-comment",
            b"ply\nformat ascii 1.0\ncomment legacy ",
            b"\xff",
            b" text\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
        ),
    ];

    for (label, prefix, offending, suffix) in fixtures {
        let mut bytes = Vec::from(*prefix);
        let offending_offset = bytes.len() as u64;
        bytes.extend_from_slice(offending);
        bytes.extend_from_slice(suffix);
        let source = format!("surface-{label}.ply");
        assert_header_profile_failure(&source, &bytes, offending_offset, "header-ascii")?;
    }
    Ok(())
}

#[test]
fn lf_crlf_and_horizontal_tab_remain_valid_ascii_syntax() -> TestResult<()> {
    let lf = "ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n1 2 3\n";
    let crlf = lf.replace('\n', "\r\n");
    let tabbed = b"ply\nformat\tascii\t1.0\nelement\tvertex\t1\nproperty\tfloat\tx\nproperty\tfloat\ty\nproperty\tfloat\tz\nend_header\n1\t2\t3\n";

    for (source, bytes) in [
        ("lf.ply", lf.as_bytes()),
        ("crlf.ply", crlf.as_bytes()),
        ("tabs.ply", tabbed.as_slice()),
    ] {
        let report = expected_report(import_bytes_forced_result(
            source,
            bytes,
            ResourceLimits::default(),
        )?)?;
        let snapshot = scene_snapshot(&report)?;
        assert!(
            snapshot_line(&snapshot, "attribute[0]")
                .contains("values(present,total=3)=[0x3f800000,0x40000000,0x40400000]")
        );
    }
    Ok(())
}

#[test]
fn binary_body_bytes_are_not_scanned_as_ascii_text() -> TestResult<()> {
    let bytes = binary_triangle(true);
    let marker = b"end_header\n";
    let body_start = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|offset| offset + marker.len())
        .expect("binary fixture has an end_header line");
    assert!(bytes[body_start..].iter().any(|byte| !byte.is_ascii()));

    let report = expected_report(import_bytes_forced_result(
        "binary-body.ply",
        &bytes,
        ResourceLimits::default(),
    )?)?;
    let snapshot = scene_snapshot(&report)?;
    assert!(snapshot_line(&snapshot, "attribute[0]").contains("elements=3"));
    Ok(())
}

#[test]
fn binary_payload_boundaries_preserve_first_bytes_and_consumed_text_evidence() -> TestResult<()> {
    const LF_HEADER: &[u8] = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty uchar x\nproperty uchar y\nproperty uchar z\nend_header\n";
    const CRLF_HEADER: &[u8] = b"ply\r\nformat binary_little_endian 1.0\r\nelement vertex 1\r\nproperty uchar x\r\nproperty uchar y\r\nproperty uchar z\r\nend_header\r\n";

    for (newline, header) in [("lf", LF_HEADER), ("crlf", CRLF_HEADER)] {
        let mut expected_text_evidence = None;
        for marker in [b'\n', b'\r', b' ', 0x00, 0xff] {
            let mut bytes = header.to_vec();
            bytes.extend_from_slice(&[marker, 0, 0]);
            if marker == 0xff {
                bytes.extend(std::iter::repeat_n(0xff, SCAN_CHUNK_BYTES * 2));
                bytes.extend_from_slice(b"\r\0\xef\xbb\xbf");
            }
            let source = format!("binary-{newline}-first-{marker:02x}.ply");
            let report = expected_report(import_bytes_forced_result(
                &source,
                &bytes,
                ResourceLimits::default(),
            )?)?;
            let snapshot = scene_snapshot(&report)?;
            let expected_position = format!(
                "values(present,total=3)=[0x{:08x},0x00000000,0x00000000]",
                f32::from(marker).to_bits()
            );
            assert!(snapshot_line(&snapshot, "attribute[0]").contains(&expected_position));

            let decode = report
                .ledger()
                .dimension(ImportDimension::DecodeWork)
                .expect("binary header must publish decode work");
            let decoded = report
                .ledger()
                .dimension(ImportDimension::DecodedTextBytes)
                .expect("binary header must publish decoded storage");
            let line_index = report
                .ledger()
                .dimension(ImportDimension::LineIndexBytes)
                .expect("binary header must publish line-index storage");
            assert!(decode.attempted() >= decode.consumed());
            let consumed_text_evidence = (
                decode.consumed(),
                decoded.peak_live_bytes(),
                line_index.peak_live_bytes(),
            );
            match expected_text_evidence {
                Some(expected) => assert_eq!(consumed_text_evidence, expected),
                None => expected_text_evidence = Some(consumed_text_evidence),
            }
        }
    }
    Ok(())
}

#[test]
fn declared_ascii_records_leave_trailing_invalid_bytes_unobserved() -> TestResult<()> {
    let mut bytes = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n1 2 3\n".to_vec();
    bytes.extend_from_slice(b"\xff\0\r\xef\xbb\xbf");

    let report = expected_report(import_bytes_forced_result(
        "ascii-trailing-bytes.ply",
        &bytes,
        ResourceLimits::default(),
    )?)?;
    let snapshot = scene_snapshot(&report)?;
    assert!(snapshot_line(&snapshot, "attribute[0]").contains("elements=1"));
    Ok(())
}

#[test]
fn earlier_ascii_record_error_precedes_later_text_violation() -> TestResult<()> {
    const HEADER: &[u8] = b"ply\nformat ascii 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nend_header\n";
    let mut bytes = HEADER.to_vec();
    bytes.extend_from_slice(b"not-a-number 0 0\n\xff 0 0\n");

    let failure = expected_error(import_bytes_forced_result(
        "ascii-error-priority.ply",
        &bytes,
        ResourceLimits::default(),
    )?)?;
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidScalar);
    assert_eq!(ply_string_detail(envelope, "context"), Some("float"));
    assert_eq!(
        ply_unsigned_detail(envelope, "byte_offset"),
        Some(HEADER.len() as u64)
    );
    Ok(())
}

#[test]
fn lf_and_crlf_line_limits_count_content_only() -> TestResult<()> {
    const LINE_BYTES: usize = 32;
    for newline in ["\n", "\r\n"] {
        let header = [
            "ply",
            "format ascii 1.0",
            "element vertex 1",
            "property int x",
            "property int y",
            "property int z",
            "end_header",
            "",
        ]
        .join(newline);
        let mut limits = ResourceLimits::default();
        limits.max_line_bytes = LINE_BYTES;

        let exact = format!(
            "{header}{} 0 0{}",
            "0".repeat(LINE_BYTES - " 0 0".len()),
            newline
        );
        expected_report(import_bytes_forced_result(
            "line-content-exact.ply",
            exact.as_bytes(),
            limits.clone(),
        )?)?;

        let excess = format!(
            "{header}{} 0 0{}",
            "0".repeat(LINE_BYTES + 1 - " 0 0".len()),
            newline
        );
        let failure = expected_error(import_bytes_forced_result(
            "line-content-excess.ply",
            excess.as_bytes(),
            limits,
        )?)?;
        assert_scope_resource_limit(&failure, "max_line_bytes", LINE_BYTES as u64);
    }
    Ok(())
}

#[test]
fn ascii_decoded_utf8_budget_counts_the_entire_input() -> TestResult<()> {
    let bytes = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n1 2 3\n";
    let decoded_bytes =
        u64::try_from(bytes.len()).expect("ASCII fixture length must fit the resource counter");

    let mut exact_limits = ResourceLimits::default();
    exact_limits.max_decoded_utf8_bytes = decoded_bytes;
    let report = expected_report(import_bytes_forced_result(
        "ascii-decoded-exact.ply",
        bytes,
        exact_limits,
    )?)?;
    let snapshot = scene_snapshot(&report)?;
    assert!(snapshot_line(&snapshot, "attribute[0]").contains("elements=1"));

    let below_limit = decoded_bytes
        .checked_sub(1)
        .expect("ASCII fixture must contain at least one byte");
    let mut below_limits = ResourceLimits::default();
    below_limits.max_decoded_utf8_bytes = below_limit;
    let failure = expected_error(import_bytes_forced_result(
        "ascii-decoded-limit.ply",
        bytes,
        below_limits,
    )?)?;
    assert_scope_resource_limit(&failure, "ply-decoded-utf8-bytes", below_limit);
    Ok(())
}

#[test]
fn binary_decoded_utf8_budget_counts_only_the_header() -> TestResult<()> {
    let bytes = binary_triangle(true);
    let marker = b"end_header\n";
    let data_start = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|offset| offset + marker.len())
        .expect("binary fixture has an end_header line");
    assert!(
        data_start < bytes.len(),
        "binary fixture must contain a body"
    );
    let decoded_bytes =
        u64::try_from(data_start).expect("binary header length must fit the resource counter");

    let mut exact_limits = ResourceLimits::default();
    exact_limits.max_decoded_utf8_bytes = decoded_bytes;
    let report = expected_report(import_bytes_forced_result(
        "binary-decoded-exact.ply",
        &bytes,
        exact_limits,
    )?)?;
    let snapshot = scene_snapshot(&report)?;
    assert!(snapshot_line(&snapshot, "attribute[0]").contains("elements=3"));

    let below_limit = decoded_bytes
        .checked_sub(1)
        .expect("binary header must contain at least one byte");
    let mut below_limits = ResourceLimits::default();
    below_limits.max_decoded_utf8_bytes = below_limit;
    let failure = expected_error(import_bytes_forced_result(
        "binary-decoded-limit.ply",
        &bytes,
        below_limits,
    )?)?;
    assert_scope_resource_limit(&failure, "ply-decoded-utf8-bytes", below_limit);
    Ok(())
}

#[test]
fn ascii_numeric_parsing_uses_dot_syntax_and_rejects_decimal_commas() -> TestResult<()> {
    let accepted = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n1.5 -2.5 1e1\n";
    let report = expected_report(import_bytes_forced_result(
        "dot-numbers.ply",
        accepted,
        ResourceLimits::default(),
    )?)?;
    let snapshot = scene_snapshot(&report)?;
    assert!(
        snapshot_line(&snapshot, "attribute[0]")
            .contains("values(present,total=3)=[0x3fc00000,0xc0200000,0x41200000]")
    );

    let comma = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n1,5 0 0\n";
    let failure = expected_error(import_bytes_forced_result(
        "comma-number.ply",
        comma,
        ResourceLimits::default(),
    )?)?;
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidScalar);
    assert_eq!(ply_string_detail(envelope, "context"), Some("float"));
    Ok(())
}

#[test]
fn ascii_numeric_literals_enforce_the_4096_byte_format_boundary() -> TestResult<()> {
    const HEADER: &str = "ply\nformat ascii 1.0\nelement vertex 1\nproperty int x\nproperty int y\nproperty int z\nend_header\n";

    let at_limit = "0".repeat(MAX_NUMERIC_LITERAL_BYTES);
    let accepted = format!("{HEADER}{at_limit} 0 0\n");
    let report = expected_report(import_bytes_forced_result(
        "numeric-literal-4096.ply",
        accepted.as_bytes(),
        ResourceLimits::default(),
    )?)?;
    let snapshot = scene_snapshot(&report)?;
    assert!(
        snapshot_line(&snapshot, "attribute[0]")
            .contains("values(present,total=3)=[0x00000000,0x00000000,0x00000000]")
    );

    let over_limit = "0".repeat(MAX_NUMERIC_LITERAL_BYTES + 1);
    let rejected = format!("{HEADER}{over_limit} 0 0\n");
    let failure = expected_error(import_bytes_forced_result(
        "numeric-literal-4097.ply",
        rejected.as_bytes(),
        ResourceLimits::default(),
    )?)?;
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidScalar);

    assert_eq!(
        ply_unsigned_detail(envelope, "byte_offset"),
        Some(HEADER.len() as u64)
    );
    assert_eq!(
        failure.source_identity().as_str(),
        "numeric-literal-4097.ply"
    );
    let records = failure
        .ledger()
        .dimension(ImportDimension::Records)
        .expect("record accounting is configured");
    assert_eq!(records.attempted(), 1);
    assert_eq!(records.consumed(), 0);
    assert_requested_terminal_failure(&failure);
    Ok(())
}

#[test]
fn header_element_counts_enforce_the_4096_byte_format_boundary() -> TestResult<()> {
    const PREFIX: &str = "ply\nformat ascii 1.0\n";
    const SUFFIX: &str =
        "\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n";

    let at_limit = format!("{}1", "0".repeat(MAX_NUMERIC_LITERAL_BYTES - 1));
    let accepted = format!("{PREFIX}element vertex {at_limit}{SUFFIX}");
    let report = expected_report(import_bytes_forced_result(
        "element-count-4096.ply",
        accepted.as_bytes(),
        ResourceLimits::default(),
    )?)?;
    let snapshot = scene_snapshot(&report)?;
    assert!(
        snapshot_line(&snapshot, "attribute[0]")
            .contains("values(present,total=3)=[0x00000000,0x00000000,0x00000000]")
    );

    let over_limit = format!("{}1", "0".repeat(MAX_NUMERIC_LITERAL_BYTES));
    let rejected = format!("{PREFIX}element vertex {over_limit}{SUFFIX}");
    let failure = expected_error(import_bytes_forced_result(
        "element-count-4097.ply",
        rejected.as_bytes(),
        ResourceLimits::default(),
    )?)?;
    let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidSchema);

    assert_eq!(
        ply_unsigned_detail(envelope, "byte_offset"),
        Some(PREFIX.len() as u64)
    );
    assert_eq!(failure.source_identity().as_str(), "element-count-4097.ply");
    let records = failure
        .ledger()
        .dimension(ImportDimension::Records)
        .expect("record accounting is configured");
    assert_eq!(records.attempted(), 0);
    assert_eq!(records.consumed(), 0);
    assert_requested_terminal_failure(&failure);
    Ok(())
}
