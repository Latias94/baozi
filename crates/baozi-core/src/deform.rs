use crate::math::Mat4;
use thiserror::Error;

/// A coordinate space participating in the canonical skin matrix equation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkinMatrixSpace {
    LogicalMeshLocalBind,
    JointLocalBind,
    JointLocal,
    Scene,
    MeshInstanceLocal,
}

/// A source-declared correction matrix with explicit input and output spaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinMatrixCorrection {
    from: SkinMatrixSpace,
    to: SkinMatrixSpace,
    matrix: Mat4,
}

impl SkinMatrixCorrection {
    pub fn new(
        from: SkinMatrixSpace,
        to: SkinMatrixSpace,
        matrix: Mat4,
    ) -> Result<Self, SkinMatrixCorrectionError> {
        if !matrix.is_finite() {
            return Err(SkinMatrixCorrectionError::NonFiniteMatrix);
        }

        Ok(Self { from, to, matrix })
    }

    pub const fn from(self) -> SkinMatrixSpace {
        self.from
    }

    pub const fn to(self) -> SkinMatrixSpace {
        self.to
    }

    pub const fn matrix(self) -> Mat4 {
        self.matrix
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SkinMatrixCorrectionError {
    #[error("skin correction matrix contains a non-finite value")]
    NonFiniteMatrix,
}

/// The explicit canonical matrix-space contract for skin evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinMatrixSpaces {
    input_position: SkinMatrixSpace,
    inverse_bind_from: SkinMatrixSpace,
    inverse_bind_to: SkinMatrixSpace,
    current_joint_from: SkinMatrixSpace,
    current_joint_to: SkinMatrixSpace,
    output_position: SkinMatrixSpace,
    correction: Option<SkinMatrixCorrection>,
}

impl SkinMatrixSpaces {
    /// Returns Baozi's canonical instance-local skinning contract.
    pub const fn canonical() -> Self {
        Self {
            input_position: SkinMatrixSpace::LogicalMeshLocalBind,
            inverse_bind_from: SkinMatrixSpace::LogicalMeshLocalBind,
            inverse_bind_to: SkinMatrixSpace::JointLocalBind,
            current_joint_from: SkinMatrixSpace::JointLocal,
            current_joint_to: SkinMatrixSpace::Scene,
            output_position: SkinMatrixSpace::MeshInstanceLocal,
            correction: None,
        }
    }

    /// Attaches a validated, source-declared correction without changing the
    /// canonical interpretation of the other matrices.
    pub const fn with_correction(mut self, correction: SkinMatrixCorrection) -> Self {
        self.correction = Some(correction);
        self
    }

    pub const fn input_position(self) -> SkinMatrixSpace {
        self.input_position
    }

    pub const fn inverse_bind_from(self) -> SkinMatrixSpace {
        self.inverse_bind_from
    }

    pub const fn inverse_bind_to(self) -> SkinMatrixSpace {
        self.inverse_bind_to
    }

    pub const fn current_joint_from(self) -> SkinMatrixSpace {
        self.current_joint_from
    }

    pub const fn current_joint_to(self) -> SkinMatrixSpace {
        self.current_joint_to
    }

    pub const fn output_position(self) -> SkinMatrixSpace {
        self.output_position
    }

    pub const fn correction(self) -> Option<SkinMatrixCorrection> {
        self.correction
    }
}

impl Default for SkinMatrixSpaces {
    fn default() -> Self {
        Self::canonical()
    }
}

/// An admitted relative morph-delta semantic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MorphDeltaSemantic {
    Position,
    Normal,
    Tangent,
}

#[cfg(test)]
mod tests {
    use super::{SkinMatrixSpace, SkinMatrixSpaces};

    #[test]
    fn canonical_skin_matrix_spaces_are_explicit() {
        let spaces = SkinMatrixSpaces::canonical();

        assert_eq!(
            spaces.input_position(),
            SkinMatrixSpace::LogicalMeshLocalBind
        );
        assert_eq!(spaces.inverse_bind_to(), SkinMatrixSpace::JointLocalBind);
        assert_eq!(spaces.current_joint_to(), SkinMatrixSpace::Scene);
        assert_eq!(spaces.output_position(), SkinMatrixSpace::MeshInstanceLocal);
        assert_eq!(spaces.correction(), None);
    }
}
