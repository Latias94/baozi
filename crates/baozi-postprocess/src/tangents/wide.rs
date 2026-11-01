//! Bounded F64 Triangle UV Derivative frames, without source mutation or allocation.

use std::cmp::Ordering;

use crate::precision::{
    self, Estimate3, GeometryNumericIssue, UNIT_ERROR, scale_exact, up_add, up_mul,
};

use super::kernel::TangentIssue;

#[cfg(test)]
use crate::precision::{common_scale, observe_magnitude};

const FRAME_EPSILON: f64 = 64.0 * f64::EPSILON;
const OUTPUT_EPSILON: f64 = 8.0 * f64::EPSILON;

/// Source declarations select each determinant independently of output width.
#[derive(Clone, Copy)]
pub(super) struct Sources {
    pub(super) position_f64: bool,
    pub(super) uv_f64: bool,
    pub(super) position_scale: precision::Pow2Scale,
    pub(super) uv_scale: precision::Pow2Scale,
}

#[cfg(test)]
fn triangle(
    face: usize,
    positions: [[f64; 3]; 3],
    normals: [[f64; 3]; 3],
    uvs: [[f64; 2]; 3],
) -> Result<[[f64; 4]; 3], TangentIssue> {
    let mut position_max = 0.0;
    let mut uv_max = 0.0;
    for local in 0..3 {
        for value in positions[local] {
            observe_magnitude(&mut position_max, value)
                .map_err(|issue| numeric(face, face * 3 + local, issue))?;
        }
        for value in uvs[local] {
            observe_magnitude(&mut uv_max, value)
                .map_err(|issue| numeric(face, face * 3 + local, issue))?;
        }
    }
    triangle_with_sources(
        face,
        positions,
        normals,
        uvs,
        Sources {
            position_f64: true,
            uv_f64: true,
            position_scale: common_scale(position_max),
            uv_scale: common_scale(uv_max),
        },
    )
}

/// The adapter admits one common Position/UV scale over the entire retained primitive.
/// Three-row arithmetic remains fixed-size and does not allocate.
pub(super) fn triangle_with_sources(
    face: usize,
    positions: [[f64; 3]; 3],
    normals: [[f64; 3]; 3],
    uvs: [[f64; 2]; 3],
    sources: Sources,
) -> Result<[[f64; 4]; 3], TangentIssue> {
    let first_corner = face
        .checked_mul(3)
        .filter(|first| *first <= usize::MAX - 2)
        .ok_or(TangentIssue::InvalidOutput { corner: usize::MAX })?;
    let original_positions = positions;
    let original_uvs = uvs;
    let mut positions = positions;
    let mut uvs = uvs;
    for local in 0..3 {
        let corner = first_corner + local;
        for component in &mut positions[local] {
            *component = scale_exact(*component, sources.position_scale)
                .map_err(|issue| numeric(face, corner, issue))?;
        }
        for component in &mut uvs[local] {
            *component = scale_exact(*component, sources.uv_scale)
                .map_err(|issue| numeric(face, corner, issue))?;
        }
        if normals[local].iter().any(|value| !value.is_finite()) {
            return Err(TangentIssue::NonFiniteInput { corner });
        }
    }

    // Fix the determinant operation order while normals retain original corners.
    let mut order = [0, 1, 2];
    for (left, right) in [(0, 1), (1, 2), (0, 1)] {
        if position_less(positions[order[right]], positions[order[left]]) {
            order.swap(left, right);
        }
    }
    let positions = order.map(|corner| positions[corner]);
    let uvs = order.map(|corner| uvs[corner]);
    let original_positions = order.map(|corner| original_positions[corner]);
    let original_uvs = order.map(|corner| original_uvs[corner]);
    let geometric = if sources.position_f64 {
        precision::face_cross_f64(positions)
    } else {
        let mut cross = Estimate3::default();
        for (axis, (a, b)) in [(1, 2), (2, 0), (0, 1)].into_iter().enumerate() {
            let estimate = exact_f32_minor(
                original_positions.map(|p| p[a]),
                original_positions.map(|p| p[b]),
            );
            cross.value[axis] = estimate.value;
            cross.abs_error[axis] = estimate.abs_error;
        }
        if cross.value == [0.0; 3] {
            Err(GeometryNumericIssue::ExactDegenerateFace)
        } else {
            Ok(cross)
        }
    }
    .and_then(precision::unit_estimate)
    .map_err(|issue| numeric(face, first_corner, issue))?;
    let v = uvs.map(|row| row[1]);
    let uv_sign = if sources.uv_f64 {
        precision::orient2_f64(uvs.map(|row| row[0]), v)
    } else {
        Ok(exact_f32_minor(
            original_uvs.map(|p| p[0]),
            original_uvs.map(|p| p[1]),
        ))
    }
    .and_then(precision::certified_uv_sign)
    .map_err(|issue| numeric(face, first_corner, issue))?;
    let mut derivative = Estimate3::default();
    for axis in 0..3 {
        let estimate = if sources.position_f64 || sources.uv_f64 {
            precision::orient2_f64(positions.map(|row| row[axis]), v)
                .map_err(|issue| numeric(face, first_corner, issue))?
        } else {
            exact_f32_minor(
                original_positions.map(|p| p[axis]),
                original_uvs.map(|p| p[1]),
            )
        };
        derivative.value[axis] = uv_sign * estimate.value;
        derivative.abs_error[axis] = estimate.abs_error;
    }
    let derivative =
        precision::unit_estimate(derivative).map_err(|issue| numeric(face, first_corner, issue))?;

    let mut output = [[0.0; 4]; 3];
    for (local, source_normal) in normals.into_iter().enumerate() {
        let corner = first_corner + local;
        let normal =
            precision::unit_f64(source_normal).map_err(|issue| numeric(face, corner, issue))?;
        let alignment = precision::dot_with_error(normal, geometric.value)
            .map_err(|issue| numeric(face, corner, issue))?;
        let alignment_error = up_add(
            alignment.abs_error,
            up_add(
                up_mul(2.0, up_add(UNIT_ERROR, geometric.error_l2)),
                up_mul(UNIT_ERROR, geometric.error_l2),
            ),
        );
        if alignment.value.abs() <= up_add(FRAME_EPSILON, alignment_error) {
            return Err(numeric(face, corner, GeometryNumericIssue::UncertainFrame));
        }

        let first_cross = precision::cross_f64(normal, derivative.value)
            .map_err(|issue| numeric(face, corner, issue))?;
        let projection = precision::cross_f64(first_cross.value, normal)
            .map_err(|issue| numeric(face, corner, issue))?;
        // Each arithmetic cross has L2 error <=64 eps for these norm-bounded inputs.
        let projection_error = up_add(
            up_add(up_mul(5.0, UNIT_ERROR), up_mul(2.0, derivative.error_l2)),
            192.0 * f64::EPSILON,
        );
        if precision::max_abs3(projection.value) <= up_add(FRAME_EPSILON, projection_error) {
            return Err(numeric(face, corner, GeometryNumericIssue::UncertainFrame));
        }
        let tangent =
            precision::unit_f64(projection.value).map_err(|issue| numeric(face, corner, issue))?;
        let length = precision::dot_with_error(tangent, tangent)
            .map_err(|issue| numeric(face, corner, issue))?
            .value;
        let orthogonality = precision::dot_with_error(normal, tangent)
            .map_err(|issue| numeric(face, corner, issue))?
            .value;
        if (length - 1.0).abs() > OUTPUT_EPSILON || orthogonality.abs() > OUTPUT_EPSILON {
            return Err(numeric(face, corner, GeometryNumericIssue::InvalidOutput));
        }
        let tangent = tangent.map(|value| if value == 0.0 { 0.0 } else { value });
        let handedness = if alignment.value > 0.0 {
            uv_sign
        } else {
            -uv_sign
        };
        output[local] = [tangent[0], tangent[1], tangent[2], handedness];
    }
    if output[0][3] != output[1][3] || output[0][3] != output[2][3] {
        return Err(TangentIssue::MixedHandedness { face });
    }
    Ok(output)
}

/// Both original operand rows have declared F32 storage. Products and expansion residuals are
/// exact and safely within binary64 range. The at-most-six-term final expansion estimate has
/// a conservative eight-epsilon relative rounding bound; a zero expansion is an exact witness.
fn exact_f32_minor(x: [f64; 3], y: [f64; 3]) -> precision::Estimate {
    let value = super::kernel::determinant(x.map(|v| v as f32), y.map(|v| v as f32));
    precision::Estimate {
        value,
        abs_error: if value == 0.0 {
            0.0
        } else {
            up_mul(8.0 * f64::EPSILON, value.abs())
        },
    }
}

fn numeric(face: usize, corner: usize, issue: GeometryNumericIssue) -> TangentIssue {
    match issue {
        GeometryNumericIssue::NonFiniteInput => TangentIssue::NonFiniteInput { corner },
        GeometryNumericIssue::ExactDegenerateFace => TangentIssue::DegenerateFace { face },
        GeometryNumericIssue::ExactDegenerateUv => TangentIssue::DegenerateUv { face },
        GeometryNumericIssue::ZeroVector => TangentIssue::ZeroNormal { corner },
        issue => TangentIssue::Numerical { corner, issue },
    }
}

fn position_less(left: [f64; 3], right: [f64; 3]) -> bool {
    for (left, right) in left.into_iter().zip(right) {
        match left.total_cmp(&right) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLANE: [[f64; 3]; 3] = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    const NORMALS: [[f64; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    const UVS: [[f64; 2]; 3] = [[0.0; 2], [1.0, 0.0], [0.0, 1.0]];

    fn assert_frame(output: [[f64; 4]; 3], tangent: [f64; 3], handedness: f64) {
        for frame in output {
            for axis in 0..3 {
                assert!((frame[axis] - tangent[axis]).abs() <= OUTPUT_EPSILON);
            }
            assert_eq!(frame[3], handedness);
        }
    }

    #[test]
    fn ordinary_frame_and_mirrored_uv_keep_analytic_orientation() {
        assert_frame(
            triangle(0, PLANE, NORMALS, UVS).unwrap(),
            [1.0, 0.0, 0.0],
            1.0,
        );
        let mirrored = [[0.0, 0.0], [1.0, 0.0], [0.0, -1.0]];
        assert_frame(
            triangle(0, PLANE, NORMALS, mirrored).unwrap(),
            [1.0, 0.0, 0.0],
            -1.0,
        );
    }

    #[test]
    fn uniform_subnormal_and_huge_positions_have_the_same_frame() {
        for scale in [f64::from_bits(1), 2.0_f64.powi(-1000), 2.0_f64.powi(1000)] {
            let positions = PLANE.map(|row| row.map(|value| value * scale));
            assert_frame(
                triangle(0, positions, NORMALS, UVS).unwrap(),
                [1.0, 0.0, 0.0],
                1.0,
            );
            let uvs = UVS.map(|row| row.map(|value| value * scale));
            assert_frame(
                triangle(0, PLANE, NORMALS, uvs).unwrap(),
                [1.0, 0.0, 0.0],
                1.0,
            );
        }
    }

    #[test]
    fn tilted_plane_has_the_analytic_increasing_u_derivative() {
        let positions = [[0.0; 3], [1.0, 0.0, 2.0], [0.0, 1.0, -3.0]];
        let normal = [-2.0, 3.0, 1.0].map(|component| component / 14.0_f64.sqrt());
        let tangent = [1.0, 0.0, 2.0].map(|component| component / 5.0_f64.sqrt());
        assert_frame(
            triangle(0, positions, [normal; 3], UVS).unwrap(),
            tangent,
            1.0,
        );
    }

    #[test]
    fn adjacent_huge_coordinates_and_all_permutations_preserve_frame() {
        let base = 2.0_f64.powi(1000);
        let step = 2.0_f64.powi(948);
        let positions = [
            [base, base, base],
            [base + step, base, base],
            [base, base + step, base],
        ];
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            assert_frame(
                triangle(
                    0,
                    order.map(|i| positions[i]),
                    NORMALS,
                    order.map(|i| UVS[i]),
                )
                .unwrap(),
                [1.0, 0.0, 0.0],
                1.0,
            );
        }
    }

    #[test]
    fn uv_cancellation_retains_the_negative_determinant_sign() {
        let h = f64::EPSILON;
        let uvs = [[0.0, 0.0], [1.0 + h, 1.0], [1.0, 1.0 - h]];
        let length = ((1.0 - h) * (1.0 - h) + 1.0).sqrt();
        assert_frame(
            triangle(0, PLANE, NORMALS, uvs).unwrap(),
            [-(1.0 - h) / length, 1.0 / length, 0.0],
            -1.0,
        );
    }

    #[test]
    fn exact_zero_and_uncertain_geometry_have_distinct_issues() {
        let mut duplicate = PLANE;
        duplicate[2] = duplicate[1];
        assert_eq!(
            triangle(2, duplicate, NORMALS, UVS),
            Err(TangentIssue::DegenerateFace { face: 2 })
        );
        assert_eq!(
            triangle(2, PLANE, NORMALS, [[0.0; 2]; 3]),
            Err(TangentIssue::DegenerateUv { face: 2 })
        );
        let t = 2.0_f64.powi(-54);
        let d = 2.0_f64.powi(-51);
        let near_line = [[t, t, 0.0], [1.0, 1.0, 0.0], [2.0, 2.0 + d, 0.0]];
        assert!(matches!(
            triangle(2, near_line, NORMALS, UVS),
            Err(TangentIssue::Numerical {
                corner: 6,
                issue: GeometryNumericIssue::UncertainDirection
            })
        ));
    }

    #[test]
    fn corner_diagnostics_keep_original_normal_ownership() {
        let mut normals = NORMALS;
        normals[1] = [1.0, 0.0, 0.0];
        assert_eq!(
            triangle(2, PLANE, normals, UVS),
            Err(TangentIssue::Numerical {
                corner: 7,
                issue: GeometryNumericIssue::UncertainFrame,
            }),
        );
        normals[1] = [0.0; 3];
        assert_eq!(
            triangle(2, PLANE, normals, UVS),
            Err(TangentIssue::ZeroNormal { corner: 7 })
        );
        normals[1] = [0.0, 0.0, -1.0];
        assert_eq!(
            triangle(2, PLANE, normals, UVS),
            Err(TangentIssue::MixedHandedness { face: 2 })
        );
    }

    #[test]
    fn promoted_output_rejects_an_uncertain_projected_direction() {
        let positions = [
            [2.0_f64.powi(-43), 0.0, 0.0],
            [1.0, 2.0, 3.0],
            [0.0, 1.0, 0.0],
        ];
        let normals = [[1.0, 2.0, 3.0]; 3];
        let sources = Sources {
            position_f64: false,
            uv_f64: false,
            position_scale: common_scale(3.0),
            uv_scale: common_scale(1.0),
        };
        assert!(matches!(
            triangle_with_sources(0, positions, normals, UVS, sources),
            Err(TangentIssue::Numerical {
                issue: GeometryNumericIssue::UncertainFrame,
                ..
            })
        ));
    }
}
