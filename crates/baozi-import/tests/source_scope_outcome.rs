use baozi_core::{FormatId, SceneDraft};
use baozi_import::{
    CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatFailureDetailSink, FormatFailureSource, FormatImporter,
    ImportConfigOverlay, ImportDimension, ImportFailure, ImportLossKind, ImportLossSubject,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, ImportResult,
    LossPolicy, OptionCompileLimits, ParseResult, ParseScope, PreparedImportRequest,
    ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder, ResourceLimits,
    ResourceLimitsOverlay, ScopeAbortCause, SourceScopeDetailState, SourceScopeDisposition,
    SourceScopeEntityKind, import_bytes,
};
use baozi_io::AssetPath;
use std::fmt;

const SOURCE: &str = "scene.scope";

#[derive(Debug, Clone, Copy)]
enum Exit {
    NoRecord,
    Success,
    TypedSuccess,
    FailureAfterRecord,
    Duplicate,
}

struct ScopeImporter {
    exit: Exit,
    identities: &'static [&'static str],
}

impl FormatImporter for ScopeImporter {
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
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        if matches!(self.exit, Exit::NoRecord) {
            return scope.scene_builder()?.finish_draft();
        }
        if matches!(self.exit, Exit::TypedSuccess) {
            record_typed(scope)?;
            return scope.scene_builder()?.finish_draft();
        }

        record(scope, self.identities)?;
        match self.exit {
            Exit::NoRecord => unreachable!(),
            Exit::Success => scope.scene_builder()?.finish_draft(),
            Exit::TypedSuccess => unreachable!(),
            Exit::FailureAfterRecord => Err(scope.format_failure(TestFormatFailure)),
            Exit::Duplicate => {
                record(scope, &["scene:duplicate"])?;
                unreachable!("the sealed source-scope slot must reject a duplicate")
            }
        }
    }
}

fn record(scope: &ParseScope<'_>, identities: &[&str]) -> ParseResult<()> {
    let mut builder = scope.source_scope_builder()?;
    for (index, identity) in identities.iter().enumerate() {
        let disposition = if index == 0 {
            SourceScopeDisposition::Selected
        } else {
            SourceScopeDisposition::Omitted
        };
        builder.observe(SourceScopeEntityKind::Scene, disposition, identity)?;
    }
    let record = builder.finish()?;
    scope.record_source_scope(record)
}

fn record_typed(scope: &ParseScope<'_>) -> ParseResult<()> {
    let mut builder = scope.source_scope_builder()?;
    for (kind, disposition, identity) in [
        (
            SourceScopeEntityKind::Scene,
            SourceScopeDisposition::Selected,
            "scene:main",
        ),
        (
            SourceScopeEntityKind::Node,
            SourceScopeDisposition::Selected,
            "node:root",
        ),
        (
            SourceScopeEntityKind::Node,
            SourceScopeDisposition::Omitted,
            "node:preview",
        ),
        (
            SourceScopeEntityKind::Mesh,
            SourceScopeDisposition::Omitted,
            "mesh:unused",
        ),
    ] {
        builder.observe(kind, disposition, identity)?;
    }
    let record = builder.finish()?;
    scope.record_source_scope(record)
}

#[derive(Debug)]
struct TestFormatFailure;

impl fmt::Display for TestFormatFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("failure after source-scope publication")
    }
}

impl std::error::Error for TestFormatFailure {}

impl FormatFailureSource for TestFormatFailure {
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
) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    registry
        .register(ScopeImporter { exit, identities })
        .unwrap();
    let snapshot = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let config = ImportConfigOverlay::default()
        .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
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
    operation_result(import_bytes(
        &request,
        AssetPath::new(SOURCE).unwrap(),
        b"scope",
        ImportOperationControl::default(),
    ))
}

fn provenance_loss_policy() -> LossPolicy {
    LossPolicy::preserve_or_fail().allow(ImportLossKind::ProvenanceEvidenceLoss)
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
    assert_eq!(record.counts(SourceScopeEntityKind::Node).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Node).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Mesh).selected(), 0);
    assert_eq!(record.counts(SourceScopeEntityKind::Mesh).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Material).total(), 0);
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
        &["abc", "def"],
        limits,
        provenance_loss_policy(),
    )
    .unwrap();

    let record = report.source_scope_record().unwrap();
    assert_eq!(record.detail_state(), SourceScopeDetailState::Collapsed);
    assert_eq!(record.counts(SourceScopeEntityKind::Scene).total(), 2);
    assert_eq!(record.details().count(), 0);
    assert_eq!(record.collapsed_detail_count(), 2);
    assert_eq!(record.collapsed_detail_bytes(), 6);
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(
        report.loss_evidence()[0].subject(),
        ImportLossSubject::SourceScopeDetails
    );
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
            0x71, 0xf5, 0x67, 0xb4, 0xa2, 0xf8, 0xb3, 0x3a, 0xec, 0xcf, 0xa4, 0xa2, 0x37, 0xf6,
            0xa2, 0x5c, 0xc6, 0xad, 0x87, 0x3e, 0x08, 0x56, 0x07, 0xd3, 0x70, 0x52, 0xa4, 0x85,
            0xa3, 0x99, 0x95, 0x33,
        ]
    );
}
