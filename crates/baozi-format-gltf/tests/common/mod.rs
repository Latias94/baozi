#![allow(dead_code)]

use baozi_core::{BaoziError, Diagnostic, Result, Scene};
use baozi_format_gltf::GltfImporter;
use baozi_import::{FormatImporter, ImportContext, ImportOptions};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::sync::Arc;

pub fn import_assets(
    source: &str,
    assets: impl IntoIterator<Item = (&'static str, Vec<u8>)>,
    options: ImportOptions,
) -> Result<(Scene, Vec<Diagnostic>)> {
    let (scene, diagnostics) = import_assets_result(source, assets, options)?;
    Ok((scene?, diagnostics))
}

pub fn import_assets_result(
    source: &str,
    assets: impl IntoIterator<Item = (&'static str, Vec<u8>)>,
    options: ImportOptions,
) -> Result<(Result<Scene>, Vec<Diagnostic>)> {
    let source_path = AssetPath::new(source)?;
    let mut io = MemoryAssetIo::new();
    for (path, bytes) in assets {
        io.insert(AssetPath::new(path)?, Arc::<[u8]>::from(bytes));
    }
    let mut ctx = ImportContext::with_options(&io, source_path, options);
    let result = GltfImporter.read(&mut ctx);
    let diagnostics = ctx.into_diagnostics();
    Ok((result, diagnostics))
}

pub fn expected_error(result: Result<Scene>) -> Result<BaoziError> {
    match result {
        Ok(_) => Err(BaoziError::parse(
            "test",
            None,
            "expected glTF import to fail",
        )),
        Err(error) => Ok(error),
    }
}

pub fn sidecar_options() -> ImportOptions {
    let mut options = ImportOptions::memory();
    options.io.external_references = baozi_import::ExternalReferencePolicy::CustomResolver;
    options
}

pub fn triangle_gltf() -> Vec<u8> {
    br#"{
  "asset": { "version": "2.0", "generator": "baozi-test" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [
    { "name": "Root", "children": [1] },
    { "name": "TriangleNode", "mesh": 0 }
  ],
  "buffers": [{ "uri": "triangle.bin", "byteLength": 104 }],
  "bufferViews": [
    { "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 },
    { "buffer": 0, "byteOffset": 36, "byteLength": 36, "target": 34962 },
    { "buffer": 0, "byteOffset": 72, "byteLength": 24, "target": 34962 },
    { "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }
  ],
  "accessors": [
    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] },
    { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" },
    { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" },
    { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }
  ],
  "images": [{ "uri": "textures/base.png", "name": "BaseImage" }],
  "samplers": [{ "wrapS": 33071, "wrapT": 33648, "magFilter": 9729, "minFilter": 9987 }],
  "textures": [{ "source": 0, "sampler": 0, "name": "BaseTex" }],
  "materials": [{
    "name": "Red",
    "pbrMetallicRoughness": {
      "baseColorFactor": [0.8, 0.1, 0.2, 0.7],
      "metallicFactor": 0.5,
      "roughnessFactor": 0.25,
      "baseColorTexture": { "index": 0, "texCoord": 0 }
    },
    "alphaMode": "BLEND",
    "doubleSided": true
  }],
  "meshes": [{
    "name": "Triangle",
    "primitives": [{
      "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
      "indices": 3,
      "material": 0,
      "mode": 4
    }]
  }]
}"#
    .to_vec()
}

pub fn triangle_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [
        0.0f32, 0.0, 0.0, //
        1.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, //
        0.0, 0.0, 1.0, //
        0.0, 0.0, 1.0, //
        0.0, 0.0, 1.0, //
        0.0, 0.0, //
        1.0, 0.0, //
        0.0, 1.0,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes.resize(104, 0);
    bytes
}

pub fn triangle_glb() -> Vec<u8> {
    let json = br#"{
  "asset": { "version": "2.0", "generator": "baozi-test" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [{ "name": "TriangleNode", "mesh": 0 }],
  "buffers": [{ "byteLength": 104 }],
  "bufferViews": [
    { "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 },
    { "buffer": 0, "byteOffset": 36, "byteLength": 36, "target": 34962 },
    { "buffer": 0, "byteOffset": 72, "byteLength": 24, "target": 34962 },
    { "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }
  ],
  "accessors": [
    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] },
    { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" },
    { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" },
    { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }
  ],
  "meshes": [{
    "name": "Triangle",
    "primitives": [{
      "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
      "indices": 3,
      "mode": 4
    }]
  }]
}"#;
    make_glb(json, &triangle_bin())
}

fn make_glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let json = padded_chunk(json, b' ');
    let bin = padded_chunk(bin, 0);
    let total_len = 12 + 8 + json.len() + 8 + bin.len();

    let mut bytes = Vec::with_capacity(total_len);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&(total_len as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"JSON");
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"BIN\0");
    bytes.extend_from_slice(&bin);
    bytes
}

fn padded_chunk(bytes: &[u8], pad: u8) -> Vec<u8> {
    let mut chunk = bytes.to_vec();
    while !chunk.len().is_multiple_of(4) {
        chunk.push(pad);
    }
    chunk
}

pub fn data_uri_gltf() -> Vec<u8> {
    br#"{
  "asset": { "version": "2.0" },
  "buffers": [{ "uri": "data:application/octet-stream;base64,AAAA", "byteLength": 3 }]
}"#
    .to_vec()
}
