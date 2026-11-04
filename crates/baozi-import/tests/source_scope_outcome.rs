mod support;

use baozi_core::FormatId;
use baozi_import::{
    CapabilityStatus, CauseOwner, CompiledOptions, DetectionOutcome, DetectionResult,
    DetectionScope, DiagnosticCode, Format, FormatCapability, FormatDescriptor, FormatEncoding,
    FormatFailureDetailSink, FormatFailureEvidenceProducer, ImportCause, ImportConfigOverlay,
    ImportDimension, ImportFailure, ImportLossDisposition, ImportLossKind, ImportLossSubject,
    ImportOperationControl, ImportPlan, ImportReport, ImportRequest, ImportRequestLayer,
    ImportResult, LossPolicy, NoFormatOptions, OptionCompileError, OptionCompileLimits,
    ParseContext, ParseResult, ParseServices, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuildScope, RegistryBuilder, ResourceLimits, ResourceLimitsOverlay, ScopeAbortCause,
    SourceScopeDetailState, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeSelectionOutcome, SourceScopeSelectionRequest,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationOrigin, CancellationToken, ControlCause};
use std::fmt;

const SOURCE: &str = "scene.scope";
const SOURCE_SEMANTIC_LOSS_CODE: &str = "test.source_channel_omitted";

fn selection_outcome() -> SourceScopeSelectionOutcome {
    SourceScopeSelectionOutcome::parentless_node_projection(AssetPath::new(SOURCE).unwrap())
}

#[derive(Debug, Clone, Copy)]
enum Exit {
    NoRecord,
    Success,
    TypedSuccess,
    LightSuccess,
    FailureAfterRecord,
    Duplicate,
}

struct ScopeImporter {
    exit: Exit,
    identities: &'static [&'static str],
}

struct SourceSemanticLossImporter;

struct SelectionImporter {
    requested: SourceScopeSelectionRequest,
    actual_scene: Option<u64>,
    document: &'static str,
    fail_after_record: bool,
}

impl Format for SelectionImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        ScopeImporter {
            exit: Exit::Success,
            identities: &[],
        }
        .descriptor()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut ParseContext<'_, '_>,
    ) -> ParseResult<()> {
        context.with_execution(|exec| -> ParseResult<()> {
            let document = AssetPath::new(self.document).unwrap();
            let actual = match self.actual_scene {
                Some(index) => SourceScopeSelectionOutcome::declared_scene(document, index),
                None => SourceScopeSelectionOutcome::parentless_node_projection(document),
            };
            let mut builder = exec
                .services()?
                .source_scope_builder(self.requested, actual)?;
            builder.observe(
                &mut exec.services()?,
                SourceScopeEntityKind::Scene,
                SourceScopeDisposition::Selected,
                "scene:main",
            )?;
            let record = builder.finish(&mut exec.services()?)?;
            exec.services()?.record_source_scope(record)?;
            if self.fail_after_record {
                return Err(exec.format_failure(TestFormatFailure));
            }
            Ok(())
        })?;
        support::submit_minimal_scene(context)
    }
}

impl Format for SourceSemanticLossImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.source-semantic-loss").unwrap(),
            "Source Semantic Loss",
        )
        .extensions(["scope"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut ParseContext<'_, '_>,
    ) -> ParseResult<()> {
        context.with_execution(|exec| -> ParseResult<()> {
            exec.services()?.record_source_semantic_loss(
                DiagnosticCode::new(SOURCE_SEMANTIC_LOSS_CODE).unwrap(),
                SourceScopeEntityKind::AnimationChannel,
                "/animations/3/channels/5",
                |details| {
                    details.unsigned("animation_index", 3);
                    details.unsigned("channel_index", 5);
                },
                format_args!("omitted one source animation channel"),
            )?;
            Ok(())
        })?;
        support::submit_minimal_scene(context)
    }
}

impl Format for ScopeImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.source-scope").unwrap(),
            "Source Scope",
        )
        .extensions(["scope"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut ParseContext<'_, '_>,
    ) -> ParseResult<()> {
        context.with_execution(|exec| -> ParseResult<()> {
            match self.exit {
                Exit::NoRecord => {}
                Exit::TypedSuccess => record_typed(&mut exec.services()?)?,
                Exit::LightSuccess => record_lights(&mut exec.services()?, self.identities)?,
                Exit::Success | Exit::FailureAfterRecord | Exit::Duplicate => {
                    record(&mut exec.services()?, self.identities)?;
                    match self.exit {
                        Exit::FailureAfterRecord => {
                            return Err(exec.format_failure(TestFormatFailure));
                        }
                        Exit::Duplicate => {
                            record(&mut exec.services()?, &["scene:duplicate"])?;
                            unreachable!("the sealed source-scope slot must reject a duplicate")
                        }
                        _ => {}
                    }
                }
            }
            Ok(())
        })?;
        support::submit_minimal_scene(context)
    }
}

struct CancelBeforeEmptyIdentityImporter {
    cancellation: CancellationToken,
}

impl Format for CancelBeforeEmptyIdentityImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.source-scope-cancellation").unwrap(),
            "Source Scope Cancellation",
        )
        .extensions(["scope"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut ParseContext<'_, '_>,
    ) -> ParseResult<()> {
        context.with_execution(|exec| -> ParseResult<()> {
            let mut builder = exec.services()?.source_scope_builder(
                SourceScopeSelectionRequest::DocumentDefault,
                selection_outcome(),
            )?;
            self.cancellation.cancel();
            builder.observe(
                &mut exec.services()?,
                SourceScopeEntityKind::Scene,
                SourceScopeDisposition::Selected,
                "",
            )?;
            unreachable!("an empty source identity must still observe cancellation")
        })
    }
}

fn record(scope: &mut ParseServices<'_, '_, '_>, identities: &[&str]) -> ParseResult<()> {
    let mut builder = scope.source_scope_builder(
        SourceScopeSelectionRequest::DocumentDefault,
        selection_outcome(),
    )?;
    for (index, identity) in identities.iter().enumerate() {
        let disposition = if index == 0 {
            SourceScopeDisposition::Selected
        } else {
            SourceScopeDisposition::Omitted
        };
        builder.observe(scope, SourceScopeEntityKind::Scene, disposition, identity)?;
    }
    let record = builder.finish(scope)?;
    scope.record_source_scope(record)
}

fn record_typed(scope: &mut ParseServices<'_, '_, '_>) -> ParseResult<()> {
    let mut builder = scope.source_scope_builder(
        SourceScopeSelectionRequest::DocumentDefault,
        selection_outcome(),
    )?;
    for (kind, disposition, identity) in [
        (
            SourceScopeEntityKind::Scene,
            SourceScopeDisposition::Selected,
            "",
        ),
        (
            SourceScopeEntityKind::Node,
            SourceScopeDisposition::Omitted,
            "a",
        ),
        (
            SourceScopeEntityKind::Mesh,
            SourceScopeDisposition::Selected,
            "bc",
        ),
    ] {
        builder.observe(scope, kind, disposition, identity)?;
    }
    let record = builder.finish(scope)?;
    scope.record_source_scope(record)
}

fn record_lights(scope: &mut ParseServices<'_, '_, '_>, identities: &[&str]) -> ParseResult<()> {
    let mut builder = scope.source_scope_builder(
        SourceScopeSelectionRequest::DocumentDefault,
        selection_outcome(),
    )?;
    for (index, identity) in identities.iter().enumerate() {
        builder.observe(
            scope,
            SourceScopeEntityKind::Light,
            if index == 1 {
                SourceScopeDisposition::Omitted
            } else {
                SourceScopeDisposition::Selected
            },
            identity,
        )?;
    }
    {
        let record = builder.finish(scope)?;
        scope.record_source_scope(record)
    }
}

#[derive(Debug)]
struct TestFormatFailure;

impl fmt::Display for TestFormatFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("failure after source-scope publication")
    }
}

impl std::error::Error for TestFormatFailure {}

impl FormatFailureEvidenceProducer for TestFormatFailure {
    fn code(&self) -> &'static str {
        "test.failure_after_source_scope"
    }

    fn visit_details(&self, _sink: &mut dyn FormatFailureDetailSink) {}
}

#[allow(
    clippy::result_large_err,
    reason = "source-scope assertions inspect complete bounded failure evidence"
)]
fn operation_result<T>(result: ImportResult<T>) -> Result<T, ImportFailure> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match error.into_operation_failure() {
            Ok(failure) => Err(failure),
            Err(source) => panic!("source-scope test failed before session creation: {source}"),
        },
    }
}

fn request_for(
    exit: Exit,
    identities: &'static [&'static str],
    limits: ResourceLimits,
    loss_policy: LossPolicy,
) -> ImportPlan {
    compile_request(ScopeImporter { exit, identities }, limits, loss_policy)
}

fn compile_request(
    importer: impl Format,
    limits: ResourceLimits,
    loss_policy: LossPolicy,
) -> ImportPlan {
    let mut registry = RegistryBuilder::new();
    registry.register(importer).unwrap();
    let snapshot = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let config = ImportConfigOverlay::default()
        .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
        .with_loss_policy(loss_policy);
    snapshot
        .plan(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(config),
            ),
            OptionCompileLimits::default(),
        )
        .unwrap()
}

#[allow(
    clippy::result_large_err,
    reason = "source-scope assertions inspect complete bounded failure evidence"
)]
fn run(
    exit: Exit,
    identities: &'static [&'static str],
    limits: ResourceLimits,
    loss_policy: LossPolicy,
) -> Result<ImportReport, ImportFailure> {
    let request = request_for(exit, identities, limits, loss_policy);
    operation_result(request.read_bytes(
        AssetPath::new(SOURCE).unwrap(),
        b"scope",
        ImportOperationControl::default(),
    ))
}

fn provenance_loss_policy() -> LossPolicy {
    LossPolicy::preserve_or_fail().allow(ImportLossKind::ProvenanceEvidenceLoss)
}

#[allow(
    clippy::result_large_err,
    reason = "source-semantic-loss assertions inspect complete bounded failure evidence"
)]
fn run_source_semantic_loss(loss_policy: LossPolicy) -> Result<ImportReport, ImportFailure> {
    let request = compile_request(
        SourceSemanticLossImporter,
        ResourceLimits::default(),
        loss_policy,
    );
    operation_result(request.read_bytes(
        AssetPath::new(SOURCE).unwrap(),
        b"scope",
        ImportOperationControl::default(),
    ))
}

fn assert_source_semantic_loss(
    evidence: &[baozi_import::ImportLossEvidence],
    disposition: ImportLossDisposition,
) {
    assert_eq!(evidence.len(), 1);
    let evidence = &evidence[0];
    assert_eq!(evidence.kind(), ImportLossKind::SourceSemanticLoss);
    assert_eq!(
        evidence.subject(),
        ImportLossSubject::SourceEntity(SourceScopeEntityKind::AnimationChannel)
    );
    assert_eq!(evidence.code().as_str(), SOURCE_SEMANTIC_LOSS_CODE);
    assert_eq!(evidence.source().as_str(), SOURCE);
    assert_eq!(evidence.raw_range(), None);
    assert_eq!(evidence.disposition(), disposition);
}

fn assert_source_scope_bundle_unpublished(failure: &ImportFailure) {
    assert!(failure.source_scope_record().is_none());
    assert!(failure.loss_evidence().is_empty());
    assert!(failure.diagnostics().is_empty());
    for dimension in [
        ImportDimension::SourceScopeRecordCount,
        ImportDimension::LossEvidenceCount,
        ImportDimension::LossEvidenceBytes,
        ImportDimension::DiagnosticCount,
        ImportDimension::DiagnosticBytes,
    ] {
        assert_eq!(
            failure.ledger().dimension(dimension).unwrap().consumed(),
            0,
            "{dimension:?} must not claim an unpublished bundle"
        );
    }
}

#[test]
fn absent_record_remains_none() {
    let report = run(
        Exit::NoRecord,
        &[],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();

    assert!(report.source_scope_record().is_none());
}

#[test]
fn source_semantic_loss_requires_explicit_authority_and_retains_typed_evidence() {
    let failure = run_source_semantic_loss(LossPolicy::preserve_or_fail())
        .expect_err("source semantic loss must be rejected by default");

    assert_source_semantic_loss(
        failure.loss_evidence(),
        ImportLossDisposition::RejectedByLossPolicy,
    );
    assert_eq!(failure.diagnostics().len(), 1);
    assert_eq!(
        failure.diagnostics()[0].object_path(),
        Some("/animations/3/channels/5")
    );
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if abort.owner() == CauseOwner::Import
                && matches!(
                    abort.cause(),
                    ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { code }
                        if code.as_str() == SOURCE_SEMANTIC_LOSS_CODE
                )
    ));

    let report = run_source_semantic_loss(
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    )
    .expect("explicit source semantic loss authority must allow publication");
    assert_source_semantic_loss(report.loss_evidence(), ImportLossDisposition::Applied);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].object_path(),
        Some("/animations/3/channels/5")
    );
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn empty_identity_observation_polls_before_charging_work() {
    let cancellation = CancellationToken::new();
    let request = compile_request(
        CancelBeforeEmptyIdentityImporter {
            cancellation: cancellation.clone(),
        },
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    );
    let failure = operation_result(request.read_bytes(
        AssetPath::new(SOURCE).unwrap(),
        b"scope",
        ImportOperationControl::default().with_cancellation(cancellation),
    ))
    .unwrap_err();

    let baozi_import::ImportCause::Scope(abort) = failure.cause() else {
        panic!("source-scope cancellation must remain an operation-owned scope failure")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        })
    ));
    assert!(failure.source_scope_record().is_none());
    for dimension in [
        ImportDimension::SourceScopeItemCount,
        ImportDimension::SourceScopeDigestBytes,
        ImportDimension::SourceScopeDetailCount,
        ImportDimension::SourceScopeDetailBytes,
    ] {
        let evidence = failure.ledger().dimension(dimension).unwrap();
        assert_eq!(evidence.attempted(), 0, "{dimension:?} charged before poll");
        assert_eq!(evidence.consumed(), 0, "{dimension:?} charged before poll");
    }
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn detailed_record_survives_success_with_exact_typed_counts() {
    let report = run(
        Exit::Success,
        &["scene:main", "scene:preview"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();
    let record = report.source_scope_record().unwrap();

    assert_eq!(record.detail_state(), SourceScopeDetailState::Detailed);
    let counts = record.counts(SourceScopeEntityKind::Scene);
    assert_eq!(counts.selected(), 1);
    assert_eq!(counts.omitted(), 1);
    let details = record.details().collect::<Vec<_>>();
    assert_eq!(details.len(), 2);
    assert_eq!(details[0].identity(), "scene:main");
    assert_eq!(details[0].disposition(), SourceScopeDisposition::Selected);
    assert_eq!(details[1].identity(), "scene:preview");
    assert_eq!(details[1].disposition(), SourceScopeDisposition::Omitted);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn typed_counts_remain_partitioned_by_source_entity_kind() {
    let report = run(
        Exit::TypedSuccess,
        &[],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();
    let record = report.source_scope_record().unwrap();

    let scenes = record.counts(SourceScopeEntityKind::Scene);
    assert_eq!(scenes.selected(), 1);
    assert_eq!(scenes.omitted(), 0);
    assert_eq!(record.counts(SourceScopeEntityKind::Node).selected(), 0);
    assert_eq!(record.counts(SourceScopeEntityKind::Node).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Mesh).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Mesh).omitted(), 0);
    assert_eq!(record.counts(SourceScopeEntityKind::Material).total(), 0);
    assert_eq!(
        record
            .details()
            .map(|detail| detail.kind())
            .collect::<Vec<_>>(),
        vec![
            SourceScopeEntityKind::Scene,
            SourceScopeEntityKind::Node,
            SourceScopeEntityKind::Mesh,
        ]
    );
    assert_eq!(
        record.digest().as_bytes(),
        &[
            0xfa, 0x60, 0x52, 0x53, 0x6a, 0x36, 0xa6, 0x1f, 0xab, 0xaf, 0x5d, 0xd8, 0xaa, 0x94,
            0xb5, 0x90, 0x3a, 0xd8, 0xf6, 0x65, 0x9d, 0x83, 0x35, 0x4f, 0x96, 0x72, 0x64, 0x2b,
            0x63, 0xf0, 0x3d, 0xd4,
        ]
    );
}

#[test]
fn light_definitions_have_a_distinct_tag_and_full_digest_when_details_collapse() {
    assert_eq!(SourceScopeEntityKind::AnimationSampler as u8, 15);
    assert_eq!(SourceScopeEntityKind::Light as u8, 16);
    // Independently framed v1 prefix, default/parentless scene.scope selection,
    // then [16, disposition] + u64_le(2) + l0/l1/l2. No production encoder.
    let expected = [
        0x1e, 0x10, 0xbb, 0x69, 0x8e, 0x58, 0x1f, 0x68, 0xe1, 0x19, 0x36, 0xac, 0x9a, 0x4c, 0x99,
        0xd2, 0xa4, 0x9e, 0xf8, 0x00, 0xb2, 0x68, 0xb7, 0x76, 0x3f, 0x89, 0x40, 0x35, 0xd3, 0xd0,
        0xf4, 0x28,
    ];
    for collapse in [false, true] {
        let mut limits = ResourceLimits::default();
        limits.max_source_scope_items = 3;
        limits.max_source_scope_digest_bytes = 6;
        if collapse {
            limits.max_source_scope_detail_count = 0;
        }
        let report = run(
            Exit::LightSuccess,
            &["l0", "l1", "l2"],
            limits,
            provenance_loss_policy(),
        )
        .unwrap();
        let record = report.source_scope_record().unwrap();
        assert_eq!(record.counts(SourceScopeEntityKind::Light).selected(), 2);
        assert_eq!(record.counts(SourceScopeEntityKind::Light).omitted(), 1);
        assert_eq!(
            record
                .counts(SourceScopeEntityKind::AnimationSampler)
                .total(),
            0
        );
        assert_eq!(record.counts(SourceScopeEntityKind::Node).total(), 0);
        assert_eq!(record.digest().as_bytes(), &expected);
        if collapse {
            assert_eq!(record.detail_state(), SourceScopeDetailState::Collapsed);
            assert_eq!(record.details().count(), 0);
            assert_eq!(record.collapsed_detail_count(), 3);
            assert_eq!(record.collapsed_detail_bytes(), 6);
        } else {
            assert_eq!(record.detail_state(), SourceScopeDetailState::Detailed);
            let details = record.details().collect::<Vec<_>>();
            assert_eq!(details.len(), 3);
            for (detail, identity) in details.iter().zip(["l0", "l1", "l2"]) {
                assert_eq!(detail.kind(), SourceScopeEntityKind::Light);
                assert_eq!(detail.identity(), identity);
            }
            assert_eq!(details[1].disposition(), SourceScopeDisposition::Omitted);
        }
        assert_eq!(report.ledger().current_live_bytes(), 0);
    }

    let mut limits = ResourceLimits::default();
    limits.max_source_scope_detail_count = 0;
    let changed = run(
        Exit::LightSuccess,
        &["l0", "l1", "l3"],
        limits,
        provenance_loss_policy(),
    )
    .unwrap();
    assert_ne!(
        changed.source_scope_record().unwrap().digest().as_bytes(),
        &expected
    );
}

#[test]
fn record_survives_a_later_format_failure() {
    let failure = run(
        Exit::FailureAfterRecord,
        &["scene:main", "scene:other"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap_err();

    let record = failure.source_scope_record().unwrap();
    assert_eq!(record.counts(SourceScopeEntityKind::Scene).total(), 2);
    assert_eq!(record.details().count(), 2);
}

#[test]
fn duplicate_record_is_rejected_and_retains_the_first_record() {
    let failure = run(
        Exit::Duplicate,
        &["scene:first"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap_err();

    assert_eq!(
        failure
            .source_scope_record()
            .unwrap()
            .details()
            .next()
            .unwrap()
            .identity(),
        "scene:first"
    );
    let baozi_import::ImportCause::Scope(abort) = failure.cause() else {
        panic!("duplicate submission must be a scope failure")
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::DescriptorContract {
            code: "source-scope-record-already-submitted"
        }
    ));
}

#[test]
fn exact_detail_budget_preserves_full_details() {
    let mut limits = ResourceLimits::default();
    limits.max_source_scope_detail_count = 2;
    limits.max_source_scope_detail_bytes = 6;
    let report = run(
        Exit::Success,
        &["abc", "def"],
        limits,
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();

    let record = report.source_scope_record().unwrap();
    assert_eq!(record.detail_state(), SourceScopeDetailState::Detailed);
    let detail_bytes = report
        .ledger()
        .dimension(ImportDimension::SourceScopeDetailBytes)
        .unwrap();
    assert_eq!(detail_bytes.consumed(), 6);
}

#[test]
fn item_and_full_digest_budgets_are_independent_and_exact() {
    let mut exact = ResourceLimits::default();
    exact.max_source_scope_items = 1;
    exact.max_source_scope_digest_bytes = 3;
    let report = run(
        Exit::Success,
        &["abc"],
        exact,
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::SourceScopeItemCount)
            .unwrap()
            .consumed(),
        1
    );
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::SourceScopeDigestBytes)
            .unwrap()
            .consumed(),
        3
    );

    let mut digest_excess = ResourceLimits::default();
    digest_excess.max_source_scope_digest_bytes = 2;
    let digest_failure = run(
        Exit::Success,
        &["abc"],
        digest_excess,
        provenance_loss_policy(),
    )
    .unwrap_err();
    assert!(digest_failure.source_scope_record().is_none());

    let mut item_excess = ResourceLimits::default();
    item_excess.max_source_scope_items = 0;
    let item_failure =
        run(Exit::Success, &[""], item_excess, provenance_loss_policy()).unwrap_err();
    assert!(item_failure.source_scope_record().is_none());
}

#[test]
fn excess_detail_fails_without_provenance_loss_authority() {
    let mut limits = ResourceLimits::default();
    limits.max_source_scope_detail_bytes = 5;
    let failure = run(
        Exit::Success,
        &["abc", "def"],
        limits,
        LossPolicy::preserve_or_fail(),
    )
    .unwrap_err();

    assert_source_scope_bundle_unpublished(&failure);
}

#[test]
fn authorized_excess_collapses_details_and_atomically_publishes_evidence() {
    let mut limits = ResourceLimits::default();
    limits.max_source_scope_detail_bytes = 5;
    let report = run(
        Exit::Success,
        &["abc", "def", "ghi"],
        limits,
        provenance_loss_policy(),
    )
    .unwrap();

    let record = report.source_scope_record().unwrap();
    assert_eq!(record.detail_state(), SourceScopeDetailState::Collapsed);
    assert_eq!(record.counts(SourceScopeEntityKind::Scene).total(), 3);
    assert_eq!(record.details().count(), 0);
    assert_eq!(record.collapsed_detail_count(), 3);
    assert_eq!(record.collapsed_detail_bytes(), 9);
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(
        report.loss_evidence()[0].subject(),
        ImportLossSubject::SourceScopeDetails
    );
    assert_eq!(report.loss_evidence()[0].raw_range(), None);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "baozi.source_scope_details_collapsed"
    );
    for dimension in [
        ImportDimension::SourceScopeRecordCount,
        ImportDimension::LossEvidenceCount,
        ImportDimension::DiagnosticCount,
    ] {
        assert_eq!(report.ledger().dimension(dimension).unwrap().consumed(), 1);
    }
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::LossEvidenceBytes)
            .unwrap()
            .consumed()
            > 0
    );
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::DiagnosticBytes)
            .unwrap()
            .consumed()
            > 0
    );
}

#[test]
fn loss_budget_failure_rolls_back_the_collapsed_bundle() {
    let mut limits = ResourceLimits::default();
    limits.max_source_scope_detail_bytes = 0;
    limits.max_loss_evidence = 0;
    let failure = run(Exit::Success, &["scene"], limits, provenance_loss_policy()).unwrap_err();

    assert_source_scope_bundle_unpublished(&failure);
}

#[test]
fn diagnostic_budget_failure_rolls_back_the_collapsed_bundle() {
    let mut limits = ResourceLimits::default();
    limits.max_source_scope_detail_bytes = 0;
    limits.max_diagnostics = 0;
    let failure = run(Exit::Success, &["scene"], limits, provenance_loss_policy()).unwrap_err();

    assert_source_scope_bundle_unpublished(&failure);
    assert_eq!(failure.diagnostic_summary().attempted_count(), 1);
    assert_eq!(failure.diagnostic_summary().dropped_count(), 1);
    assert_eq!(failure.diagnostic_summary().stored_count(), 0);
    assert!(matches!(
        failure.cause(),
        baozi_import::ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: "source-scope-collapse-diagnostic-count",
                    limit: 0,
                }
            )
    ));
}

#[test]
fn diagnostic_byte_budget_failure_reports_the_byte_limit() {
    let mut limits = ResourceLimits::default();
    limits.max_source_scope_detail_bytes = 0;
    limits.max_diagnostic_bytes = 0;
    let failure = run(Exit::Success, &["scene"], limits, provenance_loss_policy()).unwrap_err();

    assert_source_scope_bundle_unpublished(&failure);
    assert_eq!(failure.diagnostic_summary().attempted_count(), 1);
    assert_eq!(failure.diagnostic_summary().dropped_count(), 1);
    assert_eq!(failure.diagnostic_summary().stored_count(), 0);
    assert!(matches!(
        failure.cause(),
        baozi_import::ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: "source-scope-collapse-diagnostic-bytes",
                    limit: 0,
                }
            )
    ));
}

#[test]
fn record_budget_failure_rolls_back_the_collapsed_bundle() {
    let mut limits = ResourceLimits::default();
    limits.max_source_scope_detail_bytes = 0;
    limits.max_source_scope_records = 0;
    let failure = run(Exit::Success, &["scene"], limits, provenance_loss_policy()).unwrap_err();

    assert_source_scope_bundle_unpublished(&failure);
}

#[test]
fn full_digest_is_sensitive_to_tail_changes() {
    let first = run(
        Exit::Success,
        &["scene:prefix-tail-a"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();
    let second = run(
        Exit::Success,
        &["scene:prefix-tail-b"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();

    assert_ne!(
        first.source_scope_record().unwrap().digest(),
        second.source_scope_record().unwrap().digest()
    );

    let segmented = run(
        Exit::Success,
        &["a", "bc"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();
    let joined = run(
        Exit::Success,
        &["ab", "c"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();
    let reordered = run(
        Exit::Success,
        &["bc", "a"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();
    let segmented_digest = segmented.source_scope_record().unwrap().digest();
    assert_ne!(
        segmented_digest,
        joined.source_scope_record().unwrap().digest()
    );
    assert_ne!(
        segmented_digest,
        reordered.source_scope_record().unwrap().digest()
    );

    let canonical = run(
        Exit::Success,
        &["scene:main"],
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail(),
    )
    .unwrap();
    assert_eq!(
        canonical.source_scope_record().unwrap().digest().as_bytes(),
        &[
            0x78, 0xed, 0x3d, 0xe1, 0x22, 0x0e, 0xca, 0x84, 0xd0, 0x5a, 0x1d, 0x2c, 0xb0, 0xf8,
            0xce, 0xfe, 0x21, 0xce, 0x54, 0xc9, 0xb7, 0x8c, 0x91, 0xf0, 0xcb, 0x02, 0xdb, 0x79,
            0x64, 0xc7, 0x25, 0x2b,
        ]
    );
}

#[test]
fn selection_request_is_retained_and_changes_the_digest_for_the_same_outcome() {
    let mut digests = Vec::new();
    // Independent v1 transcripts use LE64 scene indices and one selected scene:main row.
    for (requested, expected) in [
        (
            SourceScopeSelectionRequest::DocumentDefault,
            [
                0xa9, 0x3f, 0xd6, 0x22, 0xcf, 0xb2, 0x0f, 0x9c, 0x45, 0x1a, 0xce, 0xb0, 0xa3, 0x04,
                0x4c, 0xbf, 0x59, 0xd7, 0xb1, 0xeb, 0x82, 0x40, 0xbb, 0xac, 0xf5, 0x59, 0x73, 0x9c,
                0x0d, 0xb7, 0x8a, 0x99,
            ],
        ),
        (
            SourceScopeSelectionRequest::ExplicitScene { scene_index: 3 },
            [
                0x81, 0xf6, 0xbf, 0x93, 0x71, 0x17, 0xa4, 0x3f, 0x20, 0xe3, 0xeb, 0x1d, 0xdb, 0x7e,
                0xc4, 0x8f, 0x2c, 0x64, 0x43, 0x21, 0x0f, 0x97, 0xc4, 0x49, 0x66, 0x5a, 0x8b, 0xad,
                0x7e, 0xe0, 0x66, 0x37,
            ],
        ),
    ] {
        let mut limits = ResourceLimits::default();
        limits.max_source_scope_items = 1;
        limits.max_source_scope_digest_bytes = 10;
        limits.max_source_scope_detail_count = 1;
        limits.max_source_scope_detail_bytes = 10;
        let request = compile_request(
            SelectionImporter {
                requested,
                actual_scene: Some(3),
                document: SOURCE,
                fail_after_record: false,
            },
            limits,
            LossPolicy::preserve_or_fail(),
        );
        let report = operation_result(request.read_bytes(
            AssetPath::new(SOURCE).unwrap(),
            b"scope",
            ImportOperationControl::default(),
        ))
        .unwrap();
        let record = report.source_scope_record().unwrap();
        assert_eq!(record.selection_request(), &requested);
        assert_eq!(record.selection_outcome().scene_index(), Some(3));
        assert_eq!(record.detail_state(), SourceScopeDetailState::Detailed);
        assert_eq!(record.details().count(), 1);
        assert_eq!(record.digest().as_bytes(), &expected);
        digests.push(record.digest());
        assert_eq!(
            report
                .ledger()
                .dimension(ImportDimension::SourceScopeDigestBytes)
                .unwrap()
                .consumed(),
            10
        );
    }
    assert_ne!(digests[0], digests[1]);
}

#[test]
fn invalid_selection_request_outcome_pairs_are_rejected_before_evidence_admission() {
    for (requested, actual_scene, document, code) in [
        (
            SourceScopeSelectionRequest::ExplicitScene { scene_index: 3 },
            Some(4),
            SOURCE,
            "source-scope-selection-request-mismatch",
        ),
        (
            SourceScopeSelectionRequest::ExplicitScene { scene_index: 0 },
            None,
            SOURCE,
            "source-scope-selection-request-mismatch",
        ),
        (
            SourceScopeSelectionRequest::DocumentDefault,
            Some(3),
            "other.scope",
            "source-scope-selection-document-mismatch",
        ),
    ] {
        let request = compile_request(
            SelectionImporter {
                requested,
                actual_scene,
                document,
                fail_after_record: false,
            },
            ResourceLimits::default(),
            LossPolicy::preserve_or_fail(),
        );
        let failure = operation_result(request.read_bytes(
            AssetPath::new(SOURCE).unwrap(),
            b"scope",
            ImportOperationControl::default(),
        ))
        .unwrap_err();
        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::DescriptorContract {code: actual} if *actual == code)
        ));
        assert_source_scope_bundle_unpublished(&failure);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
}

#[test]
fn collapsed_selection_request_survives_later_failure_with_the_complete_digest() {
    let requested = SourceScopeSelectionRequest::ExplicitScene { scene_index: 3 };
    let mut digests = Vec::new();
    for collapse in [false, true] {
        let mut limits = ResourceLimits::default();
        limits.max_source_scope_detail_count = if collapse { 0 } else { 1 };
        let request = compile_request(
            SelectionImporter {
                requested,
                actual_scene: Some(3),
                document: SOURCE,
                fail_after_record: true,
            },
            limits,
            provenance_loss_policy(),
        );
        let failure = operation_result(request.read_bytes(
            AssetPath::new(SOURCE).unwrap(),
            b"scope",
            ImportOperationControl::default(),
        ))
        .unwrap_err();
        let record = failure.source_scope_record().unwrap();
        assert_eq!(record.selection_request(), &requested);
        assert_eq!(record.selection_outcome().scene_index(), Some(3));
        assert_eq!(record.counts(SourceScopeEntityKind::Scene).selected(), 1);
        assert_eq!(record.details().count(), usize::from(!collapse));
        assert_eq!(record.collapsed_detail_count(), u64::from(collapse));
        assert_eq!(
            record.detail_state(),
            if collapse {
                SourceScopeDetailState::Collapsed
            } else {
                SourceScopeDetailState::Detailed
            }
        );
        assert_eq!(failure.loss_evidence().len(), usize::from(collapse));
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        digests.push(record.digest());
    }
    assert_eq!(digests[0], digests[1]);
}
