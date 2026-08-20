mod support;

use baozi_core::FormatId;
use baozi_import::{
    CapabilityStatus, CauseOwner, ConformancePolicy, ConformedTextInput, DetectionOutcome,
    DetectionResult, DetectionScope, DiagnosticByteRange, DiagnosticCode, DiagnosticSeverity,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatFailureEvidenceProducer,
    FormatImporter, ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure,
    ImportLossDisposition, ImportLossKind, ImportOperationControl, ImportReport, ImportRequest,
    ImportRequestLayer, ImportResult, LossPolicy, OptionCompileLimits, ParseResult, ParseScope,
    PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    ResourceLimits, ResourceLimitsOverlay, SceneDraft, ScopeAbortCause, TextBomPolicy,
    TextConformanceViolationKind, TextControlPolicy, TextDecodingMode, TextHardCaps,
    TextInputOptions, TextInputRecoveryTracker, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding, import_bytes as run_import_bytes,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SOURCE: &str = "scene.text";
const FORMAT_ID: &str = "dev.baozi.test.text-input-conformance";
const FORMAT_FAILURE_CODE: &str = "test.text_conformance_violation";
const INVALID_UTF8_REPLACEMENT_CODE: &str = "test.invalid_utf8_replaced";
const ACCEPTED_BOM_CODE: &str = "test.accepted_bom";

#[derive(Debug, Clone, Copy)]
struct ExpectedReplacement {
    raw_start: u64,
    raw_end: u64,
    decoded_start: usize,
    decoded_end: usize,
}

#[derive(Debug, Clone, Copy)]
struct ExpectedLine {
    text: &'static str,
    raw_start: u64,
    raw_end: u64,
    raw_record_end: u64,
    decoded_start: usize,
    decoded_end: usize,
    decoded_record_end: usize,
    newline: Option<TextNewline>,
    replacement_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct ExpectedAccepted {
    text: &'static str,
    bom: Option<(u64, u64, bool)>,
    lines: &'static [ExpectedLine],
    replacements: &'static [ExpectedReplacement],
}

#[derive(Debug, Clone, Copy)]
enum RecoveryProtocolViolation {
    ReplacementBeforeBom,
    BomTwice,
    ReplacementTwice,
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
    LeaveReplacementUnclassified {
        options: TextInputOptions,
    },
    LeaveBomDiagnosticUnclassified {
        options: TextInputOptions,
    },
    ClassifyReplacementOutOfOrder {
        options: TextInputOptions,
    },
    ExpectSourceRangeContractFailure {
        options: TextInputOptions,
    },
    CallbackFailureWithPendingObligations {
        options: TextInputOptions,
    },
    ExpectRecoveryProtocolFailure {
        options: TextInputOptions,
        violation: RecoveryProtocolViolation,
    },
    ExpectAcceptedShape {
        options: TextInputOptions,
        line_count: usize,
        replacement_count: usize,
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
                scope
                    .with_conformed_text_input(
                        scope.diagnostic_source(),
                        options,
                        |accepted, recoveries| {
                            assert_eq!(accepted.text(), expected.text);
                            assert_eq!(
                                accepted
                                    .bom()
                                    .map(|bom| (bom.raw_range(), bom.requires_diagnostic())),
                                expected.bom.map(|(start, end, diagnostic_required)| {
                                    (
                                        DiagnosticByteRange::new(start, end)
                                            .expect("expected BOM range must be valid"),
                                        diagnostic_required,
                                    )
                                })
                            );
                            let lines = accepted.lines().collect::<Vec<_>>();
                            assert_eq!(lines.len(), expected.lines.len());
                            let mut replacement_index = 0;
                            for (index, (actual, expected_line)) in
                                lines.iter().zip(expected.lines).enumerate()
                            {
                                assert_eq!(actual.number(), u64::try_from(index + 1).unwrap());
                                assert_eq!(actual.text(), expected_line.text);
                                assert_eq!(
                                    actual.raw_content_range(),
                                    DiagnosticByteRange::new(
                                        expected_line.raw_start,
                                        expected_line.raw_end,
                                    )
                                    .expect("expected line raw range must be valid")
                                );
                                assert_eq!(
                                    actual.raw_record_range(),
                                    DiagnosticByteRange::new(
                                        expected_line.raw_start,
                                        expected_line.raw_record_end,
                                    )
                                    .expect("expected line record range must be valid")
                                );
                                assert_eq!(
                                    actual.decoded_content_range(),
                                    expected_line.decoded_start..expected_line.decoded_end
                                );
                                assert_eq!(
                                    actual.decoded_record_range(),
                                    expected_line.decoded_start
                                        ..expected_line.decoded_record_end
                                );
                                assert_eq!(actual.newline(), expected_line.newline);
                                assert_eq!(
                                    actual.replacements().len(),
                                    expected_line.replacement_count
                                );
                                for actual in actual.replacements() {
                                    let expected = expected.replacements.get(replacement_index).expect(
                                        "actual replacement must have an expected contract entry",
                                    );
                                    assert_eq!(
                                        actual.raw_range(),
                                        DiagnosticByteRange::new(
                                            expected.raw_start,
                                            expected.raw_end,
                                        )
                                        .expect("expected replacement raw range must be valid")
                                    );
                                    assert_eq!(actual.decoded_start(), expected.decoded_start);
                                    assert_eq!(actual.decoded_end(), expected.decoded_end);
                                    replacement_index += 1;
                                }
                            }
                            assert_eq!(replacement_index, expected.replacements.len());
                            authorize_text_obligations(recoveries, accepted)
                        },
                    )?
                    .expect("test text input must conform");
                support::submit_minimal_scene(scope)
            }
            ParserBehavior::ExpectRejected {
                options,
                expected_kind,
                expected_range,
            } => {
                let rejected = scope
                    .with_conformed_text_input(
                        scope.diagnostic_source(),
                        options,
                        |_accepted, _recoveries| -> ParseResult<()> {
                            panic!("rejected text input must not invoke the callback")
                        },
                    )?
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
                scope
                    .with_conformed_text_input(
                        scope.diagnostic_source(),
                        options,
                        |accepted, recoveries| authorize_text_obligations(recoveries, accepted),
                    )?
                    .expect("test text input should only fail through the scope");
                support::submit_minimal_scene(scope)
            }
            ParserBehavior::LeaveReplacementUnclassified { options } => {
                scope
                    .with_conformed_text_input(
                        scope.diagnostic_source(),
                        options,
                        |accepted, _recoveries| {
                            assert!(
                                accepted.lines().any(|line| !line.replacements().is_empty()),
                                "the fixture must expose at least one replacement"
                            );
                            Ok(())
                        },
                    )?
                    .expect("test text input must conform before recovery classification");
                support::submit_minimal_scene(scope)
            }
            ParserBehavior::LeaveBomDiagnosticUnclassified { options } => {
                scope
                    .with_conformed_text_input(
                        scope.diagnostic_source(),
                        options,
                        |accepted, _recoveries| {
                            assert!(
                                accepted.bom().is_some_and(|bom| bom.requires_diagnostic()),
                                "the fixture must expose a required BOM diagnostic"
                            );
                            Ok(())
                        },
                    )?
                    .expect("test text input must conform before BOM classification");
                support::submit_minimal_scene(scope)
            }
            ParserBehavior::ClassifyReplacementOutOfOrder { options } => {
                scope
                    .with_conformed_text_input(
                        scope.diagnostic_source(),
                        options,
                        |accepted, recoveries| {
                            let line = accepted
                                .lines()
                                .next()
                                .expect("replacement fixture must have one line");
                            let replacement = line
                                .replacements()
                                .get(1)
                                .expect("replacement fixture must have a second replacement");
                            recoveries.discard_replacement(replacement)
                        },
                    )?
                    .expect("test text input must conform before replacement classification");
                support::submit_minimal_scene(scope)
            }
            ParserBehavior::ExpectSourceRangeContractFailure { options } => {
                let _ = scope.with_conformed_text_input(
                    scope.diagnostic_source(),
                    options,
                    |_accepted, _recoveries| -> ParseResult<()> {
                        panic!("an invalid source range must not invoke the callback")
                    },
                )?;
                panic!("the source range fixture must fail before conformance")
            }
            ParserBehavior::CallbackFailureWithPendingObligations { options } => {
                let _ = scope.with_conformed_text_input(
                    scope.diagnostic_source(),
                    options,
                    |_accepted, _recoveries| -> ParseResult<()> {
                        Err(scope.format_failure(TestTextFailure))
                    },
                )?;
                panic!("the callback failure must remain terminal")
            }
            ParserBehavior::ExpectRecoveryProtocolFailure { options, violation } => {
                scope
                    .with_conformed_text_input(
                        scope.diagnostic_source(),
                        options,
                        |accepted, recoveries| {
                            let bom = accepted
                                .bom()
                                .filter(|bom| bom.requires_diagnostic())
                                .expect("the protocol fixture must require a BOM diagnostic");
                            let line = accepted
                                .lines()
                                .find(|line| !line.replacements().is_empty())
                                .expect("the protocol fixture must expose a replacement");
                            let replacement = line
                                .replacements()
                                .first()
                                .expect("the protocol fixture must expose a replacement");
                            let bom_code = DiagnosticCode::new(ACCEPTED_BOM_CODE)
                                .expect("static BOM diagnostic code must be valid");
                            match violation {
                                RecoveryProtocolViolation::ReplacementBeforeBom => {
                                    recoveries.discard_replacement(replacement)
                                }
                                RecoveryProtocolViolation::BomTwice => {
                                    recoveries.record_bom_diagnostic(
                                        bom,
                                        bom_code,
                                        format_args!("accepted UTF-8 BOM in test text"),
                                    )?;
                                    recoveries.record_bom_diagnostic(
                                        bom,
                                        bom_code,
                                        format_args!("accepted UTF-8 BOM in test text"),
                                    )
                                }
                                RecoveryProtocolViolation::ReplacementTwice => {
                                    recoveries.record_bom_diagnostic(
                                        bom,
                                        bom_code,
                                        format_args!("accepted UTF-8 BOM in test text"),
                                    )?;
                                    recoveries.discard_replacement(replacement)?;
                                    recoveries.discard_replacement(replacement)
                                }
                            }
                        },
                    )?
                    .expect("test text input must conform before recovery classification");
                support::submit_minimal_scene(scope)
            }
            ParserBehavior::ExpectAcceptedShape {
                options,
                line_count,
                replacement_count,
            } => {
                scope
                    .with_conformed_text_input(
                        scope.diagnostic_source(),
                        options,
                        |accepted, recoveries| {
                            let mut actual_lines = 0usize;
                            let mut actual_replacements = 0usize;
                            for line in accepted.lines() {
                                actual_lines += 1;
                                actual_replacements += line.replacements().len();
                                recoveries.discard_replacements(line.replacements())?;
                            }
                            assert_eq!(actual_lines, line_count);
                            assert_eq!(actual_replacements, replacement_count);
                            Ok(())
                        },
                    )?
                    .expect("test text input shape must conform");
                support::submit_minimal_scene(scope)
            }
        }
    }
}

fn authorize_text_obligations(
    recoveries: &mut TextInputRecoveryTracker<'_, '_>,
    accepted: &ConformedTextInput<'_>,
) -> ParseResult<()> {
    if let Some(bom) = accepted.bom().filter(|bom| bom.requires_diagnostic()) {
        let code = DiagnosticCode::new(ACCEPTED_BOM_CODE)
            .expect("static BOM diagnostic code must be valid");
        recoveries.record_bom_diagnostic(
            bom,
            code,
            format_args!("accepted UTF-8 BOM in test text"),
        )?;
    }
    let code = DiagnosticCode::new(INVALID_UTF8_REPLACEMENT_CODE)
        .expect("static replacement diagnostic code must be valid");
    for line in accepted.lines() {
        for replacement in line.replacements() {
            recoveries.record_format_recovery(
                replacement,
                code,
                format_args!("replaced invalid UTF-8 in test text"),
            )?;
        }
    }
    Ok(())
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

#[allow(
    clippy::result_large_err,
    reason = "contract assertions inspect complete bounded import failure evidence"
)]
fn import_text_shape(
    bytes: &[u8],
    decoding_mode: TextDecodingMode,
    line_count: usize,
    replacement_count: usize,
    limits: ResourceLimits,
) -> Result<ImportReport, ImportFailure> {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [
            TextDecodingMode::RejectInvalid,
            TextDecodingMode::ReplaceInvalidUtf8,
        ],
        TextControlPolicy::RejectAll,
    );
    let (importer, _) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::ExpectAcceptedShape {
            options: TextInputOptions::new(decoding_mode, "test-text"),
            line_count,
            replacement_count,
        },
    );
    let request = request_for(importer, limits);
    import_bytes(&request, bytes)
}

fn line_index_peak(report: &ImportReport) -> u64 {
    report
        .ledger()
        .dimension(ImportDimension::LineIndexBytes)
        .expect("text input must publish line-index evidence")
        .peak_live_bytes()
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
    assert_eq!(failure.ledger().current_live_bytes(), 0);
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

fn assert_scope_descriptor_contract(failure: &ImportFailure, expected_code: &'static str) {
    assert_eq!(failure.cause().owner(), CauseOwner::Import);
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::DescriptorContract { code } if *code == expected_code
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
                lines: &[
                    ExpectedLine {
                        text: "v 1",
                        raw_start: 0,
                        raw_end: 3,
                        raw_record_end: 5,
                        decoded_start: 0,
                        decoded_end: 3,
                        decoded_record_end: 5,
                        newline: Some(TextNewline::CrLf),
                        replacement_count: 0,
                    },
                    ExpectedLine {
                        text: "\tcaf\u{e9}",
                        raw_start: 5,
                        raw_end: 11,
                        raw_record_end: 12,
                        decoded_start: 5,
                        decoded_end: 11,
                        decoded_record_end: 12,
                        newline: Some(TextNewline::Lf),
                        replacement_count: 0,
                    },
                ],
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
                bom: Some((0, 3, false)),
                lines: &[ExpectedLine {
                    text: "o",
                    raw_start: 3,
                    raw_end: 4,
                    raw_record_end: 5,
                    decoded_start: 0,
                    decoded_end: 1,
                    decoded_record_end: 2,
                    newline: Some(TextNewline::Lf),
                    replacement_count: 0,
                }],
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
fn empty_and_bom_only_inputs_have_no_grammar_lines() {
    for (bytes, expected_bom) in [
        (&b""[..], None),
        (&b"\xef\xbb\xbf"[..], Some((0, 3, false))),
    ] {
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
                    text: "",
                    bom: expected_bom,
                    lines: &[],
                    replacements: &[],
                },
            },
        );
        let request = request_for(importer, ResourceLimits::default());

        let report = import_bytes(&request, bytes)
            .expect("empty decoded input must not manufacture a grammar record");

        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
        assert_eq!(report.ledger().current_live_bytes(), 0);
    }
}

#[test]
fn raw_base_offsets_apply_to_cr_lines_without_reinterpreting_mid_source_bom() {
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
                text: "\u{feff}a\rz",
                bom: None,
                lines: &[
                    ExpectedLine {
                        text: "\u{feff}a",
                        raw_start: 100,
                        raw_end: 104,
                        raw_record_end: 105,
                        decoded_start: 0,
                        decoded_end: 4,
                        decoded_record_end: 5,
                        newline: Some(TextNewline::Cr),
                        replacement_count: 0,
                    },
                    ExpectedLine {
                        text: "z",
                        raw_start: 105,
                        raw_end: 106,
                        raw_record_end: 106,
                        decoded_start: 5,
                        decoded_end: 6,
                        decoded_record_end: 6,
                        newline: None,
                        replacement_count: 0,
                    },
                ],
                replacements: &[],
            },
        },
    );
    let request = request_for(importer, ResourceLimits::default());
    let mut bytes = vec![b'x'; 100];
    bytes.extend_from_slice(b"\xef\xbb\xbfa\rz");

    let report = import_bytes(&request, &bytes)
        .expect("raw-base text conformance must preserve shifted offsets");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn line_view_preserves_empty_records_and_cross_line_replacement_mapping() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf, TextNewline::CrLf],
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
                text: "\n\r\na\u{fffd}\r\nz\u{fffd}",
                bom: None,
                lines: &[
                    ExpectedLine {
                        text: "",
                        raw_start: 0,
                        raw_end: 0,
                        raw_record_end: 1,
                        decoded_start: 0,
                        decoded_end: 0,
                        decoded_record_end: 1,
                        newline: Some(TextNewline::Lf),
                        replacement_count: 0,
                    },
                    ExpectedLine {
                        text: "",
                        raw_start: 1,
                        raw_end: 1,
                        raw_record_end: 3,
                        decoded_start: 1,
                        decoded_end: 1,
                        decoded_record_end: 3,
                        newline: Some(TextNewline::CrLf),
                        replacement_count: 0,
                    },
                    ExpectedLine {
                        text: "a\u{fffd}",
                        raw_start: 3,
                        raw_end: 5,
                        raw_record_end: 7,
                        decoded_start: 3,
                        decoded_end: 7,
                        decoded_record_end: 9,
                        newline: Some(TextNewline::CrLf),
                        replacement_count: 1,
                    },
                    ExpectedLine {
                        text: "z\u{fffd}",
                        raw_start: 7,
                        raw_end: 10,
                        raw_record_end: 10,
                        decoded_start: 9,
                        decoded_end: 13,
                        decoded_record_end: 13,
                        newline: None,
                        replacement_count: 1,
                    },
                ],
                replacements: &[
                    ExpectedReplacement {
                        raw_start: 4,
                        raw_end: 5,
                        decoded_start: 4,
                        decoded_end: 7,
                    },
                    ExpectedReplacement {
                        raw_start: 8,
                        raw_end: 10,
                        decoded_start: 10,
                        decoded_end: 13,
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

    let report = import_bytes(&request, b"\n\r\na\xff\r\nz\xe2\x82")
        .expect("line mapping must remain stable after UTF-8 replacement expansion");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_replacement_recovery(&report, [(4, 5), (8, 10)]);
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
                lines: &[ExpectedLine {
                    text: "a\u{fffd}b\u{fffd}",
                    raw_start: 0,
                    raw_end: 5,
                    raw_record_end: 5,
                    decoded_start: 0,
                    decoded_end: 8,
                    decoded_record_end: 8,
                    newline: None,
                    replacement_count: 2,
                }],
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
fn replacement_ranges_remain_source_relative_after_a_nonzero_raw_base() {
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
            options: TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "test-text")
                .with_raw_base(2),
            expected: ExpectedAccepted {
                text: "a\u{fffd}b",
                bom: None,
                lines: &[ExpectedLine {
                    text: "a\u{fffd}b",
                    raw_start: 2,
                    raw_end: 5,
                    raw_record_end: 5,
                    decoded_start: 0,
                    decoded_end: 5,
                    decoded_record_end: 5,
                    newline: None,
                    replacement_count: 1,
                }],
                replacements: &[ExpectedReplacement {
                    raw_start: 3,
                    raw_end: 4,
                    decoded_start: 1,
                    decoded_end: 4,
                }],
            },
        },
    );
    let request = request_for_with_policies(
        importer,
        ResourceLimits::default(),
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
    );

    let report = import_bytes(&request, b"xxa\xffb")
        .expect("replacement evidence must retain absolute source offsets");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_replacement_recovery(&report, [(3, 4)]);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn replacement_recovery_requires_common_policies_before_parse_success() {
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
fn unclassified_replacement_cannot_publish_parse_success() {
    for (conformance_policy, loss_policy) in [
        (ConformancePolicy::Strict, recovery_loss_policy()),
        (
            ConformancePolicy::AllowDeclaredRecovery,
            LossPolicy::preserve_or_fail(),
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
            ParserBehavior::LeaveReplacementUnclassified {
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
            .expect_err("unclassified replacement must prevent parse success");

        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
        assert_scope_descriptor_contract(&failure, "text-input-replacement-unclassified");
    }
}

#[test]
fn completion_protocol_requires_bom_diagnostics_and_ordered_replacements() {
    let bom_profile = text_profile(
        TextBomPolicy::AcceptedWithDiagnostic,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let (bom_importer, bom_parse_calls) = TextInputImporter::new(
        descriptor(bom_profile),
        ParserBehavior::LeaveBomDiagnosticUnclassified {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
        },
    );
    let bom_request = request_for(bom_importer, ResourceLimits::default());
    let bom_failure = import_bytes(&bom_request, b"\xef\xbb\xbfa")
        .expect_err("required BOM diagnostic must prevent parse success when omitted");
    assert_eq!(bom_parse_calls.load(Ordering::Acquire), 1);
    assert_scope_descriptor_contract(&bom_failure, "text-input-bom-diagnostic-unclassified");

    let replacement_profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [
            TextDecodingMode::RejectInvalid,
            TextDecodingMode::ReplaceInvalidUtf8,
        ],
        TextControlPolicy::RejectAll,
    );
    let (replacement_importer, replacement_parse_calls) = TextInputImporter::new(
        descriptor(replacement_profile),
        ParserBehavior::ClassifyReplacementOutOfOrder {
            options: TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "test-text"),
        },
    );
    let replacement_request = request_for(replacement_importer, ResourceLimits::default());
    let replacement_failure = import_bytes(&replacement_request, b"\xff\xfe")
        .expect_err("replacement classification must follow source order");
    assert_eq!(replacement_parse_calls.load(Ordering::Acquire), 1);
    assert_scope_descriptor_contract(&replacement_failure, "text-input-replacement-out-of-order");
}

#[test]
fn text_input_source_range_rejects_invalid_offsets_before_conformance() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    for (options, expected_code) in [
        (
            TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text").with_raw_base(2),
            "text-input-source-offset-out-of-bounds",
        ),
        (
            TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text")
                .with_raw_base(u64::MAX),
            "text-input-source-offset-out-of-bounds",
        ),
    ] {
        let (importer, parse_calls) = TextInputImporter::new(
            descriptor(profile.clone()),
            ParserBehavior::ExpectSourceRangeContractFailure { options },
        );
        let request = request_for(importer, ResourceLimits::default());
        let failure = import_bytes(&request, b"\xff")
            .expect_err("an invalid source range must fail before conformance");

        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
        assert_scope_descriptor_contract(&failure, expected_code);
        assert_no_decode_work(&failure);
        assert_no_text_storage(&failure);
    }
}

#[test]
fn raw_base_at_source_end_selects_an_empty_suffix() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::ExpectAccepted {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text")
                .with_raw_base(4),
            expected: ExpectedAccepted {
                text: "",
                bom: None,
                lines: &[],
                replacements: &[],
            },
        },
    );
    let request = request_for(importer, ResourceLimits::default());

    let report = import_bytes(&request, b"abcd").expect("the exact source end is a valid suffix");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn combined_bom_and_replacement_protocol_is_ordered_and_exactly_once() {
    let profile = text_profile(
        TextBomPolicy::AcceptedWithDiagnostic,
        [TextNewline::Lf],
        [
            TextDecodingMode::RejectInvalid,
            TextDecodingMode::ReplaceInvalidUtf8,
        ],
        TextControlPolicy::RejectAll,
    );
    let options = TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "test-text");
    let bytes = b"\xef\xbb\xbf\xff";

    for (violation, expected_code) in [
        (
            RecoveryProtocolViolation::ReplacementBeforeBom,
            "text-input-bom-diagnostic-unclassified",
        ),
        (
            RecoveryProtocolViolation::BomTwice,
            "text-input-bom-diagnostic-overclassified",
        ),
        (
            RecoveryProtocolViolation::ReplacementTwice,
            "text-input-replacement-overclassified",
        ),
    ] {
        let (importer, parse_calls) = TextInputImporter::new(
            descriptor(profile.clone()),
            ParserBehavior::ExpectRecoveryProtocolFailure { options, violation },
        );
        let request = request_for_with_policies(
            importer,
            ResourceLimits::default(),
            ConformancePolicy::AllowDeclaredRecovery,
            recovery_loss_policy(),
        );
        let failure = import_bytes(&request, bytes)
            .expect_err("invalid recovery protocol ordering must prevent parse success");

        assert_eq!(parse_calls.load(Ordering::Acquire), 1);
        assert_scope_descriptor_contract(&failure, expected_code);
    }

    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::PropagateScopeFailure { options },
    );
    let request = request_for_with_policies(
        importer,
        ResourceLimits::default(),
        ConformancePolicy::AllowDeclaredRecovery,
        recovery_loss_policy(),
    );
    let report = import_bytes(&request, bytes)
        .expect("the ordered recovery protocol must preserve parse success");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_eq!(report.diagnostics().len(), 2);
    assert_eq!(report.diagnostics()[0].code().as_str(), ACCEPTED_BOM_CODE);
    assert_eq!(
        report.diagnostics()[0].byte_range(),
        Some(DiagnosticByteRange::new(0, 3).expect("BOM range must be valid"))
    );
    assert_eq!(
        report.diagnostics()[1].code().as_str(),
        INVALID_UTF8_REPLACEMENT_CODE
    );
    assert_eq!(
        report.diagnostics()[1].byte_range(),
        Some(DiagnosticByteRange::new(3, 4).expect("replacement range must be valid"))
    );
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(
        report.loss_evidence()[0].raw_range(),
        Some(DiagnosticByteRange::new(3, 4).expect("replacement range must be valid"))
    );
}

#[test]
fn callback_failure_precedes_pending_bom_and_replacement_obligations() {
    let profile = text_profile(
        TextBomPolicy::AcceptedWithDiagnostic,
        [TextNewline::Lf],
        [
            TextDecodingMode::RejectInvalid,
            TextDecodingMode::ReplaceInvalidUtf8,
        ],
        TextControlPolicy::RejectAll,
    );
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::CallbackFailureWithPendingObligations {
            options: TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "test-text"),
        },
    );
    let request = request_for(importer, ResourceLimits::default());

    let failure = import_bytes(&request, b"\xef\xbb\xbf\xff")
        .expect_err("the callback failure must outrank unfinished recovery obligations");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    let ImportCause::Format(format) = failure.cause() else {
        panic!("the callback failure must remain format-owned")
    };
    assert_eq!(format.code(), FORMAT_FAILURE_CODE);
    assert!(failure.diagnostics().is_empty());
    assert!(failure.loss_evidence().is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
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
    let mut bytes = vec![b'x'; 20];
    bytes.extend_from_slice(b"caf\xc3\xa9");

    let failure = import_bytes(&request, &bytes)
        .expect_err("ASCII-only text profile must reject the first non-ASCII byte");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_format_violation(&failure);
    assert_decode_work(&failure, 5, 4);
}

#[test]
fn ascii_only_reports_invalid_utf8_before_non_ascii_classification() {
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
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
            expected_kind: TextConformanceViolationKind::InvalidUtf8,
            expected_range: Some((3, 4)),
        },
    );
    let request = request_for(importer, ResourceLimits::default());

    let failure = import_bytes(&request, b"caf\xff")
        .expect_err("ASCII-only text must distinguish invalid UTF-8 from valid non-ASCII text");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_format_violation(&failure);
    assert_decode_work(&failure, 4, 4);
}

#[test]
fn line_index_limit_fails_after_newline_scan_without_partial_text_storage() {
    let profile = text_profile(
        TextBomPolicy::Forbidden,
        [TextNewline::Lf],
        [TextDecodingMode::RejectInvalid],
        TextControlPolicy::RejectAll,
    );
    let one_line = import_text_shape(
        b"a",
        TextDecodingMode::RejectInvalid,
        1,
        0,
        ResourceLimits::default(),
    )
    .expect("one line must establish the first line-index capacity");
    let first_line_index_bytes = line_index_peak(&one_line);
    let (importer, parse_calls) = TextInputImporter::new(
        descriptor(profile),
        ParserBehavior::PropagateScopeFailure {
            options: TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text"),
        },
    );
    let mut limits = ResourceLimits::default();
    limits.max_line_index_bytes = first_line_index_bytes;
    let request = request_for(importer, limits);

    let failure = import_bytes(&request, b"a\nb\nc\nd\ne")
        .expect_err("line-index cap must reject the first capacity growth");

    assert_eq!(parse_calls.load(Ordering::Acquire), 1);
    assert_scope_resource_limit(&failure, first_line_index_bytes);
    assert_decode_work(&failure, 9, 8);
    assert_no_text_storage(&failure);
}

#[test]
fn line_index_growth_boundary_is_exact_for_lines_and_replacements() {
    for (label, bytes, decoding_mode, line_count, replacement_count) in [
        (
            "five-lines",
            b"a\nb\nc\nd\ne".as_slice(),
            TextDecodingMode::RejectInvalid,
            5,
            0,
        ),
        (
            "five-replacements",
            b"\xff\xff\xff\xff\xff".as_slice(),
            TextDecodingMode::ReplaceInvalidUtf8,
            1,
            5,
        ),
        (
            "five-lines-and-replacements",
            b"\xff\n\xff\n\xff\n\xff\n\xff".as_slice(),
            TextDecodingMode::ReplaceInvalidUtf8,
            5,
            5,
        ),
    ] {
        let baseline = import_text_shape(
            bytes,
            decoding_mode,
            line_count,
            replacement_count,
            ResourceLimits::default(),
        )
        .unwrap_or_else(|failure| panic!("{label} baseline failed: {failure:?}"));
        let required = line_index_peak(&baseline);
        assert!(required > 0, "{label} must allocate a line index");

        let mut exact_limits = ResourceLimits::default();
        exact_limits.max_line_index_bytes = required;
        let exact = import_text_shape(
            bytes,
            decoding_mode,
            line_count,
            replacement_count,
            exact_limits,
        )
        .unwrap_or_else(|failure| panic!("{label} exact budget failed: {failure:?}"));
        assert_eq!(line_index_peak(&exact), required);

        let mut insufficient_limits = ResourceLimits::default();
        insufficient_limits.max_line_index_bytes = required - 1;
        let failure = import_text_shape(
            bytes,
            decoding_mode,
            line_count,
            replacement_count,
            insufficient_limits,
        )
        .expect_err("one byte below the exact line-index budget must fail");
        assert_scope_resource_limit(&failure, required - 1);
        assert_no_text_storage(&failure);
    }
}
