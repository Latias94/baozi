//! Tangent construction and bitangent reconstruction declarations owned by one attribute.

use crate::extension::ExtensionSchemaId;
use thiserror::Error;

/// Algorithm or externally defined convention used to construct a tangent attribute.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TangentBasisConvention {
    Unknown,
    MikkTspace,
    /// Per-triangle UV derivatives followed by normalized Gram-Schmidt orthogonalization.
    TriangleUvDerivative,
    Namespaced {
        schema_id: ExtensionSchemaId,
    },
}

impl TangentBasisConvention {
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, TangentSchemaBindingError> {
        let schema_id = schema_id.ok_or(TangentSchemaBindingError::SchemaRequired {
            field: TangentConventionField::Basis,
        })?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Unknown | Self::MikkTspace | Self::TriangleUvDerivative => None,
        }
    }
}

/// Encoding used to reconstruct the bitangent from this tangent attribute.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TangentHandednessEncoding {
    Unknown,
    /// Reconstructs the bitangent as `tangent.w * cross(normal, tangent.xyz)`.
    SignInW,
    Namespaced {
        schema_id: ExtensionSchemaId,
    },
}

impl TangentHandednessEncoding {
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, TangentSchemaBindingError> {
        let schema_id = schema_id.ok_or(TangentSchemaBindingError::SchemaRequired {
            field: TangentConventionField::Encoding,
        })?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Unknown | Self::SignInW => None,
        }
    }
}

/// Independent construction and reconstruction facts for exactly one tangent descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TangentConventions {
    pub basis: TangentBasisConvention,
    pub encoding: TangentHandednessEncoding,
}

impl TangentConventions {
    /// Authored tangent values whose construction and reconstruction conventions are unknown.
    pub const UNKNOWN: Self = Self {
        basis: TangentBasisConvention::Unknown,
        encoding: TangentHandednessEncoding::Unknown,
    };

    /// Validates the declarations without inferring facts from payload values or scene space.
    pub fn validate(&self) -> Result<(), TangentConventionsValidationError> {
        if matches!(self.basis, TangentBasisConvention::MikkTspace)
            && !matches!(self.encoding, TangentHandednessEncoding::SignInW)
        {
            return Err(TangentConventionsValidationError::MikkTspaceRequiresSignInW);
        }
        Ok(())
    }
}

impl Default for TangentConventions {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TangentConventionField {
    Basis,
    Encoding,
}

/// Why an external tangent convention could not be bound to a schema identity.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TangentSchemaBindingError {
    #[error("namespaced tangent {field:?} requires an extension schema")]
    SchemaRequired { field: TangentConventionField },
}

/// Why one attribute's tangent declaration is invalid.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TangentConventionsValidationError {
    #[error("MikkTSpace tangent basis requires tangent handedness encoded as a sign in w")]
    MikkTspaceRequiresSignInW,
    #[error("tangent attributes require explicit tangent conventions")]
    MissingConventions,
    #[error("only tangent attributes may declare tangent conventions")]
    UnexpectedConventions,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_known_encoding_are_independent_of_construction() {
        assert_eq!(TangentConventions::UNKNOWN.validate(), Ok(()));
        assert_eq!(
            TangentConventions {
                basis: TangentBasisConvention::Unknown,
                encoding: TangentHandednessEncoding::SignInW,
            }
            .validate(),
            Ok(())
        );
    }

    #[test]
    fn mikk_requires_its_defined_reconstruction_encoding() {
        for encoding in [
            TangentHandednessEncoding::Unknown,
            TangentHandednessEncoding::Namespaced {
                schema_id: ExtensionSchemaId::new("test:encoding", 1).unwrap(),
            },
        ] {
            assert_eq!(
                TangentConventions {
                    basis: TangentBasisConvention::MikkTspace,
                    encoding,
                }
                .validate(),
                Err(TangentConventionsValidationError::MikkTspaceRequiresSignInW)
            );
        }
    }

    #[test]
    fn external_conventions_require_schema_identities() {
        assert_eq!(
            TangentBasisConvention::try_namespaced(None),
            Err(TangentSchemaBindingError::SchemaRequired {
                field: TangentConventionField::Basis,
            })
        );
        assert_eq!(
            TangentHandednessEncoding::try_namespaced(None),
            Err(TangentSchemaBindingError::SchemaRequired {
                field: TangentConventionField::Encoding,
            })
        );
    }
}
