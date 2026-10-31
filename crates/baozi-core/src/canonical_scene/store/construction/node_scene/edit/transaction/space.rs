//! Exact-source resolution and the Scene conversion capability boundary.

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedScene;
use crate::canonical_scene::store::typed_storage::StoredAttributeSemantic;
use crate::geometry::TopologyKind;
use crate::provenance::OptionsDigest;
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpaceField, TangentHandednessEncoding,
};
use crate::space_conversion::{
    NumericConversionError, SourceSpaceError, SpaceConversion, TargetSpace,
};

/// A resolved conversion borrowing the exact input publication used to resolve source facts.
/// Resolution does not allocate or traverse scene collections.
#[derive(Clone, Copy)]
pub struct ResolvedSceneSpaceConversion<'a> {
    pub(super) input: &'a ValidatedScene,
    pub(super) conversion: SpaceConversion,
}

impl ResolvedSceneSpaceConversion<'_> {
    pub fn is_unchanged(self) -> bool {
        self.conversion.is_identity() && self.conversion.assumed_fields().is_empty()
    }

    pub const fn source(self) -> TargetSpace {
        self.conversion.source()
    }
    pub const fn target(self) -> TargetSpace {
        self.conversion.target()
    }
    pub fn changes_coordinates(self) -> bool {
        self.conversion.changes_coordinates()
    }
    pub fn reverses_winding(self) -> bool {
        self.conversion.reverse_winding()
    }
    pub const fn assumed_source_field(self, field: SceneSpaceField) -> bool {
        self.conversion.assumed_fields().contains(field)
    }
    pub fn assumed_fields_mask(self) -> u8 {
        assumed_mask(self.conversion)
    }

    /// SHA-256 over a fixed domain/version, resolved source/target facts, and assumption mask.
    /// Integer fields and unit bits use little-endian encoding in declaration order.
    pub fn options_digest(self) -> OptionsDigest {
        let mut hash = Sha256::new();
        hash.update(b"baozi.scene-space-conversion.options\0");
        hash.update(1_u32.to_le_bytes());
        for space in [self.source(), self.target()] {
            for word in space_words(space) {
                hash.update(word.to_le_bytes());
            }
            hash.update(space.unit_scale_to_meters().to_bits().to_le_bytes());
        }
        hash.update([self.assumed_fields_mask()]);
        OptionsDigest::sha256(hash.finalize().into())
    }
}

/// Call inside the already-started operation's bounded semantic action.
pub fn resolve_scene_space_conversion(
    input: &ValidatedScene,
    target: TargetSpace,
    source_assumptions: Option<TargetSpace>,
) -> Result<ResolvedSceneSpaceConversion<'_>, SourceSpaceError> {
    let source = &input.as_node_scene().tables.spatial.as_ref().space;
    Ok(ResolvedSceneSpaceConversion {
        input,
        conversion: SpaceConversion::try_new(source, target, source_assumptions)?,
    })
}

/// A numeric failure or present spatial semantic that this conversion cannot represent.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SceneSpaceConversionIssue {
    #[error("node {node} cannot be converted: {source}")]
    NumericNode {
        node: usize,
        source: NumericConversionError,
    },
    #[error("attribute {attribute} cannot be converted: {source}")]
    NumericGeometry {
        attribute: usize,
        source: NumericConversionError,
    },
    #[error("primitive {primitive} topology {topology:?} cannot reverse winding")]
    UnsupportedTopology {
        primitive: usize,
        topology: TopologyKind,
    },
    #[error("custom attribute {attribute} has no spatial conversion contract")]
    CustomAttribute { attribute: usize },
    #[error("skin {skin} correction cannot be converted: {source}")]
    NumericSkinCorrection {
        skin: usize,
        source: NumericConversionError,
    },
    #[error("inverse-bind matrix {matrix} cannot be converted: {source}")]
    NumericInverseBind {
        matrix: usize,
        source: NumericConversionError,
    },
    #[error("morph delta {delta} tuple {tuple} cannot be converted: {source}")]
    NumericMorphDelta {
        delta: usize,
        tuple: usize,
        source: NumericConversionError,
    },
    #[error("animation channel {channel} {part:?} key {key} cannot be converted: {source}")]
    NumericAnimation {
        channel: usize,
        part: crate::SceneAnimationSamplePart,
        key: usize,
        source: NumericConversionError,
    },
    #[error("camera {camera} cannot be converted: {source}")]
    NumericCamera {
        camera: usize,
        source: NumericConversionError,
    },
    #[error("camera {camera} projection cannot retain its constraints after conversion: {source}")]
    CameraProjection {
        camera: usize,
        source: crate::CameraProjectionError,
    },
    #[error("light {light} cannot be converted: {source}")]
    NumericLight {
        light: usize,
        source: NumericConversionError,
    },
    #[error("material {material} thickness cannot be converted: {source}")]
    NumericMaterial {
        material: usize,
        source: NumericConversionError,
    },
    #[error("displacement binding {binding} cannot be converted: {source}")]
    NumericDisplacement {
        binding: usize,
        source: NumericConversionError,
    },
    #[error("tangent attribute {attribute} has no supported reflection encoding")]
    TangentEncoding { attribute: usize },
}

pub(super) fn space_words(space: TargetSpace) -> [u32; 4] {
    [
        match space.handedness() {
            Handedness::Right => 1,
            Handedness::Left => 2,
            Handedness::Unknown => 0,
        },
        axis_word(space.up_axis()),
        axis_word(space.front_axis()),
        match space.front_face_winding() {
            FrontFaceWinding::Clockwise => 1,
            FrontFaceWinding::CounterClockwise => 2,
            FrontFaceWinding::Unknown => 0,
        },
    ]
}

fn axis_word(axis: Axis) -> u32 {
    match axis {
        Axis::Unknown => 0,
        Axis::PositiveX => 1,
        Axis::NegativeX => 2,
        Axis::PositiveY => 3,
        Axis::NegativeY => 4,
        Axis::PositiveZ => 5,
        Axis::NegativeZ => 6,
    }
}

pub(super) fn check_capabilities<
    C: crate::canonical_scene::store::typed_storage::build::TypedStorageBuildControl<
            Error = baozi_operation::StoreRuntimeError,
        >,
>(
    resolved: ResolvedSceneSpaceConversion<'_>,
    control: &mut C,
) -> Result<(), super::SceneEditRejection> {
    let input = resolved.input;
    let conversion = resolved.conversion;
    if conversion.determinant_sign() < 0 {
        let count = control.run_unit(|| input.as_node_scene().read_geometry().attributes.len())?;
        for attribute in 0..count {
            if control.run_unit(|| tangent_requires_encoding(input, attribute))? {
                return Err(SceneSpaceConversionIssue::TangentEncoding { attribute }.into());
            }
        }
    }
    Ok(())
}

pub(super) fn assumed_mask(conversion: SpaceConversion) -> u8 {
    [
        SceneSpaceField::Handedness,
        SceneSpaceField::UpAxis,
        SceneSpaceField::FrontAxis,
        SceneSpaceField::UnitScaleToMeters,
        SceneSpaceField::FrontFaceWinding,
    ]
    .into_iter()
    .enumerate()
    .fold(0, |mask, (bit, field)| {
        mask | (u8::from(conversion.assumed_fields().contains(field)) << bit)
    })
}

pub(super) fn tangent_requires_encoding(input: &ValidatedScene, attribute: usize) -> bool {
    matches!(
        input.as_node_scene().read_geometry().attributes[attribute].semantic,
        StoredAttributeSemantic::Tangent
    ) && !matches!(
        input
            .as_node_scene()
            .tables
            .spatial
            .as_ref()
            .space
            .tangent_handedness_encoding,
        TangentHandednessEncoding::SignInW
    )
}
