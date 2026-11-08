use super::*;
use baozi_import::{
    FormatFailureDetailSink, FormatFailureDetailValue, FormatFailureEnvelope,
    FormatFailureEvidenceProducer, ImportStorageClass, ParseErrorView, ParsePieceError,
};
use std::error::Error;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Exact,
    Incomplete,
    Parse,
    Format,
    CodePanic,
    DetailsPanic,
    DropPanic,
    Denied,
}

#[derive(Default)]
struct Observation {
    actions: AtomicUsize,
    code: AtomicUsize,
    details: AtomicUsize,
    drops: AtomicUsize,
}

struct Producer(Case, Arc<Observation>);
impl FormatFailureEvidenceProducer for Producer {
    fn code(&self) -> &str {
        self.1.code.fetch_add(1, Ordering::SeqCst);
        assert_ne!(self.0, Case::CodePanic, "single-piece format code");
        "test.single_piece"
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        self.1.details.fetch_add(1, Ordering::SeqCst);
        sink.unsigned("line", 7);
        assert_ne!(self.0, Case::DetailsPanic, "single-piece format details");
    }
}
impl Drop for Producer {
    fn drop(&mut self) {
        self.1.drops.fetch_add(1, Ordering::SeqCst);
        assert_ne!(self.0, Case::DropPanic, "single-piece producer destructor");
    }
}

struct DeniedCapture<'session>(ParseErrorView<'session>, Arc<Observation>);
impl Drop for DeniedCapture<'_> {
    fn drop(&mut self) {
        self.1.drops.fetch_add(1, Ordering::SeqCst);
        drop(
            self.0
                .resource_limit_exceeded("single-piece-drop-before-latch", 0),
        );
        panic!("uncalled single-piece capture destructor");
    }
}

fn format_code(failure: &ParseFailure) -> &str {
    failure
        .source()
        .unwrap()
        .downcast_ref::<FormatFailureEnvelope>()
        .unwrap()
        .code()
}

struct PieceImporter {
    base: WorkImporter,
    case: Case,
    cancel: bool,
    observation: Arc<Observation>,
}
impl Format for PieceImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        self.base.compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        self.base.descriptor()
    }

    fn detect(
        &self,
        probe: &baozi_import::Probe<'_>,
        hint: &ReadHint,
        scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        self.base.detect(probe, hint, scope)
    }

    fn parse(&self, _: &Self::Options, context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
        context.with_parser_execution(|root| -> ParseResult<()> {
            if self.case == Case::Denied {
                let layout =
                    root.bind_group([ImportWorkClass::LineBytes, ImportWorkClass::Tokens])?;
                let excess = u64::try_from(root.services()?.limits().max_line_bytes).unwrap() + 1;
                let captured = DeniedCapture(root.error_view(), Arc::clone(&self.observation));
                let caught = catch_unwind(AssertUnwindSafe(|| {
                    root.children()?.run_piece_with_storage::<(), Producer, 2>(
                        &layout,
                        [excess, 1],
                        move |_| {
                            std::hint::black_box(&captured);
                            self.observation.actions.fetch_add(1, Ordering::SeqCst);
                            Ok(((), [excess, 1]))
                        },
                    )
                }));
                assert!(
                    caught.is_err(),
                    "the uncalled capture really panics on disposal"
                );
                assert!(root.children().is_err());
                return Err(root
                    .poll()
                    .expect_err("the admission failure remains terminal"));
            }
            let layout = root.bind_group([ImportWorkClass::Tokens, ImportWorkClass::WorkUnits])?;
            let original = (self.case == Case::Parse)
                .then(|| root.format_failure(Producer(self.case, Arc::clone(&self.observation))));
            let original_code = original
                .as_ref()
                .map(|failure| format_code(failure).as_ptr());
            let baseline = root.work_counters(ImportWorkClass::WorkUnits)?;
            let source = root.services()?.primary_bytes();
            let result = root.children()?.run_piece_with_storage::<_, Producer, 2>(
                &layout,
                [2, 2],
                |mut storage| {
                    self.observation.actions.fetch_add(1, Ordering::SeqCst);
                    let mut values = storage.metered_vec(
                        ImportStorageClass::FormatNormalization,
                        "single-piece-bytes",
                    );
                    storage.push_vec(&mut values, source[0])?;
                    let completed = if self.case == Case::Incomplete { 1 } else { 2 };
                    if completed == 2 {
                        storage.push_vec(&mut values, source[1])?;
                    }
                    assert_eq!(values.as_slice(), &source[..completed]);
                    if self.cancel {
                        self.base.cancellation.as_ref().unwrap().cancel();
                    }
                    match self.case {
                        Case::Parse => Err(ParsePieceError::Parse(original.unwrap())),
                        Case::Format | Case::CodePanic | Case::DetailsPanic | Case::DropPanic => {
                            Err(ParsePieceError::Format(Producer(
                                self.case,
                                Arc::clone(&self.observation),
                            )))
                        }
                        _ => Ok((values, [completed as u64; 2])),
                    }
                },
            );
            if let Some(original_code) = original_code {
                let Err(failure) = &result else {
                    panic!("the original parse error must escape");
                };
                assert_eq!(format_code(failure).as_ptr(), original_code);
            }
            let completed = match self.case {
                Case::Exact => 2,
                Case::Incomplete => 1,
                _ => 0,
            };
            let internal = u64::from(matches!(
                self.case,
                Case::Format | Case::CodePanic | Case::DetailsPanic | Case::DropPanic
            )) * 2;
            let actual = root.work_counters(ImportWorkClass::WorkUnits)?;
            assert_eq!(
                (actual.0 - baseline.0, actual.1 - baseline.1, actual.2),
                (2 + internal, completed + internal, 0),
            );
            assert_eq!(
                root.work_counters(ImportWorkClass::Tokens)?,
                (2, completed, 0)
            );
            if self.case == Case::Incomplete {
                assert!(result.is_err());
                assert!(root.children().is_err());
            }
            let values = result?;
            assert_eq!(values.as_slice(), b"fi");
            Ok(())
        })?;
        support::submit_minimal_scene(context)
    }
}

fn run(
    case: Case,
    cancel: bool,
) -> (
    Result<baozi_import::ImportReport, ImportFailure>,
    Arc<Observation>,
) {
    let observation = Arc::new(Observation::default());
    let token = CancellationToken::new();
    let mut builder = RegistryBuilder::new();
    builder
        .register(PieceImporter {
            base: WorkImporter {
                case: WorkCase::ExactGroup,
                cancellation: Some(token.clone()),
            },
            case,
            cancel,
            observation: Arc::clone(&observation),
        })
        .unwrap();
    let plan = builder
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(&ImportRequest::new(), OptionCompileLimits::default())
        .unwrap();
    (
        operation_result(plan.read_bytes(
            AssetPath::new("asset.work-exact").unwrap(),
            b"fixture",
            ImportOperationControl::default().with_cancellation(token),
        )),
        observation,
    )
}

#[test]
fn exact_piece_returns_real_storage_and_publishes_exact_work() {
    let (report, observation) = run(Case::Exact, false);
    let report = report.unwrap();
    let baseline = run_case(WorkCase::ZeroGroup, None).unwrap();
    for dimension in [ImportDimension::Tokens, ImportDimension::WorkUnits] {
        let actual = report.ledger().dimension(dimension).unwrap();
        let before = baseline.ledger().dimension(dimension).unwrap();
        assert_eq!(
            (
                actual.attempted() - before.attempted(),
                actual.consumed() - before.consumed()
            ),
            (2, 2),
        );
    }
    assert_eq!(observation.actions.load(Ordering::SeqCst), 1);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}

#[test]
fn short_completion_maps_to_descriptor_after_prescribed_exit_control() {
    for (case, cancel, completed) in [
        (Case::Incomplete, false, 1),
        (Case::Incomplete, true, 1),
        (Case::Exact, true, 2),
    ] {
        let (failure, observation) = run(case, cancel);
        let failure = failure.unwrap_err();
        let ImportCause::Scope(abort) = failure.cause() else {
            panic!("single-piece close must retain its scope classification");
        };
        if cancel {
            assert!(matches!(abort.cause(), ScopeAbortCause::Control(_)));
        } else {
            assert!(matches!(
                abort.cause(),
                ScopeAbortCause::DescriptorContract {
                    code: "parse-work-callback-incomplete",
                }
            ));
        }
        assert_eq!(observation.actions.load(Ordering::SeqCst), 1);
        assert_dimension(&failure, ImportDimension::Tokens, 2, completed);
        assert_dimension(&failure, ImportDimension::WorkUnits, 2, completed);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
}

#[test]
fn parse_piece_error_preserves_its_original_owner_across_exit_cancellation() {
    for cancel in [false, true] {
        let (failure, observation) = run(Case::Parse, cancel);
        let failure = failure.unwrap_err();
        if cancel {
            assert!(matches!(failure.cause(), ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::Control(_))));
        } else {
            assert!(matches!(failure.cause(), ImportCause::Format(envelope)
                if envelope.code() == "test.single_piece"));
        }
        assert_eq!(observation.code.load(Ordering::SeqCst), 1);
        assert_eq!(observation.details.load(Ordering::SeqCst), 1);
        assert_eq!(observation.drops.load(Ordering::SeqCst), 1);
        assert_dimension(&failure, ImportDimension::Tokens, 2, 0);
        assert_dimension(&failure, ImportDimension::WorkUnits, 4, 2);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
}

#[test]
fn format_piece_materializes_accounted_evidence_and_contains_producer_panics() {
    for (case, code) in [
        (Case::Format, "test.single_piece"),
        (Case::CodePanic, "format-failure-code-panicked"),
        (Case::DetailsPanic, "format-failure-detail-panicked"),
        (Case::DropPanic, "format-failure-producer-drop-panicked"),
    ] {
        let (failure, observation) = run(case, false);
        let failure = failure.unwrap_err();
        if case == Case::Format {
            let ImportCause::Format(envelope) = failure.cause() else {
                panic!("the real format evidence owner must survive frame close");
            };
            assert_eq!(envelope.code(), code);
            assert_eq!(
                envelope.format().as_str(),
                "dev.baozi.test.work-callback-exact"
            );
            let details = envelope.details();
            assert_eq!(details.len(), 1);
            assert_eq!(
                (details[0].key(), details[0].value()),
                ("line", FormatFailureDetailValue::Unsigned(7))
            );
        } else {
            assert!(matches!(failure.cause(), ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::FormatFailureDetail, code: actual,
                } if *actual == code)));
        }
        assert_eq!(observation.code.load(Ordering::SeqCst), 1);
        assert_eq!(
            observation.details.load(Ordering::SeqCst),
            usize::from(case != Case::CodePanic)
        );
        assert_eq!(observation.drops.load(Ordering::SeqCst), 1);
        assert_dimension(&failure, ImportDimension::Tokens, 2, 0);
        assert_dimension(&failure, ImportDimension::WorkUnits, 4, 2);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
}

#[test]
fn item_limit_latches_before_uncalled_capture_panics_and_rejects_reuse() {
    let (failure, observation) = run(Case::Denied, false);
    let failure = failure.unwrap_err();
    let limit = baozi_import::ResourceLimits::default().max_line_bytes as u64;
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::ResourceLimit {
            resource: "line-bytes", limit: actual,
        } if *actual == limit)));
    assert_eq!(observation.actions.load(Ordering::SeqCst), 0);
    assert_eq!(observation.drops.load(Ordering::SeqCst), 1);
    assert_dimension(&failure, ImportDimension::DecodeWork, limit + 1, 0);
    assert_dimension(&failure, ImportDimension::Tokens, 0, 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}
