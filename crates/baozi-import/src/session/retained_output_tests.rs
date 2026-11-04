use super::*;
use crate::authoring::{
    AuthoredTransform, NodeAuthoringNode, NodeLocalId, Presence, StaticSceneBuilder,
};
use crate::{
    CapabilityStatus, CompiledOptions, DependencyRequirement, DependencyRoleId, DetectionOutcome,
    DetectionResult, DiagnosticCode, DiagnosticSeverity, Format, FormatCapability,
    FormatDescriptor, FormatEncoding, ImportConfigOverlay, ImportLossDisposition, ImportLossKind,
    ImportLossSubject, ImportRequest, ImportRequestLayer, LossPolicy, NoFormatOptions,
    OptionCompileError, OptionCompileLimits, ReadConfidence, RegistryBuildLimits,
    RegistryBuildScope, RegistryBuilder, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeSelectionOutcome, SourceScopeSelectionRequest,
};
use baozi_operation::{
    CancellationOrigin, CancellationToken, PhysicalStorageAccount, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits, ProgressDecision,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct RetainedOutputImporter {
    required: bool,
}

impl Format for RetainedOutputImporter {
    type Options = NoFormatOptions;

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

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut ParseContext<'_, '_>,
    ) -> crate::ParseResult<()> {
        context.with_execution(|exec| -> crate::ParseResult<()> {
            exec.services()?.emit_diagnostic(
                DiagnosticSeverity::Warning,
                DiagnosticCode::new("test.retained_output").unwrap(),
                format_args!("this retained diagnostic keeps its source and message"),
            )?;
            let source = exec.source().clone();
            let mut closure = exec.services()?.source_scope_builder(
                SourceScopeSelectionRequest::DocumentDefault,
                SourceScopeSelectionOutcome::parentless_node_projection(source),
            )?;
            closure.observe(
                &mut exec.services()?,
                SourceScopeEntityKind::Node,
                SourceScopeDisposition::Selected,
                "nodes/zero",
            )?;
            let finalized = closure.finish(&mut exec.services()?)?;
            exec.services()?.record_source_scope(finalized)?;
            let reference = "textures/retained-and-unavailable-image.bin";
            let origin = exec.services()?.root_origin();
            let dependency = exec.dependency(&origin, &role(), reference)?;
            if dependency.unavailable_evidence().is_some() {
                exec.services()?.record_source_semantic_loss(
                    DiagnosticCode::new("test.omitted_image").unwrap(),
                    SourceScopeEntityKind::Image,
                    "/images/0",
                    |_| {},
                    format_args!("omitted the image whose optional dependency is unavailable"),
                )?;
            }
            Ok(())
        })?;
        let roots = [NodeLocalId::new(0)];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        )];
        context.submit(StaticSceneBuilder::new(Presence::Present(&roots), &nodes))
    }
}

fn role() -> DependencyRoleId {
    DependencyRoleId::new("retained.payload").unwrap()
}

fn request(required: bool) -> ImportPlan {
    request_for(RetainedOutputImporter { required })
}

fn request_for(importer: impl Format + 'static) -> ImportPlan {
    let mut registry = RegistryBuilder::new();
    registry.register(importer).unwrap();
    registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(
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

fn run(request: &ImportPlan) -> (crate::ImportResult<ImportReport>, PhysicalStorageAccount) {
    run_with_control(request, ImportOperationControl::default())
}

fn run_with_control(
    request: &ImportPlan,
    control: ImportOperationControl,
) -> (crate::ImportResult<ImportReport>, PhysicalStorageAccount) {
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let authority = DirectBytesAuthority::new();
    let source = AssetPath::new(format!(
        "models/{}scene.retained",
        "long-directory/".repeat(24)
    ))
    .unwrap();
    let mut data = ImportSessionData::new(request, source);
    let mut control = Some(control);
    let mut finish_request = None;
    let mut account = None;
    let finish = invocation_root
        .run_body(|parts, disposition| {
            let (store, started_account) =
                start_store_with_control(parts, request, control.take().unwrap());
            let mut session = ImportSession::new(
                request,
                &authority,
                ResolverBinding::custom(&DenyDependencies),
                &mut data,
                store,
            );
            let execution = session.execute_bytes(b"immutable-input", SessionCreatedProof::new());
            let (store, outcome) = session.stage(execution);
            finish_request = Some(store.request_finish(disposition, outcome).unwrap());
            account = Some(started_account);
        })
        .unwrap();
    let result = data.finish(finish, finish_request.unwrap());
    (result, account.unwrap())
}

fn start_store<'inv>(
    parts: ImportStoreBodyParts<'inv>,
    request: &ImportPlan,
) -> (ImportStoreSessionOwner<'inv>, PhysicalStorageAccount) {
    start_store_with_control(parts, request, ImportOperationControl::default())
}

fn start_store_with_control<'inv>(
    parts: ImportStoreBodyParts<'inv>,
    request: &ImportPlan,
    control: ImportOperationControl,
) -> (ImportStoreSessionOwner<'inv>, PhysicalStorageAccount) {
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
        ImportStoreSessionOwner::try_start_in_body(parts, control, &account, bootstrap).unwrap();
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
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let request = request(true);
    let mut finish_request = None;
    let mut retained_account = None;
    let finish = invocation_root
        .run_body(|parts, disposition| {
            let (mut store, started_account) = start_store(parts, &request);
            let account = &started_account;
            let baseline = account.snapshot();
            let scratch_baseline = store
                .with_root_execution(|exec| exec.dimension_state(ImportDimension::ScratchBytes))
                .unwrap()
                .live_bytes();
            let path = AssetPath::new("models/retained.asset").unwrap();
            let (context, storage) = store
                .with_root_execution(|exec| {
                    crate::DependencyFailureContext::try_capture(
                        exec,
                        &AuthorityDomain::new(),
                        &path,
                        &role(),
                        "missing.bin",
                        None,
                        None,
                    )
                })
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
                    .with_root_execution(|exec| exec.dimension_state(ImportDimension::ScratchBytes))
                    .unwrap()
                    .live_bytes(),
                scratch_baseline
            );
            drop(error);
            finish_request = Some(
                store
                    .request_finish(disposition, SceneParseOutcome::Rejected)
                    .unwrap(),
            );
            retained_account = Some(started_account);
        })
        .unwrap();
    let account = retained_account.unwrap();
    let terminal = finish_scene(finish, finish_request.unwrap()).unwrap();
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
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let request = request(false);
    let mut finish_request = None;
    let mut account = None;
    let mut retained_candidates = None;
    let finish = invocation_root
        .run_body(|parts, disposition| {
            let (mut store, started_account) = start_store(parts, &request);
            let mut identities = AccountedVec::new();
            identities
                .try_reserve(2, "test-ambiguous-identities", |bytes| {
                    store.with_root_execution(|exec| {
                        reserve_retained_bytes(
                            exec,
                            ImportLedgerPhase::Detection,
                            ImportDimension::ScratchBytes,
                            bytes,
                        )
                    })
                })
                .unwrap();
            identities
                .try_push_reserved(FormatId::new("dev.baozi.test.first").unwrap())
                .unwrap();
            identities
                .try_push_reserved(FormatId::new("dev.baozi.test.second").unwrap())
                .unwrap();
            let identity_pointer = identities.as_ptr();
            let candidates = store
                .with_root_execution(|exec| {
                    crate::error::DetectionCandidates::try_new(identities, exec)
                })
                .unwrap();
            assert_eq!(candidates.as_slice().as_ptr(), identity_pointer);
            retained_candidates = Some(candidates);
            finish_request = Some(
                store
                    .request_finish(disposition, SceneParseOutcome::Rejected)
                    .unwrap(),
            );
            account = Some(started_account);
        })
        .unwrap();
    let account = account.unwrap();
    let candidates = retained_candidates.unwrap();
    drop(finish_scene(finish, finish_request.unwrap()).unwrap());
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

impl Format for EscapingFailureImporter {
    type Options = NoFormatOptions;

    fn descriptor(&self) -> FormatDescriptor {
        RetainedOutputImporter { required: false }.descriptor()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut ParseContext<'_, '_>,
    ) -> crate::ParseResult<()> {
        context.with_execution(|exec| {
            let failure = exec.format_failure(RetainedFormatFailure);
            *self.escaped.lock().unwrap() = Some(failure);
            Err(exec.allocation_failure("test-escaping-format-failure"))
        })
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

#[derive(Clone, Copy)]
enum ParserCompletion {
    WithoutSubmission,
    FailureAfterSubmission,
    PanicAfterSubmission,
    SwallowedFailure,
}

impl Format for ParserCompletion {
    type Options = NoFormatOptions;

    fn descriptor(&self) -> FormatDescriptor {
        RetainedOutputImporter { required: false }.descriptor()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut ParseContext<'_, '_>,
    ) -> crate::ParseResult<()> {
        if matches!(self, Self::WithoutSubmission) {
            return Ok(());
        }
        let failure = if matches!(self, Self::FailureAfterSubmission | Self::SwallowedFailure) {
            Some(context.with_execution(|exec| -> crate::ParseResult<_> {
                Ok(exec.format_failure(RetainedFormatFailure))
            })?)
        } else {
            None
        };
        let roots = [NodeLocalId::new(0)];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        )];
        context.submit(StaticSceneBuilder::new(Presence::Present(&roots), &nodes))?;
        match self {
            Self::FailureAfterSubmission => Err(failure.unwrap()),
            Self::PanicAfterSubmission => panic!("parser panic after a validated candidate"),
            Self::SwallowedFailure => {
                drop(failure);
                Ok(())
            }
            Self::WithoutSubmission => unreachable!("the missing-submission path returned above"),
        }
    }
}

#[test]
fn parser_acceptance_requires_submission_and_preserves_rejection_after_a_candidate() {
    for completion in [
        ParserCompletion::WithoutSubmission,
        ParserCompletion::FailureAfterSubmission,
        ParserCompletion::PanicAfterSubmission,
        ParserCompletion::SwallowedFailure,
    ] {
        let (result, account) = run(&request_for(completion));
        let failure = result.unwrap_err().into_operation_failure().unwrap();
        match completion {
            ParserCompletion::WithoutSubmission => {
                assert_eq!(
                    failure.last_completed().phase(),
                    ImportPhase::SelectionCompleted
                );
                assert!(matches!(failure.cause(), ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::DescriptorContract {
                    code: "parse-success-without-validated-scene",
                })));
            }
            ParserCompletion::FailureAfterSubmission => {
                assert_eq!(failure.last_completed().phase(), ImportPhase::Validated);
                assert!(matches!(failure.cause(), ImportCause::Format(envelope)
                    if envelope.code() == "test.retained_format_failure" && envelope.has_storage()));
            }
            ParserCompletion::PanicAfterSubmission => {
                assert_eq!(failure.last_completed().phase(), ImportPhase::Validated);
                assert!(matches!(
                    failure.cause(),
                    ImportCause::TrustedExtension {
                        boundary: TrustedExtensionBoundary::Importer,
                        code: "importer-panicked",
                    }
                ));
            }
            ParserCompletion::SwallowedFailure => {
                assert_eq!(failure.last_completed().phase(), ImportPhase::Validated);
                assert!(matches!(failure.cause(), ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::DescriptorContract {
                    code: "format-failure-was-swallowed",
                })));
            }
        }
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
        drop(failure);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert!(!account.snapshot().poisoned());
    }
}

#[test]
fn candidate_format_failure_yields_to_freeze_first_control() {
    #[derive(Clone, Copy, Debug)]
    enum FinalControl {
        None,
        Observer,
        Signal,
    }

    for mode in [
        FinalControl::None,
        FinalControl::Observer,
        FinalControl::Signal,
    ] {
        let cancellation = CancellationToken::new();
        let cancellation_from_observer = cancellation.clone();
        let final_events = Arc::new(AtomicUsize::new(0));
        let observed_final_events = Arc::clone(&final_events);
        let control = ImportOperationControl::default()
            .with_cancellation(cancellation)
            .with_progress_observer(
                move |event: baozi_operation::ProgressEvent<
                    ImportProgressPhase,
                    ImportProgressUnit,
                >| {
                    if event.phase() == ImportProgressPhase::Finalize {
                        observed_final_events.fetch_add(1, Ordering::AcqRel);
                        match mode {
                            FinalControl::None => {}
                            FinalControl::Observer => return ProgressDecision::Cancel,
                            FinalControl::Signal => cancellation_from_observer.cancel(),
                        }
                    }
                    ProgressDecision::Continue
                },
            );
        let (result, account) = run_with_control(
            &request_for(ParserCompletion::FailureAfterSubmission),
            control,
        );
        let failure = result.unwrap_err().into_operation_failure().unwrap();

        assert_eq!(final_events.load(Ordering::Acquire), 1, "{mode:?}");
        assert_eq!(failure.last_completed().phase(), ImportPhase::Validated);
        assert!(failure.scene_validation_error().is_none());
        assert!(failure.terminal_control_cause().is_none());
        match mode {
            FinalControl::None => {
                let ImportCause::Format(envelope) = failure.cause() else {
                    panic!("a ready rejected candidate must keep its original format failure");
                };
                assert_eq!(envelope.code(), "test.retained_format_failure");
                assert!(envelope.has_storage());
                assert!(matches!(
                    failure.cause().owner(),
                    crate::CauseOwner::Format(_)
                ));
            }
            FinalControl::Observer | FinalControl::Signal => {
                let ImportCause::Scope(abort) = failure.cause() else {
                    panic!("freeze-first control must override the returned format failure");
                };
                let expected = match mode {
                    FinalControl::Observer => CancellationOrigin::Observer,
                    FinalControl::Signal => CancellationOrigin::Signal,
                    FinalControl::None => unreachable!(),
                };
                assert!(matches!(
                    abort.cause(),
                    ScopeAbortCause::Control(ControlCause::Cancelled { origin })
                        if *origin == expected
                ));
                assert_eq!(failure.cause().owner(), crate::CauseOwner::Operation);
                assert!(matches!(
                    failure.ledger().terminal_cause(),
                    ImportTerminalCause::Failure(terminal)
                        if terminal.class() == crate::ImportTerminalFailureClass::Control
                ));
            }
        }
        let output = failure
            .ledger()
            .dimension(ImportDimension::CanonicalOutputBytes)
            .unwrap();
        assert_eq!(output.committed(), 0);
        assert_eq!(failure.ledger().committed_canonical_members(), 0);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
        drop(failure);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0, "{mode:?}");
        assert_eq!(released.live_retained_allocations(), 0, "{mode:?}");
        assert!(!released.poisoned(), "{mode:?}");
    }
}
