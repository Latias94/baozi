use baozi_core::FormatId;
use baozi_import::{
    CapabilityStatus, CompiledFormatOptions, DetectionOutcome, DetectionResult, DetectionScope,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter, FormatOptionCodec,
    FormatOptionEnvelope, FormatOptionEnvelopeError, FormatOptionKey, FormatOptionLayers,
    FormatOptionSchema, FormatOptionSchemaId, FormatOptionSchemaKey, FormatSelection,
    FormatUnavailability, ImportConfigOverlay, ImportLossKind, ImportRequest,
    ImportRequestBuildError, ImportRequestLayer, LossPolicy, OptionBudget, OptionCompileError,
    OptionCompileLimits, OptionCompileScope, OptionRuntimeTypeKey, OptionScopeLimits, ParseResult,
    ParseScope, ReadHint, RegistryBuildError, RegistryBuildLimits, RegistryBuildScope,
    RegistryBuilder, RequestError, ResourceLimits, ResourceLimitsOverlay, SceneDraft,
};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

#[derive(Debug, PartialEq, Eq)]
struct TestOptions(u8);

#[derive(Debug)]
struct WrongOptions;

struct PayloadWhoseOwnedConversionPanics(Vec<u8>);

impl AsRef<[u8]> for PayloadWhoseOwnedConversionPanics {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<PayloadWhoseOwnedConversionPanics> for Box<[u8]> {
    fn from(_payload: PayloadWhoseOwnedConversionPanics) -> Self {
        panic!("payload conversion must happen only after the size gate")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodecBehavior {
    Normal,
    PanicDefault,
    PanicLayers,
    WrongDefaultType,
    WrongLayerType,
    ExhaustDefaultBudget,
    ExhaustLayerBudget,
    ChargeLayerDecodedBytes,
    ChargeLayerDecodedCells,
}

#[derive(Debug)]
struct TestCodec {
    schema: FormatOptionSchema,
    behavior: CodecBehavior,
    default_calls: Arc<AtomicUsize>,
    layer_calls: Arc<AtomicUsize>,
    observed_value: Arc<AtomicU8>,
}

impl FormatOptionCodec for TestCodec {
    fn schema(&self) -> Option<FormatOptionSchema> {
        Some(self.schema.clone())
    }

    fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
        OptionRuntimeTypeKey::of::<TestOptions>()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        self.default_calls.fetch_add(1, Ordering::SeqCst);
        match self.behavior {
            CodecBehavior::PanicDefault => panic!("default codec panic"),
            CodecBehavior::ExhaustDefaultBudget => scope.charge_work(2)?,
            _ => scope.charge_work(1)?,
        }
        if self.behavior == CodecBehavior::WrongDefaultType {
            scope.finish(WrongOptions, 1)
        } else {
            scope.finish(TestOptions(1), 1)
        }
    }

    fn compile_layers(
        &self,
        scope: &mut OptionCompileScope<'_>,
        defaults: &CompiledFormatOptions,
        layers: FormatOptionLayers<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        self.layer_calls.fetch_add(1, Ordering::SeqCst);
        if self.behavior == CodecBehavior::PanicLayers {
            panic!("request codec panic");
        }
        if self.behavior == CodecBehavior::ExhaustLayerBudget {
            scope.charge_work(2)?;
        }
        if self.behavior == CodecBehavior::ChargeLayerDecodedBytes {
            scope.charge_decoded_bytes(1)?;
        }
        if self.behavior == CodecBehavior::ChargeLayerDecodedCells {
            scope.charge_decoded_cells(1)?;
        }

        let key = FormatOptionKey::<TestOptions>::new(self.schema.clone());
        let mut value = defaults
            .downcast_ref(&key)
            .ok_or_else(|| OptionCompileError::invalid_payload("default-type", "wrong default"))?
            .0;
        if let Some(payload) = layers.preset() {
            let bytes = scope.reader(payload).read_remaining()?;
            value = one_byte(bytes)?;
        }
        if let Some(payload) = layers.caller() {
            let bytes = scope.reader(payload).read_remaining()?;
            value = one_byte(bytes)?;
        }
        self.observed_value.store(value, Ordering::SeqCst);
        if self.behavior == CodecBehavior::WrongLayerType {
            scope.finish(WrongOptions, 1)
        } else {
            scope.finish(TestOptions(value), 1)
        }
    }
}

fn one_byte(bytes: &[u8]) -> Result<u8, OptionCompileError> {
    match bytes {
        [value] => Ok(*value),
        _ => Err(OptionCompileError::invalid_payload(
            "one-byte",
            "expected exactly one byte",
        )),
    }
}

struct TestImporter {
    descriptor: FormatDescriptor,
    codec: Option<Arc<dyn FormatOptionCodec>>,
    runtime: OptionRuntimeTypeKey,
}

impl FormatImporter for TestImporter {
    fn descriptor(&self) -> FormatDescriptor {
        self.descriptor.clone()
    }

    fn option_codec(&self) -> Arc<dyn FormatOptionCodec> {
        self.codec
            .as_ref()
            .cloned()
            .unwrap_or_else(|| panic!("no-options importer uses the trait default"))
    }

    fn option_runtime_type_key(&self) -> OptionRuntimeTypeKey {
        self.runtime
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::NoMatch)
    }

    fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        unreachable!("registry option tests never create a parse session")
    }
}

#[derive(Debug)]
struct NoOptionsImporter {
    descriptor: FormatDescriptor,
}

impl FormatImporter for NoOptionsImporter {
    fn descriptor(&self) -> FormatDescriptor {
        self.descriptor.clone()
    }

    fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        unreachable!("registry option tests never create a parse session")
    }
}

struct PanickingDuplicateHooksImporter {
    descriptor: FormatDescriptor,
    hook_calls: Arc<AtomicUsize>,
}

impl FormatImporter for PanickingDuplicateHooksImporter {
    fn descriptor(&self) -> FormatDescriptor {
        self.descriptor.clone()
    }

    fn option_codec(&self) -> Arc<dyn FormatOptionCodec> {
        self.hook_calls.fetch_add(1, Ordering::SeqCst);
        panic!("duplicate codec hook must not run")
    }

    fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        unreachable!("duplicate registration test never parses")
    }
}

fn id(value: &str) -> FormatId {
    FormatId::new(value).unwrap()
}

fn schema(format: &FormatId, version: u32) -> FormatOptionSchema {
    FormatOptionSchema::new(
        FormatOptionSchemaKey::new(
            format.clone(),
            FormatOptionSchemaId::new("import-options").unwrap(),
        ),
        NonZeroU32::new(version).unwrap(),
    )
}

fn descriptor(format: FormatId) -> FormatDescriptor {
    FormatDescriptor::builder(format, "Test format")
        .extensions(["test"])
        .media_types(["model/test"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Unknown)])
        .build()
        .unwrap()
}

fn codec(
    format: &FormatId,
    behavior: CodecBehavior,
) -> (
    Arc<dyn FormatOptionCodec>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<AtomicU8>,
) {
    let default_calls = Arc::new(AtomicUsize::new(0));
    let layer_calls = Arc::new(AtomicUsize::new(0));
    let observed_value = Arc::new(AtomicU8::new(0));
    let codec: Arc<dyn FormatOptionCodec> = Arc::new(TestCodec {
        schema: schema(format, 1),
        behavior,
        default_calls: Arc::clone(&default_calls),
        layer_calls: Arc::clone(&layer_calls),
        observed_value: Arc::clone(&observed_value),
    });
    (codec, default_calls, layer_calls, observed_value)
}

fn importer(format: FormatId, codec: Arc<dyn FormatOptionCodec>) -> TestImporter {
    TestImporter {
        descriptor: descriptor(format),
        codec: Some(codec),
        runtime: OptionRuntimeTypeKey::of::<TestOptions>(),
    }
}

fn envelope(format: &FormatId, version: u32, value: u8) -> FormatOptionEnvelope {
    FormatOptionKey::<TestOptions>::new(schema(format, version))
        .envelope_from_canonical_payload([value])
        .unwrap()
}

fn import_request(
    preset: Vec<FormatOptionEnvelope>,
    caller: Vec<FormatOptionEnvelope>,
) -> ImportRequest {
    let mut preset_layer = ImportRequestLayer::default();
    for envelope in preset {
        preset_layer.try_push_format_option(envelope).unwrap();
    }
    let mut caller_layer = ImportRequestLayer::default();
    for envelope in caller {
        caller_layer.try_push_format_option(envelope).unwrap();
    }
    ImportRequest::from_layers(preset_layer, caller_layer)
}

fn caller_request(envelope: FormatOptionEnvelope) -> ImportRequest {
    import_request(Vec::new(), vec![envelope])
}

fn assert_importer_object_safe(_: &dyn FormatImporter) {}
fn assert_codec_object_safe(_: &dyn FormatOptionCodec) {}

#[test]
fn importer_and_codec_contracts_are_object_safe() {
    let _ = assert_importer_object_safe as fn(&dyn FormatImporter);
    let _ = assert_codec_object_safe as fn(&dyn FormatOptionCodec);
}

#[test]
fn envelope_checks_borrowed_payload_length_before_fallible_copy() {
    let format = id("org.example.oversized-options");
    let result = FormatOptionEnvelope::new(
        format.clone(),
        schema(&format, 1).key().clone(),
        NonZeroU32::new(1).unwrap(),
        PayloadWhoseOwnedConversionPanics(vec![0; 1024 * 1024 + 1]),
    );

    assert!(matches!(
        result,
        Err(FormatOptionEnvelopeError::PayloadTooLarge {
            actual,
            maximum,
        }) if actual == 1024 * 1024 + 1 && maximum == 1024 * 1024
    ));
}

#[test]
fn every_registration_requires_a_qualified_format_id() {
    let format = id("obj");
    let mut builder = RegistryBuilder::new();
    let error = builder
        .register(NoOptionsImporter {
            descriptor: descriptor(format.clone()),
        })
        .unwrap_err();

    assert_eq!(error.code(), "format-id-must-be-qualified");
    assert!(matches!(
        error,
        baozi_import::RegistryRegistrationError::FormatIdMustBeQualified { id }
            if id == format
    ));
}

#[test]
fn every_unavailable_declaration_requires_a_qualified_format_id() {
    let format = id("ply");
    let mut builder = RegistryBuilder::new();
    let error = builder
        .declare_unavailable(format.clone(), FormatUnavailability::FeatureDisabled)
        .unwrap_err();

    assert!(matches!(
        error,
        baozi_import::RegistryRegistrationError::FormatIdMustBeQualified { id }
            if id == format
    ));
}

#[test]
fn duplicate_registration_is_rejected_before_codec_hooks_run() {
    let format = id("org.example.duplicate");
    let descriptor = descriptor(format.clone());
    let mut builder = RegistryBuilder::new();
    builder
        .register(NoOptionsImporter {
            descriptor: descriptor.clone(),
        })
        .unwrap();
    let hook_calls = Arc::new(AtomicUsize::new(0));

    let error = builder
        .register(PanickingDuplicateHooksImporter {
            descriptor,
            hook_calls: Arc::clone(&hook_calls),
        })
        .unwrap_err();

    assert!(matches!(
        error,
        baozi_import::RegistryRegistrationError::DuplicateFormatId { id } if id == format
    ));
    assert_eq!(hook_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn freeze_compiles_defaults_in_stable_order_and_isolates_builder_mutation() {
    let a = id("org.example.a");
    let z = id("org.example.z");
    let (a_codec, a_defaults, _, _) = codec(&a, CodecBehavior::Normal);
    let (z_codec, z_defaults, _, _) = codec(&z, CodecBehavior::Normal);
    let mut builder = RegistryBuilder::new();
    builder.register(importer(z.clone(), z_codec)).unwrap();
    builder.register(importer(a.clone(), a_codec)).unwrap();

    let first = builder.freeze(RegistryBuildLimits::default()).unwrap();
    assert_eq!(
        first
            .descriptors()
            .map(|descriptor| descriptor.id().as_str())
            .collect::<Vec<_>>(),
        ["org.example.a", "org.example.z"]
    );
    assert_eq!(a_defaults.load(Ordering::SeqCst), 1);
    assert_eq!(z_defaults.load(Ordering::SeqCst), 1);

    assert!(builder.remove(&a));
    let second = builder.freeze(RegistryBuildLimits::default()).unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 1);
    assert!(first.descriptor(&a).is_some());
    assert!(second.descriptor(&a).is_none());
}

#[test]
fn freeze_rejects_default_panic_wrong_type_and_budget_without_a_snapshot() {
    let panic_id = id("org.example.panic-default");
    let (panic_codec, panic_defaults, _, _) = codec(&panic_id, CodecBehavior::PanicDefault);
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer(panic_id.clone(), panic_codec))
        .unwrap();
    let panic_error = builder.freeze(RegistryBuildLimits::default()).unwrap_err();
    assert_eq!(panic_error.code(), "default-codec-panicked");
    assert!(matches!(
        &panic_error,
        RegistryBuildError::CodecPanicked { format, detail }
            if format == &panic_id && detail.as_ref() == "default codec panic"
    ));
    assert!(matches!(
        builder.freeze(RegistryBuildLimits::default()),
        Err(RegistryBuildError::ExtensionPoisoned { format }) if format == panic_id
    ));
    assert_eq!(panic_defaults.load(Ordering::SeqCst), 1);

    let wrong_id = id("org.example.wrong-default");
    let (wrong_codec, _, _, _) = codec(&wrong_id, CodecBehavior::WrongDefaultType);
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer(wrong_id.clone(), wrong_codec))
        .unwrap();
    assert!(matches!(
        builder.freeze(RegistryBuildLimits::default()),
        Err(RegistryBuildError::RuntimeTypeMismatch { format }) if format == wrong_id
    ));

    let budget_id = id("org.example.default-budget");
    let (budget_codec, _, _, _) = codec(&budget_id, CodecBehavior::ExhaustDefaultBudget);
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer(budget_id.clone(), budget_codec))
        .unwrap();
    let limits = RegistryBuildLimits::new(OptionScopeLimits::new(1, 1, 1, 1, 1, 1, 1), 1, 1, 1);
    assert!(matches!(
        builder.freeze(limits),
        Err(RegistryBuildError::CodecRejected {
            format,
            cause: OptionCompileError::BudgetExceeded {
                budget: OptionBudget::Work,
                ..
            }
        }) if format == budget_id
    ));
}

#[test]
fn import_request_compiler_freezes_config_sorted_overrides_and_snapshot_identity() {
    let first = id("org.example.canonical-a");
    let second = id("org.example.canonical-z");
    let (first_codec, _, first_layer_calls, first_observed) = codec(&first, CodecBehavior::Normal);
    let (second_codec, _, second_layer_calls, second_observed) =
        codec(&second, CodecBehavior::Normal);
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer(first.clone(), first_codec))
        .unwrap();
    builder
        .register(importer(second.clone(), second_codec))
        .unwrap();
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();

    let defaults = ResourceLimits::default();
    let mut preset_limits = ResourceLimitsOverlay::default();
    preset_limits.max_tokens = Some(7);
    let preset = ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_resource_limits(preset_limits)
            .with_conformance_policy(baozi_import::ConformancePolicy::AllowDeclaredRecovery)
            .with_loss_policy(
                LossPolicy::preserve_or_fail().allow(ImportLossKind::OptionalDependencyUnavailable),
            ),
    )
    .try_with_format_option(envelope(&second, 1, 2))
    .unwrap();

    let mut caller_limits = ResourceLimitsOverlay::default();
    caller_limits.max_tokens = Some(defaults.max_tokens);
    let caller = ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_resource_limits(caller_limits)
            .with_conformance_policy(baozi_import::ConformancePolicy::Strict)
            .with_loss_policy(LossPolicy::preserve_or_fail()),
    )
    .try_with_format_option(envelope(&second, 1, 3))
    .unwrap()
    .try_with_format_option(envelope(&first, 1, 1))
    .unwrap();
    let request = ImportRequest::from_layers(preset, caller);

    let compiled = snapshot
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap();

    assert_eq!(compiled.config(), &request.resolved_config());
    assert_eq!(
        compiled.config().resource_limits().max_tokens,
        defaults.max_tokens
    );
    assert_eq!(
        compiled
            .overridden_formats()
            .map(FormatId::as_str)
            .collect::<Vec<_>>(),
        ["org.example.canonical-a", "org.example.canonical-z"]
    );
    assert_eq!(first_layer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_layer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(first_observed.load(Ordering::SeqCst), 1);
    assert_eq!(second_observed.load(Ordering::SeqCst), 3);
    assert!(compiled.belongs_to(&snapshot));

    let other_publication = builder.freeze(RegistryBuildLimits::default()).unwrap();
    assert!(!compiled.belongs_to(&other_publication));
}

#[test]
fn request_rejects_unknown_disabled_no_options_duplicate_owner_schema_and_version() {
    let configured = id("org.example.configured");
    let none = id("org.example.none");
    let disabled = id("org.baozi.ply");
    let (configured_codec, _, configured_layer_calls, _) =
        codec(&configured, CodecBehavior::Normal);
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer(configured.clone(), configured_codec))
        .unwrap();
    builder
        .register(NoOptionsImporter {
            descriptor: descriptor(none.clone()),
        })
        .unwrap();
    builder
        .declare_unavailable(disabled.clone(), FormatUnavailability::FeatureDisabled)
        .unwrap();
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();

    let unknown = id("org.example.unknown");
    let selected_unknown = ImportRequest::from_layers(
        ImportRequestLayer::default(),
        ImportRequestLayer::new(
            ImportConfigOverlay::default()
                .with_format_selection(FormatSelection::Forced(unknown.clone())),
        )
        .try_with_format_option(envelope(&configured, 1, 1))
        .unwrap(),
    );
    assert!(matches!(
        snapshot.compile_request(&selected_unknown, OptionCompileLimits::default()),
        Err(RequestError::UnknownFormat { format }) if format == unknown
    ));
    assert_eq!(configured_layer_calls.load(Ordering::SeqCst), 0);

    let selected_disabled = ImportRequest::from_layers(
        ImportRequestLayer::default(),
        ImportRequestLayer::new(
            ImportConfigOverlay::default()
                .with_format_selection(FormatSelection::Forced(disabled.clone())),
        ),
    );
    assert!(matches!(
        snapshot.compile_request(&selected_disabled, OptionCompileLimits::default()),
        Err(RequestError::FormatUnavailable { format, .. }) if format == disabled
    ));

    assert!(matches!(
        snapshot.compile_request(
            &caller_request(envelope(&unknown, 1, 1)),
            OptionCompileLimits::default(),
        ),
        Err(RequestError::UnknownFormat { format }) if format == unknown
    ));
    assert!(matches!(
        snapshot.compile_request(
            &caller_request(envelope(&disabled, 1, 1)),
            OptionCompileLimits::default(),
        ),
        Err(RequestError::FormatUnavailable { format, .. }) if format == disabled
    ));
    assert!(matches!(
        snapshot.compile_request(
            &caller_request(envelope(&none, 1, 1)),
            OptionCompileLimits::default(),
        ),
        Err(RequestError::OptionsUnsupported { format }) if format == none
    ));

    let duplicate = ImportRequestLayer::default()
        .try_with_format_option(envelope(&configured, 1, 1))
        .unwrap()
        .try_with_format_option(envelope(&configured, 1, 2));
    assert!(matches!(
        duplicate,
        Err(ImportRequestBuildError::DuplicateFormatOption { target }) if target == configured
    ));

    let other = id("org.example.other");
    let wrong_owner = FormatOptionEnvelope::new(
        configured.clone(),
        schema(&other, 1).key().clone(),
        NonZeroU32::new(1).unwrap(),
        [1],
    )
    .unwrap();
    assert!(matches!(
        snapshot.compile_request(
            &caller_request(wrong_owner),
            OptionCompileLimits::default(),
        ),
        Err(RequestError::WrongSchemaOwner { target }) if target == configured
    ));

    let unknown_schema = FormatOptionEnvelope::new(
        configured.clone(),
        FormatOptionSchemaKey::new(
            configured.clone(),
            FormatOptionSchemaId::new("other-options").unwrap(),
        ),
        NonZeroU32::new(1).unwrap(),
        [1],
    )
    .unwrap();
    assert!(matches!(
        snapshot.compile_request(
            &caller_request(unknown_schema),
            OptionCompileLimits::default(),
        ),
        Err(RequestError::UnknownSchema { schema }) if schema.as_str() == "other-options"
    ));
    assert!(matches!(
        snapshot.compile_request(
            &caller_request(envelope(&configured, 2, 1)),
            OptionCompileLimits::default(),
        ),
        Err(RequestError::UnsupportedVersion { format, .. }) if format == configured
    ));
}

#[test]
fn request_validates_every_target_before_any_codec_runs() {
    let valid = id("org.example.a-valid");
    let unknown = id("org.example.z-unknown");
    let (valid_codec, _, valid_layer_calls, _) = codec(&valid, CodecBehavior::Normal);
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer(valid.clone(), valid_codec))
        .unwrap();
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let request = import_request(
        Vec::new(),
        vec![envelope(&valid, 1, 1), envelope(&unknown, 1, 1)],
    );

    assert!(matches!(
        snapshot.compile_request(&request, OptionCompileLimits::default()),
        Err(RequestError::UnknownFormat { format }) if format == unknown
    ));
    assert_eq!(valid_layer_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn request_compiles_only_submitted_targets_and_applies_precedence_once() {
    let target = id("org.example.target");
    let (target_codec, _, target_layers, observed) = codec(&target, CodecBehavior::Normal);
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer(target.clone(), target_codec))
        .unwrap();

    let mut unused_layer_counts = Vec::new();
    for index in 0..32 {
        let unused = id(&format!("org.example.unused-{index}"));
        let (unused_codec, _, unused_layers, _) = codec(&unused, CodecBehavior::PanicLayers);
        builder.register(importer(unused, unused_codec)).unwrap();
        unused_layer_counts.push(unused_layers);
    }
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();

    let common_only = ImportRequest::from_layers(
        ImportRequestLayer::new(
            ImportConfigOverlay::default()
                .with_conformance_policy(baozi_import::ConformancePolicy::AllowDeclaredRecovery),
        ),
        ImportRequestLayer::default(),
    );
    snapshot
        .compile_request(&common_only, OptionCompileLimits::default())
        .unwrap();
    assert_eq!(target_layers.load(Ordering::SeqCst), 0);

    let request = import_request(vec![envelope(&target, 1, 2)], vec![envelope(&target, 1, 3)]);
    let compiled = snapshot
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap();
    assert_eq!(target_layers.load(Ordering::SeqCst), 1);
    assert_eq!(observed.load(Ordering::SeqCst), 3);
    assert!(
        unused_layer_counts
            .iter()
            .all(|calls| calls.load(Ordering::SeqCst) == 0)
    );
    assert_eq!(
        compiled
            .overridden_formats()
            .map(FormatId::as_str)
            .collect::<Vec<_>>(),
        ["org.example.target"]
    );
}

#[test]
fn request_indexed_codec_work_is_independent_of_registry_size() {
    let target = id("org.example.scale-target");
    let (small_codec, _, small_layer_calls, _) = codec(&target, CodecBehavior::Normal);
    let mut small_builder = RegistryBuilder::new();
    small_builder
        .register(importer(target.clone(), small_codec))
        .unwrap();
    let small = small_builder
        .freeze(RegistryBuildLimits::default())
        .unwrap();

    let (large_codec, _, large_layer_calls, _) = codec(&target, CodecBehavior::Normal);
    let mut large_builder = RegistryBuilder::new();
    large_builder
        .register(importer(target.clone(), large_codec))
        .unwrap();
    let mut unused_layer_calls = Vec::new();
    for index in 0..128 {
        let unused = id(&format!("org.example.scale-unused-{index}"));
        let (unused_codec, _, layer_calls, _) = codec(&unused, CodecBehavior::PanicLayers);
        large_builder
            .register(importer(unused, unused_codec))
            .unwrap();
        unused_layer_calls.push(layer_calls);
    }
    let large = large_builder
        .freeze(RegistryBuildLimits::default())
        .unwrap();
    let request = caller_request(envelope(&target, 1, 7));

    small
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap();
    large
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap();

    assert_eq!(small_layer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(large_layer_calls.load(Ordering::SeqCst), 1);
    assert!(
        unused_layer_calls
            .iter()
            .all(|calls| calls.load(Ordering::SeqCst) == 0)
    );
}

#[test]
fn request_override_order_is_stable_by_format_id() {
    let first = id("org.example.order-a");
    let second = id("org.example.order-z");
    let (first_codec, _, _, _) = codec(&first, CodecBehavior::Normal);
    let (second_codec, _, _, _) = codec(&second, CodecBehavior::Normal);
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer(first.clone(), first_codec))
        .unwrap();
    builder
        .register(importer(second.clone(), second_codec))
        .unwrap();
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let request = import_request(
        Vec::new(),
        vec![envelope(&second, 1, 2), envelope(&first, 1, 1)],
    );

    let compiled = snapshot
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap();

    assert_eq!(
        compiled
            .overridden_formats()
            .map(FormatId::as_str)
            .collect::<Vec<_>>(),
        ["org.example.order-a", "org.example.order-z"]
    );
}

#[test]
fn request_codec_panic_poisons_once_and_prevents_reinvocation() {
    let format = id("org.example.panic-layers");
    let (codec, _, layer_calls, _) = codec(&format, CodecBehavior::PanicLayers);
    let mut builder = RegistryBuilder::new();
    builder.register(importer(format.clone(), codec)).unwrap();
    let first_snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let second_snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let request = caller_request(envelope(&format, 1, 7));

    let panic_error = first_snapshot
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap_err();
    assert_eq!(panic_error.code(), "codec-panicked");
    assert!(matches!(
        &panic_error,
        RequestError::CodecPanicked { format: failed, detail }
            if failed == &format && detail.as_ref() == "request codec panic"
    ));
    assert_eq!(layer_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        second_snapshot.compile_request(&request, OptionCompileLimits::default()),
        Err(RequestError::ExtensionPoisoned { format: failed }) if failed == format
    ));
    assert!(matches!(
        builder.freeze(RegistryBuildLimits::default()),
        Err(RegistryBuildError::ExtensionPoisoned { format: failed }) if failed == format
    ));
    assert_eq!(layer_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn request_rejects_wrong_compiled_runtime_type() {
    let format = id("org.example.wrong-layer");
    let (codec, _, _, _) = codec(&format, CodecBehavior::WrongLayerType);
    let mut builder = RegistryBuilder::new();
    builder.register(importer(format.clone(), codec)).unwrap();
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();

    assert!(matches!(
        snapshot.compile_request(
            &caller_request(envelope(&format, 1, 7)),
            OptionCompileLimits::default(),
        ),
        Err(RequestError::RuntimeTypeMismatch { format: failed }) if failed == format
    ));
}

#[test]
fn request_codec_work_is_bounded_before_session_creation() {
    let format = id("org.example.layer-budget");
    let (codec, _, _, _) = codec(&format, CodecBehavior::ExhaustLayerBudget);
    let mut builder = RegistryBuilder::new();
    builder.register(importer(format.clone(), codec)).unwrap();
    let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let limits =
        OptionCompileLimits::new(OptionScopeLimits::new(1, 8, 8, 8, 8, 8, 8), 1, 1, 8, 8, 8);

    assert!(matches!(
        snapshot.compile_request(
            &caller_request(envelope(&format, 1, 7)),
            limits,
        ),
        Err(RequestError::CodecRejected {
            format: failed,
            cause: OptionCompileError::BudgetExceeded {
                budget: OptionBudget::Work,
                ..
            }
        }) if failed == format
    ));
}

#[test]
fn request_aggregate_decoded_budgets_span_submitted_targets() {
    for (suffix, behavior, budget, total_decoded_bytes, total_decoded_cells) in [
        (
            "bytes",
            CodecBehavior::ChargeLayerDecodedBytes,
            OptionBudget::DecodedBytes,
            1,
            2,
        ),
        (
            "cells",
            CodecBehavior::ChargeLayerDecodedCells,
            OptionBudget::DecodedCells,
            2,
            1,
        ),
    ] {
        let first = id(&format!("org.example.{suffix}-a"));
        let second = id(&format!("org.example.{suffix}-b"));
        let (first_codec, _, _, _) = codec(&first, behavior);
        let (second_codec, _, _, _) = codec(&second, behavior);
        let mut builder = RegistryBuilder::new();
        builder
            .register(importer(first.clone(), first_codec))
            .unwrap();
        builder
            .register(importer(second.clone(), second_codec))
            .unwrap();
        let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
        let request = import_request(
            Vec::new(),
            vec![envelope(&first, 1, 1), envelope(&second, 1, 1)],
        );
        let limits =
            OptionCompileLimits::new(OptionScopeLimits::new(8, 1, 1, 1, 1, 1, 1), 2, 32, 2, 2, 2)
                .with_aggregate_decoded_limits(total_decoded_bytes, total_decoded_cells);

        assert!(matches!(
            snapshot.compile_request(&request, limits),
            Err(RequestError::CodecRejected {
                format,
                cause: OptionCompileError::BudgetExceeded { budget: actual, .. },
            }) if format == second && actual == budget
        ));
    }
}
