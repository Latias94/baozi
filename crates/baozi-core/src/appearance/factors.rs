//! Checked inline material factors with independent authored presence.

use crate::math::{Color, Vec3};
use thiserror::Error;

/// A numeric material field validated at a typed construction boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialFactorField {
    AlphaCutoff,
    Emissive,
    EmissiveStrength,
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

/// Raw emission RGB and its independently authored, dimensionless multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EmissionFactors {
    factor: Vec3,
    strength: Option<f32>,
}

impl EmissionFactors {
    pub fn try_new(factor: Vec3, strength: Option<f32>) -> Result<Self, MaterialFactorError> {
        validate_nonnegative_vec3(factor, MaterialFactorField::Emissive)?;
        if let Some(strength) = strength {
            validate_nonnegative_factor(strength, MaterialFactorField::EmissiveStrength)?;
        }
        Ok(Self { factor, strength })
    }

    pub const fn factor(self) -> Vec3 {
        self.factor
    }

    pub const fn strength(self) -> Option<f32> {
        self.strength
    }

    pub const fn effective_strength(self) -> f32 {
        match self.strength {
            Some(strength) => strength,
            None => 1.0,
        }
    }

    /// Widens both operands before multiplication so finite HDR inputs remain finite.
    /// This derived value does not replace the authored factor or strength.
    pub fn effective_rgb(self) -> [f64; 3] {
        let strength = f64::from(self.effective_strength());
        [self.factor.x, self.factor.y, self.factor.z].map(|value| f64::from(value) * strength)
    }
}

/// Dimensionless clearcoat factor and roughness, each in the closed unit interval.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClearcoatFactors {
    factor: f32,
    roughness: f32,
}

impl ClearcoatFactors {
    pub fn try_new(factor: f32, roughness: f32) -> Result<Self, MaterialFactorError> {
        validate_unit_factor(factor, MaterialFactorField::Clearcoat)?;
        validate_unit_factor(roughness, MaterialFactorField::ClearcoatRoughness)?;
        Ok(Self { factor, roughness })
    }

    pub const fn factor(self) -> f32 {
        self.factor
    }

    pub const fn roughness(self) -> f32 {
        self.roughness
    }
}

/// Dimensionless transmission in the closed unit interval, independent of thickness.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TransmissionFactors {
    factor: f32,
}

impl TransmissionFactors {
    pub fn try_new(factor: f32) -> Result<Self, MaterialFactorError> {
        validate_unit_factor(factor, MaterialFactorField::Transmission)?;
        Ok(Self { factor })
    }

    pub const fn factor(self) -> f32 {
        self.factor
    }
}

/// Finite nonnegative thickness in the Scene's current length unit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ThicknessFactors {
    thickness: f32,
}

impl ThicknessFactors {
    pub fn try_new(thickness: f32) -> Result<Self, MaterialFactorError> {
        validate_nonnegative_factor(thickness, MaterialFactorField::Thickness)?;
        Ok(Self { thickness })
    }

    pub const fn thickness(self) -> f32 {
        self.thickness
    }
}

/// Finite nonnegative HDR sheen color and roughness in the closed unit interval.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SheenFactors {
    color: Vec3,
    roughness: f32,
}

impl SheenFactors {
    pub fn try_new(color: Vec3, roughness: f32) -> Result<Self, MaterialFactorError> {
        validate_nonnegative_vec3(color, MaterialFactorField::SheenColor)?;
        validate_unit_factor(roughness, MaterialFactorField::SheenRoughness)?;
        Ok(Self { color, roughness })
    }

    pub const fn color(self) -> Vec3 {
        self.color
    }

    pub const fn roughness(self) -> f32 {
        self.roughness
    }
}

/// Independent inline factor groups. Present zero leaves retain authored presence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialFactors {
    emission: EmissionFactors,
    pbr_metallic_roughness: Option<PbrMetallicRoughnessFactors>,
    pbr_specular_glossiness: Option<PbrSpecularGlossinessFactors>,
    legacy: Option<LegacyMaterialFactors>,
    clearcoat: Option<ClearcoatFactors>,
    transmission: Option<TransmissionFactors>,
    thickness: Option<ThicknessFactors>,
    sheen: Option<SheenFactors>,
}

impl MaterialFactors {
    pub const fn empty() -> Self {
        Self {
            emission: EmissionFactors {
                factor: Vec3::ZERO,
                strength: None,
            },
            pbr_metallic_roughness: None,
            pbr_specular_glossiness: None,
            legacy: None,
            clearcoat: None,
            transmission: None,
            thickness: None,
            sheen: None,
        }
    }

    pub const fn with_emission(mut self, value: EmissionFactors) -> Self {
        self.emission = value;
        self
    }

    pub const fn with_pbr_metallic_roughness(
        mut self,
        value: Option<PbrMetallicRoughnessFactors>,
    ) -> Self {
        self.pbr_metallic_roughness = value;
        self
    }

    pub const fn with_pbr_specular_glossiness(
        mut self,
        value: Option<PbrSpecularGlossinessFactors>,
    ) -> Self {
        self.pbr_specular_glossiness = value;
        self
    }

    pub const fn with_legacy(mut self, value: Option<LegacyMaterialFactors>) -> Self {
        self.legacy = value;
        self
    }

    pub const fn with_clearcoat(mut self, value: Option<ClearcoatFactors>) -> Self {
        self.clearcoat = value;
        self
    }

    pub const fn with_transmission(mut self, value: Option<TransmissionFactors>) -> Self {
        self.transmission = value;
        self
    }

    pub const fn with_thickness(mut self, value: Option<ThicknessFactors>) -> Self {
        self.thickness = value;
        self
    }

    pub const fn with_sheen(mut self, value: Option<SheenFactors>) -> Self {
        self.sheen = value;
        self
    }

    pub const fn emission(self) -> EmissionFactors {
        self.emission
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

    pub const fn clearcoat(self) -> Option<ClearcoatFactors> {
        self.clearcoat
    }

    pub const fn transmission(self) -> Option<TransmissionFactors> {
        self.transmission
    }

    pub const fn thickness(self) -> Option<ThicknessFactors> {
        self.thickness
    }

    pub const fn sheen(self) -> Option<SheenFactors> {
        self.sheen
    }

    /// Rechecks actual inline values, including retained inactive fallback groups.
    pub(crate) fn validate(self) -> Result<(), MaterialFactorError> {
        EmissionFactors::try_new(self.emission.factor, self.emission.strength)?;
        if let Some(value) = self.pbr_metallic_roughness {
            PbrMetallicRoughnessFactors::try_new(
                value.base_color,
                value.metallic,
                value.roughness,
            )?;
        }
        if let Some(value) = self.pbr_specular_glossiness {
            PbrSpecularGlossinessFactors::try_new(value.diffuse, value.specular, value.glossiness)?;
        }
        if let Some(value) = self.legacy {
            LegacyMaterialFactors::try_new(
                value.ambient,
                value.diffuse,
                value.specular,
                value.emissive,
                value.shininess,
                value.opacity,
            )?;
        }
        if let Some(value) = self.clearcoat {
            ClearcoatFactors::try_new(value.factor, value.roughness)?;
        }
        if let Some(value) = self.transmission {
            TransmissionFactors::try_new(value.factor)?;
        }
        if let Some(value) = self.thickness {
            ThicknessFactors::try_new(value.thickness)?;
        }
        if let Some(value) = self.sheen {
            SheenFactors::try_new(value.color, value.roughness)?;
        }
        Ok(())
    }

    /// Counts semantic numeric cells, excluding absent padding and presence discriminants.
    pub(crate) fn semantic_cell_count(self) -> u64 {
        3 + u64::from(self.emission.strength.is_some())
            + u64::from(self.pbr_metallic_roughness.is_some()) * 6
            + u64::from(self.pbr_specular_glossiness.is_some()) * 8
            + u64::from(self.legacy.is_some()) * 14
            + u64::from(self.clearcoat.is_some()) * 2
            + u64::from(self.transmission.is_some())
            + u64::from(self.thickness.is_some())
            + u64::from(self.sheen.is_some()) * 4
    }

    /// Concrete presence and bit correspondence, independent of snapshot projection.
    pub(crate) fn same_bits(self, other: Self) -> bool {
        let metallic_roughness = |value: PbrMetallicRoughnessFactors| {
            let color = value.base_color;
            [
                color.r,
                color.g,
                color.b,
                color.a,
                value.metallic,
                value.roughness,
            ]
            .map(f32::to_bits)
        };
        let specular_glossiness = |value: PbrSpecularGlossinessFactors| {
            let color = value.diffuse;
            let specular = value.specular;
            [
                color.r,
                color.g,
                color.b,
                color.a,
                specular.x,
                specular.y,
                specular.z,
                value.glossiness,
            ]
            .map(f32::to_bits)
        };
        let legacy = |value: LegacyMaterialFactors| {
            [
                value.ambient.x,
                value.ambient.y,
                value.ambient.z,
                value.diffuse.x,
                value.diffuse.y,
                value.diffuse.z,
                value.specular.x,
                value.specular.y,
                value.specular.z,
                value.emissive.x,
                value.emissive.y,
                value.emissive.z,
                value.shininess,
                value.opacity,
            ]
            .map(f32::to_bits)
        };
        let clearcoat = |value: ClearcoatFactors| [value.factor, value.roughness].map(f32::to_bits);
        let sheen = |value: SheenFactors| {
            [value.color.x, value.color.y, value.color.z, value.roughness].map(f32::to_bits)
        };
        [
            self.emission.factor.x,
            self.emission.factor.y,
            self.emission.factor.z,
        ]
        .map(f32::to_bits)
            == [
                other.emission.factor.x,
                other.emission.factor.y,
                other.emission.factor.z,
            ]
            .map(f32::to_bits)
            && self.emission.strength.map(f32::to_bits) == other.emission.strength.map(f32::to_bits)
            && self.pbr_metallic_roughness.map(metallic_roughness)
                == other.pbr_metallic_roughness.map(metallic_roughness)
            && self.pbr_specular_glossiness.map(specular_glossiness)
                == other.pbr_specular_glossiness.map(specular_glossiness)
            && self.legacy.map(legacy) == other.legacy.map(legacy)
            && self.clearcoat.map(clearcoat) == other.clearcoat.map(clearcoat)
            && self.transmission.map(|value| value.factor.to_bits())
                == other.transmission.map(|value| value.factor.to_bits())
            && self.thickness.map(|value| value.thickness.to_bits())
                == other.thickness.map(|value| value.thickness.to_bits())
            && self.sheen.map(sheen) == other.sheen.map(sheen)
    }

    #[cfg(test)]
    pub(crate) fn corrupt_emissive_for_test(&mut self) {
        self.emission.factor.x = f32::NAN;
    }

    #[cfg(test)]
    pub(crate) fn corrupt_glossiness_for_test(&mut self) {
        self.pbr_specular_glossiness.as_mut().unwrap().glossiness = f32::NAN;
    }

    #[cfg(test)]
    pub(crate) fn corrupt_optional_factor_for_test(&mut self, field: MaterialFactorField) {
        match field {
            MaterialFactorField::EmissiveStrength => self.emission.strength = Some(f32::NAN),
            MaterialFactorField::Clearcoat => self.clearcoat.as_mut().unwrap().factor = f32::NAN,
            MaterialFactorField::ClearcoatRoughness => {
                self.clearcoat.as_mut().unwrap().roughness = f32::NAN
            }
            MaterialFactorField::Transmission => {
                self.transmission.as_mut().unwrap().factor = f32::NAN
            }
            MaterialFactorField::Thickness => self.thickness.as_mut().unwrap().thickness = f32::NAN,
            MaterialFactorField::SheenColor => self.sheen.as_mut().unwrap().color.y = f32::NAN,
            MaterialFactorField::SheenRoughness => {
                self.sheen.as_mut().unwrap().roughness = f32::NAN
            }
            _ => panic!("expected an optional material factor field"),
        }
    }
}

impl Default for MaterialFactors {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
#[path = "factors/tests.rs"]
mod factor_tests;

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

pub(super) fn validate_unit_factor(
    value: f32,
    field: MaterialFactorField,
) -> Result<(), MaterialFactorError> {
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
