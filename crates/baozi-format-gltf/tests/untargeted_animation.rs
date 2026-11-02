mod common;

use baozi_core::{ProvenanceKindRef, ProvenanceOperationKindRef};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    AnimationTarget, ImportCause, ImportDimension, ImportFailure, ImportLossKind, ImportReport,
    LossPolicy, SceneSnapshotLimits, ScopeAbortCause, SourceObjectLocatorRef,
    SourceScopeDisposition, SourceScopeEntityKind,
};
use baozi_operation::{CancellationOrigin, ControlCause};
use base64::Engine as _;
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, assert_gltf_failure, assert_resource_limit,
    controlled_import, expected_error, glb_with_json, import_assets, import_assets_result,
    import_assets_result_with_control, sidecar_options,
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

const RETAINED_CELLS: u64 = 8;

fn retained_binary() -> Vec<u8> {
    // Two times, two translation values, then an asymmetric triangle.
    [
        0.0_f32, 1.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0, 0.0,
    ]
    .into_iter()
    .flat_map(f32::to_le_bytes)
    .collect()
}

fn document(triangle: bool) -> Value {
    let uri = format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(retained_binary()),
    );
    let mut document = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}], "nodes": [{"name": "Animated"}],
        "buffers": [
            {"byteLength": 68, "uri": uri},
            {"byteLength": 32, "uri": "ignored.bin"}
        ],
        "bufferViews": [
            {"buffer": 0, "byteLength": 8},
            {"buffer": 0, "byteOffset": 8, "byteLength": 24},
            {"buffer": 0, "byteOffset": 32, "byteLength": 36},
            {"buffer": 1, "byteLength": 8},
            {"buffer": 1, "byteOffset": 8, "byteLength": 24}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR",
             "min": [0], "max": [1]},
            {"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0, 0, 0], "max": [2, 3, 0]},
            {"bufferView": 3, "componentType": 5126, "count": 2, "type": "SCALAR",
             "min": [0], "max": [100]},
            {"bufferView": 4, "componentType": 5126, "count": 2, "type": "VEC3"}
        ],
        "animations": [{
            "name": "Mixed", "samplers": [{"input": 3, "output": 4}, {"input": 0, "output": 1}],
            "channels": [
                {"sampler": 0, "target": {"path": "translation"}},
                {"sampler": 1, "target": {"node": 0, "path": "translation"}}
            ]
        }]
    });
    if triangle {
        document["nodes"][0]["mesh"] = json!(0);
        document["meshes"] = json!([{"primitives": [{"attributes": {"POSITION": 2}}]}]);
    }
    document
}

fn containers(document: &Value) -> TestResult<[(&'static str, Vec<u8>); 2]> {
    let json = serde_json::to_vec(document)?;
    let mut embedded = document.clone();
    embedded["buffers"][0]
        .as_object_mut()
        .unwrap()
        .remove("uri");
    let glb = append_glb_chunk(
        glb_with_json(&serde_json::to_vec(&embedded)?),
        b"BIN\0",
        &retained_binary(),
    );
    Ok([("untargeted.gltf", json), ("untargeted.glb", glb)])
}

fn all_ignored(document: &mut Value) {
    document["animations"][0]["channels"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
}

fn assert_released(report: &ImportReport) {
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}

fn assert_failure_released(failure: &ImportFailure) {
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

fn assert_no_dependencies(report: &ImportReport) {
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
    ] {
        assert_eq!(report.ledger().dimension(dimension).unwrap().attempted(), 0);
    }
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        1
    );
    assert_eq!(
        report
            .successful_input_manifest()
            .counts()
            .node_class_count(),
        1
    );
}

fn assert_retained_channel(report: &ImportReport, source_index: usize) -> TestResult<()> {
    let scene = report.scene();
    assert_eq!(scene.animation_clip_count(), 1);
    let clip = scene.animation_clips().next().unwrap();
    assert_eq!(clip.name(), Some("Mixed"));
    assert_eq!(clip.duration().to_bits(), 1.0_f64.to_bits());
    assert_eq!(clip.channel_count(), 1);
    let channel = clip.channel(0).unwrap();
    assert_eq!(
        channel.target(),
        AnimationTarget::NodeTranslation(scene.nodes().next().unwrap().id())
    );
    assert_eq!(channel.key_times(), &[0.0, 1.0]);
    assert_eq!(channel.values(), &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(channel.in_tangents(), None);
    assert_eq!(channel.out_tangents(), None);
    let prefix = format!("kind=animation-channel;parent=0;index={source_index};");
    let sources = scene
        .owner_provenance(channel.metadata_owner())?
        .filter_map(|node| match node.kind() {
            ProvenanceKindRef::Source(source) => match source.locator() {
                SourceObjectLocatorRef::Namespaced { value, .. }
                    if value.starts_with("kind=animation-channel;") =>
                {
                    Some(value)
                }
                _ => None,
            },
            ProvenanceKindRef::Operation(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].starts_with(&prefix));
    let snapshot =
        scene.capture_snapshot(SceneSnapshotLimits::try_new(64 * 1024, 4_096, 4_000_000)?)?;
    assert!(!snapshot.preview_bytes_truncated());
    assert!(!snapshot.preview_items_truncated());
    assert!(
        snapshot
            .preview()
            .contains(&format!("channel[{source_index}]"))
    );
    assert!(!snapshot.preview().contains("channel[0]"));
    assert!(!scene.provenance_nodes().any(|node| matches!(
        node.kind(), ProvenanceKindRef::Operation(operation)
            if operation.kind() == ProvenanceOperationKindRef::Repair
    )));
    Ok(())
}

#[test]
fn mixed_core_channels_keep_duration_source_ordinal_and_payload_in_json_and_glb() -> TestResult<()>
{
    for triangle in [false, true] {
        for (source, bytes) in containers(&document(triangle))? {
            let mut options = GltfTestOptions::memory();
            options.limits.max_animation_cells = usize::try_from(RETAINED_CELLS)?;
            let (report, diagnostics) = import_assets(source, [(source, bytes)], options)?;
            assert!(diagnostics.is_empty());
            assert!(report.loss_evidence().is_empty());
            assert_eq!(report.scene().mesh_count(), usize::from(triangle));
            assert_retained_channel(&report, 1)?;
            let cells = report
                .ledger()
                .dimension(ImportDimension::AnimationCells)
                .unwrap();
            assert_eq!(
                (cells.attempted(), cells.consumed()),
                (RETAINED_CELLS, RETAINED_CELLS)
            );
            let scope = report.source_scope_record().unwrap();
            let channels = scope.counts(SourceScopeEntityKind::AnimationChannel);
            assert_eq!((channels.selected(), channels.omitted()), (1, 1));
            let samplers = scope.counts(SourceScopeEntityKind::AnimationSampler);
            assert_eq!((samplers.selected(), samplers.omitted()), (1, 1));
            assert_eq!(
                scope
                    .details()
                    .filter(|detail| detail.kind() == SourceScopeEntityKind::AnimationChannel)
                    .map(|detail| (detail.disposition(), detail.identity()))
                    .collect::<Vec<_>>(),
                [
                    (SourceScopeDisposition::Omitted, "a0c0"),
                    (SourceScopeDisposition::Selected, "a0c1")
                ]
            );
            assert_no_dependencies(&report);
            assert_released(&report);
        }
    }
    Ok(())
}

#[test]
fn ignored_clip_then_shared_sampler_keeps_later_clip_and_channel_source_identity() -> TestResult<()>
{
    let mut document = document(false);
    let mut ignored = document["animations"][0].clone();
    ignored["name"] = json!("Ignored");
    ignored["channels"].as_array_mut().unwrap().truncate(1);
    let mut mixed = document["animations"][0].clone();
    // The ignored and retained channel now use the same real sampler. Its resource is
    // selected once, while the earlier clip's sidecar remains entirely unselected.
    mixed["channels"][0]["sampler"] = json!(1);
    document["animations"] = json!([ignored, mixed]);
    for (source, bytes) in containers(&document)? {
        let (report, diagnostics) = import_assets(source, [(source, bytes)], sidecar_options())?;
        assert!(diagnostics.is_empty());
        assert!(report.loss_evidence().is_empty());
        let scene = report.scene();
        assert_eq!(scene.animation_clip_count(), 1);
        let clip = scene.animation_clips().next().unwrap();
        assert_eq!(clip.name(), Some("Mixed"));
        assert_eq!(clip.duration(), 1.0);
        assert_eq!(clip.channel_count(), 1);
        let channel = clip.channel(0).unwrap();
        assert_eq!(channel.key_times(), &[0.0, 1.0]);
        assert_eq!(channel.values(), &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let locators = scene
            .owner_provenance(channel.metadata_owner())?
            .filter_map(|provenance| match provenance.kind() {
                ProvenanceKindRef::Source(source) => match source.locator() {
                    SourceObjectLocatorRef::Namespaced { value, .. }
                        if value.starts_with("kind=animation-channel;") =>
                    {
                        Some(value)
                    }
                    _ => None,
                },
                ProvenanceKindRef::Operation(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(locators.len(), 1);
        assert!(locators[0].starts_with("kind=animation-channel;parent=1;index=1;"));
        let snapshot =
            scene.capture_snapshot(SceneSnapshotLimits::try_new(64 * 1024, 4_096, 4_000_000)?)?;
        assert!(!snapshot.preview_bytes_truncated());
        assert!(!snapshot.preview_items_truncated());
        assert!(snapshot.preview().contains("channel[2]"));
        let scope = report.source_scope_record().unwrap();
        let channels = scope.counts(SourceScopeEntityKind::AnimationChannel);
        assert_eq!((channels.selected(), channels.omitted()), (1, 2));
        let samplers = scope.counts(SourceScopeEntityKind::AnimationSampler);
        assert_eq!((samplers.selected(), samplers.omitted()), (1, 3));
        assert!(scope.details().any(|detail| {
            detail.kind() == SourceScopeEntityKind::AnimationChannel
                && detail.disposition() == SourceScopeDisposition::Selected
                && detail.identity() == "a1c1"
        }));
        let cells = report
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .unwrap();
        assert_eq!(
            (cells.attempted(), cells.consumed()),
            (RETAINED_CELLS, RETAINED_CELLS)
        );
        assert_no_dependencies(&report);
        assert_released(&report);
    }
    Ok(())
}

#[test]
fn an_entirely_untargeted_clip_has_no_runtime_animation_or_resource_reads() -> TestResult<()> {
    for triangle in [false, true] {
        for path in ["translation", "rotation", "scale", "weights"] {
            let mut document = document(triangle);
            all_ignored(&mut document);
            document["animations"][0]["channels"][0]["target"]["path"] = json!(path);
            for (source, bytes) in containers(&document)? {
                let mut options = GltfTestOptions::memory();
                options.limits.max_animation_cells = 0;
                let (report, diagnostics) = import_assets(source, [(source, bytes)], options)?;
                assert!(diagnostics.is_empty());
                assert!(report.loss_evidence().is_empty());
                assert_eq!(report.scene().animation_clip_count(), 0);
                assert_eq!(report.scene().mesh_count(), usize::from(triangle));
                let cells = report
                    .ledger()
                    .dimension(ImportDimension::AnimationCells)
                    .unwrap();
                assert_eq!((cells.attempted(), cells.consumed()), (0, 0));
                let scope = report.source_scope_record().unwrap();
                assert_eq!(scope.counts(SourceScopeEntityKind::Animation).selected(), 0);
                assert_eq!(
                    scope
                        .counts(SourceScopeEntityKind::AnimationChannel)
                        .omitted(),
                    1
                );
                assert_eq!(
                    scope
                        .counts(SourceScopeEntityKind::AnimationSampler)
                        .selected(),
                    0
                );
                assert_no_dependencies(&report);
                assert_released(&report);
            }
        }
    }
    Ok(())
}

#[test]
fn ignored_sidecars_are_not_opened_even_when_dependency_io_is_enabled() -> TestResult<()> {
    for (source, bytes) in containers(&document(false))? {
        let (report, diagnostics) = import_assets(
            source,
            [(source, bytes), ("ignored.bin", vec![0xff])],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        assert_retained_channel(&report, 1)?;
        assert_no_dependencies(&report);
        assert_released(&report);
    }
    Ok(())
}

#[test]
fn ignored_channels_still_require_globally_valid_references_and_typed_nodes() -> TestResult<()> {
    for entirely_ignored in [false, true] {
        for (pointer, value) in [
            ("/animations/0/channels/0/sampler", json!(99)),
            ("/animations/0/samplers/0/input", json!(99)),
            ("/animations/0/samplers/0/output", json!(99)),
            ("/animations/0/samplers/1/input", json!(99)),
            ("/accessors/3/bufferView", json!(99)),
            ("/bufferViews/3/buffer", json!(99)),
        ] {
            let mut document = document(false);
            if entirely_ignored {
                all_ignored(&mut document);
            }
            *document.pointer_mut(pointer).unwrap() = value;
            for (source, bytes) in containers(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], sidecar_options())?;
                assert!(diagnostics.is_empty());
                let failure = expected_error(result)?;
                assert_gltf_failure(
                    &failure,
                    GltfFailureCode::InvalidDocument,
                    Some("core reference is out of range"),
                )?;
                assert_eq!(
                    failure
                        .ledger()
                        .dimension(ImportDimension::DependencyReference)
                        .unwrap()
                        .attempted(),
                    0
                );
                assert_failure_released(&failure);
            }
        }
        for node in [
            json!(99),
            Value::Null,
            json!(false),
            json!("0"),
            json!([]),
            json!({}),
            json!(-1),
            json!(0.5),
        ] {
            let mut document = document(false);
            if entirely_ignored {
                all_ignored(&mut document);
            }
            document["animations"][0]["channels"][0]["target"]["node"] = node;
            for (source, bytes) in containers(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], sidecar_options())?;
                assert!(diagnostics.is_empty());
                let failure = expected_error(result)?;
                assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
                assert_eq!(
                    failure
                        .ledger()
                        .dimension(ImportDimension::DependencyReference)
                        .unwrap()
                        .attempted(),
                    0
                );
                assert_failure_released(&failure);
            }
        }
    }
    Ok(())
}

#[test]
fn untargeted_channels_do_not_silently_admit_extension_targeting() -> TestResult<()> {
    for entirely_ignored in [false, true] {
        for site in ["channel", "target"] {
            for extension in [
                "KHR_animation_pointer",
                "VENDOR_target",
                "KHR_materials_unlit",
            ] {
                let mut document = document(false);
                if entirely_ignored {
                    all_ignored(&mut document);
                }
                document["extensionsUsed"] = json!([extension]);
                let channel = &mut document["animations"][0]["channels"][0];
                let owner = if site == "target" {
                    &mut channel["target"]
                } else {
                    channel
                };
                owner["extensions"] = json!({(extension): {"pointer": "/nodes/0/translation"}});
                for (source, bytes) in containers(&document)? {
                    let (result, diagnostics) =
                        import_assets_result(source, [(source, bytes)], sidecar_options())?;
                    assert!(diagnostics.is_empty());
                    let failure = expected_error(result)?;
                    assert_gltf_failure(
                        &failure,
                        GltfFailureCode::UnsupportedFeature,
                        Some("outside the admitted profile"),
                    )?;
                    assert_eq!(
                        failure
                            .ledger()
                            .dimension(ImportDimension::DependencyReference)
                            .unwrap()
                            .attempted(),
                        0
                    );
                    assert_failure_released(&failure);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn selected_and_scope_omitted_node_targets_keep_the_existing_extension_policy() -> TestResult<()> {
    for node in [0, 1] {
        let mut document = document(false);
        all_ignored(&mut document);
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "Outside"}));
        document["extensionsUsed"] = json!(["VENDOR_target"]);
        let target = &mut document["animations"][0]["channels"][0]["target"];
        target["node"] = json!(node);
        target["extensions"] = json!({"VENDOR_target": {}});
        for (source, bytes) in containers(&document)? {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], sidecar_options())?;
            if node == 0 {
                assert!(diagnostics.is_empty());
                let failure = expected_error(result)?;
                assert_gltf_failure(
                    &failure,
                    GltfFailureCode::UnsupportedFeature,
                    Some("outside the admitted profile"),
                )?;
                assert_failure_released(&failure);
            } else {
                let report = result?;
                assert_eq!(report.scene().animation_clip_count(), 0);
                assert!(report.loss_evidence().is_empty());
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(
                    diagnostics[0].code().as_str(),
                    "gltf.extension_uninterpreted_declaration"
                );
                assert_no_dependencies(&report);
                assert_released(&report);
            }
        }
    }
    Ok(())
}

#[test]
fn empty_extension_containers_are_core_only_but_authored_null_is_invalid() -> TestResult<()> {
    for site in ["channel", "target"] {
        for extensions in [json!({}), Value::Null, json!([]), json!(false)] {
            let mut document = document(false);
            all_ignored(&mut document);
            let channel = &mut document["animations"][0]["channels"][0];
            let owner = if site == "target" {
                &mut channel["target"]
            } else {
                channel
            };
            owner["extensions"] = extensions.clone();
            for (source, bytes) in containers(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(diagnostics.is_empty());
                if extensions.is_object() {
                    let report = result?;
                    assert_eq!(report.scene().animation_clip_count(), 0);
                    assert_released(&report);
                } else {
                    let failure = expected_error(result)?;
                    assert_gltf_failure(
                        &failure,
                        GltfFailureCode::InvalidDocument,
                        Some("extensions must be an object"),
                    )?;
                    assert_failure_released(&failure);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn unsupported_required_extensions_fail_even_when_all_channels_are_ignored() -> TestResult<()> {
    let mut document = document(false);
    all_ignored(&mut document);
    document["extensionsUsed"] = json!(["KHR_animation_pointer"]);
    document["extensionsRequired"] = json!(["KHR_animation_pointer"]);
    for (source, bytes) in containers(&document)? {
        let (result, diagnostics) =
            import_assets_result(source, [(source, bytes)], sidecar_options())?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::UnsupportedFeature,
            Some("required extension"),
        )?;
        assert_failure_released(&failure);
    }
    Ok(())
}

#[test]
fn core_ignoring_preserves_the_existing_outside_scene_loss_and_duration_policy() -> TestResult<()> {
    let mut document = document(false);
    document["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "Outside"}));
    document["animations"][0]["channels"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "sampler": 0, "target": {"node": 1, "path": "scale"}
        }));
    let outside = [0.0_f32, 100.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    for (source, bytes) in containers(&document)? {
        let (result, diagnostics) =
            import_assets_result(source, [(source, bytes.clone())], sidecar_options())?;
        let failure = expected_error(result)?;
        assert!(matches!(failure.cause(), ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { .. })));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].object_path(),
            Some("/animations/0/channels/2")
        );
        assert_failure_released(&failure);

        let mut options = sidecar_options();
        options.loss_policy =
            LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss);
        let (report, diagnostics) = import_assets(
            source,
            [(source, bytes), ("ignored.bin", outside.clone())],
            options,
        )?;
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(report.loss_evidence().len(), 1);
        let clip = report.scene().animation_clips().next().unwrap();
        assert_eq!(clip.duration(), 100.0);
        assert_eq!(clip.channel_count(), 1);
        let cells = report
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .unwrap();
        assert_eq!((cells.attempted(), cells.consumed()), (10, 10));
        assert_released(&report);
    }
    Ok(())
}

#[test]
fn retained_animation_limits_still_fail_without_admitting_ignored_payloads() -> TestResult<()> {
    for (source, bytes) in containers(&document(false))? {
        let mut options = GltfTestOptions::memory();
        options.limits.max_animation_cells = usize::try_from(RETAINED_CELLS - 1)?;
        let (result, diagnostics) = import_assets_result(source, [(source, bytes)], options)?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_resource_limit(&failure, "max_animation_cells");
        let cells = failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .unwrap();
        assert_eq!((cells.attempted(), cells.consumed()), (RETAINED_CELLS, 0));
        assert_failure_released(&failure);
    }
    Ok(())
}

#[test]
fn cancellation_with_ignored_channels_releases_all_import_storage() -> TestResult<()> {
    let (source, bytes) = containers(&document(false))?.into_iter().last().unwrap();
    let (control, polls) = controlled_import(None);
    let (result, diagnostics) = import_assets_result_with_control(
        source,
        [(source, bytes.clone())],
        GltfTestOptions::memory(),
        control,
    )?;
    assert!(diagnostics.is_empty());
    let report = result?;
    assert_released(&report);
    let total_polls = polls.load(Ordering::SeqCst);
    assert!(total_polls > 1);
    for cancel_on in [1, total_polls / 2, total_polls] {
        let (control, polls) = controlled_import(Some(cancel_on));
        let (result, diagnostics) = import_assets_result_with_control(
            source,
            [(source, bytes.clone())],
            GltfTestOptions::memory(),
            control,
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert!(matches!(failure.cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        }))));
        assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
        assert_failure_released(&failure);
    }
    Ok(())
}
