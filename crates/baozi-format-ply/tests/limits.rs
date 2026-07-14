mod common;

use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportProgressPhase,
    ImportProgressUnit, LedgerCause, ResourceLimits, ScopeAbortCause,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, ProgressDecision, ProgressEvent,
};
use common::{
    TestResult, assert_scope_resource_limit, expected_error,
    import_bytes_forced_result_with_control,
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
    CustomStreams,
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

fn binary_vertex_document(vertex_count: usize, profile: BinaryVertexProfile) -> Vec<u8> {
    let extra_properties = match profile {
        BinaryVertexProfile::PositionsOnly => {
            ["nx", "s", "u", "texture_u", "red", "r", "green", "g"]
        }
        BinaryVertexProfile::BuiltinStreams => ["nx", "ny", "nz", "s", "t", "red", "green", "blue"],
        BinaryVertexProfile::CustomStreams => [
            "custom_0", "custom_1", "custom_2", "custom_3", "custom_4", "custom_5", "custom_6",
            "custom_7",
        ],
    };
    let mut bytes = format!(
        "ply\nformat binary_little_endian 1.0\nelement vertex {vertex_count}\nproperty float x\nproperty float y\nproperty float z\n"
    )
    .into_bytes();
    for property in extra_properties {
        bytes.extend_from_slice(format!("property float {property}\n").as_bytes());
    }
    bytes.extend_from_slice(b"end_header\n");
    for _ in 0..vertex_count {
        for _ in 0..11 {
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
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
    assert_eq!(positions_delta, ADDITIONAL_ITEMS * 3);
    assert_eq!(
        vertex_poll_delta(BinaryVertexProfile::BuiltinStreams, "builtin-streams")?,
        positions_delta
    );
    assert_eq!(
        vertex_poll_delta(BinaryVertexProfile::CustomStreams, "custom-streams")?,
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
