//! Canonical material factors and texture-usage semantics.
//!
//! Retained scene storage owns identity, provenance, and collection layout.
//! This module defines validated appearance values and the semantic values
//! consumed while producing that storage. Only descriptive source keys borrow
//! from their producer; closed behavior values are owned directly.

use crate::math::Vec2;
use std::fmt;
use thiserror::Error;

/// Maximum UTF-8 bytes in one format-owned texture source key.
pub const MAX_TEXTURE_SOURCE_KEY_BYTES: usize = 128;

/// One primitive's authored material choice and its effective shading assignment.
///
/// The same closed value carries producer-local identities during authoring and borrowed
/// material views during reading. An implicit default has a real material owner and provenance;
/// its absence in the source remains distinct from an authored binding to equivalent factors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialAssignment<M> {
    Unspecified,
    Authored(M),
    ImplicitDefault(M),
}

impl<M> MaterialAssignment<M> {
    pub fn map<N>(self, map: impl FnOnce(M) -> N) -> MaterialAssignment<N> {
        match self {
            Self::Unspecified => MaterialAssignment::Unspecified,
            Self::Authored(material) => MaterialAssignment::Authored(map(material)),
            Self::ImplicitDefault(material) => MaterialAssignment::ImplicitDefault(map(material)),
        }
    }
}

impl<M: Copy> MaterialAssignment<M> {
    pub const fn authored(self) -> Option<M> {
        match self {
            Self::Authored(material) => Some(material),
            Self::Unspecified | Self::ImplicitDefault(_) => None,
        }
    }

    pub const fn effective(self) -> Option<M> {
        match self {
            Self::Authored(material) | Self::ImplicitDefault(material) => Some(material),
            Self::Unspecified => None,
        }
    }
}

/// The primary rendering model selected for one material.
///
/// Factor groups are retained independently, so selecting a PBR model never
/// deletes legacy source truth that was also present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadingModel {
    /// The source does not declare a rendering model.
    Unspecified,
    Unlit,
    Lambert,
    Phong,
    BlinnPhong,
    PbrMetallicRoughness,
    PbrSpecularGlossiness,
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaMode {
    Opaque,
    Mask(AlphaCutoff),
    Blend,
}

impl AlphaMode {
    pub fn mask(cutoff: f32) -> Result<Self, MaterialFactorError> {
        AlphaCutoff::try_new(cutoff).map(Self::Mask)
    }

    pub const fn mask_cutoff(self) -> Option<f32> {
        match self {
            Self::Mask(cutoff) => Some(cutoff.get()),
            Self::Opaque | Self::Blend => None,
        }
    }

    pub(crate) fn validate(self) -> Result<(), MaterialFactorError> {
        match self {
            Self::Mask(cutoff) => {
                validate_unit_factor(cutoff.get(), MaterialFactorField::AlphaCutoff)
            }
            Self::Opaque | Self::Blend => Ok(()),
        }
    }
}

mod factors;

use factors::validate_unit_factor;
pub use factors::{
    ClearcoatFactors, EmissionFactors, LegacyMaterialFactors, MaterialFactorError,
    MaterialFactorField, MaterialFactors, PbrMetallicRoughnessFactors,
    PbrSpecularGlossinessFactors, SheenFactors, ThicknessFactors, TransmissionFactors,
};

/// A canonical material texture role.
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
    Displacement,
    Clearcoat,
    ClearcoatRoughness,
    ClearcoatNormal,
    Transmission,
    Thickness,
    SheenColor,
    SheenRoughness,
}

impl TextureRole {
    pub(crate) const BUILTIN_COUNT: usize = 18;

    const fn builtin_index(self) -> usize {
        match self {
            Self::BaseColor => 0,
            Self::Diffuse => 1,
            Self::Specular => 2,
            Self::Metallic => 3,
            Self::Roughness => 4,
            Self::MetallicRoughness => 5,
            Self::Normal => 6,
            Self::Occlusion => 7,
            Self::Emissive => 8,
            Self::Opacity => 9,
            Self::Displacement => 10,
            Self::Clearcoat => 11,
            Self::ClearcoatRoughness => 12,
            Self::ClearcoatNormal => 13,
            Self::Transmission => 14,
            Self::Thickness => 15,
            Self::SheenColor => 16,
            Self::SheenRoughness => 17,
        }
    }

    const fn usage_class(self) -> TextureUsageClass {
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
        }
    }

    const fn channel_lane_range(self) -> (u8, u8) {
        match self {
            Self::BaseColor | Self::Diffuse => (3, 4),
            Self::Specular => (3, 3),
            Self::Emissive | Self::SheenColor => (3, 3),
            Self::MetallicRoughness => (2, 2),
            Self::Normal | Self::ClearcoatNormal => (2, 3),
            Self::Metallic
            | Self::Roughness
            | Self::Occlusion
            | Self::Opacity
            | Self::Displacement
            | Self::Clearcoat
            | Self::ClearcoatRoughness
            | Self::Transmission
            | Self::Thickness
            | Self::SheenRoughness => (1, 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextureUsageClass {
    Color,
    Data,
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

    pub fn try_new(channels: &[TextureChannel]) -> Result<Self, TextureChannelSwizzleError> {
        if channels.is_empty() {
            return Err(TextureChannelSwizzleError::Empty);
        }
        if channels.len() > 4 {
            return Err(TextureChannelSwizzleError::TooManyLanes {
                actual: channels.len(),
                maximum: 4,
            });
        }

        let mut canonical = [TextureChannel::Zero; 4];
        canonical[..channels.len()].copy_from_slice(channels);
        let lane_count =
            u8::try_from(channels.len()).map_err(|_| TextureChannelSwizzleError::TooManyLanes {
                actual: channels.len(),
                maximum: 4,
            })?;
        Ok(Self {
            channels: canonical,
            lane_count,
        })
    }

    pub fn as_slice(&self) -> &[TextureChannel] {
        &self.channels[..usize::from(self.lane_count)]
    }

    pub const fn lane_count(self) -> u8 {
        self.lane_count
    }

    pub(crate) fn is_valid(self) -> bool {
        self.channels
            .get(..usize::from(self.lane_count))
            .is_some_and(|channels| Self::try_new(channels).is_ok())
    }

    #[cfg(test)]
    pub(crate) fn corrupt_lane_count_for_test(&mut self, count: u8) {
        self.lane_count = count;
    }
}

/// Why a fixed channel swizzle was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TextureChannelSwizzleError {
    #[error("texture channel swizzle must contain at least one output lane")]
    Empty,

    #[error("texture channel swizzle has {actual} lanes; the maximum is {maximum}")]
    TooManyLanes { actual: usize, maximum: usize },
}

/// How sampled values are interpreted at one material usage site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorInterpretation {
    SrgbColor,
    LinearColor,
    Data,
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

    pub(crate) fn validate(self) -> Result<(), TextureTransformError> {
        Self::try_new(self.offset, self.rotation_radians, self.scale).map(|_| ())
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

    pub(crate) fn validate(self) -> Result<(), TextureStrengthError> {
        Self::try_new(self.0).map(|_| ())
    }
}

/// Why a texture strength was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TextureStrengthError {
    #[error("texture strength must be finite")]
    NonFinite,
}

/// Ordered blend behavior for a texture binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureBlendOperation {
    Replace,
    Multiply,
    Add,
    Subtract,
    Screen,
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
        validate_texture_source_key(value)?;

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

/// Shares the bounded text contract with directly borrowed retained source-key ranges.
pub(crate) fn validate_texture_source_key(value: &str) -> Result<(), TextureSourceKeyError> {
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
    Ok(())
}

/// Material-use values independent from texture identity and storage layout.
///
/// Closed behavior values are owned; only the descriptive source key is borrowed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextureBindingSemantics<'a> {
    role: TextureRole,
    uv_set: u16,
    channel_mapping: TextureChannelSwizzle,
    color_interpretation: ColorInterpretation,
    transform: TextureTransform,
    strength: TextureStrength,
    blend: TextureBlendOperation,
    source_key: Option<&'a TextureSourceKey>,
}

impl<'a> TextureBindingSemantics<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        role: TextureRole,
        uv_set: u16,
        channel_mapping: TextureChannelSwizzle,
        color_interpretation: ColorInterpretation,
        transform: TextureTransform,
        strength: TextureStrength,
        blend: TextureBlendOperation,
        source_key: Option<&'a TextureSourceKey>,
    ) -> Self {
        Self {
            role,
            uv_set,
            channel_mapping,
            color_interpretation,
            transform,
            strength,
            blend,
            source_key,
        }
    }

    pub(crate) const fn role(self) -> TextureRole {
        self.role
    }

    /// Returns the zero-based UV set, independent of any primitive's UV stream count.
    pub(crate) const fn uv_set(self) -> u16 {
        self.uv_set
    }

    pub(crate) const fn channel_mapping(self) -> TextureChannelSwizzle {
        self.channel_mapping
    }

    pub(crate) const fn color_interpretation(self) -> ColorInterpretation {
        self.color_interpretation
    }

    pub(crate) const fn transform(self) -> TextureTransform {
        self.transform
    }

    pub(crate) const fn strength(self) -> TextureStrength {
        self.strength
    }

    pub(crate) const fn blend(self) -> TextureBlendOperation {
        self.blend
    }

    pub(crate) const fn source_key(self) -> Option<&'a TextureSourceKey> {
        self.source_key
    }
}

/// Owned material-level values independent from retained representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MaterialSemantics {
    shading_model: ShadingModel,
    alpha_mode: AlphaMode,
    double_sided: bool,
    factors: MaterialFactors,
}

impl MaterialSemantics {
    pub(crate) const fn new(
        shading_model: ShadingModel,
        alpha_mode: AlphaMode,
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

    pub(crate) const fn shading_model(self) -> ShadingModel {
        self.shading_model
    }

    pub(crate) const fn alpha_mode(self) -> AlphaMode {
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
        shading: ShadingModel,
        required: MaterialFactorGroup,
    },

    #[error("texture role {role:?} does not admit color interpretation {interpretation:?}")]
    IncompatibleColorInterpretation {
        role: TextureRole,
        interpretation: ColorInterpretation,
    },

    #[error("texture role {role:?} requires {minimum}..={maximum} mapped lanes, found {actual}")]
    IncompatibleChannelCount {
        role: TextureRole,
        minimum: u8,
        maximum: u8,
        actual: u8,
    },

    #[error("texture role {role:?} violates its {requirement:?} strength requirement")]
    IncompatibleStrength {
        role: TextureRole,
        requirement: TextureStrengthRequirement,
    },

    #[error("built-in texture role {role:?} requires Replace blend semantics")]
    BuiltInRoleRequiresReplace { role: TextureRole },

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
        first_role: TextureRole,
        duplicate_index: usize,
        duplicate_role: TextureRole,
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
    semantics: MaterialSemantics,
) -> Result<(), AppearanceBuildError> {
    let shading_model = semantics.shading_model();
    let factors = semantics.factors();
    let required = match shading_model {
        ShadingModel::PbrMetallicRoughness if factors.pbr_metallic_roughness().is_none() => {
            Some(MaterialFactorGroup::PbrMetallicRoughness)
        }
        ShadingModel::PbrSpecularGlossiness if factors.pbr_specular_glossiness().is_none() => {
            Some(MaterialFactorGroup::PbrSpecularGlossiness)
        }
        ShadingModel::Lambert | ShadingModel::Phong | ShadingModel::BlinnPhong
            if factors.legacy().is_none() =>
        {
            Some(MaterialFactorGroup::Legacy)
        }
        ShadingModel::Unspecified
        | ShadingModel::Unlit
        | ShadingModel::Lambert
        | ShadingModel::Phong
        | ShadingModel::BlinnPhong
        | ShadingModel::PbrMetallicRoughness
        | ShadingModel::PbrSpecularGlossiness => None,
    };

    if let Some(required) = required {
        return Err(AppearanceBuildError::MissingFactorGroup {
            shading: shading_model,
            required,
        });
    }
    Ok(())
}

fn validate_binding_color_interpretation(
    role: TextureRole,
    interpretation: ColorInterpretation,
) -> Result<(), AppearanceBuildError> {
    let compatible = match (role.usage_class(), interpretation) {
        (TextureUsageClass::Color, ColorInterpretation::SrgbColor)
        | (TextureUsageClass::Color, ColorInterpretation::LinearColor)
        | (TextureUsageClass::Data, ColorInterpretation::Data) => true,
        (TextureUsageClass::Color, ColorInterpretation::Data)
        | (TextureUsageClass::Data, ColorInterpretation::SrgbColor)
        | (TextureUsageClass::Data, ColorInterpretation::LinearColor) => false,
    };

    if compatible {
        Ok(())
    } else {
        Err(AppearanceBuildError::IncompatibleColorInterpretation {
            role,
            interpretation,
        })
    }
}

pub(crate) fn validate_texture_binding_semantics(
    semantics: TextureBindingSemantics<'_>,
) -> Result<(), AppearanceBuildError> {
    validate_binding_color_interpretation(semantics.role(), semantics.color_interpretation())?;
    validate_binding_channel_mapping(semantics.role(), semantics.channel_mapping())?;
    validate_binding_strength(semantics.role(), semantics.strength())?;
    validate_binding_blend(semantics.role(), semantics.blend())
}

fn validate_binding_channel_mapping(
    role: TextureRole,
    mapping: TextureChannelSwizzle,
) -> Result<(), AppearanceBuildError> {
    let (minimum, maximum) = role.channel_lane_range();
    let actual = mapping.lane_count();
    if (minimum..=maximum).contains(&actual) {
        Ok(())
    } else {
        Err(AppearanceBuildError::IncompatibleChannelCount {
            role,
            minimum,
            maximum,
            actual,
        })
    }
}

fn validate_binding_strength(
    role: TextureRole,
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
        Err(AppearanceBuildError::IncompatibleStrength { role, requirement })
    }
}

fn validate_binding_blend(
    role: TextureRole,
    blend: TextureBlendOperation,
) -> Result<(), AppearanceBuildError> {
    if matches!(blend, TextureBlendOperation::Replace) {
        Ok(())
    } else {
        Err(AppearanceBuildError::BuiltInRoleRequiresReplace { role })
    }
}

pub(crate) struct MaterialTextureBindingSemanticsValidator {
    binding_count: usize,
    next_index: usize,
    builtin_first_indices: [Option<usize>; TextureRole::BUILTIN_COUNT],
}

impl MaterialTextureBindingSemanticsValidator {
    pub(crate) fn new(binding_count: usize) -> Self {
        Self {
            binding_count,
            next_index: 0,
            builtin_first_indices: [None; TextureRole::BUILTIN_COUNT],
        }
    }

    pub(crate) fn validate_next(
        &mut self,
        binding: TextureBindingSemantics<'_>,
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
        let role = binding.role();
        let first_index = &mut self.builtin_first_indices[role.builtin_index()];
        if let Some(first_index) = *first_index {
            return Err(AppearanceBuildError::DuplicateBindingAuthority {
                first_index,
                duplicate_index,
            });
        }
        *first_index = Some(duplicate_index);

        let conflict = match role {
            TextureRole::Metallic => self.builtin_first_indices
                [TextureRole::MetallicRoughness.builtin_index()]
            .map(|index| (index, TextureRole::MetallicRoughness)),
            TextureRole::Roughness => self.builtin_first_indices
                [TextureRole::MetallicRoughness.builtin_index()]
            .map(|index| (index, TextureRole::MetallicRoughness)),
            TextureRole::MetallicRoughness => self.builtin_first_indices
                [TextureRole::Metallic.builtin_index()]
            .map(|index| (index, TextureRole::Metallic))
            .or_else(|| {
                self.builtin_first_indices[TextureRole::Roughness.builtin_index()]
                    .map(|index| (index, TextureRole::Roughness))
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
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn validate_material_texture_binding_semantics<'a>(
    bindings: impl ExactSizeIterator<Item = TextureBindingSemantics<'a>>,
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
    use crate::math::{Color, Vec3};

    const UV_ZERO: u16 = 0;
    const REPLACE: TextureBlendOperation = TextureBlendOperation::Replace;

    fn binding_semantics<'a>(
        role: TextureRole,
        channel_mapping: TextureChannelSwizzle,
        color_interpretation: ColorInterpretation,
        strength: TextureStrength,
        blend: TextureBlendOperation,
        source_key: Option<&'a TextureSourceKey>,
    ) -> TextureBindingSemantics<'a> {
        TextureBindingSemantics::new(
            role,
            UV_ZERO,
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
            validate_material_semantics(MaterialSemantics::new(
                shading_model,
                alpha_mode,
                false,
                MaterialFactors::empty(),
            )),
            Err(AppearanceBuildError::MissingFactorGroup {
                shading: ShadingModel::PbrMetallicRoughness,
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
        let factors = MaterialFactors::empty()
            .with_pbr_metallic_roughness(Some(pbr))
            .with_legacy(Some(legacy));
        let semantics = MaterialSemantics::new(shading_model, alpha_mode, true, factors);

        validate_material_semantics(semantics).expect("selected factor group is present");
        assert_eq!(semantics.factors().pbr_metallic_roughness(), Some(pbr));
        assert_eq!(semantics.factors().legacy(), Some(legacy));
        assert!(semantics.double_sided());
    }

    #[test]
    fn semantics_own_closed_values_and_borrow_only_the_descriptive_source_key() {
        let material = {
            let shading_model = ShadingModel::Unlit;
            let alpha_mode = AlphaMode::mask(-0.0).unwrap();
            let factors = MaterialFactors::empty()
                .with_emission(EmissionFactors::try_new(Vec3::new(2.0, -0.0, 0.5), None).unwrap());
            MaterialSemantics::new(shading_model, alpha_mode, true, factors)
        };
        validate_material_semantics(material).unwrap();
        assert_eq!(material.shading_model(), ShadingModel::Unlit);
        assert_eq!(
            material.alpha_mode().mask_cutoff().unwrap().to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(
            material.factors().emission().factor().y.to_bits(),
            (-0.0_f32).to_bits()
        );

        let source_key = TextureSourceKey::try_new("source:normal").unwrap();
        let binding = {
            let role = TextureRole::Normal;
            let uv_set = 3;
            let channel_mapping = TextureChannelSwizzle::RGB;
            let color_interpretation = ColorInterpretation::Data;
            let blend = TextureBlendOperation::Replace;
            TextureBindingSemantics::new(
                role,
                uv_set,
                channel_mapping,
                color_interpretation,
                TextureTransform::IDENTITY,
                TextureStrength::try_new(-0.0).unwrap(),
                blend,
                Some(&source_key),
            )
        };
        validate_texture_binding_semantics(binding).unwrap();
        assert_eq!(binding.role(), TextureRole::Normal);
        assert_eq!(binding.uv_set(), 3);
        assert_eq!(binding.channel_mapping(), TextureChannelSwizzle::RGB);
        assert_eq!(binding.color_interpretation(), ColorInterpretation::Data);
        assert_eq!(binding.blend(), TextureBlendOperation::Replace);
        assert_eq!(binding.strength().get().to_bits(), (-0.0_f32).to_bits());
        assert!(std::ptr::eq(binding.source_key().unwrap(), &source_key));
    }

    #[test]
    fn public_authoring_constructors_borrow_only_names_and_source_keys() {
        use crate::__scene_integration::authoring::{
            AuthoringRange, MaterialAuthoring, MaterialBindingAuthoring, Presence, TextureLocalId,
        };

        fn material<'name>(name: Presence<&'name str>) -> MaterialAuthoring<'name> {
            let material = MaterialAuthoring::try_from_parts(
                AuthoringRange::new(0, 0),
                ShadingModel::Unlit,
                AlphaMode::mask(-0.0).unwrap(),
                true,
                MaterialFactors::empty(),
            )
            .unwrap();
            match name {
                Presence::Absent => material,
                Presence::Present(name) => material.with_name(name),
            }
        }

        fn binding<'key>(
            source_key: Option<&'key TextureSourceKey>,
        ) -> MaterialBindingAuthoring<'key> {
            MaterialBindingAuthoring::try_from_parts(
                TextureLocalId::new(0),
                TextureRole::Normal,
                3,
                TextureChannelSwizzle::RGB,
                ColorInterpretation::Data,
                TextureTransform::IDENTITY,
                TextureStrength::try_new(-0.0).unwrap(),
                TextureBlendOperation::Replace,
                source_key,
            )
            .unwrap()
        }

        // Both return paths escape their constructors without borrowing enum temporaries.
        let unnamed: MaterialAuthoring<'static> = material(Presence::Absent);
        let unkeyed: MaterialBindingAuthoring<'static> = binding(None);
        {
            let name = String::from("caller-owned material name");
            let source_key = TextureSourceKey::try_new("source:normal").unwrap();
            assert_ne!(material(Presence::Present(name.as_str())), unnamed);
            assert_ne!(binding(Some(&source_key)), unkeyed);
        }
        assert_eq!(unnamed, material(Presence::Absent));
        assert_eq!(unkeyed, binding(None));
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
        let rgb = TextureChannelSwizzle::RGB;
        let srgb = ColorInterpretation::SrgbColor;
        assert_eq!(
            validate_texture_binding_semantics(binding_semantics(
                normal,
                rgb,
                srgb,
                TextureStrength::ONE,
                REPLACE,
                None,
            )),
            Err(AppearanceBuildError::IncompatibleColorInterpretation {
                role: TextureRole::Normal,
                interpretation: ColorInterpretation::SrgbColor,
            })
        );

        let specular = TextureRole::Specular;
        let rgba = TextureChannelSwizzle::RGBA;
        let linear = ColorInterpretation::LinearColor;
        assert_eq!(
            validate_texture_binding_semantics(binding_semantics(
                specular,
                rgba,
                linear,
                TextureStrength::ONE,
                REPLACE,
                None,
            )),
            Err(AppearanceBuildError::IncompatibleChannelCount {
                role: TextureRole::Specular,
                minimum: 3,
                maximum: 3,
                actual: 4,
            })
        );

        let metallic = TextureRole::Metallic;
        let red = TextureChannelSwizzle::RED;
        let data = ColorInterpretation::Data;
        assert_eq!(
            validate_texture_binding_semantics(binding_semantics(
                metallic,
                red,
                data,
                TextureStrength::try_new(0.5).expect("finite strength should build"),
                REPLACE,
                None,
            )),
            Err(AppearanceBuildError::IncompatibleStrength {
                role: TextureRole::Metallic,
                requirement: TextureStrengthRequirement::ExactlyOne,
            })
        );

        let base_color = TextureRole::BaseColor;
        let multiply = TextureBlendOperation::Multiply;
        assert_eq!(
            validate_texture_binding_semantics(binding_semantics(
                base_color,
                rgb,
                srgb,
                TextureStrength::ONE,
                multiply,
                None,
            )),
            Err(AppearanceBuildError::BuiltInRoleRequiresReplace {
                role: TextureRole::BaseColor,
            })
        );
    }

    #[test]
    fn material_validator_rejects_duplicate_roles_independently_of_source_key() {
        let builtin_role = TextureRole::BaseColor;
        let rgb = TextureChannelSwizzle::RGB;
        let color = ColorInterpretation::SrgbColor;
        let first_key = TextureSourceKey::try_new("layer-0").expect("source key should be valid");
        let second_key = TextureSourceKey::try_new("layer-1").expect("source key should be valid");
        let first = binding_semantics(
            builtin_role,
            rgb,
            color,
            TextureStrength::ONE,
            REPLACE,
            Some(&first_key),
        );
        let duplicate = binding_semantics(
            builtin_role,
            rgb,
            color,
            TextureStrength::ONE,
            REPLACE,
            Some(&second_key),
        );
        assert_eq!(
            validate_material_texture_binding_semantics([first, duplicate].into_iter()),
            Err(AppearanceBuildError::DuplicateBindingAuthority {
                first_index: 0,
                duplicate_index: 1,
            })
        );
    }

    #[test]
    fn material_validator_rejects_role_conflicts_and_count_overrun() {
        let combined_role = TextureRole::MetallicRoughness;
        let combined_channels =
            TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green])
                .expect("two-lane mapping should be valid");
        let metallic_role = TextureRole::Metallic;
        let metallic_mapping = TextureChannelSwizzle::RED;
        let data = ColorInterpretation::Data;
        let combined = binding_semantics(
            combined_role,
            combined_channels,
            data,
            TextureStrength::ONE,
            REPLACE,
            None,
        );
        let separate = binding_semantics(
            metallic_role,
            metallic_mapping,
            data,
            TextureStrength::ONE,
            REPLACE,
            None,
        );
        assert_eq!(
            validate_material_texture_binding_semantics([combined, separate].into_iter()),
            Err(AppearanceBuildError::ConflictingBindingRoles {
                first_index: 0,
                first_role: TextureRole::MetallicRoughness,
                duplicate_index: 1,
                duplicate_role: TextureRole::Metallic,
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

    #[test]
    fn swizzle_is_the_direct_copy_value_and_preserves_source_lane_order() {
        fn require_copy<T: Copy>(value: T) -> T {
            value
        }
        let expected = [
            TextureChannel::Blue,
            TextureChannel::Green,
            TextureChannel::One,
            TextureChannel::Zero,
        ];
        let swizzle = require_copy(TextureChannelSwizzle::try_new(&expected).unwrap());
        let semantics = binding_semantics(
            TextureRole::BaseColor,
            swizzle,
            ColorInterpretation::LinearColor,
            TextureStrength::ONE,
            REPLACE,
            None,
        );
        assert_eq!(semantics.channel_mapping().as_slice(), expected);
        assert_eq!(semantics.channel_mapping().lane_count(), 4);
        assert!(swizzle.is_valid());
        assert_eq!(
            TextureChannelSwizzle::try_new(&[]),
            Err(TextureChannelSwizzleError::Empty)
        );
        assert_eq!(
            TextureChannelSwizzle::try_new(&[TextureChannel::Red; 5]),
            Err(TextureChannelSwizzleError::TooManyLanes {
                actual: 5,
                maximum: 4
            })
        );
        let mut corrupt = swizzle;
        corrupt.lane_count = 0;
        assert!(!corrupt.is_valid());
        corrupt.lane_count = 5;
        assert!(!corrupt.is_valid());
        require_copy(ShadingModel::Unspecified);
        require_copy(AlphaMode::Opaque);
        require_copy(TextureRole::Normal);
        require_copy(ColorInterpretation::Data);
        require_copy(TextureBlendOperation::Replace);
    }

    #[test]
    fn actual_numeric_values_revalidate_without_normalizing_or_allocating() {
        let mut factors = MaterialFactors::empty()
            .with_emission(EmissionFactors::try_new(Vec3::new(2.0, -0.0, 0.5), None).unwrap())
            .with_pbr_metallic_roughness(Some(PbrMetallicRoughnessFactors::default()))
            .with_pbr_specular_glossiness(Some(
                PbrSpecularGlossinessFactors::try_new(Color::WHITE, Vec3::new(0.5, 0.5, 0.5), 0.5)
                    .unwrap(),
            ))
            .with_legacy(Some(
                LegacyMaterialFactors::try_new(
                    Vec3::ZERO,
                    Vec3::new(3.0, 2.0, 1.0),
                    Vec3::ZERO,
                    Vec3::ZERO,
                    64.0,
                    1.0,
                )
                .unwrap(),
            ));
        let transform =
            TextureTransform::try_new(Vec2::new(-0.0, -2.0), 7.0, Vec2::new(-3.0, 0.0)).unwrap();
        let allocation = allocation_counter::measure(|| {
            assert!(factors.validate().is_ok());
            assert!(AlphaMode::mask(-0.0).unwrap().validate().is_ok());
            assert!(transform.validate().is_ok());
            assert!(TextureStrength::try_new(-2.0).unwrap().validate().is_ok());
            assert!(validate_texture_source_key("source:custom slot").is_ok());
        });
        assert_eq!(allocation.count_total, 0);
        assert_eq!(
            factors.emission().factor().y.to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(transform.offset().x.to_bits(), (-0.0_f32).to_bits());
        factors.corrupt_glossiness_for_test();
        assert!(factors.validate().is_err());
        assert!(
            AlphaMode::Mask(AlphaCutoff(f32::INFINITY))
                .validate()
                .is_err()
        );
        assert!(TextureStrength(f32::INFINITY).validate().is_err());
        assert!(
            TextureTransform {
                scale: Vec2::new(f32::NAN, 1.0),
                ..transform
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn opposite_role_conflict_order_and_distinct_materials_keep_the_same_rules() {
        let channels =
            TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green]).unwrap();
        let combined = binding_semantics(
            TextureRole::MetallicRoughness,
            channels,
            ColorInterpretation::Data,
            TextureStrength::ONE,
            REPLACE,
            None,
        );
        let separate = binding_semantics(
            TextureRole::Roughness,
            TextureChannelSwizzle::RED,
            ColorInterpretation::Data,
            TextureStrength::ONE,
            REPLACE,
            None,
        );
        assert_eq!(
            validate_material_texture_binding_semantics([separate, combined].into_iter()),
            Err(AppearanceBuildError::ConflictingBindingRoles {
                first_index: 0,
                first_role: TextureRole::Roughness,
                duplicate_index: 1,
                duplicate_role: TextureRole::MetallicRoughness,
            })
        );
        assert!(validate_material_texture_binding_semantics([separate].into_iter()).is_ok());
        assert!(validate_material_texture_binding_semantics([separate].into_iter()).is_ok());
    }
}
