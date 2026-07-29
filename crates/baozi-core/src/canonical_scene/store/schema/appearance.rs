//! Private canonical Scene appearance schema.
//!
//! Appearance is deliberately a separate retained domain. Images own encoded
//! source payload, samplers own behavior, textures pair those identities, and
//! materials own their flat texture-binding ranges. The row layouts below make
//! every cross-table relationship explicit for central validation and snapshot
//! traversal.

use baozi_operation::{ByteBlock, RecordBlock, Utf8Block};

use crate::appearance::TextureRole;
use crate::image::{MipmapFilter, TexelFilter, TextureAddressMode};

use super::CanonicalSchema;

pub(in super::super) const APPEARANCE_SCHEMA_REVISION: u32 = 1;

pub(in super::super) const APPEARANCE_DIRECTORY_FIELD_COUNT: usize = 15;
pub(in super::super) const APPEARANCE_IMAGE_FIELD_COUNT: usize = 10;
pub(in super::super) const APPEARANCE_SAMPLER_FIELD_COUNT: usize = 7;
pub(in super::super) const APPEARANCE_TEXTURE_FIELD_COUNT: usize = 5;
pub(in super::super) const APPEARANCE_MATERIAL_FIELD_COUNT: usize = 4;
pub(in super::super) const APPEARANCE_BINDING_FIELD_COUNT: usize = 4;

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
pub(in super::super) const APPEARANCE_ROLE_HEIGHT: u8 = 10;
pub(in super::super) const APPEARANCE_ROLE_DISPLACEMENT: u8 = 11;
pub(in super::super) const APPEARANCE_ROLE_LIGHTMAP: u8 = 12;
pub(in super::super) const APPEARANCE_ROLE_CLEARCOAT: u8 = 13;
pub(in super::super) const APPEARANCE_ROLE_CLEARCOAT_ROUGHNESS: u8 = 14;
pub(in super::super) const APPEARANCE_ROLE_CLEARCOAT_NORMAL: u8 = 15;
pub(in super::super) const APPEARANCE_ROLE_TRANSMISSION: u8 = 16;
pub(in super::super) const APPEARANCE_ROLE_THICKNESS: u8 = 17;
pub(in super::super) const APPEARANCE_ROLE_SHEEN_COLOR: u8 = 18;
pub(in super::super) const APPEARANCE_ROLE_SHEEN_ROUGHNESS: u8 = 19;

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
        TextureRole::Height => Some(APPEARANCE_ROLE_HEIGHT),
        TextureRole::Displacement => Some(APPEARANCE_ROLE_DISPLACEMENT),
        TextureRole::Lightmap => Some(APPEARANCE_ROLE_LIGHTMAP),
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
    pub(crate) const BINDING_OFFSET: usize = 1;
    pub(crate) const BINDING_COUNT: usize = 2;
    pub(crate) const BINDINGS: usize = 3;
}

pub(in super::super) mod appearance_binding_field {
    pub(crate) const BINDING: usize = 0;
    pub(crate) const TEXTURE: usize = 1;
    pub(crate) const TEXTURES: usize = 2;
    pub(crate) const ROLE: usize = 3;
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
