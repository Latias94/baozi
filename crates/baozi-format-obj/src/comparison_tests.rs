use super::*;
use baozi_import::{
    CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor,
    ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure, ImportOperationControl,
    ImportRequest, ImportRequestLayer, ImportTerminalFailureClass, NoFormatOptions,
    OptionCompileError, OptionCompileLimits, ParseContext, Probe, ReadConfidence, ReadHint,
    RegistryBuildLimits, RegistryBuildScope, RegistryBuilder, ResourceLimits,
    ResourceLimitsOverlay, ScopeAbortCause,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationToken, ControlCause};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
};

type ComparisonAction = for<'region, 'session> fn(
    &mut ParseExecution<'region, 'session>,
    &AtomicUsize,
    &CancellationToken,
) -> ParseResult<()>;

// Obtain real parser channels and terminal ledger evidence through the public import API.
struct ComparisonProbe {
    action: ComparisonAction,
    physical: Arc<AtomicUsize>,
    token: CancellationToken,
}

impl Format for ComparisonProbe {
    type Options = NoFormatOptions;

    fn descriptor(&self) -> FormatDescriptor {
        crate::format_descriptor()
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
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, _options: &Self::Options, context: &mut ParseContext<'_>) -> ParseResult<()> {
        context.with_execution(|exec| {
            (self.action)(exec, &self.physical, &self.token)?;
            // Stop before scene submission so the ledger isolates the comparison work.
            Err(exec.format_failure(crate::ObjFailure::internal("comparison-test-stop")))
        })
    }
}

fn run(action: ComparisonAction, max_work_units: Option<u64>) -> (ImportFailure, usize) {
    let physical = Arc::new(AtomicUsize::new(0));
    let token = CancellationToken::new();
    let mut registry = RegistryBuilder::new();
    registry
        .register(ComparisonProbe {
            action,
            physical: Arc::clone(&physical),
            token: token.clone(),
        })
        .unwrap();
    let mut limits = ResourceLimits::default();
    if let Some(limit) = max_work_units {
        limits.max_work_units = limit;
    }
    let config = ImportConfigOverlay::default()
        .with_resource_limits(ResourceLimitsOverlay::from_limits(limits));
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(config));
    let plan = registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(&request, OptionCompileLimits::default())
        .unwrap();
    let failure = plan
        .read_bytes(
            AssetPath::new("comparison.obj").unwrap(),
            b"comparison fixture",
            ImportOperationControl::default().with_cancellation(token),
        )
        .expect_err("the comparison probe always returns terminal ledger evidence")
        .into_operation_failure()
        .expect("ordinary comparison failure");
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    (failure, physical.load(AtomicOrdering::Relaxed))
}

fn counts(failure: &ImportFailure) -> (u64, u64) {
    let row = failure
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .unwrap();
    (row.attempted(), row.consumed())
}

fn baseline() -> u64 {
    let (failure, _) = run(|_, _, _| Ok(()), None);
    assert!(matches!(failure.cause(), ImportCause::Format(_)));
    let (attempted, consumed) = counts(&failure);
    assert_eq!(attempted, consumed);
    consumed
}

fn non_format_baseline() -> u64 {
    let (failure, _) = run(
        |exec, _, token| {
            token.cancel();
            exec.poll()
        },
        None,
    );
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled { .. }))
    ));
    let (attempted, consumed) = counts(&failure);
    assert_eq!(attempted, consumed);
    // Control and ledger stops never construct the format-failure evidence used by baseline().
    consumed
}

#[test]
fn small_numeric_sort_and_search_consume_only_physical_comparisons() {
    let before = baseline();
    let (failure, physical) = run(
        |exec, physical, _| {
            let mut values = exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "comparison-test-numbers",
            );
            values.try_extend_from_slice(exec, &[3, 1, 2])?;
            interruptible_sort_by(exec, &mut values, |exec, work, left, right| {
                work.run_leaf(exec, || {
                    let ordering = left.cmp(right);
                    physical.fetch_add(1, AtomicOrdering::Relaxed);
                    ordering
                })
            })?;
            assert_eq!(values.as_slice(), &[1, 2, 3]);
            let found = interruptible_binary_search_by(exec, &[2], |exec, work, value| {
                work.run_leaf(exec, || {
                    let ordering = value.cmp(&2);
                    physical.fetch_add(1, AtomicOrdering::Relaxed);
                    ordering
                })
            })?;
            assert_eq!(found, Some(0));
            Ok(())
        },
        None,
    );
    assert!(matches!(failure.cause(), ImportCause::Format(_)));
    assert!(physical > 1);
    let expected = before + physical as u64;
    assert_eq!(counts(&failure), (expected, expected));
}

#[test]
fn short_string_searches_count_bytes_and_length_without_outer_charges() {
    let before = baseline();
    let (failure, _) = run(
        |exec, _, _| {
            for (left, right, ordering) in [
                ("", "", Ordering::Equal),
                ("a", "b", Ordering::Less),
                ("cat", "cat", Ordering::Equal),
                ("cat", "catalog", Ordering::Less),
                ("\u{e9}", "\u{e9}", Ordering::Equal),
            ] {
                let found = interruptible_binary_search_by(exec, &[left], |exec, work, value| {
                    let actual = work.compare_str(exec, value, right)?;
                    assert_eq!(actual, ordering);
                    Ok(actual)
                })?;
                assert_eq!(found, (ordering == Ordering::Equal).then_some(0));
            }
            Ok(())
        },
        None,
    );
    assert!(matches!(failure.cause(), ImportCause::Format(_)));
    // Empty length, mismatch, equal bytes + length, prefix + length, UTF-8 bytes + length.
    let expected = before + 1 + 1 + 4 + 4 + 3;
    assert_eq!(counts(&failure), (expected, expected));
}

#[test]
fn tie_break_and_dedup_charge_each_actual_leaf_and_duplicate_drop() {
    let before = baseline();
    let (failure, _) = run(
        |exec, _, _| {
            let mut values = exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "comparison-test-ties",
            );
            values.try_extend_from_slice(exec, &[("cat", 2), ("cat", 1)])?;
            interruptible_sort_by(exec, &mut values, |exec, work, left, right| {
                match work.compare_str(exec, left.0, right.0)? {
                    Ordering::Equal => work.run_leaf(exec, || left.1.cmp(&right.1)),
                    ordering => Ok(ordering),
                }
            })?;
            interruptible_dedup_by(exec, &mut values, |exec, work, previous, current| {
                Ok(work.compare_str(exec, previous.0, current.0)? == Ordering::Equal)
            })?;
            assert_eq!(values.as_slice(), &[("cat", 1)]);
            Ok(())
        },
        None,
    );
    assert!(matches!(failure.cause(), ImportCause::Format(_)));
    let expected = before + 4 + 1 + 4 + 1;
    assert_eq!(counts(&failure), (expected, expected));
}

#[test]
fn comparator_failure_keeps_the_already_completed_string_prefix() {
    let before = baseline();
    let (failure, _) = run(
        |exec, _, _| {
            interruptible_binary_search_by(exec, &["cat"], |exec, work, value| {
                assert_eq!(work.compare_str(exec, value, "cat")?, Ordering::Equal);
                assert_eq!(work.work.remaining(), [0]);
                Err(exec.format_failure(crate::ObjFailure::internal("comparison-test-failure")))
            })?;
            unreachable!("the comparator must fail")
        },
        None,
    );
    assert!(matches!(failure.cause(), ImportCause::Format(_)));
    assert_eq!(counts(&failure), (before + 4, before + 4));
}

fn numeric_leaves(
    exec: &mut ParseExecution<'_, '_>,
    physical: &AtomicUsize,
    token: &CancellationToken,
) -> ParseResult<()> {
    let group = exec.bind_work_group([ImportWorkClass::WorkUnits])?;
    exec.with_work(&group, [0], |exec, ledger_work| {
        let mut work = ComparisonWork::new(ledger_work);
        for value in 0..130 {
            work.run_leaf(exec, || {
                let ordering = value.cmp(&7);
                let completed = physical.fetch_add(1, AtomicOrdering::Relaxed) + 1;
                if completed == 7 {
                    token.cancel();
                }
                ordering
            })?;
        }
        Ok(())
    })
}

#[test]
fn denied_admission_never_executes_or_consumes_the_next_numeric_or_byte_leaf() {
    let before = non_format_baseline();
    let string_leaves: ComparisonAction = |exec, _, _| {
        interruptible_binary_search_by(exec, &["abcdefgh"], |exec, work, value| {
            work.compare_str(exec, value, "abcdefgh")
        })?;
        Ok(())
    };
    let length_leaf: ComparisonAction = |exec, _, _| {
        interruptible_binary_search_by(exec, &["cat"], |exec, work, value| {
            work.compare_str(exec, value, "cat")
        })?;
        Ok(())
    };
    for (action, expected_physical) in [
        (numeric_leaves as ComparisonAction, 3),
        (string_leaves, 0),
        (length_leaf, 0),
    ] {
        let (failure, physical) = run(action, Some(before + 3));
        match failure.cause() {
            ImportCause::Scope(abort) => {
                assert!(matches!(abort.cause(), ScopeAbortCause::Ledger(_)));
            }
            ImportCause::OperationTerminal(terminal) => {
                assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
            }
            cause => panic!("expected comparison admission failure, got {cause:?}"),
        }
        assert_eq!(physical, expected_physical);
        assert_eq!(counts(&failure), (before + 4, before + 3));
    }
}

#[test]
fn numeric_cancellation_consumes_only_the_physical_prefix_before_the_poll() {
    let before = non_format_baseline();
    let (failure, physical) = run(numeric_leaves, None);
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled { .. }))
    ));
    assert_eq!(physical, 64);
    assert_eq!(counts(&failure), (before + 64, before + 64));
}

#[test]
fn nested_string_cancellation_completes_bytes_without_an_unfinished_outer_leaf() {
    let before = non_format_baseline();
    let (failure, physical) = run(
        |exec, physical, token| {
            let group = exec.bind_work_group([ImportWorkClass::WorkUnits])?;
            exec.with_work(&group, [0], |exec, ledger_work| {
                let mut work = ComparisonWork::new(ledger_work);
                work.run_leaf(exec, || {
                    let ordering = 1.cmp(&1);
                    physical.fetch_add(1, AtomicOrdering::Relaxed);
                    token.cancel();
                    ordering
                })?;
                assert_eq!(work.work.remaining(), [0]);
                let bytes = [b'x'; 128];
                let text = std::str::from_utf8(&bytes).unwrap();
                work.compare_str(exec, text, text)?;
                Ok(())
            })
        },
        None,
    );
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled { .. }))
    ));
    assert_eq!(physical, 1);
    // One completed scalar comparison and 63 bytes precede the first comparison poll.
    assert_eq!(counts(&failure), (before + 64, before + 64));
}

#[test]
fn chunk_cancellation_preserves_completed_items_before_a_later_push_fails() {
    let before = non_format_baseline();
    let (failure, physical) = run(
        |exec, physical, token| {
            let mut rows = exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "comparison-test-source-rows",
            );
            rows.try_reserve(exec, 64)?;
            for_work_chunks(
                exec,
                130,
                "comparison-test-source-row-work",
                |exec, work, range| {
                    for row in range {
                        rows.try_push(exec, row)?;
                        let completed = physical.fetch_add(1, AtomicOrdering::Relaxed) + 1;
                        work.complete([1])?;
                        if completed == 3 {
                            token.cancel();
                        }
                    }
                    Ok(())
                },
            )
        },
        None,
    );
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled { .. }))
    ));
    assert_eq!(physical, 3);
    assert_eq!(counts(&failure), (before + 64, before + 3));
}
