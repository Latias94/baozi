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

/// A finite affine source correction with explicit input and output spaces.
///
/// The matrix may contain shear, reflection, or a singular linear part. Construction
/// preserves all scalar bits and does not decompose the matrix into TRS components.
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
        if !matrix.is_affine() {
            return Err(SkinMatrixCorrectionError::NonAffineMatrix);
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

    #[cfg(test)]
    pub(crate) fn corrupt_matrix_for_test(&mut self, matrix: Mat4) {
        self.matrix = matrix;
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SkinMatrixCorrectionError {
    #[error("skin correction matrix contains a non-finite value")]
    NonFiniteMatrix,
    #[error("skin correction matrix is not exactly affine")]
    NonAffineMatrix,
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
    use super::{
        SkinMatrixCorrection, SkinMatrixCorrectionError, SkinMatrixSpace, SkinMatrixSpaces,
    };
    use crate::math::Mat4;

    #[test]
    fn corrections_require_finite_affine_matrices_without_rewriting_bits() {
        let make = |matrix| {
            SkinMatrixCorrection::new(
                SkinMatrixSpace::LogicalMeshLocalBind,
                SkinMatrixSpace::MeshInstanceLocal,
                matrix,
            )
        };
        let mut matrix = Mat4::IDENTITY;
        matrix.cols[0][0] = -2.0;
        matrix.cols[1][0] = 0.25;
        matrix.cols[2][2] = 0.0;
        matrix.cols[1][3] = -0.0;
        matrix.cols[3][0] = f32::from_bits(1);
        let actual = make(matrix).unwrap().matrix();
        for (actual, expected) in actual
            .cols
            .into_iter()
            .flatten()
            .zip(matrix.cols.into_iter().flatten())
        {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
        for (column, row, value) in [(0, 3, 0.5), (3, 3, 2.0)] {
            let mut invalid = matrix;
            invalid.cols[column][row] = value;
            assert_eq!(
                make(invalid),
                Err(SkinMatrixCorrectionError::NonAffineMatrix)
            );
        }
        matrix.cols[0][0] = f32::NAN;
        assert_eq!(
            make(matrix),
            Err(SkinMatrixCorrectionError::NonFiniteMatrix)
        );
    }

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
