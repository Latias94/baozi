use super::*;
use crate::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, DeformationAuthoringBatch,
    GeometryAuthoringBatch, LineageAuthoringBatch, MetadataAuthoringBatch, NodeAuthoringBatch,
    OperationKind, OptionsDigest, OwnerProvenanceAuthoring, Presence, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
    SceneAuthoringBatch, SceneSpace, SpatialAuthoringBatch,
};
use baozi_operation::{CounterClass, LedgerCause};

struct AsciiSpanImporter {
    decoded_capacity: Option<usize>,
    emission_prefix: Option<u64>,
    check: fn(&ConformedTextInput<'_>),
}

impl Format for AsciiSpanImporter {
    type Options = crate::NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut crate::RegistryBuildScope<'_>,
    ) -> Result<crate::CompiledOptions<Self::Options>, crate::OptionCompileError> {
        FrontierImporter.compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.ascii-emission-spans").unwrap(),
            "ASCII Emission Spans",
        )
        .extensions(["txt"])
        .encoding(FormatEncoding::Text)
        .text_profile(
            TextProfile::builder(
                TextHardCaps::new(65_536, 65_536, 65_536, 65_536, 65_536).unwrap(),
            )
            .source_encodings([TextSourceEncoding::Utf8])
            .bom_policy(TextBomPolicy::Accepted)
            .newlines([TextNewline::Lf, TextNewline::CrLf, TextNewline::Cr])
            .lexical_mode(TextLexicalMode::Utf8Document)
            .decoding_modes([
                TextDecodingMode::RejectInvalid,
                TextDecodingMode::ReplaceInvalidUtf8,
            ])
            .control_policy(TextControlPolicy::RejectAll)
            .build()
            .unwrap(),
        )
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        probe: &crate::Probe<'_>,
        hint: &ReadHint,
        scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        FrontierImporter.detect(probe, hint, scope)
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut crate::ParseContext<'_, '_>,
    ) -> ParseResult<()> {
        context.with_execution(|exec| -> ParseResult<()> {
            let source = exec.services()?.diagnostic_source();
            let bytes = source.bytes();
            if let Some(prefix) = self.emission_prefix {
                exec.tighten_test_decode_work(2 * bytes.len() as u64, bytes.len() as u64 + prefix);
            }
            let descriptor = self.descriptor();
            let facts = TextProfileFacts::new(descriptor.text_profile().unwrap());
            let options =
                TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "ascii-span-text");
            let mut children = exec.children()?;
            let mut plan = match plan_text_input(&mut children, &facts, bytes, options) {
                Ok(plan) => plan,
                Err(TextInputScanError::Scope(failure)) => return Err(failure),
                Err(TextInputScanError::Violation(_)) => panic!("the span fixture must conform"),
            };
            if let Some(capacity) = self.decoded_capacity {
                plan.decoded_len = capacity;
            }
            let input = build_text_input(&mut children, &facts, bytes, options, plan, true)?;
            (self.check)(&input);
            Ok(())
        })?;

        let provenance_ids = [ProvenanceLocalId::new(0)];
        let operation_kind = OperationKind::Generated;
        let provenance_nodes = [ProvenanceNodeAuthoring::operation(
            ProvenanceOperationAuthoring::new(
                &operation_kind,
                OptionsDigest::sha256([0; 32]),
                &[],
                &[],
            ),
        )];
        let owner_provenance = [
            OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &provenance_ids),
            OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &provenance_ids),
        ];
        let scene_space = SceneSpace::UNKNOWN;
        context.submit_complete(SceneAuthoringBatch::new(
            SpatialAuthoringBatch::new(&scene_space, &[], &[]),
            NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            ProfileAuthoringBatch::new(
                LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owner_provenance),
                MetadataAuthoringBatch::empty(),
            ),
        ))
    }
}

fn span_request(importer: AsciiSpanImporter) -> crate::ImportPlan {
    let mut registry = RegistryBuilder::new();
    registry.register(importer).unwrap();
    registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(&ImportRequest::new(), OptionCompileLimits::default())
        .unwrap()
}

fn bulk_ascii() -> Vec<u8> {
    format!(
        "{}\n{}\n{}",
        "a".repeat(4095),
        "b".repeat(4096),
        "c".repeat(513)
    )
    .into_bytes()
}

#[test]
fn bulk_ascii_spans_retain_text_line_indices_and_literal_work() {
    let request = span_request(AsciiSpanImporter {
        decoded_capacity: None,
        emission_prefix: None,
        check: |input| {
            assert_eq!(input.text().as_bytes(), bulk_ascii());
            assert!(input.bom().is_none());
            assert!(input.replacements.is_empty());
            assert_eq!(
                input.line_starts.as_slice(),
                [
                    TextInputLineStart {
                        raw_offset: 0,
                        decoded_offset: 0,
                        replacement_index: 0,
                    },
                    TextInputLineStart {
                        raw_offset: 4096,
                        decoded_offset: 4096,
                        replacement_index: 0,
                    },
                    TextInputLineStart {
                        raw_offset: 8193,
                        decoded_offset: 8193,
                        replacement_index: 0,
                    },
                ]
            );
            let lines = input.lines().collect::<Vec<_>>();
            assert_eq!(lines.len(), 3);
            let expected_lines = [
                (1, "a".repeat(4095), 0, 4095, 4096, Some(TextNewline::Lf)),
                (2, "b".repeat(4096), 4096, 8192, 8193, Some(TextNewline::Lf)),
                (3, "c".repeat(513), 8193, 8706, 8706, None),
            ];
            for (line, (number, text, start, content_end, record_end, newline)) in
                lines.iter().zip(expected_lines)
            {
                assert_eq!(line.number(), number);
                assert_eq!(line.text(), text);
                assert_eq!(
                    line.raw_content_range(),
                    DiagnosticByteRange::new(start, content_end).unwrap()
                );
                assert_eq!(
                    line.raw_record_range(),
                    DiagnosticByteRange::new(start, record_end).unwrap()
                );
                assert_eq!(
                    line.decoded_content_range(),
                    start as usize..content_end as usize
                );
                assert_eq!(
                    line.decoded_record_range(),
                    start as usize..record_end as usize
                );
                assert_eq!(line.newline(), newline);
            }
        },
    });
    let bytes = bulk_ascii();
    let probe = FrontierProbeGuard::install(ScanPass::Building, usize::MAX, FrontierAction::Panic);
    let report = request
        .read_bytes(
            AssetPath::new("ascii-spans.txt").unwrap(),
            &bytes,
            ImportOperationControl::default(),
        )
        .expect("bulk ASCII emission must build and submit the checked text");
    let probe = probe.finish();
    assert_eq!(probe.endpoints.first(), Some(&1));
    assert!(probe.endpoints.windows(2).any(|pair| pair[1] - pair[0] > 1));
    assert!(probe.endpoints.contains(&4096));
    assert!(probe.endpoints.contains(&4097));
    assert!(probe.endpoints.contains(&8192));
    assert!(probe.endpoints.contains(&8194));
    assert_eq!(probe.completed, bytes.len());
    assert_eq!(probe.raised_at, None);
    let decode = report
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .unwrap();
    assert_eq!(
        (decode.attempted(), decode.consumed()),
        (2 * bytes.len() as u64, 2 * bytes.len() as u64)
    );
}

#[test]
fn ascii_span_budget_denial_keeps_the_last_allowed_byte() {
    const PREFIX: u64 = 4613;
    let request = span_request(AsciiSpanImporter {
        decoded_capacity: None,
        emission_prefix: Some(PREFIX),
        check: |_| panic!("the consumed budget must stop emission before returning text"),
    });
    let bytes = bulk_ascii();
    let probe = FrontierProbeGuard::install(ScanPass::Building, usize::MAX, FrontierAction::Panic);
    let failure = request
        .read_bytes(
            AssetPath::new("ascii-span-budget.txt").unwrap(),
            &bytes,
            ImportOperationControl::default(),
        )
        .expect_err("emission must reject the first scalar beyond the bounded ASCII prefix")
        .into_operation_failure()
        .expect("ordinary consumed-work failure");
    let probe = probe.finish();
    let limit = bytes.len() as u64 + PREFIX;
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(
            abort.cause(),
            ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                dimension: ImportDimension::DecodeWork,
                class: CounterClass::Consumed,
                value,
                limit: actual_limit,
            }) if *value == limit + 1 && *actual_limit == limit
        )
    ));
    assert_eq!(probe.endpoints.first(), Some(&1));
    assert!(probe.endpoints.windows(2).any(|pair| pair[1] - pair[0] > 1));
    assert_eq!(probe.completed, PREFIX as usize);
    let final_span = probe.completed - probe.endpoints[probe.endpoints.len() - 2];
    assert!((2..=crate::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS).contains(&final_span));
    let decode = failure
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .unwrap();
    assert_eq!(
        (decode.attempted(), decode.consumed()),
        (2 * bytes.len() as u64, limit)
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert!(failure.diagnostics().is_empty());
}

#[test]
fn shortened_bulk_copy_capacity_fails_at_the_original_second_copy() {
    let request = span_request(AsciiSpanImporter {
        decoded_capacity: Some(TEXT_INPUT_WORK_CHUNK_BYTES),
        emission_prefix: None,
        check: |_| panic!("the second full decoded copy must exhaust its shortened capacity"),
    });
    let bytes = bulk_ascii();
    let probe = FrontierProbeGuard::install(ScanPass::Building, usize::MAX, FrontierAction::Panic);
    let failure = request
        .read_bytes(
            AssetPath::new("ascii-span-copy.txt").unwrap(),
            &bytes,
            ImportOperationControl::default(),
        )
        .expect_err("the original 8192-byte endpoint must complete before copy rejection")
        .into_operation_failure()
        .expect("ordinary decoded-copy failure");
    let probe = probe.finish();
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(
            abort.cause(),
            ScopeAbortCause::DescriptorContract { code: "decoded-text-length-exhausted" }
        )
    ));
    assert_eq!(probe.endpoints.first(), Some(&1));
    assert!(probe.endpoints.windows(2).any(|pair| pair[1] - pair[0] > 1));
    assert!(probe.endpoints.contains(&4096));
    assert!(probe.endpoints.contains(&4097));
    assert!(!probe.endpoints.contains(&8192));
    assert!(probe.completed < 8192);
    let failing_span = 8192 - probe.completed;
    assert!((2..=crate::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS).contains(&failing_span));
    let decode = failure
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .unwrap();
    assert_eq!(
        (decode.attempted(), decode.consumed()),
        (2 * bytes.len() as u64, bytes.len() as u64 + 8192)
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert!(failure.diagnostics().is_empty());
}

#[test]
fn ascii_continuations_keep_bom_utf8_replacement_and_whole_newline_frontiers() {
    let request = span_request(AsciiSpanImporter {
        decoded_capacity: None,
        emission_prefix: None,
        check: |input| {
            assert_eq!(
                input.text(),
                format!(
                    "{}\u{1f4a9}{}\r\n{}\u{fffd}z\r{}",
                    "a".repeat(4092),
                    "b".repeat(4091),
                    "c".repeat(300),
                    "d".repeat(600),
                )
            );
            let bom = input.bom().expect("the original BOM must remain recorded");
            assert_eq!(bom.raw_range(), DiagnosticByteRange::new(0, 3).unwrap());
            assert!(!bom.requires_diagnostic());
            assert_eq!(
                input.line_starts.as_slice(),
                [
                    TextInputLineStart {
                        raw_offset: 3,
                        decoded_offset: 0,
                        replacement_index: 0,
                    },
                    TextInputLineStart {
                        raw_offset: 8192,
                        decoded_offset: 8189,
                        replacement_index: 0,
                    },
                    TextInputLineStart {
                        raw_offset: 8496,
                        decoded_offset: 8494,
                        replacement_index: 1,
                    },
                ]
            );
            assert_eq!(input.replacements.len(), 1);
            let replacement = &input.replacements.as_slice()[0];
            assert_eq!(
                replacement.raw_range(),
                DiagnosticByteRange::new(8492, 8494).unwrap()
            );
            assert_eq!(
                (replacement.decoded_start(), replacement.decoded_end()),
                (8489, 8492)
            );
            let lines = input.lines().collect::<Vec<_>>();
            assert_eq!(lines.len(), 3);
            assert_eq!(lines[0].newline(), Some(TextNewline::CrLf));
            assert_eq!(lines[1].newline(), Some(TextNewline::Cr));
            assert_eq!(lines[2].newline(), None);
            assert_eq!(
                lines[0].raw_record_range(),
                DiagnosticByteRange::new(3, 8192).unwrap()
            );
            assert_eq!(
                lines[1].raw_record_range(),
                DiagnosticByteRange::new(8192, 8496).unwrap()
            );
            assert_eq!(
                lines[2].raw_record_range(),
                DiagnosticByteRange::new(8496, 9096).unwrap()
            );
        },
    });
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(&vec![b'a'; 4092]);
    bytes.extend_from_slice(b"\xf0\x9f\x92\xa9");
    bytes.extend_from_slice(&vec![b'b'; 4091]);
    bytes.extend_from_slice(b"\r\n");
    bytes.extend_from_slice(&vec![b'c'; 300]);
    bytes.extend_from_slice(b"\xe2\x82z\r");
    bytes.extend_from_slice(&vec![b'd'; 600]);
    let probe = FrontierProbeGuard::install(ScanPass::Building, usize::MAX, FrontierAction::Panic);
    let report = request
        .read_bytes(
            AssetPath::new("mixed-ascii-spans.txt").unwrap(),
            &bytes,
            ImportOperationControl::default(),
        )
        .expect("bounded ASCII spans must retain the original special-unit handling");
    let probe = probe.finish();
    assert_eq!(probe.endpoints[0], 4);
    assert!(probe.endpoints.windows(2).any(|pair| pair == [4095, 4099]));
    assert!(probe.endpoints.windows(2).any(|pair| pair == [8190, 8192]));
    assert!(probe.endpoints.windows(2).any(|pair| pair == [8492, 8494]));
    assert!(probe.endpoints.windows(2).any(|pair| pair[1] - pair[0] > 4));
    assert_eq!(probe.completed, bytes.len());
    let decode = report
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .unwrap();
    assert_eq!(
        (decode.attempted(), decode.consumed()),
        (2 * bytes.len() as u64, 2 * bytes.len() as u64)
    );
}
