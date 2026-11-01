#![allow(
    clippy::result_large_err,
    reason = "resource and control failures propagate inline without rejection-path allocation"
)]

use baozi_core::{
    AttributeDomain, AttributeElement, AttributeRef, AttributeSemanticRef, PrimitiveId,
    SceneLookupError, TopologyKind, ValidatedScene,
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
}

pub(crate) struct GeneratedTangents {
    values: ScratchVec<[f32; 4]>,
    had_tangent: bool,
}

impl GeneratedTangents {
    pub(crate) fn values(&self) -> &[[f32; 4]] {
        self.values.values()
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
                AttributeSemanticRef::Tangent => tangents += 1,
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
    let required = u64::from(corners)
        .checked_mul(110)
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
    Ok(Some(TangentInput {
        corners,
        indices,
        positions,
        normals,
        uvs,
        had_tangent: tangents != 0,
    }))
}

pub(super) fn generate(
    source: TangentInput<'_>,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<GeneratedTangents, InputError> {
    let mut values = ScratchVec::new(operation, keys, source.corners as usize)?;
    for _ in 0..source.corners {
        run_unit(operation, keys, || values.values_mut().push([0.0; 4]))?;
    }
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
        let generated = run_unit(operation, keys, || {
            kernel::triangle(face, positions, normals, uvs)
        })?
        .map_err(InputError::Issue)?;
        for (local, tangent) in generated.into_iter().enumerate() {
            run_unit(operation, keys, || {
                values.values_mut()[3 * face + local] = tangent
            })?;
        }
    }
    Ok(GeneratedTangents {
        values,
        had_tangent: source.had_tangent,
    })
}

fn read<const N: usize>(
    attribute: AttributeRef<'_>,
    vertex: usize,
    corner: usize,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<[f32; N], InputError> {
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
        baozi_core::AttributeData::SparseF32 { indices, .. } => indices
            .len()
            .checked_ilog2()
            .map_or(1, |log| log as usize + 2),
        _ => 1,
    };
    for _ in 1..steps {
        run_unit(operation, keys, || ())?;
    }
    run_unit(operation, keys, || {
        let Some(AttributeElement::F32(lanes)) = attribute.element(index) else {
            return Err(match attribute.semantic() {
                AttributeSemanticRef::Position => InputError::Positions,
                AttributeSemanticRef::Normal => InputError::NormalField,
                _ => InputError::UvField,
            });
        };
        lanes.try_into().map_err(|_| InputError::SizeOverflow)
    })?
}
