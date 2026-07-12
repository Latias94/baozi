mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportProgressPhase,
    ImportProgressUnit, LedgerCause, ScopeAbortCause,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, ProgressDecision, ProgressEvent,
};
use base64::Engine as _;
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, expected_error,
    import_assets_result_with_control, triangle_gltf_with_buffer_uri,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const SCAN_CHUNK_BYTES: usize = 4 * 1024;
const ENCODED_CHUNKS: usize = 9;
const ENCODED_BYTES: usize = SCAN_CHUNK_BYTES * ENCODED_CHUNKS;
const DECODED_BYTES: usize = ENCODED_BYTES / 4 * 3;

#[derive(Clone)]
struct ArmedPollCancellation {
    armed: Arc<AtomicBool>,
    polls: Arc<AtomicUsize>,
    cancel_on: Option<usize>,
}

impl CancellationSignal for ArmedPollCancellation {
    fn is_cancelled(&self) -> bool {
        if !self.armed.load(Ordering::SeqCst) {
            return false;
        }
        let poll = self.polls.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel_on.is_some_and(|cancel_on| poll >= cancel_on)
    }
}

fn controlled_import(cancel_on: Option<usize>) -> (ImportOperationControl, Arc<AtomicUsize>) {
    let armed = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let signal = ArmedPollCancellation {
        armed: Arc::clone(&armed),
        polls: Arc::clone(&polls),
        cancel_on,
    };
    let control = ImportOperationControl::default()
        .with_cancellation(signal)
        .with_progress_observer(
            move |event: ProgressEvent<ImportProgressPhase, ImportProgressUnit>| {
                if event.phase() == ImportProgressPhase::Detect {
                    armed.store(true, Ordering::SeqCst);
                }
                ProgressDecision::Continue
            },
        );
    (control, polls)
}

fn assert_cancelled(failure: &ImportFailure) {
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Control(ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                })
            )
    ));
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

fn encoded_payload() -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(vec![0_u8; DECODED_BYTES]);
    assert_eq!(encoded.len(), ENCODED_BYTES);
    encoded
}

fn data_uri(encoded: &str) -> String {
    format!("data:application/octet-stream;base64,{encoded}")
}

#[test]
fn whole_json_utf8_preflight_cancellation_is_bounded_by_scan_chunks() -> TestResult<()> {
    const PREFIX_CHUNKS: usize = 8;

    let invalid_offset = SCAN_CHUNK_BYTES * PREFIX_CHUNKS;
    let mut document = b"{\"asset\":{\"version\":\"2.0\"},\"extras\":\"".to_vec();
    document.resize(invalid_offset, b'x');
    document.push(u8::MAX);

    let decode_limit =
        u64::try_from(document.len() - 1).expect("JSON decode limit must fit the resource counter");
    let mut calibration_options = GltfTestOptions::memory();
    calibration_options.limits.max_decode_work = decode_limit;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let (calibration_result, calibration_diagnostics) = import_assets_result_with_control(
        "utf8-preflight-calibration.gltf",
        [("utf8-preflight-calibration.gltf", document.clone())],
        calibration_options,
        calibration_control,
    )?;
    let calibration_failure = expected_error(calibration_result)?;

    assert!(calibration_diagnostics.is_empty());
    assert_ledger_limit(
        &calibration_failure,
        ImportDimension::DecodeWork,
        decode_limit,
    );
    assert_eq!(
        dimension_consumed(&calibration_failure, ImportDimension::DecodeWork),
        0
    );
    let post_failure_poll = calibration_polls.load(Ordering::SeqCst);
    assert!(post_failure_poll > 0);

    let cancel_on = post_failure_poll
        .checked_add(PREFIX_CHUNKS)
        .expect("relative cancellation poll must fit usize");
    let (control, polls) = controlled_import(Some(cancel_on));
    let (result, diagnostics) = import_assets_result_with_control(
        "utf8-preflight-cancel.gltf",
        [("utf8-preflight-cancel.gltf", document)],
        GltfTestOptions::memory(),
        control,
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_cancelled(&failure);
    assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
    assert_eq!(dimension_consumed(&failure, ImportDimension::DecodeWork), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn data_uri_base64_scan_cancellation_is_bounded_by_scan_chunks() -> TestResult<()> {
    const PREFIX_CHUNKS: usize = 8;

    let mut encoded = encoded_payload().into_bytes();
    encoded[SCAN_CHUNK_BYTES * PREFIX_CHUNKS] = b' ';
    let encoded = String::from_utf8(encoded).expect("base64 scan fixture must remain ASCII");
    let uri = data_uri(&encoded);
    let document = triangle_gltf_with_buffer_uri(&uri, DECODED_BYTES);

    let string_limit =
        u64::try_from(uri.len() - 1).expect("data URI string limit must fit the resource counter");
    let mut calibration_options = GltfTestOptions::memory();
    calibration_options.limits.max_string_bytes = uri.len() - 1;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let (calibration_result, calibration_diagnostics) = import_assets_result_with_control(
        "base64-scan-calibration.gltf",
        [("base64-scan-calibration.gltf", document.clone())],
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
        document.len() as u64
    );
    let post_failure_poll = calibration_polls.load(Ordering::SeqCst);
    assert!(post_failure_poll > 0);

    let cancel_on = post_failure_poll
        .checked_add(PREFIX_CHUNKS)
        .expect("relative cancellation poll must fit usize");
    let (control, polls) = controlled_import(Some(cancel_on));
    let (result, diagnostics) = import_assets_result_with_control(
        "base64-scan-cancel.gltf",
        [("base64-scan-cancel.gltf", document.clone())],
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
        document.len() as u64
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn data_uri_base64_decode_cancellation_is_bounded_by_decode_chunks() -> TestResult<()> {
    let mut calibration_encoded = encoded_payload().into_bytes();
    calibration_encoded[0] = b'@';
    let calibration_encoded =
        String::from_utf8(calibration_encoded).expect("base64 calibration fixture must be ASCII");
    let calibration_uri = data_uri(&calibration_encoded);
    let calibration_document = triangle_gltf_with_buffer_uri(&calibration_uri, DECODED_BYTES);
    let (calibration_control, calibration_polls) = controlled_import(None);
    let (calibration_result, calibration_diagnostics) = import_assets_result_with_control(
        "base64-decode-calibration.gltf",
        [("base64-decode-calibration.gltf", calibration_document)],
        GltfTestOptions::memory(),
        calibration_control,
    )?;
    let calibration_failure = expected_error(calibration_result)?;

    assert!(calibration_diagnostics.is_empty());
    assert_gltf_failure(
        &calibration_failure,
        GltfFailureCode::InvalidReference,
        Some("invalid base64"),
    )?;
    let first_quantum_gate_poll = calibration_polls.load(Ordering::SeqCst);
    assert!(first_quantum_gate_poll > 0);

    let mut encoded = encoded_payload().into_bytes();
    encoded[SCAN_CHUNK_BYTES] = b'@';
    let encoded = String::from_utf8(encoded).expect("base64 target fixture must be ASCII");
    let uri = data_uri(&encoded);
    let document = triangle_gltf_with_buffer_uri(&uri, DECODED_BYTES);
    let cancel_on = first_quantum_gate_poll
        .checked_add(1)
        .expect("relative cancellation poll must fit usize");
    let (control, polls) = controlled_import(Some(cancel_on));
    let (result, diagnostics) = import_assets_result_with_control(
        "base64-decode-cancel.gltf",
        [("base64-decode-cancel.gltf", document.clone())],
        GltfTestOptions::memory(),
        control,
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_cancelled(&failure);
    assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
    assert_eq!(
        dimension_attempted(&failure, ImportDimension::DataUriBytes),
        DECODED_BYTES as u64
    );
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DataUriBytes),
        0
    );
    assert_eq!(
        dimension_consumed(&failure, ImportDimension::DecodeWork),
        document.len() as u64
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}
