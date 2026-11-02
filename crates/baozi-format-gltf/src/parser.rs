use crate::document::BufferSource;
use crate::document::ImageSource;
use crate::document::schema as json;
use crate::document::{MagFilter, MinFilter, WrappingMode};
use crate::document::{Mode, Semantic};
use crate::{
    GltfFailure, GltfSceneSelection, admission, external_buffer_role, external_image_role,
    gltf_import_option_key,
};
use baozi_import::authoring::{
    AttributeSemantic, AuthoredTransform, AuthoringRange, CameraLocalId, CameraProjection,
    ExtensionMediaType, GeometryMeshInstanceAuthoring, GeometryTopologyAuthoring, ImageLocalId,
    InfluenceAuthoring, LogicalMeshLocalId, Mat4, MaterialAssignment, MaterialLocalId,
    MipmapFilter, NodeAuthoringNode, NodeLocalId, NodeParent, Presence, Quat,
    ResourceIdentityLocalId, SamplerAuthoring, SamplerLocalId, SamplerState, SkinLocalId,
    TexelFilter, TextureAddressMode, TextureAuthoring, TextureLocalId, Vec3,
};
use baozi_import::{
    DiagnosticCode, DiagnosticSeverity, ImportStorageClass, ImportWorkClass, ImportWorkScope,
    MeteredOrderedMap, MeteredString, MeteredVec, ParseFailure, ParseResult, ParseScope,
    SceneDraft, SourceScopeDisposition, SourceScopeEntityKind, SourceScopeRecordBuilder,
    SourceScopeSelectionOutcome, SourceScopeSelectionRequest,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::{DecodeError, DecodeSliceError, Engine as _};
use std::fmt;

mod accessor;
use accessor::{AccessorContract, validate_accessor};
mod animation;
mod extras;
mod influences;
mod lights;
mod materials;
use materials::MaterialContracts;
mod model;
mod morph;
use model::*;
mod normalized;
mod source;
mod source_profile;
mod uri;

type Result<T> = std::result::Result<T, ContentError>;

const PARSER_POLL_INTERVAL: usize = 4_096;

use crate::document::is_supported_extension;

#[derive(Debug)]
enum ContentError {
    Scope(ParseFailure),
    Format(GltfFailure),
}

impl ContentError {
    fn parse(
        _source: impl fmt::Display,
        _location: Option<()>,
        message: impl fmt::Display,
    ) -> Self {
        Self::Format(GltfFailure::invalid_document(message))
    }

    fn into_format_failure(self) -> GltfFailure {
        match self {
            Self::Scope(_) => GltfFailure::internal("scope failure escaped its ownership boundary"),
            Self::Format(failure) => failure,
        }
    }
}

impl fmt::Display for ContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scope(error) => error.fmt(formatter),
            Self::Format(error) => error.fmt(formatter),
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

fn selected_count(ctx: &ParseScope<'_>, values: &[bool]) -> Result<usize> {
    let mut count = 0usize;
    for chunk in values.chunks(PARSER_POLL_INTERVAL) {
        ctx.charge_work(to_u64(chunk.len()))?;
        for selected in chunk {
            count = count.checked_add(usize::from(*selected)).ok_or_else(|| {
                ContentError::from(GltfFailure::internal("selection count overflow"))
            })?;
        }
    }
    Ok(count)
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

struct PreparedDocument<'scope, 'document> {
    node_graph: NodeGraph<'document, 'scope>,
    resource_closure: ResourceClosure<'scope>,
    material_contracts: MaterialContracts<'scope>,
    geometry_contracts: GeometryContracts<'scope>,
    resolved_texture_uris: ResolvedTextureUris<'scope>,
    animation_contracts: animation::AnimationContracts<'scope, 'document>,
    morph_contracts: morph::MorphContracts<'scope>,
}

fn read_gltf_inner(ctx: &ParseScope<'_>) -> Result<SceneDraft> {
    let normalized = read_normalized_gltf(ctx)?;
    normalized::submit(ctx, &normalized)
}

/// Completes decoding before canonical construction can enter its own call chain.
/// Prepared contracts, typed rows, and structural storage are released before submission.
fn read_normalized_gltf<'scope>(ctx: &'scope ParseScope<'_>) -> Result<NormalizedGltf<'scope>> {
    let gltf = crate::document::Document::decode(ctx, admission::admit(ctx)?)?;
    let node_graph = preflight_node_graph(ctx, &gltf)?;
    let animation_contracts = animation::preflight(ctx, &gltf, node_graph.selected.as_slice())?;
    let resource_closure = build_resource_closure(ctx, &gltf, &node_graph, &animation_contracts)?;
    source::preflight(
        ctx,
        gltf.source_map(),
        &gltf,
        &node_graph,
        &resource_closure,
        &animation_contracts,
    )?;
    report_uninterpreted_extension_declarations(ctx, gltf.tables())?;
    let material_contracts = materials::preflight(ctx, &gltf, &resource_closure)?;
    let lights = lights::prepare(ctx, &gltf, resource_closure.lights.as_slice())?;
    let repeated_mesh_string_bytes =
        preflight_canonical_base_strings(ctx, &gltf, &node_graph, &resource_closure, &lights)?;
    let resolved_texture_uris =
        preflight_texture_strings(ctx, &gltf, repeated_mesh_string_bytes, &resource_closure)?;
    let material_count = with_poll(ctx, || gltf.materials().count())?;
    let geometry_contracts =
        preflight_geometry_contracts(ctx, &gltf, material_count, &resource_closure)?;
    let morph_contracts = morph::preflight(
        ctx,
        &gltf,
        &resource_closure,
        &geometry_contracts,
        node_graph.selected.as_slice(),
    )?;
    preflight_skin_accessors(ctx, &gltf, &resource_closure)?;
    let prepared = PreparedDocument {
        node_graph,
        resource_closure,
        material_contracts,
        geometry_contracts,
        resolved_texture_uris,
        animation_contracts,
        morph_contracts,
    };
    record_source_scope(
        ctx,
        &gltf,
        &prepared.node_graph,
        &prepared.animation_contracts,
        &prepared.resource_closure,
    )?;
    let buffers = load_buffers(ctx, &gltf, gltf.bin_bytes(), &prepared.resource_closure)?;
    accessor::validate_sparse_indices(ctx, &gltf, buffers.as_slice(), &prepared.resource_closure)?;
    let normalized = normalize_document(ctx, &gltf, buffers, &prepared, gltf.source_map(), lights)?;
    drop(prepared);
    drop(gltf);
    Ok(normalized)
}

fn owned_name<'scope>(
    ctx: &'scope ParseScope<'_>,
    value: Option<&str>,
    resource: &'static str,
) -> Result<Option<MeteredString<'scope>>> {
    value
        .map(|value| {
            let mut output = ctx.metered_string(ImportStorageClass::FormatNormalization, resource);
            output.try_reserve(value.len())?;
            let mut remaining = value;
            while !remaining.is_empty() {
                let mut end = remaining.len().min(PARSER_POLL_INTERVAL);
                while !remaining.is_char_boundary(end) {
                    end -= 1;
                }
                let (chunk, rest) = remaining.split_at(end);
                ctx.charge_work(to_u64(chunk.len()))?;
                output.try_push_str(chunk)?;
                remaining = rest;
            }
            Ok(output)
        })
        .transpose()
}

/// Called only after retained payloads have passed profile admission. Unknown selected
/// behavior is a hard error, so these diagnostics never claim to omit a retained payload.
fn report_uninterpreted_extension_declarations(
    ctx: &ParseScope<'_>,
    root: &json::Tables<'_>,
) -> Result<()> {
    for name in &root.extensions_used {
        ctx.charge_work(1)?;
        if !is_supported_extension(name) {
            push_warning(
                ctx,
                "gltf.extension_uninterpreted_declaration",
                format_args!(
                    "glTF optional extension {name:?} has no retained payload; its declaration is not interpreted"
                ),
            )?;
        }
    }
    Ok(())
}

fn preflight_canonical_base_strings(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    graph: &NodeGraph<'_, '_>,
    closure: &ResourceClosure<'_>,
    lights: &lights::LightStorage<'_>,
) -> Result<usize> {
    with_poll(ctx, || {
        let mut bytes = 0usize;
        let limit = ctx.limits().max_canonical_string_bytes;

        for (mesh_index, mesh) in gltf.meshes().enumerate() {
            poll_parser_loop(ctx, mesh_index)?;
            if !closure
                .meshes
                .as_slice()
                .get(mesh_index)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(name) = mesh.name() {
                enforce_usize_limit(
                    ctx,
                    "max_string_bytes",
                    name.len(),
                    ctx.limits().max_string_bytes,
                )?;
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
                let primitive_count = with_poll(ctx, || mesh.primitives().len())?;
                for primitive_index in 0..primitive_count {
                    let generated_len = primitive_name_len(ctx, name, primitive_index)?;
                    enforce_usize_limit(
                        ctx,
                        "max_string_bytes",
                        generated_len,
                        ctx.limits().max_string_bytes,
                    )?;
                }
            }
        }

        for (node_index, selected) in graph.selected.as_slice().iter().copied().enumerate() {
            poll_parser_loop(ctx, node_index)?;
            if !selected {
                continue;
            }
            let Some(node) = graph.document.node(node_index) else {
                return Err(gltf_parse_error(
                    ctx,
                    format_args!("glTF node graph references missing node {node_index}"),
                ));
            };
            if let Some(name) = node.name() {
                enforce_usize_limit(
                    ctx,
                    "max_string_bytes",
                    name.len(),
                    ctx.limits().max_string_bytes,
                )?;
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
        }

        for (skin_index, skin) in gltf.skins().enumerate() {
            poll_parser_loop(ctx, skin_index)?;
            if closure.skins.as_slice().get(skin_index).copied() != Some(true) {
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
            if !closure
                .cameras
                .as_slice()
                .get(camera_index)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(name) = camera.name()
                && name.len() <= ctx.limits().max_string_bytes
            {
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
        }

        for name in lights.names.as_slice() {
            ctx.charge_work(1)?;
            if let Some(name) = name {
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
    gltf: &crate::document::Document<'_>,
    repeated_mesh_string_bytes: usize,
    closure: &ResourceClosure<'_>,
) -> Result<ResolvedTextureUris<'scope>> {
    with_poll(ctx, || {
        let mut bytes = repeated_mesh_string_bytes;
        let limit = ctx.limits().max_canonical_string_bytes;
        let image_count = with_poll(ctx, || gltf.images().len())?;
        let mut resolved_uris = ctx.metered_ordered_map(
            ImportStorageClass::FormatNormalization,
            "gltf-resolved-texture-uris",
        );

        for (index, image) in gltf.images().enumerate() {
            ctx.charge_work(1)?;
            if closure.images.as_slice()[index]
                && let Some(name) = image.name()
            {
                enforce_usize_limit(
                    ctx,
                    "max_string_bytes",
                    name.len(),
                    ctx.limits().max_string_bytes,
                )?;
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
        }
        for (index, sampler) in gltf.samplers().enumerate() {
            ctx.charge_work(1)?;
            if closure.samplers.as_slice()[index]
                && let Some(name) = sampler.name()
            {
                enforce_usize_limit(
                    ctx,
                    "max_string_bytes",
                    name.len(),
                    ctx.limits().max_string_bytes,
                )?;
                add_repeated_string_bytes(ctx, &mut bytes, name.len(), limit)?;
            }
        }

        for (material_index, material) in gltf.materials().enumerate() {
            poll_parser_loop(ctx, material_index)?;
            if !closure
                .materials
                .as_slice()
                .get(material_index)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
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
    texture: crate::document::Texture<'_>,
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
    if let ImageSource::Uri { uri, .. } = image.source() {
        enforce_usize_limit(
            ctx,
            "max_string_bytes",
            uri.len(),
            ctx.limits().max_string_bytes,
        )?;
    }

    let image_index = image.index();
    if image_index >= image_count {
        return Err(gltf_parse_error(
            ctx,
            format_args!("glTF texture references missing image {image_index}"),
        ));
    }
    let resolved_uri_len = match image.source() {
        ImageSource::Uri { uri, .. } if !is_data_uri(uri) => {
            if !resolved_uris.contains_key(&image_index) {
                let reference = uri::relative_path(ctx, uri, uri::Resource::Image(image_index))?;
                let reference = reference.as_str();
                let resolution_bytes = texture_path_resolution_allowance(ctx, reference)?;
                let _resolution_allowance = ctx.metered_opaque_allowance(
                    ImportStorageClass::FormatNormalization,
                    resolution_bytes,
                )?;
                let resolved = ctx.with_work(
                    ImportWorkClass::WorkUnits,
                    to_u64(ctx.source().as_str().len().saturating_add(reference.len())),
                    |work| -> Result<_> {
                        let resolved =
                            ctx.source().resolve_from_file(reference).map_err(|error| {
                                GltfFailure::invalid_reference(
                                    "texture-uri",
                                    Some(image_index),
                                    error,
                                )
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
            Some(
                resolved_uris
                    .get(&image_index)
                    .ok_or_else(|| {
                        gltf_parse_error(
                            ctx,
                            format_args!(
                                "glTF texture resolution plan is missing image {image_index}"
                            ),
                        )
                    })?
                    .len(),
            )
        }
        ImageSource::Uri { .. } | ImageSource::View { .. } => None,
    };

    let texture_name = texture.name();
    if let Some(name) = texture_name {
        enforce_usize_limit(
            ctx,
            "max_string_bytes",
            name.len(),
            ctx.limits().max_string_bytes,
        )?;
    }
    if let Some(name) = texture_name {
        add_repeated_string_bytes(ctx, bytes, name.len(), limit)?;
    }
    if let Some(resolved_uri_len) = resolved_uri_len {
        add_repeated_string_bytes(ctx, bytes, resolved_uri_len, limit)?;
    }
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

fn primitive_name_len(
    ctx: &ParseScope<'_>,
    mesh_name: &str,
    primitive_index: usize,
) -> Result<usize> {
    let suffix_len = if primitive_index == 0 {
        0
    } else {
        1usize
            .checked_add(decimal_digits(primitive_index))
            .ok_or_else(|| ctx.allocation_failure("gltf-primitive-name-length"))?
    };
    mesh_name
        .len()
        .checked_add(suffix_len)
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-name-length").into())
}

fn decimal_digits(mut value: usize) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
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

fn accessor_provenance(
    ctx: &ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    accessor: crate::document::Accessor<'_>,
) -> Result<AccessorSources> {
    // Collection work is prepaid before the caller enters its decode work scope.
    let sparse = accessor.sparse();
    let views = [
        accessor.view(),
        sparse.as_ref().map(|sparse| sparse.indices().view()),
        sparse.as_ref().map(|sparse| sparse.values().view()),
    ];
    let mut sources = [None; AccessorSources::MAX_SOURCES];
    for (source, view) in sources.iter_mut().zip(views) {
        let Some(view) = view else { continue };
        let buffer_index = view.buffer().index();
        *source = Some(
            buffers
                .get(buffer_index)
                .and_then(Option::as_ref)
                .map(BufferData::provenance)
                .ok_or_else(|| {
                    gltf_parse_error(
                        ctx,
                        format_args!(
                            "glTF accessor {} references unavailable buffer {buffer_index}",
                            accessor.index()
                        ),
                    )
                })?,
        );
    }
    Ok(AccessorSources::new(sources))
}

/// Conservative work for collecting R references, heap sorting, deduplicating, and copying them.
/// Heap construction takes at most 2R comparisons; extraction at most 2R * bit_length(R).
/// The remaining 3R covers collection, adjacent comparisons, and the retained copy.
fn provenance_work_bound(ctx: &ParseScope<'_>, references: usize) -> Result<u64> {
    let levels = u64::from(usize::BITS - references.leading_zeros());
    to_u64(references)
        .checked_mul(2 * levels + 5)
        .ok_or_else(|| ctx.allocation_failure("gltf-provenance-work").into())
}

/// Appends one owner's sorted unique sources, preserving every earlier owner's range.
/// Callers prepay provenance_work_bound before entering their non-nestable decode work scope.
/// Pending storage is reused between owners; the retained table only needs unique-source capacity.
fn try_extend_provenance_unique(
    ctx: &ParseScope<'_>,
    values: &mut MeteredVec<'_, u32>,
    pending: &mut MeteredVec<'_, u32>,
) -> Result<()> {
    let mut until_poll = 0;
    let mut control = || -> Result<()> {
        if until_poll == 0 {
            ctx.poll()?;
            until_poll = PARSER_POLL_INTERVAL;
        }
        until_poll -= 1;
        Ok(())
    };
    pending.try_sort_unstable_by(|left, right| -> Result<_> {
        control()?;
        Ok(left.cmp(right))
    })?;
    let retained = pending.try_partition_dedup_by(|left, right| -> Result<_> {
        control()?;
        Ok(left == right)
    })?;
    pending.truncate(retained);
    for &source in pending.as_slice() {
        control()?;
        values.try_push_reserved(source)?;
    }
    pending.truncate(0);
    Ok(())
}

fn load_buffers<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    glb_bin_bytes: Option<&[u8]>,
    closure: &ResourceClosure<'_>,
) -> Result<BufferSet<'scope>> {
    let buffer_count = with_poll(ctx, || gltf.buffers().count())?;
    let mut buffers = ctx.metered_vec(ImportStorageClass::BackendDecode, "gltf-buffer-list");
    buffers.try_reserve(buffer_count)?;
    let mut external_resource_count = 0usize;
    for (ordinal, buffer) in gltf.buffers().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        if !closure
            .buffers
            .as_slice()
            .get(ordinal)
            .copied()
            .unwrap_or(false)
        {
            buffers.try_push_reserved(None)?;
            continue;
        }
        let loaded = match buffer.source() {
            BufferSource::Bin => {
                let blob = glb_bin_bytes.ok_or_else(|| {
                    ContentError::parse(
                        ctx.source(),
                        None,
                        format_args!("glTF buffer {} requires a GLB BIN chunk", buffer.index()),
                    )
                })?;
                copy_buffer_prefix(
                    ctx,
                    blob,
                    buffer.length(),
                    "gltf-glb-buffer",
                    buffer.index(),
                )?
            }
            BufferSource::Uri(uri) if is_data_uri(uri) => {
                load_data_uri_buffer(ctx, uri, buffer.index(), buffer.length())?
            }
            BufferSource::Uri(uri) => {
                let resource_index = external_resource_count
                    .checked_add(1)
                    .ok_or_else(|| ctx.allocation_failure("gltf-external-buffer-resource"))?;
                let provenance =
                    authoring_u32(ctx, resource_index, "gltf-external-buffer-provenance")?;
                let loaded =
                    load_external_buffer(ctx, uri, buffer.index(), buffer.length(), provenance)?;
                external_resource_count = external_resource_count
                    .checked_add(1)
                    .ok_or_else(|| ctx.allocation_failure("gltf-external-buffer-count"))?;
                loaded
            }
        };

        if loaded.len() != buffer.length() {
            return Err(ContentError::parse(
                ctx.source(),
                None,
                format_args!(
                    "glTF buffer {} declares {} bytes but only {} bytes are available",
                    buffer.index(),
                    buffer.length(),
                    loaded.len()
                ),
            ));
        }
        buffers.try_push_reserved(Some(loaded))?;
    }
    Ok(BufferSet { values: buffers })
}

fn load_data_uri_buffer<'scope>(
    ctx: &'scope ParseScope<'_>,
    uri: &str,
    buffer_index: usize,
    declared_length: usize,
) -> Result<BufferData<'scope>> {
    let decoded = decode_data_uri_payload(
        ctx,
        uri,
        uri::Resource::Buffer(buffer_index),
        "data-uri-buffer",
        "gltf-data-uri-buffer",
        Some(declared_length),
    )?;
    Ok(BufferData::Owned(decoded.bytes))
}

struct DecodedDataUri<'scope, 'uri> {
    bytes: MeteredVec<'scope, u8>,
    media_type: Option<&'uri str>,
}

fn decode_data_uri_payload<'scope, 'uri>(
    ctx: &'scope ParseScope<'_>,
    uri: &'uri str,
    resource: uri::Resource,
    reference: &'static str,
    allocation: &'static str,
    retained_length: Option<usize>,
) -> Result<DecodedDataUri<'scope, 'uri>> {
    let subject = resource.name();
    let subject_index = resource.index();
    enforce_usize_limit(
        ctx,
        "max_string_bytes",
        uri.len(),
        ctx.limits().max_string_bytes,
    )?;
    let Some(payload) = uri.get(5..).filter(|_| is_data_uri(uri)) else {
        return Err(gltf_parse_error(
            ctx,
            format_args!("glTF {subject} {subject_index} has an invalid data URI"),
        ));
    };
    let Some((metadata, encoded)) = payload.split_once(',') else {
        return Err(gltf_parse_error(
            ctx,
            format_args!("glTF {subject} {subject_index} data URI has no comma separator"),
        ));
    };
    if metadata.contains('%') {
        return Err(resource.unsupported(ctx, "percent-encoded data URLs are unsupported"));
    }
    if !metadata
        .rsplit_once(';')
        .is_some_and(|(_, encoding)| encoding.eq_ignore_ascii_case("base64"))
    {
        return Err(resource.unsupported(ctx, "non-base64 data URI is unsupported"));
    }

    let decoded_len = base64_decoded_len(ctx, encoded, resource)?;
    let decoded_len = usize::try_from(decoded_len)
        .map_err(|_| ctx.allocation_failure("gltf-data-uri-decoded-length"))?;
    let retained_length = retained_length.unwrap_or(decoded_len);
    let stored_length = retained_length.min(decoded_len);
    let decoded = ctx.with_work_group(
        [
            (ImportWorkClass::DataUriBytes, to_u64(stored_length)),
            (
                ImportWorkClass::DecodeWork,
                u64::try_from(encoded.len()).unwrap_or(u64::MAX),
            ),
        ],
        |work| -> Result<_> {
            let mut decoded = ctx.metered_vec(ImportStorageClass::BackendDecode, allocation);
            decoded.try_reserve(stored_length)?;
            let mut decoded_length = 0usize;
            let mut chunk_output = [0_u8; PARSER_POLL_INTERVAL];
            for (chunk_index, chunk) in encoded.as_bytes().chunks(PARSER_POLL_INTERVAL).enumerate()
            {
                work.poll()?;
                let chunk_start = chunk_index * PARSER_POLL_INTERVAL;
                let chunk_capacity = base64::decoded_len_estimate(chunk.len());
                let output = chunk_output.get_mut(..chunk_capacity).ok_or_else(|| {
                    ContentError::from(GltfFailure::internal(
                        "base64 chunk capacity exceeds the bounded decode buffer",
                    ))
                })?;
                let chunk_written = BASE64_STANDARD.decode_slice(chunk, output)
                    .map_err(|error| match error {
                        DecodeSliceError::DecodeError(error) => {
                            let error = offset_base64_decode_error(error, chunk_start);
                            GltfFailure::invalid_reference(
                                reference,
                                Some(subject_index),
                                format_args!("invalid base64: {error}"),
                            )
                        }
                        DecodeSliceError::OutputSliceTooSmall => {
                            GltfFailure::internal("base64 decoder exceeded its preflight capacity")
                        }
                    })?;
                let retained_chunk = stored_length
                    .saturating_sub(decoded_length)
                    .min(chunk_written);
                decoded.try_extend_from_slice(&output[..retained_chunk])?;
                decoded_length = decoded_length
                    .checked_add(chunk_written)
                    .ok_or_else(|| ctx.allocation_failure("gltf-data-uri-buffer"))?;
                work.complete([to_u64(retained_chunk), to_u64(chunk.len())])?;
            }
            if decoded_length != decoded_len || decoded.len() != stored_length {
                return Err(ContentError::from(GltfFailure::internal(
                    "base64 decode length changed after preflight",
                )));
            }
            if decoded_len < retained_length {
                return Err(gltf_parse_error(
                    ctx,
                    format_args!(
                        "glTF {subject} {subject_index} declares {retained_length} bytes but only {decoded_len} bytes are available"
                    ),
                ));
            }
            Ok(decoded)
        },
    )?;
    let media_type = metadata
        .split(';')
        .next()
        .filter(|media_type| !media_type.is_empty());
    Ok(DecodedDataUri {
        bytes: decoded,
        media_type,
    })
}

fn base64_decoded_len(ctx: &ParseScope<'_>, encoded: &str, resource: uri::Resource) -> Result<u64> {
    let subject = resource.name();
    let subject_index = resource.index();
    if encoded.is_empty() {
        return Ok(0);
    }

    let mut padding = 0usize;
    let mut padding_started = false;
    let mut invalid_padding = false;
    for chunk in encoded.as_bytes().chunks(PARSER_POLL_INTERVAL) {
        ctx.poll()?;
        for &byte in chunk {
            if byte == b'%' {
                return Err(resource.unsupported(ctx, "percent-encoded data URLs are unsupported"));
            }
            if byte.is_ascii_whitespace() {
                return Err(gltf_parse_error(
                    ctx,
                    format_args!(
                        "glTF {subject} {subject_index} data URI base64 contains whitespace"
                    ),
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
            format_args!("glTF {subject} {subject_index} data URI base64 length is not padded"),
        ));
    }
    if invalid_padding {
        return Err(gltf_parse_error(
            ctx,
            format_args!("glTF {subject} {subject_index} data URI base64 padding is invalid"),
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
    buffer_index: usize,
    declared_length: usize,
    provenance: u32,
) -> Result<BufferData<'scope>> {
    enforce_usize_limit(
        ctx,
        "max_string_bytes",
        uri.len(),
        ctx.limits().max_string_bytes,
    )?;
    let reference = uri::relative_path(ctx, uri, uri::Resource::Buffer(buffer_index))?;
    let origin = ctx.root_origin();
    let outcome = ctx.dependency(&origin, &external_buffer_role(), reference.as_str())?;
    let snapshot = ctx.into_available_dependency(outcome)?;
    if snapshot.bytes().len() < declared_length {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!(
                "glTF buffer {buffer_index} declares {declared_length} bytes but only {} bytes are available",
                snapshot.bytes().len()
            ),
        ));
    }
    let identity = uri::profile_identity(ctx, snapshot.identity().path().as_str())?;
    Ok(BufferData::External {
        identity,
        snapshot,
        declared_length,
        provenance,
    })
}

fn copy_buffer<'scope>(
    ctx: &'scope ParseScope<'_>,
    source: &[u8],
    allocation: &'static str,
) -> Result<BufferData<'scope>> {
    Ok(BufferData::Owned(copy_buffer_bytes(
        ctx, source, allocation,
    )?))
}

fn copy_buffer_prefix<'scope>(
    ctx: &'scope ParseScope<'_>,
    source: &[u8],
    declared_length: usize,
    allocation: &'static str,
    buffer_index: usize,
) -> Result<BufferData<'scope>> {
    if source.len() < declared_length {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!(
                "glTF buffer {buffer_index} declares {declared_length} bytes but only {} bytes are available",
                source.len()
            ),
        ));
    }
    copy_buffer(ctx, &source[..declared_length], allocation)
}

fn copy_buffer_bytes<'scope>(
    ctx: &'scope ParseScope<'_>,
    source: &[u8],
    allocation: &'static str,
) -> Result<MeteredVec<'scope, u8>> {
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
    Ok(copied)
}

fn gltf_parse_error(ctx: &ParseScope<'_>, message: impl fmt::Display) -> ContentError {
    ContentError::parse(ctx.source(), None, message)
}

fn normalize_document<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    buffers: BufferSet<'scope>,
    prepared: &PreparedDocument<'_, '_>,
    source_map: &crate::source_map::GltfSourceMap<'_>,
    lights: lights::LightStorage<'scope>,
) -> Result<NormalizedGltf<'scope>> {
    let external_image_resource_offset = authoring_u32(
        ctx,
        buffers
            .external_resource_count()
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-external-image-resource-offset"))?,
        "gltf-external-image-resource-offset",
    )?;
    let external_images = load_external_image_resources(
        ctx,
        gltf,
        &prepared.resolved_texture_uris,
        &prepared.resource_closure,
        external_image_resource_offset,
    )?;
    let appearance_storage =
        build_appearance_storage(ctx, gltf, buffers.as_slice(), &external_images, prepared)?;
    let geometry_storage = build_geometry_storage(
        ctx,
        gltf,
        buffers.as_slice(),
        &prepared.geometry_contracts,
        &appearance_storage,
    )?;
    let morph_storage = morph::build_storage(
        ctx,
        gltf,
        buffers.as_slice(),
        &prepared.geometry_contracts,
        &prepared.morph_contracts,
    )?;
    let cameras = build_cameras(ctx, gltf, &prepared.resource_closure)?;
    let node_storage = build_scene_nodes(
        ctx,
        &prepared.node_graph,
        &geometry_storage.logical_mesh_ids,
        &cameras,
        &lights,
        prepared.resource_closure.node_lights.as_slice(),
    )?;
    let animation_storage = animation::build_storage(
        ctx,
        buffers.as_slice(),
        node_storage.node_ids.as_slice(),
        node_storage.mesh_instances.as_slice(),
        &prepared.animation_contracts,
    )?;
    let skin_storage = build_skins(
        ctx,
        gltf,
        buffers.as_slice(),
        node_storage.node_ids.as_slice(),
        &prepared.resource_closure,
    )?;
    let mesh_instance_deformations = build_mesh_instance_deformations(
        ctx,
        gltf,
        node_storage.mesh_instances.as_slice(),
        skin_storage.skin_ids.as_slice(),
    )?;
    let mut resources = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-normalized-buffer-resources",
    );
    resources.try_reserve(buffers.external_resource_count())?;
    let mut buffers = buffers;
    while let Some(buffer) = buffers.values.pop() {
        let source_index = buffers.values.len();
        ctx.charge_work(1)?;
        if let Some(BufferData::External { identity, .. }) = buffer {
            resources.try_push_reserved(BufferResource {
                source_index,
                identity,
            })?;
        }
    }
    let sources = source_profile::SelectedSourceProfile::from_source_map(ctx, source_map, |id| {
        source::selected(
            id,
            gltf,
            &prepared.node_graph,
            &prepared.resource_closure,
            &prepared.animation_contracts,
        )
    })?;
    let mut normalized = NormalizedGltf {
        selection_request: prepared.node_graph.selection_request,
        sources,
        appearance: appearance_storage,
        geometry: geometry_storage,
        cameras,
        lights,
        nodes: node_storage,
        skins: skin_storage,
        instance_deformations: mesh_instance_deformations,
        buffers: BufferResources { values: resources },
        external_images,
        animations: animation_storage,
        morphs: morph_storage,
    };
    source_profile::associate(ctx, gltf, prepared, &mut normalized)?;
    Ok(normalized)
}

fn load_external_image_resources<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    closure: &ResourceClosure<'_>,
    resource_offset: u32,
) -> Result<ExternalImageResources<'scope>> {
    let mut values = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-external-image-resources",
    );
    values.try_reserve(resolved_texture_uris.values.len())?;
    for (ordinal, image) in gltf.images().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        if !closure
            .images
            .as_slice()
            .get(ordinal)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let ImageSource::Uri { uri, .. } = image.source() else {
            continue;
        };
        if resolved_texture_uris.get(&image.index()).is_none() {
            continue;
        }
        let reference = uri::relative_path(ctx, uri, uri::Resource::Image(image.index()))?;
        let origin = ctx.root_origin();
        let outcome = ctx.dependency(&origin, &external_image_role(), reference.as_str())?;
        let snapshot = ctx.into_available_dependency(outcome)?;
        let identity = uri::profile_identity(ctx, snapshot.identity().path().as_str())?;
        values.try_push_reserved(ExternalImageResource {
            image_index: image.index(),
            identity,
            snapshot,
        })?;
    }
    Ok(ExternalImageResources {
        values,
        resource_offset,
    })
}

fn build_appearance_storage<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    external_images: &ExternalImageResources<'scope>,
    prepared: &PreparedDocument<'_, '_>,
) -> Result<AppearanceStorage<'scope>> {
    let resolved_texture_uris = &prepared.resolved_texture_uris;
    let material_contracts = &prepared.material_contracts;
    let closure = &prepared.resource_closure;
    let material_count = with_poll(ctx, || gltf.materials().count())?;
    let mut storage = AppearanceStorage {
        images: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-images",
        ),
        image_names: ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-normalized-image-names",
        ),
        encoded_image_payloads: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-encoded-image-payloads",
        ),
        samplers: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-samplers",
        ),
        sampler_names: ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-normalized-sampler-names",
        ),
        sampler_provenance: ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-sampler-provenance",
        ),
        textures: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-textures",
        ),
        texture_provenance: ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-texture-provenance",
        ),
        texture_names: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-texture-names",
        ),
        bindings: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-material-bindings",
        ),
        materials: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-materials",
        ),
        image_ids: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            gltf.images().count(),
            None,
            "gltf-image-id-map",
        )?,
        sampler_ids: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            gltf.samplers().count(),
            None,
            "gltf-sampler-id-map",
        )?,
        texture_ids: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            gltf.textures().count(),
            None,
            "gltf-texture-id-map",
        )?,
        default_sampler: None,
        default_material: None,
        material_ids: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            material_count,
            None,
            "gltf-material-id-map",
        )?,
    };
    build_shared_appearance_resources(
        ctx,
        gltf,
        buffers,
        resolved_texture_uris,
        external_images,
        closure,
        &mut storage,
    )?;
    let selected_material_count = selected_count(ctx, closure.materials.as_slice())?;
    let material_count = selected_material_count
        .checked_add(usize::from(
            prepared.geometry_contracts.needs_default_material,
        ))
        .ok_or_else(|| ctx.allocation_failure("gltf-authoring-material-count"))?;
    authoring_u32(ctx, material_count, "gltf-authoring-material-count")?;
    storage.materials.try_reserve(material_count)?;
    for (material_index, material) in gltf.materials().enumerate() {
        poll_parser_loop(ctx, material_index)?;
        if !closure
            .materials
            .as_slice()
            .get(material_index)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let local_id = MaterialLocalId::new(authoring_u32(
            ctx,
            storage.materials.len(),
            "gltf-authoring-material-id",
        )?);
        let contract = material_contracts.material(ctx, material_index)?;
        materials::add_authoring(
            ctx,
            &mut storage,
            material,
            resolved_texture_uris,
            external_images,
            contract,
        )?;
        let Some(slot) = storage.material_ids.as_mut_slice().get_mut(material_index) else {
            return Err(gltf_parse_error(ctx, "glTF material id map is incomplete"));
        };
        *slot = Some(local_id);
    }
    if prepared.geometry_contracts.needs_default_material {
        materials::add_default_authoring(ctx, &mut storage)?;
    }
    Ok(storage)
}

fn build_shared_appearance_resources<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    closure: &ResourceClosure<'_>,
    storage: &mut AppearanceStorage<'scope>,
) -> Result<()> {
    let selected_image_count = selected_count(ctx, closure.images.as_slice())?;
    storage.images.try_reserve(selected_image_count)?;
    storage.image_names.try_reserve(selected_image_count)?;
    storage
        .encoded_image_payloads
        .try_reserve(selected_image_count)?;
    for (image_index, image) in gltf.images().enumerate() {
        poll_parser_loop(ctx, image_index)?;
        if !closure
            .images
            .as_slice()
            .get(image_index)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let (payload, provenance, media_type) = match image.source() {
            ImageSource::Uri { uri, mime_type } if is_data_uri(uri) => {
                let decoded = decode_data_uri_payload(
                    ctx,
                    uri,
                    uri::Resource::Image(image_index),
                    "data-uri-image",
                    "gltf-data-uri-image",
                    None,
                )?;
                let media_type =
                    image_media_type(ctx, image_index, mime_type.or(decoded.media_type))?;
                (
                    store_encoded_image_payload(storage, decoded.bytes)?,
                    0,
                    media_type,
                )
            }
            ImageSource::Uri { mime_type, .. } => {
                let resolved = resolved_texture_uris.get(&image_index).ok_or_else(|| {
                    gltf_parse_error(
                        ctx,
                        format_args!("glTF texture resolution plan is missing image {image_index}"),
                    )
                })?;
                let (resource_index, _) =
                    external_images.get(ctx, image_index)?.ok_or_else(|| {
                        gltf_parse_error(
                            ctx,
                            format_args!("glTF texture resource is missing image {image_index}"),
                        )
                    })?;
                let media_type =
                    mime_type
                        .map(ExtensionMediaType::new)
                        .transpose()
                        .map_err(|_| {
                            gltf_parse_error(
                                ctx,
                                format_args!("glTF image {image_index} has an invalid media type"),
                            )
                        })?;
                complete_canonical_string_work(ctx, resolved.len())?;
                (
                    ImagePayloadStorage::SessionResource(ResourceIdentityLocalId::new(
                        resource_index,
                    )),
                    resource_index,
                    media_type,
                )
            }
            ImageSource::View { view, mime_type } => {
                let (bytes, provenance) = copy_buffer_view_image(ctx, buffers, view, image_index)?;
                (
                    store_encoded_image_payload(storage, bytes)?,
                    provenance,
                    image_media_type(ctx, image_index, Some(mime_type))?,
                )
            }
        };
        let local_id = ImageLocalId::new(authoring_u32(
            ctx,
            storage.images.len(),
            "gltf-authoring-image-id",
        )?);
        storage.images.try_push(ImageStorage {
            payload,
            provenance,
            media_type,
        })?;
        complete_canonical_string_work(ctx, image.name().map_or(0, str::len))?;
        storage.image_names.try_push_reserved(owned_name(
            ctx,
            image.name(),
            "gltf-normalized-image-name",
        )?)?;
        let Some(slot) = storage.image_ids.as_mut_slice().get_mut(image_index) else {
            return Err(gltf_parse_error(ctx, "glTF image id map is incomplete"));
        };
        *slot = Some(local_id);
    }

    let selected_sampler_count = selected_count(ctx, closure.samplers.as_slice())?;
    storage.samplers.try_reserve(selected_sampler_count)?;
    storage
        .sampler_names
        .try_reserve(selected_sampler_count.saturating_add(1))?;
    storage
        .sampler_provenance
        .try_reserve(selected_sampler_count.saturating_add(1))?;
    for (sampler_index, sampler) in gltf.samplers().enumerate() {
        poll_parser_loop(ctx, sampler_index)?;
        if !closure
            .samplers
            .as_slice()
            .get(sampler_index)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let local_id = SamplerLocalId::new(authoring_u32(
            ctx,
            storage.samplers.len(),
            "gltf-authoring-sampler-id",
        )?);
        storage
            .samplers
            .try_push(SamplerAuthoring::new(map_sampler(sampler)))?;
        complete_canonical_string_work(ctx, sampler.name().map_or(0, str::len))?;
        storage.sampler_names.try_push_reserved(owned_name(
            ctx,
            sampler.name(),
            "gltf-normalized-sampler-name",
        )?)?;
        storage.sampler_provenance.try_push(0)?;
        let Some(slot) = storage.sampler_ids.as_mut_slice().get_mut(sampler_index) else {
            return Err(gltf_parse_error(ctx, "glTF sampler id map is incomplete"));
        };
        *slot = Some(local_id);
    }

    let selected_texture_count = selected_count(ctx, closure.textures.as_slice())?;
    storage.textures.try_reserve(selected_texture_count)?;
    storage
        .texture_provenance
        .try_reserve(selected_texture_count)?;
    storage.texture_names.try_reserve(selected_texture_count)?;
    for (texture_index, texture) in gltf.textures().enumerate() {
        poll_parser_loop(ctx, texture_index)?;
        if !closure
            .textures
            .as_slice()
            .get(texture_index)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let Some(image_id) = storage
            .image_ids
            .as_slice()
            .get(texture.source().index())
            .copied()
            .flatten()
        else {
            return Err(gltf_parse_error(
                ctx,
                format_args!(
                    "glTF texture {texture_index} references an image outside the closure"
                ),
            ));
        };
        let sampler_id = match texture.sampler().index() {
            Some(sampler_index) => storage
                .sampler_ids
                .as_slice()
                .get(sampler_index)
                .copied()
                .flatten()
                .ok_or_else(|| {
                    gltf_parse_error(
                        ctx,
                        format_args!(
                            "glTF texture {texture_index} references a sampler outside the closure"
                        ),
                    )
                })?,
            None => {
                if let Some(default_sampler) = storage.default_sampler {
                    default_sampler
                } else {
                    let default_sampler = SamplerLocalId::new(authoring_u32(
                        ctx,
                        storage.samplers.len(),
                        "gltf-authoring-default-sampler-id",
                    )?);
                    storage
                        .samplers
                        .try_push(SamplerAuthoring::new(map_sampler(texture.sampler())))?;
                    storage.sampler_names.try_push_reserved(None)?;
                    storage.sampler_provenance.try_push(0)?;
                    storage.default_sampler = Some(default_sampler);
                    default_sampler
                }
            }
        };
        let local_id = TextureLocalId::new(authoring_u32(
            ctx,
            storage.textures.len(),
            "gltf-authoring-texture-id",
        )?);
        storage
            .textures
            .try_push(TextureAuthoring::new(image_id, sampler_id))?;
        storage.texture_provenance.try_push(0)?;
        storage.texture_names.try_push_reserved(owned_name(
            ctx,
            texture.name(),
            "gltf-normalized-texture-name",
        )?)?;
        let Some(slot) = storage.texture_ids.as_mut_slice().get_mut(texture_index) else {
            return Err(gltf_parse_error(ctx, "glTF texture id map is incomplete"));
        };
        *slot = Some(local_id);
    }
    Ok(())
}

fn store_encoded_image_payload<'scope>(
    storage: &mut AppearanceStorage<'scope>,
    bytes: MeteredVec<'scope, u8>,
) -> Result<ImagePayloadStorage> {
    let index = storage.encoded_image_payloads.len();
    storage.encoded_image_payloads.try_push_reserved(bytes)?;
    Ok(ImagePayloadStorage::Encoded(index))
}

fn image_media_type(
    ctx: &ParseScope<'_>,
    image_index: usize,
    media_type: Option<&str>,
) -> Result<Option<ExtensionMediaType>> {
    media_type
        .map(ExtensionMediaType::new)
        .transpose()
        .map_err(|_| {
            gltf_parse_error(
                ctx,
                format_args!("glTF image {image_index} has an invalid media type"),
            )
        })
}

fn copy_buffer_view_image<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    view: crate::document::View<'_>,
    image_index: usize,
) -> Result<(MeteredVec<'scope, u8>, u32)> {
    let buffer_index = view.buffer().index();
    let buffer = buffers
        .get(buffer_index)
        .and_then(Option::as_ref)
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!(
                    "glTF image {image_index} references unavailable buffer {buffer_index}"
                ),
            )
        })?;
    let end = view
        .offset()
        .checked_add(view.length())
        .ok_or_else(|| ctx.allocation_failure("gltf-image-buffer-view-range"))?;
    let bytes = buffer.as_slice().get(view.offset()..end).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!(
                "glTF image {image_index} bufferView {} exceeds buffer {buffer_index}",
                view.index()
            ),
        )
    })?;
    Ok((
        copy_buffer_bytes(ctx, bytes, "gltf-buffer-view-image")?,
        buffer.provenance(),
    ))
}

#[derive(Debug, Clone, Copy)]
struct PrimitiveContract {
    topology: GeometryTopologyAuthoring<'static>,
    position_count: usize,
    index_count: usize,
    face_count: usize,
    attribute_stream_count: usize,
    influences: influences::InfluenceContract,
    material_index: Option<usize>,
}

struct MeshContracts<'scope> {
    primitives: MeteredVec<'scope, PrimitiveContract>,
}

struct GeometryContracts<'scope> {
    meshes: MeteredVec<'scope, Option<MeshContracts<'scope>>>,
    needs_default_material: bool,
}

fn preflight_geometry_contracts<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    material_count: usize,
    closure: &ResourceClosure<'_>,
) -> Result<GeometryContracts<'scope>> {
    let mesh_count = with_poll(ctx, || gltf.meshes().count())?;
    let selected_mesh_count = selected_count(ctx, closure.meshes.as_slice())?;
    enforce_usize_limit(
        ctx,
        "max_meshes",
        selected_mesh_count,
        ctx.limits().max_meshes,
    )?;
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
    let mut total_influence_cells = 0usize;
    let mut total_influence_offsets = 0usize;
    let mut needs_default_material = false;
    for (mesh_ordinal, mesh) in gltf.meshes().enumerate() {
        poll_parser_loop(ctx, mesh_ordinal)?;
        if !closure
            .meshes
            .as_slice()
            .get(mesh_ordinal)
            .copied()
            .unwrap_or(false)
        {
            meshes.try_push_reserved(None)?;
            continue;
        }
        let primitive_count = with_poll(ctx, || mesh.primitives().len())?;
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
            needs_default_material |= contract.material_index.is_none();
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
            total_influence_cells = checked_add_with_limit(
                ctx,
                "max_influence_cells",
                total_influence_cells,
                contract.influences.cell_count,
                ctx.limits().max_influence_cells,
            )?;
            authoring_u32(ctx, total_influence_cells, "gltf-total-influence-cells")?;
            total_influence_offsets = total_influence_offsets
                .checked_add(contract.influences.offset_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-total-influence-offsets"))?;
            authoring_u32(ctx, total_influence_offsets, "gltf-total-influence-offsets")?;
            primitives.try_push_reserved(contract)?;
        }
        meshes.try_push_reserved(Some(MeshContracts { primitives }))?;
    }
    Ok(GeometryContracts {
        meshes,
        needs_default_material,
    })
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
    Animation {
        animation_index: usize,
        channel_index: usize,
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
            Self::Animation {
                animation_index,
                channel_index,
            } => write!(
                formatter,
                "glTF animation {animation_index} channel {channel_index}"
            ),
        }
    }
}

fn validate_primitive_contract(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
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
    let mode = primitive.mode;
    let position_index = primitive
        .attributes
        .get(&json::mesh::Semantic::Positions)
        .ok_or_else(|| {
            ContentError::parse(
                ctx.source(),
                None,
                format_args!(
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

    for (semantic, accessor) in &primitive.attributes {
        let label = match semantic {
            json::mesh::Semantic::Positions => continue,
            json::mesh::Semantic::Normals => "NORMAL",
            json::mesh::Semantic::Tangents => "TANGENT",
            json::mesh::Semantic::TexCoords(_) => "TEXCOORD",
            json::mesh::Semantic::Colors(_) => "COLOR",
            json::mesh::Semantic::Joints(_) => "JOINTS",
            json::mesh::Semantic::Weights(_) => "WEIGHTS",
            json::mesh::Semantic::Extras(_) => {
                return Err(GltfFailure::unsupported(
                    "custom-attribute",
                    "custom glTF vertex attributes are outside the admitted profile",
                )
                .into());
            }
        };
        let contract = match semantic {
            json::mesh::Semantic::Normals => AccessorContract::vertex(
                &[json::accessor::ComponentType::F32],
                &[json::accessor::Type::Vec3],
            ),
            json::mesh::Semantic::Tangents => AccessorContract::vertex(
                &[json::accessor::ComponentType::F32],
                &[json::accessor::Type::Vec4],
            ),
            json::mesh::Semantic::TexCoords(_) => AccessorContract::vertex(
                &[
                    json::accessor::ComponentType::F32,
                    json::accessor::ComponentType::U8,
                    json::accessor::ComponentType::U16,
                ],
                &[json::accessor::Type::Vec2],
            )
            .require_normalized_integer(),
            json::mesh::Semantic::Colors(_) => AccessorContract::vertex(
                &[
                    json::accessor::ComponentType::F32,
                    json::accessor::ComponentType::U8,
                    json::accessor::ComponentType::U16,
                ],
                &[json::accessor::Type::Vec3, json::accessor::Type::Vec4],
            )
            .require_normalized_integer(),
            json::mesh::Semantic::Joints(_) => AccessorContract::vertex(
                &[
                    json::accessor::ComponentType::U8,
                    json::accessor::ComponentType::U16,
                ],
                &[json::accessor::Type::Vec4],
            ),
            json::mesh::Semantic::Weights(_) => AccessorContract::vertex(
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
                ctx.source(),
                None,
                format_args!(
                    "{owner} {label} accessor count {} does not match POSITION count {position_count}",
                    info.count
                ),
            ));
        }
    }
    let influences = influences::preflight(ctx, primitive, position_count, owner)?;

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
    let corner_count = index_count.unwrap_or(position_count);
    let face_count = primitive_face_count(ctx, owner, mode, corner_count)?;
    let index_count = index_count.unwrap_or_default();

    let material_index = primitive.material.map(|material| material.value());
    if let Some(material_index) = material_index
        && material_index >= material_count
    {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!(
                "glTF mesh {mesh_index} primitive {primitive_index} material reference is out of range"
            ),
        ));
    }

    Ok(PrimitiveContract {
        topology: map_topology(mode),
        position_count,
        index_count,
        face_count,
        attribute_stream_count,
        influences,
        material_index,
    })
}

/// glTF requires a complete nonempty primitive; core topology also admits empty authoring input.
/// Count canonical topology elements (including points/segments) before dependency I/O.
fn primitive_face_count(
    ctx: &ParseScope<'_>,
    owner: AccessorOwner,
    mode: Mode,
    corners: usize,
) -> Result<usize> {
    let faces = match mode {
        Mode::Points if corners != 0 => corners,
        Mode::Lines if corners != 0 && corners.is_multiple_of(2) => corners / 2,
        Mode::LineLoop if corners >= 2 => corners,
        Mode::LineStrip if corners >= 2 => corners - 1,
        Mode::Triangles if corners != 0 && corners.is_multiple_of(3) => corners / 3,
        Mode::TriangleStrip | Mode::TriangleFan if corners >= 3 => corners - 2,
        _ => {
            return Err(gltf_parse_error(
                ctx,
                format_args!("{owner} mode {mode:?} has an invalid draw index count {corners}"),
            ));
        }
    };
    Ok(faces)
}

fn json_primitive<'a>(
    ctx: &ParseScope<'_>,
    gltf: &'a crate::document::Document<'_>,
    mesh_index: usize,
    primitive_index: usize,
) -> Result<&'a json::mesh::Primitive<'a>> {
    gltf.tables()
        .meshes
        .get(mesh_index)
        .and_then(|mesh| mesh.primitives.get(primitive_index))
        .ok_or_else(|| {
            ContentError::parse(
                ctx.source(),
                None,
                format_args!("glTF mesh {mesh_index} primitive {primitive_index} is out of range"),
            )
        })
}

fn with_poll<T>(ctx: &ParseScope<'_>, f: impl FnOnce() -> T) -> Result<T> {
    ctx.poll()?;
    let value = f();
    ctx.poll()?;
    Ok(value)
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
                format_args!("{allocation} produced more than {expected} values"),
            ));
        }
        collected.try_push_reserved(value)?;
    }
    if collected.len() != expected {
        return Err(gltf_parse_error(
            ctx,
            format_args!(
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
    _operation: &'static str,
    mut values: impl Iterator<Item = T>,
    per_item: [u64; N],
) -> Result<MeteredVec<'scope, T>> {
    let mut collected = ctx.metered_vec(ImportStorageClass::FormatNormalization, allocation);
    collected.try_reserve(expected)?;
    loop {
        if collected.len().is_multiple_of(PARSER_POLL_INTERVAL) {
            work.poll()?;
        }
        let next = values.next();
        let Some(value) = next else {
            break;
        };
        if collected.len() == expected {
            return Err(gltf_parse_error(
                ctx,
                format_args!("{allocation} produced more than {expected} values"),
            ));
        }
        collected.try_push_reserved(value)?;
        work.complete(per_item)?;
    }
    if collected.len() != expected {
        return Err(gltf_parse_error(
            ctx,
            format_args!(
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

fn build_geometry_storage<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    contracts: &GeometryContracts<'_>,
    appearance: &AppearanceStorage<'_>,
) -> Result<GeometryStorage<'scope>> {
    let mesh_count = with_poll(ctx, || gltf.meshes().count())?;
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
    let selected_mesh_count = contracts
        .meshes
        .as_slice()
        .iter()
        .filter(|mesh| mesh.is_some())
        .count();
    logical_meshes.try_reserve(selected_mesh_count)?;
    let mut logical_mesh_ids = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        mesh_count,
        None,
        "gltf-logical-mesh-id-map",
    )?;
    ctx.charge_work(to_u64(buffers.len()))?;
    let available_provenance_count = buffers
        .iter()
        .filter(|buffer| buffer.is_some())
        .count()
        .max(1);
    let primitive_provenance_capacity = contracts
        .meshes
        .as_slice()
        .iter()
        .filter_map(Option::as_ref)
        .flat_map(|mesh| mesh.primitives.as_slice())
        .try_fold(0usize, |count, primitive| {
            let reference_count = primitive
                .attribute_stream_count
                .checked_add(1)?
                .checked_mul(AccessorSources::MAX_SOURCES)?;
            count.checked_add(reference_count.min(available_provenance_count))
        })
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-buffer-provenance-count"))?;
    let mut primitive_provenance = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        primitive_provenance_capacity,
        "gltf-primitive-buffer-provenance",
    )?;
    let mut pending_provenance = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-primitive-pending-provenance",
    );
    for (mesh_ordinal, mesh) in gltf.meshes().enumerate() {
        poll_parser_loop(ctx, mesh_ordinal)?;
        let mesh_contracts = contracts
            .meshes
            .as_slice()
            .get(mesh_ordinal)
            .ok_or_else(|| {
                gltf_parse_error(ctx, "glTF geometry contract mesh table is incomplete")
            })?;
        let Some(mesh_contracts) = mesh_contracts.as_ref() else {
            continue;
        };
        let logical_mesh_id = LogicalMeshLocalId::new(authoring_u32(
            ctx,
            logical_meshes.len(),
            "gltf-authoring-logical-mesh-id",
        )?);
        let Some(slot) = logical_mesh_ids.as_mut_slice().get_mut(mesh_ordinal) else {
            return Err(gltf_parse_error(
                ctx,
                "glTF logical mesh id map is incomplete",
            ));
        };
        *slot = Some(logical_mesh_id);
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
        let primitive_count = with_poll(ctx, || mesh.primitives().len())?;
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
                ctx,
                buffers,
                &mesh,
                primitive,
                contract,
                appearance,
                &mut primitive_provenance,
                &mut pending_provenance,
            )?)?;
        }
        logical_meshes.try_push_reserved(LogicalMeshStorage {
            name: owned_name(ctx, mesh.name(), "gltf-normalized-mesh-name")?,
            primitives,
        })?;
    }
    Ok(GeometryStorage {
        logical_meshes,
        logical_mesh_ids,
        primitive_provenance,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the existing authoring inputs borrow separate admitted provenance output and reusable scratch"
)]
fn build_primitive_authoring<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    mesh: &crate::document::Mesh<'_>,
    primitive: crate::document::Primitive<'_>,
    contract: PrimitiveContract,
    appearance: &AppearanceStorage<'_>,
    provenance: &mut MeteredVec<'scope, u32>,
    pending_provenance: &mut MeteredVec<'scope, u32>,
) -> Result<PrimitiveStorage<'scope>> {
    let attribute_cell_count = contract
        .position_count
        .checked_mul(contract.attribute_stream_count)
        .ok_or_else(|| ctx.allocation_failure("gltf-attribute-cell-counter"))?;
    let accessor_count = contract
        .attribute_stream_count
        .checked_add(1)
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-provenance-references"))?;
    let provenance_references = accessor_count
        .checked_mul(AccessorSources::MAX_SOURCES)
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-provenance-references"))?;
    let merge_work = provenance_work_bound(ctx, provenance_references)?;
    // Each accessor contributes to the primitive and, for attributes, its own retained row.
    let collection_work = to_u64(accessor_count)
        .checked_mul(2 * AccessorSources::COLLECTION_WORK_BOUND)
        .and_then(|work| work.checked_add(merge_work))
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-provenance-work"))?;
    ctx.charge_work(collection_work)?;
    pending_provenance.try_reserve(provenance_references)?;
    let (output, non_unit_rows) = ctx.with_work_group(
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
            (ImportWorkClass::InfluenceCells, to_u64(contract.influences.cell_count)),
            (ImportWorkClass::WorkUnits, contract.influences.work),
        ],
        |work| -> Result<_> {
            let provenance_offset = provenance.len();
            for (ordinal, (_, accessor)) in primitive.attributes().enumerate() {
                poll_parser_loop(ctx, ordinal)?;
                pending_provenance.try_extend_from_slice_reserved(
                    accessor_provenance(ctx, buffers, accessor)?.as_slice(),
                )?;
            }
            if let Some(accessor) = primitive.indices() {
                pending_provenance.try_extend_from_slice_reserved(
                    accessor_provenance(ctx, buffers, accessor)?.as_slice(),
                )?;
            }
            try_extend_provenance_unique(ctx, provenance, pending_provenance)?;
            let positions = accessor::optional_vectors::<3>(ctx, primitive.get(&Semantic::Positions), buffers)?.map(|positions| {
                    collect_f32_arrays(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.position_count,
                        "gltf-position-values",
                        positions,
                    )
                })
            .transpose()?
            .ok_or_else(|| {
                ContentError::parse(
                    ctx.source(),
                    None,
                    format_args!(
                        "glTF mesh {} primitive {} POSITION accessor could not be read",
                        mesh.index(),
                        primitive.index()
                    ),
                )
            })?;
            let normals = accessor::optional_vectors::<3>(ctx, primitive.get(&Semantic::Normals), buffers)?.map(|values| {
                    collect_f32_arrays(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.position_count,
                        "gltf-normal-values",
                        values,
                    )
                })
            .transpose()?
            .unwrap_or_else(|| {
                ctx.metered_vec(ImportStorageClass::SceneConstruction, "gltf-normal-values")
            });
            let tangents = accessor::optional_vectors::<4>(ctx, primitive.get(&Semantic::Tangents), buffers)?.map(|values| {
                    collect_f32_arrays(
                        ctx,
                        ImportStorageClass::SceneConstruction,
                        contract.position_count,
                        "gltf-tangent-values",
                        values,
                    )
                })
            .transpose()?
            .unwrap_or_else(|| {
                ctx.metered_vec(ImportStorageClass::SceneConstruction, "gltf-tangent-values")
            });
            let indices = with_poll(ctx, || {
                primitive.indices().map(|source| -> Result<_> {
                    let indices = accessor::bind(ctx, source, buffers)?.indices().ok_or_else(|| {
                        GltfFailure::invalid_accessor(Some(source.index()), "invalid index accessor encoding")
                    })?;
                    collect_indices(ctx, contract, indices)
                })
            })?
            .transpose()?;
            let indices = match indices {
                Some(indices) => indices,
                None if contract.index_count == 0 => {
                    ctx.metered_vec(ImportStorageClass::SceneConstruction, "gltf-index-values")
                }
                None => return Err(gltf_parse_error(ctx, "glTF indices accessor could not be read")),
            };

            let mut attributes = ctx.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-authoring-primitive-attributes",
            );
            attributes.try_push(AttributeStorage {
                semantic: AttributeSemantic::Position,
                lanes: 3,
                values: positions,
                provenance: accessor_provenance(
                    ctx,
                    buffers,
                    primitive.get(&Semantic::Positions).ok_or_else(|| {
                        gltf_parse_error(ctx, "glTF POSITION accessor disappeared after preflight")
                    })?,
                )?,
            })?;
            if !normals.is_empty() {
                attributes.try_push(AttributeStorage {
                    semantic: AttributeSemantic::Normal,
                    lanes: 3,
                    values: normals,
                    provenance: accessor_provenance(
                        ctx,
                        buffers,
                        primitive.get(&Semantic::Normals).ok_or_else(|| {
                            gltf_parse_error(ctx, "glTF NORMAL accessor disappeared after preflight")
                        })?,
                    )?,
                })?;
            }
            if !tangents.is_empty() {
                attributes.try_push(AttributeStorage {
                    semantic: AttributeSemantic::Tangent,
                    lanes: 4,
                    values: tangents,
                    provenance: accessor_provenance(
                        ctx,
                        buffers,
                        primitive.get(&Semantic::Tangents).ok_or_else(|| {
                            gltf_parse_error(ctx, "glTF TANGENT accessor disappeared after preflight")
                        })?,
                    )?,
                })?;
            }
            for set in 0..8 {
                let Some(values) = accessor::optional_vectors::<2>(ctx, primitive.get(&Semantic::TexCoords(set)), buffers)?.map(|values| {
                        collect_f32_arrays(
                            ctx,
                            ImportStorageClass::SceneConstruction,
                            contract.position_count,
                            "gltf-texcoord-values",
                            values,
                        )
                    })
                .transpose()?
                else {
                    break;
                };
                attributes.try_push(AttributeStorage {
                    semantic: AttributeSemantic::Texcoord(set),
                    lanes: 2,
                    values,
                    provenance: accessor_provenance(
                        ctx,
                        buffers,
                        primitive.get(&Semantic::TexCoords(set)).ok_or_else(|| {
                            gltf_parse_error(
                                ctx,
                                format_args!("glTF TEXCOORD_{set} accessor disappeared after preflight"),
                            )
                        })?,
                    )?,
                })?;
            }
            for set in 0..8 {
                let Some(values) = with_poll(ctx, || {
                    primitive.get(&Semantic::Colors(set)).map(|source| -> Result<_> {
                        let values = accessor::bind(ctx, source, buffers)?.rgba().ok_or_else(|| {
                            GltfFailure::invalid_accessor(Some(source.index()), "invalid color accessor shape")
                        })?;
                        collect_f32_arrays(
                            ctx,
                            ImportStorageClass::SceneConstruction,
                            contract.position_count,
                            "gltf-color-values",
                            values,
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
                    provenance: accessor_provenance(
                        ctx,
                        buffers,
                        primitive.get(&Semantic::Colors(set)).ok_or_else(|| {
                            gltf_parse_error(
                                ctx,
                                format_args!("glTF COLOR_{set} accessor disappeared after preflight"),
                            )
                        })?,
                    )?,
                })?;
            }
            let influences = with_poll(ctx, || influences::build_rows(ctx, buffers, &primitive, contract.influences),
            )??;
            work.complete_all()?;
            Ok((PrimitiveStorage {
                topology: contract.topology,
                has_indices: primitive.indices().is_some(),
                indices,
                material_assignment: match contract.material_index {
                    Some(index) => MaterialAssignment::Authored(
                        appearance.material_ids
                            .as_slice()
                            .get(index)
                            .copied()
                            .flatten()
                            .ok_or_else(|| {
                                gltf_parse_error(
                                    ctx,
                                    format_args!(
                                        "glTF primitive references material {index} outside the closure"
                                    ),
                                )
                            })?,
                    ),
                    None => MaterialAssignment::ImplicitDefault(
                        appearance.default_material.ok_or_else(|| {
                            gltf_parse_error(ctx, "glTF selected primitive has no default material")
                        })?,
                    ),
                },
                attributes,
                influence_offsets: influences.offsets,
                influences: influences.values,
                provenance_offset,
                provenance_count: provenance.len() - provenance_offset,
            }, influences.non_unit_rows))
        },
    )?;
    if non_unit_rows != 0 {
        push_warning(
            ctx,
            "gltf.influence_weight_sum",
            format_args!(
                "glTF mesh {} primitive {} has {non_unit_rows} vertices with non-unit influence weights; values are preserved",
                mesh.index(),
                primitive.index(),
            ),
        )?;
    }
    Ok(output)
}

fn collect_indices<'scope>(
    ctx: &'scope ParseScope<'_>,
    contract: PrimitiveContract,
    indices: crate::document::read::Indices<'_>,
) -> Result<MeteredVec<'scope, u32>> {
    let restart = indices.restart;
    let mut collected = ctx.metered_vec(ImportStorageClass::SceneConstruction, "gltf-index-values");
    collected.try_reserve(contract.index_count)?;
    for index in indices {
        poll_parser_loop(ctx, collected.len())?;
        if collected.len() == contract.index_count {
            return Err(gltf_parse_error(
                ctx,
                "glTF index stream exceeds its admitted count",
            ));
        }
        // Reject the source-width restart sentinel before its original component type is lost.
        if index == restart || index as usize >= contract.position_count {
            return Err(gltf_parse_error(
                ctx,
                format_args!(
                    "glTF index {} contains invalid vertex index {index}",
                    collected.len()
                ),
            ));
        }
        collected.try_push_reserved(index)?;
    }
    if collected.len() != contract.index_count {
        return Err(gltf_parse_error(
            ctx,
            "glTF index stream differs from its admitted count",
        ));
    }
    Ok(collected)
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
                format_args!("{allocation} produced more than {expected} values"),
            ));
        }
        collected.try_extend_from_slice_reserved(&value)?;
        count += 1;
    }
    if count != expected {
        return Err(gltf_parse_error(
            ctx,
            format_args!("{allocation} produced {count} values, expected {expected}"),
        ));
    }
    Ok(collected)
}

fn build_scene_nodes<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    graph: &NodeGraph<'document, '_>,
    logical_mesh_ids: &MeteredVec<'_, Option<LogicalMeshLocalId>>,
    cameras: &CameraStorage<'_>,
    lights: &lights::LightStorage<'_>,
    node_lights: &[Option<json::Index>],
) -> Result<NodeStorage<'scope>> {
    let selected_node_count = selected_count(ctx, graph.selected.as_slice())?;
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
            selected_node_count,
            "gltf-authoring-nodes",
        )?,
        names: try_vec_capacity(
            ctx,
            ImportStorageClass::FormatNormalization,
            selected_node_count,
            "gltf-normalized-node-names",
        )?,
        node_ids: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            graph.document.tables().nodes.len(),
            None,
            "gltf-node-id-map",
        )?,
        mesh_instances: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            selected_node_count,
            "gltf-authoring-mesh-instances",
        )?,
    };
    let mut pending = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        selected_node_count,
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
        let node = graph.document.node(node_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF node graph references missing node {node_index}"),
            )
        })?;
        let slot = storage
            .node_ids
            .as_mut_slice()
            .get_mut(node_index)
            .ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format_args!("glTF node index {node_index} is out of range"),
                )
            })?;
        if slot.is_some() {
            return Err(gltf_parse_error(
                ctx,
                format_args!("glTF node {node_index} was constructed more than once"),
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

        let transform = match with_poll(ctx, || node.transform())? {
            crate::document::NodeTransform::Matrix { matrix } => {
                AuthoredTransform::try_matrix(Mat4 { cols: matrix })
            }
            crate::document::NodeTransform::Decomposed {
                translation,
                rotation,
                scale,
            } => AuthoredTransform::try_trs(
                Vec3::new(translation[0], translation[1], translation[2]),
                Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
                Vec3::new(scale[0], scale[1], scale[2]),
            ),
        }
        .map_err(|_| {
            gltf_parse_error(
                ctx,
                format_args!("glTF node {node_index} transform is invalid"),
            )
        })?;
        let camera = match with_poll(ctx, || node.camera())? {
            Some(camera) => cameras
                .camera_ids
                .as_slice()
                .get(camera.index())
                .copied()
                .flatten()
                .ok_or_else(|| {
                    gltf_parse_error(
                        ctx,
                        format_args!(
                            "glTF selected node {node_index} references camera {} outside the closure",
                            camera.index()
                        ),
                    )
                })
                .map(Some)?,
            None => None,
        };
        let light = node_lights
            .get(node_index)
            .ok_or_else(|| GltfFailure::internal("glTF node light cache is incomplete"))?
            .map(|source| {
                lights
                    .light_ids
                    .as_slice()
                    .get(source.value())
                    .copied()
                    .flatten()
                    .ok_or_else(|| {
                        gltf_parse_error(ctx, "selected glTF node light is outside the closure")
                    })
            })
            .transpose()?;
        let authored = NodeAuthoringNode::new(
            parent,
            transform,
            camera.map_or(Presence::Absent, Presence::Present),
            light.map_or(Presence::Absent, Presence::Present),
        );
        let name = with_poll(ctx, || node.name())?;
        if let Some(name) = name {
            complete_canonical_string_work(ctx, name.len())?;
        }
        storage
            .names
            .try_push_reserved(owned_name(ctx, name, "gltf-normalized-node-name")?)?;
        storage.nodes.try_push_reserved(authored)?;

        let skin_index = with_poll(ctx, || node.skin())?.map(|skin| skin.index());
        if let Some(mesh) = with_poll(ctx, || node.mesh())? {
            let logical_mesh = logical_mesh_ids
                .as_slice()
                .get(mesh.index())
                .copied()
                .flatten()
                .ok_or_else(|| {
                    gltf_parse_error(
                        ctx,
                        format_args!(
                            "glTF selected node {node_index} references mesh {} outside the closure",
                            mesh.index()
                        ),
                    )
                })?;
            storage
                .mesh_instances
                .try_push_reserved(MeshInstanceStorage {
                    authored: GeometryMeshInstanceAuthoring::new(node_id, logical_mesh),
                    skin_index,
                    source_node_index: node_index,
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
    document: &'document crate::document::Document<'document>,
    children: MeteredVec<'scope, MeteredVec<'scope, usize>>,
    roots: MeteredVec<'scope, usize>,
    selected: MeteredVec<'scope, bool>,
    selection_request: SourceScopeSelectionRequest,
    selection_outcome: NodeSelectionOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeSelectionOutcome {
    DeclaredScene(usize),
    ParentlessNodeProjection,
}

impl NodeSelectionOutcome {
    const fn selected_scene_index(self) -> Option<usize> {
        match self {
            Self::DeclaredScene(scene_index) => Some(scene_index),
            Self::ParentlessNodeProjection => None,
        }
    }
}

/// Source-index membership compiled before any selected dependency is opened.
///
/// The parser deliberately keeps these maps source-indexed.  Canonical local IDs
/// are assigned later by the materializers, so omitted source rows never acquire
/// a visible local identity or consume scene-construction capacity.
struct ResourceClosure<'scope> {
    meshes: MeteredVec<'scope, bool>,
    materials: MeteredVec<'scope, bool>,
    textures: MeteredVec<'scope, bool>,
    images: MeteredVec<'scope, bool>,
    samplers: MeteredVec<'scope, bool>,
    skins: MeteredVec<'scope, bool>,
    cameras: MeteredVec<'scope, bool>,
    lights: MeteredVec<'scope, bool>,
    node_lights: MeteredVec<'scope, Option<json::Index>>,
    animation_sampler_ranges: MeteredVec<'scope, std::ops::Range<usize>>,
    animation_samplers: MeteredVec<'scope, bool>,
    buffers: MeteredVec<'scope, bool>,
    buffer_views: MeteredVec<'scope, bool>,
    accessors: MeteredVec<'scope, bool>,
}

fn build_resource_closure<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document crate::document::Document<'_>,
    graph: &NodeGraph<'document, '_>,
    animation_contracts: &animation::AnimationContracts<'_, '_>,
) -> Result<ResourceClosure<'scope>> {
    let root = gltf.tables();
    let mut animation_sampler_ranges = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        root.animations.len(),
        "gltf-closure-animation-sampler-ranges",
    )?;
    let mut animation_sampler_count = 0usize;
    for (animation_index, animation) in root.animations.iter().enumerate() {
        charge_closure_work(ctx)?;
        poll_parser_loop(ctx, animation_index)?;
        let end = animation_sampler_count
            .checked_add(animation.samplers.len())
            .ok_or_else(|| ctx.allocation_failure("gltf-closure-animation-sampler-count"))?;
        animation_sampler_ranges.try_push_reserved(animation_sampler_count..end)?;
        animation_sampler_count = end;
    }
    let mut closure = ResourceClosure {
        meshes: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.meshes.len(),
            false,
            "gltf-closure-mesh-selection",
        )?,
        materials: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.materials.len(),
            false,
            "gltf-closure-material-selection",
        )?,
        textures: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.textures.len(),
            false,
            "gltf-closure-texture-selection",
        )?,
        images: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.images.len(),
            false,
            "gltf-closure-image-selection",
        )?,
        samplers: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.samplers.len(),
            false,
            "gltf-closure-sampler-selection",
        )?,
        skins: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.skins.len(),
            false,
            "gltf-closure-skin-selection",
        )?,
        cameras: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.cameras.len(),
            false,
            "gltf-closure-camera-selection",
        )?,
        lights: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.lights.len(),
            false,
            "gltf-closure-light-selection",
        )?,
        node_lights: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.nodes.len(),
            None,
            "gltf-closure-node-lights",
        )?,
        animation_sampler_ranges,
        animation_samplers: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            animation_sampler_count,
            false,
            "gltf-closure-animation-sampler-selection",
        )?,
        buffers: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.buffers.len(),
            false,
            "gltf-closure-buffer-selection",
        )?,
        buffer_views: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.buffer_views.len(),
            false,
            "gltf-closure-buffer-view-selection",
        )?,
        accessors: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.accessors.len(),
            false,
            "gltf-closure-accessor-selection",
        )?,
    };

    for (node_index, selected) in graph.selected.as_slice().iter().copied().enumerate() {
        charge_closure_work(ctx)?;
        poll_parser_loop(ctx, node_index)?;
        if !selected {
            continue;
        }
        let node = root.nodes.get(node_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF closure references missing node {node_index}"),
            )
        })?;
        match (node.mesh, node.skin) {
            (Some(mesh), Some(skin)) => {
                mark_mesh(ctx, root, &mut closure, mesh.value())?;
                mark_skin(
                    ctx,
                    root,
                    &mut closure,
                    graph.selected.as_slice(),
                    skin.value(),
                )?;
            }
            (Some(mesh), None) => mark_mesh(ctx, root, &mut closure, mesh.value())?,
            (None, Some(_)) => {
                return Err(gltf_parse_error(
                    ctx,
                    format_args!("glTF selected node {node_index} declares a skin without a mesh"),
                ));
            }
            (None, None) => {}
        }
        if let Some(camera) = node.camera {
            mark_bool(ctx, &mut closure.cameras, camera.value(), "camera")?;
        }
        if let Some(payload) = node.punctual_light {
            let light = gltf.punctual_light_reference(ctx, payload)?;
            mark_bool(ctx, &mut closure.lights, light.value(), "light")?;
            closure.node_lights.as_mut_slice()[node_index] = Some(light);
        }
    }

    let mut source_channel_index = 0usize;
    for (animation_index, animation) in root.animations.iter().enumerate() {
        charge_closure_work(ctx)?;
        poll_parser_loop(ctx, animation_index)?;
        let animation_selected = animation_contracts
            .selected_animations
            .as_slice()
            .get(animation_index)
            .copied()
            .ok_or_else(|| {
                ContentError::from(GltfFailure::internal(
                    "animation closure selection is incomplete",
                ))
            })?;
        for (channel_index, channel) in animation.channels.iter().enumerate() {
            charge_closure_work(ctx)?;
            poll_parser_loop(ctx, channel_index)?;
            let disposition = animation_contracts
                .source_channels
                .as_slice()
                .get(source_channel_index)
                .copied()
                .ok_or_else(|| {
                    ContentError::from(GltfFailure::internal(
                        "animation closure channel selection is incomplete",
                    ))
                })?;
            source_channel_index = source_channel_index
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-closure-channel-index"))?;
            if !animation_selected
                || matches!(
                    disposition,
                    animation::SourceChannelDisposition::CoreIgnored
                )
            {
                continue;
            }
            let sampler = animation.samplers.get(channel.sampler.value()).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format_args!(
                        "glTF animation {animation_index} channel {channel_index} sampler reference is out of range"
                    ),
                )
            })?;
            let sampler_range = closure
                .animation_sampler_ranges
                .as_slice()
                .get(animation_index)
                .ok_or_else(|| {
                    ContentError::from(GltfFailure::internal(
                        "animation closure sampler selection is incomplete",
                    ))
                })?;
            let sampler_index = sampler_range
                .start
                .checked_add(channel.sampler.value())
                .ok_or_else(|| ctx.allocation_failure("gltf-closure-animation-sampler-index"))?;
            mark_bool(
                ctx,
                &mut closure.animation_samplers,
                sampler_index,
                "animation sampler",
            )?;
            mark_accessor(ctx, root, &mut closure, sampler.input.value())?;
            if matches!(disposition, animation::SourceChannelDisposition::Retained) {
                mark_accessor(ctx, root, &mut closure, sampler.output.value())?;
            }
        }
    }
    if source_channel_index != animation_contracts.source_channels.len() {
        return Err(ContentError::from(GltfFailure::internal(
            "animation closure channel selection length changed",
        )));
    }

    Ok(closure)
}

fn validate_selected_primitive_semantics(
    ctx: &ParseScope<'_>,
    mesh_index: usize,
    primitive_index: usize,
    primitive: &json::mesh::Primitive<'_>,
) -> Result<()> {
    let mut texcoord_sets = [false; 8];
    let mut color_sets = [false; 8];
    for semantic in primitive.attributes.keys() {
        charge_closure_work(ctx)?;
        match semantic {
            json::mesh::Semantic::TexCoords(set) => {
                let Some(selected) = texcoord_sets.get_mut(*set as usize) else {
                    return Err(ContentError::from(GltfFailure::unsupported(
                        "feature",
                        format_args!(
                            "mesh {mesh_index} primitive {primitive_index} TEXCOORD_{set} is not implemented yet"
                        ),
                    )));
                };
                *selected = true;
            }
            json::mesh::Semantic::Colors(set) => {
                let Some(selected) = color_sets.get_mut(*set as usize) else {
                    return Err(ContentError::from(GltfFailure::unsupported(
                        "feature",
                        format_args!(
                            "mesh {mesh_index} primitive {primitive_index} COLOR_{set} is not implemented yet"
                        ),
                    )));
                };
                *selected = true;
            }

            _ => {}
        }
    }
    ensure_contiguous_attribute_sets(ctx, mesh_index, primitive_index, "TEXCOORD", &texcoord_sets)?;
    ensure_contiguous_attribute_sets(ctx, mesh_index, primitive_index, "COLOR", &color_sets)?;
    Ok(())
}

fn ensure_contiguous_attribute_sets(
    ctx: &ParseScope<'_>,
    mesh_index: usize,
    primitive_index: usize,
    semantic: &'static str,
    sets: &[bool],
) -> Result<()> {
    let Some(last) = sets.iter().rposition(|selected| *selected) else {
        return Ok(());
    };
    if sets[..=last].iter().all(|selected| *selected) {
        return Ok(());
    }
    Err(gltf_parse_error(
        ctx,
        format_args!(
            "glTF mesh {mesh_index} primitive {primitive_index} has non-contiguous {semantic} sets"
        ),
    ))
}

fn charge_closure_work(ctx: &ParseScope<'_>) -> Result<()> {
    ctx.charge_work(1)?;
    Ok(())
}

fn mark_bool(
    ctx: &ParseScope<'_>,
    values: &mut MeteredVec<'_, bool>,
    index: usize,
    kind: &'static str,
) -> Result<()> {
    charge_closure_work(ctx)?;
    let Some(selected) = values.as_mut_slice().get_mut(index) else {
        return Err(gltf_parse_error(
            ctx,
            format_args!("glTF closure references missing {kind} {index}"),
        ));
    };
    *selected = true;
    Ok(())
}

fn mark_skin(
    ctx: &ParseScope<'_>,
    root: &json::Tables<'_>,
    closure: &mut ResourceClosure<'_>,
    selected_nodes: &[bool],
    skin_index: usize,
) -> Result<()> {
    let was_selected = closure
        .skins
        .as_slice()
        .get(skin_index)
        .copied()
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF closure references missing skin {skin_index}"),
            )
        })?;
    if was_selected {
        charge_closure_work(ctx)?;
        return Ok(());
    }
    mark_bool(ctx, &mut closure.skins, skin_index, "skin")?;
    let skin = root.skins.get(skin_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!("glTF closure references missing skin {skin_index}"),
        )
    })?;
    for (joint_ordinal, joint) in skin.joints.iter().enumerate() {
        charge_closure_work(ctx)?;
        poll_parser_loop(ctx, joint_ordinal)?;
        let joint_index = joint.value();
        if !selected_nodes.get(joint_index).copied().unwrap_or(false) {
            return Err(gltf_parse_error(
                ctx,
                format_args!(
                    "glTF skin {skin_index} references joint node {joint_index} outside the imported scene"
                ),
            ));
        }
    }
    if let Some(skeleton) = skin.skeleton {
        charge_closure_work(ctx)?;
        let skeleton_index = skeleton.value();
        if !selected_nodes.get(skeleton_index).copied().unwrap_or(false) {
            return Err(gltf_parse_error(
                ctx,
                format_args!(
                    "glTF skin {skin_index} references skeleton root node {skeleton_index} outside the imported scene"
                ),
            ));
        }
    }
    if let Some(accessor) = skin.inverse_bind_matrices {
        mark_accessor(ctx, root, closure, accessor.value())?;
    }
    Ok(())
}

fn mark_mesh(
    ctx: &ParseScope<'_>,
    root: &json::Tables<'_>,
    closure: &mut ResourceClosure<'_>,
    mesh_index: usize,
) -> Result<()> {
    let was_selected = closure
        .meshes
        .as_slice()
        .get(mesh_index)
        .copied()
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF closure references missing mesh {mesh_index}"),
            )
        })?;
    if was_selected {
        charge_closure_work(ctx)?;
        return Ok(());
    }
    mark_bool(ctx, &mut closure.meshes, mesh_index, "mesh")?;
    let mesh = root.meshes.get(mesh_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!("glTF closure references missing mesh {mesh_index}"),
        )
    })?;
    for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
        charge_closure_work(ctx)?;
        poll_parser_loop(ctx, primitive_index)?;
        validate_selected_primitive_semantics(ctx, mesh_index, primitive_index, primitive)?;
        for accessor in primitive.attributes.values() {
            mark_accessor(ctx, root, closure, accessor.value())?;
        }
        if let Some(indices) = primitive.indices {
            mark_accessor(ctx, root, closure, indices.value())?;
        }
        for target in primitive.targets.iter().flatten() {
            charge_closure_work(ctx)?;
            for accessor in [target.positions, target.normals, target.tangents]
                .into_iter()
                .flatten()
            {
                mark_accessor(ctx, root, closure, accessor.value())?;
            }
        }
        if let Some(material) = primitive.material {
            mark_material(ctx, root, closure, material.value())?;
        }
    }
    Ok(())
}

fn mark_material(
    ctx: &ParseScope<'_>,
    root: &json::Tables<'_>,
    closure: &mut ResourceClosure<'_>,
    material_index: usize,
) -> Result<()> {
    let was_selected = closure
        .materials
        .as_slice()
        .get(material_index)
        .copied()
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF closure references missing material {material_index}"),
            )
        })?;
    if was_selected {
        charge_closure_work(ctx)?;
        return Ok(());
    }
    mark_bool(ctx, &mut closure.materials, material_index, "material")?;
    let material = root.materials.get(material_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!("glTF closure references missing material {material_index}"),
        )
    })?;
    if let Some(info) = material.pbr_metallic_roughness.base_color_texture.as_ref() {
        mark_texture(ctx, root, closure, info.index.value())?;
    }
    if let Some(info) = material
        .pbr_metallic_roughness
        .metallic_roughness_texture
        .as_ref()
    {
        mark_texture(ctx, root, closure, info.index.value())?;
    }
    if let Some(info) = material.normal_texture.as_ref() {
        mark_texture(ctx, root, closure, info.index.value())?;
    }
    if let Some(info) = material.occlusion_texture.as_ref() {
        mark_texture(ctx, root, closure, info.index.value())?;
    }
    if let Some(info) = material.emissive_texture.as_ref() {
        mark_texture(ctx, root, closure, info.index.value())?;
    }
    Ok(())
}

fn mark_texture(
    ctx: &ParseScope<'_>,
    root: &json::Tables<'_>,
    closure: &mut ResourceClosure<'_>,
    texture_index: usize,
) -> Result<()> {
    let was_selected = closure
        .textures
        .as_slice()
        .get(texture_index)
        .copied()
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF closure references missing texture {texture_index}"),
            )
        })?;
    if was_selected {
        charge_closure_work(ctx)?;
        return Ok(());
    }
    mark_bool(ctx, &mut closure.textures, texture_index, "texture")?;
    let texture = root.textures.get(texture_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!("glTF closure references missing texture {texture_index}"),
        )
    })?;
    let image_index = texture.source.value();
    mark_bool(ctx, &mut closure.images, image_index, "image")?;
    let image = root.images.get(image_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!("glTF closure references missing image {image_index}"),
        )
    })?;
    if let Some(view) = image.buffer_view {
        mark_buffer_view(ctx, root, closure, view.value())?;
    }
    if let Some(sampler) = texture.sampler {
        mark_bool(ctx, &mut closure.samplers, sampler.value(), "sampler")?;
    }
    Ok(())
}

fn mark_accessor(
    ctx: &ParseScope<'_>,
    root: &json::Tables<'_>,
    closure: &mut ResourceClosure<'_>,
    accessor_index: usize,
) -> Result<()> {
    let was_selected = closure
        .accessors
        .as_slice()
        .get(accessor_index)
        .copied()
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF closure references missing accessor {accessor_index}"),
            )
        })?;
    if was_selected {
        charge_closure_work(ctx)?;
        return Ok(());
    }
    mark_bool(ctx, &mut closure.accessors, accessor_index, "accessor")?;
    let accessor = root.accessors.get(accessor_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!("glTF closure references missing accessor {accessor_index}"),
        )
    })?;
    if let Some(view) = accessor.buffer_view {
        mark_buffer_view(ctx, root, closure, view.value())?;
    }
    if let Some(sparse) = accessor.sparse.as_ref() {
        mark_buffer_view(ctx, root, closure, sparse.indices.buffer_view.value())?;
        mark_buffer_view(ctx, root, closure, sparse.values.buffer_view.value())?;
    }
    Ok(())
}

fn mark_buffer_view(
    ctx: &ParseScope<'_>,
    root: &json::Tables<'_>,
    closure: &mut ResourceClosure<'_>,
    view_index: usize,
) -> Result<()> {
    let was_selected = closure
        .buffer_views
        .as_slice()
        .get(view_index)
        .copied()
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF closure references missing bufferView {view_index}"),
            )
        })?;
    if was_selected {
        charge_closure_work(ctx)?;
        return Ok(());
    }
    mark_bool(ctx, &mut closure.buffer_views, view_index, "bufferView")?;
    let view = root.buffer_views.get(view_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!("glTF closure references missing bufferView {view_index}"),
        )
    })?;
    mark_bool(ctx, &mut closure.buffers, view.buffer.value(), "buffer")?;
    Ok(())
}

fn record_source_scope(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    graph: &NodeGraph<'_, '_>,
    animation_contracts: &animation::AnimationContracts<'_, '_>,
    closure: &ResourceClosure<'_>,
) -> Result<()> {
    let document = gltf.tables();
    let source_document = ctx
        .source()
        .try_clone()
        .map_err(|_| ctx.allocation_failure("gltf-source-scope-document"))?;
    let selection_outcome = match graph.selection_outcome {
        NodeSelectionOutcome::DeclaredScene(scene_index) => {
            SourceScopeSelectionOutcome::declared_scene(
                source_document,
                u64::try_from(scene_index).map_err(|_| {
                    gltf_parse_error(ctx, "glTF selected scene index exceeds source-scope range")
                })?,
            )
        }
        NodeSelectionOutcome::ParentlessNodeProjection => {
            SourceScopeSelectionOutcome::parentless_node_projection(source_document)
        }
    };
    let mut builder = ctx.source_scope_builder(graph.selection_request, selection_outcome)?;
    let mut identity = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "gltf-source-scope-identity",
    );
    identity.try_reserve(32)?;

    for scene_index in 0..document.scenes.len() {
        observe_source_index(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Scene,
            graph.selection_outcome.selected_scene_index() == Some(scene_index),
            's',
            scene_index,
        )?;
    }
    for (node_index, selected) in graph.selected.as_slice().iter().copied().enumerate() {
        observe_source_index(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Node,
            selected,
            'n',
            node_index,
        )?;
    }
    for (camera_index, selected) in closure.cameras.as_slice().iter().copied().enumerate() {
        observe_source_index(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Camera,
            selected,
            'c',
            camera_index,
        )?;
    }
    for (mesh_index, selected) in closure.meshes.as_slice().iter().copied().enumerate() {
        observe_source_index(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Mesh,
            selected,
            'm',
            mesh_index,
        )?;
        let mesh = document.meshes.get(mesh_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF source-scope mesh {mesh_index} is missing"),
            )
        })?;
        for (primitive_index, _primitive) in mesh.primitives.iter().enumerate() {
            observe_source_identity(
                &mut builder,
                &mut identity,
                SourceScopeEntityKind::Primitive,
                selected,
                format_args!("m{mesh_index}p{primitive_index}"),
            )?;
        }
    }
    for (material_index, selected) in closure.materials.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Material,
            selected,
            format_args!("mat{material_index}"),
        )?;
    }
    for (texture_index, selected) in closure.textures.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Texture,
            selected,
            format_args!("t{texture_index}"),
        )?;
    }
    for (image_index, selected) in closure.images.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Image,
            selected,
            format_args!("i{image_index}"),
        )?;
    }
    for (sampler_index, selected) in closure.samplers.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Sampler,
            selected,
            format_args!("ts{sampler_index}"),
        )?;
    }
    for (skin_index, selected) in closure.skins.as_slice().iter().copied().enumerate() {
        observe_source_index(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Skin,
            selected,
            'k',
            skin_index,
        )?;
    }
    for (animation_index, selected) in animation_contracts
        .selected_animations
        .as_slice()
        .iter()
        .copied()
        .enumerate()
    {
        observe_source_index(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Animation,
            selected,
            'a',
            animation_index,
        )?;
    }
    for (animation_index, sampler_range) in closure
        .animation_sampler_ranges
        .as_slice()
        .iter()
        .enumerate()
    {
        let samplers = closure
            .animation_samplers
            .as_slice()
            .get(sampler_range.clone())
            .ok_or_else(|| {
                ContentError::from(GltfFailure::internal(
                    "source-scope animation sampler range is invalid",
                ))
            })?;
        for (sampler_index, selected) in samplers.iter().copied().enumerate() {
            observe_source_identity(
                &mut builder,
                &mut identity,
                SourceScopeEntityKind::AnimationSampler,
                selected,
                format_args!("a{animation_index}s{sampler_index}"),
            )?;
        }
    }
    let mut source_channel_index = 0usize;
    for (animation_index, animation) in document.animations.iter().enumerate() {
        for channel_index in 0..animation.channels.len() {
            let selected = animation_contracts
                .source_channels
                .as_slice()
                .get(source_channel_index)
                .copied()
                .ok_or_else(|| {
                    ContentError::from(GltfFailure::internal(
                        "animation source-scope channel selection is incomplete",
                    ))
                })?;
            observe_source_channel(
                &mut builder,
                &mut identity,
                selected.is_retained(),
                animation_index,
                channel_index,
            )?;
            source_channel_index = source_channel_index
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-source-scope-channel-index"))?;
        }
    }
    if source_channel_index != animation_contracts.source_channels.len() {
        return Err(ContentError::from(GltfFailure::internal(
            "animation source-scope channel selection length changed",
        )));
    }
    for (buffer_index, selected) in closure.buffers.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Buffer,
            selected,
            format_args!("b{buffer_index}"),
        )?;
    }
    for (view_index, selected) in closure.buffer_views.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::BufferView,
            selected,
            format_args!("bv{view_index}"),
        )?;
    }
    for (accessor_index, selected) in closure.accessors.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Accessor,
            selected,
            format_args!("acc{accessor_index}"),
        )?;
    }

    for (light_index, selected) in closure.lights.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Light,
            selected,
            format_args!("l{light_index}"),
        )?;
    }
    let record = builder.finish()?;
    ctx.record_source_scope(record)?;
    Ok(())
}

fn observe_source_index(
    builder: &mut SourceScopeRecordBuilder<'_, '_>,
    identity: &mut MeteredString<'_>,
    kind: SourceScopeEntityKind,
    selected: bool,
    prefix: char,
    index: usize,
) -> Result<()> {
    identity.truncate(0);
    identity.try_write_fmt(format_args!("{prefix}{index}"))?;
    builder.observe(
        kind,
        if selected {
            SourceScopeDisposition::Selected
        } else {
            SourceScopeDisposition::Omitted
        },
        identity.as_str(),
    )?;
    Ok(())
}

fn observe_source_identity(
    builder: &mut SourceScopeRecordBuilder<'_, '_>,
    identity: &mut MeteredString<'_>,
    kind: SourceScopeEntityKind,
    selected: bool,
    value: fmt::Arguments<'_>,
) -> Result<()> {
    identity.truncate(0);
    identity.try_write_fmt(value)?;
    builder.observe(
        kind,
        if selected {
            SourceScopeDisposition::Selected
        } else {
            SourceScopeDisposition::Omitted
        },
        identity.as_str(),
    )?;
    Ok(())
}

fn observe_source_channel(
    builder: &mut SourceScopeRecordBuilder<'_, '_>,
    identity: &mut MeteredString<'_>,
    selected: bool,
    animation_index: usize,
    channel_index: usize,
) -> Result<()> {
    identity.truncate(0);
    identity.try_write_fmt(format_args!("a{animation_index}c{channel_index}"))?;
    builder.observe(
        SourceScopeEntityKind::AnimationChannel,
        if selected {
            SourceScopeDisposition::Selected
        } else {
            SourceScopeDisposition::Omitted
        },
        identity.as_str(),
    )?;
    Ok(())
}

fn preflight_node_graph<'document, 'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document crate::document::Document<'_>,
) -> Result<NodeGraph<'document, 'scope>> {
    let selection = ctx.options(&gltf_import_option_key())?.scene_selection();
    let selection_request = match selection {
        GltfSceneSelection::DocumentDefault => SourceScopeSelectionRequest::DocumentDefault,
        GltfSceneSelection::SceneIndex(scene_index) => SourceScopeSelectionRequest::ExplicitScene {
            scene_index: u64::from(scene_index),
        },
    };
    let node_count = with_poll(ctx, || gltf.nodes().len())?;
    ctx.with_work_group(
        [
            (ImportWorkClass::GraphNodes, to_u64(node_count)),
            (ImportWorkClass::GraphWork, to_u64(node_count)),
        ],
        |work| -> Result<()> {
            // Borrowing rows removes the handle table, not the observable visit prefixes.
            work.poll()?;
            for (ordinal, _) in gltf.nodes().enumerate() {
                if ordinal.is_multiple_of(PARSER_POLL_INTERVAL) {
                    work.poll()?;
                }
                work.complete([1, 1])?;
            }
            if node_count.is_multiple_of(PARSER_POLL_INTERVAL) {
                work.poll()?;
            }
            Ok(())
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
    for (parent_index, node) in gltf.nodes().enumerate() {
        poll_parser_loop(ctx, parent_index)?;
        let child_count = with_poll(ctx, || node.children().len())?;
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
                let mut node_children = node.children();
                loop {
                    if child_indices.len().is_multiple_of(PARSER_POLL_INTERVAL) {
                        work.poll()?;
                    }
                    let next = node_children.next();
                    let Some(child) = next else {
                        break;
                    };
                    if child_indices.len() == child_count {
                        return Err(gltf_parse_error(
                            ctx,
                            format_args!(
                                "gltf-node-child-list produced more than {child_count} values"
                            ),
                        ));
                    }
                    let child_index = child.index();
                    if child_index >= node_count {
                        return Err(gltf_parse_error(
                            ctx,
                            format_args!(
                                "glTF node {parent_index} references missing child {child_index}"
                            ),
                        ));
                    }
                    if child_index == parent_index {
                        return Err(gltf_parse_error(
                            ctx,
                            format_args!("glTF node graph contains a cycle at node {parent_index}"),
                        ));
                    }
                    if let Some(existing_parent) = parents.as_slice()[child_index] {
                        return Err(gltf_parse_error(
                            ctx,
                            format_args!(
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
                        format_args!(
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
    let (roots, selection_outcome) =
        select_and_validate_scene_roots(ctx, gltf, parents.as_slice(), selection)?;
    charge_canonical_graph_shape(ctx, roots.len())?;
    let selected = validate_canonical_graph_limits(ctx, children.as_slice(), roots.as_slice())?;

    Ok(NodeGraph {
        document: gltf,
        children,
        roots,
        selected,
        selection_request,
        selection_outcome,
    })
}

fn validate_canonical_graph_limits<'scope>(
    ctx: &'scope ParseScope<'_>,
    children: &[MeteredVec<'_, usize>],
    roots: &[usize],
) -> Result<MeteredVec<'scope, bool>> {
    let mut max_depth = 0usize;
    observe_graph_depth(ctx, &mut max_depth, 1)?;
    let mut selected = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        children.len(),
        false,
        "gltf-selected-node-membership",
    )?;

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
                format_args!("glTF node graph references missing node {node_index}"),
            )
        })?;
        let selected_node = selected.as_mut_slice().get_mut(node_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF selected node {node_index} is out of range"),
            )
        })?;
        *selected_node = true;
        let child_depth = depth
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-graph-depth-counter"))?;
        pending.try_reserve(node_children.len())?;
        for (child_ordinal, child) in node_children.as_slice().iter().rev().copied().enumerate() {
            poll_parser_loop(ctx, child_ordinal)?;
            pending.try_push_reserved((child, child_depth))?;
        }
    }

    Ok(selected)
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
    gltf: &crate::document::Document<'_>,
    parents: &[Option<usize>],
    selection: GltfSceneSelection,
) -> Result<(MeteredVec<'scope, usize>, NodeSelectionOutcome)> {
    let scene_count = with_poll(ctx, || gltf.scenes().len())?;
    let mut scene_roots = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-scene-root-lists",
    );

    for (scene_index, scene) in gltf.scenes().enumerate() {
        poll_parser_loop(ctx, scene_index)?;
        let roots = ctx.with_work(ImportWorkClass::GraphWork, 1, |work| -> Result<_> {
            scene_roots.try_reserve(1)?;
            work.poll()?;
            let root_count = scene.nodes().len();
            work.complete_all()?;
            work.replenish(to_u64(root_count))?;
            work.poll()?;
            let roots = scene.nodes();
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
                    format_args!(
                        "glTF scene {scene_index} references missing root node {root_index}"
                    ),
                ));
            };
            if seen_in_scene.as_slice()[root_index] == scene_index {
                return Err(gltf_parse_error(
                    ctx,
                    format_args!(
                        "glTF scene {scene_index} contains duplicate root node {root_index}"
                    ),
                ));
            }
            seen_in_scene.as_mut_slice()[root_index] = scene_index;
            if parent.is_some() {
                return Err(gltf_parse_error(
                    ctx,
                    format_args!("glTF scene {scene_index} root node {root_index} has a parent"),
                ));
            }
        }
    }

    let selection_outcome = match selection {
        GltfSceneSelection::SceneIndex(scene_index) => {
            let scene_index = usize::try_from(scene_index).map_err(|_| {
                GltfFailure::invalid_reference(
                    "scene-selection",
                    None,
                    "glTF requested scene index exceeds the platform index range",
                )
            })?;
            if scene_index >= scene_count {
                return Err(GltfFailure::invalid_reference(
                    "scene-selection",
                    Some(scene_index),
                    "glTF requested scene index is out of range",
                )
                .into());
            }
            NodeSelectionOutcome::DeclaredScene(scene_index)
        }
        GltfSceneSelection::DocumentDefault => {
            let default_scene_index =
                with_poll(ctx, || gltf.default_scene().map(|scene| scene.index()))?;
            match (default_scene_index, scene_count) {
                (Some(scene_index), _) => NodeSelectionOutcome::DeclaredScene(scene_index),
                (None, 0) => NodeSelectionOutcome::ParentlessNodeProjection,
                (None, 1) => NodeSelectionOutcome::DeclaredScene(0),
                (None, _) => return Err(GltfFailure::scene_selection_required().into()),
            }
        }
    };
    match selection_outcome {
        NodeSelectionOutcome::DeclaredScene(scene_index) => {
            let selected = scene_roots.as_slice().get(scene_index).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format_args!("glTF selected scene index {scene_index} is out of range"),
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
            Ok((roots, NodeSelectionOutcome::DeclaredScene(scene_index)))
        }
        NodeSelectionOutcome::ParentlessNodeProjection => {
            ctx.with_work(ImportWorkClass::GraphWork, 1, |work| -> Result<_> {
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
                Ok((roots, NodeSelectionOutcome::ParentlessNodeProjection))
            })
        }
    }
}

fn build_skins<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    node_ids: &[Option<NodeLocalId>],
    closure: &ResourceClosure<'_>,
) -> Result<SkinStorage<'scope>> {
    let skin_count = with_poll(ctx, || gltf.skins().len())?;
    let selected_skin_count = selected_count(ctx, closure.skins.as_slice())?;

    let mut storage = SkinStorage {
        skins: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            selected_skin_count,
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
        let Some(is_selected) = closure.skins.as_slice().get(skin_index).copied() else {
            return Err(gltf_parse_error(
                ctx,
                format_args!("glTF skin index {skin_index} is out of range"),
            ));
        };
        if !is_selected {
            continue;
        }

        let joint_count = with_poll(ctx, || skin.joints().len())?;
        let joints = with_poll(ctx, || {
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
                    ctx.source(),
                    None,
                    format_args!(
                        "glTF skin {} references joint node {} outside the imported scene",
                        skin.index(),
                        joint.index()
                    ),
                ));
            };
            mapped_joints.try_push_reserved(*node_id)?;
        }

        let skeleton_root = match with_poll(ctx, || skin.skeleton())? {
            Some(root) => match node_ids.get(root.index()).copied().flatten() {
                Some(node_id) => Some(node_id),
                None => {
                    return Err(ContentError::parse(
                        ctx.source(),
                        None,
                        format_args!(
                            "glTF skin {} references skeleton root node {} outside the imported scene",
                            skin.index(),
                            root.index()
                        ),
                    ));
                }
            },
            None => None,
        };

        ctx.charge_work(AccessorSources::COLLECTION_WORK_BOUND)?;
        let provenance = skin
            .inverse_bind_matrices()
            .map(|accessor| accessor_provenance(ctx, buffers, accessor))
            .transpose()?
            .unwrap_or(AccessorSources::ROOT);
        let inverse_bind_matrices =
            collect_inverse_bind_matrices(ctx, buffers, &skin, mapped_joints.len())?;

        let source_name = skin.name();
        complete_canonical_string_work(ctx, source_name.map_or(0, str::len))?;
        let name = owned_name(ctx, source_name, "gltf-normalized-skin-name")?;
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
            provenance,
        })?;
        let Some(slot) = storage.skin_ids.as_mut_slice().get_mut(skin_index) else {
            return Err(gltf_parse_error(
                ctx,
                format_args!("glTF skin index {skin_index} is out of range"),
            ));
        };
        *slot = Some(local_id);
    }
    Ok(storage)
}

fn collect_inverse_bind_matrices<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    skin: &crate::document::Skin<'_>,
    joint_count: usize,
) -> Result<MeteredVec<'scope, Mat4>> {
    let scalar_count = to_u64(joint_count)
        .checked_mul(16)
        .ok_or_else(|| ctx.allocation_failure("gltf-inverse-bind-work"))?;
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        scalar_count,
        |work| -> Result<_> {
            let mut collected = try_vec_capacity(
                ctx,
                ImportStorageClass::SceneConstruction,
                joint_count,
                "gltf-inverse-bind-matrices",
            )?;
            if skin.inverse_bind_matrices().is_some() {
                let source = skin
                    .inverse_bind_matrices()
                    .ok_or_else(|| GltfFailure::internal("inverse bind accessor disappeared"))?;
                let matrices = accessor::bind(ctx, source, buffers)?
                    .matrices()
                    .ok_or_else(|| {
                        GltfFailure::invalid_accessor(
                            Some(source.index()),
                            "invalid inverse bind matrix shape",
                        )
                    })?;
                with_poll(ctx, || -> Result<()> {
                    for cols in matrices {
                        if collected.len().is_multiple_of(PARSER_POLL_INTERVAL) {
                            work.poll()?;
                        }
                        if collected.len() == joint_count {
                            return Err(gltf_parse_error(
                                ctx,
                                "glTF inverse bind matrix reader exceeded the joint count",
                            ));
                        }
                        let matrix = Mat4 { cols };
                        if !matrix.is_finite() || !matrix.is_affine() {
                            return Err(gltf_parse_error(
                                ctx,
                                format_args!(
                                    "glTF skin {} inverse bind matrix {} must be finite and affine",
                                    skin.index(),
                                    collected.len(),
                                ),
                            ));
                        }
                        collected.try_push_reserved(matrix)?;
                        work.complete([16])?;
                    }
                    Ok(())
                })??;
            } else {
                for joint in 0..joint_count {
                    if joint.is_multiple_of(PARSER_POLL_INTERVAL) {
                        work.poll()?;
                    }
                    collected.try_push_reserved(Mat4::IDENTITY)?;
                    work.complete([16])?;
                }
            }
            if collected.len() != joint_count {
                return Err(gltf_parse_error(
                    ctx,
                    "glTF inverse bind matrix count does not match joint count",
                ));
            }
            Ok(collected)
        },
    )
}

fn build_mesh_instance_deformations<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    mesh_instances: &[MeshInstanceStorage],
    skin_ids: &[Option<SkinLocalId>],
) -> Result<MeteredVec<'scope, InstanceDeformationStorage<'scope>>> {
    let mut deformations = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        mesh_instances.len(),
        "gltf-normalized-instance-deformations",
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
                    gltf_parse_error(ctx, format_args!("glTF skin {index} was not imported"))
                })?,
            None => Presence::Absent,
        };
        let weights = gltf
            .tables()
            .nodes
            .get(instance.source_node_index)
            .ok_or_else(|| gltf_parse_error(ctx, "glTF morph instance node disappeared"))?
            .weights
            .as_deref();
        let weights = weights
            .map(|weights| -> Result<_> {
                ctx.with_work_group(
                    [
                        (ImportWorkClass::WorkUnits, to_u64(weights.len())),
                        (ImportWorkClass::MorphCells, to_u64(weights.len())),
                    ],
                    |work| -> Result<_> {
                        let mut owned = try_vec_capacity(
                            ctx,
                            ImportStorageClass::FormatNormalization,
                            weights.len(),
                            "gltf-normalized-instance-weights",
                        )?;
                        for chunk in weights.chunks(PARSER_POLL_INTERVAL) {
                            work.poll()?;
                            owned.try_extend_from_slice_reserved(chunk)?;
                            work.complete([to_u64(chunk.len()), to_u64(chunk.len())])?;
                        }
                        Ok(owned)
                    },
                )
            })
            .transpose()?;
        deformations.try_push_reserved(InstanceDeformationStorage { skin, weights })?;
    }
    Ok(deformations)
}

fn validate_skin_inverse_bind_accessor(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    skin_index: usize,
    joint_count: usize,
) -> Result<bool> {
    let json_skin = gltf.tables().skins.get(skin_index).ok_or_else(|| {
        ContentError::parse(
            ctx.source(),
            None,
            format_args!("glTF skin {skin_index} is out of range"),
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
            ctx.source(),
            None,
            format_args!(
                "{owner} inverse bind matrix count {} does not match joint count {joint_count}",
                info.count
            ),
        ));
    }
    Ok(true)
}

fn preflight_skin_accessors(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    closure: &ResourceClosure<'_>,
) -> Result<()> {
    for (skin_index, skin) in gltf.tables().skins.iter().enumerate() {
        ctx.charge_work(1)?;
        poll_parser_loop(ctx, skin_index)?;
        if closure.skins.as_slice()[skin_index] {
            validate_skin_inverse_bind_accessor(ctx, gltf, skin_index, skin.joints.len())?;
        }
    }
    Ok(())
}

fn build_cameras<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    closure: &ResourceClosure<'_>,
) -> Result<CameraStorage<'scope>> {
    let camera_count = with_poll(ctx, || gltf.cameras().count())?;
    let selected_camera_count = selected_count(ctx, closure.cameras.as_slice())?;
    let mut storage = CameraStorage {
        authored: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            selected_camera_count,
            "gltf-authoring-cameras",
        )?,
        names: try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            selected_camera_count,
            "gltf-authoring-camera-names",
        )?,
        camera_ids: try_vec_filled(
            ctx,
            ImportStorageClass::FormatNormalization,
            camera_count,
            None,
            "gltf-camera-id-map",
        )?,
    };
    for (ordinal, camera) in gltf.cameras().enumerate() {
        poll_parser_loop(ctx, ordinal)?;
        if !closure
            .cameras
            .as_slice()
            .get(ordinal)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let projection = with_poll(ctx, || match camera.projection() {
            json::Projection::Orthographic {
                xmag,
                ymag,
                znear,
                zfar,
            } => CameraProjection::orthographic(
                baozi_import::authoring::LocalAxes::STANDARD,
                xmag,
                ymag,
                znear,
                zfar,
            ),
            json::Projection::Perspective {
                yfov,
                aspect_ratio,
                znear,
                zfar,
            } => CameraProjection::perspective(
                baozi_import::authoring::LocalAxes::STANDARD,
                yfov,
                aspect_ratio,
                znear,
                zfar,
            ),
        })?
        .map_err(|_| {
            gltf_parse_error(
                ctx,
                format_args!("glTF camera {ordinal} projection is invalid"),
            )
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
        let local_id = CameraLocalId::new(authoring_u32(
            ctx,
            storage.authored.len(),
            "gltf-authoring-camera-id",
        )?);
        storage.authored.try_push_reserved(projection)?;
        storage
            .names
            .try_push_reserved(owned_name(ctx, name, "gltf-normalized-camera-name")?)?;
        let Some(slot) = storage.camera_ids.as_mut_slice().get_mut(ordinal) else {
            return Err(gltf_parse_error(ctx, "glTF camera id map is incomplete"));
        };
        *slot = Some(local_id);
    }
    Ok(storage)
}

fn map_topology(mode: Mode) -> GeometryTopologyAuthoring<'static> {
    match mode {
        Mode::Points => GeometryTopologyAuthoring::Points,
        Mode::Lines => GeometryTopologyAuthoring::LineList,
        Mode::LineLoop => GeometryTopologyAuthoring::LineLoop,
        Mode::LineStrip => GeometryTopologyAuthoring::LineStrip,
        Mode::Triangles => GeometryTopologyAuthoring::TriangleList,
        Mode::TriangleStrip => GeometryTopologyAuthoring::TriangleStrip,
        Mode::TriangleFan => GeometryTopologyAuthoring::TriangleFan,
    }
}

fn map_sampler(sampler: crate::document::Sampler<'_>) -> SamplerState {
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

#[cfg(test)]
mod provenance_work_tests {
    use super::*;
    use baozi_import::{
        DetectionOutcome, DetectionResult, DetectionScope, FormatDescriptor, FormatImporter,
        FormatSelection, ImportConfigOverlay, ImportDimension, ImportFailure,
        ImportOperationControl, ImportRequest, ImportRequestLayer, OptionCompileLimits, Probe,
        ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder, ResourceLimits,
        ResourceLimitsOverlay, import_asset,
    };
    use baozi_io::{AssetPath, MemoryAssetIo};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    type Inspect = fn(&ParseScope<'_>, usize, bool) -> Result<()>;

    struct InspectWork {
        inspect: Inspect,
        count: usize,
        exercise: bool,
        completed: Arc<AtomicBool>,
    }

    impl FormatImporter for InspectWork {
        fn descriptor(&self) -> FormatDescriptor {
            crate::format_descriptor()
        }

        fn detect(
            &self,
            _: &Probe<'_>,
            _: &ReadHint,
            _: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            (self.inspect)(scope, self.count, self.exercise).map_err(|error| match error {
                ContentError::Scope(failure) => failure,
                error => scope.format_failure(error.into_format_failure()),
            })?;
            self.completed.store(true, Ordering::SeqCst);
            Err(scope.format_failure(GltfFailure::internal(
                "provenance work inspection completed",
            )))
        }
    }

    fn inspect_work(
        inspect: Inspect,
        count: usize,
        exercise: bool,
        max_work: Option<u64>,
    ) -> TestResult<(ImportFailure, bool)> {
        let completed = Arc::new(AtomicBool::new(false));
        let mut registry = RegistryBuilder::new();
        registry.register(InspectWork {
            inspect,
            count,
            exercise,
            completed: Arc::clone(&completed),
        })?;
        let mut limits = ResourceLimits::default();
        if let Some(max_work) = max_work {
            limits.max_work_units = max_work;
        }
        let request = registry
            .freeze(RegistryBuildLimits::default())?
            .compile_request(
                &ImportRequest::from_layers(
                    ImportRequestLayer::default(),
                    ImportRequestLayer::new(
                        ImportConfigOverlay::default()
                            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
                            .with_format_selection(FormatSelection::Forced(
                                baozi_import::authoring::FormatId::new(crate::FORMAT_ID)?,
                            )),
                    ),
                ),
                OptionCompileLimits::default(),
            )?;
        let io = MemoryAssetIo::new();
        let _ = io.try_insert(AssetPath::new("work.gltf")?, Arc::<[u8]>::from(&b"{}"[..]))?;
        let _ = io.try_insert(
            AssetPath::new("image.png")?,
            Arc::<[u8]>::from(&[1, 2, 3][..]),
        )?;
        let result = import_asset(
            &request,
            &io,
            AssetPath::new("work.gltf")?,
            ImportOperationControl::default(),
        );
        let failure = result
            .expect_err("inspection stops before scene construction")
            .into_operation_failure()?;
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        Ok((failure, completed.load(Ordering::SeqCst)))
    }

    fn consumed_work(failure: &ImportFailure) -> u64 {
        failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .map_or(0, |entry| entry.consumed())
    }

    fn inspect_images(scope: &ParseScope<'_>, count: usize, exercise: bool) -> Result<()> {
        let mut values = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "image-lookup-fixture",
        );
        values.try_reserve(count)?;
        let origin = scope.root_origin();
        for index in 0..count {
            let outcome = scope.dependency(&origin, &external_image_role(), "image.png")?;
            let snapshot = scope.into_available_dependency(outcome)?;
            let identity = uri::profile_identity(scope, snapshot.identity().path().as_str())?;
            values.try_push_reserved(ExternalImageResource {
                image_index: 2 * index + 1,
                identity,
                snapshot,
            })?;
        }
        let images = ExternalImageResources {
            values,
            resource_offset: 17,
        };
        if exercise {
            for index in 0..count {
                let (resource, snapshot) = images
                    .get(scope, 2 * index + 1)?
                    .expect("selected source ordinal must resolve");
                assert_eq!(resource, 17 + u32::try_from(index).unwrap());
                assert_eq!(snapshot.bytes(), &[1, 2, 3]);
            }
            for missing in [0, 2, 2 * count + 1] {
                assert!(images.get(scope, missing)?.is_none());
            }
        }
        Ok(())
    }

    fn inspect_unique_sources(scope: &ParseScope<'_>, count: usize, exercise: bool) -> Result<()> {
        let mut pending = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "pending-provenance-fixture",
        );
        let mut values = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "retained-provenance-fixture",
        );
        pending.try_reserve(2 * count)?;
        values.try_reserve(count + 2)?;
        values.try_push_reserved(91)?;
        values.try_push_reserved(72)?;
        for source in (0..count).rev() {
            pending.try_push_reserved(u32::try_from(source).unwrap())?;
            pending.try_push_reserved(u32::try_from(source).unwrap())?;
        }
        let capacity = pending.capacity();
        if exercise {
            // Match production's prepaid work and non-nestable sample decode boundary.
            let charged = scope.charge_work(provenance_work_bound(scope, pending.len())?);
            if let Err(failure) = charged {
                assert_eq!(values.as_slice(), &[91, 72]);
                return Err(failure.into());
            }
            scope.with_work(ImportWorkClass::AnimationCells, 1, |work| -> Result<()> {
                try_extend_provenance_unique(scope, &mut values, &mut pending)?;
                work.complete_all()?;
                Ok(())
            })?;
            assert_eq!(&values.as_slice()[..2], &[91, 72]);
            assert_eq!(values.len(), count + 2);
            for (index, &source) in values.as_slice()[2..].iter().enumerate() {
                assert_eq!(source, u32::try_from(index).unwrap());
            }
            assert!(pending.is_empty());
            assert_eq!(pending.capacity(), capacity);
            // Reusing the scratch must append another owner without merging with the prefix.
            values.try_reserve(2)?;
            pending.try_extend_from_slice(&[7, 2, 7])?;
            scope.charge_work(provenance_work_bound(scope, pending.len())?)?;
            try_extend_provenance_unique(scope, &mut values, &mut pending)?;
            assert_eq!(&values.as_slice()[count + 2..], &[2, 7]);
        }
        Ok(())
    }

    #[test]
    fn external_image_lookup_meters_sparse_hits_and_misses() -> TestResult<()> {
        let mut work = Vec::new();
        for count in [0usize, 32, 64] {
            let (baseline, completed) = inspect_work(inspect_images, count, false, None)?;
            assert!(completed);
            let (queried, completed) = inspect_work(inspect_images, count, true, None)?;
            assert!(completed);
            let delta = consumed_work(&queried) - consumed_work(&baseline);
            let comparisons = u64::from(usize::BITS - count.leading_zeros()) + 1;
            assert_eq!(delta, to_u64(count + 3) * comparisons);
            work.push(delta);
        }
        assert!(work[2] > work[1]);
        assert!(work[2] < 3 * work[1]);
        Ok(())
    }

    #[test]
    fn provenance_ranges_deduplicate_with_bounded_work() -> TestResult<()> {
        let (_, completed) = inspect_work(inspect_unique_sources, 0, true, None)?;
        assert!(completed);
        let mut work = Vec::new();
        for count in [64, 128] {
            let (failure, completed) = inspect_work(inspect_unique_sources, count, true, None)?;
            assert!(completed);
            let consumed = consumed_work(&failure);
            assert!(consumed >= to_u64(2 * count));
            assert!(consumed < to_u64(2 * count) * 32);
            work.push(consumed);
        }
        assert!(work[1] > work[0]);
        assert!(work[1] < 3 * work[0]);
        Ok(())
    }

    #[test]
    fn provenance_work_admission_aborts_before_appending_an_owner() -> TestResult<()> {
        let (baseline, completed) = inspect_work(inspect_unique_sources, 64, false, None)?;
        assert!(completed);
        let baseline_work = consumed_work(&baseline);
        let (failure, completed) =
            inspect_work(inspect_unique_sources, 64, true, Some(baseline_work + 1))?;
        assert!(!completed);
        assert!(matches!(
            failure.cause(),
            baozi_import::ImportCause::Scope(abort)
                if matches!(abort.cause(), baozi_import::ScopeAbortCause::Ledger(
                    baozi_import::LedgerCause::LimitExceeded {
                        dimension: ImportDimension::WorkUnits, ..
                    }
                ))
        ));
        assert!(consumed_work(&failure) <= baseline_work);
        Ok(())
    }
}
