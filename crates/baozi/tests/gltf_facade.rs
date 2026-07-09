#![cfg(feature = "format-gltf")]

use baozi::{
    AssetPath, ExternalReferencePolicy, ImportOptions, ImportStage, Importer, MemoryAssetIo, Result,
};
use base64::Engine as _;

fn triangle_gltf() -> Vec<u8> {
    br#"{
  "asset": { "version": "2.0", "generator": "baozi-facade-test" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [{ "mesh": 0 }],
  "buffers": [{ "uri": "triangle.bin", "byteLength": 42 }],
  "bufferViews": [
    { "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 },
    { "buffer": 0, "byteOffset": 36, "byteLength": 6, "target": 34963 }
  ],
  "accessors": [
    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] },
    { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
  ],
  "meshes": [{
    "primitives": [{
      "attributes": { "POSITION": 0 },
      "indices": 1,
      "mode": 4
    }]
  }]
}"#
    .to_vec()
}

fn triangle_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [
        0.0f32, 0.0, 0.0, //
        1.0, 0.0, 0.0, //
        0.0, 1.0, 0.0,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes
}

fn triangle_data_uri_gltf() -> Vec<u8> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(triangle_bin());
    let text = String::from_utf8(triangle_gltf()).expect("fixture is valid utf-8");
    text.replace(
        r#""uri": "triangle.bin", "byteLength": 42"#,
        &format!(r#""uri": "data:application/octet-stream;base64,{encoded}", "byteLength": 42"#),
    )
    .into_bytes()
}

#[test]
fn facade_reports_gltf_resource_ledger_stats() -> Result<()> {
    let source = AssetPath::new("models/scene.gltf")?;
    let buffer = AssetPath::new("models/triangle.bin")?;
    let gltf = triangle_gltf();
    let bin = triangle_bin();
    let primary_len = gltf.len() as u64;
    let sidecar_len = bin.len() as u64;

    let mut io = MemoryAssetIo::new();
    io.insert(source.clone(), gltf);
    io.insert(buffer, bin);
    let mut options = ImportOptions::memory();
    options.io.external_references = ExternalReferencePolicy::CustomResolver;

    let report = Importer::new().read_asset_with_options(&io, source, options)?;

    assert_eq!(report.format().id(), "gltf");
    assert_eq!(report.stage(), ImportStage::ValidatedImported);
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.scene().meshes.len(), 1);
    assert_eq!(report.stats().primary_asset_bytes(), primary_len);
    assert_eq!(report.stats().sidecar_asset_bytes(), sidecar_len);
    assert_eq!(report.stats().data_uri_bytes(), 0);
    assert_eq!(
        report.stats().total_asset_bytes(),
        primary_len + sidecar_len
    );
    assert_eq!(report.stats().opened_assets(), 2);
    assert_eq!(report.stats().generated_meshes(), 1);
    assert_eq!(report.stats().generated_vertices(), 3);
    assert_eq!(report.stats().generated_faces(), 1);
    Ok(())
}

#[test]
fn facade_reports_gltf_data_uri_resource_ledger_stats() -> Result<()> {
    let source = AssetPath::new("models/scene.gltf")?;
    let gltf = triangle_data_uri_gltf();
    let primary_len = gltf.len() as u64;
    let data_uri_len = triangle_bin().len() as u64;

    let mut io = MemoryAssetIo::new();
    io.insert(source.clone(), gltf);

    let report = Importer::new().read_asset_with_options(&io, source, ImportOptions::memory())?;

    assert_eq!(report.format().id(), "gltf");
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.scene().meshes.len(), 1);
    assert_eq!(report.stats().primary_asset_bytes(), primary_len);
    assert_eq!(report.stats().sidecar_asset_bytes(), 0);
    assert_eq!(report.stats().data_uri_bytes(), data_uri_len);
    assert_eq!(
        report.stats().total_asset_bytes(),
        primary_len + data_uri_len
    );
    assert_eq!(report.stats().opened_assets(), 1);
    Ok(())
}
