#![allow(dead_code)]

use baozi_format_gltf::{FORMAT_ID, GltfFailureCode, GltfImporter};
use baozi_import::authoring::{FormatId, Mat4};
use baozi_import::{
    DenyDependencies, DetectionOptions, DetectionOutcome, FormatFailureDetailValue,
    FormatFailureEnvelope, FormatSelection, ImportCause, ImportConfigOverlay, ImportDiagnostic,
    ImportFailure, ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer,
    ImportResult, LossPolicy, OptionCompileLimits, PreparedImportRequest, RegistryBuildLimits,
    RegistryBuilder, ResourceLimits, ResourceLimitsOverlay, ScopeAbortCause, import_asset,
    import_asset_with_resolver,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use base64::Engine as _;
use std::error::Error;
use std::io;
use std::sync::Arc;

pub type TestResult<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Clone)]
pub struct GltfTestOptions {
    pub limits: ResourceLimits,
    pub loss_policy: LossPolicy,
    allow_dependencies: bool,
}

impl GltfTestOptions {
    pub fn memory() -> Self {
        Self {
            limits: ResourceLimits::default(),
            loss_policy: LossPolicy::preserve_or_fail(),
            allow_dependencies: false,
        }
    }
}

pub fn import_assets(
    source: &str,
    assets: impl IntoIterator<Item = (&'static str, Vec<u8>)>,
    options: GltfTestOptions,
) -> TestResult<(ImportReport, Vec<ImportDiagnostic>)> {
    let (result, diagnostics) = import_assets_result(source, assets, options)?;
    let report = result?;
    Ok((report, diagnostics))
}

pub fn import_assets_result(
    source: &str,
    assets: impl IntoIterator<Item = (&'static str, Vec<u8>)>,
    options: GltfTestOptions,
) -> TestResult<(ImportResult<ImportReport>, Vec<ImportDiagnostic>)> {
    import_assets_result_with_control(source, assets, options, ImportOperationControl::default())
}

pub fn import_assets_result_with_control(
    source: &str,
    assets: impl IntoIterator<Item = (&'static str, Vec<u8>)>,
    options: GltfTestOptions,
    control: ImportOperationControl,
) -> TestResult<(ImportResult<ImportReport>, Vec<ImportDiagnostic>)> {
    let source_path = AssetPath::new(source)?;
    let io = MemoryAssetIo::new();
    for (path, bytes) in assets {
        let _replaced = io.try_insert(AssetPath::new(path)?, Arc::<[u8]>::from(bytes))?;
    }
    let request = prepared_request(options.limits, options.loss_policy)?;
    let result = if options.allow_dependencies {
        import_asset(&request, &io, source_path, control)
    } else {
        import_asset_with_resolver(&request, &io, source_path, &DenyDependencies, control)
    };
    let diagnostics = match &result {
        Ok(report) => report.diagnostics().to_vec(),
        Err(error) => error
            .operation_failure()
            .map(|failure| failure.diagnostics().to_vec())
            .unwrap_or_default(),
    };
    Ok((result, diagnostics))
}

fn prepared_request(
    limits: ResourceLimits,
    loss_policy: LossPolicy,
) -> TestResult<PreparedImportRequest> {
    let mut registry = RegistryBuilder::new();
    registry.register(GltfImporter)?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    let format = FormatId::new(FORMAT_ID)?;
    Ok(snapshot.compile_request(
        &ImportRequest::from_layers(
            ImportRequestLayer::default(),
            ImportRequestLayer::new(
                ImportConfigOverlay::default()
                    .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
                    .with_loss_policy(loss_policy)
                    .with_format_selection(FormatSelection::Forced(format)),
            ),
        ),
        OptionCompileLimits::default(),
    )?)
}

pub fn detection_outcome(
    source: &str,
    bytes: Vec<u8>,
    max_probe_bytes: u64,
) -> TestResult<DetectionOutcome> {
    let source_path = AssetPath::new(source)?;
    let io = MemoryAssetIo::new();
    let _replaced = io.try_insert(source_path.clone(), Arc::<[u8]>::from(bytes))?;

    let mut registry = RegistryBuilder::new();
    registry.register(GltfImporter)?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    let request = snapshot.compile_request(
        &ImportRequest::from_layers(
            ImportRequestLayer::default(),
            ImportRequestLayer::new(
                ImportConfigOverlay::default()
                    .with_detection_options(DetectionOptions::new(max_probe_bytes)?),
            ),
        ),
        OptionCompileLimits::default(),
    )?;
    let result = import_asset(
        &request,
        &io,
        source_path,
        ImportOperationControl::default(),
    );
    let evidence = match &result {
        Ok(report) => report.detection_evidence(),
        Err(error) => error
            .operation_failure()
            .map(ImportFailure::detection_evidence)
            .unwrap_or_default(),
    };
    evidence
        .iter()
        .find(|entry| entry.format().as_str() == FORMAT_ID)
        .map(|entry| entry.outcome().clone())
        .ok_or_else(|| io::Error::other("glTF detection evidence was not retained").into())
}

pub fn expected_error(result: ImportResult<ImportReport>) -> TestResult<ImportFailure> {
    match result {
        Ok(_) => Err(io::Error::other("expected glTF import to fail").into()),
        Err(error) => Ok(error.into_operation_failure()?),
    }
}

pub fn sidecar_options() -> GltfTestOptions {
    let mut options = GltfTestOptions::memory();
    options.allow_dependencies = true;
    options
}

pub fn glb_with_json(json: &[u8]) -> Vec<u8> {
    let mut json = json.to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total_len = 20usize
        .checked_add(json.len())
        .expect("test GLB size must fit usize");
    let encoded_total_len = u32::try_from(total_len).expect("test GLB size must fit u32");
    let encoded_json_len = u32::try_from(json.len()).expect("test JSON chunk size must fit u32");
    let mut document = Vec::with_capacity(total_len);
    document.extend_from_slice(b"glTF");
    document.extend_from_slice(&2_u32.to_le_bytes());
    document.extend_from_slice(&encoded_total_len.to_le_bytes());
    document.extend_from_slice(&encoded_json_len.to_le_bytes());
    document.extend_from_slice(b"JSON");
    document.extend_from_slice(&json);
    document
}

pub fn append_glb_chunk(mut document: Vec<u8>, chunk_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let padded_len = payload
        .len()
        .checked_add(3)
        .expect("test GLB chunk size must fit usize")
        & !3;
    let encoded_len = u32::try_from(padded_len).expect("test GLB chunk size must fit u32");
    document.extend_from_slice(&encoded_len.to_le_bytes());
    document.extend_from_slice(chunk_type);
    document.extend_from_slice(payload);
    document.resize(document.len() + (padded_len - payload.len()), 0);
    let total_len = u32::try_from(document.len()).expect("test GLB size must fit u32");
    document[8..12].copy_from_slice(&total_len.to_le_bytes());
    document
}

pub fn gltf_failure(error: &ImportFailure) -> TestResult<&FormatFailureEnvelope> {
    match error.cause() {
        ImportCause::Format(envelope) if envelope.format().as_str() == FORMAT_ID => Ok(envelope),
        ImportCause::Format(envelope) => Err(io::Error::other(format!(
            "expected {FORMAT_ID} format failure, got {}",
            envelope.format()
        ))
        .into()),
        cause => Err(io::Error::other(format!("expected format failure, got {cause:?}")).into()),
    }
}

pub fn gltf_failure_reason(error: &ImportFailure) -> TestResult<&str> {
    let envelope = gltf_failure(error)?;
    envelope
        .details()
        .iter()
        .find_map(|detail| {
            if detail.key() != "reason" {
                return None;
            }
            match detail.value() {
                FormatFailureDetailValue::String(value) => Some(value.as_str()),
                _ => None,
            }
        })
        .ok_or_else(|| {
            io::Error::other("glTF failure did not preserve a string reason detail").into()
        })
}

pub fn assert_gltf_failure(
    error: &ImportFailure,
    expected_code: GltfFailureCode,
    expected_reason: Option<&str>,
) -> TestResult<()> {
    let envelope = gltf_failure(error)?;
    assert_eq!(envelope.code(), expected_code.as_str());
    if let Some(expected_reason) = expected_reason {
        let reason = gltf_failure_reason(error)?;
        assert!(
            reason.contains(expected_reason),
            "expected reason containing {expected_reason:?}, got {:?}",
            reason
        );
    }
    Ok(())
}

pub fn assert_resource_limit(error: &ImportFailure, expected: &str) {
    let ImportCause::Scope(abort) = error.cause() else {
        panic!(
            "expected scope-owned resource failure, got {:?}",
            error.cause()
        );
    };
    match abort.cause() {
        ScopeAbortCause::ResourceLimit { resource, .. } => {
            let expected_resource = match expected {
                "max_dependency_asset_bytes" | "max_primary_asset_bytes" => "asset-bytes",
                other => other,
            };
            assert_eq!(*resource, expected_resource);
        }
        ScopeAbortCause::Ledger(cause) => {
            let expected_dimension = match expected {
                "max_canonical_payload_bytes" => "CanonicalOutputBytes",
                "max_canonical_string_bytes" => "CanonicalStringBytes",
                "max_data_uri_bytes" => "DataUriBytes",
                "max_decode_work" => "DecodeWork",
                "max_animation_cells" => "AnimationCells",
                "max_dependency_asset_bytes" => "DependencyBytes",
                "max_faces" => "Faces",
                "max_graph_depth" => "GraphDepth",
                "max_graph_edges" => "GraphEdges",
                "max_graph_nodes" => "GraphNodes",
                "max_graph_work" => "GraphWork",
                "max_meshes" => "Meshes",
                "max_open_assets" => "AssetOpen",
                "max_primary_asset_bytes" => "PrimaryBytes",
                "max_primitives" => "Primitives",
                "source-scope-items" => "SourceScopeItemCount",
                "source-scope-digest-bytes" => "SourceScopeDigestBytes",
                "source-scope-detail-count" => "SourceScopeDetailCount",
                "source-scope-detail-bytes" => "SourceScopeDetailBytes",
                "max_live_bytes" => "ScratchBytes",
                "max_total_asset_bytes" => "TotalAssetBytes",
                "max_total_topology_cells" => "TopologyCells",
                "max_total_vertex_attribute_streams" => "AttributeStreams",
                "max_vertices" => "Vertices",
                other => other,
            };
            assert!(
                format!("{cause:?}").contains(expected_dimension),
                "expected {expected} ({expected_dimension}), got {cause:?}"
            );
        }
        cause => panic!("expected resource limit {expected}, got {cause:?}"),
    }
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

pub fn triangle_png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
        .expect("fixture PNG is valid base64")
}

pub fn interleaved_triangle_gltf() -> Vec<u8> {
    br#"{
  "asset": { "version": "2.0", "generator": "baozi-interleaved-test" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [{ "name": "InterleavedTriangleNode", "mesh": 0 }],
  "buffers": [{ "uri": "interleaved.bin", "byteLength": 80 }],
  "bufferViews": [
    { "buffer": 0, "byteOffset": 0, "byteLength": 72, "byteStride": 24, "target": 34962 },
    { "buffer": 0, "byteOffset": 72, "byteLength": 6, "target": 34963 }
  ],
  "accessors": [
    { "bufferView": 0, "byteOffset": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] },
    { "bufferView": 0, "byteOffset": 12, "componentType": 5126, "count": 3, "type": "VEC3" },
    { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
  ],
  "meshes": [{
    "name": "InterleavedTriangle",
    "primitives": [{
      "attributes": { "POSITION": 0, "NORMAL": 1 },
      "indices": 2,
      "mode": 4
    }]
  }]
}"#
    .to_vec()
}

pub fn interleaved_triangle_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    for vertex in [
        ([0.0f32, 0.0, 0.0], [0.0f32, 0.0, 1.0]),
        ([1.0f32, 0.0, 0.0], [0.0f32, 0.0, 1.0]),
        ([0.0f32, 1.0, 0.0], [0.0f32, 0.0, 1.0]),
    ] {
        for value in vertex.0 {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in vertex.1 {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes.resize(80, 0);
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
        for value in Mat4::IDENTITY.cols.iter().flatten() {
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
    let text = text
        .replace(
            "  \"images\": [{ \"uri\": \"textures/base.png\", \"name\": \"BaseImage\" }],\n",
            "",
        )
        .replace(
            "  \"samplers\": [{ \"wrapS\": 33071, \"wrapT\": 33648, \"magFilter\": 9729, \"minFilter\": 9987 }],\n",
            "",
        )
        .replace(
            "  \"textures\": [{ \"source\": 0, \"sampler\": 0, \"name\": \"BaseTex\" }],\n",
            "",
        )
        .replace(
            "      \"roughnessFactor\": 0.25,\n      \"baseColorTexture\": { \"index\": 0, \"texCoord\": 0 }\n",
            "      \"roughnessFactor\": 0.25\n",
        );
    text.replace(
        r#""uri": "triangle.bin", "byteLength": 104"#,
        &format!(r#""uri": "{uri}", "byteLength": {byte_length}"#),
    )
    .into_bytes()
}
