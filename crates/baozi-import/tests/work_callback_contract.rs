mod support;

use baozi_core::FormatId;
use baozi_import::{
    CapabilityStatus, CauseOwner, DetectionOutcome, DetectionResult, DetectionScope,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter, ImportCause,
    ImportDimension, ImportFailure, ImportOperationControl, ImportRequest, ImportResult,
    ImportWorkClass, OptionCompileLimits, ParseFailure, ParseResult, ParseScope,
    PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    SceneDraft, ScopeAbortCause, TrustedExtensionBoundary, import_bytes,
};
use baozi_io::AssetPath;
use baozi_operation::CancellationToken;
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
    Incomplete,
    Nested,
    ParentNestedSuccess,
    ParentChildFailure,
    ParentGroupNestedSuccess,
    ParentGroupChildFailure,
    CaughtParentPanic,
    ParentCancelBeforeCompletion,
    PrematureReplenish,
    ItemLimitedReplenish,
    GroupItemLimit,
    CallbackPanic,
    CaughtCallbackPanic,
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
            WorkCase::Incomplete => "dev.baozi.test.work-callback-incomplete",
            WorkCase::Nested => "dev.baozi.test.work-callback-nested",
            WorkCase::ParentNestedSuccess => "dev.baozi.test.work-parent-success",
            WorkCase::ParentChildFailure => "dev.baozi.test.work-parent-child-failure",
            WorkCase::ParentGroupNestedSuccess => "dev.baozi.test.work-parent-group-success",
            WorkCase::ParentGroupChildFailure => "dev.baozi.test.work-parent-group-child-failure",
            WorkCase::CaughtParentPanic => "dev.baozi.test.work-parent-caught-panic",
            WorkCase::ParentCancelBeforeCompletion => "dev.baozi.test.work-parent-cancel",
            WorkCase::PrematureReplenish => "dev.baozi.test.work-callback-replenish",
            WorkCase::ItemLimitedReplenish => "dev.baozi.test.work-callback-item-limit",
            WorkCase::GroupItemLimit => "dev.baozi.test.work-callback-group-limit",
            WorkCase::CallbackPanic => "dev.baozi.test.work-callback-panic",
            WorkCase::CaughtCallbackPanic => "dev.baozi.test.work-callback-caught-panic",
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
            WorkCase::Incomplete => "work-incomplete",
            WorkCase::Nested => "work-nested",
            WorkCase::ParentNestedSuccess => "work-parent-success",
            WorkCase::ParentChildFailure => "work-parent-failure",
            WorkCase::ParentGroupNestedSuccess => "work-parent-group-success",
            WorkCase::ParentGroupChildFailure => "work-parent-group-failure",
            WorkCase::CaughtParentPanic => "work-parent-caught-panic",
            WorkCase::ParentCancelBeforeCompletion => "work-parent-cancel",
            WorkCase::PrematureReplenish => "work-replenish",
            WorkCase::ItemLimitedReplenish => "work-item-limit",
            WorkCase::GroupItemLimit => "work-group-limit",
            WorkCase::CallbackPanic => "work-panic",
            WorkCase::CaughtCallbackPanic => "work-caught-panic",
            WorkCase::DuplicateDimension => "work-duplicate",
            WorkCase::CallbackErrorCancel => "work-error-cancel",
            WorkCase::IncompleteCancel => "work-incomplete-cancel",
            WorkCase::CancelAfterCompletion => "work-cancel",
        }
    }
}

impl FormatImporter for WorkImporter {
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
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        match self.case {
            WorkCase::EntryFailure => {
                return Err(scope.resource_limit_exceeded("work-callback-entry", 0));
            }
            WorkCase::ExactGroup => {
                scope.with_work_group(
                    [
                        (ImportWorkClass::Tokens, 3),
                        (ImportWorkClass::WorkUnits, 2),
                    ],
                    |work| {
                        assert_eq!(work.remaining(), [3, 2]);
                        work.complete([1, 1])?;
                        work.run([2, 1], || Ok(()))?;
                        assert_eq!(work.remaining(), [0, 0]);
                        Ok(())
                    },
                )?;
            }
            WorkCase::ZeroGroup => {
                scope.with_work_group(
                    [
                        (ImportWorkClass::WorkUnits, 0),
                        (ImportWorkClass::Tokens, 0),
                    ],
                    |work| {
                        assert_eq!(work.remaining(), [0, 0]);
                        work.complete_all()
                    },
                )?;
            }
            WorkCase::DynamicChunks => {
                scope.with_work(ImportWorkClass::WorkUnits, 0, |work| {
                    work.replenish(4)?;
                    work.complete_all()?;
                    work.replenish(4)?;
                    work.complete_all()
                })?;
            }
            WorkCase::CallbackError => {
                scope.with_work_group(
                    [
                        (ImportWorkClass::WorkUnits, 2),
                        (ImportWorkClass::Tokens, 2),
                    ],
                    |work| {
                        work.complete([1, 1])?;
                        scope.enforce_resource_limit("work-callback-error", 2, 1)?;
                        Ok(())
                    },
                )?;
            }
            WorkCase::Incomplete => {
                scope.with_work_group(
                    [
                        (ImportWorkClass::WorkUnits, 2),
                        (ImportWorkClass::Tokens, 2),
                    ],
                    |work| {
                        work.complete([1, 1])?;
                        Ok(())
                    },
                )?;
            }
            WorkCase::Nested => {
                scope.with_work(ImportWorkClass::WorkUnits, 0, |work| {
                    work.replenish(4)?;
                    assert!(
                        scope
                            .with_work(ImportWorkClass::Tokens, 1, |nested| {
                                nested.complete_all()
                            })
                            .is_err()
                    );
                    work.complete_all()
                })?;
            }
            WorkCase::ParentNestedSuccess => {
                scope.with_parent_work(ImportWorkClass::Records, 1, || {
                    scope.with_work(ImportWorkClass::Tokens, 2, |child| child.complete_all())
                })?;
            }
            WorkCase::ParentChildFailure => {
                scope.with_parent_work(ImportWorkClass::Records, 1, || {
                    scope.with_work(ImportWorkClass::WorkUnits, 2, |child| {
                        child.complete([1])?;
                        scope.enforce_resource_limit("parent-child-failure", 2, 1)
                    })
                })?;
            }
            WorkCase::ParentGroupNestedSuccess => {
                scope.with_parent_work_group(
                    [
                        (ImportWorkClass::Records, 1),
                        (ImportWorkClass::AttributeCells, 2),
                    ],
                    || scope.with_work(ImportWorkClass::Tokens, 3, |child| child.complete_all()),
                )?;
            }
            WorkCase::ParentGroupChildFailure => {
                scope.with_parent_work_group(
                    [
                        (ImportWorkClass::Records, 1),
                        (ImportWorkClass::AttributeCells, 2),
                    ],
                    || {
                        scope.with_work(ImportWorkClass::WorkUnits, 2, |child| {
                            child.complete([1])?;
                            scope.enforce_resource_limit("parent-group-child-failure", 2, 1)
                        })
                    },
                )?;
            }
            WorkCase::CaughtParentPanic => {
                let caught = catch_unwind(AssertUnwindSafe(|| {
                    scope.with_parent_work(ImportWorkClass::Records, 1, || -> ParseResult<()> {
                        panic!("caught parent work panic")
                    })
                }));
                assert!(caught.is_err());
                scope.with_parent_work(ImportWorkClass::Records, 1, || {
                    scope.with_work(ImportWorkClass::Tokens, 1, |child| child.complete_all())
                })?;
            }
            WorkCase::ParentCancelBeforeCompletion => {
                let cancellation = self
                    .cancellation
                    .as_ref()
                    .expect("parent cancellation case owns a token");
                scope.with_parent_work(ImportWorkClass::Records, 1, || {
                    cancellation.cancel();
                    Ok(())
                })?;
            }
            WorkCase::PrematureReplenish => {
                scope.with_work(ImportWorkClass::WorkUnits, 2, |work| {
                    work.replenish(1)?;
                    Ok(())
                })?;
            }
            WorkCase::ItemLimitedReplenish => {
                scope.with_work(ImportWorkClass::StringBytes, 0, |work| {
                    work.replenish(1)?;
                    Ok(())
                })?;
            }
            WorkCase::GroupItemLimit => {
                let line_bytes = u64::try_from(scope.limits().max_line_bytes)
                    .expect("line-byte limit fits the import counter")
                    .checked_add(1)
                    .expect("test line-byte excess fits the import counter");
                scope.with_work_group(
                    [
                        (ImportWorkClass::LineBytes, line_bytes),
                        (ImportWorkClass::Tokens, 1),
                    ],
                    |work| work.complete_all(),
                )?;
            }
            WorkCase::CallbackPanic => {
                scope.with_work_group(
                    [
                        (ImportWorkClass::WorkUnits, 2),
                        (ImportWorkClass::Tokens, 2),
                    ],
                    |work| -> ParseResult<()> {
                        work.complete([1, 1])?;
                        panic!("work callback panic")
                    },
                )?;
            }
            WorkCase::CaughtCallbackPanic => {
                let caught = catch_unwind(AssertUnwindSafe(|| {
                    scope.with_work(ImportWorkClass::WorkUnits, 2, |work| -> ParseResult<()> {
                        work.complete([1])?;
                        panic!("caught work callback panic")
                    })
                }));
                assert!(caught.is_err());
                scope.with_work(ImportWorkClass::Tokens, 1, |work| work.complete_all())?;
            }
            WorkCase::DuplicateDimension => {
                scope.with_work_group(
                    [
                        (ImportWorkClass::WorkUnits, 2),
                        (ImportWorkClass::WorkUnits, 3),
                    ],
                    |work| {
                        work.complete([1, 2])?;
                        work.complete_all()
                    },
                )?;
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
                let result = scope.with_work(
                    ImportWorkClass::WorkUnits,
                    2,
                    |work| -> Result<(), CallbackError> {
                        work.complete([1])?;
                        cancellation.cancel();
                        Err(CallbackError::Requested)
                    },
                );
                assert!(matches!(result, Err(CallbackError::Requested)));
                scope.poll()?;
            }
            WorkCase::IncompleteCancel => {
                let cancellation = self
                    .cancellation
                    .as_ref()
                    .expect("incomplete cancellation case owns a token");
                scope.with_work(ImportWorkClass::WorkUnits, 2, |work| {
                    work.complete([1])?;
                    cancellation.cancel();
                    Ok(())
                })?;
            }
            WorkCase::CancelAfterCompletion => {
                let cancellation = self
                    .cancellation
                    .as_ref()
                    .expect("cancellation case owns a token");
                scope.with_work(ImportWorkClass::WorkUnits, 2, |work| {
                    work.complete_all()?;
                    cancellation.cancel();
                    Ok(())
                })?;
            }
        }
        support::submit_minimal_scene(scope)
    }
}

fn request_for(importer: WorkImporter) -> PreparedImportRequest {
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer)
        .expect("work callback importer registers");
    let registry = builder
        .freeze(RegistryBuildLimits::default())
        .expect("work callback registry freezes");
    registry
        .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
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
    operation_result(import_bytes(
        &request,
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
fn nested_work_is_rejected_before_inner_admission() {
    let failure = run_case(WorkCase::Nested, None).expect_err("nested work cannot start");
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::DescriptorContract {
                    code: "parse-work-already-in-progress",
                }
            )
    ));
    assert_dimension(&failure, ImportDimension::WorkUnits, 4, 0);
    assert_dimension(&failure, ImportDimension::Tokens, 0, 0);
}

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
fn a_caught_parent_panic_releases_the_parent_and_child_channels() {
    let report = run_case(WorkCase::CaughtParentPanic, None)
        .expect("a locally caught parent panic leaves both work channels reusable");
    let records = report
        .ledger()
        .dimension(ImportDimension::Records)
        .expect("parent record evidence");
    let tokens = report
        .ledger()
        .dimension(ImportDimension::Tokens)
        .expect("child token evidence");
    assert_eq!((records.attempted(), records.consumed()), (2, 1));
    assert_eq!((tokens.attempted(), tokens.consumed()), (1, 1));
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
                ScopeAbortCause::DescriptorContract {
                    code: "parse-work-replenish-before-completion",
                }
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
                ScopeAbortCause::DescriptorContract {
                    code: "parse-work-item-limit-replenish-unsupported",
                }
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
fn a_caught_callback_panic_releases_the_scope_for_the_next_action() {
    let report = run_case(WorkCase::CaughtCallbackPanic, None)
        .expect("a locally caught callback panic leaves the scope reusable");
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
        (2, 1)
    );
    let tokens = report
        .ledger()
        .dimension(ImportDimension::Tokens)
        .expect("token evidence");
    assert_eq!((tokens.attempted(), tokens.consumed()), (1, 1));
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
    let failure = operation_result(import_bytes(
        &request,
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
    let failure = operation_result(import_bytes(
        &request,
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
    let failure = operation_result(import_bytes(
        &request,
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
    let failure = operation_result(import_bytes(
        &request,
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
