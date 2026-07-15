//! Scene-owned hierarchy, camera, and light values.

use crate::extension::{
    ControlledRetainedFootprintError, ExtensionEntry, ExtensionEntryValidationError,
    extension_entries_retained_allocation_bytes_controlled, validate_unique_extension_entry_keys,
};
use crate::math::{Color, Mat4, Quat, Vec3};
use crate::provenance::Provenance;
use std::convert::Infallible;
use std::fmt;
use thiserror::Error;

/// Maximum UTF-8 bytes in one entity display name.
pub const MAX_ENTITY_NAME_BYTES: usize = 1_024;

/// Maximum direct children retained by one node.
pub const MAX_NODE_CHILDREN: usize = 1_048_576;

/// Maximum descriptive extension entries retained by one entity.
pub const MAX_ENTITY_EXTENSION_ENTRIES: usize = 4_096;

/// A bounded entity display name.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityName(String);

impl EntityName {
    /// Fallibly copies one bounded UTF-8 display name.
    pub fn try_new(value: impl AsRef<str>) -> Result<Self, EntityNameError> {
        let value = value.as_ref();
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

    pub(crate) fn capacity(&self) -> usize {
        self.0.capacity()
    }
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

mod node;

pub(crate) use node::NodePreparedParts;
pub use node::{Node, NodeError};

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
        validate_depth_range(near, far)?;
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
        validate_depth_range(near, Some(far))?;
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

fn validate_depth_range(near: f32, far: Option<f32>) -> Result<(), CameraProjectionError> {
    if !near.is_finite() || near <= 0.0 {
        return Err(CameraProjectionError::NearPlane);
    }
    if far.is_some_and(|value| !value.is_finite() || value <= near) {
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
    #[error("camera near plane must be finite and positive")]
    NearPlane,
    #[error("camera far plane must be finite and greater than the near plane")]
    FarPlane,
}

/// One scene-owned camera.
#[derive(Debug, PartialEq)]
pub struct Camera {
    name: Option<EntityName>,
    projection: CameraProjection,
    provenance: Provenance,
    extensions: Vec<ExtensionEntry>,
}

impl Camera {
    pub fn try_new(
        name: Option<EntityName>,
        projection: CameraProjection,
        provenance: Provenance,
        extensions: impl IntoIterator<Item = ExtensionEntry>,
    ) -> Result<Self, EntityCollectionError> {
        Ok(Self {
            name,
            projection,
            provenance,
            extensions: try_collect_extensions(extensions)?,
        })
    }

    pub const fn name(&self) -> Option<&EntityName> {
        self.name.as_ref()
    }

    pub const fn projection(&self) -> CameraProjection {
        self.projection
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub fn extensions(&self) -> impl ExactSizeIterator<Item = &ExtensionEntry> {
        self.extensions.iter()
    }

    pub(crate) fn retained_allocation_bytes(&self) -> Result<usize, EntityRetainedFootprintError> {
        unwrap_uncontrolled_retained_footprint(
            self.retained_allocation_bytes_controlled(&mut || Ok::<(), Infallible>(())),
        )
    }

    pub(crate) fn retained_allocation_bytes_controlled<E>(
        &self,
        charge_work: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<usize, ControlledRetainedFootprintError<E>> {
        owner_retained_bytes_controlled(self.name.as_ref(), &self.extensions, charge_work)
    }
}

/// A source-preserving canonical light category.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightKind {
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
    pub fn point(range: Option<f32>) -> Result<Self, LightError> {
        validate_light_range(range)?;
        Ok(Self::Point { range })
    }

    pub fn spot(
        range: Option<f32>,
        inner_cone_radians: f32,
        outer_cone_radians: f32,
    ) -> Result<Self, LightError> {
        validate_light_range(range)?;
        if !inner_cone_radians.is_finite()
            || !outer_cone_radians.is_finite()
            || inner_cone_radians < 0.0
            || outer_cone_radians <= 0.0
            || inner_cone_radians > outer_cone_radians
            || outer_cone_radians > std::f32::consts::FRAC_PI_2
        {
            return Err(LightError::InvalidSpotCone);
        }
        Ok(Self::Spot {
            range,
            inner_cone_radians,
            outer_cone_radians,
        })
    }
}

fn validate_light_range(range: Option<f32>) -> Result<(), LightError> {
    if range.is_some_and(|value| !value.is_finite() || value <= 0.0) {
        return Err(LightError::InvalidRange);
    }
    Ok(())
}

/// One scene-owned light.
#[derive(Debug, PartialEq)]
pub struct Light {
    name: Option<EntityName>,
    kind: LightKind,
    color: Color,
    intensity: f32,
    provenance: Provenance,
    extensions: Vec<ExtensionEntry>,
}

impl Light {
    pub fn try_new(
        name: Option<EntityName>,
        kind: LightKind,
        color: Color,
        intensity: f32,
        provenance: Provenance,
        extensions: impl IntoIterator<Item = ExtensionEntry>,
    ) -> Result<Self, LightError> {
        if ![color.r, color.g, color.b, color.a]
            .into_iter()
            .all(|component| component.is_finite() && component >= 0.0)
        {
            return Err(LightError::InvalidColor);
        }
        if !intensity.is_finite() || intensity < 0.0 {
            return Err(LightError::InvalidIntensity);
        }
        let extensions = try_collect_extensions(extensions).map_err(LightError::Collection)?;
        Ok(Self {
            name,
            kind,
            color,
            intensity,
            provenance,
            extensions,
        })
    }

    pub const fn name(&self) -> Option<&EntityName> {
        self.name.as_ref()
    }

    pub const fn kind(&self) -> LightKind {
        self.kind
    }

    pub const fn color(&self) -> Color {
        self.color
    }

    pub const fn intensity(&self) -> f32 {
        self.intensity
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub fn extensions(&self) -> impl ExactSizeIterator<Item = &ExtensionEntry> {
        self.extensions.iter()
    }

    pub(crate) fn retained_allocation_bytes(&self) -> Result<usize, EntityRetainedFootprintError> {
        unwrap_uncontrolled_retained_footprint(
            self.retained_allocation_bytes_controlled(&mut || Ok::<(), Infallible>(())),
        )
    }

    pub(crate) fn retained_allocation_bytes_controlled<E>(
        &self,
        charge_work: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<usize, ControlledRetainedFootprintError<E>> {
        owner_retained_bytes_controlled(self.name.as_ref(), &self.extensions, charge_work)
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntityRetainedFootprintError {
    #[error("entity retained-footprint accounting overflowed")]
    Overflow,
}

fn owner_retained_bytes_controlled<E>(
    name: Option<&EntityName>,
    extensions: &Vec<ExtensionEntry>,
    charge_work: &mut impl FnMut() -> Result<(), E>,
) -> Result<usize, ControlledRetainedFootprintError<E>> {
    name.map_or(0, EntityName::capacity)
        .checked_add(extension_entries_retained_allocation_bytes_controlled(
            extensions,
            charge_work,
        )?)
        .ok_or(ControlledRetainedFootprintError::AccountingOverflow)
}

fn unwrap_uncontrolled_retained_footprint(
    result: Result<usize, ControlledRetainedFootprintError<Infallible>>,
) -> Result<usize, EntityRetainedFootprintError> {
    match result {
        Ok(bytes) => Ok(bytes),
        Err(ControlledRetainedFootprintError::AccountingOverflow) => {
            Err(EntityRetainedFootprintError::Overflow)
        }
        Err(ControlledRetainedFootprintError::Control(error)) => match error {},
    }
}

/// Why a light was rejected.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LightError {
    #[error("light range must be finite and positive when present")]
    InvalidRange,
    #[error("spot cone angles must be finite and satisfy 0 <= inner <= outer <= pi/2")]
    InvalidSpotCone,
    #[error("light color components must be finite and non-negative")]
    InvalidColor,
    #[error("light intensity must be finite and non-negative")]
    InvalidIntensity,
    #[error(transparent)]
    Collection(#[from] EntityCollectionError),
}

/// A bounded entity collection could not be built.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EntityCollectionError {
    #[error(transparent)]
    Extension(#[from] ExtensionEntryValidationError),
    #[error("entity storage limit exceeded for {resource:?}; maximum {maximum}")]
    LimitExceeded {
        resource: EntityAllocation,
        maximum: usize,
    },
    #[error("entity allocation failed for {resource:?}")]
    AllocationFailed { resource: EntityAllocation },
}

/// The allocation category attached to a bounded entity error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityAllocation {
    NodeChildren,
    Extensions,
}

fn try_collect_extensions(
    extensions: impl IntoIterator<Item = ExtensionEntry>,
) -> Result<Vec<ExtensionEntry>, EntityCollectionError> {
    let extensions = try_collect_bounded(
        extensions,
        MAX_ENTITY_EXTENSION_ENTRIES,
        EntityAllocation::Extensions,
    )?;
    validate_unique_extension_entry_keys(&extensions)?;
    Ok(extensions)
}

fn try_collect_bounded<T>(
    values: impl IntoIterator<Item = T>,
    maximum: usize,
    resource: EntityAllocation,
) -> Result<Vec<T>, EntityCollectionError> {
    let mut values = values.into_iter();
    let (lower, upper) = values.size_hint();
    if lower > maximum {
        return Err(EntityCollectionError::LimitExceeded { resource, maximum });
    }

    let preflight = upper.filter(|count| *count <= maximum).unwrap_or(lower);
    let mut collected = Vec::new();
    if preflight > 0 {
        collected
            .try_reserve(preflight)
            .map_err(|_| EntityCollectionError::AllocationFailed { resource })?;
    }

    for value in values.by_ref() {
        if collected.len() == maximum {
            return Err(EntityCollectionError::LimitExceeded { resource, maximum });
        }
        if collected.len() == collected.capacity() {
            collected
                .try_reserve(1)
                .map_err(|_| EntityCollectionError::AllocationFailed { resource })?;
        }
        collected.push(value);
    }
    Ok(collected)
}

#[cfg(test)]
mod tests;
