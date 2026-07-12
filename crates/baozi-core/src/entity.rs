//! Scene-owned hierarchy, camera, and light values.

use crate::extension::{
    ExtensionEntry, ExtensionEntryValidationError, validate_unique_extension_entry_keys,
};
use crate::identity::{CameraIndex, LightIndex, NodeIndex};
use crate::math::{Color, Mat4, Quat, Vec3};
use crate::provenance::{Provenance, ProvenanceAttachments};
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

/// One hierarchy node.
///
/// Parent and mesh-instance lookups are derived scene indexes. The only
/// canonical hierarchy edge stored here is the ordered child list.
#[derive(Debug, PartialEq)]
pub struct Node {
    name: Option<EntityName>,
    authored_transform: AuthoredTransform,
    children: Vec<NodeIndex>,
    camera: Option<CameraIndex>,
    light: Option<LightIndex>,
    provenance: ProvenanceAttachments,
    extensions: Vec<ExtensionEntry>,
}

impl Node {
    /// Creates a node and validates all leaf-level invariants.
    pub fn try_new(
        name: Option<EntityName>,
        authored_transform: AuthoredTransform,
        children: impl IntoIterator<Item = NodeIndex>,
        camera: Option<CameraIndex>,
        light: Option<LightIndex>,
        provenance: impl Into<ProvenanceAttachments>,
        extensions: impl IntoIterator<Item = ExtensionEntry>,
    ) -> Result<Self, NodeError> {
        let children =
            try_collect_bounded(children, MAX_NODE_CHILDREN, EntityAllocation::NodeChildren)?;
        let extensions = try_collect_extensions(extensions)?;
        let provenance = provenance.into();

        Ok(Self {
            name,
            authored_transform,
            children,
            camera,
            light,
            provenance,
            extensions,
        })
    }

    pub const fn name(&self) -> Option<&EntityName> {
        self.name.as_ref()
    }

    pub const fn authored_transform(&self) -> AuthoredTransform {
        self.authored_transform
    }

    pub fn local_matrix(&self) -> Mat4 {
        self.authored_transform.local_matrix()
    }

    pub fn children(&self) -> impl ExactSizeIterator<Item = NodeIndex> + '_ {
        self.children.iter().copied()
    }

    pub const fn camera_index(&self) -> Option<CameraIndex> {
        self.camera
    }

    pub const fn light_index(&self) -> Option<LightIndex> {
        self.light
    }

    pub const fn provenance(&self) -> &ProvenanceAttachments {
        &self.provenance
    }

    pub fn extensions(&self) -> impl ExactSizeIterator<Item = &ExtensionEntry> {
        self.extensions.iter()
    }
}

/// Why a node could not enter canonical storage.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NodeError {
    #[error(transparent)]
    Extension(#[from] ExtensionEntryValidationError),

    #[error("node storage limit exceeded for {resource:?}; maximum {maximum}")]
    LimitExceeded {
        resource: EntityAllocation,
        maximum: usize,
    },

    #[error("node allocation failed for {resource:?}")]
    AllocationFailed { resource: EntityAllocation },
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

impl From<EntityCollectionError> for NodeError {
    fn from(error: EntityCollectionError) -> Self {
        match error {
            EntityCollectionError::Extension(error) => Self::Extension(error),
            EntityCollectionError::LimitExceeded { resource, maximum } => {
                Self::LimitExceeded { resource, maximum }
            }
            EntityCollectionError::AllocationFailed { resource } => {
                Self::AllocationFailed { resource }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{CameraIndex, LightIndex, NodeIndex};
    use crate::provenance::{ProvenanceAttachments, test_provenance_fixture};

    #[test]
    fn node_stores_only_canonical_child_and_attachment_indexes() {
        let (_arena, provenance) = test_provenance_fixture();
        let child = NodeIndex::try_from(3).expect("fixture index should fit");
        let camera = CameraIndex::try_from(1).expect("fixture index should fit");
        let light = LightIndex::try_from(2).expect("fixture index should fit");
        let node = Node::try_new(
            EntityName::try_new("root").ok(),
            AuthoredTransform::identity_trs(),
            [child],
            Some(camera),
            Some(light),
            provenance,
            [],
        )
        .expect("node should be valid");

        assert_eq!(node.children().collect::<Vec<_>>(), vec![child]);
        assert_eq!(node.camera_index(), Some(camera));
        assert_eq!(node.light_index(), Some(light));
        assert_eq!(node.name().map(EntityName::as_str), Some("root"));
        assert_eq!(node.authored_transform().kind(), AuthoredTransformKind::Trs);
        assert_eq!(node.local_matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn node_preserves_every_provenance_attachment() {
        let (_arena, [first, second]) = crate::provenance::test_provenance_pair_fixture();
        let attachments = ProvenanceAttachments::try_from_iter([first, second]).unwrap();
        let node = Node::try_new(
            None,
            AuthoredTransform::identity_trs(),
            [],
            None,
            None,
            attachments,
            [],
        )
        .unwrap();

        assert_eq!(
            node.provenance().iter().collect::<Vec<_>>(),
            [first, second]
        );
    }

    #[test]
    fn authored_transform_preserves_matrix_or_trs_without_conversion() {
        let mut transform = Mat4::IDENTITY;
        transform.cols[3][0] = -0.0;
        let matrix = AuthoredTransform::try_matrix(transform).unwrap();
        assert_eq!(matrix.kind(), AuthoredTransformKind::Matrix);
        assert_eq!(matrix.authored_matrix(), Some(transform));
        assert_eq!(matrix.local_matrix(), transform);
        assert_eq!(matrix.authored_trs(), None);

        let trs = AuthoredTransform::try_trs(
            Vec3::new(4.0, 5.0, 6.0),
            Quat::IDENTITY,
            Vec3::new(-2.0, 0.0, 3.0),
        )
        .unwrap();
        assert_eq!(trs.kind(), AuthoredTransformKind::Trs);
        assert_eq!(trs.authored_matrix(), None);
        assert_eq!(
            trs.authored_trs().unwrap().scale(),
            Vec3::new(-2.0, 0.0, 3.0)
        );
        assert_eq!(trs.local_matrix().cols[3], [4.0, 5.0, 6.0, 1.0]);
    }

    #[test]
    fn authored_transform_rejects_invalid_matrix_and_trs_values() {
        let mut non_finite = Mat4::IDENTITY;
        non_finite.cols[2][1] = f32::NAN;
        assert_eq!(
            AuthoredTransform::try_matrix(non_finite),
            Err(AuthoredTransformError::MatrixNotFinite)
        );

        let mut non_affine = Mat4::IDENTITY;
        non_affine.cols[0][3] = 0.25;
        assert_eq!(
            AuthoredTransform::try_matrix(non_affine),
            Err(AuthoredTransformError::MatrixNotAffine)
        );

        assert_eq!(
            AuthoredTransform::try_trs(
                Vec3::ZERO,
                Quat::from_xyzw(0.0, 0.0, 0.0, 2.0),
                Vec3::new(1.0, 1.0, 1.0),
            ),
            Err(AuthoredTransformError::RotationNotUnit)
        );
    }

    #[test]
    fn camera_projection_rejects_invalid_frusta() {
        assert_eq!(
            CameraProjection::perspective(0.0, None, 0.1, None),
            Err(CameraProjectionError::VerticalFieldOfView)
        );
        assert_eq!(
            CameraProjection::perspective(1.0, Some(0.0), 0.1, None),
            Err(CameraProjectionError::AspectRatio)
        );
        assert_eq!(
            CameraProjection::orthographic(1.0, 1.0, 2.0, 1.0),
            Err(CameraProjectionError::FarPlane)
        );
    }

    #[test]
    fn light_kind_makes_spot_invariants_explicit() {
        assert_eq!(
            LightKind::spot(None, 0.5, 0.25),
            Err(LightError::InvalidSpotCone)
        );
        assert!(LightKind::spot(Some(10.0), 0.25, 0.5).is_ok());
        assert_eq!(LightKind::point(Some(0.0)), Err(LightError::InvalidRange));
    }

    #[test]
    fn entity_name_bounds_and_sanitizes_display_text() {
        assert_eq!(
            EntityName::try_new("bad\nname"),
            Err(EntityNameError::ControlCharacter { offset: 3 })
        );
        let oversized = "x".repeat(MAX_ENTITY_NAME_BYTES + 1);
        assert_eq!(
            EntityName::try_new(oversized),
            Err(EntityNameError::TooLong {
                actual: MAX_ENTITY_NAME_BYTES + 1,
                maximum: MAX_ENTITY_NAME_BYTES,
            })
        );
    }
}
