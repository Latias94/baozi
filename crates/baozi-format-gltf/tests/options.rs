use baozi_format_gltf::{
    FORMAT_ID, GltfImportOptions, GltfImporter, GltfSceneSelection, gltf_import_option_key,
};
use baozi_import::authoring::FormatId;
use baozi_import::{
    DetectionOutcome, DetectionResult, DetectionScope, FormatDescriptor, FormatImporter,
    FormatOptionCodec, FormatOptionEnvelope, FormatSelection, ImportConfigOverlay,
    ImportOperationControl, ImportRequest, ImportRequestLayer, OptionBudget, OptionCompileError,
    OptionCompileLimits, OptionRuntimeTypeKey, OptionScopeLimits, ParseResult, ParseScope,
    PreparedImportRequest, Probe, ReadHint, RegistryBuildError, RegistryBuildLimits,
    RegistryBuilder, RegistrySnapshot, RequestError, SceneDraft, import_bytes,
};
use baozi_io::AssetPath;
use std::sync::{Arc, Mutex};

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DOCUMENT: &[u8] = br#"{
  "asset": { "version": "2.0" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }, { "nodes": [0] }, { "nodes": [0] }],
  "nodes": [{ "name": "SharedRoot" }]
}"#;

fn snapshot() -> TestResult<RegistrySnapshot> {
    let mut registry = RegistryBuilder::new();
    registry.register(GltfImporter)?;
    Ok(registry.freeze(RegistryBuildLimits::default())?)
}

fn request_layers(
    preset: Option<FormatOptionEnvelope>,
    caller: Option<FormatOptionEnvelope>,
) -> TestResult<ImportRequest> {
    let mut preset_layer = ImportRequestLayer::default();
    if let Some(envelope) = preset {
        preset_layer.try_push_format_option(envelope)?;
    }
    let config = ImportConfigOverlay::default()
        .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT_ID)?));
    let mut caller_layer = ImportRequestLayer::new(config);
    if let Some(envelope) = caller {
        caller_layer.try_push_format_option(envelope)?;
    }
    Ok(ImportRequest::from_layers(preset_layer, caller_layer))
}

fn run_request(request: &PreparedImportRequest) -> TestResult<()> {
    let report = import_bytes(
        request,
        AssetPath::new("options.gltf")?,
        DOCUMENT,
        ImportOperationControl::default(),
    )?;
    assert!(report.diagnostics().is_empty());
    Ok(())
}

struct ObservingImporter(Arc<Mutex<Vec<GltfSceneSelection>>>);

impl FormatImporter for ObservingImporter {
    fn descriptor(&self) -> FormatDescriptor {
        GltfImporter.descriptor()
    }

    fn option_codec(&self) -> Arc<dyn FormatOptionCodec> {
        GltfImporter.option_codec()
    }

    fn option_runtime_type_key(&self) -> OptionRuntimeTypeKey {
        GltfImporter.option_runtime_type_key()
    }

    fn detect(
        &self,
        probe: &Probe<'_>,
        hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        GltfImporter.detect(probe, hint, scope)
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let selection = scope.options(&gltf_import_option_key())?.scene_selection();
        self.0
            .lock()
            .expect("option observation mutex remains usable")
            .push(selection);
        GltfImporter.parse(scope)
    }
}

#[test]
fn option_schema_and_complete_canonical_payloads_are_stable() -> TestResult<()> {
    let key = gltf_import_option_key();
    assert_eq!(key.schema().key().owner().as_str(), FORMAT_ID);
    assert_eq!(key.schema().key().local().as_str(), "import-options");
    assert_eq!(key.schema().version().get(), 1);
    assert_eq!(
        GltfImporter.option_codec().schema().as_ref(),
        Some(key.schema())
    );
    assert_eq!(
        GltfImporter.option_runtime_type_key(),
        key.runtime_type_key()
    );
    assert_eq!(
        GltfImporter.option_codec().runtime_type_key(),
        key.runtime_type_key()
    );

    let snapshot = snapshot()?;
    let defaults = GltfImportOptions::default();
    let cases: &[(GltfSceneSelection, &[u8])] = &[
        (GltfSceneSelection::DocumentDefault, &[1, 0]),
        (GltfSceneSelection::SceneIndex(0), &[1, 1, 0, 0, 0, 0]),
        (
            GltfSceneSelection::SceneIndex(0x1234_5678),
            &[1, 1, 0x78, 0x56, 0x34, 0x12],
        ),
        (
            GltfSceneSelection::SceneIndex(u32::MAX),
            &[1, 1, 0xff, 0xff, 0xff, 0xff],
        ),
    ];
    for &(selection, payload) in cases {
        let options = defaults.with_scene_selection(selection);
        assert_eq!(options.scene_selection(), selection);
        let envelope = options.to_envelope()?;
        assert_eq!(envelope, key.envelope_from_canonical_payload(payload)?);
        let request = request_layers(None, Some(envelope))?;
        snapshot.compile_request(&request, OptionCompileLimits::default())?;
    }
    assert_eq!(
        defaults.scene_selection(),
        GltfSceneSelection::DocumentDefault
    );
    assert_eq!(GltfSceneSelection::default(), defaults.scene_selection());
    Ok(())
}

#[test]
fn malformed_layers_are_rejected_even_when_the_caller_overrides_them() -> TestResult<()> {
    let snapshot = snapshot()?;
    let key = gltf_import_option_key();
    let explicit_default = GltfImportOptions::default().to_envelope()?;
    let cases: &[(&[u8], &str)] = &[
        (&[], "truncated"),
        (&[2], "unknown-presence-bits"),
        (&[0x81, 0], "unknown-presence-bits"),
        (&[1], "truncated"),
        (&[1, 2], "unknown-scene-selection"),
        (&[1, 0xff], "unknown-scene-selection"),
        (&[1, 1], "truncated"),
        (&[1, 1, 0], "truncated"),
        (&[1, 1, 0, 0], "truncated"),
        (&[1, 1, 0, 0, 0], "truncated"),
        (&[0, 0], "trailing-bytes"),
        (&[1, 0, 0], "trailing-bytes"),
        (&[1, 1, 0, 0, 0, 0, 0], "trailing-bytes"),
    ];
    for &(payload, expected_code) in cases {
        let envelope = key.envelope_from_canonical_payload(payload)?;
        for (preset, caller) in [
            (Some(envelope.clone()), Some(explicit_default.clone())),
            (None, Some(envelope)),
        ] {
            let request = request_layers(preset, caller)?;
            let error = snapshot
                .compile_request(&request, OptionCompileLimits::default())
                .expect_err("noncanonical glTF options must fail before import");
            let RequestError::CodecRejected {
                format,
                cause: OptionCompileError::InvalidPayload { code, .. },
            } = &error
            else {
                panic!("expected invalid glTF option payload, got {error:?}");
            };
            assert_eq!(format.as_str(), FORMAT_ID);
            assert_eq!(code.as_str(), expected_code, "payload: {payload:?}");
        }
    }
    Ok(())
}

#[test]
fn absent_fields_inherit_and_explicit_caller_selections_win() -> TestResult<()> {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut registry = RegistryBuilder::new();
    registry.register(ObservingImporter(Arc::clone(&observed)))?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    let inherit = gltf_import_option_key().envelope_from_canonical_payload([0])?;
    let document_default = GltfImportOptions::default().to_envelope()?;
    let first = GltfImportOptions::default()
        .with_scene_selection(GltfSceneSelection::SceneIndex(1))
        .to_envelope()?;
    let second = GltfImportOptions::default()
        .with_scene_selection(GltfSceneSelection::SceneIndex(2))
        .to_envelope()?;

    let cases = [
        (None, None, GltfSceneSelection::DocumentDefault),
        (
            Some(inherit.clone()),
            None,
            GltfSceneSelection::DocumentDefault,
        ),
        (
            None,
            Some(inherit.clone()),
            GltfSceneSelection::DocumentDefault,
        ),
        (Some(first.clone()), None, GltfSceneSelection::SceneIndex(1)),
        (
            Some(first.clone()),
            Some(inherit),
            GltfSceneSelection::SceneIndex(1),
        ),
        (
            Some(first.clone()),
            Some(document_default),
            GltfSceneSelection::DocumentDefault,
        ),
        (Some(first), Some(second), GltfSceneSelection::SceneIndex(2)),
    ];
    for (preset, caller, expected) in cases {
        let request = request_layers(preset, caller)?;
        let request = snapshot.compile_request(&request, OptionCompileLimits::default())?;
        run_request(&request)?;
        assert_eq!(
            observed
                .lock()
                .expect("option observation mutex remains usable")
                .last(),
            Some(&expected)
        );
    }
    Ok(())
}

#[test]
fn prepared_requests_keep_independent_frozen_selections() -> TestResult<()> {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut registry = RegistryBuilder::new();
    registry.register(ObservingImporter(Arc::clone(&observed)))?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    let defaults =
        snapshot.compile_request(&request_layers(None, None)?, OptionCompileLimits::default())?;
    let mut submitted = request_layers(
        None,
        Some(
            GltfImportOptions::default()
                .with_scene_selection(GltfSceneSelection::SceneIndex(1))
                .to_envelope()?,
        ),
    )?;
    let first = snapshot.compile_request(&submitted, OptionCompileLimits::default())?;
    submitted = request_layers(
        None,
        Some(
            GltfImportOptions::default()
                .with_scene_selection(GltfSceneSelection::SceneIndex(2))
                .to_envelope()?,
        ),
    )?;
    let second = snapshot.compile_request(&submitted, OptionCompileLimits::default())?;
    drop(submitted);
    let first_clone = first.clone();

    for request in [&second, &first, &defaults, &first_clone, &second] {
        run_request(request)?;
    }
    assert_eq!(
        *observed
            .lock()
            .expect("option observation mutex remains usable"),
        [
            GltfSceneSelection::SceneIndex(2),
            GltfSceneSelection::SceneIndex(1),
            GltfSceneSelection::DocumentDefault,
            GltfSceneSelection::SceneIndex(1),
            GltfSceneSelection::SceneIndex(2),
        ]
    );
    Ok(())
}

#[test]
fn default_compilation_charges_the_actual_inline_option_size() -> TestResult<()> {
    let retained_bytes = size_of::<GltfImportOptions>() as u64;
    for maximum in [retained_bytes - 1, retained_bytes] {
        let per_codec = OptionScopeLimits::new(1, 0, 0, 0, 0, 0, maximum);
        let limits = RegistryBuildLimits::new(per_codec, 1, 0, retained_bytes);
        let mut registry = RegistryBuilder::new();
        registry.register(GltfImporter)?;
        let result = registry.freeze(limits);
        if maximum < retained_bytes {
            assert!(matches!(
                result,
                Err(RegistryBuildError::CodecRejected {
                    cause: OptionCompileError::BudgetExceeded {
                        budget: OptionBudget::ResultBytes,
                        ..
                    },
                    ..
                })
            ));
        } else {
            result?;
        }
    }
    Ok(())
}

#[test]
fn request_compilation_meters_both_layers_and_retained_option_bytes() -> TestResult<()> {
    let snapshot = snapshot()?;
    let retained_bytes = size_of::<GltfImportOptions>() as u64;
    let request = request_layers(
        Some(
            GltfImportOptions::default()
                .with_scene_selection(GltfSceneSelection::SceneIndex(0x1234_5678))
                .to_envelope()?,
        ),
        Some(GltfImportOptions::default().to_envelope()?),
    )?;
    for (max_work, max_payload, max_result, expected_budget) in [
        (8, 8, retained_bytes, None),
        (7, 8, retained_bytes, Some(OptionBudget::Work)),
        (8, 7, retained_bytes, Some(OptionBudget::PayloadBytes)),
        (8, 8, retained_bytes - 1, Some(OptionBudget::ResultBytes)),
    ] {
        let per_target = OptionScopeLimits::new(max_work, max_payload, 0, 0, 0, 0, max_result);
        let limits = OptionCompileLimits::new(per_target, 2, 8, 8, 0, retained_bytes);
        let result = snapshot.compile_request(&request, limits);
        if let Some(expected) = expected_budget {
            assert!(matches!(
                result,
                Err(RequestError::CodecRejected {
                    cause: OptionCompileError::BudgetExceeded { budget, .. },
                    ..
                }) if budget == expected
            ));
        } else {
            result?;
        }
    }
    Ok(())
}
