use crate::allocation::LiveStorage;
use crate::{GltfFailure, external_buffer_role};
use baozi_core::scene::{Axis, Handedness};
use baozi_core::{
    Aabb, AlphaMode, Camera, CameraProjection, Color, ColorSpace, Mat4, Material, Mesh,
    MeshBinding, MetadataMap, Node, PrimitiveTopology, SceneDraft, SceneSpace, Skin, Texture,
    TextureFilterMode, TextureRole, TextureSampler, TextureSlot, TextureSource, TextureTransform,
    TextureWrapMode, Vec2, Vec3, Vec4,
};
use baozi_import::{
    DiagnosticCode, DiagnosticSeverity, ImportStorageClass, ImportWorkClass, ParseFailure,
    ParseResult, ParseScope, ScopedSceneBuilder,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::{DecodeError, Engine as _};
use gltf::buffer::Source as BufferSource;
use gltf::image::Source as ImageSource;
use gltf::json;
use gltf::json::validation::Checked;
use gltf::mesh::Mode;
use gltf::texture::{MagFilter, MinFilter, WrappingMode};
use std::fmt;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};

type Result<T> = std::result::Result<T, ContentError>;

const GLTF_BACKEND_FIXED_ALLOWANCE: u64 = 64 * 1024;
const GLTF_JSON_EXPANSION_ALLOWANCE: u64 = 64;
const GLB_HEADER_BYTES: usize = 12;
const GLB_CHUNK_HEADER_BYTES: usize = 8;
const GLB_JSON_OFFSET: usize = GLB_HEADER_BYTES + GLB_CHUNK_HEADER_BYTES;
const PARSER_POLL_INTERVAL: usize = 4_096;

#[derive(Debug)]
enum ContentError {
    Scope(ParseFailure),
    Format(GltfFailure),
    Parse {
        message: String,
    },
    FeatureUnsupported {
        format: &'static str,
        feature: String,
    },
    InvalidScene {
        message: String,
    },
}

impl ContentError {
    fn parse(
        _source: impl fmt::Display,
        _location: Option<()>,
        message: impl Into<String>,
    ) -> Self {
        Self::Parse {
            message: message.into(),
        }
    }

    fn into_format_failure(self) -> GltfFailure {
        match self {
            Self::Scope(_) => GltfFailure::internal("scope failure escaped its ownership boundary"),
            Self::Format(failure) => failure,
            Self::Parse { message } => GltfFailure::invalid_document(message),
            Self::FeatureUnsupported { feature, .. } => {
                GltfFailure::unsupported("feature", feature)
            }
            Self::InvalidScene { message } => GltfFailure::internal(message),
        }
    }
}

impl fmt::Display for ContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scope(error) => error.fmt(formatter),
            Self::Format(error) => error.fmt(formatter),
            Self::Parse { message } | Self::InvalidScene { message } => {
                formatter.write_str(message)
            }
            Self::FeatureUnsupported { format, feature } => {
                write!(formatter, "{format} feature is unsupported: {feature}")
            }
        }
    }
}

impl From<ParseFailure> for ContentError {
    fn from(value: ParseFailure) -> Self {
        Self::Scope(value)
    }
}

impl From<GltfFailure> for ContentError {
    fn from(value: GltfFailure) -> Self {
        Self::Format(value)
    }
}

pub(crate) fn read_gltf(scope: &ParseScope<'_>) -> ParseResult<SceneDraft> {
    match read_gltf_inner(scope) {
        Ok(draft) => Ok(draft),
        Err(ContentError::Scope(error)) => Err(error),
        Err(error) => Err(scope.format_failure(error.into_format_failure())),
    }
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn enforce_usize_limit(
    ctx: &ParseScope<'_>,
    resource: &'static str,
    attempted: usize,
    limit: usize,
) -> Result<()> {
    ctx.enforce_resource_limit(resource, to_u64(attempted), to_u64(limit))?;
    Ok(())
}

fn checked_add_with_limit(
    ctx: &ParseScope<'_>,
    resource: &'static str,
    current: usize,
    amount: usize,
    limit: usize,
) -> Result<usize> {
    let attempted = current
        .checked_add(amount)
        .ok_or_else(|| ctx.allocation_failure("gltf-resource-counter-overflow"))?;
    enforce_usize_limit(ctx, resource, attempted, limit)?;
    Ok(attempted)
}

fn read_gltf_inner(ctx: &ParseScope<'_>) -> Result<SceneDraft> {
    let bytes = ctx.primary_bytes();
    let primary_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let decode_work = ctx.begin_work(ImportWorkClass::DecodeWork, primary_len)?;
    let glb_candidate = bytes.get(..4) == Some(b"glTF");
    if glb_candidate {
        if let Some(json) = glb_json_surface(bytes) {
            preflight_json_utf8(ctx, json, GLB_JSON_OFFSET as u64)?;
        }
    } else {
        preflight_json_utf8(ctx, bytes, 0)?;
    }
    let backend_allowance = gltf_backend_allocation_allowance(ctx, bytes)?;
    let _document_reservation =
        ctx.reserve_live(ImportStorageClass::BackendDecode, backend_allowance)?;
    let parsed = safe_gltf(ctx, "glTF document parse", || gltf::Gltf::from_slice(bytes))?;
    decode_work.complete()?;
    let gltf = parsed
        .map_err(|error| GltfFailure::invalid_document(format_args!("invalid glTF: {error}")))?;
    let node_graph = preflight_node_graph(ctx, &gltf)?;
    let repeated_mesh_string_bytes = preflight_canonical_base_strings(ctx, &gltf, &node_graph)?;
    let buffers = load_buffers(ctx, &gltf)?;
    scene_from_document(
        ctx,
        &gltf,
        buffers.as_slice(),
        &node_graph,
        repeated_mesh_string_bytes,
    )
}

fn preflight_json_utf8(ctx: &ParseScope<'_>, bytes: &[u8], raw_base: u64) -> Result<()> {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(GltfFailure::forbidden_bom(raw_base).into());
    }

    let mut window_start = 0usize;
    while window_start < bytes.len() {
        ctx.poll()?;
        let window_end = window_start
            .saturating_add(PARSER_POLL_INTERVAL)
            .min(bytes.len());
        let Err(source) = std::str::from_utf8(&bytes[window_start..window_end]) else {
            window_start = window_end;
            continue;
        };
        let invalid_start = window_start
            .checked_add(source.valid_up_to())
            .ok_or_else(|| ctx.allocation_failure("gltf-json-byte-offset"))?;
        if source.error_len().is_none() && window_end != bytes.len() {
            debug_assert!(window_end.saturating_sub(invalid_start) <= 3);
            window_start = invalid_start;
            continue;
        }
        let relative_offset = u64::try_from(invalid_start)
            .map_err(|_| ctx.allocation_failure("gltf-json-byte-offset"))?;
        let byte_offset = raw_base
            .checked_add(relative_offset)
            .ok_or_else(|| ctx.allocation_failure("gltf-json-byte-offset"))?;
        return Err(GltfFailure::invalid_utf8(source, byte_offset).into());
    }

    ctx.enforce_resource_limit(
        "gltf-decoded-utf8-bytes",
        to_u64(bytes.len()),
        ctx.limits().max_decoded_utf8_bytes,
    )?;
    Ok(())
}

fn glb_json_surface(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.get(..4) != Some(b"glTF") || !bytes.len().is_multiple_of(4) {
        return None;
    }
    if read_glb_u32(bytes, 4)? != 2 {
        return None;
    }
    let declared_length = usize::try_from(read_glb_u32(bytes, 8)?).ok()?;
    if declared_length != bytes.len() || declared_length < GLB_JSON_OFFSET {
        return None;
    }
    let chunk_length = usize::try_from(read_glb_u32(bytes, GLB_HEADER_BYTES)?).ok()?;
    if chunk_length % 4 != 0 || bytes.get(GLB_HEADER_BYTES + 4..GLB_JSON_OFFSET) != Some(b"JSON") {
        return None;
    }
    let chunk_end = GLB_JSON_OFFSET.checked_add(chunk_length)?;
    if chunk_end > declared_length {
        return None;
    }
    bytes.get(GLB_JSON_OFFSET..chunk_end)
}

fn read_glb_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(size_of::<u32>())?;
    let encoded: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(encoded))
}

fn gltf_backend_allocation_allowance(ctx: &ParseScope<'_>, bytes: &[u8]) -> Result<u64> {
    let input_bytes = u64::try_from(bytes.len())
        .map_err(|_| ctx.allocation_failure("gltf-backend-input-bytes"))?;
    let json_bytes = glb_json_surface(bytes)
        .map(|json| u64::try_from(json.len()).unwrap_or(u64::MAX))
        .unwrap_or(input_bytes);

    // The gltf/serde object graph is opaque to the import ledger. Retain the
    // input-sized payload allowance plus a declared structural expansion cap.
    json_bytes
        .checked_mul(GLTF_JSON_EXPANSION_ALLOWANCE)
        .and_then(|expanded| expanded.checked_add(input_bytes))
        .and_then(|expanded| expanded.checked_add(GLTF_BACKEND_FIXED_ALLOWANCE))
        .ok_or_else(|| {
            ctx.allocation_failure("gltf-backend-allocation-allowance")
                .into()
        })
}

fn preflight_canonical_base_strings(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Gltf,
    graph: &NodeGraph<'_, '_>,
) -> Result<usize> {
    safe_gltf(ctx, "glTF canonical string preflight", || {
        let mut bytes = 0usize;
        let limit = ctx.limits().max_canonical_string_bytes;

        for (mesh_index, mesh) in gltf.meshes().enumerate() {
            poll_parser_loop(ctx, mesh_index)?;
            if let Some(name) = mesh.name() {
                for (primitive_index, _) in mesh.primitives().enumerate() {
                    poll_parser_loop(ctx, primitive_index)?;
                    let generated_len = primitive_name_len(ctx, name, primitive_index)?;
                    add_repeated_string_bytes(ctx, &mut bytes, generated_len, limit)?;
                }
            }
        }

        let mut storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
        let mut visited = try_vec_filled(
            ctx,
            &mut storage,
            graph.nodes.len(),
            false,
            "gltf-canonical-string-node-membership",
        )?;
        let skin_count = gltf.skins().len();
        let mut referenced_skins = try_vec_filled(
            ctx,
            &mut storage,
            skin_count,
            false,
            "gltf-canonical-string-skin-membership",
        )?;
        let mut pending = try_vec_capacity(
            ctx,
            &mut storage,
            graph.nodes.len(),
            "gltf-canonical-string-node-stack",
        )?;
        for (root_ordinal, root) in graph.roots.iter().rev().copied().enumerate() {
            poll_parser_loop(ctx, root_ordinal)?;
            pending.push(root);
        }
        let mut selected_count = 0usize;
        while let Some(node_index) = pending.pop() {
            poll_parser_loop(ctx, selected_count)?;
            selected_count += 1;
            let Some(node) = graph.nodes.get(node_index) else {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF node graph references missing node {node_index}"),
                ));
            };
            let Some(seen) = visited.get_mut(node_index) else {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF node graph references missing node {node_index}"),
                ));
            };
            if *seen {
                continue;
            }
            *seen = true;
            if let Some(name) = node.name() {
                enforce_usize_limit(
                    ctx,
                    "max_string_bytes",
                    name.len(),
                    ctx.limits().max_string_bytes,
                )?;
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
            if let Some(skin) = node.skin() {
                let skin_index = skin.index();
                let Some(referenced) = referenced_skins.get_mut(skin_index) else {
                    return Err(gltf_parse_error(
                        ctx,
                        format!("glTF node {node_index} references skin {skin_index} out of range"),
                    ));
                };
                *referenced = true;
            }
            for (child_ordinal, child) in
                graph.children[node_index].iter().rev().copied().enumerate()
            {
                poll_parser_loop(ctx, child_ordinal)?;
                pending.push(child);
            }
        }

        for (skin_index, skin) in gltf.skins().enumerate() {
            poll_parser_loop(ctx, skin_index)?;
            if referenced_skins.get(skin_index).copied() != Some(true) {
                continue;
            }
            if let Some(name) = skin.name() {
                enforce_usize_limit(
                    ctx,
                    "max_string_bytes",
                    name.len(),
                    ctx.limits().max_string_bytes,
                )?;
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
        }

        for (camera_index, camera) in gltf.cameras().enumerate() {
            poll_parser_loop(ctx, camera_index)?;
            if let Some(name) = camera.name()
                && name.len() <= ctx.limits().max_string_bytes
            {
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
        }

        Ok(bytes)
    })?
}

struct ResolvedTextureUris<'scope> {
    values: Vec<Option<String>>,
    _storage: LiveStorage<'scope>,
}

impl ResolvedTextureUris<'_> {
    fn get(&self, image_index: &usize) -> Option<&String> {
        self.values.get(*image_index)?.as_ref()
    }
}

fn preflight_texture_strings<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Gltf,
    repeated_mesh_string_bytes: usize,
) -> Result<ResolvedTextureUris<'scope>> {
    safe_gltf(ctx, "glTF texture string preflight", || {
        let mut bytes = repeated_mesh_string_bytes;
        let limit = ctx.limits().max_canonical_string_bytes;
        let mut storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
        let image_count = safe_gltf(ctx, "glTF image count", || gltf.images().len())?;
        let mut resolved_uris = try_vec_filled(
            ctx,
            &mut storage,
            image_count,
            None,
            "gltf-resolved-texture-uris",
        )?;

        for (material_index, material) in gltf.materials().enumerate() {
            poll_parser_loop(ctx, material_index)?;
            if let Some(name) = material.name() {
                enforce_usize_limit(
                    ctx,
                    "max_string_bytes",
                    name.len(),
                    ctx.limits().max_string_bytes,
                )?;
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
            let pbr = material.pbr_metallic_roughness();
            if let Some(info) = pbr.base_color_texture() {
                preflight_texture_reference(
                    ctx,
                    &mut bytes,
                    limit,
                    info.texture(),
                    "baseColorTexture",
                    &mut resolved_uris,
                    &mut storage,
                )?;
            }
            if let Some(info) = pbr.metallic_roughness_texture() {
                preflight_texture_reference(
                    ctx,
                    &mut bytes,
                    limit,
                    info.texture(),
                    "metallicRoughnessTexture",
                    &mut resolved_uris,
                    &mut storage,
                )?;
            }
            if let Some(info) = material.normal_texture() {
                preflight_texture_reference(
                    ctx,
                    &mut bytes,
                    limit,
                    info.texture(),
                    "normalTexture",
                    &mut resolved_uris,
                    &mut storage,
                )?;
            }
            if let Some(info) = material.occlusion_texture() {
                preflight_texture_reference(
                    ctx,
                    &mut bytes,
                    limit,
                    info.texture(),
                    "occlusionTexture",
                    &mut resolved_uris,
                    &mut storage,
                )?;
            }
            if let Some(info) = material.emissive_texture() {
                preflight_texture_reference(
                    ctx,
                    &mut bytes,
                    limit,
                    info.texture(),
                    "emissiveTexture",
                    &mut resolved_uris,
                    &mut storage,
                )?;
            }
        }

        let canonical_strings =
            ctx.begin_work(ImportWorkClass::CanonicalStringBytes, to_u64(bytes))?;
        canonical_strings.complete()?;
        Ok(ResolvedTextureUris {
            values: resolved_uris,
            _storage: storage,
        })
    })?
}

fn preflight_texture_reference<'scope>(
    ctx: &'scope ParseScope<'_>,
    bytes: &mut usize,
    limit: usize,
    texture: gltf::texture::Texture<'_>,
    source_key: &'static str,
    resolved_uris: &mut [Option<String>],
    storage: &mut LiveStorage<'scope>,
) -> Result<()> {
    let image = texture.source();
    let image_name = image.name();
    if let Some(name) = image_name {
        enforce_usize_limit(
            ctx,
            "max_string_bytes",
            name.len(),
            ctx.limits().max_string_bytes,
        )?;
    }

    let ImageSource::Uri { uri, .. } = image.source() else {
        return Ok(());
    };
    if is_data_uri(uri) || uri.len() > ctx.limits().max_string_bytes {
        return Ok(());
    }

    let image_index = image.index();
    let slot = resolved_uris.get_mut(image_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format!("glTF texture references missing image {image_index}"),
        )
    })?;
    if slot.is_none() {
        let mut resolution_storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
        resolution_storage.reserve_bytes(ctx, texture_path_resolution_allowance(ctx, uri)?)?;
        let resolution_work = ctx.begin_work(
            ImportWorkClass::WorkUnits,
            to_u64(ctx.source().as_str().len().saturating_add(uri.len())),
        )?;
        let resolved = ctx.source().resolve_from_file(uri).map_err(|error| {
            GltfFailure::invalid_reference("texture-uri", Some(image_index), error)
        })?;
        resolution_work.complete()?;
        ctx.poll()?;
        let resolved = storage.copy_string(ctx, resolved.as_str(), "gltf-resolved-texture-uri")?;
        *slot = Some(resolved);
    }
    let resolved_uri = slot.as_ref().ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format!("glTF texture resolution plan is missing image {image_index}"),
        )
    })?;

    let texture_name = texture.name();
    if let Some(name) = texture_name {
        enforce_usize_limit(
            ctx,
            "max_string_bytes",
            name.len(),
            ctx.limits().max_string_bytes,
        )?;
    }
    if let Some(name) = texture_name.or(image_name) {
        add_repeated_string_bytes(ctx, bytes, name.len(), limit)?;
    }
    add_repeated_string_bytes(ctx, bytes, resolved_uri.len(), limit)?;
    add_repeated_string_bytes(ctx, bytes, source_key.len(), limit)?;
    Ok(())
}

fn texture_path_resolution_allowance(ctx: &ParseScope<'_>, reference: &str) -> Result<u64> {
    let source_bytes = ctx
        .source()
        .as_str()
        .len()
        .checked_add(reference.len())
        .ok_or_else(|| ctx.allocation_failure("gltf-texture-path-resolution"))?;
    let component_count = ctx
        .source()
        .as_str()
        .split('/')
        .count()
        .checked_add(reference.split(['/', '\\']).count())
        .ok_or_else(|| ctx.allocation_failure("gltf-texture-path-resolution"))?;
    let component_storage = size_of::<String>()
        .checked_add(size_of::<&str>())
        .and_then(|bytes| bytes.checked_mul(component_count))
        .ok_or_else(|| ctx.allocation_failure("gltf-texture-path-resolution"))?;
    let allowance = source_bytes
        .checked_mul(6)
        .and_then(|bytes| bytes.checked_add(component_storage))
        .ok_or_else(|| ctx.allocation_failure("gltf-texture-path-resolution"))?;
    Ok(to_u64(allowance))
}

fn add_repeated_string_bytes(
    ctx: &ParseScope<'_>,
    bytes: &mut usize,
    amount: usize,
    limit: usize,
) -> Result<()> {
    *bytes = checked_add_with_limit(ctx, "max_canonical_string_bytes", *bytes, amount, limit)?;
    Ok(())
}

struct BufferData<'scope> {
    bytes: Vec<u8>,
    _storage: LiveStorage<'scope>,
}

impl BufferData<'_> {
    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

struct BufferSet<'scope> {
    values: Vec<BufferData<'scope>>,
    _storage: LiveStorage<'scope>,
}

impl BufferSet<'_> {
    fn as_slice(&self) -> &[BufferData<'_>] {
        &self.values
    }
}

fn load_buffers<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Gltf,
) -> Result<BufferSet<'scope>> {
    let buffer_count = safe_gltf(ctx, "glTF buffer count", || gltf.buffers().count())?;
    let mut storage = LiveStorage::new(ImportStorageClass::BackendDecode);
    let mut buffers = Vec::new();
    storage.reserve_vec(ctx, &mut buffers, buffer_count, "gltf-buffer-list")?;
    for (ordinal, buffer) in gltf.buffers().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        let mut loaded = match buffer.source() {
            BufferSource::Bin => {
                let blob = gltf.blob.as_deref().ok_or_else(|| {
                    ContentError::parse(
                        ctx.source().to_string(),
                        None,
                        format!("glTF buffer {} requires a GLB BIN chunk", buffer.index()),
                    )
                })?;
                copy_buffer(ctx, blob, "gltf-glb-buffer")?
            }
            BufferSource::Uri(uri) if is_data_uri(uri) => {
                load_data_uri_buffer(ctx, uri, buffer.index())?
            }
            BufferSource::Uri(uri) => load_external_buffer(ctx, uri, buffer.index())?,
        };

        if loaded.bytes.len() < buffer.length() {
            return Err(ContentError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF buffer {} declares {} bytes but only {} bytes are available",
                    buffer.index(),
                    buffer.length(),
                    loaded.bytes.len()
                ),
            ));
        }
        loaded.bytes.truncate(buffer.length());
        buffers.push(loaded);
    }
    Ok(BufferSet {
        values: buffers,
        _storage: storage,
    })
}

fn load_data_uri_buffer<'scope>(
    ctx: &'scope ParseScope<'_>,
    uri: &str,
    buffer_index: usize,
) -> Result<BufferData<'scope>> {
    enforce_usize_limit(
        ctx,
        "max_string_bytes",
        uri.len(),
        ctx.limits().max_string_bytes,
    )?;
    let Some(payload) = uri.strip_prefix("data:") else {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} has an invalid data URI"),
        ));
    };
    let Some((metadata, encoded)) = payload.split_once(',') else {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} data URI has no comma separator"),
        ));
    };
    if !metadata
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        return Err(ContentError::FeatureUnsupported {
            format: "gltf",
            feature: format!("buffer {buffer_index} uses a non-base64 data URI"),
        });
    }

    let decoded_len = base64_decoded_len(ctx, encoded, buffer_index)?;
    let data_uri_work = ctx.begin_work(ImportWorkClass::DataUriBytes, decoded_len)?;
    let decode_work = ctx.begin_work(
        ImportWorkClass::DecodeWork,
        u64::try_from(encoded.len()).unwrap_or(u64::MAX),
    )?;
    let mut storage = LiveStorage::new(ImportStorageClass::BackendDecode);
    let decoded_capacity = base64::decoded_len_estimate(encoded.len());
    let mut decoded = Vec::new();
    storage.reserve_vec(ctx, &mut decoded, decoded_capacity, "gltf-data-uri-buffer")?;
    for (chunk_index, chunk) in encoded.as_bytes().chunks(PARSER_POLL_INTERVAL).enumerate() {
        ctx.poll()?;
        let chunk_start = chunk_index * PARSER_POLL_INTERVAL;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            BASE64_STANDARD.decode_vec(chunk, &mut decoded)
        }));
        finish_gltf_operation("glTF base64 buffer decode", outcome)?.map_err(|error| {
            let error = offset_base64_decode_error(error, chunk_start);
            GltfFailure::invalid_reference(
                "data-uri-buffer",
                Some(buffer_index),
                format_args!("invalid base64: {error}"),
            )
        })?;
    }
    data_uri_work.complete()?;
    decode_work.complete()?;
    Ok(BufferData {
        bytes: decoded,
        _storage: storage,
    })
}

fn base64_decoded_len(ctx: &ParseScope<'_>, encoded: &str, buffer_index: usize) -> Result<u64> {
    if encoded.is_empty() {
        return Ok(0);
    }

    let mut padding = 0usize;
    let mut padding_started = false;
    let mut invalid_padding = false;
    for chunk in encoded.as_bytes().chunks(PARSER_POLL_INTERVAL) {
        ctx.poll()?;
        for &byte in chunk {
            if byte.is_ascii_whitespace() {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF buffer {buffer_index} data URI base64 contains whitespace"),
                ));
            }
            if byte == b'=' {
                padding_started = true;
                padding = padding.saturating_add(1);
                invalid_padding |= padding > 2;
            } else if padding_started {
                invalid_padding = true;
            }
        }
    }
    if !encoded.len().is_multiple_of(4) {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} data URI base64 length is not padded"),
        ));
    }
    if invalid_padding {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} data URI base64 padding is invalid"),
        ));
    }
    let groups = (encoded.len() / 4) as u64;
    groups
        .checked_mul(3)
        .and_then(|bytes| bytes.checked_sub(padding as u64))
        .ok_or_else(|| ctx.allocation_failure("gltf-data-uri-size-overflow").into())
}

fn offset_base64_decode_error(error: DecodeError, chunk_start: usize) -> DecodeError {
    match error {
        DecodeError::InvalidByte(offset, byte) => {
            DecodeError::InvalidByte(chunk_start.saturating_add(offset), byte)
        }
        DecodeError::InvalidLength(length) => {
            DecodeError::InvalidLength(chunk_start.saturating_add(length))
        }
        DecodeError::InvalidLastSymbol(offset, byte) => {
            DecodeError::InvalidLastSymbol(chunk_start.saturating_add(offset), byte)
        }
        DecodeError::InvalidPadding => DecodeError::InvalidPadding,
    }
}

fn load_external_buffer<'scope>(
    ctx: &'scope ParseScope<'_>,
    uri: &str,
    _buffer_index: usize,
) -> Result<BufferData<'scope>> {
    enforce_usize_limit(
        ctx,
        "max_string_bytes",
        uri.len(),
        ctx.limits().max_string_bytes,
    )?;
    let snapshot = ctx.required_dependency(&external_buffer_role(), uri)?;
    copy_buffer(ctx, snapshot.bytes(), "gltf-external-buffer-copy")
}

fn copy_buffer<'scope>(
    ctx: &'scope ParseScope<'_>,
    source: &[u8],
    allocation: &'static str,
) -> Result<BufferData<'scope>> {
    let copy_work = ctx.begin_work(ImportWorkClass::DecodeWork, to_u64(source.len()))?;
    let mut storage = LiveStorage::new(ImportStorageClass::BackendDecode);
    let mut copied = Vec::new();
    storage.reserve_vec(ctx, &mut copied, source.len(), allocation)?;
    for (chunk_index, chunk) in source.chunks(PARSER_POLL_INTERVAL).enumerate() {
        poll_parser_loop(ctx, chunk_index * PARSER_POLL_INTERVAL)?;
        copied.extend_from_slice(chunk);
    }
    copy_work.complete()?;
    Ok(BufferData {
        bytes: copied,
        _storage: storage,
    })
}

fn gltf_parse_error(ctx: &ParseScope<'_>, message: impl Into<String>) -> ContentError {
    ContentError::parse(ctx.source().to_string(), None, message)
}

fn scene_from_document(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Gltf,
    buffers: &[BufferData<'_>],
    node_graph: &NodeGraph<'_, '_>,
    repeated_mesh_string_bytes: usize,
) -> Result<SceneDraft> {
    let resolved_texture_uris = preflight_texture_strings(ctx, gltf, repeated_mesh_string_bytes)?;
    let mut builder = ctx.scene_builder()?;
    let mut normalization_storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
    let material_count = safe_gltf(ctx, "glTF material count", || gltf.materials().count())?;
    let mut material_ids = try_vec_capacity(
        ctx,
        &mut normalization_storage,
        material_count,
        "gltf-material-id-map",
    )?;
    for (material_index, material) in gltf.materials().enumerate() {
        poll_parser_loop(ctx, material_index)?;
        material_ids.push(add_material(
            ctx,
            &mut builder,
            material,
            &resolved_texture_uris,
        )?);
    }

    let mesh_count = safe_gltf(ctx, "glTF mesh count", || gltf.meshes().count())?;
    let mut mesh_ids_by_gltf_mesh = try_vec_capacity(
        ctx,
        &mut normalization_storage,
        mesh_count,
        "gltf-mesh-id-map",
    )?;
    for (mesh_ordinal, mesh) in gltf.meshes().enumerate() {
        poll_parser_loop(ctx, mesh_ordinal)?;
        let primitive_count = safe_gltf(ctx, "glTF primitive count", || mesh.primitives().len())?;
        let mut primitive_meshes = try_vec_capacity(
            ctx,
            &mut normalization_storage,
            primitive_count,
            "gltf-primitive-mesh-id-map",
        )?;
        for (primitive_ordinal, primitive) in mesh.primitives().enumerate() {
            poll_parser_loop(ctx, primitive_ordinal)?;
            let mesh_id = add_primitive_mesh(
                ctx,
                &mut builder,
                gltf,
                &material_ids,
                buffers,
                &mesh,
                primitive,
            )?;
            primitive_meshes.push(mesh_id);
        }
        mesh_ids_by_gltf_mesh.push(primitive_meshes);
    }

    add_cameras(ctx, &mut builder, gltf)?;
    add_unsupported_domain_diagnostics(ctx, gltf)?;
    let node_imports = add_scene_nodes(
        ctx,
        &mut normalization_storage,
        &mut builder,
        node_graph,
        &mesh_ids_by_gltf_mesh,
    )?;
    let skin_ids = add_skins(
        ctx,
        &mut normalization_storage,
        &mut builder,
        gltf,
        buffers,
        &node_imports.node_ids,
        &node_imports.node_skins,
    )?;
    attach_node_skins(ctx, &mut builder, &node_imports.node_skins, &skin_ids)?;

    builder.set_space(SceneSpace {
        handedness: Handedness::Right,
        up_axis: Some(Axis::PositiveY),
        front_axis: Some(Axis::NegativeZ),
        unit_scale_to_meters: Some(1.0),
    })?;
    Ok(builder.finish_draft()?)
}

fn add_material(
    ctx: &ParseScope<'_>,
    builder: &mut ScopedSceneBuilder,
    material: gltf::Material<'_>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
) -> Result<baozi_core::MaterialId> {
    let pbr = material.pbr_metallic_roughness();
    let base = pbr.base_color_factor();
    let emissive = material.emissive_factor();
    let mut storage = LiveStorage::new(ImportStorageClass::SceneConstruction);

    let mut material_out = Material {
        name: optional_name(ctx, &mut storage, material.name())?,
        shading_model: baozi_core::ShadingModel::PbrMetallicRoughness,
        base_color: Color::linear_rgba(base[0], base[1], base[2], base[3]),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        emissive: Color::linear_rgba(emissive[0], emissive[1], emissive[2], 1.0),
        alpha_mode: map_alpha_mode(material.alpha_mode()),
        alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
        double_sided: material.double_sided(),
        ..Material::default()
    };

    if let Some(info) = pbr.base_color_texture() {
        add_texture_slot(
            ctx,
            builder,
            resolved_texture_uris,
            &mut storage,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::BaseColor,
            ColorSpace::Srgb,
            1.0,
            "baseColorTexture",
        )?;
    }
    if let Some(info) = pbr.metallic_roughness_texture() {
        add_texture_slot(
            ctx,
            builder,
            resolved_texture_uris,
            &mut storage,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::MetallicRoughness,
            ColorSpace::Data,
            1.0,
            "metallicRoughnessTexture",
        )?;
    }
    if let Some(info) = material.normal_texture() {
        add_texture_slot(
            ctx,
            builder,
            resolved_texture_uris,
            &mut storage,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::Normal,
            ColorSpace::Data,
            info.scale(),
            "normalTexture",
        )?;
    }
    if let Some(info) = material.occlusion_texture() {
        add_texture_slot(
            ctx,
            builder,
            resolved_texture_uris,
            &mut storage,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::Occlusion,
            ColorSpace::Data,
            info.strength(),
            "occlusionTexture",
        )?;
    }
    if let Some(info) = material.emissive_texture() {
        add_texture_slot(
            ctx,
            builder,
            resolved_texture_uris,
            &mut storage,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::Emissive,
            ColorSpace::Srgb,
            1.0,
            "emissiveTexture",
        )?;
    }

    let material_id = builder.add_material(material_out)?;
    storage.adopt(builder)?;
    Ok(material_id)
}

#[allow(clippy::too_many_arguments)]
fn add_texture_slot<'scope>(
    ctx: &'scope ParseScope<'_>,
    builder: &mut ScopedSceneBuilder,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    material_storage: &mut LiveStorage<'scope>,
    material: &mut Material,
    texture: gltf::texture::Texture<'_>,
    tex_coord: u32,
    role: TextureRole,
    color_space: ColorSpace,
    scale: f32,
    source_key: &'static str,
) -> Result<()> {
    let image = texture.source();
    let image_name = image.name();
    let mut texture_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let source = match image.source() {
        ImageSource::Uri { uri, .. } if is_data_uri(uri) => {
            push_warning(
                ctx,
                "gltf.texture_data_uri_ignored",
                format_args!("texture data URI for {source_key} was ignored"),
            )?;
            return Ok(());
        }
        ImageSource::Uri { uri, .. } => {
            if uri.len() > ctx.limits().max_string_bytes {
                push_warning(
                    ctx,
                    "gltf.texture_uri_limit",
                    format_args!(
                        "texture URI for {source_key} exceeded max_string_bytes and was ignored"
                    ),
                )?;
                return Ok(());
            }
            let image_index = image.index();
            let resolved = resolved_texture_uris.get(&image_index).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format!("glTF texture resolution plan is missing image {image_index}"),
                )
            })?;
            TextureSource::External {
                uri: texture_storage.copy_string(ctx, resolved, "gltf-texture-source-uri")?,
            }
        }
        ImageSource::View { mime_type, .. } => {
            push_warning(
                ctx,
                "gltf.embedded_texture_ignored",
                format_args!("embedded {mime_type} texture for {source_key} was ignored"),
            )?;
            return Ok(());
        }
    };

    let texture_id = builder.add_texture(Texture {
        name: optional_name(ctx, &mut texture_storage, texture.name().or(image_name))?,
        source,
        sampler: map_sampler(texture.sampler()),
        metadata: MetadataMap::new(),
    })?;
    texture_storage.adopt(builder)?;
    material_storage.reserve_vec(
        ctx,
        &mut material.texture_slots,
        1,
        "gltf-material-texture-slots",
    )?;
    let source_key =
        material_storage.copy_string(ctx, source_key, "gltf-texture-slot-source-key")?;
    material.texture_slots.push(TextureSlot {
        texture: texture_id,
        role,
        color_space,
        uv_set: tex_coord,
        scale,
        transform: TextureTransform::default(),
        source_key: Some(source_key),
    });
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct PrimitiveContract {
    mode: Mode,
    position_count: usize,
    index_count: usize,
    face_count: usize,
    attribute_stream_count: usize,
    material_index: Option<usize>,
}

#[derive(Clone, Copy)]
enum AccessorOwner {
    Primitive {
        mesh_index: usize,
        primitive_index: usize,
    },
    Skin {
        skin_index: usize,
    },
}

impl fmt::Display for AccessorOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primitive {
                mesh_index,
                primitive_index,
            } => write!(
                formatter,
                "glTF mesh {mesh_index} primitive {primitive_index}"
            ),
            Self::Skin { skin_index } => write!(formatter, "glTF skin {skin_index}"),
        }
    }
}

fn validate_primitive_contract(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Gltf,
    mesh_index: usize,
    primitive_index: usize,
    material_count: usize,
) -> Result<PrimitiveContract> {
    let primitive = json_primitive(ctx, gltf, mesh_index, primitive_index)?;
    let attribute_stream_count = primitive.attributes.len();
    enforce_usize_limit(
        ctx,
        "max_vertex_attribute_streams_per_mesh",
        attribute_stream_count,
        ctx.limits().max_vertex_attribute_streams_per_mesh,
    )?;
    let owner = AccessorOwner::Primitive {
        mesh_index,
        primitive_index,
    };
    let mode = checked_primitive_mode(ctx, mesh_index, primitive_index, primitive.mode)?;
    let position_index = primitive
        .attributes
        .get(&Checked::Valid(json::mesh::Semantic::Positions))
        .ok_or_else(|| {
            ContentError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF mesh {mesh_index} primitive {primitive_index} has no POSITION attribute"
                ),
            )
        })?
        .value();
    let position_count = validate_accessor(
        ctx,
        gltf,
        position_index,
        owner,
        "POSITION",
        AccessorContract::vertex(
            &[json::accessor::ComponentType::F32],
            &[json::accessor::Type::Vec3],
        ),
    )?
    .count;

    let mut has_joints = false;
    let mut has_weights = false;
    for (semantic, accessor) in &primitive.attributes {
        let label = match semantic {
            Checked::Valid(json::mesh::Semantic::Positions) => continue,
            Checked::Valid(json::mesh::Semantic::Normals) => "NORMAL",
            Checked::Valid(json::mesh::Semantic::Tangents) => "TANGENT",
            Checked::Valid(json::mesh::Semantic::TexCoords(_)) => "TEXCOORD",
            Checked::Valid(json::mesh::Semantic::Colors(_)) => "COLOR",
            Checked::Valid(json::mesh::Semantic::Joints(_)) => "JOINTS",
            Checked::Valid(json::mesh::Semantic::Weights(_)) => "WEIGHTS",
            Checked::Invalid => {
                return Err(ContentError::parse(
                    ctx.source().to_string(),
                    None,
                    format!(
                        "glTF mesh {mesh_index} primitive {primitive_index} has an invalid attribute semantic"
                    ),
                ));
            }
        };
        let contract = match semantic {
            Checked::Valid(json::mesh::Semantic::Normals) => AccessorContract::vertex(
                &[json::accessor::ComponentType::F32],
                &[json::accessor::Type::Vec3],
            ),
            Checked::Valid(json::mesh::Semantic::Tangents) => AccessorContract::vertex(
                &[json::accessor::ComponentType::F32],
                &[json::accessor::Type::Vec4],
            ),
            Checked::Valid(json::mesh::Semantic::TexCoords(_)) => AccessorContract::vertex(
                &[
                    json::accessor::ComponentType::F32,
                    json::accessor::ComponentType::U8,
                    json::accessor::ComponentType::U16,
                ],
                &[json::accessor::Type::Vec2],
            )
            .require_normalized_integer(),
            Checked::Valid(json::mesh::Semantic::Colors(_)) => AccessorContract::vertex(
                &[
                    json::accessor::ComponentType::F32,
                    json::accessor::ComponentType::U8,
                    json::accessor::ComponentType::U16,
                ],
                &[json::accessor::Type::Vec3, json::accessor::Type::Vec4],
            )
            .require_normalized_integer(),
            Checked::Valid(json::mesh::Semantic::Joints(_)) => AccessorContract::vertex(
                &[
                    json::accessor::ComponentType::U8,
                    json::accessor::ComponentType::U16,
                ],
                &[json::accessor::Type::Vec4],
            ),
            Checked::Valid(json::mesh::Semantic::Weights(_)) => AccessorContract::vertex(
                &[
                    json::accessor::ComponentType::F32,
                    json::accessor::ComponentType::U8,
                    json::accessor::ComponentType::U16,
                ],
                &[json::accessor::Type::Vec4],
            )
            .require_normalized_integer(),
            _ => continue,
        };
        let info = validate_accessor(ctx, gltf, accessor.value(), owner, label, contract)?;
        if info.count != position_count {
            return Err(ContentError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "{owner} {label} accessor count {} does not match POSITION count {position_count}",
                    info.count
                ),
            ));
        }
        if matches!(semantic, Checked::Valid(json::mesh::Semantic::Joints(_))) {
            has_joints = true;
        }
        if matches!(semantic, Checked::Valid(json::mesh::Semantic::Weights(_))) {
            has_weights = true;
        }
    }
    if has_joints != has_weights {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!("{owner} JOINTS and WEIGHTS attributes must be provided together"),
        ));
    }

    let index_count = if let Some(index_accessor) = primitive.indices {
        Some(
            validate_accessor(
                ctx,
                gltf,
                index_accessor.value(),
                owner,
                "indices",
                AccessorContract::tightly_packed(
                    &[
                        json::accessor::ComponentType::U8,
                        json::accessor::ComponentType::U16,
                        json::accessor::ComponentType::U32,
                    ],
                    &[json::accessor::Type::Scalar],
                ),
            )?
            .count,
        )
    } else {
        None
    };
    let face_input_count = index_count.unwrap_or(position_count);
    let index_count = index_count.unwrap_or_default();
    let face_count = match mode {
        Mode::Triangles => face_input_count / 3,
        Mode::Points | Mode::Lines => 0,
        Mode::LineLoop | Mode::LineStrip | Mode::TriangleStrip | Mode::TriangleFan => 0,
    };

    let material_index = primitive.material.map(|material| material.value());
    if let Some(material_index) = material_index
        && material_index >= material_count
    {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "glTF mesh {mesh_index} primitive {primitive_index} material reference is out of range"
            ),
        ));
    }

    Ok(PrimitiveContract {
        mode,
        position_count,
        index_count,
        face_count,
        attribute_stream_count,
        material_index,
    })
}

fn json_primitive<'a>(
    ctx: &ParseScope<'_>,
    gltf: &'a gltf::Gltf,
    mesh_index: usize,
    primitive_index: usize,
) -> Result<&'a json::mesh::Primitive> {
    gltf.as_json()
        .meshes
        .get(mesh_index)
        .and_then(|mesh| mesh.primitives.get(primitive_index))
        .ok_or_else(|| {
            ContentError::parse(
                ctx.source().to_string(),
                None,
                format!("glTF mesh {mesh_index} primitive {primitive_index} is out of range"),
            )
        })
}

fn checked_primitive_mode(
    ctx: &ParseScope<'_>,
    mesh_index: usize,
    primitive_index: usize,
    mode: Checked<json::mesh::Mode>,
) -> Result<Mode> {
    match mode {
        Checked::Valid(json::mesh::Mode::Points) => Ok(Mode::Points),
        Checked::Valid(json::mesh::Mode::Lines) => Ok(Mode::Lines),
        Checked::Valid(json::mesh::Mode::LineLoop) => Ok(Mode::LineLoop),
        Checked::Valid(json::mesh::Mode::LineStrip) => Ok(Mode::LineStrip),
        Checked::Valid(json::mesh::Mode::Triangles) => Ok(Mode::Triangles),
        Checked::Valid(json::mesh::Mode::TriangleStrip) => Ok(Mode::TriangleStrip),
        Checked::Valid(json::mesh::Mode::TriangleFan) => Ok(Mode::TriangleFan),
        Checked::Invalid => Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "glTF mesh {mesh_index} primitive {primitive_index} has an invalid primitive mode"
            ),
        )),
    }
}

#[derive(Clone, Copy)]
struct AccessorContract {
    components: &'static [json::accessor::ComponentType],
    dimensions: &'static [json::accessor::Type],
    allow_stride: bool,
    require_normalized_integer: bool,
}

impl AccessorContract {
    fn vertex(
        components: &'static [json::accessor::ComponentType],
        dimensions: &'static [json::accessor::Type],
    ) -> Self {
        Self {
            components,
            dimensions,
            allow_stride: true,
            require_normalized_integer: false,
        }
    }

    fn tightly_packed(
        components: &'static [json::accessor::ComponentType],
        dimensions: &'static [json::accessor::Type],
    ) -> Self {
        Self {
            components,
            dimensions,
            allow_stride: false,
            require_normalized_integer: false,
        }
    }

    fn require_normalized_integer(mut self) -> Self {
        self.require_normalized_integer = true;
        self
    }
}

#[derive(Debug, Clone, Copy)]
struct AccessorInfo {
    count: usize,
}

fn validate_accessor(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Gltf,
    accessor_index: usize,
    owner: AccessorOwner,
    label: &'static str,
    contract: AccessorContract,
) -> Result<AccessorInfo> {
    let accessor = gltf
        .as_json()
        .accessors
        .get(accessor_index)
        .ok_or_else(|| {
            GltfFailure::invalid_accessor(
                Some(accessor_index),
                format_args!("{owner} {label} accessor reference is out of range"),
            )
        })?;
    let component = match accessor.component_type {
        Checked::Valid(component) => component.0,
        Checked::Invalid => {
            return Err(ContentError::parse(
                ctx.source().to_string(),
                None,
                format!("{owner} {label} accessor has an invalid component type"),
            ));
        }
    };
    let dimension = match accessor.type_ {
        Checked::Valid(dimension) => dimension,
        Checked::Invalid => {
            return Err(ContentError::parse(
                ctx.source().to_string(),
                None,
                format!("{owner} {label} accessor has an invalid type"),
            ));
        }
    };
    if !contract.components.contains(&component) || !contract.dimensions.contains(&dimension) {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!("{owner} {label} accessor type is unsupported"),
        ));
    }
    if accessor.sparse.is_some() {
        return Err(ContentError::FeatureUnsupported {
            format: "gltf",
            feature: format!("{label} sparse accessor"),
        });
    }
    if accessor.normalized && component == json::accessor::ComponentType::F32 {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!("{owner} float {label} accessor must not be normalized"),
        ));
    }
    if contract.require_normalized_integer
        && component != json::accessor::ComponentType::F32
        && !accessor.normalized
    {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!("{owner} integer {label} accessor must be normalized"),
        ));
    }
    validate_accessor_layout(
        ctx,
        gltf,
        AccessorLayoutCheck {
            accessor,
            owner,
            label,
            component,
            dimension,
            contract,
        },
    )?;

    let count = usize::try_from(accessor.count.0)
        .map_err(|_| ctx.allocation_failure("gltf-accessor-count"))?;
    Ok(AccessorInfo { count })
}

#[derive(Clone, Copy)]
struct AccessorLayoutCheck<'a> {
    accessor: &'a json::accessor::Accessor,
    owner: AccessorOwner,
    label: &'static str,
    component: json::accessor::ComponentType,
    dimension: json::accessor::Type,
    contract: AccessorContract,
}

fn validate_accessor_layout(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Gltf,
    check: AccessorLayoutCheck<'_>,
) -> Result<()> {
    let buffer_view_index = check
        .accessor
        .buffer_view
        .ok_or_else(|| {
            ContentError::parse(
                ctx.source().to_string(),
                None,
                format!("{} {} accessor has no bufferView", check.owner, check.label),
            )
        })?
        .value();
    let buffer_view = gltf
        .as_json()
        .buffer_views
        .get(buffer_view_index)
        .ok_or_else(|| {
            ContentError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "{} {} bufferView reference is out of range",
                    check.owner, check.label
                ),
            )
        })?;
    let buffer_index = buffer_view.buffer.value();
    let buffer = gltf.as_json().buffers.get(buffer_index).ok_or_else(|| {
        ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "{} {} buffer reference is out of range",
                check.owner, check.label
            ),
        )
    })?;

    let component_size = component_size(check.component);
    let element_size = element_size(check.component, check.dimension);
    let stride = match buffer_view.byte_stride {
        Some(_) if !check.contract.allow_stride => {
            return Err(ContentError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "{} {} accessor uses byteStride where it is unsupported",
                    check.owner, check.label
                ),
            ));
        }
        Some(stride) => stride.0 as u64,
        None => element_size,
    };
    if stride < element_size {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "{} {} byteStride is smaller than the accessor element size",
                check.owner, check.label
            ),
        ));
    }
    if !stride.is_multiple_of(component_size) {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "{} {} byteStride is not aligned to component size",
                check.owner, check.label
            ),
        ));
    }

    let view_offset = buffer_view.byte_offset.map_or(0, |offset| offset.0);
    let view_length = buffer_view.byte_length.0;
    let accessor_offset = check.accessor.byte_offset.map_or(0, |offset| offset.0);
    let absolute_offset = view_offset.checked_add(accessor_offset).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format!("{} {} byte offset overflow", check.owner, check.label),
        )
    })?;
    if !absolute_offset.is_multiple_of(component_size) {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "{} {} byte offset is not aligned to component size",
                check.owner, check.label
            ),
        ));
    }

    let view_end = view_offset.checked_add(view_length).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format!("{} {} bufferView range overflow", check.owner, check.label),
        )
    })?;
    if view_end > buffer.byte_length.0 {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "{} {} bufferView exceeds declared buffer byteLength",
                check.owner, check.label
            ),
        ));
    }

    let required = required_accessor_bytes(check.accessor.count.0, element_size, stride)
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("{} {} byte range overflow", check.owner, check.label),
            )
        })?;
    let accessor_end = accessor_offset.checked_add(required).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format!("{} {} accessor range overflow", check.owner, check.label),
        )
    })?;
    if accessor_end > view_length {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "{} {} bufferView byteLength does not cover accessor data",
                check.owner, check.label
            ),
        ));
    }

    Ok(())
}

fn component_size(component: json::accessor::ComponentType) -> u64 {
    match component {
        json::accessor::ComponentType::I8 | json::accessor::ComponentType::U8 => 1,
        json::accessor::ComponentType::I16 | json::accessor::ComponentType::U16 => 2,
        json::accessor::ComponentType::U32 | json::accessor::ComponentType::F32 => 4,
    }
}

fn component_count(dimension: json::accessor::Type) -> u64 {
    match dimension {
        json::accessor::Type::Scalar => 1,
        json::accessor::Type::Vec2 => 2,
        json::accessor::Type::Vec3 => 3,
        json::accessor::Type::Vec4 | json::accessor::Type::Mat2 => 4,
        json::accessor::Type::Mat3 => 9,
        json::accessor::Type::Mat4 => 16,
    }
}

fn element_size(component: json::accessor::ComponentType, dimension: json::accessor::Type) -> u64 {
    component_size(component) * component_count(dimension)
}

fn required_accessor_bytes(count: u64, element_size: u64, stride: u64) -> Option<u64> {
    if count == 0 {
        return Some(0);
    }
    count
        .checked_sub(1)?
        .checked_mul(stride)?
        .checked_add(element_size)
}

fn safe_gltf<T>(ctx: &ParseScope<'_>, operation: &'static str, f: impl FnOnce() -> T) -> Result<T> {
    ctx.poll()?;
    let outcome = catch_unwind(AssertUnwindSafe(f));
    ctx.poll()?;
    finish_gltf_operation(operation, outcome)
}

fn finish_gltf_operation<T>(operation: &'static str, outcome: std::thread::Result<T>) -> Result<T> {
    match outcome {
        Ok(value) => Ok(value),
        Err(payload) => {
            if payload.is::<String>() || payload.is::<&'static str>() {
                drop(payload);
            } else {
                std::mem::forget(payload);
            }
            Err(GltfFailure::parser_panicked(operation).into())
        }
    }
}

fn poll_parser_loop(ctx: &ParseScope<'_>, completed: usize) -> Result<()> {
    if completed.is_multiple_of(PARSER_POLL_INTERVAL) {
        ctx.poll()?;
    }
    Ok(())
}

fn collect_exact<'scope, T>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    expected: usize,
    allocation: &'static str,
    values: impl Iterator<Item = T>,
) -> Result<Vec<T>> {
    let mut collected = Vec::new();
    storage.reserve_vec(ctx, &mut collected, expected, allocation)?;
    for value in values {
        poll_parser_loop(ctx, collected.len())?;
        if collected.len() == expected {
            return Err(gltf_parse_error(
                ctx,
                format!("{allocation} produced more than {expected} values"),
            ));
        }
        collected.push(value);
    }
    if collected.len() != expected {
        return Err(gltf_parse_error(
            ctx,
            format!(
                "{allocation} produced {} values, expected {expected}",
                collected.len()
            ),
        ));
    }
    Ok(collected)
}

fn try_vec_capacity<'scope, T>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    capacity: usize,
    allocation: &'static str,
) -> Result<Vec<T>> {
    let mut values = Vec::new();
    storage.reserve_vec(ctx, &mut values, capacity, allocation)?;
    Ok(values)
}

fn try_vec_filled<'scope, T: Clone>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    length: usize,
    value: T,
    allocation: &'static str,
) -> Result<Vec<T>> {
    let mut values = try_vec_capacity(ctx, storage, length, allocation)?;
    values.resize(length, value);
    Ok(values)
}

#[allow(clippy::too_many_arguments)]
fn add_primitive_mesh(
    ctx: &ParseScope<'_>,
    builder: &mut ScopedSceneBuilder,
    gltf: &gltf::Gltf,
    material_ids: &[baozi_core::MaterialId],
    buffers: &[BufferData<'_>],
    mesh: &gltf::Mesh<'_>,
    primitive: gltf::Primitive<'_>,
) -> Result<baozi_core::MeshId> {
    let contract = validate_primitive_contract(
        ctx,
        gltf,
        mesh.index(),
        primitive.index(),
        material_ids.len(),
    )?;
    let topology = match map_topology(contract.mode) {
        Some(topology) => topology,
        None => {
            return Err(ContentError::FeatureUnsupported {
                format: "gltf",
                feature: format!("primitive mode {:?} is not implemented yet", contract.mode),
            });
        }
    };

    let mesh_work = ctx.begin_work(ImportWorkClass::Meshes, 1)?;
    let primitive_work = ctx.begin_work(ImportWorkClass::Primitives, 1)?;
    let vertex_work = ctx.begin_work(ImportWorkClass::Vertices, to_u64(contract.position_count))?;
    let face_work = ctx.begin_work(ImportWorkClass::Faces, to_u64(contract.face_count))?;
    let topology_work =
        ctx.begin_work(ImportWorkClass::TopologyCells, to_u64(contract.index_count))?;
    let attribute_stream_work = ctx.begin_work(
        ImportWorkClass::AttributeStreams,
        to_u64(contract.attribute_stream_count),
    )?;
    let attribute_cell_count = contract
        .position_count
        .checked_mul(contract.attribute_stream_count)
        .ok_or_else(|| ctx.allocation_failure("gltf-attribute-cell-counter"))?;
    let attribute_cell_work = ctx.begin_work(
        ImportWorkClass::AttributeCells,
        to_u64(attribute_cell_count),
    )?;
    let mut mesh_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);

    let reader = safe_gltf(ctx, "glTF primitive reader construction", || {
        primitive.reader(|buffer| buffers.get(buffer.index()).map(BufferData::as_slice))
    })?;
    let positions: Vec<_> = safe_gltf(ctx, "glTF POSITION reader", || {
        reader.read_positions().map(|positions| {
            collect_exact(
                ctx,
                &mut mesh_storage,
                contract.position_count,
                "gltf-position-values",
                positions.map(vec3),
            )
        })
    })?
    .transpose()?
    .ok_or_else(|| {
        ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "glTF mesh {} primitive {} POSITION accessor could not be read",
                mesh.index(),
                primitive.index()
            ),
        )
    })?;
    let bounds = compute_bounds(ctx, &positions)?;
    let normals: Vec<_> = safe_gltf(ctx, "glTF NORMAL reader", || {
        reader.read_normals().map(|values| {
            collect_exact(
                ctx,
                &mut mesh_storage,
                contract.position_count,
                "gltf-normal-values",
                values.map(vec3),
            )
        })
    })?
    .transpose()?
    .unwrap_or_default();
    let tangents: Vec<_> = safe_gltf(ctx, "glTF TANGENT reader", || {
        reader.read_tangents().map(|values| {
            collect_exact(
                ctx,
                &mut mesh_storage,
                contract.position_count,
                "gltf-tangent-values",
                values.map(vec4),
            )
        })
    })?
    .transpose()?
    .unwrap_or_default();
    let indices: Vec<_> = safe_gltf(ctx, "glTF indices reader", || {
        reader.read_indices().map(|indices| {
            collect_exact(
                ctx,
                &mut mesh_storage,
                contract.index_count,
                "gltf-index-values",
                indices.into_u32(),
            )
        })
    })?
    .transpose()?
    .unwrap_or_default();

    let mut texcoords = try_vec_capacity(ctx, &mut mesh_storage, 8, "gltf-texcoord-streams")?;
    for set in 0..8 {
        let Some(values) = safe_gltf(ctx, "glTF TEXCOORD reader", || {
            reader.read_tex_coords(set).map(|values| {
                collect_exact(
                    ctx,
                    &mut mesh_storage,
                    contract.position_count,
                    "gltf-texcoord-values",
                    values.into_f32().map(vec2),
                )
            })
        })?
        .transpose()?
        else {
            break;
        };
        texcoords.push(values);
    }
    let mut colors = try_vec_capacity(ctx, &mut mesh_storage, 8, "gltf-color-streams")?;
    for set in 0..8 {
        let Some(values) = safe_gltf(ctx, "glTF COLOR reader", || {
            reader.read_colors(set).map(|values| {
                collect_exact(
                    ctx,
                    &mut mesh_storage,
                    contract.position_count,
                    "gltf-color-values",
                    values.into_rgba_f32().map(color),
                )
            })
        })?
        .transpose()?
        else {
            break;
        };
        colors.push(values);
    }
    let joint_indices = safe_gltf(ctx, "glTF JOINTS reader", || {
        reader.read_joints(0).map(|values| {
            collect_exact(
                ctx,
                &mut mesh_storage,
                contract.position_count,
                "gltf-joint-index-values",
                values.into_u16(),
            )
        })
    })?
    .transpose()?
    .unwrap_or_default();
    let joint_weights = safe_gltf(ctx, "glTF WEIGHTS reader", || {
        reader.read_weights(0).map(|values| {
            collect_exact(
                ctx,
                &mut mesh_storage,
                contract.position_count,
                "gltf-joint-weight-values",
                values.into_f32(),
            )
        })
    })?
    .transpose()?
    .unwrap_or_default();

    if safe_gltf(ctx, "glTF morph target iterator", || {
        primitive.morph_targets().next().is_some()
    })? {
        push_warning(
            ctx,
            "gltf.morph_targets_ignored",
            format_args!(
                "glTF mesh {} primitive {} morph targets were ignored",
                mesh.index(),
                primitive.index()
            ),
        )?;
    }

    let material = contract
        .material_index
        .and_then(|index| material_ids.get(index).copied());
    let mesh_name = primitive_name(ctx, &mut mesh_storage, mesh.name(), primitive.index())?;
    let mesh_out = Mesh {
        name: mesh_name,
        topology,
        positions,
        normals,
        tangents,
        texcoords,
        colors,
        indices,
        material,
        joint_indices,
        joint_weights,
        bounds,
        metadata: MetadataMap::new(),
        ..Mesh::default()
    };
    let mesh_id = builder.add_mesh(mesh_out)?;
    mesh_storage.adopt(builder)?;
    attribute_cell_work.complete()?;
    attribute_stream_work.complete()?;
    topology_work.complete()?;
    face_work.complete()?;
    vertex_work.complete()?;
    primitive_work.complete()?;
    mesh_work.complete()?;
    Ok(mesh_id)
}

fn add_scene_nodes<'scope>(
    ctx: &'scope ParseScope<'_>,
    normalization_storage: &mut LiveStorage<'scope>,
    builder: &mut ScopedSceneBuilder,
    graph: &NodeGraph<'_, '_>,
    mesh_ids_by_gltf_mesh: &[Vec<baozi_core::MeshId>],
) -> Result<NodeImportMaps> {
    let mut imports = NodeImportMaps {
        node_ids: try_vec_filled(
            ctx,
            normalization_storage,
            graph.nodes.len(),
            None,
            "gltf-node-id-map",
        )?,
        node_skins: try_vec_capacity(
            ctx,
            normalization_storage,
            graph.nodes.len(),
            "gltf-node-skin-map",
        )?,
    };
    let root = builder.root();
    let mut pending = try_vec_capacity(
        ctx,
        normalization_storage,
        graph.nodes.len(),
        "gltf-node-build-stack",
    )?;
    for (root_ordinal, node_index) in graph.roots.iter().rev().copied().enumerate() {
        poll_parser_loop(ctx, root_ordinal)?;
        pending.push((node_index, root));
    }

    let mut constructed = 0usize;
    while let Some((node_index, parent)) = pending.pop() {
        poll_parser_loop(ctx, constructed)?;
        constructed += 1;
        let node = graph.nodes.get(node_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF node graph references missing node {node_index}"),
            )
        })?;
        let node_id = add_node(
            ctx,
            builder,
            parent,
            node,
            mesh_ids_by_gltf_mesh,
            &mut imports,
        )?;
        for (child_ordinal, child_index) in
            graph.children[node_index].iter().rev().copied().enumerate()
        {
            poll_parser_loop(ctx, child_ordinal)?;
            pending.push((child_index, node_id));
        }
    }
    Ok(imports)
}

struct NodeGraph<'document, 'scope> {
    nodes: Vec<gltf::Node<'document>>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    _storage: LiveStorage<'scope>,
}

fn preflight_node_graph<'document, 'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Gltf,
) -> Result<NodeGraph<'document, 'scope>> {
    let node_count = safe_gltf(ctx, "glTF node count", || gltf.nodes().len())?;
    enforce_usize_limit(
        ctx,
        "max_graph_nodes",
        node_count,
        ctx.limits().max_graph_nodes,
    )?;

    let mut graph_work = 0usize;
    charge_graph_work(ctx, &mut graph_work, node_count)?;
    let mut graph_storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
    let mut scratch_storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
    let nodes = safe_gltf(ctx, "glTF nodes", || {
        collect_exact(
            ctx,
            &mut graph_storage,
            node_count,
            "gltf-node-list",
            gltf.nodes(),
        )
    })??;
    let mut children = try_vec_capacity(ctx, &mut graph_storage, node_count, "gltf-node-children")?;
    let mut parents = try_vec_filled(
        ctx,
        &mut scratch_storage,
        node_count,
        None,
        "gltf-node-parents",
    )?;
    let mut edge_count = 0usize;

    for (parent_index, node) in nodes.iter().enumerate() {
        poll_parser_loop(ctx, parent_index)?;
        let child_count = safe_gltf(ctx, "glTF node child count", || node.children().len())?;
        edge_count = checked_add_with_limit(
            ctx,
            "max_graph_edges",
            edge_count,
            child_count,
            ctx.limits().max_graph_edges,
        )?;
        charge_graph_work(ctx, &mut graph_work, child_count)?;

        let child_indices = safe_gltf(ctx, "glTF node children", || {
            collect_exact(
                ctx,
                &mut graph_storage,
                child_count,
                "gltf-node-child-list",
                node.children().map(|child| child.index()),
            )
        })??;
        for (child_ordinal, &child_index) in child_indices.iter().enumerate() {
            poll_parser_loop(ctx, child_ordinal)?;
            if child_index >= node_count {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF node {parent_index} references missing child {child_index}"),
                ));
            }
            if child_index == parent_index {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF node graph contains a cycle at node {parent_index}"),
                ));
            }
            if let Some(existing_parent) = parents[child_index] {
                return Err(gltf_parse_error(
                    ctx,
                    format!(
                        "glTF node {child_index} has multiple parents ({existing_parent} and {parent_index})"
                    ),
                ));
            }
            parents[child_index] = Some(parent_index);
        }
        children.push(child_indices);
    }

    validate_graph_depth_and_cycles(ctx, &children, &parents)?;
    let roots =
        select_and_validate_scene_roots(ctx, &mut graph_storage, gltf, &parents, &mut graph_work)?;
    let usage = validate_canonical_graph_limits(ctx, &children, &roots)?;
    charge_canonical_graph(ctx, usage)?;

    Ok(NodeGraph {
        nodes,
        children,
        roots,
        _storage: graph_storage,
    })
}

#[derive(Debug, Clone, Copy)]
struct CanonicalGraphUsage {
    nodes: usize,
    edges: usize,
    depth: usize,
    work: usize,
}

fn validate_canonical_graph_limits(
    ctx: &ParseScope<'_>,
    children: &[Vec<usize>],
    roots: &[usize],
) -> Result<CanonicalGraphUsage> {
    let mut node_count = 1usize;
    enforce_usize_limit(
        ctx,
        "max_graph_nodes",
        node_count,
        ctx.limits().max_graph_nodes,
    )?;
    let mut max_depth = 1usize;
    enforce_usize_limit(
        ctx,
        "max_graph_depth",
        max_depth,
        ctx.limits().max_graph_depth,
    )?;

    let mut edge_count = roots.len();
    enforce_usize_limit(
        ctx,
        "max_graph_edges",
        edge_count,
        ctx.limits().max_graph_edges,
    )?;
    validate_canonical_graph_work(ctx, node_count, edge_count)?;

    let mut storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
    let mut pending =
        try_vec_capacity(ctx, &mut storage, roots.len(), "gltf-canonical-graph-stack")?;
    for (root_ordinal, root) in roots.iter().rev().copied().enumerate() {
        poll_parser_loop(ctx, root_ordinal)?;
        pending.push((root, 2usize));
    }

    while let Some((node_index, depth)) = pending.pop() {
        poll_parser_loop(ctx, node_count)?;
        node_count = checked_add_with_limit(
            ctx,
            "max_graph_nodes",
            node_count,
            1,
            ctx.limits().max_graph_nodes,
        )?;
        enforce_usize_limit(ctx, "max_graph_depth", depth, ctx.limits().max_graph_depth)?;
        max_depth = max_depth.max(depth);

        let node_children = children.get(node_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF node graph references missing node {node_index}"),
            )
        })?;
        edge_count = checked_add_with_limit(
            ctx,
            "max_graph_edges",
            edge_count,
            node_children.len(),
            ctx.limits().max_graph_edges,
        )?;
        validate_canonical_graph_work(ctx, node_count, edge_count)?;

        let child_depth = depth
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-graph-depth-counter"))?;
        storage.reserve_vec(
            ctx,
            &mut pending,
            node_children.len(),
            "gltf-canonical-graph-stack",
        )?;
        for (child_ordinal, child) in node_children.iter().rev().copied().enumerate() {
            poll_parser_loop(ctx, child_ordinal)?;
            pending.push((child, child_depth));
        }
    }

    let work = node_count
        .checked_add(edge_count)
        .ok_or_else(|| ctx.allocation_failure("gltf-canonical-graph-work-counter"))?;
    Ok(CanonicalGraphUsage {
        nodes: node_count,
        edges: edge_count,
        depth: max_depth,
        work,
    })
}

fn validate_canonical_graph_work(
    ctx: &ParseScope<'_>,
    node_count: usize,
    edge_count: usize,
) -> Result<()> {
    let work = node_count
        .checked_add(edge_count)
        .ok_or_else(|| ctx.allocation_failure("gltf-canonical-graph-work-counter"))?;
    enforce_usize_limit(ctx, "max_graph_work", work, ctx.limits().max_graph_work)?;
    Ok(())
}

fn charge_canonical_graph(ctx: &ParseScope<'_>, usage: CanonicalGraphUsage) -> Result<()> {
    let nodes = ctx.begin_work(ImportWorkClass::GraphNodes, to_u64(usage.nodes))?;
    let edges = ctx.begin_work(ImportWorkClass::GraphEdges, to_u64(usage.edges))?;
    let depth = ctx.begin_work(ImportWorkClass::GraphDepth, to_u64(usage.depth))?;
    let work = ctx.begin_work(ImportWorkClass::GraphWork, to_u64(usage.work))?;
    work.complete()?;
    depth.complete()?;
    edges.complete()?;
    nodes.complete()?;
    Ok(())
}

fn charge_graph_work(ctx: &ParseScope<'_>, graph_work: &mut usize, amount: usize) -> Result<()> {
    *graph_work = checked_add_with_limit(
        ctx,
        "max_graph_work",
        *graph_work,
        amount,
        ctx.limits().max_graph_work,
    )?;
    Ok(())
}

fn validate_graph_depth_and_cycles(
    ctx: &ParseScope<'_>,
    children: &[Vec<usize>],
    parents: &[Option<usize>],
) -> Result<()> {
    let mut storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
    let mut depths = try_vec_filled(ctx, &mut storage, parents.len(), 0usize, "gltf-node-depths")?;
    let mut queue = try_vec_capacity(ctx, &mut storage, parents.len(), "gltf-node-depth-queue")?;
    for (node_index, parent) in parents.iter().enumerate() {
        if parent.is_none() {
            depths[node_index] = 1;
            queue.push(node_index);
        }
    }

    let mut cursor = 0usize;
    while cursor < queue.len() {
        poll_parser_loop(ctx, cursor)?;
        let node_index = queue[cursor];
        cursor += 1;
        let depth = depths[node_index];
        enforce_usize_limit(ctx, "max_graph_depth", depth, ctx.limits().max_graph_depth)?;
        for (child_ordinal, &child_index) in children[node_index].iter().enumerate() {
            poll_parser_loop(ctx, child_ordinal)?;
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-graph-depth-counter"))?;
            depths[child_index] = child_depth;
            queue.push(child_index);
        }
    }

    if queue.len() != parents.len() {
        return Err(gltf_parse_error(ctx, "glTF node graph contains a cycle"));
    }
    Ok(())
}

fn select_and_validate_scene_roots<'scope>(
    ctx: &'scope ParseScope<'_>,
    result_storage: &mut LiveStorage<'scope>,
    gltf: &gltf::Gltf,
    parents: &[Option<usize>],
    graph_work: &mut usize,
) -> Result<Vec<usize>> {
    let scene_count = safe_gltf(ctx, "glTF scene count", || gltf.scenes().len())?;
    charge_graph_work(ctx, graph_work, scene_count)?;
    let mut scratch_storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
    let mut scene_roots = try_vec_capacity(
        ctx,
        &mut scratch_storage,
        scene_count,
        "gltf-scene-root-lists",
    )?;

    for (scene_index, scene) in gltf.scenes().enumerate() {
        poll_parser_loop(ctx, scene_index)?;
        let root_count = safe_gltf(ctx, "glTF scene root count", || scene.nodes().len())?;
        charge_graph_work(ctx, graph_work, root_count)?;
        let roots = safe_gltf(ctx, "glTF scene roots", || {
            collect_exact(
                ctx,
                &mut scratch_storage,
                root_count,
                "gltf-scene-roots",
                scene.nodes().map(|node| node.index()),
            )
        })??;
        scene_roots.push(roots);
    }

    let mut seen_in_scene = try_vec_filled(
        ctx,
        &mut scratch_storage,
        parents.len(),
        usize::MAX,
        "gltf-scene-root-membership",
    )?;
    for (scene_index, roots) in scene_roots.iter().enumerate() {
        for (root_ordinal, &root_index) in roots.iter().enumerate() {
            poll_parser_loop(ctx, root_ordinal)?;
            let Some(parent) = parents.get(root_index) else {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF scene {scene_index} references missing root node {root_index}"),
                ));
            };
            if seen_in_scene[root_index] == scene_index {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF scene {scene_index} contains duplicate root node {root_index}"),
                ));
            }
            seen_in_scene[root_index] = scene_index;
            if parent.is_some() {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF scene {scene_index} root node {root_index} has a parent"),
                ));
            }
        }
    }

    let default_scene_index = safe_gltf(ctx, "glTF default scene", || {
        gltf.default_scene().map(|scene| scene.index())
    })?;
    let selected_scene_index = default_scene_index.or((scene_count > 0).then_some(0));
    match selected_scene_index {
        Some(scene_index) => {
            let selected = scene_roots.get(scene_index).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format!("glTF default scene index {scene_index} is out of range"),
                )
            })?;
            let mut roots = try_vec_capacity(
                ctx,
                result_storage,
                selected.len(),
                "gltf-selected-scene-roots",
            )?;
            for (root_ordinal, root) in selected.iter().copied().enumerate() {
                poll_parser_loop(ctx, root_ordinal)?;
                roots.push(root);
            }
            Ok(roots)
        }
        None => {
            let mut roots = try_vec_capacity(
                ctx,
                result_storage,
                parents.len(),
                "gltf-derived-scene-roots",
            )?;
            for (node_index, parent) in parents.iter().enumerate() {
                poll_parser_loop(ctx, node_index)?;
                if parent.is_none() {
                    roots.push(node_index);
                }
            }
            Ok(roots)
        }
    }
}

#[derive(Debug, Default)]
struct NodeImportMaps {
    node_ids: Vec<Option<baozi_core::NodeId>>,
    node_skins: Vec<(baozi_core::NodeId, usize)>,
}

fn add_node(
    ctx: &ParseScope<'_>,
    builder: &mut ScopedSceneBuilder,
    parent: baozi_core::NodeId,
    node: &gltf::Node<'_>,
    mesh_ids_by_gltf_mesh: &[Vec<baozi_core::MeshId>],
    imports: &mut NodeImportMaps,
) -> Result<baozi_core::NodeId> {
    let mesh = safe_gltf(ctx, "glTF node mesh", || node.mesh())?;
    let mesh_ids = mesh.and_then(|mesh| mesh_ids_by_gltf_mesh.get(mesh.index()));
    let binding_count = mesh_ids.map_or(0, Vec::len);
    let mut node_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let mut mesh_bindings = try_vec_capacity(
        ctx,
        &mut node_storage,
        binding_count,
        "gltf-node-mesh-bindings",
    )?;
    if let Some(mesh_ids) = mesh_ids {
        for (mesh_ordinal, mesh_id) in mesh_ids.iter().copied().enumerate() {
            poll_parser_loop(ctx, mesh_ordinal)?;
            mesh_bindings.push(MeshBinding::new(mesh_id));
        }
    }
    let camera = safe_gltf(ctx, "glTF node camera", || node.camera())?
        .map(|camera| baozi_core::CameraId::new(camera.index() as u32));
    let node_name = safe_gltf(ctx, "glTF node name", || node.name())?;
    let transform = safe_gltf(ctx, "glTF node transform", || node.transform().matrix())?;

    let node_id = builder.add_child_node(
        parent,
        Node {
            name: optional_name(ctx, &mut node_storage, node_name)?,
            transform: Mat4 { cols: transform },
            mesh_bindings,
            camera,
            ..Node::default()
        },
    )?;
    node_storage.adopt(builder)?;
    let node_index = node.index();
    let slot = imports.node_ids.get_mut(node_index).ok_or_else(|| {
        gltf_parse_error(ctx, format!("glTF node index {node_index} is out of range"))
    })?;
    if slot.is_some() {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF node {node_index} was constructed more than once"),
        ));
    }
    *slot = Some(node_id);
    if let Some(skin) = safe_gltf(ctx, "glTF node skin", || node.skin())? {
        imports.node_skins.push((node_id, skin.index()));
    }
    Ok(node_id)
}

fn add_skins<'scope>(
    ctx: &'scope ParseScope<'_>,
    normalization_storage: &mut LiveStorage<'scope>,
    builder: &mut ScopedSceneBuilder,
    gltf: &gltf::Gltf,
    buffers: &[BufferData<'_>],
    node_ids: &[Option<baozi_core::NodeId>],
    node_skins: &[(baozi_core::NodeId, usize)],
) -> Result<Vec<Option<baozi_core::SkinId>>> {
    let skin_count = safe_gltf(ctx, "glTF skin count", || gltf.skins().len())?;
    let mut referenced_skins = try_vec_filled(
        ctx,
        normalization_storage,
        skin_count,
        false,
        "gltf-referenced-skins",
    )?;
    for (binding_ordinal, (node_id, skin_index)) in node_skins.iter().enumerate() {
        poll_parser_loop(ctx, binding_ordinal)?;
        let Some(referenced) = referenced_skins.get_mut(*skin_index) else {
            return Err(gltf_parse_error(
                ctx,
                format!(
                    "glTF node {} references skin {} out of range",
                    node_id.as_u32(),
                    skin_index
                ),
            ));
        };
        *referenced = true;
    }

    let mut skin_ids = try_vec_filled(
        ctx,
        normalization_storage,
        skin_count,
        None,
        "gltf-skin-id-map",
    )?;
    for (ordinal, skin) in gltf.skins().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        let skin_index = skin.index();
        let Some(is_referenced) = referenced_skins.get(skin_index).copied() else {
            return Err(gltf_parse_error(
                ctx,
                format!("glTF skin index {skin_index} is out of range"),
            ));
        };
        if !is_referenced {
            continue;
        }

        let joint_count = safe_gltf(ctx, "glTF skin joint count", || skin.joints().len())?;
        let mut joint_storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
        let joints = safe_gltf(ctx, "glTF skin joints", || {
            collect_exact(
                ctx,
                &mut joint_storage,
                joint_count,
                "gltf-skin-joints",
                skin.joints(),
            )
        })??;
        let mut skin_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        let mut mapped_joints = try_vec_capacity(
            ctx,
            &mut skin_storage,
            joints.len(),
            "gltf-mapped-skin-joints",
        )?;
        for (joint_ordinal, joint) in joints.into_iter().enumerate() {
            poll_parser_loop(ctx, joint_ordinal)?;
            let Some(Some(node_id)) = node_ids.get(joint.index()) else {
                return Err(ContentError::parse(
                    ctx.source().to_string(),
                    None,
                    format!(
                        "glTF skin {} references joint node {} outside the imported scene",
                        skin.index(),
                        joint.index()
                    ),
                ));
            };
            mapped_joints.push(*node_id);
        }
        drop(joint_storage);

        let skeleton_root = match safe_gltf(ctx, "glTF skin skeleton root", || skin.skeleton())? {
            Some(root) => match node_ids.get(root.index()).copied().flatten() {
                Some(node_id) => Some(node_id),
                None => {
                    return Err(ContentError::parse(
                        ctx.source().to_string(),
                        None,
                        format!(
                            "glTF skin {} references skeleton root node {} outside the imported scene",
                            skin.index(),
                            root.index()
                        ),
                    ));
                }
            },
            None => None,
        };

        let has_inverse_bind_matrices =
            validate_skin_inverse_bind_accessor(ctx, gltf, skin.index(), mapped_joints.len())?;
        let reader = safe_gltf(ctx, "glTF skin reader construction", || {
            skin.reader(|buffer| buffers.get(buffer.index()).map(BufferData::as_slice))
        })?;
        let inverse_bind_matrices = safe_gltf(ctx, "glTF inverse bind matrix reader", || {
            reader
                .read_inverse_bind_matrices()
                .map(|matrices| {
                    collect_exact(
                        ctx,
                        &mut skin_storage,
                        mapped_joints.len(),
                        "gltf-inverse-bind-matrices",
                        matrices.map(|cols| Mat4 { cols }),
                    )
                })
                .transpose()
        })??
        .unwrap_or_default();
        if has_inverse_bind_matrices && inverse_bind_matrices.len() != mapped_joints.len() {
            return Err(ContentError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF skin {} inverse bind matrix count does not match joint count",
                    skin.index()
                ),
            ));
        }

        let skin_id = builder.add_skin(Skin {
            name: optional_name(ctx, &mut skin_storage, skin.name())?,
            joints: mapped_joints,
            inverse_bind_matrices,
            skeleton_root,
            metadata: MetadataMap::new(),
        })?;
        skin_storage.adopt(builder)?;
        let Some(slot) = skin_ids.get_mut(skin_index) else {
            return Err(gltf_parse_error(
                ctx,
                format!("glTF skin index {skin_index} is out of range"),
            ));
        };
        *slot = Some(skin_id);
    }
    Ok(skin_ids)
}

fn validate_skin_inverse_bind_accessor(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Gltf,
    skin_index: usize,
    joint_count: usize,
) -> Result<bool> {
    let json_skin = gltf.as_json().skins.get(skin_index).ok_or_else(|| {
        ContentError::parse(
            ctx.source().to_string(),
            None,
            format!("glTF skin {skin_index} is out of range"),
        )
    })?;
    let Some(accessor) = json_skin.inverse_bind_matrices else {
        return Ok(false);
    };
    let owner = AccessorOwner::Skin { skin_index };
    let info = validate_accessor(
        ctx,
        gltf,
        accessor.value(),
        owner,
        "inverseBindMatrices",
        AccessorContract::tightly_packed(
            &[json::accessor::ComponentType::F32],
            &[json::accessor::Type::Mat4],
        ),
    )?;
    if info.count != joint_count {
        return Err(ContentError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "{owner} inverse bind matrix count {} does not match joint count {joint_count}",
                info.count
            ),
        ));
    }
    Ok(true)
}

fn attach_node_skins(
    ctx: &ParseScope<'_>,
    builder: &mut ScopedSceneBuilder,
    node_skins: &[(baozi_core::NodeId, usize)],
    skin_ids: &[Option<baozi_core::SkinId>],
) -> Result<()> {
    for (binding_ordinal, (node_id, skin_index)) in node_skins.iter().enumerate() {
        poll_parser_loop(ctx, binding_ordinal)?;
        let Some(imported_skin_id) = skin_ids.get(*skin_index) else {
            return Err(ContentError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF node {} references skin {} out of range",
                    node_id.as_u32(),
                    skin_index
                ),
            ));
        };
        let Some(skin_id) = *imported_skin_id else {
            return Err(gltf_parse_error(
                ctx,
                format!(
                    "glTF node {} references skin {} that was not imported",
                    node_id.as_u32(),
                    skin_index
                ),
            ));
        };
        let Some(binding_count) = builder.node_mesh_binding_count(*node_id) else {
            return Err(ContentError::InvalidScene {
                message: format!("node {} is out of range", node_id.as_u32()),
            });
        };
        for binding_index in 0..binding_count {
            poll_parser_loop(ctx, binding_index)?;
            if !builder.set_node_mesh_binding_skin(*node_id, binding_index, skin_id) {
                return Err(ContentError::InvalidScene {
                    message: format!(
                        "node {} mesh binding {binding_index} is out of range",
                        node_id.as_u32()
                    ),
                });
            }
        }
    }
    Ok(())
}

fn add_cameras(
    ctx: &ParseScope<'_>,
    builder: &mut ScopedSceneBuilder,
    gltf: &gltf::Gltf,
) -> Result<()> {
    for (ordinal, camera) in gltf.cameras().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        let projection = safe_gltf(ctx, "glTF camera projection", || {
            match camera.projection() {
                gltf::camera::Projection::Orthographic(orthographic) => {
                    CameraProjection::Orthographic {
                        xmag: orthographic.xmag(),
                        ymag: orthographic.ymag(),
                        znear: orthographic.znear(),
                        zfar: orthographic.zfar(),
                    }
                }
                gltf::camera::Projection::Perspective(perspective) => {
                    CameraProjection::Perspective {
                        yfov_radians: perspective.yfov(),
                        aspect_ratio: perspective.aspect_ratio(),
                        znear: perspective.znear(),
                        zfar: perspective.zfar(),
                    }
                }
            }
        })?;
        let mut camera_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        let name = match camera.name() {
            Some(name) if name.len() > ctx.limits().max_string_bytes => {
                push_warning(
                    ctx,
                    "gltf.camera_name_limit",
                    "camera name exceeded max_string_bytes and was ignored",
                )?;
                None
            }
            name => optional_name(ctx, &mut camera_storage, name)?,
        };
        builder.add_camera(Camera {
            name,
            projection,
            ..Camera::default()
        })?;
        camera_storage.adopt(builder)?;
    }
    Ok(())
}

fn add_unsupported_domain_diagnostics(ctx: &ParseScope<'_>, gltf: &gltf::Gltf) -> Result<()> {
    if safe_gltf(ctx, "glTF animation presence", || {
        gltf.animations().next().is_some()
    })? {
        push_warning(
            ctx,
            "gltf.animations_ignored",
            "animation channels are not imported by the glTF MVP",
        )?;
    }
    Ok(())
}

fn map_topology(mode: Mode) -> Option<PrimitiveTopology> {
    match mode {
        Mode::Points => Some(PrimitiveTopology::Points),
        Mode::Lines => Some(PrimitiveTopology::Lines),
        Mode::Triangles => Some(PrimitiveTopology::Triangles),
        Mode::LineLoop | Mode::LineStrip | Mode::TriangleStrip | Mode::TriangleFan => None,
    }
}

fn compute_bounds(ctx: &ParseScope<'_>, positions: &[Vec3]) -> Result<Option<Aabb>> {
    let Some(first) = positions.first().copied() else {
        return Ok(None);
    };
    let mut min = first;
    let mut max = first;
    for (index, position) in positions.iter().copied().skip(1).enumerate() {
        poll_parser_loop(ctx, index)?;
        min.x = min.x.min(position.x);
        min.y = min.y.min(position.y);
        min.z = min.z.min(position.z);
        max.x = max.x.max(position.x);
        max.y = max.y.max(position.y);
        max.z = max.z.max(position.z);
    }
    Ok(Some(Aabb { min, max }))
}

fn map_alpha_mode(mode: gltf::material::AlphaMode) -> AlphaMode {
    match mode {
        gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
        gltf::material::AlphaMode::Mask => AlphaMode::Mask,
        gltf::material::AlphaMode::Blend => AlphaMode::Blend,
    }
}

fn map_sampler(sampler: gltf::texture::Sampler<'_>) -> TextureSampler {
    TextureSampler {
        mag_filter: sampler.mag_filter().map(map_mag_filter),
        min_filter: sampler.min_filter().map(map_min_filter),
        wrap_s: map_wrap(sampler.wrap_s()),
        wrap_t: map_wrap(sampler.wrap_t()),
        wrap_r: TextureWrapMode::Repeat,
    }
}

fn map_mag_filter(filter: MagFilter) -> TextureFilterMode {
    match filter {
        MagFilter::Nearest => TextureFilterMode::Nearest,
        MagFilter::Linear => TextureFilterMode::Linear,
    }
}

fn map_min_filter(filter: MinFilter) -> TextureFilterMode {
    match filter {
        MinFilter::Nearest => TextureFilterMode::Nearest,
        MinFilter::Linear => TextureFilterMode::Linear,
        MinFilter::NearestMipmapNearest => TextureFilterMode::NearestMipmapNearest,
        MinFilter::LinearMipmapNearest => TextureFilterMode::LinearMipmapNearest,
        MinFilter::NearestMipmapLinear => TextureFilterMode::NearestMipmapLinear,
        MinFilter::LinearMipmapLinear => TextureFilterMode::LinearMipmapLinear,
    }
}

fn map_wrap(wrap: WrappingMode) -> TextureWrapMode {
    match wrap {
        WrappingMode::ClampToEdge => TextureWrapMode::ClampToEdge,
        WrappingMode::MirroredRepeat => TextureWrapMode::MirroredRepeat,
        WrappingMode::Repeat => TextureWrapMode::Repeat,
    }
}

fn primitive_name<'scope>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    mesh_name: Option<&str>,
    primitive_index: usize,
) -> Result<Option<String>> {
    let Some(mesh_name) = mesh_name else {
        return Ok(None);
    };
    let name_len = primitive_name_len(ctx, mesh_name, primitive_index)?;
    let mut name = String::new();
    storage.reserve_string(ctx, &mut name, name_len, "gltf-primitive-name")?;
    let mut next_poll = 0usize;
    for (byte_index, character) in mesh_name.char_indices() {
        if byte_index >= next_poll {
            ctx.poll()?;
            next_poll = byte_index.saturating_add(PARSER_POLL_INTERVAL);
        }
        name.push(character);
    }
    if primitive_index != 0 {
        use std::fmt::Write as _;
        write!(&mut name, "#{primitive_index}")
            .map_err(|_| ctx.allocation_failure("gltf-primitive-name"))?;
    }
    debug_assert_eq!(name.len(), name_len);
    Ok(Some(name))
}

fn primitive_name_len(
    ctx: &ParseScope<'_>,
    mesh_name: &str,
    primitive_index: usize,
) -> Result<usize> {
    let suffix_len = if primitive_index == 0 {
        0
    } else {
        1 + decimal_digits(primitive_index)
    };
    checked_add_with_limit(
        ctx,
        "max_string_bytes",
        mesh_name.len(),
        suffix_len,
        ctx.limits().max_string_bytes,
    )
}

fn decimal_digits(mut value: usize) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

fn optional_name<'scope>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    value: Option<&str>,
) -> Result<Option<String>> {
    value
        .map(|value| {
            enforce_usize_limit(
                ctx,
                "max_string_bytes",
                value.len(),
                ctx.limits().max_string_bytes,
            )?;
            Ok(storage.copy_string(ctx, value, "gltf-name")?)
        })
        .transpose()
}

fn is_data_uri(uri: &str) -> bool {
    uri.get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
}

fn vec2(value: [f32; 2]) -> Vec2 {
    Vec2::new(value[0], value[1])
}

fn vec3(value: [f32; 3]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

fn vec4(value: [f32; 4]) -> Vec4 {
    Vec4::new(value[0], value[1], value[2], value[3])
}

fn color(value: [f32; 4]) -> Color {
    Color::linear_rgba(value[0], value[1], value[2], value[3])
}

fn push_warning(
    ctx: &ParseScope<'_>,
    code: &'static str,
    message: impl fmt::Display,
) -> Result<()> {
    let code = DiagnosticCode::new(code)
        .map_err(|_| GltfFailure::internal("static glTF diagnostic code is invalid"))?;
    ctx.emit_diagnostic(DiagnosticSeverity::Warning, code, format_args!("{message}"))?;
    Ok(())
}
