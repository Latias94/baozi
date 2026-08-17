#![allow(dead_code)]

use baozi_format_ply::{PlyFailureCode, PlyImporter};
use baozi_import::{
    CauseOwner, DetectionOptions, FormatFailureDetailValue, FormatFailureEnvelope, FormatSelection,
    ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure, ImportOperationControl,
    ImportReport, ImportRequest, ImportRequestLayer, ImportResult, ImportRunError,
    ImportTerminalCause, ImportTerminalFailureClass, OptionCompileLimits, RegistryBuildLimits,
    RegistryBuilder, ResourceLimits, ResourceLimitsOverlay, SceneSnapshot, SceneSnapshotLimits,
    ScopeAbortCause, import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

pub type TestResult<T> = Result<T, TestError>;

#[derive(Debug)]
pub struct TestError(Box<str>);

impl TestError {
    fn setup(stage: &'static str, error: impl fmt::Display) -> Self {
        Self(format!("{stage}: {error}").into_boxed_str())
    }

    fn unexpected_failure(failure: ImportRunError) -> Self {
        Self(format!("unexpected import failure: {failure:?}").into_boxed_str())
    }
}

impl fmt::Display for TestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TestError {}

pub fn import_bytes(source: &str, bytes: &[u8]) -> TestResult<ImportReport> {
    run_import(
        source,
        bytes,
        ResourceLimits::default(),
        DetectionOptions::default(),
    )
    .and_then(|result| result.map_err(TestError::unexpected_failure))
}

pub fn import_bytes_result(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
) -> TestResult<ImportResult<ImportReport>> {
    run_import(source, bytes, limits, DetectionOptions::default())
}

pub fn import_bytes_forced_result(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
) -> TestResult<ImportResult<ImportReport>> {
    import_bytes_forced_result_with_control(
        source,
        bytes,
        limits,
        ImportOperationControl::default(),
    )
}

pub fn import_bytes_forced_result_with_control(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
    control: ImportOperationControl,
) -> TestResult<ImportResult<ImportReport>> {
    let format = baozi_core::FormatId::new("org.baozi.ply").expect("static PLY format id is valid");
    run_import_with_selection_and_control(
        source,
        bytes,
        limits,
        DetectionOptions::default(),
        Some(FormatSelection::Forced(format)),
        control,
    )
}

pub fn import_bytes_with_detection(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
    detection: DetectionOptions,
) -> TestResult<ImportResult<ImportReport>> {
    run_import(source, bytes, limits, detection)
}

fn run_import(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
    detection: DetectionOptions,
) -> TestResult<ImportResult<ImportReport>> {
    run_import_with_selection(source, bytes, limits, detection, None)
}

fn run_import_with_selection(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
    detection: DetectionOptions,
    format_selection: Option<FormatSelection>,
) -> TestResult<ImportResult<ImportReport>> {
    run_import_with_selection_and_control(
        source,
        bytes,
        limits,
        detection,
        format_selection,
        ImportOperationControl::default(),
    )
}

fn run_import_with_selection_and_control(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
    detection: DetectionOptions,
    format_selection: Option<FormatSelection>,
    control: ImportOperationControl,
) -> TestResult<ImportResult<ImportReport>> {
    let path = AssetPath::new(source).map_err(|error| TestError::setup("source path", error))?;
    let io = MemoryAssetIo::new();
    io.try_insert(path.clone(), Arc::<[u8]>::from(bytes))
        .map_err(|error| TestError::setup("memory asset insertion", error))?;

    let mut registry = RegistryBuilder::new();
    registry
        .register(PlyImporter)
        .map_err(|error| TestError::setup("PLY registration", error))?;
    let snapshot = registry
        .freeze(RegistryBuildLimits::default())
        .map_err(|error| TestError::setup("registry freeze", error))?;
    let mut config = ImportConfigOverlay::default()
        .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
        .with_detection_options(detection);
    if let Some(selection) = format_selection {
        config = config.with_format_selection(selection);
    }
    let request = snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(config),
            ),
            OptionCompileLimits::default(),
        )
        .map_err(|error| TestError::setup("request compilation", error))?;

    Ok(import_asset(&request, &io, path, control))
}

pub fn expected_error(result: ImportResult<ImportReport>) -> TestResult<ImportFailure> {
    match result {
        Ok(report) => Err(TestError(
            format!(
                "expected PLY import to fail, but {} published generation {:?}",
                report.source(),
                report.scene().generation()
            )
            .into_boxed_str(),
        )),
        Err(error) => operation_failure(error),
    }
}

pub fn operation_failure(error: ImportRunError) -> TestResult<ImportFailure> {
    error
        .into_operation_failure()
        .map_err(|source| TestError::setup("import initialization", source))
}

pub fn expected_report(result: ImportResult<ImportReport>) -> TestResult<ImportReport> {
    result.map_err(TestError::unexpected_failure)
}

pub fn scene_snapshot(report: &ImportReport) -> TestResult<SceneSnapshot> {
    let limits = SceneSnapshotLimits::try_new(64 * 1024, 1 << 12, 4_000_000)
        .map_err(|error| TestError::setup("scene snapshot limits", error))?;
    let snapshot = report
        .scene()
        .capture_snapshot(limits)
        .map_err(|error| TestError::setup("scene snapshot capture", error))?;
    assert!(!snapshot.preview_bytes_truncated());
    assert!(!snapshot.preview_items_truncated());
    Ok(snapshot)
}

pub fn snapshot_line<'snapshot>(
    snapshot: &'snapshot SceneSnapshot,
    prefix: &str,
) -> &'snapshot str {
    snapshot
        .preview()
        .lines()
        .find(|line| line.trim_start().starts_with(prefix))
        .unwrap_or_else(|| panic!("missing canonical snapshot line: {prefix}"))
}

pub fn snapshot_line_containing<'snapshot>(
    snapshot: &'snapshot SceneSnapshot,
    fragment: &str,
) -> &'snapshot str {
    snapshot
        .preview()
        .lines()
        .find(|line| line.contains(fragment))
        .unwrap_or_else(|| panic!("missing canonical snapshot fragment: {fragment}"))
}

pub fn assert_requested_terminal_failure(failure: &ImportFailure) {
    let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
        panic!("failed import published a successful ledger terminal cause")
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Requested);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

pub fn expected_ply_failure(
    failure: &ImportFailure,
    expected_code: PlyFailureCode,
) -> &FormatFailureEnvelope {
    let expected_format =
        baozi_core::FormatId::new("org.baozi.ply").expect("static PLY format id is valid");
    assert_eq!(
        failure.cause().owner(),
        CauseOwner::Format(expected_format.clone())
    );
    let ImportCause::Format(envelope) = failure.cause() else {
        panic!("expected PLY format envelope, got {:?}", failure.cause());
    };
    assert_eq!(envelope.format(), &expected_format);
    assert_eq!(envelope.code(), expected_code.as_str());
    assert_ply_detail_schema(envelope);
    assert!(
        ply_string_detail(envelope, "reason").is_some_and(|reason| !reason.is_empty()),
        "PLY format envelope must contain a non-empty string reason"
    );
    envelope
}

pub fn ply_string_detail<'a>(envelope: &'a FormatFailureEnvelope, key: &str) -> Option<&'a str> {
    let detail = envelope
        .details()
        .iter()
        .find(|detail| detail.key() == key)?;
    let FormatFailureDetailValue::String(value) = detail.value() else {
        panic!("PLY detail {key} is not a string: {:?}", detail.value());
    };
    Some(value.as_str())
}

pub fn ply_unsigned_detail(envelope: &FormatFailureEnvelope, key: &str) -> Option<u64> {
    let detail = envelope
        .details()
        .iter()
        .find(|detail| detail.key() == key)?;
    let FormatFailureDetailValue::Unsigned(value) = detail.value() else {
        panic!("PLY detail {key} is not unsigned: {:?}", detail.value());
    };
    Some(value)
}

fn assert_ply_detail_schema(envelope: &FormatFailureEnvelope) {
    let details = envelope.details();
    for (index, detail) in details.iter().enumerate() {
        assert!(
            !details.as_slice()[..index]
                .iter()
                .any(|prior| prior.key() == detail.key()),
            "PLY format envelope contains duplicate detail key {}",
            detail.key()
        );
        match detail.key() {
            "reason" | "context" => assert!(
                matches!(detail.value(), FormatFailureDetailValue::String(_)),
                "PLY text detail {} must be a string",
                detail.key()
            ),
            "byte_offset" => assert!(
                matches!(detail.value(), FormatFailureDetailValue::Unsigned(_)),
                "PLY numeric detail {} must be unsigned",
                detail.key()
            ),
            key => panic!("unexpected PLY format detail key {key}"),
        }
    }
}

pub fn assert_resource_limit(failure: &ImportFailure, expected_resource: &'static str) {
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!(
            "expected scope-owned resource failure, got {:?}",
            failure.cause()
        );
    };
    assert_eq!(abort.owner(), CauseOwner::Import);
    let expected_dimension = match expected_resource {
        "max_vertices" => ImportDimension::Vertices,
        "max_primitives" => ImportDimension::Primitives,
        "max_records" => ImportDimension::Records,
        "max_total_topology_cells" => ImportDimension::TopologyCells,
        "max_vertex_attribute_streams_per_mesh" => ImportDimension::AttributeStreams,
        "max_total_vertex_attribute_cells" => ImportDimension::AttributeCells,
        other => panic!("test helper has no dimension mapping for {other}"),
    };
    match abort.cause() {
        ScopeAbortCause::ResourceLimit { resource, .. } => {
            assert_eq!(*resource, expected_resource);
        }
        ScopeAbortCause::Ledger(baozi_import::LedgerCause::LimitExceeded { dimension, .. }) => {
            assert_eq!(*dimension, expected_dimension);
        }
        cause => panic!("expected scope-owned resource failure, got {cause:?}"),
    }
}

pub fn assert_scope_resource_limit(
    failure: &ImportFailure,
    expected_resource: &'static str,
    expected_limit: u64,
) {
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!(
            "expected scope-owned resource failure, got {:?}",
            failure.cause()
        );
    };
    assert_eq!(abort.owner(), CauseOwner::Import);
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::ResourceLimit { resource, limit }
            if *resource == expected_resource && *limit == expected_limit
    ));
}

pub fn ascii_triangle() -> &'static [u8] {
    b"ply
format ascii 1.0
comment fixture triangle
obj_info generated by baozi tests
element vertex 3
property float x
property float y
property float z
property float nx
property float ny
property float nz
property float s
property float t
property uchar red
property uchar green
property uchar blue
property float temperature
element face 1
property list uchar uint vertex_indices
end_header
0 0 0 0 0 1 0 0 255 0 0 0.5
1 0 0 0 0 1 1 0 0 255 0 0.75
0 1 0 0 0 1 0 1 0 0 255 1.0
3 0 1 2
"
}

pub fn ascii_point_cloud() -> &'static [u8] {
    b"ply
format ascii 1.0
element vertex 2
property float x
property float y
property float z
end_header
0 0 0
1 2 3
"
}

pub fn ascii_quad() -> &'static [u8] {
    b"ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
end_header
0 0 0
1 0 0
1 1 0
0 1 0
4 0 1 2 3
"
}

pub fn binary_triangle(little_endian: bool) -> Vec<u8> {
    let format = if little_endian {
        "binary_little_endian"
    } else {
        "binary_big_endian"
    };
    let mut bytes = format!(
        "ply\nformat {format} 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n"
    )
    .into_bytes();
    for position in [[0.0f32, 0.0, 0.0], [1.0f32, 0.0, 0.0], [0.0f32, 1.0, 0.0]] {
        for value in position {
            if little_endian {
                bytes.extend_from_slice(&value.to_le_bytes());
            } else {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    bytes.push(3);
    for index in [0u32, 1, 2] {
        if little_endian {
            bytes.extend_from_slice(&index.to_le_bytes());
        } else {
            bytes.extend_from_slice(&index.to_be_bytes());
        }
    }
    bytes
}
