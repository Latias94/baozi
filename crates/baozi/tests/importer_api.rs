use baozi::{
    CancellationToken, CapabilityStatus, ConformancePolicy, DiagnosticCode, DiagnosticSeverity,
    Error, FormatCapability, FormatDescriptor, FormatEncoding, FormatId, FormatMaturity,
    ImportCause, ImportConfigOverlay, ImportOperationControl, ImportPhase, ImportProgressPhase,
    ImportProgressUnit, ImportRequest, ImportRequestLayer, Importer, OptionCompileLimits,
    PostProcessControl, PostProcessProgressPhase, PostProcessProgressUnit, ProgressDecision,
    ProgressDeliveryLimits, ProgressEvent, ReadConfidence,
};
use baozi_import::{
    DetectionOutcome, DetectionResult, DetectionScope, FormatFailureDetailSink,
    FormatFailureDetailValue, FormatFailureSource, FormatImporter, ParseResult, ParseScope, Probe,
    ReadHint,
};
use std::error::Error as StdError;
use std::fmt;

const FORMAT_ID: &str = "org.example.facade-test";

struct TestImporter {
    fail: bool,
}

#[derive(Debug)]
struct TestFormatFailure;

impl fmt::Display for TestFormatFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("intentional test format failure")
    }
}

impl StdError for TestFormatFailure {}

impl FormatFailureSource for TestFormatFailure {
    fn code(&self) -> &'static str {
        "test.intentional_failure"
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        sink.unsigned("fixture", 1);
    }
}

impl FormatImporter for TestImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new(FORMAT_ID).expect("qualified test format id"),
            "Facade test format",
        )
        .extensions(["facade-test"])
        .encoding(FormatEncoding::Binary)
        .maturity(FormatMaturity::Experimental)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Unknown)])
        .build()
        .expect("valid test descriptor")
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
        if self.fail {
            return Err(scope.format_failure(TestFormatFailure));
        }
        scope.emit_diagnostic(
            DiagnosticSeverity::Warning,
            DiagnosticCode::new("test.imported").expect("valid diagnostic code"),
            format_args!("test importer completed"),
        )?;
        scope.scene_builder()?.finish_draft()
    }
}

fn test_importer(fail: bool) -> Importer {
    let mut builder = Importer::builder();
    builder
        .register(TestImporter { fail })
        .expect("test importer must register");
    builder.build().expect("test registry must freeze")
}

#[test]
fn facade_exposes_operation_control_construction_types() {
    let cancellation = CancellationToken::new();
    let import_control = ImportOperationControl::default()
        .with_cancellation(cancellation.clone())
        .with_progress_limits(ProgressDeliveryLimits::new(8, 512, 1))
        .with_progress_observer(
            |_event: ProgressEvent<ImportProgressPhase, ImportProgressUnit>| {
                ProgressDecision::Continue
            },
        );
    let postprocess_control = PostProcessControl::default()
        .with_cancellation(cancellation.clone())
        .with_progress_limits(ProgressDeliveryLimits::new(8, 512, 1))
        .with_progress_observer(
            |_event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
                ProgressDecision::Continue
            },
        );

    assert!(!cancellation.is_cancelled());
    drop((import_control, postprocess_control));
}

#[test]
fn empty_registry_returns_structured_import_failure() {
    let importer = Importer::empty().expect("empty registry must freeze");
    let error = importer
        .read_bytes("model.unknown", b"not a model")
        .expect_err("empty registry cannot select a format");

    let Error::Import(failure) = error else {
        panic!("expected a post-session import failure");
    };
    assert!(matches!(failure.cause(), ImportCause::UnsupportedFormat));
    assert!(failure.detection_evidence().is_empty());
    assert_eq!(failure.last_completed().phase(), ImportPhase::PrimaryBound);
}

#[test]
fn facade_preserves_success_evidence_from_the_session() {
    let importer = test_importer(false);
    let report = importer
        .read_bytes("model.facade-test", b"opaque")
        .expect("test import must succeed");

    assert_eq!(report.format().id().as_str(), FORMAT_ID);
    assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
    assert_eq!(report.detection_evidence().len(), 1);
    assert_eq!(report.detection_evidence()[0].format().as_str(), FORMAT_ID);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].severity(),
        DiagnosticSeverity::Warning
    );
    assert_eq!(report.diagnostics()[0].code().as_str(), "test.imported");
    assert_eq!(report.diagnostic_summary().stored_count(), 1);
}

#[test]
fn facade_preserves_format_envelope_and_detection_evidence() {
    let importer = test_importer(true);
    let error = importer
        .read_bytes("model.facade-test", b"opaque")
        .expect_err("format fixture must fail");

    let Error::Import(failure) = error else {
        panic!("expected a post-session import failure");
    };
    let ImportCause::Format(envelope) = failure.cause() else {
        panic!("expected the format-owned cause");
    };
    assert_eq!(envelope.format().as_str(), FORMAT_ID);
    assert_eq!(envelope.code(), "test.intentional_failure");
    let fixture = envelope
        .details()
        .iter()
        .find(|detail| detail.key() == "fixture")
        .expect("sealed format evidence must preserve the fixture detail");
    assert!(matches!(
        fixture.value(),
        FormatFailureDetailValue::Unsigned(1)
    ));
    assert_eq!(failure.detection_evidence().len(), 1);
    assert_eq!(
        failure.selected_format().map(FormatId::as_str),
        Some(FORMAT_ID)
    );
}

#[test]
fn strict_conformance_does_not_promote_arbitrary_diagnostics() {
    let importer = test_importer(false);
    let options = ImportRequest::from_layers(
        ImportRequestLayer::default(),
        ImportRequestLayer::new(
            ImportConfigOverlay::default().with_conformance_policy(ConformancePolicy::Strict),
        ),
    );
    let request = importer
        .prepare_request(&options, OptionCompileLimits::default())
        .expect("request must compile");
    let report = importer
        .read_bytes_with_request(
            "model.facade-test",
            b"opaque",
            &request,
            ImportOperationControl::default(),
        )
        .expect("diagnostic severity is not conformance authority");
    assert_eq!(report.diagnostic_summary().attempted_count(), 1);
}
