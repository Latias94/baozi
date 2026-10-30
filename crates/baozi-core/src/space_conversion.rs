//! Validated spatial conventions and the sealed arithmetic for an explicit conversion.
//!
//! This foundation changes coordinates, units, and winding only. UV origin and tangent-generation
//! conventions remain owned by the source descriptor. Scene transactions must apply these helpers
//! to their complete domain inventory before publishing a converted scene.

use thiserror::Error;

use crate::entity::{AuthoredTransform, AuthoredTransformError};
use crate::math::{Mat4, Quat, Vec3};
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpace, SceneSpaceField, SceneSpaceValidationError,
};

/// The five known spatial conventions required by coordinate conversion.
///
/// All fields are validated together. This value does not select a UV origin or a tangent algorithm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetSpace {
    handedness: Handedness,
    up_axis: Axis,
    front_axis: Axis,
    unit_scale_to_meters: f64,
    front_face_winding: FrontFaceWinding,
}

impl TargetSpace {
    /// Right-handed, positive Y up, negative Z front, meters, and counterclockwise front faces.
    pub const STANDARD: Self = Self {
        handedness: Handedness::Right,
        up_axis: Axis::PositiveY,
        front_axis: Axis::NegativeZ,
        unit_scale_to_meters: 1.0,
        front_face_winding: FrontFaceWinding::CounterClockwise,
    };

    /// Requires known, distinct axes and a positive finite unit scale.
    pub fn try_new(
        handedness: Handedness,
        up_axis: Axis,
        front_axis: Axis,
        unit_scale_to_meters: f64,
        front_face_winding: FrontFaceWinding,
    ) -> Result<Self, SceneSpaceValidationError> {
        SceneSpace {
            handedness,
            up_axis,
            front_axis,
            unit_scale_to_meters: Some(unit_scale_to_meters),
            front_face_winding,
            ..SceneSpace::UNKNOWN
        }
        .validate()?;
        for (field, unknown) in [
            (SceneSpaceField::Handedness, handedness.is_unknown()),
            (SceneSpaceField::UpAxis, up_axis.is_unknown()),
            (SceneSpaceField::FrontAxis, front_axis.is_unknown()),
            (
                SceneSpaceField::FrontFaceWinding,
                front_face_winding.is_unknown(),
            ),
        ] {
            if unknown {
                return Err(SceneSpaceValidationError::UnknownField { field });
            }
        }
        Ok(Self {
            handedness,
            up_axis,
            front_axis,
            unit_scale_to_meters,
            front_face_winding,
        })
    }

    pub const fn handedness(self) -> Handedness {
        self.handedness
    }

    pub const fn up_axis(self) -> Axis {
        self.up_axis
    }

    pub const fn front_axis(self) -> Axis {
        self.front_axis
    }

    pub const fn unit_scale_to_meters(self) -> f64 {
        self.unit_scale_to_meters
    }

    pub const fn front_face_winding(self) -> FrontFaceWinding {
        self.front_face_winding
    }
}

/// A typed source fact retained when an explicit assumption contradicts a declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConventionValue {
    Handedness(Handedness),
    Axis(Axis),
    UnitScaleToMeters(f64),
    FrontFaceWinding(FrontFaceWinding),
}

/// Why the source facts could not define a conversion.
#[derive(Debug, Error, Clone, Copy, PartialEq)]
pub enum SourceSpaceError {
    #[error("source space has no declared or explicitly assumed {field}")]
    MissingSourceConvention { field: SceneSpaceField },

    #[error("source {field} declares {declared:?}, conflicting with the assumption {assumed:?}")]
    SourceAssumptionConflict {
        field: SceneSpaceField,
        declared: ConventionValue,
        assumed: ConventionValue,
    },

    #[error("resolved source space is invalid: {source}")]
    InvalidResolvedSource {
        #[source]
        source: SceneSpaceValidationError,
    },

    #[error("the source-to-target unit ratio is not a positive finite f64")]
    UnrepresentableUnitScale,
}

/// Why conversion could not retain a finite, representable authored value.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NumericConversionError {
    #[error("spatial conversion produced a non-finite component")]
    NonFiniteOutput,

    #[error("spatial conversion rounded a nonzero component to zero")]
    UnderflowToZero,

    #[error("converted authored transform is invalid: {source}")]
    InvalidAuthoredTransform {
        #[source]
        source: AuthoredTransformError,
    },
}

/// Only fields actually filled from the explicit source assumption are present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct AssumedSourceFields(u8);

impl AssumedSourceFields {
    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) const fn contains(self, field: SceneSpaceField) -> bool {
        self.0 & Self::bit(field) != 0
    }

    fn insert(&mut self, field: SceneSpaceField) {
        self.0 |= Self::bit(field);
    }

    const fn bit(field: SceneSpaceField) -> u8 {
        match field {
            SceneSpaceField::Handedness => 1,
            SceneSpaceField::UpAxis => 2,
            SceneSpaceField::FrontAxis => 4,
            SceneSpaceField::UnitScaleToMeters => 8,
            SceneSpaceField::FrontFaceWinding => 16,
            _ => 0,
        }
    }
}

/// Resolved source facts and a fixed signed permutation, sealed before scene traversal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SpaceConversion {
    source: TargetSpace,
    target: TargetSpace,
    assumed_fields: AssumedSourceFields,
    source_axes: [usize; 3],
    negative: [bool; 3],
    unit_ratio: f64,
}

impl SpaceConversion {
    pub(crate) fn try_new(
        source: &SceneSpace,
        target: TargetSpace,
        assumptions: Option<TargetSpace>,
    ) -> Result<Self, SourceSpaceError> {
        source
            .validate()
            .map_err(|source| SourceSpaceError::InvalidResolvedSource { source })?;
        if let Some(assumed) = assumptions {
            check_assumptions(source, assumed)?;
        }

        let mut assumed_fields = AssumedSourceFields::default();
        let handedness = if source.handedness.is_unknown() {
            assumed_fields.insert(SceneSpaceField::Handedness);
            assumptions
                .ok_or(SourceSpaceError::MissingSourceConvention {
                    field: SceneSpaceField::Handedness,
                })?
                .handedness
        } else {
            source.handedness
        };
        let up_axis = if source.up_axis.is_unknown() {
            assumed_fields.insert(SceneSpaceField::UpAxis);
            assumptions
                .ok_or(SourceSpaceError::MissingSourceConvention {
                    field: SceneSpaceField::UpAxis,
                })?
                .up_axis
        } else {
            source.up_axis
        };
        let front_axis = if source.front_axis.is_unknown() {
            assumed_fields.insert(SceneSpaceField::FrontAxis);
            assumptions
                .ok_or(SourceSpaceError::MissingSourceConvention {
                    field: SceneSpaceField::FrontAxis,
                })?
                .front_axis
        } else {
            source.front_axis
        };
        let unit_scale_to_meters = match source.unit_scale_to_meters {
            Some(scale) => scale,
            None => {
                assumed_fields.insert(SceneSpaceField::UnitScaleToMeters);
                assumptions
                    .ok_or(SourceSpaceError::MissingSourceConvention {
                        field: SceneSpaceField::UnitScaleToMeters,
                    })?
                    .unit_scale_to_meters
            }
        };
        let front_face_winding = if source.front_face_winding.is_unknown() {
            assumed_fields.insert(SceneSpaceField::FrontFaceWinding);
            assumptions
                .ok_or(SourceSpaceError::MissingSourceConvention {
                    field: SceneSpaceField::FrontFaceWinding,
                })?
                .front_face_winding
        } else {
            source.front_face_winding
        };
        let source = TargetSpace::try_new(
            handedness,
            up_axis,
            front_axis,
            unit_scale_to_meters,
            front_face_winding,
        )
        .map_err(|source| SourceSpaceError::InvalidResolvedSource { source })?;
        let unit_ratio = source.unit_scale_to_meters / target.unit_scale_to_meters;
        if !unit_ratio.is_finite() || unit_ratio <= 0.0 {
            return Err(SourceSpaceError::UnrepresentableUnitScale);
        }

        // Columns are (side, up, front), with side = handedness_sign * cross(front, up).
        // Integer arithmetic forms Q = B_target * transpose(B_source) exactly.
        let source_basis = basis(source);
        let target_basis = basis(target);
        let mut source_axes = [0; 3];
        let mut negative = [false; 3];
        for row in 0..3 {
            for column in [0, 1, 2] {
                let entry: i8 = target_basis
                    .iter()
                    .zip(&source_basis)
                    .map(|(target, source)| target[row] * source[column])
                    .sum();
                if entry != 0 {
                    source_axes[row] = column;
                    negative[row] = entry < 0;
                }
            }
        }
        Ok(Self {
            source,
            target,
            assumed_fields,
            source_axes,
            negative,
            unit_ratio,
        })
    }

    pub(crate) const fn source(&self) -> TargetSpace {
        self.source
    }

    pub(crate) const fn target(&self) -> TargetSpace {
        self.target
    }

    pub(crate) const fn assumed_fields(&self) -> AssumedSourceFields {
        self.assumed_fields
    }

    pub(crate) const fn unit_ratio(&self) -> f64 {
        self.unit_ratio
    }

    pub(crate) fn determinant_sign(&self) -> i8 {
        if self.source.handedness == self.target.handedness {
            1
        } else {
            -1
        }
    }

    /// Whether the numeric point map sQ differs from identity, independently of winding.
    pub(crate) fn changes_coordinates(&self) -> bool {
        self.unit_ratio != 1.0 || self.source_axes != [0, 1, 2] || self.negative != [false; 3]
    }

    /// Whether coordinates and corner order both remain unchanged.
    /// Resolving source assumptions is recorded separately in `assumed_fields`.
    pub(crate) fn is_identity(&self) -> bool {
        !self.changes_coordinates() && !self.reverse_winding()
    }

    pub(crate) fn reverse_winding(&self) -> bool {
        (self.determinant_sign() < 0)
            ^ (self.source.front_face_winding != self.target.front_face_winding)
    }

    /// Applies sQ with f64 intermediates and one final f32 rounding per component.
    pub(crate) fn point(&self, value: [f32; 3]) -> Result<[f32; 3], NumericConversionError> {
        let mut output = self.direction(value)?;
        for component in &mut output {
            *component =
                round_component(f64::from(*component) * self.unit_ratio, *component != 0.0)?;
        }
        Ok(output)
    }

    /// Applies only Q. Finite components are copied or sign-flipped without normalization.
    pub(crate) fn direction(&self, value: [f32; 3]) -> Result<[f32; 3], NumericConversionError> {
        if !value.into_iter().all(f32::is_finite) {
            return Err(NumericConversionError::NonFiniteOutput);
        }
        Ok(std::array::from_fn(|axis| {
            signed(value[self.source_axes[axis]], self.negative[axis])
        }))
    }

    /// Permutes scale components without applying coordinate signs or units.
    pub(crate) fn scale(&self, value: [f32; 3]) -> Result<[f32; 3], NumericConversionError> {
        if !value.into_iter().all(f32::is_finite) {
            return Err(NumericConversionError::NonFiniteOutput);
        }
        Ok(std::array::from_fn(|axis| value[self.source_axes[axis]]))
    }

    /// Maps the quaternion vector part by det(Q)Q and retains w without normalization.
    pub(crate) fn rotation(&self, value: Quat) -> Result<Quat, NumericConversionError> {
        if !value.is_finite() {
            return Err(NumericConversionError::NonFiniteOutput);
        }
        let vector = [value.x, value.y, value.z];
        let reflection = self.determinant_sign() < 0;
        let output: [f32; 3] = std::array::from_fn(|axis| {
            signed(
                vector[self.source_axes[axis]],
                self.negative[axis] ^ reflection,
            )
        });
        Ok(Quat::from_xyzw(output[0], output[1], output[2], value.w))
    }

    /// Applies H M H^-1 directly, where H = diag(sQ, 1).
    ///
    /// The linear block is Q A Q^T, translation is sQt, and the projective row is cQ^T/s.
    /// This preserves shear and singular matrices without decomposing or inverting M.
    pub(crate) fn matrix(&self, value: Mat4) -> Result<Mat4, NumericConversionError> {
        if !value.is_finite() {
            return Err(NumericConversionError::NonFiniteOutput);
        }
        let mut output = value;
        for column in 0..3 {
            let source_column = self.source_axes[column];
            for row in 0..3 {
                output.cols[column][row] = signed(
                    value.cols[source_column][self.source_axes[row]],
                    self.negative[column] ^ self.negative[row],
                );
            }
            let component = signed(value.cols[source_column][3], self.negative[column]);
            output.cols[column][3] =
                round_component(f64::from(component) / self.unit_ratio, component != 0.0)?;
        }
        let translation = self.point([value.cols[3][0], value.cols[3][1], value.cols[3][2]])?;
        output.cols[3][..3].copy_from_slice(&translation);
        Ok(output)
    }

    /// Retains the authored Matrix/TRS variant and never normalizes or decomposes it.
    pub(crate) fn authored_transform(
        &self,
        value: AuthoredTransform,
    ) -> Result<AuthoredTransform, NumericConversionError> {
        if let Some(matrix) = value.authored_matrix() {
            return AuthoredTransform::try_matrix(self.matrix(matrix)?)
                .map_err(|source| NumericConversionError::InvalidAuthoredTransform { source });
        }
        let trs = value
            .authored_trs()
            .expect("an authored transform has exactly one Matrix/TRS representation");
        let translation = trs.translation();
        let scale = trs.scale();
        let translation = self.point([translation.x, translation.y, translation.z])?;
        let scale = self.scale([scale.x, scale.y, scale.z])?;
        AuthoredTransform::try_trs(
            Vec3::new(translation[0], translation[1], translation[2]),
            self.rotation(trs.rotation())?,
            Vec3::new(scale[0], scale[1], scale[2]),
        )
        .map_err(|source| NumericConversionError::InvalidAuthoredTransform { source })
    }
}

fn check_assumptions(source: &SceneSpace, assumed: TargetSpace) -> Result<(), SourceSpaceError> {
    for (field, declared, assumed) in [
        (
            SceneSpaceField::Handedness,
            (!source.handedness.is_unknown())
                .then_some(ConventionValue::Handedness(source.handedness)),
            ConventionValue::Handedness(assumed.handedness),
        ),
        (
            SceneSpaceField::UpAxis,
            (!source.up_axis.is_unknown()).then_some(ConventionValue::Axis(source.up_axis)),
            ConventionValue::Axis(assumed.up_axis),
        ),
        (
            SceneSpaceField::FrontAxis,
            (!source.front_axis.is_unknown()).then_some(ConventionValue::Axis(source.front_axis)),
            ConventionValue::Axis(assumed.front_axis),
        ),
        (
            SceneSpaceField::UnitScaleToMeters,
            source
                .unit_scale_to_meters
                .map(ConventionValue::UnitScaleToMeters),
            ConventionValue::UnitScaleToMeters(assumed.unit_scale_to_meters),
        ),
        (
            SceneSpaceField::FrontFaceWinding,
            (!source.front_face_winding.is_unknown())
                .then_some(ConventionValue::FrontFaceWinding(source.front_face_winding)),
            ConventionValue::FrontFaceWinding(assumed.front_face_winding),
        ),
    ] {
        if let Some(declared) = declared
            && declared != assumed
        {
            return Err(SourceSpaceError::SourceAssumptionConflict {
                field,
                declared,
                assumed,
            });
        }
    }
    Ok(())
}

/// Signed unit columns (side, up, front).
fn basis(space: TargetSpace) -> [[i8; 3]; 3] {
    let up = axis_vector(space.up_axis);
    let front = axis_vector(space.front_axis);
    let sign = if space.handedness == Handedness::Right {
        1
    } else {
        -1
    };
    let side = [
        sign * (front[1] * up[2] - front[2] * up[1]),
        sign * (front[2] * up[0] - front[0] * up[2]),
        sign * (front[0] * up[1] - front[1] * up[0]),
    ];
    [side, up, front]
}

fn axis_vector(axis: Axis) -> [i8; 3] {
    match axis {
        Axis::PositiveX => [1, 0, 0],
        Axis::NegativeX => [-1, 0, 0],
        Axis::PositiveY => [0, 1, 0],
        Axis::NegativeY => [0, -1, 0],
        Axis::PositiveZ => [0, 0, 1],
        Axis::NegativeZ => [0, 0, -1],
        Axis::Unknown => unreachable!("TargetSpace axes are known by construction"),
    }
}

fn signed(value: f32, negative: bool) -> f32 {
    if negative { -value } else { value }
}

fn round_component(value: f64, was_nonzero: bool) -> Result<f32, NumericConversionError> {
    let output = value as f32;
    if !output.is_finite() {
        return Err(NumericConversionError::NonFiniteOutput);
    }
    // Check the original component too: the f64 intermediate itself may have underflowed.
    if was_nonzero && output == 0.0 {
        return Err(NumericConversionError::UnderflowToZero);
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
