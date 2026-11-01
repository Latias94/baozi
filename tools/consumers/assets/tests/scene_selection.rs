use std::sync::Arc;

use baozi::{
    AssetPath, GltfImportOptions, GltfSceneSelection, ImmutableMemoryAssetIo, ImportCause,
    ImportConfigOverlay, ImportDimension, ImportEvidence, ImportOperationControl, ImportPreset,
    ImportRequest, ImportRequestLayer, Importer, OptionCompileLimits, PreparedImportRequest,
    SourceScopeDisposition, SourceScopeEntityKind, SourceScopeSelectionOutcome,
    SourceScopeSelectionRequest, TopologyKind,
};
use baozi_asset_consumer::{ApplicationAsset, Result};
use serde_json::json;

const SOURCE: &str = "scene-selection/model.gltf";
const RESOURCES: [&str; 4] = ["shared", "default", "explicit", "unavailable"];
const POSITIONS: [[[f32; 3]; 3]; 4] = [
    [[0.0, 0.0, 1.0], [2.0, 0.0, 1.0], [0.0, 2.0, 1.0]],
    [[-2.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [-2.0, 1.0, 0.0]],
    [[3.0, 0.0, 2.0], [4.0, 0.0, 2.0], [3.0, 1.0, 2.0]],
    [[8.0, 0.0, 0.0], [9.0, 0.0, 0.0], [8.0, 1.0, 0.0]],
];
const COLORS: [[f32; 4]; 4] = [
    [0.25, 0.5, 0.75, 1.0],
    [1.0, 0.25, 0.0, 1.0],
    [0.0, 0.75, 0.5, 1.0],
    [1.0, 0.0, 1.0, 1.0],
];
const INDICES: [u16; 6] = [2, 0, 1, 2, 1, 0];

fn scene_selection_io(selected_scene: Option<usize>) -> Result<ImmutableMemoryAssetIo> {
    // Scenes zero and one share mesh zero and each own another mesh. Only the chosen
    // projection's two buffers exist; neither the other projection nor scene two can be loaded.
    let mut buffers = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut entries = Vec::new();
    for (index, name) in RESOURCES.into_iter().enumerate() {
        let positions = POSITIONS[index];
        let mut payload = Vec::new();
        for value in positions.into_iter().flatten() {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        for value in INDICES {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(payload.len(), 48);
        buffers.push(json!({"uri": format!("{name}.bin"), "byteLength": payload.len()}));
        views.push(json!({"buffer": index, "byteOffset": 0, "byteLength": 36}));
        views.push(json!({"buffer": index, "byteOffset": 36, "byteLength": 12}));
        accessors.push(json!({
            "bufferView": index * 2, "componentType": 5126, "count": 3, "type": "VEC3",
            "min": positions[0], "max": [positions[1][0], positions[2][1], positions[0][2]]
        }));
        accessors.push(json!({
            "bufferView": index * 2 + 1, "componentType": 5123, "count": 6, "type": "SCALAR"
        }));
        meshes.push(json!({"name": name, "primitives": [{
            "attributes": {"POSITION": index * 2}, "indices": index * 2 + 1,
            "material": index, "mode": 4
        }]}));
        materials.push(json!({"name": name, "pbrMetallicRoughness": {
            "baseColorFactor": COLORS[index], "metallicFactor": 0.25, "roughnessFactor": 0.75
        }}));
        if selected_scene.is_some_and(|scene| index == 0 || index == scene + 1) {
            entries.push((
                AssetPath::new(format!("scene-selection/{name}.bin"))?,
                Arc::<[u8]>::from(payload),
            ));
        }
    }
    let document = json!({
        "asset": {"version": "2.0", "extras": {"fixture": "scene-selection-consumer"}},
        "scene": 0,
        "scenes": [
            {"nodes": [0], "extras": {"scene": "default"}},
            {"nodes": [3], "extras": {"scene": "explicit"}},
            {"nodes": [6], "extras": {"scene": "unavailable"}}
        ],
        "nodes": [
            {"name": "default-root", "children": [1, 2], "translation": [10.0, 0.0, 0.0],
             "extras": {"projection": "default"}},
            {"name": "default-shared", "mesh": 0, "translation": [1.0, 0.0, 0.0]},
            {"name": "default-only", "mesh": 1},
            {"name": "explicit-root", "children": [4, 5], "translation": [-4.0, 2.0, 0.0],
             "extras": {"projection": "explicit"}},
            {"name": "explicit-shared", "mesh": 0, "translation": [0.0, 3.0, 0.0]},
            {"name": "explicit-only", "mesh": 2},
            {"name": "unavailable", "mesh": 3}
        ],
        "meshes": meshes, "materials": materials,
        "buffers": buffers, "bufferViews": views, "accessors": accessors
    });
    entries.push((
        AssetPath::new(SOURCE)?,
        Arc::from(serde_json::to_vec(&document)?),
    ));
    Ok(ImmutableMemoryAssetIo::try_from_entries(entries)?)
}

fn prepared_request(
    importer: &Importer,
    preset_selection: GltfSceneSelection,
    caller_selection: GltfSceneSelection,
) -> Result<PreparedImportRequest> {
    let preset = ImportPreset::new(ImportConfigOverlay::default()).try_with_format_option(
        GltfImportOptions::default()
            .with_scene_selection(preset_selection)
            .to_envelope()?,
    )?;
    let caller = ImportRequestLayer::new(ImportConfigOverlay::default()).try_with_format_option(
        GltfImportOptions::default()
            .with_scene_selection(caller_selection)
            .to_envelope()?,
    )?;
    let request = ImportRequest::new()
        .try_with_preset(&preset)?
        .with_caller_layer(caller);
    // The uncompiled request, preset, and envelopes all die before the first import.
    Ok(importer.prepare_request(&request, OptionCompileLimits::default())?)
}

fn assert_scope(
    evidence: &ImportEvidence,
    scene_index: u64,
    selection_request: SourceScopeSelectionRequest,
) -> Result<()> {
    assert!(evidence.diagnostics().is_empty());
    assert!(evidence.loss_evidence().is_empty());
    let record = evidence
        .source_scope_record()
        .ok_or("missing detached source-scope evidence")?;
    assert_eq!(record.selection_request(), &selection_request);
    assert_eq!(
        record.selection_outcome(),
        &SourceScopeSelectionOutcome::declared_scene(AssetPath::new(SOURCE)?, scene_index)
    );
    for (kind, selected, omitted) in [
        (SourceScopeEntityKind::Scene, 1, 2),
        (SourceScopeEntityKind::Node, 3, 4),
        (SourceScopeEntityKind::Mesh, 2, 2),
        (SourceScopeEntityKind::Primitive, 2, 2),
        (SourceScopeEntityKind::Material, 2, 2),
        (SourceScopeEntityKind::Buffer, 2, 2),
        (SourceScopeEntityKind::BufferView, 4, 4),
        (SourceScopeEntityKind::Accessor, 4, 4),
    ] {
        let counts = record.counts(kind);
        assert_eq!((counts.selected(), counts.omitted()), (selected, omitted));
    }
    let buffer_details: Vec<_> = record
        .details()
        .filter(|detail| detail.kind() == SourceScopeEntityKind::Buffer)
        .map(|detail| (detail.identity().to_owned(), detail.disposition()))
        .collect();
    let expected: Vec<_> = (0..4)
        .map(|index| {
            let disposition = if index == 0 || index == scene_index + 1 {
                SourceScopeDisposition::Selected
            } else {
                SourceScopeDisposition::Omitted
            };
            (format!("b{index}"), disposition)
        })
        .collect();
    assert_eq!(buffer_details, expected);
    for (dimension, expected) in [
        (ImportDimension::AssetOpen, 3),
        (ImportDimension::DependencyReference, 2),
    ] {
        let counts = evidence
            .ledger()
            .dimension(dimension)
            .ok_or("missing import accounting")?;
        assert_eq!(
            (counts.attempted(), counts.consumed()),
            (expected, expected)
        );
    }
    assert_eq!(evidence.ledger().current_live_bytes(), 0);
    assert_eq!(evidence.ledger().active_reservations(), 0);
    Ok(())
}

fn assert_owned_projection(asset: &ApplicationAsset, scene_index: usize) -> Result<()> {
    let branch = RESOURCES[scene_index + 1];
    let root_name = format!("{branch}-root");
    let shared_name = format!("{branch}-shared");
    let private_name = format!("{branch}-only");
    assert_eq!(
        (asset.mesh_count, asset.draws.len(), asset.instances.len()),
        (2, 2, 2)
    );
    assert_eq!(
        (asset.nodes.len(), asset.roots.len(), asset.materials.len()),
        (3, 1, 2)
    );
    assert!(asset.skins.is_empty());
    assert!(asset.animations.is_empty());
    assert!(asset.images.is_empty());
    let root = &asset.nodes[asset.roots[0]];
    assert_eq!(root.name.as_deref(), Some(root_name.as_str()));
    assert_eq!(
        root.children
            .iter()
            .map(|&index| asset.nodes[index].name.as_deref())
            .collect::<Vec<_>>(),
        [Some(shared_name.as_str()), Some(private_name.as_str())]
    );
    let translation = root
        .transform
        .authored_trs()
        .ok_or("selected root lost its authored TRS")?
        .translation();
    assert_eq!(
        [translation.x, translation.y, translation.z],
        [[10.0, 0.0, 0.0], [-4.0, 2.0, 0.0]][scene_index]
    );
    for (dense, source_mesh) in [0, scene_index + 1].into_iter().enumerate() {
        let draw = &asset.draws[dense];
        assert_eq!(
            (draw.mesh, draw.primitive, draw.material),
            (dense, 0, Some(dense))
        );
        assert_eq!(draw.topology, TopologyKind::TriangleList);
        assert_eq!(draw.indices, [0, 1, 2, 0, 2, 1]);
        assert_eq!(draw.vertices.len(), 3);
        // Reconstruct source Corner order through the application's own packed indices.
        for (&packed, source_vertex) in draw.indices.iter().zip(INDICES) {
            assert_eq!(
                draw.vertices[usize::try_from(packed)?].position,
                POSITIONS[source_mesh][usize::from(source_vertex)].map(f64::from)
            );
        }
        let material = &asset.materials[dense];
        assert_eq!(material.name.as_deref(), Some(RESOURCES[source_mesh]));
        assert!(material.bindings.is_empty());
        let pbr = material
            .factors
            .pbr_metallic_roughness()
            .ok_or("selected material lost its PBR factors")?;
        let color = pbr.base_color();
        assert_eq!([color.r, color.g, color.b, color.a], COLORS[source_mesh]);
        assert_eq!((pbr.metallic(), pbr.roughness()), (0.25, 0.75));
        let name = if dense == 0 {
            &shared_name
        } else {
            &private_name
        };
        let instance = asset
            .instances
            .iter()
            .find(|instance| asset.nodes[instance.node].name.as_deref() == Some(name.as_str()))
            .ok_or("missing selected mesh instance")?;
        assert_eq!((instance.mesh, instance.skin), (dense, None));
    }

    assert_eq!(asset.source_json_metadata.len(), 3);
    for (prefix, key, value) in [
        (
            "kind=document;parent=-;index=0;".to_owned(),
            "org.baozi.gltf:asset-extras",
            json!({"fixture": "scene-selection-consumer"}),
        ),
        (
            format!("kind=scene;parent=-;index={scene_index};"),
            "org.baozi.gltf:extras",
            json!({"scene": branch}),
        ),
        (
            format!("kind=node;parent=-;index={};", scene_index * 3),
            "org.baozi.gltf:extras",
            json!({"projection": branch}),
        ),
    ] {
        let metadata = asset
            .source_json_metadata
            .iter()
            .find(|metadata| metadata.locator.starts_with(&prefix))
            .ok_or("missing owned selected-source metadata")?;
        assert_eq!(metadata.resource, SOURCE);
        assert_eq!(metadata.format, "org.baozi.gltf");
        assert_eq!(metadata.locator_schema, "org.baozi.gltf:source-object");
        assert_eq!(metadata.locator_schema_version, 1);
        assert_eq!(metadata.key, key);
        assert_eq!(metadata.media_type, "application/json");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&metadata.payload)?,
            value
        );
    }
    Ok(())
}

#[test]
fn explicit_caller_selection_reuses_a_frozen_request_and_owns_only_its_projection() -> Result<()> {
    let outputs = {
        let importer = Importer::new()?;
        let prepared_request = prepared_request(
            &importer,
            GltfSceneSelection::DocumentDefault,
            GltfSceneSelection::SceneIndex(1),
        )?;
        let io = scene_selection_io(Some(1))?;
        let mut outputs = Vec::new();
        for _ in 0..2 {
            let report = importer.read_asset_with_request(
                &io,
                AssetPath::new(SOURCE)?,
                &prepared_request,
                ImportOperationControl::default(),
            )?;
            let (scene, evidence) = report.into_parts();
            let asset = ApplicationAsset::from_scene(&scene)?;
            outputs.push((asset, evidence));
        }
        outputs
    };
    // Reports, Scenes, the immutable IO namespace, importer, and prepared request are gone.
    assert_eq!(
        outputs[0]
            .1
            .source_scope_record()
            .ok_or("missing first scope")?
            .digest(),
        outputs[1]
            .1
            .source_scope_record()
            .ok_or("missing second scope")?
            .digest()
    );
    for (asset, evidence) in outputs {
        assert_scope(
            &evidence,
            1,
            SourceScopeSelectionRequest::ExplicitScene { scene_index: 1 },
        )?;
        drop(evidence);
        assert_owned_projection(&asset, 1)?;
    }
    Ok(())
}

#[test]
fn document_default_caller_overrides_explicit_preset_and_owns_default_projection() -> Result<()> {
    let (asset, evidence) = {
        let importer = Importer::new()?;
        let prepared_request = prepared_request(
            &importer,
            GltfSceneSelection::SceneIndex(1),
            GltfSceneSelection::DocumentDefault,
        )?;
        let io = scene_selection_io(Some(0))?;
        let report = importer.read_asset_with_request(
            &io,
            AssetPath::new(SOURCE)?,
            &prepared_request,
            ImportOperationControl::default(),
        )?;
        let (scene, evidence) = report.into_parts();
        (ApplicationAsset::from_scene(&scene)?, evidence)
    };
    assert_scope(&evidence, 0, SourceScopeSelectionRequest::DocumentDefault)?;
    drop(evidence);
    assert_owned_projection(&asset, 0)?;
    Ok(())
}

#[test]
fn out_of_range_selection_is_rejected_before_opening_any_dependency() -> Result<()> {
    let importer = Importer::new()?;
    let prepared_request = prepared_request(
        &importer,
        GltfSceneSelection::DocumentDefault,
        GltfSceneSelection::SceneIndex(3),
    )?;
    // All four declared buffers are absent, so dependency IO would mask a late range check.
    let io = scene_selection_io(None)?;
    let error = importer
        .read_asset_with_request(
            &io,
            AssetPath::new(SOURCE)?,
            &prepared_request,
            ImportOperationControl::default(),
        )
        .expect_err("scene index three is outside the three-scene document");
    let baozi::Error::Import(failure) = error else {
        panic!("scene selection must retain a complete public import failure");
    };
    assert!(matches!(failure.cause(), ImportCause::Format(_)));
    assert!(failure.source_scope_record().is_none());
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    for (dimension, expected) in [
        (ImportDimension::AssetOpen, 1),
        (ImportDimension::DependencyReference, 0),
        (ImportDimension::DependencyBytes, 0),
    ] {
        let counts = failure
            .ledger()
            .dimension(dimension)
            .ok_or("missing rejected-selection accounting")?;
        assert_eq!(
            (counts.attempted(), counts.consumed()),
            (expected, expected)
        );
    }
    Ok(())
}
