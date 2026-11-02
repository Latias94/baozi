mod common;

use std::{path::PathBuf, sync::Arc};

use baozi::{AssetPath, CameraProjection, Importer, LightKindCategory, LocalAxes, MemoryAssetIo};
use baozi_asset_consumer::{
    ApplicationAsset, EvaluationLimits, EvaluationRequest, FramePolicy, Result, SampleTime,
    evaluate,
};
use baozi_render_wgpu::{
    Control,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan},
    renderer::{RenderSettings, RenderedFrame},
    spatial::{CameraInput, IDENTITY, LightFrame},
};

fn detached(name: &str) -> Result<ApplicationAsset> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../assets/fixtures/upstream/Models")
        .join(name)
        .join("glTF");
    let document = std::fs::read(root.join(format!("{name}.gltf")))?;
    let source: serde_json::Value = serde_json::from_slice(&document)?;
    let io = MemoryAssetIo::new();
    io.try_insert(AssetPath::new("model.gltf")?, Arc::from(document))?;
    for buffer in source["buffers"]
        .as_array()
        .ok_or("missing source buffers")?
    {
        let uri = buffer["uri"].as_str().ok_or("missing source buffer URI")?;
        io.try_insert(
            AssetPath::new(uri)?,
            Arc::from(std::fs::read(root.join(uri))?),
        )?;
    }
    let importer = Importer::new()?;
    let report = importer.read_asset(&io, AssetPath::new("model.gltf")?)?;
    assert!(report.loss_evidence().is_empty());
    let (scene, evidence) = report.into_parts();
    let asset = ApplicationAsset::from_scene(&scene)?;
    drop((scene, evidence, importer, io, source));
    Ok(asset)
}

fn near(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 1.0e-10,
            "{actual} != {expected}"
        );
    }
}

fn common_covered_change(first: &RenderedFrame, second: &RenderedFrame) -> (usize, f32) {
    let mut covered = 0;
    let mut difference = 0.0_f32;
    for (left, right) in first.linear_rgba.iter().zip(&second.linear_rgba) {
        // Both backgrounds are transparent. Require two actual opaque surface samples;
        // a moving silhouette alone cannot satisfy this lighting-change observation.
        if left[3] > 0.99 && right[3] > 0.99 {
            covered += 1;
            for lane in 0..3 {
                difference = difference.max((left[lane] - right[lane]).abs());
            }
        }
    }
    (covered, difference)
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter is a failure"]
fn unchanged_animated_samples_have_source_anchors_and_lit_readback_after_owner_drop() -> Result<()>
{
    let renderer = common::renderer();
    let light = LightFrame {
        position: [0.0; 3],
        outgoing: [0.0, 0.0, -1.0],
        kind: LightKindCategory::Directional,
        tint: [1.0; 3],
        intensity: 2.0,
        range: None,
        cos_inner: 1.0,
        cos_outer: 1.0,
        meters_per_unit: 1.0,
    };
    // These vectors were independently decoded from the pinned original little-endian
    // accessors, sampled in source time and evaluated with the canonical skin equation.
    // They are also checked by the CPU-only evaluator suite, independently of this renderer.
    for (name, corner, camera_xy, magnification, samples) in [
        (
            "RiggedSimple",
            427,
            [1.5, 2.0],
            4.0,
            [
                (
                    1.0833330154418945,
                    [2.348135485919336, 3.947126532032868, 0.4158198015543262],
                    [
                        0.5446350752436382,
                        0.8386731394377408,
                        2.3421896532356733e-8,
                    ],
                ),
                (
                    1.0625,
                    [2.3946026263363884, 3.919394367212576, 0.4158198032957121],
                    [
                        0.5545216156306101,
                        0.8321693203900379,
                        2.3783078037406967e-8,
                    ],
                ),
            ],
        ),
        (
            "AnimatedMorphCube",
            18,
            [0.0, 0.0],
            1.5,
            [
                (
                    2.09999942779541,
                    [-1.0000002817201947, 0.9999998198406057, -0.9197691214789313],
                    [0.0, 0.20757639232310912, 0.9782188105685368],
                ),
                (
                    2.116666078567505,
                    [-1.0000002817201947, 0.999999819661411, -0.9211028238497173],
                    [0.0, 0.21845065776748473, 0.9758479953973126],
                ),
            ],
        ),
    ] {
        let mut frames = Vec::new();
        for (sample, (time, position, normal)) in samples.into_iter().enumerate() {
            let uploaded = {
                let asset = detached(name)?;
                let source_vertex = usize::try_from(asset.draws[0].indices[corner])?;
                let evaluation = evaluate(
                    &asset,
                    Some(0),
                    SampleTime::Declared(time),
                    EvaluationRequest {
                        frames: FramePolicy::FrozenWeightNormalsOnly,
                        limits: EvaluationLimits::default(),
                        cancellation: None,
                    },
                )?;
                near(evaluation.draws[0].world_positions[source_vertex], position);
                near(
                    evaluation.draws[0].world_normals.as_ref().unwrap()[source_vertex],
                    normal,
                );
                assert!(evaluation.draws[0].world_tangents.is_none());
                let normals = evaluated_frames(&evaluation)?;
                let plan = DevicePlan::prepare(
                    &asset,
                    &evaluation,
                    &normals,
                    DeviceLimits::default(),
                    Control::default(),
                )?;
                let mut world = IDENTITY;
                world[3] = [camera_xy[0], camera_xy[1], 10.0, 1.0];
                renderer.upload(
                    &plan,
                    CameraInput::Explicit {
                        projection: CameraProjection::orthographic(
                            LocalAxes::STANDARD,
                            magnification,
                            magnification,
                            1.0,
                            20.0,
                        )?,
                        world,
                    },
                    RenderSettings {
                        width: 128,
                        height: 128,
                        clear: [0.0; 4],
                        ..RenderSettings::default()
                    },
                    &[light],
                    Control::default(),
                )?
            }; // Asset, evaluation, source buffers and all import authority have been dropped.
            let mut frame = renderer.render(&uploaded, Control::default())?;
            assert!(frame.diagnostics.iter().all(|&code| code == 0));
            assert!(
                frame
                    .linear_rgba
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
            );
            frame.report["fixture"] = serde_json::json!(name);
            frame.report["source_sample_assets_revision"] =
                serde_json::json!("8f492390ddc7a2ae221a1369ef560589cf8c70fb");
            frame.report["declared_time"] = serde_json::json!(time);
            frame.report["source_anchor_corner"] = serde_json::json!(corner);
            frame.report["expected_world_position"] = serde_json::json!(position);
            frame.report["expected_world_normal"] = serde_json::json!(normal);
            let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../target/review/renderer/upstream");
            std::fs::create_dir_all(&evidence)?;
            frame.save(&evidence.join(format!("{name}-{sample}")))?;
            frames.push(frame);
        }
        let (covered, difference) = common_covered_change(&frames[0], &frames[1]);
        assert!(covered > 32, "{name}: only {covered} common opaque samples");
        assert!(
            difference > 0.0001,
            "{name}: no interior lit change: {difference}"
        );
    }
    Ok(())
}
