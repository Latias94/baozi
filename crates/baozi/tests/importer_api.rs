use baozi::DiagnosticSeverity;
use baozi::{
    BaoziError, CapabilityStatus, DetectionFailure, Diagnostic, FormatCapability, FormatInfo,
    FormatMaturity, ImportFailure, ImportStage, Importer, Scene, SceneBuilder,
};
use baozi_core::Result as CoreResult;
use baozi_import::{
    DetectionOutcome, FormatImporter, ImportContext, Probe, ReadConfidence, ReadHint,
};

struct ExtensionOnlyImporter;
struct RefusingImporter;
struct ContentImporter;
struct AmbiguousOne;
struct AmbiguousTwo;
struct FaultingDetector;
struct InconclusiveDetector;
struct ParseFailImporter;

static DUM_EXTENSIONS: &[&str] = &["dum"];
static BIN_EXTENSIONS: &[&str] = &["bin"];
static ONE_EXTENSIONS: &[&str] = &["one"];
static TWO_EXTENSIONS: &[&str] = &["two"];
static FAIL_EXTENSIONS: &[&str] = &["fail"];

fn dummy_info(id: &'static str, extensions: &'static [&'static str]) -> FormatInfo {
    FormatInfo::new(id, id, extensions)
        .with_maturity(FormatMaturity::Experimental)
        .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Unknown)])
        .with_notes("test importer")
}

fn read_scene(ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
    ctx.push_diagnostic(Diagnostic::warning("test.imported", "dummy importer ran"));
    SceneBuilder::new().finish()
}

impl FormatImporter for ExtensionOnlyImporter {
    fn info(&self) -> FormatInfo {
        dummy_info("extension-only", DUM_EXTENSIONS)
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Maybe)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for RefusingImporter {
    fn info(&self) -> FormatInfo {
        dummy_info("refusing", DUM_EXTENSIONS)
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::NoMatch
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for ContentImporter {
    fn info(&self) -> FormatInfo {
        dummy_info("content", BIN_EXTENSIONS)
    }

    fn can_read(&self, probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        if probe.bytes().starts_with(b"baozi") {
            DetectionOutcome::Match(ReadConfidence::Certain)
        } else {
            DetectionOutcome::NoMatch
        }
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for AmbiguousOne {
    fn info(&self) -> FormatInfo {
        dummy_info("ambiguous-one", ONE_EXTENSIONS)
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Likely)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for AmbiguousTwo {
    fn info(&self) -> FormatInfo {
        dummy_info("ambiguous-two", TWO_EXTENSIONS)
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Likely)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for FaultingDetector {
    fn info(&self) -> FormatInfo {
        dummy_info("a-fault", &[])
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::fault("detector failed")
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for InconclusiveDetector {
    fn info(&self) -> FormatInfo {
        dummy_info("z-inconclusive", &[])
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Inconclusive(baozi::DetectionInconclusive::ProbeExhausted)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for ParseFailImporter {
    fn info(&self) -> FormatInfo {
        dummy_info("parse-fail", FAIL_EXTENSIONS)
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        Err(BaoziError::parse(
            ctx.source().to_string(),
            None,
            "intentional parse failure",
        ))
    }
}

#[test]
fn unknown_bytes_report_unsupported_format() {
    let importer = Importer::empty();

    let error = importer
        .read_bytes("model.unknown", b"not a model")
        .unwrap_err();

    assert!(matches!(
        error,
        ImportFailure::UnsupportedFormat { evidence, .. } if evidence.is_empty()
    ));
}

#[test]
fn in_memory_total_byte_limit_is_checked_before_detection() {
    let mut options = baozi::ImportOptions::memory();
    options.limits.max_total_asset_bytes = 1;

    let error = Importer::empty()
        .read_bytes_with_options("model.bin", b"too large", options)
        .unwrap_err();

    assert!(matches!(
        error,
        ImportFailure::Operation {
            error: BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes"
            },
            evidence,
        } if evidence.is_empty()
    ));
}

#[test]
fn extension_hint_selects_importer_when_content_does_not_contradict() {
    let mut importer = Importer::empty();
    importer.register(ExtensionOnlyImporter).unwrap();

    let report = importer.read_bytes("model.dum", b"opaque bytes").unwrap();

    assert_eq!(report.scene().nodes.len(), 1);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].severity,
        DiagnosticSeverity::Warning
    );
    assert_eq!(report.stage(), ImportStage::ValidatedImported);
    assert_eq!(report.stats().diagnostics_emitted(), 1);
}

#[test]
fn strict_diagnostics_promote_warning_to_error() {
    let mut importer = Importer::empty();
    importer.register(ExtensionOnlyImporter).unwrap();
    let mut options = baozi::ImportOptions::memory();
    options.diagnostics.strict = true;

    let error = importer
        .read_bytes_with_options("model.dum", b"opaque bytes", options)
        .unwrap_err();

    assert!(matches!(
        error,
        ImportFailure::Operation {
            error: BaoziError::Parse { .. },
            ..
        }
    ));
    assert!(error.to_string().contains("test.imported"));
}

#[test]
fn strict_diagnostics_cannot_be_bypassed_by_diagnostic_cap() {
    let mut importer = Importer::empty();
    importer.register(ExtensionOnlyImporter).unwrap();
    let mut options = baozi::ImportOptions::memory();
    options.diagnostics.strict = true;
    options.limits.max_diagnostics = 0;

    let error = importer
        .read_bytes_with_options("model.dum", b"opaque bytes", options)
        .unwrap_err();

    assert!(matches!(
        error,
        ImportFailure::Operation {
            error: BaoziError::Parse { .. },
            ..
        }
    ));
    assert!(error.to_string().contains("test.imported"));
}

#[test]
fn extension_hint_does_not_select_importer_that_rejects_content() {
    let mut importer = Importer::empty();
    importer.register(RefusingImporter).unwrap();

    let error = importer
        .read_bytes("model.dum", b"contradiction")
        .unwrap_err();

    assert!(matches!(
        error,
        ImportFailure::UnsupportedFormat { evidence, .. }
            if evidence.len() == 1
                && evidence[0].outcome() == &DetectionOutcome::NoMatch
    ));
}

#[test]
fn content_detection_beats_wrong_extension() {
    let mut importer = Importer::empty();
    importer.register(FaultingDetector).unwrap();
    importer.register(ExtensionOnlyImporter).unwrap();
    importer.register(ContentImporter).unwrap();
    importer.register(InconclusiveDetector).unwrap();

    let report = importer.read_bytes("model.dum", b"baozi payload").unwrap();

    assert_eq!(report.format().id(), "content");
    assert_eq!(
        report
            .detection_evidence()
            .iter()
            .map(|item| item.format_id())
            .collect::<Vec<_>>(),
        ["a-fault", "content", "extension-only", "z-inconclusive"]
    );
    assert!(matches!(
        report.detection_evidence()[0].outcome(),
        DetectionOutcome::Fault(_)
    ));
    assert_eq!(
        report.detection_evidence()[1].outcome(),
        &DetectionOutcome::Match(ReadConfidence::Certain)
    );
    assert!(matches!(
        report.detection_evidence()[3].outcome(),
        DetectionOutcome::Inconclusive(_)
    ));
}

#[test]
fn non_definitive_detection_is_not_reported_as_unsupported() {
    let mut importer = Importer::empty();
    importer.register(FaultingDetector).unwrap();
    importer.register(ExtensionOnlyImporter).unwrap();
    importer.register(InconclusiveDetector).unwrap();

    let error = importer.read_bytes("model.unknown", b"opaque").unwrap_err();

    let ImportFailure::Detection(DetectionFailure::NoMatch { evidence, .. }) = error else {
        panic!("expected a typed aggregate detection failure");
    };
    assert_eq!(
        evidence
            .iter()
            .map(|item| item.format_id())
            .collect::<Vec<_>>(),
        ["a-fault", "extension-only", "z-inconclusive"]
    );
    assert!(matches!(evidence[0].outcome(), DetectionOutcome::Fault(_)));
    assert_eq!(
        evidence[1].outcome(),
        &DetectionOutcome::Match(ReadConfidence::Maybe)
    );
    assert!(matches!(
        evidence[2].outcome(),
        DetectionOutcome::Inconclusive(_)
    ));
}

#[test]
fn selected_format_failure_preserves_detection_evidence() {
    let mut importer = Importer::empty();
    importer.register(FaultingDetector).unwrap();
    importer.register(ParseFailImporter).unwrap();

    let error = importer.read_bytes("model.fail", b"opaque").unwrap_err();

    let ImportFailure::Operation { error, evidence } = error else {
        panic!("expected a selected-format operation failure");
    };
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert_eq!(
        evidence
            .iter()
            .map(|item| item.format_id())
            .collect::<Vec<_>>(),
        ["a-fault", "parse-fail"]
    );
}

#[test]
fn ambiguous_top_confidence_matches_return_error() {
    let mut importer = Importer::empty();
    importer.register(FaultingDetector).unwrap();
    importer.register(AmbiguousOne).unwrap();
    importer.register(AmbiguousTwo).unwrap();
    importer.register(InconclusiveDetector).unwrap();

    let error = importer.read_bytes("model.bin", b"anything").unwrap_err();

    assert!(matches!(
        error,
        ImportFailure::Detection(DetectionFailure::Ambiguous {
            candidates,
            evidence,
            ..
        }) if candidates == ["ambiguous-one", "ambiguous-two"] && evidence.len() == 4
    ));
}
