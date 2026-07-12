//! Canonical material factors and texture usage bindings.
//!
//! Image identity, sampler behavior, and their reusable texture pairing live
//! in `image`. This module owns only material interpretation. Scene-local
//! references remain private typed indexes until a read view combines them
//! with the owning scene generation.

use crate::entity::EntityName;
use crate::extension::{
    ExtensionEntry, ExtensionEntryValidationError, ExtensionSchemaId, ExtensionSchemaOwner,
    validate_unique_extension_entry_keys,
};
use crate::identity::{SceneEntityKind, SceneGenerationId, TextureId, TextureIndex};
use crate::math::{Color, Vec2, Vec3};
use crate::provenance::Provenance;
use baozi_operation::CapacityPlan;
use std::collections::HashMap;
use std::fmt;
use thiserror::Error;

/// Maximum material texture bindings retained by one canonical material.
pub const MAX_TEXTURE_BINDINGS_PER_MATERIAL: usize = 256;

/// Maximum descriptive/source extension entries retained by one material.
pub const MAX_MATERIAL_EXTENSION_ENTRIES: usize = 128;

/// Maximum descriptive/source extension entries retained by one binding.
pub const MAX_TEXTURE_BINDING_EXTENSION_ENTRIES: usize = 32;

/// Maximum logical extension cells retained directly by one material.
pub const MAX_MATERIAL_EXTENSION_CELLS: usize = 65_536;

/// Maximum logical extension cells retained directly by one texture binding.
pub const MAX_TEXTURE_BINDING_EXTENSION_CELLS: usize = 4_096;

/// Maximum heap-backed extension payload bytes retained by one material.
pub const MAX_MATERIAL_EXTENSION_DYNAMIC_BYTES: usize = 1024 * 1024;

/// Maximum heap-backed extension payload bytes retained by one binding.
pub const MAX_TEXTURE_BINDING_EXTENSION_DYNAMIC_BYTES: usize = 64 * 1024;

/// Maximum allocator-retained extension payload bytes owned by one material.
pub const MAX_MATERIAL_EXTENSION_RETAINED_BYTES: usize = 1024 * 1024;

/// Maximum allocator-retained extension payload bytes owned by one binding.
pub const MAX_TEXTURE_BINDING_EXTENSION_RETAINED_BYTES: usize = 64 * 1024;

/// Maximum UTF-8 bytes in one format-owned texture source key.
pub const MAX_TEXTURE_SOURCE_KEY_BYTES: usize = 128;

/// Maximum zero-based UV set address admitted by the canonical binding model.
pub const MAX_TEXTURE_COORDINATE_SET: u32 = u16::MAX as u32;

const MAX_APPEARANCE_COLLECTION_STORAGE_BYTES: usize = 1024 * 1024;

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

impl AppearanceBehaviorKind {
    /// Returns the declarative schema owner required at central validation.
    pub const fn expected_schema_owner(self) -> ExtensionSchemaOwner {
        match self {
            Self::ShadingModel | Self::AlphaMode => {
                ExtensionSchemaOwner::Entity(SceneEntityKind::Material)
            }
            Self::TextureRole
            | Self::CoordinateSource
            | Self::ChannelMapping
            | Self::ColorInterpretation
            | Self::BlendOperation => ExtensionSchemaOwner::TextureBinding,
        }
    }
}

/// One behavior-specific schema use exposed to the central Scene validator.
///
/// Retaining the behavior category prevents one owner-compatible schema ID
/// from silently authorizing unrelated appearance semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppearanceBehaviorSchemaUse<'a> {
    behavior: AppearanceBehaviorKind,
    schema_id: &'a ExtensionSchemaId,
}

impl<'a> AppearanceBehaviorSchemaUse<'a> {
    const fn new(behavior: AppearanceBehaviorKind, schema_id: &'a ExtensionSchemaId) -> Self {
        Self {
            behavior,
            schema_id,
        }
    }

    pub const fn behavior(self) -> AppearanceBehaviorKind {
        self.behavior
    }

    pub const fn schema_id(self) -> &'a ExtensionSchemaId {
        self.schema_id
    }

    pub const fn expected_schema_owner(self) -> ExtensionSchemaOwner {
        self.behavior.expected_schema_owner()
    }
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
    Unlit,
    Lambert,
    Phong,
    BlinnPhong,
    PbrMetallicRoughness,
    PbrSpecularGlossiness,
    Namespaced { schema_id: ExtensionSchemaId },
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
            Self::Unlit
            | Self::Lambert
            | Self::Phong
            | Self::BlinnPhong
            | Self::PbrMetallicRoughness
            | Self::PbrSpecularGlossiness => None,
        }
    }

    pub const fn kind(&self) -> ShadingModelKind {
        match self {
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

/// Independent typed factor groups retained by one material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialFactors {
    emissive: Vec3,
    pbr_metallic_roughness: Option<PbrMetallicRoughnessFactors>,
    pbr_specular_glossiness: Option<PbrSpecularGlossinessFactors>,
    legacy: Option<LegacyMaterialFactors>,
}

impl MaterialFactors {
    pub fn try_new(
        emissive: Vec3,
        pbr_metallic_roughness: Option<PbrMetallicRoughnessFactors>,
        pbr_specular_glossiness: Option<PbrSpecularGlossinessFactors>,
        legacy: Option<LegacyMaterialFactors>,
    ) -> Result<Self, MaterialFactorError> {
        validate_nonnegative_vec3(emissive, MaterialFactorField::Emissive)?;
        Ok(Self {
            emissive,
            pbr_metallic_roughness,
            pbr_specular_glossiness,
            legacy,
        })
    }

    pub const fn empty() -> Self {
        Self {
            emissive: Vec3::ZERO,
            pbr_metallic_roughness: None,
            pbr_specular_glossiness: None,
            legacy: None,
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
    Height,
    Displacement,
    Lightmap,
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
            | Self::Height
            | Self::Displacement
            | Self::Lightmap
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
            Self::Height => TextureRoleKind::Height,
            Self::Displacement => TextureRoleKind::Displacement,
            Self::Lightmap => TextureRoleKind::Lightmap,
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
            | Self::Lightmap
            | Self::SheenColor => TextureUsageClass::Color,
            Self::Metallic
            | Self::Roughness
            | Self::MetallicRoughness
            | Self::Normal
            | Self::Occlusion
            | Self::Opacity
            | Self::Height
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
            Self::BaseColor | Self::Diffuse | Self::Specular | Self::Lightmap => Some((3, 4)),
            Self::Emissive | Self::SheenColor => Some((3, 3)),
            Self::MetallicRoughness => Some((2, 2)),
            Self::Normal | Self::ClearcoatNormal => Some((2, 3)),
            Self::Metallic
            | Self::Roughness
            | Self::Occlusion
            | Self::Opacity
            | Self::Height
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
    Height,
    Displacement,
    Lightmap,
    Clearcoat,
    ClearcoatRoughness,
    ClearcoatNormal,
    Transmission,
    Thickness,
    SheenColor,
    SheenRoughness,
    Namespaced,
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

/// The stable owner-local authority key of one texture binding.
///
/// An unkeyed role has one canonical authority. Formats that admit ordered
/// layers assign distinct bounded source keys, making `(role, source_key)` the
/// deterministic authority without treating a filename as identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureBindingAuthorityKey<'a> {
    role: &'a TextureRole,
    source_key: Option<&'a TextureSourceKey>,
}

impl<'a> TextureBindingAuthorityKey<'a> {
    pub const fn role(self) -> &'a TextureRole {
        self.role
    }

    pub const fn source_key(self) -> Option<&'a TextureSourceKey> {
        self.source_key
    }
}

/// One material-owned use of a reusable scene texture.
///
/// The binding deliberately stores only a private [`TextureIndex`]. Public
/// generation-qualified identity is synthesized by [`TextureBindingView`].
/// Heap-bearing extensions make this type intentionally non-`Clone`.
#[derive(Debug, PartialEq)]
pub struct TextureBinding {
    texture: TextureIndex,
    role: TextureRole,
    coordinate_source: TextureCoordinateSource,
    channel_mapping: TextureChannelMapping,
    color_interpretation: ColorInterpretation,
    transform: TextureTransform,
    strength: TextureStrength,
    blend: TextureBlendOperation,
    source_key: Option<TextureSourceKey>,
    provenance: Provenance,
    extensions: Vec<ExtensionEntry>,
}

impl TextureBinding {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        texture: TextureIndex,
        role: TextureRole,
        coordinate_source: TextureCoordinateSource,
        channel_mapping: TextureChannelMapping,
        color_interpretation: ColorInterpretation,
        transform: TextureTransform,
        strength: TextureStrength,
        blend: TextureBlendOperation,
        source_key: Option<TextureSourceKey>,
        provenance: Provenance,
        extensions: impl IntoIterator<Item = ExtensionEntry>,
    ) -> Result<Self, AppearanceBuildError> {
        validate_binding_color_interpretation(&role, &color_interpretation)?;
        validate_binding_channel_mapping(&role, &channel_mapping)?;
        let extensions = try_collect_extension_entries(
            extensions,
            AppearanceExtensionOwner::TextureBinding,
            AppearanceBuildResource::TextureBindingExtensions,
            ExtensionLimits {
                max_entries: MAX_TEXTURE_BINDING_EXTENSION_ENTRIES,
                max_cells: MAX_TEXTURE_BINDING_EXTENSION_CELLS,
                max_dynamic_bytes: MAX_TEXTURE_BINDING_EXTENSION_DYNAMIC_BYTES,
                max_retained_bytes: MAX_TEXTURE_BINDING_EXTENSION_RETAINED_BYTES,
            },
        )?;

        Ok(Self {
            texture,
            role,
            coordinate_source,
            channel_mapping,
            color_interpretation,
            transform,
            strength,
            blend,
            source_key,
            provenance,
            extensions,
        })
    }

    pub(crate) const fn texture_index(&self) -> TextureIndex {
        self.texture
    }

    pub const fn role(&self) -> &TextureRole {
        &self.role
    }

    pub const fn coordinate_source(&self) -> &TextureCoordinateSource {
        &self.coordinate_source
    }

    pub const fn channel_mapping(&self) -> &TextureChannelMapping {
        &self.channel_mapping
    }

    pub const fn color_interpretation(&self) -> &ColorInterpretation {
        &self.color_interpretation
    }

    pub const fn transform(&self) -> TextureTransform {
        self.transform
    }

    pub const fn strength(&self) -> TextureStrength {
        self.strength
    }

    pub const fn blend(&self) -> &TextureBlendOperation {
        &self.blend
    }

    pub const fn source_key(&self) -> Option<&TextureSourceKey> {
        self.source_key.as_ref()
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub fn extensions(&self) -> &[ExtensionEntry] {
        &self.extensions
    }

    pub const fn authority_key(&self) -> TextureBindingAuthorityKey<'_> {
        TextureBindingAuthorityKey {
            role: &self.role,
            source_key: self.source_key.as_ref(),
        }
    }

    /// Returns every schema reference that the Scene validator must resolve as
    /// an `ExtensionSchemaOwner::TextureBinding` schema.
    pub(crate) fn behavior_schema_uses(
        &self,
    ) -> impl Iterator<Item = AppearanceBehaviorSchemaUse<'_>> {
        [
            self.role.schema_id().map(|schema_id| {
                AppearanceBehaviorSchemaUse::new(AppearanceBehaviorKind::TextureRole, schema_id)
            }),
            self.coordinate_source.schema_id().map(|schema_id| {
                AppearanceBehaviorSchemaUse::new(
                    AppearanceBehaviorKind::CoordinateSource,
                    schema_id,
                )
            }),
            self.channel_mapping.schema_id().map(|schema_id| {
                AppearanceBehaviorSchemaUse::new(AppearanceBehaviorKind::ChannelMapping, schema_id)
            }),
            self.color_interpretation.schema_id().map(|schema_id| {
                AppearanceBehaviorSchemaUse::new(
                    AppearanceBehaviorKind::ColorInterpretation,
                    schema_id,
                )
            }),
            self.blend.schema_id().map(|schema_id| {
                AppearanceBehaviorSchemaUse::new(AppearanceBehaviorKind::BlendOperation, schema_id)
            }),
        ]
        .into_iter()
        .flatten()
    }

    pub(crate) const fn view(&self, generation: SceneGenerationId) -> TextureBindingView<'_> {
        TextureBindingView {
            binding: self,
            generation,
        }
    }
}

/// A read-only texture binding with a generation-qualified texture handle.
#[derive(Debug, Clone, Copy)]
pub struct TextureBindingView<'a> {
    binding: &'a TextureBinding,
    generation: SceneGenerationId,
}

impl<'a> TextureBindingView<'a> {
    pub const fn texture(self) -> TextureId {
        self.binding.texture.handle(self.generation)
    }

    pub const fn role(self) -> &'a TextureRole {
        self.binding.role()
    }

    pub const fn coordinate_source(self) -> &'a TextureCoordinateSource {
        self.binding.coordinate_source()
    }

    pub const fn channel_mapping(self) -> &'a TextureChannelMapping {
        self.binding.channel_mapping()
    }

    pub const fn color_interpretation(self) -> &'a ColorInterpretation {
        self.binding.color_interpretation()
    }

    pub const fn transform(self) -> TextureTransform {
        self.binding.transform()
    }

    pub const fn strength(self) -> TextureStrength {
        self.binding.strength()
    }

    pub const fn blend(self) -> &'a TextureBlendOperation {
        self.binding.blend()
    }

    pub const fn source_key(self) -> Option<&'a TextureSourceKey> {
        self.binding.source_key()
    }

    pub const fn provenance(self) -> Provenance {
        self.binding.provenance()
    }

    pub fn extensions(self) -> &'a [ExtensionEntry] {
        self.binding.extensions()
    }
}

/// One canonical material with independent typed factor groups.
///
/// Material and binding extensions are descriptive/source facts only. Every
/// behavior-bearing escape hatch is represented by a typed schema-backed
/// variant instead. Heap-bearing collections make this type non-`Clone`.
#[derive(Debug, PartialEq)]
pub struct Material {
    name: Option<EntityName>,
    shading_model: ShadingModel,
    alpha_mode: AlphaMode,
    double_sided: bool,
    factors: MaterialFactors,
    texture_bindings: Vec<TextureBinding>,
    provenance: Provenance,
    extensions: Vec<ExtensionEntry>,
}

impl Material {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        name: Option<EntityName>,
        shading_model: ShadingModel,
        alpha_mode: AlphaMode,
        double_sided: bool,
        factors: MaterialFactors,
        texture_bindings: impl IntoIterator<Item = TextureBinding>,
        provenance: Provenance,
        extensions: impl IntoIterator<Item = ExtensionEntry>,
    ) -> Result<Self, AppearanceBuildError> {
        validate_required_factor_group(&shading_model, factors)?;
        let texture_bindings = try_collect_bounded(
            texture_bindings,
            MAX_TEXTURE_BINDINGS_PER_MATERIAL,
            MAX_APPEARANCE_COLLECTION_STORAGE_BYTES,
            AppearanceBuildResource::TextureBindings,
        )?;
        validate_unique_binding_authorities(&texture_bindings)?;
        let extensions = try_collect_extension_entries(
            extensions,
            AppearanceExtensionOwner::Material,
            AppearanceBuildResource::MaterialExtensions,
            ExtensionLimits {
                max_entries: MAX_MATERIAL_EXTENSION_ENTRIES,
                max_cells: MAX_MATERIAL_EXTENSION_CELLS,
                max_dynamic_bytes: MAX_MATERIAL_EXTENSION_DYNAMIC_BYTES,
                max_retained_bytes: MAX_MATERIAL_EXTENSION_RETAINED_BYTES,
            },
        )?;

        Ok(Self {
            name,
            shading_model,
            alpha_mode,
            double_sided,
            factors,
            texture_bindings,
            provenance,
            extensions,
        })
    }

    pub const fn name(&self) -> Option<&EntityName> {
        self.name.as_ref()
    }

    pub const fn shading_model(&self) -> &ShadingModel {
        &self.shading_model
    }

    pub const fn alpha_mode(&self) -> &AlphaMode {
        &self.alpha_mode
    }

    pub const fn double_sided(&self) -> bool {
        self.double_sided
    }

    pub const fn factors(&self) -> MaterialFactors {
        self.factors
    }

    pub(crate) fn texture_bindings(&self) -> &[TextureBinding] {
        &self.texture_bindings
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub fn extensions(&self) -> &[ExtensionEntry] {
        &self.extensions
    }

    /// Returns every schema reference that the Scene validator must resolve as
    /// an `ExtensionSchemaOwner::Entity(SceneEntityKind::Material)` schema.
    pub(crate) fn behavior_schema_uses(
        &self,
    ) -> impl Iterator<Item = AppearanceBehaviorSchemaUse<'_>> {
        [
            self.shading_model.schema_id().map(|schema_id| {
                AppearanceBehaviorSchemaUse::new(AppearanceBehaviorKind::ShadingModel, schema_id)
            }),
            self.alpha_mode.schema_id().map(|schema_id| {
                AppearanceBehaviorSchemaUse::new(AppearanceBehaviorKind::AlphaMode, schema_id)
            }),
        ]
        .into_iter()
        .flatten()
    }

    pub(crate) const fn view(&self, generation: SceneGenerationId) -> MaterialView<'_> {
        MaterialView {
            material: self,
            generation,
        }
    }
}

/// An allocation-free read facade over one scene-owned material.
#[derive(Debug, Clone, Copy)]
pub struct MaterialView<'a> {
    material: &'a Material,
    generation: SceneGenerationId,
}

impl<'a> MaterialView<'a> {
    pub const fn name(self) -> Option<&'a EntityName> {
        self.material.name()
    }

    pub const fn shading_model(self) -> &'a ShadingModel {
        self.material.shading_model()
    }

    pub const fn alpha_mode(self) -> &'a AlphaMode {
        self.material.alpha_mode()
    }

    pub const fn double_sided(self) -> bool {
        self.material.double_sided()
    }

    pub const fn factors(self) -> MaterialFactors {
        self.material.factors()
    }

    pub fn texture_bindings(self) -> impl ExactSizeIterator<Item = TextureBindingView<'a>> + 'a {
        self.material
            .texture_bindings
            .iter()
            .map(move |binding| binding.view(self.generation))
    }

    pub const fn provenance(self) -> Provenance {
        self.material.provenance()
    }

    pub fn extensions(self) -> &'a [ExtensionEntry] {
        self.material.extensions()
    }
}

/// A material factor group required by one built-in shading model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialFactorGroup {
    PbrMetallicRoughness,
    PbrSpecularGlossiness,
    Legacy,
}

/// A bounded collection or scratch resource used while building appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppearanceBuildResource {
    TextureBindings,
    MaterialExtensions,
    TextureBindingExtensions,
    BindingAuthorityScratch,
}

/// The bounded dimension exceeded by one appearance resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppearanceLimitDimension {
    Count,
    StorageBytes,
    Cells,
    DynamicBytes,
    RetainedBytes,
}

/// The owner of one rejected extension collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppearanceExtensionOwner {
    Material,
    TextureBinding,
}

/// Why a canonical material or texture binding could not be built.
///
/// Errors expose only stable categories, indexes, and sizes. Source keys and
/// extension values are never copied into an error.
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

    #[error("texture binding at index {duplicate_index} duplicates authority at {first_index}")]
    DuplicateBindingAuthority {
        first_index: usize,
        duplicate_index: usize,
    },

    #[error("extension collection for {owner:?} is invalid")]
    InvalidExtensions {
        owner: AppearanceExtensionOwner,
        #[source]
        source: ExtensionEntryValidationError,
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

    #[error("appearance resource {resource:?} size overflow for {count} elements")]
    CollectionSizeOverflow {
        resource: AppearanceBuildResource,
        count: usize,
    },

    #[error("appearance allocation failed for {resource:?}")]
    AllocationFailed { resource: AppearanceBuildResource },
}

fn validate_required_factor_group(
    shading_model: &ShadingModel,
    factors: MaterialFactors,
) -> Result<(), AppearanceBuildError> {
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
        ShadingModel::Unlit
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

fn validate_unique_binding_authorities(
    bindings: &[TextureBinding],
) -> Result<(), AppearanceBuildError> {
    let mut first_indices = HashMap::new();
    first_indices.try_reserve(bindings.len()).map_err(|_| {
        AppearanceBuildError::AllocationFailed {
            resource: AppearanceBuildResource::BindingAuthorityScratch,
        }
    })?;

    for (duplicate_index, binding) in bindings.iter().enumerate() {
        let authority = binding.authority_key();
        if let Some(&first_index) = first_indices.get(&authority) {
            return Err(AppearanceBuildError::DuplicateBindingAuthority {
                first_index,
                duplicate_index,
            });
        }
        first_indices.insert(authority, duplicate_index);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ExtensionLimits {
    max_entries: usize,
    max_cells: usize,
    max_dynamic_bytes: usize,
    max_retained_bytes: usize,
}

fn try_collect_extension_entries(
    entries: impl IntoIterator<Item = ExtensionEntry>,
    owner: AppearanceExtensionOwner,
    resource: AppearanceBuildResource,
    limits: ExtensionLimits,
) -> Result<Vec<ExtensionEntry>, AppearanceBuildError> {
    let entries = try_collect_bounded(
        entries,
        limits.max_entries,
        MAX_APPEARANCE_COLLECTION_STORAGE_BYTES,
        resource,
    )?;
    validate_unique_extension_entry_keys(&entries)
        .map_err(|source| AppearanceBuildError::InvalidExtensions { owner, source })?;

    let mut cells = 0_usize;
    let mut dynamic_bytes = 0_usize;
    let mut retained_bytes = 0_usize;
    for entry in &entries {
        cells = cells.checked_add(entry.value().cell_count()).ok_or(
            AppearanceBuildError::CollectionSizeOverflow {
                resource,
                count: usize::MAX,
            },
        )?;
        dynamic_bytes = dynamic_bytes
            .checked_add(entry.value().dynamic_payload_bytes())
            .ok_or(AppearanceBuildError::CollectionSizeOverflow {
                resource,
                count: usize::MAX,
            })?;
        retained_bytes = retained_bytes
            .checked_add(entry.value().retained_allocation_bytes())
            .ok_or(AppearanceBuildError::CollectionSizeOverflow {
                resource,
                count: usize::MAX,
            })?;
    }

    validate_limit(
        resource,
        AppearanceLimitDimension::Cells,
        cells,
        limits.max_cells,
    )?;
    validate_limit(
        resource,
        AppearanceLimitDimension::DynamicBytes,
        dynamic_bytes,
        limits.max_dynamic_bytes,
    )?;
    validate_limit(
        resource,
        AppearanceLimitDimension::RetainedBytes,
        retained_bytes,
        limits.max_retained_bytes,
    )?;
    Ok(entries)
}

fn try_collect_bounded<T>(
    values: impl IntoIterator<Item = T>,
    maximum_count: usize,
    maximum_storage_bytes: usize,
    resource: AppearanceBuildResource,
) -> Result<Vec<T>, AppearanceBuildError> {
    let mut iterator = values.into_iter();
    let (minimum, _) = iterator.size_hint();
    validate_collection_size::<T>(resource, minimum, maximum_count, maximum_storage_bytes)?;

    let mut collected = Vec::new();
    if minimum != 0 {
        try_reserve_bounded(
            &mut collected,
            minimum,
            maximum_count,
            maximum_storage_bytes,
            resource,
        )?;
    }

    for value in &mut iterator {
        let desired =
            collected
                .len()
                .checked_add(1)
                .ok_or(AppearanceBuildError::CollectionSizeOverflow {
                    resource,
                    count: usize::MAX,
                })?;
        validate_collection_size::<T>(resource, desired, maximum_count, maximum_storage_bytes)?;
        if collected.len() == collected.capacity() {
            try_reserve_bounded(
                &mut collected,
                desired,
                maximum_count,
                maximum_storage_bytes,
                resource,
            )?;
        }
        collected.push(value);
    }
    Ok(collected)
}

fn try_reserve_bounded<T>(
    values: &mut Vec<T>,
    desired: usize,
    maximum_count: usize,
    maximum_storage_bytes: usize,
    resource: AppearanceBuildResource,
) -> Result<(), AppearanceBuildError> {
    validate_collection_size::<T>(resource, desired, maximum_count, maximum_storage_bytes)?;
    if desired <= values.capacity() {
        return Ok(());
    }

    let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), desired)
        .map_err(|_| AppearanceBuildError::AllocationFailed { resource })?;
    validate_collection_size::<T>(
        resource,
        plan.envelope_capacity(),
        maximum_count,
        maximum_storage_bytes,
    )?;
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| AppearanceBuildError::AllocationFailed { resource })?;
    plan.finish(values.len(), values.capacity())
        .map_err(|_| AppearanceBuildError::AllocationFailed { resource })?;
    Ok(())
}

fn validate_collection_size<T>(
    resource: AppearanceBuildResource,
    count: usize,
    maximum_count: usize,
    maximum_storage_bytes: usize,
) -> Result<(), AppearanceBuildError> {
    validate_limit(
        resource,
        AppearanceLimitDimension::Count,
        count,
        maximum_count,
    )?;
    let storage_bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(AppearanceBuildError::CollectionSizeOverflow { resource, count })?;
    validate_limit(
        resource,
        AppearanceLimitDimension::StorageBytes,
        storage_bytes,
        maximum_storage_bytes,
    )
}

fn validate_limit(
    resource: AppearanceBuildResource,
    dimension: AppearanceLimitDimension,
    requested: usize,
    maximum: usize,
) -> Result<(), AppearanceBuildError> {
    if requested > maximum {
        Err(AppearanceBuildError::LimitExceeded {
            resource,
            dimension,
            requested,
            maximum,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{
        ExtensionEntry, ExtensionSchemaId, ExtensionValue, MAX_EXTENSION_STRING_BYTES,
    };
    use crate::identity::TextureIndex;
    use crate::namespaced::NamespacedName;
    use crate::provenance::test_provenance_fixture;

    fn texture_index(index: usize) -> TextureIndex {
        TextureIndex::try_from(index).expect("test texture index should fit")
    }

    fn schema(name: &str) -> ExtensionSchemaId {
        ExtensionSchemaId::new(name, 1).expect("test schema should be canonical")
    }

    fn binding(
        texture: TextureIndex,
        role: TextureRole,
        color: ColorInterpretation,
        channels: TextureChannelSwizzle,
        source_key: Option<&str>,
        provenance: Provenance,
    ) -> TextureBinding {
        TextureBinding::try_new(
            texture,
            role,
            TextureCoordinateSource::uv_set(0).expect("UV set zero should be valid"),
            TextureChannelMapping::swizzle(channels),
            color,
            TextureTransform::IDENTITY,
            TextureStrength::ONE,
            TextureBlendOperation::Replace,
            source_key.map(|key| {
                TextureSourceKey::try_new(key).expect("test source key should be valid")
            }),
            provenance,
            [],
        )
        .expect("test binding should be valid")
    }

    fn unlit_material(
        bindings: impl IntoIterator<Item = TextureBinding>,
        provenance: Provenance,
    ) -> Result<Material, AppearanceBuildError> {
        Material::try_new(
            None,
            ShadingModel::Unlit,
            AlphaMode::Opaque,
            false,
            MaterialFactors::empty(),
            bindings,
            provenance,
            [],
        )
    }

    #[test]
    fn one_texture_can_be_color_and_data_without_identity_duplication() {
        let (_arena, provenance) = test_provenance_fixture();
        let texture = texture_index(7);
        let color_binding = binding(
            texture,
            TextureRole::BaseColor,
            ColorInterpretation::SrgbColor,
            TextureChannelSwizzle::RGBA,
            Some("base-color"),
            provenance,
        );
        let data_binding = binding(
            texture,
            TextureRole::Normal,
            ColorInterpretation::Data,
            TextureChannelSwizzle::RGB,
            Some("normal"),
            provenance,
        );
        let material = unlit_material([color_binding, data_binding], provenance)
            .expect("distinct roles should coexist");
        let generation = SceneGenerationId::allocate().expect("generation should allocate");
        let views = material
            .view(generation)
            .texture_bindings()
            .collect::<Vec<_>>();

        assert_eq!(views[0].texture(), views[1].texture());
        assert_eq!(views[0].texture().generation(), generation);
        assert_eq!(
            views[0].color_interpretation(),
            &ColorInterpretation::SrgbColor
        );
        assert_eq!(views[1].color_interpretation(), &ColorInterpretation::Data);
    }

    #[test]
    fn multiple_materials_reuse_the_same_typed_texture_identity() {
        let (_arena, provenance) = test_provenance_fixture();
        let texture = texture_index(4);
        let first = unlit_material(
            [binding(
                texture,
                TextureRole::BaseColor,
                ColorInterpretation::LinearColor,
                TextureChannelSwizzle::RGB,
                None,
                provenance,
            )],
            provenance,
        )
        .expect("first material should build");
        let second = unlit_material(
            [binding(
                texture,
                TextureRole::Emissive,
                ColorInterpretation::SrgbColor,
                TextureChannelSwizzle::RGB,
                None,
                provenance,
            )],
            provenance,
        )
        .expect("second material should build");
        let generation = SceneGenerationId::allocate().expect("generation should allocate");

        assert_eq!(
            first
                .view(generation)
                .texture_bindings()
                .next()
                .expect("binding should exist")
                .texture(),
            second
                .view(generation)
                .texture_bindings()
                .next()
                .expect("binding should exist")
                .texture()
        );
    }

    #[test]
    fn pbr_and_legacy_source_factors_coexist_without_overwrite() {
        let (_arena, provenance) = test_provenance_fixture();
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
        let material = Material::try_new(
            None,
            ShadingModel::PbrMetallicRoughness,
            AlphaMode::Blend,
            true,
            factors,
            [],
            provenance,
            [],
        )
        .expect("material should retain both factor authorities");

        assert_eq!(material.factors().pbr_metallic_roughness(), Some(pbr));
        assert_eq!(material.factors().legacy(), Some(legacy));
    }

    #[test]
    fn invalid_numeric_factors_and_binding_values_fail_closed() {
        assert_eq!(
            PbrMetallicRoughnessFactors::try_new(Color::WHITE, f32::NAN, 0.5),
            Err(MaterialFactorError::NonFinite {
                field: MaterialFactorField::Metallic,
            })
        );
        assert_eq!(
            PbrSpecularGlossinessFactors::try_new(Color::WHITE, Vec3::new(1.1, 1.0, 1.0), 0.5),
            Err(MaterialFactorError::OutsideUnitInterval {
                field: MaterialFactorField::PbrSpecular,
            })
        );
        assert_eq!(
            LegacyMaterialFactors::try_new(
                Vec3::ZERO,
                Vec3::ZERO,
                Vec3::ZERO,
                Vec3::ZERO,
                -1.0,
                1.0,
            ),
            Err(MaterialFactorError::Negative {
                field: MaterialFactorField::LegacyShininess,
            })
        );
        assert_eq!(
            AlphaMode::mask(1.01),
            Err(MaterialFactorError::OutsideUnitInterval {
                field: MaterialFactorField::AlphaCutoff,
            })
        );
        assert_eq!(
            TextureTransform::try_new(Vec2::ZERO, f32::INFINITY, Vec2::new(1.0, 1.0)),
            Err(TextureTransformError::NonFinite {
                component: TextureTransformComponent::Rotation,
            })
        );
        assert_eq!(
            TextureStrength::try_new(f32::NAN),
            Err(TextureStrengthError::NonFinite)
        );
    }

    #[test]
    fn known_data_roles_reject_color_interpretation() {
        let (_arena, provenance) = test_provenance_fixture();
        let result = TextureBinding::try_new(
            texture_index(0),
            TextureRole::Normal,
            TextureCoordinateSource::uv_set(0).expect("UV set should be valid"),
            TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB),
            ColorInterpretation::SrgbColor,
            TextureTransform::IDENTITY,
            TextureStrength::ONE,
            TextureBlendOperation::Replace,
            None,
            provenance,
            [],
        );

        assert_eq!(
            result,
            Err(AppearanceBuildError::IncompatibleColorInterpretation {
                role: TextureRoleKind::Normal,
                interpretation: ColorInterpretationKind::SrgbColor,
            })
        );
    }

    #[test]
    fn namespaced_behaviors_cannot_be_created_without_a_schema() {
        assert_eq!(
            TextureRole::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::TextureRole,
            })
        );
        assert_eq!(
            ShadingModel::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::ShadingModel,
            })
        );
        assert_eq!(
            AlphaMode::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::AlphaMode,
            })
        );
        assert_eq!(
            TextureCoordinateSource::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::CoordinateSource,
            })
        );
        assert_eq!(
            TextureChannelMapping::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::ChannelMapping,
            })
        );
        assert_eq!(
            ColorInterpretation::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::ColorInterpretation,
            })
        );
        assert_eq!(
            TextureBlendOperation::try_namespaced(None),
            Err(AppearanceBehaviorError::SchemaRequired {
                behavior: AppearanceBehaviorKind::BlendOperation,
            })
        );

        let admitted = schema("vendor:layered-role");
        let role = TextureRole::try_namespaced(Some(admitted.clone()))
            .expect("schema-backed role should build");
        assert_eq!(role.schema_id(), Some(&admitted));
    }

    #[test]
    fn uv_set_usage_is_bounded_but_independent_of_primitive_cardinality() {
        assert_eq!(
            TextureCoordinateSource::uv_set(37)
                .expect("a bounded source need not know a primitive")
                .uv_set_index(),
            Some(37)
        );
        assert_eq!(
            TextureCoordinateSource::uv_set(MAX_TEXTURE_COORDINATE_SET + 1),
            Err(TextureCoordinateSourceError::SetTooLarge {
                actual: MAX_TEXTURE_COORDINATE_SET + 1,
                maximum: MAX_TEXTURE_COORDINATE_SET,
            })
        );
    }

    #[test]
    fn hostile_texture_source_keys_are_rejected_without_echoing_input() {
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
    fn duplicate_binding_authority_is_rejected_in_input_order() {
        let (_arena, provenance) = test_provenance_fixture();
        let first = binding(
            texture_index(0),
            TextureRole::BaseColor,
            ColorInterpretation::SrgbColor,
            TextureChannelSwizzle::RGB,
            Some("layer-0"),
            provenance,
        );
        let duplicate = binding(
            texture_index(1),
            TextureRole::BaseColor,
            ColorInterpretation::LinearColor,
            TextureChannelSwizzle::RGBA,
            Some("layer-0"),
            provenance,
        );

        assert_eq!(
            unlit_material([first, duplicate], provenance),
            Err(AppearanceBuildError::DuplicateBindingAuthority {
                first_index: 0,
                duplicate_index: 1,
            })
        );
    }

    #[test]
    fn distinct_source_keys_admit_ordered_layers_for_one_role() {
        let (_arena, provenance) = test_provenance_fixture();
        let first = binding(
            texture_index(0),
            TextureRole::BaseColor,
            ColorInterpretation::SrgbColor,
            TextureChannelSwizzle::RGB,
            Some("layer-0"),
            provenance,
        );
        let second = binding(
            texture_index(1),
            TextureRole::BaseColor,
            ColorInterpretation::LinearColor,
            TextureChannelSwizzle::RGBA,
            Some("layer-1"),
            provenance,
        );

        assert!(unlit_material([first, second], provenance).is_ok());
    }

    #[test]
    fn binding_extensions_are_unique_and_aggregate_bounded() {
        let (_arena, provenance) = test_provenance_fixture();
        let first = ExtensionEntry::descriptive_metadata(
            NamespacedName::new("test:author").expect("key should be valid"),
            ExtensionValue::try_string("first").expect("value should be valid"),
            provenance,
        );
        let duplicate = ExtensionEntry::source_extension(
            NamespacedName::new("test:author").expect("key should be valid"),
            ExtensionValue::try_string("second").expect("value should be valid"),
            provenance,
        );
        let duplicate_result = TextureBinding::try_new(
            texture_index(0),
            TextureRole::BaseColor,
            TextureCoordinateSource::uv_set(0).expect("UV set should be valid"),
            TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB),
            ColorInterpretation::SrgbColor,
            TextureTransform::IDENTITY,
            TextureStrength::ONE,
            TextureBlendOperation::Replace,
            None,
            provenance,
            [first, duplicate],
        );
        assert_eq!(
            duplicate_result,
            Err(AppearanceBuildError::InvalidExtensions {
                owner: AppearanceExtensionOwner::TextureBinding,
                source: ExtensionEntryValidationError::DuplicateKey {
                    first_index: 0,
                    duplicate_index: 1,
                },
            })
        );

        let entry_count = MAX_TEXTURE_BINDING_EXTENSION_DYNAMIC_BYTES
            .checked_div(MAX_EXTENSION_STRING_BYTES)
            .expect("test divisor is non-zero")
            + 1;
        let aggregate_result = TextureBinding::try_new(
            texture_index(0),
            TextureRole::BaseColor,
            TextureCoordinateSource::uv_set(0).expect("UV set should be valid"),
            TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB),
            ColorInterpretation::SrgbColor,
            TextureTransform::IDENTITY,
            TextureStrength::ONE,
            TextureBlendOperation::Replace,
            None,
            provenance,
            (0..entry_count).map(|index| {
                ExtensionEntry::source_extension(
                    NamespacedName::new(format!("test:payload_{index}"))
                        .expect("key should be valid"),
                    ExtensionValue::try_string("x".repeat(MAX_EXTENSION_STRING_BYTES))
                        .expect("individual value should fit"),
                    provenance,
                )
            }),
        );
        assert_eq!(
            aggregate_result,
            Err(AppearanceBuildError::LimitExceeded {
                resource: AppearanceBuildResource::TextureBindingExtensions,
                dimension: AppearanceLimitDimension::DynamicBytes,
                requested: entry_count * MAX_EXTENSION_STRING_BYTES,
                maximum: MAX_TEXTURE_BINDING_EXTENSION_DYNAMIC_BYTES,
            })
        );
    }

    #[test]
    fn material_extensions_have_owner_local_key_uniqueness() {
        let (_arena, provenance) = test_provenance_fixture();
        let first = ExtensionEntry::descriptive_metadata(
            NamespacedName::new("test:author").expect("key should be valid"),
            ExtensionValue::try_string("first").expect("value should be valid"),
            provenance,
        );
        let duplicate = ExtensionEntry::source_extension(
            NamespacedName::new("test:author").expect("key should be valid"),
            ExtensionValue::try_string("second").expect("value should be valid"),
            provenance,
        );

        assert_eq!(
            Material::try_new(
                None,
                ShadingModel::Unlit,
                AlphaMode::Opaque,
                false,
                MaterialFactors::empty(),
                [],
                provenance,
                [first, duplicate],
            ),
            Err(AppearanceBuildError::InvalidExtensions {
                owner: AppearanceExtensionOwner::Material,
                source: ExtensionEntryValidationError::DuplicateKey {
                    first_index: 0,
                    duplicate_index: 1,
                },
            })
        );
    }

    #[test]
    fn texture_binding_count_has_a_hard_limit_before_publication() {
        let (_arena, provenance) = test_provenance_fixture();
        let bindings = (0..=MAX_TEXTURE_BINDINGS_PER_MATERIAL).map(|index| {
            binding(
                texture_index(index),
                TextureRole::BaseColor,
                ColorInterpretation::SrgbColor,
                TextureChannelSwizzle::RGB,
                Some(&format!("layer-{index}")),
                provenance,
            )
        });

        assert!(matches!(
            unlit_material(bindings, provenance),
            Err(AppearanceBuildError::LimitExceeded {
                resource: AppearanceBuildResource::TextureBindings,
                dimension: AppearanceLimitDimension::Count,
                requested,
                maximum: MAX_TEXTURE_BINDINGS_PER_MATERIAL,
            }) if requested == MAX_TEXTURE_BINDINGS_PER_MATERIAL + 1
        ));
    }

    #[test]
    fn collection_growth_rejects_capacity_envelope_before_allocator_entry() {
        let mut values = Vec::<u32>::new();

        assert_eq!(
            try_reserve_bounded(
                &mut values,
                3,
                3,
                3 * std::mem::size_of::<u32>(),
                AppearanceBuildResource::TextureBindings,
            ),
            Err(AppearanceBuildError::LimitExceeded {
                resource: AppearanceBuildResource::TextureBindings,
                dimension: AppearanceLimitDimension::Count,
                requested: 4,
                maximum: 3,
            })
        );
        assert_eq!(values.capacity(), 0);
    }

    #[test]
    fn binding_stores_a_typed_index_and_qualifies_it_only_in_the_view() {
        let (_arena, provenance) = test_provenance_fixture();
        let index = texture_index(9);
        let binding = binding(
            index,
            TextureRole::Occlusion,
            ColorInterpretation::Data,
            TextureChannelSwizzle::RED,
            None,
            provenance,
        );
        let generation = SceneGenerationId::allocate().expect("generation should allocate");

        assert_eq!(binding.texture_index(), index);
        assert_eq!(binding.texture_index().as_u32(), 9);
        assert_eq!(binding.view(generation).texture().generation(), generation);
        assert_eq!(binding.view(generation).texture().index(), 9);
    }

    #[test]
    fn schema_iterators_report_typed_behavior_references_without_allocation() {
        let (_arena, provenance) = test_provenance_fixture();
        let role_schema = schema("vendor:role");
        let blend_schema = schema("vendor:blend");
        let binding = TextureBinding::try_new(
            texture_index(0),
            TextureRole::try_namespaced(Some(role_schema.clone()))
                .expect("role schema should be accepted"),
            TextureCoordinateSource::uv_set(0).expect("UV set should be valid"),
            TextureChannelMapping::swizzle(TextureChannelSwizzle::RED),
            ColorInterpretation::Data,
            TextureTransform::IDENTITY,
            TextureStrength::ONE,
            TextureBlendOperation::try_namespaced(Some(blend_schema.clone()))
                .expect("blend schema should be accepted"),
            None,
            provenance,
            [],
        )
        .expect("namespaced binding should build");

        let uses = binding.behavior_schema_uses().collect::<Vec<_>>();
        assert_eq!(uses.len(), 2);
        assert_eq!(uses[0].behavior(), AppearanceBehaviorKind::TextureRole);
        assert_eq!(uses[0].schema_id(), &role_schema);
        assert_eq!(uses[1].behavior(), AppearanceBehaviorKind::BlendOperation);
        assert_eq!(uses[1].schema_id(), &blend_schema);
        assert!(uses.iter().all(|schema_use| {
            schema_use.expected_schema_owner() == ExtensionSchemaOwner::TextureBinding
        }));
    }
}
