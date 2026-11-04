use crate::document::BufferSource;
use crate::document::ImageSource;
use crate::document::mesh_encoding::MeshAttributeUse;
use crate::document::schema as json;
use crate::document::{MagFilter, MinFilter, WrappingMode};
use crate::document::{Mode, Semantic};
use crate::{
    GltfFailure, GltfImportOptions, GltfSceneSelection, admission, external_buffer_role,
    external_image_role,
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
    DiagnosticCode, DiagnosticSeverity, ImportStorageClass, ImportWorkClass, MeteredOrderedMap,
    MeteredString, MeteredVec, ParseChildren, ParseContext, ParseFailure, ParseParser, ParseResult,
    ParseRoot, ParseServices, ParseWorkLayout, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeRecordBuilder, SourceScopeSelectionOutcome, SourceScopeSelectionRequest,
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

pub(crate) const BYTE_POLL_INTERVAL: usize = 4_096;
const GEOMETRY_POLL_INTERVAL: usize = baozi_import::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS;

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

pub(crate) fn read_gltf(
    options: &GltfImportOptions,
    context: &mut ParseContext<'_, '_>,
) -> ParseResult<()> {
    let result = (|| {
        let normalized = with_content(context, |exec| read_normalized_gltf(exec, options))?;
        normalized::submit(context, &normalized)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(ContentError::Scope(error)) => Err(error),
        Err(error) => {
            context.with_execution(|exec| Err(exec.format_failure(error.into_format_failure())))
        }
    }
}

/// Materializes format evidence before returning an error through the actual numeric root.
fn with_content<'session, R>(
    context: &mut ParseContext<'session, '_>,
    action: impl for<'op> FnOnce(&mut ParseRoot<'_, 'op, 'session>) -> Result<R>,
) -> Result<R> {
    context
        .with_execution(|root| {
            action(root).map_err(|cause| match cause {
                ContentError::Scope(cause) => cause,
                ContentError::Format(cause) => root.format_failure(cause),
            })
        })
        .map_err(ContentError::Scope)
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn authoring_u32(
    ctx: &mut ParseServices<'_, '_, '_>,
    value: usize,
    resource: &'static str,
) -> Result<u32> {
    u32::try_from(value).map_err(|_| ctx.allocation_failure(resource).into())
}

fn enforce_usize_limit(
    ctx: &mut ParseServices<'_, '_, '_>,
    resource: &'static str,
    attempted: usize,
    limit: usize,
) -> Result<()> {
    ctx.enforce_resource_limit(resource, to_u64(attempted), to_u64(limit))?;
    Ok(())
}

fn selected_count(ctx: &mut ParseRoot<'_, '_, '_>, values: &[bool]) -> Result<usize> {
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    ctx.child(&work_group, [to_u64(values.len())], |child| -> Result<_> {
        let mut count = 0usize;
        for (index, selected) in values.iter().enumerate() {
            poll_parser_loop(&mut child.services()?, index)?;
            child.run([1], || -> Result<()> {
                count = count.checked_add(usize::from(*selected)).ok_or_else(|| {
                    ContentError::from(GltfFailure::internal("selection count overflow"))
                })?;
                Ok(())
            })?;
        }
        Ok(count)
    })
}

fn checked_add_with_limit(
    ctx: &mut ParseServices<'_, '_, '_>,
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

/// Completes decoding before canonical construction can enter its own call chain.
/// Prepared contracts, typed rows, and structural storage are released before submission.
fn read_normalized_gltf<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    options: &GltfImportOptions,
) -> Result<NormalizedGltf<'scope>> {
    let surfaces = admission::admit(ctx)?;
    let gltf = crate::document::Document::decode(ctx, surfaces)?;
    let node_graph = preflight_node_graph(ctx, &gltf, options)?;
    let animation_contracts = animation::preflight(ctx, &gltf, node_graph.selected.as_slice())?;
    let mut resource_closure =
        build_resource_closure(ctx, &gltf, &node_graph, &animation_contracts)?;
    let material_contracts = materials::preflight(ctx, &gltf, &resource_closure)?;
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for binding in material_contracts.all_bindings() {
        let source_texture = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, ParseFailure>(binding.source_texture))
        })?;
        mark_texture(ctx, gltf.tables(), &mut resource_closure, source_texture)?;
    }
    source::preflight(
        ctx,
        gltf.source_map(),
        &gltf,
        &node_graph,
        &resource_closure,
        &animation_contracts,
    )?;
    report_uninterpreted_extension_declarations(ctx, gltf.tables())?;
    let lights = lights::prepare(ctx, &gltf, resource_closure.lights.as_slice())?;
    let repeated_mesh_string_bytes =
        preflight_canonical_base_strings(ctx, &gltf, &node_graph, &resource_closure, &lights)?;
    let resolved_texture_uris = preflight_texture_strings(
        ctx,
        &gltf,
        repeated_mesh_string_bytes,
        &resource_closure,
        &material_contracts,
    )?;
    accessor::validate_vertex_views(ctx, &gltf, &resource_closure)?;
    let material_count = {
        ctx.poll()?;
        let value = gltf.materials().count();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    let geometry_contracts = preflight_geometry_contracts(
        ctx,
        &gltf,
        material_count,
        &resource_closure,
        &material_contracts,
    )?;
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
        &mut ctx.services()?,
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    value: Option<&str>,
    resource: &'static str,
) -> Result<Option<MeteredString<'scope>>> {
    value
        .map(|value| {
            let mut output = ctx
                .services()?
                .metered_string(ImportStorageClass::FormatNormalization, resource);
            output.try_reserve(&mut ctx.services()?, value.len())?;
            let mut remaining = value;
            let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            while !remaining.is_empty() {
                let mut end = remaining.len().min(BYTE_POLL_INTERVAL);
                while !remaining.is_char_boundary(end) {
                    end -= 1;
                }
                let (chunk, rest) = remaining.split_at(end);
                ctx.child(&work_group, [to_u64(chunk.len())], |child| {
                    child.append_string([to_u64(chunk.len())], &mut output, chunk)
                })?;
                remaining = rest;
            }
            Ok(output)
        })
        .transpose()
}

/// Called only after retained payloads have passed profile admission. Unknown selected
/// behavior is a hard error, so these diagnostics never claim to omit a retained payload.
fn report_uninterpreted_extension_declarations(
    ctx: &mut ParseRoot<'_, '_, '_>,
    root: &json::Tables<'_>,
) -> Result<()> {
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for name in &root.extensions_used {
        let supported = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, ParseFailure>(is_supported_extension(name)))
        })?;
        if !supported {
            push_warning(
                &mut ctx.services()?,
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
    ctx: &mut ParseRoot<'_, '_, '_>,
    gltf: &crate::document::Document<'_>,
    graph: &NodeGraph<'_, '_>,
    closure: &ResourceClosure<'_>,
    lights: &lights::LightStorage<'_>,
) -> Result<usize> {
    {
        ctx.poll()?;
        let value = (|| {
            let mut bytes = 0usize;
            let limit = ctx.services()?.limits().max_canonical_string_bytes;

            for (mesh_index, mesh) in gltf.meshes().enumerate() {
                poll_parser_loop(&mut ctx.services()?, mesh_index)?;
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
                    {
                        let argument_0 = ctx.services()?.limits().max_string_bytes;
                        enforce_usize_limit(
                            &mut ctx.services()?,
                            "max_string_bytes",
                            name.len(),
                            argument_0,
                        )
                    }?;
                    add_repeated_string_bytes(&mut ctx.services()?, &mut bytes, name.len(), limit)?;
                    let primitive_count = {
                        ctx.poll()?;
                        let value = mesh.primitives().len();
                        ctx.poll()?;
                        Ok::<_, ContentError>(value)
                    }?;
                    for primitive_index in 0..primitive_count {
                        let generated_len =
                            primitive_name_len(&mut ctx.services()?, name, primitive_index)?;
                        {
                            let argument_0 = ctx.services()?.limits().max_string_bytes;
                            enforce_usize_limit(
                                &mut ctx.services()?,
                                "max_string_bytes",
                                generated_len,
                                argument_0,
                            )
                        }?;
                    }
                }
            }

            for (node_index, selected) in graph.selected.as_slice().iter().copied().enumerate() {
                poll_parser_loop(&mut ctx.services()?, node_index)?;
                if !selected {
                    continue;
                }
                let Some(node) = graph.document.node(node_index) else {
                    return Err(gltf_parse_error(format_args!(
                        "glTF node graph references missing node {node_index}"
                    )));
                };
                if let Some(name) = node.name() {
                    {
                        let argument_0 = ctx.services()?.limits().max_string_bytes;
                        enforce_usize_limit(
                            &mut ctx.services()?,
                            "max_string_bytes",
                            name.len(),
                            argument_0,
                        )
                    }?;
                    add_repeated_string_bytes(&mut ctx.services()?, &mut bytes, name.len(), limit)?;
                }
            }

            for (skin_index, skin) in gltf.skins().enumerate() {
                poll_parser_loop(&mut ctx.services()?, skin_index)?;
                if closure.skins.as_slice().get(skin_index).copied() != Some(true) {
                    continue;
                }
                if let Some(name) = skin.name() {
                    {
                        let argument_0 = ctx.services()?.limits().max_string_bytes;
                        enforce_usize_limit(
                            &mut ctx.services()?,
                            "max_string_bytes",
                            name.len(),
                            argument_0,
                        )
                    }?;
                    add_repeated_string_bytes(&mut ctx.services()?, &mut bytes, name.len(), limit)?;
                }
            }

            for (camera_index, camera) in gltf.cameras().enumerate() {
                poll_parser_loop(&mut ctx.services()?, camera_index)?;
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
                    && name.len() <= ctx.services()?.limits().max_string_bytes
                {
                    add_repeated_string_bytes(&mut ctx.services()?, &mut bytes, name.len(), limit)?;
                }
            }

            let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            for name in lights.names.as_slice() {
                let name = ctx.child(&work_group, [1], |child| {
                    child.run([1], || Ok::<_, ParseFailure>(name.as_ref()))
                })?;
                if let Some(name) = name {
                    add_repeated_string_bytes(&mut ctx.services()?, &mut bytes, name.len(), limit)?;
                }
            }
            Ok(bytes)
        })();
        ctx.poll_result(value)
    }
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    repeated_mesh_string_bytes: usize,
    closure: &ResourceClosure<'_>,
    materials: &MaterialContracts<'_>,
) -> Result<ResolvedTextureUris<'scope>> {
    {
        ctx.poll()?;
        let value = (|| {
            let mut bytes = repeated_mesh_string_bytes;
            let limit = ctx.services()?.limits().max_canonical_string_bytes;
            let image_count = {
                ctx.poll()?;
                let value = gltf.images().len();
                ctx.poll()?;
                Ok::<_, ContentError>(value)
            }?;
            let mut resolved_uris = ctx.services()?.metered_ordered_map(
                ImportStorageClass::FormatNormalization,
                "gltf-resolved-texture-uris",
            );

            let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            for (index, image) in gltf.images().enumerate() {
                let selected = ctx.child(&work_group, [1], |child| {
                    child.run([1], || {
                        Ok::<_, ParseFailure>(closure.images.as_slice()[index])
                    })
                })?;
                if selected && let Some(name) = image.name() {
                    {
                        let argument_0 = ctx.services()?.limits().max_string_bytes;
                        enforce_usize_limit(
                            &mut ctx.services()?,
                            "max_string_bytes",
                            name.len(),
                            argument_0,
                        )
                    }?;
                    add_repeated_string_bytes(&mut ctx.services()?, &mut bytes, name.len(), limit)?;
                }
            }
            for (index, sampler) in gltf.samplers().enumerate() {
                let selected = ctx.child(&work_group, [1], |child| {
                    child.run([1], || {
                        Ok::<_, ParseFailure>(closure.samplers.as_slice()[index])
                    })
                })?;
                if selected && let Some(name) = sampler.name() {
                    {
                        let argument_0 = ctx.services()?.limits().max_string_bytes;
                        enforce_usize_limit(
                            &mut ctx.services()?,
                            "max_string_bytes",
                            name.len(),
                            argument_0,
                        )
                    }?;
                    add_repeated_string_bytes(&mut ctx.services()?, &mut bytes, name.len(), limit)?;
                }
            }

            for (material_index, material) in gltf.materials().enumerate() {
                poll_parser_loop(&mut ctx.services()?, material_index)?;
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
                    {
                        let argument_0 = ctx.services()?.limits().max_string_bytes;
                        enforce_usize_limit(
                            &mut ctx.services()?,
                            "max_string_bytes",
                            name.len(),
                            argument_0,
                        )
                    }?;
                    add_repeated_string_bytes(&mut ctx.services()?, &mut bytes, name.len(), limit)?;
                }
                for binding in materials.bindings(&mut ctx.services()?, material_index)? {
                    let texture_index = ctx.child(&work_group, [1], |child| {
                        child.run([1], || Ok::<_, ParseFailure>(binding.source_texture))
                    })?;
                    let texture = gltf.textures().nth(texture_index).ok_or_else(|| {
                        gltf_parse_error("compiled material texture reference is missing")
                    })?;
                    preflight_texture_reference(
                        ctx,
                        &mut bytes,
                        limit,
                        texture,
                        binding.source_key,
                        &mut resolved_uris,
                        image_count,
                    )?;
                }
            }

            Ok(ResolvedTextureUris {
                values: resolved_uris,
            })
        })();
        ctx.poll_result(value)
    }
}

fn preflight_texture_reference<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
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
        {
            let argument_0 = ctx.services()?.limits().max_string_bytes;
            enforce_usize_limit(
                &mut ctx.services()?,
                "max_string_bytes",
                name.len(),
                argument_0,
            )
        }?;
    }
    if let ImageSource::Uri { uri, .. } = image.source() {
        {
            let argument_0 = ctx.services()?.limits().max_string_bytes;
            enforce_usize_limit(
                &mut ctx.services()?,
                "max_string_bytes",
                uri.len(),
                argument_0,
            )
        }?;
    }

    let image_index = image.index();
    if image_index >= image_count {
        return Err(gltf_parse_error(format_args!(
            "glTF texture references missing image {image_index}"
        )));
    }
    let resolved_uri_len = match image.source() {
        ImageSource::Uri { uri, .. } if !is_data_uri(uri) => {
            if !resolved_uris.contains_key(&image_index) {
                let reference = uri::relative_path(ctx, uri, uri::Resource::Image(image_index))?;
                let reference = reference.as_str();
                let resolution_bytes =
                    texture_path_resolution_allowance(&mut ctx.services()?, reference)?;
                let _resolution_allowance = ctx.services()?.metered_opaque_allowance(
                    ImportStorageClass::FormatNormalization,
                    resolution_bytes,
                )?;
                let source = ctx.source();
                let amount = to_u64(source.as_str().len().saturating_add(reference.len()));
                let resolved = {
                    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
                    ctx.child(&layout, [amount], |child| -> Result<_> {
                        let resolved = child
                            .run([amount], || {
                                Ok::<_, ParseFailure>(source.resolve_from_file(reference))
                            })?
                            .map_err(|error| {
                                GltfFailure::invalid_reference(
                                    "texture-uri",
                                    Some(image_index),
                                    error,
                                )
                            })?;
                        Ok(resolved)
                    })
                }?;
                let mut resolved_uri = ctx.services()?.metered_string(
                    ImportStorageClass::FormatNormalization,
                    "gltf-resolved-texture-uri",
                );
                resolved_uri.try_push_str(&mut ctx.services()?, resolved.as_str())?;
                resolved_uris.try_insert(&mut ctx.services()?, image_index, resolved_uri)?;
            }
            Some(
                resolved_uris
                    .get(&image_index)
                    .ok_or_else(|| {
                        gltf_parse_error(format_args!(
                            "glTF texture resolution plan is missing image {image_index}"
                        ))
                    })?
                    .len(),
            )
        }
        ImageSource::Uri { .. } | ImageSource::View { .. } => None,
    };

    let texture_name = texture.name();
    if let Some(name) = texture_name {
        {
            let argument_0 = ctx.services()?.limits().max_string_bytes;
            enforce_usize_limit(
                &mut ctx.services()?,
                "max_string_bytes",
                name.len(),
                argument_0,
            )
        }?;
    }
    if let Some(name) = texture_name {
        add_repeated_string_bytes(&mut ctx.services()?, bytes, name.len(), limit)?;
    }
    if let Some(resolved_uri_len) = resolved_uri_len {
        add_repeated_string_bytes(&mut ctx.services()?, bytes, resolved_uri_len, limit)?;
    }
    add_repeated_string_bytes(&mut ctx.services()?, bytes, source_key.len(), limit)?;
    Ok(())
}

fn texture_path_resolution_allowance(
    ctx: &mut ParseServices<'_, '_, '_>,
    reference: &str,
) -> Result<u64> {
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
    ctx: &mut ParseServices<'_, '_, '_>,
    bytes: &mut usize,
    amount: usize,
    limit: usize,
) -> Result<()> {
    *bytes = checked_add_with_limit(ctx, "max_canonical_string_bytes", *bytes, amount, limit)?;
    Ok(())
}

fn primitive_name_len(
    ctx: &mut ParseServices<'_, '_, '_>,
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

fn accessor_provenance(
    _ctx: &mut ParseServices<'_, '_, '_>,
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
                    gltf_parse_error(format_args!(
                        "glTF accessor {} references unavailable buffer {buffer_index}",
                        accessor.index()
                    ))
                })?,
        );
    }
    Ok(AccessorSources::new(sources))
}

/// Appends one owner's sorted unique sources, preserving every earlier owner's range.
/// Pending storage is reused; every compare/copy records only its completed leaf.
fn try_extend_provenance_unique<'session>(
    ctx: &mut ParseChildren<'_, '_, 'session>,
    values: &mut MeteredVec<'session, u32>,
    pending: &mut MeteredVec<'session, u32>,
) -> Result<()> {
    {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        ctx.child(&layout, [0], |child| -> Result<()> {
            let mut actions = 0usize;
            pending.try_sort_unstable_by(|left, right| -> Result<_> {
                poll_parser_loop(&mut child.services()?, actions)?;
                child.replenish(1)?;
                let ordering = child.run([1], || Ok::<_, ContentError>(left.cmp(right)))?;
                actions += 1;
                Ok(ordering)
            })?;
            let retained = pending.try_partition_dedup_by(|left, right| -> Result<_> {
                poll_parser_loop(&mut child.services()?, actions)?;
                child.replenish(1)?;
                let equal = child.run([1], || Ok::<_, ContentError>(left == right))?;
                actions += 1;
                Ok(equal)
            })?;
            pending.truncate(retained);
            for &source in pending.as_slice() {
                poll_parser_loop(&mut child.services()?, actions)?;
                child.replenish(1)?;
                child.run([1], || values.try_push_reserved(source))?;
                actions += 1;
            }
            pending.truncate(0);
            Ok(())
        })
    }
}

fn load_buffers<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    glb_bin_bytes: Option<&[u8]>,
    closure: &ResourceClosure<'_>,
) -> Result<BufferSet<'scope>> {
    let buffer_count = {
        ctx.poll()?;
        let value = gltf.buffers().count();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    let mut buffers = ctx
        .services()?
        .metered_vec(ImportStorageClass::BackendDecode, "gltf-buffer-list");
    buffers.try_reserve(&mut ctx.services()?, buffer_count)?;
    let mut external_resource_count = 0usize;
    for (ordinal, buffer) in gltf.buffers().enumerate() {
        poll_parser_loop(&mut ctx.services()?, ordinal)?;
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
                let provenance = authoring_u32(
                    &mut ctx.services()?,
                    resource_index,
                    "gltf-external-buffer-provenance",
                )?;
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    uri: &'uri str,
    resource: uri::Resource,
    reference: &'static str,
    allocation: &'static str,
    retained_length: Option<usize>,
) -> Result<DecodedDataUri<'scope, 'uri>> {
    let subject = resource.name();
    let subject_index = resource.index();
    {
        let argument_0 = ctx.services()?.limits().max_string_bytes;
        enforce_usize_limit(
            &mut ctx.services()?,
            "max_string_bytes",
            uri.len(),
            argument_0,
        )
    }?;
    let Some(payload) = uri.get(5..).filter(|_| is_data_uri(uri)) else {
        return Err(gltf_parse_error(format_args!(
            "glTF {subject} {subject_index} has an invalid data URI"
        )));
    };
    let Some((metadata, encoded)) = payload.split_once(',') else {
        return Err(gltf_parse_error(format_args!(
            "glTF {subject} {subject_index} data URI has no comma separator"
        )));
    };
    if metadata.contains('%') {
        return Err(resource.unsupported(
            &mut ctx.services()?,
            "percent-encoded data URLs are unsupported",
        ));
    }
    if !metadata
        .rsplit_once(';')
        .is_some_and(|(_, encoding)| encoding.eq_ignore_ascii_case("base64"))
    {
        return Err(
            resource.unsupported(&mut ctx.services()?, "non-base64 data URI is unsupported")
        );
    }

    let decoded_len = base64_decoded_len(&mut ctx.services()?, encoded, resource)?;
    let decoded_len = usize::try_from(decoded_len)
        .map_err(|_| ctx.allocation_failure("gltf-data-uri-decoded-length"))?;
    let retained_length = retained_length.unwrap_or(decoded_len);
    let stored_length = retained_length.min(decoded_len);
    let decoded = {
        let layout =
            ctx.bind_group([ImportWorkClass::DataUriBytes, ImportWorkClass::DecodeWork])?;
        ctx.child(&layout, [to_u64(stored_length), u64::try_from(encoded.len()).unwrap_or(u64::MAX)], |child| -> Result<_> {
            let mut decoded = child.services()?.metered_vec(ImportStorageClass::BackendDecode, allocation);
            decoded.try_reserve(&mut child.services()?, stored_length)?;
            let mut decoded_length = 0usize;
            let errors = child.error_view();
            let mut chunk_output = [0_u8; BYTE_POLL_INTERVAL];
            for (chunk_index, chunk) in encoded.as_bytes().chunks(BYTE_POLL_INTERVAL).enumerate()
            {
                child.poll()?;
                child.run_returned(|| -> Result<_> {
                let chunk_start = chunk_index * BYTE_POLL_INTERVAL;
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
                decoded.try_extend_from_slice_reserved(&output[..retained_chunk])?;
                decoded_length = decoded_length
                    .checked_add(chunk_written)
                    .ok_or_else(|| errors.allocation_failure("gltf-data-uri-buffer"))?;
                Ok(((), [to_u64(retained_chunk), to_u64(chunk.len())]))
                })?;
            }
            if decoded_length != decoded_len || decoded.len() != stored_length {
                return Err(ContentError::from(GltfFailure::internal(
                    "base64 decode length changed after preflight",
                )));
            }
            if decoded_len < retained_length {
                return Err(gltf_parse_error(
                    format_args!(
                        "glTF {subject} {subject_index} declares {retained_length} bytes but only {decoded_len} bytes are available"
                    ),
                ));
            }
            Ok(decoded)
        })
    }?;
    let media_type = metadata
        .split(';')
        .next()
        .filter(|media_type| !media_type.is_empty());
    Ok(DecodedDataUri {
        bytes: decoded,
        media_type,
    })
}

fn base64_decoded_len(
    ctx: &mut ParseServices<'_, '_, '_>,
    encoded: &str,
    resource: uri::Resource,
) -> Result<u64> {
    let subject = resource.name();
    let subject_index = resource.index();
    if encoded.is_empty() {
        return Ok(0);
    }

    let mut padding = 0usize;
    let mut padding_started = false;
    let mut invalid_padding = false;
    for chunk in encoded.as_bytes().chunks(BYTE_POLL_INTERVAL) {
        ctx.poll()?;
        for &byte in chunk {
            if byte == b'%' {
                return Err(resource.unsupported(ctx, "percent-encoded data URLs are unsupported"));
            }
            if byte.is_ascii_whitespace() {
                return Err(gltf_parse_error(format_args!(
                    "glTF {subject} {subject_index} data URI base64 contains whitespace"
                )));
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
        return Err(gltf_parse_error(format_args!(
            "glTF {subject} {subject_index} data URI base64 length is not padded"
        )));
    }
    if invalid_padding {
        return Err(gltf_parse_error(format_args!(
            "glTF {subject} {subject_index} data URI base64 padding is invalid"
        )));
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    uri: &str,
    buffer_index: usize,
    declared_length: usize,
    provenance: u32,
) -> Result<BufferData<'scope>> {
    {
        let argument_0 = ctx.services()?.limits().max_string_bytes;
        enforce_usize_limit(
            &mut ctx.services()?,
            "max_string_bytes",
            uri.len(),
            argument_0,
        )
    }?;
    let reference = uri::relative_path(ctx, uri, uri::Resource::Buffer(buffer_index))?;
    let origin = ctx.services()?.root_origin();
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    source: &[u8],
    allocation: &'static str,
) -> Result<BufferData<'scope>> {
    Ok(BufferData::Owned(copy_buffer_bytes(
        ctx, source, allocation,
    )?))
}

fn copy_buffer_prefix<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    source: &[u8],
    allocation: &'static str,
) -> Result<MeteredVec<'scope, u8>> {
    let copied = {
        let layout = ctx.bind_group([ImportWorkClass::DecodeWork])?;
        ctx.child(&layout, [to_u64(source.len())], |child| -> Result<_> {
            let mut copied = child
                .services()?
                .metered_vec(ImportStorageClass::BackendDecode, allocation);
            copied.try_reserve(&mut child.services()?, source.len())?;
            for chunk in source.chunks(BYTE_POLL_INTERVAL) {
                child.poll()?;
                child.run([to_u64(chunk.len())], || {
                    copied.try_extend_from_slice_reserved(chunk)
                })?;
            }
            Ok(copied)
        })
    }?;
    Ok(copied)
}

fn gltf_parse_error(message: impl fmt::Display) -> ContentError {
    GltfFailure::invalid_document(message).into()
}

fn normalize_document<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    buffers: BufferSet<'scope>,
    prepared: &PreparedDocument<'_, '_>,
    source_map: &crate::source_map::GltfSourceMap<'_>,
    lights: lights::LightStorage<'scope>,
) -> Result<NormalizedGltf<'scope>> {
    let external_image_resource_offset = {
        let argument_0 = buffers
            .external_resource_count()
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-external-image-resource-offset"))?;
        authoring_u32(
            &mut ctx.services()?,
            argument_0,
            "gltf-external-image-resource-offset",
        )
    }?;
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
    let mut resources = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-normalized-buffer-resources",
    );
    resources.try_reserve(&mut ctx.services()?, buffers.external_resource_count())?;
    let mut buffers = buffers;
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    while let Some(buffer) = buffers.values.pop() {
        let source_index = buffers.values.len();
        let external = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, ParseFailure>(matches!(&buffer, Some(BufferData::External { .. })))
            })
        })?;
        if external && let Some(BufferData::External { identity, .. }) = buffer {
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    closure: &ResourceClosure<'_>,
    resource_offset: u32,
) -> Result<ExternalImageResources<'scope>> {
    let mut values = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-external-image-resources",
    );
    values.try_reserve(&mut ctx.services()?, resolved_texture_uris.values.len())?;
    for (ordinal, image) in gltf.images().enumerate() {
        poll_parser_loop(&mut ctx.services()?, ordinal)?;
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
        let origin = ctx.services()?.root_origin();
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    external_images: &ExternalImageResources<'scope>,
    prepared: &PreparedDocument<'_, '_>,
) -> Result<AppearanceStorage<'scope>> {
    let resolved_texture_uris = &prepared.resolved_texture_uris;
    let material_contracts = &prepared.material_contracts;
    let closure = &prepared.resource_closure;
    let material_count = {
        ctx.poll()?;
        let value = gltf.materials().count();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    let mut storage = AppearanceStorage {
        images: ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-images",
        ),
        image_names: ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-normalized-image-names",
        ),
        encoded_image_payloads: ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-encoded-image-payloads",
        ),
        samplers: ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-samplers",
        ),
        sampler_names: ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-normalized-sampler-names",
        ),
        sampler_provenance: ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-sampler-provenance",
        ),
        textures: ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-textures",
        ),
        texture_provenance: ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-texture-provenance",
        ),
        texture_names: ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-texture-names",
        ),
        bindings: ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-material-bindings",
        ),
        materials: ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-materials",
        ),
        image_ids: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            gltf.images().count(),
            None,
            "gltf-image-id-map",
        )?,
        sampler_ids: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            gltf.samplers().count(),
            None,
            "gltf-sampler-id-map",
        )?,
        texture_ids: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            gltf.textures().count(),
            None,
            "gltf-texture-id-map",
        )?,
        default_sampler: None,
        default_material: None,
        material_ids: try_vec_filled(
            &mut ctx.services()?,
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
    authoring_u32(
        &mut ctx.services()?,
        material_count,
        "gltf-authoring-material-count",
    )?;
    storage
        .materials
        .try_reserve(&mut ctx.services()?, material_count)?;
    for (material_index, material) in gltf.materials().enumerate() {
        poll_parser_loop(&mut ctx.services()?, material_index)?;
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
            &mut ctx.services()?,
            storage.materials.len(),
            "gltf-authoring-material-id",
        )?);
        materials::add_authoring(
            ctx,
            &mut storage,
            gltf,
            material,
            resolved_texture_uris,
            external_images,
            material_contracts,
        )?;
        let Some(slot) = storage.material_ids.as_mut_slice().get_mut(material_index) else {
            return Err(gltf_parse_error("glTF material id map is incomplete"));
        };
        *slot = Some(local_id);
    }
    if prepared.geometry_contracts.needs_default_material {
        materials::add_default_authoring(ctx, &mut storage)?;
    }
    Ok(storage)
}

fn build_shared_appearance_resources<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    closure: &ResourceClosure<'_>,
    storage: &mut AppearanceStorage<'scope>,
) -> Result<()> {
    let selected_image_count = selected_count(ctx, closure.images.as_slice())?;
    storage
        .images
        .try_reserve(&mut ctx.services()?, selected_image_count)?;
    storage
        .image_names
        .try_reserve(&mut ctx.services()?, selected_image_count)?;
    storage
        .encoded_image_payloads
        .try_reserve(&mut ctx.services()?, selected_image_count)?;
    for (image_index, image) in gltf.images().enumerate() {
        poll_parser_loop(&mut ctx.services()?, image_index)?;
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
                let media_type = image_media_type(
                    &mut ctx.services()?,
                    image_index,
                    mime_type.or(decoded.media_type),
                )?;
                (
                    store_encoded_image_payload(storage, decoded.bytes)?,
                    0,
                    media_type,
                )
            }
            ImageSource::Uri { mime_type, .. } => {
                let resolved = resolved_texture_uris.get(&image_index).ok_or_else(|| {
                    gltf_parse_error(format_args!(
                        "glTF texture resolution plan is missing image {image_index}"
                    ))
                })?;
                let (resource_index, _) =
                    external_images.get(ctx, image_index)?.ok_or_else(|| {
                        gltf_parse_error(format_args!(
                            "glTF texture resource is missing image {image_index}"
                        ))
                    })?;
                let media_type =
                    mime_type
                        .map(ExtensionMediaType::new)
                        .transpose()
                        .map_err(|_| {
                            gltf_parse_error(format_args!(
                                "glTF image {image_index} has an invalid media type"
                            ))
                        })?;
                let payload = if resolved.is_empty() {
                    ImagePayloadStorage::SessionResource(ResourceIdentityLocalId::new(
                        resource_index,
                    ))
                } else {
                    let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
                    ctx.child(&layout, [to_u64(resolved.len())], |child| {
                        child.run([to_u64(resolved.len())], || {
                            Ok::<_, ContentError>(ImagePayloadStorage::SessionResource(
                                ResourceIdentityLocalId::new(resource_index),
                            ))
                        })
                    })?
                };
                (payload, resource_index, media_type)
            }
            ImageSource::View { view, mime_type } => {
                let (bytes, provenance) = copy_buffer_view_image(ctx, buffers, view, image_index)?;
                (
                    store_encoded_image_payload(storage, bytes)?,
                    provenance,
                    image_media_type(&mut ctx.services()?, image_index, Some(mime_type))?,
                )
            }
        };
        let local_id = ImageLocalId::new(authoring_u32(
            &mut ctx.services()?,
            storage.images.len(),
            "gltf-authoring-image-id",
        )?);
        storage.images.try_push(
            &mut ctx.services()?,
            ImageStorage {
                payload,
                provenance,
                media_type,
            },
        )?;
        let name_bytes = image.name().map_or(0, str::len);
        let name = if name_bytes == 0 {
            image.name()
        } else {
            let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
            ctx.child(&layout, [to_u64(name_bytes)], |child| {
                child.run([to_u64(name_bytes)], || Ok::<_, ContentError>(image.name()))
            })?
        };
        storage.image_names.try_push_reserved(owned_name(
            ctx,
            name,
            "gltf-normalized-image-name",
        )?)?;
        let Some(slot) = storage.image_ids.as_mut_slice().get_mut(image_index) else {
            return Err(gltf_parse_error("glTF image id map is incomplete"));
        };
        *slot = Some(local_id);
    }

    let selected_sampler_count = selected_count(ctx, closure.samplers.as_slice())?;
    storage
        .samplers
        .try_reserve(&mut ctx.services()?, selected_sampler_count)?;
    storage.sampler_names.try_reserve(
        &mut ctx.services()?,
        selected_sampler_count.saturating_add(1),
    )?;
    storage.sampler_provenance.try_reserve(
        &mut ctx.services()?,
        selected_sampler_count.saturating_add(1),
    )?;
    for (sampler_index, sampler) in gltf.samplers().enumerate() {
        poll_parser_loop(&mut ctx.services()?, sampler_index)?;
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
            &mut ctx.services()?,
            storage.samplers.len(),
            "gltf-authoring-sampler-id",
        )?);
        storage.samplers.try_push(
            &mut ctx.services()?,
            SamplerAuthoring::new(map_sampler(sampler)),
        )?;
        let name_bytes = sampler.name().map_or(0, str::len);
        let name = if name_bytes == 0 {
            sampler.name()
        } else {
            let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
            ctx.child(&layout, [to_u64(name_bytes)], |child| {
                child.run([to_u64(name_bytes)], || {
                    Ok::<_, ContentError>(sampler.name())
                })
            })?
        };
        storage.sampler_names.try_push_reserved(owned_name(
            ctx,
            name,
            "gltf-normalized-sampler-name",
        )?)?;
        storage
            .sampler_provenance
            .try_push(&mut ctx.services()?, 0)?;
        let Some(slot) = storage.sampler_ids.as_mut_slice().get_mut(sampler_index) else {
            return Err(gltf_parse_error("glTF sampler id map is incomplete"));
        };
        *slot = Some(local_id);
    }

    let selected_texture_count = selected_count(ctx, closure.textures.as_slice())?;
    storage
        .textures
        .try_reserve(&mut ctx.services()?, selected_texture_count)?;
    storage
        .texture_provenance
        .try_reserve(&mut ctx.services()?, selected_texture_count)?;
    storage
        .texture_names
        .try_reserve(&mut ctx.services()?, selected_texture_count)?;
    for (texture_index, texture) in gltf.textures().enumerate() {
        poll_parser_loop(&mut ctx.services()?, texture_index)?;
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
            return Err(gltf_parse_error(format_args!(
                "glTF texture {texture_index} references an image outside the closure"
            )));
        };
        let sampler_id = match texture.sampler().index() {
            Some(sampler_index) => storage
                .sampler_ids
                .as_slice()
                .get(sampler_index)
                .copied()
                .flatten()
                .ok_or_else(|| {
                    gltf_parse_error(format_args!(
                        "glTF texture {texture_index} references a sampler outside the closure"
                    ))
                })?,
            None => {
                if let Some(default_sampler) = storage.default_sampler {
                    default_sampler
                } else {
                    let default_sampler = SamplerLocalId::new(authoring_u32(
                        &mut ctx.services()?,
                        storage.samplers.len(),
                        "gltf-authoring-default-sampler-id",
                    )?);
                    storage.samplers.try_push(
                        &mut ctx.services()?,
                        SamplerAuthoring::new(map_sampler(texture.sampler())),
                    )?;
                    storage.sampler_names.try_push_reserved(None)?;
                    storage
                        .sampler_provenance
                        .try_push(&mut ctx.services()?, 0)?;
                    storage.default_sampler = Some(default_sampler);
                    default_sampler
                }
            }
        };
        let local_id = TextureLocalId::new(authoring_u32(
            &mut ctx.services()?,
            storage.textures.len(),
            "gltf-authoring-texture-id",
        )?);
        storage.textures.try_push(
            &mut ctx.services()?,
            TextureAuthoring::new(image_id, sampler_id),
        )?;
        storage
            .texture_provenance
            .try_push(&mut ctx.services()?, 0)?;
        storage.texture_names.try_push_reserved(owned_name(
            ctx,
            texture.name(),
            "gltf-normalized-texture-name",
        )?)?;
        let Some(slot) = storage.texture_ids.as_mut_slice().get_mut(texture_index) else {
            return Err(gltf_parse_error("glTF texture id map is incomplete"));
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
    _ctx: &mut ParseServices<'_, '_, '_>,
    image_index: usize,
    media_type: Option<&str>,
) -> Result<Option<ExtensionMediaType>> {
    media_type
        .map(ExtensionMediaType::new)
        .transpose()
        .map_err(|_| {
            gltf_parse_error(format_args!(
                "glTF image {image_index} has an invalid media type"
            ))
        })
}

fn copy_buffer_view_image<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    buffers: &[Option<BufferData<'_>>],
    view: crate::document::View<'_>,
    image_index: usize,
) -> Result<(MeteredVec<'scope, u8>, u32)> {
    let buffer_index = view.buffer().index();
    let buffer = buffers
        .get(buffer_index)
        .and_then(Option::as_ref)
        .ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF image {image_index} references unavailable buffer {buffer_index}"
            ))
        })?;
    let end = view
        .offset()
        .checked_add(view.length())
        .ok_or_else(|| ctx.allocation_failure("gltf-image-buffer-view-range"))?;
    let bytes = buffer.as_slice().get(view.offset()..end).ok_or_else(|| {
        gltf_parse_error(format_args!(
            "glTF image {image_index} bufferView {} exceeds buffer {buffer_index}",
            view.index()
        ))
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    material_count: usize,
    closure: &ResourceClosure<'_>,
    materials: &MaterialContracts<'_>,
) -> Result<GeometryContracts<'scope>> {
    let mesh_count = {
        ctx.poll()?;
        let value = gltf.meshes().count();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    let selected_mesh_count = selected_count(ctx, closure.meshes.as_slice())?;
    {
        let argument_0 = ctx.services()?.limits().max_meshes;
        enforce_usize_limit(
            &mut ctx.services()?,
            "max_meshes",
            selected_mesh_count,
            argument_0,
        )
    }?;
    let mut meshes = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-geometry-contract-meshes",
    );
    meshes.try_reserve(&mut ctx.services()?, mesh_count)?;

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
        poll_parser_loop(&mut ctx.services()?, mesh_ordinal)?;
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
        let primitive_count = {
            ctx.poll()?;
            let value = mesh.primitives().len();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }?;
        total_primitives = {
            let argument_0 = ctx.services()?.limits().max_primitives;
            checked_add_with_limit(
                &mut ctx.services()?,
                "max_primitives",
                total_primitives,
                primitive_count,
                argument_0,
            )
        }?;
        let mut primitives = ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-geometry-contract-primitives",
        );
        primitives.try_reserve(&mut ctx.services()?, primitive_count)?;
        for (primitive_ordinal, primitive) in mesh.primitives().enumerate() {
            poll_parser_loop(&mut ctx.services()?, primitive_ordinal)?;
            let contract = validate_primitive_contract(
                ctx,
                gltf,
                mesh.index(),
                primitive.index(),
                material_count,
            )?;
            if let Some(material) = contract.material_index {
                let material = materials.material(&mut ctx.services()?, material)?;
                if material.requires_tangent_space {
                    let has_normal = primitive.get(&Semantic::Normals).is_some();
                    let has_tangent = primitive.get(&Semantic::Tangents).is_some();
                    if !has_normal || !has_tangent {
                        return Err(gltf_parse_error(
                            "glTF clearcoatNormalTexture requires NORMAL and TANGENT attributes or a base normal texture",
                        ));
                    }
                }
            }
            needs_default_material |= contract.material_index.is_none();
            total_vertices = {
                let argument_0 = ctx.services()?.limits().max_vertices;
                checked_add_with_limit(
                    &mut ctx.services()?,
                    "max_vertices",
                    total_vertices,
                    contract.position_count,
                    argument_0,
                )
            }?;
            total_faces = {
                let argument_0 = ctx.services()?.limits().max_faces;
                checked_add_with_limit(
                    &mut ctx.services()?,
                    "max_faces",
                    total_faces,
                    contract.face_count,
                    argument_0,
                )
            }?;
            total_topology_cells = {
                let argument_0 = ctx.services()?.limits().max_total_topology_cells;
                checked_add_with_limit(
                    &mut ctx.services()?,
                    "max_total_topology_cells",
                    total_topology_cells,
                    contract.index_count,
                    argument_0,
                )
            }?;
            total_attribute_streams = {
                let argument_0 = ctx.services()?.limits().max_total_vertex_attribute_streams;
                checked_add_with_limit(
                    &mut ctx.services()?,
                    "max_total_vertex_attribute_streams",
                    total_attribute_streams,
                    contract.attribute_stream_count,
                    argument_0,
                )
            }?;
            let attribute_cells = contract
                .position_count
                .checked_mul(contract.attribute_stream_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-attribute-cell-counter"))?;
            total_attribute_cells = {
                let argument_0 = ctx.services()?.limits().max_total_vertex_attribute_cells;
                checked_add_with_limit(
                    &mut ctx.services()?,
                    "max_total_vertex_attribute_cells",
                    total_attribute_cells,
                    attribute_cells,
                    argument_0,
                )
            }?;
            total_influence_cells = {
                let argument_0 = ctx.services()?.limits().max_influence_cells;
                checked_add_with_limit(
                    &mut ctx.services()?,
                    "max_influence_cells",
                    total_influence_cells,
                    contract.influences.cell_count,
                    argument_0,
                )
            }?;
            authoring_u32(
                &mut ctx.services()?,
                total_influence_cells,
                "gltf-total-influence-cells",
            )?;
            total_influence_offsets = total_influence_offsets
                .checked_add(contract.influences.offset_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-total-influence-offsets"))?;
            authoring_u32(
                &mut ctx.services()?,
                total_influence_offsets,
                "gltf-total-influence-offsets",
            )?;
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
    ctx: &mut ParseRoot<'_, '_, '_>,
    gltf: &crate::document::Document<'_>,
    mesh_index: usize,
    primitive_index: usize,
    material_count: usize,
) -> Result<PrimitiveContract> {
    let primitive = json_primitive(&mut ctx.services()?, gltf, mesh_index, primitive_index)?;
    let attribute_stream_count = primitive.attributes.len();
    {
        let argument_0 = ctx
            .services()?
            .limits()
            .max_vertex_attribute_streams_per_mesh;
        enforce_usize_limit(
            &mut ctx.services()?,
            "max_vertex_attribute_streams_per_mesh",
            attribute_stream_count,
            argument_0,
        )
    }?;
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
    let position_count = accessor::validate_mesh_attribute(
        ctx,
        gltf,
        position_index,
        owner,
        MeshAttributeUse::Base(Semantic::Positions),
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
        let info = accessor::validate_mesh_attribute(
            ctx,
            gltf,
            accessor.value(),
            owner,
            MeshAttributeUse::Base(*semantic),
        )?;
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
                &mut ctx.services()?,
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
    let face_count = primitive_face_count(&mut ctx.services()?, owner, mode, corner_count)?;
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
    _ctx: &mut ParseServices<'_, '_, '_>,
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
            return Err(gltf_parse_error(format_args!(
                "{owner} mode {mode:?} has an invalid draw index count {corners}"
            )));
        }
    };
    Ok(faces)
}

fn json_primitive<'a>(
    ctx: &mut ParseServices<'_, '_, '_>,
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

fn poll_parser_loop(ctx: &mut ParseServices<'_, '_, '_>, completed: usize) -> Result<()> {
    if completed.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
        ctx.poll()?;
    }
    Ok(())
}

fn collect_exact<'scope, T>(
    ctx: &mut ParseServices<'_, '_, 'scope>,
    class: ImportStorageClass,
    expected: usize,
    allocation: &'static str,
    values: impl Iterator<Item = T>,
) -> Result<MeteredVec<'scope, T>> {
    let mut collected = ctx.metered_vec(class, allocation);
    collected.try_reserve(ctx, expected)?;
    for value in values {
        poll_parser_loop(ctx, collected.len())?;
        if collected.len() == expected {
            return Err(gltf_parse_error(format_args!(
                "{allocation} produced more than {expected} values"
            )));
        }
        collected.try_push_reserved(value)?;
    }
    if collected.len() != expected {
        return Err(gltf_parse_error(format_args!(
            "{allocation} produced {} values, expected {expected}",
            collected.len()
        )));
    }
    Ok(collected)
}

fn collect_exact_with_work<'scope, T, const N: usize>(
    ctx: &mut ParseParser<'_, '_, '_, '_, 'scope, N, true>,
    expected: usize,
    allocation: &'static str,
    _operation: &'static str,
    mut values: impl Iterator<Item = T>,
    per_item: [u64; N],
) -> Result<MeteredVec<'scope, T>> {
    let mut collected = ctx
        .services()?
        .metered_vec(ImportStorageClass::FormatNormalization, allocation);
    collected.try_reserve(&mut ctx.services()?, expected)?;
    loop {
        if collected.len().is_multiple_of(GEOMETRY_POLL_INTERVAL) {
            ctx.poll()?;
        }
        let next = values.next();
        let Some(value) = next else {
            break;
        };
        if collected.len() == expected {
            return Err(gltf_parse_error(format_args!(
                "{allocation} produced more than {expected} values"
            )));
        }
        ctx.run(per_item, || collected.try_push_reserved(value))?;
    }
    if collected.len() != expected {
        return Err(gltf_parse_error(format_args!(
            "{allocation} produced {} values, expected {expected}",
            collected.len()
        )));
    }
    Ok(collected)
}

fn try_vec_capacity<'scope, T>(
    ctx: &mut ParseServices<'_, '_, 'scope>,
    class: ImportStorageClass,
    capacity: usize,
    allocation: &'static str,
) -> Result<MeteredVec<'scope, T>> {
    let mut values = ctx.metered_vec(class, allocation);
    values.try_reserve(ctx, capacity)?;
    Ok(values)
}

fn try_vec_filled<'scope, T: Copy>(
    ctx: &mut ParseServices<'_, '_, 'scope>,
    class: ImportStorageClass,
    length: usize,
    value: T,
    allocation: &'static str,
) -> Result<MeteredVec<'scope, T>> {
    let mut values = try_vec_capacity(ctx, class, length, allocation)?;
    for index in 0..length {
        poll_parser_loop(ctx, index)?;
        values.try_push_reserved(value)?;
    }
    Ok(values)
}

fn build_geometry_storage<'region, 'scope>(
    ctx: &mut ParseRoot<'_, 'region, 'scope>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    contracts: &GeometryContracts<'_>,
    appearance: &AppearanceStorage<'_>,
) -> Result<GeometryStorage<'scope>> {
    let mesh_count = {
        ctx.poll()?;
        let value = gltf.meshes().count();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    if contracts.meshes.len() != mesh_count {
        return Err(gltf_parse_error(
            "glTF geometry contract mesh count changed before materialization",
        ));
    }
    let mut logical_meshes = ctx.services()?.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-authoring-logical-meshes",
    );
    let mut selected_mesh_count = 0;
    for (index, mesh) in contracts.meshes.as_slice().iter().enumerate() {
        poll_parser_loop(&mut ctx.services()?, index)?;
        selected_mesh_count += usize::from(mesh.is_some());
    }
    logical_meshes.try_reserve(&mut ctx.services()?, selected_mesh_count)?;
    let mut logical_mesh_ids = try_vec_filled(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        mesh_count,
        None,
        "gltf-logical-mesh-id-map",
    )?;
    let available_provenance_count = {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        ctx.child(&layout, [to_u64(buffers.len())], |child| -> Result<_> {
            let mut count = 0usize;
            for (index, buffer) in buffers.iter().enumerate() {
                poll_parser_loop(&mut child.services()?, index)?;
                child.run([1], || {
                    count += usize::from(buffer.is_some());
                    Ok::<_, ContentError>(())
                })?;
            }
            Ok(count.max(1))
        })
    }?;
    let mut primitive_provenance_capacity = 0usize;
    for (mesh_index, mesh) in contracts.meshes.as_slice().iter().enumerate() {
        poll_parser_loop(&mut ctx.services()?, mesh_index)?;
        let Some(mesh) = mesh else { continue };
        for (index, primitive) in mesh.primitives.as_slice().iter().enumerate() {
            poll_parser_loop(&mut ctx.services()?, index)?;
            let reference_count = primitive
                .attribute_stream_count
                .checked_add(1)
                .and_then(|count| count.checked_mul(AccessorSources::MAX_SOURCES))
                .ok_or_else(|| ctx.allocation_failure("gltf-primitive-buffer-provenance-count"))?;
            primitive_provenance_capacity = primitive_provenance_capacity
                .checked_add(reference_count.min(available_provenance_count))
                .ok_or_else(|| ctx.allocation_failure("gltf-primitive-buffer-provenance-count"))?;
        }
    }
    let mut primitive_provenance = try_vec_capacity(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        primitive_provenance_capacity,
        "gltf-primitive-buffer-provenance",
    )?;
    let mut pending_provenance = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-primitive-pending-provenance",
    );
    let primitive_work = ctx.bind_group([ImportWorkClass::Primitives, ImportWorkClass::Faces])?;
    let mesh_work = ctx.bind_group([
        ImportWorkClass::Meshes,
        ImportWorkClass::CanonicalStringBytes,
    ])?;
    for (mesh_ordinal, mesh) in gltf.meshes().enumerate() {
        poll_parser_loop(&mut ctx.services()?, mesh_ordinal)?;
        let mesh_contracts = contracts
            .meshes
            .as_slice()
            .get(mesh_ordinal)
            .ok_or_else(|| gltf_parse_error("glTF geometry contract mesh table is incomplete"))?;
        let Some(mesh_contracts) = mesh_contracts.as_ref() else {
            continue;
        };
        let logical_mesh_id = LogicalMeshLocalId::new(authoring_u32(
            &mut ctx.services()?,
            logical_meshes.len(),
            "gltf-authoring-logical-mesh-id",
        )?);
        let Some(slot) = logical_mesh_ids.as_mut_slice().get_mut(mesh_ordinal) else {
            return Err(gltf_parse_error("glTF logical mesh id map is incomplete"));
        };
        *slot = Some(logical_mesh_id);
        let name_bytes = mesh.name().map_or(0, str::len);
        let name = ctx.child(&mesh_work, [1, to_u64(name_bytes)], |child| {
            child.run([1, to_u64(name_bytes)], || {
                Ok::<_, ContentError>(mesh.name())
            })
        })?;
        let primitive_count = {
            ctx.poll()?;
            let value = mesh.primitives().len();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }?;
        if mesh_contracts.primitives.len() != primitive_count {
            return Err(gltf_parse_error(
                "glTF geometry contract primitive count changed before materialization",
            ));
        }
        let mut primitives = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-primitives-per-logical-mesh",
        );
        primitives.try_reserve(&mut ctx.services()?, primitive_count)?;
        for (primitive_ordinal, primitive) in mesh.primitives().enumerate() {
            poll_parser_loop(&mut ctx.services()?, primitive_ordinal)?;
            let contract = *mesh_contracts
                .primitives
                .as_slice()
                .get(primitive_ordinal)
                .ok_or_else(|| {
                    gltf_parse_error("glTF geometry contract primitive table is incomplete")
                })?;
            primitives.try_push_reserved(build_primitive_authoring(
                ctx,
                &primitive_work,
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
            name: owned_name(ctx, name, "gltf-normalized-mesh-name")?,
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
fn build_primitive_authoring<'region, 'scope>(
    ctx: &mut ParseRoot<'_, 'region, 'scope>,
    primitive_work: &ParseWorkLayout<'region, 2>,
    buffers: &[Option<BufferData<'_>>],
    mesh: &crate::document::Mesh<'_>,
    primitive: crate::document::Primitive<'_>,
    contract: PrimitiveContract,
    appearance: &AppearanceStorage<'_>,
    provenance: &mut MeteredVec<'scope, u32>,
    pending_provenance: &mut MeteredVec<'scope, u32>,
) -> Result<PrimitiveStorage<'scope>> {
    let accessor_count = contract
        .attribute_stream_count
        .checked_add(1)
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-provenance-references"))?;
    let provenance_references = accessor_count
        .checked_mul(AccessorSources::MAX_SOURCES)
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-provenance-references"))?;
    // Each accessor contributes to the primitive and, for attributes, its own retained row.
    let collection_work = to_u64(accessor_count)
        .checked_mul(2 * AccessorSources::COLLECTION_WORK_BOUND)
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-provenance-work"))?;
    let collection_layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    let source_attributes = ctx.child(&collection_layout, [collection_work], |child| {
        child.run([collection_work], || {
            Ok::<_, ContentError>(primitive.attributes())
        })
    })?;
    pending_provenance.try_reserve(&mut ctx.services()?, provenance_references)?;
    let (output, non_unit_rows) = ctx.parent(primitive_work, [1, to_u64(contract.face_count)], |parent| -> Result<_> {
            let mut children_loan = parent.children()?;
            let children = &mut children_loan;
            let provenance_offset = provenance.len();
            for (ordinal, (_, accessor)) in source_attributes.enumerate() {
                poll_parser_loop(&mut children.services()?, ordinal)?;
                pending_provenance.try_extend_from_slice_reserved(
                    accessor_provenance(&mut children.services()?, buffers, accessor)?.as_slice(),
                )?;
            }
            if let Some(accessor) = primitive.indices() {
                pending_provenance.try_extend_from_slice_reserved(
                    accessor_provenance(&mut children.services()?, buffers, accessor)?.as_slice(),
                )?;
            }
            try_extend_provenance_unique(children, provenance, pending_provenance)?;
            let position_source = primitive.get(&Semantic::Positions)
                .ok_or_else(|| gltf_parse_error("glTF POSITION accessor disappeared after preflight"))?;
            let positions = accessor::collect_mesh_attribute::<3>(
                children, position_source, buffers, Semantic::Positions,
                contract.position_count, "gltf-position-values",
            )?;
            let normals = primitive.get(&Semantic::Normals).map(|source| {
                accessor::collect_mesh_attribute::<3>(
                    children, source, buffers, Semantic::Normals,
                    contract.position_count, "gltf-normal-values",
                )
            })
            .transpose()?;
            let normals = match normals {
                Some(normals) => normals,
                None => children.services()?.metered_vec(ImportStorageClass::SceneConstruction, "gltf-normal-values"),
            };
            let tangents = primitive.get(&Semantic::Tangents).map(|source| {
                accessor::collect_mesh_attribute::<4>(
                    children, source, buffers, Semantic::Tangents,
                    contract.position_count, "gltf-tangent-values",
                )
            })
            .transpose()?;
            let tangents = match tangents {
                Some(tangents) => tangents,
                None => children.services()?.metered_vec(ImportStorageClass::SceneConstruction, "gltf-tangent-values"),
            };
            let indices = { children.services()?.poll()?; let value = primitive.indices().map(|source| -> Result<_> {
                    let indices = accessor::bind(&mut children.services()?, source, buffers)?.indices().ok_or_else(|| {
                        GltfFailure::invalid_accessor(Some(source.index()), "invalid index accessor encoding")
                    })?;
                    collect_indices(children, contract, indices)
                }).transpose(); children.poll_result(value) }?;
            let indices = match indices {
                Some(indices) => indices,
                None if contract.index_count == 0 => {
                    children.services()?.metered_vec(ImportStorageClass::SceneConstruction, "gltf-index-values")
                }
                None => return Err(gltf_parse_error("glTF indices accessor could not be read")),
            };

            let mut attributes = children.services()?.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-authoring-primitive-attributes",
            );
            { let value = AttributeStorage {
                semantic: AttributeSemantic::Position,
                lanes: 3,
                values: positions,
                provenance: accessor_provenance(&mut children.services()?,
                    buffers,
                    primitive.get(&Semantic::Positions).ok_or_else(|| {
                        gltf_parse_error("glTF POSITION accessor disappeared after preflight")
                    })?,
                )?,
            }; attributes.try_push(&mut children.services()?, value) }?;
            if !normals.is_empty() {
                { let value = AttributeStorage {
                    semantic: AttributeSemantic::Normal,
                    lanes: 3,
                    values: normals,
                    provenance: accessor_provenance(&mut children.services()?,
                        buffers,
                        primitive.get(&Semantic::Normals).ok_or_else(|| {
                            gltf_parse_error("glTF NORMAL accessor disappeared after preflight")
                        })?,
                    )?,
                }; attributes.try_push(&mut children.services()?, value) }?;
            }
            if !tangents.is_empty() {
                { let value = AttributeStorage {
                    semantic: AttributeSemantic::Tangent,
                    lanes: 4,
                    values: tangents,
                    provenance: accessor_provenance(&mut children.services()?,
                        buffers,
                        primitive.get(&Semantic::Tangents).ok_or_else(|| {
                            gltf_parse_error("glTF TANGENT accessor disappeared after preflight")
                        })?,
                    )?,
                }; attributes.try_push(&mut children.services()?, value) }?;
            }
            for set in 0..8 {
                let Some(values) = primitive.get(&Semantic::TexCoords(set)).map(|source| {
                    accessor::collect_mesh_attribute::<2>(
                        children, source, buffers, Semantic::TexCoords(set),
                        contract.position_count, "gltf-texcoord-values",
                    )
                })
                .transpose()?
                else {
                    break;
                };
                { let value = AttributeStorage {
                    semantic: AttributeSemantic::Texcoord(set),
                    lanes: 2,
                    values,
                    provenance: accessor_provenance(&mut children.services()?,
                        buffers,
                        primitive.get(&Semantic::TexCoords(set)).ok_or_else(|| {
                            gltf_parse_error(
                                format_args!("glTF TEXCOORD_{set} accessor disappeared after preflight"),
                            )
                        })?,
                    )?,
                }; attributes.try_push(&mut children.services()?, value) }?;
            }
            for set in 0..8 {
                let Some(values) = primitive.get(&Semantic::Colors(set)).map(|source| -> Result<_> {
                    match source.row().type_ {
                        json::accessor::Type::Vec3 => accessor::collect_mesh_attribute::<3>(
                            children, source, buffers, Semantic::Colors(set),
                            contract.position_count, "gltf-color-values",
                        ),
                        json::accessor::Type::Vec4 => accessor::collect_mesh_attribute::<4>(
                            children, source, buffers, Semantic::Colors(set),
                            contract.position_count, "gltf-color-values",
                        ),
                        _ => Err(GltfFailure::invalid_accessor(Some(source.index()), "invalid color accessor shape").into()),
                    }
                })
                .transpose()?
                else {
                    break;
                };
                { let value = AttributeStorage {
                    semantic: AttributeSemantic::Color(set),
                    lanes: 4,
                    values,
                    provenance: accessor_provenance(&mut children.services()?,
                        buffers,
                        primitive.get(&Semantic::Colors(set)).ok_or_else(|| {
                            gltf_parse_error(
                                format_args!("glTF COLOR_{set} accessor disappeared after preflight"),
                            )
                        })?,
                    )?,
                }; attributes.try_push(&mut children.services()?, value) }?;
            }
            let influences = { children.services()?.poll()?; let value = influences::build_rows(children, buffers, &primitive, contract.influences); children.poll_result(value) }?;
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
                                    format_args!(
                                        "glTF primitive references material {index} outside the closure"
                                    ),
                                )
                            })?,
                    ),
                    None => MaterialAssignment::ImplicitDefault(
                        appearance.default_material.ok_or_else(|| {
                            gltf_parse_error("glTF selected primitive has no default material")
                        })?,
                    ),
                },
                attributes,
                influence_offsets: influences.offsets,
                influences: influences.values,
                provenance_offset,
                provenance_count: provenance.len() - provenance_offset,
            }, influences.non_unit_rows))
        })?;
    if non_unit_rows != 0 {
        push_warning(
            &mut ctx.services()?,
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
    ctx: &mut ParseChildren<'_, '_, 'scope>,
    contract: PrimitiveContract,
    indices: crate::document::read::Indices<'_>,
) -> Result<MeteredVec<'scope, u32>> {
    {
        let layout = ctx.bind_group([ImportWorkClass::TopologyCells])?;
        ctx.child(
            &layout,
            [to_u64(contract.index_count)],
            |child| -> Result<_> {
                let restart = indices.restart;
                let mut collected = child
                    .services()?
                    .metered_vec(ImportStorageClass::SceneConstruction, "gltf-index-values");
                collected.try_reserve(&mut child.services()?, contract.index_count)?;
                for (ordinal, index) in indices.enumerate() {
                    poll_parser_loop(&mut child.services()?, ordinal)?;
                    child.run([1], || -> Result<()> {
                        if collected.len() == contract.index_count {
                            return Err(gltf_parse_error(
                                "glTF index stream exceeds its admitted count",
                            ));
                        }
                        // Reject restart before the source component type is lost.
                        if index == restart || index as usize >= contract.position_count {
                            return Err(gltf_parse_error(format_args!(
                                "glTF index {} contains invalid vertex index {index}",
                                collected.len()
                            )));
                        }
                        collected.try_push_reserved(index)?;
                        Ok(())
                    })?;
                }
                if collected.len() != contract.index_count {
                    return Err(gltf_parse_error(
                        "glTF index stream differs from its admitted count",
                    ));
                }
                Ok(collected)
            },
        )
    }
}

fn build_scene_nodes<'scope, 'document>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    graph: &NodeGraph<'document, '_>,
    logical_mesh_ids: &MeteredVec<'_, Option<LogicalMeshLocalId>>,
    cameras: &CameraStorage<'_>,
    lights: &lights::LightStorage<'_>,
    node_lights: &[Option<json::Index>],
) -> Result<NodeStorage<'scope>> {
    let selected_node_count = selected_count(ctx, graph.selected.as_slice())?;
    let mut storage = NodeStorage {
        roots: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::SceneConstruction,
            graph.roots.len(),
            "gltf-authoring-roots",
        )?,
        nodes: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::SceneConstruction,
            selected_node_count,
            "gltf-authoring-nodes",
        )?,
        names: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            selected_node_count,
            "gltf-normalized-node-names",
        )?,
        node_ids: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            graph.document.tables().nodes.len(),
            None,
            "gltf-node-id-map",
        )?,
        mesh_instances: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::SceneConstruction,
            selected_node_count,
            "gltf-authoring-mesh-instances",
        )?,
    };
    let mut pending = try_vec_capacity(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        selected_node_count,
        "gltf-node-build-stack",
    )?;
    for (root_ordinal, node_index) in graph.roots.as_slice().iter().rev().copied().enumerate() {
        poll_parser_loop(&mut ctx.services()?, root_ordinal)?;
        pending.try_push_reserved((node_index, Presence::Absent))?;
    }

    let mut constructed = 0usize;
    while let Some((node_index, parent)) = pending.pop() {
        poll_parser_loop(&mut ctx.services()?, constructed)?;
        constructed += 1;
        let node = graph.document.node(node_index).ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF node graph references missing node {node_index}"
            ))
        })?;
        let slot = storage
            .node_ids
            .as_mut_slice()
            .get_mut(node_index)
            .ok_or_else(|| {
                gltf_parse_error(format_args!("glTF node index {node_index} is out of range"))
            })?;
        if slot.is_some() {
            return Err(gltf_parse_error(format_args!(
                "glTF node {node_index} was constructed more than once"
            )));
        }
        let node_id = NodeLocalId::new(authoring_u32(
            &mut ctx.services()?,
            storage.nodes.len(),
            "gltf-authoring-node-id",
        )?);
        *slot = Some(node_id);
        if matches!(parent, Presence::Absent) {
            storage.roots.try_push_reserved(node_id)?;
        }

        let transform = match {
            ctx.poll()?;
            let value = node.transform();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }? {
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
            gltf_parse_error(format_args!("glTF node {node_index} transform is invalid"))
        })?;
        let camera = match {
            ctx.poll()?;
            let value = node.camera();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }? {
            Some(camera) => cameras
                .camera_ids
                .as_slice()
                .get(camera.index())
                .copied()
                .flatten()
                .ok_or_else(|| {
                    gltf_parse_error(format_args!(
                        "glTF selected node {node_index} references camera {} outside the closure",
                        camera.index()
                    ))
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
                        gltf_parse_error("selected glTF node light is outside the closure")
                    })
            })
            .transpose()?;
        let authored = NodeAuthoringNode::new(
            parent,
            transform,
            camera.map_or(Presence::Absent, Presence::Present),
            light.map_or(Presence::Absent, Presence::Present),
        );
        let name = {
            ctx.poll()?;
            let value = node.name();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }?;
        let name = match name {
            Some(name) if !name.is_empty() => {
                let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
                ctx.child(&layout, [to_u64(name.len())], |child| {
                    child.run([to_u64(name.len())], || Ok::<_, ContentError>(node.name()))
                })?
            }
            name => name,
        };
        storage
            .names
            .try_push_reserved(owned_name(ctx, name, "gltf-normalized-node-name")?)?;
        storage.nodes.try_push_reserved(authored)?;

        let skin_index = {
            ctx.poll()?;
            let value = node.skin();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }?
        .map(|skin| skin.index());
        if let Some(mesh) = {
            ctx.poll()?;
            let value = node.mesh();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }? {
            let logical_mesh = logical_mesh_ids
                .as_slice()
                .get(mesh.index())
                .copied()
                .flatten()
                .ok_or_else(|| {
                    gltf_parse_error(format_args!(
                        "glTF selected node {node_index} references mesh {} outside the closure",
                        mesh.index()
                    ))
                })?;
            storage
                .mesh_instances
                .try_push_reserved(MeshInstanceStorage {
                    authored: GeometryMeshInstanceAuthoring::new(node_id, logical_mesh),
                    skin_index,
                    source_node_index: node_index,
                })?;
        }

        for (completed, (child_ordinal, child_index)) in graph.children.as_slice()[node_index]
            .as_slice()
            .iter()
            .copied()
            .enumerate()
            .rev()
            .enumerate()
        {
            poll_parser_loop(&mut ctx.services()?, completed)?;
            pending.try_push_reserved((
                child_index,
                Presence::Present(NodeParent::new(
                    node_id,
                    authoring_u32(
                        &mut ctx.services()?,
                        child_ordinal,
                        "gltf-authoring-child-ordinal",
                    )?,
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &'document crate::document::Document<'_>,
    graph: &NodeGraph<'document, '_>,
    animation_contracts: &animation::AnimationContracts<'_, '_>,
) -> Result<ResourceClosure<'scope>> {
    let root = gltf.tables();
    let mut animation_sampler_ranges = try_vec_capacity(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        root.animations.len(),
        "gltf-closure-animation-sampler-ranges",
    )?;
    let mut animation_sampler_count = 0usize;
    for (animation_index, animation) in root.animations.iter().enumerate() {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let sampler_count = ctx.child(&layout, [1], |child| {
            child.run([1], || Ok::<_, ContentError>(animation.samplers.len()))
        })?;
        poll_parser_loop(&mut ctx.services()?, animation_index)?;
        let end = animation_sampler_count
            .checked_add(sampler_count)
            .ok_or_else(|| ctx.allocation_failure("gltf-closure-animation-sampler-count"))?;
        animation_sampler_ranges.try_push_reserved(animation_sampler_count..end)?;
        animation_sampler_count = end;
    }
    let mut closure = ResourceClosure {
        meshes: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.meshes.len(),
            false,
            "gltf-closure-mesh-selection",
        )?,
        materials: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.materials.len(),
            false,
            "gltf-closure-material-selection",
        )?,
        textures: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.textures.len(),
            false,
            "gltf-closure-texture-selection",
        )?,
        images: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.images.len(),
            false,
            "gltf-closure-image-selection",
        )?,
        samplers: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.samplers.len(),
            false,
            "gltf-closure-sampler-selection",
        )?,
        skins: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.skins.len(),
            false,
            "gltf-closure-skin-selection",
        )?,
        cameras: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.cameras.len(),
            false,
            "gltf-closure-camera-selection",
        )?,
        lights: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.lights.len(),
            false,
            "gltf-closure-light-selection",
        )?,
        node_lights: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.nodes.len(),
            None,
            "gltf-closure-node-lights",
        )?,
        animation_sampler_ranges,
        animation_samplers: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            animation_sampler_count,
            false,
            "gltf-closure-animation-sampler-selection",
        )?,
        buffers: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.buffers.len(),
            false,
            "gltf-closure-buffer-selection",
        )?,
        buffer_views: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.buffer_views.len(),
            false,
            "gltf-closure-buffer-view-selection",
        )?,
        accessors: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            root.accessors.len(),
            false,
            "gltf-closure-accessor-selection",
        )?,
    };

    for node_index in 0..graph.selected.len() {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let selected = ctx.child(&layout, [1], |child| {
            child.run([1], || {
                Ok::<_, ContentError>(graph.selected.as_slice()[node_index])
            })
        })?;
        poll_parser_loop(&mut ctx.services()?, node_index)?;
        if !selected {
            continue;
        }
        let node = root.nodes.get(node_index).ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF closure references missing node {node_index}"
            ))
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
                return Err(gltf_parse_error(format_args!(
                    "glTF selected node {node_index} declares a skin without a mesh"
                )));
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
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let animation_selected = ctx.child(&layout, [1], |child| {
            child.run([1], || {
                Ok::<_, ContentError>(
                    animation_contracts
                        .selected_animations
                        .as_slice()
                        .get(animation_index)
                        .copied(),
                )
            })
        })?;
        poll_parser_loop(&mut ctx.services()?, animation_index)?;
        let animation_selected = animation_selected.ok_or_else(|| {
            ContentError::from(GltfFailure::internal(
                "animation closure selection is incomplete",
            ))
        })?;
        for (channel_index, channel) in animation.channels.iter().enumerate() {
            let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            let disposition = ctx.child(&layout, [1], |child| {
                child.run([1], || {
                    Ok::<_, ContentError>(
                        animation_contracts
                            .source_channels
                            .as_slice()
                            .get(source_channel_index)
                            .copied(),
                    )
                })
            })?;
            poll_parser_loop(&mut ctx.services()?, channel_index)?;
            let disposition = disposition.ok_or_else(|| {
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
    ctx: &mut ParseRoot<'_, '_, '_>,
    mesh_index: usize,
    primitive_index: usize,
    primitive: &json::mesh::Primitive<'_>,
) -> Result<()> {
    let mut texcoord_sets = [false; 8];
    let mut color_sets = [false; 8];
    for semantic in primitive.attributes.keys() {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        ctx.child(&layout, [1], |child| child.run([1], || Ok::<_, ParseFailure>((|| -> Result<()> {
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
        Ok(())
        })())))??;
    }
    ensure_contiguous_attribute_sets(
        &mut ctx.services()?,
        mesh_index,
        primitive_index,
        "TEXCOORD",
        &texcoord_sets,
    )?;
    ensure_contiguous_attribute_sets(
        &mut ctx.services()?,
        mesh_index,
        primitive_index,
        "COLOR",
        &color_sets,
    )?;
    Ok(())
}

fn ensure_contiguous_attribute_sets(
    _ctx: &mut ParseServices<'_, '_, '_>,
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
    Err(gltf_parse_error(format_args!(
        "glTF mesh {mesh_index} primitive {primitive_index} has non-contiguous {semantic} sets"
    )))
}

fn mark_bool(
    ctx: &mut ParseRoot<'_, '_, '_>,
    values: &mut MeteredVec<'_, bool>,
    index: usize,
    kind: &'static str,
) -> Result<()> {
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    let selected = ctx.child(&layout, [1], |child| {
        child.run([1], || {
            Ok::<_, ParseFailure>(values.as_mut_slice().get_mut(index))
        })
    })?;
    let Some(selected) = selected else {
        return Err(gltf_parse_error(format_args!(
            "glTF closure references missing {kind} {index}"
        )));
    };
    *selected = true;
    Ok(())
}

fn mark_skin(
    ctx: &mut ParseRoot<'_, '_, '_>,
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
            gltf_parse_error(format_args!(
                "glTF closure references missing skin {skin_index}"
            ))
        })?;
    if was_selected {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        return ctx
            .child(&layout, [1], |child| {
                child.run([1], || {
                    Ok::<_, ContentError>(closure.skins.as_slice().get(skin_index).map(|_| ()))
                })
            })?
            .ok_or_else(|| gltf_parse_error("selected skin disappeared"));
    }
    mark_bool(ctx, &mut closure.skins, skin_index, "skin")?;
    let skin = root.skins.get(skin_index).ok_or_else(|| {
        gltf_parse_error(format_args!(
            "glTF closure references missing skin {skin_index}"
        ))
    })?;
    for (joint_ordinal, joint) in skin.joints.iter().enumerate() {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let joint_index = ctx.child(&layout, [1], |child| {
            child.run([1], || Ok::<_, ContentError>(joint.value()))
        })?;
        poll_parser_loop(&mut ctx.services()?, joint_ordinal)?;
        if !selected_nodes.get(joint_index).copied().unwrap_or(false) {
            return Err(gltf_parse_error(format_args!(
                "glTF skin {skin_index} references joint node {joint_index} outside the imported scene"
            )));
        }
    }
    if let Some(skeleton) = skin.skeleton {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let skeleton_index = ctx.child(&layout, [1], |child| {
            child.run([1], || Ok::<_, ContentError>(skeleton.value()))
        })?;
        if !selected_nodes.get(skeleton_index).copied().unwrap_or(false) {
            return Err(gltf_parse_error(format_args!(
                "glTF skin {skin_index} references skeleton root node {skeleton_index} outside the imported scene"
            )));
        }
    }
    if let Some(accessor) = skin.inverse_bind_matrices {
        mark_accessor(ctx, root, closure, accessor.value())?;
    }
    Ok(())
}

fn mark_mesh(
    ctx: &mut ParseRoot<'_, '_, '_>,
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
            gltf_parse_error(format_args!(
                "glTF closure references missing mesh {mesh_index}"
            ))
        })?;
    if was_selected {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        return ctx
            .child(&layout, [1], |child| {
                child.run([1], || {
                    Ok::<_, ContentError>(closure.meshes.as_slice().get(mesh_index).map(|_| ()))
                })
            })?
            .ok_or_else(|| gltf_parse_error("selected mesh disappeared"));
    }
    mark_bool(ctx, &mut closure.meshes, mesh_index, "mesh")?;
    let mesh = root.meshes.get(mesh_index).ok_or_else(|| {
        gltf_parse_error(format_args!(
            "glTF closure references missing mesh {mesh_index}"
        ))
    })?;
    for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let attributes = ctx.child(&layout, [1], |child| {
            child.run([1], || Ok::<_, ContentError>(primitive.attributes.values()))
        })?;
        poll_parser_loop(&mut ctx.services()?, primitive_index)?;
        validate_selected_primitive_semantics(ctx, mesh_index, primitive_index, primitive)?;
        for accessor in attributes {
            mark_accessor(ctx, root, closure, accessor.value())?;
        }
        if let Some(indices) = primitive.indices {
            mark_accessor(ctx, root, closure, indices.value())?;
        }
        for target in primitive.targets.iter().flatten() {
            let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            let accessors = ctx.child(&layout, [1], |child| {
                child.run([1], || {
                    Ok::<_, ContentError>([target.positions, target.normals, target.tangents])
                })
            })?;
            for accessor in accessors.into_iter().flatten() {
                mark_accessor(ctx, root, closure, accessor.value())?;
            }
        }
        if let Some(material) = primitive.material {
            mark_bool(ctx, &mut closure.materials, material.value(), "material")?;
        }
    }
    Ok(())
}

fn mark_texture(
    ctx: &mut ParseRoot<'_, '_, '_>,
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
            gltf_parse_error(format_args!(
                "glTF closure references missing texture {texture_index}"
            ))
        })?;
    if was_selected {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        return ctx
            .child(&layout, [1], |child| {
                child.run([1], || {
                    Ok::<_, ContentError>(
                        closure.textures.as_slice().get(texture_index).map(|_| ()),
                    )
                })
            })?
            .ok_or_else(|| gltf_parse_error("selected texture disappeared"));
    }
    mark_bool(ctx, &mut closure.textures, texture_index, "texture")?;
    let texture = root.textures.get(texture_index).ok_or_else(|| {
        gltf_parse_error(format_args!(
            "glTF closure references missing texture {texture_index}"
        ))
    })?;
    let image_index = texture.source.value();
    mark_bool(ctx, &mut closure.images, image_index, "image")?;
    let image = root.images.get(image_index).ok_or_else(|| {
        gltf_parse_error(format_args!(
            "glTF closure references missing image {image_index}"
        ))
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
    ctx: &mut ParseRoot<'_, '_, '_>,
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
            gltf_parse_error(format_args!(
                "glTF closure references missing accessor {accessor_index}"
            ))
        })?;
    if was_selected {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        return ctx
            .child(&layout, [1], |child| {
                child.run([1], || {
                    Ok::<_, ContentError>(
                        closure.accessors.as_slice().get(accessor_index).map(|_| ()),
                    )
                })
            })?
            .ok_or_else(|| gltf_parse_error("selected accessor disappeared"));
    }
    mark_bool(ctx, &mut closure.accessors, accessor_index, "accessor")?;
    let accessor = root.accessors.get(accessor_index).ok_or_else(|| {
        gltf_parse_error(format_args!(
            "glTF closure references missing accessor {accessor_index}"
        ))
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
    ctx: &mut ParseRoot<'_, '_, '_>,
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
            gltf_parse_error(format_args!(
                "glTF closure references missing bufferView {view_index}"
            ))
        })?;
    if was_selected {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        return ctx
            .child(&layout, [1], |child| {
                child.run([1], || {
                    Ok::<_, ContentError>(
                        closure.buffer_views.as_slice().get(view_index).map(|_| ()),
                    )
                })
            })?
            .ok_or_else(|| gltf_parse_error("selected bufferView disappeared"));
    }
    mark_bool(ctx, &mut closure.buffer_views, view_index, "bufferView")?;
    let view = root.buffer_views.get(view_index).ok_or_else(|| {
        gltf_parse_error(format_args!(
            "glTF closure references missing bufferView {view_index}"
        ))
    })?;
    mark_bool(ctx, &mut closure.buffers, view.buffer.value(), "buffer")?;
    Ok(())
}

fn record_source_scope(
    ctx: &mut ParseServices<'_, '_, '_>,
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
                    gltf_parse_error("glTF selected scene index exceeds source-scope range")
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
    identity.try_reserve(ctx, 32)?;

    for scene_index in 0..document.scenes.len() {
        observe_source_index(
            ctx,
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
            ctx,
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
            ctx,
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
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Mesh,
            selected,
            'm',
            mesh_index,
        )?;
        let mesh = document.meshes.get(mesh_index).ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF source-scope mesh {mesh_index} is missing"
            ))
        })?;
        for (primitive_index, _primitive) in mesh.primitives.iter().enumerate() {
            observe_source_identity(
                ctx,
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
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Material,
            selected,
            format_args!("mat{material_index}"),
        )?;
    }
    for (texture_index, selected) in closure.textures.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Texture,
            selected,
            format_args!("t{texture_index}"),
        )?;
    }
    for (image_index, selected) in closure.images.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Image,
            selected,
            format_args!("i{image_index}"),
        )?;
    }
    for (sampler_index, selected) in closure.samplers.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Sampler,
            selected,
            format_args!("ts{sampler_index}"),
        )?;
    }
    for (skin_index, selected) in closure.skins.as_slice().iter().copied().enumerate() {
        observe_source_index(
            ctx,
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
            ctx,
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
                ctx,
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
                ctx,
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
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Buffer,
            selected,
            format_args!("b{buffer_index}"),
        )?;
    }
    for (view_index, selected) in closure.buffer_views.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::BufferView,
            selected,
            format_args!("bv{view_index}"),
        )?;
    }
    for (accessor_index, selected) in closure.accessors.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Accessor,
            selected,
            format_args!("acc{accessor_index}"),
        )?;
    }

    for (light_index, selected) in closure.lights.as_slice().iter().copied().enumerate() {
        observe_source_identity(
            ctx,
            &mut builder,
            &mut identity,
            SourceScopeEntityKind::Light,
            selected,
            format_args!("l{light_index}"),
        )?;
    }
    let record = builder.finish(ctx)?;
    ctx.record_source_scope(record)?;
    Ok(())
}

fn observe_source_index<'session>(
    ctx: &mut ParseServices<'_, '_, 'session>,
    builder: &mut SourceScopeRecordBuilder<'session>,
    identity: &mut MeteredString<'session>,
    kind: SourceScopeEntityKind,
    selected: bool,
    prefix: char,
    index: usize,
) -> Result<()> {
    identity.truncate(0);
    identity.try_write_fmt(ctx, format_args!("{prefix}{index}"))?;
    builder.observe(
        ctx,
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

fn observe_source_identity<'session>(
    ctx: &mut ParseServices<'_, '_, 'session>,
    builder: &mut SourceScopeRecordBuilder<'session>,
    identity: &mut MeteredString<'session>,
    kind: SourceScopeEntityKind,
    selected: bool,
    value: fmt::Arguments<'_>,
) -> Result<()> {
    identity.truncate(0);
    identity.try_write_fmt(ctx, value)?;
    builder.observe(
        ctx,
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

fn observe_source_channel<'session>(
    ctx: &mut ParseServices<'_, '_, 'session>,
    builder: &mut SourceScopeRecordBuilder<'session>,
    identity: &mut MeteredString<'session>,
    selected: bool,
    animation_index: usize,
    channel_index: usize,
) -> Result<()> {
    identity.truncate(0);
    identity.try_write_fmt(ctx, format_args!("a{animation_index}c{channel_index}"))?;
    builder.observe(
        ctx,
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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &'document crate::document::Document<'_>,
    options: &GltfImportOptions,
) -> Result<NodeGraph<'document, 'scope>> {
    let selection = options.scene_selection();
    let selection_request = match selection {
        GltfSceneSelection::DocumentDefault => SourceScopeSelectionRequest::DocumentDefault,
        GltfSceneSelection::SceneIndex(scene_index) => SourceScopeSelectionRequest::ExplicitScene {
            scene_index: u64::from(scene_index),
        },
    };
    let node_count = {
        ctx.poll()?;
        let value = gltf.nodes().len();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    {
        let layout = ctx.bind_group([ImportWorkClass::GraphNodes, ImportWorkClass::GraphWork])?;
        ctx.child(
            &layout,
            [to_u64(node_count), to_u64(node_count)],
            |child| -> Result<()> {
                // Borrowing rows removes the handle table, not the observable visit prefixes.
                child.poll()?;
                let mut nodes = gltf.nodes();
                for ordinal in 0..node_count {
                    if ordinal.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
                        child.poll()?;
                    }
                    child.run([1, 1], || Ok::<_, ContentError>(nodes.next()))?;
                }
                if node_count.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
                    child.poll()?;
                }
                Ok(())
            },
        )
    }?;
    let mut children = try_vec_capacity(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        node_count,
        "gltf-node-children",
    )?;
    let mut parents = try_vec_filled(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        node_count,
        None,
        "gltf-node-parents",
    )?;
    let edge_work = ctx.bind_group([ImportWorkClass::GraphEdges, ImportWorkClass::GraphWork])?;
    for (parent_index, node) in gltf.nodes().enumerate() {
        poll_parser_loop(&mut ctx.services()?, parent_index)?;
        let child_count = {
            ctx.poll()?;
            let value = node.children().len();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }?;
        let child_indices = ctx.child(&edge_work, [to_u64(child_count), to_u64(child_count)], |child| -> Result<_> {
                let mut child_indices = child.services()?.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "gltf-node-child-list",
                );
                child_indices.try_reserve(&mut child.services()?, child_count)?;
                child.poll()?;
                let mut node_children = node.children();
                loop {
                    if child_indices.len().is_multiple_of(GEOMETRY_POLL_INTERVAL) {
                        child.poll()?;
                    }
                    let next = node_children.next();
                    let Some(node_child) = next else {
                        break;
                    };
                    child.run([1, 1], || -> Result<()> {
                    if child_indices.len() == child_count {
                        return Err(gltf_parse_error(
                            format_args!(
                                "gltf-node-child-list produced more than {child_count} values"
                            ),
                        ));
                    }
                    let child_index = node_child.index();
                    if child_index >= node_count {
                        return Err(gltf_parse_error(
                            format_args!(
                                "glTF node {parent_index} references missing child {child_index}"
                            ),
                        ));
                    }
                    if child_index == parent_index {
                        return Err(gltf_parse_error(
                            format_args!("glTF node graph contains a cycle at node {parent_index}"),
                        ));
                    }
                    if let Some(existing_parent) = parents.as_slice()[child_index] {
                        return Err(gltf_parse_error(
                            format_args!(
                                "glTF node {child_index} has multiple parents ({existing_parent} and {parent_index})"
                            ),
                        ));
                    }
                    child_indices.try_push_reserved(child_index)?;
                    parents.as_mut_slice()[child_index] = Some(parent_index);
                    Ok(())
                    })?;
                }
                if child_indices.len() != child_count {
                    return Err(gltf_parse_error(
                        format_args!(
                            "gltf-node-child-list produced {} values, expected {child_count}",
                            child_indices.len()
                        ),
                    ));
                }
                Ok(child_indices)
            })?;
        children.try_push_reserved(child_indices)?;
    }

    validate_graph_depth_and_cycles(ctx, children.as_slice(), parents.as_slice())?;
    let (roots, selection_outcome) =
        select_and_validate_scene_roots(ctx, gltf, parents.as_slice(), selection)?;
    let graph_shape = ctx.bind_group([ImportWorkClass::GraphNodes, ImportWorkClass::GraphEdges])?;
    let selected_roots = ctx.child(&graph_shape, [1, to_u64(roots.len())], |child| {
        child.run([1, to_u64(roots.len())], || {
            Ok::<_, ContentError>(roots.as_slice())
        })
    })?;
    let selected = validate_canonical_graph_limits(ctx, children.as_slice(), selected_roots)?;

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
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    children: &[MeteredVec<'_, usize>],
    roots: &[usize],
) -> Result<MeteredVec<'scope, bool>> {
    let mut max_depth = 0usize;
    observe_graph_depth(ctx, &mut max_depth, 1)?;
    let mut selected = try_vec_filled(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        children.len(),
        false,
        "gltf-selected-node-membership",
    )?;

    let mut pending = try_vec_capacity(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        roots.len(),
        "gltf-canonical-graph-stack",
    )?;
    for (root_ordinal, root) in roots.iter().rev().copied().enumerate() {
        poll_parser_loop(&mut ctx.services()?, root_ordinal)?;
        pending.try_push_reserved((root, 2usize))?;
    }

    let mut visited = 0usize;
    while let Some((node_index, depth)) = pending.pop() {
        poll_parser_loop(&mut ctx.services()?, visited)?;
        visited = visited.saturating_add(1);
        observe_graph_depth(ctx, &mut max_depth, depth)?;

        let node_children = children.get(node_index).ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF node graph references missing node {node_index}"
            ))
        })?;
        let selected_node = selected.as_mut_slice().get_mut(node_index).ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF selected node {node_index} is out of range"
            ))
        })?;
        *selected_node = true;
        let child_depth = depth
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-graph-depth-counter"))?;
        pending.try_reserve(&mut ctx.services()?, node_children.len())?;
        for (child_ordinal, child) in node_children.as_slice().iter().rev().copied().enumerate() {
            poll_parser_loop(&mut ctx.services()?, child_ordinal)?;
            pending.try_push_reserved((child, child_depth))?;
        }
    }

    Ok(selected)
}

fn observe_graph_depth(
    ctx: &mut ParseRoot<'_, '_, '_>,
    maximum: &mut usize,
    depth: usize,
) -> Result<()> {
    let additional = depth.saturating_sub(*maximum);
    if additional == 0 {
        return Ok(());
    }
    {
        let layout = ctx.bind_group([ImportWorkClass::GraphDepth])?;
        ctx.child(&layout, [to_u64(additional)], |child| {
            child.run([to_u64(additional)], || {
                *maximum = depth;
                Ok::<_, ContentError>(())
            })
        })
    }?;
    Ok(())
}

fn validate_graph_depth_and_cycles(
    ctx: &mut ParseRoot<'_, '_, '_>,
    children: &[MeteredVec<'_, usize>],
    parents: &[Option<usize>],
) -> Result<()> {
    let mut depths = try_vec_filled(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        parents.len(),
        0usize,
        "gltf-node-depths",
    )?;
    let mut queue = try_vec_capacity(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        parents.len(),
        "gltf-node-depth-queue",
    )?;
    for (node_index, parent) in parents.iter().enumerate() {
        poll_parser_loop(&mut ctx.services()?, node_index)?;
        if parent.is_none() {
            depths.as_mut_slice()[node_index] = 1;
            queue.try_push_reserved(node_index)?;
        }
    }

    let mut cursor = 0usize;
    while cursor < queue.len() {
        poll_parser_loop(&mut ctx.services()?, cursor)?;
        let node_index = queue.as_slice()[cursor];
        cursor += 1;
        let depth = depths.as_slice()[node_index];
        for (child_ordinal, &child_index) in children[node_index].as_slice().iter().enumerate() {
            poll_parser_loop(&mut ctx.services()?, child_ordinal)?;
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-graph-depth-counter"))?;
            depths.as_mut_slice()[child_index] = child_depth;
            queue.try_push_reserved(child_index)?;
        }
    }

    if queue.len() != parents.len() {
        return Err(gltf_parse_error("glTF node graph contains a cycle"));
    }
    Ok(())
}

fn select_and_validate_scene_roots<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    parents: &[Option<usize>],
    selection: GltfSceneSelection,
) -> Result<(MeteredVec<'scope, usize>, NodeSelectionOutcome)> {
    let scene_count = {
        ctx.poll()?;
        let value = gltf.scenes().len();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    let mut scene_roots = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-scene-root-lists",
    );

    let root_work = ctx.bind_group([ImportWorkClass::GraphWork])?;
    for (scene_index, scene) in gltf.scenes().enumerate() {
        poll_parser_loop(&mut ctx.services()?, scene_index)?;
        let roots = ctx.child(&root_work, [1], |child| -> Result<_> {
            scene_roots.try_reserve(&mut child.services()?, 1)?;
            child.poll()?;
            let root_count = child.run([1], || Ok::<_, ContentError>(scene.nodes().len()))?;
            child.replenish(to_u64(root_count))?;
            child.poll()?;
            let roots = scene.nodes();
            collect_exact_with_work(
                child,
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
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        parents.len(),
        usize::MAX,
        "gltf-scene-root-membership",
    )?;
    for (scene_index, roots) in scene_roots.as_slice().iter().enumerate() {
        for (root_ordinal, &root_index) in roots.as_slice().iter().enumerate() {
            poll_parser_loop(&mut ctx.services()?, root_ordinal)?;
            let Some(parent) = parents.get(root_index) else {
                return Err(gltf_parse_error(format_args!(
                    "glTF scene {scene_index} references missing root node {root_index}"
                )));
            };
            if seen_in_scene.as_slice()[root_index] == scene_index {
                return Err(gltf_parse_error(format_args!(
                    "glTF scene {scene_index} contains duplicate root node {root_index}"
                )));
            }
            seen_in_scene.as_mut_slice()[root_index] = scene_index;
            if parent.is_some() {
                return Err(gltf_parse_error(format_args!(
                    "glTF scene {scene_index} root node {root_index} has a parent"
                )));
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
            let default_scene_index = {
                ctx.poll()?;
                let value = gltf.default_scene().map(|scene| scene.index());
                ctx.poll()?;
                Ok::<_, ContentError>(value)
            }?;
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
                gltf_parse_error(format_args!(
                    "glTF selected scene index {scene_index} is out of range"
                ))
            })?;
            let mut roots = try_vec_capacity(
                &mut ctx.services()?,
                ImportStorageClass::FormatNormalization,
                selected.len(),
                "gltf-selected-scene-roots",
            )?;
            for (root_ordinal, root) in selected.as_slice().iter().copied().enumerate() {
                poll_parser_loop(&mut ctx.services()?, root_ordinal)?;
                roots.try_push_reserved(root)?;
            }
            Ok((roots, NodeSelectionOutcome::DeclaredScene(scene_index)))
        }
        NodeSelectionOutcome::ParentlessNodeProjection => {
            let layout = ctx.bind_group([ImportWorkClass::GraphWork])?;
            ctx.child(&layout, [1], |child| -> Result<_> {
                let mut roots = child.run_returned_with_storage(|mut storage| -> std::result::Result<_, baozi_import::ParsePieceError<GltfFailure>> {
                    Ok((storage.metered_vec(
                        ImportStorageClass::FormatNormalization,
                        "gltf-derived-scene-roots",
                    ), [1]))
                })?;
                for (chunk_index, chunk) in parents.chunks(GEOMETRY_POLL_INTERVAL).enumerate() {
                    child.poll()?;
                    let root_count = chunk.iter().filter(|parent| parent.is_none()).count();
                    if root_count == 0 {
                        continue;
                    }
                    child.replenish(to_u64(root_count))?;
                    roots.try_reserve(&mut child.services()?, root_count)?;
                    let base = chunk_index
                        .checked_mul(GEOMETRY_POLL_INTERVAL)
                        .ok_or_else(|| child.error_view().allocation_failure("gltf-derived-root-index"))?;
                    for (offset, parent) in chunk.iter().enumerate() {
                        if parent.is_some() {
                            continue;
                        }
                        let errors = child.error_view();
                        child.run([1], || -> Result<()> {
                        let node_index = base
                            .checked_add(offset)
                            .ok_or_else(|| errors.allocation_failure("gltf-derived-root-index"))?;
                        roots.try_push_reserved(node_index)?;
                        Ok(())
                        })?;
                    }
                }
                Ok((roots, NodeSelectionOutcome::ParentlessNodeProjection))
            })
        }
    }
}

fn build_skins<'region, 'scope>(
    ctx: &mut ParseRoot<'_, 'region, 'scope>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    node_ids: &[Option<NodeLocalId>],
    closure: &ResourceClosure<'_>,
) -> Result<SkinStorage<'scope>> {
    let skin_count = {
        ctx.poll()?;
        let value = gltf.skins().len();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    let selected_skin_count = selected_count(ctx, closure.skins.as_slice())?;

    let mut storage = SkinStorage {
        skins: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::SceneConstruction,
            selected_skin_count,
            "gltf-authoring-skins",
        )?,
        skin_ids: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            skin_count,
            None,
            "gltf-skin-id-map",
        )?,
    };
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    let matrix_work = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (ordinal, skin) in gltf.skins().enumerate() {
        poll_parser_loop(&mut ctx.services()?, ordinal)?;
        let skin_index = skin.index();
        let Some(is_selected) = closure.skins.as_slice().get(skin_index).copied() else {
            return Err(gltf_parse_error(format_args!(
                "glTF skin index {skin_index} is out of range"
            )));
        };
        if !is_selected {
            continue;
        }

        let joint_count = {
            ctx.poll()?;
            let value = skin.joints().len();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }?;
        let joints = {
            ctx.poll()?;
            let value = (|| {
                collect_exact(
                    &mut ctx.services()?,
                    ImportStorageClass::FormatNormalization,
                    joint_count,
                    "gltf-skin-joints",
                    skin.joints(),
                )
            })();
            ctx.poll_result(value)
        }?;
        let mut mapped_joints = try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::SceneConstruction,
            joints.len(),
            "gltf-mapped-skin-joints",
        )?;
        for (joint_ordinal, joint) in joints.as_slice().iter().enumerate() {
            poll_parser_loop(&mut ctx.services()?, joint_ordinal)?;
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

        let skeleton_root = match {
            ctx.poll()?;
            let value = skin.skeleton();
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }? {
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

        let inverse_bind_source = ctx.child(
            &work_group,
            [AccessorSources::COLLECTION_WORK_BOUND],
            |child| {
                child.run([AccessorSources::COLLECTION_WORK_BOUND], || {
                    Ok::<_, ParseFailure>(skin.inverse_bind_matrices())
                })
            },
        )?;
        let provenance = inverse_bind_source
            .map(|accessor| accessor_provenance(&mut ctx.services()?, buffers, accessor))
            .transpose()?
            .unwrap_or(AccessorSources::ROOT);
        let inverse_bind_matrices =
            collect_inverse_bind_matrices(ctx, &matrix_work, buffers, &skin, mapped_joints.len())?;

        let name_bytes = skin.name().map_or(0, str::len);
        let source_name = if name_bytes == 0 {
            skin.name()
        } else {
            let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
            ctx.child(&layout, [to_u64(name_bytes)], |child| {
                child.run([to_u64(name_bytes)], || Ok::<_, ContentError>(skin.name()))
            })?
        };
        let name = owned_name(ctx, source_name, "gltf-normalized-skin-name")?;
        let local_id = SkinLocalId::new(authoring_u32(
            &mut ctx.services()?,
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
            return Err(gltf_parse_error(format_args!(
                "glTF skin index {skin_index} is out of range"
            )));
        };
        *slot = Some(local_id);
    }
    Ok(storage)
}

fn collect_inverse_bind_matrices<'region, 'scope>(
    ctx: &mut ParseRoot<'_, 'region, 'scope>,
    matrix_work: &ParseWorkLayout<'region, 1>,
    buffers: &[Option<BufferData<'_>>],
    skin: &crate::document::Skin<'_>,
    joint_count: usize,
) -> Result<MeteredVec<'scope, Mat4>> {
    let scalar_count = to_u64(joint_count)
        .checked_mul(16)
        .ok_or_else(|| ctx.allocation_failure("gltf-inverse-bind-work"))?;
    ctx.child(matrix_work, [scalar_count], |child| -> Result<_> {
        let mut collected = try_vec_capacity(
            &mut child.services()?,
            ImportStorageClass::SceneConstruction,
            joint_count,
            "gltf-inverse-bind-matrices",
        )?;
        if skin.inverse_bind_matrices().is_some() {
            let source = skin
                .inverse_bind_matrices()
                .ok_or_else(|| GltfFailure::internal("inverse bind accessor disappeared"))?;
            let matrices = accessor::bind(&mut child.services()?, source, buffers)?
                .matrices()
                .ok_or_else(|| {
                    GltfFailure::invalid_accessor(
                        Some(source.index()),
                        "invalid inverse bind matrix shape",
                    )
                })?;
            {
                child.poll()?;
                let value = (|| -> Result<()> {
                    for (row, cols) in matrices.enumerate() {
                        poll_parser_loop(&mut child.services()?, row)?;
                        child.run([16], || -> Result<()> {
                            if collected.len() == joint_count {
                                return Err(gltf_parse_error(
                                    "glTF inverse bind matrix reader exceeded the joint count",
                                ));
                            }
                            let matrix = Mat4 { cols };
                            if !matrix.is_finite() || !matrix.is_affine() {
                                return Err(gltf_parse_error(format_args!(
                                    "glTF skin {} inverse bind matrix {} must be finite and affine",
                                    skin.index(),
                                    collected.len(),
                                )));
                            }
                            collected.try_push_reserved(matrix)?;
                            Ok(())
                        })?;
                    }
                    Ok(())
                })();
                child.poll_result(value)
            }?;
        } else {
            for joint in 0..joint_count {
                if joint.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
                    child.poll()?;
                }
                child.run([16], || collected.try_push_reserved(Mat4::IDENTITY))?;
            }
        }
        if collected.len() != joint_count {
            return Err(gltf_parse_error(
                "glTF inverse bind matrix count does not match joint count",
            ));
        }
        Ok(collected)
    })
}

fn build_mesh_instance_deformations<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    mesh_instances: &[MeshInstanceStorage],
    skin_ids: &[Option<SkinLocalId>],
) -> Result<MeteredVec<'scope, InstanceDeformationStorage<'scope>>> {
    let mut deformations = try_vec_capacity(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        mesh_instances.len(),
        "gltf-normalized-instance-deformations",
    )?;
    let weight_work = ctx.bind_group([ImportWorkClass::WorkUnits, ImportWorkClass::MorphCells])?;
    for (ordinal, instance) in mesh_instances.iter().enumerate() {
        poll_parser_loop(&mut ctx.services()?, ordinal)?;
        let skin = match instance.skin_index {
            Some(index) => skin_ids
                .get(index)
                .copied()
                .flatten()
                .map(Presence::Present)
                .ok_or_else(|| {
                    gltf_parse_error(format_args!("glTF skin {index} was not imported"))
                })?,
            None => Presence::Absent,
        };
        let weights = gltf
            .tables()
            .nodes
            .get(instance.source_node_index)
            .ok_or_else(|| gltf_parse_error("glTF morph instance node disappeared"))?
            .weights
            .as_deref();
        let weights = weights
            .map(|weights| -> Result<_> {
                ctx.child(
                    &weight_work,
                    [to_u64(weights.len()), to_u64(weights.len())],
                    |child| -> Result<_> {
                        let mut owned = try_vec_capacity(
                            &mut child.services()?,
                            ImportStorageClass::FormatNormalization,
                            weights.len(),
                            "gltf-normalized-instance-weights",
                        )?;
                        for (index, &weight) in weights.iter().enumerate() {
                            poll_parser_loop(&mut child.services()?, index)?;
                            child.run([1, 1], || owned.try_push_reserved(weight))?;
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
    ctx: &mut ParseServices<'_, '_, '_>,
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
    ctx: &mut ParseRoot<'_, '_, '_>,
    gltf: &crate::document::Document<'_>,
    closure: &ResourceClosure<'_>,
) -> Result<()> {
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (skin_index, skin) in gltf.tables().skins.iter().enumerate() {
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, ParseFailure>(closure.skins.as_slice()[skin_index])
            })
        })?;
        poll_parser_loop(&mut ctx.services()?, skin_index)?;
        if selected {
            validate_skin_inverse_bind_accessor(
                &mut ctx.services()?,
                gltf,
                skin_index,
                skin.joints.len(),
            )?;
        }
    }
    Ok(())
}

fn build_cameras<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    closure: &ResourceClosure<'_>,
) -> Result<CameraStorage<'scope>> {
    let camera_count = {
        ctx.poll()?;
        let value = gltf.cameras().count();
        ctx.poll()?;
        Ok::<_, ContentError>(value)
    }?;
    let selected_camera_count = selected_count(ctx, closure.cameras.as_slice())?;
    let mut storage = CameraStorage {
        authored: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::SceneConstruction,
            selected_camera_count,
            "gltf-authoring-cameras",
        )?,
        names: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::SceneConstruction,
            selected_camera_count,
            "gltf-authoring-camera-names",
        )?,
        camera_ids: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            camera_count,
            None,
            "gltf-camera-id-map",
        )?,
    };
    for (ordinal, camera) in gltf.cameras().enumerate() {
        poll_parser_loop(&mut ctx.services()?, ordinal)?;
        if !closure
            .cameras
            .as_slice()
            .get(ordinal)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let projection = {
            ctx.poll()?;
            let value = match camera.projection() {
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
            };
            ctx.poll()?;
            Ok::<_, ContentError>(value)
        }?
        .map_err(|_| {
            gltf_parse_error(format_args!("glTF camera {ordinal} projection is invalid"))
        })?;
        let name = match camera.name() {
            Some(name) if name.len() > ctx.services()?.limits().max_string_bytes => {
                push_warning(
                    &mut ctx.services()?,
                    "gltf.camera_name_limit",
                    "camera name exceeded max_string_bytes and was ignored",
                )?;
                None
            }
            name => name,
        };
        let name = match name {
            Some(name) if !name.is_empty() => {
                let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
                ctx.child(&layout, [to_u64(name.len())], |child| {
                    child.run([to_u64(name.len())], || {
                        Ok::<_, ContentError>(camera.name())
                    })
                })?
            }
            name => name,
        };
        let local_id = CameraLocalId::new(authoring_u32(
            &mut ctx.services()?,
            storage.authored.len(),
            "gltf-authoring-camera-id",
        )?);
        storage.authored.try_push_reserved(projection)?;
        storage
            .names
            .try_push_reserved(owned_name(ctx, name, "gltf-normalized-camera-name")?)?;
        let Some(slot) = storage.camera_ids.as_mut_slice().get_mut(ordinal) else {
            return Err(gltf_parse_error("glTF camera id map is incomplete"));
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
    ctx: &mut ParseServices<'_, '_, '_>,
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
        DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor,
        FormatSelection, ImportConfigOverlay, ImportDimension, ImportFailure,
        ImportOperationControl, ImportRequest, ImportRequestLayer, OptionCompileLimits, Probe,
        ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder, ResourceLimits,
        ResourceLimitsOverlay,
    };
    use baozi_io::{AssetPath, MemoryAssetIo};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    type Inspect = fn(&mut ParseRoot<'_, '_, '_>, usize, bool) -> Result<()>;

    struct InspectWork {
        inspect: Inspect,
        count: usize,
        exercise: bool,
        completed: Arc<AtomicBool>,
    }

    impl Format for InspectWork {
        type Options = baozi_import::NoFormatOptions;

        fn compile_defaults(
            &self,
            scope: &mut baozi_import::RegistryBuildScope<'_>,
        ) -> std::result::Result<
            baozi_import::CompiledOptions<Self::Options>,
            baozi_import::OptionCompileError,
        > {
            Self::Options::compile_defaults(scope)
        }

        fn descriptor(&self) -> FormatDescriptor {
            crate::format_descriptor()
        }

        fn detect(
            &self,
            _: &Probe<'_>,
            _: &ReadHint,
            _: &mut DetectionScope<'_, '_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(
            &self,
            _: &Self::Options,
            context: &mut baozi_import::ParseContext<'_, '_>,
        ) -> ParseResult<()> {
            context.with_execution(|scope| {
                (self.inspect)(scope, self.count, self.exercise).map_err(|error| match error {
                    ContentError::Scope(failure) => failure,
                    error => scope.format_failure(error.into_format_failure()),
                })?;
                self.completed.store(true, Ordering::SeqCst);
                Err(scope.format_failure(GltfFailure::internal(
                    "provenance work inspection completed",
                )))
            })
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
        let request = registry.freeze(RegistryBuildLimits::default())?.plan(
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
        let result = request.read_asset(
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

    fn inspect_images(
        scope: &mut ParseRoot<'_, '_, '_>,
        count: usize,
        exercise: bool,
    ) -> Result<()> {
        let mut values = scope.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "image-lookup-fixture",
        );
        values.try_reserve(&mut scope.services()?, count)?;
        let origin = scope.services()?.root_origin();
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

    fn inspect_unique_sources(
        scope: &mut ParseRoot<'_, '_, '_>,
        count: usize,
        exercise: bool,
    ) -> Result<()> {
        let mut pending = scope.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "pending-provenance-fixture",
        );
        let mut values = scope.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "retained-provenance-fixture",
        );
        pending.try_reserve(&mut scope.services()?, 2 * count)?;
        values.try_reserve(&mut scope.services()?, count + 2)?;
        values.try_push_reserved(91)?;
        values.try_push_reserved(72)?;
        for source in (0..count).rev() {
            pending.try_push_reserved(u32::try_from(source).unwrap())?;
            pending.try_push_reserved(u32::try_from(source).unwrap())?;
        }
        let capacity = pending.capacity();
        if exercise {
            if let Err(failure) =
                try_extend_provenance_unique(&mut scope.children()?, &mut values, &mut pending)
            {
                assert_eq!(values.as_slice(), &[91, 72]);
                return Err(failure);
            }
            assert_eq!(&values.as_slice()[..2], &[91, 72]);
            assert_eq!(values.len(), count + 2);
            for (index, &source) in values.as_slice()[2..].iter().enumerate() {
                assert_eq!(source, u32::try_from(index).unwrap());
            }
            assert!(pending.is_empty());
            assert_eq!(pending.capacity(), capacity);
            // Reusing the scratch must append another owner without merging with the prefix.
            values.try_reserve(&mut scope.services()?, 2)?;
            pending.try_extend_from_slice(&mut scope.services()?, &[7, 2, 7])?;
            try_extend_provenance_unique(&mut scope.children()?, &mut values, &mut pending)?;
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
    fn provenance_work_limit_preserves_the_unmodified_owner_prefix() -> TestResult<()> {
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
        assert_eq!(consumed_work(&failure), baseline_work + 1);
        Ok(())
    }
}
