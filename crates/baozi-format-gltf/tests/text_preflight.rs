mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{FormatFailureDetailValue, ImportCause, ScopeAbortCause};
use common::{
    GltfTestOptions, TestResult, expected_error, gltf_failure, import_assets, import_assets_result,
};

fn unsigned_detail(envelope: &baozi_import::FormatFailureEnvelope, key: &str) -> Option<u64> {
    envelope.details().iter().find_map(|detail| {
        if detail.key() != key {
            return None;
        }
        match detail.value() {
            FormatFailureDetailValue::Unsigned(value) => Some(value),
            _ => None,
        }
    })
}

fn string_detail<'failure>(
    envelope: &'failure baozi_import::FormatFailureEnvelope,
    key: &str,
) -> Option<&'failure str> {
    envelope.details().iter().find_map(|detail| {
        if detail.key() != key {
            return None;
        }
        match detail.value() {
            FormatFailureDetailValue::String(value) => Some(value.as_str()),
            _ => None,
        }
    })
}

fn graph_json(name: &str) -> Vec<u8> {
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"name":"{name}"}}]}}"#
    )
    .into_bytes()
}

fn glb_with_json(json: &[u8]) -> Vec<u8> {
    let mut json = json.to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total_len = 20 + json.len();
    let mut bytes = Vec::with_capacity(total_len);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&(total_len as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"JSON");
    bytes.extend_from_slice(&json);
    bytes
}

fn glb_header(version: u32, declared_length: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(12);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&declared_length.to_le_bytes());
    bytes
}

#[test]
fn gltf_invalid_utf8_is_a_typed_format_failure_at_the_raw_offset() -> TestResult<()> {
    let mut document = br#"{"asset":{"version":"2.0"}}"#.to_vec();
    let invalid_offset = document.len() as u64;
    document.push(0xff);

    let (result, diagnostics) = import_assets_result(
        "invalid.gltf",
        [("invalid.gltf", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(envelope.code(), "gltf.invalid_utf8");
    assert_eq!(
        unsigned_detail(envelope, "byte_offset"),
        Some(invalid_offset)
    );
    assert_eq!(string_detail(envelope, "subject"), Some("json-document"));
    assert!(
        string_detail(envelope, "reason").is_some_and(|reason| reason.contains("invalid UTF-8"))
    );
    Ok(())
}

#[test]
fn gltf_bom_is_a_typed_format_failure_at_the_json_surface() -> TestResult<()> {
    let mut document = vec![0xef, 0xbb, 0xbf];
    document.extend_from_slice(br#"{"asset":{"version":"2.0"}}"#);

    let (result, diagnostics) = import_assets_result(
        "bom.gltf",
        [("bom.gltf", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(envelope.code(), "gltf.forbidden_bom");
    assert_eq!(unsigned_detail(envelope, "byte_offset"), Some(0));
    assert_eq!(string_detail(envelope, "subject"), Some("json-document"));
    Ok(())
}

#[test]
fn glb_invalid_utf8_reports_the_container_relative_raw_offset() -> TestResult<()> {
    let mut json = br#"{"asset":{"version":"2.0"}}"#.to_vec();
    let invalid_offset = 20 + json.len() as u64;
    json.push(0xff);
    let document = glb_with_json(&json);

    let (result, diagnostics) = import_assets_result(
        "invalid.glb",
        [("invalid.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(envelope.code(), "gltf.invalid_utf8");
    assert_eq!(
        unsigned_detail(envelope, "byte_offset"),
        Some(invalid_offset)
    );
    Ok(())
}

#[test]
fn glb_bom_reports_the_json_surface_container_offset() -> TestResult<()> {
    let mut json = vec![0xef, 0xbb, 0xbf];
    json.extend_from_slice(br#"{"asset":{"version":"2.0"}}"#);
    let document = glb_with_json(&json);

    let (result, diagnostics) = import_assets_result(
        "bom.glb",
        [("bom.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(envelope.code(), GltfFailureCode::ForbiddenBom.as_str());
    assert_eq!(unsigned_detail(envelope, "byte_offset"), Some(20));
    assert_eq!(string_detail(envelope, "subject"), Some("json-document"));
    Ok(())
}

#[test]
fn valid_unicode_and_embedded_bom_scalar_are_not_surface_bom() -> TestResult<()> {
    let document = graph_json("unicode-\u{4e2d}\u{6587}-embedded-\u{feff}-marker");

    let (_report, diagnostics) = import_assets(
        "unicode.gltf",
        [("unicode.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    Ok(())
}

#[test]
fn bare_cr_json_whitespace_is_accepted() -> TestResult<()> {
    let document = b"{\r\"asset\":{\"version\":\"2.0\"},\r\"scene\":0,\r\"scenes\":[{\"nodes\":[0]}],\r\"nodes\":[{\"name\":\"bare-cr\"}]\r}"
        .to_vec();

    let (_report, diagnostics) = import_assets(
        "bare-cr.gltf",
        [("bare-cr.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    Ok(())
}

#[test]
fn effective_decoded_utf8_limit_is_enforced_by_the_parse_scope() -> TestResult<()> {
    let document = graph_json("decoded-limit");
    let decoded_bytes = u64::try_from(document.len())?;
    let limit = decoded_bytes - 1;
    let mut options = GltfTestOptions::memory();
    options.limits.max_decoded_utf8_bytes = limit;

    let (result, diagnostics) = import_assets_result(
        "decoded-limit.gltf",
        [("decoded-limit.gltf", document)],
        options,
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected scope-owned decoded UTF-8 limit failure");
    };
    let ScopeAbortCause::ResourceLimit {
        resource,
        limit: actual_limit,
    } = abort.cause()
    else {
        panic!(
            "expected decoded UTF-8 resource limit, got {:?}",
            abort.cause()
        );
    };
    assert_eq!(*resource, "gltf-decoded-utf8-bytes");
    assert_eq!(*actual_limit, limit);
    Ok(())
}

#[test]
fn malformed_glb_containers_remain_backend_document_failures() -> TestResult<()> {
    let mut truncated_chunk_header = glb_header(2, 16);
    truncated_chunk_header.extend_from_slice(&0_u32.to_le_bytes());

    let mut oversized_json_chunk = glb_header(2, 20);
    oversized_json_chunk.extend_from_slice(&4_u32.to_le_bytes());
    oversized_json_chunk.extend_from_slice(b"JSON");

    let mut non_json_first_chunk = glb_header(2, 20);
    non_json_first_chunk.extend_from_slice(&0_u32.to_le_bytes());
    non_json_first_chunk.extend_from_slice(b"BIN\0");

    let malformed = [
        ("short-header.glb", b"glTF".to_vec()),
        ("unsupported-version.glb", glb_header(1, 12)),
        ("declared-length-mismatch.glb", glb_header(2, 16)),
        ("truncated-chunk-header.glb", truncated_chunk_header),
        ("oversized-json-chunk.glb", oversized_json_chunk),
        ("non-json-first-chunk.glb", non_json_first_chunk),
    ];

    for (source, document) in malformed {
        let (result, diagnostics) =
            import_assets_result(source, [(source, document)], GltfTestOptions::memory())?;
        let failure = expected_error(result)?;
        let envelope = gltf_failure(&failure)?;

        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics for {source}"
        );
        assert_eq!(
            envelope.code(),
            GltfFailureCode::InvalidDocument.as_str(),
            "unexpected failure code for {source}"
        );
        assert_eq!(
            unsigned_detail(envelope, "byte_offset"),
            None,
            "malformed container must not manufacture a JSON offset for {source}"
        );
    }
    Ok(())
}

#[test]
fn glb_filename_does_not_bypass_whole_json_utf8_preflight() -> TestResult<()> {
    let mut document = br#"{"asset":{"version":"2.0"}}"#.to_vec();
    let invalid_offset = document.len() as u64;
    document.push(0xff);

    let (result, diagnostics) = import_assets_result(
        "json-named-glb.glb",
        [("json-named-glb.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(envelope.code(), "gltf.invalid_utf8");
    assert_eq!(
        unsigned_detail(envelope, "byte_offset"),
        Some(invalid_offset)
    );
    Ok(())
}

#[test]
fn glb_filename_does_not_bypass_whole_json_bom_preflight() -> TestResult<()> {
    let mut document = vec![0xef, 0xbb, 0xbf];
    document.extend_from_slice(br#"{"asset":{"version":"2.0"}}"#);

    let (result, diagnostics) = import_assets_result(
        "bom-named-glb.glb",
        [("bom-named-glb.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(envelope.code(), "gltf.forbidden_bom");
    assert_eq!(unsigned_detail(envelope, "byte_offset"), Some(0));
    Ok(())
}
