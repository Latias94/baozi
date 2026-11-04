//! Selected material compilation before dependency IO and reuse during authoring.

use super::{
    AppearanceStorage, BindingStorage, ContentError, ExternalImageResources, ImageSource,
    MaterialStorage, ResolvedTextureUris, ResourceClosure, Result, authoring_u32, gltf_parse_error,
    is_data_uri, owned_name, poll_parser_loop, to_u64,
};
use crate::GltfFailure;
use crate::document::KHR_TEXTURE_TRANSFORM;
use crate::document::schema as json;
use baozi_import::ParseServices;
use baozi_import::authoring::{
    AlphaMode, AuthoringRange, ClearcoatFactors, Color, ColorInterpretation, EmissionFactors,
    MaterialFactors, MaterialLocalId, PbrMetallicRoughnessFactors, ShadingModel, SheenFactors,
    TextureBlendOperation, TextureChannel, TextureChannelSwizzle, TextureRole, TextureSourceKey,
    TextureStrength, TextureTransform, Vec2, Vec3,
};
use baozi_import::{ImportStorageClass, ImportWorkClass, MeteredOrderedMap, MeteredVec, ParseRoot};
use std::ops::Range;

#[derive(Debug)]
pub(super) struct BindingContract {
    pub(super) source_texture: usize,
    pub(super) source_key: &'static str,
    pub(super) uv_set: u16,
    pub(super) role: TextureRole,
    transform: TextureTransform,
    channel_mapping: TextureChannelSwizzle,
    color_interpretation: ColorInterpretation,
    strength: TextureStrength,
}

#[derive(Debug)]
pub(super) struct MaterialContract {
    shading_model: ShadingModel,
    alpha_mode: AlphaMode,
    factors: MaterialFactors,
    bindings: Range<usize>,
    pub(super) requires_tangent_space: bool,
}

pub(super) struct MaterialContracts<'scope> {
    materials: MeteredOrderedMap<'scope, usize, MaterialContract>,
    bindings: MeteredVec<'scope, BindingContract>,
}

impl MaterialContracts<'_> {
    pub(super) fn all_bindings(&self) -> &[BindingContract] {
        self.bindings.as_slice()
    }

    pub(super) fn bindings(
        &self,
        ctx: &mut ParseServices<'_, '_, '_>,
        material_index: usize,
    ) -> Result<&[BindingContract]> {
        let range = self.material(ctx, material_index)?.bindings.clone();
        self.bindings.as_slice().get(range).ok_or_else(|| {
            ContentError::from(GltfFailure::internal(
                "glTF material binding contract range is invalid",
            ))
        })
    }

    pub(super) fn material(
        &self,
        _ctx: &mut ParseServices<'_, '_, '_>,
        material_index: usize,
    ) -> Result<&MaterialContract> {
        self.materials.get(&material_index).ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF material contract is missing material {material_index}"
            ))
        })
    }
}

pub(super) fn preflight<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    gltf: &crate::document::Document<'_>,
    closure: &ResourceClosure<'_>,
) -> Result<MaterialContracts<'scope>> {
    let root = gltf.tables();
    let amount = to_u64(root.extensions_used.len());
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    let extension_declared = ctx.child(&layout, [amount], |child| {
        child.run([amount], || {
            Ok::<_, baozi_import::ParseFailure>(
                root.extensions_used
                    .iter()
                    .any(|name| name == KHR_TEXTURE_TRANSFORM),
            )
        })
    })?;
    let mut materials = ctx.services()?.metered_ordered_map(
        ImportStorageClass::FormatNormalization,
        "gltf-material-contracts",
    );
    let mut bindings = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-material-binding-contracts",
    );

    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (material_index, material) in root.materials.iter().enumerate() {
        poll_parser_loop(&mut ctx.services()?, material_index)?;
        let selected = ctx
            .child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(
                        closure.materials.as_slice().get(material_index).copied(),
                    )
                })
            })?
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
                "glTF KHR_materials_emissive_strength excludes KHR_materials_unlit",
            ));
        }
        if material.unlit.is_some() && (material.clearcoat.is_some() || material.sheen.is_some()) {
            return Err(gltf_parse_error(
                "glTF clearcoat and sheen exclude KHR_materials_unlit",
            ));
        }
        let strength = material
            .emissive_strength
            .map(|id| gltf.emissive_strength(ctx, id))
            .transpose()?;
        let clearcoat = material
            .clearcoat
            .map(|id| gltf.clearcoat(ctx, id))
            .transpose()?;
        let sheen = material.sheen.map(|id| gltf.sheen(ctx, id)).transpose()?;
        let pbr = &material.pbr_metallic_roughness;
        let factors = compile_factors(
            &mut ctx.services()?,
            pbr.base_color_factor,
            pbr.metallic_factor,
            pbr.roughness_factor,
            material.emissive_factor,
            strength,
        )?
        .with_clearcoat(
            clearcoat
                .as_ref()
                .map(|coat| ClearcoatFactors::try_new(coat.factor, coat.roughness))
                .transpose()
                .map_err(|_| gltf_parse_error("glTF clearcoat factors are invalid"))?,
        )
        .with_sheen(
            sheen
                .as_ref()
                .map(|sheen| {
                    SheenFactors::try_new(
                        Vec3::new(sheen.color[0], sheen.color[1], sheen.color[2]),
                        sheen.roughness,
                    )
                })
                .transpose()
                .map_err(|_| gltf_parse_error("glTF sheen factors are invalid"))?,
        );
        let binding_start = bindings.len();
        // Preserve core slot order, then append layer slots, including zero-valued layers.
        for (info, source_key, role) in [
            (
                pbr.base_color_texture.as_ref(),
                "baseColorTexture",
                TextureRole::BaseColor,
            ),
            (
                pbr.metallic_roughness_texture.as_ref(),
                "metallicRoughnessTexture",
                TextureRole::MetallicRoughness,
            ),
            (
                material.normal_texture.as_ref(),
                "normalTexture",
                TextureRole::Normal,
            ),
            (
                material.occlusion_texture.as_ref(),
                "occlusionTexture",
                TextureRole::Occlusion,
            ),
            (
                material.emissive_texture.as_ref(),
                "emissiveTexture",
                TextureRole::Emissive,
            ),
            (
                clearcoat.as_ref().and_then(|coat| coat.texture.as_ref()),
                "clearcoatTexture",
                TextureRole::Clearcoat,
            ),
            (
                clearcoat
                    .as_ref()
                    .and_then(|coat| coat.roughness_texture.as_ref()),
                "clearcoatRoughnessTexture",
                TextureRole::ClearcoatRoughness,
            ),
            (
                clearcoat
                    .as_ref()
                    .and_then(|coat| coat.normal_texture.as_ref()),
                "clearcoatNormalTexture",
                TextureRole::ClearcoatNormal,
            ),
            (
                sheen
                    .as_ref()
                    .and_then(|sheen| sheen.color_texture.as_ref()),
                "sheenColorTexture",
                TextureRole::SheenColor,
            ),
            (
                sheen
                    .as_ref()
                    .and_then(|sheen| sheen.roughness_texture.as_ref()),
                "sheenRoughnessTexture",
                TextureRole::SheenRoughness,
            ),
        ] {
            let info = ctx.child(&work_group, [1], |child| {
                child.run([1], || Ok::<_, baozi_import::ParseFailure>(info.as_ref()))
            })?;
            if let Some(info) = info {
                {
                    let value = compile_texture_info_contract(
                        ctx,
                        gltf,
                        material_index,
                        source_key,
                        role,
                        info,
                        extension_declared,
                    )?;
                    bindings.try_push(&mut ctx.services()?, value)
                }?;
            }
        }
        let contract = MaterialContract {
            shading_model: if material.unlit.is_some() {
                ShadingModel::Unlit
            } else {
                ShadingModel::PbrMetallicRoughness
            },
            alpha_mode: map_alpha_mode(&mut ctx.services()?, material)?,
            factors,
            bindings: binding_start..bindings.len(),
            requires_tangent_space: clearcoat
                .as_ref()
                .is_some_and(|coat| coat.normal_texture.is_some())
                && material.normal_texture.is_none(),
        };
        if materials
            .try_insert(&mut ctx.services()?, material_index, contract)?
            .is_some()
        {
            return Err(ContentError::from(GltfFailure::internal(
                "glTF material contract index was inserted twice",
            )));
        }
    }

    Ok(MaterialContracts {
        materials,
        bindings,
    })
}

fn compile_factors(
    _ctx: &mut ParseServices<'_, '_, '_>,
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
    .map_err(|_| gltf_parse_error("glTF material factors are outside canonical bounds"))?;
    let emission =
        EmissionFactors::try_new(Vec3::new(emissive[0], emissive[1], emissive[2]), strength)
            .map_err(|_| gltf_parse_error("glTF material factors are outside canonical bounds"))?;
    Ok(MaterialFactors::empty()
        .with_emission(emission)
        .with_pbr_metallic_roughness(Some(pbr)))
}

fn compile_texture_info_contract(
    ctx: &mut ParseRoot<'_, '_, '_>,
    document: &crate::document::Document<'_>,
    material_index: usize,
    source_key: &'static str,
    role: TextureRole,
    info: &json::texture::Info,
    extension_declared: bool,
) -> Result<BindingContract> {
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    let source_texture = ctx.child(&layout, [1], |child| {
        child.run([1], || {
            Ok::<_, baozi_import::ParseFailure>(info.index.value())
        })
    })?;
    if document.tables().textures.get(source_texture).is_none() {
        return Err(gltf_parse_error(format_args!(
            "glTF material {material_index} {source_key} texture {source_texture} is out of range"
        )));
    }
    require_texture_transform_declaration(
        &mut ctx.services()?,
        material_index,
        source_key,
        info.transform.is_some(),
        extension_declared,
    )?;
    let transform = info
        .transform
        .map(|id| document.texture_transform(ctx, id))
        .transpose()?;
    let (tex_coord, transform) = match transform.as_ref() {
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
                    format_args!(
                        "glTF material {material_index} {source_key} has invalid {KHR_TEXTURE_TRANSFORM}: {error}"
                    ),
                )
            })?;
            (transform.tex_coord.unwrap_or(info.tex_coord), canonical)
        }
        None => (info.tex_coord, TextureTransform::IDENTITY),
    };
    let uv_set = u16::try_from(tex_coord).map_err(|_| {
        gltf_parse_error(
            format_args!(
                "glTF material {material_index} {source_key} texture coordinate set {tex_coord} is out of range"
            ),
        )
    })?;
    let (channels, color_interpretation, scale): (&[TextureChannel], _, _) = match role {
        TextureRole::BaseColor => (
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
                TextureChannel::Alpha,
            ],
            ColorInterpretation::SrgbColor,
            1.0,
        ),
        TextureRole::MetallicRoughness => (
            &[TextureChannel::Blue, TextureChannel::Green],
            ColorInterpretation::Data,
            1.0,
        ),
        TextureRole::Normal | TextureRole::ClearcoatNormal => (
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
            ],
            ColorInterpretation::Data,
            info.strength,
        ),
        TextureRole::Occlusion => (
            &[TextureChannel::Red],
            ColorInterpretation::Data,
            info.strength,
        ),
        TextureRole::Emissive | TextureRole::SheenColor => (
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
            ],
            ColorInterpretation::SrgbColor,
            1.0,
        ),
        TextureRole::Clearcoat => (&[TextureChannel::Red], ColorInterpretation::Data, 1.0),
        TextureRole::ClearcoatRoughness => {
            (&[TextureChannel::Green], ColorInterpretation::Data, 1.0)
        }
        TextureRole::SheenRoughness => (&[TextureChannel::Alpha], ColorInterpretation::Data, 1.0),
        _ => {
            return Err(ContentError::from(GltfFailure::internal(
                "glTF material contract has an unsupported texture role",
            )));
        }
    };
    let channel_mapping = TextureChannelSwizzle::try_new(channels)
        .map_err(|_| gltf_parse_error("static glTF texture channel mapping is invalid"))?;
    let strength = TextureStrength::try_new(scale)
        .map_err(|_| gltf_parse_error("glTF texture strength is not finite"))?;
    Ok(BindingContract {
        source_texture,
        source_key,
        uv_set,
        role,
        transform,
        channel_mapping,
        color_interpretation,
        strength,
    })
}

fn require_texture_transform_declaration(
    _ctx: &mut ParseServices<'_, '_, '_>,
    material_index: usize,
    source_key: &'static str,
    has_transform: bool,
    extension_declared: bool,
) -> Result<()> {
    if has_transform && !extension_declared {
        return Err(gltf_parse_error(format_args!(
            "glTF material {material_index} {source_key} uses {KHR_TEXTURE_TRANSFORM} without declaring it in extensionsUsed"
        )));
    }
    Ok(())
}

pub(super) fn add_authoring<'scope, 'document>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    storage: &mut AppearanceStorage<'scope>,
    gltf: &crate::document::Document<'document>,
    material: crate::document::Material<'document>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    contracts: &MaterialContracts<'_>,
) -> Result<()> {
    let material_index = material.index();
    let contract = contracts.material(&mut ctx.services()?, material_index)?;
    let name_bytes = material.name().map_or(0, str::len);
    let source_name = if name_bytes == 0 {
        material.name()
    } else {
        let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
        ctx.child(&layout, [to_u64(name_bytes)], |child| {
            child.run([to_u64(name_bytes)], || {
                Ok::<_, baozi_import::ParseFailure>(material.name())
            })
        })?
    };
    let binding_offset = authoring_u32(
        &mut ctx.services()?,
        storage.bindings.len(),
        "gltf-material-binding-offset",
    )?;

    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (ordinal, binding) in contracts
        .bindings(&mut ctx.services()?, material_index)?
        .iter()
        .enumerate()
    {
        poll_parser_loop(&mut ctx.services()?, ordinal)?;
        let valid = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(
                    gltf.tables().textures.get(binding.source_texture).is_some(),
                )
            })
        })?;
        if !valid {
            return Err(ContentError::from(GltfFailure::internal(
                "glTF compiled binding texture index is invalid",
            )));
        }
        let texture = gltf.textures().nth(binding.source_texture).ok_or_else(|| {
            ContentError::from(GltfFailure::internal(
                "glTF compiled binding texture is missing",
            ))
        })?;
        add_texture_binding(
            ctx,
            storage,
            resolved_texture_uris,
            external_images,
            texture,
            binding,
        )?;
    }

    let binding_count = storage
        .bindings
        .len()
        .checked_sub(binding_offset as usize)
        .ok_or_else(|| ctx.allocation_failure("gltf-material-binding-range"))?;
    storage.materials.try_push_reserved(MaterialStorage {
        shading_model: contract.shading_model,
        name: owned_name(ctx, source_name, "gltf-normalized-material-name")?,
        bindings: AuthoringRange::new(
            binding_offset,
            authoring_u32(
                &mut ctx.services()?,
                binding_count,
                "gltf-material-binding-count",
            )?,
        ),
        alpha_mode: contract.alpha_mode,
        double_sided: material.double_sided(),
        factors: contract.factors,
    })?;
    Ok(())
}

pub(super) fn add_default_authoring<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    storage: &mut AppearanceStorage<'scope>,
) -> Result<()> {
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    let count = ctx.child(&layout, [1], |child| {
        child.run([1], || {
            Ok::<_, baozi_import::ParseFailure>(storage.materials.len())
        })
    })?;
    let id = MaterialLocalId::new(authoring_u32(
        &mut ctx.services()?,
        count,
        "gltf-authoring-default-material-id",
    )?);
    let factors = compile_factors(&mut ctx.services()?, [1.0; 4], 1.0, 1.0, [0.0; 3], None)?;
    storage.materials.try_push_reserved(MaterialStorage {
        shading_model: ShadingModel::PbrMetallicRoughness,
        name: None,
        bindings: AuthoringRange::new(
            authoring_u32(
                &mut ctx.services()?,
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

fn add_texture_binding<'scope, 'document>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    storage: &mut AppearanceStorage<'scope>,
    resolved_texture_uris: &ResolvedTextureUris<'_>,
    external_images: &ExternalImageResources<'scope>,
    texture: crate::document::Texture<'document>,
    contract: &BindingContract,
) -> Result<()> {
    let image = texture.source();
    let image_index = image.index();
    let resolved_uri_len = match image.source() {
        ImageSource::Uri { uri, .. } if !is_data_uri(uri) => {
            let resolved = resolved_texture_uris.get(&image_index).ok_or_else(|| {
                gltf_parse_error(format_args!(
                    "glTF texture resolution plan is missing image {image_index}"
                ))
            })?;
            if external_images.get(ctx, image_index)?.is_none() {
                return Err(gltf_parse_error(format_args!(
                    "glTF texture resource is missing image {image_index}"
                )));
            }
            resolved.len()
        }
        ImageSource::Uri { .. } | ImageSource::View { .. } => 0,
    };
    let canonical_string_bytes = resolved_uri_len
        .checked_add(contract.source_key.len())
        .and_then(|bytes| bytes.checked_add(texture.name().map_or(0, str::len)))
        .ok_or_else(|| ctx.allocation_failure("gltf-texture-canonical-strings"))?;
    let source_key = if canonical_string_bytes == 0 {
        TextureSourceKey::try_new(contract.source_key)
    } else {
        let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
        ctx.child(&layout, [to_u64(canonical_string_bytes)], |child| {
            child.run([to_u64(canonical_string_bytes)], || {
                Ok::<_, baozi_import::ParseFailure>(TextureSourceKey::try_new(contract.source_key))
            })
        })?
    };
    let texture_id = storage
        .texture_ids
        .as_slice()
        .get(texture.index())
        .copied()
        .flatten()
        .ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF texture {} is outside the closure",
                texture.index()
            ))
        })?;
    let source_key =
        source_key.map_err(|_| gltf_parse_error("static glTF texture source key is invalid"))?;
    storage.bindings.try_push(
        &mut ctx.services()?,
        BindingStorage {
            texture: texture_id,
            provenance: 0,
            role: contract.role,
            uv_set: contract.uv_set,
            channel_mapping: contract.channel_mapping,
            color_interpretation: contract.color_interpretation,
            transform: contract.transform,
            strength: contract.strength,
            blend: TextureBlendOperation::Replace,
            source_key,
        },
    )?;
    Ok(())
}

fn map_alpha_mode(
    _ctx: &mut ParseServices<'_, '_, '_>,
    material: &json::Material<'_>,
) -> Result<AlphaMode> {
    match material.alpha_mode {
        crate::document::AlphaMode::Opaque => Ok(AlphaMode::Opaque),
        crate::document::AlphaMode::Mask => AlphaMode::mask(material.alpha_cutoff.unwrap_or(0.5))
            .map_err(|_| gltf_parse_error("glTF alpha cutoff is outside canonical bounds")),
        crate::document::AlphaMode::Blend => Ok(AlphaMode::Blend),
    }
}
