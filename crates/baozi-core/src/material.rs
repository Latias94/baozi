use crate::math::{Color, Vec2, Vec3, Vec4};
use crate::scene::{MetadataMap, TextureId};
use std::collections::BTreeMap;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextureWrapMode {
    #[default]
    Repeat,
    MirroredRepeat,
    ClampToEdge,
    ClampToBorder,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFilterMode {
    Nearest,
    Linear,
    NearestMipmapNearest,
    LinearMipmapNearest,
    NearestMipmapLinear,
    LinearMipmapLinear,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureSampler {
    pub mag_filter: Option<TextureFilterMode>,
    pub min_filter: Option<TextureFilterMode>,
    pub wrap_s: TextureWrapMode,
    pub wrap_t: TextureWrapMode,
    pub wrap_r: TextureWrapMode,
}

impl Default for TextureSampler {
    fn default() -> Self {
        Self {
            mag_filter: None,
            min_filter: None,
            wrap_s: TextureWrapMode::Repeat,
            wrap_t: TextureWrapMode::Repeat,
            wrap_r: TextureWrapMode::Repeat,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureTransform {
    pub offset: Vec2,
    pub rotation_radians: f32,
    pub scale: Vec2,
    pub texcoord: Option<u32>,
}

impl Default for TextureTransform {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            rotation_radians: 0.0,
            scale: Vec2::new(1.0, 1.0),
            texcoord: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextureSlot {
    pub texture: TextureId,
    pub role: TextureRole,
    pub color_space: ColorSpace,
    pub uv_set: u32,
    pub scale: f32,
    pub transform: TextureTransform,
    pub source_key: Option<String>,
}

impl Default for TextureSlot {
    fn default() -> Self {
        Self {
            texture: TextureId::new(0),
            role: TextureRole::Unknown,
            color_space: ColorSpace::Unknown,
            uv_set: 0,
            scale: 1.0,
            transform: TextureTransform::default(),
            source_key: None,
        }
    }
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
    pub sampler: TextureSampler,
    pub metadata: MetadataMap,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MaterialProperty {
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    Color(Color),
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
    Texture(TextureId),
}

pub type MaterialPropertyMap = BTreeMap<String, MaterialProperty>;

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
    pub properties: MaterialPropertyMap,
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
            properties: MaterialPropertyMap::new(),
            metadata: MetadataMap::new(),
        }
    }
}
