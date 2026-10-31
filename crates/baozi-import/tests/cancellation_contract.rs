mod support;

use baozi_core::FormatId;
use baozi_import::{
    AuthorizedDependency, CapabilityStatus, CauseOwner, DependencyPolicyError, DependencyReference,
    DependencyRequirement, DependencyResolver, DependencyRoleId, DetectionOutcome, DetectionResult,
    DetectionScope, FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter,
    ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportPhase, ImportReport,
    ImportRequest, ImportResult, OptionCompileLimits, ParseResult, ParseScope,
    PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    SceneDraft, ScopeAbortCause, import_asset as run_import_asset,
    import_asset_with_resolver as run_import_asset_with_resolver,
};
use baozi_io::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenWorkError, AssetPath, AssetReadError,
    AssetReader, AuthorityDomain, OpenedAsset,
};
use baozi_operation::{CancellationOrigin, CancellationToken, ControlCause};
use std::io::Cursor;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const EXACT_ASSET_BYTES: usize = 1024;

#[allow(
    clippy::result_large_err,
    reason = "cancellation contract assertions inspect complete bounded import failure evidence"
)]
fn operation_result<T>(result: ImportResult<T>) -> Result<T, ImportFailure> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match error.into_operation_failure() {
            Ok(failure) => Err(failure),
            Err(source) => {
                panic!("import test setup failed before a session was created: {source}")
            }
        },
    }
}

#[allow(
    clippy::result_large_err,
    reason = "cancellation contract assertions inspect complete bounded import failure evidence"
)]
fn import_asset(
    request: &PreparedImportRequest,
    io: &dyn AssetIo,
    source: AssetPath,
    control: ImportOperationControl,
) -> Result<ImportReport, ImportFailure> {
    operation_result(run_import_asset(request, io, source, control))
}

#[allow(
    clippy::result_large_err,
    reason = "cancellation contract assertions inspect complete bounded import failure evidence"
)]
fn import_asset_with_resolver(
    request: &PreparedImportRequest,
    io: &dyn AssetIo,
    source: AssetPath,
    resolver: &dyn DependencyResolver,
    control: ImportOperationControl,
) -> Result<ImportReport, ImportFailure> {
    operation_result(run_import_asset_with_resolver(
        request, io, source, resolver, control,
    ))
}

struct EmptyImporter;

struct RequiredDependencyImporter {
    role: DependencyRoleId,
}

struct SceneSubmissionCancellationImporter {
    cancellation: CancellationToken,
    late_successes: Arc<AtomicUsize>,
}

struct CachedDependencyCancellationImporter {
    role: DependencyRoleId,
    cancellation: CancellationToken,
    late_successes: Arc<AtomicUsize>,
}

impl FormatImporter for EmptyImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.cancellation").unwrap(),
            "Cancellation Contract",
        )
        .extensions(["cancel"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        support::submit_minimal_scene(scope)
    }
}

impl FormatImporter for RequiredDependencyImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.dependency-cancellation").unwrap(),
            "Dependency Cancellation Contract",
        )
        .extensions(["dependency-cancel"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .dependency_roles([(self.role.clone(), DependencyRequirement::Required)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let _dependency = support::required_root_dependency(scope, &self.role, "sidecar.bin")?;
        support::submit_minimal_scene(scope)
    }
}

impl SceneSubmissionCancellationImporter {
    fn record_late_success(&self, succeeded: bool) {
        if succeeded {
            self.late_successes.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn cancel(&self) {
        self.cancellation.cancel();
    }
}

impl FormatImporter for SceneSubmissionCancellationImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.scene-submission-cancellation").unwrap(),
            "Scene Submission Cancellation Contract",
        )
        .extensions(["scene-submit-cancel"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        self.cancel();
        match support::submit_minimal_scene(scope) {
            Ok(draft) => {
                self.record_late_success(true);
                Ok(draft)
            }
            Err(failure) => Err(failure),
        }
    }
}

impl FormatImporter for CachedDependencyCancellationImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.cached-dependency-cancellation").unwrap(),
            "Cached Dependency Cancellation Contract",
        )
        .extensions(["cached-dependency-cancel"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .dependency_roles([(self.role.clone(), DependencyRequirement::Required)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let first = support::required_root_dependency(scope, &self.role, "sidecar.bin")?;
        drop(first);

        self.cancellation.cancel();
        let second_succeeded =
            support::required_root_dependency(scope, &self.role, "sidecar.bin").is_ok();
        if second_succeeded {
            self.late_successes.fetch_add(1, Ordering::AcqRel);
        }

        scope.poll()?;
        unreachable!("cancelled parser must not continue")
    }
}

struct CountingReader {
    calls: Arc<AtomicUsize>,
    offset: usize,
    length: usize,
}

impl AssetReader for CountingReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let count = (self.length - self.offset).min(buffer.len());
        buffer[..count].fill(0);
        self.offset += count;
        Ok(count)
    }
}

struct CancelOnOpenIo {
    domain: AuthorityDomain,
    source: AssetPath,
    cancellation: CancellationToken,
    reader_calls: Arc<AtomicUsize>,
}

struct CancelOnDependencyOpenIo {
    domain: AuthorityDomain,
    primary: AssetPath,
    dependency: AssetPath,
    cancellation: CancellationToken,
    opens: AtomicUsize,
    dependency_reader_calls: Arc<AtomicUsize>,
}

struct StableDependencyIo {
    domain: AuthorityDomain,
    primary: AssetPath,
    dependency: Option<AssetPath>,
    opens: AtomicUsize,
}

struct CountingResolver {
    calls: AtomicUsize,
}

impl AssetIo for CancelOnOpenIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // One path comparison/clone plus fixed cancellation and reader setup; no cancellation here.
        Ok(NonZeroU64::new(2 * path.as_str().len() as u64 + 16).expect("bounded fixture work"))
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        assert_eq!(path, &self.source);
        self.cancellation.cancel();
        Ok(OpenedAsset::new(
            self.domain.clone(),
            self.source.clone(),
            CountingReader {
                calls: Arc::clone(&self.reader_calls),
                offset: 0,
                length: EXACT_ASSET_BYTES,
            },
            Some(AssetLength::Exact(EXACT_ASSET_BYTES as u64)),
        ))
    }
}

impl AssetIo for CancelOnDependencyOpenIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // Cover both path comparisons, a path clone, and the tiny primary copies or reader setup.
        Ok(NonZeroU64::new(3 * path.as_str().len() as u64 + 32).expect("bounded fixture work"))
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.opens.fetch_add(1, Ordering::AcqRel);
        if path == &self.primary {
            let bytes = b"primary".to_vec();
            return Ok(OpenedAsset::new(
                self.domain.clone(),
                self.primary.clone(),
                Cursor::new(bytes.clone()),
                Some(AssetLength::Exact(bytes.len() as u64)),
            ));
        }

        assert_eq!(path, &self.dependency);
        self.cancellation.cancel();
        Ok(OpenedAsset::new(
            self.domain.clone(),
            self.dependency.clone(),
            CountingReader {
                calls: Arc::clone(&self.dependency_reader_calls),
                offset: 0,
                length: EXACT_ASSET_BYTES,
            },
            Some(AssetLength::Exact(EXACT_ASSET_BYTES as u64)),
        ))
    }
}

impl AssetIo for StableDependencyIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // Two identity comparisons, one path clone, and fixed counter/borrowed-reader setup.
        Ok(NonZeroU64::new(3 * path.as_str().len() as u64 + 16).expect("bounded fixture work"))
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.opens.fetch_add(1, Ordering::AcqRel);
        let bytes = if path == &self.primary {
            b"primary".as_slice()
        } else {
            assert_eq!(Some(path), self.dependency.as_ref());
            b"dependency".as_slice()
        };
        Ok(OpenedAsset::new(
            self.domain.clone(),
            path.clone(),
            Cursor::new(bytes),
            Some(AssetLength::Exact(bytes.len() as u64)),
        ))
    }
}

impl DependencyResolver for CountingResolver {
    fn authorize<'a>(
        &'a self,
        primary: &'a dyn AssetIo,
        primary_authority: &AuthorityDomain,
        _primary_path: &AssetPath,
        _role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let path = reference
            .normalized()
            .try_clone()
            .map_err(|_| DependencyPolicyError::AllocationFailed)?;
        Ok(AuthorizedDependency::new(
            primary,
            primary_authority.clone(),
            path,
        ))
    }
}

fn request_for(importer: impl FormatImporter) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    registry.register(importer).unwrap();
    let snapshot = registry.freeze(RegistryBuildLimits::default()).unwrap();
    snapshot
        .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
        .unwrap()
}

#[test]
fn primary_open_cancellation_precedes_exact_length_charge_capacity_and_reader_entry() {
    let source = AssetPath::new("asset.cancel").unwrap();
    let cancellation = CancellationToken::new();
    let reader_calls = Arc::new(AtomicUsize::new(0));
    let io = CancelOnOpenIo {
        domain: AuthorityDomain::new(),
        source: source.clone(),
        cancellation: cancellation.clone(),
        reader_calls: Arc::clone(&reader_calls),
    };
    let request = request_for(EmptyImporter);
    let control = ImportOperationControl::default().with_cancellation(cancellation);

    let failure = import_asset(&request, &io, source, control)
        .expect_err("cancellation raised by open must stop snapshot admission");

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected operation cancellation through the scope")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        })
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SessionCreated
    );
    assert_eq!(reader_calls.load(Ordering::Acquire), 0);

    let primary = failure
        .ledger()
        .dimension(ImportDimension::PrimaryBytes)
        .unwrap();
    assert_eq!(primary.attempted(), 0);
    assert_eq!(primary.consumed(), 0);
    assert_eq!(primary.peak_live_bytes(), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn dependency_open_cancellation_precedes_exact_length_charge_capacity_and_reader_entry() {
    let source = AssetPath::new("asset.dependency-cancel").unwrap();
    let dependency = AssetPath::new("sidecar.bin").unwrap();
    let cancellation = CancellationToken::new();
    let reader_calls = Arc::new(AtomicUsize::new(0));
    let role = DependencyRoleId::new("buffer.required").unwrap();
    let io = CancelOnDependencyOpenIo {
        domain: AuthorityDomain::new(),
        primary: source.clone(),
        dependency,
        cancellation: cancellation.clone(),
        opens: AtomicUsize::new(0),
        dependency_reader_calls: Arc::clone(&reader_calls),
    };
    let request = request_for(RequiredDependencyImporter { role });
    let control = ImportOperationControl::default().with_cancellation(cancellation);

    let failure = import_asset(&request, &io, source, control)
        .expect_err("dependency-open cancellation must stop snapshot admission");

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected operation cancellation through the scope")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        })
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    assert_eq!(io.opens.load(Ordering::Acquire), 2);
    assert_eq!(reader_calls.load(Ordering::Acquire), 0);

    let dependency = failure
        .ledger()
        .dimension(ImportDimension::DependencyBytes)
        .unwrap();
    assert_eq!(dependency.attempted(), 0);
    assert_eq!(dependency.consumed(), 0);
    assert_eq!(dependency.peak_live_bytes(), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn cancellation_is_admitted_before_complete_batch_scene_submission() {
    let source = AssetPath::new("asset.scene-submit-cancel").unwrap();
    let cancellation = CancellationToken::new();
    let late_successes = Arc::new(AtomicUsize::new(0));
    let io = StableDependencyIo {
        domain: AuthorityDomain::new(),
        primary: source.clone(),
        dependency: None,
        opens: AtomicUsize::new(0),
    };
    let request = request_for(SceneSubmissionCancellationImporter {
        cancellation: cancellation.clone(),
        late_successes: Arc::clone(&late_successes),
    });
    let control = ImportOperationControl::default().with_cancellation(cancellation);

    let failure = import_asset(&request, &io, source, control)
        .expect_err("Scene submission must admit cancellation before activation");

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected operation cancellation through the scope")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        })
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    assert_eq!(late_successes.load(Ordering::Acquire), 0);
    assert_eq!(io.opens.load(Ordering::Acquire), 1);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn cached_dependency_reuse_admits_cancellation_before_reference_and_resolver_work() {
    let source = AssetPath::new("asset.cached-dependency-cancel").unwrap();
    let dependency = AssetPath::new("sidecar.bin").unwrap();
    let cancellation = CancellationToken::new();
    let late_successes = Arc::new(AtomicUsize::new(0));
    let role = DependencyRoleId::new("buffer.required").unwrap();
    let io = StableDependencyIo {
        domain: AuthorityDomain::new(),
        primary: source.clone(),
        dependency: Some(dependency),
        opens: AtomicUsize::new(0),
    };
    let resolver = CountingResolver {
        calls: AtomicUsize::new(0),
    };
    let request = request_for(CachedDependencyCancellationImporter {
        role,
        cancellation: cancellation.clone(),
        late_successes: Arc::clone(&late_successes),
    });
    let control = ImportOperationControl::default().with_cancellation(cancellation);

    let failure = import_asset_with_resolver(&request, &io, source, &resolver, control)
        .expect_err("cached dependency reuse must admit cancellation before logical work");

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected operation cancellation through the scope")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        })
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    assert_eq!(late_successes.load(Ordering::Acquire), 0);
    assert_eq!(resolver.calls.load(Ordering::Acquire), 1);
    assert_eq!(io.opens.load(Ordering::Acquire), 2);

    let references = failure
        .ledger()
        .dimension(ImportDimension::DependencyReference)
        .unwrap();
    assert_eq!(references.attempted(), 1);
    assert_eq!(references.consumed(), 1);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}
