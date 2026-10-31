mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    AnimationInterpolation, AnimationTarget, ImportCause, ImportDiagnostic, ImportDimension,
    ImportFailure, ImportLossDisposition, ImportLossKind, ImportReport, ImportResult, LossPolicy,
    ScopeAbortCause, SourceScopeEntityKind,
};
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, assert_gltf_failure, assert_resource_limit,
    controlled_import, expected_error, glb_with_json, import_assets, import_assets_result,
    import_assets_result_with_control, sidecar_options,
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

const WEIGHTS: [f32; 4] = [-0.25, 0.5, 1.25, -2.0];
const CUBIC: [f32; 12] = [
    11.0, 12.0, // key 0 in tangent
    -0.25, 0.5, // key 0 value
    21.0, 22.0, // key 0 out tangent
    31.0, 32.0, // key 1 in tangent
    1.25, -2.0, // key 1 value
    41.0, 42.0, // key 1 out tangent
];
const ANIMATION_CELLS: u64 = 6;
const CUBIC_ANIMATION_CELLS: u64 = 14;
const OUTPUT_OFFSET: usize = 44;

fn fixture(interpolation: &str, output: &[f32]) -> (Value, Vec<u8>) {
    let mut binary = Vec::new();
    for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(binary.len(), OUTPUT_OFFSET);
    for value in output {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    let document = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [1, 2, 0]}],
        "nodes": [
            {"name": "A", "mesh": 0, "weights": [0.75, 0.5]},
            {"name": "Spacer"},
            {"name": "B", "mesh": 0}
        ],
        "meshes": [{
            "weights": [0.25, -0.5],
            "primitives": [
                {"attributes": {"POSITION": 0}},
                {"attributes": {"POSITION": 0}, "targets": [{}, {}]}
            ]
        }],
        "buffers": [{"byteLength": binary.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 8},
            {"buffer": 0, "byteOffset": OUTPUT_OFFSET, "byteLength": output.len() * 4}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0, 0, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 2, "type": "SCALAR",
             "min": [0], "max": [1]},
            {"bufferView": 2, "componentType": 5126, "count": output.len(), "type": "SCALAR"}
        ],
        "animations": [{
            "name": "Weights",
            "samplers": [{"input": 1, "output": 2, "interpolation": interpolation}],
            "channels": [{"sampler": 0, "target": {"node": 0, "path": "weights"}}]
        }]
    });
    (document, binary)
}

fn glb(document: &Value, binary: &[u8]) -> Vec<u8> {
    append_glb_chunk(
        glb_with_json(&serde_json::to_vec(document).unwrap()),
        b"BIN\0",
        binary,
    )
}

fn assert_cells(report: &ImportReport, expected: u64) {
    let cells = report
        .ledger()
        .dimension(ImportDimension::AnimationCells)
        .unwrap();
    assert_eq!(cells.attempted(), expected);
    assert_eq!(cells.consumed(), expected);
}

#[test]
fn shared_mesh_channels_target_distinct_instances_in_source_channel_order() -> TestResult<()> {
    let (mut document, binary) = fixture("LINEAR", &WEIGHTS);
    document["animations"][0]["channels"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "sampler": 0, "target": {"node": 2, "path": "weights"}
        }));
    let (report, diagnostics) = import_assets(
        "weights.glb",
        [("weights.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    assert!(report.loss_evidence().is_empty());
    assert_cells(&report, 12);
    let scene = report.scene();
    let instances = scene.mesh_instances().collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].node().name(), Some("B"));
    assert_eq!(instances[1].node().name(), Some("A"));
    assert_eq!(instances[0].morph_weights(), None);
    assert_eq!(instances[1].morph_weights(), Some(&[0.75, 0.5][..]));
    assert_eq!(
        scene.meshes().next().unwrap().default_morph_weights(),
        &[0.25, -0.5]
    );
    let clip = scene.animation_clips().next().unwrap();
    assert_eq!(clip.name(), Some("Weights"));
    assert_eq!(clip.duration(), 1.0);
    assert_eq!(clip.channel_count(), 2);
    for (index, instance) in [instances[1], instances[0]].iter().enumerate() {
        let channel = clip.channel(index).unwrap();
        assert_eq!(
            channel.target(),
            AnimationTarget::MeshInstanceMorphWeights(instance.id())
        );
        assert_eq!(
            scene.mesh_instance(instance.id()).unwrap().node().id(),
            instance.node().id()
        );
        assert_eq!(channel.interpolation(), AnimationInterpolation::Linear);
        assert_eq!(channel.value_width(), 2);
        assert_eq!(channel.key_count(), 2);
        assert_eq!(channel.key_times(), &[0.0, 1.0]);
        assert_eq!(channel.values(), &[-0.25, 0.5, 1.25, -2.0]);
        assert_eq!(channel.in_tangents(), None);
        assert_eq!(channel.out_tangents(), None);
    }
    Ok(())
}

#[test]
fn step_weights_allow_matrix_nodes_and_ignore_static_default_values() -> TestResult<()> {
    let (mut document, binary) = fixture("STEP", &WEIGHTS);
    document["nodes"][0]["matrix"] = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
    document["meshes"][0]
        .as_object_mut()
        .unwrap()
        .remove("weights");
    let (report, diagnostics) = import_assets(
        "weights.glb",
        [("weights.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    assert_cells(&report, ANIMATION_CELLS);
    let scene = report.scene();
    assert_eq!(
        scene.meshes().next().unwrap().default_morph_weights(),
        &[0.0, 0.0]
    );
    let channel = scene.animation_clips().next().unwrap().channel(0).unwrap();
    assert_eq!(channel.interpolation(), AnimationInterpolation::Step);
    assert_eq!(channel.values(), &[-0.25, 0.5, 1.25, -2.0]);
    assert_eq!(channel.value_width(), 2);
    assert_eq!(channel.in_tangents(), None);
    assert_eq!(channel.out_tangents(), None);
    Ok(())
}

#[test]
fn cubic_weights_split_complete_width_two_triplets() -> TestResult<()> {
    let (document, binary) = fixture("CUBICSPLINE", &CUBIC);
    let (report, diagnostics) = import_assets(
        "weights.glb",
        [("weights.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    assert_cells(&report, CUBIC_ANIMATION_CELLS);
    let channel = report
        .scene()
        .animation_clips()
        .next()
        .unwrap()
        .channel(0)
        .unwrap();
    assert_eq!(channel.interpolation(), AnimationInterpolation::CubicSpline);
    assert_eq!(channel.key_times(), &[0.0, 1.0]);
    assert_eq!(channel.key_count(), 2);
    assert_eq!(channel.value_width(), 2);
    assert_eq!(channel.values(), &[-0.25, 0.5, 1.25, -2.0]);
    assert_eq!(channel.in_tangents(), Some(&[11.0, 12.0, 31.0, 32.0][..]));
    assert_eq!(channel.out_tangents(), Some(&[21.0, 22.0, 41.0, 42.0][..]));
    Ok(())
}

#[test]
fn normalized_integer_weights_support_the_exact_khronos_component_profile() -> TestResult<()> {
    // Signed extrema have two encodings of -1.0. No output is inferred from the reader.
    let cases = [
        (5120, vec![128, 129, 0, 127], [-1.0, -1.0, 0.0, 1.0]),
        (5121, vec![0, 255, 255, 0], [0.0, 1.0, 1.0, 0.0]),
        (
            5122,
            vec![0, 128, 1, 128, 0, 0, 255, 127],
            [-1.0, -1.0, 0.0, 1.0],
        ),
        (
            5123,
            vec![0, 0, 255, 255, 255, 255, 0, 0],
            [0.0, 1.0, 1.0, 0.0],
        ),
    ];
    for (component, output, expected) in cases {
        let (mut document, mut binary) = fixture("LINEAR", &WEIGHTS);
        binary.truncate(OUTPUT_OFFSET);
        binary.extend_from_slice(&output);
        document["buffers"][0]["byteLength"] = json!(binary.len());
        document["bufferViews"][2]["byteLength"] = json!(output.len());
        document["accessors"][2]["componentType"] = json!(component);
        document["accessors"][2]["normalized"] = json!(true);
        let (report, diagnostics) = import_assets(
            "weights.glb",
            [("weights.glb", glb(&document, &binary))],
            GltfTestOptions::memory(),
        )?;
        assert!(diagnostics.is_empty());
        let channel = report
            .scene()
            .animation_clips()
            .next()
            .unwrap()
            .channel(0)
            .unwrap();
        assert_eq!(channel.values(), &expected, "component {component}");
        assert_eq!(channel.value_width(), 2);
        assert_cells(&report, ANIMATION_CELLS);
    }
    Ok(())
}

#[test]
fn malformed_weights_fail_before_external_output_io_or_cell_admission() -> TestResult<()> {
    let mutations: &[fn(&mut Value)] = &[
        |doc| doc["accessors"][2]["count"] = json!(3),
        |doc| doc["accessors"][2]["type"] = json!("VEC2"),
        |doc| doc["accessors"][2]["componentType"] = json!(5125),
        |doc| doc["accessors"][2]["normalized"] = json!(true),
        |doc| doc["accessors"][2]["componentType"] = json!(5120),
        |doc| doc["accessors"][2]["componentType"] = json!(5121),
        |doc| doc["accessors"][2]["componentType"] = json!(5122),
        |doc| doc["accessors"][2]["componentType"] = json!(5123),
        |doc| {
            doc["nodes"][0]
                .as_object_mut()
                .unwrap()
                .remove("mesh")
                .map(|_| ())
                .unwrap()
        },
        |doc| {
            doc["meshes"][0]["primitives"][1]
                .as_object_mut()
                .unwrap()
                .remove("targets");
        },
        |doc| doc["meshes"][0]["primitives"][0]["targets"] = json!([{}]),
        |doc| doc["animations"][0]["samplers"][0]["interpolation"] = json!("CUBICSPLINE"),
        |doc| {
            doc["animations"][0]["samplers"][0]["interpolation"] = json!("CUBICSPLINE");
            doc["accessors"][1]["count"] = json!(1);
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let (mut document, _) = fixture("LINEAR", &WEIGHTS);
        document["buffers"][0]["uri"] = json!("unavailable.bin");
        mutate(&mut document);
        let (result, diagnostics) = import_assets_result(
            "weights.gltf",
            [("weights.gltf", serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        let failure = expected_error(result)?;
        assert!(diagnostics.is_empty(), "mutation {index}");
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::AnimationCells)
                .unwrap()
                .attempted(),
            0
        );
    }
    Ok(())
}

#[test]
fn duplicate_weights_reject_but_translation_and_weights_can_share_a_node() -> TestResult<()> {
    let (mut document, binary) = fixture("LINEAR", &WEIGHTS);
    let channel = document["animations"][0]["channels"][0].clone();
    document["animations"][0]["channels"]
        .as_array_mut()
        .unwrap()
        .push(channel);
    let (result, diagnostics) = import_assets_result(
        "weights.glb",
        [("weights.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &failure,
        GltfFailureCode::InvalidDocument,
        Some("duplicate animation target"),
    )?;
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .unwrap()
            .attempted(),
        0
    );

    let (mut document, mut binary) = fixture("LINEAR", &WEIGHTS);
    let offset = binary.len();
    for value in [0.0_f32, 1.0, 2.0, 3.0, 4.0, 5.0] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    document["buffers"][0]["byteLength"] = json!(binary.len());
    document["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer": 0, "byteOffset": offset, "byteLength": 24}));
    document["accessors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView": 3, "componentType": 5126, "count": 2, "type": "VEC3"}));
    document["animations"][0]["samplers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"input": 1, "output": 3}));
    document["animations"][0]["channels"]
        .as_array_mut()
        .unwrap()
        .push(json!({"sampler": 1, "target": {"node": 0, "path": "translation"}}));
    let (report, diagnostics) = import_assets(
        "weights.glb",
        [("weights.glb", glb(&document, &binary))],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    let clip = report.scene().animation_clips().next().unwrap();
    assert_eq!(clip.channel_count(), 2);
    assert!(matches!(
        clip.channel(0).unwrap().target(),
        AnimationTarget::MeshInstanceMorphWeights(_)
    ));
    assert!(matches!(
        clip.channel(1).unwrap().target(),
        AnimationTarget::NodeTranslation(_)
    ));
    assert_eq!(
        clip.channel(1).unwrap().values(),
        &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]
    );
    Ok(())
}

#[test]
fn nonfinite_weights_and_cubic_tangents_preserve_consumed_prefixes() -> TestResult<()> {
    for (interpolation, output, scalar, invalid, reason, expected_prefix) in [
        (
            "LINEAR",
            WEIGHTS.as_slice(),
            1,
            f32::NAN,
            "non-finite animation value",
            3,
        ),
        (
            "LINEAR",
            WEIGHTS.as_slice(),
            3,
            f32::INFINITY,
            "non-finite animation value",
            5,
        ),
        (
            "CUBICSPLINE",
            CUBIC.as_slice(),
            1,
            f32::NEG_INFINITY,
            "non-finite animation in tangent",
            3,
        ),
        (
            "CUBICSPLINE",
            CUBIC.as_slice(),
            3,
            f32::NAN,
            "non-finite animation value",
            5,
        ),
        (
            "CUBICSPLINE",
            CUBIC.as_slice(),
            5,
            f32::INFINITY,
            "non-finite animation out tangent",
            7,
        ),
    ] {
        let (document, mut binary) = fixture(interpolation, output);
        let offset = OUTPUT_OFFSET + scalar * 4;
        binary[offset..offset + 4].copy_from_slice(&invalid.to_le_bytes());
        let (result, diagnostics) = import_assets_result(
            "weights.glb",
            [("weights.glb", glb(&document, &binary))],
            GltfTestOptions::memory(),
        )?;
        let failure = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, Some(reason))?;
        let cells = failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .unwrap();
        assert_eq!(
            cells.attempted(),
            if interpolation == "LINEAR" { 6 } else { 14 }
        );
        assert_eq!(cells.consumed(), expected_prefix);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}

#[test]
fn weights_animation_budget_admits_all_scalar_cells_before_reading() -> TestResult<()> {
    for (interpolation, output, total) in [
        ("LINEAR", WEIGHTS.as_slice(), ANIMATION_CELLS),
        ("CUBICSPLINE", CUBIC.as_slice(), CUBIC_ANIMATION_CELLS),
    ] {
        let (document, binary) = fixture(interpolation, output);
        let mut options = GltfTestOptions::memory();
        options.limits.max_animation_cells = usize::try_from(total - 1)?;
        let (result, diagnostics) = import_assets_result(
            "weights.glb",
            [("weights.glb", glb(&document, &binary))],
            options,
        )?;
        let failure = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert_resource_limit(&failure, "max_animation_cells");
        let cells = failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .unwrap();
        assert_eq!(cells.attempted(), total);
        assert_eq!(cells.consumed(), 0);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}

fn filtered_fixture() -> (Value, Vec<u8>, Vec<u8>, Vec<u8>) {
    let (mut document, mut binary) = fixture("LINEAR", &WEIGHTS);
    let output = binary.split_off(OUTPUT_OFFSET);
    let duration = [0.0_f32, 3.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    document["scenes"] = json!([{"nodes": [0]}, {"nodes": [2]}]);
    document["buffers"] = json!([
        {"uri": "base.bin", "byteLength": binary.len()},
        {"uri": "output.bin", "byteLength": output.len()},
        {"uri": "duration.bin", "byteLength": duration.len()},
        {"uri": "unavailable.bin", "byteLength": 16}
    ]);
    document["bufferViews"][2] = json!({"buffer": 1, "byteLength": 16});
    document["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 2, "byteLength": 8}),
        json!({"buffer": 3, "byteLength": 16}),
    ]);
    document["accessors"].as_array_mut().unwrap().extend([
        json!({"bufferView": 3, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [3]}),
        json!({"bufferView": 4, "componentType": 5126, "count": 4, "type": "SCALAR"})
    ]);
    document["animations"][0]["samplers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"input": 3, "output": 4}));
    document["animations"][0]["channels"] = json!([
        {"sampler": 1, "target": {"node": 2, "path": "weights"}},
        {"sampler": 0, "target": {"node": 0, "path": "weights"}}
    ]);
    (document, binary, output, duration)
}

#[test]
fn selected_weights_keep_external_outputs_and_omitted_input_duration_closure() -> TestResult<()> {
    let (document, base, output, duration) = filtered_fixture();
    let mut options = sidecar_options();
    options.loss_policy = LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss);
    let (report, diagnostics) = import_assets(
        "model/weights.gltf",
        [
            ("model/weights.gltf", serde_json::to_vec(&document)?),
            ("model/base.bin", base),
            ("model/output.bin", output),
            ("model/duration.bin", duration),
        ],
        options,
    )?;
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code().as_str(),
        "gltf.animation_channel_outside_scene_omitted"
    );
    assert_eq!(
        diagnostics[0].object_path(),
        Some("/animations/0/channels/0")
    );
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(
        report.loss_evidence()[0].disposition(),
        ImportLossDisposition::Applied
    );
    assert_cells(&report, 8);
    let scene = report.scene();
    let clip = scene.animation_clips().next().unwrap();
    assert_eq!(clip.duration(), 3.0);
    assert_eq!(clip.channel_count(), 1);
    let channel = clip.channel(0).unwrap();
    assert_eq!(channel.key_times(), &[0.0, 1.0]);
    assert_eq!(channel.values(), &[-0.25, 0.5, 1.25, -2.0]);
    assert_eq!(
        channel.target(),
        AnimationTarget::MeshInstanceMorphWeights(scene.mesh_instances().next().unwrap().id())
    );
    let record = report.source_scope_record().unwrap();
    assert_eq!(
        record
            .counts(SourceScopeEntityKind::AnimationChannel)
            .selected(),
        1
    );
    assert_eq!(
        record
            .counts(SourceScopeEntityKind::AnimationChannel)
            .omitted(),
        1
    );
    assert_eq!(record.counts(SourceScopeEntityKind::Accessor).selected(), 4);
    assert_eq!(record.counts(SourceScopeEntityKind::Accessor).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Buffer).selected(), 3);
    assert_eq!(record.counts(SourceScopeEntityKind::Buffer).omitted(), 1);
    assert_eq!(
        report
            .successful_input_manifest()
            .counts()
            .node_class_count(),
        4
    );
    Ok(())
}

#[test]
fn entirely_omitted_weights_do_not_load_output_buffers_or_publish_clips() -> TestResult<()> {
    let (mut document, base, _, _) = filtered_fixture();
    document["animations"][0]["channels"] = json!([
        {"sampler": 0, "target": {"node": 2, "path": "weights"}}
    ]);
    let (report, diagnostics) = import_assets(
        "model/weights.gltf",
        [
            ("model/weights.gltf", serde_json::to_vec(&document)?),
            ("model/base.bin", base),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.scene().animation_clip_count(), 0);
    assert_cells(&report, 0);
    let record = report.source_scope_record().unwrap();
    assert_eq!(
        record.counts(SourceScopeEntityKind::Animation).selected(),
        0
    );
    assert_eq!(
        record
            .counts(SourceScopeEntityKind::AnimationChannel)
            .omitted(),
        1
    );
    assert_eq!(record.counts(SourceScopeEntityKind::Accessor).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Buffer).selected(), 1);
    Ok(())
}

fn controlled_weights_result(
    bytes: &[u8],
    cancel_on: Option<usize>,
) -> TestResult<(ImportResult<ImportReport>, Vec<ImportDiagnostic>, usize)> {
    let (control, polls) = controlled_import(cancel_on);
    let (result, diagnostics) = import_assets_result_with_control(
        "weights.glb",
        [("weights.glb", bytes.to_vec())],
        GltfTestOptions::memory(),
        control,
    )?;
    Ok((result, diagnostics, polls.load(Ordering::SeqCst)))
}

fn assert_signal_cancelled(failure: &ImportFailure) {
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(abort.cause(),
            ScopeAbortCause::Control(ControlCause::Cancelled { origin: CancellationOrigin::Signal }))
    ));
}

#[test]
fn weight_materialization_cancellation_keeps_prefix_and_releases_all_storage() -> TestResult<()> {
    // More than one poll interval ensures cancellation can occur inside a weight vector stream.
    let output = [0.25_f32, -0.5].repeat(4_097);
    let (mut document, mut binary) = fixture("LINEAR", &output);
    binary.truncate(36);
    for key in 0..4_097_u32 {
        binary.extend_from_slice(&(key as f32).to_le_bytes());
    }
    let output_offset = binary.len();
    for value in output {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    document["buffers"][0]["byteLength"] = json!(binary.len());
    document["bufferViews"][1]["byteLength"] = json!(4_097 * 4);
    document["bufferViews"][2]["byteOffset"] = json!(output_offset);
    document["accessors"][1]["count"] = json!(4_097);
    document["accessors"][1]["max"] = json!([4_096]);
    let bytes = glb(&document, &binary);
    let (calibration, diagnostics, total_polls) = controlled_weights_result(&bytes, None)?;
    assert!(diagnostics.is_empty());
    let report = calibration?;
    assert_cells(&report, 12_291);
    drop(report);
    let mut cancelled = None;
    for cancel_on in 1..=total_polls {
        let (result, diagnostics, polls) = controlled_weights_result(&bytes, Some(cancel_on))?;
        assert!(diagnostics.is_empty());
        let failure = match result {
            Ok(_) => continue,
            Err(error) => error.into_operation_failure()?,
        };
        assert_eq!(polls, cancel_on);
        assert_signal_cancelled(&failure);
        let cells = failure
            .ledger()
            .dimension(ImportDimension::AnimationCells)
            .unwrap();
        if cells.attempted() == 12_291 && cells.consumed() > 4_097 && cells.consumed() < 12_291 {
            cancelled = Some(failure);
            break;
        }
    }
    let failure = cancelled.expect("a calibrated poll cancels within scalar weight output");
    assert!(failure.source_scope_record().is_some());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    Ok(())
}
