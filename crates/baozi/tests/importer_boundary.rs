mod support;

use baozi::{
    AssetIo, AssetLength, AssetOpenError, AssetPath, AuthorityDomain, CapabilityStatus, Error,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatId, ImportCause, ImportDimension,
    ImportOperationControl, ImportTerminalCause, ImportTerminalFailureClass, Importer,
    ReadConfidence, RequestError, SceneSnapshotLimits,
};
use baozi_import::{
    DenyDependencies, DetectionOutcome, DetectionResult, DetectionScope, FormatImporter,
    ParseResult, ParseScope, Probe, ReadHint, SceneDraft,
};
use baozi_io::OpenedAsset;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

const ONCE_ID: &str = "org.example.open-once";
const INVALID_ID: &str = "org.example.invalid-draft";
const GEOMETRY_ID: &str = "org.example.geometry";

fn descriptor(id: &str, extension: &str) -> FormatDescriptor {
    FormatDescriptor::builder(
        FormatId::new(id).expect("qualified test format id"),
        "Facade boundary fixture",
    )
    .extensions([extension])
    .encoding(FormatEncoding::Binary)
    .capabilities([(FormatCapability::Geometry, CapabilityStatus::Unknown)])
    .build()
    .expect("valid test descriptor")
}

struct OpenOnceImporter;

impl FormatImporter for OpenOnceImporter {
    fn descriptor(&self) -> FormatDescriptor {
        descriptor(ONCE_ID, "once")
    }

    fn detect(
        &self,
        probe: &Probe<'_>,
        _hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        let detect = || {
            Ok(if probe.bytes() == b"stable primary" {
                DetectionOutcome::Match(ReadConfidence::Certain)
            } else {
                DetectionOutcome::NoMatch
            })
        };
        if probe.bytes().is_empty() {
            detect()
        } else {
            scope.with_work(probe.bytes().len() as u64, detect)
        }
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        assert_eq!(scope.primary_bytes(), b"stable primary");
        support::submit_minimal_scene(scope)
    }
}

struct InvalidDraftImporter;

impl FormatImporter for InvalidDraftImporter {
    fn descriptor(&self) -> FormatDescriptor {
        descriptor(INVALID_ID, "invalid")
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        support::submit_invalid_parent_scene(scope)
    }
}

struct GeometryImporter;

impl FormatImporter for GeometryImporter {
    fn descriptor(&self) -> FormatDescriptor {
        descriptor(GEOMETRY_ID, "geometry")
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        support::submit_point_geometry_scene(scope)
    }
}

struct ChangingAssetIo {
    domain: AuthorityDomain,
    domain_lookups: AtomicUsize,
    opens: AtomicUsize,
}

impl ChangingAssetIo {
    fn new() -> Self {
        Self {
            domain: AuthorityDomain::new(),
            domain_lookups: AtomicUsize::new(0),
            opens: AtomicUsize::new(0),
        }
    }

    fn domain_lookup_count(&self) -> usize {
        self.domain_lookups.load(Ordering::Acquire)
    }

    fn open_count(&self) -> usize {
        self.opens.load(Ordering::Acquire)
    }
}

impl AssetIo for ChangingAssetIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain_lookups.fetch_add(1, Ordering::AcqRel);
        self.domain.clone()
    }

    fn open(&self, path: &AssetPath) -> std::result::Result<OpenedAsset, AssetOpenError> {
        let invocation = self.opens.fetch_add(1, Ordering::AcqRel);
        let bytes = if invocation == 0 {
            b"stable primary".to_vec()
        } else {
            b"changed after first open".to_vec()
        };
        let length = u64::try_from(bytes.len()).expect("fixture length fits u64");
        Ok(OpenedAsset::new(
            self.domain.clone(),
            path.clone(),
            Cursor::new(bytes),
            Some(AssetLength::Exact(length)),
        ))
    }
}

fn importer_with<I: FormatImporter>(format: I) -> Importer {
    let mut builder = Importer::builder();
    builder.register(format).expect("fixture must register");
    builder.build().expect("fixture registry must freeze")
}

fn assert_registry_snapshot_mismatch(error: Error) {
    let Error::Request(error @ RequestError::RegistrySnapshotMismatch) = error else {
        panic!("expected a registry snapshot mismatch, got {error:?}");
    };
    assert_eq!(error.code(), "registry-snapshot-mismatch");
}

#[test]
fn foreign_prepared_request_is_rejected_before_io_or_session() {
    let owner = importer_with(OpenOnceImporter);
    let foreign = importer_with(OpenOnceImporter);
    let request = owner
        .prepare_default_request()
        .expect("owner request must compile");

    let report = owner
        .read_bytes_with_request(
            "model.once",
            b"stable primary",
            &request,
            ImportOperationControl::default(),
        )
        .expect("the owner must accept its own request");
    assert_eq!(report.format().id().as_str(), ONCE_ID);

    let bytes_error = foreign
        .read_bytes_with_request(
            "../escape.once",
            b"stable primary",
            &request,
            ImportOperationControl::default(),
        )
        .expect_err("foreign request must fail before source-path parsing");
    assert_registry_snapshot_mismatch(bytes_error);

    let asset_io = ChangingAssetIo::new();
    let asset_error = foreign
        .read_asset_with_request(
            &asset_io,
            AssetPath::new("model.once").expect("valid source path"),
            &request,
            ImportOperationControl::default(),
        )
        .expect_err("foreign request must fail before asset IO");
    assert_registry_snapshot_mismatch(asset_error);
    assert_eq!(asset_io.domain_lookup_count(), 0);
    assert_eq!(asset_io.open_count(), 0);

    let resolver_io = ChangingAssetIo::new();
    let resolver_error = foreign
        .read_asset_with_resolver(
            &resolver_io,
            AssetPath::new("model.once").expect("valid source path"),
            &request,
            &DenyDependencies,
            ImportOperationControl::default(),
        )
        .expect_err("foreign request must fail before resolver or asset IO");
    assert_registry_snapshot_mismatch(resolver_error);
    assert_eq!(resolver_io.domain_lookup_count(), 0);
    assert_eq!(resolver_io.open_count(), 0);
}

#[test]
fn facade_opens_primary_exactly_once_and_reuses_the_bound_snapshot() {
    let importer = importer_with(OpenOnceImporter);
    let io = ChangingAssetIo::new();
    let report = importer
        .read_asset(
            &io,
            AssetPath::new("model.once").expect("valid source path"),
        )
        .expect("one-open import must succeed");

    assert_eq!(io.open_count(), 1);
    assert_eq!(report.format().id().as_str(), ONCE_ID);
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .expect("asset-open evidence")
            .consumed(),
        1
    );
}

#[test]
fn facade_forwards_canonical_authoring_rejection_without_reclassification() {
    let importer = importer_with(InvalidDraftImporter);
    let error = importer
        .read_bytes("model.invalid", b"invalid")
        .expect_err("invalid draft cannot become a report");

    let Error::Import(failure) = error else {
        panic!("expected import failure");
    };
    assert!(matches!(
        failure.cause(),
        ImportCause::SceneValidation(baozi::SceneValidationError::ParentOutOfRange {
            node: 1,
            parent: 2,
            node_count: 2,
        })
    ));
    let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
        panic!("expected canonical authoring terminal rejection");
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Requested);
    assert_eq!(
        failure.selected_format().map(FormatId::as_str),
        Some(INVALID_ID)
    );
    assert_eq!(failure.detection_evidence().len(), 1);
}

#[test]
fn facade_imports_complete_batch_point_geometry() {
    let importer = importer_with(GeometryImporter);
    let report = importer
        .read_bytes("model.geometry", b"geometry")
        .expect("point geometry import must succeed");
    let snapshot = report
        .scene()
        .capture_snapshot(
            SceneSnapshotLimits::try_new(16 * 1024, 128, 16 * 1024)
                .expect("fixture snapshot limits must be valid"),
        )
        .expect("point geometry snapshot must succeed");

    assert_eq!(report.format().id().as_str(), GEOMETRY_ID);
    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(snapshot.root_count(), 1);
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.attribute_count(), 1);
    assert_eq!(snapshot.mesh_instance_count(), 1);
    assert_eq!(snapshot.f32_scalar_count(), 3);
}

#[test]
fn invalid_source_path_fails_before_session_creation() {
    let importer = importer_with(OpenOnceImporter);
    let error = importer
        .read_bytes("../escape.once", b"stable primary")
        .expect_err("parent traversal must be rejected");
    assert!(matches!(error, Error::Path(_)));
}

#[test]
fn enabled_builtin_registry_is_frozen_with_qualified_ids() {
    let importer = Importer::builder_with_builtin_formats()
        .expect("enabled first-party formats must register")
        .build()
        .expect("enabled first-party registry must freeze");
    let actual = importer
        .formats()
        .map(|format| format.id().as_str())
        .collect::<Vec<_>>();
    let mut expected: Vec<&str> = vec![
        #[cfg(feature = "format-stl")]
        "org.baozi.stl",
        #[cfg(feature = "format-obj")]
        "org.baozi.obj",
        #[cfg(feature = "format-ply")]
        "org.baozi.ply",
        #[cfg(feature = "format-gltf")]
        "org.baozi.gltf",
    ];
    expected.sort_unstable();

    assert_eq!(actual, expected);
    assert!(actual.iter().all(|id| id.contains('.')));
}
