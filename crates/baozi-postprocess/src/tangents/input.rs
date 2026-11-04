#![allow(
    clippy::result_large_err,
    reason = "resource and control failures propagate inline without rejection-path allocation"
)]

use baozi_core::__scene_integration::integration::SceneVectorValues;
use baozi_core::{AttributeData, AttributeDomain, AttributeRef, AttributeSemanticRef};
use baozi_operation::StoreRuntimeError;

use super::{TangentIssue, kernel};
use crate::operation::runtime::{ActiveRuntime, RuntimeKeys, ScratchVec, run_unit};

#[derive(Debug)]
pub(crate) enum InputError {
    Positions,
    NormalField,
    UvField,
    UnsupportedDomain,
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
    operation: &mut ActiveRuntime<'_>,
    keys: RuntimeKeys,
) -> Result<GeneratedTangents, InputError> {
    enum PendingTangents {
        F32(Option<ScratchVec<[f32; 4]>>),
        F64(Option<ScratchVec<[f64; 4]>>),
    }

    let mut values = if source.wide_output {
        PendingTangents::F64(None)
    } else {
        PendingTangents::F32(None)
    };
    for face in 0..source.corners as usize / 3 {
        let mut positions = [[0.0; 3]; 3];
        let mut normals = [[0.0; 3]; 3];
        let mut uvs = [[0.0; 2]; 3];
        for local in 0..3 {
            let corner = 3 * face + local;
            let vertex = run_unit(operation, || {
                source.indices.map_or(corner, |ids| ids[corner] as usize)
            })?;
            positions[local] = read(source.positions, vertex, corner, operation)?;
            let original_corner = source.origins.map_or(corner, |o| o[face][local] as usize);
            normals[local] = match source.normals {
                ProjectedNormals::Authored(attribute) => {
                    read(attribute, vertex, original_corner, operation)?
                }
                ProjectedNormals::Generated(SceneVectorValues::F32(values)) => {
                    run_unit(operation, || values[corner].map(f64::from))?
                }
                ProjectedNormals::Generated(SceneVectorValues::F64(values)) => {
                    run_unit(operation, || values[corner])?
                }
            };
            uvs[local] = read(source.uvs, vertex, original_corner, operation)?;
        }
        match &mut values {
            PendingTangents::F32(values) => {
                // All declared inputs are F32 in this branch. These are exact reversals of
                // the logical-read widening, preserving the F32 exact-product kernel.
                let generated = run_unit(operation, || {
                    kernel::triangle(
                        face,
                        positions.map(|p| p.map(|v| v as f32)),
                        normals.map(|p| p.map(|v| v as f32)),
                        uvs.map(|p| p.map(|v| v as f32)),
                    )
                })?
                .map_err(InputError::Issue)?;
                append_triangle(values, generated, source.corners as usize, operation, keys)?;
            }
            PendingTangents::F64(values) => {
                let generated = run_unit(operation, || {
                    super::wide::triangle_with_sources(
                        face,
                        positions,
                        normals,
                        uvs,
                        source.sources,
                    )
                })?
                .map_err(InputError::Issue)?;
                append_triangle(values, generated, source.corners as usize, operation, keys)?;
            }
        }
    }
    let values = match values {
        PendingTangents::F32(values) => TangentValues::F32(match values {
            Some(values) => values,
            None => ScratchVec::new(operation, keys, source.corners as usize)?,
        }),
        PendingTangents::F64(values) => TangentValues::F64(match values {
            Some(values) => values,
            None => ScratchVec::new(operation, keys, source.corners as usize)?,
        }),
    };
    Ok(GeneratedTangents { values })
}

fn append_triangle<T>(
    values: &mut Option<ScratchVec<T>>,
    generated: [T; 3],
    count: usize,
    operation: &mut ActiveRuntime<'_>,
    keys: RuntimeKeys,
) -> Result<(), InputError> {
    let values = match values {
        Some(values) => values,
        None => values.insert(ScratchVec::new(operation, keys, count)?),
    };
    for tangent in generated {
        run_unit(operation, || values.values_mut().push(tangent))?;
    }
    Ok(())
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
    operation: &mut ActiveRuntime<'_>,
    mut visit: impl FnMut(f64) -> Result<(), crate::GeometryNumericIssue>,
) -> Result<(), InputError> {
    fn run<T: Copy + Into<f64>>(
        attribute: AttributeRef<'_>,
        values: &[T],
        default: &[T],
        indices: Option<&[u32]>,
        operation: &mut ActiveRuntime<'_>,
        visit: &mut impl FnMut(f64) -> Result<(), crate::GeometryNumericIssue>,
    ) -> Result<(), InputError> {
        for (index, value) in values.iter().chain(default).enumerate() {
            run_unit(operation, || visit((*value).into()))?.map_err(|issue| {
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
            run(attribute, values, &[], None, operation, &mut visit)
        }
        AttributeData::DenseF64 { values, .. } => {
            run(attribute, values, &[], None, operation, &mut visit)
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
            &mut visit,
        ),
        _ => Err(InputError::Positions),
    }
}

pub(crate) fn admit_scale(
    attribute: AttributeRef<'_>,
    operation: &mut ActiveRuntime<'_>,
) -> Result<crate::precision::Pow2Scale, InputError> {
    let mut maximum = 0.0;
    visit_scalars(attribute, operation, |value| {
        crate::precision::observe_magnitude(&mut maximum, value)
    })?;
    let scale = crate::precision::common_scale(maximum);
    visit_scalars(attribute, operation, |value| {
        crate::precision::scale_exact(value, scale).map(|_| ())
    })?;
    Ok(scale)
}

pub(crate) fn read<const N: usize>(
    attribute: AttributeRef<'_>,
    vertex: usize,
    corner: usize,
    operation: &mut ActiveRuntime<'_>,
) -> Result<[f64; N], InputError> {
    let index = match attribute.domain() {
        AttributeDomain::Vertex => vertex,
        AttributeDomain::Corner => corner,
        AttributeDomain::Face | AttributeDomain::Instance => {
            return Err(InputError::UnsupportedDomain);
        }
    };
    let invalid = || match attribute.semantic() {
        AttributeSemanticRef::Position => InputError::Positions,
        AttributeSemanticRef::Normal => InputError::NormalField,
        _ => InputError::UvField,
    };
    let data = attribute.data();
    let sparse = match data {
        AttributeData::SparseF32 { indices, .. } | AttributeData::SparseF64 { indices, .. } => {
            Some(indices)
        }
        _ => None,
    };
    let found = if let Some(indices) = sparse {
        let logical = u32::try_from(index).map_err(|_| invalid())?;
        let (mut low, mut high) = (0, indices.len());
        let mut found = None;
        while low < high {
            let middle = low + (high - low) / 2;
            match run_unit(operation, || indices[middle].cmp(&logical))? {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => {
                    found = Some(middle);
                    break;
                }
            }
        }
        found
    } else {
        Some(index)
    };
    run_unit(operation, || {
        if index >= attribute.len() {
            return Err(invalid());
        }
        fn tuple<T: Copy + Into<f64>, const N: usize>(
            values: &[T],
            index: usize,
            lanes: u8,
        ) -> Option<[f64; N]> {
            let lanes = usize::from(lanes);
            if lanes != N {
                return None;
            }
            let start = index.checked_mul(N)?;
            let values = values.get(start..)?.get(..N)?;
            Some(std::array::from_fn(|lane| values[lane].into()))
        }
        let value = match (data, found) {
            (AttributeData::DenseF32 { values, lanes }, Some(index))
            | (AttributeData::SparseF32 { values, lanes, .. }, Some(index)) => {
                tuple(values, index, lanes)
            }
            (AttributeData::DenseF64 { values, lanes }, Some(index))
            | (AttributeData::SparseF64 { values, lanes, .. }, Some(index)) => {
                tuple(values, index, lanes)
            }
            (AttributeData::SparseF32 { default, lanes, .. }, None) => tuple(default, 0, lanes),
            (AttributeData::SparseF64 { default, lanes, .. }, None) => tuple(default, 0, lanes),
            _ => None,
        };
        value.ok_or_else(invalid)
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
        operation: &mut ActiveRuntime<'_>,
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
                admit_scale(positions, operation)?
            } else {
                crate::precision::common_scale(0.0)
            },
            uv_scale: if wide_output {
                admit_scale(uvs, operation)?
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
