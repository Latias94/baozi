#![allow(
    clippy::result_large_err,
    reason = "resource failures propagate inline without rejection-path allocation"
)]

use super::{
    NormalOrientation, PreparationCause, TangentPolicy, TriangleLowering,
    TrianglePreparationOptions,
};
use crate::operation::runtime::{EditOperation, RuntimeKeys, ScratchVec, run_unit};
use crate::{ExistingNormals, ExistingTangents, NormalCause, NormalMode, TangentCause};
use baozi_core::{
    AttributeData, AttributeDomain, AttributeRef, AttributeSemanticRef, FrontFaceWinding,
    PrimitiveId, TopologyKind, ValidatedScene,
};

pub(super) struct PreparedPrimitive {
    pub target: PrimitiveId,
    pub origins: Option<crate::triangulate::input::Origins>,
    pub normals: Option<crate::normals::input::GeneratedNormals>,
    pub tangents: Option<crate::tangents::input::GeneratedTangents>,
    pub remove_tangents: bool,
    pub smooth: bool,
    pub normal_domain: AttributeDomain,
    pub uv_set: u32,
    pub uv_domain: AttributeDomain,
}

impl PreparedPrimitive {
    fn unchanged(target: PrimitiveId) -> Self {
        Self {
            target,
            origins: None,
            normals: None,
            tangents: None,
            remove_tangents: false,
            smooth: false,
            normal_domain: AttributeDomain::Corner,
            uv_set: 0,
            uv_domain: AttributeDomain::Vertex,
        }
    }

    pub(super) fn changed(&self) -> bool {
        self.origins.is_some()
            || self.normals.is_some()
            || self.tangents.is_some()
            || self.remove_tangents
    }
}

pub(super) fn prepare(
    scene: &ValidatedScene,
    targets: &[PrimitiveId],
    options: TrianglePreparationOptions,
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<ScratchVec<PreparedPrimitive>, PreparationCause> {
    if targets.is_empty() {
        return ScratchVec::new(operation, keys, 0).map_err(PreparationCause::from);
    }
    let mut sorted = ScratchVec::new(operation, keys, targets.len())?;
    for &target in targets {
        run_unit(operation, keys, || sorted.values_mut().push(target))?;
    }
    sort_targets(sorted.values_mut(), operation, keys)?;
    let mut selected = ScratchVec::new(operation, keys, targets.len())?;
    // Public handles stay opaque. Resolve their source ordering in one borrowed scene walk.
    for &target in sorted.values() {
        run_unit(operation, keys, || scene.primitive(target))?.map_err(PreparationCause::Target)?;
    }
    let mut meshes = run_unit(operation, keys, || scene.meshes())?;
    loop {
        let mesh = run_unit(operation, keys, || meshes.next())?;
        let Some(mesh) = mesh else {
            break;
        };
        let mut primitives = run_unit(operation, keys, || mesh.primitives())?;
        loop {
            let primitive = run_unit(operation, keys, || primitives.next())?;
            let Some(primitive) = primitive else {
                break;
            };
            let id = run_unit(operation, keys, || primitive.id())?;
            let (mut first, mut last) = (0, sorted.values().len());
            let mut found = false;
            while first < last {
                let middle = first + (last - first) / 2;
                match run_unit(operation, keys, || sorted.values()[middle].cmp(&id))? {
                    std::cmp::Ordering::Less => first = middle + 1,
                    std::cmp::Ordering::Greater => last = middle,
                    std::cmp::Ordering::Equal => {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                run_unit(operation, keys, || selected.values_mut().push(primitive))?;
            }
        }
    }
    let mut output = ScratchVec::new(operation, keys, targets.len())?;
    for pair in sorted.values().windows(2) {
        if run_unit(operation, keys, || pair[0] == pair[1])? {
            return Err(PreparationCause::DuplicateTarget);
        }
    }
    for &primitive in selected.values() {
        let target = run_unit(operation, keys, || primitive.id())?;
        let (topology, corners, faces, indices, sizes, mut attributes) =
            run_unit(operation, keys, || {
                (
                    primitive.topology(),
                    primitive.corner_count(),
                    primitive.face_count(),
                    primitive.indices(),
                    primitive.polygon_face_sizes(),
                    primitive.attributes(),
                )
            })?;
        if !matches!(
            topology,
            TopologyKind::TriangleList
                | TopologyKind::TriangleStrip
                | TopologyKind::TriangleFan
                | TopologyKind::Polygons
        ) || options.lowering == TriangleLowering::Preserve
            && topology != TopologyKind::TriangleList
        {
            return Err(PreparationCause::UnsupportedTopology(topology));
        }
        if faces == 0 {
            run_unit(operation, keys, || {
                output
                    .values_mut()
                    .push(PreparedPrimitive::unchanged(target))
            })?;
            continue;
        }
        let (mut positions, mut selected_normal, mut selected_uv, mut smoothing) =
            (None, None, None, None);
        let (mut normal_count, mut tangent_count) = (0, 0);
        let (mut normal_wide, mut tangent_wide) = (false, false);
        let tangent_options = match options.tangents {
            TangentPolicy::Generate(t) => Some(t),
            _ => None,
        };
        loop {
            if !run_unit(operation, keys, || {
                let Some(a) = attributes.next() else {
                    return false;
                };
                match a.semantic() {
                    AttributeSemanticRef::Position => positions = Some(a),
                    AttributeSemanticRef::Normal => {
                        normal_count += 1;
                        normal_wide |= is_wide(a);
                        if tangent_options.is_some_and(|t| t.normal_domain == a.domain()) {
                            selected_normal = Some(a);
                        }
                    }
                    AttributeSemanticRef::Tangent => {
                        tangent_count += 1;
                        tangent_wide |= is_wide(a);
                    }
                    AttributeSemanticRef::Texcoord(set)
                        if tangent_options
                            .is_some_and(|t| t.uv_set == set && t.uv_domain == a.domain()) =>
                    {
                        selected_uv = Some(a)
                    }
                    AttributeSemanticRef::SmoothingGroup => smoothing = Some(a),
                    _ => {}
                }
                true
            })? {
                break;
            }
        }
        let positions = positions
            .filter(|a| a.domain() == AttributeDomain::Vertex && a.lanes() == 3 && floating(*a))
            .ok_or(PreparationCause::Normal(NormalCause::Positions))?;
        let generate_normal = options
            .normals
            .is_some_and(|o| normal_count == 0 || o.existing == ExistingNormals::Replace);
        if generate_normal && normal_count > 1 {
            return Err(PreparationCause::Normal(NormalCause::MultipleNormalFields));
        }
        let generate_tangent = tangent_options
            .is_some_and(|t| tangent_count == 0 || t.existing == ExistingTangents::Replace);
        if (generate_tangent || options.tangents == TangentPolicy::Remove) && tangent_count > 1 {
            return Err(PreparationCause::Tangent(
                TangentCause::MultipleTangentFields,
            ));
        }
        if generate_normal
            && tangent_count != 0
            && !generate_tangent
            && options.tangents != TangentPolicy::Remove
        {
            return Err(PreparationCause::InvalidatedTangent);
        }
        let remove_tangents = tangent_count != 0 && options.tangents == TangentPolicy::Remove;
        let origins = if topology == TopologyKind::TriangleList {
            None
        } else if topology == TopologyKind::Polygons {
            let polygon = crate::triangulate::input::projected(
                positions,
                sizes.ok_or(PreparationCause::Triangulation(
                    crate::TriangulationCause::CoreRejected,
                ))?,
                indices,
                operation,
                keys,
            )
            .map_err(PreparationCause::Triangulation)?;
            Some(
                crate::triangulate::input::build_origins(polygon, operation, keys)
                    .map_err(PreparationCause::Triangulation)?,
            )
        } else {
            let mut triangles = ScratchVec::new(operation, keys, faces as usize)?;
            let mut face_offsets = ScratchVec::new(operation, keys, faces as usize + 1)?;
            for f in 0..faces {
                run_unit(operation, keys, || {
                    triangles.values_mut().push(match topology {
                        TopologyKind::TriangleFan => [f + 1, f + 2, 0],
                        _ if f % 2 == 0 => [f, f + 1, f + 2],
                        _ => [f, f + 2, f + 1],
                    });
                    face_offsets.values_mut().push(f);
                })?;
            }
            run_unit(operation, keys, || face_offsets.values_mut().push(faces))?;
            Some(crate::triangulate::input::Origins {
                triangles,
                face_offsets,
            })
        };
        if origins.is_none() && !generate_normal && !generate_tangent && !remove_tangents {
            run_unit(operation, keys, || {
                output
                    .values_mut()
                    .push(PreparedPrimitive::unchanged(target))
            })?;
            continue;
        }
        let count = origins
            .as_ref()
            .map_or(corners as usize, |o| o.triangles.values().len() * 3);
        let output_corners = u32::try_from(count).map_err(|_| {
            PreparationCause::Triangulation(crate::TriangulationCause::SizeOverflow)
        })?;
        let projected_indices = if let Some(origins) = &origins {
            let mut projected = ScratchVec::new(operation, keys, count)?;
            for triangle in origins.triangles.values() {
                for &corner in triangle {
                    let vertex = run_unit(operation, keys, || {
                        indices.map_or(corner, |ids| ids[corner as usize])
                    })?;
                    run_unit(operation, keys, || projected.values_mut().push(vertex))?;
                }
            }
            Some(projected)
        } else {
            None
        };
        let kernel_indices = projected_indices
            .as_ref()
            .map(|p| p.values().as_slice())
            .or(indices);
        let smooth = generate_normal
            && options
                .normals
                .is_some_and(|o| o.mode == NormalMode::Smooth);
        let mut projected_groups = None;
        let groups = if smooth {
            match smoothing {
                Some(a) => {
                    if a.domain() != AttributeDomain::Face {
                        return Err(PreparationCause::Normal(NormalCause::SmoothingGroups));
                    }
                    let AttributeData::DenseU32 { lanes: 1, values } = a.data() else {
                        return Err(PreparationCause::Normal(NormalCause::SmoothingGroups));
                    };
                    if let Some(origins) = &origins {
                        let mut projected = ScratchVec::new(operation, keys, count / 3)?;
                        for (face, interval) in origins.face_offsets.values().windows(2).enumerate()
                        {
                            for _ in interval[0]..interval[1] {
                                run_unit(operation, keys, || {
                                    projected.values_mut().push(values[face])
                                })?;
                            }
                        }
                        projected_groups = Some(projected);
                        projected_groups.as_ref().map(|g| g.values().as_slice())
                    } else {
                        Some(values)
                    }
                }
                None => None,
            }
        } else {
            None
        };
        let mut normals = if generate_normal {
            let source = crate::normals::input::NormalInput::projected(
                positions,
                kernel_indices,
                groups,
                output_corners,
                options.normals.expect("requested normals").mode,
                normal_wide || is_wide(positions),
            );
            Some(
                crate::normals::input::generate(source, operation, keys)
                    .map_err(|e| PreparationCause::Normal(e.into()))?,
            )
        } else {
            None
        };
        drop(projected_groups);
        if let Some(normals) = &mut normals {
            let reverse = match options.orientation {
                NormalOrientation::StoredOrder => false,
                NormalOrientation::SceneFrontFace { assume_unknown } => {
                    let winding = run_unit(operation, keys, || scene.space().front_face_winding)?;
                    let winding = if winding == FrontFaceWinding::Unknown {
                        assume_unknown.unwrap_or(FrontFaceWinding::Unknown)
                    } else {
                        winding
                    };
                    match winding {
                        FrontFaceWinding::Clockwise => true,
                        FrontFaceWinding::CounterClockwise => false,
                        _ => return Err(PreparationCause::UnknownFrontFace),
                    }
                }
            };
            normals
                .orient(reverse, operation, keys)
                .map_err(|e| PreparationCause::Normal(e.into()))?;
        }
        let mut normal_domain =
            tangent_options.map_or(AttributeDomain::Corner, |t| t.normal_domain);
        let tangents = if generate_tangent {
            let t = tangent_options.expect("requested tangents");
            if !matches!(
                t.normal_domain,
                AttributeDomain::Vertex | AttributeDomain::Corner
            ) || !matches!(
                t.uv_domain,
                AttributeDomain::Vertex | AttributeDomain::Corner
            ) {
                return Err(PreparationCause::Tangent(TangentCause::UnsupportedDomain));
            }
            let uvs = selected_uv
                .filter(|a| a.lanes() == 2 && floating(*a))
                .ok_or(PreparationCause::Tangent(TangentCause::UvField))?;
            let source_normal = match &normals {
                Some(n) => {
                    normal_domain = AttributeDomain::Corner;
                    crate::tangents::input::ProjectedNormals::Generated(n.values())
                }
                None => crate::tangents::input::ProjectedNormals::Authored(
                    selected_normal
                        .filter(|a| a.lanes() == 3 && floating(*a))
                        .ok_or(PreparationCause::Tangent(TangentCause::NormalField))?,
                ),
            };
            let source = crate::tangents::input::TangentInput::projected(
                output_corners,
                kernel_indices,
                positions,
                source_normal,
                uvs,
                origins.as_ref().map(|o| o.triangles.values().as_slice()),
                tangent_wide,
                operation,
                keys,
            )
            .map_err(|e| PreparationCause::Tangent(e.into()))?;
            Some(
                crate::tangents::input::generate(source, operation, keys)
                    .map_err(|e| PreparationCause::Tangent(e.into()))?,
            )
        } else {
            None
        };
        drop(projected_indices);
        let prepared = PreparedPrimitive {
            target,
            origins,
            normals,
            tangents,
            remove_tangents,
            smooth,
            normal_domain,
            uv_set: tangent_options.map_or(0, |t| t.uv_set),
            uv_domain: tangent_options.map_or(AttributeDomain::Vertex, |t| t.uv_domain),
        };
        run_unit(operation, keys, || output.values_mut().push(prepared))?;
    }
    Ok(output)
}

fn floating(a: AttributeRef<'_>) -> bool {
    matches!(
        a.data(),
        AttributeData::DenseF32 { .. }
            | AttributeData::DenseF64 { .. }
            | AttributeData::SparseF32 { .. }
            | AttributeData::SparseF64 { .. }
    )
}
fn is_wide(a: AttributeRef<'_>) -> bool {
    matches!(
        a.data(),
        AttributeData::DenseF64 { .. } | AttributeData::SparseF64 { .. }
    )
}
fn sort_targets(
    values: &mut [PrimitiveId],
    operation: &mut EditOperation,
    keys: RuntimeKeys,
) -> Result<(), PreparationCause> {
    fn sift(
        values: &mut [PrimitiveId],
        mut root: usize,
        end: usize,
        operation: &mut EditOperation,
        keys: RuntimeKeys,
    ) -> Result<(), PreparationCause> {
        while let Some(left) = root
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .filter(|&n| n < end)
        {
            let child = if left + 1 < end
                && run_unit(operation, keys, || values[left] < values[left + 1])?
            {
                left + 1
            } else {
                left
            };
            if !run_unit(operation, keys, || values[root] < values[child])? {
                break;
            }
            run_unit(operation, keys, || values.swap(root, child))?;
            root = child;
        }
        Ok(())
    }
    for root in (0..values.len() / 2).rev() {
        sift(values, root, values.len(), operation, keys)?;
    }
    for end in (1..values.len()).rev() {
        run_unit(operation, keys, || values.swap(0, end))?;
        sift(values, 0, end, operation, keys)?;
    }
    Ok(())
}
