//! Selected material compilation before dependency IO and reuse during authoring.

use super::{
    AppearanceStorage, BindingStorage, ContentError, ExternalImageResources, ImageSource,
    MaterialStorage, ResolvedTextureUris, ResourceClosure, Result, authoring_u32,
    complete_canonical_string_work, gltf_parse_error, is_data_uri, owned_name, poll_parser_loop,
    to_u64,
};
use crate::GltfFailure;
use crate::document::KHR_TEXTURE_TRANSFORM;
use crate::document::schema as json;
use baozi_import::authoring::{
    AlphaMode, AuthoringRange, Color, ColorInterpretation, EmissionFactors, MaterialFactors,
    MaterialLocalId, PbrMetallicRoughnessFactors, ShadingModel, TextureBlendOperation,
    TextureChannel, TextureChannelSwizzle, TextureRole, TextureSourceKey, TextureStrength,
    TextureTransform, Vec2, Vec3,
};
use baozi_import::{ImportStorageClass, MeteredOrderedMap, ParseScope};

#[derive(Debug)]
struct TextureBindingContract {
    uv_set: u16,
    transform: TextureTransform,
}

#[derive(Debug)]
pub(super) struct MaterialContract {
    shading_model: ShadingModel,
    alpha_mode: AlphaMode,
    factors: MaterialFactors,
    base_color: Option<TextureBindingContract>,
    metallic_roughness: Option<TextureBindingContract>,
    normal: Option<TextureBindingContract>,
    occlusion: Option<TextureBindingContract>,
    emissive: Option<TextureBindingContract>,
}

pub(super) struct MaterialContracts<'scope> {
    materials: MeteredOrderedMap<'scope, usize, MaterialContract>,
}

impl MaterialContracts<'_> {
    pub(super) fn material(
        &self,
        ctx: &ParseScope<'_>,
        material_index: usize,
    ) -> Result<&MaterialContract> {
        self.materials.get(&material_index).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("glTF material contract is missing material {material_index}"),
            )
        })
    }
}

pub(super) fn preflight<'scope>(
    ctx: &'scope ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    closure: &ResourceClosure<'_>,
) -> Result<MaterialContracts<'scope>> {
    let root = gltf.tables();
    ctx.charge_work(to_u64(root.extensions_used.len()))?;
    let extension_declared = root
        .extensions_used
        .iter()
        .any(|name| name == KHR_TEXTURE_TRANSFORM);
    let mut materials = ctx.metered_ordered_map(
        ImportStorageClass::FormatNormalization,
        "gltf-material-contracts",
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
                    "glTF material closure length changed during material preflight",
                ))
            })?;
        if !selected {
            continue;
        }
        if material.unlit.is_some() && material.emissive_strength.is_some() {
            return Err(gltf_parse_error(
                ctx,
                "glTF KHR_materials_emissive_strength excludes KHR_materials_unlit",
            ));
        }
        let strength = material
            .emissive_strength
            .map(|id| gltf.emissive_strength(ctx, id))
            .transpose()?;
        let pbr = &material.pbr_metallic_roughness;
        let contracts = MaterialContract {
            shading_model: if material.unlit.is_some() {
                ShadingModel::Unlit
            } else {
                ShadingModel::PbrMetallicRoughness
            },
            alpha_mode: map_alpha_mode(ctx, material)?,
            factors: compile_factors(
                ctx,
                pbr.base_color_factor,
                pbr.metallic_factor,
                pbr.roughness_factor,
                material.emissive_factor,
                strength,
            )?,
            base_color: pbr
                .base_color_texture
                .as_ref()
                .map(|info| {
                    compile_texture_info_contract(
                        ctx,
                        gltf,
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
                        gltf,
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
                    compile_texture_info_contract(
                        ctx,
                        gltf,
                        material_index,
                        "normalTexture",
                        info,
                        extension_declared,
                    )
                })
                .transpose()?,
            occlusion: material
                .occlusion_texture
                .as_ref()
                .map(|info| {
                    compile_texture_info_contract(
                        ctx,
                        gltf,
                        material_index,
                        "occlusionTexture",
                        info,
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
                        gltf,
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
                "glTF material contract index was inserted twice",
            )));
        }
    }

    Ok(MaterialContracts { materials })
}

fn compile_factors(
    ctx: &ParseScope<'_>,
    base: [f32; 4],
    metallic: f32,
    roughness: f32,
    emissive: [f32; 3],
    strength: Option<f32>,
) -> Result<MaterialFactors> {
    let pbr = PbrMetallicRoughnessFactors::try_new(
        Color::linear_rgba(base[0], base[1], base[2], base[3]),
        metallic,
        roughness,
    )
    .map_err(|_| gltf_parse_error(ctx, "glTF material factors are outside canonical bounds"))?;
    let emission =
        EmissionFactors::try_new(Vec3::new(emissive[0], emissive[1], emissive[2]), strength)
            .map_err(|_| {
                gltf_parse_error(ctx, "glTF material factors are outside canonical bounds")
            })?;
    Ok(MaterialFactors::empty()
        .with_emission(emission)
        .with_pbr_metallic_roughness(Some(pbr)))
}

fn compile_texture_info_contract(
    ctx: &ParseScope<'_>,
    document: &crate::document::Document<'_>,
    material_index: usize,
    source_key: &'static str,
    info: &json::texture::Info,
    extension_declared: bool,
) -> Result<TextureBindingContract> {
    require_texture_transform_declaration(
        ctx,
        material_index,
        source_key,
        info.transform.is_some(),
        extension_declared,
    )?;
    let transform = info
        .transform
        .map(|id| document.texture_transform(ctx, id))
        .transpose()?;
    compile_texture_binding_contract(
        ctx,
        material_index,
        source_key,
        info.tex_coord,
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
            format_args!(
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
            let offset = transform.offset;
            let scale = transform.scale;
            let canonical = TextureTransform::try_new(
                Vec2::new(offset[0], offset[1]),
                transform.rotation,
                Vec2::new(scale[0], scale[1]),
            )
            .map_err(|error| {
                gltf_parse_error(
                    ctx,
                    format_args!(
                        "glTF material {material_index} {source_key} has invalid {KHR_TEXTURE_TRANSFORM}: {error}"
                    ),
                )
            })?;
            (transform.tex_coord.unwrap_or(tex_coord), canonical)
        }
        None => (tex_coord, TextureTransform::IDENTITY),
    };
    let uv_set = u16::try_from(tex_coord).map_err(|_| {
        gltf_parse_error(
            ctx,
            format_args!(
                "glTF material {material_index} {source_key} texture coordinate set {tex_coord} is out of range"
            ),
        )
    })?;
    Ok(TextureBindingContract { uv_set, transform })
}

pub(super) fn add_authoring<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut AppearanceStorage<'scope>,
    material: crate::document::Material<'document>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    contracts: &MaterialContract,
) -> Result<()> {
    let material_index = material.index().ok_or_else(|| {
        ContentError::from(GltfFailure::internal(
            "glTF default material reached explicit material authoring",
        ))
    })?;
    let pbr = material.pbr_metallic_roughness();
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
    storage.materials.try_push_reserved(MaterialStorage {
        shading_model: contracts.shading_model,
        name: owned_name(ctx, material.name(), "gltf-normalized-material-name")?,
        bindings: AuthoringRange::new(
            binding_offset,
            authoring_u32(ctx, binding_count, "gltf-material-binding-count")?,
        ),
        alpha_mode: contracts.alpha_mode,
        double_sided: material.double_sided(),
        factors: contracts.factors,
    })?;
    Ok(())
}

pub(super) fn add_default_authoring<'scope>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut AppearanceStorage<'scope>,
) -> Result<()> {
    ctx.charge_work(1)?;
    let id = MaterialLocalId::new(authoring_u32(
        ctx,
        storage.materials.len(),
        "gltf-authoring-default-material-id",
    )?);
    let factors = compile_factors(ctx, [1.0; 4], 1.0, 1.0, [0.0; 3], None)?;
    storage.materials.try_push_reserved(MaterialStorage {
        shading_model: ShadingModel::PbrMetallicRoughness,
        name: None,
        bindings: AuthoringRange::new(
            authoring_u32(
                ctx,
                storage.bindings.len(),
                "gltf-default-material-bindings",
            )?,
            0,
        ),
        alpha_mode: AlphaMode::Opaque,
        double_sided: false,
        factors,
    })?;
    storage.default_material = Some(id);
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
    texture: crate::document::Texture<'document>,
    contract: &TextureBindingContract,
    role: TextureRole,
    channel_swizzle: TextureChannelSwizzle,
    color_interpretation: ColorInterpretation,
    scale: f32,
    source_key: &'static str,
) -> Result<()> {
    let image = texture.source();
    let image_index = image.index();
    let resolved_uri_len = match image.source() {
        ImageSource::Uri { uri, .. } if !is_data_uri(uri) => {
            let resolved = resolved_texture_uris.get(&image_index).ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format_args!("glTF texture resolution plan is missing image {image_index}"),
                )
            })?;
            if external_images.get(ctx, image_index)?.is_none() {
                return Err(gltf_parse_error(
                    ctx,
                    format_args!("glTF texture resource is missing image {image_index}"),
                ));
            }
            resolved.len()
        }
        ImageSource::Uri { .. } | ImageSource::View { .. } => 0,
    };
    let canonical_string_bytes = resolved_uri_len
        .checked_add(source_key.len())
        .and_then(|bytes| bytes.checked_add(texture.name().map_or(0, str::len)))
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
                format_args!("glTF texture {} is outside the closure", texture.index()),
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
        uv_set: contract.uv_set,
        channel_mapping: channel_swizzle,
        color_interpretation,
        transform: contract.transform,
        strength,
        blend: TextureBlendOperation::Replace,
        source_key,
    })?;
    Ok(())
}

fn map_alpha_mode(ctx: &ParseScope<'_>, material: &json::Material<'_>) -> Result<AlphaMode> {
    match material.alpha_mode {
        crate::document::AlphaMode::Opaque => Ok(AlphaMode::Opaque),
        crate::document::AlphaMode::Mask => AlphaMode::mask(material.alpha_cutoff.unwrap_or(0.5))
            .map_err(|_| gltf_parse_error(ctx, "glTF alpha cutoff is outside canonical bounds")),
        crate::document::AlphaMode::Blend => Ok(AlphaMode::Blend),
    }
}
