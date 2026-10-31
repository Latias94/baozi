mod support;

use baozi::{
    CancellationToken, CapabilityStatus, ConformancePolicy, DiagnosticCode, DiagnosticSeverity,
    Error, FormatCapability, FormatDescriptor, FormatEncoding, FormatFailureEvidence, FormatId,
    ImportCause, ImportConfigOverlay, ImportOperationControl, ImportPhase, ImportProgressPhase,
    ImportProgressUnit, ImportRequest, ImportRequestLayer, Importer, OptionCompileLimits,
    ProgressDecision, ProgressDeliveryLimits, ProgressEvent, ReadConfidence,
};
use baozi_import::{
    DetectionOutcome, DetectionResult, DetectionScope, FormatFailureDetailSink,
    FormatFailureDetailValue, FormatFailureEvidenceProducer, FormatImporter, ParseResult,
    ParseScope, Probe, ReadHint, SceneDraft,
};
use std::error::Error as StdError;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const FORMAT_ID: &str = "org.example.facade-test";

struct TestImporter {
    fail: bool,
    failure_drops: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct TestFormatFailure {
    code: String,
    detail_key: String,
    detail_value: String,
    drops: Arc<AtomicUsize>,
}

impl TestFormatFailure {
    fn new(drops: Arc<AtomicUsize>) -> Self {
        Self {
            code: String::from("test.intentional_failure"),
            detail_key: String::from("fixture_name"),
            detail_value: String::from("facade-runtime-value"),
            drops,
        }
    }
}

impl Drop for TestFormatFailure {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

impl FormatFailureEvidenceProducer for TestFormatFailure {
    fn code(&self) -> &str {
        &self.code
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        sink.unsigned("fixture", 1);
        sink.string(&self.detail_key, &self.detail_value);
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

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        if self.fail {
            return Err(
                scope.format_failure(TestFormatFailure::new(Arc::clone(&self.failure_drops)))
            );
        }
        scope.emit_diagnostic(
            DiagnosticSeverity::Warning,
            DiagnosticCode::new("test.imported").expect("valid diagnostic code"),
            format_args!("test importer completed"),
        )?;
        support::submit_minimal_scene(scope)
    }
}

fn test_importer(fail: bool) -> Importer {
    test_importer_with_failure_probe(fail).0
}

fn test_importer_with_failure_probe(fail: bool) -> (Importer, Arc<AtomicUsize>) {
    let failure_drops = Arc::new(AtomicUsize::new(0));
    let mut builder = Importer::builder();
    builder
        .register(TestImporter {
            fail,
            failure_drops: Arc::clone(&failure_drops),
        })
        .expect("test importer must register");
    (
        builder.build().expect("test registry must freeze"),
        failure_drops,
    )
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
    assert!(!cancellation.is_cancelled());
    drop(import_control);
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
    let (importer, failure_drops) = test_importer_with_failure_probe(true);
    let error = importer
        .read_bytes("model.facade-test", b"opaque")
        .expect_err("format fixture must fail");
    assert_eq!(failure_drops.load(Ordering::Acquire), 1);
    drop(importer);

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
    let runtime_detail = envelope
        .details()
        .iter()
        .find(|detail| detail.key() == "fixture_name")
        .expect("host evidence must copy the runtime detail key");
    let FormatFailureDetailValue::String(runtime_value) = runtime_detail.value() else {
        panic!("expected the copied runtime detail value")
    };
    assert_eq!(runtime_value.as_str(), "facade-runtime-value");
    let evidence = StdError::source(envelope).expect("envelope must expose host-owned evidence");
    assert!(evidence.downcast_ref::<FormatFailureEvidence>().is_some());
    assert!(evidence.source().is_none());
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
