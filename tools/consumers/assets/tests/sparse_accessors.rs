use std::sync::Arc;

use baozi::{AssetPath, Importer, MemoryAssetIo, MorphDeltaSemantic, TopologyKind};
use baozi_asset_consumer::{ApplicationAsset, Result};
use serde_json::json;

#[test]
fn sparse_geometry_and_zero_base_morph_survive_owned_application_extraction() -> Result<()> {
    let io = MemoryAssetIo::new();
    let source = AssetPath::new("sparse.gltf")?;
    // The base contains two deliberately different values. Sparse replacements must win.
    let mut base = Vec::new();
    for value in [0.0f32, 0.0, 0.0, 99.0, 99.0, 99.0, 88.0, 88.0, 88.0] {
        base.extend_from_slice(&value.to_le_bytes());
    }
    let mut values = Vec::new();
    for value in [
        1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, // Position replacements.
        0.0, 0.0, 0.25, 0.0, 0.0, -0.5, // Zero-base morph replacements.
    ] {
        values.extend_from_slice(&value.to_le_bytes());
    }
    let document = json!({
        "asset": {"version": "2.0"},
        "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"weights": [0.5], "primitives": [{
            "attributes": {"POSITION": 0}, "targets": [{"POSITION": 1}]
        }]}],
        "buffers": [
            {"uri": "base.bin", "byteLength": base.len()},
            {"uri": "indices.bin", "byteLength": 2},
            {"uri": "values.bin", "byteLength": values.len()}
        ],
        "bufferViews": [
            {"buffer": 0, "byteLength": base.len()},
            {"buffer": 1, "byteLength": 2},
            {"buffer": 2, "byteLength": 24},
            {"buffer": 2, "byteOffset": 24, "byteLength": 24}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "type": "VEC3", "count": 3,
             "min": [0,0,0], "max": [1,1,0],
             "sparse": {"count": 2, "indices": {"bufferView": 1, "componentType": 5121},
                        "values": {"bufferView": 2}}},
            {"componentType": 5126, "type": "VEC3", "count": 3,
             "min": [0,0,-0.5], "max": [0,0,0.25],
             "sparse": {"count": 2, "indices": {"bufferView": 1, "componentType": 5121},
                        "values": {"bufferView": 3}}}
        ]
    });
    for (path, bytes) in [
        ("sparse.gltf", serde_json::to_vec(&document)?),
        ("base.bin", base),
        ("indices.bin", vec![1, 2]),
        ("values.bin", values),
    ] {
        io.try_insert(AssetPath::new(path)?, Arc::from(bytes))?;
    }
    let report = Importer::new()?.read_asset(&io, source)?;
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    let asset = ApplicationAsset::from_scene(report.scene())?;
    drop(report);
    drop(io);

    let draw = asset.draws.first().ok_or("missing sparse draw")?;
    assert_eq!(draw.topology, TopologyKind::TriangleList);
    assert_eq!(draw.indices, [0, 1, 2]);
    let positions: Vec<_> = draw
        .vertices
        .iter()
        .map(|vertex| vertex.position.map(f64::to_bits))
        .collect();
    assert_eq!(
        positions,
        [
            [0.0f64, 0.0, 0.0].map(f64::to_bits),
            [1.0f64, 0.0, 0.0].map(f64::to_bits),
            [0.0f64, 1.0, 0.0].map(f64::to_bits),
        ]
    );
    let morph = draw.morphs.first().ok_or("missing sparse morph")?;
    assert_eq!(morph.semantic, MorphDeltaSemantic::Position);
    assert_eq!(
        morph.values,
        [[0.0, 0.0, 0.0], [0.0, 0.0, 0.25], [0.0, 0.0, -0.5]]
    );
    Ok(())
}
