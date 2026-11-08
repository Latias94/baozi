use super::Decoder;
use crate::{GltfFailure, GltfFailureCode};
use baozi_import::authoring::FormatId;
use baozi_import::{
    CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor,
    FormatFailureDetailValue, FormatSelection, ImportCause, ImportConfigOverlay, ImportDimension,
    ImportFailure, ImportOperationControl, ImportRequest, ImportRequestLayer, ImportTerminalCause,
    ImportTerminalFailureClass, ImportWorkClass, LedgerCause, NoFormatOptions, OptionCompileError,
    OptionCompileLimits, ParseContext, ParseResult, Probe, ReadConfidence, ReadHint,
    RegistryBuildLimits, RegistryBuildScope, RegistryBuilder, ResourceLimits,
    ResourceLimitsOverlay, ScopeAbortCause,
};
use baozi_io::AssetPath;
use baozi_operation::CounterClass;
use std::error::Error;
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

const ESCAPED_NAME: &[u8] = br#"{"na\u006de":0}"#;
const INSPECTION_COMPLETED: &str = "repeated field lookup inspection completed";

#[derive(Default)]
struct LookupObservation {
    admission_work: AtomicU64,
    completed_queries: AtomicUsize,
}

struct InspectFieldLookups {
    observation: Arc<LookupObservation>,
}

impl Format for InspectFieldLookups {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        Self::Options::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        crate::format_descriptor()
    }

    fn detect(
        &self,
        _: &Probe<'_>,
        _: &ReadHint,
        _: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, _: &Self::Options, context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
        context.with_execution(|root| {
            let (bytes, _, _source_map, index) = crate::admission::admit(root)?.into_parts();
            assert_eq!(bytes, ESCAPED_NAME);
            let baseline = root.work_counters(ImportWorkClass::WorkUnits)?;
            assert_eq!(baseline.0, baseline.1);
            assert_eq!(baseline.2, 0);
            self.observation
                .admission_work
                .store(baseline.0, Ordering::SeqCst);
            let decoder = Decoder {
                bytes,
                index: &index,
            };

            let first = decoder.field(0, "name", root)?;
            assert_eq!(first, Some(1));
            self.observation
                .completed_queries
                .fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                root.work_counters(ImportWorkClass::WorkUnits)?,
                (baseline.0 + 12, baseline.1 + 12, 0)
            );

            match decoder.field(0, "name", root) {
                Ok(second) => {
                    assert_eq!(second, first);
                    self.observation
                        .completed_queries
                        .fetch_add(1, Ordering::SeqCst);
                    assert_eq!(
                        root.work_counters(ImportWorkClass::WorkUnits)?,
                        (baseline.0 + 24, baseline.1 + 24, 0)
                    );
                    Err(root.format_failure(GltfFailure::internal(INSPECTION_COMPLETED)))
                }
                Err(failure) => {
                    assert_eq!(
                        root.work_counters(ImportWorkClass::WorkUnits)?,
                        (baseline.0 + 24, baseline.1 + 12, 0)
                    );
                    Err(failure)
                }
            }
        })
    }
}

fn inspect_field_lookups(
    limits: ResourceLimits,
    observation: Arc<LookupObservation>,
) -> Result<ImportFailure, Box<dyn Error>> {
    let mut registry = RegistryBuilder::new();
    registry.register(InspectFieldLookups { observation })?;
    let request = registry.freeze(RegistryBuildLimits::default())?.plan(
        &ImportRequest::from_layers(
            ImportRequestLayer::default(),
            ImportRequestLayer::new(
                ImportConfigOverlay::default()
                    .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
                    .with_format_selection(FormatSelection::Forced(FormatId::new(
                        crate::FORMAT_ID,
                    )?)),
            ),
        ),
        OptionCompileLimits::default(),
    )?;
    let failure = request
        .read_bytes(
            AssetPath::new("repeated-field.gltf")?,
            ESCAPED_NAME,
            ImportOperationControl::default(),
        )
        .expect_err("field lookup inspection stops before scene construction")
        .into_operation_failure()?;
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    Ok(failure)
}

#[test]
fn repeated_escaped_field_lookup_preserves_attempted_limit_and_consumed_prefix()
-> Result<(), Box<dyn Error>> {
    let calibration = Arc::new(LookupObservation::default());
    let reference = inspect_field_lookups(ResourceLimits::default(), Arc::clone(&calibration))?;
    assert_eq!(calibration.completed_queries.load(Ordering::SeqCst), 2);
    let ImportCause::Format(envelope) = reference.cause() else {
        panic!(
            "expected the completed lookup inspection, got {:?}",
            reference.cause()
        );
    };
    assert_eq!(envelope.format().as_str(), crate::FORMAT_ID);
    assert_eq!(envelope.code(), GltfFailureCode::InternalInvariant.as_str());
    assert!(envelope.details().iter().any(|detail| {
        detail.key() == "reason"
            && matches!(detail.value(), FormatFailureDetailValue::String(reason)
                if reason.as_str() == INSPECTION_COMPLETED)
    }));

    let baseline = calibration.admission_work.load(Ordering::SeqCst);
    let limit = baseline + 12;
    let limited = Arc::new(LookupObservation::default());
    let mut lookup_limits = ResourceLimits::default();
    lookup_limits.max_work_units = limit;
    let failure = inspect_field_lookups(lookup_limits, Arc::clone(&limited))?;
    assert_eq!(limited.admission_work.load(Ordering::SeqCst), baseline);
    assert_eq!(limited.completed_queries.load(Ordering::SeqCst), 1);
    let work = failure
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("the actual import ledger contains WorkUnits");
    assert_eq!(
        (work.attempted(), work.consumed()),
        (baseline + 24, baseline + 12)
    );
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!(
            "expected the second lookup's admission limit, got {:?}",
            failure.cause()
        );
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: ImportDimension::WorkUnits,
            value,
            limit: actual_limit,
        }) if *value == baseline + 24 && *actual_limit == limit
    ));
    let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
        panic!("the rejected lookup must retain a failure terminal");
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
    Ok(())
}
