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

fn assert_scope_limit(failure: &ImportFailure, resource: &'static str, limit: u64) {
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: actual_resource,
                    limit: actual_limit,
                } if *actual_resource == resource && *actual_limit == limit
            )
    ));
}

fn consumed(failure: &ImportFailure, dimension: ImportDimension) -> u64 {
    failure
        .ledger()
        .dimension(dimension)
        .expect("tested STL dimension is configured")
        .consumed()
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

    let invalid_offset = SCAN_CHUNK_BYTES * PREFIX_CHUNKS;
    let mut bytes = Vec::with_capacity(invalid_offset + 1);
    bytes.extend_from_slice(b"solid ");
    bytes.resize(invalid_offset, b' ');
    bytes.push(0);

    let limit = bytes.len() as u64 - 1;
    let mut calibration_limits = ResourceLimits::default();
    calibration_limits.max_decode_work = limit;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let calibration_failure = import_bytes_forced_with_limits_and_control(
        "profile-calibration.stl",
        &bytes,
        calibration_limits,
        calibration_control,
    )
    .expect_err("decode-work calibration must fail before the ASCII profile scan");
    assert_ledger_limit(&calibration_failure, ImportDimension::DecodeWork, limit);
    assert_eq!(
        consumed(&calibration_failure, ImportDimension::DecodeWork),
        0
    );
    let profile_entry_poll = calibration_polls.load(Ordering::SeqCst);
    assert!(profile_entry_poll > 0);

    let cancel_on = profile_entry_poll + PREFIX_CHUNKS;
    let (control, _) = controlled_import(Some(cancel_on));
    let failure = import_bytes_forced_with_limits_and_control(
        "profile-cancel.stl",
        &bytes,
        ResourceLimits::default(),
        control,
    )
    .expect_err("ASCII profile scanning must observe cancellation before the distant NUL");

    assert_cancelled(&failure);
    assert_eq!(consumed(&failure, ImportDimension::DecodeWork), 0);
}

#[test]
fn ascii_solid_name_copy_cancellation_is_bounded_by_scan_chunks() {
    const NAME_CHUNKS: usize = 8;

    let name = "x".repeat(SCAN_CHUNK_BYTES * NAME_CHUNKS);
    let document = ascii_triangle(&name);
    let string_limit = name.len() as u64 - 1;
    let mut calibration_limits = ResourceLimits::default();
    calibration_limits.max_string_bytes = string_limit as usize;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let calibration_failure = import_bytes_forced_with_limits_and_control(
        "name-calibration.stl",
        document.as_bytes(),
        calibration_limits,
        calibration_control,
    )
    .expect_err("string-byte calibration must fail at the first solid-name copy");
    assert_scope_limit(&calibration_failure, "string-bytes", string_limit);
    let name_copy_entry_poll = calibration_polls.load(Ordering::SeqCst);
    assert!(name_copy_entry_poll > 0);

    let (control, _) = controlled_import(Some(name_copy_entry_poll + 2));
    let failure = import_bytes_forced_with_limits_and_control(
        "name-cancel.stl",
        document.as_bytes(),
        ResourceLimits::default(),
        control,
    )
    .expect_err("solid-name copying must observe cancellation before its second chunk");

    assert_cancelled(&failure);
    assert_eq!(consumed(&failure, ImportDimension::CanonicalStringBytes), 0);
}

#[test]
fn binary_facet_cancellation_is_bounded_to_one_record() {
    const FACET_COUNT: usize = 2 * SCAN_CHUNK_BYTES + 1;

    let facets = vec![BinaryFacet::unit_triangle(); FACET_COUNT];
    let bytes = binary_stl(b"binary", &facets);

    let mut calibration_limits = ResourceLimits::default();
    calibration_limits.max_records = 0;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let calibration_failure = import_bytes_forced_with_limits_and_control(
        "facet-calibration.stl",
        &bytes,
        calibration_limits,
        calibration_control,
    )
    .expect_err("record calibration must fail immediately before binary facet iteration");
    assert_ledger_limit(&calibration_failure, ImportDimension::Records, 0);
    let facet_entry_poll = calibration_polls.load(Ordering::SeqCst);
    assert!(facet_entry_poll > 0);

    let mut post_loop_limits = ResourceLimits::default();
    post_loop_limits.max_meshes = 0;
    let (post_loop_control, post_loop_polls) = controlled_import(None);
    let post_loop_failure = import_bytes_forced_with_limits_and_control(
        "facet-post-loop.stl",
        &bytes,
        post_loop_limits,
        post_loop_control,
    )
    .expect_err("mesh calibration must fail after every binary facet was consumed");
    assert_ledger_limit(&post_loop_failure, ImportDimension::Meshes, 0);
    assert_eq!(
        consumed(&post_loop_failure, ImportDimension::Records),
        FACET_COUNT as u64
    );
    assert!(
        post_loop_polls.load(Ordering::SeqCst) - facet_entry_poll >= FACET_COUNT,
        "binary parsing must poll at least once per facet"
    );

    let (control, _) = controlled_import(Some(facet_entry_poll + 2));
    let failure = import_bytes_forced_with_limits_and_control(
        "facet-cancel.stl",
        &bytes,
        ResourceLimits::default(),
        control,
    )
    .expect_err("binary facet parsing must cancel before consuming its second record");

    assert_cancelled(&failure);
    assert_eq!(consumed(&failure, ImportDimension::Records), 0);
}

#[test]
fn binary_color_expansion_cancellation_is_bounded_by_scan_chunks() {
    const FACET_COUNT: usize = 2 * SCAN_CHUNK_BYTES + 1;

    let mut colored = BinaryFacet::unit_triangle();
    colored.attribute = 0x8000;
    let facets = vec![colored; FACET_COUNT];
    let bytes = binary_stl(b"binary", &facets);

    let mut calibration_limits = ResourceLimits::default();
    calibration_limits.max_work_units = 0;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let calibration_failure = import_bytes_forced_with_limits_and_control(
        "color-calibration.stl",
        &bytes,
        calibration_limits,
        calibration_control,
    )
    .expect_err("work calibration must fail immediately before binary color expansion");
    assert_ledger_limit(&calibration_failure, ImportDimension::WorkUnits, 0);
    let color_entry_poll = calibration_polls.load(Ordering::SeqCst);
    assert!(color_entry_poll > 0);

    let expected_color_polls = FACET_COUNT.div_ceil(SCAN_CHUNK_BYTES);
    let mut post_loop_limits = ResourceLimits::default();
    post_loop_limits.max_meshes = 0;
    let (post_loop_control, post_loop_polls) = controlled_import(None);
    let post_loop_failure = import_bytes_forced_with_limits_and_control(
        "color-post-loop.stl",
        &bytes,
        post_loop_limits,
        post_loop_control,
    )
    .expect_err("mesh calibration must fail after binary color expansion");
    assert_eq!(
        consumed(&post_loop_failure, ImportDimension::WorkUnits),
        FACET_COUNT as u64
    );
    assert_eq!(
        consumed(&post_loop_failure, ImportDimension::Records),
        FACET_COUNT as u64
    );
    assert!(
        post_loop_polls.load(Ordering::SeqCst) - color_entry_poll > expected_color_polls,
        "binary color expansion must poll at least once per 4096 facets"
    );

    let (control, _) = controlled_import(Some(color_entry_poll + 2));
    let failure = import_bytes_forced_with_limits_and_control(
        "color-cancel.stl",
        &bytes,
        ResourceLimits::default(),
        control,
    )
    .expect_err("binary color expansion must cancel before its second chunk completes");

    assert_cancelled(&failure);
    let expansion = failure
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("work-unit dimension is configured");
    assert_eq!(expansion.attempted(), FACET_COUNT as u64);
    assert_eq!(expansion.consumed(), 0);
    assert_eq!(consumed(&failure, ImportDimension::Records), 0);
}
