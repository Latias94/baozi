mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::authoring::{MorphDeltaSemantic, Vec3};
use baozi_import::{ImportCause, ImportDimension, ImportReport, ScopeAbortCause};
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, assert_gltf_failure, assert_resource_limit,
    controlled_import, expected_error, glb_with_json, import_assets, import_assets_result,
    import_assets_result_with_control, sidecar_options,
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

const MORPH_CELLS: usize = 57;

fn fixture() -> (Value, Vec<u8>) {
    let document = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0, 1]}],
        "nodes": [{"mesh": 0, "weights": [0.75, 0.5]}, {"mesh": 0}],
        "meshes": [{
            "name": "deformable",
            "weights": [0.25, -1.0],
            "primitives": [
                {"attributes": {"POSITION": 0, "NORMAL": 1, "TANGENT": 2},
                 "targets": [{"POSITION": 3, "NORMAL": 4, "TANGENT": 5}, {}]},
                {"attributes": {"POSITION": 0, "NORMAL": 1, "TANGENT": 2}},
                {"attributes": {"POSITION": 0, "NORMAL": 1, "TANGENT": 2},
                 "targets": [{"NORMAL": 4}, {"POSITION": 3}]}
            ]
        }],
        "buffers": [{"byteLength": 228}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 36},
            {"buffer": 0, "byteOffset": 72, "byteLength": 48},
            {"buffer": 0, "byteOffset": 120, "byteLength": 36},
            {"buffer": 0, "byteOffset": 156, "byteLength": 36},
            {"buffer": 0, "byteOffset": 192, "byteLength": 36}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"},
            {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0.0, 0.0, 0.5], "max": [0.0, 0.0, 1.5]},
            {"bufferView": 4, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 5, "componentType": 5126, "count": 3, "type": "VEC3"}
        ]
    });
    let streams: &[&[f32]] = &[
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
        &[0.0, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 1.5],
        &[0.125, 0.0, 0.0, 0.125, 0.0, 0.0, 0.125, 0.0, 0.0],
        &[0.0, 0.25, 0.0, 0.0, 0.25, 0.0, 0.0, 0.25, 0.0],
    ];
    let binary = streams
        .iter()
        .flat_map(|stream| stream.iter().flat_map(|value| value.to_le_bytes()))
        .collect::<Vec<_>>();
    assert_eq!(binary.len(), 228);
    (document, binary)
}

fn glb(document: &Value, binary: &[u8]) -> Vec<u8> {
    append_glb_chunk(
        glb_with_json(&serde_json::to_vec(document).unwrap()),
        b"BIN\0",
        binary,
    )
}

fn assert_morphs(report: &ImportReport) {
    let scene = report.scene();
    let mesh = scene.meshes().next().unwrap();
    assert_eq!(mesh.morph_target_count(), 2);
    assert_eq!(mesh.default_morph_weights(), &[0.25, -1.0]);
    let definitions = mesh.morph_target_definitions().collect::<Vec<_>>();
    assert_eq!(definitions[0].stable_identity(), 0);
    assert_eq!(definitions[1].stable_identity(), 1);
    assert!(
        definitions
            .iter()
            .all(|definition| definition.name().is_none())
    );
    let primitives = mesh.primitives().collect::<Vec<_>>();
    assert_eq!(primitives.len(), 3);
    assert!(
        primitives
            .iter()
            .all(|primitive| primitive.morph_target_count() == 2)
    );
    let first = primitives[0].morph_targets().next().unwrap();
    assert_eq!(first.id(), definitions[0].id());
    assert_eq!(first.delta_count(), 3);
    let positions = first.delta(MorphDeltaSemantic::Position).unwrap().data();
    assert_eq!(positions.value(0), Some(Vec3::new(0.0, 0.0, 0.5)));
    assert_eq!(positions.value(2), Some(Vec3::new(0.0, 0.0, 1.5)));
    assert_eq!(positions.value(3), None);
    assert_eq!(
        first
            .delta(MorphDeltaSemantic::Normal)
            .unwrap()
            .data()
            .value(1),
        Some(Vec3::new(0.125, 0.0, 0.0))
    );
    assert_eq!(
        first
            .delta(MorphDeltaSemantic::Tangent)
            .unwrap()
            .data()
            .value(1),
        Some(Vec3::new(0.0, 0.25, 0.0))
    );
    assert_eq!(
        primitives[0]
            .morph_targets()
            .next_back()
            .unwrap()
            .delta_count(),
        0
    );
    assert!(
        primitives[1]
            .morph_targets()
            .all(|target| target.delta_count() == 0)
    );
    let projected = primitives[2].morph_target(definitions[1].id()).unwrap();
    assert_eq!(projected.id(), definitions[1].id());
    assert_eq!(projected.delta_count(), 1);
    let instances = scene.mesh_instances().collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].morph_weights(), Some([0.75, 0.5].as_slice()));
    assert_eq!(instances[1].morph_weights(), None);
    let cells = report
        .ledger()
        .dimension(ImportDimension::MorphCells)
        .unwrap();
    assert_eq!(cells.attempted(), MORPH_CELLS as u64);
    assert_eq!(cells.consumed(), MORPH_CELLS as u64);
}

#[test]
fn imports_dense_morphs_zero_targets_and_instance_overrides() -> TestResult<()> {
    let (document, binary) = fixture();
    let mut options = GltfTestOptions::memory();
    options.limits.max_morph_cells = MORPH_CELLS;
    let (report, diagnostics) = import_assets(
        "morph.glb",
        [("morph.glb", glb(&document, &binary))],
        options,
    )?;
    assert!(diagnostics.is_empty());
    assert_morphs(&report);
    Ok(())
}

#[test]
fn external_morph_buffers_have_the_same_typed_values() -> TestResult<()> {
    let (mut document, binary) = fixture();
    document["buffers"][0]["uri"] = json!("morph.bin");
    let (report, diagnostics) = import_assets(
        "models/morph.gltf",
        [
            ("models/morph.gltf", serde_json::to_vec(&document)?),
            ("models/morph.bin", binary),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    assert_morphs(&report);
    Ok(())
}

#[test]
fn absent_mesh_weights_use_zero_defaults() -> TestResult<()> {
    let (mut document, binary) = fixture();
    document["meshes"][0]
        .as_object_mut()
        .unwrap()
        .remove("weights");
    let (report, _) = import_assets(
        "morph.glb",
        [("morph.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert_eq!(
        report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .default_morph_weights(),
        &[0.0, 0.0]
    );
    Ok(())
}

#[test]
fn malformed_morph_contracts_fail_before_sidecar_io() -> TestResult<()> {
    let mutations: &[fn(&mut Value)] = &[
        |doc| doc["meshes"][0]["weights"] = json!([0.5]),
        |doc| doc["nodes"][0]["weights"] = json!([0.5]),
        |doc| doc["meshes"][0]["primitives"][2]["targets"] = json!([{}]),
        |doc| doc["meshes"][0]["primitives"][2]["targets"] = json!([]),
        |doc| doc["accessors"][3]["count"] = json!(2),
        |doc| doc["accessors"][5]["type"] = json!("VEC4"),
        |doc| doc["accessors"][3]["min"] = json!([0.0, 0.0, 2.0]),
        |doc| {
            doc["accessors"][3].as_object_mut().unwrap().remove("min");
        },
        |doc| {
            doc["meshes"][0]["primitives"][0]["attributes"]
                .as_object_mut()
                .unwrap()
                .remove("NORMAL");
        },
    ];
    for mutate in mutations {
        let (mut document, _) = fixture();
        document["buffers"][0]["uri"] = json!("unavailable.bin");
        mutate(&mut document);
        let (result, _) = import_assets_result(
            "morph.gltf",
            [("morph.gltf", serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        let error = expected_error(result)?;
        assert_gltf_failure(&error, GltfFailureCode::InvalidDocument, None)?;
        assert_eq!(
            error
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
    }
    Ok(())
}

#[test]
fn morph_cells_over_limit_fail_before_sidecar_io() -> TestResult<()> {
    let (mut document, _) = fixture();
    document["buffers"][0]["uri"] = json!("unavailable.bin");
    let mut options = sidecar_options();
    options.limits.max_morph_cells = MORPH_CELLS - 1;
    let (result, _) = import_assets_result(
        "morph.gltf",
        [("morph.gltf", serde_json::to_vec(&document)?)],
        options,
    )?;
    let error = expected_error(result)?;
    assert_resource_limit(&error, "max_morph_cells");
    assert_eq!(
        error
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .unwrap()
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn non_finite_delta_payloads_are_rejected() -> TestResult<()> {
    let (document, mut binary) = fixture();
    binary[120..124].copy_from_slice(&f32::NAN.to_le_bytes());
    let (result, _) = import_assets_result(
        "morph.glb",
        [("morph.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert_gltf_failure(
        &expected_error(result)?,
        GltfFailureCode::InvalidDocument,
        Some("finite VEC3"),
    )?;
    Ok(())
}

#[test]
fn morph_position_bounds_must_describe_the_decoded_values() -> TestResult<()> {
    let (mut document, binary) = fixture();
    document["accessors"][3]["max"] = json!([0.0, 0.0, 2.0]);
    let (result, _) = import_assets_result(
        "morph.glb",
        [("morph.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert_gltf_failure(
        &expected_error(result)?,
        GltfFailureCode::InvalidDocument,
        Some("bounds do not match"),
    )?;
    Ok(())
}

#[test]
fn unknown_morph_semantics_are_explicitly_rejected() -> TestResult<()> {
    let (mut document, binary) = fixture();
    document["meshes"][0]["primitives"][0]["targets"][0]["TEXCOORD_0"] = json!(3);
    let (result, _) = import_assets_result(
        "morph.glb",
        [("morph.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert_gltf_failure(
        &expected_error(result)?,
        GltfFailureCode::UnsupportedFeature,
        Some("morph attribute"),
    )?;
    Ok(())
}

#[test]
fn unselected_morph_payloads_do_not_enter_resource_closure() -> TestResult<()> {
    let (mut document, binary) = fixture();
    document["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"mesh": 1}));
    document["meshes"].as_array_mut().unwrap().push(json!({
        "primitives": [{"attributes": {"POSITION": 0}, "targets": [{"TEXCOORD_0": 3}]}]
    }));
    let (report, diagnostics) = import_assets(
        "morph.glb",
        [("morph.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    assert_eq!(report.scene().meshes().len(), 1);
    assert_morphs(&report);
    Ok(())
}

#[test]
fn cancellation_keeps_the_completed_morph_prefix() -> TestResult<()> {
    let (document, binary) = fixture();
    let bytes = glb(&document, &binary);
    let (control, polls) = controlled_import(None);
    let (result, _) = import_assets_result_with_control(
        "morph.glb",
        [("morph.glb", bytes.clone())],
        GltfTestOptions::memory(),
        control,
    )?;
    result?;
    let mut low = 1;
    let mut high = polls.load(Ordering::SeqCst);
    while low < high {
        let middle = low + (high - low) / 2;
        let (control, _) = controlled_import(Some(middle));
        let (result, _) = import_assets_result_with_control(
            "morph.glb",
            [("morph.glb", bytes.clone())],
            GltfTestOptions::memory(),
            control,
        )?;
        let error = expected_error(result)?;
        if error
            .ledger()
            .dimension(ImportDimension::MorphCells)
            .unwrap()
            .consumed()
            >= 15
        {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let (control, _) = controlled_import(Some(low));
    let (result, _) = import_assets_result_with_control(
        "morph.glb",
        [("morph.glb", bytes)],
        GltfTestOptions::memory(),
        control,
    )?;
    let error = expected_error(result)?;
    assert!(
        matches!(error.cause(), ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled { origin: CancellationOrigin::Signal })))
    );
    let cells = error
        .ledger()
        .dimension(ImportDimension::MorphCells)
        .unwrap();
    assert_eq!(cells.attempted(), (MORPH_CELLS - 2) as u64);
    assert!((15..(MORPH_CELLS - 2) as u64).contains(&cells.consumed()));
    Ok(())
}
