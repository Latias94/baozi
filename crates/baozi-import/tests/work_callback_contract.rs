mod support;

use baozi_core::FormatId;
use baozi_import::{
    CapabilityStatus, CauseOwner, CompiledOptions, DetectionOutcome, DetectionResult,
    DetectionScope, Format, FormatCapability, FormatDescriptor, FormatEncoding, ImportCause,
    ImportDimension, ImportFailure, ImportOperationControl, ImportPlan, ImportRequest,
    ImportResult, ImportWorkClass, NoFormatOptions, OptionCompileError, OptionCompileLimits,
    ParseContext, ParseFailure, ParseResult, ParseScanReturn, ReadConfidence, ReadHint,
    RegistryBuildLimits, RegistryBuildScope, RegistryBuilder, ScopeAbortCause,
    TrustedExtensionBoundary,
};
use baozi_io::AssetPath;
use baozi_operation::CancellationToken;
use baozi_operation::LedgerCause;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[allow(
    clippy::result_large_err,
    reason = "work callback assertions inspect complete bounded import failure evidence"
)]
fn operation_result<T>(result: ImportResult<T>) -> Result<T, ImportFailure> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match error.into_operation_failure() {
            Ok(failure) => Err(failure),
            Err(source) => {
                panic!("work callback test setup failed before session creation: {source}")
            }
        },
    }
}

#[derive(Clone, Copy)]
enum WorkCase {
    EntryFailure,
    ExactGroup,
    ZeroGroup,
    DynamicChunks,
    CallbackError,
    ReturnedOwnedError,
    ScanBoundary,
    ScanIncomplete,
    ScanBoundaryCancel,
    Incomplete,
    ParentNestedSuccess,
    ParentChildFailure,
    ParentGroupNestedSuccess,
    ParentGroupChildFailure,
    ParentCancelBeforeCompletion,
    PrematureReplenish,
    ItemLimitedReplenish,
    GroupItemLimit,
    CallbackPanic,
    DuplicateDimension,
    CallbackErrorCancel,
    IncompleteCancel,
    CancelAfterCompletion,
}

struct WorkImporter {
    case: WorkCase,
    cancellation: Option<CancellationToken>,
}

impl WorkImporter {
    fn id(&self) -> &'static str {
        match self.case {
            WorkCase::EntryFailure => "dev.baozi.test.work-callback-entry",
            WorkCase::ExactGroup => "dev.baozi.test.work-callback-exact",
            WorkCase::ZeroGroup => "dev.baozi.test.work-callback-zero",
            WorkCase::DynamicChunks => "dev.baozi.test.work-callback-dynamic",
            WorkCase::CallbackError => "dev.baozi.test.work-callback-error",
            WorkCase::ReturnedOwnedError => "dev.baozi.test.work-returned-owned-error",
            WorkCase::ScanBoundary => "dev.baozi.test.scan-boundary",
            WorkCase::ScanIncomplete => "dev.baozi.test.scan-incomplete",
            WorkCase::ScanBoundaryCancel => "dev.baozi.test.scan-boundary-cancel",
            WorkCase::Incomplete => "dev.baozi.test.work-callback-incomplete",
            WorkCase::ParentNestedSuccess => "dev.baozi.test.work-parent-success",
            WorkCase::ParentChildFailure => "dev.baozi.test.work-parent-child-failure",
            WorkCase::ParentGroupNestedSuccess => "dev.baozi.test.work-parent-group-success",
            WorkCase::ParentGroupChildFailure => "dev.baozi.test.work-parent-group-child-failure",
            WorkCase::ParentCancelBeforeCompletion => "dev.baozi.test.work-parent-cancel",
            WorkCase::PrematureReplenish => "dev.baozi.test.work-callback-replenish",
            WorkCase::ItemLimitedReplenish => "dev.baozi.test.work-callback-item-limit",
            WorkCase::GroupItemLimit => "dev.baozi.test.work-callback-group-limit",
            WorkCase::CallbackPanic => "dev.baozi.test.work-callback-panic",
            WorkCase::DuplicateDimension => "dev.baozi.test.work-callback-duplicate",
            WorkCase::CallbackErrorCancel => "dev.baozi.test.work-callback-error-cancel",
            WorkCase::IncompleteCancel => "dev.baozi.test.work-callback-incomplete-cancel",
            WorkCase::CancelAfterCompletion => "dev.baozi.test.work-callback-cancel",
        }
    }

    fn extension(&self) -> &'static str {
        match self.case {
            WorkCase::EntryFailure => "work-entry",
            WorkCase::ExactGroup => "work-exact",
            WorkCase::ZeroGroup => "work-zero",
            WorkCase::DynamicChunks => "work-dynamic",
            WorkCase::CallbackError => "work-error",
            WorkCase::ReturnedOwnedError => "work-returned-owned-error",
            WorkCase::ScanBoundary => "scan-boundary",
            WorkCase::ScanIncomplete => "scan-incomplete",
            WorkCase::ScanBoundaryCancel => "scan-boundary-cancel",
            WorkCase::Incomplete => "work-incomplete",
            WorkCase::ParentNestedSuccess => "work-parent-success",
            WorkCase::ParentChildFailure => "work-parent-failure",
            WorkCase::ParentGroupNestedSuccess => "work-parent-group-success",
            WorkCase::ParentGroupChildFailure => "work-parent-group-failure",
            WorkCase::ParentCancelBeforeCompletion => "work-parent-cancel",
            WorkCase::PrematureReplenish => "work-replenish",
            WorkCase::ItemLimitedReplenish => "work-item-limit",
            WorkCase::GroupItemLimit => "work-group-limit",
            WorkCase::CallbackPanic => "work-panic",
            WorkCase::DuplicateDimension => "work-duplicate",
            WorkCase::CallbackErrorCancel => "work-error-cancel",
            WorkCase::IncompleteCancel => "work-incomplete-cancel",
            WorkCase::CancelAfterCompletion => "work-cancel",
        }
    }
}

impl Format for WorkImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new(self.id()).expect("qualified work callback format id"),
            "Work Callback Contract",
        )
        .extensions([self.extension()])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .expect("valid work callback descriptor")
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
        context.with_execution(|root| -> ParseResult<()> {
            let work_units = root.bind_group([ImportWorkClass::WorkUnits])?;
            let tokens = root.bind_group([ImportWorkClass::Tokens])?;
            let records = root.bind_group([ImportWorkClass::Records])?;
            let string_bytes = root.bind_group([ImportWorkClass::StringBytes])?;
            let tokens_work =
                root.bind_group([ImportWorkClass::Tokens, ImportWorkClass::WorkUnits])?;
            let work_tokens =
                root.bind_group([ImportWorkClass::WorkUnits, ImportWorkClass::Tokens])?;
            let record_attributes =
                root.bind_group([ImportWorkClass::Records, ImportWorkClass::AttributeCells])?;
            let line_tokens =
                root.bind_group([ImportWorkClass::LineBytes, ImportWorkClass::Tokens])?;
            let duplicate_work =
                root.bind_group([ImportWorkClass::WorkUnits, ImportWorkClass::WorkUnits])?;
            let source = root.services()?.primary_bytes();
            match self.case {
                WorkCase::EntryFailure => {
                    return Err(root
                        .services()?
                        .resource_limit_exceeded("work-callback-entry", 0));
                }
                WorkCase::ExactGroup => {
                    let mut decoded = [0; 3];
                    root.child(&tokens_work, [3, 2], |work| {
                        assert_eq!(work.remaining(), [3, 2]);
                        work.run_returned(|| {
                            decoded[0] = source[0];
                            Ok(((), [1, 1]))
                        })?;
                        work.run([2, 1], || {
                            decoded[1..].copy_from_slice(&source[1..3]);
                            Ok::<_, ParseFailure>(())
                        })?;
                        assert_eq!(work.remaining(), [0, 0]);
                        Ok::<_, ParseFailure>(())
                    })?;
                    assert_eq!(&decoded, b"fix");
                }
                WorkCase::ZeroGroup => {
                    root.child(&work_tokens, [0, 0], |work| {
                        assert_eq!(work.remaining(), [0, 0]);
                        Ok::<_, ParseFailure>(())
                    })?;
                }
                WorkCase::DynamicChunks => {
                    let mut decoded = [[0; 4]; 2];
                    root.child(&work_units, [0], |work| {
                        for chunk in &mut decoded {
                            work.replenish(4)?;
                            work.run([4], || {
                                chunk.copy_from_slice(&source[..4]);
                                Ok::<_, ParseFailure>(())
                            })?;
                        }
                        Ok::<_, ParseFailure>(())
                    })?;
                    assert_eq!(decoded, [*b"fixt"; 2]);
                }
                WorkCase::ScanBoundary
                | WorkCase::ScanIncomplete
                | WorkCase::ScanBoundaryCancel => {
                    let mut decoded = [0; 3];
                    let result = root.scan(&tokens, [3], |scan| {
                        scan.run([2], || {
                            decoded[..2].copy_from_slice(&source[..2]);
                            Ok::<_, ParseFailure>(())
                        })?;
                        if matches!(self.case, WorkCase::ScanBoundaryCancel) {
                            self.cancellation
                                .as_ref()
                                .expect("the boundary cancellation case owns a token")
                                .cancel();
                        }
                        Ok::<_, ParseFailure>(if matches!(self.case, WorkCase::ScanIncomplete) {
                            ParseScanReturn::Exhausted(2)
                        } else {
                            ParseScanReturn::Boundary(2)
                        })
                    });
                    assert_eq!(decoded, [b'f', b'i', 0]);
                    match result {
                        Ok(offset) => {
                            assert_eq!(offset, 2);
                            root.child(&tokens, [1], |work| {
                                work.run([1], || {
                                    decoded[2] = source[offset];
                                    Ok::<_, ParseFailure>(())
                                })
                            })?;
                            assert_eq!(&decoded, b"fix");
                        }
                        Err(cause) => {
                            let mut following = false;
                            assert!(
                                root.child(&tokens, [1], |_| {
                                    following = true;
                                    Ok::<_, ParseFailure>(())
                                })
                                .is_err()
                            );
                            assert!(!following);
                            return Err(cause);
                        }
                    }
                }
                WorkCase::CallbackError => {
                    let mut decoded = [0; 2];
                    root.child(&work_tokens, [2, 2], |work| {
                        work.run([1, 1], || {
                            decoded[0] = source[0];
                            Ok::<_, ParseFailure>(())
                        })?;
                        assert_eq!(decoded, [b'f', 0]);
                        work.services()?
                            .enforce_resource_limit("work-callback-error", 2, 1)?;
                        Ok::<_, ParseFailure>(())
                    })?;
                }
                WorkCase::Incomplete => {
                    let mut decoded = [0; 2];
                    root.child(&work_tokens, [2, 2], |work| {
                        work.run([1, 1], || {
                            decoded[0] = source[0];
                            Ok::<_, ParseFailure>(())
                        })?;
                        assert_eq!(decoded, [b'f', 0]);
                        Ok::<_, ParseFailure>(())
                    })?;
                }
                WorkCase::ParentNestedSuccess => {
                    let mut decoded = [0; 2];
                    root.parent(&records, [1], |parent| {
                        parent.children()?.child(&tokens, [2], |child| {
                            child.run([2], || {
                                decoded.copy_from_slice(&source[..2]);
                                Ok::<_, ParseFailure>(())
                            })
                        })
                    })?;
                    assert_eq!(&decoded, b"fi");
                }
                WorkCase::ParentChildFailure => {
                    let mut decoded = [0; 2];
                    root.parent(&records, [1], |parent| {
                        parent.children()?.child(&work_units, [2], |child| {
                            child.run([1], || {
                                decoded[0] = source[0];
                                Ok::<_, ParseFailure>(())
                            })?;
                            assert_eq!(decoded, [b'f', 0]);
                            child
                                .services()?
                                .enforce_resource_limit("parent-child-failure", 2, 1)
                        })
                    })?;
                }
                WorkCase::ParentGroupNestedSuccess => {
                    let mut decoded = [0; 3];
                    root.parent(&record_attributes, [1, 2], |parent| {
                        parent.children()?.child(&tokens, [3], |child| {
                            child.run([3], || {
                                decoded.copy_from_slice(&source[..3]);
                                Ok::<_, ParseFailure>(())
                            })
                        })
                    })?;
                    assert_eq!(&decoded, b"fix");
                }
                WorkCase::ParentGroupChildFailure => {
                    let mut decoded = [0; 2];
                    root.parent(&record_attributes, [1, 2], |parent| {
                        parent.children()?.child(&work_units, [2], |child| {
                            child.run([1], || {
                                decoded[0] = source[0];
                                Ok::<_, ParseFailure>(())
                            })?;
                            assert_eq!(decoded, [b'f', 0]);
                            child.services()?.enforce_resource_limit(
                                "parent-group-child-failure",
                                2,
                                1,
                            )
                        })
                    })?;
                }
                WorkCase::ParentCancelBeforeCompletion => {
                    let cancellation = self
                        .cancellation
                        .as_ref()
                        .expect("parent cancellation case owns a token");
                    root.parent(&records, [1], |_parent| {
                        cancellation.cancel();
                        Ok::<_, ParseFailure>(())
                    })?;
                }
                WorkCase::PrematureReplenish => {
                    root.child(&work_units, [2], |work| {
                        work.replenish(1)?;
                        Ok::<_, ParseFailure>(())
                    })?;
                }
                WorkCase::ItemLimitedReplenish => {
                    root.child(&string_bytes, [0], |work| {
                        work.replenish(1)?;
                        Ok::<_, ParseFailure>(())
                    })?;
                }
                WorkCase::GroupItemLimit => {
                    let line_bytes = u64::try_from(root.services()?.limits().max_line_bytes)
                        .expect("line-byte limit fits the import counter")
                        .checked_add(1)
                        .expect("test line-byte excess fits the import counter");
                    root.child(&line_tokens, [line_bytes, 1], |_work| -> ParseResult<()> {
                        panic!("an item-limit failure cannot enter the physical callback")
                    })?;
                }
                WorkCase::CallbackPanic => {
                    let mut decoded = [0; 2];
                    root.child(&work_tokens, [2, 2], |work| -> ParseResult<()> {
                        work.run([1, 1], || {
                            decoded[0] = source[0];
                            Ok::<_, ParseFailure>(())
                        })?;
                        assert_eq!(decoded, [b'f', 0]);
                        panic!("work callback panic")
                    })?;
                }
                WorkCase::DuplicateDimension => {
                    let mut decoded = [0; 5];
                    root.child(&duplicate_work, [2, 3], |work| {
                        work.run_returned(|| {
                            decoded[..3].copy_from_slice(&source[..3]);
                            Ok(((), [1, 2]))
                        })?;
                        work.run([1, 1], || {
                            decoded[3..].copy_from_slice(&source[3..5]);
                            Ok::<_, ParseFailure>(())
                        })
                    })?;
                    assert_eq!(&decoded, b"fixtu");
                }
                WorkCase::CallbackErrorCancel => {
                    enum CallbackError {
                        Parse,
                        Requested,
                    }

                    impl From<ParseFailure> for CallbackError {
                        fn from(_value: ParseFailure) -> Self {
                            Self::Parse
                        }
                    }

                    let cancellation = self
                        .cancellation
                        .as_ref()
                        .expect("callback error cancellation case owns a token");
                    let mut decoded = [0; 2];
                    let result =
                        root.child(&work_units, [2], |work| -> Result<(), CallbackError> {
                            work.run([1], || {
                                decoded[0] = source[0];
                                Ok::<_, ParseFailure>(())
                            })?;
                            cancellation.cancel();
                            Err(CallbackError::Requested)
                        });
                    assert_eq!(decoded, [b'f', 0]);
                    assert!(matches!(result, Err(CallbackError::Requested)));
                    root.poll()?;
                }
                WorkCase::ReturnedOwnedError => {
                    enum PieceError {
                        Parse,
                        Owned(std::rc::Rc<()>),
                    }
                    impl From<ParseFailure> for PieceError {
                        fn from(_: ParseFailure) -> Self {
                            Self::Parse
                        }
                    }
                    let marker = std::rc::Rc::new(());
                    let mut decoded = [0; 2];
                    let result = root.child(&work_units, [2], |work| {
                        work.run([1], || {
                            decoded[0] = source[0];
                            Ok::<_, PieceError>(())
                        })?;
                        work.run_returned(|| {
                            Err::<((), [u64; 1]), _>(PieceError::Owned(std::rc::Rc::clone(&marker)))
                        })
                    });
                    let result = root.poll_result(result);
                    let Err(PieceError::Owned(returned)) = result else {
                        panic!("the physical callback's owned error must escape unchanged")
                    };
                    assert!(std::rc::Rc::ptr_eq(&marker, &returned));
                    assert_eq!(decoded, [b'f', 0]);
                    let mut following = false;
                    let failure = root
                        .child(&work_units, [1], |work| {
                            work.run([1], || {
                                following = true;
                                Ok::<_, ParseFailure>(())
                            })
                        })
                        .expect_err(
                            "the owned piece error must stop numeric work before any abort",
                        );
                    assert!(!following);
                    return Err(failure);
                }
                WorkCase::IncompleteCancel => {
                    let cancellation = self
                        .cancellation
                        .as_ref()
                        .expect("incomplete cancellation case owns a token");
                    let mut decoded = [0; 2];
                    root.child(&work_units, [2], |work| {
                        work.run([1], || {
                            decoded[0] = source[0];
                            Ok::<_, ParseFailure>(())
                        })?;
                        assert_eq!(decoded, [b'f', 0]);
                        cancellation.cancel();
                        Ok::<_, ParseFailure>(())
                    })?;
                }
                WorkCase::CancelAfterCompletion => {
                    let cancellation = self
                        .cancellation
                        .as_ref()
                        .expect("cancellation case owns a token");
                    let mut decoded = [0; 2];
                    root.child(&work_units, [2], |work| {
                        work.run([2], || {
                            decoded.copy_from_slice(&source[..2]);
                            Ok::<_, ParseFailure>(())
                        })?;
                        assert_eq!(&decoded, b"fi");
                        cancellation.cancel();
                        Ok::<_, ParseFailure>(())
                    })?;
                }
            }
            Ok(())
        })?;
        support::submit_minimal_scene(context)
    }
}

fn request_for(importer: WorkImporter) -> ImportPlan {
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer)
        .expect("work callback importer registers");
    let registry = builder
        .freeze(RegistryBuildLimits::default())
        .expect("work callback registry freezes");
    registry
        .plan(&ImportRequest::new(), OptionCompileLimits::default())
        .expect("work callback request compiles")
}

#[allow(
    clippy::result_large_err,
    reason = "work callback assertions inspect complete bounded import failure evidence"
)]
fn run_case(
    case: WorkCase,
    cancellation: Option<CancellationToken>,
) -> Result<baozi_import::ImportReport, ImportFailure> {
    let extension = WorkImporter {
        case,
        cancellation: cancellation.clone(),
    }
    .extension();
    let request = request_for(WorkImporter { case, cancellation });
    operation_result(request.read_bytes(
        AssetPath::new(format!("asset.{extension}")).expect("valid work callback source"),
        b"fixture",
        ImportOperationControl::default(),
    ))
}

fn assert_dimension(
    failure: &ImportFailure,
    dimension: ImportDimension,
    attempted: u64,
    consumed: u64,
) {
    let evidence = failure
        .ledger()
        .dimension(dimension)
        .expect("work callback dimension evidence");
    // Stop a matching direct-bytes import at parser entry so startup, ownership sharing and
    // hashing remain observable without being mistaken for attempted or completed callback work.
    let (entry_attempted, entry_consumed) = if dimension == ImportDimension::WorkUnits {
        let baseline = run_case(WorkCase::EntryFailure, None)
            .expect_err("the baseline stops before callback admission");
        assert!(matches!(
            baseline.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::ResourceLimit {
                        resource: "work-callback-entry",
                        limit: 0,
                    }
                )
        ));
        let entry_work = baseline
            .ledger()
            .dimension(dimension)
            .expect("parser-entry work evidence");
        (entry_work.attempted(), entry_work.consumed())
    } else {
        (0, 0)
    };
    assert_eq!(evidence.attempted(), entry_attempted + attempted);
    assert_eq!(evidence.consumed(), entry_consumed + consumed);
}

#[test]
fn exact_group_and_zero_group_publish_exact_work() {
    let exact = run_case(WorkCase::ExactGroup, None).expect("exact work group succeeds");
    let zero = run_case(WorkCase::ZeroGroup, None).expect("zero work group is a no-op");
    let dynamic =
        run_case(WorkCase::DynamicChunks, None).expect("dynamic work chunks succeed exactly");
    let exact_work = exact
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("work unit evidence");
    let zero_work = zero
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("zero work unit evidence");
    assert_eq!(
        exact_work.attempted() - zero_work.attempted(),
        2,
        "the callback contributes only its own attempted work"
    );
    assert_eq!(
        exact_work.consumed() - zero_work.consumed(),
        2,
        "the callback contributes only its own consumed work"
    );
    let exact_tokens = exact
        .ledger()
        .dimension(ImportDimension::Tokens)
        .expect("token evidence");
    let zero_tokens = zero
        .ledger()
        .dimension(ImportDimension::Tokens)
        .expect("zero token evidence");
    assert_eq!(
        (
            exact_tokens.attempted() - zero_tokens.attempted(),
            exact_tokens.consumed() - zero_tokens.consumed(),
        ),
        (3, 3)
    );
    let dynamic_work = dynamic
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("dynamic work evidence");
    assert_eq!(
        (
            dynamic_work.attempted() - zero_work.attempted(),
            dynamic_work.consumed() - zero_work.consumed(),
        ),
        (8, 8)
    );
}

#[test]
fn callback_error_and_incomplete_success_preserve_exact_prefixes() {
    let callback =
        run_case(WorkCase::CallbackError, None).expect_err("callback error remains terminal");
    assert!(matches!(
        callback.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: "work-callback-error",
                    limit: 1,
                }
            )
    ));
    assert_dimension(&callback, ImportDimension::WorkUnits, 2, 1);
    assert_dimension(&callback, ImportDimension::Tokens, 2, 1);

    let incomplete = run_case(WorkCase::Incomplete, None)
        .expect_err("an incomplete successful callback is a contract failure");
    assert!(matches!(
        incomplete.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::DescriptorContract {
                    code: "parse-work-callback-incomplete",
                }
            )
    ));
    assert_eq!(incomplete.cause().owner(), CauseOwner::Import);
    assert_dimension(&incomplete, ImportDimension::WorkUnits, 2, 1);
    assert_dimension(&incomplete, ImportDimension::Tokens, 2, 1);
}

#[test]
fn returned_piece_preserves_non_send_owned_errors_and_the_actual_completed_prefix() {
    let failure = run_case(WorkCase::ReturnedOwnedError, None)
        .expect_err("an owned piece error must stop subsequent physical work");
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
    if matches!(abort.cause(), ScopeAbortCause::DescriptorContract {
        code: "import-store-runtime-contract",
    })));
    assert_dimension(&failure, ImportDimension::WorkUnits, 2, 1);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

#[test]
fn scan_boundary_releases_only_the_unused_suffix_and_allows_the_following_child() {
    let report =
        run_case(WorkCase::ScanBoundary, None).expect("a classified scan boundary is valid");
    let tokens = report
        .ledger()
        .dimension(ImportDimension::Tokens)
        .expect("the actual scan token evidence");
    assert_eq!((tokens.attempted(), tokens.consumed()), (4, 3));
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}

#[test]
fn scan_exhaustion_requires_its_exact_physical_prefix() {
    let failure = run_case(WorkCase::ScanIncomplete, None)
        .expect_err("exhaustion cannot discard an unfinished suffix");
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
    if matches!(abort.cause(), ScopeAbortCause::DescriptorContract {
        code: "parse-work-callback-incomplete",
    })));
    assert_dimension(&failure, ImportDimension::Tokens, 3, 2);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

#[test]
fn scan_boundary_exit_preserves_control_priority_and_the_physical_prefix() {
    let cancellation = CancellationToken::new();
    let request = request_for(WorkImporter {
        case: WorkCase::ScanBoundaryCancel,
        cancellation: Some(cancellation.clone()),
    });
    let failure = operation_result(request.read_bytes(
        AssetPath::new("asset.scan-boundary-cancel").expect("valid scan source"),
        b"fixture",
        ImportOperationControl::default().with_cancellation(cancellation),
    ))
    .expect_err("a normal scan boundary must still observe cancellation");
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
    if matches!(abort.cause(), ScopeAbortCause::Control(_))));
    assert_dimension(&failure, ImportDimension::Tokens, 3, 2);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

/// The paired valid and compile-fail examples on `ParseRoot` in
/// `src/scope/parser_execution.rs` cover same-root nested child reentry statically.
/// This runtime case retains the valid explicit parent-to-child completion contract.
#[test]
fn parent_work_can_span_child_work_and_completes_only_after_the_child() {
    let report = run_case(WorkCase::ParentNestedSuccess, None)
        .expect("explicit parent work may contain sequential child work");
    let records = report
        .ledger()
        .dimension(ImportDimension::Records)
        .expect("parent record evidence");
    let tokens = report
        .ledger()
        .dimension(ImportDimension::Tokens)
        .expect("child token evidence");
    assert_eq!((records.attempted(), records.consumed()), (1, 1));
    assert_eq!((tokens.attempted(), tokens.consumed()), (2, 2));
}

#[test]
fn child_failure_leaves_parent_work_attempted_but_incomplete() {
    let failure = run_case(WorkCase::ParentChildFailure, None)
        .expect_err("a child failure must terminate its parent action");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: "parent-child-failure",
                    limit: 1,
                }
            )
    ));
    assert_dimension(&failure, ImportDimension::Records, 1, 0);
    assert_dimension(&failure, ImportDimension::WorkUnits, 2, 1);
}

#[test]
fn parent_work_group_completes_all_dimensions_only_after_the_child() {
    let report = run_case(WorkCase::ParentGroupNestedSuccess, None)
        .expect("an explicit parent group may contain sequential child work");
    for (dimension, expected) in [
        (ImportDimension::Records, 1),
        (ImportDimension::AttributeCells, 2),
        (ImportDimension::Tokens, 3),
    ] {
        let evidence = report
            .ledger()
            .dimension(dimension)
            .expect("parent group evidence remains queryable");
        assert_eq!(
            (evidence.attempted(), evidence.consumed()),
            (expected, expected)
        );
    }
}

#[test]
fn child_failure_leaves_every_parent_group_dimension_incomplete() {
    let failure = run_case(WorkCase::ParentGroupChildFailure, None)
        .expect_err("a child failure must terminate its parent group action");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: "parent-group-child-failure",
                    limit: 1,
                }
            )
    ));
    assert_dimension(&failure, ImportDimension::Records, 1, 0);
    assert_dimension(&failure, ImportDimension::AttributeCells, 2, 0);
    assert_dimension(&failure, ImportDimension::WorkUnits, 2, 1);
}

#[test]
fn a_caught_parent_panic_stops_the_root_before_any_later_parent_or_child() {
    exclusive_bridge::assert_caught_reuse(false);
}

#[test]
fn a_dynamic_chunk_cannot_replace_unfinished_work() {
    let failure = run_case(WorkCase::PrematureReplenish, None)
        .expect_err("unfinished work cannot be replaced by another chunk");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Ledger(LedgerCause::InactiveLinearWorkPermit)
            )
    ));
    assert_dimension(&failure, ImportDimension::WorkUnits, 2, 0);
}

#[test]
fn item_limited_work_cannot_be_split_across_dynamic_chunks() {
    let failure = run_case(WorkCase::ItemLimitedReplenish, None)
        .expect_err("item-limited work cannot use dynamic chunks");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Ledger(LedgerCause::InactiveLinearWorkPermit)
            )
    ));
    assert_dimension(&failure, ImportDimension::CanonicalStringBytes, 0, 0);
}

#[test]
fn grouped_admission_stops_at_the_first_item_limit_failure() {
    let failure = run_case(WorkCase::GroupItemLimit, None)
        .expect_err("the first item limit failure stops grouped admission");
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected a scope-owned item-limit failure")
    };
    let ScopeAbortCause::ResourceLimit {
        resource: "line-bytes",
        limit,
    } = abort.cause()
    else {
        panic!("expected the line-byte resource limit")
    };
    assert_dimension(
        &failure,
        ImportDimension::DecodeWork,
        limit.checked_add(1).expect("test excess fits the counter"),
        0,
    );
    assert_dimension(&failure, ImportDimension::Tokens, 0, 0);
}

#[test]
fn callback_panic_preserves_the_completed_prefix_and_releases_the_scope() {
    let failure = run_case(WorkCase::CallbackPanic, None)
        .expect_err("a callback panic is contained by the importer boundary");
    assert!(matches!(
        failure.cause(),
        ImportCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::Importer,
            code: "importer-panicked",
        }
    ));
    assert_dimension(&failure, ImportDimension::WorkUnits, 2, 1);
    assert_dimension(&failure, ImportDimension::Tokens, 2, 1);
}

#[test]
fn a_caught_callback_panic_stops_the_scope_before_the_next_action() {
    exclusive_bridge::assert_caught_reuse(true);
}

#[test]
fn duplicate_dimensions_are_aggregated_without_losing_caller_prefixes() {
    let report = run_case(WorkCase::DuplicateDimension, None)
        .expect("duplicate dimensions are one atomic ledger batch");
    let baseline = run_case(WorkCase::ZeroGroup, None).expect("zero-work baseline succeeds");
    let work = report
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("work unit evidence");
    let baseline_work = baseline
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("baseline work unit evidence");
    assert_eq!(
        (
            work.attempted() - baseline_work.attempted(),
            work.consumed() - baseline_work.consumed(),
        ),
        (5, 5)
    );
}

#[test]
fn callback_error_reconciles_a_same_action_cancel_to_control() {
    let cancellation = CancellationToken::new();
    let request = request_for(WorkImporter {
        case: WorkCase::CallbackErrorCancel,
        cancellation: Some(cancellation.clone()),
    });
    let failure = operation_result(request.read_bytes(
        AssetPath::new("asset.work-error-cancel").expect("valid cancellation source"),
        b"fixture",
        ImportOperationControl::default().with_cancellation(cancellation),
    ))
    .expect_err("the callback error path must preserve final control authority");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::Control(_))
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_dimension(&failure, ImportDimension::WorkUnits, 2, 1);
}

#[test]
fn cancellation_after_the_last_action_reconciles_to_control() {
    let cancellation = CancellationToken::new();
    let request = request_for(WorkImporter {
        case: WorkCase::CancelAfterCompletion,
        cancellation: Some(cancellation.clone()),
    });
    let failure = operation_result(request.read_bytes(
        AssetPath::new("asset.work-cancel").expect("valid cancellation source"),
        b"fixture",
        ImportOperationControl::default().with_cancellation(cancellation),
    ))
    .expect_err("the final callback poll must observe cancellation");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::Control(_))
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_dimension(&failure, ImportDimension::WorkUnits, 2, 2);
}

#[test]
fn parent_cancellation_precedes_automatic_parent_completion() {
    let cancellation = CancellationToken::new();
    let request = request_for(WorkImporter {
        case: WorkCase::ParentCancelBeforeCompletion,
        cancellation: Some(cancellation.clone()),
    });
    let failure = operation_result(request.read_bytes(
        AssetPath::new("asset.work-parent-cancel").expect("valid cancellation source"),
        b"fixture",
        ImportOperationControl::default().with_cancellation(cancellation),
    ))
    .expect_err("cancellation must stop the sealed parent completion");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::Control(_))
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_dimension(&failure, ImportDimension::Records, 1, 0);
}

#[test]
fn incomplete_success_reconciles_a_final_cancel_before_contract_failure() {
    let cancellation = CancellationToken::new();
    let request = request_for(WorkImporter {
        case: WorkCase::IncompleteCancel,
        cancellation: Some(cancellation.clone()),
    });
    let failure = operation_result(request.read_bytes(
        AssetPath::new("asset.work-incomplete-cancel").expect("valid cancellation source"),
        b"fixture",
        ImportOperationControl::default().with_cancellation(cancellation),
    ))
    .expect_err("the final poll must make cancellation authoritative");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::Control(_))
    ));
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_dimension(&failure, ImportDimension::WorkUnits, 2, 1);
}

/// Real registry/session vertical cases for the replacement entry. These observations count
/// physical callbacks and destructors; all work/storage arithmetic remains in the production owner.
mod exclusive_bridge {
    use super::*;
    use baozi_import::{
        DetectionFailure, DetectionFaultCode, FormatFailureDetailSink, FormatFailureDetailValue,
        FormatFailureEvidenceProducer, ImportConfigOverlay, ImportPhase, ImportProgressPhase,
        ImportProgressUnit, ImportRequestLayer, ImportTerminalCause, ImportTerminalFailureClass,
        ResourceLimits, ResourceLimitsOverlay,
    };
    use baozi_io::{AssetReadError, AssetReadErrorKind, AssetReader, AuthorityDomain, OpenedAsset};
    use baozi_operation::{
        CancellationOrigin, CancellationSignal, ControlCause, ProgressDecision, ProgressEvent,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Case {
        Success,
        ReplenishedComparisons,
        Progress,
        ProgressObserverCancel,
        ProgressFirstPollCancel,
        ProgressSecondPollCancel,
        ProgressObserverPanic,
        DeniedPushDrop,
        DeniedCaptureDrop,
        Incomplete,
        IncompleteCancel,
        Semantic,
        SemanticCancel,
        Format,
        CodePanic,
        DetailsPanic,
        DropPanic,
        CodeAndDropPanic,
        FormatCancel,
        Panic,
        PanicCancel,
        CaughtReturn,
        CaughtNew,
        CaughtLegacy,
        CaughtSubmit,
        CaughtSubmitComplete,
        SubmitPanic,
        IoReturn,
        IoEof,
        IoError,
        IoPanic,
    }

    #[derive(Default)]
    struct Observation {
        writes: AtomicUsize,
        next: AtomicUsize,
        producer_drops: AtomicUsize,
        code: AtomicUsize,
        details: AtomicUsize,
        reader_calls: AtomicUsize,
        reader_drops: AtomicUsize,
        polls: AtomicUsize,
        cancel_at_poll: AtomicUsize,
        region_polls: AtomicUsize,
        progress_checks: AtomicUsize,
        progress_rejections: AtomicUsize,
        progress_cancels: AtomicUsize,
        progress_panics: AtomicUsize,
        progress_events: Mutex<Vec<ProgressEvent<ImportProgressPhase, ImportProgressUnit>>>,
        owned_drops: AtomicUsize,
        entries: AtomicUsize,
        submitted: AtomicUsize,
    }
    #[derive(Clone)]
    struct Signal {
        token: CancellationToken,
        observation: Arc<Observation>,
    }
    impl CancellationSignal for Signal {
        fn is_cancelled(&self) -> bool {
            let poll = self.observation.polls.fetch_add(1, Ordering::SeqCst) + 1;
            if poll == self.observation.cancel_at_poll.load(Ordering::SeqCst) {
                self.observation
                    .progress_cancels
                    .fetch_add(1, Ordering::SeqCst);
                self.token.cancel();
            }
            self.token.is_cancelled()
        }
    }
    struct Producer {
        case: Case,
        observation: Arc<Observation>,
        token: CancellationToken,
    }
    impl FormatFailureEvidenceProducer for Producer {
        fn code(&self) -> &str {
            self.observation.code.fetch_add(1, Ordering::SeqCst);
            if matches!(self.case, Case::CodePanic | Case::CodeAndDropPanic) {
                panic!("real format code callback");
            }
            "test.exclusive_format"
        }
        fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
            self.observation.details.fetch_add(1, Ordering::SeqCst);
            sink.unsigned("line", 7);
            sink.unsigned("column", 11);
            if self.case == Case::DetailsPanic {
                panic!("real format details callback");
            }
            if self.case == Case::FormatCancel {
                self.token.cancel();
            }
        }
    }
    impl Drop for Producer {
        fn drop(&mut self) {
            self.observation
                .producer_drops
                .fetch_add(1, Ordering::SeqCst);
            if matches!(self.case, Case::DropPanic | Case::CodeAndDropPanic) {
                panic!("real producer destructor");
            }
        }
    }
    struct OwnedValue(Arc<Observation>);
    impl Drop for OwnedValue {
        fn drop(&mut self) {
            self.0.owned_drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct DeniedDrop(Arc<Observation>);
    impl Drop for DeniedDrop {
        fn drop(&mut self) {
            self.0.owned_drops.fetch_add(1, Ordering::SeqCst);
            panic!("real denied value/capture destructor");
        }
    }
    struct Reader {
        case: Case,
        observation: Arc<Observation>,
    }
    impl AssetReader for Reader {
        fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
            self.observation.reader_calls.fetch_add(1, Ordering::SeqCst);
            match self.case {
                Case::IoPanic => panic!("actual AssetReader callback"),
                Case::IoError => Err(AssetReadError::new(AssetReadErrorKind::InvalidData)),
                Case::IoEof => Ok(0),
                _ => {
                    buffer[0] = 73;
                    Ok(1)
                }
            }
        }
    }
    impl Drop for Reader {
        fn drop(&mut self) {
            self.observation.reader_drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct ExclusiveWorkImporter {
        case: Case,
        observation: Arc<Observation>,
        token: CancellationToken,
    }
    impl Format for ExclusiveWorkImporter {
        type Options = NoFormatOptions;
        fn compile_defaults(
            &self,
            scope: &mut RegistryBuildScope<'_>,
        ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
            NoFormatOptions::compile_defaults(scope)
        }
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.exclusive-work").unwrap(),
                "Exclusive Work",
            )
            .extensions(["exclusive-work"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .unwrap()
        }
        fn detect(
            &self,
            _: &baozi_import::Probe<'_>,
            _: &ReadHint,
            _: &mut DetectionScope<'_, '_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }
        fn parse(&self, _: &Self::Options, context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
            self.observation.entries.fetch_add(1, Ordering::SeqCst);
            let before = self.observation.polls.load(Ordering::SeqCst);
            let result = context.with_parser_execution(|root| -> ParseResult<()> {
                if matches!(
                    self.case,
                    Case::Progress
                        | Case::ProgressObserverCancel
                        | Case::ProgressFirstPollCancel
                        | Case::ProgressSecondPollCancel
                        | Case::ProgressObserverPanic
                ) {
                    let group =
                        root.bind_group([ImportWorkClass::Tokens, ImportWorkClass::WorkUnits])?;
                    let baseline = root.work_counters(ImportWorkClass::WorkUnits)?;
                    let before = self.observation.polls.load(Ordering::SeqCst);
                    root.report_progress(
                        ImportProgressPhase::Decode,
                        ImportProgressUnit::WorkUnits,
                        0,
                        Some(2),
                    )?;
                    assert_eq!(self.observation.polls.load(Ordering::SeqCst) - before, 2);
                    self.observation
                        .progress_checks
                        .fetch_add(1, Ordering::SeqCst);
                    assert_eq!(root.work_counters(ImportWorkClass::WorkUnits)?, baseline);
                    let result = root.child(&group, [2, 2], |child| -> ParseResult<()> {
                        child.run([1, 1], || {
                            self.observation.writes.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })?;
                        let before = self.observation.polls.load(Ordering::SeqCst);
                        let cancel_poll = match self.case {
                            Case::ProgressFirstPollCancel => Some(1),
                            Case::ProgressSecondPollCancel => Some(2),
                            _ => None,
                        };
                        if let Some(cancel_poll) = cancel_poll {
                            self.observation
                                .cancel_at_poll
                                .store(before + cancel_poll, Ordering::SeqCst);
                        }
                        let progress = child.report_progress(
                            ImportProgressPhase::Decode,
                            ImportProgressUnit::WorkUnits,
                            1,
                            Some(2),
                        );
                        assert_eq!(
                            self.observation.polls.load(Ordering::SeqCst) - before,
                            if self.case == Case::ProgressFirstPollCancel {
                                1
                            } else {
                                2
                            }
                        );
                        self.observation
                            .progress_checks
                            .fetch_add(1, Ordering::SeqCst);
                        let actual = child.work_counters(ImportWorkClass::WorkUnits)?;
                        assert_eq!((actual.0 - baseline.0, actual.1 - baseline.1), (2, 1));
                        let tokens = child.work_counters(ImportWorkClass::Tokens)?;
                        assert_eq!((tokens.0, tokens.1), (2, 1));
                        if matches!(
                            self.case,
                            Case::ProgressObserverCancel
                                | Case::ProgressFirstPollCancel
                                | Case::ProgressSecondPollCancel
                                | Case::ProgressObserverPanic
                        ) {
                            let first_failure =
                                progress.expect_err("the actual progress boundary stops");
                            self.token.cancel();
                            assert!(
                                child
                                    .run([1, 1], || -> ParseResult<()> {
                                        self.observation.next.fetch_add(1, Ordering::SeqCst);
                                        Ok(())
                                    })
                                    .is_err()
                            );
                            self.observation
                                .progress_rejections
                                .fetch_add(1, Ordering::SeqCst);
                            assert!(
                                child
                                    .report_progress(
                                        ImportProgressPhase::Decode,
                                        ImportProgressUnit::WorkUnits,
                                        2,
                                        Some(2),
                                    )
                                    .is_err()
                            );
                            self.observation
                                .progress_rejections
                                .fetch_add(1, Ordering::SeqCst);
                            return Err(first_failure);
                        }
                        progress?;
                        child.run([1, 1], || {
                            self.observation.writes.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    });
                    let completed = if matches!(
                        self.case,
                        Case::ProgressObserverCancel
                            | Case::ProgressFirstPollCancel
                            | Case::ProgressSecondPollCancel
                            | Case::ProgressObserverPanic
                    ) {
                        1
                    } else {
                        2
                    };
                    let actual = root.work_counters(ImportWorkClass::WorkUnits)?;
                    assert_eq!(
                        (actual.0 - baseline.0, actual.1 - baseline.1, actual.2),
                        (2, completed, 0)
                    );
                    assert_eq!(
                        root.work_counters(ImportWorkClass::Tokens)?,
                        (2, completed, 0)
                    );
                    if matches!(
                        self.case,
                        Case::ProgressObserverCancel
                            | Case::ProgressFirstPollCancel
                            | Case::ProgressSecondPollCancel
                            | Case::ProgressObserverPanic
                    ) {
                        assert!(result.is_err());
                        assert!(
                            root.report_progress(
                                ImportProgressPhase::Decode,
                                ImportProgressUnit::WorkUnits,
                                2,
                                Some(2),
                            )
                            .is_err()
                        );
                        self.observation
                            .progress_rejections
                            .fetch_add(1, Ordering::SeqCst);
                        assert!(
                            root.child(&group, [1, 1], |_| -> ParseResult<()> {
                                self.observation.next.fetch_add(1, Ordering::SeqCst);
                                Ok(())
                            })
                            .is_err()
                        );
                        self.observation
                            .progress_rejections
                            .fetch_add(1, Ordering::SeqCst);
                        return result;
                    }
                    result?;
                    let before = self.observation.polls.load(Ordering::SeqCst);
                    root.report_progress(
                        ImportProgressPhase::Decode,
                        ImportProgressUnit::WorkUnits,
                        2,
                        Some(2),
                    )?;
                    assert_eq!(self.observation.polls.load(Ordering::SeqCst) - before, 2);
                    self.observation
                        .progress_checks
                        .fetch_add(1, Ordering::SeqCst);
                    assert_eq!(root.work_counters(ImportWorkClass::WorkUnits)?, actual);
                    return Ok(());
                }
                if self.case == Case::ReplenishedComparisons {
                    let comparisons = root.bind_group([ImportWorkClass::WorkUnits])?;
                    let baseline = root.work_counters(ImportWorkClass::WorkUnits)?;
                    let found = root.child(&comparisons, [0], |child| -> ParseResult<_> {
                        let mut found = [None; 2];
                        for (slot, needle) in found.iter_mut().zip([5_u32, 14]) {
                            for (index, value) in [2_u32, 5, 9, 14].into_iter().enumerate() {
                                child.replenish(1)?;
                                let equal = child.run([1], || {
                                    self.observation.writes.fetch_add(1, Ordering::SeqCst);
                                    Ok::<_, ParseFailure>(value == needle)
                                })?;
                                if equal {
                                    *slot = Some(index);
                                    break;
                                }
                            }
                        }
                        Ok(found)
                    })?;
                    assert_eq!(found, [Some(1), Some(3)]);
                    let actual = root.work_counters(ImportWorkClass::WorkUnits)?;
                    assert_eq!(
                        (actual.0 - baseline.0, actual.1 - baseline.1, actual.2),
                        (6, 6, 0)
                    );
                    assert_eq!(root.work_counters(ImportWorkClass::Tokens)?, (0, 0, 0));
                    return Ok(());
                }
                if matches!(self.case, Case::DeniedPushDrop | Case::DeniedCaptureDrop) {
                    let group = root.bind_group([ImportWorkClass::Tokens])?;
                    return root.child(&group, [2], |child| -> ParseResult<()> {
                        child.run([1], || {
                            self.observation.writes.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })?;
                        let captured = DeniedDrop(Arc::clone(&self.observation));
                        if self.case == Case::DeniedPushDrop {
                            let mut values = child.storage()?.metered_vec(
                                baozi_import::ImportStorageClass::FormatNormalization,
                                "denied-value",
                            );
                            child.push_growing([2], &mut values, captured)?;
                        } else {
                            child.run([2], move || {
                                std::hint::black_box(&captured);
                                self.observation.writes.fetch_add(1, Ordering::SeqCst);
                                Ok(())
                            })?;
                        }
                        self.observation.next.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    });
                }
                if matches!(
                    self.case,
                    Case::IoReturn | Case::IoEof | Case::IoError | Case::IoPanic
                ) {
                    let call_before = root.work_counters(ImportWorkClass::WorkUnits)?;
                    let mut reader = OpenedAsset::new(
                        AuthorityDomain::new(),
                        root.source().clone(),
                        Reader {
                            case: self.case,
                            observation: Arc::clone(&self.observation),
                        },
                        None,
                    );
                    let mut byte = [0];
                    let result = root.read_chunk(&mut reader, &mut byte);
                    let call_after = root.work_counters(ImportWorkClass::WorkUnits)?;
                    assert_eq!(
                        (
                            call_after.0 - call_before.0,
                            call_after.1 - call_before.1,
                            call_after.2
                        ),
                        (1, if self.case == Case::IoPanic { 0 } else { 1 }, 0)
                    );
                    match self.case {
                        Case::IoReturn => {
                            assert_eq!(result?.unwrap(), 1);
                            assert_eq!(byte, [73]);
                        }
                        Case::IoEof => assert_eq!(result?.unwrap(), 0),
                        Case::IoError => assert!(result?.is_err()),
                        Case::IoPanic => match result {
                            Err(cause) => return Err(cause),
                            Ok(returned) => {
                                panic!("panicking AssetReader unexpectedly returned {returned:?}")
                            }
                        },
                        _ => unreachable!(),
                    }
                    assert_eq!(root.work_counters(ImportWorkClass::Tokens)?, (0, 0, 0));
                    return Ok(());
                }
                let group =
                    root.bind_group([ImportWorkClass::Tokens, ImportWorkClass::WorkUnits])?;
                let records = root.bind_group([ImportWorkClass::Records])?;
                let baseline = root.work_counters(ImportWorkClass::WorkUnits)?;
                let attempt = catch_unwind(AssertUnwindSafe(|| {
                    root.parent(&records, [1], |parent| -> ParseResult<()> {
                        parent.child(&group, [2, 2], |child| -> ParseResult<()> {
                            let value = child.run([1, 1], || {
                                self.observation.writes.fetch_add(1, Ordering::SeqCst);
                                Ok(OwnedValue(Arc::clone(&self.observation)))
                            })?;
                            drop(value);
                            if matches!(self.case, Case::Incomplete | Case::IncompleteCancel) {
                                if self.case == Case::IncompleteCancel {
                                    self.token.cancel();
                                }
                                return Ok(());
                            }
                            if matches!(
                                self.case,
                                Case::Format
                                    | Case::CodePanic
                                    | Case::DetailsPanic
                                    | Case::DropPanic
                                    | Case::CodeAndDropPanic
                                    | Case::FormatCancel
                            ) {
                                return child.run_format([1, 1], || -> Result<(), Producer> {
                                    self.observation.writes.fetch_add(1, Ordering::SeqCst);
                                    Err(Producer {
                                        case: self.case,
                                        observation: Arc::clone(&self.observation),
                                        token: self.token.clone(),
                                    })
                                });
                            }
                            let cold = child.error_view();
                            child.run([1, 1], || {
                                self.observation.writes.fetch_add(1, Ordering::SeqCst);
                                match self.case {
                                    Case::Semantic | Case::SemanticCancel => {
                                        if self.case == Case::SemanticCancel {
                                            self.token.cancel();
                                        }
                                        Err(cold.resource_limit_exceeded("exclusive-semantic", 41))
                                    }
                                    Case::Panic
                                    | Case::PanicCancel
                                    | Case::CaughtReturn
                                    | Case::CaughtNew
                                    | Case::CaughtLegacy
                                    | Case::CaughtSubmit
                                    | Case::CaughtSubmitComplete => {
                                        if self.case == Case::PanicCancel {
                                            self.token.cancel();
                                        }
                                        panic!("actual exclusive parser leaf");
                                    }
                                    _ => Ok(()),
                                }
                            })?;
                            Ok(())
                        })?;
                        Ok(())
                    })
                }));
                match attempt {
                    Err(payload) => {
                        // This catch is deliberately inside the importer; rethrow is the actual
                        // uncaught registry path, while the caught cases must remain stopped.
                        if matches!(self.case, Case::Panic | Case::PanicCancel) {
                            std::panic::resume_unwind(payload);
                        }
                        drop(payload);
                        let counts = root.work_counters(ImportWorkClass::Tokens)?;
                        assert_eq!(counts, (2, 1, 0));
                        assert_eq!(root.work_counters(ImportWorkClass::Records)?, (1, 0, 0));
                        Ok(())
                    }
                    Ok(result) => {
                        let tokens = root.work_counters(ImportWorkClass::Tokens)?;
                        if result.is_err() {
                            assert_eq!(tokens, (2, 1, 0));
                            assert_eq!(root.work_counters(ImportWorkClass::Records)?, (1, 0, 0));
                        } else {
                            assert_eq!(tokens, (2, 2, 0));
                        }
                        let actual = root.work_counters(ImportWorkClass::WorkUnits)?;
                        // The accounted format error adds its real Internal clone/handle work
                        // on WorkUnits in addition to the admitted parser piece.
                        let internal = if self.observation.code.load(Ordering::SeqCst) != 0 {
                            2
                        } else {
                            0
                        };
                        assert_eq!(
                            (actual.0 - baseline.0, actual.1 - baseline.1, actual.2),
                            (
                                2 + internal,
                                if result.is_ok() { 2 } else { 1 } + internal,
                                0
                            )
                        );
                        result
                    }
                }
            });
            self.observation.region_polls.store(
                self.observation.polls.load(Ordering::SeqCst) - before,
                Ordering::SeqCst,
            );
            if matches!(
                self.case,
                Case::ProgressObserverCancel
                    | Case::ProgressFirstPollCancel
                    | Case::ProgressSecondPollCancel
                    | Case::ProgressObserverPanic
            ) {
                assert!(result.is_err());
                assert!(
                    context
                        .with_parser_execution(|_| -> ParseResult<()> {
                            self.observation.next.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                        .is_err()
                );
                self.observation
                    .progress_rejections
                    .fetch_add(1, Ordering::SeqCst);
                assert!(support::submit_minimal_scene(context).is_err());
                self.observation
                    .progress_rejections
                    .fetch_add(1, Ordering::SeqCst);
                return Ok(());
            }
            if matches!(
                self.case,
                Case::CaughtReturn
                    | Case::CaughtNew
                    | Case::CaughtLegacy
                    | Case::CaughtSubmit
                    | Case::CaughtSubmitComplete
            ) {
                assert!(
                    result.is_err(),
                    "caught numeric panic cannot complete the region"
                );
                match self.case {
                    Case::CaughtNew => assert!(
                        context
                            .with_parser_execution(|_| -> ParseResult<()> {
                                self.observation.next.fetch_add(1, Ordering::SeqCst);
                                Ok(())
                            })
                            .is_err()
                    ),
                    Case::CaughtLegacy => assert!(
                        context
                            .with_execution(|_| -> ParseResult<()> {
                                self.observation.next.fetch_add(1, Ordering::SeqCst);
                                Ok(())
                            })
                            .is_err()
                    ),
                    Case::CaughtSubmit => {
                        use baozi_import::authoring::{Presence, StaticSceneBuilder};
                        assert!(
                            context
                                .submit(StaticSceneBuilder::new(Presence::Absent, &[]))
                                .is_err()
                        );
                    }
                    Case::CaughtSubmitComplete => {
                        assert!(support::submit_minimal_scene(context).is_err())
                    }
                    _ => {}
                }
                return Ok(());
            }
            result?;
            support::submit_minimal_scene(context)?;
            self.observation.submitted.fetch_add(1, Ordering::SeqCst);
            if self.case == Case::SubmitPanic {
                panic!("importer panic after actual Pending submit");
            }
            Ok(())
        }
    }
    fn plan(
        case: Case,
        observation: Arc<Observation>,
        token: CancellationToken,
        limits: Option<ResourceLimits>,
    ) -> ImportPlan {
        let mut builder = RegistryBuilder::new();
        builder
            .register(ExclusiveWorkImporter {
                case,
                observation,
                token,
            })
            .unwrap();
        let request = match limits {
            None => ImportRequest::new(),
            Some(limits) => ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
                ImportConfigOverlay::default()
                    .with_resource_limits(ResourceLimitsOverlay::from_limits(limits)),
            )),
        };
        builder
            .freeze(RegistryBuildLimits::default())
            .unwrap()
            .plan(&request, OptionCompileLimits::default())
            .unwrap()
    }
    fn run(
        case: Case,
    ) -> (
        Result<baozi_import::ImportReport, ImportFailure>,
        Arc<Observation>,
    ) {
        let observation = Arc::new(Observation::default());
        let token = CancellationToken::new();
        let plan = plan(case, Arc::clone(&observation), token.clone(), None);
        let signal = Signal {
            token,
            observation: Arc::clone(&observation),
        };
        let mut control = ImportOperationControl::default().with_cancellation(signal);
        if matches!(
            case,
            Case::Progress
                | Case::ProgressObserverCancel
                | Case::ProgressFirstPollCancel
                | Case::ProgressSecondPollCancel
                | Case::ProgressObserverPanic
        ) {
            let observation = Arc::clone(&observation);
            control = control.with_progress_observer(
                move |event: ProgressEvent<ImportProgressPhase, ImportProgressUnit>| {
                    observation.progress_events.lock().unwrap().push(event);
                    if event.phase() == ImportProgressPhase::Decode
                        && event.unit() == ImportProgressUnit::WorkUnits
                        && event.completed() == 1
                    {
                        match case {
                            Case::ProgressObserverCancel => {
                                observation.progress_cancels.fetch_add(1, Ordering::SeqCst);
                                ProgressDecision::Cancel
                            }
                            Case::ProgressObserverPanic => {
                                observation.progress_panics.fetch_add(1, Ordering::SeqCst);
                                panic!("actual Decode work progress observer");
                            }
                            _ => ProgressDecision::Continue,
                        }
                    } else {
                        ProgressDecision::Continue
                    }
                },
            );
        }
        (
            operation_result(plan.read_bytes(
                AssetPath::new("asset.exclusive-work").unwrap(),
                b"fixture",
                control,
            )),
            observation,
        )
    }
    fn assert_progress_events(observation: &Observation, completed: &[u64]) {
        let events = observation.progress_events.lock().unwrap();
        let decode: Vec<_> = events
            .iter()
            .filter(|event| {
                event.phase() == ImportProgressPhase::Decode
                    && event.unit() == ImportProgressUnit::WorkUnits
            })
            .collect();
        assert_eq!(
            decode
                .iter()
                .map(|event| event.completed())
                .collect::<Vec<_>>(),
            completed
        );
        assert!(decode.iter().all(|event| event.total() == Some(2)));
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0].sequence() < pair[1].sequence())
        );
    }
    fn scope_code(failure: &ImportFailure) -> &'static str {
        match failure.cause() {
            ImportCause::Scope(abort) => match abort.cause() {
                ScopeAbortCause::DescriptorContract { code }
                | ScopeAbortCause::TrustedExtension { code, .. } => code,
                _ => panic!("unexpected scope cause: {:?}", abort.cause()),
            },
            _ => panic!("expected actual scope failure"),
        }
    }
    #[test]
    fn exclusive_success_releases_root_before_actual_pending_submit() {
        let (report, observation) = run(Case::Success);
        let report = report.unwrap();
        assert_eq!(observation.writes.load(Ordering::SeqCst), 2);
        assert_eq!(observation.owned_drops.load(Ordering::SeqCst), 1);
        assert_eq!(observation.region_polls.load(Ordering::SeqCst), 5);
        assert_eq!(observation.submitted.load(Ordering::SeqCst), 1);
        assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
        let tokens = report.ledger().dimension(ImportDimension::Tokens).unwrap();
        assert_eq!((tokens.attempted(), tokens.consumed()), (2, 2));
    }

    #[test]
    fn exclusive_replenished_comparisons_keep_literal_work_before_actual_submit() {
        let (report, observation) = run(Case::ReplenishedComparisons);
        let report = report.unwrap();
        assert_eq!(observation.entries.load(Ordering::SeqCst), 1);
        assert_eq!(observation.writes.load(Ordering::SeqCst), 6);
        assert_eq!(observation.submitted.load(Ordering::SeqCst), 1);
        assert_eq!(observation.region_polls.load(Ordering::SeqCst), 3);
        drop(report);
    }

    #[test]
    fn exclusive_root_and_active_child_progress_keep_work_prefix_and_actual_submit() {
        let (report, observation) = run(Case::Progress);
        let report = report.expect("root and child progress permit actual Scene submission");
        assert_eq!(observation.entries.load(Ordering::SeqCst), 1);
        assert_eq!(observation.writes.load(Ordering::SeqCst), 2);
        assert_eq!(observation.progress_checks.load(Ordering::SeqCst), 3);
        assert_eq!(observation.region_polls.load(Ordering::SeqCst), 9);
        assert_eq!(observation.submitted.load(Ordering::SeqCst), 1);
        assert_progress_events(&observation, &[0, 1, 2]);
        assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
        assert_eq!(
            report.ledger().terminal_cause(),
            ImportTerminalCause::Success
        );
        let tokens = report.ledger().dimension(ImportDimension::Tokens).unwrap();
        assert_eq!((tokens.attempted(), tokens.consumed()), (2, 2));
    }

    #[test]
    fn exclusive_progress_observer_cancellation_latches_first_cause_and_rejects_reuse() {
        let (failure, observation) = run(Case::ProgressObserverCancel);
        let failure = failure.expect_err("the actual progress observer stops publication");
        assert_eq!(observation.entries.load(Ordering::SeqCst), 1);
        assert_eq!(observation.writes.load(Ordering::SeqCst), 1);
        assert_eq!(observation.progress_checks.load(Ordering::SeqCst), 2);
        assert_eq!(observation.progress_cancels.load(Ordering::SeqCst), 1);
        assert_eq!(observation.progress_rejections.load(Ordering::SeqCst), 6);
        assert_eq!(observation.region_polls.load(Ordering::SeqCst), 5);
        assert_eq!(observation.next.load(Ordering::SeqCst), 0);
        assert_eq!(observation.submitted.load(Ordering::SeqCst), 0);
        assert_progress_events(&observation, &[0, 1]);
        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Control(ControlCause::Cancelled {
                        origin: CancellationOrigin::Observer,
                    })
                )
        ));
        assert_eq!(failure.cause().owner(), CauseOwner::Operation);
        // Observer cancellation is the primary cause above, not an additional interruption.
        assert!(failure.terminal_control_cause().is_none());
        assert_dimension(&failure, ImportDimension::Tokens, 2, 1);
        assert_dimension(&failure, ImportDimension::WorkUnits, 2, 1);
        assert_eq!(
            failure.last_completed().phase(),
            ImportPhase::SelectionCompleted
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }

    #[test]
    fn exclusive_progress_poll_cancellation_and_observer_panic_preserve_actual_prefix() {
        for case in [
            Case::ProgressFirstPollCancel,
            Case::ProgressSecondPollCancel,
            Case::ProgressObserverPanic,
        ] {
            let (failure, observation) = run(case);
            let failure = failure.expect_err("the actual active-child progress boundary stops");
            assert_eq!(observation.entries.load(Ordering::SeqCst), 1);
            assert_eq!(observation.writes.load(Ordering::SeqCst), 1);
            assert_eq!(observation.progress_checks.load(Ordering::SeqCst), 2);
            assert_eq!(observation.progress_rejections.load(Ordering::SeqCst), 6);
            assert_eq!(observation.next.load(Ordering::SeqCst), 0);
            assert_eq!(observation.submitted.load(Ordering::SeqCst), 0);
            assert_eq!(
                observation.region_polls.load(Ordering::SeqCst),
                if case == Case::ProgressFirstPollCancel {
                    4
                } else {
                    5
                }
            );
            let expected_cause = if case == Case::ProgressObserverPanic {
                assert_eq!(observation.progress_panics.load(Ordering::SeqCst), 1);
                assert_eq!(observation.progress_cancels.load(Ordering::SeqCst), 0);
                assert_progress_events(&observation, &[0, 1]);
                ControlCause::ObserverPanicked
            } else {
                assert_eq!(observation.progress_panics.load(Ordering::SeqCst), 0);
                assert_eq!(observation.progress_cancels.load(Ordering::SeqCst), 1);
                assert_eq!(
                    observation.polls.load(Ordering::SeqCst),
                    observation.cancel_at_poll.load(Ordering::SeqCst)
                );
                assert_progress_events(&observation, &[0]);
                ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                }
            };
            let events = observation.progress_events.lock().unwrap();
            assert_eq!(
                events.last().map(|event| (
                    event.phase(),
                    event.unit(),
                    event.completed(),
                    event.total()
                )),
                Some((
                    ImportProgressPhase::Decode,
                    ImportProgressUnit::WorkUnits,
                    u64::from(case == Case::ProgressObserverPanic),
                    Some(2)
                ))
            );
            assert!(matches!(
                failure.cause(),
                ImportCause::Scope(abort)
                    if matches!(
                        abort.cause(),
                        ScopeAbortCause::Control(cause) if cause == &expected_cause
                    )
            ));
            assert_eq!(failure.cause().owner(), CauseOwner::Operation);
            assert!(failure.terminal_control_cause().is_none());
            assert_dimension(&failure, ImportDimension::Tokens, 2, 1);
            assert_dimension(&failure, ImportDimension::WorkUnits, 2, 1);
            assert_eq!(
                failure.last_completed().phase(),
                ImportPhase::SelectionCompleted
            );
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }

    #[test]
    fn exclusive_incomplete_and_semantic_error_use_real_exit_classification() {
        for case in [
            Case::Incomplete,
            Case::IncompleteCancel,
            Case::Semantic,
            Case::SemanticCancel,
        ] {
            let (failure, observation) = run(case);
            let failure = failure.unwrap_err();
            assert_eq!(observation.owned_drops.load(Ordering::SeqCst), 1);
            assert_eq!(
                observation.writes.load(Ordering::SeqCst),
                if matches!(case, Case::Incomplete | Case::IncompleteCancel) {
                    1
                } else {
                    2
                }
            );
            assert_eq!(observation.next.load(Ordering::SeqCst), 0);
            let tokens = failure.ledger().dimension(ImportDimension::Tokens).unwrap();
            let records = failure
                .ledger()
                .dimension(ImportDimension::Records)
                .unwrap();
            assert_eq!((tokens.attempted(), tokens.consumed()), (2, 1));
            assert_eq!((records.attempted(), records.consumed()), (1, 0));
            match case {
                Case::Incomplete => {
                    assert_eq!(scope_code(&failure), "parse-work-callback-incomplete");
                    assert_eq!(failure.cause().owner(), CauseOwner::Import);
                }
                Case::Semantic => assert!(
                    matches!(failure.cause(),ImportCause::Scope(abort) if matches!(abort.cause(),ScopeAbortCause::ResourceLimit {resource:"exclusive-semantic",limit:41}))
                ),
                _ => assert!(
                    matches!(failure.cause(),ImportCause::Scope(abort) if matches!(abort.cause(),ScopeAbortCause::Control(_)))
                ),
            }
            assert_eq!(
                observation.region_polls.load(Ordering::SeqCst),
                if matches!(case, Case::Semantic | Case::SemanticCancel) {
                    1
                } else {
                    2
                }
            );
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }
    #[test]
    fn exclusive_real_format_evidence_callbacks_drop_and_internal_work_stay_owned() {
        for case in [
            Case::Format,
            Case::CodePanic,
            Case::DetailsPanic,
            Case::DropPanic,
            Case::CodeAndDropPanic,
            Case::FormatCancel,
        ] {
            let (failure, observation) = run(case);
            let failure = failure.unwrap_err();
            assert_eq!(observation.writes.load(Ordering::SeqCst), 2);
            assert_eq!(observation.owned_drops.load(Ordering::SeqCst), 1);
            assert_eq!(observation.producer_drops.load(Ordering::SeqCst), 1);
            assert_eq!(observation.code.load(Ordering::SeqCst), 1);
            assert_eq!(
                observation.details.load(Ordering::SeqCst),
                if matches!(case, Case::CodePanic | Case::CodeAndDropPanic) {
                    0
                } else {
                    1
                }
            );
            assert_dimension(&failure, ImportDimension::Tokens, 2, 1);
            match case {
                Case::Format => {
                    let ImportCause::Format(envelope) = failure.cause() else {
                        panic!("accounted format owner was replaced");
                    };
                    assert_eq!(envelope.code(), "test.exclusive_format");
                    let details = envelope.details();
                    assert_eq!(details.len(), 2);
                    assert_eq!(
                        (details[0].key(), details[0].value()),
                        ("line", FormatFailureDetailValue::Unsigned(7))
                    );
                    assert_eq!(
                        (details[1].key(), details[1].value()),
                        ("column", FormatFailureDetailValue::Unsigned(11))
                    );
                    assert!(failure.terminal_control_cause().is_none());
                }
                Case::CodePanic | Case::CodeAndDropPanic => {
                    assert_eq!(scope_code(&failure), "format-failure-code-panicked")
                }
                Case::DetailsPanic => {
                    assert_eq!(scope_code(&failure), "format-failure-detail-panicked")
                }
                Case::DropPanic => assert_eq!(
                    scope_code(&failure),
                    "format-failure-producer-drop-panicked"
                ),
                Case::FormatCancel => assert!(
                    matches!(failure.cause(),ImportCause::Scope(abort) if matches!(abort.cause(),ScopeAbortCause::Control(_)))
                ),
                _ => unreachable!(),
            }
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }
    #[test]
    fn exclusive_admission_precedes_producer_callbacks_and_still_disposes_producer() {
        let (failure, _) = run(Case::Format);
        let failure = failure.unwrap_err();
        let mut limits = ResourceLimits::default();
        limits.max_live_bytes = failure.ledger().peak_live_bytes() - 1;
        let observation = Arc::new(Observation::default());
        let token = CancellationToken::new();
        let plan = plan(
            Case::Format,
            Arc::clone(&observation),
            token.clone(),
            Some(limits),
        );
        let failure = operation_result(plan.read_bytes(
            AssetPath::new("asset.exclusive-work").unwrap(),
            b"fixture",
            ImportOperationControl::default().with_cancellation(Signal {
                token,
                observation: Arc::clone(&observation),
            }),
        ))
        .unwrap_err();
        assert_eq!(observation.writes.load(Ordering::SeqCst), 2);
        assert_eq!(observation.code.load(Ordering::SeqCst), 0);
        assert_eq!(observation.details.load(Ordering::SeqCst), 0);
        assert_eq!(observation.producer_drops.load(Ordering::SeqCst), 1);
        assert_eq!(observation.owned_drops.load(Ordering::SeqCst), 1);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    #[test]
    fn exclusive_uncaught_importer_unwind_and_late_control_are_selected_at_real_boundary() {
        for case in [Case::Panic, Case::PanicCancel, Case::SubmitPanic] {
            let observation = Arc::new(Observation::default());
            let token = CancellationToken::new();
            let plan = plan(case, Arc::clone(&observation), token.clone(), None);
            let failure = operation_result(plan.read_bytes(
                AssetPath::new("asset.exclusive-work").unwrap(),
                b"fixture",
                ImportOperationControl::default().with_cancellation(Signal {
                    token,
                    observation: Arc::clone(&observation),
                }),
            ))
            .unwrap_err();
            if case == Case::PanicCancel {
                assert!(
                    matches!(failure.cause(),ImportCause::Scope(abort) if matches!(abort.cause(),ScopeAbortCause::Control(_)))
                );
            } else {
                assert!(matches!(
                    failure.cause(),
                    ImportCause::TrustedExtension {
                        boundary: TrustedExtensionBoundary::Importer,
                        code: "importer-panicked"
                    }
                ));
            }
            let expected = if case == Case::SubmitPanic { 2 } else { 1 };
            assert_dimension(&failure, ImportDimension::Tokens, 2, expected);
            assert_eq!(
                observation.submitted.load(Ordering::SeqCst),
                usize::from(case == Case::SubmitPanic)
            );
            if case == Case::SubmitPanic {
                assert_eq!(failure.last_completed().phase(), ImportPhase::Validated);
            }
            let repeated = operation_result(plan.read_bytes(
                AssetPath::new("asset.exclusive-work").unwrap(),
                b"fixture",
                ImportOperationControl::default(),
            ))
            .unwrap_err();
            assert!(
                matches!(
                    repeated.cause(),
                    ImportCause::Detection(DetectionFailure::NoSelection {
                        had_match: false,
                        had_inconclusive: false,
                        had_fault: true,
                    })
                ),
                "unexpected repeat cause: {:?}",
                repeated.cause()
            );
            let [evidence] = repeated.detection_evidence() else {
                panic!("the single poisoned registration must retain one detector fault");
            };
            assert_eq!(evidence.format().as_str(), "dev.baozi.test.exclusive-work");
            let DetectionOutcome::Fault(fault) = evidence.outcome() else {
                panic!("a poisoned registration cannot call its detector");
            };
            assert_eq!(fault.code(), DetectionFaultCode::ExtensionPoisoned);
            assert_eq!(fault.detail(), "format registration is poisoned");
            assert!(repeated.selected_format().is_none());
            assert_dimension(&repeated, ImportDimension::Tokens, 0, 0);
            assert_dimension(&repeated, ImportDimension::Records, 0, 0);
            assert_eq!(observation.entries.load(Ordering::SeqCst), 1);
            assert_eq!(observation.owned_drops.load(Ordering::SeqCst), 1);
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(repeated.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
            assert_eq!(repeated.ledger().active_reservations(), 0);
        }
    }
    pub(super) fn assert_caught_reuse(legacy: bool) {
        let (failure, observation) = run(if legacy {
            Case::CaughtLegacy
        } else {
            Case::CaughtNew
        });
        let failure = failure.unwrap_err();
        assert_dimension(&failure, ImportDimension::Tokens, 2, 1);
        assert_dimension(&failure, ImportDimension::Records, 1, 0);
        assert_eq!(observation.writes.load(Ordering::SeqCst), 2);
        assert_eq!(observation.next.load(Ordering::SeqCst), 0);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }

    #[test]
    fn exclusive_caught_numeric_panic_blocks_all_actual_entry_and_submit_routes() {
        for case in [
            Case::CaughtReturn,
            Case::CaughtNew,
            Case::CaughtLegacy,
            Case::CaughtSubmit,
            Case::CaughtSubmitComplete,
        ] {
            let (failure, observation) = run(case);
            let failure = failure.unwrap_err();
            assert_eq!(observation.writes.load(Ordering::SeqCst), 2);
            assert_eq!(observation.next.load(Ordering::SeqCst), 0);
            assert_eq!(observation.owned_drops.load(Ordering::SeqCst), 1);
            assert_dimension(&failure, ImportDimension::Tokens, 2, 1);
            assert_dimension(&failure, ImportDimension::Records, 1, 0);
            assert_eq!(observation.region_polls.load(Ordering::SeqCst), 1);
            let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
                panic!("caught numeric panic published success");
            };
            assert_eq!(terminal.class(), ImportTerminalFailureClass::ProductGraph);
            assert_eq!(terminal.code(), 0x4558_4142);
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }
    #[test]
    fn exclusive_opaque_reader_return_error_eof_and_caught_panic_keep_distinct_completion() {
        for case in [Case::IoReturn, Case::IoEof, Case::IoError, Case::IoPanic] {
            let (result, observation) = run(case);
            assert_eq!(observation.reader_calls.load(Ordering::SeqCst), 1);
            assert_eq!(observation.reader_drops.load(Ordering::SeqCst), 1);
            if case == Case::IoPanic {
                assert_eq!(scope_code(&result.unwrap_err()), "asset-read-panicked");
            } else {
                result.unwrap();
            }
        }
    }
    #[test]
    fn denied_compound_value_and_leaf_capture_latch_before_real_drop_panic() {
        for case in [Case::DeniedPushDrop, Case::DeniedCaptureDrop] {
            let (failure, observation) = run(case);
            let failure = failure.unwrap_err();
            assert_eq!(scope_code(&failure), "import-store-observed-work-overrun");
            assert_dimension(&failure, ImportDimension::Tokens, 2, 1);
            assert_dimension(&failure, ImportDimension::Records, 0, 0);
            assert_eq!(observation.writes.load(Ordering::SeqCst), 1);
            assert_eq!(observation.owned_drops.load(Ordering::SeqCst), 1);
            assert_eq!(observation.next.load(Ordering::SeqCst), 0);
            assert_eq!(observation.submitted.load(Ordering::SeqCst), 0);
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }
}
