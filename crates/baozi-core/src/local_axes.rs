//! A complete discrete reference frame, independent of an entity's node placement.

use thiserror::Error;

use crate::math::Mat4;
use crate::scene_space::Axis;

/// A reference direction in a camera or light's intrinsic frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAxis {
    Right,
    Up,
    Forward,
}

/// Why three signed cardinal axes do not define a complete reference frame.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LocalAxesError {
    #[error("the local {axis:?} axis is unknown")]
    UnknownAxis { axis: LocalAxis },
    #[error("local {first:?} and {second:?} axes use the same underlying axis")]
    CoincidentAxes { first: LocalAxis, second: LocalAxis },
}

/// Maps the fixed optical reference frame into node-local coordinates.
///
/// All 48 signed permutations are representable, including reflections. This value
/// contains no origin, distance, scale, or authored placement rotation. The attached
/// node remains the sole placement owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalAxes {
    right: Axis,
    up: Axis,
    forward: Axis,
}

impl LocalAxes {
    /// Right +X, up +Y, forward -Z in the attached node's coordinates.
    pub const STANDARD: Self = Self {
        right: Axis::PositiveX,
        up: Axis::PositiveY,
        forward: Axis::NegativeZ,
    };

    pub fn try_new(right: Axis, up: Axis, forward: Axis) -> Result<Self, LocalAxesError> {
        let result = Self { right, up, forward };
        result.validate()?;
        Ok(result)
    }

    pub const fn right(self) -> Axis {
        self.right
    }

    pub const fn up(self) -> Axis {
        self.up
    }

    pub const fn forward(self) -> Axis {
        self.forward
    }

    /// The column-vector map from the fixed +X/+Y/-Z optical frame to node-local space.
    /// It is orthogonal, has zero translation, and may have determinant -1.
    pub fn basis_matrix(self) -> Mat4 {
        let mut result = Mat4::IDENTITY;
        for (column, axis) in [self.right, self.up, opposite(self.forward)]
            .into_iter()
            .enumerate()
        {
            result.cols[column] = axis_column(axis);
        }
        result
    }

    pub(crate) fn validate(self) -> Result<(), LocalAxesError> {
        let axes = [
            (LocalAxis::Right, self.right),
            (LocalAxis::Up, self.up),
            (LocalAxis::Forward, self.forward),
        ];
        for (axis, value) in axes {
            if value.is_unknown() {
                return Err(LocalAxesError::UnknownAxis { axis });
            }
        }
        for (first, second) in [(0, 1), (0, 2), (1, 2)] {
            if axes[first].1.shares_underlying_axis(axes[second].1) {
                return Err(LocalAxesError::CoincidentAxes {
                    first: axes[first].0,
                    second: axes[second].0,
                });
            }
        }
        Ok(())
    }
}

fn opposite(axis: Axis) -> Axis {
    match axis {
        Axis::PositiveX => Axis::NegativeX,
        Axis::NegativeX => Axis::PositiveX,
        Axis::PositiveY => Axis::NegativeY,
        Axis::NegativeY => Axis::PositiveY,
        Axis::PositiveZ => Axis::NegativeZ,
        Axis::NegativeZ => Axis::PositiveZ,
        Axis::Unknown => unreachable!("LocalAxes construction requires known axes"),
    }
}

fn axis_column(axis: Axis) -> [f32; 4] {
    match axis {
        Axis::PositiveX => [1.0, 0.0, 0.0, 0.0],
        Axis::NegativeX => [-1.0, 0.0, 0.0, 0.0],
        Axis::PositiveY => [0.0, 1.0, 0.0, 0.0],
        Axis::NegativeY => [0.0, -1.0, 0.0, 0.0],
        Axis::PositiveZ => [0.0, 0.0, 1.0, 0.0],
        Axis::NegativeZ => [0.0, 0.0, -1.0, 0.0],
        Axis::Unknown => unreachable!("LocalAxes construction requires known axes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_48_frames_preserve_three_independent_signed_axes_without_allocation() {
        let axes = [
            Axis::PositiveX,
            Axis::NegativeX,
            Axis::PositiveY,
            Axis::NegativeY,
            Axis::PositiveZ,
            Axis::NegativeZ,
        ];
        let mut count = 0;
        let measured = allocation_counter::measure(|| {
            for right in axes {
                for up in axes {
                    for forward in axes {
                        let Ok(frame) = LocalAxes::try_new(right, up, forward) else {
                            continue;
                        };
                        assert_eq!(
                            (frame.right(), frame.up(), frame.forward()),
                            (right, up, forward)
                        );
                        let matrix = frame.basis_matrix();
                        assert_eq!(matrix.cols[0], axis_column(right));
                        assert_eq!(matrix.cols[1], axis_column(up));
                        assert_eq!(matrix.cols[2], axis_column(opposite(forward)));
                        assert_eq!(matrix.cols[3], [0.0, 0.0, 0.0, 1.0]);
                        count += 1;
                    }
                }
            }
        });
        assert_eq!(count, 48);
        assert_eq!(measured.count_total, 0);
        assert_eq!(LocalAxes::STANDARD.basis_matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn unknown_and_coincident_axes_are_rejected_before_a_frame_exists() {
        assert_eq!(
            LocalAxes::try_new(Axis::Unknown, Axis::PositiveY, Axis::NegativeZ),
            Err(LocalAxesError::UnknownAxis {
                axis: LocalAxis::Right
            })
        );
        assert_eq!(
            LocalAxes::try_new(Axis::PositiveX, Axis::NegativeX, Axis::NegativeZ),
            Err(LocalAxesError::CoincidentAxes {
                first: LocalAxis::Right,
                second: LocalAxis::Up
            })
        );
    }
}
