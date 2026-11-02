mod common;

use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportProgressPhase,
    ImportProgressUnit, ImportReport, ImportTerminalFailureClass, LedgerCause,
    MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS, MetadataOwnerRef, MetadataValueRef, ResourceLimits,
    ScopeAbortCause,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, ProgressDecision, ProgressEvent,
};
use common::{
    TestResult, assert_scope_resource_limit, expected_error, expected_report,
    import_bytes_forced_result, import_bytes_forced_result_with_control,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const SCAN_CHUNK_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy)]
enum BinaryVertexProfile {
    PositionsOnly,
    BuiltinStreams,
    BuiltinF64Streams,
    CustomStreams,
    CustomF64Streams,
}

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

fn dimension_consumed(failure: &ImportFailure, dimension: ImportDimension) -> u64 {
    failure
        .ledger()
        .dimension(dimension)
        .expect("tested PLY dimension is configured")
        .consumed()
}

fn assert_ledger_limit(failure: &ImportFailure, dimension: ImportDimension, limit: u64) {
    assert!(
        matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                        dimension: actual_dimension,
                        limit: actual_limit,
                        ..
                    }) if *actual_dimension == dimension && *actual_limit == limit
                )
        ),
        "expected {dimension:?} limit {limit}, got {:?}",
        failure.cause()
    );
}

fn first_cancelled_dimension_prefix_at_least(
    total_polls: usize,
    dimension: ImportDimension,
    target_consumed: u64,
    mut run_cancelled: impl FnMut(usize) -> TestResult<ImportFailure>,
) -> TestResult<(usize, ImportFailure)> {
    let mut low = 1;
    let mut high = total_polls;
    while low < high {
        let middle = low + (high - low) / 2;
        let failure = run_cancelled(middle)?;
        if dimension_consumed(&failure, dimension) >= target_consumed {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let failure = run_cancelled(low)?;
    assert!(dimension_consumed(&failure, dimension) >= target_consumed);
    Ok((low, failure))
}

fn short_metadata_document(comment_count: usize, obj_info_count: usize) -> String {
    let mut document = String::from("ply\nformat ascii 1.0\n");
    for index in 0..comment_count.max(obj_info_count) {
        if index < comment_count {
            document.push_str(&format!("comment {}\n", index % 10));
        }
        if index < obj_info_count {
            document.push_str(&format!("obj_info {}\n", index % 10));
        }
    }
    document.push_str(
        "element vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
    );
    document
}

fn assert_metadata(
    report: &ImportReport,
    source: &str,
    comment_count: usize,
    obj_info_count: usize,
) {
    let mut expected = vec![
        ("ply:format".to_owned(), "ascii".to_owned()),
        ("ply:source".to_owned(), source.to_owned()),
    ];
    for index in 0..comment_count {
        expected.push((format!("ply:comment_{index}"), (index % 10).to_string()));
    }
    for index in 0..obj_info_count {
        expected.push((format!("ply:obj_info_{index}"), (index % 10).to_string()));
    }
    expected.sort_unstable_by(|left, right| left.0.cmp(&right.0));

    let scene = report.scene();
    let owner = scene
        .meshes()
        .next()
        .expect("metadata fixture retains its logical mesh")
        .metadata_owner();
    assert_eq!(scene.metadata_count(), expected.len());
    for (entry, (key, value)) in scene.metadata_entries().zip(&expected) {
        assert_eq!(entry.owner(), MetadataOwnerRef::Entity(owner));
        assert_eq!(entry.key(), key.as_str());
        assert_eq!(entry.value(), MetadataValueRef::String(value.as_str()));
    }
}

fn assert_metadata_work_growth(
    comments_per_row: usize,
    obj_infos_per_row: usize,
) -> TestResult<()> {
    const SOURCE: &str = "metadata-growth.ply";
    let mut work = [0_u64; 3];
    for (sample, count) in [1_000, 5_000, 20_000].into_iter().enumerate() {
        let comment_count = count * comments_per_row;
        let obj_info_count = count * obj_infos_per_row;
        let document = short_metadata_document(comment_count, obj_info_count);
        if comment_count == 20_000 && obj_info_count == 0 {
            // Preserve the audited reproducer's byte size without pinning its broken work charge.
            assert_eq!(document.len(), 200_106);
        }
        let report = expected_report(import_bytes_forced_result(
            SOURCE,
            document.as_bytes(),
            ResourceLimits::default(),
        )?)?;
        assert!(report.diagnostics().is_empty());
        assert_metadata(&report, SOURCE, comment_count, obj_info_count);
        assert_eq!(report.ledger().current_live_bytes(), 0);
        assert_eq!(report.ledger().active_reservations(), 0);
        let dimension = report
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .expect("metadata work remains observable");
        assert_eq!(dimension.attempted(), dimension.consumed());
        work[sample] = dimension.consumed();
    }

    eprintln!(
        "PLY metadata work (comments/row={comments_per_row}, obj_info/row={obj_infos_per_row}, rows=1000/5000/20000): {work:?}"
    );
    // Allow linear or n log n ordering, while rejecting the old 25x and 16x growth.
    assert!(work[0] > 0 && work[1] > work[0] && work[2] > work[1]);
    assert!(
        work[1] <= work[0] * 8,
        "metadata work grew too fast: {work:?}"
    );
    assert!(
        work[2] <= work[1] * 6,
        "metadata work grew too fast: {work:?}"
    );
    Ok(())
}

fn binary_vertex_document(vertex_count: usize, profile: BinaryVertexProfile) -> Vec<u8> {
    let extra_properties = match profile {
        BinaryVertexProfile::PositionsOnly => {
            // Keep the same scalar-read width without incomplete channels requiring loss authority.
            // Repeated Position components retain this profile's single output stream.
            ["x", "y", "z", "x", "y", "z", "x", "y"]
        }
        BinaryVertexProfile::BuiltinStreams | BinaryVertexProfile::BuiltinF64Streams => {
            ["nx", "ny", "nz", "s", "t", "red", "green", "blue"]
        }
        BinaryVertexProfile::CustomStreams | BinaryVertexProfile::CustomF64Streams => [
            "custom_0", "custom_1", "custom_2", "custom_3", "custom_4", "custom_5", "custom_6",
            "custom_7",
        ],
    };
    let mut bytes = format!(
        "ply\nformat binary_little_endian 1.0\nelement vertex {vertex_count}\nproperty float x\nproperty float y\nproperty float z\n"
    )
    .into_bytes();
    let wide_extra_scalars = matches!(
        profile,
        BinaryVertexProfile::BuiltinF64Streams | BinaryVertexProfile::CustomF64Streams
    );
    let scalar = if wide_extra_scalars {
        "double"
    } else {
        "float"
    };
    for property in extra_properties {
        bytes.extend_from_slice(format!("property {scalar} {property}\n").as_bytes());
    }
    bytes.extend_from_slice(b"end_header\n");
    for _ in 0..vertex_count {
        for _ in 0..3 {
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        for _ in 0..8 {
            if wide_extra_scalars {
                bytes.extend_from_slice(&0.0_f64.to_le_bytes());
            } else {
                bytes.extend_from_slice(&0.0_f32.to_le_bytes());
            }
        }
    }
    bytes
}

fn binary_face_document(face_count: usize) -> Vec<u8> {
    let mut bytes = format!(
        "ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face {face_count}\nproperty list uchar uint vertex_indices\nend_header\n"
    )
    .into_bytes();
    for position in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for component in position {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    let mut face_record = Vec::with_capacity(13);
    face_record.push(3);
    for index in [0_u32, 1, 2] {
        face_record.extend_from_slice(&index.to_le_bytes());
    }
    for _ in 0..face_count {
        bytes.extend_from_slice(&face_record);
    }
    bytes
}

fn polls_until_graph_limit(source: &str, bytes: &[u8]) -> TestResult<usize> {
    let mut limits = ResourceLimits::default();
    limits.max_graph_nodes = 0;
    let (control, polls) = controlled_import(None);
    let failure = expected_error(import_bytes_forced_result_with_control(
        source, bytes, limits, control,
    )?)?;

    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                    dimension: ImportDimension::GraphNodes,
                    limit: 0,
                    ..
                })
            )
    ));
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(polls.load(Ordering::SeqCst))
}

#[test]
fn pre_reserved_vertex_and_face_streams_do_not_add_per_item_polls() -> TestResult<()> {
    const SMALL_COUNT: usize = 16;
    const LARGE_COUNT: usize = 32;
    const ADDITIONAL_ITEMS: usize = LARGE_COUNT - SMALL_COUNT;

    let vertex_poll_delta = |profile, label: &str| -> TestResult<usize> {
        let small = binary_vertex_document(SMALL_COUNT, profile);
        let large = binary_vertex_document(LARGE_COUNT, profile);
        let small_polls = polls_until_graph_limit(&format!("{label}-small.ply"), &small)?;
        let large_polls = polls_until_graph_limit(&format!("{label}-large.ply"), &large)?;
        Ok(large_polls
            .checked_sub(small_polls)
            .expect("larger binary fixture must not require fewer polls"))
    };

    let positions_delta = vertex_poll_delta(BinaryVertexProfile::PositionsOnly, "positions-only")?;
    // Planning and decoding poll at bounded property frontiers, including row entry.
    // Extra pre-reserved attribute streams must not add per-scalar control calls.
    assert!(positions_delta > 0);
    assert!(positions_delta <= ADDITIONAL_ITEMS * 3);
    assert_eq!(
        vertex_poll_delta(BinaryVertexProfile::BuiltinStreams, "builtin-streams")?,
        positions_delta
    );
    assert_eq!(
        vertex_poll_delta(
            BinaryVertexProfile::BuiltinF64Streams,
            "builtin-f64-streams"
        )?,
        positions_delta
    );
    assert_eq!(
        vertex_poll_delta(BinaryVertexProfile::CustomStreams, "custom-streams")?,
        positions_delta
    );
    assert_eq!(
        vertex_poll_delta(BinaryVertexProfile::CustomF64Streams, "custom-f64-streams")?,
        positions_delta
    );

    let small_faces = binary_face_document(SMALL_COUNT);
    let large_faces = binary_face_document(LARGE_COUNT);
    let small_face_polls = polls_until_graph_limit("faces-small.ply", &small_faces)?;
    let large_face_polls = polls_until_graph_limit("faces-large.ply", &large_faces)?;
    let face_poll_delta = large_face_polls
        .checked_sub(small_face_polls)
        .expect("larger face fixture must not require fewer polls");
    assert!(
        face_poll_delta <= ADDITIONAL_ITEMS * 7,
        "pre-reserved face streams added redundant per-item polls: {face_poll_delta}"
    );
    Ok(())
}

#[test]
fn ascii_comment_copy_cancellation_is_bounded_by_string_chunks() -> TestResult<()> {
    const COMMENT_CHUNKS: usize = 8;
    const COMMENT_BYTES: usize = SCAN_CHUNK_BYTES * COMMENT_CHUNKS;

    let comment = "x".repeat(COMMENT_BYTES);
    let mut document = String::from("ply\nformat ascii 1.0\ncomment ");
    document.push_str(&comment);
    document.push_str(
        "\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n",
    );

    let string_limit =
        u64::try_from(COMMENT_BYTES - 1).expect("comment byte limit must fit the resource counter");
    let mut calibration_limits = ResourceLimits::default();
    calibration_limits.max_string_bytes = COMMENT_BYTES - 1;
    let (calibration_control, calibration_polls) = controlled_import(None);
    let calibration_failure = expected_error(import_bytes_forced_result_with_control(
        "comment-copy-calibration.ply",
        document.as_bytes(),
        calibration_limits,
        calibration_control,
    )?)?;

    assert_scope_resource_limit(&calibration_failure, "string-bytes", string_limit);
    let calibration_strings = calibration_failure
        .ledger()
        .dimension(ImportDimension::CanonicalStringBytes)
        .expect("canonical string byte dimension is configured");
    assert_eq!(calibration_strings.attempted(), COMMENT_BYTES as u64);
    assert_eq!(calibration_strings.consumed(), 0);
    let post_failure_poll = calibration_polls.load(Ordering::SeqCst);
    assert!(post_failure_poll > 0);

    let cancel_on = post_failure_poll
        .checked_add(1)
        .expect("relative cancellation poll must fit usize");
    let (control, polls) = controlled_import(Some(cancel_on));
    let failure = expected_error(import_bytes_forced_result_with_control(
        "comment-copy-cancel.ply",
        document.as_bytes(),
        ResourceLimits::default(),
        control,
    )?)?;

    assert_cancelled(&failure);
    assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
    let strings = failure
        .ledger()
        .dimension(ImportDimension::CanonicalStringBytes)
        .expect("canonical string byte dimension is configured");
    assert_eq!(strings.attempted(), COMMENT_BYTES as u64);
    assert_eq!(strings.consumed(), 0);
    assert_eq!(dimension_consumed(&failure, ImportDimension::Records), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn short_comments_retain_ordered_metadata_with_bounded_work_growth() -> TestResult<()> {
    assert_metadata_work_growth(1, 0)
}

#[test]
fn short_obj_infos_retain_ordered_metadata_with_bounded_work_growth() -> TestResult<()> {
    assert_metadata_work_growth(0, 1)
}

#[test]
fn interleaved_comments_and_obj_infos_retain_ordered_metadata_with_bounded_work_growth()
-> TestResult<()> {
    assert_metadata_work_growth(1, 1)
}

#[test]
fn metadata_obeys_the_exact_work_budget() -> TestResult<()> {
    const SOURCE: &str = "metadata-work.ply";
    let document = short_metadata_document(1_000, 1_000);
    let baseline = expected_report(import_bytes_forced_result(
        SOURCE,
        document.as_bytes(),
        ResourceLimits::default(),
    )?)?;
    let required = baseline
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("metadata work remains observable")
        .consumed();
    assert!(required > 0);

    let mut exact = ResourceLimits::default();
    exact.max_work_units = required;
    let report = expected_report(import_bytes_forced_result(
        SOURCE,
        document.as_bytes(),
        exact,
    )?)?;
    assert_metadata(&report, SOURCE, 1_000, 1_000);
    let work = report
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("metadata work remains observable");
    assert_eq!((work.attempted(), work.consumed()), (required, required));

    let mut one_short = ResourceLimits::default();
    one_short.max_work_units = required - 1;
    let failure = expected_error(import_bytes_forced_result(
        SOURCE,
        document.as_bytes(),
        one_short,
    )?)?;
    // Whole-import exhaustion can belong to the retained-store terminal after parser work.
    if let ImportCause::OperationTerminal(terminal) = failure.cause() {
        assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
    } else {
        assert_ledger_limit(&failure, ImportDimension::WorkUnits, required - 1);
    }
    let work = failure
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("metadata work remains observable");
    assert_eq!(work.attempted(), required);
    assert!(work.consumed() < required);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    Ok(())
}

#[test]
fn metadata_ordering_cancellation_retains_completed_comparisons() -> TestResult<()> {
    const SOURCE: &str = "metadata-sort.ply";
    let document = short_metadata_document(256, 256);

    // Header strings fit; admitting the completed metadata strings exceeds this calibration cap.
    let mut string_limits = ResourceLimits::default();
    string_limits.max_canonical_string_bytes = 1_024;
    let calibration = expected_error(import_bytes_forced_result(
        SOURCE,
        document.as_bytes(),
        string_limits,
    )?)?;
    assert_ledger_limit(&calibration, ImportDimension::CanonicalStringBytes, 1_024);
    let prepared_strings = calibration
        .ledger()
        .dimension(ImportDimension::CanonicalStringBytes)
        .expect("metadata string admission remains observable")
        .attempted();
    assert!(prepared_strings > 1_024);

    let (control, polls) = controlled_import(None);
    let _report = expected_report(import_bytes_forced_result_with_control(
        SOURCE,
        document.as_bytes(),
        ResourceLimits::default(),
        control,
    )?)?;
    let total_polls = polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);
    let mut run_cancelled = |cancel_on| -> TestResult<ImportFailure> {
        let (control, polls) = controlled_import(Some(cancel_on));
        let failure = expected_error(import_bytes_forced_result_with_control(
            SOURCE,
            document.as_bytes(),
            ResourceLimits::default(),
            control,
        )?)?;
        assert_cancelled(&failure);
        assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
        assert_eq!(failure.ledger().committed_canonical_members(), 0);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
        Ok(failure)
    };

    // Locate normalization completion through the ledger instead of fixed global poll numbers.
    let (prepared_poll, prepared) = first_cancelled_dimension_prefix_at_least(
        total_polls,
        ImportDimension::CanonicalStringBytes,
        prepared_strings,
        &mut run_cancelled,
    )?;
    assert_eq!(
        dimension_consumed(&prepared, ImportDimension::CanonicalStringBytes),
        prepared_strings
    );
    let work_before_sort = dimension_consumed(&prepared, ImportDimension::WorkUnits);
    assert_eq!(
        prepared
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .expect("metadata work remains observable")
            .attempted(),
        work_before_sort
    );

    let (comparison_poll, failure) = first_cancelled_dimension_prefix_at_least(
        total_polls,
        ImportDimension::WorkUnits,
        work_before_sort + 1,
        &mut run_cancelled,
    )?;
    assert!(comparison_poll > prepared_poll);
    // Each comparison now completes immediately; 64-item envelopes no longer replay
    // an unperformed suffix. Only control polling uses the shared geometry frontier.
    for (failure, frontiers) in [(failure, 1), (run_cancelled(comparison_poll + 1)?, 2)] {
        let work = failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .expect("metadata work remains observable");
        assert_eq!(
            work.attempted() - work_before_sort,
            frontiers * MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS as u64
        );
        assert_eq!(
            work.consumed() - work_before_sort,
            frontiers * MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS as u64
        );
        assert_eq!(
            dimension_consumed(&failure, ImportDimension::CanonicalStringBytes),
            prepared_strings,
            "comparison cancellation must precede canonical metadata construction"
        );
    }
    Ok(())
}
