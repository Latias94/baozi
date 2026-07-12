//! Declarative scene coordinate and rendering conventions.
//!
//! This module describes and validates source or target conventions. It does
//! not transform geometry, hierarchy data, animation, or derived attributes.

use crate::extension::ExtensionSchemaId;
use std::fmt;
use thiserror::Error;

/// A signed coordinate axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    Unknown,
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl Axis {
    /// Returns whether this axis is explicitly unknown.
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// Returns whether both values refer to the same unsigned axis.
    ///
    /// Unknown values never match another axis, including another unknown.
    pub const fn shares_underlying_axis(self, other: Self) -> bool {
        match (self.underlying_axis(), other.underlying_axis()) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    }

    const fn underlying_axis(self) -> Option<u8> {
        match self {
            Self::Unknown => None,
            Self::PositiveX | Self::NegativeX => Some(0),
            Self::PositiveY | Self::NegativeY => Some(1),
            Self::PositiveZ | Self::NegativeZ => Some(2),
        }
    }
}

/// Orientation of the scene coordinate basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handedness {
    Unknown,
    Right,
    Left,
}

impl Handedness {
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// Vertex winding that identifies the front side of a face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrontFaceWinding {
    Unknown,
    Clockwise,
    CounterClockwise,
}

impl FrontFaceWinding {
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// Origin used by authored UV coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UvOrigin {
    Unknown,
    UpperLeft,
    LowerLeft,
}

impl UvOrigin {
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// Matrix/vector convention used by scene transforms.
///
/// The known canonical convention means
/// `p_parent = M_local_to_parent * p_local`. It states semantic multiplication
/// order and does not promise a matrix byte layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransformConvention {
    Unknown,
    ColumnVectorLocalToParent,
}

impl TransformConvention {
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// Algorithm or externally defined convention used to construct tangent space.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TangentBasisConvention {
    Unknown,
    MikkTspace,
    Namespaced { schema_id: ExtensionSchemaId },
}

impl TangentBasisConvention {
    /// Creates an external tangent-basis behavior backed by a declarative schema.
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, SceneSpaceSchemaBindingError> {
        let schema_id = schema_id.ok_or(SceneSpaceSchemaBindingError::SchemaRequired {
            field: SceneSpaceField::TangentBasis,
        })?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// Returns the schema defining an external tangent-basis convention.
    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Unknown | Self::MikkTspace => None,
        }
    }
}

/// Encoding used to reconstruct the bitangent from a tangent attribute.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TangentHandednessEncoding {
    Unknown,

    /// Reconstructs the bitangent as
    /// `tangent.w * cross(normal, tangent.xyz)`.
    SignInW,

    /// A bounded extension convention defined outside Baozi core.
    Namespaced {
        schema_id: ExtensionSchemaId,
    },
}

impl TangentHandednessEncoding {
    /// Creates an external handedness encoding backed by a declarative schema.
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, SceneSpaceSchemaBindingError> {
        let schema_id = schema_id.ok_or(SceneSpaceSchemaBindingError::SchemaRequired {
            field: SceneSpaceField::TangentHandednessEncoding,
        })?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// Returns the schema defining an external tangent-handedness encoding.
    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Unknown | Self::SignInW => None,
        }
    }
}

/// Coordinate, unit, transform, and render conventions carried by a scene.
///
/// Unknown source facts remain explicit. [`Self::validate`] accepts a partial
/// descriptor, while [`Self::validate_complete`] requires every field to be
/// known. Neither method performs coordinate conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSpace {
    pub handedness: Handedness,
    pub up_axis: Axis,
    pub front_axis: Axis,
    pub unit_scale_to_meters: Option<f64>,
    pub front_face_winding: FrontFaceWinding,
    pub uv_origin: UvOrigin,
    pub transform_convention: TransformConvention,
    pub tangent_basis: TangentBasisConvention,
    pub tangent_handedness_encoding: TangentHandednessEncoding,
}

impl SceneSpace {
    /// A descriptor in which every convention is explicitly unknown.
    pub const UNKNOWN: Self = Self {
        handedness: Handedness::Unknown,
        up_axis: Axis::Unknown,
        front_axis: Axis::Unknown,
        unit_scale_to_meters: None,
        front_face_winding: FrontFaceWinding::Unknown,
        uv_origin: UvOrigin::Unknown,
        transform_convention: TransformConvention::Unknown,
        tangent_basis: TangentBasisConvention::Unknown,
        tangent_handedness_encoding: TangentHandednessEncoding::Unknown,
    };

    /// The standard spatial target for an explicit conversion request.
    ///
    /// UV origin and tangent conventions intentionally remain unknown so a
    /// caller must preserve them or explicitly name target conventions.
    pub const STANDARD_TARGET: Self = Self {
        handedness: Handedness::Right,
        up_axis: Axis::PositiveY,
        front_axis: Axis::NegativeZ,
        unit_scale_to_meters: Some(1.0),
        front_face_winding: FrontFaceWinding::CounterClockwise,
        uv_origin: UvOrigin::Unknown,
        transform_convention: TransformConvention::ColumnVectorLocalToParent,
        tangent_basis: TangentBasisConvention::Unknown,
        tangent_handedness_encoding: TangentHandednessEncoding::Unknown,
    };

    pub const fn unknown() -> Self {
        Self::UNKNOWN
    }

    pub const fn standard_target() -> Self {
        Self::STANDARD_TARGET
    }

    /// Validates a partial scene-space descriptor.
    ///
    /// Individual spatial conventions may remain unknown. Tangent basis and
    /// handedness encoding are one semantic declaration, so they must either
    /// both be unknown or both be known.
    pub fn validate(&self) -> Result<(), SceneSpaceValidationError> {
        if let Some(scale) = self.unit_scale_to_meters {
            if !scale.is_finite() {
                return Err(SceneSpaceValidationError::NonFiniteUnitScale);
            }
            if scale <= 0.0 {
                return Err(SceneSpaceValidationError::NonPositiveUnitScale);
            }
        }

        if self.up_axis.shares_underlying_axis(self.front_axis) {
            return Err(SceneSpaceValidationError::CoincidentAxes {
                up_axis: self.up_axis,
                front_axis: self.front_axis,
            });
        }

        let tangent_basis_unknown = self.tangent_basis.is_unknown();
        let tangent_encoding_unknown = self.tangent_handedness_encoding.is_unknown();
        match (tangent_basis_unknown, tangent_encoding_unknown) {
            (true, false) => {
                return Err(SceneSpaceValidationError::IncompleteTangentConvention {
                    missing: SceneSpaceField::TangentBasis,
                });
            }
            (false, true) => {
                return Err(SceneSpaceValidationError::IncompleteTangentConvention {
                    missing: SceneSpaceField::TangentHandednessEncoding,
                });
            }
            _ => {}
        }

        if matches!(self.tangent_basis, TangentBasisConvention::MikkTspace)
            && !matches!(
                self.tangent_handedness_encoding,
                TangentHandednessEncoding::SignInW
            )
        {
            return Err(SceneSpaceValidationError::MikkTspaceRequiresSignInW);
        }

        Ok(())
    }

    /// Validates a descriptor that claims every convention is known.
    pub fn validate_complete(&self) -> Result<(), SceneSpaceValidationError> {
        self.validate()?;

        let missing = if self.handedness.is_unknown() {
            Some(SceneSpaceField::Handedness)
        } else if self.up_axis.is_unknown() {
            Some(SceneSpaceField::UpAxis)
        } else if self.front_axis.is_unknown() {
            Some(SceneSpaceField::FrontAxis)
        } else if self.unit_scale_to_meters.is_none() {
            Some(SceneSpaceField::UnitScaleToMeters)
        } else if self.front_face_winding.is_unknown() {
            Some(SceneSpaceField::FrontFaceWinding)
        } else if self.uv_origin.is_unknown() {
            Some(SceneSpaceField::UvOrigin)
        } else if self.transform_convention.is_unknown() {
            Some(SceneSpaceField::TransformConvention)
        } else if self.tangent_basis.is_unknown() {
            Some(SceneSpaceField::TangentBasis)
        } else if self.tangent_handedness_encoding.is_unknown() {
            Some(SceneSpaceField::TangentHandednessEncoding)
        } else {
            None
        };

        match missing {
            Some(field) => Err(SceneSpaceValidationError::UnknownField { field }),
            None => Ok(()),
        }
    }
}

impl Default for SceneSpace {
    fn default() -> Self {
        Self::unknown()
    }
}

/// A field whose convention may be unknown in a partial [`SceneSpace`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneSpaceField {
    Handedness,
    UpAxis,
    FrontAxis,
    UnitScaleToMeters,
    FrontFaceWinding,
    UvOrigin,
    TransformConvention,
    TangentBasis,
    TangentHandednessEncoding,
}

impl fmt::Display for SceneSpaceField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Handedness => "handedness",
            Self::UpAxis => "up axis",
            Self::FrontAxis => "front axis",
            Self::UnitScaleToMeters => "unit scale to meters",
            Self::FrontFaceWinding => "front-face winding",
            Self::UvOrigin => "UV origin",
            Self::TransformConvention => "transform convention",
            Self::TangentBasis => "tangent-basis convention",
            Self::TangentHandednessEncoding => "tangent-handedness encoding",
        })
    }
}

/// Why an external scene-space behavior could not be bound declaratively.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SceneSpaceSchemaBindingError {
    #[error("namespaced scene-space {field} requires an admitted extension schema")]
    SchemaRequired { field: SceneSpaceField },
}

/// Why a scene-space descriptor is invalid or incomplete.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SceneSpaceValidationError {
    #[error("unit scale to meters must be finite when known")]
    NonFiniteUnitScale,

    #[error("unit scale to meters must be positive when known")]
    NonPositiveUnitScale,

    #[error("up axis {up_axis:?} and front axis {front_axis:?} share an underlying axis")]
    CoincidentAxes { up_axis: Axis, front_axis: Axis },

    #[error("tangent convention is incomplete because {missing} is unknown")]
    IncompleteTangentConvention { missing: SceneSpaceField },

    #[error("MikkTSpace tangent basis requires tangent handedness encoded as a sign in w")]
    MikkTspaceRequiresSignInW,

    #[error("complete scene space has an unknown {field}")]
    UnknownField { field: SceneSpaceField },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_space() -> SceneSpace {
        SceneSpace {
            handedness: Handedness::Right,
            up_axis: Axis::PositiveY,
            front_axis: Axis::NegativeZ,
            unit_scale_to_meters: Some(1.0),
            front_face_winding: FrontFaceWinding::CounterClockwise,
            uv_origin: UvOrigin::UpperLeft,
            transform_convention: TransformConvention::ColumnVectorLocalToParent,
            tangent_basis: TangentBasisConvention::MikkTspace,
            tangent_handedness_encoding: TangentHandednessEncoding::SignInW,
        }
    }

    #[test]
    fn unknown_descriptor_is_valid_but_not_complete() {
        let space = SceneSpace::default();

        assert_eq!(space, SceneSpace::UNKNOWN);
        assert_eq!(space.validate(), Ok(()));
        assert_eq!(
            space.validate_complete(),
            Err(SceneSpaceValidationError::UnknownField {
                field: SceneSpaceField::Handedness,
            })
        );
    }

    #[test]
    fn standard_target_is_valid_and_preserves_unspecified_conventions() {
        let space = SceneSpace::standard_target();

        assert_eq!(space.validate(), Ok(()));
        assert_eq!(space.uv_origin, UvOrigin::Unknown);
        assert_eq!(space.tangent_basis, TangentBasisConvention::Unknown);
        assert_eq!(
            space.validate_complete(),
            Err(SceneSpaceValidationError::UnknownField {
                field: SceneSpaceField::UvOrigin,
            })
        );
    }

    #[test]
    fn complete_descriptor_is_valid() {
        assert_eq!(complete_space().validate_complete(), Ok(()));
    }

    #[test]
    fn unit_scale_accepts_positive_finite_boundaries() {
        for scale in [f64::from_bits(1), f64::MIN_POSITIVE, f64::MAX] {
            let space = SceneSpace {
                unit_scale_to_meters: Some(scale),
                ..SceneSpace::default()
            };
            assert_eq!(space.validate(), Ok(()), "rejected scale {scale}");
        }
    }

    #[test]
    fn unit_scale_rejects_non_finite_and_non_positive_values() {
        for scale in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let space = SceneSpace {
                unit_scale_to_meters: Some(scale),
                ..SceneSpace::default()
            };
            assert_eq!(
                space.validate(),
                Err(SceneSpaceValidationError::NonFiniteUnitScale)
            );
        }

        for scale in [0.0, -0.0, -f64::from_bits(1), -1.0] {
            let space = SceneSpace {
                unit_scale_to_meters: Some(scale),
                ..SceneSpace::default()
            };
            assert_eq!(
                space.validate(),
                Err(SceneSpaceValidationError::NonPositiveUnitScale)
            );
        }
    }

    #[test]
    fn signed_up_and_front_axes_must_have_distinct_underlying_axes() {
        let invalid = SceneSpace {
            up_axis: Axis::PositiveX,
            front_axis: Axis::NegativeX,
            ..SceneSpace::default()
        };
        assert_eq!(
            invalid.validate(),
            Err(SceneSpaceValidationError::CoincidentAxes {
                up_axis: Axis::PositiveX,
                front_axis: Axis::NegativeX,
            })
        );

        let valid = SceneSpace {
            up_axis: Axis::NegativeY,
            front_axis: Axis::PositiveZ,
            ..SceneSpace::default()
        };
        assert_eq!(valid.validate(), Ok(()));
    }

    #[test]
    fn complete_validation_reports_each_unknown_field() {
        let cases = [
            (
                SceneSpace {
                    handedness: Handedness::Unknown,
                    ..complete_space()
                },
                SceneSpaceField::Handedness,
            ),
            (
                SceneSpace {
                    up_axis: Axis::Unknown,
                    ..complete_space()
                },
                SceneSpaceField::UpAxis,
            ),
            (
                SceneSpace {
                    front_axis: Axis::Unknown,
                    ..complete_space()
                },
                SceneSpaceField::FrontAxis,
            ),
            (
                SceneSpace {
                    unit_scale_to_meters: None,
                    ..complete_space()
                },
                SceneSpaceField::UnitScaleToMeters,
            ),
            (
                SceneSpace {
                    front_face_winding: FrontFaceWinding::Unknown,
                    ..complete_space()
                },
                SceneSpaceField::FrontFaceWinding,
            ),
            (
                SceneSpace {
                    uv_origin: UvOrigin::Unknown,
                    ..complete_space()
                },
                SceneSpaceField::UvOrigin,
            ),
            (
                SceneSpace {
                    transform_convention: TransformConvention::Unknown,
                    ..complete_space()
                },
                SceneSpaceField::TransformConvention,
            ),
            (
                SceneSpace {
                    tangent_basis: TangentBasisConvention::Unknown,
                    tangent_handedness_encoding: TangentHandednessEncoding::Unknown,
                    ..complete_space()
                },
                SceneSpaceField::TangentBasis,
            ),
        ];

        for (space, field) in cases {
            assert_eq!(
                space.validate_complete(),
                Err(SceneSpaceValidationError::UnknownField { field })
            );
        }
    }

    #[test]
    fn tangent_basis_and_handedness_must_form_one_declaration() {
        let missing_encoding = SceneSpace {
            tangent_basis: TangentBasisConvention::MikkTspace,
            ..SceneSpace::default()
        };
        assert_eq!(
            missing_encoding.validate(),
            Err(SceneSpaceValidationError::IncompleteTangentConvention {
                missing: SceneSpaceField::TangentHandednessEncoding,
            })
        );

        let missing_basis = SceneSpace {
            tangent_handedness_encoding: TangentHandednessEncoding::SignInW,
            ..SceneSpace::default()
        };
        assert_eq!(
            missing_basis.validate(),
            Err(SceneSpaceValidationError::IncompleteTangentConvention {
                missing: SceneSpaceField::TangentBasis,
            })
        );
    }

    #[test]
    fn mikk_tspace_rejects_a_contradictory_handedness_encoding() {
        let schema_id = ExtensionSchemaId::new("example:tangent-sign", 1)
            .expect("test tangent-handedness schema should be canonical");
        let encoding = TangentHandednessEncoding::try_namespaced(Some(schema_id))
            .expect("schema-bound tangent handedness should be accepted");
        let space = SceneSpace {
            tangent_basis: TangentBasisConvention::MikkTspace,
            tangent_handedness_encoding: encoding,
            ..SceneSpace::default()
        };

        assert_eq!(
            space.validate(),
            Err(SceneSpaceValidationError::MikkTspaceRequiresSignInW)
        );
    }

    #[test]
    fn namespaced_tangent_conventions_are_preserved() {
        assert_eq!(
            TangentBasisConvention::try_namespaced(None),
            Err(SceneSpaceSchemaBindingError::SchemaRequired {
                field: SceneSpaceField::TangentBasis,
            })
        );
        assert_eq!(
            TangentHandednessEncoding::try_namespaced(None),
            Err(SceneSpaceSchemaBindingError::SchemaRequired {
                field: SceneSpaceField::TangentHandednessEncoding,
            })
        );

        let basis_schema = ExtensionSchemaId::new("example:tangent-basis", 2)
            .expect("test tangent-basis schema should be canonical");
        let handedness_schema = ExtensionSchemaId::new("example:tangent-sign", 3)
            .expect("test tangent-handedness schema should be canonical");
        let mut space = complete_space();
        space.handedness = Handedness::Left;
        space.front_face_winding = FrontFaceWinding::Clockwise;
        space.uv_origin = UvOrigin::LowerLeft;
        space.tangent_basis = TangentBasisConvention::try_namespaced(Some(basis_schema.clone()))
            .expect("schema-bound tangent basis should be accepted");
        space.tangent_handedness_encoding =
            TangentHandednessEncoding::try_namespaced(Some(handedness_schema.clone()))
                .expect("schema-bound tangent handedness should be accepted");

        assert_eq!(space.validate_complete(), Ok(()));
        assert_eq!(space.tangent_basis.schema_id(), Some(&basis_schema));
        assert_eq!(
            space.tangent_handedness_encoding.schema_id(),
            Some(&handedness_schema)
        );
    }
}
