#![allow(
    clippy::result_large_err,
    reason = "resource failures propagate inline without rejection-path allocation"
)]

use super::{
    NormalOrientation, PreparationCause, TangentPolicy, TriangleLowering,
    TrianglePreparationOptions,
};
use crate::operation::runtime::{ActiveRuntime, RuntimeKeys, ScratchVec, run_unit};
use crate::{ExistingNormals, ExistingTangents, NormalCause, NormalMode, TangentCause};
use baozi_core::{
    AttributeData, AttributeDomain, AttributeRef, AttributeSemanticRef, FrontFaceWinding,
    PrimitiveId, PrimitiveRef, TopologyKind, ValidatedScene,
};

#[cfg(test)]
pub(super) mod selection_tests;
#[cfg(test)]
use selection_tests::{Leaf, before_leaf, entered_leaf, returned_leaf};

pub(super) enum SelectedPrimitives<'scene> {
    Empty,
    One([PrimitiveRef<'scene>; 1]),
    Many(ScratchVec<PrimitiveRef<'scene>>),
}

impl<'scene> SelectedPrimitives<'scene> {
    fn resolve(
        scene: &'scene ValidatedScene,
        targets: &[PrimitiveId],
        operation: &mut ActiveRuntime<'_>,
        keys: RuntimeKeys,
    ) -> Result<Self, PreparationCause> {
        match targets {
            [] => Ok(Self::Empty),
            [target] => {
                let primitive = run_unit(operation, || scene.primitive(*target))?
                    .map_err(PreparationCause::Target)?;
                Ok(Self::One([primitive]))
            }
            _ => {
                let mut selected = ScratchVec::new(operation, keys, targets.len())?;
                #[cfg(test)]
                selection_tests::storage_ready(operation, keys, selected.values());
                for &target in targets {
                    #[cfg(test)]
                    before_leaf(operation, Leaf::Lookup, selected.values());
                    let primitive = run_unit(operation, || {
                        #[cfg(test)]
                        entered_leaf();
                        scene.primitive(target)
                    });
                    #[cfg(test)]
                    selection_tests::lookup_returned(operation, selected.values(), &primitive);
                    let primitive = primitive?.map_err(PreparationCause::Target)?;
                    #[cfg(test)]
                    before_leaf(operation, Leaf::Append, selected.values());
                    let appended = run_unit(operation, || {
                        #[cfg(test)]
                        entered_leaf();
                        selected.values_mut().push(primitive)
                    });
                    #[cfg(test)]
                    returned_leaf(operation, selected.values(), appended.is_ok());
                    appended?;
                }
                sort_selected(selected.values_mut(), operation)?;
                for pair in selected.values().windows(2) {
                    #[cfg(test)]
                    before_leaf(operation, Leaf::Duplicate, selected.values());
                    let duplicate = run_unit(operation, || {
                        #[cfg(test)]
                        entered_leaf();
                        pair[0].id() == pair[1].id()
                    });
                    #[cfg(test)]
                    returned_leaf(operation, selected.values(), duplicate.is_ok());
                    if duplicate? {
                        return Err(PreparationCause::DuplicateTarget);
                    }
                }
                Ok(Self::Many(selected))
            }
        }
    }

    pub(super) fn as_slice(&self) -> &[PrimitiveRef<'scene>] {
        match self {
            Self::Empty => &[],
            Self::One(selected) => selected,
            Self::Many(selected) => selected.values(),
        }
    }

    #[cfg(test)]
    pub(super) fn scratch_values(&self) -> Option<&Vec<PrimitiveRef<'scene>>> {
        match self {
            Self::Many(selected) => Some(selected.values()),
            Self::Empty | Self::One(_) => None,
        }
    }
}

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

pub(super) enum PreparedSelection {
    Unchanged,
    Prepared(ScratchVec<PreparedPrimitive>),
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
}

pub(super) fn prepare(
    scene: &ValidatedScene,
    targets: &[PrimitiveId],
    options: TrianglePreparationOptions,
    operation: &mut ActiveRuntime<'_>,
    keys: RuntimeKeys,
) -> Result<PreparedSelection, PreparationCause> {
    let selected = SelectedPrimitives::resolve(scene, targets, operation, keys)?;
    #[cfg(test)]
    selection_tests::validation_started();
    let mut output: Option<ScratchVec<PreparedPrimitive>> = None;
    for (index, &primitive) in selected.as_slice().iter().enumerate() {
        let target = run_unit(operation, || primitive.id())?;
        let (topology, corners, faces, indices, sizes, mut attributes) =
            run_unit(operation, || {
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
            if let Some(output) = &mut output {
                run_unit(operation, || {
                    output
                        .values_mut()
                        .push(PreparedPrimitive::unchanged(target))
                })?;
            }
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
            if !run_unit(operation, || {
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
                run_unit(operation, || {
                    triangles.values_mut().push(match topology {
                        TopologyKind::TriangleFan => [f + 1, f + 2, 0],
                        _ if f % 2 == 0 => [f, f + 1, f + 2],
                        _ => [f, f + 2, f + 1],
                    });
                    face_offsets.values_mut().push(f);
                })?;
            }
            run_unit(operation, || face_offsets.values_mut().push(faces))?;
            Some(crate::triangulate::input::Origins {
                triangles,
                face_offsets,
            })
        };
        if origins.is_none() && !generate_normal && !generate_tangent && !remove_tangents {
            if let Some(output) = &mut output {
                run_unit(operation, || {
                    output
                        .values_mut()
                        .push(PreparedPrimitive::unchanged(target))
                })?;
            }
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
                    let vertex = run_unit(operation, || {
                        indices.map_or(corner, |ids| ids[corner as usize])
                    })?;
                    run_unit(operation, || projected.values_mut().push(vertex))?;
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
                                run_unit(operation, || projected.values_mut().push(values[face]))?;
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
                    let winding = run_unit(operation, || scene.space().front_face_winding)?;
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
                .orient(reverse, operation)
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
        let output = match &mut output {
            Some(output) => output,
            empty => {
                #[cfg(test)]
                super::cleanup_tests::before_prepared_output(
                    operation, keys, &prepared, &selected,
                )?;
                let mut output = ScratchVec::new(operation, keys, targets.len())?;
                for previous in &selected.as_slice()[..index] {
                    let target = run_unit(operation, || previous.id())?;
                    run_unit(operation, || {
                        output
                            .values_mut()
                            .push(PreparedPrimitive::unchanged(target))
                    })?;
                    #[cfg(test)]
                    super::cleanup_tests::prepared_prefix_ready(
                        operation, keys, &prepared, &output,
                    )?;
                }
                empty.insert(output)
            }
        };
        run_unit(operation, || output.values_mut().push(prepared))?;
    }
    Ok(match output {
        Some(output) => PreparedSelection::Prepared(output),
        None => PreparedSelection::Unchanged,
    })
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
fn sort_selected(
    values: &mut [PrimitiveRef<'_>],
    operation: &mut ActiveRuntime<'_>,
) -> Result<(), PreparationCause> {
    fn sift(
        values: &mut [PrimitiveRef<'_>],
        mut root: usize,
        end: usize,
        operation: &mut ActiveRuntime<'_>,
    ) -> Result<(), PreparationCause> {
        while let Some(left) = root
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .filter(|&n| n < end)
        {
            let child = if left + 1 < end && {
                #[cfg(test)]
                before_leaf(operation, Leaf::Comparison, values);
                let comparison = run_unit(operation, || {
                    #[cfg(test)]
                    entered_leaf();
                    values[left].id() < values[left + 1].id()
                });
                #[cfg(test)]
                returned_leaf(operation, values, comparison.is_ok());
                comparison?
            } {
                left + 1
            } else {
                left
            };
            #[cfg(test)]
            before_leaf(operation, Leaf::Comparison, values);
            let comparison = run_unit(operation, || {
                #[cfg(test)]
                entered_leaf();
                values[root].id() < values[child].id()
            });
            #[cfg(test)]
            returned_leaf(operation, values, comparison.is_ok());
            if !comparison? {
                break;
            }
            #[cfg(test)]
            before_leaf(operation, Leaf::Swap, values);
            let swapped = run_unit(operation, || {
                #[cfg(test)]
                entered_leaf();
                values.swap(root, child)
            });
            #[cfg(test)]
            returned_leaf(operation, values, swapped.is_ok());
            swapped?;
            root = child;
        }
        Ok(())
    }
    for root in (0..values.len() / 2).rev() {
        sift(values, root, values.len(), operation)?;
    }
    for end in (1..values.len()).rev() {
        #[cfg(test)]
        before_leaf(operation, Leaf::Swap, values);
        let swapped = run_unit(operation, || {
            #[cfg(test)]
            entered_leaf();
            values.swap(0, end)
        });
        #[cfg(test)]
        returned_leaf(operation, values, swapped.is_ok());
        swapped?;
        sift(values, 0, end, operation)?;
    }
    Ok(())
}
