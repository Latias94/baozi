//! Canonical material factors and texture-usage semantics.
//!
//! Retained scene storage owns identity, provenance, and collection layout.
//! This module defines validated appearance values and the borrowed semantic
//! views consumed while producing that storage.

use crate::extension::ExtensionSchemaId;
use crate::math::{Color, Vec2, Vec3};
use std::collections::HashMap;
use std::fmt;
use thiserror::Error;

/// Maximum UTF-8 bytes in one format-owned texture source key.
pub const MAX_TEXTURE_SOURCE_KEY_BYTES: usize = 128;

/// Maximum zero-based UV set address admitted by the canonical binding model.
pub const MAX_TEXTURE_COORDINATE_SET: u32 = u16::MAX as u32;

/// A behavior category whose namespaced form requires a declarative schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppearanceBehaviorKind {
    ShadingModel,
    AlphaMode,
    TextureRole,
    CoordinateSource,
    ChannelMapping,
    ColorInterpretation,
    BlendOperation,
}

/// A behavior-bearing extension was requested without schema authority.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceBehaviorError {
    #[error("namespaced {behavior:?} requires an admitted extension schema")]
    SchemaRequired { behavior: AppearanceBehaviorKind },
}

fn require_behavior_schema(
    schema_id: Option<ExtensionSchemaId>,
    behavior: AppearanceBehaviorKind,
) -> Result<ExtensionSchemaId, AppearanceBehaviorError> {
    schema_id.ok_or(AppearanceBehaviorError::SchemaRequired { behavior })
}

/// The primary rendering model selected for one material.
///
/// Factor groups are retained independently, so selecting a PBR model never
/// deletes legacy source truth that was also present.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ShadingModel {
    /// The source does not declare a rendering model.
    Unspecified,
    Unlit,
    Lambert,
    Phong,
    BlinnPhong,
    PbrMetallicRoughness,
    PbrSpecularGlossiness,
    Namespaced {
        schema_id: ExtensionSchemaId,
    },
}

impl ShadingModel {
    /// Creates a source-specific shading behavior only with schema authority.
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AppearanceBehaviorError> {
        let schema_id = require_behavior_schema(schema_id, AppearanceBehaviorKind::ShadingModel)?;
        Ok(Self::Namespaced { schema_id })
    }

    /// Returns the schema required by a namespaced behavior.
    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Unspecified
            | Self::Unlit
            | Self::Lambert
            | Self::Phong
            | Self::BlinnPhong
            | Self::PbrMetallicRoughness
            | Self::PbrSpecularGlossiness => None,
        }
    }

    pub const fn kind(&self) -> ShadingModelKind {
        match self {
            Self::Unspecified => ShadingModelKind::Unspecified,
            Self::Unlit => ShadingModelKind::Unlit,
            Self::Lambert => ShadingModelKind::Lambert,
            Self::Phong => ShadingModelKind::Phong,
            Self::BlinnPhong => ShadingModelKind::BlinnPhong,
            Self::PbrMetallicRoughness => ShadingModelKind::PbrMetallicRoughness,
            Self::PbrSpecularGlossiness => ShadingModelKind::PbrSpecularGlossiness,
            Self::Namespaced { .. } => ShadingModelKind::Namespaced,
        }
    }
}

/// A text-free shading category suitable for structured errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadingModelKind {
    Unspecified,
    Unlit,
    Lambert,
    Phong,
    BlinnPhong,
    PbrMetallicRoughness,
    PbrSpecularGlossiness,
    Namespaced,
}

/// A validated alpha-test cutoff in the closed unit interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlphaCutoff(f32);

impl AlphaCutoff {
    pub fn try_new(value: f32) -> Result<Self, MaterialFactorError> {
        validate_unit_factor(value, MaterialFactorField::AlphaCutoff)?;
        Ok(Self(value))
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

/// Canonical material alpha behavior.
#[derive(Debug, Clone, PartialEq)]
pub enum AlphaMode {
    Opaque,
    Mask(AlphaCutoff),
    Blend,
    Namespaced { schema_id: ExtensionSchemaId },
}

impl AlphaMode {
    pub fn mask(cutoff: f32) -> Result<Self, MaterialFactorError> {
        AlphaCutoff::try_new(cutoff).map(Self::Mask)
    }

    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AppearanceBehaviorError> {
        let schema_id = require_behavior_schema(schema_id, AppearanceBehaviorKind::AlphaMode)?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn mask_cutoff(&self) -> Option<f32> {
        match self {
            Self::Mask(cutoff) => Some(cutoff.get()),
            Self::Opaque | Self::Blend | Self::Namespaced { .. } => None,
        }
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Opaque | Self::Mask(_) | Self::Blend => None,
        }
    }
}

/// A numeric material field validated at a typed construction boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialFactorField {
    AlphaCutoff,
    Emissive,
    PbrBaseColor,
    Metallic,
    Roughness,
    PbrDiffuse,
    PbrSpecular,
    Glossiness,
    LegacyAmbient,
    LegacyDiffuse,
    LegacySpecular,
    LegacyEmissive,
    LegacyShininess,
    LegacyOpacity,
    Clearcoat,
    ClearcoatRoughness,
    Transmission,
    Thickness,
    SheenColor,
    SheenRoughness,
}

/// Why a typed material factor was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MaterialFactorError {
    #[error("material factor {field:?} contains a non-finite component")]
    NonFinite { field: MaterialFactorField },

    #[error("material factor {field:?} contains a negative component")]
    Negative { field: MaterialFactorField },

    #[error("material factor {field:?} is outside the closed unit interval")]
    OutsideUnitInterval { field: MaterialFactorField },
}

/// glTF-style metallic/roughness factors with no texture identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PbrMetallicRoughnessFactors {
    base_color: Color,
    metallic: f32,
    roughness: f32,
}

impl PbrMetallicRoughnessFactors {
    pub fn try_new(
        base_color: Color,
        metallic: f32,
        roughness: f32,
    ) -> Result<Self, MaterialFactorError> {
        validate_unit_color(base_color, MaterialFactorField::PbrBaseColor)?;
        validate_unit_factor(metallic, MaterialFactorField::Metallic)?;
        validate_unit_factor(roughness, MaterialFactorField::Roughness)?;
        Ok(Self {
            base_color,
            metallic,
            roughness,
        })
    }

    pub const fn base_color(self) -> Color {
        self.base_color
    }

    pub const fn metallic(self) -> f32 {
        self.metallic
    }

    pub const fn roughness(self) -> f32 {
        self.roughness
    }
}

impl Default for PbrMetallicRoughnessFactors {
    fn default() -> Self {
        Self {
            base_color: Color::WHITE,
            metallic: 1.0,
            roughness: 1.0,
        }
    }
}

/// PBR specular/glossiness factors retained independently from metallic PBR.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PbrSpecularGlossinessFactors {
    diffuse: Color,
    specular: Vec3,
    glossiness: f32,
}

impl PbrSpecularGlossinessFactors {
    pub fn try_new(
        diffuse: Color,
        specular: Vec3,
        glossiness: f32,
    ) -> Result<Self, MaterialFactorError> {
        validate_unit_color(diffuse, MaterialFactorField::PbrDiffuse)?;
        validate_unit_vec3(specular, MaterialFactorField::PbrSpecular)?;
        validate_unit_factor(glossiness, MaterialFactorField::Glossiness)?;
        Ok(Self {
            diffuse,
            specular,
            glossiness,
        })
    }

    pub const fn diffuse(self) -> Color {
        self.diffuse
    }

    pub const fn specular(self) -> Vec3 {
        self.specular
    }

    pub const fn glossiness(self) -> f32 {
        self.glossiness
    }
}

/// Source-authored legacy lighting factors.
///
/// RGB components may exceed one to preserve finite HDR source truth. Opacity
/// remains a unit factor, while shininess is finite and non-negative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LegacyMaterialFactors {
    ambient: Vec3,
    diffuse: Vec3,
    specular: Vec3,
    emissive: Vec3,
    shininess: f32,
    opacity: f32,
}

impl LegacyMaterialFactors {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        ambient: Vec3,
        diffuse: Vec3,
        specular: Vec3,
        emissive: Vec3,
        shininess: f32,
        opacity: f32,
    ) -> Result<Self, MaterialFactorError> {
        validate_nonnegative_vec3(ambient, MaterialFactorField::LegacyAmbient)?;
        validate_nonnegative_vec3(diffuse, MaterialFactorField::LegacyDiffuse)?;
        validate_nonnegative_vec3(specular, MaterialFactorField::LegacySpecular)?;
        validate_nonnegative_vec3(emissive, MaterialFactorField::LegacyEmissive)?;
        validate_nonnegative_factor(shininess, MaterialFactorField::LegacyShininess)?;
        validate_unit_factor(opacity, MaterialFactorField::LegacyOpacity)?;
        Ok(Self {
            ambient,
            diffuse,
            specular,
            emissive,
            shininess,
            opacity,
        })
    }

    pub const fn ambient(self) -> Vec3 {
        self.ambient
    }

    pub const fn diffuse(self) -> Vec3 {
        self.diffuse
    }

    pub const fn specular(self) -> Vec3 {
        self.specular
    }

    pub const fn emissive(self) -> Vec3 {
        self.emissive
    }

    pub const fn shininess(self) -> f32 {
        self.shininess
    }

    pub const fn opacity(self) -> f32 {
        self.opacity
    }
}

/// Stable supplemental PBR factors shared by clearcoat, transmission, thickness, and sheen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SupplementalPbrFactors {
    clearcoat: f32,
    clearcoat_roughness: f32,
    transmission: f32,
    thickness: f32,
    sheen_color: Vec3,
    sheen_roughness: f32,
}

impl SupplementalPbrFactors {
    pub fn try_new(
        clearcoat: f32,
        clearcoat_roughness: f32,
        transmission: f32,
        thickness: f32,
        sheen_color: Vec3,
        sheen_roughness: f32,
    ) -> Result<Self, MaterialFactorError> {
        validate_unit_factor(clearcoat, MaterialFactorField::Clearcoat)?;
        validate_unit_factor(clearcoat_roughness, MaterialFactorField::ClearcoatRoughness)?;
        validate_unit_factor(transmission, MaterialFactorField::Transmission)?;
        validate_nonnegative_factor(thickness, MaterialFactorField::Thickness)?;
        validate_nonnegative_vec3(sheen_color, MaterialFactorField::SheenColor)?;
        validate_unit_factor(sheen_roughness, MaterialFactorField::SheenRoughness)?;
        Ok(Self {
            clearcoat,
            clearcoat_roughness,
            transmission,
            thickness,
            sheen_color,
            sheen_roughness,
        })
    }

    pub const fn clearcoat(self) -> f32 {
        self.clearcoat
    }

    pub const fn clearcoat_roughness(self) -> f32 {
        self.clearcoat_roughness
    }

    pub const fn transmission(self) -> f32 {
        self.transmission
    }

    pub const fn thickness(self) -> f32 {
        self.thickness
    }

    pub const fn sheen_color(self) -> Vec3 {
        self.sheen_color
    }

    pub const fn sheen_roughness(self) -> f32 {
        self.sheen_roughness
    }
}

impl Default for SupplementalPbrFactors {
    fn default() -> Self {
        Self {
            clearcoat: 0.0,
            clearcoat_roughness: 0.0,
            transmission: 0.0,
            thickness: 0.0,
            sheen_color: Vec3::ZERO,
            sheen_roughness: 0.0,
        }
    }
}

/// Independent typed factor groups retained by one material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialFactors {
    emissive: Vec3,
    pbr_metallic_roughness: Option<PbrMetallicRoughnessFactors>,
    pbr_specular_glossiness: Option<PbrSpecularGlossinessFactors>,
    legacy: Option<LegacyMaterialFactors>,
    supplemental_pbr: SupplementalPbrFactors,
}

impl MaterialFactors {
    pub fn try_new(
        emissive: Vec3,
        pbr_metallic_roughness: Option<PbrMetallicRoughnessFactors>,
        pbr_specular_glossiness: Option<PbrSpecularGlossinessFactors>,
        legacy: Option<LegacyMaterialFactors>,
    ) -> Result<Self, MaterialFactorError> {
        Self::try_new_with_supplemental_pbr(
            emissive,
            pbr_metallic_roughness,
            pbr_specular_glossiness,
            legacy,
            SupplementalPbrFactors::default(),
        )
    }

    pub fn try_new_with_supplemental_pbr(
        emissive: Vec3,
        pbr_metallic_roughness: Option<PbrMetallicRoughnessFactors>,
        pbr_specular_glossiness: Option<PbrSpecularGlossinessFactors>,
        legacy: Option<LegacyMaterialFactors>,
        supplemental_pbr: SupplementalPbrFactors,
    ) -> Result<Self, MaterialFactorError> {
        validate_nonnegative_vec3(emissive, MaterialFactorField::Emissive)?;
        Ok(Self {
            emissive,
            pbr_metallic_roughness,
            pbr_specular_glossiness,
            legacy,
            supplemental_pbr,
        })
    }

    pub const fn empty() -> Self {
        Self {
            emissive: Vec3::ZERO,
            pbr_metallic_roughness: None,
            pbr_specular_glossiness: None,
            legacy: None,
            supplemental_pbr: SupplementalPbrFactors {
                clearcoat: 0.0,
                clearcoat_roughness: 0.0,
                transmission: 0.0,
                thickness: 0.0,
                sheen_color: Vec3::ZERO,
                sheen_roughness: 0.0,
            },
        }
    }

    pub const fn emissive(self) -> Vec3 {
        self.emissive
    }

    pub const fn pbr_metallic_roughness(self) -> Option<PbrMetallicRoughnessFactors> {
        self.pbr_metallic_roughness
    }

    pub const fn pbr_specular_glossiness(self) -> Option<PbrSpecularGlossinessFactors> {
        self.pbr_specular_glossiness
    }

    pub const fn legacy(self) -> Option<LegacyMaterialFactors> {
        self.legacy
    }

    pub const fn supplemental_pbr(self) -> SupplementalPbrFactors {
        self.supplemental_pbr
    }
}

impl Default for MaterialFactors {
    fn default() -> Self {
        Self::empty()
    }
}

fn validate_unit_color(
    value: Color,
    field: MaterialFactorField,
) -> Result<(), MaterialFactorError> {
    for component in [value.r, value.g, value.b, value.a] {
        validate_unit_factor(component, field)?;
    }
    Ok(())
}

fn validate_unit_vec3(value: Vec3, field: MaterialFactorField) -> Result<(), MaterialFactorError> {
    for component in [value.x, value.y, value.z] {
        validate_unit_factor(component, field)?;
    }
    Ok(())
}

fn validate_nonnegative_vec3(
    value: Vec3,
    field: MaterialFactorField,
) -> Result<(), MaterialFactorError> {
    for component in [value.x, value.y, value.z] {
        validate_nonnegative_factor(component, field)?;
    }
    Ok(())
}

fn validate_unit_factor(value: f32, field: MaterialFactorField) -> Result<(), MaterialFactorError> {
    if !value.is_finite() {
        return Err(MaterialFactorError::NonFinite { field });
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(MaterialFactorError::OutsideUnitInterval { field });
    }
    Ok(())
}

fn validate_nonnegative_factor(
    value: f32,
    field: MaterialFactorField,
) -> Result<(), MaterialFactorError> {
    if !value.is_finite() {
        return Err(MaterialFactorError::NonFinite { field });
    }
    if value < 0.0 {
        return Err(MaterialFactorError::Negative { field });
    }
    Ok(())
}

/// A canonical or schema-backed material texture role.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    Displacement,
    Clearcoat,
    ClearcoatRoughness,
    ClearcoatNormal,
    Transmission,
    Thickness,
    SheenColor,
    SheenRoughness,
    Namespaced { schema_id: ExtensionSchemaId },
}

impl TextureRole {
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AppearanceBehaviorError> {
        let schema_id = require_behavior_schema(schema_id, AppearanceBehaviorKind::TextureRole)?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::BaseColor
            | Self::Diffuse
            | Self::Specular
            | Self::Metallic
            | Self::Roughness
            | Self::MetallicRoughness
            | Self::Normal
            | Self::Occlusion
            | Self::Emissive
            | Self::Opacity
            | Self::Displacement
            | Self::Clearcoat
            | Self::ClearcoatRoughness
            | Self::ClearcoatNormal
            | Self::Transmission
            | Self::Thickness
            | Self::SheenColor
            | Self::SheenRoughness => None,
        }
    }

    pub const fn kind(&self) -> TextureRoleKind {
        match self {
            Self::BaseColor => TextureRoleKind::BaseColor,
            Self::Diffuse => TextureRoleKind::Diffuse,
            Self::Specular => TextureRoleKind::Specular,
            Self::Metallic => TextureRoleKind::Metallic,
            Self::Roughness => TextureRoleKind::Roughness,
            Self::MetallicRoughness => TextureRoleKind::MetallicRoughness,
            Self::Normal => TextureRoleKind::Normal,
            Self::Occlusion => TextureRoleKind::Occlusion,
            Self::Emissive => TextureRoleKind::Emissive,
            Self::Opacity => TextureRoleKind::Opacity,
            Self::Displacement => TextureRoleKind::Displacement,
            Self::Clearcoat => TextureRoleKind::Clearcoat,
            Self::ClearcoatRoughness => TextureRoleKind::ClearcoatRoughness,
            Self::ClearcoatNormal => TextureRoleKind::ClearcoatNormal,
            Self::Transmission => TextureRoleKind::Transmission,
            Self::Thickness => TextureRoleKind::Thickness,
            Self::SheenColor => TextureRoleKind::SheenColor,
            Self::SheenRoughness => TextureRoleKind::SheenRoughness,
            Self::Namespaced { .. } => TextureRoleKind::Namespaced,
        }
    }

    const fn usage_class(&self) -> TextureUsageClass {
        match self {
            Self::BaseColor
            | Self::Diffuse
            | Self::Specular
            | Self::Emissive
            | Self::SheenColor => TextureUsageClass::Color,
            Self::Metallic
            | Self::Roughness
            | Self::MetallicRoughness
            | Self::Normal
            | Self::Occlusion
            | Self::Opacity
            | Self::Displacement
            | Self::Clearcoat
            | Self::ClearcoatRoughness
            | Self::ClearcoatNormal
            | Self::Transmission
            | Self::Thickness
            | Self::SheenRoughness => TextureUsageClass::Data,
            Self::Namespaced { .. } => TextureUsageClass::Namespaced,
        }
    }

    const fn channel_lane_range(&self) -> Option<(u8, u8)> {
        match self {
            Self::BaseColor | Self::Diffuse => Some((3, 4)),
            Self::Specular => Some((3, 3)),
            Self::Emissive | Self::SheenColor => Some((3, 3)),
            Self::MetallicRoughness => Some((2, 2)),
            Self::Normal | Self::ClearcoatNormal => Some((2, 3)),
            Self::Metallic
            | Self::Roughness
            | Self::Occlusion
            | Self::Opacity
            | Self::Displacement
            | Self::Clearcoat
            | Self::ClearcoatRoughness
            | Self::Transmission
            | Self::Thickness
            | Self::SheenRoughness => Some((1, 1)),
            Self::Namespaced { .. } => None,
        }
    }
}

/// A text-free texture role category suitable for errors and snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureRoleKind {
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
    Displacement,
    Clearcoat,
    ClearcoatRoughness,
    ClearcoatNormal,
    Transmission,
    Thickness,
    SheenColor,
    SheenRoughness,
    Namespaced,
}

impl TextureRoleKind {
    pub(crate) const BUILTIN_COUNT: usize = 18;

    const fn builtin_index(self) -> Option<usize> {
        match self {
            Self::BaseColor => Some(0),
            Self::Diffuse => Some(1),
            Self::Specular => Some(2),
            Self::Metallic => Some(3),
            Self::Roughness => Some(4),
            Self::MetallicRoughness => Some(5),
            Self::Normal => Some(6),
            Self::Occlusion => Some(7),
            Self::Emissive => Some(8),
            Self::Opacity => Some(9),
            Self::Displacement => Some(10),
            Self::Clearcoat => Some(11),
            Self::ClearcoatRoughness => Some(12),
            Self::ClearcoatNormal => Some(13),
            Self::Transmission => Some(14),
            Self::Thickness => Some(15),
            Self::SheenColor => Some(16),
            Self::SheenRoughness => Some(17),
            Self::Namespaced => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextureUsageClass {
    Color,
    Data,
    Namespaced,
}

/// A coordinate source used to evaluate one texture binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TextureCoordinateSource {
    /// A zero-based UV set independent from any one primitive cardinality.
    UvSet(u16),
    ObjectPosition,
    ObjectNormal,
    ReflectionVector,
    Namespaced {
        schema_id: ExtensionSchemaId,
    },
}

impl TextureCoordinateSource {
    pub fn uv_set(set: u32) -> Result<Self, TextureCoordinateSourceError> {
        let set = u16::try_from(set).map_err(|_| TextureCoordinateSourceError::SetTooLarge {
            actual: set,
            maximum: MAX_TEXTURE_COORDINATE_SET,
        })?;
        Ok(Self::UvSet(set))
    }

    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AppearanceBehaviorError> {
        let schema_id =
            require_behavior_schema(schema_id, AppearanceBehaviorKind::CoordinateSource)?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn uv_set_index(&self) -> Option<u32> {
        match self {
            Self::UvSet(set) => Some(*set as u32),
            Self::ObjectPosition
            | Self::ObjectNormal
            | Self::ReflectionVector
            | Self::Namespaced { .. } => None,
        }
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::UvSet(_) | Self::ObjectPosition | Self::ObjectNormal | Self::ReflectionVector => {
                None
            }
        }
    }
}

/// Why a coordinate-set address was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TextureCoordinateSourceError {
    #[error("texture coordinate set {actual} exceeds canonical maximum {maximum}")]
    SetTooLarge { actual: u32, maximum: u32 },
}

/// One sampled source component or a canonical constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureChannel {
    Red,
    Green,
    Blue,
    Alpha,
    Zero,
    One,
}

/// A validated one-to-four-lane channel swizzle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureChannelSwizzle {
    channels: [TextureChannel; 4],
    lane_count: u8,
}

impl TextureChannelSwizzle {
    pub const RED: Self = Self {
        channels: [
            TextureChannel::Red,
            TextureChannel::Zero,
            TextureChannel::Zero,
            TextureChannel::Zero,
        ],
        lane_count: 1,
    };

    pub const RGB: Self = Self {
        channels: [
            TextureChannel::Red,
            TextureChannel::Green,
            TextureChannel::Blue,
            TextureChannel::Zero,
        ],
        lane_count: 3,
    };

    pub const RGBA: Self = Self {
        channels: [
            TextureChannel::Red,
            TextureChannel::Green,
            TextureChannel::Blue,
            TextureChannel::Alpha,
        ],
        lane_count: 4,
    };

    pub fn try_new(channels: &[TextureChannel]) -> Result<Self, TextureChannelMappingError> {
        if channels.is_empty() {
            return Err(TextureChannelMappingError::Empty);
        }
        if channels.len() > 4 {
            return Err(TextureChannelMappingError::TooManyLanes {
                actual: channels.len(),
                maximum: 4,
            });
        }

        let mut canonical = [TextureChannel::Zero; 4];
        canonical[..channels.len()].copy_from_slice(channels);
        let lane_count =
            u8::try_from(channels.len()).map_err(|_| TextureChannelMappingError::TooManyLanes {
                actual: channels.len(),
                maximum: 4,
            })?;
        Ok(Self {
            channels: canonical,
            lane_count,
        })
    }

    pub fn channels(&self) -> &[TextureChannel] {
        &self.channels[..usize::from(self.lane_count)]
    }

    pub const fn lane_count(self) -> u8 {
        self.lane_count
    }
}

/// Canonical channel selection or a declaratively schema-backed mapping.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TextureChannelMapping {
    Swizzle(TextureChannelSwizzle),
    Namespaced { schema_id: ExtensionSchemaId },
}

impl TextureChannelMapping {
    pub const fn swizzle(swizzle: TextureChannelSwizzle) -> Self {
        Self::Swizzle(swizzle)
    }

    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AppearanceBehaviorError> {
        let schema_id = require_behavior_schema(schema_id, AppearanceBehaviorKind::ChannelMapping)?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn lane_count(&self) -> Option<u8> {
        match self {
            Self::Swizzle(swizzle) => Some(swizzle.lane_count()),
            Self::Namespaced { .. } => None,
        }
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Swizzle(_) => None,
        }
    }
}

/// Why a fixed channel mapping was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TextureChannelMappingError {
    #[error("texture channel mapping must contain at least one output lane")]
    Empty,

    #[error("texture channel mapping has {actual} lanes; the maximum is {maximum}")]
    TooManyLanes { actual: usize, maximum: usize },
}

/// How sampled values are interpreted at one material usage site.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ColorInterpretation {
    SrgbColor,
    LinearColor,
    Data,
    Namespaced { schema_id: ExtensionSchemaId },
}

impl ColorInterpretation {
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AppearanceBehaviorError> {
        let schema_id =
            require_behavior_schema(schema_id, AppearanceBehaviorKind::ColorInterpretation)?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::SrgbColor | Self::LinearColor | Self::Data => None,
        }
    }

    pub const fn kind(&self) -> ColorInterpretationKind {
        match self {
            Self::SrgbColor => ColorInterpretationKind::SrgbColor,
            Self::LinearColor => ColorInterpretationKind::LinearColor,
            Self::Data => ColorInterpretationKind::Data,
            Self::Namespaced { .. } => ColorInterpretationKind::Namespaced,
        }
    }
}

/// A text-free sampled-value interpretation category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorInterpretationKind {
    SrgbColor,
    LinearColor,
    Data,
    Namespaced,
}

/// A finite two-dimensional texture coordinate transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureTransform {
    offset: Vec2,
    rotation_radians: f32,
    scale: Vec2,
}

impl TextureTransform {
    pub const IDENTITY: Self = Self {
        offset: Vec2::ZERO,
        rotation_radians: 0.0,
        scale: Vec2 { x: 1.0, y: 1.0 },
    };

    pub fn try_new(
        offset: Vec2,
        rotation_radians: f32,
        scale: Vec2,
    ) -> Result<Self, TextureTransformError> {
        if !offset.is_finite() {
            return Err(TextureTransformError::NonFinite {
                component: TextureTransformComponent::Offset,
            });
        }
        if !rotation_radians.is_finite() {
            return Err(TextureTransformError::NonFinite {
                component: TextureTransformComponent::Rotation,
            });
        }
        if !scale.is_finite() {
            return Err(TextureTransformError::NonFinite {
                component: TextureTransformComponent::Scale,
            });
        }
        Ok(Self {
            offset,
            rotation_radians,
            scale,
        })
    }

    pub const fn offset(self) -> Vec2 {
        self.offset
    }

    pub const fn rotation_radians(self) -> f32 {
        self.rotation_radians
    }

    pub const fn scale(self) -> Vec2 {
        self.scale
    }
}

impl Default for TextureTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// One component of a rejected texture transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureTransformComponent {
    Offset,
    Rotation,
    Scale,
}

/// Why a texture coordinate transform was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TextureTransformError {
    #[error("texture transform {component:?} contains a non-finite value")]
    NonFinite {
        component: TextureTransformComponent,
    },
}

/// A finite role-specific scale or strength.
///
/// The value is intentionally signed: displacement and normal-map conventions
/// can use sign, while unit-constrained concepts use typed material factors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureStrength(f32);

impl TextureStrength {
    pub const ONE: Self = Self(1.0);

    pub fn try_new(value: f32) -> Result<Self, TextureStrengthError> {
        if !value.is_finite() {
            return Err(TextureStrengthError::NonFinite);
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

/// Why a texture strength was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TextureStrengthError {
    #[error("texture strength must be finite")]
    NonFinite,
}

/// Ordered blend behavior for a texture binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TextureBlendOperation {
    Replace,
    Multiply,
    Add,
    Subtract,
    Screen,
    Namespaced { schema_id: ExtensionSchemaId },
}

impl TextureBlendOperation {
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AppearanceBehaviorError> {
        let schema_id = require_behavior_schema(schema_id, AppearanceBehaviorKind::BlendOperation)?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Replace | Self::Multiply | Self::Add | Self::Subtract | Self::Screen => None,
        }
    }
}

/// A bounded source-format property or slot key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureSourceKey {
    bytes: [u8; MAX_TEXTURE_SOURCE_KEY_BYTES],
    length: u8,
}

impl TextureSourceKey {
    pub fn try_new(value: impl AsRef<str>) -> Result<Self, TextureSourceKeyError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(TextureSourceKeyError::Empty);
        }
        if value.len() > MAX_TEXTURE_SOURCE_KEY_BYTES {
            return Err(TextureSourceKeyError::TooLong {
                actual: value.len(),
                maximum: MAX_TEXTURE_SOURCE_KEY_BYTES,
            });
        }
        if let Some((offset, _)) = value
            .char_indices()
            .find(|(_, character)| character.is_control())
        {
            return Err(TextureSourceKeyError::ControlCharacter { offset });
        }

        let mut bytes = [0; MAX_TEXTURE_SOURCE_KEY_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        let length = u8::try_from(value.len()).map_err(|_| TextureSourceKeyError::TooLong {
            actual: value.len(),
            maximum: MAX_TEXTURE_SOURCE_KEY_BYTES,
        })?;
        Ok(Self { bytes, length })
    }

    pub fn as_str(&self) -> &str {
        let length = usize::from(self.length);
        std::str::from_utf8(&self.bytes[..length])
            .expect("TextureSourceKey construction receives valid UTF-8")
    }
}

impl AsRef<str> for TextureSourceKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for TextureSourceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TextureSourceKey")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for TextureSourceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a source texture key was rejected.
///
/// Rejected input is never retained or echoed.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TextureSourceKeyError {
    #[error("texture source key is empty")]
    Empty,

    #[error("texture source key is {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },

    #[error("texture source key contains a control character at byte offset {offset}")]
    ControlCharacter { offset: usize },
}

/// The stable owner-local authority key used while validating one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextureBindingAuthorityKey<'a> {
    role: &'a TextureRole,
    source_key: Option<&'a TextureSourceKey>,
}

/// Borrowed material-use semantics independent from texture identity and storage layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextureBindingSemanticsRef<'a> {
    role: &'a TextureRole,
    coordinate_source: &'a TextureCoordinateSource,
    channel_mapping: &'a TextureChannelMapping,
    color_interpretation: &'a ColorInterpretation,
    transform: TextureTransform,
    strength: TextureStrength,
    blend: &'a TextureBlendOperation,
    source_key: Option<&'a TextureSourceKey>,
}

impl<'a> TextureBindingSemanticsRef<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        role: &'a TextureRole,
        coordinate_source: &'a TextureCoordinateSource,
        channel_mapping: &'a TextureChannelMapping,
        color_interpretation: &'a ColorInterpretation,
        transform: TextureTransform,
        strength: TextureStrength,
        blend: &'a TextureBlendOperation,
        source_key: Option<&'a TextureSourceKey>,
    ) -> Self {
        Self {
            role,
            coordinate_source,
            channel_mapping,
            color_interpretation,
            transform,
            strength,
            blend,
            source_key,
        }
    }

    pub(crate) const fn role(self) -> &'a TextureRole {
        self.role
    }

    pub(crate) const fn coordinate_source(self) -> &'a TextureCoordinateSource {
        self.coordinate_source
    }

    pub(crate) const fn channel_mapping(self) -> &'a TextureChannelMapping {
        self.channel_mapping
    }

    pub(crate) const fn color_interpretation(self) -> &'a ColorInterpretation {
        self.color_interpretation
    }

    pub(crate) const fn transform(self) -> TextureTransform {
        self.transform
    }

    pub(crate) const fn strength(self) -> TextureStrength {
        self.strength
    }

    pub(crate) const fn blend(self) -> &'a TextureBlendOperation {
        self.blend
    }

    pub(crate) const fn source_key(self) -> Option<&'a TextureSourceKey> {
        self.source_key
    }

    const fn authority_key(self) -> TextureBindingAuthorityKey<'a> {
        TextureBindingAuthorityKey {
            role: self.role,
            source_key: match self.role {
                TextureRole::Namespaced { .. } => self.source_key,
                _ => None,
            },
        }
    }
}

/// Borrowed material-level semantics independent from retained representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MaterialSemanticsRef<'a> {
    shading_model: &'a ShadingModel,
    alpha_mode: &'a AlphaMode,
    double_sided: bool,
    factors: MaterialFactors,
}

impl<'a> MaterialSemanticsRef<'a> {
    pub(crate) const fn new(
        shading_model: &'a ShadingModel,
        alpha_mode: &'a AlphaMode,
        double_sided: bool,
        factors: MaterialFactors,
    ) -> Self {
        Self {
            shading_model,
            alpha_mode,
            double_sided,
            factors,
        }
    }

    pub(crate) const fn shading_model(self) -> &'a ShadingModel {
        self.shading_model
    }

    pub(crate) const fn alpha_mode(self) -> &'a AlphaMode {
        self.alpha_mode
    }

    pub(crate) const fn double_sided(self) -> bool {
        self.double_sided
    }

    pub(crate) const fn factors(self) -> MaterialFactors {
        self.factors
    }
}

/// A material factor group required by one built-in shading model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialFactorGroup {
    PbrMetallicRoughness,
    PbrSpecularGlossiness,
    Legacy,
}

/// A bounded resource used while validating appearance semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppearanceBuildResource {
    TextureBindings,
    BindingAuthorityScratch,
}

/// The bounded dimension exceeded by one appearance resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppearanceLimitDimension {
    Count,
}

/// Why material or texture-binding semantics could not be admitted.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AppearanceBuildError {
    #[error("{shading:?} material requires factor group {required:?}")]
    MissingFactorGroup {
        shading: ShadingModelKind,
        required: MaterialFactorGroup,
    },

    #[error("texture role {role:?} does not admit color interpretation {interpretation:?}")]
    IncompatibleColorInterpretation {
        role: TextureRoleKind,
        interpretation: ColorInterpretationKind,
    },

    #[error("texture role {role:?} requires {minimum}..={maximum} mapped lanes, found {actual}")]
    IncompatibleChannelCount {
        role: TextureRoleKind,
        minimum: u8,
        maximum: u8,
        actual: u8,
    },

    #[error("texture role {role:?} violates its {requirement:?} strength requirement")]
    IncompatibleStrength {
        role: TextureRoleKind,
        requirement: TextureStrengthRequirement,
    },

    #[error("built-in texture role {role:?} requires Replace blend semantics")]
    BuiltInRoleRequiresReplace { role: TextureRoleKind },

    #[error("texture binding at index {duplicate_index} duplicates authority at {first_index}")]
    DuplicateBindingAuthority {
        first_index: usize,
        duplicate_index: usize,
    },

    #[error(
        "texture role {duplicate_role:?} at index {duplicate_index} conflicts with {first_role:?} authority at index {first_index}"
    )]
    ConflictingBindingRoles {
        first_index: usize,
        first_role: TextureRoleKind,
        duplicate_index: usize,
        duplicate_role: TextureRoleKind,
    },

    #[error(
        "appearance resource {resource:?} exceeds {dimension:?} limit: requested {requested}, maximum {maximum}"
    )]
    LimitExceeded {
        resource: AppearanceBuildResource,
        dimension: AppearanceLimitDimension,
        requested: usize,
        maximum: usize,
    },

    #[error("appearance allocation failed for {resource:?}")]
    AllocationFailed { resource: AppearanceBuildResource },
}

/// The role-specific numeric contract for one texture binding strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureStrengthRequirement {
    UnitInterval,
    Nonnegative,
    ExactlyOne,
    FiniteSigned,
}

pub(crate) fn validate_material_semantics(
    semantics: MaterialSemanticsRef<'_>,
) -> Result<(), AppearanceBuildError> {
    let shading_model = semantics.shading_model();
    let factors = semantics.factors();
    let required = match shading_model {
        ShadingModel::PbrMetallicRoughness if factors.pbr_metallic_roughness.is_none() => {
            Some(MaterialFactorGroup::PbrMetallicRoughness)
        }
        ShadingModel::PbrSpecularGlossiness if factors.pbr_specular_glossiness.is_none() => {
            Some(MaterialFactorGroup::PbrSpecularGlossiness)
        }
        ShadingModel::Lambert | ShadingModel::Phong | ShadingModel::BlinnPhong
            if factors.legacy.is_none() =>
        {
            Some(MaterialFactorGroup::Legacy)
        }
        ShadingModel::Unspecified
        | ShadingModel::Unlit
        | ShadingModel::Lambert
        | ShadingModel::Phong
        | ShadingModel::BlinnPhong
        | ShadingModel::PbrMetallicRoughness
        | ShadingModel::PbrSpecularGlossiness
        | ShadingModel::Namespaced { .. } => None,
    };

    if let Some(required) = required {
        return Err(AppearanceBuildError::MissingFactorGroup {
            shading: shading_model.kind(),
            required,
        });
    }
    Ok(())
}

fn validate_binding_color_interpretation(
    role: &TextureRole,
    interpretation: &ColorInterpretation,
) -> Result<(), AppearanceBuildError> {
    let compatible = match (role.usage_class(), interpretation) {
        (TextureUsageClass::Color, ColorInterpretation::SrgbColor)
        | (TextureUsageClass::Color, ColorInterpretation::LinearColor)
        | (TextureUsageClass::Data, ColorInterpretation::Data)
        | (_, ColorInterpretation::Namespaced { .. })
        | (TextureUsageClass::Namespaced, _) => true,
        (TextureUsageClass::Color, ColorInterpretation::Data)
        | (TextureUsageClass::Data, ColorInterpretation::SrgbColor)
        | (TextureUsageClass::Data, ColorInterpretation::LinearColor) => false,
    };

    if compatible {
        Ok(())
    } else {
        Err(AppearanceBuildError::IncompatibleColorInterpretation {
            role: role.kind(),
            interpretation: interpretation.kind(),
        })
    }
}

pub(crate) fn validate_texture_binding_semantics(
    semantics: TextureBindingSemanticsRef<'_>,
) -> Result<(), AppearanceBuildError> {
    validate_binding_color_interpretation(semantics.role(), semantics.color_interpretation())?;
    validate_binding_channel_mapping(semantics.role(), semantics.channel_mapping())?;
    validate_binding_strength(semantics.role(), semantics.strength())?;
    validate_binding_blend(semantics.role(), semantics.blend())
}

fn validate_binding_channel_mapping(
    role: &TextureRole,
    mapping: &TextureChannelMapping,
) -> Result<(), AppearanceBuildError> {
    let Some((minimum, maximum)) = role.channel_lane_range() else {
        return Ok(());
    };
    let Some(actual) = mapping.lane_count() else {
        return Ok(());
    };
    if (minimum..=maximum).contains(&actual) {
        Ok(())
    } else {
        Err(AppearanceBuildError::IncompatibleChannelCount {
            role: role.kind(),
            minimum,
            maximum,
            actual,
        })
    }
}

fn validate_binding_strength(
    role: &TextureRole,
    strength: TextureStrength,
) -> Result<(), AppearanceBuildError> {
    let requirement = match role {
        TextureRole::BaseColor
        | TextureRole::Diffuse
        | TextureRole::Specular
        | TextureRole::Occlusion
        | TextureRole::Opacity => TextureStrengthRequirement::UnitInterval,
        TextureRole::Emissive | TextureRole::SheenColor => TextureStrengthRequirement::Nonnegative,
        TextureRole::Metallic
        | TextureRole::Roughness
        | TextureRole::MetallicRoughness
        | TextureRole::Clearcoat
        | TextureRole::ClearcoatRoughness
        | TextureRole::Transmission
        | TextureRole::Thickness
        | TextureRole::SheenRoughness => TextureStrengthRequirement::ExactlyOne,
        TextureRole::Normal | TextureRole::ClearcoatNormal | TextureRole::Displacement => {
            TextureStrengthRequirement::FiniteSigned
        }
        TextureRole::Namespaced { .. } => return Ok(()),
    };
    let value = strength.get();
    let valid = match requirement {
        TextureStrengthRequirement::UnitInterval => (0.0..=1.0).contains(&value),
        TextureStrengthRequirement::Nonnegative => value >= 0.0,
        TextureStrengthRequirement::ExactlyOne => value == 1.0,
        TextureStrengthRequirement::FiniteSigned => value.is_finite(),
    };
    if valid {
        Ok(())
    } else {
        Err(AppearanceBuildError::IncompatibleStrength {
            role: role.kind(),
            requirement,
        })
    }
}

fn validate_binding_blend(
    role: &TextureRole,
    blend: &TextureBlendOperation,
) -> Result<(), AppearanceBuildError> {
    if matches!(role, TextureRole::Namespaced { .. })
        || matches!(blend, TextureBlendOperation::Replace)
    {
        Ok(())
    } else {
        Err(AppearanceBuildError::BuiltInRoleRequiresReplace { role: role.kind() })
    }
}

pub(crate) struct MaterialTextureBindingSemanticsValidator<'a> {
    binding_count: usize,
    next_index: usize,
    builtin_first_indices: [Option<usize>; TextureRoleKind::BUILTIN_COUNT],
    namespaced_first_indices: HashMap<TextureBindingAuthorityKey<'a>, usize>,
    namespaced_capacity_reserved: bool,
    metallic: Option<usize>,
    roughness: Option<usize>,
    metallic_roughness: Option<usize>,
}

impl<'a> MaterialTextureBindingSemanticsValidator<'a> {
    pub(crate) fn new(binding_count: usize) -> Self {
        Self {
            binding_count,
            next_index: 0,
            builtin_first_indices: [None; TextureRoleKind::BUILTIN_COUNT],
            namespaced_first_indices: HashMap::new(),
            namespaced_capacity_reserved: false,
            metallic: None,
            roughness: None,
            metallic_roughness: None,
        }
    }

    pub(crate) fn validate_next(
        &mut self,
        binding: TextureBindingSemanticsRef<'a>,
    ) -> Result<(), AppearanceBuildError> {
        if self.next_index >= self.binding_count {
            return Err(AppearanceBuildError::LimitExceeded {
                resource: AppearanceBuildResource::TextureBindings,
                dimension: AppearanceLimitDimension::Count,
                requested: self.next_index.saturating_add(1),
                maximum: self.binding_count,
            });
        }
        let duplicate_index = self.next_index;
        self.next_index += 1;
        let role = binding.role().kind();
        if let Some(role_index) = role.builtin_index() {
            let first_index = &mut self.builtin_first_indices[role_index];
            if let Some(first_index) = *first_index {
                return Err(AppearanceBuildError::DuplicateBindingAuthority {
                    first_index,
                    duplicate_index,
                });
            }
            *first_index = Some(duplicate_index);
        } else {
            if !self.namespaced_capacity_reserved {
                self.namespaced_first_indices
                    .try_reserve(self.binding_count)
                    .map_err(|_| AppearanceBuildError::AllocationFailed {
                        resource: AppearanceBuildResource::BindingAuthorityScratch,
                    })?;
                self.namespaced_capacity_reserved = true;
            }
            let authority = binding.authority_key();
            if let Some(&first_index) = self.namespaced_first_indices.get(&authority) {
                return Err(AppearanceBuildError::DuplicateBindingAuthority {
                    first_index,
                    duplicate_index,
                });
            }
            self.namespaced_first_indices
                .insert(authority, duplicate_index);
        }

        let conflict = match role {
            TextureRoleKind::Metallic => self
                .metallic_roughness
                .map(|index| (index, TextureRoleKind::MetallicRoughness)),
            TextureRoleKind::Roughness => self
                .metallic_roughness
                .map(|index| (index, TextureRoleKind::MetallicRoughness)),
            TextureRoleKind::MetallicRoughness => self
                .metallic
                .map(|index| (index, TextureRoleKind::Metallic))
                .or_else(|| {
                    self.roughness
                        .map(|index| (index, TextureRoleKind::Roughness))
                }),
            _ => None,
        };
        if let Some((first_index, first_role)) = conflict {
            return Err(AppearanceBuildError::ConflictingBindingRoles {
                first_index,
                first_role,
                duplicate_index,
                duplicate_role: role,
            });
        }
        match role {
            TextureRoleKind::Metallic => self.metallic = Some(duplicate_index),
            TextureRoleKind::Roughness => self.roughness = Some(duplicate_index),
            TextureRoleKind::MetallicRoughness => {
                self.metallic_roughness = Some(duplicate_index);
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn validate_material_texture_binding_semantics<'a>(
    bindings: impl ExactSizeIterator<Item = TextureBindingSemanticsRef<'a>>,
) -> Result<(), AppearanceBuildError> {
    let mut validator = MaterialTextureBindingSemanticsValidator::new(bindings.len());
    for binding in bindings {
        validator.validate_next(binding)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static UV_ZERO: TextureCoordinateSource = TextureCoordinateSource::UvSet(0);
    static REPLACE: TextureBlendOperation = TextureBlendOperation::Replace;

    fn schema(name: &str) -> ExtensionSchemaId {
        ExtensionSchemaId::new(name, 1).expect("test schema should be canonical")
    }

    fn binding_semantics<'a>(
        role: &'a TextureRole,
        channel_mapping: &'a TextureChannelMapping,
        color_interpretation: &'a ColorInterpretation,
        strength: TextureStrength,
        blend: &'a TextureBlendOperation,
        source_key: Option<&'a TextureSourceKey>,
    ) -> TextureBindingSemanticsRef<'a> {
        TextureBindingSemanticsRef::new(
            role,
            &UV_ZERO,
            channel_mapping,
            color_interpretation,
            TextureTransform::IDENTITY,
            strength,
            blend,
            source_key,
        )
    }

    #[test]
    fn material_semantics_require_and_preserve_factor_groups() {
        let shading_model = ShadingModel::PbrMetallicRoughness;
        let alpha_mode = AlphaMode::Opaque;
        assert_eq!(
            validate_material_semantics(MaterialSemanticsRef::new(
                &shading_model,
                &alpha_mode,
                false,
                MaterialFactors::empty(),
            )),
            Err(AppearanceBuildError::MissingFactorGroup {
                shading: ShadingModelKind::PbrMetallicRoughness,
                required: MaterialFactorGroup::PbrMetallicRoughness,
            })
        );

        let pbr = PbrMetallicRoughnessFactors::try_new(
            Color::linear_rgba(0.7, 0.6, 0.5, 1.0),
            0.25,
            0.75,
        )
        .expect("PBR factors should be valid");
        let legacy = LegacyMaterialFactors::try_new(
            Vec3::new(0.1, 0.1, 0.1),
            Vec3::new(0.7, 0.6, 0.5),
            Vec3::new(0.3, 0.3, 0.3),
            Vec3::new(2.0, 1.0, 0.5),
            64.0,
            0.9,
        )
        .expect("legacy factors should be valid");
        let factors = MaterialFactors::try_new(Vec3::ZERO, Some(pbr), None, Some(legacy))
            .expect("factor set should be valid");
        let semantics = MaterialSemanticsRef::new(&shading_model, &alpha_mode, true, factors);

        validate_material_semantics(semantics).expect("selected factor group is present");
        assert_eq!(semantics.factors().pbr_metallic_roughness(), Some(pbr));
        assert_eq!(semantics.factors().legacy(), Some(legacy));
        assert!(semantics.double_sided());
    }

    #[test]
    fn namespaced_behaviors_require_schema_authority() {
        assert_eq!(
            ShadingModel::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::ShadingModel,
            })
        );
        assert_eq!(
            TextureRole::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::TextureRole,
            })
        );

        let admitted = schema("vendor:layered-role");
        let role = TextureRole::try_namespaced(Some(admitted.clone()))
            .expect("schema-backed role should build");
        assert_eq!(role.schema_id(), Some(&admitted));
    }

    #[test]
    fn semantic_value_boundaries_fail_closed() {
        assert_eq!(
            PbrMetallicRoughnessFactors::try_new(Color::WHITE, f32::NAN, 0.5),
            Err(MaterialFactorError::NonFinite {
                field: MaterialFactorField::Metallic,
            })
        );
        assert_eq!(
            TextureStrength::try_new(f32::INFINITY),
            Err(TextureStrengthError::NonFinite)
        );
        assert_eq!(
            TextureCoordinateSource::uv_set(MAX_TEXTURE_COORDINATE_SET + 1),
            Err(TextureCoordinateSourceError::SetTooLarge {
                actual: MAX_TEXTURE_COORDINATE_SET + 1,
                maximum: MAX_TEXTURE_COORDINATE_SET,
            })
        );

        let hostile = "private-path\ncredential";
        let error = TextureSourceKey::try_new(hostile).expect_err("control text must fail");
        assert_eq!(
            error,
            TextureSourceKeyError::ControlCharacter {
                offset: "private-path".len(),
            }
        );
        assert!(!error.to_string().contains("private-path"));
        assert!(!error.to_string().contains("credential"));
    }

    #[test]
    fn binding_semantics_enforce_role_contracts() {
        let normal = TextureRole::Normal;
        let rgb = TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB);
        let srgb = ColorInterpretation::SrgbColor;
        assert_eq!(
            validate_texture_binding_semantics(binding_semantics(
                &normal,
                &rgb,
                &srgb,
                TextureStrength::ONE,
                &REPLACE,
                None,
            )),
            Err(AppearanceBuildError::IncompatibleColorInterpretation {
                role: TextureRoleKind::Normal,
                interpretation: ColorInterpretationKind::SrgbColor,
            })
        );

        let specular = TextureRole::Specular;
        let rgba = TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA);
        let linear = ColorInterpretation::LinearColor;
        assert_eq!(
            validate_texture_binding_semantics(binding_semantics(
                &specular,
                &rgba,
                &linear,
                TextureStrength::ONE,
                &REPLACE,
                None,
            )),
            Err(AppearanceBuildError::IncompatibleChannelCount {
                role: TextureRoleKind::Specular,
                minimum: 3,
                maximum: 3,
                actual: 4,
            })
        );

        let metallic = TextureRole::Metallic;
        let red = TextureChannelMapping::swizzle(TextureChannelSwizzle::RED);
        let data = ColorInterpretation::Data;
        assert_eq!(
            validate_texture_binding_semantics(binding_semantics(
                &metallic,
                &red,
                &data,
                TextureStrength::try_new(0.5).expect("finite strength should build"),
                &REPLACE,
                None,
            )),
            Err(AppearanceBuildError::IncompatibleStrength {
                role: TextureRoleKind::Metallic,
                requirement: TextureStrengthRequirement::ExactlyOne,
            })
        );

        let base_color = TextureRole::BaseColor;
        let multiply = TextureBlendOperation::Multiply;
        assert_eq!(
            validate_texture_binding_semantics(binding_semantics(
                &base_color,
                &rgb,
                &srgb,
                TextureStrength::ONE,
                &multiply,
                None,
            )),
            Err(AppearanceBuildError::BuiltInRoleRequiresReplace {
                role: TextureRoleKind::BaseColor,
            })
        );
    }

    #[test]
    fn material_validator_tracks_builtin_and_namespaced_authority() {
        let builtin_role = TextureRole::BaseColor;
        let rgb = TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB);
        let color = ColorInterpretation::SrgbColor;
        let first_key = TextureSourceKey::try_new("layer-0").expect("source key should be valid");
        let second_key = TextureSourceKey::try_new("layer-1").expect("source key should be valid");
        let first = binding_semantics(
            &builtin_role,
            &rgb,
            &color,
            TextureStrength::ONE,
            &REPLACE,
            Some(&first_key),
        );
        let duplicate = binding_semantics(
            &builtin_role,
            &rgb,
            &color,
            TextureStrength::ONE,
            &REPLACE,
            Some(&second_key),
        );
        assert_eq!(
            validate_material_texture_binding_semantics([first, duplicate].into_iter()),
            Err(AppearanceBuildError::DuplicateBindingAuthority {
                first_index: 0,
                duplicate_index: 1,
            })
        );

        let namespaced_role = TextureRole::try_namespaced(Some(schema("vendor:layered-role")))
            .expect("schema-backed role should build");
        let red = TextureChannelMapping::swizzle(TextureChannelSwizzle::RED);
        let data = ColorInterpretation::Data;
        let first = binding_semantics(
            &namespaced_role,
            &red,
            &data,
            TextureStrength::ONE,
            &REPLACE,
            Some(&first_key),
        );
        let second = binding_semantics(
            &namespaced_role,
            &red,
            &data,
            TextureStrength::ONE,
            &REPLACE,
            Some(&second_key),
        );
        validate_material_texture_binding_semantics([first, second].into_iter())
            .expect("distinct schema-local keys define distinct authorities");
    }

    #[test]
    fn material_validator_rejects_role_conflicts_and_count_overrun() {
        let combined_role = TextureRole::MetallicRoughness;
        let combined_channels =
            TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green])
                .expect("two-lane mapping should be valid");
        let combined_mapping = TextureChannelMapping::swizzle(combined_channels);
        let metallic_role = TextureRole::Metallic;
        let metallic_mapping = TextureChannelMapping::swizzle(TextureChannelSwizzle::RED);
        let data = ColorInterpretation::Data;
        let combined = binding_semantics(
            &combined_role,
            &combined_mapping,
            &data,
            TextureStrength::ONE,
            &REPLACE,
            None,
        );
        let separate = binding_semantics(
            &metallic_role,
            &metallic_mapping,
            &data,
            TextureStrength::ONE,
            &REPLACE,
            None,
        );
        assert_eq!(
            validate_material_texture_binding_semantics([combined, separate].into_iter()),
            Err(AppearanceBuildError::ConflictingBindingRoles {
                first_index: 0,
                first_role: TextureRoleKind::MetallicRoughness,
                duplicate_index: 1,
                duplicate_role: TextureRoleKind::Metallic,
            })
        );

        let mut validator = MaterialTextureBindingSemanticsValidator::new(0);
        assert_eq!(
            validator.validate_next(separate),
            Err(AppearanceBuildError::LimitExceeded {
                resource: AppearanceBuildResource::TextureBindings,
                dimension: AppearanceLimitDimension::Count,
                requested: 1,
                maximum: 0,
            })
        );
    }
}
