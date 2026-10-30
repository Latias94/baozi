mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportOperationControl, LedgerCause,
    ScopeAbortCause,
};
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, assert_gltf_failure, controlled_import,
    expected_error, glb_with_json, import_assets_result_with_control, sidecar_options,
    triangle_gltf_with_buffer_uri,
};
use std::sync::atomic::{AtomicUsize, Ordering};

const SCAN_CHUNK_BYTES: usize = 4 * 1024;
const ENCODED_CHUNKS: usize = 9;
const ENCODED_BYTES: usize = SCAN_CHUNK_BYTES * ENCODED_CHUNKS;
const DECODED_BYTES: usize = ENCODED_BYTES / 4 * 3;
const CANCELLATION_FIXTURE_CHUNKS: usize = 64;

fn cancel_on_next_poll(calibration_polls: &AtomicUsize) -> usize {
    let completed_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(completed_polls > 0);
    completed_polls
        .checked_add(1)
        .expect("relative cancellation poll must fit usize")
}

fn assert_cancelled(failure: &ImportFailure) {
    assert!(
        matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Control(ControlCause::Cancelled {
                        origin: CancellationOrigin::Signal,
                    })
                )
        ),
        "expected signal cancellation, got {:?}",
        failure.cause()
    );
}

fn cancelled_failure(
    source: &'static str,
    document: &[u8],
    cancel_on: usize,
) -> TestResult<(ImportFailure, usize)> {
    let (control, polls) = controlled_import(Some(cancel_on));
    let (result, diagnostics) = import_assets_result_with_control(
        source,
        [(source, document.to_vec())],
        GltfTestOptions::memory(),
        control,
    )?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert_cancelled(&failure);
    let observed_polls = polls.load(Ordering::SeqCst);
    assert_eq!(observed_polls, cancel_on);
    Ok((failure, observed_polls))
}

fn first_cancelled_prefix_at_least(
    source: &'static str,
    document: &[u8],
    total_polls: usize,
    dimension: ImportDimension,
    target_consumed: u64,
) -> TestResult<(usize, ImportFailure)> {
    let mut low = 1usize;
    let mut high = total_polls;
    while low < high {
        let middle = low + (high - low) / 2;
        let (failure, _) = cancelled_failure(source, document, middle)?;
        if dimension_consumed(&failure, dimension) >= target_consumed {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let (failure, _) = cancelled_failure(source, document, low)?;
    assert!(dimension_consumed(&failure, dimension) >= target_consumed);
    Ok((low, failure))
}

fn assert_ledger_limit(
    failure: &ImportFailure,
    expected_dimension: ImportDimension,
    expected_limit: u64,
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
            dimension,
            limit,
            ..
        }) if *dimension == expected_dimension && *limit == expected_limit
    ));
}

fn assert_scope_limit(
    failure: &ImportFailure,
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

fn dimension_attempted(failure: &ImportFailure, dimension: ImportDimension) -> u64 {
    failure
        .ledger()
        .dimension(dimension)
        .expect("tested glTF dimension is configured")
        .attempted()
}

fn dimension_consumed(failure: &ImportFailure, dimension: ImportDimension) -> u64 {
    failure
        .ledger()
        .dimension(dimension)
        .expect("tested glTF dimension is configured")
        .consumed()
}

fn encoded_payload_with_chunks(chunks: usize) -> String {
    let encoded_bytes = SCAN_CHUNK_BYTES
        .checked_mul(chunks)
        .expect("encoded fixture size must fit usize");
    "A".repeat(encoded_bytes)
}

fn encoded_payload() -> String {
    encoded_payload_with_chunks(ENCODED_CHUNKS)
}

fn json_document_with_scan_chunks(chunks: usize) -> Vec<u8> {
    let document_bytes = SCAN_CHUNK_BYTES
        .checked_mul(chunks)
        .expect("JSON cancellation fixture size must fit usize");
    let prefix = br#"{"asset":{"version":"2.0"}"#;
    let suffix = br#"}"#;
    assert!(document_bytes > prefix.len() + suffix.len());

    let mut document = Vec::with_capacity(document_bytes);
    document.extend_from_slice(prefix);
    document.resize(document_bytes - suffix.len(), b' ');
    document.extend_from_slice(suffix);
    assert_eq!(document.len(), document_bytes);
    document
}

fn data_uri(encoded: &str) -> String {
    format!("data:application/octet-stream;base64,{encoded}")
}

#[test]
fn whole_json_utf8_preflight_cancellation_is_bounded_by_scan_chunks() -> TestResult<()> {
    let document = json_document_with_scan_chunks(CANCELLATION_FIXTURE_CHUNKS);
    let document_bytes = document.len();
    let (calibration_control, calibration_polls) = controlled_import(None);
    let (calibration_result, calibration_diagnostics) = import_assets_result_with_control(
        "utf8-preflight-calibration.gltf",
        [("utf8-preflight-calibration.gltf", document.clone())],
        GltfTestOptions::memory(),
        calibration_control,
    )?;
    assert!(calibration_diagnostics.is_empty());
    let _report = calibration_result?;
    let total_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);
    let (cancel_on, failure) = first_cancelled_prefix_at_least(
        "utf8-preflight-cancel.gltf",
        &document,
        total_polls,
        ImportDimension::DecodeWork,
        SCAN_CHUNK_BYTES as u64,
    )?;
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DecodeWork),
        document_bytes as u64
    );
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DecodeWork),
        SCAN_CHUNK_BYTES as u64
    );

    let mut malformed = document;
    malformed[SCAN_CHUNK_BYTES] = u8::MAX;
    let (malformed_failure, _) = cancelled_failure(
        "utf8-preflight-malformed-cancel.gltf",
        &malformed,
        cancel_on,
    )?;
    assert_eq!(
        dimension_consumed(&malformed_failure, ImportDimension::DecodeWork),
        SCAN_CHUNK_BYTES as u64
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn glb_json_surface_cancellation_is_bounded_by_scan_chunks() -> TestResult<()> {
    let json = json_document_with_scan_chunks(CANCELLATION_FIXTURE_CHUNKS);
    let json_bytes = u64::try_from(json.len())?;
    let document = glb_with_json(&json);
    let (calibration_control, calibration_polls) = controlled_import(None);
    let (calibration_result, calibration_diagnostics) = import_assets_result_with_control(
        "glb-utf8-preflight-calibration.glb",
        [("glb-utf8-preflight-calibration.glb", document.clone())],
        GltfTestOptions::memory(),
        calibration_control,
    )?;
    assert!(calibration_diagnostics.is_empty());
    let _report = calibration_result?;

    let total_polls = calibration_polls.load(Ordering::SeqCst);
    let (_cancel_on, failure) = first_cancelled_prefix_at_least(
        "glb-utf8-preflight-cancel.glb",
        &document,
        total_polls,
        ImportDimension::DecodeWork,
        SCAN_CHUNK_BYTES as u64,
    )?;

    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DecodeWork),
        json_bytes
    );
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DecodeWork),
        SCAN_CHUNK_BYTES as u64
    );
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DecodedTextBytes),
        0
    );
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::LineIndexBytes),
        0
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn glb_container_chunk_scan_respects_the_work_budget() -> TestResult<()> {
    let json = br#"{"asset":{"version":"2.0"}}"#;
    let mut document = append_glb_chunk(glb_with_json(json), b"BIN\0", &[0, 0, 0, 0]);
    // Keep enough valid extension headers that the small scan allowance cannot finish the
    // container even when terminal failure ownership adds a few work units to calibration.
    for _ in 0..32 {
        document = append_glb_chunk(document, b"PAD0", &[0, 0, 0, 0]);
    }
    // An equally sized invalid first chunk enters the same controlled work group, including
    // its owned handles, then rejects before completing the first admitted header.
    let mut invalid_first_chunk = document.clone();
    invalid_first_chunk[16..20].copy_from_slice(b"BAD!");
    let (baseline, baseline_diagnostics) = import_assets_result_with_control(
        "chunk-work-limit.glb",
        [("chunk-work-limit.glb", invalid_first_chunk)],
        GltfTestOptions::memory(),
        ImportOperationControl::default(),
    )?;
    assert!(baseline_diagnostics.is_empty());
    let baseline = expected_error(baseline)?;
    assert_gltf_failure(&baseline, GltfFailureCode::InvalidDocument, None)?;
    let prefix = dimension_consumed(&baseline, ImportDimension::WorkUnits);
    assert_eq!(
        dimension_attempted(&baseline, ImportDimension::WorkUnits),
        prefix + 1
    );
    let mut options = GltfTestOptions::memory();
    options.limits.max_work_units = prefix + 3;

    let (result, diagnostics) = import_assets_result_with_control(
        "chunk-work-limit.glb",
        [("chunk-work-limit.glb", document)],
        options,
        ImportOperationControl::default(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_ledger_limit(&failure, ImportDimension::WorkUnits, prefix + 3);
    // Terminal failure ownership also performs handle work. The absent DecodeWork admission
    // proves the exhausted budget stopped container scanning before JSON processing began.
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DecodeWork),
        0
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn data_uri_base64_scan_cancellation_is_bounded_by_scan_chunks() -> TestResult<()> {
    let valid_encoded = encoded_payload_with_chunks(CANCELLATION_FIXTURE_CHUNKS);
    let decoded_bytes = base64::decoded_len_estimate(valid_encoded.len());
    let valid_uri = data_uri(&valid_encoded);
    let valid_document = triangle_gltf_with_buffer_uri(&valid_uri, decoded_bytes);
    let document_decode_work = (valid_document.len() * 2) as u64;

    let mut early_encoded = valid_encoded.as_bytes().to_vec();
    early_encoded[0] = b' ';
    let early_encoded =
        String::from_utf8(early_encoded).expect("base64 scan fixture must remain ASCII");
    let early_uri = data_uri(&early_encoded);
    let early_document = triangle_gltf_with_buffer_uri(
        &early_uri,
        base64::decoded_len_estimate(early_encoded.len()),
    );
    let (early_control, early_polls) = controlled_import(None);
    let (early_result, early_diagnostics) = import_assets_result_with_control(
        "base64-scan-early.gltf",
        [("base64-scan-early.gltf", early_document)],
        GltfTestOptions::memory(),
        early_control,
    )?;
    let early_failure = expected_error(early_result)?;
    assert!(early_diagnostics.is_empty());
    assert_gltf_failure(
        &early_failure,
        GltfFailureCode::InvalidDocument,
        Some("contains whitespace"),
    )?;

    let mut late_encoded = valid_encoded.as_bytes().to_vec();
    late_encoded[SCAN_CHUNK_BYTES * (CANCELLATION_FIXTURE_CHUNKS - 1)] = b' ';
    let late_encoded =
        String::from_utf8(late_encoded).expect("base64 scan fixture must remain ASCII");
    let late_document = triangle_gltf_with_buffer_uri(&data_uri(&late_encoded), decoded_bytes);

    let (late_control, late_polls) = controlled_import(None);
    let (late_result, late_diagnostics) = import_assets_result_with_control(
        "base64-scan-late.gltf",
        [("base64-scan-late.gltf", late_document)],
        GltfTestOptions::memory(),
        late_control,
    )?;
    let late_failure = expected_error(late_result)?;
    assert!(late_diagnostics.is_empty());
    assert_gltf_failure(
        &late_failure,
        GltfFailureCode::InvalidDocument,
        Some("contains whitespace"),
    )?;
    assert_eq!(
        late_polls.load(Ordering::SeqCst) - early_polls.load(Ordering::SeqCst),
        CANCELLATION_FIXTURE_CHUNKS - 1,
        "base64 scanning must poll once per 4 KiB chunk"
    );

    let string_limit = u64::try_from(valid_uri.len() - 1)
        .expect("data URI string limit must fit the resource counter");
    let mut calibration_options = GltfTestOptions::memory();
    calibration_options.limits.max_string_bytes = valid_uri.len() - 1;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let (calibration_result, calibration_diagnostics) = import_assets_result_with_control(
        "base64-scan-calibration.gltf",
        [("base64-scan-calibration.gltf", valid_document.clone())],
        calibration_options,
        calibration_control,
    )?;
    let calibration_failure = expected_error(calibration_result)?;

    assert!(calibration_diagnostics.is_empty());
    assert_scope_limit(&calibration_failure, "max_string_bytes", string_limit);
    assert_eq!(
        dimension_attempted(&calibration_failure, ImportDimension::DataUriBytes),
        0
    );
    assert_eq!(
        dimension_consumed(&calibration_failure, ImportDimension::DataUriBytes),
        0
    );
    assert_eq!(
        dimension_consumed(&calibration_failure, ImportDimension::DecodeWork),
        document_decode_work
    );
    let cancel_on = cancel_on_next_poll(&calibration_polls);
    let (control, polls) = controlled_import(Some(cancel_on));
    let (result, diagnostics) = import_assets_result_with_control(
        "base64-scan-cancel.gltf",
        [("base64-scan-cancel.gltf", valid_document)],
        GltfTestOptions::memory(),
        control,
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_cancelled(&failure);
    assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DataUriBytes),
        0
    );
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DataUriBytes),
        0
    );
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DecodeWork),
        document_decode_work
    );

    let mut boundary_encoded = valid_encoded.into_bytes();
    boundary_encoded[0] = b' ';
    let boundary_encoded =
        String::from_utf8(boundary_encoded).expect("base64 scan fixture must remain ASCII");
    let boundary_document =
        triangle_gltf_with_buffer_uri(&data_uri(&boundary_encoded), decoded_bytes);
    let (boundary_failure, _) = cancelled_failure(
        "base64-scan-malformed-cancel.gltf",
        &boundary_document,
        cancel_on,
    )?;
    assert_eq!(
        dimension_attempted(&boundary_failure, ImportDimension::DataUriBytes),
        0
    );
    assert_eq!(
        dimension_consumed(&boundary_failure, ImportDimension::DecodeWork),
        document_decode_work
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn data_uri_base64_decode_cancellation_is_bounded_by_decode_chunks() -> TestResult<()> {
    let encoded = encoded_payload_with_chunks(CANCELLATION_FIXTURE_CHUNKS);
    let decoded_bytes = base64::decoded_len_estimate(encoded.len());
    let uri = data_uri(&encoded);
    let document = triangle_gltf_with_buffer_uri(&uri, decoded_bytes);
    let document_decode_work = (document.len() * 2) as u64;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let (calibration_result, calibration_diagnostics) = import_assets_result_with_control(
        "base64-decode-calibration.gltf",
        [("base64-decode-calibration.gltf", document.clone())],
        GltfTestOptions::memory(),
        calibration_control,
    )?;
    assert!(calibration_diagnostics.is_empty());
    let _report = calibration_result?;
    let total_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);
    let decoded_chunk_bytes = base64::decoded_len_estimate(SCAN_CHUNK_BYTES) as u64;
    let (cancel_on, failure) = first_cancelled_prefix_at_least(
        "base64-decode-cancel.gltf",
        &document,
        total_polls,
        ImportDimension::DataUriBytes,
        decoded_chunk_bytes,
    )?;
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DataUriBytes),
        decoded_bytes as u64
    );
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DecodeWork),
        document_decode_work + encoded.len() as u64
    );
    let consumed_decoded = dimension_consumed(&failure, ImportDimension::DataUriBytes);
    assert_eq!(consumed_decoded, decoded_chunk_bytes);
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DecodeWork),
        document_decode_work + SCAN_CHUNK_BYTES as u64
    );

    let mut malformed_encoded = encoded.into_bytes();
    malformed_encoded[SCAN_CHUNK_BYTES] = b'@';
    let malformed_encoded =
        String::from_utf8(malformed_encoded).expect("base64 decode fixture must remain ASCII");
    let malformed_document =
        triangle_gltf_with_buffer_uri(&data_uri(&malformed_encoded), decoded_bytes);
    let (malformed_failure, _) = cancelled_failure(
        "base64-decode-malformed-cancel.gltf",
        &malformed_document,
        cancel_on,
    )?;
    assert_eq!(
        dimension_consumed(&malformed_failure, ImportDimension::DataUriBytes),
        decoded_chunk_bytes
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn data_uri_work_admission_precedes_decode_buffer_allocation() -> TestResult<()> {
    let encoded = encoded_payload();
    let uri = data_uri(&encoded);
    let document = triangle_gltf_with_buffer_uri(&uri, DECODED_BYTES);
    let decode_limit = (document.len() * 2) as u64;

    let mut calibration_options = GltfTestOptions::memory();
    calibration_options.limits.max_decode_work = decode_limit;
    let (calibration_result, calibration_diagnostics) = import_assets_result_with_control(
        "data-uri-admission.gltf",
        [("data-uri-admission.gltf", document.clone())],
        calibration_options,
        ImportOperationControl::default(),
    )?;
    let calibration_failure = expected_error(calibration_result)?;
    assert!(calibration_diagnostics.is_empty());
    assert_ledger_limit(
        &calibration_failure,
        ImportDimension::DecodeWork,
        decode_limit,
    );
    let pre_buffer_peak = calibration_failure.ledger().peak_live_bytes();
    assert!(pre_buffer_peak > 0);

    let mut options = GltfTestOptions::memory();
    options.limits.max_decode_work = decode_limit;
    options.limits.max_live_bytes = pre_buffer_peak;
    let (result, diagnostics) = import_assets_result_with_control(
        "data-uri-admission.gltf",
        [("data-uri-admission.gltf", document.clone())],
        options,
        ImportOperationControl::default(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_ledger_limit(&failure, ImportDimension::DecodeWork, decode_limit);
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DataUriBytes),
        DECODED_BYTES as u64
    );
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DataUriBytes),
        0
    );
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DecodeWork),
        (document.len() * 2 + encoded.len()) as u64
    );
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DecodeWork),
        (document.len() * 2) as u64
    );
    assert_eq!(failure.ledger().peak_live_bytes(), pre_buffer_peak);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn external_buffer_snapshot_size_does_not_add_copy_decode_work() -> TestResult<()> {
    const SMALL_BUFFER_BYTES: usize = 128 * 1024;
    const LARGE_BUFFER_BYTES: usize = 256 * 1024;

    let small_document = triangle_gltf_with_buffer_uri("payload.bin", SMALL_BUFFER_BYTES);
    let large_document = triangle_gltf_with_buffer_uri("payload.bin", LARGE_BUFFER_BYTES);
    assert_eq!(small_document.len(), large_document.len());

    let (small_result, small_diagnostics) = import_assets_result_with_control(
        "external-borrowed-small.gltf",
        [
            ("external-borrowed-small.gltf", small_document),
            ("payload.bin", vec![0_u8; SMALL_BUFFER_BYTES]),
        ],
        sidecar_options(),
        ImportOperationControl::default(),
    )?;
    let (large_result, large_diagnostics) = import_assets_result_with_control(
        "external-borrowed-large.gltf",
        [
            ("external-borrowed-large.gltf", large_document),
            ("payload.bin", vec![0_u8; LARGE_BUFFER_BYTES]),
        ],
        sidecar_options(),
        ImportOperationControl::default(),
    )?;
    let small_report = small_result?;
    let large_report = large_result?;

    assert!(small_diagnostics.is_empty());
    assert!(large_diagnostics.is_empty());
    let small_decode = small_report
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .expect("decode work is configured");
    let large_decode = large_report
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .expect("decode work is configured");
    assert_eq!(small_decode.attempted(), large_decode.attempted());
    assert_eq!(small_decode.consumed(), large_decode.consumed());
    let small_dependency = small_report
        .ledger()
        .dimension(ImportDimension::DependencyBytes)
        .expect("dependency bytes are configured");
    let large_dependency = large_report
        .ledger()
        .dimension(ImportDimension::DependencyBytes)
        .expect("dependency bytes are configured");
    assert_eq!(small_dependency.consumed(), SMALL_BUFFER_BYTES as u64);
    assert_eq!(large_dependency.consumed(), LARGE_BUFFER_BYTES as u64);
    assert_eq!(
        small_report
            .successful_input_manifest()
            .node_classes()
            .iter()
            .find(|class| class.identity().value().as_str() == "payload.bin")
            .expect("external buffer must enter the input manifest")
            .byte_length(),
        SMALL_BUFFER_BYTES as u64
    );
    assert_eq!(
        large_report
            .successful_input_manifest()
            .node_classes()
            .iter()
            .find(|class| class.identity().value().as_str() == "payload.bin")
            .expect("external buffer must enter the input manifest")
            .byte_length(),
        LARGE_BUFFER_BYTES as u64
    );
    Ok(())
}
