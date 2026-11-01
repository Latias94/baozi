use baozi::{
    AttributeData, AttributeDomain, Importer, MorphDeltaSemantic,
    postprocess::{NormalMode, NormalOptions, PostProcessControl, generate_normals},
};
use baozi_asset_consumer::{ApplicationAsset, Result};
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
        assert!(vertex.uv.is_none());
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
