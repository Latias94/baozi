mod support;

use baozi_core::FormatId;
use baozi_import::{
    CapabilityStatus, CauseOwner, ConformancePolicy, DetectionOutcome, DetectionResult,
    DetectionScope, DiagnosticByteRange, DiagnosticCode, DiagnosticSeverity, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, ImportCause, ImportConfigOverlay,
    ImportDiagnostic, ImportDimension, ImportFailure, ImportLossDisposition, ImportLossKind,
    ImportOperationControl, ImportPhase, ImportReport, ImportRequest, ImportRequestLayer,
    ImportResult, LossPolicy, OptionCompileLimits, ParseResult, ParseScope, PreparedImportRequest,
    ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder, ResourceLimits,
    ResourceLimitsOverlay, SceneDraft, ScopeAbortCause, TextBomPolicy, TextControlPolicy,
    TextDecodingMode, TextHardCaps, TextLexicalMode, TextNewline, TextProfile, TextSourceEncoding,
    import_asset as run_import_asset, import_bytes as run_import_bytes,
};
use baozi_io::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenWorkError, AssetPath, AssetReadError,
    AssetReader, AuthorityDomain, OpenedAsset,
};
use baozi_operation::CounterClass;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const SOURCE: &str = "scene.profile";
const SOURCE_BYTES: &[u8] = b"abcdef";
const RECOVERY_CODE: &str = "test.invalid_utf8_replaced";
const FORMAT_ID: &str = "dev.baozi.test.text-profile-session";

#[derive(Debug, Clone, Copy)]
enum ParserBehavior {
    Succeed,
    Recover,
}

struct ContractImporter {
    descriptor: FormatDescriptor,
    behavior: ParserBehavior,
    parse_calls: Arc<AtomicUsize>,
    captured_limits: Arc<Mutex<Option<ResourceLimits>>>,
}

impl ContractImporter {
    fn new(
        descriptor: FormatDescriptor,
        behavior: ParserBehavior,
    ) -> (Self, Arc<AtomicUsize>, Arc<Mutex<Option<ResourceLimits>>>) {
        let parse_calls = Arc::new(AtomicUsize::new(0));
        let captured_limits = Arc::new(Mutex::new(None));
        (
            Self {
                descriptor,
                behavior,
                parse_calls: Arc::clone(&parse_calls),
                captured_limits: Arc::clone(&captured_limits),
            },
            parse_calls,
            captured_limits,
        )
    }
}

impl FormatImporter for ContractImporter {
    fn descriptor(&self) -> FormatDescriptor {
        self.descriptor.clone()
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
        self.parse_calls.fetch_add(1, Ordering::AcqRel);
        *self
            .captured_limits
            .lock()
            .expect("captured-limit mutex must remain available") = Some(scope.limits().clone());

        if matches!(self.behavior, ParserBehavior::Recover) {
            scope.record_format_recovery(
                DiagnosticCode::new(RECOVERY_CODE).expect("static diagnostic code must be valid"),
                scope.diagnostic_source(),
                DiagnosticByteRange::new(1, 4).expect("static byte range must be valid"),
                format_args!("replaced invalid UTF-8"),
            )?;
        }

        support::submit_minimal_scene(scope)
    }
}

fn descriptor(encoding: FormatEncoding, profile: Option<TextProfile>) -> FormatDescriptor {
    let mut builder = FormatDescriptor::builder(
        FormatId::new(FORMAT_ID).expect("static format id must be valid"),
        "Text Profile Session Contract",
    )
    .extensions(["profile"])
    .encoding(encoding)
    .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)]);
    if let Some(profile) = profile {
        builder = builder.text_profile(profile);
    }
    builder.build().expect("test descriptor must be valid")
}

fn text_profile(hard_caps: [u64; 5], allow_recovery: bool) -> TextProfile {
    let builder = TextProfile::builder(
        TextHardCaps::new(
            hard_caps[0],
            hard_caps[1],
            hard_caps[2],
            hard_caps[3],
            hard_caps[4],
        )
        .expect("positive text hard caps must be valid"),
    )
    .source_encodings([TextSourceEncoding::Utf8])
    .bom_policy(TextBomPolicy::Forbidden)
    .newlines([TextNewline::Lf])
    .lexical_mode(TextLexicalMode::Utf8Document)
    .control_policy(TextControlPolicy::AllowHorizontalTab);
    let builder = if allow_recovery {
        builder.decoding_modes([
            TextDecodingMode::RejectInvalid,
            TextDecodingMode::ReplaceInvalidUtf8,
        ])
    } else {
        builder.decoding_modes([TextDecodingMode::RejectInvalid])
    };
    builder.build().expect("test text profile must be valid")
}

fn request_for(
    importer: ContractImporter,
    limits: ResourceLimits,
    conformance: ConformancePolicy,
    loss_policy: LossPolicy,
) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    registry
        .register(importer)
        .expect("test importer registration must succeed");
    let snapshot = registry
        .freeze(RegistryBuildLimits::default())
        .expect("test registry must freeze");
    let config = ImportConfigOverlay::default()
        .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
        .with_conformance_policy(conformance)
        .with_loss_policy(loss_policy);
    snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(config),
            ),
            OptionCompileLimits::default(),
        )
        .expect("test request must compile")
}

fn recovery_loss_policy() -> LossPolicy {
    LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery)
}

#[allow(
    clippy::result_large_err,
    reason = "contract assertions inspect complete bounded import failure evidence"
)]
fn operation_result<T>(result: ImportResult<T>) -> Result<T, ImportFailure> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match error.into_operation_failure() {
            Ok(failure) => Err(failure),
            Err(source) => panic!("test failed before session creation: {source}"),
        },
    }
}

#[allow(
    clippy::result_large_err,
    reason = "contract assertions inspect complete bounded import failure evidence"
)]
fn import_bytes(
    request: &PreparedImportRequest,
    bytes: &[u8],
) -> Result<ImportReport, ImportFailure> {
    operation_result(run_import_bytes(
        request,
        AssetPath::new(SOURCE).expect("static source path must be valid"),
        bytes,
        ImportOperationControl::default(),
    ))
}

fn assert_recovery_diagnostic(diagnostics: &[ImportDiagnostic]) {
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.severity(), DiagnosticSeverity::Warning);
    assert_eq!(diagnostic.code().as_str(), RECOVERY_CODE);
    assert_eq!(diagnostic.source().as_str(), SOURCE);
    assert_eq!(
        diagnostic.byte_range(),
        Some(DiagnosticByteRange::new(1, 4).expect("static byte range must be valid"))
    );
}

fn assert_conformance_gate(failure: &ImportFailure) {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("recovery conformance gate must remain a scope-owned failure")
    };
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::FormatRecoveryRejectedByConformance { code }
            if code.as_str() == RECOVERY_CODE
    ));
    assert_recovery_disposition(
        failure.loss_evidence(),
        ImportLossDisposition::RejectedByConformance,
    );
    assert_recovery_diagnostic(failure.diagnostics());
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    for dimension in [
        ImportDimension::RecoveryCount,
        ImportDimension::LossEvidenceCount,
        ImportDimension::DiagnosticCount,
    ] {
        let evidence = failure
            .ledger()
            .dimension(dimension)
            .expect("recovery evidence dimension must exist");
        assert_eq!(evidence.attempted(), 1);
        assert_eq!(evidence.consumed(), 1);
    }
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

fn assert_recovery_disposition(
    evidence: &[baozi_import::ImportLossEvidence],
    expected: ImportLossDisposition,
) {
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].kind(), ImportLossKind::FormatRecovery);
    assert_eq!(evidence[0].code().as_str(), RECOVERY_CODE);
    assert_eq!(
        evidence[0].raw_range(),
        Some(DiagnosticByteRange::new(1, 4).expect("static byte range must be valid"))
    );
    assert_eq!(evidence[0].disposition(), expected);
}

#[test]
fn binary_descriptor_cannot_record_text_recovery() {
    let (importer, parse_calls, _) = ContractImporter::new(
        descriptor(FormatEncoding::Binary, None),
        ParserBehavior::Recover,
    );
    let request = request_for(
        importer,
        ResourceLimits::default(),
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
    );

    let failure = import_bytes(&request, SOURCE_BYTES)
        .expect_err("binary descriptors must not authorize text recovery");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_conformance_gate(&failure);
}

#[test]
fn reject_only_text_profile_cannot_record_replacement_recovery() {
    let profile = text_profile([1024, 1024, 1024, 1024, 1024], false);
    let (importer, parse_calls, _) = ContractImporter::new(
        descriptor(FormatEncoding::Text, Some(profile)),
        ParserBehavior::Recover,
    );
    let request = request_for(
        importer,
        ResourceLimits::default(),
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
    );

    let failure = import_bytes(&request, SOURCE_BYTES)
        .expect_err("reject-only text profiles must not authorize replacement recovery");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_conformance_gate(&failure);
}

#[test]
fn declared_replacement_recovery_still_requires_both_common_policies() {
    for (conformance, loss_policy, expected) in [
        (
            ConformancePolicy::AllowDeclaredRecovery,
            recovery_loss_policy(),
            ImportLossDisposition::Applied,
        ),
        (
            ConformancePolicy::Strict,
            recovery_loss_policy(),
            ImportLossDisposition::RejectedByConformance,
        ),
        (
            ConformancePolicy::AllowDeclaredRecovery,
            LossPolicy::preserve_or_fail(),
            ImportLossDisposition::RejectedByLossPolicy,
        ),
    ] {
        let profile = text_profile([1024, 1024, 1024, 1024, 1024], true);
        let (importer, parse_calls, _) = ContractImporter::new(
            descriptor(FormatEncoding::Text, Some(profile)),
            ParserBehavior::Recover,
        );
        let request = request_for(
            importer,
            ResourceLimits::default(),
            conformance,
            loss_policy,
        );
        let result = import_bytes(&request, SOURCE_BYTES);

        match expected {
            ImportLossDisposition::Applied => {
                let report = result.expect("both common policies must authorize declared recovery");
                assert_recovery_disposition(report.loss_evidence(), expected);
                assert_recovery_diagnostic(report.diagnostics());
            }
            ImportLossDisposition::RejectedByConformance => {
                let failure = result.expect_err("strict conformance must reject recovery");
                assert_recovery_disposition(failure.loss_evidence(), expected);
                assert_recovery_diagnostic(failure.diagnostics());
                assert!(matches!(
                    failure.cause(),
                    ImportCause::Scope(abort)
                        if matches!(
                            abort.cause(),
                            ScopeAbortCause::FormatRecoveryRejectedByConformance { .. }
                        )
                ));
            }
            ImportLossDisposition::RejectedByLossPolicy => {
                let failure = result.expect_err("strict loss policy must reject recovery");
                assert_recovery_disposition(failure.loss_evidence(), expected);
                assert_recovery_diagnostic(failure.diagnostics());
                assert!(matches!(
                    failure.cause(),
                    ImportCause::Scope(abort)
                        if matches!(
                            abort.cause(),
                            ScopeAbortCause::FormatRecoveryRejectedByLossPolicy { .. }
                        )
                ));
            }
        }
        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    }
}

fn limits_with_text_caps(caps: [u64; 5]) -> ResourceLimits {
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = caps[0];
    limits.max_decoded_utf8_bytes = caps[1];
    limits.max_line_index_bytes = caps[2];
    limits.max_tokens = caps[3];
    limits.max_canonical_string_bytes = caps[4] as usize;
    limits
}

fn assert_text_limits(limits: &ResourceLimits, expected: [u64; 5]) {
    assert_eq!(limits.max_primary_asset_bytes, expected[0]);
    assert_eq!(limits.max_decoded_utf8_bytes, expected[1]);
    assert_eq!(limits.max_line_index_bytes, expected[2]);
    assert_eq!(limits.max_tokens, expected[3]);
    assert_eq!(limits.max_canonical_string_bytes, expected[4] as usize);
}

#[test]
fn selected_text_profile_freezes_per_dimension_minima_without_raising_request_limits() {
    let profile_caps = [64, 80, 96, 112, 128];
    for (request_caps, expected_caps) in [
        ([128, 40, 192, 56, 256], [64, 40, 96, 56, 128]),
        ([32, 40, 48, 56, 60], [32, 40, 48, 56, 60]),
    ] {
        let profile = text_profile(profile_caps, false);
        let (importer, parse_calls, captured_limits) = ContractImporter::new(
            descriptor(FormatEncoding::Text, Some(profile)),
            ParserBehavior::Succeed,
        );
        let request_limits = limits_with_text_caps(request_caps);
        let request = request_for(
            importer,
            request_limits.clone(),
            ConformancePolicy::Strict,
            LossPolicy::preserve_or_fail(),
        );

        import_bytes(&request, SOURCE_BYTES).expect("input is below every effective text limit");

        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
        let effective = captured_limits
            .lock()
            .expect("captured-limit mutex must remain available")
            .clone()
            .expect("parser must observe frozen limits");
        assert_text_limits(&effective, expected_caps);
        assert_eq!(request.limits(), &request_limits);
    }
}

#[test]
fn primary_bytes_over_text_profile_raw_cap_fail_before_parser_entry() {
    let profile = text_profile([5, 64, 64, 64, 64], false);
    let (importer, parse_calls, _) = ContractImporter::new(
        descriptor(FormatEncoding::Text, Some(profile)),
        ParserBehavior::Succeed,
    );
    let request = request_for(
        importer,
        ResourceLimits::default(),
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    );

    let failure = import_bytes(&request, SOURCE_BYTES)
        .expect_err("profile raw cap must reject the primary snapshot before parsing");

    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Ledger(baozi_import::LedgerCause::LimitExceeded {
                    class: CounterClass::Attempted,
                    dimension: ImportDimension::PrimaryBytes,
                    value: 6,
                    limit: 5,
                })
            )
    ));
    assert_eq!(
        failure.selected_format().map(FormatId::as_str),
        Some(FORMAT_ID)
    );
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    let primary = failure
        .ledger()
        .dimension(ImportDimension::PrimaryBytes)
        .expect("primary byte dimension must exist");
    assert_eq!(primary.attempted(), SOURCE_BYTES.len() as u64);
    assert_eq!(primary.consumed(), SOURCE_BYTES.len() as u64);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

struct CountingReader {
    bytes: Arc<[u8]>,
    offset: usize,
    calls: Arc<AtomicUsize>,
}

impl AssetReader for CountingReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let remaining = &self.bytes[self.offset..];
        let count = remaining.len().min(buffer.len());
        buffer[..count].copy_from_slice(&remaining[..count]);
        self.offset += count;
        Ok(count)
    }
}

struct BackendCappedIo {
    domain: AuthorityDomain,
    source: AssetPath,
    bytes: Arc<[u8]>,
    hard_limit: u64,
    reader_calls: Arc<AtomicUsize>,
}

impl AssetIo for BackendCappedIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // One identity comparison, one path clone, and fixed shared-reader/cap setup.
        Ok(NonZeroU64::new(2 * path.as_str().len() as u64 + 16).expect("bounded fixture work"))
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        assert_eq!(path, &self.source);
        Ok(OpenedAsset::new(
            self.domain.clone(),
            self.source.clone(),
            CountingReader {
                bytes: Arc::clone(&self.bytes),
                offset: 0,
                calls: Arc::clone(&self.reader_calls),
            },
            Some(AssetLength::Exact(self.bytes.len() as u64)),
        )
        .with_backend_hard_limit(self.hard_limit))
    }
}

#[test]
fn successful_primary_bind_freezes_backend_hard_cap_into_parse_scope_limits() {
    const BACKEND_HARD_CAP: u64 = 16;
    const PROFILE_RAW_CAP: u64 = 64;
    const REQUEST_RAW_CAP: u64 = 128;

    const {
        assert!((SOURCE_BYTES.len() as u64) < BACKEND_HARD_CAP);
        assert!(BACKEND_HARD_CAP < PROFILE_RAW_CAP);
        assert!(PROFILE_RAW_CAP < REQUEST_RAW_CAP);
    }

    let profile = text_profile([PROFILE_RAW_CAP, 1024, 1024, 1024, 1024], false);
    let (importer, parse_calls, captured_limits) = ContractImporter::new(
        descriptor(FormatEncoding::Text, Some(profile)),
        ParserBehavior::Succeed,
    );
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = REQUEST_RAW_CAP;
    let request = request_for(
        importer,
        limits,
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    );
    let source = AssetPath::new(SOURCE).expect("static source path must be valid");
    let reader_calls = Arc::new(AtomicUsize::new(0));
    let io = BackendCappedIo {
        domain: AuthorityDomain::new(),
        source: source.clone(),
        bytes: Arc::from(SOURCE_BYTES),
        hard_limit: BACKEND_HARD_CAP,
        reader_calls: Arc::clone(&reader_calls),
    };

    let report = operation_result(run_import_asset(
        &request,
        &io,
        source,
        ImportOperationControl::default(),
    ))
    .expect("input below every intersected primary cap must import successfully");

    assert!(reader_calls.load(Ordering::Acquire) > 0);
    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    let effective = captured_limits
        .lock()
        .expect("captured-limit mutex must remain available")
        .clone()
        .expect("parser must observe frozen limits");
    assert_eq!(effective.max_primary_asset_bytes, BACKEND_HARD_CAP);
    assert_eq!(request.limits().max_primary_asset_bytes, REQUEST_RAW_CAP);
    assert_eq!(report.format().id().as_str(), FORMAT_ID);
    assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    let primary = report
        .ledger()
        .dimension(ImportDimension::PrimaryBytes)
        .expect("primary byte dimension must exist");
    assert_eq!(primary.attempted(), SOURCE_BYTES.len() as u64);
    assert_eq!(primary.consumed(), SOURCE_BYTES.len() as u64);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn lower_backend_hard_cap_wins_on_primary_open_before_read_or_parse() {
    let profile = text_profile([64, 64, 64, 64, 64], false);
    let (importer, parse_calls, _) = ContractImporter::new(
        descriptor(FormatEncoding::Text, Some(profile)),
        ParserBehavior::Succeed,
    );
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = 128;
    let request = request_for(
        importer,
        limits,
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    );
    let source = AssetPath::new(SOURCE).expect("static source path must be valid");
    let reader_calls = Arc::new(AtomicUsize::new(0));
    let io = BackendCappedIo {
        domain: AuthorityDomain::new(),
        source: source.clone(),
        bytes: Arc::from(SOURCE_BYTES),
        hard_limit: 5,
        reader_calls: Arc::clone(&reader_calls),
    };

    let failure = operation_result(run_import_asset(
        &request,
        &io,
        source,
        ImportOperationControl::default(),
    ))
    .expect_err("backend cap below request and profile caps must win");

    assert_eq!(reader_calls.load(Ordering::Acquire), 0);
    assert_eq!(parse_calls.load(Ordering::Acquire), 0);
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    limit: 5,
                    ..
                }
            )
    ));
    assert!(failure.selected_format().is_none());
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SessionCreated
    );
    let primary = failure
        .ledger()
        .dimension(ImportDimension::PrimaryBytes)
        .expect("primary byte dimension must exist");
    assert_eq!(primary.attempted(), SOURCE_BYTES.len() as u64);
    assert_eq!(primary.consumed(), 0);
    assert_eq!(primary.peak_live_bytes(), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}
