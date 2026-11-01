#![allow(
    clippy::result_large_err,
    reason = "bounded runtime failures stay inline so allocation and cancellation rejection need no new allocation"
)]

use baozi_core::{AttributeData, AttributeRef};

use crate::TriangulationCause;
use crate::operation::runtime::{EditOperation, RuntimeKeys, ScratchVec, run_unit};
use crate::triangulation::{
    CornerScratch, TriangulationError, triangulate_polygon, validate_polygon_triangulation,
};

const MAX_SPARSE_COMPARISONS: u64 = 33;

pub(crate) struct PolygonInput<'a> {
    positions: AttributeRef<'a>,
    face_sizes: &'a [u32],
    indices: Option<&'a [u32]>,
    triangles: usize,
    max_corners: usize,
}

pub(crate) struct Origins {
    pub(crate) triangles: ScratchVec<[u32; 3]>,
    pub(crate) face_offsets: ScratchVec<u32>,
}

pub(crate) fn projected<'a>(
    positions: AttributeRef<'a>,
    face_sizes: &'a [u32],
    indices: Option<&'a [u32]>,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<PolygonInput<'a>, TriangulationCause> {
    let mut triangles = 0_usize;
    let mut max_corners = 0_usize;
    for &size in face_sizes {
        run_unit(operation, keys, || {
            let size = size as usize;
            triangles = triangles
                .checked_add(
                    size.checked_sub(2)
                        .ok_or(TriangulationCause::SizeOverflow)?,
                )
                .ok_or(TriangulationCause::SizeOverflow)?;
            max_corners = max_corners.max(size);
            Ok::<_, TriangulationCause>(())
        })??;
    }
    Ok(PolygonInput {
        positions,
        face_sizes,
        indices,
        triangles,
        max_corners,
    })
}

pub(crate) fn build_origins(
    source: PolygonInput<'_>,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<Origins, TriangulationCause> {
    let mut triangles = ScratchVec::new(operation, keys, source.triangles)?;
    let prefixes = source
        .face_sizes
        .len()
        .checked_add(1)
        .ok_or(TriangulationCause::SizeOverflow)?;
    let mut face_offsets = ScratchVec::new(operation, keys, prefixes)?;
    let mut positions = ScratchVec::new(operation, keys, source.max_corners)?;
    let mut corners = ScratchVec::new(operation, keys, source.max_corners)?;
    let mut scratch = ScratchVec::new(operation, keys, source.max_corners)?;
    let mut output = ScratchVec::new(operation, keys, source.max_corners - 2)?;
    run_unit(operation, keys, || face_offsets.values_mut().push(0))?;

    let mut first_corner = 0_usize;
    for (face, &count) in source.face_sizes.iter().enumerate() {
        let count = count as usize;
        positions.values_mut().clear();
        corners.values_mut().clear();
        scratch.values_mut().clear();
        output.values_mut().clear();
        for local in 0..count {
            let (corner, vertex) = run_unit(operation, keys, || {
                let corner = first_corner
                    .checked_add(local)
                    .ok_or(TriangulationCause::SizeOverflow)?;
                let vertex = match source.indices {
                    Some(indices) => {
                        *indices.get(corner).ok_or(TriangulationCause::Positions)? as usize
                    }
                    None => corner,
                };
                if vertex >= source.positions.len() {
                    return Err(TriangulationCause::Positions);
                }
                Ok::<_, TriangulationCause>((corner, vertex))
            })??;
            let value = read_position(source.positions, vertex, operation, keys)?;
            run_unit(operation, keys, || {
                positions.values_mut().push(value);
                corners.values_mut().push(corner);
                scratch.values_mut().push(CornerScratch::default());
                if local < count - 2 {
                    output.values_mut().push([0; 3]);
                }
                Ok::<_, TriangulationCause>(())
            })??;
        }
        let mut control = || run_unit(operation, keys, || ());
        triangulate_polygon(
            positions.values(),
            corners.values(),
            scratch.values_mut(),
            output.values_mut(),
            &mut control,
        )
        .map_err(|cause| polygon_cause(face as u32, cause))?;
        validate_polygon_triangulation(
            positions.values(),
            corners.values(),
            output.values(),
            scratch.values_mut(),
            &mut control,
        )
        .map_err(|cause| polygon_cause(face as u32, cause))?;
        for triple in output.values() {
            run_unit(operation, keys, || {
                let mut checked = [0; 3];
                for lane in 0..3 {
                    checked[lane] = u32::try_from(triple[lane])
                        .map_err(|_| TriangulationCause::SizeOverflow)?;
                }
                triangles.values_mut().push(checked);
                Ok::<_, TriangulationCause>(())
            })??;
        }
        run_unit(operation, keys, || {
            let end = u32::try_from(triangles.values().len())
                .map_err(|_| TriangulationCause::SizeOverflow)?;
            face_offsets.values_mut().push(end);
            first_corner = first_corner
                .checked_add(count)
                .ok_or(TriangulationCause::SizeOverflow)?;
            Ok::<_, TriangulationCause>(())
        })??;
    }
    operation.control_checkpoint().poll()?;
    Ok(Origins {
        triangles,
        face_offsets,
    })
}

fn read_position(
    attribute: AttributeRef<'_>,
    vertex: usize,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<[f64; 3], TriangulationCause> {
    let data = attribute.data();
    let index = match data {
        AttributeData::SparseF32 { indices, .. } | AttributeData::SparseF64 { indices, .. } => {
            sparse_index(indices, vertex, operation, keys)?
        }
        _ => Some(vertex),
    };
    run_unit(operation, keys, || match (data, index) {
        (AttributeData::DenseF32 { lanes: 3, values }, Some(index))
        | (
            AttributeData::SparseF32 {
                lanes: 3, values, ..
            },
            Some(index),
        ) => position_tuple(values, index),
        (AttributeData::DenseF64 { lanes: 3, values }, Some(index))
        | (
            AttributeData::SparseF64 {
                lanes: 3, values, ..
            },
            Some(index),
        ) => position_tuple(values, index),
        (
            AttributeData::SparseF32 {
                lanes: 3, default, ..
            },
            None,
        ) => position_tuple(default, 0),
        (
            AttributeData::SparseF64 {
                lanes: 3, default, ..
            },
            None,
        ) => position_tuple(default, 0),
        _ => Err(TriangulationCause::Positions),
    })?
}

fn position_tuple<T: Copy + Into<f64>>(
    values: &[T],
    index: usize,
) -> Result<[f64; 3], TriangulationCause> {
    let start = index
        .checked_mul(3)
        .ok_or(TriangulationCause::SizeOverflow)?;
    let values = values
        .get(start..)
        .and_then(|tail| tail.get(..3))
        .ok_or(TriangulationCause::Positions)?;
    Ok([values[0].into(), values[1].into(), values[2].into()])
}

fn sparse_index(
    indices: &[u32],
    vertex: usize,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<Option<usize>, TriangulationCause> {
    let vertex = u32::try_from(vertex).map_err(|_| TriangulationCause::Positions)?;
    let (mut low, mut high) = (0, indices.len());
    for _ in 0..MAX_SPARSE_COMPARISONS {
        if low == high {
            return Ok(None);
        }
        let middle = low + (high - low) / 2;
        match run_unit(operation, keys, || indices[middle].cmp(&vertex))? {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Ok(Some(middle)),
        }
    }
    if low == high {
        Ok(None)
    } else {
        Err(TriangulationCause::CoreRejected)
    }
}

fn polygon_cause(
    face: u32,
    cause: TriangulationError<baozi_operation::StoreRuntimeError>,
) -> TriangulationCause {
    match cause {
        TriangulationError::Polygon(issue) => TriangulationCause::Polygon { face, issue },
        TriangulationError::Control(cause) => cause.into(),
    }
}
