mod common;

use baozi_format_obj::ObjFailureCode;
use baozi_import::{
    FormatFailureDetailValue, FormatFailureEnvelope, ImportCause, ImportFailure, ResourceLimits,
};
use common::{TestResult, expected_failure, import_bytes, import_bytes_result, scene_snapshot};

fn format_envelope(failure: &ImportFailure) -> &FormatFailureEnvelope {
    match failure.cause() {
        ImportCause::Format(envelope) => envelope,
        cause => panic!("expected format failure, got {cause:?}"),
    }
}

fn string_detail<'failure>(
    envelope: &'failure FormatFailureEnvelope,
    key: &str,
) -> Option<&'failure str> {
    envelope
        .details()
        .iter()
        .find(|detail| detail.key() == key)
        .and_then(|detail| match detail.value() {
            FormatFailureDetailValue::String(value) => Some(value.as_str()),
            _ => None,
        })
}

fn unsigned_detail(envelope: &FormatFailureEnvelope, key: &str) -> Option<u64> {
    envelope
        .details()
        .iter()
        .find(|detail| detail.key() == key)
        .and_then(|detail| match detail.value() {
            FormatFailureDetailValue::Unsigned(value) => Some(value),
            _ => None,
        })
}

#[test]
fn empty_obj_is_parse_error() -> TestResult<()> {
    let result = import_bytes_result("empty.obj", b"", ResourceLimits::default())?;
    let error = expected_failure(result);

    assert_eq!(
        format_envelope(&error).code(),
        ObjFailureCode::EmptyGeometry.as_str()
    );
    assert!(error.diagnostics().is_empty());
    Ok(())
}

#[test]
fn non_utf8_obj_is_parse_error() -> TestResult<()> {
    let result = import_bytes_result(
        "bad-utf8.obj",
        &[0xff, 0xfe, b'\n'],
        ResourceLimits::default(),
    )?;
    let error = expected_failure(result);

    let envelope = format_envelope(&error);
    assert_eq!(envelope.code(), ObjFailureCode::InvalidUtf8.as_str());
    assert_eq!(unsigned_detail(envelope, "byte_offset"), Some(0));
    assert_eq!(string_detail(envelope, "subject"), Some("document"));
    assert_eq!(string_detail(envelope, "reason"), Some("invalid-utf8"));
    assert!(error.diagnostics().is_empty());
    Ok(())
}

#[test]
fn invalid_number_retains_location_details() -> TestResult<()> {
    let result = import_bytes_result("bad-number.obj", b"v nope 0 0\n", ResourceLimits::default())?;
    let error = expected_failure(result);
    let envelope = format_envelope(&error);
    assert_eq!(envelope.code(), ObjFailureCode::InvalidNumber.as_str());
    assert_eq!(unsigned_detail(envelope, "line"), Some(1));
    assert_eq!(unsigned_detail(envelope, "column"), Some(3));
    assert_eq!(string_detail(envelope, "subject"), Some("position"));
    assert_eq!(string_detail(envelope, "reason"), Some("invalid-float"));
    Ok(())
}

#[test]
fn face_with_too_few_vertices_is_parse_error() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nf 1 2\n";

    let result = import_bytes_result("few-face-vertices.obj", bytes, ResourceLimits::default())?;
    let error = expected_failure(result);

    assert_eq!(
        format_envelope(&error).code(),
        ObjFailureCode::InvalidFace.as_str()
    );
    Ok(())
}

#[test]
fn face_index_zero_is_parse_error() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n";

    let result = import_bytes_result("zero-index.obj", bytes, ResourceLimits::default())?;
    let error = expected_failure(result);

    assert_eq!(
        format_envelope(&error).code(),
        ObjFailureCode::InvalidIndex.as_str()
    );
    Ok(())
}

#[test]
fn face_index_out_of_range_is_parse_error() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n";

    let result = import_bytes_result("bad-index.obj", bytes, ResourceLimits::default())?;
    let error = expected_failure(result);

    assert_eq!(
        format_envelope(&error).code(),
        ObjFailureCode::InvalidIndex.as_str()
    );
    Ok(())
}

#[test]
fn unsupported_obj_statement_warns_and_geometry_imports() -> TestResult<()> {
    let bytes = b"curv 0 1 1 2\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let report = import_bytes("unsupported.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;
    let diagnostics = report.diagnostics();

    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code().as_str(), "obj.unsupported_statement");
    Ok(())
}

#[test]
fn invalid_smoothing_statements_retain_stable_location_details() -> TestResult<()> {
    struct Case {
        name: &'static str,
        statement: &'static str,
        column: u64,
        reason: &'static str,
    }

    for case in [
        Case {
            name: "missing",
            statement: "s",
            column: 1,
            reason: "missing-value",
        },
        Case {
            name: "negative",
            statement: "s -1",
            column: 3,
            reason: "negative",
        },
        Case {
            name: "bare-negative-sign",
            statement: "s -",
            column: 3,
            reason: "invalid-integer",
        },
        Case {
            name: "negative-text",
            statement: "s -group",
            column: 3,
            reason: "invalid-integer",
        },
        Case {
            name: "double-negative",
            statement: "s --1",
            column: 3,
            reason: "invalid-integer",
        },
        Case {
            name: "non-integer",
            statement: "s 1.5",
            column: 3,
            reason: "invalid-integer",
        },
        Case {
            name: "u32-overflow",
            statement: "s 4294967296",
            column: 3,
            reason: "out-of-range",
        },
        Case {
            name: "extra-argument",
            statement: "s 7 extra",
            column: 5,
            reason: "extra-value",
        },
    ] {
        let document = format!("v 0 0 0\nv 1 0 0\nv 0 1 0\n{}\nf 1 2 3\n", case.statement);
        let result = import_bytes_result(
            &format!("invalid-smoothing-{}.obj", case.name),
            document.as_bytes(),
            ResourceLimits::default(),
        )?;
        let error = expected_failure(result);
        let envelope = format_envelope(&error);

        assert_eq!(
            envelope.code(),
            ObjFailureCode::InvalidStatement.as_str(),
            "case {}",
            case.name
        );
        assert_eq!(
            unsigned_detail(envelope, "line"),
            Some(4),
            "case {}",
            case.name
        );
        assert_eq!(
            unsigned_detail(envelope, "column"),
            Some(case.column),
            "case {}",
            case.name
        );
        assert_eq!(
            string_detail(envelope, "subject"),
            Some("smoothing-group"),
            "case {}",
            case.name
        );
        assert_eq!(
            string_detail(envelope, "reason"),
            Some(case.reason),
            "case {}",
            case.name
        );
        assert!(error.diagnostics().is_empty(), "case {}", case.name);
    }
    Ok(())
}
