use baozi_core::{FormatId, MeshBinding, MeshId, SceneDraft};
use baozi_import::{
    CapabilityStatus, CauseOwner, ConformancePolicy, DetectionOutcome, DetectionResult,
    DetectionScope, DiagnosticByteRange, DiagnosticCode, FormatCapability, FormatDescriptor,
    FormatEncoding, FormatFailureDetailSink, FormatFailureSource, FormatImporter, ImportCause,
    ImportConfigOverlay, ImportDimension, ImportFailure, ImportLossDisposition, ImportLossKind,
    ImportOperationControl, ImportProgressPhase, ImportProgressUnit, ImportReport, ImportRequest,
    ImportRequestLayer, ImportResult, LossPolicy, OptionCompileLimits, ParseResult, ParseScope,
    PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    ResourceLimits, ResourceLimitsOverlay, ScopeAbortCause, TextBomPolicy, TextControlPolicy,
    TextDecodingMode, TextHardCaps, TextLexicalMode, TextNewline, TextProfile, TextSourceEncoding,
    import_bytes,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationToken, ProgressDecision};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SOURCE: &str = "scene.recovery";
const SOURCE_BYTES: &[u8] = b"abcdef";
const RECOVERY_CODE: &str = "test.recovered_token";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryExit {
    Success,
    FormatFailure,
    ValidationFailure,
    Cancellation,
}

struct RecoveryImporter {
    exit: RecoveryExit,
    cancellation: Option<CancellationToken>,
}

impl FormatImporter for RecoveryImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.recovery-evidence").unwrap(),
            "Recovery Evidence",
        )
        .extensions(["recovery"])
        .encoding(FormatEncoding::Text)
        .text_profile(recovery_text_profile())
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
        scope.record_format_recovery(
            DiagnosticCode::new(RECOVERY_CODE).unwrap(),
            scope.diagnostic_source(),
            DiagnosticByteRange::new(1, 4).unwrap(),
            format_args!("recovered malformed token"),
        )?;

        match self.exit {
            RecoveryExit::Success => scope.scene_builder()?.finish_draft(),
            RecoveryExit::FormatFailure => Err(scope.format_failure(TestFormatFailure)),
            RecoveryExit::ValidationFailure => {
                let mut builder = scope.scene_builder()?;
                builder.add_node_mesh_binding(builder.root(), MeshBinding::new(MeshId::new(99)))?;
                builder.finish_draft()
            }
            RecoveryExit::Cancellation => {
                self.cancellation
                    .as_ref()
                    .expect("cancellation token")
                    .cancel();
                scope.poll()?;
                unreachable!("cancelled recovery import must stop at the scope boundary")
            }
        }
    }
}

fn recovery_text_profile() -> TextProfile {
    TextProfile::builder(
        TextHardCaps::new(
            1024 * 1024,
            1024 * 1024,
            1024 * 1024,
            1024 * 1024,
            1024 * 1024,
        )
        .expect("valid recovery fixture hard caps"),
    )
    .source_encodings([TextSourceEncoding::Utf8])
    .bom_policy(TextBomPolicy::Forbidden)
    .newlines([TextNewline::Lf])
    .lexical_mode(TextLexicalMode::Utf8Document)
    .decoding_modes([
        TextDecodingMode::RejectInvalid,
        TextDecodingMode::ReplaceInvalidUtf8,
    ])
    .control_policy(TextControlPolicy::AllowHorizontalTab)
    .build()
    .expect("valid recovery fixture text profile")
}

#[derive(Debug)]
struct TestFormatFailure;

impl fmt::Display for TestFormatFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("test format failure after recovery")
    }
}

impl std::error::Error for TestFormatFailure {}

impl FormatFailureSource for TestFormatFailure {
    fn code(&self) -> &'static str {
        "test.failure_after_recovery"
    }

    fn visit_details(&self, _sink: &mut dyn FormatFailureDetailSink) {}
}

#[allow(
    clippy::result_large_err,
    reason = "recovery contract assertions inspect complete bounded import failure evidence"
)]
fn operation_result<T>(result: ImportResult<T>) -> Result<T, ImportFailure> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match error.into_operation_failure() {
            Ok(failure) => Err(failure),
            Err(source) => panic!("recovery test failed before session creation: {source}"),
        },
    }
}

fn request_for(
    exit: RecoveryExit,
    cancellation: Option<CancellationToken>,
    conformance: ConformancePolicy,
    loss_policy: LossPolicy,
    limits: ResourceLimits,
) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    registry
        .register(RecoveryImporter { exit, cancellation })
        .unwrap();
    let snapshot = registry.freeze(RegistryBuildLimits::default()).unwrap();
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
        .unwrap()
}

fn recovery_loss_policy() -> LossPolicy {
    LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery)
}

#[allow(
    clippy::result_large_err,
    reason = "recovery contract assertions inspect complete bounded import failure evidence"
)]
fn run(
    exit: RecoveryExit,
    cancellation: Option<CancellationToken>,
    conformance: ConformancePolicy,
    loss_policy: LossPolicy,
    limits: ResourceLimits,
) -> Result<ImportReport, ImportFailure> {
    let request = request_for(exit, cancellation.clone(), conformance, loss_policy, limits);
    let control = cancellation.map_or_else(ImportOperationControl::default, |token| {
        ImportOperationControl::default().with_cancellation(token)
    });
    operation_result(import_bytes(
        &request,
        AssetPath::new(SOURCE).unwrap(),
        SOURCE_BYTES,
        control,
    ))
}

fn assert_recovery_evidence(
    evidence: &[baozi_import::ImportLossEvidence],
    disposition: ImportLossDisposition,
) {
    assert_eq!(evidence.len(), 1);
    let evidence = &evidence[0];
    assert_eq!(evidence.kind(), ImportLossKind::FormatRecovery);
    assert_eq!(evidence.code().as_str(), RECOVERY_CODE);
    assert_eq!(evidence.source().as_str(), SOURCE);
    assert_eq!(
        evidence.raw_range(),
        DiagnosticByteRange::new(1, 4).unwrap()
    );
    assert_eq!(evidence.disposition(), disposition);
}

#[test]
fn authorized_recovery_is_published_with_ordered_diagnostic_and_ledger_evidence() {
    let report = run(
        RecoveryExit::Success,
        None,
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
        ResourceLimits::default(),
    )
    .expect("authorized recovery must publish");

    assert_recovery_evidence(report.loss_evidence(), ImportLossDisposition::Applied);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(report.diagnostics()[0].code().as_str(), RECOVERY_CODE);
    assert_eq!(
        report.diagnostics()[0].byte_range(),
        Some(DiagnosticByteRange::new(1, 4).unwrap())
    );
    let recovery_count = report
        .ledger()
        .dimension(ImportDimension::RecoveryCount)
        .unwrap();
    assert_eq!(recovery_count.attempted(), 1);
    assert_eq!(recovery_count.consumed(), 1);
    let raw_bytes = report
        .ledger()
        .dimension(ImportDimension::RecoveryRawBytes)
        .unwrap();
    assert_eq!(raw_bytes.attempted(), 3);
    assert_eq!(raw_bytes.consumed(), 3);
    let evidence_count = report
        .ledger()
        .dimension(ImportDimension::LossEvidenceCount)
        .unwrap();
    assert_eq!(evidence_count.attempted(), 1);
    assert_eq!(evidence_count.consumed(), 1);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn final_observer_cancellation_rolls_back_output_and_retains_recovery_evidence() {
    let request = request_for(
        RecoveryExit::Success,
        None,
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
        ResourceLimits::default(),
    );
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

    let failure = operation_result(import_bytes(
        &request,
        AssetPath::new(SOURCE).unwrap(),
        SOURCE_BYTES,
        control,
    ))
    .expect_err("final observer cancellation must roll back publication");

    assert_eq!(final_events.load(Ordering::Acquire), 1);
    assert_recovery_evidence(failure.loss_evidence(), ImportLossDisposition::Applied);
    assert_eq!(failure.diagnostics().len(), 1);
    assert_eq!(failure.diagnostics()[0].code().as_str(), RECOVERY_CODE);
    let output = failure
        .ledger()
        .dimension(ImportDimension::CanonicalOutputBytes)
        .unwrap();
    assert_eq!(output.committed(), 0);
    assert_eq!(output.pending_publication(), 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(failure.finalization_cause().is_none());
}

#[test]
fn common_policies_reject_recovery_after_retaining_the_typed_disposition() {
    for (conformance, loss_policy, expected) in [
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
        let failure = run(
            RecoveryExit::Success,
            None,
            conformance,
            loss_policy,
            ResourceLimits::default(),
        )
        .expect_err("common policy must reject recovery");

        assert_recovery_evidence(failure.loss_evidence(), expected);
        assert_eq!(failure.diagnostics().len(), 1);
        let ImportCause::Scope(abort) = failure.cause() else {
            panic!("policy rejection must remain a scope-owned import cause")
        };
        assert_eq!(abort.owner(), CauseOwner::Import);
        match expected {
            ImportLossDisposition::RejectedByConformance => assert!(matches!(
                abort.cause(),
                ScopeAbortCause::FormatRecoveryRejectedByConformance { code }
                    if code.as_str() == RECOVERY_CODE
            )),
            ImportLossDisposition::RejectedByLossPolicy => assert!(matches!(
                abort.cause(),
                ScopeAbortCause::FormatRecoveryRejectedByLossPolicy { code }
                    if code.as_str() == RECOVERY_CODE
            )),
            ImportLossDisposition::Applied => unreachable!(),
        }
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
}

#[test]
fn later_format_validation_and_cancellation_failures_retain_recovery_evidence() {
    for exit in [
        RecoveryExit::FormatFailure,
        RecoveryExit::ValidationFailure,
        RecoveryExit::Cancellation,
    ] {
        let cancellation = (exit == RecoveryExit::Cancellation).then(CancellationToken::new);
        let failure = run(
            exit,
            cancellation,
            ConformancePolicy::AllowDeclaredRecovery,
            recovery_loss_policy(),
            ResourceLimits::default(),
        )
        .expect_err("the injected post-recovery failure must remain terminal");

        assert_recovery_evidence(failure.loss_evidence(), ImportLossDisposition::Applied);
        assert_eq!(failure.diagnostics().len(), 1);
        match (exit, failure.cause()) {
            (RecoveryExit::FormatFailure, ImportCause::Format(_))
            | (RecoveryExit::ValidationFailure, ImportCause::Validation(_))
            | (RecoveryExit::Cancellation, ImportCause::Scope(_)) => {}
            _ => panic!("{exit:?}: wrong terminal cause: {:?}", failure.cause()),
        }
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
}

#[test]
fn recovery_and_loss_evidence_limits_accept_exact_values_and_reject_excess() {
    let baseline = run(
        RecoveryExit::Success,
        None,
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
        ResourceLimits::default(),
    )
    .unwrap();
    let exact_evidence_bytes = baseline
        .ledger()
        .dimension(ImportDimension::LossEvidenceBytes)
        .unwrap()
        .consumed();
    assert!(exact_evidence_bytes > 0);

    let mut exact = ResourceLimits::default();
    exact.max_attempted_recoveries = 1;
    exact.max_loss_evidence = 1;
    exact.max_attempted_recovery_bytes = 3;
    exact.max_loss_evidence_bytes = exact_evidence_bytes;
    run(
        RecoveryExit::Success,
        None,
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
        exact.clone(),
    )
    .expect("exact recovery and evidence limits must pass");

    enum Excess {
        AttemptedCount,
        StoredCount,
        RawBytes,
        StoredBytes,
    }
    for excess_kind in [
        Excess::AttemptedCount,
        Excess::StoredCount,
        Excess::RawBytes,
        Excess::StoredBytes,
    ] {
        let mut excess = exact.clone();
        match excess_kind {
            Excess::AttemptedCount => excess.max_attempted_recoveries = 0,
            Excess::StoredCount => excess.max_loss_evidence = 0,
            Excess::RawBytes => excess.max_attempted_recovery_bytes = 2,
            Excess::StoredBytes => {
                excess.max_loss_evidence_bytes = exact_evidence_bytes - 1;
            }
        }
        let failure = run(
            RecoveryExit::Success,
            None,
            ConformancePolicy::AllowDeclaredRecovery,
            recovery_loss_policy(),
            excess,
        )
        .expect_err("one recovery or evidence unit beyond the limit must fail");
        assert!(matches!(failure.cause(), ImportCause::Scope(_)));
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
}
