mod support;

use baozi_core::{FormatId, SCENE_VALIDATION_POLL_INTERVAL};
use baozi_import::{
    CapabilityStatus, CauseOwner, DependencyRequirement, DependencyRoleId, DetectionAbort,
    DetectionFaultCode, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, ImportCause, ImportDimension, ImportFailure,
    ImportOperationControl, ImportPhase, ImportProgressPhase, ImportProgressUnit, ImportReport,
    ImportRequest, ImportResult, OptionCompileLimits, ParseResult, ParseScope,
    PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    SceneDraft, ScopeAbortCause, import_asset as run_import_asset,
    import_bytes as run_import_bytes,
};
use baozi_io::{
    AssetIo, AssetLength, AssetOpenError, AssetPath, AssetReadError, AssetReader, AuthorityDomain,
    OpenedAsset,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, CancellationToken, ControlCause, ProgressDecision,
};
use std::cell::RefCell;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const READ_CHUNK_BYTES: usize = 16 * 1024;
const MULTI_CHUNK_BYTES: usize = READ_CHUNK_BYTES * 3;
const VALIDATION_NODE_COUNT: usize = SCENE_VALIDATION_POLL_INTERVAL as usize * 2;
const CANCEL_DURING_VALIDATION_POLL: usize = 12;
const DETECTOR_WORK_CHUNK_BYTES: usize = 4;
const DETECTOR_SCAN_BYTES: &[u8] = b"12345678";

#[allow(
    clippy::result_large_err,
    reason = "operation-only test helper preserves complete bounded terminal import evidence"
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
    reason = "cancellation stage assertions inspect complete bounded import failure evidence"
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
    reason = "cancellation stage assertions inspect complete bounded import failure evidence"
)]
fn import_bytes(
    request: &PreparedImportRequest,
    source: AssetPath,
    bytes: &[u8],
    control: ImportOperationControl,
) -> Result<ImportReport, ImportFailure> {
    operation_result(run_import_bytes(request, source, bytes, control))
}

fn descriptor(id: &str, name: &str, extension: &str) -> FormatDescriptor {
    FormatDescriptor::builder(
        FormatId::new(id).expect("qualified cancellation stage format id"),
        name,
    )
    .extensions([extension])
    .encoding(FormatEncoding::Binary)
    .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
    .build()
    .expect("valid binary cancellation stage descriptor")
}

struct EmptyImporter {
    id: &'static str,
    name: &'static str,
    extension: &'static str,
}

impl FormatImporter for EmptyImporter {
    fn descriptor(&self) -> FormatDescriptor {
        descriptor(self.id, self.name, self.extension)
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

struct RequiredDependencyImporter {
    role: DependencyRoleId,
}

impl FormatImporter for RequiredDependencyImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.dependency-read-cancellation")
                .expect("qualified dependency read cancellation format id"),
            "Dependency read cancellation",
        )
        .extensions(["dependency-read-cancel"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .dependency_roles([(self.role.clone(), DependencyRequirement::Required)])
        .build()
        .expect("valid dependency read cancellation descriptor")
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

struct DeepSceneImporter;

impl FormatImporter for DeepSceneImporter {
    fn descriptor(&self) -> FormatDescriptor {
        descriptor(
            "dev.baozi.test.validation-cancellation",
            "Validation cancellation",
            "validation-cancel",
        )
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
        support::submit_node_scene(scope, VALIDATION_NODE_COUNT)
    }
}

struct DetectorScanCancellationImporter {
    cancellation: CancellationToken,
    scanned_bytes: Arc<AtomicUsize>,
    parse_calls: Arc<AtomicUsize>,
}

impl FormatImporter for DetectorScanCancellationImporter {
    fn descriptor(&self) -> FormatDescriptor {
        descriptor(
            "dev.baozi.test.detector-scan-cancellation",
            "Detector scan cancellation",
            "detector-scan-cancel",
        )
    }

    fn detect(
        &self,
        probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        let mut chunks = probe.bytes().chunks_exact(DETECTOR_WORK_CHUNK_BYTES);
        let first = chunks.next().expect("detector fixture has a first chunk");
        scope.with_work(first.len() as u64, || {
            self.scanned_bytes.fetch_add(first.len(), Ordering::AcqRel);
            Ok(())
        })?;

        self.cancellation.cancel();

        let second = chunks.next().expect("detector fixture has a second chunk");
        scope.with_work(second.len() as u64, || {
            self.scanned_bytes.fetch_add(second.len(), Ordering::AcqRel);
            Ok(())
        })?;
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        self.parse_calls.fetch_add(1, Ordering::AcqRel);
        support::submit_minimal_scene(scope)
    }
}

struct RecoveringDetectorCancellationImporter {
    cancellation: CancellationToken,
    caught_scope_failure: Arc<AtomicBool>,
    parse_calls: Arc<AtomicUsize>,
}

struct LatchThenPanicDetectorImporter {
    cancellation: CancellationToken,
    detect_calls: Arc<AtomicUsize>,
    parse_calls: Arc<AtomicUsize>,
}

impl FormatImporter for LatchThenPanicDetectorImporter {
    fn descriptor(&self) -> FormatDescriptor {
        descriptor(
            "dev.baozi.test.detector-latch-then-panic",
            "Detector latch then panic",
            "detector-latch-then-panic",
        )
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        self.detect_calls.fetch_add(1, Ordering::AcqRel);
        self.cancellation.cancel();
        assert!(scope.poll().is_err());
        panic!("detector panicked after latching cancellation")
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        self.parse_calls.fetch_add(1, Ordering::AcqRel);
        support::submit_minimal_scene(scope)
    }
}

#[derive(Clone, Copy)]
enum DetectorContractViolation {
    ZeroWork,
    NestedWork,
    CallbackError,
    PollDuringCallback,
    ReplayAbort,
}

thread_local! {
    static STALE_DETECTION_ABORT: RefCell<Option<DetectionAbort>> = const { RefCell::new(None) };
}

struct DetectorContractImporter {
    violation: DetectorContractViolation,
    detect_calls: AtomicUsize,
    parse_calls: Arc<AtomicUsize>,
}

impl FormatImporter for DetectorContractImporter {
    fn descriptor(&self) -> FormatDescriptor {
        let (id, name, extension) = match self.violation {
            DetectorContractViolation::ZeroWork => (
                "dev.baozi.test.detector-zero-work",
                "Detector zero work",
                "detector-zero-work",
            ),
            DetectorContractViolation::NestedWork => (
                "dev.baozi.test.detector-nested-work",
                "Detector nested work",
                "detector-nested-work",
            ),
            DetectorContractViolation::CallbackError => (
                "dev.baozi.test.detector-callback-error",
                "Detector callback error",
                "detector-callback-error",
            ),
            DetectorContractViolation::PollDuringCallback => (
                "dev.baozi.test.detector-poll-during-callback",
                "Detector poll during callback",
                "detector-poll-during-callback",
            ),
            DetectorContractViolation::ReplayAbort => (
                "dev.baozi.test.detector-replay-abort",
                "Detector replay abort",
                "detector-replay-abort",
            ),
        };
        descriptor(id, name, extension)
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        match self.violation {
            DetectorContractViolation::ZeroWork => {
                assert!(
                    scope
                        .with_work(0, || Ok(DetectionOutcome::NoMatch))
                        .is_err()
                );
            }
            DetectorContractViolation::NestedWork => {
                assert!(
                    scope
                        .with_work(1, || {
                            assert!(
                                scope
                                    .with_work(1, || Ok(DetectionOutcome::NoMatch))
                                    .is_err()
                            );
                            Ok(DetectionOutcome::NoMatch)
                        })
                        .is_err()
                );
            }
            DetectorContractViolation::CallbackError => {
                return scope.with_work(1, || {
                    scope.poll()?;
                    Ok(DetectionOutcome::NoMatch)
                });
            }
            DetectorContractViolation::PollDuringCallback => {
                assert!(
                    scope
                        .with_work(1, || {
                            assert!(scope.poll().is_err());
                            Ok(DetectionOutcome::NoMatch)
                        })
                        .is_err()
                );
            }
            DetectorContractViolation::ReplayAbort => {
                if self.detect_calls.fetch_add(1, Ordering::AcqRel) == 0 {
                    let stale = match scope.with_work(u64::MAX, || Ok(DetectionOutcome::NoMatch)) {
                        Ok(_) => panic!("oversized detector work must be rejected"),
                        Err(abort) => abort,
                    };
                    STALE_DETECTION_ABORT.with(|slot| {
                        *slot.borrow_mut() = Some(stale);
                    });
                } else {
                    return Err(STALE_DETECTION_ABORT.with(|slot| {
                        slot.borrow_mut()
                            .take()
                            .expect("first detector scope stored an abort token")
                    }));
                }
            }
        }
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        self.parse_calls.fetch_add(1, Ordering::AcqRel);
        support::submit_minimal_scene(scope)
    }
}

impl FormatImporter for RecoveringDetectorCancellationImporter {
    fn descriptor(&self) -> FormatDescriptor {
        descriptor(
            "dev.baozi.test.detector-cancellation-latch",
            "Detector cancellation latch",
            "detector-cancel-latch",
        )
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        self.cancellation.cancel();
        if scope.poll().is_err() {
            self.caught_scope_failure.store(true, Ordering::Release);
        }
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        self.parse_calls.fetch_add(1, Ordering::AcqRel);
        support::submit_minimal_scene(scope)
    }
}

struct CancelAfterFirstChunkReader {
    cancellation: CancellationToken,
    calls: Arc<AtomicUsize>,
    offset: usize,
}

impl AssetReader for CancelAfterFirstChunkReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        let count = (MULTI_CHUNK_BYTES - self.offset).min(buffer.len());
        buffer[..count].fill(0x5a);
        self.offset += count;
        if call == 1 {
            self.cancellation.cancel();
        }
        Ok(count)
    }
}

#[derive(Clone, Copy)]
enum CancelReadTarget {
    Primary,
    Dependency,
}

struct CancelDuringReadIo {
    domain: AuthorityDomain,
    primary: AssetPath,
    dependency: Option<AssetPath>,
    target: CancelReadTarget,
    cancellation: CancellationToken,
    opens: AtomicUsize,
    reader_calls: Arc<AtomicUsize>,
}

impl AssetIo for CancelDuringReadIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.opens.fetch_add(1, Ordering::AcqRel);
        let cancel_this_read = match self.target {
            CancelReadTarget::Primary => path == &self.primary,
            CancelReadTarget::Dependency => self.dependency.as_ref() == Some(path),
        };
        if cancel_this_read {
            return Ok(OpenedAsset::new(
                self.domain.clone(),
                path.clone(),
                CancelAfterFirstChunkReader {
                    cancellation: self.cancellation.clone(),
                    calls: Arc::clone(&self.reader_calls),
                    offset: 0,
                },
                None,
            ));
        }

        assert_eq!(path, &self.primary, "only the primary read remains stable");
        Ok(OpenedAsset::new(
            self.domain.clone(),
            self.primary.clone(),
            Cursor::new(&b"primary"[..]),
            Some(AssetLength::Exact(b"primary".len() as u64)),
        ))
    }
}

#[derive(Debug)]
struct ValidationSignalState {
    armed: AtomicBool,
    polls: AtomicUsize,
}

#[derive(Debug)]
struct ValidationCancellation {
    state: Arc<ValidationSignalState>,
}

impl CancellationSignal for ValidationCancellation {
    fn is_cancelled(&self) -> bool {
        if !self.state.armed.load(Ordering::Acquire) {
            return false;
        }
        self.state.polls.fetch_add(1, Ordering::AcqRel) + 1 >= CANCEL_DURING_VALIDATION_POLL
    }
}

fn request_for(importer: impl FormatImporter) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    registry
        .register(importer)
        .expect("cancellation stage importer registers");
    let snapshot = registry
        .freeze(RegistryBuildLimits::default())
        .expect("cancellation stage registry freezes");
    snapshot
        .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
        .expect("cancellation stage request compiles")
}

fn assert_signal_cancellation(failure: &ImportFailure) {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected operation cancellation through an operation scope")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        })
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

fn assert_detector_contract_failure(
    failure: &ImportFailure,
    expected_code: &str,
    attempted_work: u64,
    consumed_work: u64,
) {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected a detector descriptor contract failure")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::DescriptorContract { code } if *code == expected_code
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert_eq!(failure.last_completed().phase(), ImportPhase::PrimaryBound);
    assert!(failure.selected_format().is_none());
    assert!(failure.detection_evidence().is_empty());
    let detection_work = failure
        .ledger()
        .dimension(ImportDimension::DetectionWork)
        .expect("detector work evidence");
    assert_eq!(detection_work.attempted(), attempted_work);
    assert_eq!(detection_work.consumed(), consumed_work);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

#[test]
fn detector_scan_cancellation_stops_before_the_second_work_chunk() {
    let cancellation = CancellationToken::new();
    let scanned_bytes = Arc::new(AtomicUsize::new(0));
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = request_for(DetectorScanCancellationImporter {
        cancellation: cancellation.clone(),
        scanned_bytes: Arc::clone(&scanned_bytes),
        parse_calls: Arc::clone(&parse_calls),
    });
    let control = ImportOperationControl::default().with_cancellation(cancellation);

    let failure = import_bytes(
        &request,
        AssetPath::new("asset.detector-scan-cancel")
            .expect("valid detector scan cancellation source"),
        DETECTOR_SCAN_BYTES,
        control,
    )
    .expect_err("the second detector work chunk must observe cancellation");

    assert_signal_cancellation(&failure);
    assert_eq!(
        scanned_bytes.load(Ordering::Acquire),
        DETECTOR_WORK_CHUNK_BYTES
    );
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
    assert_eq!(failure.last_completed().phase(), ImportPhase::PrimaryBound);
    assert!(failure.selected_format().is_none());
    let detection_work = failure
        .ledger()
        .dimension(ImportDimension::DetectionWork)
        .expect("detector work evidence");
    assert_eq!(detection_work.attempted(), DETECTOR_WORK_CHUNK_BYTES as u64);
    assert_eq!(detection_work.consumed(), DETECTOR_WORK_CHUNK_BYTES as u64);
}

#[test]
fn detector_cannot_recover_from_latched_scan_cancellation() {
    let cancellation = CancellationToken::new();
    let caught_scope_failure = Arc::new(AtomicBool::new(false));
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = request_for(RecoveringDetectorCancellationImporter {
        cancellation: cancellation.clone(),
        caught_scope_failure: Arc::clone(&caught_scope_failure),
        parse_calls: Arc::clone(&parse_calls),
    });
    let control = ImportOperationControl::default().with_cancellation(cancellation);

    let failure = import_bytes(
        &request,
        AssetPath::new("asset.detector-cancel-latch")
            .expect("valid detector cancellation latch source"),
        DETECTOR_SCAN_BYTES,
        control,
    )
    .expect_err("a caught detector scope failure must remain session-terminal");

    assert_signal_cancellation(&failure);
    assert!(caught_scope_failure.load(Ordering::Acquire));
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
    assert_eq!(failure.last_completed().phase(), ImportPhase::PrimaryBound);
    assert!(failure.selected_format().is_none());
}

#[test]
fn detector_panic_cannot_replace_latched_cancellation_and_poisons_registration() {
    let cancellation = CancellationToken::new();
    let detect_calls = Arc::new(AtomicUsize::new(0));
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = request_for(LatchThenPanicDetectorImporter {
        cancellation: cancellation.clone(),
        detect_calls: Arc::clone(&detect_calls),
        parse_calls: Arc::clone(&parse_calls),
    });
    let source = AssetPath::new("asset.detector-latch-then-panic")
        .expect("valid detector latch then panic source");

    let first = import_bytes(
        &request,
        source.clone(),
        DETECTOR_SCAN_BYTES,
        ImportOperationControl::default().with_cancellation(cancellation),
    )
    .expect_err("latched cancellation must outrank the detector panic fault");

    assert_signal_cancellation(&first);
    assert!(first.detection_evidence().is_empty());
    assert_eq!(detect_calls.load(Ordering::Acquire), 1);
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);

    let poisoned = import_bytes(
        &request,
        source,
        DETECTOR_SCAN_BYTES,
        ImportOperationControl::default(),
    )
    .expect_err("the poisoned detector registration must reject reinvocation");

    assert!(matches!(poisoned.cause(), ImportCause::Detection(_)));
    let [evidence] = poisoned.detection_evidence() else {
        panic!("poisoned detector must produce exactly one detection evidence item")
    };
    let DetectionOutcome::Fault(fault) = evidence.outcome() else {
        panic!("poisoned detector must return fault evidence")
    };
    assert_eq!(fault.code(), DetectionFaultCode::ExtensionPoisoned);
    assert_eq!(
        evidence.format(),
        &FormatId::new("dev.baozi.test.detector-latch-then-panic")
            .expect("qualified detector latch then panic format id")
    );
    assert_eq!(detect_calls.load(Ordering::Acquire), 1);
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
}

#[test]
fn detector_cannot_nest_work_callbacks() {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = request_for(DetectorContractImporter {
        violation: DetectorContractViolation::NestedWork,
        detect_calls: AtomicUsize::new(0),
        parse_calls: Arc::clone(&parse_calls),
    });

    let failure = import_bytes(
        &request,
        AssetPath::new("asset.detector-nested-work").expect("valid nested work source"),
        DETECTOR_SCAN_BYTES,
        ImportOperationControl::default(),
    )
    .expect_err("nested detector work must terminate the session");

    assert_detector_contract_failure(&failure, "detection-work-already-in-progress", 1, 1);
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
}

#[test]
fn detector_cannot_admit_zero_sized_work() {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = request_for(DetectorContractImporter {
        violation: DetectorContractViolation::ZeroWork,
        detect_calls: AtomicUsize::new(0),
        parse_calls: Arc::clone(&parse_calls),
    });

    let failure = import_bytes(
        &request,
        AssetPath::new("asset.detector-zero-work").expect("valid zero work source"),
        DETECTOR_SCAN_BYTES,
        ImportOperationControl::default(),
    )
    .expect_err("zero-sized detector work must terminate the session");

    assert_detector_contract_failure(&failure, "detector-work-zero-amount", 0, 0);
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
}

#[test]
fn detector_callback_error_preserves_attempted_without_consumed_work() {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = request_for(DetectorContractImporter {
        violation: DetectorContractViolation::CallbackError,
        detect_calls: AtomicUsize::new(0),
        parse_calls: Arc::clone(&parse_calls),
    });

    let failure = import_bytes(
        &request,
        AssetPath::new("asset.detector-callback-error").expect("valid callback error source"),
        DETECTOR_SCAN_BYTES,
        ImportOperationControl::default(),
    )
    .expect_err("a detector callback error must terminate the session");

    assert_detector_contract_failure(&failure, "detection-poll-during-work", 1, 0);
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
}

#[test]
fn detector_poll_during_callback_latches_contract_after_consumed_work() {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = request_for(DetectorContractImporter {
        violation: DetectorContractViolation::PollDuringCallback,
        detect_calls: AtomicUsize::new(0),
        parse_calls: Arc::clone(&parse_calls),
    });

    let failure = import_bytes(
        &request,
        AssetPath::new("asset.detector-poll-during-callback")
            .expect("valid poll during callback source"),
        DETECTOR_SCAN_BYTES,
        ImportOperationControl::default(),
    )
    .expect_err("polling while detector work is active must terminate the session");

    assert_detector_contract_failure(&failure, "detection-poll-during-work", 1, 1);
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
}

#[test]
fn detector_abort_token_cannot_be_replayed_across_scopes() {
    STALE_DETECTION_ABORT.with(|slot| {
        let _ = slot.borrow_mut().take();
    });
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = request_for(DetectorContractImporter {
        violation: DetectorContractViolation::ReplayAbort,
        detect_calls: AtomicUsize::new(0),
        parse_calls: Arc::clone(&parse_calls),
    });
    let source =
        AssetPath::new("asset.detector-replay-abort").expect("valid detector abort replay source");

    let first = import_bytes(
        &request,
        source.clone(),
        DETECTOR_SCAN_BYTES,
        ImportOperationControl::default(),
    )
    .expect_err("the originating scope must retain the oversized-work failure");
    assert_detector_contract_failure(&first, "detector-work-quantum-exceeded", 0, 0);

    let replay = import_bytes(
        &request,
        source,
        DETECTOR_SCAN_BYTES,
        ImportOperationControl::default(),
    )
    .expect_err("an abort token without this scope's latch must be rejected");
    assert_detector_contract_failure(&replay, "detection-abort-without-terminal-cause", 0, 0);
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
}

#[test]
fn primary_multichunk_read_polls_cancellation_before_the_second_chunk() {
    let source = AssetPath::new("asset.primary-read-cancel")
        .expect("valid primary read cancellation source");
    let cancellation = CancellationToken::new();
    let reader_calls = Arc::new(AtomicUsize::new(0));
    let io = CancelDuringReadIo {
        domain: AuthorityDomain::new(),
        primary: source.clone(),
        dependency: None,
        target: CancelReadTarget::Primary,
        cancellation: cancellation.clone(),
        opens: AtomicUsize::new(0),
        reader_calls: Arc::clone(&reader_calls),
    };
    let request = request_for(EmptyImporter {
        id: "dev.baozi.test.primary-read-cancellation",
        name: "Primary read cancellation",
        extension: "primary-read-cancel",
    });
    let control = ImportOperationControl::default().with_cancellation(cancellation);

    let failure = import_asset(&request, &io, source, control)
        .expect_err("primary read cancellation must stop before the second chunk");

    assert_signal_cancellation(&failure);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SessionCreated
    );
    assert_eq!(io.opens.load(Ordering::Acquire), 1);
    assert_eq!(reader_calls.load(Ordering::Acquire), 1);
    let primary = failure
        .ledger()
        .dimension(ImportDimension::PrimaryBytes)
        .expect("primary byte evidence");
    assert_eq!(primary.attempted(), READ_CHUNK_BYTES as u64);
    assert_eq!(primary.consumed(), READ_CHUNK_BYTES as u64);
}

#[test]
fn dependency_multichunk_read_polls_cancellation_before_the_second_chunk() {
    let source = AssetPath::new("asset.dependency-read-cancel")
        .expect("valid dependency read cancellation source");
    let dependency = AssetPath::new("sidecar.bin").expect("valid dependency path");
    let cancellation = CancellationToken::new();
    let reader_calls = Arc::new(AtomicUsize::new(0));
    let role = DependencyRoleId::new("buffer.required").expect("valid required dependency role");
    let io = CancelDuringReadIo {
        domain: AuthorityDomain::new(),
        primary: source.clone(),
        dependency: Some(dependency),
        target: CancelReadTarget::Dependency,
        cancellation: cancellation.clone(),
        opens: AtomicUsize::new(0),
        reader_calls: Arc::clone(&reader_calls),
    };
    let request = request_for(RequiredDependencyImporter { role });
    let control = ImportOperationControl::default().with_cancellation(cancellation);

    let failure = import_asset(&request, &io, source, control)
        .expect_err("dependency read cancellation must stop before the second chunk");

    assert_signal_cancellation(&failure);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    assert_eq!(io.opens.load(Ordering::Acquire), 2);
    assert_eq!(reader_calls.load(Ordering::Acquire), 1);
    let dependency = failure
        .ledger()
        .dimension(ImportDimension::DependencyBytes)
        .expect("dependency byte evidence");
    assert_eq!(dependency.attempted(), READ_CHUNK_BYTES as u64);
    assert_eq!(dependency.consumed(), READ_CHUNK_BYTES as u64);
    let references = failure
        .ledger()
        .dimension(ImportDimension::DependencyReference)
        .expect("dependency reference evidence");
    assert_eq!(references.attempted(), 1);
    assert_eq!(references.consumed(), 1);
}

#[test]
fn import_validation_cancellation_preserves_draft_stage_and_releases_validation_storage() {
    let state = Arc::new(ValidationSignalState {
        armed: AtomicBool::new(false),
        polls: AtomicUsize::new(0),
    });
    let observer_state = Arc::clone(&state);
    let validation_events = Arc::new(AtomicUsize::new(0));
    let observed_validation_events = Arc::clone(&validation_events);
    let control = ImportOperationControl::default()
        .with_cancellation(ValidationCancellation {
            state: Arc::clone(&state),
        })
        .with_progress_observer(
            move |event: baozi_operation::ProgressEvent<
                ImportProgressPhase,
                ImportProgressUnit,
            >| {
                if event.phase() == ImportProgressPhase::Validate {
                    observed_validation_events.fetch_add(1, Ordering::AcqRel);
                    observer_state.armed.store(true, Ordering::Release);
                }
                ProgressDecision::Continue
            },
        );
    let request = request_for(DeepSceneImporter);

    let failure = import_bytes(
        &request,
        AssetPath::new("asset.validation-cancel").expect("valid validation cancellation source"),
        b"primary",
        control,
    )
    .expect_err("validation cancellation must stop before proof publication");

    assert_signal_cancellation(&failure);
    assert!(state.armed.load(Ordering::Acquire));
    assert!(
        state.polls.load(Ordering::Acquire) >= CANCEL_DURING_VALIDATION_POLL,
        "cancellation must be observed from a poll inside validation"
    );
    assert_eq!(validation_events.load(Ordering::Acquire), 1);
    assert_eq!(failure.last_completed().phase(), ImportPhase::DraftBuilt);
    let validation_scratch = failure
        .ledger()
        .dimension(ImportDimension::ScratchBytes)
        .expect("validation scratch evidence");
    assert_eq!(validation_scratch.current_live_bytes(), 0);
    assert!(validation_scratch.peak_live_bytes() > 0);
    let output = failure
        .ledger()
        .dimension(ImportDimension::CanonicalOutputBytes)
        .expect("canonical output evidence");
    assert_eq!(output.committed(), 0);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
}

#[test]
fn final_progress_can_trigger_signal_cancellation_at_the_last_publication_checkpoint() {
    let cancellation = CancellationToken::new();
    let cancellation_from_observer = cancellation.clone();
    let final_events = Arc::new(AtomicUsize::new(0));
    let observed_final_events = Arc::clone(&final_events);
    let control = ImportOperationControl::default()
        .with_cancellation(cancellation)
        .with_progress_observer(
            move |event: baozi_operation::ProgressEvent<
                ImportProgressPhase,
                ImportProgressUnit,
            >| {
                if event.phase() == ImportProgressPhase::Finalize {
                    observed_final_events.fetch_add(1, Ordering::AcqRel);
                    cancellation_from_observer.cancel();
                }
                ProgressDecision::Continue
            },
        );
    let source = AssetPath::new("asset.publication-checkpoint")
        .expect("valid publication checkpoint source");
    let request = request_for(EmptyImporter {
        id: "dev.baozi.test.publication-checkpoint",
        name: "Publication checkpoint cancellation",
        extension: "publication-checkpoint",
    });

    let failure = import_bytes(&request, source, b"primary", control)
        .expect_err("last pre-publication poll must observe signal cancellation");

    assert_signal_cancellation(&failure);
    assert_eq!(final_events.load(Ordering::Acquire), 1);
    assert_eq!(failure.last_completed().phase(), ImportPhase::Validated);
    let output = failure
        .ledger()
        .dimension(ImportDimension::CanonicalOutputBytes)
        .expect("canonical output evidence");
    assert_eq!(output.committed(), 0);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
}
