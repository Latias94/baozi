mod common;

use common::{GltfTestOptions, TestResult, append_glb_chunk, glb_with_json, import_assets};

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
