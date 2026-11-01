//! Allocation-free Baozi Triangle UV Derivative v1. This is not MikkTSpace.

#![forbid(unsafe_code)]

use std::cmp::Ordering;

const FRAME_EPSILON: f64 = 64.0 * f32::EPSILON as f64;
const OUTPUT_EPSILON: f64 = 8.0 * f32::EPSILON as f64;

/// Why a triangle cannot supply a finite, consistently oriented tangent frame.
///
/// Corner locations refer to the original global Corner domain, before the private
/// ordering used for determinant evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TangentIssue {
    /// The positions have exactly zero geometric area.
    DegenerateFace { face: usize },
    /// The selected UV coordinates have an exactly zero determinant.
    DegenerateUv { face: usize },
    /// The selected normal has zero length.
    ZeroNormal { corner: usize },
    /// The normal orientation or projected tangent is too close to zero.
    DegenerateFrame { corner: usize },
    /// The three selected normals would produce different handedness signs.
    MixedHandedness { face: usize },
    /// A position, selected normal, or selected UV component is not finite.
    NonFiniteInput { corner: usize },
    /// Calculation or final F32 rounding cannot supply a valid unit tangent.
    InvalidOutput { corner: usize },
}

/// Returns one dense F32x4 tangent for each original corner of a triangle.
///
/// The caller must check `face <= (usize::MAX - 2) / 3` before entry and must
/// charge work and poll control for this face. All loops, stack storage, and
/// arithmetic here have fixed bounds independent of the primitive's size.
/// There are no allocations, callbacks, recursion, welding, or cross-face sums.
///
/// Normals are normalized locally without changing the selected source values.
/// Tangent XYZ approximates the projected increasing-U derivative. W records the
/// analytic frame orientation, evaluated using the documented F64 arithmetic.
/// Reconstruction uses `B = W * cross(N, T)`; rounding can make its dot product
/// with the raw V derivative nonpositive for ill-conditioned UVs. W is not flipped
/// in response to that rounded dot product. Normal orientation and relative
/// projected length must exceed 64 F32 epsilons, even though computation uses F64.
/// All three corners must have the same W, exactly +1 or -1. Generated zero
/// components are canonical +0. Positions and UVs are interpreted as stored;
/// Scene handedness, winding declarations, and UV origin add no extra sign.
pub(super) fn triangle(
    face: usize,
    positions: [[f32; 3]; 3],
    normals: [[f32; 3]; 3],
    uvs: [[f32; 2]; 3],
) -> Result<[[f32; 4]; 3], TangentIssue> {
    let first_corner = face * 3;
    for (local, position) in positions.iter().enumerate() {
        if position
            .iter()
            .chain(&normals[local])
            .chain(&uvs[local])
            .any(|value| !value.is_finite())
        {
            return Err(TangentIssue::NonFiniteInput {
                corner: first_corner + local,
            });
        }
    }

    // Fix the six-product summation order under every permutation of the face.
    // Reordering both Position and UV preserves their derivative and handedness.
    // Equal positions already imply a degenerate face, so no other key is needed.
    let mut order = [0, 1, 2];
    for (left, right) in [(0, 1), (1, 2), (0, 1)] {
        if position_less(positions[order[right]], positions[order[left]]) {
            order.swap(left, right);
        }
    }
    let positions = order.map(|corner| positions[corner]);
    let uvs = order.map(|corner| uvs[corner]);

    let geometric = [(1, 2), (2, 0), (0, 1)].map(|(a, b)| {
        determinant(
            positions.map(|value| value[a]),
            positions.map(|value| value[b]),
        )
    });
    if geometric == [0.0; 3] {
        return Err(TangentIssue::DegenerateFace { face });
    }
    let geometric = unit(geometric).ok_or(TangentIssue::InvalidOutput {
        corner: first_corner,
    })?;

    let u = uvs.map(|value| value[0]);
    let v = uvs.map(|value| value[1]);
    let uv_determinant = determinant(u, v);
    if uv_determinant == 0.0 {
        return Err(TangentIssue::DegenerateUv { face });
    }
    if !uv_determinant.is_finite() {
        return Err(TangentIssue::InvalidOutput {
            corner: first_corner,
        });
    }
    let uv_sign = if uv_determinant > 0.0 { 1.0 } else { -1.0 };
    // Only direction is needed: positive scaling by |D| avoids UV division.
    let derivative =
        [0, 1, 2].map(|axis| uv_sign * determinant(positions.map(|value| value[axis]), v));
    let derivative = unit(derivative).ok_or(TangentIssue::InvalidOutput {
        corner: first_corner,
    })?;

    let mut output = [[0.0; 4]; 3];
    for (local, normal) in normals.into_iter().enumerate() {
        let corner = first_corner + local;
        let normal = unit(normal.map(f64::from)).ok_or(TangentIssue::ZeroNormal { corner })?;
        let alignment = dot(normal, geometric);
        if alignment.abs() <= FRAME_EPSILON {
            return Err(TangentIssue::DegenerateFrame { corner });
        }

        // cross(cross(N, A), N) is Gram-Schmidt projection for unit N. It
        // avoids subtracting nearly equal vectors when A almost follows N.
        let projected = cross(cross(normal, derivative), normal);
        if dot(projected, projected) <= FRAME_EPSILON * FRAME_EPSILON {
            return Err(TangentIssue::DegenerateFrame { corner });
        }
        let tangent = unit(projected).ok_or(TangentIssue::InvalidOutput { corner })?;
        let tangent = tangent.map(|value| {
            let rounded = value as f32;
            if rounded == 0.0 { 0.0 } else { rounded }
        });
        let rounded = tangent.map(f64::from);
        if tangent.iter().any(|value| !value.is_finite())
            || (dot(rounded, rounded) - 1.0).abs() > OUTPUT_EPSILON
            || dot(normal, rounded).abs() > OUTPUT_EPSILON
        {
            return Err(TangentIssue::InvalidOutput { corner });
        }

        // Analytically, cross(dP/du, dP/dv) = G / D. Record that orientation
        // without subtracting nearly parallel UV derivatives. This identity does
        // not promise positive dot(cross(N, rounded_T) * W, raw_dP/dv).
        let handedness = if alignment > 0.0 { uv_sign } else { -uv_sign };
        output[local] = [tangent[0], tangent[1], tangent[2], handedness as f32];
    }
    if output[0][3] != output[1][3] || output[0][3] != output[2][3] {
        return Err(TangentIssue::MixedHandedness { face });
    }
    Ok(output)
}

fn position_less(left: [f32; 3], right: [f32; 3]) -> bool {
    for (left, right) in left.into_iter().zip(right) {
        match left.total_cmp(&right) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
    }
    false
}

/// Evaluates (x1 - x0)(y2 - y0) - (y1 - y0)(x2 - x0).
fn determinant(x: [f32; 3], y: [f32; 3]) -> f64 {
    let x = x.map(f64::from);
    let y = y.map(f64::from);
    // A product of two finite F32 values is exact and in range in F64.
    // Expanding original coordinates avoids rounding away small edge differences.
    let products = [
        x[1] * y[2],
        -y[1] * x[2],
        x[2] * y[0],
        -y[2] * x[0],
        x[0] * y[1],
        -y[0] * x[1],
    ];
    // Reuse Baozi's normal-kernel TwoSum expansion, independently of Mikk/Assimp.
    // Six products need at most six entries and fifteen inner iterations in total.
    let mut expansion = [0.0; 6];
    let mut length = 0;
    for mut carry in products {
        let mut written = 0;
        for index in 0..length {
            let next = expansion[index];
            let sum = carry + next;
            let virtual_next = sum - carry;
            let residual = (carry - (sum - virtual_next)) + (next - virtual_next);
            if residual != 0.0 {
                expansion[written] = residual;
                written += 1;
            }
            carry = sum;
        }
        if carry != 0.0 || written == 0 {
            expansion[written] = carry;
            written += 1;
        }
        length = written;
    }
    let mut estimate = 0.0;
    for &term in &expansion[..length] {
        estimate += term;
    }
    estimate
}

fn unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    if vector.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let scale = vector[0].abs().max(vector[1].abs()).max(vector[2].abs());
    if scale == 0.0 {
        return None;
    }
    let scaled = vector.map(|value| value / scale);
    let length = dot(scaled, scaled).sqrt();
    Some(scaled.map(|value| value / length))
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    (left[0] * right[0] + left[1] * right[1]) + left[2] * right[2]
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

#[cfg(test)]
#[path = "kernel_tests.rs"]
mod tests;
