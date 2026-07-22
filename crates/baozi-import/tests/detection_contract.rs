#![allow(
    clippy::result_large_err,
    reason = "detection failures retain complete bounded terminal evidence by value"
)]

use baozi_core::{FormatId, SceneDraft, SceneSpace};
use baozi_import::{
    CapabilityStatus, DetectionEvidence, DetectionFailure, DetectionInconclusive, DetectionOptions,
    DetectionOutcome, DetectionResult, DetectionScope, FormatCapability, FormatDescriptor,
    FormatEncoding, FormatFailureEvidenceProducer, FormatImporter, FormatSelection, ImportCause,
    ImportConfigOverlay, ImportFailure, ImportOperationControl, ImportPhase, ImportProgressPhase,
    ImportProgressUnit, ImportRequest, ImportRequestLayer, ImportRunError, OptionCompileLimits,
    ParseResult, ParseScope, PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuilder, import_bytes,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationToken, ProgressDecision, ProgressEvent};
use std::error::Error;
use std::fmt;

const PREFERRED_ID: &str = "dev.baozi.detect.preferred";
const EXTENSION_ID: &str = "dev.baozi.detect.extension";
const FAULT_ID: &str = "dev.baozi.detect.fault";
const INCONCLUSIVE_ID: &str = "dev.baozi.detect.inconclusive";
const NO_MATCH_ID: &str = "dev.baozi.detect.nomatch";

#[derive(Debug, Clone, Copy)]
enum DetectorBehavior {
    Match(ReadConfidence),
    Prefix(&'static [u8]),
    Fault(&'static str),
    Inconclusive,
    NoMatch,
}

impl DetectorBehavior {
    fn outcome(self, probe: &baozi_import::Probe<'_>) -> DetectionOutcome {
        match self {
            Self::Match(confidence) => DetectionOutcome::Match(confidence),
            Self::Prefix(prefix) if probe.bytes().starts_with(prefix) => {
                DetectionOutcome::Match(ReadConfidence::Certain)
            }
            Self::Prefix(prefix) if !probe.is_complete() && prefix.starts_with(probe.bytes()) => {
                DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
            }
            Self::Prefix(_) | Self::NoMatch => DetectionOutcome::NoMatch,
            Self::Fault(detail) => DetectionOutcome::fault(detail),
            Self::Inconclusive => {
                DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
            }
        }
    }
}

#[derive(Debug, Clone)]
enum ParseBehavior {
    Success,
    FormatFailure,
    InvalidScene,
    Cancel(CancellationToken),
}

#[derive(Debug, Clone)]
struct TestImporter {
    id: &'static str,
    extension: &'static str,
    detector: DetectorBehavior,
    parser: ParseBehavior,
}

impl TestImporter {
    fn successful(id: &'static str, extension: &'static str, detector: DetectorBehavior) -> Self {
        Self {
            id,
            extension,
            detector,
            parser: ParseBehavior::Success,
        }
    }

    fn with_parser(mut self, parser: ParseBehavior) -> Self {
        self.parser = parser;
        self
    }
}

impl FormatImporter for TestImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new(self.id).expect("qualified test format id"),
            "Detection Contract Test",
        )
        .extensions([self.extension])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .expect("valid test descriptor")
    }

    fn detect(
        &self,
        probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        let work = if probe.bytes().is_empty() {
            None
        } else {
            Some(scope.begin_work(probe.bytes().len() as u64)?)
        };
        let outcome = self.detector.outcome(probe);
        if let Some(work) = work {
            work.complete()?;
        }
        Ok(outcome)
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        match &self.parser {
            ParseBehavior::Success => scope.scene_builder()?.finish_draft(),
            ParseBehavior::FormatFailure => Err(scope.format_failure(TestFormatCause)),
            ParseBehavior::InvalidScene => {
                let mut builder = scope.scene_builder()?;
                builder.set_space(SceneSpace {
                    unit_scale_to_meters: Some(f32::NAN),
                    ..SceneSpace::default()
                })?;
                builder.finish_draft()
            }
            ParseBehavior::Cancel(token) => {
                token.cancel();
                scope.poll()?;
                scope.scene_builder()?.finish_draft()
            }
        }
    }
}

#[derive(Debug)]
struct TestFormatCause;

impl fmt::Display for TestFormatCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("injected parser failure")
    }
}

impl Error for TestFormatCause {}

impl FormatFailureEvidenceProducer for TestFormatCause {
    fn code(&self) -> &'static str {
        "test.detection.parser_failure"
    }
}

fn format_id(value: &str) -> FormatId {
    FormatId::new(value).expect("qualified test format id")
}

fn compile_request(
    importers: impl IntoIterator<Item = TestImporter>,
    config: ImportConfigOverlay,
) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    for importer in importers {
        registry
            .register(importer)
            .expect("unique valid test importer");
    }
    let snapshot = registry
        .freeze(RegistryBuildLimits::default())
        .expect("valid frozen test registry");
    snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(config),
            ),
            OptionCompileLimits::default(),
        )
        .expect("valid test request")
}

fn operation_failure(error: ImportRunError) -> ImportFailure {
    error
        .into_operation_failure()
        .expect("detection contract import must initialize")
}

fn assert_evidence(actual: &[DetectionEvidence], expected: &[(&str, DetectionOutcome)]) {
    assert_eq!(
        actual
            .iter()
            .map(|evidence| (evidence.format().as_str(), evidence.outcome().clone()))
            .collect::<Vec<_>>(),
        expected
    );
}

fn for_each_permutation<T>(values: &mut [T], fixed: usize, visitor: &mut impl FnMut(&[T])) {
    if fixed == values.len() {
        visitor(values);
        return;
    }

    for swap_index in fixed..values.len() {
        values.swap(fixed, swap_index);
        for_each_permutation(values, fixed + 1, visitor);
        values.swap(fixed, swap_index);
    }
}

#[test]
fn preferred_and_extension_confidence_order_is_exhaustive_and_registration_independent() {
    let cases = [
        (ReadConfidence::Maybe, ReadConfidence::Maybe, PREFERRED_ID),
        (ReadConfidence::Maybe, ReadConfidence::Likely, EXTENSION_ID),
        (ReadConfidence::Maybe, ReadConfidence::Certain, EXTENSION_ID),
        (ReadConfidence::Likely, ReadConfidence::Maybe, PREFERRED_ID),
        (ReadConfidence::Likely, ReadConfidence::Likely, PREFERRED_ID),
        (
            ReadConfidence::Likely,
            ReadConfidence::Certain,
            EXTENSION_ID,
        ),
        (ReadConfidence::Certain, ReadConfidence::Maybe, PREFERRED_ID),
        (
            ReadConfidence::Certain,
            ReadConfidence::Likely,
            PREFERRED_ID,
        ),
        (
            ReadConfidence::Certain,
            ReadConfidence::Certain,
            PREFERRED_ID,
        ),
    ];

    for (preferred_confidence, extension_confidence, expected_format) in cases {
        let mut importers = [
            TestImporter::successful(
                PREFERRED_ID,
                "preferred",
                DetectorBehavior::Match(preferred_confidence),
            ),
            TestImporter::successful(
                EXTENSION_ID,
                "mesh",
                DetectorBehavior::Match(extension_confidence),
            ),
            TestImporter::successful(FAULT_ID, "fault", DetectorBehavior::Fault("detector fault")),
            TestImporter::successful(
                INCONCLUSIVE_ID,
                "inconclusive",
                DetectorBehavior::Inconclusive,
            ),
            TestImporter::successful(NO_MATCH_ID, "nomatch", DetectorBehavior::NoMatch),
        ];
        let mut permutations = 0_usize;
        for_each_permutation(&mut importers, 0, &mut |registration_order| {
            permutations += 1;
            let request = compile_request(
                registration_order.iter().cloned(),
                ImportConfigOverlay::default()
                    .with_format_selection(FormatSelection::Preferred(format_id(PREFERRED_ID))),
            );
            let report = import_bytes(
                &request,
                AssetPath::new("asset.mesh").expect("valid source"),
                b"primary",
                ImportOperationControl::default(),
            )
            .expect("the fully ordered policy must select one candidate");

            assert_eq!(report.format().id().as_str(), expected_format);
            assert_evidence(
                report.detection_evidence(),
                &[
                    (EXTENSION_ID, DetectionOutcome::Match(extension_confidence)),
                    (FAULT_ID, DetectionOutcome::fault("detector fault")),
                    (
                        INCONCLUSIVE_ID,
                        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted),
                    ),
                    (NO_MATCH_ID, DetectionOutcome::NoMatch),
                    (PREFERRED_ID, DetectionOutcome::Match(preferred_confidence)),
                ],
            );
        });
        assert_eq!(permutations, 120, "every 5! registration order must run");
    }
}

#[test]
fn forced_selection_only_consults_the_forced_detector_and_classifies_every_outcome() {
    let forced_id = "dev.baozi.detect.forced";
    let cases = [
        DetectorBehavior::Match(ReadConfidence::Maybe),
        DetectorBehavior::Inconclusive,
        DetectorBehavior::NoMatch,
        DetectorBehavior::Fault("forced detector fault"),
    ];

    for behavior in cases {
        let request = compile_request(
            [
                TestImporter::successful(
                    "dev.baozi.detect.distractor",
                    "forced",
                    DetectorBehavior::Match(ReadConfidence::Certain),
                ),
                TestImporter::successful(forced_id, "forced", behavior),
                TestImporter::successful(FAULT_ID, "fault", DetectorBehavior::Fault("fault")),
                TestImporter::successful(
                    INCONCLUSIVE_ID,
                    "inconclusive",
                    DetectorBehavior::Inconclusive,
                ),
                TestImporter::successful(NO_MATCH_ID, "nomatch", DetectorBehavior::NoMatch),
            ],
            ImportConfigOverlay::default()
                .with_format_selection(FormatSelection::Forced(format_id(forced_id))),
        );
        let result = import_bytes(
            &request,
            AssetPath::new("asset.forced").expect("valid source"),
            b"primary",
            ImportOperationControl::default(),
        );

        match behavior {
            DetectorBehavior::Match(confidence) => {
                let report = result.expect("a forced match must parse");
                assert_eq!(report.format().id().as_str(), forced_id);
                assert_evidence(
                    report.detection_evidence(),
                    &[(forced_id, DetectionOutcome::Match(confidence))],
                );
            }
            DetectorBehavior::Inconclusive => {
                let report = result.expect("forced inconclusive evidence defers to the parser");
                assert_eq!(report.format().id().as_str(), forced_id);
                assert_evidence(
                    report.detection_evidence(),
                    &[(
                        forced_id,
                        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted),
                    )],
                );
            }
            DetectorBehavior::NoMatch => {
                let failure = operation_failure(result.expect_err("forced contradiction"));
                assert!(matches!(
                    failure.cause(),
                    ImportCause::Detection(DetectionFailure::ForcedFormatContradiction {
                        format
                    }) if format.as_str() == forced_id
                ));
                assert_evidence(
                    failure.detection_evidence(),
                    &[(forced_id, DetectionOutcome::NoMatch)],
                );
            }
            DetectorBehavior::Fault(detail) => {
                let failure = operation_failure(result.expect_err("forced detector fault"));
                assert!(matches!(
                    failure.cause(),
                    ImportCause::Detection(DetectionFailure::ForcedDetectorFault { format })
                        if format.as_str() == forced_id
                ));
                assert_evidence(
                    failure.detection_evidence(),
                    &[(forced_id, DetectionOutcome::fault(detail))],
                );
            }
            DetectorBehavior::Prefix(_) => unreachable!("prefix behavior is not in this table"),
        }
    }
}

#[test]
fn automatic_selection_keeps_complete_sorted_evidence_while_choosing_the_strongest_match() {
    let selected_id = "dev.baozi.detect.automatic";
    let request = compile_request(
        [
            TestImporter::successful(NO_MATCH_ID, "nomatch", DetectorBehavior::NoMatch),
            TestImporter::successful(
                INCONCLUSIVE_ID,
                "inconclusive",
                DetectorBehavior::Inconclusive,
            ),
            TestImporter::successful(FAULT_ID, "fault", DetectorBehavior::Fault("detector fault")),
            TestImporter::successful(
                "dev.baozi.detect.weaker",
                "weaker",
                DetectorBehavior::Match(ReadConfidence::Likely),
            ),
            TestImporter::successful(
                selected_id,
                "automatic",
                DetectorBehavior::Match(ReadConfidence::Certain),
            ),
        ],
        ImportConfigOverlay::default(),
    );
    let report = import_bytes(
        &request,
        AssetPath::new("asset.unhinted").expect("valid source"),
        b"primary",
        ImportOperationControl::default(),
    )
    .expect("the unique strongest automatic match must win");

    assert_eq!(report.format().id().as_str(), selected_id);
    assert_evidence(
        report.detection_evidence(),
        &[
            (
                selected_id,
                DetectionOutcome::Match(ReadConfidence::Certain),
            ),
            (FAULT_ID, DetectionOutcome::fault("detector fault")),
            (
                INCONCLUSIVE_ID,
                DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted),
            ),
            (NO_MATCH_ID, DetectionOutcome::NoMatch),
            (
                "dev.baozi.detect.weaker",
                DetectionOutcome::Match(ReadConfidence::Likely),
            ),
        ],
    );
}

#[test]
fn automatic_no_selection_distinguishes_clean_absence_from_degraded_evidence() {
    let clean_request = compile_request(
        [
            TestImporter::successful(
                "dev.baozi.detect.clean.alpha",
                "alpha",
                DetectorBehavior::NoMatch,
            ),
            TestImporter::successful(
                "dev.baozi.detect.clean.zeta",
                "zeta",
                DetectorBehavior::NoMatch,
            ),
        ],
        ImportConfigOverlay::default(),
    );
    let clean = operation_failure(
        import_bytes(
            &clean_request,
            AssetPath::new("asset.unknown").expect("valid source"),
            b"primary",
            ImportOperationControl::default(),
        )
        .expect_err("clean no-match evidence is unsupported format"),
    );
    assert!(matches!(clean.cause(), ImportCause::UnsupportedFormat));
    assert_evidence(
        clean.detection_evidence(),
        &[
            ("dev.baozi.detect.clean.alpha", DetectionOutcome::NoMatch),
            ("dev.baozi.detect.clean.zeta", DetectionOutcome::NoMatch),
        ],
    );

    let degraded_request = compile_request(
        [
            TestImporter::successful(NO_MATCH_ID, "nomatch", DetectorBehavior::NoMatch),
            TestImporter::successful(FAULT_ID, "fault", DetectorBehavior::Fault("detector fault")),
            TestImporter::successful(
                INCONCLUSIVE_ID,
                "inconclusive",
                DetectorBehavior::Inconclusive,
            ),
        ],
        ImportConfigOverlay::default(),
    );
    let degraded = operation_failure(
        import_bytes(
            &degraded_request,
            AssetPath::new("asset.unknown").expect("valid source"),
            b"primary",
            ImportOperationControl::default(),
        )
        .expect_err("fault and inconclusive evidence must not become unsupported format"),
    );
    assert!(matches!(
        degraded.cause(),
        ImportCause::Detection(DetectionFailure::NoSelection {
            had_match: false,
            had_inconclusive: true,
            had_fault: true,
        })
    ));

    let unhinted_maybe_request = compile_request(
        [TestImporter::successful(
            "dev.baozi.detect.maybe",
            "maybe",
            DetectorBehavior::Match(ReadConfidence::Maybe),
        )],
        ImportConfigOverlay::default(),
    );
    let unhinted_maybe = operation_failure(
        import_bytes(
            &unhinted_maybe_request,
            AssetPath::new("asset.unknown").expect("valid source"),
            b"primary",
            ImportOperationControl::default(),
        )
        .expect_err("an unhinted maybe match is not selectable automatically"),
    );
    assert!(matches!(
        unhinted_maybe.cause(),
        ImportCause::Detection(DetectionFailure::NoSelection {
            had_match: true,
            had_inconclusive: false,
            had_fault: false,
        })
    ));
}

#[test]
fn automatic_ambiguity_and_evidence_are_registration_independent() {
    let alpha_id = "dev.baozi.detect.ambiguous.alpha";
    let zeta_id = "dev.baozi.detect.ambiguous.zeta";
    let mut importers = [
        TestImporter::successful(
            zeta_id,
            "zeta",
            DetectorBehavior::Match(ReadConfidence::Likely),
        ),
        TestImporter::successful(
            alpha_id,
            "alpha",
            DetectorBehavior::Match(ReadConfidence::Likely),
        ),
        TestImporter::successful(FAULT_ID, "fault", DetectorBehavior::Fault("detector fault")),
        TestImporter::successful(
            INCONCLUSIVE_ID,
            "inconclusive",
            DetectorBehavior::Inconclusive,
        ),
        TestImporter::successful(NO_MATCH_ID, "nomatch", DetectorBehavior::NoMatch),
    ];
    let mut permutations = 0_usize;
    for_each_permutation(&mut importers, 0, &mut |registration_order| {
        permutations += 1;
        let request = compile_request(
            registration_order.iter().cloned(),
            ImportConfigOverlay::default(),
        );
        let failure = operation_failure(
            import_bytes(
                &request,
                AssetPath::new("asset.unhinted").expect("valid source"),
                b"primary",
                ImportOperationControl::default(),
            )
            .expect_err("equal strong unhinted candidates are ambiguous"),
        );

        let ImportCause::Detection(DetectionFailure::Ambiguous { candidates }) = failure.cause()
        else {
            panic!("expected structured ambiguity");
        };
        assert_eq!(
            candidates.iter().map(FormatId::as_str).collect::<Vec<_>>(),
            [alpha_id, zeta_id]
        );
        assert_evidence(
            failure.detection_evidence(),
            &[
                (alpha_id, DetectionOutcome::Match(ReadConfidence::Likely)),
                (zeta_id, DetectionOutcome::Match(ReadConfidence::Likely)),
                (FAULT_ID, DetectionOutcome::fault("detector fault")),
                (
                    INCONCLUSIVE_ID,
                    DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted),
                ),
                (NO_MATCH_ID, DetectionOutcome::NoMatch),
            ],
        );
    });
    assert_eq!(permutations, 120, "every 5! registration order must run");
}

#[test]
fn probe_budget_boundary_preserves_typed_inconclusive_evidence() {
    let prefix_id = "dev.baozi.detect.prefix";
    for budget in [0_u64, 4] {
        let request = compile_request(
            [TestImporter::successful(
                prefix_id,
                "prefix",
                DetectorBehavior::Prefix(b"baozi"),
            )],
            ImportConfigOverlay::default().with_detection_options(
                DetectionOptions::new(budget).expect("bounded detector budget"),
            ),
        );
        let failure = operation_failure(
            import_bytes(
                &request,
                AssetPath::new("asset.unknown").expect("valid source"),
                b"baozi payload",
                ImportOperationControl::default(),
            )
            .expect_err("an exhausted signature probe remains inconclusive"),
        );
        assert!(matches!(
            failure.cause(),
            ImportCause::Detection(DetectionFailure::NoSelection {
                had_match: false,
                had_inconclusive: true,
                had_fault: false,
            })
        ));
        assert_evidence(
            failure.detection_evidence(),
            &[(
                prefix_id,
                DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted),
            )],
        );
    }

    for budget in [5_u64, 6] {
        let request = compile_request(
            [TestImporter::successful(
                prefix_id,
                "prefix",
                DetectorBehavior::Prefix(b"baozi"),
            )],
            ImportConfigOverlay::default().with_detection_options(
                DetectionOptions::new(budget).expect("bounded detector budget"),
            ),
        );
        let report = import_bytes(
            &request,
            AssetPath::new("asset.unknown").expect("valid source"),
            b"baozi payload",
            ImportOperationControl::default(),
        )
        .expect("an exact or larger probe reveals the signature");
        assert_eq!(report.format().id().as_str(), prefix_id);
        assert_evidence(
            report.detection_evidence(),
            &[(prefix_id, DetectionOutcome::Match(ReadConfidence::Certain))],
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum TerminalCase {
    Parse,
    Validation,
    Cancellation,
    Publication,
}

#[test]
fn ordered_detection_evidence_survives_every_post_selection_terminal_failure() {
    let selected_id = "dev.baozi.detect.terminal.selected";
    let contender_id = "dev.baozi.detect.terminal.contender";

    for terminal_case in [
        TerminalCase::Parse,
        TerminalCase::Validation,
        TerminalCase::Cancellation,
        TerminalCase::Publication,
    ] {
        let cancellation = CancellationToken::new();
        let parser = match terminal_case {
            TerminalCase::Parse => ParseBehavior::FormatFailure,
            TerminalCase::Validation => ParseBehavior::InvalidScene,
            TerminalCase::Cancellation => ParseBehavior::Cancel(cancellation.clone()),
            TerminalCase::Publication => ParseBehavior::Success,
        };
        let request = compile_request(
            [
                TestImporter::successful(
                    selected_id,
                    "selected",
                    DetectorBehavior::Match(ReadConfidence::Certain),
                )
                .with_parser(parser),
                TestImporter::successful(NO_MATCH_ID, "nomatch", DetectorBehavior::NoMatch),
                TestImporter::successful(
                    INCONCLUSIVE_ID,
                    "inconclusive",
                    DetectorBehavior::Inconclusive,
                ),
                TestImporter::successful(
                    FAULT_ID,
                    "fault",
                    DetectorBehavior::Fault("detector fault"),
                ),
                TestImporter::successful(
                    contender_id,
                    "contender",
                    DetectorBehavior::Match(ReadConfidence::Likely),
                ),
            ],
            ImportConfigOverlay::default(),
        );
        let control = match terminal_case {
            TerminalCase::Cancellation => {
                ImportOperationControl::default().with_cancellation(cancellation)
            }
            TerminalCase::Publication => ImportOperationControl::default().with_progress_observer(
                |event: ProgressEvent<ImportProgressPhase, ImportProgressUnit>| {
                    if event.phase() == ImportProgressPhase::Finalize {
                        ProgressDecision::Cancel
                    } else {
                        ProgressDecision::Continue
                    }
                },
            ),
            TerminalCase::Parse | TerminalCase::Validation => ImportOperationControl::default(),
        };
        let failure = operation_failure(
            import_bytes(
                &request,
                AssetPath::new("asset.unhinted").expect("valid source"),
                b"primary",
                control,
            )
            .expect_err("the injected post-selection terminal branch must fail"),
        );

        assert_eq!(
            failure.selected_format().map(FormatId::as_str),
            Some(selected_id),
            "selected format for {terminal_case:?}"
        );
        assert_evidence(
            failure.detection_evidence(),
            &[
                (FAULT_ID, DetectionOutcome::fault("detector fault")),
                (
                    INCONCLUSIVE_ID,
                    DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted),
                ),
                (NO_MATCH_ID, DetectionOutcome::NoMatch),
                (
                    contender_id,
                    DetectionOutcome::Match(ReadConfidence::Likely),
                ),
                (
                    selected_id,
                    DetectionOutcome::Match(ReadConfidence::Certain),
                ),
            ],
        );

        let expected_phase = match terminal_case {
            TerminalCase::Parse | TerminalCase::Cancellation => ImportPhase::SelectionCompleted,
            TerminalCase::Validation => ImportPhase::DraftBuilt,
            TerminalCase::Publication => ImportPhase::Validated,
        };
        assert_eq!(failure.last_completed().phase(), expected_phase);
        match terminal_case {
            TerminalCase::Parse => {
                let ImportCause::Format(envelope) = failure.cause() else {
                    panic!("expected format-owned parser failure");
                };
                assert_eq!(envelope.code(), "test.detection.parser_failure");
            }
            TerminalCase::Validation => {
                assert!(matches!(failure.cause(), ImportCause::Validation(_)));
            }
            TerminalCase::Cancellation | TerminalCase::Publication => {
                assert!(matches!(failure.cause(), ImportCause::Scope(_)));
            }
        }
    }
}
