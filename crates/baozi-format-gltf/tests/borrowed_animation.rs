mod common;

use baozi_core::{
    AnimationExtrapolation, AnimationInterpolation, AnimationTarget, AuthoredTransform, Quat, Vec3,
};
use common::{GltfTestOptions, TestResult, append_glb_chunk, glb_with_json, import_assets};
use serde_json::json;

fn named_animation_glb() -> Vec<u8> {
    let json = br#"{
      "asset": {"version": "2.0"},
      "scene": 0,
      "scenes": [{"nodes": [0]}],
      "nodes": [{"name": "selected"}, {"name": "outside"}],
      "buffers": [{"byteLength": 32}],
      "bufferViews": [
        {"buffer": 0, "byteOffset": 0, "byteLength": 8},
        {"buffer": 0, "byteOffset": 8, "byteLength": 24}
      ],
      "accessors": [
        {"bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.25], "max": [1.5]},
        {"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3"}
      ],
      "animations": [
        {"name": "omitted", "samplers": [{"input": 0, "output": 1}], "channels": [{"sampler": 0, "target": {"node": 1, "path": "translation"}}]},
        {"name": "walk", "samplers": [{"input": 0, "output": 1, "interpolation": "LINEAR"}], "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]},
        {"name": "", "samplers": [{"input": 0, "output": 1, "interpolation": "STEP"}], "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]},
        {"samplers": [{"input": 0, "output": 1}], "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]}
      ]
    }"#;
    let mut binary = Vec::new();
    for scalar in [0.25_f32, 1.5, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0] {
        binary.extend_from_slice(&scalar.to_le_bytes());
    }
    append_glb_chunk(glb_with_json(json), b"BIN\0", &binary)
}

#[test]
fn retained_animation_names_and_borrowed_keys_follow_selected_clip_order() -> TestResult<()> {
    let (report, _) = import_assets(
        "named.glb",
        [("named.glb", named_animation_glb())],
        GltfTestOptions::memory(),
    )?;
    let scene = report.scene();
    assert_eq!(scene.animation_clip_count(), 3);
    let mut clips = scene.animation_clips();
    let named = clips.next().unwrap();
    let empty_name = clips.next().unwrap();
    let absent_name = clips.next().unwrap();
    assert!(clips.next().is_none());
    assert_eq!(named.name(), Some("walk"));
    assert_eq!(empty_name.name(), Some(""));
    assert_eq!(absent_name.name(), None);
    assert_ne!(named.id(), empty_name.id());
    assert_ne!(empty_name.id(), absent_name.id());
    assert_ne!(named.id(), absent_name.id());
    assert_eq!(named.duration(), 1.5);
    assert_eq!(named.channel_count(), 1);
    let channel = named.channel(0).unwrap();
    assert_eq!(channel.key_count(), 2);
    assert_eq!(channel.value_width(), 3);
    assert_eq!(channel.key_times(), &[0.25_f64, 1.5]);
    assert_eq!(channel.values(), &[0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    assert_eq!(channel.in_tangents(), None);
    assert_eq!(channel.out_tangents(), None);
    assert_eq!(
        scene.animation_clip(named.id()).unwrap().name(),
        Some("walk")
    );

    // The slices remain tied to the scene after temporary wrappers and iterators disappear.
    let values = scene
        .animation_clips()
        .next_back()
        .unwrap()
        .channel(0)
        .unwrap()
        .values();
    assert_eq!(values, &[0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    assert_eq!(scene.animation_clips().len(), 3);
    Ok(())
}

#[test]
fn trs_endpoint_tags_clamp_instead_of_reverting_to_base_pose() -> TestResult<()> {
    let expected_base = AuthoredTransform::try_trs(
        Vec3::new(9.0, 8.0, 7.0),
        Quat::from_xyzw(1.0, 0.0, 0.0, 0.0),
        Vec3::new(4.0, 5.0, 6.0),
    )?;
    let targets: [(&str, &[f32], &[f32]); 3] = [
        (
            "translation",
            &[9.0, 8.0, 7.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        ),
        (
            "rotation",
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0],
        ),
        ("scale", &[4.0, 5.0, 6.0], &[2.0, 3.0, 4.0, 6.0, 7.0, 8.0]),
    ];
    for (source_interpolation, interpolation) in [
        ("STEP", AnimationInterpolation::Step),
        ("LINEAR", AnimationInterpolation::Linear),
        ("CUBICSPLINE", AnimationInterpolation::CubicSpline),
    ] {
        for (path, base_values, key_values) in targets {
            let width = base_values.len();
            let cubic = interpolation == AnimationInterpolation::CubicSpline;
            let mut binary = Vec::new();
            for time in [0.25_f32, 1.5] {
                binary.extend_from_slice(&time.to_le_bytes());
            }
            for key in key_values.chunks_exact(width) {
                if cubic {
                    for _ in 0..width {
                        binary.extend_from_slice(&0.25_f32.to_le_bytes());
                    }
                }
                for value in key {
                    binary.extend_from_slice(&value.to_le_bytes());
                }
                if cubic {
                    for _ in 0..width {
                        binary.extend_from_slice(&0.5_f32.to_le_bytes());
                    }
                }
            }
            let document = json!({
                "asset": {"version": "2.0"},
                "scene": 0,
                "scenes": [{"nodes": [0]}],
                "nodes": [{
                    "translation": [9, 8, 7],
                    "rotation": [1, 0, 0, 0],
                    "scale": [4, 5, 6]
                }],
                "buffers": [{"byteLength": binary.len()}],
                "bufferViews": [
                    {"buffer": 0, "byteOffset": 0, "byteLength": 8},
                    {"buffer": 0, "byteOffset": 8, "byteLength": binary.len() - 8}
                ],
                "accessors": [
                    {"bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR",
                     "min": [0.25], "max": [1.5]},
                    {"bufferView": 1, "componentType": 5126, "count": if cubic { 6 } else { 2 },
                     "type": if width == 4 { "VEC4" } else { "VEC3" }}
                ],
                "animations": [{
                    "samplers": [{"input": 0, "output": 1, "interpolation": source_interpolation}],
                    "channels": [{"sampler": 0, "target": {"node": 0, "path": path}}]
                }]
            });
            let bytes = append_glb_chunk(
                glb_with_json(&serde_json::to_vec(&document)?),
                b"BIN\0",
                &binary,
            );
            let (report, diagnostics) = import_assets(
                "endpoints.glb",
                [("endpoints.glb", bytes)],
                GltfTestOptions::memory(),
            )?;
            assert!(diagnostics.is_empty());
            let scene = report.scene();
            let node = scene.nodes().next().unwrap();
            assert_eq!(node.authored_transform(), expected_base);
            let target = match path {
                "translation" => AnimationTarget::NodeTranslation(node.id()),
                "rotation" => AnimationTarget::NodeRotation(node.id()),
                "scale" => AnimationTarget::NodeScale(node.id()),
                _ => unreachable!(),
            };
            let clip = scene.animation_clips().next().unwrap();
            assert_eq!(clip.channel_count(), 1);
            let channel = clip.channel(0).unwrap();
            assert_eq!(channel.target(), target);
            assert_eq!(channel.interpolation(), interpolation);
            assert_eq!(channel.key_times(), &[0.25, 1.5]);
            assert_eq!(channel.value_width(), width);
            assert_eq!(channel.values(), key_values);
            assert_ne!(&channel.values()[..width], base_values);
            assert_ne!(&channel.values()[width..], base_values);
            assert_eq!(
                (channel.pre_extrapolation(), channel.post_extrapolation()),
                (
                    AnimationExtrapolation::Constant,
                    AnimationExtrapolation::Constant,
                ),
                "{source_interpolation} {path} endpoints"
            );
        }
    }
    Ok(())
}
