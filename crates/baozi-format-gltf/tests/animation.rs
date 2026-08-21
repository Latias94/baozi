mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportProgressPhase,
    ImportProgressUnit, ImportReport, ImportResult, SceneSnapshot, SceneSnapshotLimits,
    ScopeAbortCause,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, ProgressDecision, ProgressEvent,
};
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, assert_gltf_failure, assert_resource_limit,
    expected_error, glb_with_json, import_assets, import_assets_result,
    import_assets_result_with_control,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const ANIMATION_CELLS: u64 = 38;

#[derive(Clone)]
struct AnimationPollCancellation {
    armed: Arc<AtomicBool>,
    polls: Arc<AtomicUsize>,
    cancel_on: Option<usize>,
}

impl CancellationSignal for AnimationPollCancellation {
    fn is_cancelled(&self) -> bool {
        if !self.armed.load(Ordering::SeqCst) {
            return false;
        }
        let poll = self.polls.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel_on.is_some_and(|cancel_on| poll >= cancel_on)
    }
}

fn controlled_animation_result(
    document: &[u8],
    cancel_on: Option<usize>,
) -> TestResult<(ImportResult<ImportReport>, usize)> {
    let armed = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let signal = AnimationPollCancellation {
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
    let (result, diagnostics) = import_assets_result_with_control(
        "animation.glb",
        [("animation.glb", document.to_vec())],
        GltfTestOptions::memory(),
        control,
    )?;
    assert!(diagnostics.is_empty());
    Ok((result, polls.load(Ordering::SeqCst)))
}

fn assert_signal_cancelled(failure: &ImportFailure) {
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

#[test]
fn imports_node_trs_animation_with_source_order_and_exact_payloads() -> TestResult<()> {
    let document = animation_glb(base_animation_json(), &base_animation_bin());
    let (report, diagnostics) = import_assets(
        "animation.glb",
        [("animation.glb", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    let animation_cells = report
        .ledger()
        .dimension(ImportDimension::AnimationCells)
        .expect("animation cells are configured");
    assert_eq!(animation_cells.attempted(), ANIMATION_CELLS);
    assert_eq!(animation_cells.consumed(), ANIMATION_CELLS);

    let snapshot = scene_snapshot(&report);
    let preview = snapshot.preview();
    assert!(preview.contains("animation(present)"));
    assert!(snapshot_line(&snapshot, "clip[0]").contains(concat!(
        "duration_bits=4607182418800017408 time_domain=0 ",
        "channel_offset=0 channel_count=3"
    )));
    let channel_0 = snapshot_line(&snapshot, "channel[0]");
    assert!(
        channel_0.contains(concat!(
            "target_kind=0 target=1 interpolation=1 key_count=2 value_width=3 ",
            "times(present,total=2)=[ first_bits=0] ",
            "values(present,total=6)=[0x00000000,0x00000000,0x00000000,",
            "0x3f800000,0x40000000,0x40400000] ",
            "in_tangents(absent,total=0)=[] out_tangents(absent,total=0)=[]"
        )),
        "unexpected channel[0] snapshot: {channel_0}"
    );
    assert!(snapshot_line(&snapshot, "channel[1]").contains(concat!(
        "target_kind=1 target=1 interpolation=0 key_count=2 value_width=4 ",
        "times(present,total=2)=[ first_bits=0] ",
        "values(present,total=8)=[0x00000000,0x00000000,0x00000000,",
        "0x3f800000,0x00000000,0x00000000,0x3f800000,0x00000000]"
    )));
    assert!(snapshot_line(&snapshot, "channel[2]").contains(concat!(
        "target_kind=2 target=1 interpolation=2 key_count=2 value_width=3 ",
        "times(present,total=2)=[ first_bits=0] ",
        "values(present,total=6)=[0x3f800000,0x3f800000,0x3f800000,",
        "0x40000000,0x40400000,0x40800000] ",
        "in_tangents(present,total=6)=[0x00000000,0x00000000,0x00000000,",
        "0x3e800000,0x3e800000,0x3e800000] ",
        "out_tangents(present,total=6)=[0x3f000000,0x3f000000,0x3f000000,",
        "0x00000000,0x00000000,0x00000000]"
    )));
    Ok(())
}

#[test]
fn duplicate_node_animation_target_fails_before_payload_admission() -> TestResult<()> {
    let json = replace_once(base_animation_json(), r#""output": 2"#, r#""output": 1"#);
    let json = replace_once(&json, r#""path": "rotation""#, r#""path": "translation""#);
    let document = animation_glb(json.as_bytes(), &base_animation_bin());
    let (result, diagnostics) = import_assets_result(
        "duplicate-target.glb",
        [("duplicate-target.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &failure,
        GltfFailureCode::InvalidDocument,
        Some("duplicate animation target"),
    )?;
    let animation_cells = failure
        .ledger()
        .dimension(ImportDimension::AnimationCells)
        .expect("animation cells are configured");
    assert_eq!(animation_cells.attempted(), 0);
    assert_eq!(animation_cells.consumed(), 0);
    Ok(())
}

#[test]
fn matrix_authored_node_animation_is_rejected_before_payload_admission() -> TestResult<()> {
    let json = replace_once(
        base_animation_json(),
        r#"{ "name": "AnimatedNode" }"#,
        r#"{ "name": "AnimatedNode", "matrix": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] }"#,
    );
    let document = animation_glb(json.as_bytes(), &base_animation_bin());
    let (result, diagnostics) = import_assets_result(
        "matrix-target.glb",
        [("matrix-target.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &failure,
        GltfFailureCode::InvalidDocument,
        Some("matrix-authored node"),
    )?;
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .expect("animation cells are configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn animation_output_count_is_rejected_before_payload_admission() -> TestResult<()> {
    let json = replace_once(
        base_animation_json(),
        r#"{ "bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3" }"#,
        r#"{ "bufferView": 1, "componentType": 5126, "count": 1, "type": "VEC3" }"#,
    );
    let document = animation_glb(json.as_bytes(), &base_animation_bin());
    let (result, diagnostics) = import_assets_result(
        "count-mismatch.glb",
        [("count-mismatch.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &failure,
        GltfFailureCode::InvalidDocument,
        Some("output accessor count"),
    )?;
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .expect("animation cells are configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn non_finite_animation_payload_is_rejected_without_committing_cells() -> TestResult<()> {
    let mut bin = base_animation_bin();
    bin[20..24].copy_from_slice(&f32::NAN.to_le_bytes());
    let document = animation_glb(base_animation_json(), &bin);
    let (result, diagnostics) = import_assets_result(
        "non-finite.glb",
        [("non-finite.glb", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &failure,
        GltfFailureCode::InvalidDocument,
        Some("non-finite animation value"),
    )?;
    let animation_cells = failure
        .ledger()
        .dimension(ImportDimension::AnimationCells)
        .expect("animation cells are configured");
    assert_eq!(animation_cells.attempted(), ANIMATION_CELLS);
    assert!(animation_cells.consumed() < ANIMATION_CELLS);
    Ok(())
}

#[test]
fn invalid_animation_times_fail_during_bounded_materialization() -> TestResult<()> {
    for (first, second, reason, consumed) in [
        (-1.0_f32, 1.0_f32, "negative animation time", 0_u64),
        (f32::NAN, 1.0_f32, "non-finite animation time", 0_u64),
        (0.0_f32, 0.0_f32, "strictly increasing", 1_u64),
    ] {
        let mut bin = base_animation_bin();
        write_f32(&mut bin, 0, first);
        write_f32(&mut bin, 4, second);
        let document = animation_glb(base_animation_json(), &bin);
        let (result, diagnostics) = import_assets_result(
            "invalid-animation-time.glb",
            [("invalid-animation-time.glb", document)],
            GltfTestOptions::memory(),
        )?;
        let failure = expected_error(result)?;

        assert!(diagnostics.is_empty());
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, Some(reason))?;
        let animation_cells = failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .expect("animation cells are configured");
        assert_eq!(animation_cells.attempted(), ANIMATION_CELLS);
        assert_eq!(animation_cells.consumed(), consumed);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}

#[test]
fn animation_cell_budget_is_admitted_before_accessor_reading() -> TestResult<()> {
    let document = animation_glb(base_animation_json(), &base_animation_bin());
    let mut options = GltfTestOptions::memory();
    options.limits.max_animation_cells = usize::try_from(ANIMATION_CELLS - 1)?;
    let (result, diagnostics) = import_assets_result(
        "animation-limit.glb",
        [("animation-limit.glb", document)],
        options,
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_resource_limit(&failure, "max_animation_cells");
    let animation_cells = failure
        .ledger()
        .dimension(ImportDimension::AnimationCells)
        .expect("animation cells are configured");
    assert_eq!(animation_cells.attempted(), ANIMATION_CELLS);
    assert_eq!(animation_cells.consumed(), 0);
    Ok(())
}

#[test]
fn animation_cancellation_preserves_cell_prefix_and_releases_storage() -> TestResult<()> {
    let document = animation_glb(base_animation_json(), &base_animation_bin());
    let (calibration, total_polls) = controlled_animation_result(&document, None)?;
    let _report = calibration?;
    assert!(total_polls > 0);

    let mut cancelled = None;
    for cancel_on in 1..=total_polls {
        let (result, polls) = controlled_animation_result(&document, Some(cancel_on))?;
        let failure = match result {
            Ok(_) => continue,
            Err(error) => error.into_operation_failure()?,
        };
        assert_eq!(polls, cancel_on);
        assert_signal_cancelled(&failure);
        let animation_cells = failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .expect("animation cells are configured");
        if animation_cells.attempted() == ANIMATION_CELLS
            && animation_cells.consumed() > 0
            && animation_cells.consumed() < ANIMATION_CELLS
        {
            cancelled = Some(failure);
            break;
        }
    }

    let failure =
        cancelled.expect("a calibrated poll must cancel during animation materialization");
    let animation_cells = failure
        .ledger()
        .dimension(ImportDimension::AnimationCells)
        .expect("animation cells are configured");
    assert_eq!(animation_cells.attempted(), ANIMATION_CELLS);
    assert!(animation_cells.consumed() < ANIMATION_CELLS);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    Ok(())
}

#[test]
fn morph_weight_animation_remains_an_explicit_controlled_diagnostic() -> TestResult<()> {
    let bin = [0.0_f32, 1.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let document = animation_glb(morph_animation_json(), &bin);
    let (report, diagnostics) = import_assets(
        "morph-animation.glb",
        [("morph-animation.glb", document)],
        GltfTestOptions::memory(),
    )?;

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code().as_str(),
        "gltf.animation_morph_weights_ignored"
    );
    let snapshot = scene_snapshot(&report);
    assert!(snapshot.preview().contains("animation(present)"));
    assert!(snapshot.preview().contains("clips(present,total=0)"));
    assert!(snapshot.preview().contains("channels(present,total=0)"));
    Ok(())
}

fn scene_snapshot(report: &ImportReport) -> SceneSnapshot {
    let limits = SceneSnapshotLimits::try_new(64 * 1024, 4_096, 4_000_000)
        .expect("animation snapshot limits are valid");
    let snapshot = report
        .scene()
        .capture_snapshot(limits)
        .expect("validated animation snapshot succeeds");
    assert!(!snapshot.preview_bytes_truncated());
    assert!(!snapshot.preview_items_truncated());
    snapshot
}

fn snapshot_line<'snapshot>(snapshot: &'snapshot SceneSnapshot, prefix: &str) -> &'snapshot str {
    snapshot
        .preview()
        .lines()
        .find(|line| line.trim_start().starts_with(prefix))
        .unwrap_or_else(|| panic!("missing canonical snapshot line: {prefix}"))
}

fn replace_once(source: impl AsRef<[u8]>, from: &str, to: &str) -> String {
    let source = std::str::from_utf8(source.as_ref()).expect("fixture JSON is valid UTF-8");
    assert_eq!(
        source.matches(from).count(),
        1,
        "fixture replacement is unique"
    );
    source.replacen(from, to, 1)
}

fn animation_glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    append_glb_chunk(glb_with_json(json), b"BIN\0", bin)
}

fn base_animation_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    extend_f32(&mut bytes, &[0.0, 1.0]);
    extend_f32(&mut bytes, &[0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    extend_f32(&mut bytes, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
    extend_f32(
        &mut bytes,
        &[
            0.0, 0.0, 0.0, // key 0 in tangent
            1.0, 1.0, 1.0, // key 0 value
            0.5, 0.5, 0.5, // key 0 out tangent
            0.25, 0.25, 0.25, // key 1 in tangent
            2.0, 3.0, 4.0, // key 1 value
            0.0, 0.0, 0.0, // key 1 out tangent
        ],
    );
    assert_eq!(bytes.len(), 136);
    bytes
}

fn extend_f32(bytes: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    let end = offset.checked_add(4).expect("test offset must fit");
    bytes[offset..end].copy_from_slice(&value.to_le_bytes());
}

fn base_animation_json() -> &'static [u8] {
    br#"{
  "asset": { "version": "2.0", "generator": "baozi-animation-test" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [{ "name": "AnimatedNode" }],
  "buffers": [{ "byteLength": 136 }],
  "bufferViews": [
    { "buffer": 0, "byteOffset": 0, "byteLength": 8 },
    { "buffer": 0, "byteOffset": 8, "byteLength": 24 },
    { "buffer": 0, "byteOffset": 32, "byteLength": 32 },
    { "buffer": 0, "byteOffset": 64, "byteLength": 72 }
  ],
  "accessors": [
    { "bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] },
    { "bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3" },
    { "bufferView": 2, "componentType": 5126, "count": 2, "type": "VEC4" },
    { "bufferView": 3, "componentType": 5126, "count": 6, "type": "VEC3" }
  ],
  "animations": [{
    "name": "TRS",
    "samplers": [
      { "input": 0, "output": 1, "interpolation": "LINEAR" },
      { "input": 0, "output": 2, "interpolation": "STEP" },
      { "input": 0, "output": 3, "interpolation": "CUBICSPLINE" }
    ],
    "channels": [
      { "sampler": 0, "target": { "node": 0, "path": "translation" } },
      { "sampler": 1, "target": { "node": 0, "path": "rotation" } },
      { "sampler": 2, "target": { "node": 0, "path": "scale" } }
    ]
  }]
}"#
}

fn morph_animation_json() -> &'static [u8] {
    br#"{
  "asset": { "version": "2.0", "generator": "baozi-morph-animation-test" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [{ "name": "AnimatedNode" }],
  "buffers": [{ "byteLength": 8 }],
  "bufferViews": [{ "buffer": 0, "byteOffset": 0, "byteLength": 8 }],
  "accessors": [
    { "bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] }
  ],
  "animations": [{
    "samplers": [{ "input": 0, "output": 0, "interpolation": "LINEAR" }],
    "channels": [
      { "sampler": 0, "target": { "node": 0, "path": "weights" } }
    ]
  }]
}"#
}
