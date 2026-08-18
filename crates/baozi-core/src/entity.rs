//! Canonical entity, camera, and light values.

use crate::math::{Color, Mat4, Quat, Vec3};
use std::fmt;
use thiserror::Error;

/// Maximum UTF-8 bytes in one entity display name.
pub const MAX_ENTITY_NAME_BYTES: usize = 1_024;

/// A bounded entity display name.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityName(String);

impl EntityName {
    /// Fallibly copies one bounded UTF-8 display name.
    pub fn try_new(value: impl AsRef<str>) -> Result<Self, EntityNameError> {
        let value = value.as_ref();
        validate_entity_name_str(value)?;

        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| EntityNameError::AllocationFailed)?;
        owned.push_str(value);
        Ok(Self(owned))
    }

    /// Returns the retained display text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Fallibly copies this name without hiding an allocation failure.
    pub fn try_clone(&self) -> Result<Self, EntityNameError> {
        Self::try_new(self.as_str())
    }
}

pub(crate) fn validate_entity_name_str(value: &str) -> Result<(), EntityNameError> {
    if value.len() > MAX_ENTITY_NAME_BYTES {
        return Err(EntityNameError::TooLong {
            actual: value.len(),
            maximum: MAX_ENTITY_NAME_BYTES,
        });
    }
    if let Some(offset) = value
        .char_indices()
        .find_map(|(offset, character)| character.is_control().then_some(offset))
    {
        return Err(EntityNameError::ControlCharacter { offset });
    }
    Ok(())
}

impl AsRef<str> for EntityName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for EntityName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("EntityName").field(&self.0).finish()
    }
}

impl fmt::Display for EntityName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why an entity display name was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum EntityNameError {
    #[error("entity name is {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },

    #[error("entity name contains a control character at byte offset {offset}")]
    ControlCharacter { offset: usize },

    #[error("entity name allocation failed")]
    AllocationFailed,
}

/// The source-authored representation of one node base pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthoredTransform {
    representation: AuthoredTransformRepresentation,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AuthoredTransformRepresentation {
    Matrix(Mat4),
    Trs(TrsTransform),
}

impl AuthoredTransform {
    /// Preserves one finite affine matrix without decomposition or rewriting.
    pub fn try_matrix(matrix: Mat4) -> Result<Self, AuthoredTransformError> {
        if !matrix.is_finite() {
            return Err(AuthoredTransformError::MatrixNotFinite);
        }
        if !matrix.is_affine() {
            return Err(AuthoredTransformError::MatrixNotAffine);
        }
        Ok(Self {
            representation: AuthoredTransformRepresentation::Matrix(matrix),
        })
    }

    /// Preserves materialized source TRS components without normalization.
    pub fn try_trs(
        translation: Vec3,
        rotation: Quat,
        scale: Vec3,
    ) -> Result<Self, AuthoredTransformError> {
        Ok(Self {
            representation: AuthoredTransformRepresentation::Trs(TrsTransform::try_new(
                translation,
                rotation,
                scale,
            )?),
        })
    }

    pub const fn identity_trs() -> Self {
        Self {
            representation: AuthoredTransformRepresentation::Trs(TrsTransform::IDENTITY),
        }
    }

    pub const fn kind(self) -> AuthoredTransformKind {
        match self.representation {
            AuthoredTransformRepresentation::Matrix(_) => AuthoredTransformKind::Matrix,
            AuthoredTransformRepresentation::Trs(_) => AuthoredTransformKind::Trs,
        }
    }

    pub const fn authored_matrix(self) -> Option<Mat4> {
        match self.representation {
            AuthoredTransformRepresentation::Matrix(matrix) => Some(matrix),
            AuthoredTransformRepresentation::Trs(_) => None,
        }
    }

    pub const fn authored_trs(self) -> Option<TrsTransform> {
        match self.representation {
            AuthoredTransformRepresentation::Matrix(_) => None,
            AuthoredTransformRepresentation::Trs(trs) => Some(trs),
        }
    }

    /// Returns the derived local matrix while preserving the authored form.
    pub fn local_matrix(self) -> Mat4 {
        match self.representation {
            AuthoredTransformRepresentation::Matrix(matrix) => matrix,
            AuthoredTransformRepresentation::Trs(trs) => {
                Mat4::from_trs(trs.translation, trs.rotation, trs.scale)
            }
        }
    }

    pub(crate) fn has_same_authored_bits(self, other: Self) -> bool {
        match (self.representation, other.representation) {
            (
                AuthoredTransformRepresentation::Matrix(left),
                AuthoredTransformRepresentation::Matrix(right),
            ) => left
                .cols
                .iter()
                .flatten()
                .zip(right.cols.iter().flatten())
                .all(|(left, right)| left.to_bits() == right.to_bits()),
            (
                AuthoredTransformRepresentation::Trs(left),
                AuthoredTransformRepresentation::Trs(right),
            ) => {
                vec3_has_same_bits(left.translation, right.translation)
                    && quat_has_same_bits(left.rotation, right.rotation)
                    && vec3_has_same_bits(left.scale, right.scale)
            }
            _ => false,
        }
    }
}

fn vec3_has_same_bits(left: Vec3, right: Vec3) -> bool {
    left.x.to_bits() == right.x.to_bits()
        && left.y.to_bits() == right.y.to_bits()
        && left.z.to_bits() == right.z.to_bits()
}

fn quat_has_same_bits(left: Quat, right: Quat) -> bool {
    left.x.to_bits() == right.x.to_bits()
        && left.y.to_bits() == right.y.to_bits()
        && left.z.to_bits() == right.z.to_bits()
        && left.w.to_bits() == right.w.to_bits()
}

/// The authored base-transform representation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthoredTransformKind {
    Matrix,
    Trs,
}

/// Materialized authored translation, rotation, and scale components.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrsTransform {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

impl TrsTransform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::new(1.0, 1.0, 1.0),
    };

    fn try_new(
        translation: Vec3,
        rotation: Quat,
        scale: Vec3,
    ) -> Result<Self, AuthoredTransformError> {
        if !translation.is_finite() {
            return Err(AuthoredTransformError::TranslationNotFinite);
        }
        if !rotation.is_finite() {
            return Err(AuthoredTransformError::RotationNotFinite);
        }
        if !rotation.is_unit() {
            return Err(AuthoredTransformError::RotationNotUnit);
        }
        if !scale.is_finite() {
            return Err(AuthoredTransformError::ScaleNotFinite);
        }
        Ok(Self {
            translation,
            rotation,
            scale,
        })
    }

    pub const fn translation(self) -> Vec3 {
        self.translation
    }

    pub const fn rotation(self) -> Quat {
        self.rotation
    }

    pub const fn scale(self) -> Vec3 {
        self.scale
    }
}

/// Why an authored node transform was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AuthoredTransformError {
    #[error("authored matrix contains a non-finite component")]
    MatrixNotFinite,
    #[error("authored matrix is not affine")]
    MatrixNotAffine,
    #[error("authored translation contains a non-finite component")]
    TranslationNotFinite,
    #[error("authored rotation contains a non-finite component")]
    RotationNotFinite,
    #[error("authored rotation is not a unit quaternion")]
    RotationNotUnit,
    #[error("authored scale contains a non-finite component")]
    ScaleNotFinite,
}

/// The supported camera projection category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraProjectionKind {
    Perspective,
    Orthographic,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CameraProjectionState {
    Perspective {
        vertical_fov_radians: f32,
        aspect_ratio: Option<f32>,
        near: f32,
        far: Option<f32>,
    },
    Orthographic {
        horizontal_magnification: f32,
        vertical_magnification: f32,
        near: f32,
        far: f32,
    },
}

/// A validated camera projection with no producerless unknown variant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraProjection(CameraProjectionState);

impl CameraProjection {
    pub fn perspective(
        vertical_fov_radians: f32,
        aspect_ratio: Option<f32>,
        near: f32,
        far: Option<f32>,
    ) -> Result<Self, CameraProjectionError> {
        if !vertical_fov_radians.is_finite()
            || vertical_fov_radians <= 0.0
            || vertical_fov_radians >= std::f32::consts::PI
        {
            return Err(CameraProjectionError::VerticalFieldOfView);
        }
        if aspect_ratio.is_some_and(|value| !value.is_finite() || value <= 0.0) {
            return Err(CameraProjectionError::AspectRatio);
        }
        validate_perspective_depth_range(near, far)?;
        Ok(Self(CameraProjectionState::Perspective {
            vertical_fov_radians,
            aspect_ratio,
            near,
            far,
        }))
    }

    pub fn orthographic(
        horizontal_magnification: f32,
        vertical_magnification: f32,
        near: f32,
        far: f32,
    ) -> Result<Self, CameraProjectionError> {
        if !horizontal_magnification.is_finite() || horizontal_magnification <= 0.0 {
            return Err(CameraProjectionError::HorizontalMagnification);
        }
        if !vertical_magnification.is_finite() || vertical_magnification <= 0.0 {
            return Err(CameraProjectionError::VerticalMagnification);
        }
        validate_orthographic_depth_range(near, far)?;
        let near = canonical_positive_zero(near);
        Ok(Self(CameraProjectionState::Orthographic {
            horizontal_magnification,
            vertical_magnification,
            near,
            far,
        }))
    }

    pub const fn kind(self) -> CameraProjectionKind {
        match self.0 {
            CameraProjectionState::Perspective { .. } => CameraProjectionKind::Perspective,
            CameraProjectionState::Orthographic { .. } => CameraProjectionKind::Orthographic,
        }
    }

    pub const fn perspective_parameters(self) -> Option<(f32, Option<f32>, f32, Option<f32>)> {
        match self.0 {
            CameraProjectionState::Perspective {
                vertical_fov_radians,
                aspect_ratio,
                near,
                far,
            } => Some((vertical_fov_radians, aspect_ratio, near, far)),
            CameraProjectionState::Orthographic { .. } => None,
        }
    }

    pub const fn orthographic_parameters(self) -> Option<(f32, f32, f32, f32)> {
        match self.0 {
            CameraProjectionState::Orthographic {
                horizontal_magnification,
                vertical_magnification,
                near,
                far,
            } => Some((horizontal_magnification, vertical_magnification, near, far)),
            CameraProjectionState::Perspective { .. } => None,
        }
    }
}

fn validate_perspective_depth_range(
    near: f32,
    far: Option<f32>,
) -> Result<(), CameraProjectionError> {
    if !near.is_finite() || near <= 0.0 {
        return Err(CameraProjectionError::NearPlane);
    }
    if far.is_some_and(|value| !value.is_finite() || value <= near) {
        return Err(CameraProjectionError::FarPlane);
    }
    Ok(())
}

fn validate_orthographic_depth_range(near: f32, far: f32) -> Result<(), CameraProjectionError> {
    if !near.is_finite() || near < 0.0 {
        return Err(CameraProjectionError::NearPlane);
    }
    if !far.is_finite() || far <= near {
        return Err(CameraProjectionError::FarPlane);
    }
    Ok(())
}

/// Why camera projection parameters were rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CameraProjectionError {
    #[error("camera vertical field of view must be finite and in (0, pi)")]
    VerticalFieldOfView,
    #[error("camera aspect ratio must be finite and positive when present")]
    AspectRatio,
    #[error("camera horizontal magnification must be finite and positive")]
    HorizontalMagnification,
    #[error("camera vertical magnification must be finite and positive")]
    VerticalMagnification,
    #[error(
        "camera near plane must be finite and positive for perspective or non-negative for orthographic"
    )]
    NearPlane,
    #[error("camera far plane must be finite and greater than the near plane")]
    FarPlane,
}

/// The source-preserving category of a canonical light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightKindCategory {
    /// A light with direction but no position or range.
    Directional,
    /// An omnidirectional light emitted from one position.
    Point,
    /// A position and direction constrained by a cone.
    Spot,
}

/// A source-preserving canonical light category and its validated parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightKind(LightKindState);

#[derive(Debug, Clone, Copy, PartialEq)]
enum LightKindState {
    Directional,
    Point {
        range: Option<f32>,
    },
    Spot {
        range: Option<f32>,
        inner_cone_radians: f32,
        outer_cone_radians: f32,
    },
}

impl LightKind {
    /// Creates a directional light category.
    pub const fn directional() -> Self {
        Self(LightKindState::Directional)
    }

    /// Creates a point light category with an optional positive range.
    pub fn point(range: Option<f32>) -> Result<Self, LightError> {
        validate_light_range(range)?;
        Ok(Self(LightKindState::Point { range }))
    }

    /// Creates a spot light category with a validated cone and optional positive range.
    pub fn spot(
        range: Option<f32>,
        inner_cone_radians: f32,
        outer_cone_radians: f32,
    ) -> Result<Self, LightError> {
        validate_light_range(range)?;
        let inner_cone_radians = canonical_positive_zero(inner_cone_radians);
        if !inner_cone_radians.is_finite()
            || !outer_cone_radians.is_finite()
            || inner_cone_radians < 0.0
            || outer_cone_radians <= 0.0
            || inner_cone_radians > outer_cone_radians
            || outer_cone_radians > std::f32::consts::FRAC_PI_2
        {
            return Err(LightError::InvalidSpotCone);
        }
        Ok(Self(LightKindState::Spot {
            range,
            inner_cone_radians,
            outer_cone_radians,
        }))
    }

    /// Returns the category without its parameters.
    pub const fn category(self) -> LightKindCategory {
        match self.0 {
            LightKindState::Directional => LightKindCategory::Directional,
            LightKindState::Point { .. } => LightKindCategory::Point,
            LightKindState::Spot { .. } => LightKindCategory::Spot,
        }
    }

    /// Returns the optional range of a point or spot light.
    pub const fn range(self) -> Option<f32> {
        match self.0 {
            LightKindState::Directional => None,
            LightKindState::Point { range } | LightKindState::Spot { range, .. } => range,
        }
    }

    /// Returns the inner cone angle for a spot light.
    pub const fn inner_cone_radians(self) -> Option<f32> {
        match self.0 {
            LightKindState::Spot {
                inner_cone_radians, ..
            } => Some(inner_cone_radians),
            LightKindState::Directional | LightKindState::Point { .. } => None,
        }
    }

    /// Returns the outer cone angle for a spot light.
    pub const fn outer_cone_radians(self) -> Option<f32> {
        match self.0 {
            LightKindState::Spot {
                outer_cone_radians, ..
            } => Some(outer_cone_radians),
            LightKindState::Directional | LightKindState::Point { .. } => None,
        }
    }
}

fn validate_light_range(range: Option<f32>) -> Result<(), LightError> {
    if range.is_some_and(|value| !value.is_finite() || value <= 0.0) {
        return Err(LightError::InvalidRange);
    }
    Ok(())
}

const fn canonical_positive_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

const REC709_RED_NUMERATOR: f64 = 2_126.0;
const REC709_GREEN_NUMERATOR: f64 = 7_152.0;
const REC709_BLUE_NUMERATOR: f64 = 722.0;
const REC709_LUMINANCE_DENOMINATOR: f64 = 10_000.0;
const LIGHT_TINT_LUMINANCE_TOLERANCE: f64 = REC709_LUMINANCE_DENOMINATOR / 8_388_608.0;

/// A validated canonical light tint and photometric intensity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LightEmission {
    tint: Vec3,
    intensity: f32,
}

impl LightEmission {
    /// Converts a source RGB multiplier and scalar intensity into canonical emission.
    pub(crate) fn try_from_scaled_linear_srgb(
        color: Color,
        intensity: f32,
    ) -> Result<Self, LightEmissionError> {
        validate_light_color(color)?;
        if !intensity.is_finite() || intensity < 0.0 {
            return Err(LightEmissionError::InvalidIntensity);
        }

        let luminance_numerator = rec709_luminance_numerator(Vec3::new(color.r, color.g, color.b));
        if intensity == 0.0 || luminance_numerator == 0.0 {
            return Ok(Self {
                tint: Vec3::new(1.0, 1.0, 1.0),
                intensity: 0.0,
            });
        }

        let canonical_intensity =
            (f64::from(intensity) * luminance_numerator / REC709_LUMINANCE_DENOMINATOR) as f32;
        if !canonical_intensity.is_finite() || canonical_intensity == 0.0 {
            return Err(LightEmissionError::Unrepresentable);
        }

        let normalize = |component: f32| -> Result<f32, LightEmissionError> {
            if component == 0.0 {
                return Ok(0.0);
            }
            let normalized =
                (f64::from(component) * REC709_LUMINANCE_DENOMINATOR / luminance_numerator) as f32;
            if !normalized.is_finite() || normalized == 0.0 {
                return Err(LightEmissionError::Unrepresentable);
            }
            Ok(normalized)
        };
        let emission = Self {
            tint: Vec3::new(
                normalize(color.r)?,
                normalize(color.g)?,
                normalize(color.b)?,
            ),
            intensity: canonical_intensity,
        };
        validate_canonical_light_emission(emission.tint, emission.intensity)?;
        Ok(emission)
    }

    pub(crate) const fn tint(self) -> Vec3 {
        self.tint
    }

    pub(crate) const fn intensity(self) -> f32 {
        self.intensity
    }
}

pub(crate) fn validate_canonical_light_emission(
    tint: Vec3,
    intensity: f32,
) -> Result<(), LightEmissionError> {
    if ![tint.x, tint.y, tint.z]
        .into_iter()
        .all(|component| component.is_finite() && component >= 0.0)
    {
        return Err(LightEmissionError::InvalidColor);
    }
    if !intensity.is_finite() || intensity < 0.0 {
        return Err(LightEmissionError::InvalidIntensity);
    }

    if intensity == 0.0 {
        if intensity.to_bits() != 0.0_f32.to_bits()
            || !has_exact_vec3_bits(tint, Vec3::new(1.0, 1.0, 1.0))
        {
            return Err(LightEmissionError::NonCanonicalZero);
        }
    } else {
        if [tint.x, tint.y, tint.z]
            .into_iter()
            .any(|component| component == 0.0 && component.to_bits() != 0.0_f32.to_bits())
        {
            return Err(LightEmissionError::NonCanonicalTint);
        }
        let luminance_error =
            (rec709_luminance_numerator(tint) - REC709_LUMINANCE_DENOMINATOR).abs();
        if luminance_error > LIGHT_TINT_LUMINANCE_TOLERANCE {
            return Err(LightEmissionError::NonCanonicalTint);
        }
    }

    Ok(())
}

fn validate_light_color(color: Color) -> Result<(), LightEmissionError> {
    if ![color.r, color.g, color.b]
        .into_iter()
        .all(|component| component.is_finite() && component >= 0.0)
    {
        return Err(LightEmissionError::InvalidColor);
    }
    if color.a != 1.0 {
        return Err(LightEmissionError::InvalidAlpha);
    }
    Ok(())
}

fn rec709_luminance_numerator(tint: Vec3) -> f64 {
    let red = REC709_RED_NUMERATOR * f64::from(tint.x);
    let green = REC709_GREEN_NUMERATOR * f64::from(tint.y);
    let blue = REC709_BLUE_NUMERATOR * f64::from(tint.z);
    (red + green) + blue
}

fn has_exact_vec3_bits(left: Vec3, right: Vec3) -> bool {
    [left.x, left.y, left.z]
        .into_iter()
        .zip([right.x, right.y, right.z])
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

/// Why a light was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LightError {
    #[error("light range must be finite and positive when present")]
    InvalidRange,
    #[error("spot cone angles must be finite and satisfy 0 <= inner <= outer <= pi/2")]
    InvalidSpotCone,
}

/// Why a light emission value was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LightEmissionError {
    #[error("light tint components must be finite and non-negative")]
    InvalidColor,
    #[error("light tint alpha must be exactly one")]
    InvalidAlpha,
    #[error("light intensity must be finite and non-negative")]
    InvalidIntensity,
    #[error("zero light emission must use exact white tint and positive zero intensity")]
    NonCanonicalZero,
    #[error("positive light tint must be unit-luminance and use positive zero channels")]
    NonCanonicalTint,
    #[error("source light emission is not representable in canonical f32 storage")]
    Unrepresentable,
}

#[cfg(test)]
mod tests;
