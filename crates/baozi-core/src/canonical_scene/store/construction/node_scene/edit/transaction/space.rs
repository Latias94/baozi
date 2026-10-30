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
    #[error("camera {camera} requires local-frame conversion")]
    Camera { camera: u32 },
    #[error("light {light} requires local-frame conversion")]
    Light { light: u32 },
    #[error("material {material} has a nonzero scene-unit thickness")]
    MaterialThickness { material: u32 },
    #[error("material {material} binding {binding} has a nonzero scene-unit displacement strength")]
    DisplacementStrength { material: u32, binding: u32 },
    #[error("material {material} binding {binding} uses spatial texture coordinates")]
    TextureCoordinates { material: u32, binding: u32 },
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
    use super::{SceneEditCompileError, SceneEditRejection};
    use crate::appearance::{TextureCoordinateSource, TextureRole};
    let input = resolved.input;
    let conversion = resolved.conversion;
    if conversion.changes_coordinates() {
        let counts = control.run_unit(|| input.as_node_scene().proof.counts())?;
        let unsupported = if counts.camera_count != 0 {
            Some(SceneSpaceConversionIssue::Camera { camera: 0 })
        } else if counts.light_count != 0 {
            Some(SceneSpaceConversionIssue::Light { light: 0 })
        } else {
            None
        };
        if let Some(issue) = unsupported {
            return Err(issue.into());
        }
        for material_index in 0..counts.material_count {
            let material = control
                .run_unit(|| {
                    input.material(crate::identity::MaterialId::new(
                        input.generation(),
                        material_index,
                    ))
                })?
                .map_err(|_| SceneEditCompileError::SemanticRead)?;
            if conversion.unit_ratio() != 1.0
                && control.run_unit(|| material.factors().supplemental_pbr().thickness() != 0.0)?
            {
                return Err(SceneSpaceConversionIssue::MaterialThickness {
                    material: material_index,
                }
                .into());
            }
            let bindings = control.run_unit(|| material.bindings())?;
            for (binding_index, binding) in bindings.enumerate() {
                let issue = control.run_unit(|| {
                    if !matches!(
                        binding.coordinate_source(),
                        TextureCoordinateSource::UvSet(_)
                    ) {
                        Some(SceneSpaceConversionIssue::TextureCoordinates {
                            material: material_index,
                            binding: binding_index as u32,
                        })
                    } else if conversion.unit_ratio() != 1.0
                        && matches!(binding.role(), TextureRole::Displacement)
                        && binding.strength().get() != 0.0
                    {
                        Some(SceneSpaceConversionIssue::DisplacementStrength {
                            material: material_index,
                            binding: binding_index as u32,
                        })
                    } else {
                        None
                    }
                })?;
                if let Some(issue) = issue {
                    return Err(SceneEditRejection::Space(issue));
                }
            }
        }
    }
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

impl super::SceneEditPostflight<'_> {
    pub(super) fn validate_spatial_correspondence<
        C: crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl,
    >(
        &self,
        actual: &crate::canonical_scene::store::typed_storage::SceneTables,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        let super::EditRequest::ConvertSpace(resolved) = self.replacement else {
            return Ok(true);
        };
        let source = self.input.as_node_scene().tables.spatial.as_ref();
        let actual = actual.spatial.as_ref();
        if source.cameras.len() != actual.cameras.len()
            || source.lights.len() != actual.lights.len()
        {
            return Ok(false);
        }
        if control.run_unit(|| {
            let target = resolved.target();
            let mut expected = source.space.clone();
            expected.handedness = target.handedness();
            expected.up_axis = target.up_axis();
            expected.front_axis = target.front_axis();
            expected.unit_scale_to_meters = Some(target.unit_scale_to_meters());
            expected.front_face_winding = target.front_face_winding();
            actual.space == expected
                && actual.space.unit_scale_to_meters.map(f64::to_bits)
                    == expected.unit_scale_to_meters.map(f64::to_bits)
        })? != Some(true)
        {
            return Ok(false);
        }
        for (source, actual) in source.cameras.iter().zip(&actual.cameras) {
            // Canonical projections contain finite parameters and canonical positive zero.
            if control.run_unit(|| source == actual)? != Some(true) {
                return Ok(false);
            }
        }
        for (source, actual) in source.lights.iter().zip(&actual.lights) {
            if control.run_unit(|| {
                source.kind == actual.kind
                    && source.tint.x.to_bits() == actual.tint.x.to_bits()
                    && source.tint.y.to_bits() == actual.tint.y.to_bits()
                    && source.tint.z.to_bits() == actual.tint.z.to_bits()
                    && source.intensity.to_bits() == actual.intensity.to_bits()
            })? != Some(true)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
