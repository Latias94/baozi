mod common;

use baozi_core::{DigestAlgorithm, ProvenanceKindRef, ProvenanceOperationKindRef, ProvenanceRef};
use baozi_format_gltf::{
    FORMAT_ID, GltfFailureCode, GltfImportOptions, GltfImporter, GltfSceneSelection,
};
use baozi_import::authoring::FormatId;
use baozi_import::{
    AnimationTarget, FormatFailureDetailValue, FormatSelection, ImportCause, ImportConfigOverlay,
    ImportDimension, ImportFailure, ImportLossDisposition, ImportLossKind, ImportLossSubject,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, ImportResult,
    LossPolicy, OptionCompileLimits, RegistryBuildLimits, RegistryBuilder, ResourceLimits,
    ResourceLimitsOverlay, ScopeAbortCause, SourceObjectLocatorRef, SourceScopeDetailState,
    SourceScopeDisposition, SourceScopeEntityKind, SourceScopeSelectionOutcome,
    SourceScopeSelectionRequest, import_asset, import_bytes,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use common::{
    TestResult, assert_gltf_failure, expected_error, glb_with_json, gltf_failure,
    skinned_triangle_bin, skinned_triangle_gltf, triangle_png,
};
use serde_json::{Value, json};
use std::sync::Arc;

const SOURCE: &str = "models/scene-selection.gltf";
const CHANNEL_OMITTED: &str = "gltf.animation_channel_outside_scene_omitted";

fn import_scene(
    document: &Value,
    selection: GltfSceneSelection,
    dependencies: impl IntoIterator<Item = (&'static str, Vec<u8>)>,
) -> TestResult<ImportResult<ImportReport>> {
    import_with_config(
        document,
        selection,
        dependencies,
        ImportConfigOverlay::default().with_loss_policy(LossPolicy::preserve_or_fail()),
    )
}

fn import_with_config(
    document: &Value,
    selection: GltfSceneSelection,
    dependencies: impl IntoIterator<Item = (&'static str, Vec<u8>)>,
    config: ImportConfigOverlay,
) -> TestResult<ImportResult<ImportReport>> {
    let io = MemoryAssetIo::new();
    let source = AssetPath::new(SOURCE)?;
    let _replaced = io.try_insert(
        source.clone(),
        Arc::<[u8]>::from(serde_json::to_vec(document)?),
    )?;
    for (path, bytes) in dependencies {
        let _replaced = io.try_insert(AssetPath::new(path)?, Arc::<[u8]>::from(bytes))?;
    }
    let mut registry = RegistryBuilder::new();
    registry.register(GltfImporter)?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    let envelope = GltfImportOptions::default()
        .with_scene_selection(selection)
        .to_envelope()?;
    let caller = ImportRequestLayer::new(
        config.with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT_ID)?)),
    )
    .try_with_format_option(envelope)?;
    let request = snapshot.compile_request(
        &ImportRequest::from_layers(ImportRequestLayer::default(), caller),
        OptionCompileLimits::default(),
    )?;
    Ok(import_asset(
        &request,
        &io,
        source,
        ImportOperationControl::default(),
    ))
}

fn assert_selection(report: &ImportReport, selection: GltfSceneSelection, scene_index: u64) {
    let scope = report.source_scope_record().expect("selection evidence");
    let expected = match selection {
        GltfSceneSelection::DocumentDefault => SourceScopeSelectionRequest::DocumentDefault,
        GltfSceneSelection::SceneIndex(index) => SourceScopeSelectionRequest::ExplicitScene {
            scene_index: u64::from(index),
        },
    };
    assert_eq!(scope.selection_request(), &expected);
    assert_eq!(
        scope.selection_outcome(),
        &SourceScopeSelectionOutcome::declared_scene(AssetPath::new(SOURCE).unwrap(), scene_index),
    );
}

fn assert_scope_counts(report: &ImportReport, expected: &[(SourceScopeEntityKind, u64, u64)]) {
    let scope = report.source_scope_record().expect("selection evidence");
    for &(kind, selected, omitted) in expected {
        assert_eq!(scope.counts(kind).selected(), selected, "{kind:?}");
        assert_eq!(scope.counts(kind).omitted(), omitted, "{kind:?}");
    }
}

fn scope_details(
    report: &ImportReport,
    kind: SourceScopeEntityKind,
) -> Vec<(SourceScopeDisposition, &str)> {
    report
        .source_scope_record()
        .expect("selection evidence")
        .details()
        .filter(|detail| detail.kind() == kind)
        .map(|detail| (detail.disposition(), detail.identity()))
        .collect()
}

fn assert_before_dependency_io(failure: &ImportFailure) {
    assert!(failure.source_scope_record().is_none());
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
    ] {
        assert_eq!(
            failure.ledger().dimension(dimension).unwrap().attempted(),
            0,
            "unexpected {dimension:?} before selection validation",
        );
    }
    let opens = failure
        .ledger()
        .dimension(ImportDimension::AssetOpen)
        .unwrap();
    assert_eq!(opens.attempted(), 1);
    assert_eq!(opens.consumed(), 1);
}

fn assert_invalid_scene(failure: &ImportFailure, index: u32) -> TestResult<()> {
    assert_gltf_failure(
        failure,
        GltfFailureCode::InvalidReference,
        Some("out of range"),
    )?;
    let envelope = gltf_failure(failure)?;
    assert!(envelope.details().iter().any(|detail| {
        detail.key() == "subject"
            && matches!(detail.value(), FormatFailureDetailValue::String(value)
                if value.as_str() == "scene-selection")
    }));
    assert!(envelope.details().iter().any(|detail| {
        detail.key() == "index"
            && matches!(detail.value(), FormatFailureDetailValue::Unsigned(value)
                if value == u64::from(index))
    }));
    assert!(failure.diagnostics().is_empty());
    assert_before_dependency_io(failure);
    Ok(())
}

fn generated_root(report: &ImportReport) -> ProvenanceRef<'_> {
    let mut roots = report.scene().provenance_nodes().filter(|node| {
        matches!(node.kind(), ProvenanceKindRef::Operation(operation)
            if operation.kind() == ProvenanceOperationKindRef::Generated)
    });
    let root = roots.next().expect("synthetic root operation");
    assert!(roots.next().is_none());
    root
}

#[test]
fn explicit_nonzero_scene_overrides_the_declared_default() -> TestResult<()> {
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}, {"nodes": [2]}],
        "nodes": [
            {"name": "Default"}, {"name": "Detached"},
            {"name": "Chosen", "children": [3]}, {"name": "ChosenChild"}
        ]
    });
    let report = import_scene(&document, GltfSceneSelection::SceneIndex(1), [])??;
    assert!(report.diagnostics().is_empty());
    assert_selection(&report, GltfSceneSelection::SceneIndex(1), 1);
    assert_scope_counts(
        &report,
        &[
            (SourceScopeEntityKind::Scene, 1, 1),
            (SourceScopeEntityKind::Node, 2, 2),
        ],
    );
    let scene = report.scene();
    assert_eq!(scene.node_count(), 2);
    assert_eq!(scene.root_count(), 1);
    let root = scene.roots().next().unwrap();
    assert_eq!(root.name(), Some("Chosen"));
    assert_eq!(
        root.children().map(|node| node.name()).collect::<Vec<_>>(),
        [Some("ChosenChild")],
    );
    assert_eq!(
        scope_details(&report, SourceScopeEntityKind::Node),
        [
            (SourceScopeDisposition::Omitted, "n0"),
            (SourceScopeDisposition::Omitted, "n1"),
            (SourceScopeDisposition::Selected, "n2"),
            (SourceScopeDisposition::Selected, "n3"),
        ],
    );
    Ok(())
}

#[test]
fn glb_uses_the_same_frozen_selector_and_records_its_container_identity() -> TestResult<()> {
    let document = br#"{
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}, {"nodes": [1]}],
        "nodes": [{"name": "Default"}, {"name": "ExplicitGlb"}]
    }"#;
    let mut registry = RegistryBuilder::new();
    registry.register(GltfImporter)?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    let caller = ImportRequestLayer::default().try_with_format_option(
        GltfImportOptions::default()
            .with_scene_selection(GltfSceneSelection::SceneIndex(1))
            .to_envelope()?,
    )?;
    let request = snapshot.compile_request(
        &ImportRequest::new().with_caller_layer(caller),
        OptionCompileLimits::default(),
    )?;
    let source = AssetPath::new("models/explicit.glb")?;
    let report = import_bytes(
        &request,
        source.clone(),
        &glb_with_json(document),
        ImportOperationControl::default(),
    )?;
    let record = report.source_scope_record().unwrap();
    assert_eq!(
        record.selection_request(),
        &SourceScopeSelectionRequest::ExplicitScene { scene_index: 1 },
    );
    assert_eq!(
        record.selection_outcome(),
        &SourceScopeSelectionOutcome::declared_scene(source, 1),
    );
    assert_eq!(
        report
            .scene()
            .roots()
            .map(|node| node.name())
            .collect::<Vec<_>>(),
        [Some("ExplicitGlb")],
    );
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn explicit_indices_resolve_both_scenes_without_a_declared_default() -> TestResult<()> {
    let document = json!({
        "asset": {"version": "2.0"},
        "scenes": [{"nodes": [1]}, {"nodes": [0]}],
        "nodes": [{"name": "Second"}, {"name": "First"}]
    });
    let failure = expected_error(import_scene(
        &document,
        GltfSceneSelection::DocumentDefault,
        [],
    )?)?;
    assert_gltf_failure(
        &failure,
        GltfFailureCode::SceneSelectionRequired,
        Some("multiple scenes"),
    )?;
    assert!(gltf_failure(&failure)?.details().iter().any(|detail| {
        detail.key() == "subject"
            && matches!(detail.value(), FormatFailureDetailValue::String(value)
                if value.as_str() == "scene-selection")
    }));
    assert!(failure.diagnostics().is_empty());
    assert_before_dependency_io(&failure);
    for (index, name) in [(0, "First"), (1, "Second")] {
        let report = import_scene(&document, GltfSceneSelection::SceneIndex(index), [])??;
        assert!(report.diagnostics().is_empty());
        assert_selection(
            &report,
            GltfSceneSelection::SceneIndex(index),
            u64::from(index),
        );
        assert_scope_counts(
            &report,
            &[
                (SourceScopeEntityKind::Scene, 1, 1),
                (SourceScopeEntityKind::Node, 1, 1),
            ],
        );
        assert_eq!(
            report
                .scene()
                .roots()
                .map(|node| node.name())
                .collect::<Vec<_>>(),
            [Some(name)],
        );
    }
    Ok(())
}

#[test]
fn explicit_zero_never_falls_back_to_parentless_nodes_without_scenes() -> TestResult<()> {
    for empty_array in [false, true] {
        let mut document = json!({
            "asset": {"version": "2.0"}, "nodes": [{"name": "Parentless"}],
            "buffers": [{"uri": "must-not-open.bin", "byteLength": 4}]
        });
        if empty_array {
            document["scenes"] = json!([]);
        }
        let failure = expected_error(import_scene(
            &document,
            GltfSceneSelection::SceneIndex(0),
            [],
        )?)?;
        assert_invalid_scene(&failure, 0)?;
    }
    Ok(())
}

#[test]
fn explicit_scene_at_length_or_u32_max_is_rejected_before_dependencies() -> TestResult<()> {
    let document = resource_document();
    for index in [2, u32::MAX] {
        let failure = expected_error(import_scene(
            &document,
            GltfSceneSelection::SceneIndex(index),
            [],
        )?)?;
        assert_invalid_scene(&failure, index)?;
    }
    Ok(())
}

#[test]
fn explicit_empty_scene_keeps_a_synthetic_root_without_importing_other_nodes() -> TestResult<()> {
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}, {"name": "Empty"}],
        "nodes": [{"name": "Omitted"}]
    });
    let report = import_scene(&document, GltfSceneSelection::SceneIndex(1), [])??;
    assert!(report.diagnostics().is_empty());
    assert_selection(&report, GltfSceneSelection::SceneIndex(1), 1);
    assert_scope_counts(
        &report,
        &[
            (SourceScopeEntityKind::Scene, 1, 1),
            (SourceScopeEntityKind::Node, 0, 1),
        ],
    );
    assert_eq!(report.scene().node_count(), 0);
    assert_eq!(report.scene().root_count(), 0);
    assert!(report.scene().canonical_block_count() > 0);
    let root = generated_root(&report);
    assert_eq!(root.parents().len(), 2);
    assert!(root.parents().any(|parent| {
        matches!(parent.kind(), ProvenanceKindRef::Source(source)
            if matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                if value.starts_with("kind=scene;parent=-;index=1;")))
    }));
    Ok(())
}

#[test]
fn shared_roots_are_imported_once_in_the_explicit_scenes_authored_order() -> TestResult<()> {
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0, 2, 5]}, {"nodes": [2, 0]}],
        "nodes": [
            {"name": "RootA", "children": [4, 1]}, {"name": "SecondChild"},
            {"name": "RootB", "children": [3]}, {"name": "BChild"},
            {"name": "FirstChild"}, {"name": "Omitted"}
        ]
    });
    let report = import_scene(&document, GltfSceneSelection::SceneIndex(1), [])??;
    assert!(report.diagnostics().is_empty());
    assert_selection(&report, GltfSceneSelection::SceneIndex(1), 1);
    assert_scope_counts(
        &report,
        &[
            (SourceScopeEntityKind::Scene, 1, 1),
            (SourceScopeEntityKind::Node, 5, 1),
        ],
    );
    let scene = report.scene();
    assert_eq!(scene.node_count(), 5);
    let roots = scene.roots().collect::<Vec<_>>();
    assert_eq!(
        roots.iter().map(|node| node.name()).collect::<Vec<_>>(),
        [Some("RootB"), Some("RootA")],
    );
    assert_eq!(
        roots[0]
            .children()
            .map(|node| node.name())
            .collect::<Vec<_>>(),
        [Some("BChild")],
    );
    assert_eq!(
        roots[1]
            .children()
            .map(|node| node.name())
            .collect::<Vec<_>>(),
        [Some("FirstChild"), Some("SecondChild")],
    );
    for name in ["RootA", "RootB", "FirstChild", "SecondChild", "BChild"] {
        assert_eq!(
            scene
                .nodes()
                .filter(|node| node.name() == Some(name))
                .count(),
            1,
        );
    }
    assert_eq!(
        scope_details(&report, SourceScopeEntityKind::Node),
        [
            (SourceScopeDisposition::Selected, "n0"),
            (SourceScopeDisposition::Selected, "n1"),
            (SourceScopeDisposition::Selected, "n2"),
            (SourceScopeDisposition::Selected, "n3"),
            (SourceScopeDisposition::Selected, "n4"),
            (SourceScopeDisposition::Omitted, "n5"),
        ],
    );
    Ok(())
}

#[test]
fn structural_errors_in_an_omitted_scene_still_reject_the_document() -> TestResult<()> {
    for duplicate_root in [false, true] {
        let mut document = resource_document();
        let reason = if duplicate_root {
            document["scenes"][0]["nodes"] = json!([0, 0]);
            "duplicate root"
        } else {
            document["nodes"][0]["children"] = json!([0]);
            "cycle"
        };
        let failure = expected_error(import_scene(
            &document,
            GltfSceneSelection::SceneIndex(1),
            [],
        )?)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, Some(reason))?;
        assert!(failure.diagnostics().is_empty());
        assert_before_dependency_io(&failure);
    }
    Ok(())
}

fn resource_document() -> Value {
    json!({
        "asset": {"version": "2.0", "generator": "baozi-scene-selection-test"}, "scene": 0,
        "scenes": [{"nodes": [0]}, {"nodes": [1]}],
        "nodes": [{"name": "Omitted", "mesh": 0}, {"name": "Chosen", "mesh": 1}],
        "buffers": [
            {"uri": "omitted.bin", "byteLength": 60},
            {"uri": "chosen.bin", "byteLength": 60}
        ],
        "bufferViews": [
            {"buffer": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 24},
            {"buffer": 1, "byteLength": 36}, {"buffer": 1, "byteOffset": 36, "byteLength": 24}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"},
            {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
            {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2"}
        ],
        "images": [{"uri": "omitted.png"}, {"uri": "chosen.png"}],
        "samplers": [{"wrapS": 33071}, {"wrapS": 10497}],
        "textures": [{"source": 0, "sampler": 0}, {"source": 1, "sampler": 1}],
        "materials": [
            {"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}},
            {"pbrMetallicRoughness": {"baseColorTexture": {"index": 1}}}
        ],
        "meshes": [
            {"name": "OmittedMesh", "primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "material": 0}]},
            {"name": "ChosenMesh", "primitives": [{"attributes": {"POSITION": 2, "TEXCOORD_0": 3}, "material": 1}]}
        ]
    })
}

fn chosen_triangle() -> Vec<u8> {
    [
        0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
    ]
    .into_iter()
    .flat_map(f32::to_le_bytes)
    .collect()
}

#[test]
fn explicit_scene_acquires_only_its_mesh_material_texture_and_buffer_closure() -> TestResult<()> {
    let report = import_scene(
        &resource_document(),
        GltfSceneSelection::SceneIndex(1),
        [
            ("models/chosen.bin", chosen_triangle()),
            ("models/chosen.png", triangle_png()),
        ],
    )??;
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    assert_selection(&report, GltfSceneSelection::SceneIndex(1), 1);
    for (kind, omitted, selected) in [
        (SourceScopeEntityKind::Scene, "s0", "s1"),
        (SourceScopeEntityKind::Node, "n0", "n1"),
        (SourceScopeEntityKind::Mesh, "m0", "m1"),
        (SourceScopeEntityKind::Primitive, "m0p0", "m1p0"),
        (SourceScopeEntityKind::Material, "mat0", "mat1"),
        (SourceScopeEntityKind::Texture, "t0", "t1"),
        (SourceScopeEntityKind::Image, "i0", "i1"),
        (SourceScopeEntityKind::Sampler, "ts0", "ts1"),
        (SourceScopeEntityKind::Buffer, "b0", "b1"),
    ] {
        assert_scope_counts(&report, &[(kind, 1, 1)]);
        assert_eq!(
            scope_details(&report, kind),
            [
                (SourceScopeDisposition::Omitted, omitted),
                (SourceScopeDisposition::Selected, selected),
            ]
        );
    }
    assert_scope_counts(
        &report,
        &[
            (SourceScopeEntityKind::BufferView, 2, 2),
            (SourceScopeEntityKind::Accessor, 2, 2),
        ],
    );
    assert_eq!(
        scope_details(&report, SourceScopeEntityKind::Accessor),
        [
            (SourceScopeDisposition::Omitted, "acc0"),
            (SourceScopeDisposition::Omitted, "acc1"),
            (SourceScopeDisposition::Selected, "acc2"),
            (SourceScopeDisposition::Selected, "acc3"),
        ]
    );
    let scene = report.scene();
    assert_eq!(scene.node_count(), 1);
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.mesh_instance_count(), 1);
    assert_eq!(scene.material_count(), 1);
    assert_eq!(scene.image_count(), 1);
    assert_eq!(scene.sampler_count(), 1);
    assert_eq!(scene.texture_count(), 1);
    let mesh = scene.meshes().next().unwrap();
    assert_eq!(mesh.name(), Some("ChosenMesh"));
    assert_eq!(mesh.primitives().next().unwrap().vertex_count(), 3);
    let manifest = report.successful_input_manifest();
    let mut resources = manifest
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    resources.sort_unstable();
    assert_eq!(
        resources,
        ["models/chosen.bin", "models/chosen.png", SOURCE]
    );
    assert_eq!(manifest.counts().relation_occurrence_count(), 2);
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        3
    );
    Ok(())
}

#[test]
fn selection_request_survives_dependency_failure_and_detail_collapse() -> TestResult<()> {
    let mut digests = Vec::new();
    for collapse in [false, true] {
        let mut limits = ResourceLimits::default();
        limits.max_source_scope_detail_count = if collapse { 0 } else { 100 };
        let config = ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
            .with_loss_policy(
                LossPolicy::preserve_or_fail().allow(ImportLossKind::ProvenanceEvidenceLoss),
            );
        let failure = expected_error(import_with_config(
            &resource_document(),
            GltfSceneSelection::SceneIndex(1),
            [],
            config,
        )?)?;
        assert!(matches!(failure.cause(), ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::AssetOpen(_))));
        let scope = failure
            .source_scope_record()
            .expect("validated selection precedes dependency IO");
        assert_eq!(
            scope.selection_request(),
            &SourceScopeSelectionRequest::ExplicitScene { scene_index: 1 },
        );
        assert_eq!(
            scope.selection_outcome(),
            &SourceScopeSelectionOutcome::declared_scene(AssetPath::new(SOURCE)?, 1),
        );
        assert_eq!(scope.counts(SourceScopeEntityKind::Scene).selected(), 1);
        assert_eq!(scope.counts(SourceScopeEntityKind::Scene).omitted(), 1);
        assert_eq!(scope.counts(SourceScopeEntityKind::Buffer).selected(), 1);
        assert_eq!(scope.counts(SourceScopeEntityKind::Buffer).omitted(), 1);
        assert_eq!(
            scope.detail_state(),
            if collapse {
                SourceScopeDetailState::Collapsed
            } else {
                SourceScopeDetailState::Detailed
            },
        );
        // All 26 source declarations contribute to the digest even if no details survive.
        assert_eq!(scope.details().count(), if collapse { 0 } else { 26 });
        assert_eq!(
            scope.collapsed_detail_count(),
            if collapse { 26 } else { 0 }
        );
        assert_eq!(failure.diagnostics().len(), usize::from(collapse));
        assert_eq!(failure.loss_evidence().len(), usize::from(collapse));
        if collapse {
            assert_eq!(
                failure.diagnostics()[0].code().as_str(),
                "baozi.source_scope_details_collapsed",
            );
            assert_eq!(
                failure.loss_evidence()[0].kind(),
                ImportLossKind::ProvenanceEvidenceLoss,
            );
            assert_eq!(
                failure.loss_evidence()[0].subject(),
                ImportLossSubject::SourceScopeDetails,
            );
            assert_eq!(
                failure.loss_evidence()[0].disposition(),
                ImportLossDisposition::Applied,
            );
        }
        assert!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted()
                > 0
        );
        digests.push(scope.digest());
    }
    assert_eq!(digests[0], digests[1]);
    Ok(())
}

#[test]
fn explicit_skin_rejects_joints_and_skeletons_outside_the_selected_closure() -> TestResult<()> {
    for skeleton_outside in [false, true] {
        let mut document: Value = serde_json::from_slice(&skinned_triangle_gltf())?;
        document["nodes"][0]
            .as_object_mut()
            .unwrap()
            .remove("children");
        document["nodes"][1]
            .as_object_mut()
            .unwrap()
            .remove("children");
        document["scenes"] = json!([{"nodes": [0,1,2,3]}, {"nodes": [1,2]}]);
        let reason = if skeleton_outside {
            document["scenes"][1]["nodes"] = json!([1, 2, 3]);
            document["skins"][0]["skeleton"] = json!(0);
            "skeleton root node 0 outside the imported scene"
        } else {
            "joint node 3 outside the imported scene"
        };
        let failure = expected_error(import_scene(
            &document,
            GltfSceneSelection::SceneIndex(1),
            [],
        )?)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, Some(reason))?;
        assert!(failure.diagnostics().is_empty());
        assert_before_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn explicit_skin_closure_keeps_original_skin_and_node_ordinals() -> TestResult<()> {
    let mut document: Value = serde_json::from_slice(&skinned_triangle_gltf())?;
    document["scenes"] = json!([{"nodes": [4]}, {"nodes": [0]}]);
    document["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "OmittedJoint"}));
    document["skins"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!({"joints": [4], "skeleton": 4}));
    document["nodes"][2]["skin"] = json!(1);
    let report = import_scene(
        &document,
        GltfSceneSelection::SceneIndex(1),
        [("models/skin.bin", skinned_triangle_bin())],
    )??;
    assert!(report.diagnostics().is_empty());
    assert_selection(&report, GltfSceneSelection::SceneIndex(1), 1);
    assert_scope_counts(
        &report,
        &[
            (SourceScopeEntityKind::Scene, 1, 1),
            (SourceScopeEntityKind::Node, 4, 1),
            (SourceScopeEntityKind::Skin, 1, 1),
            (SourceScopeEntityKind::Mesh, 1, 0),
            (SourceScopeEntityKind::Buffer, 1, 0),
            (SourceScopeEntityKind::BufferView, 5, 0),
            (SourceScopeEntityKind::Accessor, 5, 0),
        ],
    );
    assert_eq!(
        scope_details(&report, SourceScopeEntityKind::Skin),
        [
            (SourceScopeDisposition::Omitted, "k0"),
            (SourceScopeDisposition::Selected, "k1"),
        ],
    );
    assert_eq!(report.scene().mesh_instance_count(), 1);
    Ok(())
}

fn animation_document() -> Value {
    json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}, {"nodes": [1]}],
        "nodes": [{"name": "Omitted"}, {"name": "Chosen"}],
        "buffers": [
            {"uri": "omitted-clip.bin", "byteLength": 32},
            {"uri": "chosen-animation.bin", "byteLength": 32},
            {"uri": "duration.bin", "byteLength": 8},
            {"uri": "omitted-output.bin", "byteLength": 24}
        ],
        "bufferViews": [
            {"buffer": 0, "byteLength": 8}, {"buffer": 0, "byteOffset": 8, "byteLength": 24},
            {"buffer": 1, "byteLength": 8}, {"buffer": 1, "byteOffset": 8, "byteLength": 24},
            {"buffer": 2, "byteLength": 8}, {"buffer": 3, "byteLength": 24}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
            {"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
            {"bufferView": 3, "componentType": 5126, "count": 2, "type": "VEC3"},
            {"bufferView": 4, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [4]},
            {"bufferView": 5, "componentType": 5126, "count": 2, "type": "VEC3"}
        ],
        "animations": [
            {"name": "OmittedClip", "samplers": [{"input": 0, "output": 1}],
             "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]},
            {"name": "Mixed", "samplers": [
                {"input": 4, "output": 5}, {"input": 2, "output": 3}, {"input": 2, "output": 3}
             ], "channels": [
                {"sampler": 0, "target": {"node": 0, "path": "translation"}},
                {"sampler": 1, "target": {"node": 1, "path": "translation"}},
                {"sampler": 2, "target": {"node": 1, "path": "scale"}}
             ]}
        ]
    })
}

#[test]
fn explicit_mixed_animation_requires_source_semantic_loss_permission() -> TestResult<()> {
    let failure = expected_error(import_scene(
        &animation_document(),
        GltfSceneSelection::SceneIndex(1),
        [],
    )?)?;
    assert_before_dependency_io(&failure);
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { code }
            if code.as_str() == CHANNEL_OMITTED)));
    assert_eq!(failure.diagnostics().len(), 1);
    assert_eq!(
        failure.diagnostics()[0].object_path(),
        Some("/animations/1/channels/0")
    );
    assert_eq!(failure.loss_evidence().len(), 1);
    let loss = &failure.loss_evidence()[0];
    assert_eq!(loss.kind(), ImportLossKind::SourceSemanticLoss);
    assert_eq!(
        loss.subject(),
        ImportLossSubject::SourceEntity(SourceScopeEntityKind::AnimationChannel),
    );
    assert_eq!(loss.code().as_str(), CHANNEL_OMITTED);
    assert_eq!(
        loss.disposition(),
        ImportLossDisposition::RejectedByLossPolicy
    );
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .unwrap()
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn allowed_mixed_animation_preserves_duration_order_and_source_identities() -> TestResult<()> {
    let chosen = [0.0_f32, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let duration = [0.0_f32, 4.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let report = import_with_config(
        &animation_document(),
        GltfSceneSelection::SceneIndex(1),
        [
            ("models/chosen-animation.bin", chosen),
            ("models/duration.bin", duration),
        ],
        ImportConfigOverlay::default().with_loss_policy(
            LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
        ),
    )??;
    assert_selection(&report, GltfSceneSelection::SceneIndex(1), 1);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(report.diagnostics()[0].code().as_str(), CHANNEL_OMITTED);
    assert_eq!(
        report.diagnostics()[0].object_path(),
        Some("/animations/1/channels/0")
    );
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(
        report.loss_evidence()[0].kind(),
        ImportLossKind::SourceSemanticLoss
    );
    assert_eq!(
        report.loss_evidence()[0].subject(),
        ImportLossSubject::SourceEntity(SourceScopeEntityKind::AnimationChannel),
    );
    assert_eq!(
        report.loss_evidence()[0].disposition(),
        ImportLossDisposition::Applied
    );
    assert_scope_counts(
        &report,
        &[
            (SourceScopeEntityKind::Scene, 1, 1),
            (SourceScopeEntityKind::Node, 1, 1),
            (SourceScopeEntityKind::Animation, 1, 1),
            (SourceScopeEntityKind::AnimationChannel, 2, 2),
            (SourceScopeEntityKind::AnimationSampler, 3, 1),
            (SourceScopeEntityKind::Buffer, 2, 2),
            (SourceScopeEntityKind::BufferView, 3, 3),
            (SourceScopeEntityKind::Accessor, 3, 3),
        ],
    );
    assert_eq!(
        scope_details(&report, SourceScopeEntityKind::AnimationChannel),
        [
            (SourceScopeDisposition::Omitted, "a0c0"),
            (SourceScopeDisposition::Omitted, "a1c0"),
            (SourceScopeDisposition::Selected, "a1c1"),
            (SourceScopeDisposition::Selected, "a1c2"),
        ]
    );
    let scene = report.scene();
    assert_eq!(scene.animation_clip_count(), 1);
    let clip = scene.animation_clips().next().unwrap();
    assert_eq!(clip.name(), Some("Mixed"));
    assert_eq!(clip.duration(), 4.0);
    assert_eq!(clip.channel_count(), 2);
    let target = scene.nodes().next().unwrap().id();
    for (index, expected_target) in [
        AnimationTarget::NodeTranslation(target),
        AnimationTarget::NodeScale(target),
    ]
    .into_iter()
    .enumerate()
    {
        let channel = clip.channel(index).unwrap();
        assert_eq!(channel.target(), expected_target);
        assert_eq!(channel.key_times(), &[0.0, 1.0]);
        assert_eq!(channel.values(), &[0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
        let prefix = format!("kind=animation-channel;parent=1;index={};", index + 1);
        assert!(
            scene
                .owner_provenance(channel.metadata_owner())?
                .any(|node| {
                    matches!(node.kind(), ProvenanceKindRef::Source(source)
                if matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                    if value.starts_with(&prefix)))
                })
        );
    }
    let mut resources = report
        .successful_input_manifest()
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    resources.sort_unstable();
    assert_eq!(
        resources,
        ["models/chosen-animation.bin", "models/duration.bin", SOURCE]
    );
    Ok(())
}

#[test]
fn channel_owners_use_global_rows_when_source_ordinals_repeat_across_clips() -> TestResult<()> {
    let mut document = animation_document();
    document["animations"] = json!([
        {"name": "FirstRetained", "samplers": [{"input": 2, "output": 3}],
         "channels": [{"sampler": 0, "target": {"node": 1, "path": "translation"}}]},
        {"name": "SecondRetained", "samplers": [{"input": 2, "output": 3}],
         "channels": [{"sampler": 0, "target": {"node": 1, "path": "scale"}}]}
    ]);
    let chosen = [0.0_f32, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let report = import_scene(
        &document,
        GltfSceneSelection::SceneIndex(1),
        [("models/chosen-animation.bin", chosen)],
    )??;
    let scene = report.scene();
    assert_eq!(scene.animation_clip_count(), 2);
    for (index, clip) in scene.animation_clips().enumerate() {
        for channel in [clip.channel(0).unwrap(), clip.channels().next().unwrap()] {
            let locators = scene
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
            assert_eq!(locators.len(), 1);
            assert!(
                locators[0].starts_with(&format!("kind=animation-channel;parent={index};index=0;"))
            );
        }
    }
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn requested_mode_changes_root_options_digest_but_keeps_actual_source_parents() -> TestResult<()> {
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 1,
        "scenes": [{"nodes": [0]}, {"nodes": [1]}],
        "nodes": [{"name": "Omitted"}, {"name": "Chosen"}]
    });
    let default = import_scene(&document, GltfSceneSelection::DocumentDefault, [])??;
    let explicit = import_scene(&document, GltfSceneSelection::SceneIndex(1), [])??;
    assert_selection(&default, GltfSceneSelection::DocumentDefault, 1);
    assert_selection(&explicit, GltfSceneSelection::SceneIndex(1), 1);
    for report in [&default, &explicit] {
        assert_eq!(report.scene().nodes().len(), 1);
        let roots = report.scene().roots().collect::<Vec<_>>();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name(), Some("Chosen"));
        assert_eq!(report.scene().nodes().next().unwrap().id(), roots[0].id());
    }
    assert_ne!(
        default.source_scope_record().unwrap().digest(),
        explicit.source_scope_record().unwrap().digest(),
    );
    let default_root = generated_root(&default);
    let explicit_root = generated_root(&explicit);
    let operation = |root: ProvenanceRef<'_>| match root.kind() {
        ProvenanceKindRef::Operation(operation) => operation.options_digest().1.to_vec(),
        ProvenanceKindRef::Source(_) => panic!("expected generated root operation"),
    };
    assert_ne!(operation(default_root), operation(explicit_root));
    let mut parent_sets = Vec::new();
    // Independent SHA-256 of the v1 code (no NUL) and the 18-byte BE64 instruction.
    for (root, expected) in [
        (
            default_root,
            [
                0x9b, 0x83, 0x96, 0x59, 0xbb, 0x4e, 0xf2, 0xf3, 0xae, 0xca, 0xa2, 0xe1, 0xdc, 0xa9,
                0x7b, 0x5e, 0x32, 0xa5, 0x49, 0x19, 0x75, 0x6c, 0x0d, 0x19, 0x8b, 0xd5, 0x18, 0xbf,
                0xa1, 0x24, 0xc9, 0x74,
            ],
        ),
        (
            explicit_root,
            [
                0x2c, 0xd8, 0x50, 0xec, 0x4c, 0x02, 0x83, 0x5f, 0xcb, 0xd6, 0x74, 0x36, 0x24, 0x43,
                0x88, 0xc9, 0xa9, 0xf2, 0xa3, 0xa6, 0x43, 0x53, 0xc2, 0x21, 0x9a, 0x4b, 0x08, 0x8a,
                0x0e, 0xf7, 0xb4, 0x3e,
            ],
        ),
    ] {
        let parents = root
            .parents()
            .map(|parent| match parent.kind() {
                ProvenanceKindRef::Source(source) => {
                    (source.resource().identity(), source.locator())
                }
                ProvenanceKindRef::Operation(_) => panic!("root parents must be source identities"),
            })
            .collect::<Vec<_>>();
        assert_eq!(parents.len(), 2);
        assert!(parents.iter().all(|(resource, _)| *resource == SOURCE));
        assert!(parents.iter().any(|(_, locator)| {
            matches!(locator, SourceObjectLocatorRef::Namespaced { value, .. }
                if value.starts_with("kind=scene;parent=-;index=1;"))
        }));
        parent_sets.push(parents);
        let ProvenanceKindRef::Operation(operation) = root.kind() else {
            unreachable!()
        };
        assert_eq!(
            operation.options_digest(),
            (DigestAlgorithm::Sha256, expected.as_slice()),
        );
        let evidence = operation.evidence().collect::<Vec<_>>();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].code(), "org.baozi.gltf:projection-root-v1");
        assert_eq!(evidence[0].occurrence_count(), 1);
        assert_eq!(
            evidence[0].digest(),
            Some((DigestAlgorithm::Sha256, expected.as_slice())),
        );
    }
    assert_eq!(parent_sets[0], parent_sets[1]);
    Ok(())
}
