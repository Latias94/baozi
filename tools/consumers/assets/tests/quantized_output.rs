use std::sync::Arc;

use baozi::{
    AssetPath, AttributeElement, Axis, FrontFaceWinding, Handedness, ImportOperationControl,
    ImportRequest, Importer, MaterialAssignment, MemoryAssetIo, MorphDeltaSemantic,
    OptionCompileLimits, ProvenanceKindRef, SourceObjectLocatorRef, SourceScopeEntityKind,
    TargetSpace, TextureRole,
    postprocess::{PostProcessControl, SpaceConversionOptions, convert_scene_space},
};
use baozi_asset_consumer::{
    ApplicationAsset, ChannelTarget, EvaluationLimits, EvaluationRequest, FramePolicy, Result,
    SampleTime, evaluate,
};
use serde_json::{Value, json};

const SOURCE: &str = "quantized.gltf";
const BINARY: &str = "quantized.bin";
const QUANTIZATION: &str = "KHR_mesh_quantization";

fn append_f32(binary: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        binary.extend_from_slice(&value.to_le_bytes());
    }
}

fn triangle(representation: &str) -> (Value, Vec<u8>) {
    let mut binary = Vec::new();
    let (views, accessors) = if representation == "quantized" {
        for position in [[1_i16, 2, 3], [3, 2, 3], [1, 4, 3]] {
            for value in position {
                binary.extend_from_slice(&value.to_le_bytes());
            }
            binary
                .extend_from_slice(&[0xa5, 0xa5, 0, 0, 127, 0x5a, 127, 0, 0, 127, 2, 0, 253, 255]);
        }
        assert_eq!(binary.len(), 60);
        for index in [0_u16, 1, 2] {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        (
            json!([
                {"buffer": 0, "byteLength": 60, "byteStride": 20},
                {"buffer": 0, "byteOffset": 60, "byteLength": 6}
            ]),
            json!([
                {"bufferView": 0, "componentType": 5122, "count": 3, "type": "VEC3",
                 "min": [1,2,3], "max": [3,4,3]},
                {"bufferView": 0, "byteOffset": 8, "componentType": 5120, "normalized": true, "count": 3, "type": "VEC3"},
                {"bufferView": 0, "byteOffset": 12, "componentType": 5120, "normalized": true, "count": 3, "type": "VEC4"},
                {"bufferView": 0, "byteOffset": 16, "componentType": 5122, "count": 3, "type": "VEC2"},
                {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ]),
        )
    } else {
        let positions: &[f32] = if representation == "physical-float" {
            &[10.5, -1.0, 2.5, 11.5, -1.0, 2.5, 10.5, 0.0, 2.5]
        } else {
            &[1.0, 2.0, 3.0, 3.0, 2.0, 3.0, 1.0, 4.0, 3.0]
        };
        append_f32(&mut binary, positions);
        append_f32(&mut binary, &[0.0, 0.0, 1.0].repeat(3));
        append_f32(&mut binary, &[1.0, 0.0, 0.0, 1.0].repeat(3));
        let uv: &[f32] = if representation == "physical-float" {
            &[0.625, -1.75]
        } else {
            &[2.0, -3.0]
        };
        append_f32(&mut binary, &uv.repeat(3));
        for index in [0_u16, 1, 2] {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        (
            json!([
                {"buffer": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 36},
                {"buffer": 0, "byteOffset": 72, "byteLength": 48},
                {"buffer": 0, "byteOffset": 120, "byteLength": 24},
                {"buffer": 0, "byteOffset": 144, "byteLength": 6}
            ]),
            json!([
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                 "min": if representation == "physical-float" {json!([10.5,-1,2.5])} else {json!([1,2,3])},
                 "max": if representation == "physical-float" {json!([11.5,0,2.5])} else {json!([3,4,3])}},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"},
                {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2"},
                {"bufferView": 4, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ]),
        )
    };
    let physical = representation == "physical-float";
    let mut document = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}, {"nodes": [3]}],
        "nodes": [
            {"name": "placement", "translation": [2,3,4], "children": [1,2]},
            {"name": "decode-leaf", "mesh": 0,
             "translation": if physical {[0,0,0]} else {[10,-2,1]},
             "scale": if physical {[1.0,1.0,1.0]} else {[0.5,0.5,0.5]}},
            {"name": "second-placement", "mesh": 0,
             "translation": if physical {[10,0,0]} else {[20,-2,1]},
             "scale": if physical {[1.0,1.0,1.0]} else {[0.5,0.5,0.5]}},
            {"name": "omitted", "mesh": 1}
        ],
        "meshes": [
            {"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TANGENT": 2, "TEXCOORD_0": 3}, "indices": 4}]},
            {"primitives": [{"mode": 0, "attributes": {"POSITION": 5}}]}
        ],
        "buffers": [
            {"uri": BINARY, "byteLength": binary.len()},
            {"uri": "omitted-never-open.bin", "byteLength": 12}
        ],
        "bufferViews": views, "accessors": accessors
    });
    let omitted_view = document["bufferViews"].as_array().unwrap().len();
    document["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer": 1, "byteLength": 12}));
    document["accessors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView": omitted_view,
        "componentType": 5126, "count": 1, "type": "VEC3", "min": [0,0,0], "max": [0,0,0]}));
    if representation == "quantized" {
        document["extensionsUsed"] = json!([QUANTIZATION]);
        document["extensionsRequired"] = json!([QUANTIZATION]);
    }
    (document, binary)
}

fn add_skin(document: &mut Value, binary: &mut Vec<u8>, representation: &str, nonunit: bool) {
    while !binary.len().is_multiple_of(4) {
        binary.push(0x5a);
    }
    document["nodes"][0]["translation"] = json!([0, 0, 0]);
    document["nodes"][0]["children"] = json!([1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
    document["nodes"][1] =
        json!({"name": "mesh-a", "mesh": 0, "skin": 0, "translation": [100,-20,7]});
    document["nodes"][2] =
        json!({"name": "mesh-b", "mesh": 0, "skin": 1, "translation": [-100,20,-7]});
    for _ in 0..6 {
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"translation": [0,5,0]}));
    }
    for _ in 0..6 {
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"translation": [0,-1,0]}));
    }
    // All eight authored lanes survive, including zero lanes and a signed zero.
    for (semantic, packed, component) in [
        ("JOINTS_0", [0_u8, 1, 2, 3].repeat(3), 5121),
        ("JOINTS_1", [4_u8, 5, 0, 0].repeat(3), 5121),
    ] {
        let view = document["bufferViews"].as_array().unwrap().len();
        let accessor = document["accessors"].as_array().unwrap().len();
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 0, "byteOffset": binary.len(), "byteLength": packed.len(), "byteStride": 4}));
        document["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView": view, "componentType": component, "count": 3, "type": "VEC4"}));
        document["meshes"][0]["primitives"][0]["attributes"][semantic] = json!(accessor);
        binary.extend_from_slice(&packed);
    }
    for (semantic, weights) in [
        (
            "WEIGHTS_0",
            if nonunit {
                [0.125_f32; 4]
            } else {
                [1.0, 0.0, 0.0, 0.0]
            },
        ),
        (
            "WEIGHTS_1",
            if nonunit {
                [0.125, 0.125, 0.0, -0.0]
            } else {
                [0.0, 0.0, 0.0, -0.0]
            },
        ),
    ] {
        let view = document["bufferViews"].as_array().unwrap().len();
        let accessor = document["accessors"].as_array().unwrap().len();
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 0, "byteOffset": binary.len(), "byteLength": 48}));
        document["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView": view, "componentType": 5126, "count": 3, "type": "VEC4"}));
        document["meshes"][0]["primitives"][0]["attributes"][semantic] = json!(accessor);
        append_f32(binary, &weights.repeat(3));
    }
    let mut inverse_binds = Vec::new();
    for skin in 0..2 {
        let matrix: [f32; 16] = match (representation == "physical-float", skin) {
            (false, 0) => [
                0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 10.0, -2.0, 1.0, 1.0,
            ],
            (false, 1) => [
                0.25, 0.0, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0, 0.0, 0.0, 0.25, 0.0, -3.0, 2.0, -1.0, 1.0,
            ],
            (true, 0) => [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            (true, 1) => [
                0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, -8.0, 3.0, -1.5, 1.0,
            ],
            _ => unreachable!(),
        };
        let view = document["bufferViews"].as_array().unwrap().len();
        let accessor = document["accessors"].as_array().unwrap().len();
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 0, "byteOffset": binary.len(), "byteLength": 384}));
        document["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView": view, "componentType": 5126, "count": 6, "type": "MAT4"}));
        for _ in 0..6 {
            append_f32(binary, &matrix);
        }
        inverse_binds.push(accessor);
    }
    document["skins"] = json!([
        {"joints": [4,5,6,7,8,9], "inverseBindMatrices": inverse_binds[0]},
        {"joints": [10,11,12,13,14,15], "inverseBindMatrices": inverse_binds[1]}
    ]);
    document["buffers"][0]["byteLength"] = json!(binary.len());
}

fn add_morph(document: &mut Value, binary: &mut Vec<u8>, representation: &str) {
    while !binary.len().is_multiple_of(4) {
        binary.push(0x5a);
    }
    let mut target = json!({});
    for semantic in ["POSITION", "NORMAL", "TANGENT"] {
        let view = document["bufferViews"].as_array().unwrap().len();
        let accessor = document["accessors"].as_array().unwrap().len();
        let offset = binary.len();
        let quantized = representation == "quantized";
        let values: [f32; 3] = match (semantic, representation == "physical-float") {
            ("POSITION", true) => [1.0, -1.0, 2.0],
            ("POSITION", false) => [2.0, -2.0, 4.0],
            ("NORMAL", _) => [0.503937, 0.0, 0.0],
            ("TANGENT", _) => [0.0, 0.503937, 0.0],
            _ => unreachable!(),
        };
        if quantized {
            for _ in 0..3 {
                binary.extend_from_slice(match semantic {
                    "POSITION" => &[2, 0, 254, 255, 4, 0, 0xa5, 0xa5],
                    "NORMAL" => &[64, 0, 0, 0xa5],
                    "TANGENT" => &[0, 64, 0, 0xa5],
                    _ => unreachable!(),
                });
            }
        } else {
            append_f32(binary, &values.repeat(3));
        }
        let mut buffer_view =
            json!({"buffer": 0, "byteOffset": offset, "byteLength": binary.len() - offset});
        if quantized {
            buffer_view["byteStride"] = json!(if semantic == "POSITION" { 8 } else { 4 });
        }
        document["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(buffer_view);
        let mut row = json!({"bufferView": view, "componentType": if quantized {
                if semantic == "POSITION" {5122} else {5120}
            } else {5126},
            "normalized": quantized && semantic != "POSITION", "count": 3, "type": "VEC3"});
        if semantic == "POSITION" {
            row["min"] = json!(values);
            row["max"] = json!(values);
        }
        document["accessors"].as_array_mut().unwrap().push(row);
        target[semantic] = json!(accessor);
    }
    document["meshes"][0]["weights"] = json!([0.25, 0.0]);
    document["meshes"][0]["primitives"][0]["targets"] = json!([target, {}]);
    document["buffers"][0]["byteLength"] = json!(binary.len());
}

#[test]
fn detached_static_controls_keep_decode_leaf_mesh_sharing_and_selected_source_closure() -> Result<()>
{
    for representation in ["quantized", "decoded-float", "physical-float"] {
        let (document, binary) = triangle(representation);
        let io = MemoryAssetIo::new();
        io.try_insert(
            AssetPath::new(SOURCE)?,
            Arc::from(serde_json::to_vec(&document)?),
        )?;
        io.try_insert(AssetPath::new(BINARY)?, Arc::from(binary))?;
        let importer = Importer::new()?;
        let request =
            importer.prepare_request(&ImportRequest::new(), OptionCompileLimits::default())?;
        let report = importer.read_asset_with_request(
            &io,
            AssetPath::new(SOURCE)?,
            &request,
            ImportOperationControl::default(),
        )?;
        assert!(report.diagnostics().is_empty());
        assert!(report.loss_evidence().is_empty());
        let (scene, evidence) = report.into_parts();
        assert_eq!(scene.mesh_count(), 1);
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(primitive.indices(), Some([0, 1, 2].as_slice()));
        let positions = primitive.positions().unwrap();
        let Some(AttributeElement::F32(first)) = positions.element(0) else {
            panic!("F32");
        };
        assert_eq!(
            first,
            if representation == "physical-float" {
                &[10.5, -1.0, 2.5]
            } else {
                &[1.0, 2.0, 3.0]
            }
        );
        let mut pending = scene
            .owner_provenance(positions.metadata_owner())?
            .collect::<Vec<_>>();
        let mut visited = Vec::new();
        let mut reached_accessor = false;
        let mut reached_buffer = false;
        while let Some(node) = pending.pop() {
            if visited.contains(&node.id()) {
                continue;
            }
            assert_eq!(node.id().generation(), scene.generation());
            visited.push(node.id());
            assert!(visited.len() <= scene.provenance_node_count());
            if let ProvenanceKindRef::Source(source) = node.kind() {
                reached_buffer |= source.resource().identity() == BINARY;
                if let SourceObjectLocatorRef::Namespaced { value, .. } = source.locator() {
                    reached_accessor |= value.starts_with("kind=accessor;parent=-;index=0;");
                }
            }
            pending.extend(node.parents());
        }
        assert!(reached_accessor && reached_buffer);
        let asset = ApplicationAsset::from_scene(&scene)?;
        drop(scene);
        drop(request);
        drop(importer);
        drop(io);
        drop(document);

        assert_eq!(
            (asset.mesh_count, asset.draws.len(), asset.instances.len()),
            (1, 1, 2)
        );
        assert_eq!(
            asset
                .instances
                .iter()
                .map(|instance| instance.mesh)
                .collect::<Vec<_>>(),
            [0, 0]
        );
        assert_eq!(asset.nodes[1].name.as_deref(), Some("decode-leaf"));
        let trs = asset.nodes[1]
            .transform
            .authored_trs()
            .ok_or("authored decode TRS")?;
        let translation = trs.translation();
        let scale = trs.scale();
        assert_eq!(
            [translation.x, translation.y, translation.z],
            if representation == "physical-float" {
                [0.0, 0.0, 0.0]
            } else {
                [10.0, -2.0, 1.0]
            }
        );
        assert_eq!(
            [scale.x, scale.y, scale.z],
            if representation == "physical-float" {
                [1.0; 3]
            } else {
                [0.5; 3]
            }
        );
        assert_eq!(asset.draws[0].indices, [0, 1, 2]);
        assert_eq!(
            asset.draws[0].material,
            MaterialAssignment::ImplicitDefault(0)
        );
        let output = evaluate(
            &asset,
            None,
            SampleTime::Declared(0.0),
            EvaluationRequest {
                frames: FramePolicy::PositionOnly,
                limits: EvaluationLimits::default(),
                cancellation: None,
            },
        )?;
        assert_eq!(
            output.draws[0].world_positions,
            [[12.5, 2.0, 6.5], [13.5, 2.0, 6.5], [12.5, 3.0, 6.5]]
        );
        assert_eq!(output.draws[1].world_positions[0], [22.5, 2.0, 6.5]);
        assert_eq!(
            asset.draws[0].vertices[0].position,
            if representation == "physical-float" {
                [10.5, -1.0, 2.5]
            } else {
                [1.0, 2.0, 3.0]
            }
        );
        let scope = evidence
            .source_scope_record()
            .ok_or("detached source scope")?;
        for (kind, selected, omitted) in [
            (SourceScopeEntityKind::Scene, 1, 1),
            (SourceScopeEntityKind::Node, 3, 1),
            (SourceScopeEntityKind::Mesh, 1, 1),
            (SourceScopeEntityKind::Accessor, 5, 1),
            (SourceScopeEntityKind::Buffer, 1, 1),
        ] {
            assert_eq!(
                (scope.counts(kind).selected(), scope.counts(kind).omitted()),
                (selected, omitted)
            );
        }
        assert_eq!(evidence.ledger().current_live_bytes(), 0);
    }
    Ok(())
}

#[test]
fn shared_quantized_mesh_uses_each_skins_inverse_bind_once_and_keeps_every_influence() -> Result<()>
{
    for representation in ["quantized", "decoded-float", "physical-float"] {
        for nonunit in [false, true] {
            let (mut document, mut binary) = triangle(representation);
            add_skin(&mut document, &mut binary, representation, nonunit);
            let io = MemoryAssetIo::new();
            io.try_insert(
                AssetPath::new(SOURCE)?,
                Arc::from(serde_json::to_vec(&document)?),
            )?;
            io.try_insert(AssetPath::new(BINARY)?, Arc::from(binary))?;
            let importer = Importer::new()?;
            let report = importer.read_asset(&io, AssetPath::new(SOURCE)?)?;
            assert!(report.loss_evidence().is_empty());
            let asset = ApplicationAsset::from_scene(report.scene())?;
            drop(report);
            drop(importer);
            drop(io);
            drop(document);
            assert_eq!(
                (asset.mesh_count, asset.draws.len(), asset.skins.len()),
                (1, 1, 2)
            );
            assert_eq!(
                (asset.instances[0].skin, asset.instances[1].skin),
                (Some(0), Some(1))
            );
            let influences = asset.draws[0]
                .influences
                .as_ref()
                .ok_or("complete influence rows")?;
            assert_eq!(influences.offsets, [0, 8, 16, 24]);
            for row in 0..3 {
                assert_eq!(
                    &influences.joints[row * 8..row * 8 + 8],
                    &[0, 1, 2, 3, 4, 5, 0, 0]
                );
                let expected: [f32; 8] = if nonunit {
                    [0.125, 0.125, 0.125, 0.125, 0.125, 0.125, 0.0, -0.0]
                } else {
                    [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -0.0]
                };
                assert_eq!(
                    influences.weights[row * 8..row * 8 + 8]
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>(),
                    expected.map(f32::to_bits)
                );
            }
            if representation != "physical-float" {
                assert_eq!(
                    asset.skins[0].inverse_binds[0].cols,
                    [
                        [0.5, 0.0, 0.0, 0.0],
                        [0.0, 0.5, 0.0, 0.0],
                        [0.0, 0.0, 0.5, 0.0],
                        [10.0, -2.0, 1.0, 1.0]
                    ]
                );
                assert_eq!(
                    asset.skins[1].inverse_binds[0].cols,
                    [
                        [0.25, 0.0, 0.0, 0.0],
                        [0.0, 0.25, 0.0, 0.0],
                        [0.0, 0.0, 0.25, 0.0],
                        [-3.0, 2.0, -1.0, 1.0]
                    ]
                );
            }
            let output = evaluate(
                &asset,
                None,
                SampleTime::Declared(0.0),
                EvaluationRequest {
                    frames: FramePolicy::PositionOnly,
                    limits: EvaluationLimits::default(),
                    cancellation: None,
                },
            )?;
            assert_eq!(
                output.draws[0].world_positions[0],
                if nonunit {
                    [7.875, 3.0, 1.875]
                } else {
                    [10.5, 4.0, 2.5]
                }
            );
            assert_eq!(
                output.draws[1].world_positions[0],
                if nonunit {
                    [-2.0625, 1.125, -0.1875]
                } else {
                    [-2.75, 1.5, -0.25]
                }
            );
        }
    }
    Ok(())
}

#[test]
fn retained_morph_streams_are_relative_and_precede_static_or_skin_dequantization() -> Result<()> {
    for representation in ["quantized", "decoded-float", "physical-float"] {
        for skinned in [false, true] {
            let (mut document, mut binary) = triangle(representation);
            if skinned {
                add_skin(&mut document, &mut binary, representation, false);
            }
            add_morph(&mut document, &mut binary, representation);
            let io = MemoryAssetIo::new();
            io.try_insert(
                AssetPath::new(SOURCE)?,
                Arc::from(serde_json::to_vec(&document)?),
            )?;
            io.try_insert(AssetPath::new(BINARY)?, Arc::from(binary))?;
            let importer = Importer::new()?;
            let report = importer.read_asset(&io, AssetPath::new(SOURCE)?)?;
            assert!(report.loss_evidence().is_empty());
            let mut asset = ApplicationAsset::from_scene(report.scene())?;
            drop(report);
            drop(importer);
            drop(io);
            drop(document);
            assert_eq!(asset.mesh_morph_weights, [vec![0.25, 0.0]]);
            assert_eq!(asset.draws[0].morphs.len(), 3);
            for (stream, semantic) in asset.draws[0].morphs.iter().zip([
                MorphDeltaSemantic::Position,
                MorphDeltaSemantic::Normal,
                MorphDeltaSemantic::Tangent,
            ]) {
                assert_eq!(
                    (stream.target, stream.semantic, stream.values.len()),
                    (0, semantic, 3)
                );
                let expected: [f32; 3] = match semantic {
                    MorphDeltaSemantic::Position => {
                        if representation == "physical-float" {
                            [1.0, -1.0, 2.0]
                        } else {
                            [2.0, -2.0, 4.0]
                        }
                    }
                    MorphDeltaSemantic::Normal => [0.503937, 0.0, 0.0],
                    MorphDeltaSemantic::Tangent => [0.0, 0.503937, 0.0],
                };
                for value in &stream.values {
                    assert_eq!(value.map(f32::to_bits), expected.map(f32::to_bits));
                }
            }
            for (weight, static_expected, skin_expected) in [
                (0.0, [12.5, 2.0, 6.5], [10.5, 4.0, 2.5]),
                (0.25, [12.75, 1.75, 7.0], [10.75, 3.75, 3.0]),
                (1.0, [13.5, 1.0, 8.5], [11.5, 3.0, 4.5]),
                (-0.5, [12.0, 2.5, 5.5], [10.0, 4.5, 1.5]),
            ] {
                asset.mesh_morph_weights[0][0] = weight;
                let output = evaluate(
                    &asset,
                    None,
                    SampleTime::Declared(0.0),
                    EvaluationRequest {
                        frames: FramePolicy::FrozenWeightNormalsOnly,
                        limits: EvaluationLimits::default(),
                        cancellation: None,
                    },
                )?;
                assert_eq!(
                    output.draws[0].world_positions[0],
                    if skinned {
                        skin_expected
                    } else {
                        static_expected
                    }
                );
                assert!(output.draws[0].world_tangents.is_none());
                let normal = output.draws[0]
                    .world_normals
                    .as_ref()
                    .ok_or("deformed normal")?[0];
                let x = f64::from(0.503937_f32) * f64::from(weight);
                let length = (1.0 + x * x).sqrt();
                for (actual, expected) in normal.into_iter().zip([x / length, 0.0, 1.0 / length]) {
                    assert!((actual - expected).abs() <= 1.0e-12);
                }
            }
            asset.instances[0].morph_weights = Some(vec![-0.5, 0.0]);
            let output = evaluate(
                &asset,
                None,
                SampleTime::Declared(0.0),
                EvaluationRequest {
                    frames: FramePolicy::PositionOnly,
                    limits: EvaluationLimits::default(),
                    cancellation: None,
                },
            )?;
            assert_eq!(output.morph_weights[0], [-0.5, 0.0]);
            assert_eq!(
                output.draws[0].world_positions[0],
                if skinned {
                    [10.0, 4.5, 1.5]
                } else {
                    [12.0, 2.5, 5.5]
                }
            );
        }
    }
    Ok(())
}

#[test]
fn animated_parent_and_linear_morph_sample_leave_the_decode_leaf_authored() -> Result<()> {
    let (mut document, mut binary) = triangle("quantized");
    add_morph(&mut document, &mut binary, "quantized");
    let mut rows = Vec::new();
    for (shape, count, values) in [
        ("SCALAR", 2, vec![0.0_f32, 2.0]),
        ("VEC3", 2, vec![2.0, 3.0, 4.0, 6.0, 3.0, 4.0]),
        ("SCALAR", 4, vec![0.0, 0.0, 1.0, 0.0]),
    ] {
        let view = document["bufferViews"].as_array().unwrap().len();
        rows.push(document["accessors"].as_array().unwrap().len());
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer":0,"byteOffset":binary.len(),"byteLength":values.len() * 4}));
        let mut accessor =
            json!({"bufferView":view,"componentType":5126,"type":shape,"count":count});
        if rows.len() == 1 {
            accessor["min"] = json!([0]);
            accessor["max"] = json!([2]);
        }
        document["accessors"].as_array_mut().unwrap().push(accessor);
        append_f32(&mut binary, &values);
    }
    document["buffers"][0]["byteLength"] = json!(binary.len());
    document["animations"] = json!([{
        "samplers":[{"input":rows[0],"output":rows[1],"interpolation":"LINEAR"},
                    {"input":rows[0],"output":rows[2],"interpolation":"LINEAR"}],
        "channels":[{"sampler":0,"target":{"node":0,"path":"translation"}},
                    {"sampler":1,"target":{"node":1,"path":"weights"}}]
    }]);
    let io = MemoryAssetIo::new();
    io.try_insert(
        AssetPath::new(SOURCE)?,
        Arc::from(serde_json::to_vec(&document)?),
    )?;
    io.try_insert(AssetPath::new(BINARY)?, Arc::from(binary))?;
    let importer = Importer::new()?;
    let report = importer.read_asset(&io, AssetPath::new(SOURCE)?)?;
    let asset = ApplicationAsset::from_scene(report.scene())?;
    drop(report);
    drop(importer);
    drop(io);
    drop(document);
    assert_eq!(
        asset.animations[0].channels[0].target,
        ChannelTarget::Translation(0)
    );
    assert_eq!(asset.instances[0].node, 1);
    assert_eq!(asset.nodes[1].name.as_deref(), Some("decode-leaf"));
    assert_eq!(
        asset.animations[0].channels[1].target,
        ChannelTarget::MorphWeights(0)
    );
    let output = evaluate(
        &asset,
        Some(0),
        SampleTime::Declared(0.5),
        EvaluationRequest {
            frames: FramePolicy::PositionOnly,
            limits: EvaluationLimits::default(),
            cancellation: None,
        },
    )?;
    assert_eq!(output.morph_weights[0], [0.25, 0.0]);
    assert_eq!(output.world_transforms[0][3], [3.0, 3.0, 4.0, 1.0]);
    assert_eq!(
        output.local_transforms[1],
        [
            [0.5, 0.0, 0.0, 0.0],
            [0.0, 0.5, 0.0, 0.0],
            [0.0, 0.0, 0.5, 0.0],
            [10.0, -2.0, 1.0, 1.0]
        ]
    );
    assert_eq!(output.draws[0].world_positions[0], [13.75, 1.75, 7.0]);
    // The first sample coincides with the authored default weight. This second
    // time must distinguish actual channel application from keeping defaults.
    let midpoint = evaluate(
        &asset,
        Some(0),
        SampleTime::Declared(1.0),
        EvaluationRequest {
            frames: FramePolicy::PositionOnly,
            limits: EvaluationLimits::default(),
            cancellation: None,
        },
    )?;
    assert_eq!(midpoint.morph_weights[0], [0.5, 0.0]);
    assert_eq!(midpoint.morph_weights[1], [0.25, 0.0]);
    assert_eq!(midpoint.world_transforms[0][3], [4.0, 3.0, 4.0, 1.0]);
    assert_eq!(midpoint.draws[0].world_positions[0], [15.0, 1.5, 7.5]);
    Ok(())
}

#[test]
fn detached_uv_bindings_keep_the_combined_transform_and_override_without_baking_shared_uvs()
-> Result<()> {
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5,
        1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    for representation in ["quantized", "decoded-float", "physical-float"] {
        let (mut document, mut binary) = triangle(representation);
        let quantized = representation == "quantized";
        let physical = representation == "physical-float";
        if quantized {
            for vertex in 0..3 {
                binary[vertex * 20 + 16..vertex * 20 + 20].copy_from_slice(&[7, 0, 11, 0]);
            }
        } else {
            for vertex in 0..3 {
                binary[120 + vertex * 8..124 + vertex * 8].copy_from_slice(&7.0_f32.to_le_bytes());
                binary[124 + vertex * 8..128 + vertex * 8].copy_from_slice(&11.0_f32.to_le_bytes());
            }
        }
        while !binary.len().is_multiple_of(4) {
            binary.push(0x5a);
        }
        let view = document["bufferViews"].as_array().unwrap().len();
        let accessor = document["accessors"].as_array().unwrap().len();
        let offset = binary.len();
        if quantized {
            binary.extend_from_slice(&[2, 0, 253, 255].repeat(3));
        } else {
            let uv = if physical {
                [0.625, -1.75]
            } else {
                [2.0, -3.0]
            };
            append_f32(&mut binary, &uv.repeat(3));
        }
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer":0,"byteOffset":offset,"byteLength":binary.len() - offset,"byteStride":if quantized {4} else {8}}));
        document["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView":view,"componentType":if quantized {5122} else {5126},"count":3,"type":"VEC2"}));
        document["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_1"] = json!(accessor);
        document["meshes"][0]["primitives"][0]["material"] = json!(0);
        let mut extensions = vec!["KHR_texture_transform", "KHR_materials_unlit"];
        if quantized {
            extensions.push(QUANTIZATION);
        }
        document["extensionsUsed"] = json!(extensions);
        document["extensionsRequired"] = json!(extensions);
        document["materials"] = json!([{
            "extensions":{"KHR_materials_unlit":{}},
            "pbrMetallicRoughness":{"baseColorTexture":{"index":0,"texCoord":0,
                "extensions":{"KHR_texture_transform":{
                    "texCoord":1,"offset":if physical {[1.0,2.0]} else {[1.125,2.25]},
                    "scale":if physical {[2.0,0.5]} else {[0.5,0.25]},
                    "rotation":std::f32::consts::FRAC_PI_2}}}},
            "occlusionTexture":{"index":0,"texCoord":0,"extensions":{"KHR_texture_transform":{
                "offset":[-0.25,0.5],"scale":[0.5,2.0]}}}
        }]);
        document["images"] = json!([{"uri":"sentinel.png","mimeType":"image/png"}]);
        document["textures"] = json!([{"source":0,"sampler":0}]);
        document["samplers"] =
            json!([{"magFilter":9728,"minFilter":9728,"wrapS":10497,"wrapT":10497}]);
        document["buffers"][0]["byteLength"] = json!(binary.len());
        let io = MemoryAssetIo::new();
        io.try_insert(
            AssetPath::new(SOURCE)?,
            Arc::from(serde_json::to_vec(&document)?),
        )?;
        io.try_insert(AssetPath::new(BINARY)?, Arc::from(binary))?;
        io.try_insert(AssetPath::new("sentinel.png")?, Arc::from(PNG))?;
        let importer = Importer::new()?;
        let report = importer.read_asset(&io, AssetPath::new(SOURCE)?)?;
        let asset = ApplicationAsset::from_scene(report.scene())?;
        drop(report);
        drop(importer);
        drop(io);
        drop(document);
        assert_eq!(asset.draws[0].material, MaterialAssignment::Authored(0));
        assert_eq!(
            (
                asset.materials[0].bindings.len(),
                asset.images.len(),
                asset.textures.len(),
                asset.samplers.len()
            ),
            (2, 1, 1, 1)
        );
        assert_eq!(asset.images[0].encoded_bytes, PNG);
        let draw = &asset.draws[0];
        assert_eq!(draw.uv_streams.len(), 2);
        assert_eq!(draw.uv_stream(0).unwrap().values, [[7.0, 11.0]; 3]);
        assert_eq!(
            draw.uv_stream(1).unwrap().values,
            if physical {
                [[0.625, -1.75]; 3]
            } else {
                [[2.0, -3.0]; 3]
            }
        );
        for binding in &asset.materials[0].bindings {
            assert_eq!(binding.texture, 0);
            let uv = draw
                .uv_stream(u32::from(binding.uv_set))
                .ok_or("exact bound UV set")?
                .values[0];
            let transform = binding.transform;
            let offset = transform.offset();
            let scale = transform.scale();
            let (sine, cosine) = f64::from(transform.rotation_radians()).sin_cos();
            let sampled = [
                f64::from(offset.x) + f64::from(scale.x) * uv[0] * cosine
                    - f64::from(scale.y) * uv[1] * sine,
                f64::from(offset.y)
                    + f64::from(scale.x) * uv[0] * sine
                    + f64::from(scale.y) * uv[1] * cosine,
            ];
            let expected = match binding.role {
                TextureRole::BaseColor => {
                    assert_eq!(binding.uv_set, 1);
                    assert_eq!(
                        [offset.x, offset.y],
                        if physical { [1.0, 2.0] } else { [1.125, 2.25] }
                    );
                    assert_eq!(
                        [scale.x, scale.y],
                        if physical { [2.0, 0.5] } else { [0.5, 0.25] }
                    );
                    [1.875, 3.25]
                }
                TextureRole::Occlusion => {
                    assert_eq!(binding.uv_set, 0);
                    [3.25, 22.5]
                }
                _ => panic!("unexpected retained binding"),
            };
            for (actual, expected) in sampled.into_iter().zip(expected) {
                assert!((actual - expected).abs() <= 2.0e-6);
            }
        }
        // The image remains encoded; actual 2x2 sampling/readback belongs to the renderer lane.
        assert_eq!(asset.textures[0].image, 0);
        assert_eq!(asset.textures[0].sampler, 0);
    }
    Ok(())
}

#[test]
fn coordinate_conversion_reuses_the_same_static_skin_and_morph_equations() -> Result<()> {
    for (skinned, morphed, expected) in [
        (false, false, [-25.0, 4.0, 13.0]),
        (true, false, [-21.0, 8.0, 5.0]),
        (true, true, [-21.5, 7.5, 6.0]),
    ] {
        let (mut document, mut binary) = triangle("quantized");
        if skinned {
            add_skin(&mut document, &mut binary, "quantized", false);
        }
        if morphed {
            add_morph(&mut document, &mut binary, "quantized");
        }
        let io = MemoryAssetIo::new();
        io.try_insert(
            AssetPath::new(SOURCE)?,
            Arc::from(serde_json::to_vec(&document)?),
        )?;
        io.try_insert(AssetPath::new(BINARY)?, Arc::from(binary))?;
        let importer = Importer::new()?;
        let (scene, evidence) = importer
            .read_asset(&io, AssetPath::new(SOURCE)?)?
            .into_parts();
        let (scene, conversion) = convert_scene_space(
            scene,
            SpaceConversionOptions {
                target: TargetSpace::try_new(
                    Handedness::Left,
                    Axis::PositiveY,
                    Axis::PositiveZ,
                    0.5,
                    FrontFaceWinding::CounterClockwise,
                )?,
                source_assumptions: None,
            },
            PostProcessControl::default(),
        )?
        .into_parts();
        let asset = ApplicationAsset::from_scene(&scene)?;
        drop(scene);
        drop(evidence);
        drop(conversion);
        drop(importer);
        drop(io);
        drop(document);
        let output = evaluate(
            &asset,
            None,
            SampleTime::Declared(0.0),
            EvaluationRequest {
                frames: FramePolicy::PositionOnly,
                limits: EvaluationLimits::default(),
                cancellation: None,
            },
        )?;
        assert!(output.draws[0].world_positions.contains(&expected));
    }
    Ok(())
}
