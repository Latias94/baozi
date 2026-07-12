mod common;

use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportProgressPhase,
    ImportProgressUnit, ResourceLimits, ScopeAbortCause,
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
