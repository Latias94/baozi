//! Allocation-free TriangleList normals. This sidecar is not a Scene operation.

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
pub(crate) struct Input<'a> {
    pub positions: &'a [[f32; 3]],
    pub vertex_ids: VertexIds<'a>,
    /// One value per face. None smooths all shared identities; zero is hard per face.
    pub smoothing_groups: Option<&'a [u32]>,
}

impl Input<'_> {
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

#[derive(Debug)]
pub(crate) struct Workspace<'a> {
    /// Exactly C entries for Smooth; empty for Flat. Initial contents are ignored.
    pub corner_order: &'a mut [usize],
    /// Optional cache of exactly C / 3 unnormalized f64 face crosses.
    /// Initial contents are ignored. Without it, Smooth recomputes each corner's cross.
    pub face_crosses: Option<&'a mut [[f64; 3]]>,
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
pub(crate) fn generate<E>(
    input: Input<'_>,
    mode: Mode,
    output: &mut [[f32; 3]],
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

    for (vertex, position) in input.positions.iter().enumerate() {
        control().map_err(Error::Control)?;
        for (component, value) in position.iter().enumerate() {
            if !value.is_finite() {
                return Err(Error::NonFinitePosition { vertex, component });
            }
        }
    }

    for face in 0..faces {
        control().map_err(Error::Control)?;
        let cross = face_cross(input, face)?;
        if let Some(crosses) = workspace.face_crosses.as_deref_mut() {
            crosses[face] = cross;
        }
        if mode == Mode::Flat {
            let normal = unit(cross).ok_or(Error::InvalidFaceCross { face })?;
            output[3 * face..3 * face + 3].fill(normal);
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
            while end < corners {
                control().map_err(Error::Control)?;
                let corner = workspace.corner_order[end];
                if smoothing_key(input, corner) != key {
                    break;
                }
                let cross = match workspace.face_crosses.as_deref() {
                    Some(crosses) => crosses[corner / 3],
                    None => face_cross(input, corner / 3)?,
                };
                accumulate(&mut sum, &mut correction, cross);
                end += 1;
            }
            let sum = [
                sum[0] + correction[0],
                sum[1] + correction[1],
                sum[2] + correction[2],
            ];
            let normal = unit(sum).ok_or(Error::InvalidSmoothSum {
                vertex: key.0,
                group: input.smoothing_groups.map(|groups| groups[first / 3]),
                corner: first,
            })?;
            for &corner in &workspace.corner_order[start..end] {
                control().map_err(Error::Control)?;
                output[corner] = normal;
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

fn face_cross<E>(input: Input<'_>, face: usize) -> Result<[f64; 3], Error<E>> {
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
    // Every product of two finite f32 coordinates is exact in f64, even at the exponent limits.
    // Expand the determinant before summation: rounded edge differences can lose its direction.
    let a = input.positions[vertices[0]].map(f64::from);
    let b = input.positions[vertices[1]].map(f64::from);
    let c = input.positions[vertices[2]].map(f64::from);
    let cross = [
        determinant(a, b, c, 1, 2),
        determinant(a, b, c, 2, 0),
        determinant(a, b, c, 0, 1),
    ];
    if cross.iter().any(|value| !value.is_finite()) {
        return Err(Error::InvalidFaceCross { face });
    }
    if cross == [0.0; 3] {
        return Err(Error::DegenerateFace { face });
    }
    Ok(if reversed {
        cross.map(|value| -value)
    } else {
        cross
    })
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

fn unit(vector: [f64; 3]) -> Option<[f32; 3]> {
    if vector.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let scale = vector[0].abs().max(vector[1].abs()).max(vector[2].abs());
    if scale == 0.0 {
        return None;
    }
    let scaled = vector.map(|value| value / scale);
    let length = (scaled[0] * scaled[0] + scaled[1] * scaled[1] + scaled[2] * scaled[2]).sqrt();
    Some(scaled.map(|value| (value / length) as f32))
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

fn smoothing_key(input: Input<'_>, corner: usize) -> (usize, u32, usize) {
    let face = corner / 3;
    let group = input.smoothing_groups.map_or(1, |groups| groups[face]);
    (
        input.vertex(corner),
        group,
        if group == 0 { face } else { 0 },
    )
}

fn less(input: Input<'_>, left: usize, right: usize) -> bool {
    // The corner tie-break gives a deterministic original-order accumulation within each bucket.
    (smoothing_key(input, left), left) < (smoothing_key(input, right), right)
}

fn heapsort<E>(
    input: Input<'_>,
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
    input: Input<'_>,
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
