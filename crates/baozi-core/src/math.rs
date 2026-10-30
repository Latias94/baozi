/// Maximum absolute error accepted for a quaternion's squared length.
pub const QUATERNION_UNIT_TOLERANCE: f64 = 1.0e-4;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// A matrix with column-vector multiplication semantics: `p_out = M * p_in`.
///
/// The owning domain defines the input and output spaces. Node base poses use
/// [`crate::AuthoredTransform`]'s local-to-parent contract. These semantics are
/// fixed independently of scene-space declarations and promise no binary ABI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4 {
    pub cols: [[f32; 4]; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Vec4 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }
}

impl Quat {
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub const fn from_xyzw(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }

    pub fn length_squared_f64(self) -> f64 {
        [self.x, self.y, self.z, self.w]
            .into_iter()
            .map(|component| f64::from(component) * f64::from(component))
            .sum()
    }

    pub fn is_unit(self) -> bool {
        self.is_finite() && (self.length_squared_f64() - 1.0).abs() <= QUATERNION_UNIT_TOLERANCE
    }
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    pub fn is_finite(self) -> bool {
        self.cols.iter().flatten().all(|value| value.is_finite())
    }

    pub fn is_affine(self) -> bool {
        self.cols[0][3] == 0.0
            && self.cols[1][3] == 0.0
            && self.cols[2][3] == 0.0
            && self.cols[3][3] == 1.0
    }

    /// Composes a column-vector local transform as `T * R * S`.
    pub fn from_trs(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        let x2 = rotation.x + rotation.x;
        let y2 = rotation.y + rotation.y;
        let z2 = rotation.z + rotation.z;
        let xx = rotation.x * x2;
        let xy = rotation.x * y2;
        let xz = rotation.x * z2;
        let yy = rotation.y * y2;
        let yz = rotation.y * z2;
        let zz = rotation.z * z2;
        let wx = rotation.w * x2;
        let wy = rotation.w * y2;
        let wz = rotation.w * z2;

        Self {
            cols: [
                [
                    (1.0 - (yy + zz)) * scale.x,
                    (xy + wz) * scale.x,
                    (xz - wy) * scale.x,
                    0.0,
                ],
                [
                    (xy - wz) * scale.y,
                    (1.0 - (xx + zz)) * scale.y,
                    (yz + wx) * scale.y,
                    0.0,
                ],
                [
                    (xz + wy) * scale.z,
                    (yz - wx) * scale.z,
                    (1.0 - (xx + yy)) * scale.z,
                    0.0,
                ],
                [translation.x, translation.y, translation.z, 1.0],
            ],
        }
    }
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Color {
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    pub const fn linear_rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::WHITE
    }
}

#[cfg(feature = "mint")]
impl From<mint::Vector3<f32>> for Vec3 {
    fn from(value: mint::Vector3<f32>) -> Self {
        Self::new(value.x, value.y, value.z)
    }
}

#[cfg(feature = "mint")]
impl From<Vec3> for mint::Vector3<f32> {
    fn from(value: Vec3) -> Self {
        Self {
            x: value.x,
            y: value.y,
            z: value.z,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_non_finite_vectors() {
        assert!(Vec3::new(1.0, 2.0, 3.0).is_finite());
        assert!(!Vec3::new(1.0, f32::NAN, 3.0).is_finite());
    }

    #[test]
    fn composes_column_vector_trs_without_rewriting_components() {
        let matrix = Mat4::from_trs(
            Vec3::new(4.0, 5.0, 6.0),
            Quat::IDENTITY,
            Vec3::new(-2.0, 0.0, 3.0),
        );

        assert_eq!(
            matrix,
            Mat4 {
                cols: [
                    [-2.0, -0.0, -0.0, 0.0],
                    [0.0, 0.0, 0.0, 0.0],
                    [0.0, 0.0, 3.0, 0.0],
                    [4.0, 5.0, 6.0, 1.0],
                ],
            }
        );
        assert!(matrix.is_affine());
    }

    #[test]
    fn composes_non_identity_rotation_and_non_uniform_scale_in_column_major_order() {
        let half_sqrt = std::f32::consts::FRAC_1_SQRT_2;
        let matrix = Mat4::from_trs(
            Vec3::new(5.0, 6.0, 7.0),
            Quat::from_xyzw(0.0, 0.0, half_sqrt, half_sqrt),
            Vec3::new(2.0, 3.0, 4.0),
        );
        let expected = Mat4 {
            cols: [
                [0.0, 2.0, 0.0, 0.0],
                [-3.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 4.0, 0.0],
                [5.0, 6.0, 7.0, 1.0],
            ],
        };

        for (actual_column, expected_column) in matrix.cols.iter().zip(expected.cols) {
            for (actual, expected) in actual_column.iter().zip(expected_column) {
                assert!(
                    (actual - expected).abs() <= 1.0e-6,
                    "expected {expected}, got {actual}"
                );
            }
        }
        assert!(matrix.is_affine());
    }
}
