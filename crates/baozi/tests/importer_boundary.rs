use baozi::{
    AssetIo, AssetLength, AssetOpenError, AssetPath, AuthorityDomain, CapabilityStatus, Error,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatId, FormatMaturity, ImportCause,
    ImportDimension, ImportOperationControl, Importer, MeshBinding, PostProcessLimits,
    PostProcessPipeline, PostProcessRunError, PostProcessStep, PrimitiveTopology, ReadConfidence,
    RequestError, Vec3,
};
use baozi_import::{
    DetectionOutcome, DetectionResult, DetectionScope, FormatImporter, ImportStorageClass,
    MeshBuildConfig, ParseResult, ParseScope, Probe, ReadHint, SameAuthorityResolver,
};
use baozi_io::OpenedAsset;
use std::io::Cursor;
#[cfg(feature = "native-fs-candidate")]
use std::path::Path;
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
    .maturity(FormatMaturity::Experimental)
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
        let permit = if probe.bytes().is_empty() {
            None
        } else {
            Some(scope.begin_work(probe.bytes().len() as u64)?)
        };
        let outcome = if probe.bytes() == b"stable primary" {
            DetectionOutcome::Match(ReadConfidence::Certain)
        } else {
            DetectionOutcome::NoMatch
        };
        if let Some(permit) = permit {
            permit.complete()?;
        }
        Ok(outcome)
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<baozi_core::SceneDraft> {
        assert_eq!(scope.primary_bytes(), b"stable primary");
        scope.scene_builder()?.finish_draft()
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

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<baozi_core::SceneDraft> {
        let mut builder = scope.scene_builder()?;
        builder.add_node_mesh_binding(builder.root(), MeshBinding::new(baozi::MeshId::new(99)))?;
        builder.finish_draft()
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

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<baozi_core::SceneDraft> {
        let mut positions = scope.metered_vec(
            ImportStorageClass::SceneConstruction,
            "test-position-storage",
        );
        positions.try_push(Vec3::ZERO)?;
        let name = scope
            .metered_string(ImportStorageClass::SceneConstruction, "test-mesh-name")
            .try_into_scene_parts()?
            .discard_into_none();
        let mesh = name
            .into_mesh(MeshBuildConfig {
                topology: PrimitiveTopology::Points,
                material: None,
                bounds: None,
            })
            .with_positions(positions.try_into_scene_parts()?)?
            .finish()?;

        let mut builder = scope.scene_builder()?;
        let mesh = builder.add_prepared_mesh(mesh)?;
        builder.add_node_mesh_binding(builder.root(), MeshBinding::new(mesh))?;
        builder.finish_draft()
    }
}

struct ChangingAssetIo {
    domain: AuthorityDomain,
    opens: AtomicUsize,
}

impl ChangingAssetIo {
    fn new() -> Self {
        Self {
            domain: AuthorityDomain::new(),
            opens: AtomicUsize::new(0),
        }
    }

    fn open_count(&self) -> usize {
        self.opens.load(Ordering::Acquire)
    }
}

impl AssetIo for ChangingAssetIo {
    fn authority_domain(&self) -> AuthorityDomain {
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

#[cfg(feature = "native-fs-candidate")]
struct ObservedSensitivePath {
    resolutions: AtomicUsize,
}

#[cfg(feature = "native-fs-candidate")]
impl ObservedSensitivePath {
    fn new() -> Self {
        Self {
            resolutions: AtomicUsize::new(0),
        }
    }

    fn resolution_count(&self) -> usize {
        self.resolutions.load(Ordering::Acquire)
    }
}

#[cfg(feature = "native-fs-candidate")]
impl AsRef<Path> for ObservedSensitivePath {
    fn as_ref(&self) -> &Path {
        self.resolutions.fetch_add(1, Ordering::AcqRel);
        Path::new("../__baozi_foreign_request_nonexistent__/sensitive.once")
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
    assert_eq!(asset_io.open_count(), 0);

    let resolver_io = ChangingAssetIo::new();
    let resolver_error = foreign
        .read_asset_with_resolver(
            &resolver_io,
            AssetPath::new("model.once").expect("valid source path"),
            &request,
            &SameAuthorityResolver,
            ImportOperationControl::default(),
        )
        .expect_err("foreign request must fail before resolver or asset IO");
    assert_registry_snapshot_mismatch(resolver_error);
    assert_eq!(resolver_io.open_count(), 0);
}

#[cfg(feature = "native-fs-candidate")]
#[test]
fn foreign_prepared_request_is_rejected_before_native_path_resolution() {
    let owner = importer_with(OpenOnceImporter);
    let foreign = importer_with(OpenOnceImporter);
    let request = owner
        .prepare_default_request()
        .expect("owner request must compile");
    let path = ObservedSensitivePath::new();

    let error = foreign
        .read_path_with_request(&path, &request, ImportOperationControl::default())
        .expect_err("foreign request must fail before resolving a native path");

    assert_registry_snapshot_mismatch(error);
    assert_eq!(path.resolution_count(), 0);
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
fn facade_forwards_central_validation_failure_without_reclassification() {
    let importer = importer_with(InvalidDraftImporter);
    let error = importer
        .read_bytes("model.invalid", b"invalid")
        .expect_err("invalid draft cannot become a report");

    let Error::Import(failure) = error else {
        panic!("expected import failure");
    };
    assert!(matches!(failure.cause(), ImportCause::Validation(_)));
    assert_eq!(
        failure.selected_format().map(FormatId::as_str),
        Some(INVALID_ID)
    );
    assert_eq!(failure.detection_evidence().len(), 1);
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
fn processed_failure_retains_the_completed_import_and_separate_ledger() {
    let importer = importer_with(GeometryImporter);
    let report = importer
        .read_bytes("model.geometry", b"geometry")
        .expect("geometry import must succeed");
    let import_generation = report.scene().generation();
    let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateBoundingBoxes])
        .expect("the static test post-process plan must be valid")
        .with_limits(PostProcessLimits::new(0, 1024));

    let failure = Importer::process(report, &pipeline)
        .expect_err("zero work budget must reject post-processing");
    let completed = failure
        .completed_import()
        .expect("post-process failure retains import report");

    assert_eq!(completed.scene().generation(), import_generation);
    assert_eq!(completed.format().id().as_str(), GEOMETRY_ID);
    assert!(matches!(
        failure.postprocess_run_error(),
        Some(PostProcessRunError::Operation { .. })
    ));
    assert!(
        std::error::Error::source(&failure)
            .and_then(|source| source.downcast_ref::<PostProcessRunError>())
            .is_some()
    );
    assert!(failure.postprocess_failure().is_some());
    assert_eq!(completed.ledger().current_live_bytes(), 0);
    assert_eq!(
        failure
            .postprocess_failure()
            .expect("post-process evidence")
            .ledger()
            .current_live_bytes(),
        0
    );
}

#[test]
fn enabled_default_registry_is_frozen_with_qualified_ids() {
    let importer = Importer::new().expect("enabled first-party registry must freeze");
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
    ];
    expected.sort_unstable();

    assert_eq!(actual, expected);
    assert!(actual.iter().all(|id| id.contains('.')));
}
