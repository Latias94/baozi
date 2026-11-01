//! Bounded binary64 geometry arithmetic. Inputs stay borrowed and unchanged.
//!
//! Requires round-to-nearest, gradual underflow and no reassociation. TwoSum:
//! <https://www.tuhh.de/ti3/paper/rump/OgRuOi05.pdf>, Theorem 3.4.
//! FMA residuals below carry their own error bounds, including underflow.

use std::fmt;

const ETA: f64 = f64::from_bits(1);
const DIRECTION_EPSILON: f64 = 32.0 * f64::EPSILON;
pub(crate) const UNIT_ERROR: f64 = 16.0 * f64::EPSILON;

/// Why binary64 geometry arithmetic cannot publish a certified result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeometryNumericIssue {
    NonFiniteInput,
    ScalingUnderflow,
    ScalingPrecisionLoss,
    NonFiniteIntermediate,
    ExactDegenerateFace,
    ExactDegenerateUv,
    UncertainDirection,
    UncertainUvDeterminant,
    UncertainFrame,
    ZeroVector,
    NormalizationUnderflow,
    InvalidOutput,
}

impl fmt::Display for GeometryNumericIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteInput => "geometry input is not finite",
            Self::ScalingUnderflow => "binary scaling lost a nonzero component",
            Self::ScalingPrecisionLoss => "binary scaling lost significand bits",
            Self::NonFiniteIntermediate => "geometry arithmetic or its error bound is not finite",
            Self::ExactDegenerateFace => "geometry has a witnessed zero area",
            Self::ExactDegenerateUv => "UV coordinates have a witnessed zero determinant",
            Self::UncertainDirection => "the direction exceeds its binary64 error budget",
            Self::UncertainUvDeterminant => "the UV determinant sign is uncertain",
            Self::UncertainFrame => "the tangent frame exceeds its binary64 error budget",
            Self::ZeroVector => "the vector is exactly zero",
            Self::NormalizationUnderflow => "normalization lost a nonzero component",
            Self::InvalidOutput => "the generated unit vector is not representable",
        })
    }
}

impl std::error::Error for GeometryNumericIssue {}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Pow2Scale {
    shift: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Estimate {
    pub(crate) value: f64,
    pub(crate) abs_error: f64,
}

impl Estimate {
    pub(crate) fn is_exact_zero(self) -> bool {
        self.value == 0.0 && self.abs_error == 0.0
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Estimate3 {
    pub(crate) value: [f64; 3],
    pub(crate) abs_error: [f64; 3],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Unit3 {
    pub(crate) value: [f64; 3],
    pub(crate) error_l2: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AreaAccumulator3 {
    sum: [f64; 3],
    correction: [f64; 3],
    abs_error: [f64; 3],
}

pub(crate) fn observe_magnitude(max_abs: &mut f64, value: f64) -> Result<(), GeometryNumericIssue> {
    if !value.is_finite() {
        return Err(GeometryNumericIssue::NonFiniteInput);
    }
    *max_abs = (*max_abs).max(value.abs());
    Ok(())
}

/// `max_abs` comes from `observe_magnitude`, starting at zero.
pub(crate) fn common_scale(max_abs: f64) -> Pow2Scale {
    if max_abs == 0.0 {
        return Pow2Scale { shift: 0 };
    }
    let bits = max_abs.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let exponent = if exponent == 0 {
        let fraction = bits & ((1_u64 << 52) - 1);
        63 - fraction.leading_zeros() as i32 - 1074
    } else {
        exponent - 1023
    };
    Pow2Scale {
        shift: -exponent - 1,
    }
}

/// Applies at most three normal power-of-two factors; never constructs 2^1074.
pub(crate) fn scale_exact(mut value: f64, scale: Pow2Scale) -> Result<f64, GeometryNumericIssue> {
    if !value.is_finite() {
        return Err(GeometryNumericIssue::NonFiniteInput);
    }
    let mut remaining = scale.shift;
    while remaining != 0 {
        let step = remaining.clamp(-512, 512);
        let factor = f64::from_bits(((step + 1023) as u64) << 52);
        let reverse = f64::from_bits(((-step + 1023) as u64) << 52);
        let next = value * factor;
        if !next.is_finite() {
            return Err(GeometryNumericIssue::NonFiniteIntermediate);
        }
        if value != 0.0 && next == 0.0 {
            return Err(GeometryNumericIssue::ScalingUnderflow);
        }
        if next * reverse != value {
            return Err(GeometryNumericIssue::ScalingPrecisionLoss);
        }
        value = next;
        remaining -= step;
    }
    Ok(value)
}

// Nonnegative bound arithmetic; zero identities avoid inventing uncertainty.
pub(crate) fn up_add(a: f64, b: f64) -> f64 {
    if a == 0.0 {
        b
    } else if b == 0.0 {
        a
    } else {
        (a + b).next_up()
    }
}

pub(crate) fn up_mul(a: f64, b: f64) -> f64 {
    if a == 0.0 || b == 0.0 {
        0.0
    } else {
        (a * b).next_up()
    }
}

fn up_div(a: f64, b: f64) -> f64 {
    if a == 0.0 { 0.0 } else { (a / b).next_up() }
}

fn roundoff(result: f64) -> f64 {
    up_add(up_mul(f64::EPSILON, result.abs()), ETA)
}

fn checked(value: f64, abs_error: f64) -> Result<Estimate, GeometryNumericIssue> {
    if !value.is_finite() || !abs_error.is_finite() || abs_error < 0.0 {
        return Err(GeometryNumericIssue::NonFiniteIntermediate);
    }
    Ok(Estimate { value, abs_error })
}

/// Error-free addition when the operation and its intermediates cannot overflow.
pub(crate) fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let virtual_b = sum - a;
    let residual = (a - (sum - virtual_b)) + (b - virtual_b);
    (sum, residual)
}

/// Kahan's difference of products, with bounds on all three residual roundings.
pub(crate) fn diff_products_f64(
    a: f64,
    b: f64,
    c: f64,
    d: f64,
) -> Result<Estimate, GeometryNumericIssue> {
    if [a, b, c, d].iter().any(|value| !value.is_finite()) {
        return Err(GeometryNumericIssue::NonFiniteInput);
    }
    let product = c * d;
    let error = (-c).mul_add(d, product);
    let difference = a.mul_add(b, -product);
    let value = difference + error;
    let bound = up_add(
        up_add(roundoff(error), roundoff(difference)),
        roundoff(value),
    );
    checked(value, bound)
}

/// Determinant of three scaled rows; edge subtraction residuals are bounded.
pub(crate) fn orient2_f64(x: [f64; 3], y: [f64; 3]) -> Result<Estimate, GeometryNumericIssue> {
    if x.iter().chain(&y).any(|value| !value.is_finite()) {
        return Err(GeometryNumericIssue::NonFiniteInput);
    }
    let duplicate = [(0, 1), (1, 2), (0, 2)]
        .into_iter()
        .any(|(a, b)| x[a] == x[b] && y[a] == y[b]);
    if duplicate
        || (x[0] == x[1] && x[1] == x[2])
        || (y[0] == y[1] && y[1] == y[2])
        || x == y
        || x == y.map(|value| -value)
    {
        return Ok(Estimate::default());
    }
    let (a, a_low) = two_sum(x[1], -x[0]);
    let (b, b_low) = two_sum(y[2], -y[0]);
    let (c, c_low) = two_sum(y[1], -y[0]);
    let (d, d_low) = two_sum(x[2], -x[0]);
    let mut estimate = diff_products_f64(a, b, c, d)?;
    for (left, right) in [
        (a, b_low),
        (a_low, b),
        (a_low, b_low),
        (c, d_low),
        (c_low, d),
        (c_low, d_low),
    ] {
        estimate.abs_error = up_add(estimate.abs_error, up_mul(left.abs(), right.abs()));
    }
    checked(estimate.value, estimate.abs_error)
}

/// Preserves the input winding; callers retain their existing identity ordering.
pub(crate) fn face_cross_f64(positions: [[f64; 3]; 3]) -> Result<Estimate3, GeometryNumericIssue> {
    let mut result = Estimate3::default();
    for (axis, (left, right)) in [(1, 2), (2, 0), (0, 1)].into_iter().enumerate() {
        let estimate = orient2_f64(
            positions.map(|row| row[left]),
            positions.map(|row| row[right]),
        )?;
        result.value[axis] = estimate.value;
        result.abs_error[axis] = estimate.abs_error;
    }
    if result.value == [0.0; 3] && result.abs_error == [0.0; 3] {
        return Err(GeometryNumericIssue::ExactDegenerateFace);
    }
    Ok(result)
}

pub(crate) fn certified_uv_sign(estimate: Estimate) -> Result<f64, GeometryNumericIssue> {
    checked(estimate.value, estimate.abs_error)?;
    if estimate.is_exact_zero() {
        return Err(GeometryNumericIssue::ExactDegenerateUv);
    }
    if estimate.value.abs() <= estimate.abs_error {
        return Err(GeometryNumericIssue::UncertainUvDeterminant);
    }
    Ok(if estimate.value > 0.0 { 1.0 } else { -1.0 })
}

/// Candidate state is disposable after an error, just like the kernel workspace.
pub(crate) fn area_add(
    acc: &mut AreaAccumulator3,
    face: Estimate3,
) -> Result<(), GeometryNumericIssue> {
    for axis in 0..3 {
        checked(face.value[axis], face.abs_error[axis])?;
        let (sum, residual) = two_sum(acc.sum[axis], face.value[axis]);
        if residual != 0.0 {
            let correction = acc.correction[axis] + residual;
            acc.abs_error[axis] = up_add(acc.abs_error[axis], roundoff(correction));
            acc.correction[axis] = correction;
        }
        acc.sum[axis] = sum;
        acc.abs_error[axis] = up_add(acc.abs_error[axis], face.abs_error[axis]);
        checked(acc.sum[axis], acc.abs_error[axis])?;
        checked(acc.correction[axis], 0.0)?;
    }
    Ok(())
}

pub(crate) fn area_finish(acc: &AreaAccumulator3) -> Result<Estimate3, GeometryNumericIssue> {
    let mut result = Estimate3::default();
    for axis in 0..3 {
        let (value, residual) = two_sum(acc.sum[axis], acc.correction[axis]);
        let estimate = checked(value, up_add(acc.abs_error[axis], residual.abs()))?;
        result.value[axis] = estimate.value;
        result.abs_error[axis] = estimate.abs_error;
    }
    Ok(result)
}

pub(crate) fn max_abs3(value: [f64; 3]) -> f64 {
    value[0].abs().max(value[1].abs()).max(value[2].abs())
}

pub(crate) fn dot_with_error(a: [f64; 3], b: [f64; 3]) -> Result<Estimate, GeometryNumericIssue> {
    if a.iter().chain(&b).any(|value| !value.is_finite()) {
        return Err(GeometryNumericIssue::NonFiniteInput);
    }
    let tail = a[2] * b[2];
    let middle = a[1].mul_add(b[1], tail);
    let value = a[0].mul_add(b[0], middle);
    checked(
        value,
        up_add(up_add(roundoff(tail), roundoff(middle)), roundoff(value)),
    )
}

pub(crate) fn cross_f64(a: [f64; 3], b: [f64; 3]) -> Result<Estimate3, GeometryNumericIssue> {
    let mut result = Estimate3::default();
    for (axis, (left, right)) in [(1, 2), (2, 0), (0, 1)].into_iter().enumerate() {
        let estimate = diff_products_f64(a[left], b[right], a[right], b[left])?;
        result.value[axis] = estimate.value;
        result.abs_error[axis] = estimate.abs_error;
    }
    Ok(result)
}

pub(crate) fn unit_f64(value: [f64; 3]) -> Result<[f64; 3], GeometryNumericIssue> {
    if value.iter().any(|component| !component.is_finite()) {
        return Err(GeometryNumericIssue::NonFiniteInput);
    }
    let magnitude = max_abs3(value);
    if magnitude == 0.0 {
        return Err(GeometryNumericIssue::ZeroVector);
    }
    let scaled = value.map(|component| component / magnitude);
    for axis in 0..3 {
        if value[axis] != 0.0 && scaled[axis] == 0.0 {
            return Err(GeometryNumericIssue::NormalizationUnderflow);
        }
    }
    let length = dot_with_error(scaled, scaled)?.value.sqrt();
    let unit = scaled.map(|component| component / length);
    for axis in 0..3 {
        if scaled[axis] != 0.0 && unit[axis] == 0.0 {
            return Err(GeometryNumericIssue::NormalizationUnderflow);
        }
    }
    if unit.iter().any(|component| !component.is_finite())
        || (dot_with_error(unit, unit)?.value - 1.0).abs() > 8.0 * f64::EPSILON
    {
        return Err(GeometryNumericIssue::InvalidOutput);
    }
    Ok(unit)
}

pub(crate) fn unit_estimate(value: Estimate3) -> Result<Unit3, GeometryNumericIssue> {
    for axis in 0..3 {
        checked(value.value[axis], value.abs_error[axis])?;
    }
    let magnitude = max_abs3(value.value);
    let error = max_abs3(value.abs_error);
    if magnitude == 0.0 {
        return Err(if error == 0.0 {
            GeometryNumericIssue::ZeroVector
        } else {
            GeometryNumericIssue::UncertainDirection
        });
    }
    let relative = up_div(error, magnitude);
    if relative > DIRECTION_EPSILON {
        return Err(GeometryNumericIssue::UncertainDirection);
    }
    Ok(Unit3 {
        value: unit_f64(value.value)?,
        error_l2: up_add(up_mul(4.0, relative), UNIT_ERROR),
    })
}

#[cfg(test)]
#[path = "precision/tests.rs"]
mod tests;
