mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    FormatFailureDetailValue, ImportCause, ImportDimension, LedgerCause, ScopeAbortCause,
};
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, expected_error, glb_with_json, gltf_failure,
    import_assets, import_assets_result,
};

const JSON_NESTING_HARD_LIMIT: u64 = 256;

fn minimal_gltf() -> Vec<u8> {
    br#"{"asset":{"version":"2.0"}}"#.to_vec()
}

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

fn dimension_attempted(
    ledger: &baozi_import::ImportLedgerSnapshot,
    dimension: ImportDimension,
) -> u64 {
    ledger
        .dimension(dimension)
        .expect("tested glTF dimension must be configured")
        .attempted()
}

fn dimension_consumed(
    ledger: &baozi_import::ImportLedgerSnapshot,
    dimension: ImportDimension,
) -> u64 {
    ledger
        .dimension(dimension)
        .expect("tested glTF dimension must be configured")
        .consumed()
}

fn assert_format_range(
    failure: &baozi_import::ImportFailure,
    subject: &str,
    offset: usize,
    length: usize,
) -> TestResult<()> {
    let envelope = gltf_failure(failure)?;
    assert_eq!(envelope.code(), GltfFailureCode::InvalidDocument.as_str());
    assert_eq!(string_detail(envelope, "subject"), Some(subject));
    assert_eq!(
        unsigned_detail(envelope, "byte_offset"),
        Some(offset as u64)
    );
    assert_eq!(
        unsigned_detail(envelope, "byte_length"),
        Some(length as u64)
    );
    Ok(())
}

fn assert_scope_resource(
    failure: &baozi_import::ImportFailure,
    expected_resource: &'static str,
    expected_limit: u64,
) {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!(
            "expected scope-owned resource failure, got {:?}",
            failure.cause()
        );
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::ResourceLimit { resource, limit }
            if *resource == expected_resource && *limit == expected_limit
    ));
}

fn assert_ledger_limit(
    failure: &baozi_import::ImportFailure,
    dimension: ImportDimension,
    limit: u64,
) {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!(
            "expected scope-owned ledger failure, got {:?}",
            failure.cause()
        );
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
            dimension: actual,
            limit: actual_limit,
            ..
        }) if *actual == dimension && *actual_limit == limit
    ));
}

#[test]
fn unknown_optional_extension_payload_is_explicitly_rejected() -> TestResult<()> {
    let document = br#"{
  "asset": { "version": "2.0" },
  "extensionsUsed": ["VENDOR_physics"],
  "extensions": { "VENDOR_physics": { "gravity": 9.8 } }
}"#
    .to_vec();
    let (result, diagnostics) = import_assets_result(
        "optional-extension.gltf",
        [("optional-extension.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code().as_str(), "gltf.extension_ignored");
    assert!(diagnostics[0].message().contains("VENDOR_physics"));
    let error = expected_error(result)?;
    common::assert_gltf_failure(
        &error,
        GltfFailureCode::UnsupportedFeature,
        Some("extension payload"),
    )?;
    Ok(())
}

#[test]
fn unsupported_required_extension_fails_before_scene_projection() -> TestResult<()> {
    let document = br#"{
  "asset": { "version": "2.0" },
  "extensionsUsed": ["VENDOR_physics"],
  "extensionsRequired": ["VENDOR_physics"]
}"#
    .to_vec();
    let (result, diagnostics) = import_assets_result(
        "required-extension.gltf",
        [("required-extension.gltf", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(
        envelope.code(),
        GltfFailureCode::UnsupportedFeature.as_str()
    );
    assert_eq!(
        string_detail(envelope, "subject"),
        Some("required-extension")
    );
    assert!(string_detail(envelope, "reason").is_some_and(|reason| {
        reason.contains("VENDOR_physics") && reason.contains("not supported")
    }));
    assert!(failure.source_scope_record().is_none());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn required_extensions_must_be_used() -> TestResult<()> {
    let document = br#"{
  "asset": { "version": "2.0" },
  "extensionsRequired": ["VENDOR_physics"]
}"#
    .to_vec();
    let (result, diagnostics) = import_assets_result(
        "missing-used-extension.gltf",
        [("missing-used-extension.gltf", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(envelope.code(), GltfFailureCode::InvalidDocument.as_str());
    assert!(
        string_detail(envelope, "reason")
            .is_some_and(|reason| reason.contains("missing from extensionsUsed"))
    );
    assert!(failure.source_scope_record().is_none());
    Ok(())
}

#[test]
fn extension_declaration_lists_reject_duplicates_and_empty_names() -> TestResult<()> {
    let documents = [
        (
            "duplicate-used-extension.gltf",
            br#"{
  "asset": { "version": "2.0" },
  "extensionsUsed": ["VENDOR_physics", "VENDOR_physics"]
}"#
            .to_vec(),
            "extensionsUsed contains duplicate",
        ),
        (
            "duplicate-required-extension.gltf",
            br#"{
  "asset": { "version": "2.0" },
  "extensionsUsed": ["VENDOR_physics"],
  "extensionsRequired": ["VENDOR_physics", "VENDOR_physics"]
}"#
            .to_vec(),
            "extensionsRequired contains duplicate",
        ),
        (
            "empty-extension.gltf",
            br#"{
  "asset": { "version": "2.0" },
  "extensionsUsed": [""]
}"#
            .to_vec(),
            "has an empty extension name",
        ),
    ];

    for (source, document, expected_reason) in documents {
        let (result, diagnostics) =
            import_assets_result(source, [(source, document)], GltfTestOptions::memory())?;
        let failure = expected_error(result)?;
        let envelope = gltf_failure(&failure)?;
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics for {source}"
        );
        assert_eq!(envelope.code(), GltfFailureCode::InvalidDocument.as_str());
        assert!(
            string_detail(envelope, "reason")
                .is_some_and(|reason| reason.contains(expected_reason))
        );
        assert!(failure.source_scope_record().is_none());
    }
    Ok(())
}

#[test]
fn extension_declaration_sort_obeys_the_work_budget() -> TestResult<()> {
    let document = br#"{
  "asset": { "version": "2.0" },
  "extensionsUsed": [
    "VENDOR_h", "VENDOR_g", "VENDOR_f", "VENDOR_e",
    "VENDOR_d", "VENDOR_c", "VENDOR_b", "VENDOR_a", "VENDOR_h"
  ]
}"#
    .to_vec();
    let (calibration_result, calibration_diagnostics) = import_assets_result(
        "extension-work-calibration.gltf",
        [("extension-work-calibration.gltf", document.clone())],
        GltfTestOptions::memory(),
    )?;
    let calibration_failure = expected_error(calibration_result)?;
    assert!(calibration_diagnostics.is_empty());
    assert_eq!(
        gltf_failure(&calibration_failure)?.code(),
        GltfFailureCode::InvalidDocument.as_str()
    );
    let attempted = dimension_attempted(calibration_failure.ledger(), ImportDimension::WorkUnits);
    let limit = attempted
        .checked_sub(1)
        .expect("extension validation must attempt bounded work");

    let mut options = GltfTestOptions::memory();
    options.limits.max_work_units = limit;
    let (result, diagnostics) = import_assets_result(
        "extension-work-limit.gltf",
        [("extension-work-limit.gltf", document)],
        options,
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_ledger_limit(&failure, ImportDimension::WorkUnits, limit);
    assert_eq!(
        dimension_attempted(failure.ledger(), ImportDimension::WorkUnits),
        attempted
    );
    assert!(failure.source_scope_record().is_none());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn backend_accepted_duplicate_keys_fail_before_backend_allocation() -> TestResult<()> {
    let document = br#"{"asset":{"version":"2.0"},"vendor":{"a":1,"a":2}}"#.to_vec();
    assert!(
        gltf::Gltf::from_slice(&document).is_ok(),
        "the differential fixture must be accepted by the current backend"
    );
    let duplicate = br#""a""#;
    let duplicate_offset = document
        .windows(duplicate.len())
        .rposition(|window| window == duplicate)
        .expect("fixture must contain the later duplicate key");
    let mut options = GltfTestOptions::memory();
    options.limits.max_live_bytes = 32 * 1024;

    let (result, diagnostics) = import_assets_result(
        "duplicate-key.gltf",
        [("duplicate-key.gltf", document.clone())],
        options,
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_format_range(
        &failure,
        "json-object-key",
        duplicate_offset,
        duplicate.len(),
    )?;
    assert!(
        string_detail(gltf_failure(&failure)?, "reason")
            .is_some_and(|reason| reason.contains("duplicate"))
    );
    assert_eq!(
        dimension_consumed(failure.ledger(), ImportDimension::DecodeWork),
        document.len() as u64
    );
    assert!(
        dimension_attempted(failure.ledger(), ImportDimension::Tokens) > 0,
        "the complete syntax map must publish its attempted token count"
    );
    assert_eq!(
        dimension_consumed(failure.ledger(), ImportDimension::Tokens),
        0,
        "a duplicate-key map must not publish accepted tokens"
    );
    assert!(
        dimension_attempted(failure.ledger(), ImportDimension::ScratchBytes) < 32 * 1024,
        "the backend allowance must not be attempted"
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(
        dimension_attempted(failure.ledger(), ImportDimension::DecodedTextBytes),
        0
    );
    assert_eq!(
        dimension_attempted(failure.ledger(), ImportDimension::LineIndexBytes),
        0
    );
    Ok(())
}

#[test]
fn escaped_and_literal_object_keys_share_one_duplicate_identity_in_glb() -> TestResult<()> {
    let json = br#"{"asset":{"version":"2.0"},"vendor":{"a":1,"\u0061":2}}"#;
    assert!(
        gltf::Gltf::from_slice(json).is_ok(),
        "the differential fixture must be accepted by the current backend"
    );
    let duplicate = br#""\u0061""#;
    let relative_offset = json
        .windows(duplicate.len())
        .position(|window| window == duplicate)
        .expect("fixture must contain the escaped duplicate key");
    let document = glb_with_json(json);

    let (result, diagnostics) = import_assets_result(
        "escaped-duplicate.glb",
        [("escaped-duplicate.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_format_range(
        &failure,
        "json-object-key",
        20 + relative_offset,
        duplicate.len(),
    )?;
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn strict_json_failures_retain_the_offending_raw_range() -> TestResult<()> {
    let mut control_character = br#"{"asset":{"version":"2.0"},"x":"ab"#.to_vec();
    let control_offset = control_character.len();
    control_character.push(b'\n');
    control_character.extend_from_slice(br#"cd"}"#);

    let cases = [
        (
            "trailing-comma.gltf",
            br#"{"asset":{"version":"2.0",}}"#.to_vec(),
            "json-object",
            br#",}"#.as_slice(),
            1,
            1,
            "object key",
        ),
        (
            "leading-zero.gltf",
            br#"{"asset":{"version":"2.0"},"scene":01}"#.to_vec(),
            "json-number",
            br#"1}"#.as_slice(),
            0,
            1,
            "leading zero",
        ),
        (
            "invalid-escape.gltf",
            br#"{"asset":{"version":"\x"}}"#.to_vec(),
            "json-string",
            br#"\x"#.as_slice(),
            1,
            1,
            "escape",
        ),
        (
            "lone-surrogate.gltf",
            br#"{"asset":{"version":"\uD800"}}"#.to_vec(),
            "json-string",
            br#"\uD800"#.as_slice(),
            0,
            6,
            "surrogate",
        ),
        (
            "trailing-garbage.gltf",
            br#"{"asset":{"version":"2.0"}}x"#.to_vec(),
            "json-document",
            br#"x"#.as_slice(),
            0,
            1,
            "trailing",
        ),
    ];

    for (source, document, subject, marker, marker_delta, length, reason_fragment) in cases {
        let marker_offset = document
            .windows(marker.len())
            .position(|window| window == marker)
            .expect("fixture marker must exist");
        let offset = marker_offset + marker_delta;
        let (result, diagnostics) =
            import_assets_result(source, [(source, document)], GltfTestOptions::memory())?;
        let failure = expected_error(result)?;
        let envelope = gltf_failure(&failure)?;

        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics for {source}"
        );
        assert_format_range(&failure, subject, offset, length)?;
        assert!(
            string_detail(envelope, "reason")
                .is_some_and(|reason| reason.contains(reason_fragment)),
            "unexpected reason for {source}: {:?}",
            string_detail(envelope, "reason")
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }

    let (result, diagnostics) = import_assets_result(
        "control-character.gltf",
        [("control-character.gltf", control_character)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_format_range(&failure, "json-string", control_offset, 1)?;
    Ok(())
}

#[test]
fn valid_json_with_unsupported_extras_reaches_the_explicit_profile_check() -> TestResult<()> {
    let document = br#"{
        "asset": {
            "version": "2.0",
            "extras": {
                "escaped": "quote:\" slash:\/ line:\n scalar:\u4e2d pair:\uD834\uDD1E",
                "values": [0, -0, 12, -1.25, 6.02e23, -2E-3, true, false, null]
            }
        }
    }"#
    .to_vec();
    assert!(gltf::Gltf::from_slice(&document).is_ok());

    let (result, diagnostics) = import_assets_result(
        "grammar-family.gltf",
        [("grammar-family.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    let error = expected_error(result)?;
    common::assert_gltf_failure(&error, GltfFailureCode::UnsupportedFeature, Some("extras"))?;
    Ok(())
}

#[test]
fn glb_non_whitespace_padding_is_rejected_at_the_container_offset() -> TestResult<()> {
    let json = minimal_gltf();
    let mut document = glb_with_json(&json);
    assert_eq!(
        document.last(),
        Some(&b' '),
        "fixture must have JSON padding"
    );
    let padding_offset = document.len() - 1;
    document[padding_offset] = 0;

    let (result, diagnostics) = import_assets_result(
        "invalid-padding.glb",
        [("invalid-padding.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_format_range(&failure, "json-padding", padding_offset, 1)?;
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn glb_authored_json_whitespace_before_space_padding_remains_valid() -> TestResult<()> {
    let mut json = minimal_gltf();
    json.extend_from_slice(b"\r\n");
    let document = glb_with_json(&json);
    assert_eq!(document.len() % 4, 0);
    assert_eq!(document.last(), Some(&b' '));

    let (report, diagnostics) = import_assets(
        "authored-whitespace.glb",
        [("authored-whitespace.glb", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn glb_bin_bytes_do_not_expand_the_backend_allowance() -> TestResult<()> {
    let mut json = minimal_gltf();
    json.push(b' ');
    assert!(json.len().is_multiple_of(4));

    let (json_report, json_diagnostics) = import_assets(
        "plain.gltf",
        [("plain.gltf", json.clone())],
        GltfTestOptions::memory(),
    )?;
    assert!(json_diagnostics.is_empty());

    let bin = vec![0x5a; 256 * 1024];
    let document = append_glb_chunk(glb_with_json(&json), b"BIN\0", &bin);
    let primary_growth = u64::try_from(document.len() - json.len())?;
    let (glb_report, glb_diagnostics) = import_assets(
        "binary.glb",
        [("binary.glb", document)],
        GltfTestOptions::memory(),
    )?;
    assert!(glb_diagnostics.is_empty());

    let json_scratch = json_report
        .ledger()
        .dimension(ImportDimension::ScratchBytes)
        .expect("scratch bytes are configured");
    let glb_scratch = glb_report
        .ledger()
        .dimension(ImportDimension::ScratchBytes)
        .expect("scratch bytes are configured");
    assert_eq!(glb_scratch.attempted(), json_scratch.attempted());
    assert_eq!(glb_scratch.consumed(), json_scratch.consumed());

    let json_primary = json_report
        .ledger()
        .dimension(ImportDimension::PrimaryBytes)
        .expect("primary bytes are configured");
    let glb_primary = glb_report
        .ledger()
        .dimension(ImportDimension::PrimaryBytes)
        .expect("primary bytes are configured");
    assert_eq!(
        glb_primary.consumed() - json_primary.consumed(),
        primary_growth
    );
    assert_eq!(glb_report.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn json_token_and_container_limits_fail_before_backend_decode() -> TestResult<()> {
    let minimal = minimal_gltf();

    let mut token_options = GltfTestOptions::memory();
    token_options.limits.max_tokens = 8;
    let (result, diagnostics) = import_assets_result(
        "token-limit.gltf",
        [("token-limit.gltf", minimal.clone())],
        token_options,
    )?;
    let token_failure = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_ledger_limit(&token_failure, ImportDimension::Tokens, 8);
    assert_eq!(
        dimension_attempted(token_failure.ledger(), ImportDimension::Tokens),
        9
    );
    assert_eq!(
        dimension_consumed(token_failure.ledger(), ImportDimension::Tokens),
        0
    );
    assert!(
        dimension_consumed(token_failure.ledger(), ImportDimension::DecodeWork)
            < minimal.len() as u64
    );
    assert!(
        dimension_attempted(token_failure.ledger(), ImportDimension::ScratchBytes) < 64 * 1024,
        "the token limit must fail before the backend allowance"
    );
    assert_eq!(token_failure.ledger().current_live_bytes(), 0);

    let mut container_options = GltfTestOptions::memory();
    container_options.limits.max_records = 1;
    let (result, diagnostics) = import_assets_result(
        "object-limit.gltf",
        [("object-limit.gltf", minimal.clone())],
        container_options,
    )?;
    let object_failure = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_ledger_limit(&object_failure, ImportDimension::Records, 1);
    assert_eq!(
        dimension_attempted(object_failure.ledger(), ImportDimension::Records),
        2
    );
    assert_eq!(
        dimension_consumed(object_failure.ledger(), ImportDimension::Records),
        0
    );
    assert!(
        dimension_consumed(object_failure.ledger(), ImportDimension::DecodeWork)
            < minimal.len() as u64
    );
    assert_eq!(object_failure.ledger().current_live_bytes(), 0);

    let mut array_options = GltfTestOptions::memory();
    array_options.limits.max_records = 1;
    let (result, diagnostics) = import_assets_result(
        "array-limit.gltf",
        [("array-limit.gltf", b"[[]]".to_vec())],
        array_options,
    )?;
    let array_failure = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_ledger_limit(&array_failure, ImportDimension::Records, 1);
    assert_eq!(
        dimension_attempted(array_failure.ledger(), ImportDimension::Records),
        2
    );
    assert_eq!(array_failure.ledger().current_live_bytes(), 0);

    let mut exact_options = GltfTestOptions::memory();
    exact_options.limits.max_tokens = 9;
    exact_options.limits.max_records = 2;
    let (report, diagnostics) = import_assets(
        "exact-syntax-limits.gltf",
        [("exact-syntax-limits.gltf", minimal)],
        exact_options,
    )?;
    assert!(diagnostics.is_empty());
    assert_eq!(
        dimension_consumed(report.ledger(), ImportDimension::Tokens),
        9
    );
    assert_eq!(
        dimension_consumed(report.ledger(), ImportDimension::Records),
        2
    );
    assert_eq!(
        dimension_attempted(report.ledger(), ImportDimension::DecodedTextBytes),
        0
    );
    assert_eq!(
        dimension_attempted(report.ledger(), ImportDimension::LineIndexBytes),
        0
    );
    Ok(())
}

#[test]
fn json_string_number_and_nesting_limits_are_bounded_before_backend_decode() -> TestResult<()> {
    for (source, document, resource) in [
        (
            "string-token-limit.gltf",
            br#""123456789""#.to_vec(),
            "gltf-json-string-bytes",
        ),
        (
            "number-token-limit.gltf",
            b"123456789".to_vec(),
            "gltf-json-number-bytes",
        ),
        (
            "literal-token-limit.gltf",
            b"false".to_vec(),
            "gltf-json-literal-bytes",
        ),
    ] {
        let mut options = GltfTestOptions::memory();
        let limit = if source == "literal-token-limit.gltf" {
            4
        } else {
            8
        };
        options.limits.max_token_bytes = limit;
        let (result, diagnostics) = import_assets_result(source, [(source, document)], options)?;
        let failure = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert_scope_resource(&failure, resource, limit as u64);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }

    let depth = usize::try_from(JSON_NESTING_HARD_LIMIT)? + 1;
    let mut nested = Vec::with_capacity(depth * 2 + 1);
    nested.extend(std::iter::repeat_n(b'[', depth));
    nested.push(b'0');
    nested.extend(std::iter::repeat_n(b']', depth));
    let (result, diagnostics) = import_assets_result(
        "nesting-limit.gltf",
        [("nesting-limit.gltf", nested)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_scope_resource(&failure, "gltf-json-nesting-depth", JSON_NESTING_HARD_LIMIT);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn syntax_valid_backend_rejection_remains_fail_closed() -> TestResult<()> {
    let document = br#"{"asset":{"version":"2.0"},"scene":"not-an-index"}"#.to_vec();
    let backend_allowance = (document.len() as u64)
        .checked_mul(65)
        .and_then(|bytes| bytes.checked_add(64 * 1024))
        .expect("test backend allowance must fit u64");
    let calibration = vec![b' '; document.len()];
    let (calibration_result, calibration_diagnostics) = import_assets_result(
        "backend-baseline.gltf",
        [("backend-baseline.gltf", calibration)],
        GltfTestOptions::memory(),
    )?;
    let calibration_failure = expected_error(calibration_result)?;
    assert!(calibration_diagnostics.is_empty());
    assert_eq!(
        gltf_failure(&calibration_failure)?.code(),
        GltfFailureCode::InvalidDocument.as_str()
    );
    let baseline_peak = calibration_failure.ledger().peak_live_bytes();
    let live_limit = baseline_peak
        .checked_add(backend_allowance)
        .expect("test live-memory limit must fit u64");
    let mut options = GltfTestOptions::memory();
    options.limits.max_live_bytes = live_limit;

    let (result, diagnostics) = import_assets_result(
        "backend-rejection.gltf",
        [("backend-rejection.gltf", document)],
        options,
    )?;
    let failure = expected_error(result)?;
    let envelope = gltf_failure(&failure)?;

    assert!(diagnostics.is_empty());
    assert_eq!(envelope.code(), GltfFailureCode::InvalidDocument.as_str());
    assert!(
        string_detail(envelope, "reason").is_some_and(|reason| reason.contains("invalid glTF"))
    );
    assert!(
        failure.ledger().peak_live_bytes() >= backend_allowance,
        "syntax scratch must be released before the opaque backend allowance; peak {}, reason {:?}, decode {}",
        failure.ledger().peak_live_bytes(),
        string_detail(envelope, "reason"),
        dimension_attempted(failure.ledger(), ImportDimension::DecodeWork)
    );
    assert!(failure.ledger().peak_live_bytes() <= live_limit);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}
