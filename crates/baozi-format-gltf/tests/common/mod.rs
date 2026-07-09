#![allow(dead_code)]

use baozi_core::{BaoziError, Diagnostic, Result, Scene};
use baozi_format_gltf::GltfImporter;
use baozi_import::{FormatImporter, ImportContext, ImportOptions};
use baozi_io::{AssetPath, MemoryAssetIo};
use base64::Engine as _;
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

pub fn skinned_triangle_gltf() -> Vec<u8> {
    br#"{
  "asset": { "version": "2.0", "generator": "baozi-skin-test" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [
    { "name": "RigRoot", "children": [1, 2] },
    { "name": "Joint0", "children": [3] },
    { "name": "MeshNode", "mesh": 0, "skin": 0 },
    { "name": "Joint1" }
  ],
  "buffers": [{ "uri": "skin.bin", "byteLength": 244 }],
  "bufferViews": [
    { "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 },
    { "buffer": 0, "byteOffset": 36, "byteLength": 6, "target": 34963 },
    { "buffer": 0, "byteOffset": 42, "byteLength": 24, "target": 34962 },
    { "buffer": 0, "byteOffset": 68, "byteLength": 48, "target": 34962 },
    { "buffer": 0, "byteOffset": 116, "byteLength": 128 }
  ],
  "accessors": [
    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] },
    { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" },
    { "bufferView": 2, "componentType": 5123, "count": 3, "type": "VEC4" },
    { "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4" },
    { "bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4" }
  ],
  "skins": [{ "name": "Skin", "joints": [1, 3], "skeleton": 1, "inverseBindMatrices": 4 }],
  "meshes": [{
    "name": "SkinnedTriangle",
    "primitives": [{
      "attributes": { "POSITION": 0, "JOINTS_0": 2, "WEIGHTS_0": 3 },
      "indices": 1,
      "mode": 4
    }]
  }]
}"#
    .to_vec()
}

pub fn skinned_triangle_without_inverse_bind_matrices_gltf() -> Vec<u8> {
    String::from_utf8(skinned_triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(r#", "inverseBindMatrices": 4"#, "")
        .into_bytes()
}

pub fn skinned_triangle_bin() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(244);
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
    for joint in [
        0u16, 0, 0, 0, //
        1, 0, 0, 0, //
        0, 1, 0, 0,
    ] {
        bytes.extend_from_slice(&joint.to_le_bytes());
    }
    bytes.resize(68, 0);
    for weight in [
        1.0f32, 0.0, 0.0, 0.0, //
        1.0, 0.0, 0.0, 0.0, //
        0.5, 0.5, 0.0, 0.0,
    ] {
        bytes.extend_from_slice(&weight.to_le_bytes());
    }
    for _ in 0..2 {
        for value in baozi_core::Mat4::IDENTITY.cols.iter().flatten() {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes.resize(244, 0);
    bytes
}

pub fn skinned_triangle_bin_with_joint_index(index: u16) -> Vec<u8> {
    let mut bytes = skinned_triangle_bin();
    let offset = 42;
    bytes[offset..offset + 2].copy_from_slice(&index.to_le_bytes());
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
    let encoded = base64::engine::general_purpose::STANDARD.encode(triangle_bin());
    triangle_gltf_with_buffer_uri(
        &format!("data:application/octet-stream;base64,{encoded}"),
        104,
    )
}

pub fn triangle_gltf_with_buffer_uri(uri: &str, byte_length: usize) -> Vec<u8> {
    let text = String::from_utf8(triangle_gltf()).expect("fixture is valid utf-8");
    text.replace(
        r#""uri": "triangle.bin", "byteLength": 104"#,
        &format!(r#""uri": "{uri}", "byteLength": {byte_length}"#),
    )
    .into_bytes()
}
