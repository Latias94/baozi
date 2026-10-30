#![allow(
    clippy::result_large_err,
    reason = "runtime and kernel failures stay inline when allocation or cancellation rejects work"
)]

use baozi_core::{
    AttributeData, AttributeDomain, AttributeSemanticRef, PrimitiveId, SceneLookupError,
    TopologyKind, ValidatedScene,
};
use baozi_operation::StoreRuntimeError;

use super::kernel;
use super::options::{ExistingNormals, NormalMode, NormalOptions, TangentHandling};
use crate::operation::runtime::{EditOperation, MAX_WORK, RuntimeKeys, ScratchVec, run_unit};

#[derive(Debug)]
pub(super) enum InputError {
    Target(SceneLookupError),
    UnsupportedTopology,
    Positions,
    SmoothingGroups,
    TangentsRequireRemoval,
    MultipleNormalFields,
    MultipleTangentFields,
    SizeOverflow,
    WorkLimit { required: u64, maximum: u64 },
    Kernel(kernel::Error<StoreRuntimeError>),
    Runtime(StoreRuntimeError),
}

impl From<StoreRuntimeError> for InputError {
    fn from(cause: StoreRuntimeError) -> Self {
        Self::Runtime(cause)
    }
}

/// Source payloads remain borrowed until generation finishes; no position projection is retained.
pub(super) struct NormalInput<'a> {
    corner_count: u32,
    positions: &'a [[f32; 3]],
    indices: Option<&'a [u32]>,
    smoothing_groups: Option<&'a [u32]>,
    mode: NormalMode,
    had_normal: bool,
}

/// Only the generated Corner values and source facts survive into the Scene transaction.
pub(crate) struct GeneratedNormals {
    normals: ScratchVec<[f32; 3]>,
    had_normal: bool,
}

impl GeneratedNormals {
    pub(crate) fn values(&self) -> &[[f32; 3]] {
        self.normals.values()
    }

    pub(crate) fn had_normal(&self) -> bool {
        self.had_normal
    }
}

pub(super) fn inspect<'a>(
    input: &'a ValidatedScene,
    target: PrimitiveId,
    options: NormalOptions,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<Option<NormalInput<'a>>, InputError> {
    let primitive =
        run_unit(operation, keys, || input.primitive(target))?.map_err(InputError::Target)?;
    let (topology, vertices, corners, faces, indices, mut attributes) =
        run_unit(operation, keys, || {
            (
                primitive.topology(),
                primitive.vertex_count(),
                primitive.corner_count(),
                primitive.face_count(),
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
    let mut smoothing_groups = None;
    let mut normal_fields = 0_usize;
    let mut tangent_fields = 0_usize;
    // Advancing the iterator reads a descriptor. Admit that read as well as its inspection;
    // convenience lookups such as `positions()` would hide a complete descriptor search.
    loop {
        let more = run_unit(operation, keys, || {
            let Some(attribute) = attributes.next() else {
                return false;
            };
            match attribute.semantic() {
                AttributeSemanticRef::Position => positions = Some(attribute),
                AttributeSemanticRef::Normal => normal_fields += 1,
                AttributeSemanticRef::Tangent => tangent_fields += 1,
                AttributeSemanticRef::SmoothingGroup => smoothing_groups = Some(attribute),
                _ => {}
            }
            true
        })?;
        if !more {
            break;
        }
    }
    let had_normal = normal_fields != 0;
    if had_normal && options.existing == ExistingNormals::Preserve {
        return Ok(None);
    }

    run_unit(operation, keys, || {
        if normal_fields > 1 {
            return Err(InputError::MultipleNormalFields);
        }
        if tangent_fields != 0 && options.tangents == TangentHandling::Reject {
            return Err(InputError::TangentsRequireRemoval);
        }
        if tangent_fields > 1 {
            return Err(InputError::MultipleTangentFields);
        }
        let positions = positions.ok_or(InputError::Positions)?;
        if positions.domain() != AttributeDomain::Vertex {
            return Err(InputError::Positions);
        }
        let AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
            return Err(InputError::Positions);
        };
        let (positions, remainder) = values.as_chunks::<3>();
        if !remainder.is_empty() || positions.len() != vertices as usize {
            return Err(InputError::Positions);
        }
        let actual_corners = indices.map_or(positions.len(), <[u32]>::len);
        if actual_corners != corners as usize
            || !actual_corners.is_multiple_of(3)
            || actual_corners / 3 != faces as usize
        {
            return Err(InputError::Positions);
        }
        let smoothing_groups = if options.mode == NormalMode::Smooth {
            match smoothing_groups {
                Some(attribute) => {
                    if attribute.domain() != AttributeDomain::Face {
                        return Err(InputError::SmoothingGroups);
                    }
                    let AttributeData::DenseU32 { lanes: 1, values } = attribute.data() else {
                        return Err(InputError::SmoothingGroups);
                    };
                    if values.len() != faces as usize {
                        return Err(InputError::SmoothingGroups);
                    }
                    Some(values)
                }
                None => None,
            }
        } else {
            // Flat normals do not consult smoothing groups, including valid sparse fields.
            None
        };
        let required = work_bound(
            positions.len(),
            actual_corners,
            attribute_count,
            options.mode,
        )?;
        if required > MAX_WORK {
            return Err(InputError::WorkLimit {
                required,
                maximum: MAX_WORK,
            });
        }
        Ok(Some(NormalInput {
            corner_count: corners,
            positions,
            indices,
            smoothing_groups,
            mode: options.mode,
            had_normal,
        }))
    })?
}

/// A checked envelope for this module's inspection, initialization, and kernel callbacks.
/// Scene preflight, correspondence, and publication retain their own runtime admission.
fn work_bound(
    vertices: usize,
    corners: usize,
    attributes: usize,
    mode: NormalMode,
) -> Result<u64, InputError> {
    let vertices = u64::try_from(vertices).map_err(|_| InputError::SizeOverflow)?;
    let corners = u64::try_from(corners).map_err(|_| InputError::SizeOverflow)?;
    let attributes = u64::try_from(attributes).map_err(|_| InputError::SizeOverflow)?;
    let faces = corners / 3;
    let base = vertices
        .checked_add(attributes)
        .and_then(|work| work.checked_add(8))
        .ok_or(InputError::SizeOverflow)?;
    let work = match mode {
        // C output initializations, V finite checks, F face computations, and fixed entry work.
        NormalMode::Flat => base
            .checked_add(corners)
            .and_then(|work| work.checked_add(faces)),
        NormalMode::Smooth => {
            let levels = corners.checked_ilog2().map_or(0, u64::from);
            // Heap construction and extraction make fewer than 2C sift calls, each with at
            // most floor(log2(C)) iterations and one outer callback. Initialization, corner
            // ordering, bucket scans (including boundary probes), and scatter add at most 9C
            // in total with those outer callbacks. Cross computation and cache init add 2F.
            levels
                .checked_mul(2)
                .and_then(|factor| factor.checked_add(9))
                .and_then(|factor| corners.checked_mul(factor))
                .and_then(|work| work.checked_add(faces.checked_mul(2)?))
                .and_then(|work| work.checked_add(base))
        }
    };
    work.ok_or(InputError::SizeOverflow)
}

pub(super) fn generate(
    source: NormalInput<'_>,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<GeneratedNormals, InputError> {
    let corners = source.corner_count;
    let faces = corners / 3;
    let mut normals = ScratchVec::new(operation, keys, corners as usize)?;
    let mut order = if source.mode == NormalMode::Smooth {
        Some(ScratchVec::new(operation, keys, corners as usize)?)
    } else {
        None
    };
    let mut crosses = if source.mode == NormalMode::Smooth {
        Some(ScratchVec::new(operation, keys, faces as usize)?)
    } else {
        None
    };
    for _ in 0..corners {
        run_unit(operation, keys, || normals.values_mut().push([0.0_f32; 3]))?;
    }
    if let Some(order) = &mut order {
        for _ in 0..corners {
            run_unit(operation, keys, || order.values_mut().push(0_usize))?;
        }
    }
    if let Some(crosses) = &mut crosses {
        for _ in 0..faces {
            run_unit(operation, keys, || crosses.values_mut().push([0.0_f64; 3]))?;
        }
    }

    kernel::generate(
        kernel::Input {
            positions: source.positions,
            vertex_ids: source
                .indices
                .map_or(kernel::VertexIds::NonIndexed, kernel::VertexIds::Indexed),
            smoothing_groups: source.smoothing_groups,
        },
        match source.mode {
            NormalMode::Flat => kernel::Mode::Flat,
            NormalMode::Smooth => kernel::Mode::Smooth,
        },
        normals.values_mut(),
        kernel::Workspace {
            corner_order: order
                .as_mut()
                .map_or(&mut [], |order| order.values_mut().as_mut_slice()),
            face_crosses: crosses
                .as_mut()
                .map(|crosses| crosses.values_mut().as_mut_slice()),
        },
        || run_unit(operation, keys, || ()),
    )
    .map_err(InputError::Kernel)?;
    drop(crosses);
    drop(order);
    Ok(GeneratedNormals {
        normals,
        had_normal: source.had_normal,
    })
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
