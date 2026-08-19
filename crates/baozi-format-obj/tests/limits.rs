mod common;

use baozi_import::{
    FormatFailureDetailValue, ImportCause, ImportDimension, ImportOperationControl,
    ImportProgressPhase, ImportProgressUnit, ImportTerminalCause, ImportTerminalFailureClass,
    ResourceLimits, ScopeAbortCause,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, ProgressDecision, ProgressEvent,
};
use common::{
    TestResult, expected_failure, expected_success, import_assets_result,
    import_assets_result_with_control, import_bytes_result, scene_snapshot,
    snapshot_line_containing,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const SCAN_CHUNK_BYTES: usize = 4 * 1024;
const SORT_COMPARISON_CHUNK: u64 = 64;

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
    controlled_import_from_phase(ImportProgressPhase::Detect, cancel_on)
}

fn controlled_import_from_phase(
    arm_phase: ImportProgressPhase,
    cancel_on: Option<usize>,
) -> (ImportOperationControl, Arc<AtomicUsize>) {
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
                if event.phase() == arm_phase {
                    armed.store(true, Ordering::SeqCst);
                }
                ProgressDecision::Continue
            },
        );
    (control, polls)
}

fn assert_cancelled(failure: &baozi_import::ImportFailure) {
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

fn work_consumed(failure: &baozi_import::ImportFailure, dimension: ImportDimension) -> u64 {
    failure
        .ledger()
        .dimension(dimension)
        .expect("tested work dimension is configured")
        .consumed()
}

fn first_cancelled_work_prefix_at_least(
    total_polls: usize,
    target_consumed: u64,
    mut run_cancelled: impl FnMut(usize) -> TestResult<baozi_import::ImportFailure>,
) -> TestResult<(usize, baozi_import::ImportFailure)> {
    let mut low = 1usize;
    let mut high = total_polls;
    while low < high {
        let middle = low + (high - low) / 2;
        let failure = run_cancelled(middle)?;
        if work_consumed(&failure, ImportDimension::WorkUnits) >= target_consumed {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let failure = run_cancelled(low)?;
    assert!(work_consumed(&failure, ImportDimension::WorkUnits) >= target_consumed);
    Ok((low, failure))
}

fn format_byte_offset(failure: &baozi_import::ImportFailure) -> Option<u64> {
    let ImportCause::Format(envelope) = failure.cause() else {
        return None;
    };
    envelope
        .details()
        .iter()
        .find(|detail| detail.key() == "byte_offset")
        .and_then(|detail| match detail.value() {
            FormatFailureDetailValue::Unsigned(value) => Some(value),
            _ => None,
        })
}

fn reverse_material_library(material_count: usize) -> String {
    let mut library = String::new();
    for material in (0..material_count).rev() {
        library.push_str(&format!("newmtl material-{material:04}\n"));
    }
    library
}

fn polygon_document(vertex_count: usize) -> String {
    let mut document = String::new();
    for vertex in 0..vertex_count {
        document.push_str(&format!("v {vertex} 0 0\n"));
    }
    document.push('f');
    for vertex in (1..=vertex_count).rev() {
        document.push_str(&format!(" {vertex}"));
    }
    document.push('\n');
    document
}

fn assert_scope_limit(failure: &baozi_import::ImportFailure, resource: &'static str, limit: u64) {
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: actual,
                    limit: actual_limit,
                } if *actual == resource && *actual_limit == limit
            )
    ));
}

fn assert_ledger_limit(failure: &baozi_import::ImportFailure, dimension: ImportDimension) {
    match failure.cause() {
        ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::Ledger(_)) => {}
        ImportCause::OperationTerminal(terminal) => {
            assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
            assert_eq!(
                failure.ledger().terminal_cause(),
                ImportTerminalCause::Failure(*terminal)
            );
        }
        cause => panic!("expected ledger failure, got {cause:?}"),
    }
    let snapshot = failure
        .ledger()
        .dimension(dimension)
        .expect("tested import dimension is configured");
    assert!(snapshot.attempted() > snapshot.consumed());
}

fn assert_live_limit(failure: &baozi_import::ImportFailure, limit: u64) {
    match failure.cause() {
        ImportCause::Scope(abort) => match abort.cause() {
            ScopeAbortCause::Ledger(cause) => {
                assert!(cause.to_string().ends_with(&format!("above limit {limit}")));
            }
            cause => panic!("expected ledger failure, got {cause:?}"),
        },
        ImportCause::OperationTerminal(terminal) => {
            assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
            assert_eq!(
                failure.ledger().terminal_cause(),
                ImportTerminalCause::Failure(*terminal)
            );
            assert!(failure.ledger().peak_live_bytes() <= limit);
        }
        cause => panic!("expected ledger failure, got {cause:?}"),
    }
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn primary_utf8_and_line_scans_poll_within_fixed_byte_chunks() -> TestResult<()> {
    let mut document = String::from("#");
    document.push_str(&"x".repeat(SCAN_CHUNK_BYTES - 2));
    document.push('\u{e9}');
    document.push_str(&"x".repeat(SCAN_CHUNK_BYTES * 8 - document.len()));
    assert_eq!(document.len(), SCAN_CHUNK_BYTES * 8);
    let line_bytes = document.len();
    let expected_scan_polls =
        document.len().div_ceil(SCAN_CHUNK_BYTES) + line_bytes.div_ceil(SCAN_CHUNK_BYTES);
    let expected_decode_work = document
        .len()
        .checked_add(line_bytes)
        .expect("small fixture scan work fits usize") as u64;
    let mut limits = ResourceLimits::default();
    limits.max_records = 0;
    let (control, polls) = controlled_import(None);

    let failure = expected_failure(import_assets_result_with_control(
        "long-comment.obj",
        [("long-comment.obj", document.as_bytes())],
        limits,
        false,
        control,
    )?);

    assert_ledger_limit(&failure, ImportDimension::Records);
    assert_eq!(
        work_consumed(&failure, ImportDimension::DecodeWork),
        expected_decode_work
    );
    assert!(
        polls.load(Ordering::SeqCst) >= expected_scan_polls,
        "UTF-8 and line scanning must poll at least once per {SCAN_CHUNK_BYTES}-byte chunk"
    );
    Ok(())
}

#[test]
fn primary_ascii_decode_adds_only_chunk_bounded_polls() -> TestResult<()> {
    let short_document = String::from("#");
    let mut long_document = String::from("#");
    long_document.push_str(&"x".repeat(SCAN_CHUNK_BYTES * 8 - 1));
    let extra_chunks = long_document.len().div_ceil(SCAN_CHUNK_BYTES)
        - short_document.len().div_ceil(SCAN_CHUNK_BYTES);
    let mut limits = ResourceLimits::default();
    limits.max_records = 0;

    let (short_control, short_polls) = controlled_import(None);
    let short_failure = expected_failure(import_assets_result_with_control(
        "short-ascii.obj",
        [("short-ascii.obj", short_document.as_bytes())],
        limits.clone(),
        false,
        short_control,
    )?);
    let (long_control, long_polls) = controlled_import(None);
    let long_failure = expected_failure(import_assets_result_with_control(
        "long-ascii.obj",
        [("long-ascii.obj", long_document.as_bytes())],
        limits.clone(),
        false,
        long_control,
    )?);

    assert_ledger_limit(&short_failure, ImportDimension::Records);
    assert_ledger_limit(&long_failure, ImportDimension::Records);
    let short_poll_count = short_polls.load(Ordering::SeqCst);
    let actual_extra_polls = long_polls.load(Ordering::SeqCst) - short_poll_count;
    assert!(
        actual_extra_polls >= extra_chunks * 2,
        "long ASCII decode added {actual_extra_polls} polls, expected at least {} chunk-boundary polls",
        extra_chunks * 2
    );
    // Each extra chunk crosses the planning and decode callback boundaries plus the bounded
    // line-index scan; the constant absorbs phase-level polls shared by short and long inputs.
    assert!(
        actual_extra_polls <= extra_chunks * 5 + 4,
        "long ASCII decode added {actual_extra_polls} polls for {extra_chunks} extra chunks"
    );

    let (cancel_control, _) = controlled_import(Some(short_poll_count + 1));
    let cancelled = expected_failure(import_assets_result_with_control(
        "cancelled-long-ascii.obj",
        [("cancelled-long-ascii.obj", long_document.as_bytes())],
        limits,
        false,
        cancel_control,
    )?);
    assert_cancelled(&cancelled);
    Ok(())
}

#[test]
fn chunked_utf8_validation_reports_the_global_byte_offset() -> TestResult<()> {
    let invalid_offset = SCAN_CHUNK_BYTES + 17;
    let mut document = vec![b'#'; invalid_offset + 1];
    document[invalid_offset] = u8::MAX;

    let failure = expected_failure(import_bytes_result(
        "chunked-invalid-utf8.obj",
        &document,
        ResourceLimits::default(),
    )?);

    assert_eq!(format_byte_offset(&failure), Some(invalid_offset as u64));
    Ok(())
}

#[test]
fn mtl_utf8_and_line_scans_charge_each_fixed_byte_chunk() -> TestResult<()> {
    let obj = b"mtllib materials.mtl\nusemtl material-0000\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let short_mtl = String::from("#");
    let mut long_mtl = String::from("#");
    long_mtl.push_str(&"x".repeat(SCAN_CHUNK_BYTES - 2));
    long_mtl.push('\u{e9}');
    long_mtl.push_str(&"x".repeat(SCAN_CHUNK_BYTES * 8 - long_mtl.len()));
    let mut limits = ResourceLimits::default();
    limits.max_records = 6;

    let (short_control, short_polls) = controlled_import(None);
    let short_failure = expected_failure(import_assets_result_with_control(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("materials.mtl", short_mtl.as_bytes()),
        ],
        limits.clone(),
        true,
        short_control,
    )?);
    let (long_control, long_polls) = controlled_import(None);
    let long_failure = expected_failure(import_assets_result_with_control(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("materials.mtl", long_mtl.as_bytes()),
        ],
        limits,
        true,
        long_control,
    )?);

    assert_ledger_limit(&short_failure, ImportDimension::Records);
    assert_ledger_limit(&long_failure, ImportDimension::Records);
    let extra_bytes = long_mtl.len() - short_mtl.len();
    assert_eq!(
        work_consumed(&long_failure, ImportDimension::DecodeWork)
            - work_consumed(&short_failure, ImportDimension::DecodeWork),
        (extra_bytes * 2) as u64
    );
    let extra_scan_chunks =
        long_mtl.len().div_ceil(SCAN_CHUNK_BYTES) - short_mtl.len().div_ceil(SCAN_CHUNK_BYTES);
    assert!(
        long_polls.load(Ordering::SeqCst) - short_polls.load(Ordering::SeqCst)
            >= extra_scan_chunks * 2
    );
    Ok(())
}

#[test]
fn repeated_texture_replacement_does_not_accumulate_live_reservations() -> TestResult<()> {
    const REPLACEMENTS: usize = 64;
    const MAP_LINE: &str = "map_Kd textures/diffuse.png\n";
    const TEXTURE_BYTES: &[u8] = b"snapshotted-diffuse-texture";
    let obj = b"mtllib materials.mtl\nusemtl material\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mut repeated = String::from("newmtl material\n");
    repeated.push_str(&MAP_LINE.repeat(REPLACEMENTS));
    let mut padded = String::from("newmtl material\n");
    padded.push_str(&MAP_LINE.repeat(2));
    let padding_bytes = repeated
        .len()
        .checked_sub(padded.len())
        .expect("repeated fixture is longer than its control");
    padded.push('#');
    padded.push_str(&"x".repeat(padding_bytes - 2));
    padded.push('\n');
    assert_eq!(repeated.len(), padded.len());

    let repeated = expected_success(import_assets_result(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("materials.mtl", repeated.as_bytes()),
            ("textures/diffuse.png", TEXTURE_BYTES),
        ],
        ResourceLimits::default(),
        true,
    )?);
    let padded = expected_success(import_assets_result(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("materials.mtl", padded.as_bytes()),
            ("textures/diffuse.png", TEXTURE_BYTES),
        ],
        ResourceLimits::default(),
        true,
    )?);

    assert_eq!(
        repeated.ledger().peak_live_bytes(),
        padded.ledger().peak_live_bytes()
    );
    for report in [&repeated, &padded] {
        let snapshot = scene_snapshot(report)?;
        assert_eq!(snapshot.image_count(), 1);
        assert_eq!(snapshot.sampler_count(), 1);
        assert_eq!(snapshot.texture_count(), 1);
        assert_eq!(snapshot.material_binding_count(), 1);
    }
    Ok(())
}

#[test]
fn owned_string_copy_polls_within_fixed_byte_chunks() -> TestResult<()> {
    let short_name = "x";
    let long_name = "x".repeat(SCAN_CHUNK_BYTES * 8);
    let short_document = format!("o {short_name}\nv 0 0 0\n");
    let long_document = format!("o {long_name}\nv 0 0 0\n");
    let short_state_line_bytes = "o ".len() + short_name.len();
    let long_state_line_bytes = "o ".len() + long_name.len();
    let expected_extra_polls = long_document.len().div_ceil(SCAN_CHUNK_BYTES)
        - short_document.len().div_ceil(SCAN_CHUNK_BYTES)
        + long_state_line_bytes.div_ceil(SCAN_CHUNK_BYTES)
        - short_state_line_bytes.div_ceil(SCAN_CHUNK_BYTES)
        + long_state_line_bytes.div_ceil(SCAN_CHUNK_BYTES)
        - short_state_line_bytes.div_ceil(SCAN_CHUNK_BYTES)
        + long_name.len().div_ceil(SCAN_CHUNK_BYTES)
        - short_name.len().div_ceil(SCAN_CHUNK_BYTES);
    let mut limits = ResourceLimits::default();
    limits.max_records = 1;

    let (short_control, short_polls) = controlled_import(None);
    let short_failure = expected_failure(import_assets_result_with_control(
        "short-name.obj",
        [("short-name.obj", short_document.as_bytes())],
        limits.clone(),
        false,
        short_control,
    )?);
    let (long_control, long_polls) = controlled_import(None);
    let long_failure = expected_failure(import_assets_result_with_control(
        "long-name.obj",
        [("long-name.obj", long_document.as_bytes())],
        limits,
        false,
        long_control,
    )?);

    assert_ledger_limit(&short_failure, ImportDimension::Records);
    assert_ledger_limit(&long_failure, ImportDimension::Records);
    let actual_extra_polls = long_polls.load(Ordering::SeqCst) - short_polls.load(Ordering::SeqCst);
    assert!(
        actual_extra_polls >= expected_extra_polls,
        "long owned-string copy added {actual_extra_polls} polls, expected at least {expected_extra_polls}"
    );
    Ok(())
}

#[test]
fn material_sort_cancellation_is_bounded_by_comparison_chunks() -> TestResult<()> {
    const MATERIAL_COUNT: usize = 512;
    let obj = b"mtllib materials.mtl\nusemtl material-0000\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = reverse_material_library(MATERIAL_COUNT);
    let assets = || {
        [
            ("model.obj", obj.as_slice()),
            ("materials.mtl", mtl.as_bytes()),
        ]
    };

    let (calibration_control, calibration_polls) = controlled_import(None);
    let _report = expected_success(import_assets_result_with_control(
        "model.obj",
        assets(),
        ResourceLimits::default(),
        true,
        calibration_control,
    )?);
    let total_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);

    let (_, failure) =
        first_cancelled_work_prefix_at_least(total_polls, SORT_COMPARISON_CHUNK, |cancel_on| {
            let (control, polls) = controlled_import(Some(cancel_on));
            let failure = expected_failure(import_assets_result_with_control(
                "model.obj",
                assets(),
                ResourceLimits::default(),
                true,
                control,
            )?);
            assert_cancelled(&failure);
            assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
            Ok(failure)
        })?;
    let consumed = work_consumed(&failure, ImportDimension::WorkUnits);
    assert_eq!(consumed, SORT_COMPARISON_CHUNK);
    Ok(())
}

#[test]
fn vertex_key_sort_cancellation_is_bounded_by_comparison_chunks() -> TestResult<()> {
    const VERTEX_COUNT: usize = 512;
    let document = polygon_document(VERTEX_COUNT);

    let (calibration_control, calibration_polls) = controlled_import(None);
    let _report = expected_success(import_assets_result_with_control(
        "polygon.obj",
        [("polygon.obj", document.as_bytes())],
        ResourceLimits::default(),
        false,
        calibration_control,
    )?);
    let total_polls = calibration_polls.load(Ordering::SeqCst);
    assert!(total_polls > 0);

    let expected_prefix = SORT_COMPARISON_CHUNK + 1;
    let (_, failure) =
        first_cancelled_work_prefix_at_least(total_polls, expected_prefix, |cancel_on| {
            let (control, polls) = controlled_import(Some(cancel_on));
            let failure = expected_failure(import_assets_result_with_control(
                "polygon.obj",
                [("polygon.obj", document.as_bytes())],
                ResourceLimits::default(),
                false,
                control,
            )?);
            assert_cancelled(&failure);
            assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
            Ok(failure)
        })?;
    let consumed = work_consumed(&failure, ImportDimension::WorkUnits);
    assert_eq!(consumed, expected_prefix);
    Ok(())
}

#[test]
fn primary_asset_byte_limit_is_enforced() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = 4;
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let failure = expected_failure(import_bytes_result("primary-limit.obj", bytes, limits)?);

    assert_ledger_limit(&failure, ImportDimension::PrimaryBytes);
    Ok(())
}

#[test]
fn line_byte_limit_is_enforced() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_line_bytes = 3;
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let failure = expected_failure(import_bytes_result("line-limit.obj", bytes, limits)?);

    assert_scope_limit(&failure, "line-bytes", 3);
    Ok(())
}

#[test]
fn token_byte_limit_is_enforced() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_token_bytes = 1;
    let bytes = b"v 10 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let failure = expected_failure(import_bytes_result("token-limit.obj", bytes, limits)?);

    assert_scope_limit(&failure, "token-bytes", 1);
    Ok(())
}

#[test]
fn string_byte_limit_is_enforced_for_obj_names() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_string_bytes = 2;
    let bytes = b"g abc\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let failure = expected_failure(import_bytes_result("string-limit.obj", bytes, limits)?);

    assert_scope_limit(&failure, "string-bytes", 2);
    Ok(())
}

#[test]
fn repeating_the_current_state_name_does_not_raise_live_peak() -> TestResult<()> {
    let repeated = b"o reused\no reused\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let padded = b"o reused\n# reused\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    assert_eq!(repeated.len(), padded.len());

    let repeated = expected_success(import_bytes_result(
        "state-repeated.obj",
        repeated,
        ResourceLimits::default(),
    )?);
    let padded = expected_success(import_bytes_result(
        "state-padded.obj",
        padded,
        ResourceLimits::default(),
    )?);

    assert_eq!(
        repeated.ledger().peak_live_bytes(),
        padded.ledger().peak_live_bytes()
    );
    Ok(())
}

#[test]
fn state_name_allocation_respects_the_exact_live_limit() -> TestResult<()> {
    let document = format!("o {}\n", "x".repeat(4_096));
    let baseline = expected_failure(import_bytes_result(
        "state-live-baseline.obj",
        document.as_bytes(),
        ResourceLimits::default(),
    )?);
    let exact_peak = baseline.ledger().peak_live_bytes();
    assert!(exact_peak > document.len() as u64);
    assert_eq!(baseline.ledger().current_live_bytes(), 0);

    let limit = exact_peak - 1;
    let mut limits = ResourceLimits::default();
    limits.max_live_bytes = limit;
    let failure = expected_failure(import_bytes_result(
        "state-live-excess.obj",
        document.as_bytes(),
        limits,
    )?);

    assert_live_limit(&failure, limit);
    Ok(())
}

#[test]
fn generated_vertex_limit_is_aggregate_across_segments() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\ng left\nf 1 2 3\ng right\nf 1 2 3\n";

    let mut exact = ResourceLimits::default();
    exact.max_vertices = 9;
    let report = expected_success(import_bytes_result("vertices-exact.obj", bytes, exact)?);
    assert!(report.diagnostics().is_empty());
    let snapshot = scene_snapshot(&report)?;
    assert_eq!(snapshot.logical_mesh_count(), 2);
    assert_eq!(snapshot.f32_scalar_count(), 18);

    let mut excess = ResourceLimits::default();
    excess.max_vertices = 8;
    let failure = expected_failure(import_bytes_result("vertices-excess.obj", bytes, excess)?);
    assert_ledger_limit(&failure, ImportDimension::Vertices);
    assert!(failure.diagnostics().is_empty());
    Ok(())
}

#[test]
fn face_limit_is_enforced() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_faces = 0;
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let failure = expected_failure(import_bytes_result("face-limit.obj", bytes, limits)?);

    assert_ledger_limit(&failure, ImportDimension::Faces);
    Ok(())
}

#[test]
fn topology_cell_limit_is_enforced_before_face_storage() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let mut exact = ResourceLimits::default();
    exact.max_total_topology_cells = 6;
    let report = expected_success(import_bytes_result("topology-exact.obj", bytes, exact)?);
    assert!(report.diagnostics().is_empty());
    assert_eq!(scene_snapshot(&report)?.primitive_index_count(), 3);

    let mut excess = ResourceLimits::default();
    excess.max_total_topology_cells = 5;
    let failure = expected_failure(import_bytes_result("topology-excess.obj", bytes, excess)?);
    assert_ledger_limit(&failure, ImportDimension::TopologyCells);
    Ok(())
}

#[test]
fn topology_cell_limit_includes_polygon_face_counts_at_the_exact_boundary() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3\nf 1 2 3 4\n";

    let mut exact = ResourceLimits::default();
    exact.max_total_topology_cells = 16;
    let report = expected_success(import_bytes_result(
        "polygon-topology-exact.obj",
        bytes,
        exact,
    )?);
    assert!(report.diagnostics().is_empty());
    let snapshot = scene_snapshot(&report)?;
    assert_eq!(snapshot.primitive_index_count(), 7);
    assert_eq!(snapshot.polygon_boundary_count(), 2);
    assert!(
        snapshot
            .preview()
            .contains("polygon_boundaries(present,total=2)=[3,4]")
    );

    let mut excess = ResourceLimits::default();
    excess.max_total_topology_cells = 15;
    let failure = expected_failure(import_bytes_result(
        "polygon-topology-excess.obj",
        bytes,
        excess,
    )?);
    assert_ledger_limit(&failure, ImportDimension::TopologyCells);
    Ok(())
}

#[test]
fn canonical_attribute_cells_are_charged_at_the_exact_boundary() -> TestResult<()> {
    let bytes =
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\n";

    let mut exact = ResourceLimits::default();
    exact.max_total_vertex_attribute_cells = 10;
    let report = expected_success(import_bytes_result("attribute-exact.obj", bytes, exact)?);
    assert!(report.diagnostics().is_empty());
    let snapshot = scene_snapshot(&report)?;
    assert_eq!(snapshot.attribute_count(), 3);
    assert!(snapshot_line_containing(&snapshot, "semantic=texcoord").contains("elements=3"));
    assert!(snapshot_line_containing(&snapshot, "semantic=normal").contains("elements=3"));

    let mut excess = ResourceLimits::default();
    excess.max_total_vertex_attribute_cells = 9;
    let failure = expected_failure(import_bytes_result("attribute-excess.obj", bytes, excess)?);
    assert_ledger_limit(&failure, ImportDimension::AttributeCells);
    Ok(())
}

#[test]
fn mesh_limit_is_enforced_after_group_split() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_meshes = 1;
    let bytes = b"g left\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\ng right\nv 2 0 0\nv 3 0 0\nv 2 1 0\nf 4 5 6\n";

    let failure = expected_failure(import_bytes_result("mesh-limit.obj", bytes, limits)?);

    assert_ledger_limit(&failure, ImportDimension::Meshes);
    Ok(())
}

#[test]
fn canonical_string_budget_is_charged_before_segment_materialization() -> TestResult<()> {
    const NAME_BYTES: usize = 16;
    const SEGMENT_COUNT: usize = 8;
    const SOURCE: &str = "canonical-strings.obj";

    let object = "x".repeat(NAME_BYTES);
    let mut document = format!("o {object}\nv 0 0 0\nv 1 0 0\nv 0 1 0\n");
    for segment in 0..SEGMENT_COUNT {
        let smoothing = if segment.is_multiple_of(2) { "a" } else { "b" };
        document.push_str(&format!("s {smoothing}\nf 1 2 3\n"));
    }
    let parsed_name_bytes = NAME_BYTES + SEGMENT_COUNT;
    let segment_bytes = 3 * NAME_BYTES + "obj:object".len() + "obj:smoothing".len() + "a".len();
    let exact_bytes = parsed_name_bytes
        + SEGMENT_COUNT * segment_bytes
        + SOURCE.len()
        + baozi_format_obj::FORMAT_ID.len();

    let mut exact = ResourceLimits::default();
    exact.max_string_bytes = NAME_BYTES;
    exact.max_canonical_string_bytes = ResourceLimits::default().max_canonical_string_bytes;
    let report = expected_success(import_bytes_result(SOURCE, document.as_bytes(), exact)?);
    let canonical_strings = report
        .ledger()
        .dimension(ImportDimension::CanonicalStringBytes)
        .expect("canonical string evidence");
    assert_eq!(canonical_strings.attempted(), exact_bytes as u64);
    assert_eq!(canonical_strings.consumed(), exact_bytes as u64);
    assert_eq!(scene_snapshot(&report)?.logical_mesh_count(), SEGMENT_COUNT);

    let mut excess = ResourceLimits::default();
    excess.max_string_bytes = NAME_BYTES;
    excess.max_canonical_string_bytes = exact_bytes - 1;
    let failure = expected_failure(import_bytes_result(SOURCE, document.as_bytes(), excess)?);
    assert_ledger_limit(&failure, ImportDimension::CanonicalStringBytes);
    Ok(())
}

#[test]
fn live_byte_limit_is_exact_across_normalization_and_scene_handoff() -> TestResult<()> {
    let bytes = b"o bounded\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\n";

    let baseline = expected_success(import_bytes_result(
        "live-baseline.obj",
        bytes,
        ResourceLimits::default(),
    )?);
    let exact_peak = baseline.ledger().peak_live_bytes();
    assert!(exact_peak > bytes.len() as u64);
    assert_eq!(baseline.ledger().current_live_bytes(), 0);

    let mut exact = ResourceLimits::default();
    exact.max_live_bytes = exact_peak;
    let exact_report = expected_success(import_bytes_result("live-exact.obj", bytes, exact)?);
    assert_eq!(exact_report.ledger().peak_live_bytes(), exact_peak);

    let excess_limit = exact_peak - 1;
    let mut excess = ResourceLimits::default();
    excess.max_live_bytes = excess_limit;
    let failure = expected_failure(import_bytes_result("live-excess.obj", bytes, excess)?);
    assert_live_limit(&failure, excess_limit);
    Ok(())
}

#[test]
fn diagnostic_limit_caps_unsupported_statement_warnings() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_diagnostics = 1;
    let bytes =
        b"curv 0 1 1 2\nsurf 0 1 0 1 1 2 3 4\nparm u 0 1\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let report = expected_success(import_bytes_result("diagnostic-limit.obj", bytes, limits)?);

    assert_eq!(scene_snapshot(&report)?.logical_mesh_count(), 1);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "obj.unsupported_statement"
    );
    assert_eq!(report.diagnostic_summary().attempted_count(), 3);
    assert_eq!(report.diagnostic_summary().dropped_count(), 2);
    Ok(())
}

#[test]
fn texture_path_string_limit_warns_and_skips_texture() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_string_bytes = 16;
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl red\nmap_Kd textures/very-long-texture-name.png\n";

    let report = expected_success(import_assets_result(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        limits,
        true,
    )?);

    assert_eq!(scene_snapshot(&report)?.texture_count(), 0);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "obj.mtl_texture_path_limit"
    );
    Ok(())
}

#[test]
fn texture_path_larger_than_one_chunk_is_normalized_by_the_dependency_host() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_path_length = SCAN_CHUNK_BYTES * 2;
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let texture_path = format!("textures/{}.png", "x".repeat(SCAN_CHUNK_BYTES));
    let mtl = format!("newmtl red\nmap_Kd {texture_path}\n");

    let report = expected_success(import_assets_result(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_bytes()),
        ],
        limits,
        true,
    )?);

    assert_eq!(scene_snapshot(&report)?.texture_count(), 0);
    assert_eq!(report.diagnostics().len(), 2);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "import.optional_dependency_unavailable"
    );
    assert_eq!(
        report.diagnostics()[1].code().as_str(),
        "obj.mtl_texture_unavailable"
    );
    Ok(())
}
