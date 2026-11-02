mod common;

use std::sync::Arc;

use baozi::{AssetPath, Importer, LightKindCategory, MemoryAssetIo, MorphDeltaSemantic};
use baozi_asset_consumer::{
    ApplicationAsset, EvaluationLimits, EvaluationRequest, FramePolicy, Result, SampleTime,
    evaluate,
};
use baozi_render_wgpu::{
    Control,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan, SOURCE_NORMAL, SOURCE_TANGENT, WORLD_TANGENT},
    material::BASE,
    renderer::{GpuAsset, RenderSettings, RenderedFrame, Renderer},
    spatial::LightFrame,
};
use serde_json::{Value, json};

// Independently rounded 63/127 and 126/127; the importer must retain nonunit frames.
const SN63: f32 = f32::from_bits(0x3efd_fbf8);
const SN126: f32 = f32::from_bits(0x3f7d_fbf8);
const PHYSICAL: [[f64; 3]; 3] = [[-0.75, -0.75, 0.0], [0.75, -0.75, 0.0], [0.0, 0.75, 0.0]];

struct Witness {
    time: f64,
    shift: [f64; 3],
    normal: [f64; 3],
    tangent: [f64; 4],
    rgba: [f32; 4],
}

// Literal controls, independent of the importer, evaluator and production BRDF helpers.
const SAMPLES: [Witness; 3] = [
    Witness {
        time: 0.0,
        shift: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
        tangent: [1.0, 0.0, 0.0, -1.0],
        rgba: [0.06914521, 0.01884989, 0.16425762, 1.0],
    },
    Witness {
        time: 1.0,
        shift: [0.125, 0.0625, 0.125],
        normal: [0.0, -0.24253562503633297, 0.9701425001453319],
        tangent: [
            0.9718253158075499,
            0.22866478019001177,
            0.05716619504750294,
            -1.0,
        ],
        rgba: [0.06717575, 0.01838212, 0.15944833, 1.0],
    },
    Witness {
        time: 2.0,
        shift: [0.25, 0.125, 0.25],
        normal: [0.0, -0.4472135954999579, 0.8944271909999159],
        tangent: [
            0.9128709291752768,
            0.3651483716701107,
            0.18257418583505536,
            -1.0,
        ],
        rgba: [0.062_181_4, 0.017_195_9, 0.14725253, 1.0],
    },
];

fn append_view(binary: &mut Vec<u8>, views: &mut Vec<Value>, bytes: &[u8], stride: usize) -> usize {
    while !binary.len().is_multiple_of(4) {
        binary.push(0xa5);
    }
    let mut view = json!({"buffer": 0, "byteOffset": binary.len(), "byteLength": bytes.len()});
    if stride != 0 {
        view["byteStride"] = json!(stride);
    }
    let index = views.len();
    views.push(view);
    binary.extend_from_slice(bytes);
    index
}

fn float_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn detached(quantized: bool, skinned: bool) -> Result<ApplicationAsset> {
    let mut binary = Vec::new();
    let mut views = Vec::new();
    let mut vertices = Vec::new();
    if quantized {
        for position in [[10_i16, -6, 4], [34, -6, 4], [22, 18, 4]] {
            vertices.extend(position.into_iter().flat_map(i16::to_le_bytes));
            // Interleaved I16 position, padded I8 normal, I8 tangent and signed I16 UV1.
            vertices.extend_from_slice(&[0xa5, 0xa5, 0, 0, 126, 0xa5, 126, 0, 0, 128]);
            vertices.extend_from_slice(&(-3_i16).to_le_bytes());
            vertices.extend_from_slice(&5_i16.to_le_bytes());
        }
    } else {
        for position in [[-0.75, -0.75, 0.0], [0.75, -0.75, 0.0], [0.0, 0.75, 0.0]] {
            vertices.extend(float_bytes(&[
                position[0],
                position[1],
                position[2],
                0.0,
                0.0,
                1.0,
                1.0,
                0.0,
                0.0,
                -1.0,
                0.75,
                0.25,
            ]));
        }
    }
    append_view(
        &mut binary,
        &mut views,
        &vertices,
        if quantized { 20 } else { 48 },
    );
    let mut accessors = vec![
        json!({"bufferView": 0, "componentType": if quantized {5122} else {5126},
            "count": 3, "type": "VEC3",
            "min": if quantized {json!([10,-6,4])} else {json!([-0.75,-0.75,0])},
            "max": if quantized {json!([34,18,4])} else {json!([0.75,0.75,0])}}),
        json!({"bufferView": 0, "byteOffset": if quantized {8} else {12},
            "componentType": if quantized {5120} else {5126}, "normalized": quantized,
            "count": 3, "type": "VEC3"}),
        json!({"bufferView": 0, "byteOffset": if quantized {12} else {24},
            "componentType": if quantized {5120} else {5126}, "normalized": quantized,
            "count": 3, "type": "VEC4"}),
        json!({"bufferView": 0, "byteOffset": if quantized {16} else {40},
            "componentType": if quantized {5122} else {5126}, "count": 3, "type": "VEC2"}),
    ];
    let mut document = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {
            "POSITION": 0, "NORMAL": 1, "TANGENT": 2, "TEXCOORD_1": 3}, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1,1,1,1],
            "metallicFactor": 0, "roughnessFactor": 1,
            "baseColorTexture": {"index": 0, "texCoord": 1}}}],
        "textures": [{"source": 0, "sampler": 0}], "images": [{"uri": "quadrants.png"}],
        "samplers": [{"magFilter": 9728, "minFilter": 9728, "wrapS": 10497, "wrapT": 10497}]
    });
    if quantized {
        document["extensionsUsed"] = json!(["KHR_mesh_quantization", "KHR_texture_transform"]);
        document["extensionsRequired"] = document["extensionsUsed"].clone();
        document["nodes"][0]["translation"] = json!([-1.375, -0.375, -0.25]);
        document["nodes"][0]["scale"] = json!([0.0625, 0.0625, 0.0625]);
        document["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
            json!({"KHR_texture_transform": {"offset": [1.125,-0.375], "scale": [0.125,0.125]}});
    }
    if skinned {
        // Bound meshes use D in the inverse bind, with a distinct nonidentity mesh M.
        document["scenes"][0]["nodes"] = json!([0, 1]);
        document["nodes"] = json!([
            {"mesh": 0, "skin": 0, "translation": [0.25,-0.125,0.5], "scale": [2,1,0.5]},
            {"name": "animated-joint"}
        ]);
        let mut influences = Vec::new();
        for _ in 0..3 {
            influences.extend_from_slice(&[0; 4]);
            influences.extend(float_bytes(&[1.0, 0.0, 0.0, 0.0]));
        }
        let view = append_view(&mut binary, &mut views, &influences, 20);
        accessors.extend([
            json!({"bufferView": view, "componentType": 5121, "count": 3, "type": "VEC4"}),
            json!({"bufferView": view, "byteOffset": 4, "componentType": 5126, "count": 3, "type": "VEC4"}),
        ]);
        document["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(4);
        document["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(5);
        let bind = if quantized {
            [
                0.0625, 0.0, 0.0, 0.0, 0.0, 0.0625, 0.0, 0.0, 0.0, 0.0, 0.0625, 0.0, -1.375,
                -0.375, -0.25, 1.0,
            ]
        } else {
            [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]
        };
        let view = append_view(&mut binary, &mut views, &float_bytes(&bind), 0);
        accessors
            .push(json!({"bufferView": view, "componentType": 5126, "count": 1, "type": "MAT4"}));
        document["skins"] = json!([{"joints": [1], "inverseBindMatrices": 6}]);
        let mut deltas = Vec::new();
        for _ in 0..3 {
            if quantized {
                deltas.extend_from_slice(&[
                    2, 0, 254, 255, 4, 0, 0xa5, 0xa5, 0, 193, 0, 0xa5, 0, 63, 0, 0xa5,
                ]);
            } else {
                deltas.extend(float_bytes(&[
                    0.125, -0.125, 0.25, 0.0, -0.5, 0.0, 0.0, 0.5, 0.0,
                ]));
            }
        }
        let view = append_view(
            &mut binary,
            &mut views,
            &deltas,
            if quantized { 16 } else { 36 },
        );
        for (offset, component, normalized) in [
            (0, if quantized { 5122 } else { 5126 }, false),
            (
                if quantized { 8 } else { 12 },
                if quantized { 5120 } else { 5126 },
                quantized,
            ),
            (
                if quantized { 12 } else { 24 },
                if quantized { 5120 } else { 5126 },
                quantized,
            ),
        ] {
            let mut accessor = json!({"bufferView": view, "byteOffset": offset,
                "componentType": component, "normalized": normalized, "count": 3, "type": "VEC3"});
            if offset == 0 {
                let bounds = if quantized {
                    json!([2, -2, 4])
                } else {
                    json!([0.125, -0.125, 0.25])
                };
                accessor["min"] = bounds.clone();
                accessor["max"] = bounds;
            }
            accessors.push(accessor);
        }
        document["meshes"][0]["weights"] = json!([0.0]);
        document["meshes"][0]["primitives"][0]["targets"] =
            json!([{"POSITION": 7, "NORMAL": 8, "TANGENT": 9}]);
        for (values, count, shape) in [
            (vec![0.0, 2.0], 2, "SCALAR"),
            (vec![0.0, 1.0], 2, "SCALAR"),
            (vec![0.0, 0.0, 0.0, 0.125, 0.25, 0.0], 2, "VEC3"),
        ] {
            let view = append_view(&mut binary, &mut views, &float_bytes(&values), 0);
            accessors.push(
                json!({"bufferView": view, "componentType": 5126, "count": count, "type": shape}),
            );
        }
        accessors[10]["min"] = json!([0]);
        accessors[10]["max"] = json!([2]);
        document["animations"] = json!([{
            "samplers": [{"input": 10, "output": 11, "interpolation": "LINEAR"},
                         {"input": 10, "output": 12, "interpolation": "LINEAR"}],
            "channels": [{"sampler": 0, "target": {"node": 0, "path": "weights"}},
                         {"sampler": 1, "target": {"node": 1, "path": "translation"}}]
        }]);
    }
    // glTF attribute sets are contiguous. Keep an independently valued, inactive
    // UV0 control so selecting it accidentally cannot reproduce the UV1 sample.
    let uv0 = accessors.len();
    accessors.push(json!({"componentType": 5126, "count": 3, "type": "VEC2"}));
    document["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(uv0);
    document["buffers"] = json!([{"uri": "mesh.bin", "byteLength": binary.len()}]);
    document["bufferViews"] = json!(views);
    document["accessors"] = json!(accessors);
    let io = MemoryAssetIo::new();
    io.try_insert(
        AssetPath::new("scene.gltf")?,
        Arc::from(serde_json::to_vec(&document)?),
    )?;
    io.try_insert(AssetPath::new("mesh.bin")?, Arc::from(binary))?;
    // Omitted UV decode selects red; one decode selects the purple texel; two select green.
    io.try_insert(
        AssetPath::new("quadrants.png")?,
        Arc::from(common::png(
            2,
            2,
            &[
                255, 0, 0, 255, 128, 64, 192, 255, 0, 255, 0, 255, 0, 0, 255, 255,
            ],
        )),
    )?;
    let importer = Importer::new()?;
    let report = importer.read_asset(&io, AssetPath::new("scene.gltf")?)?;
    assert!(report.loss_evidence().is_empty());
    let asset = ApplicationAsset::from_scene(report.scene())?;
    drop((report, importer, io, document));
    Ok(asset)
}

fn near<const N: usize>(actual: [f64; N], expected: [f64; N], tolerance: f64) {
    for (lane, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            actual.is_finite() && (actual - expected).abs() <= tolerance,
            "lane {lane}: {actual} != {expected} (tolerance {tolerance})"
        );
    }
}

fn packed<const N: usize>(bytes: &[u8], offset: usize) -> [f64; N] {
    std::array::from_fn(|lane| {
        let start = offset + lane * 4;
        f64::from(f32::from_le_bytes(
            bytes[start..start + 4].try_into().unwrap(),
        ))
    })
}

fn upload_sample(
    renderer: &Renderer,
    quantized: bool,
    skinned: bool,
    sample: &Witness,
) -> Result<GpuAsset> {
    let asset = detached(quantized, skinned)?;
    assert_eq!((asset.draws.len(), asset.instances.len()), (1, 1));
    assert_eq!(asset.instances[0].skin.is_some(), skinned);
    let source = &asset.draws[0];
    assert_eq!(source.vertices.len(), 3);
    let source_uv = if quantized { [-3.0, 5.0] } else { [0.75, 0.25] };
    assert_eq!(source.uv_streams.len(), 2);
    assert_eq!(source.uv_streams[0].set, 0);
    assert_eq!(source.uv_streams[0].values, [[0.0; 2]; 3]);
    assert_eq!(source.uv_streams[1].set, 1);
    assert_eq!(source.uv_streams[1].values, [source_uv; 3]);
    for (vertex, position) in source.vertices.iter().zip(if quantized {
        [[10.0, -6.0, 4.0], [34.0, -6.0, 4.0], [22.0, 18.0, 4.0]]
    } else {
        PHYSICAL
    }) {
        near(vertex.position, position, 0.0);
        let magnitude = if quantized { f64::from(SN126) } else { 1.0 };
        near(vertex.normal.unwrap(), [0.0, 0.0, magnitude], 0.0);
        near(vertex.tangent.unwrap(), [magnitude, 0.0, 0.0, -1.0], 0.0);
    }
    assert_eq!(source.morphs.len(), if skinned { 3 } else { 0 });
    for morph in &source.morphs {
        let expected = match morph.semantic {
            MorphDeltaSemantic::Position if quantized => [2.0, -2.0, 4.0],
            MorphDeltaSemantic::Position => [0.125, -0.125, 0.25],
            MorphDeltaSemantic::Normal => [0.0, if quantized { -SN63 } else { -0.5 }, 0.0],
            MorphDeltaSemantic::Tangent => [0.0, if quantized { SN63 } else { 0.5 }, 0.0],
        };
        assert_eq!(morph.target, 0);
        assert_eq!(morph.values, [expected; 3]);
    }
    let evaluation = evaluate(
        &asset,
        skinned.then_some(0),
        SampleTime::Declared(sample.time),
        EvaluationRequest {
            frames: FramePolicy::FrozenWeight,
            limits: EvaluationLimits::default(),
            cancellation: None,
        },
    )?;
    if skinned {
        assert_eq!(evaluation.morph_weights[0], [sample.time * 0.5]);
    }
    let draw = &evaluation.draws[0];
    let frames = evaluated_frames(&evaluation)?;
    let plan = DevicePlan::prepare(
        &asset,
        &evaluation,
        &frames,
        DeviceLimits::default(),
        Control::default(),
    )?;
    assert_eq!(plan.chunks().len(), 1);
    let chunk = &plan.chunks()[0];
    assert_eq!(chunk.vertex_sources, [0, 1, 2]);
    assert_eq!(chunk.indices, [0, 1, 2]);
    assert_eq!(
        chunk.field_flags,
        SOURCE_NORMAL | SOURCE_TANGENT | WORLD_TANGENT
    );
    assert_eq!(chunk.orientation, 1);
    assert_eq!(chunk.vertices.len(), 3 * 64);
    assert_eq!(chunk.uv_bytes.len(), 3 * 16);
    assert_eq!(plan.materials()[0].active_uv_sets, [1]);
    let slot = plan.materials()[0].slots[BASE].unwrap();
    assert_eq!(
        slot.coordinate(source_uv.map(|value| value as f32)),
        [0.75, 0.25]
    );
    assert_eq!(
        slot.affine,
        if quantized {
            [[0.125, 0.0, 1.125], [0.0, 0.125, -0.375]]
        } else {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        }
    );
    for (vertex, base) in PHYSICAL.into_iter().enumerate() {
        let world = std::array::from_fn(|lane| base[lane] + sample.shift[lane]);
        near(draw.world_positions[vertex], world, 1.0e-12);
        if skinned {
            near(
                draw.positions[vertex],
                [
                    (world[0] - 0.25) * 0.5,
                    world[1] + 0.125,
                    (world[2] - 0.5) * 2.0,
                ],
                1.0e-12,
            );
        } else {
            near(
                draw.positions[vertex],
                source.vertices[vertex].position,
                0.0,
            );
        }
        near(
            draw.world_normals.as_ref().unwrap()[vertex],
            sample.normal,
            1.0e-12,
        );
        near(
            draw.world_tangents.as_ref().unwrap()[vertex],
            sample.tangent,
            1.0e-12,
        );
        assert_eq!(draw.world_orientations.as_ref().unwrap()[vertex], 1);
        near(packed(&chunk.vertices, vertex * 64), world, 0.0);
        near(
            packed(&chunk.vertices, vertex * 64 + 16),
            sample.normal,
            6.0e-8,
        );
        near(
            packed(&chunk.vertices, vertex * 64 + 32),
            sample.tangent,
            6.0e-8,
        );
        near(packed(&chunk.uv_bytes, vertex * 16), [0.0; 2], 0.0);
        near(packed(&chunk.uv_bytes, vertex * 16 + 8), source_uv, 0.0);
    }
    let light = LightFrame {
        position: [0.0; 3],
        outgoing: [0.0, 0.0, -1.0],
        kind: LightKindCategory::Directional,
        tint: [1.0; 3],
        intensity: 1.0,
        range: None,
        cos_inner: 1.0,
        cos_outer: 1.0,
        meters_per_unit: 1.0,
    };
    // All importer/Scene/IO owners have already gone; returning drops asset/evaluation/plan.
    Ok(renderer.upload(
        &plan,
        common::camera(),
        RenderSettings {
            width: 64,
            height: 64,
            clear: [0.0; 4],
            ..RenderSettings::default()
        },
        &[light],
        Control::default(),
    )?)
}

fn compare_frames(quantized: &RenderedFrame, float: &RenderedFrame, sample: &Witness) {
    for frame in [quantized, float] {
        assert_eq!(
            (frame.width, frame.height, frame.linear_rgba.len()),
            (64, 64, 4096)
        );
        assert_eq!(frame.diagnostics.len(), 4096);
        assert!(frame.diagnostics.iter().all(|&code| code == 0));
        assert!(
            frame
                .linear_rgba
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        );
        assert!(
            frame
                .linear_rgba
                .iter()
                .filter(|pixel| pixel[3] > 0.99)
                .count()
                > 500
        );
        for y in 29..33 {
            for x in 29..33 {
                near(
                    frame.pixel(x, y).unwrap().map(f64::from),
                    sample.rgba.map(f64::from),
                    0.001,
                );
            }
        }
        assert_eq!(frame.pixel(2, 2).unwrap(), [0.0; 4]);
        let edge = if sample.time == 0.0 {
            sample.rgba
        } else {
            [0.0; 4]
        };
        near(
            frame.pixel(12, 50).unwrap().map(f64::from),
            edge.map(f64::from),
            0.001,
        );
    }
    for (pixel, (left, right)) in quantized
        .linear_rgba
        .iter()
        .zip(&float.linear_rgba)
        .enumerate()
    {
        for lane in 0..4 {
            assert!(
                (left[lane] - right[lane]).abs() <= 0.0005,
                "t={}: pixel {pixel}, lane {lane}: {} != {}",
                sample.time,
                left[lane],
                right[lane]
            );
        }
    }
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn quantized_static_textured_lighting_matches_physical_float_after_owner_drop() -> Result<()> {
    let renderer = common::renderer();
    let quantized = upload_sample(&renderer, true, false, &SAMPLES[0])?;
    let float = upload_sample(&renderer, false, false, &SAMPLES[0])?;
    compare_frames(
        &renderer.render(&quantized, Control::default())?,
        &renderer.render(&float, Control::default())?,
        &SAMPLES[0],
    );
    Ok(())
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn quantized_bound_skin_and_pnt_morph_match_float_at_keys_and_midpoint_after_owner_drop()
-> Result<()> {
    let renderer = common::renderer();
    let mut blue = Vec::new();
    for sample in &SAMPLES {
        let quantized = upload_sample(&renderer, true, true, sample)?;
        let float = upload_sample(&renderer, false, true, sample)?;
        let quantized = renderer.render(&quantized, Control::default())?;
        let float = renderer.render(&float, Control::default())?;
        compare_frames(&quantized, &float, sample);
        blue.push(quantized.pixel(31, 31).unwrap()[2]);
    }
    assert!(blue.windows(2).all(|pair| pair[0] - pair[1] > 0.003));
    Ok(())
}
