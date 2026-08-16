use baozi_core::{FormatId, Mesh, SceneDraft, SceneSpace};
use baozi_import::{
    AuthorizedDependency, CapabilityStatus, CauseOwner, DependencyPolicyError, DependencyReference,
    DependencyRequirement, DependencyResolver, DependencyRoleId, DetectionFailure,
    DetectionInconclusive, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatFailureDetailSink, FormatFailureDetailValue,
    FormatFailureEvidence, FormatFailureEvidenceProducer, FormatImporter, FormatSelection,
    ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure, ImportLossKind,
    ImportOperationControl, ImportPhase, ImportProgressPhase, ImportProgressUnit, ImportReport,
    ImportRequest, ImportRequestLayer, ImportResult, ImportStorageClass, ImportWorkClass,
    LossPolicy, MeteredDecodedText, MeteredString, MeteredVec, OptionCompileLimits, ParseResult,
    ParseScope, PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuilder, ResourceLimits, ResourceLimitsOverlay, ScopeAbortCause, TextBomPolicy,
    TextControlPolicy, TextDecodingMode, TextHardCaps, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding, TrustedExtensionBoundary, import_asset as run_import_asset,
    import_asset_with_resolver as run_import_asset_with_resolver, import_bytes as run_import_bytes,
};
use baozi_io::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetPath,
    AssetReadError, AssetReadErrorKind, AssetReader, AuthorityDomain, OpenedAsset,
};
use baozi_operation::{CancellationToken, LedgerCause, ProgressDecision};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[allow(
    clippy::result_large_err,
    reason = "session contract assertions inspect complete bounded import failure evidence"
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
    reason = "session contract assertions inspect complete bounded import failure evidence"
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
    reason = "session contract assertions inspect complete bounded import failure evidence"
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

#[allow(
    clippy::result_large_err,
    reason = "session contract assertions inspect complete bounded import failure evidence"
)]
fn import_bytes(
    request: &PreparedImportRequest,
    source: AssetPath,
    bytes: &[u8],
    control: ImportOperationControl,
) -> Result<ImportReport, ImportFailure> {
    operation_result(run_import_bytes(request, source, bytes, control))
}

#[derive(Clone)]
struct RecordingImporter {
    id: &'static str,
    extension: &'static str,
    dependency: Option<(DependencyRoleId, DependencyRequirement, DependencyBehavior)>,
    dependency_views: Arc<Mutex<Vec<Vec<u8>>>>,
    dependency_dispositions: Arc<Mutex<Vec<baozi_import::DependencyCacheDisposition>>>,
    primary_views: Arc<Mutex<Vec<Vec<u8>>>>,
    primary_addresses: Arc<Mutex<Vec<usize>>>,
    detector_cancel: Option<CancellationToken>,
    detection: DetectionOutcome,
}

#[derive(Debug, Clone, Copy)]
enum DependencyBehavior {
    RequiredCaught,
    RequiredAvailableWithDiagnostic,
    RequiredAvailableTwice,
    OptionalTwice,
    OptionalThenFail,
}

impl RecordingImporter {
    fn plain(id: &'static str, extension: &'static str) -> Self {
        Self {
            id,
            extension,
            dependency: None,
            dependency_views: Arc::new(Mutex::new(Vec::new())),
            dependency_dispositions: Arc::new(Mutex::new(Vec::new())),
            primary_views: Arc::new(Mutex::new(Vec::new())),
            primary_addresses: Arc::new(Mutex::new(Vec::new())),
            detector_cancel: None,
            detection: DetectionOutcome::Match(ReadConfidence::Certain),
        }
    }

    fn with_dependency(
        mut self,
        role: DependencyRoleId,
        requirement: DependencyRequirement,
        behavior: DependencyBehavior,
    ) -> Self {
        self.dependency = Some((role, requirement, behavior));
        self
    }

    fn descriptor_value(&self) -> FormatDescriptor {
        let mut builder = FormatDescriptor::builder(
            FormatId::new(self.id).expect("valid format id"),
            "Session Test",
        )
        .extensions([self.extension])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)]);
        if let Some((role, requirement, _)) = &self.dependency {
            builder = builder.dependency_roles([(role.clone(), *requirement)]);
        }
        builder.build().expect("valid descriptor")
    }
}

impl FormatImporter for RecordingImporter {
    fn descriptor(&self) -> FormatDescriptor {
        self.descriptor_value()
    }

    fn detect(
        &self,
        probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        self.primary_addresses
            .lock()
            .expect("recording mutex")
            .push(probe.bytes().as_ptr() as usize);
        let mut observed = Vec::new();
        for (index, byte) in probe.bytes().iter().copied().enumerate() {
            observed.push(byte);
            if index == probe.bytes().len() / 2
                && let Some(token) = &self.detector_cancel
            {
                token.cancel();
            }
        }
        self.primary_views
            .lock()
            .expect("recording mutex")
            .push(observed);
        Ok(self.detection.clone())
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        self.primary_addresses
            .lock()
            .expect("recording mutex")
            .push(scope.primary_bytes().as_ptr() as usize);
        self.primary_views
            .lock()
            .expect("recording mutex")
            .push(scope.primary_bytes().to_vec());
        if let Some((role, _, behavior)) = &self.dependency {
            match behavior {
                DependencyBehavior::RequiredCaught => {
                    let _ignored = scope.required_dependency(role, "missing.bin");
                }
                DependencyBehavior::RequiredAvailableWithDiagnostic => {
                    let diagnostic_source = scope.diagnostic_source();
                    assert_eq!(diagnostic_source.path(), scope.source());
                    assert_eq!(diagnostic_source.bytes(), scope.primary_bytes());
                    let dependency = scope.required_dependency(role, "missing.bin")?;
                    assert_eq!(dependency.bytes(), b"dependency");
                }
                DependencyBehavior::RequiredAvailableTwice => {
                    let first = scope.required_dependency(role, "missing.bin")?;
                    let second = scope.required_dependency(role, "missing.bin")?;
                    self.dependency_views
                        .lock()
                        .expect("recording mutex")
                        .extend([first.bytes().to_vec(), second.bytes().to_vec()]);
                    self.dependency_dispositions
                        .lock()
                        .expect("recording mutex")
                        .extend([first.cache_disposition(), second.cache_disposition()]);
                }
                DependencyBehavior::OptionalTwice => {
                    let first = scope.optional_dependency(role, "missing.bin")?;
                    let second = scope.optional_dependency(role, "missing.bin")?;
                    assert!(first.unavailable_evidence().is_some());
                    assert!(second.unavailable_evidence().is_some());
                }
                DependencyBehavior::OptionalThenFail => {
                    let dependency = scope.optional_dependency(role, "missing.bin")?;
                    assert!(dependency.unavailable_evidence().is_some());
                    return Err(scope.format_failure(TestFormatCause::new(Arc::new(
                        FormatFailureProducerObservation::default(),
                    ))));
                }
            }
        }
        scope.scene_builder()?.finish_draft()
    }
}

struct FormatFailingImporter {
    observation: Arc<FormatFailureProducerObservation>,
}
struct OversizedFormatFailingImporter;

#[derive(Debug, Clone, Copy)]
enum ReentrantFormatFailurePhase {
    Code,
    Details,
    DecodedStorage,
    FormattedStorage,
    MeteredStorage,
    ReservedStorage,
    ScenePartsConversion,
    NestedFormatFailure,
    Drop,
}

struct ReentrantFormatFailureImporter {
    phase: ReentrantFormatFailurePhase,
}

struct PanickingImporter {
    calls: Arc<AtomicUsize>,
}

struct OpaquePanickingImporter {
    payload_drops: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Copy)]
enum OpaqueAssetIoPanicBoundary {
    Authority,
    Open,
    Read,
}

struct OpaquePanickingAssetIo {
    boundary: OpaqueAssetIoPanicBoundary,
    domain: AuthorityDomain,
    source: AssetPath,
    payload_drops: Arc<AtomicUsize>,
}

struct OpaquePanickingReader {
    payload_drops: Arc<AtomicUsize>,
}

struct RacingParsePoisonImporter {
    calls: Arc<AtomicUsize>,
    first_entered: SyncSender<()>,
    release_first: Mutex<Receiver<()>>,
}

struct PanickingDropPayload {
    drops: Arc<AtomicUsize>,
}

impl Drop for PanickingDropPayload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        panic!("untrusted panic payload destructor")
    }
}

impl AssetReader for OpaquePanickingReader {
    fn read_chunk(&mut self, _buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        std::panic::panic_any(PanickingDropPayload {
            drops: Arc::clone(&self.payload_drops),
        })
    }
}

impl AssetIo for OpaquePanickingAssetIo {
    fn authority_domain(&self) -> AuthorityDomain {
        if matches!(self.boundary, OpaqueAssetIoPanicBoundary::Authority) {
            std::panic::panic_any(PanickingDropPayload {
                drops: Arc::clone(&self.payload_drops),
            });
        }
        self.domain.clone()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        assert_eq!(path, &self.source);
        if matches!(self.boundary, OpaqueAssetIoPanicBoundary::Open) {
            std::panic::panic_any(PanickingDropPayload {
                drops: Arc::clone(&self.payload_drops),
            });
        }
        Ok(OpenedAsset::new(
            self.domain.clone(),
            self.source.clone(),
            OpaquePanickingReader {
                payload_drops: Arc::clone(&self.payload_drops),
            },
            Some(AssetLength::Exact(1)),
        ))
    }
}

impl FormatImporter for PanickingImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.panic-shared").unwrap(),
            "Panic Shared",
        )
        .extensions(["panic"])
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

    fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        panic!("shared registration panic")
    }
}

impl FormatImporter for OpaquePanickingImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.opaque-panic").unwrap(),
            "Opaque Panic",
        )
        .extensions(["opaque-panic"])
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

    fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        std::panic::panic_any(PanickingDropPayload {
            drops: Arc::clone(&self.payload_drops),
        })
    }
}

impl FormatImporter for RacingParsePoisonImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.racing-parse-poison").unwrap(),
            "Racing Parse Poison",
        )
        .extensions(["racing-parse-poison"])
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
        if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
            self.first_entered
                .send(())
                .expect("test observes the first parser entry");
            self.release_first
                .lock()
                .expect("release receiver mutex")
                .recv_timeout(Duration::from_secs(5))
                .expect("test releases the first parser");
            scope.scene_builder()?.finish_draft()
        } else {
            panic!("racing parser panic")
        }
    }
}

impl FormatImporter for FormatFailingImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.format-failure").unwrap(),
            "Format Failure",
        )
        .extensions(["failure"])
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
        Err(scope.format_failure(TestFormatCause::new(Arc::clone(&self.observation))))
    }
}

impl FormatImporter for OversizedFormatFailingImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.oversized-format-failure").unwrap(),
            "Oversized Format Failure",
        )
        .extensions(["oversized-failure"])
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
        Err(scope.format_failure(OversizedFormatCause {
            _payload: [0; 4 * 1024 + 1],
        }))
    }
}

#[derive(Debug)]
struct OversizedFormatCause {
    _payload: [u8; 4 * 1024 + 1],
}

impl fmt::Display for OversizedFormatCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("oversized format failure")
    }
}

impl Error for OversizedFormatCause {}

impl FormatFailureEvidenceProducer for OversizedFormatCause {
    fn code(&self) -> &'static str {
        "test.oversized_format_failure"
    }
}

#[derive(Debug)]
struct TestFormatCause {
    code: String,
    key: String,
    value: String,
    _source: TestBackendCause,
    observation: Arc<FormatFailureProducerObservation>,
}

impl TestFormatCause {
    fn new(observation: Arc<FormatFailureProducerObservation>) -> Self {
        let code = String::from("test.format_failure");
        let key = String::from("backend_message");
        let value = String::from("private test backend failure");
        observation
            .code_pointer
            .store(code.as_ptr() as usize, Ordering::Release);
        observation
            .key_pointer
            .store(key.as_ptr() as usize, Ordering::Release);
        observation
            .value_pointer
            .store(value.as_ptr() as usize, Ordering::Release);
        Self {
            code,
            key,
            value,
            _source: TestBackendCause {
                observation: Arc::clone(&observation),
            },
            observation,
        }
    }
}

impl Drop for TestFormatCause {
    fn drop(&mut self) {
        self.observation.drops.fetch_add(1, Ordering::AcqRel);
    }
}

impl FormatFailureEvidenceProducer for TestFormatCause {
    fn code(&self) -> &str {
        &self.code
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        sink.unsigned("record", 7);
        sink.string(&self.key, &self.value);
    }
}

#[derive(Debug)]
struct TestBackendCause {
    observation: Arc<FormatFailureProducerObservation>,
}

impl Drop for TestBackendCause {
    fn drop(&mut self) {
        self.observation.drops.fetch_add(1, Ordering::AcqRel);
    }
}

#[derive(Debug, Default)]
struct FormatFailureProducerObservation {
    drops: AtomicUsize,
    code_pointer: AtomicUsize,
    key_pointer: AtomicUsize,
    value_pointer: AtomicUsize,
}

#[derive(Debug, Clone, Copy)]
enum FailureEvidenceMode {
    Normal,
    CodePanics,
    InvalidCode,
    OverlongCode,
    DetailsPanic,
    DetailsRejected,
    DropPanics,
}

#[derive(Debug)]
struct LifecycleFormatCause {
    mode: FailureEvidenceMode,
    visits: Arc<AtomicUsize>,
}

impl fmt::Display for LifecycleFormatCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("lifecycle format failure")
    }
}

impl Error for LifecycleFormatCause {}

impl FormatFailureEvidenceProducer for LifecycleFormatCause {
    fn code(&self) -> &'static str {
        match self.mode {
            FailureEvidenceMode::CodePanics => panic!("format failure code panic"),
            FailureEvidenceMode::InvalidCode => "",
            FailureEvidenceMode::OverlongCode => concat!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            FailureEvidenceMode::Normal
            | FailureEvidenceMode::DetailsPanic
            | FailureEvidenceMode::DetailsRejected
            | FailureEvidenceMode::DropPanics => "test.lifecycle_failure",
        }
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        self.visits.fetch_add(1, Ordering::AcqRel);
        match self.mode {
            FailureEvidenceMode::DetailsPanic => panic!("format failure detail panic"),
            FailureEvidenceMode::DetailsRejected => sink.unsigned("INVALID", 1),
            FailureEvidenceMode::Normal | FailureEvidenceMode::DropPanics => {
                sink.unsigned("record", 7);
            }
            FailureEvidenceMode::CodePanics
            | FailureEvidenceMode::InvalidCode
            | FailureEvidenceMode::OverlongCode => {}
        }
    }
}

impl Drop for LifecycleFormatCause {
    fn drop(&mut self) {
        if matches!(self.mode, FailureEvidenceMode::DropPanics) {
            panic!("format failure producer drop panic");
        }
    }
}

struct NestedFormatFailureProducer;

impl FormatFailureEvidenceProducer for NestedFormatFailureProducer {
    fn code(&self) -> &str {
        "test.nested_format_failure"
    }
}

struct ReentrantFormatFailureProducer<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    phase: ReentrantFormatFailurePhase,
    storage: RefCell<Option<MeteredVec<'scope, u8>>>,
    decoded: Option<RefCell<MeteredDecodedText<'scope>>>,
    formatted: Option<RefCell<MeteredString<'scope>>>,
}

struct FailingFormatValue;

impl fmt::Display for FailingFormatValue {
    fn fmt(&self, _formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Err(fmt::Error)
    }
}

impl FormatFailureEvidenceProducer for ReentrantFormatFailureProducer<'_, '_> {
    fn code(&self) -> &str {
        if matches!(self.phase, ReentrantFormatFailurePhase::Code) {
            drop(
                self.scope
                    .with_work(ImportWorkClass::WorkUnits, 1, |work| work.complete_all()),
            );
        }
        "test.reentrant_format_failure"
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        match self.phase {
            ReentrantFormatFailurePhase::Details => drop(self.scope.scene_builder()),
            ReentrantFormatFailurePhase::DecodedStorage => {
                drop(
                    self.decoded
                        .as_ref()
                        .expect("decoded-storage phase owns decoded text")
                        .borrow_mut()
                        .try_push_str_reserved("x"),
                );
            }
            ReentrantFormatFailurePhase::FormattedStorage => {
                drop(
                    self.formatted
                        .as_ref()
                        .expect("formatted-storage phase owns a string")
                        .borrow_mut()
                        .try_write_fmt(format_args!("{}", FailingFormatValue)),
                );
            }
            ReentrantFormatFailurePhase::MeteredStorage => {
                drop(
                    self.storage
                        .borrow_mut()
                        .as_mut()
                        .expect("metered-storage phase owns storage")
                        .try_push(1),
                );
            }
            ReentrantFormatFailurePhase::ReservedStorage => {
                drop(
                    self.storage
                        .borrow_mut()
                        .as_mut()
                        .expect("reserved-storage phase owns storage")
                        .try_push_reserved(1),
                );
            }
            ReentrantFormatFailurePhase::ScenePartsConversion => {
                let storage = self
                    .storage
                    .borrow_mut()
                    .take()
                    .expect("scene-parts phase owns storage");
                drop(storage.try_into_scene_parts());
            }
            ReentrantFormatFailurePhase::NestedFormatFailure => {
                drop(self.scope.format_failure(NestedFormatFailureProducer));
            }
            ReentrantFormatFailurePhase::Code | ReentrantFormatFailurePhase::Drop => {}
        }
        sink.unsigned("ignored", 1);
    }
}

impl Drop for ReentrantFormatFailureProducer<'_, '_> {
    fn drop(&mut self) {
        if matches!(self.phase, ReentrantFormatFailurePhase::Drop) {
            drop(
                self.scope
                    .with_work(ImportWorkClass::WorkUnits, 1, |work| work.complete_all()),
            );
        }
    }
}

impl FormatImporter for ReentrantFormatFailureImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.reentrant-format-failure").unwrap(),
            "Reentrant Format Failure",
        )
        .extensions(["reentrant-failure"])
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
        let class = if matches!(
            self.phase,
            ReentrantFormatFailurePhase::ScenePartsConversion
        ) {
            ImportStorageClass::SceneConstruction
        } else {
            ImportStorageClass::FormatNormalization
        };
        let mut storage = scope.metered_vec(class, "reentrant-format-failure-storage");
        if matches!(self.phase, ReentrantFormatFailurePhase::ReservedStorage) {
            storage.try_reserve(1)?;
        }
        let decoded = if matches!(self.phase, ReentrantFormatFailurePhase::DecodedStorage) {
            Some(RefCell::new(scope.metered_decoded_text(
                1,
                "reentrant-format-failure-decoded-text",
            )?))
        } else {
            None
        };
        let formatted =
            matches!(self.phase, ReentrantFormatFailurePhase::FormattedStorage).then(|| {
                RefCell::new(scope.metered_string(
                    ImportStorageClass::FormatNormalization,
                    "reentrant-format-failure-formatted-storage",
                ))
            });
        Err(scope.format_failure(ReentrantFormatFailureProducer {
            scope,
            phase: self.phase,
            storage: RefCell::new(Some(storage)),
            decoded,
            formatted,
        }))
    }
}

#[derive(Debug, Clone, Copy)]
enum FailureReturnMode {
    Return,
    Swallow,
    CancelAfterCreate,
}

struct LifecycleFailureImporter {
    evidence: FailureEvidenceMode,
    return_mode: FailureReturnMode,
    visits: Arc<AtomicUsize>,
    cancellation: Option<CancellationToken>,
}

#[derive(Debug, Clone, Copy)]
enum TerminalLatchTrigger {
    MissingRequiredDependency,
    ResourceLimit,
    Cancellation,
}

struct TerminalLatchImporter {
    trigger: TerminalLatchTrigger,
    required_role: DependencyRoleId,
    optional_role: DependencyRoleId,
    cancellation: CancellationToken,
    late_successes: Arc<AtomicUsize>,
    diagnostic_callbacks: Arc<AtomicUsize>,
    format_detail_callbacks: Arc<AtomicUsize>,
    builder_mutations: Arc<AtomicUsize>,
}

struct CountingResolver {
    calls: AtomicUsize,
}

impl CountingResolver {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::Acquire)
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

struct NormalizationAdmissionImporter {
    role: DependencyRoleId,
    raw_reference: String,
}

#[derive(Debug, Clone, Copy)]
enum DiagnosticPanicMode {
    Details,
    Message,
}

struct DiagnosticPanickingImporter {
    mode: DiagnosticPanicMode,
}

struct PanickingDiagnosticMessage;

impl fmt::Display for PanickingDiagnosticMessage {
    fn fmt(&self, _formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("diagnostic message panic")
    }
}

impl FormatImporter for DiagnosticPanickingImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.diagnostic-panic").unwrap(),
            "Diagnostic Panic",
        )
        .extensions(["diagnostic-panic"])
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
        let code = baozi_import::DiagnosticCode::new("test.diagnostic_panic").unwrap();
        match self.mode {
            DiagnosticPanicMode::Details => scope.emit_diagnostic_with(
                baozi_import::DiagnosticSeverity::Warning,
                code,
                ImportProgressPhase::Decode,
                scope.diagnostic_source(),
                None,
                None,
                |_| panic!("diagnostic details panic"),
                format_args!("diagnostic details panic"),
            )?,
            DiagnosticPanicMode::Message => {
                let message = PanickingDiagnosticMessage;
                scope.emit_diagnostic_with(
                    baozi_import::DiagnosticSeverity::Warning,
                    code,
                    ImportProgressPhase::Decode,
                    scope.diagnostic_source(),
                    None,
                    None,
                    |_| {},
                    format_args!("{message}"),
                )?;
            }
        }
        scope.scene_builder()?.finish_draft()
    }
}

impl FormatImporter for NormalizationAdmissionImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.normalization-admission").unwrap(),
            "Normalization Admission",
        )
        .extensions(["normalization-admission"])
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
        let _dependency = scope.required_dependency(&self.role, &self.raw_reference)?;
        scope.scene_builder()?.finish_draft()
    }
}

impl FormatImporter for TerminalLatchImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.terminal-latch").unwrap(),
            "Terminal Latch",
        )
        .extensions(["terminal-latch"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .dependency_roles([
            (self.required_role.clone(), DependencyRequirement::Required),
            (self.optional_role.clone(), DependencyRequirement::Optional),
        ])
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
        let prepared_mesh = scope.prepare_scene_value(Mesh::default())?;
        let mut resize_storage = scope.metered_vec::<u8>(
            ImportStorageClass::BackendDecode,
            "terminal-latch-resize-storage",
        );
        resize_storage.try_push(0)?;
        let resize_capacity = resize_storage.capacity();
        let mut builder = scope.scene_builder()?;
        let first_failure = match self.trigger {
            TerminalLatchTrigger::MissingRequiredDependency => {
                match scope.required_dependency(&self.required_role, "missing.bin") {
                    Ok(_) => panic!("the required dependency is absent"),
                    Err(failure) => failure,
                }
            }
            TerminalLatchTrigger::ResourceLimit => scope
                .enforce_resource_limit("terminal-latch-trigger", 2, 1)
                .expect_err("the injected resource limit must fail"),
            TerminalLatchTrigger::Cancellation => {
                self.cancellation.cancel();
                scope
                    .poll()
                    .expect_err("the injected cancellation must fail")
            }
        };
        drop(first_failure);

        let record_late_success = |succeeded: bool| {
            if succeeded {
                self.late_successes.fetch_add(1, Ordering::AcqRel);
            }
        };
        record_late_success(
            scope
                .optional_dependency(&self.optional_role, "late-optional.bin")
                .is_ok(),
        );
        record_late_success(
            scope
                .required_dependency(&self.required_role, "late-required.bin")
                .is_ok(),
        );
        record_late_success(
            scope
                .with_work(ImportWorkClass::WorkUnits, 1, |work| work.complete_all())
                .is_ok(),
        );
        record_late_success(
            scope
                .metered_opaque_allowance(ImportStorageClass::BackendDecode, 1)
                .is_ok(),
        );
        record_late_success(resize_storage.try_reserve(resize_capacity).is_ok());
        if resize_storage.capacity() != resize_capacity {
            self.builder_mutations.fetch_add(1, Ordering::AcqRel);
        }
        record_late_success(
            scope
                .report_progress(
                    ImportProgressPhase::Decode,
                    ImportProgressUnit::Records,
                    0,
                    None,
                )
                .is_ok(),
        );
        record_late_success(
            scope
                .enforce_resource_limit("late-noop-limit", 0, 1)
                .is_ok(),
        );

        let diagnostic_callbacks = Arc::clone(&self.diagnostic_callbacks);
        record_late_success(
            scope
                .emit_diagnostic_with(
                    baozi_import::DiagnosticSeverity::Warning,
                    baozi_import::DiagnosticCode::new("test.late_diagnostic").unwrap(),
                    ImportProgressPhase::Decode,
                    scope.diagnostic_source(),
                    None,
                    None,
                    move |_| {
                        diagnostic_callbacks.fetch_add(1, Ordering::AcqRel);
                    },
                    format_args!("late diagnostic"),
                )
                .is_ok(),
        );

        let mesh_count = builder.mesh_count();
        record_late_success(builder.add_prepared_mesh(prepared_mesh).is_ok());
        if builder.mesh_count() != mesh_count {
            self.builder_mutations.fetch_add(1, Ordering::AcqRel);
        }
        let root = builder.root();
        record_late_success(builder.node_mesh_binding_count(root).is_some());
        record_late_success(builder.set_space(SceneSpace::default()).is_ok());
        record_late_success(scope.scene_builder().is_ok());

        let terminal_failure = scope.format_failure(LifecycleFormatCause {
            mode: FailureEvidenceMode::Normal,
            visits: Arc::clone(&self.format_detail_callbacks),
        });
        record_late_success(builder.finish_draft().is_ok());
        Err(terminal_failure)
    }
}

impl LifecycleFailureImporter {
    fn returning(evidence: FailureEvidenceMode, visits: Arc<AtomicUsize>) -> Self {
        Self {
            evidence,
            return_mode: FailureReturnMode::Return,
            visits,
            cancellation: None,
        }
    }
}

impl FormatImporter for LifecycleFailureImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.lifecycle-failure").unwrap(),
            "Lifecycle Failure",
        )
        .extensions(["lifecycle-failure"])
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
        let failure = scope.format_failure(LifecycleFormatCause {
            mode: self.evidence,
            visits: Arc::clone(&self.visits),
        });
        match self.return_mode {
            FailureReturnMode::Return => Err(failure),
            FailureReturnMode::Swallow => {
                drop(failure);
                scope.scene_builder()?.finish_draft()
            }
            FailureReturnMode::CancelAfterCreate => {
                self.cancellation
                    .as_ref()
                    .expect("cancellation token")
                    .cancel();
                Err(failure)
            }
        }
    }
}

struct SequencedIo {
    domain: AuthorityDomain,
    assets: Mutex<BTreeMap<AssetPath, Vec<Arc<[u8]>>>>,
    opens: AtomicUsize,
}

impl SequencedIo {
    fn new(path: AssetPath, versions: impl IntoIterator<Item = Arc<[u8]>>) -> Self {
        Self {
            domain: AuthorityDomain::new(),
            assets: Mutex::new(BTreeMap::from([(path, versions.into_iter().collect())])),
            opens: AtomicUsize::new(0),
        }
    }

    fn open_count(&self) -> usize {
        self.opens.load(Ordering::Acquire)
    }

    fn with_asset(self, path: AssetPath, versions: impl IntoIterator<Item = Arc<[u8]>>) -> Self {
        self.assets
            .lock()
            .expect("asset mutex")
            .insert(path, versions.into_iter().collect());
        self
    }
}

impl AssetIo for SequencedIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        let index = self.opens.fetch_add(1, Ordering::AcqRel);
        let assets = self.assets.lock().expect("asset mutex");
        let versions = assets.get(path).ok_or_else(|| {
            AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::NotFound)
        })?;
        let bytes = versions
            .get(index.min(versions.len().saturating_sub(1)))
            .cloned()
            .ok_or_else(|| {
                AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::NotFound)
            })?;
        Ok(OpenedAsset::new(
            self.domain.clone(),
            path.clone(),
            Cursor::new(Arc::clone(&bytes)),
            Some(AssetLength::Exact(bytes.len() as u64)),
        ))
    }
}

struct PartialThenInvalidReader {
    returned_prefix: bool,
}

impl AssetReader for PartialThenInvalidReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        if self.returned_prefix {
            return Err(AssetReadError::new(AssetReadErrorKind::InvalidData));
        }

        let prefix = b"old";
        buffer[..prefix.len()].copy_from_slice(prefix);
        self.returned_prefix = true;
        Ok(prefix.len())
    }
}

struct PartialDependencyReadIo {
    domain: AuthorityDomain,
    primary: AssetPath,
    dependency: AssetPath,
    opens: AtomicUsize,
    dependency_opens: AtomicUsize,
}

impl PartialDependencyReadIo {
    fn open_count(&self) -> usize {
        self.opens.load(Ordering::Acquire)
    }
}

impl AssetIo for PartialDependencyReadIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.opens.fetch_add(1, Ordering::AcqRel);
        if path == &self.primary {
            return Ok(OpenedAsset::new(
                self.domain.clone(),
                self.primary.clone(),
                Cursor::new(&b"primary"[..]),
                Some(AssetLength::Exact(7)),
            ));
        }

        assert_eq!(path, &self.dependency);
        let dependency_open = self.dependency_opens.fetch_add(1, Ordering::AcqRel);
        if dependency_open == 0 {
            Ok(OpenedAsset::new(
                self.domain.clone(),
                self.dependency.clone(),
                PartialThenInvalidReader {
                    returned_prefix: false,
                },
                Some(AssetLength::Exact(8)),
            ))
        } else {
            Ok(OpenedAsset::new(
                self.domain.clone(),
                self.dependency.clone(),
                Cursor::new(&b"changed!"[..]),
                Some(AssetLength::Exact(8)),
            ))
        }
    }
}

struct SingleAuthorityQueryIo {
    domain: AuthorityDomain,
    assets: BTreeMap<AssetPath, Arc<[u8]>>,
    authority_queries: AtomicUsize,
    opens: AtomicUsize,
}

impl SingleAuthorityQueryIo {
    fn new(assets: impl IntoIterator<Item = (AssetPath, Arc<[u8]>)>) -> Self {
        Self {
            domain: AuthorityDomain::new(),
            assets: assets.into_iter().collect(),
            authority_queries: AtomicUsize::new(0),
            opens: AtomicUsize::new(0),
        }
    }
}

impl AssetIo for SingleAuthorityQueryIo {
    fn authority_domain(&self) -> AuthorityDomain {
        let query = self.authority_queries.fetch_add(1, Ordering::AcqRel);
        assert_eq!(
            query, 0,
            "authority domain must be frozen by primary binding"
        );
        self.domain.clone()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.opens.fetch_add(1, Ordering::AcqRel);
        let bytes = self.assets.get(path).cloned().ok_or_else(|| {
            AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::NotFound)
        })?;
        Ok(OpenedAsset::new(
            self.domain.clone(),
            path.clone(),
            Cursor::new(Arc::clone(&bytes)),
            Some(AssetLength::Exact(bytes.len() as u64)),
        ))
    }
}

struct RecordingReader {
    bytes: Arc<[u8]>,
    offset: usize,
    requests: Arc<Mutex<Vec<usize>>>,
    observed: Arc<AtomicUsize>,
}

impl AssetReader for RecordingReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        self.requests
            .lock()
            .expect("read request mutex")
            .push(buffer.len());
        let remaining = &self.bytes[self.offset..];
        let count = remaining.len().min(buffer.len());
        buffer[..count].copy_from_slice(&remaining[..count]);
        self.offset += count;
        self.observed.fetch_add(count, Ordering::AcqRel);
        Ok(count)
    }
}

struct VariableLengthIo {
    domain: AuthorityDomain,
    source: AssetPath,
    bytes: Arc<[u8]>,
    length: Option<AssetLength>,
    requests: Arc<Mutex<Vec<usize>>>,
    observed: Arc<AtomicUsize>,
}

impl AssetIo for VariableLengthIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        assert_eq!(path, &self.source);
        Ok(OpenedAsset::new(
            self.domain.clone(),
            self.source.clone(),
            RecordingReader {
                bytes: Arc::clone(&self.bytes),
                offset: 0,
                requests: Arc::clone(&self.requests),
                observed: Arc::clone(&self.observed),
            },
            self.length,
        ))
    }
}

fn request_for(importer: impl FormatImporter) -> PreparedImportRequest {
    request_for_with_config(importer, ImportConfigOverlay::default())
}

fn request_for_with_config(
    importer: impl FormatImporter,
    config: ImportConfigOverlay,
) -> PreparedImportRequest {
    let mut builder = RegistryBuilder::new();
    builder.register(importer).unwrap();
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(config),
            ),
            OptionCompileLimits::default(),
        )
        .unwrap()
}

fn lossy_request_for(importer: impl FormatImporter) -> PreparedImportRequest {
    request_for_with_config(
        importer,
        ImportConfigOverlay::default().with_loss_policy(
            LossPolicy::preserve_or_fail().allow(ImportLossKind::OptionalDependencyUnavailable),
        ),
    )
}

fn request_for_with_limits(
    importer: impl FormatImporter,
    limits: ResourceLimits,
) -> PreparedImportRequest {
    request_for_with_config(
        importer,
        ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits)),
    )
}

fn lossy_request_for_with_limits(
    importer: impl FormatImporter,
    limits: ResourceLimits,
) -> PreparedImportRequest {
    request_for_with_config(
        importer,
        ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
            .with_loss_policy(
                LossPolicy::preserve_or_fail().allow(ImportLossKind::OptionalDependencyUnavailable),
            ),
    )
}

fn request_for_with_selection(
    importer: impl FormatImporter,
    selection: FormatSelection,
) -> PreparedImportRequest {
    request_for_with_config(
        importer,
        ImportConfigOverlay::default().with_format_selection(selection),
    )
}

fn request_for_with_detection(
    importer: impl FormatImporter,
    detection: baozi_import::DetectionOptions,
) -> PreparedImportRequest {
    request_for_with_config(
        importer,
        ImportConfigOverlay::default().with_detection_options(detection),
    )
}

fn request_for_all(
    importers: impl IntoIterator<Item = RecordingImporter>,
) -> PreparedImportRequest {
    let mut builder = RegistryBuilder::new();
    for importer in importers {
        builder.register(importer).unwrap();
    }
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    snapshot
        .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
        .unwrap()
}

#[test]
fn primary_is_opened_once_and_probe_and_parser_share_the_first_snapshot() {
    let source = AssetPath::new("asset.once").unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut importer = RecordingImporter::plain("dev.baozi.test.once", "once");
    importer.primary_views = Arc::clone(&observed);
    let request = request_for(importer);
    let io = SequencedIo::new(
        source.clone(),
        [Arc::from(&b"first"[..]), Arc::from(&b"second"[..])],
    );

    let report = import_asset(&request, &io, source, ImportOperationControl::default()).unwrap();

    assert_eq!(io.open_count(), 1);
    assert_eq!(
        &*observed.lock().unwrap(),
        &[b"first".to_vec(), b"first".to_vec()]
    );
    assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn unhinted_maybe_retains_fault_and_inconclusive_selection_evidence() {
    let source = AssetPath::new("asset.unknown").unwrap();
    let mut maybe = RecordingImporter::plain("dev.baozi.test.maybe", "maybe");
    maybe.detection = DetectionOutcome::Match(ReadConfidence::Maybe);
    let mut fault = RecordingImporter::plain("dev.baozi.test.fault", "fault");
    fault.detection = DetectionOutcome::fault("detector failed");
    let mut inconclusive = RecordingImporter::plain("dev.baozi.test.inconclusive", "inconclusive");
    inconclusive.detection = DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted);
    let request = request_for_all([maybe, fault, inconclusive]);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let failure = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect_err("an unhinted Maybe candidate must not be selected");

    assert!(matches!(
        failure.cause(),
        ImportCause::Detection(DetectionFailure::NoSelection {
            had_match: true,
            had_inconclusive: true,
            had_fault: true,
        })
    ));
}

#[test]
fn primary_authority_is_frozen_for_diagnostics_and_same_authority_dependencies() {
    let source = AssetPath::new("asset.authority").unwrap();
    let dependency = AssetPath::new("missing.bin").unwrap();
    let role = DependencyRoleId::new("buffer.required").unwrap();
    let importer = RecordingImporter::plain("dev.baozi.test.authority", "authority")
        .with_dependency(
            role,
            DependencyRequirement::Required,
            DependencyBehavior::RequiredAvailableWithDiagnostic,
        );
    let request = request_for(importer);
    let io = SingleAuthorityQueryIo::new([
        (source.clone(), Arc::from(&b"primary"[..])),
        (dependency, Arc::from(&b"dependency"[..])),
    ]);

    let report = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect("one primary binding must authorize the whole session");

    assert_eq!(io.authority_queries.load(Ordering::Acquire), 1);
    assert_eq!(io.opens.load(Ordering::Acquire), 2);
    assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
}

#[test]
fn variable_length_reads_are_admitted_before_backend_entry_and_retain_consumption() {
    for (max_primary_asset_bytes, max_total_asset_bytes) in [(4, 1024), (10, 4)] {
        for length in [None, Some(AssetLength::Hint(1024))] {
            let source = AssetPath::new("asset.variable-budget").unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::new(AtomicUsize::new(0));
            let io = VariableLengthIo {
                domain: AuthorityDomain::new(),
                source: source.clone(),
                bytes: Arc::from(&b"ten bytes!"[..]),
                length,
                requests: Arc::clone(&requests),
                observed: Arc::clone(&observed),
            };
            let mut limits = ResourceLimits::default();
            limits.max_primary_asset_bytes = max_primary_asset_bytes;
            limits.max_total_asset_bytes = max_total_asset_bytes;
            let request = request_for_with_limits(
                RecordingImporter::plain("dev.baozi.test.variable-budget", "variable-budget"),
                limits,
            );

            let failure = import_asset(&request, &io, source, ImportOperationControl::default())
                .expect_err("the bounded variable-length resource must fail closed at its limit");

            let requests = requests.lock().expect("read request mutex");
            assert!(!requests.is_empty());
            assert!(requests.iter().all(|request| *request <= 4));
            assert_eq!(requests.iter().sum::<usize>(), 4);
            assert_eq!(observed.load(Ordering::Acquire), 4);
            let primary = failure
                .ledger()
                .dimension(ImportDimension::PrimaryBytes)
                .unwrap();
            assert_eq!(primary.attempted(), 4);
            assert_eq!(primary.consumed(), 4);
        }
    }
}

#[test]
fn direct_bytes_uses_one_admitted_snapshot_for_detection_and_parsing() {
    let source = AssetPath::new("asset.direct").unwrap();
    let importer = RecordingImporter::plain("dev.baozi.test.direct", "direct");
    let addresses = Arc::clone(&importer.primary_addresses);
    let request = request_for(importer);
    let input = b"direct primary bytes";
    let input_address = input.as_ptr() as usize;

    let report = import_bytes(&request, source, input, ImportOperationControl::default())
        .expect("direct bytes import");

    let addresses = addresses.lock().unwrap();
    assert_eq!(addresses.len(), 2);
    assert_eq!(addresses[0], addresses[1]);
    assert_ne!(addresses[0], input_address);
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .attempted(),
        0
    );
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn direct_bytes_rejects_primary_and_live_limits_before_copying() {
    let input = b"direct primary bytes";
    for (max_primary_asset_bytes, max_live_bytes) in [
        (
            (input.len() - 1) as u64,
            ResourceLimits::default().max_live_bytes,
        ),
        (1024, (input.len() - 1) as u64),
    ] {
        let source = AssetPath::new("asset.direct-limit").unwrap();
        let importer = RecordingImporter::plain("dev.baozi.test.direct-limit", "direct-limit");
        let addresses = Arc::clone(&importer.primary_addresses);
        let mut limits = ResourceLimits::default();
        limits.max_primary_asset_bytes = max_primary_asset_bytes;
        limits.max_live_bytes = max_live_bytes;
        let request = request_for_with_limits(importer, limits);

        let failure = import_bytes(&request, source, input, ImportOperationControl::default())
            .expect_err("direct snapshot admission must fail");

        assert!(matches!(failure.cause(), ImportCause::Scope(_)));
        assert!(addresses.lock().unwrap().is_empty());
        let primary = failure
            .ledger()
            .dimension(ImportDimension::PrimaryBytes)
            .unwrap();
        assert_eq!(primary.consumed(), 0);
        assert_eq!(primary.peak_live_bytes(), 0);
        assert_eq!(
            failure.last_completed().phase(),
            ImportPhase::SessionCreated
        );
    }
}

#[test]
fn swallowed_required_scope_abort_still_forces_terminal_failure() {
    let source = AssetPath::new("asset.req").unwrap();
    let role = DependencyRoleId::new("buffer.required").unwrap();
    let importer = RecordingImporter::plain("dev.baozi.test.required", "req").with_dependency(
        role,
        DependencyRequirement::Required,
        DependencyBehavior::RequiredCaught,
    );
    let request = request_for(importer);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let failure = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect_err("latched abort must beat parser success");

    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn terminal_latch_rejects_every_late_capability_without_side_effects() {
    for trigger in [
        TerminalLatchTrigger::MissingRequiredDependency,
        TerminalLatchTrigger::ResourceLimit,
        TerminalLatchTrigger::Cancellation,
    ] {
        let required_role = DependencyRoleId::new("buffer.required").unwrap();
        let optional_role = DependencyRoleId::new("material.optional").unwrap();
        let cancellation = CancellationToken::new();
        let late_successes = Arc::new(AtomicUsize::new(0));
        let diagnostic_callbacks = Arc::new(AtomicUsize::new(0));
        let format_detail_callbacks = Arc::new(AtomicUsize::new(0));
        let builder_mutations = Arc::new(AtomicUsize::new(0));
        let importer = TerminalLatchImporter {
            trigger,
            required_role,
            optional_role,
            cancellation: cancellation.clone(),
            late_successes: Arc::clone(&late_successes),
            diagnostic_callbacks: Arc::clone(&diagnostic_callbacks),
            format_detail_callbacks: Arc::clone(&format_detail_callbacks),
            builder_mutations: Arc::clone(&builder_mutations),
        };
        let request = lossy_request_for(importer);
        let source = AssetPath::new("asset.terminal-latch").unwrap();
        let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
        let control = match trigger {
            TerminalLatchTrigger::Cancellation => {
                ImportOperationControl::default().with_cancellation(cancellation)
            }
            TerminalLatchTrigger::MissingRequiredDependency
            | TerminalLatchTrigger::ResourceLimit => ImportOperationControl::default(),
        };

        let failure = import_asset(&request, &io, source, control)
            .expect_err("the first scope abort must remain terminal");

        let ImportCause::Scope(abort) = failure.cause() else {
            panic!("{trigger:?}: expected a scope abort")
        };
        match trigger {
            TerminalLatchTrigger::MissingRequiredDependency => {
                assert!(matches!(abort.cause(), ScopeAbortCause::AssetOpen(_)));
                assert_eq!(failure.cause().owner(), CauseOwner::Io);
            }
            TerminalLatchTrigger::ResourceLimit => {
                assert!(matches!(
                    abort.cause(),
                    ScopeAbortCause::ResourceLimit {
                        resource: "terminal-latch-trigger",
                        limit: 1,
                    }
                ));
                assert_eq!(failure.cause().owner(), CauseOwner::Import);
            }
            TerminalLatchTrigger::Cancellation => {
                assert!(matches!(abort.cause(), ScopeAbortCause::Control(_)));
                assert_eq!(failure.cause().owner(), CauseOwner::Operation);
            }
        }
        assert_eq!(
            failure.last_completed().phase(),
            ImportPhase::SelectionCompleted,
            "{trigger:?}"
        );
        assert_eq!(late_successes.load(Ordering::Acquire), 0, "{trigger:?}");
        assert_eq!(
            diagnostic_callbacks.load(Ordering::Acquire),
            0,
            "{trigger:?}"
        );
        assert_eq!(
            format_detail_callbacks.load(Ordering::Acquire),
            0,
            "{trigger:?}"
        );
        assert_eq!(builder_mutations.load(Ordering::Acquire), 0, "{trigger:?}");
        assert!(failure.optional_dependency_evidence().is_empty());
        assert_eq!(failure.diagnostic_summary().attempted_count(), 0);
        assert_eq!(failure.diagnostic_summary().stored_count(), 0);

        let expected_references = usize::from(matches!(
            trigger,
            TerminalLatchTrigger::MissingRequiredDependency
        )) as u64;
        let references = failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .unwrap();
        assert_eq!(references.attempted(), expected_references, "{trigger:?}");
        assert_eq!(references.consumed(), expected_references, "{trigger:?}");
        let work = failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap();
        assert_eq!(work.attempted(), 0, "{trigger:?}");
        assert_eq!(work.consumed(), 0, "{trigger:?}");
        let expected_opens = if expected_references == 0 { 1 } else { 2 };
        assert_eq!(io.open_count(), expected_opens, "{trigger:?}");
        assert_eq!(failure.ledger().current_live_bytes(), 0, "{trigger:?}");
    }
}

#[test]
fn dependency_normalization_is_admitted_before_path_growth_or_resolver_entry() {
    let raw_reference = String::from("dependency.bin");
    let source_text = format!("{}asset.normalization-admission", "a/".repeat(2_000));
    assert!(source_text.len() < AssetPath::MAX_BYTES);
    let source = AssetPath::new(source_text).unwrap();
    let role = DependencyRoleId::new("buffer.required").unwrap();
    let importer = NormalizationAdmissionImporter {
        role,
        raw_reference: raw_reference.clone(),
    };
    let mut limits = ResourceLimits::default();
    assert!(raw_reference.len() < limits.max_path_length);
    limits.max_live_bytes = 32 * 1024;
    let request = request_for_with_limits(importer, limits);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let resolver = CountingResolver::new();

    let failure = import_asset_with_resolver(
        &request,
        &io,
        source,
        &resolver,
        ImportOperationControl::default(),
    )
    .expect_err("normalization scratch must be admitted before allocation");

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected a scope abort")
    };
    let rejected_live_bytes = match abort.cause() {
        ScopeAbortCause::Ledger(baozi_import::LedgerCause::LimitExceeded {
            dimension: ImportDimension::ScratchBytes,
            value,
            ..
        }) => *value,
        cause => panic!("unexpected cause: {cause:?}"),
    };
    assert_eq!(resolver.call_count(), 0);
    assert_eq!(io.open_count(), 1, "only the primary resource may open");
    let references = failure
        .ledger()
        .dimension(ImportDimension::DependencyReference)
        .unwrap();
    assert_eq!(references.attempted(), 1);
    assert_eq!(references.consumed(), 1);
    let scratch = failure
        .ledger()
        .dimension(ImportDimension::ScratchBytes)
        .unwrap();
    assert_eq!(scratch.current_live_bytes(), 0);
    assert!(scratch.peak_live_bytes() < rejected_live_bytes);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
}

#[test]
fn optional_failure_is_cached_once_but_charged_for_each_logical_reference() {
    let source = AssetPath::new("asset.opt").unwrap();
    let role = DependencyRoleId::new("material.library").unwrap();
    let importer = RecordingImporter::plain("dev.baozi.test.optional", "opt").with_dependency(
        role,
        DependencyRequirement::Optional,
        DependencyBehavior::OptionalTwice,
    );
    let request = lossy_request_for(importer);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let report = import_asset(&request, &io, source, ImportOperationControl::default()).unwrap();

    assert_eq!(
        io.open_count(),
        2,
        "primary plus one cached dependency attempt"
    );
    let references = report
        .ledger()
        .dimension(ImportDimension::DependencyReference)
        .unwrap();
    assert_eq!(references.attempted(), 2);
    assert_eq!(references.consumed(), 2);
    assert_eq!(report.diagnostic_summary().stored_count(), 2);
    assert_eq!(report.optional_dependency_evidence().len(), 2);
    assert_eq!(
        report.optional_dependency_evidence()[0].cache_disposition(),
        baozi_import::DependencyCacheDisposition::Fresh
    );
    assert_eq!(
        report.optional_dependency_evidence()[1].cache_disposition(),
        baozi_import::DependencyCacheDisposition::Reused
    );
}

#[test]
fn successful_dependency_snapshot_is_shared_and_reused_without_reopening() {
    let source = AssetPath::new("asset.dependency-cache").unwrap();
    let dependency = AssetPath::new("missing.bin").unwrap();
    let role = DependencyRoleId::new("material.library").unwrap();
    let importer = RecordingImporter::plain("dev.baozi.test.dependency-cache", "dep-cache")
        .with_dependency(
            role,
            DependencyRequirement::Required,
            DependencyBehavior::RequiredAvailableTwice,
        );
    let dependency_views = Arc::clone(&importer.dependency_views);
    let dependency_dispositions = Arc::clone(&importer.dependency_dispositions);
    let mut limits = ResourceLimits::default();
    limits.max_live_bytes = 128 * 1024;
    let request = request_for_with_limits(importer, limits);
    let dependency_bytes = Arc::<[u8]>::from(vec![0x5a; 64 * 1024]);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]).with_asset(
        dependency,
        [
            Arc::from(&b"unused"[..]),
            Arc::clone(&dependency_bytes),
            Arc::from(vec![0xa5; 64 * 1024]),
        ],
    );

    let report = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect("two references must share the first admitted dependency snapshot");

    assert_eq!(io.open_count(), 2, "primary plus one dependency open");
    assert_eq!(
        &*dependency_views.lock().expect("recording mutex"),
        &[dependency_bytes.to_vec(), dependency_bytes.to_vec()]
    );
    assert_eq!(
        &*dependency_dispositions.lock().expect("recording mutex"),
        &[
            baozi_import::DependencyCacheDisposition::Fresh,
            baozi_import::DependencyCacheDisposition::Reused,
        ]
    );
    let references = report
        .ledger()
        .dimension(ImportDimension::DependencyReference)
        .unwrap();
    assert_eq!(references.attempted(), 2);
    assert_eq!(references.consumed(), 2);
    let dependency = report
        .ledger()
        .dimension(ImportDimension::DependencyBytes)
        .unwrap();
    assert_eq!(dependency.consumed(), 64 * 1024);
}

#[test]
fn optional_snapshot_exposure_limit_is_cached_after_one_physical_read() {
    let source = AssetPath::new("asset.optional-snapshot").unwrap();
    let dependency = AssetPath::new("missing.bin").unwrap();
    let role = DependencyRoleId::new("material.library").unwrap();
    let importer =
        RecordingImporter::plain("dev.baozi.test.optional-snapshot", "optional-snapshot")
            .with_dependency(
                role,
                DependencyRequirement::Optional,
                DependencyBehavior::OptionalTwice,
            );
    let mut limits = ResourceLimits::default();
    // Admit the complete 64 KiB read, then reject the final shared-snapshot
    // exposure reservation. The consumed-byte assertion below guards this seam.
    limits.max_live_bytes = 65_976;
    let request = lossy_request_for_with_limits(importer, limits);
    let dependency_bytes = Arc::<[u8]>::from(vec![0x5a; 64 * 1024]);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])])
        .with_asset(dependency, [dependency_bytes]);

    let report = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect("optional snapshot exposure failure must remain reusable");

    assert_eq!(io.open_count(), 2, "dependency bytes are read only once");
    assert_eq!(report.optional_dependency_evidence().len(), 2);
    assert_eq!(
        report.optional_dependency_evidence()[0].cache_disposition(),
        baozi_import::DependencyCacheDisposition::Fresh
    );
    assert_eq!(
        report.optional_dependency_evidence()[1].cache_disposition(),
        baozi_import::DependencyCacheDisposition::Reused
    );
    let dependency = report
        .ledger()
        .dimension(ImportDimension::DependencyBytes)
        .unwrap();
    assert_eq!(dependency.consumed(), 64 * 1024);
}

#[test]
fn optional_partial_read_failure_is_cached_with_physical_byte_accounting() {
    let source = AssetPath::new("asset.optional-partial-read").unwrap();
    let dependency = AssetPath::new("missing.bin").unwrap();
    let role = DependencyRoleId::new("material.library").unwrap();
    let importer = RecordingImporter::plain(
        "dev.baozi.test.optional-partial-read",
        "optional-partial-read",
    )
    .with_dependency(
        role,
        DependencyRequirement::Optional,
        DependencyBehavior::OptionalTwice,
    );
    let request = lossy_request_for(importer);
    let io = PartialDependencyReadIo {
        domain: AuthorityDomain::new(),
        primary: source.clone(),
        dependency,
        opens: AtomicUsize::new(0),
        dependency_opens: AtomicUsize::new(0),
    };

    let report = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect("optional read failure must be retained and reused");

    assert_eq!(io.open_count(), 2, "primary plus one dependency open");
    assert_eq!(report.optional_dependency_evidence().len(), 2);
    assert_eq!(
        report.optional_dependency_evidence()[0].cache_disposition(),
        baozi_import::DependencyCacheDisposition::Fresh
    );
    assert_eq!(
        report.optional_dependency_evidence()[1].cache_disposition(),
        baozi_import::DependencyCacheDisposition::Reused
    );
    let references = report
        .ledger()
        .dimension(ImportDimension::DependencyReference)
        .unwrap();
    assert_eq!(references.attempted(), 2);
    assert_eq!(references.consumed(), 2);
    let dependency_bytes = report
        .ledger()
        .dimension(ImportDimension::DependencyBytes)
        .unwrap();
    assert_eq!(dependency_bytes.attempted(), 8);
    assert_eq!(dependency_bytes.consumed(), 3);
}

#[test]
fn optional_dependency_loss_requires_explicit_caller_authority() {
    let source = AssetPath::new("asset.optional-policy").unwrap();
    let role = DependencyRoleId::new("material.library").unwrap();
    let importer = RecordingImporter::plain("dev.baozi.test.optional-policy", "optional-policy")
        .with_dependency(
            role,
            DependencyRequirement::Optional,
            DependencyBehavior::OptionalTwice,
        );

    let preserving_request = request_for(importer.clone());
    let preserving_io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let failure = import_asset(
        &preserving_request,
        &preserving_io,
        source.clone(),
        ImportOperationControl::default(),
    )
    .expect_err("default policy must preserve optional content or fail");
    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
    assert!(failure.optional_dependency_evidence().is_empty());

    let lossy_request = lossy_request_for(importer);
    let lossy_io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let report = import_asset(
        &lossy_request,
        &lossy_io,
        source,
        ImportOperationControl::default(),
    )
    .expect("explicit caller authority permits unavailable optional content");
    assert_eq!(report.optional_dependency_evidence().len(), 2);
}

#[test]
fn format_failure_retains_stable_packed_evidence_and_owner() {
    let source = AssetPath::new("asset.failure").unwrap();
    let observation = Arc::new(FormatFailureProducerObservation::default());
    let request = request_for(FormatFailingImporter {
        observation: Arc::clone(&observation),
    });
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let failure = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect_err("format failure must be retained");
    drop(request);
    drop(io);

    let ImportCause::Format(envelope) = failure.cause() else {
        panic!("expected format envelope")
    };
    assert_eq!(envelope.code(), "test.format_failure");
    assert!(matches!(
        envelope.details().get(0).unwrap().value(),
        FormatFailureDetailValue::Unsigned(7)
    ));
    let detail = envelope.details().get(1).expect("copied string evidence");
    assert_eq!(detail.key(), "backend_message");
    let FormatFailureDetailValue::String(value) = detail.value() else {
        panic!("expected copied string evidence")
    };
    assert_eq!(value.as_str(), "private test backend failure");
    assert_eq!(observation.drops.load(Ordering::Acquire), 2);
    assert_ne!(
        envelope.code().as_ptr() as usize,
        observation.code_pointer.load(Ordering::Acquire)
    );
    assert_ne!(
        detail.key().as_ptr() as usize,
        observation.key_pointer.load(Ordering::Acquire)
    );
    assert_ne!(
        value.as_str().as_ptr() as usize,
        observation.value_pointer.load(Ordering::Acquire)
    );
    let evidence = Error::source(envelope).expect("host evidence must be the direct source");
    assert!(evidence.downcast_ref::<FormatFailureEvidence>().is_some());
    assert!(evidence.source().is_none());
    assert!(std::ptr::eq(
        evidence.downcast_ref::<FormatFailureEvidence>().unwrap(),
        envelope.evidence()
    ));
    assert!(!format!("{envelope:?}").contains("TestBackendCause"));
    assert!(
        !envelope
            .to_string()
            .contains("private test backend failure")
    );
    assert_eq!(envelope.code(), "test.format_failure");
    assert_eq!(envelope.details().get(1).unwrap().key(), detail.key());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn large_format_failure_producer_is_destroyed_after_bounded_evidence_capture() {
    let source = AssetPath::new("asset.oversized-failure").unwrap();
    let request = request_for(OversizedFormatFailingImporter);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let failure = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect_err("the importer deliberately returns a format failure");

    assert!(matches!(
        failure.cause(),
        ImportCause::Format(envelope)
            if envelope.code() == "test.oversized_format_failure"
    ));
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn format_failure_storage_survives_all_terminal_evidence_paths() {
    for mode in [
        FailureEvidenceMode::Normal,
        FailureEvidenceMode::CodePanics,
        FailureEvidenceMode::InvalidCode,
        FailureEvidenceMode::OverlongCode,
        FailureEvidenceMode::DetailsPanic,
        FailureEvidenceMode::DetailsRejected,
        FailureEvidenceMode::DropPanics,
    ] {
        let visits = Arc::new(AtomicUsize::new(0));
        let request = request_for(LifecycleFailureImporter::returning(
            mode,
            Arc::clone(&visits),
        ));
        let failure = import_bytes(
            &request,
            AssetPath::new("asset.lifecycle-failure").unwrap(),
            b"primary",
            ImportOperationControl::default(),
        )
        .expect_err("every lifecycle mode must fail");

        match mode {
            FailureEvidenceMode::Normal => {
                assert!(matches!(failure.cause(), ImportCause::Format(_)));
                assert_eq!(visits.load(Ordering::Acquire), 1);
            }
            FailureEvidenceMode::CodePanics
            | FailureEvidenceMode::InvalidCode
            | FailureEvidenceMode::OverlongCode => {
                assert!(matches!(failure.cause(), ImportCause::Scope(_)));
                assert_eq!(visits.load(Ordering::Acquire), 0);
            }
            FailureEvidenceMode::DetailsPanic | FailureEvidenceMode::DetailsRejected => {
                assert!(matches!(failure.cause(), ImportCause::Scope(_)));
                assert_eq!(visits.load(Ordering::Acquire), 1);
                if matches!(mode, FailureEvidenceMode::DetailsPanic) {
                    let ImportCause::Scope(abort) = failure.cause() else {
                        unreachable!()
                    };
                    assert!(matches!(
                        abort.cause(),
                        ScopeAbortCause::TrustedExtension {
                            boundary: TrustedExtensionBoundary::FormatFailureDetail,
                            code: "format-failure-detail-panicked",
                        }
                    ));
                }
            }
            FailureEvidenceMode::DropPanics => {
                let ImportCause::Scope(abort) = failure.cause() else {
                    panic!("expected a contained producer drop panic")
                };
                assert!(matches!(
                    abort.cause(),
                    ScopeAbortCause::TrustedExtension {
                        boundary: TrustedExtensionBoundary::FormatFailureDetail,
                        code: "format-failure-producer-drop-panicked",
                    }
                ));
                assert_eq!(visits.load(Ordering::Acquire), 1);
            }
        }
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert!(failure.finalization_cause().is_none());
    }
}

#[test]
fn format_failure_evidence_callbacks_cannot_reenter_parse_capabilities() {
    for phase in [
        ReentrantFormatFailurePhase::Code,
        ReentrantFormatFailurePhase::Details,
        ReentrantFormatFailurePhase::DecodedStorage,
        ReentrantFormatFailurePhase::FormattedStorage,
        ReentrantFormatFailurePhase::MeteredStorage,
        ReentrantFormatFailurePhase::ReservedStorage,
        ReentrantFormatFailurePhase::ScenePartsConversion,
        ReentrantFormatFailurePhase::NestedFormatFailure,
        ReentrantFormatFailurePhase::Drop,
    ] {
        let request = request_for(ReentrantFormatFailureImporter { phase });
        let failure = import_bytes(
            &request,
            AssetPath::new("asset.reentrant-format-failure").unwrap(),
            b"primary",
            ImportOperationControl::default(),
        )
        .expect_err("reentrant format evidence must fail closed");

        let ImportCause::Scope(abort) = failure.cause() else {
            panic!("expected a scope contract failure for {phase:?}")
        };
        assert!(matches!(
            abort.cause(),
            ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            }
        ));
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
}

#[test]
fn diagnostic_callback_panics_have_their_own_stable_boundary_and_code() {
    for mode in [DiagnosticPanicMode::Details, DiagnosticPanicMode::Message] {
        let request = request_for(DiagnosticPanickingImporter { mode });

        let failure = import_bytes(
            &request,
            AssetPath::new("asset.diagnostic-panic").unwrap(),
            b"primary",
            ImportOperationControl::default(),
        )
        .expect_err("diagnostic callback panic must be contained");

        let ImportCause::Scope(abort) = failure.cause() else {
            panic!("{mode:?}: expected a scope abort")
        };
        assert!(matches!(
            abort.cause(),
            ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::DiagnosticCallback,
                code: "diagnostic-callback-panicked",
            }
        ));
        assert_eq!(failure.ledger().current_live_bytes(), 0, "{mode:?}");
    }
}

#[test]
fn format_failure_storage_is_admitted_before_trusted_callbacks() {
    let baseline_visits = Arc::new(AtomicUsize::new(0));
    let baseline_request = request_for(LifecycleFailureImporter::returning(
        FailureEvidenceMode::Normal,
        Arc::clone(&baseline_visits),
    ));
    let source = AssetPath::new("asset.lifecycle-failure").unwrap();
    let baseline = import_bytes(
        &baseline_request,
        source.clone(),
        b"primary",
        ImportOperationControl::default(),
    )
    .expect_err("baseline format failure");
    assert_eq!(baseline_visits.load(Ordering::Acquire), 1);

    let mut limits = ResourceLimits::default();
    limits.max_live_bytes = baseline
        .ledger()
        .peak_live_bytes()
        .checked_sub(1)
        .expect("format failure admission raises the live peak");
    let rejected_visits = Arc::new(AtomicUsize::new(0));
    let rejected_request = request_for_with_limits(
        LifecycleFailureImporter::returning(
            FailureEvidenceMode::Normal,
            Arc::clone(&rejected_visits),
        ),
        limits,
    );
    let rejected = import_bytes(
        &rejected_request,
        source,
        b"primary",
        ImportOperationControl::default(),
    )
    .expect_err("one byte below the observed peak must reject admission");

    assert_eq!(rejected_visits.load(Ordering::Acquire), 0);
    assert!(matches!(rejected.cause(), ImportCause::Scope(_)));
    assert_eq!(rejected.ledger().current_live_bytes(), 0);
    assert!(rejected.finalization_cause().is_none());
}

#[test]
fn swallowed_format_failure_is_a_descriptor_contract_failure() {
    let visits = Arc::new(AtomicUsize::new(0));
    let request = request_for(LifecycleFailureImporter {
        evidence: FailureEvidenceMode::Normal,
        return_mode: FailureReturnMode::Swallow,
        visits: Arc::clone(&visits),
        cancellation: None,
    });
    let failure = import_bytes(
        &request,
        AssetPath::new("asset.lifecycle-failure").unwrap(),
        b"primary",
        ImportOperationControl::default(),
    )
    .expect_err("a parser cannot swallow a format failure capability");

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected a scope contract failure")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::DescriptorContract { code }
            if *code == "format-failure-was-swallowed"
    ));
    assert_eq!(visits.load(Ordering::Acquire), 1);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(failure.finalization_cause().is_none());
}

#[test]
fn post_parse_cancellation_releases_staged_format_failure_storage() {
    let cancellation = CancellationToken::new();
    let visits = Arc::new(AtomicUsize::new(0));
    let request = request_for(LifecycleFailureImporter {
        evidence: FailureEvidenceMode::Normal,
        return_mode: FailureReturnMode::CancelAfterCreate,
        visits: Arc::clone(&visits),
        cancellation: Some(cancellation.clone()),
    });
    let control = ImportOperationControl::default().with_cancellation(cancellation);
    let failure = import_bytes(
        &request,
        AssetPath::new("asset.lifecycle-failure").unwrap(),
        b"primary",
        control,
    )
    .expect_err("post-parse cancellation must override the returned format failure");

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected a cancellation scope failure")
    };
    assert!(matches!(abort.cause(), ScopeAbortCause::Control(_)));
    assert_eq!(visits.load(Ordering::Acquire), 1);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(failure.finalization_cause().is_none());
}

#[test]
fn failure_retains_optional_dependency_loss_evidence() {
    let source = AssetPath::new("asset.optional-failure").unwrap();
    let role = DependencyRoleId::new("material.library").unwrap();
    let importer = RecordingImporter::plain("dev.baozi.test.optional-failure", "optional-failure")
        .with_dependency(
            role,
            DependencyRequirement::Optional,
            DependencyBehavior::OptionalThenFail,
        );
    let request = lossy_request_for(importer);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let failure = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect_err("format failure after optional loss");

    assert_eq!(failure.optional_dependency_evidence().len(), 1);
    assert_eq!(failure.diagnostic_summary().stored_count(), 1);
}

fn structured_diagnostic_text_profile() -> TextProfile {
    TextProfile::builder(
        TextHardCaps::new(
            512 * 1024 * 1024,
            512 * 1024 * 1024,
            64 * 1024 * 1024,
            1_000_000_000,
            64 * 1024 * 1024,
        )
        .expect("static structured-diagnostic text hard caps must be valid"),
    )
    .source_encodings([TextSourceEncoding::Utf8])
    .bom_policy(TextBomPolicy::Forbidden)
    .newlines([TextNewline::Lf])
    .lexical_mode(TextLexicalMode::Utf8Document)
    .decoding_modes([TextDecodingMode::RejectInvalid])
    .control_policy(TextControlPolicy::AllowHorizontalTab)
    .build()
    .expect("static structured-diagnostic text profile must be valid")
}

struct StructuredDiagnosticImporter {
    diagnostic_count: usize,
}

impl StructuredDiagnosticImporter {
    const fn new(diagnostic_count: usize) -> Self {
        Self { diagnostic_count }
    }
}

impl FormatImporter for StructuredDiagnosticImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.structured-diagnostic").unwrap(),
            "Structured Diagnostic",
        )
        .extensions(["structured-diagnostic"])
        .encoding(FormatEncoding::Text)
        .text_profile(structured_diagnostic_text_profile())
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
        for record in 0..self.diagnostic_count {
            scope.emit_diagnostic_with(
                baozi_import::DiagnosticSeverity::Warning,
                baozi_import::DiagnosticCode::new("test.structured_warning").unwrap(),
                ImportProgressPhase::Decode,
                scope.diagnostic_source(),
                Some(baozi_import::DiagnosticByteRange::new(2, 4).unwrap()),
                Some("/records/0"),
                |details| {
                    details.unsigned(
                        "record",
                        u64::try_from(record).expect("bounded diagnostic count"),
                    );
                    details.string("token", "bad\nvalue");
                },
                format_args!("bad\nrecord"),
            )?;
        }
        scope.scene_builder()?.finish_draft()
    }
}

#[allow(
    clippy::result_large_err,
    reason = "the test helper returns the public terminal failure with complete bounded evidence"
)]
fn import_structured_diagnostics(
    diagnostic_count: usize,
    limits: ResourceLimits,
) -> Result<ImportReport, ImportFailure> {
    let source = AssetPath::new("asset.structured-diagnostic").unwrap();
    let request =
        request_for_with_limits(StructuredDiagnosticImporter::new(diagnostic_count), limits);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"a\nprimary"[..])]);
    import_asset(&request, &io, source, ImportOperationControl::default())
}

fn line_index_peak(report: &ImportReport) -> u64 {
    report
        .ledger()
        .dimension(ImportDimension::LineIndexBytes)
        .expect("line-index evidence")
        .peak_live_bytes()
}

#[test]
fn structured_diagnostic_retains_bounded_location_and_details() {
    let source = AssetPath::new("asset.structured-diagnostic").unwrap();
    let request = request_for(StructuredDiagnosticImporter::new(1));
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"a\nprimary"[..])]);

    let report = import_asset(&request, &io, source, ImportOperationControl::default()).unwrap();
    let diagnostic = &report.diagnostics()[0];

    assert_eq!(diagnostic.phase(), ImportProgressPhase::Decode);
    assert_eq!(diagnostic.byte_range().unwrap().start(), 2);
    assert_eq!(diagnostic.line_column().unwrap().line(), 2);
    assert_eq!(diagnostic.line_column().unwrap().column(), 1);
    assert_eq!(diagnostic.object_path(), Some("/records/0"));
    assert_eq!(diagnostic.details().len(), 2);
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::DiagnosticBytes)
            .unwrap()
            .current_live_bytes(),
        0
    );
}

#[test]
fn repeated_diagnostics_for_one_source_reuse_the_accounted_line_index() {
    let single = import_structured_diagnostics(1, ResourceLimits::default())
        .expect("single diagnostic import");
    let repeated = import_structured_diagnostics(2, ResourceLimits::default())
        .expect("repeated diagnostic import");

    assert_eq!(single.diagnostics().len(), 1);
    assert_eq!(repeated.diagnostics().len(), 2);
    assert_eq!(line_index_peak(&repeated), line_index_peak(&single));
}

#[test]
fn line_index_storage_honors_the_exact_dimension_budget() {
    let baseline = import_structured_diagnostics(1, ResourceLimits::default())
        .expect("baseline line-index import");
    let required = line_index_peak(&baseline);
    assert!(required > 0);

    let mut exact_limits = ResourceLimits::default();
    exact_limits.max_line_index_bytes = required;
    import_structured_diagnostics(1, exact_limits)
        .expect("the exact observed line-index budget must succeed");

    let mut insufficient_limits = ResourceLimits::default();
    insufficient_limits.max_line_index_bytes = required - 1;
    let failure = import_structured_diagnostics(1, insufficient_limits)
        .expect_err("one byte below the line-index requirement must fail");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                    dimension: ImportDimension::LineIndexBytes,
                    ..
                })
            )
    ));
}

#[test]
fn publication_peak_includes_output_and_retained_diagnostic_storage() {
    let source = AssetPath::new("asset.diagnostic-peak").unwrap();
    let request = request_for(StructuredDiagnosticImporter::new(1));
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"a\nprimary"[..])]);

    let report = import_asset(&request, &io, source, ImportOperationControl::default()).unwrap();
    let canonical_output = report
        .ledger()
        .dimension(ImportDimension::CanonicalOutputBytes)
        .unwrap()
        .peak_live_bytes();
    let diagnostic_payload = report
        .ledger()
        .dimension(ImportDimension::DiagnosticBytes)
        .unwrap()
        .peak_live_bytes();

    assert!(canonical_output > 0);
    assert!(diagnostic_payload > 0);
    assert!(
        report.ledger().peak_live_bytes()
            >= canonical_output
                .checked_add(diagnostic_payload)
                .expect("bounded test accounting")
    );
}

#[test]
fn oversized_detector_configuration_is_rejected_before_session_creation() {
    assert!(
        baozi_import::DetectionOptions::new(baozi_import::MAX_DETECTOR_WORK_BETWEEN_POLLS + 1)
            .is_err()
    );
}

#[test]
fn forced_detection_distinguishes_contradiction_fault_and_inconclusive() {
    let source = AssetPath::new("asset.forced").unwrap();

    let mut contradiction = RecordingImporter::plain("dev.baozi.test.forced-no", "forced");
    contradiction.detection = DetectionOutcome::NoMatch;
    let request = request_for_with_selection(
        contradiction,
        FormatSelection::Forced(FormatId::new("dev.baozi.test.forced-no").unwrap()),
    );
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let failure = import_asset(
        &request,
        &io,
        source.clone(),
        ImportOperationControl::default(),
    )
    .expect_err("forced contradiction");
    assert!(matches!(
        failure.cause(),
        ImportCause::Detection(DetectionFailure::ForcedFormatContradiction { .. })
    ));

    let mut fault = RecordingImporter::plain("dev.baozi.test.forced-fault", "forced");
    fault.detection = DetectionOutcome::fault("detector failed");
    let request = request_for_with_selection(
        fault,
        FormatSelection::Forced(FormatId::new("dev.baozi.test.forced-fault").unwrap()),
    );
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let failure = import_asset(
        &request,
        &io,
        source.clone(),
        ImportOperationControl::default(),
    )
    .expect_err("forced fault");
    assert!(matches!(
        failure.cause(),
        ImportCause::Detection(DetectionFailure::ForcedDetectorFault { .. })
    ));

    let mut inconclusive = RecordingImporter::plain("dev.baozi.test.forced-inc", "forced");
    inconclusive.detection = DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted);
    let request = request_for_with_selection(
        inconclusive,
        FormatSelection::Forced(FormatId::new("dev.baozi.test.forced-inc").unwrap()),
    );
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    assert!(import_asset(&request, &io, source, ImportOperationControl::default()).is_ok());
}

#[test]
fn automatic_detection_only_reports_unsupported_for_clean_no_match() {
    let source = AssetPath::new("asset.auto").unwrap();
    let mut clean = RecordingImporter::plain("dev.baozi.test.clean-no", "auto");
    clean.detection = DetectionOutcome::NoMatch;
    let request = request_for(clean);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let failure = import_asset(
        &request,
        &io,
        source.clone(),
        ImportOperationControl::default(),
    )
    .expect_err("clean no match");
    assert!(matches!(failure.cause(), ImportCause::UnsupportedFormat));

    let mut faulty = RecordingImporter::plain("dev.baozi.test.faulty-no", "auto");
    faulty.detection = DetectionOutcome::fault("fault");
    let request = request_for(faulty);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let failure = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect_err("fault is not unsupported");
    assert!(matches!(
        failure.cause(),
        ImportCause::Detection(DetectionFailure::NoSelection {
            had_fault: true,
            ..
        })
    ));
}

#[test]
fn parser_panic_poison_is_shared_across_snapshots() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut builder = RegistryBuilder::new();
    builder
        .register(PanickingImporter {
            calls: Arc::clone(&calls),
        })
        .unwrap();
    let snapshot_a = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let snapshot_b = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let compile = |snapshot: &baozi_import::RegistrySnapshot| {
        snapshot
            .compile_request(
                &ImportRequest::from_layers(
                    ImportRequestLayer::default(),
                    ImportRequestLayer::new(ImportConfigOverlay::default().with_format_selection(
                        FormatSelection::Forced(
                            FormatId::new("dev.baozi.test.panic-shared").unwrap(),
                        ),
                    )),
                ),
                OptionCompileLimits::default(),
            )
            .unwrap()
    };
    let request_a = compile(&snapshot_a);
    let request_b = compile(&snapshot_b);
    let source = AssetPath::new("asset.panic").unwrap();
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let panicked = import_asset(
        &request_a,
        &io,
        source.clone(),
        ImportOperationControl::default(),
    )
    .expect_err("the first parser invocation panics");
    let poisoned = import_asset(&request_b, &io, source, ImportOperationControl::default())
        .expect_err("the shared registration remains poisoned");

    assert!(matches!(
        panicked.cause(),
        ImportCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::Importer,
            code: "importer-panicked",
        }
    ));
    assert!(matches!(
        poisoned.cause(),
        ImportCause::Detection(DetectionFailure::ForcedDetectorFault { .. })
    ));
    for failure in [&panicked, &poisoned] {
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert!(failure.finalization_cause().is_none());
    }
    assert_eq!(calls.load(Ordering::Acquire), 1);
}

#[test]
fn in_flight_parse_success_is_rejected_after_a_parallel_parser_panics() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (first_entered, entered) = sync_channel(0);
    let (release, release_first) = sync_channel(0);
    let mut builder = RegistryBuilder::new();
    builder
        .register(RacingParsePoisonImporter {
            calls: Arc::clone(&calls),
            first_entered,
            release_first: Mutex::new(release_first),
        })
        .unwrap();
    let first_snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let second_snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let compile = |snapshot: &baozi_import::RegistrySnapshot| {
        snapshot
            .compile_request(
                &ImportRequest::from_layers(
                    ImportRequestLayer::default(),
                    ImportRequestLayer::new(ImportConfigOverlay::default().with_format_selection(
                        FormatSelection::Forced(
                            FormatId::new("dev.baozi.test.racing-parse-poison").unwrap(),
                        ),
                    )),
                ),
                OptionCompileLimits::default(),
            )
            .unwrap()
    };
    let first_request = compile(&first_snapshot);
    let second_request = compile(&second_snapshot);

    let first = std::thread::spawn(move || {
        import_bytes(
            &first_request,
            AssetPath::new("first.racing-parse-poison").unwrap(),
            b"primary",
            ImportOperationControl::default(),
        )
        .expect_err("the in-flight successful parse must be discarded after shared poison")
    });
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("the first parser reaches the controlled boundary");
    let panicked = import_bytes(
        &second_request,
        AssetPath::new("second.racing-parse-poison").unwrap(),
        b"primary",
        ImportOperationControl::default(),
    )
    .expect_err("the parallel parser panics");
    release
        .send(())
        .expect("the first parser still waits for release");
    let poisoned = first
        .join()
        .expect("the first import thread remains contained");

    assert!(matches!(
        panicked.cause(),
        ImportCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::Importer,
            code: "importer-panicked",
        }
    ));
    assert!(matches!(
        poisoned.cause(),
        ImportCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::Importer,
            code: "importer-poisoned",
        }
    ));
    for failure in [&panicked, &poisoned] {
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert!(failure.finalization_cause().is_none());
    }
    assert_eq!(calls.load(Ordering::Acquire), 2);
}

#[test]
fn parser_panic_does_not_run_an_untrusted_payload_destructor() {
    let payload_drops = Arc::new(AtomicUsize::new(0));
    let request = request_for(OpaquePanickingImporter {
        payload_drops: Arc::clone(&payload_drops),
    });
    let failure = import_bytes(
        &request,
        AssetPath::new("asset.opaque-panic").unwrap(),
        b"primary",
        ImportOperationControl::default(),
    )
    .expect_err("opaque parser panic must be contained");

    assert!(matches!(
        failure.cause(),
        ImportCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::Importer,
            code: "importer-panicked",
        }
    ));
    assert_eq!(payload_drops.load(Ordering::Acquire), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(failure.finalization_cause().is_none());
}

#[test]
fn asset_io_panics_do_not_run_untrusted_payload_destructors() {
    let request = request_for(RecordingImporter::plain(
        "dev.baozi.test.opaque-asset-io",
        "opaque-asset-io",
    ));

    for (boundary, expected_code) in [
        (
            OpaqueAssetIoPanicBoundary::Authority,
            "primary-authority-panicked",
        ),
        (OpaqueAssetIoPanicBoundary::Open, "primary-open-panicked"),
        (OpaqueAssetIoPanicBoundary::Read, "asset-read-panicked"),
    ] {
        let source = AssetPath::new("asset.opaque-asset-io").unwrap();
        let payload_drops = Arc::new(AtomicUsize::new(0));
        let io = OpaquePanickingAssetIo {
            boundary,
            domain: AuthorityDomain::new(),
            source: source.clone(),
            payload_drops: Arc::clone(&payload_drops),
        };

        let failure = import_asset(&request, &io, source, ImportOperationControl::default())
            .expect_err("opaque AssetIo panic must be contained");
        let matched_boundary = match failure.cause() {
            ImportCause::TrustedExtension { boundary, code } => {
                *boundary == TrustedExtensionBoundary::AssetIo && *code == expected_code
            }
            ImportCause::Scope(abort) => matches!(
                abort.cause(),
                ScopeAbortCause::TrustedExtension { boundary, code }
                    if *boundary == TrustedExtensionBoundary::AssetIo && *code == expected_code
            ),
            _ => false,
        };

        assert!(matched_boundary, "{boundary:?}: {:#?}", failure.cause());
        assert_eq!(payload_drops.load(Ordering::Acquire), 0, "{boundary:?}");
        assert_eq!(failure.ledger().current_live_bytes(), 0, "{boundary:?}");
    }
}

#[test]
fn cancellation_before_primary_open_creates_failure_evidence_without_io() {
    let source = AssetPath::new("asset.cancel").unwrap();
    let request = request_for(RecordingImporter::plain("dev.baozi.test.cancel", "cancel"));
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let token = CancellationToken::new();
    token.cancel();
    let control = ImportOperationControl::default().with_cancellation(token);

    let failure = import_asset(&request, &io, source, control).expect_err("cancelled");

    assert_eq!(io.open_count(), 0);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SessionCreated
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn detector_work_is_probe_bounded_and_post_detector_cancellation_prevents_selection() {
    let source = AssetPath::new("asset.scan").unwrap();
    let token = CancellationToken::new();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut importer = RecordingImporter::plain("dev.baozi.test.scan", "scan");
    importer.primary_views = Arc::clone(&observed);
    importer.detector_cancel = Some(token.clone());
    let request =
        request_for_with_detection(importer, baozi_import::DetectionOptions::new(8).unwrap());
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"0123456789abcdef"[..])]);
    let control = ImportOperationControl::default().with_cancellation(token);

    let failure = import_asset(&request, &io, source, control).expect_err("cancelled after scan");

    assert_eq!(observed.lock().unwrap()[0].len(), 8);
    assert_eq!(failure.last_completed().phase(), ImportPhase::PrimaryBound);
    assert!(failure.selected_format().is_none());
}

#[test]
fn observer_panic_is_contained_and_releases_primary_snapshot() {
    let source = AssetPath::new("asset.observer").unwrap();
    let request = request_for(RecordingImporter::plain(
        "dev.baozi.test.observer",
        "observer",
    ));
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);
    let control = ImportOperationControl::default()
        .with_progress_observer(|_| -> ProgressDecision { panic!("observer panic") });

    let failure = import_asset(&request, &io, source, control).expect_err("observer panic");

    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn progress_observer_can_run_a_nested_import_without_lock_reentry() {
    let nested_source = AssetPath::new("asset.nested-observer").unwrap();
    let nested_request = Arc::new(request_for(RecordingImporter::plain(
        "dev.baozi.test.nested-observer",
        "nested-observer",
    )));
    let nested_io = Arc::new(SequencedIo::new(
        nested_source.clone(),
        [Arc::from(&b"nested"[..])],
    ));
    let observer_calls = Arc::new(AtomicUsize::new(0));
    let nested_successes = Arc::new(AtomicUsize::new(0));
    let observed_calls = Arc::clone(&observer_calls);
    let observed_successes = Arc::clone(&nested_successes);
    let control = ImportOperationControl::default().with_progress_observer(move |_| {
        if observed_calls.fetch_add(1, Ordering::AcqRel) == 0
            && import_asset(
                &nested_request,
                nested_io.as_ref(),
                nested_source.clone(),
                ImportOperationControl::default(),
            )
            .is_ok()
        {
            observed_successes.fetch_add(1, Ordering::AcqRel);
        }
        ProgressDecision::Continue
    });
    let source = AssetPath::new("asset.outer-observer").unwrap();
    let request = request_for(RecordingImporter::plain(
        "dev.baozi.test.outer-observer",
        "outer-observer",
    ));
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"outer"[..])]);

    let report = import_asset(&request, &io, source, control).expect("outer import");

    assert!(observer_calls.load(Ordering::Acquire) > 0);
    assert_eq!(nested_successes.load(Ordering::Acquire), 1);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn final_observer_cancellation_rolls_back_publication_exactly_once() {
    let final_events = Arc::new(AtomicUsize::new(0));
    let observed_final_events = Arc::clone(&final_events);
    let control = ImportOperationControl::default().with_progress_observer(
        move |event: baozi_operation::ProgressEvent<ImportProgressPhase, ImportProgressUnit>| {
            if event.phase() == ImportProgressPhase::Finalize {
                observed_final_events.fetch_add(1, Ordering::AcqRel);
                ProgressDecision::Cancel
            } else {
                ProgressDecision::Continue
            }
        },
    );
    let source = AssetPath::new("asset.final-cancel").unwrap();
    let request = request_for(RecordingImporter::plain(
        "dev.baozi.test.final-cancel",
        "final-cancel",
    ));
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let failure = import_asset(&request, &io, source, control)
        .expect_err("final observer cancellation must roll back publication");

    let output = failure
        .ledger()
        .dimension(ImportDimension::CanonicalOutputBytes)
        .unwrap();
    assert_eq!(final_events.load(Ordering::Acquire), 1);
    assert_eq!(failure.last_completed().phase(), ImportPhase::Validated);
    assert_eq!(output.committed(), 0);
    assert_eq!(output.pending_publication(), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(failure.finalization_cause().is_none());
}

#[test]
fn diagnostic_storage_caps_drop_excess_without_changing_success() {
    let source = AssetPath::new("asset.diag").unwrap();
    let role = DependencyRoleId::new("material.library").unwrap();
    let importer = RecordingImporter::plain("dev.baozi.test.diag", "diag").with_dependency(
        role,
        DependencyRequirement::Optional,
        DependencyBehavior::OptionalTwice,
    );
    let mut limits = ResourceLimits::default();
    limits.max_diagnostics = 1;
    let request = lossy_request_for_with_limits(importer, limits);
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let report = import_asset(&request, &io, source, ImportOperationControl::default()).unwrap();

    assert_eq!(report.diagnostic_summary().attempted_count(), 2);
    assert_eq!(report.diagnostic_summary().stored_count(), 1);
    assert_eq!(report.diagnostic_summary().dropped_count(), 1);
    assert_eq!(report.optional_dependency_evidence().len(), 2);
}

#[test]
fn scoped_builder_reservation_transitions_to_committed_output() {
    let source = AssetPath::new("asset.builder-budget").unwrap();
    let request = request_for(RecordingImporter::plain(
        "dev.baozi.test.builder-budget",
        "builder-budget",
    ));
    let io = SequencedIo::new(source.clone(), [Arc::from(&b"primary"[..])]);

    let report = import_asset(&request, &io, source, ImportOperationControl::default()).unwrap();
    let output = report
        .ledger()
        .dimension(ImportDimension::CanonicalOutputBytes)
        .expect("canonical output accounting");

    assert!(output.peak_live_bytes() > 0);
    assert!(output.committed() > 0);
    assert_eq!(output.current_live_bytes(), 0);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}
