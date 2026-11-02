//! Allocation-free TriangleList normals. This sidecar is not a Scene operation.

use baozi_core::{AttributeData, AttributeRef};

use crate::GeometryNumericIssue;
use crate::precision::{self, AreaAccumulator3, Estimate3, Pow2Scale};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Flat,
    /// Smooth by vertex identity and Face group, weighted by triangle area.
    Smooth,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum VertexIds<'a> {
    Indexed(&'a [u32]),
    /// Every position is a distinct corner/vertex, even at coincident coordinates.
    NonIndexed,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Input<'a, P = &'a [[f32; 3]]> {
    pub positions: P,
    pub vertex_ids: VertexIds<'a>,
    /// One value per face. None smooths all shared identities; zero is hard per face.
    pub smoothing_groups: Option<&'a [u32]>,
}

impl<P: PositionSource> Input<'_, P> {
    fn corner_count(self) -> usize {
        match self.vertex_ids {
            VertexIds::Indexed(ids) => ids.len(),
            VertexIds::NonIndexed => self.positions.len(),
        }
    }

    fn vertex(self, corner: usize) -> usize {
        match self.vertex_ids {
            VertexIds::Indexed(ids) => ids[corner] as usize,
            VertexIds::NonIndexed => corner,
        }
    }
}

/// Only the normal kernel's logical Position read seam; arithmetic dispatch follows storage.
pub(crate) trait PositionSource: Copy {
    fn len(self) -> usize;
    fn is_f64(self) -> bool;
    fn read<E>(
        self,
        vertex: usize,
        control: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<[f64; 3], Error<E>>;
}

#[cfg(test)]
impl PositionSource for &[[f32; 3]] {
    fn len(self) -> usize {
        <[[f32; 3]]>::len(self)
    }

    fn is_f64(self) -> bool {
        false
    }

    fn read<E>(
        self,
        vertex: usize,
        _: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<[f64; 3], Error<E>> {
        self.get(vertex)
            .copied()
            .map(|value| value.map(f64::from))
            .ok_or(Error::InvalidPositionStorage { vertex })
    }
}

#[cfg(test)]
impl PositionSource for &[[f64; 3]] {
    fn len(self) -> usize {
        <[[f64; 3]]>::len(self)
    }

    fn is_f64(self) -> bool {
        true
    }

    fn read<E>(
        self,
        vertex: usize,
        _: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<[f64; 3], Error<E>> {
        self.get(vertex)
            .copied()
            .ok_or(Error::InvalidPositionStorage { vertex })
    }
}

impl PositionSource for AttributeRef<'_> {
    fn len(self) -> usize {
        AttributeRef::len(self)
    }

    fn is_f64(self) -> bool {
        matches!(
            self.data(),
            AttributeData::DenseF64 { .. } | AttributeData::SparseF64 { .. }
        )
    }

    fn read<E>(
        self,
        vertex: usize,
        control: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<[f64; 3], Error<E>> {
        let data = self.data();
        let index = match data {
            AttributeData::SparseF32 { indices, .. } | AttributeData::SparseF64 { indices, .. } => {
                sparse_index(indices, vertex, control)?
            }
            _ => Some(vertex),
        };
        control().map_err(Error::Control)?;
        match (data, index) {
            (AttributeData::DenseF32 { lanes: 3, values }, Some(index))
            | (
                AttributeData::SparseF32 {
                    lanes: 3, values, ..
                },
                Some(index),
            ) => position_tuple(values, index, vertex),
            (AttributeData::DenseF64 { lanes: 3, values }, Some(index))
            | (
                AttributeData::SparseF64 {
                    lanes: 3, values, ..
                },
                Some(index),
            ) => position_tuple(values, index, vertex),
            (
                AttributeData::SparseF32 {
                    lanes: 3, default, ..
                },
                None,
            ) => position_tuple(default, 0, vertex),
            (
                AttributeData::SparseF64 {
                    lanes: 3, default, ..
                },
                None,
            ) => position_tuple(default, 0, vertex),
            _ => Err(Error::InvalidPositionStorage { vertex }),
        }
    }
}

fn position_tuple<T: Copy + Into<f64>, E>(
    values: &[T],
    index: usize,
    vertex: usize,
) -> Result<[f64; 3], Error<E>> {
    let tuple = index
        .checked_mul(3)
        .and_then(|start| values.get(start..))
        .and_then(|tail| tail.get(..3))
        .ok_or(Error::InvalidPositionStorage { vertex })?;
    Ok([tuple[0].into(), tuple[1].into(), tuple[2].into()])
}

fn sparse_index<E>(
    indices: &[u32],
    vertex: usize,
    control: &mut impl FnMut() -> Result<(), E>,
) -> Result<Option<usize>, Error<E>> {
    let logical = u32::try_from(vertex).map_err(|_| Error::InvalidPositionStorage { vertex })?;
    let (mut low, mut high) = (0, indices.len());
    // Validated sparse indices have a u32 logical domain. Each comparison is separately charged.
    for _ in 0..33 {
        if low == high {
            return Ok(None);
        }
        let middle = low + (high - low) / 2;
        control().map_err(Error::Control)?;
        match indices[middle].cmp(&logical) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Ok(Some(middle)),
        }
    }
    if low == high {
        Ok(None)
    } else {
        Err(Error::InvalidPositionStorage { vertex })
    }
}

pub(crate) enum Output<'a> {
    F32(&'a mut [[f32; 3]]),
    F64(&'a mut [[f64; 3]]),
}

impl Output<'_> {
    fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    fn publish(&mut self, corner: usize, normal: [f64; 3]) -> Result<(), GeometryNumericIssue> {
        let (rounded, epsilon) = match self {
            Self::F32(values) => {
                let normal = normal.map(|value| {
                    let value = value as f32;
                    if value == 0.0 { 0.0 } else { value }
                });
                values[corner] = normal;
                (normal.map(f64::from), f64::from(f32::EPSILON))
            }
            Self::F64(values) => {
                let normal = normal.map(|value| if value == 0.0 { 0.0 } else { value });
                values[corner] = normal;
                (normal, f64::EPSILON)
            }
        };
        let length_squared = rounded[0].mul_add(
            rounded[0],
            rounded[1].mul_add(rounded[1], rounded[2] * rounded[2]),
        );
        if rounded.iter().any(|value| !value.is_finite())
            || (length_squared - 1.0).abs() > 8.0 * epsilon
        {
            return Err(GeometryNumericIssue::InvalidOutput);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Workspace<'a> {
    /// Exactly C entries for Smooth; empty for Flat. Initial contents are ignored.
    pub corner_order: &'a mut [usize],
    /// Optional cache of exactly C / 3 unnormalized face crosses and their error bounds.
    /// Initial contents are ignored. Without it, Smooth recomputes each corner's cross.
    pub face_crosses: Option<&'a mut [Estimate3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Buffer {
    Output,
    CornerOrder,
    FaceCrosses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Error<E> {
    Control(E),
    CornerCount {
        actual: usize,
    },
    GroupCount {
        expected: usize,
        actual: usize,
    },
    BufferLength {
        buffer: Buffer,
        expected: usize,
        actual: usize,
    },
    NonFinitePosition {
        vertex: usize,
        component: usize,
    },
    InvalidPositionStorage {
        vertex: usize,
    },
    VertexOutOfBounds {
        corner: usize,
        vertex: usize,
        vertex_count: usize,
    },
    DegenerateFace {
        face: usize,
    },
    InvalidFaceCross {
        face: usize,
    },
    InvalidSmoothSum {
        vertex: usize,
        /// None distinguishes absent groups from an explicit group.
        group: Option<u32>,
        /// Lowest original corner index in the failing smoothing bucket.
        corner: usize,
    },
    PositionNumerical {
        vertex: usize,
        component: usize,
        issue: GeometryNumericIssue,
    },
    Numerical {
        face: usize,
        issue: GeometryNumericIssue,
    },
    SmoothNumerical {
        vertex: usize,
        group: Option<u32>,
        corner: usize,
        issue: GeometryNumericIssue,
    },
}

/// Writes one finite unit normal per original corner, within f32 rounding.
///
/// All position rows, including unused indexed vertices, must be finite. No coordinate welding
/// or UV inspection occurs. Only vertex identity and optional Face groups determine smoothing.
/// The input slices are never modified. Output and workspace are provisional on any error.
///
/// Work is O(V + C log C) for Smooth and O(V + C) for Flat. Smooth uses C caller-owned usize
/// entries, an optional C / 3 cross cache, and constant stack space; Flat needs no corner scratch.
/// Every input-sized loop polls control, including heap construction, sift-down, and output.
/// Polling also occurs at entry and completion. The callback's error is returned unchanged.
#[cfg(test)]
pub(crate) fn generate<E>(
    input: Input<'_>,
    mode: Mode,
    output: &mut [[f32; 3]],
    workspace: Workspace<'_>,
    control: impl FnMut() -> Result<(), E>,
) -> Result<(), Error<E>> {
    generate_into(input, mode, Output::F32(output), workspace, control)
}

/// Reads only selected logical Position rows; source width chooses determinant arithmetic.
pub(crate) fn generate_into<E, P: PositionSource>(
    input: Input<'_, P>,
    mode: Mode,
    mut output: Output<'_>,
    mut workspace: Workspace<'_>,
    mut control: impl FnMut() -> Result<(), E>,
) -> Result<(), Error<E>> {
    control().map_err(Error::Control)?;
    let corners = input.corner_count();
    if !corners.is_multiple_of(3) {
        return Err(Error::CornerCount { actual: corners });
    }
    let faces = corners / 3;
    if let Some(groups) = input.smoothing_groups
        && groups.len() != faces
    {
        return Err(Error::GroupCount {
            expected: faces,
            actual: groups.len(),
        });
    }
    check_length(Buffer::Output, corners, output.len())?;
    check_length(
        Buffer::CornerOrder,
        if mode == Mode::Smooth { corners } else { 0 },
        workspace.corner_order.len(),
    )?;
    if let Some(crosses) = workspace.face_crosses.as_deref() {
        check_length(Buffer::FaceCrosses, faces, crosses.len())?;
    }

    let wide = input.positions.is_f64();
    let mut maximum = 0.0_f64;
    for vertex in 0..input.positions.len() {
        control().map_err(Error::Control)?;
        let position = input.positions.read(vertex, &mut control)?;
        for (component, value) in position.into_iter().enumerate() {
            if !value.is_finite() {
                return Err(Error::NonFinitePosition { vertex, component });
            }
            if wide {
                precision::observe_magnitude(&mut maximum, value).map_err(|issue| {
                    Error::PositionNumerical {
                        vertex,
                        component,
                        issue,
                    }
                })?;
            }
        }
    }
    // Every face and smoothing bucket uses this single scale, retaining relative areas.
    let scale = wide.then(|| precision::common_scale(maximum));

    for face in 0..faces {
        control().map_err(Error::Control)?;
        let cross = read_face_cross(input, face, scale, &mut control)?;
        let wide_normal = if wide {
            Some(
                precision::unit_estimate(cross)
                    .map_err(|issue| Error::Numerical { face, issue })?
                    .value,
            )
        } else {
            None
        };
        if let Some(crosses) = workspace.face_crosses.as_deref_mut() {
            crosses[face] = cross;
        }
        if mode == Mode::Flat {
            let normal = match wide_normal {
                Some(normal) => normal,
                None => unit_exact_source(cross.value).ok_or(Error::InvalidFaceCross { face })?,
            };
            for corner in 3 * face..3 * face + 3 {
                output
                    .publish(corner, normal)
                    .map_err(|issue| Error::Numerical { face, issue })?;
            }
        }
    }

    if mode == Mode::Smooth {
        for (corner, slot) in workspace.corner_order.iter_mut().enumerate() {
            control().map_err(Error::Control)?;
            *slot = corner;
        }
        heapsort(input, workspace.corner_order, &mut control)?;
        let mut start = 0;
        while start < corners {
            control().map_err(Error::Control)?;
            let first = workspace.corner_order[start];
            let key = smoothing_key(input, first);
            let mut end = start;
            let mut sum = [0.0_f64; 3];
            let mut correction = [0.0_f64; 3];
            let mut area = AreaAccumulator3::default();
            let numeric_error = |issue| Error::SmoothNumerical {
                vertex: key.0,
                group: input.smoothing_groups.map(|groups| groups[first / 3]),
                corner: first,
                issue,
            };
            while end < corners {
                control().map_err(Error::Control)?;
                let corner = workspace.corner_order[end];
                if smoothing_key(input, corner) != key {
                    break;
                }
                let cross = match workspace.face_crosses.as_deref() {
                    Some(crosses) => crosses[corner / 3],
                    None => read_face_cross(input, corner / 3, scale, &mut control)?,
                };
                if wide {
                    precision::area_add(&mut area, cross).map_err(numeric_error)?;
                } else {
                    accumulate(&mut sum, &mut correction, cross.value);
                }
                end += 1;
            }
            let normal = if wide {
                let estimate = precision::area_finish(&area).map_err(numeric_error)?;
                precision::unit_estimate(estimate)
                    .map_err(|issue| {
                        numeric_error(match issue {
                            GeometryNumericIssue::ZeroVector => {
                                GeometryNumericIssue::UncertainDirection
                            }
                            issue => issue,
                        })
                    })?
                    .value
            } else {
                let sum = [
                    sum[0] + correction[0],
                    sum[1] + correction[1],
                    sum[2] + correction[2],
                ];
                unit_exact_source(sum).ok_or(Error::InvalidSmoothSum {
                    vertex: key.0,
                    group: input.smoothing_groups.map(|groups| groups[first / 3]),
                    corner: first,
                })?
            };
            for &corner in &workspace.corner_order[start..end] {
                control().map_err(Error::Control)?;
                output.publish(corner, normal).map_err(numeric_error)?;
            }
            start = end;
        }
    }
    control().map_err(Error::Control)
}

fn check_length<E>(buffer: Buffer, expected: usize, actual: usize) -> Result<(), Error<E>> {
    if actual != expected {
        return Err(Error::BufferLength {
            buffer,
            expected,
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
fn face_cross<E>(input: Input<'_>, face: usize) -> Result<[f64; 3], Error<E>> {
    read_face_cross(input, face, None, &mut || Ok(())).map(|cross| cross.value)
}

fn read_face_cross<E, P: PositionSource>(
    input: Input<'_, P>,
    face: usize,
    scale: Option<Pow2Scale>,
    control: &mut impl FnMut() -> Result<(), E>,
) -> Result<Estimate3, Error<E>> {
    let first = 3 * face;
    let mut vertices = [
        input.vertex(first),
        input.vertex(first + 1),
        input.vertex(first + 2),
    ];
    for (offset, &vertex) in vertices.iter().enumerate() {
        if vertex >= input.positions.len() {
            return Err(Error::VertexOutOfBounds {
                corner: first + offset,
                vertex,
                vertex_count: input.positions.len(),
            });
        }
    }
    // One exact determinant can have different floating expansion estimates under permutation.
    // Fix the product order by Vertex identity, then restore winding with an exact sign change.
    // Opposite faces must cancel bit-for-bit instead of normalizing a rounding residual.
    let mut reversed = false;
    for (left, right) in [(0, 1), (1, 2), (0, 1)] {
        if vertices[left] > vertices[right] {
            vertices.swap(left, right);
            reversed = !reversed;
        }
    }
    let mut positions = [[0.0; 3]; 3];
    for (row, vertex) in positions.iter_mut().zip(vertices) {
        *row = input.positions.read(vertex, control)?;
        if let Some(scale) = scale {
            for (component, value) in row.iter_mut().enumerate() {
                *value = precision::scale_exact(*value, scale).map_err(|issue| {
                    Error::PositionNumerical {
                        vertex,
                        component,
                        issue,
                    }
                })?;
            }
        }
    }
    let mut cross = if scale.is_some() {
        precision::face_cross_f64(positions).map_err(|issue| Error::Numerical { face, issue })?
    } else {
        // Two finite F32 coordinate factors multiply exactly in F64 at every exponent.
        // Preserve the original six-product expansion, including its operation order.
        let [a, b, c] = positions;
        let value = [
            determinant(a, b, c, 1, 2),
            determinant(a, b, c, 2, 0),
            determinant(a, b, c, 0, 1),
        ];
        if value.iter().any(|value| !value.is_finite()) {
            return Err(Error::InvalidFaceCross { face });
        }
        if value == [0.0; 3] {
            return Err(Error::DegenerateFace { face });
        }
        Estimate3 {
            value,
            abs_error: [0.0; 3],
        }
    };
    if reversed {
        cross.value = cross.value.map(|value| -value);
    }
    Ok(cross)
}

fn determinant(a: [f64; 3], b: [f64; 3], c: [f64; 3], i: usize, j: usize) -> f64 {
    let products = [
        b[i] * c[j],
        -b[j] * c[i],
        c[i] * a[j],
        -c[j] * a[i],
        a[i] * b[j],
        -a[j] * b[i],
    ];
    // Grow a nonoverlapping expansion using TwoSum, eliminating zero residuals. Six exact
    // products need at most six stack entries; no input-sized scratch or approximation cutoff.
    // Algorithm: https://www.cs.cmu.edu/~quake/robust.html (grow_expansion_zeroelim).
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
    expansion[..length].iter().sum()
}

#[cfg(test)]
fn unit(vector: [f64; 3]) -> Option<[f32; 3]> {
    unit_exact_source(vector).map(|normal| normal.map(|value| value as f32))
}

fn unit_exact_source(vector: [f64; 3]) -> Option<[f64; 3]> {
    if vector.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let scale = vector[0].abs().max(vector[1].abs()).max(vector[2].abs());
    if scale == 0.0 {
        return None;
    }
    let scaled = vector.map(|value| value / scale);
    let length = (scaled[0] * scaled[0] + scaled[1] * scaled[1] + scaled[2] * scaled[2]).sqrt();
    Some(scaled.map(|value| value / length))
}

fn accumulate(sum: &mut [f64; 3], correction: &mut [f64; 3], cross: [f64; 3]) {
    // Neumaier summation retains small faces between large, opposing contributions.
    for axis in 0..3 {
        let next = sum[axis] + cross[axis];
        correction[axis] += if sum[axis].abs() >= cross[axis].abs() {
            (sum[axis] - next) + cross[axis]
        } else {
            (cross[axis] - next) + sum[axis]
        };
        sum[axis] = next;
    }
}

fn smoothing_key(input: Input<'_, impl PositionSource>, corner: usize) -> (usize, u32, usize) {
    let face = corner / 3;
    let group = input.smoothing_groups.map_or(1, |groups| groups[face]);
    (
        input.vertex(corner),
        group,
        if group == 0 { face } else { 0 },
    )
}

fn less(input: Input<'_, impl PositionSource>, left: usize, right: usize) -> bool {
    // The corner tie-break gives a deterministic original-order accumulation within each bucket.
    (smoothing_key(input, left), left) < (smoothing_key(input, right), right)
}

fn heapsort<E>(
    input: Input<'_, impl PositionSource>,
    order: &mut [usize],
    control: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), Error<E>> {
    for root in (0..order.len() / 2).rev() {
        control().map_err(Error::Control)?;
        sift_down(input, order, root, order.len(), control)?;
    }
    for end in (1..order.len()).rev() {
        control().map_err(Error::Control)?;
        order.swap(0, end);
        sift_down(input, order, 0, end, control)?;
    }
    Ok(())
}

fn sift_down<E>(
    input: Input<'_, impl PositionSource>,
    order: &mut [usize],
    mut root: usize,
    end: usize,
    control: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), Error<E>> {
    // This bound also keeps 2 * root + 1 in range without overflowing.
    while root < end / 2 {
        control().map_err(Error::Control)?;
        let mut child = 2 * root + 1;
        if child + 1 < end && less(input, order[child], order[child + 1]) {
            child += 1;
        }
        if !less(input, order[root], order[child]) {
            break;
        }
        order.swap(root, child);
        root = child;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "diagnostics.rs"]
mod diagnostics;
