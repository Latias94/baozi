mod support;

use baozi::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenWorkError, AssetPath, AuthorityDomain,
    CapabilityStatus, Error, FormatCapability, FormatDescriptor, FormatEncoding, FormatId,
    ImportCause, ImportDimension, ImportOperationControl, ImportTerminalCause,
    ImportTerminalFailureClass, Importer, ReadConfidence, SceneSnapshotLimits,
};
use baozi_import::{
    CompiledOptions, DenyDependencies, DetectionOutcome, DetectionResult, DetectionScope, Format,
    NoFormatOptions, OptionCompileError, ParseContext, ParseResult, Probe, ReadHint,
    RegistryBuildScope,
};
use baozi_io::OpenedAsset;
use std::io::Cursor;
use std::num::NonZeroU64;
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

impl Format for OpenOnceImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> std::result::Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

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

    fn parse(&self, _options: &Self::Options, scope: &mut ParseContext<'_>) -> ParseResult<()> {
        assert_eq!(scope.primary_bytes(), b"stable primary");
        support::submit_minimal_scene(scope)
    }
}

struct InvalidDraftImporter;

impl Format for InvalidDraftImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> std::result::Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

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

    fn parse(&self, _options: &Self::Options, scope: &mut ParseContext<'_>) -> ParseResult<()> {
        support::submit_invalid_parent_scene(scope)
    }
}

struct GeometryImporter;

impl Format for GeometryImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> std::result::Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

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

    fn parse(&self, _options: &Self::Options, scope: &mut ParseContext<'_>) -> ParseResult<()> {
        support::submit_point_geometry_scene(scope)
    }
}

struct ChangingAssetIo {
    domain: AuthorityDomain,
    domain_lookups: AtomicUsize,
    bound_lookups: AtomicUsize,
    opens: AtomicUsize,
}

impl ChangingAssetIo {
    fn new() -> Self {
        Self {
            domain: AuthorityDomain::new(),
            domain_lookups: AtomicUsize::new(0),
            bound_lookups: AtomicUsize::new(0),
            opens: AtomicUsize::new(0),
        }
    }

    fn domain_lookup_count(&self) -> usize {
        self.domain_lookups.load(Ordering::Acquire)
    }

    fn bound_lookup_count(&self) -> usize {
        self.bound_lookups.load(Ordering::Acquire)
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

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // Query instrumentation must not affect opens; both payload versions fit this weight.
        self.bound_lookups.fetch_add(1, Ordering::AcqRel);
        Ok(NonZeroU64::new(path.as_str().len() as u64 + 32).expect("bounded fixture work"))
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

fn importer_with<I: Format>(format: I) -> Importer {
    let mut builder = Importer::builder();
    builder.register(format).expect("fixture must register");
    builder.build().expect("fixture registry must freeze")
}

#[test]
fn plan_executes_bytes_assets_and_resolver_after_importer_is_dropped() {
    let plan = {
        let importer = importer_with(OpenOnceImporter);
        importer.default_plan().expect("default plan must compile")
    };

    let report = plan
        .read_bytes(
            AssetPath::new("model.once").expect("valid source path"),
            b"stable primary",
            ImportOperationControl::default(),
        )
        .expect("the plan retains its format and options after the importer is dropped");
    assert_eq!(report.format().id().as_str(), ONCE_ID);

    let asset_io = ChangingAssetIo::new();
    let report = plan
        .read_asset(
            &asset_io,
            AssetPath::new("model.once").expect("valid source path"),
            ImportOperationControl::default(),
        )
        .expect("the retained plan imports through the primary authority");
    assert_eq!(report.format().id().as_str(), ONCE_ID);
    assert!(asset_io.domain_lookup_count() > 0);
    assert_eq!(asset_io.bound_lookup_count(), 1);
    assert_eq!(asset_io.open_count(), 1);

    let resolver_io = ChangingAssetIo::new();
    let report = plan
        .read_asset_with_resolver(
            &resolver_io,
            AssetPath::new("model.once").expect("valid source path"),
            &DenyDependencies,
            ImportOperationControl::default(),
        )
        .expect("the retained plan accepts an explicit resolver");
    assert_eq!(report.format().id().as_str(), ONCE_ID);
    assert!(resolver_io.domain_lookup_count() > 0);
    assert_eq!(resolver_io.bound_lookup_count(), 1);
    assert_eq!(resolver_io.open_count(), 1);
}

#[test]
fn cloned_plans_import_point_geometry_concurrently_without_an_importer() {
    let plan = {
        let importer = importer_with(GeometryImporter);
        importer.default_plan().expect("default plan must compile")
    };
    let start = std::sync::Barrier::new(4);
    std::thread::scope(|threads| {
        let handles = (0..4)
            .map(|_| {
                let plan = plan.clone();
                let start = &start;
                threads.spawn(move || {
                    start.wait();
                    for _ in 0..2 {
                        let report = plan
                            .read_bytes(
                                AssetPath::new("model.geometry").expect("valid source path"),
                                b"geometry",
                                ImportOperationControl::default(),
                            )
                            .expect("each cloned plan owns an independent import operation");
                        assert_eq!(report.format().id().as_str(), GEOMETRY_ID);
                        let snapshot = report
                            .scene()
                            .capture_snapshot(
                                SceneSnapshotLimits::try_new(16 * 1024, 128, 16 * 1024)
                                    .expect("valid snapshot limits"),
                            )
                            .expect("each imported point remains observable");
                        assert_eq!(snapshot.node_count(), 1);
                        assert_eq!(snapshot.primitive_count(), 1);
                        assert_eq!(snapshot.f32_scalar_count(), 3);
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("cloned-plan import completes");
        }
    });
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
    assert_eq!(io.bound_lookup_count(), 1);
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
