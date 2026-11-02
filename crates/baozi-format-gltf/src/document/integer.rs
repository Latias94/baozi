use crate::GltfFailure;
use baozi_import::{ImportWorkClass, ParseExecution, ParseResult};

/// Interprets a number whose kind and complete JSON grammar were already admitted.
pub(super) fn unsigned(ctx: &mut ParseExecution<'_, '_>, bytes: &[u8]) -> ParseResult<u64> {
    let unsupported_length = |ctx: &mut ParseExecution<'_, '_>| {
        ctx.format_failure(GltfFailure::invalid_document(
            "glTF integer token exceeds the supported length domain",
        ))
    };
    let length = i128::try_from(bytes.len()).map_err(|_| unsupported_length(ctx))?;
    let exponent_bound = length
        .checked_add(21)
        .ok_or_else(|| unsupported_length(ctx))?;
    let out_of_range = |ctx: &mut ParseExecution<'_, '_>| {
        ctx.format_failure(GltfFailure::invalid_document(
            "glTF integer exceeds the U64 domain",
        ))
    };
    let fractional = |ctx: &mut ParseExecution<'_, '_>| {
        ctx.format_failure(GltfFailure::invalid_document(
            "glTF unsigned integer has a nonzero fractional part",
        ))
    };

    let mut negative = false;
    let mut fraction = false;
    let mut exponent = false;
    let mut exponent_negative = false;
    let mut exponent_magnitude = 0_i128;
    let mut exponent_over_bound = false;
    let mut mantissa_digits = 0_i128;
    let mut fractional_digits = 0_i128;
    let mut first_nonzero = None;
    let mut last_nonzero_end = 0;
    let mut last_nonzero_digit = 0_i128;
    let mut chunk_start = 0;

    let integer_work = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for chunk in bytes.chunks(4096) {
        ctx.poll()?;
        ctx.with_work(&integer_work, [chunk.len() as u64], |ctx, work| {
            for (offset, &byte) in chunk.iter().enumerate() {
                match byte {
                    b'-' => {
                        if exponent {
                            exponent_negative = true;
                        } else {
                            negative = true;
                        }
                    }
                    b'+' => {}
                    b'.' => fraction = true,
                    b'e' | b'E' => exponent = true,
                    b'0'..=b'9' => {
                        if exponent {
                            if !exponent_over_bound {
                                exponent_magnitude = exponent_magnitude
                                    .saturating_mul(10)
                                    .saturating_add(i128::from(byte - b'0'))
                                    .min(exponent_bound);
                                // N+21 is a sentinel, never an exact exponent.
                                exponent_over_bound = exponent_magnitude == exponent_bound;
                            }
                        } else {
                            if fraction {
                                fractional_digits += 1;
                            }
                            if byte != b'0' {
                                first_nonzero
                                    .get_or_insert((chunk_start + offset, mantissa_digits));
                                last_nonzero_end = chunk_start + offset + 1;
                                last_nonzero_digit = mantissa_digits;
                            }
                            mantissa_digits += 1;
                        }
                    }
                    _ => {
                        return Err(ctx.format_failure(GltfFailure::internal(
                            "unsigned interpreter requires an admitted number",
                        )));
                    }
                }
            }
            work.complete_all()
        })?;
        chunk_start += chunk.len();
    }

    let Some((first_byte, first_digit)) = first_nonzero else {
        return Ok(0);
    };
    if negative {
        return Err(ctx.format_failure(GltfFailure::invalid_document(
            "glTF unsigned integer is negative",
        )));
    }
    if exponent_over_bound {
        return Err(if exponent_negative {
            fractional(ctx)
        } else {
            out_of_range(ctx)
        });
    }
    let signed_exponent = if exponent_negative {
        -exponent_magnitude
    } else {
        exponent_magnitude
    };
    let trailing_zeros = mantissa_digits - last_nonzero_digit - 1;
    let shift = signed_exponent
        .checked_sub(fractional_digits)
        .and_then(|shift| shift.checked_add(trailing_zeros))
        .ok_or_else(|| unsupported_length(ctx))?;
    if shift < 0 {
        return Err(fractional(ctx));
    }
    let significant_digits = last_nonzero_digit - first_digit + 1;
    if significant_digits > 20 || shift > 20 - significant_digits {
        return Err(out_of_range(ctx));
    }

    // Trimming bounds the second pass to 21 bytes and at most 19 appended zeros.
    let coefficient = bytes.get(first_byte..last_nonzero_end).ok_or_else(|| {
        ctx.format_failure(GltfFailure::internal(
            "unsigned integer coefficient range is inconsistent",
        ))
    })?;
    let accumulation_work = u64::try_from(shift)
        .ok()
        .and_then(|shift| (coefficient.len() as u64).checked_add(shift))
        .ok_or_else(|| {
            ctx.format_failure(GltfFailure::internal(
                "unsigned integer work bound is inconsistent",
            ))
        })?;
    ctx.poll()?;
    ctx.with_work(
        &integer_work,
        [accumulation_work],
        |ctx, work| -> ParseResult<u64> {
            let mut value = 0_u64;
            for &byte in coefficient {
                if byte != b'.' {
                    value = value
                        .checked_mul(10)
                        .and_then(|value| value.checked_add(u64::from(byte - b'0')))
                        .ok_or_else(|| out_of_range(ctx))?;
                }
            }
            for _ in 0..shift {
                value = value.checked_mul(10).ok_or_else(|| out_of_range(ctx))?;
            }
            work.complete_all()?;
            Ok(value)
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::{GltfFailureCode, json_syntax};
    use baozi_import::{
        DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor,
        FormatFailureDetailValue, FormatSelection, ImportCause, ImportConfigOverlay,
        ImportDimension, ImportFailure, ImportOperationControl, ImportRequest, ImportRequestLayer,
        LedgerCause, OptionCompileLimits, ParseResult, Probe, ReadConfidence, ReadHint,
        RegistryBuildLimits, RegistryBuilder, ResourceLimits, ResourceLimitsOverlay,
        ScopeAbortCause,
    };
    use baozi_io::AssetPath;
    use baozi_operation::{CancellationOrigin, CancellationSignal, ControlCause};
    use std::error::Error;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    };

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    const INSPECTION_STOP: &str = "unsigned integer inspection completed";
    const CHUNK_BYTES: u64 = 4096;

    #[derive(Default)]
    struct Inspection {
        admitted: AtomicBool,
        interpreting: AtomicBool,
        completed: AtomicBool,
        value: AtomicU64,
        polls: AtomicUsize,
        cancelled: AtomicBool,
        cancel_on: Option<usize>,
    }

    struct IntegerCancellation(Arc<Inspection>);

    impl CancellationSignal for IntegerCancellation {
        fn is_cancelled(&self) -> bool {
            if self.0.cancelled.load(Ordering::SeqCst) {
                return true;
            }
            if !self.0.interpreting.load(Ordering::SeqCst) {
                return false;
            }
            let poll = self.0.polls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.0.cancel_on.is_some_and(|limit| poll >= limit) {
                self.0.cancelled.store(true, Ordering::SeqCst);
                return true;
            }
            false
        }
    }

    struct InspectInteger {
        inspection: Arc<Inspection>,
        interpret: bool,
    }

    impl Format for InspectInteger {
        type Options = baozi_import::NoFormatOptions;

        fn compile_defaults(
            &self,
            scope: &mut baozi_import::RegistryBuildScope<'_>,
        ) -> std::result::Result<
            baozi_import::CompiledOptions<Self::Options>,
            baozi_import::OptionCompileError,
        > {
            Self::Options::compile_defaults(scope)
        }

        fn descriptor(&self) -> FormatDescriptor {
            crate::format_descriptor()
        }

        fn detect(
            &self,
            _: &Probe<'_>,
            _: &ReadHint,
            _: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(
            &self,
            _: &Self::Options,
            context: &mut baozi_import::ParseContext<'_>,
        ) -> ParseResult<()> {
            context.with_execution(|scope| {
                let bytes = scope.primary_bytes();
                let (_map, index) = json_syntax::admit(scope, bytes, 0, false)?;
                let root = index.values.as_slice().first().expect("admitted root");
                assert!(root.kind == json_syntax::ValueKind::Number);
                self.inspection.admitted.store(true, Ordering::SeqCst);
                if self.interpret {
                    self.inspection.interpreting.store(true, Ordering::SeqCst);
                    let result = super::super::decode::Decoder {
                        bytes,
                        index: &index,
                    }
                    .uint(0, scope);
                    self.inspection.interpreting.store(false, Ordering::SeqCst);
                    self.inspection.value.store(result?, Ordering::SeqCst);
                    self.inspection.completed.store(true, Ordering::SeqCst);
                }
                // Stop without allocating format-failure evidence or charging unrelated work.
                scope.enforce_resource_limit(INSPECTION_STOP, 1, 0)?;
                unreachable!("the inspection-only resource bound must stop parsing")
            })
        }
    }

    fn inspect(
        text: &str,
        interpret: bool,
        limits: ResourceLimits,
        cancel_on: Option<usize>,
    ) -> TestResult<(ImportFailure, Arc<Inspection>)> {
        let inspection = Arc::new(Inspection {
            cancel_on,
            ..Inspection::default()
        });
        let mut registry = RegistryBuilder::new();
        registry.register(InspectInteger {
            inspection: Arc::clone(&inspection),
            interpret,
        })?;
        let request = registry.freeze(RegistryBuildLimits::default())?.plan(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(
                    ImportConfigOverlay::default()
                        .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
                        .with_format_selection(FormatSelection::Forced(
                            baozi_import::authoring::FormatId::new(crate::FORMAT_ID)?,
                        )),
                ),
            ),
            OptionCompileLimits::default(),
        )?;
        let failure = request
            .read_bytes(
                AssetPath::new("integer-inspection.gltf")?,
                text.as_bytes(),
                ImportOperationControl::default()
                    .with_cancellation(IntegerCancellation(Arc::clone(&inspection))),
            )
            .expect_err("integer inspection must never publish a scene")
            .into_operation_failure()?;
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
        assert_eq!(failure.ledger().committed_canonical_members(), 0);
        Ok((failure, inspection))
    }

    fn assert_format_failure(failure: &ImportFailure, code: GltfFailureCode, reason: &str) {
        let ImportCause::Format(envelope) = failure.cause() else {
            panic!("expected format failure, got {:?}", failure.cause());
        };
        assert_eq!(envelope.code(), code.as_str());
        assert!(envelope.details().iter().any(|detail| {
            detail.key() == "reason"
                && matches!(detail.value(), FormatFailureDetailValue::String(value) if value.as_str() == reason)
        }));
    }

    fn assert_stopped(failure: &ImportFailure) {
        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::ResourceLimit {
                    resource: INSPECTION_STOP,
                    limit: 0,
                })
        ));
    }

    fn assert_value(text: &str, expected: u64) -> TestResult {
        let (failure, inspection) = inspect(text, true, ResourceLimits::default(), None)?;
        assert!(inspection.admitted.load(Ordering::SeqCst), "{text}");
        assert!(inspection.completed.load(Ordering::SeqCst), "{text}");
        assert_eq!(inspection.value.load(Ordering::SeqCst), expected, "{text}");
        assert_stopped(&failure);
        Ok(())
    }

    fn assert_rejected(text: &str, reason: &str) -> TestResult {
        let (failure, inspection) = inspect(text, true, ResourceLimits::default(), None)?;
        assert!(inspection.admitted.load(Ordering::SeqCst), "{text}");
        assert!(!inspection.completed.load(Ordering::SeqCst), "{text}");
        assert_format_failure(&failure, GltfFailureCode::InvalidDocument, reason);
        Ok(())
    }

    fn work(failure: &ImportFailure) -> (u64, u64) {
        let counter = failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .expect("configured work dimension");
        (counter.attempted(), counter.consumed())
    }

    fn admission_work(text: &str) -> TestResult<u64> {
        let (failure, inspection) = inspect(text, false, ResourceLimits::default(), None)?;
        assert!(inspection.admitted.load(Ordering::SeqCst));
        assert!(!inspection.completed.load(Ordering::SeqCst));
        assert_eq!(inspection.polls.load(Ordering::SeqCst), 0);
        assert_stopped(&failure);
        let (attempted, consumed) = work(&failure);
        assert_eq!(attempted, consumed);
        Ok(consumed)
    }

    fn long_tokens() -> [(String, u64, u64); 4] {
        [
            (format!("1e+{}", "0".repeat(8192)), 1, 1),
            (format!("1{}e-8192", "0".repeat(8192)), 1, 1),
            (format!("-0e-{}", "9".repeat(8192)), 0, 0),
            (
                format!("18446744073709551615{}e-8192", "0".repeat(8192)),
                u64::MAX,
                20,
            ),
        ]
    }

    #[test]
    fn integral_decimal_and_exponent_spellings_are_exact() -> TestResult {
        for (text, expected) in [
            ("3", 3),
            ("3.0", 3),
            ("3e0", 3),
            ("3E+000", 3),
            ("0.1e1", 1),
            ("10e-1", 1),
            ("0.0001000e4", 1),
            ("123.4500e2", 12345),
            ("1234500e-2", 12345),
            ("100.001e3", 100001),
            ("1.2345678901234567890e19", 12345678901234567890),
            ("0.000000000000000000000001e24", 1),
            ("1e19", 10000000000000000000),
        ] {
            assert_value(text, expected)?;
        }
        Ok(())
    }

    #[test]
    fn all_zero_mantissas_ignore_sign_and_exponent_magnitude() -> TestResult {
        for text in [
            "0",
            "-0",
            "-0.000",
            "0e999",
            "-0.0e-18446744073709551616",
            "0.000e18446744073709551616",
            "-0e99999999999999999999999999999999999999999999",
        ] {
            assert_value(text, 0)?;
        }
        Ok(())
    }

    #[test]
    fn precision_boundaries_and_coefficient_cancellation_preserve_u64() -> TestResult {
        for (text, expected) in [
            ("9007199254740993", 9007199254740993),
            ("9007199254740993.0", 9007199254740993),
            ("90071992547409930e-1", 9007199254740993),
            ("18446744073709551614.0", u64::MAX - 1),
            ("18446744073709551615", u64::MAX),
            ("18446744073709551615.0", u64::MAX),
            ("184467440737095516150e-1", u64::MAX),
            ("184467440737095516150000e-4", u64::MAX),
            ("1.8446744073709551615e19", u64::MAX),
            ("0.18446744073709551615e20", u64::MAX),
        ] {
            assert_value(text, expected)?;
        }
        Ok(())
    }

    #[test]
    fn negative_nonzero_values_are_rejected_without_underflow() -> TestResult {
        for text in ["-1", "-3.0", "-1e-999", "-0.1e1"] {
            assert_rejected(text, "glTF unsigned integer is negative")?;
        }
        Ok(())
    }

    #[test]
    fn nonzero_fractions_are_rejected_before_any_rounding() -> TestResult {
        for text in [
            "3.0000000000000000001",
            "2.9999999999999999999",
            "12e-1",
            "1e-999",
            "1e-18446744073709551616",
            "9007199254740993.1",
            "18446744073709551615.1",
            "184467440737095516150e-2",
            "999999999999999999999e-1",
        ] {
            assert_rejected(text, "glTF unsigned integer has a nonzero fractional part")?;
        }
        Ok(())
    }

    #[test]
    fn out_of_range_exact_integers_are_rejected_as_u64_domain_failures() -> TestResult {
        for text in [
            "18446744073709551616",
            "18446744073709551616.0",
            "184467440737095516160e-1",
            "99999999999999999999",
            "1e20",
            "1e999",
            "1e18446744073709551616",
            "1e99999999999999999999999999999999999999999999",
        ] {
            assert_rejected(text, "glTF integer exceeds the U64 domain")?;
        }
        Ok(())
    }

    #[test]
    fn long_admitted_tokens_cancel_decimal_zeros_before_u64_accumulation() -> TestResult {
        for (text, expected, _) in long_tokens() {
            assert_value(&text, expected)?;
        }
        assert_value(&format!("0.{}1e8193", "0".repeat(8192)), 1)?;
        Ok(())
    }

    #[test]
    fn interpretation_work_counts_scanned_bytes_and_appended_zeros() -> TestResult {
        let mut tokens = long_tokens().to_vec();
        tokens.extend([
            ("3.0".to_owned(), 3, 1),
            ("1.23e2".to_owned(), 123, 4),
            ("1e19".to_owned(), 10000000000000000000, 20),
            ("-0.000".to_owned(), 0, 0),
        ]);
        for (text, expected, accumulation_work) in tokens {
            let baseline = admission_work(&text)?;
            let (failure, inspection) = inspect(&text, true, ResourceLimits::default(), None)?;
            assert!(inspection.completed.load(Ordering::SeqCst));
            assert_eq!(inspection.value.load(Ordering::SeqCst), expected);
            assert_stopped(&failure);
            let expected_work = baseline + u64::try_from(text.len())? + accumulation_work;
            assert_eq!(work(&failure), (expected_work, expected_work));
        }
        Ok(())
    }

    #[test]
    fn cancellation_is_polled_during_integer_interpretation_after_admission() -> TestResult {
        for (text, _, _) in long_tokens() {
            let baseline = admission_work(&text)?;
            let (calibration, completed) = inspect(&text, true, ResourceLimits::default(), None)?;
            assert!(completed.completed.load(Ordering::SeqCst));
            assert_stopped(&calibration);
            let polls = completed.polls.load(Ordering::SeqCst);
            assert!(polls > 1);
            let mut observed_mid_scan = false;
            for cancel_on in 1..=polls {
                let (failure, inspection) =
                    inspect(&text, true, ResourceLimits::default(), Some(cancel_on))?;
                assert!(inspection.admitted.load(Ordering::SeqCst));
                assert!(!inspection.completed.load(Ordering::SeqCst));
                assert_eq!(inspection.polls.load(Ordering::SeqCst), cancel_on);
                assert!(matches!(
                    failure.cause(),
                    ImportCause::Scope(abort)
                        if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled {
                            origin: CancellationOrigin::Signal,
                        }))
                ));
                let (attempted, consumed) = work(&failure);
                if cancel_on == 1 {
                    assert_eq!((attempted, consumed), (baseline, baseline));
                }
                let scanned = consumed - baseline;
                if scanned > 0 && scanned < u64::try_from(text.len())? {
                    assert_eq!(scanned % CHUNK_BYTES, 0);
                    observed_mid_scan = true;
                }
            }
            assert!(observed_mid_scan, "no cancellation between metadata chunks");
        }
        Ok(())
    }

    #[test]
    fn work_exhaustion_stops_the_next_metadata_chunk_after_admission() -> TestResult {
        for (text, _, _) in long_tokens() {
            let baseline = admission_work(&text)?;
            let mut limits = ResourceLimits::default();
            limits.max_work_units = baseline + CHUNK_BYTES;
            let (failure, inspection) = inspect(&text, true, limits, None)?;
            assert!(inspection.admitted.load(Ordering::SeqCst));
            assert!(!inspection.completed.load(Ordering::SeqCst));
            assert!(matches!(
                failure.cause(),
                ImportCause::Scope(abort)
                    if matches!(abort.cause(), ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                        dimension: ImportDimension::WorkUnits,
                        limit,
                        ..
                    }) if *limit == baseline + CHUNK_BYTES)
            ));
            assert_eq!(
                work(&failure),
                (baseline + CHUNK_BYTES * 2, baseline + CHUNK_BYTES)
            );
        }
        Ok(())
    }

    #[test]
    fn coefficient_and_appended_zero_work_require_the_exact_budget() -> TestResult {
        for (text, expected, accumulation_work) in [
            ("1.8446744073709551615e19", u64::MAX, 21),
            ("1e19", 10000000000000000000, 20),
        ] {
            let baseline = admission_work(text)?;
            let metadata_work = u64::try_from(text.len())?;
            let exact_work = baseline + metadata_work + accumulation_work;
            let mut limits = ResourceLimits::default();
            limits.max_work_units = exact_work - 1;
            let (failure, inspection) = inspect(text, true, limits.clone(), None)?;
            assert!(inspection.admitted.load(Ordering::SeqCst));
            assert!(!inspection.completed.load(Ordering::SeqCst));
            assert!(matches!(
                failure.cause(),
                ImportCause::Scope(abort)
                    if matches!(abort.cause(), ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                        dimension: ImportDimension::WorkUnits,
                        limit,
                        ..
                    }) if *limit == exact_work - 1)
            ));
            assert_eq!(work(&failure), (exact_work, baseline + metadata_work));

            limits.max_work_units = exact_work;
            let (failure, inspection) = inspect(text, true, limits, None)?;
            assert!(inspection.completed.load(Ordering::SeqCst));
            assert_eq!(inspection.value.load(Ordering::SeqCst), expected);
            assert_stopped(&failure);
            assert_eq!(work(&failure), (exact_work, exact_work));
        }
        Ok(())
    }

    #[test]
    fn token_length_admits_the_exact_limit_and_rejects_one_byte_over() -> TestResult {
        let text = format!("1e+{}", "0".repeat(8192));
        let mut limits = ResourceLimits::default();
        limits.max_token_bytes = text.len();
        let (failure, inspection) = inspect(&text, true, limits.clone(), None)?;
        assert!(inspection.completed.load(Ordering::SeqCst));
        assert_eq!(inspection.value.load(Ordering::SeqCst), 1);
        assert_stopped(&failure);

        limits.max_token_bytes -= 1;
        let expected_limit = u64::try_from(limits.max_token_bytes)?;
        let (failure, inspection) = inspect(&text, true, limits, None)?;
        assert!(!inspection.admitted.load(Ordering::SeqCst));
        assert!(!inspection.completed.load(Ordering::SeqCst));
        assert_eq!(inspection.polls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::ResourceLimit {
                    resource: "gltf-json-number-bytes",
                    limit,
                } if *limit == expected_limit)
        ));
        Ok(())
    }
}
