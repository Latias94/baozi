#![allow(
    clippy::result_large_err,
    reason = "resource and control failures propagate inline without rejection-path allocation"
)]

use baozi_core::__scene_integration::integration::SceneVectorValues;
use baozi_core::{
    AttributeData, AttributeDomain, AttributeElement, AttributeRef, AttributeSemanticRef,
};
use baozi_operation::StoreRuntimeError;

use super::{TangentIssue, kernel};
use crate::operation::runtime::{EditOperation, RuntimeKeys, ScratchVec, run_unit};

#[derive(Debug)]
pub(crate) enum InputError {
    Positions,
    NormalField,
    UvField,
    UnsupportedDomain,
    SizeOverflow,
    Issue(TangentIssue),
    Runtime(StoreRuntimeError),
}

impl From<StoreRuntimeError> for InputError {
    fn from(error: StoreRuntimeError) -> Self {
        Self::Runtime(error)
    }
}

pub(crate) struct TangentInput<'s> {
    corners: u32,
    indices: Option<&'s [u32]>,
    positions: AttributeRef<'s>,
    normals: ProjectedNormals<'s>,
    origins: Option<&'s [[u32; 3]]>,
    uvs: AttributeRef<'s>,
    sources: super::wide::Sources,
    wide_output: bool,
}

pub(crate) struct GeneratedTangents {
    values: TangentValues,
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
}

pub(crate) fn generate(
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
            let original_corner = source.origins.map_or(corner, |o| o[face][local] as usize);
            normals[local] = match source.normals {
                ProjectedNormals::Authored(attribute) => {
                    read(attribute, vertex, original_corner, operation, keys)?
                }
                ProjectedNormals::Generated(SceneVectorValues::F32(values)) => {
                    run_unit(operation, keys, || values[corner].map(f64::from))?
                }
                ProjectedNormals::Generated(SceneVectorValues::F64(values)) => {
                    run_unit(operation, keys, || values[corner])?
                }
            };
            uvs[local] = read(source.uvs, vertex, original_corner, operation, keys)?;
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
    Ok(GeneratedTangents { values })
}

pub(crate) fn is_f64(attribute: AttributeRef<'_>) -> bool {
    matches!(
        attribute.data(),
        AttributeData::DenseF64 { .. } | AttributeData::SparseF64 { .. }
    )
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

pub(crate) fn admit_scale(
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

pub(crate) fn read<const N: usize>(
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

#[derive(Clone, Copy)]
pub(crate) enum ProjectedNormals<'a> {
    Authored(AttributeRef<'a>),
    Generated(SceneVectorValues<'a, 3>),
}
impl<'a> TangentInput<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn projected(
        corners: u32,
        indices: Option<&'a [u32]>,
        positions: AttributeRef<'a>,
        normals: ProjectedNormals<'a>,
        uvs: AttributeRef<'a>,
        origins: Option<&'a [[u32; 3]]>,
        replaced_wide: bool,
        operation: &mut EditOperation,
        keys: RuntimeKeys,
    ) -> Result<Self, InputError> {
        let normal_wide = match normals {
            ProjectedNormals::Authored(a) => is_f64(a),
            ProjectedNormals::Generated(v) => v.is_f64(),
        };
        let wide_output = replaced_wide || is_f64(positions) || normal_wide || is_f64(uvs);
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
        Ok(Self {
            corners,
            indices,
            positions,
            normals,
            uvs,
            origins,
            sources,
            wide_output,
        })
    }
}
