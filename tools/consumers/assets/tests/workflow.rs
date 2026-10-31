use std::{path::PathBuf, sync::Arc};

use baozi::{
    AnimationInterpolation, AssetPath, AttributeSemanticRef, FormatFailureDetailValue,
    FrontFaceWinding, ImportCause, Importer, MemoryAssetIo, MorphDeltaSemantic, TangentBasisRef,
    TangentEncodingRef, TargetSpace, UvOrigin,
    postprocess::{PostProcessControl, SpaceConversionOptions, convert_scene_space},
};
use baozi_asset_consumer::{ChannelTarget, Options, Result, process, process_file};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn model(name: &str) -> String {
    format!("upstream/Models/{name}/glTF/{name}.gltf")
}

#[test]
fn upstream_models_match_source_counts_and_converted_mesh_bounds() -> Result<()> {
    let expected: Value = serde_json::from_str(include_str!("../fixtures/expectations.json"))?;
    for fixture in expected["fixtures"]
        .as_array()
        .ok_or("missing fixture list")?
    {
        if fixture["expected_outcome"] != "success" {
            continue;
        }
        let name = fixture["id"].as_str().ok_or("missing model name")?;
        let output = process_file(
            &fixtures(),
            fixture["path"].as_str().unwrap(),
            Options::default(),
        )
        .map_err(|e| format!("{name}: {e}"))?;
        let asset = &output.asset;
        let counts = &fixture["counts"];
        for (key, actual) in [
            ("nodes", asset.nodes.len()),
            ("meshes", asset.mesh_count),
            ("primitives", asset.draws.len()),
            ("mesh_instances", asset.instances.len()),
            ("skins", asset.skins.len()),
            ("animations", asset.animations.len()),
            ("materials", asset.materials.len()),
            ("images", asset.images.len()),
        ] {
            assert_eq!(
                actual as u64,
                counts[key].as_u64().unwrap(),
                "{name}: {key}"
            );
        }
        assert!(output.import_evidence.loss_evidence().is_empty(), "{name}");
        assert_eq!(output.import_evidence.ledger().current_live_bytes(), 0);
        assert_eq!(output.import_evidence.ledger().active_reservations(), 0);
        assert_eq!(
            output.metrics.opened_assets,
            1 + fixture["external_resources"].as_array().unwrap().len() as u64
        );
        for (draw, expected) in asset
            .draws
            .iter()
            .zip(fixture["primitives"].as_array().unwrap())
        {
            assert_eq!(
                draw.vertices.len() as u64,
                expected["index_count"].as_u64().unwrap(),
                "{name}"
            );
            assert_eq!(
                draw.indices,
                (0..draw.vertices.len() as u32).collect::<Vec<_>>()
            );
            let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
            for v in &draw.vertices {
                assert!(v.normal.is_some(), "{name}: normals retained or generated");
                let [low, high] = &mut bounds;
                for ((low, high), value) in low.iter_mut().zip(high).zip(v.position) {
                    *low = low.min(value);
                    *high = high.max(value);
                }
            }
            for lane in 0..3 {
                let (low, high) = if lane == 2 {
                    (
                        -expected["position_max"][lane].as_f64().unwrap() * 100.0,
                        -expected["position_min"][lane].as_f64().unwrap() * 100.0,
                    )
                } else {
                    (
                        expected["position_min"][lane].as_f64().unwrap() * 100.0,
                        expected["position_max"][lane].as_f64().unwrap() * 100.0,
                    )
                };
                assert!(
                    (bounds[0][lane] - low).abs() < 0.0001,
                    "{name}: lower {lane}"
                );
                assert!(
                    (bounds[1][lane] - high).abs() < 0.0001,
                    "{name}: upper {lane}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn texture_bytes_material_binding_uvs_and_reflected_winding_survive_scene_drop() -> Result<()> {
    let output = process_file(&fixtures(), &model("BoxTextured"), Options::default())?;
    let asset = output.asset;
    assert_eq!(
        asset.images[0],
        std::fs::read(fixtures().join("upstream/Models/BoxTextured/glTF/CesiumLogoFlat.png"))?
    );
    let binding = &asset.materials[0].bindings[0];
    assert_eq!(binding.image, 0);
    assert_eq!(binding.uv_set, 0);
    assert!(asset.draws[0].vertices.iter().all(|v| v.uv.is_some()));
    let standard = process_file(
        &fixtures(),
        &model("BoxTextured"),
        Options {
            target: TargetSpace::STANDARD,
            ..Options::default()
        },
    )?;
    for (before, after) in standard.asset.draws[0]
        .vertices
        .chunks_exact(3)
        .zip(asset.draws[0].vertices.chunks_exact(3))
    {
        for (lane, source) in [0, 2, 1].into_iter().enumerate() {
            assert_eq!(after[lane].uv, before[source].uv);
            let p = before[source].position;
            assert_eq!(
                after[lane].position,
                [-p[0] * 100.0, p[1] * 100.0, p[2] * 100.0]
            );
        }
    }
    Ok(())
}

#[test]
fn shared_mesh_instances_keep_distinct_authored_transforms() -> Result<()> {
    let output = process_file(&fixtures(), &model("SimpleMeshes"), Options::default())?;
    assert_eq!(
        output.asset.instances[0].mesh,
        output.asset.instances[1].mesh
    );
    let node = &output.asset.nodes[output.asset.instances[1].node];
    let trs = node
        .transform
        .authored_trs()
        .ok_or("source TRS changed form")?;
    assert_eq!(trs.translation().x, 100.0);
    assert_eq!(trs.translation().y, 0.0);
    Ok(())
}

#[test]
fn skeletal_animation_keeps_all_influences_joint_bindings_and_keys() -> Result<()> {
    let expected: Value = serde_json::from_str(include_str!("../fixtures/expectations.json"))?;
    let expected = expected["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "RiggedSimple")
        .unwrap();
    let output = process_file(&fixtures(), &model("RiggedSimple"), Options::default())?;
    let asset = output.asset;
    let skin = &asset.skins[0];
    assert_eq!(
        skin.joints
            .iter()
            .map(|i| asset.nodes[*i].name.as_deref())
            .collect::<Vec<_>>(),
        [Some("Bone"), Some("Bone.001")]
    );
    assert_eq!(skin.inverse_binds.len(), 2);
    assert_eq!(asset.instances[0].skin, Some(0));
    assert_eq!(
        asset.nodes[asset.instances[0].node].name.as_deref(),
        Some("Cylinder")
    );
    let row = asset.draws[0]
        .influences
        .as_ref()
        .ok_or("missing influences")?;
    assert_eq!(row.offsets.len(), 565);
    assert_eq!(row.joints.len(), row.weights.len());
    for range in row.offsets.windows(2) {
        assert!(row.joints[range[0]..range[1]].iter().all(|j| *j < 2));
        assert!((row.weights[range[0]..range[1]].iter().sum::<f32>() - 1.0).abs() < 0.00001);
    }
    let sample = &expected["skin_influences"][0]["samples"][1];
    let joints: Vec<_> = sample["joints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u32)
        .collect();
    let weights: Vec<_> = sample["weights"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap() as f32)
        .collect();
    assert!(
        row.offsets
            .windows(2)
            .any(|r| row.joints[r[0]..r[1]] == joints && row.weights[r[0]..r[1]] == weights)
    );
    let animation = &expected["animations"][0];
    let times: Vec<_> = animation["input_times"]
        .as_array()
        .unwrap()
        .iter()
        // The source accessor is F32. Widen its value exactly, independently of
        // serde_json's decimal-to-F64 parsing precision.
        .map(|v| f64::from(v.as_f64().unwrap() as f32))
        .collect();
    assert_eq!(asset.animations[0].channels.len(), 3);
    assert_eq!(asset.animations[0].duration, *times.last().unwrap());
    for (channel, source) in asset.animations[0]
        .channels
        .iter()
        .zip(animation["channels"].as_array().unwrap())
    {
        assert_eq!(channel.interpolation, AnimationInterpolation::Linear);
        assert_eq!(channel.times, times);
        assert_eq!(channel.values.len(), times.len() * channel.width);
        let node = match channel.target {
            ChannelTarget::Translation(node)
            | ChannelTarget::Rotation(node)
            | ChannelTarget::Scale(node) => node,
            target => panic!("unexpected target: {target:?}"),
        };
        assert_eq!(asset.nodes[node].name.as_deref(), Some("Bone.001"));
        let path = source["target_path"].as_str().unwrap();
        assert!(matches!(
            (path, channel.target),
            ("translation", ChannelTarget::Translation(_))
                | ("rotation", ChannelTarget::Rotation(_))
                | ("scale", ChannelTarget::Scale(_))
        ));
        for sample in source["samples"].as_array().unwrap() {
            let key = sample["key"].as_u64().unwrap() as usize;
            for (lane, value) in sample["value"].as_array().unwrap().iter().enumerate() {
                let value = value.as_f64().unwrap() as f32;
                let expected = match (path, lane) {
                    ("translation", 2) => (f64::from(value) * -100.0) as f32,
                    ("translation", _) => (f64::from(value) * 100.0) as f32,
                    ("rotation", 0 | 1) => -value,
                    _ => value,
                };
                assert_eq!(
                    channel.values[key * channel.width + lane].to_bits(),
                    expected.to_bits()
                );
            }
        }
    }
    for (matrix, source) in skin.inverse_binds.iter().zip(
        expected["skins"][0]["inverse_bind_matrices"]
            .as_array()
            .unwrap(),
    ) {
        for column in 0..4 {
            for row in 0..4 {
                let source = source["values_column_major"][column * 4 + row]
                    .as_f64()
                    .unwrap() as f32;
                let scale = if column == 3 && row < 3 { 100.0 } else { 1.0 };
                let sign = if (column == 2) != (row == 2) {
                    -1.0
                } else {
                    1.0
                };
                let expected = (f64::from(source) * scale * sign) as f32;
                assert_eq!(matrix.cols[column][row], expected);
            }
        }
    }
    Ok(())
}

#[test]
fn morph_streams_and_weight_animation_remain_app_owned() -> Result<()> {
    let output = process_file(&fixtures(), &model("AnimatedMorphCube"), Options::default())?;
    let asset = output.asset;
    let draw = &asset.draws[0];
    assert!(draw.vertices.iter().all(|v| v.tangent.is_some()));
    assert_eq!(draw.morphs.len(), 6);
    assert!(draw.morphs.iter().all(|m| m.values.len() == 36));
    assert_eq!(asset.instances[0].morph_weights, [0.0, 0.0]);
    let first = draw
        .morphs
        .iter()
        .find(|m| m.target == 0 && m.semantic == MorphDeltaSemantic::Position)
        .unwrap();
    let max_y = first.values.iter().map(|v| v[1]).fold(0.0, f32::max);
    assert!((max_y - 1.893253).abs() < 0.00001);
    let clip = &asset.animations[0];
    assert_eq!(clip.name.as_deref(), Some("Square"));
    let channel = &clip.channels[0];
    assert_eq!(channel.target, ChannelTarget::MorphWeights(0));
    assert_eq!(channel.width, 2);
    assert_eq!(channel.times.len(), 127);
    assert_eq!(channel.values.len(), 254);
    // Independently decoded from source accessor 11: preserve the authored tiny
    // negative final weight rather than clamping or normalizing it.
    assert_eq!(&channel.values[..2], &[0.0, 0.0]);
    assert_eq!(
        &channel.values[126..128],
        &[0.7233077883720398_f64, 0.2766921818256378].map(|v| v as f32)
    );
    assert_eq!(
        &channel.values[252..],
        &[0.0, (-1.5258788721439487e-7_f64) as f32]
    );
    assert!((clip.duration - 4.19999743).abs() < 0.000001);
    Ok(())
}

#[test]
fn unchanged_morph_primitives_keep_source_json_after_edits_and_scene_drop() -> Result<()> {
    let primary = std::fs::read(fixtures().join(model("MorphPrimitivesTest")))?;
    for winding in [
        FrontFaceWinding::CounterClockwise,
        FrontFaceWinding::Clockwise,
    ] {
        let target = Options::default().target;
        let output = process_file(
            &fixtures(),
            &model("MorphPrimitivesTest"),
            Options {
                target: TargetSpace::try_new(
                    target.handedness(),
                    target.up_axis(),
                    target.front_axis(),
                    target.unit_scale_to_meters(),
                    winding,
                )?,
                ..Options::default()
            },
        )?;
        // process_file has destroyed every Scene before returning these owned values.
        let asset = output.asset;
        assert_eq!(asset.vertex_count(), 96);
        assert_eq!(asset.draws.len(), 2);
        assert_eq!(asset.materials.len(), 2);
        assert_ne!(asset.draws[0].material, asset.draws[1].material);
        assert_eq!(asset.instances[0].morph_weights, [0.5]);
        for draw in &asset.draws {
            let delta = draw
                .morphs
                .iter()
                .find(|m| m.target == 0 && m.semantic == MorphDeltaSemantic::Position)
                .unwrap();
            assert_eq!(delta.values.len(), draw.vertices.len());
            assert!((delta.values.iter().map(|v| v[1]).fold(0.0, f32::max) - 20.0).abs() < 0.00001);
        }
        assert_eq!(
            asset.images[0],
            std::fs::read(
                fixtures().join("upstream/Models/MorphPrimitivesTest/glTF/uv_texture.jpg")
            )?
        );
        assert!(output.import_evidence.loss_evidence().is_empty());
        assert_eq!(asset.source_json_metadata.len(), 1);
        let metadata = &asset.source_json_metadata[0];
        assert_eq!(metadata.resource, model("MorphPrimitivesTest"));
        assert_eq!(metadata.format, "org.baozi.gltf");
        assert_eq!(metadata.locator_schema, "org.baozi.gltf:source-object");
        assert_eq!(metadata.locator_schema_version, 1);
        // The upstream resource is 5,610 bytes; its final newline is outside the root value.
        assert_eq!(
            metadata.locator,
            "kind=document;parent=-;index=0;offset=0;length=5609"
        );
        assert_eq!(metadata.key, "org.baozi.gltf:asset-extras");
        assert_eq!(metadata.media_type, "application/json");
        assert_eq!(metadata.payload, primary[98..245]);
        let value: Value = serde_json::from_slice(&metadata.payload)?;
        assert_eq!(value["title"], "multiple_primitives");
        assert_eq!(value["author"], "ft-lab");
        assert_eq!(
            value["license"],
            "CC BY-4.0 (https://creativecommons.org/licenses/by/4.0/)"
        );
    }
    Ok(())
}

#[test]
fn authored_gltf_tangents_reflect_without_claiming_a_construction_algorithm() -> Result<()> {
    let io =
        FileSystemAssetIo::from_capability(NativeDirectoryCapability::open_ambient(fixtures())?);
    let report = Importer::new()?.read_asset(&io, AssetPath::new(model("AnimatedMorphCube"))?)?;
    let (scene, _) = report.into_parts();
    assert_eq!(scene.space().front_axis, baozi::Axis::PositiveZ);
    assert_eq!(scene.space().uv_origin, UvOrigin::UpperLeft);
    assert_eq!(scene.space().tangent_basis, TangentBasisRef::Unknown);
    assert_eq!(
        scene.space().tangent_handedness_encoding,
        TangentEncodingRef::SignInW
    );
    let tangents = |scene: &baozi::ValidatedScene| -> Vec<[f32; 4]> {
        let attribute = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .attributes()
            .find(|a| a.semantic() == AttributeSemanticRef::Tangent)
            .unwrap();
        let baozi::AttributeData::DenseF32 { values, .. } = attribute.data() else {
            panic!("expected dense tangents");
        };
        values
            .chunks_exact(4)
            .map(|v| [v[0], v[1], v[2], v[3]])
            .collect()
    };
    let before = tangents(&scene);
    let (scene, _) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: Options::default().target,
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    for (before, after) in before.iter().zip(tangents(&scene)) {
        assert_eq!(after, [before[0], before[1], -before[2], -before[3]]);
    }
    assert_eq!(scene.space().tangent_basis, TangentBasisRef::Unknown);
    Ok(())
}

#[test]
fn unchanged_upstream_policy_rejections_are_typed() -> Result<()> {
    let expected: Value = serde_json::from_str(include_str!("../fixtures/expectations.json"))?;
    for fixture in expected["fixtures"].as_array().unwrap() {
        if fixture["expected_outcome"] != "rejection" {
            continue;
        }
        let error = process_file(
            &fixtures(),
            fixture["path"].as_str().unwrap(),
            Options::default(),
        )
        .unwrap_err();
        let Some(baozi::Error::Import(failure)) = error.downcast_ref::<baozi::Error>() else {
            panic!("expected typed import rejection: {error:?}");
        };
        let ImportCause::Format(format) = failure.cause() else {
            panic!("expected format rejection: {failure:?}");
        };
        let rejection = &fixture["rejection"];
        assert_eq!(format.code(), rejection["error_code"].as_str().unwrap());
        let detail = |key| {
            format
                .details()
                .iter()
                .find_map(|d| match (d.key(), d.value()) {
                    (name, FormatFailureDetailValue::String(value)) if name == key => {
                        Some(value.as_str())
                    }
                    _ => None,
                })
        };
        if let Some(subject) = rejection["subject"].as_str() {
            assert_eq!(detail("subject"), Some(subject));
        }
        if let Some(reason) = rejection["reason_contains"].as_str() {
            assert!(detail("reason").unwrap().contains(reason));
        }
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}

#[test]
fn consecutive_polygon_edits_reacquire_current_generation_handles() -> Result<()> {
    let io = MemoryAssetIo::new();
    let path = AssetPath::new("two.obj")?;
    io.try_insert(path.clone(), Arc::from(b"o first\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\no second\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\nf 5 6 7 8\n".as_slice()))?;
    let output = process(
        &io,
        path,
        Options {
            source_assumptions: Some(TargetSpace::STANDARD),
            ..Options::default()
        },
    )?;
    assert_eq!(output.asset.draws.len(), 2);
    assert_eq!(output.asset.vertex_count(), 12);
    assert_eq!(output.metrics.changed_operations, 5);
    assert_eq!(
        output.asset.bounds(),
        Some([[-100.0, 0.0, 0.0], [0.0, 100.0, 100.0]])
    );
    Ok(())
}

#[test]
fn missing_normals_follow_the_declared_source_and_target_fronts() -> Result<()> {
    use FrontFaceWinding::{Clockwise, CounterClockwise};
    let with_winding = |space: TargetSpace, winding| {
        TargetSpace::try_new(
            space.handedness(),
            space.up_axis(),
            space.front_axis(),
            space.unit_scale_to_meters(),
            winding,
        )
    };
    for source_winding in [Clockwise, CounterClockwise] {
        for target_winding in [Clockwise, CounterClockwise] {
            let io = MemoryAssetIo::new();
            let path = AssetPath::new("triangle.obj")?;
            let source: &[u8] = match source_winding {
                Clockwise => b"v 0 0 0\nv 0 1 0\nv 1 0 0\nf 1 2 3\n",
                CounterClockwise => b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
                _ => unreachable!(),
            };
            io.try_insert(path.clone(), Arc::from(source))?;
            let output = process(
                &io,
                path,
                Options {
                    source_assumptions: Some(with_winding(TargetSpace::STANDARD, source_winding)?),
                    target: with_winding(Options::default().target, target_winding)?,
                    ..Options::default()
                },
            )?;
            let vertices = &output.asset.draws[0].vertices;
            assert!(vertices.iter().all(|v| v.normal == Some([0.0, 0.0, 1.0])));
            let [a, b, c] = std::array::from_fn::<_, 3, _>(|i| vertices[i].position);
            let cross_z = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            assert_eq!(cross_z > 0.0, target_winding == CounterClockwise);
        }
    }
    Ok(())
}
