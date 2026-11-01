#![allow(
    clippy::result_large_err,
    reason = "resource and control failures propagate inline without rejection-path allocation"
)]

use baozi_core::__scene_integration::integration::SceneVectorValues;
use baozi_core::{
    AttributeData, AttributeDomain, AttributeElement, AttributeRef, AttributeSemanticRef,
    PrimitiveId, SceneLookupError, TopologyKind, ValidatedScene,
};
use baozi_operation::StoreRuntimeError;

use super::{ExistingTangents, TangentIssue, TangentOptions, kernel};
use crate::operation::runtime::{EditOperation, MAX_WORK, RuntimeKeys, ScratchVec, run_unit};

#[derive(Debug)]
pub(super) enum InputError {
    Target(SceneLookupError),
    UnsupportedTopology,
    Positions,
    NormalField,
    UvField,
    UnsupportedDomain,
    MultipleTangentFields,
    MorphDeltas,
    SizeOverflow,
    WorkLimit { required: u64, maximum: u64 },
    Issue(TangentIssue),
    Runtime(StoreRuntimeError),
}

impl From<StoreRuntimeError> for InputError {
    fn from(error: StoreRuntimeError) -> Self {
        Self::Runtime(error)
    }
}

pub(super) struct TangentInput<'s> {
    corners: u32,
    indices: Option<&'s [u32]>,
    positions: AttributeRef<'s>,
    normals: AttributeRef<'s>,
    uvs: AttributeRef<'s>,
    had_tangent: bool,
    sources: super::wide::Sources,
    wide_output: bool,
}

pub(crate) struct GeneratedTangents {
    values: TangentValues,
    had_tangent: bool,
}

enum TangentValues {
    F32(ScratchVec<[f32; 4]>),
    F64(ScratchVec<[f64; 4]>),
}

impl GeneratedTangents {
    pub(crate) fn values(&self) -> SceneVectorValues<'_, 4> {
        match &self.values {
            TangentValues::F32(values) => SceneVectorValues::F32(values.values()),
            TangentValues::F64(values) => SceneVectorValues::F64(values.values()),
        }
    }
    pub(crate) const fn had_tangent(&self) -> bool {
        self.had_tangent
    }
}

pub(super) fn inspect<'s>(
    scene: &'s ValidatedScene,
    target: PrimitiveId,
    options: TangentOptions,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<Option<TangentInput<'s>>, InputError> {
    let primitive =
        run_unit(operation, keys, || scene.primitive(target))?.map_err(InputError::Target)?;
    let (topology, corners, indices, mut attributes) = run_unit(operation, keys, || {
        (
            primitive.topology(),
            primitive.corner_count(),
            primitive.indices(),
            primitive.attributes(),
        )
    })?;
    if topology != TopologyKind::TriangleList {
        return Err(InputError::UnsupportedTopology);
    }
    if corners == 0 {
        return Ok(None);
    }
    let attribute_count = attributes.len();
    let mut positions = None;
    let mut normals = None;
    let mut uvs = None;
    let mut tangents = 0;
    let mut replaced_wide = false;
    loop {
        let more = run_unit(operation, keys, || {
            let Some(attribute) = attributes.next() else {
                return false;
            };
            match attribute.semantic() {
                AttributeSemanticRef::Position => positions = Some(attribute),
                AttributeSemanticRef::Normal if attribute.domain() == options.normal_domain => {
                    normals = Some(attribute);
                }
                AttributeSemanticRef::Texcoord(set)
                    if set == options.uv_set && attribute.domain() == options.uv_domain =>
                {
                    uvs = Some(attribute);
                }
                AttributeSemanticRef::Tangent => {
                    tangents += 1;
                    replaced_wide |= is_f64(attribute);
                }
                _ => {}
            }
            true
        })?;
        if !more {
            break;
        }
    }
    if tangents != 0 && options.existing == ExistingTangents::Preserve {
        return Ok(None);
    }
    if tangents > 1 {
        return Err(InputError::MultipleTangentFields);
    }
    if !matches!(
        options.normal_domain,
        AttributeDomain::Vertex | AttributeDomain::Corner
    ) || !matches!(
        options.uv_domain,
        AttributeDomain::Vertex | AttributeDomain::Corner
    ) {
        return Err(InputError::UnsupportedDomain);
    }
    let positions = positions.ok_or(InputError::Positions)?;
    let normals = normals.ok_or(InputError::NormalField)?;
    let uvs = uvs.ok_or(InputError::UvField)?;
    if positions.domain() != AttributeDomain::Vertex || positions.lanes() != 3 {
        return Err(InputError::Positions);
    }
    if normals.lanes() != 3 {
        return Err(InputError::NormalField);
    }
    if uvs.lanes() != 2 {
        return Err(InputError::UvField);
    }

    // Reading each retained morph target is charged, including empty targets. Any delta would
    // need a separate derived tangent frame and cannot be carried into this base-only profile.
    let mut morphs = run_unit(operation, keys, || primitive.morph_targets())?;
    let morph_count = u64::try_from(morphs.len()).map_err(|_| InputError::SizeOverflow)?;
    loop {
        let next = run_unit(operation, keys, || {
            morphs.next().map(|target| target.delta_count())
        })?;
        match next {
            Some(0) => {}
            Some(_) => return Err(InputError::MorphDeltas),
            None => break,
        }
    }
    // Sparse lookups are binary searches over validated u32 indices: at most 33 comparisons
    // plus one fixed read per field. Each triangle action bounds nine field reads and a fixed
    // stack-only kernel; initialization and preflight/publication retain separate admission.
    let wide_output = replaced_wide || is_f64(positions) || is_f64(normals) || is_f64(uvs);
    let scale_work = if wide_output {
        retained_scalar_count(positions)?
            .checked_add(retained_scalar_count(uvs)?)
            .and_then(|count| count.checked_mul(2))
            .ok_or(InputError::SizeOverflow)?
    } else {
        0
    };
    let required = u64::from(corners)
        .checked_mul(110)
        .and_then(|work| work.checked_add(scale_work))
        .and_then(|work| work.checked_add(u64::try_from(attribute_count).ok()?))
        .and_then(|work| work.checked_add(morph_count))
        .and_then(|work| work.checked_add(16))
        .ok_or(InputError::SizeOverflow)?;
    if required > MAX_WORK {
        return Err(InputError::WorkLimit {
            required,
            maximum: MAX_WORK,
        });
    }
    let sources = super::wide::Sources {
        position_f64: is_f64(positions),
        uv_f64: is_f64(uvs),
        position_scale: if wide_output {
            admit_scale(positions, operation, keys)?
        } else {
            crate::precision::common_scale(0.0)
        },
        uv_scale: if wide_output {
            admit_scale(uvs, operation, keys)?
        } else {
            crate::precision::common_scale(0.0)
        },
    };
    Ok(Some(TangentInput {
        corners,
        indices,
        positions,
        normals,
        uvs,
        had_tangent: tangents != 0,
        sources,
        wide_output,
    }))
}

pub(super) fn generate(
    source: TangentInput<'_>,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<GeneratedTangents, InputError> {
    let mut values = if source.wide_output {
        let mut values = ScratchVec::new(operation, keys, source.corners as usize)?;
        for _ in 0..source.corners {
            run_unit(operation, keys, || values.values_mut().push([0.0_f64; 4]))?;
        }
        TangentValues::F64(values)
    } else {
        let mut values = ScratchVec::new(operation, keys, source.corners as usize)?;
        for _ in 0..source.corners {
            run_unit(operation, keys, || values.values_mut().push([0.0_f32; 4]))?;
        }
        TangentValues::F32(values)
    };
    for face in 0..source.corners as usize / 3 {
        let mut positions = [[0.0; 3]; 3];
        let mut normals = [[0.0; 3]; 3];
        let mut uvs = [[0.0; 2]; 3];
        for local in 0..3 {
            let corner = 3 * face + local;
            let vertex = run_unit(operation, keys, || {
                source.indices.map_or(corner, |ids| ids[corner] as usize)
            })?;
            positions[local] = read(source.positions, vertex, corner, operation, keys)?;
            normals[local] = read(source.normals, vertex, corner, operation, keys)?;
            uvs[local] = read(source.uvs, vertex, corner, operation, keys)?;
        }
        match &mut values {
            TangentValues::F32(values) => {
                // All declared inputs are F32 in this branch. These are exact reversals of
                // the logical-read widening, preserving the F32 exact-product kernel.
                let generated = run_unit(operation, keys, || {
                    kernel::triangle(
                        face,
                        positions.map(|p| p.map(|v| v as f32)),
                        normals.map(|p| p.map(|v| v as f32)),
                        uvs.map(|p| p.map(|v| v as f32)),
                    )
                })?
                .map_err(InputError::Issue)?;
                for (local, tangent) in generated.into_iter().enumerate() {
                    run_unit(operation, keys, || {
                        values.values_mut()[3 * face + local] = tangent
                    })?;
                }
            }
            TangentValues::F64(values) => {
                let generated = run_unit(operation, keys, || {
                    super::wide::triangle_with_sources(
                        face,
                        positions,
                        normals,
                        uvs,
                        source.sources,
                    )
                })?
                .map_err(InputError::Issue)?;
                for (local, tangent) in generated.into_iter().enumerate() {
                    run_unit(operation, keys, || {
                        values.values_mut()[3 * face + local] = tangent
                    })?;
                }
            }
        }
    }
    Ok(GeneratedTangents {
        values,
        had_tangent: source.had_tangent,
    })
}

fn is_f64(attribute: AttributeRef<'_>) -> bool {
    matches!(
        attribute.data(),
        AttributeData::DenseF64 { .. } | AttributeData::SparseF64 { .. }
    )
}

fn retained_scalar_count(attribute: AttributeRef<'_>) -> Result<u64, InputError> {
    let (values, default) = match attribute.data() {
        AttributeData::DenseF32 { values, .. } => (values.len(), 0),
        AttributeData::DenseF64 { values, .. } => (values.len(), 0),
        AttributeData::SparseF32 {
            values,
            default,
            indices,
            logical_count,
            ..
        } => (
            values.len(),
            if indices.len() < logical_count as usize {
                default.len()
            } else {
                0
            },
        ),
        AttributeData::SparseF64 {
            values,
            default,
            indices,
            logical_count,
            ..
        } => (
            values.len(),
            if indices.len() < logical_count as usize {
                default.len()
            } else {
                0
            },
        ),
        _ => return Err(InputError::Positions),
    };
    values
        .checked_add(default)
        .and_then(|n| u64::try_from(n).ok())
        .ok_or(InputError::SizeOverflow)
}

/// Sparse explicit values and a used default cover exactly the logical magnitudes, even when
/// the logical domain is large. Each retained scalar read is admitted; no dense copy is built.
fn visit_scalars(
    attribute: AttributeRef<'_>,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
    mut visit: impl FnMut(f64) -> Result<(), crate::GeometryNumericIssue>,
) -> Result<(), InputError> {
    fn run<T: Copy + Into<f64>>(
        attribute: AttributeRef<'_>,
        values: &[T],
        default: &[T],
        indices: Option<&[u32]>,
        operation: &mut EditOperation,
        keys: RuntimeKeys,
        visit: &mut impl FnMut(f64) -> Result<(), crate::GeometryNumericIssue>,
    ) -> Result<(), InputError> {
        for (index, value) in values.iter().chain(default).enumerate() {
            run_unit(operation, keys, || visit((*value).into()))?.map_err(|issue| {
                let element = if index >= values.len() {
                    None
                } else {
                    let row = index / usize::from(attribute.lanes());
                    Some(indices.map_or(row, |indices| indices[row] as usize))
                };
                InputError::Issue(TangentIssue::Scaling {
                    semantic: if attribute.semantic() == AttributeSemanticRef::Position {
                        baozi_core::AttributeSemanticKind::Position
                    } else {
                        baozi_core::AttributeSemanticKind::Texcoord
                    },
                    element,
                    issue,
                })
            })?;
        }
        Ok(())
    }
    match attribute.data() {
        AttributeData::DenseF32 { values, .. } => {
            run(attribute, values, &[], None, operation, keys, &mut visit)
        }
        AttributeData::DenseF64 { values, .. } => {
            run(attribute, values, &[], None, operation, keys, &mut visit)
        }
        AttributeData::SparseF32 {
            values,
            default,
            indices,
            logical_count,
            ..
        } => run(
            attribute,
            values,
            if indices.len() < logical_count as usize {
                default
            } else {
                &[]
            },
            Some(indices),
            operation,
            keys,
            &mut visit,
        ),
        AttributeData::SparseF64 {
            values,
            default,
            indices,
            logical_count,
            ..
        } => run(
            attribute,
            values,
            if indices.len() < logical_count as usize {
                default
            } else {
                &[]
            },
            Some(indices),
            operation,
            keys,
            &mut visit,
        ),
        _ => Err(InputError::Positions),
    }
}

fn admit_scale(
    attribute: AttributeRef<'_>,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<crate::precision::Pow2Scale, InputError> {
    let mut maximum = 0.0;
    visit_scalars(attribute, operation, keys, |value| {
        crate::precision::observe_magnitude(&mut maximum, value)
    })?;
    let scale = crate::precision::common_scale(maximum);
    visit_scalars(attribute, operation, keys, |value| {
        crate::precision::scale_exact(value, scale).map(|_| ())
    })?;
    Ok(scale)
}

fn read<const N: usize>(
    attribute: AttributeRef<'_>,
    vertex: usize,
    corner: usize,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<[f64; N], InputError> {
    let index = match attribute.domain() {
        AttributeDomain::Vertex => vertex,
        AttributeDomain::Corner => corner,
        AttributeDomain::Face | AttributeDomain::Instance => {
            return Err(InputError::UnsupportedDomain);
        }
    };
    // Conservatively charge the bounded sparse lookup rather than hiding its comparisons in
    // one scalar-read action. Dense reads need one action, sparse reads at most 34.
    let steps = match attribute.data() {
        AttributeData::SparseF32 { indices, .. } | AttributeData::SparseF64 { indices, .. } => {
            indices
                .len()
                .checked_ilog2()
                .map_or(1, |log| log as usize + 2)
        }
        _ => 1,
    };
    for _ in 1..steps {
        run_unit(operation, keys, || ())?;
    }
    run_unit(operation, keys, || match attribute.element(index) {
        Some(AttributeElement::F32(lanes)) => {
            let tuple: &[f32; N] = lanes.try_into().map_err(|_| InputError::SizeOverflow)?;
            Ok(tuple.map(f64::from))
        }
        Some(AttributeElement::F64(lanes)) => {
            let tuple: &[f64; N] = lanes.try_into().map_err(|_| InputError::SizeOverflow)?;
            Ok(*tuple)
        }
        _ => Err(match attribute.semantic() {
            AttributeSemanticRef::Position => InputError::Positions,
            AttributeSemanticRef::Normal => InputError::NormalField,
            _ => InputError::UvField,
        }),
    })?
}
