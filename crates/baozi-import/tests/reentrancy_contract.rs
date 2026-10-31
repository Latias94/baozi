mod support;

use baozi_core::FormatId;
use baozi_import::authoring::SceneSpace;
use baozi_import::{
    CapabilityStatus, CompiledFormatOptions, DetectionOutcome, DetectionResult, DetectionScope,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter, FormatOptionCodec,
    FormatOptionKey, FormatOptionLayers, FormatOptionSchema, FormatOptionSchemaId,
    FormatOptionSchemaKey, FormatSelection, ImportConfigOverlay, ImportOperationControl,
    ImportReport, ImportRequest, ImportRequestLayer, OptionCompileError, OptionCompileLimits,
    OptionCompileScope, OptionRuntimeTypeKey, ParseResult, ParseScope, PreparedImportRequest,
    ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuildScope, RegistryBuilder,
    RegistrySnapshot, SceneDraft, import_bytes,
};
use baozi_io::AssetPath;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

const FORMAT_ID: &str = "dev.baozi.test.parser-reentrancy";
const FORMAT_EXTENSION: &str = "parser-reentrancy";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReentrantOptions {
    scale: u8,
}

#[derive(Debug)]
struct ReentrantCodec {
    schema: FormatOptionSchema,
}

impl FormatOptionCodec for ReentrantCodec {
    fn schema(&self) -> Option<FormatOptionSchema> {
        Some(self.schema.clone())
    }

    fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
        OptionRuntimeTypeKey::of::<ReentrantOptions>()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        scope.charge_work(1)?;
        scope.finish(ReentrantOptions { scale: 1 }, 1)
    }

    fn compile_layers(
        &self,
        scope: &mut OptionCompileScope<'_>,
        defaults: &CompiledFormatOptions,
        layers: FormatOptionLayers<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        scope.charge_work(1)?;
        let key = FormatOptionKey::<ReentrantOptions>::new(self.schema.clone());
        let mut scale = defaults
            .downcast_ref(&key)
            .ok_or_else(|| {
                OptionCompileError::invalid_payload(
                    "default-type",
                    "default parser options have the wrong runtime type",
                )
            })?
            .scale;
        if let Some(payload) = layers.preset() {
            scale = read_scale(scope, payload)?;
        }
        if let Some(payload) = layers.caller() {
            scale = read_scale(scope, payload)?;
        }
        scope.finish(ReentrantOptions { scale }, 1)
    }
}

fn read_scale(
    scope: &mut OptionCompileScope<'_>,
    payload: baozi_import::FormatOptionPayload<'_>,
) -> Result<u8, OptionCompileError> {
    match scope.reader(payload).read_remaining()? {
        [scale] if *scale != 0 => Ok(*scale),
        _ => Err(OptionCompileError::invalid_payload(
            "scale",
            "parser reentrancy scale must be one nonzero byte",
        )),
    }
}

struct ParserOverlap {
    first_pair: Barrier,
    entries: AtomicUsize,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
}

impl ParserOverlap {
    fn new() -> Self {
        Self {
            first_pair: Barrier::new(2),
            entries: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    fn enter(&self) -> ParserPermit<'_> {
        let entry = self.entries.fetch_add(1, Ordering::AcqRel);
        let current = self.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
        self.peak.fetch_max(current, Ordering::AcqRel);
        let permit = ParserPermit { overlap: self };
        if entry < 2 {
            self.first_pair.wait();
        }
        permit
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }
}

struct ParserPermit<'a> {
    overlap: &'a ParserOverlap,
}

impl Drop for ParserPermit<'_> {
    fn drop(&mut self) {
        self.overlap.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ReentrantImporter {
    descriptor: FormatDescriptor,
    option_key: FormatOptionKey<ReentrantOptions>,
    codec: Arc<dyn FormatOptionCodec>,
    overlap: Arc<ParserOverlap>,
    observations: Arc<Mutex<Vec<ParseObservation>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ParseObservation {
    source: String,
    input: u8,
    scale: u8,
    scaled_input: u16,
}

impl FormatImporter for ReentrantImporter {
    fn descriptor(&self) -> FormatDescriptor {
        self.descriptor.clone()
    }

    fn option_codec(&self) -> Arc<dyn FormatOptionCodec> {
        Arc::clone(&self.codec)
    }

    fn option_runtime_type_key(&self) -> OptionRuntimeTypeKey {
        OptionRuntimeTypeKey::of::<ReentrantOptions>()
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
        let _permit = self.overlap.enter();
        let scale = scope.options(&self.option_key)?.scale;
        let input = *scope
            .primary_bytes()
            .first()
            .expect("parser reentrancy fixtures are nonempty");
        self.observations
            .lock()
            .expect("parser observation mutex remains usable")
            .push(ParseObservation {
                source: scope.source().as_str().to_owned(),
                input,
                scale,
                scaled_input: u16::from(input) * u16::from(scale),
            });
        support::submit_scene_with_space(
            scope,
            SceneSpace {
                unit_scale_to_meters: Some(f64::from(input) * f64::from(scale)),
                ..SceneSpace::default()
            },
        )
    }
}

type RegistryFixture = (
    RegistrySnapshot,
    FormatOptionKey<ReentrantOptions>,
    Arc<ParserOverlap>,
    Arc<Mutex<Vec<ParseObservation>>>,
);

fn registry_fixture() -> RegistryFixture {
    let format = FormatId::new(FORMAT_ID).expect("qualified parser reentrancy format id");
    let schema = FormatOptionSchema::new(
        FormatOptionSchemaKey::new(
            format.clone(),
            FormatOptionSchemaId::new("import-options")
                .expect("valid parser reentrancy option schema id"),
        ),
        NonZeroU32::new(1).expect("nonzero schema version"),
    );
    let option_key = FormatOptionKey::<ReentrantOptions>::new(schema.clone());
    let codec: Arc<dyn FormatOptionCodec> = Arc::new(ReentrantCodec { schema });
    let overlap = Arc::new(ParserOverlap::new());
    let observations = Arc::new(Mutex::new(Vec::new()));
    let descriptor = FormatDescriptor::builder(format, "Parser reentrancy contract")
        .extensions([FORMAT_EXTENSION])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .expect("valid binary parser reentrancy descriptor");
    let mut builder = RegistryBuilder::new();
    builder
        .register(ReentrantImporter {
            descriptor,
            option_key: option_key.clone(),
            codec,
            overlap: Arc::clone(&overlap),
            observations: Arc::clone(&observations),
        })
        .expect("parser reentrancy importer registers");
    let snapshot = builder
        .freeze(RegistryBuildLimits::default())
        .expect("parser reentrancy registry freezes");
    (snapshot, option_key, overlap, observations)
}

fn compile_request(
    snapshot: &RegistrySnapshot,
    option_key: &FormatOptionKey<ReentrantOptions>,
    scale: Option<u8>,
) -> PreparedImportRequest {
    let selection = FormatSelection::Forced(option_key.schema().key().owner().clone());
    let mut caller =
        ImportRequestLayer::new(ImportConfigOverlay::default().with_format_selection(selection));
    if let Some(scale) = scale {
        caller = caller
            .try_with_format_option(
                option_key
                    .envelope_from_canonical_payload([scale])
                    .expect("valid parser reentrancy option envelope"),
            )
            .expect("one option envelope per request");
    }
    snapshot
        .compile_request(
            &ImportRequest::from_layers(ImportRequestLayer::default(), caller),
            OptionCompileLimits::default(),
        )
        .expect("parser reentrancy request compiles")
}

fn import_success(
    request: &PreparedImportRequest,
    source: &'static str,
    input: u8,
) -> ImportReport {
    match import_bytes(
        request,
        AssetPath::new(source).expect("valid parser reentrancy source"),
        &[input],
        ImportOperationControl::default(),
    ) {
        Ok(report) => report,
        Err(error) => panic!("parser reentrancy import {source} failed: {error:?}"),
    }
}

fn run_concurrently(
    left_request: Arc<PreparedImportRequest>,
    right_request: Arc<PreparedImportRequest>,
    left_input: u8,
    right_input: u8,
) -> (ImportReport, ImportReport) {
    let left = thread::Builder::new()
        .name("parser-reentrancy-left".into())
        .spawn(move || import_success(left_request.as_ref(), "left.parser-reentrancy", left_input))
        .expect("left parser reentrancy thread starts");
    let right = thread::Builder::new()
        .name("parser-reentrancy-right".into())
        .spawn(move || {
            import_success(
                right_request.as_ref(),
                "right.parser-reentrancy",
                right_input,
            )
        })
        .expect("right parser reentrancy thread starts");
    let left = left
        .join()
        .expect("left parser reentrancy thread must not panic");
    let right = right
        .join()
        .expect("right parser reentrancy thread must not panic");
    (left, right)
}

fn assert_equivalent(concurrent: &ImportReport, serial: &ImportReport) {
    assert_eq!(concurrent.source(), serial.source());
    assert_eq!(concurrent.format().id(), serial.format().id());
    assert_eq!(concurrent.detection_evidence(), serial.detection_evidence());
    assert_eq!(concurrent.diagnostics(), serial.diagnostics());
    assert_eq!(concurrent.diagnostic_summary(), serial.diagnostic_summary());
    let concurrent_losses = concurrent.loss_evidence();
    let serial_losses = serial.loss_evidence();
    assert_eq!(concurrent_losses.len(), serial_losses.len());
    for (concurrent, serial) in concurrent_losses.iter().zip(serial_losses) {
        assert_eq!(concurrent.kind(), serial.kind());
        assert_eq!(concurrent.code(), serial.code());
        assert_eq!(concurrent.source(), serial.source());
        assert_eq!(concurrent.raw_range(), serial.raw_range());
        assert_eq!(concurrent.disposition(), serial.disposition());
    }
    let concurrent_dependencies = concurrent.optional_dependency_evidence();
    let serial_dependencies = serial.optional_dependency_evidence();
    assert_eq!(concurrent_dependencies.len(), serial_dependencies.len());
    for (concurrent, serial) in concurrent_dependencies.iter().zip(serial_dependencies) {
        assert_eq!(concurrent.identity(), serial.identity());
        assert_eq!(concurrent.normalized_path(), serial.normalized_path());
        assert_eq!(concurrent.owner(), serial.owner());
        assert_eq!(concurrent.cache_disposition(), serial.cache_disposition());
    }
    assert_eq!(concurrent.ledger(), serial.ledger());
    assert_eq!(concurrent.last_completed(), serial.last_completed());

    assert_eq!(
        concurrent.scene().canonical_block_count(),
        serial.scene().canonical_block_count()
    );
}

fn assert_observations(
    observations: &Mutex<Vec<ParseObservation>>,
    expected_once: [ParseObservation; 2],
) {
    let mut observed = observations
        .lock()
        .expect("parser observation mutex remains usable")
        .clone();
    observed.sort();
    let mut expected = expected_once
        .into_iter()
        .flat_map(|observation| [observation.clone(), observation])
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(observed, expected);
}

#[test]
fn one_prepared_request_is_semantically_reentrant_for_distinct_primary_inputs() {
    let (snapshot, option_key, overlap, observations) = registry_fixture();
    let request = Arc::new(compile_request(&snapshot, &option_key, None));
    let (concurrent_left, concurrent_right) =
        run_concurrently(Arc::clone(&request), Arc::clone(&request), 3, 11);
    let serial_left = import_success(request.as_ref(), "left.parser-reentrancy", 3);
    let serial_right = import_success(request.as_ref(), "right.parser-reentrancy", 11);

    assert_eq!(overlap.peak(), 2, "both parser calls must overlap");
    assert_equivalent(&concurrent_left, &serial_left);
    assert_equivalent(&concurrent_right, &serial_right);
    assert_observations(
        &observations,
        [
            ParseObservation {
                source: "left.parser-reentrancy".into(),
                input: 3,
                scale: 1,
                scaled_input: 3,
            },
            ParseObservation {
                source: "right.parser-reentrancy".into(),
                input: 11,
                scale: 1,
                scaled_input: 11,
            },
        ],
    );
}

#[test]
fn one_snapshot_is_semantically_reentrant_for_distinct_immutable_parser_options() {
    let (snapshot, option_key, overlap, observations) = registry_fixture();
    let left_request = Arc::new(compile_request(&snapshot, &option_key, Some(2)));
    let right_request = Arc::new(compile_request(&snapshot, &option_key, Some(7)));
    let (concurrent_left, concurrent_right) =
        run_concurrently(Arc::clone(&left_request), Arc::clone(&right_request), 5, 3);
    let serial_left = import_success(left_request.as_ref(), "left.parser-reentrancy", 5);
    let serial_right = import_success(right_request.as_ref(), "right.parser-reentrancy", 3);

    assert_eq!(overlap.peak(), 2, "both parser calls must overlap");
    assert_equivalent(&concurrent_left, &serial_left);
    assert_equivalent(&concurrent_right, &serial_right);
    assert_observations(
        &observations,
        [
            ParseObservation {
                source: "left.parser-reentrancy".into(),
                input: 5,
                scale: 2,
                scaled_input: 10,
            },
            ParseObservation {
                source: "right.parser-reentrancy".into(),
                input: 3,
                scale: 7,
                scaled_input: 21,
            },
        ],
    );
}
