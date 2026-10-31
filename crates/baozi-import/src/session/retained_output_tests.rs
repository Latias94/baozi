use super::*;
use crate::authoring::{
    AuthoredTransform, NodeAuthoringNode, NodeLocalId, Presence, StaticSceneBuilder,
};
use crate::{
    CapabilityStatus, DependencyRequirement, DependencyRoleId, DetectionOutcome, DetectionResult,
    DiagnosticCode, DiagnosticSeverity, FormatCapability, FormatDescriptor, FormatEncoding,
    FormatImporter, ImportConfigOverlay, ImportLossDisposition, ImportLossKind, ImportLossSubject,
    ImportRequest, ImportRequestLayer, LossPolicy, OptionCompileLimits, ReadConfidence,
    RegistryBuildLimits, RegistryBuilder, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeSelectionOutcome,
};
use baozi_operation::{
    PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
};
use std::sync::{Arc, Mutex};

struct RetainedOutputImporter {
    required: bool,
}

impl FormatImporter for RetainedOutputImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.retained-output").unwrap(),
            "Retained output lifetime",
        )
        .extensions(["retained"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .dependency_roles([(
            role(),
            if self.required {
                DependencyRequirement::Required
            } else {
                DependencyRequirement::Optional
            },
        )])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> crate::ParseResult<SceneDraft> {
        scope.emit_diagnostic(
            DiagnosticSeverity::Warning,
            DiagnosticCode::new("test.retained_output").unwrap(),
            format_args!("this retained diagnostic keeps its source and message"),
        )?;
        let mut closure = scope.source_scope_builder(
            SourceScopeSelectionOutcome::parentless_node_projection(scope.source().clone()),
        )?;
        closure.observe(
            SourceScopeEntityKind::Node,
            SourceScopeDisposition::Selected,
            "nodes/zero",
        )?;
        scope.record_source_scope(closure.finish()?)?;
        let reference = "textures/retained-and-unavailable-image.bin";
        let dependency = scope.dependency(&scope.root_origin(), &role(), reference)?;
        if dependency.unavailable_evidence().is_some() {
            scope.record_source_semantic_loss(
                DiagnosticCode::new("test.omitted_image").unwrap(),
                SourceScopeEntityKind::Image,
                "/images/0",
                |_| {},
                format_args!("omitted the image whose optional dependency is unavailable"),
            )?;
        }
        let roots = [NodeLocalId::new(0)];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        )];
        scope.submit_static_scene(StaticSceneBuilder::new(Presence::Present(&roots), &nodes))
    }
}

fn role() -> DependencyRoleId {
    DependencyRoleId::new("retained.payload").unwrap()
}

fn request(required: bool) -> PreparedImportRequest {
    request_for(RetainedOutputImporter { required })
}

fn request_for(importer: impl FormatImporter + 'static) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    registry.register(importer).unwrap();
    registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(
                    ImportConfigOverlay::default().with_loss_policy(
                        LossPolicy::preserve_or_fail()
                            .allow(ImportLossKind::OptionalDependencyUnavailable)
                            .allow(ImportLossKind::SourceSemanticLoss),
                    ),
                ),
            ),
            OptionCompileLimits::default(),
        )
        .unwrap()
}

fn run(
    request: &PreparedImportRequest,
) -> (crate::ImportResult<ImportReport>, PhysicalStorageAccount) {
    let (store, account) = start_store(request);
    let authority = DirectBytesAuthority::new();
    let source = AssetPath::new(format!(
        "models/{}scene.retained",
        "long-directory/".repeat(24)
    ))
    .unwrap();
    let mut session = ImportSession::new(
        request,
        &authority,
        source,
        ResolverBinding::custom(&DenyDependencies),
        store,
    );
    let result = match session.execute_bytes(b"immutable-input", SessionCreatedProof::new()) {
        Ok(draft) => session.finalize_success(draft),
        Err(cause) => session.finalize_failure(cause),
    };
    (result, account)
}

fn start_store(
    request: &PreparedImportRequest,
) -> (ImportStoreSessionOwner, PhysicalStorageAccount) {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX).unwrap())
        .unwrap();
    let requirements = ImportStoreBootstrapRequirements::for_node_scene(request.limits()).unwrap();
    let bootstrap = try_import_store_bootstrap(request.limits(), requirements).unwrap();
    let store =
        ImportStoreSessionOwner::try_start(ImportOperationControl::default(), &account, bootstrap)
            .unwrap();
    (store, account)
}

#[test]
fn successful_manifest_and_evidence_keep_their_physical_storage_after_scene_drop() {
    let (result, account) = run(&request(false));
    let report = result.unwrap();
    assert!(!report.optional_dependency_evidence().is_empty());
    assert!(!report.loss_evidence().is_empty());
    assert!(!report.diagnostics().is_empty());
    let loss_diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "test.omitted_image")
        .expect("the actual source-semantic loss must retain its diagnostic");
    assert_eq!(loss_diagnostic.object_path(), Some("/images/0"));
    assert_eq!(report.source_scope_record().unwrap().details().len(), 1);
    let (scene, evidence) = report.into_parts();
    assert_eq!(evidence.ledger().current_live_bytes(), 0);
    assert_eq!(evidence.ledger().active_reservations(), 0);
    let source = evidence.source().as_str();
    assert!(source.len() > 300);
    assert!(
        evidence
            .successful_input_manifest()
            .node_classes()
            .iter()
            .any(|node| node.identity().value().as_str() == source)
    );
    let together = account.snapshot();
    assert!(together.live_retained_bytes() > 0);
    assert!(together.peak_retained_bytes() >= together.live_retained_bytes());
    drop(scene);
    let remaining = account.snapshot();
    assert!(remaining.live_retained_bytes() > 0);
    assert!(remaining.live_retained_bytes() < together.live_retained_bytes());
    let loss = evidence
        .loss_evidence()
        .first()
        .expect("the omitted image must retain its actual source-semantic loss");
    assert_eq!(loss.kind(), ImportLossKind::SourceSemanticLoss);
    assert_eq!(
        loss.subject(),
        ImportLossSubject::SourceEntity(SourceScopeEntityKind::Image)
    );
    assert_eq!(loss.code().as_str(), "test.omitted_image");
    assert_eq!(loss.disposition(), ImportLossDisposition::Applied);
    assert_eq!(loss.source(), evidence.source());
    assert!(
        evidence.successful_input_manifest().node_classes()[0]
            .identity()
            .value()
            .as_str()
            .len()
            > 300
    );
    drop(evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn required_dependency_context_and_other_failure_evidence_release_on_host_unwind() {
    let (result, account) = run(&request(true));
    let failure = result.unwrap_err().into_operation_failure().unwrap();
    let context = failure.cause().dependency_context().unwrap();
    assert_eq!(context.role().as_str(), "retained.payload");
    assert!(context.origin().path().as_str().len() > 300);
    assert!(!failure.diagnostics().is_empty());
    assert!(failure.source_scope_record().is_some());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(account.snapshot().live_retained_bytes() > 0);
    let unwind = catch_unwind(AssertUnwindSafe(move || {
        let _failure = failure;
        panic!("ordinary host unwind while retaining failed import evidence");
    }));
    assert!(unwind.is_err());
    drop(unwind);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn evidence_drop_before_scene_preserves_the_published_generation() {
    let (result, account) = run(&request(false));
    let (scene, evidence) = result.unwrap().into_parts();
    let together = account.snapshot().live_retained_bytes();
    drop(evidence);
    assert!(account.snapshot().live_retained_bytes() > 0);
    assert!(account.snapshot().live_retained_bytes() < together);
    assert_eq!(scene.node_count(), 1);
    assert!(scene.nodes().next().is_some());
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn replacing_an_accounted_error_releases_its_detail_before_terminal_publication() {
    let (store, account) = start_store(&request(true));
    let baseline = account.snapshot();
    let scratch_baseline = store
        .host()
        .dimension_state(ImportDimension::ScratchBytes)
        .unwrap()
        .live_bytes();
    let path = AssetPath::new("models/retained.asset").unwrap();
    let (context, storage) = crate::DependencyFailureContext::try_capture(
        store.host(),
        &AuthorityDomain::new(),
        &path,
        &role(),
        "missing.bin",
        None,
        None,
    )
    .unwrap();
    let mut error = AccountedErrorContext::new(
        ImportCause::Scope(ScopeAbort::with_dependency_context(
            ScopeAbortCause::DescriptorContract {
                code: "test-original-error",
            },
            context,
        )),
        Some(storage),
    );
    assert!(account.snapshot().live_retained_bytes() > baseline.live_retained_bytes());
    assert!(error.cause().dependency_context().is_some());
    error.replace_cause(ImportCause::UnsupportedFormat);
    assert!(matches!(error.cause(), ImportCause::UnsupportedFormat));
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        baseline.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        baseline.live_retained_allocations()
    );
    assert_eq!(
        store
            .host()
            .dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes(),
        scratch_baseline
    );
    drop(error);
    let terminal = store.finish_scene(None).unwrap();
    let evidence = match &terminal {
        ImportStoreTerminalOutcome::Success { evidence, .. }
        | ImportStoreTerminalOutcome::RichFailure { evidence, .. }
        | ImportStoreTerminalOutcome::MinimalFallback { evidence, .. } => evidence,
    };
    assert_eq!(evidence.current_live_bytes(), 0);
    drop(terminal);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn ambiguous_candidate_handle_retains_its_actual_storage_after_terminal_drop() {
    let (store, account) = start_store(&request(false));
    let mut identities = AccountedVec::new();
    identities
        .try_reserve(2, "test-ambiguous-identities", |bytes| {
            reserve_retained_bytes(
                store.host(),
                ImportLedgerPhase::Detection,
                ImportDimension::ScratchBytes,
                bytes,
            )
        })
        .unwrap();
    identities
        .try_push_reserved(FormatId::new("dev.baozi.test.first").unwrap())
        .unwrap();
    identities
        .try_push_reserved(FormatId::new("dev.baozi.test.second").unwrap())
        .unwrap();
    let identity_pointer = identities.as_ptr();
    let candidates = crate::error::DetectionCandidates::try_new(identities, store.host()).unwrap();
    assert_eq!(candidates.as_slice().as_ptr(), identity_pointer);
    drop(store.finish_scene(None).unwrap());
    assert_eq!(account.snapshot().live_retained_allocations(), 2);
    assert_eq!(candidates[0].as_str(), "dev.baozi.test.first");
    assert_eq!(candidates[1].as_str(), "dev.baozi.test.second");
    assert!(!account.snapshot().poisoned());
    drop(candidates);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

struct RetainedFormatFailure;

impl crate::FormatFailureEvidenceProducer for RetainedFormatFailure {
    fn code(&self) -> &str {
        "test.retained_format_failure"
    }

    fn visit_details(&self, sink: &mut dyn crate::FormatFailureDetailSink) {
        sink.string("message", "retained beyond the import session");
    }
}

struct EscapingFailureImporter {
    escaped: Arc<Mutex<Option<crate::ParseFailure>>>,
}

impl FormatImporter for EscapingFailureImporter {
    fn descriptor(&self) -> FormatDescriptor {
        RetainedOutputImporter { required: false }.descriptor()
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> crate::ParseResult<SceneDraft> {
        let failure = scope.format_failure(RetainedFormatFailure);
        *self.escaped.lock().unwrap() = Some(failure);
        Err(scope.allocation_failure("test-escaping-format-failure"))
    }
}

#[test]
fn format_failure_retained_by_an_extension_keeps_its_lease_after_session_drop() {
    let escaped = Arc::new(Mutex::new(None));
    let request = request_for(EscapingFailureImporter {
        escaped: Arc::clone(&escaped),
    });
    let (result, account) = run(&request);
    let failure = result.unwrap_err().into_operation_failure().unwrap();
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
    drop(failure);
    let retained = account.snapshot();
    assert!(retained.live_retained_bytes() > 0);
    assert_eq!(retained.live_retained_allocations(), 3);
    assert!(!retained.poisoned());
    let escaped = escaped.lock().unwrap().take().unwrap();
    let ParseFailureKind::Format(envelope) = escaped.into_kind() else {
        panic!("the escaped transport must retain its original format evidence");
    };
    assert_eq!(envelope.code(), "test.retained_format_failure");
    assert_eq!(envelope.details().len(), 1);
    assert!(envelope.has_storage());
    drop(envelope);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}
