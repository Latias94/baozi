//! Private canonical Scene appearance schema.
//!
//! Appearance is deliberately a separate retained domain. Images own encoded
//! source payload, samplers own behavior, textures pair those identities, and
//! materials own their flat texture-binding ranges. The row layouts below make
//! every cross-table relationship explicit for central validation and snapshot
//! traversal.

use baozi_operation::{ByteBlock, RecordBlock, ScalarSequence, Utf8Block};

use crate::appearance::{
    AlphaMode, ColorInterpretation, LegacyMaterialFactors, MaterialFactors,
    PbrMetallicRoughnessFactors, PbrSpecularGlossinessFactors, ShadingModel,
    SupplementalPbrFactors, TextureBlendOperation, TextureChannel, TextureCoordinateSource,
    TextureRole, TextureStrength, TextureTransform,
};
use crate::image::{MipmapFilter, TexelFilter, TextureAddressMode};
use crate::math::{Color, Vec2, Vec3};

use super::CanonicalSchema;

pub(in super::super) const APPEARANCE_SCHEMA_REVISION: u32 = 2;

pub(in super::super) const APPEARANCE_DIRECTORY_FIELD_COUNT: usize = 23;
pub(in super::super) const APPEARANCE_IMAGE_FIELD_COUNT: usize = 10;
pub(in super::super) const APPEARANCE_SAMPLER_FIELD_COUNT: usize = 7;
pub(in super::super) const APPEARANCE_TEXTURE_FIELD_COUNT: usize = 5;
pub(in super::super) const APPEARANCE_MATERIAL_FIELD_COUNT: usize = 18;
pub(in super::super) const APPEARANCE_BINDING_FIELD_COUNT: usize = 19;
pub(in super::super) const APPEARANCE_MATERIAL_FACTOR_STRIDE: usize = 39;
pub(in super::super) const APPEARANCE_BINDING_SEMANTIC_STRIDE: usize = 6;

pub(in super::super) const APPEARANCE_PRESENCE_ABSENT: u8 = 0;
pub(in super::super) const APPEARANCE_PRESENCE_PRESENT: u8 = 1;

pub(in super::super) const APPEARANCE_IMAGE_PAYLOAD_ENCODED: u8 = 0;
pub(in super::super) const APPEARANCE_MEDIA_TYPE_UTF8: u8 = 0;

pub(in super::super) const APPEARANCE_TEXEL_FILTER_NEAREST: u8 = 0;
pub(in super::super) const APPEARANCE_TEXEL_FILTER_LINEAR: u8 = 1;

pub(in super::super) const APPEARANCE_MIPMAP_FILTER_NONE: u8 = 0;
pub(in super::super) const APPEARANCE_MIPMAP_FILTER_NEAREST: u8 = 1;
pub(in super::super) const APPEARANCE_MIPMAP_FILTER_LINEAR: u8 = 2;

pub(in super::super) const APPEARANCE_ADDRESS_REPEAT: u8 = 0;
pub(in super::super) const APPEARANCE_ADDRESS_MIRRORED_REPEAT: u8 = 1;
pub(in super::super) const APPEARANCE_ADDRESS_CLAMP_TO_EDGE: u8 = 2;
pub(in super::super) const APPEARANCE_ADDRESS_CLAMP_TO_BORDER: u8 = 3;

pub(in super::super) const APPEARANCE_ROLE_BASE_COLOR: u8 = 0;
pub(in super::super) const APPEARANCE_ROLE_DIFFUSE: u8 = 1;
pub(in super::super) const APPEARANCE_ROLE_SPECULAR: u8 = 2;
pub(in super::super) const APPEARANCE_ROLE_METALLIC: u8 = 3;
pub(in super::super) const APPEARANCE_ROLE_ROUGHNESS: u8 = 4;
pub(in super::super) const APPEARANCE_ROLE_METALLIC_ROUGHNESS: u8 = 5;
pub(in super::super) const APPEARANCE_ROLE_NORMAL: u8 = 6;
pub(in super::super) const APPEARANCE_ROLE_OCCLUSION: u8 = 7;
pub(in super::super) const APPEARANCE_ROLE_EMISSIVE: u8 = 8;
pub(in super::super) const APPEARANCE_ROLE_OPACITY: u8 = 9;
pub(in super::super) const APPEARANCE_ROLE_DISPLACEMENT: u8 = 10;
pub(in super::super) const APPEARANCE_ROLE_CLEARCOAT: u8 = 11;
pub(in super::super) const APPEARANCE_ROLE_CLEARCOAT_ROUGHNESS: u8 = 12;
pub(in super::super) const APPEARANCE_ROLE_CLEARCOAT_NORMAL: u8 = 13;
pub(in super::super) const APPEARANCE_ROLE_TRANSMISSION: u8 = 14;
pub(in super::super) const APPEARANCE_ROLE_THICKNESS: u8 = 15;
pub(in super::super) const APPEARANCE_ROLE_SHEEN_COLOR: u8 = 16;
pub(in super::super) const APPEARANCE_ROLE_SHEEN_ROUGHNESS: u8 = 17;

pub(in super::super) const APPEARANCE_SHADING_UNLIT: u8 = 0;
pub(in super::super) const APPEARANCE_SHADING_LAMBERT: u8 = 1;
pub(in super::super) const APPEARANCE_SHADING_PHONG: u8 = 2;
pub(in super::super) const APPEARANCE_SHADING_BLINN_PHONG: u8 = 3;
pub(in super::super) const APPEARANCE_SHADING_PBR_METALLIC_ROUGHNESS: u8 = 4;
pub(in super::super) const APPEARANCE_SHADING_PBR_SPECULAR_GLOSSINESS: u8 = 5;
pub(in super::super) const APPEARANCE_SHADING_UNSPECIFIED: u8 = 6;

pub(in super::super) const APPEARANCE_ALPHA_OPAQUE: u8 = 0;
pub(in super::super) const APPEARANCE_ALPHA_MASK: u8 = 1;
pub(in super::super) const APPEARANCE_ALPHA_BLEND: u8 = 2;

pub(in super::super) const APPEARANCE_COORDINATE_UV_SET: u8 = 0;
pub(in super::super) const APPEARANCE_COORDINATE_OBJECT_POSITION: u8 = 1;
pub(in super::super) const APPEARANCE_COORDINATE_OBJECT_NORMAL: u8 = 2;
pub(in super::super) const APPEARANCE_COORDINATE_REFLECTION_VECTOR: u8 = 3;

pub(in super::super) const APPEARANCE_CHANNEL_RED: u8 = 0;
pub(in super::super) const APPEARANCE_CHANNEL_GREEN: u8 = 1;
pub(in super::super) const APPEARANCE_CHANNEL_BLUE: u8 = 2;
pub(in super::super) const APPEARANCE_CHANNEL_ALPHA: u8 = 3;
pub(in super::super) const APPEARANCE_CHANNEL_ZERO: u8 = 4;
pub(in super::super) const APPEARANCE_CHANNEL_ONE: u8 = 5;

pub(in super::super) const APPEARANCE_COLOR_SRGB: u8 = 0;
pub(in super::super) const APPEARANCE_COLOR_LINEAR: u8 = 1;
pub(in super::super) const APPEARANCE_COLOR_DATA: u8 = 2;

pub(in super::super) const APPEARANCE_BLEND_REPLACE: u8 = 0;

pub(in super::super) const fn encode_appearance_texel_filter(filter: TexelFilter) -> u8 {
    match filter {
        TexelFilter::Nearest => APPEARANCE_TEXEL_FILTER_NEAREST,
        TexelFilter::Linear => APPEARANCE_TEXEL_FILTER_LINEAR,
    }
}

pub(in super::super) const fn encode_appearance_mipmap_filter(filter: MipmapFilter) -> u8 {
    match filter {
        MipmapFilter::None => APPEARANCE_MIPMAP_FILTER_NONE,
        MipmapFilter::Nearest => APPEARANCE_MIPMAP_FILTER_NEAREST,
        MipmapFilter::Linear => APPEARANCE_MIPMAP_FILTER_LINEAR,
    }
}

pub(in super::super) const fn encode_appearance_address_mode(mode: TextureAddressMode) -> u8 {
    match mode {
        TextureAddressMode::Repeat => APPEARANCE_ADDRESS_REPEAT,
        TextureAddressMode::MirroredRepeat => APPEARANCE_ADDRESS_MIRRORED_REPEAT,
        TextureAddressMode::ClampToEdge => APPEARANCE_ADDRESS_CLAMP_TO_EDGE,
        TextureAddressMode::ClampToBorder => APPEARANCE_ADDRESS_CLAMP_TO_BORDER,
    }
}

/// Returns no tag for a schema-backed role until extension authority is stored
/// in this production rail. Callers must reject that case rather than erase it.
pub(in super::super) const fn encode_appearance_texture_role(role: &TextureRole) -> Option<u8> {
    match role {
        TextureRole::BaseColor => Some(APPEARANCE_ROLE_BASE_COLOR),
        TextureRole::Diffuse => Some(APPEARANCE_ROLE_DIFFUSE),
        TextureRole::Specular => Some(APPEARANCE_ROLE_SPECULAR),
        TextureRole::Metallic => Some(APPEARANCE_ROLE_METALLIC),
        TextureRole::Roughness => Some(APPEARANCE_ROLE_ROUGHNESS),
        TextureRole::MetallicRoughness => Some(APPEARANCE_ROLE_METALLIC_ROUGHNESS),
        TextureRole::Normal => Some(APPEARANCE_ROLE_NORMAL),
        TextureRole::Occlusion => Some(APPEARANCE_ROLE_OCCLUSION),
        TextureRole::Emissive => Some(APPEARANCE_ROLE_EMISSIVE),
        TextureRole::Opacity => Some(APPEARANCE_ROLE_OPACITY),
        TextureRole::Displacement => Some(APPEARANCE_ROLE_DISPLACEMENT),
        TextureRole::Clearcoat => Some(APPEARANCE_ROLE_CLEARCOAT),
        TextureRole::ClearcoatRoughness => Some(APPEARANCE_ROLE_CLEARCOAT_ROUGHNESS),
        TextureRole::ClearcoatNormal => Some(APPEARANCE_ROLE_CLEARCOAT_NORMAL),
        TextureRole::Transmission => Some(APPEARANCE_ROLE_TRANSMISSION),
        TextureRole::Thickness => Some(APPEARANCE_ROLE_THICKNESS),
        TextureRole::SheenColor => Some(APPEARANCE_ROLE_SHEEN_COLOR),
        TextureRole::SheenRoughness => Some(APPEARANCE_ROLE_SHEEN_ROUGHNESS),
        TextureRole::Namespaced { .. } => None,
    }
}

pub(in super::super) const fn decode_appearance_texture_role(tag: u8) -> Option<TextureRole> {
    match tag {
        APPEARANCE_ROLE_BASE_COLOR => Some(TextureRole::BaseColor),
        APPEARANCE_ROLE_DIFFUSE => Some(TextureRole::Diffuse),
        APPEARANCE_ROLE_SPECULAR => Some(TextureRole::Specular),
        APPEARANCE_ROLE_METALLIC => Some(TextureRole::Metallic),
        APPEARANCE_ROLE_ROUGHNESS => Some(TextureRole::Roughness),
        APPEARANCE_ROLE_METALLIC_ROUGHNESS => Some(TextureRole::MetallicRoughness),
        APPEARANCE_ROLE_NORMAL => Some(TextureRole::Normal),
        APPEARANCE_ROLE_OCCLUSION => Some(TextureRole::Occlusion),
        APPEARANCE_ROLE_EMISSIVE => Some(TextureRole::Emissive),
        APPEARANCE_ROLE_OPACITY => Some(TextureRole::Opacity),
        APPEARANCE_ROLE_DISPLACEMENT => Some(TextureRole::Displacement),
        APPEARANCE_ROLE_CLEARCOAT => Some(TextureRole::Clearcoat),
        APPEARANCE_ROLE_CLEARCOAT_ROUGHNESS => Some(TextureRole::ClearcoatRoughness),
        APPEARANCE_ROLE_CLEARCOAT_NORMAL => Some(TextureRole::ClearcoatNormal),
        APPEARANCE_ROLE_TRANSMISSION => Some(TextureRole::Transmission),
        APPEARANCE_ROLE_THICKNESS => Some(TextureRole::Thickness),
        APPEARANCE_ROLE_SHEEN_COLOR => Some(TextureRole::SheenColor),
        APPEARANCE_ROLE_SHEEN_ROUGHNESS => Some(TextureRole::SheenRoughness),
        _ => None,
    }
}

pub(in super::super) const fn encode_appearance_shading_model(model: &ShadingModel) -> Option<u8> {
    match model {
        ShadingModel::Unspecified => Some(APPEARANCE_SHADING_UNSPECIFIED),
        ShadingModel::Unlit => Some(APPEARANCE_SHADING_UNLIT),
        ShadingModel::Lambert => Some(APPEARANCE_SHADING_LAMBERT),
        ShadingModel::Phong => Some(APPEARANCE_SHADING_PHONG),
        ShadingModel::BlinnPhong => Some(APPEARANCE_SHADING_BLINN_PHONG),
        ShadingModel::PbrMetallicRoughness => Some(APPEARANCE_SHADING_PBR_METALLIC_ROUGHNESS),
        ShadingModel::PbrSpecularGlossiness => Some(APPEARANCE_SHADING_PBR_SPECULAR_GLOSSINESS),
        ShadingModel::Namespaced { .. } => None,
    }
}

pub(in super::super) const fn decode_appearance_shading_model(tag: u8) -> Option<ShadingModel> {
    match tag {
        APPEARANCE_SHADING_UNSPECIFIED => Some(ShadingModel::Unspecified),
        APPEARANCE_SHADING_UNLIT => Some(ShadingModel::Unlit),
        APPEARANCE_SHADING_LAMBERT => Some(ShadingModel::Lambert),
        APPEARANCE_SHADING_PHONG => Some(ShadingModel::Phong),
        APPEARANCE_SHADING_BLINN_PHONG => Some(ShadingModel::BlinnPhong),
        APPEARANCE_SHADING_PBR_METALLIC_ROUGHNESS => Some(ShadingModel::PbrMetallicRoughness),
        APPEARANCE_SHADING_PBR_SPECULAR_GLOSSINESS => Some(ShadingModel::PbrSpecularGlossiness),
        _ => None,
    }
}

pub(in super::super) const fn encode_appearance_alpha_mode(mode: &AlphaMode) -> Option<u8> {
    match mode {
        AlphaMode::Opaque => Some(APPEARANCE_ALPHA_OPAQUE),
        AlphaMode::Mask(_) => Some(APPEARANCE_ALPHA_MASK),
        AlphaMode::Blend => Some(APPEARANCE_ALPHA_BLEND),
        AlphaMode::Namespaced { .. } => None,
    }
}

pub(in super::super) fn decode_appearance_alpha_mode(tag: u8, cutoff: f32) -> Option<AlphaMode> {
    match tag {
        APPEARANCE_ALPHA_OPAQUE if cutoff.to_bits() == 0 => Some(AlphaMode::Opaque),
        APPEARANCE_ALPHA_MASK => AlphaMode::mask(cutoff).ok(),
        APPEARANCE_ALPHA_BLEND if cutoff.to_bits() == 0 => Some(AlphaMode::Blend),
        _ => None,
    }
}

pub(in super::super) const fn encode_appearance_coordinate_source(
    source: &TextureCoordinateSource,
) -> Option<(u8, u32)> {
    match source {
        TextureCoordinateSource::UvSet(set) => Some((APPEARANCE_COORDINATE_UV_SET, *set as u32)),
        TextureCoordinateSource::ObjectPosition => Some((APPEARANCE_COORDINATE_OBJECT_POSITION, 0)),
        TextureCoordinateSource::ObjectNormal => Some((APPEARANCE_COORDINATE_OBJECT_NORMAL, 0)),
        TextureCoordinateSource::ReflectionVector => {
            Some((APPEARANCE_COORDINATE_REFLECTION_VECTOR, 0))
        }
        TextureCoordinateSource::Namespaced { .. } => None,
    }
}

pub(in super::super) fn decode_appearance_coordinate_source(
    tag: u8,
    set: u32,
) -> Option<TextureCoordinateSource> {
    match tag {
        APPEARANCE_COORDINATE_UV_SET => TextureCoordinateSource::uv_set(set).ok(),
        APPEARANCE_COORDINATE_OBJECT_POSITION if set == 0 => {
            Some(TextureCoordinateSource::ObjectPosition)
        }
        APPEARANCE_COORDINATE_OBJECT_NORMAL if set == 0 => {
            Some(TextureCoordinateSource::ObjectNormal)
        }
        APPEARANCE_COORDINATE_REFLECTION_VECTOR if set == 0 => {
            Some(TextureCoordinateSource::ReflectionVector)
        }
        _ => None,
    }
}

pub(in super::super) const fn encode_appearance_channel(channel: TextureChannel) -> u8 {
    match channel {
        TextureChannel::Red => APPEARANCE_CHANNEL_RED,
        TextureChannel::Green => APPEARANCE_CHANNEL_GREEN,
        TextureChannel::Blue => APPEARANCE_CHANNEL_BLUE,
        TextureChannel::Alpha => APPEARANCE_CHANNEL_ALPHA,
        TextureChannel::Zero => APPEARANCE_CHANNEL_ZERO,
        TextureChannel::One => APPEARANCE_CHANNEL_ONE,
    }
}

pub(in super::super) const fn decode_appearance_channel(tag: u8) -> Option<TextureChannel> {
    match tag {
        APPEARANCE_CHANNEL_RED => Some(TextureChannel::Red),
        APPEARANCE_CHANNEL_GREEN => Some(TextureChannel::Green),
        APPEARANCE_CHANNEL_BLUE => Some(TextureChannel::Blue),
        APPEARANCE_CHANNEL_ALPHA => Some(TextureChannel::Alpha),
        APPEARANCE_CHANNEL_ZERO => Some(TextureChannel::Zero),
        APPEARANCE_CHANNEL_ONE => Some(TextureChannel::One),
        _ => None,
    }
}

pub(in super::super) const fn encode_appearance_color_interpretation(
    interpretation: &ColorInterpretation,
) -> Option<u8> {
    match interpretation {
        ColorInterpretation::SrgbColor => Some(APPEARANCE_COLOR_SRGB),
        ColorInterpretation::LinearColor => Some(APPEARANCE_COLOR_LINEAR),
        ColorInterpretation::Data => Some(APPEARANCE_COLOR_DATA),
        ColorInterpretation::Namespaced { .. } => None,
    }
}

pub(in super::super) const fn decode_appearance_color_interpretation(
    tag: u8,
) -> Option<ColorInterpretation> {
    match tag {
        APPEARANCE_COLOR_SRGB => Some(ColorInterpretation::SrgbColor),
        APPEARANCE_COLOR_LINEAR => Some(ColorInterpretation::LinearColor),
        APPEARANCE_COLOR_DATA => Some(ColorInterpretation::Data),
        _ => None,
    }
}

pub(in super::super) const fn encode_appearance_blend_operation(
    blend: &TextureBlendOperation,
) -> Option<u8> {
    match blend {
        TextureBlendOperation::Replace => Some(APPEARANCE_BLEND_REPLACE),
        TextureBlendOperation::Multiply
        | TextureBlendOperation::Add
        | TextureBlendOperation::Subtract
        | TextureBlendOperation::Screen
        | TextureBlendOperation::Namespaced { .. } => None,
    }
}

pub(in super::super) const fn decode_appearance_blend_operation(
    tag: u8,
) -> Option<TextureBlendOperation> {
    match tag {
        APPEARANCE_BLEND_REPLACE => Some(TextureBlendOperation::Replace),
        _ => None,
    }
}

pub(in super::super) fn decode_appearance_binding_numeric_semantics(
    values: &[f32],
) -> Option<(TextureTransform, TextureStrength)> {
    if values.len() != APPEARANCE_BINDING_SEMANTIC_STRIDE {
        return None;
    }
    let transform = TextureTransform::try_new(
        Vec2::new(
            *values.get(appearance_binding_semantic_scalar::OFFSET_X)?,
            *values.get(appearance_binding_semantic_scalar::OFFSET_Y)?,
        ),
        *values.get(appearance_binding_semantic_scalar::ROTATION_RADIANS)?,
        Vec2::new(
            *values.get(appearance_binding_semantic_scalar::SCALE_X)?,
            *values.get(appearance_binding_semantic_scalar::SCALE_Y)?,
        ),
    )
    .ok()?;
    let strength =
        TextureStrength::try_new(*values.get(appearance_binding_semantic_scalar::STRENGTH)?)
            .ok()?;
    Some((transform, strength))
}

pub(in super::super) fn decode_appearance_material_factors(
    values: &[f32],
    pbr_metallic_roughness_presence: u8,
    pbr_specular_glossiness_presence: u8,
    legacy_presence: u8,
) -> Option<MaterialFactors> {
    if values.len() != APPEARANCE_MATERIAL_FACTOR_STRIDE {
        return None;
    }
    let emissive = Vec3::new(
        *values.get(appearance_material_factor_scalar::EMISSIVE_X)?,
        *values.get(appearance_material_factor_scalar::EMISSIVE_Y)?,
        *values.get(appearance_material_factor_scalar::EMISSIVE_Z)?,
    );
    let pbr_metallic_roughness = decode_optional_factor_group(
        values,
        pbr_metallic_roughness_presence,
        appearance_material_factor_scalar::PBR_BASE_COLOR_R,
        appearance_material_factor_scalar::ROUGHNESS + 1,
        || {
            PbrMetallicRoughnessFactors::try_new(
                Color::linear_rgba(
                    values[appearance_material_factor_scalar::PBR_BASE_COLOR_R],
                    values[appearance_material_factor_scalar::PBR_BASE_COLOR_G],
                    values[appearance_material_factor_scalar::PBR_BASE_COLOR_B],
                    values[appearance_material_factor_scalar::PBR_BASE_COLOR_A],
                ),
                values[appearance_material_factor_scalar::METALLIC],
                values[appearance_material_factor_scalar::ROUGHNESS],
            )
            .ok()
        },
    )?;
    let pbr_specular_glossiness = decode_optional_factor_group(
        values,
        pbr_specular_glossiness_presence,
        appearance_material_factor_scalar::PBR_DIFFUSE_R,
        appearance_material_factor_scalar::GLOSSINESS + 1,
        || {
            PbrSpecularGlossinessFactors::try_new(
                Color::linear_rgba(
                    values[appearance_material_factor_scalar::PBR_DIFFUSE_R],
                    values[appearance_material_factor_scalar::PBR_DIFFUSE_G],
                    values[appearance_material_factor_scalar::PBR_DIFFUSE_B],
                    values[appearance_material_factor_scalar::PBR_DIFFUSE_A],
                ),
                Vec3::new(
                    values[appearance_material_factor_scalar::PBR_SPECULAR_X],
                    values[appearance_material_factor_scalar::PBR_SPECULAR_Y],
                    values[appearance_material_factor_scalar::PBR_SPECULAR_Z],
                ),
                values[appearance_material_factor_scalar::GLOSSINESS],
            )
            .ok()
        },
    )?;
    let legacy = decode_optional_factor_group(
        values,
        legacy_presence,
        appearance_material_factor_scalar::LEGACY_AMBIENT_X,
        appearance_material_factor_scalar::LEGACY_OPACITY + 1,
        || {
            LegacyMaterialFactors::try_new(
                Vec3::new(
                    values[appearance_material_factor_scalar::LEGACY_AMBIENT_X],
                    values[appearance_material_factor_scalar::LEGACY_AMBIENT_Y],
                    values[appearance_material_factor_scalar::LEGACY_AMBIENT_Z],
                ),
                Vec3::new(
                    values[appearance_material_factor_scalar::LEGACY_DIFFUSE_X],
                    values[appearance_material_factor_scalar::LEGACY_DIFFUSE_Y],
                    values[appearance_material_factor_scalar::LEGACY_DIFFUSE_Z],
                ),
                Vec3::new(
                    values[appearance_material_factor_scalar::LEGACY_SPECULAR_X],
                    values[appearance_material_factor_scalar::LEGACY_SPECULAR_Y],
                    values[appearance_material_factor_scalar::LEGACY_SPECULAR_Z],
                ),
                Vec3::new(
                    values[appearance_material_factor_scalar::LEGACY_EMISSIVE_X],
                    values[appearance_material_factor_scalar::LEGACY_EMISSIVE_Y],
                    values[appearance_material_factor_scalar::LEGACY_EMISSIVE_Z],
                ),
                values[appearance_material_factor_scalar::LEGACY_SHININESS],
                values[appearance_material_factor_scalar::LEGACY_OPACITY],
            )
            .ok()
        },
    )?;
    let supplemental = SupplementalPbrFactors::try_new(
        values[appearance_material_factor_scalar::CLEARCOAT],
        values[appearance_material_factor_scalar::CLEARCOAT_ROUGHNESS],
        values[appearance_material_factor_scalar::TRANSMISSION],
        values[appearance_material_factor_scalar::THICKNESS],
        Vec3::new(
            values[appearance_material_factor_scalar::SHEEN_COLOR_X],
            values[appearance_material_factor_scalar::SHEEN_COLOR_Y],
            values[appearance_material_factor_scalar::SHEEN_COLOR_Z],
        ),
        values[appearance_material_factor_scalar::SHEEN_ROUGHNESS],
    )
    .ok()?;
    MaterialFactors::try_new_with_supplemental_pbr(
        emissive,
        pbr_metallic_roughness,
        pbr_specular_glossiness,
        legacy,
        supplemental,
    )
    .ok()
}

fn decode_optional_factor_group<T>(
    values: &[f32],
    presence: u8,
    start: usize,
    end: usize,
    decode: impl FnOnce() -> Option<T>,
) -> Option<Option<T>> {
    match presence {
        APPEARANCE_PRESENCE_ABSENT => values
            .get(start..end)?
            .iter()
            .all(|value| value.to_bits() == 0)
            .then_some(None),
        APPEARANCE_PRESENCE_PRESENT => decode().map(Some),
        _ => None,
    }
}

pub(in super::super) mod appearance_directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const IMAGE_COUNT: usize = 1;
    pub(crate) const IMAGES: usize = 2;
    pub(crate) const SAMPLER_COUNT: usize = 3;
    pub(crate) const SAMPLERS: usize = 4;
    pub(crate) const TEXTURE_COUNT: usize = 5;
    pub(crate) const TEXTURES: usize = 6;
    pub(crate) const MATERIAL_COUNT: usize = 7;
    pub(crate) const MATERIALS: usize = 8;
    pub(crate) const BINDING_COUNT: usize = 9;
    pub(crate) const BINDINGS: usize = 10;
    pub(crate) const IMAGE_PAYLOAD_BYTE_COUNT: usize = 11;
    pub(crate) const IMAGE_PAYLOADS: usize = 12;
    pub(crate) const MEDIA_TYPE_BYTE_COUNT: usize = 13;
    pub(crate) const MEDIA_TYPES: usize = 14;
    pub(crate) const SOURCE_KEY_BYTE_COUNT: usize = 15;
    pub(crate) const SOURCE_KEYS: usize = 16;
    pub(crate) const BINDING_SEMANTIC_SCALAR_COUNT: usize = 17;
    pub(crate) const BINDING_SEMANTIC_SCALARS: usize = 18;
    pub(crate) const MATERIAL_FACTOR_SCALAR_COUNT: usize = 19;
    pub(crate) const MATERIAL_FACTOR_SCALARS: usize = 20;
    pub(crate) const MATERIAL_NAME_BYTE_COUNT: usize = 21;
    pub(crate) const MATERIAL_NAMES: usize = 22;
}

pub(in super::super) mod appearance_image_field {
    pub(crate) const IMAGE: usize = 0;
    pub(crate) const PAYLOAD_KIND: usize = 1;
    pub(crate) const PAYLOAD_OFFSET: usize = 2;
    pub(crate) const PAYLOAD_COUNT: usize = 3;
    pub(crate) const PAYLOADS: usize = 4;
    pub(crate) const MEDIA_TYPE_PRESENCE: usize = 5;
    pub(crate) const MEDIA_TYPE_ENCODING: usize = 6;
    pub(crate) const MEDIA_TYPE_OFFSET: usize = 7;
    pub(crate) const MEDIA_TYPE_COUNT: usize = 8;
    pub(crate) const MEDIA_TYPES: usize = 9;
}

pub(in super::super) mod appearance_sampler_field {
    pub(crate) const SAMPLER: usize = 0;
    pub(crate) const MINIFICATION: usize = 1;
    pub(crate) const MAGNIFICATION: usize = 2;
    pub(crate) const MIPMAP: usize = 3;
    pub(crate) const ADDRESS_U: usize = 4;
    pub(crate) const ADDRESS_V: usize = 5;
    pub(crate) const ADDRESS_W: usize = 6;
}

pub(in super::super) mod appearance_texture_field {
    pub(crate) const TEXTURE: usize = 0;
    pub(crate) const IMAGE: usize = 1;
    pub(crate) const IMAGES: usize = 2;
    pub(crate) const SAMPLER: usize = 3;
    pub(crate) const SAMPLERS: usize = 4;
}

pub(in super::super) mod appearance_material_field {
    pub(crate) const MATERIAL: usize = 0;
    pub(crate) const SHADING_MODEL: usize = 1;
    pub(crate) const ALPHA_MODE: usize = 2;
    pub(crate) const ALPHA_CUTOFF: usize = 3;
    pub(crate) const DOUBLE_SIDED: usize = 4;
    pub(crate) const PBR_METALLIC_ROUGHNESS_PRESENCE: usize = 5;
    pub(crate) const PBR_SPECULAR_GLOSSINESS_PRESENCE: usize = 6;
    pub(crate) const LEGACY_PRESENCE: usize = 7;
    pub(crate) const BINDING_OFFSET: usize = 8;
    pub(crate) const BINDING_COUNT: usize = 9;
    pub(crate) const BINDINGS: usize = 10;
    pub(crate) const FACTOR_OFFSET: usize = 11;
    pub(crate) const FACTOR_COUNT: usize = 12;
    pub(crate) const FACTORS: usize = 13;
    pub(crate) const NAME_PRESENCE: usize = 14;
    pub(crate) const NAME_OFFSET: usize = 15;
    pub(crate) const NAME_COUNT: usize = 16;
    pub(crate) const NAMES: usize = 17;
}

pub(in super::super) mod appearance_binding_field {
    pub(crate) const BINDING: usize = 0;
    pub(crate) const TEXTURE: usize = 1;
    pub(crate) const TEXTURES: usize = 2;
    pub(crate) const ROLE: usize = 3;
    pub(crate) const COORDINATE_SOURCE: usize = 4;
    pub(crate) const COORDINATE_SET: usize = 5;
    pub(crate) const CHANNEL_LANE_COUNT: usize = 6;
    pub(crate) const CHANNEL_0: usize = 7;
    pub(crate) const CHANNEL_1: usize = 8;
    pub(crate) const CHANNEL_2: usize = 9;
    pub(crate) const CHANNEL_3: usize = 10;
    pub(crate) const COLOR_INTERPRETATION: usize = 11;
    pub(crate) const SEMANTIC_SCALAR_OFFSET: usize = 12;
    pub(crate) const SEMANTIC_SCALAR_COUNT: usize = 13;
    pub(crate) const SEMANTIC_SCALARS: usize = 14;
    pub(crate) const BLEND_OPERATION: usize = 15;
    pub(crate) const SOURCE_KEY_OFFSET: usize = 16;
    pub(crate) const SOURCE_KEY_COUNT: usize = 17;
    pub(crate) const SOURCE_KEYS: usize = 18;
}

pub(in super::super) mod appearance_binding_semantic_scalar {
    pub(crate) const OFFSET_X: usize = 0;
    pub(crate) const OFFSET_Y: usize = 1;
    pub(crate) const ROTATION_RADIANS: usize = 2;
    pub(crate) const SCALE_X: usize = 3;
    pub(crate) const SCALE_Y: usize = 4;
    pub(crate) const STRENGTH: usize = 5;
}

pub(in super::super) mod appearance_material_factor_scalar {
    pub(crate) const EMISSIVE_X: usize = 0;
    pub(crate) const EMISSIVE_Y: usize = 1;
    pub(crate) const EMISSIVE_Z: usize = 2;
    pub(crate) const PBR_BASE_COLOR_R: usize = 3;
    pub(crate) const PBR_BASE_COLOR_G: usize = 4;
    pub(crate) const PBR_BASE_COLOR_B: usize = 5;
    pub(crate) const PBR_BASE_COLOR_A: usize = 6;
    pub(crate) const METALLIC: usize = 7;
    pub(crate) const ROUGHNESS: usize = 8;
    pub(crate) const PBR_DIFFUSE_R: usize = 9;
    pub(crate) const PBR_DIFFUSE_G: usize = 10;
    pub(crate) const PBR_DIFFUSE_B: usize = 11;
    pub(crate) const PBR_DIFFUSE_A: usize = 12;
    pub(crate) const PBR_SPECULAR_X: usize = 13;
    pub(crate) const PBR_SPECULAR_Y: usize = 14;
    pub(crate) const PBR_SPECULAR_Z: usize = 15;
    pub(crate) const GLOSSINESS: usize = 16;
    pub(crate) const LEGACY_AMBIENT_X: usize = 17;
    pub(crate) const LEGACY_AMBIENT_Y: usize = 18;
    pub(crate) const LEGACY_AMBIENT_Z: usize = 19;
    pub(crate) const LEGACY_DIFFUSE_X: usize = 20;
    pub(crate) const LEGACY_DIFFUSE_Y: usize = 21;
    pub(crate) const LEGACY_DIFFUSE_Z: usize = 22;
    pub(crate) const LEGACY_SPECULAR_X: usize = 23;
    pub(crate) const LEGACY_SPECULAR_Y: usize = 24;
    pub(crate) const LEGACY_SPECULAR_Z: usize = 25;
    pub(crate) const LEGACY_EMISSIVE_X: usize = 26;
    pub(crate) const LEGACY_EMISSIVE_Y: usize = 27;
    pub(crate) const LEGACY_EMISSIVE_Z: usize = 28;
    pub(crate) const LEGACY_SHININESS: usize = 29;
    pub(crate) const LEGACY_OPACITY: usize = 30;
    pub(crate) const CLEARCOAT: usize = 31;
    pub(crate) const CLEARCOAT_ROUGHNESS: usize = 32;
    pub(crate) const TRANSMISSION: usize = 33;
    pub(crate) const THICKNESS: usize = 34;
    pub(crate) const SHEEN_COLOR_X: usize = 35;
    pub(crate) const SHEEN_COLOR_Y: usize = 36;
    pub(crate) const SHEEN_COLOR_Z: usize = 37;
    pub(crate) const SHEEN_ROUGHNESS: usize = 38;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceDirectorySchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceImageTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceSamplerTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceTextureTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceMaterialTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceBindingTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceEncodedImagePayloadSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceMediaTypeSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceSourceKeySchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceMaterialNameSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceBindingSemanticScalarSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AppearanceMaterialFactorScalarSchema;

impl CanonicalSchema for AppearanceDirectorySchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AppearanceImageTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AppearanceSamplerTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AppearanceTextureTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AppearanceMaterialTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AppearanceBindingTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AppearanceEncodedImagePayloadSchema {
    type StoreKind = ByteBlock;
}

impl CanonicalSchema for AppearanceMediaTypeSchema {
    type StoreKind = Utf8Block;
}

impl CanonicalSchema for AppearanceSourceKeySchema {
    type StoreKind = Utf8Block;
}

impl CanonicalSchema for AppearanceMaterialNameSchema {
    type StoreKind = Utf8Block;
}

impl CanonicalSchema for AppearanceBindingSemanticScalarSchema {
    type StoreKind = ScalarSequence<f32>;
}

impl CanonicalSchema for AppearanceMaterialFactorScalarSchema {
    type StoreKind = ScalarSequence<f32>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_appearance_tags_round_trip_through_explicit_codecs() {
        for tag in APPEARANCE_ROLE_BASE_COLOR..=APPEARANCE_ROLE_SHEEN_ROUGHNESS {
            let role =
                decode_appearance_texture_role(tag).expect("built-in role tag should decode");
            assert_eq!(encode_appearance_texture_role(&role), Some(tag));
        }
        for tag in APPEARANCE_SHADING_UNLIT..=APPEARANCE_SHADING_PBR_SPECULAR_GLOSSINESS {
            let model =
                decode_appearance_shading_model(tag).expect("built-in shading tag should decode");
            assert_eq!(encode_appearance_shading_model(&model), Some(tag));
        }
        let unspecified = decode_appearance_shading_model(APPEARANCE_SHADING_UNSPECIFIED)
            .expect("unspecified shading tag should decode");
        assert_eq!(unspecified, ShadingModel::Unspecified);
        assert_eq!(
            encode_appearance_shading_model(&unspecified),
            Some(APPEARANCE_SHADING_UNSPECIFIED)
        );
        for tag in APPEARANCE_CHANNEL_RED..=APPEARANCE_CHANNEL_ONE {
            let channel = decode_appearance_channel(tag).expect("channel tag should decode");
            assert_eq!(encode_appearance_channel(channel), tag);
        }
        for tag in APPEARANCE_COLOR_SRGB..=APPEARANCE_COLOR_DATA {
            let interpretation = decode_appearance_color_interpretation(tag)
                .expect("color interpretation tag should decode");
            assert_eq!(
                encode_appearance_color_interpretation(&interpretation),
                Some(tag)
            );
        }
    }

    #[test]
    fn appearance_decoders_reject_noncanonical_payload_combinations() {
        assert!(decode_appearance_texture_role(u8::MAX).is_none());
        assert!(decode_appearance_alpha_mode(APPEARANCE_ALPHA_OPAQUE, -0.0).is_none());
        assert!(decode_appearance_alpha_mode(APPEARANCE_ALPHA_BLEND, 0.5).is_none());
        assert!(
            decode_appearance_coordinate_source(APPEARANCE_COORDINATE_OBJECT_NORMAL, 1).is_none()
        );
        assert!(decode_appearance_channel(u8::MAX).is_none());
        assert!(decode_appearance_blend_operation(u8::MAX).is_none());
    }

    #[test]
    fn dense_appearance_payload_decoders_enforce_unique_absence() {
        let binding = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert_eq!(
            decode_appearance_binding_numeric_semantics(&binding),
            Some((TextureTransform::IDENTITY, TextureStrength::ONE))
        );

        let mut factors = [0.0; APPEARANCE_MATERIAL_FACTOR_STRIDE];
        assert_eq!(
            decode_appearance_material_factors(
                &factors,
                APPEARANCE_PRESENCE_ABSENT,
                APPEARANCE_PRESENCE_ABSENT,
                APPEARANCE_PRESENCE_ABSENT,
            ),
            Some(MaterialFactors::empty())
        );
        factors[appearance_material_factor_scalar::PBR_BASE_COLOR_R] = -0.0;
        assert!(
            decode_appearance_material_factors(
                &factors,
                APPEARANCE_PRESENCE_ABSENT,
                APPEARANCE_PRESENCE_ABSENT,
                APPEARANCE_PRESENCE_ABSENT,
            )
            .is_none()
        );
    }
}
