mod support;

use baozi_core::FormatId;
use baozi_import::{
    CapabilityStatus, CauseOwner, ConformancePolicy, DetectionOutcome, DetectionResult,
    DetectionScope, DiagnosticByteRange, DiagnosticSeverity, FormatCapability, FormatDescriptor,
    FormatEncoding, FormatFailureEvidenceProducer, FormatImporter, ImportCause,
    ImportConfigOverlay, ImportDimension, ImportFailure, ImportLossDisposition, ImportLossKind,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, ImportResult,
    LossPolicy, OptionCompileLimits, ParseResult, ParseScope, PreparedImportRequest,
    ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder, ResourceLimits,
    ResourceLimitsOverlay, SceneDraft, ScopeAbortCause, TextBomPolicy,
    TextConformanceViolationKind, TextControlPolicy, TextDecodingMode, TextHardCaps,
    TextInputOptions, TextLexicalMode, TextNewline, TextProfile, TextSourceEncoding,
    import_bytes as run_import_bytes,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SOURCE: &str = "scene.text";
const FORMAT_ID: &str = "dev.baozi.test.text-input-conformance";
const FORMAT_FAILURE_CODE: &str = "test.text_conformance_violation";
const INVALID_UTF8_REPLACEMENT_CODE: &str = "import.text.invalid_utf8_replaced";

#[derive(Debug, Clone, Copy)]
struct ExpectedReplacement {
    raw_start: u64,
    raw_end: u64,
    decoded_start: usize,
    decoded_end: usize,
}

#[derive(Debug, Clone, Copy)]
struct ExpectedAccepted {
    text: &'static str,
    bom: Option<(u64, u64)>,
    line_starts: &'static [u64],
    replacements: &'static [ExpectedReplacement],
}

#[derive(Debug, Clone, Copy)]
enum ParserBehavior {
    ExpectAccepted {
        options: TextInputOptions,
        expected: ExpectedAccepted,
    },
    ExpectRejected {
        options: TextInputOptions,
        expected_kind: TextConformanceViolationKind,
        expected_range: Option<(u64, u64)>,
    },
    PropagateScopeFailure {
        options: TextInputOptions,
    },
}

struct TextInputImporter {
    descriptor: FormatDescriptor,
    behavior: ParserBehavior,
    parse_calls: Arc<AtomicUsize>,
}

impl TextInputImporter {
    fn new(descriptor: FormatDescriptor, behavior: ParserBehavior) -> (Self, Arc<AtomicUsize>) {
        let parse_calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                descriptor,
                behavior,
                parse_calls: Arc::clone(&parse_calls),
            },
            parse_calls,
        )
    }
}

impl FormatImporter for TextInputImporter {
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
        match self.behavior {
            ParserBehavior::ExpectAccepted { options, expected } => {
                let accepted = scope
                    .conform_text_input(scope.primary_bytes(), options)?
                    .expect("test text input must conform");
                assert_eq!(accepted.text(), expected.text);
                assert_eq!(
                    accepted.bom().map(|bom| bom.raw_range()),
                    expected
                        .bom
                        .map(|(start, end)| DiagnosticByteRange::new(start, end)
                            .expect("expected BOM range must be valid"))
                );
                assert_eq!(accepted.line_start_offsets(), expected.line_starts);
                assert_eq!(accepted.replacements().len(), expected.replacements.len());
                for (actual, expected) in accepted.replacements().iter().zip(expected.replacements)
                {
                    assert_eq!(
                        actual.raw_range(),
                        DiagnosticByteRange::new(expected.raw_start, expected.raw_end)
                            .expect("expected replacement raw range must be valid")
                    );
                    assert_eq!(actual.decoded_start(), expected.decoded_start);
                    assert_eq!(actual.decoded_end(), expected.decoded_end);
                }
                drop(accepted);
                support::submit_minimal_scene(scope)
            }
            ParserBehavior::ExpectRejected {
                options,
                expected_kind,
                expected_range,
            } => {
                let rejected = scope
                    .conform_text_input(scope.primary_bytes(), options)?
                    .expect_err("test text input must be rejected");
                assert_eq!(rejected.kind(), expected_kind);
                assert_eq!(
                    rejected.raw_range(),
                    expected_range.map(|(start, end)| {
                        DiagnosticByteRange::new(start, end)
                            .expect("expected violation range must be valid")
                    })
                );
                Err(scope.format_failure(TestTextFailure))
            }
            ParserBehavior::PropagateScopeFailure { options } => {
                let accepted = scope
                    .conform_text_input(scope.primary_bytes(), options)?
                    .expect("test text input should only fail through the scope");
                drop(accepted);
                support::submit_minimal_scene(scope)
            }
        }
    }
}

struct TestTextFailure;

impl FormatFailureEvidenceProducer for TestTextFailure {
    fn code(&self) -> &str {
        FORMAT_FAILURE_CODE
    }
}

fn descriptor(profile: TextProfile) -> FormatDescriptor {
    FormatDescriptor::builder(
        FormatId::new(FORMAT_ID).expect("static format id must be valid"),
        "Text Input Conformance Contract",
    )
    .extensions(["txt"])
    .encoding(FormatEncoding::Text)
    .text_profile(profile)
    .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
    .build()
    .expect("test descriptor must be valid")
}

fn binary_descriptor() -> FormatDescriptor {
    FormatDescriptor::builder(
        FormatId::new(FORMAT_ID).expect("static format id must be valid"),
        "Binary Descriptor Text Input Contract",
    )
    .extensions(["bin"])
    .encoding(FormatEncoding::Binary)
    .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
    .build()
    .expect("test binary descriptor must be valid")
}

fn text_profile(
    bom_policy: TextBomPolicy,
    newlines: impl IntoIterator<Item = TextNewline>,
    decoding_modes: impl IntoIterator<Item = TextDecodingMode>,
    control_policy: TextControlPolicy,
) -> TextProfile {
    text_profile_with_lexical(
        bom_policy,
        newlines,
        decoding_modes,
        TextLexicalMode::Utf8Document,
        control_policy,
    )
}

fn text_profile_with_lexical(
    bom_policy: TextBomPolicy,
    newlines: impl IntoIterator<Item = TextNewline>,
    decoding_modes: impl IntoIterator<Item = TextDecodingMode>,
    lexical_mode: TextLexicalMode,
    control_policy: TextControlPolicy,
) -> TextProfile {
    TextProfile::builder(
        TextHardCaps::new(1024, 1024, 1024, 1024, 1024)
            .expect("positive text hard caps must be valid"),
    )
    .source_encodings([TextSourceEncoding::Utf8])
    .bom_policy(bom_policy)
    .newlines(newlines)
    .lexical_mode(lexical_mode)
    .decoding_modes(decoding_modes)
    .control_policy(control_policy)
    .build()
    .expect("test text profile must be valid")
}

fn request_for(importer: TextInputImporter, limits: ResourceLimits) -> PreparedImportRequest {
    request_for_with_policies(
        importer,
        limits,
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    )
}

fn request_for_with_policies(
    importer: TextInputImporter,
    limits: ResourceLimits,
    conformance_policy: ConformancePolicy,
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
        .with_conformance_policy(conformance_policy)
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
        baozi_io::AssetPath::new(SOURCE).expect("static source path must be valid"),
        bytes,
        ImportOperationControl::default(),
    ))
}

fn assert_replacement_recovery(
    report: &ImportReport,
    expected_ranges: impl IntoIterator<Item = (u64, u64)>,
) {
    let expected_ranges: Vec<(u64, u64)> = expected_ranges.into_iter().collect();
    assert_eq!(report.loss_evidence().len(), expected_ranges.len());
    assert_eq!(report.diagnostics().len(), expected_ranges.len());
    for (index, (start, end)) in expected_ranges.into_iter().enumerate() {
        let range = DiagnosticByteRange::new(start, end).expect("expected range must be valid");
        let evidence = &report.loss_evidence()[index];
        assert_eq!(evidence.kind(), ImportLossKind::FormatRecovery);
        assert_eq!(evidence.disposition(), ImportLossDisposition::Applied);
        assert_eq!(evidence.code().as_str(), INVALID_UTF8_REPLACEMENT_CODE);
        assert_eq!(evidence.raw_range(), Some(range));

        let diagnostic = &report.diagnostics()[index];
        assert_eq!(diagnostic.severity(), DiagnosticSeverity::Warning);
        assert_eq!(diagnostic.code().as_str(), INVALID_UTF8_REPLACEMENT_CODE);
        assert_eq!(diagnostic.source().as_str(), SOURCE);
        assert_eq!(diagnostic.byte_range(), Some(range));
    }
}

fn assert_no_text_storage(failure: &ImportFailure) {
    for dimension in [
        ImportDimension::DecodedTextBytes,
        ImportDimension::LineIndexBytes,
    ] {
        if let Some(evidence) = failure.ledger().dimension(dimension) {
            assert_eq!(evidence.attempted(), 0);
            assert_eq!(evidence.peak_live_bytes(), 0);
        }
    }
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

fn assert_no_decoded_text_storage(failure: &ImportFailure) {
    if let Some(evidence) = failure
        .ledger()
        .dimension(ImportDimension::DecodedTextBytes)
    {
        assert_eq!(evidence.attempted(), 0);
        assert_eq!(evidence.peak_live_bytes(), 0);
    }
}

fn assert_decode_work(failure: &ImportFailure, attempted: u64, consumed: u64) {
    let decode_work = failure
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .expect("decode work must be accounted");
    assert_eq!(decode_work.attempted(), attempted);
    assert_eq!(decode_work.consumed(), consumed);
}

fn assert_no_decode_work(failure: &ImportFailure) {
    if let Some(decode_work) = failure.ledger().dimension(ImportDimension::DecodeWork) {
        assert_eq!(decode_work.attempted(), 0);
        assert_eq!(decode_work.consumed(), 0);
    }
}

fn assert_format_violation(failure: &ImportFailure) {
    let ImportCause::Format(format) = failure.cause() else {
        panic!("text conformance violation must remain a format-owned failure")
    };
    assert!(matches!(
        failure.cause().owner(),
        CauseOwner::Format(format) if format.as_str() == FORMAT_ID
    ));
    assert_eq!(format.code(), FORMAT_FAILURE_CODE);
    assert_no_text_storage(failure);
}

fn assert_replacement_recovery_rejected(
    failure: &ImportFailure,
    expected_disposition: ImportLossDisposition,
    expected_range: (u64, u64),
) {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("replacement recovery policy rejection must remain import-owned")
    };
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    match expected_disposition {
        ImportLossDisposition::RejectedByConformance => assert!(matches!(
            abort.cause(),
            ScopeAbortCause::FormatRecoveryRejectedByConformance { code }
                if code.as_str() == INVALID_UTF8_REPLACEMENT_CODE
        )),
        ImportLossDisposition::RejectedByLossPolicy => assert!(matches!(
            abort.cause(),
            ScopeAbortCause::FormatRecoveryRejectedByLossPolicy { code }
                if code.as_str() == INVALID_UTF8_REPLACEMENT_CODE
        )),
        ImportLossDisposition::Applied => panic!("applied recovery must not be rejected"),
    }

    let range = DiagnosticByteRange::new(expected_range.0, expected_range.1)
        .expect("expected recovery range must be valid");
    assert_eq!(failure.loss_evidence().len(), 1);
    assert_eq!(
        failure.loss_evidence()[0].kind(),
        ImportLossKind::FormatRecovery
    );
    assert_eq!(
        failure.loss_evidence()[0].disposition(),
        expected_disposition
    );
    assert_eq!(
        failure.loss_evidence()[0].code().as_str(),
        INVALID_UTF8_REPLACEMENT_CODE
    );
    assert_eq!(failure.loss_evidence()[0].raw_range(), Some(range));
    assert_eq!(failure.diagnostics().len(), 1);
    assert_eq!(
        failure.diagnostics()[0].code().as_str(),
        INVALID_UTF8_REPLACEMENT_CODE
    );
    assert_eq!(failure.diagnostics()[0].byte_range(), Some(range));
    assert_no_decoded_text_storage(failure);
}

fn assert_scope_resource_limit(failure: &ImportFailure, expected_limit: u64) {
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit { limit, .. } if *limit == expected_limit
            )
    ));
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn strict_utf8_text_is_copied_with_raw_line_starts() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf, TextNewline::CrLf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::AllowHorizontalTab,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::ExpectAccepted {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
            expected: ExpectedAccepted {
                text: "v 1\r\n\tcaf\u{e9}\n",
                bom: None,
                line_starts: &[0, 5, 12],
                replacements: &[],
            },
        },
    );
    let request = request_for(importer, ResourceLimits::default());

    let report = import_bytes(&request, b"v 1\r\n\tcaf\xc3\xa9\n")
        .expect("valid profiled UTF-8 text must import");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn accepted_bom_is_classified_and_removed_without_shifting_raw_lines() {
    let profile = text_profile(
        TextBomPolicy::Accepted,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::ExpectAccepted {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
            expected: ExpectedAccepted {
                text: "o\n",
                bom: Some((0, 3)),
                line_starts: &[3, 5],
                replacements: &[],
            },
        },
    );
    let request = request_for(importer, ResourceLimits::default());

    let report =
        import_bytes(&request, b"\xef\xbb\xbfo\n").expect("accepted BOM must not reach grammar");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn raw_base_offsets_apply_to_bom_and_cr_lines() {
    let profile = text_profile(
        TextBomPolicy::AcceptedWithDiagnostic,
        [TextNewline::Cr],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::ExpectAccepted {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text")
                .with_raw_base(100),
            expected: ExpectedAccepted {
                text: "a\rz",
                bom: Some((100, 103)),
                line_starts: &[103, 105],
                replacements: &[],
            },
        },
    );
    let request = request_for(importer, ResourceLimits::default());

    let report = import_bytes(&request, b"\xef\xbb\xbfa\rz")
        .expect("raw-base text conformance must preserve shifted offsets");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn replace_invalid_utf8_records_raw_and_decoded_spans() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [
            TextDecodingMode::RejectInvalid,
            TextDecodingMode::ReplaceInvalidUtf8,
        ],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::ExpectAccepted {
            options: TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "test-text"),
            expected: ExpectedAccepted {
                text: "a\u{fffd}b\u{fffd}",
                bom: None,
                line_starts: &[0],
                replacements: &[
                    ExpectedReplacement {
                        raw_start: 1,
                        raw_end: 2,
                        decoded_start: 1,
                        decoded_end: 4,
                    },
                    ExpectedReplacement {
                        raw_start: 3,
                        raw_end: 5,
                        decoded_start: 5,
                        decoded_end: 8,
                    },
                ],
            },
        },
    );
    let request = request_for_with_policies(
        importer,
        ResourceLimits::default(),
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
    );

    let report =
        import_bytes(&request, b"a\xffb\xe2\x82").expect("declared recovery mode must conform");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_replacement_recovery(&report, [(1, 2), (3, 5)]);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn replacement_recovery_requires_common_policies_before_text_storage() {
    for (conformance_policy, loss_policy, expected_disposition) in [
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
        let profile = text_profile(
            TextBomPolicy::Forbidden,
            [TextNewline::Lf],
            [
                TextDecodingMode::RejectInvalid,
                TextDecodingMode::ReplaceInvalidUtf8,
            ],
            TextControlPolicy::RejectAll,
        );
        let (importer, parse_calls) = TextInputImporter::new(
            descriptor(profile),
            ParserBehavior::PropagateScopeFailure {
                options: TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "test-text"),
            },
        );
        let request = request_for_with_policies(
            importer,
            ResourceLimits::default(),
            conformance_policy,
            loss_policy,
        );

        let failure = import_bytes(&request, b"a\xff")
            .expect_err("common policy gates must reject replacement recovery");

        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
        assert_replacement_recovery_rejected(&failure, expected_disposition, (1, 2));
    }
}

#[test]
fn text_profile_declaration_violations_are_structured_without_storage() {
    for (descriptor, options, expected_kind) in [
        (
            binary_descriptor(),
            TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
            TextConformanceViolationKind::MissingTextProfile,
        ),
        (
            descriptor(text_profile(
                TextBomPolicy::Forbidden,
                [TextNewline::Lf],
                [TextDecodingMode::RejectInvalid],
                TextControlPolicy::RejectAll,
            )),
            TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "test-text"),
            TextConformanceViolationKind::DecodingModeNotDeclared,
        ),
    ] {
        let (importer, parse_calls) = TextInputImporter::new(
            descriptor,
            ParserBehavior::ExpectRejected {
                options,
                expected_kind,
                expected_range: None,
            },
        );
        let request = request_for(importer, ResourceLimits::default());

        let failure = import_bytes(&request, b"abc")
            .expect_err("text profile declaration violation must remain format-owned");

        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
        assert_format_violation(&failure);
        assert_no_decode_work(&failure);
    }
}

#[test]
fn text_raw_limit_fails_before_decoded_storage_is_exposed() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::PropagateScopeFailure {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text")
                .with_raw_limit("test-text-raw-bytes", 5),
        },
    );
    let request = request_for(importer, ResourceLimits::default());

    let failure = import_bytes(&request, b"abcdef")
        .expect_err("text raw surface cap must fail inside the parser scope");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_scope_resource_limit(&failure, 5);
    assert_no_text_storage(&failure);
}

#[test]
fn decoded_limit_fails_after_budgeted_scan_without_partial_text_storage() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::PropagateScopeFailure {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
        },
    );
    let mut limits = ResourceLimits::default();
    limits.max_decoded_utf8_bytes = 4;
    let request = request_for(importer, limits);

    let failure =
        import_bytes(&request, b"abcde").expect_err("decoded text cap must reject the input");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_scope_resource_limit(&failure, 4);
    let decode_work = failure
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .expect("decode work must be accounted for the failed scan");
    assert_eq!(decode_work.attempted(), 5);
    assert_eq!(decode_work.consumed(), 5);
    assert_no_text_storage(&failure);
}

#[test]
fn line_byte_limit_fails_after_budgeted_scan_without_partial_text_storage() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::PropagateScopeFailure {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
        },
    );
    let mut limits = ResourceLimits::default();
    limits.max_line_bytes = 3;
    let request = request_for(importer, limits);

    let failure = import_bytes(&request, b"abcd\n")
        .expect_err("line byte cap must reject an overlong raw line");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_scope_resource_limit(&failure, 3);
    assert_decode_work(&failure, 5, 5);
    assert_no_text_storage(&failure);
}

#[test]
fn text_profile_violations_are_structured_before_storage_allocation() {
    for (bytes, expected_kind, expected_range, expected_consumed) in [
        (
            b"\xef\xbb\xbfa".as_slice(),
            TextConformanceViolationKind::ForbiddenBom,
            Some((0, 3)),
            3,
        ),
        (
            b"a\rb".as_slice(),
            TextConformanceViolationKind::UnsupportedNewline,
            Some((1, 2)),
            2,
        ),
        (
            b"a\0b".as_slice(),
            TextConformanceViolationKind::ForbiddenControl,
            Some((1, 2)),
            2,
        ),
        (
            b"a\xffb".as_slice(),
            TextConformanceViolationKind::InvalidUtf8,
            Some((1, 2)),
            2,
        ),
    ] {
        let profile = text_profile(
            TextBomPolicy::Forbidden,
            [TextNewline::Lf],
            [TextDecodingMode::RejectInvalid],
            TextControlPolicy::RejectAll,
        );
        let (importer, parse_calls) = TextInputImporter::new(
            descriptor(profile),
            ParserBehavior::ExpectRejected {
                options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
                expected_kind,
                expected_range,
            },
        );
        let request = request_for(importer, ResourceLimits::default());

        let failure = import_bytes(&request, bytes)
            .expect_err("text profile violation must be reported by the format");

        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
        assert_format_violation(&failure);
        assert_decode_work(&failure, bytes.len() as u64, expected_consumed);
    }
}

#[test]
fn ascii_only_rejects_non_ascii_with_shifted_raw_range() {
    let profile = text_profile_with_lexical(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextLexicalMode::AsciiOnly,
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::ExpectRejected {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text")
                .with_raw_base(20),
            expected_kind: TextConformanceViolationKind::NonAscii,
            expected_range: Some((23, 24)),
        },
    );
    let request = request_for(importer, ResourceLimits::default());

    let failure = import_bytes(&request, b"caf\xc3\xa9")
        .expect_err("ASCII-only text profile must reject the first non-ASCII byte");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_format_violation(&failure);
    assert_decode_work(&failure, 5, 4);
}

#[test]
fn line_index_limit_fails_after_newline_scan_without_partial_text_storage() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::PropagateScopeFailure {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
        },
    );
    let mut limits = ResourceLimits::default();
    limits.max_line_index_bytes = 31;
    let request = request_for(importer, limits);

    let failure = import_bytes(&request, b"a\nb")
        .expect_err("line-index cap must reject the second raw line start");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_scope_resource_limit(&failure, 31);
    assert_decode_work(&failure, 3, 2);
    assert_no_text_storage(&failure);
}
