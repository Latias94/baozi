use crate::admission::AdmittedSurfaces;
use crate::{GltfFailure, admission, external_buffer_role, external_image_role};
use baozi_import::authoring::{
    AlphaMode, AttributeSemantic, AuthoredTransform, AuthoringRange, CameraAuthoring,
    CameraLocalId, CameraProjection, Color, ColorInterpretation, ExtensionMediaType,
    GeometryMeshInstanceAuthoring, GeometryTopologyAuthoring, ImageLocalId, InfluenceAuthoring,
    LogicalMeshLocalId, Mat4, MaterialFactors, MaterialLocalId, MipmapFilter, NodeAuthoringNode,
    NodeLocalId, NodeParent, PbrMetallicRoughnessFactors, Presence, Quat, ResourceIdentityLocalId,
    SamplerAuthoring, SamplerLocalId, SamplerState, SkinLocalId, TexelFilter, TextureAddressMode,
    TextureAuthoring, TextureBlendOperation, TextureChannel, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureLocalId, TextureRole, TextureSourceKey,
    TextureStrength, TextureTransform, Vec2, Vec3,
};
use baozi_import::{
    DiagnosticCode, DiagnosticSeverity, ImportStorageClass, ImportWorkClass, ImportWorkScope,
    MeteredOrderedMap, MeteredString, MeteredVec, ParseFailure, ParseResult, ParseScope,
    SceneDraft, SourceScopeDisposition, SourceScopeEntityKind, SourceScopeRecordBuilder,
    SourceScopeSelectionOutcome,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::{DecodeError, DecodeSliceError, Engine as _};
use gltf::buffer::Source as BufferSource;
use gltf::image::Source as ImageSource;
use gltf::json;
use gltf::json::validation::Checked;
use gltf::mesh::{Mode, Semantic};
use gltf::texture::{MagFilter, MinFilter, WrappingMode};
use serde::Deserialize as _;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

mod animation;
mod model;
mod morph;
use model::*;
mod normalized;
mod source;
mod source_profile;
mod uri;

type Result<T> = std::result::Result<T, ContentError>;

const GLTF_BACKEND_FIXED_ALLOWANCE: u64 = 64 * 1024;
const GLTF_JSON_EXPANSION_ALLOWANCE: u64 = 64;
const PARSER_POLL_INTERVAL: usize = 4_096;

const KHR_TEXTURE_TRANSFORM: &str = "KHR_texture_transform";

// Only schemas with a bounded typed Baozi mapping belong in this list.
const BAOZI_SUPPORTED_EXTENSION_NAMES: &[&str] = &[KHR_TEXTURE_TRANSFORM];

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
    texture_binding_contracts: TextureBindingContracts<'scope>,
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
/// The decoder's temporary frames and backend allowance end at this boundary.
fn read_normalized_gltf<'scope>(ctx: &'scope ParseScope<'_>) -> Result<NormalizedGltf<'scope>> {
    let surface = admission::admit(ctx)?;
    let primary_len = to_u64(ctx.primary_bytes().len());
    let (json, _document_allowance) =
        ctx.with_parent_work(ImportWorkClass::DecodeWork, primary_len, || -> Result<_> {
            let backend_allowance = gltf_backend_allocation_allowance(ctx, &surface)?;
            let document_allowance =
                ctx.metered_opaque_allowance(ImportStorageClass::BackendDecode, backend_allowance)?;
            preflight_asset_versions(ctx, surface.json_bytes())?;
            let json = safe_gltf(ctx, "glTF JSON parse", || {
                gltf::json::Root::from_slice(surface.json_bytes())
            })?
            .map_err(|error| {
                GltfFailure::invalid_document(format_args!("invalid glTF JSON: {error}"))
            })?;
            Ok((json, document_allowance))
        })?;
    preflight_extension_declarations(ctx, &json)?;
    let gltf = safe_gltf(ctx, "glTF document parse", || {
        gltf::Document::from_json(json)
    })?
    .map_err(|error| GltfFailure::invalid_document(format_args!("invalid glTF: {error}")))?;
    let node_graph = preflight_node_graph(ctx, &gltf)?;
    let animation_contracts = animation::preflight(ctx, &gltf, node_graph.selected.as_slice())?;
    let resource_closure = build_resource_closure(ctx, &gltf, &node_graph, &animation_contracts)?;
    source::preflight(
        ctx,
        surface.source_map(),
        &gltf,
        &node_graph,
        &resource_closure,
        &animation_contracts,
    )?;
    let texture_binding_contracts =
        preflight_texture_binding_contracts(ctx, &gltf, &resource_closure)?;
    let repeated_mesh_string_bytes =
        preflight_canonical_base_strings(ctx, &gltf, &node_graph, &resource_closure)?;
    let resolved_texture_uris =
        preflight_texture_strings(ctx, &gltf, repeated_mesh_string_bytes, &resource_closure)?;
    let material_count = safe_gltf(ctx, "glTF material count", || gltf.materials().count())?;
    let geometry_contracts =
        preflight_geometry_contracts(ctx, &gltf, material_count, &resource_closure)?;
    let morph_contracts = morph::preflight(
        ctx,
        &gltf,
        &resource_closure,
        &geometry_contracts,
        node_graph.selected.as_slice(),
    )?;
    let prepared = PreparedDocument {
        node_graph,
        resource_closure,
        texture_binding_contracts,
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
    let buffers = load_buffers(ctx, &gltf, surface.bin_bytes(), &prepared.resource_closure)?;
    let normalized = normalize_document(ctx, &gltf, buffers, &prepared, surface.into_source_map())?;
    drop(prepared);
    drop(gltf);
    drop(_document_allowance);
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

struct VersionHeader {
    asset: AssetVersionHeader,
}

struct AssetVersionHeader {
    version: String,
    min_version: Option<String>,
}

impl<'de> serde::Deserialize<'de> for VersionHeader {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct HeaderVisitor;

        impl<'de> serde::de::Visitor<'de> for HeaderVisitor {
            type Value = VersionHeader;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a glTF document object")
            }

            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut asset = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "asset" {
                        asset = Some(map.next_value()?);
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                let asset =
                    asset.ok_or_else(|| <A::Error as serde::de::Error>::missing_field("asset"))?;
                Ok(VersionHeader { asset })
            }
        }

        deserializer.deserialize_map(HeaderVisitor)
    }
}

impl<'de> serde::Deserialize<'de> for AssetVersionHeader {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AssetVisitor;

        impl<'de> serde::de::Visitor<'de> for AssetVisitor {
            type Value = AssetVersionHeader;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an asset object containing version metadata")
            }

            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut version = None;
                let mut min_version = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "version" => {
                            version = Some(map.next_value::<String>().map_err(|error| {
                                <A::Error as serde::de::Error>::custom(format_args!(
                                    "asset.version: {error}"
                                ))
                            })?)
                        }
                        // Optional means absent, not an explicit JSON null.
                        "minVersion" => {
                            min_version = Some(map.next_value::<String>().map_err(|error| {
                                <A::Error as serde::de::Error>::custom(format_args!(
                                    "asset.minVersion: {error}"
                                ))
                            })?)
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                let version = version.ok_or_else(|| {
                    <A::Error as serde::de::Error>::custom("asset.version is required")
                })?;
                Ok(AssetVersionHeader {
                    version,
                    min_version,
                })
            }
        }

        deserializer.deserialize_map(AssetVisitor)
    }
}

fn preflight_asset_versions(ctx: &ParseScope<'_>, bytes: &[u8]) -> Result<()> {
    // This schema pass retains only version metadata and skips all other values. Its
    // validation scan is separate from the backend's existing DecodeWork admission.
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        to_u64(bytes.len()),
        |work| -> Result<()> {
            // Unlike Serde's derived struct visitors, these visitors require JSON objects.
            let header = safe_gltf(ctx, "glTF version metadata parse", || {
                serde_json::from_slice::<VersionHeader>(bytes)
            })?
            .map_err(|error| {
                GltfFailure::invalid_document(format_args!("glTF asset metadata: {error}"))
            })?;
            let version = parse_asset_version(&header.asset.version).ok_or_else(|| {
                GltfFailure::invalid_document(
                    "asset.version must be two decimal components without leading zeros",
                )
            })?;
            let min_version = header
                .asset
                .min_version
                .as_deref()
                .map(|value| {
                    parse_asset_version(value).ok_or_else(|| {
                        GltfFailure::invalid_document(
                            "asset.minVersion must be two decimal components without leading zeros",
                        )
                    })
                })
                .transpose()?;
            work.complete_all()?;
            if min_version.is_some_and(|minimum| minimum > version) {
                return Err(GltfFailure::invalid_document(
                    "asset.minVersion must not exceed asset.version",
                )
                .into());
            }
            if version.0 != 2 {
                return Err(GltfFailure::unsupported(
                    "asset.version",
                    "asset.version major version is outside the glTF 2.x profile",
                )
                .into());
            }
            if min_version.is_some_and(|minimum| minimum != (2, 0)) {
                return Err(GltfFailure::unsupported(
                    "asset.minVersion",
                    "asset.minVersion requires an unsupported glTF version; supported: 2.0",
                )
                .into());
            }
            Ok(())
        },
    )
}

fn parse_asset_version(value: &str) -> Option<(u32, u32)> {
    fn component(value: &str) -> Option<u32> {
        if value.is_empty() || value.len() > 9 || (value.len() > 1 && value.starts_with('0')) {
            return None;
        }
        value.bytes().try_fold(0, |number, byte| {
            byte.is_ascii_digit()
                .then(|| number * 10 + u32::from(byte - b'0'))
        })
    }

    let (major, minor) = value.split_once('.')?;
    Some((component(major)?, component(minor)?))
}

fn gltf_backend_allocation_allowance(
    ctx: &ParseScope<'_>,
    surface: &AdmittedSurfaces<'_>,
) -> Result<u64> {
    let json_bytes = u64::try_from(surface.json_bytes().len())
        .map_err(|_| ctx.allocation_failure("gltf-backend-json-bytes"))?;

    // The gltf/serde object graph is opaque to the import ledger. The GLB BIN
    // payload stays in the admission-owned input snapshot and is not copied by
    // the backend, so only the borrowed JSON surface needs an opaque allowance.
    json_bytes
        .checked_mul(GLTF_JSON_EXPANSION_ALLOWANCE)
        .and_then(|expanded| expanded.checked_add(GLTF_BACKEND_FIXED_ALLOWANCE))
        .ok_or_else(|| {
            ctx.allocation_failure("gltf-backend-allocation-allowance")
                .into()
        })
}

fn preflight_extension_declarations(ctx: &ParseScope<'_>, root: &json::Root) -> Result<()> {
    let used_names = sorted_extension_names(
        ctx,
        "extensionsUsed",
        &root.extensions_used,
        "gltf-extension-used-names",
    )?;
    let required_names = sorted_extension_names(
        ctx,
        "extensionsRequired",
        &root.extensions_required,
        "gltf-extension-required-names",
    )?;

    let validation_work = to_u64(
        root.extensions_required
            .len()
            .saturating_add(root.extensions_used.len()),
    );
    if validation_work == 0 {
        return Ok(());
    }
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        validation_work,
        |work| -> Result<()> {
            for name in required_names.as_slice() {
                work.poll()?;
                let missing = used_names.as_slice().binary_search(name).is_err();
                let unsupported = !is_supported_extension(name);
                work.complete([1])?;
                if missing {
                    return Err(gltf_parse_error(
                        ctx,
                        format!("required extension {name:?} missing from extensionsUsed"),
                    ));
                }
                if unsupported {
                    return Err(ContentError::from(GltfFailure::unsupported(
                        "required-extension",
                        format_args!("glTF required extension {name:?} is not supported"),
                    )));
                }
            }

            for name in &root.extensions_used {
                work.poll()?;
                if !is_supported_extension(name) {
                    push_warning(
                        ctx,
                        "gltf.extension_ignored",
                        format_args!(
                            "glTF optional extension {name:?} is not interpreted; its payload is omitted"
                        ),
                    )?;
                }
                work.complete([1])?;
            }
            Ok(())
        },
    )
}

fn sorted_extension_names<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    field: &'static str,
    names: &'document [String],
    allocation: &'static str,
) -> Result<MeteredVec<'scope, &'document str>> {
    let mut sorted = ctx.metered_vec(ImportStorageClass::FormatNormalization, allocation);
    sorted.try_reserve(names.len())?;
    if names.is_empty() {
        return Ok(sorted);
    }

    let sort_work = extension_sort_work(names.len());
    let total_work = to_u64(names.len())
        .saturating_mul(2)
        .saturating_add(sort_work);
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        total_work,
        |work| -> Result<()> {
            for (index, name) in names.iter().enumerate() {
                work.poll()?;
                sorted.try_push_reserved(name.as_str())?;
                work.complete([1])?;
                if name.is_empty() {
                    return Err(gltf_parse_error(
                        ctx,
                        format!("glTF {field} entry {index} has an empty extension name"),
                    ));
                }
            }

            if sort_work != 0 {
                let mut comparisons = 0usize;
                sorted.try_sort_unstable_by(|left, right| -> Result<_> {
                    comparisons = comparisons.saturating_add(1);
                    if comparisons.is_multiple_of(PARSER_POLL_INTERVAL) {
                        work.poll()?;
                    }
                    Ok(left.cmp(right))
                })?;
                work.complete([sort_work])?;
            }

            for pair in sorted.as_slice().windows(2) {
                work.poll()?;
                work.complete([1])?;
                if pair[0] == pair[1] {
                    return Err(gltf_parse_error(
                        ctx,
                        format!("glTF {field} contains duplicate extension {:?}", pair[1]),
                    ));
                }
            }
            work.complete_all()?;
            Ok(())
        },
    )?;
    Ok(sorted)
}

fn extension_sort_work(name_count: usize) -> u64 {
    if name_count <= 1 {
        return 0;
    }
    let count = to_u64(name_count);
    let levels = u64::from(usize::BITS - (name_count - 1).leading_zeros());
    count.saturating_mul(levels.saturating_mul(4).saturating_add(4))
}

fn is_supported_extension(name: &str) -> bool {
    BAOZI_SUPPORTED_EXTENSION_NAMES.contains(&name)
}

#[derive(Debug)]
struct TextureBindingContract {
    coordinate_source: TextureCoordinateSource,
    transform: TextureTransform,
}

#[derive(Debug)]
struct MaterialTextureContracts {
    base_color: Option<TextureBindingContract>,
    metallic_roughness: Option<TextureBindingContract>,
    normal: Option<TextureBindingContract>,
    occlusion: Option<TextureBindingContract>,
    emissive: Option<TextureBindingContract>,
}

struct TextureBindingContracts<'scope> {
    materials: MeteredOrderedMap<'scope, usize, MaterialTextureContracts>,
}

impl TextureBindingContracts<'_> {
    fn material(
        &self,
        ctx: &ParseScope<'_>,
        material_index: usize,
    ) -> Result<&MaterialTextureContracts> {
        self.materials.get(&material_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF texture binding contract is missing material {material_index}"),
            )
        })
    }
}

fn preflight_texture_binding_contracts<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Document,
    closure: &ResourceClosure<'_>,
) -> Result<TextureBindingContracts<'scope>> {
    let root = gltf.as_json();
    let extension_declared = root
        .extensions_used
        .iter()
        .any(|name| name == KHR_TEXTURE_TRANSFORM);
    let mut materials = ctx.metered_ordered_map(
        ImportStorageClass::FormatNormalization,
        "gltf-texture-binding-contracts",
    );

    for (material_index, material) in root.materials.iter().enumerate() {
        poll_parser_loop(ctx, material_index)?;
        ctx.charge_work(1)?;
        let selected = closure
            .materials
            .as_slice()
            .get(material_index)
            .copied()
            .ok_or_else(|| {
                ContentError::from(GltfFailure::internal(
                    "glTF material closure length changed during texture preflight",
                ))
            })?;
        if !selected {
            continue;
        }
        let pbr = &material.pbr_metallic_roughness;
        let contracts = MaterialTextureContracts {
            base_color: pbr
                .base_color_texture
                .as_ref()
                .map(|info| {
                    compile_texture_info_contract(
                        ctx,
                        material_index,
                        "baseColorTexture",
                        info,
                        extension_declared,
                    )
                })
                .transpose()?,
            metallic_roughness: pbr
                .metallic_roughness_texture
                .as_ref()
                .map(|info| {
                    compile_texture_info_contract(
                        ctx,
                        material_index,
                        "metallicRoughnessTexture",
                        info,
                        extension_declared,
                    )
                })
                .transpose()?,
            normal: material
                .normal_texture
                .as_ref()
                .map(|info| {
                    compile_material_texture_contract(
                        ctx,
                        material_index,
                        "normalTexture",
                        info.tex_coord,
                        info.extensions
                            .as_ref()
                            .and_then(|extensions| extensions.others.get(KHR_TEXTURE_TRANSFORM)),
                        extension_declared,
                    )
                })
                .transpose()?,
            occlusion: material
                .occlusion_texture
                .as_ref()
                .map(|info| {
                    compile_material_texture_contract(
                        ctx,
                        material_index,
                        "occlusionTexture",
                        info.tex_coord,
                        info.extensions
                            .as_ref()
                            .and_then(|extensions| extensions.others.get(KHR_TEXTURE_TRANSFORM)),
                        extension_declared,
                    )
                })
                .transpose()?,
            emissive: material
                .emissive_texture
                .as_ref()
                .map(|info| {
                    compile_texture_info_contract(
                        ctx,
                        material_index,
                        "emissiveTexture",
                        info,
                        extension_declared,
                    )
                })
                .transpose()?,
        };
        if materials.try_insert(material_index, contracts)?.is_some() {
            return Err(ContentError::from(GltfFailure::internal(
                "glTF texture binding contract material index was inserted twice",
            )));
        }
    }

    Ok(TextureBindingContracts { materials })
}

fn compile_texture_info_contract(
    ctx: &ParseScope<'_>,
    material_index: usize,
    source_key: &'static str,
    info: &json::texture::Info,
    extension_declared: bool,
) -> Result<TextureBindingContract> {
    let transform = info
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.texture_transform.as_ref());
    require_texture_transform_declaration(
        ctx,
        material_index,
        source_key,
        transform.is_some(),
        extension_declared,
    )?;
    compile_texture_binding_contract(ctx, material_index, source_key, info.tex_coord, transform)
}

fn compile_material_texture_contract(
    ctx: &ParseScope<'_>,
    material_index: usize,
    source_key: &'static str,
    tex_coord: u32,
    extension_value: Option<&serde_json::Value>,
    extension_declared: bool,
) -> Result<TextureBindingContract> {
    require_texture_transform_declaration(
        ctx,
        material_index,
        source_key,
        extension_value.is_some(),
        extension_declared,
    )?;
    let transform = extension_value
        .map(|value| {
            json::extensions::texture::TextureTransform::deserialize(value).map_err(|error| {
                gltf_parse_error(
                    ctx,
                    format!(
                        "glTF material {material_index} {source_key} has invalid {KHR_TEXTURE_TRANSFORM}: {error}"
                    ),
                )
            })
        })
        .transpose()?;
    compile_texture_binding_contract(
        ctx,
        material_index,
        source_key,
        tex_coord,
        transform.as_ref(),
    )
}

fn require_texture_transform_declaration(
    ctx: &ParseScope<'_>,
    material_index: usize,
    source_key: &'static str,
    has_transform: bool,
    extension_declared: bool,
) -> Result<()> {
    if has_transform && !extension_declared {
        return Err(gltf_parse_error(
            ctx,
            format!(
                "glTF material {material_index} {source_key} uses {KHR_TEXTURE_TRANSFORM} without declaring it in extensionsUsed"
            ),
        ));
    }
    Ok(())
}

fn compile_texture_binding_contract(
    ctx: &ParseScope<'_>,
    material_index: usize,
    source_key: &'static str,
    tex_coord: u32,
    transform: Option<&json::extensions::texture::TextureTransform>,
) -> Result<TextureBindingContract> {
    ctx.charge_work(1)?;
    let (tex_coord, transform) = match transform {
        Some(transform) => {
            let offset = transform.offset.0;
            let scale = transform.scale.0;
            let canonical = TextureTransform::try_new(
                Vec2::new(offset[0], offset[1]),
                transform.rotation.0,
                Vec2::new(scale[0], scale[1]),
            )
            .map_err(|error| {
                gltf_parse_error(
                    ctx,
                    format!(
                        "glTF material {material_index} {source_key} has invalid {KHR_TEXTURE_TRANSFORM}: {error}"
                    ),
                )
            })?;
            (transform.tex_coord.unwrap_or(tex_coord), canonical)
        }
        None => (tex_coord, TextureTransform::IDENTITY),
    };
    let coordinate_source = TextureCoordinateSource::uv_set(tex_coord).map_err(|_| {
        gltf_parse_error(
            ctx,
            format!(
                "glTF material {material_index} {source_key} texture coordinate set {tex_coord} is out of range"
            ),
        )
    })?;
    Ok(TextureBindingContract {
        coordinate_source,
        transform,
    })
}

fn preflight_canonical_base_strings(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Document,
    graph: &NodeGraph<'_, '_>,
    closure: &ResourceClosure<'_>,
) -> Result<usize> {
    safe_gltf(ctx, "glTF canonical string preflight", || {
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
                let primitive_count =
                    safe_gltf(ctx, "glTF primitive count", || mesh.primitives().len())?;
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
            let Some(node) = graph.nodes.as_slice().get(node_index) else {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF node graph references missing node {node_index}"),
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
    gltf: &gltf::Document,
    repeated_mesh_string_bytes: usize,
    closure: &ResourceClosure<'_>,
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
            format!("glTF texture references missing image {image_index}"),
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
                            format!("glTF texture resolution plan is missing image {image_index}"),
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
    if let Some(name) = texture_name.or(image_name) {
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
    accessor: gltf::Accessor<'_>,
) -> Result<u32> {
    let view = accessor.view().ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format!(
                "glTF accessor {} has no bufferView after provenance preflight",
                accessor.index()
            ),
        )
    })?;
    let buffer_index = view.buffer().index();
    buffers
        .get(buffer_index)
        .and_then(Option::as_ref)
        .map(BufferData::provenance)
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!(
                    "glTF accessor {} references unavailable buffer {buffer_index}",
                    accessor.index()
                ),
            )
        })
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
    gltf: &gltf::Document,
    glb_bin_bytes: Option<&[u8]>,
    closure: &ResourceClosure<'_>,
) -> Result<BufferSet<'scope>> {
    let buffer_count = safe_gltf(ctx, "glTF buffer count", || gltf.buffers().count())?;
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
                        ctx.source().to_string(),
                        None,
                        format!("glTF buffer {} requires a GLB BIN chunk", buffer.index()),
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
                ctx.source().to_string(),
                None,
                format!(
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
            format!("glTF {subject} {subject_index} has an invalid data URI"),
        ));
    };
    let Some((metadata, encoded)) = payload.split_once(',') else {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF {subject} {subject_index} data URI has no comma separator"),
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
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    BASE64_STANDARD.decode_slice(chunk, output)
                }));
                let chunk_written = finish_gltf_operation("glTF base64 buffer decode", outcome)?
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
                    format!(
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
                    format!("glTF {subject} {subject_index} data URI base64 contains whitespace"),
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
            format!("glTF {subject} {subject_index} data URI base64 length is not padded"),
        ));
    }
    if invalid_padding {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF {subject} {subject_index} data URI base64 padding is invalid"),
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
            ctx.source().to_string(),
            None,
            format!(
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
            ctx.source().to_string(),
            None,
            format!(
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

fn gltf_parse_error(ctx: &ParseScope<'_>, message: impl Into<String>) -> ContentError {
    ContentError::parse(ctx.source().to_string(), None, message)
}

fn normalize_document<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Document,
    buffers: BufferSet<'scope>,
    prepared: &PreparedDocument<'_, '_>,
    source_map: crate::source_map::GltfSourceMap<'scope>,
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
    let appearance_storage = build_appearance_storage(
        ctx,
        gltf,
        buffers.as_slice(),
        &prepared.resolved_texture_uris,
        &external_images,
        &prepared.texture_binding_contracts,
        &prepared.resource_closure,
    )?;
    let geometry_storage = build_geometry_storage(
        ctx,
        gltf,
        buffers.as_slice(),
        &prepared.geometry_contracts,
        &appearance_storage.material_ids,
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
    let sources = source_profile::SelectedSourceProfile::from_source_map(ctx, &source_map, |id| {
        source::selected(
            id,
            gltf,
            &prepared.node_graph,
            &prepared.resource_closure,
            &prepared.animation_contracts,
        )
    })?;
    let mut normalized = NormalizedGltf {
        sources,
        appearance: appearance_storage,
        geometry: geometry_storage,
        cameras,
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
    gltf: &gltf::Document,
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
    gltf: &gltf::Document,
    buffers: &[Option<BufferData<'_>>],
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    texture_binding_contracts: &TextureBindingContracts<'_>,
    closure: &ResourceClosure<'_>,
) -> Result<AppearanceStorage<'scope>> {
    let material_count = safe_gltf(ctx, "glTF material count", || gltf.materials().count())?;
    let mut storage = AppearanceStorage {
        images: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-images",
        ),
        encoded_image_payloads: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-encoded-image-payloads",
        ),
        samplers: ctx.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-authoring-samplers",
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
    storage.materials.try_reserve(selected_material_count)?;
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
        let texture_contracts = texture_binding_contracts.material(ctx, material_index)?;
        add_material_authoring(
            ctx,
            &mut storage,
            material,
            resolved_texture_uris,
            external_images,
            texture_contracts,
        )?;
        let Some(slot) = storage.material_ids.as_mut_slice().get_mut(material_index) else {
            return Err(gltf_parse_error(ctx, "glTF material id map is incomplete"));
        };
        *slot = Some(local_id);
    }
    Ok(storage)
}

fn build_shared_appearance_resources<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Document,
    buffers: &[Option<BufferData<'_>>],
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    closure: &ResourceClosure<'_>,
    storage: &mut AppearanceStorage<'scope>,
) -> Result<()> {
    let selected_image_count = selected_count(ctx, closure.images.as_slice())?;
    storage.images.try_reserve(selected_image_count)?;
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
                        format!("glTF texture resolution plan is missing image {image_index}"),
                    )
                })?;
                let (resource_index, _) =
                    external_images.get(ctx, image_index)?.ok_or_else(|| {
                        gltf_parse_error(
                            ctx,
                            format!("glTF texture resource is missing image {image_index}"),
                        )
                    })?;
                let media_type =
                    mime_type
                        .map(ExtensionMediaType::new)
                        .transpose()
                        .map_err(|_| {
                            gltf_parse_error(
                                ctx,
                                format!("glTF image {image_index} has an invalid media type"),
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
        let Some(slot) = storage.image_ids.as_mut_slice().get_mut(image_index) else {
            return Err(gltf_parse_error(ctx, "glTF image id map is incomplete"));
        };
        *slot = Some(local_id);
    }

    let selected_sampler_count = selected_count(ctx, closure.samplers.as_slice())?;
    storage.samplers.try_reserve(selected_sampler_count)?;
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
                format!("glTF texture {texture_index} references an image outside the closure"),
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
                        format!(
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
        let texture_name = texture
            .name()
            .or_else(|| texture.source().name())
            .map(|name| {
                let mut owned = ctx.metered_string(
                    ImportStorageClass::SceneConstruction,
                    "gltf-authoring-texture-name",
                );
                owned.try_push_str(name).map(|()| owned)
            });
        storage.texture_names.try_push(texture_name.transpose()?)?;
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
                format!("glTF image {image_index} has an invalid media type"),
            )
        })
}

fn copy_buffer_view_image<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    view: gltf::buffer::View<'_>,
    image_index: usize,
) -> Result<(MeteredVec<'scope, u8>, u32)> {
    let buffer_index = view.buffer().index();
    let buffer = buffers
        .get(buffer_index)
        .and_then(Option::as_ref)
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF image {image_index} references unavailable buffer {buffer_index}"),
            )
        })?;
    let end = view
        .offset()
        .checked_add(view.length())
        .ok_or_else(|| ctx.allocation_failure("gltf-image-buffer-view-range"))?;
    let bytes = buffer.as_slice().get(view.offset()..end).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format!(
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

fn add_material_authoring<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut AppearanceStorage<'scope>,
    material: gltf::Material<'document>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    contracts: &MaterialTextureContracts,
) -> Result<()> {
    let material_index = material.index().ok_or_else(|| {
        ContentError::from(GltfFailure::internal(
            "glTF default material reached explicit material authoring",
        ))
    })?;
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
            required_texture_binding_contract(
                material_index,
                "baseColorTexture",
                contracts.base_color.as_ref(),
            )?,
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
            required_texture_binding_contract(
                material_index,
                "metallicRoughnessTexture",
                contracts.metallic_roughness.as_ref(),
            )?,
            TextureRole::MetallicRoughness,
            TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green])
                .map_err(|_| {
                    gltf_parse_error(ctx, "glTF metallic-roughness channel mapping is invalid")
                })?,
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
            required_texture_binding_contract(
                material_index,
                "normalTexture",
                contracts.normal.as_ref(),
            )?,
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
            required_texture_binding_contract(
                material_index,
                "occlusionTexture",
                contracts.occlusion.as_ref(),
            )?,
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
            required_texture_binding_contract(
                material_index,
                "emissiveTexture",
                contracts.emissive.as_ref(),
            )?,
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
        name: owned_name(ctx, material.name(), "gltf-normalized-material-name")?,
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

fn required_texture_binding_contract<'contract>(
    material_index: usize,
    source_key: &'static str,
    contract: Option<&'contract TextureBindingContract>,
) -> Result<&'contract TextureBindingContract> {
    contract.ok_or_else(|| {
        ContentError::from(GltfFailure::internal(format_args!(
            "glTF texture binding contract is missing material {material_index} {source_key}"
        )))
    })
}

#[allow(clippy::too_many_arguments)]
fn add_texture_binding<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut AppearanceStorage<'scope>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    texture: gltf::texture::Texture<'document>,
    contract: &TextureBindingContract,
    role: TextureRole,
    channel_swizzle: TextureChannelSwizzle,
    color_interpretation: ColorInterpretation,
    scale: f32,
    source_key: &'static str,
) -> Result<()> {
    let image = texture.source();
    let image_name = image.name();
    let image_index = image.index();
    let resolved_uri_len = match image.source() {
        ImageSource::Uri { uri, .. } if !is_data_uri(uri) => {
            let resolved = resolved_texture_uris.get(&image_index).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format!("glTF texture resolution plan is missing image {image_index}"),
                )
            })?;
            if external_images.get(ctx, image_index)?.is_none() {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF texture resource is missing image {image_index}"),
                ));
            }
            resolved.len()
        }
        ImageSource::Uri { .. } | ImageSource::View { .. } => 0,
    };
    let canonical_string_bytes = resolved_uri_len
        .checked_add(source_key.len())
        .and_then(|bytes| bytes.checked_add(texture.name().or(image_name).map_or(0, str::len)))
        .ok_or_else(|| ctx.allocation_failure("gltf-texture-canonical-strings"))?;
    complete_canonical_string_work(ctx, canonical_string_bytes)?;
    let texture_id = storage
        .texture_ids
        .as_slice()
        .get(texture.index())
        .copied()
        .flatten()
        .ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF texture {} is outside the closure", texture.index()),
            )
        })?;
    let strength = TextureStrength::try_new(scale)
        .map_err(|_| gltf_parse_error(ctx, "glTF texture strength is not finite"))?;
    let source_key = TextureSourceKey::try_new(source_key)
        .map_err(|_| gltf_parse_error(ctx, "static glTF texture source key is invalid"))?;
    storage.bindings.try_push(BindingStorage {
        texture: texture_id,
        provenance: 0,
        role,
        coordinate_source: contract.coordinate_source.clone(),
        channel_mapping: TextureChannelMapping::swizzle(channel_swizzle),
        color_interpretation,
        transform: contract.transform,
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
    meshes: MeteredVec<'scope, Option<MeshContracts<'scope>>>,
}

fn preflight_geometry_contracts<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Document,
    material_count: usize,
    closure: &ResourceClosure<'_>,
) -> Result<GeometryContracts<'scope>> {
    let mesh_count = safe_gltf(ctx, "glTF mesh count", || gltf.meshes().count())?;
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
        meshes.try_push_reserved(Some(MeshContracts { primitives }))?;
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
    gltf: &gltf::Document,
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
            Checked::Valid(json::mesh::Semantic::Extras(_)) => {
                return Err(GltfFailure::unsupported(
                    "custom-attribute",
                    "custom glTF vertex attributes are outside the admitted profile",
                )
                .into());
            }
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
    gltf: &'a gltf::Document,
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
    gltf: &gltf::Document,
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
    gltf: &gltf::Document,
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

fn build_geometry_storage<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Document,
    buffers: &[Option<BufferData<'_>>],
    contracts: &GeometryContracts<'_>,
    material_ids: &MeteredVec<'_, Option<MaterialLocalId>>,
) -> Result<GeometryStorage<'scope>> {
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
            let reference_count = primitive.attribute_stream_count.checked_add(1)?;
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
                ctx,
                buffers,
                &mesh,
                primitive,
                contract,
                material_ids,
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
    mesh: &gltf::Mesh<'_>,
    primitive: gltf::Primitive<'_>,
    contract: PrimitiveContract,
    material_ids: &MeteredVec<'_, Option<MaterialLocalId>>,
    provenance: &mut MeteredVec<'scope, u32>,
    pending_provenance: &mut MeteredVec<'scope, u32>,
) -> Result<PrimitiveStorage<'scope>> {
    let topology = map_topology(contract.mode).ok_or_else(|| ContentError::FeatureUnsupported {
        format: "gltf",
        feature: format!("primitive mode {:?} is not implemented yet", contract.mode),
    })?;

    let attribute_cell_count = contract
        .position_count
        .checked_mul(contract.attribute_stream_count)
        .ok_or_else(|| ctx.allocation_failure("gltf-attribute-cell-counter"))?;
    let provenance_references = contract
        .attribute_stream_count
        .checked_add(1)
        .ok_or_else(|| ctx.allocation_failure("gltf-primitive-provenance-references"))?;
    ctx.charge_work(provenance_work_bound(ctx, provenance_references)?)?;
    pending_provenance.try_reserve(provenance_references)?;
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
            let provenance_offset = provenance.len();
            for (ordinal, (_, accessor)) in primitive.attributes().enumerate() {
                poll_parser_loop(ctx, ordinal)?;
                pending_provenance
                    .try_push_reserved(accessor_provenance(ctx, buffers, accessor)?)?;
            }
            if let Some(accessor) = primitive.indices() {
                pending_provenance
                    .try_push_reserved(accessor_provenance(ctx, buffers, accessor)?)?;
            }
            try_extend_provenance_unique(ctx, provenance, pending_provenance)?;
            let reader = safe_gltf(ctx, "glTF primitive reader construction", || {
                primitive.reader(|buffer| {
                    buffers
                        .get(buffer.index())
                        .and_then(Option::as_ref)
                        .map(BufferData::as_slice)
                })
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
                    provenance: accessor_provenance(
                        ctx,
                        buffers,
                        primitive.get(&Semantic::TexCoords(set)).ok_or_else(|| {
                            gltf_parse_error(
                                ctx,
                                format!("glTF TEXCOORD_{set} accessor disappeared after preflight"),
                            )
                        })?,
                    )?,
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
                    provenance: accessor_provenance(
                        ctx,
                        buffers,
                        primitive.get(&Semantic::Colors(set)).ok_or_else(|| {
                            gltf_parse_error(
                                ctx,
                                format!("glTF COLOR_{set} accessor disappeared after preflight"),
                            )
                        })?,
                    )?,
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
                        material_ids
                            .as_slice()
                            .get(index)
                            .copied()
                            .flatten()
                            .ok_or_else(|| {
                                gltf_parse_error(
                                    ctx,
                                    format!(
                                        "glTF primitive references material {index} outside the closure"
                                    ),
                                )
                            })
                    })
                    .transpose()?,
                attributes,
                influence_offsets,
                influences,
                provenance_offset,
                provenance_count: provenance.len() - provenance_offset,
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

fn build_scene_nodes<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    graph: &NodeGraph<'document, '_>,
    logical_mesh_ids: &MeteredVec<'_, Option<LogicalMeshLocalId>>,
    cameras: &CameraStorage<'_>,
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
            graph.nodes.len(),
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

        let transform = match safe_gltf(ctx, "glTF node transform", || node.transform())? {
            gltf::scene::Transform::Matrix { matrix } => {
                AuthoredTransform::try_matrix(Mat4 { cols: matrix })
            }
            gltf::scene::Transform::Decomposed {
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
            gltf_parse_error(ctx, format!("glTF node {node_index} transform is invalid"))
        })?;
        let camera = match safe_gltf(ctx, "glTF node camera", || node.camera())? {
            Some(camera) => cameras
                .camera_ids
                .as_slice()
                .get(camera.index())
                .copied()
                .flatten()
                .ok_or_else(|| {
                    gltf_parse_error(
                        ctx,
                        format!(
                            "glTF selected node {node_index} references camera {} outside the closure",
                            camera.index()
                        ),
                    )
                })
                .map(Some)?,
            None => None,
        };
        let authored = NodeAuthoringNode::new(
            parent,
            transform,
            camera.map_or(Presence::Absent, Presence::Present),
            Presence::Absent,
        );
        let name = safe_gltf(ctx, "glTF node name", || node.name())?;
        if let Some(name) = name {
            complete_canonical_string_work(ctx, name.len())?;
        }
        storage
            .names
            .try_push_reserved(owned_name(ctx, name, "gltf-normalized-node-name")?)?;
        storage.nodes.try_push_reserved(authored)?;

        let skin_index = safe_gltf(ctx, "glTF node skin", || node.skin())?.map(|skin| skin.index());
        if let Some(mesh) = safe_gltf(ctx, "glTF node mesh", || node.mesh())? {
            let logical_mesh = logical_mesh_ids
                .as_slice()
                .get(mesh.index())
                .copied()
                .flatten()
                .ok_or_else(|| {
                    gltf_parse_error(
                        ctx,
                        format!(
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
    nodes: MeteredVec<'scope, gltf::Node<'document>>,
    children: MeteredVec<'scope, MeteredVec<'scope, usize>>,
    roots: MeteredVec<'scope, usize>,
    selected: MeteredVec<'scope, bool>,
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
    animation_sampler_ranges: MeteredVec<'scope, std::ops::Range<usize>>,
    animation_samplers: MeteredVec<'scope, bool>,
    buffers: MeteredVec<'scope, bool>,
    buffer_views: MeteredVec<'scope, bool>,
    accessors: MeteredVec<'scope, bool>,
}

fn build_resource_closure<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Document,
    graph: &NodeGraph<'document, '_>,
    animation_contracts: &animation::AnimationContracts<'_, '_>,
) -> Result<ResourceClosure<'scope>> {
    let root = gltf.as_json();
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
                format!("glTF closure references missing node {node_index}"),
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
                    format!("glTF selected node {node_index} declares a skin without a mesh"),
                ));
            }
            (None, None) => {}
        }
        if let Some(camera) = node.camera {
            mark_bool(ctx, &mut closure.cameras, camera.value(), "camera")?;
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
            if !animation_selected {
                continue;
            }
            let sampler = animation.samplers.get(channel.sampler.value()).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format!(
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
    primitive: &json::mesh::Primitive,
) -> Result<()> {
    let mut texcoord_sets = [false; 8];
    let mut color_sets = [false; 8];
    for semantic in primitive.attributes.keys() {
        charge_closure_work(ctx)?;
        match semantic {
            Checked::Valid(json::mesh::Semantic::TexCoords(set)) => {
                let Some(selected) = texcoord_sets.get_mut(*set as usize) else {
                    return Err(ContentError::FeatureUnsupported {
                        format: "gltf",
                        feature: format!(
                            "mesh {mesh_index} primitive {primitive_index} TEXCOORD_{set} is not implemented yet"
                        ),
                    });
                };
                *selected = true;
            }
            Checked::Valid(json::mesh::Semantic::Colors(set)) => {
                let Some(selected) = color_sets.get_mut(*set as usize) else {
                    return Err(ContentError::FeatureUnsupported {
                        format: "gltf",
                        feature: format!(
                            "mesh {mesh_index} primitive {primitive_index} COLOR_{set} is not implemented yet"
                        ),
                    });
                };
                *selected = true;
            }
            Checked::Valid(json::mesh::Semantic::Joints(set))
            | Checked::Valid(json::mesh::Semantic::Weights(set))
                if *set != 0 =>
            {
                return Err(ContentError::FeatureUnsupported {
                    format: "gltf",
                    feature: format!(
                        "mesh {mesh_index} primitive {primitive_index} has multiple JOINTS/WEIGHTS sets"
                    ),
                });
            }
            Checked::Invalid => {
                return Err(gltf_parse_error(
                    ctx,
                    format!(
                        "glTF mesh {mesh_index} primitive {primitive_index} has an invalid attribute semantic"
                    ),
                ));
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
        format!(
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
            format!("glTF closure references missing {kind} {index}"),
        ));
    };
    *selected = true;
    Ok(())
}

fn mark_skin(
    ctx: &ParseScope<'_>,
    root: &json::Root,
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
                format!("glTF closure references missing skin {skin_index}"),
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
            format!("glTF closure references missing skin {skin_index}"),
        )
    })?;
    for (joint_ordinal, joint) in skin.joints.iter().enumerate() {
        charge_closure_work(ctx)?;
        poll_parser_loop(ctx, joint_ordinal)?;
        let joint_index = joint.value();
        if !selected_nodes.get(joint_index).copied().unwrap_or(false) {
            return Err(gltf_parse_error(
                ctx,
                format!(
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
                format!(
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
    root: &json::Root,
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
                format!("glTF closure references missing mesh {mesh_index}"),
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
            format!("glTF closure references missing mesh {mesh_index}"),
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
    root: &json::Root,
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
                format!("glTF closure references missing material {material_index}"),
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
            format!("glTF closure references missing material {material_index}"),
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
    root: &json::Root,
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
                format!("glTF closure references missing texture {texture_index}"),
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
            format!("glTF closure references missing texture {texture_index}"),
        )
    })?;
    let image_index = texture.source.value();
    mark_bool(ctx, &mut closure.images, image_index, "image")?;
    let image = root.images.get(image_index).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format!("glTF closure references missing image {image_index}"),
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
    root: &json::Root,
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
                format!("glTF closure references missing accessor {accessor_index}"),
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
            format!("glTF closure references missing accessor {accessor_index}"),
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
    root: &json::Root,
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
                format!("glTF closure references missing bufferView {view_index}"),
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
            format!("glTF closure references missing bufferView {view_index}"),
        )
    })?;
    mark_bool(ctx, &mut closure.buffers, view.buffer.value(), "buffer")?;
    Ok(())
}

fn record_source_scope(
    ctx: &ParseScope<'_>,
    gltf: &gltf::Document,
    graph: &NodeGraph<'_, '_>,
    animation_contracts: &animation::AnimationContracts<'_, '_>,
    closure: &ResourceClosure<'_>,
) -> Result<()> {
    let document = gltf.as_json();
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
    let mut builder = ctx.source_scope_builder(selection_outcome)?;
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
                format!("glTF source-scope mesh {mesh_index} is missing"),
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
    gltf: &'document gltf::Document,
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
    let (roots, selection_outcome) =
        select_and_validate_scene_roots(ctx, gltf, parents.as_slice())?;
    charge_canonical_graph_shape(ctx, roots.len())?;
    let selected = validate_canonical_graph_limits(ctx, children.as_slice(), roots.as_slice())?;

    Ok(NodeGraph {
        nodes,
        children,
        roots,
        selected,
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
                format!("glTF node graph references missing node {node_index}"),
            )
        })?;
        let selected_node = selected.as_mut_slice().get_mut(node_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF selected node {node_index} is out of range"),
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
    gltf: &gltf::Document,
    parents: &[Option<usize>],
) -> Result<(MeteredVec<'scope, usize>, NodeSelectionOutcome)> {
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
    let selection_outcome = match (default_scene_index, scene_count) {
        (Some(scene_index), _) => NodeSelectionOutcome::DeclaredScene(scene_index),
        (None, 0) => NodeSelectionOutcome::ParentlessNodeProjection,
        (None, 1) => NodeSelectionOutcome::DeclaredScene(0),
        (None, _) => {
            return Err(gltf_parse_error(
                ctx,
                "multiple scenes require an explicit default scene",
            ));
        }
    };
    match selection_outcome {
        NodeSelectionOutcome::DeclaredScene(scene_index) => {
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
    gltf: &gltf::Document,
    buffers: &[Option<BufferData<'_>>],
    node_ids: &[Option<NodeLocalId>],
    closure: &ResourceClosure<'_>,
) -> Result<SkinStorage<'scope>> {
    let skin_count = safe_gltf(ctx, "glTF skin count", || gltf.skins().len())?;
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
                format!("glTF skin index {skin_index} is out of range"),
            ));
        };
        if !is_selected {
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
        let provenance = skin
            .inverse_bind_matrices()
            .map(|accessor| accessor_provenance(ctx, buffers, accessor))
            .transpose()?
            .unwrap_or(0);
        let inverse_bind_matrices = if has_inverse_bind_matrices {
            let reader = safe_gltf(ctx, "glTF skin reader construction", || {
                skin.reader(|buffer| {
                    buffers
                        .get(buffer.index())
                        .and_then(Option::as_ref)
                        .map(BufferData::as_slice)
                })
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
                format!("glTF skin index {skin_index} is out of range"),
            ));
        };
        *slot = Some(local_id);
    }
    Ok(storage)
}

fn build_mesh_instance_deformations<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Document,
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
                    gltf_parse_error(ctx, format!("glTF skin {index} was not imported"))
                })?,
            None => Presence::Absent,
        };
        let weights = gltf
            .as_json()
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
    gltf: &gltf::Document,
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

fn build_cameras<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &gltf::Document,
    closure: &ResourceClosure<'_>,
) -> Result<CameraStorage<'scope>> {
    let camera_count = safe_gltf(ctx, "glTF camera count", || gltf.cameras().count())?;
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
        let local_id = CameraLocalId::new(authoring_u32(
            ctx,
            storage.authored.len(),
            "gltf-authoring-camera-id",
        )?);
        storage
            .authored
            .try_push_reserved(CameraAuthoring::new(projection))?;
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
