use baozi::DiagnosticSeverity;
use baozi::ReadSeek;
use baozi::{
    BaoziError, CapabilityStatus, Diagnostic, FormatCapability, FormatImporter, FormatInfo,
    FormatMaturity, ImportContext, Importer, ReadConfidence, ReadHint, Result, Scene, SceneBuilder,
};

struct ExtensionOnlyImporter;
struct RefusingImporter;
struct ContentImporter;
struct AmbiguousOne;
struct AmbiguousTwo;

static DUM_EXTENSIONS: &[&str] = &["dum"];
static BIN_EXTENSIONS: &[&str] = &["bin"];
static ONE_EXTENSIONS: &[&str] = &["one"];
static TWO_EXTENSIONS: &[&str] = &["two"];

fn dummy_info(id: &'static str, extensions: &'static [&'static str]) -> FormatInfo {
    FormatInfo {
        id,
        display_name: id,
        extensions,
        maturity: FormatMaturity::Experimental,
        capabilities: &[(FormatCapability::Geometry, CapabilityStatus::Unknown)],
        notes: "test importer",
    }
}

fn read_scene(ctx: &mut ImportContext<'_>) -> Result<Scene> {
    ctx.push_diagnostic(Diagnostic::warning("test.imported", "dummy importer ran"));
    SceneBuilder::new().finish()
}

impl FormatImporter for ExtensionOnlyImporter {
    fn info(&self) -> FormatInfo {
        dummy_info("extension-only", DUM_EXTENSIONS)
    }

    fn can_read(&self, _input: &mut dyn ReadSeek, _hint: &ReadHint) -> Result<ReadConfidence> {
        Ok(ReadConfidence::Maybe)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for RefusingImporter {
    fn info(&self) -> FormatInfo {
        dummy_info("refusing", DUM_EXTENSIONS)
    }

    fn can_read(&self, _input: &mut dyn ReadSeek, _hint: &ReadHint) -> Result<ReadConfidence> {
        Ok(ReadConfidence::No)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for ContentImporter {
    fn info(&self) -> FormatInfo {
        dummy_info("content", BIN_EXTENSIONS)
    }

    fn can_read(&self, input: &mut dyn ReadSeek, _hint: &ReadHint) -> Result<ReadConfidence> {
        let mut bytes = [0; 5];
        match input.read_exact(&mut bytes) {
            Ok(()) if &bytes == b"baozi" => Ok(ReadConfidence::Certain),
            Ok(()) => Ok(ReadConfidence::No),
            Err(_) => Ok(ReadConfidence::No),
        }
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for AmbiguousOne {
    fn info(&self) -> FormatInfo {
        dummy_info("ambiguous-one", ONE_EXTENSIONS)
    }

    fn can_read(&self, _input: &mut dyn ReadSeek, _hint: &ReadHint) -> Result<ReadConfidence> {
        Ok(ReadConfidence::Likely)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        read_scene(ctx)
    }
}

impl FormatImporter for AmbiguousTwo {
    fn info(&self) -> FormatInfo {
        dummy_info("ambiguous-two", TWO_EXTENSIONS)
    }

    fn can_read(&self, _input: &mut dyn ReadSeek, _hint: &ReadHint) -> Result<ReadConfidence> {
        Ok(ReadConfidence::Likely)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        read_scene(ctx)
    }
}

#[test]
fn unknown_bytes_report_unsupported_format() {
    let importer = Importer::empty();

    let error = importer
        .read_bytes("model.unknown", b"not a model")
        .unwrap_err();

    assert!(matches!(error, BaoziError::UnsupportedFormat { .. }));
}

#[test]
fn extension_hint_selects_importer_when_content_does_not_contradict() {
    let mut importer = Importer::empty();
    importer.register(ExtensionOnlyImporter);

    let report = importer.read_bytes("model.dum", b"opaque bytes").unwrap();

    assert_eq!(report.scene.nodes.len(), 1);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].severity, DiagnosticSeverity::Warning);
}

#[test]
fn extension_hint_does_not_select_importer_that_rejects_content() {
    let mut importer = Importer::empty();
    importer.register(RefusingImporter);

    let error = importer
        .read_bytes("model.dum", b"contradiction")
        .unwrap_err();

    assert!(matches!(error, BaoziError::UnsupportedFormat { .. }));
}

#[test]
fn content_detection_beats_wrong_extension() {
    let mut importer = Importer::empty();
    importer.register(ExtensionOnlyImporter);
    importer.register(ContentImporter);

    let report = importer.read_bytes("model.dum", b"baozi payload").unwrap();

    assert_eq!(report.format.id, "content");
}

#[test]
fn ambiguous_top_confidence_matches_return_error() {
    let mut importer = Importer::empty();
    importer.register(AmbiguousOne);
    importer.register(AmbiguousTwo);

    let error = importer.read_bytes("model.bin", b"anything").unwrap_err();

    assert!(matches!(error, BaoziError::AmbiguousFormat { .. }));
}
