use crate::math::Color;
use crate::scene::{MetadataMap, TextureId};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadingModel {
    Unknown,
    Unlit,
    Phong,
    Blinn,
    PbrMetallicRoughness,
    PbrSpecularGlossiness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    Linear,
    Srgb,
    Data,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureRole {
    BaseColor,
    Diffuse,
    Specular,
    Metallic,
    Roughness,
    MetallicRoughness,
    Normal,
    Occlusion,
    Emissive,
    Opacity,
    Height,
    Displacement,
    Lightmap,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextureSlot {
    pub texture: TextureId,
    pub role: TextureRole,
    pub color_space: ColorSpace,
    pub uv_set: u32,
    pub scale: f32,
    pub source_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextureSource {
    External {
        uri: String,
    },
    Embedded {
        bytes: Arc<[u8]>,
        mime_type: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub name: Option<String>,
    pub source: TextureSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: Option<String>,
    pub shading_model: ShadingModel,
    pub base_color: Color,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Color,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub texture_slots: Vec<TextureSlot>,
    pub metadata: MetadataMap,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: None,
            shading_model: ShadingModel::Unknown,
            base_color: Color::WHITE,
            metallic: 0.0,
            roughness: 1.0,
            emissive: Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            texture_slots: Vec::new(),
            metadata: MetadataMap::new(),
        }
    }
}
