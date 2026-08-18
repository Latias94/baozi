use crate::{FORMAT_ID, GltfFailure, external_buffer_role, external_image_role};
use baozi_core::__canonical_import::{ResourceSnapshotAuthoring, SceneDraft};
use baozi_import::{
    AlphaMode, AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain,
    AttributeSemantic, AuthoredTransform, AuthoringRange, CameraAuthoring, CameraLocalId,
    CameraProjection, Color, ColorInterpretation, DeformationAuthoringBatch, DependencySnapshot,
    DiagnosticCode, DiagnosticSeverity, EntityName, ExtensionMediaType, ExtensionValueRef,
    FormatId, GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    ImageAuthoring, ImageLocalId, ImportStorageClass, ImportWorkClass, ImportWorkScope,
    InfluenceAuthoring, InfluenceRowsAuthoring, LineageAuthoringBatch, LogicalMeshLocalId,
    LogicalMorphAuthoring, Mat4, MaterialAuthoring, MaterialBindingAuthoring, MaterialFactors,
    MaterialLocalId, MeshInstanceDeformationAuthoring, MetadataAuthoringBatch,
    MetadataAuthoringEntry, MeteredOrderedMap, MeteredString, MeteredVec, MipmapFilter,
    NamespacedName, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent,
    ObservedResourceAuthoring, OwnerProvenanceAuthoring, ParseFailure, ParseResult, ParseScope,
    PbrMetallicRoughnessFactors, Presence, PrimitiveDeformationAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentity,
    ResourceIdentityLocalId, SamplerAuthoring, SamplerLocalId, SamplerState, SceneAuthoringBatch,
    SceneSpace, SessionOpaqueReason, ShadingModel, SkinAuthoring, SkinLocalId, SkinMatrixSpaces,
    SourceIdentityAuthoring, SourceIdentityLocalId, SourceObjectKind, SourceObjectLocator,
    SpatialAuthoringBatch, TexelFilter, TextureAddressMode, TextureAuthoring,
    TextureBlendOperation, TextureChannelMapping, TextureChannelSwizzle, TextureCoordinateSource,
    TextureLocalId, TextureRole, TextureSourceKey, TextureStrength, TextureTransform, Vec3,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::{DecodeError, DecodeSliceError, Engine as _};
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
        }
    }
}

impl fmt::Display for ContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scope(error) => error.fmt(formatter),
            Self::Format(error) => error.fmt(formatter),
            Self::Parse { message } => formatter.write_str(message),
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

fn authoring_u32(ctx: &ParseScope<'_>, value: usize, resource: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| ctx.allocation_failure(resource).into())
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
    let glb_candidate = bytes.get(..4) == Some(b"glTF");
    if glb_candidate {
        if let Some(json) = glb_json_surface(bytes) {
            preflight_json_utf8(ctx, json, GLB_JSON_OFFSET as u64)?;
        }
    } else {
        preflight_json_utf8(ctx, bytes, 0)?;
    }
    let (parsed, _document_allowance) = ctx.with_work(
        ImportWorkClass::DecodeWork,
        primary_len,
        |work| -> Result<_> {
            let backend_allowance = gltf_backend_allocation_allowance(ctx, bytes)?;
            let document_allowance =
                ctx.metered_opaque_allowance(ImportStorageClass::BackendDecode, backend_allowance)?;
            let parsed = safe_gltf(ctx, "glTF document parse", || gltf::Gltf::from_slice(bytes))?;
            work.complete_all()?;
            Ok((parsed, document_allowance))
        },
    )?;
    let gltf = parsed
        .map_err(|error| GltfFailure::invalid_document(format_args!("invalid glTF: {error}")))?;
    let node_graph = preflight_node_graph(ctx, &gltf)?;
    let repeated_mesh_string_bytes = preflight_canonical_base_strings(ctx, &gltf, &node_graph)?;
    let resolved_texture_uris = preflight_texture_strings(ctx, &gltf, repeated_mesh_string_bytes)?;
    let material_count = safe_gltf(ctx, "glTF material count", || gltf.materials().count())?;
    let geometry_contracts = preflight_geometry_contracts(ctx, &gltf, material_count)?;
    let buffers = load_buffers(ctx, &gltf)?;
    scene_from_document(
        ctx,
        &gltf,
        buffers.as_slice(),
        &node_graph,
        &geometry_contracts,
        &resolved_texture_uris,
    )
}

fn preflight_json_utf8(ctx: &ParseScope<'_>, bytes: &[u8], raw_base: u64) -> Result<()> {
    ctx.enforce_resource_limit(
        "gltf-decoded-utf8-bytes",
        to_u64(bytes.len()),
        ctx.limits().max_decoded_utf8_bytes,
    )?;
    ctx.with_work(
        ImportWorkClass::DecodeWork,
        to_u64(bytes.len()),
        |work| -> Result<()> {
            if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
                work.complete([3])?;
                return Err(GltfFailure::forbidden_bom(raw_base).into());
            }

            let mut window_start = 0usize;
            while window_start < bytes.len() {
                work.poll()?;
                let window_end = window_start
                    .saturating_add(PARSER_POLL_INTERVAL)
                    .min(bytes.len());
                let Err(source) = std::str::from_utf8(&bytes[window_start..window_end]) else {
                    work.complete([to_u64(window_end - window_start)])?;
                    window_start = window_end;
                    continue;
                };
                let valid_prefix = source.valid_up_to();
                if valid_prefix != 0 {
                    work.complete([to_u64(valid_prefix)])?;
                }
                let invalid_start = window_start
                    .checked_add(valid_prefix)
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
            Ok(())
        },
    )
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
                enforce_usize_limit(
                    ctx,
                    "max_string_bytes",
                    name.len(),
                    ctx.limits().max_string_bytes,
                )?;
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
        }

        let mut visited = try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            graph.nodes.len(),
            false,
            "gltf-canonical-string-node-membership",
        )?;
        let skin_count = gltf.skins().len();
        let mut referenced_skins = try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            skin_count,
            false,
            "gltf-canonical-string-skin-membership",
        )?;
        let mut pending = try_vec_capacity(
            ctx,
            ImportStorageClass::FormatNormalization,
            graph.nodes.len(),
            "gltf-canonical-string-node-stack",
        )?;
        for (root_ordinal, root) in graph.roots.as_slice().iter().rev().copied().enumerate() {
            poll_parser_loop(ctx, root_ordinal)?;
            pending.try_push_reserved(root)?;
        }
        let mut selected_count = 0usize;
        while let Some(node_index) = pending.pop() {
            poll_parser_loop(ctx, selected_count)?;
            selected_count += 1;
            let Some(node) = graph.nodes.as_slice().get(node_index) else {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF node graph references missing node {node_index}"),
                ));
            };
            let Some(seen) = visited.as_mut_slice().get_mut(node_index) else {
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
                let Some(referenced) = referenced_skins.as_mut_slice().get_mut(skin_index) else {
                    return Err(gltf_parse_error(
                        ctx,
                        format!("glTF node {node_index} references skin {skin_index} out of range"),
                    ));
                };
                *referenced = true;
            }
            for (child_ordinal, child) in graph.children.as_slice()[node_index]
                .as_slice()
                .iter()
                .rev()
                .copied()
                .enumerate()
            {
                poll_parser_loop(ctx, child_ordinal)?;
                pending.try_push_reserved(child)?;
            }
        }

        for (skin_index, skin) in gltf.skins().enumerate() {
            poll_parser_loop(ctx, skin_index)?;
            if referenced_skins.as_slice().get(skin_index).copied() != Some(true) {
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
    values: MeteredOrderedMap<'scope, usize, MeteredString<'scope>>,
}

impl ResolvedTextureUris<'_> {
    fn get(&self, image_index: &usize) -> Option<&str> {
        self.values.get(image_index).map(MeteredString::as_str)
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
        let image_count = safe_gltf(ctx, "glTF image count", || gltf.images().len())?;
        let mut resolved_uris = ctx.metered_ordered_map(
            ImportStorageClass::FormatNormalization,
            "gltf-resolved-texture-uris",
        );

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
                    image_count,
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
                    image_count,
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
                    image_count,
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
                    image_count,
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
                    image_count,
                )?;
            }
        }

        Ok(ResolvedTextureUris {
            values: resolved_uris,
        })
    })?
}

fn preflight_texture_reference<'scope>(
    ctx: &'scope ParseScope<'_>,
    bytes: &mut usize,
    limit: usize,
    texture: gltf::texture::Texture<'_>,
    source_key: &'static str,
    resolved_uris: &mut MeteredOrderedMap<'scope, usize, MeteredString<'scope>>,
    image_count: usize,
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
    if image_index >= image_count {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF texture references missing image {image_index}"),
        ));
    }
    if !resolved_uris.contains_key(&image_index) {
        let resolution_bytes = texture_path_resolution_allowance(ctx, uri)?;
        let _resolution_allowance = ctx
            .metered_opaque_allowance(ImportStorageClass::FormatNormalization, resolution_bytes)?;
        let resolved = ctx.with_work(
            ImportWorkClass::WorkUnits,
            to_u64(ctx.source().as_str().len().saturating_add(uri.len())),
            |work| -> Result<_> {
                let resolved = ctx.source().resolve_from_file(uri).map_err(|error| {
                    GltfFailure::invalid_reference("texture-uri", Some(image_index), error)
                })?;
                work.complete_all()?;
                Ok(resolved)
            },
        )?;
        let mut resolved_uri = ctx.metered_string(
            ImportStorageClass::FormatNormalization,
            "gltf-resolved-texture-uri",
        );
        resolved_uri.try_push_str(resolved.as_str())?;
        resolved_uris.try_insert(image_index, resolved_uri)?;
    }
    let resolved_uri = resolved_uris.get(&image_index).ok_or_else(|| {
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

fn complete_canonical_string_work(ctx: &ParseScope<'_>, bytes: usize) -> Result<()> {
    if bytes == 0 {
        return Ok(());
    }
    ctx.with_work(
        ImportWorkClass::CanonicalStringBytes,
        to_u64(bytes),
        |work| -> Result<()> {
            work.complete_all()?;
            Ok(())
        },
    )
}

struct BufferData<'scope> {
    bytes: MeteredVec<'scope, u8>,
}

impl BufferData<'_> {
    fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

struct BufferSet<'scope> {
    values: MeteredVec<'scope, BufferData<'scope>>,
}

impl BufferSet<'_> {
    fn as_slice(&self) -> &[BufferData<'_>] {
        self.values.as_slice()
    }
}

fn load_buffers<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Gltf,
) -> Result<BufferSet<'scope>> {
    let buffer_count = safe_gltf(ctx, "glTF buffer count", || gltf.buffers().count())?;
    let mut buffers = ctx.metered_vec(ImportStorageClass::BackendDecode, "gltf-buffer-list");
    buffers.try_reserve(buffer_count)?;
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
        buffers.try_push_reserved(loaded)?;
    }
    Ok(BufferSet { values: buffers })
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
    let decoded_capacity = base64::decoded_len_estimate(encoded.len());
    let decoded = ctx.with_work_group(
        [
            (ImportWorkClass::DataUriBytes, decoded_len),
            (
                ImportWorkClass::DecodeWork,
                u64::try_from(encoded.len()).unwrap_or(u64::MAX),
            ),
        ],
        |work| -> Result<_> {
            let mut decoded =
                ctx.metered_vec(ImportStorageClass::BackendDecode, "gltf-data-uri-buffer");
            decoded.try_reserve(decoded_capacity)?;
            let mut written = 0usize;
            for (chunk_index, chunk) in encoded.as_bytes().chunks(PARSER_POLL_INTERVAL).enumerate()
            {
                work.poll()?;
                let chunk_start = chunk_index * PARSER_POLL_INTERVAL;
                let chunk_capacity = base64::decoded_len_estimate(chunk.len());
                let chunk_end = written
                    .checked_add(chunk_capacity)
                    .ok_or_else(|| ctx.allocation_failure("gltf-data-uri-buffer"))?;
                decoded.try_resize(chunk_end, 0_u8)?;
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    BASE64_STANDARD
                        .decode_slice(chunk, &mut decoded.as_mut_slice()[written..chunk_end])
                }));
                let chunk_written = finish_gltf_operation("glTF base64 buffer decode", outcome)?
                    .map_err(|error| match error {
                        DecodeSliceError::DecodeError(error) => {
                            let error = offset_base64_decode_error(error, chunk_start);
                            GltfFailure::invalid_reference(
                                "data-uri-buffer",
                                Some(buffer_index),
                                format_args!("invalid base64: {error}"),
                            )
                        }
                        DecodeSliceError::OutputSliceTooSmall => {
                            GltfFailure::internal("base64 decoder exceeded its preflight capacity")
                        }
                    })?;
                written = written
                    .checked_add(chunk_written)
                    .ok_or_else(|| ctx.allocation_failure("gltf-data-uri-buffer"))?;
                decoded.truncate(written);
                work.complete([to_u64(chunk_written), to_u64(chunk.len())])?;
            }
            Ok(decoded)
        },
    )?;
    Ok(BufferData { bytes: decoded })
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
    let origin = ctx.root_origin();
    let outcome = ctx.dependency(&origin, &external_buffer_role(), uri)?;
    let snapshot = ctx.into_available_dependency(outcome)?;
    copy_buffer(ctx, snapshot.bytes(), "gltf-external-buffer-copy")
}

fn copy_buffer<'scope>(
    ctx: &'scope ParseScope<'_>,
    source: &[u8],
    allocation: &'static str,
) -> Result<BufferData<'scope>> {
    let copied = ctx.with_work(
        ImportWorkClass::DecodeWork,
        to_u64(source.len()),
        |work| -> Result<_> {
            let mut copied = ctx.metered_vec(ImportStorageClass::BackendDecode, allocation);
            copied.try_reserve(source.len())?;
            for chunk in source.chunks(PARSER_POLL_INTERVAL) {
                work.poll()?;
                copied.try_extend_from_slice(chunk)?;
                work.complete([to_u64(chunk.len())])?;
            }
            Ok(copied)
        },
    )?;
    Ok(BufferData { bytes: copied })
}

fn gltf_parse_error(ctx: &ParseScope<'_>, message: impl Into<String>) -> ContentError {
    ContentError::parse(ctx.source().to_string(), None, message)
}

fn scene_from_document(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Gltf,
    buffers: &[BufferData<'_>],
    node_graph: &NodeGraph<'_, '_>,
    geometry_contracts: &GeometryContracts<'_>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
) -> Result<SceneDraft> {
    let external_images = load_external_image_resources(ctx, gltf, resolved_texture_uris)?;
    let appearance_storage =
        build_appearance_storage(ctx, gltf, resolved_texture_uris, &external_images)?;
    let geometry_storage = build_geometry_storage(ctx, gltf, buffers, geometry_contracts)?;
    let cameras = build_cameras(ctx, gltf)?;
    add_unsupported_domain_diagnostics(ctx, gltf)?;
    let node_storage = build_scene_nodes(ctx, node_graph, geometry_storage.logical_meshes.len())?;
    let skin_storage = build_skins(
        ctx,
        gltf,
        buffers,
        node_storage.node_ids.as_slice(),
        node_storage.skin_references.as_slice(),
    )?;
    let mesh_instance_deformations = build_mesh_instance_deformations(
        ctx,
        node_storage.mesh_instances.as_slice(),
        skin_storage.skin_ids.as_slice(),
    )?;
    submit_complete_batch(
        ctx,
        &appearance_storage,
        &geometry_storage,
        &cameras,
        &node_storage,
        &skin_storage,
        &mesh_instance_deformations,
        &external_images,
    )
}

struct ExternalImageResource<'scope> {
    image_index: usize,
    identity: ResourceIdentity,
    snapshot: DependencySnapshot<'scope>,
}

struct ExternalImageResources<'scope> {
    values: MeteredVec<'scope, ExternalImageResource<'scope>>,
}

impl ExternalImageResources<'_> {
    fn get(&self, image_index: usize) -> Option<(u32, &DependencySnapshot<'_>)> {
        self.values
            .as_slice()
            .iter()
            .enumerate()
            .find(|(_, resource)| resource.image_index == image_index)
            .and_then(|(index, resource)| {
                let index = u32::try_from(index).ok()?.checked_add(1)?;
                Some((index, &resource.snapshot))
            })
    }
}

fn load_external_image_resources<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Gltf,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
) -> Result<ExternalImageResources<'scope>> {
    let mut values = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-external-image-resources",
    );
    values.try_reserve(resolved_texture_uris.values.len())?;
    for (ordinal, image) in gltf.images().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        let ImageSource::Uri { uri, .. } = image.source() else {
            continue;
        };
        if resolved_texture_uris.get(&image.index()).is_none() {
            continue;
        }
        let origin = ctx.root_origin();
        let outcome = ctx.dependency(&origin, &external_image_role(), uri)?;
        let snapshot = ctx.into_available_dependency(outcome)?;
        let identity =
            ResourceIdentity::new(snapshot.identity().path().as_str()).map_err(|_| {
                GltfFailure::invalid_reference(
                    "texture-uri",
                    Some(image.index()),
                    "resolved image identity is not canonical",
                )
            })?;
        values.try_push_reserved(ExternalImageResource {
            image_index: image.index(),
            identity,
            snapshot,
        })?;
    }
    Ok(ExternalImageResources { values })
}

struct ImageStorage<'scope> {
    resource: ResourceIdentityLocalId,
    resource_index: u32,
    media_type: Option<ExtensionMediaType>,
    texture_name: Option<MeteredString<'scope>>,
}

struct BindingStorage {
    texture: TextureLocalId,
    role: TextureRole,
    coordinate_source: TextureCoordinateSource,
    channel_mapping: TextureChannelMapping,
    color_interpretation: ColorInterpretation,
    strength: TextureStrength,
    blend: TextureBlendOperation,
    source_key: TextureSourceKey,
}

struct MaterialStorage<'document> {
    name: Option<&'document str>,
    bindings: AuthoringRange,
    alpha_mode: AlphaMode,
    double_sided: bool,
    factors: MaterialFactors,
}

struct AppearanceStorage<'scope, 'document> {
    images: MeteredVec<'scope, ImageStorage<'scope>>,
    samplers: MeteredVec<'scope, SamplerAuthoring>,
    textures: MeteredVec<'scope, TextureAuthoring>,
    bindings: MeteredVec<'scope, BindingStorage>,
    materials: MeteredVec<'scope, MaterialStorage<'document>>,
}

fn build_appearance_storage<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Gltf,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
) -> Result<AppearanceStorage<'scope, 'document>> {
    let material_count = safe_gltf(ctx, "glTF material count", || gltf.materials().count())?;
    let mut storage = AppearanceStorage {
        images: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-images",
        ),
        samplers: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-samplers",
        ),
        textures: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-textures",
        ),
        bindings: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-material-bindings",
        ),
        materials: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-materials",
        ),
    };
    storage.materials.try_reserve(material_count)?;
    for (material_index, material) in gltf.materials().enumerate() {
        poll_parser_loop(ctx, material_index)?;
        add_material_authoring(
            ctx,
            &mut storage,
            material,
            resolved_texture_uris,
            external_images,
        )?;
    }
    Ok(storage)
}

fn add_material_authoring<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut AppearanceStorage<'scope, 'document>,
    material: gltf::Material<'document>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
) -> Result<()> {
    let pbr = material.pbr_metallic_roughness();
    let base = pbr.base_color_factor();
    let emissive = material.emissive_factor();
    complete_canonical_string_work(ctx, material.name().map_or(0, str::len))?;
    let binding_offset =
        authoring_u32(ctx, storage.bindings.len(), "gltf-material-binding-offset")?;

    if let Some(info) = pbr.base_color_texture() {
        add_texture_binding(
            ctx,
            storage,
            resolved_texture_uris,
            external_images,
            info.texture(),
            info.tex_coord(),
            TextureRole::BaseColor,
            TextureChannelSwizzle::RGBA,
            ColorInterpretation::SrgbColor,
            1.0,
            "baseColorTexture",
        )?;
    }
    if let Some(info) = pbr.metallic_roughness_texture() {
        add_texture_binding(
            ctx,
            storage,
            resolved_texture_uris,
            external_images,
            info.texture(),
            info.tex_coord(),
            TextureRole::MetallicRoughness,
            TextureChannelSwizzle::RGBA,
            ColorInterpretation::Data,
            1.0,
            "metallicRoughnessTexture",
        )?;
    }
    if let Some(info) = material.normal_texture() {
        add_texture_binding(
            ctx,
            storage,
            resolved_texture_uris,
            external_images,
            info.texture(),
            info.tex_coord(),
            TextureRole::Normal,
            TextureChannelSwizzle::RGB,
            ColorInterpretation::Data,
            info.scale(),
            "normalTexture",
        )?;
    }
    if let Some(info) = material.occlusion_texture() {
        add_texture_binding(
            ctx,
            storage,
            resolved_texture_uris,
            external_images,
            info.texture(),
            info.tex_coord(),
            TextureRole::Occlusion,
            TextureChannelSwizzle::RED,
            ColorInterpretation::Data,
            info.strength(),
            "occlusionTexture",
        )?;
    }
    if let Some(info) = material.emissive_texture() {
        add_texture_binding(
            ctx,
            storage,
            resolved_texture_uris,
            external_images,
            info.texture(),
            info.tex_coord(),
            TextureRole::Emissive,
            TextureChannelSwizzle::RGB,
            ColorInterpretation::SrgbColor,
            1.0,
            "emissiveTexture",
        )?;
    }

    let binding_count = storage
        .bindings
        .len()
        .checked_sub(binding_offset as usize)
        .ok_or_else(|| ctx.allocation_failure("gltf-material-binding-range"))?;
    let pbr_factors = PbrMetallicRoughnessFactors::try_new(
        Color::linear_rgba(base[0], base[1], base[2], base[3]),
        pbr.metallic_factor(),
        pbr.roughness_factor(),
    )
    .map_err(|_| gltf_parse_error(ctx, "glTF material factors are outside canonical bounds"))?;
    let factors = MaterialFactors::try_new(
        Vec3::new(emissive[0], emissive[1], emissive[2]),
        Some(pbr_factors),
        None,
        None,
    )
    .map_err(|_| gltf_parse_error(ctx, "glTF material factors are outside canonical bounds"))?;
    storage.materials.try_push_reserved(MaterialStorage {
        name: material.name(),
        bindings: AuthoringRange::new(
            binding_offset,
            authoring_u32(ctx, binding_count, "gltf-material-binding-count")?,
        ),
        alpha_mode: map_alpha_mode(ctx, &material)?,
        double_sided: material.double_sided(),
        factors,
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_texture_binding<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut AppearanceStorage<'scope, 'document>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    texture: gltf::texture::Texture<'document>,
    tex_coord: u32,
    role: TextureRole,
    channel_swizzle: TextureChannelSwizzle,
    color_interpretation: ColorInterpretation,
    scale: f32,
    source_key: &'static str,
) -> Result<()> {
    let image = texture.source();
    let image_name = image.name();
    let source = match image.source() {
        ImageSource::Uri { uri, .. } if is_data_uri(uri) => {
            push_warning(
                ctx,
                "gltf.texture_data_uri_ignored",
                format_args!("texture data URI for {source_key} was ignored"),
            )?;
            return Ok(());
        }
        ImageSource::Uri { uri, mime_type } => {
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
            let (resource_index, _) = external_images.get(image_index).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format!("glTF texture resource is missing image {image_index}"),
                )
            })?;
            let media_type = mime_type
                .map(ExtensionMediaType::new)
                .transpose()
                .map_err(|_| {
                    gltf_parse_error(
                        ctx,
                        format!("glTF image {image_index} has an invalid media type"),
                    )
                })?;
            let canonical_string_bytes = resolved
                .len()
                .checked_add(source_key.len())
                .and_then(|bytes| {
                    bytes.checked_add(texture.name().or(image_name).map_or(0, str::len))
                })
                .ok_or_else(|| ctx.allocation_failure("gltf-texture-canonical-strings"))?;
            complete_canonical_string_work(ctx, canonical_string_bytes)?;
            (
                ResourceIdentityLocalId::new(resource_index),
                resource_index,
                media_type,
            )
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

    let image_id = ImageLocalId::new(authoring_u32(
        ctx,
        storage.images.len(),
        "gltf-authoring-image-id",
    )?);
    let texture_name = match texture.name().or(image_name) {
        Some(name) => {
            let mut owned = ctx.metered_string(
                ImportStorageClass::SceneConstruction,
                "gltf-authoring-texture-name",
            );
            owned.try_push_str(name)?;
            Some(owned)
        }
        None => None,
    };
    storage.images.try_push(ImageStorage {
        resource: source.0,
        resource_index: source.1,
        media_type: source.2,
        texture_name,
    })?;
    let sampler_id = SamplerLocalId::new(authoring_u32(
        ctx,
        storage.samplers.len(),
        "gltf-authoring-sampler-id",
    )?);
    storage
        .samplers
        .try_push(SamplerAuthoring::new(map_sampler(texture.sampler())))?;
    let texture_id = TextureLocalId::new(authoring_u32(
        ctx,
        storage.textures.len(),
        "gltf-authoring-texture-id",
    )?);
    storage
        .textures
        .try_push(TextureAuthoring::new(image_id, sampler_id))?;
    let coordinate_source = TextureCoordinateSource::uv_set(tex_coord)
        .map_err(|_| gltf_parse_error(ctx, "glTF texture coordinate set is out of range"))?;
    let strength = TextureStrength::try_new(scale)
        .map_err(|_| gltf_parse_error(ctx, "glTF texture strength is not finite"))?;
    let source_key = TextureSourceKey::try_new(source_key)
        .map_err(|_| gltf_parse_error(ctx, "static glTF texture source key is invalid"))?;
    storage.bindings.try_push(BindingStorage {
        texture: texture_id,
        role,
        coordinate_source,
        channel_mapping: TextureChannelMapping::swizzle(channel_swizzle),
        color_interpretation,
        strength,
        blend: TextureBlendOperation::Replace,
        source_key,
    })?;
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

struct MeshContracts<'scope> {
    primitives: MeteredVec<'scope, PrimitiveContract>,
}

struct GeometryContracts<'scope> {
    meshes: MeteredVec<'scope, MeshContracts<'scope>>,
}

fn preflight_geometry_contracts<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Gltf,
    material_count: usize,
) -> Result<GeometryContracts<'scope>> {
    let mesh_count = safe_gltf(ctx, "glTF mesh count", || gltf.meshes().count())?;
    enforce_usize_limit(ctx, "max_meshes", mesh_count, ctx.limits().max_meshes)?;
    let mut meshes = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-geometry-contract-meshes",
    );
    meshes.try_reserve(mesh_count)?;

    let mut total_primitives = 0usize;
    let mut total_vertices = 0usize;
    let mut total_faces = 0usize;
    let mut total_topology_cells = 0usize;
    let mut total_attribute_streams = 0usize;
    let mut total_attribute_cells = 0usize;
    for (mesh_ordinal, mesh) in gltf.meshes().enumerate() {
        poll_parser_loop(ctx, mesh_ordinal)?;
        let primitive_count = safe_gltf(ctx, "glTF primitive count", || mesh.primitives().len())?;
        total_primitives = checked_add_with_limit(
            ctx,
            "max_primitives",
            total_primitives,
            primitive_count,
            ctx.limits().max_primitives,
        )?;
        let mut primitives = ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-geometry-contract-primitives",
        );
        primitives.try_reserve(primitive_count)?;
        for (primitive_ordinal, primitive) in mesh.primitives().enumerate() {
            poll_parser_loop(ctx, primitive_ordinal)?;
            let contract = validate_primitive_contract(
                ctx,
                gltf,
                mesh.index(),
                primitive.index(),
                material_count,
            )?;
            if map_topology(contract.mode).is_none() {
                return Err(ContentError::FeatureUnsupported {
                    format: "gltf",
                    feature: format!("primitive mode {:?} is not implemented yet", contract.mode),
                });
            }
            total_vertices = checked_add_with_limit(
                ctx,
                "max_vertices",
                total_vertices,
                contract.position_count,
                ctx.limits().max_vertices,
            )?;
            total_faces = checked_add_with_limit(
                ctx,
                "max_faces",
                total_faces,
                contract.face_count,
                ctx.limits().max_faces,
            )?;
            total_topology_cells = checked_add_with_limit(
                ctx,
                "max_total_topology_cells",
                total_topology_cells,
                contract.index_count,
                ctx.limits().max_total_topology_cells,
            )?;
            total_attribute_streams = checked_add_with_limit(
                ctx,
                "max_total_vertex_attribute_streams",
                total_attribute_streams,
                contract.attribute_stream_count,
                ctx.limits().max_total_vertex_attribute_streams,
            )?;
            let attribute_cells = contract
                .position_count
                .checked_mul(contract.attribute_stream_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-attribute-cell-counter"))?;
            total_attribute_cells = checked_add_with_limit(
                ctx,
                "max_total_vertex_attribute_cells",
                total_attribute_cells,
                attribute_cells,
                ctx.limits().max_total_vertex_attribute_cells,
            )?;
            primitives.try_push_reserved(contract)?;
        }
        meshes.try_push_reserved(MeshContracts { primitives })?;
    }
    Ok(GeometryContracts { meshes })
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
    class: ImportStorageClass,
    expected: usize,
    allocation: &'static str,
    values: impl Iterator<Item = T>,
) -> Result<MeteredVec<'scope, T>> {
    let mut collected = ctx.metered_vec(class, allocation);
    collected.try_reserve(expected)?;
    for value in values {
        poll_parser_loop(ctx, collected.len())?;
        if collected.len() == expected {
            return Err(gltf_parse_error(
                ctx,
                format!("{allocation} produced more than {expected} values"),
            ));
        }
        collected.try_push_reserved(value)?;
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

fn collect_exact_with_work<'scope, T, const N: usize>(
    ctx: &'scope ParseScope<'_>,
    work: &mut ImportWorkScope<'_, '_, N>,
    expected: usize,
    allocation: &'static str,
    operation: &'static str,
    mut values: impl Iterator<Item = T>,
    per_item: [u64; N],
) -> Result<MeteredVec<'scope, T>> {
    let mut collected = ctx.metered_vec(ImportStorageClass::FormatNormalization, allocation);
    collected.try_reserve(expected)?;
    loop {
        if collected.len().is_multiple_of(PARSER_POLL_INTERVAL) {
            work.poll()?;
        }
        let next =
            finish_gltf_operation(operation, catch_unwind(AssertUnwindSafe(|| values.next())))?;
        let Some(value) = next else {
            break;
        };
        if collected.len() == expected {
            return Err(gltf_parse_error(
                ctx,
                format!("{allocation} produced more than {expected} values"),
            ));
        }
        collected.try_push_reserved(value)?;
        work.complete(per_item)?;
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
    class: ImportStorageClass,
    capacity: usize,
    allocation: &'static str,
) -> Result<MeteredVec<'scope, T>> {
    let mut values = ctx.metered_vec(class, allocation);
    values.try_reserve(capacity)?;
    Ok(values)
}

fn try_vec_filled<'scope, T: Copy>(
    ctx: &'scope ParseScope<'_>,
    class: ImportStorageClass,
    length: usize,
    value: T,
    allocation: &'static str,
) -> Result<MeteredVec<'scope, T>> {
    let mut values = try_vec_capacity(ctx, class, length, allocation)?;
    values.try_resize(length, value)?;
    Ok(values)
}

struct AttributeStorage<'scope> {
    semantic: AttributeSemantic,
    lanes: u8,
    values: MeteredVec<'scope, f32>,
}

struct PrimitiveStorage<'scope> {
    topology: GeometryTopologyAuthoring<'static>,
    has_indices: bool,
    indices: MeteredVec<'scope, u32>,
    material: Option<MaterialLocalId>,
    attributes: MeteredVec<'scope, AttributeStorage<'scope>>,
    influence_offsets: MeteredVec<'scope, u32>,
    influences: MeteredVec<'scope, InfluenceAuthoring>,
}

struct LogicalMeshStorage<'scope, 'document> {
    name: Option<&'document str>,
    primitives: MeteredVec<'scope, PrimitiveStorage<'scope>>,
}

struct GeometryStorage<'scope, 'document> {
    logical_meshes: MeteredVec<'scope, LogicalMeshStorage<'scope, 'document>>,
}

fn build_geometry_storage<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Gltf,
    buffers: &[BufferData<'_>],
    contracts: &GeometryContracts<'_>,
) -> Result<GeometryStorage<'scope, 'document>> {
    let mesh_count = safe_gltf(ctx, "glTF mesh count", || gltf.meshes().count())?;
    if contracts.meshes.len() != mesh_count {
        return Err(gltf_parse_error(
            ctx,
            "glTF geometry contract mesh count changed before materialization",
        ));
    }
    let mut logical_meshes = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-authoring-logical-meshes",
    );
    logical_meshes.try_reserve(mesh_count)?;
    for (mesh_ordinal, mesh) in gltf.meshes().enumerate() {
        poll_parser_loop(ctx, mesh_ordinal)?;
        let mesh_contracts = contracts
            .meshes
            .as_slice()
            .get(mesh_ordinal)
            .ok_or_else(|| {
                gltf_parse_error(ctx, "glTF geometry contract mesh table is incomplete")
            })?;
        let name_bytes = mesh.name().map_or(0, str::len);
        ctx.with_work_group(
            [
                (ImportWorkClass::Meshes, 1),
                (ImportWorkClass::CanonicalStringBytes, to_u64(name_bytes)),
            ],
            |work| -> Result<()> {
                work.complete_all()?;
                Ok(())
            },
        )?;
        let primitive_count = safe_gltf(ctx, "glTF primitive count", || mesh.primitives().len())?;
        if mesh_contracts.primitives.len() != primitive_count {
            return Err(gltf_parse_error(
                ctx,
                "glTF geometry contract primitive count changed before materialization",
            ));
        }
        let mut primitives = ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-primitives-per-logical-mesh",
        );
        primitives.try_reserve(primitive_count)?;
        for (primitive_ordinal, primitive) in mesh.primitives().enumerate() {
            poll_parser_loop(ctx, primitive_ordinal)?;
            let contract = *mesh_contracts
                .primitives
                .as_slice()
                .get(primitive_ordinal)
                .ok_or_else(|| {
                    gltf_parse_error(ctx, "glTF geometry contract primitive table is incomplete")
                })?;
            primitives.try_push_reserved(build_primitive_authoring(
                ctx, buffers, &mesh, primitive, contract,
            )?)?;
        }
        logical_meshes.try_push_reserved(LogicalMeshStorage {
            name: mesh.name(),
            primitives,
        })?;
    }
    Ok(GeometryStorage { logical_meshes })
}

fn build_primitive_authoring<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[BufferData<'_>],
    mesh: &gltf::Mesh<'_>,
    primitive: gltf::Primitive<'_>,
    contract: PrimitiveContract,
) -> Result<PrimitiveStorage<'scope>> {
    let topology = map_topology(contract.mode).ok_or_else(|| ContentError::FeatureUnsupported {
        format: "gltf",
        feature: format!("primitive mode {:?} is not implemented yet", contract.mode),
    })?;

    let attribute_cell_count = contract
        .position_count
        .checked_mul(contract.attribute_stream_count)
        .ok_or_else(|| ctx.allocation_failure("gltf-attribute-cell-counter"))?;
    ctx.with_work_group(
        [
            (ImportWorkClass::Primitives, 1),
            (ImportWorkClass::Vertices, to_u64(contract.position_count)),
            (ImportWorkClass::Faces, to_u64(contract.face_count)),
            (ImportWorkClass::TopologyCells, to_u64(contract.index_count)),
            (
                ImportWorkClass::AttributeStreams,
                to_u64(contract.attribute_stream_count),
            ),
            (
                ImportWorkClass::AttributeCells,
                to_u64(attribute_cell_count),
            ),
        ],
        |work| -> Result<PrimitiveStorage<'scope>> {
            let reader = safe_gltf(ctx, "glTF primitive reader construction", || {
                primitive.reader(|buffer| buffers.get(buffer.index()).map(BufferData::as_slice))
            })?;
            let positions = safe_gltf(ctx, "glTF POSITION reader", || {
                reader.read_positions().map(|positions| {
                    collect_f32_arrays(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.position_count,
                        "gltf-position-values",
                        positions,
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
            let normals = safe_gltf(ctx, "glTF NORMAL reader", || {
                reader.read_normals().map(|values| {
                    collect_f32_arrays(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.position_count,
                        "gltf-normal-values",
                        values,
                    )
                })
            })?
            .transpose()?
            .unwrap_or_else(|| {
                ctx.metered_vec(ImportStorageClass::SceneConstruction, "gltf-normal-values")
            });
            let tangents = safe_gltf(ctx, "glTF TANGENT reader", || {
                reader.read_tangents().map(|values| {
                    collect_f32_arrays(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.position_count,
                        "gltf-tangent-values",
                        values,
                    )
                })
            })?
            .transpose()?
            .unwrap_or_else(|| {
                ctx.metered_vec(ImportStorageClass::SceneConstruction, "gltf-tangent-values")
            });
            let indices = safe_gltf(ctx, "glTF indices reader", || {
                reader.read_indices().map(|indices| {
                    collect_exact(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.index_count,
                        "gltf-index-values",
                        indices.into_u32(),
                    )
                })
            })?
            .transpose()?
            .unwrap_or_else(|| {
                ctx.metered_vec(ImportStorageClass::SceneConstruction, "gltf-index-values")
            });

            let mut attributes = ctx.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-authoring-primitive-attributes",
            );
            attributes.try_push(AttributeStorage {
                semantic: AttributeSemantic::Position,
                lanes: 3,
                values: positions,
            })?;
            if !normals.is_empty() {
                attributes.try_push(AttributeStorage {
                    semantic: AttributeSemantic::Normal,
                    lanes: 3,
                    values: normals,
                })?;
            }
            if !tangents.is_empty() {
                attributes.try_push(AttributeStorage {
                    semantic: AttributeSemantic::Tangent,
                    lanes: 4,
                    values: tangents,
                })?;
            }
            for set in 0..8 {
                let Some(values) = safe_gltf(ctx, "glTF TEXCOORD reader", || {
                    reader.read_tex_coords(set).map(|values| {
                        collect_f32_arrays(
                            ctx,
                            ImportStorageClass::SceneConstruction,
                            contract.position_count,
                            "gltf-texcoord-values",
                            values.into_f32(),
                        )
                    })
                })?
                .transpose()?
                else {
                    break;
                };
                attributes.try_push(AttributeStorage {
                    semantic: AttributeSemantic::Texcoord(set),
                    lanes: 2,
                    values,
                })?;
            }
            for set in 0..8 {
                let Some(values) = safe_gltf(ctx, "glTF COLOR reader", || {
                    reader.read_colors(set).map(|values| {
                        collect_f32_arrays(
                            ctx,
                            ImportStorageClass::SceneConstruction,
                            contract.position_count,
                            "gltf-color-values",
                            values.into_rgba_f32(),
                        )
                    })
                })?
                .transpose()?
                else {
                    break;
                };
                attributes.try_push(AttributeStorage {
                    semantic: AttributeSemantic::Color(set),
                    lanes: 4,
                    values,
                })?;
            }
            let joint_indices = safe_gltf(ctx, "glTF JOINTS reader", || {
                reader.read_joints(0).map(|values| {
                    collect_exact(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.position_count,
                        "gltf-joint-index-values",
                        values.into_u16(),
                    )
                })
            })?
            .transpose()?
            .unwrap_or_else(|| {
                ctx.metered_vec(
                    ImportStorageClass::SceneConstruction,
                    "gltf-joint-index-values",
                )
            });
            let joint_weights = safe_gltf(ctx, "glTF WEIGHTS reader", || {
                reader.read_weights(0).map(|values| {
                    collect_exact(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.position_count,
                        "gltf-joint-weight-values",
                        values.into_f32(),
                    )
                })
            })?
            .transpose()?
            .unwrap_or_else(|| {
                ctx.metered_vec(
                    ImportStorageClass::SceneConstruction,
                    "gltf-joint-weight-values",
                )
            });

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

            let (influence_offsets, influences) = build_influence_rows(
                ctx,
                contract.position_count,
                joint_indices.as_slice(),
                joint_weights.as_slice(),
            )?;
            work.complete_all()?;
            Ok(PrimitiveStorage {
                topology,
                has_indices: primitive.indices().is_some(),
                indices,
                material: contract
                    .material_index
                    .map(|index| {
                        authoring_u32(ctx, index, "gltf-authoring-material-id")
                            .map(MaterialLocalId::new)
                    })
                    .transpose()?,
                attributes,
                influence_offsets,
                influences,
            })
        },
    )
}

fn collect_f32_arrays<'scope, const N: usize>(
    ctx: &'scope ParseScope<'_>,
    class: ImportStorageClass,
    expected: usize,
    allocation: &'static str,
    values: impl Iterator<Item = [f32; N]>,
) -> Result<MeteredVec<'scope, f32>> {
    let scalar_count = expected
        .checked_mul(N)
        .ok_or_else(|| ctx.allocation_failure(allocation))?;
    let mut collected = ctx.metered_vec(class, allocation);
    collected.try_reserve(scalar_count)?;
    let mut count = 0usize;
    for value in values {
        poll_parser_loop(ctx, count)?;
        if count == expected {
            return Err(gltf_parse_error(
                ctx,
                format!("{allocation} produced more than {expected} values"),
            ));
        }
        collected.try_extend_from_slice_reserved(&value)?;
        count += 1;
    }
    if count != expected {
        return Err(gltf_parse_error(
            ctx,
            format!("{allocation} produced {count} values, expected {expected}"),
        ));
    }
    Ok(collected)
}

fn build_influence_rows<'scope>(
    ctx: &'scope ParseScope<'_>,
    vertex_count: usize,
    joint_indices: &[[u16; 4]],
    joint_weights: &[[f32; 4]],
) -> Result<(
    MeteredVec<'scope, u32>,
    MeteredVec<'scope, InfluenceAuthoring>,
)> {
    let mut offsets = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-authoring-influence-offsets",
    );
    let mut influences = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-authoring-influences",
    );
    if joint_indices.is_empty() && joint_weights.is_empty() {
        return Ok((offsets, influences));
    }
    if joint_indices.len() != vertex_count || joint_weights.len() != vertex_count {
        return Err(gltf_parse_error(
            ctx,
            "glTF joint and weight row counts do not match the vertex count",
        ));
    }
    offsets.try_reserve(vertex_count.saturating_add(1))?;
    influences.try_reserve(vertex_count.saturating_mul(4))?;
    offsets.try_push_reserved(0)?;
    for (vertex_index, (joints, weights)) in joint_indices.iter().zip(joint_weights).enumerate() {
        poll_parser_loop(ctx, vertex_index)?;
        for source_order in 0..4 {
            influences.try_push_reserved(InfluenceAuthoring::new(
                u32::from(joints[source_order]),
                weights[source_order],
                source_order as u32,
            ))?;
        }
        offsets.try_push_reserved(authoring_u32(
            ctx,
            influences.len(),
            "gltf-influence-offset",
        )?)?;
    }
    Ok((offsets, influences))
}

struct MeshInstanceStorage {
    authored: GeometryMeshInstanceAuthoring,
    skin_index: Option<usize>,
}

struct NodeStorage<'scope, 'document> {
    roots: MeteredVec<'scope, NodeLocalId>,
    nodes: MeteredVec<'scope, NodeAuthoringNode<'document>>,
    node_ids: MeteredVec<'scope, Option<NodeLocalId>>,
    skin_references: MeteredVec<'scope, (NodeLocalId, usize)>,
    mesh_instances: MeteredVec<'scope, MeshInstanceStorage>,
}

fn build_scene_nodes<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    graph: &NodeGraph<'document, '_>,
    logical_mesh_count: usize,
) -> Result<NodeStorage<'scope, 'document>> {
    let mut storage = NodeStorage {
        roots: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            graph.roots.len(),
            "gltf-authoring-roots",
        )?,
        nodes: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            graph.nodes.len(),
            "gltf-authoring-nodes",
        )?,
        node_ids: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            graph.nodes.len(),
            None,
            "gltf-node-id-map",
        )?,
        skin_references: try_vec_capacity(
            ctx,
            ImportStorageClass::FormatNormalization,
            graph.nodes.len(),
            "gltf-node-skin-map",
        )?,
        mesh_instances: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            graph.nodes.len(),
            "gltf-authoring-mesh-instances",
        )?,
    };
    let mut pending = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        graph.nodes.len(),
        "gltf-node-build-stack",
    )?;
    for (root_ordinal, node_index) in graph.roots.as_slice().iter().rev().copied().enumerate() {
        poll_parser_loop(ctx, root_ordinal)?;
        pending.try_push_reserved((node_index, Presence::Absent))?;
    }

    let mut constructed = 0usize;
    while let Some((node_index, parent)) = pending.pop() {
        poll_parser_loop(ctx, constructed)?;
        constructed += 1;
        let node = graph.nodes.as_slice().get(node_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF node graph references missing node {node_index}"),
            )
        })?;
        let slot = storage
            .node_ids
            .as_mut_slice()
            .get_mut(node_index)
            .ok_or_else(|| {
                gltf_parse_error(ctx, format!("glTF node index {node_index} is out of range"))
            })?;
        if slot.is_some() {
            return Err(gltf_parse_error(
                ctx,
                format!("glTF node {node_index} was constructed more than once"),
            ));
        }
        let node_id = NodeLocalId::new(authoring_u32(
            ctx,
            storage.nodes.len(),
            "gltf-authoring-node-id",
        )?);
        *slot = Some(node_id);
        if matches!(parent, Presence::Absent) {
            storage.roots.try_push_reserved(node_id)?;
        }

        let transform = safe_gltf(ctx, "glTF node transform", || node.transform().matrix())?;
        let transform = AuthoredTransform::try_matrix(Mat4 { cols: transform }).map_err(|_| {
            gltf_parse_error(ctx, format!("glTF node {node_index} transform is invalid"))
        })?;
        let camera = safe_gltf(ctx, "glTF node camera", || node.camera())?
            .map(|camera| {
                authoring_u32(ctx, camera.index(), "gltf-authoring-camera-id")
                    .map(CameraLocalId::new)
            })
            .transpose()?;
        let mut authored = NodeAuthoringNode::new(
            parent,
            transform,
            camera.map_or(Presence::Absent, Presence::Present),
            Presence::Absent,
        );
        if let Some(name) = safe_gltf(ctx, "glTF node name", || node.name())? {
            complete_canonical_string_work(ctx, name.len())?;
            authored = authored.with_name(name);
        }
        storage.nodes.try_push_reserved(authored)?;

        let skin_index = safe_gltf(ctx, "glTF node skin", || node.skin())?.map(|skin| skin.index());
        if let Some(skin_index) = skin_index {
            storage
                .skin_references
                .try_push_reserved((node_id, skin_index))?;
        }
        if let Some(mesh) = safe_gltf(ctx, "glTF node mesh", || node.mesh())? {
            if mesh.index() >= logical_mesh_count {
                return Err(gltf_parse_error(
                    ctx,
                    format!(
                        "glTF node {node_index} references mesh {} out of range",
                        mesh.index()
                    ),
                ));
            }
            storage
                .mesh_instances
                .try_push_reserved(MeshInstanceStorage {
                    authored: GeometryMeshInstanceAuthoring::new(
                        node_id,
                        LogicalMeshLocalId::new(authoring_u32(
                            ctx,
                            mesh.index(),
                            "gltf-authoring-logical-mesh-id",
                        )?),
                    ),
                    skin_index,
                })?;
        }

        for (child_ordinal, child_index) in graph.children.as_slice()[node_index]
            .as_slice()
            .iter()
            .copied()
            .enumerate()
            .rev()
        {
            poll_parser_loop(ctx, child_ordinal)?;
            pending.try_push_reserved((
                child_index,
                Presence::Present(NodeParent::new(
                    node_id,
                    authoring_u32(ctx, child_ordinal, "gltf-authoring-child-ordinal")?,
                )),
            ))?;
        }
    }
    Ok(storage)
}

struct NodeGraph<'document, 'scope> {
    nodes: MeteredVec<'scope, gltf::Node<'document>>,
    children: MeteredVec<'scope, MeteredVec<'scope, usize>>,
    roots: MeteredVec<'scope, usize>,
}

fn preflight_node_graph<'document, 'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Gltf,
) -> Result<NodeGraph<'document, 'scope>> {
    let node_count = safe_gltf(ctx, "glTF node count", || gltf.nodes().len())?;
    let nodes = ctx.with_work_group(
        [
            (ImportWorkClass::GraphNodes, to_u64(node_count)),
            (ImportWorkClass::GraphWork, to_u64(node_count)),
        ],
        |work| -> Result<_> {
            work.poll()?;
            let nodes = finish_gltf_operation(
                "glTF node iterator construction",
                catch_unwind(AssertUnwindSafe(|| gltf.nodes())),
            )?;
            collect_exact_with_work(
                ctx,
                work,
                node_count,
                "gltf-node-list",
                "glTF node iteration",
                nodes,
                [1, 1],
            )
        },
    )?;
    let mut children = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        node_count,
        "gltf-node-children",
    )?;
    let mut parents = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        node_count,
        None,
        "gltf-node-parents",
    )?;
    for (parent_index, node) in nodes.as_slice().iter().enumerate() {
        poll_parser_loop(ctx, parent_index)?;
        let child_count = safe_gltf(ctx, "glTF node child count", || node.children().len())?;
        let child_indices = ctx.with_work_group(
            [
                (ImportWorkClass::GraphEdges, to_u64(child_count)),
                (ImportWorkClass::GraphWork, to_u64(child_count)),
            ],
            |work| -> Result<_> {
                let mut child_indices = ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "gltf-node-child-list",
                );
                child_indices.try_reserve(child_count)?;
                work.poll()?;
                let mut node_children = finish_gltf_operation(
                    "glTF node child iterator construction",
                    catch_unwind(AssertUnwindSafe(|| node.children())),
                )?;
                loop {
                    if child_indices.len().is_multiple_of(PARSER_POLL_INTERVAL) {
                        work.poll()?;
                    }
                    let next = finish_gltf_operation(
                        "glTF node child iteration",
                        catch_unwind(AssertUnwindSafe(|| node_children.next())),
                    )?;
                    let Some(child) = next else {
                        break;
                    };
                    if child_indices.len() == child_count {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "gltf-node-child-list produced more than {child_count} values"
                            ),
                        ));
                    }
                    let child_index = child.index();
                    if child_index >= node_count {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF node {parent_index} references missing child {child_index}"
                            ),
                        ));
                    }
                    if child_index == parent_index {
                        return Err(gltf_parse_error(
                            ctx,
                            format!("glTF node graph contains a cycle at node {parent_index}"),
                        ));
                    }
                    if let Some(existing_parent) = parents.as_slice()[child_index] {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF node {child_index} has multiple parents ({existing_parent} and {parent_index})"
                            ),
                        ));
                    }
                    child_indices.try_push_reserved(child_index)?;
                    parents.as_mut_slice()[child_index] = Some(parent_index);
                    work.complete([1, 1])?;
                }
                if child_indices.len() != child_count {
                    return Err(gltf_parse_error(
                        ctx,
                        format!(
                            "gltf-node-child-list produced {} values, expected {child_count}",
                            child_indices.len()
                        ),
                    ));
                }
                Ok(child_indices)
            },
        )?;
        children.try_push_reserved(child_indices)?;
    }

    validate_graph_depth_and_cycles(ctx, children.as_slice(), parents.as_slice())?;
    let roots = select_and_validate_scene_roots(ctx, gltf, parents.as_slice())?;
    charge_canonical_graph_shape(ctx, roots.len())?;
    validate_canonical_graph_limits(ctx, children.as_slice(), roots.as_slice())?;

    Ok(NodeGraph {
        nodes,
        children,
        roots,
    })
}

fn validate_canonical_graph_limits(
    ctx: &ParseScope<'_>,
    children: &[MeteredVec<'_, usize>],
    roots: &[usize],
) -> Result<()> {
    let mut max_depth = 0usize;
    observe_graph_depth(ctx, &mut max_depth, 1)?;

    let mut pending = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        roots.len(),
        "gltf-canonical-graph-stack",
    )?;
    for (root_ordinal, root) in roots.iter().rev().copied().enumerate() {
        poll_parser_loop(ctx, root_ordinal)?;
        pending.try_push_reserved((root, 2usize))?;
    }

    let mut visited = 0usize;
    while let Some((node_index, depth)) = pending.pop() {
        poll_parser_loop(ctx, visited)?;
        visited = visited.saturating_add(1);
        observe_graph_depth(ctx, &mut max_depth, depth)?;

        let node_children = children.get(node_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF node graph references missing node {node_index}"),
            )
        })?;
        let child_depth = depth
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-graph-depth-counter"))?;
        pending.try_reserve(node_children.len())?;
        for (child_ordinal, child) in node_children.as_slice().iter().rev().copied().enumerate() {
            poll_parser_loop(ctx, child_ordinal)?;
            pending.try_push_reserved((child, child_depth))?;
        }
    }

    Ok(())
}

fn charge_canonical_graph_shape(ctx: &ParseScope<'_>, root_count: usize) -> Result<()> {
    Ok(ctx.with_work_group(
        [
            (ImportWorkClass::GraphNodes, 1),
            (ImportWorkClass::GraphEdges, to_u64(root_count)),
        ],
        |work| work.complete_all(),
    )?)
}

fn observe_graph_depth(ctx: &ParseScope<'_>, maximum: &mut usize, depth: usize) -> Result<()> {
    let additional = depth.saturating_sub(*maximum);
    if additional == 0 {
        return Ok(());
    }
    ctx.with_work(ImportWorkClass::GraphDepth, to_u64(additional), |work| {
        work.complete_all()
    })?;
    *maximum = depth;
    Ok(())
}

fn validate_graph_depth_and_cycles(
    ctx: &ParseScope<'_>,
    children: &[MeteredVec<'_, usize>],
    parents: &[Option<usize>],
) -> Result<()> {
    let mut depths = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        parents.len(),
        0usize,
        "gltf-node-depths",
    )?;
    let mut queue = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        parents.len(),
        "gltf-node-depth-queue",
    )?;
    for (node_index, parent) in parents.iter().enumerate() {
        if parent.is_none() {
            depths.as_mut_slice()[node_index] = 1;
            queue.try_push_reserved(node_index)?;
        }
    }

    let mut cursor = 0usize;
    while cursor < queue.len() {
        poll_parser_loop(ctx, cursor)?;
        let node_index = queue.as_slice()[cursor];
        cursor += 1;
        let depth = depths.as_slice()[node_index];
        for (child_ordinal, &child_index) in children[node_index].as_slice().iter().enumerate() {
            poll_parser_loop(ctx, child_ordinal)?;
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-graph-depth-counter"))?;
            depths.as_mut_slice()[child_index] = child_depth;
            queue.try_push_reserved(child_index)?;
        }
    }

    if queue.len() != parents.len() {
        return Err(gltf_parse_error(ctx, "glTF node graph contains a cycle"));
    }
    Ok(())
}

fn select_and_validate_scene_roots<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Gltf,
    parents: &[Option<usize>],
) -> Result<MeteredVec<'scope, usize>> {
    let scene_count = safe_gltf(ctx, "glTF scene count", || gltf.scenes().len())?;
    let mut scene_roots = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-scene-root-lists",
    );

    for (scene_index, scene) in gltf.scenes().enumerate() {
        poll_parser_loop(ctx, scene_index)?;
        let roots = ctx.with_work(ImportWorkClass::GraphWork, 1, |work| -> Result<_> {
            scene_roots.try_reserve(1)?;
            work.poll()?;
            let root_count = finish_gltf_operation(
                "glTF scene root count",
                catch_unwind(AssertUnwindSafe(|| scene.nodes().len())),
            )?;
            work.complete_all()?;
            work.replenish(to_u64(root_count))?;
            work.poll()?;
            let roots = finish_gltf_operation(
                "glTF scene root iterator construction",
                catch_unwind(AssertUnwindSafe(|| scene.nodes())),
            )?;
            collect_exact_with_work(
                ctx,
                work,
                root_count,
                "gltf-scene-roots",
                "glTF scene root iteration",
                roots.map(|node| node.index()),
                [1],
            )
        })?;
        scene_roots.try_push_reserved(roots)?;
    }

    let mut seen_in_scene = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        parents.len(),
        usize::MAX,
        "gltf-scene-root-membership",
    )?;
    for (scene_index, roots) in scene_roots.as_slice().iter().enumerate() {
        for (root_ordinal, &root_index) in roots.as_slice().iter().enumerate() {
            poll_parser_loop(ctx, root_ordinal)?;
            let Some(parent) = parents.get(root_index) else {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF scene {scene_index} references missing root node {root_index}"),
                ));
            };
            if seen_in_scene.as_slice()[root_index] == scene_index {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF scene {scene_index} contains duplicate root node {root_index}"),
                ));
            }
            seen_in_scene.as_mut_slice()[root_index] = scene_index;
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
            let selected = scene_roots.as_slice().get(scene_index).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format!("glTF default scene index {scene_index} is out of range"),
                )
            })?;
            let mut roots = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                selected.len(),
                "gltf-selected-scene-roots",
            )?;
            for (root_ordinal, root) in selected.as_slice().iter().copied().enumerate() {
                poll_parser_loop(ctx, root_ordinal)?;
                roots.try_push_reserved(root)?;
            }
            Ok(roots)
        }
        None => ctx.with_work(ImportWorkClass::GraphWork, 1, |work| -> Result<_> {
            let mut roots = ctx.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-derived-scene-roots",
            );
            work.complete_all()?;
            for (chunk_index, chunk) in parents.chunks(PARSER_POLL_INTERVAL).enumerate() {
                work.poll()?;
                let root_count = chunk.iter().filter(|parent| parent.is_none()).count();
                if root_count == 0 {
                    continue;
                }
                work.replenish(to_u64(root_count))?;
                roots.try_reserve(root_count)?;
                let base = chunk_index
                    .checked_mul(PARSER_POLL_INTERVAL)
                    .ok_or_else(|| ctx.allocation_failure("gltf-derived-root-index"))?;
                for (offset, parent) in chunk.iter().enumerate() {
                    if parent.is_some() {
                        continue;
                    }
                    let node_index = base
                        .checked_add(offset)
                        .ok_or_else(|| ctx.allocation_failure("gltf-derived-root-index"))?;
                    roots.try_push_reserved(node_index)?;
                    work.complete([1])?;
                }
            }
            Ok(roots)
        }),
    }
}

struct SkinStorageRow<'scope> {
    name: Option<EntityName>,
    joints: MeteredVec<'scope, NodeLocalId>,
    inverse_bind_matrices: MeteredVec<'scope, Mat4>,
    skeleton_root: Option<NodeLocalId>,
}

struct SkinStorage<'scope> {
    skins: MeteredVec<'scope, SkinStorageRow<'scope>>,
    skin_ids: MeteredVec<'scope, Option<SkinLocalId>>,
}

fn build_skins<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Gltf,
    buffers: &[BufferData<'_>],
    node_ids: &[Option<NodeLocalId>],
    node_skins: &[(NodeLocalId, usize)],
) -> Result<SkinStorage<'scope>> {
    let skin_count = safe_gltf(ctx, "glTF skin count", || gltf.skins().len())?;
    let mut referenced_skins = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        skin_count,
        false,
        "gltf-referenced-skins",
    )?;
    for (binding_ordinal, (_node_id, skin_index)) in node_skins.iter().enumerate() {
        poll_parser_loop(ctx, binding_ordinal)?;
        let Some(referenced) = referenced_skins.as_mut_slice().get_mut(*skin_index) else {
            return Err(gltf_parse_error(
                ctx,
                format!("glTF node references skin {skin_index} out of range"),
            ));
        };
        *referenced = true;
    }

    let mut storage = SkinStorage {
        skins: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            skin_count,
            "gltf-authoring-skins",
        )?,
        skin_ids: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            skin_count,
            None,
            "gltf-skin-id-map",
        )?,
    };
    for (ordinal, skin) in gltf.skins().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        let skin_index = skin.index();
        let Some(is_referenced) = referenced_skins.as_slice().get(skin_index).copied() else {
            return Err(gltf_parse_error(
                ctx,
                format!("glTF skin index {skin_index} is out of range"),
            ));
        };
        if !is_referenced {
            continue;
        }

        let joint_count = safe_gltf(ctx, "glTF skin joint count", || skin.joints().len())?;
        let joints = safe_gltf(ctx, "glTF skin joints", || {
            collect_exact(
                ctx,
                ImportStorageClass::FormatNormalization,
                joint_count,
                "gltf-skin-joints",
                skin.joints(),
            )
        })??;
        let mut mapped_joints = try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            joints.len(),
            "gltf-mapped-skin-joints",
        )?;
        for (joint_ordinal, joint) in joints.as_slice().iter().enumerate() {
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
            mapped_joints.try_push_reserved(*node_id)?;
        }

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
        let inverse_bind_matrices = if has_inverse_bind_matrices {
            let reader = safe_gltf(ctx, "glTF skin reader construction", || {
                skin.reader(|buffer| buffers.get(buffer.index()).map(BufferData::as_slice))
            })?;
            safe_gltf(ctx, "glTF inverse bind matrix reader", || {
                reader
                    .read_inverse_bind_matrices()
                    .map(|matrices| {
                        collect_exact(
                            ctx,
                            ImportStorageClass::SceneConstruction,
                            mapped_joints.len(),
                            "gltf-inverse-bind-matrices",
                            matrices.map(|cols| Mat4 { cols }),
                        )
                    })
                    .transpose()
            })??
            .ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format!(
                        "glTF skin {} declares inverse bind matrices but exposes no reader",
                        skin.index()
                    ),
                )
            })?
        } else {
            let mut matrices = ctx.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-inverse-bind-matrices",
            );
            matrices.try_reserve(mapped_joints.len())?;
            for joint in 0..mapped_joints.len() {
                poll_parser_loop(ctx, joint)?;
                matrices.try_push_reserved(Mat4::IDENTITY)?;
            }
            matrices
        };
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

        let source_name = skin.name();
        complete_canonical_string_work(ctx, source_name.map_or(0, str::len))?;
        let name = source_name
            .map(EntityName::try_new)
            .transpose()
            .map_err(|_| gltf_parse_error(ctx, "glTF skin name is not canonical"))?;
        let local_id = SkinLocalId::new(authoring_u32(
            ctx,
            storage.skins.len(),
            "gltf-authoring-skin-id",
        )?);
        storage.skins.try_push_reserved(SkinStorageRow {
            name,
            joints: mapped_joints,
            inverse_bind_matrices,
            skeleton_root,
        })?;
        let Some(slot) = storage.skin_ids.as_mut_slice().get_mut(skin_index) else {
            return Err(gltf_parse_error(
                ctx,
                format!("glTF skin index {skin_index} is out of range"),
            ));
        };
        *slot = Some(local_id);
    }
    Ok(storage)
}

fn build_mesh_instance_deformations<'scope>(
    ctx: &'scope ParseScope<'_>,
    mesh_instances: &[MeshInstanceStorage],
    skin_ids: &[Option<SkinLocalId>],
) -> Result<MeteredVec<'scope, MeshInstanceDeformationAuthoring<'static>>> {
    let mut deformations = try_vec_capacity(
        ctx,
        ImportStorageClass::SceneConstruction,
        mesh_instances.len(),
        "gltf-authoring-mesh-instance-deformations",
    )?;
    for (ordinal, instance) in mesh_instances.iter().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        let skin = match instance.skin_index {
            Some(index) => skin_ids
                .get(index)
                .copied()
                .flatten()
                .map(Presence::Present)
                .ok_or_else(|| {
                    gltf_parse_error(ctx, format!("glTF skin {index} was not imported"))
                })?,
            None => Presence::Absent,
        };
        deformations.try_push_reserved(MeshInstanceDeformationAuthoring::new(
            skin,
            Presence::Absent,
        ))?;
    }
    Ok(deformations)
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

struct CameraStorage<'scope, 'document> {
    authored: MeteredVec<'scope, CameraAuthoring>,
    names: MeteredVec<'scope, Option<&'document str>>,
}

fn build_cameras<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Gltf,
) -> Result<CameraStorage<'scope, 'document>> {
    let camera_count = safe_gltf(ctx, "glTF camera count", || gltf.cameras().count())?;
    let mut storage = CameraStorage {
        authored: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            camera_count,
            "gltf-authoring-cameras",
        )?,
        names: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            camera_count,
            "gltf-authoring-camera-names",
        )?,
    };
    for (ordinal, camera) in gltf.cameras().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        let projection = safe_gltf(ctx, "glTF camera projection", || {
            match camera.projection() {
                gltf::camera::Projection::Orthographic(orthographic) => {
                    CameraProjection::orthographic(
                        orthographic.xmag(),
                        orthographic.ymag(),
                        orthographic.znear(),
                        orthographic.zfar(),
                    )
                }
                gltf::camera::Projection::Perspective(perspective) => {
                    CameraProjection::perspective(
                        perspective.yfov(),
                        perspective.aspect_ratio(),
                        perspective.znear(),
                        perspective.zfar(),
                    )
                }
            }
        })?
        .map_err(|_| {
            gltf_parse_error(ctx, format!("glTF camera {ordinal} projection is invalid"))
        })?;
        let name = match camera.name() {
            Some(name) if name.len() > ctx.limits().max_string_bytes => {
                push_warning(
                    ctx,
                    "gltf.camera_name_limit",
                    "camera name exceeded max_string_bytes and was ignored",
                )?;
                None
            }
            name => name,
        };
        complete_canonical_string_work(ctx, name.map_or(0, str::len))?;
        storage
            .authored
            .try_push_reserved(CameraAuthoring::new(projection))?;
        storage.names.try_push_reserved(name)?;
    }
    Ok(storage)
}

struct OwnerSource {
    owner: ProfileOwner,
    provenance: [ProvenanceLocalId; 1],
}

#[allow(clippy::too_many_arguments)]
fn submit_complete_batch(
    ctx: &ParseScope<'_>,
    appearance_storage: &AppearanceStorage<'_, '_>,
    geometry_storage: &GeometryStorage<'_, '_>,
    cameras: &CameraStorage<'_, '_>,
    nodes: &NodeStorage<'_, '_>,
    skins: &SkinStorage<'_>,
    mesh_instance_deformations: &MeteredVec<'_, MeshInstanceDeformationAuthoring<'static>>,
    external_images: &ExternalImageResources<'_>,
) -> Result<SceneDraft> {
    let mut images = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-images",
    );
    images.try_reserve(appearance_storage.images.len())?;
    for image in appearance_storage.images.as_slice() {
        let media_type = image
            .media_type
            .as_ref()
            .map_or(Presence::Absent, Presence::Present);
        images.try_push_reserved(ImageAuthoring::session_resource(image.resource, media_type))?;
    }

    let mut bindings = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-material-bindings",
    );
    bindings.try_reserve(appearance_storage.bindings.len())?;
    for binding in appearance_storage.bindings.as_slice() {
        let authored = MaterialBindingAuthoring::try_from_parts(
            binding.texture,
            &binding.role,
            &binding.coordinate_source,
            &binding.channel_mapping,
            &binding.color_interpretation,
            TextureTransform::IDENTITY,
            binding.strength,
            &binding.blend,
            Some(&binding.source_key),
        )
        .map_err(|_| gltf_parse_error(ctx, "glTF texture binding semantics are invalid"))?;
        bindings.try_push_reserved(authored)?;
    }

    let mut materials = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-materials",
    );
    let shading_model = ShadingModel::PbrMetallicRoughness;
    materials.try_reserve(appearance_storage.materials.len())?;
    for material in appearance_storage.materials.as_slice() {
        let mut authored = MaterialAuthoring::try_from_parts(
            material.bindings,
            &shading_model,
            &material.alpha_mode,
            material.double_sided,
            material.factors,
        )
        .map_err(|_| gltf_parse_error(ctx, "glTF material semantics are invalid"))?;
        if let Some(name) = material.name {
            authored = authored.with_name(name);
        }
        materials.try_push_reserved(authored)?;
    }

    let mut logical_meshes = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-logical-meshes",
    );
    let mut primitives = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-primitives",
    );
    let mut attributes = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-attributes",
    );
    let primitive_count = geometry_storage
        .logical_meshes
        .as_slice()
        .iter()
        .try_fold(0usize, |count, mesh| {
            count.checked_add(mesh.primitives.len())
        })
        .ok_or_else(|| ctx.allocation_failure("gltf-complete-batch-primitive-count"))?;
    let attribute_count = geometry_storage
        .logical_meshes
        .as_slice()
        .iter()
        .flat_map(|mesh| mesh.primitives.as_slice())
        .try_fold(0usize, |count, primitive| {
            count.checked_add(primitive.attributes.len())
        })
        .ok_or_else(|| ctx.allocation_failure("gltf-complete-batch-attribute-count"))?;
    logical_meshes.try_reserve(geometry_storage.logical_meshes.len())?;
    primitives.try_reserve(primitive_count)?;
    attributes.try_reserve(attribute_count)?;
    for mesh in geometry_storage.logical_meshes.as_slice() {
        let primitive_offset = authoring_u32(
            ctx,
            primitives.len(),
            "gltf-complete-batch-primitive-offset",
        )?;
        for primitive in mesh.primitives.as_slice() {
            let attribute_offset = authoring_u32(
                ctx,
                attributes.len(),
                "gltf-complete-batch-attribute-offset",
            )?;
            for attribute in primitive.attributes.as_slice() {
                attributes.try_push_reserved(GeometryAttributeAuthoring::f32(
                    attribute.semantic.clone(),
                    AttributeDomain::Vertex,
                    attribute.lanes,
                    attribute.values.as_slice(),
                    Presence::Absent,
                ))?;
            }
            let indices = if primitive.has_indices {
                Presence::Present(primitive.indices.as_slice())
            } else {
                Presence::Absent
            };
            let material = primitive
                .material
                .map_or(Presence::Absent, Presence::Present);
            primitives.try_push_reserved(GeometryPrimitiveAuthoring::new(
                primitive.topology,
                indices,
                material,
                AuthoringRange::new(
                    attribute_offset,
                    authoring_u32(
                        ctx,
                        primitive.attributes.len(),
                        "gltf-complete-batch-attribute-count",
                    )?,
                ),
            ))?;
        }
        let mut authored = GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
            primitive_offset,
            authoring_u32(
                ctx,
                mesh.primitives.len(),
                "gltf-complete-batch-logical-mesh-primitive-count",
            )?,
        ));
        if let Some(name) = mesh.name {
            authored = authored.with_name(name);
        }
        logical_meshes.try_push_reserved(authored)?;
    }

    let mut mesh_instances = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-mesh-instances",
    );
    mesh_instances.try_reserve(nodes.mesh_instances.len())?;
    for instance in nodes.mesh_instances.as_slice() {
        mesh_instances.try_push_reserved(instance.authored)?;
    }

    let mut skin_authoring = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-skins",
    );
    skin_authoring.try_reserve(skins.skins.len())?;
    for skin in skins.skins.as_slice() {
        let name = skin
            .name
            .as_ref()
            .map_or(Presence::Absent, Presence::Present);
        let skeleton_root = skin
            .skeleton_root
            .map_or(Presence::Absent, Presence::Present);
        skin_authoring.try_push_reserved(SkinAuthoring::new(
            name,
            skin.joints.as_slice(),
            skin.inverse_bind_matrices.as_slice(),
            skeleton_root,
            SkinMatrixSpaces::canonical(),
        ))?;
    }

    let mut logical_morphs = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-logical-morphs",
    );
    logical_morphs.try_reserve(logical_meshes.len())?;
    for _ in 0..logical_meshes.len() {
        logical_morphs
            .try_push_reserved(LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[]))?;
    }
    let mut primitive_deformations = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-primitive-deformations",
    );
    primitive_deformations.try_reserve(primitives.len())?;
    for mesh in geometry_storage.logical_meshes.as_slice() {
        for primitive in mesh.primitives.as_slice() {
            let influences = if primitive.influences.is_empty() {
                Presence::Absent
            } else {
                Presence::Present(InfluenceRowsAuthoring::new(
                    primitive.influence_offsets.as_slice(),
                    primitive.influences.as_slice(),
                ))
            };
            primitive_deformations
                .try_push_reserved(PrimitiveDeformationAuthoring::new(influences, &[]))?;
        }
    }

    let appearance = AppearanceAuthoringBatch::new(
        images.as_slice(),
        appearance_storage.samplers.as_slice(),
        appearance_storage.textures.as_slice(),
        bindings.as_slice(),
        materials.as_slice(),
    );
    let geometry = GeometryAuthoringBatch::new(
        logical_meshes.as_slice(),
        primitives.as_slice(),
        attributes.as_slice(),
        mesh_instances.as_slice(),
    );
    let deformation = DeformationAuthoringBatch::new(
        skin_authoring.as_slice(),
        &[],
        logical_morphs.as_slice(),
        primitive_deformations.as_slice(),
        mesh_instance_deformations.as_slice(),
    );
    let scene_space = SceneSpace::STANDARD_TARGET;
    let spatial = SpatialAuthoringBatch::new(&scene_space, cameras.authored.as_slice(), &[]);
    let node_batch = NodeAuthoringBatch::new(
        Presence::Present(nodes.roots.as_slice()),
        nodes.nodes.as_slice(),
    );
    let animations = AnimationAuthoringBatch::empty();

    let primary_identity = ResourceIdentity::new(ctx.source().as_str())
        .map_err(|_| gltf_parse_error(ctx, "glTF primary resource identity is not canonical"))?;
    let resource_count = external_images
        .values
        .len()
        .checked_add(1)
        .ok_or_else(|| ctx.allocation_failure("gltf-profile-resource-count"))?;
    let mut resources = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-resources",
    );
    let mut snapshots = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-resource-snapshots",
    );
    let mut locators = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-source-locators",
    );
    resources.try_reserve(resource_count)?;
    snapshots.try_reserve(resource_count)?;
    locators.try_reserve(resource_count)?;
    resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
        &primary_identity,
        SessionOpaqueReason::CallerLabel,
    ))?;
    snapshots.try_push_reserved(ResourceSnapshotAuthoring::absent())?;
    locators.try_push_reserved(SourceObjectLocator::indexed(SourceObjectKind::Document, 0))?;
    for resource in external_images.values.as_slice() {
        resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
            &resource.identity,
            SessionOpaqueReason::UnregisteredResolver,
        ))?;
        snapshots.try_push_reserved(ResourceSnapshotAuthoring::snapshotted(
            resource.snapshot.bytes(),
        ))?;
        locators.try_push_reserved(SourceObjectLocator::indexed(
            SourceObjectKind::Image,
            u64::try_from(resource.image_index)
                .map_err(|_| ctx.allocation_failure("gltf-profile-image-locator"))?,
        ))?;
    }

    let format = FormatId::new(FORMAT_ID)
        .map_err(|_| gltf_parse_error(ctx, "static glTF format identity is invalid"))?;
    let mut source_identities = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-source-identities",
    );
    let mut provenance_nodes = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-provenance-nodes",
    );
    source_identities.try_reserve(resource_count)?;
    provenance_nodes.try_reserve(resource_count)?;
    for index in 0..resource_count {
        let local = authoring_u32(ctx, index, "gltf-profile-source-identity")?;
        let locator = locators.as_slice().get(index).ok_or_else(|| {
            gltf_parse_error(ctx, "glTF profile source locator table is incomplete")
        })?;
        source_identities.try_push_reserved(SourceIdentityAuthoring::new(
            ResourceIdentityLocalId::new(local),
            &format,
            locator,
        ))?;
        provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::source(
            SourceIdentityLocalId::new(local),
        ))?;
    }

    let mut owner_sources = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-owner-sources",
    );
    push_owner_source(&mut owner_sources, ProfileOwner::Scene, 0)?;
    push_owner_source(&mut owner_sources, ProfileOwner::SyntheticRoot, 0)?;
    for index in 0..nodes.nodes.len() {
        push_owner_source(
            &mut owner_sources,
            ProfileOwner::Node(NodeLocalId::new(authoring_u32(
                ctx,
                index,
                "gltf-profile-node-owner",
            )?)),
            0,
        )?;
    }
    for index in 0..cameras.authored.len() {
        push_owner_source(
            &mut owner_sources,
            ProfileOwner::Camera(authoring_u32(ctx, index, "gltf-profile-camera-owner")?),
            0,
        )?;
    }
    for (index, image) in appearance_storage.images.as_slice().iter().enumerate() {
        let provenance = image.resource_index;
        let index = authoring_u32(ctx, index, "gltf-profile-image-owner")?;
        push_owner_source(&mut owner_sources, ProfileOwner::Image(index), provenance)?;
        push_owner_source(&mut owner_sources, ProfileOwner::Sampler(index), provenance)?;
        push_owner_source(&mut owner_sources, ProfileOwner::Texture(index), provenance)?;
        push_owner_source(
            &mut owner_sources,
            ProfileOwner::TextureBinding(index),
            provenance,
        )?;
    }
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        materials.len(),
        ProfileOwner::Material,
    )?;
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        logical_meshes.len(),
        ProfileOwner::LogicalMesh,
    )?;
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        primitives.len(),
        ProfileOwner::Primitive,
    )?;
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        attributes.len(),
        ProfileOwner::Attribute,
    )?;
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        mesh_instances.len(),
        ProfileOwner::MeshInstance,
    )?;
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        skin_authoring.len(),
        ProfileOwner::Skin,
    )?;

    let mut owner_provenance = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-owner-provenance",
    );
    owner_provenance.try_reserve(owner_sources.len())?;
    for owner in owner_sources.as_slice() {
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            owner.owner,
            &owner.provenance,
        ))?;
    }

    let source_name_key = NamespacedName::new("org.baozi.gltf:source-name")
        .map_err(|_| gltf_parse_error(ctx, "static glTF metadata key is invalid"))?;
    let mut metadata = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-metadata",
    );
    for (index, name) in cameras.names.as_slice().iter().copied().enumerate() {
        if let Some(name) = name {
            metadata.try_push(MetadataAuthoringEntry::new(
                ProfileOwner::Camera(authoring_u32(ctx, index, "gltf-metadata-camera")?),
                &source_name_key,
                ExtensionValueRef::String(name),
            ))?;
        }
    }
    for (index, image) in appearance_storage.images.as_slice().iter().enumerate() {
        let index = authoring_u32(ctx, index, "gltf-metadata-image")?;
        if let Some(name) = image.texture_name.as_ref() {
            metadata.try_push(MetadataAuthoringEntry::new(
                ProfileOwner::Texture(index),
                &source_name_key,
                ExtensionValueRef::String(name.as_str()),
            ))?;
        }
    }

    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            resources.as_slice(),
            source_identities.as_slice(),
            provenance_nodes.as_slice(),
            owner_provenance.as_slice(),
        ),
        MetadataAuthoringBatch::new(metadata.as_slice()),
    )
    .with_resource_snapshots(snapshots.as_slice());
    Ok(ctx.submit_scene(SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile,
    ))?)
}

fn push_owner_source(
    owners: &mut MeteredVec<'_, OwnerSource>,
    owner: ProfileOwner,
    provenance: u32,
) -> Result<()> {
    owners.try_push(OwnerSource {
        owner,
        provenance: [ProvenanceLocalId::new(provenance)],
    })?;
    Ok(())
}

fn push_indexed_owner_sources(
    ctx: &ParseScope<'_>,
    owners: &mut MeteredVec<'_, OwnerSource>,
    count: usize,
    owner: impl Fn(u32) -> ProfileOwner,
) -> Result<()> {
    for index in 0..count {
        owners.try_push(OwnerSource {
            owner: owner(authoring_u32(ctx, index, "gltf-profile-owner-index")?),
            provenance: [ProvenanceLocalId::new(0)],
        })?;
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

fn map_topology(mode: Mode) -> Option<GeometryTopologyAuthoring<'static>> {
    match mode {
        Mode::Points => Some(GeometryTopologyAuthoring::Points),
        Mode::Lines => Some(GeometryTopologyAuthoring::LineList),
        Mode::Triangles => Some(GeometryTopologyAuthoring::TriangleList),
        Mode::LineLoop | Mode::LineStrip | Mode::TriangleStrip | Mode::TriangleFan => None,
    }
}

fn map_alpha_mode(ctx: &ParseScope<'_>, material: &gltf::Material<'_>) -> Result<AlphaMode> {
    match material.alpha_mode() {
        gltf::material::AlphaMode::Opaque => Ok(AlphaMode::Opaque),
        gltf::material::AlphaMode::Mask => AlphaMode::mask(material.alpha_cutoff().unwrap_or(0.5))
            .map_err(|_| gltf_parse_error(ctx, "glTF alpha cutoff is outside canonical bounds")),
        gltf::material::AlphaMode::Blend => Ok(AlphaMode::Blend),
    }
}

fn map_sampler(sampler: gltf::texture::Sampler<'_>) -> SamplerState {
    let magnification = sampler
        .mag_filter()
        .map(map_mag_filter)
        .unwrap_or(TexelFilter::Linear);
    let (minification, mipmap) = sampler
        .min_filter()
        .map(map_min_filter)
        .unwrap_or((TexelFilter::Linear, MipmapFilter::Linear));
    SamplerState::new(
        minification,
        magnification,
        mipmap,
        map_wrap(sampler.wrap_s()),
        map_wrap(sampler.wrap_t()),
        TextureAddressMode::Repeat,
    )
}

fn map_mag_filter(filter: MagFilter) -> TexelFilter {
    match filter {
        MagFilter::Nearest => TexelFilter::Nearest,
        MagFilter::Linear => TexelFilter::Linear,
    }
}

fn map_min_filter(filter: MinFilter) -> (TexelFilter, MipmapFilter) {
    match filter {
        MinFilter::Nearest => (TexelFilter::Nearest, MipmapFilter::None),
        MinFilter::Linear => (TexelFilter::Linear, MipmapFilter::None),
        MinFilter::NearestMipmapNearest => (TexelFilter::Nearest, MipmapFilter::Nearest),
        MinFilter::LinearMipmapNearest => (TexelFilter::Linear, MipmapFilter::Nearest),
        MinFilter::NearestMipmapLinear => (TexelFilter::Nearest, MipmapFilter::Linear),
        MinFilter::LinearMipmapLinear => (TexelFilter::Linear, MipmapFilter::Linear),
    }
}

fn map_wrap(wrap: WrappingMode) -> TextureAddressMode {
    match wrap {
        WrappingMode::ClampToEdge => TextureAddressMode::ClampToEdge,
        WrappingMode::MirroredRepeat => TextureAddressMode::MirroredRepeat,
        WrappingMode::Repeat => TextureAddressMode::Repeat,
    }
}

fn is_data_uri(uri: &str) -> bool {
    uri.get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
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
