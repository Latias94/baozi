use std::sync::Arc;

use baozi::{
    AssetPath, AttributeData, AttributeDomain, Axis, DiagnosticSeverity, FrontFaceWinding,
    Handedness, Importer, Mat4, MemoryAssetIo, MorphDeltaSemantic, TargetSpace,
    postprocess::{
        NormalMode, NormalOptions, PostProcessControl, SpaceConversionOptions, TangentPolicy,
        convert_scene_space, generate_normals,
    },
};
use baozi_asset_consumer::{
    ApplicationAsset, EvaluationLimits, EvaluationRequest, FramePolicy, Result, SampleTime,
    evaluate,
};
use serde_json::json;

const POSITIONS: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
];
const CORNERS: [u32; 6] = [2, 0, 1, 1, 0, 3];
const POSITION_DELTAS: [[f32; 3]; 4] = [
    [0.125, -0.0, 0.25],
    [0.5, -0.125, 0.0],
    [-0.25, 0.75, 0.125],
    [1.0, 0.25, -0.5],
];
const JOINTS: [[u32; 4]; 4] = [[3, 1, 2, 0], [0, 2, 3, 1], [1, 3, 0, 2], [2, 0, 1, 3]];
const WEIGHTS: [[f32; 4]; 4] = [
    [-0.0, 0.125, 0.375, 0.5],
    [0.25, 0.5, 0.25, 0.0],
    [0.625, 0.125, 0.125, 0.125],
    [0.5, 0.25, 0.125, 0.125],
];

fn folded_quad_gltf() -> Result<Vec<u8>> {
    // Independently authored little-endian payload: four F32 positions, six U16 indices,
    // four F32 position deltas, four U8 joint rows, and four F32 weight rows.
    // The two faces point along +Z and +Y. No authored normal or tangent masks generation.
    // Unbound JOINTS/WEIGHTS are retained data; this fixture deliberately needs no skin.
    let document = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{
            "weights": [0.5],
            "primitives": [{
                "attributes": {"POSITION": 0, "JOINTS_0": 3, "WEIGHTS_0": 4},
                "indices": 1,
                "mode": 4,
                "targets": [{"POSITION": 2}]
            }]
        }],
        "buffers": [{
            "byteLength": 188,
            "uri": concat!(
                "data:application/octet-stream;base64,",
                "AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/",
                "AgAAAAEAAQAAAAMAAAAAPgAAAIAAAIA+AAAAPwAAAL4AAAAAAACAvgAAQD8AAAA+",
                "AACAPwAAgD4AAAC/AwECAAACAwEBAwACAgABAwAAAIAAAAA+AADAPgAAAD8AAIA+",
                "AAAAPwAAgD4AAAAAAAAgPwAAAD4AAAA+AAAAPgAAAD8AAIA+AAAAPgAAAD4="
            )
        }],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48, "target": 34962},
            {"buffer": 0, "byteOffset": 48, "byteLength": 12, "target": 34963},
            {"buffer": 0, "byteOffset": 60, "byteLength": 48, "target": 34962},
            {"buffer": 0, "byteOffset": 108, "byteLength": 16, "target": 34962},
            {"buffer": 0, "byteOffset": 124, "byteLength": 64, "target": 34962}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
             "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 1.0]},
            {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"},
            {"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC3",
             "min": [-0.25, -0.125, -0.5], "max": [1.0, 0.75, 0.25]},
            {"bufferView": 3, "componentType": 5121, "count": 4, "type": "VEC4"},
            {"bufferView": 4, "componentType": 5126, "count": 4, "type": "VEC4"}
        ]
    });
    Ok(serde_json::to_vec(&document)?)
}

#[test]
fn flat_normal_seams_reindex_morphs_and_influences_after_scene_drop() -> Result<()> {
    let document = folded_quad_gltf()?;
    let report = Importer::new()?.read_bytes("deformation-seams.gltf", &document)?;
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    let (scene, _) = report.into_parts();
    assert_eq!(
        (
            scene.mesh_count(),
            scene.primitive_count(),
            scene.skin_count()
        ),
        (1, 1, 0)
    );
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!((primitive.vertex_count(), primitive.corner_count()), (4, 6));
    assert_eq!(primitive.indices(), Some(CORNERS.as_slice()));
    assert_eq!(primitive.morph_target_count(), 1);
    assert!(primitive.normals(AttributeDomain::Vertex).is_none());
    assert!(primitive.normals(AttributeDomain::Corner).is_none());
    let target = primitive.id();

    let (scene, _) = generate_normals(
        scene,
        target,
        NormalOptions {
            mode: NormalMode::Flat,
            ..NormalOptions::default()
        },
        TangentPolicy::default(),
        PostProcessControl::default(),
    )?
    .into_parts();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!((primitive.vertex_count(), primitive.corner_count()), (4, 6));
    assert_eq!(primitive.indices(), Some(CORNERS.as_slice()));
    assert!(primitive.normals(AttributeDomain::Vertex).is_none());
    let AttributeData::DenseF32 { values, lanes: 3 } = primitive
        .normals(AttributeDomain::Corner)
        .ok_or("missing generated Corner normals")?
        .data()
    else {
        panic!("expected generated F32x3 normals");
    };
    assert_eq!(values.len(), 18);
    assert_eq!(&values[..3], &[0.0, 0.0, 1.0]);
    assert_eq!(&values[9..12], &[0.0, 1.0, 0.0]);
    let normal_bits: Vec<[u64; 3]> = values
        .chunks_exact(3)
        .map(|row| std::array::from_fn(|lane| f64::from(row[lane]).to_bits()))
        .collect();

    let asset = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    drop(document);

    assert_eq!(asset.draws.len(), 1);
    let draw = &asset.draws[0];
    assert_eq!((draw.vertices.len(), draw.indices.len()), (6, 6));
    assert_eq!(draw.indices, [0, 1, 2, 3, 4, 5]);
    assert_eq!(draw.morphs.len(), 1);
    let morph = &draw.morphs[0];
    assert_eq!(
        (morph.target, morph.semantic),
        (0, MorphDeltaSemantic::Position)
    );
    assert_eq!(morph.values.len(), 6);
    let rows = draw
        .influences
        .as_ref()
        .ok_or("missing exported influence rows")?;
    assert_eq!(rows.offsets, [0, 4, 8, 12, 16, 20, 24]);
    assert_eq!((rows.joints.len(), rows.weights.len()), (24, 24));

    // The first encounter order differs from source order; shared sources 0 and 1 each split.
    // Every assertion below reads owned output after the Scene has been destroyed.
    for (corner, &source) in CORNERS.iter().enumerate() {
        let source = usize::try_from(source)?;
        let index = usize::try_from(draw.indices[corner])?;
        let vertex = &draw.vertices[index];
        assert_eq!(
            vertex.position.map(f64::to_bits),
            POSITIONS[source].map(|value| f64::from(value).to_bits())
        );
        assert_eq!(
            vertex.normal.map(|v| v.map(f64::to_bits)),
            Some(normal_bits[corner])
        );
        assert!(vertex.tangent.is_none());
        assert!(draw.uv_streams.is_empty());
        assert_eq!(
            morph.values[index].map(f32::to_bits),
            POSITION_DELTAS[source].map(f32::to_bits)
        );
        let range = rows.offsets[index]..rows.offsets[index + 1];
        assert_eq!(&rows.joints[range.clone()], JOINTS[source].as_slice());
        let weights: [f32; 4] = rows.weights[range].try_into()?;
        assert_eq!(weights.map(f32::to_bits), WEIGHTS[source].map(f32::to_bits));
    }
    Ok(())
}

// Six distinct positive joints across two sets, followed by retained zero and signed-zero
// lanes. Different source rows make a seam/CSR representative shift observable.
const BOUND_JOINTS: [[u8; 8]; 4] = [
    [5, 4, 3, 2, 1, 0, 0, 1],
    [0, 1, 2, 3, 4, 5, 0, 1],
    [2, 5, 0, 3, 4, 1, 0, 1],
    [3, 2, 1, 0, 5, 4, 0, 1],
];
const BOUND_WEIGHTS: [f32; 8] = [0.25, 0.25, 0.25, 0.25, 0.25, 0.75, 0.0, -0.0];
const BOUND_DELTAS: [[f32; 3]; 4] = [
    [0.125, -0.0, 0.25],
    [2.0, 0.0, 1.0],
    [-0.25, 0.75, 0.125],
    [1.0, 0.25, -0.5],
];

fn bound_folded_quad_io() -> Result<MemoryAssetIo> {
    let mut payload = Vec::new();
    for value in POSITIONS.iter().flatten() {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for index in CORNERS {
        payload.extend_from_slice(&u16::try_from(index)?.to_le_bytes());
    }
    for value in BOUND_DELTAS.iter().flatten() {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for set in 0..2 {
        for joints in BOUND_JOINTS {
            payload.extend_from_slice(&joints[set * 4..set * 4 + 4]);
        }
        for _ in 0..4 {
            for weight in &BOUND_WEIGHTS[set * 4..set * 4 + 4] {
                payload.extend_from_slice(&weight.to_le_bytes());
            }
        }
    }
    for offset in 1..=6 {
        let mut bind = Mat4::IDENTITY;
        bind.cols[3][0] = -(offset as f32);
        for value in bind.cols.into_iter().flatten() {
            payload.extend_from_slice(&value.to_le_bytes());
        }
    }
    assert_eq!(payload.len(), 652);
    // A's effective joint-bind translations alternate 10/12. B's reverse-ordered palette
    // alternates 22/20. Every inverse bind has its own nonidentity translation.
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0, 1, 2]}],
        "nodes": [
            {"name": "first", "mesh": 0, "skin": 0, "translation": [10, 0, 0]},
            {"name": "second", "mesh": 0, "skin": 1, "translation": [20, 0, 0], "weights": [0.25]},
            {"name": "rig", "children": [3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]},
            {"name": "a0", "translation": [11, 0, 0]},
            {"name": "a1", "translation": [14, 0, 0]},
            {"name": "a2", "translation": [13, 0, 0]},
            {"name": "a3", "translation": [16, 0, 0]},
            {"name": "a4", "translation": [15, 0, 0]},
            {"name": "a5", "translation": [18, 0, 0]},
            {"name": "b5", "translation": [26, 0, 0]},
            {"name": "b4", "translation": [27, 0, 0]},
            {"name": "b3", "translation": [24, 0, 0]},
            {"name": "b2", "translation": [25, 0, 0]},
            {"name": "b1", "translation": [22, 0, 0]},
            {"name": "b0", "translation": [23, 0, 0]}
        ],
        "skins": [
            {"joints": [3, 4, 5, 6, 7, 8], "skeleton": 2, "inverseBindMatrices": 7},
            {"joints": [14, 13, 12, 11, 10, 9], "skeleton": 2, "inverseBindMatrices": 7}
        ],
        "meshes": [{"weights": [0.5], "primitives": [{
            "attributes": {"POSITION": 0, "JOINTS_0": 3, "WEIGHTS_0": 4,
                "JOINTS_1": 5, "WEIGHTS_1": 6},
            "indices": 1, "targets": [{"POSITION": 2}]
        }]}],
        "buffers": [{"uri": "bound-seams.bin", "byteLength": 652}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48},
            {"buffer": 0, "byteOffset": 48, "byteLength": 12},
            {"buffer": 0, "byteOffset": 60, "byteLength": 48},
            {"buffer": 0, "byteOffset": 108, "byteLength": 16},
            {"buffer": 0, "byteOffset": 124, "byteLength": 64},
            {"buffer": 0, "byteOffset": 188, "byteLength": 16},
            {"buffer": 0, "byteOffset": 204, "byteLength": 64},
            {"buffer": 0, "byteOffset": 268, "byteLength": 384}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
                "min": [0, 0, 0], "max": [1, 1, 1]},
            {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"},
            {"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC3",
                "min": [-0.25, 0, -0.5], "max": [2, 0.75, 1]},
            {"bufferView": 3, "componentType": 5121, "count": 4, "type": "VEC4"},
            {"bufferView": 4, "componentType": 5126, "count": 4, "type": "VEC4"},
            {"bufferView": 5, "componentType": 5121, "count": 4, "type": "VEC4"},
            {"bufferView": 6, "componentType": 5126, "count": 4, "type": "VEC4"},
            {"bufferView": 7, "componentType": 5126, "count": 6, "type": "MAT4"}
        ]
    });
    let io = MemoryAssetIo::new();
    io.try_insert(
        AssetPath::new("bound-seams.gltf")?,
        Arc::from(serde_json::to_vec(&document)?),
    )?;
    io.try_insert(AssetPath::new("bound-seams.bin")?, Arc::from(payload))?;
    Ok(io)
}

fn detached_bound_seams() -> Result<(ApplicationAsset, ApplicationAsset)> {
    let io = bound_folded_quad_io()?;
    let importer = Importer::new()?;
    let report = importer.read_asset(&io, AssetPath::new("bound-seams.gltf")?)?;
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "gltf.influence_weight_sum"
    );
    assert_eq!(
        report.diagnostics()[0].severity(),
        DiagnosticSeverity::Warning
    );
    assert!(report.loss_evidence().is_empty());
    let (scene, evidence) = report.into_parts();
    assert_eq!(
        (
            scene.mesh_count(),
            scene.skin_count(),
            scene.mesh_instance_count()
        ),
        (1, 2, 2)
    );
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.indices(), Some(CORNERS.as_slice()));
    assert!(primitive.normals(AttributeDomain::Vertex).is_none());
    assert!(primitive.normals(AttributeDomain::Corner).is_none());
    let target = primitive.id();
    let (scene, normals) = generate_normals(
        scene,
        target,
        NormalOptions {
            mode: NormalMode::Flat,
            ..NormalOptions::default()
        },
        TangentPolicy::default(),
        PostProcessControl::default(),
    )?
    .into_parts();
    let source = ApplicationAsset::from_scene(&scene)?;
    let (scene, conversion) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: TargetSpace::try_new(
                Handedness::Left,
                Axis::PositiveY,
                Axis::NegativeZ,
                0.01,
                FrontFaceWinding::CounterClockwise,
            )?,
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    let converted = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    drop(conversion);
    drop(normals);
    drop(evidence);
    drop(importer);
    drop(io);
    Ok((source, converted))
}

fn point(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            actual.is_finite() && (actual - expected).abs() <= 1.0e-10,
            "{actual} != {expected}"
        );
    }
}

#[test]
fn bound_skin_morph_and_flat_normal_seams_evaluate_after_every_source_owner_is_dropped()
-> Result<()> {
    // Independent authored source-vertex tables. W=2, and morph precedes each palette.
    const LOCAL: [[[f64; 3]; 4]; 2] = [
        [
            [1.625, 0.0, 0.25],
            [6.5, 0.0, 1.0],
            [2.25, 2.75, 0.125],
            [2.5, 0.25, 1.5],
        ],
        [
            [2.5625, 0.0, 0.125],
            [4.5, 0.0, 0.5],
            [1.375, 2.375, 0.0625],
            [3.0, 0.125, 1.75],
        ],
    ];
    const WORLD: [[[f64; 3]; 4]; 2] = [
        [
            [21.625, 0.0, 0.25],
            [26.5, 0.0, 1.0],
            [22.25, 2.75, 0.125],
            [22.5, 0.25, 1.5],
        ],
        [
            [42.5625, 0.0, 0.125],
            [44.5, 0.0, 0.5],
            [41.375, 2.375, 0.0625],
            [43.0, 0.125, 1.75],
        ],
    ];
    let (source, converted) = detached_bound_seams()?;
    for (asset, corners, scale) in [
        (source, CORNERS, [1.0, 1.0, 1.0]),
        (converted, [2, 1, 0, 1, 3, 0], [100.0, 100.0, -100.0]),
    ] {
        let h = |point: [f64; 3]| std::array::from_fn(|lane| point[lane] * scale[lane]);
        assert_eq!(
            (asset.draws.len(), asset.instances.len(), asset.skins.len()),
            (1, 2, 2)
        );
        assert_eq!(asset.mesh_morph_weights, [vec![0.5]]);
        let indices = ["first", "second"].map(|name| {
            asset
                .instances
                .iter()
                .position(|instance| asset.nodes[instance.node].name.as_deref() == Some(name))
                .unwrap()
        });
        assert!(asset.instances[indices[0]].morph_weights.is_none());
        assert_eq!(
            asset.instances[indices[1]].morph_weights.as_deref(),
            Some([0.25].as_slice())
        );
        for (instance, names) in indices.into_iter().zip([
            ["a0", "a1", "a2", "a3", "a4", "a5"],
            ["b0", "b1", "b2", "b3", "b4", "b5"],
        ]) {
            let skin = &asset.skins[asset.instances[instance].skin.unwrap()];
            assert_eq!(skin.joints.len(), 6);
            assert_eq!(skin.inverse_binds.len(), 6);
            for (slot, (&joint, name)) in skin.joints.iter().zip(names).enumerate() {
                assert_eq!(asset.nodes[joint].name.as_deref(), Some(name));
                assert_eq!(
                    skin.inverse_binds[slot].cols[3][0],
                    -(slot as f32 + 1.0) * scale[0] as f32
                );
            }
        }
        let draw = &asset.draws[0];
        assert_eq!(draw.indices, [0, 1, 2, 3, 4, 5]);
        assert_eq!(draw.vertices.len(), 6);
        assert_eq!(draw.morphs.len(), 1);
        assert_eq!(
            (draw.morphs[0].target, draw.morphs[0].semantic),
            (0, MorphDeltaSemantic::Position)
        );
        let rows = draw
            .influences
            .as_ref()
            .ok_or("missing complete skin rows")?;
        assert_eq!(rows.offsets, [0, 8, 16, 24, 32, 40, 48]);
        assert_eq!((rows.joints.len(), rows.weights.len()), (48, 48));
        for (corner, source) in corners.into_iter().enumerate() {
            let source = usize::try_from(source)?;
            let vertex = usize::try_from(draw.indices[corner])?;
            point(
                draw.vertices[vertex].position,
                h(POSITIONS[source].map(f64::from)),
            );
            point(
                draw.vertices[vertex].normal.unwrap(),
                if corner < 3 {
                    [0.0, 0.0, scale[2].signum()]
                } else {
                    [0.0, 1.0, 0.0]
                },
            );
            point(
                draw.morphs[0].values[vertex].map(f64::from),
                h(BOUND_DELTAS[source].map(f64::from)),
            );
            let range = rows.offsets[vertex]..rows.offsets[vertex + 1];
            assert_eq!(
                &rows.joints[range.clone()],
                BOUND_JOINTS[source].map(u32::from).as_slice()
            );
            let weights: [f32; 8] = rows.weights[range].try_into()?;
            assert_eq!(weights.map(f32::to_bits), BOUND_WEIGHTS.map(f32::to_bits));
        }
        let evaluated = evaluate(
            &asset,
            None,
            SampleTime::Declared(0.0),
            EvaluationRequest {
                frames: FramePolicy::PositionOnly,
                limits: EvaluationLimits::default(),
                cancellation: None,
            },
        )?;
        assert_eq!(evaluated.morph_weights[indices[0]], [0.5]);
        assert_eq!(evaluated.morph_weights[indices[1]], [0.25]);
        drop(asset);
        assert_eq!(evaluated.draws.len(), 2);
        for (instance, &index) in indices.iter().enumerate() {
            let draw = evaluated
                .draws
                .iter()
                .find(|draw| draw.instance == index && draw.draw == 0)
                .ok_or("missing evaluated placement")?;
            assert!(draw.world_normals.is_none() && draw.world_tangents.is_none());
            assert!(draw.world_orientations.is_none());
            for (corner, source) in corners.into_iter().enumerate() {
                let source = usize::try_from(source)?;
                point(draw.positions[corner], h(LOCAL[instance][source]));
                point(draw.world_positions[corner], h(WORLD[instance][source]));
            }
        }
    }
    Ok(())
}
