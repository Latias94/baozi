mod common;

use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportProgressPhase,
    ImportProgressUnit, LedgerCause, ResourceLimits, ScopeAbortCause,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, ProgressDecision, ProgressEvent,
};
use common::{
    BinaryFacet, ascii_triangle, binary_stl, import_bytes_forced_with_limits,
    import_bytes_forced_with_limits_and_control,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const SCAN_CHUNK_BYTES: usize = 4 * 1024;

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

fn assert_ledger_limit(failure: &ImportFailure, dimension: ImportDimension, limit: u64) {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!(
            "expected scope-owned ledger failure, got {:?}",
            failure.cause()
        );
    };
    let ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
        dimension: actual_dimension,
        limit: actual_limit,
        ..
    }) = abort.cause()
    else {
        panic!("expected ledger limit failure, got {:?}", abort.cause());
    };
    assert_eq!(*actual_dimension, dimension);
    assert_eq!(*actual_limit, limit);
}

fn consumed(failure: &ImportFailure, dimension: ImportDimension) -> u64 {
    failure
        .ledger()
        .dimension(dimension)
        .expect("tested STL dimension is configured")
        .consumed()
}

fn attempted(failure: &ImportFailure, dimension: ImportDimension) -> u64 {
    failure
        .ledger()
        .dimension(dimension)
        .expect("tested STL dimension is configured")
        .attempted()
}

fn cancelled_failure(source: &str, bytes: &[u8], cancel_on: usize) -> ImportFailure {
    let (control, polls) = controlled_import(Some(cancel_on));
    let failure = import_bytes_forced_with_limits_and_control(
        source,
        bytes,
        ResourceLimits::default(),
        control,
    )
    .expect_err("the selected public checkpoint must observe cancellation");
    assert_cancelled(&failure);
    assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
    failure
}

fn first_cancelled_prefix_at_least(
    source: &str,
    bytes: &[u8],
    total_polls: usize,
    dimension: ImportDimension,
    target_consumed: u64,
) -> (usize, ImportFailure) {
    let mut low = 1usize;
    let mut high = total_polls;
    while low < high {
        let middle = low + (high - low) / 2;
        let failure = cancelled_failure(source, bytes, middle);
        if consumed(&failure, dimension) >= target_consumed {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let failure = cancelled_failure(source, bytes, low);
    assert!(consumed(&failure, dimension) >= target_consumed);
    (low, failure)
}

#[test]
fn ascii_decoded_text_budget_is_exact() {
    let document = ascii_triangle("decoded-budget");
    let decoded_bytes = document.len() as u64;

    let mut exact = ResourceLimits::default();
    exact.max_decoded_utf8_bytes = decoded_bytes;
    let report = import_bytes_forced_with_limits("decoded-exact.stl", document.as_bytes(), exact)
        .expect("the exact decoded-text byte limit must admit ASCII STL");
    let decoded = report
        .ledger()
        .dimension(ImportDimension::DecodedTextBytes)
        .expect("decoded text dimension is configured");
    assert_eq!(decoded.peak_live_bytes(), decoded_bytes);
    assert_eq!(decoded.current_live_bytes(), 0);

    let limit = decoded_bytes - 1;
    let mut below = ResourceLimits::default();
    below.max_decoded_utf8_bytes = limit;
    let failure =
        import_bytes_forced_with_limits("decoded-over-limit.stl", document.as_bytes(), below)
            .expect_err("one byte below the decoded ASCII length must fail before parsing");
    assert_ledger_limit(&failure, ImportDimension::DecodedTextBytes, limit);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn binary_stl_does_not_consume_decoded_text_budget() {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let mut limits = ResourceLimits::default();
    limits.max_decoded_utf8_bytes = 0;

    let report = import_bytes_forced_with_limits("binary-no-text.stl", &bytes, limits)
        .expect("binary STL must not require decoded-text storage");
    let decoded = report
        .ledger()
        .dimension(ImportDimension::DecodedTextBytes)
        .expect("decoded text dimension is configured");
    assert_eq!(decoded.attempted(), 0);
    assert_eq!(decoded.consumed(), 0);
    assert_eq!(decoded.peak_live_bytes(), 0);
}

#[test]
fn ascii_profile_cancellation_is_bounded_by_scan_chunks() {
    const PREFIX_CHUNKS: usize = 8;

    let mut document = ascii_triangle("profile").into_bytes();
    document.resize(document.len() + SCAN_CHUNK_BYTES * PREFIX_CHUNKS, b' ');
    document.push(b'\n');
    let (calibration_control, calibration_polls) = controlled_import(None);
    let _report = import_bytes_forced_with_limits_and_control(
        "profile-calibration.stl",
        &document,
        ResourceLimits::default(),
        calibration_control,
    )
    .expect("the valid long ASCII fixture must complete without cancellation");
    let total_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);

    let (cancel_on, failure) = first_cancelled_prefix_at_least(
        "profile-cancel.stl",
        &document,
        total_polls,
        ImportDimension::DecodeWork,
        SCAN_CHUNK_BYTES as u64,
    );
    assert_eq!(
        attempted(&failure, ImportDimension::DecodeWork),
        SCAN_CHUNK_BYTES as u64
    );
    assert_eq!(
        consumed(&failure, ImportDimension::DecodeWork),
        SCAN_CHUNK_BYTES as u64
    );

    let mut malformed = document;
    *malformed
        .last_mut()
        .expect("the ASCII cancellation fixture is non-empty") = 0;
    let malformed_failure =
        cancelled_failure("profile-malformed-cancel.stl", &malformed, cancel_on);
    assert_eq!(
        consumed(&malformed_failure, ImportDimension::DecodeWork),
        SCAN_CHUNK_BYTES as u64
    );
}

#[test]
fn ascii_solid_name_copy_cancellation_is_bounded_by_scan_chunks() {
    const NAME_CHUNKS: usize = 8;

    let name = "x".repeat(SCAN_CHUNK_BYTES * NAME_CHUNKS);
    let document = ascii_triangle(&name);
    let (calibration_control, calibration_polls) = controlled_import(None);
    let calibration_failure = import_bytes_forced_with_limits_and_control(
        "name-calibration.stl",
        document.as_bytes(),
        ResourceLimits::default(),
        calibration_control,
    )
    .expect_err("the deliberately overlong canonical name must be rejected after parsing");
    assert!(
        consumed(&calibration_failure, ImportDimension::CanonicalStringBytes) >= name.len() as u64
    );
    let total_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);

    let (copy_complete_poll, copy_complete_failure) = first_cancelled_prefix_at_least(
        "name-cancel.stl",
        document.as_bytes(),
        total_polls,
        ImportDimension::CanonicalStringBytes,
        name.len() as u64,
    );
    assert_eq!(
        consumed(
            &copy_complete_failure,
            ImportDimension::CanonicalStringBytes
        ),
        name.len() as u64
    );
    assert!(copy_complete_poll > 1);

    let failure = cancelled_failure(
        "name-last-copy-chunk-cancel.stl",
        document.as_bytes(),
        copy_complete_poll - 1,
    );
    assert_eq!(
        attempted(&failure, ImportDimension::CanonicalStringBytes),
        name.len() as u64
    );
    assert_eq!(consumed(&failure, ImportDimension::CanonicalStringBytes), 0);
}

#[test]
fn binary_facet_cancellation_is_bounded_to_one_record() {
    const FACET_COUNT: usize = 2 * SCAN_CHUNK_BYTES + 1;

    let facets = vec![BinaryFacet::unit_triangle(); FACET_COUNT];
    let bytes = binary_stl(b"binary", &facets);

    let (calibration_control, calibration_polls) = controlled_import(None);
    let _report = import_bytes_forced_with_limits_and_control(
        "facet-calibration.stl",
        &bytes,
        ResourceLimits::default(),
        calibration_control,
    )
    .expect("the valid binary facet fixture must complete without cancellation");
    let total_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);

    let (_, failure) = first_cancelled_prefix_at_least(
        "facet-cancel.stl",
        &bytes,
        total_polls,
        ImportDimension::Records,
        1,
    );
    assert_eq!(
        attempted(&failure, ImportDimension::Records),
        FACET_COUNT as u64
    );
    assert_eq!(consumed(&failure, ImportDimension::Records), 1);
}

#[test]
fn binary_color_expansion_cancellation_is_bounded_by_scan_chunks() {
    const FACET_COUNT: usize = 2 * SCAN_CHUNK_BYTES + 1;

    let mut colored = BinaryFacet::unit_triangle();
    colored.attribute = 0x8000;
    let facets = vec![colored; FACET_COUNT];
    let bytes = binary_stl(b"binary", &facets);

    let (calibration_control, calibration_polls) = controlled_import(None);
    let _report = import_bytes_forced_with_limits_and_control(
        "color-calibration.stl",
        &bytes,
        ResourceLimits::default(),
        calibration_control,
    )
    .expect("the valid colored binary fixture must complete without cancellation");
    let total_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);

    let (_, failure) = first_cancelled_prefix_at_least(
        "color-cancel.stl",
        &bytes,
        total_polls,
        ImportDimension::WorkUnits,
        1,
    );
    assert_eq!(
        attempted(&failure, ImportDimension::WorkUnits),
        FACET_COUNT as u64
    );
    assert_eq!(
        consumed(&failure, ImportDimension::WorkUnits),
        SCAN_CHUNK_BYTES as u64
    );
    assert_eq!(
        consumed(&failure, ImportDimension::Records),
        FACET_COUNT as u64
    );
}
