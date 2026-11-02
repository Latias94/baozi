#![forbid(unsafe_code)]

//! Direct single-format importer use, with caller-owned facts and detached import evidence.
//!
//! This consumer does not expose a raw glTF document or implement another scene model.

use baozi_format_gltf::{GltfImportOptions, GltfSceneSelection};
use baozi_import::{
    FormatOptionEnvelopeError, FormatSelection, ImportConfigOverlay, ImportEvidence,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestBuildError,
    ImportRequestLayer, ImportResult, LossPolicy, OptionCompileLimits, PreparedImportRequest,
    RegistryBuildError, RegistryBuildLimits, RegistryBuilder, RegistrySnapshot, RequestError,
    ValidatedScene,
};
use baozi_io::{AssetIo, AssetPath};
use std::fmt;

/// Application facts survive destruction of the Scene; evidence retains its original Baozi types.
#[derive(Debug)]
pub struct Observation<T> {
    pub facts: T,
    pub evidence: ImportEvidence,
}

/// Registers only the public glTF plugin and freezes its options before any import IO.
pub fn register() -> Result<RegistrySnapshot, RegistryBuildError> {
    let mut registry = RegistryBuilder::new();
    baozi_format_gltf::register(&mut registry).map_err(RegistryBuildError::Registration)?;
    registry.freeze(RegistryBuildLimits::default())
}

/// Compiles explicit scene selection with glTF forced and no authorized semantic losses.
pub fn compile(
    registry: &RegistrySnapshot,
    selection: GltfSceneSelection,
) -> Result<PreparedImportRequest, CompileError> {
    let options = GltfImportOptions::default()
        .with_scene_selection(selection)
        .to_envelope()
        .map_err(CompileError::Options)?;
    let caller = ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_format_selection(FormatSelection::Forced(
                baozi_format_gltf::format_descriptor().id().clone(),
            ))
            .with_loss_policy(LossPolicy::preserve_or_fail()),
    )
    .try_with_format_option(options)
    .map_err(CompileError::Layer)?;
    registry
        .compile_request(
            &ImportRequest::from_layers(ImportRequestLayer::default(), caller),
            OptionCompileLimits::default(),
        )
        .map_err(CompileError::Request)
}

/// Imports embedded bytes without sidecar authority, then detaches caller-selected facts.
#[allow(
    clippy::result_large_err,
    reason = "preserve the public import error and its complete typed terminal evidence"
)]
pub fn import_embedded<T>(
    request: &PreparedImportRequest,
    source: AssetPath,
    bytes: &[u8],
    read: impl FnOnce(&ValidatedScene) -> T,
) -> ImportResult<Observation<T>> {
    baozi_import::import_bytes(request, source, bytes, ImportOperationControl::default())
        .map(|report| detach(report, read))
}

/// Imports through the supplied asset authority, including selected external buffers.
#[allow(
    clippy::result_large_err,
    reason = "preserve the public import error and its complete typed terminal evidence"
)]
pub fn import_from_io<T>(
    request: &PreparedImportRequest,
    io: &dyn AssetIo,
    source: AssetPath,
    read: impl FnOnce(&ValidatedScene) -> T,
) -> ImportResult<Observation<T>> {
    baozi_import::import_asset(request, io, source, ImportOperationControl::default())
        .map(|report| detach(report, read))
}

fn detach<T>(report: ImportReport, read: impl FnOnce(&ValidatedScene) -> T) -> Observation<T> {
    let facts = read(report.scene());
    let (scene, evidence) = report.into_parts();
    drop(scene);
    Observation { facts, evidence }
}

/// Request preparation failures remain distinguishable from import operation failures.
#[derive(Debug)]
pub enum CompileError {
    Options(FormatOptionEnvelopeError),
    Layer(ImportRequestBuildError),
    Request(RequestError),
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Options(error) => error.fmt(formatter),
            Self::Layer(error) => error.fmt(formatter),
            Self::Request(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Options(error) => error,
            Self::Layer(error) => error,
            Self::Request(error) => error,
        })
    }
}

/// Returns the exact original assets used by the selected-profile tests.
///
/// The fixed list includes all buffers for whole-document validation. Import tests deliberately
/// withhold the unselected buffer. This exports fixture bytes, not a public document API.
/// No reference implementation, JSON codec, image decoder, or resource resolver creates them.
pub fn selected_fixtures() -> [(&'static str, Vec<u8>); 6] {
    let data = SELECTED_JSON.replacen(
        r#""uri": "selected.bin""#,
        concat!(r#""uri": "data:application/octet-stream;base64,"#, "paWlpaWlpaUAAACAAACgPwAAIMAAQID/AADwQQAAoMEAACBB/wB//wAA6MAAAAhBAAAcQQPIEf8CAAAAAQABAAAAQMAAAIBAAACgQAD/AABAgAAA/wAAAFo=", r#"""#),
        1,
    );
    let glb_json = SELECTED_JSON.replacen(r#""uri": "selected.bin", "#, "", 1);
    let no_default = data.replacen(r#""scene": 0"#, r#""extras": {}"#, 1);
    let other = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    [
        ("selected.glb", fixed_glb(glb_json.as_bytes())),
        ("selected-data.gltf", data.into_bytes()),
        ("selected-external.gltf", SELECTED_JSON.as_bytes().to_vec()),
        ("no-default.gltf", no_default.into_bytes()),
        ("selected.bin", SELECTED_BIN.to_vec()),
        ("omitted.bin", other),
    ]
}

fn fixed_glb(json: &[u8]) -> Vec<u8> {
    let mut json = json.to_vec();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut bin = SELECTED_BIN.to_vec();
    bin.resize(bin.len().next_multiple_of(4), 0);
    // All lengths come from these fixed, small constants, never caller-controlled input.
    let json_len = json.len() as u32;
    let bin_len = bin.len() as u32;
    let mut bytes = Vec::new();
    for word in [
        0x4654_6c67_u32,
        2,
        28 + json_len + bin_len,
        json_len,
        0x4e4f_534a,
    ] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(&bin_len.to_le_bytes());
    bytes.extend_from_slice(&0x004e_4942_u32.to_le_bytes());
    bytes.extend_from_slice(&bin);
    bytes
}

// Interleaved POSITION/COLOR, indices, sparse index/value, strided UV, logical tail.
const SELECTED_BIN: &[u8] = &[
    0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0xa0, 0x3f,
    0x00, 0x00, 0x20, 0xc0, 0x00, 0x40, 0x80, 0xff, 0x00, 0x00, 0xf0, 0x41, 0x00, 0x00, 0xa0, 0xc1,
    0x00, 0x00, 0x20, 0x41, 0xff, 0x00, 0x7f, 0xff, 0x00, 0x00, 0xe8, 0xc0, 0x00, 0x00, 0x08, 0x41,
    0x00, 0x00, 0x1c, 0x41, 0x03, 0xc8, 0x11, 0xff, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x40, 0xc0, 0x00, 0x00, 0x80, 0x40, 0x00, 0x00, 0xa0, 0x40, 0x00, 0xff, 0x00, 0x00,
    0x40, 0x80, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x5a,
];

const SELECTED_JSON: &str = r#"{
"asset": {"version": "2.0", "generator": "Baozi original selected-profile probe"},
"extensionsUsed": ["KHR_materials_emissive_strength", "KHR_materials_unlit",
    "KHR_texture_transform", "KHR_lights_punctual"],
"extensions": {"KHR_lights_punctual": {"lights": [
    {"type": "spot", "name": "Red spot", "color": [1, 0, 0], "intensity": 10000,
        "range": 8, "spot": {"innerConeAngle": 0.125, "outerConeAngle": 0.5}},
    {"type": "directional", "name": "Unselected light"}
]}},
"buffers": [{"uri": "selected.bin", "byteLength": 89}, {"byteLength": 36, "uri": "omitted.bin"}],
"bufferViews": [
    {"buffer": 0, "byteOffset": 8, "byteLength": 48, "byteStride": 16, "target": 34962},
    {"buffer": 0, "byteOffset": 56, "byteLength": 6, "target": 34963},
    {"buffer": 0, "byteOffset": 62, "byteLength": 1},
    {"buffer": 0, "byteOffset": 64, "byteLength": 12},
    {"buffer": 0, "byteOffset": 76, "byteLength": 12, "byteStride": 4, "target": 34962},
    {"buffer": 1, "byteLength": 36, "target": 34962}
],
"accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
        "min": [-7.25, 1.25, -2.5], "max": [0, 8.5, 9.75],
        "sparse": {"count": 1, "indices": {"bufferView": 2, "componentType": 5121},
            "values": {"bufferView": 3}}},
    {"bufferView": 0, "byteOffset": 12, "componentType": 5121, "normalized": true,
        "count": 3, "type": "VEC4"},
    {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"},
    {"bufferView": 4, "componentType": 5121, "normalized": true, "count": 3, "type": "VEC2"},
    {"bufferView": 5, "componentType": 5126, "count": 3, "type": "VEC3",
        "min": [0, 0, 0], "max": [1, 1, 0]}
],
"images": [{"uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQ1/T/DwACeQGP2F+lXgAAAABJRU5ErkJggg=="}],
"samplers": [{"magFilter": 9728, "minFilter": 9728, "wrapS": 33071, "wrapT": 33648}],
"textures": [{"source": 0, "sampler": 0}],
"materials": [
    {"name": "Radiant", "emissiveFactor": [0.125, 0.5, 0.75],
        "extensions": {"KHR_materials_emissive_strength": {"emissiveStrength": 8}},
        "pbrMetallicRoughness": {"baseColorFactor": [0.25, 0.5, 0.75, 1],
            "metallicFactor": 0.25, "roughnessFactor": 0.625,
            "baseColorTexture": {"index": 0, "texCoord": 1,
                "extensions": {"KHR_texture_transform": {
                    "offset": [0.25, -0.5], "rotation": 0, "scale": [2, 0.5], "texCoord": 0}}}}},
    {"name": "Ink", "extensions": {"KHR_materials_unlit": {}},
        "alphaMode": "BLEND",
        "pbrMetallicRoughness": {"baseColorFactor": [0.75, 0.25, 0.125, 0.5]}}
],
"meshes": [
    {"name": "Selected mesh", "primitives": [
        {"attributes": {"POSITION": 0, "COLOR_0": 1, "TEXCOORD_0": 3, "TEXCOORD_1": 3},
            "indices": 2, "material": 0},
        {"attributes": {"POSITION": 0, "COLOR_0": 1, "TEXCOORD_0": 3, "TEXCOORD_1": 3},
            "indices": 2, "material": 1}
    ]},
    {"name": "Other mesh", "primitives": [{"attributes": {"POSITION": 4}}]}
],
"cameras": [
    {"name": "Wide", "type": "perspective",
        "perspective": {"yfov": 0.75, "znear": 0.125, "zfar": 96, "aspectRatio": 1.5}},
    {"name": "Ortho", "type": "orthographic",
        "orthographic": {"xmag": 2, "ymag": 3, "znear": 0, "zfar": 32}},
    {"type": "perspective", "perspective": {"yfov": 1, "znear": 0.25}}
],
"nodes": [
    {"name": "Original", "mesh": 0, "extras": {"purpose": "owned-consumer", "ordinal": 7}},
    {"name": "Instance", "mesh": 0, "translation": [2, 0, 0]},
    {"name": "Wide node", "camera": 0},
    {"name": "Spot A", "extensions": {"KHR_lights_punctual": {"light": 0}}},
    {"name": "Spot B", "extensions": {"KHR_lights_punctual": {"light": 0}}},
    {"name": "Ortho node", "camera": 1},
    {"name": "Other node", "mesh": 1},
    {"name": "Infinite node", "camera": 2}
],
"scenes": [{"name": "Chosen", "nodes": [0, 1, 2, 3, 4, 5, 7]},
    {"name": "Other", "nodes": [6]}],
"scene": 0
}"#;
