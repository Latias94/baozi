use std::sync::Arc;

use baozi::{
    AssetPath, Importer, Mat4, MaterialAssignment, MemoryAssetIo, MorphDeltaSemantic, TopologyKind,
    ValidatedScene, postprocess::list_vertices,
};
use baozi_asset_consumer::{ApplicationAsset, Result};
use serde_json::{Value, json};

const POSITIONS: [[f32; 3]; 6] = [
    [0.0, -0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [1.0, 1.0, 0.0],
    [2.0, 0.0, 0.0],
    [2.0, 1.0, 0.0],
];
const NORMAL: [f32; 3] = [0.0, -0.0, 1.0];
const TANGENT: [f32; 4] = [1.0, 0.0, -0.0, -1.0];
const UVS: [[f32; 2]; 6] = [
    [-0.0, 0.0],
    [0.25, 0.0],
    [0.0, 0.5],
    [0.25, 0.5],
    [1.0, 0.0],
    [1.0, 0.5],
];
const DELTAS: [[f32; 3]; 6] = [
    [0.125, -0.0, 0.25],
    [0.25, 0.5, -0.125],
    [-0.5, 0.125, 0.0],
    [0.75, -0.25, 0.5],
    [-0.125, 1.0, -0.5],
    [1.0, 0.25, -0.0],
];
const JOINTS: [[u8; 4]; 6] = [
    [3, 1, 0, 2],
    [1, 2, 3, 0],
    [2, 0, 1, 3],
    [0, 3, 2, 1],
    [3, 2, 1, 0],
    [1, 0, 3, 2],
];
const WEIGHTS: [[f32; 4]; 6] = [
    [-0.0, 0.125, 0.375, 0.5],
    [0.25, 0.5, 0.125, 0.125],
    [0.5, 0.25, 0.25, 0.0],
    [0.125, 0.125, 0.25, 0.5],
    [0.625, -0.0, 0.25, 0.125],
    [0.25, 0.25, 0.125, 0.375],
];
// The leading repeated index makes the first strip triangle degenerate. The first real
// triangle must still use odd parity. Source vertex 5 is deliberately unreferenced.
const INDICES: [u32; 6] = [4, 4, 1, 3, 0, 2];

fn read_gltf(document: Value, payload: Vec<u8>) -> Result<ValidatedScene> {
    let source = AssetPath::new("primitive-modes.gltf")?;
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::from(serde_json::to_vec(&document)?))?;
    io.try_insert(AssetPath::new("primitive-modes.bin")?, Arc::from(payload))?;
    let report = Importer::new()?.read_asset(&io, source)?;
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    Ok(report.into_parts().0)
}

fn mixed_modes_scene() -> Result<ValidatedScene> {
    // One independently authored buffer and one shared mesh; every glTF mode has both an
    // implicit stream and a reordered indexed stream. All fields are authored, without edits.
    let mut payload = Vec::new();
    for value in POSITIONS
        .into_iter()
        .flatten()
        .chain([NORMAL; 6].into_iter().flatten())
        .chain([TANGENT; 6].into_iter().flatten())
        .chain(UVS.into_iter().flatten())
        .chain(DELTAS.into_iter().flatten())
    {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    payload.extend(JOINTS.into_iter().flatten());
    for value in WEIGHTS.into_iter().flatten() {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for index in INDICES {
        payload.extend_from_slice(&u16::try_from(index)?.to_le_bytes());
    }
    assert_eq!(payload.len(), 492);
    let primitives: Vec<_> = (0..=6)
        .flat_map(|mode| {
            [false, true].map(move |indexed| {
                let mut primitive = json!({
                    "attributes": {"POSITION": 0, "NORMAL": 1, "TANGENT": 2,
                                   "TEXCOORD_0": 3, "JOINTS_0": 5, "WEIGHTS_0": 6},
                    "mode": mode,
                    "targets": [{"POSITION": 4}]
                });
                if indexed {
                    primitive["indices"] = json!(7);
                }
                primitive
            })
        })
        .collect();
    read_gltf(
        json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0, 1, 2]}],
            "nodes": [
                {"name": "first", "mesh": 0, "skin": 0},
                {"name": "second", "mesh": 0, "skin": 0,
                 "translation": [10.0, 0.0, 0.0], "weights": [0.25]},
                {"name": "joint0", "children": [3, 4, 5]},
                {"name": "joint1"}, {"name": "joint2"}, {"name": "joint3"}
            ],
            "skins": [{"skeleton": 2, "joints": [4, 2, 5, 3]}],
            "meshes": [{"weights": [0.5], "primitives": primitives}],
            "buffers": [{"uri": "primitive-modes.bin", "byteLength": payload.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 72},
                {"buffer": 0, "byteOffset": 72, "byteLength": 72},
                {"buffer": 0, "byteOffset": 144, "byteLength": 96},
                {"buffer": 0, "byteOffset": 240, "byteLength": 48},
                {"buffer": 0, "byteOffset": 288, "byteLength": 72},
                {"buffer": 0, "byteOffset": 360, "byteLength": 24, "byteStride": 4},
                {"buffer": 0, "byteOffset": 384, "byteLength": 96},
                {"buffer": 0, "byteOffset": 480, "byteLength": 12}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 6, "type": "VEC3",
                 "min": [0.0, 0.0, 0.0], "max": [2.0, 1.0, 0.0]},
                {"bufferView": 1, "componentType": 5126, "count": 6, "type": "VEC3"},
                {"bufferView": 2, "componentType": 5126, "count": 6, "type": "VEC4"},
                {"bufferView": 3, "componentType": 5126, "count": 6, "type": "VEC2"},
                {"bufferView": 4, "componentType": 5126, "count": 6, "type": "VEC3",
                 "min": [-0.5, -0.25, -0.5], "max": [1.0, 1.0, 0.5]},
                {"bufferView": 5, "componentType": 5121, "count": 6, "type": "VEC4"},
                {"bufferView": 6, "componentType": 5126, "count": 6, "type": "VEC4"},
                {"bufferView": 7, "componentType": 5123, "count": 6, "type": "SCALAR"}
            ]
        }),
        payload,
    )
}

#[test]
fn seven_modes_preserve_source_order_and_owned_deformation_tuples() -> Result<()> {
    use TopologyKind::{
        LineList, LineLoop, LineStrip, Points, TriangleFan, TriangleList, TriangleStrip,
    };

    // Expected source Corner order is authored independently of the reader. Odd strip faces
    // use [f, f + 2, f + 1]; fan faces use [f + 1, f + 2, 0], including degenerate faces.
    let cases: [(TopologyKind, TopologyKind, u32, &[u32]); 7] = [
        (Points, Points, 6, &[0, 1, 2, 3, 4, 5]),
        (LineList, LineList, 3, &[0, 1, 2, 3, 4, 5]),
        (LineLoop, LineList, 6, &[0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 0]),
        (LineStrip, LineList, 5, &[0, 1, 1, 2, 2, 3, 3, 4, 4, 5]),
        (TriangleList, TriangleList, 2, &[0, 1, 2, 3, 4, 5]),
        (
            TriangleStrip,
            TriangleList,
            4,
            &[0, 1, 2, 1, 3, 2, 2, 3, 4, 3, 5, 4],
        ),
        (
            TriangleFan,
            TriangleList,
            4,
            &[1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5, 0],
        ),
    ];
    let scene = mixed_modes_scene()?;
    assert_eq!((scene.mesh_count(), scene.mesh_instance_count()), (1, 2));
    assert_eq!((scene.primitive_count(), scene.skin_count()), (14, 1));
    let generation = scene.generation();
    let mesh = scene.meshes().next().ok_or("missing shared mesh")?;
    let mut expected_draws = Vec::new();
    for (mode, (source_topology, topology, face_count, corners)) in cases.into_iter().enumerate() {
        for indexed in [false, true] {
            let primitive = mesh
                .primitives()
                .nth(mode * 2 + usize::from(indexed))
                .ok_or("missing primitive")?;
            assert_eq!(primitive.topology(), source_topology);
            assert_eq!((primitive.vertex_count(), primitive.corner_count()), (6, 6));
            assert_eq!(primitive.face_count(), face_count);
            assert_eq!(primitive.indices(), indexed.then_some(INDICES.as_slice()));
            let view = list_vertices(primitive)?;
            let count = u32::try_from(corners.len())?;
            assert_eq!(view.topology(), topology);
            assert_eq!(view.primitive().id(), primitive.id());
            assert_eq!(view.len(), usize::try_from(count)?);
            assert_eq!(view.indices(), 0..count);
            assert!(view.vertex(view.len()).is_none());
            let source_vertices = if indexed { INDICES } else { [0, 1, 2, 3, 4, 5] };
            let target = primitive
                .morph_targets()
                .next()
                .ok_or("missing morph target")?;
            let arity = count / face_count;
            let mut sources = Vec::new();
            for (ordinal, (tuple, &corner)) in view.vertices().zip(corners).enumerate() {
                let source = source_vertices[usize::try_from(corner)?];
                assert_eq!(tuple.index(), u32::try_from(ordinal)?);
                assert_eq!(tuple.source_corner().index, corner);
                assert_eq!(tuple.source_vertex().index, source);
                assert_eq!(tuple.source_face().index, u32::try_from(ordinal)? / arity);
                let row = tuple.influences().ok_or("missing tuple influences")?;
                let source_index = usize::try_from(source)?;
                assert_eq!(row.joint_indices(), JOINTS[source_index].map(u32::from));
                assert_eq!(row.source_orders(), [0, 1, 2, 3]);
                assert_eq!(
                    row.weights()
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>(),
                    WEIGHTS[source_index].map(f32::to_bits)
                );
                let delta = tuple
                    .morph_delta(target.id(), MorphDeltaSemantic::Position)?
                    .ok_or("missing tuple morph delta")?;
                assert_eq!(
                    [delta.x, delta.y, delta.z].map(f32::to_bits),
                    DELTAS[source_index].map(f32::to_bits)
                );
                sources.push(source);
            }
            assert_eq!(sources.len(), corners.len());
            expected_draws.push((topology, sources));
        }
    }
    let asset = ApplicationAsset::from_scene(&scene)?;
    // Expansion preserves the original Scene generation, topology, and index streams.
    assert_eq!(scene.generation(), generation);
    for (index, primitive) in mesh.primitives().enumerate() {
        assert_eq!(primitive.topology(), cases[index / 2].0);
        assert_eq!(primitive.face_count(), cases[index / 2].2);
        assert_eq!((primitive.vertex_count(), primitive.corner_count()), (6, 6));
        assert_eq!(
            primitive.indices(),
            (index % 2 == 1).then_some(INDICES.as_slice())
        );
    }
    drop(scene);

    assert_eq!(
        (asset.mesh_count, asset.draws.len(), asset.instances.len()),
        (1, 14, 2)
    );
    for (index, (draw, (topology, sources))) in asset.draws.iter().zip(expected_draws).enumerate() {
        assert_eq!(
            (draw.mesh, draw.primitive, draw.material),
            (0, index, MaterialAssignment::ImplicitDefault(0))
        );
        assert_eq!(draw.topology, topology);
        assert_eq!(draw.indices.len(), sources.len());
        assert_eq!(draw.vertices.len(), if index % 2 == 0 { 6 } else { 5 });
        let rows = draw.influences.as_ref().ok_or("missing owned influences")?;
        assert_eq!(
            rows.offsets,
            (0..=draw.vertices.len()).map(|v| v * 4).collect::<Vec<_>>()
        );
        assert_eq!(
            (rows.joints.len(), rows.weights.len()),
            (draw.vertices.len() * 4, draw.vertices.len() * 4)
        );
        assert_eq!(draw.morphs.len(), 1);
        let morph = &draw.morphs[0];
        assert_eq!(
            (morph.target, morph.semantic),
            (0, MorphDeltaSemantic::Position)
        );
        assert_eq!(morph.values.len(), draw.vertices.len());
        let mut representatives = Vec::new();
        for (&packed, source) in draw.indices.iter().zip(sources) {
            let packed = usize::try_from(packed)?;
            let expected_index = match representatives.iter().position(|&v| v == source) {
                Some(index) => index,
                None => {
                    representatives.push(source);
                    representatives.len() - 1
                }
            };
            assert_eq!(
                packed, expected_index,
                "representatives follow output tuple order"
            );
            let source = usize::try_from(source)?;
            let vertex = &draw.vertices[packed];
            assert_eq!(
                vertex.position.map(f64::to_bits),
                POSITIONS[source].map(|v| f64::from(v).to_bits())
            );
            assert_eq!(
                vertex.normal.map(|v| v.map(f64::to_bits)),
                Some(NORMAL.map(|v| f64::from(v).to_bits()))
            );
            assert_eq!(
                vertex.tangent.map(|v| v.map(f64::to_bits)),
                Some(TANGENT.map(|v| f64::from(v).to_bits()))
            );
            assert_eq!(
                draw.uv_stream(0).unwrap().values[packed].map(f64::to_bits),
                UVS[source].map(|v| f64::from(v).to_bits())
            );
            assert_eq!(
                morph.values[packed].map(f32::to_bits),
                DELTAS[source].map(f32::to_bits)
            );
            let range = rows.offsets[packed]..rows.offsets[packed + 1];
            assert_eq!(rows.joints[range.clone()], JOINTS[source].map(u32::from));
            assert_eq!(
                rows.weights[range]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                WEIGHTS[source].map(f32::to_bits)
            );
        }
    }
    assert_eq!(asset.mesh_morph_weights, [vec![0.5]]);
    for (name, weights) in [("first", None), ("second", Some([0.25]))] {
        let instance = asset
            .instances
            .iter()
            .find(|i| asset.nodes[i.node].name.as_deref() == Some(name))
            .ok_or("missing owned instance")?;
        assert_eq!((instance.mesh, instance.skin), (0, Some(0)));
        assert_eq!(
            instance.morph_weights.as_deref(),
            weights.as_ref().map(|w| w.as_slice())
        );
    }
    assert_eq!(asset.skins.len(), 1);
    let skin = &asset.skins[0];
    assert_eq!(skin.inverse_binds, [Mat4::IDENTITY; 4]);
    assert_eq!(
        skin.joints
            .iter()
            .map(|&i| asset.nodes[i].name.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("joint2"),
            Some("joint0"),
            Some("joint3"),
            Some("joint1")
        ]
    );
    Ok(())
}

#[test]
fn two_point_loops_retain_both_directed_edges_after_scene_drop() -> Result<()> {
    let mut payload = Vec::new();
    for value in POSITIONS[..2].iter().flatten() {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for index in [1_u16, 0] {
        payload.extend_from_slice(&index.to_le_bytes());
    }
    let scene = read_gltf(
        json!({
            "asset": {"version": "2.0"},
            "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [
                {"attributes": {"POSITION": 0}, "mode": 2},
                {"attributes": {"POSITION": 0}, "indices": 1, "mode": 2}
            ]}],
            "buffers": [{"uri": "primitive-modes.bin", "byteLength": payload.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 24},
                {"buffer": 0, "byteOffset": 24, "byteLength": 4}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 2, "type": "VEC3",
                 "min": [0.0, 0.0, 0.0], "max": [1.0, 0.0, 0.0]},
                {"bufferView": 1, "componentType": 5123, "count": 2, "type": "SCALAR"}
            ]
        }),
        payload,
    )?;
    let asset = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    assert_eq!(asset.draws.len(), 2);
    for (draw, sources) in asset.draws.iter().zip([[0, 1, 1, 0], [1, 0, 0, 1]]) {
        assert_eq!(draw.topology, TopologyKind::LineList);
        assert_eq!(draw.indices, [0, 1, 1, 0]);
        assert_eq!(draw.vertices.len(), 2);
        assert!(
            draw.vertices
                .iter()
                .all(|vertex| { vertex.normal.is_none() && vertex.tangent.is_none() })
        );
        assert!(draw.uv_streams.is_empty());
        for (&index, source) in draw.indices.iter().zip(sources) {
            assert_eq!(
                draw.vertices[usize::try_from(index)?]
                    .position
                    .map(f64::to_bits),
                POSITIONS[source].map(|v| f64::from(v).to_bits())
            );
        }
    }
    Ok(())
}
